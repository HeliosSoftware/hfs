//! SearchParameter registry — re-export shim.
//!
//! The registry implementation moved to [`helios_fhir::search::registry`]
//! so `helios-sof` can do compartment-aware filtering without a circular
//! dependency. This module re-exports the types and provides a thin
//! adapter for [`resolve_param_type`] that accepts the persistence-side
//! [`SearchValue`] type (the helios-fhir version takes `&[&str]`).

pub use helios_fhir::search::registry::{
    CompositeComponentDef, SearchParameterDefinition, SearchParameterRegistry,
    SearchParameterSource, SearchParameterStatus, resolve_param_targets,
};
pub use helios_fhir::search::types::SearchParamType;

use crate::error::SearchError;
use crate::types::{SearchModifier, SearchQuery, SearchValue};

/// Adapter wrapping [`helios_fhir::search::resolve_param_type`] so callers
/// can keep passing the persistence [`SearchValue`] type.
pub fn resolve_param_type(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    name: &str,
    values: &[SearchValue],
) -> SearchParamType {
    let strs: Vec<&str> = values.iter().map(|v| v.value.as_str()).collect();
    helios_fhir::search::registry::resolve_param_type(registry, resource_type, name, &strs)
}

/// The type to assume for a search parameter the registry does not know.
///
/// Conditional operations (`If-None-Exist`, conditional update/delete) parse a
/// raw query string with no `SearchParameter` definition to hand, so a
/// registry miss has to be guessed. The guess decides which index *column* the
/// query reads, so it must agree with the type the extractor wrote the row
/// under: guessing `String` for `_source` sends the query to `value_string`
/// while the row lives in `value_uri`, the match never fires, and
/// `If-None-Exist: Patient?_source=…` creates a duplicate on every request
/// rather than being the idempotency guard it exists to be.
///
/// The `_`-prefixed entries therefore mirror the embedded fallback definitions
/// in [`crate::search::loader`] exactly — including `_profile` as `Uri`, which
/// is what the embedded definition declares on every FHIR version even though
/// R5 and R6 re-type the spec's own copy as `reference`. The bare names below
/// are heuristics for the most common resource-level parameters, and `String`
/// remains the last-resort default.
///
/// This is a fallback, not a lookup: callers consult the registry first and
/// only land here when that misses.
pub fn fallback_param_type(name: &str) -> SearchParamType {
    match name {
        "_id" | "_tag" | "_security" | "identifier" => SearchParamType::Token,
        "_lastUpdated" => SearchParamType::Date,
        "_profile" | "_source" => SearchParamType::Uri,
        "patient" | "subject" | "encounter" | "performer" | "author" | "requester" | "recorder"
        | "asserter" | "practitioner" | "organization" | "location" | "device" => {
            SearchParamType::Reference
        }
        _ => SearchParamType::String,
    }
}

/// Rejects a modifier on `_id` or `_lastUpdated` that the backends do not
/// honour, instead of letting it degrade to a positive match.
///
/// Both parameters are answered from the `resources` table (or the top-level
/// document on Elasticsearch), not from the search index, so every backend
/// lowers them through a dedicated builder that sits outside the generic
/// modifier dispatch. Those builders honour `:not` and `:missing` on `_id`
/// and `:missing` on `_lastUpdated`; nothing else. Any other modifier that
/// reached them used to be dropped on the floor and the value consumed as a
/// plain match — `_id:not=abc` returned exactly `abc`, the precise inverse of
/// the request, with a 200 and a well-formed Bundle (#1055 on MongoDB, #1092
/// on SQLite, PostgreSQL and Elasticsearch).
///
/// The REST layer's `is_valid_for` gate does not stop this: `_id` resolves to
/// a token parameter, and every token modifier is spec-valid on it. This is
/// therefore a backend-side gate, called from each `search`/`search_count`
/// entry so it also covers the conditional-interaction paths that build a
/// `SearchQuery` without going through the REST extractor.
pub fn reject_unhonoured_metadata_modifiers(query: &SearchQuery) -> Result<(), SearchError> {
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
            return Err(SearchError::UnsupportedModifier {
                modifier: param
                    .modifier
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                param_type: param.param_type.to_string(),
            });
        }
    }
    Ok(())
}

/// Update notification for registry changes. Kept here as a stub for any
/// callers that still re-export it; the broadcast machinery was removed
/// during the move (no subscribers existed).
#[derive(Debug, Clone)]
pub enum RegistryUpdate {
    /// A parameter was added.
    Added(String),
    /// A parameter was removed.
    Removed(String),
    /// A parameter's status changed.
    StatusChanged(String, SearchParameterStatus),
    /// Registry was bulk-reloaded.
    Reloaded,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SearchParameter;

    fn param(
        name: &str,
        param_type: SearchParamType,
        modifier: Option<SearchModifier>,
    ) -> SearchQuery {
        SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: name.to_string(),
            param_type,
            modifier,
            values: vec![SearchValue::eq("x")],
            chain: vec![],
            components: vec![],
        })
    }

    #[test]
    fn honoured_metadata_modifiers_pass() {
        for (name, param_type, modifier) in [
            ("_id", SearchParamType::Token, None),
            ("_id", SearchParamType::Token, Some(SearchModifier::Not)),
            ("_id", SearchParamType::Token, Some(SearchModifier::Missing)),
            ("_lastUpdated", SearchParamType::Date, None),
            (
                "_lastUpdated",
                SearchParamType::Date,
                Some(SearchModifier::Missing),
            ),
        ] {
            let query = param(name, param_type, modifier.clone());
            assert!(
                reject_unhonoured_metadata_modifiers(&query).is_ok(),
                "{name}:{modifier:?} must be honoured"
            );
        }
    }

    #[test]
    fn unhonoured_metadata_modifiers_are_rejected_by_name() {
        for (name, param_type, modifier) in [
            ("_id", SearchParamType::Token, SearchModifier::Text),
            ("_id", SearchParamType::Token, SearchModifier::In),
            ("_id", SearchParamType::Token, SearchModifier::Above),
            ("_lastUpdated", SearchParamType::Date, SearchModifier::Not),
        ] {
            let query = param(name, param_type, Some(modifier.clone()));
            let err = reject_unhonoured_metadata_modifiers(&query).unwrap_err();
            assert!(
                matches!(
                    err,
                    SearchError::UnsupportedModifier { modifier: ref m, param_type: ref t }
                        if *m == modifier.to_string() && *t == param_type.to_string()
                ),
                "{name}:{modifier} must be rejected, got {err:?}"
            );
        }
    }

    #[test]
    fn ordinary_parameters_are_not_gated() {
        // The gate is keyed on the two metadata names only; a token `:text`
        // on a regular parameter is the index path's business.
        let query = param("gender", SearchParamType::Token, Some(SearchModifier::Text));
        assert!(reject_unhonoured_metadata_modifiers(&query).is_ok());
    }
}
