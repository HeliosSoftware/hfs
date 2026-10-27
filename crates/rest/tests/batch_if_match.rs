//! `ifMatch` preconditions on the REST batch path.
//!
//! #311 fixed `If-Match` list handling across the bundle arms, but the batch
//! half of that fix landed in `BundleProvider::process_batch`, which no
//! deployment ever calls — the REST layer runs its own entry loop. Over HTTP a
//! batch `PUT`/`DELETE` carrying `ifMatch` was therefore still unconditional:
//! stale tag, 200 OK, lost update.
//!
//! These tests exercise the gate where the server actually runs it: through the
//! router, over the wire, against a real backend. They replace the nine batch
//! scenarios in `helios-persistence`'s `if_match_suite`, which drove the
//! unreachable copy.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

async fn create_test_server() -> (TestServer, Arc<SqliteBackend>) {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let backend_config = SqliteBackendConfig {
        data_dir: Some(data_dir),
        ..Default::default()
    };
    let backend = SqliteBackend::with_config(":memory:", backend_config)
        .expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to init schema");
    let backend = Arc::new(backend);

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    };

    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let server = TestServer::new(app).expect("Failed to create test server");

    (server, backend)
}

fn test_tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
}

/// Seeds a Patient and returns its version id (`"1"` for a fresh create).
async fn seed_patient(backend: &SqliteBackend, id: &str, family: &str) -> String {
    let tenant = test_tenant();
    let patient = json!({
        "resourceType": "Patient",
        "id": id,
        "name": [{"family": family}],
        "active": true
    });
    let stored = backend
        .create(&tenant, "Patient", patient, FhirVersion::R4)
        .await
        .expect("Failed to seed patient");
    stored.version_id().to_string()
}

/// Posts a batch Bundle. The bundle-level status is asserted 200 on every call:
/// a failed precondition is a per-*entry* outcome and must never fail the batch.
async fn post_batch(server: &TestServer, bundle: Value) -> Value {
    let response = server
        .post("/")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&bundle)
        .await;
    response.assert_status_ok();
    response.json()
}

fn batch(entries: Vec<Value>) -> Value {
    json!({
        "resourceType": "Bundle",
        "type": "batch",
        "entry": entries,
    })
}

/// A `PUT Patient/{id}` entry, optionally carrying an `ifMatch`.
fn put_entry(id: &str, family: &str, if_match: Option<&str>) -> Value {
    let mut request = json!({ "method": "PUT", "url": format!("Patient/{id}") });
    if let Some(if_match) = if_match {
        request["ifMatch"] = json!(if_match);
    }
    json!({
        "request": request,
        "resource": {
            "resourceType": "Patient",
            "id": id,
            "name": [{"family": family}],
            "active": true
        }
    })
}

/// A `DELETE Patient/{id}` entry, optionally carrying an `ifMatch`.
fn delete_entry(id: &str, if_match: Option<&str>) -> Value {
    let mut request = json!({ "method": "DELETE", "url": format!("Patient/{id}") });
    if let Some(if_match) = if_match {
        request["ifMatch"] = json!(if_match);
    }
    json!({ "request": request })
}

