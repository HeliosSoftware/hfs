//! Batch and transaction response conformance tests.
//!
//! Tests FHIR spec compliance for batch/transaction responses:
//! - Response Bundle type (batch-response / transaction-response)
//! - fullUrl on response entries
//! - Prefer header handling (return=minimal, return=representation, return=OperationOutcome)
//! - Error outcome placement (response.outcome, not entry.resource)
//! - lastModified and location on response entries
//! - Entry count matches request

mod common;

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
const PREFER: HeaderName = HeaderName::from_static("prefer");

/// An in-memory SQLite backend with the spec search parameters loaded, so
/// conditional criteria on `identifier` and friends resolve.
fn create_test_backend() -> SqliteBackend {
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
    backend
}

/// Creates a test server with a known base URL.
async fn create_test_server() -> (TestServer, Arc<SqliteBackend>) {
    create_test_server_from(create_test_backend()).await
}

/// Creates a test server over a caller-configured backend.
async fn create_test_server_from(backend: SqliteBackend) -> (TestServer, Arc<SqliteBackend>) {
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

async fn seed_patient(backend: &SqliteBackend, id: &str, family: &str) {
    let tenant = test_tenant();
    let patient = json!({
        "resourceType": "Patient",
        "id": id,
        "name": [{"family": family}],
        "active": true
    });
    backend
        .create(&tenant, "Patient", patient, FhirVersion::R4)
        .await
        .expect("Failed to seed patient");
}

async fn seed_audit_event(backend: &SqliteBackend, id: &str) {
    backend
        .create(
            &test_tenant(),
            "AuditEvent",
            json!({ "resourceType": "AuditEvent", "id": id }),
            FhirVersion::R4,
        )
        .await
        .expect("Failed to seed AuditEvent");
}

/// Helper: post a batch bundle and return the parsed response body.
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

/// Helper: post a batch bundle with a Prefer header.
async fn post_batch_with_prefer(server: &TestServer, bundle: Value, prefer: &str) -> Value {
    let response = server
        .post("/")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .add_header(PREFER, HeaderValue::from_str(prefer).unwrap())
        .json(&bundle)
        .await;
    response.assert_status_ok();
    response.json()
}

// =============================================================================
// Bundle Type Tests
// =============================================================================

mod bundle_type {
    use super::*;

    #[tokio::test]
    async fn test_batch_returns_batch_response_type() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "POST", "url": "Patient" },
                "resource": { "resourceType": "Patient", "name": [{"family": "Test"}] }
            }]
        });

        let body = post_batch(&server, bundle).await;
        assert_eq!(body["resourceType"], "Bundle");
        assert_eq!(body["type"], "batch-response");
    }

    #[tokio::test]
    async fn test_transaction_returns_transaction_response_type() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [{
                "request": { "method": "POST", "url": "Patient" },
                "resource": { "resourceType": "Patient", "name": [{"family": "Test"}] }
            }]
        });

        let body = post_batch(&server, bundle).await;
        assert_eq!(body["resourceType"], "Bundle");
        assert_eq!(body["type"], "transaction-response");
    }
}

// =============================================================================
// Response Entry Count Tests
// =============================================================================

mod entry_count {
    use super::*;

    #[tokio::test]
    async fn test_batch_response_has_one_entry_per_request() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [
                {
                    "request": { "method": "POST", "url": "Patient" },
                    "resource": { "resourceType": "Patient", "name": [{"family": "New"}] }
                },
                {
                    "request": { "method": "GET", "url": "Patient/p1" }
                },
                {
                    "request": { "method": "DELETE", "url": "Patient/p1" }
                }
            ]
        });

        let body = post_batch(&server, bundle).await;
        let entries = body["entry"].as_array().expect("entry should be an array");
        assert_eq!(
            entries.len(),
            3,
            "Response should have one entry per request"
        );
    }
}

// =============================================================================
// fullUrl Tests
// =============================================================================

mod full_url {
    use super::*;

    #[tokio::test]
    async fn test_batch_create_response_has_full_url() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "POST", "url": "Patient" },
                "resource": { "resourceType": "Patient", "name": [{"family": "Test"}] }
            }]
        });

        let body = post_batch(&server, bundle).await;
        let entry = &body["entry"][0];

        let full_url = entry["fullUrl"]
            .as_str()
            .expect("fullUrl should be present");
        assert!(
            full_url.starts_with("http://localhost:8080/Patient/"),
            "fullUrl should start with base URL + resource type: {}",
            full_url
        );
    }

    #[tokio::test]
    async fn test_batch_read_response_has_full_url() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "GET", "url": "Patient/p1" }
            }]
        });

        let body = post_batch(&server, bundle).await;
        let entry = &body["entry"][0];

        let full_url = entry["fullUrl"]
            .as_str()
            .expect("fullUrl should be present");
        assert_eq!(
            full_url, "http://localhost:8080/Patient/p1",
            "fullUrl should be base URL + resource path"
        );
    }

    #[tokio::test]
    async fn test_batch_delete_response_has_no_full_url() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "DELETE", "url": "Patient/p1" }
            }]
        });

        let body = post_batch(&server, bundle).await;
        let entry = &body["entry"][0];

        // DELETE returns no resource and no location, so no fullUrl
        assert!(
            entry.get("fullUrl").is_none() || entry["fullUrl"].is_null(),
            "DELETE response should not have fullUrl"
        );
    }

    #[tokio::test]
    async fn test_transaction_create_response_has_full_url() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [{
                "fullUrl": "urn:uuid:test-1",
                "request": { "method": "POST", "url": "Patient" },
                "resource": { "resourceType": "Patient", "name": [{"family": "TxTest"}] }
            }]
        });

        let body = post_batch(&server, bundle).await;
        let entry = &body["entry"][0];

        let full_url = entry["fullUrl"]
            .as_str()
            .expect("fullUrl should be present");
        assert!(
            full_url.starts_with("http://localhost:8080/Patient/"),
            "fullUrl should start with base URL: {}",
            full_url
        );
    }
}

// =============================================================================
// Response Fields Tests (status, etag, lastModified, location)
// =============================================================================

mod response_fields {
    use super::*;

    #[tokio::test]
    async fn test_batch_create_has_status_location_etag_last_modified() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "POST", "url": "Patient" },
                "resource": { "resourceType": "Patient", "name": [{"family": "Test"}] }
            }]
        });

        let body = post_batch(&server, bundle).await;
        let response = &body["entry"][0]["response"];

        assert_eq!(
            response["status"].as_str().unwrap(),
            "201 Created",
            "Create should return 201"
        );

        assert!(
            response["location"].as_str().is_some(),
            "Create response should have location"
        );

        let etag = response["etag"].as_str().expect("Create should have etag");
        assert!(etag.starts_with("W/\""), "ETag should be weak: {}", etag);

        assert!(
            response["lastModified"].as_str().is_some(),
            "Create response should have lastModified"
        );
    }

    #[tokio::test]
    async fn test_batch_read_has_etag_and_last_modified() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "GET", "url": "Patient/p1" }
            }]
        });

        let body = post_batch(&server, bundle).await;
        let response = &body["entry"][0]["response"];

        assert_eq!(response["status"].as_str().unwrap(), "200 OK");

        assert!(
            response["etag"].as_str().is_some(),
            "Read response should have etag"
        );

        assert!(
            response["lastModified"].as_str().is_some(),
            "Read response should have lastModified"
        );
    }

    #[tokio::test]
    async fn test_batch_update_has_location() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "PUT", "url": "Patient/p1" },
                "resource": {
                    "resourceType": "Patient",
                    "id": "p1",
                    "name": [{"family": "Updated"}]
                }
            }]
        });

        let body = post_batch(&server, bundle).await;
        let response = &body["entry"][0]["response"];

        assert_eq!(response["status"].as_str().unwrap(), "200 OK");

        assert!(
            response["etag"].as_str().is_some(),
            "Update response should have etag"
        );

        assert!(
            response["lastModified"].as_str().is_some(),
            "Update response should have lastModified"
        );
    }

    #[tokio::test]
    async fn test_batch_delete_has_status_204() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "DELETE", "url": "Patient/p1" }
            }]
        });

        let body = post_batch(&server, bundle).await;
        let response = &body["entry"][0]["response"];

        assert_eq!(
            response["status"].as_str().unwrap(),
            "204 No Content",
            "Delete should return 204"
        );
    }

    #[tokio::test]
    async fn test_batch_upsert_create_returns_201() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "PUT", "url": "Patient/new-upsert" },
                "resource": {
                    "resourceType": "Patient",
                    "id": "new-upsert",
                    "name": [{"family": "Upserted"}]
                }
            }]
        });

        let body = post_batch(&server, bundle).await;
        let response = &body["entry"][0]["response"];

        assert_eq!(
            response["status"].as_str().unwrap(),
            "201 Created",
            "Upsert of new resource should return 201"
        );

        assert!(
            response["location"].as_str().is_some(),
            "Upsert create should have location"
        );
    }
}

// =============================================================================
// Error Outcome Placement Tests
// =============================================================================

mod error_outcome {
    use super::*;

    #[tokio::test]
    async fn test_batch_error_outcome_in_response_not_resource() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "GET", "url": "Patient/nonexistent" }
            }]
        });

        let body = post_batch(&server, bundle).await;
        let entry = &body["entry"][0];

        // outcome should be in response.outcome
        let outcome = &entry["response"]["outcome"];
        assert_eq!(
            outcome["resourceType"].as_str().unwrap(),
            "OperationOutcome",
            "Error outcome should be in response.outcome"
        );

        // resource should NOT be set
        assert!(
            entry.get("resource").is_none() || entry["resource"].is_null(),
            "Error entry should not have a resource field"
        );
    }

    #[tokio::test]
    async fn test_batch_error_has_status_and_outcome() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "POST", "url": "Patient" }
                // Missing resource — should produce an error
            }]
        });

        let body = post_batch(&server, bundle).await;
        let entry = &body["entry"][0];

        let status = entry["response"]["status"].as_str().unwrap();
        assert!(
            status.starts_with("400"),
            "Missing resource should return 400: {}",
            status
        );

        let outcome = &entry["response"]["outcome"];
        assert_eq!(
            outcome["resourceType"].as_str().unwrap(),
            "OperationOutcome"
        );
    }

    #[tokio::test]
    async fn test_batch_mixed_success_and_error() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "exists", "Smith").await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [
                {
                    "request": { "method": "GET", "url": "Patient/exists" }
                },
                {
                    "request": { "method": "GET", "url": "Patient/does-not-exist" }
                }
            ]
        });

        let body = post_batch(&server, bundle).await;
        let entries = body["entry"].as_array().unwrap();

        // First entry: success
        assert_eq!(entries[0]["response"]["status"].as_str().unwrap(), "200 OK");
        assert!(entries[0].get("resource").is_some());

        // Second entry: error
        let status = entries[1]["response"]["status"].as_str().unwrap();
        assert!(status.starts_with("404"), "Not found should return 404");
        assert!(entries[1]["response"]["outcome"]["resourceType"] == "OperationOutcome");
    }
}

// =============================================================================
// Prefer Header Tests
// =============================================================================

mod prefer_header {
    use super::*;

