//! A DELETE Bundle entry whose URL is deeper than `[type]/[id]` is read as
//! its last two segments, so `DELETE Patient/p1/foo` addressed id `foo` of
//! type `p1`. The resource type of a DELETE entry is now admitted like a
//! PATCH entry's: an unknown type is a 404 entry in a batch and refuses a
//! transaction whole, and nothing answers 204 for a type that does not exist.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::{AppState, ServerConfig};
use serde_json::{Value, json};

async fn server() -> TestServer {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"));
    let backend = Arc::new(
        SqliteBackend::with_config(
            ":memory:",
            SqliteBackendConfig {
                data_dir,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    backend.init_schema().unwrap();
    let state = AppState::new(backend, ServerConfig::for_testing());
    let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).unwrap();
    let response = server
        .put("/Patient/p1")
        .json(&json!({"resourceType": "Patient", "id": "p1"}))
        .await;
    assert!(response.status_code().is_success(), "{}", response.text());
    server
}

fn bundle(kind: &str, url: &str) -> Value {
    json!({"resourceType": "Bundle", "type": kind, "entry": [{"request": {"method": "DELETE", "url": url}}]})
}

#[tokio::test]
async fn a_delete_entry_of_an_unknown_type_is_refused_not_answered_204() {
    let server = server().await;

    for url in ["Patient/p1/foo", "Nonsense/x"] {
        let batch = server.post("/").json(&bundle("batch", url)).await;
        assert_eq!(batch.status_code(), StatusCode::OK, "{}", batch.text());
        let body: Value = batch.json();
        let status = body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap_or_default();
        assert!(status.starts_with("404"), "batch {url}: {body}");

        let transaction = server.post("/").json(&bundle("transaction", url)).await;
        assert_eq!(
            transaction.status_code(),
            StatusCode::NOT_FOUND,
            "transaction {url}: {}",
            transaction.text()
        );
    }

    let still_there = server.get("/Patient/p1").await;
    assert_eq!(still_there.status_code(), StatusCode::OK);

    let deleted = server.post("/").json(&bundle("batch", "Patient/p1")).await;
    let body: Value = deleted.json();
    assert!(
        body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap_or_default()
            .starts_with("204"),
        "{body}"
    );
}
