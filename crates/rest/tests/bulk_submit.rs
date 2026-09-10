//! Integration tests for the FHIR Bulk Data Submit (`$bulk-submit`) endpoints.
//!
//! Exercises the kick-off → status-kickoff → poll → ingest → delete lifecycle
//! through the real Axum router, plus the validation SHALLs and the
//! poll/cancel → 404 contract.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use axum_test::TestServer;
use helios_persistence::backends::local_fs::LocalFsOutputStore;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{
    BulkSubmitJobStore, BulkSubmitProvider, DefaultSubmitWorker, ExportOutputStore, ExportPartKey,
    ManifestPhase, ManifestPublicationStatus, RemoteFile, RemoteManifest, ResourceStorage,
    SubmissionId, SubmitClaimStrategy, SubmitFileRecord, SubmitInputFetcher, SubmitWorkerStorage,
    WorkerId, submission_output_job_id,
};
use helios_persistence::error::StorageResult;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use helios_rest::bulk_export_auth::BearerScopeAuth;
use helios_rest::config::{BulkSubmitConfig, MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

/// A fetcher that serves a fixed manifest + NDJSON from memory.
struct MockFetcher {
    files: std::collections::HashMap<String, Vec<u8>>,
    manifest: RemoteManifest,
    manifests: std::collections::HashMap<String, RemoteManifest>,
}

#[async_trait]
impl SubmitInputFetcher for MockFetcher {
    async fn fetch_manifest(
        &self,
        _url: &str,
        _headers: &[(String, String)],
        _oauth: &[String],
        _encryption_key: Option<&Value>,
    ) -> StorageResult<RemoteManifest> {
        if let Some(manifest) = self.manifests.get(_url) {
            return Ok(RemoteManifest {
                requires_access_token: manifest.requires_access_token,
                output: manifest.output.clone(),
                deleted: manifest.deleted.clone(),
            });
        }

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
    ) -> StorageResult<(Box<dyn tokio::io::AsyncBufRead + Send + Unpin>, Option<u64>)> {
        let data = self.files.get(url).cloned().unwrap_or_default();
        let len = data.len() as u64;
        Ok((
            Box::new(tokio::io::BufReader::new(std::io::Cursor::new(data))),
            Some(len),
        ))
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
        manifests: std::collections::HashMap::new(),
    })
}

/// A fetcher whose manifest yields three status artifacts — one `output` receipt,
/// one aggregated `outcome` (the second line's type does not match the declared
/// type), and one `deleted` receipt — so the status manifest has enough entries to
/// paginate across all three arrays.
fn multi_artifact_fetcher() -> Arc<MockFetcher> {
    let mut files = std::collections::HashMap::new();
    files.insert(
        "https://provider/patients.ndjson".to_string(),
        concat!(
            "{\"resourceType\":\"Patient\",\"id\":\"pg-p1\"}\n",
            "{\"resourceType\":\"Observation\",\"id\":\"pg-o1\",\"status\":\"final\"}\n",
        )
        .as_bytes()
        .to_vec(),
    );
    files.insert(
        "https://provider/deleted.ndjson".to_string(),
        b"{\"resourceType\":\"Patient\",\"id\":\"pg-p1\"}\n".to_vec(),
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
            deleted: vec![RemoteFile {
                resource_type: None,
                url: "https://provider/deleted.ndjson".to_string(),
                count: Some(1),
            }],
        },
        manifests: std::collections::HashMap::new(),
    })
}

/// A fetcher whose output receipt declares `OperationOutcome` but whose second
/// NDJSON line is another resource type, producing one valid output artifact and
/// one aggregated error artifact with the same resource type but different
/// protocol kind.
fn operation_outcome_fetcher() -> Arc<MockFetcher> {
    let mut files = std::collections::HashMap::new();
    files.insert(
        "https://provider/operation-outcome.ndjson".to_string(),
        concat!(
            "{\"resourceType\":\"OperationOutcome\",\"id\":\"oo-out-1\"}\n",
            "{\"resourceType\":\"Patient\",\"id\":\"oo-wrong-type\"}\n",
        )
        .as_bytes()
        .to_vec(),
    );
    Arc::new(MockFetcher {
        files,
        manifest: RemoteManifest {
            requires_access_token: false,
            output: vec![RemoteFile {
                resource_type: Some("OperationOutcome".to_string()),
                url: "https://provider/operation-outcome.ndjson".to_string(),
                count: Some(1),
            }],
            deleted: vec![],
        },
        manifests: std::collections::HashMap::new(),
    })
}

/// Serves distinct Patient manifests for the same submitter/submission ID.
fn two_manifest_fetcher() -> Arc<MockFetcher> {
    let mut files = std::collections::HashMap::new();
    files.insert(
        "https://provider/mm-a.ndjson".to_string(),
        b"{\"resourceType\":\"Patient\",\"id\":\"mm-a-1\"}\n".to_vec(),
    );
    files.insert(
        "https://provider/mm-b.ndjson".to_string(),
        b"{\"resourceType\":\"Patient\",\"id\":\"mm-b-1\"}\n".to_vec(),
    );

    let mut manifests = std::collections::HashMap::new();
    for (manifest_name, patient_id) in [("manifest-a", "mm-a"), ("manifest-b", "mm-b")] {
        manifests.insert(
            format!("https://provider/{manifest_name}.json"),
            RemoteManifest {
                requires_access_token: false,
                output: vec![RemoteFile {
                    resource_type: Some("Patient".to_string()),
                    url: format!("https://provider/{patient_id}.ndjson"),
                    count: Some(1),
                }],
                deleted: vec![],
            },
        );
    }

    Arc::new(MockFetcher {
        files,
        manifest: manifests["https://provider/manifest-a.json"].clone(),
        manifests,
    })
}

async fn create_file_submit_server(
    bulk_submit: BulkSubmitConfig,
) -> (
    TestServer,
    Arc<SqliteBackend>,
    Arc<LocalFsOutputStore>,
    PathBuf,
    tempfile::TempDir,
) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("legacy.db");
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));
    let backend = Arc::new(
        SqliteBackend::with_config(
            &db_path,
            SqliteBackendConfig {
                data_dir: Some(data_dir),
                ..Default::default()
            },
        )
        .expect("create file-backed SQLite backend"),
    );
    backend.init_schema().expect("init schema");

    let output_dir = tmp.path().join("output");
    std::fs::create_dir_all(&output_dir).expect("create legacy output dir");
    let output = Arc::new(LocalFsOutputStore::new(
        &output_dir,
        "https://wrong-internal.example",
    ));

    let config = ServerConfig {
        bulk_submit,
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::clone(&backend), config).with_bulk_submit(
        backend.clone() as Arc<dyn BulkSubmitJobStore>,
        mock_fetcher() as Arc<dyn SubmitInputFetcher>,
        output.clone() as Arc<dyn ExportOutputStore>,
        Arc::new(BearerScopeAuth),
    );
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let server = TestServer::new(app).expect("create test server");
    (server, backend, output, db_path, tmp)
}

async fn create_submit_server() -> (
    TestServer,
    Arc<SqliteBackend>,
    Arc<MockFetcher>,
    Arc<LocalFsOutputStore>,
    tempfile::TempDir,
) {
    create_submit_server_with(mock_fetcher(), BulkSubmitConfig::default()).await
}

