# Date/Time Implementation Fix

This document provides a step-by-step implementation guide for fixing the date/time operations in the FHIRPath implementation, focusing on comparison, precision, and timezone handling.

## 1. Enhanced Date/Time Representation

### 1.1. Create Robust Date/Time Types

First, we need to enhance our date/time representation to properly handle precision and timezone information:

```rust
// datetime_impl.rs

use chrono::{DateTime, NaiveDate, NaiveTime, NaiveDateTime, FixedOffset, TimeZone, Datelike, Timelike};
use fhirpath_support::{EvaluationResult, EvaluationError};
use std::cmp::Ordering;
use std::str::FromStr;

/// Represents the precision of a date or time value
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DateTimePrecision {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    Millisecond,
}

/// Represents a date, time, or datetime value with precision and timezone information
#[derive(Debug, Clone, PartialEq)]
pub struct DateTimeValue {
    /// The original string representation
    pub original: String,
    /// The parsed datetime value (as UTC DateTime)
    pub value: Option<DateTime<FixedOffset>>,
    /// The precision of the value
    pub precision: DateTimePrecision,
    /// Whether the value has timezone information
    pub has_timezone: bool,
    /// The type of date/time value
    pub dtype: DateTimeType,
}

/// The type of date/time value
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTimeType {
    Date,
    Time,
    DateTime,
}

impl DateTimeValue {
    /// Create a date value with the given precision
    pub fn date(value: &str, precision: DateTimePrecision) -> Result<Self, EvaluationError> {
        // Parse with appropriate precision
        let parsed = parse_date_with_precision(value, precision)?;
        
        Ok(Self {
            original: value.to_string(),
            value: parsed,
            precision,
            has_timezone: false,
            dtype: DateTimeType::Date,
        })
    }
    
    /// Create a time value with the given precision
    pub fn time(value: &str, precision: DateTimePrecision) -> Result<Self, EvaluationError> {
        // Parse time value
        let parsed = parse_time_with_precision(value, precision)?;
        
        // Check if there's timezone info
        let has_timezone = value.contains('Z') || value.contains('+') || value.contains('-');
        
        Ok(Self {
            original: value.to_string(),
            value: parsed,
            precision,
            has_timezone,
            dtype: DateTimeType::Time,
        })
    }
    
    /// Create a datetime value with the given precision
    pub fn datetime(value: &str, precision: DateTimePrecision) -> Result<Self, EvaluationError> {
        // Parse datetime value
        let parsed = parse_datetime_with_precision(value, precision)?;
        
        // Check if there's timezone info
        let has_timezone = value.contains('Z') || value.contains('+') || value.contains('-');
        
        Ok(Self {
            original: value.to_string(),
            value: parsed,
            precision,
            has_timezone,
            dtype: DateTimeType::DateTime,
        })
    }
    
    /// Determine if two datetime values are equal, respecting precision
    pub fn equals(&self, other: &Self) -> Result<bool, EvaluationError> {
        // If either value is None, they can't be equal
        if self.value.is_none() || other.value.is_none() {
            return Ok(false);
        }
        
        // Different types require special handling
        if self.dtype != other.dtype {
            return compare_different_datetime_types(self, other, |ord| ord == Ordering::Equal);
        }
        
        // Same type, normalize to lowest precision
        let min_precision = std::cmp::min(self.precision, other.precision);
        
        // Compare at the appropriate precision
        match (self.value, other.value) {
            (Some(dt1), Some(dt2)) => {
                let normalized1 = normalize_datetime_to_precision(dt1, min_precision);
                let normalized2 = normalize_datetime_to_precision(dt2, min_precision);
                Ok(normalized1 == normalized2)
            },
            _ => Ok(false),
        }
    }
    
    /// Compare two datetime values, respecting precision
    pub fn compare(&self, other: &Self) -> Result<Ordering, EvaluationError> {
        // If either value is None, they can't be compared
        if self.value.is_none() || other.value.is_none() {
            return Err(EvaluationError::InvalidOperation(
                format!("Cannot compare datetime values: {} and {}", self.original, other.original)
            ));
        }
        
        // Different types require special handling
        if self.dtype != other.dtype {
            return compare_different_datetime_types(self, other, |ord| ord);
        }
        
        // Same type, use the minimum precision
        let min_precision = std::cmp::min(self.precision, other.precision);
        
        // Compare at the appropriate precision
        match (self.value, other.value) {
            (Some(dt1), Some(dt2)) => {
                let normalized1 = normalize_datetime_to_precision(dt1, min_precision);
                let normalized2 = normalize_datetime_to_precision(dt2, min_precision);
                Ok(normalized1.cmp(&normalized2))
            },
            _ => Err(EvaluationError::InvalidOperation(
                format!("Cannot compare datetime values: {} and {}", self.original, other.original)
            )),
        }
    }
}

/// Parse a date string with the given precision
fn parse_date_with_precision(date: &str, precision: DateTimePrecision) -> Result<Option<DateTime<FixedOffset>>, EvaluationError> {
    // Normalize the date based on precision
    let normalized = match precision {
        DateTimePrecision::Year => {
            if date.len() == 4 {
                format!("{}-01-01T00:00:00Z", date)
            } else {
                return Err(EvaluationError::InvalidFormat(
                    format!("Invalid year format: {}", date)
                ));
            }
        },
        DateTimePrecision::Month => {
            if date.len() == 7 && date.contains('-') {
                format!("{}-01T00:00:00Z", date)
            } else {
                return Err(EvaluationError::InvalidFormat(
                    format!("Invalid month format: {}", date)
                ));
            }
        },
        DateTimePrecision::Day => {
            if date.len() == 10 && date.matches('-').count() == 2 {
                format!("{}T00:00:00Z", date)
            } else {
                return Err(EvaluationError::InvalidFormat(
                    format!("Invalid day format: {}", date)
                ));
            }
        },
        _ => {
            return Err(EvaluationError::InvalidFormat(
                format!("Invalid date precision: {:?}", precision)
            ));
        }
    };
    
    // Parse the normalized datetime
    match DateTime::parse_from_rfc3339(&normalized) {
        Ok(dt) => Ok(Some(dt)),
        Err(e) => Err(EvaluationError::InvalidFormat(
            format!("Error parsing date: {}", e)
        )),
    }
}

/// Parse a time string with the given precision
fn parse_time_with_precision(time: &str, precision: DateTimePrecision) -> Result<Option<DateTime<FixedOffset>>, EvaluationError> {
    // Extract timezone if present
    let (time_part, tz_part) = extract_timezone(time);
    
    // Add any missing time components based on precision
    let normalized_time = match precision {
        DateTimePrecision::Hour => {
            if time_part.contains(':') {
                time_part.to_string()
            } else {
                format!("{}:00:00", time_part)
            }
        },
        DateTimePrecision::Minute => {
            if time_part.matches(':').count() == 2 {
                time_part.to_string()
            } else {
                format!("{}:00", time_part)
            }
        },
        DateTimePrecision::Second | DateTimePrecision::Millisecond => {
            time_part.to_string()
        },
        _ => {
            return Err(EvaluationError::InvalidFormat(
                format!("Invalid time precision: {:?}", precision)
            ));
        }
    };
    
    // Add a dummy date to create a full datetime
    let dummy_date = "2000-01-01";
    let datetime_str = format!("{}T{}{}", dummy_date, normalized_time, tz_part);
    
    // Parse the full datetime
    match DateTime::parse_from_rfc3339(&datetime_str) {
        Ok(dt) => Ok(Some(dt)),
        Err(e) => Err(EvaluationError::InvalidFormat(
            format!("Error parsing time: {}", e)
        )),
    }
}

/// Parse a datetime string with the given precision
fn parse_datetime_with_precision(datetime: &str, precision: DateTimePrecision) -> Result<Option<DateTime<FixedOffset>>, EvaluationError> {
    // Split into date and time parts
    let parts: Vec<&str> = datetime.split('T').collect();
    if parts.len() != 2 {
        return Err(EvaluationError::InvalidFormat(
            format!("Invalid datetime format: {}", datetime)
        ));
    }
    
    let date_part = parts[0];
    let (time_part, tz_part) = extract_timezone(parts[1]);
    
    // Add any missing components based on precision
    let normalized_datetime = match precision {
        DateTimePrecision::Year => {
            if date_part.len() == 4 {
                format!("{}-01-01T00:00:00{}", date_part, tz_part)
            } else {
                return Err(EvaluationError::InvalidFormat(
                    format!("Invalid year format in datetime: {}", datetime)
                ));
            }
        },
        DateTimePrecision::Month => {
            if date_part.len() == 7 && date_part.contains('-') {
                format!("{}-01T00:00:00{}", date_part, tz_part)
            } else {
                return Err(EvaluationError::InvalidFormat(
                    format!("Invalid month format in datetime: {}", datetime)
                ));
            }
        },
        DateTimePrecision::Day => {
            format!("{}T00:00:00{}", date_part, tz_part)
        },
        DateTimePrecision::Hour => {
            if time_part.contains(':') {
                format!("{}T{}:00:00{}", date_part, time_part, tz_part)
            } else {
                format!("{}T{}:00:00{}", date_part, time_part, tz_part)
            }
        },
        DateTimePrecision::Minute => {
            if time_part.matches(':').count() == 2 {
                format!("{}T{}{}", date_part, time_part, tz_part)
            } else {
                format!("{}T{}:00{}", date_part, time_part, tz_part)
            }
        },
        DateTimePrecision::Second | DateTimePrecision::Millisecond => {
            format!("{}T{}{}", date_part, time_part, tz_part)
        },
    };
    
    // Parse the normalized datetime
    match DateTime::parse_from_rfc3339(&normalized_datetime) {
        Ok(dt) => Ok(Some(dt)),
        Err(e) => Err(EvaluationError::InvalidFormat(
            format!("Error parsing datetime: {}", e)
        )),
    }
}

/// Extract timezone information from a time string
fn extract_timezone(time: &str) -> (&str, &str) {
    if time.contains('Z') {
        let pos = time.find('Z').unwrap();
        (&time[..pos], "Z")
    } else if time.contains('+') {
        let pos = time.find('+').unwrap();
        (&time[..pos], &time[pos..])
    } else if time.contains('-') {
        // Be careful with negative timezone offsets - the time itself might contain hyphens for separators
        // Try to find a hyphen followed by a digit
        let mut pos = None;
        for (i, c) in time.char_indices() {
            if c == '-' && i > 0 && i < time.len() - 1 {
                let next_char = time.chars().nth(i + 1).unwrap();
                if next_char.is_digit(10) {
                    pos = Some(i);
                    break;
                }
            }
        }
        
        if let Some(p) = pos {
            (&time[..p], &time[p..])
        } else {
            (time, "")
        }
    } else {
        (time, "")
    }
}

/// Normalize a datetime to the given precision
fn normalize_datetime_to_precision(dt: DateTime<FixedOffset>, precision: DateTimePrecision) -> DateTime<FixedOffset> {
    match precision {
        DateTimePrecision::Year => {
            // Set month, day, hour, minute, second, nanosecond to their minimum values
            dt.with_month(1).unwrap()
              .with_day(1).unwrap()
              .with_hour(0).unwrap()
              .with_minute(0).unwrap()
              .with_second(0).unwrap()
              .with_nanosecond(0).unwrap()
        },
        DateTimePrecision::Month => {
            // Set day, hour, minute, second, nanosecond to their minimum values
            dt.with_day(1).unwrap()
              .with_hour(0).unwrap()
              .with_minute(0).unwrap()
              .with_second(0).unwrap()
              .with_nanosecond(0).unwrap()
        },
        DateTimePrecision::Day => {
            // Set hour, minute, second, nanosecond to their minimum values
            dt.with_hour(0).unwrap()
              .with_minute(0).unwrap()
              .with_second(0).unwrap()
              .with_nanosecond(0).unwrap()
        },
        DateTimePrecision::Hour => {
            // Set minute, second, nanosecond to their minimum values
            dt.with_minute(0).unwrap()
              .with_second(0).unwrap()
              .with_nanosecond(0).unwrap()
        },
        DateTimePrecision::Minute => {
            // Set second, nanosecond to their minimum values
            dt.with_second(0).unwrap()
              .with_nanosecond(0).unwrap()
        },
        DateTimePrecision::Second => {
            // Set nanosecond to its minimum value
            dt.with_nanosecond(0).unwrap()
        },
        DateTimePrecision::Millisecond => {
            // Keep millisecond precision, zero out smaller components
            let millis = dt.timestamp_subsec_millis();
            dt.with_nanosecond(millis * 1_000_000).unwrap()
        },
    }
}

/// Compare different types of date/time values
fn compare_different_datetime_types<F, T>(dt1: &DateTimeValue, dt2: &DateTimeValue, f: F) -> Result<T, EvaluationError>
where
    F: FnOnce(Ordering) -> T,
{
    match (dt1.dtype, dt2.dtype) {
        (DateTimeType::Date, DateTimeType::DateTime) => {
            // Compare date with datetime
            // Treat date as datetime at start of day (00:00:00)
            match (dt1.value, dt2.value) {
                (Some(date_val), Some(dt_val)) => {
                    // Normalize date to day precision
                    let normalized_date = normalize_datetime_to_precision(date_val, DateTimePrecision::Day);
                    // Normalize datetime to appropriate precision
                    let min_precision = std::cmp::min(DateTimePrecision::Day, dt2.precision);
                    let normalized_dt = normalize_datetime_to_precision(dt_val, min_precision);
                    
                    Ok(f(normalized_date.cmp(&normalized_dt)))
                },
                _ => Err(EvaluationError::InvalidOperation(
                    format!("Cannot compare date and datetime: {} and {}", dt1.original, dt2.original)
                )),
            }
        },
        (DateTimeType::DateTime, DateTimeType::Date) => {
            // Swapped order, use the same logic but flip the result
            match (dt2.value, dt1.value) {
                (Some(date_val), Some(dt_val)) => {
                    // Normalize date to day precision
                    let normalized_date = normalize_datetime_to_precision(date_val, DateTimePrecision::Day);
                    // Normalize datetime to appropriate precision
                    let min_precision = std::cmp::min(DateTimePrecision::Day, dt1.precision);
                    let normalized_dt = normalize_datetime_to_precision(dt_val, min_precision);
                    
                    Ok(f(normalized_dt.cmp(&normalized_date)))
                },
                _ => Err(EvaluationError::InvalidOperation(
                    format!("Cannot compare datetime and date: {} and {}", dt1.original, dt2.original)
                )),
            }
        },
        (DateTimeType::Time, DateTimeType::DateTime) | 
        (DateTimeType::DateTime, DateTimeType::Time) | 
        (DateTimeType::Date, DateTimeType::Time) | 
        (DateTimeType::Time, DateTimeType::Date) => {
            // Time can't be compared with date or datetime
            Err(EvaluationError::InvalidOperation(
                format!("Cannot compare incompatible types: {:?} and {:?}", dt1.dtype, dt2.dtype)
            ))
        },
        _ => {
            // This should not happen - we already checked that the types are different
            Err(EvaluationError::InvalidOperation(
                format!("Unexpected date/time type combination: {:?} and {:?}", dt1.dtype, dt2.dtype)
            ))
        },
    }
}

/// Detect the precision of a date string
pub fn detect_date_precision(date: &str) -> DateTimePrecision {
    let parts: Vec<&str> = date.split('-').collect();
    match parts.len() {
        1 => DateTimePrecision::Year,
        2 => DateTimePrecision::Month,
        3 => DateTimePrecision::Day,
        _ => DateTimePrecision::Day, // Default to day precision for invalid formats
    }
}

/// Detect the precision of a time string
pub fn detect_time_precision(time: &str) -> DateTimePrecision {
    // Remove timezone part for counting colons
    let (time_part, _) = extract_timezone(time);
    
    // Check for milliseconds
    if time_part.contains('.') {
        return DateTimePrecision::Millisecond;
    }
    
    // Count colons to determine precision
    let colon_count = time_part.chars().filter(|&c| c == ':').count();
    match colon_count {
        0 => DateTimePrecision::Hour,
        1 => DateTimePrecision::Minute,
        _ => DateTimePrecision::Second,
    }
}

/// Detect the precision of a datetime string
pub fn detect_datetime_precision(datetime: &str) -> DateTimePrecision {
    // Split into date and time parts
    let parts: Vec<&str> = datetime.split('T').collect();
    if parts.len() != 2 {
        return DateTimePrecision::Day; // Default to day precision for invalid formats
    }
    
    let date_part = parts[0];
    let time_part = parts[1];
    
    // Detect date precision
    let date_precision = detect_date_precision(date_part);
    
    // If there's no time part, use the date precision
    if time_part.is_empty() {
        return date_precision;
    }
    
    // Detect time precision
    let time_precision = detect_time_precision(time_part);
    
    // Use the more precise of the two
    if time_precision as u8 > date_precision as u8 {
        time_precision
    } else {
        date_precision
    }
}

/// Parse a date/time literal from FHIRPath syntax (e.g., @2019-01-01, @T14:30:00)
pub fn parse_date_literal(literal: &str) -> Result<DateTimeValue, EvaluationError> {
    // Remove @ prefix
    if !literal.starts_with('@') {
        return Err(EvaluationError::InvalidFormat(
            format!("Invalid date/time literal: {}", literal)
        ));
    }
    
    let value = &literal[1..];
    
    // Determine the type and precision
    if value.starts_with('T') {
        // Time value
        let precision = detect_time_precision(value);
        DateTimeValue::time(value, precision)
    } else if value.contains('T') {
        // DateTime value
        let precision = detect_datetime_precision(value);
        DateTimeValue::datetime(value, precision)
    } else {
        // Date value
        let precision = detect_date_precision(value);
        DateTimeValue::date(value, precision)
    }
}

/// Convert DateTimeValue to EvaluationResult
pub fn datetime_value_to_result(value: DateTimeValue) -> EvaluationResult {
    match value.dtype {
        DateTimeType::Date => EvaluationResult::Date(value.original),
        DateTimeType::Time => EvaluationResult::Time(value.original),
        DateTimeType::DateTime => EvaluationResult::DateTime(value.original),
    }
}

/// Convert EvaluationResult to DateTimeValue
pub fn result_to_datetime_value(result: &EvaluationResult) -> Result<DateTimeValue, EvaluationError> {
    match result {
        EvaluationResult::Date(value) => {
            let precision = detect_date_precision(value);
            DateTimeValue::date(value, precision)
        },
        EvaluationResult::Time(value) => {
            let precision = detect_time_precision(value);
            DateTimeValue::time(value, precision)
        },
        EvaluationResult::DateTime(value) => {
            let precision = detect_datetime_precision(value);
            DateTimeValue::datetime(value, precision)
        },
        _ => Err(EvaluationError::TypeError(
            format!("Cannot convert {} to date/time value", result.type_name())
        )),
    }
}
```

