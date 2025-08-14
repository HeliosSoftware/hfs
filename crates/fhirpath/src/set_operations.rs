use crate::distinct_functions::normalize_collection_result;
use crate::evaluator::EvaluationContext;
use helios_fhirpath_support::EvaluationError;
use helios_fhirpath_support::EvaluationResult;
use std::collections::HashSet;

/// Implements the FHIRPath `intersect` function
///
/// Syntax: collection.intersect(other : collection) : collection
///
/// Returns a collection containing items that appear in both the input and other collections.
/// The result has no duplicates and the order is undefined.
///
/// # Arguments
///
/// * `invocation_base` - The input collection
/// * `other_collection` - The collection to find common elements with
/// * `context` - The evaluation context
///
/// # Returns
///
/// * A collection containing only the elements that appear in both collections
///
/// # Examples
///
/// ```text
/// [1, 2, 3].intersect([2, 3, 4]) = [2, 3]
/// [1, 2, 3].intersect([4, 5, 6]) = []
/// [1, 2, 1].intersect([1, 2]) = [1, 2]
/// [].intersect([1, 2, 3]) = []
/// [1, 2, 3].intersect([]) = []
/// ```
pub fn intersect_function(
    invocation_base: &EvaluationResult,
    other_collection: &EvaluationResult,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    // If either input is empty, the intersection is empty
    if invocation_base == &EvaluationResult::Empty || other_collection == &EvaluationResult::Empty {
        return Ok(EvaluationResult::Empty);
    }

    // Convert inputs to Vec for processing
    let left_items = match invocation_base {
        EvaluationResult::Collection { items, .. } => items.clone(),
        single_item => vec![single_item.clone()],
    };

    let right_items = match other_collection {
        EvaluationResult::Collection { items, .. } => items.clone(),
        single_item => vec![single_item.clone()],
    };

    let mut intersection_items = Vec::new();
    // Use HashSet for efficient duplicate checking in the result
    let mut added_items_set = HashSet::new();

    for left_item in &left_items {
        // Check if the left_item exists in the right_items (using equality '=')
        let exists_in_right = right_items
            .iter()
            .any(|right_item| equal_helper(left_item, right_item, context));

        if exists_in_right {
            // Attempt to insert the item into the HashSet.
            // If insert returns true, it means the item was not already present.
            if added_items_set.insert(left_item.clone()) {
                intersection_items.push(left_item.clone());
            }
        }
    }

    // intersect() output order is not guaranteed by spec, so mark as undefined.
    Ok(normalize_collection_result(intersection_items, true))
}

/// Implements the FHIRPath `exclude` function
///
/// Syntax: collection.exclude(other : collection) : collection
///
/// Returns a collection containing all items from the input collection that are not in the other collection.
/// The result preserves order and duplicates from the input collection.
///
/// # Arguments
///
/// * `invocation_base` - The input collection
/// * `other_collection` - The collection of elements to exclude
/// * `context` - The evaluation context
///
/// # Returns
///
/// * A collection containing elements from the input that don't appear in the other collection
///
/// # Examples
///
/// ```text
/// [1, 2, 3].exclude([2, 3, 4]) = [1]
/// [1, 2, 3].exclude([4, 5, 6]) = [1, 2, 3]
/// [1, 2, 1].exclude([2]) = [1, 1]
/// [].exclude([1, 2, 3]) = []
/// [1, 2, 3].exclude([]) = [1, 2, 3]
/// ```
pub fn exclude_function(
    invocation_base: &EvaluationResult,
    other_collection: &EvaluationResult,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    // If invocation_base is empty, result is empty
    if invocation_base == &EvaluationResult::Empty {
        return Ok(EvaluationResult::Empty);
    }
    // If other_collection is empty, result is invocation_base
    if other_collection == &EvaluationResult::Empty {
        return Ok(invocation_base.clone());
    }

    // Convert inputs to Vec for processing
    let left_items = match invocation_base {
        EvaluationResult::Collection { items, .. } => items.clone(),
        single_item => vec![single_item.clone()],
    };

    let right_items = match other_collection {
        EvaluationResult::Collection { items, .. } => items.clone(),
        single_item => vec![single_item.clone()],
    };

    let mut result_items = Vec::new();
    for left_item in &left_items {
        // Check if the left_item exists in the right_items (using equality '=')
        let exists_in_right = right_items
            .iter()
            .any(|right_item| equal_helper(left_item, right_item, context));

        // Keep the item if it does NOT exist in the right collection
        if !exists_in_right {
            result_items.push(left_item.clone());
        }
    }

    // exclude() preserves order of the left operand.
    let input_was_unordered = matches!(
        invocation_base,
        EvaluationResult::Collection {
            has_undefined_order: true,
            type_info: None,
            ..
        }
    );

    Ok(normalize_collection_result(
        result_items,
        input_was_unordered,
    ))
}

