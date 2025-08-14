use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
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
    let d1 = parse_date(date1)?;
    let d2 = parse_date(date2)?;
    Some(d1.cmp(&d2))
}

/// Compares two time values
pub fn compare_times(time1: &str, time2: &str) -> Option<Ordering> {
    let t1 = parse_time(time1)?;
    let t2 = parse_time(time2)?;
    Some(t1.cmp(&t2))
}

/// Compares two datetime values
pub fn compare_datetimes(dt1: &str, dt2: &str) -> Option<Ordering> {
    let d1 = parse_datetime(dt1)?;
    let d2 = parse_datetime(dt2)?;
    Some(d1.cmp(&d2))
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
        (EvaluationResult::Date(d1, _), EvaluationResult::Date(d2, _)) => compare_dates(d1, d2),
        (EvaluationResult::Time(t1, _), EvaluationResult::Time(t2, _)) => compare_times(t1, t2),
        (EvaluationResult::DateTime(dt1, _), EvaluationResult::DateTime(dt2, _)) => {
            compare_datetimes(dt1, dt2)
        }

        // Date vs DateTime comparison
        (EvaluationResult::Date(d, _), EvaluationResult::DateTime(dt, _)) => {
            // Convert date to datetime with time 00:00:00 for comparison
            let d_normalized = normalize_date(d);
            let d_as_dt = format!("{}T00:00:00", d_normalized);
            compare_datetimes(&d_as_dt, dt)
        }
        (EvaluationResult::DateTime(dt, _), EvaluationResult::Date(d, _)) => {
            // Convert date to datetime with time 00:00:00 for comparison
            let d_normalized = normalize_date(d);
            let d_as_dt = format!("{}T00:00:00", d_normalized);
            compare_datetimes(dt, &d_as_dt)
        }

        // Handle string-based date/time formats
        (EvaluationResult::String(s1, _), EvaluationResult::String(s2, _)) => {
            if s1.starts_with('@') && s2.starts_with('@') {
                // Both are date literals
                let value1 = s1.trim_start_matches('@');
                let value2 = s2.trim_start_matches('@');

                // Determine type and compare
                if value1.starts_with('T') && value2.starts_with('T') {
                    // Both are times
                    compare_times(
                        value1.trim_start_matches('T'),
                        value2.trim_start_matches('T'),
                    )
                } else if value1.contains('T') && value2.contains('T') {
                    // Both are datetimes
                    compare_datetimes(value1, value2)
                } else if !value1.contains('T') && !value2.contains('T') {
                    // Both are dates
                    compare_dates(value1, value2)
                } else {
                    // Mixed types
                    None
                }
            } else {
                // Not date literals
                None
            }
        }

        // Handle other conversions
        // String vs Date
        (EvaluationResult::String(s_val, _), EvaluationResult::Date(d_val, _)) => {
            // Attempt to parse s_val as a date and compare with d_val
            compare_dates(s_val, d_val)
        }
        (EvaluationResult::Date(d_val, _), EvaluationResult::String(s_val, _)) => {
            // Attempt to parse s_val as a date and compare with d_val
            compare_dates(d_val, s_val)
        }
        // String vs DateTime
        (EvaluationResult::String(s_val, _), EvaluationResult::DateTime(dt_val, _)) => {
            // Attempt to parse s_val as a datetime and compare with dt_val
            compare_datetimes(s_val, dt_val)
        }
        (EvaluationResult::DateTime(dt_val, _), EvaluationResult::String(s_val, _)) => {
            // Attempt to parse s_val as a datetime and compare with dt_val
            compare_datetimes(dt_val, s_val)
        }
        // String vs Time
        (EvaluationResult::String(s_val, _), EvaluationResult::Time(t_val, _)) => {
            // Attempt to parse s_val as a time and compare with t_val
            compare_times(s_val, t_val)
        }
        (EvaluationResult::Time(t_val, _), EvaluationResult::String(s_val, _)) => {
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
        EvaluationResult::Date(d, _) => Some(d.clone()),
        EvaluationResult::DateTime(dt, _) => {
            // Extract date part from datetime
            let parts: Vec<&str> = dt.split('T').collect();
            if !parts.is_empty() {
                Some(parts[0].to_string())
            } else {
                None
            }
        }
        EvaluationResult::String(s, _) => {
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
        EvaluationResult::DateTime(dt, _) => Some(dt.clone()),
        EvaluationResult::Date(d, _) => {
            // Extend date to datetime
            Some(format!("{}T00:00:00", d))
        }
        EvaluationResult::String(s, _) => {
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
        assert_eq!(compare_dates("2015", "2015-01-01"), Some(Ordering::Equal));
        assert_eq!(
            compare_dates("2015-01", "2015-01-01"),
            Some(Ordering::Equal)
        );
    }

    #[test]
    fn test_compare_times() {
        assert_eq!(compare_times("14:30:00", "14:30:00"), Some(Ordering::Equal));
        assert_eq!(compare_times("14:30:00", "14:30:01"), Some(Ordering::Less));
        assert_eq!(
            compare_times("14:30:01", "14:30:00"),
            Some(Ordering::Greater)
        );
        assert_eq!(compare_times("14", "14:00:00"), Some(Ordering::Equal));
        assert_eq!(compare_times("14:30", "14:30:00"), Some(Ordering::Equal));
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
}
