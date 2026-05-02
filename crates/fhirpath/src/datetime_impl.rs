//! # FHIRPath DateTime Implementation
//!
//! Provides internal date and time handling implementation for FHIRPath temporal functions.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use helios_fhir::{PrecisionDate, PrecisionDateTime, PrecisionTime};
use helios_fhirpath_support::EvaluationResult;
use std::cmp::Ordering;

/// Normalizes a date string to a consistent format
/// FHIR dates can be YYYY, YYYY-MM, or YYYY-MM-DD format
pub fn normalize_date(date_str: &str) -> String {
    match date_str.len() {
        4 => format!("{}-01-01", date_str), // YYYY -> YYYY-01-01
        7 => format!("{}-01", date_str),    // YYYY-MM -> YYYY-MM-01
        _ => date_str.to_string(),          // Already YYYY-MM-DD
    }
}

/// Normalizes a time string to a consistent format
/// FHIR times can be HH, HH:mm, HH:mm:ss, or HH:mm:ss.sss format
pub fn normalize_time(time_str: &str) -> String {
    match time_str.len() {
        2 => format!("{}:00:00", time_str), // HH -> HH:00:00
        5 => format!("{}:00", time_str),    // HH:mm -> HH:mm:00
        _ => time_str.to_string(),          // Already HH:mm:ss or HH:mm:ss.sss
    }
}

/// Parses a date string to a NaiveDate
/// Handles various date formats: YYYY, YYYY-MM, YYYY-MM-DD
pub fn parse_date(date_str: &str) -> Option<NaiveDate> {
    let normalized = normalize_date(date_str);
    NaiveDate::parse_from_str(&normalized, "%Y-%m-%d").ok()
}

/// Parses a time string to a NaiveTime
/// Handles various time formats: HH, HH:mm, HH:mm:ss, HH:mm:ss.sss
pub fn parse_time(time_str: &str) -> Option<NaiveTime> {
    let normalized = normalize_time(time_str);
    // Try different formats
    if normalized.contains('.') {
        // With milliseconds
        NaiveTime::parse_from_str(&normalized, "%H:%M:%S%.f").ok()
    } else {
        // Without milliseconds
        NaiveTime::parse_from_str(&normalized, "%H:%M:%S").ok()
    }
}

/// Parses a datetime string to a DateTime\<Utc\>
/// Handles various formats including timezone information by normalizing to UTC.
pub fn parse_datetime(datetime_str: &str) -> Option<DateTime<Utc>> {
    // Attempt to parse directly as RFC3339, which handles offsets.
    // This will work for "YYYY-MM-DDTHH:MM:SS[.sss][Z|+/-HH:MM]"
    if let Ok(dt_with_offset) = DateTime::parse_from_rfc3339(datetime_str) {
        return Some(dt_with_offset.with_timezone(&Utc));
    }

    // Fallback for partial datetimes or those without explicit offsets.
    // These are interpreted based on available components and assumed UTC if no offset specified.
    let parts: Vec<&str> = datetime_str.splitn(2, 'T').collect();

    if parts.len() == 2 {
        // Format like "YYYY-MM-DDTHH:MM:SS" (no offset) or "YYYY-MM-DDTHH" etc.
        let date_part_str = parts[0];
        let time_part_str = parts[1];

        let date = parse_date(date_part_str)?; // NaiveDate

        let time = if time_part_str.is_empty() {
            // e.g., "YYYY-MM-DDTHH" (T implies start of period)
            NaiveTime::from_hms_opt(0, 0, 0)?
        } else {
            parse_time(time_part_str)? // NaiveTime
        };

        let naive_dt = NaiveDateTime::new(date, time);
        // For datetimes parsed without an explicit offset, assume they are UTC.
        Some(DateTime::<Utc>::from_naive_utc_and_offset(naive_dt, Utc))
    } else if parts.len() == 1 {
        // Only a date part, e.g., "YYYY-MM-DD". FHIRPath treats this as a DateTime at the start of the day.
        let date = parse_date(parts[0])?; // NaiveDate
        let naive_dt = NaiveDateTime::new(date, NaiveTime::from_hms_opt(0, 0, 0)?);
        // Assume UTC for date-only strings converted to DateTime.
        Some(DateTime::<Utc>::from_naive_utc_and_offset(naive_dt, Utc))
    } else {
        None // Unparseable format
    }
}

