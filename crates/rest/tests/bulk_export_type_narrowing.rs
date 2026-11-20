//! `$export` without `_type` under a per-type token exports the types the
//! token may read, not every type the tenant holds.

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
    BulkExportJobStore, DefaultExportWorker, ExportClaimStrategy, ExportOutputStore,
    ResourceStorage, WorkerId,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use helios_rest::bulk_export_auth::BearerScopeAuth;
use serde_json::{Value, json};

const TENANT: &str = "test-tenant";

fn tenant() -> TenantContext {
    TenantContext::new(TenantId::new(TENANT), TenantPermissions::full_access())
}

/// A bulk-export server whose every request carries a principal with `scopes`.
async fn server_for(scopes: &str) -> (TestServer, Arc<SqliteBackend>, Arc<LocalFsOutputStore>) {
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
    // The directory lives as long as the process; the test is short.
    std::mem::forget(tmp);

    let config = ServerConfig {
        base_url: "http://localhost:8080".to_string(),
        default_tenant: TENANT.to_string(),
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::clone(&backend), config).with_bulk_export(
        backend.clone() as Arc<dyn BulkExportJobStore>,
        output.clone() as Arc<dyn ExportOutputStore>,
        Arc::new(BearerScopeAuth),
    );
    let principal = Principal {
        subject: "narrow-client".to_string(),
        issuer: "https://issuer.example".to_string(),
        tenant_id: Some(TENANT.to_string()),
        scopes: ScopeSet::parse(scopes),
        jti: None,
        expires_at: Utc::now() + chrono::Duration::hours(1),
        custom_claims: serde_json::Map::new(),
        ..Default::default()
    };
    let app = helios_rest::routing::fhir_routes::create_routes(state).layer(
        axum::middleware::from_fn(move |mut request: axum::extract::Request, next: Next| {
            let principal = principal.clone();
            async move {
                request.extensions_mut().insert(principal);
                next.run(request).await
            }
        }),
    );
    (TestServer::new(app).expect("test server"), backend, output)
}

async fn seed(backend: &Arc<SqliteBackend>) {
    for resource in [
        json!({"resourceType": "Patient", "id": "p1"}),
        json!({"resourceType": "Observation", "id": "o1", "status": "final",
               "code": {"text": "x"}, "subject": {"reference": "Patient/p1"}}),
        json!({"resourceType": "Condition", "id": "c1", "subject": {"reference": "Patient/p1"}}),
    ] {
        let resource_type = resource["resourceType"].as_str().unwrap().to_string();
        backend
            .create(&tenant(), &resource_type, resource, FhirVersion::default())
            .await
            .expect("seed");
    }
}

async fn drain_workers(backend: &Arc<SqliteBackend>, output: &Arc<LocalFsOutputStore>) {
    let worker_id = WorkerId::new("test-worker");
    let worker = DefaultExportWorker::new(
        backend.clone(),
        backend.clone(),
        output.clone(),
        worker_id.clone(),
    );
    while let Some(lease) = backend
        .claim_next(&worker_id, Duration::from_secs(60), 3)
        .await
        .expect("claim_next")
    {
        worker.run_job(lease).await.expect("run_job");
    }
}

/// Kicks off `/$export` without `_type` and returns the finished manifest's
/// output types, sorted.
async fn exported_types(
    server: &TestServer,
    backend: &Arc<SqliteBackend>,
    output: &Arc<LocalFsOutputStore>,
) -> Vec<String> {
    let kickoff = server
        .get("/$export")
        .add_header("x-tenant-id", TENANT)
        .add_header("prefer", "respond-async")
        .await;
    assert_eq!(
        kickoff.status_code(),
        StatusCode::ACCEPTED,
        "{}",
        kickoff.text()
    );
    let status_path = kickoff
        .headers()
        .get("content-location")
        .expect("Content-Location")
        .to_str()
        .unwrap()
        .strip_prefix("http://localhost:8080")
        .unwrap()
        .to_string();
    drain_workers(backend, output).await;
    let done = server
        .get(&status_path)
        .add_header("x-tenant-id", TENANT)
        .await;
    assert_eq!(done.status_code(), StatusCode::OK, "{}", done.text());
    let manifest: Value = done.json();
    let mut types: Vec<String> = manifest["output"]
        .as_array()
        .expect("output")
        .iter()
        .map(|f| f["type"].as_str().unwrap().to_string())
        .collect();
    types.sort();
    types.dedup();
    types
}

#[tokio::test]
async fn an_export_without_type_under_a_per_type_token_exports_only_the_granted_types() {
    let (server, backend, output) = server_for("user/Observation.rs user/Condition.rs").await;
    seed(&backend).await;
    assert_eq!(
        exported_types(&server, &backend, &output).await,
        vec!["Condition".to_string(), "Observation".to_string()],
        "Patient is held by the tenant but not granted to the token"
    );
}

#[tokio::test]
async fn an_export_without_type_under_a_wildcard_token_still_exports_every_type() {
    let (server, backend, output) = server_for("user/*.rs").await;
    seed(&backend).await;
    assert_eq!(
        exported_types(&server, &backend, &output).await,
        vec![
            "Condition".to_string(),
            "Observation".to_string(),
            "Patient".to_string()
        ]
    );
}

#[tokio::test]
async fn an_export_without_type_under_a_token_granting_no_type_is_refused() {
    let (server, backend, _output) = server_for("openid user/export-status.r").await;
    seed(&backend).await;
    let kickoff = server
        .get("/$export")
        .add_header("x-tenant-id", TENANT)
        .add_header("prefer", "respond-async")
        .await;
    assert_eq!(
        kickoff.status_code(),
        StatusCode::FORBIDDEN,
        "{}",
        kickoff.text()
    );
}

#[tokio::test]
async fn a_named_type_outside_the_grant_is_still_refused() {
    let (server, backend, _output) = server_for("user/Observation.rs").await;
    seed(&backend).await;
    let kickoff = server
        .get("/$export")
        .add_header("x-tenant-id", TENANT)
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient")
        .await;
    assert_eq!(
        kickoff.status_code(),
        StatusCode::FORBIDDEN,
        "{}",
        kickoff.text()
    );
}
