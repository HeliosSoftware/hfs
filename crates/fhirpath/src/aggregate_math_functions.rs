//! # FHIRPath Aggregate Math Functions
//!
//! Implements the aggregate math functions: `sum()`, `min()`, `max()`, `avg()`.

use helios_fhirpath_support::{EvaluationError, EvaluationResult};
use rust_decimal::Decimal;
use std::cmp::Ordering;

/// Extracts items from an EvaluationResult into a Vec.
fn extract_items(input: &EvaluationResult) -> Vec<&EvaluationResult> {
    match input {
        EvaluationResult::Collection { items, .. } => items.iter().collect(),
        EvaluationResult::Empty => vec![],
        single_item => vec![single_item],
    }
}

/// Compares two EvaluationResults of compatible types.
/// Returns None if the types are incomparable.
fn compare_values(a: &EvaluationResult, b: &EvaluationResult) -> Option<Ordering> {
    match (a, b) {
        (EvaluationResult::Integer(a, _, _), EvaluationResult::Integer(b, _, _)) => Some(a.cmp(b)),
        (EvaluationResult::Integer64(a, _, _), EvaluationResult::Integer64(b, _, _)) => {
            Some(a.cmp(b))
        }
        (EvaluationResult::Decimal(a, _, _), EvaluationResult::Decimal(b, _, _)) => Some(a.cmp(b)),
        // Mixed numeric: promote to Decimal
        (EvaluationResult::Integer(a, _, _), EvaluationResult::Decimal(b, _, _)) => {
            Some(Decimal::from(*a).cmp(b))
        }
        (EvaluationResult::Decimal(a, _, _), EvaluationResult::Integer(b, _, _)) => {
            Some(a.cmp(&Decimal::from(*b)))
        }
        (EvaluationResult::Integer(a, _, _), EvaluationResult::Integer64(b, _, _)) => {
            Some(a.cmp(b))
        }
        (EvaluationResult::Integer64(a, _, _), EvaluationResult::Integer(b, _, _)) => {
            Some(a.cmp(b))
        }
        // Quantity comparison (same unit only)
        (
            EvaluationResult::Quantity(val_a, unit_a, _, _),
            EvaluationResult::Quantity(val_b, unit_b, _, _),
        ) => {
            if unit_a == unit_b {
                Some(val_a.cmp(val_b))
            } else {
                None
            }
        }
        // String comparison
        (EvaluationResult::String(a, _, _), EvaluationResult::String(b, _, _)) => Some(a.cmp(b)),
        // Date/Time comparisons
        (EvaluationResult::Date(a, _, _), EvaluationResult::Date(b, _, _)) => Some(a.cmp(b)),
        (EvaluationResult::DateTime(a, _, _), EvaluationResult::DateTime(b, _, _)) => {
            Some(a.cmp(b))
        }
        (EvaluationResult::Time(a, _, _), EvaluationResult::Time(b, _, _)) => Some(a.cmp(b)),
        _ => None,
    }
}

/// Implements the FHIRPath `sum()` function.
///
/// Returns the sum of all items in the collection.
/// If the collection is empty, returns 0 (Integer).
/// Items must be Integer, Decimal, or Quantity (same unit).
pub fn sum_function(
    invocation_base: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    let items = extract_items(invocation_base);

    if items.is_empty() {
        return Ok(EvaluationResult::integer(0));
    }

    let mut acc = items[0].clone();

    for item in &items[1..] {
        acc = add_values(&acc, item)?;
    }

    Ok(acc)
}

/// Implements the FHIRPath `min()` function.
///
/// Returns the minimum value in the collection.
/// If the collection is empty, returns Empty.
pub fn min_function(
    invocation_base: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    let items = extract_items(invocation_base);

    if items.is_empty() {
        return Ok(EvaluationResult::Empty);
    }

    let mut min_val = items[0];

    for item in &items[1..] {
        match compare_values(min_val, item) {
            Some(Ordering::Greater) => min_val = item,
            Some(_) => {}
            None => {
                return Err(EvaluationError::TypeError(format!(
                    "min() cannot compare {} and {}",
                    min_val.type_name(),
                    item.type_name()
                )));
            }
        }
    }

    Ok(min_val.clone())
}

