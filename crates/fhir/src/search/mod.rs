//! FHIR SearchParameter spec data.
//!
//! Foundational spec types (definition, registry, loader) lifted from
//! `helios-persistence` so `helios-sof` can use them for compartment-aware
//! filtering without a circular dependency. The runtime/index machinery
//! (extractor, writer, reindex, converters) stays in
//! `helios-persistence::search` where it belongs.
//!
//! - [`types`] – `SearchParamType` (FHIR ptypes enum).
//! - [`registry`] – `SearchParameterRegistry`, `SearchParameterDefinition`,
//!   status/source enums, and the deterministic type-resolution helpers.
//! - [`loader`] – `SearchParameterLoader` for parsing embedded fallbacks,
//!   FHIR spec bundles, and custom SearchParameter files.
//! - [`errors`] – `LoaderError`, `RegistryError`.

pub mod errors;
pub mod loader;
pub mod registry;
pub mod types;

pub use errors::{LoaderError, RegistryError};
pub use loader::SearchParameterLoader;
pub use registry::{
    CompositeComponentDef, SearchParameterDefinition, SearchParameterRegistry,
    SearchParameterSource, SearchParameterStatus, resolve_param_targets, resolve_param_type,
};
pub use types::SearchParamType;
