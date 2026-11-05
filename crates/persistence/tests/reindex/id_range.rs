//! #1767: `$reindex` id ranges (`idStart` included, `idEnd` not included,
//! #1739) on the SQL primaries. The same scenarios as MongoDB's
//! `tests/mongodb/reindex_id_range.rs`, for any backend that is both the
//! reindex source and its target: SQLite and PostgreSQL run them.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::error::StorageResult;
use helios_persistence::search::{
    ReindexIdRange, ReindexOperation, ReindexProgress, ReindexRequest, ReindexSource,
    ReindexStatus, ReindexTarget, TenantSearchRegistries,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchQuery, SearchValue, StoredResource,
};
use serde_json::json;

/// What the suite needs of a backend.
pub trait RangedBackend:
    ResourceStorage + SearchProvider + ReindexSource + ReindexTarget + 'static
{
}

impl<B: ResourceStorage + SearchProvider + ReindexSource + ReindexTarget + 'static> RangedBackend
    for B
{
}

/// The ranges the tests lay end to end: each `idEnd` is the next `idStart`.
const RANGES: [(Option<&str>, Option<&str>); 4] = [
    (None, Some("4")),
    (Some("4"), Some("8")),
    (Some("8"), Some("c")),
    (Some("c"), None),
];

/// Sixteen random-UUID-shaped ids, one per leading hex digit, so each of
/// [`RANGES`] holds four.
fn patient_ids() -> Vec<String> {
    (0..16)
        .map(|n| format!("{n:x}0000000-0000-4000-8000-000000000000"))
        .collect()
}

fn range(start: Option<&str>, end: Option<&str>) -> ReindexIdRange {
    ReindexIdRange {
        start: start.map(str::to_string),
        end: end.map(str::to_string),
    }
}

fn ranged_request(start: Option<&str>, end: Option<&str>) -> ReindexRequest {
    ReindexRequest {
        id_start: start.map(str::to_string),
        id_end: end.map(str::to_string),
        ..ReindexRequest::for_types(["Patient"])
    }
    .with_batch_size(2)
}

fn new_tenant(prefix: &str) -> TenantContext {
    TenantContext::new(
        TenantId::new(format!("{prefix}-{}", uuid::Uuid::new_v4().simple())),
        TenantPermissions::full_access(),
    )
}

async fn create_patient<B: RangedBackend>(backend: &B, tenant: &TenantContext, id: &str) {
    backend
        .create(
            tenant,
            "Patient",
            json!({"resourceType": "Patient", "id": id, "gender": "female"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
}

/// Creates a female Patient per id and one Observation.
async fn seed<B: RangedBackend>(backend: &B, tenant: &TenantContext, ids: &[String]) {
    for id in ids {
        create_patient(backend, tenant, id).await;
    }
    backend
        .create(
            tenant,
            "Observation",
            json!({"resourceType": "Observation", "id": "o1", "status": "final", "code": {"text": "range test"}}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
}

async fn found<B: RangedBackend>(
    backend: &B,
    tenant: &TenantContext,
    resource_type: &str,
    name: &str,
    value: &str,
) -> usize {
    let query = SearchQuery::new(resource_type)
        .with_count(100)
        .with_parameter(SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq(value)],
            chain: vec![],
            components: vec![],
        });
    backend
        .search(tenant, &query)
        .await
        .unwrap()
        .resources
        .items
        .len()
}

async fn patients_found<B: RangedBackend>(backend: &B, tenant: &TenantContext) -> usize {
    found(backend, tenant, "Patient", "gender", "female").await
}

async fn observations_found<B: RangedBackend>(backend: &B, tenant: &TenantContext) -> usize {
    found(backend, tenant, "Observation", "status", "final").await
}

/// Ids of every page `source` serves for `Patient`, with `limit` and
/// `max_bytes`, checking each page holds only ids in `bounds`.
async fn walk(
    source: &Arc<dyn ReindexSource>,
    tenant: &TenantContext,
    bounds: &ReindexIdRange,
    limit: u32,
    max_bytes: u64,
) -> Vec<String> {
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..100 {
        let page = source
            .fetch_resources_page_capped(tenant, "Patient", cursor.as_deref(), limit, max_bytes)
            .await
            .unwrap();
        assert!(
            page.resources.iter().all(|r| bounds.contains(r.id())),
            "{bounds:?}: {:?}",
            page.resources.iter().map(|r| r.id()).collect::<Vec<_>>()
        );
        seen.extend(page.resources.iter().map(|r| r.id().to_string()));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return seen,
        }
    }
    panic!("{bounds:?}: the walk did not end");
}

/// Writes through to the backend and records every id it writes, so a test
/// can check each resource was written exactly once. Runs `after_first_page`
/// once, after the first page is written.
struct RecordingWriter<B> {
    inner: Arc<B>,
    written: Mutex<Vec<String>>,
    after_first_page:
        Option<Box<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>>,
    fired: AtomicBool,
}

impl<B> RecordingWriter<B> {
    fn new(inner: Arc<B>) -> Self {
        Self {
            inner,
            written: Mutex::new(Vec::new()),
            after_first_page: None,
            fired: AtomicBool::new(false),
        }
    }

    fn written_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for id in self.written.lock().unwrap().iter() {
            *counts.entry(id.clone()).or_insert(0) += 1;
        }
        counts
    }
}

#[async_trait]
impl<B: RangedBackend> ReindexTarget for RecordingWriter<B> {
    async fn delete_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<u64> {
        self.inner
            .delete_search_entries(tenant, resource_type, resource_id)
            .await
    }

    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource: &StoredResource,
    ) -> StorageResult<usize> {
        self.inner.write_search_entries(tenant, resource).await
    }

    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
        self.inner.clear_search_index(tenant).await
    }

    async fn clear_search_index_for_types(
        &self,
        tenant: &TenantContext,
        resource_types: Option<&[String]>,
    ) -> StorageResult<u64> {
        self.inner
            .clear_search_index_for_types(tenant, resource_types)
            .await
    }

    async fn write_search_entries_page(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
    ) -> Vec<StorageResult<usize>> {
        self.written
            .lock()
            .unwrap()
            .extend(resources.iter().map(|r| r.id().to_string()));
        let results = self
            .inner
            .write_search_entries_page(tenant, resources)
            .await;
        if let Some(after_first_page) = &self.after_first_page
            && !self.fired.swap(true, Ordering::SeqCst)
        {
            after_first_page().await;
        }
        results
    }
}