async fn create_submit_server_with(
    fetcher: Arc<MockFetcher>,
    bulk_submit: BulkSubmitConfig,
) -> (
    TestServer,
    Arc<SqliteBackend>,
    Arc<MockFetcher>,
    Arc<LocalFsOutputStore>,
    tempfile::TempDir,
) {
    create_submit_server_with_routing(
        fetcher,
        bulk_submit,
        TenantRoutingMode::HeaderOnly,
        "http://localhost:8080",
        "test-tenant",
    )
    .await
}

async fn create_submit_server_with_routing(
    fetcher: Arc<MockFetcher>,
    bulk_submit: BulkSubmitConfig,
    routing_mode: TenantRoutingMode,
    base_url: &str,
    default_tenant: &str,
) -> (
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
    let output = Arc::new(LocalFsOutputStore::new(
        tmp.path(),
        "https://wrong-internal.example",
    ));

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode,
            ..Default::default()
        },
        base_url: base_url.to_string(),
        default_tenant: default_tenant.to_string(),
        bulk_submit,
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

const LEGACY_OUTPUT_BYTES: &str = "{\"reference\":\"OperationOutcome/legacy-1\"}\n";
const LEGACY_DELETED_BYTES: &str = "{\"reference\":\"Bundle/legacy-deleted\"}\n";
const LEGACY_ERROR_BYTES: &str = "{\"legacy\":\"collides\"}\n";

#[derive(Debug)]
struct LegacyArtifact {
    file_type: &'static str,
    resource_type: &'static str,
    body: &'static str,
}

impl LegacyArtifact {
    fn new(file_type: &'static str, resource_type: &'static str, body: &'static str) -> Self {
        Self {
            file_type,
            resource_type,
            body,
        }
    }
}

async fn write_legacy_artifact(
    output: &Arc<LocalFsOutputStore>,
    tenant: &TenantContext,
    submission_id: &SubmissionId,
    artifact: &LegacyArtifact,
    fencing_token: u64,
) -> (u64, u64) {
    let key = ExportPartKey {
        tenant_id: tenant.tenant_id().as_str().to_owned(),
        job_id: submission_output_job_id(submission_id),
        resource_type: artifact.resource_type.to_owned(),
        file_type: artifact.file_type.to_owned(),
        part_index: 0,
        fencing_token,
    };
    let mut writer = output.open_writer(&key).await.expect("open legacy writer");
    let line = artifact.body.trim_end_matches('\n');
    writer.write_line(line).await.expect("write legacy line");
    let finalized = output
        .finalize_part(&key, writer)
        .await
        .expect("finalize legacy part");
    (finalized.line_count, finalized.size_bytes)
}

async fn publish_legacy_manifest(
    backend: &Arc<SqliteBackend>,
    output: &Arc<LocalFsOutputStore>,
    tenant: &TenantContext,
    submission_id: &SubmissionId,
    manifest_url: &str,
    artifacts: &[LegacyArtifact],
) {
    let manifest = backend
        .list_manifests(tenant, submission_id)
        .await
        .expect("list legacy manifests")
        .into_iter()
        .find(|m| m.manifest_url.as_deref() == Some(manifest_url))
        .expect("legacy manifest");
    let worker_id = WorkerId::new("legacy-fixture-worker");
    let lease = backend
        .claim_next_manifest(&worker_id, Duration::from_secs(60))
        .await
        .expect("claim legacy manifest")
        .expect("a pending legacy manifest");
    assert_eq!(lease.manifest_id, manifest.manifest_id);
    backend
        .mark_manifest_processing(&lease)
        .await
        .expect("mark legacy manifest processing");

    let mut records = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        let (line_count, byte_count) =
            write_legacy_artifact(output, tenant, submission_id, artifact, lease.fencing_token)
                .await;
        let record = SubmitFileRecord {
            manifest_url: Some(manifest_url.to_string()),
            file_type: artifact.file_type.to_owned(),
            resource_type: Some(artifact.resource_type.to_owned()),
            part_index: 0,
            file_path: format!("{}-0", artifact.resource_type),
            line_count,
            byte_count,
            count_severity: None,
        };
        records.push(record.clone());
        backend
            .record_submit_file(&lease, &record)
            .await
            .expect("stage legacy artifact");
    }

    backend
        .publish_manifest_artifacts(&lease, &records, ManifestPublicationStatus::Completed)
        .await
        .expect("publish legacy manifest");
}

fn clear_legacy_publication_worker(
    db_path: &Path,
    tenant: &TenantContext,
    submission_id: &SubmissionId,
) {
    let connection = rusqlite::Connection::open(db_path).expect("open legacy SQLite database");
    connection
        .execute(
            "UPDATE bulk_manifests
             SET publication_worker_id = NULL
             WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
            rusqlite::params![
                tenant.tenant_id().as_str(),
                submission_id.submitter,
                submission_id.submission_id,
            ],
        )
        .expect("clear legacy publication marker");
}

fn legacy_test_tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
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

/// A kickoff with the same submitter/submission identity but another manifest URL.
fn kickoff_body_at(manifest_url: &str) -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "submitter", "valueIdentifier": {"system": "http://ehr", "value": "ehr-1"}},
            {"name": "submissionId", "valueString": "it-1"},
            {"name": "manifestUrl", "valueUrl": manifest_url},
            {"name": "fhirBaseUrl", "valueUrl": "https://provider/fhir"}
        ]
    })
}

fn replace_only_body() -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "submitter", "valueIdentifier": {"system": "http://ehr", "value": "ehr-1"}},
            {"name": "submissionId", "valueString": "it-1"},
            {"name": "submissionStatus", "valueCoding": {
                "system": "http://hl7.org/fhir/event-status", "code": "in-progress"}},
            {"name": "replacesManifestUrl", "valueUrl": "https://provider/manifest.json"}
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

/// Builds a kick-off carrying `manifestUrl` plus a terminal `submissionStatus`,
/// the one-shot shape `bulk-submit-smoke.yml` uses.
fn kickoff_body_with_status(submission_id: &str, code: &str) -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "submitter", "valueIdentifier": {"system": "http://ehr", "value": "ehr-1"}},
            {"name": "submissionId", "valueString": submission_id},
            {"name": "manifestUrl", "valueUrl": "https://provider/manifest.json"},
            {"name": "fhirBaseUrl", "valueUrl": "https://provider/fhir"},
            {"name": "submissionStatus", "valueCoding": {
                "system": "http://hl7.org/fhir/event-status", "code": code}}
        ]
    })
}

