//! Shared modifier gate for the metadata parameters (`_id`, `_lastUpdated`)
//! whose query builders are dispatched by parameter name rather than through
//! the generic per-type modifier handling.
//!
//! `_id` and `_lastUpdated` search directly against the resources table (or
//! its Elasticsearch equivalent), bypassing the modifier-aware machinery every
//! other parameter type goes through. Each backend's builder therefore has to
//! honour `param.modifier` itself, and can only honour a narrow set of it:
//!
//! - `_id`: `:not` (negation, #1092 — the fix mirrors MongoDB's
//!   `metadata_param_honoured` gate from #1055/#1091) and `:missing`.
//! - `_lastUpdated`: `:missing` only. The `_lastUpdated` builders read
//!   `param.values` and never `param.modifier`, so any other modifier
//!   (`:not` in particular) would be dropped and the value consumed as a
//!   plain date match — the same silent-inverse failure `_id:not` had.
//!
//! `:missing` is a presence test resolved earlier, before the value is ever
//! handed to either builder. Any other modifier must be rejected before the
//! builder runs, rather than silently degrading to a positive match.

use crate::error::{SearchError, StorageError, StorageResult};
use crate::types::{SearchModifier, SearchQuery};

/// Rejects any `_id` or `_lastUpdated` modifier that a backend's dedicated
/// builder for that parameter cannot honour.
///
/// Call this once, before building the query, from every entry point that
/// can reach those builders — not just the top-level `SearchProvider::search`/
/// `search_count`, but also any conditional-create (`ifNoneExist`) or
/// in-transaction search path that resolves criteria against the same
/// backend directly. See each backend's `search_impl.rs` for the specific
/// call sites and why they are all needed.
pub fn reject_unsupported_metadata_modifier(query: &SearchQuery) -> StorageResult<()> {
    for param in &query.parameters {
        let honoured = match param.name.as_str() {
            "_id" => matches!(
                param.modifier,
                None | Some(SearchModifier::Not) | Some(SearchModifier::Missing)
            ),
            "_lastUpdated" => matches!(param.modifier, None | Some(SearchModifier::Missing)),
            _ => true,
        };
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

    fn param(
        name: &str,
        param_type: SearchParamType,
        modifier: Option<SearchModifier>,
    ) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type,
            modifier,
            values: vec![SearchValue::eq("a")],
            chain: vec![],
            components: vec![],
        }
    }

    fn id_param(modifier: Option<SearchModifier>) -> SearchParameter {
        param("_id", SearchParamType::Token, modifier)
    }

    fn last_updated_param(modifier: Option<SearchModifier>) -> SearchParameter {
        param("_lastUpdated", SearchParamType::Date, modifier)
    }

    fn assert_rejected(query: &SearchQuery, expected_modifier: &str) {
        let err = reject_unsupported_metadata_modifier(query).unwrap_err();
        assert!(
            matches!(
                err,
                StorageError::Search(SearchError::UnsupportedModifier { ref modifier, .. })
                    if modifier == expected_modifier
            ),
            "expected UnsupportedModifier({expected_modifier}), got: {err:?}"
        );
    }

    #[test]
    fn id_none_not_and_missing_are_honoured() {
        for modifier in [
            None,
            Some(SearchModifier::Not),
            Some(SearchModifier::Missing),
        ] {
            let query = SearchQuery::new("Patient").with_parameter(id_param(modifier));
            assert!(reject_unsupported_metadata_modifier(&query).is_ok());
        }
    }

    #[test]
    fn id_other_modifiers_are_rejected() {
        let query =
            SearchQuery::new("Patient").with_parameter(id_param(Some(SearchModifier::Text)));
        assert_rejected(&query, "text");
    }

    #[test]
    fn last_updated_none_and_missing_are_honoured() {
        for modifier in [None, Some(SearchModifier::Missing)] {
            let query = SearchQuery::new("Patient").with_parameter(last_updated_param(modifier));
            assert!(reject_unsupported_metadata_modifier(&query).is_ok());
        }
    }

    #[test]
    fn last_updated_not_is_rejected() {
        // The `_lastUpdated` builders never read the modifier, so `:not`
        // would otherwise be consumed as a plain positive date match.
        let query = SearchQuery::new("Patient")
            .with_parameter(last_updated_param(Some(SearchModifier::Not)));
        assert_rejected(&query, "not");
    }

    #[test]
    fn last_updated_other_modifiers_are_rejected() {
        for (modifier, name) in [
            (SearchModifier::Text, "text"),
            (SearchModifier::Exact, "exact"),
            (SearchModifier::Contains, "contains"),
        ] {
            let query =
                SearchQuery::new("Patient").with_parameter(last_updated_param(Some(modifier)));
            assert_rejected(&query, name);
        }
    }

    #[test]
    fn non_metadata_parameters_are_ignored() {
        let query = SearchQuery::new("Patient").with_parameter(param(
            "name",
            SearchParamType::String,
            Some(SearchModifier::Text),
        ));
        assert!(reject_unsupported_metadata_modifier(&query).is_ok());
    }
}
