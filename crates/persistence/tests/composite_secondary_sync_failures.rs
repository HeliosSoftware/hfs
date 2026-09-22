//! #1334: a composite write succeeds once the primary has committed it, in
//! every sync mode, even when a secondary refuses the change after retries.
//! That refusal must not be silent: it is counted, reported, and recorded
//! durably as "needs reindex" so the resource can be repaired.
//!
//! The secondary here is a SQLite backend behind a switch that makes every
//! indexing write fail, so "the secondary is down" is a fact of the test
//! rather than a race. Asynchronous outcomes are awaited through the sync
//! queue's barrier (`ensure_writes_visible`), never by sleeping.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, RetryConfig,
    SecondarySyncFailure, SecondarySyncFailureLedger, SecondarySyncObserver, SyncConfig, SyncMode,
    SyncOperation,
};
use helios_persistence::core::search::{SearchProvider, SearchResult};
use helios_persistence::core::{BackendKind, ResourceStorage};
use helios_persistence::error::{BackendError, StorageError, StorageResult};
use helios_persistence::search::SearchParameterRegistry;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchQuery, SearchValue, StoredResource,
};
use serde_json::{Value, json};

/// Retries the sync manager makes after the first attempt.
const MAX_RETRIES: u32 = 2;
/// Attempts one final failure therefore stands for.
const ATTEMPTS: u64 = MAX_RETRIES as u64 + 1;

const MODES: [SyncMode; 4] = [
    SyncMode::Synchronous,
    SyncMode::Asynchronous,
    SyncMode::Hybrid {
        sync_for_search: true,
    },
    SyncMode::Hybrid {
        sync_for_search: false,
    },
];

fn tenant() -> TenantContext {
    TenantContext::new(TenantId::new("default"), TenantPermissions::full_access())
}

fn sqlite_at(path: &str) -> SqliteBackend {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .expect("workspace data dir");
    let backend = SqliteBackend::with_config(
        path,
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("sqlite");
    backend.init_schema().expect("schema");
    backend
}

/// A search secondary that can be switched off: while `failing`, every
/// indexing write is refused the way an unreachable cluster refuses it.
struct FlakyIndex {
    inner: SqliteBackend,
    failing: AtomicBool,
    delete_calls: AtomicUsize,
}

impl FlakyIndex {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: sqlite_at(":memory:"),
            failing: AtomicBool::new(false),
            delete_calls: AtomicUsize::new(0),
        })
    }

    fn set_failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }

    fn refuse(&self) -> StorageResult<()> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(StorageError::Backend(BackendError::Unavailable {
                backend_name: "flaky-index".to_string(),
                message: "index is down".to_string(),
            }));
        }
        Ok(())
    }

    async fn holds(&self, resource_type: &str, id: &str) -> Option<StoredResource> {
        self.inner
            .read(&tenant(), resource_type, id)
            .await
            .unwrap_or(None)
    }
}

#[async_trait]
impl ResourceStorage for FlakyIndex {
    fn backend_name(&self) -> &'static str {
        "flaky-index"
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        self.refuse()?;
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
        self.refuse()?;
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
        self.refuse()?;
        self.inner.update(tenant, current, resource).await
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        self.delete_calls.fetch_add(1, Ordering::SeqCst);
        self.refuse()?;
        self.inner.delete(tenant, resource_type, id).await
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        self.inner.count(tenant, resource_type).await
    }
}

#[async_trait]
impl SearchProvider for FlakyIndex {
    async fn search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        self.inner.search(tenant, query).await
    }

    async fn search_count(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<u64> {
        self.inner.search_count(tenant, query).await
    }

    fn search_param_registry(
        &self,
        tenant: &TenantContext,
    ) -> Arc<parking_lot::RwLock<SearchParameterRegistry>> {
        self.inner.search_param_registry(tenant)
    }
}

/// What the server's metrics exporter would have been told.
#[derive(Default)]
struct Metrics {
    failures: Mutex<Vec<(String, SyncOperation)>>,
    needs_reindex: AtomicU64,
}

impl Metrics {
    fn failures(&self) -> Vec<(String, SyncOperation)> {
        self.failures.lock().unwrap().clone()
    }

    fn needs_reindex(&self) -> u64 {
        self.needs_reindex.load(Ordering::SeqCst)
    }
}

