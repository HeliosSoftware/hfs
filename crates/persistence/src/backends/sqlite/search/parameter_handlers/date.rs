//! Date parameter SQL handler.

use crate::types::{DatePrecision, SearchPrefix, SearchValue};

use super::super::query_builder::{SqlFragment, SqlParam};

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
        let date_value = &value.value;
        let precision = DatePrecision::from_date_string(date_value);

        // Precision range [start, end). Comparators match against its
        // boundaries per the FHIR spec. When the value is full-precision the
        // range is degenerate (start == end); fall back to scalar comparison
        // so an exact instant still matches le/ge/eq.
        let (start, end) = Self::get_precision_range(date_value, precision);

        match value.prefix {
            SearchPrefix::Eq => Self::build_equals(date_value, precision, param_num),
            SearchPrefix::Ne => Self::build_not_equals(date_value, precision, param_num),
            // gt / sa: strictly after the whole range → value_date >= end.
            SearchPrefix::Gt | SearchPrefix::Sa if start != end => Self::cmp(">=", &end, param_num),
            SearchPrefix::Gt | SearchPrefix::Sa => Self::cmp(">", date_value, param_num),
            // lt / eb: strictly before the whole range → value_date < start.
            SearchPrefix::Lt | SearchPrefix::Eb if start != end => {
                Self::cmp("<", &start, param_num)
            }
            SearchPrefix::Lt | SearchPrefix::Eb => Self::cmp("<", date_value, param_num),
            SearchPrefix::Ge if start != end => Self::cmp(">=", &start, param_num),
            SearchPrefix::Ge => Self::cmp(">=", date_value, param_num),
            SearchPrefix::Le if start != end => Self::cmp("<", &end, param_num),
            SearchPrefix::Le => Self::cmp("<=", date_value, param_num),
            SearchPrefix::Ap => Self::build_approximately(date_value, precision, param_num),
        }
    }

    /// Builds a single-boundary date comparison `value_date {op} ?`.
    fn cmp(op: &str, bound: &str, param_num: usize) -> SqlFragment {
        SqlFragment::with_params(
            format!("value_date {} ?{}", op, param_num),
            vec![SqlParam::string(bound)],
        )
    }

    /// Equality - matches any date within the precision range.
    fn build_equals(date: &str, precision: DatePrecision, param_num: usize) -> SqlFragment {
        let (start, end) = Self::get_precision_range(date, precision);

        SqlFragment::with_params(
            format!(
                "value_date >= ?{} AND value_date < ?{}",
                param_num,
                param_num + 1
            ),
            vec![SqlParam::string(start), SqlParam::string(end)],
        )
    }

    /// Not equals - outside the precision range.
    fn build_not_equals(date: &str, precision: DatePrecision, param_num: usize) -> SqlFragment {
        let (start, end) = Self::get_precision_range(date, precision);

        SqlFragment::with_params(
            format!(
                "(value_date < ?{} OR value_date >= ?{})",
                param_num,
                param_num + 1
            ),
            vec![SqlParam::string(start), SqlParam::string(end)],
        )
    }

    /// Approximately equals - +/- based on precision.
    fn build_approximately(date: &str, precision: DatePrecision, param_num: usize) -> SqlFragment {
        // SQLite datetime functions for range calculation
        let modifier = match precision {
            DatePrecision::Year => "1 year",
            DatePrecision::Month => "1 month",
            DatePrecision::Day => "1 day",
            DatePrecision::Hour => "1 hour",
            DatePrecision::Minute => "10 minutes",
            DatePrecision::Second | DatePrecision::Millisecond => "10 seconds",
        };

        SqlFragment::with_params(
            format!(
                "value_date BETWEEN datetime(?{}, '-{}') AND datetime(?{}, '+{}')",
                param_num, modifier, param_num, modifier
            ),
            vec![SqlParam::string(date)],
        )
    }

    /// Gets the start and end of the range for a date at a given precision.
    fn get_precision_range(date: &str, precision: DatePrecision) -> (String, String) {
        match precision {
            DatePrecision::Year => {
                let year = &date[..4];
                (
                    format!("{}-01-01T00:00:00", year),
                    format!("{}-01-01T00:00:00", year.parse::<i32>().unwrap_or(0) + 1),
                )
            }
            DatePrecision::Month => {
                let (year, month) = (&date[..4], &date[5..7]);
                let year_num: i32 = year.parse().unwrap_or(0);
                let month_num: i32 = month.parse().unwrap_or(1);

                let (next_year, next_month) = if month_num >= 12 {
                    (year_num + 1, 1)
                } else {
                    (year_num, month_num + 1)
                };

                (
                    format!("{}-{:02}-01T00:00:00", year, month_num),
                    format!("{}-{:02}-01T00:00:00", next_year, next_month),
                )
            }
            DatePrecision::Day => (
                format!("{}T00:00:00", date),
                format!("{}T00:00:00", Self::add_day(date)),
            ),
            _ => {
                // For finer precisions, use the exact value
                (date.to_string(), date.to_string())
            }
        }
    }

    /// Adds one day to a date string.
    fn add_day(date: &str) -> String {
        // Simple date arithmetic - in production, use proper datetime library
        let parts: Vec<&str> = date.split('-').collect();
        if parts.len() >= 3 {
            let year: i32 = parts[0].parse().unwrap_or(0);
            let month: i32 = parts[1].parse().unwrap_or(1);
            let day: i32 = parts[2].parse().unwrap_or(1);

            let days_in_month = match month {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                2 => {
                    if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                        29
                    } else {
                        28
                    }
                }
                _ => 30,
            };

            if day >= days_in_month {
                if month >= 12 {
                    format!("{}-01-01", year + 1)
                } else {
                    format!("{}-{:02}-01", year, month + 1)
                }
            } else {
                format!("{}-{:02}-{:02}", year, month, day + 1)
            }
        } else {
            date.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_date_eq_day() {
        let value = SearchValue::new(SearchPrefix::Eq, "2024-01-15");
        let frag = DateHandler::build_sql(&value, 0);

        assert!(frag.sql.contains(">="));
        assert!(frag.sql.contains("<"));
        assert_eq!(frag.params.len(), 2);
    }

    #[test]
    fn test_date_gt() {
        // gt on day-precision "2024-01-15" matches strictly after the day, i.e.
        // value_date >= 2024-01-16T00:00:00.
        let value = SearchValue::new(SearchPrefix::Gt, "2024-01-15");
        let frag = DateHandler::build_sql(&value, 0);

        assert!(frag.sql.contains(">= ?1"));
        assert_eq!(frag.params.len(), 1);
        match &frag.params[0] {
            SqlParam::String(s) => assert_eq!(s, "2024-01-16T00:00:00"),
            _ => panic!("expected string bound"),
        }
    }

    #[test]
    fn test_date_le() {
        // le on day-precision matches up to the end of the day → < next day.
        let value = SearchValue::new(SearchPrefix::Le, "2024-01-15");
        let frag = DateHandler::build_sql(&value, 0);

        assert!(frag.sql.contains("< ?1"));
        match &frag.params[0] {
            SqlParam::String(s) => assert_eq!(s, "2024-01-16T00:00:00"),
            _ => panic!("expected string bound"),
        }
    }

    #[test]
    fn test_date_ap() {
        let value = SearchValue::new(SearchPrefix::Ap, "2024-01-15");
        let frag = DateHandler::build_sql(&value, 0);

        assert!(frag.sql.contains("BETWEEN"));
        assert!(frag.sql.contains("datetime"));
    }

    #[test]
    fn test_add_day() {
        assert_eq!(DateHandler::add_day("2024-01-15"), "2024-01-16");
        assert_eq!(DateHandler::add_day("2024-01-31"), "2024-02-01");
        assert_eq!(DateHandler::add_day("2024-12-31"), "2025-01-01");
    }
}
