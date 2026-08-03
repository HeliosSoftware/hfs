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
    create_test_server_with(|_| {}).await
}

/// Creates a test server, letting the caller adjust the [`ServerConfig`] first.
///
/// Used by the tests that need a non-default flag (e.g. `require_if_match`).
/// The flag is set on the struct rather than through its environment variable:
/// `HFS_REQUIRE_IF_MATCH` is read by clap at config-parse time, and
/// `std::env::set_var` would race every other test in this binary.
async fn create_test_server_with(
    adjust: impl FnOnce(&mut ServerConfig),
) -> (TestServer, Arc<SqliteBackend>) {
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

    let mut config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    };
    adjust(&mut config);

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

    /// A deleted resource is brought back to life by a subsequent update
    /// (<https://hl7.org/fhir/http.html#delete>), so `PUT` onto a deleted id
    /// must restore it rather than fail with 410 Gone.
    #[tokio::test]
    async fn test_update_after_delete_restores_resource() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        server
            .delete("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        // Read of the deleted resource is 410 Gone
        server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await
            .assert_status(StatusCode::GONE);

        let restored = json!({
            "resourceType": "Patient",
            "id": "patient-1",
            "name": [{"family": "Restored"}]
        });

        let response = server
            .put("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&restored)
            .await;

        response.assert_status(StatusCode::CREATED);

        // The resource is readable again with the new content
        let read = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        read.assert_status_ok();
        let body: Value = read.json();
        assert_eq!(body["name"][0]["family"], "Restored");

        // The restore continues the version chain rather than resetting to "1":
        // v1 create, v2 delete, v3 restore.
        let etag = read
            .headers()
            .get("etag")
            .expect("restored resource should have an ETag")
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(
            etag, "W/\"3\"",
            "restored resource should continue the version chain"
        );
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

    // ---------------------------------------------------------------------
    // Issue #311 — `If-Match` is a comma-separated list (RFC 9110 §13.1.1)
    // ---------------------------------------------------------------------

    /// The headline bug: a multi-valued `If-Match` is satisfied when ANY listed
    /// tag matches. The whole field value used to be compared as one opaque
    /// string, so this was a permanent 412 on a request that must succeed.
    #[tokio::test]
    async fn test_multi_valued_if_match_succeeds_on_any_member() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let etag = current_etag(&server, "/Patient/patient-1").await;

        let updated = json!({
            "resourceType": "Patient",
            "id": "patient-1",
            "name": [{"family": "UpdatedSmith"}]
        });

        // A stale tag first, then the current one — must still match.
        let list = format!("W/\"99\", {etag}");
        let response = server
            .put("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .add_header(IF_MATCH, HeaderValue::from_str(&list).unwrap())
            .json(&updated)
            .await;

        response.assert_status_ok();
    }

    /// A list in which nothing matches must still be refused.
    #[tokio::test]
    async fn test_multi_valued_if_match_fails_when_none_match() {
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
            .add_header(IF_MATCH, HeaderValue::from_static("W/\"98\", W/\"99\""))
            .json(&updated)
            .await;

        response.assert_status(StatusCode::PRECONDITION_FAILED);
    }

    /// A client echoing the strong form must match the weak ETag FHIR mandates.
    #[tokio::test]
    async fn test_strong_form_if_match_matches_weak_etag() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let etag = current_etag(&server, "/Patient/patient-1").await;
        let strong = etag.trim_start_matches("W/").to_string();

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
            .add_header(IF_MATCH, HeaderValue::from_str(&strong).unwrap())
            .json(&updated)
            .await;

        response.assert_status_ok();
    }

    /// `If-Match: *` asserts a *current representation exists*, so on an absent
    /// resource it is a failed precondition — it must NOT license an
    /// update-as-create. This is a deliberate behavior change (RFC 9110
    /// §13.1.1); previously the create proceeded.
    #[tokio::test]
    async fn test_if_match_star_on_absent_resource_is_412() {
        let (server, _backend) = create_test_server().await;

        let created = json!({
            "resourceType": "Patient",
            "id": "does-not-exist",
            "name": [{"family": "New"}]
        });

        let response = server
            .put("/Patient/does-not-exist")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .add_header(IF_MATCH, HeaderValue::from_static("*"))
            .json(&created)
            .await;

        response.assert_status(StatusCode::PRECONDITION_FAILED);
    }

    /// A malformed `If-Match` must fail the precondition rather than be
    /// discarded — discarding it turns a guarded update into an unconditional
    /// overwrite.
    #[tokio::test]
    async fn test_malformed_if_match_is_412_not_an_unconditional_write() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let updated = json!({
            "resourceType": "Patient",
            "id": "patient-1",
            "name": [{"family": "Overwritten"}]
        });

        let response = server
            .put("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            // Unquoted: not a valid entity-tag.
            .add_header(IF_MATCH, HeaderValue::from_static("garbage"))
            .json(&updated)
            .await;

        response.assert_status(StatusCode::PRECONDITION_FAILED);

        // And the stale write must not have landed.
        let read = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        let body: serde_json::Value = read.json();
        assert_eq!(body["name"][0]["family"], "Smith");
    }

    /// PATCH shares the same precondition path as PUT.
    #[tokio::test]
    async fn test_patch_honors_multi_valued_if_match() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let etag = current_etag(&server, "/Patient/patient-1").await;
        let list = format!("W/\"99\", {etag}");

        let patch = json!([
            {"op": "replace", "path": "/name/0/family", "value": "Patched"}
        ]);

        // `.json()` would overwrite the content type with `application/json`,
        // which the patch handler rejects with 415. `.bytes()` leaves it alone,
        // so set it explicitly with `.content_type()`.
        let response = server
            .patch("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .content_type("application/json-patch+json")
            .add_header(IF_MATCH, HeaderValue::from_str(&list).unwrap())
            .bytes(serde_json::to_vec(&patch).unwrap().into())
            .await;

        response.assert_status_ok();

        // The patch must actually have been applied — a 200 alone would also be
        // consistent with the precondition being skipped entirely.
        let read = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        let body: serde_json::Value = read.json();
        assert_eq!(body["name"][0]["family"], "Patched");
    }

    /// A PATCH whose `If-Match` list matches nothing must be refused.
    #[tokio::test]
    async fn test_patch_multi_valued_if_match_fails_when_none_match() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let patch = json!([
            {"op": "replace", "path": "/name/0/family", "value": "Patched"}
        ]);

        let response = server
            .patch("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .content_type("application/json-patch+json")
            .add_header(IF_MATCH, HeaderValue::from_static("W/\"98\", W/\"99\""))
            .bytes(serde_json::to_vec(&patch).unwrap().into())
            .await;

        response.assert_status(StatusCode::PRECONDITION_FAILED);
    }

    /// A PATCH whose `If-Match` cannot be parsed must fail closed with 412 and
    /// leave the resource untouched.
    ///
    /// This is the fail-open half of issue #311 on the PATCH path: an
    /// unparseable value used to be indistinguishable from an absent one, so a
    /// client that asked for optimistic locking and got the syntax wrong had its
    /// write applied unconditionally. The stored-state assertion is the point —
    /// a 412 alone would also be consistent with the patch landing first.
    #[tokio::test]
    async fn test_patch_malformed_if_match_fails_closed() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let patch = json!([
            {"op": "replace", "path": "/name/0/family", "value": "Patched"}
        ]);

        let response = server
            .patch("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .content_type("application/json-patch+json")
            .add_header(IF_MATCH, HeaderValue::from_static("garbage"))
            .bytes(serde_json::to_vec(&patch).unwrap().into())
            .await;

        response.assert_status(StatusCode::PRECONDITION_FAILED);

        let read = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        let body: serde_json::Value = read.json();
        assert_eq!(
            body["name"][0]["family"], "Smith",
            "a malformed precondition must not become an unconditional patch"
        );
    }

    /// Reads the current `ETag` for a resource.
    async fn current_etag(server: &TestServer, path: &str) -> String {
        let response = server
            .get(path)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response
            .headers()
            .get("etag")
            .expect("ETag header")
            .to_str()
            .unwrap()
            .to_string()
    }
}

