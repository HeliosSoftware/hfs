//! Integration tests verifying that bulk export, `$purge`, and `$reindex`
//! emit BALP-compliant `AuditEvent` records via the configured audit sink.
//!
//! Each test wires an [`InMemoryAuditSink`] into the app state, exercises a
//! handler (and, for bulk export, the worker), and asserts that the expected
//! `AuditEvent` resources show up in the sink buffer with the right
//! `audit-operation`, action code, outcome, and lifecycle phase.
//!
//! Gated on `feature = "R4"` because the assertions reach into
//! `helios_fhir::r4::AuditEvent` directly and the fixture uses
//! `FhirVersion::default()` (which is also R4-gated). For single-version
//! minimal builds without R4, this file compiles away to nothing.

#![cfg(feature = "R4")]
#![allow(missing_docs)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::Response;
use axum_test::TestServer;
use chrono::Utc;
use helios_audit::{ExclusionFilter, InMemoryAuditSink};
use helios_auth::{AuthConfig, AuthError, AuthProvider, Principal, ScopeSet};
use helios_fhir::FhirVersion;
use helios_persistence::backends::local_fs::LocalFsOutputStore;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::storage::PurgableStorage;
use helios_persistence::core::{
    BulkExportJobStore, DefaultExportWorker, ExportClaimStrategy, ExportOutputStore,
    ResourceStorage, WorkerId,
};
use helios_persistence::search::reindex::ReindexOperation;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use helios_rest::bulk_export_auth::BearerScopeAuth;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use helios_rest::reindex::ReindexController;
use helios_rest::{AuthMiddlewareState, middleware::auth::authz_middleware};
use serde_json::json;

/// Source observer reference used by every test fixture.
const SOURCE_OBSERVER: &str = "Device/hfs";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn test_tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
}

struct AuditedServer {
    server: TestServer,
    sink: Arc<InMemoryAuditSink>,
    backend: Arc<SqliteBackend>,
    output: Arc<LocalFsOutputStore>,
    _tmp: tempfile::TempDir,
}

/// Builds an Axum test server with bulk export, purge, and reindex all wired,
/// plus an [`InMemoryAuditSink`] threaded into the app state.
async fn create_audited_server() -> AuditedServer {
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

    let sink = Arc::new(InMemoryAuditSink::new());

    let reindex_op = ReindexOperation::new(backend.clone(), backend.search_extractor().clone())
        .with_audit(
            sink.clone() as Arc<dyn helios_audit::AuditSink>,
            SOURCE_OBSERVER,
        );

    let state = helios_rest::AppState::with_auth_and_audit(
        Arc::clone(&backend),
        config,
        helios_auth::AuthConfig::default(),
        None,
        Some(sink.clone() as Arc<dyn helios_audit::AuditSink>),
        SOURCE_OBSERVER,
    )
    .with_bulk_export(
        backend.clone() as Arc<dyn BulkExportJobStore>,
        output.clone() as Arc<dyn ExportOutputStore>,
        file_auth,
    )
    .with_purge_provider(backend.clone() as Arc<dyn PurgableStorage>)
    .with_reindex_controller(Arc::new(reindex_op) as Arc<dyn ReindexController>);

    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let server = TestServer::new(app).expect("create test server");

    AuditedServer {
        server,
        sink,
        backend,
        output,
        _tmp: tmp,
    }
}

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

/// Drains every claimable export job. The worker is constructed with the
/// audit sink so worker-level lifecycle events are captured.
async fn drain_workers_with_audit(
    backend: &Arc<SqliteBackend>,
    output: &Arc<LocalFsOutputStore>,
    sink: &Arc<InMemoryAuditSink>,
) {
    let worker_id = WorkerId::new("test-worker");
    let worker = DefaultExportWorker::new(
        backend.clone(),
        backend.clone(),
        output.clone(),
        worker_id.clone(),
    )
    .with_audit(
        sink.clone() as Arc<dyn helios_audit::AuditSink>,
        SOURCE_OBSERVER,
    );
    while let Some(lease) = backend
        .claim_next(&worker_id, Duration::from_secs(60))
        .await
        .expect("claim_next")
    {
        worker.run_job(lease).await.expect("run_job");
    }
}

// ---------------------------------------------------------------------------
// AuditEvent inspection helpers
// ---------------------------------------------------------------------------

type AuditEvent = helios_fhir::r4::AuditEvent;

