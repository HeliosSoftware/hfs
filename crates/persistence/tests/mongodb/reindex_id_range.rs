//! #1739: `$reindex` id ranges (`idStart` included, `idEnd` not included) and
//! `clearOnly` on MongoDB. Child module of the `mongodb_tests` root, the same
//! arrangement as `tests/mongodb/reindex_id_walk.rs`.

use super::*;

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use helios_persistence::error::StorageResult;
use helios_persistence::search::{
    ReindexIdRange, ReindexOperation, ReindexProgress, ReindexRequest, ReindexSource,
    ReindexStatus, ReindexTarget, ResourcePage,
};
use helios_persistence::types::StoredResource;

use super::reindex_id_walk::wait_for_terminal;

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
    .with_batch_bytes(1)
}

/// Creates a female Patient per id and an Observation, then backdates every
/// other Patient so a walk reads half of each range in the id phase (with
/// prefetch) and half in the catch-up rounds.
async fn seed(backend: &MongoBackend, tenant: &TenantContext, ids: &[String]) {
    for id in ids {
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
    backend
        .create(
            tenant,
            "Observation",
            json!({"resourceType": "Observation", "id": "o1", "status": "final", "code": {"text": "range test"}}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let old: Vec<String> = ids.iter().step_by(2).cloned().collect();
    backend
        .get_database()
        .await
        .unwrap()
        .collection::<Document>("resources")
        .update_many(
            doc! { "tenant_id": tenant.tenant_id().as_str(), "resource_type": "Patient", "id": { "$in": old } },
            doc! { "$set": { "last_updated": mongodb::bson::DateTime::from_millis(946_684_800_000) } },
        )
        .await
        .unwrap();
}

fn token_query(resource_type: &str, name: &str, value: &str) -> SearchQuery {
    let mut query = SearchQuery::new(resource_type)
        .with_count(100)
        .with_parameter(SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq(value)],
            chain: vec![],
            components: vec![],
        });
    query.total = Some(TotalMode::Accurate);
    query
}

async fn patients_found(backend: &MongoBackend, tenant: &TenantContext) -> Option<u64> {
    backend
        .search(tenant, &token_query("Patient", "gender", "female"))
        .await
        .unwrap()
        .total
}

async fn observations_found(backend: &MongoBackend, tenant: &TenantContext) -> Option<u64> {
    backend
        .search(tenant, &token_query("Observation", "status", "final"))
        .await
        .unwrap()
        .total
}

/// Writes through to the backend and records every id it writes, so a test
/// can check each resource was written exactly once. Runs `after_first_page`
/// once, after the first page is written.
struct RecordingWriter {
    inner: Arc<MongoBackend>,
    written: Mutex<Vec<String>>,
    after_first_page:
        Option<Box<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>>,
    fired: AtomicBool,
}

impl RecordingWriter {
    fn new(inner: Arc<MongoBackend>) -> Self {
        Self {
            inner,
            written: Mutex::new(Vec::new()),
            after_first_page: None,
            fired: AtomicBool::new(false),
        }
    }

    fn written_once_each(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for id in self.written.lock().unwrap().iter() {
            *counts.entry(id.clone()).or_insert(0) += 1;
        }
        counts
    }
}

#[async_trait]
impl ReindexTarget for RecordingWriter {
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

/// Counts the pages it serves and delegates everything to `inner`.
struct CountingSource {
    inner: Arc<MongoBackend>,
    pages: AtomicUsize,
}

#[async_trait]
impl ReindexSource for CountingSource {
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        self.inner.list_resource_types(tenant).await
    }

    async fn count_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        self.inner.count_resources(tenant, resource_type).await
    }

    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<ResourcePage> {
        self.pages.fetch_add(1, Ordering::SeqCst);
        self.inner
            .fetch_resources_page(tenant, resource_type, cursor, limit)
            .await
    }
}

/// Asserts a ranged job completed with the range's own totals, both on the
/// progress and on the `$reindex-status` parameters built from it.
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
    let status = progress.to_parameters();
    let integer = |name: &str| {
        status["parameter"]
            .as_array()
            .and_then(|parameters| parameters.iter().find(|p| p["name"] == name))
            .and_then(|p| p["valueInteger"].as_u64())
    };
    assert_eq!(integer("total"), Some(expected));
    assert_eq!(integer("processed"), Some(expected));
}

