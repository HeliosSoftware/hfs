//! #1769: a scalar `string` / `code` column of a `$sql-run` keeps its stored
//! text in every output format. The in-DB runners used to parse each text
//! value as JSON, so `"44054006"` came back as a number, `"true"` as a boolean
//! and `"null"` as `null` (an empty cell in CSV).

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

const CODES: [&str; 6] = ["44054006", "0123", "4548-4", "true", "null", "1e3"];

async fn server_with_codes() -> TestServer {
    let backend = SqliteBackend::with_config(":memory:", Default::default()).expect("sqlite");
    backend.init_schema().expect("schema");
    let backend = Arc::new(backend);
    let tenant = TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    );
    for (i, code) in CODES.iter().enumerate() {
        let resource = json!({
            "resourceType": "Condition", "id": format!("c{i}"),
            "subject": {"reference": "Patient/p1"},
            "code": {"coding": [{"system": "http://example.org/cs", "code": code}]}
        });
        backend
            .create(&tenant, "Condition", resource, FhirVersion::R4)
            .await
            .expect("seed");
    }
    let runner = backend.sof_runner().expect("in-DB runner");
    let state = helios_rest::AppState::new(Arc::clone(&backend), ServerConfig::for_testing())
        .with_sof_runner(runner);
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).expect("server")
}

fn condition_codes(code_column: Value) -> Value {
    json!({
        "resourceType": "ViewDefinition",
        "name": "condition_codes",
        "status": "active",
        "resource": "Condition",
        "select": [{"column": [
            {"name": "id", "path": "getResourceKey()", "type": "id"},
            code_column
        ]}]
    })
}

async fn run(server: &TestServer, format: &str, view: &Value) -> String {
    let response = server
        .post(&format!("/$sql-run?_format={format}"))
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(view)
        .await;
    assert_eq!(response.status_code(), 200, "{}", response.text());
    response.text()
}

/// Every row's `code` must be the JSON string stored for that `id`.
fn assert_codes_are_strings(rows: &[Value]) {
    assert_eq!(rows.len(), CODES.len(), "{rows:?}");
    for (i, code) in CODES.iter().enumerate() {
        let id = format!("c{i}");
        let row = rows
            .iter()
            .find(|r| r["id"] == id.as_str())
            .unwrap_or_else(|| panic!("row {id} missing from {rows:?}"));
        assert_eq!(row["code"], Value::String((*code).into()), "{row}");
    }
}

fn column_variants() -> Vec<(&'static str, Value)> {
    let path = "code.coding.first().code";
    vec![
        (
            "type code",
            json!({"name": "code", "path": path, "type": "code"}),
        ),
        (
            "type string",
            json!({"name": "code", "path": path, "type": "string"}),
        ),
        ("no type", json!({"name": "code", "path": path})),
    ]
}

#[tokio::test]
async fn json_keeps_every_code_as_a_string() {
    let server = server_with_codes().await;
    for (_label, column) in column_variants() {
        let body = run(&server, "json", &condition_codes(column)).await;
        let rows: Vec<Value> = serde_json::from_str(&body).expect("json rows");
        assert_codes_are_strings(&rows);
    }
}

#[tokio::test]
async fn ndjson_keeps_every_code_as_a_string() {
    let server = server_with_codes().await;
    for (_label, column) in column_variants() {
        let body = run(&server, "ndjson", &condition_codes(column)).await;
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("ndjson row"))
            .collect();
        assert_codes_are_strings(&rows);
    }
}

#[tokio::test]
async fn csv_writes_the_text_null_instead_of_an_empty_cell() {
    let server = server_with_codes().await;
    for (_label, column) in column_variants() {
        let body = run(&server, "csv", &condition_codes(column)).await;
        let mut lines = body.lines();
        assert_eq!(lines.next(), Some("id,code"), "{body}");
        let cells: Vec<&str> = lines.collect();
        for (i, code) in CODES.iter().enumerate() {
            let expected = format!("c{i},{code}");
            assert!(
                cells.contains(&expected.as_str()),
                "expected line {expected:?} in {body}"
            );
        }
    }
}
