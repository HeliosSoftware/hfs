//! SQL query ValueSet/ConceptMap dependencies (#2185), end to end on the
//! PostgreSQL backend. The scenarios are shared by every backend; see
//! `common/sof_terminology_e2e.rs`.
//!
//! Requires Docker (testcontainers spins up a real PostgreSQL instance), like
//! `sof_conformance_postgres.rs`. Every scenario runs under its own tenant in
//! the one database.

#![cfg(feature = "postgres")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

#[path = "common/sof_terminology_e2e.rs"]
mod sof_terminology_e2e;

use std::sync::Arc;

use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
use sof_terminology_e2e::Harness;
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

fn config(host: &str, port: u16) -> PostgresConfig {
    PostgresConfig {
        host: host.to_string(),
        port,
        dbname: "postgres".to_string(),
        user: "postgres".to_string(),
        password: Some("postgres".to_string()),
        max_connections: 5,
        data_dir: Some(sof_terminology_e2e::data_dir()),
        ..Default::default()
    }
}

async fn shared_pg() -> &'static SharedPg {
    SHARED_PG
        .get_or_init(|| async {
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            // Pinned: testcontainers-modules defaults to postgres:11, which
            // predates `plan_cache_mode` and rejects every connection.
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
            // Migrate once, so concurrent scenarios do not race the schema.
            PostgresBackend::new(config(&host, port))
                .await
                .expect("failed to create PostgresBackend")
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

async fn postgres(terminology: Option<String>) -> Option<Harness> {
    let pg = shared_pg().await;
    let backend = PostgresBackend::new(config(&pg.host, pg.port))
        .await
        .expect("failed to create PostgresBackend");
    Some(sof_terminology_e2e::harness(Arc::new(backend), terminology))
}

sof_terminology_e2e_suite!(postgres);