impl SecondarySyncObserver for Metrics {
    fn sync_failed(&self, backend_id: &str, operation: SyncOperation) {
        self.failures
            .lock()
            .unwrap()
            .push((backend_id.to_string(), operation));
    }

    fn needs_reindex(&self, outstanding: u64) {
        self.needs_reindex.store(outstanding, Ordering::SeqCst);
    }
}

struct Rig {
    composite: CompositeStorage,
    primary: Arc<SqliteBackend>,
    index: Arc<FlakyIndex>,
    metrics: Arc<Metrics>,
}

impl Rig {
    /// Everything queued before this call has reached the index (or failed
    /// for good). A no-op wait under synchronous sync.
    async fn settle(&self) {
        self.composite
            .ensure_writes_visible(&tenant(), &["Organization"])
            .await
            .expect("sync barrier");
    }

    async fn records(&self) -> Vec<SecondarySyncFailure> {
        self.primary.list_sync_failures(100).await.expect("ledger")
    }

    async fn record_for(&self, id: &str) -> Option<SecondarySyncFailure> {
        self.records()
            .await
            .into_iter()
            .find(|record| record.key.resource_id == id)
    }

    async fn found_by_search(&self, identifier: &str) -> usize {
        self.composite
            .search(&tenant(), &by_identifier(identifier))
            .await
            .expect("search through composite")
            .resources
            .items
            .len()
    }
}

/// A production-shaped composite: primary with its own index offloaded, a
/// dedicated search secondary, the primary as the failure ledger.
fn rig_on(mode: SyncMode, primary_path: &str, with_ledger: bool) -> Rig {
    let mut primary = sqlite_at(primary_path);
    primary.set_search_offloaded(true);
    let primary = Arc::new(primary);
    let index = FlakyIndex::new();
    let metrics = Arc::new(Metrics::default());

    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("search", BackendKind::Sqlite)
        .with_sync_config(SyncConfig {
            mode,
            retry: RetryConfig {
                max_retries: MAX_RETRIES,
                initial_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(2),
                backoff_multiplier: 1.0,
            },
            ..Default::default()
        })
        .build()
        .expect("composite config");

    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    backends.insert("search".to_string(), index.clone() as DynStorage);
    let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
    providers.insert("sqlite".to_string(), primary.clone() as DynSearchProvider);
    providers.insert("search".to_string(), index.clone() as DynSearchProvider);

    let mut composite = CompositeStorage::new(config, backends)
        .expect("composite")
        .with_search_providers(providers)
        .with_full_primary(primary.clone())
        .with_sync_observer(metrics.clone());
    if with_ledger {
        composite = composite.with_sync_failure_ledger(primary.clone());
    }

    Rig {
        composite: composite.start_sync_workers(),
        primary,
        index,
        metrics,
    }
}

fn rig(mode: SyncMode) -> Rig {
    rig_on(mode, ":memory:", true)
}

fn organization(identifier: &str, name: &str) -> Value {
    json!({
        "resourceType": "Organization",
        "identifier": [{"system": "urn:zzz:probe", "value": identifier}],
        "name": name
    })
}

fn by_identifier(identifier: &str) -> SearchQuery {
    SearchQuery::new("Organization").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(Some("urn:zzz:probe"), identifier)],
        chain: vec![],
        components: vec![],
    })
}

/// The ledger contract PostgreSQL and MongoDB are held to as well.
#[path = "common/sync_failure_ledger_suite.rs"]
mod sync_failure_ledger_suite;

#[tokio::test]
async fn sqlite_sync_failure_ledger_contract() {
    let backend = sqlite_at(":memory:");
    sync_failure_ledger_suite::ledger_folds_orders_clears_and_counts(&backend, "ledger-1334").await;
}

