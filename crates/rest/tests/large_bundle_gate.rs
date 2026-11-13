use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::json;

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

fn create_gated_server(large_bundle_concurrency: usize) -> TestServer {
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
        large_bundle_concurrency,
        ..ServerConfig::default()
    };

    let app = helios_rest::create_app_with_config(backend, config);
    TestServer::new(app).expect("Failed to create test server")
}

fn padded_transaction(id: &str) -> Vec<u8> {
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [{
            "resource": {"resourceType": "Patient", "id": id},
            "request": {"method": "PUT", "url": format!("Patient/{id}")}
        }]
    });
    let mut body = serde_json::to_vec(&bundle).unwrap();
    body.extend(std::iter::repeat_n(b' ', 9 * 1024 * 1024));
    body
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_large_bundles_all_pass_through_the_gate() {
    let server = Arc::new(create_gated_server(1));
    let mut handles = Vec::new();
    for n in 0..3 {
        let server = Arc::clone(&server);
        handles.push(tokio::spawn(async move {
            server
                .post("/")
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .add_header(
                    HeaderName::from_static("content-type"),
                    HeaderValue::from_static("application/fhir+json"),
                )
                .bytes(padded_transaction(&format!("gated-{n}")).into())
                .await
                .status_code()
        }));
    }
    for handle in handles {
        assert_eq!(handle.await.unwrap(), StatusCode::OK);
    }

    for n in 0..3 {
        let read = server
            .get(&format!("/Patient/gated-{n}"))
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        assert_eq!(read.status_code(), StatusCode::OK);
    }
}

#[tokio::test]
async fn disabled_gate_still_processes_large_bundles() {
    let server = create_gated_server(0);
    let response = server
        .post("/")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .add_header(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/fhir+json"),
        )
        .bytes(padded_transaction("ungated").into())
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
}