fn action_code(event: &AuditEvent) -> Option<String> {
    event
        .action
        .as_ref()
        .and_then(|a| a.value.as_ref())
        .cloned()
}

fn outcome_code(event: &AuditEvent) -> Option<String> {
    event
        .outcome
        .as_ref()
        .and_then(|o| o.value.as_ref())
        .cloned()
}

fn detail_value(event: &AuditEvent, name: &str) -> Option<String> {
    use helios_fhir::r4::AuditEventEntityDetailValue;
    let entities = event.entity.as_ref()?;
    for ent in entities {
        let details = match ent.detail.as_ref() {
            Some(d) => d,
            None => continue,
        };
        for d in details {
            if d.r#type.value.as_deref() == Some(name) {
                return match &d.value {
                    Some(AuditEventEntityDetailValue::String(s)) => s.value.clone(),
                    _ => None,
                };
            }
        }
    }
    None
}

/// Returns every event whose `audit-operation` detail equals the given value.
fn events_for_operation(sink: &InMemoryAuditSink, op: &str) -> Vec<AuditEvent> {
    sink.events()
        .into_iter()
        .filter(|e| detail_value(e, "audit-operation").as_deref() == Some(op))
        .collect()
}

/// Returns every bulk-export event with a specific `bulk-export-operation`
/// (kickoff / status-complete / delete / download / worker-complete / ...).
fn export_events(sink: &InMemoryAuditSink, op: &str) -> Vec<AuditEvent> {
    sink.events()
        .into_iter()
        .filter(|e| {
            detail_value(e, "audit-operation").as_deref() == Some("bulk-export")
                && detail_value(e, "bulk-export-operation").as_deref() == Some(op)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Bulk export
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bulk_export_kickoff_emits_audit_event() {
    let env = create_audited_server().await;
    seed_patients(&env.backend, 1).await;

    let resp = env
        .server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient")
        .await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);

    let kickoffs = export_events(&env.sink, "kickoff");
    assert_eq!(kickoffs.len(), 1, "exactly one kickoff event");
    let event = &kickoffs[0];
    assert_eq!(action_code(event).as_deref(), Some("E"));
    assert_eq!(outcome_code(event).as_deref(), Some("0"));
    assert_eq!(
        detail_value(event, "export-level").as_deref(),
        Some("system")
    );
    assert_eq!(
        detail_value(event, "resource-types").as_deref(),
        Some("Patient")
    );
    assert!(detail_value(event, "job-id").is_some(), "job-id captured");
}

#[tokio::test]
async fn bulk_export_status_unknown_job_emits_failure() {
    let env = create_audited_server().await;
    let resp = env
        .server
        .get("/export-status/nonexistent")
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

    let events = export_events(&env.sink, "status");
    assert_eq!(events.len(), 1);
    assert_eq!(outcome_code(&events[0]).as_deref(), Some("8"));
}

#[tokio::test]
async fn bulk_export_cancel_unknown_job_emits_failure() {
    let env = create_audited_server().await;
    let resp = env
        .server
        .delete("/export-status/nonexistent")
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

    let events = export_events(&env.sink, "delete");
    assert_eq!(events.len(), 1);
    assert_eq!(outcome_code(&events[0]).as_deref(), Some("8"));
}

#[tokio::test]
async fn bulk_export_download_unknown_file_emits_failure() {
    let env = create_audited_server().await;
    let resp = env
        .server
        .get("/export-file/nonexistent/Patient-0")
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

    let events = export_events(&env.sink, "download");
    assert_eq!(events.len(), 1);
    assert_eq!(outcome_code(&events[0]).as_deref(), Some("8"));
}

#[tokio::test]
async fn bulk_export_full_lifecycle_emits_lifecycle_events() {
    let env = create_audited_server().await;
    seed_patients(&env.backend, 2).await;

    let resp = env
        .server
        .get("/$export")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("prefer", "respond-async")
        .add_query_param("_type", "Patient")
        .await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);
    let status_path = resp
        .headers()
        .get("content-location")
        .unwrap()
        .to_str()
        .unwrap()
        .strip_prefix("http://localhost:8080")
        .unwrap()
        .to_string();

    // Run the worker to completion.
    drain_workers_with_audit(&env.backend, &env.output, &env.sink).await;

    // Worker-level completion event emitted.
    let worker_complete = export_events(&env.sink, "worker-complete");
    assert_eq!(worker_complete.len(), 1, "worker emits a completion event");
    assert_eq!(outcome_code(&worker_complete[0]).as_deref(), Some("0"));

    // Successful status poll (terminal "Complete" branch).
    let done = env
        .server
        .get(&status_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(done.status_code(), StatusCode::OK);
    let status_complete = export_events(&env.sink, "status-complete");
    assert_eq!(status_complete.len(), 1);
    assert_eq!(outcome_code(&status_complete[0]).as_deref(), Some("0"));

    // Delete the job.
    let deleted = env
        .server
        .delete(&status_path)
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(deleted.status_code(), StatusCode::ACCEPTED);
    let delete_events = export_events(&env.sink, "delete");
    assert!(
        delete_events
            .iter()
            .any(|e| outcome_code(e).as_deref() == Some("0")),
        "delete success event present"
    );
}

// ---------------------------------------------------------------------------
// $purge
// ---------------------------------------------------------------------------

#[tokio::test]
async fn purge_instance_success_emits_audit_event() {
    let env = create_audited_server().await;
    seed_patients(&env.backend, 1).await;

    let resp = env
        .server
        .delete("/Patient/p0/$purge")
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);

    let events = events_for_operation(&env.sink, "purge");
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(action_code(event).as_deref(), Some("D"));
    assert_eq!(outcome_code(event).as_deref(), Some("0"));
    assert_eq!(detail_value(event, "count").as_deref(), Some("1"));
}

#[tokio::test]
async fn purge_instance_unknown_resource_emits_failure() {
    let env = create_audited_server().await;
    let resp = env
        .server
        .delete("/Patient/missing/$purge")
        .add_header("x-tenant-id", "test-tenant")
        .await;
    // SqliteBackend::purge returns StorageError::Resource(NotFound), which
    // RestError::From<StorageError> maps to 404.
    assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

    let events = events_for_operation(&env.sink, "purge");
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(action_code(event).as_deref(), Some("D"));
    assert_eq!(outcome_code(event).as_deref(), Some("8"));
}

#[tokio::test]
async fn purge_type_success_emits_audit_event_with_count() {
    let env = create_audited_server().await;
    seed_patients(&env.backend, 3).await;

    let resp = env
        .server
        .post("/Patient/$purge")
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);

    let events = events_for_operation(&env.sink, "purge");
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(outcome_code(event).as_deref(), Some("0"));
    assert_eq!(detail_value(event, "count").as_deref(), Some("3"));
    assert_eq!(
        detail_value(event, "resource-type").as_deref(),
        Some("Patient")
    );
}

