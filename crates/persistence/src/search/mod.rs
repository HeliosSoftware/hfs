//! FHIR Search Parameter Management and Extraction.
//!
//! This module provides comprehensive search support including:
//!
//! - [`registry`] - In-memory registry of active SearchParameters
//! - [`loader`] - Loads parameters from embedded, stored, and config sources
//! - [`extractor`] - FHIRPath-based value extraction from resources
//! - [`converters`] - Conversion between FHIRPath results and index values
//! - [`date_value`] - The shared grammar, precision range and prefix mapping for date search values
//! - [`writer`] - Trait for writing extracted values to search indexes
//! - [`reindex`] - $reindex operation for rebuilding search indexes
//! - [`errors`] - Search-specific error types
//!
//! # Search Parameter Lifecycle
//!
//! ```text
//! 1. POST /SearchParameter (status: active)
//!    └── Registry updated
//!    └── New resources indexed with this parameter
//!    └── Existing resources NOT indexed (require $reindex)
//!
//! 2. $reindex operation
//!    └── Indexes existing resources for new/changed SearchParameters
//!    └── Can target specific resource types or all
//!
//! 3. PUT /SearchParameter (status: retired)
//!    └── Parameter no longer usable in searches
//!    └── Index entries remain (can be cleaned up later)
//!
//! 4. DELETE /SearchParameter
//!    └── Parameter removed from registry
//!    └── Index entries deleted
//! ```
//!
//! # Three Sources of SearchParameter Definitions
//!
//! 1. **Embedded Standard Parameters**: Built-in R4/R5/R6 standard search
//!    parameters (bundled at compile time)
//! 2. **Stored SearchParameter Resources**: Custom parameters POSTed to
//!    the server (persisted in database)
//! 3. **Runtime Configuration**: Optional config file for server-specific
//!    customizations
//!
//! # Example
//!
//! ```ignore
//! use helios_persistence::search::{
//!     SearchParameterRegistry, SearchParameterLoader, SearchParameterExtractor,
//! };
//!
//! // Load and register parameters
//! let loader = SearchParameterLoader::new(FhirVersion::R4);
//! let mut registry = SearchParameterRegistry::new();
//! registry.load_all(&loader).await?;
//!
//! // Extract searchable values from a resource
//! let extractor = SearchParameterExtractor::new(Arc::new(registry));
//! let values = extractor.extract(&patient_json, "Patient")?;
//!
//! for value in values {
//!     println!("{}: {:?}", value.param_name, value.value);
//! }
//! ```

pub mod chain_resolver;
pub mod conditional;
pub mod converters;
pub mod date_value;
pub mod errors;
pub mod extractor;
pub mod list_resolver;
pub mod loader;
pub mod metadata_modifier;
pub mod numeric_value;
pub mod range;
pub mod registry;
pub mod reindex;
pub mod seeder;
pub mod tenant_registries;
pub mod text_fold;
pub mod type_qualifier;
pub mod uri;
pub mod value_parser;
pub mod writer;

// Re-export main types
pub use chain_resolver::{
    ChainResolveOptions, TerminologyExpander, TerminologyExpansion, query_has_chains,
    resolve_chains, resolve_chains_with,
};
pub use conditional::{
    build_conditional_parameters, build_conditional_query, build_conditional_query_from_pairs,
    parse_conditional_criteria,
};
pub use converters::{
    IMPLICIT_TOKEN_SYSTEM, IndexValue, ValueConverter, implicit_system_candidates,
};
pub use date_value::{
    DatePredicate, DateValueError, DateValueErrorReason, DateValuePrecision, FhirDateValue,
    StorageResolution, validate_date_parameter, validate_date_values,
};
pub use errors::{ExtractionError, LoaderError, RegistryError, ReindexError};
pub use extractor::{ContainedExtraction, ExtractedValue, SearchParameterExtractor};
pub use list_resolver::{query_has_list, resolve_list};
pub use loader::SearchParameterLoader;
pub use metadata_modifier::reject_unsupported_metadata_modifier;
pub use numeric_value::{
    FhirNumberValue, FhirQuantityValue, NumberValueError, NumberValueErrorReason,
    validate_numeric_parameter, validate_numeric_values,
};
pub use range::{implicit_precision, implicit_range};
pub use registry::{
    RegistryUpdate, SearchParameterDefinition, SearchParameterRegistry, SearchParameterSource,
    SearchParameterStatus, fallback_param_type, resolve_param_targets, resolve_param_type,
};
pub use reindex::{
    DEFERRED_REINDEX_BATCH_SIZE, DeferredReindexLedger, ReindexOnFinish, ReindexOperation,
    ReindexProgress, ReindexProgressError, ReindexRequest, ReindexSource, ReindexStatus,
    ReindexTarget, ReindexableStorage, ResourcePage, ResourceRef, SkippedResource,
};
pub use seeder::{
    SeedOutcome, seed_spec_compartment_definitions, seed_spec_search_parameters,
    seed_tenant_conformance,
};
pub use tenant_registries::{StoredParamLoader, TenantSearchRegistries};
pub use text_fold::fold_text;
pub use type_qualifier::ResourceTypeScope;
pub use uri::compute_parent_uris;
pub use value_parser::{
    EMPTY_VALUE_REASON, has_empty_value, modifier_requires_terminology, param_requires_terminology,
    parse_typed_values, split_unescaped_commas, validate_modifier, validate_value_presence,
};
pub use writer::SearchIndexWriter;
