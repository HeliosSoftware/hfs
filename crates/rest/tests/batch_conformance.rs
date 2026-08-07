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

/// Creates a test server with a known base URL.
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
