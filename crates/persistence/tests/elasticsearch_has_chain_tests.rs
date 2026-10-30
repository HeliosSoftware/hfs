//! `_has` (reverse chain) resolution over Elasticsearch (#1753).
//!
//! On the Elasticsearch composites a `_has` hop used to page through every
//! matching source resource (175,355 Observations on the full Synthea corpus)
//! just to collect the distinct patient ids. `ElasticsearchBackend` now answers
//! `SearchProvider::search_reference_values` with one aggregation over the
//! reference index, and `CompositeStorage` forwards the hook to its search
//! provider. These tests pin, against a real Elasticsearch (Docker):
//!
//! - the values the hook returns are what the resources hold: updated
//!   references replace the old one, deleted and contained copies are absent,
//!   other tenants (including a tenant whose id extends this one) never leak;
//! - a missing index answers `Some([])`, not an error and not `None`;
//! - more distinct values than one aggregation partition holds are returned
//!   without loss or duplicates;
//! - an end-to-end `_has` resolves with no `search` on the source type;
//! - the same over a real PostgreSQL + Elasticsearch composite (`postgres`).
//!
//! Benchmark note: on the issue's corpus (`pg-es`, 11,705 Patients, 7,699,987
//! Observations, 175,355 of them LOINC `8302-2`) the request
//! `Patient?_has:Observation:patient:code=http://loinc.org|8302-2` took 67 s
//! because the resolver drained 176 pages of 1,000 full Observations. With the
//! hook it is one cardinality request plus one `terms` request per partition
//! returning ~11.7k buckets; `_has_resolution_issues_no_source_type_search`
//! asserts the structural half (zero `Observation` searches). Record the
//! measured before/after wall time on #1753 from a run on the full corpus.
//!
//! Run with:
//!   cargo test -p helios-persistence --features elasticsearch --test elasticsearch_has_chain_tests
//!   cargo test -p helios-persistence --features postgres,elasticsearch --test elasticsearch_has_chain_tests

#![cfg(feature = "elasticsearch")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use parking_lot::RwLock;
use serde_json::{Value, json};
use testcontainers::ImageExt;
use testcontainers::core::WaitFor;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::elastic_search::ElasticSearch;
use tokio::sync::OnceCell;

use helios_persistence::backends::elasticsearch::{
    ElasticsearchBackend, ElasticsearchConfig, WriteRefreshPolicy,
};
use helios_persistence::core::{Backend, ResourceStorage, SearchProvider, SearchResult};
use helios_persistence::error::StorageResult;
use helios_persistence::search::{
    SearchParameterLoader, SearchParameterRegistry, TenantSearchRegistries, resolve_chains,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    ChainedParameter, Page, ReverseChainedParameter, SearchParamType, SearchParameter, SearchQuery,
    SearchValue, StoredResource,
};

// ============================================================================
// Container and backend
// ============================================================================

const DEFAULT_ES_IMAGE_TAG: &str = "7.17.29";

/// Elasticsearch image tag; override with `HFS_TEST_ES_IMAGE_TAG` (e.g. `8.15.0`).
fn es_image_tag() -> String {
    std::env::var("HFS_TEST_ES_IMAGE_TAG").unwrap_or_else(|_| DEFAULT_ES_IMAGE_TAG.to_string())
}
const ES_STARTUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
const ES_START_ATTEMPTS: usize = 2;

struct SharedEs {
    host: String,
    port: u16,
    _container: testcontainers::ContainerAsync<ElasticSearch>,
}

static SHARED_ES: OnceCell<SharedEs> = OnceCell::const_new();

async fn start_es_container() -> testcontainers::ContainerAsync<ElasticSearch> {
    let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
    let mut last_err = None;
    for attempt in 1..=ES_START_ATTEMPTS {
        let mut request = ElasticSearch::default()
            .with_tag(es_image_tag())
            .with_env_var("ES_JAVA_OPTS", "-Xms256m -Xmx256m")
            // 8.x enables security (HTTPS + auth) by default; the backend speaks plain HTTP.
            .with_env_var("xpack.security.enabled", "false")
            .with_label("github.run_id", &run_id)
            .with_startup_timeout(ES_STARTUP_TIMEOUT);
        if !es_image_tag().starts_with('7') {
            // The module waits for "[YELLOW] to [GREEN]", which 8.x only logs when a
            // system index is created; with security off none is, so wait for startup (`create_backend` then retries `initialize` until HTTP answers).
            request = request.with_ready_conditions(vec![WaitFor::message_on_stdout("started")]);
        }
        match container_cleanup::with_cleanup_label(request).start().await {
            Ok(container) => return container,
            Err(err) => {
                eprintln!(
                    "Elasticsearch container start attempt {attempt}/{ES_START_ATTEMPTS} failed: {err}"
                );
                last_err = Some(err);
            }
        }
    }
    panic!(
        "failed to start Elasticsearch container after {ES_START_ATTEMPTS} attempts: {:?}",
        last_err.expect("at least one attempt ran")
    );
}

