//! Integration tests for the FHIR Bulk Data Export (`$export`) endpoints.
//!
//! Exercises the kick-off → poll → manifest → download → delete lifecycle for
//! all three export levels, plus parameter validation, the `ExportStatus` →
//! HTTP mapping, and the unsupported-parameter behavior.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum::middleware::Next;
use axum_test::TestServer;
use chrono::Utc;
use helios_auth::{Principal, ScopeSet};
use helios_fhir::FhirVersion;
use helios_persistence::backends::local_fs::LocalFsOutputStore;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{
    BulkExportJobStore, BulkExportStorage, DefaultExportWorker, ExportClaimStrategy,
    ExportOutputStore, ExportWorkerStorage, ResourceStorage, WorkerId,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use helios_rest::bulk_export_auth::BearerScopeAuth;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

/// Builds a test server with the bulk-export subsystem wired in, plus the
/// SQLite backend and the local-FS output store (for driving a worker).
async fn create_bulk_export_server() -> (
    TestServer,
    Arc<SqliteBackend>,
    Arc<LocalFsOutputStore>,
    tempfile::TempDir,
) {
    create_bulk_export_server_with(
        TenantRoutingMode::HeaderOnly,
        "http://localhost:8080",
        "test-tenant",
        None,
    )
    .await
}

async fn create_bulk_export_server_with(
    routing_mode: TenantRoutingMode,
    base_url: &str,
    default_tenant: &str,
    principal_tenant: Option<&str>,
) -> (
    TestServer,
    Arc<SqliteBackend>,
    Arc<LocalFsOutputStore>,
    tempfile::TempDir,
) {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let backend_config = SqliteBackendConfig {
        data_dir: Some(data_dir),
        ..Default::default()
    };
    let backend = Arc::new(
        SqliteBackend::with_config(":memory:", backend_config).expect("create SQLite backend"),
    );
    backend.init_schema().expect("init schema");

    let tmp = tempfile::tempdir().expect("tempdir");
    let output = Arc::new(LocalFsOutputStore::new(
        tmp.path(),
        "https://wrong-internal.example",
    ));
    let file_auth = Arc::new(BearerScopeAuth);

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode,
            strict_validation: principal_tenant.is_some(),
            ..Default::default()
        },
        base_url: base_url.to_string(),
        default_tenant: default_tenant.to_string(),
        ..ServerConfig::for_testing()
    };

    let state = helios_rest::AppState::new(Arc::clone(&backend), config).with_bulk_export(
        backend.clone() as Arc<dyn BulkExportJobStore>,
        output.clone() as Arc<dyn ExportOutputStore>,
        file_auth,
    );
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let app = if let Some(tenant_id) = principal_tenant {
        let principal = Principal {
            subject: "bulk-export-client".to_string(),
            issuer: "https://issuer.example".to_string(),
            tenant_id: Some(tenant_id.to_string()),
            scopes: ScopeSet::parse("system/Patient.rs"),
            jti: None,
            expires_at: Utc::now() + chrono::Duration::hours(1),
            custom_claims: serde_json::Map::new(),
        };
        app.layer(axum::middleware::from_fn(
            move |mut request: axum::extract::Request, next: Next| {
                let principal = principal.clone();
                async move {
                    request.extensions_mut().insert(principal);
                    next.run(request).await
                }
            },
        ))
    } else {
        app
    };
    let server = TestServer::new(app).expect("create test server");

    (server, backend, output, tmp)
}

fn test_tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
}

/// Drains all currently-claimable export jobs by running a worker synchronously.
async fn drain_workers(backend: &Arc<SqliteBackend>, output: &Arc<LocalFsOutputStore>) {
    let worker_id = WorkerId::new("test-worker");
    let worker = DefaultExportWorker::new(
        backend.clone(),
        backend.clone(),
        output.clone(),
        worker_id.clone(),
    );
    while let Some(lease) = backend
        .claim_next(&worker_id, Duration::from_secs(60))
        .await
        .expect("claim_next")
    {
        worker.run_job(lease).await.expect("run_job");
    }
}

/// Seeds N Patient resources.
async fn seed_patients(backend: &Arc<SqliteBackend>, n: usize) {
    seed_patients_for(backend, "test-tenant", n).await;
}

