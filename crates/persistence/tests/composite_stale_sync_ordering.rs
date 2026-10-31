//! Stale syncs against a real Elasticsearch cluster (#2288).
//!
//! Elasticsearch documents are written without an ordering guard, so a late
//! sync of an older version can replace a newer document, and a late sync can
//! bring back a resource that was deleted. These tests pin the contract the
//! fix has to meet, using only the API that exists today
//! (`ReindexTarget::write_search_entries`, `write_synced_entries_page`,
//! `ResourceStorage::delete`, and the composite storage):
//!
//! - a late or retried sync never replaces a newer document or resurrects a
//!   deleted one, even after `index.gc_deletes` has expired;
//! - the same write sent twice stays idempotent (never an error);
//! - contained documents follow their container: a late writer neither
//!   re-adds contained documents of an older version nor orphans them under a
//!   deleted container;
//! - documents written before the guard existed (many unguarded PUTs, so a
//!   large internal `_version`) still accept the first guarded write;
//! - a resource may be recreated after its delete.
//!
//! Until the guard lands, the tests named `late_*`, `*_is_not_resurrected*`
//! and `*_orphan*` FAIL: that is the reproduction the issue asks for. The
//! idempotent, migration and recreate tests pass today and must keep passing.
//!
//! Set `HFS_TEST_ES_URL` (for example `http://127.0.0.1:9200`) to use a
//! cluster that is already up; otherwise a 7.17.29 container is started.
//!
//! Run with:
//! `cargo test -p helios-persistence --features R4,elasticsearch,sqlite --test composite_stale_sync_ordering`

#![cfg(all(feature = "elasticsearch", feature = "sqlite"))]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use elasticsearch::Elasticsearch;
use elasticsearch::http::transport::Transport;
use helios_fhir::FhirVersion;
use helios_persistence::backends::elasticsearch::{
    ElasticsearchBackend, ElasticsearchConfig, WriteRefreshPolicy,
};
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
};
use helios_persistence::core::{Backend, BackendKind, ResourceStorage, SearchProvider};
use helios_persistence::search::{ReindexTarget, SearchParameterLoader, TenantSearchRegistries};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    ContainedMode, ContainedReturn, SearchParamType, SearchParameter, SearchQuery, SearchValue,
    StoredResource,
};
use serde_json::{Value, json};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::elastic_search::ElasticSearch;
use tokio::sync::OnceCell;

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

const ES_IMAGE_TAG: &str = "7.17.29";
const ES_STARTUP_TIMEOUT: Duration = Duration::from_secs(300);

struct SharedEs {
    addr: String,
    _container: Option<testcontainers::ContainerAsync<ElasticSearch>>,
}

static SHARED_ES: OnceCell<SharedEs> = OnceCell::const_new();

async fn es_addr() -> &'static str {
    &SHARED_ES
        .get_or_init(|| async {
            if let Ok(url) = std::env::var("HFS_TEST_ES_URL") {
                return SharedEs {
                    addr: url
                        .trim_start_matches("http://")
                        .trim_end_matches('/')
                        .to_string(),
                    _container: None,
                };
            }
            let container = container_cleanup::with_cleanup_label(
                ElasticSearch::default()
                    .with_tag(ES_IMAGE_TAG)
                    .with_env_var("ES_JAVA_OPTS", "-Xms256m -Xmx256m")
                    .with_startup_timeout(ES_STARTUP_TIMEOUT),
            )
            .start()
            .await
            .expect("Failed to start Elasticsearch container");
            let port = container.get_host_port_ipv4(9200).await.expect("port");
            let host = container.get_host().await.expect("host");
            SharedEs {
                addr: format!("{host}:{port}"),
                _container: Some(container),
            }
        })
        .await
        .addr
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn search_registry() -> Arc<TenantSearchRegistries> {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let loader = SearchParameterLoader::new(FhirVersion::default());
    let registries = Arc::new(TenantSearchRegistries::base_only());
    {
        let mut registry = registries.base().write();
        for param in loader.load_embedded().expect("embedded params") {
            let _ = registry.register(param);
        }
        for param in loader
            .load_from_spec_file(&data_dir)
            .expect("spec search parameters")
        {
            let _ = registry.register(param);
        }
    }
    registries
}

