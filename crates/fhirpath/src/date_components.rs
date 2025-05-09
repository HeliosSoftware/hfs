use crate::datetime_impl;
use chrono::{Datelike, Timelike};
use fhirpath_support::{EvaluationError, EvaluationResult};

/// Extract and return the year component from a date or datetime value
pub fn year_of(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Date(date_str) => {
            if let Some(date) = datetime_impl::parse_date(date_str) {
                Ok(EvaluationResult::Integer(date.year() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid date format: {}",
                    date_str
                )))
            }
        }
        EvaluationResult::DateTime(dt_str) => {
            if let Some(dt) = datetime_impl::parse_datetime(dt_str) {
                Ok(EvaluationResult::Integer(dt.year() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid datetime format: {}",
                    dt_str
                )))
            }
        }
        EvaluationResult::String(s) => {
            // Try to parse as date or datetime
            if let Some(date) = datetime_impl::parse_date(s) {
                Ok(EvaluationResult::Integer(date.year() as i64))
            } else if let Some(dt) = datetime_impl::parse_datetime(s) {
                Ok(EvaluationResult::Integer(dt.year() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Cannot extract year from string: {}",
                    s
                )))
            }
        }
        _ => Err(EvaluationError::TypeError(format!(
            "Cannot extract year from {}",
            value.type_name()
        ))),
    }
}

/// Extract and return the month component from a date or datetime value
pub fn month_of(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Date(date_str) => {
            if let Some(date) = datetime_impl::parse_date(date_str) {
                Ok(EvaluationResult::Integer(date.month() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid date format: {}",
                    date_str
                )))
            }
        }
        EvaluationResult::DateTime(dt_str) => {
            if let Some(dt) = datetime_impl::parse_datetime(dt_str) {
                Ok(EvaluationResult::Integer(dt.month() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid datetime format: {}",
                    dt_str
                )))
            }
        }
        EvaluationResult::String(s) => {
            // Try to parse as date or datetime
            if let Some(date) = datetime_impl::parse_date(s) {
                Ok(EvaluationResult::Integer(date.month() as i64))
            } else if let Some(dt) = datetime_impl::parse_datetime(s) {
                Ok(EvaluationResult::Integer(dt.month() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Cannot extract month from string: {}",
                    s
                )))
            }
        }
        _ => Err(EvaluationError::TypeError(format!(
            "Cannot extract month from {}",
            value.type_name()
        ))),
    }
}

/// Extract and return the day component from a date or datetime value
pub fn day_of(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Date(date_str) => {
            if let Some(date) = datetime_impl::parse_date(date_str) {
                Ok(EvaluationResult::Integer(date.day() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid date format: {}",
                    date_str
                )))
            }
        }
        EvaluationResult::DateTime(dt_str) => {
            if let Some(dt) = datetime_impl::parse_datetime(dt_str) {
                Ok(EvaluationResult::Integer(dt.day() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid datetime format: {}",
                    dt_str
                )))
            }
        }
        EvaluationResult::String(s) => {
            // Try to parse as date or datetime
            if let Some(date) = datetime_impl::parse_date(s) {
                Ok(EvaluationResult::Integer(date.day() as i64))
            } else if let Some(dt) = datetime_impl::parse_datetime(s) {
                Ok(EvaluationResult::Integer(dt.day() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Cannot extract day from string: {}",
                    s
                )))
            }
        }
        _ => Err(EvaluationError::TypeError(format!(
            "Cannot extract day from {}",
            value.type_name()
        ))),
    }
}

/// Extract and return the hour component from a time or datetime value
pub fn hour_of(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Time(time_str) => {
            if let Some(time) = datetime_impl::parse_time(time_str) {
                Ok(EvaluationResult::Integer(time.hour() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid time format: {}",
                    time_str
                )))
            }
        }
        EvaluationResult::DateTime(dt_str) => {
            if let Some(dt) = datetime_impl::parse_datetime(dt_str) {
                Ok(EvaluationResult::Integer(dt.hour() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid datetime format: {}",
                    dt_str
                )))
            }
        }
        EvaluationResult::String(s) => {
            // Try to parse as time or datetime
            if let Some(time) = datetime_impl::parse_time(s) {
                Ok(EvaluationResult::Integer(time.hour() as i64))
            } else if let Some(dt) = datetime_impl::parse_datetime(s) {
                Ok(EvaluationResult::Integer(dt.hour() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Cannot extract hour from string: {}",
                    s
                )))
            }
        }
        _ => Err(EvaluationError::TypeError(format!(
            "Cannot extract hour from {}",
            value.type_name()
        ))),
    }
}

/// Extract and return the minute component from a time or datetime value
pub fn minute_of(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Time(time_str) => {
            if let Some(time) = datetime_impl::parse_time(time_str) {
                Ok(EvaluationResult::Integer(time.minute() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid time format: {}",
                    time_str
                )))
            }
        }
        EvaluationResult::DateTime(dt_str) => {
            if let Some(dt) = datetime_impl::parse_datetime(dt_str) {
                Ok(EvaluationResult::Integer(dt.minute() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid datetime format: {}",
                    dt_str
                )))
            }
        }
        EvaluationResult::String(s) => {
            // Try to parse as time or datetime
            if let Some(time) = datetime_impl::parse_time(s) {
                Ok(EvaluationResult::Integer(time.minute() as i64))
            } else if let Some(dt) = datetime_impl::parse_datetime(s) {
                Ok(EvaluationResult::Integer(dt.minute() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Cannot extract minute from string: {}",
                    s
                )))
            }
        }
        _ => Err(EvaluationError::TypeError(format!(
            "Cannot extract minute from {}",
            value.type_name()
        ))),
    }
}

