use crate::evaluator::EvaluationContext;
use fhirpath_support::{EvaluationError, EvaluationResult};
use rust_decimal::prelude::ToPrimitive;

/// Implementation of the FHIRPath `toLong()` function.
/// 
/// This function attempts to convert its input to a Long (64-bit integer) value.
/// According to the FHIRPath specification, various input types can be converted to Long:
/// - Integer values are returned as is
/// - Decimal values are truncated (fractional part discarded)
/// - Strings that represent valid Long values are parsed
/// - Boolean values: true becomes 1, false becomes 0
/// - For other types or unsuccessful conversions, the function returns Empty
///
/// # Parameters
/// * `input` - The value to convert to a Long
/// * `_context` - The evaluation context (not used in this implementation)
///
/// # Returns
/// An `EvaluationResult` containing the Long value if conversion was successful,
/// or `EvaluationResult::Empty` if conversion failed
pub fn to_long(
    input: &EvaluationResult,
    _context: &EvaluationContext
) -> Result<EvaluationResult, EvaluationError> {
    match input {
        // Collection handling: if it contains exactly one item, convert that item
        // Otherwise, return Empty
        EvaluationResult::Collection { items, .. } => {
            if items.len() == 1 {
                to_long(&items[0], _context)
            } else {
                Ok(EvaluationResult::Empty)
            }
        },
        
        // Integer is already a 64-bit integer in our implementation, so just return it
        EvaluationResult::Integer(i) => Ok(EvaluationResult::Integer(*i)),
        
        // Decimal is converted to Long by truncating the fractional part
        // Return Empty if conversion fails (e.g., overflow)
        EvaluationResult::Decimal(d) => {
            match d.to_i64() {
                Some(i) => Ok(EvaluationResult::Integer(i)),
                None => Ok(EvaluationResult::Empty)
            }
        },
        
        // Boolean: true -> 1, false -> 0
        EvaluationResult::Boolean(b) => {
            if *b {
                Ok(EvaluationResult::Integer(1))
            } else {
                Ok(EvaluationResult::Integer(0))
            }
        },
        
        // String: attempt to parse as a Long
        EvaluationResult::String(s) => {
            match s.parse::<i64>() {
                Ok(i) => Ok(EvaluationResult::Integer(i)),
                Err(_) => Ok(EvaluationResult::Empty)
            }
        },
        
        // All other types: return Empty
        _ => Ok(EvaluationResult::Empty)
    }
}

/// Implementation of the FHIRPath `convertsToLong()` function.
/// 
/// This function determines whether a value can be successfully converted to a Long value.
/// It follows the same conversion rules as `toLong()` but returns a Boolean result
/// indicating success or failure instead of the converted value.
///
/// # Parameters
/// * `input` - The value to check for conversion to Long
/// * `_context` - The evaluation context (not used in this implementation)
///
/// # Returns
/// `EvaluationResult::Boolean(true)` if conversion would succeed, 
/// `EvaluationResult::Boolean(false)` otherwise
pub fn converts_to_long(
    input: &EvaluationResult,
    _context: &EvaluationContext
) -> Result<EvaluationResult, EvaluationError> {
    match input {
        // Collection handling: if it contains exactly one item, check that item
        // Otherwise, return false
        EvaluationResult::Collection { items, .. } => {
            if items.len() == 1 {
                converts_to_long(&items[0], _context)
            } else {
                Ok(EvaluationResult::Boolean(false))
            }
        },
        
        // Integer is already a 64-bit integer in our implementation, so always convertible
        EvaluationResult::Integer(_) => Ok(EvaluationResult::Boolean(true)),
        
        // Decimal is convertible if it can fit in an i64
        EvaluationResult::Decimal(d) => {
            Ok(EvaluationResult::Boolean(d.to_i64().is_some()))
        },
        
        // Boolean is always convertible
        EvaluationResult::Boolean(_) => Ok(EvaluationResult::Boolean(true)),
        
        // String is convertible if it can be parsed as an i64
        EvaluationResult::String(s) => {
            Ok(EvaluationResult::Boolean(s.parse::<i64>().is_ok()))
        },
        
        // All other types are not convertible
        _ => Ok(EvaluationResult::Boolean(false))
    }
}