/// The whole contract, in every sync mode: create, update and delete against
/// a secondary that is down all succeed, each final failure is counted exactly
/// once, one record per resource says what is owed, a later successful write
/// clears its record, and the repair drains the rest.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_syncs_are_counted_recorded_cleared_and_repaired_in_every_mode() {
    for mode in MODES {
        let rig = rig(mode);
        let t = tenant();

        // Positive control: a healthy secondary indexes the write, and
        // nothing is counted or recorded.
        let kept = rig
            .composite
            .create(
                &t,
                "Organization",
                organization("KEPT", "v1"),
                FhirVersion::R4,
            )
            .await
            .expect("healthy create");
        rig.settle().await;
        assert_eq!(rig.found_by_search("KEPT").await, 1, "{mode:?}: control");
        assert!(rig.metrics.failures().is_empty(), "{mode:?}");
        assert!(rig.records().await.is_empty(), "{mode:?}");

        // The secondary goes down. Every write still succeeds.
        rig.index.set_failing(true);
        let lost = rig
            .composite
            .create(
                &t,
                "Organization",
                organization("LOST", "v1"),
                FhirVersion::R4,
            )
            .await
            .unwrap_or_else(|e| panic!("{mode:?}: create must succeed: {e}"));
        rig.settle().await;
        assert_eq!(
            rig.metrics.failures(),
            vec![("search".to_string(), SyncOperation::Create)],
            "{mode:?}: one final failure, counted once (not once per retry)"
        );
        let record = rig.record_for(lost.id()).await.expect("create recorded");
        assert_eq!(record.key.tenant_id, "default");
        assert_eq!(record.key.resource_type, "Organization");
        assert_eq!(record.key.backend_id, "search");
        assert_eq!(record.operation, SyncOperation::Create);
        assert_eq!(record.attempts, ATTEMPTS, "{mode:?}");
        assert!(record.last_error.contains("index is down"), "{mode:?}");
        assert!(
            !record.last_error.contains("urn:zzz:probe"),
            "no resource content on the record"
        );
        assert_eq!(rig.found_by_search("LOST").await, 0, "{mode:?}: it is lost");

        let updated = rig
            .composite
            .update(&t, &kept, organization("KEPT", "v2"))
            .await
            .unwrap_or_else(|e| panic!("{mode:?}: update must succeed: {e}"));
        rig.settle().await;
        assert_eq!(updated.version_id(), "2");
        assert_eq!(rig.metrics.failures().len(), 2, "{mode:?}");
        assert_eq!(
            rig.metrics.failures()[1],
            ("search".to_string(), SyncOperation::Update)
        );
        let record = rig.record_for(kept.id()).await.expect("update recorded");
        assert_eq!(record.operation, SyncOperation::Update);

        rig.composite
            .delete(&t, "Organization", kept.id())
            .await
            .unwrap_or_else(|e| panic!("{mode:?}: delete must succeed: {e}"));
        rig.settle().await;
        assert_eq!(rig.metrics.failures().len(), 3, "{mode:?}");
        assert_eq!(
            rig.metrics.failures()[2],
            ("search".to_string(), SyncOperation::Delete)
        );
        // Same resource, same secondary: folded into the record it had.
        assert_eq!(rig.records().await.len(), 2, "{mode:?}: one per resource");
        let folded = rig.record_for(kept.id()).await.expect("still recorded");
        assert_eq!(folded.operation, SyncOperation::Delete);
        assert_eq!(folded.attempts, 2 * ATTEMPTS, "{mode:?}");
        assert_eq!(folded.first_failed_at, record.first_failed_at, "{mode:?}");
        assert_eq!(rig.metrics.needs_reindex(), 2, "{mode:?}: gauge");

        // The secondary comes back. A successful write of a recorded
        // resource delivers its current state, so its record is cleared.
        rig.index.set_failing(false);
        rig.composite
            .update(&t, &lost, organization("LOST", "v2"))
            .await
            .expect("update after recovery");
        rig.settle().await;
        assert!(rig.record_for(lost.id()).await.is_none(), "{mode:?}");
        assert_eq!(rig.found_by_search("LOST").await, 1, "{mode:?}");
        assert_eq!(rig.metrics.needs_reindex(), 1, "{mode:?}: gauge");

        // The repair takes care of the one nobody wrote again: deleted on
        // the primary, still indexed on the secondary.
        assert!(rig.index.holds("Organization", kept.id()).await.is_some());
        let report = rig
            .composite
            .repair_secondary_sync_failures(10)
            .await
            .expect("repair");
        assert_eq!((report.examined, report.repaired), (1, 1), "{mode:?}");
        assert_eq!(report.remaining, 0, "{mode:?}");
        assert!(rig.index.holds("Organization", kept.id()).await.is_none());
        assert_eq!(rig.found_by_search("KEPT").await, 0, "{mode:?}");
        assert!(rig.records().await.is_empty(), "{mode:?}");
        assert_eq!(rig.metrics.needs_reindex(), 0, "{mode:?}: gauge");
        assert_eq!(
            rig.metrics.failures().len(),
            3,
            "{mode:?}: recovery and repair count nothing"
        );

        // Idempotent: nothing left, nothing done.
        let again = rig.composite.repair_secondary_sync_failures(10).await;
        assert_eq!(again.expect("repair").examined, 0, "{mode:?}");
    }
}