/// Implements the FHIRPath `max()` function.
///
/// Returns the maximum value in the collection.
/// If the collection is empty, returns Empty.
pub fn max_function(
    invocation_base: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    let items = extract_items(invocation_base);

    if items.is_empty() {
        return Ok(EvaluationResult::Empty);
    }

    let mut max_val = items[0];

    for item in &items[1..] {
        match compare_values(max_val, item) {
            Some(Ordering::Less) => max_val = item,
            Some(_) => {}
            None => {
                return Err(EvaluationError::TypeError(format!(
                    "max() cannot compare {} and {}",
                    max_val.type_name(),
                    item.type_name()
                )));
            }
        }
    }

    Ok(max_val.clone())
}

/// Implements the FHIRPath `avg()` function.
///
/// Returns the average of all items in the collection.
/// If the collection is empty, returns Empty.
/// Always returns Decimal (or Quantity).
pub fn avg_function(
    invocation_base: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    let items = extract_items(invocation_base);

    if items.is_empty() {
        return Ok(EvaluationResult::Empty);
    }

    let count = Decimal::from(items.len() as i64);
    let sum = sum_function(invocation_base)?;

    match sum {
        EvaluationResult::Integer(v, _, _) => {
            Ok(EvaluationResult::decimal(Decimal::from(v) / count))
        }
        EvaluationResult::Integer64(v, _, _) => {
            Ok(EvaluationResult::decimal(Decimal::from(v) / count))
        }
        EvaluationResult::Decimal(v, _, _) => Ok(EvaluationResult::decimal(v / count)),
        EvaluationResult::Quantity(v, unit, _, _) => {
            Ok(EvaluationResult::quantity(v / count, unit))
        }
        _ => Err(EvaluationError::TypeError(
            "avg() requires numeric or quantity items".to_string(),
        )),
    }
}