fn new_prefix() -> String {
    let base = std::env::var("HFS_TEST_ES_INDEX_PREFIX").unwrap_or_else(|_| "hfs".to_string());
    format!("{base}_{}", uuid::Uuid::new_v4().simple())
}

fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

async fn backend(prefix: &str) -> ElasticsearchBackend {
    let config = ElasticsearchConfig {
        nodes: vec![format!("http://{}", es_addr().await)],
        index_prefix: prefix.to_string(),
        number_of_replicas: 0,
        refresh_interval: "1ms".to_string(),
        write_refresh: WriteRefreshPolicy::WaitFor,
        ..Default::default()
    };
    let backend = ElasticsearchBackend::with_shared_registry(config, search_registry())
        .expect("create ES backend");
    backend.initialize().await.expect("initialize ES backend");
    backend
}

fn raw_client(addr: &str) -> Elasticsearch {
    Elasticsearch::new(Transport::single_node(&format!("http://{addr}")).unwrap())
}

async fn cleanup(prefix: &str) {
    let client = raw_client(es_addr().await);
    let pattern = format!("{prefix}_*");
    if let Ok(response) = client
        .cat()
        .indices(elasticsearch::cat::CatIndicesParts::Index(&[&pattern]))
        .h(&["index"])
        .format("json")
        .send()
        .await
        && let Ok(rows) = response.json::<Value>().await
    {
        for name in rows
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| r["index"].as_str())
        {
            let _ = client
                .indices()
                .delete(elasticsearch::indices::IndicesDeleteParts::Index(&[name]))
                .send()
                .await;
        }
    }
    let _ = client
        .indices()
        .delete_template(elasticsearch::indices::IndicesDeleteTemplateParts::Name(
            &format!("{prefix}_template"),
        ))
        .send()
        .await;
}

/// Makes Elasticsearch forget a delete immediately instead of after the
/// default 60 s, so a test can prove the protection does not rely on it.
async fn expire_deletes_immediately(prefix: &str) {
    let client = raw_client(es_addr().await);
    let pattern = format!("{prefix}_*");
    let response = client
        .indices()
        .put_settings(elasticsearch::indices::IndicesPutSettingsParts::Index(&[
            &pattern,
        ]))
        .body(json!({ "index": { "gc_deletes": "0s" } }))
        .send()
        .await
        .expect("put gc_deletes");
    assert!(response.status_code().is_success(), "gc_deletes setting");
}

/// A Patient as the primary would hand it to the secondary at `version`.
/// `org` adds a contained Organization named `org`.
fn patient(id: &str, version: u64, family: &str, org: Option<&str>) -> StoredResource {
    let mut content = json!({
        "resourceType": "Patient", "id": id,
        "meta": { "versionId": version.to_string() },
        "name": [{ "family": family }],
    });
    if let Some(org) = org {
        content["contained"] =
            json!([{ "resourceType": "Organization", "id": "org", "name": org }]);
        content["managingOrganization"] = json!({ "reference": "#org" });
    }
    let now = Utc::now();
    StoredResource::from_storage(
        "Patient",
        id,
        version.to_string(),
        TenantId::new("stale-sync"),
        content,
        now,
        now,
        None,
        FhirVersion::default(),
    )
}

fn t() -> TenantContext {
    tenant("stale-sync")
}

/// Ids of the Patients whose family is `family`, as the search sees them.
async fn found_by_family(es: &ElasticsearchBackend, family: &str) -> Vec<String> {
    let t = t();
    es.refresh_index(t.tenant_id().as_str(), "Patient")
        .await
        .ok();
    let mut q = SearchQuery::new("Patient");
    q.parameters.push(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq(family)],
        chain: vec![],
        components: vec![],
    });
    let mut ids: Vec<String> = es
        .search(&t, &q)
        .await
        .expect("search")
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect();
    ids.sort();
    ids
}

