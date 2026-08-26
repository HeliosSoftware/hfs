//! Resource-type admission tests for REST write paths.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode, ValidationConfig};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

async fn create_test_server(
    validation_mode: &str,
    default_fhir_version: FhirVersion,
) -> (TestServer, Arc<SqliteBackend>) {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(|path| path.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("create SQLite backend");
    backend.init_schema().expect("initialize SQLite schema");
    let backend = Arc::new(backend);

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        default_fhir_version,
        validation: ValidationConfig {
            mode: validation_mode.to_string(),
            ..Default::default()
        },
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    (TestServer::new(app).expect("create test server"), backend)
}

fn tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
}

async fn put(server: &TestServer, path: &str, body: Value) -> axum_test::TestResponse {
    server
        .put(path)
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&body)
        .await
}

#[tokio::test]
async fn direct_updates_reject_unknown_wrong_case_missing_and_mismatched_types() {
    let (server, backend) = create_test_server("off", FhirVersion::default_enabled()).await;

    put(
        &server,
        "/NoLongerValid/unknown",
        json!({ "resourceType": "NoLongerValid", "id": "unknown" }),
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);
    put(
        &server,
        "/patient/lowercase",
        json!({ "resourceType": "patient", "id": "lowercase" }),
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);
    put(&server, "/Patient/missing", json!({ "id": "missing" }))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    put(
        &server,
        "/Patient/mismatch",
        json!({ "resourceType": "Observation", "id": "mismatch" }),
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);

    for (resource_type, id) in [
        ("NoLongerValid", "unknown"),
        ("patient", "lowercase"),
        ("Patient", "missing"),
        ("Patient", "mismatch"),
    ] {
        assert!(
            backend
                .read(&tenant(), resource_type, id)
                .await
                .unwrap()
                .is_none(),
            "{resource_type}/{id} must not be stored"
        );
    }
}

#[tokio::test]
async fn type_admission_is_independent_of_validation_mode() {
    for mode in ["off", "enforce"] {
        let (server, backend) = create_test_server(mode, FhirVersion::default_enabled()).await;
        put(
            &server,
            "/NoLongerValid/rejected",
            json!({ "resourceType": "NoLongerValid", "id": "rejected" }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
        assert!(
            backend
                .read(&tenant(), "NoLongerValid", "rejected")
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn conditional_update_keeps_audit_event_immutable() {
    let (server, backend) = create_test_server("off", FhirVersion::default_enabled()).await;
    put(
        &server,
        "/AuditEvent?entity=Patient%2Fexample",
        json!({ "resourceType": "AuditEvent" }),
    )
    .await
    .assert_status(StatusCode::METHOD_NOT_ALLOWED);
    assert!(
        backend
            .read(&tenant(), "AuditEvent", "example")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn conditional_delete_keeps_audit_event_immutable() {
    let (server, backend) = create_test_server("off", FhirVersion::default_enabled()).await;
    backend
        .create(
            &tenant(),
            "AuditEvent",
            json!({ "resourceType": "AuditEvent", "id": "audit-1" }),
            FhirVersion::default_enabled(),
        )
        .await
        .expect("seed AuditEvent");

    let response = server
        .delete("/AuditEvent?_id=audit-1")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await;
    response.assert_status(StatusCode::METHOD_NOT_ALLOWED);
    assert!(
        backend
            .read(&tenant(), "AuditEvent", "audit-1")
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn capability_statement_does_not_advertise_audit_event_mutation() {
    let (server, _) = create_test_server("off", FhirVersion::default_enabled()).await;
    let response = server.get("/metadata").await;
    response.assert_status_ok();
    let body: Value = response.json();
    let audit = body["rest"][0]["resource"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["type"] == "AuditEvent")
        .expect("AuditEvent capability");
    let interactions: Vec<&str> = audit["interaction"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|interaction| interaction["code"].as_str())
        .collect();

    for mutation in ["create", "update", "patch", "delete"] {
        assert!(!interactions.contains(&mutation));
    }
    for flag in [
        "updateCreate",
        "conditionalCreate",
        "conditionalUpdate",
        "conditionalDelete",
    ] {
        assert!(audit.get(flag).is_none(), "{flag} must be omitted");
    }
}

#[cfg(all(feature = "R4", any(feature = "R5", feature = "R6")))]
#[tokio::test]
async fn direct_update_rejects_a_type_from_another_compiled_version() {
    let (server, backend) = create_test_server("off", FhirVersion::R4).await;
    put(
        &server,
        "/ActorDefinition/r5-only",
        json!({ "resourceType": "ActorDefinition", "id": "r5-only" }),
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);
    assert!(
        backend
            .read(&tenant(), "ActorDefinition", "r5-only")
            .await
            .unwrap()
            .is_none()
    );
}

#[cfg(all(feature = "R4", feature = "R6"))]
#[tokio::test]
async fn r6_rejects_a_resource_removed_after_r4() {
    let (server, backend) = create_test_server("off", FhirVersion::R6).await;
    put(
        &server,
        "/Media/r4-only",
        json!({ "resourceType": "Media", "id": "r4-only" }),
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);
    assert!(
        backend
            .read(&tenant(), "Media", "r4-only")
            .await
            .unwrap()
            .is_none()
    );
}
