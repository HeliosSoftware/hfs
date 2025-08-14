use chrono::{Datelike, NaiveDate};
use helios_fhirpath_support::{EvaluationError, EvaluationResult};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

/// Implements the FHIRPath lowBoundary() function
///
/// Returns the lowest possible value that could be represented by the input value,
/// given its precision. For example:
/// - Decimal 1.0 with precision 1 -> 0.95 (precision boundary)
/// - Date 1970-06 -> 1970-06-01 (start of month)
/// - DateTime 1970-06-01T12:34 -> 1970-06-01T12:34:00.000Z (start of minute)
/// - Time 12:34 -> 12:34:00.000 (start of minute)
///
/// # Arguments
///
/// * `invocation_base` - The input value to find the low boundary for
///
/// # Returns
///
/// * `Ok(value)` - The low boundary value with appropriate type
/// * `Ok(Empty)` - If the input is Empty or boundary cannot be determined
/// * `Err` - If an error occurs, such as when the input is a multi-item collection
pub fn low_boundary_function(
    invocation_base: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // Check for singleton
    if invocation_base.count() > 1 {
        return Err(EvaluationError::SingletonEvaluationError(
            "lowBoundary requires a singleton input".to_string(),
        ));
    }

    // Handle each type according to FHIRPath boundary rules
    Ok(match invocation_base {
        EvaluationResult::Empty => EvaluationResult::Empty,
        EvaluationResult::Decimal(d, _) => {
            // For decimals, the low boundary depends on the precision
            // The precision is determined by the number of decimal places
            let decimal_str = d.to_string();
            let precision = get_decimal_precision(&decimal_str);
            let low_bound = calculate_decimal_low_boundary(*d, precision);
            EvaluationResult::decimal(low_bound)
        }
        EvaluationResult::Date(date_str, _) => {
            // For dates, return the earliest possible date given the precision
            calculate_date_low_boundary(date_str)
        }
        EvaluationResult::DateTime(datetime_str, _) => {
            // For datetimes, return the earliest possible datetime given the precision
            calculate_datetime_low_boundary(datetime_str)
        }
        EvaluationResult::Time(time_str, _) => {
            // For times, return the earliest possible time given the precision
            calculate_time_low_boundary(time_str)
        }
        EvaluationResult::String(s, type_info) => {
            // Handle FHIR primitive values that are represented as strings
            if let Some(ti) = type_info {
                match ti.name.to_lowercase().as_str() {
                    "date" => calculate_date_low_boundary(s),
                    "datetime" => calculate_datetime_low_boundary(s),
                    "time" => calculate_time_low_boundary(s),
                    _ => {
                        // Fallback to pattern matching if type info doesn't match
                        if looks_like_date(s) {
                            calculate_date_low_boundary(s)
                        } else if looks_like_datetime(s) {
                            calculate_datetime_low_boundary(s)
                        } else if looks_like_time(s) {
                            calculate_time_low_boundary(s)
                        } else {
                            EvaluationResult::Empty
                        }
                    }
                }
            } else {
                // Try to infer type from string format
                if looks_like_date(s) {
                    calculate_date_low_boundary(s)
                } else if looks_like_datetime(s) {
                    calculate_datetime_low_boundary(s)
                } else if looks_like_time(s) {
                    calculate_time_low_boundary(s)
                } else {
                    EvaluationResult::Empty
                }
            }
        }
        // Other types don't have boundaries
        _ => EvaluationResult::Empty,
    })
}