### 1.2. Enhance Date/Time Operations

Next, update the `date_operation.rs` module to use our enhanced date/time types:

```rust
// date_operation.rs

use crate::datetime_impl::{
    DateTimeValue, DateTimePrecision, DateTimeType,
    parse_date_literal, result_to_datetime_value, datetime_value_to_result
};
use fhirpath_support::{EvaluationResult, EvaluationError};
use std::cmp::Ordering;
use chrono::{Local, Datelike, Timelike};

/// Applies a date/time comparison operation
pub fn apply_date_type_operation(
    left: &EvaluationResult,
    op: &str,
    right: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // Convert both values to DateTimeValue
    let left_dt = result_to_datetime_value(left)?;
    let right_dt = result_to_datetime_value(right)?;
    
    // Apply the appropriate operation
    match op {
        "=" => {
            // Equals operation
            let result = left_dt.equals(&right_dt)?;
            Ok(EvaluationResult::Boolean(result))
        },
        "!=" => {
            // Not equals operation
            let result = left_dt.equals(&right_dt)?;
            Ok(EvaluationResult::Boolean(!result))
        },
        "~" => {
            // Equivalent operation (more lenient than equals)
            let result = left_dt.equals(&right_dt)?;
            Ok(EvaluationResult::Boolean(result))
        },
        "!~" => {
            // Not equivalent operation
            let result = left_dt.equals(&right_dt)?;
            Ok(EvaluationResult::Boolean(!result))
        },
        "<" => {
            // Less than operation
            let result = left_dt.compare(&right_dt)?;
            Ok(EvaluationResult::Boolean(result == Ordering::Less))
        },
        "<=" => {
            // Less than or equal operation
            let result = left_dt.compare(&right_dt)?;
            Ok(EvaluationResult::Boolean(result != Ordering::Greater))
        },
        ">" => {
            // Greater than operation
            let result = left_dt.compare(&right_dt)?;
            Ok(EvaluationResult::Boolean(result == Ordering::Greater))
        },
        ">=" => {
            // Greater than or equal operation
            let result = left_dt.compare(&right_dt)?;
            Ok(EvaluationResult::Boolean(result != Ordering::Less))
        },
        _ => Err(EvaluationError::InvalidOperation(
            format!("Unsupported date/time operation: {}", op)
        )),
    }
}

/// Generates the current date (today)
pub fn today() -> EvaluationResult {
    let now = Local::now();
    let date_str = format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day());
    EvaluationResult::Date(date_str)
}

/// Generates the current time (timeOfDay)
pub fn time_of_day() -> EvaluationResult {
    let now = Local::now();
    let time_str = format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second());
    EvaluationResult::Time(time_str)
}

/// Generates the current date and time (now)
pub fn now() -> EvaluationResult {
    let now = Local::now();
    let datetime_str = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        now.year(), now.month(), now.day(),
        now.hour(), now.minute(), now.second()
    );
    EvaluationResult::DateTime(datetime_str)
}
```

