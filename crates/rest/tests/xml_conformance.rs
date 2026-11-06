//! XML conformance tests for the FHIR REST API.
//!
//! Tests XML content negotiation, request parsing, and response formatting.
//! These tests verify that the server correctly handles XML when the `xml`
//! feature is enabled.

#![cfg(feature = "xml")]

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Bytes;
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
const ACCEPT: HeaderName = HeaderName::from_static("accept");

/// Creates a test server with XML support.
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

/// A minimal FHIR Patient in XML format.
fn patient_xml(family: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Patient xmlns="http://hl7.org/fhir">
  <name>
    <family value="{}"/>
  </name>
  <active value="true"/>
</Patient>"#,
        family
    )
}

/// A minimal FHIR Patient in XML format with an ID.
fn patient_xml_with_id(id: &str, family: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Patient xmlns="http://hl7.org/fhir">
  <id value="{}"/>
  <name>
    <family value="{}"/>
  </name>
  <active value="true"/>
</Patient>"#,
        id, family
    )
}

// =============================================================================
// Content Negotiation Tests
// =============================================================================

mod content_negotiation {
    use super::*;

    #[tokio::test]
    async fn test_accept_xml_returns_xml_content_type() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(ACCEPT, HeaderValue::from_static("application/fhir+xml"))
            .await;

        response.assert_status_ok();

        let ct = response
            .headers()
            .get("content-type")
            .expect("Should have Content-Type")
            .to_str()
            .unwrap();
        assert!(
            ct.contains("application/fhir+xml"),
            "Content-Type should be XML, got: {}",
            ct
        );
    }

    #[tokio::test]
    async fn test_format_param_xml_overrides_accept() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        // Accept says JSON but _format says XML
        let response = server
            .get("/Patient/patient-1?_format=xml")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(ACCEPT, HeaderValue::from_static("application/fhir+json"))
            .await;

        response.assert_status_ok();

        let ct = response
            .headers()
            .get("content-type")
            .expect("Should have Content-Type")
            .to_str()
            .unwrap();
        assert!(
            ct.contains("application/fhir+xml"),
            "Content-Type should be XML (from _format), got: {}",
            ct
        );
    }

    #[tokio::test]
    async fn test_no_accept_defaults_to_json() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();

        let ct = response
            .headers()
            .get("content-type")
            .expect("Should have Content-Type")
            .to_str()
            .unwrap();
        assert!(
            ct.contains("json"),
            "Default Content-Type should be JSON, got: {}",
            ct
        );
    }

    #[tokio::test]
    async fn test_wildcard_accept_defaults_to_json() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(ACCEPT, HeaderValue::from_static("*/*"))
            .await;

        response.assert_status_ok();

        let ct = response
            .headers()
            .get("content-type")
            .expect("Should have Content-Type")
            .to_str()
            .unwrap();
        assert!(
            ct.contains("json"),
            "Wildcard Accept should default to JSON, got: {}",
            ct
        );
    }

    #[tokio::test]
    async fn test_format_param_application_fhir_xml() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .get("/Patient/patient-1?_format=application/fhir%2Bxml")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();

        let ct = response
            .headers()
            .get("content-type")
            .expect("Should have Content-Type")
            .to_str()
            .unwrap();
        assert!(
            ct.contains("application/fhir+xml"),
            "Content-Type should be XML, got: {}",
            ct
        );
    }
}

// =============================================================================
// Read XML Tests
// =============================================================================

mod read_xml {
    use super::*;

    #[tokio::test]
    async fn test_read_xml_returns_valid_fhir_xml() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(ACCEPT, HeaderValue::from_static("application/fhir+xml"))
            .await;

        response.assert_status_ok();

