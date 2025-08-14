use crate::evaluator::EvaluationContext;
use helios_fhirpath_support::{EvaluationError, EvaluationResult};
use std::collections::HashSet;

/// Implements the FHIRPath isDistinct() function
///
/// The isDistinct() function returns true if all items in the input collection
/// are distinct (not equal to each other). Returns true for empty collections
/// and single-item collections.
///
/// # Arguments
///
/// * `invocation_base` - The collection to check for distinct items
/// * `context` - The evaluation context, used for comparing values
///
/// # Returns
///
/// * `Ok(Boolean(true))` - If all items are distinct, or if the collection has 0-1 items
/// * `Ok(Boolean(false))` - If any items are equal
/// * `Err` - If an error occurs during equality comparison
pub fn is_distinct_function(
    invocation_base: &EvaluationResult,
    _context: &EvaluationContext, // Not used directly but needed for API consistency
) -> Result<EvaluationResult, EvaluationError> {
    // Extract items from the invocation base
    let items = match invocation_base {
        EvaluationResult::Collection { items, .. } => items.clone(),
        EvaluationResult::Empty => vec![],
        single_item => vec![single_item.clone()], // Treat single item as collection
    };

    // Empty or single-item collections are always distinct
    if items.len() <= 1 {
        return Ok(EvaluationResult::boolean(true));
    }

    // Check all pairs of items for equality
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            // Compare items[i] and items[j] for equality
            // We need to use simple_equality_check here since compare_equality is private
            if simple_equality_check(&items[i], &items[j]) {
                return Ok(EvaluationResult::boolean(false)); // Found a duplicate
            }
        }
    }

    // No duplicates found
    Ok(EvaluationResult::boolean(true))
}

/// Implements the FHIRPath distinct() function
///
/// The distinct() function returns a collection containing only the unique items
/// from the input collection. This is determined based on equality comparison.
///
/// # Arguments
///
/// * `invocation_base` - The collection to get distinct items from
///
/// # Returns
///
/// * A collection with duplicates removed
/// * Empty if input is empty
/// * The input item if input is a single item
pub fn distinct_function(
    invocation_base: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // Handle special cases
    match invocation_base {
        EvaluationResult::Empty => Ok(EvaluationResult::Empty),
        EvaluationResult::Collection { items, .. } => {
            // If collection is empty, return empty
            if items.is_empty() {
                return Ok(EvaluationResult::Empty);
            }
            // If collection has only one item, return it
            if items.len() == 1 {
                return Ok(items[0].clone());
            }

            // Extract items for processing
            let items = items.clone();

            // Use a set to track unique items
            let mut distinct_set = HashSet::new();
            let mut distinct_items = Vec::new(); // Maintain original order of first appearance

            // Process each item in the collection
            for item in items {
                // If the item is new (not in the distinct_set), add it to our results
                if distinct_set.insert(item.clone()) {
                    distinct_items.push(item);
                }
            }

            // Return the distinct items as a collection
            // distinct() output order is not guaranteed by spec, so mark as undefined
            Ok(normalize_collection_result(distinct_items, true))
        }
        // For single (non-collection) items, return as is
        single_item => Ok(single_item.clone()),
    }
}

/// Normalize a collection result based on FHIRPath rules
///
/// This helper function handles collection normalization rules:
/// - If the collection is empty, return Empty
/// - If the collection has a single item, return that item directly
/// - Otherwise, wrap items in a Collection
pub fn normalize_collection_result(
    mut items: Vec<EvaluationResult>,
    items_have_undefined_order: bool,
) -> EvaluationResult {
    if items.is_empty() {
        EvaluationResult::Empty
    } else if items.len() == 1 {
        // If the single item is itself a collection, preserve its undefined_order status.
        // Otherwise, a single non-collection item is considered ordered.
        let single_item = items.pop().unwrap();
        if let EvaluationResult::Collection {
            items: inner_items,
            has_undefined_order: inner_undef_order,
            type_info: None,
        } = single_item
        {
            // If the single item was a collection, re-wrap it, preserving its order status.
            // This typically happens if flatten_collections_recursive returns a single collection.
            EvaluationResult::Collection {
                items: inner_items,
                has_undefined_order: inner_undef_order,
                type_info: None,
            }
        } else {
            single_item // Not a collection, or already handled.
        }
    } else {
        EvaluationResult::Collection {
            items,
            has_undefined_order: items_have_undefined_order,
            type_info: None,
        }
    }
}

