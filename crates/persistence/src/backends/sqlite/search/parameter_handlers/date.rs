//! Date parameter SQL handler.

use crate::search::{DateValuePrecision, FhirDateValue};
use crate::types::{SearchPrefix, SearchValue};

use super::super::query_builder::{SqlFragment, SqlParam};

/// Builds a precision-aware date comparison against a `value_date`-style TEXT
/// column, with one bind parameter (#456).
///
/// Stored values keep whatever precision the resource carried
/// (`"1995-10-02"`, `"2016-01-23T13:07:42-04:00"`), while search bounds are
/// full datetimes — and SQLite compares TEXT lexicographically, where
/// `'1995-10-02' < '1995-10-02T00:00:00'`, so a day never fell inside its own
/// range. Both sides therefore go through `datetime()`, which normalizes
/// partial dates to `YYYY-MM-DD HH:MM:SS` and folds timezone offsets to UTC.
/// The upper bound of a partial-precision value is derived in SQL with a
/// modifier (`'+1 day'`), so a single parameter serves both ends of the range
/// wherever the caller can only bind one.
///
/// `datetime()` truncates fractional seconds, which is right for a search
/// value of second precision (its range is the whole second) but not for one
/// of millisecond precision: `_lastUpdated=eq2026-09-06T08:44:27.804Z` would
/// match every resource written in that second, which is exactly what a
/// transaction Bundle produces once the writes are fast enough to land in one.
/// Millisecond-precision values therefore go through `strftime('%f')`, which
/// keeps `SS.SSS` and still folds timezone offsets to UTC. Both sides are
/// first cut to three fractional digits in SQL ([`truncated_to_millis`]):
/// `last_updated` holds nanoseconds, which SQLite would *round* to the
/// nearest millisecond while `meta.lastUpdated` (what the client searches
/// with) truncates — off by one millisecond half the time. The approximate
/// prefix keeps `datetime()`: its ±10-second window is coarser than a second
/// anyway.
///
/// The search value itself is read by [`FhirDateValue`], the grammar every
/// backend shares, and what is bound is a canonical UTC string built from the
/// parsed instant — never the client's text. `datetime()` is lenient in ways
/// the grammar is not: it rolled `2024-02-30` over to March 1st and searched
/// for that (#1295), and it returns NULL for a leap second or for an offset
/// whose `+` was form-decoded into a space (#1296).
///
/// This does not translate [`crate::search::DatePredicate`]: the single-bind
/// SQL above predates it and already agrees with it (the tests hold the two
/// against each other), with one documented exception. A fraction is always
/// compared at the millisecond — `.5` means `[.500, .501)` here, not
/// `[.5, .6)`, and digits past the third are cut — because both sides go
/// through the same millisecond truncation.
///
/// Returns the SQL and the value to bind for its (single) parameter, or `None`
/// when the value is not a date. The search gate
/// ([`crate::search::validate_date_values`]) rejects such a value before any
/// SQL is built, so `None` means a caller skipped it; every caller must then
/// match nothing rather than drop the constraint.
pub(crate) fn date_condition(
    column: &str,
    prefix: SearchPrefix,
    value: &str,
    param_num: usize,
) -> Option<(String, String)> {
    let parsed = match FhirDateValue::parse(value) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!("unvalidated date search value reached the SQLite handler: {error}");
            return None;
        }
    };
    let precision = parsed.precision;

    // The SQL modifier that derives the range end from the bound range start.
    // Second precision needs none: `datetime()` cuts the column to the second,
    // so plain comparisons already treat the whole second as one value.
    let bump = match precision {
        DateValuePrecision::Year => Some("+1 year"),
        DateValuePrecision::Month => Some("+1 month"),
        DateValuePrecision::Day => Some("+1 day"),
        DateValuePrecision::Minute => Some("+1 minute"),
        DateValuePrecision::Second | DateValuePrecision::Fraction(_) => None,
    };

    // The range start, in UTC. `%.3f` truncates, as the column side does.
    let has_fractional_seconds = matches!(precision, DateValuePrecision::Fraction(_));
    let start = match precision {
        DateValuePrecision::Year | DateValuePrecision::Month | DateValuePrecision::Day => {
            parsed.start.format("%Y-%m-%dT%H:%M:%S").to_string()
        }
        DateValuePrecision::Minute | DateValuePrecision::Second => {
            parsed.start.format("%Y-%m-%dT%H:%M:%SZ").to_string()
        }
        DateValuePrecision::Fraction(_) => {
            parsed.start.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
        }
    };

    let normalize = |expr: &str| {
        if has_fractional_seconds && prefix != SearchPrefix::Ap {
            format!(
                "strftime('%Y-%m-%d %H:%M:%f', {})",
                truncated_to_millis(expr)
            )
        } else {
            format!("datetime({expr})")
        }
    };
    let col = normalize(column);
    let p = normalize(&format!("?{param_num}"));
    let end = |m: &str| format!("datetime(?{param_num}, '{m}')");

    let sql = match (prefix, bump) {
        (SearchPrefix::Eq, Some(m)) => format!("({col} >= {p} AND {col} < {})", end(m)),
        (SearchPrefix::Eq, None) => format!("{col} = {p}"),
        (SearchPrefix::Ne, Some(m)) => format!("({col} < {p} OR {col} >= {})", end(m)),
        (SearchPrefix::Ne, None) => format!("{col} != {p}"),
        // gt / sa: strictly after the whole range.
        (SearchPrefix::Gt | SearchPrefix::Sa, Some(m)) => format!("{col} >= {}", end(m)),
        (SearchPrefix::Gt | SearchPrefix::Sa, None) => format!("{col} > {p}"),
        // lt / eb: strictly before the whole range.
        (SearchPrefix::Lt | SearchPrefix::Eb, _) => format!("{col} < {p}"),
        (SearchPrefix::Ge, _) => format!("{col} >= {p}"),
        (SearchPrefix::Le, Some(m)) => format!("{col} < {}", end(m)),
        (SearchPrefix::Le, None) => format!("{col} <= {p}"),
        (SearchPrefix::Ap, _) => {
            let m = match precision {
                DateValuePrecision::Year => "1 year",
                DateValuePrecision::Month => "1 month",
                DateValuePrecision::Day => "1 day",
                DateValuePrecision::Minute => "10 minutes",
                DateValuePrecision::Second | DateValuePrecision::Fraction(_) => "10 seconds",
            };
            format!(
                "{col} BETWEEN datetime(?{param_num}, '-{m}') AND datetime(?{param_num}, '+{m}')"
            )
        }
    };
    Some((sql, start))
}

