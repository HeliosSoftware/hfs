//! Integration tests for storage-backed CompartmentDefinitions (#237/#238):
//! seeding the store from the spec bundle so `GET /CompartmentDefinition`
//! discovers the definitions the server resolves compartment membership from.

#![cfg(feature = "sqlite")]

use std::path::PathBuf;

use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::search::{
    seed_spec_compartment_definitions, seed_tenant_conformance,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use serde_json::json;

/// The workspace data directory holding `compartment-definitions-r4.json`.
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

    let first = seed_spec_compartment_definitions(&backend, FhirVersion::R4, &data_dir, "default")
        .await
        .expect("first seed");
    // R4 ships 5 compartments (Device, Encounter, Patient, Practitioner,
    // RelatedPerson); questionnaire is excluded.
    assert_eq!(first.created, 5);
    assert_eq!(first.failed, 0);

    // Discoverable via a storage read (what GET /CompartmentDefinition executes).
    let stored = backend
        .count(&tenant("default"), Some("CompartmentDefinition"))
        .await
        .expect("count seeded");
    assert_eq!(stored, 5);

    let patient = backend
        .read(&tenant("default"), "CompartmentDefinition", "patient")
        .await
        .expect("read patient compartment")
        .expect("patient compartment present");
    assert_eq!(patient.content()["code"], json!("Patient"));

    // A second boot takes the fast path and writes nothing.
    let second = seed_spec_compartment_definitions(&backend, FhirVersion::R4, &data_dir, "default")
        .await
        .expect("second seed");
    assert_eq!(second.created, 0);
    assert_eq!(second.failed, 0);

    // Other tenants are unaffected by a single-tenant seed.
    let other = backend
        .count(&tenant("acme"), Some("CompartmentDefinition"))
        .await
        .expect("count other tenant");
    assert_eq!(other, 0);
}

/// The combined helper seeds both SearchParameters and CompartmentDefinitions
/// for a tenant in one call.
#[tokio::test]
async fn seed_tenant_conformance_seeds_both_sets() {
    let backend = create_backend();
    let data_dir = workspace_data_dir();

    let outcome = seed_tenant_conformance(&backend, FhirVersion::R4, &data_dir, "acme").await;
    assert_eq!(outcome.failed, 0);

    let sp = backend
        .count(&tenant("acme"), Some("SearchParameter"))
        .await
        .expect("count SearchParameter");
    let cd = backend
        .count(&tenant("acme"), Some("CompartmentDefinition"))
        .await
        .expect("count CompartmentDefinition");
    assert!(sp > 1300, "expected the R4 search-param set, got {sp}");
    assert_eq!(cd, 5);
}

/// A CompartmentDefinition already stored under a spec bundle id is reported
/// `existing`, not clobbered.
#[tokio::test]
async fn seeding_reports_existing_and_never_clobbers() {
    let backend = create_backend();
    let data_dir = workspace_data_dir();

    let preexisting = json!({
        "resourceType": "CompartmentDefinition",
        "id": "patient",
        "url": "http://acme.health/fhir/CompartmentDefinition/custom-patient",
        "status": "active",
        "code": "Patient",
        "search": true,
        "resource": []
    });
    backend
        .create(
            &tenant("default"),
            "CompartmentDefinition",
            preexisting,
            FhirVersion::R4,
        )
        .await
        .expect("pre-create under a spec id");

    let outcome = seed_spec_compartment_definitions(&backend, FhirVersion::R4, &data_dir, "default")
        .await
        .expect("seed over a present spec id");
    assert!(
        outcome.existing >= 1,
        "the pre-existing spec id should be reported existing, got {outcome:?}"
    );
    assert_eq!(outcome.failed, 0);

    // The pre-existing resource was left untouched (create never clobbers).
    let read = backend
        .read(&tenant("default"), "CompartmentDefinition", "patient")
        .await
        .expect("read pre-existing")
        .expect("still present");
    assert_eq!(
        read.content()["url"],
        json!("http://acme.health/fhir/CompartmentDefinition/custom-patient")
    );
}

/// With no bundle in the data directory, seeding is a no-op (not an error).
#[tokio::test]
async fn seeding_with_missing_bundle_is_a_noop() {
    let backend = create_backend();
    let empty_dir = tempfile::tempdir().expect("temp data dir");

    let outcome =
        seed_spec_compartment_definitions(&backend, FhirVersion::R4, empty_dir.path(), "default")
            .await
            .expect("seed with no bundle");
    assert_eq!(outcome.created, 0);
    assert_eq!(outcome.failed, 0);
}
