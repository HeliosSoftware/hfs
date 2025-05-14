use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc, Datelike};
use fhirpath_support::{EvaluationError, EvaluationResult};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use crate::datetime_impl;

/// Add a quantity to a date or datetime
/// Handles time-based quantities like '1 month' or '2 days'
pub fn add_date_time_quantity(
    left: &EvaluationResult,
    right: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // Extract time-based unit and value from right operand (quantity)
    let (value, unit) = match right {
        EvaluationResult::Quantity(val, unit) => (*val, unit.clone()),
        EvaluationResult::Integer(i) => (Decimal::from(*i), "day".to_string()), // Assume days if no unit provided
        _ => {
            return Err(EvaluationError::TypeError(format!(
                "Cannot add {} to date or time",
                right.type_name()
            )))
        }
    };

    // Convert numeric value to integer
    let amount = if value.is_integer() {
        value.to_i64().ok_or_else(|| {
            EvaluationError::TypeError(format!("Quantity value too large: {}", value))
        })?
    } else {
        return Err(EvaluationError::TypeError(format!(
            "Cannot add fractional time units: {} {}",
            value, unit
        )));
    };

    // Process the date/time operation based on left type and unit
    match left {
        EvaluationResult::Date(date_str) => {
            if let Some(date) = datetime_impl::parse_date(date_str) {
                let result = match unit.as_str() {
                    "year" | "years" => add_years_to_date(date, amount),
                    "month" | "months" => add_months_to_date(date, amount),
                    "week" | "weeks" => date.checked_add_signed(Duration::days(amount * 7)),
                    "day" | "days" => date.checked_add_signed(Duration::days(amount)),
                    _ => {
                        return Err(EvaluationError::TypeError(format!(
                            "Cannot add {} to date",
                            unit
                        )))
                    }
                };

                match result {
                    Some(new_date) => Ok(EvaluationResult::Date(new_date.format("%Y-%m-%d").to_string())),
                    None => Err(EvaluationError::TypeError("Date arithmetic overflow".to_string())),
                }
            } else {
                Err(EvaluationError::TypeError(format!("Invalid date: {}", date_str)))
            }
        },
        EvaluationResult::DateTime(dt_str) => {
            if let Some(dt) = datetime_impl::parse_datetime(dt_str) {
                let result = match unit.as_str() {
                    "year" | "years" => add_years_to_datetime(dt, amount),
                    "month" | "months" => add_months_to_datetime(dt, amount),
                    "week" | "weeks" => dt.checked_add_signed(Duration::days(amount * 7)),
                    "day" | "days" => dt.checked_add_signed(Duration::days(amount)),
                    "hour" | "hours" => dt.checked_add_signed(Duration::hours(amount)),
                    "minute" | "minutes" => dt.checked_add_signed(Duration::minutes(amount)),
                    "second" | "seconds" => dt.checked_add_signed(Duration::seconds(amount)),
                    "millisecond" | "milliseconds" => dt.checked_add_signed(Duration::milliseconds(amount)),
                    _ => {
                        return Err(EvaluationError::TypeError(format!(
                            "Cannot add {} to datetime",
                            unit
                        )))
                    }
                };

                match result {
                    Some(new_dt) => Ok(EvaluationResult::DateTime(
                        new_dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    )),
                    None => Err(EvaluationError::TypeError("DateTime arithmetic overflow".to_string())),
                }
            } else {
                Err(EvaluationError::TypeError(format!("Invalid datetime: {}", dt_str)))
            }
        },
        EvaluationResult::Time(time_str) => {
            if let Some(time) = datetime_impl::parse_time(time_str) {
                let result = match unit.as_str() {
                    "hour" | "hours" => time.overflowing_add_signed(Duration::hours(amount)).0,
                    "minute" | "minutes" => time.overflowing_add_signed(Duration::minutes(amount)).0,
                    "second" | "seconds" => time.overflowing_add_signed(Duration::seconds(amount)).0,
                    "millisecond" | "milliseconds" => time.overflowing_add_signed(Duration::milliseconds(amount)).0,
                    _ => {
                        return Err(EvaluationError::TypeError(format!(
                            "Cannot add {} to time",
                            unit
                        )))
                    }
                };

                // Format the new time
                Ok(EvaluationResult::Time(
                    result.format("%H:%M:%S%.3f").to_string(),
                ))
            } else {
                Err(EvaluationError::TypeError(format!("Invalid time: {}", time_str)))
            }
        },
        _ => Err(EvaluationError::TypeError(format!(
            "Cannot add a quantity to {}",
            left.type_name()
        ))),
    }
}

