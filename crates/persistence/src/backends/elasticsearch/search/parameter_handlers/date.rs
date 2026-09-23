//! Date parameter handler for Elasticsearch.

use chrono::{DateTime, SecondsFormat, Timelike, Utc};
use serde_json::{Value, json};

use crate::search::{DatePredicate, DateValuePrecision, FhirDateValue, StorageResolution};
use crate::types::SearchPrefix;

/// A precision-aware comparison on one ES date field, ready to be wrapped in
/// whatever query shape the caller needs (nested for indexed parameters, a
/// bare top-level clause for `_lastUpdated`).
#[derive(Debug)]
pub(crate) enum DateRange {
    /// `{ "range": { field: bounds } }` — the value must fall inside.
    Within(Value),
    /// `{ "range": { field: bounds } }` — the value must fall *outside*
    /// (`ne`). The caller negates it with `must_not` so the negation is
    /// applied at the right level of the enclosing query.
    Outside(Value),
}

/// The clause for a date value that is not a date: it matches no document.
///
/// The search gate (`validate_date_values`) turns such a value into an error
/// before a query is built, so this is only reached by a caller that skipped
/// it. It must still be a clause and never `None`: the query builder collects
/// value clauses with `filter_map`, so a `None` would silently drop the
/// constraint and return every resource of the type — the very widening this
/// handler used to perform by reading garbage as the year 2000 (#1293).
pub(crate) fn match_none() -> Value {
    json!({ "match_none": {} })
}

/// Builds the `range` comparison for `field` from the range the value names
/// at its own precision — a year, a month, a day, a minute, a second, or a
/// fraction of one, as [`FhirDateValue`] defines for every backend: `eq` means
/// `[start, end)`, `ne` its complement, `gt`/`sa` start at the end of the
/// range, `lt`/`eb` end before its start, and `le` reaches its end. `ap` is
/// that range widened by a tenth of its distance from `now`, the instant the
/// search is evaluated at (#1390).
///
/// Only `gte` and `lt` bounds are ever emitted, and always as complete
/// server-generated dates. This used to send a value with a time as written,
/// under `gte`/`lte`, and matched the whole second only because Elasticsearch
/// happens to round an `lte` bound *up* over the fields it is missing; an
/// explicit half-open range does not depend on that. The range is taken at
/// millisecond resolution, which is what an Elasticsearch `date` holds.
///
/// `None` when the value is not a date; see [`match_none`].
pub(crate) fn field_range(
    field: &str,
    value: &str,
    prefix: SearchPrefix,
    now: DateTime<Utc>,
) -> Option<DateRange> {
    let parsed = match FhirDateValue::parse(value) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(
                "unvalidated date search value reached the Elasticsearch handler: {error}"
            );
            return None;
        }
    };
    let bound = |instant: DateTime<Utc>| es_bound(instant, parsed.precision);
    let range = |bounds: Value| json!({ "range": { field: bounds } });

    Some(
        match parsed.predicate(prefix, StorageResolution::Millis, now) {
            DatePredicate::Within { ge, lt } => {
                DateRange::Within(range(json!({ "gte": bound(ge), "lt": bound(lt) })))
            }
            DatePredicate::Outside { lt, ge } => {
                // The complement of `[lt, ge)`, negated by the caller.
                DateRange::Outside(range(json!({ "gte": bound(lt), "lt": bound(ge) })))
            }
            DatePredicate::AtOrAfter(at) => DateRange::Within(range(json!({ "gte": bound(at) }))),
            DatePredicate::Before(at) => DateRange::Within(range(json!({ "lt": bound(at) }))),
        },
    )
}

/// Formats a range bound in a form the `date` mapping accepts: a plain
/// `yyyy-MM-dd` for a UTC midnight bound of a date-only precision, and an
/// RFC 3339 UTC instant with milliseconds otherwise. An `ap` window moves the
/// bounds of a date-only value off midnight, so the time must be kept then.
fn es_bound(instant: DateTime<Utc>, precision: DateValuePrecision) -> String {
    let midnight = instant.num_seconds_from_midnight() == 0 && instant.nanosecond() == 0;
    match precision {
        DateValuePrecision::Year | DateValuePrecision::Month | DateValuePrecision::Day
            if midnight =>
        {
            instant.format("%Y-%m-%d").to_string()
        }
        _ => instant.to_rfc3339_opts(SecondsFormat::Millis, true),
    }
}