/// [`date_condition`] for callers whose return type promises exactly one bind
/// parameter: a value that is not a date becomes a condition that matches
/// nothing and still refers to `?param_num`, so the numbering of the
/// parameters around it is undisturbed.
pub(crate) fn date_condition_or_nothing(
    column: &str,
    prefix: SearchPrefix,
    value: &str,
    param_num: usize,
) -> (String, String) {
    date_condition(column, prefix, value, param_num)
        .unwrap_or_else(|| (format!("(1 = 0 AND ?{param_num} IS NULL)"), String::new()))
}

/// SQL for `expr` (a date/time TEXT value or bind parameter) with its
/// fractional seconds cut to at most three digits — truncated, never rounded
/// — and everything after the digits (a timezone offset, `Z`, nothing) kept.
///
/// `2026-09-06T20:35:32.364567890+00:00` becomes `2026-09-06T20:35:32.364+00:00`;
/// `2016-01-23T13:07:42.5-04:00` and values without a fraction are unchanged.
/// SQLite has no regular expressions, so the digit run after the dot is
/// measured by stripping leading digits with `ltrim`.
fn truncated_to_millis(expr: &str) -> String {
    let rest = format!("substr({expr}, instr({expr}, '.') + 1)");
    let tail = format!("ltrim({rest}, '0123456789')");
    format!(
        "CASE WHEN instr({expr}, '.') = 0 THEN {expr} \
         ELSE substr({expr}, 1, instr({expr}, '.')) \
         || substr({rest}, 1, min(3, length({rest}) - length({tail}))) \
         || {tail} END"
    )
}