/// The version `read` returns, `None` when the resource is absent or deleted.
async fn read_version(es: &ElasticsearchBackend, id: &str) -> Option<String> {
    match es.read(&t(), "Patient", id).await {
        Ok(Some(r)) => Some(r.version_id().to_string()),
        Ok(None) => None,
        Err(e) => panic!("read failed: {e}"),
    }
}

/// Containers of contained Organizations named `name`.
async fn containers_of(es: &ElasticsearchBackend, name: &str) -> Vec<String> {
    let t = t();
    es.refresh_index(t.tenant_id().as_str(), "Organization")
        .await
        .ok();
    let mut q = SearchQuery::new("Organization");
    q.contained = ContainedMode::On;
    q.contained_return = ContainedReturn::Container;
    q.parameters.push(SearchParameter {
        name: "name".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq(name)],
        chain: vec![],
        components: vec![],
    });
    let mut urls: Vec<String> = es
        .search(&t, &q)
        .await
        .expect("contained search")
        .resources
        .items
        .iter()
        .map(|r| r.url())
        .collect();
    urls.sort();
    urls
}

/// Contained documents the cluster holds, whether or not their container's
/// own document exists.
async fn contained_doc_count(prefix: &str) -> u64 {
    let client = raw_client(es_addr().await);
    let pattern = format!("{prefix}_*");
    let _ = client
        .indices()
        .refresh(elasticsearch::indices::IndicesRefreshParts::Index(&[
            &pattern,
        ]))
        .send()
        .await;
    let body = client
        .count(elasticsearch::CountParts::Index(&[&pattern]))
        .body(json!({ "query": { "term": { "is_contained": true } } }))
        .send()
        .await
        .expect("count")
        .json::<Value>()
        .await
        .expect("count body");
    body["count"].as_u64().expect("count")
}

