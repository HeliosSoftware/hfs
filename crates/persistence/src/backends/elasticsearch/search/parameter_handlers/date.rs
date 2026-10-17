//! Date parameter handler for Elasticsearch.

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Value, json};

use crate::search::{
    DatePredicate, DateValuePrecision, FhirDateValue, RangeCondition, StorageResolution,
};
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
/// the window [`FhirDateValue::approx_window`] — the range widened by the
/// shared margin.
///
/// Only `gte` and `lt` bounds are ever emitted, and always as complete
/// server-generated dates. This used to send a value with a time as written,
/// under `gte`/`lte`, and matched the whole second only because Elasticsearch
/// happens to round an `lte` bound *up* over the fields it is missing; an
/// explicit half-open range does not depend on that. The range is taken at
/// millisecond resolution, which is what an Elasticsearch `date` holds.
///
/// `None` when the value is not a date; see [`match_none`].
pub(crate) fn field_range(field: &str, value: &str, prefix: SearchPrefix) -> Option<DateRange> {
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

    // `ap` accepts a point inside the window every backend shares
    // ([`FhirDateValue::approx_window`], #1391): the precision range widened by
    // the same margin as the indexed ranges, so `_lastUpdated=ap…` and a
    // composite's date component agree with MongoDB and with `build_clause`.
    let Some(predicate) = parsed.predicate(prefix, StorageResolution::Millis) else {
        let (low, high) = parsed.approx_window(StorageResolution::Millis);
        return Some(DateRange::Within(range(
            json!({ "gte": bound(low), "lt": bound(high) }),
        )));
    };

    Some(match predicate {
        DatePredicate::Within { ge, lt } => {
            DateRange::Within(range(json!({ "gte": bound(ge), "lt": bound(lt) })))
        }
        DatePredicate::Outside { lt, ge } => {
            // The complement of `[lt, ge)`, negated by the caller.
            DateRange::Outside(range(json!({ "gte": bound(lt), "lt": bound(ge) })))
        }
        DatePredicate::AtOrAfter(at) => DateRange::Within(range(json!({ "gte": bound(at) }))),
        DatePredicate::Before(at) => DateRange::Within(range(json!({ "lt": bound(at) }))),
    })
}

/// Formats a range bound in a form the `date` mapping accepts: a plain
/// `yyyy-MM-dd` for the date-only precisions, whose bounds are always UTC
/// midnights, and an RFC 3339 UTC instant with milliseconds otherwise.
fn es_bound(instant: DateTime<Utc>, precision: DateValuePrecision) -> String {
    match precision {
        DateValuePrecision::Year | DateValuePrecision::Month | DateValuePrecision::Day
            if instant.timestamp_subsec_nanos() == 0 =>
        {
            instant.format("%Y-%m-%d").to_string()
        }
        _ => instant.to_rfc3339_opts(SecondsFormat::Millis, true),
    }
}

