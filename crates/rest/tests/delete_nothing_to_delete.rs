//! #2523: a delete with nothing to delete is a successful no-op.
//!
//! FHIR R4, R4B and R5 word it identically (http.html#delete):
//!
//! > Upon successful deletion, or if the resource does not exist at all, the
//! > server should return either a 200 OK if the response contains a payload,
//! > or a 204 No Content with no response payload
//!
//! > Performing this interaction on a resource that is already deleted has no
//! > effect, and the server should return either a 200 OK if the response
//! > contains a payload, or a 204 No Content with no response payload.
//!
//! HFS answered the same "nothing to delete" five ways: `404` for the instance
//! delete and a batch entry, `400` for the whole Bundle when a SQLite or
//! PostgreSQL transaction carried it, `204` for a conditional no-match and a
//! MongoDB transaction entry, and `410` on S3 for an already-deleted id. Every
//! path now answers the conditional no-match's `204` — or `200` with an
//! informational OperationOutcome under `Prefer: return=OperationOutcome`.
//!
//! "Has no effect" is pinned as well: a no-op writes no history version and no
//! tombstone, reports no write event (so no subscription notice and no live
//! count), and the audit trail records the request as a successful delete of
//! the addressed id and nothing more.
//!
//! What stays as it was is pinned elsewhere and named here so the boundary is
//! visible: `If-Match` on an absent or deleted resource is `412`
//! (`rest_conformance::conditional_delete::*_is_412`, and `ifMatch` in a
//! transaction below); `If-Match` losing a race is `409`
//! (`if_match_write_race::delete_with_if_match_does_not_delete_a_version_written_after_the_check`);
//! a read of a deleted resource is `410` (below, and
//! `rest_conformance::status_codes`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_audit::{AuditConfig, AuditMiddlewareState, AuditSink, ExclusionFilter};
use helios_fhir::r4::AuditEvent;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{CompositeConfig, CompositeStorage, DynStorage};
use helios_persistence::core::{BackendKind, ResourceStorage, WriteEvent, WriteObserver};
use helios_persistence::error::StorageResult;
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::StoredResource;
use helios_rest::ServerConfig;
use serde_json::{Value, json};

const PREFER: HeaderName = HeaderName::from_static("prefer");

/// Retains every recorded [`AuditEvent`], from the middleware and from the
/// Bundle handler's per-entry audit alike.
#[derive(Clone, Default)]
struct CollectorSink {
    events: Arc<Mutex<Vec<AuditEvent>>>,
}

#[async_trait]
impl AuditSink for CollectorSink {
    async fn record(&self, event: AuditEvent) {
        self.events.lock().unwrap().push(event);
    }
    async fn flush(&self) {}
    fn name(&self) -> &str {
        "collector"
    }
}