/// Implements the FHIRPath highBoundary() function
///
/// Returns the highest possible value that could be represented by the input value,
/// given its precision. For example:
/// - Decimal 1.0 with precision 1 -> 1.05 (precision boundary)
/// - Date 1970-06 -> 1970-06-30 (end of month)
/// - DateTime 1970-06-01T12:34 -> 1970-06-01T12:34:59.999Z (end of minute)
/// - Time 12:34 -> 12:34:59.999 (end of minute)
///
/// # Arguments
///
/// * `invocation_base` - The input value to find the high boundary for
///
/// # Returns
///
/// * `Ok(value)` - The high boundary value with appropriate type
/// * `Ok(Empty)` - If the input is Empty or boundary cannot be determined
/// * `Err` - If an error occurs, such as when the input is a multi-item collection
pub fn high_boundary_function(
    invocation_base: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // Check for singleton
    if invocation_base.count() > 1 {
        return Err(EvaluationError::SingletonEvaluationError(
            "highBoundary requires a singleton input".to_string(),
        ));
    }

    // Handle each type according to FHIRPath boundary rules
    Ok(match invocation_base {
        EvaluationResult::Empty => EvaluationResult::Empty,
        EvaluationResult::Decimal(d, _) => {
            // For decimals, the high boundary depends on the precision
            // The precision is determined by the number of decimal places
            let decimal_str = d.to_string();
            let precision = get_decimal_precision(&decimal_str);
            let high_bound = calculate_decimal_high_boundary(*d, precision);
            EvaluationResult::decimal(high_bound)
        }
        EvaluationResult::Date(date_str, _) => {
            // For dates, return the latest possible date given the precision
            calculate_date_high_boundary(date_str)
        }
        EvaluationResult::DateTime(datetime_str, _) => {
            // For datetimes, return the latest possible datetime given the precision
            calculate_datetime_high_boundary(datetime_str)
        }
        EvaluationResult::Time(time_str, _) => {
            // For times, return the latest possible time given the precision
            calculate_time_high_boundary(time_str)
        }
        EvaluationResult::String(s, type_info) => {
            // Handle FHIR primitive values that are represented as strings
            if let Some(ti) = type_info {
                match ti.name.to_lowercase().as_str() {
                    "date" => calculate_date_high_boundary(s),
                    "datetime" => calculate_datetime_high_boundary(s),
                    "time" => calculate_time_high_boundary(s),
                    _ => {
                        // Fallback to pattern matching if type info doesn't match
                        if looks_like_date(s) {
                            calculate_date_high_boundary(s)
                        } else if looks_like_datetime(s) {
                            calculate_datetime_high_boundary(s)
                        } else if looks_like_time(s) {
                            calculate_time_high_boundary(s)
                        } else {
                            EvaluationResult::Empty
                        }
                    }
                }
            } else {
                // Try to infer type from string format
                if looks_like_date(s) {
                    calculate_date_high_boundary(s)
                } else if looks_like_datetime(s) {
                    calculate_datetime_high_boundary(s)
                } else if looks_like_time(s) {
                    calculate_time_high_boundary(s)
                } else {
                    EvaluationResult::Empty
                }
            }
        }
        // Other types don't have boundaries
        _ => EvaluationResult::Empty,
    })
}

/// Gets the decimal precision (number of decimal places) from a decimal string
fn get_decimal_precision(decimal_str: &str) -> u32 {
    if let Some(dot_pos) = decimal_str.find('.') {
        (decimal_str.len() - dot_pos - 1) as u32
    } else {
        0 // No decimal point means precision is 0
    }
}

/// Calculates the low boundary for a decimal value based on its precision
fn calculate_decimal_low_boundary(value: Decimal, precision: u32) -> Decimal {
    if precision == 0 {
        // For integers, the boundary is the value itself minus 0.5
        value - Decimal::from_f64(0.5).unwrap_or(Decimal::ZERO)
    } else {
        // For decimals, subtract half of the smallest unit at the given precision
        let unit = Decimal::from(10_i64.pow(precision));
        let half_unit = Decimal::ONE / (unit * Decimal::from(2));
        value - half_unit
    }
}

/// Calculates the high boundary for a decimal value based on its precision
fn calculate_decimal_high_boundary(value: Decimal, precision: u32) -> Decimal {
    if precision == 0 {
        // For integers, the boundary is the value itself plus 0.5
        value + Decimal::from_f64(0.5).unwrap_or(Decimal::ZERO)
    } else {
        // For decimals, add half of the smallest unit at the given precision
        let unit = Decimal::from(10_i64.pow(precision));
        let half_unit = Decimal::ONE / (unit * Decimal::from(2));
        value + half_unit
    }
}