    #[tokio::test]
    async fn test_prefer_representation_includes_resource() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "POST", "url": "Patient" },
                "resource": { "resourceType": "Patient", "name": [{"family": "Test"}] }
            }]
        });

        let body = post_batch_with_prefer(&server, bundle, "return=representation").await;
        let entry = &body["entry"][0];

        assert!(
            entry.get("resource").is_some() && !entry["resource"].is_null(),
            "return=representation should include resource in response"
        );

        assert_eq!(
            entry["resource"]["resourceType"].as_str().unwrap(),
            "Patient",
            "Resource should be the created Patient"
        );
    }

    #[tokio::test]
    async fn test_prefer_minimal_omits_resource() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "POST", "url": "Patient" },
                "resource": { "resourceType": "Patient", "name": [{"family": "Test"}] }
            }]
        });

        let body = post_batch_with_prefer(&server, bundle, "return=minimal").await;
        let entry = &body["entry"][0];

        assert!(
            entry.get("resource").is_none() || entry["resource"].is_null(),
            "return=minimal should NOT include resource in response"
        );

        // Response metadata should still be present
        assert!(
            entry["response"]["status"].as_str().is_some(),
            "Status should still be present"
        );
        assert!(
            entry["response"]["etag"].as_str().is_some(),
            "ETag should still be present even with minimal"
        );
    }

    #[tokio::test]
    async fn test_prefer_operation_outcome_returns_outcome() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "POST", "url": "Patient" },
                "resource": { "resourceType": "Patient", "name": [{"family": "Test"}] }
            }]
        });

        let body = post_batch_with_prefer(&server, bundle, "return=OperationOutcome").await;
        let entry = &body["entry"][0];

        assert!(
            entry.get("resource").is_some() && !entry["resource"].is_null(),
            "return=OperationOutcome should include a resource (the OperationOutcome)"
        );

        assert_eq!(
            entry["resource"]["resourceType"].as_str().unwrap(),
            "OperationOutcome",
            "Resource should be an OperationOutcome when return=OperationOutcome"
        );
    }

    #[tokio::test]
    async fn test_default_prefer_includes_resource() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "POST", "url": "Patient" },
                "resource": { "resourceType": "Patient", "name": [{"family": "Test"}] }
            }]
        });

        // No Prefer header — should default to representation
        let body = post_batch(&server, bundle).await;
        let entry = &body["entry"][0];

        assert!(
            entry.get("resource").is_some() && !entry["resource"].is_null(),
            "Default (no Prefer) should include resource in response"
        );

        assert_eq!(
            entry["resource"]["resourceType"].as_str().unwrap(),
            "Patient"
        );
    }

    #[tokio::test]
    async fn test_prefer_minimal_on_transaction() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [{
                "fullUrl": "urn:uuid:tx-1",
                "request": { "method": "POST", "url": "Patient" },
                "resource": { "resourceType": "Patient", "name": [{"family": "TxMinimal"}] }
            }]
        });

        let body = post_batch_with_prefer(&server, bundle, "return=minimal").await;
        assert_eq!(body["type"], "transaction-response");

        let entry = &body["entry"][0];
        assert!(
            entry.get("resource").is_none() || entry["resource"].is_null(),
            "return=minimal on transaction should omit resource"
        );

        // Metadata should still be present
        assert_eq!(entry["response"]["status"].as_str().unwrap(), "201 Created");
    }

    #[tokio::test]
    async fn test_prefer_minimal_read_omits_resource() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "GET", "url": "Patient/p1" }
            }]
        });

        let body = post_batch_with_prefer(&server, bundle, "return=minimal").await;
        let entry = &body["entry"][0];

        assert!(
            entry.get("resource").is_none() || entry["resource"].is_null(),
            "return=minimal should omit resource even for reads"
        );
    }
}

// =============================================================================
// Transaction Error Response Tests
// =============================================================================

mod transaction_errors {
    use super::*;

    #[tokio::test]
    async fn test_failed_transaction_returns_operation_outcome() {
        let (server, _backend) = create_test_server().await;

        // Transaction with a bad entry (missing resource for POST)
        let bundle = json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [{
                "request": { "method": "POST", "url": "Patient" }
                // Missing resource
            }]
        });

        let response = server
            .post("/")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&bundle)
            .await;

        // Failed transaction should return 4xx/5xx with OperationOutcome, not a Bundle
        let status = response.status_code();
        assert!(
            status.is_client_error() || status.is_server_error(),
            "Failed transaction should return error status: {}",
            status
        );

        let body: Value = response.json();
        assert_eq!(
            body["resourceType"].as_str().unwrap(),
            "OperationOutcome",
            "Failed transaction should return OperationOutcome, not a Bundle"
        );
    }

    #[tokio::test]
    async fn test_invalid_bundle_type_returns_400() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": []
        });

        let response = server
            .post("/")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&bundle)
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
    }
}

mod resource_type_admission {
    use super::*;

    #[tokio::test]
    async fn batch_keeps_valid_siblings_and_rejects_invalid_write_types() {
        let (server, backend) = create_test_server().await;
        let body = post_batch(
            &server,
            json!({
                "resourceType": "Bundle",
                "type": "batch",
                "entry": [
                    {
                        "request": { "method": "PUT", "url": "Patient/good" },
                        "resource": { "resourceType": "Patient", "id": "good" }
                    },
                    {
                        "request": { "method": "PUT", "url": "NoLongerValid/bad" },
                        "resource": { "resourceType": "NoLongerValid", "id": "bad" }
                    }
                ]
            }),
        )
        .await;

        assert_eq!(body["entry"][0]["response"]["status"], "201 Created");
        // "Resource type not supported" is a `404` inside a Bundle exactly as
        // it is at `PUT [base]/NoLongerValid/bad` (#504, #989).
        assert_eq!(body["entry"][1]["response"]["status"], "404 Not Found");
        assert_eq!(
            body["entry"][1]["response"]["outcome"]["issue"][0]["code"],
            "not-supported"
        );
        assert!(
            backend
                .read(&test_tenant(), "Patient", "good")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            backend
                .read(&test_tenant(), "NoLongerValid", "bad")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn batch_rejects_missing_mismatched_and_audit_event_bodies() {
        let (server, backend) = create_test_server().await;
        let body = post_batch(
            &server,
            json!({
                "resourceType": "Bundle",
                "type": "batch",
                "entry": [
                    {
                        "request": { "method": "PUT", "url": "Patient/missing" },
                        "resource": { "id": "missing" }
                    },
                    {
                        "request": { "method": "PUT", "url": "Patient/mismatch" },
                        "resource": { "resourceType": "Observation", "id": "mismatch" }
                    },
                    {
                        "request": { "method": "POST", "url": "AuditEvent" },
                        "resource": { "resourceType": "AuditEvent" }
                    }
                ]
            }),
        )
        .await;

        assert_eq!(body["entry"][0]["response"]["status"], "400 Bad Request");
        assert_eq!(body["entry"][1]["response"]["status"], "400 Bad Request");
        assert_eq!(
            body["entry"][2]["response"]["status"],
            "405 Method Not Allowed"
        );
        assert!(
            backend
                .read(&test_tenant(), "Patient", "missing")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            backend
                .read(&test_tenant(), "Patient", "mismatch")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn transaction_type_failure_prevents_every_sibling_write() {
        let (server, backend) = create_test_server().await;
        let response = server
            .post("/")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&json!({
                "resourceType": "Bundle",
                "type": "transaction",
                "entry": [
                    {
                        "request": { "method": "PUT", "url": "Patient/sibling" },
                        "resource": { "resourceType": "Patient", "id": "sibling" }
                    },
                    {
                        "request": { "method": "PUT", "url": "NoLongerValid/bad" },
                        "resource": { "resourceType": "NoLongerValid", "id": "bad" }
                    }
                ]
            }))
            .await;

        // The transaction is declined whole with the failing entry's own
        // status: an unsupported resource type is a `404` (#989).
        response.assert_status(StatusCode::NOT_FOUND);
        let outcome: Value = response.json();
        assert_eq!(outcome["resourceType"], "OperationOutcome");
        assert_eq!(outcome["issue"][0]["code"], "not-supported");
        assert!(
            backend
                .read(&test_tenant(), "Patient", "sibling")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            backend
                .read(&test_tenant(), "NoLongerValid", "bad")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn transaction_audit_event_failure_prevents_sibling_write() {
        let (server, backend) = create_test_server().await;
        let response = server
            .post("/")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&json!({
                "resourceType": "Bundle",
                "type": "transaction",
                "entry": [
                    {
                        "request": { "method": "PUT", "url": "Patient/sibling" },
                        "resource": { "resourceType": "Patient", "id": "sibling" }
                    },
                    {
                        "request": { "method": "POST", "url": "AuditEvent" },
                        "resource": { "resourceType": "AuditEvent" }
                    }
                ]
            }))
            .await;

        response.assert_status(StatusCode::METHOD_NOT_ALLOWED);
        assert!(
            backend
                .read(&test_tenant(), "Patient", "sibling")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn batch_audit_event_delete_is_405_without_blocking_valid_siblings() {
        let (server, backend) = create_test_server().await;
        seed_audit_event(&backend, "audit-1").await;
        seed_audit_event(&backend, "audit-2").await;

        let body = post_batch(
            &server,
            json!({
                "resourceType": "Bundle",
                "type": "batch",
                "entry": [
                    {
                        "request": {
                            "method": "DELETE",
                            "url": "https://example.test/fhir/AuditEvent/audit-1"
                        }
                    },
                    {
                        "request": { "method": "DELETE", "url": "fhir/AuditEvent/audit-2" }
                    },
                    {
                        "request": { "method": "PUT", "url": "Patient/good" },
                        "resource": { "resourceType": "Patient", "id": "good" }
                    }
                ]
            }),
        )
        .await;

        assert_eq!(
            body["entry"][0]["response"]["status"],
            "405 Method Not Allowed"
        );
        assert_eq!(
            body["entry"][1]["response"]["status"],
            "405 Method Not Allowed"
        );
        assert_eq!(body["entry"][2]["response"]["status"], "201 Created");
        assert!(
            backend
                .read(&test_tenant(), "AuditEvent", "audit-1")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            backend
                .read(&test_tenant(), "Patient", "good")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            backend
                .read(&test_tenant(), "AuditEvent", "audit-2")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn transaction_audit_event_delete_preflight_prevents_all_mutations() {
        for audit_url in [
            "https://example.test/fhir/AuditEvent/audit-1",
            "fhir/AuditEvent/audit-1",
        ] {
            let (server, backend) = create_test_server().await;
            seed_audit_event(&backend, "audit-1").await;
            seed_patient(&backend, "existing", "Keep").await;

            let response = server
                .post("/")
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .add_header(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/fhir+json"),
                )
                .json(&json!({
                    "resourceType": "Bundle",
                    "type": "transaction",
                    "entry": [
                        {
                            "request": { "method": "DELETE", "url": "Patient/existing" }
                        },
                        {
                            "request": { "method": "PUT", "url": "Patient/new" },
                            "resource": { "resourceType": "Patient", "id": "new" }
                        },
                        {
                            "request": { "method": "DELETE", "url": audit_url }
                        }
                    ]
                }))
                .await;

            response.assert_status(StatusCode::METHOD_NOT_ALLOWED);
            assert!(
                backend
                    .read(&test_tenant(), "AuditEvent", "audit-1")
                    .await
                    .unwrap()
                    .is_some(),
                "AuditEvent must survive {audit_url}"
            );
            assert!(
                backend
                    .read(&test_tenant(), "Patient", "existing")
                    .await
                    .unwrap()
                    .is_some(),
                "sibling delete must not run for {audit_url}"
            );
            assert!(
                backend
                    .read(&test_tenant(), "Patient", "new")
                    .await
                    .unwrap()
                    .is_none(),
                "sibling write must not run for {audit_url}"
            );
        }
    }
}

/// #459: conditional references (`Type?query`) resolve against the server's
/// content before the transaction executes — exactly one match rewrites the
/// reference; zero or several reject the bundle. They used to be stored
/// verbatim, unsearchable and unresolvable.
mod conditional_references {
    use super::*;

    async fn seed_location(backend: &SqliteBackend, id: &str, ident: &str) {
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "Location",
                json!({
                    "resourceType": "Location",
                    "id": id,
                    "status": "active",
                    "name": format!("Location {id}"),
                    "identifier": [{"system": "https://example.org/locs", "value": ident}]
                }),
                FhirVersion::R4,
            )
            .await
            .expect("seed location");
    }

    fn immunization_bundle() -> serde_json::Value {
        json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [{
                "fullUrl": "urn:uuid:11111111-1111-1111-1111-111111111111",
                "resource": {
                    "resourceType": "Immunization",
                    "status": "completed",
                    "vaccineCode": {"coding": [{"system": "http://hl7.org/fhir/sid/cvx", "code": "140"}]},
                    "patient": {"reference": "Patient/p1"},
                    "occurrenceDateTime": "2020-01-01",
                    "location": {"reference": "Location?identifier=https://example.org/locs|loc-a"}
                },
                "request": {"method": "POST", "url": "Immunization"}
            }]
        })
    }

    #[tokio::test]
    async fn a_unique_match_is_rewritten_into_storage() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "CondRef").await;
        seed_location(&backend, "loc-1", "loc-a").await;

        let response = server
            .post("/")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .json(&immunization_bundle())
            .await;
        response.assert_status_ok();

        let stored = server
            .get("/Immunization?_count=5")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        let body: serde_json::Value = stored.json();
        let imm = &body["entry"][0]["resource"];
        assert_eq!(
            imm["location"]["reference"], "Location/loc-1",
            "the conditional reference is resolved, not stored verbatim: {imm}"
        );
    }

    #[tokio::test]
    async fn no_match_rejects_the_bundle() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "CondRef").await;

        let response = server
            .post("/")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .json(&immunization_bundle())
            .await;
        response.assert_status(StatusCode::BAD_REQUEST);
    }

    /// Seeds `p1` and `p2`; only `p2` has an Observation with code 1234-5
    /// and only `p2` is managed by the Organization named Acme.
    async fn seed_chain_targets(backend: &SqliteBackend) {
        let tenant = test_tenant();
        seed_patient(backend, "p1", "CondRef").await;
        for (ty, body) in [
            (
                "Organization",
                json!({"resourceType": "Organization", "id": "org-acme", "name": "Acme"}),
            ),
            (
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "p2",
                    "name": [{"family": "Chained"}],
                    "managingOrganization": {"reference": "Organization/org-acme"}
                }),
            ),
            (
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "o2",
                    "status": "final",
                    "code": {"coding": [{"system": "http://loinc.org", "code": "1234-5"}]},
                    "subject": {"reference": "Patient/p2"}
                }),
            ),
        ] {
            backend
                .create(&tenant, ty, body, FhirVersion::R4)
                .await
                .expect("seed chain target");
        }
    }

    /// Posts the Immunization bundle with `patient` set to `reference`.
    async fn post_with_patient_reference(
        server: &TestServer,
        reference: &str,
    ) -> axum_test::TestResponse {
        let mut bundle = immunization_bundle();
        bundle["entry"][0]["resource"]["location"] = Value::Null;
        bundle["entry"][0]["resource"]["patient"] = json!({ "reference": reference });
        server
            .post("/")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .json(&bundle)
            .await
    }

    /// A chained or `_has` conditional reference is resolved like a type
    /// search (#1389). `search()` does not read chains, and this path used to
    /// call it without resolving them: `_has` was dropped (so the reference
    /// bound to whichever Patient happened to be unique) and a dotted chain
    /// was misread as a plain reference (so it matched nothing).
    #[tokio::test]
    async fn chained_conditional_references_resolve_to_the_matching_resource() {
        for reference in [
            "Patient?_has:Observation:subject:code=1234-5",
            "Patient?organization.name=Acme",
        ] {
            let (server, backend) = create_test_server().await;
            seed_chain_targets(&backend).await;

            let response = post_with_patient_reference(&server, reference).await;
            response.assert_status_ok();

            let stored = server
                .get("/Immunization?_count=5")
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;
            let body: Value = stored.json();
            let imm = &body["entry"][0]["resource"];
            assert_eq!(
                imm["patient"]["reference"], "Patient/p2",
                "{reference} must resolve to the chained match: {imm}"
            );
        }
    }

    /// With no Observation behind it, a `_has` reference matches nothing and
    /// the bundle is rejected. It used to bind to the only Patient present.
    #[tokio::test]
    async fn a_has_conditional_reference_with_no_match_rejects_the_bundle() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "CondRef").await;

        let response =
            post_with_patient_reference(&server, "Patient?_has:Observation:subject:code=1234-5")
                .await;
        response.assert_status(StatusCode::BAD_REQUEST);
        let text = response.text();
        assert!(text.contains("matches no existing resource"), "got: {text}");
    }

    #[tokio::test]
    async fn an_ambiguous_match_rejects_the_bundle() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "CondRef").await;
        seed_location(&backend, "loc-1", "loc-a").await;
        seed_location(&backend, "loc-2", "loc-a").await;

        let response = server
            .post("/")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .json(&immunization_bundle())
            .await;
        response.assert_status(StatusCode::BAD_REQUEST);
    }
}

