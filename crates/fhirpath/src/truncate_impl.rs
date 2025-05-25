use fhirpath_support::{EvaluationError, EvaluationResult};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

/// Truncate a numeric value to an integer by removing the fractional part
pub fn truncate_numeric(input: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match input {
        EvaluationResult::Integer(i, _) => {
            // Integer values remain unchanged
            Ok(EvaluationResult::integer(*i))
        }
        EvaluationResult::Decimal(d, _) => {
            // For decimals, remove the fractional part
            let truncated = d.trunc();

            // If the truncated value fits within i64 range, return as Integer
            if truncated.abs() <= Decimal::from(i64::MAX) {
                Ok(EvaluationResult::integer(truncated.to_i64().unwrap()))
            } else {
                // Otherwise return as Decimal with no fractional part
                Ok(EvaluationResult::decimal(truncated))
            }
        }
        EvaluationResult::Quantity(value, unit, _) => {
            // For Quantity, truncate the value but preserve the unit
            let truncated = value.trunc();
            Ok(EvaluationResult::quantity(truncated, unit.clone()))
        }
        _ => Err(EvaluationError::TypeError(
            "truncate can only be invoked on numeric types".to_string(),
        )),
    }
}

/// Truncate function implementation for FHIRPath expressions
/// Handles collection cases and delegates to truncate_numeric for actual truncation
pub fn truncate(
    invocation_base: &EvaluationResult,
    args: &[EvaluationResult],
) -> Result<EvaluationResult, EvaluationError> {
    // Check that no arguments were provided
    if !args.is_empty() {
        return Err(EvaluationError::InvalidArity(
            "Function 'truncate' does not accept any arguments".to_string(),
        ));
    }

    // Handle different collection cases
    match invocation_base {
        EvaluationResult::Empty => {
            return Ok(EvaluationResult::Empty);
        }
        EvaluationResult::Collection { items, .. } => {
            // Destructure
            if items.is_empty() {
                return Ok(EvaluationResult::Empty);
            } else if items.len() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "truncate requires a singleton input".to_string(),
                ));
            } else {
                return truncate_numeric(&items[0]);
            }
        }
        _ => truncate_numeric(invocation_base),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_truncate_integers() {
        assert_eq!(
            truncate_numeric(&EvaluationResult::integer(101)).unwrap(),
            EvaluationResult::integer(101)
        );
        assert_eq!(
            truncate_numeric(&EvaluationResult::integer(-42)).unwrap(),
            EvaluationResult::integer(-42)
        );
    }

    #[test]
    fn test_truncate_decimals() {
        assert_eq!(
            truncate_numeric(&EvaluationResult::decimal(dec!(101.5))).unwrap(),
            EvaluationResult::integer(101)
        );
        assert_eq!(
            truncate_numeric(&EvaluationResult::decimal(dec!(-1.56))).unwrap(),
            EvaluationResult::integer(-1)
        );
    }

    #[test]
    fn test_truncate_quantities() {
        assert_eq!(
            truncate_numeric(&EvaluationResult::quantity(dec!(5.7), "mm".to_string())).unwrap(),
            EvaluationResult::quantity(dec!(5), "mm".to_string())
        );
    }

    #[test]
    fn test_truncate_error_cases() {
        assert!(truncate_numeric(&EvaluationResult::string("not a number".to_string())).is_err());
        assert!(truncate_numeric(&EvaluationResult::boolean(true)).is_err());
    }
}
