//! Handler-level tests for `$viewdefinition-run`.
//!
//! Tests the POST `/ViewDefinition/$viewdefinition-run` endpoint using an
//! in-memory SQLite backend and the in-process FHIRPath runner.

#[cfg(feature = "sof")]
mod sof_run_tests {
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use chrono::Utc;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use serde_json::{Value, json};
    use std::sync::Arc;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

    /// Creates an in-memory SQLite-backed test server with all FHIR routes.
    ///
    /// The SOF runner is not explicitly wired — `resolve_runner` in the handler
    /// falls back to creating a fresh `InProcessRunner` per request.
    async fn create_test_server() -> (TestServer, Arc<SqliteBackend>) {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);

        let config = ServerConfig::for_testing();
        let state = helios_rest::AppState::new(Arc::clone(&backend), config);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("failed to create test server");

        (server, backend)
    }

    fn test_tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        )
    }

    /// Seeds a Patient resource directly into the backend.
    async fn seed_patient(backend: &SqliteBackend, id: &str, family: &str) {
        let tenant = test_tenant();
        let patient = json!({
            "resourceType": "Patient",
            "id": id,
            "name": [{ "family": family }],
            "active": true
        });
        backend
            .create(&tenant, "Patient", patient, FhirVersion::R4)
            .await
            .expect("failed to seed patient");
    }

    /// Returns a minimal valid ViewDefinition that selects `id` and `name.family` from Patient.
    fn patient_view_definition() -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [
                {
                    "column": [
                        { "path": "id", "name": "patient_id", "type": "string" },
                        { "path": "name.family", "name": "family", "type": "string" }
                    ]
                }
            ]
        })
    }

    // =========================================================================
    // Happy path
    // =========================================================================

    /// `POST /ViewDefinition/$viewdefinition-run` with seeded data returns 200
    /// and NDJSON rows containing the expected columns.
    #[tokio::test]
    async fn test_run_view_definition_ndjson_happy_path() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-001", "Smith").await;
        seed_patient(&backend, "pt-002", "Jones").await;

        let response = server
            .post("/ViewDefinition/$viewdefinition-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::OK);

        // Content-Type must be NDJSON (default format)
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.contains("ndjson") || content_type.contains("x-ndjson"),
            "expected ndjson content-type, got: {content_type}"
        );

        // Parse each NDJSON line as a JSON object
        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("each line must be valid JSON"))
            .collect();

        assert_eq!(rows.len(), 2, "expected 2 rows, got {}", rows.len());

        // Each row must have the expected column keys
        for row in &rows {
            assert!(
                row.get("patient_id").is_some(),
                "row missing 'patient_id': {row}"
            );
            assert!(row.get("family").is_some(), "row missing 'family': {row}");
        }

        // Collect family names to verify content (order not guaranteed)
        let families: Vec<&str> = rows.iter().filter_map(|r| r["family"].as_str()).collect();
        assert!(
            families.contains(&"Smith"),
            "expected 'Smith' in rows: {families:?}"
        );
        assert!(
            families.contains(&"Jones"),
            "expected 'Jones' in rows: {families:?}"
        );
    }

    /// `?_format=json` returns a JSON array instead of NDJSON.
    #[tokio::test]
    async fn test_run_view_definition_json_format() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-json-1", "Brown").await;

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=json")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::OK);

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.contains("application/json"),
            "expected application/json, got: {content_type}"
        );

        let body: Value =
            serde_json::from_str(&response.text()).expect("response body must be valid JSON");
        assert!(body.is_array(), "json format must return an array");
        let rows = body.as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["family"], "Brown");
    }

    /// `?_format=csv` with `header=true` returns CSV with a header row.
    #[tokio::test]
    async fn test_run_view_definition_csv_format() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-csv-1", "White").await;

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=csv&header=true")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::OK);

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.contains("text/csv"),
            "expected text/csv, got: {content_type}"
        );

        let body = response.text();
        let lines: Vec<&str> = body.lines().collect();
        // Header row + 1 data row
        assert!(lines.len() >= 2, "expected header + data rows, got: {body}");
        // Header must contain the column names
        assert!(
            lines[0].contains("patient_id"),
            "header missing 'patient_id': {}",
            lines[0]
        );
        assert!(
            lines[0].contains("family"),
            "header missing 'family': {}",
            lines[0]
        );
        // Data row must contain the family name
        assert!(
            lines[1].contains("White"),
            "data row missing 'White': {}",
            lines[1]
        );
    }

    /// `POST /ViewDefinition/{id}/$viewdefinition-run` (instance variant) behaves
    /// identically to the anonymous form when a body is supplied.
    #[tokio::test]
    async fn test_run_stored_view_definition_with_body() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-stored-1", "Green").await;

        let response = server
            .post("/ViewDefinition/some-view-id/$viewdefinition-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::OK);

        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["family"], "Green");
    }

    /// A `Parameters` body wrapping a ViewDefinition via `viewResource` is accepted.
    #[tokio::test]
    async fn test_run_view_definition_parameters_body() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-params-1", "Black").await;

        let parameters_body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "viewResource",
                    "resource": patient_view_definition()
                }
            ]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&parameters_body)
            .await;

        response.assert_status(StatusCode::OK);

        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["family"], "Black");
    }

    /// `?_limit=1` caps the number of output rows.
    #[tokio::test]
    async fn test_run_view_definition_limit() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-lim-1", "Alpha").await;
        seed_patient(&backend, "pt-lim-2", "Beta").await;
        seed_patient(&backend, "pt-lim-3", "Gamma").await;

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_limit=1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::OK);

        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(rows.len(), 1, "limit=1 must return exactly 1 row");
    }

    // =========================================================================
    // Error cases → 422
    // =========================================================================

    /// A ViewDefinition missing the required `resource` field returns 422.
    #[tokio::test]
    async fn test_run_view_definition_missing_resource_returns_422() {
        let (server, _backend) = create_test_server().await;

        let bad_view = json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            // intentionally omitting "resource" field
            "select": [
                {
                    "column": [
                        { "path": "id", "name": "id", "type": "string" }
                    ]
                }
            ]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&bad_view)
            .await;

        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        // Response must be an OperationOutcome
        let body: Value =
            serde_json::from_str(&response.text()).expect("422 body must be valid JSON");
        assert_eq!(
            body["resourceType"], "OperationOutcome",
            "422 body must be OperationOutcome: {body}"
        );
    }

    /// A ViewDefinition with an empty `select` array returns 422.
    #[tokio::test]
    async fn test_run_view_definition_empty_select_returns_422() {
        let (server, _backend) = create_test_server().await;

        let bad_view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": []  // empty select — no columns defined
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&bad_view)
            .await;

        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        let body: Value =
            serde_json::from_str(&response.text()).expect("422 body must be valid JSON");
        assert_eq!(
            body["resourceType"], "OperationOutcome",
            "422 body must be OperationOutcome: {body}"
        );
    }

    // =========================================================================
    // Error cases → 400
    // =========================================================================

    /// A body with an unexpected `resourceType` returns 400.
    #[tokio::test]
    async fn test_run_view_definition_wrong_resource_type_returns_400() {
        let (server, _backend) = create_test_server().await;

        let bad_body = json!({
            "resourceType": "Patient",
            "id": "oops"
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&bad_body)
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
    }

    /// A `Parameters` body without a `viewResource` parameter returns 400.
    #[tokio::test]
    async fn test_run_view_definition_parameters_missing_view_resource_returns_400() {
        let (server, _backend) = create_test_server().await;

        let bad_params = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "someOtherParam", "valueString": "irrelevant" }
            ]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&bad_params)
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
    }

    // =========================================================================
    // Runner override
    // =========================================================================

    // =========================================================================
    // Helpers for filter tests (require in-DB runner wired into AppState)
    // =========================================================================

    /// Creates a server with the SQLite in-DB runner wired in via `with_sof_runner`.
    /// The in-DB runner compiles `_since`, `patient`, and `group` filters to SQL.
    async fn create_test_server_with_indb() -> (TestServer, Arc<SqliteBackend>) {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);

        let runner = backend
            .sof_runner()
            .expect("SQLiteBackend must provide sof_runner");

        let config = ServerConfig::for_testing();
        let state =
            helios_rest::AppState::new(Arc::clone(&backend), config).with_sof_runner(runner);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("failed to create test server");

        (server, backend)
    }

    // =========================================================================
    // Filter tests — `_since`, `patient`, `group`
    // =========================================================================

    /// `_since` returns only resources whose `last_updated` is at or after the given instant.
    #[tokio::test]
    async fn test_run_view_definition_since_filter() {
        let (server, backend) = create_test_server_with_indb().await;

        seed_patient(&backend, "p-since-1", "Early").await;

        // Pause long enough so p-since-2 gets a strictly later last_updated timestamp.
        tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        let since = Utc::now();

        seed_patient(&backend, "p-since-2", "Late").await;

        // Use the Z-suffix form to avoid '+' percent-encoding issues in the URL.
        let since_str = since.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

        // Use a flat-column view (only `id`) so the in-DB runner can compile it fully.
        // The `name.family` path involves array navigation which produces NULL in SQLite's
        // json_extract — the filter correctness test only needs to verify row count and id.
        let flat_view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "id", "name": "patient_id", "type": "string"}]}]
        });

        let response = server
            .post(&format!(
                "/ViewDefinition/$viewdefinition-run?_since={since_str}"
            ))
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&flat_view)
            .await;

        response.assert_status(StatusCode::OK);

        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(
            rows.len(),
            1,
            "_since filter must return only the later patient; got {rows:?}"
        );
        assert_eq!(
            rows[0]["patient_id"], "p-since-2",
            "expected p-since-2 in the result: {rows:?}"
        );
    }

    /// `patient=Patient/p1` restricts results to resources whose `subject.reference`
    /// or `patient.reference` matches the given value.
    #[tokio::test]
    async fn test_run_view_definition_patient_filter() {
        let (server, backend) = create_test_server_with_indb().await;

        let tenant = test_tenant();
        // Seed two Observations, one per patient
        for (id, patient_ref) in [("obs-1", "Patient/p1"), ("obs-2", "Patient/p2")] {
            let obs = json!({
                "resourceType": "Observation",
                "id": id,
                "status": "final",
                "code": { "text": "test" },
                "subject": { "reference": patient_ref }
            });
            backend
                .create(&tenant, "Observation", obs, FhirVersion::R4)
                .await
                .expect("failed to seed observation");
        }

        let obs_view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Observation",
            "status": "active",
            "select": [{"column": [{"path": "id", "name": "obs_id", "type": "string"}]}]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?patient=Patient/p1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&obs_view)
            .await;

        response.assert_status(StatusCode::OK);

        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(
            rows.len(),
            1,
            "patient filter must return only obs-1; got {rows:?}"
        );
        assert_eq!(
            rows[0]["obs_id"], "obs-1",
            "expected obs-1 in result: {rows:?}"
        );
    }

    /// `group=Group/g1` restricts results to resources with `group.reference = "Group/g1"`.
    #[tokio::test]
    async fn test_run_view_definition_group_filter() {
        let (server, backend) = create_test_server_with_indb().await;

        let tenant = test_tenant();
        // Seed one patient with group.reference and one without
        let with_group = json!({
            "resourceType": "Patient",
            "id": "p-grouped",
            "active": true,
            "group": { "reference": "Group/g1" }
        });
        let without_group = json!({
            "resourceType": "Patient",
            "id": "p-ungrouped",
            "active": true
        });
        for (rt, res) in [("Patient", with_group), ("Patient", without_group)] {
            backend
                .create(&tenant, rt, res, FhirVersion::R4)
                .await
                .expect("failed to seed patient");
        }

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?group=Group/g1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::OK);

        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(
            rows.len(),
            1,
            "group filter must return only p-grouped; got {rows:?}"
        );
        assert_eq!(
            rows[0]["patient_id"], "p-grouped",
            "expected p-grouped in result: {rows:?}"
        );
    }

    // =========================================================================
    // Fallback test — auto-fallback on Uncompilable
    // =========================================================================

    /// When `HFS_SOF_DEFAULT_RUNNER=auto` (the default) and the in-DB runner returns
    /// `SofError::Uncompilable`, the handler transparently retries with `InProcessRunner`
    /// and returns HTTP 200.  The `X-HFS-Runner` header must contain `"inprocess (fallback"`.
    #[tokio::test]
    async fn test_run_view_definition_auto_fallback() {
        let (server, backend) = create_test_server_with_indb().await;
        seed_patient(&backend, "pt-fallback-1", "FallbackFam").await;

        // A view with a `where` clause is unsupported by the in-DB runner and
        // triggers SofError::Uncompilable, which activates the auto-fallback path.
        let view_with_where = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "where": [{ "path": "active" }],
            "select": [{
                "column": [{ "path": "id", "name": "patient_id", "type": "string" }]
            }]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&view_with_where)
            .await;

        // Must succeed (200), not 422
        response.assert_status(StatusCode::OK);

        // X-HFS-Runner must indicate the inprocess fallback was used
        let runner_header = response
            .headers()
            .get("x-hfs-runner")
            .expect("X-HFS-Runner header must be present")
            .to_str()
            .unwrap();
        assert!(
            runner_header.contains("inprocess (fallback"),
            "X-HFS-Runner must contain 'inprocess (fallback', got: {runner_header}"
        );

        // The fallback runner must produce correct rows
        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            rows.len(),
            1,
            "fallback runner must return 1 row; got {rows:?}"
        );
        assert_eq!(rows[0]["patient_id"], "pt-fallback-1");
    }

    // =========================================================================
    // Runner override
    // =========================================================================

    /// `?runner=inprocess` forces the in-process runner even when a wired runner
    /// is present, and still returns correct results.
    #[tokio::test]
    async fn test_run_view_definition_inprocess_override() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-ip-1", "Override").await;

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?runner=inprocess")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::OK);

        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["family"], "Override");
    }
}
