//! Bundle entries whose `request.url` names a FHIR operation (#868, step 1).
//!
//! An entry URL such as `ValueSet/$lookup`, `Patient/123/$everything` or
//! `$diff` names an *operation*, which a Bundle entry cannot run: the handlers
//! behind those routes are not dispatched from `batch.rs`. What was wrong was
//! how the entry was refused, and for `GET` that it was not refused at all:
//!
//! - `POST ValueSet/$lookup` took its last segment for a resource type, so the
//!   entry failed as "Resource type '$lookup' is not supported" - the operation
//!   called a resource type.
//! - `GET Patient/123/$everything` was read as `Patient/123`: the URL parser
//!   keeps only the first two segments of a GET address, so the entry ran as a
//!   plain read and answered `200` with the Patient, with no sign the operation
//!   had never run. `GET ValueSet/$expand` was read as the literal id
//!   `$expand` and answered `404`.
//!
//! These tests drive the real router and assert what a client sees on the wire:
//! each such entry is refused as what it is, `501` with issue code
//! `not-supported`, in a batch as that entry's own outcome and in a transaction
//! as the whole bundle's, and a refused entry has no effect.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
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

/// Distinctive enough that finding it in a response entry can only mean the
/// seeded Patient was returned.
const SEEDED_FAMILY: &str = "Zyxwvutsrq";

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
    let patient = json!({
        "resourceType": "Patient",
        "id": id,
        "name": [{ "family": SEEDED_FAMILY }],
        "active": true
    });
    backend
        .create(&test_tenant(), "Patient", patient, FhirVersion::R4)
        .await
        .expect("Failed to seed patient");
}

async fn patient_count(backend: &SqliteBackend) -> u64 {
    backend
        .count(&test_tenant(), Some("Patient"))
        .await
        .expect("count failed")
}

fn bundle_of(bundle_type: &str, entries: Vec<Value>) -> Value {
    json!({ "resourceType": "Bundle", "type": bundle_type, "entry": entries })
}

async fn post_bundle(server: &TestServer, bundle: Value) -> axum_test::TestResponse {
    server
        .post("/")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&bundle)
        .await
}

/// Posts a batch and returns its `entry` array.
async fn post_batch(server: &TestServer, entries: Vec<Value>) -> Vec<Value> {
    let response = post_bundle(server, bundle_of("batch", entries)).await;
    response.assert_status_ok();
    let body: Value = response.json();
    body["entry"].as_array().expect("entry array").clone()
}

fn issue_text(outcome: &Value) -> String {
    outcome["issue"][0]["details"]["text"]
        .as_str()
        .or_else(|| outcome["issue"][0]["diagnostics"].as_str())
        .unwrap_or_default()
        .to_string()
}

/// The refusal a client reads for an operation entry, whichever arm it came
/// from: what it says (`not-supported`), that it names the operation, that it
/// says the fault is the Bundle entry and not a resource type.
fn assert_operation_refusal_text(outcome: &Value, operation: &str) {
    assert_eq!(
        outcome["issue"][0]["code"], "not-supported",
        "issue code: {outcome}"
    );
    let text = issue_text(outcome);
    assert!(
        text.contains(operation),
        "the refusal must name {operation}: {text}"
    );
    assert!(
        text.contains("not supported in Bundle entries"),
        "the refusal must say operations are not supported in Bundle entries: {text}"
    );
    assert!(
        !text.to_lowercase().contains("resource type"),
        "an operation must not be called a resource type: {text}"
    );
}

/// A batch response entry that is the honest refusal of `operation`: `501`,
/// `not-supported`, naming the operation, carrying no resource.
fn assert_batch_entry_refused(entry: &Value, operation: &str) {
    assert_eq!(
        entry["response"]["status"], "501 Not Implemented",
        "entry: {entry}"
    );
    assert_operation_refusal_text(&entry["response"]["outcome"], operation);
    assert!(
        entry.get("resource").is_none(),
        "a refused operation entry must not carry a resource: {entry}"
    );
}

// =============================================================================
// Batch
// =============================================================================

/// The silent half: before the fix this entry answered `200` with the Patient,
/// because only the first two segments of a GET address were read.
#[tokio::test]
async fn batch_get_instance_operation_is_refused_not_read_as_the_instance() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1").await;

    let entries = post_batch(
        &server,
        vec![json!({ "request": { "method": "GET", "url": "Patient/p1/$everything" } })],
    )
    .await;

    assert_eq!(entries.len(), 1);
    assert_batch_entry_refused(&entries[0], "$everything");
    assert!(
        !entries[0].to_string().contains(SEEDED_FAMILY),
        "the Patient must not have been read: {}",
        entries[0]
    );
}

