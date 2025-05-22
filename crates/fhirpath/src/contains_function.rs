use crate::evaluator::EvaluationContext;
use fhirpath_support::{EvaluationError, EvaluationResult};

/// Implements the FHIRPath contains() function
///
/// The contains() function returns true if the specified item is in the source collection,
/// and false otherwise. For strings, it returns true if the source string contains the
/// argument as a substring. The function uses equality (=) for comparing collection items.
///
/// # Arguments
///
/// * `invocation_base` - The source collection or string to search in
/// * `arg` - The item or substring to search for
/// * `context` - The evaluation context, used for comparing values
///
/// # Returns
///
/// * `Ok(Boolean(true))` - If the source contains the argument
/// * `Ok(Boolean(false))` - If the source does not contain the argument
/// * `Ok(Empty)` - If the argument is Empty
/// * `Err` - If there's a type mismatch or multi-item collection is used as argument
pub fn contains_function(
    invocation_base: &EvaluationResult,
    arg: &EvaluationResult,
    _context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    // Check if we're dealing with a collection of strings, which is not allowed by the spec
    if let EvaluationResult::Collection { items, .. } = invocation_base {
        // Check if any of the items in the collection are strings
        if items.iter().any(|item| matches!(item, EvaluationResult::String(_))) {
            return Err(EvaluationError::SingletonEvaluationError(
                "contains function requires singleton string input, not a collection of strings".to_string(),
            ));
        }
    }

    // Spec: X contains {} -> {}
    if arg == &EvaluationResult::Empty {
        return Ok(EvaluationResult::Empty);
    }
    
    // Spec: {} contains X -> false (where X is not empty)
    if invocation_base == &EvaluationResult::Empty {
        return Ok(EvaluationResult::Boolean(false));
    }
    
    // Check for multi-item argument (error)
    if arg.count() > 1 {
        return Err(EvaluationError::SingletonEvaluationError(
            "contains argument must be a singleton".to_string(),
        ));
    }

    // Handle the string case specially
    if let EvaluationResult::String(s) = invocation_base {
        // String contains substring: Check the type of arg here
        if let EvaluationResult::String(substr) = arg {
            return Ok(EvaluationResult::Boolean(s.contains(substr)));
        } else {
            // Argument is not String (and not Empty, checked earlier) -> Error
            return Err(EvaluationError::TypeError(format!(
                "contains function on String requires String argument, found {}",
                arg.type_name()
            )));
        }
    }
    
    // For collections, we need to manually check each item for equality
    if let EvaluationResult::Collection { items, .. } = invocation_base {
        let contains = items.iter().any(|item| simple_equality_check(item, arg));
        return Ok(EvaluationResult::Boolean(contains));
    }
    
    // For a single item (not a collection or string), use simple equality
    let contains = simple_equality_check(invocation_base, arg);
    Ok(EvaluationResult::Boolean(contains))
}