#[tokio::test]
async fn purge_audit_event_is_blocked() {
    let env = create_audited_server().await;
    let resp = env
        .server
        .delete("/AuditEvent/anything/$purge")
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(resp.status_code(), StatusCode::METHOD_NOT_ALLOWED);
    // No purge AuditEvent should be emitted for blocked AuditEvent purges.
    assert!(events_for_operation(&env.sink, "purge").is_empty());
}

// ---------------------------------------------------------------------------
// $reindex
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reindex_kickoff_emits_kickoff_and_start_events() {
    // Both events fire synchronously before `controller.start().await`
    // returns: the persistence-layer "start" event is recorded in
    // `ReindexOperation::start` *before* the background task is spawned, and
    // the REST "kickoff" event is recorded right after `start` resolves.
    // No timing slack is needed.
    let env = create_audited_server().await;
    seed_patients(&env.backend, 1).await;

    let resp = env
        .server
        .post("/$reindex")
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);

    let events = events_for_operation(&env.sink, "reindex");
    let kickoff = events
        .iter()
        .find(|e| detail_value(e, "phase").as_deref() == Some("kickoff"))
        .expect("REST handler emitted a kickoff event");
    assert_eq!(outcome_code(kickoff).as_deref(), Some("0"));

    let start = events
        .iter()
        .find(|e| detail_value(e, "phase").as_deref() == Some("start"))
        .expect("ReindexOperation emitted a start event");
    assert_eq!(outcome_code(start).as_deref(), Some("0"));
}

#[tokio::test]
async fn reindex_type_scoped_kickoff_records_resource_types() {
    let env = create_audited_server().await;
    seed_patients(&env.backend, 1).await;

    let resp = env
        .server
        .post("/Patient/$reindex")
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(resp.status_code(), StatusCode::ACCEPTED);

    let kickoffs: Vec<_> = events_for_operation(&env.sink, "reindex")
        .into_iter()
        .filter(|e| detail_value(e, "phase").as_deref() == Some("kickoff"))
        .collect();
    assert_eq!(kickoffs.len(), 1);
    assert_eq!(
        detail_value(&kickoffs[0], "resource-types").as_deref(),
        Some("Patient")
    );
}

