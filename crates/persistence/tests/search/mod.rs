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

/// Backend-agnostic scenarios, shared with the PostgreSQL, MongoDB and
/// Elasticsearch test binaries via `#[path]` (#1390).
pub mod ap_prefix_suite;
/// Backend-agnostic `ap` scenarios for quantity composites,
/// shared with the PostgreSQL, MongoDB and Elasticsearch test binaries via
/// `#[path]` (#1390).
pub mod ap_relations_suite;
pub mod chained_tests;
/// The date component of a composite compares as a point on a Period's start,
/// not as the range a date parameter compares (#1391).
pub mod composite_period_pin;
/// Backend-agnostic scenarios, shared with the PostgreSQL, MongoDB and
/// Elasticsearch test binaries via `#[path]` (#1293, #1295, #1296, #1297).
/// Backend-agnostic scenarios, shared with the PostgreSQL and MongoDB test
/// binaries via `#[path]` (#1315).
pub mod date_minute_index_suite;
/// Backend-agnostic scenarios for Period and Timing targets, shared with the
/// PostgreSQL, MongoDB and Elasticsearch test binaries via `#[path]` (#1391).
pub mod date_period_suite;
pub mod date_precision_suite;
pub mod date_tests;
/// Backend-agnostic scenarios, shared with the PostgreSQL, MongoDB and
/// Elasticsearch test binaries via `#[path]` (#1380).
pub mod empty_value_suite;
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
/// Backend-agnostic scenarios, shared with the PostgreSQL, MongoDB and
/// Elasticsearch test binaries via `#[path]` (#1408).
pub mod modifier_parity_suite;
pub mod modifier_tests;
/// Backend-agnostic scenarios, shared with the PostgreSQL, MongoDB and
/// Elasticsearch test binaries via `#[path]` (#1337).
pub mod number_exponent_suite;
pub mod number_tests;
/// Backend-agnostic scenarios, shared with the PostgreSQL, MongoDB and
/// Elasticsearch test binaries via `#[path]` (#1340).
pub mod numeric_validation_suite;
pub mod pagination_tests;
pub mod quantity_tests;
pub mod reference_tests;
pub mod string_tests;
/// Backend-agnostic scenarios, shared with the PostgreSQL, MongoDB and
/// Elasticsearch test binaries via `#[path]` (#1379).
pub mod token_code_system_suite;
pub mod token_tests;

/// The shared Period table (#1391): a Period is one range, a missing side is
/// unbounded, and every prefix follows the FHIR rules for range targets.
/// PostgreSQL, MongoDB and Elasticsearch run the same one.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn date_period_targets_are_ranges() {
    let backend = make_sqlite_backend();
    date_period_suite::period_targets_are_ranges(&backend, "date-period").await;
}

/// A composite's date component is a point on a Period's start, where the plain
/// date parameter is a range (#1391); `Observation?code-value-date` is the
/// registry composite that admits a Period there.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn date_composite_component_is_a_point_on_a_period() {
    let backend = make_sqlite_backend();
    composite_period_pin::composite_date_component_is_a_point(&backend, "date-composite-period")
        .await;
}
