//! Integration tests for `POST /$sql-run` (SoF v2).

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
        create_test_server_with_config(ServerConfig::for_testing()).await
    }

    async fn create_test_server_with_config(
        config: ServerConfig,
    ) -> (TestServer, Arc<SqliteBackend>) {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);

        let runner = backend
            .sof_runner()
            .expect("SqliteBackend must provide an in-DB SOF runner");

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
            "name": "patient_flat",
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
            json!({"name": "subjectResource", "resource": library}),
        ];
        if let Some(p) = inner_params {
            entries.push(json!({"name": "parameters", "resource": p}));
        }
        json!({"resourceType": "Parameters", "parameter": entries})
    }

    fn run_body_reference(reference: &str, format: &str, inner_params: Option<Value>) -> Value {
        let mut entries = vec![
            json!({"name": "_format", "valueCode": format}),
            json!({"name": "subjectReference", "valueReference": {"reference": reference}}),
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
            .post("/$sql-run")
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
            .post("/$sql-run")
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
            .post("/$sql-run")
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
            .post("/$sql-run")
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
            .post("/$sql-run")
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
            .post("/$sql-run")
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
    // Naming a stored Library as the subject
    // =========================================================================

    /// `$sql-run` is `instance=false`, so a stored Library is named by
    /// `subjectReference` rather than by an instance-level URL.
    #[tokio::test]
    async fn subject_reference_binds_stored_library() {
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
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "subjectReference", "valueReference": {"reference": "Library/demo"}}
            ]
        });
        let response = server
            .post("/$sql-run")
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

    /// The pre-ballot type- and instance-level Library endpoints were
    /// consolidated into the system-level `$sql-run`.
    #[tokio::test]
    async fn pre_ballot_library_urls_are_not_routed() {
        let (server, _) = create_test_server().await;
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "_format", "valueCode": "json"}]
        });
        for url in [
            "/Library/$sqlquery-run",
            "/Library/demo/$sqlquery-run",
            "/$sqlquery-run",
        ] {
            let response = server
                .post(url)
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .add_header(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/fhir+json"),
                )
                .json(&body)
                .expect_failure()
                .await;
            assert_ne!(
                response.status_code(),
                StatusCode::OK,
                "{url} was consolidated into $sql-run"
            );
        }
    }

    // =========================================================================
    // Errors
    // =========================================================================

    #[tokio::test]
    async fn missing_format_defaults_to_ndjson() {
        // SoF v2 PR #353: `_format` is `0..1` and defaults to `ndjson` when
        // neither `_format` (body or query) nor a usable `Accept` header is
        // supplied. Previously returned 400; now returns ndjson.
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd("SELECT patient_id FROM t", &vd_url, "t", vec![]);
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "subjectResource", "resource": lib}]
        });
        let response = server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::OK);
        let content_type = response
            .header(axum::http::header::CONTENT_TYPE)
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            content_type.starts_with("application/x-ndjson"),
            "default _format should be ndjson, got Content-Type: {content_type}"
        );
    }

    /// SoF v2 PR #353: `_limit` truncates the final result set silently;
    /// returning fewer rows than the cap is not an error.
    #[tokio::test]
    async fn limit_in_body_truncates_silently() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        seed_patient(&backend, "p2", "Jones", false).await;
        seed_patient(&backend, "p3", "Lee", true).await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd(
            "SELECT patient_id FROM t ORDER BY patient_id",
            &vd_url,
            "t",
            vec![],
        );
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "subjectResource", "resource": lib},
                {"name": "_limit", "valueInteger": 2}
            ]
        });
        let response = server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::OK);
        let rows: Value = response.json();
        assert_eq!(
            rows.as_array().map(|a| a.len()),
            Some(2),
            "_limit=2 should cap at 2 rows, got {rows}"
        );
    }

    /// A final result cap must not truncate the ViewDefinition dependency before SQL runs.
    #[tokio::test]
    async fn limit_on_final_query_does_not_truncate_view_dependency_above_preview_cap() {
        let (server, backend) = create_test_server().await;
        for i in 0..80 {
            seed_patient(&backend, &format!("count-all-{i:03}"), "CountAll", true).await;
        }
        let vd_url = seed_patient_view(&backend).await;
        let library =
            library_with_canonical_vd("SELECT COUNT(*) AS total FROM t", &vd_url, "t", vec![]);
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "subjectResource", "resource": library},
                {"name": "_limit", "valueInteger": 1}
            ]
        });
        let response = server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::OK);
        let rows: Value = response.json();
        assert_eq!(
            rows,
            json!([{"total": 80}]),
            "_limit=1 caps the final query, while COUNT must see all 80 dependency rows"
        );
    }

    /// Posts a `$sql-run` for `library` with `extra` Parameters entries and
    /// returns the response.
    async fn post_library_run(
        server: &TestServer,
        url: &str,
        library: Value,
        extra: Vec<Value>,
    ) -> axum_test::TestResponse {
        let mut entries = vec![
            json!({"name": "_format", "valueCode": "json"}),
            json!({"name": "subjectResource", "resource": library}),
        ];
        entries.extend(extra);
        server
            .post(url)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&json!({"resourceType": "Parameters", "parameter": entries}))
            .await
    }

    /// #1701: `patient`, `group` and `_since` narrow every dependency
    /// ViewDefinition of a Library subject.
    #[tokio::test]
    async fn dependency_views_honour_patient_group_and_since() {
        let (server, backend) = create_test_server().await;
        for id in ["p1", "p2", "p3"] {
            seed_patient(&backend, id, "Smith", true).await;
        }
        backend
            .create(
                &tenant(),
                "Group",
                json!({
                    "resourceType": "Group",
                    "id": "g1",
                    "type": "person",
                    "actual": true,
                    "member": [{"entity": {"reference": "Patient/p2"}}]
                }),
                FhirVersion::R4,
            )
            .await
            .expect("seed group");
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd(
            "SELECT patient_id FROM t ORDER BY patient_id",
            &vd_url,
            "t",
            vec![],
        );

        let cases: Vec<(&str, Vec<Value>, Value)> = vec![
            (
                "/$sql-run",
                vec![json!({"name": "patient", "valueReference": {"reference": "Patient/p1"}})],
                json!([{"patient_id": "p1"}]),
            ),
            (
                "/$sql-run",
                vec![json!({"name": "group", "valueReference": {"reference": "Group/g1"}})],
                json!([{"patient_id": "p2"}]),
            ),
            (
                "/$sql-run?patient=Patient/p3",
                vec![],
                json!([{"patient_id": "p3"}]),
            ),
            (
                "/$sql-run",
                vec![json!({"name": "_since", "valueInstant": "2999-01-01T00:00:00Z"})],
                json!([]),
            ),
            (
                "/$sql-run",
                vec![json!({"name": "group", "valueReference": {"reference": "Group/missing"}})],
                json!([]),
            ),
            (
                "/$sql-run",
                vec![json!({"name": "_since", "valueInstant": "2000-01-01T00:00:00Z"})],
                json!([
                    {"patient_id": "p1"},
                    {"patient_id": "p2"},
                    {"patient_id": "p3"}
                ]),
            ),
        ];
        for (url, extra, expected) in cases {
            let response = post_library_run(&server, url, lib.clone(), extra.clone()).await;
            response.assert_status(StatusCode::OK);
            let rows: Value = response.json();
            assert_eq!(rows, expected, "url={url} extra={extra:?}");
        }
    }

    /// #1701: `_limit` caps the subject's final rows, never a filtered
    /// dependency: two patients reach the leaf, the COUNT sees both, and the
    /// single result row survives `_limit=1`.
    #[tokio::test]
    async fn limit_caps_final_rows_not_filtered_dependencies() {
        let (server, backend) = create_test_server().await;
        for id in ["p1", "p2", "p3"] {
            seed_patient(&backend, id, "Smith", true).await;
        }
        let vd_url = seed_patient_view(&backend).await;
        let lib =
            library_with_canonical_vd("SELECT COUNT(*) AS total FROM t", &vd_url, "t", vec![]);
        let response = post_library_run(
            &server,
            "/$sql-run",
            lib,
            vec![
                json!({"name": "patient", "valueReference": {"reference": "Patient/p1"}}),
                json!({"name": "patient", "valueReference": {"reference": "Patient/p2"}}),
                json!({"name": "_limit", "valueInteger": 1}),
            ],
        )
        .await;
        response.assert_status(StatusCode::OK);
        let rows: Value = response.json();
        assert_eq!(rows, json!([{"total": 2}]));
    }

    /// An unparsable `_since` is a 400 for a Library subject, as for a
    /// ViewDefinition (#1550).
    #[tokio::test]
    async fn invalid_since_returns_400_for_library_subject() {
        let (server, backend) = create_test_server().await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd("SELECT patient_id FROM t", &vd_url, "t", vec![]);
        let response = post_library_run(
            &server,
            "/$sql-run",
            lib,
            vec![json!({"name": "_since", "valueInstant": "yesterday"})],
        )
        .await;
        response.assert_status(StatusCode::BAD_REQUEST);
    }

    /// `_limit` works from the URL query string too, and body wins on conflict.
    #[tokio::test]
    async fn limit_in_query_truncates_silently() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        seed_patient(&backend, "p2", "Jones", false).await;
        seed_patient(&backend, "p3", "Lee", true).await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd(
            "SELECT patient_id FROM t ORDER BY patient_id",
            &vd_url,
            "t",
            vec![],
        );
        let body = run_body_inline(lib, "json", None);
        let response = server
            .post("/$sql-run?_limit=1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::OK);
        let rows: Value = response.json();
        assert_eq!(
            rows.as_array().map(|a| a.len()),
            Some(1),
            "_limit=1 (query) should cap at 1 row, got {rows}"
        );
    }

    /// Result set smaller than `_limit` returns all rows without erroring.
    #[tokio::test]
    async fn limit_larger_than_result_is_not_an_error() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd("SELECT patient_id FROM t", &vd_url, "t", vec![]);
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "subjectResource", "resource": lib},
                {"name": "_limit", "valueInteger": 100}
            ]
        });
        let response = server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::OK);
        let rows: Value = response.json();
        assert_eq!(rows.as_array().map(|a| a.len()), Some(1));
    }

    #[tokio::test]
    async fn non_select_sql_returns_400() {
        let (server, backend) = create_test_server().await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd("DELETE FROM t", &vd_url, "t", vec![]);
        let body = run_body_inline(lib, "json", None);
        let response = server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::BAD_REQUEST);
    }

    /// #1702: a `WITH`-prefixed INSERT / UPDATE parses as a query but is still
    /// rejected as non-SELECT SQL before anything runs.
    #[tokio::test]
    async fn with_prefixed_insert_or_update_returns_400() {
        let (server, backend) = create_test_server().await;
        let vd_url = seed_patient_view(&backend).await;
        for (sql, keyword) in [
            (
                "WITH c AS (SELECT 'p9' AS id) INSERT INTO t (patient_id) SELECT id FROM c",
                "INSERT",
            ),
            ("WITH c AS (SELECT 1) UPDATE t SET family = 'x'", "UPDATE"),
        ] {
            let lib = library_with_canonical_vd(sql, &vd_url, "t", vec![]);
            let response = post_inline_json(&server, lib).await;
            response.assert_status(StatusCode::BAD_REQUEST);
            let outcome: Value = response.json();
            assert_eq!(
                outcome["issue"][0]["details"]["text"],
                format!("only SELECT queries are allowed; {keyword} statements are not permitted"),
                "{sql}"
            );
        }
    }

    #[tokio::test]
    async fn source_parameter_returns_400() {
        // Spec marks `source` as 0..1 — an external data source containing
        // ViewDefinition tables. We don't implement external sources, so a
        // request that supplies one is asking for behavior we can't honor.
        // Per the spec's error mapping, return 400 BadRequest.
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd("SELECT patient_id FROM t", &vd_url, "t", vec![]);
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "subjectResource", "resource": lib},
                {"name": "source", "valueString": "http://example.org/data.ndjson"}
            ]
        });
        let response = server
            .post("/$sql-run")
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
    async fn format_from_query_string() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd_url = seed_patient_view(&backend).await;
        let lib =
            library_with_canonical_vd("SELECT patient_id, family FROM t", &vd_url, "t", vec![]);
        // No `_format` in the body; only in the URL query.
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "subjectResource", "resource": lib}]
        });
        let response = server
            .post("/$sql-run?_format=csv")
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
        assert!(response.text().contains("p1,Smith"));
    }

    #[tokio::test]
    async fn format_from_accept_header() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd("SELECT patient_id FROM t", &vd_url, "t", vec![]);
        // No _format in body or URL; rely on Accept.
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "subjectResource", "resource": lib}]
        });
        let response = server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .add_header(
                HeaderName::from_static("accept"),
                HeaderValue::from_static("application/x-ndjson"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::OK);
        let ct = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.starts_with("application/x-ndjson"), "got {ct}");
    }

    #[tokio::test]
    async fn body_format_wins_over_query_string() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd("SELECT patient_id FROM t", &vd_url, "t", vec![]);
        let body = run_body_inline(lib, "json", None);
        // URL says csv, body says json — body wins.
        let response = server
            .post("/$sql-run?_format=csv")
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
        assert!(ct.starts_with("application/json"), "got {ct}");
    }

    #[tokio::test]
    async fn accept_fhir_json_wraps_flat_format_as_binary() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd_url = seed_patient_view(&backend).await;
        let lib =
            library_with_canonical_vd("SELECT patient_id, family FROM t", &vd_url, "t", vec![]);
        let body = run_body_inline(lib, "csv", None);
        let response = server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .add_header(
                HeaderName::from_static("accept"),
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
        assert!(ct.starts_with("application/fhir+json"), "got {ct}");
        let v: Value = response.json();
        assert_eq!(v["resourceType"], json!("Binary"));
        assert!(
            v["contentType"]
                .as_str()
                .unwrap_or("")
                .starts_with("text/csv")
        );
        let data = v["data"].as_str().expect("Binary.data string");
        let decoded = B64.decode(data).expect("Binary.data is valid base64");
        let text = String::from_utf8(decoded).expect("decoded csv is utf8");
        assert!(text.contains("p1,Smith"), "decoded csv: {text}");
    }

    #[tokio::test]
    async fn both_query_resource_and_query_reference_returns_400() {
        let (server, backend) = create_test_server().await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd("SELECT 1 FROM t", &vd_url, "t", vec![]);
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "subjectResource", "resource": lib},
                {"name": "subjectReference", "valueReference": {"reference": "Library/other"}}
            ]
        });
        let response = server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::BAD_REQUEST);
    }

    /// A `subjectReference` naming a Library that does not exist is a 404: the
    /// subject is what the operation is about, so it cannot proceed without it.
    #[tokio::test]
    async fn subject_reference_to_absent_library_returns_404() {
        let (server, _) = create_test_server().await;
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "subjectReference", "valueString": "Library/does-not-exist"}
            ]
        });
        let response = server
            .post("/$sql-run")
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
            .post("/$sql-run")
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
            .post("/$sql-run")
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
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "subjectReference", "valueReference": {"reference": "Library/does-not-exist"}}
            ]
        });
        let response = server
            .post("/$sql-run")
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
    async fn unknown_view_definition_returns_404() {
        // Spec: "Library or ViewDefinition not found" → 404. Both the
        // relative `ViewDefinition/{id}` and the canonical-URL lookup
        // paths now return 404 consistently.
        let (server, _) = create_test_server().await;
        let lib = library_with_canonical_vd(
            "SELECT 1 FROM t",
            "ViewDefinition/does-not-exist",
            "t",
            vec![],
        );
        let body = run_body_inline(lib, "json", None);
        let response = server
            .post("/$sql-run")
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
    async fn unknown_canonical_view_definition_returns_404() {
        // Same expectation when the depends-on resource is an unresolved
        // canonical URL rather than a relative reference.
        let (server, _) = create_test_server().await;
        let lib = library_with_canonical_vd(
            "SELECT 1 FROM t",
            "http://example.org/ViewDefinition/never-registered",
            "t",
            vec![],
        );
        let body = run_body_inline(lib, "json", None);
        let response = server
            .post("/$sql-run")
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
    async fn library_without_sql_query_type_returns_422() {
        let (server, backend) = create_test_server().await;
        let vd_url = seed_patient_view(&backend).await;
        let mut lib = library_with_canonical_vd("SELECT 1 FROM t", &vd_url, "t", vec![]);
        // Strip the spec-required Library.type → 422 MalformedLibrary.
        lib.as_object_mut().unwrap().remove("type");
        let body = run_body_inline(lib, "json", None);
        let response = server
            .post("/$sql-run")
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
            .post("/$sql-run")
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

    /// A Library subject runs through the same `$sql-run` the CapabilityStatement
    /// advertises; there is no separate `$sqlquery-run` to declare.
    #[tokio::test]
    async fn capabilities_declare_one_run_operation() {
        let (server, _) = create_test_server().await;
        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status(StatusCode::OK);
        let v: Value = response.json();
        let names: Vec<&str> = v["rest"][0]["operation"]
            .as_array()
            .expect("rest[0].operation")
            .iter()
            .filter_map(|op| op["name"].as_str())
            .collect();
        assert!(names.contains(&"sql-run"), "{names:?}");
        assert!(
            !names.contains(&"sqlquery-run"),
            "$sqlquery-run was consolidated into $sql-run: {names:?}"
        );
    }

    // =========================================================================
    // Inline `context` artifacts on `$sql-run`
    // =========================================================================

    /// A `$sql-run` with an inline `queryResource` whose `depends-on`
    /// ViewDefinition is supplied via `context` in the same body succeeds
    /// without the ViewDefinition being stored on the server.
    #[tokio::test]
    async fn inline_view_resource_satisfies_depends_on_without_storage() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        seed_patient(&backend, "p2", "Jones", false).await;

        let vd_url = "http://example.org/sof/ViewDefinition/patient-flat-inline";
        let vd = json!({
            "resourceType": "ViewDefinition",
            "url": vd_url,
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [
                {"path": "id", "name": "patient_id", "type": "string"},
                {"path": "name.family", "name": "family", "type": "string"}
            ]}]
        });

        let lib = library_with_canonical_vd(
            "SELECT patient_id, family FROM t ORDER BY patient_id",
            vd_url,
            "t",
            vec![],
        );

        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "subjectResource", "resource": lib},
                {"name": "context", "resource": vd}
            ]
        });

        let response = server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;

        response.assert_status(StatusCode::OK);
        let rows: Value = response.json();
        assert_eq!(rows.as_array().map(|a| a.len()), Some(2));
        let ids: Vec<&str> = rows
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r["patient_id"].as_str())
            .collect();
        assert!(ids.contains(&"p1") && ids.contains(&"p2"));
    }

    /// When a `context` entry's URL matches a `depends-on` canonical, the
    /// inline VD is used and storage is never consulted for that dependency.
    /// A dependency whose URL does NOT appear in any supplied `context` entry
    /// still falls back to storage and returns 404 when absent.
    #[tokio::test]
    async fn depends_on_not_in_inline_views_falls_back_to_storage_404() {
        let (server, _) = create_test_server().await;

        let vd_url = "http://example.org/sof/ViewDefinition/not-stored";
        let lib = library_with_canonical_vd("SELECT patient_id FROM t", vd_url, "t", vec![]);

        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "subjectResource", "resource": lib}
            ]
        });

        let response = server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;

        response.assert_status(StatusCode::NOT_FOUND);
    }

    // =========================================================================
    // Undeclared table check (#841/#842)
    // =========================================================================

    /// A table the subject's own SQL reads but doesn't declare as a
    /// `relatedArtifact[depends-on]` label is rejected before SQLite ever
    /// runs — `422` with one `OperationOutcome.issue` whose `diagnostics`
    /// carries the same `Line: N, Column: M` marker a SQL parse error
    /// already uses.
    #[tokio::test]
    async fn undeclared_table_returns_422_with_line_and_column() {
        let (server, backend) = create_test_server().await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd("SELECT * FROM vv", &vd_url, "v", vec![]);
        let body = run_body_inline(lib, "json", None);
        let response = server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        let outcome: Value = response.json();
        let issues = outcome["issue"].as_array().expect("issue array");
        assert_eq!(issues.len(), 1);
        assert_eq!(
            issues[0]["diagnostics"],
            "unknown table 'vv' at Line: 1, Column: 15; declare it as a \
             relatedArtifact depends-on label or fix the name"
        );
    }

    /// Two tables the subject's SQL reads and doesn't declare produce two
    /// issues, not one — every problem is reported together, the same way
    /// the dependency-graph resolver reports multiple structural problems
    /// found at once.
    #[tokio::test]
    async fn two_undeclared_tables_return_two_issues() {
        let (server, backend) = create_test_server().await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd(
            "SELECT * FROM aa JOIN bb ON aa.id = bb.id",
            &vd_url,
            "v",
            vec![],
        );
        let body = run_body_inline(lib, "json", None);
        let response = server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        let outcome: Value = response.json();
        let issues = outcome["issue"].as_array().expect("issue array");
        assert_eq!(issues.len(), 2);
        let diagnostics: Vec<&str> = issues
            .iter()
            .map(|i| i["diagnostics"].as_str().unwrap_or_default())
            .collect();
        assert!(diagnostics[0].contains("'aa'"), "{diagnostics:?}");
        assert!(diagnostics[1].contains("'bb'"), "{diagnostics:?}");
    }

    /// A SQL whose tables all match a declared label runs exactly as it did
    /// before this check existed — same `200`, same rows.
    #[tokio::test]
    async fn declared_tables_run_unchanged() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith", true).await;
        let vd_url = seed_patient_view(&backend).await;
        let lib = library_with_canonical_vd("SELECT patient_id FROM t", &vd_url, "t", vec![]);
        let body = run_body_inline(lib, "json", None);
        let response = server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::OK);
        let rows: Value = response.json();
        assert_eq!(rows.as_array().map(|a| a.len()), Some(1));
    }

    // =========================================================================
    // Per-dependency row cap (#1473)
    // =========================================================================

    async fn post_inline_json(server: &TestServer, library: Value) -> axum_test::TestResponse {
        server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&run_body_inline(library, "json", None))
            .await
    }

    /// A dependency with more rows than `HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD`
    /// fails with a 422 that names the dependency, the cap and the setting,
    /// even when the query's own WHERE would have narrowed it to one row. The
    /// message must not suggest a WHERE/LIMIT clause: the dependency is
    /// materialized in full before the query's WHERE runs.
    #[tokio::test]
    async fn dependency_over_the_source_row_cap_returns_422_naming_dependency_and_setting() {
        let mut config = ServerConfig::for_testing();
        config.sof_sqlquery_max_source_rows_per_vd = 3;
        let (server, backend) = create_test_server_with_config(config).await;
        for i in 0..5 {
            seed_patient(&backend, &format!("p{i}"), "Smith", true).await;
        }
        let vd_url = seed_patient_view(&backend).await;

        let lib = library_with_canonical_vd(
            "SELECT patient_id FROM t WHERE patient_id = 'p1'",
            &vd_url,
            "t",
            vec![],
        );
        let response = post_inline_json(&server, lib).await;

        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        let outcome: Value = response.json();
        let diagnostics = outcome["issue"][0]["details"]["text"]
            .as_str()
            .unwrap_or_default();
        for needle in [
            "dependency 't'",
            "(ViewDefinition patient_flat)",
            "exceeds 3-row limit",
            "HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD",
            "ViewDefinition 'where'",
        ] {
            assert!(
                diagnostics.contains(needle),
                "missing {needle:?} in: {diagnostics}"
            );
        }
        assert!(
            !diagnostics.contains("add a WHERE/LIMIT clause"),
            "{diagnostics}"
        );
    }

    /// The cap is inclusive: a dependency of exactly the cap's size runs.
    #[tokio::test]
    async fn dependency_at_the_source_row_cap_still_runs() {
        let mut config = ServerConfig::for_testing();
        config.sof_sqlquery_max_source_rows_per_vd = 5;
        let (server, backend) = create_test_server_with_config(config).await;
        for i in 0..5 {
            seed_patient(&backend, &format!("p{i}"), "Smith", true).await;
        }
        let vd_url = seed_patient_view(&backend).await;

        let lib = library_with_canonical_vd("SELECT patient_id FROM t", &vd_url, "t", vec![]);
        let response = post_inline_json(&server, lib).await;

        response.assert_status(StatusCode::OK);
        let rows: Value = response.json();
        assert_eq!(rows.as_array().map(|a| a.len()), Some(5));
    }

    /// The subject's own result is not a dependency: `HFS_SOF_SQLQUERY_MAX_ROWS`
    /// silently truncates it (SoF v2 PR #353) instead of failing the request.
    #[tokio::test]
    async fn subject_result_over_max_rows_is_silently_truncated() {
        let mut config = ServerConfig::for_testing();
        config.sof_sqlquery_max_rows = 2;
        let (server, backend) = create_test_server_with_config(config).await;
        for i in 0..5 {
            seed_patient(&backend, &format!("p{i}"), "Smith", true).await;
        }
        let vd_url = seed_patient_view(&backend).await;

        let lib = library_with_canonical_vd("SELECT patient_id FROM t", &vd_url, "t", vec![]);
        let response = post_inline_json(&server, lib).await;

        response.assert_status(StatusCode::OK);
        let rows: Value = response.json();
        assert_eq!(rows.as_array().map(|a| a.len()), Some(2));
    }
}
