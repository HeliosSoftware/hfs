//! End-to-end tests for `$purge` and `$reindex`, and for the `AuditEvent`s the
//! persistence-layer operations emit (issue #168).
//!
//! These drive the real REST stack — routing, the auth middleware's scope
//! classification, the handlers, and a real SQLite backend — rather than calling
//! the handlers directly, because several of the bugs being guarded against here
//! live in the routing and middleware layers, not in the handler bodies.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum_test::TestServer;
use chrono::Utc;
use helios_audit::AuditSink;
use helios_auth::Principal;
use helios_auth::scope::ScopeSet;
use helios_fhir::FhirVersion;
use helios_fhir::r4::{AuditEvent, AuditEventEntityDetailValue};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{
    HistoryParams, InstanceHistoryProvider, PurgableStorage, ResourceStorage,
};
use helios_persistence::error::{ResourceError, StorageError};
use helios_persistence::search::ReindexOperation;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Test-only audit sink
//
// Deliberately defined here, in test code, rather than shipped as a sink in the
// production `helios-audit` crate. A test double is not a product feature, and a
// cargo-feature-gated one would be compiled into the release binary by CI's
// `--all-features --release` build.
// ---------------------------------------------------------------------------

/// Retains every recorded [`AuditEvent`] so tests can assert on them.
#[derive(Clone, Default)]
struct CollectorSink {
    events: Arc<Mutex<Vec<AuditEvent>>>,
}

impl CollectorSink {
    fn new() -> Self {
        Self::default()
    }

    fn events(&self) -> Vec<AuditEvent> {
        self.events.lock().expect("collector poisoned").clone()
    }

    /// Every event carrying the given `audit-operation` detail.
    fn by_operation(&self, operation: &str) -> Vec<AuditEvent> {
        self.events()
            .into_iter()
            .filter(|e| detail_map(e).get("audit-operation").map(String::as_str) == Some(operation))
            .collect()
    }
}

#[async_trait]
impl AuditSink for CollectorSink {
    async fn record(&self, event: AuditEvent) {
        self.events.lock().expect("collector poisoned").push(event);
    }
    async fn flush(&self) {}
    fn name(&self) -> &str {
        "collector"
    }
}

/// Flattens an event's `entity[].detail[]` string values into a lookup map.
fn detail_map(event: &AuditEvent) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for entity in event.entity.as_ref().into_iter().flatten() {
        for detail in entity.detail.as_ref().into_iter().flatten() {
            let Some(key) = detail.r#type.value.clone() else {
                continue;
            };
            if let Some(AuditEventEntityDetailValue::String(s)) = &detail.value {
                map.insert(key, s.value.clone().unwrap_or_default());
            }
        }
    }
    map
}

/// Pulls one named value out of a `Parameters` body.
fn param_string(body: &Value, name: &str, value_key: &str) -> Option<String> {
    body["parameter"]
        .as_array()?
        .iter()
        .find(|p| p["name"] == name)?
        .get(value_key)?
        .as_str()
        .map(str::to_string)
}

fn outcome_of(event: &AuditEvent) -> Option<String> {
    event
        .outcome
        .as_ref()
        .and_then(|o| o.value.as_ref())
        .cloned()
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
}

/// An in-memory backend with the *full* search-parameter registry loaded.
///
/// `data_dir` must be set explicitly. It defaults to `./data`, resolved against
/// the *cwd* — `crates/rest/` under `cargo test -p helios-rest`, where no such
/// directory exists. The registry would then hold only the 5 embedded params
/// (`_id`, `_lastUpdated`, `_tag`, `_profile`, `_security`), so `?name=` would
/// match nothing and the reindex test below would pass without ever having
/// indexed, un-indexed, or re-indexed anything observable.
fn test_backend() -> Arc<SqliteBackend> {
    let config = SqliteBackendConfig {
        data_dir: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../data")
                .canonicalize()
                .expect("repo data dir"),
        ),
        ..Default::default()
    };
    let backend = SqliteBackend::with_config(":memory:", config).expect("in-memory SQLite");
    backend.init_schema().expect("init schema");
    Arc::new(backend)
}

