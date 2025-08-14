use crate::distinct_functions::normalize_collection_result;
use crate::evaluator::EvaluationContext;
use helios_fhirpath_support::EvaluationError;
use helios_fhirpath_support::EvaluationResult;
use rust_decimal::prelude::ToPrimitive;

/// Implements the FHIRPath `skip` function
///
/// Syntax: collection.skip(num : Integer) : collection
///
/// Returns a collection containing all but the first num items from the input collection.
/// If num is greater than the number of items in the collection, an empty collection is returned.
/// If num is less than or equal to 0, the input collection is returned.
///
/// # Arguments
///
/// * `invocation_base` - The input collection
/// * `num` - The number of items to skip
/// * `context` - The evaluation context
///
/// # Returns
///
/// * A collection with the first `num` items removed
///
/// # Examples
///
/// ```text
/// [1, 2, 3].skip(1) = [2, 3]
/// [1, 2, 3].skip(2) = [3]
/// [1, 2, 3].skip(3) = []
/// [1, 2, 3].skip(4) = []
/// [1, 2, 3].skip(0) = [1, 2, 3]
/// [].skip(1) = []
/// ```
pub fn skip_function(
    invocation_base: &EvaluationResult,
    num_arg: &EvaluationResult,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    // Determine the number of items to skip
    let num_to_skip = match num_arg {
        EvaluationResult::Integer(i, _) => {
            if *i < 0 { 0 } else { *i as usize } // Treat negative skip as 0
        }
        // Add conversion from Decimal if it's an integer value
        EvaluationResult::Decimal(d, _) if d.is_integer() && d.is_sign_positive() => {
            d.to_usize().unwrap_or(0) // Convert non-negative integer Decimal
        }
        _ => {
            return Err(EvaluationError::InvalidArgument(
                "skip argument must be a non-negative integer".to_string(),
            ));
        }
    };

    // Get the items and order status from the invocation base
    let (items, input_was_unordered) = match invocation_base {
        EvaluationResult::Collection {
            items,
            has_undefined_order,
            ..
        } => {
            if *has_undefined_order && context.check_ordered_functions {
                return Err(EvaluationError::SemanticError(
                    "skip() operation on collection with undefined order is not allowed when checkOrderedFunctions is true.".to_string()
                ));
            }
            (items.clone(), *has_undefined_order)
        }
        EvaluationResult::Empty => (vec![], false), // Default order status for empty
        single_item => (vec![single_item.clone()], false), // Single item is ordered
    };

    // Return the skipped collection, or Empty if we skip all items
    Ok(if num_to_skip >= items.len() {
        EvaluationResult::Empty
    } else {
        let skipped_items = items[num_to_skip..].to_vec();
        normalize_collection_result(skipped_items, input_was_unordered)
    })
}

/// Implements the FHIRPath `tail` function
///
/// Syntax: collection.tail() : collection
///
/// Returns a collection containing all but the first item from the input collection.
/// If the input collection has only one item, an empty collection is returned.
/// The tail function is equivalent to skip(1).
///
/// # Arguments
///
/// * `invocation_base` - The input collection
/// * `context` - The evaluation context
///
/// # Returns
///
/// * A collection with the first item removed
///
/// # Examples
///
/// ```text
/// [1, 2, 3].tail() = [2, 3]
/// [1].tail() = []
/// [].tail() = []
/// ```
pub fn tail_function(
    invocation_base: &EvaluationResult,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    // Check if the collection has undefined order with ordered functions check enabled
    if let EvaluationResult::Collection {
        has_undefined_order,
        ..
    } = invocation_base
    {
        if *has_undefined_order && context.check_ordered_functions {
            return Err(EvaluationError::SemanticError(
                "tail() operation on collection with undefined order is not allowed when checkOrderedFunctions is true.".to_string()
            ));
        }
    }

    // Get the order status
    let input_was_unordered = if let EvaluationResult::Collection {
        has_undefined_order,
        ..
    } = invocation_base
    {
        *has_undefined_order
    } else {
        false
    };

    // Process the collection
    Ok(
        if let EvaluationResult::Collection { items, .. } = invocation_base {
            if items.len() > 1 {
                EvaluationResult::Collection {
                    items: items[1..].to_vec(), // Skip the first item
                    has_undefined_order: input_was_unordered,
                    type_info: None,
                }
            } else {
                EvaluationResult::Empty // Empty if 0 or 1 item
            }
        } else {
            EvaluationResult::Empty // Empty input or single item
        },
    )
}