/// Calculates the low boundary for a date value based on its precision
fn calculate_date_low_boundary(date_str: &str) -> EvaluationResult {
    match date_str.len() {
        4 => {
            // YYYY format - return January 1st
            EvaluationResult::date(format!("{}-01-01", date_str))
        }
        7 => {
            // YYYY-MM format - return first day of month
            EvaluationResult::date(format!("{}-01", date_str))
        }
        10 => {
            // YYYY-MM-DD format - already at day precision, return as-is
            EvaluationResult::date(date_str.to_string())
        }
        _ => EvaluationResult::Empty,
    }
}

/// Calculates the high boundary for a date value based on its precision
fn calculate_date_high_boundary(date_str: &str) -> EvaluationResult {
    match date_str.len() {
        4 => {
            // YYYY format - return December 31st
            EvaluationResult::date(format!("{}-12-31", date_str))
        }
        7 => {
            // YYYY-MM format - return last day of month
            if let Ok(year) = date_str[0..4].parse::<i32>() {
                if let Ok(month) = date_str[5..7].parse::<u32>() {
                    if let Some(last_day) = last_day_of_month(year, month) {
                        return EvaluationResult::date(format!("{}-{:02}", date_str, last_day));
                    }
                }
            }
            EvaluationResult::Empty
        }
        10 => {
            // YYYY-MM-DD format - already at day precision, return as-is
            EvaluationResult::date(date_str.to_string())
        }
        _ => EvaluationResult::Empty,
    }
}

/// Gets the last day of a given month and year
fn last_day_of_month(year: i32, month: u32) -> Option<u32> {
    // Create the first day of the next month, then subtract one day
    let next_month = if month == 12 { 1 } else { month + 1 };
    let next_year = if month == 12 { year + 1 } else { year };

    if let Some(first_of_next) = NaiveDate::from_ymd_opt(next_year, next_month, 1) {
        let last_of_current = first_of_next.pred_opt()?;
        Some(last_of_current.day())
    } else {
        None
    }
}

/// Calculates the low boundary for a datetime value based on its precision
fn calculate_datetime_low_boundary(datetime_str: &str) -> EvaluationResult {
    // Parse the datetime to understand its components
    if let Some(t_pos) = datetime_str.find('T') {
        let date_part = &datetime_str[..t_pos];
        let time_part = &datetime_str[t_pos + 1..];

        // Get timezone info if present
        let (time_only, _timezone) = extract_timezone(time_part);

        // Determine precision based on time format
        let low_time = match time_only.len() {
            2 => format!("{}:00:00.000", time_only), // HH -> HH:00:00.000
            5 => format!("{}:00.000", time_only),    // HH:MM -> HH:MM:00.000
            8 => format!("{}.000", time_only),       // HH:MM:SS -> HH:MM:SS.000
            _ => time_only.to_string(),              // Already precise or unknown format
        };

        // Combine with date boundary and use the earliest timezone (+14:00 for low boundary)
        let low_date = match calculate_date_low_boundary(date_part) {
            EvaluationResult::Date(d, _) => d,
            _ => date_part.to_string(),
        };

        let result_str = format!("{}T{}+14:00", low_date, low_time);

        EvaluationResult::datetime(result_str)
    } else {
        // No time part, treat as date-only but convert to datetime with earliest timezone
        let low_date = match calculate_date_low_boundary(datetime_str) {
            EvaluationResult::Date(d, _) => d,
            _ => datetime_str.to_string(),
        };
        EvaluationResult::datetime(format!("{}T00:00:00.000+14:00", low_date))
    }
}

