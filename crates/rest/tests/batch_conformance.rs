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
        assert_eq!(body["entry"][1]["response"]["status"], "400 Bad Request");
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

        response.assert_status(StatusCode::BAD_REQUEST);
        let outcome: Value = response.json();
        assert_eq!(outcome["resourceType"], "OperationOutcome");
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

    #[tokio::test]
    async fn if_match_on_a_conditional_entry_is_400_and_writes_nothing() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;

        let mut bundle = conditional_put("Patient?identifier=http://example.org|12345", "Stale");
        bundle["entry"][0]["request"]["ifMatch"] = json!("W/\"1\"");
        let body = post_batch(&server, bundle).await;

        assert_eq!(
            body["entry"][0]["response"]["status"], "400 Bad Request",
            "{body}"
        );
        assert_eq!(family_of(&backend, "p1").await, "Nguyen");
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

    /// A `_` parameter that shapes results rather than matching is refused:
    /// dropped, it would match every resource of the type (#866).
    #[tokio::test]
    async fn a_result_parameter_in_transaction_criteria_is_400_and_writes_nothing() {
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
                    .is_some_and(|t| t.contains("not a search criterion")),
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
        assert_eq!(family_of(&backend, "p1").await, "Nguyen");
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
                .is_some_and(|t| t.contains("unknown search parameter")),
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
                .is_some_and(|t| t.contains("invalid criteria")),
            "{body}"
        );
        assert_eq!(patient_count(&backend).await, before);
    }

    #[tokio::test]
    async fn if_match_on_a_transaction_conditional_entry_is_400() {
        let (server, backend) = create_test_server().await;
        seed_patient_with_identifier(&backend, "p1", "Nguyen").await;

        let mut put = put_entry(CRITERIA_URL, "Guarded");
        put["request"]["ifMatch"] = json!("W/\"1\"");
        let response = post_bundle(&server, transaction(vec![sibling_post(), put])).await;

        response.assert_status(StatusCode::BAD_REQUEST);
        assert_eq!(family_of(&backend, "p1").await, "Nguyen");
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

    /// PATCH is declined at 501 — the status all three backends already return
    /// from inside a transaction, and the one both READMEs already claimed.
    #[tokio::test]
    async fn batch_patch_is_declined_at_501_and_changes_nothing() {
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

        assert_eq!(
            body["entry"][0]["response"]["status"],
            "501 Not Implemented"
        );
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

    /// A PATCH transaction is declined before anything executes, so a sibling
    /// create in the same bundle must not have landed.
    #[tokio::test]
    async fn a_transaction_patch_is_declined_intact_at_501() {
        let (server, backend) = create_test_server().await;
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

        response.assert_status(StatusCode::NOT_IMPLEMENTED);
        let body: Value = response.json();
        assert_eq!(body["resourceType"], "OperationOutcome");
        assert_eq!(body["issue"][0]["code"], "not-supported");
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
    #[tokio::test]
    async fn the_two_arms_agree_on_the_refusal_status() {
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