/// Before the fix: a read of the literal id `$expand`, answering `404`.
#[tokio::test]
async fn batch_get_type_operation_is_refused_not_read_as_an_id() {
    let (server, _backend) = create_test_server().await;

    let entries = post_batch(
        &server,
        vec![json!({
            "request": { "method": "GET", "url": "ValueSet/$expand?url=http://example.org/vs" }
        })],
    )
    .await;

    assert_batch_entry_refused(&entries[0], "$expand");
}

/// Before the fix: `Resource type '$lookup' is not supported for FHIR R4`.
#[tokio::test]
async fn batch_post_type_operation_is_not_called_a_resource_type() {
    let (server, _backend) = create_test_server().await;

    let entries = post_batch(
        &server,
        vec![json!({
            "request": { "method": "POST", "url": "ValueSet/$lookup" },
            "resource": {
                "resourceType": "Parameters",
                "parameter": [
                    { "name": "system", "valueUri": "http://loinc.org" },
                    { "name": "code", "valueCode": "1963-8" }
                ]
            }
        })],
    )
    .await;

    assert_batch_entry_refused(&entries[0], "$lookup");
}

/// A system-level operation: no type segment at all.
#[tokio::test]
async fn batch_post_system_operation_is_refused() {
    let (server, _backend) = create_test_server().await;

    let entries = post_batch(
        &server,
        vec![json!({
            "request": { "method": "POST", "url": "$diff" },
            "resource": { "resourceType": "Parameters" }
        })],
    )
    .await;

    assert_batch_entry_refused(&entries[0], "$diff");
}

/// Before the fix: `Observation/$lastn` was read as the id `$lastn`: `404`.
#[tokio::test]
async fn batch_get_operation_with_a_query_is_refused() {
    let (server, _backend) = create_test_server().await;

    let entries = post_batch(
        &server,
        vec![json!({ "request": { "method": "GET", "url": "Observation/$lastn?max=3" } })],
    )
    .await;

    assert_batch_entry_refused(&entries[0], "$lastn");
}

/// A batch's entries are independent: the refused one is its own outcome and
/// its normal neighbours still run, before it and after it.
#[tokio::test]
async fn batch_operation_entry_does_not_stop_its_siblings() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1").await;

    let entries = post_batch(
        &server,
        vec![
            json!({
                "request": { "method": "POST", "url": "Patient" },
                "resource": { "resourceType": "Patient", "name": [{ "family": "Before" }] }
            }),
            json!({ "request": { "method": "GET", "url": "Patient/p1/$everything" } }),
            json!({
                "request": { "method": "POST", "url": "Patient" },
                "resource": { "resourceType": "Patient", "name": [{ "family": "After" }] }
            }),
        ],
    )
    .await;

    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0]["response"]["status"], "201 Created");
    assert_batch_entry_refused(&entries[1], "$everything");
    assert_eq!(entries[2]["response"]["status"], "201 Created");
    assert_eq!(
        patient_count(&backend).await,
        3,
        "seed plus the two creates"
    );
}

/// "On any method": a mutation entry naming an operation is refused as an
/// operation, whatever its verb, and nothing it names is written or deleted.
#[tokio::test]
async fn batch_operation_url_is_refused_on_every_method() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1").await;

    let entries = post_batch(
        &server,
        vec![
            json!({ "request": { "method": "GET", "url": "Patient/p1/$everything" } }),
            json!({
                "request": { "method": "POST", "url": "Patient/p1/$validate" },
                "resource": { "resourceType": "Parameters" }
            }),
            json!({
                "request": { "method": "PUT", "url": "Patient/p1/$purge" },
                "resource": { "resourceType": "Patient", "id": "p1" }
            }),
            json!({
                "request": { "method": "PATCH", "url": "Patient/p1/$purge" },
                "resource": { "resourceType": "Parameters" }
            }),
            json!({ "request": { "method": "DELETE", "url": "Patient/p1/$expunge" } }),
        ],
    )
    .await;

    assert_eq!(entries.len(), 5);
    for (entry, operation) in
        entries
            .iter()
            .zip(["$everything", "$validate", "$purge", "$purge", "$expunge"])
    {
        assert_batch_entry_refused(entry, operation);
    }

    // The DELETE was refused rather than read as `p1/$expunge`, and the PUT and
    // PATCH did not rewrite the Patient.
    let read = server
        .get("/Patient/p1")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await;
    read.assert_status_ok();
    let patient: Value = read.json();
    assert_eq!(patient["name"][0]["family"], SEEDED_FAMILY);
    assert_eq!(patient["meta"]["versionId"], "1", "{patient}");
}