/// `submissionStatus=completed` SHALL make the submission terminal.
///
/// Regression: the kick-off handler branched only on `stopped`, so `completed`
/// returned 200 while leaving the row `in-progress` forever. Each such
/// submission then held a slot against `max_concurrent_per_tenant` (default 4),
/// after which every further kick-off returned 429.
#[tokio::test]
async fn test_completed_status_finalizes_submission() {
    let (server, backend, ..) = create_submit_server().await;

    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&kickoff_body_with_status("done-1", "completed"))
            .await
            .status_code(),
        StatusCode::OK
    );

    let tenant = helios_persistence::tenant::TenantContext::new(
        helios_persistence::tenant::TenantId::new("test-tenant"),
        helios_persistence::tenant::TenantPermissions::full_access(),
    );
    let sub_id = helios_persistence::core::SubmissionId::new("http://ehr|ehr-1", "done-1");
    let summary = backend
        .get_submission(&tenant, &sub_id)
        .await
        .expect("get_submission")
        .expect("submission exists");
    assert_eq!(
        summary.status,
        helios_persistence::core::SubmissionStatus::Complete,
        "completed kick-off must finalize the submission, got {}",
        summary.status
    );

    // And being terminal, it must reject further kick-offs.
    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&kickoff_body_with_status("done-1", "completed"))
            .await
            .status_code(),
        StatusCode::CONFLICT
    );
}

/// #850: a drained submission — every manifest terminal, but never closed
/// with `submissionStatus=completed` — must not hold a tenant concurrency
/// slot. A submitter that ingests and walks away used to leak its slot
/// forever, until the tenant's every kick-off returned 429.
#[tokio::test]
async fn test_drained_submission_frees_its_concurrency_slot() {
    let (server, backend, fetcher, output, _tmp) = create_submit_server_with(
        mock_fetcher(),
        BulkSubmitConfig {
            max_concurrent_per_tenant: 1,
            ..Default::default()
        },
    )
    .await;

    // First submission ingests to the end; the submitter never closes it.
    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&kickoff_body_with_status("leak-1", "in-progress"))
            .await
            .status_code(),
        StatusCode::OK
    );
    drain_submit(&backend, &fetcher, &output).await;

    // Drained, the open submission holds no slot: a second one is admitted.
    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&kickoff_body_with_status("leak-2", "in-progress"))
            .await
            .status_code(),
        StatusCode::OK,
        "a drained submission must not consume the tenant's only slot"
    );

    // The second one's manifest is still pending, so the cap is now real.
    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&kickoff_body_with_status("leak-3", "in-progress"))
            .await
            .status_code(),
        StatusCode::TOO_MANY_REQUESTS,
        "a submission with work in flight must still count toward the cap"
    );
}

/// A completed submission's already-registered manifests SHALL still be ingested.
///
/// `completed` means "no further manifests are coming", not "stop processing".
/// The worker's claim query gates on submission status, so this pins that a
/// finalized submission is still drained — the one-shot shape CI relies on.
#[tokio::test]
async fn test_completed_submission_still_ingests_manifests() {
    let (server, backend, fetcher, output, _tmp) = create_submit_server().await;

    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&kickoff_body_with_status("done-2", "completed"))
            .await
            .status_code(),
        StatusCode::OK
    );

    drain_submit(&backend, &fetcher, &output).await;

    let tenant = helios_persistence::tenant::TenantContext::new(
        helios_persistence::tenant::TenantId::new("test-tenant"),
        helios_persistence::tenant::TenantPermissions::full_access(),
    );
    for id in ["sub-p1", "sub-p2"] {
        assert!(
            backend
                .read(&tenant, "Patient", id)
                .await
                .unwrap()
                .is_some(),
            "Patient/{id} must be ingested even though the submission is complete"
        );
    }
}

/// `submissionStatus=stopped` aborts rather than completes.
#[tokio::test]
async fn test_stopped_status_aborts_submission() {
    let (server, backend, ..) = create_submit_server().await;

    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&kickoff_body_with_status("stop-1", "stopped"))
            .await
            .status_code(),
        StatusCode::OK
    );

    let tenant = helios_persistence::tenant::TenantContext::new(
        helios_persistence::tenant::TenantId::new("test-tenant"),
        helios_persistence::tenant::TenantPermissions::full_access(),
    );
    let sub_id = helios_persistence::core::SubmissionId::new("http://ehr|ehr-1", "stop-1");
    let summary = backend
        .get_submission(&tenant, &sub_id)
        .await
        .expect("get_submission")
        .expect("submission exists");
    assert_eq!(
        summary.status,
        helios_persistence::core::SubmissionStatus::Aborted
    );
}

/// Kicks off a submission plus its status request and returns the poll path.
async fn start_and_get_poll_path(server: &TestServer) -> String {
    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&kickoff_body())
            .await
            .status_code(),
        StatusCode::OK
    );
    let status = server
        .post("/$bulk-submit-status")
        .json(&status_body())
        .await;
    assert_eq!(status.status_code(), StatusCode::ACCEPTED);
    status
        .headers()
        .get("content-location")
        .expect("Content-Location")
        .to_str()
        .unwrap()
        .trim_start_matches("http://localhost:8080")
        .to_string()
}

#[tokio::test]
async fn test_kickoff_returns_200() {
    let (server, ..) = create_submit_server().await;
    let resp = server.post("/$bulk-submit").json(&kickoff_body()).await;
    assert_eq!(resp.status_code(), StatusCode::OK);
}

#[tokio::test]
async fn test_replace_only_kickoff_retires_manifest_without_adding_an_empty_one() {
    let (server, backend, ..) = create_submit_server().await;
    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&kickoff_body())
            .await
            .status_code(),
        StatusCode::OK
    );
    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&replace_only_body())
            .await
            .status_code(),
        StatusCode::OK
    );

    let tenant = helios_persistence::tenant::TenantContext::new(
        helios_persistence::tenant::TenantId::new("test-tenant"),
        helios_persistence::tenant::TenantPermissions::full_access(),
    );
    let sub_id = helios_persistence::core::SubmissionId::new("http://ehr|ehr-1", "it-1");
    let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
    assert_eq!(manifests.len(), 1, "replace-only must not add a manifest");
    assert_eq!(
        manifests[0].status,
        helios_persistence::core::ManifestStatus::Replaced
    );
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

/// Runs kick-off → status kick-off → worker drain and returns the poll path.
async fn run_to_completion(
    server: &TestServer,
    backend: &Arc<SqliteBackend>,
    fetcher: &Arc<MockFetcher>,
    output: &Arc<LocalFsOutputStore>,
) -> String {
    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&kickoff_body())
            .await
            .status_code(),
        StatusCode::OK
    );
    let status = server
        .post("/$bulk-submit-status")
        .json(&status_body())
        .await;
    assert_eq!(status.status_code(), StatusCode::ACCEPTED);
    let poll_path = status
        .headers()
        .get("content-location")
        .expect("Content-Location")
        .to_str()
        .unwrap()
        .trim_start_matches("http://localhost:8080")
        .to_string();
    drain_submit(backend, fetcher, output).await;
    poll_path
}

/// Trims the configured public origin from an advertised URL for local GETs.
fn local_file_path(url: &str) -> &str {
    url.trim_start_matches("http://localhost:8080")
}

