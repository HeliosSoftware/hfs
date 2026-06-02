//! REST API conformance tests.
//!
//! Tests standard FHIR REST behaviors:
//! - HTTP status codes (200, 201, 400, 404, 409, 410, 412)
//! - Response headers (ETag, Last-Modified, Location, Content-Type)
//! - Conditional operations (If-Match, If-None-Match, If-None-Exist)
//! - HEAD requests
//! - Content negotiation

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
const IF_MATCH: HeaderName = HeaderName::from_static("if-match");
const IF_NONE_MATCH: HeaderName = HeaderName::from_static("if-none-match");
const IF_NONE_EXIST: HeaderName = HeaderName::from_static("if-none-exist");

/// Creates a test server.
async fn create_test_server() -> (TestServer, Arc<SqliteBackend>) {
    // Configure with data directory to load spec SearchParameters
    // CARGO_MANIFEST_DIR for this test is crates/rest
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

/// Gets the test tenant context.
fn test_tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
}

/// Seeds a patient for testing.
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

// =============================================================================
// HTTP Status Code Tests
// =============================================================================

mod status_codes {
    use super::*;

    #[tokio::test]
    async fn test_read_returns_200() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
    }

    #[tokio::test]
    async fn test_create_returns_201() {
        let (server, _backend) = create_test_server().await;

        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": "NewPatient"}]
        });

        let response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient)
            .await;

        response.assert_status(StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_update_returns_200() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let updated = json!({
            "resourceType": "Patient",
            "id": "patient-1",
            "name": [{"family": "UpdatedSmith"}]
        });

        let response = server
            .put("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&updated)
            .await;

        response.assert_status_ok();
    }

    #[tokio::test]
    async fn test_update_upsert_returns_201() {
        let (server, _backend) = create_test_server().await;

        let patient = json!({
            "resourceType": "Patient",
            "id": "new-patient",
            "name": [{"family": "NewPatient"}]
        });

        let response = server
            .put("/Patient/new-patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient)
            .await;

        response.assert_status(StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_delete_returns_204() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .delete("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn test_read_not_found_returns_404() {
        let (server, _backend) = create_test_server().await;

        let response = server
            .get("/Patient/nonexistent")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_invalid_resource_returns_400() {
        let (server, _backend) = create_test_server().await;

        // Missing resourceType
        let invalid = json!({
            "name": [{"family": "Smith"}]
        });

        let response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&invalid)
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_resource_type_mismatch_returns_400() {
        let (server, _backend) = create_test_server().await;

        // resourceType doesn't match URL
        let wrong_type = json!({
            "resourceType": "Observation",
            "status": "final"
        });

        let response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&wrong_type)
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
    }
}

// =============================================================================
// Response Header Tests
// =============================================================================

mod response_headers {
    use super::*;

    #[tokio::test]
    async fn test_read_returns_etag() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();

        let etag = response.headers().get("etag");
        assert!(etag.is_some(), "Response should have ETag header");

        let etag_value = etag.unwrap().to_str().unwrap();
        assert!(
            etag_value.starts_with("W/\""),
            "ETag should be weak: {}",
            etag_value
        );
    }

    #[tokio::test]
    async fn test_read_returns_last_modified() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();

        let last_modified = response.headers().get("last-modified");
        assert!(
            last_modified.is_some(),
            "Response should have Last-Modified header"
        );
    }

    #[tokio::test]
    async fn test_read_returns_content_type() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();

        let content_type = response.headers().get("content-type");
        assert!(
            content_type.is_some(),
            "Response should have Content-Type header"
        );

        let ct_value = content_type.unwrap().to_str().unwrap();
        assert!(
            ct_value.contains("application/fhir+json") || ct_value.contains("application/json"),
            "Content-Type should be FHIR JSON: {}",
            ct_value
        );
    }

    #[tokio::test]
    async fn test_create_returns_location() {
        let (server, _backend) = create_test_server().await;

        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": "NewPatient"}]
        });

        let response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient)
            .await;

        response.assert_status(StatusCode::CREATED);

        let location = response.headers().get("location");
        assert!(location.is_some(), "Response should have Location header");

        let location_value = location.unwrap().to_str().unwrap();
        assert!(
            location_value.contains("/Patient/"),
            "Location should contain resource path: {}",
            location_value
        );
    }
}