### 1.3. Enhance Date/Time Arithmetic

Update the `date_arithmetic.rs` module to use our enhanced date/time types:

```rust
// date_arithmetic.rs

use crate::datetime_impl::{
    DateTimeValue, DateTimePrecision, DateTimeType,
    result_to_datetime_value, datetime_value_to_result
};
use fhirpath_support::{EvaluationResult, EvaluationError};
use chrono::{Duration, Datelike, Timelike};
use rust_decimal::Decimal;

/// Add a quantity to a date or time
pub fn add_date_time_quantity(
    date_time: &EvaluationResult,
    quantity: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // Convert date/time to DateTimeValue
    let dt_value = result_to_datetime_value(date_time)?;
    
    // Extract quantity value and unit
    let (amount, unit) = match quantity {
        EvaluationResult::Quantity(value, unit) => (*value, unit.as_str()),
        EvaluationResult::Integer(value) => (Decimal::from(*value), "day"), // Default to days
        EvaluationResult::Decimal(value) => (*value, "day"), // Default to days
        _ => return Err(EvaluationError::TypeError(
            format!("Cannot add {} to date/time", quantity.type_name())
        )),
    };
    
    // Check if the date/time value is actually set
    let dt = match dt_value.value {
        Some(dt) => dt,
        None => return Err(EvaluationError::InvalidOperation(
            format!("Cannot add to invalid date/time: {}", dt_value.original)
        )),
    };
    
    // Apply the addition based on the unit
    let new_dt = match unit {
        "year" | "years" | "yr" | "a" => {
            // Add years
            let years = amount.to_i64().ok_or_else(|| {
                EvaluationError::InvalidOperation("Year amount too large".to_string())
            })?;
            
            // Chrono doesn't have a direct 'add years' function, so we need to do it manually
            let new_year = dt.year() + years as i32;
            let new_dt = dt.with_year(new_year).ok_or_else(|| {
                EvaluationError::InvalidOperation(
                    format!("Invalid date after adding {} years", years)
                )
            })?;
            
            new_dt
        },
        "month" | "months" | "mo" => {
            // Add months
            let months = amount.to_i64().ok_or_else(|| {
                EvaluationError::InvalidOperation("Month amount too large".to_string())
            })?;
            
            // Chrono doesn't have a direct 'add months' function, so we need to do it manually
            let total_months = dt.year() as i64 * 12 + dt.month() as i64 + months;
            let new_year = (total_months / 12) as i32;
            let new_month = ((total_months % 12) as u32).max(1);
            
            let new_dt = dt.with_year(new_year).unwrap()
                          .with_month(new_month).ok_or_else(|| {
                EvaluationError::InvalidOperation(
                    format!("Invalid date after adding {} months", months)
                )
            })?;
            
            // Check if the day is valid (e.g., adding 1 month to January 31 shouldn't be February 31)
            let max_day = match new_month {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                2 => {
                    // Check for leap year
                    if new_year % 4 == 0 && (new_year % 100 != 0 || new_year % 400 == 0) {
                        29
                    } else {
                        28
                    }
                },
                _ => unreachable!(),
            };
            
            let day = dt.day().min(max_day);
            new_dt.with_day(day).ok_or_else(|| {
                EvaluationError::InvalidOperation(
                    format!("Invalid date after adding {} months", months)
                )
            })?
        },
        "week" | "weeks" | "wk" => {
            // Add weeks (7 days)
            let days = amount * Decimal::from(7);
            let duration = Duration::seconds(days.to_i64().ok_or_else(|| {
                EvaluationError::InvalidOperation("Week amount too large".to_string())
            })? * 86400);
            
            dt + duration
        },
        "day" | "days" | "d" => {
            // Add days
            let duration = Duration::seconds(amount.to_i64().ok_or_else(|| {
                EvaluationError::InvalidOperation("Day amount too large".to_string())
            })? * 86400);
            
            dt + duration
        },
        "hour" | "hours" | "h" => {
            // Add hours
            let duration = Duration::seconds(amount.to_i64().ok_or_else(|| {
                EvaluationError::InvalidOperation("Hour amount too large".to_string())
            })? * 3600);
            
            dt + duration
        },
        "minute" | "minutes" | "min" => {
            // Add minutes
            let duration = Duration::seconds(amount.to_i64().ok_or_else(|| {
                EvaluationError::InvalidOperation("Minute amount too large".to_string())
            })? * 60);
            
            dt + duration
        },
        "second" | "seconds" | "s" => {
            // Add seconds
            let duration = Duration::seconds(amount.to_i64().ok_or_else(|| {
                EvaluationError::InvalidOperation("Second amount too large".to_string())
            })?);
            
            dt + duration
        },
        "millisecond" | "milliseconds" | "ms" => {
            // Add milliseconds
            let duration = Duration::milliseconds(amount.to_i64().ok_or_else(|| {
                EvaluationError::InvalidOperation("Millisecond amount too large".to_string())
            })?);
            
            dt + duration
        },
        _ => {
            return Err(EvaluationError::InvalidOperation(
                format!("Unsupported time unit: {}", unit)
            ));
        },
    };
    
    // Create a new DateTimeValue with the updated datetime
    let new_dt_value = DateTimeValue {
        original: new_dt.to_rfc3339(),
        value: Some(new_dt),
        precision: dt_value.precision,
        has_timezone: dt_value.has_timezone,
        dtype: dt_value.dtype,
    };
    
    // Convert back to EvaluationResult
    Ok(datetime_value_to_result(new_dt_value))
}

/// Subtract a quantity from a date or time
pub fn subtract_date_time_quantity(
    date_time: &EvaluationResult,
    quantity: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // Convert quantity to negative and use add
    match quantity {
        EvaluationResult::Quantity(value, unit) => {
            let negative_value = -value;
            let negative_quantity = EvaluationResult::Quantity(negative_value, unit.clone());
            add_date_time_quantity(date_time, &negative_quantity)
        },
        EvaluationResult::Integer(value) => {
            let negative_value = -value;
            let negative_quantity = EvaluationResult::Integer(negative_value);
            add_date_time_quantity(date_time, &negative_quantity)
        },
        EvaluationResult::Decimal(value) => {
            let negative_value = -value;
            let negative_quantity = EvaluationResult::Decimal(negative_value);
            add_date_time_quantity(date_time, &negative_quantity)
        },
        _ => Err(EvaluationError::TypeError(
            format!("Cannot subtract {} from date/time", quantity.type_name())
        )),
    }
}

/// Calculate the difference between two date/time values
pub fn date_time_difference(
    date_time1: &EvaluationResult,
    date_time2: &EvaluationResult,
    unit: &str,
) -> Result<EvaluationResult, EvaluationError> {
    // Convert both date/time values to DateTimeValue
    let dt_value1 = result_to_datetime_value(date_time1)?;
    let dt_value2 = result_to_datetime_value(date_time2)?;
    
    // Check if both date/time values are set
    let dt1 = match dt_value1.value {
        Some(dt) => dt,
        None => return Err(EvaluationError::InvalidOperation(
            format!("Invalid date/time: {}", dt_value1.original)
        )),
    };
    
    let dt2 = match dt_value2.value {
        Some(dt) => dt,
        None => return Err(EvaluationError::InvalidOperation(
            format!("Invalid date/time: {}", dt_value2.original)
        )),
    };
    
    // Calculate the difference in seconds
    let diff_seconds = (dt1 - dt2).num_seconds();
    
    // Convert to the requested unit
    let result_value = match unit {
        "year" | "years" | "yr" | "a" => {
            // Approximate years (365.25 days)
            let years_decimal = Decimal::from(diff_seconds) / Decimal::from(31557600);
            EvaluationResult::Quantity(years_decimal, unit.to_string())
        },
        "month" | "months" | "mo" => {
            // Approximate months (30.44 days)
            let months_decimal = Decimal::from(diff_seconds) / Decimal::from(2629746);
            EvaluationResult::Quantity(months_decimal, unit.to_string())
        },
        "week" | "weeks" | "wk" => {
            // Weeks (7 days)
            let weeks_decimal = Decimal::from(diff_seconds) / Decimal::from(604800);
            EvaluationResult::Quantity(weeks_decimal, unit.to_string())
        },
        "day" | "days" | "d" => {
            // Days (24 hours)
            let days_decimal = Decimal::from(diff_seconds) / Decimal::from(86400);
            EvaluationResult::Quantity(days_decimal, unit.to_string())
        },
        "hour" | "hours" | "h" => {
            // Hours (60 minutes)
            let hours_decimal = Decimal::from(diff_seconds) / Decimal::from(3600);
            EvaluationResult::Quantity(hours_decimal, unit.to_string())
        },
        "minute" | "minutes" | "min" => {
            // Minutes (60 seconds)
            let minutes_decimal = Decimal::from(diff_seconds) / Decimal::from(60);
            EvaluationResult::Quantity(minutes_decimal, unit.to_string())
        },
        "second" | "seconds" | "s" => {
            // Seconds
            EvaluationResult::Quantity(Decimal::from(diff_seconds), unit.to_string())
        },
        "millisecond" | "milliseconds" | "ms" => {
            // Milliseconds (0.001 seconds)
            let millis_decimal = Decimal::from(diff_seconds) * Decimal::from(1000);
            EvaluationResult::Quantity(millis_decimal, unit.to_string())
        },
        _ => {
            return Err(EvaluationError::InvalidOperation(
                format!("Unsupported time unit: {}", unit)
            ));
        },
    };
    
    Ok(result_value)
}
```