// =============================================================================
// Conditional Entry Tests (#503, #511)
// =============================================================================

/// Conditional interactions in batch entries — `PUT`/`DELETE [type]?[criteria]`
/// and `POST` with `ifNoneExist` — resolve against storage with the status
/// mapping the resource endpoints use (#511), and `ifNoneExist` resolves inside
/// a transaction.
///
/// Before #503 the criteria rode along inside the parsed resource type, so a
/// conditional `PUT`/`DELETE` addressed storage with a type like
/// `Patient?identifier=http:` and an empty id; the type-level guards below keep
/// that row from ever being written. URL criteria inside a transaction remain
/// declined whole.
mod conditional_entries {
    use super::*;

    /// Posts a bundle and returns the raw response without asserting on status,
    /// so declined transactions can be inspected.
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

    async fn patient_count(backend: &SqliteBackend) -> u64 {
        backend
            .count(&test_tenant(), Some("Patient"))
            .await
            .expect("count failed")
    }

    async fn seed_patient_with_identifier(backend: &SqliteBackend, id: &str, family: &str) {
        backend
            .create(
                &test_tenant(),
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": id,
                    "identifier": [{"system": "http://example.org", "value": "12345"}],
                    "name": [{"family": family}]
                }),
                FhirVersion::R4,
            )
            .await
            .expect("Failed to seed patient");
    }

    fn conditional_put(url: &str, family: &str) -> Value {
        json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "PUT", "url": url },
                "resource": {
                    "resourceType": "Patient",
                    "identifier": [{"system": "http://example.org", "value": "12345"}],
                    "name": [{"family": family}]
                }
            }]
        })
    }

    fn if_none_exist_post(criteria: &str, family: &str, full_url: Option<&str>) -> Value {
        let mut entry = json!({
            "request": { "method": "POST", "url": "Patient", "ifNoneExist": criteria },
            "resource": {
                "resourceType": "Patient",
                "identifier": [{"system": "http://example.org", "value": "12345"}],
                "name": [{"family": family}]
            }
        });
        if let Some(full_url) = full_url {
            entry["fullUrl"] = json!(full_url);
        }
        entry
    }

    async fn family_of(backend: &SqliteBackend, id: &str) -> String {
        backend
            .read(&test_tenant(), "Patient", id)
            .await
            .expect("read failed")
            .expect("patient exists")
            .content()["name"][0]["family"]
            .as_str()
            .expect("family")
            .to_string()
    }

    // ── PUT [type]?[criteria] ────────────────────────────────────────────────

    #[tokio::test]
    async fn conditional_put_creates_when_nothing_matches() {
        let (server, backend) = create_test_server().await;
        let before = patient_count(&backend).await;

        let body = post_batch(
            &server,
            conditional_put("Patient?identifier=http://example.org|12345", "Conditional"),
        )
        .await;

        let entry = &body["entry"][0];
        assert_eq!(entry["response"]["status"], "201 Created", "{entry}");
        assert!(
            entry["response"]["location"]
                .as_str()
                .is_some_and(|l| l.starts_with("Patient/") && l.contains("/_history/1")),
            "{entry}"
        );
        assert_eq!(entry["resource"]["name"][0]["family"], "Conditional");
        assert_eq!(patient_count(&backend).await, before + 1);
    }

    #[tokio::test]
    async fn conditional_put_updates_the_single_match() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;
        let before = patient_count(&backend).await;

        let body = post_batch(
            &server,
            conditional_put("Patient?identifier=http://example.org|12345", "Updated"),
        )
        .await;

        let entry = &body["entry"][0];
        assert_eq!(entry["response"]["status"], "200 OK", "{entry}");
        assert_eq!(entry["response"]["location"], "Patient/p1");
        assert_eq!(entry["resource"]["id"], "p1");
        assert_eq!(family_of(&backend, "p1").await, "Updated");
        assert_eq!(
            patient_count(&backend).await,
            before,
            "an update creates nothing"
        );
    }

    #[tokio::test]
    async fn conditional_put_with_several_matches_is_412_and_writes_nothing() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "One").await;
        seed_patient_with_identifier(&backend, "p2", "Two").await;
        let before = patient_count(&backend).await;

        let body = post_batch(
            &server,
            conditional_put("Patient?identifier=http://example.org|12345", "Ambiguous"),
        )
        .await;

        let entry = &body["entry"][0];
        assert_eq!(
            entry["response"]["status"], "412 Precondition Failed",
            "{entry}"
        );
        // Entry failures render through `create_error_result`, which carries
        // the message in `details.text` (the issue-code refinement is #516).
        assert!(
            entry["response"]["outcome"]["issue"][0]["details"]["text"]
                .as_str()
                .is_some_and(|t| t.contains("matched 2")),
            "{entry}"
        );
        assert_eq!(patient_count(&backend).await, before);
        assert_eq!(family_of(&backend, "p1").await, "One");
        assert_eq!(family_of(&backend, "p2").await, "Two");
    }

    /// Bundle entry URLs arrive percent-encoded; the criteria are decoded
    /// before the backend sees them, as a request URL's query would be.
    #[tokio::test]
    async fn percent_encoded_criteria_match_the_decoded_identifier() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;

        let body = post_batch(
            &server,
            conditional_put(
                "Patient?identifier=http%3A%2F%2Fexample.org%7C12345",
                "Decoded",
            ),
        )
        .await;

        assert_eq!(body["entry"][0]["response"]["status"], "200 OK", "{body}");
        assert_eq!(family_of(&backend, "p1").await, "Decoded");
    }

    /// `ifMatch` on a conditional entry is evaluated against the resource the
    /// criteria resolve to (#1381; it was a `400` before). The full matrix is
    /// `tests/conditional_if_match.rs`.
    #[tokio::test]
    async fn if_match_on_a_conditional_entry_is_honoured() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;

        let mut bundle = conditional_put("Patient?identifier=http://example.org|12345", "Stale");
        bundle["entry"][0]["request"]["ifMatch"] = json!("W/\"7\"");
        let body = post_batch(&server, bundle).await;

        assert_eq!(
            body["entry"][0]["response"]["status"], "412 Precondition Failed",
            "{body}"
        );
        assert_eq!(family_of(&backend, "p1").await, "Nguyen");

        let mut bundle = conditional_put("Patient?identifier=http://example.org|12345", "Fresh");
        bundle["entry"][0]["request"]["ifMatch"] = json!("W/\"1\"");
        let body = post_batch(&server, bundle).await;

        assert_eq!(body["entry"][0]["response"]["status"], "200 OK", "{body}");
        assert_eq!(family_of(&backend, "p1").await, "Fresh");
    }

    // ── DELETE [type]?[criteria] ─────────────────────────────────────────────

    fn conditional_delete(url: &str) -> Value {
        json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{ "request": { "method": "DELETE", "url": url } }]
        })
    }

    #[tokio::test]
    async fn conditional_delete_removes_the_single_match() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;
        seed_patient(&backend, "p2", "Bystander").await;

        let body = post_batch(
            &server,
            conditional_delete("Patient?identifier=http://example.org|12345"),
        )
        .await;

        let entry = &body["entry"][0];
        assert_eq!(entry["response"]["status"], "204 No Content", "{entry}");
        assert!(
            entry.get("resource").is_none(),
            "a 204 carries no body: {entry}"
        );
        assert!(
            !matches!(
                backend.read(&test_tenant(), "Patient", "p1").await,
                Ok(Some(_))
            ),
            "the match must be gone"
        );
        assert_eq!(family_of(&backend, "p2").await, "Bystander");
    }

    #[tokio::test]
    async fn conditional_delete_with_no_match_is_204() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Nguyen").await;
        let before = patient_count(&backend).await;

        let body = post_batch(
            &server,
            conditional_delete("Patient?identifier=http://example.org|nobody"),
        )
        .await;

        assert_eq!(
            body["entry"][0]["response"]["status"], "204 No Content",
            "{body}"
        );
        assert_eq!(patient_count(&backend).await, before);
    }

    #[tokio::test]
    async fn conditional_delete_with_several_matches_is_412_and_deletes_nothing() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "One").await;
        seed_patient_with_identifier(&backend, "p2", "Two").await;
        let before = patient_count(&backend).await;

        let body = post_batch(
            &server,
            conditional_delete("Patient?identifier=http://example.org|12345"),
        )
        .await;

        assert_eq!(
            body["entry"][0]["response"]["status"], "412 Precondition Failed",
            "{body}"
        );
        assert_eq!(patient_count(&backend).await, before);
    }

    // ── POST + ifNoneExist ───────────────────────────────────────────────────

    #[tokio::test]
    async fn if_none_exist_creates_then_answers_the_match() {
        let (server, backend) = create_test_server().await;
        let bundle = |family: &str| {
            json!({
                "resourceType": "Bundle",
                "type": "batch",
                "entry": [if_none_exist_post("identifier=http://example.org|12345", family, None)]
            })
        };

        let first = post_batch(&server, bundle("First")).await;
        let first_entry = &first["entry"][0];
        assert_eq!(
            first_entry["response"]["status"], "201 Created",
            "{first_entry}"
        );
        let created_location = first_entry["response"]["location"]
            .as_str()
            .expect("location")
            .to_string();

        let second = post_batch(&server, bundle("Second")).await;
        let second_entry = &second["entry"][0];
        assert_eq!(
            second_entry["response"]["status"], "200 OK",
            "{second_entry}"
        );
        assert_eq!(
            second_entry["response"]["location"], created_location,
            "the match is named, exactly as the create was"
        );
        assert_eq!(second_entry["resource"]["name"][0]["family"], "First");
        assert_eq!(patient_count(&backend).await, 1);
    }

    #[tokio::test]
    async fn if_none_exist_with_several_matches_is_412() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "One").await;
        seed_patient_with_identifier(&backend, "p2", "Two").await;

        let body = post_batch(
            &server,
            json!({
                "resourceType": "Bundle",
                "type": "batch",
                "entry": [if_none_exist_post("identifier=http://example.org|12345", "Third", None)]
            }),
        )
        .await;

        assert_eq!(
            body["entry"][0]["response"]["status"], "412 Precondition Failed",
            "{body}"
        );
        assert_eq!(patient_count(&backend).await, 2);
    }

    #[tokio::test]
    async fn transaction_failure_names_the_clients_entry_index() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "One").await;
        seed_patient_with_identifier(&backend, "p2", "Two").await;
        let before = patient_count(&backend).await;

        let response = post_bundle(
            &server,
            json!({
                "resourceType": "Bundle",
                "type": "transaction",
                "entry": [
                    {
                        "request": { "method": "PUT", "url": "Patient/px" },
                        "resource": {
                            "resourceType": "Patient",
                            "id": "px",
                            "name": [{"family": "Valid"}]
                        }
                    },
                    if_none_exist_post("identifier=http://example.org|12345", "Third", None)
                ]
            }),
        )
        .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        let text = body["issue"][0]["details"]["text"]
            .as_str()
            .expect("outcome text");
        assert!(
            text.contains("entry 1"),
            "the failing entry is the client's entry 1: {text}"
        );
        assert_eq!(patient_count(&backend).await, before);
    }

    /// The transaction executor resolves `ifNoneExist` inside the transaction:
    /// the same transaction twice creates once, and on the replay a `urn:uuid`
    /// reference to the matched entry resolves to the match.
    #[tokio::test]
    async fn transaction_if_none_exist_is_idempotent_and_resolves_references() {
        let (server, backend) = create_test_server().await;
        let bundle = json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [
                if_none_exist_post(
                    "identifier=http://example.org|12345",
                    "Once",
                    Some("urn:uuid:patient")
                ),
                {
                    "fullUrl": "urn:uuid:observation",
                    "request": { "method": "POST", "url": "Observation" },
                    "resource": {
                        "resourceType": "Observation",
                        "status": "final",
                        "code": {"text": "test"},
                        "subject": {"reference": "urn:uuid:patient"}
                    }
                }
            ]
        });

        let first = post_batch(&server, bundle.clone()).await;
        assert_eq!(
            first["entry"][0]["response"]["status"], "201 Created",
            "{first}"
        );
        let patient_id = first["entry"][0]["resource"]["id"]
            .as_str()
            .expect("created patient id")
            .to_string();
        assert_eq!(
            first["entry"][1]["resource"]["subject"]["reference"],
            json!(format!("Patient/{patient_id}"))
        );

        let second = post_batch(&server, bundle).await;
        assert_eq!(
            second["entry"][0]["response"]["status"], "200 OK",
            "{second}"
        );
        assert_eq!(second["entry"][0]["resource"]["id"], json!(patient_id));
        assert_eq!(
            second["entry"][1]["resource"]["subject"]["reference"],
            json!(format!("Patient/{patient_id}")),
            "a urn:uuid reference to the matched entry resolves to the match"
        );
        assert_eq!(patient_count(&backend).await, 1);
    }

    /// The corruption #503 closes: `create_or_update` with an empty id inserts
    /// `"id": ""` into the resource and delegates to `create`, whose id fallback
    /// fires on an absent id rather than an empty one — so the row is written,
    /// and every later type-level PUT reads it back and overwrites it.
    #[tokio::test]
    async fn a_type_level_put_never_writes_an_empty_id_row() {
        let (server, backend) = create_test_server().await;
        let before = patient_count(&backend).await;

        let body = post_batch(
            &server,
            json!({
                "resourceType": "Bundle",
                "type": "batch",
                "entry": [{
                    "request": { "method": "PUT", "url": "Patient" },
                    "resource": { "resourceType": "Patient", "name": [{"family": "NoId"}] }
                }]
            }),
        )
        .await;

        assert_eq!(body["entry"][0]["response"]["status"], "400 Bad Request");
        assert_eq!(patient_count(&backend).await, before);
        assert!(
            backend
                .read(&test_tenant(), "Patient", "")
                .await
                .ok()
                .flatten()
                .is_none(),
            "no resource may be stored under the empty id"
        );
    }

    /// An instance URL carrying a control parameter still addresses its
    /// instance — the query is dropped, not read as criteria.
    #[tokio::test]
    async fn an_instance_url_with_a_query_still_resolves() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Nguyen").await;

        let body = post_batch(
            &server,
            json!({
                "resourceType": "Bundle",
                "type": "batch",
                "entry": [{
                    "request": { "method": "GET", "url": "Patient/p1?_format=json" }
                }]
            }),
        )
        .await;

        assert_eq!(body["entry"][0]["response"]["status"], "200 OK");
        assert_eq!(body["entry"][0]["resource"]["id"], "p1");
    }

    // ── Transactions: resolved inside the atomic scope (#859) ──────────────

    const CRITERIA_URL: &str = "Patient?identifier=http://example.org|12345";

    fn transaction(entries: Vec<Value>) -> Value {
        json!({ "resourceType": "Bundle", "type": "transaction", "entry": entries })
    }

    fn put_entry(url: &str, family: &str) -> Value {
        json!({
            "request": { "method": "PUT", "url": url },
            "resource": {
                "resourceType": "Patient",
                "identifier": [{"system": "http://example.org", "value": "12345"}],
                "name": [{"family": family}]
            }
        })
    }

    fn delete_entry(url: &str) -> Value {
        json!({ "request": { "method": "DELETE", "url": url } })
    }

    fn sibling_post() -> Value {
        json!({
            "request": { "method": "POST", "url": "Patient" },
            "resource": { "resourceType": "Patient", "name": [{"family": "Sibling"}] }
        })
    }

    /// The bundle #503 used to decline whole: the conditional entry now
    /// updates its match inside the transaction, and the sibling commits.
    #[tokio::test]
    async fn a_transaction_conditional_put_updates_the_single_match() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;
        let before = patient_count(&backend).await;

        let body = post_batch(
            &server,
            transaction(vec![sibling_post(), put_entry(CRITERIA_URL, "Updated")]),
        )
        .await;

        assert_eq!(body["type"], "transaction-response");
        assert_eq!(
            body["entry"][0]["response"]["status"], "201 Created",
            "{body}"
        );
        let entry = &body["entry"][1];
        assert_eq!(entry["response"]["status"], "200 OK", "{entry}");
        assert_eq!(entry["response"]["location"], "Patient/p1/_history/2");
        assert_eq!(entry["resource"]["id"], "p1");
        assert_eq!(family_of(&backend, "p1").await, "Updated");
        assert_eq!(
            patient_count(&backend).await,
            before + 1,
            "the sibling committed"
        );
    }

    #[tokio::test]
    async fn a_transaction_conditional_put_creates_when_nothing_matches() {
        let (server, backend) = create_test_server().await;
        let before = patient_count(&backend).await;

        let body = post_batch(&server, transaction(vec![put_entry(CRITERIA_URL, "New")])).await;

        let entry = &body["entry"][0];
        assert_eq!(entry["response"]["status"], "201 Created", "{entry}");
        assert!(
            entry["response"]["location"]
                .as_str()
                .is_some_and(|l| l.starts_with("Patient/") && l.contains("/_history/1")),
            "{entry}"
        );
        assert_eq!(patient_count(&backend).await, before + 1);
    }

    /// A transaction bundle with a conditional PUT and a sibling create: the
    /// first bundle creates the matched-nothing Patient, the second updates it
    /// in place, and the sibling create lands both times. This is the shape
    /// an application sends when it addresses resources by identifier.
    fn transaction_with_conditional_put(family: &str) -> Value {
        json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [
                {
                    "request": { "method": "POST", "url": "Patient" },
                    "resource": { "resourceType": "Patient", "name": [{"family": "Sibling"}] }
                },
                {
                    "fullUrl": "urn:uuid:patient",
                    "request": {
                        "method": "PUT",
                        "url": "Patient?identifier=http://example.org|12345"
                    },
                    "resource": {
                        "resourceType": "Patient",
                        "identifier": [{"system": "http://example.org", "value": "12345"}],
                        "name": [{"family": family}]
                    }
                },
                {
                    "request": { "method": "POST", "url": "Observation" },
                    "resource": {
                        "resourceType": "Observation",
                        "status": "final",
                        "code": {"text": "test"},
                        "subject": {"reference": "urn:uuid:patient"}
                    }
                }
            ]
        })
    }

    #[tokio::test]
    async fn a_transaction_conditional_put_creates_then_updates_in_place() {
        let (server, backend) = create_test_server().await;
        let before = patient_count(&backend).await;

        let response = post_bundle(&server, transaction_with_conditional_put("First")).await;
        response.assert_status(StatusCode::OK);
        let first: Value = response.json();
        assert_eq!(
            first["entry"][1]["response"]["status"], "201 Created",
            "{first}"
        );
        let patient_id = first["entry"][1]["resource"]["id"]
            .as_str()
            .expect("created patient id")
            .to_string();
        assert_eq!(
            first["entry"][2]["resource"]["subject"]["reference"],
            json!(format!("Patient/{patient_id}")),
            "a reference to the conditional PUT resolves to the resource it created"
        );
        assert_eq!(patient_count(&backend).await, before + 2);

        let response = post_bundle(&server, transaction_with_conditional_put("Second")).await;
        response.assert_status(StatusCode::OK);
        let second: Value = response.json();
        assert_eq!(
            second["entry"][1]["response"]["status"], "200 OK",
            "{second}"
        );
        assert_eq!(second["entry"][1]["resource"]["id"], json!(patient_id));
        assert_eq!(
            second["entry"][1]["resource"]["name"][0]["family"],
            "Second"
        );
        assert_eq!(
            second["entry"][2]["resource"]["subject"]["reference"],
            json!(format!("Patient/{patient_id}")),
            "a reference to the conditional PUT resolves to the resource it matched"
        );
        assert_eq!(
            patient_count(&backend).await,
            before + 3,
            "the second bundle adds only its sibling create"
        );
    }

    #[tokio::test]
    async fn a_transaction_conditional_put_with_several_matches_is_412_and_rolls_back() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "One").await;
        seed_patient_with_identifier(&backend, "p2", "Two").await;
        let before = patient_count(&backend).await;

        let response = post_bundle(
            &server,
            transaction(vec![sibling_post(), put_entry(CRITERIA_URL, "Ambiguous")]),
        )
        .await;

        response.assert_status(StatusCode::PRECONDITION_FAILED);
        let body: Value = response.json();
        assert_eq!(body["resourceType"], "OperationOutcome");
        assert_eq!(body["issue"][0]["code"], "multiple-matches", "{body}");
        assert_eq!(
            patient_count(&backend).await,
            before,
            "the sibling rolled back"
        );
        assert_eq!(family_of(&backend, "p1").await, "One");
    }

    #[tokio::test]
    async fn a_transaction_conditional_delete_removes_the_single_match() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;

        let body = post_batch(&server, transaction(vec![delete_entry(CRITERIA_URL)])).await;

        let entry = &body["entry"][0];
        assert_eq!(entry["response"]["status"], "204 No Content", "{entry}");
        assert_eq!(
            entry["response"]["location"], "Patient/p1/_history/1",
            "the 204 names what it deleted"
        );
        assert!(entry.get("resource").is_none(), "{entry}");
        assert!(
            !backend
                .exists(&test_tenant(), "Patient", "p1")
                .await
                .expect("exists"),
            "the match is gone"
        );
    }

    #[tokio::test]
    async fn a_transaction_conditional_delete_with_no_match_is_204() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "other", "Other").await;
        let before = patient_count(&backend).await;

        let body = post_batch(&server, transaction(vec![delete_entry(CRITERIA_URL)])).await;

        assert_eq!(
            body["entry"][0]["response"]["status"], "204 No Content",
            "{body}"
        );
        assert_eq!(patient_count(&backend).await, before);
    }

    #[tokio::test]
    async fn a_transaction_conditional_delete_with_several_matches_is_412_and_deletes_nothing() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "One").await;
        seed_patient_with_identifier(&backend, "p2", "Two").await;
        let before = patient_count(&backend).await;

        let response = post_bundle(
            &server,
            transaction(vec![sibling_post(), delete_entry(CRITERIA_URL)]),
        )
        .await;

        response.assert_status(StatusCode::PRECONDITION_FAILED);
        let body: Value = response.json();
        assert_eq!(body["issue"][0]["code"], "multiple-matches", "{body}");
        assert_eq!(patient_count(&backend).await, before);
    }

    /// R4 §3.1.0.11.2: a resolved identity another entry addresses by id
    /// fails the bundle, naming both entries; neither write lands.
    #[tokio::test]
    async fn a_transaction_conditional_entry_overlapping_an_instance_entry_is_400() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;

        let response = post_bundle(
            &server,
            transaction(vec![
                json!({
                    "request": { "method": "PUT", "url": "Patient/p1" },
                    "resource": {
                        "resourceType": "Patient",
                        "id": "p1",
                        "identifier": [{"system": "http://example.org", "value": "12345"}],
                        "name": [{"family": "ByInstance"}]
                    }
                }),
                put_entry(CRITERIA_URL, "ByCriteria"),
            ]),
        )
        .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        let text = body["issue"][0]["details"]["text"]
            .as_str()
            .unwrap_or_default();
        assert!(text.contains("Patient/p1"), "{body}");
        assert!(
            text.contains("Patient/p1") && text.contains("resolves to"),
            "{body}"
        );
        assert_eq!(
            family_of(&backend, "p1").await,
            "Nguyen",
            "neither write may land"
        );
    }

    #[tokio::test]
    async fn two_transaction_conditional_entries_resolving_to_one_resource_are_400() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;

        let response = post_bundle(
            &server,
            transaction(vec![
                delete_entry(CRITERIA_URL),
                put_entry(CRITERIA_URL, "Again"),
            ]),
        )
        .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        assert_eq!(family_of(&backend, "p1").await, "Nguyen");
    }

    /// A conditional PUT that matched is resolved before anything executes,
    /// so a `urn:uuid` reference to it resolves from any entry.
    #[tokio::test]
    async fn a_urn_uuid_reference_to_a_matched_transaction_conditional_put_resolves() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;

        let mut put = put_entry(CRITERIA_URL, "Updated");
        put["fullUrl"] = json!("urn:uuid:patient");
        let body = post_batch(
            &server,
            transaction(vec![
                json!({
                    "fullUrl": "urn:uuid:observation",
                    "request": { "method": "POST", "url": "Observation" },
                    "resource": {
                        "resourceType": "Observation",
                        "status": "final",
                        "code": {"text": "test"},
                        "subject": {"reference": "urn:uuid:patient"}
                    }
                }),
                put,
            ]),
        )
        .await;

        assert_eq!(
            body["entry"][0]["response"]["status"], "201 Created",
            "{body}"
        );
        assert_eq!(body["entry"][1]["response"]["status"], "200 OK", "{body}");
        assert_eq!(
            body["entry"][0]["resource"]["subject"]["reference"], "Patient/p1",
            "{body}"
        );
    }

    #[tokio::test]
    async fn percent_encoded_criteria_resolve_in_a_transaction() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;

        let body = post_batch(
            &server,
            transaction(vec![put_entry(
                "Patient?identifier=http%3A%2F%2Fexample.org%7C12345",
                "Decoded",
            )]),
        )
        .await;

        assert_eq!(body["entry"][0]["response"]["status"], "200 OK", "{body}");
        assert_eq!(family_of(&backend, "p1").await, "Decoded");
    }

    /// Criteria are parsed with the search parser, so a modifier reaches the
    /// query builder instead of being dropped and matching nothing (#865).
    #[tokio::test]
    async fn a_modifier_in_transaction_criteria_is_honoured() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;
        let before = patient_count(&backend).await;

        let body = post_batch(
            &server,
            transaction(vec![put_entry("Patient?family:exact=Nguyen", "Exact")]),
        )
        .await;
        assert_eq!(body["entry"][0]["response"]["status"], "200 OK", "{body}");
        assert_eq!(family_of(&backend, "p1").await, "Exact");
        assert_eq!(patient_count(&backend).await, before);

        let body = post_batch(
            &server,
            transaction(vec![put_entry("Patient?family:exact=exact", "Case")]),
        )
        .await;
        assert_eq!(
            body["entry"][0]["response"]["status"], "201 Created",
            ":exact is case-sensitive, so this is no match: {body}"
        );
        assert_eq!(patient_count(&backend).await, before + 1);
    }

    /// A result parameter (`_count`, `_sort`, `_format`, …) is not a criterion.
    /// Beside real criteria it is dropped, as in a batch; alone it leaves
    /// nothing to match on, which is a `400` rather than "matches nothing" —
    /// a `PUT` would otherwise create, and a `_`-name once matched everything
    /// (#866). The refusal names the dropped parameters (#1542).
    #[tokio::test]
    async fn a_result_parameter_in_transaction_criteria_is_dropped() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;
        let before = patient_count(&backend).await;

        for url in [
            "Patient?_count=1",
            "Patient?_sort=name",
            "Patient?_include=Patient:link",
        ] {
            let response = post_bundle(&server, transaction(vec![delete_entry(url)])).await;
            response.assert_status(StatusCode::BAD_REQUEST);
            let body: Value = response.json();
            assert!(
                body["issue"][0]["details"]["text"]
                    .as_str()
                    .is_some_and(|t| t.contains("nothing to match on")),
                "{url}: {body}"
            );
        }
        let response = post_bundle(
            &server,
            transaction(vec![delete_entry("Patient?_id=nonexistent")]),
        )
        .await;
        assert!(response.status_code() != StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(patient_count(&backend).await, before, "nothing was deleted");

        let body = post_batch(
            &server,
            transaction(vec![put_entry(
                &format!("{CRITERIA_URL}&_format=json&_count=5"),
                "Updated",
            )]),
        )
        .await;
        assert_eq!(body["entry"][0]["response"]["status"], "200 OK", "{body}");
        assert_eq!(family_of(&backend, "p1").await, "Updated");
        assert_eq!(
            patient_count(&backend).await,
            before,
            "updated, not created"
        );
    }

    #[tokio::test]
    async fn an_unknown_parameter_in_transaction_criteria_is_400() {
        let (server, backend) = create_test_server().await;
        let before = patient_count(&backend).await;

        let response = post_bundle(
            &server,
            transaction(vec![put_entry("Patient?ident'ifier=x", "Unknown")]),
        )
        .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        assert!(
            body["issue"][0]["details"]["text"]
                .as_str()
                .is_some_and(|t| t.contains("'ident'ifier', which is not known")),
            "{body}"
        );
        assert_eq!(patient_count(&backend).await, before);
    }

    /// A modifier the parameter's type does not define is refused by the
    /// search parser; dropping it would silently widen the criteria (#865).
    #[tokio::test]
    async fn an_invalid_modifier_in_transaction_criteria_is_400() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;
        let before = patient_count(&backend).await;

        let response = post_bundle(
            &server,
            transaction(vec![put_entry(
                "Patient?birthdate:contains=1980",
                "Invalid",
            )]),
        )
        .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        assert!(
            body["issue"][0]["details"]["text"]
                .as_str()
                .is_some_and(|t| t.contains("':contains' is not supported")),
            "{body}"
        );
        assert_eq!(patient_count(&backend).await, before);
    }

    /// `ifMatch` on a conditional entry is evaluated against the resource the
    /// criteria resolve to, as in a batch (#1381): the current version writes.
    #[tokio::test]
    async fn if_match_on_a_transaction_conditional_entry_is_honoured() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;

        let mut put = put_entry(CRITERIA_URL, "Guarded");
        put["request"]["ifMatch"] = json!("W/\"1\"");
        let body = post_batch(&server, transaction(vec![put])).await;

        assert_eq!(body["entry"][0]["response"]["status"], "200 OK", "{body}");
        assert_eq!(family_of(&backend, "p1").await, "Guarded");
    }

    /// A stale `ifMatch` fails the whole bundle with `412` before anything is
    /// written, rolling back a sibling that would otherwise have committed.
    #[tokio::test]
    async fn a_stale_if_match_on_a_transaction_conditional_entry_is_412() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;
        let before = patient_count(&backend).await;

        for method in ["PUT", "DELETE"] {
            let mut entry = if method == "PUT" {
                put_entry(CRITERIA_URL, "Guarded")
            } else {
                delete_entry(CRITERIA_URL)
            };
            entry["request"]["ifMatch"] = json!("W/\"7\"");
            let response = post_bundle(&server, transaction(vec![sibling_post(), entry])).await;

            response.assert_status(StatusCode::PRECONDITION_FAILED);
            let body: Value = response.json();
            assert_eq!(body["issue"][0]["code"], "conflict", "{method}: {body}");
        }
        assert_eq!(
            patient_count(&backend).await,
            before,
            "sibling POST rolled back"
        );
        assert_eq!(family_of(&backend, "p1").await, "Nguyen");
    }

    /// Criteria that match nothing fail any supplied `ifMatch`, `*` included:
    /// the guarded `PUT` does not fall through to a create (#1381).
    #[tokio::test]
    async fn if_match_on_an_unmatched_transaction_conditional_put_does_not_create() {
        let (server, backend) = create_test_server().await;
        let before = patient_count(&backend).await;

        let mut put = put_entry(CRITERIA_URL, "Guarded");
        put["request"]["ifMatch"] = json!("*");
        let response = post_bundle(&server, transaction(vec![put])).await;

        response.assert_status(StatusCode::PRECONDITION_FAILED);
        assert_eq!(patient_count(&backend).await, before);
    }

    #[tokio::test]
    async fn a_transaction_post_with_url_criteria_is_400() {
        let (server, backend) = create_test_server().await;
        let before = patient_count(&backend).await;

        let mut post = put_entry(CRITERIA_URL, "Posted");
        post["request"]["method"] = json!("POST");
        let response = post_bundle(&server, transaction(vec![post])).await;

        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        assert!(
            body["issue"][0]["details"]["text"]
                .as_str()
                .is_some_and(|t| t.contains("ifNoneExist")),
            "{body}"
        );
        assert_eq!(patient_count(&backend).await, before);
    }

    fn conditional_patch_entry(url: &str) -> Value {
        json!({
            "request": { "method": "PATCH", "url": url },
            "resource": {
                "resourceType": "Parameters",
                "parameter": [{
                    "name": "operation",
                    "part": [
                        { "name": "type", "valueCode": "add" },
                        { "name": "path", "valueString": "Patient" },
                        { "name": "name", "valueString": "active" },
                        { "name": "value", "valueBoolean": true }
                    ]
                }]
            }
        })
    }

    /// `PATCH [type]?[criteria]` is resolved inside the transaction and
    /// patches the match (#1535).
    #[tokio::test]
    async fn a_transaction_conditional_patch_patches_the_match() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;
        let before = patient_count(&backend).await;

        let body = post_batch(
            &server,
            transaction(vec![sibling_post(), conditional_patch_entry(CRITERIA_URL)]),
        )
        .await;

        assert_eq!(body["entry"][1]["response"]["status"], "200 OK", "{body}");
        let p1 = backend
            .read(&test_tenant(), "Patient", "p1")
            .await
            .unwrap()
            .expect("p1");
        assert_eq!(p1.content()["active"], json!(true), "{body}");
        assert_eq!(family_of(&backend, "p1").await, "Nguyen");
        assert_eq!(
            patient_count(&backend).await,
            before + 1,
            "the sibling committed"
        );
    }

    /// No match is the `404` `PATCH [type]/[id]` answers, for the whole bundle;
    /// several matches are `412 multiple-matches`. Nothing is written either way.
    #[tokio::test]
    async fn a_transaction_conditional_patch_without_a_single_match_fails_the_bundle() {
        let (server, backend) = create_test_server().await;
        let before = patient_count(&backend).await;

        let response = post_bundle(
            &server,
            transaction(vec![sibling_post(), conditional_patch_entry(CRITERIA_URL)]),
        )
        .await;
        response.assert_status(StatusCode::NOT_FOUND);
        assert_eq!(patient_count(&backend).await, before, "sibling rolled back");

        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;
        seed_patient_with_identifier(&backend, "p2", "Nguyen").await;
        let response = post_bundle(
            &server,
            transaction(vec![conditional_patch_entry(CRITERIA_URL)]),
        )
        .await;
        response.assert_status(StatusCode::PRECONDITION_FAILED);
        let body: Value = response.json();
        assert_eq!(body["issue"][0]["code"], "multiple-matches", "{body}");
    }

    /// An instance URL's query is a control parameter, not criteria; it is
    /// dropped rather than reaching the backend as part of the id (#503).
    #[tokio::test]
    async fn an_instance_url_control_parameter_is_dropped_in_a_transaction() {
        let (server, backend) = create_test_server().await;

        let body = post_batch(
            &server,
            transaction(vec![json!({
                "request": { "method": "PUT", "url": "Patient/p9?_format=json" },
                "resource": { "resourceType": "Patient", "id": "p9" }
            })]),
        )
        .await;

        assert_eq!(
            body["entry"][0]["response"]["status"], "201 Created",
            "{body}"
        );
        assert!(
            backend
                .read(&test_tenant(), "Patient", "p9")
                .await
                .expect("read")
                .is_some(),
            "the row is keyed by the id, not by `p9?_format=json`"
        );
    }

    /// A backend whose search index lives in a secondary declines the bundle
    /// intact with 501, for URL criteria and `ifNoneExist` alike.
    #[tokio::test]
    async fn a_transaction_conditional_entry_is_501_when_search_is_offloaded() {
        let mut offloaded = create_test_backend();
        offloaded.set_search_offloaded(true);
        let (server, backend) = create_test_server_from(offloaded).await;
        let before = patient_count(&backend).await;

        for bundle in [
            transaction(vec![sibling_post(), put_entry(CRITERIA_URL, "Offloaded")]),
            transaction(vec![
                sibling_post(),
                if_none_exist_post("identifier=http://example.org|12345", "Offloaded", None),
            ]),
        ] {
            let response = post_bundle(&server, bundle).await;
            response.assert_status(StatusCode::NOT_IMPLEMENTED);
            let body: Value = response.json();
            assert_eq!(body["issue"][0]["code"], "not-supported", "{body}");
        }
        assert_eq!(
            patient_count(&backend).await,
            before,
            "declined before anything executes"
        );
    }
}

