//! Per-entry OperationOutcome issue codes on the batch path (#504).
//!
//! Every error a batch entry could produce was built by one helper that
//! hardcoded `"code": "processing"`, so a scope denial, a missing resource, a
//! malformed entry and an unsupported method were distinguishable only by
//! `response.status` and free-text English. These tests assert the code **on
//! the wire**, which the unit tests in `handlers::batch` cannot: they drive
//! `run_batch` directly and so never exercise the router or
//! `bundle_entry_result_to_json`'s placement of `outcome` under
//! `response.outcome`.
//!
//! A separate file rather than a module appended to `batch_conformance.rs`:
//! that file's tail already has several claimants across the open stack, and
//! the harness below is ~40 lines it duplicates twice internally anyway.

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

async fn seed_patient(backend: &SqliteBackend, id: &str) {
    let patient = json!({ "resourceType": "Patient", "id": id, "active": true });
    backend
        .create(&test_tenant(), "Patient", patient, FhirVersion::R4)
        .await
        .expect("Failed to seed patient");
}

async fn post_batch(server: &TestServer, entries: Vec<Value>) -> Value {
    let response = server
        .post("/")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": entries,
        }))
        .await;
    response.assert_status_ok();
    response.json()
}

/// Each failure class reaches the client with its own issue code.
///
/// One bundle, one entry per reachable class. Before #504 the `code` column
/// below read `processing` for every row, so a client could only tell these
/// apart by parsing `details.text`.
#[tokio::test]
async fn a_batch_entry_reports_the_issue_code_for_its_failure() {
    let (server, _backend) = create_test_server().await;

    // (entry, expected status prefix, expected issue code)
    let cases: Vec<(Value, &str, &str)> = vec![
        (
            json!({ "request": { "method": "GET", "url": "Patient/ghost" } }),
            "404",
            "not-found",
        ),
        (
            // A lowercase verb is invalid instance data: `request.method` is a
            // code with a required binding to `http-verb` (#502).
            json!({
                "request": { "method": "post", "url": "Patient" },
                "resource": { "resourceType": "Patient" }
            }),
            "400",
            "value",
        ),
        (
            json!({ "resource": { "resourceType": "Patient" } }),
            "400",
            "required",
        ),
        (
            json!({ "request": { "url": "Patient/p1" } }),
            "400",
            "required",
        ),
        (
            // Criteria on a POST. #511 resolves URL criteria on PUT and DELETE
            // entries, so what remains here is the form FHIR leaves undefined:
            // a conditional create is expressed through `ifNoneExist`, not the
            // url. `value` — the url is present and unusable for this method.
            json!({
                "request": { "method": "POST", "url": "Patient?identifier=x" },
                "resource": { "resourceType": "Patient" }
            }),
            "400",
            "value",
        ),
        (
            // `ifMatch` names a version of one instance; a conditional entry
            // names no instance until the server resolves it. Both elements are
            // individually well-formed, so the fault is the pairing and
            // `invalid` — the parent of `value` — is as precise as it gets.
            json!({
                "request": {
                    "method": "PUT",
                    "url": "Patient?identifier=x",
                    "ifMatch": "W/\"1\""
                },
                "resource": { "resourceType": "Patient" }
            }),
            "400",
            "invalid",
        ),
        (
            // Carries a `resource` deliberately: without one this is refused by
            // the missing-resource guard first and would never reach the
            // `id.is_empty()` guard this row names.
            json!({
                "request": { "method": "PUT", "url": "Patient" },
                "resource": { "resourceType": "Patient" }
            }),
            "400",
            "value",
        ),
        (
            json!({ "request": { "method": "HEAD", "url": "Patient/p1" } }),
            "405",
            "not-supported",
        ),
        (
            json!({
                "request": { "method": "PATCH", "url": "Patient/p1" },
                "resource": { "resourceType": "Patient" }
            }),
            "501",
            "not-supported",
        ),
    ];

    let entries: Vec<Value> = cases.iter().map(|(entry, _, _)| entry.clone()).collect();
    let body = post_batch(&server, entries).await;
    let responses = body["entry"].as_array().expect("entry array");
    assert_eq!(responses.len(), cases.len());

    for (index, (request, status, code)) in cases.iter().enumerate() {
        let entry = &responses[index];
        let response = &entry["response"];

        assert!(
            response["status"]
                .as_str()
                .unwrap_or_default()
                .starts_with(status),
            "entry {index} ({request}) status: {response}"
        );
        assert_eq!(
            response["outcome"]["issue"][0]["code"], *code,
            "entry {index} ({request}) issue code: {response}"
        );
        // #504 changes the code, not the placement: the outcome stays under
        // `response.outcome` and never becomes the entry resource.
        assert_eq!(
            response["outcome"]["resourceType"], "OperationOutcome",
            "entry {index}: {response}"
        );
        assert!(
            entry.get("resource").is_none(),
            "entry {index} must not carry the outcome as its resource: {entry}"
        );
    }
}