/// Compares two date values
pub fn compare_dates(date1: &str, date2: &str) -> Option<Ordering> {
    // Strip @ prefix if present (used in FHIRPath date literals)
    let date1 = date1.strip_prefix('@').unwrap_or(date1);
    let date2 = date2.strip_prefix('@').unwrap_or(date2);

    // Try to parse as precision-aware dates
    let pd1 = PrecisionDate::parse(date1)?;
    let pd2 = PrecisionDate::parse(date2)?;

    // Use precision-aware comparison
    pd1.compare(&pd2)
}

/// Compares two time values
pub fn compare_times(time1: &str, time2: &str) -> Option<Ordering> {
    // Strip @ prefix and T prefix if present (used in FHIRPath time literals)
    let time1 = time1
        .strip_prefix('@')
        .unwrap_or(time1)
        .strip_prefix('T')
        .unwrap_or(time1);
    let time2 = time2
        .strip_prefix('@')
        .unwrap_or(time2)
        .strip_prefix('T')
        .unwrap_or(time2);

    // Try to parse as precision-aware times
    let pt1 = PrecisionTime::parse(time1)?;
    let pt2 = PrecisionTime::parse(time2)?;

    // Use precision-aware comparison
    pt1.compare(&pt2)
}

/// Compares two datetime values
pub fn compare_datetimes(dt1: &str, dt2: &str) -> Option<Ordering> {
    // Strip @ prefix if present (used in FHIRPath date literals)
    let dt1 = dt1.strip_prefix('@').unwrap_or(dt1);
    let dt2 = dt2.strip_prefix('@').unwrap_or(dt2);

    // Try to parse as precision-aware datetimes
    let pdt1 = PrecisionDateTime::parse(dt1)?;
    let pdt2 = PrecisionDateTime::parse(dt2)?;

    // Use precision-aware comparison
    pdt1.compare(&pdt2)
}

