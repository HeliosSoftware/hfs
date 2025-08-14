use crate::datetime_impl;
use helios_fhirpath_support::{EvaluationError, EvaluationResult};

/// Check if a value is of a particular date/time type
/// Handles both 'is' and 'as' operations
pub fn apply_date_type_operation(
    value: &EvaluationResult,
    op: &str,
    type_name: &str,
    _namespace: Option<&str>,
) -> Result<EvaluationResult, EvaluationError> {
    match op {
        "is" => {
            // Handle date literal checks (e.g., @2015.is(Date))
            match value {
                // Check Date, DateTime, Time types first
                EvaluationResult::Date(_, _) => Ok(EvaluationResult::boolean(
                    type_name == "Date" || type_name == "date",
                )),
                EvaluationResult::DateTime(_, _) => Ok(EvaluationResult::boolean(
                    type_name == "DateTime" || type_name == "dateTime",
                )),
                EvaluationResult::Time(_, _) => Ok(EvaluationResult::boolean(
                    type_name == "Time" || type_name == "time",
                )),
                // Various date literals in string form
                EvaluationResult::String(s, _) if s.starts_with('@') => {
                    // Extract the actual date/time string by removing the leading @
                    let date_value = s.trim_start_matches('@');

                    // Check format and match against requested type
                    match type_name {
                        "Date" | "date" => {
                            // Valid date formats: YYYY, YYYY-MM, YYYY-MM-DD
                            let is_date = date_value.len() == 4 || // YYYY
                                          date_value.len() == 7 || // YYYY-MM
                                          date_value.len() == 10; // YYYY-MM-DD

                            // Also check that it doesn't contain a T (which would make it a datetime)
                            let is_date = is_date && !date_value.contains('T');

                            Ok(EvaluationResult::boolean(is_date))
                        }
                        "DateTime" | "dateTime" => {
                            // Valid datetime format: contains a T
                            let is_datetime = date_value.contains('T');
                            Ok(EvaluationResult::boolean(is_datetime))
                        }
                        "Time" | "time" => {
                            // Valid time format: starts with T
                            let is_time = date_value.starts_with('T');
                            Ok(EvaluationResult::boolean(is_time))
                        }
                        _ => Ok(EvaluationResult::boolean(false)),
                    }
                }

                // Not a date/time value
                _ => Ok(EvaluationResult::boolean(false)),
            }
        }
        "as" => {
            // Check if the value is of the target type
            let is_result = apply_date_type_operation(value, "is", type_name, _namespace)?;
            match is_result {
                EvaluationResult::Boolean(true, _) => {
                    // Value is already of the target type, return as is
                    Ok(value.clone())
                }
                _ => {
                    // Value is not of the target type, try to convert it
                    match (type_name, value) {
                        // Try to convert to date
                        ("Date" | "date", EvaluationResult::String(s, _)) => {
                            if let Some(date) = datetime_impl::parse_date(s) {
                                Ok(EvaluationResult::date(date.format("%Y-%m-%d").to_string()))
                            } else {
                                Ok(EvaluationResult::Empty)
                            }
                        }
                        ("Date" | "date", EvaluationResult::DateTime(dt, None)) => {
                            // Extract date part from datetime
                            if let Some(date) =
                                datetime_impl::to_date(&EvaluationResult::datetime(dt.clone()))
                            {
                                Ok(EvaluationResult::date(date))
                            } else {
                                Ok(EvaluationResult::Empty)
                            }
                        }

                        // Try to convert to datetime
                        ("DateTime" | "dateTime", EvaluationResult::String(s, _)) => {
                            if let Some(dt) = datetime_impl::parse_datetime(s) {
                                Ok(EvaluationResult::datetime(
                                    dt.format("%Y-%m-%dT%H:%M:%S").to_string(),
                                ))
                            } else {
                                Ok(EvaluationResult::Empty)
                            }
                        }
                        ("DateTime" | "dateTime", EvaluationResult::Date(d, _)) => {
                            // Convert date to datetime by adding T00:00:00
                            if let Some(dt) =
                                datetime_impl::to_datetime(&EvaluationResult::date(d.clone()))
                            {
                                Ok(EvaluationResult::datetime(dt))
                            } else {
                                Ok(EvaluationResult::Empty)
                            }
                        }

                        // Try to convert to time
                        ("Time" | "time", EvaluationResult::String(s, _)) => {
                            if let Some(time) = datetime_impl::parse_time(s) {
                                Ok(EvaluationResult::time(time.format("%H:%M:%S").to_string()))
                            } else {
                                Ok(EvaluationResult::Empty)
                            }
                        }

                        // Cannot convert to the target type
                        _ => Ok(EvaluationResult::Empty),
                    }
                }
            }
        }
        _ => Err(EvaluationError::TypeError(format!(
            "Unsupported date type operation: {}",
            op
        ))),
    }
}

