//! Date parameter SQL handler.

use chrono::{DateTime, Datelike, Utc};

use crate::search::{DatePredicate, DateValuePrecision, FhirDateValue, StorageResolution};
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
/// Fractional-precision values therefore go through `strftime('%f')`, which
/// keeps `SS.SSS` and still folds timezone offsets to UTC. Both sides are
/// first cut to three fractional digits in SQL ([`truncated_to_millis`]):
/// `last_updated` holds nanoseconds, which SQLite would *round* to the
/// nearest millisecond while `meta.lastUpdated` (what the client searches
/// with) truncates — off by one millisecond half the time.
///
/// `ap` is the shared window ([`FhirDateValue::predicate`], #1390): the
/// value's range widened by a tenth of its distance from `now`. Its bounds
/// are the bound range start moved by a whole number of milliseconds, written
/// as `'±S seconds'` modifiers computed here — numbers, never client text —
/// so it still needs one parameter, and it is compared at the millisecond
/// because the margin rarely falls on a whole second.
///
/// The search value itself is read by [`FhirDateValue`], the grammar every
/// backend shares, and what is bound is a canonical UTC string built from the
/// parsed instant — never the client's text. `datetime()` is lenient in ways
/// the grammar is not: it rolled `2024-02-30` over to March 1st and searched
/// for that (#1295), and it returns NULL for a leap second or for an offset
/// whose `+` was form-decoded into a space (#1296).
///
/// Other than for `ap`, this does not translate [`DatePredicate`]: the
/// single-bind SQL above predates it, and tests hold the two against each
/// other. A one- or two-digit fraction uses `strftime()` with a
/// fractional-second modifier for its upper bound; `.5` means `[.500, .600)`.
/// Fractions with three or more digits compare at the millisecond after
/// truncation. If the modifier crosses year 9999, `strftime()` returns NULL,
/// so the parsed range end provides the bounded fallback.
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
    now: DateTime<Utc>,
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
    // Fractions with three or more digits occupy one stored millisecond.
    let bump = match precision {
        DateValuePrecision::Year => Some("+1 year"),
        DateValuePrecision::Month => Some("+1 month"),
        DateValuePrecision::Day => Some("+1 day"),
        DateValuePrecision::Minute => Some("+1 minute"),
        DateValuePrecision::Fraction(1) => Some("+0.1 seconds"),
        DateValuePrecision::Fraction(2) => Some("+0.01 seconds"),
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

    if prefix == SearchPrefix::Ap {
        return Some((approx_condition(column, &parsed, param_num, now), start));
    }

    let normalize = |expr: &str| {
        if has_fractional_seconds {
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
    let end = |m: &str| {
        if has_fractional_seconds {
            let bounded_end = parsed
                .range_at(StorageResolution::Millis)
                .1
                .format("%Y-%m-%d %H:%M:%S%.3f");
            format!(
                "COALESCE(strftime('%Y-%m-%d %H:%M:%f', {}, '{m}'), '{bounded_end}')",
                truncated_to_millis(&format!("?{param_num}")),
            )
        } else {
            format!("datetime(?{param_num}, '{m}')")
        }
    };

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
        (SearchPrefix::Ap, _) => unreachable!("`ap` returned above"),
    };
    Some((sql, start))
}

/// The `ap` condition of [`date_condition`]: `[ge, lt)` of the shared window,
/// each bound written as the bound parameter (`parsed`'s range start, cut to
/// the millisecond) plus a whole number of milliseconds, and compared at the
/// millisecond on both sides.
///
/// A bound outside the years SQLite's date functions accept (0000–9999) —
/// where the shared window saturates, or a far-off value's margin reaches —
/// is dropped: nothing stored lies beyond it, and SQLite would make it NULL.
fn approx_condition(
    column: &str,
    parsed: &FhirDateValue,
    param_num: usize,
    now: DateTime<Utc>,
) -> String {
    let DatePredicate::Within { ge, lt } =
        parsed.predicate(SearchPrefix::Ap, StorageResolution::Millis, now)
    else {
        unreachable!("the shared `ap` predicate is always a window");
    };
    let (bound, _) = parsed.range_at(StorageResolution::Millis);
    let shifted = |expr: &str, offset_ms: i64| {
        let sign = if offset_ms < 0 { '-' } else { '+' };
        let offset_ms = offset_ms.unsigned_abs();
        format!(
            "strftime('%Y-%m-%d %H:%M:%f', {expr}, '{sign}{}.{:03} seconds')",
            offset_ms / 1000,
            offset_ms % 1000
        )
    };
    let col = format!(
        "strftime('%Y-%m-%d %H:%M:%f', {})",
        truncated_to_millis(column)
    );
    let param = format!("?{param_num}");
    let in_range = |t: DateTime<Utc>| (0..=9999).contains(&t.year());

    let mut sides = Vec::new();
    if in_range(ge) {
        sides.push(format!(
            "{col} >= {}",
            shifted(&param, (ge - bound).num_milliseconds())
        ));
    }
    if in_range(lt) {
        sides.push(format!(
            "{col} < {}",
            shifted(&param, (lt - bound).num_milliseconds())
        ));
    }
    if sides.is_empty() {
        // Still refer to the parameter, so the numbering around it holds.
        sides.push(format!("{col} IS NOT NULL AND {param} IS NOT NULL"));
    }
    format!("({})", sides.join(" AND "))
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
    now: DateTime<Utc>,
) -> (String, String) {
    date_condition(column, prefix, value, param_num, now)
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
    /// `now` is the instant an `ap` window is measured from.
    ///
    /// A value that is not a date yields `1 = 0`, the match-nothing fragment
    /// the composite handler already uses, with no bind parameter.
    pub fn build_sql(value: &SearchValue, param_offset: usize, now: DateTime<Utc>) -> SqlFragment {
        let param_num = param_offset + 1;
        match date_condition("value_date", value.prefix, &value.value, param_num, now) {
            Some((sql, bound)) => SqlFragment::with_params(sql, vec![SqlParam::string(bound)]),
            None => SqlFragment::new("1 = 0"),
        }
    }
}

/// The `now` the tests measure `ap` windows from.
#[cfg(test)]
fn test_now() -> DateTime<Utc> {
    "2026-01-01T00:00:00Z".parse().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sql_and_param(prefix: SearchPrefix, value: &str) -> (String, String) {
        date_condition("value_date", prefix, value, 1, test_now()).expect("a valid date value")
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
    fn ap_is_the_shared_window_around_the_bound_range_start() {
        // `2016` ends 9 years (3287 days) before `now`: a margin of 328.7 days
        // either side of the (leap) year, as offsets from the one parameter.
        let (sql, param) = sql_and_param(SearchPrefix::Ap, "2016");
        let col = format!(
            "strftime('%Y-%m-%d %H:%M:%f', {})",
            truncated_to_millis("value_date")
        );
        assert_eq!(
            sql,
            format!(
                "({col} >= strftime('%Y-%m-%d %H:%M:%f', ?1, '-28399680.000 seconds') \
                 AND {col} < strftime('%Y-%m-%d %H:%M:%f', ?1, '+60022080.000 seconds'))"
            )
        );
        assert_eq!(param, "2016-01-01T00:00:00");
    }

    #[test]
    fn ap_matches_what_the_shared_window_holds() {
        // Past: 328.7 days either side of 2016.
        assert!(sqlite_matches(SearchPrefix::Ap, "2016", "2015-03-01"));
        assert!(!sqlite_matches(SearchPrefix::Ap, "2016", "2015-01-15"));
        assert!(sqlite_matches(
            SearchPrefix::Ap,
            "2016",
            "2017-06-01T10:00:00Z"
        ));
        assert!(!sqlite_matches(SearchPrefix::Ap, "2016", "2017-12-15"));
        // `now` inside the range: no margin, the same as `eq`.
        assert!(sqlite_matches(
            SearchPrefix::Ap,
            "2026",
            "2026-12-31T23:59:59.999Z"
        ));
        assert!(!sqlite_matches(SearchPrefix::Ap, "2026", "2027-01-01"));
        assert!(!sqlite_matches(
            SearchPrefix::Ap,
            "2026",
            "2025-12-31T23:59:59Z"
        ));
    }

    #[test]
    fn ap_binds_exactly_one_parameter() {
        let value = SearchValue::new(SearchPrefix::Ap, "2024-01-15");
        let frag = DateHandler::build_sql(&value, 0, test_now());
        assert_eq!(frag.params.len(), 1);
    }

    #[test]
    fn ap_drops_a_bound_past_the_years_sqlite_reads() {
        // `9999` is ~7973 years after `now`: its upper bound falls past 9999.
        let (sql, _) = sql_and_param(SearchPrefix::Ap, "9999");
        assert!(sql.contains(">="), "{sql}");
        assert!(!sql.contains(" < "), "{sql}");
        assert!(sqlite_matches(SearchPrefix::Ap, "9999", "9999-12-31"));
    }

    #[test]
    fn build_sql_binds_exactly_one_parameter() {
        // The multi-value caller advances the offset by one per value, so eq
        // must not consume two slots.
        for search in [
            "2024-01-15",
            "2024-01-15T23:59:59.9Z",
            "2024-01-15T23:59:59.99Z",
        ] {
            let value = SearchValue::new(SearchPrefix::Eq, search);
            let frag = DateHandler::build_sql(&value, 0, test_now());
            assert_eq!(frag.params.len(), 1, "{search}");
        }
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
        sqlite_matches_at(prefix, search, stored, test_now())
    }

    /// [`sqlite_matches`] with `ap` measured from `now`.
    fn sqlite_matches_at(
        prefix: SearchPrefix,
        search: &str,
        stored: &str,
        now: DateTime<Utc>,
    ) -> bool {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory sqlite");
        let (sql, bound) =
            date_condition("?2", prefix, search, 1, now).expect("a valid date value");
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
        // `ap` is compared at the millisecond too: 100 s from `now`, its
        // window is ±10 s; with `now` on the value, just that millisecond.
        let now = "2026-09-06T08:46:10Z".parse().unwrap();
        assert!(sqlite_matches_at(
            SearchPrefix::Ap,
            "2026-09-06T08:44:30.000Z",
            stored,
            now
        ));
        assert!(!sqlite_matches_at(
            SearchPrefix::Ap,
            "2026-09-06T08:44:40.000Z",
            stored,
            now
        ));
        let at_value = "2026-09-06T08:44:27.828Z";
        assert!(sqlite_matches_at(
            SearchPrefix::Ap,
            at_value,
            stored,
            at_value.parse().unwrap()
        ));
        assert!(!sqlite_matches_at(
            SearchPrefix::Ap,
            "2026-09-06T08:44:27.827Z",
            stored,
            at_value.parse().unwrap()
        ));
    }
}

#[cfg(test)]
mod prefix_coverage_tests {
    use super::*;

    fn sql(prefix: SearchPrefix, value: &str) -> String {
        date_condition("value_date", prefix, value, 1, test_now())
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
    fn ap_with_now_in_the_range_is_the_range_at_the_millisecond() {
        let (sql, _) = date_condition(
            "value_date",
            SearchPrefix::Ap,
            "2016-01-23T13:07:42Z",
            1,
            "2016-01-23T13:07:42.5Z".parse().unwrap(),
        )
        .unwrap();
        let col = format!(
            "strftime('%Y-%m-%d %H:%M:%f', {})",
            truncated_to_millis("value_date")
        );
        assert_eq!(
            sql,
            format!(
                "({col} >= strftime('%Y-%m-%d %H:%M:%f', ?1, '+0.000 seconds') \
                 AND {col} < strftime('%Y-%m-%d %H:%M:%f', ?1, '+1.000 seconds'))"
            )
        );
    }

    #[test]
    fn aliased_columns_pass_through() {
        let (sql, bound) = date_condition(
            "t3.value_date",
            SearchPrefix::Eq,
            "1995-10-02",
            4,
            test_now(),
        )
        .unwrap();
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
                    date_condition("value_date", prefix, value, 1, test_now()),
                    None,
                    "{prefix}{value}"
                );
            }
        }
    }

    #[test]
    fn handlers_match_nothing_for_a_value_that_is_not_a_date() {
        // `ne` included: invalid input never returns more than valid input could.
        let frag = DateHandler::build_sql(
            &SearchValue::new(SearchPrefix::Ne, "2024-02-30"),
            0,
            test_now(),
        );
        assert_eq!(frag.sql, "1 = 0");
        assert!(frag.params.is_empty());

        // The single-bind callers keep their parameter, so the numbering of
        // the binds around it holds.
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory sqlite");
        let (sql, bound) =
            date_condition_or_nothing("value_date", SearchPrefix::Ne, "2024-02-30", 1, test_now());
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
        let (sql, param) = date_condition(
            "value_date",
            SearchPrefix::Eq,
            "2013-04-05T09:20",
            1,
            test_now(),
        )
        .unwrap();
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
        let (_, param) = date_condition(
            "value_date",
            SearchPrefix::Eq,
            "2016-12-31T23:59:60Z",
            1,
            test_now(),
        )
        .unwrap();
        assert_eq!(param, "2017-01-01T00:00:00Z");
        let (_, param) = date_condition(
            "value_date",
            SearchPrefix::Eq,
            "2013-04-05T18:50:00 05:30",
            1,
            test_now(),
        )
        .unwrap();
        assert_eq!(param, "2013-04-05T13:20:00Z");
    }

    /// Evaluates the generated condition in SQLite against one stored value.
    fn sqlite_matches(prefix: SearchPrefix, search: &str, stored: &str) -> bool {
        sqlite_matches_at(prefix, search, stored, test_now())
    }

    /// [`sqlite_matches`] with `ap` measured from `now`.
    fn sqlite_matches_at(
        prefix: SearchPrefix,
        search: &str,
        stored: &str,
        now: DateTime<Utc>,
    ) -> bool {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory sqlite");
        let (sql, bound) =
            date_condition("?2", prefix, search, 1, now).expect("a valid date value");
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
    /// the shared predicate answers — `ap` included, from `now`s before,
    /// inside and after the searched ranges.
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
            "2013-04-06T03:30:00.5Z",
            "2013-04-06T03:30:00.50Z",
            "2013-04-06T03:30:00.99Z",
            "2013-04-06T03:30:00.00Z",
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
            "2013-04-06T03:30:00.499Z",
            "2013-04-06T03:30:00.500Z",
            "2013-04-06T03:30:00.509Z",
            "2013-04-06T03:30:00.550Z",
            "2013-04-06T03:30:00.599Z",
            "2013-04-06T03:30:00.600Z",
            "2013-04-06T03:30:00.990Z",
            "2013-04-06T03:30:00.999Z",
            "2013-04-06T03:30:01.000Z",
            "2013-04-05T23:30:01-04:00",
            "2013-04-06",
            "2013-12-31T23:59:59.999Z",
            "2014-01-01T00:00:00Z",
        ];
        for search in searches {
            let value = FhirDateValue::parse(search).unwrap();
            for (prefix, now) in [
                SearchPrefix::Eq,
                SearchPrefix::Ne,
                SearchPrefix::Gt,
                SearchPrefix::Lt,
                SearchPrefix::Ge,
                SearchPrefix::Le,
                SearchPrefix::Sa,
                SearchPrefix::Eb,
            ]
            .into_iter()
            .map(|prefix| (prefix, test_now()))
            .chain(
                [
                    "2013-04-01T00:00:00Z",
                    "2013-04-06T03:30:00Z",
                    "2013-04-10T00:00:00Z",
                    "2026-01-01T00:00:00Z",
                ]
                .map(|now| (SearchPrefix::Ap, now.parse().unwrap())),
            ) {
                let predicate = value.predicate(prefix, StorageResolution::Millis, now);
                for text in stored {
                    // A stored date-only value is its first instant, in UTC.
                    let point = FhirDateValue::parse(text).unwrap().start;
                    assert_eq!(
                        sqlite_matches_at(prefix, search, text, now),
                        predicate.matches(point),
                        "{prefix}{search} (now {now}) against stored {text}"
                    );
                }
            }
        }
    }

    #[test]
    fn short_fractions_cover_their_full_precision_range() {
        assert!(sqlite_matches(
            SearchPrefix::Eq,
            "2013-04-06T03:30:00.5Z",
            "2013-04-06T03:30:00.500Z"
        ));
        assert!(sqlite_matches(
            SearchPrefix::Eq,
            "2013-04-06T03:30:00.5Z",
            "2013-04-06T03:30:00.55Z"
        ));
        assert!(sqlite_matches(
            SearchPrefix::Eq,
            "2013-04-06T03:30:00.5Z",
            "2013-04-06T03:30:00.5996Z"
        ));
        assert!(!sqlite_matches(
            SearchPrefix::Eq,
            "2013-04-06T03:30:00.5Z",
            "2013-04-06T03:30:00.600Z"
        ));
        assert!(sqlite_matches(
            SearchPrefix::Eq,
            "2013-04-05T23:30:00.99-04:00",
            "2013-04-06T03:30:00.999Z"
        ));
        assert!(!sqlite_matches(
            SearchPrefix::Eq,
            "2013-04-05T23:30:00.99-04:00",
            "2013-04-06T03:30:01.000Z"
        ));
        assert!(sqlite_matches(
            SearchPrefix::Eq,
            "2013-04-06T23:59:59.99Z",
            "2013-04-06T23:59:59.999Z"
        ));
        assert!(!sqlite_matches(
            SearchPrefix::Eq,
            "2013-04-06T23:59:59.99Z",
            "2013-04-07T00:00:00.000Z"
        ));
    }

    #[test]
    fn short_fractions_at_last_supported_second_match_the_shared_predicate() {
        for search in ["9999-12-31T23:59:59.9Z", "9999-12-31T23:59:59.99Z"] {
            let value = FhirDateValue::parse(search).unwrap();
            for prefix in [
                SearchPrefix::Eq,
                SearchPrefix::Ne,
                SearchPrefix::Gt,
                SearchPrefix::Sa,
                SearchPrefix::Le,
            ] {
                let predicate = value.predicate(prefix, StorageResolution::Millis, test_now());
                for stored in [
                    "9999-12-31T23:59:59.950Z",
                    "9999-12-31T23:59:59.995Z",
                    "9999-12-31T23:59:59.998Z",
                    "9999-12-31T23:59:59.999Z",
                ] {
                    let point = FhirDateValue::parse(stored).unwrap().start;
                    assert_eq!(
                        sqlite_matches(prefix, search, stored),
                        predicate.matches(point),
                        "{prefix}{search} against stored {stored}"
                    );
                }
            }
        }
    }
}
