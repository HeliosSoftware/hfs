//! #2304: a single-resource write whose commit outcome stays unknown
//! (`TransactionError::CommitOutcomeUnknown`) reports no write event.
//!
//! The handler's `?` returns on the storage error before `write_event::report`,
//! and the write may not have happened, so neither the subscriptions engine nor
//! the dashboard's live counts hear of it. These tests pin that behaviour, which
//! the persistence README documents together with how each consumer catches up.
//! A MongoDB backend settles such an outcome with one read first and answers
//! `Ok` when the write is stored (#2304); that case is covered by the MongoDB
//! integration tests, because this crate's MongoDB harness is a standalone
//! mongod without transactions. The control in each test shows that an `Ok`
//! write does report its event.
//!
//! The primary is wrapped so that a write can be armed to fail the way a MongoDB
//! write with an unknown commit does; every other call delegates to SQLite.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_observability::dashboard_counters::DashboardCounters;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{CompositeConfig, CompositeStorage, DynStorage};
use helios_persistence::core::{
    BackendKind, ResourceStorage, WriteEvent, WriteKind, WriteObserver,
};
use helios_persistence::error::{StorageError, StorageResult, TransactionError};
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::StoredResource;
use helios_rest::ServerConfig;
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const TENANT: &str = "unknown-commit";

fn sqlite() -> SqliteBackend {
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
    backend
}

/// A primary whose writes answer `CommitOutcomeUnknown` while it is armed.
struct UnknownCommit {
    inner: SqliteBackend,
    armed: AtomicBool,
}

impl UnknownCommit {
    fn refuse_when_armed(&self) -> StorageResult<()> {
        if self.armed.load(Ordering::SeqCst) {
            Err(StorageError::Transaction(
                TransactionError::CommitOutcomeUnknown {
                    reason: "test".to_string(),
                },
            ))
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl ResourceStorage for UnknownCommit {
    fn backend_name(&self) -> &'static str {
        "unknown-commit"
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        self.refuse_when_armed()?;
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
        fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        self.refuse_when_armed()?;
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
        self.refuse_when_armed()?;
        self.inner.update(tenant, current, resource).await
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        self.refuse_when_armed()?;
        self.inner.delete(tenant, resource_type, id).await
    }

    // A wrapper around a real backend delegates; the trait's default is
    // read-compare-delete.
    async fn delete_versioned(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
    ) -> StorageResult<()> {
        self.refuse_when_armed()?;
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

/// Every event reported to the state's observer, in order.
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
    fn take(&self) -> Vec<WriteEvent> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
}

struct Harness {
    server: TestServer,
    storage: Arc<UnknownCommit>,
    counters: Arc<DashboardCounters>,
    recording: Arc<Recording>,
}

fn harness() -> Harness {
    let storage = Arc::new(UnknownCommit {
        inner: sqlite(),
        armed: AtomicBool::new(false),
    });

    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .build()
        .expect("composite config");
    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), storage.clone() as DynStorage);
    let composite = Arc::new(CompositeStorage::new(config, backends).expect("composite"));

    let state = helios_rest::AppState::new(
        composite,
        ServerConfig {
            base_url: "http://localhost:8080".to_string(),
            ..ServerConfig::for_testing()
        },
    );
    let recording = Arc::new(Recording::default());
    state.write_observer().subscribe(recording.clone());
    let counters = Arc::clone(state.dashboard_counters());
    let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
        .expect("create test server");
    Harness {
        server,
        storage,
        counters,
        recording,
    }
}

impl Harness {
    fn arm(&self) {
        self.storage.armed.store(true, Ordering::SeqCst);
    }

    fn disarm(&self) {
        self.storage.armed.store(false, Ordering::SeqCst);
    }

