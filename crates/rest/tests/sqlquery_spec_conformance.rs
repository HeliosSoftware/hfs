//! `$sqlquery-run` conformance tests against the SoF v2 reference scenarios.
//!
//! The SQL-on-FHIR v2 IG does not ship declarative JSON test fixtures for
//! `$sqlquery-run` the way it does for ViewDefinition (see
//! `crates/sof/tests/sql-on-fhir-v2/tests/*.json`). The closest upstream
//! reference is `HL7/sql-on-fhir:sof-js/tests/server/sql.test.js`, a
//! JavaScript/Bun test suite that drives a live HTTP server. This file
//! ports those scenarios to Rust integration tests against our own
//! `$sqlquery-run` REST handler so they run in CI alongside the rest of
//! our test suite.
//!
//! Each test below carries a `// sof-js:` comment naming the equivalent
//! upstream test it covers. Tests in `sof_sqlquery.rs` already cover most
//! of the request/response shapes; this file focuses on the gaps:
//!
//! 1. **OperationOutcome `issue[0].code`** assertions — sof-js asserts
//!    specific FHIR issue codes (`not-found`, `invalid`, `processing`),
//!    not just HTTP statuses.
//! 2. **`_format=fhir` for a boolean column** — round-trips through the
//!    handler with a VD-declared `boolean` column type and confirms the
//!    output `Parameters` part uses `valueBoolean`.
//! 3. **Empty-result `Parameters`** — confirms the `parameter` key is
//!    *omitted* (FHIR JSON convention for empty repeating elements; the
//!    sof-js test expects `body.parameter === undefined`).
//! 4. **Documented deviation** from sof-js: SoF v2 PR #353 makes
//!    `_format` default to `ndjson`, while the older sof-js test still
//!    asserts a 400 with `required` when `_format` is omitted. The
//!    deviation is intentional — we track the newer spec.

mod sqlquery_spec_conformance_tests {
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use serde_json::{Value, json};
    use std::sync::Arc;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

    const LIB_TYPE_SYSTEM: &str = "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes";

    async fn create_test_server() -> (TestServer, Arc<SqliteBackend>) {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);

        let runner = backend
            .sof_runner()
            .expect("SqliteBackend must provide an in-DB SOF runner");

        let config = ServerConfig::for_testing();
        let state =
            helios_rest::AppState::new(Arc::clone(&backend), config).with_sof_runner(runner);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("failed to create test server");