/// Implements the FHIRPath `union` function
///
/// Syntax: collection.union(other : collection) : collection
///
/// Returns a collection containing all unique items from both the input and other collections.
/// The result has no duplicates and the order is undefined.
///
/// # Arguments
///
/// * `invocation_base` - The input collection
/// * `other_collection` - The collection to combine with
/// * `context` - The evaluation context
///
/// # Returns
///
/// * A collection containing all unique elements from both input collections
///
/// # Examples
///
/// ```text
/// [1, 2, 3].union([2, 3, 4]) = [1, 2, 3, 4]
/// [1, 2, 3].union([4, 5, 6]) = [1, 2, 3, 4, 5, 6]
/// [1, 2, 1].union([1, 2]) = [1, 2]
/// [].union([1, 2, 3]) = [1, 2, 3]
/// [1, 2, 3].union([]) = [1, 2, 3]
/// ```
pub fn union_function(
    invocation_base: &EvaluationResult,
    other_collection: &EvaluationResult,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    // Convert inputs to Vec for processing
    let left_items = match invocation_base {
        EvaluationResult::Collection { items, .. } => items.clone(),
        EvaluationResult::Empty => vec![],
        single_item => vec![single_item.clone()],
    };

    let right_items = match other_collection {
        EvaluationResult::Collection { items, .. } => items.clone(),
        EvaluationResult::Empty => vec![],
        single_item => vec![single_item.clone()],
    };

    let mut union_items = Vec::new();
    // Use HashSet to track items already added to ensure uniqueness
    let mut added_items_set = HashSet::new();

    // Add items from the left collection if they haven't been added
    for item in left_items {
        if added_items_set.insert(item.clone()) {
            union_items.push(item);
        }
    }

    // Add items from the right collection if they haven't been added
    for item in right_items {
        if !union_items
            .iter()
            .any(|existing| equal_helper(existing, &item, context))
        {
            union_items.push(item);
        }
    }

    // union() output order is not guaranteed by spec, so mark as undefined.
    Ok(normalize_collection_result(union_items, true))
}

/// Implements the FHIRPath `combine` function
///
/// Syntax: collection.combine(other : collection) : collection
///
/// Returns a collection containing all items from both the input and other collections.
/// The result preserves all items including duplicates, and the order is undefined.
///
/// # Arguments
///
/// * `invocation_base` - The input collection
/// * `other_collection` - The collection to combine with
/// * `context` - The evaluation context
///
/// # Returns
///
/// * A collection containing all elements from both collections
///
/// # Examples
///
/// ```text
/// [1, 2, 3].combine([2, 3, 4]) = [1, 2, 3, 2, 3, 4]
/// [1, 2, 3].combine([4, 5, 6]) = [1, 2, 3, 4, 5, 6]
/// [1, 2, 1].combine([1, 2]) = [1, 2, 1, 1, 2]
/// [].combine([1, 2, 3]) = [1, 2, 3]
/// [1, 2, 3].combine([]) = [1, 2, 3]
/// ```
pub fn combine_function(
    invocation_base: &EvaluationResult,
    other_collection: &EvaluationResult,
    _context: &EvaluationContext, // Prefixed with underscore as it's intentionally unused
) -> Result<EvaluationResult, EvaluationError> {
    // Convert inputs to Vec for processing
    let left_items = match invocation_base {
        EvaluationResult::Collection { items, .. } => items.clone(),
        EvaluationResult::Empty => vec![],
        single_item => vec![single_item.clone()],
    };

    let right_items = match other_collection {
        EvaluationResult::Collection { items, .. } => items.clone(),
        EvaluationResult::Empty => vec![],
        single_item => vec![single_item.clone()],
    };

    // Concatenate the two vectors
    let mut combined_items = left_items;
    combined_items.extend(right_items);

    // combine() output order is not guaranteed by spec, so mark as undefined.
    Ok(normalize_collection_result(combined_items, true))
}