    fn live(&self, resource_type: &str) -> i64 {
        self.counters.live_delta(TENANT, resource_type)
    }
}

fn tenant_header() -> HeaderValue {
    HeaderValue::from_static(TENANT)
}

fn patient(family: &str) -> Value {
    json!({"resourceType": "Patient", "id": "p-1", "name": [{"family": family}]})
}

/// `(resource_type, live_delta, notice kind)` of each resource event.
fn resource_events(events: &[WriteEvent]) -> Vec<(String, i64, Option<WriteKind>)> {
    events
        .iter()
        .filter_map(|event| match event {
            WriteEvent::Resource(write) => Some((
                write.resource_type.clone(),
                write.live_delta,
                write.notice.as_ref().map(|notice| notice.kind),
            )),
            _ => None,
        })
        .collect()
}

/// The answer to a write whose commit outcome is unknown: a 500 that says so.
fn assert_unknown_outcome(response: &axum_test::TestResponse) {
    assert_eq!(
        response.status_code(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        response.text()
    );
    assert!(
        response.text().contains("commit outcome is unknown"),
        "{}",
        response.text()
    );
}

#[tokio::test]
async fn a_create_whose_commit_outcome_is_unknown_reports_no_write_event() {
    let h = harness();

    h.arm();
    let response = h
        .server
        .post("/Patient")
        .add_header(X_TENANT_ID, tenant_header())
        .json(&json!({"resourceType": "Patient", "name": [{"family": "Unknown"}]}))
        .await;
    assert_unknown_outcome(&response);
    assert!(
        resource_events(&h.recording.take()).is_empty(),
        "an unknown outcome reports nothing"
    );
    assert_eq!(h.live("Patient"), 0);

    // Control: the same write answered `Ok` runs its post-write steps.
    h.disarm();
    let response = h
        .server
        .post("/Patient")
        .add_header(X_TENANT_ID, tenant_header())
        .json(&json!({"resourceType": "Patient", "name": [{"family": "Known"}]}))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::CREATED,
        "{}",
        response.text()
    );
    assert_eq!(
        resource_events(&h.recording.take()),
        vec![("Patient".to_string(), 1, Some(WriteKind::Create))]
    );
    assert_eq!(h.live("Patient"), 1);
}

#[tokio::test]
async fn an_update_whose_commit_outcome_is_unknown_reports_no_write_event() {
    let h = harness();
    let seeded = h
        .server
        .put("/Patient/p-1")
        .add_header(X_TENANT_ID, tenant_header())
        .json(&patient("Seed"))
        .await;
    assert_eq!(
        seeded.status_code(),
        StatusCode::CREATED,
        "{}",
        seeded.text()
    );
    h.recording.take();
    assert_eq!(h.live("Patient"), 1);

    h.arm();
    let response = h
        .server
        .put("/Patient/p-1")
        .add_header(X_TENANT_ID, tenant_header())
        .json(&patient("Amended"))
        .await;
    assert_unknown_outcome(&response);
    assert!(
        resource_events(&h.recording.take()).is_empty(),
        "an unknown outcome reports nothing"
    );
    assert_eq!(h.live("Patient"), 1, "the live count did not move");

    // Control: the same update answered `Ok` reports an update event.
    h.disarm();
    let response = h
        .server
        .put("/Patient/p-1")
        .add_header(X_TENANT_ID, tenant_header())
        .json(&patient("Amended"))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    assert_eq!(
        resource_events(&h.recording.take()),
        vec![("Patient".to_string(), 0, Some(WriteKind::Update))]
    );
}

#[tokio::test]
async fn a_delete_whose_commit_outcome_is_unknown_reports_no_write_event() {
    let h = harness();
    let seeded = h
        .server
        .put("/Patient/p-1")
        .add_header(X_TENANT_ID, tenant_header())
        .json(&patient("Seed"))
        .await;
    assert_eq!(
        seeded.status_code(),
        StatusCode::CREATED,
        "{}",
        seeded.text()
    );
    h.recording.take();
    assert_eq!(h.live("Patient"), 1);

    h.arm();
    let response = h
        .server
        .delete("/Patient/p-1")
        .add_header(X_TENANT_ID, tenant_header())
        .await;
    assert_unknown_outcome(&response);
    assert!(
        resource_events(&h.recording.take()).is_empty(),
        "an unknown outcome reports nothing"
    );
    assert_eq!(h.live("Patient"), 1, "the live count did not move");

    // Control: the same delete answered `Ok` reports a delete event.
    h.disarm();
    let response = h
        .server
        .delete("/Patient/p-1")
        .add_header(X_TENANT_ID, tenant_header())
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::NO_CONTENT,
        "{}",
        response.text()
    );
    assert_eq!(
        resource_events(&h.recording.take()),
        vec![("Patient".to_string(), -1, Some(WriteKind::Delete))]
    );
    assert_eq!(h.live("Patient"), 0);
}
