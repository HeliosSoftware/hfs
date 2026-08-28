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
    BulkSubmitJobStore, BulkSubmitProvider, DefaultSubmitWorker, ExportOutputStore, RemoteFile,
    RemoteManifest, ResourceStorage, SubmitClaimStrategy, SubmitInputFetcher, WorkerId,
};
use helios_persistence::error::StorageResult;
use helios_rest::ServerConfig;
use helios_rest::bulk_export_auth::BearerScopeAuth;
use helios_rest::config::{BulkSubmitConfig, MultitenancyConfig, TenantRoutingMode};
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
        _encryption_key: Option<&Value>,
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
    })
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
