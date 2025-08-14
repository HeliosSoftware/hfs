use helios_fhirpath_support::{EvaluationError, EvaluationResult};

/// Implements the FHIRPath allTrue() function
///
/// The allTrue() function returns true if all items in the input collection
/// are true. It returns false if any item is false or empty.
/// For an empty collection, the result is true.
///
/// # Arguments
///
/// * `invocation_base` - The collection to check
///
/// # Returns
///
/// * `Ok(Boolean(true))` - If all items in the collection are true
/// * `Ok(Boolean(false))` - If any item in the collection is false or empty
/// * `Ok(Boolean(true))` - If the collection is empty
/// * `Err` - If any item in the collection is not a boolean
pub fn all_true_function(
    invocation_base: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // Extract items from the invocation base
    let items = match invocation_base {
        EvaluationResult::Collection { items, .. } => items.clone(),
        EvaluationResult::Empty => vec![],
        single_item => vec![single_item.clone()],
    };

    // allTrue is true for an empty collection
    if items.is_empty() {
        return Ok(EvaluationResult::boolean(true));
    }

    // Check each item in the collection
    for item in items {
        match item {
            EvaluationResult::Boolean(true, _) => continue,
            EvaluationResult::Boolean(false, _) | EvaluationResult::Empty => {
                return Ok(EvaluationResult::boolean(false));
            }
            // If any item is not boolean, it's an error according to spec
            _ => {
                return Err(EvaluationError::TypeError(
                    "allTrue expects a collection of Booleans".to_string(),
                ));
            }
        }
    }

    // All items are true
    Ok(EvaluationResult::boolean(true))
}

/// Implements the FHIRPath anyTrue() function
///
/// The anyTrue() function returns true if any item in the input collection
/// is true. It returns false if all items are false or empty.
/// For an empty collection, the result is false.
///
/// # Arguments
///
/// * `invocation_base` - The collection to check
///
/// # Returns
///
/// * `Ok(Boolean(true))` - If any item in the collection is true
/// * `Ok(Boolean(false))` - If all items in the collection are false or empty
/// * `Ok(Boolean(false))` - If the collection is empty
/// * `Err` - If any item in the collection is not a boolean
pub fn any_true_function(
    invocation_base: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // Extract items from the invocation base
    let items = match invocation_base {
        EvaluationResult::Collection { items, .. } => items.clone(),
        EvaluationResult::Empty => vec![],
        single_item => vec![single_item.clone()],
    };

    // anyTrue is false for an empty collection
    if items.is_empty() {
        return Ok(EvaluationResult::boolean(false));
    }

    // Check each item in the collection
    for item in items {
        match item {
            EvaluationResult::Boolean(true, _) => return Ok(EvaluationResult::boolean(true)),
            EvaluationResult::Boolean(false, _) | EvaluationResult::Empty => continue,
            // If any item is not boolean, it's an error according to spec
            _ => {
                return Err(EvaluationError::TypeError(
                    "anyTrue expects a collection of Booleans".to_string(),
                ));
            }
        }
    }

    // No true item found
    Ok(EvaluationResult::boolean(false))
}

/// Implements the FHIRPath allFalse() function
///
/// The allFalse() function returns true if all items in the input collection
/// are false. It returns false if any item is true or empty.
/// For an empty collection, the result is true.
///
/// # Arguments
///
/// * `invocation_base` - The collection to check
///
/// # Returns
///
/// * `Ok(Boolean(true))` - If all items in the collection are false
/// * `Ok(Boolean(false))` - If any item in the collection is true or empty
/// * `Ok(Boolean(true))` - If the collection is empty
/// * `Err` - If any item in the collection is not a boolean
pub fn all_false_function(
    invocation_base: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // Extract items from the invocation base
    let items = match invocation_base {
        EvaluationResult::Collection { items, .. } => items.clone(),
        EvaluationResult::Empty => vec![],
        single_item => vec![single_item.clone()],
    };

    // allFalse is true for an empty collection
    if items.is_empty() {
        return Ok(EvaluationResult::boolean(true));
    }

    // Check each item in the collection
    for item in items {
        match item {
            EvaluationResult::Boolean(false, _) => continue,
            EvaluationResult::Boolean(true, _) | EvaluationResult::Empty => {
                return Ok(EvaluationResult::boolean(false));
            }
            // If any item is not boolean, it's an error according to spec
            _ => {
                return Err(EvaluationError::TypeError(
                    "allFalse expects a collection of Booleans".to_string(),
                ));
            }
        }
    }

    // All items are false
    Ok(EvaluationResult::boolean(true))
}

