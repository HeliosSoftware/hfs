//! Regression tests for #1801 — the S3 sink's completed sql-export jobs
//! survive an HFS restart and are reaped after `HFS_EXPORT_OUTPUT_TTL`.
//!
//! `S3Sink` used to keep the `ExportSink` defaults for `persist_completion` /
//! `load_completed`, so a job completed by an earlier process was forgotten on
//! restart: its status/result/download endpoints 404ed and the reaper never
//! saw it, leaving its shards in the bucket forever. These tests complete an
//! export against an S3-backed sink, build a brand-new controller (and
//! `TestServer`) over the same bucket — standing in for a restarted process —
//! and confirm it serves the job, keeps its original retention clock, and
//! eventually deletes it.
//!
//! Requires Docker: MinIO runs via testcontainers, the same as the other
//! Docker-backed rest suites. There is no env opt-in.

#![cfg(feature = "s3")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

mod sof_export_s3_restart_tests {
    use aws_config::{BehaviorVersion, Region};
    use aws_sdk_s3::config::Credentials;
    use aws_sdk_s3::primitives::ByteStream;
    use axum::http::{HeaderName, StatusCode};
    use axum_test::TestServer;
    use futures::StreamExt;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::core::sof_runner::SofRunner;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use helios_rest::export::{
        CleanupConfig, ExportSink, InMemoryController, JobManifest, ManifestFile, S3Sink,
    };
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::time::Duration;
    use testcontainers::core::{IntoContainerPort, WaitFor};
    use testcontainers::runners::AsyncRunner;
    use testcontainers::{GenericImage, ImageExt};
    use tokio::sync::OnceCell;
    use uuid::Uuid;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const PREFER: HeaderName = HeaderName::from_static("prefer");

    const DEFAULT_MINIO_IMAGE: &str = "ghcr.io/coollabsio/minio";
    const DEFAULT_MINIO_TAG: &str = "RELEASE.2025-10-15T17-29-55Z";
    const MINIO_ROOT_USER: &str = "minioadmin";
    const MINIO_ROOT_PASSWORD: &str = "minioadmin";

    struct SharedMinio {
        endpoint_url: String,
        /// Kept alive for the duration of the test binary; the
        /// `container_cleanup` exit hook removes it at process exit.
        _container: testcontainers::ContainerAsync<GenericImage>,
    }

    static SHARED_MINIO: OnceCell<SharedMinio> = OnceCell::const_new();

