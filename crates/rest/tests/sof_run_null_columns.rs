//! #1569: a `$sql-run` whose first row (by `last_updated, id`) is a resource
//! without some of the view's elements must still answer every column — the
//! CSV header and every JSON row carry `gender`, `birth_date` and `city`, with
//! `null` where the resource has no value. The in-DB SQLite runner used to
//! drop a row's NULL columns, and the formatters take the column list from the
//! first row, so a bare first Patient cut the whole result down to `id,family`.

use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

async fn server_with_bare_patient_first() -> TestServer {
    let backend = SqliteBackend::with_config(":memory:", Default::default()).expect("sqlite");
    backend.init_schema().expect("schema");
    let backend = Arc::new(backend);
    let tenant = TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    );
    for resource in [
        json!({"resourceType": "Patient", "id": "bare", "name": [{"family": "Bare"}]}),
        json!({
            "resourceType": "Patient", "id": "full", "active": true, "gender": "female",
            "birthDate": "2015-12-29", "name": [{"family": "Parker433"}],
            "address": [{"city": "Everett"}]
        }),
    ] {
        backend
            .create(&tenant, "Patient", resource, FhirVersion::R4)
            .await
            .expect("seed");
    }
    let runner = backend.sof_runner().expect("in-DB runner");
    let state = helios_rest::AppState::new(Arc::clone(&backend), ServerConfig::for_testing())
        .with_sof_runner(runner);
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).expect("server")
}

fn patient_demographics() -> Value {
    json!({
        "resourceType": "ViewDefinition",
        "name": "patient_demographics",
        "status": "active",
        "resource": "Patient",
        "select": [{"column": [
            {"name": "id", "path": "getResourceKey()", "type": "id"},
            {"name": "gender", "path": "gender"},
            {"name": "birth_date", "path": "birthDate", "type": "date"},
            {"name": "family", "path": "name.first().family"},
            {"name": "city", "path": "address.first().city"}
        ]}],
        "where": [{"path": "active.exists().not() or active = true"}]
    })
}

#[tokio::test]
async fn the_csv_header_lists_every_column_when_the_first_row_is_bare() {
    let server = server_with_bare_patient_first().await;
    let response = server
        .post("/$sql-run?_format=csv")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&patient_demographics())
        .await;
    assert_eq!(response.status_code(), 200, "{}", response.text());
    let body = response.text();
    let mut lines = body.lines();
    assert_eq!(
        lines.next(),
        Some("id,gender,birth_date,family,city"),
        "{body}"
    );
    assert!(
        body.contains("female,2015-12-29,Parker433,Everett"),
        "the full row keeps its values: {body}"
    );
    assert!(
        body.contains(",,,Bare,"),
        "the bare row is empty, not absent: {body}"
    );
}

#[tokio::test]
async fn every_json_row_carries_every_column_with_null_for_a_missing_value() {
    let server = server_with_bare_patient_first().await;
    let response = server
        .post("/$sql-run?_format=json")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&patient_demographics())
        .await;
    assert_eq!(response.status_code(), 200, "{}", response.text());
    let rows: Vec<Value> = response.json();
    assert_eq!(rows.len(), 2, "{rows:?}");
    for row in &rows {
        for column in ["id", "gender", "birth_date", "family", "city"] {
            assert!(row.get(column).is_some(), "{column} missing from {row}");
        }
    }
    let bare = rows
        .iter()
        .find(|r| r["family"] == "Bare")
        .expect("bare row");
    assert_eq!(bare["gender"], Value::Null, "{bare}");
    let full = rows
        .iter()
        .find(|r| r["family"] == "Parker433")
        .expect("full row");
    assert_eq!(full["gender"], "female", "{full}");
    assert_eq!(full["city"], "Everett", "{full}");
}