/// A simplified equality check for the contains function
/// 
/// This is needed because we can't access the private compare_equality function directly.
/// In a production implementation, we would want to ensure this follows the FHIRPath
/// equality rules exactly.
fn simple_equality_check(a: &EvaluationResult, b: &EvaluationResult) -> bool {
    match (a, b) {
        // Direct equality for simple types
        (EvaluationResult::Boolean(a_val), EvaluationResult::Boolean(b_val)) => a_val == b_val,
        (EvaluationResult::Integer(a_val), EvaluationResult::Integer(b_val)) => a_val == b_val,
        (EvaluationResult::Decimal(a_val), EvaluationResult::Decimal(b_val)) => a_val == b_val,
        (EvaluationResult::String(a_val), EvaluationResult::String(b_val)) => a_val == b_val,
        
        // Quantity comparison with same units
        (EvaluationResult::Quantity(a_val, a_unit), EvaluationResult::Quantity(b_val, b_unit)) => {
            a_val == b_val && a_unit == b_unit
        }
        
        // Object comparison by checking all keys/values are equal
        (EvaluationResult::Object(a_map), EvaluationResult::Object(b_map)) => {
            if a_map.len() != b_map.len() {
                return false;
            }
            for (key, a_value) in a_map {
                if let Some(b_value) = b_map.get(key) {
                    if !simple_equality_check(a_value, b_value) {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            true
        }
        
        // Collection comparison (not part of contains functionality, but added for completeness)
        (EvaluationResult::Collection { items: a_items, .. }, EvaluationResult::Collection { items: b_items, .. }) => {
            if a_items.len() != b_items.len() {
                return false;
            }
            a_items.iter().zip(b_items.iter()).all(|(a, b)| simple_equality_check(a, b))
        }
        
        // Special cases
        (EvaluationResult::Empty, EvaluationResult::Empty) => true,
        
        // Default: not equal
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a test collection
    fn create_test_collection(items: Vec<EvaluationResult>) -> EvaluationResult {
        EvaluationResult::Collection {
            items,
            has_undefined_order: false,
        }
    }

    // Helper function to create a simple EvaluationContext for tests
    fn create_test_context() -> EvaluationContext {
        EvaluationContext::new_empty()
    }

    #[test]
    fn test_contains_string_substring() {
        // Test contains on a string with a substring that exists
        let base = EvaluationResult::String("Hello, world!".to_string());
        let arg = EvaluationResult::String("world".to_string());
        let context = create_test_context();
        let result = contains_function(&base, &arg, &context).unwrap();
        assert_eq!(result, EvaluationResult::Boolean(true));
    }

    #[test]
    fn test_contains_string_missing_substring() {
        // Test contains on a string with a substring that doesn't exist
        let base = EvaluationResult::String("Hello, world!".to_string());
        let arg = EvaluationResult::String("universe".to_string());
        let context = create_test_context();
        let result = contains_function(&base, &arg, &context).unwrap();
        assert_eq!(result, EvaluationResult::Boolean(false));
    }

    #[test]
    fn test_contains_string_type_error() {
        // Test contains on a string with a non-string argument
        let base = EvaluationResult::String("Hello, world!".to_string());
        let arg = EvaluationResult::Integer(42);
        let context = create_test_context();
        let result = contains_function(&base, &arg, &context);
        assert!(result.is_err());
    }

    #[test]
    fn test_contains_collection_with_matching_item() {
        // Test contains on a collection with an item that matches
        let base = create_test_collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2),
            EvaluationResult::Integer(3),
        ]);
        let arg = EvaluationResult::Integer(2);
        let context = create_test_context();
        
        let result = contains_function(&base, &arg, &context).unwrap();
        assert_eq!(result, EvaluationResult::Boolean(true));
    }

    #[test]
    fn test_contains_collection_without_matching_item() {
        // Test contains on a collection without a matching item
        let base = create_test_collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2),
            EvaluationResult::Integer(3),
        ]);
        let arg = EvaluationResult::Integer(4);
        let context = create_test_context();
        
        let result = contains_function(&base, &arg, &context).unwrap();
        assert_eq!(result, EvaluationResult::Boolean(false));
    }

    #[test]
    fn test_contains_empty_source() {
        // Test contains on an empty collection
        let base = EvaluationResult::Empty;
        let arg = EvaluationResult::Integer(1);
        let context = create_test_context();
        let result = contains_function(&base, &arg, &context).unwrap();
        assert_eq!(result, EvaluationResult::Boolean(false));
    }

    #[test]
    fn test_contains_empty_argument() {
        // Test contains with an empty argument
        let base = create_test_collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2),
        ]);
        let arg = EvaluationResult::Empty;
        let context = create_test_context();
        let result = contains_function(&base, &arg, &context).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_contains_single_item() {
        // Test contains on a single item (not a collection)
        let base = EvaluationResult::Integer(42);
        let arg = EvaluationResult::Integer(42);
        let context = create_test_context();
        
        let result = contains_function(&base, &arg, &context).unwrap();
        assert_eq!(result, EvaluationResult::Boolean(true));
    }

    #[test]
    fn test_contains_multi_item_argument_error() {
        // Test contains with a multi-item collection as argument (should error)
        let base = EvaluationResult::String("Hello".to_string());
        let arg = create_test_collection(vec![
            EvaluationResult::String("Hello".to_string()),
            EvaluationResult::String("World".to_string()),
        ]);
        let context = create_test_context();
        let result = contains_function(&base, &arg, &context);
        assert!(result.is_err());
    }

    #[test]
    fn test_simple_equality_integers() {
        assert!(simple_equality_check(
            &EvaluationResult::Integer(42),
            &EvaluationResult::Integer(42)
        ));
        assert!(!simple_equality_check(
            &EvaluationResult::Integer(42),
            &EvaluationResult::Integer(43)
        ));
    }

    #[test]
    fn test_simple_equality_strings() {
        assert!(simple_equality_check(
            &EvaluationResult::String("test".to_string()),
            &EvaluationResult::String("test".to_string())
        ));
        assert!(!simple_equality_check(
            &EvaluationResult::String("test".to_string()),
            &EvaluationResult::String("different".to_string())
        ));
    }

    #[test]
    fn test_simple_equality_different_types() {
        assert!(!simple_equality_check(
            &EvaluationResult::Integer(42),
            &EvaluationResult::String("42".to_string())
        ));
    }
}