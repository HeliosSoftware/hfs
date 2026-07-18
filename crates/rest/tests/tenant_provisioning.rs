//! Integration tests for provisioned-only tenant enforcement
//! (`HFS_TENANT_REQUIRE_PROVISIONED`): a request resolving to a tenant that was
//! never provisioned through the admin API is rejected, while provisioned
//! tenants work normally.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_rest::ServerConfig;
use helios_rest::config::MultitenancyConfig;

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

async fn create_server(require_provisioned: bool) -> (TestServer, Arc<SqliteBackend>) {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("create SQLite backend");
    backend.init_schema().expect("init schema");
    let backend = Arc::new(backend);

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            require_provisioned_tenant: require_provisioned,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "default".to_string(),
        ..ServerConfig::for_testing()
    };

    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let server = TestServer::new(app).expect("create test server");
    (server, backend)
}

#[tokio::test]
async fn unprovisioned_tenant_is_rejected_when_enforced() {
    let (server, backend) = create_server(true).await;
    backend
        .register_tenant("acme", None)
        .await
        .expect("provision acme");

    // A provisioned tenant is served normally.
    let ok = server
        .get("/Patient")
        .add_header(X_TENANT_ID, HeaderValue::from_static("acme"))
        .await;
    assert_eq!(ok.status_code(), StatusCode::OK);

    // An unprovisioned tenant is rejected.
    let rejected = server
        .get("/Patient")
        .add_header(X_TENANT_ID, HeaderValue::from_static("ghost"))
        .await;
    assert_eq!(rejected.status_code(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unprovisioned_tenant_is_allowed_when_not_enforced() {
    // Default (backward-compatible) behavior: any tenant id works.
    let (server, _backend) = create_server(false).await;
    let resp = server
        .get("/Patient")
        .add_header(X_TENANT_ID, HeaderValue::from_static("ghost"))
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
}
