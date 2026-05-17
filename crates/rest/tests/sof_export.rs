//! Handler-level tests for `$viewdefinition-export`.
//!
//! Tests the POST `/ViewDefinition/$viewdefinition-export`, GET/DELETE
//! `/export/{job-id}/status`, and GET `/export/{job-id}/{file}` endpoints
//! using an in-memory SQLite backend and InMemoryController.

mod sof_export_tests {
    use axum::http::{HeaderName, StatusCode};
    use axum_test::TestServer;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::core::sof_runner::SofRunner;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use helios_rest::export::{
        ExportJobController, ExportTask, InMemoryController, InMemorySink, JobStatus,
    };
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
        // Spec: Content-Location is an absolute URL ending in /status.
        assert!(
            location.starts_with("http://")
                && location.contains("/export/")
                && location.ends_with("/status"),
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

        // Spec: one `output` entry per view, with `location` (1..*) repeating
        // once per shard inside it. shard_rows=1 + 3 patients = 3 locations.
        let params = manifest["parameter"].as_array().unwrap();
        let outputs: Vec<&Value> = params
            .iter()
            .filter(|p| p["name"].as_str() == Some("output"))
            .collect();
        assert_eq!(
            outputs.len(),
            1,
            "expected one output entry per view, got {}: {manifest}",
            outputs.len()
        );
        let locations: Vec<&str> = outputs[0]["part"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["name"].as_str() == Some("location"))
            .filter_map(|p| p["valueUri"].as_str())
            .collect();
        assert_eq!(
            locations.len(),
            3,
            "expected 3 location parts for 3 rows with shard_rows=1, got {}: {manifest}",
            locations.len()
        );
        for (i, url) in locations.iter().enumerate() {
            assert!(
                url.contains(&format!("shard-{i}")),
                "location {i} URL should contain 'shard-{i}', got: {url}"
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

        // Spec-shaped output: each `output` has `part[name=location]` of type valueUri,
        // and only carries the spec-defined `name` / `location` parts (no extras).
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
        for p in parts {
            let part_name = p["name"].as_str().unwrap_or("");
            assert!(
                matches!(part_name, "name" | "location"),
                "spec-conformant `output.part` only allows `name`/`location`, got `{part_name}`: {output}"
            );
        }

        // Spec: `location` and `cancelUrl` must be absolute URLs.
        for required in ["location", "cancelUrl"] {
            let uri = params
                .iter()
                .find(|p| p["name"].as_str() == Some(required))
                .and_then(|p| p["valueUri"].as_str())
                .unwrap_or_else(|| panic!("missing {required}"));
            assert!(
                uri.starts_with("http://") || uri.starts_with("https://"),
                "{required} must be absolute URL, got: {uri}"
            );
        }
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
    // 14. `source` parameter rejected with 400 (spec: unsupported params)
    // =========================================================================

    #[tokio::test]
    async fn test_export_source_param_rejected_in_query() {
        let (server, _backend) = create_test_server_with_export().await;
        let resp = server
            .post("/ViewDefinition/$viewdefinition-export?source=s3://bucket")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&patient_view())
            .await;
        assert_eq!(
            resp.status_code(),
            StatusCode::BAD_REQUEST,
            "{}",
            resp.text()
        );
        let body: Value = resp.json();
        assert_eq!(body["resourceType"].as_str(), Some("OperationOutcome"));
        assert_eq!(
            body["issue"][0]["code"].as_str(),
            Some("not-supported"),
            "expected not-supported code: {body}"
        );
    }

    #[tokio::test]
    async fn test_export_source_param_rejected_in_body() {
        let (server, _backend) = create_test_server_with_export().await;
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "view", "part": [
                    {"name": "viewResource", "resource": patient_view()}
                ]},
                {"name": "source", "valueString": "s3://bucket"}
            ]
        });
        let resp = server
            .post("/ViewDefinition/$viewdefinition-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&body)
            .await;
        assert_eq!(
            resp.status_code(),
            StatusCode::BAD_REQUEST,
            "{}",
            resp.text()
        );
    }

    // =========================================================================
    // 15. In-progress poll body is `Parameters`, not OperationOutcome.
    //     X-Progress is a percentage like "100%" (spec format).
    // =========================================================================

    #[tokio::test]
    async fn test_export_poll_body_is_parameters() {
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

        // Poll repeatedly; capture either the in-progress (202) or completed (303)
        // shape and assert each body shape conforms to the spec.
        for _ in 0..40 {
            let poll = server
                .get(&location)
                .add_header(X_TENANT_ID, "test-tenant")
                .await;
            match poll.status_code() {
                StatusCode::ACCEPTED => {
                    let body: Value = poll.json();
                    assert_eq!(
                        body["resourceType"].as_str(),
                        Some("Parameters"),
                        "in-progress body must be Parameters, got: {body}"
                    );
                    let params = body["parameter"].as_array().unwrap();
                    let status = params
                        .iter()
                        .find(|p| p["name"].as_str() == Some("status"))
                        .and_then(|p| p["valueCode"].as_str());
                    assert_eq!(status, Some("in-progress"));
                    assert!(
                        params
                            .iter()
                            .any(|p| p["name"].as_str() == Some("exportId")),
                        "in-progress body must include exportId: {body}"
                    );
                    // X-Progress must be a percentage ("0%".."99%").
                    let xp = poll
                        .headers()
                        .get("x-progress")
                        .expect("missing X-Progress")
                        .to_str()
                        .unwrap();
                    assert!(
                        xp.ends_with('%'),
                        "X-Progress must be a percentage, got: {xp:?}"
                    );
                    let n: u32 = xp.trim_end_matches('%').parse().unwrap_or_else(|_| {
                        panic!("X-Progress percent must parse as integer: {xp:?}")
                    });
                    assert!(n <= 99, "running percent must be <= 99, got {n}");
                }
                StatusCode::SEE_OTHER => return, // completed — test passes
                other => panic!("unexpected poll status: {other}"),
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }
        // It's fine if the job completed quickly and we never saw a 202.
    }

    // =========================================================================
    // 16. Multi-view export (T2.5): `view[]` with two named views
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

    // =========================================================================
    // 17. Failed jobs: status URL returns 303 → result URL returns 500.
    //     Uses a test-only controller that always reports `Failed`.
    // =========================================================================

    struct FailingController {
        tenant: String,
        job_id: String,
    }

    impl ExportJobController for FailingController {
        fn submit(&self, _task: ExportTask) -> String {
            self.job_id.clone()
        }
        fn get_status(&self, tenant_id: &str, job_id: &str) -> Option<JobStatus> {
            if tenant_id != self.tenant || job_id != self.job_id {
                return None;
            }
            Some(JobStatus::Failed {
                message: "view runner exploded".to_string(),
                submitted_at: chrono::Utc::now(),
            })
        }
        fn cancel(&self, _t: &str, _j: &str) -> bool {
            false
        }
        fn read_shard(&self, _t: &str, _j: &str, _f: &str) -> Option<Vec<u8>> {
            None
        }
    }

    #[tokio::test]
    async fn test_export_failed_status_returns_303_then_500() {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);

        let controller = FailingController {
            tenant: "test-tenant".to_string(),
            job_id: "fail-1".to_string(),
        };
        let config = ServerConfig::for_testing();
        let state = helios_rest::AppState::new(Arc::clone(&backend), config)
            .with_export_controller(Arc::new(controller));
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("failed to create test server");

        // Status endpoint must 303 to /export/fail-1/result, mirroring the
        // success case (spec: terminal states both redirect to result URL).
        let poll = server
            .get("/export/fail-1/status")
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(
            poll.status_code(),
            StatusCode::SEE_OTHER,
            "failed status should 303, got: {} {}",
            poll.status_code(),
            poll.text()
        );
        let loc = poll
            .headers()
            .get("location")
            .expect("303 missing Location")
            .to_str()
            .unwrap();
        // Spec: Location is an absolute URL.
        assert!(
            loc.starts_with("http://") && loc.ends_with("/export/fail-1/result"),
            "expected absolute result URL, got: {loc}"
        );

        // Result endpoint surfaces the OperationOutcome with 500.
        let result = server.get(loc).add_header(X_TENANT_ID, "test-tenant").await;
        assert_eq!(result.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
        let body: Value = result.json();
        assert_eq!(body["resourceType"].as_str(), Some("OperationOutcome"));
        assert!(
            body["issue"][0]["diagnostics"]
                .as_str()
                .unwrap_or("")
                .contains("view runner exploded"),
            "diagnostics must surface failure message: {body}"
        );
    }

    // =========================================================================
    // 18. Empty result set: manifest has zero `output` entries (spec: 0..*).
    // =========================================================================

    #[tokio::test]
    async fn test_export_empty_dataset_yields_no_outputs() {
        // No `seed_patients` — the view will match zero rows.
        let (server, _backend) = create_test_server_with_export().await;

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
        let params = manifest["parameter"].as_array().unwrap();
        let output_count = params
            .iter()
            .filter(|p| p["name"].as_str() == Some("output"))
            .count();
        assert_eq!(
            output_count, 0,
            "empty dataset must produce zero output entries: {manifest}"
        );
    }

    // =========================================================================
    // 19. Result response carries an `Expires` header (IMF-fixdate).
    // =========================================================================

    #[tokio::test]
    async fn test_export_result_has_expires_header() {
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

        // Poll until 303, then GET the result URL directly so we can read
        // its response headers.
        let result_url = loop {
            let poll = server
                .get(&location)
                .add_header(X_TENANT_ID, "test-tenant")
                .await;
            if poll.status_code() == StatusCode::SEE_OTHER {
                break poll
                    .headers()
                    .get("location")
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string();
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        };

        let result = server
            .get(&result_url)
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(result.status_code(), StatusCode::OK);
        let expires = result
            .headers()
            .get("expires")
            .expect("result response missing Expires header")
            .to_str()
            .unwrap();
        // IMF-fixdate per RFC 7231: e.g. "Sun, 06 Nov 1994 08:49:37 GMT".
        assert!(
            expires.ends_with(" GMT"),
            "Expires must be IMF-fixdate ending in GMT: {expires:?}"
        );
        let naive = chrono::NaiveDateTime::parse_from_str(expires, "%a, %d %b %Y %H:%M:%S GMT")
            .unwrap_or_else(|e| panic!("Expires must parse as IMF-fixdate: {expires:?} — {e}"));
        let parsed = naive.and_utc();
        // Expiration must be ~24h in the future (allow a generous window).
        let delta = parsed.signed_duration_since(chrono::Utc::now());
        assert!(
            delta.num_hours() >= 23 && delta.num_hours() <= 25,
            "Expires must be ~24h ahead, got {} hours: {expires:?}",
            delta.num_hours()
        );
    }

    // =========================================================================
    // 20. Kick-off response body is `Parameters` (spec):
    //     exportId, status="accepted", location, optional clientTrackingId.
    // =========================================================================

    #[tokio::test]
    async fn test_export_kickoff_body_is_parameters() {
        let (server, _backend) = create_test_server_with_export().await;

        let resp = server
            .post("/ViewDefinition/$viewdefinition-export?clientTrackingId=tracker-99")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&patient_view())
            .await;
        assert_eq!(resp.status_code(), StatusCode::ACCEPTED);

        let body: Value = resp.json();
        assert_eq!(
            body["resourceType"].as_str(),
            Some("Parameters"),
            "kick-off body must be Parameters, got: {body}"
        );
        let params = body["parameter"].as_array().unwrap();
        let names: Vec<&str> = params.iter().filter_map(|p| p["name"].as_str()).collect();
        for required in ["exportId", "status", "location"] {
            assert!(
                names.contains(&required),
                "kick-off body missing '{required}': {body}"
            );
        }
        // `status` must be the spec's "accepted" code.
        let status_code = params
            .iter()
            .find(|p| p["name"].as_str() == Some("status"))
            .and_then(|p| p["valueCode"].as_str());
        assert_eq!(status_code, Some("accepted"));
        // `location` must be the absolute status URL.
        let loc = params
            .iter()
            .find(|p| p["name"].as_str() == Some("location"))
            .and_then(|p| p["valueUri"].as_str())
            .expect("missing location valueUri");
        assert!(
            loc.starts_with("http://") && loc.ends_with("/status"),
            "kick-off location must be absolute status URL, got: {loc}"
        );
        // clientTrackingId echoed when supplied.
        let tracking = params
            .iter()
            .find(|p| p["name"].as_str() == Some("clientTrackingId"))
            .and_then(|p| p["valueString"].as_str());
        assert_eq!(tracking, Some("tracker-99"));
    }

    // =========================================================================
    // 21. System-level endpoint: POST /$viewdefinition-export (spec defines
    //     this operation at system, type, AND instance levels).
    // =========================================================================

    #[tokio::test]
    async fn test_export_system_level_endpoint() {
        let (server, backend) = create_test_server_with_export().await;
        seed_patients(&backend).await;

        let resp = server
            .post("/$viewdefinition-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&patient_view())
            .await;
        assert_eq!(
            resp.status_code(),
            StatusCode::ACCEPTED,
            "system-level kick-off failed: {}",
            resp.text()
        );
        // The job should be drivable end-to-end via the same status URL.
        let location = resp
            .headers()
            .get("content-location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let manifest = poll_to_manifest(&server, &location, "test-tenant").await;
        assert_eq!(manifest["resourceType"].as_str(), Some("Parameters"));
    }
}
