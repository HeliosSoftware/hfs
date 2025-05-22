use fhirpath_support::{EvaluationError, EvaluationResult};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
// The FromStr trait is actually used via the parse() method which requires this trait in scope
#[allow(unused_imports)]
use std::str::FromStr;

/// Implements the FHIRPath toDecimal() function
///
/// Converts the input to a Decimal value according to FHIRPath rules.
/// Returns Empty for non-convertible types or when conversion fails.
///
/// # Arguments
///
/// * `invocation_base` - The input value to convert to Decimal
///
/// # Returns
///
/// * `Ok(Decimal)` - The input converted to a Decimal value
/// * `Ok(Empty)` - If the input is Empty or cannot be converted to Decimal
/// * `Err` - If an error occurs, such as when the input is a multi-item collection
pub fn to_decimal_function(
    invocation_base: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // Check for singleton
    if invocation_base.count() > 1 {
        return Err(EvaluationError::SingletonEvaluationError(
            "toDecimal requires a singleton input".to_string(),
        ));
    }

    // Handle each type according to FHIRPath rules
    Ok(match invocation_base {
        EvaluationResult::Empty => EvaluationResult::Empty,
        EvaluationResult::Boolean(b) => {
            EvaluationResult::Decimal(if *b { Decimal::ONE } else { Decimal::ZERO })
        }
        EvaluationResult::Integer(i) => EvaluationResult::Decimal(Decimal::from(*i)),
        EvaluationResult::Decimal(d) => EvaluationResult::Decimal(*d),
        EvaluationResult::String(s) => {
            // Try parsing as Decimal
            s.parse::<Decimal>()
                .map(EvaluationResult::Decimal)
                .unwrap_or(EvaluationResult::Empty) // Return Empty if parsing fails
        }
        EvaluationResult::Quantity(val, _) => EvaluationResult::Decimal(*val),
        // Collections are handled by the count check above
        EvaluationResult::Collection { .. } => unreachable!(),
        // Other types are not convertible to Decimal
        _ => EvaluationResult::Empty,
    })
}

