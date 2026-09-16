//! Shared `_id` modifier gate for backends whose `_id` query builder is
//! dispatched by parameter name rather than through the generic per-type
//! modifier handling.
//!
//! `_id` searches directly against the resources table (or its Elasticsearch
//! equivalent), bypassing the modifier-aware machinery every other parameter
//! type goes through. Each backend's `_id` builder therefore has to honour
//! `param.modifier` itself, and can only honour a narrow set of it: `:not`
//! (negation, #1092 — the fix mirrors MongoDB's `metadata_param_honoured`
//! gate from #1055/#1091) and `:missing` (a presence test resolved earlier,
//! before the value is ever handed to the `_id` builder). Any other modifier
//! must be rejected before the builder runs, rather than silently degrading
//! to a positive match the way `:not` used to.

use crate::error::{SearchError, StorageError, StorageResult};
use crate::types::{SearchModifier, SearchQuery};

/// Rejects any `_id` modifier that a backend's `_id` query builder cannot
/// honour.
///
/// Call this once, before building the query, from every entry point that
/// can reach the `_id` builder — not just the top-level `SearchProvider::search`/
/// `search_count`, but also any conditional-create (`ifNoneExist`) or
/// in-transaction search path that resolves criteria against the same
/// backend directly. See each backend's `search_impl.rs` for the specific
/// call sites and why they are all needed.
pub fn reject_unsupported_id_modifier(query: &SearchQuery) -> StorageResult<()> {
    for param in &query.parameters {
        if param.name != "_id" {
            continue;
        }
        let honoured = matches!(
            param.modifier,
            None | Some(SearchModifier::Not) | Some(SearchModifier::Missing)
        );
        if !honoured {
            return Err(StorageError::Search(SearchError::UnsupportedModifier {
                modifier: param
                    .modifier
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                param_type: param.param_type.to_string(),
            }));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SearchParamType, SearchParameter, SearchValue};

    fn id_param(modifier: Option<SearchModifier>) -> SearchParameter {
        SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier,
            values: vec![SearchValue::eq("a")],
            chain: vec![],
            components: vec![],
        }
    }

    #[test]
    fn none_not_and_missing_are_honoured() {
        for modifier in [
            None,
            Some(SearchModifier::Not),
            Some(SearchModifier::Missing),
        ] {
            let query = SearchQuery::new("Patient").with_parameter(id_param(modifier));
            assert!(reject_unsupported_id_modifier(&query).is_ok());
        }
    }

    #[test]
    fn other_modifiers_are_rejected() {
        let query =
            SearchQuery::new("Patient").with_parameter(id_param(Some(SearchModifier::Text)));
        let err = reject_unsupported_id_modifier(&query).unwrap_err();
        assert!(matches!(
            err,
            StorageError::Search(SearchError::UnsupportedModifier { ref modifier, .. })
                if modifier == "text"
        ));
    }

    #[test]
    fn non_id_parameters_are_ignored() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: Some(SearchModifier::Text),
            values: vec![SearchValue::eq("a")],
            chain: vec![],
            components: vec![],
        });
        assert!(reject_unsupported_id_modifier(&query).is_ok());
    }
}