// ---------------------------------------------------------------------------
// Late writes after a newer version
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_v1_does_not_replace_v2() {
    let prefix = new_prefix();
    let es = backend(&prefix).await;
    let t = t();

    es.write_search_entries(&t, &patient("p1", 1, "Onefam", None))
        .await
        .expect("v1");
    es.write_search_entries(&t, &patient("p1", 2, "Twofam", None))
        .await
        .expect("v2");
    // A late retry of v1: never an error the caller must handle, never a
    // replacement of the newer document.
    let late = es
        .write_search_entries(&t, &patient("p1", 1, "Onefam", None))
        .await;
    assert!(late.is_ok(), "a superseded sync is a success, got {late:?}");

    assert_eq!(read_version(&es, "p1").await.as_deref(), Some("2"));
    assert_eq!(found_by_family(&es, "Twofam").await, vec!["p1"]);
    assert!(
        found_by_family(&es, "Onefam").await.is_empty(),
        "the older version's values are not searchable"
    );
    cleanup(&prefix).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_page_does_not_replace_newer_documents() {
    let prefix = new_prefix();
    let es = backend(&prefix).await;
    let t = t();

    es.write_search_entries(&t, &patient("p1", 2, "Twofam", None))
        .await
        .expect("v2");

    // The bulk paths: the async worker's page, then a $reindex page.
    let synced = es
        .write_synced_entries_page(&t, &[patient("p1", 1, "Onefam", None)])
        .await;
    assert!(
        synced.iter().all(|r| r.is_ok()),
        "a superseded bulk item is a success, got {synced:?}"
    );
    assert_eq!(read_version(&es, "p1").await.as_deref(), Some("2"));
    assert!(found_by_family(&es, "Onefam").await.is_empty());

    let reindexed = es
        .write_search_entries_page(&t, &[patient("p1", 1, "Onefam", None)])
        .await;
    assert!(
        reindexed.iter().all(|r| r.is_ok()),
        "a superseded reindex item is a success, got {reindexed:?}"
    );
    assert_eq!(read_version(&es, "p1").await.as_deref(), Some("2"));
    assert_eq!(found_by_family(&es, "Twofam").await, vec!["p1"]);
    assert!(found_by_family(&es, "Onefam").await.is_empty());
    cleanup(&prefix).await;
}

// ---------------------------------------------------------------------------
// Late writes after a delete
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleted_resource_is_not_resurrected_by_a_late_sync() {
    let prefix = new_prefix();
    let es = backend(&prefix).await;
    let t = t();

    es.write_search_entries(&t, &patient("p1", 1, "Onefam", None))
        .await
        .expect("v1");
    // A versioned delete (v2 on the primary) leaves a tombstone; a bare
    // `delete` is a physical purge and orders nothing.
    es.tombstone_search_entries(&t, "Patient", "p1", "2")
        .await
        .expect("tombstone");
    assert_eq!(read_version(&es, "p1").await, None);

    // The delete was version 2 on the primary; the late write is version 1.
    es.write_search_entries(&t, &patient("p1", 1, "Onefam", None))
        .await
        .expect("late v1 is a success");
    assert_eq!(read_version(&es, "p1").await, None, "still deleted");
    assert!(found_by_family(&es, "Onefam").await.is_empty());

    let page = es
        .write_synced_entries_page(&t, &[patient("p1", 1, "Onefam", None)])
        .await;
    assert!(page.iter().all(|r| r.is_ok()), "bulk late v1: {page:?}");
    assert_eq!(read_version(&es, "p1").await, None, "still deleted");
    assert!(found_by_family(&es, "Onefam").await.is_empty());
    cleanup(&prefix).await;
}

/// Elasticsearch keeps the version of a deleted document only for
/// `index.gc_deletes` (60 s by default). The protection must not depend on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleted_resource_is_not_resurrected_after_gc_deletes_expired() {
    let prefix = new_prefix();
    let es = backend(&prefix).await;
    let t = t();

    es.write_search_entries(&t, &patient("p1", 1, "Onefam", None))
        .await
        .expect("v1");
    expire_deletes_immediately(&prefix).await;
    // A versioned delete (v2 on the primary) leaves a tombstone; a bare
    // `delete` is a physical purge and carries no ordering information.
    es.tombstone_search_entries(&t, "Patient", "p1", "2")
        .await
        .expect("tombstone");
    // Give the gc a chance to forget the delete, then keep the shard busy so
    // the pruning runs.
    tokio::time::sleep(Duration::from_secs(3)).await;
    es.write_search_entries(&t, &patient("other", 1, "Otherfam", None))
        .await
        .expect("unrelated write");
    tokio::time::sleep(Duration::from_secs(2)).await;

    es.write_search_entries(&t, &patient("p1", 1, "Onefam", None))
        .await
        .expect("late v1 is a success");
    assert_eq!(read_version(&es, "p1").await, None, "still deleted");
    assert!(found_by_family(&es, "Onefam").await.is_empty());
    cleanup(&prefix).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recreate_after_delete_is_visible() {
    let prefix = new_prefix();
    let es = backend(&prefix).await;
    let t = t();

    es.write_search_entries(&t, &patient("p1", 1, "Onefam", None))
        .await
        .expect("v1");
    es.delete(&t, "Patient", "p1").await.expect("delete (v2)");
    // The primary numbers the recreate after the delete: v3.
    es.write_search_entries(&t, &patient("p1", 3, "Threefam", None))
        .await
        .expect("recreate v3");
    assert_eq!(read_version(&es, "p1").await.as_deref(), Some("3"));
    assert_eq!(found_by_family(&es, "Threefam").await, vec!["p1"]);
    cleanup(&prefix).await;
}

// ---------------------------------------------------------------------------
// Retries and migration (pass today, must keep passing)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resending_the_same_write_is_idempotent() {
    let prefix = new_prefix();
    let es = backend(&prefix).await;
    let t = t();

    for _ in 0..3 {
        es.write_search_entries(&t, &patient("p1", 2, "Twofam", None))
            .await
            .expect("same write, again");
    }
    for _ in 0..2 {
        let page = es
            .write_synced_entries_page(&t, &[patient("p1", 2, "Twofam", None)])
            .await;
        assert!(page.iter().all(|r| r.is_ok()), "bulk resend: {page:?}");
    }
    assert_eq!(read_version(&es, "p1").await.as_deref(), Some("2"));
    assert_eq!(found_by_family(&es, "Twofam").await, vec!["p1"]);

    // A resent delete: the second one finds nothing and is a success or a
    // plain NotFound, never another kind of failure.
    es.delete(&t, "Patient", "p1").await.expect("delete");
    let again = es.delete(&t, "Patient", "p1").await;
    assert!(
        again.is_ok()
            || matches!(
                again,
                Err(helios_persistence::error::StorageError::Resource(
                    helios_persistence::error::ResourceError::NotFound { .. }
                ))
            ),
        "a resent delete is idempotent, got {again:?}"
    );
    cleanup(&prefix).await;
}