/// Helper function to check equality between two EvaluationResult values
///
/// This is a simplified version of the compare_equality function from evaluator.rs
/// that only handles the equality operation (=) and returns a boolean result.
fn equal_helper(
    left: &EvaluationResult,
    right: &EvaluationResult,
    _context: &EvaluationContext,
) -> bool {
    // Handler for collection equality
    if let (
        EvaluationResult::Collection { items: l_items, .. },
        EvaluationResult::Collection { items: r_items, .. },
    ) = (left, right)
    {
        if l_items.len() != r_items.len() {
            return false;
        }
        return l_items
            .iter()
            .zip(r_items.iter())
            .all(|(li, ri)| equal_helper(li, ri, _context));
    }

    // Handler for singleton collection equality
    if let (EvaluationResult::Collection { items, .. }, _) = (left, right) {
        if items.len() == 1 && !right.is_collection() {
            return equal_helper(&items[0], right, _context);
        }
    }

    if let (_, EvaluationResult::Collection { items, .. }) = (left, right) {
        if items.len() == 1 && !left.is_collection() {
            return equal_helper(left, &items[0], _context);
        }
    }

    // If one is a collection (but not a singleton), they're not equal
    if left.is_collection() || right.is_collection() {
        return false;
    }

    // Empty values
    if *left == EvaluationResult::Empty || *right == EvaluationResult::Empty {
        return false;
    }

    // Direct primitive value comparisons
    match (left, right) {
        (EvaluationResult::Boolean(l, _), EvaluationResult::Boolean(r, _)) => l == r,
        (EvaluationResult::String(l, _), EvaluationResult::String(r, _)) => l == r,
        (EvaluationResult::Integer(l, _), EvaluationResult::Integer(r, _)) => l == r,
        (EvaluationResult::Decimal(l, _), EvaluationResult::Decimal(r, _)) => l == r,
        (EvaluationResult::Decimal(l, _), EvaluationResult::Integer(r, _)) => {
            *l == rust_decimal::Decimal::from(*r)
        }
        (EvaluationResult::Integer(l, _), EvaluationResult::Decimal(r, _)) => {
            rust_decimal::Decimal::from(*l) == *r
        }
        (EvaluationResult::Date(l, _), EvaluationResult::Date(r, _)) => l == r,
        (EvaluationResult::DateTime(l, _), EvaluationResult::DateTime(r, _)) => l == r,
        (EvaluationResult::Time(l, _), EvaluationResult::Time(r, _)) => l == r,
        (
            EvaluationResult::Quantity(l_val, l_unit, _),
            EvaluationResult::Quantity(r_val, r_unit, _),
        ) => l_val == r_val && units_are_equivalent(l_unit, r_unit),
        // Fallback: not equal for different types
        _ => false,
    }
}

/// Helper function to check if two unit strings are equivalent
/// This handles common UCUM unit equivalences like "lbs" vs "[lb_av]"
fn units_are_equivalent(unit1: &str, unit2: &str) -> bool {
    // Direct string equality
    if unit1 == unit2 {
        return true;
    }

    // Handle common UCUM equivalences
    let normalized_unit1 = normalize_ucum_unit(unit1);
    let normalized_unit2 = normalize_ucum_unit(unit2);

    normalized_unit1 == normalized_unit2
}

