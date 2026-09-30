//! Regression tests for #1474 — sql-export completed-job download 404s after
//! an HFS restart.
//!
//! The in-memory export controller's `jobs`/`job_tenants` maps start empty on
//! every process boot (`InMemoryController::with_options`), so a job
//! completed by an earlier process becomes unreachable through the
//! status/result/download endpoints even though its output is still sitting
//! on disk. These tests drive a filesystem-backed export to completion, then
//! build a brand-new controller (and `TestServer`) over the same on-disk
//! export directory — standing in for a restarted process — and confirm it
//! can still serve that job, without widening what it will serve.
//!
//! No Docker required: this exercises `FilesystemSink` against a local
//! `tempfile::tempdir()` and the in-memory SQLite `ResourceStorage` backend,
//! same as `crates/rest/tests/sof_export.rs`.

mod sof_export_restart_tests {
    use axum::http::{HeaderName, StatusCode};
    use axum_test::TestServer;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::core::sof_runner::SofRunner;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use helios_rest::export::{FilesystemSink, InMemoryController};
    use serde_json::{Value, json};
    use std::path::Path;
    use std::sync::Arc;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const PREFER: HeaderName = HeaderName::from_static("prefer");

    /// Builds a fresh in-memory SQLite `ResourceStorage`, an `AppState` wired
    /// to a `FilesystemSink` rooted at `export_dir`, and the `TestServer` on
    /// top of it — mirroring `sof_export.rs`'s
    /// `create_test_server_with_export_config` but with a filesystem sink at
    /// a caller-chosen directory instead of the in-memory one, so a later
    /// call against the same directory can stand in for a restarted process.
    async fn build_server(export_dir: &Path) -> (TestServer, Arc<SqliteBackend>) {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);

        let runner: Arc<dyn SofRunner> = backend
            .sof_runner()
            .expect("SQLiteBackend must provide sof_runner");
        let sink = FilesystemSink::new(export_dir, "http://localhost");
        let controller = InMemoryController::new(runner, sink, None);

        let config = ServerConfig {
            base_url: "http://localhost".to_string(),
            ..ServerConfig::for_testing()
        };
        let state = helios_rest::AppState::new(Arc::clone(&backend), config)
            .with_export_controller(Arc::new(controller));
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("failed to create test server");

