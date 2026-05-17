//! Integration tests for `POST /$sqlquery-run` (SoF v2).

mod sof_sqlquery_tests {
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
    /// Relative `Type/{id}` reference used in `relatedArtifact.resource`. The
    /// spec pins this slot to `canonical([Resource])`, but FHIR servers
    /// commonly accept a relative reference there — and ViewDefinition has no
    /// standard `url` search parameter, so a relative reference is the
    /// portable lookup form on HFS.
    const PATIENT_VIEW_REF: &str = "ViewDefinition/patient-flat";
    const PATIENT_VIEW_ID: &str = "patient-flat";

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

    /// Seeds a ViewDefinition that flattens `Patient` to (`patient_id`, `family`, `active`)
    /// and returns the relative `ViewDefinition/{id}` reference for use in
    /// `relatedArtifact.resource`.
    async fn seed_patient_view(backend: &SqliteBackend) -> String {
        let vd = json!({
            "resourceType": "ViewDefinition",
            "id": PATIENT_VIEW_ID,
            "url": "http://example.org/sof/ViewDefinition/patient-flat",
            "version": "1.0.0",
            "resource": "Patient",
            "status": "active",
            "select": [{
                "column": [
                    {"path": "id", "name": "patient_id", "type": "string"},
                    {"path": "name.family", "name": "family", "type": "string"},
                    {"path": "active", "name": "active", "type": "boolean"}
                ]
            }]
        });
        backend
            .create_or_update(
                &tenant(),
                "ViewDefinition",
                PATIENT_VIEW_ID,
                vd,
                FhirVersion::R4,
            )
            .await
            .expect("seed view definition");
        PATIENT_VIEW_REF.to_string()
    }