/// A migrated SQLite generation keeps manifest IDs but uses the old
/// `{resource_type}-{part}` route. Two different file kinds sharing
/// `OperationOutcome-0` are ambiguous and fail the whole visible status set;
/// a distinct `Bundle-0` route remains readable.
#[tokio::test]
async fn legacy_routes_reject_duplicate_operation_outcomes() {
    let (server, backend, output, db_path, _tmp) = create_file_submit_server(BulkSubmitConfig {
        manifest_page_size: 1,
        ..Default::default()
    })
    .await;
    let tenant = legacy_test_tenant();
    let submission_id = SubmissionId::new("http://ehr|ehr-1", "it-1");

    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&kickoff_body_at("https://provider/legacy-a.json"))
            .await
            .status_code(),
        StatusCode::OK
    );
    let token = backend
        .ensure_poll_token(&tenant, &submission_id)
        .await
        .expect("mint legacy poll token");
    let poll_path = format!("/bulk-submit-status/{token}");
    publish_legacy_manifest(
        &backend,
        &output,
        &tenant,
        &submission_id,
        "https://provider/legacy-a.json",
        &[
            LegacyArtifact::new("output", "OperationOutcome", LEGACY_OUTPUT_BYTES),
            LegacyArtifact::new("deleted", "Bundle", LEGACY_DELETED_BYTES),
        ],
    )
    .await;
    clear_legacy_publication_worker(&db_path, &tenant, &submission_id);
    let files = backend
        .list_submit_files(&tenant, &submission_id)
        .await
        .expect("list migrated legacy files");
    assert_eq!(files.len(), 2);
    assert!(
        files
            .iter()
            .all(|row| row.manifest_id.is_some() && row.legacy_locator)
    );

    // Canonical publication order puts `deleted` before `output`; page 1 is
    // therefore the deleted receipt, and page 2 advertises the legacy output.
    let manifest: Value = server.get(&format!("{poll_path}?page=2")).await.json();
    let output_entries = manifest["output"].as_array().expect("legacy output array");
    assert_eq!(output_entries.len(), 1);
    assert_eq!(output_entries[0]["count"], 1);
    assert_eq!(
        output_entries[0]["fileSize"],
        LEGACY_OUTPUT_BYTES.len() as u64
    );
    let output_url = output_entries[0]["url"].as_str().expect("output url");
    assert!(output_url.ends_with(&format!("/bulk-submit-file/{token}/OperationOutcome-0")));
    assert_eq!(
        server.get(local_file_path(output_url)).await.text(),
        LEGACY_OUTPUT_BYTES
    );

    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&kickoff_body_at("https://provider/legacy-b.json"))
            .await
            .status_code(),
        StatusCode::OK
    );
    publish_legacy_manifest(
        &backend,
        &output,
        &tenant,
        &submission_id,
        "https://provider/legacy-b.json",
        &[LegacyArtifact::new(
            "error",
            "OperationOutcome",
            LEGACY_ERROR_BYTES,
        )],
    )
    .await;
    clear_legacy_publication_worker(&db_path, &tenant, &submission_id);
    let files = backend
        .list_submit_files(&tenant, &submission_id)
        .await
        .expect("list colliding legacy files");
    assert_eq!(files.len(), 3);
    assert!(
        files
            .iter()
            .all(|row| row.manifest_id.is_some() && row.legacy_locator)
    );

    let status = server.get(&poll_path).await;
    assert_eq!(status.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    let status_outcome: Value = status.json();
    assert_eq!(status_outcome["issue"][0]["code"], "exception");

    let duplicate = server
        .get(&format!("/bulk-submit-file/{token}/OperationOutcome-0"))
        .await;
    assert_eq!(duplicate.status_code(), StatusCode::NOT_FOUND);
    let duplicate_outcome: Value = duplicate.json();
    assert_eq!(duplicate_outcome["issue"][0]["code"], "not-found");

    let independent = server
        .get(&format!("/bulk-submit-file/{token}/Bundle-0"))
        .await;
    assert_eq!(independent.status_code(), StatusCode::OK);
    assert_eq!(independent.text(), LEGACY_DELETED_BYTES);
}

/// The old `{type}-{part}` route could collapse an `output` and an `error`
/// OperationOutcome with the same part index. Manifest-aware v1 routes bind the
/// protocol file kind too, so both artifacts remain addressable and downloadable.
#[tokio::test]
async fn operation_outcome_output_and_error_routes_are_distinct() {
    let (server, backend, fetcher, output, _tmp) =
        create_submit_server_with(operation_outcome_fetcher(), BulkSubmitConfig::default()).await;
    let poll_path = run_to_completion(&server, &backend, &fetcher, &output).await;

    let manifest: Value = server.get(&poll_path).await.json();
    let output = manifest["output"].as_array().expect("output array");
    let outcome = manifest["outcome"].as_array().expect("outcome array");
    assert_eq!(output.len(), 1);
    assert_eq!(outcome.len(), 1);

    let output_url = output[0]["url"].as_str().expect("output url");
    let outcome_url = outcome[0]["url"].as_str().expect("outcome url");
    assert_eq!(output[0]["count"], 1);
    assert_eq!(output[0]["fileSize"], 42);
    assert_ne!(output_url, outcome_url);
    for url in [output_url, outcome_url] {
        assert!(
            url.contains("submit-v1-"),
            "v1 locator must include its route hash: {url}"
        );
        let resp = server.get(local_file_path(url)).await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{url}");
    }

    let output_bytes = server.get(local_file_path(output_url)).await.text();
    assert_eq!(
        output_bytes,
        "{\"reference\":\"OperationOutcome/oo-out-1\"}\n"
    );

    let outcome_bytes = server.get(local_file_path(outcome_url)).await.text();
    assert_eq!(
        outcome_bytes,
        concat!(
            "{\"resourceType\":\"OperationOutcome\",\"issue\":[{",
            "\"severity\":\"error\",\"code\":\"processing\",",
            "\"diagnostics\":\"1 submitted resource(s) could not be parsed ",
            "or did not match the declared resource type\"}]}\n"
        )
    );
    assert_eq!(outcome[0]["count"], 1);
    assert_eq!(outcome[0]["fileSize"], 191);
    assert_eq!(
        outcome[0]["countSeverity"],
        json!([{"code": "error", "count": 1}])
    );
}

/// Two manifests under one submitter/submission ID are separate generations.
/// Their part-0 Patient outputs must use distinct manifest-aware routes, and each
/// route must return its own exact receipt.
#[tokio::test]
async fn manifest_generations_use_distinct_v1_routes() {
    let (server, backend, fetcher, output, _tmp) =
        create_submit_server_with(two_manifest_fetcher(), BulkSubmitConfig::default()).await;

    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&kickoff_body_at("https://provider/manifest-a.json"))
            .await
            .status_code(),
        StatusCode::OK
    );
    assert_eq!(
        server
            .post("/$bulk-submit")
            .json(&kickoff_body_at("https://provider/manifest-b.json"))
            .await
            .status_code(),
        StatusCode::OK
    );
    let status = server
        .post("/$bulk-submit-status")
        .json(&status_body())
        .await;
    assert_eq!(status.status_code(), StatusCode::ACCEPTED);
    let poll_path = status
        .headers()
        .get("content-location")
        .expect("Content-Location")
        .to_str()
        .unwrap()
        .trim_start_matches("http://localhost:8080")
        .to_string();
    drain_submit(&backend, &fetcher, &output).await;

    let manifest: Value = server.get(&poll_path).await.json();
    let outputs = manifest["output"].as_array().expect("output array");
    assert_eq!(outputs.len(), 2);
    let manifest_urls = outputs
        .iter()
        .map(|entry| entry["manifestUrl"].as_str().expect("manifest url"))
        .collect::<Vec<_>>();
    assert!(manifest_urls.contains(&"https://provider/manifest-a.json"));
    assert!(manifest_urls.contains(&"https://provider/manifest-b.json"));

    let mut routes = Vec::new();
    for (manifest_url, url) in outputs.iter().map(|entry| {
        (
            entry["manifestUrl"].as_str().expect("manifest url"),
            entry["url"].as_str().expect("output url"),
        )
    }) {
        let route = url
            .rsplit('/')
            .next()
            .expect("URL always has a path component");
        assert!(route.starts_with("submit-v1-"));
        routes.push(route.to_string());
        assert_eq!(
            server.get(local_file_path(url)).await.text(),
            match manifest_url {
                "https://provider/manifest-a.json" => "{\"reference\":\"Patient/mm-a-1\"}\n",
                "https://provider/manifest-b.json" => "{\"reference\":\"Patient/mm-b-1\"}\n",
                unexpected => unreachable!("unexpected manifestUrl: {unexpected}"),
            }
        );
    }
    assert_ne!(routes[0], routes[1]);
}