fn entry_status(body: &Value, index: usize) -> String {
    body["entry"][index]["response"]["status"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn assert_precondition_failed(body: &Value, index: usize) {
    let status = entry_status(body, index);
    assert!(
        status.starts_with("412"),
        "entry {index} should have failed its precondition, got {status}"
    );

    // The gate emits a properly coded OperationOutcome, in `response.outcome`
    // rather than as the entry resource.
    let code = body["entry"][index]["response"]["outcome"]["issue"][0]["code"].as_str();
    assert_eq!(code, Some("conflict"), "entry {index} outcome code");
}

async fn read_family(server: &TestServer, id: &str) -> Option<String> {
    let response = server
        .get(&format!("/Patient/{id}"))
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await;
    if !response.status_code().is_success() {
        return None;
    }
    let body: Value = response.json();
    body["name"][0]["family"].as_str().map(str::to_string)
}

// =============================================================================
// PUT
// =============================================================================

/// The regression #311 left on the wire: a stale tag must not overwrite.
#[tokio::test]
async fn put_with_stale_if_match_is_rejected_and_leaves_the_resource_unchanged() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1", "Original").await;

    let body = post_batch(
        &server,
        batch(vec![put_entry("p1", "Overwritten", Some("W/\"99\""))]),
    )
    .await;

    assert_precondition_failed(&body, 0);
    assert_eq!(
        read_family(&server, "p1").await.as_deref(),
        Some("Original"),
        "a rejected precondition must not have written"
    );
}

#[tokio::test]
async fn put_with_matching_if_match_succeeds() {
    let (server, backend) = create_test_server().await;
    let version = seed_patient(&backend, "p1", "Original").await;

    let body = post_batch(
        &server,
        batch(vec![put_entry(
            "p1",
            "Updated",
            Some(&format!("W/\"{version}\"")),
        )]),
    )
    .await;

    assert!(
        entry_status(&body, 0).starts_with("200"),
        "got {}",
        entry_status(&body, 0)
    );
    assert_eq!(read_family(&server, "p1").await.as_deref(), Some("Updated"));
}

/// The heart of #311: `If-Match` is a comma-separated list, satisfied when *any*
/// member matches. Comparing the field value as one opaque string made a
/// multi-valued precondition permanently unsatisfiable.
#[tokio::test]
async fn put_with_a_multi_valued_if_match_succeeds_when_any_tag_matches() {
    let (server, backend) = create_test_server().await;
    let version = seed_patient(&backend, "p1", "Original").await;

    let body = post_batch(
        &server,
        batch(vec![put_entry(
            "p1",
            "Updated",
            Some(&format!("W/\"7\", W/\"{version}\", W/\"9\"")),
        )]),
    )
    .await;

    assert!(
        entry_status(&body, 0).starts_with("200"),
        "a list containing the current version must match, got {}",
        entry_status(&body, 0)
    );
    assert_eq!(read_family(&server, "p1").await.as_deref(), Some("Updated"));
}

/// `*` means "any current representation", so it must succeed against one that
/// exists...
#[tokio::test]
async fn put_with_star_if_match_succeeds_against_an_existing_resource() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1", "Original").await;

    let body = post_batch(&server, batch(vec![put_entry("p1", "Updated", Some("*"))])).await;

    assert!(entry_status(&body, 0).starts_with("200"));
    assert_eq!(read_family(&server, "p1").await.as_deref(), Some("Updated"));
}

/// ...and fail against one that does not, rather than creating it.
#[tokio::test]
async fn put_with_star_if_match_is_rejected_when_the_resource_is_absent() {
    let (server, _backend) = create_test_server().await;

    let body = post_batch(&server, batch(vec![put_entry("ghost", "New", Some("*"))])).await;

    assert_precondition_failed(&body, 0);
    assert_eq!(
        read_family(&server, "ghost").await,
        None,
        "a precondition against an absent resource must not create it"
    );
}

#[tokio::test]
async fn put_with_if_match_on_an_absent_resource_is_rejected_not_created() {
    let (server, _backend) = create_test_server().await;

    let body = post_batch(
        &server,
        batch(vec![put_entry("ghost", "New", Some("W/\"1\""))]),
    )
    .await;

    assert_precondition_failed(&body, 0);
    assert_eq!(read_family(&server, "ghost").await, None);
}

/// A deleted resource has no current representation, so any precondition —
/// including `*` — fails rather than resurrecting it unconditionally.
#[tokio::test]
async fn put_with_if_match_on_a_deleted_resource_is_rejected() {
    let (server, backend) = create_test_server().await;
    let version = seed_patient(&backend, "p1", "Original").await;

    let deleted = post_batch(&server, batch(vec![delete_entry("p1", None)])).await;
    assert!(entry_status(&deleted, 0).starts_with("204"));

    let body = post_batch(
        &server,
        batch(vec![put_entry(
            "p1",
            "Resurrected",
            Some(&format!("W/\"{version}\"")),
        )]),
    )
    .await;

    assert_precondition_failed(&body, 0);
}