async fn shared_es() -> &'static SharedEs {
    SHARED_ES
        .get_or_init(|| async {
            let container = start_es_container().await;
            let port = container
                .get_host_port_ipv4(9200)
                .await
                .expect("ES host port");
            let host = container.get_host().await.expect("ES host").to_string();
            SharedEs {
                host,
                port,
                _container: container,
            }
        })
        .await
}

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"))
}

fn build_search_registry() -> Arc<TenantSearchRegistries> {
    let loader = SearchParameterLoader::new(FhirVersion::default());
    let registries = Arc::new(TenantSearchRegistries::base_only());
    let mut registry = registries.base().write();
    if let Ok(params) = loader.load_embedded() {
        for param in params {
            let _ = registry.register(param);
        }
    }
    if let Ok(params) = loader.load_from_spec_file(&data_dir()) {
        for param in params {
            let _ = registry.register(param);
        }
    }
    drop(registry);
    registries
}

fn es_config(es: &SharedEs) -> ElasticsearchConfig {
    ElasticsearchConfig {
        nodes: vec![format!("http://{}:{}", es.host, es.port)],
        // A fresh prefix per backend keeps every test's indices apart.
        index_prefix: format!("hfs_{}", uuid::Uuid::new_v4().simple()),
        number_of_replicas: 0,
        refresh_interval: "1ms".to_string(),
        // Writes are searchable when the call returns.
        write_refresh: WriteRefreshPolicy::WaitFor,
        ..Default::default()
    }
}

/// 8.x logs "started" before the HTTP listener is up: retry until it answers.
async fn initialize_when_ready(backend: &ElasticsearchBackend) {
    let mut attempts = 0;
    loop {
        match backend.initialize().await {
            Ok(()) => return,
            Err(err) if attempts < 120 => {
                attempts += 1;
                eprintln!("ES not ready ({err}); retry {attempts}");
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Err(err) => panic!("initialize ES backend: {err}"),
        }
    }
}

async fn create_backend() -> ElasticsearchBackend {
    let es = shared_es().await;
    let backend =
        ElasticsearchBackend::with_shared_registry(es_config(es), build_search_registry())
            .expect("create ElasticsearchBackend");
    initialize_when_ready(&backend).await;
    backend
}

fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

// ============================================================================
// Fixtures
// ============================================================================

const LOINC: &str = "http://loinc.org";
const HEIGHT: &str = "8302-2";
const OTHER_CODE: &str = "29463-7";

fn observation(id: &str, code: &str, subject: &str) -> Value {
    json!({
        "resourceType": "Observation",
        "id": id,
        "status": "final",
        "code": {"coding": [{"system": LOINC, "code": code}]},
        "subject": {"reference": subject},
    })
}

fn patient(id: &str) -> Value {
    json!({"resourceType": "Patient", "id": id})
}

fn height_query() -> SearchQuery {
    SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        values: vec![SearchValue::token(Some(LOINC), HEIGHT)],
        ..Default::default()
    })
}

fn has_height_query() -> SearchQuery {
    let mut query = SearchQuery::new("Patient");
    query.reverse_chains.push(ReverseChainedParameter::terminal(
        "Observation",
        "patient",
        "code",
        SearchValue::token(Some(LOINC), HEIGHT),
    ));
    query
}

async fn put<S: ResourceStorage>(
    storage: &S,
    tenant: &TenantContext,
    resource_type: &str,
    resource: Value,
) -> StoredResource {
    storage
        .create(tenant, resource_type, resource, FhirVersion::default())
        .await
        .expect("create")
}