/// Spec (submit.html): when the status manifest is returned incrementally, `link`
/// carries a single `relation: next` entry pointing at the following manifest page,
/// and every other field repeats identically across pages.
#[tokio::test]
async fn test_status_manifest_paginates_with_next_links() {
    let (server, backend, fetcher, output, _tmp) = create_submit_server_with(
        multi_artifact_fetcher(),
        BulkSubmitConfig {
            manifest_page_size: 1,
            ..Default::default()
        },
    )
    .await;
    let poll_path = run_to_completion(&server, &backend, &fetcher, &output).await;

    // Walk the `next` chain, collecting every advertised artifact URL.
    let mut pages: Vec<Value> = Vec::new();
    let mut next = Some(poll_path.clone());
    while let Some(path) = next {
        let resp = server.get(&path).await;
        assert_eq!(resp.status_code(), StatusCode::OK, "page {path}");
        let manifest: Value = resp.json();
        let links = manifest["link"].as_array().expect("link array").clone();
        assert!(links.len() <= 1, "at most one next link: {links:?}");
        next = links.first().map(|l| {
            assert_eq!(l["relation"], "next");
            l["url"]
                .as_str()
                .expect("link url")
                .trim_start_matches("http://localhost:8080")
                .to_string()
        });
        pages.push(manifest);
    }

    // Three artifacts (output + outcome + deleted) → three single-entry pages.
    assert_eq!(pages.len(), 3, "expected 3 pages, got {}", pages.len());
    let mut totals = (0, 0, 0);
    for page in &pages {
        let (o, c, d) = (
            page["output"].as_array().unwrap().len(),
            page["outcome"].as_array().unwrap().len(),
            page["deleted"].as_array().unwrap().len(),
        );
        assert_eq!(o + c + d, 1, "each page holds one entry: {page}");
        totals = (totals.0 + o, totals.1 + c, totals.2 + d);
    }
    // Paging spans all three arrays without dropping or duplicating an entry.
    assert_eq!(
        totals,
        (1, 1, 1),
        "output/outcome/deleted entries across the chain"
    );
    // Non-paged fields repeat identically (spec SHALL).
    for page in &pages[1..] {
        for field in [
            "submissionId",
            "transactionTime",
            "requiresAccessToken",
            "outputFormat",
        ] {
            assert_eq!(page[field], pages[0][field], "field {field} must repeat");
        }
    }
    // The last page has no `next`.
    assert!(pages[2]["link"].as_array().unwrap().is_empty());

    // A page past the end is 404; a malformed page is 400.
    assert_eq!(
        server
            .get(&format!("{poll_path}?page=4"))
            .await
            .status_code(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        server
            .get(&format!("{poll_path}?page=0"))
            .await
            .status_code(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        server
            .get(&format!("{poll_path}?page=abc"))
            .await
            .status_code(),
        StatusCode::BAD_REQUEST
    );
}

/// With `manifest_page_size = 0` pagination is off: one manifest, empty `link`.
#[tokio::test]
async fn test_pagination_disabled_returns_single_manifest() {
    let (server, backend, fetcher, output, _tmp) = create_submit_server_with(
        multi_artifact_fetcher(),
        BulkSubmitConfig {
            manifest_page_size: 0,
            ..Default::default()
        },
    )
    .await;
    let poll_path = run_to_completion(&server, &backend, &fetcher, &output).await;

    let manifest: Value = server.get(&poll_path).await.json();
    assert_eq!(manifest["output"].as_array().unwrap().len(), 1);
    assert_eq!(manifest["outcome"].as_array().unwrap().len(), 1);
    assert_eq!(manifest["deleted"].as_array().unwrap().len(), 1);
    assert!(manifest["link"].as_array().unwrap().is_empty());
    assert_eq!(
        server
            .get(&format!("{poll_path}?page=2"))
            .await
            .status_code(),
        StatusCode::NOT_FOUND
    );
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

#[tokio::test]
async fn path_tenant_and_public_prefix_flow_through_bulk_submit_urls() {
    let (server, backend, fetcher, output, _tmp) = create_submit_server_with_routing(
        multi_artifact_fetcher(),
        BulkSubmitConfig {
            manifest_page_size: 1,
            ..Default::default()
        },
        TenantRoutingMode::Both,
        "https://public.example/fhir/",
        "default",
    )
    .await;

    assert_eq!(
        server
            .post("/acme/$bulk-submit")
            .json(&kickoff_body())
            .await
            .status_code(),
        StatusCode::OK
    );
    let status = server
        .post("/acme/$bulk-submit-status")
        .json(&status_body())
        .await;
    assert_eq!(status.status_code(), StatusCode::ACCEPTED);
    let status_url = status
        .headers()
        .get("content-location")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(status_url.starts_with("https://public.example/fhir/acme/bulk-submit-status/"));
    let poll_path = status_url
        .strip_prefix("https://public.example/fhir")
        .unwrap();
    let wrong_tenant_path = poll_path.replacen("/acme/", "/other/", 1);
    assert_eq!(
        server.get(&wrong_tenant_path).await.status_code(),
        StatusCode::NOT_FOUND
    );

    drain_submit(&backend, &fetcher, &output).await;
    let done = server.get(poll_path).await;
    assert_eq!(done.status_code(), StatusCode::OK);
    let manifest: Value = done.json();
    let artifact_url = ["output", "outcome", "deleted"]
        .into_iter()
        .find_map(|kind| manifest[kind][0]["url"].as_str())
        .expect("first page has an artifact URL");
    assert!(artifact_url.starts_with("https://public.example/fhir/acme/bulk-submit-file/"));
    assert!(
        manifest["link"][0]["url"]
            .as_str()
            .unwrap()
            .starts_with(status_url)
    );
}

#[tokio::test]
async fn both_mode_header_tenant_can_follow_unprefixed_bulk_submit_urls() {
    let public_base = "https://public.example/fhir";
    let (server, backend, fetcher, output, _tmp) = create_submit_server_with_routing(
        mock_fetcher(),
        BulkSubmitConfig::default(),
        TenantRoutingMode::Both,
        public_base,
        "default",
    )
    .await;

    assert_eq!(
        server
            .post("/$bulk-submit")
            .add_header("x-tenant-id", "acme")
            .json(&kickoff_body())
            .await
            .status_code(),
        StatusCode::OK
    );
    let status = server
        .post("/$bulk-submit-status")
        .add_header("x-tenant-id", "acme")
        .json(&status_body())
        .await;
    assert_eq!(status.status_code(), StatusCode::ACCEPTED);
    let status_url = status
        .headers()
        .get("content-location")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(status_url.starts_with("https://public.example/fhir/bulk-submit-status/"));
    assert!(!status_url.contains("/acme/"));
    let poll_path = status_url.strip_prefix(public_base).unwrap();

    assert_eq!(
        server
            .get(poll_path)
            .add_header("x-tenant-id", "acme")
            .await
            .status_code(),
        StatusCode::ACCEPTED
    );
    drain_submit(&backend, &fetcher, &output).await;

    let done = server
        .get(poll_path)
        .add_header("x-tenant-id", "acme")
        .await;
    assert_eq!(done.status_code(), StatusCode::OK);
    let manifest: Value = done.json();
    let artifact_url = manifest["output"][0]["url"].as_str().unwrap();
    assert!(artifact_url.starts_with("https://public.example/fhir/bulk-submit-file/"));
    let artifact_path = artifact_url.strip_prefix(public_base).unwrap();
    assert_eq!(
        server
            .get(artifact_path)
            .add_header("x-tenant-id", "other")
            .await
            .status_code(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        server
            .delete(poll_path)
            .add_header("x-tenant-id", "other")
            .await
            .status_code(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        server
            .get(artifact_path)
            .add_header("x-tenant-id", "acme")
            .await
            .status_code(),
        StatusCode::OK
    );

    // The URL-path form remains available in `both` mode.
    assert_eq!(
        server.get(&format!("/acme{poll_path}")).await.status_code(),
        StatusCode::OK
    );
}

// ── Status-poll pacing: Retry-After + rate limiting (issue #399) ──────────────

#[tokio::test]
async fn test_in_progress_poll_advertises_the_configured_retry_after() {
    let (server, ..) = create_submit_server_with(
        mock_fetcher(),
        BulkSubmitConfig {
            retry_after_secs: 7,
            pre_ingest_retry_after_secs: 7,
            ..BulkSubmitConfig::default()
        },
    )
    .await;
    let poll_path = start_and_get_poll_path(&server).await;

    // The worker has not run, so the submission is still in progress.
    let resp = server.get(&poll_path).await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    assert_eq!(
        resp.headers().get("retry-after").expect("Retry-After"),
        "7",
        "the in-progress poll must advertise HFS_BULK_SUBMIT_RETRY_AFTER"
    );
}

/// The pre-ingest phases (#953) each last seconds, while the ingest cadence
/// is two minutes by default. A poller that honours the long cadence from the
/// first `202` sleeps straight through queued -> reading -> sizing ->
/// downloading and the phase reports never reach a screen — HFS's own Import
/// page showed the queued text for its whole first window. So the short
/// cadence is advertised until the first counted byte or entry, and the long
/// one after.
#[tokio::test]
async fn test_poll_advertises_the_short_cadence_until_ingest_starts() {
    let (server, backend, _fetcher, _output, _tmp) = create_submit_server_with(
        mock_fetcher(),
        BulkSubmitConfig {
            retry_after_secs: 90,
            pre_ingest_retry_after_secs: 8,
            ..BulkSubmitConfig::default()
        },
    )
    .await;
    let poll_path = start_and_get_poll_path(&server).await;

    // Queued: nothing claimed.
    let resp = server.get(&poll_path).await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    assert_eq!(
        resp.headers().get("retry-after").expect("Retry-After"),
        "8",
        "a queued submission must advertise the pre-ingest cadence"
    );

    // Claimed and in a named phase: still pre-ingest.
    let lease = backend
        .claim_next_manifest(&WorkerId::new("cadence-worker"), Duration::from_secs(60))
        .await
        .expect("claim")
        .expect("a manifest to claim");
    backend
        .update_manifest_phase(&lease, ManifestPhase::Sizing, 3, 12)
        .await
        .expect("phase update");
    let resp = server.get(&poll_path).await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    assert_eq!(
        resp.headers().get("retry-after").expect("Retry-After"),
        "8",
        "a sizing submission must advertise the pre-ingest cadence"
    );

    // First bytes counted: ingest cadence.
    backend
        .update_manifest_bytes(&lease, 350, 1_000)
        .await
        .expect("bytes update");
    let resp = server.get(&poll_path).await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    assert_eq!(
        resp.headers().get("retry-after").expect("Retry-After"),
        "90",
        "once bytes are counted the poll must advertise HFS_BULK_SUBMIT_RETRY_AFTER"
    );
}

/// The advertised pre-ingest cadence must never be one the rate limiter
/// would punish: a client doing exactly what the header says has to stay
/// inside `POLL_RATE_LIMIT` per `POLL_RATE_WINDOW`.
#[tokio::test]
async fn test_pre_ingest_cadence_is_clamped_to_the_poll_rate_limit() {
    let (server, ..) = create_submit_server_with(
        mock_fetcher(),
        BulkSubmitConfig {
            pre_ingest_retry_after_secs: 1,
            poll_rate_limit: 4,
            poll_rate_window_secs: 60,
            ..BulkSubmitConfig::default()
        },
    )
    .await;
    let poll_path = start_and_get_poll_path(&server).await;

    let resp = server.get(&poll_path).await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    assert_eq!(
        resp.headers().get("retry-after").expect("Retry-After"),
        "15",
        "4 polls per 60s means one poll every 15s at most"
    );
}

#[tokio::test]
async fn test_poll_beyond_the_rate_limit_returns_429_with_retry_after() {
    // Spec (build.fhir.org submit.html): Data Consumers SHOULD rate-limit the
    // status endpoint and return Retry-After so clients back off.
    let (server, ..) = create_submit_server_with(
        mock_fetcher(),
        BulkSubmitConfig {
            poll_rate_limit: 2,
            poll_rate_window_secs: 60,
            ..BulkSubmitConfig::default()
        },
    )
    .await;
    let poll_path = start_and_get_poll_path(&server).await;

    for _ in 0..2 {
        assert_eq!(
            server.get(&poll_path).await.status_code(),
            StatusCode::ACCEPTED,
            "polls within the limit must be served"
        );
    }

    let throttled = server.get(&poll_path).await;
    assert_eq!(throttled.status_code(), StatusCode::TOO_MANY_REQUESTS);
    let secs: u64 = throttled
        .headers()
        .get("retry-after")
        .expect("429 must carry Retry-After")
        .to_str()
        .unwrap()
        .parse()
        .expect("Retry-After must be delta-seconds");
    assert!(
        (1..=60).contains(&secs),
        "Retry-After must point inside the rate window, got {secs}"
    );
    // The rejection is a FHIR OperationOutcome, not an empty body.
    let outcome: Value = throttled.json();
    assert_eq!(outcome["resourceType"], "OperationOutcome");
    assert_eq!(outcome["issue"][0]["code"], "throttled");
}

#[tokio::test]
async fn test_poll_rate_limit_of_zero_disables_throttling() {
    let (server, ..) = create_submit_server_with(
        mock_fetcher(),
        BulkSubmitConfig {
            poll_rate_limit: 0,
            ..BulkSubmitConfig::default()
        },
    )
    .await;
    let poll_path = start_and_get_poll_path(&server).await;

    for _ in 0..12 {
        assert_eq!(
            server.get(&poll_path).await.status_code(),
            StatusCode::ACCEPTED,
            "poll rate limiting must be off when the limit is 0"
        );
    }
}

/// The in-progress percentage is byte-based: a worker that has streamed part
/// of a file moves the poll's X-Progress long before any manifest turns
/// terminal, instead of a single-manifest submission sitting at 0% to the end.
#[tokio::test]
async fn test_poll_percentage_tracks_ingested_bytes() {
    let (server, backend, _fetcher, _output, _tmp) =
        create_submit_server_with(mock_fetcher(), BulkSubmitConfig::default()).await;
    let poll_path = start_and_get_poll_path(&server).await;

    let lease = backend
        .claim_next_manifest(&WorkerId::new("byte-worker"), Duration::from_secs(60))
        .await
        .expect("claim")
        .expect("a manifest to claim");
    backend
        .update_manifest_bytes(&lease, 350, 1_000)
        .await
        .expect("bytes update");

    let resp = server.get(&poll_path).await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    let progress = resp
        .headers()
        .get("x-progress")
        .expect("X-Progress")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        progress.contains("Processing 35%"),
        "the percentage must follow ingested bytes, got: {progress}"
    );
}

/// #969: the resource counter has to survive the trip through the header.
///
/// `X-Progress` gained the "N resources written" clause joined by an em dash,
/// and an em dash is not US-ASCII. `HeaderValue::to_str()` — which is how
/// `reqwest`, and therefore HFS's own Import page, reads a header — refuses
/// anything outside visible ASCII, so the poller saw no value at all and fell
/// back to a bare "in progress". The regression was invisible to the byte-
/// percentage test above because that one leaves `processed_entries` at zero,
/// which is exactly the branch that stays ASCII.
///
/// So this asserts the readable form, not the bytes: a counter that only a
/// permissive client can decode is a counter the operator does not have.
#[tokio::test]
async fn test_poll_progress_header_is_ascii_readable_with_a_resource_count() {
    let (server, backend, _fetcher, _output, _tmp) =
        create_submit_server_with(mock_fetcher(), BulkSubmitConfig::default()).await;
    let poll_path = start_and_get_poll_path(&server).await;

    let lease = backend
        .claim_next_manifest(&WorkerId::new("counting-worker"), Duration::from_secs(60))
        .await
        .expect("claim")
        .expect("a manifest to claim");
    backend
        .update_manifest_bytes(&lease, 350, 1_000)
        .await
        .expect("bytes update");
    backend
        .add_manifest_progress(&lease, 1_234, 0, 1_234)
        .await
        .expect("progress update");

    let resp = server.get(&poll_path).await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    let raw = resp.headers().get("x-progress").expect("X-Progress");
    let progress = raw
        .to_str()
        .unwrap_or_else(|e| panic!("X-Progress must be ASCII a client can read: {e}"));
    assert_eq!(
        progress, "Processing 35% - 1,234 Resources written",
        "the poll must report both the byte percentage and the resource count"
    );
}

/// #646: a `processing` manifest whose worker lease expired without renewal
/// or reclaim used to poll as a quiet "processing" forever — a frozen worker
/// pool was indistinguishable from progress. The poll now names the stall.
#[tokio::test]
async fn test_poll_reports_a_stalled_ingestion() {
    // lease_duration_secs = 0 makes the stall threshold (3 leases) immediate.
    let (server, backend, _fetcher, _output, _tmp) = create_submit_server_with(
        mock_fetcher(),
        BulkSubmitConfig {
            lease_duration_secs: 0,
            ..Default::default()
        },
    )
    .await;
    let poll_path = start_and_get_poll_path(&server).await;

    // A worker claims the manifest (zero-duration lease: expired on arrival)
    // and then never heartbeats, finishes, or gets reclaimed — the freeze.
    let worker_id = WorkerId::new("frozen-worker");
    backend
        .claim_next_manifest(&worker_id, Duration::ZERO)
        .await
        .expect("claim")
        .expect("a manifest to claim");

    let resp = server.get(&poll_path).await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    let progress = resp
        .headers()
        .get("x-progress")
        .expect("X-Progress")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        progress.contains("stalled"),
        "a dead worker pool must be visible to the poller, got: {progress}"
    );
}

// ── Pre-ingest phase vocabulary (issue #953) ─────────────────────────────────
//
// Claiming a manifest, fetching the remote Bulk Export Manifest, and HEAD-ing
// its output files all precede the first counted byte, so the poll used to read
// a flat "processing 0% complete" for the whole pre-ingest window. Each phase
// now names itself — and none of the names may start with `processing `, which
// the UI parses as a determinate percentage (the #827 regression).

/// Polls once and returns the `X-Progress` text, asserting the 202.
async fn poll_progress(server: &TestServer, poll_path: &str) -> String {
    let resp = server.get(poll_path).await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    resp.headers()
        .get("x-progress")
        .expect("X-Progress")
        .to_str()
        .unwrap()
        .to_string()
}

/// A submission whose manifests are all still `pending` is queued, not slow:
/// no worker has claimed anything, so there is no percentage to report. With
/// the in-process pool running an idle worker claims within seconds, so the
/// text says "queued", not "waiting" — nothing scarce is being waited on.
#[tokio::test]
async fn test_poll_reports_queued_before_any_claim() {
    let (server, ..) = create_submit_server_with(mock_fetcher(), BulkSubmitConfig::default()).await;
    let poll_path = start_and_get_poll_path(&server).await;

    let progress = poll_progress(&server, &poll_path).await;
    assert_eq!(
        progress, "Queued - starting shortly",
        "an unclaimed submission must say it is queued, got: {progress}"
    );
    assert!(
        !progress.to_ascii_lowercase().starts_with("processing "),
        "an indeterminate phase must not look like a determinate percentage (#827)"
    );
}

/// With the in-process worker pool disabled nothing in this process will ever
/// claim the manifest, so "starting shortly" would be a promise HFS cannot
/// keep. The operator needs to hear that an external worker is the missing
/// piece.
#[tokio::test]
async fn test_poll_reports_the_external_worker_wait_when_the_local_pool_is_off() {
    let (server, ..) = create_submit_server_with(
        mock_fetcher(),
        BulkSubmitConfig {
            disable_local_worker: true,
            ..BulkSubmitConfig::default()
        },
    )
    .await;
    let poll_path = start_and_get_poll_path(&server).await;

    let progress = poll_progress(&server, &poll_path).await;
    assert_eq!(
        progress, "Queued - waiting for an external worker",
        "an unclaimed submission with no local workers must name the dependency, got: {progress}"
    );
}

/// While the worker downloads and parses the remote manifest there is no file
/// list yet, so the phase carries the status on its own.
#[tokio::test]
async fn test_poll_reports_the_manifest_read_phase() {
    let (server, backend, _fetcher, _output, _tmp) =
        create_submit_server_with(mock_fetcher(), BulkSubmitConfig::default()).await;
    let poll_path = start_and_get_poll_path(&server).await;

    let lease = backend
        .claim_next_manifest(&WorkerId::new("phase-worker"), Duration::from_secs(60))
        .await
        .expect("claim")
        .expect("a manifest to claim");
    backend
        .update_manifest_phase(&lease, ManifestPhase::ReadingManifest, 0, 0)
        .await
        .expect("phase update");

    let progress = poll_progress(&server, &poll_path).await;
    assert_eq!(
        progress, "reading manifest",
        "the manifest fetch must be visible to the poller, got: {progress}"
    );
    assert!(
        !progress.starts_with("processing "),
        "an indeterminate phase must not look like a determinate percentage (#827)"
    );
}

/// Pre-sizing HEAD-s every output file to learn the byte denominator; on a
/// large export that alone can take minutes, so it reports file counts.
#[tokio::test]
async fn test_poll_reports_the_sizing_phase_with_file_counts() {
    let (server, backend, _fetcher, _output, _tmp) =
        create_submit_server_with(mock_fetcher(), BulkSubmitConfig::default()).await;
    let poll_path = start_and_get_poll_path(&server).await;

    let lease = backend
        .claim_next_manifest(&WorkerId::new("sizing-worker"), Duration::from_secs(60))
        .await
        .expect("claim")
        .expect("a manifest to claim");
    backend
        .update_manifest_phase(&lease, ManifestPhase::Sizing, 37, 412)
        .await
        .expect("phase update");

    let progress = poll_progress(&server, &poll_path).await;
    assert_eq!(
        progress, "sizing 37 of 412 files",
        "pre-sizing must report its file counts, got: {progress}"
    );
    assert!(
        !progress.starts_with("processing "),
        "an indeterminate phase must not look like a determinate percentage (#827)"
    );
}

/// The first file is open but no batch has flushed its byte counter yet — the
/// window that used to read 0%.
#[tokio::test]
async fn test_poll_reports_the_downloading_phase_with_file_counts() {
    let (server, backend, _fetcher, _output, _tmp) =
        create_submit_server_with(mock_fetcher(), BulkSubmitConfig::default()).await;
    let poll_path = start_and_get_poll_path(&server).await;

    let lease = backend
        .claim_next_manifest(&WorkerId::new("download-worker"), Duration::from_secs(60))
        .await
        .expect("claim")
        .expect("a manifest to claim");
    backend
        .update_manifest_phase(&lease, ManifestPhase::Downloading, 1, 412)
        .await
        .expect("phase update");

    let progress = poll_progress(&server, &poll_path).await;
    assert_eq!(
        progress, "downloading file 1 of 412",
        "the file being fetched must be visible to the poller, got: {progress}"
    );
    assert!(
        !progress.starts_with("processing "),
        "an indeterminate phase must not look like a determinate percentage (#827)"
    );
}

/// A phase whose denominator is not known yet must not print "of 0": it falls
/// back to the plain percentage instead.
#[tokio::test]
async fn test_poll_falls_back_when_the_phase_has_no_file_total() {
    let (server, backend, _fetcher, _output, _tmp) =
        create_submit_server_with(mock_fetcher(), BulkSubmitConfig::default()).await;
    let poll_path = start_and_get_poll_path(&server).await;

    let lease = backend
        .claim_next_manifest(&WorkerId::new("unsized-worker"), Duration::from_secs(60))
        .await
        .expect("claim")
        .expect("a manifest to claim");
    backend
        .update_manifest_phase(&lease, ManifestPhase::Sizing, 0, 0)
        .await
        .expect("phase update");

    let progress = poll_progress(&server, &poll_path).await;
    assert_eq!(
        progress, "Processing 0%",
        "an unknown file total must not render as 'of 0', got: {progress}"
    );
}

/// Rules 2-3 outrank the phase, which is what makes a stale phase harmless: a
/// manifest still flagged `Downloading` while its bytes move reports the real
/// percentage, never the pre-ingest text.
#[tokio::test]
async fn test_moving_bytes_outrank_a_stale_phase() {
    let (server, backend, _fetcher, _output, _tmp) =
        create_submit_server_with(mock_fetcher(), BulkSubmitConfig::default()).await;
    let poll_path = start_and_get_poll_path(&server).await;

    let lease = backend
        .claim_next_manifest(
            &WorkerId::new("stale-phase-worker"),
            Duration::from_secs(60),
        )
        .await
        .expect("claim")
        .expect("a manifest to claim");
    backend
        .update_manifest_phase(&lease, ManifestPhase::Downloading, 1, 412)
        .await
        .expect("phase update");
    backend
        .update_manifest_bytes(&lease, 350, 1_000)
        .await
        .expect("bytes update");

    let progress = poll_progress(&server, &poll_path).await;
    assert!(
        progress.contains("Processing 35%"),
        "a real percentage must take over from the pre-ingest phase, got: {progress}"
    );
}

/// `X-Progress` is an HTTP field value, so RFC 9110 §5.5 confines it to
/// US-ASCII; a byte above 0x7F is `obs-text` with undefined meaning, and
/// conservative clients drop the whole value rather than guess. `to_str` is one
/// of those clients — an em dash in the resource-count wording made our own
/// Bulk Import card fall back to a literal "in progress", replacing the phase
/// text with a placeholder.
///
/// The count branch is the one that carried it, and no other test reaches that
/// branch: `poll_progress` would panic on a non-ASCII header, so this asserts
/// the bytes directly and states the invariant for every branch at once.
#[tokio::test]
async fn test_progress_header_stays_ascii_in_every_branch() {
    let (server, backend, _fetcher, _output, _tmp) =
        create_submit_server_with(mock_fetcher(), BulkSubmitConfig::default()).await;
    let poll_path = start_and_get_poll_path(&server).await;

    let lease = backend
        .claim_next_manifest(&WorkerId::new("ascii-worker"), Duration::from_secs(60))
        .await
        .expect("claim")
        .expect("a manifest to claim");
    backend
        .update_manifest_bytes(&lease, 350, 1_000)
        .await
        .expect("bytes update");
    // Entries outrank bytes, so this selects the resource-count wording. The
    // counters are cumulative deltas (#969) and this manifest starts at zero,
    // so the added count is the reported one.
    backend
        .add_manifest_progress(&lease, 609_191, 0, 609_191)
        .await
        .expect("entry update");

    let resp = server.get(&poll_path).await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    let raw = resp
        .headers()
        .get("x-progress")
        .expect("X-Progress")
        .as_bytes()
        .to_vec();
    let text = String::from_utf8_lossy(&raw);
    assert!(
        raw.is_ascii(),
        "X-Progress must be US-ASCII; a conservative client discards it otherwise. Got: {text}"
    );
    assert!(
        text.contains("609,191 Resources written"),
        "the count branch must still be the one under test, got: {text}"
    );
}