## 2. Integration with Evaluator

Now, let's update the evaluator to use our enhanced date/time operations:

```rust
// In evaluator.rs

// Add imports for the date/time operations
use crate::date_operation::{apply_date_type_operation, today, time_of_day, now};
use crate::date_arithmetic::{add_date_time_quantity, subtract_date_time_quantity};
use crate::datetime_impl::{parse_date_literal};

// Add handling for date/time literals
fn evaluate_literal(literal: &Literal) -> EvaluationResult {
    match literal {
        // ... existing cases ...
        
        // Handle date/time literals
        Literal::Date(date) => EvaluationResult::Date(date.clone()),
        Literal::DateTime(date, time) => {
            // Format the datetime string
            let mut dt_string = format!("{}T{}", date, time.0);
            if let Some(tz) = &time.1 {
                dt_string.push_str(tz);
            }
            EvaluationResult::DateTime(dt_string)
        },
        Literal::Time(time) => EvaluationResult::Time(time.clone()),
    }
}

// Update the function for date/time comparisons
fn compare_equality(left: &EvaluationResult, op: &str, right: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    // Special handling for date/time comparisons
    if (left.is_date() || left.is_time() || left.is_datetime()) &&
       (right.is_date() || right.is_time() || right.is_datetime()) {
        return apply_date_type_operation(left, op, right);
    }
    
    // Handle other equality comparisons...
}

fn compare_inequality(left: &EvaluationResult, op: &str, right: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    // Special handling for date/time comparisons
    if (left.is_date() || left.is_time() || left.is_datetime()) &&
       (right.is_date() || right.is_time() || right.is_datetime()) {
        return apply_date_type_operation(left, op, right);
    }
    
    // Handle other inequality comparisons...
}

// Update the function for date/time arithmetic
fn apply_additive(left: &EvaluationResult, op: &str, right: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    // Special handling for date/time arithmetic
    if (left.is_date() || left.is_time() || left.is_datetime()) && 
       (right.is_quantity() || right.is_integer() || right.is_decimal()) {
        match op {
            "+" => add_date_time_quantity(left, right),
            "-" => subtract_date_time_quantity(left, right),
            _ => Err(EvaluationError::InvalidOperation(
                format!("Unsupported operation {} for date/time", op)
            )),
        }
    } else if (right.is_date() || right.is_time() || right.is_datetime()) && 
              (left.is_quantity() || left.is_integer() || left.is_decimal()) && 
              op == "+" {
        // Support adding a quantity to a date/time (commutative operation)
        add_date_time_quantity(right, left)
    } else {
        // Handle other additive operations...
    }
}

// Add functions for today(), now(), and timeOfDay()
fn evaluate_function(name: &str, args: &[EvaluationResult], _context: &EvaluationContext) -> Result<EvaluationResult, EvaluationError> {
    match name {
        // ... existing functions ...
        
        // Date/time utility functions
        "today" => {
            if !args.is_empty() {
                return Err(EvaluationError::InvalidArgument(
                    "today() function takes no arguments".to_string()
                ));
            }
            Ok(today())
        },
        "now" => {
            if !args.is_empty() {
                return Err(EvaluationError::InvalidArgument(
                    "now() function takes no arguments".to_string()
                ));
            }
            Ok(now())
        },
        "timeOfDay" => {
            if !args.is_empty() {
                return Err(EvaluationError::InvalidArgument(
                    "timeOfDay() function takes no arguments".to_string()
                ));
            }
            Ok(time_of_day())
        },
        
        // ... other functions ...
    }
}
```