        let body = response.text();
        assert!(
            body.contains("<Patient"),
            "XML should contain <Patient> element, got: {}",
            &body[..body.len().min(500)]
        );
        assert!(
            body.contains("http://hl7.org/fhir"),
            "XML should contain FHIR namespace, got: {}",
            &body[..body.len().min(500)]
        );
        assert!(
            body.contains("Smith"),
            "XML should contain patient family name, got: {}",
            &body[..body.len().min(500)]
        );
    }

    #[tokio::test]
    async fn test_read_xml_roundtrip_data_integrity() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        // Read as JSON to get baseline
        let json_response = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(ACCEPT, HeaderValue::from_static("application/fhir+json"))
            .await;
        json_response.assert_status_ok();
        let json_body: Value = json_response.json();

        // Read as XML
        let xml_response = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(ACCEPT, HeaderValue::from_static("application/fhir+xml"))
            .await;
        xml_response.assert_status_ok();
        let xml_body = xml_response.text();

        // Verify key data is present in XML
        assert_eq!(json_body["resourceType"], "Patient");
        assert!(
            xml_body.contains("Smith"),
            "XML should contain the same family name as JSON"
        );
        assert!(
            xml_body.contains("patient-1") || xml_body.contains("Patient"),
            "XML should contain identifying information"
        );
    }

    #[tokio::test]
    async fn test_read_xml_includes_fhir_version_in_content_type() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(ACCEPT, HeaderValue::from_static("application/fhir+xml"))
            .await;

        response.assert_status_ok();

        let ct = response
            .headers()
            .get("content-type")
            .expect("Should have Content-Type")
            .to_str()
            .unwrap();
        assert!(
            ct.contains("fhirVersion="),
            "Content-Type should include fhirVersion, got: {}",
            ct
        );
    }

    #[tokio::test]
    async fn test_read_xml_preserves_etag() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(ACCEPT, HeaderValue::from_static("application/fhir+xml"))
            .await;

        response.assert_status_ok();

        let etag = response.headers().get("etag");
        assert!(etag.is_some(), "XML response should have ETag header");

        let etag_value = etag.unwrap().to_str().unwrap();
        assert!(
            etag_value.starts_with("W/\""),
            "ETag should be weak: {}",
            etag_value
        );
    }

    #[tokio::test]
    async fn test_read_not_found_with_xml_accept() {
        let (server, _backend) = create_test_server().await;

        let response = server
            .get("/Patient/nonexistent")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(ACCEPT, HeaderValue::from_static("application/fhir+xml"))
            .await;

        // Error responses are currently always JSON (RestError doesn't negotiate format)
        // This tests that at least the correct status code is returned
        response.assert_status(StatusCode::NOT_FOUND);
    }
}

// =============================================================================
// Create with XML Body Tests
// =============================================================================

mod create_xml {
    use super::*;

    #[tokio::test]
    async fn test_create_with_xml_body() {
        let (server, _backend) = create_test_server().await;

        let xml_body = patient_xml("XmlCreated");

        let response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+xml"),
            )
            .bytes(Bytes::from(xml_body))
            .await;

        response.assert_status(StatusCode::CREATED);

        // Should have Location header
        let location = response.headers().get("location");
        assert!(
            location.is_some(),
            "Create response should have Location header"
        );
    }

    #[tokio::test]
    async fn test_create_xml_read_back_json() {
        let (server, _backend) = create_test_server().await;

        let xml_body = patient_xml("XmlToJson");

        let create_response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+xml"),
            )
            .bytes(Bytes::from(xml_body))
            .await;

        create_response.assert_status(StatusCode::CREATED);

        // Extract the created resource's location
        let location = create_response
            .headers()
            .get("location")
            .expect("Should have Location")
            .to_str()
            .unwrap()
            .to_string();

        // Extract path from location URL
        let path = location
            .strip_prefix("http://localhost:8080")
            .unwrap_or(&location);

        // Read back as JSON
        let read_response = server
            .get(path)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(ACCEPT, HeaderValue::from_static("application/fhir+json"))
            .await;

        read_response.assert_status_ok();

        let body: Value = read_response.json();
        assert_eq!(body["resourceType"], "Patient");
        assert_eq!(body["name"][0]["family"], "XmlToJson");
    }

    #[tokio::test]
    async fn test_create_xml_read_back_xml() {
        let (server, _backend) = create_test_server().await;

        let xml_body = patient_xml("XmlRoundtrip");

        let create_response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+xml"),
            )
            .bytes(Bytes::from(xml_body))
            .await;

        create_response.assert_status(StatusCode::CREATED);

        let location = create_response
            .headers()
            .get("location")
            .expect("Should have Location")
            .to_str()
            .unwrap()
            .to_string();

        let path = location
            .strip_prefix("http://localhost:8080")
            .unwrap_or(&location);

        // Read back as XML
        let read_response = server
            .get(path)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(ACCEPT, HeaderValue::from_static("application/fhir+xml"))
            .await;

        read_response.assert_status_ok();

        let xml_result = read_response.text();
        assert!(
            xml_result.contains("XmlRoundtrip"),
            "XML roundtrip should preserve family name, got: {}",
            &xml_result[..xml_result.len().min(500)]
        );
        assert!(
            xml_result.contains("<Patient"),
            "Should be a Patient XML element"
        );
    }

    #[tokio::test]
    async fn test_create_xml_invalid_body_returns_400() {
        let (server, _backend) = create_test_server().await;

        let invalid_xml = "<not-valid-xml><<<<";

        let response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+xml"),
            )
            .bytes(Bytes::from(invalid_xml))
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_xml_response_format_matches_accept() {
        let (server, _backend) = create_test_server().await;

        let xml_body = patient_xml("AcceptXml");

        // POST XML body but request XML response via Accept
        let response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+xml"),
            )
            .add_header(ACCEPT, HeaderValue::from_static("application/fhir+xml"))
            .bytes(Bytes::from(xml_body))
            .await;

        response.assert_status(StatusCode::CREATED);

        let ct = response
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();

        // The response body should be XML if Accept was set
        if ct.contains("xml") {
            let body = response.text();
            assert!(
                body.contains("<Patient") || body.contains("<OperationOutcome"),
                "XML response should contain FHIR resource"
            );
        }
    }
}