/// Handles date parameter SQL generation.
pub struct DateHandler;

impl DateHandler {
    /// Builds SQL for a date parameter value.
    ///
    /// Date comparisons respect the precision of the input:
    /// - "2024" matches the entire year
    /// - "2024-01" matches the entire month
    /// - "2024-01-15" matches the entire day
    ///
    /// A value that is not a date yields `1 = 0`, the match-nothing fragment
    /// the composite handler already uses, with no bind parameter.
    pub fn build_sql(value: &SearchValue, param_offset: usize) -> SqlFragment {
        let param_num = param_offset + 1;
        match date_condition("value_date", value.prefix, &value.value, param_num) {
            Some((sql, bound)) => SqlFragment::with_params(sql, vec![SqlParam::string(bound)]),
            None => SqlFragment::new("1 = 0"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sql_and_param(prefix: SearchPrefix, value: &str) -> (String, String) {
        date_condition("value_date", prefix, value, 1).expect("a valid date value")
    }

    #[test]
    fn eq_day_is_a_normalized_half_open_range() {
        let (sql, param) = sql_and_param(SearchPrefix::Eq, "1995-10-02");
        assert_eq!(
            sql,
            "(datetime(value_date) >= datetime(?1) AND datetime(value_date) < datetime(?1, '+1 day'))"
        );
        assert_eq!(param, "1995-10-02T00:00:00");
    }

    #[test]
    fn eq_full_precision_is_normalized_equality_not_an_empty_range() {
        let (sql, param) = sql_and_param(SearchPrefix::Eq, "2016-01-23T13:07:42-04:00");
        assert_eq!(sql, "datetime(value_date) = datetime(?1)");
        // Bound as the UTC instant the value names, not as the client's text.
        assert_eq!(param, "2016-01-23T17:07:42Z");
    }

    #[test]
    fn ge_includes_the_named_day_itself() {
        let (sql, param) = sql_and_param(SearchPrefix::Ge, "1995-10-02");
        assert_eq!(sql, "datetime(value_date) >= datetime(?1)");
        assert_eq!(param, "1995-10-02T00:00:00");
    }

    #[test]
    fn gt_starts_strictly_after_the_day() {
        let (sql, _) = sql_and_param(SearchPrefix::Gt, "2024-01-15");
        assert_eq!(sql, "datetime(value_date) >= datetime(?1, '+1 day')");
    }

    #[test]
    fn lt_excludes_the_boundary_day() {
        let (sql, param) = sql_and_param(SearchPrefix::Lt, "1996-01-01");
        assert_eq!(sql, "datetime(value_date) < datetime(?1)");
        assert_eq!(param, "1996-01-01T00:00:00");
    }

    #[test]
    fn le_reaches_the_end_of_the_named_day() {
        let (sql, _) = sql_and_param(SearchPrefix::Le, "2024-01-15");
        assert_eq!(sql, "datetime(value_date) < datetime(?1, '+1 day')");
    }

    #[test]
    fn year_and_month_bounds_are_datetime_parseable() {
        let (_, year) = sql_and_param(SearchPrefix::Eq, "1995");
        assert_eq!(year, "1995-01-01T00:00:00");
        let (sql, month) = sql_and_param(SearchPrefix::Eq, "1995-10");
        assert_eq!(month, "1995-10-01T00:00:00");
        assert!(sql.contains("'+1 month'"));
    }

    #[test]
    fn ap_scales_with_precision() {
        let (sql, param) = sql_and_param(SearchPrefix::Ap, "2024-01-15");
        assert!(sql.contains("BETWEEN datetime(?1, '-1 day') AND datetime(?1, '+1 day')"));
        assert_eq!(param, "2024-01-15T00:00:00");
    }

    #[test]
    fn build_sql_binds_exactly_one_parameter() {
        // The multi-value caller advances the offset by one per value, so eq
        // must not consume two slots.
        let value = SearchValue::new(SearchPrefix::Eq, "2024-01-15");
        let frag = DateHandler::build_sql(&value, 0);
        assert_eq!(frag.params.len(), 1);
    }

    #[test]
    fn eq_millisecond_precision_keeps_the_milliseconds() {
        let (sql, param) = sql_and_param(SearchPrefix::Eq, "2026-09-06T08:44:27.804Z");
        assert!(
            sql.starts_with("strftime('%Y-%m-%d %H:%M:%f', ")
                && sql.contains(" = strftime('%Y-%m-%d %H:%M:%f', "),
            "both sides keep milliseconds: {sql}"
        );
        assert!(
            !sql.contains("datetime("),
            "no second-precision fold: {sql}"
        );
        assert_eq!(param, "2026-09-06T08:44:27.804Z");

        // An offset is folded and surplus fraction digits are cut, never
        // rounded, before the value is bound.
        let (_, param) = sql_and_param(SearchPrefix::Eq, "2021-11-10T16:48:57.246958-08:00");
        assert_eq!(param, "2021-11-11T00:48:57.246Z");
    }

    /// The SQL truncation, evaluated by SQLite on the shapes that occur:
    /// nanoseconds (`last_updated`), a short fraction with an offset
    /// (`value_date` as the resource carried it), and no fraction at all.
    #[test]
    fn truncation_cuts_fraction_digits_and_keeps_the_offset() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory sqlite");
        let truncate = |value: &str| -> String {
            conn.query_row(
                &format!("SELECT {}", truncated_to_millis("?1")),
                rusqlite::params![value],
                |row| row.get::<_, String>(0),
            )
            .expect("evaluate truncation")
        };
        assert_eq!(
            truncate("2026-09-06T20:35:32.364567890+00:00"),
            "2026-09-06T20:35:32.364+00:00"
        );
        assert_eq!(
            truncate("2026-09-06T20:35:32.9996Z"),
            "2026-09-06T20:35:32.999Z"
        );
        assert_eq!(
            truncate("2016-01-23T13:07:42.5-04:00"),
            "2016-01-23T13:07:42.5-04:00"
        );
        assert_eq!(truncate("2016-01-23T13:07:42.25"), "2016-01-23T13:07:42.25");
        assert_eq!(
            truncate("2016-01-23T13:07:42-04:00"),
            "2016-01-23T13:07:42-04:00"
        );
        assert_eq!(truncate("1995-10-02"), "1995-10-02");
    }

    #[test]
    fn second_precision_still_covers_the_whole_second() {
        let (sql, _) = sql_and_param(SearchPrefix::Eq, "2026-09-06T08:44:27Z");
        assert_eq!(sql, "datetime(value_date) = datetime(?1)");
        // A negative offset is not fractional seconds.
        let (sql, _) = sql_and_param(SearchPrefix::Eq, "2026-09-06T04:44:27-04:00");
        assert_eq!(sql, "datetime(value_date) = datetime(?1)");
    }

    /// Evaluates the generated condition in SQLite itself against one stored
    /// `last_updated` value (the `Utc::now().to_rfc3339()` shape the resources
    /// table holds), returning whether the search value matches it.
    fn sqlite_matches(prefix: SearchPrefix, search: &str, stored: &str) -> bool {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory sqlite");
        let (sql, bound) = date_condition("?2", prefix, search, 1).expect("a valid date value");
        conn.query_row(
            &format!("SELECT {sql}"),
            rusqlite::params![bound, stored],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or_else(|e| panic!("evaluating `{sql}`: {e}"))
    }

    /// The Inferno US Core `_lastUpdated` case: a transaction Bundle writes
    /// several resources within one second, and a search for one resource's
    /// exact `meta.lastUpdated` must not return its neighbours from the same
    /// second.
    #[test]
    fn millisecond_precision_evaluates_at_millisecond_in_sqlite() {
        let stored = "2026-09-06T08:44:27.828123+00:00";

        assert!(
            !sqlite_matches(SearchPrefix::Eq, "2026-09-06T08:44:27.804Z", stored),
            "a different millisecond in the same second is not a match"
        );
        assert!(sqlite_matches(
            SearchPrefix::Eq,
            "2026-09-06T08:44:27.828Z",
            stored
        ));
        assert!(
            sqlite_matches(SearchPrefix::Eq, "2026-09-06T08:44:27Z", stored),
            "a second-precision search still covers the whole second"
        );
        assert!(
            sqlite_matches(SearchPrefix::Eq, "2026-09-06T04:44:27.828-04:00", stored),
            "timezone offsets still fold to UTC"
        );

        // Truncated, not rounded: `.828567` is `.828` to the client (that is
        // what `meta.lastUpdated` says), so `.829` must not match it — and a
        // fraction that would round up into the next second must not either.
        let rounds_up = "2026-09-06T08:44:27.828567+00:00";
        assert!(sqlite_matches(
            SearchPrefix::Eq,
            "2026-09-06T08:44:27.828Z",
            rounds_up
        ));
        assert!(!sqlite_matches(
            SearchPrefix::Eq,
            "2026-09-06T08:44:27.829Z",
            rounds_up
        ));
        let next_second = "2026-09-06T08:44:27.9996+00:00";
        assert!(sqlite_matches(
            SearchPrefix::Eq,
            "2026-09-06T08:44:27.999Z",
            next_second
        ));
        assert!(!sqlite_matches(
            SearchPrefix::Eq,
            "2026-09-06T08:44:28.000Z",
            next_second
        ));

        assert!(sqlite_matches(
            SearchPrefix::Ne,
            "2026-09-06T08:44:27.804Z",
            stored
        ));
        assert!(!sqlite_matches(
            SearchPrefix::Ne,
            "2026-09-06T08:44:27.828Z",
            stored
        ));
        assert!(sqlite_matches(
            SearchPrefix::Ge,
            "2026-09-06T08:44:27.828Z",
            stored
        ));
        assert!(sqlite_matches(
            SearchPrefix::Gt,
            "2026-09-06T08:44:27.827Z",
            stored
        ));
        assert!(!sqlite_matches(
            SearchPrefix::Gt,
            "2026-09-06T08:44:27.828Z",
            stored
        ));
        assert!(sqlite_matches(
            SearchPrefix::Lt,
            "2026-09-06T08:44:27.829Z",
            stored
        ));
        assert!(!sqlite_matches(
            SearchPrefix::Lt,
            "2026-09-06T08:44:27.828Z",
            stored
        ));
        assert!(sqlite_matches(
            SearchPrefix::Le,
            "2026-09-06T08:44:27.828Z",
            stored
        ));
        assert!(
            sqlite_matches(SearchPrefix::Ap, "2026-09-06T08:44:30.000Z", stored),
            "approximate keeps its ±10-second window"
        );
    }
}

#[cfg(test)]
mod prefix_coverage_tests {
    use super::*;

    fn sql(prefix: SearchPrefix, value: &str) -> String {
        date_condition("value_date", prefix, value, 1)
            .expect("a valid date value")
            .0
    }

    #[test]
    fn ne_day_is_the_complement_of_the_range() {
        assert_eq!(
            sql(SearchPrefix::Ne, "1995-10-02"),
            "(datetime(value_date) < datetime(?1) OR datetime(value_date) >= datetime(?1, '+1 day'))"
        );
    }

    #[test]
    fn ne_full_precision_is_normalized_inequality() {
        assert_eq!(
            sql(SearchPrefix::Ne, "2016-01-23T13:07:42-04:00"),
            "datetime(value_date) != datetime(?1)"
        );
    }

    #[test]
    fn sa_and_eb_mirror_gt_and_lt() {
        assert_eq!(
            sql(SearchPrefix::Sa, "1995-10-02"),
            "datetime(value_date) >= datetime(?1, '+1 day')"
        );
        assert_eq!(
            sql(SearchPrefix::Eb, "1995-10-02"),
            "datetime(value_date) < datetime(?1)"
        );
    }

    #[test]
    fn full_precision_single_bounds() {
        let instant = "2016-01-23T13:07:42Z";
        assert_eq!(
            sql(SearchPrefix::Gt, instant),
            "datetime(value_date) > datetime(?1)"
        );
        assert_eq!(
            sql(SearchPrefix::Ge, instant),
            "datetime(value_date) >= datetime(?1)"
        );
        assert_eq!(
            sql(SearchPrefix::Lt, instant),
            "datetime(value_date) < datetime(?1)"
        );
        assert_eq!(
            sql(SearchPrefix::Le, instant),
            "datetime(value_date) <= datetime(?1)"
        );
    }

    #[test]
    fn ap_at_finer_precisions_scales_its_window() {
        assert!(sql(SearchPrefix::Ap, "2016-01-23T13:07:42Z").contains("'-10 seconds'"));
        assert!(sql(SearchPrefix::Ap, "1995").contains("'-1 year'"));
        assert!(sql(SearchPrefix::Ap, "1995-10").contains("'-1 month'"));
    }

    #[test]
    fn aliased_columns_pass_through() {
        let (sql, bound) =
            date_condition("t3.value_date", SearchPrefix::Eq, "1995-10-02", 4).unwrap();
        assert_eq!(
            sql,
            "(datetime(t3.value_date) >= datetime(?4) AND datetime(t3.value_date) < datetime(?4, '+1 day'))"
        );
        assert_eq!(bound, "1995-10-02T00:00:00");
    }
}

#[cfg(test)]
mod shared_grammar_tests {
    use super::*;
    use crate::search::StorageResolution;

    #[test]
    fn a_value_that_is_not_a_date_has_no_condition() {
        // `datetime('2024-02-30')` is `2024-03-01 00:00:00`: SQLite searched
        // for the wrong day instead of failing (#1295).
        for value in [
            "2024-02-30",
            "2024-13-45",
            "not-a-date",
            "2013-04-05T10",
            "",
        ] {
            for prefix in [SearchPrefix::Eq, SearchPrefix::Ne, SearchPrefix::Lt] {
                assert_eq!(
                    date_condition("value_date", prefix, value, 1),
                    None,
                    "{prefix}{value}"
                );
            }
        }
    }

    #[test]
    fn handlers_match_nothing_for_a_value_that_is_not_a_date() {
        // `ne` included: invalid input never returns more than valid input could.
        let frag = DateHandler::build_sql(&SearchValue::new(SearchPrefix::Ne, "2024-02-30"), 0);
        assert_eq!(frag.sql, "1 = 0");
        assert!(frag.params.is_empty());

        // The single-bind callers keep their parameter, so the numbering of
        // the binds around it holds.
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory sqlite");
        let (sql, bound) =
            date_condition_or_nothing("value_date", SearchPrefix::Ne, "2024-02-30", 1);
        assert_eq!(sql, "(1 = 0 AND ?1 IS NULL)");
        let matched: bool = conn
            .query_row(&format!("SELECT {sql}"), rusqlite::params![bound], |row| {
                row.get(0)
            })
            .expect("evaluate");
        assert!(!matched);
    }

    #[test]
    fn minute_precision_is_a_one_minute_range() {
        let (sql, param) =
            date_condition("value_date", SearchPrefix::Eq, "2013-04-05T09:20", 1).unwrap();
        assert_eq!(
            sql,
            "(datetime(value_date) >= datetime(?1) AND datetime(value_date) < datetime(?1, '+1 minute'))"
        );
        assert_eq!(param, "2013-04-05T09:20:00Z");
    }

    #[test]
    fn what_datetime_cannot_read_is_bound_in_a_form_it_can() {
        // A leap second, and a `+` that form decoding turned into a space
        // (#1296): `datetime()` returns NULL for both as written.
        let (_, param) =
            date_condition("value_date", SearchPrefix::Eq, "2016-12-31T23:59:60Z", 1).unwrap();
        assert_eq!(param, "2017-01-01T00:00:00Z");
        let (_, param) = date_condition(
            "value_date",
            SearchPrefix::Eq,
            "2013-04-05T18:50:00 05:30",
            1,
        )
        .unwrap();
        assert_eq!(param, "2013-04-05T13:20:00Z");
    }

    /// Evaluates the generated condition in SQLite against one stored value.
    fn sqlite_matches(prefix: SearchPrefix, search: &str, stored: &str) -> bool {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory sqlite");
        let (sql, bound) = date_condition("?2", prefix, search, 1).expect("a valid date value");
        conn.query_row(
            &format!("SELECT {sql}"),
            rusqlite::params![bound, stored],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or_else(|e| panic!("evaluating `{sql}`: {e}"))
    }

    /// This handler keeps its own single-bind SQL rather than translating
    /// `DatePredicate`, so hold the two against each other: for every prefix
    /// the shared layer defines, at every precision, SQLite must answer what
    /// the shared predicate answers.
    #[test]
    fn sql_agrees_with_the_shared_predicate() {
        let searches = [
            "2013",
            "2013-04",
            "2013-12",
            "2013-04-05",
            "2013-04-05T09:20",
            "2013-04-05T09:20-04:00",
            "2013-04-05T23:30:00-04:00",
            "2013-04-06T03:30:00Z",
            "2013-04-06T09:00:00+05:30",
            "2013-04-06T09:00:00 05:30",
            "2013-04-06T03:30:00.123Z",
            "2013-04-05T23:30:00.123456-04:00",
        ];
        let stored = [
            "2012-12-31T23:59:59Z",
            "2013-01-01",
            "2013-04-05",
            "2013-04-05T09:19:59-04:00",
            "2013-04-05T09:20:00-04:00",
            "2013-04-05T09:20:59.999-04:00",
            "2013-04-05T09:21:00-04:00",
            "2013-04-05T09:20:30Z",
            "2013-04-05T23:29:59.999-04:00",
            "2013-04-05T23:30:00-04:00",
            "2013-04-05T23:30:00.123-04:00",
            "2013-04-05T23:30:00.123999-04:00",
            "2013-04-05T23:30:00.124-04:00",
            "2013-04-05T23:30:01-04:00",
            "2013-04-06",
            "2013-12-31T23:59:59.999Z",
            "2014-01-01T00:00:00Z",
        ];
        for search in searches {
            let value = FhirDateValue::parse(search).unwrap();
            for prefix in [
                SearchPrefix::Eq,
                SearchPrefix::Ne,
                SearchPrefix::Gt,
                SearchPrefix::Lt,
                SearchPrefix::Ge,
                SearchPrefix::Le,
                SearchPrefix::Sa,
                SearchPrefix::Eb,
            ] {
                let predicate = value.predicate(prefix, StorageResolution::Millis).unwrap();
                for text in stored {
                    // A stored date-only value is its first instant, in UTC.
                    let point = FhirDateValue::parse(text).unwrap().start;
                    assert_eq!(
                        sqlite_matches(prefix, search, text),
                        predicate.matches(point),
                        "{prefix}{search} against stored {text}"
                    );
                }
            }
        }
    }

    /// The one place this handler is narrower than the shared range: a
    /// fraction is compared at the millisecond whatever its digit count.
    #[test]
    fn short_fractions_compare_at_the_millisecond() {
        assert!(sqlite_matches(
            SearchPrefix::Eq,
            "2013-04-06T03:30:00.5Z",
            "2013-04-06T03:30:00.500Z"
        ));
        assert!(!sqlite_matches(
            SearchPrefix::Eq,
            "2013-04-06T03:30:00.5Z",
            "2013-04-06T03:30:00.55Z"
        ));
    }
}