// =============================================================================
// Entry Method Tests (#502)
// =============================================================================

/// The batch and transaction arms parse `request.method` through one shared
/// matcher, so they accept exactly the same codes and refuse the rest with the
/// same status.
///
/// `Bundle.entry.request.method` is a `code` with a required binding to
/// `http-verb`, whose concepts are case-sensitive and uppercase — so a lowercase
/// verb is invalid instance data, and the transaction arm's old `to_uppercase()`
/// was the non-conformant matcher rather than batch being wrongly strict.
mod entry_methods {
    use super::*;

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

    async fn patient_count(backend: &SqliteBackend) -> u64 {
        backend
            .count(&test_tenant(), Some("Patient"))
            .await
            .expect("count failed")
    }

    fn batch_of(entries: Vec<Value>) -> Value {
        json!({ "resourceType": "Bundle", "type": "batch", "entry": entries })
    }

    /// A malformed PATCH payload fails only its batch entry and changes nothing.
    #[tokio::test]
    async fn malformed_batch_patch_is_400_and_changes_nothing() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Nguyen").await;

        let body = post_batch(
            &server,
            batch_of(vec![json!({
                "request": { "method": "PATCH", "url": "Patient/p1" },
                "resource": { "resourceType": "Patient", "name": [{"family": "Patched"}] }
            })]),
        )
        .await;

