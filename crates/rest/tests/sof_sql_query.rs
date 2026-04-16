//! Integration tests for `POST /$sql-query-run`.
//!
//! These tests verify the handler-level behaviour: feature gate (501 by
//! default), DDL rejection (400), NDJSON / CSV output, and tenant isolation.
//! A `MockRawSqlRunner` is used so no real database file is required.

#[cfg(feature = "sof")]
mod sof_sql_query_tests {
    use async_trait::async_trait;
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::raw_sql::{RawSqlError, RawSqlRunner, SqlRow};
    use helios_rest::{AppState, ServerConfig};
    use serde_json::{Value, json};
    use std::sync::Arc;

    // -------------------------------------------------------------------------
    // Test helpers
    // -------------------------------------------------------------------------

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const CONTENT_TYPE_FHIR: HeaderValue = HeaderValue::from_static("application/fhir+json");
    const CONTENT_TYPE_JSON: HeaderValue = HeaderValue::from_static("application/json");
    const TENANT_TEST: HeaderValue = HeaderValue::from_static("test-tenant");
    const TENANT_CLINIC_A: HeaderValue = HeaderValue::from_static("clinic-a");
    const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

    /// A mock runner that returns a pre-configured response (or error).
    struct MockRawSqlRunner {
        rows: Result<Vec<SqlRow>, RawSqlError>,
    }

    impl MockRawSqlRunner {
        fn ok(rows: Vec<SqlRow>) -> Arc<Self> {
            Arc::new(Self { rows: Ok(rows) })
        }

        fn row_limit_exceeded(max: usize) -> Arc<Self> {
            Arc::new(Self {
                rows: Err(RawSqlError::RowLimitExceeded { max_rows: max }),
            })
        }

        fn timeout(secs: u64) -> Arc<Self> {
            Arc::new(Self {
                rows: Err(RawSqlError::Timeout { secs }),
            })
        }
    }

    #[async_trait]
    impl RawSqlRunner for MockRawSqlRunner {
        async fn run_query(
            &self,
            _tenant_id: &str,
            _sql: &str,
            _max_rows: usize,
            _timeout_secs: u64,
        ) -> Result<Vec<SqlRow>, RawSqlError> {
            match &self.rows {
                Ok(r) => Ok(r.clone()),
                Err(RawSqlError::Timeout { secs }) => Err(RawSqlError::Timeout { secs: *secs }),
                Err(RawSqlError::RowLimitExceeded { max_rows }) => {
                    Err(RawSqlError::RowLimitExceeded {
                        max_rows: *max_rows,
                    })
                }
                Err(e) => Err(RawSqlError::Query(e.to_string())),
            }
        }