//
// Note: The handle_to_long and handle_converts_to_long functions have been removed
// as they were not needed. The direct function calls are now used from the evaluator.
//

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    
    #[test]
    fn test_to_long_integer() {
        let context = EvaluationContext::new_empty();
        
        // Test with Integer values
        let result = to_long(&EvaluationResult::Integer(42), &context).unwrap();
        assert_eq!(result, EvaluationResult::Integer(42));
        
        let result = to_long(&EvaluationResult::Integer(-42), &context).unwrap();
        assert_eq!(result, EvaluationResult::Integer(-42));
        
        let result = to_long(&EvaluationResult::Integer(0), &context).unwrap();
        assert_eq!(result, EvaluationResult::Integer(0));
    }
    
    #[test]
    fn test_to_long_decimal() {
        let context = EvaluationContext::new_empty();
        
        // Test with Decimal values
        let result = to_long(&EvaluationResult::Decimal(dec!(42.75)), &context).unwrap();
        assert_eq!(result, EvaluationResult::Integer(42));
        
        let result = to_long(&EvaluationResult::Decimal(dec!(-42.75)), &context).unwrap();
        assert_eq!(result, EvaluationResult::Integer(-42));
        
        let result = to_long(&EvaluationResult::Decimal(dec!(0.999)), &context).unwrap();
        assert_eq!(result, EvaluationResult::Integer(0));
    }
    
    #[test]
    fn test_to_long_boolean() {
        let context = EvaluationContext::new_empty();
        
        // Test with Boolean values
        let result = to_long(&EvaluationResult::Boolean(true), &context).unwrap();
        assert_eq!(result, EvaluationResult::Integer(1));
        
        let result = to_long(&EvaluationResult::Boolean(false), &context).unwrap();
        assert_eq!(result, EvaluationResult::Integer(0));
    }
    
    #[test]
    fn test_to_long_string() {
        let context = EvaluationContext::new_empty();
        
        // Test with String values
        let result = to_long(&EvaluationResult::String("42".to_string()), &context).unwrap();
        assert_eq!(result, EvaluationResult::Integer(42));
        
        let result = to_long(&EvaluationResult::String("-42".to_string()), &context).unwrap();
        assert_eq!(result, EvaluationResult::Integer(-42));
        
        // Test with invalid String value
        let result = to_long(&EvaluationResult::String("not a number".to_string()), &context).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
        
        let result = to_long(&EvaluationResult::String("42.5".to_string()), &context).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }
    
    #[test]
    fn test_to_long_collection() {
        let context = EvaluationContext::new_empty();
        
        // Test with single-item Collection
        let collection = EvaluationResult::Collection {
            items: vec![EvaluationResult::Integer(42)],
            has_undefined_order: false,
        };
        let result = to_long(&collection, &context).unwrap();
        assert_eq!(result, EvaluationResult::Integer(42));
        
        // Test with multi-item Collection
        let collection = EvaluationResult::Collection {
            items: vec![EvaluationResult::Integer(42), EvaluationResult::Integer(43)],
            has_undefined_order: false,
        };
        let result = to_long(&collection, &context).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
        
        // Test with empty Collection
        let collection = EvaluationResult::Collection {
            items: vec![],
            has_undefined_order: false,
        };
        let result = to_long(&collection, &context).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }
    
    #[test]
    fn test_to_long_other_types() {
        let context = EvaluationContext::new_empty();
        
        // Test with other types that should return Empty
        let result = to_long(&EvaluationResult::Date("2022-01-01".to_string()), &context).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
        
        let result = to_long(&EvaluationResult::DateTime("2022-01-01T12:00:00".to_string()), &context).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
        
        let result = to_long(&EvaluationResult::Time("12:00:00".to_string()), &context).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
        
        let result = to_long(&EvaluationResult::Empty, &context).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
        
        let map = std::collections::HashMap::new();
        let result = to_long(&EvaluationResult::Object(map), &context).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }
    
    #[test]
    fn test_converts_to_long() {
        let context = EvaluationContext::new_empty();
        
        // Types that should convert
        assert_eq!(
            converts_to_long(&EvaluationResult::Integer(42), &context).unwrap(),
            EvaluationResult::Boolean(true)
        );
        
        assert_eq!(
            converts_to_long(&EvaluationResult::Decimal(dec!(42.75)), &context).unwrap(),
            EvaluationResult::Boolean(true)
        );
        
        assert_eq!(
            converts_to_long(&EvaluationResult::Boolean(true), &context).unwrap(),
            EvaluationResult::Boolean(true)
        );
        
        assert_eq!(
            converts_to_long(&EvaluationResult::String("42".to_string()), &context).unwrap(),
            EvaluationResult::Boolean(true)
        );
        
        // Types that should not convert
        assert_eq!(
            converts_to_long(&EvaluationResult::String("not a number".to_string()), &context).unwrap(),
            EvaluationResult::Boolean(false)
        );
        
        assert_eq!(
            converts_to_long(&EvaluationResult::Date("2022-01-01".to_string()), &context).unwrap(),
            EvaluationResult::Boolean(false)
        );
        
        assert_eq!(
            converts_to_long(&EvaluationResult::Empty, &context).unwrap(),
            EvaluationResult::Boolean(false)
        );
        
        // Collections
        let collection = EvaluationResult::Collection {
            items: vec![EvaluationResult::Integer(42)],
            has_undefined_order: false,
        };
        assert_eq!(
            converts_to_long(&collection, &context).unwrap(),
            EvaluationResult::Boolean(true)
        );
        
        let collection = EvaluationResult::Collection {
            items: vec![EvaluationResult::Integer(42), EvaluationResult::Integer(43)],
            has_undefined_order: false,
        };
        assert_eq!(
            converts_to_long(&collection, &context).unwrap(),
            EvaluationResult::Boolean(false)
        );
    }
}