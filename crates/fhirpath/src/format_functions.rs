//! # FHIRPath Format Functions
//!
//! Implements format code support for `toDate()`, `toDateTime()`, and `toString()`.
//! Translates FHIRPath format codes to chrono format specifiers.

use crate::datetime_impl;
use chrono::NaiveDate;
use helios_fhirpath_support::{EvaluationError, EvaluationResult};

/// Translates FHIRPath format codes to chrono format specifiers.
///
/// FHIRPath format codes:
/// - `yyyy` → `%Y` (4-digit year)
/// - `yy` → `%y` (2-digit year)
/// - `MM` → `%m` (2-digit month)
/// - `M` → `%-m` (month, no padding)
/// - `dd` → `%d` (2-digit day)
/// - `d` → `%-d` (day, no padding)
/// - `HH` → `%H` (24-hour hour)
/// - `mm` → `%M` (minute)
/// - `ss` → `%S` (second)
/// - `fff` or `S+` → `%3f` (fractional seconds, 3 digits)
/// - `XXX` → `%:z` (timezone offset with colon)
fn translate_format_codes(fhirpath_format: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = fhirpath_format.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Try to match multi-character tokens first (longest match)
        if i + 4 <= chars.len() && &fhirpath_format[i..i + 4] == "yyyy" {
            result.push_str("%Y");
            i += 4;
        } else if i + 3 <= chars.len() && &fhirpath_format[i..i + 3] == "XXX" {
            result.push_str("%:z");
            i += 3;
        } else if i + 3 <= chars.len() && &fhirpath_format[i..i + 3] == "fff" {
            result.push_str("%3f");
            i += 3;
        } else if i + 2 <= chars.len() && &fhirpath_format[i..i + 2] == "yy" {
            result.push_str("%y");
            i += 2;
        } else if i + 2 <= chars.len() && &fhirpath_format[i..i + 2] == "MM" {
            result.push_str("%m");
            i += 2;
        } else if i + 2 <= chars.len() && &fhirpath_format[i..i + 2] == "dd" {
            result.push_str("%d");
            i += 2;
        } else if i + 2 <= chars.len() && &fhirpath_format[i..i + 2] == "HH" {
            result.push_str("%H");
            i += 2;
        } else if i + 2 <= chars.len() && &fhirpath_format[i..i + 2] == "mm" {
            result.push_str("%M");
            i += 2;
        } else if i + 2 <= chars.len() && &fhirpath_format[i..i + 2] == "ss" {
            result.push_str("%S");
            i += 2;
        } else if chars[i] == 'M' {
            result.push_str("%-m");
            i += 1;
        } else if chars[i] == 'd' {
            result.push_str("%-d");
            i += 1;
        } else if chars[i] == 'S' {
            // Handle S+ (one or more S's for fractional seconds)
            let mut count = 0;
            while i < chars.len() && chars[i] == 'S' {
                count += 1;
                i += 1;
            }
            // Map to chrono fractional format
            match count {
                1 => result.push_str("%1f"),
                2 => result.push_str("%2f"),
                3 => result.push_str("%3f"),
                _ => result.push_str("%f"),
            }
        } else {
            // Pass through literal characters (delimiters like /, -, T, :, etc.)
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Formats a Date value using the given format string.
pub fn format_date(date_str: &str, format: &str) -> Result<EvaluationResult, EvaluationError> {
    let chrono_fmt = translate_format_codes(format);
    let date = datetime_impl::parse_date(date_str).ok_or_else(|| {
        EvaluationError::InvalidArgument(format!("Cannot parse date: {}", date_str))
    })?;
    Ok(EvaluationResult::string(
        date.format(&chrono_fmt).to_string(),
    ))
}

/// Formats a DateTime value using the given format string.
pub fn format_datetime(
    datetime_str: &str,
    format: &str,
) -> Result<EvaluationResult, EvaluationError> {
    let chrono_fmt = translate_format_codes(format);
    let dt = datetime_impl::parse_datetime(datetime_str).ok_or_else(|| {
        EvaluationError::InvalidArgument(format!("Cannot parse datetime: {}", datetime_str))
    })?;
    Ok(EvaluationResult::string(dt.format(&chrono_fmt).to_string()))
}

/// Formats a Time value using the given format string.
pub fn format_time(time_str: &str, format: &str) -> Result<EvaluationResult, EvaluationError> {
    let chrono_fmt = translate_format_codes(format);
    let time = datetime_impl::parse_time(time_str).ok_or_else(|| {
        EvaluationError::InvalidArgument(format!("Cannot parse time: {}", time_str))
    })?;
    Ok(EvaluationResult::string(
        time.format(&chrono_fmt).to_string(),
    ))
}

/// Parses a string to Date using the given format string.
pub fn parse_date_with_format(
    input: &str,
    format: &str,
) -> Result<EvaluationResult, EvaluationError> {
    let chrono_fmt = translate_format_codes(format);
    match NaiveDate::parse_from_str(input, &chrono_fmt) {
        Ok(date) => Ok(EvaluationResult::date(date.format("%Y-%m-%d").to_string())),
        Err(_) => Ok(EvaluationResult::Empty),
    }
}

/// Parses a string to DateTime using the given format string.
pub fn parse_datetime_with_format(
    input: &str,
    format: &str,
) -> Result<EvaluationResult, EvaluationError> {
    let chrono_fmt = translate_format_codes(format);
    // Try with timezone first
    if let Ok(dt) = chrono::DateTime::parse_from_str(input, &chrono_fmt) {
        return Ok(EvaluationResult::datetime(
            dt.with_timezone(&chrono::Utc)
                .format("%Y-%m-%dT%H:%M:%S%:z")
                .to_string(),
        ));
    }
    // Try as NaiveDateTime
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(input, &chrono_fmt) {
        return Ok(EvaluationResult::datetime(
            ndt.format("%Y-%m-%dT%H:%M:%S").to_string(),
        ));
    }
    // Try as NaiveDate (for date-only format strings applied to datetime conversion)
    if let Ok(date) = NaiveDate::parse_from_str(input, &chrono_fmt) {
        return Ok(EvaluationResult::datetime(
            date.format("%Y-%m-%d").to_string(),
        ));
    }
    Ok(EvaluationResult::Empty)
}

/// Formats any EvaluationResult to a string using the given format.
pub fn to_string_with_format(
    value: &EvaluationResult,
    format: &str,
) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Date(d, _, _) => format_date(d, format),
        EvaluationResult::DateTime(dt, _, _) => format_datetime(dt, format),
        EvaluationResult::Time(t, _, _) => format_time(t, format),
        EvaluationResult::Integer(v, _, _) => {
            // For integers, format isn't really applicable; just convert to string
            Ok(EvaluationResult::string(v.to_string()))
        }
        EvaluationResult::Decimal(v, _, _) => Ok(EvaluationResult::string(v.to_string())),
        EvaluationResult::String(s, _, _) => {
            // Strings pass through
            Ok(EvaluationResult::string(s.clone()))
        }
        EvaluationResult::Empty => Ok(EvaluationResult::Empty),
        _ => Ok(EvaluationResult::string(value.to_string_value())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_translate_basic() {
        assert_eq!(translate_format_codes("yyyy"), "%Y");
        assert_eq!(translate_format_codes("MM"), "%m");
        assert_eq!(translate_format_codes("dd"), "%d");
        assert_eq!(translate_format_codes("yyyy-MM-dd"), "%Y-%m-%d");
    }

    #[test]
    fn test_translate_datetime() {
        assert_eq!(
            translate_format_codes("yyyy-MM-ddTHH:mm:ss"),
            "%Y-%m-%dT%H:%M:%S"
        );
    }

    #[test]
    fn test_translate_unpadded() {
        assert_eq!(translate_format_codes("M/d/yyyy"), "%-m/%-d/%Y");
    }

    #[test]
    fn test_format_date() {
        let result = format_date("2025-01-15", "yyyy").unwrap();
        assert_eq!(result, EvaluationResult::string("2025".to_string()));
    }

    #[test]
    fn test_format_date_full() {
        let result = format_date("2025-01-15", "MM/dd/yyyy").unwrap();
        assert_eq!(result, EvaluationResult::string("01/15/2025".to_string()));
    }

    #[test]
    fn test_parse_date_with_format() {
        let result = parse_date_with_format("01/15/2025", "MM/dd/yyyy").unwrap();
        assert_eq!(result, EvaluationResult::date("2025-01-15".to_string()));
    }

    #[test]
    fn test_format_date_unpadded() {
        let result = format_date("2025-01-05", "M/d/yyyy").unwrap();
        assert_eq!(result, EvaluationResult::string("1/5/2025".to_string()));
    }

    #[test]
    fn test_to_string_date_with_format() {
        let value = EvaluationResult::date("2025-01-15".to_string());
        let result = to_string_with_format(&value, "yyyy").unwrap();
        assert_eq!(result, EvaluationResult::string("2025".to_string()));
    }
}