async fn seed_patients_for(backend: &Arc<SqliteBackend>, tenant_id: &str, n: usize) {
    let tenant = TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access());
    for i in 0..n {
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": format!("p{i}")}),
                FhirVersion::default(),
            )
            .await
            .expect("seed patient");
    }
}

#[tokio::test]
async fn authenticated_path_tenant_and_public_prefix_flow_through_bulk_export_urls() {
    let (server, backend, output, _tmp) = create_bulk_export_server_with(
        TenantRoutingMode::UrlPath,
        "https://public.example/fhir/",
        "default",
        Some("acme"),
    )
    .await;
    seed_patients_for(&backend, "acme", 1).await;

    let kickoff = server
        .get("/acme/$export")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient")
        .await;
    assert_eq!(kickoff.status_code(), StatusCode::ACCEPTED);
    let status_url = kickoff
        .headers()
        .get("content-location")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(status_url.starts_with("https://public.example/fhir/acme/export-status/"));
    let status_path = status_url
        .strip_prefix("https://public.example/fhir")
        .unwrap();

    drain_workers(&backend, &output).await;
    let done = server.get(status_path).await;
    assert_eq!(done.status_code(), StatusCode::OK);
    let manifest: Value = done.json();
    assert_eq!(
        manifest["request"],
        "https://public.example/fhir/acme/$export?_type=Patient"
    );
    assert!(
        manifest["output"][0]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://public.example/fhir/acme/export-file/")
    );
}

