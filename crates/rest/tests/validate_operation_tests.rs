//! Integration tests for the `$validate` operation.
//!
//! `POST [base]/[type]/$validate` (raw resource or Parameters wrapper),
//! `GET/POST [base]/[type]/[id]/$validate`, mode/profile handling, and the
//! always-200-OperationOutcome contract.

use std::path::PathBuf;
use std::sync::Arc;

use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::{MultitenancyConfig, ServerConfig, TenantRoutingMode};
use serde_json::{json, Value};

async fn create_test_server() -> (TestServer, Arc<SqliteBackend>) {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let backend_config = SqliteBackendConfig { data_dir: Some(data_dir), ..Default::default() };
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
    TenantContext::new(TenantId::new("test-tenant"), TenantPermissions::full_access())
}

fn issue_codes(outcome: &Value) -> Vec<(String, String)> {
    outcome["issue"]
        .as_array()
        .map(|issues| {
            issues
                .iter()
                .map(|i| {
                    (
                        i["severity"].as_str().unwrap_or_default().to_string(),
                        i["code"].as_str().unwrap_or_default().to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn validate_valid_patient_returns_all_clear() {
    let (server, _backend) = create_test_server().await;
    let response = server
        .post("/Patient/$validate")
        .json(&json!({
            "resourceType": "Patient",
            "name": [{ "family": "Smith", "given": ["Jan"] }],
            "gender": "female",
            "birthDate": "1980-02-29"
        }))
        .await;

    response.assert_status_ok();
    let outcome: Value = response.json();
    assert_eq!(outcome["resourceType"], "OperationOutcome");
    assert_eq!(
        issue_codes(&outcome),
        vec![("information".to_string(), "informational".to_string())],
        "clean validation reports the all-clear issue: {outcome:#}"
    );
}

#[tokio::test]
async fn validate_reports_structural_issues_with_expressions() {
    let (server, _backend) = create_test_server().await;
    let response = server
        .post("/Patient/$validate")
        .json(&json!({
            "resourceType": "Patient",
            "bogusElement": true,
            "gender": ["male"],
            "name": { "family": "NotAnArray" }
        }))
        .await;

    // Invalid resource is still a SUCCESSFUL validation: 200 + issues.
    response.assert_status_ok();
    let outcome: Value = response.json();
    let issues = outcome["issue"].as_array().expect("issues");
    assert!(
        issues.iter().any(|i| i["code"] == "structure"
            && i["expression"][0] == "Patient.bogusElement"),
        "unknown element issue with FHIRPath expression expected: {outcome:#}"
    );
    assert!(
        issues.iter().any(|i| i["expression"][0] == "Patient.gender"),
        "not-singular issue on gender expected: {outcome:#}"
    );
    assert!(
        issues.iter().any(|i| i["expression"][0] == "Patient.name"),
        "not-array issue on name expected: {outcome:#}"
    );
}

#[tokio::test]
async fn validate_accepts_parameters_wrapper() {
    let (server, _backend) = create_test_server().await;
    let response = server
        .post("/Patient/$validate")
        .json(&json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "mode", "valueCode": "create" },
                { "name": "resource", "resource": { "resourceType": "Patient", "active": true } }
            ]
        }))
        .await;

    response.assert_status_ok();
    let outcome: Value = response.json();
    assert_eq!(
        issue_codes(&outcome),
        vec![("information".to_string(), "informational".to_string())],
        "{outcome:#}"
    );
}

#[tokio::test]
async fn validate_mode_delete_skips_content_validation() {
    let (server, _backend) = create_test_server().await;
    // Delete validation needs no resource at all.
    let response = server
        .post("/Patient/$validate?mode=delete")
        .json(&json!({
            "resourceType": "Parameters",
            "parameter": []
        }))
        .await;
    response.assert_status_ok();
    let outcome: Value = response.json();
    assert_eq!(outcome["issue"][0]["severity"], "information", "{outcome:#}");
}

#[tokio::test]
async fn validate_mode_profile_requires_profile() {
    let (server, _backend) = create_test_server().await;
    let response = server
        .post("/Patient/$validate?mode=profile")
        .json(&json!({ "resourceType": "Patient" }))
        .await;
    response.assert_status_bad_request();
}

#[tokio::test]
async fn validate_rejects_unknown_mode() {
    let (server, _backend) = create_test_server().await;
    let response = server
        .post("/Patient/$validate?mode=bogus")
        .json(&json!({ "resourceType": "Patient" }))
        .await;
    response.assert_status_bad_request();
}

#[tokio::test]
async fn validate_rejects_type_mismatch() {
    let (server, _backend) = create_test_server().await;
    let response = server
        .post("/Patient/$validate")
        .json(&json!({ "resourceType": "Observation", "status": "final" }))
        .await;
    response.assert_status_bad_request();
}

#[tokio::test]
async fn validate_unknown_profile_reports_warning() {
    let (server, _backend) = create_test_server().await;
    let response = server
        .post("/Patient/$validate?profile=http://example.org/StructureDefinition/nope")
        .json(&json!({ "resourceType": "Patient" }))
        .await;
    response.assert_status_ok();
    let outcome: Value = response.json();
    assert!(
        issue_codes(&outcome)
            .iter()
            .any(|(sev, code)| sev == "warning" && code == "not-supported"),
        "unresolvable profile surfaces as a warning: {outcome:#}"
    );
}

#[tokio::test]
async fn validate_instance_get_validates_stored_resource() {
    let (server, backend) = create_test_server().await;
    let tenant = test_tenant();
    backend
        .create(
            &tenant,
            "Patient",
            json!({ "resourceType": "Patient", "id": "p1", "active": true }),
            FhirVersion::R4,
        )
        .await
        .expect("seed patient");

    let response = server.get("/Patient/p1/$validate").await;
    response.assert_status_ok();
    let outcome: Value = response.json();
    assert_eq!(outcome["resourceType"], "OperationOutcome", "{outcome:#}");
    assert!(
        !issue_codes(&outcome).iter().any(|(sev, _)| sev == "error"),
        "stored minimal patient must validate without errors: {outcome:#}"
    );
}

#[tokio::test]
async fn validate_instance_get_unknown_id_is_404() {
    let (server, _backend) = create_test_server().await;
    let response = server.get("/Patient/does-not-exist/$validate").await;
    response.assert_status_not_found();
}

#[tokio::test]
async fn validate_evaluates_real_fhirpath_invariants() {
    // End-to-end: the default ValidationService carries the FHIRPath
    // constraint evaluator, so the real spec invariant pat-1 ("contact
    // SHALL contain details or an organization reference") fires against
    // the embedded R4 pack.
    let (server, _backend) = create_test_server().await;
    let response = server
        .post("/Patient/$validate")
        .json(&json!({
            "resourceType": "Patient",
            "contact": [{ "gender": "male" }]
        }))
        .await;
    response.assert_status_ok();
    let outcome: Value = response.json();
    let issues = outcome["issue"].as_array().expect("issues");
    assert!(
        issues.iter().any(|i| i["code"] == "invariant"
            && i["severity"] == "error"
            && i["expression"][0] == "Patient.contact[0]"
            && i["details"]["text"].as_str().unwrap_or_default().contains("pat-1")),
        "pat-1 invariant issue expected: {outcome:#}"
    );

    // Satisfying the invariant clears it.
    let response = server
        .post("/Patient/$validate")
        .json(&json!({
            "resourceType": "Patient",
            "contact": [{ "gender": "male", "name": { "family": "Smith" } }]
        }))
        .await;
    response.assert_status_ok();
    let outcome: Value = response.json();
    assert!(
        !outcome["issue"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["code"] == "invariant" && i["severity"] == "error"),
        "satisfied contact must not fire pat-1: {outcome:#}"
    );
}

#[tokio::test]
async fn validate_instance_post_validates_body() {
    let (server, _backend) = create_test_server().await;
    let response = server
        .post("/Patient/p1/$validate?mode=update")
        .json(&json!({ "resourceType": "Patient", "id": "p1", "oops": 1 }))
        .await;
    response.assert_status_ok();
    let outcome: Value = response.json();
    assert!(
        outcome["issue"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["expression"][0] == "Patient.oops"),
        "{outcome:#}"
    );
}
