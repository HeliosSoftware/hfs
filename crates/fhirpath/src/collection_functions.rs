use crate::evaluator::EvaluationContext;
use fhirpath_support::{EvaluationError, EvaluationResult};

/// Implements the FHIRPath first() function
///
/// Returns the first item in the collection. Returns empty ({ }) if the input
/// collection is empty. When invoked on a collection with undefined order,
/// may produce inconsistent results if checkOrderedFunctions is true.
///
/// # Arguments
///
/// * `invocation_base` - The collection to get the first item from
/// * `context` - The evaluation context, used to check if ordered functions are allowed
///
/// # Returns
///
/// * The first item in the collection or Empty if the collection is empty
/// * Error if the collection has undefined order and checkOrderedFunctions is true
pub fn first_function(
    invocation_base: &EvaluationResult,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    // Check if the collection has undefined order
    if let EvaluationResult::Collection {
        has_undefined_order,
        ..
    } = invocation_base
    {
        if *has_undefined_order && context.check_ordered_functions {
            return Err(EvaluationError::SemanticError(
                "first() operation on collection with undefined order is not allowed when checkOrderedFunctions is true."
                    .to_string(),
            ));
        }
    }

    // Return the first item or Empty if collection is empty
    Ok(match invocation_base {
        EvaluationResult::Collection { items, .. } => {
            items.first().cloned().unwrap_or(EvaluationResult::Empty)
        }
        _ => invocation_base.clone(), // For non-collections, return the item itself
    })
}

/// Implements the FHIRPath last() function
///
/// Returns the last item in the collection. Returns empty ({ }) if the input
/// collection is empty. When invoked on a collection with undefined order,
/// may produce inconsistent results if checkOrderedFunctions is true.
///
/// # Arguments
///
/// * `invocation_base` - The collection to get the last item from
/// * `context` - The evaluation context, used to check if ordered functions are allowed
///
/// # Returns
///
/// * The last item in the collection or Empty if the collection is empty
/// * Error if the collection has undefined order and checkOrderedFunctions is true
pub fn last_function(
    invocation_base: &EvaluationResult,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    // Check if the collection has undefined order
    if let EvaluationResult::Collection {
        has_undefined_order,
        ..
    } = invocation_base
    {
        if *has_undefined_order && context.check_ordered_functions {
            return Err(EvaluationError::SemanticError(
                "last() operation on collection with undefined order is not allowed when checkOrderedFunctions is true."
                    .to_string(),
            ));
        }
    }

    // Return the last item or Empty if collection is empty
    Ok(match invocation_base {
        EvaluationResult::Collection { items, .. } => {
            items.last().cloned().unwrap_or(EvaluationResult::Empty)
        }
        _ => invocation_base.clone(), // For non-collections, return the item itself
    })
}

/// Implements the FHIRPath count() function
///
/// Returns the number of items in the collection. Returns 0 for empty collections
/// and 1 for a single item that's not a collection.
///
/// # Arguments
///
/// * `invocation_base` - The collection to count items in
///
/// # Returns
///
/// * Integer representing the number of items in the collection
pub fn count_function(invocation_base: &EvaluationResult) -> EvaluationResult {
    match invocation_base {
        EvaluationResult::Collection { items, .. } => EvaluationResult::Integer(items.len() as i64),
        EvaluationResult::Empty => EvaluationResult::Integer(0),
        _ => EvaluationResult::Integer(1), // Single item counts as 1
    }
}

/// Implements the FHIRPath empty() function
///
/// Returns true if the input collection is empty (contains no items),
/// and false otherwise.
///
/// # Arguments
///
/// * `invocation_base` - The collection to check for emptiness
///
/// # Returns
///
/// * Boolean result: true if the collection is empty, false otherwise
pub fn empty_function(invocation_base: &EvaluationResult) -> EvaluationResult {
    match invocation_base {
        EvaluationResult::Empty => EvaluationResult::Boolean(true),
        EvaluationResult::Collection { items, .. } => EvaluationResult::Boolean(items.is_empty()),
        _ => EvaluationResult::Boolean(false), // Single non-empty item is not empty
    }
}