/// Parse date literals from @ notation
/// For example, @2015, @2015-02, @2015-02-04, @2015-02-04T14:30, @T14:30
pub fn parse_date_literal(literal: &str) -> Result<EvaluationResult, EvaluationError> {
    if !literal.starts_with('@') {
        return Err(EvaluationError::TypeError(format!(
            "Not a date/time literal: {}",
            literal
        )));
    }

    let value = literal.trim_start_matches('@');

    // Check if it's a date, datetime, or time
    if value.starts_with('T') {
        // Time literal: @T14:30
        if let Some(time) = datetime_impl::parse_time(value.trim_start_matches('T')) {
            Ok(EvaluationResult::time(time.format("%H:%M:%S").to_string()))
        } else {
            Err(EvaluationError::TypeError(format!(
                "Invalid time literal: {}",
                literal
            )))
        }
    } else if value.contains('T') {
        // DateTime literal: @2015-02-04T14:30
        if let Some(dt) = datetime_impl::parse_datetime(value) {
            Ok(EvaluationResult::datetime(
                dt.format("%Y-%m-%dT%H:%M:%S").to_string(),
            ))
        } else {
            Err(EvaluationError::TypeError(format!(
                "Invalid datetime literal: {}",
                literal
            )))
        }
    } else {
        // Date literal: @2015, @2015-02, @2015-02-04
        if let Some(date) = datetime_impl::parse_date(value) {
            Ok(EvaluationResult::date(date.format("%Y-%m-%d").to_string()))
        } else {
            Err(EvaluationError::TypeError(format!(
                "Invalid date literal: {}",
                literal
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_date_is_operation() {
        // Test @2015.is(Date)
        let year_literal = EvaluationResult::string("@2015".to_string());
        let result = apply_date_type_operation(&year_literal, "is", "Date", None).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));

        // Test @2015-02.is(Date)
        let month_literal = EvaluationResult::string("@2015-02".to_string());
        let result = apply_date_type_operation(&month_literal, "is", "Date", None).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));

        // Test @2015-02-04.is(Date)
        let day_literal = EvaluationResult::string("@2015-02-04".to_string());
        let result = apply_date_type_operation(&day_literal, "is", "Date", None).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));

        // Test @2015T.is(DateTime)
        let year_dt_literal = EvaluationResult::string("@2015T".to_string());
        let result = apply_date_type_operation(&year_dt_literal, "is", "DateTime", None).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));

        // Test @T14:30.is(Time)
        let time_literal = EvaluationResult::string("@T14:30".to_string());
        let result = apply_date_type_operation(&time_literal, "is", "Time", None).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_parse_date_literal() {
        // Test @2015
        let result = parse_date_literal("@2015").unwrap();
        assert_eq!(result, EvaluationResult::date("2015-01-01".to_string()));

        // Test @2015-02
        let result = parse_date_literal("@2015-02").unwrap();
        assert_eq!(result, EvaluationResult::date("2015-02-01".to_string()));

        // Test @2015-02-04
        let result = parse_date_literal("@2015-02-04").unwrap();
        assert_eq!(result, EvaluationResult::date("2015-02-04".to_string()));

        // Test @2015-02-04T14:30
        let result = parse_date_literal("@2015-02-04T14:30").unwrap();
        assert_eq!(
            result,
            EvaluationResult::datetime("2015-02-04T14:30:00".to_string())
        );

        // Test @T14:30
        let result = parse_date_literal("@T14:30").unwrap();
        assert_eq!(result, EvaluationResult::time("14:30:00".to_string()));
    }
}
