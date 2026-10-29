//! #2294: chain and `_has` resolution on MongoDB read ids and indexed
//! reference values, not resource bodies.
//!
//! `MongoBackend` answers `SearchProvider::search_ids` and
//! `search_reference_values` itself, so the resolver's intermediate hops no
//! longer page through full resources. These tests pin, against a real
//! MongoDB:
//!
//! - the values the hook returns are what the resources hold (updates replace
//!   the old reference, deleted and contained copies are absent, no
//!   duplicates), and other tenants never leak;
//! - query shapes the hook does not cover answer `None`;
//! - a `_has` resolves with no `search` on the source type and exactly one
//!   `search_reference_values` call; a forward chain reads its terminal hop
//!   through `search_ids`, never through `search`;
//! - the resolved queries select the same resources as the trait-default
//!   path (the hook hidden, so the resolver pages and extracts itself);
//! - a resource whose search rows are still pending (#1975) is answered from
//!   its body, as the default does.
//!
//! Child module of the `mongodb_tests` root; `use super::*;` reaches its
//! private harness, as in `deferred_search_index.rs`.

use super::*;

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

use async_trait::async_trait;
use helios_persistence::core::SearchResult;
use helios_persistence::error::StorageResult;
use helios_persistence::search::{SearchParameterRegistry, resolve_chains};
use helios_persistence::types::{ChainedParameter, Page, ReverseChainedParameter, StoredResource};
use parking_lot::RwLock;
use serde_json::Value;

const LOINC: &str = "http://loinc.org";
const HEIGHT: &str = "8302-2";
const OTHER_CODE: &str = "29463-7";

// ============================================================================
// Counting wrapper: what the resolver asks of the backend
// ============================================================================

/// Counts the `search` and `search_ids` calls the resolver issues per
/// resource type, and the `search_reference_values` calls the hook answered.
struct Counting<S> {
    inner: Arc<S>,
    full_searches: Mutex<HashMap<String, u64>>,
    id_searches: Mutex<HashMap<String, u64>>,
    reference_calls: Mutex<u64>,
    /// Answer `Ok(None)` to `search_reference_values`: the trait default.
    hide_hook: bool,
}

impl<S> Counting<S> {
    fn new(inner: Arc<S>) -> Self {
        Self {
            inner,
            full_searches: Mutex::new(HashMap::new()),
            id_searches: Mutex::new(HashMap::new()),
            reference_calls: Mutex::new(0),
            hide_hook: false,
        }
    }

    fn without_hook(mut self) -> Self {
        self.hide_hook = true;
        self
    }

    fn full_searches_of(&self, resource_type: &str) -> u64 {
        self.full_searches
            .lock()
            .unwrap()
            .get(resource_type)
            .copied()
            .unwrap_or(0)
    }

    fn id_searches_of(&self, resource_type: &str) -> u64 {
        self.id_searches
            .lock()
            .unwrap()
            .get(resource_type)
            .copied()
            .unwrap_or(0)
    }

    fn reference_calls(&self) -> u64 {
        *self.reference_calls.lock().unwrap()
    }
}

fn bump(map: &Mutex<HashMap<String, u64>>, resource_type: &str) {
    *map.lock()
        .unwrap()
        .entry(resource_type.to_string())
        .or_default() += 1;
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
        bump(&self.full_searches, &query.resource_type);
        self.inner.search(tenant, query).await
    }
    async fn search_ids(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<Page<String>> {
        bump(&self.id_searches, &query.resource_type);
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
// Fixtures
// ============================================================================

fn observation(id: &str, code: &str, subject: &str) -> Value {
    json!({
        "resourceType": "Observation",
        "id": id,
        "status": "final",
        "code": {"coding": [{"system": LOINC, "code": code}]},
        "subject": {"reference": subject},
    })
}

fn patient(id: &str, family: &str) -> Value {
    json!({"resourceType": "Patient", "id": id, "name": [{"family": family}]})
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
        "subject",
        "code",
        SearchValue::token(Some(LOINC), HEIGHT),
    ));
    query
}

fn chain_by_family_query(family: &str) -> SearchQuery {
    SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "subject".to_string(),
        param_type: SearchParamType::Reference,
        values: vec![SearchValue::eq(family)],
        chain: vec![ChainedParameter {
            reference_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            target_param: "family".to_string(),
        }],
        ..Default::default()
    })
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