/// Implements the FHIRPath `take` function
///
/// Syntax: collection.take(num : Integer) : collection
///
/// Returns a collection containing the first num items from the input collection.
/// If num is greater than the number of items in the collection, the entire collection is returned.
/// If num is less than or equal to 0, an empty collection is returned.
///
/// # Arguments
///
/// * `invocation_base` - The input collection
/// * `num` - The number of items to take
/// * `context` - The evaluation context
///
/// # Returns
///
/// * A collection with at most the first `num` items
///
/// # Examples
///
/// ```text
/// [1, 2, 3].take(1) = [1]
/// [1, 2, 3].take(2) = [1, 2]
/// [1, 2, 3].take(3) = [1, 2, 3]
/// [1, 2, 3].take(4) = [1, 2, 3]
/// [1, 2, 3].take(0) = []
/// [].take(1) = []
/// ```
pub fn take_function(
    invocation_base: &EvaluationResult,
    num_arg: &EvaluationResult,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    // Determine the number of items to take
    let num_to_take = match num_arg {
        EvaluationResult::Integer(i, _) => {
            if *i <= 0 { 0 } else { *i as usize } // Treat non-positive take as 0
        }
        // Add conversion from Decimal if it's an integer value
        EvaluationResult::Decimal(d, _) if d.is_integer() && d.is_sign_positive() => {
            d.to_usize().unwrap_or(0) // Convert non-negative integer Decimal
        }
        _ => {
            return Err(EvaluationError::InvalidArgument(
                "take argument must be a non-negative integer".to_string(),
            ));
        }
    };

    // Early return if taking 0 items
    if num_to_take == 0 {
        return Ok(EvaluationResult::Empty);
    }

    // Get the items and order status from the invocation base
    let (items, input_was_unordered) = match invocation_base {
        EvaluationResult::Collection {
            items,
            has_undefined_order,
            ..
        } => {
            if *has_undefined_order && context.check_ordered_functions {
                return Err(EvaluationError::SemanticError(
                    "take() operation on collection with undefined order is not allowed when checkOrderedFunctions is true.".to_string()
                ));
            }
            (items.clone(), *has_undefined_order)
        }
        EvaluationResult::Empty => (vec![], false), // Default order status for empty
        single_item => (vec![single_item.clone()], false), // Single item is ordered
    };

    // Take the requested number of items
    let taken_items: Vec<EvaluationResult> = items.into_iter().take(num_to_take).collect();
    Ok(normalize_collection_result(
        taken_items,
        input_was_unordered,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    #[test]
    fn test_skip_basic() {
        // Create a test collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test skip(1)
        let num = EvaluationResult::integer(1);
        let result = skip_function(&collection, &num, &context).unwrap();

        // Should return [2, 3]
        let expected = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(2), EvaluationResult::integer(3)],
            has_undefined_order: false,
            type_info: None,
        };
        assert_eq!(result, expected);
    }

    #[test]
    fn test_skip_zero() {
        // Create a test collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test skip(0)
        let num = EvaluationResult::integer(0);
        let result = skip_function(&collection, &num, &context).unwrap();

        // Should return the original collection
        assert_eq!(result, collection);
    }

    #[test]
    fn test_skip_negative() {
        // Create a test collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test skip(-1) - negative skips are treated as 0
        let num = EvaluationResult::integer(-1);
        let result = skip_function(&collection, &num, &context).unwrap();

        // Should return the original collection
        assert_eq!(result, collection);
    }

    #[test]
    fn test_skip_all() {
        // Create a test collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test skip(3) - skip all elements
        let num = EvaluationResult::integer(3);
        let result = skip_function(&collection, &num, &context).unwrap();

        // Should return empty
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_skip_beyond() {
        // Create a test collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test skip(4) - skip more than available
        let num = EvaluationResult::integer(4);
        let result = skip_function(&collection, &num, &context).unwrap();

        // Should return empty
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_skip_single_item() {
        // Create a single item (not in collection form)
        let single = EvaluationResult::integer(42);

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test skip(1) on single item
        let num = EvaluationResult::integer(1);
        let result = skip_function(&single, &num, &context).unwrap();

        // Should return empty
        assert_eq!(result, EvaluationResult::Empty);

        // Test skip(0) on single item
        let num = EvaluationResult::integer(0);
        let result = skip_function(&single, &num, &context).unwrap();

        // Should return the single item
        let expected = EvaluationResult::integer(42);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_skip_empty() {
        // Create an empty collection
        let empty = EvaluationResult::Empty;

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test skip(1) on empty collection
        let num = EvaluationResult::integer(1);
        let result = skip_function(&empty, &num, &context).unwrap();

        // Should return empty
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_skip_decimal() {
        // Create a test collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test skip with a decimal value
        let num = EvaluationResult::decimal(Decimal::from(2));
        let result = skip_function(&collection, &num, &context).unwrap();

        // When there's a single result, normalize_collection_result should return it directly
        // So the expected result is just the Integer(3) not in a collection
        assert_eq!(result, EvaluationResult::integer(3));
    }

    #[test]
    fn test_skip_invalid_arg() {
        // Create a test collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test skip with an invalid arg type
        let num = EvaluationResult::string("not a number".to_string());
        let result = skip_function(&collection, &num, &context);

        // Should return an error
        assert!(result.is_err());
        if let Err(EvaluationError::InvalidArgument(msg)) = result {
            assert!(msg.contains("non-negative integer"));
        } else {
            panic!("Expected InvalidArgument error");
        }
    }

    #[test]
    fn test_tail_basic() {
        // Create a test collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test tail()
        let result = tail_function(&collection, &context).unwrap();

        // Should return [2, 3]
        let expected = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(2), EvaluationResult::integer(3)],
            has_undefined_order: false,
            type_info: None,
        };
        assert_eq!(result, expected);
    }

    #[test]
    fn test_tail_single_item() {
        // Create a collection with one item
        let collection = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1)],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test tail()
        let result = tail_function(&collection, &context).unwrap();

        // Should return Empty
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_tail_empty() {
        // Create an empty collection
        let empty = EvaluationResult::Empty;

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test tail() on empty
        let result = tail_function(&empty, &context).unwrap();

        // Should return Empty
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_tail_single_value() {
        // Create a single value (not in collection form)
        let single = EvaluationResult::integer(42);

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test tail() on single value
        let result = tail_function(&single, &context).unwrap();

        // Should return Empty
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_take_basic() {
        // Create a test collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test take(2)
        let num = EvaluationResult::integer(2);
        let result = take_function(&collection, &num, &context).unwrap();

        // Should return [1, 2]
        let expected = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1), EvaluationResult::integer(2)],
            has_undefined_order: false,
            type_info: None,
        };
        assert_eq!(result, expected);
    }

    #[test]
    fn test_take_zero() {
        // Create a test collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test take(0)
        let num = EvaluationResult::integer(0);
        let result = take_function(&collection, &num, &context).unwrap();

        // Should return Empty
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_take_negative() {
        // Create a test collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test take(-1) - negative takes are treated as 0
        let num = EvaluationResult::integer(-1);
        let result = take_function(&collection, &num, &context).unwrap();

        // Should return Empty
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_take_all() {
        // Create a test collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test take(3) - take all elements
        let num = EvaluationResult::integer(3);
        let result = take_function(&collection, &num, &context).unwrap();

        // Should return the original collection
        assert_eq!(result, collection);
    }

    #[test]
    fn test_take_beyond() {
        // Create a test collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test take(4) - take more than available
        let num = EvaluationResult::integer(4);
        let result = take_function(&collection, &num, &context).unwrap();

        // Should return the original collection
        assert_eq!(result, collection);
    }

    #[test]
    fn test_take_single_item() {
        // Create a single item (not in collection form)
        let single = EvaluationResult::integer(42);

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test take(1) on single item
        let num = EvaluationResult::integer(1);
        let result = take_function(&single, &num, &context).unwrap();

        // Should return the single item
        assert_eq!(result, single);

        // Test take(0) on single item
        let num = EvaluationResult::integer(0);
        let result = take_function(&single, &num, &context).unwrap();

        // Should return Empty
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_take_empty() {
        // Create an empty collection
        let empty = EvaluationResult::Empty;

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test take(1) on empty collection
        let num = EvaluationResult::integer(1);
        let result = take_function(&empty, &num, &context).unwrap();

        // Should return Empty
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_take_decimal() {
        // Create a test collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test take with a decimal value
        let num = EvaluationResult::decimal(Decimal::from(2));
        let result = take_function(&collection, &num, &context).unwrap();

        // Should return [1, 2]
        let expected = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1), EvaluationResult::integer(2)],
            has_undefined_order: false,
            type_info: None,
        };
        assert_eq!(result, expected);
    }

    #[test]
    fn test_take_invalid_arg() {
        // Create a test collection
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test take with an invalid arg type
        let num = EvaluationResult::string("not a number".to_string());
        let result = take_function(&collection, &num, &context);

        // Should return an error
        assert!(result.is_err());
        if let Err(EvaluationError::InvalidArgument(msg)) = result {
            assert!(msg.contains("non-negative integer"));
        } else {
            panic!("Expected InvalidArgument error");
        }
    }
}