/// Builds an ES query clause for an indexed date search parameter.
///
/// Always `Some`: a value that is not a date yields [`match_none`].
pub fn build_clause(
    name: &str,
    value: &str,
    prefix: SearchPrefix,
    now: DateTime<Utc>,
) -> Option<Value> {
    let name_term = json!({ "term": { "search_params.date.name": name } });
    let bool_body = match field_range("search_params.date.value", value, prefix, now) {
        Some(DateRange::Within(range)) => json!({ "must": [name_term, range] }),
        Some(DateRange::Outside(range)) => json!({ "must": [name_term], "must_not": [range] }),
        None => return Some(match_none()),
    };

    Some(json!({
        "nested": {
            "path": "search_params.date",
            "query": { "bool": bool_body }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// The instant `ap` windows are measured from in these tests.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    fn within(value: &str, prefix: SearchPrefix) -> Value {
        match field_range("f", value, prefix, now()) {
            Some(DateRange::Within(range)) => range["range"]["f"].clone(),
            other => panic!("{prefix}{value} must be a plain range: {other:?}"),
        }
    }

    #[test]
    fn test_year_precision() {
        assert_eq!(
            within("2024", SearchPrefix::Eq),
            json!({ "gte": "2024-01-01", "lt": "2025-01-01" })
        );
    }

    #[test]
    fn test_month_precision() {
        assert_eq!(
            within("2024-01", SearchPrefix::Eq),
            json!({ "gte": "2024-01-01", "lt": "2024-02-01" })
        );
        assert_eq!(
            within("2024-12", SearchPrefix::Eq),
            json!({ "gte": "2024-12-01", "lt": "2025-01-01" })
        );
    }

    #[test]
    fn test_day_precision() {
        assert_eq!(
            within("2024-01-15", SearchPrefix::Eq),
            json!({ "gte": "2024-01-15", "lt": "2024-01-16" })
        );
    }

    #[test]
    fn test_eq_range() {
        let clause = build_clause("birthdate", "2024-01-15", SearchPrefix::Eq, now()).unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(s.contains("gte"));
        assert!(s.contains("2024-01-15"));
        assert!(s.contains("2024-01-16"));
    }

    #[test]
    fn test_gt_range() {
        let clause = build_clause("birthdate", "2024-01-15", SearchPrefix::Gt, now()).unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(s.contains("gte"));
        assert!(s.contains("2024-01-16")); // starts after precision range
    }

    #[test]
    fn ne_is_the_negated_precision_range() {
        let clause = build_clause("birthdate", "2024-01-15", SearchPrefix::Ne, now()).unwrap();
        let bool_body = &clause["nested"]["query"]["bool"];
        assert_eq!(
            bool_body["must"][0]["term"]["search_params.date.name"],
            "birthdate"
        );
        let range = &bool_body["must_not"][0]["range"]["search_params.date.value"];
        assert_eq!(range["gte"], "2024-01-15");
        assert_eq!(range["lt"], "2024-01-16");
    }

    #[test]
    fn sa_and_eb_mirror_gt_and_lt_on_the_whole_period() {
        assert_eq!(
            within("2024-01", SearchPrefix::Sa),
            json!({ "gte": "2024-02-01" })
        );
        assert_eq!(
            within("2024-01", SearchPrefix::Eb),
            json!({ "lt": "2024-01-01" })
        );
    }

    /// A value with a time is the whole second, as an explicit half-open
    /// range — not `gte X, lte X` relying on Elasticsearch rounding `lte` up.
    #[test]
    fn second_precision_is_an_explicit_one_second_range() {
        let instant = "2024-01-15T10:00:00Z";
        let (start, end) = ("2024-01-15T10:00:00.000Z", "2024-01-15T10:00:01.000Z");
        assert_eq!(
            within(instant, SearchPrefix::Eq),
            json!({ "gte": start, "lt": end })
        );
        assert_eq!(within(instant, SearchPrefix::Gt), json!({ "gte": end }));
        assert_eq!(within(instant, SearchPrefix::Sa), json!({ "gte": end }));
        assert_eq!(within(instant, SearchPrefix::Ge), json!({ "gte": start }));
        assert_eq!(within(instant, SearchPrefix::Lt), json!({ "lt": start }));
        assert_eq!(within(instant, SearchPrefix::Eb), json!({ "lt": start }));
        assert_eq!(within(instant, SearchPrefix::Le), json!({ "lt": end }));

        let Some(DateRange::Outside(ne)) = field_range("f", instant, SearchPrefix::Ne, now())
        else {
            panic!("ne must be a negated range")
        };
        assert_eq!(ne["range"]["f"], json!({ "gte": start, "lt": end }));
    }

    #[test]
    fn offsets_minutes_and_fractions_become_utc_millisecond_bounds() {
        // A negative offset is folded to UTC rather than sent as written.
        assert_eq!(
            within("2013-04-05T23:30:00-04:00", SearchPrefix::Eq),
            json!({ "gte": "2013-04-06T03:30:00.000Z", "lt": "2013-04-06T03:30:01.000Z" })
        );
        // Minute precision is valid in FHIR search.
        assert_eq!(
            within("2013-04-05T09:20", SearchPrefix::Eq),
            json!({ "gte": "2013-04-05T09:20:00.000Z", "lt": "2013-04-05T09:21:00.000Z" })
        );
        // An ES date holds milliseconds; a finer value is its millisecond.
        assert_eq!(
            within("2021-11-10T16:48:57.246958-08:00", SearchPrefix::Eq),
            json!({ "gte": "2021-11-11T00:48:57.246Z", "lt": "2021-11-11T00:48:57.247Z" })
        );
        // #1296: a `+` that form decoding turned into a space.
        assert_eq!(
            within("2013-04-05T18:50:00 05:30", SearchPrefix::Eq),
            within("2013-04-05T18:50:00+05:30", SearchPrefix::Eq)
        );
    }

    /// `ap` is the shared window (#1390): the precision range widened by a
    /// tenth of its distance from `now`, on both sides. It used to fall back
    /// to `eq`, the precision range alone.
    #[test]
    fn ap_widens_the_range_by_a_tenth_of_its_distance_from_now() {
        // 2016 ends 3287 days before `now`: 328.7 days each side, and the
        // bounds, no longer midnights, keep their time.
        assert_eq!(
            within("2016", SearchPrefix::Ap),
            json!({ "gte": "2015-02-06T07:12:00.000Z", "lt": "2017-11-25T16:48:00.000Z" })
        );

        // 2036 starts 3652 days after `now`: 365.2 days each side.
        assert_eq!(
            within("2036", SearchPrefix::Ap),
            json!({ "gte": "2034-12-31T19:12:00.000Z", "lt": "2038-01-01T04:48:00.000Z" })
        );

        // A range that holds `now` is exactly `eq`.
        assert_eq!(
            within("2026", SearchPrefix::Ap),
            within("2026", SearchPrefix::Eq)
        );

        // A second-precision value close to `now` barely widens.
        assert_eq!(
            within("2025-12-31T23:59:50Z", SearchPrefix::Ap),
            json!({ "gte": "2025-12-31T23:59:49.100Z", "lt": "2025-12-31T23:59:51.900Z" })
        );
    }

    #[test]
    fn ap_contains_eq_at_every_precision() {
        for value in [
            "2016",
            "2016-03",
            "2016-03-04",
            "2016-03-04T05:06",
            "2016-03-04T05:06:07Z",
            "2016-03-04T05:06:07.890Z",
            "2040-07-08",
        ] {
            let Some(DateRange::Within(ap)) = field_range("f", value, SearchPrefix::Ap, now())
            else {
                panic!("ap{value} must be a plain range")
            };
            let Some(DateRange::Within(eq)) = field_range("f", value, SearchPrefix::Eq, now())
            else {
                panic!("eq{value} must be a plain range")
            };
            let instant = |v: &Value| {
                let text = v.as_str().expect("a bound is a string");
                DateTime::parse_from_rfc3339(text)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(|_| {
                        chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
                            .expect("a date-only bound")
                            .and_hms_opt(0, 0, 0)
                            .unwrap()
                            .and_utc()
                    })
            };
            let (ap, eq) = (&ap["range"]["f"], &eq["range"]["f"]);
            assert!(
                instant(&ap["gte"]) <= instant(&eq["gte"]),
                "ap{value}: {ap} vs {eq}"
            );
            assert!(
                instant(&ap["lt"]) >= instant(&eq["lt"]),
                "ap{value}: {ap} vs {eq}"
            );
        }
    }

    /// #1293: these were read as the year 2000 (`gtnot-a-date` was "after
    /// 2000-01-01" and matched everything), or sent to Elasticsearch as
    /// written. Under every prefix they now match nothing — `ne` included.
    #[test]
    fn a_value_that_is_not_a_date_matches_nothing() {
        for value in [
            "not-a-date",
            "abcd",
            "2024-1x",
            "2024-13-45",
            "2024-02-30",
            "T25:00:00Z",
            "2013-04-05T10",
            "",
        ] {
            for prefix in [
                SearchPrefix::Eq,
                SearchPrefix::Ne,
                SearchPrefix::Gt,
                SearchPrefix::Lt,
                SearchPrefix::Ge,
                SearchPrefix::Le,
                SearchPrefix::Sa,
                SearchPrefix::Eb,
                SearchPrefix::Ap,
            ] {
                assert!(
                    field_range("f", value, prefix, now()).is_none(),
                    "{prefix}{value}"
                );
                assert_eq!(
                    build_clause("date", value, prefix, now()),
                    Some(json!({ "match_none": {} })),
                    "{prefix}{value}"
                );
            }
        }
    }
}
