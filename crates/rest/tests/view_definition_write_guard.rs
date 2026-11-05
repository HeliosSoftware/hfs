//! Write-path guard against a ViewDefinition whose `resource` names no FHIR
//! resource type (#1014).
//!
//! A `resource` such as `"Nope"` can never run: `$sql-run` would answer zero
//! rows for every subsequent invocation, silently. Create, update and
//! conditional update reject it with `422` and the SoF linter's own
//! `OperationOutcome`, independent of `HFS_VALIDATION_MODE`. Every other
//! lint finding still stores the resource: the editor's "save it anyway"
//! flow (#821) keeps drafts on purpose.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode, ValidationConfig};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

/// Creates a test server backed by an in-memory SQLite database, wired to the
/// real workspace `data/` directory, with the given `HFS_VALIDATION_MODE`
/// equivalent. Mirrors `view_definition_search_params.rs`'s harness.
async fn create_test_server(mode: &str) -> (TestServer, Arc<SqliteBackend>) {
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

    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let server = TestServer::new(app).expect("Failed to create test server");

    (server, backend)
}

/// A structurally valid ViewDefinition draft (per the SQL-on-FHIR IG) with a
/// `resource` and `name` the caller can vary.
fn view_definition_with_resource(name: &str, resource: &str) -> Value {
    json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "name": name,
        "resource": resource,
        "select": [{"column": [{"name": "id", "path": "id"}]}]
    })
}

/// Asserts the single expected `unknown-resource-type` issue and that the
/// message names the offending value.
fn assert_unknown_resource_type_outcome(outcome: &Value) {
    assert_eq!(outcome["resourceType"], "OperationOutcome", "{outcome:#}");
    let issues = outcome["issue"].as_array().expect("issue array");
    assert_eq!(issues.len(), 1, "{outcome:#}");
    let issue = &issues[0];
    assert_eq!(issue["severity"], "error", "{outcome:#}");
    assert_eq!(issue["code"], "code-invalid", "{outcome:#}");
    assert_eq!(
        issue["details"]["coding"][0]["code"], "unknown-resource-type",
        "{outcome:#}"
    );
    assert!(
        issue["diagnostics"]
            .as_str()
            .unwrap_or_default()
            .contains("Nope"),
        "{outcome:#}"
    );
}

#[tokio::test]
async fn create_rejects_a_view_definition_with_an_unknown_resource() {
    let (server, _backend) = create_test_server("off").await;

    let response = server
        .post("/ViewDefinition")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .json(&view_definition_with_resource("nope_view", "Nope"))
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    let outcome: Value = response.json();
    assert_unknown_resource_type_outcome(&outcome);

    // Nothing was stored.
    let search = server
        .get("/ViewDefinition?name=nope_view")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await;
    search.assert_status_ok();
    let bundle: Value = search.json();
    assert_eq!(
        bundle["entry"].as_array().map(Vec::len).unwrap_or_default(),
        0,
        "{bundle:#}"
    );
}

#[tokio::test]
async fn update_rejects_an_unknown_resource_and_keeps_the_stored_version() {
    let (server, _backend) = create_test_server("off").await;

    let created = server
        .post("/ViewDefinition")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .json(&view_definition_with_resource(
            "keeps_stored_version",
            "Patient",
        ))
        .await;
    created.assert_status(StatusCode::CREATED);
    let created_body: Value = created.json();
    let id = created_body["id"].as_str().expect("id").to_string();
    let version_id = created_body["meta"]["versionId"].clone();

    let mut update_body = view_definition_with_resource("keeps_stored_version", "Nope");
    update_body["id"] = json!(id);
    let response = server
        .put(&format!("/ViewDefinition/{id}"))
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .json(&update_body)
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    let outcome: Value = response.json();
    assert_unknown_resource_type_outcome(&outcome);

    let read = server
        .get(&format!("/ViewDefinition/{id}"))
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await;
    read.assert_status_ok();
    let read_body: Value = read.json();
    assert_eq!(read_body["resource"], "Patient", "{read_body:#}");
    assert_eq!(read_body["meta"]["versionId"], version_id, "{read_body:#}");
}

#[tokio::test]
async fn conditional_update_rejects_an_unknown_resource() {
    let (server, _backend) = create_test_server("off").await;

    let response = server
        .put("/ViewDefinition?name=conditional_nope_view")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .json(&view_definition_with_resource(
            "conditional_nope_view",
            "Nope",
        ))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    let outcome: Value = response.json();
    assert_unknown_resource_type_outcome(&outcome);

    let search = server
        .get("/ViewDefinition?name=conditional_nope_view")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await;
    search.assert_status_ok();
    let bundle: Value = search.json();
    assert_eq!(
        bundle["entry"].as_array().map(Vec::len).unwrap_or_default(),
        0,
        "{bundle:#}"
    );
}

#[tokio::test]
async fn create_still_stores_a_draft_with_other_lint_errors() {
    let (server, _backend) = create_test_server("off").await;

    // `resource: "Patient"` is a known type, but `select` is missing — a
    // `MissingRequired` lint finding, not `UnknownResourceType`. The write
    // guard is a no-op here; the draft is stored so the editor's "save it
    // anyway" flow (#821) keeps working.
    let draft = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "name": "draft_without_select",
        "resource": "Patient"
    });
    let response = server
        .post("/ViewDefinition")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .json(&draft)
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::CREATED,
        "{}",
        response.text()
    );
}

#[tokio::test]
async fn create_of_other_resource_types_is_untouched() {
    let (server, _backend) = create_test_server("off").await;

    let response = server
        .post("/Patient")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .json(&json!({ "resourceType": "Patient", "active": true }))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::CREATED,
        "{}",
        response.text()
    );
}

/// `check_write` (`HFS_VALIDATION_MODE=enforce`) runs before the write guard,
/// so an `enforce`-mode response for an unknown `resource` could in principle
/// be answered by the validator instead of the linter. This test only pins
/// the observable contract from the client's point of view: `422` with an
/// `OperationOutcome` carrying at least one `error`-severity issue. If the
/// linter's own `code-invalid` / `unknown-resource-type` issue is not what
/// comes back, that is worth a follow-up, not a failure here.
#[tokio::test]
async fn enforce_mode_still_answers_the_lint_outcome_for_an_unknown_resource() {
    let (server, _backend) = create_test_server("enforce").await;

    let response = server
        .post("/ViewDefinition")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .json(&view_definition_with_resource("enforce_nope_view", "Nope"))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    let outcome: Value = response.json();
    assert_eq!(outcome["resourceType"], "OperationOutcome", "{outcome:#}");
    let issues = outcome["issue"].as_array().expect("issue array");
    assert!(
        issues.iter().any(|issue| issue["severity"] == "error"),
        "{outcome:#}"
    );
}