/// The repair pushes the primary's current state of a resource the secondary
/// never received, and a secondary that is still down leaves the record in
/// place without counting a new write failure.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repair_resyncs_a_create_the_secondary_never_received() {
    for mode in MODES {
        let rig = rig(mode);
        let t = tenant();

        rig.index.set_failing(true);
        let lost = rig
            .composite
            .create(
                &t,
                "Organization",
                organization("LOST", "v1"),
                FhirVersion::R4,
            )
            .await
            .expect("create");
        rig.settle().await;
        assert_eq!(rig.metrics.failures().len(), 1, "{mode:?}");

        // Still down: bounded, one attempt, the record stays and ages.
        let report = rig
            .composite
            .repair_secondary_sync_failures(10)
            .await
            .expect("repair against a down secondary is not an error");
        assert_eq!((report.repaired, report.still_failing), (0, 1), "{mode:?}");
        assert_eq!(report.remaining, 1, "{mode:?}");
        let record = rig.record_for(lost.id()).await.expect("record stays");
        assert_eq!(record.attempts, ATTEMPTS + 1, "{mode:?}");
        assert_eq!(rig.metrics.failures().len(), 1, "{mode:?}: writes only");

        rig.index.set_failing(false);
        let report = rig
            .composite
            .repair_secondary_sync_failures(10)
            .await
            .expect("repair");
        assert_eq!((report.repaired, report.remaining), (1, 0), "{mode:?}");
        assert_eq!(rig.found_by_search("LOST").await, 1, "{mode:?}: searchable");
        assert!(rig.records().await.is_empty(), "{mode:?}");
    }
}

/// The record lives in the primary, so it outlives the process: re-open the
/// primary's database and the resource is still owed, and still repairable.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn records_survive_a_restart_of_the_primary() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path: &Path = &dir.path().join("primary.db");
    let path = path.to_str().expect("utf-8 temp path");

    let lost_id = {
        let rig = rig_on(SyncMode::Asynchronous, path, true);
        rig.index.set_failing(true);
        let lost = rig
            .composite
            .create(
                &tenant(),
                "Organization",
                organization("LOST", "v1"),
                FhirVersion::R4,
            )
            .await
            .expect("create");
        rig.settle().await;
        assert!(rig.record_for(lost.id()).await.is_some());
        lost.id().to_string()
    };

    // A new process: fresh composite, fresh (empty) index, same database.
    let rig = rig_on(SyncMode::Asynchronous, path, true);
    assert_eq!(rig.found_by_search("LOST").await, 0, "control: not indexed");
    let record = rig.record_for(&lost_id).await.expect("record survived");
    assert_eq!(record.operation, SyncOperation::Create);

    let report = rig
        .composite
        .repair_secondary_sync_failures(10)
        .await
        .expect("repair");
    assert_eq!((report.repaired, report.remaining), (1, 0));
    assert_eq!(rig.found_by_search("LOST").await, 1);
}

/// A record left by an earlier run is cleared by the first successful write
/// of that resource in this one, not only by the repair.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_successful_write_clears_a_record_left_by_an_earlier_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("primary.db");
    let path = path.to_str().expect("utf-8 temp path");

    let lost = {
        let rig = rig_on(SyncMode::Synchronous, path, true);
        rig.index.set_failing(true);
        rig.composite
            .create(
                &tenant(),
                "Organization",
                organization("LOST", "v1"),
                FhirVersion::R4,
            )
            .await
            .expect("create")
    };

    let rig = rig_on(SyncMode::Synchronous, path, true);
    rig.composite
        .update(&tenant(), &lost, organization("LOST", "v2"))
        .await
        .expect("update");
    assert!(rig.records().await.is_empty());
    assert_eq!(rig.found_by_search("LOST").await, 1);
    assert_eq!(rig.metrics.needs_reindex(), 0);
}