// =============================================================================
// Conditional Delete Tests (If-Match on DELETE) — issue #312
// =============================================================================

/// `If-Match` on `DELETE [type]/[id]`.
///
/// Before #312 `delete_handler` took no `ConditionalHeaders` extractor at all,
/// so a supplied precondition was unreachable and the resource was destroyed
/// unconditionally.
///
/// Roughly half of these cells pass against the unfixed code. They are kept
/// deliberately, as **controls**: without them, a "fix" that answered 412 to
/// everything would look correct. Each is labelled. The regression evidence is
/// the cells marked REGRESSION, which cannot pass while the header is unread.
///
/// Every REGRESSION cell asserts the resource **survives**, not merely that the
/// status was 412 — a status-only assertion would also pass against a handler
/// that refused *after* deleting.
mod conditional_delete {
    use super::*;

    /// Asserts a resource is still readable, with the family name it was
    /// seeded with. This is the assertion that makes a 412 meaningful.
    async fn assert_still_present(server: &TestServer, id: &str, expect_family: &str) {
        let read = server
            .get(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        read.assert_status(StatusCode::OK);
        let body: serde_json::Value = read.json();
        assert_eq!(
            body["name"][0]["family"], expect_family,
            "a refused delete must leave the resource intact"
        );
    }

    /// Reads the current `ETag` for a resource.
    async fn etag_of(server: &TestServer, id: &str) -> String {
        let response = server
            .get(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response
            .headers()
            .get("etag")
            .expect("ETag header")
            .to_str()
            .unwrap()
            .to_string()
    }

    fn delete_req(server: &TestServer, id: &str) -> axum_test::TestRequest {
        server
            .delete(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
    }

    /// CONTROL — no precondition still deletes. The majority population must
    /// be untouched by #312.
    #[tokio::test]
    async fn absent_if_match_still_deletes() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        delete_req(&server, "patient-1")
            .await
            .assert_status(StatusCode::NO_CONTENT);
    }

    /// CONTROL — a matching tag must not be over-rejected.
    #[tokio::test]
    async fn matching_if_match_deletes() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;
        let etag = etag_of(&server, "patient-1").await;

        delete_req(&server, "patient-1")
            .add_header(IF_MATCH, HeaderValue::from_str(&etag).unwrap())
            .await
            .assert_status(StatusCode::NO_CONTENT);
    }

    /// REGRESSION — the headline case from #312. Client saw v1, someone else
    /// advanced the resource, the client's delete must be refused.
    #[tokio::test]
    async fn stale_if_match_is_412_and_the_resource_survives() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = delete_req(&server, "patient-1")
            .add_header(IF_MATCH, HeaderValue::from_static("W/\"99\""))
            .await;

        response.assert_status(StatusCode::PRECONDITION_FAILED);

        // Not merely a 412 — the resource must still be there.
        assert_still_present(&server, "patient-1", "Smith").await;

        // And it must be *this* precondition failing, not an unrelated 412.
        let body: serde_json::Value = response.json();
        assert_eq!(body["issue"][0]["code"], "conflict");
        let diagnostics = body["issue"][0]["details"]["text"]
            .as_str()
            .unwrap_or_default();
        assert!(
            diagnostics.contains("If-Match"),
            "diagnostics should name the failing precondition, got {diagnostics:?}"
        );
    }

