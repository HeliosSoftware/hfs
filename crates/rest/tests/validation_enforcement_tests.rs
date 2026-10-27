//! Write-path validation enforcement (`HFS_VALIDATION_MODE`) and
//! stored-profile (tenant StructureDefinition registry) integration tests.

use std::path::PathBuf;
use std::sync::Arc;

use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::config::ValidationConfig;
use helios_rest::{MultitenancyConfig, ServerConfig, TenantRoutingMode};
use serde_json::{Value, json};

async fn create_test_server(mode: &str) -> TestServer {
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
        validation: ValidationConfig {
            mode: mode.to_string(),
            ..Default::default()
        },
        ..ServerConfig::for_testing()
    };

    let state = helios_rest::AppState::new(backend, config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    TestServer::new(app).expect("Failed to create test server")
}

fn invalid_patient() -> Value {
    json!({ "resourceType": "Patient", "bogusElement": true })
}

fn valid_patient() -> Value {
    json!({ "resourceType": "Patient", "active": true })
}

#[tokio::test]
async fn enforce_mode_rejects_invalid_writes_with_outcome() {
    let server = create_test_server("enforce").await;

    let response = server.post("/Patient").json(&invalid_patient()).await;
    response.assert_status_unprocessable_entity();
    let outcome: Value = response.json();
    assert_eq!(outcome["resourceType"], "OperationOutcome", "{outcome:#}");
    assert!(
        outcome["issue"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["code"] == "structure" && i["expression"][0] == "Patient.bogusElement"),
        "expected the structural issue in the 422 body: {outcome:#}"
    );

    // Updates are enforced too.
    let response = server
        .put("/Patient/p1")
        .json(&json!({ "resourceType": "Patient", "id": "p1", "bogusElement": true }))
        .await;
    response.assert_status_unprocessable_entity();
}

#[tokio::test]
async fn enforce_mode_allows_valid_writes() {
    let server = create_test_server("enforce").await;
    let response = server.post("/Patient").json(&valid_patient()).await;
    assert_eq!(response.status_code(), 201, "{}", response.text());
}

#[tokio::test]
async fn log_and_off_modes_do_not_reject() {
    for mode in ["log", "off"] {
        let server = create_test_server(mode).await;
        let response = server.post("/Patient").json(&invalid_patient()).await;
        assert_eq!(
            response.status_code(),
            201,
            "mode={mode} must not reject: {}",
            response.text()
        );
    }
}

#[tokio::test]
async fn enforce_mode_rejects_invalid_batch_entries_individually() {
    let server = create_test_server("enforce").await;
    let response = server
        .post("/")
        .json(&json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [
                {
                    "request": { "method": "POST", "url": "Patient" },
                    "resource": invalid_patient()
                },
                {
                    "request": { "method": "POST", "url": "Patient" },
                    "resource": valid_patient()
                }
            ]
        }))
        .await;
    response.assert_status_ok();
    let bundle: Value = response.json();
    let entries = bundle["entry"].as_array().expect("entries");
    assert_eq!(entries.len(), 2);
    assert!(
        entries[0]["response"]["status"]
            .as_str()
            .unwrap_or_default()
            .starts_with("422"),
        "first entry must fail validation: {bundle:#}"
    );
    assert!(
        entries[1]["response"]["status"]
            .as_str()
            .unwrap_or_default()
            .starts_with("201"),
        "second entry must succeed: {bundle:#}"
    );
}

/// The write validator's own issues reach a batch entry, not a flattened
/// sentence.
///
/// `check_write` returns a fully-formed multi-issue OperationOutcome — one
/// issue per finding, each with its own code, severity and `expression` giving
/// the FHIRPath location of the element that failed. The batch arm used to walk
/// `issue[].details.text`, join the strings with `"; "`, and hand one sentence
/// to a wrapper that stamped `processing` over it: N coded, located issues
/// became one uncoded, unlocated issue.
///
/// The transaction arm never did that — it propagates the same error from the
/// same call with a bare `?`, reaching `IntoResponse`'s pass-through. So an
/// identical resource returned typed codes and FHIRPath expressions as a
/// `transaction` and one English sentence as a `batch`, decided purely by
/// `Bundle.type` (#504).
///
/// Complements `enforce_mode_rejects_invalid_writes_with_outcome` above, which
/// pinned the single-resource half and left the batch half asserting only a
/// status.
#[tokio::test]
async fn the_two_surfaces_report_the_same_validation_issues() {
    let server = create_test_server("enforce").await;

    /// Every `(code, expression)` pair in an outcome, sorted.
    fn issue_keys(outcome: &Value) -> Vec<(String, String)> {
        let mut keys: Vec<(String, String)> = outcome["issue"]
            .as_array()
            .expect("issue array")
            .iter()
            .map(|i| {
                (
                    i["code"].as_str().unwrap_or_default().to_string(),
                    i["expression"][0].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        keys.sort();
        keys
    }

    let single: Value = server
        .post("/Patient")
        .json(&invalid_patient())
        .await
        .json();

    let batch: Value = server
        .post("/")
        .json(&json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "POST", "url": "Patient" },
                "resource": invalid_patient()
            }]
        }))
        .await
        .json();
    let entry = &batch["entry"][0]["response"];

    assert!(
        entry["status"]
            .as_str()
            .unwrap_or_default()
            .starts_with("422"),
        "entry status: {batch:#}"
    );

    let entry_outcome = &entry["outcome"];
    assert_eq!(
        entry_outcome["resourceType"], "OperationOutcome",
        "{batch:#}"
    );

    let single_keys = issue_keys(&single);
    let entry_keys = issue_keys(entry_outcome);
    assert_eq!(
        entry_keys, single_keys,
        "a batch entry must report the issues the resource endpoint reports\n\
         single: {single:#}\nentry: {entry_outcome:#}"
    );

    // Pinned literally, not just to each other: a regression that flattened
    // *both* surfaces would satisfy the equality above vacuously.
    assert!(
        entry_keys.contains(&("structure".to_string(), "Patient.bogusElement".to_string())),
        "the structural issue must survive with its FHIRPath location: {entry_outcome:#}"
    );
}

