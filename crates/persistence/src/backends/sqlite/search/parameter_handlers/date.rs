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
/// `datetime()` truncates fractional seconds, so millisecond-precision values
/// compare at second precision — a match too many beats never matching.
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

    let col = format!("datetime({column})");
    let p = format!("datetime(?{param_num})");
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
}