/// #504's stated impact, as an executable claim: the same failure, described
/// identically, whether it arrives at the resource endpoint or inside a Bundle
/// entry.
///
/// Both bodies are now produced by `RestError::client_outcome`, so this is a
/// property of the code rather than a coincidence two mappings happen to share.
/// Before #504 the entry said `processing` with the text "Resource not found"
/// while the endpoint said `not-found` with the resource named in its prose
/// — a different code *and* different prose for one condition.
#[tokio::test]
async fn a_missing_resource_is_described_identically_by_both_surfaces() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1").await;

    let single: Value = server
        .get("/Patient/ghost")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await
        .json();

    let batch = post_batch(
        &server,
        vec![json!({ "request": { "method": "GET", "url": "Patient/ghost" } })],
    )
    .await;
    let response = &batch["entry"][0]["response"];

    assert!(
        response["status"]
            .as_str()
            .unwrap_or_default()
            .starts_with("404"),
        "entry status: {response}"
    );

    // Field by field. `details.text` is included deliberately: an issue code
    // that agrees while the prose diverges is half a fix.
    let endpoint_issue = &single["issue"][0];
    let entry_issue = &response["outcome"]["issue"][0];
    assert_eq!(entry_issue["severity"], endpoint_issue["severity"]);
    assert_eq!(entry_issue["code"], endpoint_issue["code"]);
    assert_eq!(
        entry_issue["details"]["text"],
        endpoint_issue["details"]["text"]
    );

    // Pinned literally, not just to each other: a regression that made *both*
    // surfaces say `processing` would satisfy every assertion above.
    assert_eq!(entry_issue["code"], "not-found");
    assert_eq!(
        entry_issue["details"]["text"],
        "Could not find the resource 'Patient/ghost'."
    );
}

// =============================================================================
// Search entries (#478)
// =============================================================================
//
// #478 added two failure sites — one per arm — and covered neither: all five of
// its tests are happy-path `200 OK`. Both were written with the same
// `let (status, _, details) = e.client_response()` code-discard #504 removed
// everywhere else, so without these a search entry would have been the one path
// left answering `processing`.

/// Post `bundle` and return the parsed body, asserting HTTP 200.
async fn post_bundle_ok(server: &TestServer, bundle: Value) -> Value {
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

/// A search entry that fails carries the issue code for *why* it failed.
#[tokio::test]
async fn a_failed_search_entry_reports_its_issue_code() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1").await;

    // (entry url, expected status prefix, expected issue code)
    let cases: Vec<(&str, &str, &str)> = vec![
        // `_query` is a known-but-unimplemented control parameter.
        ("Patient?_query=byName", "400", "invalid"),
        // `:not-in` needs negated value-set filtering no backend implements.
        (
            "Patient?code:not-in=http://example.org/vs",
            "501",
            "not-supported",
        ),
    ];

    let body = post_batch(
        &server,
        cases
            .iter()
            .map(|(url, _, _)| json!({ "request": { "method": "GET", "url": url } }))
            .collect(),
    )
    .await;

    for (index, (url, status, code)) in cases.iter().enumerate() {
        let response = &body["entry"][index]["response"];
        assert!(
            response["status"]
                .as_str()
                .unwrap_or_default()
                .starts_with(status),
            "entry {index} ({url}) status: {response}"
        );
        assert_eq!(
            response["outcome"]["issue"][0]["code"], *code,
            "entry {index} ({url}) issue code: {response}"
        );
    }
}

/// The transaction arm's search loop is the **first reachable** per-entry
/// outcome on that arm.
///
/// #504 could accurately say none existed: the backends discard an entry result
/// at their `status >= 400` guard, so a transaction never surfaced a per-entry
/// outcome. #478's search loop bypasses the backend executor entirely and
/// surfaces the failure as that entry's own outcome — deliberately, since a
/// search failure cannot roll back writes that already committed. So this is a
/// per-entry code on the transaction arm, and it must not be `processing`.
#[tokio::test]
async fn a_failed_transaction_search_entry_reports_its_issue_code() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1").await;

    let body = post_bundle_ok(
        &server,
        json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [{
                "request": { "method": "GET", "url": "Patient?_query=byName" }
            }]
        }),
    )
    .await;

    assert_eq!(body["type"], "transaction-response", "{body}");
    let response = &body["entry"][0]["response"];
    assert!(
        response["status"]
            .as_str()
            .unwrap_or_default()
            .starts_with("400"),
        "entry status: {response}"
    );
    assert_eq!(
        response["outcome"]["issue"][0]["code"], "invalid",
        "{response}"
    );
}

/// The same bad search, described identically by both surfaces — the parity
/// claim extended to the arm #478 added.
#[tokio::test]
async fn a_failed_search_is_described_identically_by_both_surfaces() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1").await;

    let single: Value = server
        .get("/Patient?_query=byName")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await
        .json();

    let batch = post_batch(
        &server,
        vec![json!({ "request": { "method": "GET", "url": "Patient?_query=byName" } })],
    )
    .await;
    let entry_issue = &batch["entry"][0]["response"]["outcome"]["issue"][0];
    let endpoint_issue = &single["issue"][0];

    assert_eq!(entry_issue["severity"], endpoint_issue["severity"]);
    assert_eq!(entry_issue["code"], endpoint_issue["code"]);
    assert_eq!(
        entry_issue["details"]["text"],
        endpoint_issue["details"]["text"]
    );

    // Pinned literally: `processing == processing` would satisfy the equalities.
    assert_eq!(entry_issue["code"], "invalid");
}