/// Extract and return the second component from a time or datetime value
pub fn second_of(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Time(time_str) => {
            if let Some(time) = datetime_impl::parse_time(time_str) {
                Ok(EvaluationResult::Integer(time.second() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid time format: {}",
                    time_str
                )))
            }
        }
        EvaluationResult::DateTime(dt_str) => {
            if let Some(dt) = datetime_impl::parse_datetime(dt_str) {
                Ok(EvaluationResult::Integer(dt.second() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid datetime format: {}",
                    dt_str
                )))
            }
        }
        EvaluationResult::String(s) => {
            // Try to parse as time or datetime
            if let Some(time) = datetime_impl::parse_time(s) {
                Ok(EvaluationResult::Integer(time.second() as i64))
            } else if let Some(dt) = datetime_impl::parse_datetime(s) {
                Ok(EvaluationResult::Integer(dt.second() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Cannot extract second from string: {}",
                    s
                )))
            }
        }
        _ => Err(EvaluationError::TypeError(format!(
            "Cannot extract second from {}",
            value.type_name()
        ))),
    }
}

/// Extract and return the millisecond component from a time or datetime value
pub fn millisecond_of(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Time(time_str) => {
            if let Some(time) = datetime_impl::parse_time(time_str) {
                Ok(EvaluationResult::Integer(
                    (time.nanosecond() / 1_000_000) as i64,
                ))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid time format: {}",
                    time_str
                )))
            }
        }
        EvaluationResult::DateTime(dt_str) => {
            if let Some(dt) = datetime_impl::parse_datetime(dt_str) {
                Ok(EvaluationResult::Integer(
                    (dt.nanosecond() / 1_000_000) as i64,
                ))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid datetime format: {}",
                    dt_str
                )))
            }
        }
        EvaluationResult::String(s) => {
            // Try to parse as time or datetime
            if let Some(time) = datetime_impl::parse_time(s) {
                Ok(EvaluationResult::Integer(
                    (time.nanosecond() / 1_000_000) as i64,
                ))
            } else if let Some(dt) = datetime_impl::parse_datetime(s) {
                Ok(EvaluationResult::Integer(
                    (dt.nanosecond() / 1_000_000) as i64,
                ))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Cannot extract millisecond from string: {}",
                    s
                )))
            }
        }
        _ => Err(EvaluationError::TypeError(format!(
            "Cannot extract millisecond from {}",
            value.type_name()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_year_of() {
        let date = EvaluationResult::Date("2023-05-15".to_string());
        let datetime = EvaluationResult::DateTime("2023-05-15T14:30:45.123".to_string());
        let string_date = EvaluationResult::String("2023-05-15".to_string());

        assert_eq!(year_of(&date).unwrap(), EvaluationResult::Integer(2023));
        assert_eq!(year_of(&datetime).unwrap(), EvaluationResult::Integer(2023));
        assert_eq!(
            year_of(&string_date).unwrap(),
            EvaluationResult::Integer(2023)
        );

        // Test with invalid values
        let invalid_string = EvaluationResult::String("not-a-date".to_string());
        let number = EvaluationResult::Integer(2023);

        assert!(year_of(&invalid_string).is_err());
        assert!(year_of(&number).is_err());
    }

    #[test]
    fn test_month_of() {
        let date = EvaluationResult::Date("2023-05-15".to_string());
        let datetime = EvaluationResult::DateTime("2023-05-15T14:30:45.123".to_string());

        assert_eq!(month_of(&date).unwrap(), EvaluationResult::Integer(5));
        assert_eq!(month_of(&datetime).unwrap(), EvaluationResult::Integer(5));
    }

    #[test]
    fn test_day_of() {
        let date = EvaluationResult::Date("2023-05-15".to_string());
        let datetime = EvaluationResult::DateTime("2023-05-15T14:30:45.123".to_string());

        assert_eq!(day_of(&date).unwrap(), EvaluationResult::Integer(15));
        assert_eq!(day_of(&datetime).unwrap(), EvaluationResult::Integer(15));
    }

    #[test]
    fn test_hour_of() {
        let time = EvaluationResult::Time("14:30:45".to_string());
        let datetime = EvaluationResult::DateTime("2023-05-15T14:30:45.123".to_string());

        assert_eq!(hour_of(&time).unwrap(), EvaluationResult::Integer(14));
        assert_eq!(hour_of(&datetime).unwrap(), EvaluationResult::Integer(14));
    }

    #[test]
    fn test_minute_of() {
        let time = EvaluationResult::Time("14:30:45".to_string());
        let datetime = EvaluationResult::DateTime("2023-05-15T14:30:45.123".to_string());

        assert_eq!(minute_of(&time).unwrap(), EvaluationResult::Integer(30));
        assert_eq!(minute_of(&datetime).unwrap(), EvaluationResult::Integer(30));
    }

    #[test]
    fn test_second_of() {
        let time = EvaluationResult::Time("14:30:45".to_string());
        let datetime = EvaluationResult::DateTime("2023-05-15T14:30:45.123".to_string());

        assert_eq!(second_of(&time).unwrap(), EvaluationResult::Integer(45));
        assert_eq!(second_of(&datetime).unwrap(), EvaluationResult::Integer(45));
    }

    #[test]
    fn test_millisecond_of() {
        let time = EvaluationResult::Time("14:30:45.123".to_string());
        let datetime = EvaluationResult::DateTime("2023-05-15T14:30:45.123".to_string());

        assert_eq!(
            millisecond_of(&time).unwrap(),
            EvaluationResult::Integer(123)
        );
        assert_eq!(
            millisecond_of(&datetime).unwrap(),
            EvaluationResult::Integer(123)
        );
    }
}

