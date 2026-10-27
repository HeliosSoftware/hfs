//! # FHIRPath Collection Functions
//!
//! Implements collection manipulation functions including `count()`, `empty()`, `exists()`,
//! `select()`, `where()`, and other collection operations.

use crate::evaluator::EvaluationContext;
use helios_fhirpath_support::{EvaluationError, EvaluationResult};

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
        EvaluationResult::Collection { items, .. } => EvaluationResult::integer(items.len() as i64),
        EvaluationResult::Empty => EvaluationResult::integer(0),
        _ => EvaluationResult::integer(1), // Single item counts as 1
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
        EvaluationResult::Empty => EvaluationResult::boolean(true),
        EvaluationResult::Collection { items, .. } => EvaluationResult::boolean(items.is_empty()),
        _ => EvaluationResult::boolean(false), // Single non-empty item is not empty
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
        EvaluationResult::Empty => EvaluationResult::boolean(false),
        EvaluationResult::Collection { items, .. } => EvaluationResult::boolean(!items.is_empty()),
        _ => EvaluationResult::boolean(true), // Single non-empty item exists
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
        EvaluationResult::Empty => EvaluationResult::boolean(true), // all() is true for empty
        EvaluationResult::Collection { items, .. } => {
            // Check if all items evaluate to true
            EvaluationResult::boolean(items.iter().all(|item| item.to_boolean()))
        }
        single_item => EvaluationResult::boolean(single_item.to_boolean()), // Check single item
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

    #[test]
    fn test_first_empty_collection() {
        // Test first() on an empty collection
        let empty = EvaluationResult::Empty;
        let context = EvaluationContext::new_empty_with_default_version();
        let result = first_function(&empty, &context).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_first_non_empty_collection() {
        // Test first() on a non-empty collection
        let collection = create_test_collection(
            vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            false,
        );
        let context = EvaluationContext::new_empty_with_default_version();
        let result = first_function(&collection, &context).unwrap();
        assert_eq!(result, EvaluationResult::integer(1));
    }

    #[test]
    fn test_first_single_item() {
        // Test first() on a single item
        let single = EvaluationResult::string("test".to_string());
        let context = EvaluationContext::new_empty_with_default_version();
        let result = first_function(&single, &context).unwrap();
        assert_eq!(result, EvaluationResult::string("test".to_string()));
    }

    #[test]
    fn test_first_undefined_order() {
        // Test first() on a collection with undefined order
        let collection = create_test_collection(
            vec![EvaluationResult::integer(1), EvaluationResult::integer(2)],
            true, // undefined order
        );
        let mut context = EvaluationContext::new_empty_with_default_version();

        // First test with check_ordered_functions = false (should succeed)
        context.check_ordered_functions = false;
        let result = first_function(&collection, &context);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), EvaluationResult::integer(1));

        // Then test with check_ordered_functions = true (should fail)
        context.check_ordered_functions = true;
        let result = first_function(&collection, &context);
        assert!(result.is_err());
    }

    #[test]
    fn test_last_empty_collection() {
        // Test last() on an empty collection
        let empty = EvaluationResult::Empty;
        let context = EvaluationContext::new_empty_with_default_version();
        let result = last_function(&empty, &context).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_last_non_empty_collection() {
        // Test last() on a non-empty collection
        let collection = create_test_collection(
            vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            false,
        );
        let context = EvaluationContext::new_empty_with_default_version();
        let result = last_function(&collection, &context).unwrap();
        assert_eq!(result, EvaluationResult::integer(3));
    }

    #[test]
    fn test_last_single_item() {
        // Test last() on a single item
        let single = EvaluationResult::string("test".to_string());
        let context = EvaluationContext::new_empty_with_default_version();
        let result = last_function(&single, &context).unwrap();
        assert_eq!(result, EvaluationResult::string("test".to_string()));
    }

    #[test]
    fn test_last_undefined_order() {
        // Test last() on a collection with undefined order
        let collection = create_test_collection(
            vec![EvaluationResult::integer(1), EvaluationResult::integer(2)],
            true, // undefined order
        );
        let mut context = EvaluationContext::new_empty_with_default_version();

        // First test with check_ordered_functions = false (should succeed)
        context.check_ordered_functions = false;
        let result = last_function(&collection, &context);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), EvaluationResult::integer(2));

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
        assert_eq!(result, EvaluationResult::integer(0));
    }

    #[test]
    fn test_count_non_empty_collection() {
        // Test count() on a non-empty collection
        let collection = create_test_collection(
            vec![
                EvaluationResult::integer(1),
                EvaluationResult::integer(2),
                EvaluationResult::integer(3),
            ],
            false,
        );
        let result = count_function(&collection);
        assert_eq!(result, EvaluationResult::integer(3));
    }

    #[test]
    fn test_count_single_item() {
        // Test count() on a single item
        let single = EvaluationResult::string("test".to_string());
        let result = count_function(&single);
        assert_eq!(result, EvaluationResult::integer(1));
    }

    #[test]
    fn test_empty_on_empty_collection() {
        // Test empty() on an empty collection
        let empty = EvaluationResult::Empty;
        let result = empty_function(&empty);
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_empty_on_non_empty_collection() {
        // Test empty() on a non-empty collection
        let collection = create_test_collection(vec![EvaluationResult::integer(1)], false);
        let result = empty_function(&collection);
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_empty_on_single_item() {
        // Test empty() on a single item
        let single = EvaluationResult::string("test".to_string());
        let result = empty_function(&single);
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_exists_on_empty_collection() {
        // Test exists() on an empty collection
        let empty = EvaluationResult::Empty;
        let result = exists_function(&empty);
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_exists_on_non_empty_collection() {
        // Test exists() on a non-empty collection
        let collection = create_test_collection(vec![EvaluationResult::integer(1)], false);
        let result = exists_function(&collection);
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_exists_on_single_item() {
        // Test exists() on a single item
        let single = EvaluationResult::string("test".to_string());
        let result = exists_function(&single);
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_all_on_empty_collection() {
        // Test all() on an empty collection
        let empty = EvaluationResult::Empty;
        let result = all_function(&empty);
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_all_on_all_true_collection() {
        // Test all() on a collection with all true values
        let collection = create_test_collection(
            vec![
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(true),
            ],
            false,
        );
        let result = all_function(&collection);
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_all_on_mixed_collection() {
        // Test all() on a collection with mixed boolean values
        let collection = create_test_collection(
            vec![
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(false),
            ],
            false,
        );
        let result = all_function(&collection);
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    #[test]
    fn test_all_on_single_true() {
        // Test all() on a single true value
        let single = EvaluationResult::boolean(true);
        let result = all_function(&single);
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_all_on_single_false() {
        // Test all() on a single false value
        let single = EvaluationResult::boolean(false);
        let result = all_function(&single);
        assert_eq!(result, EvaluationResult::boolean(false));
    }
}

/// Implements the FHIRPath sort() function
///
/// The sort() function sorts a collection of items. It can take optional
/// lambda expressions to specify sort keys. If no lambda is provided,
/// it sorts by the natural order of the items.
///
/// For descending sort, the lambda can use unary minus (-) operator.
/// Multiple sort keys are supported for multi-level sorting.
pub fn sort_function(
    invocation_base: &EvaluationResult,
    args: &[crate::parser::Expression],
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    // Convert to collection
    let items = match invocation_base {
        EvaluationResult::Empty => return Ok(EvaluationResult::Empty),
        EvaluationResult::Collection { items, .. } => items.clone(),
        single => vec![single.clone()],
    };

    if items.is_empty() {
        return Ok(EvaluationResult::Empty);
    }

    // If no arguments, sort by natural order
    if args.is_empty() {
        let mut sorted_items = items;
        sorted_items.sort_by(compare_evaluation_results);

        return Ok(if sorted_items.len() == 1 {
            sorted_items.into_iter().next().unwrap()
        } else {
            EvaluationResult::Collection {
                items: sorted_items,
                has_undefined_order: false,
                type_info: None,
            }
        });
    }

    // Process each sort key argument
    let mut sort_keys: Vec<(bool, crate::parser::Expression)> = Vec::new();
    for arg in args {
        // Check if it's a descending sort (starts with unary minus)
        let (is_descending, sort_expr) =
            if let crate::parser::Expression::Polarity('-', inner) = arg {
                (true, inner.as_ref().clone())
            } else {
                (false, arg.clone())
            };
        sort_keys.push((is_descending, sort_expr));
    }

    // Create items with all their sort keys evaluated
    let mut items_with_keys: Vec<(Vec<(bool, EvaluationResult)>, EvaluationResult)> = Vec::new();

    for item in &items {
        let mut keys = Vec::new();

        // Evaluate each sort expression for this item
        for (is_descending, sort_expr) in &sort_keys {
            // Set up context with $this as the current item
            let mut sort_context = context.clone();
            sort_context.this = Some(item.clone());

            // Evaluate the sort expression
            let sort_key = crate::evaluator::evaluate(sort_expr, &sort_context, Some(item))?;
            keys.push((*is_descending, sort_key));
        }

        items_with_keys.push((keys, item.clone()));
    }

    // Sort by the keys (multi-level sort)
    items_with_keys.sort_by(|a, b| {
        // Compare each sort key in order
        for (key_a, key_b) in a.0.iter().zip(b.0.iter()) {
            let is_descending = key_a.0;

            // Special handling for Empty values
            // In FHIRPath, Empty sorts first regardless of sort direction
            let ord = match (&key_a.1, &key_b.1) {
                (EvaluationResult::Empty, EvaluationResult::Empty) => std::cmp::Ordering::Equal,
                (EvaluationResult::Empty, _) => std::cmp::Ordering::Less, // Empty always sorts first
                (_, EvaluationResult::Empty) => std::cmp::Ordering::Greater, // Non-empty always sorts after empty
                _ => {
                    // Normal comparison for non-empty values
                    let ord = compare_evaluation_results(&key_a.1, &key_b.1);
                    // Apply descending if needed
                    if is_descending { ord.reverse() } else { ord }
                }
            };

            // If not equal, return the comparison result
            if ord != std::cmp::Ordering::Equal {
                return ord;
            }
            // If equal, continue to next sort key
        }

        // All keys are equal
        std::cmp::Ordering::Equal
    });

    // Extract the sorted items
    let sorted_items: Vec<EvaluationResult> =
        items_with_keys.into_iter().map(|(_, item)| item).collect();

    Ok(if sorted_items.len() == 1 {
        sorted_items.into_iter().next().unwrap()
    } else {
        EvaluationResult::Collection {
            items: sorted_items,
            has_undefined_order: false,
            type_info: None,
        }
    })
}

/// Compare two EvaluationResults for sorting
fn compare_evaluation_results(a: &EvaluationResult, b: &EvaluationResult) -> std::cmp::Ordering {
    use rust_decimal::Decimal;
    use std::cmp::Ordering;

    match (a, b) {
        // Empty values sort first
        (EvaluationResult::Empty, EvaluationResult::Empty) => Ordering::Equal,
        (EvaluationResult::Empty, _) => Ordering::Less,
        (_, EvaluationResult::Empty) => Ordering::Greater,

        // Boolean comparison
        (EvaluationResult::Boolean(a, _, _), EvaluationResult::Boolean(b, _, _)) => a.cmp(b),

        // Numeric comparisons
        (EvaluationResult::Integer(a, _, _), EvaluationResult::Integer(b, _, _)) => a.cmp(b),
        (EvaluationResult::Integer64(a, _, _), EvaluationResult::Integer64(b, _, _)) => a.cmp(b),
        (EvaluationResult::Decimal(a, _, _), EvaluationResult::Decimal(b, _, _)) => {
            // Decimal doesn't implement Ord, so we need to handle it
            if a < b {
                Ordering::Less
            } else if a > b {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }

        // Mixed numeric types - convert to Decimal for comparison
        (EvaluationResult::Integer(a, _, _), EvaluationResult::Decimal(b, _, _)) => {
            let a_dec = Decimal::from(*a);
            if a_dec < *b {
                Ordering::Less
            } else if a_dec > *b {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }
        (EvaluationResult::Decimal(a, _, _), EvaluationResult::Integer(b, _, _)) => {
            let b_dec = Decimal::from(*b);
            if a < &b_dec {
                Ordering::Less
            } else if a > &b_dec {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }
        (EvaluationResult::Integer(a, _, _), EvaluationResult::Integer64(b, _, _)) => a.cmp(b),
        (EvaluationResult::Integer64(a, _, _), EvaluationResult::Integer(b, _, _)) => a.cmp(b),

        // String comparison
        (EvaluationResult::String(a, _, _), EvaluationResult::String(b, _, _)) => a.cmp(b),

        // Date/Time comparisons
        (EvaluationResult::Date(a, _, _), EvaluationResult::Date(b, _, _)) => a.cmp(b),
        (EvaluationResult::DateTime(a, _, _), EvaluationResult::DateTime(b, _, _)) => a.cmp(b),
        (EvaluationResult::Time(a, _, _), EvaluationResult::Time(b, _, _)) => a.cmp(b),

        // Quantity comparison (only if same unit)
        (
            EvaluationResult::Quantity(val_a, unit_a, _, _),
            EvaluationResult::Quantity(val_b, unit_b, _, _),
        ) => {
            if unit_a == unit_b {
                if val_a < val_b {
                    Ordering::Less
                } else if val_a > val_b {
                    Ordering::Greater
                } else {
                    Ordering::Equal
                }
            } else {
                // Different units - sort by unit then value
                match unit_a.cmp(unit_b) {
                    Ordering::Equal => {
                        if val_a < val_b {
                            Ordering::Less
                        } else if val_a > val_b {
                            Ordering::Greater
                        } else {
                            Ordering::Equal
                        }
                    }
                    other => other,
                }
            }
        }

        // Different types - define a type ordering
        _ => {
            let type_order = |v: &EvaluationResult| match v {
                EvaluationResult::Empty => 0,
                EvaluationResult::Boolean(_, _, _) => 1,
                EvaluationResult::Integer(_, _, _) => 2,
                EvaluationResult::Integer64(_, _, _) => 3,
                EvaluationResult::Decimal(_, _, _) => 4,
                EvaluationResult::String(_, _, _) => 5,
                EvaluationResult::Date(_, _, _) => 6,
                EvaluationResult::DateTime(_, _, _) => 7,
                EvaluationResult::Time(_, _, _) => 8,
                EvaluationResult::Quantity(_, _, _, _) => 9,
                EvaluationResult::Collection { .. } => 10,
                EvaluationResult::Object { .. } => 11,
            };

            type_order(a).cmp(&type_order(b))
        }
    }
}
