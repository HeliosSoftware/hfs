//! #1769 on the PostgreSQL in-DB runner: scalar `string` / `code` columns keep
//! their stored text in `json`, `ndjson` and `csv`, and a SQL NULL is written
//! as an explicit `null` so a NULL in the first row no longer drops the column.
//!
//! Requires Docker (testcontainers spins up a real PostgreSQL instance). The
//! container setup is copied from `sof_conformance_postgres.rs`.

#![cfg(feature = "postgres")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

mod sof_run_scalar_strings_postgres_tests {
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

    const CODES: [&str; 6] = ["44054006", "0123", "4548-4", "true", "null", "1e3"];

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
        let tenant_id = format!("sof_pg_str_{}", uuid::Uuid::new_v4().simple());
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

    fn conditions() -> Vec<Value> {
        CODES
            .iter()
            .enumerate()
            .map(|(i, code)| {
                json!({
                    "resourceType": "Condition", "id": format!("c{i}"),
                    "subject": {"reference": "Patient/p1"},
                    "code": {"coding": [{"system": "http://example.org/cs", "code": code}]}
                })
            })
            .collect()
    }

    fn condition_codes(code_column: Value) -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "name": "condition_codes",
            "status": "active",
            "resource": "Condition",
            "select": [{"column": [
                {"name": "id", "path": "getResourceKey()", "type": "id"},
                code_column
            ]}]
        })
    }

    fn column_variants() -> Vec<Value> {
        let path = "code.coding.first().code";
        vec![
            json!({"name": "code", "path": path, "type": "code"}),
            json!({"name": "code", "path": path, "type": "string"}),
            json!({"name": "code", "path": path}),
        ]
    }

    async fn run(server: &TestServer, tenant: &str, format: &str, view: &Value) -> String {
        let response = server
            .post(&format!("/$sql-run?_format={format}"))
            .add_header(X_TENANT_ID, HeaderValue::from_str(tenant).unwrap())
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(view)
            .await;
        assert_eq!(response.status_code(), 200, "{}", response.text());
        response.text()
    }

    fn assert_codes_are_strings(rows: &[Value]) {
        assert_eq!(rows.len(), CODES.len(), "{rows:?}");
        for (i, code) in CODES.iter().enumerate() {
            let id = format!("c{i}");
            let row = rows
                .iter()
                .find(|r| r["id"] == id.as_str())
                .unwrap_or_else(|| panic!("row {id} missing from {rows:?}"));
            assert_eq!(row["code"], Value::String((*code).into()), "{row}");
        }
    }

    #[tokio::test]
    async fn json_and_ndjson_keep_every_code_as_a_string() {
        let (server, tenant) = server_with(&conditions()).await;
        for column in column_variants() {
            let view = condition_codes(column);
            let rows: Vec<Value> =
                serde_json::from_str(&run(&server, &tenant, "json", &view).await).expect("json");
            assert_codes_are_strings(&rows);

            let nd = run(&server, &tenant, "ndjson", &view).await;
            let rows: Vec<Value> = nd
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| serde_json::from_str(l).expect("ndjson row"))
                .collect();
            assert_codes_are_strings(&rows);
        }
    }

    #[tokio::test]
    async fn csv_writes_the_text_null_instead_of_an_empty_cell() {
        let (server, tenant) = server_with(&conditions()).await;
        for column in column_variants() {
            let body = run(&server, &tenant, "csv", &condition_codes(column)).await;
            let mut lines = body.lines();
            assert_eq!(lines.next(), Some("id,code"), "{body}");
            let cells: Vec<&str> = lines.collect();
            for (i, code) in CODES.iter().enumerate() {
                let expected = format!("c{i},{code}");
                assert!(
                    cells.contains(&expected.as_str()),
                    "expected line {expected:?} in {body}"
                );
            }
        }
    }

    /// A NULL in the first row must not drop the column from the CSV header
    /// or from the JSON rows; ndjson rows carry an explicit `null`.
    #[tokio::test]
    async fn a_null_in_the_first_row_keeps_every_column() {
        let bare = json!({"resourceType": "Patient", "id": "bare", "name": [{"family": "Bare"}]});
        let full = json!({
            "resourceType": "Patient", "id": "full", "gender": "female",
            "name": [{"family": "Parker433"}]
        });
        let (server, tenant) = server_with(&[bare, full]).await;
        let view = json!({
            "resourceType": "ViewDefinition",
            "name": "patient_gender",
            "status": "active",
            "resource": "Patient",
            "select": [{"column": [
                {"name": "id", "path": "getResourceKey()", "type": "id"},
                {"name": "gender", "path": "gender", "type": "code"},
                {"name": "family", "path": "name.first().family", "type": "string"}
            ]}]
        });

        let csv = run(&server, &tenant, "csv", &view).await;
        assert_eq!(csv.lines().next(), Some("id,gender,family"), "{csv}");

        let rows: Vec<Value> =
            serde_json::from_str(&run(&server, &tenant, "json", &view).await).expect("json");
        for row in &rows {
            for column in ["id", "gender", "family"] {
                assert!(row.get(column).is_some(), "{column} missing from {row}");
            }
        }

        let nd = run(&server, &tenant, "ndjson", &view).await;
        let bare_row: Value = nd
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).expect("ndjson row"))
            .find(|r| r["family"] == "Bare")
            .expect("bare row");
        assert_eq!(bare_row["gender"], Value::Null, "{bare_row}");
    }
}