/// Implements the FHIRPath exists() function without criteria
///
/// Returns true if the collection has any elements, and false otherwise.
/// This is the negation of empty().
///
/// # Arguments
///
/// * `invocation_base` - The collection to check for existence
///
/// # Returns
///
/// * Boolean result: true if the collection has elements, false otherwise
pub fn exists_function(invocation_base: &EvaluationResult) -> EvaluationResult {
    match invocation_base {
        EvaluationResult::Empty => EvaluationResult::Boolean(false),
        EvaluationResult::Collection { items, .. } => EvaluationResult::Boolean(!items.is_empty()),
        _ => EvaluationResult::Boolean(true), // Single non-empty item exists
    }
}

/// Implements the FHIRPath all() function without criteria
///
/// Returns true if all items in the collection are true.
/// Returns true for an empty collection.
///
/// # Arguments
///
/// * `invocation_base` - The collection to check all items
///
/// # Returns
///
/// * Boolean result: true if all items are true, false otherwise
pub fn all_function(invocation_base: &EvaluationResult) -> EvaluationResult {
    match invocation_base {
        EvaluationResult::Empty => EvaluationResult::Boolean(true), // all() is true for empty
        EvaluationResult::Collection { items, .. } => {
            // Check if all items evaluate to true
            EvaluationResult::Boolean(items.iter().all(|item| item.to_boolean()))
        }
        single_item => EvaluationResult::Boolean(single_item.to_boolean()), // Check single item
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a test collection
    fn create_test_collection(
        items: Vec<EvaluationResult>,
        has_undefined_order: bool,
    ) -> EvaluationResult {
        EvaluationResult::Collection {
            items,
            has_undefined_order,
        }
    }

    #[test]
    fn test_first_empty_collection() {
        // Test first() on an empty collection
        let empty = EvaluationResult::Empty;
        let context = EvaluationContext::new_empty();
        let result = first_function(&empty, &context).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_first_non_empty_collection() {
        // Test first() on a non-empty collection
        let collection = create_test_collection(
            vec![
                EvaluationResult::Integer(1),
                EvaluationResult::Integer(2),
                EvaluationResult::Integer(3),
            ],
            false,
        );
        let context = EvaluationContext::new_empty();
        let result = first_function(&collection, &context).unwrap();
        assert_eq!(result, EvaluationResult::Integer(1));
    }

    #[test]
    fn test_first_single_item() {
        // Test first() on a single item
        let single = EvaluationResult::String("test".to_string());
        let context = EvaluationContext::new_empty();
        let result = first_function(&single, &context).unwrap();
        assert_eq!(result, EvaluationResult::String("test".to_string()));
    }

    #[test]
    fn test_first_undefined_order() {
        // Test first() on a collection with undefined order
        let collection = create_test_collection(
            vec![EvaluationResult::Integer(1), EvaluationResult::Integer(2)],
            true, // undefined order
        );
        let mut context = EvaluationContext::new_empty();

        // First test with check_ordered_functions = false (should succeed)
        context.check_ordered_functions = false;
        let result = first_function(&collection, &context);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), EvaluationResult::Integer(1));

        // Then test with check_ordered_functions = true (should fail)
        context.check_ordered_functions = true;
        let result = first_function(&collection, &context);
        assert!(result.is_err());
    }

    #[test]
    fn test_last_empty_collection() {
        // Test last() on an empty collection
        let empty = EvaluationResult::Empty;
        let context = EvaluationContext::new_empty();
        let result = last_function(&empty, &context).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_last_non_empty_collection() {
        // Test last() on a non-empty collection
        let collection = create_test_collection(
            vec![
                EvaluationResult::Integer(1),
                EvaluationResult::Integer(2),
                EvaluationResult::Integer(3),
            ],
            false,
        );
        let context = EvaluationContext::new_empty();
        let result = last_function(&collection, &context).unwrap();
        assert_eq!(result, EvaluationResult::Integer(3));
    }

    #[test]
    fn test_last_single_item() {
        // Test last() on a single item
        let single = EvaluationResult::String("test".to_string());
        let context = EvaluationContext::new_empty();
        let result = last_function(&single, &context).unwrap();
        assert_eq!(result, EvaluationResult::String("test".to_string()));
    }

    #[test]
    fn test_last_undefined_order() {
        // Test last() on a collection with undefined order
        let collection = create_test_collection(
            vec![EvaluationResult::Integer(1), EvaluationResult::Integer(2)],
            true, // undefined order
        );
        let mut context = EvaluationContext::new_empty();

        // First test with check_ordered_functions = false (should succeed)
        context.check_ordered_functions = false;
        let result = last_function(&collection, &context);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), EvaluationResult::Integer(2));

        // Then test with check_ordered_functions = true (should fail)
        context.check_ordered_functions = true;
        let result = last_function(&collection, &context);
        assert!(result.is_err());
    }

    #[test]
    fn test_count_empty_collection() {
        // Test count() on an empty collection
        let empty = EvaluationResult::Empty;
        let result = count_function(&empty);
        assert_eq!(result, EvaluationResult::Integer(0));
    }

    #[test]
    fn test_count_non_empty_collection() {
        // Test count() on a non-empty collection
        let collection = create_test_collection(
            vec![
                EvaluationResult::Integer(1),
                EvaluationResult::Integer(2),
                EvaluationResult::Integer(3),
            ],
            false,
        );
        let result = count_function(&collection);
        assert_eq!(result, EvaluationResult::Integer(3));
    }

    #[test]
    fn test_count_single_item() {
        // Test count() on a single item
        let single = EvaluationResult::String("test".to_string());
        let result = count_function(&single);
        assert_eq!(result, EvaluationResult::Integer(1));
    }

    #[test]
    fn test_empty_on_empty_collection() {
        // Test empty() on an empty collection
        let empty = EvaluationResult::Empty;
        let result = empty_function(&empty);
        assert_eq!(result, EvaluationResult::Boolean(true));
    }

    #[test]
    fn test_empty_on_non_empty_collection() {
        // Test empty() on a non-empty collection
        let collection = create_test_collection(vec![EvaluationResult::Integer(1)], false);
        let result = empty_function(&collection);
        assert_eq!(result, EvaluationResult::Boolean(false));
    }

    #[test]
    fn test_empty_on_single_item() {
        // Test empty() on a single item
        let single = EvaluationResult::String("test".to_string());
        let result = empty_function(&single);
        assert_eq!(result, EvaluationResult::Boolean(false));
    }

    #[test]
    fn test_exists_on_empty_collection() {
        // Test exists() on an empty collection
        let empty = EvaluationResult::Empty;
        let result = exists_function(&empty);
        assert_eq!(result, EvaluationResult::Boolean(false));
    }

    #[test]
    fn test_exists_on_non_empty_collection() {
        // Test exists() on a non-empty collection
        let collection = create_test_collection(vec![EvaluationResult::Integer(1)], false);
        let result = exists_function(&collection);
        assert_eq!(result, EvaluationResult::Boolean(true));
    }

    #[test]
    fn test_exists_on_single_item() {
        // Test exists() on a single item
        let single = EvaluationResult::String("test".to_string());
        let result = exists_function(&single);
        assert_eq!(result, EvaluationResult::Boolean(true));
    }

    #[test]
    fn test_all_on_empty_collection() {
        // Test all() on an empty collection
        let empty = EvaluationResult::Empty;
        let result = all_function(&empty);
        assert_eq!(result, EvaluationResult::Boolean(true));
    }

    #[test]
    fn test_all_on_all_true_collection() {
        // Test all() on a collection with all true values
        let collection = create_test_collection(
            vec![
                EvaluationResult::Boolean(true),
                EvaluationResult::Boolean(true),
            ],
            false,
        );
        let result = all_function(&collection);
        assert_eq!(result, EvaluationResult::Boolean(true));
    }

    #[test]
    fn test_all_on_mixed_collection() {
        // Test all() on a collection with mixed boolean values
        let collection = create_test_collection(
            vec![
                EvaluationResult::Boolean(true),
                EvaluationResult::Boolean(false),
            ],
            false,
        );
        let result = all_function(&collection);
        assert_eq!(result, EvaluationResult::Boolean(false));
    }

    #[test]
    fn test_all_on_single_true() {
        // Test all() on a single true value
        let single = EvaluationResult::Boolean(true);
        let result = all_function(&single);
        assert_eq!(result, EvaluationResult::Boolean(true));
    }

    #[test]
    fn test_all_on_single_false() {
        // Test all() on a single false value
        let single = EvaluationResult::Boolean(false);
        let result = all_function(&single);
        assert_eq!(result, EvaluationResult::Boolean(false));
    }
}