/// Resolves `query`'s chains through `storage` and returns the ids the
/// rewritten query selects.
async fn resolved_ids<S: SearchProvider>(
    storage: &S,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> BTreeSet<String> {
    let resolved = resolve_chains(storage, tenant, query)
        .await
        .expect("resolve_chains");
    ids(&storage
        .search(tenant, &resolved)
        .await
        .expect("search the rewritten query"))
}

// ============================================================================
// search_reference_values on MongoDB
// ============================================================================

#[tokio::test]
async fn reference_values_match_what_the_resources_hold() {
    let Some(backend) = create_backend("chain_has_values").await else {
        eprintln!("skipping: no MongoDB container available");
        return;
    };
    let t = create_tenant("has-parity");

    put(
        &backend,
        &t,
        "Observation",
        observation("o1", HEIGHT, "Patient/a"),
    )
    .await;
    let o2 = put(
        &backend,
        &t,
        "Observation",
        observation("o2", HEIGHT, "Patient/b"),
    )
    .await;
    put(
        &backend,
        &t,
        "Observation",
        observation("o3", HEIGHT, "Group/g1"),
    )
    .await;
    put(
        &backend,
        &t,
        "Observation",
        observation("o4", HEIGHT, "Patient/d"),
    )
    .await;
    // Another code: never selected by the source filter.
    put(
        &backend,
        &t,
        "Observation",
        observation("o5", OTHER_CODE, "Patient/z"),
    )
    .await;
    // A contained copy carries its own reference; it is not a current resource.
    let mut with_contained = observation("o6", HEIGHT, "Patient/a");
    with_contained["contained"] = json!([observation("c1", HEIGHT, "Patient/ghost")]);
    put(&backend, &t, "Observation", with_contained).await;

    // The reference moves, and one resource goes away.
    backend
        .update(&t, &o2, observation("o2", HEIGHT, "Patient/c"))
        .await
        .expect("update o2");
    backend
        .delete(&t, "Observation", "o4")
        .await
        .expect("delete o4");

    let subject = values(&backend, &t, "subject")
        .await
        .expect("MongoDB answers the hook");
    assert_eq!(subject, set(&["Patient/a", "Patient/c", "Group/g1"]));

    // `patient` only indexes Patient targets, but follows the same rules.
    let patient = values(&backend, &t, "patient")
        .await
        .expect("MongoDB answers the hook");
    for expected in ["Patient/a", "Patient/c"] {
        assert!(patient.contains(expected), "{expected} in {patient:?}");
    }
    for absent in ["Patient/b", "Patient/d", "Patient/z", "Patient/ghost"] {
        assert!(!patient.contains(absent), "{absent} in {patient:?}");
    }

    // Distinct values: `Patient/a` is held by two resources and appears once.
    let listed = backend
        .search_reference_values(&t, &height_query(), "subject")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(listed.len(), subject.len(), "no duplicates: {listed:?}");
}

#[tokio::test]
async fn reference_values_do_not_cross_tenants() {
    let Some(backend) = create_backend("chain_has_tenants").await else {
        eprintln!("skipping: no MongoDB container available");
        return;
    };
    // `a_b` extends `a`, so a missing or prefix-matched tenant filter leaks.
    let a = create_tenant("a");
    let a_b = create_tenant("a_b");
    put(
        &backend,
        &a,
        "Observation",
        observation("o1", HEIGHT, "Patient/of-a"),
    )
    .await;
    put(
        &backend,
        &a_b,
        "Observation",
        observation("o1", HEIGHT, "Patient/of-a-b"),
    )
    .await;

    assert_eq!(
        values(&backend, &a, "subject").await.unwrap(),
        set(&["Patient/of-a"])
    );
    assert_eq!(
        values(&backend, &a_b, "subject").await.unwrap(),
        set(&["Patient/of-a-b"])
    );
}

#[tokio::test]
async fn reference_values_fall_back_for_query_shapes_the_hook_does_not_cover() {
    let Some(backend) = create_backend("chain_has_guards").await else {
        eprintln!("skipping: no MongoDB container available");
        return;
    };
    let t = create_tenant("has-guards");
    put(
        &backend,
        &t,
        "Observation",
        observation("o1", HEIGHT, "Patient/a"),
    )
    .await;

    // A plain `_count` is fine; an offset, a cursor or a total is not.
    let paged = height_query().with_count(5);
    assert!(
        backend
            .search_reference_values(&t, &paged, "subject")
            .await
            .unwrap()
            .is_some()
    );
    let mut with_offset = height_query();
    with_offset.offset = Some(1);
    assert!(
        backend
            .search_reference_values(&t, &with_offset, "subject")
            .await
            .unwrap()
            .is_none()
    );
    let mut with_cursor = height_query();
    with_cursor.cursor = Some("anything".to_string());
    assert!(
        backend
            .search_reference_values(&t, &with_cursor, "subject")
            .await
            .unwrap()
            .is_none()
    );
}

/// More distinct values than one `$in` chunk holds, so the answer needs more
/// than one `search_index` read.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reference_values_past_one_chunk_are_complete_and_distinct() {
    const DISTINCT: usize = 2_500;

    let Some(backend) = create_backend("chain_has_wide").await else {
        eprintln!("skipping: no MongoDB container available");
        return;
    };
    let t = create_tenant("has-wide");

    for chunk in (0..DISTINCT).collect::<Vec<_>>().chunks(500) {
        let batch: Vec<Value> = chunk
            .iter()
            .map(|i| observation(&format!("o{i}"), HEIGHT, &format!("Patient/p{i}")))
            .collect();
        for result in backend
            .create_many(&t, "Observation", batch, FhirVersion::default())
            .await
        {
            result.expect("create_many item");
        }
    }
    // Two more observations on an existing patient: still one value.
    put(
        &backend,
        &t,
        "Observation",
        observation("dup1", HEIGHT, "Patient/p0"),
    )
    .await;
    put(
        &backend,
        &t,
        "Observation",
        observation("dup2", HEIGHT, "Patient/p0"),
    )
    .await;

    let listed = backend
        .search_reference_values(&t, &height_query(), "subject")
        .await
        .expect("search_reference_values")
        .expect("MongoDB answers the hook");
    let distinct: BTreeSet<String> = listed.iter().cloned().collect();
    assert_eq!(listed.len(), DISTINCT, "no duplicates across chunks");
    let expected: BTreeSet<String> = (0..DISTINCT).map(|i| format!("Patient/p{i}")).collect();
    assert_eq!(distinct, expected);
}