/// Adds two numeric or quantity values together.
fn add_values(
    a: &EvaluationResult,
    b: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    match (a, b) {
        (EvaluationResult::Integer(a, _, _), EvaluationResult::Integer(b, _, _)) => {
            Ok(EvaluationResult::integer(a + b))
        }
        (EvaluationResult::Integer64(a, _, _), EvaluationResult::Integer64(b, _, _)) => {
            Ok(EvaluationResult::integer64(*a + *b))
        }
        (EvaluationResult::Decimal(a, _, _), EvaluationResult::Decimal(b, _, _)) => {
            Ok(EvaluationResult::decimal(*a + *b))
        }
        // Mixed numeric: promote to Decimal
        (EvaluationResult::Integer(a, _, _), EvaluationResult::Decimal(b, _, _)) => {
            Ok(EvaluationResult::decimal(Decimal::from(*a) + *b))
        }
        (EvaluationResult::Decimal(a, _, _), EvaluationResult::Integer(b, _, _)) => {
            Ok(EvaluationResult::decimal(*a + Decimal::from(*b)))
        }
        (EvaluationResult::Integer(a, _, _), EvaluationResult::Integer64(b, _, _)) => {
            Ok(EvaluationResult::integer64(*a + *b))
        }
        (EvaluationResult::Integer64(a, _, _), EvaluationResult::Integer(b, _, _)) => {
            Ok(EvaluationResult::integer64(*a + *b))
        }
        // Quantity addition (same unit)
        (
            EvaluationResult::Quantity(val_a, unit_a, _, _),
            EvaluationResult::Quantity(val_b, unit_b, _, _),
        ) => {
            if unit_a == unit_b {
                Ok(EvaluationResult::quantity(*val_a + *val_b, unit_a.clone()))
            } else {
                Err(EvaluationError::TypeError(format!(
                    "sum() cannot add quantities with different units: '{}' and '{}'",
                    unit_a, unit_b
                )))
            }
        }
        _ => Err(EvaluationError::TypeError(format!(
            "sum() requires numeric or quantity items, found {} and {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_sum_integers() {
        let input = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = sum_function(&input).unwrap();
        assert_eq!(result, EvaluationResult::integer(6));
    }

    #[test]
    fn test_sum_decimals() {
        let input = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::decimal(dec!(1.5)),
                EvaluationResult::decimal(dec!(2.5)),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = sum_function(&input).unwrap();
        assert_eq!(result, EvaluationResult::decimal(dec!(4.0)));
    }

    #[test]
    fn test_sum_mixed_int_decimal() {
        let input = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::decimal(dec!(2.5)),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = sum_function(&input).unwrap();
        assert_eq!(result, EvaluationResult::decimal(dec!(3.5)));
    }

    #[test]
    fn test_sum_empty() {
        let result = sum_function(&EvaluationResult::Empty).unwrap();
        assert_eq!(result, EvaluationResult::integer(0));
    }

    #[test]
    fn test_sum_quantities() {
        let input = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::quantity(dec!(5), "mg".to_string()),
                EvaluationResult::quantity(dec!(3), "mg".to_string()),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = sum_function(&input).unwrap();
        assert_eq!(
            result,
            EvaluationResult::quantity(dec!(8), "mg".to_string())
        );
    }

    #[test]
    fn test_sum_quantities_different_units() {
        let input = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::quantity(dec!(5), "mg".to_string()),
                EvaluationResult::quantity(dec!(3), "kg".to_string()),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        assert!(sum_function(&input).is_err());
    }

    #[test]
    fn test_min_integers() {
        let input = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(3),
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = min_function(&input).unwrap();
        assert_eq!(result, EvaluationResult::integer(1));
    }

    #[test]
    fn test_min_empty() {
        let result = min_function(&EvaluationResult::Empty).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_max_integers() {
        let input = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(3),
                EvaluationResult::integer(2),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = max_function(&input).unwrap();
        assert_eq!(result, EvaluationResult::integer(3));
    }

    #[test]
    fn test_max_empty() {
        let result = max_function(&EvaluationResult::Empty).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_avg_integers() {
        let input = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = avg_function(&input).unwrap();
        assert_eq!(result, EvaluationResult::decimal(dec!(2)));
    }

    #[test]
    fn test_avg_decimals() {
        let input = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::decimal(dec!(1.5)),
                EvaluationResult::decimal(dec!(2.5)),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = avg_function(&input).unwrap();
        assert_eq!(result, EvaluationResult::decimal(dec!(2.0)));
    }

    #[test]
    fn test_avg_empty() {
        let result = avg_function(&EvaluationResult::Empty).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_avg_quantities() {
        let input = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::quantity(dec!(4), "mg".to_string()),
                EvaluationResult::quantity(dec!(6), "mg".to_string()),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = avg_function(&input).unwrap();
        assert_eq!(
            result,
            EvaluationResult::quantity(dec!(5), "mg".to_string())
        );
    }

    #[test]
    fn test_min_strings() {
        let input = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::string("banana".to_string()),
                EvaluationResult::string("apple".to_string()),
                EvaluationResult::string("cherry".to_string()),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = min_function(&input).unwrap();
        assert_eq!(result, EvaluationResult::string("apple".to_string()));
    }

    #[test]
    fn test_max_strings() {
        let input = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::string("banana".to_string()),
                EvaluationResult::string("apple".to_string()),
                EvaluationResult::string("cherry".to_string()),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = max_function(&input).unwrap();
        assert_eq!(result, EvaluationResult::string("cherry".to_string()));
    }

    #[test]
    fn test_min_dates() {
        let input = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::date("2025-03-01".to_string()),
                EvaluationResult::date("2025-01-01".to_string()),
                EvaluationResult::date("2025-02-01".to_string()),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = min_function(&input).unwrap();
        assert_eq!(result, EvaluationResult::date("2025-01-01".to_string()));
    }

    #[test]
    fn test_sum_single_item() {
        let input = EvaluationResult::integer(42);
        let result = sum_function(&input).unwrap();
        assert_eq!(result, EvaluationResult::integer(42));
    }

    #[test]
    fn test_min_single_item() {
        let input = EvaluationResult::integer(42);
        let result = min_function(&input).unwrap();
        assert_eq!(result, EvaluationResult::integer(42));
    }
}
