//! Date parameter SQL handler.

use crate::types::{DatePrecision, SearchPrefix, SearchValue};

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
/// keeps `SS.SSS` (SQLite carries dates as integer milliseconds internally, so
/// this is exact) and still folds timezone offsets to UTC. The approximate
/// prefix keeps `datetime()`: its ±10-second window is coarser than a second
/// anyway.
///
/// Returns the SQL and the value to bind for its (single) parameter.
pub(crate) fn date_condition(
    column: &str,
    prefix: SearchPrefix,
    value: &str,
    param_num: usize,
) -> (String, String) {
    let precision = DatePrecision::from_date_string(value);

    // The range start as a full datetime (always parseable by datetime()),
    // and the SQL modifier that derives the range end for partial precisions.
    let (start, bump) = match precision {
        DatePrecision::Year => (format!("{}-01-01T00:00:00", &value[..4]), Some("+1 year")),
        DatePrecision::Month => (format!("{}-01T00:00:00", &value[..7]), Some("+1 month")),
        DatePrecision::Day => (format!("{value}T00:00:00"), Some("+1 day")),
        _ => (value.to_string(), None),
    };

    // Decided on the value's own text rather than `DatePrecision`, which reads
    // a negative offset (`...:42-04:00`) as extra length and calls a plain
    // second-precision value millisecond.
    let has_fractional_seconds = value
        .split_once('T')
        .is_some_and(|(_, time)| time.contains('.'));
    let normalize = |expr: &str| {
        if has_fractional_seconds && prefix != SearchPrefix::Ap {
            format!("strftime('%Y-%m-%d %H:%M:%f', {expr})")
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
                DatePrecision::Year => "1 year",
                DatePrecision::Month => "1 month",
                DatePrecision::Day => "1 day",
                DatePrecision::Hour => "1 hour",
                DatePrecision::Minute => "10 minutes",
                DatePrecision::Second | DatePrecision::Millisecond => "10 seconds",
            };
            format!(
                "{col} BETWEEN datetime(?{param_num}, '-{m}') AND datetime(?{param_num}, '+{m}')"
            )
        }
    };
    (sql, start)
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
    pub fn build_sql(value: &SearchValue, param_offset: usize) -> SqlFragment {
        let param_num = param_offset + 1;
        let (sql, bound) = date_condition("value_date", value.prefix, &value.value, param_num);
        SqlFragment::with_params(sql, vec![SqlParam::string(bound)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sql_and_param(prefix: SearchPrefix, value: &str) -> (String, String) {
        date_condition("value_date", prefix, value, 1)
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
        assert_eq!(param, "2016-01-23T13:07:42-04:00");
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
        assert_eq!(
            sql,
            "strftime('%Y-%m-%d %H:%M:%f', value_date) = strftime('%Y-%m-%d %H:%M:%f', ?1)"
        );
        assert_eq!(param, "2026-09-06T08:44:27.804Z");
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
        let (sql, bound) = date_condition("?2", prefix, search, 1);
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
        date_condition("value_date", prefix, value, 1).0
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
        let (sql, bound) = date_condition("t3.value_date", SearchPrefix::Eq, "1995-10-02", 4);
        assert_eq!(
            sql,
            "(datetime(t3.value_date) >= datetime(?4) AND datetime(t3.value_date) < datetime(?4, '+1 day'))"
        );
        assert_eq!(bound, "1995-10-02T00:00:00");
    }
}
