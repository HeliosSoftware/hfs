use crate::datetime_impl;
use chrono::{Datelike, Timelike};
use fhirpath_support::{EvaluationError, EvaluationResult};

/// Extract and return the year component from a date or datetime value
pub fn year_of(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Date(date_str, _) => {
            if let Some(date) = datetime_impl::parse_date(date_str) {
                Ok(EvaluationResult::integer(date.year() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid date format: {}",
                    date_str
                )))
            }
        }
        EvaluationResult::DateTime(dt_str, _) => {
            if let Some(dt) = datetime_impl::parse_datetime(dt_str) {
                Ok(EvaluationResult::integer(dt.year() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid datetime format: {}",
                    dt_str
                )))
            }
        }
        EvaluationResult::String(s, _) => {
            // Try to parse as date or datetime
            if let Some(date) = datetime_impl::parse_date(s) {
                Ok(EvaluationResult::integer(date.year() as i64))
            } else if let Some(dt) = datetime_impl::parse_datetime(s) {
                Ok(EvaluationResult::integer(dt.year() as i64))
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
        EvaluationResult::Date(date_str, _) => {
            if let Some(date) = datetime_impl::parse_date(date_str) {
                Ok(EvaluationResult::integer(date.month() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid date format: {}",
                    date_str
                )))
            }
        }
        EvaluationResult::DateTime(dt_str, _) => {
            if let Some(dt) = datetime_impl::parse_datetime(dt_str) {
                Ok(EvaluationResult::integer(dt.month() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid datetime format: {}",
                    dt_str
                )))
            }
        }
        EvaluationResult::String(s, _) => {
            // Try to parse as date or datetime
            if let Some(date) = datetime_impl::parse_date(s) {
                Ok(EvaluationResult::integer(date.month() as i64))
            } else if let Some(dt) = datetime_impl::parse_datetime(s) {
                Ok(EvaluationResult::integer(dt.month() as i64))
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
        EvaluationResult::Date(date_str, _) => {
            if let Some(date) = datetime_impl::parse_date(date_str) {
                Ok(EvaluationResult::integer(date.day() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid date format: {}",
                    date_str
                )))
            }
        }
        EvaluationResult::DateTime(dt_str, _) => {
            if let Some(dt) = datetime_impl::parse_datetime(dt_str) {
                Ok(EvaluationResult::integer(dt.day() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid datetime format: {}",
                    dt_str
                )))
            }
        }
        EvaluationResult::String(s, _) => {
            // Try to parse as date or datetime
            if let Some(date) = datetime_impl::parse_date(s) {
                Ok(EvaluationResult::integer(date.day() as i64))
            } else if let Some(dt) = datetime_impl::parse_datetime(s) {
                Ok(EvaluationResult::integer(dt.day() as i64))
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
        EvaluationResult::Time(time_str, _) => {
            if let Some(time) = datetime_impl::parse_time(time_str) {
                Ok(EvaluationResult::integer(time.hour() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid time format: {}",
                    time_str
                )))
            }
        }
        EvaluationResult::DateTime(dt_str, _) => {
            if let Some(dt) = datetime_impl::parse_datetime(dt_str) {
                Ok(EvaluationResult::integer(dt.hour() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid datetime format: {}",
                    dt_str
                )))
            }
        }
        EvaluationResult::String(s, _) => {
            // Try to parse as time or datetime
            if let Some(time) = datetime_impl::parse_time(s) {
                Ok(EvaluationResult::integer(time.hour() as i64))
            } else if let Some(dt) = datetime_impl::parse_datetime(s) {
                Ok(EvaluationResult::integer(dt.hour() as i64))
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
        EvaluationResult::Time(time_str, _) => {
            if let Some(time) = datetime_impl::parse_time(time_str) {
                Ok(EvaluationResult::integer(time.minute() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid time format: {}",
                    time_str
                )))
            }
        }
        EvaluationResult::DateTime(dt_str, _) => {
            if let Some(dt) = datetime_impl::parse_datetime(dt_str) {
                Ok(EvaluationResult::integer(dt.minute() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid datetime format: {}",
                    dt_str
                )))
            }
        }
        EvaluationResult::String(s, _) => {
            // Try to parse as time or datetime
            if let Some(time) = datetime_impl::parse_time(s) {
                Ok(EvaluationResult::integer(time.minute() as i64))
            } else if let Some(dt) = datetime_impl::parse_datetime(s) {
                Ok(EvaluationResult::integer(dt.minute() as i64))
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
        EvaluationResult::Time(time_str, _) => {
            if let Some(time) = datetime_impl::parse_time(time_str) {
                Ok(EvaluationResult::integer(time.second() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid time format: {}",
                    time_str
                )))
            }
        }
        EvaluationResult::DateTime(dt_str, _) => {
            if let Some(dt) = datetime_impl::parse_datetime(dt_str) {
                Ok(EvaluationResult::integer(dt.second() as i64))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid datetime format: {}",
                    dt_str
                )))
            }
        }
        EvaluationResult::String(s, _) => {
            // Try to parse as time or datetime
            if let Some(time) = datetime_impl::parse_time(s) {
                Ok(EvaluationResult::integer(time.second() as i64))
            } else if let Some(dt) = datetime_impl::parse_datetime(s) {
                Ok(EvaluationResult::integer(dt.second() as i64))
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
        EvaluationResult::Time(time_str, _) => {
            if let Some(time) = datetime_impl::parse_time(time_str) {
                Ok(EvaluationResult::integer(
                    (time.nanosecond() / 1_000_000) as i64,
                ))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid time format: {}",
                    time_str
                )))
            }
        }
        EvaluationResult::DateTime(dt_str, _) => {
            if let Some(dt) = datetime_impl::parse_datetime(dt_str) {
                Ok(EvaluationResult::integer(
                    (dt.nanosecond() / 1_000_000) as i64,
                ))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "Invalid datetime format: {}",
                    dt_str
                )))
            }
        }
        EvaluationResult::String(s, _) => {
            // Try to parse as time or datetime
            if let Some(time) = datetime_impl::parse_time(s) {
                Ok(EvaluationResult::integer(
                    (time.nanosecond() / 1_000_000) as i64,
                ))
            } else if let Some(dt) = datetime_impl::parse_datetime(s) {
                Ok(EvaluationResult::integer(
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
        let date = EvaluationResult::date("2023-05-15".to_string());
        let datetime = EvaluationResult::datetime("2023-05-15T14:30:45.123".to_string());
        let string_date = EvaluationResult::string("2023-05-15".to_string());

        assert_eq!(year_of(&date).unwrap(), EvaluationResult::integer(2023));
        assert_eq!(year_of(&datetime).unwrap(), EvaluationResult::integer(2023));
        assert_eq!(
            year_of(&string_date).unwrap(),
            EvaluationResult::integer(2023)
        );

        // Test with invalid values
        let invalid_string = EvaluationResult::string("not-a-date".to_string());
        let number = EvaluationResult::integer(2023);

        assert!(year_of(&invalid_string).is_err());
        assert!(year_of(&number).is_err());
    }

    #[test]
    fn test_month_of() {
        let date = EvaluationResult::date("2023-05-15".to_string());
        let datetime = EvaluationResult::datetime("2023-05-15T14:30:45.123".to_string());

        assert_eq!(month_of(&date).unwrap(), EvaluationResult::integer(5));
        assert_eq!(month_of(&datetime).unwrap(), EvaluationResult::integer(5));
    }

    #[test]
    fn test_day_of() {
        let date = EvaluationResult::date("2023-05-15".to_string());
        let datetime = EvaluationResult::datetime("2023-05-15T14:30:45.123".to_string());

        assert_eq!(day_of(&date).unwrap(), EvaluationResult::integer(15));
        assert_eq!(day_of(&datetime).unwrap(), EvaluationResult::integer(15));
    }

    #[test]
    fn test_hour_of() {
        let time = EvaluationResult::time("14:30:45".to_string());
        let datetime = EvaluationResult::datetime("2023-05-15T14:30:45.123".to_string());

        assert_eq!(hour_of(&time).unwrap(), EvaluationResult::integer(14));
        assert_eq!(hour_of(&datetime).unwrap(), EvaluationResult::integer(14));
    }

    #[test]
    fn test_minute_of() {
        let time = EvaluationResult::time("14:30:45".to_string());
        let datetime = EvaluationResult::datetime("2023-05-15T14:30:45.123".to_string());

        assert_eq!(minute_of(&time).unwrap(), EvaluationResult::integer(30));
        assert_eq!(minute_of(&datetime).unwrap(), EvaluationResult::integer(30));
    }

    #[test]
    fn test_second_of() {
        let time = EvaluationResult::time("14:30:45".to_string());
        let datetime = EvaluationResult::datetime("2023-05-15T14:30:45.123".to_string());

        assert_eq!(second_of(&time).unwrap(), EvaluationResult::integer(45));
        assert_eq!(second_of(&datetime).unwrap(), EvaluationResult::integer(45));
    }

    #[test]
    fn test_millisecond_of() {
        let time = EvaluationResult::time("14:30:45.123".to_string());
        let datetime = EvaluationResult::datetime("2023-05-15T14:30:45.123".to_string());

        assert_eq!(
            millisecond_of(&time).unwrap(),
            EvaluationResult::integer(123)
        );
        assert_eq!(
            millisecond_of(&datetime).unwrap(),
            EvaluationResult::integer(123)
        );
    }
}