        (server, backend)
    }

    async fn seed_patients(backend: &SqliteBackend, tenant_id: &str) {
        let tenant = TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access());
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

    /// Polls the status URL until the job finishes and returns the manifest
    /// `Parameters` resource fetched from the result URL. Mirrors
    /// `sof_export.rs`'s helper of the same name. Times out after ~2s.
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

    /// Seeds two patients for `"test-tenant"`, then submits and polls a
    /// `$sql-export` job to completion, downloads its (sole) output file, and
    /// returns `(job_id, download_path, body_bytes)`.
    async fn run_export_to_completion(
        server: &TestServer,
        backend: &SqliteBackend,
    ) -> (String, String, Vec<u8>) {
        seed_patients(backend, "test-tenant").await;

        let submit_resp = server
            .post("/$sql-export")
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

        let manifest = poll_to_manifest(server, &location, "test-tenant").await;
        let params = manifest["parameter"]
            .as_array()
            .expect("expected parameter array");
        let job_id = params
            .iter()
            .find(|p| p["name"].as_str() == Some("exportId"))
            .and_then(|p| p["valueString"].as_str())
            .expect("missing exportId parameter")
            .to_string();
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
        let download_path = url.trim_start_matches("http://localhost").to_string();

        let download_resp = server
            .get(&download_path)
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(
            download_resp.status_code(),
            StatusCode::OK,
            "sanity download before restart failed: {}",
            download_resp.text()
        );
        let body = download_resp.as_bytes().to_vec();

        (job_id, download_path, body)
    }

    // =========================================================================
    // A completed job survives a restart: download, status, and result all
    // keep working against a brand-new controller over the same export dir.
    // =========================================================================

    #[tokio::test]
    async fn download_status_and_result_survive_a_controller_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (server, backend) = build_server(dir.path()).await;
        let (job_id, download_path, original_body) =
            run_export_to_completion(&server, &backend).await;

        // "Restart": a brand-new controller (empty `jobs`/`job_tenants`) built
        // fresh over the very same on-disk export directory, exactly like a
        // fresh process boot would build one.
        let (restarted, _restarted_backend) = build_server(dir.path()).await;

        let download_after_restart = restarted
            .get(&download_path)
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(
            download_after_restart.status_code(),
            StatusCode::OK,
            "download after restart failed: {}",
            download_after_restart.text()
        );
        assert_eq!(
            download_after_restart.as_bytes().to_vec(),
            original_body,
            "downloaded bytes must be identical after a restart"
        );

        let status_path = format!("/export/{job_id}/status");
        let status_after_restart = restarted
            .get(&status_path)
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(
            status_after_restart.status_code(),
            StatusCode::SEE_OTHER,
            "status poll after restart failed: {}",
            status_after_restart.text()
        );

        let result_path = format!("/export/{job_id}/result");
        let result_after_restart = restarted
            .get(&result_path)
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(
            result_after_restart.status_code(),
            StatusCode::OK,
            "result fetch after restart failed: {}",
            result_after_restart.text()
        );

        // Not just a 200: the rehydrated manifest must actually be *this*
        // job's, pointing at the same download URL as before the restart.
        let manifest_after_restart = result_after_restart.json::<Value>();
        let params = manifest_after_restart["parameter"]
            .as_array()
            .expect("expected parameter array");
        let rehydrated_export_id = params
            .iter()
            .find(|p| p["name"].as_str() == Some("exportId"))
            .and_then(|p| p["valueString"].as_str())
            .expect("missing exportId parameter");
        assert_eq!(
            rehydrated_export_id, job_id,
            "rehydrated manifest must report the same exportId"
        );
        let rehydrated_location = params
            .iter()
            .find(|p| p["name"].as_str() == Some("output"))
            .and_then(|p| p["part"].as_array())
            .and_then(|parts| {
                parts
                    .iter()
                    .find(|p| p["name"].as_str() == Some("location"))
            })
            .and_then(|p| p["valueUri"].as_str())
            .expect("missing output location part");
        assert_eq!(
            rehydrated_location,
            format!("http://localhost{download_path}"),
            "rehydrated manifest must point at the same download URL as before the restart"
        );
    }

    // =========================================================================
    // Regression guard: a different tenant must still get 404 after a
    // restart — rehydration must not create a cross-tenant hole.
    // =========================================================================

    #[tokio::test]
    async fn a_different_tenant_still_gets_404_after_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (server, backend) = build_server(dir.path()).await;
        let (_job_id, download_path, _body) = run_export_to_completion(&server, &backend).await;

        let (restarted, _restarted_backend) = build_server(dir.path()).await;
        let resp = restarted
            .get(&download_path)
            .add_header(X_TENANT_ID, "someone-else")
            .await;
        assert_eq!(
            resp.status_code(),
            StatusCode::NOT_FOUND,
            "a different tenant must not be able to download another tenant's export"
        );
    }

    // =========================================================================
    // Serving is scoped to exactly the filenames the job's own manifest
    // lists — an ordinary filename the job never produced still 404s.
    // =========================================================================

    #[tokio::test]
    async fn a_filename_outside_the_manifest_404s_after_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (server, backend) = build_server(dir.path()).await;
        let (job_id, _download_path, _body) = run_export_to_completion(&server, &backend).await;

        // Plant an ordinary file inside the job's own directory that the
        // manifest never listed (the job only ever wrote a single
        // `shard-0.ndjson`) — this proves serving is scoped to exactly the
        // manifest's filenames, not to whatever exists on disk under the
        // job id. `job.json` itself (this fix's own manifest file) would
        // make the same point, but a planted file keeps this test decoupled
        // from the manifest's filename.
        let job_dir = dir.path().join(&job_id);
        std::fs::write(
            job_dir.join("extra.ndjson"),
            b"{\"id\":\"not-in-manifest\"}\n",
        )
        .expect("failed to plant an extra file in the job directory");

        let other_path = format!("/export/{job_id}/extra.ndjson");

        // 404 before the restart too: the running/original controller's
        // in-memory record is just as authoritative as the rehydrated one.
        let resp_before = server
            .get(&other_path)
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(
            resp_before.status_code(),
            StatusCode::NOT_FOUND,
            "a filename absent from the job's own manifest must 404 before a restart"
        );

        let (restarted, _restarted_backend) = build_server(dir.path()).await;
        let resp_after = restarted
            .get(&other_path)
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(
            resp_after.status_code(),
            StatusCode::NOT_FOUND,
            "a filename absent from the job's own manifest must 404 after a restart"
        );
    }

    // =========================================================================
    // A job directory with output but no manifest (e.g. left over from before
    // this fix shipped) is never rehydrated and stays 404.
    // =========================================================================

    #[tokio::test]
    async fn a_job_directory_without_a_manifest_stays_404_after_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let job_id = uuid::Uuid::new_v4().to_string();
        let job_dir = dir.path().join(&job_id);
        std::fs::create_dir_all(&job_dir).expect("failed to create job dir");
        std::fs::write(job_dir.join("shard-0.ndjson"), b"{\"id\":\"p1\"}\n")
            .expect("failed to write shard");

        let (restarted, _backend) = build_server(dir.path()).await;
        let path = format!("/export/{job_id}/shard-0.ndjson");
        let resp = restarted
            .get(&path)
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(
            resp.status_code(),
            StatusCode::NOT_FOUND,
            "a job directory with no manifest must never be served, even though the shard file exists on disk"
        );
    }

    // =========================================================================
    // A corrupt `job.json` (unparsable JSON) or one written with an
    // unrecognized schema version must never stop the server from starting —
    // both are skipped, and the job they'd otherwise describe stays 404.
    // =========================================================================

    #[tokio::test]
    async fn corrupt_and_unknown_version_manifests_are_skipped_and_the_server_still_builds() {
        let dir = tempfile::tempdir().expect("tempdir");

        let corrupt_job_id = uuid::Uuid::new_v4().to_string();
        let corrupt_dir = dir.path().join(&corrupt_job_id);
        std::fs::create_dir_all(&corrupt_dir).expect("failed to create job dir");
        std::fs::write(corrupt_dir.join("job.json"), b"not json")
            .expect("failed to write corrupt job.json");

        let future_version_job_id = uuid::Uuid::new_v4().to_string();
        let future_version_dir = dir.path().join(&future_version_job_id);
        std::fs::create_dir_all(&future_version_dir).expect("failed to create job dir");
        let future_version_manifest = json!({
            "version": 999,
            "job_id": future_version_job_id,
            "tenant_id": "test-tenant",
            "format": "ndjson",
            "files": [],
            "submitted_at": "2024-01-01T00:00:00Z",
            "completed_at": "2024-01-01T00:00:01Z",
            "client_tracking_id": null
        });
        std::fs::write(
            future_version_dir.join("job.json"),
            serde_json::to_vec(&future_version_manifest).unwrap(),
        )
        .expect("failed to write future-version job.json");

        // The server must still build despite the two bad directories.
        let (restarted, _backend) = build_server(dir.path()).await;

        for job_id in [&corrupt_job_id, &future_version_job_id] {
            let status_path = format!("/export/{job_id}/status");
            let resp = restarted
                .get(&status_path)
                .add_header(X_TENANT_ID, "test-tenant")
                .await;
            assert_eq!(
                resp.status_code(),
                StatusCode::NOT_FOUND,
                "a corrupt or unknown-version job.json must never be rehydrated (job {job_id}): {}",
                resp.text()
            );
        }
    }
}