/// An absolute URL, and a percent-encoded `$`, are the same operation.
#[tokio::test]
async fn batch_absolute_and_percent_encoded_operation_urls_are_refused() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1").await;

    let entries = post_batch(
        &server,
        vec![
            json!({
                "request": {
                    "method": "GET",
                    "url": "http://localhost:8080/Patient/p1/$everything"
                }
            }),
            json!({ "request": { "method": "GET", "url": "Patient/p1/%24everything" } }),
            json!({
                "request": { "method": "POST", "url": "http://localhost:8080/ValueSet/$lookup" },
                "resource": { "resourceType": "Parameters" }
            }),
        ],
    )
    .await;

    assert_batch_entry_refused(&entries[0], "$everything");
    assert_batch_entry_refused(&entries[1], "$everything");
    assert_batch_entry_refused(&entries[2], "$lookup");
}

/// A `$` before the last segment is no operation URL and no valid FHIR URL: it
/// is refused as malformed (`400` `value`), as a malformed entry URL is.
#[tokio::test]
async fn batch_dollar_segment_before_the_last_is_a_malformed_url() {
    let (server, _backend) = create_test_server().await;

    let entries = post_batch(
        &server,
        vec![
            json!({ "request": { "method": "GET", "url": "$everything/Patient" } }),
            json!({ "request": { "method": "GET", "url": "Patient/$everything/p1" } }),
        ],
    )
    .await;

    for entry in &entries {
        assert_eq!(
            entry["response"]["status"], "400 Bad Request",
            "entry: {entry}"
        );
        assert_eq!(
            entry["response"]["outcome"]["issue"][0]["code"], "value",
            "entry: {entry}"
        );
        let text = issue_text(&entry["response"]["outcome"]);
        assert!(text.contains("$everything"), "names the segment: {text}");
    }
}

/// Not a regression of the fix: a `$` in the *query* is a search value, not an
/// operation, and a search entry carrying one still runs as a search.
#[tokio::test]
async fn batch_dollar_in_a_query_value_is_still_a_search() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1").await;

    let entries = post_batch(
        &server,
        vec![json!({ "request": { "method": "GET", "url": "Patient?family=%24nobody" } })],
    )
    .await;

    assert_eq!(entries[0]["response"]["status"], "200 OK");
    assert_eq!(entries[0]["resource"]["resourceType"], "Bundle");
    assert_eq!(entries[0]["resource"]["type"], "searchset");
}

// =============================================================================
// Transaction
// =============================================================================

/// One operation entry fails the whole transaction, as any refused entry does:
/// nothing is written, and the refusal names the entry the *client* numbered.
///
/// The operation entry sits at client index 2, while the order a transaction
/// runs entries in (DELETE, POST, PUT/PATCH, GET) would put it at position 1,
/// so a message naming "entry 2" is the client's number and not the sorted one.
#[tokio::test]
async fn transaction_with_an_operation_entry_fails_whole_and_writes_nothing() {
    let (server, backend) = create_test_server().await;

    let response = post_bundle(
        &server,
        bundle_of(
            "transaction",
            vec![
                json!({ "request": { "method": "GET", "url": "Patient?family=Nobody" } }),
                json!({
                    "request": { "method": "POST", "url": "Patient" },
                    "resource": { "resourceType": "Patient", "name": [{ "family": "Created" }] }
                }),
                json!({
                    "request": { "method": "POST", "url": "ValueSet/$lookup" },
                    "resource": { "resourceType": "Parameters" }
                }),
            ],
        ),
    )
    .await;

    response.assert_status(StatusCode::NOT_IMPLEMENTED);
    let outcome: Value = response.json();
    assert_operation_refusal_text(&outcome, "$lookup");
    assert!(
        issue_text(&outcome).contains("entry 2"),
        "the refusal must name the client's entry index (2): {outcome}"
    );

    assert_eq!(patient_count(&backend).await, 0, "nothing was written");
    let search = server
        .get("/Patient")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await;
    search.assert_status_ok();
    let bundle: Value = search.json();
    assert!(
        bundle["entry"].as_array().is_none_or(Vec::is_empty),
        "a follow-up search finds no Patient: {bundle}"
    );
}

