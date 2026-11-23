//! A `DELETE [type]/[id]/_history` or `.../_history/[vid]` Bundle entry is
//! refused with 400 in both arms: read as the last two segments it used to
//! address a made-up instance, answer 204 and erase nothing. The history
//! deletes have no Bundle form; the instance endpoints erase history.

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

    for status in ["preliminary", "final"] {
        let response = server
            .put("/Observation/o1")
            .json(&json!({"resourceType": "Observation", "id": "o1", "status": status, "code": {"text": "x"}}))
            .await;
        assert!(response.status_code().is_success(), "{}", response.text());
    }
    server
}

async fn history_total(server: &TestServer) -> u64 {
    let response = server.get("/Observation/o1/_history").await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    response.json::<Value>()["total"].as_u64().unwrap_or(0)
}

fn bundle(kind: &str, url: &str) -> Value {
    json!({"resourceType": "Bundle", "type": kind, "entry": [{"request": {"method": "DELETE", "url": url}}]})
}

#[tokio::test]
async fn history_delete_entries_are_refused_and_erase_nothing() {
    let server = server().await;

    for url in ["Observation/o1/_history", "Observation/o1/_history/1"] {
        let batch = server.post("/").json(&bundle("batch", url)).await;
        assert_eq!(batch.status_code(), StatusCode::OK, "{}", batch.text());
        let body: Value = batch.json();
        let status = body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap_or_default();
        assert!(status.starts_with("400"), "batch {url}: {body}");
        assert!(body.to_string().contains("_history"), "batch {url}: {body}");

        let transaction = server.post("/").json(&bundle("transaction", url)).await;
        assert_eq!(
            transaction.status_code(),
            StatusCode::BAD_REQUEST,
            "transaction {url}: {}",
            transaction.text()
        );
        assert!(
            transaction.text().contains("_history"),
            "transaction {url}: {}",
            transaction.text()
        );
    }
    assert_eq!(history_total(&server).await, 2, "nothing was erased");
}
