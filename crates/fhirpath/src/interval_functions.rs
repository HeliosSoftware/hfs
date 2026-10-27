//! # FHIRPath Interval Functions
//!
//! Implements the `duration()` and `difference()` functions for date/time arithmetic.

use crate::datetime_impl;
use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};
use helios_fhirpath_support::{EvaluationError, EvaluationResult};

/// Extracts the date precision level from a date string.
/// Returns: 1 for year, 2 for year-month, 3 for full date.
fn date_precision(date_str: &str) -> u8 {
    match date_str.len() {
        4 => 1, // YYYY
        7 => 2, // YYYY-MM
        _ => 3, // YYYY-MM-DD
    }
}

/// Returns the minimum required precision level for a given precision keyword
/// when applied to dates.
fn required_date_precision(precision: &str) -> Option<u8> {
    match precision {
        "year" | "years" => Some(1),
        "month" | "months" => Some(2),
        "week" | "weeks" | "day" | "days" => Some(3),
        _ => None,
    }
}

/// Returns true if the precision is valid for time values.
fn is_time_precision(precision: &str) -> bool {
    matches!(
        precision,
        "hour"
            | "hours"
            | "minute"
            | "minutes"
            | "second"
            | "seconds"
            | "millisecond"
            | "milliseconds"
    )
}

/// Returns true if the precision is valid for date values.
fn is_date_precision(precision: &str) -> bool {
    matches!(
        precision,
        "year" | "years" | "month" | "months" | "week" | "weeks" | "day" | "days"
    )
}

/// Implements the FHIRPath `duration()` function.
///
/// Returns the number of whole calendar periods between two date/time values.
pub fn duration_function(
    invocation_base: &EvaluationResult,
    target: &EvaluationResult,
    precision: &str,
) -> Result<EvaluationResult, EvaluationError> {
    compute_interval(invocation_base, target, precision, false)
}

/// Implements the FHIRPath `difference()` function.
///
/// Returns the number of boundaries crossed between two date/time values.
pub fn difference_function(
    invocation_base: &EvaluationResult,
    target: &EvaluationResult,
    precision: &str,
) -> Result<EvaluationResult, EvaluationError> {
    compute_interval(invocation_base, target, precision, true)
}