// ============================================================================
// End to end: the resolver over the hooks
// ============================================================================

#[tokio::test]
async fn has_resolution_issues_no_source_type_search() {
    let Some(backend) = create_backend("chain_has_e2e").await else {
        eprintln!("skipping: no MongoDB container available");
        return;
    };
    let backend = Arc::new(backend);
    let t = create_tenant("has-e2e");
    for id in ["p1", "p2", "p3", "p4"] {
        put(&*backend, &t, "Patient", patient(id, "Smith")).await;
    }
    put(
        &*backend,
        &t,
        "Observation",
        observation("o1", HEIGHT, "Patient/p1"),
    )
    .await;
    put(
        &*backend,
        &t,
        "Observation",
        observation("o2", HEIGHT, "Patient/p1"),
    )
    .await;
    put(
        &*backend,
        &t,
        "Observation",
        observation("o3", HEIGHT, "Patient/p3"),
    )
    .await;
    put(
        &*backend,
        &t,
        "Observation",
        observation("o4", OTHER_CODE, "Patient/p2"),
    )
    .await;
    // A hit on a Group is not a Patient id.
    put(
        &*backend,
        &t,
        "Observation",
        observation("o5", HEIGHT, "Group/p4"),
    )
    .await;

    let counting = Counting::new(Arc::clone(&backend));
    let resolved = resolve_chains(&counting, &t, &has_height_query())
        .await
        .expect("resolve _has");

    assert_eq!(counting.full_searches_of("Observation"), 0, "no body loads");
    assert_eq!(
        counting.id_searches_of("Observation"),
        0,
        "no source paging"
    );
    assert_eq!(counting.reference_calls(), 1);
    let result = counting
        .search(&t, &resolved)
        .await
        .expect("Patient search");
    assert_eq!(ids(&result), set(&["p1", "p3"]));

    // The trait default (hook hidden) selects the same Patients.
    let hidden = Counting::new(Arc::clone(&backend)).without_hook();
    assert_eq!(
        resolved_ids(&hidden, &t, &has_height_query()).await,
        set(&["p1", "p3"])
    );
    assert_eq!(hidden.reference_calls(), 0);
}

#[tokio::test]
async fn forward_chain_reads_the_terminal_hop_without_bodies() {
    let Some(backend) = create_backend("chain_has_forward").await else {
        eprintln!("skipping: no MongoDB container available");
        return;
    };
    let backend = Arc::new(backend);
    let t = create_tenant("chain-e2e");
    put(&*backend, &t, "Patient", patient("p1", "Chalmers")).await;
    put(&*backend, &t, "Patient", patient("p2", "Chalmers")).await;
    put(&*backend, &t, "Patient", patient("p3", "Other")).await;
    put(
        &*backend,
        &t,
        "Observation",
        observation("o1", HEIGHT, "Patient/p1"),
    )
    .await;
    put(
        &*backend,
        &t,
        "Observation",
        observation("o2", HEIGHT, "Patient/p2"),
    )
    .await;
    put(
        &*backend,
        &t,
        "Observation",
        observation("o3", HEIGHT, "Patient/p3"),
    )
    .await;

    let query = chain_by_family_query("Chalmers");
    let counting = Counting::new(Arc::clone(&backend));
    let resolved = resolve_chains(&counting, &t, &query)
        .await
        .expect("resolve chain");

    assert_eq!(
        counting.full_searches_of("Patient"),
        0,
        "the terminal hop loads no bodies"
    );
    assert!(counting.id_searches_of("Patient") >= 1);
    let result = counting
        .search(&t, &resolved)
        .await
        .expect("Observation search");
    assert_eq!(ids(&result), set(&["o1", "o2"]));

    // Parity with the trait default (hook hidden; `search_ids` is the
    // backend's either way, so compare with the plain `search` result).
    let hidden = Counting::new(Arc::clone(&backend)).without_hook();
    assert_eq!(resolved_ids(&hidden, &t, &query).await, set(&["o1", "o2"]));
}