/// Builds an ES query clause for an indexed date search parameter.
///
/// Every indexed date is a range `[value, end)` — one unit of its precision
/// for a point, the whole of a `Period` — and each prefix compares the search
/// range against it by the FHIR rules for a range target, as
/// [`FhirDateValue::range_predicate`] spells them for every backend (#1391):
/// `eq` needs the stored range inside the search range, `sa` its start after
/// the search range, `ap` an overlap with the search range widened by
/// [`FhirDateValue::approx_margin`], and so on. Both comparisons of a group
/// apply to the same nested entry, so a `Period` is never matched by its start
/// against one condition and by its end against another.
///
/// The bounds are complete instants, never a partial date: Elasticsearch
/// rounds a partial `gt`/`lte` bound *up* over the fields it is missing, which
/// would move the `end` comparisons by up to a day.
///
/// `_lastUpdated` and composite components are points and keep
/// [`field_range`].
///
/// Always `Some`: a value that is not a date yields [`match_none`].
pub fn build_clause(name: &str, value: &str, prefix: SearchPrefix) -> Option<Value> {
    let parsed = match FhirDateValue::parse(value) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(
                "unvalidated date search value reached the Elasticsearch handler: {error}"
            );
            return Some(match_none());
        }
    };
    let predicate = parsed.range_predicate(prefix, StorageResolution::Millis);

    let condition = |condition: &RangeCondition| {
        let (field, op, at) = match *condition {
            RangeCondition::StartAtOrAfter(at) => ("search_params.date.value", "gte", at),
            RangeCondition::StartBefore(at) => ("search_params.date.value", "lt", at),
            RangeCondition::EndAfter(at) => ("search_params.date.end", "gt", at),
            RangeCondition::EndAtOrBefore(at) => ("search_params.date.end", "lte", at),
        };
        json!({ "range": { field: { op: at.to_rfc3339_opts(SecondsFormat::Millis, true) } } })
    };
    let mut must = vec![json!({ "term": { "search_params.date.name": name } })];
    match predicate.any_of.as_slice() {
        [group] => must.extend(group.iter().map(condition)),
        groups => must.push(json!({
            "bool": {
                "should": groups
                    .iter()
                    .map(|group| json!({ "bool": { "must": group.iter().map(condition).collect::<Vec<_>>() } }))
                    .collect::<Vec<_>>(),
                "minimum_should_match": 1
            }
        })),
    }

    Some(json!({
        "nested": {
            "path": "search_params.date",
            "query": { "bool": { "must": must } }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn within(value: &str, prefix: SearchPrefix) -> Value {
        match field_range("f", value, prefix) {
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

    /// The `must` list of a clause, after the name term.
    fn conditions(value: &str, prefix: SearchPrefix) -> Vec<Value> {
        let clause = build_clause("date", value, prefix).unwrap();
        let must = clause["nested"]["query"]["bool"]["must"]
            .as_array()
            .expect("a bool must")
            .clone();
        assert_eq!(
            must[0],
            json!({ "term": { "search_params.date.name": "date" } })
        );
        must[1..].to_vec()
    }

    fn range(field: &str, op: &str, at: &str) -> Value {
        json!({ "range": { format!("search_params.date.{field}"): { op: at } } })
    }

    fn any_of(groups: Vec<Vec<Value>>) -> Value {
        json!({
            "bool": {
                "should": groups
                    .into_iter()
                    .map(|group| json!({ "bool": { "must": group } }))
                    .collect::<Vec<_>>(),
                "minimum_should_match": 1
            }
        })
    }

    /// #1391: every prefix compares the search range `[s, e)` against the
    /// stored range `[value, end)` of the same nested entry, per the FHIR
    /// table for a range target.
    #[test]
    fn prefixes_compare_the_search_range_with_the_stored_range() {
        let (s, e) = ("2024-01-15T00:00:00.000Z", "2024-01-16T00:00:00.000Z");
        let eq = vec![range("value", "gte", s), range("end", "lte", e)];
        assert_eq!(conditions("2024-01-15", SearchPrefix::Eq), eq);
        assert_eq!(
            conditions("2024-01-15", SearchPrefix::Ne),
            vec![any_of(vec![
                vec![range("value", "lt", s)],
                vec![range("end", "gt", e)],
            ])]
        );
        assert_eq!(
            conditions("2024-01-15", SearchPrefix::Gt),
            vec![range("end", "gt", e)]
        );
        assert_eq!(
            conditions("2024-01-15", SearchPrefix::Lt),
            vec![range("value", "lt", s)]
        );
        assert_eq!(
            conditions("2024-01-15", SearchPrefix::Ge),
            vec![any_of(vec![vec![range("end", "gt", e)], eq.clone()])]
        );
        assert_eq!(
            conditions("2024-01-15", SearchPrefix::Le),
            vec![any_of(vec![vec![range("value", "lt", s)], eq])]
        );
        assert_eq!(
            conditions("2024-01-15", SearchPrefix::Sa),
            vec![range("value", "gte", e)]
        );
        assert_eq!(
            conditions("2024-01-15", SearchPrefix::Eb),
            vec![range("end", "lte", s)]
        );
    }

    /// #1391: `ap` is an overlap with the search range widened by the shared
    /// margin — a day either side of a day — no longer the plain `eq` range.
    #[test]
    fn ap_is_an_overlap_with_the_widened_range() {
        assert_eq!(
            conditions("2024-01-15", SearchPrefix::Ap),
            vec![
                range("value", "lt", "2024-01-17T00:00:00.000Z"),
                range("end", "gt", "2024-01-14T00:00:00.000Z"),
            ]
        );
        assert_eq!(
            conditions("2024-01", SearchPrefix::Ap),
            vec![
                range("value", "lt", "2024-03-01T00:00:00.000Z"),
                range("end", "gt", "2023-12-01T00:00:00.000Z"),
            ]
        );
    }

    /// Bounds are complete instants even for a date-only value: Elasticsearch
    /// rounds a partial `gt`/`lte` bound up to the end of the missing fields.
    #[test]
    fn bounds_are_complete_instants() {
        let clause =
            serde_json::to_string(&build_clause("date", "2024", SearchPrefix::Le).unwrap())
                .unwrap();
        assert!(clause.contains("\"2025-01-01T00:00:00.000Z\""), "{clause}");
        assert!(!clause.contains("\"2025-01-01\""), "{clause}");
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
        // `ap` is the range widened by ten seconds a side (#1391).
        assert_eq!(
            within(instant, SearchPrefix::Ap),
            json!({ "gte": "2024-01-15T09:59:50.000Z", "lt": "2024-01-15T10:00:11.000Z" })
        );

        let Some(DateRange::Outside(ne)) = field_range("f", instant, SearchPrefix::Ne) else {
            panic!("ne must be a negated range")
        };
        assert_eq!(ne["range"]["f"], json!({ "gte": start, "lt": end }));
    }

    /// #1391: on a point field (`_lastUpdated`, a composite's date component)
    /// `ap` is the shared window, the same one MongoDB builds: a year, a month,
    /// a day, ten minutes or ten seconds either side of the value's own range,
    /// by precision — no longer plain `eq`.
    #[test]
    fn ap_on_a_point_field_is_the_shared_approx_window() {
        for (value, low, high) in [
            ("2024", "2023-01-01", "2026-01-01"),
            ("2024-03", "2024-02-01", "2024-05-01"),
            ("2024-03-15", "2024-03-14", "2024-03-17"),
            (
                "2024-03-15T10:30Z",
                "2024-03-15T10:20:00.000Z",
                "2024-03-15T10:41:00.000Z",
            ),
            (
                "2024-03-15T10:30:00Z",
                "2024-03-15T10:29:50.000Z",
                "2024-03-15T10:30:11.000Z",
            ),
            (
                "2024-03-15T10:30:00.123Z",
                "2024-03-15T10:29:50.123Z",
                "2024-03-15T10:30:10.124Z",
            ),
        ] {
            assert_eq!(
                within(value, SearchPrefix::Ap),
                json!({ "gte": low, "lt": high }),
                "ap{value}"
            );
            // Not the `eq` range it used to be.
            assert_ne!(
                within(value, SearchPrefix::Ap),
                within(value, SearchPrefix::Eq),
                "ap{value}"
            );
        }
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
                assert!(field_range("f", value, prefix).is_none(), "{prefix}{value}");
                assert_eq!(
                    build_clause("date", value, prefix),
                    Some(json!({ "match_none": {} })),
                    "{prefix}{value}"
                );
            }
        }
    }
}