/// Deleting what the secondary does not have is the delete having worked,
/// not a failure to retry: one call, nothing counted, and the record the
/// missed create left behind is settled by it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_of_a_resource_the_secondary_lacks_is_success() {
    for mode in MODES {
        let rig = rig(mode);
        let t = tenant();

        rig.index.set_failing(true);
        let lost = rig
            .composite
            .create(
                &t,
                "Organization",
                organization("LOST", "v1"),
                FhirVersion::R4,
            )
            .await
            .expect("create");
        rig.settle().await;
        rig.index.set_failing(false);

        rig.composite
            .delete(&t, "Organization", lost.id())
            .await
            .expect("delete");
        rig.settle().await;

        assert_eq!(
            rig.index.delete_calls.load(Ordering::SeqCst),
            1,
            "{mode:?}: NotFound on a delete must not be retried"
        );
        assert_eq!(rig.metrics.failures().len(), 1, "{mode:?}: only the create");
        assert!(rig.records().await.is_empty(), "{mode:?}: in sync again");
    }
}

/// The batch path (`create_many`: conformance seeding, bulk loads) reports
/// each resource the secondary refused, not the batch as one anonymous unit.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_batch_is_recorded_per_resource() {
    for mode in MODES {
        let rig = rig(mode);
        rig.index.set_failing(true);
        let results = rig
            .composite
            .create_many(
                &tenant(),
                "Organization",
                vec![organization("A", "a"), organization("B", "b")],
                FhirVersion::R4,
            )
            .await;
        assert!(results.iter().all(Result::is_ok), "{mode:?}");
        rig.settle().await;

        assert_eq!(rig.metrics.failures().len(), 2, "{mode:?}");
        assert_eq!(rig.records().await.len(), 2, "{mode:?}");

        rig.index.set_failing(false);
        let report = rig.composite.repair_secondary_sync_failures(10).await;
        assert_eq!(report.expect("repair").repaired, 2, "{mode:?}");
        assert_eq!(rig.found_by_search("A").await, 1, "{mode:?}");
        assert_eq!(rig.found_by_search("B").await, 1, "{mode:?}");
    }
}

/// A primary with no ledger (S3): the write still succeeds and the failure
/// is still counted and logged; there is simply nothing to list or drain.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_ledger_failures_are_still_counted() {
    for mode in MODES {
        let rig = rig_on(mode, ":memory:", false);
        rig.index.set_failing(true);
        rig.composite
            .create(
                &tenant(),
                "Organization",
                organization("LOST", "v1"),
                FhirVersion::R4,
            )
            .await
            .expect("create");
        rig.settle().await;

        assert_eq!(rig.metrics.failures().len(), 1, "{mode:?}");
        assert!(rig.records().await.is_empty(), "{mode:?}");
        let report = rig.composite.repair_secondary_sync_failures(10).await;
        assert_eq!(report.expect("repair").examined, 0, "{mode:?}");
    }
}

/// The bound is honoured, and a record that keeps failing goes to the back
/// of the queue rather than starving the ones behind it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repair_is_bounded_and_rotates_through_the_queue() {
    let rig = rig(SyncMode::Synchronous);
    rig.index.set_failing(true);
    for identifier in ["A", "B", "C"] {
        rig.composite
            .create(
                &tenant(),
                "Organization",
                organization(identifier, "v1"),
                FhirVersion::R4,
            )
            .await
            .expect("create");
    }
    let first = rig.records().await[0].key.clone();

    let report = rig.composite.repair_secondary_sync_failures(1).await;
    let report = report.expect("repair");
    assert_eq!((report.examined, report.still_failing), (1, 1));
    assert_eq!(report.remaining, 3);
    assert_ne!(
        rig.records().await[0].key,
        first,
        "the failed one moved back"
    );

    rig.index.set_failing(false);
    let report = rig.composite.repair_secondary_sync_failures(2).await;
    let report = report.expect("repair");
    assert_eq!((report.examined, report.repaired), (2, 2));
    assert_eq!(report.remaining, 1);
}