/// Calculates the high boundary for a datetime value based on its precision
fn calculate_datetime_high_boundary(datetime_str: &str) -> EvaluationResult {
    // Parse the datetime to understand its components
    if let Some(t_pos) = datetime_str.find('T') {
        let date_part = &datetime_str[..t_pos];
        let time_part = &datetime_str[t_pos + 1..];

        // Get timezone info if present
        let (time_only, _timezone) = extract_timezone(time_part);

        // Determine precision based on time format
        let high_time = match time_only.len() {
            2 => format!("{}:59:59.999", time_only), // HH -> HH:59:59.999
            5 => format!("{}:59.999", time_only),    // HH:MM -> HH:MM:59.999
            8 => format!("{}.999", time_only),       // HH:MM:SS -> HH:MM:SS.999
            _ => time_only.to_string(),              // Already precise or unknown format
        };

        // Combine with date boundary and use the latest timezone (-12:00 for high boundary)
        let high_date = match calculate_date_high_boundary(date_part) {
            EvaluationResult::Date(d, _) => d,
            _ => date_part.to_string(),
        };

        let result_str = format!("{}T{}-12:00", high_date, high_time);

        EvaluationResult::datetime(result_str)
    } else {
        // No time part, treat as date-only but convert to datetime with latest timezone
        let high_date = match calculate_date_high_boundary(datetime_str) {
            EvaluationResult::Date(d, _) => d,
            _ => datetime_str.to_string(),
        };
        EvaluationResult::datetime(format!("{}T23:59:59.999-12:00", high_date))
    }
}

/// Extracts timezone information from a time string
fn extract_timezone(time_str: &str) -> (&str, &str) {
    // Look for timezone indicators: Z, +HH:MM, -HH:MM
    if let Some(stripped) = time_str.strip_suffix('Z') {
        (stripped, "Z")
    } else if let Some(plus_pos) = time_str.rfind('+') {
        (&time_str[..plus_pos], &time_str[plus_pos..])
    } else if let Some(minus_pos) = time_str.rfind('-') {
        // Make sure this is actually a timezone offset, not part of the date
        if minus_pos > 2 {
            // Avoid confusion with time like "12:34-05:00"
            (&time_str[..minus_pos], &time_str[minus_pos..])
        } else {
            (time_str, "")
        }
    } else {
        (time_str, "")
    }
}

/// Calculates the low boundary for a time value based on its precision
fn calculate_time_low_boundary(time_str: &str) -> EvaluationResult {
    match time_str.len() {
        2 => {
            // HH format - return start of hour (00:00.000)
            EvaluationResult::time(format!("{}:00:00.000", time_str))
        }
        5 => {
            // HH:MM format - return start of minute (00.000)
            EvaluationResult::time(format!("{}:00.000", time_str))
        }
        8 => {
            // HH:MM:SS format - return start of second (.000)
            EvaluationResult::time(format!("{}.000", time_str))
        }
        _ => {
            // Already precise or unknown format
            EvaluationResult::time(time_str.to_string())
        }
    }
}

/// Calculates the high boundary for a time value based on its precision
fn calculate_time_high_boundary(time_str: &str) -> EvaluationResult {
    match time_str.len() {
        2 => {
            // HH format - return end of hour (59:59.999)
            EvaluationResult::time(format!("{}:59:59.999", time_str))
        }
        5 => {
            // HH:MM format - return end of minute (59.999)
            EvaluationResult::time(format!("{}:59.999", time_str))
        }
        8 => {
            // HH:MM:SS format - return end of second (.999)
            EvaluationResult::time(format!("{}.999", time_str))
        }
        _ => {
            // Already precise or unknown format
            EvaluationResult::time(time_str.to_string())
        }
    }
}

/// Checks if a string looks like a date (YYYY, YYYY-MM, YYYY-MM-DD)
fn looks_like_date(s: &str) -> bool {
    // Basic date pattern matching
    if s.len() == 4 {
        // YYYY
        s.chars().all(|c| c.is_ascii_digit())
    } else if s.len() == 7 {
        // YYYY-MM
        s.chars()
            .enumerate()
            .all(|(i, c)| if i == 4 { c == '-' } else { c.is_ascii_digit() })
    } else if s.len() == 10 {
        // YYYY-MM-DD
        s.chars().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        })
    } else {
        false
    }
}