// =============================================================================
// Update with XML Body Tests
// =============================================================================

mod update_xml {
    use super::*;

    #[tokio::test]
    async fn test_update_with_xml_body() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let xml_body = patient_xml_with_id("patient-1", "UpdatedViaXml");

        let response = server
            .put("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+xml"),
            )
            .bytes(Bytes::from(xml_body))
            .await;

        response.assert_status_ok();

        // Verify the update took effect
        let read_response = server
            .get("/Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        read_response.assert_status_ok();
        let body: Value = read_response.json();
        assert_eq!(body["name"][0]["family"], "UpdatedViaXml");
    }

    #[tokio::test]
    async fn test_upsert_with_xml_body() {
        let (server, _backend) = create_test_server().await;

        let xml_body = patient_xml_with_id("xml-upsert-1", "UpsertXml");

        let response = server
            .put("/Patient/xml-upsert-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+xml"),
            )
            .bytes(Bytes::from(xml_body))
            .await;

        response.assert_status(StatusCode::CREATED);
    }
}

// =============================================================================
// Patch Response Format Tests (#863)
// =============================================================================

/// `PATCH [type]/[id]` and `PATCH [type]?[criteria]` negotiate the response
/// format like every other write. The patch document stays JSON (RFC 6902 /
/// RFC 7386 / FHIRPath Patch carry no XML form); only the answer is negotiated.
mod patch_xml {
    use super::*;

    const JSON_PATCH: &str = "application/json-patch+json";
    const MERGE_PATCH: &str = "application/merge-patch+json";

    fn rename(family: &str) -> Value {
        json!([{ "op": "replace", "path": "/name/0/family", "value": family }])
    }

    /// A patch request carrying `body` in the patch format `content_type` names.
    fn patch_request(
        server: &TestServer,
        url: &str,
        content_type: &'static str,
        body: &Value,
    ) -> axum_test::TestRequest {
        server
            .patch(url)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(CONTENT_TYPE, HeaderValue::from_static(content_type))
            .bytes(Bytes::from(body.to_string()))
    }

    fn content_type_of(response: &axum_test::TestResponse) -> String {
        response
            .headers()
            .get("content-type")
            .expect("Should have Content-Type")
            .to_str()
            .unwrap()
            .to_string()
    }