/// The ranged source itself: counts, pages, prefetched pages and catch-up
/// rounds stay inside the range, and adjacent ranges cover every id once.
#[tokio::test]
async fn mongodb_reindex_id_range_bounds_counts_pages_prefetch_and_catch_up() {
    let Some(backend) = create_backend("reindex_id_range_source").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("id-range-source");
    let ids = patient_ids();
    seed(&backend, &tenant, &ids).await;

    let mut seen = Vec::new();
    let mut prefetched = 0;
    let mut catch_up = 0;
    for (start, end) in RANGES {
        let bounds = range(start, end);
        let source = backend.clone().with_id_range(bounds.clone()).unwrap();
        assert_eq!(source.count_resources(&tenant, "Patient").await.unwrap(), 4);
        let mut cursor: Option<String> = None;
        for _ in 0..30 {
            let page = source
                .fetch_resources_page_capped(&tenant, "Patient", cursor.as_deref(), 2, 1)
                .await
                .unwrap();
            assert!(page.resources.iter().all(|r| bounds.contains(r.id())));
            seen.extend(page.resources.iter().map(|r| r.id().to_string()));
            let Some(next) = page.next_cursor else {
                break;
            };
            if source.may_prefetch_page(&next) {
                if let Some(ahead) = source
                    .fetch_resources_page_ahead(&tenant, "Patient", &next, 2, 1)
                    .await
                    .unwrap()
                {
                    assert!(ahead.resources.iter().all(|r| bounds.contains(r.id())));
                    prefetched += ahead.resources.len();
                }
            } else {
                catch_up += page.resources.len();
            }
            cursor = Some(next);
        }
    }
    assert!(prefetched > 0, "the id-phase prefetch must run");
    assert_eq!(
        catch_up, 8,
        "every fresh Patient is read in a catch-up round"
    );
    seen.sort();
    assert_eq!(seen, ids, "adjacent ranges cover every id exactly once");
}

/// Ranges laid end to end and run one after another rebuild the type exactly
/// once, each job reporting its own range's totals.
#[tokio::test]
async fn mongodb_reindex_id_ranges_run_in_sequence_cover_a_type_once() {
    let Some(backend) = create_backend("reindex_id_range_sequence").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("id-range-sequence");
    let ids = patient_ids();
    seed(&backend, &tenant, &ids).await;
    let writer = Arc::new(RecordingWriter::new(backend.clone()));
    let op = ReindexOperation::with_parts(
        backend.clone(),
        vec![writer.clone()],
        backend.tenant_registries().clone(),
    );

    let cleared = op
        .start(
            tenant.clone(),
            ReindexRequest::for_types(["Patient"]).clear_only(),
            None,
        )
        .await
        .unwrap();
    assert_range_totals(&wait_for_terminal(&op, &cleared).await, 0);
    assert_eq!(patients_found(&backend, &tenant).await, Some(0));

    for (start, end) in RANGES {
        let job = op
            .start(tenant.clone(), ranged_request(start, end), None)
            .await
            .unwrap();
        assert_range_totals(&wait_for_terminal(&op, &job).await, 4);
    }

    let written = writer.written_once_each();
    assert_eq!(written.keys().cloned().collect::<Vec<_>>(), ids);
    assert!(written.values().all(|&n| n == 1), "{written:?}");
    assert_eq!(patients_found(&backend, &tenant).await, Some(16));
    assert_eq!(observations_found(&backend, &tenant).await, Some(1));
}

/// The same ranges started at the same time on one type also rebuild it
/// exactly once.
#[tokio::test]
async fn mongodb_reindex_id_ranges_run_concurrently_cover_a_type_once() {
    let Some(backend) = create_backend("reindex_id_range_concurrent").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("id-range-concurrent");
    let ids = patient_ids();
    seed(&backend, &tenant, &ids).await;
    let types = ["Patient".to_string()];
    backend
        .clear_search_index_for_types(&tenant, Some(&types))
        .await
        .unwrap();
    assert_eq!(patients_found(&backend, &tenant).await, Some(0));
    let writer = Arc::new(RecordingWriter::new(backend.clone()));
    let op = Arc::new(ReindexOperation::with_parts(
        backend.clone(),
        vec![writer.clone()],
        backend.tenant_registries().clone(),
    ));

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

    let written = writer.written_once_each();
    assert_eq!(written.keys().cloned().collect::<Vec<_>>(), ids);
    assert!(written.values().all(|&n| n == 1), "{written:?}");
    assert_eq!(patients_found(&backend, &tenant).await, Some(16));
    assert_eq!(observations_found(&backend, &tenant).await, Some(1));
}