#[tokio::test]
async fn stored_profile_registers_and_validates() {
    let server = create_test_server("off").await;

    // Upload a differential profile constraining Patient: birthDate 1..1,
    // gender prohibited.
    let profile = json!({
        "resourceType": "StructureDefinition",
        "id": "strict-patient",
        "url": "http://example.org/StructureDefinition/strict-patient",
        "name": "StrictPatient",
        "status": "active",
        "kind": "resource",
        "abstract": false,
        "type": "Patient",
        "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Patient",
        "derivation": "constraint",
        "differential": {
            "element": [
                { "id": "Patient", "path": "Patient" },
                { "id": "Patient.birthDate", "path": "Patient.birthDate", "min": 1 },
                { "id": "Patient.gender", "path": "Patient.gender", "max": "0" }
            ]
        }
    });
    let response = server
        .put("/StructureDefinition/strict-patient")
        .json(&profile)
        .await;
    assert!(
        response.status_code() == 201 || response.status_code() == 200,
        "profile upload failed: {}",
        response.text()
    );

    // $validate against the stored profile: violations reported.
    let response = server
        .post("/Patient/$validate?profile=http://example.org/StructureDefinition/strict-patient")
        .json(&json!({ "resourceType": "Patient", "gender": "male" }))
        .await;
    response.assert_status_ok();
    let outcome: Value = response.json();
    let issues = outcome["issue"].as_array().expect("issues");
    assert!(
        issues
            .iter()
            .any(|i| i["code"] == "required" && i["expression"][0] == "Patient.birthDate"),
        "profile-required birthDate must be reported: {outcome:#}"
    );
    assert!(
        issues
            .iter()
            .any(|i| i["code"] == "structure" && i["expression"][0] == "Patient.gender"),
        "profile-excluded gender must be reported: {outcome:#}"
    );

    // A conforming resource passes.
    let response = server
        .post("/Patient/$validate?profile=http://example.org/StructureDefinition/strict-patient")
        .json(&json!({ "resourceType": "Patient", "birthDate": "1980-01-01" }))
        .await;
    response.assert_status_ok();
    let outcome: Value = response.json();
    assert_eq!(
        outcome["issue"][0]["severity"], "information",
        "conforming resource must validate clean: {outcome:#}"
    );
}

#[tokio::test]
async fn enforce_mode_honors_meta_profile_claims() {
    let server = create_test_server("enforce").await;

    let profile = json!({
        "resourceType": "StructureDefinition",
        "id": "must-have-birthdate",
        "url": "http://example.org/StructureDefinition/must-have-birthdate",
        "name": "MustHaveBirthDate",
        "status": "active",
        "kind": "resource",
        "abstract": false,
        "type": "Patient",
        "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Patient",
        "derivation": "constraint",
        "differential": {
            "element": [
                { "id": "Patient", "path": "Patient" },
                { "id": "Patient.birthDate", "path": "Patient.birthDate", "min": 1 }
            ]
        }
    });
    let response = server
        .put("/StructureDefinition/must-have-birthdate")
        .json(&profile)
        .await;
    assert!(
        response.status_code().is_success(),
        "profile upload failed: {}",
        response.text()
    );

    // A write claiming the profile but violating it is rejected.
    let response = server
        .post("/Patient")
        .json(&json!({
            "resourceType": "Patient",
            "meta": { "profile": ["http://example.org/StructureDefinition/must-have-birthdate"] }
        }))
        .await;
    response.assert_status_unprocessable_entity();

    // Satisfying the claimed profile passes.
    let response = server
        .post("/Patient")
        .json(&json!({
            "resourceType": "Patient",
            "meta": { "profile": ["http://example.org/StructureDefinition/must-have-birthdate"] },
            "birthDate": "1980-01-01"
        }))
        .await;
    assert_eq!(response.status_code(), 201, "{}", response.text());
}
