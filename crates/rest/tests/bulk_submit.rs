//! Integration tests for the FHIR Bulk Data Submit (`$bulk-submit`) endpoints.
//!
//! Exercises the kick-off → status-kickoff → poll → ingest → delete lifecycle
//! through the real Axum router, plus the validation SHALLs and the
//! poll/cancel → 404 contract.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use axum_test::TestServer;
use helios_persistence::backends::local_fs::LocalFsOutputStore;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{
    BulkSubmitJobStore, DefaultSubmitWorker, ExportOutputStore, RemoteFile, RemoteManifest,
    ResourceStorage, SubmitClaimStrategy, SubmitInputFetcher, WorkerId,
};
use helios_persistence::error::StorageResult;
use helios_rest::ServerConfig;
use helios_rest::bulk_export_auth::BearerScopeAuth;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

/// A fetcher that serves a fixed manifest + NDJSON from memory.
struct MockFetcher {
    files: std::collections::HashMap<String, Vec<u8>>,
    manifest: RemoteManifest,
}

#[async_trait]
impl SubmitInputFetcher for MockFetcher {
    async fn fetch_manifest(
        &self,
        _url: &str,
        _headers: &[(String, String)],
        _oauth: &[String],
    ) -> StorageResult<RemoteManifest> {
        Ok(RemoteManifest {
            requires_access_token: self.manifest.requires_access_token,
            output: self.manifest.output.clone(),
            deleted: self.manifest.deleted.clone(),
        })
    }

    async fn open_file_stream(
        &self,
        url: &str,
        _headers: &[(String, String)],
        _requires_access_token: bool,
        _oauth: &[String],
        _encryption_key: Option<&Value>,
    ) -> StorageResult<Box<dyn tokio::io::AsyncBufRead + Send + Unpin>> {
        let data = self.files.get(url).cloned().unwrap_or_default();
        Ok(Box::new(tokio::io::BufReader::new(std::io::Cursor::new(
            data,
        ))))
    }
}

fn mock_fetcher() -> Arc<MockFetcher> {
    let ndjson = concat!(
        "{\"resourceType\":\"Patient\",\"id\":\"sub-p1\",\"name\":[{\"family\":\"A\"}]}\n",
        "{\"resourceType\":\"Patient\",\"id\":\"sub-p2\",\"name\":[{\"family\":\"B\"}]}\n"
    );
    let mut files = std::collections::HashMap::new();
    files.insert(
        "https://provider/patients.ndjson".to_string(),
        ndjson.as_bytes().to_vec(),
    );
    Arc::new(MockFetcher {
        files,
        manifest: RemoteManifest {
            requires_access_token: false,
            output: vec![RemoteFile {
                resource_type: Some("Patient".to_string()),
                url: "https://provider/patients.ndjson".to_string(),
                count: Some(2),
            }],
            deleted: vec![],
        },
    })
}

async fn create_submit_server() -> (
    TestServer,
    Arc<SqliteBackend>,
    Arc<MockFetcher>,
    Arc<LocalFsOutputStore>,
    tempfile::TempDir,
) {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));
    let backend = Arc::new(
        SqliteBackend::with_config(
            ":memory:",
            SqliteBackendConfig {
                data_dir: Some(data_dir),
                ..Default::default()
            },
        )
        .expect("create SQLite backend"),
    );
    backend.init_schema().expect("init schema");

    let tmp = tempfile::tempdir().expect("tempdir");
    let output = Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080"));
    let fetcher = mock_fetcher();

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    };

    let state = helios_rest::AppState::new(Arc::clone(&backend), config).with_bulk_submit(
        backend.clone() as Arc<dyn BulkSubmitJobStore>,
        fetcher.clone() as Arc<dyn SubmitInputFetcher>,
        output.clone() as Arc<dyn ExportOutputStore>,
        Arc::new(BearerScopeAuth),
    );
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let server = TestServer::new(app).expect("create test server");
    (server, backend, fetcher, output, tmp)
}

async fn drain_submit(
    backend: &Arc<SqliteBackend>,
    fetcher: &Arc<MockFetcher>,
    output: &Arc<LocalFsOutputStore>,
) {
    let worker_id = WorkerId::new("test-submit-worker");
    let worker = DefaultSubmitWorker::new(
        backend.clone(),
        fetcher.clone(),
        output.clone(),
        worker_id.clone(),
    );
    while let Some(lease) = backend
        .claim_next_manifest(&worker_id, Duration::from_secs(60))
        .await
        .expect("claim_next_manifest")
    {
        worker.run_job(lease).await.expect("run_job");
    }
}