fn compute_interval(
    from: &EvaluationResult,
    to: &EvaluationResult,
    precision: &str,
    is_difference: bool,
) -> Result<EvaluationResult, EvaluationError> {
    match (from, to) {
        // Date-Date
        (EvaluationResult::Date(from_str, _, _), EvaluationResult::Date(to_str, _, _)) => {
            if !is_date_precision(precision) {
                return Err(EvaluationError::InvalidArgument(format!(
                    "Precision '{}' is not valid for Date values",
                    precision
                )));
            }
            let req = required_date_precision(precision).unwrap();
            if date_precision(from_str) < req || date_precision(to_str) < req {
                return Ok(EvaluationResult::Empty);
            }
            let from_date = datetime_impl::parse_date(from_str).ok_or_else(|| {
                EvaluationError::InvalidArgument(format!("Cannot parse date: {}", from_str))
            })?;
            let to_date = datetime_impl::parse_date(to_str).ok_or_else(|| {
                EvaluationError::InvalidArgument(format!("Cannot parse date: {}", to_str))
            })?;
            let result = if is_difference {
                date_difference(from_date, to_date, precision)
            } else {
                date_duration(from_date, to_date, precision)
            };
            Ok(EvaluationResult::integer(result))
        }

        // DateTime-DateTime
        (EvaluationResult::DateTime(from_str, _, _), EvaluationResult::DateTime(to_str, _, _)) => {
            if !is_date_precision(precision) && !is_time_precision(precision) {
                return Err(EvaluationError::InvalidArgument(format!(
                    "Precision '{}' is not valid for DateTime values",
                    precision
                )));
            }
            if is_date_precision(precision) {
                let from_date_str = from_str.split('T').next().unwrap_or(from_str);
                let to_date_str = to_str.split('T').next().unwrap_or(to_str);
                let req = required_date_precision(precision).unwrap();
                if date_precision(from_date_str) < req || date_precision(to_date_str) < req {
                    return Ok(EvaluationResult::Empty);
                }
                let from_date = datetime_impl::parse_date(from_date_str).ok_or_else(|| {
                    EvaluationError::InvalidArgument(format!("Cannot parse date: {}", from_str))
                })?;
                let to_date = datetime_impl::parse_date(to_date_str).ok_or_else(|| {
                    EvaluationError::InvalidArgument(format!("Cannot parse date: {}", to_str))
                })?;
                let result = if is_difference {
                    date_difference(from_date, to_date, precision)
                } else {
                    date_duration(from_date, to_date, precision)
                };
                Ok(EvaluationResult::integer(result))
            } else {
                // Time precision on datetime
                let from_dt = datetime_impl::parse_datetime(from_str).ok_or_else(|| {
                    EvaluationError::InvalidArgument(format!("Cannot parse datetime: {}", from_str))
                })?;
                let to_dt = datetime_impl::parse_datetime(to_str).ok_or_else(|| {
                    EvaluationError::InvalidArgument(format!("Cannot parse datetime: {}", to_str))
                })?;
                let diff_ms = (to_dt - from_dt).num_milliseconds();
                let result = time_interval_from_ms(diff_ms, precision, is_difference);
                Ok(EvaluationResult::integer(result))
            }
        }

        // Time-Time
        (EvaluationResult::Time(from_str, _, _), EvaluationResult::Time(to_str, _, _)) => {
            if !is_time_precision(precision) {
                return Err(EvaluationError::InvalidArgument(format!(
                    "Precision '{}' is not valid for Time values",
                    precision
                )));
            }
            let from_time = datetime_impl::parse_time(from_str).ok_or_else(|| {
                EvaluationError::InvalidArgument(format!("Cannot parse time: {}", from_str))
            })?;
            let to_time = datetime_impl::parse_time(to_str).ok_or_else(|| {
                EvaluationError::InvalidArgument(format!("Cannot parse time: {}", to_str))
            })?;
            let diff_ms = time_diff_ms(from_time, to_time);
            let result = time_interval_from_ms(diff_ms, precision, is_difference);
            Ok(EvaluationResult::integer(result))
        }

        // Empty cases
        (EvaluationResult::Empty, _) | (_, EvaluationResult::Empty) => Ok(EvaluationResult::Empty),

        _ => Err(EvaluationError::TypeError(format!(
            "duration/difference requires matching date/time types, found {} and {}",
            from.type_name(),
            to.type_name()
        ))),
    }
}

/// Computes the number of whole calendar periods between two dates (duration).
fn date_duration(from: NaiveDate, to: NaiveDate, precision: &str) -> i64 {
    let sign = if to >= from { 1i64 } else { -1i64 };
    let (earlier, later) = if to >= from { (from, to) } else { (to, from) };

    match precision {
        "year" | "years" => {
            let mut years = later.year() as i64 - earlier.year() as i64;
            // Check if we've completed full years
            let anniversary = add_years(earlier, years as i32);
            if let Some(ann) = anniversary {
                if ann > later {
                    years -= 1;
                }
            }
            sign * years
        }
        "month" | "months" => {
            let mut months = (later.year() as i64 - earlier.year() as i64) * 12
                + (later.month() as i64 - earlier.month() as i64);
            let anniversary = add_months(earlier, months as i32);
            if let Some(ann) = anniversary {
                if ann > later {
                    months -= 1;
                }
            }
            sign * months
        }
        "week" | "weeks" => {
            let days = (later - earlier).num_days();
            sign * (days / 7)
        }
        "day" | "days" => {
            let days = (later - earlier).num_days();
            sign * days
        }
        _ => 0,
    }
}

/// Computes the number of boundaries crossed between two dates (difference).
fn date_difference(from: NaiveDate, to: NaiveDate, precision: &str) -> i64 {
    let sign = if to >= from { 1i64 } else { -1i64 };
    let (earlier, later) = if to >= from { (from, to) } else { (to, from) };

    match precision {
        "year" | "years" => {
            // Number of Jan 1 boundaries crossed
            sign * (later.year() as i64 - earlier.year() as i64)
        }
        "month" | "months" => {
            // Number of 1st-of-month boundaries crossed
            sign * ((later.year() as i64 - earlier.year() as i64) * 12
                + (later.month() as i64 - earlier.month() as i64))
        }
        "week" | "weeks" => {
            // Number of week boundaries (Sunday) crossed
            let earlier_week = iso_week_start(earlier);
            let later_week = iso_week_start(later);
            let weeks = (later_week - earlier_week).num_weeks();
            sign * weeks
        }
        "day" | "days" => {
            let days = (later - earlier).num_days();
            sign * days
        }
        _ => 0,
    }
}

