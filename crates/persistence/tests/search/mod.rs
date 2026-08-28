//! Search operation tests for persistence backends.
//!
//! This module contains comprehensive tests for FHIR search operations
//! including all parameter types, modifiers, chaining, and pagination.

#[cfg(feature = "sqlite")]
use std::path::PathBuf;

#[cfg(feature = "sqlite")]
use helios_fhir::FhirVersion;
#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};

/// Builds an in-memory SQLite backend that has the spec `SearchParameter` set
/// loaded from the workspace `data/` directory.
///
/// Search is registry-driven: `create` only indexes the values named by a
/// registered `SearchParameter`, so a backend built with a bare
/// `SqliteBackend::in_memory()` (no `data_dir`) indexes nothing and every
/// query in this suite comes back empty. Production always loads the spec set
/// from `data_dir`; the sibling `sqlite_tests.rs` and `rest_conformance.rs`
/// harnesses do the same. Centralised here so all `search/` files share one
/// correctly-configured builder.
#[cfg(feature = "sqlite")]
pub fn make_sqlite_backend() -> SqliteBackend {
    make_sqlite_backend_for(FhirVersion::default_enabled())
}

/// Builds the same test backend with an explicit FHIR search-parameter set.
#[cfg(feature = "sqlite")]
pub fn make_sqlite_backend_for(fhir_version: FhirVersion) -> SqliteBackend {
    // CARGO_MANIFEST_DIR for these tests is crates/persistence.
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let config = SqliteBackendConfig {
        fhir_version,
        data_dir: Some(data_dir),
        ..Default::default()
    };
    let backend =
        SqliteBackend::with_config(":memory:", config).expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to initialize schema");
    backend
}

pub mod chained_tests;
pub mod date_tests;
/// Backend-agnostic scenarios, shared with the PostgreSQL test binary via
/// `#[path]` (issue #386).
pub mod fts_purge_suite;
pub mod fts_purge_tests;
pub mod include_tests;
/// Backend-agnostic scenarios, shared with the PostgreSQL test binary via
/// `#[path]` (issue #523). Complements the SQLite-only `meta_params_tests`
/// from #474: those cover the query side on one engine, this covers indexing
/// and querying on both.
pub mod meta_params_suite;
pub mod meta_params_suite_tests;
pub mod meta_params_tests;
pub mod modifier_tests;
pub mod number_tests;
pub mod pagination_tests;
pub mod quantity_tests;
pub mod reference_tests;
pub mod string_tests;
pub mod token_tests;