/// Subtract a quantity from a date or datetime
/// Handles time-based quantities like '1 month' or '2 days'
pub fn subtract_date_time_quantity(
    left: &EvaluationResult,
    right: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // Extract time-based unit and value from right operand (quantity)
    let (value, unit) = match right {
        EvaluationResult::Quantity(val, unit) => (*val, unit.clone()),
        EvaluationResult::Integer(i) => (Decimal::from(*i), "day".to_string()), // Assume days if no unit provided
        _ => {
            return Err(EvaluationError::TypeError(format!(
                "Cannot subtract {} from date or time",
                right.type_name()
            )))
        }
    };

    // Convert numeric value to integer
    let amount = if value.is_integer() {
        value.to_i64().ok_or_else(|| {
            EvaluationError::TypeError(format!("Quantity value too large: {}", value))
        })?
    } else {
        return Err(EvaluationError::TypeError(format!(
            "Cannot subtract fractional time units: {} {}",
            value, unit
        )));
    };

    // Use the add function with the negated amount
    add_date_time_quantity(left, &EvaluationResult::Quantity(
        Decimal::from(-amount),  // Negate the amount for subtraction
        unit,
    ))
}

/// Calculate the difference between two dates or datetimes
/// Returns a quantity with the appropriate unit
pub fn date_time_difference(
    left: &EvaluationResult,
    right: &EvaluationResult,
    unit: &str,
) -> Result<EvaluationResult, EvaluationError> {
    match (left, right) {
        (EvaluationResult::Date(date1), EvaluationResult::Date(date2)) => {
            let d1 = datetime_impl::parse_date(date1)
                .ok_or_else(|| EvaluationError::TypeError(format!("Invalid date: {}", date1)))?;
            let d2 = datetime_impl::parse_date(date2)
                .ok_or_else(|| EvaluationError::TypeError(format!("Invalid date: {}", date2)))?;
            
            let diff = match unit {
                "year" | "years" => calculate_year_difference(d1, d2),
                "month" | "months" => calculate_month_difference(d1, d2),
                "week" | "weeks" => {
                    let days = (d1.signed_duration_since(d2)).num_days();
                    Decimal::from(days / 7)
                },
                "day" | "days" => {
                    let days = (d1.signed_duration_since(d2)).num_days();
                    Decimal::from(days)
                },
                _ => return Err(EvaluationError::TypeError(format!(
                    "Cannot calculate date difference in {}", unit
                ))),
            };
            
            Ok(EvaluationResult::Quantity(diff, unit.to_string()))
        },
        (EvaluationResult::DateTime(dt1), EvaluationResult::DateTime(dt2)) => {
            let d1 = datetime_impl::parse_datetime(dt1)
                .ok_or_else(|| EvaluationError::TypeError(format!("Invalid datetime: {}", dt1)))?;
            let d2 = datetime_impl::parse_datetime(dt2)
                .ok_or_else(|| EvaluationError::TypeError(format!("Invalid datetime: {}", dt2)))?;
            
            let diff = match unit {
                "year" | "years" => calculate_year_difference(d1.date_naive(), d2.date_naive()),
                "month" | "months" => calculate_month_difference(d1.date_naive(), d2.date_naive()),
                "week" | "weeks" => {
                    let days = (d1.signed_duration_since(d2)).num_days();
                    Decimal::from(days / 7)
                },
                "day" | "days" => {
                    let days = (d1.signed_duration_since(d2)).num_days();
                    Decimal::from(days)
                },
                "hour" | "hours" => {
                    let hours = (d1.signed_duration_since(d2)).num_hours();
                    Decimal::from(hours)
                },
                "minute" | "minutes" => {
                    let minutes = (d1.signed_duration_since(d2)).num_minutes();
                    Decimal::from(minutes)
                },
                "second" | "seconds" => {
                    let seconds = (d1.signed_duration_since(d2)).num_seconds();
                    Decimal::from(seconds)
                },
                "millisecond" | "milliseconds" => {
                    let millis = (d1.signed_duration_since(d2)).num_milliseconds();
                    Decimal::from(millis)
                },
                _ => return Err(EvaluationError::TypeError(format!(
                    "Cannot calculate datetime difference in {}", unit
                ))),
            };
            
            Ok(EvaluationResult::Quantity(diff, unit.to_string()))
        },
        (EvaluationResult::Time(time1), EvaluationResult::Time(time2)) => {
            let t1 = datetime_impl::parse_time(time1)
                .ok_or_else(|| EvaluationError::TypeError(format!("Invalid time: {}", time1)))?;
            let t2 = datetime_impl::parse_time(time2)
                .ok_or_else(|| EvaluationError::TypeError(format!("Invalid time: {}", time2)))?;
            
            // Create NaiveDateTime with a dummy date to calculate duration
            let dt1 = NaiveDateTime::new(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(), t1);
            let dt2 = NaiveDateTime::new(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(), t2);
            
            let diff = match unit {
                "hour" | "hours" => {
                    let hours = (dt1.signed_duration_since(dt2)).num_hours();
                    Decimal::from(hours)
                },
                "minute" | "minutes" => {
                    let minutes = (dt1.signed_duration_since(dt2)).num_minutes();
                    Decimal::from(minutes)
                },
                "second" | "seconds" => {
                    let seconds = (dt1.signed_duration_since(dt2)).num_seconds();
                    Decimal::from(seconds)
                },
                "millisecond" | "milliseconds" => {
                    let millis = (dt1.signed_duration_since(dt2)).num_milliseconds();
                    Decimal::from(millis)
                },
                _ => return Err(EvaluationError::TypeError(format!(
                    "Cannot calculate time difference in {}", unit
                ))),
            };
            
            Ok(EvaluationResult::Quantity(diff, unit.to_string()))
        },
        _ => Err(EvaluationError::TypeError(format!(
            "Cannot calculate difference between {} and {}",
            left.type_name(), right.type_name()
        ))),
    }
}

