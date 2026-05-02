//! # FHIRPath Boundary Functions
//!
//! Implements functions for accessing collection boundaries: `first()`, `last()`, `tail()`, `skip()`, `take()`.

use chrono::{Datelike, NaiveDate};
use helios_fhirpath_support::{EvaluationError, EvaluationResult};
use rust_decimal::Decimal;
use std::str::FromStr;

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
/// * `args` - Optional precision parameter
///
/// # Returns
///
/// * `Ok(value)` - The low boundary value with appropriate type
/// * `Ok(Empty)` - If the input is Empty or boundary cannot be determined
/// * `Err` - If an error occurs, such as when the input is a multi-item collection
pub fn low_boundary_function(
    invocation_base: &EvaluationResult,
    args: &[EvaluationResult],
) -> Result<EvaluationResult, EvaluationError> {
    // Check for singleton
    if invocation_base.count() > 1 {
        return Err(EvaluationError::SingletonEvaluationError(
            "lowBoundary requires a singleton input".to_string(),
        ));
    }

    // Get precision parameter if provided
    let precision_param = if args.is_empty() {
        None
    } else if args.len() == 1 {
        match &args[0] {
            EvaluationResult::Integer(p, _, _) => {
                if *p < 0 {
                    return Err(EvaluationError::InvalidArgument(
                        "lowBoundary precision must be >= 0".to_string(),
                    ));
                }
                // rust_decimal supports up to 28 decimal places
                if *p > 28 {
                    return Ok(EvaluationResult::Empty);
                }
                Some(*p as u32)
            }
            EvaluationResult::Empty => return Ok(EvaluationResult::Empty),
            _ => {
                return Err(EvaluationError::TypeError(
                    "lowBoundary precision must be an Integer".to_string(),
                ));
            }
        }
    } else {
        return Err(EvaluationError::InvalidArity(
            "Function 'lowBoundary' expects 0 or 1 argument".to_string(),
        ));
    };

    // Handle each type according to FHIRPath boundary rules
    Ok(match invocation_base {
        EvaluationResult::Empty => EvaluationResult::Empty,
        EvaluationResult::Decimal(d, _, _) => {
            // For decimals, the low boundary depends on the precision
            let precision = precision_param.unwrap_or(8); // Default precision is 8 
            let low_bound = calculate_decimal_low_boundary(*d, precision);
            EvaluationResult::decimal(low_bound)
        }
        EvaluationResult::Integer(i, _, _) => {
            // For integers, treat as decimal
            let decimal_val = Decimal::from(*i);
            let precision = precision_param.unwrap_or(8); // Default precision is 8
            let low_bound = calculate_decimal_low_boundary(decimal_val, precision);
            EvaluationResult::decimal(low_bound)
        }
        EvaluationResult::Quantity(value, unit, _, _) => {
            // For quantities, apply boundary to the value part
            let precision = precision_param.unwrap_or(8); // Default precision is 8
            let low_bound = calculate_decimal_low_boundary(*value, precision);
            EvaluationResult::quantity(low_bound, unit.clone())
        }
        EvaluationResult::Date(date_str, _, _) => {
            // For dates, return the earliest possible date given the precision
            calculate_date_low_boundary(date_str, precision_param)
        }
        EvaluationResult::DateTime(datetime_str, _, _) => {
            // For datetimes, return the earliest possible datetime given the precision
            calculate_datetime_low_boundary(datetime_str, precision_param)
        }
        EvaluationResult::Time(time_str, _, _) => {
            // For times, return the earliest possible time given the precision
            calculate_time_low_boundary(time_str, precision_param)
        }
        EvaluationResult::String(s, type_info, _) => {
            // Handle FHIR primitive values that are represented as strings
            if let Some(ti) = type_info {
                match ti.name.to_lowercase().as_str() {
                    "date" => calculate_date_low_boundary(s, precision_param),
                    "datetime" => calculate_datetime_low_boundary(s, precision_param),
                    "time" => calculate_time_low_boundary(s, precision_param),
                    _ => {
                        // Fallback to pattern matching if type info doesn't match
                        if looks_like_date(s) {
                            calculate_date_low_boundary(s, precision_param)
                        } else if looks_like_datetime(s) {
                            calculate_datetime_low_boundary(s, precision_param)
                        } else if looks_like_time(s) {
                            calculate_time_low_boundary(s, precision_param)
                        } else {
                            EvaluationResult::Empty
                        }
                    }
                }
            } else {
                // Try to infer type from string format
                if looks_like_date(s) {
                    calculate_date_low_boundary(s, precision_param)
                } else if looks_like_datetime(s) {
                    calculate_datetime_low_boundary(s, precision_param)
                } else if looks_like_time(s) {
                    calculate_time_low_boundary(s, precision_param)
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
/// * `args` - Optional precision parameter
///
/// # Returns
///
/// * `Ok(value)` - The high boundary value with appropriate type
/// * `Ok(Empty)` - If the input is Empty or boundary cannot be determined
/// * `Err` - If an error occurs, such as when the input is a multi-item collection
pub fn high_boundary_function(
    invocation_base: &EvaluationResult,
    args: &[EvaluationResult],
) -> Result<EvaluationResult, EvaluationError> {
    // Check for singleton
    if invocation_base.count() > 1 {
        return Err(EvaluationError::SingletonEvaluationError(
            "highBoundary requires a singleton input".to_string(),
        ));
    }

    // Get precision parameter if provided
    let precision_param = if args.is_empty() {
        None
    } else if args.len() == 1 {
        match &args[0] {
            EvaluationResult::Integer(p, _, _) => {
                if *p < 0 {
                    return Err(EvaluationError::InvalidArgument(
                        "highBoundary precision must be >= 0".to_string(),
                    ));
                }
                // rust_decimal supports up to 28 decimal places
                if *p > 28 {
                    return Ok(EvaluationResult::Empty);
                }
                Some(*p as u32)
            }
            EvaluationResult::Empty => return Ok(EvaluationResult::Empty),
            _ => {
                return Err(EvaluationError::TypeError(
                    "highBoundary precision must be an Integer".to_string(),
                ));
            }
        }
    } else {
        return Err(EvaluationError::InvalidArity(
            "Function 'highBoundary' expects 0 or 1 argument".to_string(),
        ));
    };

    // Handle each type according to FHIRPath boundary rules
    Ok(match invocation_base {
        EvaluationResult::Empty => EvaluationResult::Empty,
        EvaluationResult::Decimal(d, _, _) => {
            // For decimals, the high boundary depends on the precision
            // Default precision is 8 for decimals
            let precision = precision_param.unwrap_or(8);
            let high_bound = calculate_decimal_high_boundary(*d, precision);
            EvaluationResult::decimal(high_bound)
        }
        EvaluationResult::Integer(i, _, _) => {
            // For integers, treat as decimal
            let decimal_val = Decimal::from(*i);
            let precision = precision_param.unwrap_or(8); // Default precision is 8
            let high_bound = calculate_decimal_high_boundary(decimal_val, precision);
            EvaluationResult::decimal(high_bound)
        }
        EvaluationResult::Quantity(value, unit, _, _) => {
            // For quantities, apply boundary to the value part
            let precision = precision_param.unwrap_or(8); // Default precision is 8
            let high_bound = calculate_decimal_high_boundary(*value, precision);
            EvaluationResult::quantity(high_bound, unit.clone())
        }
        EvaluationResult::Date(date_str, _, _) => {
            // For dates, return the latest possible date given the precision
            calculate_date_high_boundary(date_str, precision_param)
        }
        EvaluationResult::DateTime(datetime_str, _, _) => {
            // For datetimes, return the latest possible datetime given the precision
            calculate_datetime_high_boundary(datetime_str, precision_param)
        }
        EvaluationResult::Time(time_str, _, _) => {
            // For times, return the latest possible time given the precision
            calculate_time_high_boundary(time_str, precision_param)
        }
        EvaluationResult::String(s, type_info, _) => {
            // Handle FHIR primitive values that are represented as strings
            if let Some(ti) = type_info {
                match ti.name.to_lowercase().as_str() {
                    "date" => calculate_date_high_boundary(s, precision_param),
                    "datetime" => calculate_datetime_high_boundary(s, precision_param),
                    "time" => calculate_time_high_boundary(s, precision_param),
                    _ => {
                        // Fallback to pattern matching if type info doesn't match
                        if looks_like_date(s) {
                            calculate_date_high_boundary(s, precision_param)
                        } else if looks_like_datetime(s) {
                            calculate_datetime_high_boundary(s, precision_param)
                        } else if looks_like_time(s) {
                            calculate_time_high_boundary(s, precision_param)
                        } else {
                            EvaluationResult::Empty
                        }
                    }
                }
            } else {
                // Try to infer type from string format
                if looks_like_date(s) {
                    calculate_date_high_boundary(s, precision_param)
                } else if looks_like_datetime(s) {
                    calculate_datetime_high_boundary(s, precision_param)
                } else if looks_like_time(s) {
                    calculate_time_high_boundary(s, precision_param)
                } else {
                    EvaluationResult::Empty
                }
            }
        }
        // Other types don't have boundaries
        _ => EvaluationResult::Empty,
    })
}

/// Calculates the low boundary for a decimal value based on its precision
fn calculate_decimal_low_boundary(value: Decimal, precision: u32) -> Decimal {
    if precision == 0 {
        // For integer precision, return the truncated value for positive
        // or the next lower integer for negative
        let truncated = value.trunc();
        if value > Decimal::ZERO && value != truncated {
            // Positive non-integer: return the truncated value
            return truncated;
        } else if value > Decimal::ZERO {
            // Positive integer: return value - 1
            return truncated - Decimal::ONE;
        } else if value < Decimal::ZERO && value != truncated {
            // Negative non-integer: return the next lower integer
            return truncated - Decimal::ONE;
        } else {
            // Negative integer: return value - 1
            return truncated - Decimal::ONE;
        }
    }

    // Convert to string to examine the actual digits
    let value_str = value.to_string();
    let is_negative = value < Decimal::ZERO;

    // Remove negative sign for processing
    let value_str = value_str.trim_start_matches('-');
    let (_integer_part, decimal_part) = if let Some(dot_pos) = value_str.find('.') {
        (&value_str[..dot_pos], &value_str[dot_pos + 1..])
    } else {
        (value_str, "")
    };

    // Determine how many decimal places we actually have
    let actual_decimals = decimal_part.len() as u32;

    if actual_decimals < precision {
        // We have fewer decimal places than the precision
        if is_negative {
            // For negative numbers, low boundary is further from zero
            // We need to add 5 at the next position (which moves away from zero)
            let mut result = value;
            result.rescale(precision);

            // Subtract 5 * 10^(-(actual_decimals + 1))
            if let Some(unit) = 10_i64.checked_pow(actual_decimals + 1) {
                let decimal_unit = Decimal::from(unit);
                let five_at_position = Decimal::from(5) / decimal_unit;
                result -= five_at_position; // Subtract because we're negative, this moves away from zero
            }
            result
        } else {
            // For positive numbers, subtract 5 at the next position
            let mut result = value;
            result.rescale(precision);

            // Subtract 5 * 10^(-(actual_decimals + 1))
            if let Some(unit) = 10_i64.checked_pow(actual_decimals + 1) {
                let decimal_unit = Decimal::from(unit);
                let five_at_position = Decimal::from(5) / decimal_unit;
                result -= five_at_position;
            }
            result
        }
    } else {
        // We have more or equal decimal places than the precision
        // First check if the value rounds to 0
        let rounded = value.round_dp(precision);
        if rounded == Decimal::ZERO {
            // Special case: if rounds to 0, return 0
            return Decimal::ZERO;
        }

        // For both positive and negative, use floor for low boundary
        // Floor always moves towards negative infinity
        if let Some(scale) = 10_i64.checked_pow(precision) {
            let scale_dec = Decimal::from(scale);
            (value * scale_dec).floor() / scale_dec
        } else {
            value.round_dp(precision)
        }
    }
}

/// Calculates the high boundary for a decimal value based on its precision
fn calculate_decimal_high_boundary(value: Decimal, precision: u32) -> Decimal {
    // Special case: check if value rounds to 0 at given precision
    let rounded = value.round_dp(precision);
    if rounded == Decimal::ZERO {
        return Decimal::ZERO;
    }

    if precision == 0 {
        // For integer precision
        if value >= Decimal::ZERO {
            // Positive: next integer
            return value.trunc() + Decimal::ONE;
        } else {
            // Negative: truncate towards zero (ceiling)
            return value.trunc();
        }
    }

    // Get the string representation to check actual decimals
    let value_str = value.to_string();
    let is_negative = value < Decimal::ZERO;

    // Remove negative sign for processing
    let value_str_no_sign = value_str.trim_start_matches('-');
    let (_integer_part, decimal_part) = if let Some(dot_pos) = value_str_no_sign.find('.') {
        (
            &value_str_no_sign[..dot_pos],
            &value_str_no_sign[dot_pos + 1..],
        )
    } else {
        (value_str_no_sign, "")
    };

    let actual_decimals = decimal_part.len() as u32;

    if actual_decimals < precision {
        // Need to pad with 5 then 0s
        if is_negative {
            // For negative numbers: -1.587 with precision 8 should become -1.58650000
            // This moves the value towards zero (less negative)
            // We need to subtract 0.00050000 from the absolute value
            let padding_value = Decimal::from(5) / Decimal::from(10_i64.pow(actual_decimals + 1));
            value + padding_value // Adding to negative makes it less negative (towards zero)
        } else {
            // For positive numbers, pad normally
            let mut result = value_str.clone();
            if actual_decimals == 0 {
                result.push('.');
            }
            result.push('5');
            // Add zeros to reach the precision
            for _ in (actual_decimals + 1)..precision {
                result.push('0');
            }
            Decimal::from_str(&result).unwrap_or(value)
        }
    } else if actual_decimals > precision {
        // Need to round up at the precision
        // For positive: ceiling, for negative: floor (towards zero)
        if let Some(scale) = 10_i64.checked_pow(precision) {
            let scale_dec = Decimal::from(scale);
            if is_negative {
                // For negative, truncate towards zero
                (value * scale_dec).trunc() / scale_dec
            } else {
                // For positive, ceiling
                (value * scale_dec).ceil() / scale_dec
            }
        } else {
            value.round_dp(precision)
        }
    } else {
        // Exact match - return as is
        value
    }
}

/// Calculates the low boundary for a date value based on its precision
fn calculate_date_low_boundary(date_str: &str, precision_param: Option<u32>) -> EvaluationResult {
    // Strip @ prefix if present
    let date_str = date_str.strip_prefix('@').unwrap_or(date_str);

    // For dates, precision parameter limits the number of components
    // If not specified, infer from the input format
    let default_precision = match date_str.len() {
        4 => 4,   // YYYY
        7 => 7,   // YYYY-MM
        10 => 10, // YYYY-MM-DD
        _ => 4,   // Default to year
    };
    let precision = precision_param.unwrap_or(default_precision).min(10);

    if precision < 4 {
        return EvaluationResult::Empty;
    }

    match (date_str.len(), precision) {
        (4, 4) => {
            // YYYY format with year precision - return January 1st
            EvaluationResult::date(format!("{}-01-01", date_str))
        }
        (4, 6) => {
            // YYYY format with month precision - return as DateTime @YYYY-01
            EvaluationResult::datetime(format!("@{}-01", date_str))
        }
        (4, _) => {
            // YYYY format - return January 1st
            EvaluationResult::date(format!("{}-01-01", date_str))
        }
        (7, 4) => {
            // YYYY-MM format with year precision
            EvaluationResult::date(format!("{}-01-01", &date_str[0..4]))
        }
        (7, _) => {
            // YYYY-MM format - return first day of month
            EvaluationResult::date(format!("{}-01", date_str))
        }
        (10, 4) => {
            // YYYY-MM-DD format with year precision
            EvaluationResult::date(format!("{}-01-01", &date_str[0..4]))
        }
        (10, 6) | (10, 7) => {
            // YYYY-MM-DD format with month precision
            EvaluationResult::date(format!("{}-01", &date_str[0..7]))
        }
        (10, _) => {
            // YYYY-MM-DD format - already at day precision, return as-is
            EvaluationResult::date(date_str.to_string())
        }
        _ => EvaluationResult::Empty,
    }
}

/// Calculates the high boundary for a date value based on its precision
fn calculate_date_high_boundary(date_str: &str, precision_param: Option<u32>) -> EvaluationResult {
    // Strip @ prefix if present
    let date_str = date_str.strip_prefix('@').unwrap_or(date_str);

    // For dates, precision parameter limits the number of components
    // If not specified, infer from the input format
    let default_precision = match date_str.len() {
        4 => 4,   // YYYY
        7 => 7,   // YYYY-MM
        10 => 10, // YYYY-MM-DD
        _ => 4,   // Default to year
    };
    let precision = precision_param.unwrap_or(default_precision).min(10);

    if precision < 4 {
        return EvaluationResult::Empty;
    }

    match (date_str.len(), precision) {
        (4, 4) => {
            // YYYY format with year precision - return December 31st
            EvaluationResult::date(format!("{}-12-31", date_str))
        }
        (4, 6) => {
            // YYYY format with month precision - return as DateTime @YYYY-12
            EvaluationResult::datetime(format!("@{}-12", date_str))
        }
        (4, _) => {
            // YYYY format - return December 31st
            EvaluationResult::date(format!("{}-12-31", date_str))
        }
        (7, 4) => {
            // YYYY-MM format with year precision - return December 31st of year
            EvaluationResult::date(format!("{}-12-31", &date_str[0..4]))
        }
        (7, 6) => {
            // YYYY-MM format with month precision - return as DateTime
            EvaluationResult::datetime(format!("@{}", date_str))
        }
        (7, _) => {
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
        (10, 4) => {
            // YYYY-MM-DD format with year precision
            EvaluationResult::date(format!("{}-12-31", &date_str[0..4]))
        }
        (10, 6) => {
            // YYYY-MM-DD format with month precision - return as DateTime
            if let Ok(year) = date_str[0..4].parse::<i32>() {
                if let Ok(month) = date_str[5..7].parse::<u32>() {
                    if let Some(last_day) = last_day_of_month(year, month) {
                        return EvaluationResult::datetime(format!(
                            "{}-{:02}-{:02}T23:59:59.999-12:00",
                            year, month, last_day
                        ));
                    }
                }
            }
            EvaluationResult::Empty
        }
        (10, _) => {
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
fn calculate_datetime_low_boundary(
    datetime_str: &str,
    precision_param: Option<u32>,
) -> EvaluationResult {
    // Default precision for datetime is 17
    let precision = precision_param.unwrap_or(17);

    // Parse the datetime to understand its components
    if let Some(t_pos) = datetime_str.find('T') {
        let date_part = &datetime_str[..t_pos];

        // If precision is 8 or less, return just the date part
        if precision <= 8 {
            return EvaluationResult::datetime(format!("@{}", date_part));
        }

        let time_part = &datetime_str[t_pos + 1..];

        // Get timezone info if present
        let (time_only, timezone) = extract_timezone(time_part);

        // Normalize time_only if it's just HH format (e.g., "08" -> "08:00")
        let normalized_time = if time_only.len() == 2 {
            format!("{}:00", time_only)
        } else {
            time_only.to_string()
        };

        // Determine precision based on time format and precision parameter
        let low_time = if precision >= 17 {
            // Full precision - add milliseconds
            match normalized_time.len() {
                5 => format!("{}:00.000", normalized_time), // HH:MM -> HH:MM:00.000
                8 => format!("{}.000", normalized_time),    // HH:MM:SS -> HH:MM:SS.000
                _ => normalized_time.to_string(),           // Already has milliseconds
            }
        } else {
            // Limited precision handling
            normalized_time.to_string()
        };

        // Use the timezone from input if present, otherwise use +14:00 for low boundary
        let final_timezone = if !timezone.is_empty() {
            timezone
        } else {
            "+14:00"
        };

        let result_str = format!("@{}T{}{}", date_part, low_time, final_timezone);
        EvaluationResult::datetime(result_str)
    } else {
        // No time part, treat as date-only but convert to datetime with earliest timezone
        let low_date = match calculate_date_low_boundary(datetime_str, None) {
            EvaluationResult::Date(d, _, _) => d,
            _ => datetime_str.to_string(),
        };
        EvaluationResult::datetime(format!("@{}T00:00:00.000+14:00", low_date))
    }
}

/// Calculates the high boundary for a datetime value based on its precision
fn calculate_datetime_high_boundary(
    datetime_str: &str,
    precision_param: Option<u32>,
) -> EvaluationResult {
    // Default precision for datetime is 17
    let precision = precision_param.unwrap_or(17);

    // Parse the datetime to understand its components
    if let Some(t_pos) = datetime_str.find('T') {
        let date_part = &datetime_str[..t_pos];
        let time_part = &datetime_str[t_pos + 1..];

        // Get timezone info if present
        let (time_only, timezone) = extract_timezone(time_part);

        // Normalize time_only if it's just HH format (e.g., "08" -> "08:00")
        let normalized_time = if time_only.len() == 2 {
            format!("{}:00", time_only)
        } else {
            time_only.to_string()
        };

        // Determine precision based on time format and precision parameter
        let high_time = if precision >= 17 {
            // Full precision - add milliseconds
            match normalized_time.len() {
                5 => format!("{}:59.999", normalized_time), // HH:MM -> HH:MM:59.999
                8 => format!("{}.999", normalized_time),    // HH:MM:SS -> HH:MM:SS.999
                _ => normalized_time.to_string(),           // Already has milliseconds
            }
        } else {
            // Limited precision handling
            normalized_time.to_string()
        };

        // Use the timezone from input if present, otherwise use -12:00 for high boundary
        let final_timezone = if !timezone.is_empty() {
            timezone
        } else {
            "-12:00"
        };

        let result_str = format!("@{}T{}{}", date_part, high_time, final_timezone);
        EvaluationResult::datetime(result_str)
    } else {
        // No time part, treat as date-only but convert to datetime with latest timezone
        let high_date = match calculate_date_high_boundary(datetime_str, None) {
            EvaluationResult::Date(d, _, _) => d,
            _ => datetime_str.to_string(),
        };
        EvaluationResult::datetime(format!("@{}T23:59:59.999-12:00", high_date))
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
fn calculate_time_low_boundary(time_str: &str, precision_param: Option<u32>) -> EvaluationResult {
    // Default precision for time is 9
    let _precision = precision_param.unwrap_or(9);

    // Strip @ prefix if present
    let time_str = time_str
        .strip_prefix('@')
        .unwrap_or(time_str)
        .strip_prefix('T')
        .unwrap_or(time_str);

    match time_str.len() {
        2 => {
            // HH format - return start of hour (00:00.000)
            EvaluationResult::time(format!("@T{}:00:00.000", time_str))
        }
        5 => {
            // HH:MM format - return start of minute (00.000)
            EvaluationResult::time(format!("@T{}:00.000", time_str))
        }
        8 => {
            // HH:MM:SS format - return start of second (.000)
            EvaluationResult::time(format!("@T{}.000", time_str))
        }
        _ => {
            // Already precise or unknown format
            EvaluationResult::time(format!("@T{}", time_str))
        }
    }
}

/// Calculates the high boundary for a time value based on its precision
fn calculate_time_high_boundary(time_str: &str, precision_param: Option<u32>) -> EvaluationResult {
    // Default precision for time is 9
    let _precision = precision_param.unwrap_or(9);

    // Strip @ prefix if present
    let time_str = time_str
        .strip_prefix('@')
        .unwrap_or(time_str)
        .strip_prefix('T')
        .unwrap_or(time_str);

    match time_str.len() {
        2 => {
            // HH format - return end of hour (59:59.999)
            EvaluationResult::time(format!("@T{}:59:59.999", time_str))
        }
        5 => {
            // HH:MM format - return end of minute (59.999)
            EvaluationResult::time(format!("@T{}:59.999", time_str))
        }
        8 => {
            // HH:MM:SS format - return end of second (.999)
            EvaluationResult::time(format!("@T{}.999", time_str))
        }
        _ => {
            // Already precise or unknown format
            EvaluationResult::time(format!("@T{}", time_str))
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
        let result = low_boundary_function(&decimal_val, &[]).unwrap();
        assert_eq!(
            result,
            EvaluationResult::decimal(Decimal::from_str("0.95").unwrap())
        );
    }

    #[test]
    fn test_high_boundary_decimal() {
        // Test decimal with precision 1
        let decimal_val = EvaluationResult::decimal(Decimal::from_str("1.0").unwrap());
        let result = high_boundary_function(&decimal_val, &[]).unwrap();
        assert_eq!(
            result,
            EvaluationResult::decimal(Decimal::from_str("1.05").unwrap())
        );
    }

    #[test]
    fn test_low_boundary_date_month() {
        // Test date with month precision
        let date_val = EvaluationResult::date("1970-06".to_string());
        let result = low_boundary_function(&date_val, &[]).unwrap();
        assert_eq!(result, EvaluationResult::date("1970-06-01".to_string()));
    }

    #[test]
    fn test_high_boundary_date_month() {
        // Test date with month precision
        let date_val = EvaluationResult::date("1970-06".to_string());
        let result = high_boundary_function(&date_val, &[]).unwrap();
        assert_eq!(result, EvaluationResult::date("1970-06-30".to_string()));
    }

    #[test]
    fn test_low_boundary_time_minute() {
        // Test time with minute precision
        let time_val = EvaluationResult::time("12:34".to_string());
        let result = low_boundary_function(&time_val, &[]).unwrap();
        assert_eq!(result, EvaluationResult::time("@T12:34:00.000".to_string()));
    }

    #[test]
    fn test_high_boundary_time_minute() {
        // Test time with minute precision
        let time_val = EvaluationResult::time("12:34".to_string());
        let result = high_boundary_function(&time_val, &[]).unwrap();
        assert_eq!(result, EvaluationResult::time("@T12:34:59.999".to_string()));
    }

    #[test]
    fn test_boundary_empty() {
        let empty = EvaluationResult::Empty;
        assert_eq!(
            low_boundary_function(&empty, &[]).unwrap(),
            EvaluationResult::Empty
        );
        assert_eq!(
            high_boundary_function(&empty, &[]).unwrap(),
            EvaluationResult::Empty
        );
    }

    #[test]
    fn test_last_day_of_month() {
        assert_eq!(last_day_of_month(2020, 2), Some(29)); // Leap year February
        assert_eq!(last_day_of_month(2021, 2), Some(28)); // Non-leap year February
        assert_eq!(last_day_of_month(2021, 4), Some(30)); // April
        assert_eq!(last_day_of_month(2021, 12), Some(31)); // December
    }
}