/// Compare two date/time values regardless of their specific types
/// This function normalizes and compares dates, times, and datetimes,
/// converting between them as needed for comparison
pub fn compare_date_time_values(
    left: &EvaluationResult,
    right: &EvaluationResult,
) -> Option<Ordering> {
    match (left, right) {
        // Direct comparisons of same types
        (EvaluationResult::Date(d1, _, _), EvaluationResult::Date(d2, _, _)) => {
            compare_dates(d1, d2)
        }
        (EvaluationResult::Time(t1, _, _), EvaluationResult::Time(t2, _, _)) => {
            compare_times(t1, t2)
        }
        (EvaluationResult::DateTime(dt1, _, _), EvaluationResult::DateTime(dt2, _, _)) => {
            compare_datetimes(dt1, dt2)
        }

        // Date vs DateTime comparison
        // Per FHIRPath spec: "If one value is specified to a different level of precision than
        // the other and the result is not determined before running out of precision, then
        // the result is empty indicating that the result of the comparison is unknown"
        (EvaluationResult::Date(date_str, _, _), EvaluationResult::DateTime(dt_str, _, _)) => {
            // Strip @ prefix if present
            let date_str = date_str.strip_prefix('@').unwrap_or(date_str);
            let dt_str = dt_str.strip_prefix('@').unwrap_or(dt_str);

            // Parse the date to get its precision
            let date_precision = PrecisionDate::parse(date_str)?;

            // Parse the datetime to get the date portion
            let dt_precision = PrecisionDateTime::parse(dt_str)?;

            // Compare at the date's precision level
            // First compare year
            let date_year = date_precision.year();
            let dt_year = dt_precision.date.year();

            match date_year.cmp(&dt_year) {
                Ordering::Less => Some(Ordering::Less),
                Ordering::Greater => Some(Ordering::Greater),
                Ordering::Equal => {
                    // Years are equal, check if date has month precision
                    if let Some(date_month) = date_precision.month() {
                        if let Some(dt_month) = dt_precision.date.month() {
                            match date_month.cmp(&dt_month) {
                                Ordering::Less => Some(Ordering::Less),
                                Ordering::Greater => Some(Ordering::Greater),
                                Ordering::Equal => {
                                    // Months are equal, check if date has day precision
                                    if let Some(date_day) = date_precision.day() {
                                        if let Some(dt_day) = dt_precision.date.day() {
                                            match date_day.cmp(&dt_day) {
                                                Ordering::Less => Some(Ordering::Less),
                                                Ordering::Greater => Some(Ordering::Greater),
                                                Ordering::Equal => {
                                                    // Date and DateTime are equal up to the date's precision
                                                    // Since DateTime has more precision (time), the comparison is indeterminate
                                                    None
                                                }
                                            }
                                        } else {
                                            // DateTime has no day component, which shouldn't happen
                                            // but if it does, we can't compare
                                            None
                                        }
                                    } else {
                                        // Date has no day precision, but year and month are equal
                                        // We've run out of precision without determining the result
                                        None
                                    }
                                }
                            }
                        } else {
                            // DateTime has no month component, which shouldn't happen
                            // but if it does, we can't compare
                            None
                        }
                    } else {
                        // Date has no month precision (year-only), but DateTime might have month
                        // We've run out of precision on the date side
                        None
                    }
                }
            }
        }
        (EvaluationResult::DateTime(dt_str, _, _), EvaluationResult::Date(date_str, _, _)) => {
            // Flip the comparison for DateTime vs Date
            match compare_date_time_values(
                &EvaluationResult::Date(date_str.clone(), None, None),
                &EvaluationResult::DateTime(dt_str.clone(), None, None),
            ) {
                Some(Ordering::Less) => Some(Ordering::Greater),
                Some(Ordering::Greater) => Some(Ordering::Less),
                Some(Ordering::Equal) => Some(Ordering::Equal),
                None => None,
            }
        }

        // Date vs Time comparison - these are incomparable types
        // For ordering comparisons (used by <, >, <=, >=), return None
        // For equality comparisons, this will be handled differently in the evaluator
        (EvaluationResult::Date(_, _, _), EvaluationResult::Time(_, _, _)) => None,
        (EvaluationResult::Time(_, _, _), EvaluationResult::Date(_, _, _)) => None,

        // Handle string-based date/time formats
        (EvaluationResult::String(s1, _, _), EvaluationResult::String(s2, _, _)) => {
            // Handle @ prefix for date literals
            let s1_clean = s1.strip_prefix('@').unwrap_or(s1);
            let s2_clean = s2.strip_prefix('@').unwrap_or(s2);

            // Check if these are date/time strings
            let s1_is_time = s1_clean.starts_with('T');
            let s2_is_time = s2_clean.starts_with('T');
            let s1_has_t = s1_clean.contains('T');
            let s2_has_t = s2_clean.contains('T');

            // Try to parse as dates/times
            let s1_is_date = !s1_is_time && !s1_has_t && parse_date(s1_clean).is_some();
            let s2_is_date = !s2_is_time && !s2_has_t && parse_date(s2_clean).is_some();
            let s1_is_datetime = !s1_is_time && s1_has_t && parse_datetime(s1_clean).is_some();
            let s2_is_datetime = !s2_is_time && s2_has_t && parse_datetime(s2_clean).is_some();

            match (
                s1_is_time,
                s2_is_time,
                s1_is_date,
                s2_is_date,
                s1_is_datetime,
                s2_is_datetime,
            ) {
                // Both are times
                (true, true, _, _, _, _) => compare_times(
                    s1_clean.trim_start_matches('T'),
                    s2_clean.trim_start_matches('T'),
                ),
                // Both are dates
                (false, false, true, true, false, false) => compare_dates(s1_clean, s2_clean),
                // Both are datetimes
                (false, false, false, false, true, true) => compare_datetimes(s1_clean, s2_clean),
                // Mixed date and datetime - use precision-aware comparison
                (false, false, true, false, false, true) => {
                    // s1 is date, s2 is datetime
                    compare_date_time_values(
                        &EvaluationResult::Date(s1_clean.to_string(), None, None),
                        &EvaluationResult::DateTime(s2_clean.to_string(), None, None),
                    )
                }
                (false, false, false, true, true, false) => {
                    // s1 is datetime, s2 is date
                    compare_date_time_values(
                        &EvaluationResult::DateTime(s1_clean.to_string(), None, None),
                        &EvaluationResult::Date(s2_clean.to_string(), None, None),
                    )
                }
                // Otherwise, not comparable as date/time types
                _ => None,
            }
        }

        // Handle other conversions
        // String vs Date
        (EvaluationResult::String(s_val, _, _), EvaluationResult::Date(d_val, _, _)) => {
            // Check if string is a datetime
            if s_val.contains('T') && parse_datetime(s_val).is_some() {
                // String is datetime, Date is date - indeterminate
                None
            } else {
                // Attempt to parse s_val as a date and compare with d_val
                compare_dates(s_val, d_val)
            }
        }
        (EvaluationResult::Date(d_val, _, _), EvaluationResult::String(s_val, _, _)) => {
            // Check if string is a datetime
            if s_val.contains('T') && parse_datetime(s_val).is_some() {
                // Date is date, String is datetime - indeterminate
                None
            } else {
                // Attempt to parse s_val as a date and compare with d_val
                compare_dates(d_val, s_val)
            }
        }
        // String vs DateTime
        (EvaluationResult::String(s_val, _, _), EvaluationResult::DateTime(dt_val, _, _)) => {
            // Check if string is a date (not datetime)
            if !s_val.contains('T') && parse_date(s_val).is_some() {
                // String is date, DateTime is datetime - indeterminate
                None
            } else {
                // Attempt to parse s_val as a datetime and compare with dt_val
                compare_datetimes(s_val, dt_val)
            }
        }
        (EvaluationResult::DateTime(dt_val, _, _), EvaluationResult::String(s_val, _, _)) => {
            // Check if string is a date (not datetime)
            if !s_val.contains('T') && parse_date(s_val).is_some() {
                // DateTime is datetime, String is date - indeterminate
                None
            } else {
                // Attempt to parse s_val as a datetime and compare with dt_val
                compare_datetimes(dt_val, s_val)
            }
        }
        // String vs Time
        (EvaluationResult::String(s_val, _, _), EvaluationResult::Time(t_val, _, _)) => {
            // Attempt to parse s_val as a time and compare with t_val
            compare_times(s_val, t_val)
        }
        (EvaluationResult::Time(t_val, _, _), EvaluationResult::String(s_val, _, _)) => {
            // Attempt to parse s_val as a time and compare with t_val
            compare_times(t_val, s_val)
        }

        // Cannot compare different types
        _ => None,
    }
}

