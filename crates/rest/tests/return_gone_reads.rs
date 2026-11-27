//! `HFS_RETURN_GONE` on the read paths of a deleted resource (#2526).
//!
//! With the flag off, `GET [type]/[id]`, `HEAD [type]/[id]`, a conditional
//! read, a batch `GET` entry and vread of the delete's version all answer
//! `404 Not Found` with `code: not-found`, exactly as an id that never
//! existed. With the default `true`, each answers `410 Gone`.

#![cfg(feature = "sqlite")]

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_rest::ServerConfig;
use serde_json::{Value, json};

fn server(return_gone: bool) -> TestServer {
    let backend = SqliteBackend::in_memory().expect("create SQLite backend");
    backend.init_schema().expect("init schema");
    let config = ServerConfig {
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "default".to_string(),
        return_gone,
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::new(backend), config);
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
        .expect("create test server")
}

async fn seed_and_delete(server: &TestServer, id: &str) {
    let put = server
        .put(&format!("/Patient/{id}"))
        .json(&json!({"resourceType": "Patient", "id": id}))
        .await;
    assert_eq!(put.status_code(), StatusCode::CREATED, "{}", put.text());
    let delete = server.delete(&format!("/Patient/{id}")).await;
    assert_eq!(
        delete.status_code(),
        StatusCode::NO_CONTENT,
        "{}",
        delete.text()
    );
}

fn assert_outcome_code(body: &Value, code: &str) {
    assert_eq!(
        body["issue"][0]["code"].as_str(),
        Some(code),
        "unexpected outcome: {body}"
    );
}

#[tokio::test]
async fn read_of_deleted_is_404_with_return_gone_off() {
    let server = server(false);
    seed_and_delete(&server, "g1").await;

    let response = server.get("/Patient/g1").await;
    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    assert_outcome_code(&response.json(), "not-found");

    let never = server.get("/Patient/never").await;
    assert_eq!(never.status_code(), StatusCode::NOT_FOUND);
    assert_eq!(
        response.json::<Value>()["issue"][0]["code"],
        never.json::<Value>()["issue"][0]["code"]
    );
}

#[tokio::test]
async fn head_of_deleted_is_404_with_return_gone_off() {
    let server = server(false);
    seed_and_delete(&server, "g1").await;

    let response = server.method(Method::HEAD, "/Patient/g1").await;
    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn conditional_read_of_deleted_is_404_with_return_gone_off() {
    let server = server(false);
    seed_and_delete(&server, "g1").await;

    let response = server
        .get("/Patient/g1")
        .add_header("If-None-Match", "W/\"1\"")
        .await;
    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    assert_outcome_code(&response.json(), "not-found");
}

#[tokio::test]
async fn batch_get_of_deleted_is_404_with_return_gone_off() {
    let server = server(false);
    seed_and_delete(&server, "g1").await;

    let response = server
        .post("/")
        .json(&json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{"request": {"method": "GET", "url": "Patient/g1"}}]
        }))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let body: Value = response.json();
    assert_eq!(
        body["entry"][0]["response"]["status"].as_str(),
        Some("404 Not Found"),
        "unexpected entry: {body}"
    );
    assert_outcome_code(&body["entry"][0]["response"]["outcome"], "not-found");
}

#[tokio::test]
async fn vread_of_delete_version_is_404_with_return_gone_off() {
    let server = server(false);
    seed_and_delete(&server, "g1").await;

    let response = server.get("/Patient/g1/_history/2").await;
    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    assert_outcome_code(&response.json(), "not-found");
}

#[tokio::test]
async fn reads_of_deleted_are_410_by_default() {
    let server = server(true);
    seed_and_delete(&server, "g1").await;

    let read = server.get("/Patient/g1").await;
    assert_eq!(read.status_code(), StatusCode::GONE);
    assert_outcome_code(&read.json(), "deleted");

    let head = server.method(Method::HEAD, "/Patient/g1").await;
    assert_eq!(head.status_code(), StatusCode::GONE);

    let conditional = server
        .get("/Patient/g1")
        .add_header("If-None-Match", "W/\"1\"")
        .await;
    assert_eq!(conditional.status_code(), StatusCode::GONE);

    let batch = server
        .post("/")
        .json(&json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{"request": {"method": "GET", "url": "Patient/g1"}}]
        }))
        .await;
    assert_eq!(batch.status_code(), StatusCode::OK, "{}", batch.text());
    let body: Value = batch.json();
    assert_eq!(
        body["entry"][0]["response"]["status"].as_str(),
        Some("410 Gone"),
        "unexpected entry: {body}"
    );

    let vread = server.get("/Patient/g1/_history/2").await;
    assert_eq!(vread.status_code(), StatusCode::GONE);
}