async fn values<S: SearchProvider>(
    storage: &S,
    tenant: &TenantContext,
    param: &str,
) -> Option<BTreeSet<String>> {
    storage
        .search_reference_values(tenant, &height_query(), param)
        .await
        .expect("search_reference_values")
        .map(|v| v.into_iter().collect())
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn ids(result: &SearchResult) -> BTreeSet<String> {
    result
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect()
}

// ============================================================================
// Counting wrapper: what the resolver asks of the backend
// ============================================================================

/// Counts the searches the resolver issues, per resource type, and the
/// `search_reference_values` calls the hook answered.
struct Counting<S> {
    inner: Arc<S>,
    searches: Mutex<HashMap<String, u64>>,
    reference_calls: Mutex<u64>,
    /// `search_ids` calls (a subset of `searches`).
    id_calls: Mutex<u64>,
    /// Answer `Ok(None)` to `search_reference_values`, as before #1753.
    hide_hook: bool,
}

impl<S> Counting<S> {
    fn new(inner: Arc<S>) -> Self {
        Self {
            inner,
            searches: Mutex::new(HashMap::new()),
            reference_calls: Mutex::new(0),
            id_calls: Mutex::new(0),
            hide_hook: false,
        }
    }

    fn without_hook(mut self) -> Self {
        self.hide_hook = true;
        self
    }

    fn searches_of(&self, resource_type: &str) -> u64 {
        self.searches
            .lock()
            .unwrap()
            .get(resource_type)
            .copied()
            .unwrap_or(0)
    }

    fn count(&self, resource_type: &str) {
        *self
            .searches
            .lock()
            .unwrap()
            .entry(resource_type.to_string())
            .or_default() += 1;
    }
}

#[async_trait]
impl<S: ResourceStorage> ResourceStorage for Counting<S> {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
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
        fhir_version: FhirVersion,
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
impl<S: SearchProvider> SearchProvider for Counting<S> {
    fn supports_native_reverse_chains(&self, tenant: &TenantContext, query: &SearchQuery) -> bool {
        self.inner.supports_native_reverse_chains(tenant, query)
    }
    async fn search_reference_values(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
        reference_param: &str,
    ) -> StorageResult<Option<Vec<String>>> {
        if self.hide_hook {
            return Ok(None);
        }
        let result = self
            .inner
            .search_reference_values(tenant, query, reference_param)
            .await?;
        if result.is_some() {
            *self.reference_calls.lock().unwrap() += 1;
        }
        Ok(result)
    }
    async fn search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        self.count(&query.resource_type);
        self.inner.search(tenant, query).await
    }
    async fn search_ids(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<Page<String>> {
        self.count(&query.resource_type);
        *self.id_calls.lock().unwrap() += 1;
        self.inner.search_ids(tenant, query).await
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
    ) -> Arc<RwLock<SearchParameterRegistry>> {
        self.inner.search_param_registry(tenant)
    }
}

// ============================================================================
// search_reference_values on Elasticsearch
// ============================================================================

#[tokio::test]
async fn reference_values_match_what_the_resources_hold() {
    let es = create_backend().await;
    let t = tenant("has-parity");

    put(
        &es,
        &t,
        "Observation",
        observation("o1", HEIGHT, "Patient/a"),
    )
    .await;
    let o2 = put(
        &es,
        &t,
        "Observation",
        observation("o2", HEIGHT, "Patient/b"),
    )
    .await;
    put(
        &es,
        &t,
        "Observation",
        observation("o3", HEIGHT, "Group/g1"),
    )
    .await;
    put(
        &es,
        &t,
        "Observation",
        observation("o4", HEIGHT, "Patient/d"),
    )
    .await;
    // Another code: never selected by the source filter.
    put(
        &es,
        &t,
        "Observation",
        observation("o5", OTHER_CODE, "Patient/z"),
    )
    .await;
    // A contained copy carries its own reference; it is not a current resource.
    let mut with_contained = observation("o6", HEIGHT, "Patient/a");
    with_contained["contained"] = json!([observation("c1", HEIGHT, "Patient/ghost")]);
    put(&es, &t, "Observation", with_contained).await;

    // The reference moves, and one resource goes away.
    es.update(&t, &o2, observation("o2", HEIGHT, "Patient/c"))
        .await
        .expect("update o2");
    es.delete(&t, "Observation", "o4").await.expect("delete o4");

    let subject = values(&es, &t, "subject")
        .await
        .expect("ES answers the hook");
    assert_eq!(subject, set(&["Patient/a", "Patient/c", "Group/g1"]));

    // `patient` only indexes Patient targets, but follows the same rules.
    let patient = values(&es, &t, "patient")
        .await
        .expect("ES answers the hook");
    for expected in ["Patient/a", "Patient/c"] {
        assert!(patient.contains(expected), "{expected} in {patient:?}");
    }
    for absent in ["Patient/b", "Patient/d", "Patient/z", "Patient/ghost"] {
        assert!(!patient.contains(absent), "{absent} in {patient:?}");
    }

    // Distinct values: `Patient/a` is held by two resources and appears once.
    let listed = es
        .search_reference_values(&t, &height_query(), "subject")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(listed.len(), subject.len(), "no duplicates: {listed:?}");
}

#[tokio::test]
async fn reference_values_do_not_cross_tenants() {
    let es = create_backend().await;
    // `a_b` matches a glob built from `a`, so a missing tenant term would leak.
    let a = tenant("a");
    let a_b = tenant("a_b");
    put(
        &es,
        &a,
        "Observation",
        observation("o1", HEIGHT, "Patient/of-a"),
    )
    .await;
    put(
        &es,
        &a_b,
        "Observation",
        observation("o1", HEIGHT, "Patient/of-a-b"),
    )
    .await;

    assert_eq!(
        values(&es, &a, "subject").await.unwrap(),
        set(&["Patient/of-a"])
    );
    assert_eq!(
        values(&es, &a_b, "subject").await.unwrap(),
        set(&["Patient/of-a-b"])
    );
}

#[tokio::test]
async fn reference_values_of_a_missing_index_are_empty_not_none() {
    let es = create_backend().await;
    let t = tenant("never-written");
    let answer = es
        .search_reference_values(&t, &height_query(), "subject")
        .await
        .expect("a missing index is not an error");
    assert_eq!(answer, Some(vec![]));
}

#[tokio::test]
async fn reference_values_fall_back_for_query_shapes_the_hook_does_not_cover() {
    let es = create_backend().await;
    let t = tenant("has-guards");
    put(
        &es,
        &t,
        "Observation",
        observation("o1", HEIGHT, "Patient/a"),
    )
    .await;

    let paged = height_query().with_count(5);
    // A plain `_count` is fine; a cursor/offset/sort is not.
    assert!(
        es.search_reference_values(&t, &paged, "subject")
            .await
            .unwrap()
            .is_some()
    );
    let mut with_offset = height_query();
    with_offset.offset = Some(1);
    assert!(
        es.search_reference_values(&t, &with_offset, "subject")
            .await
            .unwrap()
            .is_none()
    );
}

/// More distinct values than one aggregation partition holds (10,000), so the
/// answer needs more than one partition request.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reference_values_past_one_partition_are_complete_and_distinct() {
    const DISTINCT: usize = 12_000;

    let es = create_backend().await;
    let t = tenant("has-wide");

    for chunk in (0..DISTINCT).collect::<Vec<_>>().chunks(1_000) {
        let batch: Vec<Value> = chunk
            .iter()
            .map(|i| observation(&format!("o{i}"), HEIGHT, &format!("Patient/p{i}")))
            .collect();
        for result in es
            .create_many(&t, "Observation", batch, FhirVersion::default())
            .await
        {
            result.expect("create_many item");
        }
    }
    // Two more observations on an existing patient: still one value.
    put(
        &es,
        &t,
        "Observation",
        observation("dup1", HEIGHT, "Patient/p0"),
    )
    .await;
    put(
        &es,
        &t,
        "Observation",
        observation("dup2", HEIGHT, "Patient/p0"),
    )
    .await;

    let listed = es
        .search_reference_values(&t, &height_query(), "subject")
        .await
        .expect("search_reference_values")
        .expect("ES answers the hook");
    let distinct: BTreeSet<String> = listed.iter().cloned().collect();
    assert_eq!(listed.len(), DISTINCT, "no duplicates across partitions");
    assert_eq!(distinct.len(), DISTINCT, "no value lost");
    let expected: BTreeSet<String> = (0..DISTINCT).map(|i| format!("Patient/p{i}")).collect();
    assert_eq!(distinct, expected);
}

// ============================================================================
// End to end: the resolver over the hook
// ============================================================================

#[tokio::test]
async fn has_resolution_issues_no_source_type_search() {
    let es = Arc::new(create_backend().await);
    let t = tenant("has-e2e");
    for id in ["p1", "p2", "p3", "p4"] {
        put(&*es, &t, "Patient", patient(id)).await;
    }
    put(
        &*es,
        &t,
        "Observation",
        observation("o1", HEIGHT, "Patient/p1"),
    )
    .await;
    put(
        &*es,
        &t,
        "Observation",
        observation("o2", HEIGHT, "Patient/p1"),
    )
    .await;
    put(
        &*es,
        &t,
        "Observation",
        observation("o3", HEIGHT, "Patient/p3"),
    )
    .await;
    put(
        &*es,
        &t,
        "Observation",
        observation("o4", OTHER_CODE, "Patient/p2"),
    )
    .await;
    // A hit on a Group is not a Patient id.
    put(
        &*es,
        &t,
        "Observation",
        observation("o5", HEIGHT, "Group/p4"),
    )
    .await;

    let counting = Counting::new(es);
    let resolved = resolve_chains(&counting, &t, &has_height_query())
        .await
        .expect("resolve _has");

    assert_eq!(counting.searches_of("Observation"), 0, "no source paging");
    assert_eq!(*counting.reference_calls.lock().unwrap(), 1);

    let result = counting
        .search(&t, &resolved)
        .await
        .expect("Patient search");
    assert_eq!(ids(&result), set(&["p1", "p3"]));
}

// ============================================================================
// search_ids on Elasticsearch (#2295): ids only, same pages as `search`
// ============================================================================

/// Pages `query` with both `search` and `search_ids` at `count` and checks
/// that every page carries the same ids in the same order, the same
/// `has_next` and the same `next_cursor`. Returns the ids in page order.
async fn page_both_ways(
    es: &ElasticsearchBackend,
    t: &TenantContext,
    query: &SearchQuery,
    count: u32,
) -> Vec<String> {
    let mut all = Vec::new();
    let mut seen_cursors = BTreeSet::new();
    let mut cursor: Option<String> = None;
    for _ in 0..1000 {
        let mut q = query.clone().with_count(count);
        if let Some(c) = &cursor {
            q = q.with_cursor(c.clone());
        }
        let full = es.search(t, &q).await.expect("search");
        let lean = es.search_ids(t, &q).await.expect("search_ids");

        let full_ids: Vec<String> = full
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        assert_eq!(lean.items, full_ids, "page ids differ at cursor {cursor:?}");
        assert_eq!(lean.page_info.has_next, full.resources.page_info.has_next);
        assert_eq!(
            lean.page_info.next_cursor, full.resources.page_info.next_cursor,
            "next_cursor differs at cursor {cursor:?}"
        );
        assert!(
            !(lean.page_info.has_next && lean.items.is_empty()),
            "empty page with has_next"
        );
        assert!(lean.items.len() <= count as usize);
        all.extend(lean.items.iter().cloned());

        match lean.page_info.next_cursor.clone() {
            Some(next) if lean.page_info.has_next => {
                assert!(seen_cursors.insert(next.clone()), "cursor repeated: {next}");
                cursor = Some(next);
            }
            _ => return all,
        }
    }
    panic!("paging did not terminate");
}

#[tokio::test]
async fn search_ids_pages_match_search_pages() {
    let es = create_backend().await;
    let t = tenant("ids-parity");
    // A tenant whose id extends this one, and a plain neighbour.
    let other = tenant("ids-parity-x");

    let mut expected = BTreeSet::new();
    for i in 0..23 {
        let id = format!("o{i:02}");
        put(
            &es,
            &t,
            "Observation",
            observation(&id, HEIGHT, "Patient/a"),
        )
        .await;
        expected.insert(id);
    }
    for i in 0..5 {
        let id = format!("d{i}");
        put(
            &es,
            &t,
            "Observation",
            observation(&id, HEIGHT, "Patient/a"),
        )
        .await;
        es.delete(&t, "Observation", &id).await.expect("delete");
    }
    // Another code in the same tenant, and the same code in another tenant.
    put(
        &es,
        &t,
        "Observation",
        observation("z1", OTHER_CODE, "Patient/a"),
    )
    .await;
    for i in 0..4 {
        put(
            &es,
            &other,
            "Observation",
            observation(&format!("x{i}"), HEIGHT, "Patient/a"),
        )
        .await;
    }

    for count in [1, 4, 7, 23, 24, 1000] {
        let paged = page_both_ways(&es, &t, &height_query(), count).await;
        let distinct: BTreeSet<String> = paged.iter().cloned().collect();
        assert_eq!(paged.len(), distinct.len(), "count={count}: duplicates");
        assert_eq!(distinct, expected, "count={count}");
    }

    // A missing index is an empty page, not an error.
    let none = es
        .search_ids(&t, &SearchQuery::new("Encounter"))
        .await
        .expect("search_ids on a missing index");
    assert!(none.items.is_empty() && !none.page_info.has_next);
}

#[tokio::test]
async fn search_ids_falls_back_to_search_for_sorted_and_offset_queries() {
    let es = create_backend().await;
    let t = tenant("ids-fallback");
    for i in 0..6 {
        put(
            &es,
            &t,
            "Observation",
            observation(&format!("o{i}"), HEIGHT, "Patient/a"),
        )
        .await;
    }
    let mut offset = height_query().with_count(2);
    offset.offset = Some(2);
    for q in [offset, {
        let mut q = height_query().with_count(3);
        q.total = Some(helios_persistence::types::TotalMode::Accurate);
        q
    }] {
        let full: Vec<String> = es
            .search(&t, &q)
            .await
            .unwrap()
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        let lean = es.search_ids(&t, &q).await.unwrap().items;
        assert_eq!(lean, full);
    }
}

#[tokio::test]
async fn forward_chain_resolves_the_same_through_search_ids() {
    let es = create_backend().await;
    let t = tenant("ids-chain");
    for (id, family) in [("p1", "Smith"), ("p2", "Smith"), ("p3", "Jones")] {
        put(
            &es,
            &t,
            "Patient",
            json!({"resourceType": "Patient", "id": id, "name": [{"family": family}]}),
        )
        .await;
    }
    let mut expected = BTreeSet::new();
    for i in 0..12 {
        let id = format!("o{i:02}");
        let subject = ["Patient/p1", "Patient/p2", "Patient/p3"][i % 3];
        put(&es, &t, "Observation", observation(&id, HEIGHT, subject)).await;
        if i % 3 != 2 {
            expected.insert(id);
        }
    }

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "subject".to_string(),
        param_type: SearchParamType::Reference,
        modifier: None,
        values: vec![SearchValue::eq("Smith")],
        chain: vec![ChainedParameter {
            reference_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            target_param: "name".to_string(),
        }],
        components: vec![],
    });
    let resolved = resolve_chains(&es, &t, &query)
        .await
        .expect("resolve chain");

    for count in [3, 100] {
        let paged = page_both_ways(&es, &t, &resolved, count).await;
        assert_eq!(paged.iter().cloned().collect::<BTreeSet<_>>(), expected);
        assert_eq!(paged.len(), expected.len(), "count={count}: duplicates");
    }
}

