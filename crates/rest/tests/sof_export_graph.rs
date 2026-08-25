//! Integration test proving `$sql-export` consumes the same two-phase
//! dependency-graph resolver as `$sql-run` (design #567): a SQLQuery whose
//! subject depends on a SQLView, which in turn depends on a leaf
//! ViewDefinition, resolves and executes correctly through the async export
//! path.

mod sof_export_graph_tests {
    use axum::http::{HeaderName, StatusCode};
    use axum_test::TestServer;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::core::sof_runner::SofRunner;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use helios_rest::export::{InMemoryController, InMemorySink};
    use serde_json::{Value, json};
    use std::sync::Arc;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const PREFER: HeaderName = HeaderName::from_static("prefer");
    const LIB_TYPE_SYSTEM: &str = "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes";

    fn test_tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        )
    }

    async fn create_test_server_with_export() -> (TestServer, Arc<SqliteBackend>) {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);

        let runner: Arc<dyn SofRunner> = backend
            .sof_runner()
            .expect("SQLiteBackend must provide sof_runner");
        let sink = InMemorySink::new("http://localhost");
        let controller = InMemoryController::new(runner, sink, None);

        let config = ServerConfig::for_testing();
        let state = helios_rest::AppState::new(Arc::clone(&backend), config)
            .with_export_controller(Arc::new(controller));
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("failed to create test server");

        (server, backend)
    }

    async fn seed_patients(backend: &SqliteBackend) {
        let tenant = test_tenant();
        for (id, family) in [("p1", "Smith"), ("p2", "Jones")] {
            let resource = json!({
                "resourceType": "Patient",
                "id": id,
                "name": [{"family": family}],
                "active": true
            });
            backend
                .create(&tenant, "Patient", resource, FhirVersion::R4)
                .await
                .expect("failed to seed patient");
        }
    }

    async fn seed_view_definition(backend: &SqliteBackend, id: &str, url: &str) -> String {
        let vd = json!({
            "resourceType": "ViewDefinition",
            "id": id,
            "url": url,
            "status": "active",
            "resource": "Patient",
            "select": [{
                "column": [
                    {"path": "id", "name": "patient_id", "type": "string"},
                    {"path": "name.family", "name": "family", "type": "string"}
                ]
            }]
        });
        backend
            .create_or_update(&test_tenant(), "ViewDefinition", id, vd, FhirVersion::R4)
            .await
            .expect("seed view definition");
        url.to_string()
    }

    fn sql_lib(
        id: &str,
        url: Option<&str>,
        type_code: &str,
        sql: &str,
        depends_on: &[(&str, &str)],
    ) -> Value {
        let data = B64.encode(sql.as_bytes());
        let mut lib = json!({
            "resourceType": "Library",
            "id": id,
            "status": "active",
            "type": {"coding": [{"system": LIB_TYPE_SYSTEM, "code": type_code}]},
            "content": [{ "contentType": "application/sql", "data": data }],
            "relatedArtifact": depends_on.iter().map(|(label, target)| json!({
                "type": "depends-on",
                "label": label,
                "resource": target
            })).collect::<Vec<_>>(),
        });
        if let Some(u) = url {
            lib["url"] = json!(u);
        }
        lib
    }

    async fn seed_library(backend: &SqliteBackend, id: &str, lib: Value) {
        backend
            .create_or_update(&test_tenant(), "Library", id, lib, FhirVersion::R4)
            .await
            .expect("seed library");
    }

    /// Polls the status URL until the job finishes, returning the completion
    /// manifest. Mirrors the helper in `sof_export.rs`; duplicated here so
    /// this file has no cross-file dependency (each integration test file is
    /// its own binary).
    async fn poll_to_manifest(server: &TestServer, status_url: &str, tenant: &str) -> Value {
        for _ in 0..40 {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let poll = server.get(status_url).add_header(X_TENANT_ID, tenant).await;
            match poll.status_code() {
                StatusCode::SEE_OTHER => {
                    let result_url = poll
                        .headers()
                        .get(axum::http::header::LOCATION)
                        .and_then(|v| v.to_str().ok())
                        .expect("303 completion response missing Location header")
                        .to_string();
                    let result = server
                        .get(&result_url)
                        .add_header(X_TENANT_ID, tenant)
                        .await;
                    assert_eq!(
                        result.status_code(),
                        StatusCode::OK,
                        "result fetch failed: {}",
                        result.text()
                    );
                    return result.json::<Value>();
                }
                StatusCode::ACCEPTED => continue,
                other => panic!("unexpected poll status {other}: {}", poll.text()),
            }
        }
        panic!("export did not complete within 2s for {status_url}");
    }

    #[tokio::test]
    async fn sql_export_resolves_a_sqlview_dependency_chain() {
        let (server, backend) = create_test_server_with_export().await;
        seed_patients(&backend).await;

        let leaf_url = seed_view_definition(&backend, "leaf", "http://example.org/leaf").await;
        let mid_url = "http://example.org/mid";
        seed_library(
            &backend,
            "mid",
            sql_lib(
                "mid",
                Some(mid_url),
                "sql-view",
                "SELECT * FROM leaf_t",
                &[("leaf_t", &leaf_url)],
            ),
        )
        .await;

        let subject = sql_lib(
            "subject",
            None,
            "sql-query",
            "SELECT patient_id, family FROM mid_t ORDER BY patient_id",
            &[("mid_t", mid_url)],
        );
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "subject", "part": [
                {"name": "name", "valueString": "families"},
                {"name": "subjectResource", "resource": subject}
            ]}]
        });

        let submit_resp = server
            .post("/$sql-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&body)
            .await;
        assert_eq!(
            submit_resp.status_code(),
            StatusCode::ACCEPTED,
            "{}",
            submit_resp.text()
        );
        let location = submit_resp
            .headers()
            .get("content-location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let manifest = poll_to_manifest(&server, &location, "test-tenant").await;
        let params = manifest["parameter"].as_array().unwrap();
        let output = params
            .iter()
            .find(|p| p["name"].as_str() == Some("output"))
            .expect("manifest must have an output entry");
        let output_parts = output["part"].as_array().unwrap();
        let file_url = output_parts
            .iter()
            .find(|p| p["name"].as_str() == Some("location"))
            .and_then(|p| p["valueUri"].as_str())
            .expect("output must carry a location")
            .to_string();
        let path = file_url.strip_prefix("http://localhost").unwrap();
        let download = server
            .get(path)
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(download.status_code(), StatusCode::OK);

        let rows: Vec<Value> = download
            .text()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0]["patient_id"], json!("p1"));
        assert_eq!(rows[1]["patient_id"], json!("p2"));
    }

    /// `context` is a single list shared by the whole job (design #568): one
    /// entry satisfying a dependency neither subject's storage has can serve
    /// both subjects in the same `$sql-export` request.
    #[tokio::test]
    async fn a_single_context_entry_serves_two_subjects_in_one_export_job() {
        let (server, backend) = create_test_server_with_export().await;
        seed_patients(&backend).await;

        let vd_url = "http://example.org/export-ctx-vd";
        let vd = json!({
            "resourceType": "ViewDefinition",
            "url": vd_url,
            "status": "active",
            "resource": "Patient",
            "select": [{
                "column": [
                    {"path": "id", "name": "patient_id", "type": "string"},
                    {"path": "name.family", "name": "family", "type": "string"}
                ]
            }]
        });

        let subject_flat = sql_lib(
            "export-ctx-flat",
            None,
            "sql-query",
            "SELECT patient_id, family FROM t ORDER BY patient_id",
            &[("t", vd_url)],
        );
        let subject_count = sql_lib(
            "export-ctx-count",
            None,
            "sql-query",
            "SELECT COUNT(*) AS n FROM t",
            &[("t", vd_url)],
        );

        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "context", "resource": vd},
                {"name": "subject", "part": [
                    {"name": "name", "valueString": "flat"},
                    {"name": "subjectResource", "resource": subject_flat}
                ]},
                {"name": "subject", "part": [
                    {"name": "name", "valueString": "count"},
                    {"name": "subjectResource", "resource": subject_count}
                ]}
            ]
        });

        let submit_resp = server
            .post("/$sql-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&body)
            .await;
        assert_eq!(
            submit_resp.status_code(),
            StatusCode::ACCEPTED,
            "{}",
            submit_resp.text()
        );
        let location = submit_resp
            .headers()
            .get("content-location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let manifest = poll_to_manifest(&server, &location, "test-tenant").await;
        let params = manifest["parameter"].as_array().unwrap();
        let outputs: Vec<&Value> = params
            .iter()
            .filter(|p| p["name"].as_str() == Some("output"))
            .collect();
        assert_eq!(
            outputs.len(),
            2,
            "both subjects must produce an output entry: {manifest}"
        );

        for output in outputs {
            let output_parts = output["part"].as_array().unwrap();
            let name = output_parts
                .iter()
                .find(|p| p["name"].as_str() == Some("name"))
                .and_then(|p| p["valueString"].as_str())
                .expect("output must carry a name");
            let file_url = output_parts
                .iter()
                .find(|p| p["name"].as_str() == Some("location"))
                .and_then(|p| p["valueUri"].as_str())
                .expect("output must carry a location")
                .to_string();
            let path = file_url.strip_prefix("http://localhost").unwrap();
            let download = server
                .get(path)
                .add_header(X_TENANT_ID, "test-tenant")
                .await;
            assert_eq!(download.status_code(), StatusCode::OK);
            let rows: Vec<Value> = download
                .text()
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            match name {
                "flat" => assert_eq!(rows.len(), 2, "{rows:?}"),
                "count" => {
                    assert_eq!(rows.len(), 1, "{rows:?}");
                    assert_eq!(rows[0]["n"], json!(2));
                }
                other => panic!("unexpected output name '{other}'"),
            }
        }
    }
}