/// Documents indexed before the guard existed carry an internal `_version`
/// that counts every PUT, retries and reindexes included, so it is usually
/// above the FHIR version. The first guarded write must still land.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_document_written_many_times_still_accepts_the_next_version() {
    let prefix = new_prefix();
    let es = backend(&prefix).await;
    let t = t();

    for _ in 0..8 {
        es.write_search_entries(&t, &patient("p1", 1, "Onefam", None))
            .await
            .expect("repeated v1");
    }
    es.write_search_entries(&t, &patient("p1", 2, "Twofam", None))
        .await
        .expect("v2 after many v1 writes");
    assert_eq!(read_version(&es, "p1").await.as_deref(), Some("2"));
    assert_eq!(found_by_family(&es, "Twofam").await, vec!["p1"]);
    cleanup(&prefix).await;
}

// ---------------------------------------------------------------------------
// Contained documents
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_writer_does_not_bring_back_dropped_contained_documents() {
    let prefix = new_prefix();
    let es = backend(&prefix).await;
    let t = t();

    es.write_search_entries(&t, &patient("p1", 1, "Onefam", Some("Acme")))
        .await
        .expect("v1 with contained");
    es.write_search_entries(&t, &patient("p1", 2, "Onefam", None))
        .await
        .expect("v2 drops contained");
    assert!(containers_of(&es, "Acme").await.is_empty());

    es.write_search_entries(&t, &patient("p1", 1, "Onefam", Some("Acme")))
        .await
        .expect("late v1 is a success");
    assert_eq!(read_version(&es, "p1").await.as_deref(), Some("2"));
    assert!(
        containers_of(&es, "Acme").await.is_empty(),
        "the contained document of the superseded version is not re-added"
    );
    assert_eq!(contained_doc_count(&prefix).await, 0);
    cleanup(&prefix).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_writer_does_not_sweep_the_newer_versions_contained_documents() {
    let prefix = new_prefix();
    let es = backend(&prefix).await;
    let t = t();

    es.write_search_entries(&t, &patient("p1", 3, "Onefam", Some("Globex")))
        .await
        .expect("v3 with contained");
    // A late v2 (which sweeps the container's contained documents first).
    es.write_search_entries(&t, &patient("p1", 2, "Onefam", Some("Acme")))
        .await
        .expect("late v2 is a success");

    assert_eq!(containers_of(&es, "Globex").await, vec!["Patient/p1"]);
    assert!(containers_of(&es, "Acme").await.is_empty());
    cleanup(&prefix).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_sync_does_not_leave_orphan_contained_documents_of_a_deleted_container() {
    let prefix = new_prefix();
    let es = backend(&prefix).await;
    let t = t();

    es.write_search_entries(&t, &patient("p1", 1, "Onefam", Some("Acme")))
        .await
        .expect("v1 with contained");
    // A versioned delete (v2 on the primary) leaves a tombstone; a bare
    // `delete` is a physical purge and orders nothing.
    es.tombstone_search_entries(&t, "Patient", "p1", "2")
        .await
        .expect("tombstone");
    assert_eq!(contained_doc_count(&prefix).await, 0);

    es.write_search_entries(&t, &patient("p1", 1, "Onefam", Some("Acme")))
        .await
        .expect("late v1 is a success");
    assert_eq!(read_version(&es, "p1").await, None);
    assert_eq!(
        contained_doc_count(&prefix).await,
        0,
        "no contained document under a deleted container"
    );
    assert!(containers_of(&es, "Acme").await.is_empty());
    cleanup(&prefix).await;
}

// ---------------------------------------------------------------------------
// Composite SQLite + Elasticsearch
// ---------------------------------------------------------------------------

fn composite_over(es: Arc<ElasticsearchBackend>) -> CompositeStorage {
    let sqlite = Arc::new(SqliteBackend::in_memory().expect("sqlite"));
    sqlite.init_schema().expect("schema");
    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("es", BackendKind::Elasticsearch)
        .sync_mode(SyncMode::Synchronous)
        .build()
        .expect("composite config");
    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), sqlite.clone() as DynStorage);
    backends.insert("es".to_string(), es.clone() as DynStorage);
    let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
    providers.insert("sqlite".to_string(), sqlite.clone() as DynSearchProvider);
    providers.insert("es".to_string(), es as DynSearchProvider);
    CompositeStorage::new(config, backends)
        .expect("composite")
        .with_search_providers(providers)
        .with_full_primary(sqlite)
        .start_sync_workers()
}

