//! A `patient/` scope is confined to the launch patient's compartment (#1618)
//! — PostgreSQL.
//!
//! Runs the scenarios in `common/patient_scope.rs` (see
//! `patient_scope_compartment.rs` for SQLite) against a PostgreSQL container,
//! with PostgreSQL also holding the bulk export jobs, as `hfs` wires it. One
//! container hosts the suite; each scenario runs under its own tenant.
//!
//! Requires Docker (testcontainers spins up a real PostgreSQL instance).

#![cfg(feature = "postgres")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;
#[path = "common/patient_scope.rs"]
mod patient_scope;

use std::sync::Arc;

use axum_test::TestServer;
use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
use helios_persistence::core::BulkExportJobStore;
use helios_rest::ServerConfig;
use helios_rest::bulk_export_auth::BearerScopeAuth;
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

/// Same container as `patient_everything_postgres.rs`: PG 16 pinned, because
/// the testcontainers default (11) rejects the backend's `plan_cache_mode`.
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

async fn create_test_server(name: &str) -> Option<TestServer> {
    let pg = shared_pg().await;
    let backend = PostgresBackend::new(PostgresConfig {
        host: pg.host.clone(),
        port: pg.port,
        dbname: "postgres".to_string(),
        user: "postgres".to_string(),
        password: Some("postgres".to_string()),
        max_connections: 5,
        data_dir: Some(patient_scope::data_dir()),
        ..Default::default()
    })
    .await
    .expect("failed to create PostgresBackend");
    backend
        .init_schema()
        .await
        .expect("failed to initialize schema");
    let backend = Arc::new(backend);

    let server_config = ServerConfig {
        default_tenant: format!("pscope_{name}_{}", uuid::Uuid::new_v4().simple()),
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::clone(&backend), server_config).with_bulk_export(
        backend.clone() as Arc<dyn BulkExportJobStore>,
        patient_scope::export_output(),
        Arc::new(BearerScopeAuth),
    );
    Some(patient_scope::serve(
        helios_rest::routing::fhir_routes::create_routes(state),
    ))
}

patient_scope_suite!(create_test_server);