fn kickoff_body() -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "submitter", "valueIdentifier": {"system": "http://ehr", "value": "ehr-1"}},
            {"name": "submissionId", "valueString": "it-1"},
            {"name": "manifestUrl", "valueUrl": "https://provider/manifest.json"},
            {"name": "fhirBaseUrl", "valueUrl": "https://provider/fhir"}
        ]
    })
}

fn status_body() -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "submitter", "valueIdentifier": {"system": "http://ehr", "value": "ehr-1"}},
            {"name": "submissionId", "valueString": "it-1"}
        ]
    })
}

#[tokio::test]
async fn test_kickoff_returns_200() {
    let (server, ..) = create_submit_server().await;
    let resp = server.post("/$bulk-submit").json(&kickoff_body()).await;
    assert_eq!(resp.status_code(), StatusCode::OK);
}

#[tokio::test]
async fn test_kickoff_validation_shalls() {
    let (server, ..) = create_submit_server().await;

    // Missing submitter → 400.
    let body = json!({"resourceType": "Parameters", "parameter": [
        {"name": "submissionId", "valueString": "x"},
        {"name": "manifestUrl", "valueUrl": "https://p/m"},
        {"name": "fhirBaseUrl", "valueUrl": "https://p/fhir"}
    ]});
    assert_eq!(
        server.post("/$bulk-submit").json(&body).await.status_code(),
        StatusCode::BAD_REQUEST
    );

    // manifestUrl without fhirBaseUrl → 400.
    let body = json!({"resourceType": "Parameters", "parameter": [
        {"name": "submitter", "valueIdentifier": {"value": "e"}},
        {"name": "submissionId", "valueString": "x"},
        {"name": "manifestUrl", "valueUrl": "https://p/m"}
    ]});
    assert_eq!(
        server.post("/$bulk-submit").json(&body).await.status_code(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn test_strict_handling_rejects_unknown_directives() {
    let (server, ..) = create_submit_server().await;
    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "submitter", "valueIdentifier": {"value": "e"}},
            {"name": "submissionId", "valueString": "strict-1"},
            {"name": "submissionStatus", "valueCoding": {"code": "in-progress"}},
            {"name": "import", "part": [
                {"name": "parameterUrl", "valueUri": "https://unknown/directive"},
                {"name": "parameterValue", "valueString": "x"}
            ]}
        ]
    });
    let resp = server
        .post("/$bulk-submit")
        .add_header("Prefer", "handling=strict")
        .json(&body)
        .await;
    assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_strict_handling_accepts_recognized_import_mode() {
    // Only *unrecognized* directives are rejected under strict handling — the
    // import mode HFS publishes is honored regardless of posture.
    let (server, ..) = create_submit_server().await;
    let mut body = kickoff_body();
    body["parameter"].as_array_mut().unwrap().push(json!({
        "name": "import",
        "part": [
            {"name": "parameterUrl", "valueUri": "https://helios.software/import-mode"},
            {"name": "parameterValue", "valueString": "merge"}
        ]
    }));
    let resp = server
        .post("/$bulk-submit")
        .add_header("Prefer", "handling=strict")
        .json(&body)
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
}

#[tokio::test]
async fn test_unsupported_import_mode_value_rejected() {
    // A directive we recognize with a value we cannot honor is a 400 even under
    // lenient handling — silently ingesting under the wrong mode would corrupt data.
    let (server, ..) = create_submit_server().await;
    let mut body = kickoff_body();
    body["parameter"].as_array_mut().unwrap().push(json!({
        "name": "import",
        "part": [
            {"name": "parameterUrl", "valueUri": "https://helios.software/import-mode"},
            {"name": "parameterValue", "valueString": "upsert"}
        ]
    }));
    let resp = server.post("/$bulk-submit").json(&body).await;
    assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
}

