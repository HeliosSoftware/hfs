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
/// Backend-agnostic scenarios for `instant` targets, shared with the
/// PostgreSQL, MongoDB and Elasticsearch test binaries via `#[path]` (#2962).
pub mod date_instant_suite;
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
/// Backend-agnostic scenarios, shared with the PostgreSQL and MongoDB test
/// binaries via `#[path]` (#2509). Elasticsearch does not run this one: it has
/// its own, narrower test, `es_integration_empty_compartment_membership_matches_nothing`
/// in `elasticsearch_tests.rs`.
pub mod empty_compartment_suite;
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
/// Backend-agnostic scenarios for `Range` values of quantity parameters,
/// shared with the PostgreSQL, MongoDB and Elasticsearch test binaries via
/// `#[path]` (#2602).
pub mod quantity_range_suite;
pub mod quantity_tests;
pub mod reference_tests;
/// Backend-agnostic `_revinclude` reference-form scenarios, shared with the
/// PostgreSQL, MongoDB and Elasticsearch test binaries via `#[path]` (#2296).
pub mod revinclude_reference_forms_suite;
/// Backend-agnostic scenarios, shared with the PostgreSQL, MongoDB and
/// Elasticsearch test binaries via `#[path]` (#1711).
pub mod sort_meta_suite;
/// Backend-agnostic scenarios, shared with the PostgreSQL, MongoDB and
/// Elasticsearch test binaries via `#[path]` (#1606).
pub mod sort_missing_suite;
pub mod string_tests;
/// Backend-agnostic scenarios, shared with the PostgreSQL and MongoDB test
/// binaries via `#[path]` (#2603).
pub mod tiebreak_order_suite;
/// Backend-agnostic scenarios, shared with the PostgreSQL, MongoDB and
/// Elasticsearch test binaries via `#[path]` (#1379).
pub mod token_code_system_suite;
pub mod token_tests;
/// Backend-agnostic scenarios, shared with the PostgreSQL test binary via
/// `#[path]` (#2731).
pub mod unfiltered_date_sort_suite;

/// The shared Period table (#1391): a Period is one range, a missing side is
/// unbounded, and every prefix follows the FHIR rules for range targets.
/// PostgreSQL, MongoDB and Elasticsearch run the same one.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn date_period_targets_are_ranges() {
    let backend = make_sqlite_backend();
    date_period_suite::period_targets_are_ranges(&backend, "date-period").await;
}

/// The shared `instant` table (#2962): an `instant` is a point, so a
/// millisecond search finds one written to the second, and a `dateTime` with
/// the same text keeps its one-second range. PostgreSQL, MongoDB and
/// Elasticsearch run the same one.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn date_instant_targets_are_points() {
    let backend = make_sqlite_backend();
    date_instant_suite::instant_targets_are_points(&backend, "date-instant").await;
}

/// The backend's configured FHIR version decides which elements are
/// `instant`s (#2962): R6 makes `DiagnosticReport.issued` a `dateTime`.
/// PostgreSQL, MongoDB and Elasticsearch run the same three.
#[cfg(all(feature = "sqlite", feature = "R6"))]
#[tokio::test]
async fn date_instant_types_follow_an_r6_backend() {
    let backend = make_sqlite_backend_for(FhirVersion::R6);
    date_instant_suite::r6_backend_types_by_r6(&backend, "date-instant-r6").await;
}

/// The R4 control for `date_instant_types_follow_an_r6_backend`.
#[cfg(all(feature = "sqlite", feature = "R4"))]
#[tokio::test]
async fn date_instant_types_follow_an_r4_backend() {
    let backend = make_sqlite_backend_for(FhirVersion::R4);
    date_instant_suite::r4_backend_types_by_r4(&backend, "date-instant-r4").await;
}

/// R5's `ImagingSelection.issued`, an `instant` that R4 does not have.
#[cfg(all(feature = "sqlite", feature = "R5"))]
#[tokio::test]
async fn date_instant_types_follow_an_r5_backend() {
    let backend = make_sqlite_backend_for(FhirVersion::R5);
    date_instant_suite::r5_backend_types_by_r5(&backend, "date-instant-r5").await;
}

/// The shared quantity `Range` table (#2602): a `Range` is one closed
/// interval, a missing bound is open, and every prefix compares against the
/// interval. PostgreSQL, MongoDB and Elasticsearch run the same one.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn quantity_range_targets_are_intervals() {
    let backend = make_sqlite_backend();
    quantity_range_suite::quantity_ranges_are_intervals(&backend, "quantity-range").await;
}

/// A compartment membership with no params or no reference matches nothing,
/// on every search path, where it used to be read as "no compartment" and
/// widened the search to every resource of the type (#2509). PostgreSQL and
/// MongoDB run the same one; Elasticsearch has its own, narrower test.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn empty_compartment_membership_matches_nothing() {
    let backend = make_sqlite_backend();
    empty_compartment_suite::empty_membership_matches_nothing_on_every_path(
        &backend,
        "empty-compartment",
    )
    .await;
}

/// A resource with no value for a sort key sorts after those with one, in
/// either direction and across page boundaries (#1606). PostgreSQL, MongoDB
/// and Elasticsearch run the same one.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sort_missing_values_sort_last() {
    let backend = make_sqlite_backend();
    sort_missing_suite::missing_sort_values_sort_last(&backend, "sort-missing", true).await;
}

/// An unfiltered sort by a date parameter read from the date index returns what
/// the general path returns, page by page and cursor by cursor (#2731).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn unfiltered_date_sort_matches_the_general_path() {
    let backend = make_sqlite_backend();
    unfiltered_date_sort_suite::unfiltered_date_sort_matches_the_general_path(
        &backend,
        "unfiltered-date-sort",
    )
    .await;
}

/// `_sort` on `_tag`, `_security`, `_profile` and `_source` orders by the
/// indexed value (#1711). PostgreSQL, MongoDB and Elasticsearch run the same
/// one.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sort_meta_params_sort_by_value() {
    let backend = make_sqlite_backend();
    sort_meta_suite::meta_params_sort_by_value(&backend, "sort-meta").await;
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

/// Resources sharing one `last_updated` page forward and back without a gap or
/// a repeat (#2603). The in-memory database cannot be reached from outside, so
/// the timestamps are pinned through a second connection onto a file-backed
/// one. PostgreSQL and MongoDB run the same suite.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn tiebreak_tied_timestamps_page_without_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tiebreak.db");
    let config = SqliteBackendConfig {
        data_dir: Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.join("data"))
                .unwrap(),
        ),
        ..Default::default()
    };
    let backend = SqliteBackend::with_config(&path, config).unwrap();
    backend.init_schema().unwrap();
    tiebreak_order_suite::tied_timestamps_page_without_gaps(
        &backend,
        "tiebreak-2603",
        async |tenant_id: &str, resource_type: &str| {
            let conn = rusqlite::Connection::open(&path).unwrap();
            let changed = conn
                .execute(
                    "UPDATE resources SET last_updated = '2001-02-03T04:05:06.789012+00:00' \
                     WHERE tenant_id = ?1 AND resource_type = ?2",
                    [tenant_id, resource_type],
                )
                .unwrap();
            assert!(changed > 0, "no {resource_type} rows to pin");
        },
    )
    .await;
}