    async fn shared_minio() -> &'static SharedMinio {
        SHARED_MINIO
            .get_or_init(|| async {
                let image = std::env::var("MINIO_IMAGE")
                    .unwrap_or_else(|_| DEFAULT_MINIO_IMAGE.to_string());
                let tag =
                    std::env::var("MINIO_TAG").unwrap_or_else(|_| DEFAULT_MINIO_TAG.to_string());
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();

                // `SHARED_MINIO` is a static and never dropped; the cleanup
                // label lets the exit hook remove the container.
                let container = super::container_cleanup::with_cleanup_label(
                    GenericImage::new(image, tag)
                        .with_wait_for(WaitFor::message_on_stderr("API:"))
                        .with_exposed_port(9000.tcp())
                        .with_exposed_port(9001.tcp())
                        .with_env_var("MINIO_ROOT_USER", MINIO_ROOT_USER)
                        .with_env_var("MINIO_ROOT_PASSWORD", MINIO_ROOT_PASSWORD)
                        .with_env_var("MINIO_CONSOLE_ADDRESS", ":9001")
                        .with_cmd(["server", "/data", "--console-address", ":9001"])
                        .with_label("github.run_id", &run_id),
                )
                .start()
                .await
                .expect("failed to start MinIO container");

                let host = container
                    .get_host()
                    .await
                    .expect("failed to resolve MinIO host")
                    .to_string();
                let port = container
                    .get_host_port_ipv4(9000)
                    .await
                    .expect("failed to resolve MinIO API port");

                SharedMinio {
                    endpoint_url: format!("http://{host}:{port}"),
                    _container: container,
                }
            })
            .await
    }

    /// An S3 client aimed at the shared MinIO with path-style addressing. Uses
    /// explicit credentials, so no `AWS_*` env vars are touched.
    async fn sdk_client() -> aws_sdk_s3::Client {
        let shared = shared_minio().await;
        let creds = Credentials::new(
            MINIO_ROOT_USER,
            MINIO_ROOT_PASSWORD,
            None,
            None,
            "export-tests",
        );
        let cfg = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(shared.endpoint_url.clone())
            .credentials_provider(creds)
            .load()
            .await;
        let s3_config = aws_sdk_s3::config::Builder::from(&cfg)
            .force_path_style(true)
            .build();
        aws_sdk_s3::Client::from_conf(s3_config)
    }

    async fn fresh_bucket(client: &aws_sdk_s3::Client) -> String {
        let bucket = format!("hfs-sqlexport-{}", Uuid::new_v4().simple());
        client
            .create_bucket()
            .bucket(&bucket)
            .send()
            .await
            .expect("failed to create MinIO test bucket");
        bucket
    }

    /// An `S3Sink` over `bucket`/`prefix` with the production-default 24 h
    /// pre-signed URL lifetime.
    fn sink(client: &aws_sdk_s3::Client, bucket: &str, prefix: &str) -> S3Sink {
        S3Sink::from_client(client.clone(), bucket, prefix, 86_400)
    }

    async fn keys_under(client: &aws_sdk_s3::Client, bucket: &str, prefix: &str) -> Vec<String> {
        let mut keys = Vec::new();
        let mut token = None;
        loop {
            let resp = client
                .list_objects_v2()
                .bucket(bucket)
                .prefix(prefix)
                .set_continuation_token(token.take())
                .send()
                .await
                .expect("list_objects_v2 failed");
            keys.extend(
                resp.contents()
                    .iter()
                    .filter_map(|o| o.key().map(str::to_string)),
            );
            match resp.next_continuation_token() {
                Some(t) => token = Some(t.to_string()),
                None => break,
            }
        }
        keys
    }

    /// Builds a fresh in-memory SQLite `ResourceStorage`, an `AppState` wired
    /// to an `InMemoryController` over `sink` (with the given reaper config),
    /// and the `TestServer` on top of it. Calling it again over the same
    /// bucket stands in for a restarted process.
    async fn build_server(
        sink: S3Sink,
        cleanup: Option<CleanupConfig>,
    ) -> (TestServer, Arc<SqliteBackend>) {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);

        let runner: Arc<dyn SofRunner> = backend
            .sof_runner()
            .expect("SQLiteBackend must provide sof_runner");
        let controller = InMemoryController::with_options(runner, sink, None, None, cleanup);

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
    /// `sof_export_restart.rs`'s helper of the same name. Times out after ~10 s.
    async fn poll_to_manifest(server: &TestServer, status_url: &str, tenant: &str) -> Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
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
        panic!("export did not complete within 10 s for {status_url}");
    }

    /// Pulls the `exportId` and the (sole) output `location` out of a
    /// completion manifest.
    fn manifest_id_and_location(manifest: &Value) -> (String, String) {
        let params = manifest["parameter"]
            .as_array()
            .expect("expected parameter array");
        let job_id = params
            .iter()
            .find(|p| p["name"].as_str() == Some("exportId"))
            .and_then(|p| p["valueString"].as_str())
            .expect("missing exportId parameter")
            .to_string();
        let location = params
            .iter()
            .find(|p| p["name"].as_str() == Some("output"))
            .and_then(|p| p["part"].as_array())
            .and_then(|parts| {
                parts
                    .iter()
                    .find(|p| p["name"].as_str() == Some("location"))
            })
            .and_then(|p| p["valueUri"].as_str())
            .expect("missing output location part")
            .to_string();
        (job_id, location)
    }

    /// Seeds two patients for `"test-tenant"`, then submits and polls a
    /// `$sql-export` job to completion on `server`, and returns
    /// `(job_id, location_url, bytes)` where `bytes` is the shard fetched
    /// through the server's own download route.
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
        let status_url = submit_resp
            .headers()
            .get("content-location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let manifest = poll_to_manifest(server, &status_url, "test-tenant").await;
        let (job_id, location) = manifest_id_and_location(&manifest);
        let location_path = url::Url::parse(&location)
            .expect("output location must be an absolute URL")
            .path()
            .to_string();
        assert!(
            location_path.ends_with(&format!("/exports/{job_id}/shard-0.ndjson")),
            "unexpected output location path {location_path}"
        );

        let download = server
            .get(&format!("/export/{job_id}/shard-0.ndjson"))
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(
            download.status_code(),
            StatusCode::OK,
            "sanity download before restart failed: {}",
            download.text()
        );
        (job_id, location, download.as_bytes().to_vec())
    }

    /// A hand-built manifest for a one-shard `ndjson` job of `"test-tenant"`;
    /// `job_id` is both the storage key and `manifest.job_id`.
    fn hand_built_manifest(
        job_id: &str,
        submitted_at: chrono::DateTime<chrono::Utc>,
        completed_at: chrono::DateTime<chrono::Utc>,
    ) -> JobManifest {
        JobManifest {
            // 1 is the current manifest version (`MANIFEST_VERSION`, which is
            // crate-private).
            version: 1,
            job_id: job_id.to_string(),
            tenant_id: "test-tenant".to_string(),
            format: "ndjson".to_string(),
            files: vec![ManifestFile {
                view_name: "patients".to_string(),
                filename: "shard-0.ndjson".to_string(),
                row_count: 1,
            }],
            submitted_at,
            completed_at,
            client_tracking_id: None,
        }
    }

    /// Writes one shard and persists a hand-built manifest for `job_id`
    /// completed at `completed_at`.
    fn store_completed_job(
        s3: &S3Sink,
        job_id: &str,
        submitted_at: chrono::DateTime<chrono::Utc>,
        completed_at: chrono::DateTime<chrono::Utc>,
    ) {
        let filename = s3
            .write_shard(job_id, 0, b"{\"patient_id\":\"p1\"}\n".to_vec(), "ndjson")
            .expect("write_shard failed");
        assert_eq!(filename, "shard-0.ndjson");
        s3.persist_completion(
            job_id,
            &hand_built_manifest(job_id, submitted_at, completed_at),
        )
        .expect("persist_completion failed");
    }

    async fn status_code(server: &TestServer, job_id: &str, tenant: &str) -> StatusCode {
        server
            .get(&format!("/export/{job_id}/status"))
            .add_header(X_TENANT_ID, tenant)
            .await
            .status_code()
    }

    // =========================================================================
    // The completion manifest round-trips through persist_completion /
    // load_completed on its own, with no controller involved.
    // =========================================================================

    /// Guards the sink-level contract: `persist_completion` stores one JSON
    /// object under the job's own prefix, and a fresh sink over the same
    /// bucket and key prefix reads back an identical manifest. A sink with a
    /// different key prefix must not see it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn s3_manifest_round_trips_through_persist_and_load() {
        let client = sdk_client().await;
        let bucket = fresh_bucket(&client).await;
        let s3 = sink(&client, &bucket, "hfs/");

        let job_id = Uuid::new_v4().to_string();
        let filename = s3
            .write_shard(&job_id, 0, b"{\"patient_id\":\"p1\"}\n".to_vec(), "ndjson")
            .expect("write_shard failed");
        assert_eq!(filename, "shard-0.ndjson");

        let now = chrono::Utc::now();
        let manifest = JobManifest {
            // 1 is the current manifest version (`MANIFEST_VERSION`, which is
            // crate-private).
            version: 1,
            job_id: job_id.clone(),
            tenant_id: "t1".to_string(),
            format: "ndjson".to_string(),
            files: vec![ManifestFile {
                view_name: "patients".to_string(),
                filename,
                row_count: 2,
            }],
            submitted_at: now - chrono::Duration::minutes(10),
            completed_at: now - chrono::Duration::minutes(5),
            client_tracking_id: Some("trk-1".to_string()),
        };
        s3.persist_completion(&job_id, &manifest)
            .expect("persist_completion failed");

        let head = client
            .head_object()
            .bucket(&bucket)
            .key(format!("hfs/exports/{job_id}/job.json"))
            .send()
            .await
            .expect("job.json must exist under the job prefix after persist_completion");
        assert_eq!(head.content_type(), Some("application/json"));

        let reloaded = sink(&client, &bucket, "hfs/").load_completed();
        assert_eq!(reloaded.len(), 1, "expected exactly the one persisted job");
        assert_eq!(reloaded[0].0, job_id);
        assert_eq!(
            serde_json::to_value(&reloaded[0].1).unwrap(),
            serde_json::to_value(&manifest).unwrap(),
            "the reloaded manifest must equal the persisted one"
        );

        assert!(
            sink(&client, &bucket, "").load_completed().is_empty(),
            "a sink with a different key prefix must not see this job"
        );
    }

    /// Guards the paging loop: with more than one listing page (1000 keys) of
    /// unrelated objects sorting ahead of the job, `load_completed` must still
    /// find the job, and the manifest-less prefix is ignored.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn s3_load_completed_pages_past_the_first_listing_page() {
        let client = sdk_client().await;
        let bucket = fresh_bucket(&client).await;

        // The nil UUID string sorts before any v4 UUID.
        let filler_prefix = "exports/00000000-0000-0000-0000-000000000000/";
        futures::stream::iter(0..1005)
            .map(|i| {
                let client = client.clone();
                let bucket = bucket.clone();
                async move {
                    client
                        .put_object()
                        .bucket(bucket)
                        .key(format!("{filler_prefix}filler-{i:04}"))
                        .body(ByteStream::from_static(b"x"))
                        .send()
                        .await
                        .expect("put_object failed for a filler object");
                }
            })
            .buffer_unordered(32)
            .collect::<Vec<_>>()
            .await;

        let job_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        store_completed_job(
            &sink(&client, &bucket, ""),
            &job_id,
            now - chrono::Duration::minutes(2),
            now - chrono::Duration::minutes(1),
        );

        // Precondition: the manifest is NOT on the first listing page.
        let first_page = client
            .list_objects_v2()
            .bucket(&bucket)
            .prefix("exports/")
            .send()
            .await
            .expect("list_objects_v2 failed");
        let manifest_key = format!("exports/{job_id}/job.json");
        assert!(
            !first_page
                .contents()
                .iter()
                .any(|o| o.key() == Some(manifest_key.as_str())),
            "the manifest must sort past the first listing page"
        );
        assert_eq!(first_page.is_truncated(), Some(true));

        let loaded = sink(&client, &bucket, "").load_completed();
        assert_eq!(
            loaded.len(),
            1,
            "expected exactly the one job past the first page"
        );
        assert_eq!(loaded[0].0, job_id);
    }

    // =========================================================================
    // A completed job survives a restart, keeps serving, then is reaped.
    // =========================================================================

    /// Guards the headline fix: after a restart over the same bucket the job's
    /// status, result and pre-signed download all still work (with the URL
    /// capped by the job's remaining retention, not the sink's 24 h), another
    /// tenant still gets 404, and the new process's reaper deletes the job and
    /// its objects once `output_ttl` elapses.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn completed_s3_job_is_served_after_a_restart_and_reaped_after_its_ttl() {
        let client = sdk_client().await;
        let bucket = fresh_bucket(&client).await;

        let (server1, backend1) = build_server(sink(&client, &bucket, ""), None).await;
        let (job_id, _location, original_bytes) =
            run_export_to_completion(&server1, &backend1).await;

        let manifest_key = format!("exports/{job_id}/job.json");
        assert!(
            keys_under(&client, &bucket, &format!("exports/{job_id}/"))
                .await
                .contains(&manifest_key),
            "the completion manifest must be stored at {manifest_key} before the restart"
        );

        // "Restart": a brand-new controller built over the very same bucket.
        let (server2, _backend2) = build_server(
            sink(&client, &bucket, ""),
            Some(CleanupConfig {
                output_ttl: Duration::from_secs(20),
                interval: Duration::from_millis(200),
            }),
        )
        .await;

        assert_eq!(
            status_code(&server2, &job_id, "test-tenant").await,
            StatusCode::SEE_OTHER,
            "status poll after restart must report the job completed"
        );

        let result = server2
            .get(&format!("/export/{job_id}/result"))
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(
            result.status_code(),
            StatusCode::OK,
            "result fetch after restart failed: {}",
            result.text()
        );
        let (rehydrated_id, location) = manifest_id_and_location(&result.json::<Value>());
        assert_eq!(rehydrated_id, job_id);

        let location_url = url::Url::parse(&location).expect("output location must be a URL");
        assert!(
            location_url
                .path()
                .ends_with(&format!("/exports/{job_id}/shard-0.ndjson")),
            "unexpected output location {location}"
        );
        let expires: u64 = location_url
            .query_pairs()
            .find(|(k, _)| k.eq_ignore_ascii_case("X-Amz-Expires"))
            .and_then(|(_, v)| v.parse().ok())
            .expect("pre-signed URL must carry X-Amz-Expires");
        // The controller floors the lifetime at 60 s, so a 20 s retention gives
        // exactly the floor. This shows the URL is capped by retention rather
        // than the sink's 86400 s; it does not by itself prove `terminal_at`
        // came from the manifest (a restart-time clock caps the same way). The
        // `rehydrated_s3_job_url_is_capped_from_the_original_completion` test
        // pins that.
        assert!(
            expires <= 60,
            "URL must be capped by retention (60 s floor), not the sink's 86400 s; got \
             {expires} s"
        );

        // The URL was pre-signed fresh by the new process and actually works.
        let fetched = reqwest::get(location.as_str())
            .await
            .expect("fetching the pre-signed URL failed");
        assert_eq!(fetched.status(), reqwest::StatusCode::OK);
        assert_eq!(
            fetched.bytes().await.unwrap().to_vec(),
            original_bytes,
            "pre-signed download must return the pre-restart bytes"
        );

        let download = server2
            .get(&format!("/export/{job_id}/shard-0.ndjson"))
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(download.status_code(), StatusCode::OK);
        assert_eq!(download.as_bytes().to_vec(), original_bytes);

        assert_eq!(
            status_code(&server2, &job_id, "someone-else").await,
            StatusCode::NOT_FOUND,
            "a different tenant must not see another tenant's rehydrated job"
        );

        // The restarted process's background reaper must now expire the job.
        let mut reaped = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        while tokio::time::Instant::now() < deadline {
            if status_code(&server2, &job_id, "test-tenant").await == StatusCode::NOT_FOUND {
                reaped = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        assert!(
            reaped,
            "the job must be reaped within 60 s of a 20 s output TTL"
        );
        // The reaper drops the tenant entry (so polls 404) before the sink's
        // S3 deletes finish, so the bucket can briefly lag the 404 on a slow
        // machine; poll for it rather than checking once.
        let mut emptied = false;
        while tokio::time::Instant::now() < deadline {
            if keys_under(&client, &bucket, &format!("exports/{job_id}/"))
                .await
                .is_empty()
            {
                emptied = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        assert!(
            emptied,
            "reaping must delete the shards and job.json from the bucket"
        );
    }

    // =========================================================================
    // Retention is counted from the original completion, not the restart.
    // =========================================================================

    /// Guards `terminal_at`: a job already older than `output_ttl` when the new
    /// process starts is reaped by the startup sweep. This only passes if the
    /// rehydrated job's clock is the manifest's original `completed_at`, not
    /// the restart time.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rehydrated_s3_job_past_its_ttl_is_reaped_at_startup() {
        let client = sdk_client().await;
        let bucket = fresh_bucket(&client).await;

        let (server1, backend1) = build_server(sink(&client, &bucket, ""), None).await;
        let (job_id, _location, _bytes) = run_export_to_completion(&server1, &backend1).await;

        tokio::time::sleep(Duration::from_millis(1500)).await;

        let (server2, _backend2) = build_server(
            sink(&client, &bucket, ""),
            Some(CleanupConfig {
                output_ttl: Duration::from_secs(1),
                interval: Duration::from_secs(3600),
            }),
        )
        .await;

        assert_eq!(
            status_code(&server2, &job_id, "test-tenant").await,
            StatusCode::NOT_FOUND,
            "a job older than the TTL must be reaped by the startup sweep"
        );
        assert!(
            keys_under(&client, &bucket, &format!("exports/{job_id}/"))
                .await
                .is_empty(),
            "the startup sweep must delete the expired job's objects"
        );
    }

    /// Guards that the URL cap counts from the manifest's `completed_at`: a
    /// job completed 60 minutes ago with a 2 h retention has about 3600 s left.
    /// A restart-time clock would give about 7200 s and an uncapped URL 86400 s.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rehydrated_s3_job_url_is_capped_from_the_original_completion() {
        let client = sdk_client().await;
        let bucket = fresh_bucket(&client).await;

        let job_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        store_completed_job(
            &sink(&client, &bucket, ""),
            &job_id,
            now - chrono::Duration::minutes(61),
            now - chrono::Duration::minutes(60),
        );

        let (server, _backend) = build_server(
            sink(&client, &bucket, ""),
            Some(CleanupConfig {
                output_ttl: Duration::from_secs(2 * 3600),
                interval: Duration::from_secs(3600),
            }),
        )
        .await;

        let result = server
            .get(&format!("/export/{job_id}/result"))
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(
            result.status_code(),
            StatusCode::OK,
            "result fetch for the rehydrated job failed: {}",
            result.text()
        );
        let (_, location) = manifest_id_and_location(&result.json::<Value>());
        let expires: u64 = url::Url::parse(&location)
            .expect("output location must be a URL")
            .query_pairs()
            .find(|(k, _)| k.eq_ignore_ascii_case("X-Amz-Expires"))
            .and_then(|(_, v)| v.parse().ok())
            .expect("pre-signed URL must carry X-Amz-Expires");
        assert!(
            (3000..=3600).contains(&expires),
            "URL lifetime must be the ~3600 s left of the 2 h retention counted from the \
             original completion, got {expires} s"
        );
    }

    // =========================================================================
    // Anything without a valid manifest is ignored, never served or deleted.
    // =========================================================================

    /// Guards the skip rules: a job prefix with shards but no manifest (in
    /// flight at a crash), an unparsable `job.json`, and an unknown-version
    /// `job.json` are all ignored — the server still builds, none of the jobs
    /// is served, and loading never deletes anything. The corrupt job sorts
    /// first and the future-version job last, with one valid job between them,
    /// so a skip that stopped the load early instead of moving on would lose
    /// the valid job.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn s3_job_without_a_manifest_is_ignored_after_restart() {
        let client = sdk_client().await;
        let bucket = fresh_bucket(&client).await;

        let in_flight = Uuid::new_v4().to_string();
        // Force the listing order: corrupt < valid < future_version.
        let with_first_char = |c: char| format!("{c}{}", &Uuid::new_v4().to_string()[1..]);
        let corrupt = with_first_char('0');
        let valid = with_first_char('8');
        let future_version = with_first_char('f');

        let put = |key: String, body: Vec<u8>| {
            let client = client.clone();
            let bucket = bucket.clone();
            async move {
                client
                    .put_object()
                    .bucket(bucket)
                    .key(key)
                    .body(ByteStream::from(body))
                    .send()
                    .await
                    .expect("put_object failed");
            }
        };

        let orphan_key = format!("exports/{in_flight}/shard-0.ndjson");
        put(orphan_key.clone(), b"{\"id\":\"p1\"}\n".to_vec()).await;
        put(format!("exports/{corrupt}/job.json"), b"not json".to_vec()).await;
        let future_manifest = json!({
            "version": 999,
            "job_id": future_version,
            "tenant_id": "test-tenant",
            "format": "ndjson",
            "files": [],
            "submitted_at": "2024-01-01T00:00:00Z",
            "completed_at": "2024-01-01T00:00:01Z",
            "client_tracking_id": null
        });
        put(
            format!("exports/{future_version}/job.json"),
            serde_json::to_vec(&future_manifest).unwrap(),
        )
        .await;

        store_completed_job(
            &sink(&client, &bucket, ""),
            &valid,
            chrono::Utc::now() - chrono::Duration::seconds(1),
            chrono::Utc::now(),
        );

        let loaded = sink(&client, &bucket, "").load_completed();
        assert_eq!(
            loaded.len(),
            1,
            "only the valid job has a usable manifest, got {:?}",
            loaded.iter().map(|(id, _)| id).collect::<Vec<_>>()
        );
        assert_eq!(loaded[0].0, valid);

        // The server must still build despite the bad entries.
        let (server, _backend) = build_server(sink(&client, &bucket, ""), None).await;

        assert_eq!(
            status_code(&server, &valid, "test-tenant").await,
            StatusCode::SEE_OTHER,
            "the valid job between the bad ones must still be rehydrated"
        );
        for job_id in [&in_flight, &corrupt, &future_version] {
            assert_eq!(
                status_code(&server, job_id, "test-tenant").await,
                StatusCode::NOT_FOUND,
                "a job without a valid manifest must never be rehydrated (job {job_id})"
            );
        }
        let resp = server
            .get(&format!("/export/{in_flight}/shard-0.ndjson"))
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(
            resp.status_code(),
            StatusCode::NOT_FOUND,
            "a shard with no manifest must not be served even though the object exists"
        );

        assert!(
            keys_under(&client, &bucket, "exports/")
                .await
                .contains(&orphan_key),
            "loading must never delete an in-flight job's objects"
        );
    }
}
