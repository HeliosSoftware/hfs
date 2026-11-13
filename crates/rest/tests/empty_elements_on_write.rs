//! #2554: a write whose body has an empty object, an empty array or a `null`
//! member is stored without them, on the REST write paths — SQLite.
//!
//! The scenarios live in `common/empty_elements.rs`; the MongoDB and
//! PostgreSQL twins run the same ones.

#![cfg(feature = "sqlite")]

#[path = "common/empty_elements.rs"]
mod empty_elements;

use std::sync::Arc;

use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};

async fn create_test_server(_name: &str) -> Option<TestServer> {
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(empty_elements::data_dir()),
            ..Default::default()
        },
    )
    .expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to init schema");
    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: empty_elements::TENANT.to_string(),
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::new(backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    Some(TestServer::new(app).expect("Failed to create test server"))
}

empty_elements_suite!(create_test_server);