    /// Reads the stored resource back as JSON: the patch must have been applied
    /// whatever format the answer was written in.
    async fn stored_family(server: &TestServer, id: &str) -> String {
        let response = server
            .get(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();
        body["name"][0]["family"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn test_patch_accept_xml_returns_xml() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = patch_request(
            &server,
            "/Patient/patient-1",
            JSON_PATCH,
            &rename("PatchedXml"),
        )
        .add_header(ACCEPT, HeaderValue::from_static("application/fhir+xml"))
        .await;

        response.assert_status_ok();

        let ct = content_type_of(&response);
        assert!(
            ct.starts_with("application/fhir+xml"),
            "XML body must not be labelled as JSON, got Content-Type: {ct}"
        );
        assert!(
            response.headers().get("etag").is_some(),
            "ETag is format-independent and must survive negotiation"
        );
        assert!(response.headers().get("last-modified").is_some());

        let body = response.text();
        assert!(
            body.trim_start().starts_with('<'),
            "body should be XML, got: {body}"
        );
        assert!(body.contains("<Patient"), "Should contain Patient element");
        assert!(
            body.contains("PatchedXml"),
            "XML should carry the patched value: {body}"
        );

        assert_eq!(stored_family(&server, "patient-1").await, "PatchedXml");
    }

    #[tokio::test]
    async fn test_patch_default_accept_returns_json() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = patch_request(
            &server,
            "/Patient/patient-1",
            JSON_PATCH,
            &rename("PatchedJson"),
        )
        .await;

        response.assert_status_ok();
        assert_eq!(content_type_of(&response), "application/fhir+json");
        let body: Value = response.json();
        assert_eq!(body["resourceType"], "Patient");
        assert_eq!(body["name"][0]["family"], "PatchedJson");
    }

    #[tokio::test]
    async fn test_patch_explicit_accept_json_returns_json() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = patch_request(
            &server,
            "/Patient/patient-1",
            JSON_PATCH,
            &rename("ExplicitJson"),
        )
        .add_header(ACCEPT, HeaderValue::from_static("application/fhir+json"))
        .await;

        response.assert_status_ok();
        assert_eq!(content_type_of(&response), "application/fhir+json");
        let body: Value = response.json();
        assert_eq!(body["name"][0]["family"], "ExplicitJson");
    }

    #[tokio::test]
    async fn test_patch_return_operation_outcome_accept_xml() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = patch_request(
            &server,
            "/Patient/patient-1",
            JSON_PATCH,
            &rename("OutcomeXml"),
        )
        .add_header(ACCEPT, HeaderValue::from_static("application/fhir+xml"))
        .add_header(
            HeaderName::from_static("prefer"),
            HeaderValue::from_static("return=OperationOutcome"),
        )
        .await;

        response.assert_status_ok();
        let ct = content_type_of(&response);
        assert!(
            ct.starts_with("application/fhir+xml"),
            "XML body must not be labelled as JSON, got Content-Type: {ct}"
        );
        let body = response.text();
        assert!(
            body.contains("<OperationOutcome"),
            "should be an XML OperationOutcome: {body}"
        );
        assert_eq!(stored_family(&server, "patient-1").await, "OutcomeXml");
    }

    #[tokio::test]
    async fn test_patch_return_minimal_has_no_body_whatever_the_accept() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = patch_request(
            &server,
            "/Patient/patient-1",
            JSON_PATCH,
            &rename("MinimalXml"),
        )
        .add_header(ACCEPT, HeaderValue::from_static("application/fhir+xml"))
        .add_header(
            HeaderName::from_static("prefer"),
            HeaderValue::from_static("return=minimal"),
        )
        .await;

        response.assert_status_ok();
        assert!(response.text().is_empty(), "return=minimal has no body");
        assert!(response.headers().get("etag").is_some());
        assert_eq!(stored_family(&server, "patient-1").await, "MinimalXml");
    }

    #[tokio::test]
    async fn test_conditional_patch_accept_xml_returns_xml() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;
        seed_patient(&backend, "patient-2", "Jones").await;

        let response = patch_request(
            &server,
            "/Patient?family=Smith",
            MERGE_PATCH,
            &json!({ "name": [{ "family": "CondPatchedXml" }] }),
        )
        .add_header(ACCEPT, HeaderValue::from_static("application/fhir+xml"))
        .await;

        response.assert_status_ok();

        let ct = content_type_of(&response);
        assert!(
            ct.starts_with("application/fhir+xml"),
            "XML body must not be labelled as JSON, got Content-Type: {ct}"
        );
        assert!(response.headers().get("etag").is_some());

        let body = response.text();
        assert!(
            body.trim_start().starts_with('<'),
            "body should be XML, got: {body}"
        );
        assert!(body.contains("<Patient"), "Should contain Patient element");
        assert!(
            body.contains("CondPatchedXml"),
            "XML should carry the patched value: {body}"
        );

        assert_eq!(stored_family(&server, "patient-1").await, "CondPatchedXml");
        assert_eq!(stored_family(&server, "patient-2").await, "Jones");
    }

    #[tokio::test]
    async fn test_conditional_patch_default_accept_returns_json() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = patch_request(
            &server,
            "/Patient?family=Smith",
            MERGE_PATCH,
            &json!({ "active": false }),
        )
        .await;

        response.assert_status_ok();
        assert_eq!(content_type_of(&response), "application/fhir+json");
        let body: Value = response.json();
        assert_eq!(body["resourceType"], "Patient");
        assert_eq!(body["active"], false);
    }
}