// =============================================================================
// Conditional Read Tests (If-None-Match)
// =============================================================================

mod conditional_read {
    use super::*;

    #[tokio::test]
    async fn test_if_none_match_returns_304() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        // First, get the ETag
        let response1 = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response1.assert_status_ok();
        let etag = response1
            .headers()
            .get("etag")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        // Second request with If-None-Match
        let response2 = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(IF_NONE_MATCH, HeaderValue::from_str(&etag).unwrap())
            .await;

        response2.assert_status(StatusCode::NOT_MODIFIED);
    }

    #[tokio::test]
    async fn test_if_none_match_star_returns_304() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(IF_NONE_MATCH, HeaderValue::from_static("*"))
            .await;

        response.assert_status(StatusCode::NOT_MODIFIED);
    }

    #[tokio::test]
    async fn test_if_none_match_different_returns_200() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(IF_NONE_MATCH, HeaderValue::from_static("W/\"different\""))
            .await;

        response.assert_status_ok();
    }
}

// =============================================================================
// Conditional Update Tests (If-Match)
// =============================================================================

mod conditional_update {
    use super::*;

    #[tokio::test]
    async fn test_if_match_success() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        // Get current ETag
        let response1 = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        let etag = response1
            .headers()
            .get("etag")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        // Update with correct ETag
        let updated = json!({
            "resourceType": "Patient",
            "id": "patient-1",
            "name": [{"family": "UpdatedSmith"}]
        });

        let response2 = server
            .put("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .add_header(IF_MATCH, HeaderValue::from_str(&etag).unwrap())
            .json(&updated)
            .await;

        response2.assert_status_ok();
    }

    #[tokio::test]
    async fn test_if_match_failure_returns_412() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        // Update with wrong ETag
        let updated = json!({
            "resourceType": "Patient",
            "id": "patient-1",
            "name": [{"family": "UpdatedSmith"}]
        });

        let response = server
            .put("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .add_header(IF_MATCH, HeaderValue::from_static("W/\"wrong-version\""))
            .json(&updated)
            .await;

        response.assert_status(StatusCode::PRECONDITION_FAILED);
    }

    #[tokio::test]
    async fn test_if_match_star_success() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let updated = json!({
            "resourceType": "Patient",
            "id": "patient-1",
            "name": [{"family": "UpdatedSmith"}]
        });

        let response = server
            .put("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .add_header(IF_MATCH, HeaderValue::from_static("*"))
            .json(&updated)
            .await;

        response.assert_status_ok();
    }
}

// =============================================================================
// Conditional Create Tests (If-None-Exist)
// =============================================================================

mod conditional_create {
    use super::*;

    #[tokio::test]
    async fn test_if_none_exist_creates_when_no_match() {
        let (server, _backend) = create_test_server().await;

        let patient = json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://example.org/mrn", "value": "UNIQUE123"}],
            "name": [{"family": "Unique"}]
        });

        let response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .add_header(
                IF_NONE_EXIST,
                HeaderValue::from_static("identifier=UNIQUE123"),
            )
            .json(&patient)
            .await;

        response.assert_status(StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_if_none_exist_returns_existing_when_match() {
        let (server, backend) = create_test_server().await;

        // Create existing patient with identifier
        let tenant = test_tenant();
        let existing = json!({
            "resourceType": "Patient",
            "id": "existing-1",
            "identifier": [{"system": "http://example.org/mrn", "value": "EXISTING123"}],
            "name": [{"family": "Existing"}]
        });
        backend
            .create(&tenant, "Patient", existing, FhirVersion::R4)
            .await
            .unwrap();

        // Try to create another with same identifier
        let new_patient = json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://example.org/mrn", "value": "EXISTING123"}],
            "name": [{"family": "NewPatient"}]
        });

        let response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .add_header(
                IF_NONE_EXIST,
                HeaderValue::from_static("identifier=EXISTING123"),
            )
            .json(&new_patient)
            .await;

        // Should return 200 OK with existing resource
        response.assert_status_ok();

        let body: Value = response.json();
        assert_eq!(body["id"], "existing-1");
    }
}