        fn runner_name(&self) -> &'static str {
            "mock"
        }
    }

    /// Creates a test server with a `MockRawSqlRunner` wired in.
    ///
    /// `sql_query_enabled` controls `HFS_SOF_SQL_QUERY_ENABLED`.
    fn create_server_with_runner(
        runner: Option<Arc<MockRawSqlRunner>>,
        sql_query_enabled: bool,
    ) -> TestServer {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create in-memory SQLite backend");
        backend.init_schema().expect("failed to init schema");

        let mut config = ServerConfig::for_testing();
        config.sof_sql_query_enabled = sql_query_enabled;

        let mut state = AppState::new(Arc::new(backend), config);

        if let Some(r) = runner {
            use helios_persistence::core::raw_sql::RawSqlRunner as RR;
            state = state.with_raw_sql_runner(r as Arc<dyn RR>);
        }

        let app = helios_rest::routing::fhir_routes::create_routes(state);
        TestServer::new(app).expect("failed to create test server")
    }

    /// Convenience: server with the feature enabled and a simple mock runner.
    fn enabled_server(rows: Vec<SqlRow>) -> TestServer {
        create_server_with_runner(Some(MockRawSqlRunner::ok(rows)), true)
    }

    fn fhir_parameters(query: &str) -> Value {
        json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "query", "valueString": query }
            ]
        })
    }

    // -------------------------------------------------------------------------
    // Feature gate
    // -------------------------------------------------------------------------

    /// When `sof_sql_query_enabled = false` (the default), the endpoint returns
    /// `501 Not Implemented`.
    #[tokio::test]
    async fn test_disabled_by_default_returns_501() {
        let server = create_server_with_runner(None, false);

        let resp = server
            .post("/$sql-query-run")
            .add_header(X_TENANT_ID, TENANT_TEST)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters("SELECT 1"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::NOT_IMPLEMENTED);

        let body: Value = resp.json();
        assert_eq!(body["resourceType"], "OperationOutcome");
        assert_eq!(body["issue"][0]["code"], "not-supported");
    }

    /// When enabled but no runner is configured, also returns `501`.
    #[tokio::test]
    async fn test_enabled_but_no_runner_returns_501() {
        let server = create_server_with_runner(None, true);

        let resp = server
            .post("/$sql-query-run")
            .add_header(X_TENANT_ID, TENANT_TEST)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters("SELECT 1"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::NOT_IMPLEMENTED);
    }

    // -------------------------------------------------------------------------
    // SQL validation
    // -------------------------------------------------------------------------

    /// DDL (`DROP TABLE`) must be rejected with `400 Bad Request`.
    #[tokio::test]
    async fn test_ddl_drop_table_rejected_400() {
        let server = enabled_server(vec![]);

        let resp = server
            .post("/$sql-query-run")
            .add_header(X_TENANT_ID, TENANT_TEST)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters("DROP TABLE resources"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);

        let body: Value = resp.json();
        assert_eq!(body["issue"][0]["code"], "invalid");
        let diag = body["issue"][0]["diagnostics"].as_str().unwrap();
        assert!(
            diag.contains("DROP"),
            "expected diagnostics to mention DROP, got: {diag}"
        );
    }

    /// `INSERT` is rejected with `400`.
    #[tokio::test]
    async fn test_dml_insert_rejected_400() {
        let server = enabled_server(vec![]);

        let resp = server
            .post("/$sql-query-run")
            .add_header(X_TENANT_ID, TENANT_TEST)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters(
                "INSERT INTO resources (id) VALUES ('evil')",
            ))
            .await;

        assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
    }

    /// Multiple statements are rejected (would allow a DDL smuggled after a SELECT).
    #[tokio::test]
    async fn test_multiple_statements_rejected_400() {
        let server = enabled_server(vec![]);

        let resp = server
            .post("/$sql-query-run")
            .add_header(X_TENANT_ID, TENANT_TEST)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters("SELECT 1; DROP TABLE resources"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
    }

    /// A valid `SELECT` passes validation and returns 200.
    #[tokio::test]
    async fn test_valid_select_returns_200() {
        let server = enabled_server(vec![json!({"id": "pt-1", "name": "Smith"})]);

        let resp = server
            .post("/$sql-query-run")
            .add_header(X_TENANT_ID, TENANT_TEST)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters(
                "SELECT id, name FROM resources WHERE resource_type = 'Patient'",
            ))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);
    }

    // -------------------------------------------------------------------------
    // Output formats
    // -------------------------------------------------------------------------

    /// Default output is NDJSON; Content-Type is `application/x-ndjson`.
    #[tokio::test]
    async fn test_output_ndjson_default() {
        let rows = vec![
            json!({"id": "pt-1", "family": "Smith"}),
            json!({"id": "pt-2", "family": "Jones"}),
        ];
        let server = enabled_server(rows);

        let resp = server
            .post("/$sql-query-run")
            .add_header(X_TENANT_ID, TENANT_TEST)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters("SELECT id, family FROM resources"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            ct.contains("ndjson"),
            "expected ndjson content-type, got: {ct}"
        );

        let body = resp.text();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2);

        let row1: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(row1["id"], "pt-1");
        let row2: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(row2["family"], "Jones");
    }

    /// With `?_format=csv`, output is CSV with a header row.
    #[tokio::test]
    async fn test_output_csv_format() {
        let rows = vec![
            json!({"id": "pt-1", "family": "Smith"}),
            json!({"id": "pt-2", "family": "Jones"}),
        ];
        let server = enabled_server(rows);

        let resp = server
            .post("/$sql-query-run?_format=csv")
            .add_header(X_TENANT_ID, TENANT_TEST)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters("SELECT id, family FROM resources"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.contains("text/csv"), "expected text/csv, got: {ct}");

        let body = resp.text();
        let lines: Vec<&str> = body.lines().collect();
        // header + 2 data rows
        assert_eq!(lines.len(), 3, "expected 3 CSV lines, got: {body:?}");
        assert!(lines[0].contains("id"), "header line should contain 'id'");
        assert!(lines[1].contains("pt-1"));
        assert!(lines[2].contains("pt-2"));
    }

    /// Empty result set returns 200 with empty body (ndjson).
    #[tokio::test]
    async fn test_empty_result_set() {
        let server = enabled_server(vec![]);

        let resp = server
            .post("/$sql-query-run")
            .add_header(X_TENANT_ID, TENANT_TEST)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters("SELECT id FROM resources WHERE 1=0"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);
        assert!(resp.text().trim().is_empty());
    }

    // -------------------------------------------------------------------------
    // Error cases from runner
    // -------------------------------------------------------------------------

    /// Row limit exceeded → `422 Unprocessable Entity`.
    #[tokio::test]
    async fn test_row_limit_exceeded_returns_422() {
        let server =
            create_server_with_runner(Some(MockRawSqlRunner::row_limit_exceeded(100)), true);

        let resp = server
            .post("/$sql-query-run")
            .add_header(X_TENANT_ID, TENANT_TEST)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters("SELECT * FROM resources"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::UNPROCESSABLE_ENTITY);

        let body: Value = resp.json();
        assert_eq!(body["issue"][0]["code"], "too-costly");
    }

    /// Query timeout → `504 Gateway Timeout`.
    #[tokio::test]
    async fn test_timeout_returns_504() {
        let server = create_server_with_runner(Some(MockRawSqlRunner::timeout(30)), true);

        let resp = server
            .post("/$sql-query-run")
            .add_header(X_TENANT_ID, TENANT_TEST)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters("SELECT * FROM resources"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::GATEWAY_TIMEOUT);

        let body: Value = resp.json();
        assert_eq!(body["issue"][0]["code"], "timeout");
    }

    // -------------------------------------------------------------------------
    // Request body parsing
    // -------------------------------------------------------------------------

    /// Empty body returns `400` with a helpful message.
    #[tokio::test]
    async fn test_empty_body_returns_400() {
        let server = enabled_server(vec![]);

        let resp = server
            .post("/$sql-query-run")
            .add_header(X_TENANT_ID, TENANT_TEST)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .bytes(axum::body::Bytes::new())
            .await;

        assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
    }

    /// Missing `query` parameter in the Parameters body returns `400`.
    #[tokio::test]
    async fn test_missing_query_param_returns_400() {
        let server = enabled_server(vec![]);

        // Well-formed Parameters but no `query` entry
        let body = json!({
            "resourceType": "Parameters",
            "parameter": []
        });

        let resp = server
            .post("/$sql-query-run")
            .add_header(X_TENANT_ID, TENANT_TEST)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&body)
            .await;

        assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
    }

    /// The handler also accepts `{"query": "SELECT ..."}` as a shorthand body.
    #[tokio::test]
    async fn test_bare_query_object_accepted() {
        let server = enabled_server(vec![json!({"n": 1})]);

        let resp = server
            .post("/$sql-query-run")
            .add_header(X_TENANT_ID, TENANT_TEST)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_JSON)
            .json(&json!({"query": "SELECT 1 AS n"}))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);
    }

    // -------------------------------------------------------------------------
    // Tenant isolation (mock verifies the correct tenant_id is forwarded)
    // -------------------------------------------------------------------------

    /// A mock that records the `tenant_id` it was called with.
    struct TenantCapturingRunner {
        captured: tokio::sync::Mutex<Option<String>>,
    }

    impl TenantCapturingRunner {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                captured: tokio::sync::Mutex::new(None),
            })
        }

        async fn captured_tenant(&self) -> Option<String> {
            self.captured.lock().await.clone()
        }
    }

    #[async_trait]
    impl RawSqlRunner for TenantCapturingRunner {
        async fn run_query(
            &self,
            tenant_id: &str,
            _sql: &str,
            _max_rows: usize,
            _timeout_secs: u64,
        ) -> Result<Vec<SqlRow>, RawSqlError> {
            *self.captured.lock().await = Some(tenant_id.to_string());
            Ok(vec![])
        }

        fn runner_name(&self) -> &'static str {
            "capturing"
        }
    }

    /// The runner receives the tenant extracted from the `X-Tenant-ID` header.
    #[tokio::test]
    async fn test_tenant_id_forwarded_to_runner() {
        let runner = TenantCapturingRunner::new();

        let backend = SqliteBackend::with_config(":memory:", Default::default()).unwrap();
        backend.init_schema().unwrap();

        let mut config = ServerConfig::for_testing();
        config.sof_sql_query_enabled = true;

        let mut state = AppState::new(Arc::new(backend), config);
        use helios_persistence::core::raw_sql::RawSqlRunner as RR;
        state = state.with_raw_sql_runner(Arc::clone(&runner) as Arc<dyn RR>);

        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).unwrap();

        server
            .post("/$sql-query-run")
            .add_header(X_TENANT_ID, TENANT_CLINIC_A)
            .add_header(CONTENT_TYPE, CONTENT_TYPE_FHIR)
            .json(&fhir_parameters("SELECT 1"))
            .await;

        let tenant = runner.captured_tenant().await;
        assert_eq!(tenant.as_deref(), Some("clinic-a"));
    }
}