/// Converts a value to a date representation if possible
pub fn to_date(value: &EvaluationResult) -> Option<String> {
    match value {
        EvaluationResult::Date(d, _, _) => Some(d.clone()),
        EvaluationResult::DateTime(dt, _, _) => {
            // Extract date part from datetime
            let parts: Vec<&str> = dt.split('T').collect();
            if !parts.is_empty() {
                Some(parts[0].to_string())
            } else {
                None
            }
        }
        EvaluationResult::String(s, _, _) => {
            // Try to interpret as a date
            if parse_date(s).is_some() {
                Some(s.clone())
            } else if let Some(parts) = s.split_once('T') {
                // Try to extract date part from a datetime string
                if parse_date(parts.0).is_some() {
                    Some(parts.0.to_string())
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Converts a value to a datetime representation if possible
pub fn to_datetime(value: &EvaluationResult) -> Option<String> {
    match value {
        EvaluationResult::DateTime(dt, _, _) => Some(dt.clone()),
        EvaluationResult::Date(d, _, _) => {
            // Extend date to datetime
            Some(format!("{}T00:00:00", d))
        }
        EvaluationResult::String(s, _, _) => {
            // Check if it's already a datetime format
            if s.contains('T') {
                if parse_datetime(s).is_some() {
                    Some(s.clone())
                } else {
                    None
                }
            } else {
                // Check if it's a date that we can extend to datetime
                if parse_date(s).is_some() {
                    Some(format!("{}T00:00:00", s))
                } else {
                    None
                }
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_date() {
        assert_eq!(normalize_date("2015"), "2015-01-01");
        assert_eq!(normalize_date("2015-02"), "2015-02-01");
        assert_eq!(normalize_date("2015-02-04"), "2015-02-04");
    }

    #[test]
    fn test_normalize_time() {
        assert_eq!(normalize_time("14"), "14:00:00");
        assert_eq!(normalize_time("14:30"), "14:30:00");
        assert_eq!(normalize_time("14:30:45"), "14:30:45");
        assert_eq!(normalize_time("14:30:45.123"), "14:30:45.123");
    }

    #[test]
    fn test_parse_date() {
        assert!(parse_date("2015").is_some());
        assert!(parse_date("2015-02").is_some());
        assert!(parse_date("2015-02-04").is_some());
        assert!(parse_date("invalid").is_none());
    }

    #[test]
    fn test_parse_time() {
        assert!(parse_time("14").is_some());
        assert!(parse_time("14:30").is_some());
        assert!(parse_time("14:30:45").is_some());
        assert!(parse_time("14:30:45.123").is_some());
        assert!(parse_time("invalid").is_none());
    }

    #[test]
    fn test_compare_dates() {
        assert_eq!(
            compare_dates("2015-01-01", "2015-01-01"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_dates("2015-01-01", "2015-01-02"),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_dates("2015-01-02", "2015-01-01"),
            Some(Ordering::Greater)
        );
        // Different precisions return None (indeterminate)
        assert_eq!(compare_dates("2015", "2015-01-01"), None);
        assert_eq!(compare_dates("2015-01", "2015-01-01"), None);
    }

    #[test]
    fn test_compare_times() {
        assert_eq!(compare_times("14:30:00", "14:30:00"), Some(Ordering::Equal));
        assert_eq!(compare_times("14:30:00", "14:30:01"), Some(Ordering::Less));
        assert_eq!(
            compare_times("14:30:01", "14:30:00"),
            Some(Ordering::Greater)
        );
        // Different precisions return None (indeterminate)
        assert_eq!(compare_times("14", "14:00:00"), None);
        assert_eq!(compare_times("14:30", "14:30:00"), None);
    }

    #[test]
    fn test_to_date() {
        assert_eq!(
            to_date(&EvaluationResult::date("2015-01-01".to_string())),
            Some("2015-01-01".to_string())
        );
        assert_eq!(
            to_date(&EvaluationResult::datetime(
                "2015-01-01T14:30:00".to_string()
            )),
            Some("2015-01-01".to_string())
        );
        assert_eq!(
            to_date(&EvaluationResult::string("2015-01-01".to_string())),
            Some("2015-01-01".to_string())
        );
    }

    #[test]
    fn test_to_datetime() {
        assert_eq!(
            to_datetime(&EvaluationResult::datetime(
                "2015-01-01T14:30:00".to_string()
            )),
            Some("2015-01-01T14:30:00".to_string())
        );
        assert_eq!(
            to_datetime(&EvaluationResult::date("2015-01-01".to_string())),
            Some("2015-01-01T00:00:00".to_string())
        );
        assert_eq!(
            to_datetime(&EvaluationResult::string("2015-01-01".to_string())),
            Some("2015-01-01T00:00:00".to_string())
        );
    }

    #[test]
    fn test_compare_datetimes_with_at_prefix() {
        // Test that @ prefix is properly stripped
        assert_eq!(
            compare_datetimes("@2015-01-01T00:00:00Z", "@2015-01-01T00:00:00Z"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_datetimes("@2015-01-01T00:00:00Z", "2015-01-01T00:00:00Z"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_datetimes("2015-01-01T00:00:00Z", "@2015-01-01T00:00:00Z"),
            Some(Ordering::Equal)
        );
        // Test actual comparison with different timezones
        assert_eq!(
            compare_datetimes(
                "@2001-05-06T00:00:00.000+14:00",
                "@2001-05-06T10:10:10.999Z"
            ),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn test_compare_dates_with_at_prefix() {
        // Test that @ prefix is properly stripped
        assert_eq!(
            compare_dates("@2015-01-01", "@2015-01-01"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_dates("@2015-01-01", "2015-01-01"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_dates("2015-01-01", "@2015-01-01"),
            Some(Ordering::Equal)
        );
    }

    #[test]
    fn test_compare_times_with_at_prefix() {
        // Test that @ and T prefixes are properly stripped
        assert_eq!(
            compare_times("@T14:30:00", "@T14:30:00"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_times("@T14:30:00", "T14:30:00"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_times("T14:30:00", "@T14:30:00"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_times("@T14:30:00", "14:30:00"),
            Some(Ordering::Equal)
        );
    }
}
