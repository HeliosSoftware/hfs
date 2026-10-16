//! #1812 on the PostgreSQL in-DB runner: a typed view streamed as Parquet gets
//! the declared Arrow types (the runner returns text for several columns) and
//! the values survive the round trip.
//!
//! Requires Docker (testcontainers spins up a real PostgreSQL instance). The
//! container setup is copied from `sof_run_scalar_strings_postgres.rs`.

#![cfg(feature = "postgres")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

mod sof_run_parquet_postgres_tests {
    use axum::http::{HeaderName, HeaderValue};
    use axum_test::TestServer;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use serde_json::{Value, json};
    use std::path::PathBuf;
    use std::sync::Arc;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use tokio::sync::OnceCell;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

    struct SharedPg {
        host: String,
        port: u16,
        /// Kept alive for the duration of the test binary; the
        /// `container_cleanup` exit hook removes it at process exit.
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"))
    }

    fn config(host: &str, port: u16) -> PostgresConfig {
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
                // postgres:11, which rejects the backend's startup options.
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
                let backend = PostgresBackend::new(config(&host, port))
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

    async fn server_with(resources: &[Value]) -> (TestServer, String) {
        let pg = shared_pg().await;
        let backend = Arc::new(
            PostgresBackend::new(config(&pg.host, pg.port))
                .await
                .expect("failed to create PostgresBackend"),
        );
        let tenant_id = format!("sof_pg_pq_{}", uuid::Uuid::new_v4().simple());
        let tenant =
            TenantContext::new(TenantId::new(&tenant_id), TenantPermissions::full_access());
        for resource in resources {
            let rt = resource["resourceType"].as_str().expect("resourceType");
            backend
                .create(&tenant, rt, resource.clone(), FhirVersion::R4)
                .await
                .expect("seed");
        }
        let runner = backend.sof_runner().expect("in-DB runner");
        let state = helios_rest::AppState::new(Arc::clone(&backend), ServerConfig::for_testing())
            .with_sof_runner(runner);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        (TestServer::new(app).expect("server"), tenant_id)
    }

    fn observations() -> Vec<Value> {
        (0..5)
            .map(|i| {
                json!({
                    "resourceType": "Observation", "id": format!("o{i}"),
                    "status": "final",
                    "code": {"text": "t"},
                    "valueInteger": i * 10,
                    "valueQuantity": {"value": i as f64 + 0.5},
                    "valueBoolean": i % 2 == 0
                })
            })
            .collect()
    }

    fn typed_view() -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "name": "typed",
            "status": "active",
            "resource": "Observation",
            "select": [{"column": [
                {"name": "id", "path": "getResourceKey()", "type": "id"},
                {"name": "n", "path": "valueInteger", "type": "integer"},
                {"name": "d", "path": "valueQuantity.value", "type": "decimal"},
                {"name": "b", "path": "valueBoolean", "type": "boolean"}
            ]}]
        })
    }

    #[tokio::test]
    async fn typed_view_streams_declared_parquet_types() {
        use arrow::array::{Array, BooleanArray, Int32Array, StringArray};
        use arrow::datatypes::DataType;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

        let (server, tenant) = server_with(&observations()).await;
        let response = server
            .post("/$sql-run?_format=parquet")
            .add_header(X_TENANT_ID, HeaderValue::from_str(&tenant).unwrap())
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&typed_view())
            .await;
        assert_eq!(response.status_code(), 200, "{}", response.text());
        let builder =
            ParquetRecordBatchReaderBuilder::try_new(response.into_bytes()).expect("valid parquet");
        let schema = builder.schema().clone();
        for (name, ty) in [
            ("n", DataType::Int32),
            ("d", DataType::Utf8), // decimal is character data (#569)
            ("b", DataType::Boolean),
        ] {
            assert_eq!(
                schema.field_with_name(name).unwrap().data_type(),
                &ty,
                "{name}"
            );
        }
        let batches: Vec<_> = builder.build().unwrap().map(|b| b.unwrap()).collect();
        let (mut ns, mut ds, mut bs) = (vec![], vec![], vec![]);
        for batch in &batches {
            let n = batch
                .column_by_name("n")
                .unwrap()
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap();
            let d = batch
                .column_by_name("d")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let b = batch
                .column_by_name("b")
                .unwrap()
                .as_any()
                .downcast_ref::<BooleanArray>()
                .unwrap();
            for i in 0..batch.num_rows() {
                assert!(!n.is_null(i) && !d.is_null(i) && !b.is_null(i), "null cell");
                ns.push(n.value(i));
                ds.push(d.value(i).to_string());
                bs.push(b.value(i));
            }
        }
        ns.sort();
        ds.sort();
        assert_eq!(ns, vec![0, 10, 20, 30, 40]);
        assert_eq!(ds, vec!["0.5", "1.5", "2.5", "3.5", "4.5"]);
        assert_eq!(bs.iter().filter(|b| **b).count(), 3);
    }
}
