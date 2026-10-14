//! # FHIRPath Distinct Functions
//!
//! Implements `distinct()` and `isDistinct()` functions for removing duplicates from collections.
//!
//! Both functions treat two items as duplicates when the FHIRPath equals (`=`)
//! operator returns `true` for them, so they follow the operator's implicit
//! conversions: `1` and `1.0` are equal, decimals that differ only in trailing
//! zeros are equal, a `code` and a `string` with the same value are equal, and
//! date/time values are compared on the same instant across timezones.
//!
//! Two behaviours match the reference (HAPI/Java) implementation exercised by the
//! upstream `testDistinct` conformance group:
//!
//! - `distinct()` keeps the **last** of each set of equal items, in input order
//!   (`'a'.combine('b').combine('a').distinct()` is `'b', 'a'`).
//! - When two date/time values cannot be compared because of differing
//!   precision (`@2020 = @2020-01-01` is empty), whether they are distinct is
//!   unknown, so both functions return empty.

use crate::evaluator::{EvaluationContext, compare_equality};
use helios_fhirpath_support::{EvaluationError, EvaluationResult};
use rust_decimal::Decimal;
use std::collections::HashSet;

/// Outcome of comparing two items for `distinct()` / `isDistinct()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemEquality {
    Equal,
    NotEqual,
    /// The `=` operator returned empty for two temporal values (differing precision).
    Indeterminate,
}

fn is_temporal(item: &EvaluationResult) -> bool {
    matches!(
        item,
        EvaluationResult::Date(..) | EvaluationResult::DateTime(..) | EvaluationResult::Time(..)
    )
}

/// Compares two items with the FHIRPath `=` operator.
///
/// Only an empty result between two date/time values is indeterminate; any other
/// empty or error result (for example a primitive element that has extensions but
/// no value) counts as "not equal", per the spec's "false and empty both indicate
/// that the items are distinct".
fn item_equality(
    a: &EvaluationResult,
    b: &EvaluationResult,
    context: &EvaluationContext,
) -> ItemEquality {
    match compare_equality(a, "=", b, context) {
        Ok(EvaluationResult::Boolean(true, _, _)) => ItemEquality::Equal,
        Ok(EvaluationResult::Empty) if is_temporal(a) && is_temporal(b) => {
            ItemEquality::Indeterminate
        }
        _ => ItemEquality::NotEqual,
    }
}

/// Hash key for the fast path over collections of plain primitives whose `=`
/// semantics reduce to value equality on a canonical form.
#[derive(Hash, PartialEq, Eq)]
enum FastKey<'a> {
    Bool(bool),
    // string, code, uri, id, ... all compare by their string value
    Str(&'a str),
    // Integer and Decimal compare numerically; `normalize()` drops trailing zeros
    Num(Decimal),
}

fn fast_key(item: &EvaluationResult) -> Option<FastKey<'_>> {
    match item {
        EvaluationResult::Boolean(b, _, _) => Some(FastKey::Bool(*b)),
        EvaluationResult::String(s, _, _) => Some(FastKey::Str(s.as_str())),
        EvaluationResult::Integer(i, _, _) => Some(FastKey::Num(Decimal::from(*i))),
        EvaluationResult::Decimal(d, _, _) => Some(FastKey::Num(d.normalize())),
        _ => None,
    }
}

/// Returns, for each item, whether it is the last of its set of equal items
/// (i.e. whether `distinct()` keeps it), or `None` when some pair of items
/// cannot be compared.
fn last_occurrence_flags(
    items: &[EvaluationResult],
    context: &EvaluationContext,
) -> Option<Vec<bool>> {
    // Fast path: booleans, strings, integers and decimals hash on a canonical key.
    if let Some(keys) = items.iter().map(fast_key).collect::<Option<Vec<_>>>() {
        let mut seen = HashSet::with_capacity(keys.len());
        let mut keep = vec![false; keys.len()];
        for (i, key) in keys.into_iter().enumerate().rev() {
            keep[i] = seen.insert(key);
        }
        return Some(keep);
    }

    // General path: pairwise `=` (quantities with unit conversion, dates and
    // times, complex types, mixed collections).
    let mut keep = vec![true; items.len()];
    let mut indeterminate = false;
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            match item_equality(&items[i], &items[j], context) {
                ItemEquality::Equal => {
                    keep[i] = false;
                    break;
                }
                ItemEquality::Indeterminate => indeterminate = true,
                ItemEquality::NotEqual => {}
            }
        }
    }
    if indeterminate { None } else { Some(keep) }
}

fn collection_items(invocation_base: &EvaluationResult) -> &[EvaluationResult] {
    match invocation_base {
        EvaluationResult::Collection { items, .. } => items,
        EvaluationResult::Empty => &[],
        single_item => std::slice::from_ref(single_item),
    }
}

/// Implements the FHIRPath isDistinct() function
///
/// Returns true if all items in the input collection are distinct (no two are
/// equal under `=`). Returns true for empty and single-item collections, and
/// empty when two date/time items cannot be compared because of differing
/// precision.
pub fn is_distinct_function(
    invocation_base: &EvaluationResult,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    let items = collection_items(invocation_base);

    // Empty or single-item collections are always distinct
    if items.len() <= 1 {
        return Ok(EvaluationResult::boolean(true));
    }

    Ok(match last_occurrence_flags(items, context) {
        Some(keep) => EvaluationResult::boolean(keep.iter().all(|k| *k)),
        None => EvaluationResult::Empty,
    })
}

/// Implements the FHIRPath distinct() function
///
/// Returns the unique items of the input collection, keeping the last of each
/// set of equal items (under `=`) in input order. Returns empty if the input is
/// empty, or if two date/time items cannot be compared because of differing
/// precision.
pub fn distinct_function(
    invocation_base: &EvaluationResult,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    match invocation_base {
        EvaluationResult::Empty => Ok(EvaluationResult::Empty),
        EvaluationResult::Collection { items, .. } => {
            if items.is_empty() {
                return Ok(EvaluationResult::Empty);
            }
            if items.len() == 1 {
                return Ok(items[0].clone());
            }

            let Some(keep) = last_occurrence_flags(items, context) else {
                return Ok(EvaluationResult::Empty);
            };
            let distinct_items: Vec<EvaluationResult> = items
                .iter()
                .zip(keep)
                .filter(|(_, keep)| *keep)
                .map(|(item, _)| item.clone())
                .collect();

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
        let result = distinct_function(&empty, &create_test_context()).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_distinct_single_item() {
        // Test distinct() on a single item
        let single = EvaluationResult::integer(42);
        let result = distinct_function(&single, &create_test_context()).unwrap();
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
        let result = distinct_function(&collection, &create_test_context()).unwrap();

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
        let result = distinct_function(&collection, &create_test_context()).unwrap();

        // The result should have unique items only
        if let EvaluationResult::Collection {
            items,
            has_undefined_order,
            ..
        } = result
        {
            assert_eq!(items.len(), 3);
            assert!(has_undefined_order); // This should be true as per spec

            // The last of each set of equal items is kept, in input order
            // (1, 2, 1, 3, 2 -> 1, 3, 2), matching the reference implementation.
            assert_eq!(items[0], EvaluationResult::integer(1));
            assert_eq!(items[1], EvaluationResult::integer(3));
            assert_eq!(items[2], EvaluationResult::integer(2));
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
}