async fn wait_for_terminal(op: &ReindexOperation, job_id: &str) -> ReindexProgress {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let progress = op.get_progress(job_id).await.unwrap();
            if progress.status.is_finished() {
                return progress;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("reindex status should become terminal")
}

/// Asserts a ranged job completed with the range's own totals.
fn assert_range_totals(progress: &ReindexProgress, expected: u64) {
    assert_eq!(
        progress.status,
        ReindexStatus::Completed,
        "{:?}",
        progress.error_message
    );
    assert_eq!(progress.total_resources, expected);
    assert_eq!(progress.processed_resources, expected);
    assert!(progress.errors.is_empty(), "{:?}", progress.errors);
}

/// The ranged source itself: counts and pages, capped by bytes or not, stay
/// inside the range, adjacent ranges cover every id once, and a lookup by id
/// drops ids outside the range.
pub async fn assert_ranged_source_bounds_counts_and_pages<B: RangedBackend>(backend: Arc<B>) {
    let tenant = new_tenant("id-range-source");
    let ids = patient_ids();
    seed(backend.as_ref(), &tenant, &ids).await;

    for max_bytes in [0, 1] {
        let mut seen = Vec::new();
        for (start, end) in RANGES {
            let bounds = range(start, end);
            let source = backend.clone().with_id_range(bounds.clone()).unwrap();
            assert_eq!(source.count_resources(&tenant, "Patient").await.unwrap(), 4);
            assert_eq!(
                source
                    .count_resources(&tenant, "Observation")
                    .await
                    .unwrap(),
                u64::from(bounds.contains("o1"))
            );
            let walked = walk(&source, &tenant, &bounds, 3, max_bytes).await;
            assert_eq!(walked.len(), 4, "{bounds:?} (max_bytes {max_bytes})");
            seen.extend(walked);
        }
        assert_eq!(
            seen, ids,
            "adjacent ranges cover every id exactly once, in id order (max_bytes {max_bytes})"
        );
    }

    let source = backend
        .clone()
        .with_id_range(range(Some("4"), Some("8")))
        .unwrap();
    let mut by_ids: Vec<String> = source
        .fetch_resources_by_ids(&tenant, "Patient", &ids)
        .await
        .unwrap()
        .iter()
        .map(|r| r.id().to_string())
        .collect();
    by_ids.sort();
    assert_eq!(by_ids, ids[4..8].to_vec());
}

/// Ids that differ only in `-`, `.` or case sit where byte order puts them,
/// not where a locale collation (PostgreSQL's database default) would: a
/// locale collation ignores `-` and `.` and folds case at its first level.
pub async fn assert_ranges_follow_byte_order<B: RangedBackend>(backend: Arc<B>) {
    let tenant = new_tenant("id-range-bytes");
    let ids = ["A", "B", "B-1", "a", "a-b", "a.b", "ab", "b"];
    for id in ids {
        create_patient(backend.as_ref(), &tenant, id).await;
    }

    let expected: [(Option<&str>, Option<&str>, &[&str]); 4] = [
        (None, Some("a"), &["A", "B", "B-1"]),
        (Some("a"), Some("a.b"), &["a", "a-b"]),
        (Some("a.b"), Some("b"), &["a.b", "ab"]),
        (Some("b"), None, &["b"]),
    ];
    for (start, end, want) in expected {
        let bounds = range(start, end);
        let source = backend.clone().with_id_range(bounds.clone()).unwrap();
        assert_eq!(
            source.count_resources(&tenant, "Patient").await.unwrap(),
            want.len() as u64,
            "{bounds:?}"
        );
        for (limit, max_bytes) in [(1, 0), (2, 0), (100, 0), (2, 1)] {
            assert_eq!(
                walk(&source, &tenant, &bounds, limit, max_bytes).await,
                want,
                "{bounds:?}, limit {limit}, max_bytes {max_bytes}"
            );
        }
        // The driver's own range check agrees with the backend.
        assert!(want.iter().all(|id| bounds.contains(id)));
    }
}

/// Ranges laid end to end, run in sequence or all at once, rebuild the type
/// exactly once, each job reporting its own range's totals.
pub async fn assert_ranges_cover_a_type_once<B: RangedBackend>(
    backend: Arc<B>,
    registries: Arc<TenantSearchRegistries>,
    concurrently: bool,
) {
    let tenant = new_tenant(if concurrently {
        "id-range-concurrent"
    } else {
        "id-range-sequence"
    });
    let ids = patient_ids();
    seed(backend.as_ref(), &tenant, &ids).await;
    let writer = Arc::new(RecordingWriter::new(backend.clone()));
    let op = ReindexOperation::with_parts(backend.clone(), vec![writer.clone()], registries);

    let cleared = op
        .start(
            tenant.clone(),
            ReindexRequest::for_types(["Patient"]).clear_only(),
            None,
        )
        .await
        .unwrap();
    assert_range_totals(&wait_for_terminal(&op, &cleared).await, 0);
    assert_eq!(patients_found(backend.as_ref(), &tenant).await, 0);

    if concurrently {
        let jobs: Vec<String> = futures::future::join_all(
            RANGES.map(|(start, end)| op.start(tenant.clone(), ranged_request(start, end), None)),
        )
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
        let finished =
            futures::future::join_all(jobs.iter().map(|job| wait_for_terminal(&op, job))).await;
        for progress in &finished {
            assert_range_totals(progress, 4);
        }
    } else {
        for (start, end) in RANGES {
            let job = op
                .start(tenant.clone(), ranged_request(start, end), None)
                .await
                .unwrap();
            assert_range_totals(&wait_for_terminal(&op, &job).await, 4);
        }
    }

    let written = writer.written_counts();
    assert_eq!(written.keys().cloned().collect::<Vec<_>>(), ids);
    assert!(written.values().all(|&n| n == 1), "{written:?}");
    assert_eq!(patients_found(backend.as_ref(), &tenant).await, 16);
    assert_eq!(observations_found(backend.as_ref(), &tenant).await, 1);
}

/// Resources created while a ranged job runs: one in range ahead of the
/// cursor is read by the job, one in range behind the cursor and one outside
/// the range are not, and all three are searchable through their own writes.
pub async fn assert_ranged_run_with_a_write_during_it<B: RangedBackend>(
    backend: Arc<B>,
    registries: Arc<TenantSearchRegistries>,
) {
    let tenant = new_tenant("id-range-during");
    let ids = patient_ids();
    seed(backend.as_ref(), &tenant, &ids).await;

    // The first page of `[4, 8)` at two per page ends at `5000…`.
    let mut writer = RecordingWriter::new(backend.clone());
    let (during_backend, during_tenant) = (backend.clone(), tenant.clone());
    writer.after_first_page = Some(Box::new(move || {
        let (backend, tenant) = (during_backend.clone(), during_tenant.clone());
        Box::pin(async move {
            for id in ["6-during", "4-during", "c-during"] {
                create_patient(backend.as_ref(), &tenant, id).await;
            }
        }) as futures::future::BoxFuture<'static, ()>
    }));
    let writer = Arc::new(writer);
    let op = ReindexOperation::with_parts(backend.clone(), vec![writer.clone()], registries);

    let job = op
        .start(tenant.clone(), ranged_request(Some("4"), Some("8")), None)
        .await
        .unwrap();
    let progress = wait_for_terminal(&op, &job).await;

    assert_eq!(
        progress.status,
        ReindexStatus::Completed,
        "{:?}",
        progress.error_message
    );
    // `total` was counted before the writes; the walk reads one of them.
    assert_eq!(progress.total_resources, 4);
    assert_eq!(progress.processed_resources, 5);
    let written = writer.written_counts();
    assert_eq!(written.get("6-during"), Some(&1));
    assert_eq!(written.get("4-during"), None, "behind the cursor");
    assert_eq!(written.get("c-during"), None, "outside the range");
    assert_eq!(patients_found(backend.as_ref(), &tenant).await, 19);
}
