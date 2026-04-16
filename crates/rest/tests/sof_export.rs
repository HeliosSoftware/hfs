//! Handler-level tests for `$viewdefinition-export`.
//!
//! Tests the POST `/ViewDefinition/$viewdefinition-export`, GET/DELETE
//! `/_operations/export/{job-id}`, and GET `/_operations/export/{job-id}/{file}`
//! endpoints using an in-memory SQLite backend and InMemoryController.

#[cfg(feature = "sof")]
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

    // =========================================================================
    // 1. Submit → 202 + Content-Location
    // =========================================================================

    #[tokio::test]
    async fn test_export_submit_returns_202() {
        let (server, backend) = create_test_server_with_export().await;
        seed_patients(&backend).await;

        let resp = server
            .post("/ViewDefinition/$viewdefinition-export")
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

        // Poll up to 20 times (50ms each = 1s max)
        let mut final_status = StatusCode::ACCEPTED;
        let mut final_body: Value = json!(null);
        for _ in 0..20 {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let poll = server.get(&location).await;
            final_status = poll.status_code();
            final_body = poll.json::<Value>();
            if final_status == StatusCode::OK {
                break;
            }
        }

        assert_eq!(
            final_status,
            StatusCode::OK,
            "export did not complete within 1s: {final_body}"
        );
        assert_eq!(
            final_body["resourceType"].as_str(),
            Some("Parameters"),
            "expected Parameters manifest: {final_body}"
        );

        // Verify output file is listed in manifest
        let params = final_body["parameter"].as_array().unwrap();
        let has_output = params.iter().any(|p| p["name"].as_str() == Some("output"));
        assert!(
            has_output,
            "manifest missing 'output' parameter: {final_body}"
        );
    }

    // =========================================================================
    // 3. Cancel → 204, then poll → 404
    // =========================================================================

    #[tokio::test]
    async fn test_export_cancel() {
        let (server, _backend) = create_test_server_with_export().await;

        // Submit (no data seeded — export will complete fast, but we'll cancel immediately)
        let submit_resp = server
            .post("/ViewDefinition/$viewdefinition-export")
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
        let cancel_resp = server.delete(&location).await;
        // 204 means cancelled; 200 means it already completed — either is acceptable
        let cancel_status = cancel_resp.status_code();
        assert!(
            cancel_status == StatusCode::NO_CONTENT || cancel_status == StatusCode::OK,
            "unexpected cancel status: {cancel_status}"
        );

        // If we cancelled (204), polling should now return 404
        if cancel_status == StatusCode::NO_CONTENT {
            let poll = server.get(&location).await;
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

        // Wait for completion
        let mut manifest: Value = json!(null);
        for _ in 0..30 {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let poll = server.get(&location).await;
            if poll.status_code() == StatusCode::OK {
                manifest = poll.json::<Value>();
                break;
            }
        }

        // Extract download URL from manifest
        let params = manifest["parameter"]
            .as_array()
            .expect("expected parameter array");
        let output_param = params
            .iter()
            .find(|p| p["name"].as_str() == Some("output"))
            .expect("missing output parameter");
        let url = output_param["valueAttachment"]["url"]
            .as_str()
            .expect("missing url in attachment");

        // The URL is absolute (http://localhost/...), extract the path
        let path = url.trim_start_matches("http://localhost");

        // Download the file
        let download_resp = server.get(path).await;
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

        // Wait for completion
        let mut manifest: Value = json!(null);
        for _ in 0..40 {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let poll = server.get(&location).await;
            if poll.status_code() == StatusCode::OK {
                manifest = poll.json::<Value>();
                break;
            }
        }

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
            .filter_map(|p| p["valueAttachment"]["url"].as_str())
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

        // Poll to completion
        let mut manifest: Value = json!(null);
        for _ in 0..30 {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let poll = server.get(&location).await;
            if poll.status_code() == StatusCode::OK {
                manifest = poll.json::<Value>();
                break;
            }
        }
        assert_eq!(
            manifest["resourceType"].as_str(),
            Some("Parameters"),
            "export did not complete: {manifest}"
        );

        // Extract shard URL
        let params = manifest["parameter"]
            .as_array()
            .expect("expected parameter array");
        let output_param = params
            .iter()
            .find(|p| p["name"].as_str() == Some("output"))
            .expect("missing output parameter");
        let url = output_param["valueAttachment"]["url"]
            .as_str()
            .expect("missing url in attachment");
        let path = url.trim_start_matches("http://localhost");

        // Download the shard
        let download_resp = server.get(path).await;
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
}