// =============================================================================
// HEAD Request Tests
// =============================================================================

mod head_requests {
    use super::*;

    #[tokio::test]
    async fn test_head_returns_headers_no_body() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .method(axum::http::Method::HEAD, "/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();

        // Should have headers
        assert!(response.headers().get("etag").is_some());
        assert!(response.headers().get("last-modified").is_some());

        // Body should be empty
        let body = response.text();
        assert!(body.is_empty(), "HEAD response should have empty body");
    }

    #[tokio::test]
    async fn test_head_not_found_returns_404() {
        let (server, _backend) = create_test_server().await;

        let response = server
            .method(axum::http::Method::HEAD, "/Patient/nonexistent")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_head_conditional_returns_304() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        // First get the ETag
        let response1 = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        let etag = response1
            .headers()
            .get("etag")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        // HEAD with If-None-Match
        let response2 = server
            .method(axum::http::Method::HEAD, "/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(IF_NONE_MATCH, HeaderValue::from_str(&etag).unwrap())
            .await;

        response2.assert_status(StatusCode::NOT_MODIFIED);
    }
}

// =============================================================================
// Error Response Tests
// =============================================================================

mod error_responses {
    use super::*;

    #[tokio::test]
    async fn test_404_returns_operation_outcome() {
        let (server, _backend) = create_test_server().await;

        let response = server
            .get("/Patient/nonexistent")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::NOT_FOUND);

        let body: Value = response.json();
        assert_eq!(body["resourceType"], "OperationOutcome");
        assert!(body["issue"].is_array());
    }

    #[tokio::test]
    async fn test_400_returns_operation_outcome() {
        let (server, _backend) = create_test_server().await;

        let invalid = json!({
            "invalid": "resource"
        });

        let response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&invalid)
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);

        let body: Value = response.json();
        assert_eq!(body["resourceType"], "OperationOutcome");
    }
}

// =============================================================================
// Capability Statement Tests
// =============================================================================

mod capabilities {
    use super::*;

    #[tokio::test]
    async fn test_metadata_endpoint() {
        let (server, _backend) = create_test_server().await;

        let response = server.get("/metadata").await;

        response.assert_status_ok();

        let body: Value = response.json();
        assert_eq!(body["resourceType"], "CapabilityStatement");
        assert!(body["fhirVersion"].is_string());
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let (server, _backend) = create_test_server().await;

        let response = server.get("/health").await;

        response.assert_status_ok();
    }
}

// =============================================================================
// Delete History Tests (FHIR v6.0.0 Trial Use)
// =============================================================================

mod delete_history {
    use super::*;

    #[tokio::test]
    async fn test_delete_instance_history() {
        let (server, backend) = create_test_server().await;

        // Create a patient and update it to create history
        let patient = json!({
            "resourceType": "Patient",
            "id": "history-test",
            "name": [{"family": "Original"}]
        });

        backend
            .create(&test_tenant(), "Patient", patient, FhirVersion::R4)
            .await
            .unwrap();

        // Update to create version 2
        let update1 = json!({
            "resourceType": "Patient",
            "id": "history-test",
            "name": [{"family": "Updated1"}]
        });

        server
            .put("/Patient/history-test")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&update1)
            .await;

        // Update again to create version 3
        let update2 = json!({
            "resourceType": "Patient",
            "id": "history-test",
            "name": [{"family": "Updated2"}]
        });

