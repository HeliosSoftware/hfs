//! Pure helpers for MongoDB composite search parameters (#1206).
//!
//! Composite rows are stored one `search_index` document per component
//! value, all sharing `param_name` = the composite's own code and a
//! `composite_group` (the base-instance index) — the same layout SQLite
//! queries. A value `A$B` matches a resource when some `composite_group`
//! under the composite parameter has a row satisfying A and a row
//! satisfying B. Nothing here talks to MongoDB: the Mongo-facing wiring
//! (probing, per-batch pair checks) lives in `search_impl.rs`.
//! The value splitter shared with every backend is
//! `crate::search::composite_value::split_composite_value`.

use std::collections::HashSet;

use crate::types::{CompositeSearchComponent, SearchParameter, SearchValue};

/// Builds a synthetic single-value [`SearchParameter`] for one composite
/// component, suitable for passing to `MongoBackend::build_index_value_filter`.
///
/// `name` is set to the composite's own name because that is what every
/// `search_index` row for this composite carries in `param_name` — the
/// component has no row of its own to scope by.
pub(super) fn component_param(
    composite: &SearchParameter,
    component: &CompositeSearchComponent,
    value: SearchValue,
) -> SearchParameter {
    SearchParameter {
        name: composite.name.clone(),
        param_type: component.param_type,
        modifier: None,
        values: vec![value],
        chain: vec![],
        components: vec![],
    }
}

/// Intersects `(resource_id, composite_group)` pairs across components: a
/// resource id survives only if some single `composite_group` satisfies
/// every component (present in every set at that same pair).
///
/// Empty input (no components) yields an empty set.
pub(super) fn intersect_component_pairs(
    per_component: Vec<HashSet<(String, i32)>>,
) -> HashSet<String> {
    let mut iter = per_component.into_iter();
    let Some(mut surviving_pairs) = iter.next() else {
        return HashSet::new();
    };
    for next in iter {
        surviving_pairs.retain(|pair| next.contains(pair));
    }
    surviving_pairs.into_iter().map(|(id, _)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersect_pairs_drops_group_where_only_one_component_matches() {
        // Component A matches in group 0 only; component B matches nothing.
        let a: HashSet<(String, i32)> = [("r1".to_string(), 0)].into_iter().collect();
        let b: HashSet<(String, i32)> = HashSet::new();
        let result = intersect_component_pairs(vec![a, b]);
        assert!(result.is_empty());
    }

    #[test]
    fn intersect_pairs_rejects_cross_group_match() {
        // r1: component A matches in group 0, component B matches in group 1.
        // Not the same group, so r1 must not match.
        let a: HashSet<(String, i32)> = [("r1".to_string(), 0)].into_iter().collect();
        let b: HashSet<(String, i32)> = [("r1".to_string(), 1)].into_iter().collect();
        let result = intersect_component_pairs(vec![a, b]);
        assert!(result.is_empty());
    }

    #[test]
    fn intersect_pairs_accepts_same_group_match() {
        // r1 matches both components in group 1.
        let a: HashSet<(String, i32)> = [("r1".to_string(), 0), ("r1".to_string(), 1)]
            .into_iter()
            .collect();
        let b: HashSet<(String, i32)> = [("r1".to_string(), 1)].into_iter().collect();
        let result = intersect_component_pairs(vec![a, b]);
        assert_eq!(result, ["r1".to_string()].into_iter().collect());
    }

    #[test]
    fn intersect_pairs_empty_input_yields_empty_set() {
        let result = intersect_component_pairs(vec![]);
        assert!(result.is_empty());
    }
}
