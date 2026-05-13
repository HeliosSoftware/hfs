//! End-to-end integration tests for `POST /$sqlquery-run` against a real
//! SQLite database file.
//!
//! Unlike `sof_sql_query.rs` (which uses a `MockRawSqlRunner`), these tests
//! seed data into a temporary SQLite file, wire up the real `SqliteRawRunner`,
//! and verify that the full request-to-response path works correctly — including
//! the tenant-boundary CTE, output serialisation, and DDL rejection.

mod sof_sql_query_sqlite_tests {
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::core::raw_sql::RawSqlRunner;
    use helios_persistence::raw_sql::SqliteRawRunner;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::{AppState, ServerConfig};
    use serde_json::{Value, json};
    use std::sync::Arc;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
    const CONTENT_TYPE_FHIR: HeaderValue = HeaderValue::from_static("application/fhir+json");
    const TENANT_A: HeaderValue = HeaderValue::from_static("tenant-a");
    const TENANT_B: HeaderValue = HeaderValue::from_static("tenant-b");

    // -------------------------------------------------------------------------
    // Test helpers
    // -------------------------------------------------------------------------

    fn tenant(id: &str) -> TenantContext {
        TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
    }

    fn fhir_parameters(query: &str) -> Value {
        json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "query", "valueString": query }
            ]
        })
    }

    /// Creates a temp-file SQLite backend, seeds data, then returns the
    /// path so a `SqliteRawRunner` can open it read-only.
    async fn setup() -> (tempfile::TempPath, SqliteBackend) {
        let tmp = tempfile::NamedTempFile::new()
            .expect("failed to create temp file")
            .into_temp_path();
        let path = tmp.to_str().unwrap().to_string();

        let backend = SqliteBackend::with_config(&path, Default::default())
            .expect("failed to open SQLite backend");
        backend.init_schema().expect("failed to init schema");

        (tmp, backend)
    }

    /// Builds a `TestServer` with the real `SqliteRawRunner` pointing at the
    /// same file the `backend` uses.
    fn make_server(db_path: &str, backend: Arc<SqliteBackend>) -> TestServer {
        let mut config = ServerConfig::for_testing();
        config.sof_sql_query_enabled = true;
        config.sof_sql_query_max_rows = 1000;
        config.sof_sql_query_timeout_secs = 5;

        let runner = Arc::new(SqliteRawRunner::new(db_path)) as Arc<dyn RawSqlRunner>;
        let state = AppState::new(backend, config).with_raw_sql_runner(runner);

        let app = helios_rest::routing::fhir_routes::create_routes(state);
        TestServer::new(app).expect("failed to create test server")
    }

    // -------------------------------------------------------------------------
    // 1. Basic SELECT returns seeded data
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_select_returns_seeded_patients() {
        let (tmp, backend) = setup().await;
        let db_path = tmp.to_str().unwrap().to_string();
        let t = tenant("tenant-a");

        // Seed two patients
        for (id, family) in [("p1", "Smith"), ("p2", "Jones")] {
            backend
                .create(
                    &t,
                    "Patient",
                    json!({ "resourceType": "Patient", "id": id,
                             "name": [{"family": family}] }),
                    FhirVersion::R4,
                )
                .await
                .unwrap();
        }

        let server = make_server(&db_path, Arc::new(backend));

        let resp = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, TENANT_A)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters(
                "SELECT id FROM resources WHERE resource_type = 'Patient' ORDER BY id",
            ))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK, "{}", resp.text());

        let body = resp.text();
        let rows: Vec<Value> = body
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(rows.len(), 2, "expected 2 patients, got: {body}");
        let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"p1"));
        assert!(ids.contains(&"p2"));
    }

    // -------------------------------------------------------------------------
    // 2. Tenant boundary — tenant-b cannot see tenant-a's resources
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_tenant_isolation() {
        let (tmp, backend) = setup().await;
        let db_path = tmp.to_str().unwrap().to_string();

        // Seed a patient in tenant-a
        backend
            .create(
                &tenant("tenant-a"),
                "Patient",
                json!({ "resourceType": "Patient", "id": "p-a" }),
                FhirVersion::R4,
            )
            .await
            .unwrap();

        let server = make_server(&db_path, Arc::new(backend));

        // Query as tenant-b — should see 0 rows
        let resp = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, TENANT_B)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters(
                "SELECT id FROM resources WHERE resource_type = 'Patient'",
            ))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK, "{}", resp.text());
        assert!(
            resp.text().trim().is_empty(),
            "tenant-b should see no rows from tenant-a"
        );
    }

    // -------------------------------------------------------------------------
    // 3. Row cap enforcement
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_row_cap_exceeded_returns_422() {
        let (tmp, backend) = setup().await;
        let db_path = tmp.to_str().unwrap().to_string();
        let t = tenant("tenant-a");

        // Seed 5 patients
        for i in 0..5 {
            backend
                .create(
                    &t,
                    "Patient",
                    json!({ "resourceType": "Patient", "id": format!("p{i}") }),
                    FhirVersion::R4,
                )
                .await
                .unwrap();
        }

        let mut config = ServerConfig::for_testing();
        config.sof_sql_query_enabled = true;
        config.sof_sql_query_max_rows = 2; // cap at 2 rows
        config.sof_sql_query_timeout_secs = 5;

        let runner = Arc::new(SqliteRawRunner::new(&db_path)) as Arc<dyn RawSqlRunner>;
        let state = AppState::new(Arc::new(backend), config).with_raw_sql_runner(runner);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).unwrap();

        let resp = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, TENANT_A)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters("SELECT id FROM resources"))
            .await;

        assert_eq!(
            resp.status_code(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            resp.text()
        );

        let body: Value = resp.json();
        assert_eq!(body["issue"][0]["code"], "too-costly");
    }

    // -------------------------------------------------------------------------
    // 4. CSV output format
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_csv_output_format() {
        let (tmp, backend) = setup().await;
        let db_path = tmp.to_str().unwrap().to_string();
        let t = tenant("tenant-a");

        backend
            .create(
                &t,
                "Patient",
                json!({ "resourceType": "Patient", "id": "p1" }),
                FhirVersion::R4,
            )
            .await
            .unwrap();

        let server = make_server(&db_path, Arc::new(backend));

        let resp = server
            .post("/$sqlquery-run?_format=csv")
            .add_header(X_TENANT_ID, TENANT_A)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters(
                "SELECT id FROM resources WHERE resource_type = 'Patient'",
            ))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK, "{}", resp.text());

        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.contains("text/csv"), "expected text/csv, got: {ct}");

        let body = resp.text();
        let lines: Vec<&str> = body.lines().collect();
        // header + 1 data row
        assert!(
            lines.len() >= 2,
            "expected at least header + 1 data row, got: {body:?}"
        );
        assert!(lines[0].contains("id"), "first CSV line should be header");
        assert!(lines[1].contains("p1"), "data row should contain 'p1'");
    }

    // -------------------------------------------------------------------------
    // 5. DDL rejected — cannot modify database via this endpoint
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_ddl_rejected_on_real_sqlite() {
        let (tmp, backend) = setup().await;
        let db_path = tmp.to_str().unwrap().to_string();

        let server = make_server(&db_path, Arc::new(backend));

        let resp = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, TENANT_A)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters("DROP TABLE resources"))
            .await;

        assert_eq!(
            resp.status_code(),
            StatusCode::BAD_REQUEST,
            "DDL must be rejected: {}",
            resp.text()
        );
    }

    // -------------------------------------------------------------------------
    // 6. Deleted resources are excluded
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_deleted_resources_excluded() {
        let (tmp, backend) = setup().await;
        let db_path = tmp.to_str().unwrap().to_string();
        let t = tenant("tenant-a");

        // Create two patients
        for id in ["p-alive", "p-dead"] {
            backend
                .create(
                    &t,
                    "Patient",
                    json!({ "resourceType": "Patient", "id": id }),
                    FhirVersion::R4,
                )
                .await
                .unwrap();
        }

        // Delete one
        backend.delete(&t, "Patient", "p-dead").await.unwrap();

        let server = make_server(&db_path, Arc::new(backend));

        let resp = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, TENANT_A)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters(
                "SELECT id FROM resources WHERE resource_type = 'Patient'",
            ))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK, "{}", resp.text());

        let body = resp.text();
        assert!(
            body.contains("p-alive"),
            "alive patient should appear in results"
        );
        assert!(
            !body.contains("p-dead"),
            "deleted patient must not appear in results (tenant CTE excludes is_deleted)"
        );
    }

    // -------------------------------------------------------------------------
    // Named parameter binding (T1.3): spec MUST — values bound, not interpolated
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_named_parameter_binding_returns_matching_row() {
        let (tmp, backend) = setup().await;
        let db_path = tmp.to_str().unwrap().to_string();
        let t = tenant("tenant-a");
        for (id, family) in [("a1", "Alpha"), ("b2", "Bravo")] {
            backend
                .create(
                    &t,
                    "Patient",
                    json!({"resourceType": "Patient", "id": id, "name": [{"family": family}]}),
                    FhirVersion::R4,
                )
                .await
                .unwrap();
        }
        let server = make_server(&db_path, Arc::new(backend));

        // Bind a parameter by name and assert only the matching row comes back.
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "query",
                 "valueString": "SELECT id FROM resources WHERE resource_type = 'Patient' AND id = :pid"},
                {"name": "parameters", "resource": {
                    "resourceType": "Parameters",
                    "parameter": [{"name": "pid", "valueString": "a1"}]
                }}
            ]
        });

        let resp = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, TENANT_A)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&body)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{}", resp.text());

        let text = resp.text();
        let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 1, "expected exactly one row, got: {lines:?}");
        assert!(lines[0].contains("a1"));
    }

    #[tokio::test]
    async fn test_named_parameter_binding_blocks_sql_injection() {
        let (tmp, backend) = setup().await;
        let db_path = tmp.to_str().unwrap().to_string();
        let t = tenant("tenant-a");
        backend
            .create(
                &t,
                "Patient",
                json!({"resourceType": "Patient", "id": "safe", "name": [{"family": "Safe"}]}),
                FhirVersion::R4,
            )
            .await
            .unwrap();
        let server = make_server(&db_path, Arc::new(backend));

        // The :pid value contains a SQL-injection attempt. Because the runner
        // binds the value via the driver instead of interpolating it, the
        // query simply matches zero rows — it does NOT drop the table.
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "query",
                 "valueString": "SELECT id FROM resources WHERE id = :pid"},
                {"name": "parameters", "resource": {
                    "resourceType": "Parameters",
                    "parameter": [{"name": "pid",
                                   "valueString": "safe'; DROP TABLE resources; --"}]
                }}
            ]
        });

        let resp = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, TENANT_A)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&body)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{}", resp.text());

        // Empty result set — the literal string didn't match any id.
        let text = resp.text();
        let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
        assert!(lines.is_empty(), "expected no rows, got: {lines:?}");

        // The table still exists: a follow-up SELECT must succeed.
        let probe = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, TENANT_A)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters(
                "SELECT id FROM resources WHERE resource_type = 'Patient'",
            ))
            .await;
        assert_eq!(probe.status_code(), StatusCode::OK);
        assert!(probe.text().contains("safe"));
    }

    #[tokio::test]
    async fn test_named_parameter_missing_value_returns_400() {
        let (tmp, backend) = setup().await;
        let db_path = tmp.to_str().unwrap().to_string();
        let server = make_server(&db_path, Arc::new(backend));

        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "query",
                 "valueString": "SELECT 1 WHERE 1 = :missing"}
            ]
        });

        let resp = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, TENANT_A)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&body)
            .await;
        assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
    }

    // -------------------------------------------------------------------------
    // _format=fhir (T5.1): SQL result serialised as a FHIR Parameters resource
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_format_fhir_returns_parameters_resource() {
        let (tmp, backend) = setup().await;
        let db_path = tmp.to_str().unwrap().to_string();
        let t = tenant("tenant-a");
        backend
            .create(
                &t,
                "Patient",
                json!({"resourceType": "Patient", "id": "fhir-1", "name": [{"family": "Fhir"}]}),
                FhirVersion::R4,
            )
            .await
            .unwrap();
        let server = make_server(&db_path, Arc::new(backend));

        let resp = server
            .post("/$sqlquery-run?_format=fhir")
            .add_header(X_TENANT_ID, TENANT_A)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters(
                "SELECT id FROM resources WHERE resource_type = 'Patient'",
            ))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{}", resp.text());

        let body: Value = resp.json::<Value>();
        assert_eq!(body["resourceType"].as_str(), Some("Parameters"));
        let parts = body["parameter"].as_array().unwrap();
        let any_row = parts
            .iter()
            .any(|p| p["name"].as_str() == Some("row") && p["part"].is_array());
        assert!(
            any_row,
            "_format=fhir must produce row parameters with parts: {body}"
        );
    }
}