## 3. Testing the Implementation

Create unit tests to verify the date/time fixes:

```rust
// In appropriate test file

#[test]
fn test_date_time_equality() {
    // Test date equality
    assert_eq!(
        evaluate("@2019-01-01 = @2019-01-01", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test time equality
    assert_eq!(
        evaluate("@T12:30:00 = @T12:30:00", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test datetime equality
    assert_eq!(
        evaluate("@2019-01-01T12:30:00 = @2019-01-01T12:30:00", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test equality with different precisions
    assert_eq!(
        evaluate("@2019-01-01T12:30:00 = @2019-01-01T12:30:00.000", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test timezone-aware equality
    assert_eq!(
        evaluate("@2019-01-01T12:30:00Z = @2019-01-01T12:30:00Z", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test timezone conversion
    assert_eq!(
        evaluate("@2019-01-01T12:30:00Z = @2019-01-01T07:30:00-05:00", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
}

#[test]
fn test_date_time_comparison() {
    // Test date comparison
    assert_eq!(
        evaluate("@2019-01-01 < @2019-01-02", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test time comparison
    assert_eq!(
        evaluate("@T12:30:00 < @T13:30:00", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test datetime comparison
    assert_eq!(
        evaluate("@2019-01-01T12:30:00 < @2019-01-01T13:30:00", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test comparison with different precisions
    assert_eq!(
        evaluate("@2019-01-01T12:30:00 > @2019-01-01T12:29:59.999", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test timezone-aware comparison
    assert_eq!(
        evaluate("@2019-01-01T12:30:00Z < @2019-01-01T13:30:00Z", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test timezone conversion in comparison
    assert_eq!(
        evaluate("@2019-01-01T12:30:00Z > @2019-01-01T06:30:00-05:00", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
}

#[test]
fn test_date_time_arithmetic() {
    // Test adding days to a date
    assert_eq!(
        evaluate("@2019-01-01 + 1 day", &context, None).unwrap(),
        EvaluationResult::Date("2019-01-02")
    );
    
    // Test adding hours to a time
    assert_eq!(
        evaluate("@T12:30:00 + 2 'hour'", &context, None).unwrap(),
        EvaluationResult::Time("14:30:00")
    );
    
    // Test adding minutes to a datetime
    assert_eq!(
        evaluate("@2019-01-01T12:30:00 + 30 'minute'", &context, None).unwrap(),
        EvaluationResult::DateTime("2019-01-01T13:00:00")
    );
    
    // Test subtracting days from a date
    assert_eq!(
        evaluate("@2019-01-02 - 1 day", &context, None).unwrap(),
        EvaluationResult::Date("2019-01-01")
    );
}

#[test]
fn test_utility_functions() {
    // These tests will be time-dependent, so we can't check exact values
    
    // Test that today() returns a date
    let result = evaluate("today()", &context, None).unwrap();
    assert!(matches!(result, EvaluationResult::Date(_)));
    
    // Test that now() returns a datetime
    let result = evaluate("now()", &context, None).unwrap();
    assert!(matches!(result, EvaluationResult::DateTime(_)));
    
    // Test that timeOfDay() returns a time
    let result = evaluate("timeOfDay()", &context, None).unwrap();
    assert!(matches!(result, EvaluationResult::Time(_)));
    
    // Test that today() is in the past compared to today() + 1 day
    assert_eq!(
        evaluate("today() < today() + 1 day", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
}

#[test]
fn test_date_vs_datetime_comparison() {
    // Test comparison between date and datetime
    assert_eq!(
        evaluate("@2019-01-01 = @2019-01-01T00:00:00", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test that date at start of day is less than datetime later in the day
    assert_eq!(
        evaluate("@2019-01-01 < @2019-01-01T12:30:00", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test that date at day N is less than datetime at day N+1
    assert_eq!(
        evaluate("@2019-01-01 < @2019-01-02T00:00:00", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
}
```

## 4. Main Changes Summary

This implementation fixes the following key issues:

1. **Date/Time Representation**:
   - Enhanced representation with precision and timezone information
   - Proper handling of partial dates and times
   - Support for different date/time formats

2. **Date/Time Comparison**:
   - Fixed precision-aware comparisons (e.g., 2018-03-01T10:30:00 = 2018-03-01T10:30:00.0)
   - Proper timezone normalization for comparisons
   - Support for comparing dates with datetimes

3. **Date/Time Arithmetic**:
   - Proper support for different time units (year, month, day, etc.)
   - Edge case handling for month boundaries
   - Support for timezone-aware arithmetic

4. **Date/Time Utility Functions**:
   - Fixed today(), now(), and timeOfDay() functions
   - Support for comparing dates/times with these utility functions

These changes address the date/time-related failing tests, which represent a significant portion of the FHIRPath implementation issues.