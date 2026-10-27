//! #1636: a transaction Bundle that cannot BEGIN because SQLite is busy answers
//! `503` with `Retry-After`, not `500`.
//!
//! Concurrent bundle imports queue on SQLite's single write lock; a bundle that
//! outwaits `busy_timeout` used to surface as a flattened, twice-wrapped
//! `RolledBack` and a 500 — a server-defect status for contention that a retry
//! cures. The lock is held here by a second plain connection on the same WAL
//! file, and the backend's `busy_timeout` is short so the failure is quick.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use serde_json::{Value, json};

const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
const RETRY_AFTER: HeaderName = HeaderName::from_static("retry-after");

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"))
}

/// A server over a file-backed SQLite database whose `busy_timeout` is 200 ms.
fn server(db_path: &std::path::Path) -> TestServer {
    let backend = SqliteBackend::with_config(
        db_path.to_str().unwrap(),
        SqliteBackendConfig {
            data_dir: Some(data_dir()),
            busy_timeout_ms: 200,
            ..Default::default()
        },
    )
    .expect("file-backed SQLite backend");
    backend.init_schema().expect("init schema");

    let state = helios_rest::AppState::new(Arc::new(backend), ServerConfig::for_testing());
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).expect("server")
}

fn transaction_bundle() -> Value {
    json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [{
            "fullUrl": "urn:uuid:1",
            "request": {"method": "POST", "url": "Patient"},
            "resource": {"resourceType": "Patient", "name": [{"family": "Busy"}]}
        }]
    })
}

async fn post_bundle(server: &TestServer) -> axum_test::TestResponse {
    server
        .post("/")
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&transaction_bundle())
        .await
}

#[tokio::test]
async fn a_transaction_that_cannot_begin_for_a_held_write_lock_is_a_503_with_retry_after() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("busy.db");
    let server = server(&db_path);

    // Another writer takes SQLite's write lock and keeps it.
    let holder = rusqlite::Connection::open(&db_path).expect("second connection");
    holder
        .busy_timeout(std::time::Duration::from_millis(0))
        .unwrap();
    holder
        .execute_batch("BEGIN IMMEDIATE")
        .expect("the second connection takes the write lock");

    let response = post_bundle(&server).await;
    let text = response.text();
    assert_eq!(
        response.status_code(),
        StatusCode::SERVICE_UNAVAILABLE,
        "{text}"
    );
    assert_eq!(
        response
            .headers()
            .get(&RETRY_AFTER)
            .and_then(|v| v.to_str().ok()),
        Some("5"),
        "a retryable 503 carries Retry-After"
    );

    let body: Value = response.json();
    assert_eq!(body["resourceType"], "OperationOutcome");
    assert_eq!(body["issue"][0]["code"], "transient");
    for leak in ["locked", "SQLite", "sqlite", "BEGIN"] {
        assert!(
            !text.contains(leak),
            "the driver's text must stay out of the response ({leak:?}): {text}"
        );
    }

    // The failed attempt left nothing behind: the same bundle succeeds once the
    // lock is released.
    drop(holder);
    let retry = post_bundle(&server).await;
    assert_eq!(retry.status_code(), StatusCode::OK, "{}", retry.text());
}