/// Helper function to add years to a date
fn add_years_to_date(date: NaiveDate, years: i64) -> Option<NaiveDate> {
    let new_year = date.year() + years as i32;
    
    // Handle leap years for February 29
    if date.month() == 2 && date.day() == 29 {
        // Check if the new year is a leap year
        if !is_leap_year(new_year) {
            // If not, use February 28 instead
            NaiveDate::from_ymd_opt(new_year, 2, 28)
        } else {
            NaiveDate::from_ymd_opt(new_year, date.month(), date.day())
        }
    } else {
        NaiveDate::from_ymd_opt(new_year, date.month(), date.day())
    }
}

/// Helper function to add years to a datetime
fn add_years_to_datetime(dt: DateTime<Utc>, years: i64) -> Option<DateTime<Utc>> {
    add_years_to_date(dt.date_naive(), years).map(|new_date_naive| {
        let naive_dt = NaiveDateTime::new(new_date_naive, dt.time());
        DateTime::<Utc>::from_naive_utc_and_offset(naive_dt, Utc)
    })
}

/// Helper function to add months to a date
fn add_months_to_date(date: NaiveDate, months: i64) -> Option<NaiveDate> {
    let total_months = date.year() as i64 * 12 + date.month() as i64 - 1 + months;
    
    let new_year = total_months.div_euclid(12) as i32;
    let new_month = total_months.rem_euclid(12) as u32 + 1;
    
    // Handle month length differences
    let max_day = get_days_in_month(new_year, new_month);
    let new_day = std::cmp::min(date.day(), max_day);
    
    NaiveDate::from_ymd_opt(new_year, new_month, new_day)
}

/// Helper function to add months to a datetime
fn add_months_to_datetime(dt: DateTime<Utc>, months: i64) -> Option<DateTime<Utc>> {
    add_months_to_date(dt.date_naive(), months).map(|new_date_naive| {
        let naive_dt = NaiveDateTime::new(new_date_naive, dt.time());
        DateTime::<Utc>::from_naive_utc_and_offset(naive_dt, Utc)
    })
}

/// Helper function to check if a year is a leap year
fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Helper function to get the number of days in a month
fn get_days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap_year(year) { 29 } else { 28 },
        _ => 0, // Invalid month
    }
}