/// A transaction's instance-read entry runs inside the storage transaction, so
/// before the fix the backend's `[type]/[id]` parser took the last two segments
/// of `Patient/p1/$everything` for the type `p1` and the id `$everything`, and
/// the bundle failed as the opaque "Entry failed with status 404" - not an
/// honest refusal. (A transaction `DELETE` of such a URL answered `204` for a
/// delete of nothing: see the mutation test below.)
#[tokio::test]
async fn transaction_get_instance_operation_is_refused_not_read_as_the_instance() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1").await;

    let response = post_bundle(
        &server,
        bundle_of(
            "transaction",
            vec![json!({ "request": { "method": "GET", "url": "Patient/p1/$everything" } })],
        ),
    )
    .await;

    response.assert_status(StatusCode::NOT_IMPLEMENTED);
    let body = response.text();
    let outcome: Value = serde_json::from_str(&body).expect("OperationOutcome");
    assert_operation_refusal_text(&outcome, "$everything");
    assert!(
        !body.contains(SEEDED_FAMILY),
        "the Patient must not have been read: {body}"
    );
}

/// A transaction entry that is a DELETE, PUT or POST naming an operation never
/// reaches a storage backend's `[type]/[id]` parser, which would take
/// `Patient/p1/$purge` for the type `p1` and the id `$purge`: the REST layer
/// refuses it first, so the seeded Patient is untouched. Before the fix a
/// transaction `DELETE Patient/p1/$expunge` answered `204 No Content`, as the
/// delete of a resource that was never there.
#[tokio::test]
async fn transaction_mutation_operation_urls_never_reach_storage() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1").await;

    for (method, url) in [
        ("DELETE", "Patient/p1/$expunge"),
        ("PUT", "Patient/p1/$purge"),
        ("POST", "Patient/p1/$validate"),
    ] {
        let mut entry = json!({ "request": { "method": method, "url": url } });
        if method != "DELETE" {
            entry["resource"] = json!({ "resourceType": "Patient", "id": "p1" });
        }
        let response = post_bundle(&server, bundle_of("transaction", vec![entry])).await;

        response.assert_status(StatusCode::NOT_IMPLEMENTED);
        let outcome: Value = response.json();
        let operation = url.rsplit('/').next().unwrap();
        assert_operation_refusal_text(&outcome, operation);
    }

    assert_eq!(patient_count(&backend).await, 1);
    let read = server
        .get("/Patient/p1")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await;
    read.assert_status_ok();
    let patient: Value = read.json();
    assert_eq!(patient["meta"]["versionId"], "1", "{patient}");
}

/// A `$` before the last segment fails a transaction as a malformed entry URL.
#[tokio::test]
async fn transaction_dollar_segment_before_the_last_is_a_malformed_url() {
    let (server, _backend) = create_test_server().await;

    let response = post_bundle(
        &server,
        bundle_of(
            "transaction",
            vec![json!({ "request": { "method": "GET", "url": "Patient/$everything/p1" } })],
        ),
    )
    .await;

    response.assert_status(StatusCode::BAD_REQUEST);
    let outcome: Value = response.json();
    assert_eq!(outcome["issue"][0]["code"], "value", "{outcome}");
    assert!(issue_text(&outcome).contains("Entry 0"), "{outcome}");
}

// =============================================================================
// Both arms
// =============================================================================

/// One refusal, one sentence: the batch entry and the whole-bundle transaction
/// error for the same URL carry the same status family, code and wording, the
/// agreement #502 and #504 established for the other entry refusals.
#[tokio::test]
async fn the_two_arms_agree_on_the_operation_refusal() {
    let (server, _backend) = create_test_server().await;
    let request = json!({
        "request": { "method": "POST", "url": "ValueSet/$lookup" },
        "resource": { "resourceType": "Parameters" }
    });

    let batch = post_batch(&server, vec![request.clone()]).await;
    let batch_outcome = &batch[0]["response"]["outcome"];

    let transaction = post_bundle(&server, bundle_of("transaction", vec![request])).await;
    transaction.assert_status(StatusCode::NOT_IMPLEMENTED);
    let transaction_outcome: Value = transaction.json();

    assert_eq!(
        batch_outcome["issue"][0]["code"],
        transaction_outcome["issue"][0]["code"]
    );
    assert_eq!(
        issue_text(batch_outcome),
        issue_text(&transaction_outcome),
        "one refusal, one sentence"
    );
    assert_eq!(batch[0]["response"]["status"], "501 Not Implemented");
}