        assert_eq!(body["entry"][0]["response"]["status"], "400 Bad Request");
        let stored = backend
            .read(&test_tenant(), "Patient", "p1")
            .await
            .expect("read failed")
            .expect("patient must survive");
        assert_eq!(stored.content()["name"][0]["family"], "Nguyen");
    }

    /// HEAD is a legal http-verb code this server does not accept in a Bundle.
    #[tokio::test]
    async fn batch_head_is_refused_at_405() {
        let (server, _backend) = create_test_server().await;

        let body = post_batch(
            &server,
            batch_of(vec![json!({
                "request": { "method": "HEAD", "url": "Patient/p1" }
            })]),
        )
        .await;

        assert_eq!(
            body["entry"][0]["response"]["status"],
            "405 Method Not Allowed"
        );
    }

    #[tokio::test]
    async fn batch_refuses_a_lowercase_verb_and_a_missing_one() {
        let (server, backend) = create_test_server().await;
        let before = patient_count(&backend).await;

        let body = post_batch(
            &server,
            batch_of(vec![
                json!({
                    "request": { "method": "post", "url": "Patient" },
                    "resource": { "resourceType": "Patient" }
                }),
                json!({ "request": { "url": "Patient/p1" } }),
            ]),
        )
        .await;

        assert_eq!(body["entry"][0]["response"]["status"], "400 Bad Request");
        assert_eq!(body["entry"][1]["response"]["status"], "400 Bad Request");
        assert_eq!(patient_count(&backend).await, before);
    }

    /// **The regression test for #502.** On the old code this entry created a
    /// Patient: the transaction matcher upper-cased `"post"` and dispatched it,
    /// while the same Bundle 405'd as a batch.
    #[tokio::test]
    async fn a_transaction_lowercase_verb_no_longer_writes() {
        let (server, backend) = create_test_server().await;
        let before = patient_count(&backend).await;

        let response = post_bundle(
            &server,
            json!({
                "resourceType": "Bundle",
                "type": "transaction",
                "entry": [{
                    "request": { "method": "post", "url": "Patient" },
                    "resource": { "resourceType": "Patient", "name": [{"family": "Lowercase"}] }
                }]
            }),
        )
        .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        assert_eq!(
            patient_count(&backend).await,
            before,
            "a lowercase verb must not create a resource"
        );
    }

    /// A malformed PATCH rolls back a preceding create in the transaction.
    #[tokio::test]
    async fn a_malformed_transaction_patch_rolls_back_at_400() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Nguyen").await;
        let before = patient_count(&backend).await;

        let response = post_bundle(
            &server,
            json!({
                "resourceType": "Bundle",
                "type": "transaction",
                "entry": [
                    {
                        "request": { "method": "POST", "url": "Patient" },
                        "resource": { "resourceType": "Patient", "name": [{"family": "Sibling"}] }
                    },
                    {
                        "request": { "method": "PATCH", "url": "Patient/p1" },
                        "resource": { "resourceType": "Patient" }
                    }
                ]
            }),
        )
        .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        assert_eq!(body["resourceType"], "OperationOutcome");
        assert_eq!(body["issue"][0]["code"], "invalid");
        assert!(
            body["issue"][0]["details"]["text"]
                .as_str()
                .is_some_and(|t| t.contains("PATCH")),
            "the outcome must name PATCH: {body}"
        );
        assert_eq!(patient_count(&backend).await, before);
    }

    /// The two arms agree on status, which is what #502 asks for: HEAD is 405
    /// whether it arrives per-entry in a batch or as a whole-bundle transaction
    /// failure. Flattening the refusal at the transaction boundary would have
    /// made this 400 and re-created the divergence in a new place.
    ///
    /// They now agree on the issue code and the message too. #502 closed the
    /// status half and named the rest as #504's: the batch entry said
    /// `processing` while the transaction body said `not-supported`, and the
    /// batch arm printed HEAD guidance the transaction arm dropped, because
    /// `into_rest_error` computed a message and discarded it on that arm.
    #[tokio::test]
    async fn the_two_arms_agree_on_the_refusal_status_and_code() {
        let (server, _backend) = create_test_server().await;

        let batch = post_batch(
            &server,
            batch_of(vec![json!({
                "request": { "method": "HEAD", "url": "Patient/p1" }
            })]),
        )
        .await;
        assert_eq!(
            batch["entry"][0]["response"]["status"],
            "405 Method Not Allowed"
        );
        let batch_issue = &batch["entry"][0]["response"]["outcome"]["issue"][0];

        let transaction = post_bundle(
            &server,
            json!({
                "resourceType": "Bundle",
                "type": "transaction",
                "entry": [{ "request": { "method": "HEAD", "url": "Patient/p1" } }]
            }),
        )
        .await;
        transaction.assert_status(StatusCode::METHOD_NOT_ALLOWED);
        let transaction_body: Value = transaction.json();
        let transaction_issue = &transaction_body["issue"][0];

        assert_eq!(batch_issue["code"], transaction_issue["code"]);
        assert_eq!(
            batch_issue["details"]["text"], transaction_issue["details"]["text"],
            "one refusal, one sentence"
        );

        // Pinned literally: equality alone is satisfied by both arms being
        // wrong in the same way.
        assert_eq!(batch_issue["code"], "not-supported");
        assert!(
            batch_issue["details"]["text"]
                .as_str()
                .is_some_and(|t| t.contains("send HEAD to the instance endpoint")),
            "both arms must keep the guidance: {batch_issue}"
        );
    }
}

