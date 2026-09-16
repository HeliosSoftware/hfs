//! #1047: on a composite, lookups that resolve a write against existing
//! content (a transaction's conditional references, an `If-None-Exist`
//! create) read the search secondary, which lags the primary the server has
//! already acknowledged the write from. `ensure_writes_visible` closes that
//! gap; `conditional_create` must use it.
//!
//! The secondary here is a SQLite backend behind a wrapper that delays every
//! indexing write, so the lag is deterministic rather than a race against
//! the sync worker's batch window: without the barrier, a lookup issued
//! right after a create *always* misses.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
};
use helios_persistence::core::search::{SearchProvider, SearchResult};
use helios_persistence::core::{
    BackendKind, ConditionalCreateResult, ConditionalStorage, ResourceStorage,
};
use helios_persistence::error::StorageResult;
use helios_persistence::search::SearchParameterRegistry;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchQuery, SearchValue, StoredResource,
};
use serde_json::{Value, json};

/// How long the search secondary sits on each write before indexing it.
const INDEX_LAG: Duration = Duration::from_millis(300);

fn tenant() -> TenantContext {
    TenantContext::new(TenantId::new("default"), TenantPermissions::full_access())
}

fn sqlite() -> SqliteBackend {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .expect("workspace data dir");
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("sqlite");
    backend.init_schema().expect("schema");
    backend
}

/// A search secondary whose writes land late: the shape of an Elasticsearch
/// index between a write's acknowledgement and its next refresh, or of an
/// asynchronous sync queue that has not drained yet.
struct LaggingIndex {
    inner: SqliteBackend,
}

#[async_trait]
impl ResourceStorage for LaggingIndex {
    fn backend_name(&self) -> &'static str {
        "lagging-index"
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        tokio::time::sleep(INDEX_LAG).await;
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
        tokio::time::sleep(INDEX_LAG).await;
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
        tokio::time::sleep(INDEX_LAG).await;
        self.inner.update(tenant, current, resource).await
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        tokio::time::sleep(INDEX_LAG).await;
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
impl SearchProvider for LaggingIndex {
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

/// A production-shaped composite: primary with its own index offloaded, a
/// dedicated search secondary, writes synced in the given mode.
fn composite(mode: SyncMode) -> CompositeStorage {
    let mut primary = sqlite();
    primary.set_search_offloaded(true);
    let primary = Arc::new(primary);
    let index = Arc::new(LaggingIndex { inner: sqlite() });

    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("search", BackendKind::Sqlite)
        .sync_mode(mode)
        .build()
        .expect("composite config");

    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    backends.insert("search".to_string(), index.clone() as DynStorage);
    let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
    providers.insert("sqlite".to_string(), primary.clone() as DynSearchProvider);
    providers.insert("search".to_string(), index as DynSearchProvider);

    CompositeStorage::new(config, backends)
        .expect("composite")
        .with_search_providers(providers)
        .with_full_primary(primary)
        .start_sync_workers()
}

fn organization(identifier: &str) -> Value {
    json!({
        "resourceType": "Organization",
        "identifier": [{"system": "urn:zzz:probe", "value": identifier}],
        "name": "ZZZ Probe Org"
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

/// The primitive itself, on the default (asynchronous) sync mode: the write
/// is still in the queue, and behind a slow index once it leaves it. After
/// `ensure_writes_visible` a search finds it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_after_ensure_writes_visible_sees_the_queued_write() {
    let composite = composite(SyncMode::Asynchronous);
    let t = tenant();

    let created = composite
        .create(&t, "Organization", organization("ORG-1"), FhirVersion::R4)
        .await
        .expect("create through composite");

    composite
        .ensure_writes_visible(&t, &["Organization"])
        .await
        .expect("ensure_writes_visible");

    let found = composite
        .search(&t, &by_identifier("ORG-1"))
        .await
        .expect("search through composite");
    assert_eq!(
        found.resources.items.len(),
        1,
        "a write acknowledged before ensure_writes_visible must be searchable after it"
    );
    assert_eq!(found.resources.items[0].id(), created.id());
}

/// `If-None-Exist` on a composite resolves its criteria through the same
/// lagging index. Issued right after the matching create, it must report
/// the existing resource rather than create the duplicate it exists to
/// prevent (#1047).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conditional_create_right_after_create_reports_existing() {
    let composite = composite(SyncMode::Asynchronous);
    let t = tenant();

    let created = composite
        .create(&t, "Organization", organization("ORG-2"), FhirVersion::R4)
        .await
        .expect("create through composite");

    let outcome = composite
        .conditional_create(
            &t,
            "Organization",
            organization("ORG-2"),
            "identifier=urn:zzz:probe|ORG-2",
            FhirVersion::R4,
        )
        .await
        .expect("conditional create");

    match outcome {
        ConditionalCreateResult::Exists(existing) => {
            assert_eq!(existing.id(), created.id());
        }
        other => panic!(
            "expected Exists for a resource created moments earlier, got {}",
            match other {
                ConditionalCreateResult::Created(_) => "Created (a duplicate)".to_string(),
                ConditionalCreateResult::MultipleMatches(n) => format!("MultipleMatches({n})"),
                ConditionalCreateResult::Exists(_) => unreachable!(),
            }
        ),
    }
    assert_eq!(
        composite.count(&t, Some("Organization")).await.unwrap(),
        1,
        "the conditional create must not have written a second Organization"
    );
}

/// A genuine no-match is still a no-match: visibility is about writes that
/// happened, not about inventing them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conditional_create_with_no_match_still_creates() {
    let composite = composite(SyncMode::Asynchronous);
    let t = tenant();

    composite
        .create(&t, "Organization", organization("ORG-3"), FhirVersion::R4)
        .await
        .expect("create through composite");

    let outcome = composite
        .conditional_create(
            &t,
            "Organization",
            organization("ORG-OTHER"),
            "identifier=urn:zzz:probe|ORG-OTHER",
            FhirVersion::R4,
        )
        .await
        .expect("conditional create");
    assert!(
        matches!(outcome, ConditionalCreateResult::Created(_)),
        "no existing match: the conditional create must create"
    );
}

/// Under synchronous sync there is no queue to drain: the barrier is a
/// no-op and the search backend's own step is all that runs. The lookup
/// still sees the write, and nothing waits on a worker that was never
/// started.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn synchronous_mode_needs_no_queue_barrier() {
    let composite = composite(SyncMode::Synchronous);
    let t = tenant();

    let created = composite
        .create(&t, "Organization", organization("ORG-4"), FhirVersion::R4)
        .await
        .expect("create through composite");

    let started = std::time::Instant::now();
    composite
        .ensure_writes_visible(&t, &["Organization"])
        .await
        .expect("ensure_writes_visible");
    assert!(
        started.elapsed() < INDEX_LAG,
        "synchronous sync already waited for the index; the barrier must not wait again"
    );

    let found = composite
        .search(&t, &by_identifier("ORG-4"))
        .await
        .expect("search");
    assert_eq!(found.resources.items.len(), 1);
    assert_eq!(found.resources.items[0].id(), created.id());
}