#[tokio::test]
async fn both_mode_header_tenant_can_follow_unprefixed_bulk_export_urls() {
    let public_base = "https://public.example/fhir";
    let (server, backend, output, _tmp) =
        create_bulk_export_server_with(TenantRoutingMode::Both, public_base, "default", None).await;
    seed_patients_for(&backend, "acme", 1).await;

    let kickoff = server
        .get("/$export")
        .add_header("x-tenant-id", "acme")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient")
        .await;
    assert_eq!(kickoff.status_code(), StatusCode::ACCEPTED);
    let status_url = kickoff
        .headers()
        .get("content-location")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(status_url.starts_with("https://public.example/fhir/export-status/"));
    assert!(!status_url.contains("/acme/"));
    let status_path = status_url.strip_prefix(public_base).unwrap();

    assert_eq!(
        server
            .get(status_path)
            .add_header("x-tenant-id", "acme")
            .await
            .status_code(),
        StatusCode::ACCEPTED
    );
    drain_workers(&backend, &output).await;

    let done = server
        .get(status_path)
        .add_header("x-tenant-id", "acme")
        .await;
    assert_eq!(done.status_code(), StatusCode::OK);
    let manifest: Value = done.json();
    assert_eq!(
        manifest["request"],
        "https://public.example/fhir/$export?_type=Patient"
    );
    let file_url = manifest["output"][0]["url"].as_str().unwrap();
    assert!(file_url.starts_with("https://public.example/fhir/export-file/"));
    let file_path = file_url.strip_prefix(public_base).unwrap();
    assert_eq!(
        server
            .get(file_path)
            .add_header("x-tenant-id", "acme")
            .await
            .status_code(),
        StatusCode::OK
    );

    // `both` still accepts the canonical URL-path form for the same job.
    assert_eq!(
        server
            .get(&format!("/acme{status_path}"))
            .await
            .status_code(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn test_system_export_full_lifecycle() {
    let (server, backend, output, _tmp) = create_bulk_export_server().await;
    seed_patients(&backend, 3).await;

    // Kick-off — requires Prefer: respond-async.
    let resp = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient")
        .await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    let status_url = resp
        .headers()
        .get("content-location")
        .expect("Content-Location header")
        .to_str()
        .unwrap()
        .to_string();
    let status_path = status_url.strip_prefix("http://localhost:8080").unwrap();

    // Poll before the worker runs — still 202.
    let polling = server
        .get(status_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(polling.status_code(), StatusCode::ACCEPTED);
    assert!(polling.headers().get("retry-after").is_some());

    // Run the worker.
    drain_workers(&backend, &output).await;

    // Poll again — now 200 + manifest.
    let done = server
        .get(status_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(done.status_code(), StatusCode::OK);
    let manifest: Value = done.json();
    assert!(manifest["transactionTime"].is_string());
    assert_eq!(
        manifest["request"],
        "http://localhost:8080/$export?_type=Patient"
    );
    assert_eq!(manifest["requiresAccessToken"], true);
    let output_files = manifest["output"].as_array().expect("output array");
    let total: u64 = output_files
        .iter()
        .map(|f| f["count"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(total, 3);

    // Download the first output file.
    let file_url = output_files[0]["url"].as_str().unwrap();
    let file_path = file_url.strip_prefix("http://localhost:8080").unwrap();
    let download = server
        .get(file_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(download.status_code(), StatusCode::OK);
    assert_eq!(
        download.headers().get("content-type").unwrap(),
        "application/fhir+ndjson"
    );
    assert_eq!(download.text().lines().count(), 3);

    // Delete the job.
    let deleted = server
        .delete(status_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(deleted.status_code(), StatusCode::ACCEPTED);

    // Status URL is now gone.
    let gone = server
        .get(status_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(gone.status_code(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_status_poll_reports_types_progress_while_in_flight() {
    let (server, backend, _output, _tmp) = create_bulk_export_server().await;
    let tenant = test_tenant();
    seed_patients(&backend, 1).await;
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "o1",
                "status": "final",
                "code": {"text": "test"},
            }),
            FhirVersion::default(),
        )
        .await
        .expect("seed observation");
    backend
        .create(
            &tenant,
            "Condition",
            json!({"resourceType": "Condition", "id": "c1"}),
            FhirVersion::default(),
        )
        .await
        .expect("seed condition");

    let kickoff = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient,Observation,Condition")
        .await;
    assert_eq!(kickoff.status_code(), StatusCode::ACCEPTED);
    let status_url = kickoff
        .headers()
        .get("content-location")
        .expect("Content-Location header")
        .to_str()
        .unwrap()
        .to_string();
    let status_path = status_url.strip_prefix("http://localhost:8080").unwrap();
    let job_id = status_path.rsplit('/').next().unwrap().to_string();

    // Put the job in flight without running the real worker: claim it, mark
    // it in-progress, and record that one of the three types is done.
    let worker_id = WorkerId::new("t");
    let lease = backend
        .claim_next(&worker_id, Duration::from_secs(60))
        .await
        .expect("claim_next")
        .expect("a job is claimable right after kick-off");
    backend
        .mark_export_in_progress(&tenant, &lease.job_id, &worker_id, lease.fencing_token)
        .await
        .expect("mark_export_in_progress");
    backend
        .set_export_current_type(
            &tenant,
            &lease.job_id,
            &worker_id,
            lease.fencing_token,
            Some("Observation"),
            1,
            3,
        )
        .await
        .expect("set_export_current_type");

    let polling = server
        .get(status_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(polling.status_code(), StatusCode::ACCEPTED);
    assert_eq!(
        polling
            .headers()
            .get("x-progress")
            .unwrap()
            .to_str()
            .unwrap(),
        "33%"
    );
    assert!(
        polling
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("application/fhir+json")
    );
    let body: Value = polling.json();
    assert_eq!(body["resourceType"], "Parameters");
    let params = body["parameter"].as_array().expect("parameter array");
    let find = |name: &str| params.iter().find(|p| p["name"] == name);
    assert_eq!(find("exportId").unwrap()["valueString"], job_id);
    assert_eq!(find("status").unwrap()["valueCode"], "in-progress");
    assert_eq!(find("typesTotal").unwrap()["valueInteger"], 3);
    assert_eq!(find("typesDone").unwrap()["valueInteger"], 1);
    assert_eq!(find("currentType").unwrap()["valueString"], "Observation");
}

#[tokio::test]
async fn test_status_poll_before_worker_starts_reports_zero_without_current_type() {
    let (server, backend, _output, _tmp) = create_bulk_export_server().await;
    seed_patients(&backend, 1).await;

    let kickoff = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient")
        .await;
    assert_eq!(kickoff.status_code(), StatusCode::ACCEPTED);
    let status_url = kickoff
        .headers()
        .get("content-location")
        .expect("Content-Location header")
        .to_str()
        .unwrap()
        .to_string();
    let status_path = status_url.strip_prefix("http://localhost:8080").unwrap();

    let polling = server
        .get(status_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(polling.status_code(), StatusCode::ACCEPTED);
    assert_eq!(
        polling
            .headers()
            .get("x-progress")
            .unwrap()
            .to_str()
            .unwrap(),
        "0%"
    );
    let body: Value = polling.json();
    let params = body["parameter"].as_array().expect("parameter array");
    let find = |name: &str| params.iter().find(|p| p["name"] == name);
    assert_eq!(find("typesTotal").unwrap()["valueInteger"], 0);
    assert_eq!(find("typesDone").unwrap()["valueInteger"], 0);
    assert!(find("currentType").is_none());
}

#[tokio::test]
async fn test_status_poll_percent_is_capped_at_99_while_running() {
    let (server, backend, _output, _tmp) = create_bulk_export_server().await;
    let tenant = test_tenant();
    seed_patients(&backend, 1).await;

    let kickoff = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient")
        .await;
    assert_eq!(kickoff.status_code(), StatusCode::ACCEPTED);
    let status_url = kickoff
        .headers()
        .get("content-location")
        .expect("Content-Location header")
        .to_str()
        .unwrap()
        .to_string();
    let status_path = status_url.strip_prefix("http://localhost:8080").unwrap();

    let worker_id = WorkerId::new("t");
    let lease = backend
        .claim_next(&worker_id, Duration::from_secs(60))
        .await
        .expect("claim_next")
        .expect("a job is claimable right after kick-off");
    backend
        .mark_export_in_progress(&tenant, &lease.job_id, &worker_id, lease.fencing_token)
        .await
        .expect("mark_export_in_progress");
    // Every type reported done, but the job has not yet transitioned to
    // `complete` — a real, if transitory, state while the worker finalizes
    // output files and the manifest.
    backend
        .set_export_current_type(
            &tenant,
            &lease.job_id,
            &worker_id,
            lease.fencing_token,
            Some("Patient"),
            3,
            3,
        )
        .await
        .expect("set_export_current_type");

    let polling = server
        .get(status_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(polling.status_code(), StatusCode::ACCEPTED);
    assert_eq!(
        polling
            .headers()
            .get("x-progress")
            .unwrap()
            .to_str()
            .unwrap(),
        "99%"
    );
}

#[tokio::test]
async fn test_patient_and_group_export_levels() {
    let (server, backend, output, _tmp) = create_bulk_export_server().await;
    seed_patients(&backend, 2).await;

    // Patient-level kick-off.
    let resp = server
        .get("/Patient/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);

    // Group-level kick-off.
    let tenant = test_tenant();
    backend
        .create(
            &tenant,
            "Group",
            json!({"resourceType": "Group", "id": "g1", "member": []}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let resp = server
        .get("/Group/g1/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);

    drain_workers(&backend, &output).await;
}

#[tokio::test]
async fn test_group_export_missing_group_is_404_and_creates_no_job() {
    let (server, backend, _output, _tmp) = create_bulk_export_server().await;

    let resp = server
        .get("/Group/no-such-group/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .await;
    assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

    let body: Value = resp.json();
    assert_eq!(body["issue"][0]["code"], "not-found");
    let diagnostics = body["issue"][0]["details"]["text"].as_str().unwrap();
    assert!(
        diagnostics.contains("no-such-group"),
        "diagnostics should mention the missing group id, got: {diagnostics}"
    );

    let tenant = test_tenant();
    assert_eq!(
        backend.count_active_exports(&tenant).await.unwrap(),
        0,
        "no job should have been created"
    );
    assert!(
        backend
            .list_exports(&tenant, true)
            .await
            .unwrap()
            .is_empty(),
        "no job should be listed for the tenant"
    );
}

#[tokio::test]
async fn test_group_export_deleted_group_is_404() {
    let (server, backend, _output, _tmp) = create_bulk_export_server().await;

    let tenant = test_tenant();
    backend
        .create(
            &tenant,
            "Group",
            json!({"resourceType": "Group", "id": "g-del", "member": []}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend.delete(&tenant, "Group", "g-del").await.unwrap();

    let resp = server
        .get("/Group/g-del/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .await;
    assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_failed_job_status_poll_returns_operation_outcome_with_diagnostics() {
    let (server, backend, output, _tmp) = create_bulk_export_server().await;
    seed_patients(&backend, 2).await;

    let tenant = test_tenant();
    backend
        .create(
            &tenant,
            "Group",
            json!({
                "resourceType": "Group",
                "id": "g-gone",
                "member": [{"entity": {"reference": "Patient/p1"}}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Kick-off succeeds — the Group exists at this point.
    let resp = server
        .get("/Group/g-gone/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    let status_url = resp
        .headers()
        .get("content-location")
        .expect("Content-Location header")
        .to_str()
        .unwrap()
        .to_string();
    let status_path = status_url
        .strip_prefix("http://localhost:8080")
        .unwrap()
        .to_string();

    // The Group is removed after kick-off, so the worker fails the job when
    // it tries to resolve its members.
    backend.delete(&tenant, "Group", "g-gone").await.unwrap();

    // A local drain loop rather than the shared `drain_workers` helper, which
    // asserts every run succeeds — this job is expected to fail.
    let worker_id = WorkerId::new("test-worker-failing");
    let worker = DefaultExportWorker::new(
        backend.clone(),
        backend.clone(),
        output.clone(),
        worker_id.clone(),
    );
    while let Some(lease) = backend
        .claim_next(&worker_id, Duration::from_secs(60))
        .await
        .expect("claim_next")
    {
        let _ = worker.run_job(lease).await;
    }

    let polled = server
        .get(&status_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(polled.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        polled.headers().get("content-type").unwrap(),
        "application/fhir+json"
    );
    let body: Value = polled.json();
    assert_eq!(body["resourceType"], "OperationOutcome");
    assert_eq!(body["issue"][0]["severity"], "error");
    assert_eq!(body["issue"][0]["code"], "processing");
    let diagnostics = body["issue"][0]["diagnostics"].as_str().unwrap();
    assert!(
        diagnostics.contains("g-gone"),
        "diagnostics should mention the missing group id, got: {diagnostics}"
    );
    let lower = diagnostics.to_lowercase();
    assert!(!lower.contains("sqlite"), "got: {diagnostics}");
    assert!(!lower.contains("select"), "got: {diagnostics}");
    assert!(!lower.contains("table"), "got: {diagnostics}");
}

#[tokio::test]
async fn test_kickoff_requires_respond_async() {
    let (server, _backend, _output, _tmp) = create_bulk_export_server().await;
    let resp = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_unsupported_output_format_rejected() {
    let (server, _backend, _output, _tmp) = create_bulk_export_server().await;
    let resp = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_outputFormat", "text/csv")
        .await;
    assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_unsupported_param_strict_vs_lenient() {
    let (server, _backend, _output, _tmp) = create_bulk_export_server().await;

    // strict → 400
    let strict = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async, handling=strict")
        .add_query_param("includeAssociatedData", "LatestProvenanceResources")
        .await;
    assert_eq!(strict.status_code(), StatusCode::BAD_REQUEST);

    // no handling directive (lenient default) → accepted
    let lenient = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("includeAssociatedData", "LatestProvenanceResources")
        .await;
    assert_eq!(lenient.status_code(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn test_type_filter_validation() {
    let (server, _backend, _output, _tmp) = create_bulk_export_server().await;

    // _typeFilter whose resource type is not in _type → 400
    let mismatch = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient")
        .add_query_param("_typeFilter", "Observation?status=final")
        .await;
    assert_eq!(mismatch.status_code(), StatusCode::BAD_REQUEST);

    // _typeFilter carrying a result-control param → 400
    let bad_param = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Observation")
        .add_query_param("_typeFilter", "Observation?_sort=date")
        .await;
    assert_eq!(bad_param.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_type_filter_unknown_param_rejected() {
    let (server, backend, _output, _tmp) = create_bulk_export_server().await;

    let resp = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient")
        .add_query_param("_typeFilter", "Patient?foo=bar")
        .await;
    assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
    let body: Value = resp.json();
    let text = body["issue"][0]["details"]["text"].as_str().unwrap();
    assert!(text.contains("foo"), "got: {text}");
    assert!(text.contains("_typeFilter"), "got: {text}");

    let tenant = test_tenant();
    assert_eq!(
        backend.count_active_exports(&tenant).await.unwrap(),
        0,
        "no job should be created when the type filter is rejected"
    );
}

#[tokio::test]
async fn test_type_filter_invalid_value_rejected() {
    let (server, _backend, _output, _tmp) = create_bulk_export_server().await;

    // `active` is a Patient token search parameter; `:exact` is only valid
    // for string parameters, so the builder rejects this combination.
    let resp = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient")
        .add_query_param("_typeFilter", "Patient?active:exact=true")
        .await;
    assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
    let body: Value = resp.json();
    let text = body["issue"][0]["details"]["text"].as_str().unwrap();
    assert!(text.contains("_typeFilter"), "got: {text}");
}

#[tokio::test]
async fn test_type_filter_unknown_param_rejected_even_when_lenient() {
    let (server, _backend, _output, _tmp) = create_bulk_export_server().await;

    let resp = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async, handling=lenient")
        .add_query_param("_type", "Patient")
        .add_query_param("_typeFilter", "Patient?foo=bar")
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::BAD_REQUEST,
        "_typeFilter validation is always strict, regardless of Prefer: handling"
    );
}

#[tokio::test]
async fn test_status_and_download_unknown_job() {
    let (server, _backend, _output, _tmp) = create_bulk_export_server().await;

    let status = server
        .get("/export-status/nonexistent")
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(status.status_code(), StatusCode::NOT_FOUND);

    let download = server
        .get("/export-file/nonexistent/Patient-0")
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(download.status_code(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_post_kickoff_with_parameters_body() {
    let (server, backend, output, _tmp) = create_bulk_export_server().await;
    seed_patients(&backend, 2).await;

    // POST kickoff using a FHIR Parameters resource body.
    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "_type", "valueString": "Patient"}
        ]
    });
    let resp = server
        .post("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .json(&body)
        .await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    assert!(resp.headers().get("content-location").is_some());

    drain_workers(&backend, &output).await;
}

#[tokio::test]
async fn test_since_parameter_accepted() {
    let (server, backend, output, _tmp) = create_bulk_export_server().await;
    seed_patients(&backend, 1).await;

    let resp = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_since", "2020-01-01T00:00:00Z")
        .await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);

    drain_workers(&backend, &output).await;
}

#[tokio::test]
async fn test_invalid_since_rejected() {
    let (server, _backend, _output, _tmp) = create_bulk_export_server().await;

    let resp = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_since", "not-a-date")
        .await;
    assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_elements_parameter_accepted() {
    let (server, backend, output, _tmp) = create_bulk_export_server().await;
    seed_patients(&backend, 1).await;

    let resp = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient")
        .add_query_param("_elements", "id,name")
        .await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    let status_url = resp
        .headers()
        .get("content-location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let status_path = status_url.strip_prefix("http://localhost:8080").unwrap();

    drain_workers(&backend, &output).await;

    let done = server
        .get(status_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(done.status_code(), StatusCode::OK);
}

#[tokio::test]
async fn test_valid_type_filter_accepted() {
    let (server, backend, output, _tmp) = create_bulk_export_server().await;
    seed_patients(&backend, 1).await;

    // _typeFilter with valid resource type (in _type) and allowed search param.
    let resp = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient")
        .add_query_param("_typeFilter", "Patient?active=true")
        .await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);

    // The kick-off must have compiled the filter against the search
    // parameter registry and persisted it on the job, rather than leaving
    // the worker to reinterpret the raw query string.
    let worker_id = WorkerId::new("t");
    let lease = backend
        .claim_next(&worker_id, Duration::from_secs(60))
        .await
        .expect("claim_next")
        .expect("a job should be claimable");
    let view = backend
        .get_export_job_for_worker(
            &lease.tenant,
            &lease.job_id,
            &lease.worker_id,
            lease.fencing_token,
        )
        .await
        .expect("get_export_job_for_worker");
    let compiled = view.request.type_filters[0]
        .compiled
        .as_ref()
        .expect("the compiled filter should be persisted on the job");
    assert_eq!(compiled.resource_type, "Patient");
    assert!(
        compiled.parameters.iter().any(|p| p.name == "active"),
        "compiled filter should carry the 'active' parameter, got: {:?}",
        compiled.parameters
    );

    // Run the job to completion so the leased worker doesn't leak into other
    // assertions and the output store is exercised end to end.
    let worker = DefaultExportWorker::new(
        backend.clone(),
        backend.clone(),
        output.clone(),
        worker_id.clone(),
    );
    worker.run_job(lease).await.expect("run_job");
}

#[tokio::test]
async fn test_capability_statement_advertises_export() {
    let (server, _backend, _output, _tmp) = create_bulk_export_server().await;
    let resp = server
        .get("/metadata")
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let cs: Value = resp.json();
    let ops = cs["rest"][0]["operation"]
        .as_array()
        .expect("operation array");
    let names: Vec<&str> = ops.iter().filter_map(|o| o["name"].as_str()).collect();
    assert!(names.contains(&"export"));
    assert!(names.contains(&"patient-export"));
    assert!(names.contains(&"group-export"));
    assert_eq!(
        cs["instantiates"][0],
        "http://hl7.org/fhir/uv/bulkdata/CapabilityStatement/bulk-data"
    );
}

/// Downloads an export output file and parses each NDJSON line as JSON.
async fn fetch_ndjson_lines(server: &TestServer, file_url: &str, base_url: &str) -> Vec<Value> {
    let file_path = file_url
        .strip_prefix(base_url)
        .expect("file URL should be under the server's base URL");
    let download = server
        .get(file_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(download.status_code(), StatusCode::OK);
    download
        .text()
        .lines()
        .map(|line| serde_json::from_str(line).expect("ndjson line parses as JSON"))
        .collect()
}

#[tokio::test]
async fn test_type_filter_is_applied_to_system_export() {
    let (server, backend, output, _tmp) = create_bulk_export_server().await;
    let tenant = test_tenant();
    for (id, active) in [
        ("p-active-1", true),
        ("p-active-2", true),
        ("p-inactive", false),
    ] {
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": id, "active": active}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let resp = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient")
        .add_query_param("_typeFilter", "Patient?active=true")
        .await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    let status_url = resp
        .headers()
        .get("content-location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let status_path = status_url.strip_prefix("http://localhost:8080").unwrap();

    drain_workers(&backend, &output).await;

    let done = server
        .get(status_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(done.status_code(), StatusCode::OK);
    let manifest: Value = done.json();
    let output_files = manifest["output"].as_array().expect("output array");
    assert_eq!(output_files.len(), 1, "one Patient output file");

    let lines = fetch_ndjson_lines(
        &server,
        output_files[0]["url"].as_str().unwrap(),
        "http://localhost:8080",
    )
    .await;
    assert_eq!(
        lines.len(),
        2,
        "only the two active patients should be exported, got: {lines:?}"
    );
    let ids: Vec<&str> = lines.iter().map(|v| v["id"].as_str().unwrap()).collect();
    assert!(
        !ids.contains(&"p-inactive"),
        "the inactive patient must not be in the filtered output"
    );
}

#[tokio::test]
async fn test_unfiltered_type_is_exported_whole_next_to_a_filtered_one() {
    let (server, backend, output, _tmp) = create_bulk_export_server().await;
    let tenant = test_tenant();
    backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "id": "p1"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "id": "p2"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create(
            &tenant,
            "Condition",
            json!({
                "resourceType": "Condition",
                "id": "c-active",
                "subject": {"reference": "Patient/p1"},
                "clinicalStatus": {
                    "coding": [{
                        "system": "http://terminology.hl7.org/CodeSystem/condition-clinical",
                        "code": "active"
                    }]
                }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create(
            &tenant,
            "Condition",
            json!({
                "resourceType": "Condition",
                "id": "c-resolved",
                "subject": {"reference": "Patient/p1"},
                "clinicalStatus": {
                    "coding": [{
                        "system": "http://terminology.hl7.org/CodeSystem/condition-clinical",
                        "code": "resolved"
                    }]
                }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let resp = server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient,Condition")
        .add_query_param("_typeFilter", "Condition?clinical-status=active")
        .await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    let status_url = resp
        .headers()
        .get("content-location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let status_path = status_url.strip_prefix("http://localhost:8080").unwrap();

    drain_workers(&backend, &output).await;

    let done = server
        .get(status_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(done.status_code(), StatusCode::OK);
    let manifest: Value = done.json();
    let output_files = manifest["output"].as_array().expect("output array");

    let patient_file = output_files
        .iter()
        .find(|f| f["type"] == "Patient")
        .expect("Patient output file");
    let patient_lines = fetch_ndjson_lines(
        &server,
        patient_file["url"].as_str().unwrap(),
        "http://localhost:8080",
    )
    .await;
    assert_eq!(
        patient_lines.len(),
        2,
        "the unfiltered type exports every resource"
    );

    let condition_file = output_files
        .iter()
        .find(|f| f["type"] == "Condition")
        .expect("Condition output file");
    let condition_lines = fetch_ndjson_lines(
        &server,
        condition_file["url"].as_str().unwrap(),
        "http://localhost:8080",
    )
    .await;
    assert_eq!(condition_lines.len(), 1);
    assert_eq!(condition_lines[0]["id"], "c-active");
}

#[tokio::test]
async fn test_type_filter_is_applied_to_group_export() {
    let (server, backend, output, _tmp) = create_bulk_export_server().await;
    let tenant = test_tenant();
    backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "id": "p1"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create(
            &tenant,
            "Group",
            json!({
                "resourceType": "Group",
                "id": "g1",
                "member": [{"entity": {"reference": "Patient/p1"}}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let condition_clinical = "http://terminology.hl7.org/CodeSystem/condition-clinical";
    backend
        .create(
            &tenant,
            "Condition",
            json!({
                "resourceType": "Condition",
                "id": "c-active",
                "subject": {"reference": "Patient/p1"},
                "clinicalStatus": {
                    "coding": [{"system": condition_clinical, "code": "active"}]
                }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    for id in ["c-resolved-1", "c-resolved-2"] {
        backend
            .create(
                &tenant,
                "Condition",
                json!({
                    "resourceType": "Condition",
                    "id": id,
                    "subject": {"reference": "Patient/p1"},
                    "clinicalStatus": {
                        "coding": [{"system": condition_clinical, "code": "resolved"}]
                    }
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    backend
        .create(
            &tenant,
            "Condition",
            json!({
                "resourceType": "Condition",
                "id": "c-none",
                "subject": {"reference": "Patient/p1"}
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let resp = server
        .get("/Group/g1/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient,Condition")
        .add_query_param("_typeFilter", "Condition?clinical-status=active")
        .await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    let status_url = resp
        .headers()
        .get("content-location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let status_path = status_url.strip_prefix("http://localhost:8080").unwrap();

    drain_workers(&backend, &output).await;

    let done = server
        .get(status_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(done.status_code(), StatusCode::OK);
    let manifest: Value = done.json();
    let output_files = manifest["output"].as_array().expect("output array");

    let condition_file = output_files
        .iter()
        .find(|f| f["type"] == "Condition")
        .expect("Condition output file");
    let condition_lines = fetch_ndjson_lines(
        &server,
        condition_file["url"].as_str().unwrap(),
        "http://localhost:8080",
    )
    .await;
    assert_eq!(
        condition_lines.len(),
        1,
        "only the active Condition should be exported, got: {condition_lines:?}"
    );
    assert_eq!(condition_lines[0]["id"], "c-active");

    let patient_file = output_files
        .iter()
        .find(|f| f["type"] == "Patient")
        .expect("Patient output file");
    let patient_lines = fetch_ndjson_lines(
        &server,
        patient_file["url"].as_str().unwrap(),
        "http://localhost:8080",
    )
    .await;
    assert_eq!(patient_lines.len(), 1, "the sole group member is exported");
}