/// Builds a server with `$purge` and `$reindex` wired, plus a capturing sink.
///
/// Auth is not configured here, so there is no `Principal` and the scope checks
/// pass — the same behavior as any other endpoint with auth disabled. Scope
/// enforcement is covered separately by [`server_with_principal`].
fn server_with_ops() -> (TestServer, Arc<SqliteBackend>, CollectorSink) {
    let backend = test_backend();
    let sink = CollectorSink::new();
    let config = ServerConfig {
        base_url: "http://localhost:8080".to_string(),
        ..ServerConfig::for_testing()
    };

    // `.with_audit` mirrors what `hfs`'s `wire_reindex` does at start-up. The
    // reindex driver emits its own lifecycle events from the background job, so
    // it needs the sink directly — unlike purge, which the handler audits.
    let reindex = Arc::new(
        ReindexOperation::new(backend.clone(), backend.search_extractor().clone())
            .with_audit(Arc::new(sink.clone()) as Arc<dyn AuditSink>, "Device/hfs"),
    );

    let state = helios_rest::AppState::with_auth_and_audit(
        backend.clone(),
        config,
        helios_auth::AuthConfig::default(),
        None,
        Some(Arc::new(sink.clone()) as Arc<dyn AuditSink>),
        "Device/hfs",
    )
    .with_purge(backend.clone() as Arc<dyn PurgableStorage>)
    .with_reindex(reindex);

    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let server = TestServer::new(app).expect("create test server");
    (server, backend, sink)
}