/// Kicks off a submission over a pre-existing `sub-p1` and returns its stored
/// content after ingestion, so `replace` and `merge` can be compared end-to-end.
async fn ingest_with_import_mode(mode: Option<&str>) -> Value {
    let (server, backend, fetcher, output, _tmp) = create_submit_server().await;
    let tenant = helios_persistence::tenant::TenantContext::new(
        helios_persistence::tenant::TenantId::new("test-tenant"),
        helios_persistence::tenant::TenantPermissions::full_access(),
    );
    // Already stored: a Patient with a `gender` the submission never mentions.
    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "sub-p1",
                "gender": "female",
                "name": [{"family": "Stale"}]
            }),
            helios_fhir::FhirVersion::default_enabled(),
        )
        .await
        .expect("seed patient");

    let mut body = kickoff_body();
    if let Some(mode) = mode {
        body["parameter"].as_array_mut().unwrap().push(json!({
            "name": "import",
            "part": [
                {"name": "parameterUrl", "valueUri": "https://helios.software/import-mode"},
                {"name": "parameterValue", "valueString": mode}
            ]
        }));
    }
    assert_eq!(
        server.post("/$bulk-submit").json(&body).await.status_code(),
        StatusCode::OK
    );
    drain_submit(&backend, &fetcher, &output).await;

    backend
        .read(&tenant, "Patient", "sub-p1")
        .await
        .unwrap()
        .expect("patient still stored")
        .content()
        .clone()
}

#[tokio::test]
async fn test_import_mode_merge_applied_to_ingestion() {
    let stored = ingest_with_import_mode(Some("merge")).await;
    // Submitted element wins...
    assert_eq!(stored["name"], json!([{"family": "A"}]));
    // ...and the element the submission omitted survives.
    assert_eq!(stored["gender"], json!("female"));
}

#[tokio::test]
async fn test_import_mode_replace_applied_to_ingestion() {
    for mode in [Some("replace"), None] {
        let stored = ingest_with_import_mode(mode).await;
        assert_eq!(stored["name"], json!([{"family": "A"}]));
        assert!(
            stored.get("gender").is_none(),
            "replace must not retain unsubmitted elements, got {stored}"
        );
    }
}

#[tokio::test]
async fn test_full_lifecycle_ingests_and_polls() {
    let (server, backend, fetcher, output, _tmp) = create_submit_server().await;

    // Kick off.
    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&kickoff_body())
            .await
            .status_code(),
        StatusCode::OK
    );

    // Status kickoff → 202 + Content-Location.
    let status = server
        .post("/$bulk-submit-status")
        .json(&status_body())
        .await;
    assert_eq!(status.status_code(), StatusCode::ACCEPTED);
    let loc = status
        .headers()
        .get("content-location")
        .expect("Content-Location")
        .to_str()
        .unwrap()
        .to_string();
    let poll_path = loc.trim_start_matches("http://localhost:8080");
    assert!(poll_path.starts_with("/bulk-submit-status/"));

    // Before the worker runs: in-progress → 202.
    assert_eq!(
        server.get(poll_path).await.status_code(),
        StatusCode::ACCEPTED
    );

    // Run the worker, then poll → 200 + status manifest.
    drain_submit(&backend, &fetcher, &output).await;
    let done = server.get(poll_path).await;
    assert_eq!(done.status_code(), StatusCode::OK);
    let manifest: Value = done.json();
    assert_eq!(manifest["submissionId"], "it-1");
    assert!(manifest["output"].is_array());
    // Spec (build.fhir.org submit.html): the OperationOutcome array is `outcome`,
    // output entries carry `fileSize`, and `link` is always present.
    assert!(manifest["outcome"].is_array());
    assert!(manifest["link"].is_array());
    assert!(
        manifest["output"][0]["fileSize"].is_number(),
        "output entry must carry fileSize, got {}",
        manifest["output"][0]
    );
    // Every local-fs artifact is token-gated, so requiresAccessToken aggregates true.
    assert_eq!(manifest["requiresAccessToken"], true);
    // HFS-served (local-fs) artifacts MUST be advertised on the submit-file
    // surface, not the export-file surface.
    let out_url = manifest["output"][0]["url"].as_str().unwrap();
    assert!(
        out_url.contains("/bulk-submit-file/"),
        "artifact URL must use /bulk-submit-file/, got {out_url}"
    );
    assert!(!out_url.contains("/export-file/"));
    // And the advertised artifact is downloadable through that surface.
    let file_path = out_url.trim_start_matches("http://localhost:8080");
    assert_eq!(server.get(file_path).await.status_code(), StatusCode::OK);

    // The resources were ingested.
    let tenant = helios_persistence::tenant::TenantContext::new(
        helios_persistence::tenant::TenantId::new("test-tenant"),
        helios_persistence::tenant::TenantPermissions::full_access(),
    );
    assert!(
        backend
            .read(&tenant, "Patient", "sub-p1")
            .await
            .unwrap()
            .is_some()
    );

    // Cancel → 202, then poll → 404.
    assert_eq!(
        server.delete(poll_path).await.status_code(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        server.get(poll_path).await.status_code(),
        StatusCode::NOT_FOUND
    );
}