// =============================================================================
// GET Search Entry Tests (#478)
// =============================================================================

mod search_entries {
    use super::*;

    #[tokio::test]
    async fn test_batch_get_search_entry_returns_searchset() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Nguyen").await;
        seed_patient(&backend, "p2", "Smith").await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "GET", "url": "Patient?family=Nguyen" }
            }]
        });

        let body = post_batch(&server, bundle).await;
        let entry = &body["entry"][0];

        assert_eq!(entry["response"]["status"].as_str().unwrap(), "200 OK");
        let searchset = &entry["resource"];
        assert_eq!(searchset["resourceType"].as_str().unwrap(), "Bundle");
        assert_eq!(searchset["type"].as_str().unwrap(), "searchset");
        assert_eq!(searchset["entry"].as_array().unwrap().len(), 1);
        assert_eq!(
            searchset["entry"][0]["resource"]["name"][0]["family"]
                .as_str()
                .unwrap(),
            "Nguyen"
        );
    }

    #[tokio::test]
    async fn test_batch_get_bare_type_is_a_search() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Nguyen").await;
        seed_patient(&backend, "p2", "Smith").await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "GET", "url": "Patient" }
            }]
        });

        let body = post_batch(&server, bundle).await;
        let searchset = &body["entry"][0]["resource"];

        assert_eq!(searchset["type"].as_str().unwrap(), "searchset");
        assert_eq!(searchset["entry"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_batch_mixes_search_and_read_entries() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Nguyen").await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [
                { "request": { "method": "GET", "url": "Patient/p1" } },
                { "request": { "method": "GET", "url": "Patient?family=Nguyen" } }
            ]
        });

        let body = post_batch(&server, bundle).await;

        let read = &body["entry"][0];
        assert_eq!(read["response"]["status"].as_str().unwrap(), "200 OK");
        assert_eq!(
            read["resource"]["resourceType"].as_str().unwrap(),
            "Patient"
        );

        let search = &body["entry"][1];
        assert_eq!(search["response"]["status"].as_str().unwrap(), "200 OK");
        assert_eq!(search["resource"]["type"].as_str().unwrap(), "searchset");
    }

    #[tokio::test]
    async fn test_transaction_get_search_sees_the_bundles_own_writes() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [
                {
                    "resource": {
                        "resourceType": "Patient",
                        "name": [{"family": "Tran"}]
                    },
                    "request": { "method": "POST", "url": "Patient" }
                },
                { "request": { "method": "GET", "url": "Patient?family=Tran" } }
            ]
        });

        let body = post_batch(&server, bundle).await;
        assert_eq!(body["type"].as_str().unwrap(), "transaction-response");

        let created = &body["entry"][0];
        assert_eq!(
            created["response"]["status"].as_str().unwrap(),
            "201 Created"
        );

        let search = &body["entry"][1];
        assert_eq!(search["response"]["status"].as_str().unwrap(), "200 OK");
        let searchset = &search["resource"];
        assert_eq!(searchset["type"].as_str().unwrap(), "searchset");
        assert_eq!(
            searchset["entry"].as_array().unwrap().len(),
            1,
            "the search runs after the writes and must see the created patient"
        );
        assert_eq!(
            searchset["entry"][0]["resource"]["name"][0]["family"]
                .as_str()
                .unwrap(),
            "Tran"
        );
    }

    #[tokio::test]
    async fn test_transaction_get_by_id_still_reads_in_transaction() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Nguyen").await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [{
                "request": { "method": "GET", "url": "Patient/p1" }
            }]
        });

        let body = post_batch(&server, bundle).await;
        let entry = &body["entry"][0];
        assert_eq!(entry["response"]["status"].as_str().unwrap(), "200 OK");
        assert_eq!(
            entry["resource"]["resourceType"].as_str().unwrap(),
            "Patient"
        );
    }
}

