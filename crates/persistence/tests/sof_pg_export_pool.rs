//! The PostgreSQL export pool `$sql-export` jobs read through.
//!
//! Verifies, against a real PostgreSQL 16 (testcontainers, Docker required),
//! that `export_sof_runner` reads through its own `hfs-export` connections
//! and returns exactly the rows of `sof_runner`, in the same order. The
//! settings those connections carry are checked by the backend's own unit
//! tests, which can reach the main pool to compare.
//!
//! Run with:
//!   cargo test -p helios-persistence --features postgres --test sof_pg_export_pool

#![cfg(feature = "postgres")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

mod sof_pg_export_pool_tests {
    use std::path::PathBuf;

    use futures::StreamExt;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::core::sof_runner::{SofRunner, ViewFilters};
    use helios_persistence::core::{ExportRunnerOptions, ResourceStorage};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use serde_json::{Value, json};
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use tokio_postgres::NoTls;

    /// A container of its own: the assertions read `pg_stat_activity`, which
    /// another test's connections would pollute.
    async fn start_pg() -> (testcontainers::ContainerAsync<Postgres>, PostgresConfig) {
        let container = super::container_cleanup::with_cleanup_label(
            Postgres::default().with_tag("16-alpine").with_label(
                "github.run_id",
                std::env::var("GITHUB_RUN_ID").unwrap_or_default(),
            ),
        )
        .start()
        .await
        .expect("start PostgreSQL 16 testcontainer");
        let config = PostgresConfig {
            host: container.get_host().await.expect("host").to_string(),
            port: container.get_host_port_ipv4(5432).await.expect("port"),
            dbname: "postgres".into(),
            user: "postgres".into(),
            password: Some("postgres".into()),
            max_connections: 3,
            data_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
            ..Default::default()
        };
        (container, config)
    }

    /// A connection outside both pools, for reading `pg_stat_activity`.
    async fn observer(config: &PostgresConfig) -> tokio_postgres::Client {
        let (client, connection) = tokio_postgres::Config::new()
            .host(&config.host)
            .port(config.port)
            .user("postgres")
            .password("postgres")
            .dbname("postgres")
            .application_name("export-pool-test")
            .connect(NoTls)
            .await
            .expect("observer connection");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
    }

    async fn connections_named(observer: &tokio_postgres::Client, name: &str) -> i64 {
        observer
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE application_name = $1",
                &[&name],
            )
            .await
            .expect("read pg_stat_activity")
            .get(0)
    }

    #[tokio::test]
    async fn export_runner_reads_through_the_export_pool() {
        let (_container, config) = start_pg().await;
        let backend = PostgresBackend::new(config.clone())
            .await
            .expect("PostgresBackend");
        backend.init_schema().await.expect("schema");

        let tenant = TenantContext::new(
            TenantId::new("export_pool"),
            TenantPermissions::full_access(),
        );
        for (id, given) in [("a", vec!["Ann", "Amy"]), ("b", vec!["Bob"]), ("c", vec![])] {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": id,
                        "name": [{"family": format!("F-{id}"), "given": given}]
                    }),
                    FhirVersion::R4,
                )
                .await
                .expect("seed patient");
        }
        // A forEach view: the shape the export pool's settings are for.
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [
                {"column": [{"name": "id", "path": "getResourceKey()"}]},
                {"forEachOrNull": "name.given", "column": [{"name": "given", "path": "$this"}]}
            ]
        });

        let observer = observer(&config).await;
        let main_runner = backend.sof_runner().expect("main runner");
        let main_rows = rows(main_runner.as_ref(), &tenant, &view).await;
        assert_eq!(connections_named(&observer, "hfs-export").await, 0);

        let export_runner = backend
            .export_sof_runner(&ExportRunnerOptions::default())
            .expect("export runner");
        assert_eq!(export_runner.runner_name(), "postgres-indb");
        let export_rows = rows(export_runner.as_ref(), &tenant, &view).await;

        // Same rows, same multiplicity, same order: only the connection moved.
        assert_eq!(export_rows, main_rows);
        assert_eq!(export_rows.len(), 4, "{export_rows:?}");
        // The export connection stays pooled after the run.
        assert_eq!(connections_named(&observer, "hfs-export").await, 1);
    }

    async fn rows(runner: &dyn SofRunner, tenant: &TenantContext, view: &Value) -> Vec<Value> {
        let mut stream = runner
            .run_view(tenant, view.clone(), ViewFilters::default())
            .await
            .expect("run_view");
        let mut rows = Vec::new();
        while let Some(row) = stream.next().await {
            rows.push(row.expect("row"));
        }
        rows
    }
}