fn body(id: &str, family: &str) -> Value {
    json!({ "resourceType": "Patient", "id": id, "name": [{ "family": family }] })
}

/// A sync of an old version that was queued before a newer write (a retry, a
/// repair, a slow worker) lands after the composite already indexed v2.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn composite_update_is_not_overwritten_by_a_late_sync_of_the_old_version() {
    let prefix = new_prefix();
    let es = Arc::new(backend(&prefix).await);
    let composite = composite_over(es.clone());
    let t = t();

    let v1 = composite
        .create(&t, "Patient", body("c1", "Onefam"), FhirVersion::default())
        .await
        .expect("create");
    let v2 = composite
        .update(&t, &v1, body("c1", "Twofam"))
        .await
        .expect("update");
    assert_ne!(v1.version_id(), v2.version_id());

    es.write_search_entries(&t, &v1)
        .await
        .expect("late sync of v1");

    assert_eq!(
        read_version(&es, "c1").await.as_deref(),
        Some(v2.version_id())
    );
    assert_eq!(found_by_family(&es, "Twofam").await, vec!["c1"]);
    assert!(found_by_family(&es, "Onefam").await.is_empty());
    cleanup(&prefix).await;
}

/// A delete through the composite, then a late sync (the async worker's
/// retry, or a repair that read the primary before the delete) of the last
/// live version.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn composite_delete_is_not_undone_by_a_late_sync() {
    let prefix = new_prefix();
    let es = Arc::new(backend(&prefix).await);
    let composite = composite_over(es.clone());
    let t = t();

    let v1 = composite
        .create(&t, "Patient", body("c2", "Gonefam"), FhirVersion::default())
        .await
        .expect("create");
    composite.delete(&t, "Patient", "c2").await.expect("delete");
    assert_eq!(read_version(&es, "c2").await, None);

    es.write_search_entries(&t, &v1)
        .await
        .expect("late sync of v1");
    assert_eq!(read_version(&es, "c2").await, None, "still deleted");
    assert!(found_by_family(&es, "Gonefam").await.is_empty());

    let page = es.write_synced_entries_page(&t, &[v1]).await;
    assert!(page.iter().all(|r| r.is_ok()), "bulk late sync: {page:?}");
    assert_eq!(read_version(&es, "c2").await, None, "still deleted");
    cleanup(&prefix).await;
}