// =============================================================================
// GET history and vread entries (#2380)
// =============================================================================

/// A `GET` entry whose URL is a history interaction is answered like the same
/// direct request: a vread entry carries the requested version, and a history
/// entry carries the `history` Bundle. Before #2380 the instance forms were
/// read as the current version of `[type]/[id]`, `Patient/_history` was a read
/// of a Patient whose id is `_history`, and `_history` a search of a resource
/// type of that name.
mod history_entries {
    use super::*;
    use chrono::{SecondsFormat, Utc};
    use std::time::Duration;

    /// PUTs a Patient named `family` (a create the first time, an update after).
    async fn put_patient(server: &TestServer, id: &str, family: &str) {
        let response = server
            .put(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&json!({
                "resourceType": "Patient",
                "id": id,
                "name": [{"family": family}]
            }))
            .await;
        assert!(
            response.status_code().is_success(),
            "PUT Patient/{id}: {} {}",
            response.status_code(),
            response.text()
        );
    }

    /// Creates `id` and updates it twice: versions 1, 2 and 3, families `V1`,
    /// `V2` and `V3`, written over HTTP like `rest_conformance`'s helper.
    ///
    /// Returns an instant strictly between versions 1 and 2, form-urlencoded as
    /// a `_since` value. The pauses keep it away from both writes: SQLite
    /// compares `last_updated` as a string, so an instant that is close to a
    /// write could land on either side of it.
    async fn seed_versioned_patient(server: &TestServer, id: &str) -> String {
        put_patient(server, id, "V1").await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let between = Utc::now().to_rfc3339_opts(SecondsFormat::AutoSi, true);
        tokio::time::sleep(Duration::from_millis(50)).await;
        put_patient(server, id, "V2").await;
        put_patient(server, id, "V3").await;
        url::form_urlencoded::byte_serialize(between.as_bytes()).collect()
    }

    /// The direct request for `path` (with its leading `/`).
    async fn direct_response(server: &TestServer, path: &str) -> axum_test::TestResponse {
        server
            .get(path)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await
    }

    /// The direct request's status and JSON body.
    async fn direct_get(server: &TestServer, path: &str) -> (StatusCode, Value) {
        let response = direct_response(server, path).await;
        (response.status_code(), response.json())
    }

    /// A bundle of `GET` entries, one per URL.
    fn gets(kind: &str, urls: &[&str]) -> Value {
        json!({
            "resourceType": "Bundle",
            "type": kind,
            "entry": urls
                .iter()
                .map(|url| json!({"request": {"method": "GET", "url": url}}))
                .collect::<Vec<_>>()
        })
    }

    fn status(entry: &Value) -> &str {
        entry["response"]["status"]
            .as_str()
            .unwrap_or_else(|| panic!("entry without a status: {entry}"))
    }

    /// The first issue's code of an entry's `response.outcome`.
    fn outcome_code(entry: &Value) -> &str {
        entry["response"]["outcome"]["issue"][0]["code"]
            .as_str()
            .unwrap_or_else(|| panic!("entry without an outcome code: {entry}"))
    }

