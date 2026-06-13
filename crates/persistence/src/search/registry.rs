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