/// A simplified equality check for comparing items
///
/// This is a simple implementation for equality checks
/// as we can't access the private compare_equality function directly.
fn simple_equality_check(a: &EvaluationResult, b: &EvaluationResult) -> bool {
    match (a, b) {
        // Direct equality for simple types
        (EvaluationResult::Boolean(a_val, _), EvaluationResult::Boolean(b_val, _)) => {
            a_val == b_val
        }
        (EvaluationResult::Integer(a_val, _), EvaluationResult::Integer(b_val, _)) => {
            a_val == b_val
        }
        (EvaluationResult::Decimal(a_val, _), EvaluationResult::Decimal(b_val, _)) => {
            a_val == b_val
        }
        (EvaluationResult::String(a_val, _), EvaluationResult::String(b_val, _)) => a_val == b_val,

        // Quantity comparison with same units
        (
            EvaluationResult::Quantity(a_val, a_unit, _),
            EvaluationResult::Quantity(b_val, b_unit, _),
        ) => a_val == b_val && a_unit == b_unit,

        // Object comparison by checking all keys/values are equal
        (
            EvaluationResult::Object { map: a_map, .. },
            EvaluationResult::Object { map: b_map, .. },
        ) => {
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

        // Collection comparison
        (
            EvaluationResult::Collection { items: a_items, .. },
            EvaluationResult::Collection { items: b_items, .. },
        ) => {
            if a_items.len() != b_items.len() {
                return false;
            }
            a_items
                .iter()
                .zip(b_items.iter())
                .all(|(a, b)| simple_equality_check(a, b))
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
    fn create_test_collection(
        items: Vec<EvaluationResult>,
        has_undefined_order: bool,
    ) -> EvaluationResult {
        EvaluationResult::Collection {
            items,
            has_undefined_order,
            type_info: None,
        }
    }

    // Helper function to create a simple EvaluationContext for tests
    fn create_test_context() -> EvaluationContext {
        EvaluationContext::new_empty_with_default_version()
    }

    #[test]
    fn test_is_distinct_empty_collection() {
        // Test isDistinct() on an empty collection
        let empty = EvaluationResult::Empty;
        let context = create_test_context();
        let result = is_distinct_function(&empty, &context).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_is_distinct_single_item() {
        // Test isDistinct() on a single item
        let single = EvaluationResult::integer(42);
        let context = create_test_context();
        let result = is_distinct_function(&single, &context).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_is_distinct_all_unique() {
        // Test isDistinct() on a collection with all unique items
        let collection = create_test_collection(
            vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            false,
        );
        let context = create_test_context();
        let result = is_distinct_function(&collection, &context).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_is_distinct_with_duplicates() {
        // Test isDistinct() on a collection with duplicates
        let collection = create_test_collection(
            vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(1), // Duplicate
            ],
            false,
        );
        let context = create_test_context();
        let result = is_distinct_function(&collection, &context).unwrap();
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_distinct_empty_collection() {
        // Test distinct() on an empty collection
        let empty = EvaluationResult::Empty;
        let result = distinct_function(&empty).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_distinct_single_item() {
        // Test distinct() on a single item
        let single = EvaluationResult::integer(42);
        let result = distinct_function(&single).unwrap();
        assert_eq!(result, EvaluationResult::integer(42));
    }

    #[test]
    fn test_distinct_all_unique() {
        // Test distinct() on a collection with all unique items
        let collection = create_test_collection(
            vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            false,
        );
        let result = distinct_function(&collection).unwrap();

        // The result should be a collection with the same items
        // but has_undefined_order should be true (according to the spec)
        if let EvaluationResult::Collection {
            items,
            has_undefined_order,
            ..
        } = result
        {
            assert_eq!(items.len(), 3);
            assert!(has_undefined_order); // This should be true as per spec
            assert!(items.contains(&EvaluationResult::integer(1)));
            assert!(items.contains(&EvaluationResult::integer(2)));
            assert!(items.contains(&EvaluationResult::integer(3)));
        } else {
            panic!("Expected Collection result");
        }
    }

    #[test]
    fn test_distinct_with_duplicates() {
        // Test distinct() on a collection with duplicates
        let collection = create_test_collection(
            vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(1), // Duplicate
                EvaluationResult::integer(3),
                EvaluationResult::integer(2), // Duplicate
            ],
            false,
        );
        let result = distinct_function(&collection).unwrap();

        // The result should have unique items only
        if let EvaluationResult::Collection {
            items,
            has_undefined_order,
            ..
        } = result
        {
            assert_eq!(items.len(), 3);
            assert!(has_undefined_order); // This should be true as per spec

            // First occurrences should be preserved in order
            assert_eq!(items[0], EvaluationResult::integer(1));
            assert_eq!(items[1], EvaluationResult::integer(2));
            assert_eq!(items[2], EvaluationResult::integer(3));
        } else {
            panic!("Expected Collection result");
        }
    }

    #[test]
    fn test_normalize_collection_empty() {
        // Test normalize_collection_result with empty vec
        let result = normalize_collection_result(vec![], false);
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_normalize_collection_single() {
        // Test normalize_collection_result with single item
        let result = normalize_collection_result(vec![EvaluationResult::integer(42)], false);
        assert_eq!(result, EvaluationResult::integer(42));
    }

    #[test]
    fn test_normalize_collection_multiple() {
        // Test normalize_collection_result with multiple items
        let items = vec![EvaluationResult::integer(1), EvaluationResult::integer(2)];
        let result = normalize_collection_result(items.clone(), true);
        assert_eq!(
            result,
            EvaluationResult::Collection {
                items,
                has_undefined_order: true,
                type_info: None,
            }
        );
    }

    #[test]
    fn test_normalize_collection_nested() {
        // Test normalize_collection_result with a single nested collection
        let inner_collection = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1), EvaluationResult::integer(2)],
            has_undefined_order: true,
            type_info: None,
        };

        let result = normalize_collection_result(vec![inner_collection.clone()], false);
        assert_eq!(result, inner_collection);
    }

    #[test]
    fn test_simple_equality_integers() {
        assert!(simple_equality_check(
            &EvaluationResult::integer(42),
            &EvaluationResult::integer(42)
        ));
        assert!(!simple_equality_check(
            &EvaluationResult::integer(42),
            &EvaluationResult::integer(43)
        ));
    }

    #[test]
    fn test_simple_equality_strings() {
        assert!(simple_equality_check(
            &EvaluationResult::string("test".to_string()),
            &EvaluationResult::string("test".to_string())
        ));
        assert!(!simple_equality_check(
            &EvaluationResult::string("test".to_string()),
            &EvaluationResult::string("different".to_string())
        ));
    }

    #[test]
    fn test_simple_equality_different_types() {
        assert!(!simple_equality_check(
            &EvaluationResult::integer(42),
            &EvaluationResult::string("42".to_string())
        ));
    }
}