/// Implements the FHIRPath anyFalse() function
///
/// The anyFalse() function returns true if any item in the input collection
/// is false. It returns false if all items are true or empty.
/// For an empty collection, the result is false.
///
/// # Arguments
///
/// * `invocation_base` - The collection to check
///
/// # Returns
///
/// * `Ok(Boolean(true))` - If any item in the collection is false
/// * `Ok(Boolean(false))` - If all items in the collection are true or empty
/// * `Ok(Boolean(false))` - If the collection is empty
/// * `Err` - If any item in the collection is not a boolean
pub fn any_false_function(
    invocation_base: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // Extract items from the invocation base
    let items = match invocation_base {
        EvaluationResult::Collection { items, .. } => items.clone(),
        EvaluationResult::Empty => vec![],
        single_item => vec![single_item.clone()],
    };

    // anyFalse is false for an empty collection
    if items.is_empty() {
        return Ok(EvaluationResult::boolean(false));
    }

    // Check each item in the collection
    for item in items {
        match item {
            EvaluationResult::Boolean(false, _) => return Ok(EvaluationResult::boolean(true)),
            EvaluationResult::Boolean(true, _) | EvaluationResult::Empty => continue,
            // If any item is not boolean, it's an error according to spec
            _ => {
                return Err(EvaluationError::TypeError(
                    "anyFalse expects a collection of Booleans".to_string(),
                ));
            }
        }
    }

    // No false item found
    Ok(EvaluationResult::boolean(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_true_empty_collection() {
        // Empty collection should return true
        let empty = EvaluationResult::Empty;
        let result = all_true_function(&empty).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_all_true_all_true_items() {
        // Collection with all true items should return true
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(true),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = all_true_function(&collection).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_all_true_some_false_items() {
        // Collection with some false items should return false
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(false),
                EvaluationResult::boolean(true),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = all_true_function(&collection).unwrap();
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_all_true_non_boolean_items() {
        // Collection with non-boolean items should error
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::boolean(true),
                EvaluationResult::integer(42),
                EvaluationResult::boolean(true),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = all_true_function(&collection);
        assert!(result.is_err());
    }

    #[test]
    fn test_any_true_empty_collection() {
        // Empty collection should return false
        let empty = EvaluationResult::Empty;
        let result = any_true_function(&empty).unwrap();
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_any_true_some_true_items() {
        // Collection with some true items should return true
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::boolean(false),
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(false),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = any_true_function(&collection).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_any_true_no_true_items() {
        // Collection with no true items should return false
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::boolean(false),
                EvaluationResult::boolean(false),
                EvaluationResult::boolean(false),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = any_true_function(&collection).unwrap();
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_all_false_empty_collection() {
        // Empty collection should return true
        let empty = EvaluationResult::Empty;
        let result = all_false_function(&empty).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_all_false_all_false_items() {
        // Collection with all false items should return true
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::boolean(false),
                EvaluationResult::boolean(false),
                EvaluationResult::boolean(false),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = all_false_function(&collection).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_all_false_some_true_items() {
        // Collection with some true items should return false
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::boolean(false),
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(false),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = all_false_function(&collection).unwrap();
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_any_false_empty_collection() {
        // Empty collection should return false
        let empty = EvaluationResult::Empty;
        let result = any_false_function(&empty).unwrap();
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_any_false_some_false_items() {
        // Collection with some false items should return true
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(false),
                EvaluationResult::boolean(true),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = any_false_function(&collection).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_any_false_no_false_items() {
        // Collection with no false items should return false
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(true),
            ],
            has_undefined_order: false,
            type_info: None,
        };
        let result = any_false_function(&collection).unwrap();
        assert_eq!(result, EvaluationResult::boolean(false));
    }
}