/// A precondition the server cannot evaluate must fail closed. An unconditional
/// write would be the one outcome the client definitely did not ask for.
#[tokio::test]
async fn malformed_if_match_fails_closed() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1", "Original").await;

    // An empty field value: a precondition was sent and cannot be satisfied.
    // And `*` mixed with tags, which RFC 9110 §13.1.1 does not permit.
    let body = post_batch(
        &server,
        batch(vec![
            put_entry("p1", "Empty", Some("")),
            put_entry("p1", "Mixed", Some("*, W/\"1\"")),
        ]),
    )
    .await;

    assert_precondition_failed(&body, 0);
    assert_precondition_failed(&body, 1);
    assert_eq!(
        read_family(&server, "p1").await.as_deref(),
        Some("Original"),
        "neither malformed entry may write"
    );
}

// =============================================================================
// DELETE
// =============================================================================

/// `ifMatch` on DELETE was ignored outright, so a client asking to delete only
/// the version it had reviewed could destroy a concurrent amendment with no 412.
#[tokio::test]
async fn delete_with_stale_if_match_is_rejected_and_the_resource_survives() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1", "Original").await;

    let body = post_batch(&server, batch(vec![delete_entry("p1", Some("W/\"99\""))])).await;

    assert_precondition_failed(&body, 0);
    assert_eq!(
        read_family(&server, "p1").await.as_deref(),
        Some("Original"),
        "a rejected DELETE precondition must not have deleted"
    );
}

#[tokio::test]
async fn delete_with_matching_if_match_succeeds() {
    let (server, backend) = create_test_server().await;
    let version = seed_patient(&backend, "p1", "Original").await;

    let body = post_batch(
        &server,
        batch(vec![delete_entry("p1", Some(&format!("W/\"{version}\"")))]),
    )
    .await;

    assert!(
        entry_status(&body, 0).starts_with("204"),
        "got {}",
        entry_status(&body, 0)
    );
    assert_eq!(read_family(&server, "p1").await, None);
}

#[tokio::test]
async fn delete_with_if_match_on_a_deleted_resource_is_rejected() {
    let (server, backend) = create_test_server().await;
    let version = seed_patient(&backend, "p1", "Original").await;

    let deleted = post_batch(&server, batch(vec![delete_entry("p1", None)])).await;
    assert!(entry_status(&deleted, 0).starts_with("204"));

    let body = post_batch(
        &server,
        batch(vec![delete_entry("p1", Some(&format!("W/\"{version}\"")))]),
    )
    .await;

    assert_precondition_failed(&body, 0);
}

// =============================================================================
// Scope of the change
// =============================================================================

/// Entries that send no precondition keep behaving exactly as before — and pay
/// nothing for the feature, since the gate skips the read entirely.
#[tokio::test]
async fn entries_without_if_match_are_unconditional_as_before() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1", "Original").await;

    let body = post_batch(&server, batch(vec![put_entry("p1", "Updated", None)])).await;

    assert!(entry_status(&body, 0).starts_with("200"));
    assert_eq!(read_family(&server, "p1").await.as_deref(), Some("Updated"));
}

/// A failed precondition is one entry's outcome, not the bundle's: batch
/// entries are independent, and the bundle still returns HTTP 200.
#[tokio::test]
async fn a_failed_precondition_does_not_affect_the_other_entries() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1", "Original").await;
    seed_patient(&backend, "p2", "Second").await;

    let body = post_batch(
        &server,
        batch(vec![
            put_entry("p1", "Overwritten", Some("W/\"99\"")),
            put_entry("p2", "Updated", None),
        ]),
    )
    .await;

    assert_precondition_failed(&body, 0);
    assert!(entry_status(&body, 1).starts_with("200"));
    assert_eq!(
        read_family(&server, "p1").await.as_deref(),
        Some("Original")
    );
    assert_eq!(read_family(&server, "p2").await.as_deref(), Some("Updated"));
}