/// Checks if a string looks like a datetime (contains 'T')
fn looks_like_datetime(s: &str) -> bool {
    s.contains('T')
}

/// Checks if a string looks like a time (HH, HH:MM, HH:MM:SS, HH:MM:SS.sss)
fn looks_like_time(s: &str) -> bool {
    // Basic time pattern matching
    if s.len() == 2 {
        // HH
        s.chars().all(|c| c.is_ascii_digit())
    } else if s.len() == 5 {
        // HH:MM
        s.chars()
            .enumerate()
            .all(|(i, c)| if i == 2 { c == ':' } else { c.is_ascii_digit() })
    } else if s.len() == 8 {
        // HH:MM:SS
        s.chars().enumerate().all(|(i, c)| {
            if i == 2 || i == 5 {
                c == ':'
            } else {
                c.is_ascii_digit()
            }
        })
    } else if s.len() > 8 && s.contains(':') && s.contains('.') {
        // HH:MM:SS.sss (rough check)
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::prelude::FromStr;

    #[test]
    fn test_low_boundary_decimal() {
        // Test decimal with precision 1
        let decimal_val = EvaluationResult::decimal(Decimal::from_str("1.0").unwrap());
        let result = low_boundary_function(&decimal_val).unwrap();
        assert_eq!(
            result,
            EvaluationResult::decimal(Decimal::from_str("0.95").unwrap())
        );
    }

    #[test]
    fn test_high_boundary_decimal() {
        // Test decimal with precision 1
        let decimal_val = EvaluationResult::decimal(Decimal::from_str("1.0").unwrap());
        let result = high_boundary_function(&decimal_val).unwrap();
        assert_eq!(
            result,
            EvaluationResult::decimal(Decimal::from_str("1.05").unwrap())
        );
    }

    #[test]
    fn test_low_boundary_date_month() {
        // Test date with month precision
        let date_val = EvaluationResult::date("1970-06".to_string());
        let result = low_boundary_function(&date_val).unwrap();
        assert_eq!(result, EvaluationResult::date("1970-06-01".to_string()));
    }

    #[test]
    fn test_high_boundary_date_month() {
        // Test date with month precision
        let date_val = EvaluationResult::date("1970-06".to_string());
        let result = high_boundary_function(&date_val).unwrap();
        assert_eq!(result, EvaluationResult::date("1970-06-30".to_string()));
    }

    #[test]
    fn test_low_boundary_time_minute() {
        // Test time with minute precision
        let time_val = EvaluationResult::time("12:34".to_string());
        let result = low_boundary_function(&time_val).unwrap();
        assert_eq!(result, EvaluationResult::time("12:34:00.000".to_string()));
    }

    #[test]
    fn test_high_boundary_time_minute() {
        // Test time with minute precision
        let time_val = EvaluationResult::time("12:34".to_string());
        let result = high_boundary_function(&time_val).unwrap();
        assert_eq!(result, EvaluationResult::time("12:34:59.999".to_string()));
    }

    #[test]
    fn test_boundary_empty() {
        let empty = EvaluationResult::Empty;
        assert_eq!(
            low_boundary_function(&empty).unwrap(),
            EvaluationResult::Empty
        );
        assert_eq!(
            high_boundary_function(&empty).unwrap(),
            EvaluationResult::Empty
        );
    }

    #[test]
    fn test_get_decimal_precision() {
        assert_eq!(get_decimal_precision("1"), 0);
        assert_eq!(get_decimal_precision("1.0"), 1);
        assert_eq!(get_decimal_precision("1.23"), 2);
        assert_eq!(get_decimal_precision("1.2345"), 4);
    }

    #[test]
    fn test_last_day_of_month() {
        assert_eq!(last_day_of_month(2020, 2), Some(29)); // Leap year February
        assert_eq!(last_day_of_month(2021, 2), Some(28)); // Non-leap year February
        assert_eq!(last_day_of_month(2021, 4), Some(30)); // April
        assert_eq!(last_day_of_month(2021, 12), Some(31)); // December
    }
}