        (server, backend)
    }

    fn tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        )
    }

    async fn seed_patient(backend: &SqliteBackend, id: &str, family: &str, active: bool) {
        let p = json!({
            "resourceType": "Patient",
            "id": id,
            "name": [{"family": family}],
            "active": active,
        });
        backend
            .create(&tenant(), "Patient", p, FhirVersion::R4)
            .await
            .expect("seed patient");
    }

    /// ViewDefinition with a VD-declared boolean column (`active`). Returns
    /// the relative `ViewDefinition/{id}` reference; the SQLQuery Library
    /// uses it as a `depends-on`.
    async fn seed_boolean_view(backend: &SqliteBackend) -> String {
        let id = "patient-active-flat";
        let vd = json!({
            "resourceType": "ViewDefinition",
            "id": id,
            "url": "http://example.org/sof/ViewDefinition/patient-active-flat",
            "version": "1.0.0",
            "resource": "Patient",
            "status": "active",
            "select": [{
                "column": [
                    {"path": "id", "name": "patient_id", "type": "string"},
                    {"path": "active", "name": "active", "type": "boolean"}
                ]
            }]
        });
        backend
            .create_or_update(&tenant(), "ViewDefinition", id, vd, FhirVersion::R4)
            .await
            .expect("seed view definition");
        format!("ViewDefinition/{id}")
    }

    fn library(sql: &str, depends_on_url: &str, label: &str, parameters: Vec<Value>) -> Value {
        let data = B64.encode(sql.as_bytes());
        let mut lib = json!({
            "resourceType": "Library",
            "id": "demo",
            "status": "active",
            "type": {"coding": [{"system": LIB_TYPE_SYSTEM, "code": "sql-query"}]},
            "content": [{ "contentType": "application/sql", "data": data }],
            "relatedArtifact": [{
                "type": "depends-on",
                "label": label,
                "resource": depends_on_url
            }],
        });
        if !parameters.is_empty() {
            lib["parameter"] = json!(parameters);
        }
        lib
    }

    fn run_body_inline(lib: Value, format: &str, inner_params: Option<Value>) -> Value {
        let mut entries = vec![
            json!({"name": "_format", "valueCode": format}),
            json!({"name": "queryResource", "resource": lib}),
        ];
        if let Some(p) = inner_params {
            entries.push(json!({"name": "parameters", "resource": p}));
        }
        json!({"resourceType": "Parameters", "parameter": entries})
    }

    async fn post(server: &TestServer, path: &str, body: &Value) -> axum_test::TestResponse {
        server
            .post(path)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(body)
            .await
    }

    /// Returns `body.issue[0].code` from an OperationOutcome response. Panics
    /// if the body is not an OperationOutcome or the field is missing.
    fn issue_code(body: &Value) -> &str {
        assert_eq!(
            body.get("resourceType").and_then(|v| v.as_str()),
            Some("OperationOutcome"),
            "expected OperationOutcome, got {body}"
        );
        body.get("issue")
            .and_then(|i| i.as_array())
            .and_then(|arr| arr.first())
            .and_then(|first| first.get("code"))
            .and_then(|c| c.as_str())
            .expect("issue[0].code missing")
    }

    // =========================================================================
    // OperationOutcome shape conformance — sof-js tests assert specific
    // `issue[0].code` values, not just HTTP statuses.
    // =========================================================================

    /// sof-js: `unknown Library id on instance route returns 404`.
    #[tokio::test]
    async fn unknown_library_returns_operation_outcome_not_found() {
        let (server, _) = create_test_server().await;
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "_format", "valueCode": "json"}]
        });
        let response = post(&server, "/Library/does-not-exist/$sqlquery-run", &body).await;
        response.assert_status(StatusCode::NOT_FOUND);
        let body: Value = response.json();
        assert_eq!(issue_code(&body), "not-found");
    }

    /// sof-js: `referenced ViewDefinition that cannot be resolved returns 404`.
    #[tokio::test]
    async fn unresolved_view_definition_returns_operation_outcome_not_found() {
        let (server, _) = create_test_server().await;
        let lib = library(
            "SELECT 1 AS one FROM t",
            "http://example.org/ViewDefinition/no-such-view",
            "t",
            vec![],
        );
        let body = run_body_inline(lib, "json", None);
        let response = post(&server, "/$sqlquery-run", &body).await;
        response.assert_status(StatusCode::NOT_FOUND);
        let body: Value = response.json();
        assert_eq!(issue_code(&body), "not-found");
    }

    /// sof-js: `unknown nested parameter name returns 400` with
    /// `issue[0].code === 'invalid'`. Our `bind_supplied_params` rejects
    /// unknown supplied names.
    #[tokio::test]
    async fn unknown_supplied_parameter_returns_operation_outcome_invalid() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd = seed_boolean_view(&backend).await;
        // Library declares no parameters; the supplied `unknown_param` must
        // be rejected.
        let lib = library("SELECT patient_id FROM t", &vd, "t", vec![]);
        let inner_params = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "unknown_param", "valueString": "x"}]
        });
        let body = run_body_inline(lib, "json", Some(inner_params));
        let response = post(&server, "/$sqlquery-run", &body).await;
        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        assert_eq!(issue_code(&body), "invalid");
    }

    /// sof-js: `parameter type mismatch with declared Library.parameter.type
    /// returns 400` with `issue[0].code === 'invalid'`. The Library declares
    /// `:name` as a string; we supply `valueInteger` and expect rejection.
    #[tokio::test]
    async fn parameter_type_mismatch_returns_operation_outcome_invalid() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd = seed_boolean_view(&backend).await;
        let lib = library(
            "SELECT patient_id FROM t WHERE patient_id = :name",
            &vd,
            "t",
            vec![json!({"name": "name", "use": "in", "type": "string"})],
        );
        let inner_params = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "name", "valueInteger": 42}]
        });
        let body = run_body_inline(lib, "json", Some(inner_params));
        let response = post(&server, "/$sqlquery-run", &body).await;
        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        assert_eq!(issue_code(&body), "invalid");
    }

    /// sof-js: `SQL referencing a non-existent column returns 422` with
    /// `issue[0].code === 'processing'`. The SQL parses (SELECT) but
    /// executing against the materialised table fails on the missing column.
    #[tokio::test]
    async fn sql_runtime_error_returns_operation_outcome_processing() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd = seed_boolean_view(&backend).await;
        let lib = library("SELECT no_such_column FROM t", &vd, "t", vec![]);
        let body = run_body_inline(lib, "json", None);
        let response = post(&server, "/$sqlquery-run", &body).await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        let body: Value = response.json();
        assert_eq!(issue_code(&body), "processing");
    }

    // =========================================================================
    // `_format=fhir` output shape conformance.
    // =========================================================================

    /// sof-js: `boolean column maps to valueBoolean under _format=fhir`.
    /// Verifies end-to-end: a VD-declared `boolean` column round-trips
    /// through query execution and emits `valueBoolean` (not `valueInteger`)
    /// in the `Parameters` output.
    #[tokio::test]
    async fn boolean_column_emits_value_boolean_under_format_fhir() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        seed_patient(&backend, "p2", "Jones", false).await;
        let vd = seed_boolean_view(&backend).await;
        let lib = library("SELECT patient_id, active FROM t", &vd, "t", vec![]);
        let body = run_body_inline(lib, "fhir", None);
        let response = post(&server, "/$sqlquery-run", &body).await;
        response.assert_status(StatusCode::OK);
        let body: Value = response.json();
        assert_eq!(body["resourceType"], json!("Parameters"));
        let rows = body["parameter"]
            .as_array()
            .expect("Parameters.parameter must be present for non-empty result");
        assert!(rows.len() >= 2, "expected at least 2 rows, got {rows:?}");
        for row in rows {
            assert_eq!(row["name"], json!("row"));
            let active = row["part"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == json!("active"))
                .expect("active part must be present");
            assert!(
                active.get("valueBoolean").is_some(),
                "boolean column must emit valueBoolean, got {active}"
            );
            assert!(
                active.get("valueInteger").is_none() && active.get("valueString").is_none(),
                "boolean column must NOT emit valueInteger or valueString, got {active}"
            );
        }
    }

    /// sof-js: `empty result under _format=fhir returns Parameters with no
    /// parameter array`. Strict FHIR JSON convention: empty repeating
    /// elements are omitted, not emitted as `"parameter": []`.
    #[tokio::test]
    async fn empty_result_omits_parameter_key() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd = seed_boolean_view(&backend).await;
        // SQL that returns zero rows.
        let lib = library(
            "SELECT patient_id FROM t WHERE patient_id = 'no-such-patient'",
            &vd,
            "t",
            vec![],
        );
        let body = run_body_inline(lib, "fhir", None);
        let response = post(&server, "/$sqlquery-run", &body).await;
        response.assert_status(StatusCode::OK);
        let body: Value = response.json();
        assert_eq!(body["resourceType"], json!("Parameters"));
        assert!(
            body.get("parameter").is_none(),
            "empty result must omit the 'parameter' key, got {body}"
        );
    }

    // =========================================================================
    // Documented deviation from sof-js.
    // =========================================================================

    /// **DEVIATION** from sof-js's `missing _format returns 400` test.
    ///
    /// SoF v2 PR #353 ("`_format` is `0..1` and defaults to `ndjson`") made
    /// the operation's `_format` optional. sof-js's test still asserts a
    /// 400 with `issue[0].code === 'required'`; we follow the newer spec
    /// and default to `ndjson`. This test pins our behavior so any future
    /// drift is visible.
    #[tokio::test]
    async fn missing_format_defaults_to_ndjson_per_pr_353() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd = seed_boolean_view(&backend).await;
        let lib = library("SELECT patient_id FROM t", &vd, "t", vec![]);
        // No `_format` in body or query string, no `Accept` header.
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "queryResource", "resource": lib}]
        });
        let response = post(&server, "/$sqlquery-run", &body).await;
        response.assert_status(StatusCode::OK);
        let ct = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            ct.contains("application/x-ndjson") || ct.contains("application/ndjson"),
            "expected ndjson default, got Content-Type: {ct}"
        );
    }
}
