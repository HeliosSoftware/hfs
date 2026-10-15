//! "Terminology in SQL" (#1824) — PostgreSQL.
//!
//! Runs the scenarios in `common/terminology_scenarios.rs` (see
//! `sof_terminology.rs` for SQLite) against a PostgreSQL container: one
//! container for the whole binary, one tenant per server, so the scenarios
//! never see each other. Same search path as SQLite, JSONB storage; no
//! backend-specific behaviour is expected, and this suite is what proves it.
//!
//! Requires Docker (testcontainers spins up a real PostgreSQL instance).

#![cfg(feature = "postgres")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;
#[path = "common/terminology_scenarios.rs"]
mod terminology_scenarios;

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
use helios_rest::ServerConfig;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::OnceCell;

use terminology_scenarios::{Driver, Hfs};

struct SharedPg {
    host: String,
    port: u16,
    /// Kept alive for the duration of the test binary; the
    /// `container_cleanup` exit hook removes it at process exit.
    _container: testcontainers::ContainerAsync<Postgres>,
}

static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn pg_config(host: &str, port: u16) -> PostgresConfig {
    PostgresConfig {
        host: host.to_string(),
        port,
        dbname: "postgres".to_string(),
        user: "postgres".to_string(),
        password: Some("postgres".to_string()),
        max_connections: 5,
        data_dir: Some(data_dir()),
        ..Default::default()
    }
}

async fn shared_pg() -> &'static SharedPg {
    SHARED_PG
        .get_or_init(|| async {
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            // Pin the major version: testcontainers-modules defaults to
            // postgres:11, which predates `plan_cache_mode` (a GUC the backend
            // sends as a startup option), so every connection is FATAL there.
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
            let backend = PostgresBackend::new(pg_config(&host, port))
                .await
                .expect("failed to create PostgresBackend");
            backend
                .init_schema()
                .await
                .expect("failed to initialize schema");
            SharedPg {
                host,
                port,
                _container: container,
            }
        })
        .await
}

struct PostgresDriver;

#[async_trait]
impl Driver for PostgresDriver {
    async fn server(&self, config: ServerConfig, scopes: Option<&str>) -> Hfs {
        let pg = shared_pg().await;
        let backend = PostgresBackend::new(pg_config(&pg.host, pg.port))
            .await
            .expect("failed to create PostgresBackend");
        terminology_scenarios::build(Arc::new(backend), config, scopes, true, true).await
    }
}

async fn driver(_name: &str) -> Option<PostgresDriver> {
    Some(PostgresDriver)
}

terminology_suite!(driver);