impl CollectorSink {
    /// As [`take`](Self::take), once at least `n` events have arrived. A
    /// Bundle entry's event is recorded from a spawned task, after the
    /// response.
    async fn take_at_least(&self, n: usize) -> Vec<(String, String, Vec<String>)> {
        for _ in 0..200 {
            if self.events.lock().unwrap().len() >= n {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        // Room for one too many to show up, which the caller would catch.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        self.take()
    }

    /// `(action, outcome, entity references)` of each event since the last
    /// call.
    fn take(&self) -> Vec<(String, String, Vec<String>)> {
        std::mem::take(&mut *self.events.lock().unwrap())
            .iter()
            .map(|event| {
                let event = serde_json::to_value(event).unwrap();
                let entities = event["entity"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e["what"]["reference"].as_str().map(str::to_string))
                    .collect();
                (
                    event["action"].as_str().unwrap_or_default().to_string(),
                    event["outcome"].as_str().unwrap_or_default().to_string(),
                    entities,
                )
            })
            .collect()
    }
}

/// Every write event reported to the state's observer — what feeds the
/// subscriptions engine and the dashboard's live counts.
#[derive(Default)]
struct Recording {
    events: Mutex<Vec<WriteEvent>>,
}

impl WriteObserver for Recording {
    fn on_write(&self, event: &WriteEvent) {
        self.events.lock().unwrap().push(event.clone());
    }
}

impl Recording {
    /// `(resource_type, live_delta, announces)` of each resource event since
    /// the last call.
    fn take(&self) -> Vec<(String, i64, bool)> {
        std::mem::take(&mut *self.events.lock().unwrap())
            .into_iter()
            .filter_map(|event| match event {
                WriteEvent::Resource(write) => Some((
                    write.resource_type.clone(),
                    write.live_delta,
                    write.notice.is_some(),
                )),
                _ => None,
            })
            .collect()
    }
}

struct Harness {
    server: TestServer,
    audit: CollectorSink,
    writes: Arc<Recording>,
}

/// A server over in-memory SQLite, audited by the real middleware and with a
/// write observer subscribed.
fn harness() -> Harness {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .canonicalize()
        .expect("repo data dir");
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("in-memory SQLite");
    backend.init_schema().expect("init schema");

    let audit = CollectorSink::default();
    let state = helios_rest::AppState::with_auth_and_audit(
        Arc::new(backend),
        ServerConfig {
            base_url: "http://localhost:8080".to_string(),
            ..ServerConfig::for_testing()
        },
        helios_auth::AuthConfig::default(),
        None,
        Some(Arc::new(audit.clone()) as Arc<dyn AuditSink>),
        "Device/hfs",
    );
    let writes = Arc::new(Recording::default());
    state.write_observer().subscribe(writes.clone());

    let middleware = Arc::new(AuditMiddlewareState {
        sink: Arc::new(audit.clone()),
        config: AuditConfig::default(),
        exclusion_filter: ExclusionFilter::new(Vec::new()),
    });
    let app = helios_rest::routing::fhir_routes::create_routes(state).layer(
        axum::middleware::from_fn_with_state(
            middleware,
            helios_audit::middleware::audit_middleware,
        ),
    );
    Harness {
        server: TestServer::new(app).expect("test server"),
        audit,
        writes,
    }
}

impl Harness {
    /// Stores `Observation/{id}` for `Patient/pat` and deletes it, so it is a
    /// tombstone with two history versions; then forgets the events that
    /// produced it.
    async fn seed_deleted(&self, id: &str) {
        self.server
            .put(&format!("/Observation/{id}"))
            .json(&json!({
                "resourceType": "Observation",
                "id": id,
                "status": "final",
                "code": {"text": "x"},
                "subject": {"reference": "Patient/pat"}
            }))
            .await
            .assert_status(StatusCode::CREATED);
        self.server
            .delete(&format!("/Observation/{id}"))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        self.forget();
    }

    fn forget(&self) {
        self.audit.take();
        self.writes.take();
    }

    /// `(versionId, lastModified)` of each history entry, newest first.
    async fn history(&self, path: &str) -> Vec<(String, String)> {
        let bundle: Value = self.server.get(&format!("{path}/_history")).await.json();
        bundle["entry"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| {
                (
                    e["response"]["etag"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    e["response"]["lastModified"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                )
            })
            .collect()
    }

    async fn bundle(&self, kind: &str, entries: Value) -> Value {
        let response = self
            .server
            .post("/")
            .json(&json!({"resourceType": "Bundle", "type": kind, "entry": entries}))
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{kind}: {}",
            response.text()
        );
        response.json()
    }
}

fn statuses(bundle: &Value) -> Vec<String> {
    bundle["entry"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|e| e["response"]["status"].as_str().unwrap().to_string())
        .collect()
}

fn delete_entry(url: &str) -> Value {
    json!({"request": {"method": "DELETE", "url": url}})
}

#[tokio::test]
async fn delete_of_a_resource_that_never_existed_is_204_and_leaves_no_tombstone() {
    let h = harness();

    for _ in 0..2 {
        let response = h.server.delete("/Observation/never-existed").await;
        assert_eq!(
            response.status_code(),
            StatusCode::NO_CONTENT,
            "{}",
            response.text()
        );
        assert!(response.text().is_empty(), "a 204 has no body");
    }

    // No tombstone was written: the id is still unknown, not deleted.
    h.server
        .get("/Observation/never-existed")
        .await
        .assert_status(StatusCode::NOT_FOUND);
    assert!(h.history("/Observation/never-existed").await.is_empty());
}

#[tokio::test]
async fn delete_of_an_already_deleted_resource_is_204_and_writes_nothing() {
    let h = harness();
    h.seed_deleted("obs").await;
    let before = h.history("/Observation/obs").await;
    assert_eq!(before.len(), 2, "create + delete: {before:?}");

    for _ in 0..2 {
        let response = h.server.delete("/Observation/obs").await;
        assert_eq!(
            response.status_code(),
            StatusCode::NO_CONTENT,
            "{}",
            response.text()
        );
        assert!(response.text().is_empty(), "a 204 has no body");
    }

    // "Has no effect": no new history version, no new tombstone, the same
    // `lastUpdated` — and a read is still `410 Gone`, unchanged.
    assert_eq!(h.history("/Observation/obs").await, before);
    h.server
        .get("/Observation/obs")
        .await
        .assert_status(StatusCode::GONE);
}

/// The client that needs to know asks for an OperationOutcome and gets `200`
/// with one `information` / `informational` issue saying nothing was deleted,
/// in place of the `404` it used to read that from.
#[tokio::test]
async fn prefer_operation_outcome_says_nothing_was_deleted() {
    let h = harness();
    h.seed_deleted("obs").await;

    for id in ["obs", "never-existed"] {
        let response = h
            .server
            .delete(&format!("/Observation/{id}"))
            .add_header(PREFER, HeaderValue::from_static("return=OperationOutcome"))
            .await;
        assert_eq!(response.status_code(), StatusCode::OK, "{id}");
        let outcome: Value = response.json();
        assert_eq!(outcome["resourceType"], "OperationOutcome", "{id}");
        let issues = outcome["issue"].as_array().unwrap();
        assert_eq!(issues.len(), 1, "{id}: {outcome}");
        assert_eq!(issues[0]["severity"], "information", "{id}");
        assert_eq!(issues[0]["code"], "informational", "{id}");
        assert_eq!(
            issues[0]["details"]["text"],
            format!(
                "Resource Observation/{id} does not exist or is already deleted; \
                 nothing was deleted"
            ),
        );
    }
}

/// No write event — so no subscription delete notice and no live-count change
/// — and an audit record of a successful delete of the addressed id, with no
/// patient taken from content there is none of. The control is the delete
/// that did remove something.
#[tokio::test]
async fn a_no_op_instance_delete_reports_no_write_and_audits_a_plain_success() {
    let h = harness();
    h.server
        .put("/Observation/live")
        .json(&json!({
            "resourceType": "Observation",
            "id": "live",
            "status": "final",
            "code": {"text": "x"},
            "subject": {"reference": "Patient/pat"}
        }))
        .await
        .assert_status(StatusCode::CREATED);
    h.forget();

    // Control: a delete that removes the resource is counted, announced, and
    // audited with the patient from its content.
    h.server
        .delete("/Observation/live")
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(h.writes.take(), vec![("Observation".to_string(), -1, true)]);
    assert_eq!(
        h.audit.take(),
        vec![(
            "D".to_string(),
            "0".to_string(),
            vec!["Observation/live".to_string(), "Patient/pat".to_string()]
        )]
    );

    // The same delete again, and one of an id never stored: nothing reported.
    for id in ["live", "never-existed"] {
        h.server
            .delete(&format!("/Observation/{id}"))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert!(h.writes.take().is_empty(), "{id}: no write event");
        assert_eq!(
            h.audit.take(),
            vec![(
                "D".to_string(),
                "0".to_string(),
                vec![format!("Observation/{id}")]
            )],
            "{id}"
        );
    }
}

#[tokio::test]
async fn a_batch_delete_with_nothing_to_delete_is_entry_204() {
    let h = harness();
    h.seed_deleted("obs").await;
    let before = h.history("/Observation/obs").await;
    h.forget();

    let bundle = h
        .bundle(
            "batch",
            json!([
                delete_entry("Observation/never-existed"),
                delete_entry("Observation/obs"),
            ]),
        )
        .await;
    assert_eq!(statuses(&bundle), ["204 No Content", "204 No Content"]);
    for entry in bundle["entry"].as_array().unwrap() {
        assert!(entry["response"].get("outcome").is_none(), "{entry}");
    }

    assert!(h.writes.take().is_empty(), "no write event");
    let mut audit = h.audit.take_at_least(2).await;
    audit.sort();
    assert_eq!(
        audit,
        [
            (
                "D".to_string(),
                "0".to_string(),
                vec!["Observation/never-existed".to_string()]
            ),
            (
                "D".to_string(),
                "0".to_string(),
                vec!["Observation/obs".to_string()]
            ),
        ]
    );
    assert_eq!(h.history("/Observation/obs").await, before);
}

/// The entry is `204` and the rest of the transaction commits; it used to fail
/// the whole Bundle with `400` on SQLite and PostgreSQL.
#[tokio::test]
async fn a_transaction_delete_with_nothing_to_delete_is_entry_204_and_the_rest_commits() {
    let h = harness();
    h.seed_deleted("obs").await;
    let before = h.history("/Observation/obs").await;
    h.forget();

    let bundle = h
        .bundle(
            "transaction",
            json!([
                delete_entry("Observation/never-existed"),
                delete_entry("Observation/obs"),
                {
                    "fullUrl": "Patient/t1",
                    "resource": {"resourceType": "Patient", "id": "t1"},
                    "request": {"method": "PUT", "url": "Patient/t1"}
                }
            ]),
        )
        .await;
    assert_eq!(
        statuses(&bundle),
        ["204 No Content", "204 No Content", "201 Created"]
    );

    // Only the PUT reports; neither delete announces or moves a count.
    assert_eq!(h.writes.take(), vec![("Patient".to_string(), 1, true)]);
    let mut deletes: Vec<_> = h
        .audit
        .take_at_least(3)
        .await
        .into_iter()
        .filter(|(action, _, _)| action == "D")
        .collect();
    deletes.sort();
    assert_eq!(
        deletes,
        [
            (
                "D".to_string(),
                "0".to_string(),
                vec!["Observation/never-existed".to_string()]
            ),
            (
                "D".to_string(),
                "0".to_string(),
                vec!["Observation/obs".to_string()]
            ),
        ]
    );

    h.server
        .get("/Patient/t1")
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(h.history("/Observation/obs").await, before);
    h.server
        .get("/Observation/never-existed")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

/// Unchanged: any other failing entry still fails the whole transaction (`400`,
/// naming the entry and its own status), and a no-op delete beside it does not
/// change that. Here the failure is `ifMatch` on a deleted resource, which is
/// still a `412` entry.
#[tokio::test]
async fn other_transaction_failures_still_fail_the_bundle() {
    let h = harness();
    h.seed_deleted("obs").await;

    let response = h
        .server
        .post("/")
        .json(&json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [
                delete_entry("Observation/never-existed"),
                {
                    "request": {"method": "DELETE", "url": "Observation/obs", "ifMatch": "W/\"2\""}
                },
                {
                    "fullUrl": "Patient/t2",
                    "resource": {"resourceType": "Patient", "id": "t2"},
                    "request": {"method": "PUT", "url": "Patient/t2"}
                }
            ]
        }))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::BAD_REQUEST,
        "{}",
        response.text()
    );
    assert!(
        response
            .text()
            .contains("Transaction failed at entry 1: Entry failed with status 412"),
        "{}",
        response.text()
    );
    h.server
        .get("/Patient/t2")
        .await
        .assert_status(StatusCode::NOT_FOUND);
    assert!(h.writes.take().is_empty(), "nothing committed");
}

/// Unchanged: `If-Match` on a resource that is absent or deleted is `412` — no
/// current representation satisfies a precondition — not the no-op `204`.
#[tokio::test]
async fn if_match_on_nothing_to_delete_is_still_412() {
    let h = harness();
    h.seed_deleted("obs").await;

    for (id, tag) in [
        ("obs", "W/\"2\""),
        ("obs", "*"),
        ("never-existed", "W/\"1\""),
        ("never-existed", "*"),
    ] {
        h.server
            .delete(&format!("/Observation/{id}"))
            .add_header(
                HeaderName::from_static("if-match"),
                HeaderValue::from_static(tag),
            )
            .await
            .assert_status(StatusCode::PRECONDITION_FAILED);
    }
    let bundle = h
        .bundle(
            "batch",
            json!([
                {"request": {"method": "DELETE", "url": "Observation/obs", "ifMatch": "W/\"2\""}},
                {"request": {"method": "DELETE", "url": "Observation/never-existed", "ifMatch": "*"}},
            ]),
        )
        .await;
    assert_eq!(
        statuses(&bundle),
        ["412 Precondition Failed", "412 Precondition Failed"]
    );
}

/// A primary whose plain `delete` loses a race while armed: another writer
/// deletes the resource after the handler's read saw it live, so this request's
/// own delete finds nothing to delete.
struct DeletedUnderneath {
    inner: SqliteBackend,
    armed: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl ResourceStorage for DeletedUnderneath {
    fn backend_name(&self) -> &'static str {
        "deleted-underneath"
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: helios_fhir::FhirVersion,
    ) -> StorageResult<StoredResource> {
        self.inner
            .create(tenant, resource_type, resource, fhir_version)
            .await
    }

    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: helios_fhir::FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        self.inner
            .create_or_update(tenant, resource_type, id, resource, fhir_version)
            .await
    }

    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        self.inner.read(tenant, resource_type, id).await
    }

    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        self.inner.update(tenant, current, resource).await
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        if self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            // The interloper's delete lands first.
            self.inner.delete(tenant, resource_type, id).await?;
        }
        self.inner.delete(tenant, resource_type, id).await
    }

    async fn delete_versioned(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
    ) -> StorageResult<()> {
        self.inner
            .delete_versioned(tenant, resource_type, id, expected_version)
            .await
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        self.inner.count(tenant, resource_type).await
    }
}