/// Returns the start of the week (Sunday) for a given date.
fn iso_week_start(date: NaiveDate) -> NaiveDate {
    // weekday().num_days_from_sunday() gives 0=Sun, 1=Mon, ..., 6=Sat
    let days_from_sunday = date.weekday().num_days_from_sunday();
    date - chrono::Duration::days(days_from_sunday as i64)
}

/// Computes time interval from milliseconds difference.
fn time_interval_from_ms(diff_ms: i64, precision: &str, _is_difference: bool) -> i64 {
    // For time-based precision, duration and difference are the same
    // (no calendar boundaries to consider)
    match precision {
        "hour" | "hours" => diff_ms / 3_600_000,
        "minute" | "minutes" => diff_ms / 60_000,
        "second" | "seconds" => diff_ms / 1_000,
        "millisecond" | "milliseconds" => diff_ms,
        _ => 0,
    }
}

/// Computes the difference in milliseconds between two NaiveTimes.
fn time_diff_ms(from: NaiveTime, to: NaiveTime) -> i64 {
    let from_ms =
        from.num_seconds_from_midnight() as i64 * 1000 + from.nanosecond() as i64 / 1_000_000;
    let to_ms = to.num_seconds_from_midnight() as i64 * 1000 + to.nanosecond() as i64 / 1_000_000;
    to_ms - from_ms
}

/// Adds years to a date, clamping to month end for Feb 29 edge cases.
fn add_years(date: NaiveDate, years: i32) -> Option<NaiveDate> {
    let target_year = date.year() + years;
    NaiveDate::from_ymd_opt(target_year, date.month(), date.day()).or_else(|| {
        // Handle Feb 29 -> non-leap year
        NaiveDate::from_ymd_opt(target_year, date.month(), 28)
    })
}

/// Adds months to a date, clamping to month end for day overflow.
fn add_months(date: NaiveDate, months: i32) -> Option<NaiveDate> {
    let total_months = date.year() * 12 + date.month() as i32 - 1 + months;
    let target_year = total_months.div_euclid(12);
    let target_month = (total_months.rem_euclid(12) + 1) as u32;
    NaiveDate::from_ymd_opt(target_year, target_month, date.day()).or_else(|| {
        // Clamp to last day of month
        let last_day = last_day_of_month(target_year, target_month);
        NaiveDate::from_ymd_opt(target_year, target_month, last_day)
    })
}