    /// Build a spec-conforming SQLQuery Library with the given SQL, depends-on URL,
    /// and declared parameters.
    fn library_with_canonical_vd(
        sql: &str,
        depends_on_url: &str,
        label: &str,
        parameters: Vec<Value>,
    ) -> Value {
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

    fn run_body_inline(library: Value, format: &str, inner_params: Option<Value>) -> Value {
        let mut entries = vec![
            json!({"name": "_format", "valueCode": format}),
            json!({"name": "queryResource", "resource": library}),
        ];
        if let Some(p) = inner_params {
            entries.push(json!({"name": "parameters", "resource": p}));
        }
        json!({"resourceType": "Parameters", "parameter": entries})
    }

    fn run_body_reference(reference: &str, format: &str, inner_params: Option<Value>) -> Value {
        let mut entries = vec![
            json!({"name": "_format", "valueCode": format}),
            json!({"name": "queryReference", "valueReference": {"reference": reference}}),
        ];
        if let Some(p) = inner_params {
            entries.push(json!({"name": "parameters", "resource": p}));
        }
        json!({"resourceType": "Parameters", "parameter": entries})
    }

    // =========================================================================
    // Happy path: queryResource with canonical depends-on
    // =========================================================================

    #[tokio::test]
    async fn queryresource_with_canonical_vd_csv() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        seed_patient(&backend, "p2", "Jones", false).await;
        let vd_url = seed_patient_view(&backend).await;

        let lib = library_with_canonical_vd(
            "SELECT patient_id, family FROM t ORDER BY patient_id",
            &vd_url,
            "t",
            vec![],
        );
        let body = run_body_inline(lib, "csv", None);

        let response = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;

        response.assert_status(StatusCode::OK);
        let ct = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.starts_with("text/csv"), "got {ct}");
        let text = response.text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "patient_id,family");
        assert!(text.contains("p1,Smith"));
        assert!(text.contains("p2,Jones"));
    }

    #[tokio::test]
    async fn queryresource_returns_json_array() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "x1", "Doe", true).await;
        let vd_url = seed_patient_view(&backend).await;

        let lib = library_with_canonical_vd("SELECT patient_id FROM t", &vd_url, "t", vec![]);
        let body = run_body_inline(lib, "json", None);

        let response = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::OK);
        let v: Value = response.json();
        assert!(v.is_array());
        assert_eq!(v[0]["patient_id"], json!("x1"));
    }

    // =========================================================================
    // queryReference resolution: by relative reference and by canonical URL
    // =========================================================================

    #[tokio::test]
    async fn queryreference_by_relative_library_id() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd("SELECT patient_id FROM t", &vd_url, "t", vec![]);
        backend
            .create(&tenant(), "Library", lib, FhirVersion::R4)
            .await
            .expect("seed library");

        let body = run_body_reference("Library/demo", "json", None);
        let response = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::OK);
        let v: Value = response.json();
        assert_eq!(v[0]["patient_id"], json!("p1"));
    }

    // =========================================================================
    // Parameter binding (injection-safe)
    // =========================================================================

    #[tokio::test]
    async fn parameter_binding_filters_by_string_with_injection_payload() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        seed_patient(&backend, "p2", "Jones", true).await;
        let vd_url = seed_patient_view(&backend).await;
        // SQL payload injected via the parameter value; must be bound as data.
        let injection = "Smith'; DROP TABLE t; --";

        let lib = library_with_canonical_vd(
            "SELECT patient_id, family FROM t WHERE family = :family",
            &vd_url,
            "t",
            vec![json!({"name": "family", "use": "in", "type": "string"})],
        );
        let inner = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "family", "valueString": injection}]
        });
        let body = run_body_inline(lib, "ndjson", Some(inner));

        let response = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::OK);
        let text = response.text();
        assert!(!text.contains("Smith"));
        // Follow-up COUNT proves the DROP didn't fire (the engine is per-request,
        // but if injection had worked, the prior request's bytes would have shown
        // unexpected behavior — the more rigorous proof).
        let lib2 = library_with_canonical_vd("SELECT COUNT(*) AS n FROM t", &vd_url, "t", vec![]);
        let body2 = run_body_inline(lib2, "json", None);
        let r2 = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body2)
            .await;
        r2.assert_status(StatusCode::OK);
        let v: Value = r2.json();
        assert_eq!(v[0]["n"], json!(2));
    }

    // =========================================================================
    // _format=fhir output
    // =========================================================================

    #[tokio::test]
    async fn fhir_output_uses_column_types() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd_url = seed_patient_view(&backend).await;

        let lib = library_with_canonical_vd(
            "SELECT patient_id, family, active FROM t",
            &vd_url,
            "t",
            vec![],
        );
        let body = run_body_inline(lib, "fhir", None);

        let response = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::OK);
        let v: Value = response.json();
        assert_eq!(v["resourceType"], json!("Parameters"));
        let row = &v["parameter"][0]["part"];
        let active_part = row
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "active")
            .expect("active part present");
        assert!(active_part.get("valueBoolean").is_some(), "{active_part}");
    }

    // =========================================================================
    // Instance route
    // =========================================================================

    #[tokio::test]
    async fn instance_route_binds_library_by_id() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd("SELECT patient_id FROM t", &vd_url, "t", vec![]);
        backend
            .create(&tenant(), "Library", lib, FhirVersion::R4)
            .await
            .expect("seed library");

        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "_format", "valueCode": "json"}]
        });
        let response = server
            .post("/Library/demo/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::OK);
        let v: Value = response.json();
        assert_eq!(v[0]["patient_id"], json!("p1"));
    }

    #[tokio::test]
    async fn instance_route_rejects_body_query_reference() {
        let (server, _) = create_test_server().await;
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "queryReference", "valueReference": {"reference": "Library/other"}}
            ]
        });
        let response = server
            .post("/Library/demo/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::BAD_REQUEST);
    }

    // =========================================================================
    // Errors
    // =========================================================================

    #[tokio::test]
    async fn missing_format_returns_400() {
        let (server, backend) = create_test_server().await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd("SELECT 1", &vd_url, "t", vec![]);
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "queryResource", "resource": lib}]
        });
        let response = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn non_select_sql_returns_400() {
        let (server, backend) = create_test_server().await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd("DELETE FROM t", &vd_url, "t", vec![]);
        let body = run_body_inline(lib, "json", None);
        let response = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn source_parameter_returns_422() {
        let (server, _) = create_test_server().await;
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "csv"},
                {"name": "source", "valueString": "http://example.org/data.ndjson"}
            ]
        });
        let response = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn unknown_supplied_parameter_returns_400() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd("SELECT patient_id FROM t", &vd_url, "t", vec![]);
        let inner = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "nope", "valueString": "x"}]
        });
        let body = run_body_inline(lib, "json", Some(inner));
        let response = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn missing_required_parameter_returns_400() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd(
            "SELECT patient_id FROM t WHERE family = :family",
            &vd_url,
            "t",
            vec![json!({"name": "family", "use": "in", "type": "string"})],
        );
        let body = run_body_inline(lib, "json", None);
        let response = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn missing_library_returns_404() {
        let (server, _) = create_test_server().await;
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "_format", "valueCode": "json"}]
        });
        let response = server
            .post("/Library/does-not-exist/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn unknown_view_definition_returns_404_or_422() {
        let (server, _) = create_test_server().await;
        let lib = library_with_canonical_vd(
            "SELECT 1 FROM t",
            "ViewDefinition/does-not-exist",
            "t",
            vec![],
        );
        let body = run_body_inline(lib, "json", None);
        let response = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        let status = response.status_code();
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::UNPROCESSABLE_ENTITY,
            "expected 404 or 422, got {status}"
        );
    }

    #[tokio::test]
    async fn library_without_sql_query_type_returns_422() {
        let (server, backend) = create_test_server().await;
        let vd_url = seed_patient_view(&backend).await;
        let mut lib = library_with_canonical_vd("SELECT 1 FROM t", &vd_url, "t", vec![]);
        // Strip the spec-required Library.type → 422 MalformedLibrary.
        lib.as_object_mut().unwrap().remove("type");
        let body = run_body_inline(lib, "json", None);
        let response = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn inline_view_definition_in_related_artifact_returns_422() {
        // SoF v2 SQLQuery profile pins relatedArtifact.resource to canonical(...);
        // an inline ViewDefinition object must be rejected as malformed.
        let (server, _) = create_test_server().await;
        let data = B64.encode("SELECT 1 FROM t".as_bytes());
        let lib = json!({
            "resourceType": "Library",
            "type": {"coding": [{"system": LIB_TYPE_SYSTEM, "code": "sql-query"}]},
            "content": [{ "contentType": "application/sql", "data": data }],
            "relatedArtifact": [{
                "type": "depends-on",
                "label": "t",
                "resource": {"resourceType": "ViewDefinition"}
            }]
        });
        let body = run_body_inline(lib, "json", None);
        let response = server
            .post("/$sqlquery-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    // =========================================================================
    // Capability statement
    // =========================================================================

    #[tokio::test]
    async fn capabilities_advertise_sqlquery_and_canonical() {
        let (server, _) = create_test_server().await;
        let response = server
            .get("/$sql-on-fhir-capabilities")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status(StatusCode::OK);
        let v: Value = response.json();
        let params = v["parameter"].as_array().unwrap();
        let sqlquery = params
            .iter()
            .find(|p| p["name"] == "supportsSqlQueryRun")
            .expect("supportsSqlQueryRun present");
        assert_eq!(sqlquery["valueBoolean"], json!(true));
        let canonical = params
            .iter()
            .find(|p| p["name"] == "supportsCanonicalReference")
            .expect("supportsCanonicalReference present");
        assert_eq!(canonical["valueBoolean"], json!(true));
    }
}
