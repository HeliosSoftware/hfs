//! Integration tests for the FHIR Bulk Data Export (`$export`) endpoints.
//!
//! Exercises the kick-off → poll → manifest → download → delete lifecycle for
//! all three export levels, plus parameter validation, the `ExportStatus` →
//! HTTP mapping, and the unsupported-parameter behavior.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::local_fs::LocalFsOutputStore;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{
    BulkExportJobStore, DefaultExportWorker, ExportClaimStrategy, ExportOutputStore,
    ResourceStorage, WorkerId,
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
    let output = Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080"));
    let file_auth = Arc::new(BearerScopeAuth);

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    };

    let state = helios_rest::AppState::new(Arc::clone(&backend), config).with_bulk_export(
        backend.clone() as Arc<dyn BulkExportJobStore>,
        output.clone() as Arc<dyn ExportOutputStore>,
        file_auth,
    );
    let app = helios_rest::routing::fhir_routes::create_routes(state);
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
    let tenant = test_tenant();
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