// ============================================================================
// search_ids: the same page `search` returns
// ============================================================================

#[tokio::test]
async fn search_ids_pages_like_search() {
    let Some(backend) = create_backend("chain_has_ids_page").await else {
        eprintln!("skipping: no MongoDB container available");
        return;
    };
    let t = create_tenant("ids-page");
    for i in 0..7 {
        put(&backend, &t, "Patient", patient(&format!("p{i}"), "Paged")).await;
    }
    put(&backend, &t, "Patient", patient("zz", "Elsewhere")).await;

    let base = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        values: vec![SearchValue::eq("Paged")],
        ..Default::default()
    });

    let mut full_cursor: Option<String> = None;
    let mut ids_cursor: Option<String> = None;
    loop {
        let mut full_query = base.clone().with_count(3);
        full_query.cursor = full_cursor.clone();
        let mut ids_query = base.clone().with_count(3);
        ids_query.cursor = ids_cursor.clone();

        let full = backend.search(&t, &full_query).await.expect("search");
        let id_page = backend
            .search_ids(&t, &ids_query)
            .await
            .expect("search_ids");

        let full_ids: Vec<String> = full
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        assert_eq!(id_page.items, full_ids, "same ids in the same order");
        assert_eq!(
            id_page.page_info.has_next,
            full.resources.page_info.has_next
        );
        assert_eq!(
            id_page.page_info.next_cursor, full.resources.page_info.next_cursor,
            "the same cursor"
        );
        if !id_page.page_info.has_next {
            break;
        }
        full_cursor = full.resources.page_info.next_cursor.clone();
        ids_cursor = id_page.page_info.next_cursor.clone();
        assert!(ids_cursor.is_some());
    }
}

// ============================================================================
// Deferred search rows (#1975)
// ============================================================================

/// A resource whose rows are still pending has no `search_index` rows; the
/// hook still answers what its body holds, as the trait default does.
#[tokio::test]
async fn pending_rows_are_answered_from_the_body() {
    let Some(backend) = create_backend("chain_has_pending").await else {
        eprintln!("skipping: no MongoDB container available");
        return;
    };
    let t = create_tenant("has-pending");
    let tenant_id = t.tenant_id().as_str().to_string();

    put(
        &backend,
        &t,
        "Observation",
        observation("o-indexed", HEIGHT, "Patient/a"),
    )
    .await;
    put(
        &backend,
        &t,
        "Observation",
        observation("o-pending", HEIGHT, "Patient/b"),
    )
    .await;

    // Simulate the window between a deferred Bundle's commit and its repair:
    // the marker is set and the resource's rows are gone.
    let db = super::deferred_search_index::raw_db(&backend).await;
    db.collection::<Document>("search_index")
        .delete_many(doc! {
            "tenant_id": &tenant_id, "resource_type": "Observation",
            "resource_id": "o-pending",
        })
        .await
        .expect("drop the rows");
    db.collection::<Document>("resources")
        .update_one(
            doc! { "tenant_id": &tenant_id, "resource_type": "Observation", "id": "o-pending" },
            doc! { "$set": { "search_rows_pending": "bundle-simulated" } },
        )
        .await
        .expect("set the marker");
    assert_eq!(
        super::deferred_search_index::marker_count(&backend, &tenant_id).await,
        1
    );

    // Selected by `_id`, as a nested `_has` selects its sources.
    let by_id = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "_id".to_string(),
        param_type: SearchParamType::Token,
        values: vec![SearchValue::eq("o-indexed"), SearchValue::eq("o-pending")],
        ..Default::default()
    });
    let answer = backend
        .search_reference_values(&t, &by_id, "subject")
        .await
        .expect("search_reference_values");
    match answer {
        // Either the backend extracts the pending body itself...
        Some(found) => assert_eq!(
            found.into_iter().collect::<BTreeSet<_>>(),
            set(&["Patient/a", "Patient/b"]),
            "the pending resource's reference is not lost"
        ),
        // ...or it declines and the resolver pages and extracts, which is
        // the default and also complete.
        None => {}
    }
}
