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
    use chrono::{DateTime, SecondsFormat, Utc};
    use helios_persistence::types::{CursorValue, PageCursor};
    use std::time::Duration;

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

    /// A vread of the version that deleted the resource is `410 Gone`, and the
    /// versions before it are still readable.
    ///
    /// FHIR's vread interaction (https://hl7.org/fhir/http.html#vread) answers a
    /// deleted version with `410`, and the history Bundle has always agreed —
    /// `history_entry_to_json` gives a `Delete` entry `request.method = DELETE`
    /// and no `resource` at all. This used to return `200` with whatever body
    /// the resource held before the delete, which is not a version it ever had.
    ///
    /// It is also what lets the PostgreSQL delete path stop copying that body
    /// into `resource_history`.
    #[tokio::test]
    async fn test_vread_of_deleted_version_is_gone() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "vread-deleted").await;

        server
            .delete("/Patient/vread-deleted")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        // v4 is the tombstone the DELETE wrote.
        server
            .get("/Patient/vread-deleted/_history/4")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await
            .assert_status(StatusCode::GONE);

        // The versions the resource actually had are unaffected.
        let v3 = server
            .get("/Patient/vread-deleted/_history/3")
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

    // -------------------------------------------------------------------------
    // `_at` is refused, not silently ignored (#2374)
    // -------------------------------------------------------------------------

    const PREFER: HeaderName = HeaderName::from_static("prefer");

    /// Every `Prefer: handling` a client can send, plus none at all. The refusal
    /// of `_at` must not depend on it.
    const PREFER_HANDLINGS: [Option<&'static str>; 3] =
        [None, Some("handling=strict"), Some("handling=lenient")];

    /// GETs `url` as the test tenant, with an optional `Prefer` header.
    async fn get_history(
        server: &TestServer,
        url: &str,
        prefer: Option<&'static str>,
    ) -> axum_test::TestResponse {
        let mut request = server
            .get(url)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"));
        if let Some(prefer) = prefer {
            request = request.add_header(PREFER, HeaderValue::from_static(prefer));
        }
        request.await
    }

    /// Asserts the response is a `400` OperationOutcome with severity `error`
    /// and returns it.
    ///
    /// The body is parsed from text so that a plain-text `400` (what axum's
    /// `Query` extractor answered before #2381) fails with the body in the
    /// message instead of a JSON decode error.
    fn history_outcome(response: &axum_test::TestResponse, context: &str) -> Value {
        assert_eq!(
            response.status_code(),
            StatusCode::BAD_REQUEST,
            "{context}: expected 400, got {} with body {}",
            response.status_code(),
            response.text()
        );
        let text = response.text();
        let outcome: Value = serde_json::from_str(&text).unwrap_or_else(|_| {
            panic!("{context}: expected an OperationOutcome body, got: {text}")
        });
        assert_eq!(
            outcome["resourceType"], "OperationOutcome",
            "{context}: body was {outcome}"
        );
        assert_eq!(
            outcome["issue"][0]["severity"], "error",
            "{context}: body was {outcome}"
        );
        outcome
    }

    /// Asserts the response is a `400` OperationOutcome whose first issue has
    /// the given code and whose `details.text` contains `needle`.
    ///
    /// The message is read from `details.text`, which is where `RestError`
    /// renders it; there is no `diagnostics`.
    fn assert_history_outcome(
        response: &axum_test::TestResponse,
        code: &str,
        needle: &str,
        context: &str,
    ) {
        let outcome = history_outcome(response, context);
        assert_eq!(
            outcome["issue"][0]["code"], code,
            "{context}: body was {outcome}"
        );
        let text = outcome["issue"][0]["details"]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{context}: details.text is not a string: {outcome}"));
        assert!(
            text.contains(needle),
            "{context}: details.text should contain {needle:?}, got: {text}"
        );
    }

    /// Asserts the response is the refusal of `_at`: a `400` OperationOutcome
    /// with issue code `not-supported` whose `details.text` names `_at` and
    /// points to `_since`.
    fn assert_at_refused(response: &axum_test::TestResponse, context: &str) {
        assert_history_outcome(response, "not-supported", "_at", context);
        let outcome: Value = serde_json::from_str(&response.text()).expect("outcome body");
        let text = outcome["issue"][0]["details"]["text"]
            .as_str()
            .expect("details.text is a string");
        assert!(
            text.contains("_since"),
            "{context}: details.text should point to _since, got: {text}"
        );
    }

    /// The instance, type and system history URLs (no query string) for a
    /// seeded Patient.
    fn history_urls(id: &str) -> [(&'static str, String); 3] {
        [
            ("instance history", format!("/Patient/{id}/_history")),
            ("type history", "/Patient/_history".to_string()),
            ("system history", "/_history".to_string()),
        ]
    }

    /// GETs `url` and asserts `200`, returning the body as JSON.
    async fn get_history_ok(
        server: &TestServer,
        url: &str,
        prefer: Option<&'static str>,
        context: &str,
    ) -> Value {
        let response = get_history(server, url, prefer).await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{context}: expected 200, got {} with body {}",
            response.status_code(),
            response.text()
        );
        response.json()
    }

    // -------------------------------------------------------------------------
    // Paging helpers (#2382)
    // -------------------------------------------------------------------------

    /// The `base_url` of [`create_test_server`], which prefixes every link.
    const BASE: &str = "http://localhost:8080";

    /// PUTs `resource` to `/{resourceType}/{id}` as the test tenant and asserts
    /// the `200` / `201`.
    async fn put_resource(server: &TestServer, resource: Value) {
        let resource_type = resource["resourceType"]
            .as_str()
            .expect("resourceType")
            .to_string();
        let id = resource["id"].as_str().expect("id").to_string();
        let response = server
            .put(&format!("/{resource_type}/{id}"))
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&resource)
            .await;
        assert!(
            matches!(response.status_code(), StatusCode::OK | StatusCode::CREATED),
            "PUT {resource_type}/{id}: expected 200 or 201, got {} with body {}",
            response.status_code(),
            response.text()
        );
    }

    /// A Patient with the given id and family name.
    fn patient(id: &str, family: &str) -> Value {
        json!({"resourceType": "Patient", "id": id, "name": [{"family": family}]})
    }

    /// The url of the `relation` link of `bundle`, if it has one.
    fn link<'a>(bundle: &'a Value, relation: &str) -> Option<&'a str> {
        bundle["link"]
            .as_array()?
            .iter()
            .find(|l| l["relation"] == relation)
            .and_then(|l| l["url"].as_str())
    }

    /// The decoded query pairs of an absolute URL.
    fn link_query(url: &str) -> Vec<(String, String)> {
        url::Url::parse(url)
            .unwrap_or_else(|e| panic!("link {url:?} is not a URL: {e}"))
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    }

    /// The value of query parameter `key` on the `relation` link of `bundle`.
    fn link_param(bundle: &Value, relation: &str, key: &str) -> Option<String> {
        link_query(link(bundle, relation)?)
            .into_iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// `bundle` without its `link` array.
    fn without_link(mut bundle: Value) -> Value {
        bundle
            .as_object_mut()
            .expect("bundle is an object")
            .remove("link");
        bundle
    }

    /// Identifies one version: the entry's `fullUrl` without the base (so
    /// `Patient/{id}`) and the version from its `W/"n"` ETag.
    fn version_key(entry: &Value) -> (String, String) {
        let full_url = entry["fullUrl"].as_str().expect("fullUrl");
        let resource = full_url
            .strip_prefix(&format!("{BASE}/"))
            .unwrap_or_else(|| panic!("fullUrl {full_url:?} does not start with {BASE}/"));
        let etag = entry["response"]["etag"].as_str().expect("etag");
        let version = etag
            .strip_prefix("W/\"")
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or_else(|| panic!("etag {etag:?} is not W/\"n\""));
        (resource.to_string(), version.to_string())
    }

    /// The entries of one history page.
    fn entries(page: &Value) -> &Vec<Value> {
        page["entry"].as_array().expect("entry array")
    }

    /// Follows `next` links from `first` to the end and returns every page.
    ///
    /// Each page must carry a `self` link under [`BASE`]. The walk panics after
    /// 20 pages, so a `next` that never ends fails the test instead of hanging.
    async fn walk_history(
        server: &TestServer,
        first: &str,
        prefer: Option<&'static str>,
        context: &str,
    ) -> Vec<Value> {
        let mut pages = Vec::new();
        let mut url = first.to_string();
        loop {
            assert!(
                pages.len() < 20,
                "{context}: the walk did not end after 20 pages"
            );
            let page_context = format!("{context}, page {}", pages.len() + 1);
            let page = get_history_ok(server, &url, prefer, &page_context).await;
            let self_url = link(&page, "self")
                .unwrap_or_else(|| panic!("{page_context}: no self link in {page}"));
            assert!(
                self_url.starts_with(BASE),
                "{page_context}: self {self_url:?} should start with {BASE}"
            );
            let next = link(&page, "next").map(|next| {
                next.strip_prefix(BASE)
                    .unwrap_or_else(|| {
                        panic!("{page_context}: next {next:?} should start with {BASE}")
                    })
                    .to_string()
            });
            pages.push(page);
            match next {
                Some(next) => url = next,
                None => return pages,
            }
        }
    }

    /// Asserts every page but the last has a `next` carrying `_count=count` and
    /// a `_cursor`, the last has none, and every page after the first has a
    /// `self` carrying a `_cursor` while the first has none.
    fn assert_page_links(pages: &[Value], count: usize, context: &str) {
        let count = count.to_string();
        for (i, page) in pages.iter().enumerate() {
            let context = format!("{context}, page {}", i + 1);
            assert_eq!(
                link_param(page, "self", "_count").as_deref(),
                Some(count.as_str()),
                "{context}: self _count"
            );
            assert_eq!(
                link_param(page, "self", "_cursor").is_some(),
                i > 0,
                "{context}: only pages after the first have a cursor on self"
            );
            if i + 1 < pages.len() {
                assert_eq!(
                    link_param(page, "next", "_count").as_deref(),
                    Some(count.as_str()),
                    "{context}: next _count"
                );
                assert!(
                    link_param(page, "next", "_cursor").is_some(),
                    "{context}: next should carry a _cursor"
                );
            } else {
                assert!(
                    link(page, "next").is_none(),
                    "{context}: the last page must not have a next link"
                );
            }
        }
    }

    /// Every entry of every page, in order.
    fn all_entries(pages: &[Value]) -> Vec<&Value> {
        pages.iter().flat_map(entries).collect()
    }

    /// Asserts `lastModified` never increases along the walk, which is what
    /// newest-first paging means.
    fn assert_newest_first(pages: &[Value], context: &str) {
        let times: Vec<DateTime<Utc>> = all_entries(pages)
            .iter()
            .map(|entry| {
                let text = entry["response"]["lastModified"]
                    .as_str()
                    .expect("lastModified");
                DateTime::parse_from_rfc3339(text)
                    .unwrap_or_else(|e| panic!("{context}: lastModified {text:?}: {e}"))
                    .with_timezone(&Utc)
            })
            .collect();
        assert!(
            times.windows(2).all(|pair| pair[0] >= pair[1]),
            "{context}: lastModified should be non-increasing, got {times:?}"
        );
    }

    /// The `(resource, version)` of every entry of the walk, sorted.
    fn sorted_keys(pages: &[Value]) -> Vec<(String, String)> {
        let mut keys: Vec<(String, String)> =
            all_entries(pages).into_iter().map(version_key).collect();
        keys.sort();
        keys
    }

    /// `(resource, version)` for `Type/id` at each of `versions`, sorted.
    fn expected_keys(resources: &[&str], versions: &[u32]) -> Vec<(String, String)> {
        let mut keys: Vec<(String, String)> = resources
            .iter()
            .flat_map(|resource| {
                versions
                    .iter()
                    .map(move |version| (resource.to_string(), version.to_string()))
            })
            .collect();
        keys.sort();
        keys
    }

    #[tokio::test]
    async fn test_instance_history_at_is_refused_under_every_prefer_handling() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-at-inst").await;

        for prefer in PREFER_HANDLINGS {
            let response = get_history(
                &server,
                "/Patient/hist-at-inst/_history?_at=2020-01-01T00:00:00Z",
                prefer,
            )
            .await;
            assert_at_refused(&response, &format!("instance history, Prefer {prefer:?}"));
        }
    }

    #[tokio::test]
    async fn test_type_history_at_is_refused_under_every_prefer_handling() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-at-type").await;

        for prefer in PREFER_HANDLINGS {
            let response = get_history(
                &server,
                "/Patient/_history?_at=2020-01-01T00:00:00Z",
                prefer,
            )
            .await;
            assert_at_refused(&response, &format!("type history, Prefer {prefer:?}"));
        }
    }

    #[tokio::test]
    async fn test_system_history_at_is_refused_under_every_prefer_handling() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-at-sys").await;

        for prefer in PREFER_HANDLINGS {
            let response = get_history(&server, "/_history?_at=2020-01-01T00:00:00Z", prefer).await;
            assert_at_refused(&response, &format!("system history, Prefer {prefer:?}"));
        }
    }

    /// The refusal never parses the value: an empty, bare, malformed or
    /// otherwise odd `_at` is refused exactly like a well-formed one, and it is
    /// judged before `_since`.
    #[tokio::test]
    async fn test_history_at_is_refused_whatever_its_value() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-at-values").await;

        for query in [
            "_at=",
            "_at",
            "_at=not-a-date",
            "_at=2020",
            "_at=ge2020-01-01",
            "_at=2020-01-01T00:00:00Z&_since=2000-01-01T00:00:00Z&_count=1",
            // An invalid `_since` alongside `_at` answers the `_at` refusal
            // (`not-supported`), not the `_since` error (`invalid`).
            "_since=not-a-date&_at=2020",
        ] {
            let response = get_history(
                &server,
                &format!("/Patient/hist-at-values/_history?{query}"),
                None,
            )
            .await;
            assert_at_refused(&response, &format!("query string {query:?}"));
        }
    }

    /// `_at` is refused before the existence check, so a missing instance is a
    /// `400`, not a `404`.
    #[tokio::test]
    async fn test_instance_history_at_on_missing_resource_is_400_not_404() {
        let (server, _backend) = create_test_server().await;

        for prefer in PREFER_HANDLINGS {
            let response = get_history(
                &server,
                "/Patient/does-not-exist/_history?_at=2020-01-01T00:00:00Z",
                prefer,
            )
            .await;
            assert_ne!(
                response.status_code(),
                StatusCode::NOT_FOUND,
                "Prefer {prefer:?}: a missing instance with _at must not be a 404"
            );
            assert_at_refused(&response, &format!("missing instance, Prefer {prefer:?}"));
        }
    }

    /// Regression guard: history without `_at` is unchanged, whatever `Prefer`
    /// says.
    #[tokio::test]
    async fn test_history_without_at_is_unchanged_under_every_prefer_handling() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-no-at").await;

        for prefer in PREFER_HANDLINGS {
            let instance = get_history(&server, "/Patient/hist-no-at/_history", prefer).await;
            instance.assert_status_ok();
            let bundle: Value = instance.json();
            assert_eq!(bundle["type"], "history", "Prefer {prefer:?}");
            assert_eq!(bundle["total"], 3, "Prefer {prefer:?}");
            assert_eq!(
                bundle["entry"].as_array().expect("entry array").len(),
                3,
                "Prefer {prefer:?}"
            );

            let type_level = get_history(&server, "/Patient/_history", prefer).await;
            type_level.assert_status_ok();
            let bundle: Value = type_level.json();
            assert!(
                bundle["entry"].as_array().expect("entry array").len() >= 3,
                "Prefer {prefer:?}: type history should have at least the 3 seeded versions"
            );

            let system = get_history(&server, "/_history", prefer).await;
            system.assert_status_ok();
            let bundle: Value = system.json();
            assert!(
                !bundle["entry"].as_array().expect("entry array").is_empty(),
                "Prefer {prefer:?}: system history should not be empty"
            );
        }
    }

    /// Regression guard: `_since` still filters at all three levels, and an
    /// empty filtered instance history is a `200`, not a `404`.
    ///
    /// Only far-past and far-future `Z` instants are used: SQLite compares
    /// `last_updated` as a string, so an instant between two versions' own
    /// timestamps could be flaky.
    #[tokio::test]
    async fn test_history_since_still_filters() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-since-filter").await;

        let future = "_since=2999-01-01T00:00:00Z";
        let past = "_since=2000-01-01T00:00:00Z";

        let none = get_history(
            &server,
            &format!("/Patient/hist-since-filter/_history?{future}"),
            None,
        )
        .await;
        none.assert_status_ok();
        let bundle: Value = none.json();
        assert_eq!(bundle["type"], "history");
        assert!(bundle["entry"].as_array().is_none_or(|e| e.is_empty()));
        assert_eq!(bundle["total"], 0);

        let all = get_history(
            &server,
            &format!("/Patient/hist-since-filter/_history?{past}"),
            None,
        )
        .await;
        all.assert_status_ok();
        let bundle: Value = all.json();
        assert_eq!(bundle["entry"].as_array().expect("entry array").len(), 3);

        for path in ["/Patient/_history", "/_history"] {
            let none = get_history(&server, &format!("{path}?{future}"), None).await;
            none.assert_status_ok();
            let bundle: Value = none.json();
            assert!(
                bundle["entry"].as_array().is_none_or(|e| e.is_empty()),
                "{path}?{future} should have no entries"
            );

            let all = get_history(&server, &format!("{path}?{past}"), None).await;
            all.assert_status_ok();
            let bundle: Value = all.json();
            assert!(
                bundle["entry"].as_array().expect("entry array").len() >= 3,
                "{path}?{past} should have at least the 3 seeded versions"
            );
        }
    }

    // -------------------------------------------------------------------------
    // `_list`, `_sort` and unknown keys (#2381)
    // -------------------------------------------------------------------------

    /// `_list` and `_sort` as (parameter name, query string).
    const LIST_AND_SORT_QUERIES: [(&str, &str); 2] =
        [("_list", "_list=example"), ("_sort", "_sort=-_lastUpdated")];

    /// Asserts `_list` and `_sort` are each refused with `not-supported` at
    /// `base`, under every `Prefer: handling`.
    async fn assert_list_and_sort_refused(server: &TestServer, base: &str, level: &str) {
        for (param, query) in LIST_AND_SORT_QUERIES {
            for prefer in PREFER_HANDLINGS {
                let response = get_history(server, &format!("{base}?{query}"), prefer).await;
                assert_history_outcome(
                    &response,
                    "not-supported",
                    param,
                    &format!("{level}, {query}, Prefer {prefer:?}"),
                );
            }
        }
    }

    #[tokio::test]
    async fn test_instance_history_list_and_sort_are_refused_under_every_prefer_handling() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-ls-inst").await;

        assert_list_and_sort_refused(
            &server,
            "/Patient/hist-ls-inst/_history",
            "instance history",
        )
        .await;
    }

    #[tokio::test]
    async fn test_type_history_list_and_sort_are_refused_under_every_prefer_handling() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-ls-type").await;

        assert_list_and_sort_refused(&server, "/Patient/_history", "type history").await;
    }

    #[tokio::test]
    async fn test_system_history_list_and_sort_are_refused_under_every_prefer_handling() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-ls-sys").await;

        assert_list_and_sort_refused(&server, "/_history", "system history").await;
    }

    /// The refusal never judges the value: an empty, bare or odd `_list` /
    /// `_sort` is refused like a well-formed one, and a refused parameter wins
    /// over every other error in the query.
    #[tokio::test]
    async fn test_history_list_and_sort_are_refused_whatever_their_value() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-ls-values").await;

        for (query, param) in [
            ("_list=", "_list"),
            ("_list", "_list"),
            ("_sort=", "_sort"),
            ("_sort", "_sort"),
            ("_sort=_lastUpdated", "_sort"),
            ("_sort=none", "_sort"),
            ("_sort=family", "_sort"),
            ("_list=x&_since=2000-01-01T00:00:00Z&_count=1", "_list"),
            // Ordering: the refusal comes before the value errors.
            ("_since=not-a-date&_list=x", "_list"),
            ("_count=abc&_sort=x", "_sort"),
            (
                "_since=2000-01-01T00:00:00Z&_since=2001-01-01T00:00:00Z&_list=x",
                "_list",
            ),
        ] {
            let response = get_history(
                &server,
                &format!("/Patient/hist-ls-values/_history?{query}"),
                None,
            )
            .await;
            assert_history_outcome(
                &response,
                "not-supported",
                param,
                &format!("query string {query:?}"),
            );
        }

        // ... and before the unknown-key check under strict.
        let response = get_history(
            &server,
            "/Patient/hist-ls-values/_history?_snice=x&_sort=_lastUpdated",
            Some("handling=strict"),
        )
        .await;
        assert_history_outcome(
            &response,
            "not-supported",
            "_sort",
            "unknown key plus _sort under strict",
        );
    }

    /// `_list` / `_sort` are refused before the existence check, so a missing
    /// instance is a `400`, not a `404`.
    #[tokio::test]
    async fn test_instance_history_list_and_sort_on_missing_resource_are_400_not_404() {
        let (server, _backend) = create_test_server().await;

        for (param, query) in [("_list", "_list=x"), ("_sort", "_sort=x")] {
            for prefer in PREFER_HANDLINGS {
                let response = get_history(
                    &server,
                    &format!("/Patient/does-not-exist/_history?{query}"),
                    prefer,
                )
                .await;
                assert_ne!(
                    response.status_code(),
                    StatusCode::NOT_FOUND,
                    "{query}, Prefer {prefer:?}: a missing instance must not be a 404"
                );
                assert_history_outcome(
                    &response,
                    "not-supported",
                    param,
                    &format!("missing instance, {query}, Prefer {prefer:?}"),
                );
            }
        }
    }

    /// Under `Prefer: handling=strict` an unknown key is a `400` `invalid`
    /// naming it, at all three levels. That includes search-only parameters,
    /// which history does not apply.
    #[tokio::test]
    async fn test_history_unknown_param_is_rejected_under_strict() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-unknown").await;
        let strict = Some("handling=strict");

        for (key, query) in [
            ("_snice", "_snice=2000-01-01T00:00:00Z"),
            ("foo", "foo=bar"),
            ("_id", "_id=hist-unknown"),
            ("_offset", "_offset=1"),
        ] {
            for (level, base) in history_urls("hist-unknown") {
                let response = get_history(&server, &format!("{base}?{query}"), strict).await;
                assert_history_outcome(
                    &response,
                    "invalid",
                    key,
                    &format!("{level}, {query}, strict"),
                );
            }
        }

        // Every unknown key is named.
        for needle in ["_snice", "foo"] {
            let response = get_history(
                &server,
                "/Patient/hist-unknown/_history?_snice=x&foo=y",
                strict,
            )
            .await;
            assert_history_outcome(&response, "invalid", needle, "two unknown keys, strict");
        }

        // Judged before the existence check: a missing instance is a 400.
        let response =
            get_history(&server, "/Patient/does-not-exist/_history?_snice=x", strict).await;
        assert_ne!(
            response.status_code(),
            StatusCode::NOT_FOUND,
            "a missing instance with an unknown key under strict must not be a 404"
        );
        assert_history_outcome(
            &response,
            "invalid",
            "_snice",
            "missing instance, unknown key, strict",
        );
    }

    /// Regression guard: without `Prefer: handling=strict` an unknown key is
    /// ignored, as before. A history Bundle cannot carry search's ignored-
    /// parameter outcome entry, so the response is exactly the one without the
    /// key.
    #[tokio::test]
    async fn test_history_unknown_param_is_ignored_without_strict() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-lenient").await;

        for prefer in [None, Some("handling=lenient")] {
            for (level, base) in history_urls("hist-lenient") {
                let context = format!("{level}, Prefer {prefer:?}");

                let baseline = get_history_ok(&server, &base, prefer, &context).await;
                assert!(
                    baseline["entry"]
                        .as_array()
                        .is_some_and(|entries| !entries.is_empty()),
                    "{context}: the baseline should have entries"
                );
                let ignored = get_history_ok(
                    &server,
                    &format!("{base}?_snice=2999-01-01T00:00:00Z&foo=bar&_id=other"),
                    prefer,
                    &context,
                )
                .await;
                assert_eq!(ignored, baseline, "{context}: unknown keys must be ignored");

                let counted =
                    get_history_ok(&server, &format!("{base}?_count=1"), prefer, &context).await;
                let counted_ignored = get_history_ok(
                    &server,
                    &format!("{base}?_count=1&_snice=x"),
                    prefer,
                    &context,
                )
                .await;
                assert_eq!(
                    counted_ignored, counted,
                    "{context}: an unknown key must not change a _count read"
                );
            }
        }
    }

    /// Regression guard: FHIR's general parameters are accepted under every
    /// `Prefer: handling` and not applied, so the body is the plain history.
    ///
    /// The bodies are compared without `link`: `_format` is carried in the
    /// `self` link (#2382), so that is the one place they may differ. The link
    /// is checked on its own: it carries `_format=json` exactly when the request
    /// did, and never `_pretty`, `_summary` or `_elements`.
    #[tokio::test]
    async fn test_history_general_params_are_accepted_under_every_prefer_handling() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-general").await;

        for (level, base) in history_urls("hist-general") {
            let baseline = get_history_ok(&server, &base, None, level).await;
            assert!(
                baseline["entry"]
                    .as_array()
                    .is_some_and(|entries| !entries.is_empty()),
                "{level}: the baseline should have entries"
            );
            let baseline = without_link(baseline);

            for query in [
                "_pretty=true",
                "_summary=true",
                "_elements=id",
                "_format=json",
                "_pretty=true&_summary=true&_elements=id&_format=json",
            ] {
                for prefer in PREFER_HANDLINGS {
                    let context = format!("{level}, {query}, Prefer {prefer:?}");
                    let body =
                        get_history_ok(&server, &format!("{base}?{query}"), prefer, &context).await;

                    let self_url = link(&body, "self")
                        .unwrap_or_else(|| panic!("{context}: no self link in {body}"));
                    let pairs = link_query(self_url);
                    assert_eq!(
                        pairs.iter().any(|(k, v)| k == "_format" && v == "json"),
                        query.contains("_format=json"),
                        "{context}: self was {self_url}"
                    );
                    for unapplied in ["_pretty", "_summary", "_elements"] {
                        assert!(
                            pairs.iter().all(|(k, _)| k != unapplied),
                            "{context}: self must not carry {unapplied}: {self_url}"
                        );
                    }

                    assert_eq!(
                        without_link(body),
                        baseline,
                        "{context}: general parameters must not change the body"
                    );
                }
            }
        }
    }

    /// A `_count` that is not a non-negative integer is a `400` OperationOutcome
    /// (`invalid`), not axum's plain-text rejection.
    #[tokio::test]
    async fn test_history_malformed_count_is_an_operation_outcome() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-count").await;

        // `+5` is the wire form of " 5": `+` is a space in a query string.
        for query in [
            "_count=abc",
            "_count=-1",
            "_count=1.5",
            "_count=+5",
            "_count=",
            "_count",
        ] {
            for prefer in PREFER_HANDLINGS {
                for (level, base) in history_urls("hist-count") {
                    let response = get_history(&server, &format!("{base}?{query}"), prefer).await;
                    assert_history_outcome(
                        &response,
                        "invalid",
                        "_count",
                        &format!("{level}, {query}, Prefer {prefer:?}"),
                    );
                }
            }
        }

        for prefer in PREFER_HANDLINGS {
            let response = get_history(
                &server,
                "/Patient/does-not-exist/_history?_count=abc",
                prefer,
            )
            .await;
            assert_history_outcome(
                &response,
                "invalid",
                "_count",
                &format!("missing instance, Prefer {prefer:?}"),
            );
        }
    }

    /// A `_count` too large for `usize` is still a non-negative integer, not a
    /// `400`: it is clamped to the configured maximum page size like any other
    /// oversized `_count`, because FHIR lets a server return fewer than `_count`.
    #[tokio::test]
    async fn test_history_overflowing_count_is_clamped_to_the_max_page_size() {
        // A default page size below the maximum shows that the overflowing
        // `_count` was applied (and clamped), not ignored.
        let (server, _backend) = create_test_server_with(|c| {
            c.default_page_size = 1;
            c.max_page_size = 2;
        })
        .await;
        seed_versioned_patient(&server, "hist-overflow").await;

        for prefer in PREFER_HANDLINGS {
            for (level, base) in history_urls("hist-overflow") {
                let context = |query: &str| format!("{level}, {query}, Prefer {prefer:?}");

                let query = "_count=99999999999999999999";
                let body =
                    get_history_ok(&server, &format!("{base}?{query}"), prefer, &context(query))
                        .await;
                assert_eq!(
                    body["entry"].as_array().expect("entry array").len(),
                    2,
                    "{}: expected the page clamped to the max page size of 2",
                    context(query)
                );
                // The self link carries the effective count, not the one sent.
                assert_eq!(
                    link_param(&body, "self", "_count").as_deref(),
                    Some("2"),
                    "{}: expected the self link to carry the clamped _count",
                    context(query)
                );

                // Control: no `_count` returns the default page size of 1.
                let body = get_history_ok(&server, &base, prefer, &context("no _count")).await;
                assert_eq!(
                    body["entry"].as_array().expect("entry array").len(),
                    1,
                    "{}: expected the default page size of 1",
                    context("no _count")
                );
                assert_eq!(
                    link_param(&body, "self", "_count").as_deref(),
                    Some("1"),
                    "{}: expected the self link to carry the default _count",
                    context("no _count")
                );
            }
        }
    }

    /// A repeated `_since`, `_count`, `_format` or `_cursor` is a `400`
    /// OperationOutcome (`invalid`) naming it, under every `Prefer: handling`.
    #[tokio::test]
    async fn test_history_repeated_params_are_an_operation_outcome() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-repeat").await;

        for (query, needle) in [
            (
                "_since=2000-01-01T00:00:00Z&_since=2001-01-01T00:00:00Z",
                "_since",
            ),
            ("_count=1&_count=2", "_count"),
            ("_format=json&_format=json", "_format"),
            ("_cursor=a&_cursor=b", "_cursor"),
        ] {
            for prefer in PREFER_HANDLINGS {
                for (level, base) in history_urls("hist-repeat") {
                    let response = get_history(&server, &format!("{base}?{query}"), prefer).await;
                    assert_history_outcome(
                        &response,
                        "invalid",
                        needle,
                        &format!("{level}, {query}, Prefer {prefer:?}"),
                    );
                }
            }
        }
    }

    /// Guard: the `_since` error message is the one clients already got.
    #[tokio::test]
    async fn test_history_invalid_since_message_is_unchanged() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-since-msg").await;

        for prefer in PREFER_HANDLINGS {
            for (level, base) in history_urls("hist-since-msg") {
                let context = format!("{level}, Prefer {prefer:?}");
                let response =
                    get_history(&server, &format!("{base}?_since=not-a-date"), prefer).await;
                assert_history_outcome(&response, "invalid", "_since", &context);
                let outcome = history_outcome(&response, &context);
                assert_eq!(
                    outcome["issue"][0]["details"]["text"],
                    "Invalid _since value (expected an RFC3339 instant): not-a-date",
                    "{context}: body was {outcome}"
                );
            }
        }
    }

    // -------------------------------------------------------------------------
    // Paging: `self` / `next` links and `_cursor` (#2382)
    // -------------------------------------------------------------------------

    /// Writes 3 Patients and 1 Observation, two versions each, all after the
    /// instant it returns (formatted as a `_since` value).
    ///
    /// The 50 ms pause keeps every write strictly after the instant. The
    /// instant is never close to a write: SQLite compares `last_updated` as a
    /// string.
    async fn seed_walk_set(server: &TestServer, tag: &str) -> String {
        let since = Utc::now().to_rfc3339_opts(SecondsFormat::AutoSi, true);
        tokio::time::sleep(Duration::from_millis(50)).await;
        for n in ["a", "b", "c"] {
            for family in ["V1", "V2"] {
                put_resource(server, patient(&format!("{tag}-{n}"), family)).await;
            }
        }
        for text in ["one", "two"] {
            put_resource(
                server,
                json!({
                    "resourceType": "Observation",
                    "id": format!("{tag}-obs"),
                    "status": "final",
                    "code": {"text": text}
                }),
            )
            .await;
        }
        since
    }

    /// Following `next` from the first page of an instance history returns every
    /// version once, newest first, and stops.
    #[tokio::test]
    async fn test_instance_history_next_walks_every_version_once() {
        let (server, _backend) = create_test_server().await;
        for family in ["V1", "V2", "V3", "V4", "V5"] {
            put_resource(&server, patient("hist-walk-inst", family)).await;
        }

        let pages = walk_history(
            &server,
            "/Patient/hist-walk-inst/_history?_count=2",
            None,
            "instance history",
        )
        .await;

        let sizes: Vec<usize> = pages.iter().map(|page| entries(page).len()).collect();
        assert_eq!(sizes, [2, 2, 1], "page sizes");
        let versions: Vec<(String, String)> =
            all_entries(&pages).into_iter().map(version_key).collect();
        let expected: Vec<(String, String)> = (1..=5)
            .rev()
            .map(|version| ("Patient/hist-walk-inst".to_string(), version.to_string()))
            .collect();
        assert_eq!(versions, expected, "every version once, newest first");

        assert_eq!(
            link(&pages[0], "self"),
            Some("http://localhost:8080/Patient/hist-walk-inst/_history?_count=2")
        );
        assert_page_links(&pages, 2, "instance history");

        // A page's `self` reproduces that page.
        let self_url = link(&pages[1], "self")
            .and_then(|url| url.strip_prefix(BASE))
            .expect("page 2 self link");
        let again = get_history_ok(&server, self_url, None, "page 2 self").await;
        assert_eq!(entries(&again), entries(&pages[1]));
    }

    #[tokio::test]
    async fn test_type_history_next_walks_every_version_once() {
        let (server, _backend) = create_test_server().await;
        let since = seed_walk_set(&server, "hist-walk-type").await;

        let pages = walk_history(
            &server,
            &format!("/Patient/_history?_count=2&_since={since}"),
            None,
            "type history",
        )
        .await;

        assert!(
            pages.iter().all(|page| entries(page).len() <= 2),
            "no page may exceed _count=2"
        );
        assert_eq!(
            sorted_keys(&pages),
            expected_keys(
                &[
                    "Patient/hist-walk-type-a",
                    "Patient/hist-walk-type-b",
                    "Patient/hist-walk-type-c",
                ],
                &[1, 2]
            ),
            "exactly the 6 Patient versions, none twice, no Observation"
        );
        assert_newest_first(&pages, "type history");
        assert_page_links(&pages, 2, "type history");
    }

    #[tokio::test]
    async fn test_system_history_next_walks_every_version_once() {
        let (server, _backend) = create_test_server().await;
        let since = seed_walk_set(&server, "hist-walk-sys").await;

        let pages = walk_history(
            &server,
            &format!("/_history?_count=3&_since={since}"),
            None,
            "system history",
        )
        .await;

        assert!(
            pages.iter().all(|page| entries(page).len() <= 3),
            "no page may exceed _count=3"
        );
        assert_eq!(
            sorted_keys(&pages),
            expected_keys(
                &[
                    "Observation/hist-walk-sys-obs",
                    "Patient/hist-walk-sys-a",
                    "Patient/hist-walk-sys-b",
                    "Patient/hist-walk-sys-c",
                ],
                &[1, 2]
            ),
            "exactly the 8 seeded versions, none twice"
        );
        assert_newest_first(&pages, "system history");
        assert_page_links(&pages, 3, "system history");
    }

    /// Asserts every `self` and `next` link of the walk carries `_count=count`
    /// and a `_since` equal to `since`.
    fn assert_links_keep_since(pages: &[Value], count: usize, since: DateTime<Utc>, context: &str) {
        let count = count.to_string();
        for (i, page) in pages.iter().enumerate() {
            for relation in ["self", "next"] {
                if link(page, relation).is_none() {
                    continue;
                }
                let context = format!("{context}, page {}, {relation}", i + 1);
                assert_eq!(
                    link_param(page, relation, "_count").as_deref(),
                    Some(count.as_str()),
                    "{context}: _count"
                );
                let carried = link_param(page, relation, "_since")
                    .unwrap_or_else(|| panic!("{context}: no _since on the link"));
                let carried = DateTime::parse_from_rfc3339(&carried)
                    .unwrap_or_else(|e| panic!("{context}: _since {carried:?}: {e}"))
                    .with_timezone(&Utc);
                assert_eq!(carried, since, "{context}: _since");
            }
        }
    }

    /// `next` keeps `_since` and `_count`: a walk with a `_since` never leaks a
    /// version from before it, at any level, and every link says so.
    #[tokio::test]
    async fn test_history_next_keeps_since_across_pages() {
        let (server, _backend) = create_test_server().await;
        for family in ["V1", "V2"] {
            put_resource(&server, patient("hist-since-walk", family)).await;
        }
        put_resource(&server, patient("hist-since-early", "V1")).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let since = Utc::now();
        tokio::time::sleep(Duration::from_millis(50)).await;
        for family in ["V3", "V4", "V5", "V6", "V7"] {
            put_resource(&server, patient("hist-since-walk", family)).await;
        }
        put_resource(&server, patient("hist-since-late", "V1")).await;
        put_resource(
            &server,
            json!({
                "resourceType": "Observation",
                "id": "hist-since-obs",
                "status": "final",
                "code": {"text": "after"}
            }),
        )
        .await;
        let since_text = since.to_rfc3339_opts(SecondsFormat::AutoSi, true);

        // Instance: v7..v3 in pages of 2, 2 and 1; v1 and v2 are before `since`.
        let pages = walk_history(
            &server,
            &format!("/Patient/hist-since-walk/_history?_count=2&_since={since_text}"),
            None,
            "instance history",
        )
        .await;
        let sizes: Vec<usize> = pages.iter().map(|page| entries(page).len()).collect();
        assert_eq!(sizes, [2, 2, 1], "instance history page sizes");
        let versions: Vec<(String, String)> =
            all_entries(&pages).into_iter().map(version_key).collect();
        let expected: Vec<(String, String)> = (3..=7)
            .rev()
            .map(|version| ("Patient/hist-since-walk".to_string(), version.to_string()))
            .collect();
        assert_eq!(versions, expected, "instance history versions");
        assert_page_links(&pages, 2, "instance history");
        assert_links_keep_since(&pages, 2, since, "instance history");

        // Type: the walked Patient's v3..v7 and the late Patient, not the early one.
        let mut patient_keys = expected_keys(&["Patient/hist-since-walk"], &[3, 4, 5, 6, 7]);
        patient_keys.push(("Patient/hist-since-late".to_string(), "1".to_string()));
        patient_keys.sort();
        let pages = walk_history(
            &server,
            &format!("/Patient/_history?_count=2&_since={since_text}"),
            None,
            "type history",
        )
        .await;
        assert_eq!(sorted_keys(&pages), patient_keys, "type history versions");
        assert_page_links(&pages, 2, "type history");
        assert_links_keep_since(&pages, 2, since, "type history");

        // System: the same, plus the Observation.
        let mut system_keys = patient_keys;
        system_keys.push(("Observation/hist-since-obs".to_string(), "1".to_string()));
        system_keys.sort();
        let pages = walk_history(
            &server,
            &format!("/_history?_count=2&_since={since_text}"),
            None,
            "system history",
        )
        .await;
        assert_eq!(sorted_keys(&pages), system_keys, "system history versions");
        assert_page_links(&pages, 2, "system history");
        assert_links_keep_since(&pages, 2, since, "system history");
    }

    /// A `_cursor` that does not decode is a `400` OperationOutcome (`invalid`)
    /// with the message search renders for the same fault, at all three levels
    /// and under every `Prefer: handling`. An empty one counts, and the check
    /// comes before the existence check.
    #[tokio::test]
    async fn test_history_bad_cursor_is_an_operation_outcome() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-bad-cursor").await;

        // "" and "abc" decode to nothing JSON, "not-a-cursor!!" is not base64
        // and "e30" is base64 of `{}`, JSON of the wrong shape.
        for value in ["not-a-cursor!!", "", "abc", "e30"] {
            let expected = format!("invalid pagination cursor: {value}");
            for prefer in PREFER_HANDLINGS {
                for (level, base) in history_urls("hist-bad-cursor") {
                    let context = format!("{level}, _cursor={value:?}, Prefer {prefer:?}");
                    let response =
                        get_history(&server, &format!("{base}?_cursor={value}"), prefer).await;
                    assert_history_outcome(&response, "invalid", &expected, &context);
                    let outcome = history_outcome(&response, &context);
                    assert_eq!(
                        outcome["issue"][0]["details"]["text"],
                        expected.as_str(),
                        "{context}: body was {outcome}"
                    );
                }
            }
        }

        for prefer in PREFER_HANDLINGS {
            let response = get_history(
                &server,
                "/Patient/does-not-exist/_history?_cursor=abc",
                prefer,
            )
            .await;
            assert_history_outcome(
                &response,
                "invalid",
                "invalid pagination cursor: abc",
                &format!("missing instance, Prefer {prefer:?}"),
            );
        }
    }

    /// `_cursor` is a supported history parameter, so `Prefer: handling=strict`
    /// accepts it: a `next` link can be followed by a strict client.
    #[tokio::test]
    async fn test_history_cursor_is_accepted_under_strict() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-strict-cursor").await;
        let strict = Some("handling=strict");

        for (level, base) in history_urls("hist-strict-cursor") {
            let first = get_history_ok(&server, &format!("{base}?_count=1"), strict, level).await;
            let next = link(&first, "next")
                .and_then(|url| url.strip_prefix(BASE))
                .unwrap_or_else(|| panic!("{level}: page 1 should have a next link: {first}"));

            let second = get_history_ok(&server, next, strict, level).await;
            assert_eq!(entries(&second).len(), 1, "{level}: page 2 size");
            assert_ne!(
                version_key(&entries(&first)[0]),
                version_key(&entries(&second)[0]),
                "{level}: page 2 must be a different version"
            );
        }
    }

    /// An empty page reached through a `_cursor` that fits the resource is a
    /// `200` with an empty Bundle, not a `404`: the walk ran past the end
    /// (versions deleted between pages, say), and the resource may well exist.
    /// Without a cursor the same request for a resource that never existed is
    /// still a `404`.
    ///
    /// The cursor has to be one the backend could have issued for that
    /// resource, so the test builds it by hand: instance history writes exactly
    /// `[version]` with the instance id as `resource_id`. A cursor issued for
    /// another resource is not "past the end" but a bad request: it is refused
    /// with a `400` OperationOutcome (#2402), pinned here at the REST level.
    #[tokio::test]
    async fn test_instance_history_empty_page_after_cursor_is_200_not_404() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-cursor-source").await;

        let never = "/Patient/hist-cursor-never/_history";
        get_history(&server, never, None)
            .await
            .assert_status(StatusCode::NOT_FOUND);

        // Nothing is older than version 1, so the page is empty.
        let past_end = PageCursor::new(
            vec![CursorValue::String("1".to_string())],
            "hist-cursor-never",
        )
        .encode();
        let bundle = get_history_ok(
            &server,
            &format!("{never}?_cursor={past_end}"),
            None,
            "past end",
        )
        .await;
        assert_eq!(bundle["type"], "history");
        assert_eq!(bundle["total"], 0);
        assert!(entries(&bundle).is_empty(), "body was {bundle}");
        assert!(link(&bundle, "self").is_some(), "body was {bundle}");
        assert!(link(&bundle, "next").is_none(), "body was {bundle}");

        // A cursor taken from another resource's `next` link is refused, not
        // read as an empty page.
        let first = get_history_ok(
            &server,
            "/Patient/hist-cursor-source/_history?_count=1",
            None,
            "source",
        )
        .await;
        let foreign = link_param(&first, "next", "_cursor").expect("page 1 should have a next");
        assert_eq!(
            PageCursor::decode(&foreign)
                .expect("the next cursor decodes")
                .resource_id(),
            "hist-cursor-source",
            "the cursor was issued for the source resource"
        );
        let response = get_history(&server, &format!("{never}?_cursor={foreign}"), None).await;
        assert_history_outcome(
            &response,
            "invalid",
            "invalid pagination cursor",
            "cursor of another resource",
        );
    }

    /// A `_count=0` page returns no entries and must not offer `next`: a client
    /// following it would read the same empty page forever. This pins the
    /// contract at the REST layer for every level, whatever the backend answers
    /// (S3 reports `has_next`; `next_page_cursor` is unit-tested in `history.rs`).
    ///
    /// The instance request carries no `_since`: an empty page cannot tell an
    /// existing resource from one that never existed when `_count=0`, so the
    /// handler asks storage whether the resource has any version.
    #[tokio::test]
    async fn test_history_count_zero_has_self_but_no_next() {
        let (server, _backend) = create_test_server().await;
        seed_versioned_patient(&server, "hist-count-zero").await;

        for (level, url) in [
            ("instance", "/Patient/hist-count-zero/_history?_count=0"),
            ("type", "/Patient/_history?_count=0"),
            ("system", "/_history?_count=0"),
        ] {
            let bundle = get_history_ok(&server, url, None, level).await;
            assert_eq!(bundle["type"], "history", "{level}: {bundle}");
            assert!(link(&bundle, "self").is_some(), "{level}: {bundle}");
            assert_eq!(
                link_param(&bundle, "self", "_count").as_deref(),
                Some("0"),
                "{level}: {bundle}"
            );
            assert!(link(&bundle, "next").is_none(), "{level}: {bundle}");
        }
    }

    /// `_count=0` does not make a resource that never existed look like it did:
    /// the empty page is still a `404`.
    #[tokio::test]
    async fn test_instance_history_count_zero_of_unknown_resource_is_404() {
        let (server, _backend) = create_test_server().await;

        get_history(
            &server,
            "/Patient/hist-count-zero-never/_history?_count=0",
            None,
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
    }
}