/// Builds the same server, but with every request carrying an authenticated
/// `Principal` holding exactly `scopes`.
///
/// This is what proves the authorization chain end to end. The auth middleware
/// deliberately declines to classify `$purge` / `$reindex` (it would otherwise
/// read `POST /Patient/$purge` as a *Create* on Patient and grant it to anyone
/// with `Patient.c`), so the handler's own scope check is the *only* thing
/// standing between a token and irreversible deletion. A handler that forgot to
/// look the principal up in the request extensions would silently authorize
/// everyone, and every unit test of `check_purge_scope` would still pass.
fn server_with_principal(scopes: &str) -> (TestServer, Arc<SqliteBackend>) {
    let backend = test_backend();

    let reindex = Arc::new(ReindexOperation::new(
        backend.clone(),
        backend.search_extractor().clone(),
    ));
    let state = helios_rest::AppState::new(
        backend.clone(),
        ServerConfig {
            base_url: "http://localhost:8080".to_string(),
            ..ServerConfig::for_testing()
        },
    )
    .with_purge(backend.clone() as Arc<dyn PurgableStorage>)
    .with_reindex(reindex);

    let principal = Principal {
        subject: "client-1".to_string(),
        issuer: "https://idp.example.com".to_string(),
        tenant_id: Some("test-tenant".to_string()),
        scopes: ScopeSet::parse(scopes),
        jti: None,
        expires_at: Utc::now() + chrono::Duration::hours(1),
        custom_claims: serde_json::Map::new(),
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

    (
        TestServer::new(app).expect("create test server"),
        backend.clone(),
    )
}

/// Number of matches for a search on an *indexed* field, so it goes to zero
/// when the search index is dropped.
///
/// Counts `Bundle.entry` rather than reading `Bundle.total`: `total` is only
/// populated when the caller asks for it (`_total`), and is `null` otherwise —
/// which would read as "no matches" for every search and make this helper
/// useless as an assertion.
async fn search_matches(server: &TestServer) -> usize {
    let response = server.get("/Patient?name=Purgeable").await;
    response.assert_status_ok();
    response.json::<Value>()["entry"]
        .as_array()
        .map_or(0, Vec::len)
}

async fn seed(backend: &SqliteBackend, id: &str) {
    backend
        .create(
            &tenant(),
            "Patient",
            json!({"resourceType": "Patient", "id": id, "name": [{"family": "Purgeable"}]}),
            FhirVersion::default(),
        )
        .await
        .expect("seed patient");
}

// ---------------------------------------------------------------------------
// $purge
// ---------------------------------------------------------------------------

/// A soft `DELETE` tombstones the resource — it stays in storage and reads as
/// `Gone` (410). `$purge` removes the bytes, so the read becomes an ordinary
/// miss. That difference is the entire reason the operation exists.
#[tokio::test]
async fn test_purge_removes_the_resource_and_its_history() {
    let (server, backend, _sink) = server_with_ops();
    seed(&backend, "p1").await;

    server.delete("/Patient/p1").await;
    assert!(
        matches!(
            backend.read(&tenant(), "Patient", "p1").await,
            Err(StorageError::Resource(ResourceError::Gone { .. }))
        ),
        "a soft DELETE must keep the resource as a tombstone, reading as Gone"
    );

    // The tombstone is still in history, so a vread of v1 still resolves.
    let history_before = backend
        .history_instance(&tenant(), "Patient", "p1", &HistoryParams::default())
        .await
        .expect("history before purge");
    assert!(
        !history_before.items.is_empty(),
        "a soft-deleted resource keeps its history"
    );

    server
        .delete("/Patient/p1/$purge")
        .await
        .assert_status(StatusCode::OK);

    assert!(
        matches!(
            backend.read(&tenant(), "Patient", "p1").await,
            Ok(None) | Err(StorageError::Resource(ResourceError::NotFound { .. }))
        ),
        "$purge must remove the resource entirely, not tombstone it"
    );

    let history_after = backend
        .history_instance(&tenant(), "Patient", "p1", &HistoryParams::default())
        .await
        .map(|h| h.items.len())
        .unwrap_or(0);
    assert_eq!(
        history_after, 0,
        "$purge must remove the history too — a purged resource that is still \
         readable via _history has not been erased"
    );
}

/// The purge route must not be shadowed by `/{resource_type}/{id}`, which would
/// route `DELETE /Patient/p1/$purge` as a delete of a resource whose id is
/// literally "$purge".
#[tokio::test]
async fn test_purge_of_missing_resource_is_404_not_a_stray_route() {
    let (server, _backend, _sink) = server_with_ops();
    let response = server.delete("/Patient/does-not-exist/$purge").await;
    response.assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_purge_type_removes_every_resource_of_that_type() {
    let (server, backend, _sink) = server_with_ops();
    seed(&backend, "p1").await;
    seed(&backend, "p2").await;
    seed(&backend, "p3").await;

    let response = server.post("/Patient/$purge").await;
    response.assert_status(StatusCode::OK);

    let remaining = backend
        .count(&tenant(), Some("Patient"))
        .await
        .expect("count after purge_all");
    assert_eq!(remaining, 0, "$purge on a type must purge every resource");
}

/// No scope grants the erasure of the audit trail. A caller who can purge
/// `AuditEvent` can erase the record of everything else they did.
#[tokio::test]
async fn test_purge_of_audit_event_is_refused() {
    let (server, _backend, _sink) = server_with_ops();

    server
        .delete("/AuditEvent/any/$purge")
        .await
        .assert_status(StatusCode::FORBIDDEN);
    server
        .post("/AuditEvent/$purge")
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

/// The purge must be auditable, with the `D` action code and a count.
#[tokio::test]
async fn test_purge_emits_an_audit_event() {
    let (server, backend, sink) = server_with_ops();
    seed(&backend, "p1").await;

    server
        .delete("/Patient/p1/$purge")
        .await
        .assert_status(StatusCode::OK);

    let purges = sink.by_operation("purge");
    assert_eq!(purges.len(), 1, "exactly one purge event");

    let event = &purges[0];
    assert_eq!(
        event.action.as_ref().and_then(|a| a.value.as_deref()),
        Some("D"),
        "a purge is a Delete, and BALP consumers filter on that code"
    );
    assert_eq!(outcome_of(event).as_deref(), Some("0"));
    assert_eq!(
        detail_map(event).get("count").map(String::as_str),
        Some("1")
    );
}

// ---------------------------------------------------------------------------
// $reindex
// ---------------------------------------------------------------------------

/// The whole point of `$reindex`: after the search index is dropped, search
/// stops working, and reindexing brings it back.
#[tokio::test]
async fn test_reindex_rebuilds_the_search_index() {
    use helios_persistence::search::ReindexTarget;

    let (server, backend, sink) = server_with_ops();
    seed(&backend, "p1").await;

    // Guard against a vacuous pass: if `name` were not a registered search
    // parameter, every search below would return 0 and the test would "prove"
    // the rebuild worked by never observing anything at all.
    assert_eq!(
        search_matches(&server).await,
        1,
        "search must work before the index is dropped"
    );

    // Drop the index out from under the resource.
    backend
        .clear_search_index(&tenant())
        .await
        .expect("clear index");
    assert_eq!(
        search_matches(&server).await,
        0,
        "search must be broken once the index is gone"
    );

    let kickoff = server.post("/$reindex").await;
    kickoff.assert_status(StatusCode::ACCEPTED);
    let job_id = param_string(&kickoff.json::<Value>(), "jobId", "valueString").expect("job id");

    // The rebuild runs in the background, so poll rather than assuming.
    let mut final_status = String::new();
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let status = server.get(&format!("/$reindex-status/{job_id}")).await;
        if status.status_code() != StatusCode::OK {
            continue;
        }
        let state =
            param_string(&status.json::<Value>(), "status", "valueCode").unwrap_or_default();
        if state == "completed" || state == "failed" || state == "cancelled" {
            final_status = state;
            break;
        }
    }
    assert_eq!(final_status, "completed", "reindex job did not complete");

    assert_eq!(
        search_matches(&server).await,
        1,
        "search must work again once the index is rebuilt"
    );

    // A start event and a terminal event, so an operator can see both that a
    // rebuild was requested and how it ended.
    let events = sink.by_operation("reindex");
    let phases: Vec<String> = events
        .iter()
        .filter_map(|e| detail_map(e).get("phase").cloned())
        .collect();
    assert!(
        phases.contains(&"start".to_string()),
        "expected a start event, got {phases:?}"
    );
    assert!(
        phases.contains(&"complete".to_string()),
        "expected a terminal complete event, got {phases:?}"
    );
}

// ---------------------------------------------------------------------------
// Authorization, end to end
// ---------------------------------------------------------------------------

/// Full CRUD on Patient — including delete — must not authorize a purge. A
/// token that may soft-delete a Patient must not thereby be able to destroy it
/// and its entire history irrecoverably.
#[tokio::test]
async fn test_purge_is_refused_without_the_purge_scope() {
    let (server, backend) = server_with_principal("system/Patient.cruds");
    seed(&backend, "p1").await;

    server
        .delete("/Patient/p1/$purge")
        .await
        .assert_status(StatusCode::FORBIDDEN);
    server
        .post("/Patient/$purge")
        .await
        .assert_status(StatusCode::FORBIDDEN);

    // And the refusal actually protected the data.
    assert!(
        backend
            .read(&tenant(), "Patient", "p1")
            .await
            .expect("read after refused purge")
            .is_some(),
        "a refused purge must leave the resource intact"
    );
}

#[tokio::test]
async fn test_purge_is_allowed_with_the_purge_scope() {
    let (server, backend) = server_with_principal("system/purge");
    seed(&backend, "p1").await;

    server
        .delete("/Patient/p1/$purge")
        .await
        .assert_status(StatusCode::OK);
}

/// Ordinary write scope must not authorize a full search-index rebuild.
#[tokio::test]
async fn test_reindex_is_refused_without_the_reindex_scope() {
    let (server, _backend) = server_with_principal("system/Patient.cruds");

    server
        .post("/$reindex")
        .await
        .assert_status(StatusCode::FORBIDDEN);
    server
        .post("/Patient/$reindex")
        .await
        .assert_status(StatusCode::FORBIDDEN);
    server
        .get("/$reindex-status/any")
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_reindex_is_allowed_with_the_reindex_scope() {
    let (server, _backend) = server_with_principal("system/reindex");

    server
        .post("/$reindex")
        .await
        .assert_status(StatusCode::ACCEPTED);
}

#[tokio::test]
async fn test_reindex_status_of_unknown_job_is_404() {
    let (server, _backend, _sink) = server_with_ops();
    server
        .get("/$reindex-status/no-such-job")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

/// The type-scoped route must not be shadowed by `/{resource_type}/{id}`.
#[tokio::test]
async fn test_reindex_type_route_is_reachable() {
    let (server, backend, _sink) = server_with_ops();
    seed(&backend, "p1").await;

    server
        .post("/Patient/$reindex")
        .await
        .assert_status(StatusCode::ACCEPTED);
}
