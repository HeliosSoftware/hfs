//! #2554: a write whose body has an empty object, an empty array or a `null`
//! member is stored without them, on the REST write paths — PostgreSQL.
//!
//! Runs the scenarios in `common/empty_elements.rs` (see
//! `empty_elements_on_write.rs` for SQLite) against the PostgreSQL container
//! `patient_scope_compartment_postgres.rs` uses. The scenarios write distinct
//! ids and identifiers, so they share one database.
//!
//! Requires Docker (testcontainers spins up a real PostgreSQL instance).

#![cfg(feature = "postgres")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;
#[path = "common/empty_elements.rs"]
mod empty_elements;

use std::sync::Arc;

use axum_test::TestServer;
use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::OnceCell;

struct SharedPg {
    host: String,
    port: u16,
    /// Kept alive for the duration of the test binary; the
    /// `container_cleanup` exit hook removes it at process exit.
    _container: testcontainers::ContainerAsync<Postgres>,
}

static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

/// PG 16 pinned, because the testcontainers default (11) rejects the
/// backend's `plan_cache_mode`.
async fn shared_pg() -> &'static SharedPg {
    SHARED_PG
        .get_or_init(|| async {
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            let container = container_cleanup::with_cleanup_label(
                Postgres::default()
                    .with_tag("16-alpine")
                    .with_label("github.run_id", &run_id),
            )
            .start()
            .await
            .expect("failed to start PostgreSQL container");
            let port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("failed to get host port");
            let host = container
                .get_host()
                .await
                .expect("failed to get host")
                .to_string();
            SharedPg {
                host,
                port,
                _container: container,
            }
        })
        .await
}

async fn create_test_server(_name: &str) -> Option<TestServer> {
    let pg = shared_pg().await;
    let backend = PostgresBackend::new(PostgresConfig {
        host: pg.host.clone(),
        port: pg.port,
        dbname: "postgres".to_string(),
        user: "postgres".to_string(),
        password: Some("postgres".to_string()),
        max_connections: 5,
        data_dir: Some(empty_elements::data_dir()),
        ..Default::default()
    })
    .await
    .expect("failed to create PostgresBackend");
    backend
        .init_schema()
        .await
        .expect("failed to initialize schema");
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
