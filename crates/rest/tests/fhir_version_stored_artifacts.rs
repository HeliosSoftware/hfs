//! Search and `$sql-run` honor the request's FHIR version (#2254).
//!
//! The UI's ViewDefinitions / SQL Queries / SQL Views pages list, open and
//! run user-authored artifacts that exist only in storage. A request naming a
//! `fhirVersion` MIME parameter — on `Accept` for a search, on `Content-Type`
//! (else `Accept`) for `$sql-run` — must see only the resources stored under
//! that version; a request naming none keeps the version-blind behavior.
//!
//! Runs the REST stack over in-memory SQLite with its in-DB SoF runner.

#![cfg(all(feature = "R4", feature = "R5", feature = "sqlite"))]

use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::ResourceStorage;
use helios_rest::ServerConfig;
use serde_json::{Value, json};

const ACCEPT: HeaderName = HeaderName::from_static("accept");

fn fhir_json_mime(version: &str) -> String {
    format!("application/fhir+json; fhirVersion={version}")
}

fn fhir_json(version: &str) -> HeaderValue {
    HeaderValue::from_str(&fhir_json_mime(version)).unwrap()
}

fn body(value: &Value) -> axum::body::Bytes {
    serde_json::to_vec(value).unwrap().into()
}

fn create_test_server() -> TestServer {
    let backend = SqliteBackend::with_config(":memory:", Default::default())
        .expect("failed to create SQLite backend");
    backend.init_schema().expect("failed to init schema");
    let backend = Arc::new(backend);
    let runner = backend
        .sof_runner()
        .expect("SqliteBackend must provide an in-DB SOF runner");
    let state = helios_rest::AppState::new(Arc::clone(&backend), ServerConfig::for_testing())
        .with_sof_runner(runner);
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
        .expect("failed to create test server")
}

async fn put(server: &TestServer, resource: Value, version: &str) {
    let path = format!(
        "/{}/{}",
        resource["resourceType"].as_str().unwrap(),
        resource["id"].as_str().unwrap()
    );
    let response = server
        .put(&path)
        .bytes(body(&resource))
        .content_type(&fhir_json_mime(version))
        .await;
    assert!(
        response.status_code().is_success(),
        "PUT {path} under {version}: {}",
        response.text()
    );
}

fn view_definition(id: &str) -> Value {
    json!({
        "resourceType": "ViewDefinition",
        "id": id,
        "name": id.replace('-', "_"),
        "status": "active",
        "resource": "Patient",
        "select": [{"column": [{"name": "id", "path": "getResourceKey()"}]}]
    })
}

async fn seed(server: &TestServer) {
    put(server, view_definition("vd-r4"), "4.0").await;
    put(server, view_definition("vd-r5"), "5.0").await;
    put(
        server,
        json!({"resourceType": "Patient", "id": "a4"}),
        "4.0",
    )
    .await;
    put(
        server,
        json!({"resourceType": "Patient", "id": "a5"}),
        "5.0",
    )
    .await;
    put(
        server,
        json!({"resourceType": "Patient", "id": "b4"}),
        "4.0",
    )
    .await;
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

async fn search_ids(server: &TestServer, accept: Option<&str>) -> Vec<String> {
    let mut request = server
        .get("/ViewDefinition")
        .add_query_param("_sort", "name");
    if let Some(version) = accept {
        request = request.add_header(ACCEPT, fhir_json(version));
    }
    let response = request.await;
    response.assert_status(StatusCode::OK);
    let bundle: Value = response.json();
    bundle["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                // Skip any OperationOutcome entry (e.g. a `_sort` warning).
                .filter(|e| e["resource"]["resourceType"] == "ViewDefinition")
                .map(|e| e["resource"]["id"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn search_names_a_version_and_sees_only_its_artifacts() {
    let server = create_test_server();
    seed(&server).await;

    assert_eq!(search_ids(&server, Some("5.0")).await, ["vd-r5"]);
    assert_eq!(search_ids(&server, Some("4.0")).await, ["vd-r4"]);
    // Naming no version keeps the version-blind search.
    assert_eq!(sorted(search_ids(&server, None).await), ["vd-r4", "vd-r5"]);

    // The accurate total counts the version's matches only.
    let response = server
        .get("/Patient")
        .add_query_param("_total", "accurate")
        .add_header(ACCEPT, fhir_json("4.0"))
        .await;
    response.assert_status(StatusCode::OK);
    assert_eq!(response.json::<Value>()["total"], 2);
}

async fn run_ids(server: &TestServer, content_version: Option<&str>) -> Vec<String> {
    let parameters = json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "subjectResource", "resource": view_definition("inline")}]
    });
    let content_type = content_version
        .map(fhir_json_mime)
        .unwrap_or_else(|| "application/json".to_string());
    let response = server
        .post("/$sql-run")
        .add_query_param("_format", "json")
        .add_header(ACCEPT, HeaderValue::from_static("application/json"))
        .bytes(body(&parameters))
        .content_type(&content_type)
        .await;
    response.assert_status(StatusCode::OK);
    let rows: Vec<Value> = response.json();
    sorted(
        rows.iter()
            .map(|row| row["id"].as_str().unwrap().to_string())
            .collect(),
    )
}

#[tokio::test]
async fn sql_run_names_a_version_and_reads_only_its_resources() {
    let server = create_test_server();
    seed(&server).await;

    assert_eq!(run_ids(&server, Some("5.0")).await, ["a5"]);
    assert_eq!(run_ids(&server, Some("4.0")).await, ["a4", "b4"]);
    assert_eq!(run_ids(&server, None).await, ["a4", "a5", "b4"]);
}