#[tokio::test]
async fn reindex_cancel_unknown_job_emits_failure() {
    let env = create_audited_server().await;
    let resp = env
        .server
        .delete("/$reindex-status/nonexistent")
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

    let cancels: Vec<_> = events_for_operation(&env.sink, "reindex")
        .into_iter()
        .filter(|e| detail_value(e, "phase").as_deref() == Some("cancel"))
        .collect();
    assert_eq!(cancels.len(), 1);
    assert_eq!(outcome_code(&cancels[0]).as_deref(), Some("8"));
}

#[tokio::test]
async fn reindex_unavailable_when_controller_missing() {
    // Build a server WITHOUT the reindex controller and confirm the handler
    // returns 501 with no audit emission (the early bail beats the audit
    // path, mirroring bulk-export's "disabled" behavior).
    let backend_config = SqliteBackendConfig::default();
    let backend = Arc::new(
        SqliteBackend::with_config(":memory:", backend_config).expect("create SQLite backend"),
    );
    backend.init_schema().expect("init schema");

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    };
    let sink = Arc::new(InMemoryAuditSink::new());
    let state = helios_rest::AppState::with_auth_and_audit(
        Arc::clone(&backend),
        config,
        helios_auth::AuthConfig::default(),
        None,
        Some(sink.clone() as Arc<dyn helios_audit::AuditSink>),
        SOURCE_OBSERVER,
    );

    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let server = TestServer::new(app).expect("create test server");

    let resp = server
        .post("/$reindex")
        .add_header("x-tenant-id", "test-tenant")
        .await;
    assert_eq!(resp.status_code(), StatusCode::NOT_IMPLEMENTED);
    assert!(
        events_for_operation(&sink, "reindex").is_empty(),
        "no audit event when controller is absent"
    );
}

// ---------------------------------------------------------------------------
// `authz_middleware` deferral for `$`-suffixed operation endpoints
// ---------------------------------------------------------------------------

/// `authz_middleware` reads the [`Principal`] from request extensions and
/// never calls its provider — this stub only satisfies the type.
#[derive(Debug)]
struct UnreachableAuthProvider;

#[async_trait::async_trait]
impl AuthProvider for UnreachableAuthProvider {
    async fn authenticate(&self, _authorization_header: &str) -> Result<Principal, AuthError> {
        unreachable!("authz_middleware never calls the provider");
    }

    fn name(&self) -> &str {
        "test-unreachable"
    }
}

fn principal_with_scopes(scope_str: &str) -> Principal {
    Principal {
        subject: "test-subject".to_string(),
        issuer: "test-issuer".to_string(),
        tenant_id: Some("test-tenant".to_string()),
        scopes: ScopeSet::parse(scope_str),
        jti: None,
        expires_at: Utc::now() + chrono::Duration::hours(1),
        custom_claims: serde_json::Map::new(),
    }
}

async fn inject_principal_layer(mut request: Request, next: Next) -> Response {
    let scopes = request
        .extensions()
        .get::<TestScopes>()
        .cloned()
        .expect("TestScopes extension must be inserted before injection");
    request.extensions_mut().insert(principal_with_scopes(&scopes.0));
    next.run(request).await
}

#[derive(Clone)]
struct TestScopes(String);

async fn set_test_scopes_from_header(mut request: Request, next: Next) -> Response {
    let scopes = request
        .headers()
        .get("x-test-scopes")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    if let Some(s) = scopes {
        request.extensions_mut().insert(TestScopes(s));
    }
    next.run(request).await
}