/// Returns the last day of a given month.
fn last_day_of_month(year: i32, month: u32) -> u32 {
    NaiveDate::from_ymd_opt(year, month + 1, 1)
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap())
        .pred_opt()
        .unwrap()
        .day()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_duration_days() {
        let from = EvaluationResult::date("2025-01-02".to_string());
        let to = EvaluationResult::date("2025-01-07".to_string());
        let result = duration_function(&from, &to, "day").unwrap();
        assert_eq!(result, EvaluationResult::integer(5));
    }

    #[test]
    fn test_duration_weeks_partial() {
        // 5 days is 0 complete weeks
        let from = EvaluationResult::date("2025-01-02".to_string());
        let to = EvaluationResult::date("2025-01-07".to_string());
        let result = duration_function(&from, &to, "week").unwrap();
        assert_eq!(result, EvaluationResult::integer(0));
    }

    #[test]
    fn test_duration_weeks_full() {
        let from = EvaluationResult::date("2025-01-01".to_string());
        let to = EvaluationResult::date("2025-01-15".to_string());
        let result = duration_function(&from, &to, "week").unwrap();
        assert_eq!(result, EvaluationResult::integer(2));
    }

    #[test]
    fn test_duration_year_partial() {
        // Jan 1 to Sep 1 = 0 full years (only 8 months)
        let from = EvaluationResult::date("2025-01-01".to_string());
        let to = EvaluationResult::date("2025-09-01".to_string());
        let result = duration_function(&from, &to, "year").unwrap();
        assert_eq!(result, EvaluationResult::integer(0));
    }

    #[test]
    fn test_duration_year_dec_to_sep() {
        // Dec 2024 to Sep 2025 = 0 full years (only 9 months)
        let from = EvaluationResult::date("2024-12-01".to_string());
        let to = EvaluationResult::date("2025-09-01".to_string());
        let result = duration_function(&from, &to, "year").unwrap();
        assert_eq!(result, EvaluationResult::integer(0));
    }

    #[test]
    fn test_difference_week_boundary() {
        // Jan 2 (Thu) to Jan 7 (Tue) - crosses 1 Sunday boundary (Jan 5)
        let from = EvaluationResult::date("2025-01-02".to_string());
        let to = EvaluationResult::date("2025-01-07".to_string());
        let result = difference_function(&from, &to, "week").unwrap();
        assert_eq!(result, EvaluationResult::integer(1));
    }

    #[test]
    fn test_difference_year_same_year() {
        // Jan 1 to Sep 1 in 2025 = 0 year boundaries crossed
        let from = EvaluationResult::date("2025-01-01".to_string());
        let to = EvaluationResult::date("2025-09-01".to_string());
        let result = difference_function(&from, &to, "year").unwrap();
        assert_eq!(result, EvaluationResult::integer(0));
    }

    #[test]
    fn test_difference_year_cross() {
        // Dec 2024 to Sep 2025 = 1 year boundary crossed (Jan 1 2025)
        let from = EvaluationResult::date("2024-12-01".to_string());
        let to = EvaluationResult::date("2025-09-01".to_string());
        let result = difference_function(&from, &to, "year").unwrap();
        assert_eq!(result, EvaluationResult::integer(1));
    }

    #[test]
    fn test_duration_negative() {
        let from = EvaluationResult::date("2025-01-10".to_string());
        let to = EvaluationResult::date("2025-01-05".to_string());
        let result = duration_function(&from, &to, "day").unwrap();
        assert_eq!(result, EvaluationResult::integer(-5));
    }

    #[test]
    fn test_duration_empty_input() {
        let from = EvaluationResult::Empty;
        let to = EvaluationResult::date("2025-01-05".to_string());
        let result = duration_function(&from, &to, "day").unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_duration_insufficient_precision() {
        // Year-only date can't compute day precision
        let from = EvaluationResult::date("2025".to_string());
        let to = EvaluationResult::date("2025-06-01".to_string());
        let result = duration_function(&from, &to, "day").unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_duration_time_hours() {
        let from = EvaluationResult::time("10:00:00".to_string());
        let to = EvaluationResult::time("13:30:00".to_string());
        let result = duration_function(&from, &to, "hour").unwrap();
        assert_eq!(result, EvaluationResult::integer(3));
    }

    #[test]
    fn test_duration_time_minutes() {
        let from = EvaluationResult::time("10:00:00".to_string());
        let to = EvaluationResult::time("10:45:30".to_string());
        let result = duration_function(&from, &to, "minute").unwrap();
        assert_eq!(result, EvaluationResult::integer(45));
    }

    #[test]
    fn test_duration_months() {
        let from = EvaluationResult::date("2025-01-15".to_string());
        let to = EvaluationResult::date("2025-04-10".to_string());
        // Jan 15 to Apr 10 = 2 complete months (Feb 15, Mar 15 passed, Apr 15 not yet)
        let result = duration_function(&from, &to, "month").unwrap();
        assert_eq!(result, EvaluationResult::integer(2));
    }

    #[test]
    fn test_difference_months() {
        let from = EvaluationResult::date("2025-01-15".to_string());
        let to = EvaluationResult::date("2025-04-10".to_string());
        // 3 first-of-month boundaries crossed (Feb 1, Mar 1, Apr 1)
        let result = difference_function(&from, &to, "month").unwrap();
        assert_eq!(result, EvaluationResult::integer(3));
    }

    #[test]
    fn test_invalid_precision_for_date() {
        let from = EvaluationResult::date("2025-01-01".to_string());
        let to = EvaluationResult::date("2025-06-01".to_string());
        assert!(duration_function(&from, &to, "hour").is_err());
    }

    #[test]
    fn test_invalid_precision_for_time() {
        let from = EvaluationResult::time("10:00:00".to_string());
        let to = EvaluationResult::time("12:00:00".to_string());
        assert!(duration_function(&from, &to, "year").is_err());
    }
}
