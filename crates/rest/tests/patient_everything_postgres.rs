//! `Patient/$everything` router test — PostgreSQL-backed.
//!
//! Mirrors `patient_everything.rs` (SQLite in-memory) but wires the HTTP
//! server's storage backend to a PostgreSQL container via `testcontainers`,
//! the same way `sof_conformance_postgres.rs` does. Proves the paged walk
//! (`_count`-driven `next` links) returns the same match set as the unpaged
//! walk against a real relational backend, not just SQLite.
//!
//! Requires Docker (testcontainers spins up a real PostgreSQL instance).

#![cfg(feature = "postgres")]

mod common;

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

mod patient_everything_postgres_tests {
    use axum_test::TestServer;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_rest::ServerConfig;
    use std::path::PathBuf;
    use std::sync::Arc;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use tokio::sync::OnceCell;

    use crate::common::everything::*;

    // =========================================================================
    // Shared container setup — copied verbatim from `sof_conformance_postgres.rs`
    // (lines 43-137 there). A single PG container hosts the whole suite; each
    // test runs under its own tenant so the container starts up once.
    // =========================================================================

    struct SharedPg {
        host: String,
        port: u16,
        /// Kept alive for the duration of the test binary; the
        /// `container_cleanup` exit hook removes it at process exit.
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

    async fn shared_pg() -> &'static SharedPg {
        SHARED_PG
            .get_or_init(|| async {
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                // Pin the major version. testcontainers-modules defaults to
                // postgres:11, which is EOL and predates `plan_cache_mode` — a GUC
                // the backend sends as a startup option, so PG 11 rejects every
                // connection FATAL. The rest of the repo runs 16.
                // `SHARED_PG` is a static and never dropped; the cleanup label
                // lets the exit hook remove the container.
                let container = super::container_cleanup::with_cleanup_label(
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

    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"))
    }

    async fn create_test_server(name: &str) -> TestServer {
        let pg = shared_pg().await;

        // `data_dir` points at the workspace `data/` directory so the
        // backend can load search-parameter definitions for the active
        // FHIR version — needed for `$everything`'s `date`, `onset-date`,
        // `_lastUpdated` and compartment reference params.
        let config = PostgresConfig {
            host: pg.host.clone(),
            port: pg.port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: 5,
            data_dir: Some(data_dir()),
            ..Default::default()
        };

        let backend = PostgresBackend::new(config)
            .await
            .expect("failed to create PostgresBackend");
        backend
            .init_schema()
            .await
            .expect("failed to initialize schema");

        let tenant_id = format!("everything_pg_{name}_{}", uuid::Uuid::new_v4().simple());
        let server_config = ServerConfig {
            base_url: "http://localhost:8080".to_string(),
            default_tenant: tenant_id,
            everything_max_unpaged: 10_000,
            ..ServerConfig::for_testing()
        };

        let state = helios_rest::AppState::new(Arc::new(backend), server_config);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        TestServer::new(app).expect("failed to create test server")
    }

    #[tokio::test]
    async fn postgres_everything_paged_walk_matches_unpaged() {
        let server = create_test_server("everything-pg").await;
        seed(&server).await;
        let (unpaged, _) = walk(&server, "/Patient/p1/$everything").await;
        let (paged, pages) = walk(&server, "/Patient/p1/$everything?_count=2").await;
        assert!(pages.len() >= 4);
        let mut a = unpaged.clone();
        a.sort();
        let mut b = paged.clone();
        b.sort();
        assert_eq!(a, b);
        assert_eq!(paged.len(), unpaged.len());
        assert_eq!(unpaged[0], "Patient/p1");
        assert_eq!(unpaged.len(), 7, "{unpaged:?}");
    }
}