/// Without `If-Match`, a resource another writer deleted after this request's
/// read saw it live is the same no-op: the client's goal is reached, but not
/// by this request, so it reports no write event — the interloper's delete is
/// the one that removed it. (With `If-Match` that race keeps its old error;
/// `if_match_write_race` pins it.)
#[tokio::test]
async fn a_delete_that_loses_a_race_without_if_match_is_a_no_op() {
    let storage = Arc::new(DeletedUnderneath {
        inner: SqliteBackend::in_memory().expect("in-memory SQLite"),
        armed: std::sync::atomic::AtomicBool::new(false),
    });
    storage.inner.init_schema().expect("init schema");
    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .build()
        .expect("composite config");
    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), storage.clone() as DynStorage);
    let state = helios_rest::AppState::new(
        Arc::new(CompositeStorage::new(config, backends).expect("composite")),
        ServerConfig {
            base_url: "http://localhost:8080".to_string(),
            ..ServerConfig::for_testing()
        },
    );
    let writes = Arc::new(Recording::default());
    state.write_observer().subscribe(writes.clone());
    let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
        .expect("test server");

    server
        .put("/Patient/raced")
        .json(&json!({"resourceType": "Patient", "id": "raced"}))
        .await
        .assert_status(StatusCode::CREATED);
    writes.take();

    storage
        .armed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let response = server
        .delete("/Patient/raced")
        .add_header(PREFER, HeaderValue::from_static("return=OperationOutcome"))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let outcome: Value = response.json();
    assert_eq!(
        outcome["issue"][0]["details"]["text"],
        "Resource Patient/raced does not exist or is already deleted; nothing was deleted"
    );
    assert!(writes.take().is_empty(), "this request deleted nothing");
    server
        .get("/Patient/raced")
        .await
        .assert_status(StatusCode::GONE);
}
