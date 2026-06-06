//! FHIR Search Parameter Management and Extraction.
//!
//! This module provides comprehensive search support including:
//!
//! - [`registry`] - In-memory registry of active SearchParameters
//! - [`loader`] - Loads parameters from embedded, stored, and config sources
//! - [`extractor`] - FHIRPath-based value extraction from resources
//! - [`converters`] - Conversion between FHIRPath results and index values
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
pub mod converters;
pub mod errors;
pub mod extractor;
pub mod loader;
pub mod registry;
pub mod reindex;
pub mod text_fold;
pub mod writer;

// Re-export main types
pub use chain_resolver::{query_has_chains, resolve_chains};
pub use converters::{IndexValue, ValueConverter};
pub use errors::{ExtractionError, LoaderError, RegistryError, ReindexError};
pub use extractor::{ExtractedValue, SearchParameterExtractor};
pub use loader::SearchParameterLoader;
pub use registry::{
    RegistryUpdate, SearchParameterDefinition, SearchParameterRegistry, SearchParameterSource,
    SearchParameterStatus, resolve_param_targets, resolve_param_type,
};
pub use reindex::{
    ReindexOperation, ReindexProgress, ReindexRequest, ReindexStatus, ReindexableStorage,
    ResourcePage,
};
pub use text_fold::fold_text;
pub use writer::SearchIndexWriter;