    /// CONTROL — the list semantics #311 established apply here too.
    #[tokio::test]
    async fn multi_valued_if_match_matching_any_member_deletes() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;
        let etag = etag_of(&server, "patient-1").await;
        let list = format!("W/\"99\", {etag}");

        delete_req(&server, "patient-1")
            .add_header(IF_MATCH, HeaderValue::from_str(&list).unwrap())
            .await
            .assert_status(StatusCode::NO_CONTENT);
    }

    /// REGRESSION — a list where nothing matches is still a failed precondition.
    #[tokio::test]
    async fn multi_valued_if_match_matching_nothing_is_412() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        delete_req(&server, "patient-1")
            .add_header(IF_MATCH, HeaderValue::from_static("W/\"98\", W/\"99\""))
            .await
            .assert_status(StatusCode::PRECONDITION_FAILED);

        assert_still_present(&server, "patient-1", "Smith").await;
    }

    /// CONTROL — HFS compares the opaque tag value and ignores the `W/`
    /// weakness flag, so the strong spelling of a weak ETag matches.
    #[tokio::test]
    async fn strong_form_matches_the_weak_etag() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;
        let etag = etag_of(&server, "patient-1").await;
        let strong = etag.trim_start_matches("W/").to_string();

        delete_req(&server, "patient-1")
            .add_header(IF_MATCH, HeaderValue::from_str(&strong).unwrap())
            .await
            .assert_status(StatusCode::NO_CONTENT);
    }

    /// REGRESSION — a malformed value must fail closed. Treating it as absent
    /// is precisely what turns a guarded delete into an unconditional one.
    #[tokio::test]
    async fn malformed_if_match_is_412_not_an_unconditional_delete() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = delete_req(&server, "patient-1")
            // Unquoted: not a valid entity-tag.
            .add_header(IF_MATCH, HeaderValue::from_static("garbage"))
            .await;

        response.assert_status(StatusCode::PRECONDITION_FAILED);
        assert_still_present(&server, "patient-1", "Smith").await;

        let body: serde_json::Value = response.json();
        let diagnostics = body["issue"][0]["details"]["text"]
            .as_str()
            .unwrap_or_default();
        assert!(
            diagnostics.contains("If-Match"),
            "diagnostics should name the failing precondition, got {diagnostics:?}"
        );
    }

    /// REGRESSION — a non-UTF-8 field value is well-formed HTTP (`etagc` admits
    /// `obs-text`) but cannot equal any tag this server issues. It must
    /// evaluate to "no match", never to "absent".
    #[tokio::test]
    async fn non_utf8_if_match_is_412() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        delete_req(&server, "patient-1")
            .add_header(IF_MATCH, HeaderValue::from_bytes(&[0xFF, 0xFE]).unwrap())
            .await
            .assert_status(StatusCode::PRECONDITION_FAILED);

        assert_still_present(&server, "patient-1", "Smith").await;
    }

    /// CONTROL — `*` is satisfied by any current representation.
    #[tokio::test]
    async fn star_deletes_an_existing_resource() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        delete_req(&server, "patient-1")
            .add_header(IF_MATCH, HeaderValue::from_static("*"))
            .await
            .assert_status(StatusCode::NO_CONTENT);
    }

    /// REGRESSION — `*` asserts a current representation EXISTS, so it must
    /// fail against a resource that was never created. 412, not 404: the
    /// precondition is evaluated before the not-found answer, matching the
    /// bundle path (`if_match_suite::star_if_match_requires_an_existing_resource`).
    #[tokio::test]
    async fn star_on_an_absent_resource_is_412() {
        let (server, _backend) = create_test_server().await;

        delete_req(&server, "never-existed")
            .add_header(IF_MATCH, HeaderValue::from_static("*"))
            .await
            .assert_status(StatusCode::PRECONDITION_FAILED);
    }

    /// REGRESSION — a soft-deleted resource has no current representation, so
    /// `*` must not match the tombstone.
    #[tokio::test]
    async fn star_on_a_soft_deleted_resource_is_412() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        delete_req(&server, "patient-1")
            .await
            .assert_status(StatusCode::NO_CONTENT);

        delete_req(&server, "patient-1")
            .add_header(IF_MATCH, HeaderValue::from_static("*"))
            .await
            .assert_status(StatusCode::PRECONDITION_FAILED);
    }

    /// REGRESSION — a concrete tag against a resource that does not exist.
    #[tokio::test]
    async fn concrete_tag_on_an_absent_resource_is_412() {
        let (server, _backend) = create_test_server().await;

        delete_req(&server, "never-existed")
            .add_header(IF_MATCH, HeaderValue::from_static("W/\"1\""))
            .await
            .assert_status(StatusCode::PRECONDITION_FAILED);
    }

    /// REGRESSION — the tombstone's own bumped version must not be matchable.
    /// Comparing against it would let `DELETE If-Match: W/"2"` "succeed"
    /// against an already-deleted resource whose delete bumped it to 2.
    #[tokio::test]
    async fn concrete_tag_on_a_soft_deleted_resource_is_412() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        delete_req(&server, "patient-1")
            .await
            .assert_status(StatusCode::NO_CONTENT);

        for tag in ["W/\"1\"", "W/\"2\"", "W/\"3\""] {
            delete_req(&server, "patient-1")
                .add_header(IF_MATCH, HeaderValue::from_str(tag).unwrap())
                .await
                .assert_status(StatusCode::PRECONDITION_FAILED);
        }
    }

    /// CONTROL — a precondition-less delete of an already-deleted resource
    /// keeps answering 404, exactly as it did before #312.
    ///
    /// This is the cell that pins the trap: evaluating the precondition needs a
    /// `read()`, and `read()` returns `Gone` for a tombstone. Propagating that
    /// instead of mapping it to "no current version" would have silently made
    /// 410 the answer here in every build.
    #[tokio::test]
    async fn absent_if_match_on_a_soft_deleted_resource_is_still_404() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        delete_req(&server, "patient-1")
            .await
            .assert_status(StatusCode::NO_CONTENT);

        delete_req(&server, "patient-1")
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }

    /// REGRESSION — `HFS_REQUIRE_IF_MATCH` was honored only by the update
    /// handler, so a deployment that had opted into mandatory preconditions
    /// still got unconditional deletes.
    #[tokio::test]
    async fn require_if_match_rejects_a_delete_with_no_header() {
        let (server, backend) = create_test_server_with(|c| c.require_if_match = true).await;
        seed_patient(&backend, "patient-1", "Smith").await;

        delete_req(&server, "patient-1")
            .await
            .assert_status(StatusCode::PRECONDITION_FAILED);

        assert_still_present(&server, "patient-1", "Smith").await;
    }

    /// REGRESSION — under `require_if_match`, a malformed value counts as
    /// SUPPLIED and then fails on its own merits. Reporting it as missing would
    /// tell a client to retry with the header it already sent.
    #[tokio::test]
    async fn require_if_match_treats_a_malformed_value_as_supplied() {
        let (server, backend) = create_test_server_with(|c| c.require_if_match = true).await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = delete_req(&server, "patient-1")
            .add_header(IF_MATCH, HeaderValue::from_static("garbage"))
            .await;

        response.assert_status(StatusCode::PRECONDITION_FAILED);
        let body: serde_json::Value = response.json();
        let diagnostics = body["issue"][0]["details"]["text"]
            .as_str()
            .unwrap_or_default();
        assert!(
            diagnostics.contains("Malformed"),
            "a malformed value must be rejected on its merits, not reported as \
             missing; got {diagnostics:?}"
        );
    }

    /// CONTROL — the precondition check must not be hoisted above the
    /// `AuditEvent` immutability guard. Audit records are immutable regardless
    /// of any tag, and reordering these would turn a 405 into a 412 (and make
    /// delete scope an existence probe over the audit trail).
    #[tokio::test]
    async fn audit_event_immutability_outranks_the_precondition() {
        let (server, _backend) = create_test_server().await;

        let response = server
            .delete("/AuditEvent/any-id")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(IF_MATCH, HeaderValue::from_static("W/\"99\""))
            .await;

        response.assert_status(StatusCode::METHOD_NOT_ALLOWED);
    }

    /// The client-facing 412 must not echo the current version.
    ///
    /// `DELETE` is authorized by `FhirOperation::Delete` alone, so a
    /// `system/Patient.d` principal with no read scope reaches this path.
    /// Telling it the versionId would disclose how many times the record has
    /// been amended. Operators still get the value from the debug log.
    #[tokio::test]
    async fn the_412_body_does_not_disclose_the_current_version() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;
        // Advance it so the current version is distinctive.
        let etag = etag_of(&server, "patient-1").await;
        server
            .put("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .add_header(IF_MATCH, HeaderValue::from_str(&etag).unwrap())
            .json(&json!({
                "resourceType": "Patient",
                "id": "patient-1",
                "name": [{"family": "Smith"}]
            }))
            .await
            .assert_status(StatusCode::OK);

        let current = etag_of(&server, "patient-1").await;
        let version = current
            .trim_start_matches("W/")
            .trim_matches('"')
            .to_string();

        let response = delete_req(&server, "patient-1")
            .add_header(IF_MATCH, HeaderValue::from_static("W/\"99\""))
            .await;
        response.assert_status(StatusCode::PRECONDITION_FAILED);

        let body: serde_json::Value = response.json();
        let diagnostics = body["issue"][0]["details"]["text"]
            .as_str()
            .unwrap_or_default();
        assert!(
            !diagnostics.contains(&format!("\"{version}\"")),
            "the 412 must not disclose the current version, got {diagnostics:?}"
        );
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