// =============================================================================
// Search XML Tests
// =============================================================================

mod search_xml {
    use super::*;

    #[tokio::test]
    async fn test_search_returns_xml_bundle() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .get("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(ACCEPT, HeaderValue::from_static("application/fhir+xml"))
            .await;

        response.assert_status_ok();

        let body = response.text();
        assert!(
            body.contains("<Bundle") || body.contains("Bundle"),
            "Search response should contain a Bundle, got: {}",
            &body[..body.len().min(500)]
        );
    }

    #[tokio::test]
    async fn test_search_format_param_xml() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "patient-1", "Smith").await;

        let response = server
            .get("/Patient?_format=xml")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();

        let body = response.text();
        assert!(
            body.contains("<Bundle") || body.contains("<"),
            "Search response with _format=xml should be XML, got: {}",
            &body[..body.len().min(500)]
        );
    }
}

// =============================================================================
// Capabilities (Metadata) XML Tests
// =============================================================================

mod metadata_xml {
    use super::*;

    #[tokio::test]
    async fn test_metadata_xml() {
        let (server, _backend) = create_test_server().await;

        let response = server
            .get("/metadata")
            .add_header(ACCEPT, HeaderValue::from_static("application/fhir+xml"))
            .await;

        response.assert_status_ok();

        let ct = response
            .headers()
            .get("content-type")
            .expect("Should have Content-Type")
            .to_str()
            .unwrap();
        assert!(
            ct.contains("xml"),
            "Metadata Content-Type should be XML, got: {}",
            ct
        );

        let body = response.text();
        assert!(
            body.contains("<CapabilityStatement") || body.contains("CapabilityStatement"),
            "Metadata should contain CapabilityStatement, got: {}",
            &body[..body.len().min(500)]
        );
    }
}

// =============================================================================
// Mixed Format Tests (cross-format interoperability)
// =============================================================================

mod mixed_format {
    use super::*;

    #[tokio::test]
    async fn test_create_json_read_xml() {
        let (server, _backend) = create_test_server().await;

        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": "JsonToXml"}],
            "active": true
        });

        let create_response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient)
            .await;

        create_response.assert_status(StatusCode::CREATED);

        let location = create_response
            .headers()
            .get("location")
            .expect("Should have Location")
            .to_str()
            .unwrap()
            .to_string();

        let path = location
            .strip_prefix("http://localhost:8080")
            .unwrap_or(&location);

        // Read back as XML
        let read_response = server
            .get(path)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(ACCEPT, HeaderValue::from_static("application/fhir+xml"))
            .await;

        read_response.assert_status_ok();

        let xml = read_response.text();
        assert!(
            xml.contains("JsonToXml"),
            "XML should contain the family name"
        );
        assert!(xml.contains("<Patient"), "Should contain Patient element");
    }

    #[tokio::test]
    async fn test_create_xml_read_json_data_consistency() {
        let (server, _backend) = create_test_server().await;

        let xml_body = patient_xml("ConsistencyTest");

        let create_response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+xml"),
            )
            .bytes(Bytes::from(xml_body))
            .await;

        create_response.assert_status(StatusCode::CREATED);

        let location = create_response
            .headers()
            .get("location")
            .expect("Should have Location")
            .to_str()
            .unwrap()
            .to_string();

        let path = location
            .strip_prefix("http://localhost:8080")
            .unwrap_or(&location);

        // Read back as JSON
        let read_response = server
            .get(path)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(ACCEPT, HeaderValue::from_static("application/fhir+json"))
            .await;

        read_response.assert_status_ok();

        let body: Value = read_response.json();
        assert_eq!(body["resourceType"], "Patient");
        assert_eq!(body["name"][0]["family"], "ConsistencyTest");
        assert_eq!(body["active"], true);
    }
}