/// Implements the FHIRPath toInteger() function
///
/// Converts the input to an Integer value according to FHIRPath rules.
/// Returns Empty for non-convertible types or when conversion fails.
///
/// # Arguments
///
/// * `invocation_base` - The input value to convert to Integer
///
/// # Returns
///
/// * `Ok(Integer)` - The input converted to an Integer value
/// * `Ok(Empty)` - If the input is Empty or cannot be converted to Integer
/// * `Err` - If an error occurs, such as when the input is a multi-item collection
pub fn to_integer_function(
    invocation_base: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // Check for singleton
    if invocation_base.count() > 1 {
        return Err(EvaluationError::SingletonEvaluationError(
            "toInteger requires a singleton input".to_string(),
        ));
    }

    // Handle each type according to FHIRPath rules
    Ok(match invocation_base {
        EvaluationResult::Empty => EvaluationResult::Empty,
        EvaluationResult::Boolean(b) => EvaluationResult::Integer(if *b { 1 } else { 0 }),
        EvaluationResult::Integer(i) => EvaluationResult::Integer(*i),
        EvaluationResult::String(s) => {
            // Try parsing as i64
            s.parse::<i64>()
                .map(EvaluationResult::Integer)
                .unwrap_or(EvaluationResult::Empty) // Return Empty if parsing fails
        }
        // Per FHIRPath spec, Decimal cannot be converted to Integer via toInteger()
        EvaluationResult::Decimal(_) => EvaluationResult::Empty,
        // Quantity to Integer (returns value if integer, else empty)
        EvaluationResult::Quantity(val, _) => {
            if val.is_integer() {
                val.to_i64()
                    .map(EvaluationResult::Integer)
                    .unwrap_or(EvaluationResult::Empty)
            } else {
                EvaluationResult::Empty
            }
        }
        // Collections are handled by the count check above
        EvaluationResult::Collection { .. } => unreachable!(),
        // Other types are not convertible to Integer
        _ => EvaluationResult::Empty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_decimal_empty() {
        // Test toDecimal on Empty
        let empty = EvaluationResult::Empty;
        let result = to_decimal_function(&empty).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_to_decimal_boolean() {
        // Test toDecimal on Boolean values
        let true_val = EvaluationResult::Boolean(true);
        let result = to_decimal_function(&true_val).unwrap();
        assert_eq!(result, EvaluationResult::Decimal(Decimal::ONE));

        let false_val = EvaluationResult::Boolean(false);
        let result = to_decimal_function(&false_val).unwrap();
        assert_eq!(result, EvaluationResult::Decimal(Decimal::ZERO));
    }

    #[test]
    fn test_to_decimal_integer() {
        // Test toDecimal on Integer
        let int_val = EvaluationResult::Integer(42);
        let result = to_decimal_function(&int_val).unwrap();
        assert_eq!(result, EvaluationResult::Decimal(Decimal::from(42)));
    }

    #[test]
    fn test_to_decimal_decimal() {
        // Test toDecimal on Decimal
        let decimal = Decimal::from_str("3.14159").unwrap();
        let decimal_val = EvaluationResult::Decimal(decimal);
        let result = to_decimal_function(&decimal_val).unwrap();
        assert_eq!(result, decimal_val);
    }

    #[test]
    fn test_to_decimal_string_valid() {
        // Test toDecimal on valid String
        let string_val = EvaluationResult::String("3.14159".to_string());
        let result = to_decimal_function(&string_val).unwrap();
        assert_eq!(
            result,
            EvaluationResult::Decimal(Decimal::from_str("3.14159").unwrap())
        );
    }

    #[test]
    fn test_to_decimal_string_invalid() {
        // Test toDecimal on invalid String
        let string_val = EvaluationResult::String("not a number".to_string());
        let result = to_decimal_function(&string_val).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_to_decimal_quantity() {
        // Test toDecimal on Quantity
        let decimal = Decimal::from_str("3.14159").unwrap();
        let quantity_val = EvaluationResult::Quantity(decimal, "m".to_string());
        let result = to_decimal_function(&quantity_val).unwrap();
        assert_eq!(result, EvaluationResult::Decimal(decimal));
    }

    #[test]
    fn test_to_decimal_collection() {
        // Test toDecimal on multi-item collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::Integer(1),
                EvaluationResult::Integer(2),
            ],
            has_undefined_order: false,
        };
        let result = to_decimal_function(&collection);
        assert!(result.is_err());
    }

    #[test]
    fn test_to_integer_empty() {
        // Test toInteger on Empty
        let empty = EvaluationResult::Empty;
        let result = to_integer_function(&empty).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_to_integer_boolean() {
        // Test toInteger on Boolean values
        let true_val = EvaluationResult::Boolean(true);
        let result = to_integer_function(&true_val).unwrap();
        assert_eq!(result, EvaluationResult::Integer(1));

        let false_val = EvaluationResult::Boolean(false);
        let result = to_integer_function(&false_val).unwrap();
        assert_eq!(result, EvaluationResult::Integer(0));
    }

    #[test]
    fn test_to_integer_integer() {
        // Test toInteger on Integer
        let int_val = EvaluationResult::Integer(42);
        let result = to_integer_function(&int_val).unwrap();
        assert_eq!(result, int_val);
    }

    #[test]
    fn test_to_integer_decimal() {
        // Test toInteger on Decimal - should return Empty per spec
        let decimal = Decimal::from_str("3.14159").unwrap();
        let decimal_val = EvaluationResult::Decimal(decimal);
        let result = to_integer_function(&decimal_val).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_to_integer_string_valid() {
        // Test toInteger on valid Integer String
        let string_val = EvaluationResult::String("42".to_string());
        let result = to_integer_function(&string_val).unwrap();
        assert_eq!(result, EvaluationResult::Integer(42));
    }

    #[test]
    fn test_to_integer_string_invalid() {
        // Test toInteger on invalid or decimal String
        let string_val = EvaluationResult::String("not a number".to_string());
        let result = to_integer_function(&string_val).unwrap();
        assert_eq!(result, EvaluationResult::Empty);

        let decimal_string = EvaluationResult::String("3.14".to_string());
        let result = to_integer_function(&decimal_string).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_to_integer_quantity_integer() {
        // Test toInteger on Quantity with integer value
        let decimal = Decimal::from(42);
        let quantity_val = EvaluationResult::Quantity(decimal, "units".to_string());
        let result = to_integer_function(&quantity_val).unwrap();
        assert_eq!(result, EvaluationResult::Integer(42));
    }

    #[test]
    fn test_to_integer_quantity_decimal() {
        // Test toInteger on Quantity with decimal value - should return Empty
        let decimal = Decimal::from_str("3.14159").unwrap();
        let quantity_val = EvaluationResult::Quantity(decimal, "units".to_string());
        let result = to_integer_function(&quantity_val).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_to_integer_collection() {
        // Test toInteger on multi-item collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::Integer(1),
                EvaluationResult::Integer(2),
            ],
            has_undefined_order: false,
        };
        let result = to_integer_function(&collection);
        assert!(result.is_err());
    }
}