async fn create_authz_layered_server() -> AuditedServer {
    let mut env = create_audited_server().await;

    let auth_state = Arc::new(AuthMiddlewareState {
        provider: Arc::new(UnreachableAuthProvider),
        config: Arc::new(AuthConfig::default()),
        audit_sink: env.sink.clone() as Arc<dyn helios_audit::AuditSink>,
        audit_source_observer: SOURCE_OBSERVER.to_string(),
        audit_exclusion_filter: ExclusionFilter::new(Vec::new()),
        tenant_url_routing: false,
    });

    // `TestServer` owns the router by value, so reconstruct the state instead
    // of swapping layers on the existing server.
    let backend = env.backend.clone();
    let output = env.output.clone();
    let sink = env.sink.clone();
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
    let reindex_op = ReindexOperation::new(backend.clone(), backend.search_extractor().clone())
        .with_audit(
            sink.clone() as Arc<dyn helios_audit::AuditSink>,
            SOURCE_OBSERVER,
        );

    let state = helios_rest::AppState::with_auth_and_audit(
        Arc::clone(&backend),
        config,
        helios_auth::AuthConfig::default(),
        None,
        Some(sink.clone() as Arc<dyn helios_audit::AuditSink>),
        SOURCE_OBSERVER,
    )
    .with_bulk_export(
        backend.clone() as Arc<dyn BulkExportJobStore>,
        output.clone() as Arc<dyn ExportOutputStore>,
        file_auth,
    )
    .with_purge_provider(backend.clone() as Arc<dyn PurgableStorage>)
    .with_reindex_controller(Arc::new(reindex_op) as Arc<dyn ReindexController>);

    // Axum applies the last-added `.layer` outermost, giving the flow
    // header → inject → authz → router — matching production, where
    // `authz_middleware` runs after `auth_middleware` has populated Principal.
    let app = helios_rest::routing::fhir_routes::create_routes(state)
        .layer(from_fn_with_state(auth_state, authz_middleware))
        .layer(from_fn(inject_principal_layer))
        .layer(from_fn(set_test_scopes_from_header));

    env.server = TestServer::new(app).expect("create authz-layered test server");
    env
}

/// Events emitted by `authz_middleware` itself (its `outcomeDesc` always
/// starts with `Granted:` or `Forbidden:`), separated from handler events.
fn middleware_authz_events(sink: &InMemoryAuditSink) -> Vec<AuditEvent> {
    sink.events()
        .into_iter()
        .filter(|e| {
            e.outcome_desc
                .as_ref()
                .and_then(|s| s.value.as_deref())
                .is_some_and(|desc| desc.starts_with("Granted:") || desc.starts_with("Forbidden:"))
        })
        .collect()
}

#[tokio::test]
async fn type_purge_reaches_handler_with_delete_only_scope() {
    let env = create_authz_layered_server().await;
    seed_patients(&env.backend, 2).await;

    let resp = env
        .server
        .post("/Patient/$purge")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("x-test-scopes", "patient/Patient.d")
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::OK,
        "least-privilege .d token must reach the $purge handler"
    );

    let purge_events = events_for_operation(&env.sink, "purge");
    assert_eq!(purge_events.len(), 1);
    assert_eq!(action_code(&purge_events[0]).as_deref(), Some("D"));
    assert_eq!(outcome_code(&purge_events[0]).as_deref(), Some("0"));

    let mw_events = middleware_authz_events(&env.sink);
    assert!(
        mw_events.is_empty(),
        "middleware must defer, got: {:?}",
        mw_events
            .iter()
            .map(|e| e.outcome_desc.as_ref().and_then(|s| s.value.clone()))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn instance_purge_reaches_handler_with_delete_only_scope() {
    let env = create_authz_layered_server().await;
    seed_patients(&env.backend, 1).await;

    let resp = env
        .server
        .delete("/Patient/p0/$purge")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("x-test-scopes", "patient/Patient.d")
        .await;
    assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);

    let purge_events = events_for_operation(&env.sink, "purge");
    assert_eq!(purge_events.len(), 1);
    assert_eq!(action_code(&purge_events[0]).as_deref(), Some("D"));

    assert!(middleware_authz_events(&env.sink).is_empty());
}

#[tokio::test]
async fn type_reindex_reaches_handler_with_update_only_scope() {
    let env = create_authz_layered_server().await;
    seed_patients(&env.backend, 1).await;

    let resp = env
        .server
        .post("/Patient/$reindex")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("x-test-scopes", "patient/Patient.u")
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::ACCEPTED,
        "least-privilege .u token must reach the $reindex handler"
    );

    let kickoffs: Vec<_> = events_for_operation(&env.sink, "reindex")
        .into_iter()
        .filter(|e| detail_value(e, "phase").as_deref() == Some("kickoff"))
        .collect();
    assert_eq!(kickoffs.len(), 1);
    assert_eq!(outcome_code(&kickoffs[0]).as_deref(), Some("0"));

    assert!(middleware_authz_events(&env.sink).is_empty());
}
