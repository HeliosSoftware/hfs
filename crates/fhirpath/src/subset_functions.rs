use helios_fhirpath_support::EvaluationError;
use helios_fhirpath_support::EvaluationResult;
use std::collections::HashSet;

/// Implements the FHIRPath `subsetOf` function
///
/// Syntax: collection.subsetOf(other : collection) : Boolean
///
/// Returns true if the input collection is a subset of the collection passed as the argument
/// (that is, every element in the input collection is also in the argument collection).
///
/// # Arguments
///
/// * `invocation_base` - The input collection
/// * `other_collection` - The collection to check against
///
/// # Returns
///
/// * A Boolean value indicating whether the input collection is a subset of the argument collection
///
/// # Examples
///
/// ```text
/// [1, 2].subsetOf([1, 2, 3]) // true
/// [1, 2, 4].subsetOf([1, 2, 3]) // false
/// [].subsetOf([1, 2, 3]) // true (empty set is a subset of any set)
/// [1, 2, 3].subsetOf([]) // false
/// ```
pub fn subset_of_function(
    invocation_base: &EvaluationResult,
    other_collection: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    let self_items = match invocation_base {
        EvaluationResult::Collection { items, .. } => items,
        EvaluationResult::Empty => return Ok(EvaluationResult::boolean(true)), // Empty set is subset of anything
        single => &[single.clone()][..], // Treat single item as slice
    };

    let other_items = match other_collection {
        EvaluationResult::Collection { items, .. } => items,
        EvaluationResult::Empty => &[][..], // Empty slice
        single => &[single.clone()][..],    // Treat single item as slice
    };

    // Use HashSet for efficient lookup in the 'other' collection
    let other_set: HashSet<_> = other_items.iter().collect();

    // Check if every item in self_items is present in other_set
    let is_subset = self_items.iter().all(|item| other_set.contains(item));

    Ok(EvaluationResult::boolean(is_subset))
}

/// Implements the FHIRPath `supersetOf` function
///
/// Syntax: collection.supersetOf(other : collection) : Boolean
///
/// Returns true if the input collection is a superset of the collection passed as the argument
/// (that is, every element in the argument collection is also in the input collection).
///
/// # Arguments
///
/// * `invocation_base` - The input collection
/// * `other_collection` - The collection to check against
///
/// # Returns
///
/// * A Boolean value indicating whether the input collection is a superset of the argument collection
///
/// # Examples
///
/// ```text
/// [1, 2, 3].supersetOf([1, 2]) // true
/// [1, 2, 3].supersetOf([1, 2, 4]) // false
/// [1, 2, 3].supersetOf([]) // true (any set is a superset of empty set)
/// [].supersetOf([1, 2, 3]) // false
/// ```
pub fn superset_of_function(
    invocation_base: &EvaluationResult,
    other_collection: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    let self_items = match invocation_base {
        EvaluationResult::Collection { items, .. } => items,
        EvaluationResult::Empty => &[][..],
        single => &[single.clone()][..],
    };

    let other_items = match other_collection {
        EvaluationResult::Collection { items, .. } => items,
        EvaluationResult::Empty => return Ok(EvaluationResult::boolean(true)), // Anything is superset of empty set
        single => &[single.clone()][..],
    };

    // Use HashSet for efficient lookup in the 'self' collection
    let self_set: HashSet<_> = self_items.iter().collect();

    // Check if every item in other_items is present in self_set
    let is_superset = other_items.iter().all(|item| self_set.contains(item));

    Ok(EvaluationResult::boolean(is_superset))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subset_of_simple_collections() {
        // Create test collections
        let collection1 = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1), EvaluationResult::integer(2)],
            has_undefined_order: false,
            type_info: None,
        };

        let collection2 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Test [1, 2].subsetOf([1, 2, 3]) should be true
        let result = subset_of_function(&collection1, &collection2).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));

        // Test [1, 2, 3].subsetOf([1, 2]) should be false
        let result = subset_of_function(&collection2, &collection1).unwrap();
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_subset_of_empty_collections() {
        // Create test collections
        let empty = EvaluationResult::Empty;
        let collection = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1), EvaluationResult::integer(2)],
            has_undefined_order: false,
            type_info: None,
        };

        // Test [].subsetOf([1, 2]) should be true (empty set is subset of anything)
        let result = subset_of_function(&empty, &collection).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));

        // Test [1, 2].subsetOf([]) should be false
        let result = subset_of_function(&collection, &empty).unwrap();
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_subset_of_with_single_items() {
        // Create test items
        let item1 = EvaluationResult::integer(1);
        let collection = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1), EvaluationResult::integer(2)],
            has_undefined_order: false,
            type_info: None,
        };

        // Test 1.subsetOf([1, 2]) should be true
        let result = subset_of_function(&item1, &collection).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));

        // Create a single item not in the collection
        let item3 = EvaluationResult::integer(3);

        // Test 3.subsetOf([1, 2]) should be false
        let result = subset_of_function(&item3, &collection).unwrap();
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_superset_of_simple_collections() {
        // Create test collections
        let collection1 = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1), EvaluationResult::integer(2)],
            has_undefined_order: false,
            type_info: None,
        };

        let collection2 = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Test [1, 2, 3].supersetOf([1, 2]) should be true
        let result = superset_of_function(&collection2, &collection1).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));

        // Test [1, 2].supersetOf([1, 2, 3]) should be false
        let result = superset_of_function(&collection1, &collection2).unwrap();
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_superset_of_empty_collections() {
        // Create test collections
        let empty = EvaluationResult::Empty;
        let collection = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1), EvaluationResult::integer(2)],
            has_undefined_order: false,
            type_info: None,
        };

        // Test [1, 2].supersetOf([]) should be true (any set is superset of empty set)
        let result = superset_of_function(&collection, &empty).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));

        // Test [].supersetOf([1, 2]) should be false
        let result = superset_of_function(&empty, &collection).unwrap();
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_superset_of_with_single_items() {
        // Create test items
        let item1 = EvaluationResult::integer(1);
        let collection = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1), EvaluationResult::integer(2)],
            has_undefined_order: false,
            type_info: None,
        };

        // Test [1, 2].supersetOf(1) should be true
        let result = superset_of_function(&collection, &item1).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));

        // Create a single item not in the collection
        let item3 = EvaluationResult::integer(3);

        // Test [1, 2].supersetOf(3) should be false
        let result = superset_of_function(&collection, &item3).unwrap();
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_equal_collections() {
        // Create identical collections
        let collection1 = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1), EvaluationResult::integer(2)],
            has_undefined_order: false,
            type_info: None,
        };

        let collection2 = EvaluationResult::Collection {
            items: vec![EvaluationResult::integer(1), EvaluationResult::integer(2)],
            has_undefined_order: false,
            type_info: None,
        };

        // Test [1, 2].subsetOf([1, 2]) should be true
        let result = subset_of_function(&collection1, &collection2).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));

        // Test [1, 2].supersetOf([1, 2]) should be true
        let result = superset_of_function(&collection1, &collection2).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));
    }
}