/// Which resolver hops use which provider method (#2295):
/// - forward chain hops (terminal and every walk back) go through `search_ids`;
/// - a `_has` terminal goes through `search_reference_values` and issues no
///   source-type search at all (see `has_resolution_issues_no_source_type_search`);
/// - only when that hook declines does `_has` page the source type with the
///   full `search`, because it needs the reference values held in the bodies.
#[tokio::test]
async fn resolver_hops_use_search_ids_for_forward_chains_only() {
    let es = Arc::new(create_backend().await);
    let t = tenant("ids-routing");
    for (id, family) in [("p1", "Smith"), ("p2", "Jones")] {
        put(
            &*es,
            &t,
            "Patient",
            json!({"resourceType": "Patient", "id": id, "name": [{"family": family}]}),
        )
        .await;
    }
    put(
        &*es,
        &t,
        "Observation",
        observation("o1", HEIGHT, "Patient/p1"),
    )
    .await;
    put(
        &*es,
        &t,
        "Observation",
        observation("o2", HEIGHT, "Patient/p2"),
    )
    .await;

    let forward = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "subject".to_string(),
        param_type: SearchParamType::Reference,
        modifier: None,
        values: vec![SearchValue::eq("Smith")],
        chain: vec![ChainedParameter {
            reference_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            target_param: "name".to_string(),
        }],
        components: vec![],
    });
    let counting = Counting::new(es.clone());
    let resolved = resolve_chains(&counting, &t, &forward)
        .await
        .expect("resolve forward chain");
    assert_eq!(
        *counting.id_calls.lock().unwrap(),
        2,
        "terminal + walk back"
    );
    assert_eq!(
        counting.searches_of("Patient") + counting.searches_of("Observation"),
        2,
        "every forward hop is an id-only search"
    );
    assert_eq!(
        ids(&counting.search(&t, &resolved).await.unwrap()),
        set(&["o1"])
    );

    let counting = Counting::new(es.clone());
    resolve_chains(&counting, &t, &has_height_query())
        .await
        .expect("resolve _has");
    assert_eq!(*counting.id_calls.lock().unwrap(), 0);
    assert_eq!(counting.searches_of("Observation"), 0);

    let counting = Counting::new(es).without_hook();
    resolve_chains(&counting, &t, &has_height_query())
        .await
        .expect("resolve _has without the hook");
    assert_eq!(*counting.id_calls.lock().unwrap(), 0);
    assert_eq!(
        counting.searches_of("Observation"),
        1,
        "full search fallback"
    );
}