    fn outcome_text(entry: &Value) -> &str {
        entry["response"]["outcome"]["issue"][0]["details"]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("entry without an outcome text: {entry}"))
    }

    fn entry_versions(history: &Value) -> Vec<String> {
        history["entry"]
            .as_array()
            .expect("a history Bundle has entries")
            .iter()
            .map(|e| {
                e["resource"]["meta"]["versionId"]
                    .as_str()
                    .expect("entry versionId")
                    .to_string()
            })
            .collect()
    }

    fn has_link(history: &Value, relation: &str) -> bool {
        history["link"]
            .as_array()
            .is_some_and(|links| links.iter().any(|l| l["relation"] == relation))
    }

    #[tokio::test]
    async fn history_and_vread_entries_answer_like_the_direct_requests() {
        let (server, backend) = create_test_server().await;
        let since = seed_versioned_patient(&server, "p1").await;
        seed_patient(&backend, "p2", "Other").await;

        let paths = [
            "Patient/p1/_history/1".to_string(),
            "Patient/p1/_history?_count=2".to_string(),
            format!("Patient/p1/_history?_since={since}"),
            "Patient/_history".to_string(),
            "_history".to_string(),
        ];
        let urls: Vec<&str> = paths.iter().map(String::as_str).collect();
        let body = post_batch(&server, gets("batch", &urls)).await;
        let entries = body["entry"].as_array().expect("response entries");
        assert_eq!(entries.len(), paths.len(), "{body}");

        for (path, entry) in paths.iter().zip(entries) {
            assert_eq!(status(entry), "200 OK", "{path}: {entry}");
            let (direct_status, direct_body) = direct_get(&server, &format!("/{path}")).await;
            assert_eq!(direct_status, StatusCode::OK, "direct {path}");
            assert_eq!(
                entry["resource"], direct_body,
                "{path}: the entry is not the direct response"
            );
        }

        // The vread is version 1, not the current version, with the etag and
        // lastModified of a read entry.
        let vread = &entries[0];
        assert_eq!(vread["resource"]["resourceType"], "Patient");
        assert_eq!(vread["resource"]["name"][0]["family"], "V1", "{vread}");
        assert_eq!(vread["response"]["etag"], "W/\"1\"");
        assert!(vread["response"]["lastModified"].is_string(), "{vread}");
        let direct = direct_response(&server, "/Patient/p1/_history/1").await;
        assert_eq!(
            direct
                .header("etag")
                .to_str()
                .expect("the direct vread has an ETag"),
            vread["response"]["etag"].as_str().unwrap()
        );

        // Instance history, paged: 2 of 3 versions and a next link. `total` is
        // whatever the backend reports, which the equality with the direct
        // response above already pins.
        let paged = &entries[1]["resource"];
        assert_eq!(paged["type"], "history");
        assert_eq!(paged["entry"].as_array().unwrap().len(), 2);
        assert!(has_link(paged, "next"), "{paged}");

        // `_since` keeps the versions written after the instant.
        let since_page = &entries[2]["resource"];
        assert_eq!(since_page["type"], "history");
        let mut versions = entry_versions(since_page);
        versions.sort();
        assert_eq!(versions, ["2", "3"]);

        // Type and system history span every Patient: 3 versions of p1, 1 of p2.
        for (index, level) in [(3, "type"), (4, "system")] {
            let history = &entries[index]["resource"];
            assert_eq!(history["type"], "history", "{level}");
            assert_eq!(history["total"], 4, "{level}: {history}");
        }
    }

    #[tokio::test]
    async fn vread_entry_of_a_missing_or_deleted_version_is_that_entrys_error() {
        let (server, backend) = create_test_server().await;
        seed_versioned_patient(&server, "p1").await;
        seed_patient(&backend, "p2", "Other").await;

        // The DELETE writes version 4, the tombstone.
        server
            .delete("/Patient/p1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        let urls = [
            "Patient/p1/_history/999",
            "Patient/p1/_history/4",
            "Patient/p2",
        ];
        let body = post_batch(&server, gets("batch", &urls)).await;
        let entries = body["entry"].as_array().expect("response entries");

        assert_eq!(status(&entries[0]), "404 Not Found", "{}", entries[0]);
        assert_eq!(outcome_code(&entries[0]), "not-found");
        assert_eq!(status(&entries[1]), "410 Gone", "{}", entries[1]);
        assert_eq!(status(&entries[2]), "200 OK", "{}", entries[2]);

        // Each outcome is the direct response's.
        for (url, entry) in urls.iter().zip(entries).take(2) {
            let (direct_status, direct_body) = direct_get(&server, &format!("/{url}")).await;
            assert!(
                status(entry).starts_with(&direct_status.as_u16().to_string()),
                "{url}: entry {} vs direct {direct_status}",
                status(entry)
            );
            assert_eq!(
                outcome_code(entry),
                direct_body["issue"][0]["code"].as_str().unwrap(),
                "{url}"
            );
        }
    }

    #[tokio::test]
    async fn refused_history_parameter_is_that_entrys_400_and_siblings_succeed() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Nguyen").await;

        let urls = [
            "Patient/p1/_history?_at=2020-01-01",
            "Patient/p1",
            "Patient/_history?_sort=_lastUpdated",
            "_history?_list=x",
        ];
        let body = post_batch(&server, gets("batch", &urls)).await;
        let entries = body["entry"].as_array().expect("response entries");

        for index in [0, 2, 3] {
            let entry = &entries[index];
            assert_eq!(status(entry), "400 Bad Request", "{}: {entry}", urls[index]);
            assert_eq!(outcome_code(entry), "not-supported", "{}", urls[index]);
            let (direct_status, direct_body) =
                direct_get(&server, &format!("/{}", urls[index])).await;
            assert_eq!(direct_status, StatusCode::BAD_REQUEST, "{}", urls[index]);
            assert_eq!(
                outcome_text(entry),
                direct_body["issue"][0]["details"]["text"].as_str().unwrap(),
                "{}: the entry says what the direct request says",
                urls[index]
            );
        }
        assert_eq!(status(&entries[1]), "200 OK", "{}", entries[1]);
    }

    #[tokio::test]
    async fn bundle_prefer_handling_strict_applies_to_history_entries() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Nguyen").await;

        let urls = ["Patient/p1/_history?_snice=x"];
        let strict = post_batch_with_prefer(&server, gets("batch", &urls), "handling=strict").await;
        let entry = &strict["entry"][0];
        assert_eq!(status(entry), "400 Bad Request", "{entry}");
        assert_eq!(outcome_code(entry), "invalid");
        assert!(outcome_text(entry).contains("_snice"), "{entry}");

        // An absent Prefer is lenient: the unknown key is ignored.
        let lenient = post_batch(&server, gets("batch", &urls)).await;
        let entry = &lenient["entry"][0];
        assert_eq!(status(entry), "200 OK", "{entry}");
        assert_eq!(entry["resource"]["type"], "history");
    }

    #[tokio::test]
    async fn format_in_a_history_entry_url_is_ignored() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Nguyen").await;

        let body = post_batch(&server, gets("batch", &["Patient/p1/_history?_format=xml"])).await;
        let entry = &body["entry"][0];
        assert_eq!(status(entry), "200 OK", "{entry}");

        // A Bundle response is always JSON, so the entry is the JSON Bundle,
        // not an XML string...
        let history = &entry["resource"];
        assert_eq!(history["resourceType"], "Bundle", "{entry}");
        assert_eq!(history["type"], "history");
        // ...and, as on a direct request, `_format` is carried by its links.
        let self_link = history["link"]
            .as_array()
            .and_then(|links| links.iter().find(|l| l["relation"] == "self"))
            .and_then(|l| l["url"].as_str())
            .expect("a self link");
        assert!(self_link.contains("_format=xml"), "{self_link}");
    }

    #[tokio::test]
    async fn history_url_that_is_none_of_the_shapes_is_a_400_entry() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Nguyen").await;

        let urls = [
            "Patient/p1/_history/1/extra",
            "_history/x",
            "Patient/_history/x",
        ];
        let body = post_batch(&server, gets("batch", &urls)).await;
        for (url, entry) in urls.iter().zip(body["entry"].as_array().unwrap()) {
            assert_eq!(status(entry), "400 Bad Request", "{url}: {entry}");
            assert_eq!(outcome_code(entry), "value", "{url}");
        }
    }

    #[tokio::test]
    async fn unknown_type_history_entry_is_404_like_the_direct_request() {
        let (server, _backend) = create_test_server().await;

        let urls = ["Patinet/_history", "Patinet/1/_history/1"];
        let body = post_batch(&server, gets("batch", &urls)).await;
        for (url, entry) in urls.iter().zip(body["entry"].as_array().unwrap()) {
            assert_eq!(status(entry), "404 Not Found", "{url}: {entry}");
            assert_eq!(outcome_code(entry), "not-supported", "{url}");
            let (direct_status, direct_body) = direct_get(&server, &format!("/{url}")).await;
            assert_eq!(direct_status, StatusCode::NOT_FOUND, "{url}");
            assert_eq!(
                direct_body["issue"][0]["code"], "not-supported",
                "direct {url}"
            );
        }
    }

    #[tokio::test]
    async fn transaction_history_entry_sees_the_bundles_own_writes() {
        let (server, _backend) = create_test_server().await;
        put_patient(&server, "t1", "Old").await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [
                {
                    "request": {"method": "PUT", "url": "Patient/t1"},
                    "resource": {
                        "resourceType": "Patient",
                        "id": "t1",
                        "name": [{"family": "New"}]
                    }
                },
                {"request": {"method": "GET", "url": "Patient/t1/_history"}}
            ]
        });
        let body = post_batch(&server, bundle).await;
        assert_eq!(body["type"], "transaction-response", "{body}");

        let history = &body["entry"][1];
        assert_eq!(status(history), "200 OK", "{history}");
        assert_eq!(history["resource"]["type"], "history");
        assert_eq!(history["resource"]["total"], 2, "{history}");
        assert_eq!(
            history["resource"]["entry"][0]["resource"]["name"][0]["family"], "New",
            "the history runs after the write and lists it first: {history}"
        );
    }

    #[tokio::test]
    async fn transaction_vread_entry_answers_the_version() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "p1").await;

        let body = post_batch(&server, gets("transaction", &["Patient/p1/_history/1"])).await;
        let entry = &body["entry"][0];
        assert_eq!(status(entry), "200 OK", "{entry}");
        assert_eq!(entry["resource"]["name"][0]["family"], "V1", "{entry}");
    }

    #[tokio::test]
    async fn transaction_with_a_refused_history_parameter_fails_whole_before_writing() {
        let (server, _backend) = create_test_server().await;

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [
                {
                    "request": {"method": "POST", "url": "Patient"},
                    "resource": {"resourceType": "Patient", "name": [{"family": "Never"}]}
                },
                {"request": {"method": "GET", "url": "Patient/_history?_at=2020-01-01"}}
            ]
        });
        let response = server
            .post("/")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&bundle)
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::BAD_REQUEST,
            "{}",
            response.text()
        );
        let outcome: Value = response.json();
        assert_eq!(outcome["resourceType"], "OperationOutcome", "{outcome}");
        assert_eq!(outcome["issue"][0]["code"], "not-supported", "{outcome}");

        let (_, search) = direct_get(&server, "/Patient?family=Never").await;
        assert_eq!(
            search["entry"].as_array().map_or(0, Vec::len),
            0,
            "the POST must not have been applied: {search}"
        );
    }

    #[tokio::test]
    async fn transaction_vread_of_a_missing_version_is_that_entrys_404_and_the_writes_commit() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Nguyen").await;

        // A vread entry runs after the writes, as a search entry does (#478),
        // so a version that is not there cannot roll the transaction back.
        let bundle = json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [
                {
                    "request": {"method": "POST", "url": "Patient"},
                    "resource": {"resourceType": "Patient", "name": [{"family": "Kept"}]}
                },
                {"request": {"method": "GET", "url": "Patient/p1/_history/999"}}
            ]
        });
        let body = post_batch(&server, bundle).await;
        assert_eq!(status(&body["entry"][0]), "201 Created", "{body}");
        assert_eq!(status(&body["entry"][1]), "404 Not Found", "{body}");
        assert_eq!(outcome_code(&body["entry"][1]), "not-found");

        let (_, search) = direct_get(&server, "/Patient?family=Kept").await;
        assert_eq!(
            search["entry"].as_array().map_or(0, Vec::len),
            1,
            "the POST committed: {search}"
        );
    }
}
