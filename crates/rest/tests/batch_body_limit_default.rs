//! The default `HFS_MAX_BODY_SIZE` accepts routine Synthea bundles (#1662).
//!
//! The default was raised from 10 MiB to 128 MiB so a whole per-patient
//! Synthea Bundle (largest measured: 69.4 MiB) can be posted as one batch or
//! transaction. This test posts a batch one byte under the default through the
//! full middleware stack and expects it to be processed, not rejected.

use std::path::PathBuf;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

/// Creates a test server running the full middleware stack with the default
/// body limit (not the `for_testing()` one).
fn create_default_limit_server() -> TestServer {
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

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::default()
    };

    let app = helios_rest::create_app_with_config(backend, config);
    TestServer::new(app).expect("Failed to create test server")
}

#[tokio::test]
async fn batch_just_under_the_default_body_limit_is_accepted() {
    let limit = ServerConfig::default().max_body_size;
    assert_eq!(limit, 128 * 1024 * 1024);

    // A one-entry batch padded with trailing whitespace (valid JSON) to one
    // byte under the limit: the size is what is under test, not the entries.
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "batch",
        "entry": [{"request": {"method": "GET", "url": "Patient?_count=1"}}]
    });
    let mut body = serde_json::to_vec(&bundle).unwrap();
    body.resize(limit - 1, b' ');

    let response = create_default_limit_server()
        .post("/")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .content_type("application/fhir+json")
        .bytes(body.into())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let json: Value = response.json();
    assert_eq!(json["type"], "batch-response");
    assert_eq!(json["entry"][0]["response"]["status"], "200 OK");
}