// ============================================================================
// Manual benchmark (ignored): `_has` latency with and without the hook
// ============================================================================

/// Loads `HFS_BENCH_OBS` Observations (default 200000; 90% carry the height
/// code) over `HFS_BENCH_PATIENTS` Patients (default 11705) and times the
/// reverse-chain resolution through the paged fallback and through the hook.
/// Run: `cargo test -p helios-persistence --features elasticsearch --test
/// elasticsearch_has_chain_tests -- --ignored --nocapture bench_has_latency`.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "manual benchmark"]
async fn bench_has_latency() {
    use futures::StreamExt;
    use std::time::Instant;

    let observations: usize = std::env::var("HFS_BENCH_OBS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200_000);
    let patients: usize = std::env::var("HFS_BENCH_PATIENTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(11_705);

    let es_env = shared_es().await;
    let mut config = es_config(es_env);
    config.write_refresh = WriteRefreshPolicy::False;
    config.refresh_interval = "1s".to_string();
    let es = Arc::new(
        ElasticsearchBackend::with_shared_registry(config, build_search_registry())
            .expect("create ElasticsearchBackend"),
    );
    initialize_when_ready(&es).await;
    let t = tenant("bench-has");

    let started = Instant::now();
    let patient_docs = futures::stream::iter(0..patients).map(|i| {
        let es = es.clone();
        let t = t.clone();
        async move { put(&*es, &t, "Patient", patient(&format!("p{i}"))).await }
    });
    patient_docs.buffer_unordered(64).collect::<Vec<_>>().await;
    let observation_docs = futures::stream::iter(0..observations).map(|i| {
        let es = es.clone();
        let t = t.clone();
        async move {
            let code = if i % 10 == 0 { OTHER_CODE } else { HEIGHT };
            let subject = format!("Patient/p{}", i % patients);
            put(
                &*es,
                &t,
                "Observation",
                observation(&format!("o{i}"), code, &subject),
            )
            .await
        }
    });
    observation_docs
        .buffer_unordered(64)
        .collect::<Vec<_>>()
        .await;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    eprintln!(
        "BENCH loaded {patients} Patients + {observations} Observations in {:?}",
        started.elapsed()
    );

    for round in ["cold", "warm"] {
        for (label, hide) in [("paged (before)", true), ("hook (after)", false)] {
            let mut counting = Counting::new(es.clone());
            if hide {
                counting = counting.without_hook();
            }
            let begin = Instant::now();
            let resolved = resolve_chains(&counting, &t, &has_height_query())
                .await
                .expect("resolve _has");
            let resolve_time = begin.elapsed();
            let result = counting
                .search(&t, &resolved)
                .await
                .expect("Patient search");
            eprintln!(
                "BENCH {round} {label}: resolve {resolve_time:?}, total {:?}, \
                 Observation searches {}, Patients returned {}",
                begin.elapsed(),
                counting.searches_of("Observation"),
                ids(&result).len()
            );
        }
    }
}

// ============================================================================
// Real PostgreSQL + Elasticsearch composite
// ============================================================================

#[cfg(feature = "postgres")]
mod pg_es {
    use super::*;
    use std::collections::HashMap;

    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::composite::{
        CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
    };
    use helios_persistence::core::BackendKind;
    use testcontainers_modules::postgres::Postgres;

    struct SharedPg {
        host: String,
        port: u16,
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

    async fn shared_pg() -> &'static SharedPg {
        SHARED_PG
            .get_or_init(|| async {
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                let container = container_cleanup::with_cleanup_label(
                    Postgres::default()
                        .with_tag("16-alpine")
                        .with_label("github.run_id", &run_id),
                )
                .start()
                .await
                .expect("start PostgreSQL container");
                let host = container.get_host().await.expect("PG host").to_string();
                let port = container
                    .get_host_port_ipv4(5432)
                    .await
                    .expect("PG host port");
                SharedPg {
                    host,
                    port,
                    _container: container,
                }
            })
            .await
    }

    async fn isolated_postgres_primary() -> Arc<PostgresBackend> {
        let pg = shared_pg().await;
        let dbname = format!("has_chain_{}", uuid::Uuid::new_v4().simple());
        let admin = format!(
            "host={} port={} user=postgres password=postgres dbname=postgres",
            pg.host, pg.port
        );
        let (client, connection) = tokio_postgres::connect(&admin, tokio_postgres::NoTls)
            .await
            .expect("connect to PostgreSQL admin database");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
            .batch_execute(&format!("CREATE DATABASE {dbname}"))
            .await
            .expect("create isolated PostgreSQL database");

        let backend = Arc::new(
            PostgresBackend::new(PostgresConfig {
                host: pg.host.clone(),
                port: pg.port,
                dbname,
                user: "postgres".to_string(),
                password: Some("postgres".to_string()),
                max_connections: 5,
                data_dir: Some(data_dir()),
                search_offloaded: true,
                ..Default::default()
            })
            .await
            .expect("create PostgreSQL primary"),
        );
        backend.init_schema().await.expect("init PostgreSQL schema");
        backend
    }

    /// `_has` over `postgres` (primary) + `es` (search): the shape of the
    /// issue's server. Before #1753 the composite did not forward the hook, so
    /// every Observation that matched was paged through the resolver.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn has_over_postgres_elasticsearch_composite_skips_source_paging() {
        let primary = isolated_postgres_primary().await;
        let es_env = shared_es().await;
        let es = Arc::new(
            ElasticsearchBackend::with_shared_registry(
                es_config(es_env),
                primary.tenant_registries().clone(),
            )
            .expect("create ES backend"),
        );
        initialize_when_ready(&es).await;

        let config = CompositeConfig::builder()
            .primary("postgres", BackendKind::Postgres)
            .search_backend("es", BackendKind::Elasticsearch)
            .sync_mode(SyncMode::Synchronous)
            .build()
            .expect("composite config");
        let mut backends: HashMap<String, DynStorage> = HashMap::new();
        backends.insert("postgres".to_string(), primary.clone() as DynStorage);
        backends.insert("es".to_string(), es.clone() as DynStorage);
        let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
        providers.insert("postgres".to_string(), primary.clone() as DynSearchProvider);
        providers.insert("es".to_string(), es.clone() as DynSearchProvider);
        let composite = Arc::new(
            CompositeStorage::new(config, backends)
                .expect("composite")
                .with_search_providers(providers)
                .with_full_primary(primary.clone()),
        );

        let t = tenant(&format!("has-pg-es-{}", uuid::Uuid::new_v4().simple()));
        for id in ["p1", "p2", "p3"] {
            put(&*composite, &t, "Patient", patient(id)).await;
        }
        put(
            &*composite,
            &t,
            "Observation",
            observation("o1", HEIGHT, "Patient/p1"),
        )
        .await;
        put(
            &*composite,
            &t,
            "Observation",
            observation("o2", HEIGHT, "Patient/p1"),
        )
        .await;
        put(
            &*composite,
            &t,
            "Observation",
            observation("o3", HEIGHT, "Patient/p3"),
        )
        .await;
        put(
            &*composite,
            &t,
            "Observation",
            observation("o4", OTHER_CODE, "Patient/p2"),
        )
        .await;

        // The composite forwards the hook to ES.
        let direct = values(&*composite, &t, "patient")
            .await
            .expect("the composite forwards the hook to its search provider");
        assert_eq!(direct, set(&["Patient/p1", "Patient/p3"]));

        let counting = Counting::new(composite);
        let resolved = resolve_chains(&counting, &t, &has_height_query())
            .await
            .expect("resolve _has");
        assert_eq!(counting.searches_of("Observation"), 0, "no source paging");
        assert_eq!(*counting.reference_calls.lock().unwrap(), 1);

        let result = counting
            .search(&t, &resolved)
            .await
            .expect("Patient search");
        assert_eq!(ids(&result), set(&["p1", "p3"]));
    }
}
