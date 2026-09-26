//! Pure helpers for MongoDB composite search parameters (#1206).
//!
//! Composite rows are stored one `search_index` document per component
//! value, all sharing `param_name` = the composite's own code and a
//! `composite_group` (the base-instance index) — the same layout SQLite
//! queries. A value `A$B` matches a resource when some `composite_group`
//! under the composite parameter has a row satisfying A and a row
//! satisfying B. Nothing here talks to MongoDB: the Mongo-facing wiring
//! (probing, per-batch pair checks) lives in `search_impl.rs`.

use std::collections::HashSet;

use crate::error::{SearchError, StorageError, StorageResult};
use crate::types::{
    CompositeSearchComponent, SearchParamType, SearchParameter, SearchPrefix, SearchValue,
};

/// Comparison prefixes recognized on a composite component part, tried in
/// this order.
///
/// This deliberately diverges from Postgres `parse_component_value`
/// (`backends/postgres/search/query_builder.rs`): Postgres (and SQLite)
/// strip a leading comparison prefix for *every* component type, because
/// each component gets its own typed column/slot to compare against.
/// MongoDB writes one row per component, with a `composite_slot` to
/// distinguish components of the same type. A token
/// component's part must be scoped by `parse_component_part` below rather
/// than mirrored verbatim. Stripping is therefore applied only for the
/// ordered types (Date/Number/Quantity) where a prefix is meaningful; a
/// token code like `left` (or `ge123`, a plausible identifier) must keep
/// its letters, since `ge`/`eq`/etc. are not comparison prefixes for
/// token/string/reference/uri values and stripping them would corrupt the
/// value being searched for.
const PREFIXES: [(&str, SearchPrefix); 9] = [
    ("ne", SearchPrefix::Ne),
    ("gt", SearchPrefix::Gt),
    ("lt", SearchPrefix::Lt),
    ("ge", SearchPrefix::Ge),
    ("le", SearchPrefix::Le),
    ("sa", SearchPrefix::Sa),
    ("eb", SearchPrefix::Eb),
    ("ap", SearchPrefix::Ap),
    ("eq", SearchPrefix::Eq),
];

/// Splits a raw composite value (`"http://loinc.org|8302-2$gt150"`) into one
/// [`SearchValue`] per declared component, in component order.
///
/// A leading comparison prefix is stripped only for Date/Number/Quantity
/// components — a token code like `ge123` must keep its letters, since
/// `ge`/`eq`/etc. are not meaningful prefixes for token/string/reference/uri
/// values.
///
/// Returns [`SearchError::InvalidComposite`] when there are no declared
/// components, or the value's `$`-separated part count does not match the
/// component count.
pub(super) fn split_composite_value(
    raw: &str,
    components: &[CompositeSearchComponent],
) -> StorageResult<Vec<SearchValue>> {
    if components.is_empty() {
        return Err(StorageError::Search(SearchError::InvalidComposite {
            message: "composite search parameter has no declared components".to_string(),
        }));
    }

    let parts: Vec<&str> = raw.split('$').collect();
    if parts.len() != components.len() {
        return Err(StorageError::Search(SearchError::InvalidComposite {
            message: format!(
                "composite value '{raw}' has {} component(s) separated by '$', but this \
                 parameter expects {}",
                parts.len(),
                components.len()
            ),
        }));
    }

    Ok(parts
        .into_iter()
        .zip(components.iter())
        .map(|(part, component)| parse_component_part(part, component.param_type))
        .collect())
}

/// Parses one `$`-separated part of a composite value into a [`SearchValue`],
/// stripping a leading comparison prefix only for ordered types (Date,
/// Number, Quantity). Token/String/Reference/Uri parts are kept verbatim
/// with [`SearchPrefix::Eq`].
fn parse_component_part(part: &str, param_type: SearchParamType) -> SearchValue {
    if !matches!(
        param_type,
        SearchParamType::Date | SearchParamType::Number | SearchParamType::Quantity
    ) {
        return SearchValue::new(SearchPrefix::Eq, part);
    }

    for (prefix_str, prefix) in PREFIXES {
        if let Some(stripped) = part.strip_prefix(prefix_str) {
            return SearchValue::new(prefix, stripped);
        }
    }

    SearchValue::new(SearchPrefix::Eq, part)
}

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

    fn token(name: &str) -> CompositeSearchComponent {
        CompositeSearchComponent {
            param_type: SearchParamType::Token,
            param_name: name.to_string(),
        }
    }

    fn quantity(name: &str) -> CompositeSearchComponent {
        CompositeSearchComponent {
            param_type: SearchParamType::Quantity,
            param_name: name.to_string(),
        }
    }

    #[test]
    fn arity_mismatch_too_few_parts_errors() {
        let components = vec![token("code"), quantity("value-quantity")];
        let err = split_composite_value("8302-2", &components).unwrap_err();
        match err {
            StorageError::Search(SearchError::InvalidComposite { message }) => {
                assert!(
                    message.contains('1'),
                    "message should name 1 part: {message}"
                );
                assert!(
                    message.contains('2'),
                    "message should name 2 expected: {message}"
                );
            }
            other => panic!("expected InvalidComposite, got {other:?}"),
        }
    }

    #[test]
    fn arity_mismatch_too_many_parts_errors() {
        let components = vec![token("code"), quantity("value-quantity")];
        let err = split_composite_value("a$b$c", &components).unwrap_err();
        assert!(matches!(
            err,
            StorageError::Search(SearchError::InvalidComposite { .. })
        ));
    }

    #[test]
    fn no_declared_components_errors() {
        let err = split_composite_value("a$b", &[]).unwrap_err();
        assert!(matches!(
            err,
            StorageError::Search(SearchError::InvalidComposite { .. })
        ));
    }

    #[test]
    fn quantity_component_strips_known_prefix() {
        let components = vec![token("code"), quantity("value-quantity")];
        let values = split_composite_value("8302-2$gt150", &components).unwrap();
        assert_eq!(values[0].prefix, SearchPrefix::Eq);
        assert_eq!(values[0].value, "8302-2");
        assert_eq!(values[1].prefix, SearchPrefix::Gt);
        assert_eq!(values[1].value, "150");
    }

    #[test]
    fn token_component_keeps_prefix_like_letters_verbatim() {
        let components = vec![token("code"), quantity("value-quantity")];
        // "ge123" looks like a `ge` prefix, but this component is a Token, so
        // the whole string must survive untouched.
        let values = split_composite_value("ge123$50", &components).unwrap();
        assert_eq!(values[0].prefix, SearchPrefix::Eq);
        assert_eq!(values[0].value, "ge123");
    }

    #[test]
    fn token_component_with_system_is_untouched() {
        let components = vec![token("code"), quantity("value-quantity")];
        let values = split_composite_value("http://loinc.org|8302-2$150", &components).unwrap();
        assert_eq!(values[0].prefix, SearchPrefix::Eq);
        assert_eq!(values[0].value, "http://loinc.org|8302-2");
    }

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
