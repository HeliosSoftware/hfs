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

use crate::types::SearchValue;

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
