//! Handler-level tests for `$viewdefinition-export`.
//!
//! Tests the POST `/ViewDefinition/$viewdefinition-export`, GET/DELETE
//! `/_operations/export/{job-id}`, and GET `/_operations/export/{job-id}/{file}`
//! endpoints using an in-memory SQLite backend and InMemoryController.

mod sof_export_tests {
    use axum::http::{HeaderName, StatusCode};
    use axum_test::TestServer;
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

        // Build runner: prefer in-DB runner from the backend
        let runner: Arc<dyn SofRunner> = backend
            .sof_runner()
            .expect("SQLiteBackend must provide sof_runner");

        // Build in-memory sink and controller
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

    fn patient_view() -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{
                "column": [
                    {"path": "id", "name": "patient_id", "type": "string"}
                ]
            }]
        })
    }

    /// Polls the status URL until the job completes (303), then fetches and
    /// returns the manifest from the result URL. Times out after ~2s.
    async fn poll_to_manifest(server: &TestServer, status_url: &str, tenant: &str) -> Value {
        for _ in 0..40 {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let poll = server.get(status_url).add_header(X_TENANT_ID, tenant).await;
            if poll.status_code() == StatusCode::SEE_OTHER {
                let result_url = poll
                    .headers()
                    .get("location")
                    .expect("303 response missing Location header")
                    .to_str()
                    .unwrap()
                    .to_string();
                let result = server
                    .get(&result_url)
                    .add_header(X_TENANT_ID, tenant)
                    .await;
                assert_eq!(
                    result.status_code(),
                    StatusCode::OK,
                    "result URL did not return 200: {}",
                    result.text()
                );
                return result.json::<Value>();
            }
        }
        panic!("export did not complete within 2s for {status_url}");
    }

    // =========================================================================
    // 1. Submit → 202 + Content-Location
    // =========================================================================

    #[tokio::test]
    async fn test_export_submit_returns_202() {
        let (server, backend) = create_test_server_with_export().await;
        seed_patients(&backend).await;

        let resp = server
            .post("/ViewDefinition/$viewdefinition-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&patient_view())
            .await;

        assert_eq!(resp.status_code(), StatusCode::ACCEPTED, "{}", resp.text());
        assert!(
            resp.headers().contains_key("content-location"),
            "missing Content-Location header"
        );
        let location = resp
            .headers()
            .get("content-location")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            location.starts_with("/_operations/export/"),
            "unexpected location: {location}"
        );
    }

    // =========================================================================
    // 2. Poll → eventually 200 + manifest
    // =========================================================================

    #[tokio::test]
    async fn test_export_poll_to_completion() {
        let (server, backend) = create_test_server_with_export().await;
        seed_patients(&backend).await;

        // Submit
        let submit_resp = server
            .post("/ViewDefinition/$viewdefinition-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&patient_view())
            .await;
        assert_eq!(submit_resp.status_code(), StatusCode::ACCEPTED);

        let location = submit_resp
            .headers()
            .get("content-location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let manifest = poll_to_manifest(&server, &location, "test-tenant").await;
        assert_eq!(
            manifest["resourceType"].as_str(),
            Some("Parameters"),
            "expected Parameters manifest: {manifest}"
        );

        // Verify output file is listed in manifest
        let params = manifest["parameter"].as_array().unwrap();
        let has_output = params.iter().any(|p| p["name"].as_str() == Some("output"));
        assert!(
            has_output,
            "manifest missing 'output' parameter: {manifest}"
        );
    }

    // =========================================================================
    // 3. Cancel → 202 Accepted, then poll → 404
    // =========================================================================

    #[tokio::test]
    async fn test_export_cancel() {
        let (server, _backend) = create_test_server_with_export().await;

        // Submit (no data seeded — export will complete fast, but we'll cancel immediately)
        let submit_resp = server
            .post("/ViewDefinition/$viewdefinition-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&patient_view())
            .await;
        assert_eq!(submit_resp.status_code(), StatusCode::ACCEPTED);

        let location = submit_resp
            .headers()
            .get("content-location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        // Cancel immediately
        let cancel_resp = server
            .delete(&location)
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        // 202 means cancellation accepted (per spec); 200 means it already
        // completed — either is acceptable depending on timing.
        let cancel_status = cancel_resp.status_code();
        assert!(
            cancel_status == StatusCode::ACCEPTED || cancel_status == StatusCode::OK,
            "unexpected cancel status: {cancel_status}"
        );

        // If we cancelled (202), polling should now return 404.
        if cancel_status == StatusCode::ACCEPTED {
            let poll = server
                .get(&location)
                .add_header(X_TENANT_ID, "test-tenant")
                .await;
            assert_eq!(
                poll.status_code(),
                StatusCode::NOT_FOUND,
                "cancelled job should return 404: {}",
                poll.text()
            );
        }
    }

    // =========================================================================
    // 4. Missing controller → 503
    // =========================================================================

    #[tokio::test]
    async fn test_export_without_controller_returns_503() {
        // Create a server WITHOUT an export controller
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);

        let config = ServerConfig::for_testing();
        let state = helios_rest::AppState::new(backend, config);
        // No .with_export_controller(...)

        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("failed to create test server");

        let resp = server
            .post("/ViewDefinition/$viewdefinition-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&patient_view())
            .await;

        assert_eq!(resp.status_code(), StatusCode::SERVICE_UNAVAILABLE);
    }

    // =========================================================================
    // 5. 422 on invalid ViewDefinition (missing 'resource' field)
    // =========================================================================

    #[tokio::test]
    async fn test_export_422_on_missing_resource() {
        let (server, _) = create_test_server_with_export().await;

        let bad_view = json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "select": [{"column": [{"path": "id", "name": "id"}]}]
            // Missing "resource" field
        });

        let resp = server
            .post("/ViewDefinition/$viewdefinition-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&bad_view)
            .await;

        assert_eq!(resp.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    // =========================================================================
    // 6. Download file from completed export
    // =========================================================================

    #[tokio::test]
    async fn test_export_download_file() {
        let (server, backend) = create_test_server_with_export().await;
        seed_patients(&backend).await;

        // Submit
        let submit_resp = server
            .post("/ViewDefinition/$viewdefinition-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&patient_view())
            .await;
        assert_eq!(submit_resp.status_code(), StatusCode::ACCEPTED);

        let location = submit_resp
            .headers()
            .get("content-location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let manifest = poll_to_manifest(&server, &location, "test-tenant").await;

        // Extract download URL from manifest (spec shape: output[].part[name=location].valueUri)
        let params = manifest["parameter"]
            .as_array()
            .expect("expected parameter array");
        let output_param = params
            .iter()
            .find(|p| p["name"].as_str() == Some("output"))
            .expect("missing output parameter");
        let url = output_param["part"]
            .as_array()
            .and_then(|parts| {
                parts
                    .iter()
                    .find(|p| p["name"].as_str() == Some("location"))
            })
            .and_then(|p| p["valueUri"].as_str())
            .expect("missing location part with valueUri");

        // The URL is absolute (http://localhost/...), extract the path
        let path = url.trim_start_matches("http://localhost");

        // Download the file
        let download_resp = server
            .get(path)
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(
            download_resp.status_code(),
            StatusCode::OK,
            "download failed: {}",
            download_resp.text()
        );

        // Verify it contains NDJSON rows
        let body = download_resp.text();
        assert!(!body.is_empty(), "downloaded file should not be empty");
        // Each line should be a valid JSON object
        for line in body.lines() {
            serde_json::from_str::<Value>(line)
                .unwrap_or_else(|e| panic!("invalid NDJSON line: {line:?} — {e}"));
        }
    }

    // =========================================================================
    // 7. Multi-shard export: shard_rows = 1 forces one row per shard
    // =========================================================================

    #[tokio::test]
    async fn test_export_multi_shard() {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);

        // Seed 3 patients
        let tenant = test_tenant();
        for (id, family) in [("p1", "Smith"), ("p2", "Jones"), ("p3", "Taylor")] {
            let resource = json!({
                "resourceType": "Patient",
                "id": id,
                "name": [{"family": family}]
            });
            backend
                .create(&tenant, "Patient", resource, FhirVersion::R4)
                .await
                .expect("failed to seed patient");
        }

        let runner: Arc<dyn SofRunner> = backend
            .sof_runner()
            .expect("SQLiteBackend must provide sof_runner");

        let sink = InMemorySink::new("http://localhost");
        // shard_rows = 1 forces each row into its own shard
        let controller = InMemoryController::with_shard_rows(runner, sink, None, Some(1));

        let config = ServerConfig::for_testing();
        let state = helios_rest::AppState::new(Arc::clone(&backend), config)
            .with_export_controller(Arc::new(controller));

        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("failed to create test server");

        // Submit
        let submit_resp = server
            .post("/ViewDefinition/$viewdefinition-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&patient_view())
            .await;
        assert_eq!(submit_resp.status_code(), StatusCode::ACCEPTED);

        let location = submit_resp
            .headers()
            .get("content-location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let manifest = poll_to_manifest(&server, &location, "test-tenant").await;
        assert_eq!(
            manifest["resourceType"].as_str(),
            Some("Parameters"),
            "export did not complete: {manifest}"
        );

        // With shard_rows=1 and 3 patients we expect 3 output shards
        let params = manifest["parameter"].as_array().unwrap();
        let output_count = params
            .iter()
            .filter(|p| p["name"].as_str() == Some("output"))
            .count();
        assert_eq!(
            output_count, 3,
            "expected 3 shards for 3 rows with shard_rows=1, got {output_count}: {manifest}"
        );

        // Each shard URL should have a distinct index (shard-0, shard-1, shard-2)
        let urls: Vec<&str> = params
            .iter()
            .filter(|p| p["name"].as_str() == Some("output"))
            .filter_map(|p| {
                p["part"].as_array().and_then(|parts| {
                    parts
                        .iter()
                        .find(|q| q["name"].as_str() == Some("location"))
                        .and_then(|q| q["valueUri"].as_str())
                })
            })
            .collect();
        for (i, url) in urls.iter().enumerate() {
            assert!(
                url.contains(&format!("shard-{i}")),
                "shard {i} URL should contain 'shard-{i}', got: {url}"
            );
        }
    }

    // =========================================================================
    // 8. Parquet format export: shard bytes start with PAR1 magic
    // =========================================================================

    #[tokio::test]
    async fn test_export_parquet_format() {
        let (server, backend) = create_test_server_with_export().await;
        seed_patients(&backend).await;

        // Submit with _format=parquet
        let submit_resp = server
            .post("/ViewDefinition/$viewdefinition-export?_format=parquet")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&patient_view())
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
        assert_eq!(
            manifest["resourceType"].as_str(),
            Some("Parameters"),
            "export did not complete: {manifest}"
        );

        // Extract shard URL (spec shape: output[].part[name=location].valueUri)
        let params = manifest["parameter"]
            .as_array()
            .expect("expected parameter array");
        let output_param = params
            .iter()
            .find(|p| p["name"].as_str() == Some("output"))
            .expect("missing output parameter");
        let url = output_param["part"]
            .as_array()
            .and_then(|parts| {
                parts
                    .iter()
                    .find(|p| p["name"].as_str() == Some("location"))
            })
            .and_then(|p| p["valueUri"].as_str())
            .expect("missing location part with valueUri");
        let path = url.trim_start_matches("http://localhost");

        // Download the shard
        let download_resp = server
            .get(path)
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(
            download_resp.status_code(),
            StatusCode::OK,
            "download failed: {}",
            download_resp.text()
        );

        // Content-Type must be application/octet-stream for Parquet
        let ct = download_resp
            .headers()
            .get("content-type")
            .expect("missing Content-Type header")
            .to_str()
            .unwrap();
        assert_eq!(
            ct, "application/octet-stream",
            "unexpected Content-Type: {ct}"
        );

        // Bytes must start with Parquet magic b"PAR1"
        let body = download_resp.as_bytes().to_vec();
        assert!(!body.is_empty(), "Parquet shard must not be empty");
        assert_eq!(
            &body[..4],
            b"PAR1",
            "Parquet shard must start with PAR1 magic bytes"
        );
    }

    // =========================================================================
    // 9. Manifest fields conform to SQL-on-FHIR v2 spec (T1.4)
    // =========================================================================

    #[tokio::test]
    async fn test_export_manifest_uses_spec_field_names() {
        let (server, backend) = create_test_server_with_export().await;
        seed_patients(&backend).await;

        let submit_resp = server
            .post("/ViewDefinition/$viewdefinition-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&patient_view())
            .await;
        assert_eq!(submit_resp.status_code(), StatusCode::ACCEPTED);

        let location = submit_resp
            .headers()
            .get("content-location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let manifest = poll_to_manifest(&server, &location, "test-tenant").await;
        assert_eq!(
            manifest["resourceType"].as_str(),
            Some("Parameters"),
            "export did not complete: {manifest}"
        );

        let params = manifest["parameter"].as_array().unwrap();
        let names: Vec<&str> = params.iter().filter_map(|p| p["name"].as_str()).collect();

        for required in [
            "exportId",
            "status",
            "location",
            "cancelUrl",
            "_format",
            "exportStartTime",
            "exportEndTime",
            "exportDuration",
            "output",
        ] {
            assert!(
                names.contains(&required),
                "manifest missing '{required}' parameter; got: {names:?}"
            );
        }

        // `status` must be the code "completed"
        let status_code = params
            .iter()
            .find(|p| p["name"].as_str() == Some("status"))
            .and_then(|p| p["valueCode"].as_str());
        assert_eq!(status_code, Some("completed"));

        // Spec-shaped output: each `output` has `part[name=location]` of type valueUri.
        let output = params
            .iter()
            .find(|p| p["name"].as_str() == Some("output"))
            .expect("missing output");
        let parts = output["part"].as_array().expect("output.part must exist");
        let has_location = parts
            .iter()
            .any(|p| p["name"].as_str() == Some("location") && p["valueUri"].as_str().is_some());
        assert!(
            has_location,
            "output.part missing 'location' with valueUri: {output}"
        );
    }

    // =========================================================================
    // 10. Tenant isolation on status/cancel/download (T1.2)
    // =========================================================================

    #[tokio::test]
    async fn test_export_tenant_isolation() {
        let (server, backend) = create_test_server_with_export().await;
        seed_patients(&backend).await;

        // Tenant A submits an export.
        let submit_resp = server
            .post("/ViewDefinition/$viewdefinition-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "tenant-a")
            .json(&patient_view())
            .await;
        assert_eq!(submit_resp.status_code(), StatusCode::ACCEPTED);

        let location = submit_resp
            .headers()
            .get("content-location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        // Tenant B must not be able to poll, cancel, or download — every
        // operation returns 404 (not 200/202/204) to avoid leaking existence.
        let poll_b = server
            .get(&location)
            .add_header(X_TENANT_ID, "tenant-b")
            .await;
        assert_eq!(
            poll_b.status_code(),
            StatusCode::NOT_FOUND,
            "tenant-b must not see tenant-a's export: {}",
            poll_b.text()
        );

        let cancel_b = server
            .delete(&location)
            .add_header(X_TENANT_ID, "tenant-b")
            .await;
        assert_eq!(
            cancel_b.status_code(),
            StatusCode::NOT_FOUND,
            "tenant-b must not cancel tenant-a's export"
        );

        // Tenant A can still see its own export — wait for it to finish.
        // poll_to_manifest follows the 303 → $result redirect.
        let manifest = poll_to_manifest(&server, &location, "tenant-a").await;
        assert_eq!(manifest["resourceType"].as_str(), Some("Parameters"));
    }

    // =========================================================================
    // 11. Prefer: respond-async (T3.3) required at kickoff
    // =========================================================================

    #[tokio::test]
    async fn test_export_missing_prefer_returns_400() {
        let (server, _backend) = create_test_server_with_export().await;
        let resp = server
            .post("/ViewDefinition/$viewdefinition-export")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&patient_view())
            .await;
        assert_eq!(
            resp.status_code(),
            StatusCode::BAD_REQUEST,
            "{}",
            resp.text()
        );
    }

    // =========================================================================
    // 12. clientTrackingId (T5.4) echoed in the completion manifest
    // =========================================================================

    #[tokio::test]
    async fn test_export_client_tracking_id_echo() {
        let (server, backend) = create_test_server_with_export().await;
        seed_patients(&backend).await;

        let submit_resp = server
            .post("/ViewDefinition/$viewdefinition-export?clientTrackingId=tracker-42")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&patient_view())
            .await;
        assert_eq!(submit_resp.status_code(), StatusCode::ACCEPTED);
        let location = submit_resp
            .headers()
            .get("content-location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let manifest = poll_to_manifest(&server, &location, "test-tenant").await;
        let params = manifest["parameter"].as_array().unwrap();
        let tracking = params
            .iter()
            .find(|p| p["name"].as_str() == Some("clientTrackingId"))
            .and_then(|p| p["valueString"].as_str());
        assert_eq!(tracking, Some("tracker-42"));
    }

    // =========================================================================
    // 13. JSON format export (T5.2)
    // =========================================================================

    #[tokio::test]
    async fn test_export_json_format() {
        let (server, backend) = create_test_server_with_export().await;
        seed_patients(&backend).await;

        let submit_resp = server
            .post("/ViewDefinition/$viewdefinition-export?_format=json")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&patient_view())
            .await;
        assert_eq!(submit_resp.status_code(), StatusCode::ACCEPTED);
        let location = submit_resp
            .headers()
            .get("content-location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let manifest = poll_to_manifest(&server, &location, "test-tenant").await;

        // Format is echoed in the manifest.
        let format = manifest["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"].as_str() == Some("_format"))
            .and_then(|p| p["valueCode"].as_str());
        assert_eq!(format, Some("json"));

        // Download the shard and assert it's a JSON array.
        let url = manifest["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"].as_str() == Some("output"))
            .and_then(|p| {
                p["part"].as_array().and_then(|parts| {
                    parts
                        .iter()
                        .find(|q| q["name"].as_str() == Some("location"))
                        .and_then(|q| q["valueUri"].as_str())
                })
            })
            .expect("missing location");
        let path = url.trim_start_matches("http://localhost");
        let dl = server
            .get(path)
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(dl.status_code(), StatusCode::OK);
        let body = dl.text();
        assert!(
            body.trim_start().starts_with('['),
            "expected JSON array, got: {body}"
        );
    }

    // =========================================================================
    // 14. Multi-view export (T2.5): `view[]` with two named views
    // =========================================================================

    #[tokio::test]
    async fn test_export_multi_view() {
        let (server, backend) = create_test_server_with_export().await;
        seed_patients(&backend).await;

        // Two views: one over Patient (named "demographics"), one a copy of
        // the same view (named "demographics2") to keep the test self-contained.
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "view", "part": [
                    {"name": "name", "valueString": "demographics"},
                    {"name": "viewResource", "resource": patient_view()}
                ]},
                {"name": "view", "part": [
                    {"name": "name", "valueString": "demographics2"},
                    {"name": "viewResource", "resource": patient_view()}
                ]}
            ]
        });

        let submit_resp = server
            .post("/ViewDefinition/$viewdefinition-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&body)
            .await;
        assert_eq!(submit_resp.status_code(), StatusCode::ACCEPTED);
        let location = submit_resp
            .headers()
            .get("content-location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let manifest = poll_to_manifest(&server, &location, "test-tenant").await;
        let params = manifest["parameter"].as_array().unwrap();

        // Should have at least two `output` entries, one per view, each
        // carrying the view name in its `part`.
        let output_names: Vec<&str> = params
            .iter()
            .filter(|p| p["name"].as_str() == Some("output"))
            .filter_map(|p| {
                p["part"].as_array().and_then(|parts| {
                    parts
                        .iter()
                        .find(|q| q["name"].as_str() == Some("name"))
                        .and_then(|q| q["valueString"].as_str())
                })
            })
            .collect();
        assert!(
            output_names.contains(&"demographics"),
            "manifest missing demographics: {output_names:?}"
        );
        assert!(
            output_names.contains(&"demographics2"),
            "manifest missing demographics2: {output_names:?}"
        );
    }
}