/// Normalize UCUM unit strings to handle common equivalences
fn normalize_ucum_unit(unit: &str) -> &str {
    match unit {
        "lbs" | "[lb_av]" => "[lb_av]", // Normalize pounds to UCUM code
        "kg" | "[kg]" => "kg",          // Normalize kilograms
        "g" | "[g]" => "g",             // Normalize grams
        "mg" | "[mg]" => "mg",          // Normalize milligrams
        _ => unit,                      // Return as-is for other units
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    #[test]
    fn test_intersect_basic() {
        // Create test collections
        let collection1 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        let collection2 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
                EvaluationResult::integer(4),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test intersect
        let result = intersect_function(&collection1, &collection2, &context).unwrap();

        // Should return [2, 3] with undefined order (though we're not checking order here)
        // Since result may be in any order, we can't directly compare with a fixed expected collection
        match result {
            EvaluationResult::Collection {
                items,
                has_undefined_order,
                ..
            } => {
                assert_eq!(items.len(), 2);
                assert!(items.contains(&EvaluationResult::integer(2)));
                assert!(items.contains(&EvaluationResult::integer(3)));
                assert!(has_undefined_order); // Result should have undefined order
            }
            _ => panic!("Expected collection result"),
        }
    }

    #[test]
    fn test_intersect_no_common_elements() {
        // Create test collections
        let collection1 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        let collection2 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(4),
                EvaluationResult::integer(5),
                EvaluationResult::integer(6),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test intersect
        let result = intersect_function(&collection1, &collection2, &context).unwrap();

        // Should return an empty collection
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_intersect_with_duplicates() {
        // Create test collections
        let collection1 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        let collection2 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(2),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
                EvaluationResult::integer(4),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test intersect
        let result = intersect_function(&collection1, &collection2, &context).unwrap();

        // Should return [2, 3] with undefined order (no duplicates in result)
        match result {
            EvaluationResult::Collection {
                items,
                has_undefined_order,
                ..
            } => {
                assert_eq!(items.len(), 2);
                assert!(items.contains(&EvaluationResult::integer(2)));
                assert!(items.contains(&EvaluationResult::integer(3)));
                assert!(has_undefined_order); // Result should have undefined order
            }
            _ => panic!("Expected collection result"),
        }
    }

    #[test]
    fn test_intersect_with_empty() {
        // Create test collections
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        let empty = EvaluationResult::Empty;

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test intersect with empty
        let result = intersect_function(&collection, &empty, &context).unwrap();

        // Should return empty
        assert_eq!(result, EvaluationResult::Empty);

        // Test empty intersect
        let result = intersect_function(&empty, &collection, &context).unwrap();

        // Should return empty
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_exclude_basic() {
        // Create test collections
        let collection1 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        let collection2 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
                EvaluationResult::integer(4),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test exclude
        let result = exclude_function(&collection1, &collection2, &context).unwrap();

        // Should return [1] preserving order
        let expected = EvaluationResult::integer(1); // Normalized to single value
        assert_eq!(result, expected);
    }

    #[test]
    fn test_exclude_no_common_elements() {
        // Create test collections
        let collection1 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        let collection2 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(4),
                EvaluationResult::integer(5),
                EvaluationResult::integer(6),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test exclude
        let result = exclude_function(&collection1, &collection2, &context).unwrap();

        // Should return the entire first collection
        assert_eq!(result, collection1);
    }

    #[test]
    fn test_exclude_with_duplicates() {
        // Create test collections
        let collection1 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(1),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        let collection2 = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(2), EvaluationResult::integer(3)],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test exclude
        let result = exclude_function(&collection1, &collection2, &context).unwrap();

        // Should return [1, 1] preserving order and duplicates
        let expected = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1), EvaluationResult::integer(1)],
            has_undefined_order: false,
            type_info: None,
        };
        assert_eq!(result, expected);
    }

    #[test]
    fn test_exclude_with_empty() {
        // Create test collections
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        let empty = EvaluationResult::Empty;

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test exclude with empty (should return original collection)
        let result = exclude_function(&collection, &empty, &context).unwrap();

        // Should return the original collection
        assert_eq!(result, collection);

        // Test empty exclude
        let result = exclude_function(&empty, &collection, &context).unwrap();

        // Should return empty
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_union_basic() {
        // Create test collections
        let collection1 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        let collection2 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(3),
                EvaluationResult::integer(4),
                EvaluationResult::integer(5),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test union
        let result = union_function(&collection1, &collection2, &context).unwrap();

        // Should return [1, 2, 3, 4, 5] with undefined order
        match result {
            EvaluationResult::Collection {
                items,
                has_undefined_order,
                ..
            } => {
                assert_eq!(items.len(), 5);
                assert!(items.contains(&EvaluationResult::integer(1)));
                assert!(items.contains(&EvaluationResult::integer(2)));
                assert!(items.contains(&EvaluationResult::integer(3)));
                assert!(items.contains(&EvaluationResult::integer(4)));
                assert!(items.contains(&EvaluationResult::integer(5)));
                assert!(has_undefined_order); // Result should have undefined order
            }
            _ => panic!("Expected collection result"),
        }
    }

    #[test]
    fn test_union_with_duplicates() {
        // Create test collections
        let collection1 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        let collection2 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
                EvaluationResult::integer(3),
                EvaluationResult::integer(4),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test union
        let result = union_function(&collection1, &collection2, &context).unwrap();

        // Should return [1, 2, 3, 4] with undefined order (no duplicates)
        match result {
            EvaluationResult::Collection {
                items,
                has_undefined_order,
                ..
            } => {
                assert_eq!(items.len(), 4);
                assert!(items.contains(&EvaluationResult::integer(1)));
                assert!(items.contains(&EvaluationResult::integer(2)));
                assert!(items.contains(&EvaluationResult::integer(3)));
                assert!(items.contains(&EvaluationResult::integer(4)));
                assert!(has_undefined_order); // Result should have undefined order
            }
            _ => panic!("Expected collection result"),
        }
    }

    #[test]
    fn test_union_with_empty() {
        // Create test collections
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        let empty = EvaluationResult::Empty;

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test union with empty
        let result = union_function(&collection, &empty, &context).unwrap();

        // Result should have the same elements as collection, but with undefined order
        match result {
            EvaluationResult::Collection {
                items,
                has_undefined_order,
                ..
            } => {
                assert_eq!(items.len(), 3);
                assert!(items.contains(&EvaluationResult::integer(1)));
                assert!(items.contains(&EvaluationResult::integer(2)));
                assert!(items.contains(&EvaluationResult::integer(3)));
                assert!(has_undefined_order); // Result should have undefined order
            }
            _ => panic!("Expected collection result"),
        }

        // Test empty union
        let result = union_function(&empty, &collection, &context).unwrap();

        // Result should have the same elements as collection, but with undefined order
        match result {
            EvaluationResult::Collection {
                items,
                has_undefined_order,
                ..
            } => {
                assert_eq!(items.len(), 3);
                assert!(items.contains(&EvaluationResult::integer(1)));
                assert!(items.contains(&EvaluationResult::integer(2)));
                assert!(items.contains(&EvaluationResult::integer(3)));
                assert!(has_undefined_order); // Result should have undefined order
            }
            _ => panic!("Expected collection result"),
        }
    }

    #[test]
    fn test_combine_basic() {
        // Create test collections
        let collection1 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        let collection2 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(4),
                EvaluationResult::integer(5),
                EvaluationResult::integer(6),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test combine
        let result = combine_function(&collection1, &collection2, &context).unwrap();

        // Should return [1, 2, 3, 4, 5, 6] with undefined order
        match result {
            EvaluationResult::Collection {
                items,
                has_undefined_order,
                ..
            } => {
                assert_eq!(items.len(), 6);
                // Check items are included (order might vary)
                let mut found_count = 0;
                for i in 1..=6 {
                    if items.contains(&EvaluationResult::integer(i)) {
                        found_count += 1;
                    }
                }
                assert_eq!(found_count, 6);
                assert!(has_undefined_order); // Result should have undefined order
            }
            _ => panic!("Expected collection result"),
        }
    }

    #[test]
    fn test_combine_with_duplicates() {
        // Create test collections
        let collection1 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(2),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        let collection2 = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(2), EvaluationResult::integer(3)],
            has_undefined_order: false,
            type_info: None,
        };

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test combine
        let result = combine_function(&collection1, &collection2, &context).unwrap();

        // Should return [1, 2, 2, 2, 3] with undefined order
        match result {
            EvaluationResult::Collection {
                items,
                has_undefined_order,
                ..
            } => {
                assert_eq!(items.len(), 5);

                // Count occurrences of each number
                let mut count_1 = 0;
                let mut count_2 = 0;
                let mut count_3 = 0;

                for item in items {
                    match item {
                        EvaluationResult::Integer(1, _) => count_1 += 1,
                        EvaluationResult::Integer(2, _) => count_2 += 1,
                        EvaluationResult::Integer(3, _) => count_3 += 1,
                        _ => panic!("Unexpected item in collection"),
                    }
                }

                assert_eq!(count_1, 1);
                assert_eq!(count_2, 3);
                assert_eq!(count_3, 1);
                assert!(has_undefined_order); // Result should have undefined order
            }
            _ => panic!("Expected collection result"),
        }
    }

    #[test]
    fn test_combine_with_empty() {
        // Create test collections
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        let empty = EvaluationResult::Empty;

        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test combine with empty
        let result = combine_function(&collection, &empty, &context).unwrap();

        // Result should have the same elements as collection, but with undefined order
        match result {
            EvaluationResult::Collection {
                items,
                has_undefined_order,
                ..
            } => {
                assert_eq!(items.len(), 3);
                assert!(items.contains(&EvaluationResult::integer(1)));
                assert!(items.contains(&EvaluationResult::integer(2)));
                assert!(items.contains(&EvaluationResult::integer(3)));
                assert!(has_undefined_order); // Result should have undefined order
            }
            _ => panic!("Expected collection result"),
        }

        // Test empty combine
        let result = combine_function(&empty, &collection, &context).unwrap();

        // Result should have the same elements as collection, but with undefined order
        match result {
            EvaluationResult::Collection {
                items,
                has_undefined_order,
                type_info: _,
            } => {
                assert_eq!(items.len(), 3);
                assert!(items.contains(&EvaluationResult::integer(1)));
                assert!(items.contains(&EvaluationResult::integer(2)));
                assert!(items.contains(&EvaluationResult::integer(3)));
                assert!(has_undefined_order); // Result should have undefined order
            }
            _ => panic!("Expected collection result"),
        }
    }

    #[test]
    fn test_equal_helper() {
        // Create evaluation context
        let context = EvaluationContext::new_empty_with_default_version();

        // Test basic equality
        assert!(equal_helper(
            &EvaluationResult::integer(1),
            &EvaluationResult::integer(1),
            &context
        ));
        assert!(!equal_helper(
            &EvaluationResult::integer(1),
            &EvaluationResult::integer(2),
            &context
        ));

        // Test different types
        assert!(!equal_helper(
            &EvaluationResult::integer(1),
            &EvaluationResult::string("1".to_string()),
            &context
        ));

        // Test Number equality (Integer and Decimal)
        assert!(equal_helper(
            &EvaluationResult::integer(1),
            &EvaluationResult::decimal(Decimal::from(1)),
            &context
        ));
        assert!(equal_helper(
            &EvaluationResult::decimal(Decimal::from(1)),
            &EvaluationResult::integer(1),
            &context
        ));

        // Test collections
        let collection1 = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1), EvaluationResult::integer(2)],
            has_undefined_order: false,
            type_info: None,
        };

        let collection2 = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1), EvaluationResult::integer(2)],
            has_undefined_order: true, // Different order flag
            type_info: None,
        };

        let collection3 = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(2), EvaluationResult::integer(1)],
            has_undefined_order: false,
            type_info: None,
        };

        // Collections with same items but different order flags are equal
        assert!(equal_helper(&collection1, &collection2, &context));
        // Collections with different item order are not equal
        assert!(!equal_helper(&collection1, &collection3, &context));

        // Test singleton collections
        let singleton = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1)],
            has_undefined_order: false,
            type_info: None,
        };

        // Singleton collection equals its contained value
        assert!(equal_helper(
            &singleton,
            &EvaluationResult::integer(1),
            &context
        ));
        assert!(equal_helper(
            &EvaluationResult::integer(1),
            &singleton,
            &context
        ));
    }
}
