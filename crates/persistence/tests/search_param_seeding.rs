//! Integration tests for storage-backed SearchParameters (#235): seeding the
//! store from the spec bundle, and refreshing the in-memory registry from
//! storage so cluster-mates' writes become visible.

#![cfg(feature = "sqlite")]

use std::path::PathBuf;

use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::search::seed_spec_search_parameters;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use serde_json::json;

/// The workspace data directory holding `search-parameters-r4.json`.
fn workspace_data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn create_backend() -> SqliteBackend {
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(workspace_data_dir()),
            ..Default::default()
        },
    )
    .expect("create in-memory SQLite backend");
    backend.init_schema().expect("init schema");
    backend
}

fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

#[tokio::test]
async fn seeding_is_idempotent_and_discoverable() {
    let backend = create_backend();
    let data_dir = workspace_data_dir();

    let first = seed_spec_search_parameters(&backend, FhirVersion::R4, &data_dir, "default")
        .await
        .expect("first seed");
    assert!(
        first.created > 1300,
        "expected the R4 spec set to seed, created only {}",
        first.created
    );
    assert_eq!(first.failed, 0);

    // The point of the issue: the spec parameters are now discoverable via a
    // storage read (what GET /SearchParameter executes) in the default tenant.
    let stored = backend
        .count(&tenant("default"), Some("SearchParameter"))
        .await
        .expect("count seeded");
    assert_eq!(stored as usize, first.created);

    // A second boot takes the fast path and writes nothing.
    let second = seed_spec_search_parameters(&backend, FhirVersion::R4, &data_dir, "default")
        .await
        .expect("second seed");
    assert_eq!(second.created, 0);
    assert_eq!(second.failed, 0);

    // Other tenants are unaffected (seeding is default-tenant only).
    let other = backend
        .count(&tenant("acme"), Some("SearchParameter"))
        .await
        .expect("count other tenant");
    assert_eq!(other, 0);
}

/// A partial set (e.g. a user-POSTed parameter predating the seeding feature)
/// is completed without clobbering what exists.
#[tokio::test]
async fn seeding_completes_a_partial_set_without_clobbering() {
    let backend = create_backend();
    let data_dir = workspace_data_dir();

    let custom = json!({
        "resourceType": "SearchParameter",
        "id": "acme-preexisting",
        "url": "http://acme.health/fhir/SearchParameter/preexisting",
        "name": "preexisting",
        "status": "active",
        "code": "preexisting",
        "base": ["Patient"],
        "type": "string",
        "expression": "Patient.name.given"
    });
    backend
        .create(
            &tenant("default"),
            "SearchParameter",
            custom,
            FhirVersion::R4,
        )
        .await
        .expect("pre-existing custom parameter");

    let outcome = seed_spec_search_parameters(&backend, FhirVersion::R4, &data_dir, "default")
        .await
        .expect("seed over partial set");
    assert!(outcome.created > 1300);
    assert_eq!(outcome.failed, 0);

    // The custom parameter survived untouched.
    let read = backend
        .read(&tenant("default"), "SearchParameter", "acme-preexisting")
        .await
        .expect("read custom")
        .expect("custom parameter still present");
    assert_eq!(
        read.content()["url"],
        json!("http://acme.health/fhir/SearchParameter/preexisting")
    );
}

/// A resource already stored under a spec bundle id is reported `existing`,
/// not re-created or clobbered — the idempotency the seeder relies on.
#[tokio::test]
async fn seeding_reports_existing_for_present_spec_ids() {
    let backend = create_backend();
    let data_dir = workspace_data_dir();

    // Pre-create a resource under a spec bundle id (`Patient-name`) but with a
    // distinct body, so the seed pass hits the `AlreadyExists` branch for it.
    let preexisting = json!({
        "resourceType": "SearchParameter",
        "id": "Patient-name",
        "url": "http://acme.health/fhir/SearchParameter/preexisting-patient-name",
        "name": "preexisting",
        "status": "active",
        "code": "name",
        "base": ["Patient"],
        "type": "string",
        "expression": "Patient.name"
    });
    backend
        .create(
            &tenant("default"),
            "SearchParameter",
            preexisting,
            FhirVersion::R4,
        )
        .await
        .expect("pre-create under a spec id");

    let outcome = seed_spec_search_parameters(&backend, FhirVersion::R4, &data_dir, "default")
        .await
        .expect("seed over a present spec id");
    assert!(
        outcome.existing >= 1,
        "the pre-existing spec id should be reported existing, got {outcome:?}"
    );
    assert_eq!(outcome.failed, 0);

    // The pre-existing resource was left untouched (create never clobbers).
    let read = backend
        .read(&tenant("default"), "SearchParameter", "Patient-name")
        .await
        .expect("read pre-existing")
        .expect("still present");
    assert_eq!(
        read.content()["url"],
        json!("http://acme.health/fhir/SearchParameter/preexisting-patient-name")
    );
}

/// With no spec bundle in the data directory, seeding still writes the embedded
/// fallback parameters (a supported minimal deployment).
#[tokio::test]
async fn seeding_with_missing_spec_bundle_seeds_only_fallbacks() {
    let backend = create_backend();
    let empty_dir = tempfile::tempdir().expect("temp data dir");

    let outcome =
        seed_spec_search_parameters(&backend, FhirVersion::R4, empty_dir.path(), "default")
            .await
            .expect("seed fallbacks");
    // Only the handful of embedded fallbacks are seeded — no spec set.
    assert!(
        outcome.created > 0 && outcome.created <= 9,
        "expected only embedded fallbacks, got {outcome:?}"
    );
    assert_eq!(outcome.failed, 0);
}

/// The TTL-cache contract: a SearchParameter present in storage (written by
/// this node or a cluster-mate) enters search resolution on a refresh, and a
/// deleted one leaves it.
#[tokio::test]
async fn refresh_rebuilds_stored_parameters_from_storage() {
    use helios_persistence::search::registry::SearchParameterSource;

    let backend = create_backend();
    let registry = backend.search_registry();

    let nickname = json!({
        "resourceType": "SearchParameter",
        "id": "acme-nickname",
        "url": "http://acme.health/fhir/SearchParameter/patient-nickname",
        "name": "nickname",
        "status": "active",
        "code": "nickname",
        "base": ["Patient"],
        "type": "string",
        "expression": "Patient.name.where(use='nickname').given"
    });
    backend
        .create(
            &tenant("default"),
            "SearchParameter",
            nickname,
            FhirVersion::R4,
        )
        .await
        .expect("store the parameter");

    // Simulate a freshly booted cluster-mate: its storage holds the parameter
    // but its registry has never seen it (drop the write-hook registration).
    registry
        .write()
        .unregister_source(SearchParameterSource::Stored);
    assert!(
        registry.read().get_param("Patient", "nickname").is_none(),
        "not visible before the refresh"
    );

    let stored = backend.refresh_stored_search_parameters().expect("refresh");
    assert_eq!(stored, 1);
    assert!(
        registry.read().get_param("Patient", "nickname").is_some(),
        "visible after the refresh"
    );

    // Delete it from storage; the next refresh drops it from resolution.
    backend
        .delete(&tenant("default"), "SearchParameter", "acme-nickname")
        .await
        .expect("delete the parameter");
    let stored = backend
        .refresh_stored_search_parameters()
        .expect("refresh after delete");
    assert_eq!(stored, 0);
    assert!(
        registry.read().get_param("Patient", "nickname").is_none(),
        "gone after the refresh"
    );
}