        server
            .put("/Patient/history-test")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&update2)
            .await;

        // Delete the instance history
        let response = server
            .delete("/Patient/history-test/_history")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        // Should return 200 OK with OperationOutcome
        response.assert_status_ok();
        let body: Value = response.json();
        assert_eq!(body["resourceType"], "OperationOutcome");

        // The current version should still be accessible
        let read_response = server
            .get("/Patient/history-test")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        read_response.assert_status_ok();
        let patient: Value = read_response.json();
        assert_eq!(patient["resourceType"], "Patient");
        assert_eq!(patient["id"], "history-test");
    }

    #[tokio::test]
    async fn test_delete_instance_history_not_found() {
        let (server, _backend) = create_test_server().await;

        // Try to delete history for a resource that doesn't exist
        let response = server
            .delete("/Patient/nonexistent/_history")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_specific_version() {
        let (server, backend) = create_test_server().await;

        // Create a patient and update it to create history
        let patient = json!({
            "resourceType": "Patient",
            "id": "version-delete-test",
            "name": [{"family": "Original"}]
        });

        backend
            .create(&test_tenant(), "Patient", patient, FhirVersion::R4)
            .await
            .unwrap();

        // Update to create version 2
        let update1 = json!({
            "resourceType": "Patient",
            "id": "version-delete-test",
            "name": [{"family": "Updated1"}]
        });

        server
            .put("/Patient/version-delete-test")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&update1)
            .await;

        // Update again to create version 3
        let update2 = json!({
            "resourceType": "Patient",
            "id": "version-delete-test",
            "name": [{"family": "Updated2"}]
        });

        server
            .put("/Patient/version-delete-test")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&update2)
            .await;

        // Delete version 1 (historical)
        let response = server
            .delete("/Patient/version-delete-test/_history/1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        // Should return 204 No Content
        response.assert_status(StatusCode::NO_CONTENT);

        // Trying to vread the deleted version should return 404 or 501 (if vread not implemented)
        let vread_response = server
            .get("/Patient/version-delete-test/_history/1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        let vread_status = vread_response.status_code();
        assert!(
            vread_status == StatusCode::NOT_FOUND || vread_status == StatusCode::NOT_IMPLEMENTED,
            "Expected 404 or 501, got {}",
            vread_status
        );

        // Current version should still be accessible
        let read_response = server
            .get("/Patient/version-delete-test")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        read_response.assert_status_ok();
    }

    #[tokio::test]
    async fn test_delete_current_version_fails() {
        let (server, backend) = create_test_server().await;

        // Create a patient and update it
        let patient = json!({
            "resourceType": "Patient",
            "id": "current-delete-test",
            "name": [{"family": "Test"}]
        });

        backend
            .create(&test_tenant(), "Patient", patient, FhirVersion::R4)
            .await
            .unwrap();

        // Update to create version 2
        let update = json!({
            "resourceType": "Patient",
            "id": "current-delete-test",
            "name": [{"family": "Updated"}]
        });

        server
            .put("/Patient/current-delete-test")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&update)
            .await;

        // Try to delete the current version (2)
        let response = server
            .delete("/Patient/current-delete-test/_history/2")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        // Should fail with 400 Bad Request (can't delete current version)
        response.assert_status(StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_delete_version_not_found() {
        let (server, backend) = create_test_server().await;

        // Create a patient
        let patient = json!({
            "resourceType": "Patient",
            "id": "version-not-found-test",
            "name": [{"family": "Test"}]
        });

        backend
            .create(&test_tenant(), "Patient", patient, FhirVersion::R4)
            .await
            .unwrap();

        // Try to delete a version that doesn't exist
        let response = server
            .delete("/Patient/version-not-found-test/_history/999")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_version_resource_not_found() {
        let (server, _backend) = create_test_server().await;

        // Try to delete a version for a resource that doesn't exist
        let response = server
            .delete("/Patient/nonexistent/_history/1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::NOT_FOUND);
    }
}

// =============================================================================
// History reads: instance / type / system history + vread
// =============================================================================

mod history_reads {
    use super::*;

    /// Creates a Patient and updates it twice, yielding versions 1, 2, 3.
    async fn seed_versioned_patient(server: &TestServer, id: &str) {
        let create = json!({
            "resourceType": "Patient",
            "id": id,
            "name": [{"family": "V1"}]
        });
        server
            .put(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&create)
            .await;

        for family in ["V2", "V3"] {
            let update = json!({
                "resourceType": "Patient",
                "id": id,
                "name": [{"family": family}]
            });
            server
                .put(&format!("/Patient/{id}"))
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .add_header(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/fhir+json"),
                )
                .json(&update)
                .await;
        }
    }

    #[tokio::test]
    async fn test_instance_history_returns_all_versions() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-inst").await;

        let response = server
            .get("/Patient/hist-inst/_history")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let bundle: Value = response.json();
        assert_eq!(bundle["resourceType"], "Bundle");
        assert_eq!(bundle["type"], "history");
        assert_eq!(bundle["total"], 3);

        let entries = bundle["entry"].as_array().expect("entry array");
        assert_eq!(entries.len(), 3);

        // Each entry carries request + response metadata with a weak ETag.
        for entry in entries {
            assert!(entry["request"]["method"].is_string());
            assert!(
                entry["response"]["etag"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("W/\"")
            );
        }
    }

    #[tokio::test]
    async fn test_instance_history_not_found() {
        let (server, _backend) = create_test_server().await;

        let response = server
            .get("/Patient/does-not-exist/_history")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_instance_history_count_limits_entries() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-count").await;

        let response = server
            .get("/Patient/hist-count/_history?_count=1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let bundle: Value = response.json();
        let entries = bundle["entry"].as_array().expect("entry array");
        assert_eq!(
            entries.len(),
            1,
            "expected _count=1 to return a single entry"
        );
    }

    #[tokio::test]
    async fn test_instance_history_invalid_since_is_bad_request() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-since").await;

        let response = server
            .get("/Patient/hist-since/_history?_since=not-a-date")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_type_history_returns_bundle() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-type-a").await;
        seed_versioned_patient(&server, "hist-type-b").await;

        let response = server
            .get("/Patient/_history")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let bundle: Value = response.json();
        assert_eq!(bundle["type"], "history");
        let entries = bundle["entry"].as_array().expect("entry array");
        // Two resources × three versions each.
        assert!(
            entries.len() >= 6,
            "expected >= 6 entries, got {}",
            entries.len()
        );
    }

    #[tokio::test]
    async fn test_system_history_returns_bundle() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-sys").await;

        let response = server
            .get("/_history")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let bundle: Value = response.json();
        assert_eq!(bundle["resourceType"], "Bundle");
        assert_eq!(bundle["type"], "history");
        assert!(!bundle["entry"].as_array().expect("entry array").is_empty());
    }

    #[tokio::test]
    async fn test_vread_returns_specific_version() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "vread-test").await;

        // Version 1 was the original ("V1"); version 3 is current ("V3").
        let v1 = server
            .get("/Patient/vread-test/_history/1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        v1.assert_status_ok();
        let v1_body: Value = v1.json();
        assert_eq!(v1_body["resourceType"], "Patient");
        assert_eq!(v1_body["id"], "vread-test");
        assert_eq!(v1_body["name"][0]["family"], "V1");

        let v3 = server
            .get("/Patient/vread-test/_history/3")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        v3.assert_status_ok();
        let v3_body: Value = v3.json();
        assert_eq!(v3_body["name"][0]["family"], "V3");
    }

    #[tokio::test]
    async fn test_vread_unknown_version_not_found() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "vread-missing").await;

        let response = server
            .get("/Patient/vread-missing/_history/999")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::NOT_FOUND);
    }
}