/// Helper function to calculate the difference in years between two dates
fn calculate_year_difference(date1: NaiveDate, date2: NaiveDate) -> Decimal {
    let years = date1.year() - date2.year();
    
    // Adjust if we haven't reached the anniversary yet
    let adjustment = if (date1.month(), date1.day()) < (date2.month(), date2.day()) {
        -1
    } else {
        0
    };
    
    Decimal::from(years + adjustment)
}

/// Helper function to calculate the difference in months between two dates
fn calculate_month_difference(date1: NaiveDate, date2: NaiveDate) -> Decimal {
    let year_diff = date1.year() - date2.year();
    let month_diff = date1.month() as i32 - date2.month() as i32;
    let total_months = year_diff * 12 + month_diff;
    
    // Adjust if we haven't reached the monthly anniversary yet
    let adjustment = if date1.day() < date2.day() { -1 } else { 0 };
    
    Decimal::from(total_months + adjustment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    
    #[test]
    fn test_add_days_to_date() {
        let date = EvaluationResult::Date("2023-05-15".to_string());
        let quantity = EvaluationResult::Quantity(dec!(5), "day".to_string());
        
        let result = add_date_time_quantity(&date, &quantity).unwrap();
        assert_eq!(result, EvaluationResult::Date("2023-05-20".to_string()));
    }
    
    #[test]
    fn test_add_months_to_date() {
        let date = EvaluationResult::Date("2023-01-31".to_string());
        let quantity = EvaluationResult::Quantity(dec!(1), "month".to_string());
        
        let result = add_date_time_quantity(&date, &quantity).unwrap();
        // Should handle month length differences - Jan 31 + 1 month = Feb 28
        assert_eq!(result, EvaluationResult::Date("2023-02-28".to_string()));
    }
    
    #[test]
    fn test_add_years_to_date() {
        // Test leap year handling
        let leap_date = EvaluationResult::Date("2020-02-29".to_string());
        let quantity = EvaluationResult::Quantity(dec!(1), "year".to_string());
        
        let result = add_date_time_quantity(&leap_date, &quantity).unwrap();
        // Should handle leap years - Feb 29, 2020 + 1 year = Feb 28, 2021
        assert_eq!(result, EvaluationResult::Date("2021-02-28".to_string()));
    }
    
    #[test]
    fn test_add_time_to_datetime() {
        let datetime = EvaluationResult::DateTime("2023-05-15T14:30:45".to_string());
        let quantity = EvaluationResult::Quantity(dec!(30), "minute".to_string());
        
        let result = add_date_time_quantity(&datetime, &quantity).unwrap();
        assert_eq!(result, EvaluationResult::DateTime("2023-05-15T15:00:45.000".to_string()));
    }
    
    #[test]
    fn test_subtract_days_from_date() {
        let date = EvaluationResult::Date("2023-05-15".to_string());
        let quantity = EvaluationResult::Quantity(dec!(5), "day".to_string());
        
        let result = subtract_date_time_quantity(&date, &quantity).unwrap();
        assert_eq!(result, EvaluationResult::Date("2023-05-10".to_string()));
    }
    
    #[test]
    fn test_date_difference_in_days() {
        let date1 = EvaluationResult::Date("2023-05-15".to_string());
        let date2 = EvaluationResult::Date("2023-05-10".to_string());
        
        let result = date_time_difference(&date1, &date2, "day").unwrap();
        assert_eq!(result, EvaluationResult::Quantity(dec!(5), "day".to_string()));
    }
    
    #[test]
    fn test_datetime_difference_in_hours() {
        let dt1 = EvaluationResult::DateTime("2023-05-15T14:30:00".to_string());
        let dt2 = EvaluationResult::DateTime("2023-05-15T12:30:00".to_string());
        
        let result = date_time_difference(&dt1, &dt2, "hour").unwrap();
        assert_eq!(result, EvaluationResult::Quantity(dec!(2), "hour".to_string()));
    }
    
    #[test]
    fn test_time_difference() {
        let time1 = EvaluationResult::Time("14:30:00".to_string());
        let time2 = EvaluationResult::Time("12:30:00".to_string());
        
        let result = date_time_difference(&time1, &time2, "hour").unwrap();
        assert_eq!(result, EvaluationResult::Quantity(dec!(2), "hour".to_string()));
    }
}