/// A resource created while a ranged job runs is indexed by that job's
/// catch-up when its id is in range, and left alone when it is not.
#[tokio::test]
async fn mongodb_reindex_id_range_indexes_a_resource_written_during_the_run() {
    use helios_persistence::core::{BulkProcessingOptions, BulkSubmitProvider, NdjsonEntry};

    let Some(backend) = create_backend("reindex_id_range_during").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("id-range-during");
    let ids = patient_ids();
    seed(&backend, &tenant, &ids).await;
    let (submission, manifest) = super::bulk_submit::seed(&backend, &tenant).await;

    // Created with deferred indexing, so only a reindex can index them.
    let mut writer = RecordingWriter::new(backend.clone());
    let (during_backend, during_tenant) = (backend.clone(), tenant.clone());
    writer.after_first_page = Some(Box::new(move || {
        let (backend, tenant) = (during_backend.clone(), during_tenant.clone());
        let (submission, manifest) = (submission.clone(), manifest.clone());
        Box::pin(async move {
            let entries = ["6-during", "c-during"]
                .into_iter()
                .enumerate()
                .map(|(line, id)| {
                    NdjsonEntry::new(
                        line as u64 + 1,
                        "Patient",
                        json!({"resourceType": "Patient", "id": id, "gender": "female"}),
                    )
                })
                .collect();
            backend
                .process_entries(
                    &tenant,
                    &submission,
                    &manifest,
                    entries,
                    &BulkProcessingOptions::new().with_defer_indexing(true),
                )
                .await
                .unwrap();
        }) as futures::future::BoxFuture<'static, ()>
    }));
    let writer = Arc::new(writer);
    let op = ReindexOperation::with_parts(
        backend.clone(),
        vec![writer.clone()],
        backend.tenant_registries().clone(),
    );

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
    // `total` was counted before the write; the catch-up adds the new one.
    assert_eq!(progress.total_resources, 4);
    assert_eq!(progress.processed_resources, 5);
    assert_eq!(writer.written_once_each().get("6-during"), Some(&1));
    assert!(search_index_entry_count(&backend, &tenant, "Patient", "6-during").await > 0);
    assert_eq!(
        search_index_entry_count(&backend, &tenant, "Patient", "c-during").await,
        0,
        "an id outside the range is not indexed"
    );
}

/// `clearOnly` on one type clears that type, pages no resources and leaves
/// other types searchable; on the tenant it clears every type.
#[tokio::test]
async fn mongodb_reindex_clear_only_clears_its_scope_without_paging() {
    let Some(backend) = create_backend("reindex_clear_only").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("clear-only");
    let other = create_tenant("clear-only-other");
    let ids = patient_ids();
    seed(&backend, &tenant, &ids).await;
    seed(&backend, &other, &ids).await;
    let source = Arc::new(CountingSource {
        inner: backend.clone(),
        pages: AtomicUsize::new(0),
    });
    let writer = Arc::new(RecordingWriter::new(backend.clone()));
    let op = ReindexOperation::with_parts(
        source.clone(),
        vec![writer.clone()],
        backend.tenant_registries().clone(),
    );
    let run = |request: ReindexRequest| {
        let (op, tenant) = (&op, tenant.clone());
        async move {
            let job = op.start(tenant, request, None).await.unwrap();
            wait_for_terminal(op, &job).await
        }
    };

    assert_range_totals(
        &run(ReindexRequest::for_types(Vec::<String>::new()).clear_only()).await,
        0,
    );
    assert_eq!(patients_found(&backend, &tenant).await, Some(16));

    assert_range_totals(
        &run(ReindexRequest::for_types(["Patient"]).clear_only()).await,
        0,
    );
    assert_eq!(patients_found(&backend, &tenant).await, Some(0));
    assert_eq!(observations_found(&backend, &tenant).await, Some(1));
    assert_eq!(patients_found(&backend, &other).await, Some(16));

    assert_range_totals(&run(ReindexRequest::all().clear_only()).await, 0);
    assert_eq!(observations_found(&backend, &tenant).await, Some(0));
    assert_eq!(observations_found(&backend, &other).await, Some(1));

    assert_eq!(source.pages.load(Ordering::SeqCst), 0, "no page was read");
    assert!(writer.written.lock().unwrap().is_empty());
}
