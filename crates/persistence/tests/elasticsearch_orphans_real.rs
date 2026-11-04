//! The contained-orphan collector (`ElasticsearchBackend::collect_orphans`,
//! #2291) against a real Elasticsearch cluster.
//!
//! A contained document whose container is missing, or at another
//! `version_id`, is hidden by the read-side filter but stays on disk. These
//! tests prove, end to end, that the collector:
//!
//! 1. deletes such documents physically (counted with `_count` on
//!    `is_contained`), across several pages, and leaves current ones alone;
//! 2. is idempotent (a second run deletes nothing) and honours
//!    `container_type`;
//! 3. does not delete the children of a container that appears between its
//!    two reads (the container is written right before the real-time re-read);
//! 4. waits for the `contained_sweeps` gate (#2203): with one permit held by a
//!    sweep, none of its requests goes out;
//! 5. runs at the end of a `$reindex` without `clearExisting` through
//!    `ReindexTarget::collect_contained_orphans`.
//!
//! The backend talks to the cluster through a small TCP proxy that can hold
//! back `_delete_by_query` requests and run a step right before the first
//! real-time `_mget`, so the races are staged on the wire.
//!
//! Set `HFS_TEST_ES_URL` to run against a cluster that is already up;
//! otherwise a 7.17.29 container is started. `HFS_TEST_ES_INDEX_PREFIX`
//! (default `hfs`) names the index prefix; the tests delete what they create.
//!
//! Run with:
//! `cargo test -p helios-persistence --features R4,elasticsearch,sqlite --test elasticsearch_orphans_real`

#![cfg(all(feature = "elasticsearch", feature = "sqlite"))]

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use elasticsearch::Elasticsearch;
use elasticsearch::http::transport::Transport;
use helios_fhir::FhirVersion;
use helios_persistence::StoredResource;
use helios_persistence::backends::elasticsearch::{
    ElasticsearchBackend, ElasticsearchConfig, WriteRefreshPolicy,
};
use helios_persistence::core::{Backend, ResourceStorage};
use helios_persistence::error::StorageResult;
use helios_persistence::search::{
    ReindexOperation, ReindexRequest, ReindexSource, ReindexStatus, ReindexTarget,
    SearchParameterLoader, TenantSearchRegistries,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use serde_json::{Value, json};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::elastic_search::ElasticSearch;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
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
                let addr = url
                    .trim_start_matches("http://")
                    .trim_end_matches('/')
                    .to_string();
                return SharedEs {
                    addr,
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

fn raw_client(addr: &str) -> Elasticsearch {
    Elasticsearch::new(Transport::single_node(&format!("http://{addr}")).unwrap())
}

// ---------------------------------------------------------------------------
// The staging proxy
// ---------------------------------------------------------------------------

type Step = Pin<Box<dyn Future<Output = ()> + Send>>;

#[derive(Default)]
struct Wire {
    /// `_delete_by_query` requests sent and not yet answered.
    sweeps_in_flight: AtomicUsize,
    /// `_delete_by_query` requests seen.
    sweeps: AtomicUsize,
    /// Other requests that started while a `_delete_by_query` was in flight.
    others_during_sweep: AtomicUsize,
    /// Real-time `_mget` requests seen.
    live_mgets: AtomicUsize,
    /// Held back before a `_delete_by_query` is forwarded, in ms.
    sweep_delay_ms: AtomicU64,
    /// Run (once) right before the first real-time `_mget` is forwarded.
    before_live_mget: tokio::sync::Mutex<Option<Step>>,
    stop: AtomicBool,
}

/// Forwards connections to `upstream`. The ES client does not pipeline, so on
/// a connection a chunk arriving while no request is outstanding starts a
/// request (its first line names the endpoint) and the next chunk from the
/// cluster starts its answer.
async fn start_proxy(upstream: String) -> (String, Arc<Wire>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind proxy");
    let addr = listener.local_addr().expect("proxy addr").to_string();
    let wire = Arc::new(Wire::default());
    let accept_wire = wire.clone();
    tokio::spawn(async move {
        while !accept_wire.stop.load(Ordering::SeqCst) {
            let Ok((client, _)) = listener.accept().await else {
                break;
            };
            let upstream = upstream.clone();
            let wire = accept_wire.clone();
            tokio::spawn(async move {
                let Ok(server) = TcpStream::connect(&upstream).await else {
                    return;
                };
                let (mut cr, mut cw) = client.into_split();
                let (mut sr, mut sw) = server.into_split();
                // Whether the request outstanding on this connection is a sweep.
                let outstanding: Arc<std::sync::Mutex<Option<bool>>> = Default::default();
                let up_state = outstanding.clone();
                let up_wire = wire.clone();
                let up = tokio::spawn(async move {
                    let mut buf = vec![0u8; 64 * 1024];
                    loop {
                        let n = match cr.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        let mut hold_ms = 0;
                        let mut live_mget = false;
                        {
                            let mut state = up_state.lock().unwrap();
                            if state.is_none() {
                                let head = &buf[..n.min(512)];
                                let first = head.split(|b| *b == b'\n').next().unwrap_or(head);
                                let has = |needle: &[u8]| {
                                    first.windows(needle.len()).any(|w| w == needle)
                                };
                                let is_sweep = has(b"_delete_by_query");
                                if is_sweep {
                                    up_wire.sweeps.fetch_add(1, Ordering::SeqCst);
                                    up_wire.sweeps_in_flight.fetch_add(1, Ordering::SeqCst);
                                    hold_ms = up_wire.sweep_delay_ms.load(Ordering::SeqCst);
                                } else if up_wire.sweeps_in_flight.load(Ordering::SeqCst) > 0 {
                                    up_wire.others_during_sweep.fetch_add(1, Ordering::SeqCst);
                                }
                                if has(b"_mget") && has(b"realtime=true") {
                                    up_wire.live_mgets.fetch_add(1, Ordering::SeqCst);
                                    live_mget = true;
                                }
                                *state = Some(is_sweep);
                            }
                        }
                        if hold_ms > 0 {
                            tokio::time::sleep(Duration::from_millis(hold_ms)).await;
                        }
                        if live_mget {
                            let step = up_wire.before_live_mget.lock().await.take();
                            if let Some(step) = step {
                                step.await;
                            }
                        }
                        if sw.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                });
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    let n = match sr.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    let answered = outstanding.lock().unwrap().take();
                    if answered == Some(true) {
                        wire.sweeps_in_flight.fetch_sub(1, Ordering::SeqCst);
                    }
                    if cw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
                up.abort();
            });
        }
    });
    (addr, wire)
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

async fn backend_via(via: &str, prefix: &str, sweep_concurrency: usize) -> ElasticsearchBackend {
    let config = ElasticsearchConfig {
        nodes: vec![format!("http://{via}")],
        index_prefix: prefix.to_string(),
        number_of_replicas: 0,
        refresh_interval: "1ms".to_string(),
        write_refresh: WriteRefreshPolicy::WaitFor,
        contained_sweep_concurrency: sweep_concurrency,
        ..Default::default()
    };
    let backend = ElasticsearchBackend::with_shared_registry(config, search_registry())
        .expect("create ES backend");
    backend.initialize().await.expect("initialize ES backend");
    backend
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

async fn refresh(prefix: &str) {
    let pattern = format!("{prefix}_*");
    let _ = raw_client(es_addr().await)
        .indices()
        .refresh(elasticsearch::indices::IndicesRefreshParts::Index(&[
            &pattern,
        ]))
        .send()
        .await;
}

/// A patient at version `version` with one contained Organization per name.
fn patient(id: &str, version: &str, orgs: &[&str]) -> Value {
    let mut p = json!({
        "resourceType": "Patient", "id": id,
        "meta": { "versionId": version, "lastUpdated": "2026-01-01T00:00:00Z" },
        "name": [{ "family": id }],
    });
    if !orgs.is_empty() {
        p["contained"] = Value::Array(
            orgs.iter()
                .enumerate()
                .map(|(i, name)| {
                    json!({ "resourceType": "Organization", "id": format!("org{i}"), "name": name })
                })
                .collect(),
        );
        p["managingOrganization"] = json!({ "reference": "#org0" });
    }
    p
}

/// The `_id`s of the contained documents under `prefix`, sorted (refreshed).
async fn contained_ids(prefix: &str) -> Vec<String> {
    refresh(prefix).await;
    let pattern = format!("{prefix}_*");
    let body = raw_client(es_addr().await)
        .search(elasticsearch::SearchParts::Index(&[&pattern]))
        .body(json!({ "size": 1000, "query": { "term": { "is_contained": true } } }))
        .send()
        .await
        .expect("search")
        .json::<Value>()
        .await
        .expect("search body");
    let mut ids: Vec<String> = body["hits"]["hits"]
        .as_array()
        .expect("hits")
        .iter()
        .map(|h| h["_id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

/// How many contained documents the cluster holds under `prefix`.
async fn contained_count(prefix: &str) -> u64 {
    refresh(prefix).await;
    let pattern = format!("{prefix}_*");
    let body = raw_client(es_addr().await)
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

/// One document under `prefix` matching `query`: `(_index, _source)`.
async fn find_doc(prefix: &str, query: Value) -> (String, Value) {
    refresh(prefix).await;
    let pattern = format!("{prefix}_*");
    let body = raw_client(es_addr().await)
        .search(elasticsearch::SearchParts::Index(&[&pattern]))
        .body(json!({ "size": 1, "query": query }))
        .send()
        .await
        .expect("search")
        .json::<Value>()
        .await
        .expect("search body");
    let hit = &body["hits"]["hits"][0];
    (
        hit["_index"].as_str().expect("index").to_string(),
        hit["_source"].clone(),
    )
}

async fn raw_index(index: &str, doc_id: &str, source: Value) {
    raw_client(es_addr().await)
        .index(elasticsearch::IndexParts::IndexId(index, doc_id))
        .refresh(elasticsearch::params::Refresh::True)
        .body(source)
        .send()
        .await
        .expect("raw index")
        .error_for_status_code()
        .expect("raw index ok");
}

/// Writes, behind the backend's back, a contained Organization document of
/// `container_type/container_id` with local id `local` at `version`: the
/// shape a cut write or a stale rebuild leaves. Copies the document of a real
/// contained Organization so every field the backend writes is there.
async fn plant_child(
    prefix: &str,
    container_type: &str,
    container_id: &str,
    local: &str,
    version: &str,
) -> String {
    let (index, mut source) = find_doc(prefix, json!({ "term": { "is_contained": true } })).await;
    let synthetic = format!("{container_type}_{container_id}#{local}");
    let obj = source.as_object_mut().unwrap();
    obj.insert("container_type".into(), json!(container_type));
    obj.insert("container_id".into(), json!(container_id));
    obj.insert("contained_local_id".into(), json!(local));
    obj.insert("resource_id".into(), json!(synthetic));
    obj.insert("version_id".into(), json!(version));
    let doc_id = format!("Organization_{synthetic}");
    raw_index(&index, &doc_id, source).await;
    doc_id
}

/// Writes, behind the backend's back, the container document of `Patient/id`
/// at `version` (copied from the container `Patient/template`).
async fn plant_patient(prefix: &str, template: &str, id: &str, version: &str) {
    let (index, mut source) = find_doc(
        prefix,
        json!({ "bool": {
            "filter": [
                { "term": { "resource_type": "Patient" } },
                { "term": { "resource_id": template } },
            ],
            "must_not": [ { "term": { "is_contained": true } } ],
        } }),
    )
    .await;
    let obj = source.as_object_mut().unwrap();
    obj.insert("resource_id".into(), json!(id));
    obj.insert("version_id".into(), json!(version));
    obj.insert("contained_doc_ids".into(), json!([]));
    raw_index(&index, &format!("Patient_{id}"), source).await;
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn orphans_are_deleted_across_pages_and_current_children_are_kept() {
    let prefix = new_prefix();
    let es = backend_via(es_addr().await, &prefix, 16).await;
    let t = tenant("orph-a");

    es.create(
        &t,
        "Patient",
        patient("live1", "1", &["A"]),
        FhirVersion::R4,
    )
    .await
    .expect("create live1");
    es.create(
        &t,
        "Patient",
        patient("live2", "2", &["B", "C"]),
        FhirVersion::R4,
    )
    .await
    .expect("create live2");
    let current = contained_ids(&prefix).await;
    assert_eq!(current.len(), 3, "precondition: three current children");

    // Container absent.
    plant_child(&prefix, "Patient", "gone", "a", "1").await;
    plant_child(&prefix, "Patient", "gone", "b", "1").await;
    plant_child(&prefix, "Patient", "gone2", "a", "1").await;
    // Container present at another version (older and newer).
    plant_child(&prefix, "Patient", "live1", "old", "0").await;
    plant_child(&prefix, "Patient", "live2", "new", "3").await;
    // Another container type: out of scope of a `Patient` run.
    let other = plant_child(&prefix, "Observation", "gone", "a", "1").await;
    assert_eq!(contained_count(&prefix).await, 9);

    let stats = es
        .collect_orphans_paged(&t, Some("Patient"), 2)
        .await
        .expect("collect");
    eprintln!("first Patient run: {stats:?}");
    assert_eq!(stats.examined, 8);
    assert_eq!(stats.orphans_deleted, 5);
    assert_eq!(stats.errors, 0);
    assert!(stats.pages >= 4, "pages of 2 over 8 documents: {stats:?}");
    let mut expected = current.clone();
    expected.push(other.clone());
    assert_eq!(contained_ids(&prefix).await, sorted(expected));

    // Idempotent.
    let again = es
        .collect_orphans_paged(&t, Some("Patient"), 2)
        .await
        .expect("collect again");
    assert_eq!(again.orphans_deleted, 0);
    assert_eq!(again.examined, 3);

    // Every container type.
    let all = es.collect_orphans(&t, None).await.expect("collect all");
    assert_eq!(all.orphans_deleted, 1);
    assert_eq!(contained_ids(&prefix).await, current);

    // Another tenant's run touches nothing of this one.
    let foreign = es
        .collect_orphans(&tenant("orph-a-other"), None)
        .await
        .expect("collect foreign");
    assert_eq!(foreign.examined, 0);
    assert_eq!(contained_count(&prefix).await, 3);

    cleanup(&prefix).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_container_written_between_the_two_reads_keeps_its_children() {
    let prefix = new_prefix();
    let (proxy, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = backend_via(&proxy, &prefix, 16).await;
    let t = tenant("orph-b");

    es.create(&t, "Patient", patient("live", "1", &["A"]), FhirVersion::R4)
        .await
        .expect("create live");
    // The children of a write in flight: its container is not there yet.
    let late = plant_child(&prefix, "Patient", "late", "a", "1").await;
    let gone = plant_child(&prefix, "Patient", "gone", "a", "1").await;

    // The container lands after the searchable lookup and before the
    // real-time re-read.
    let landed = Arc::new(AtomicBool::new(false));
    let flag = landed.clone();
    let prefix_for_step = prefix.clone();
    *wire.before_live_mget.lock().await = Some(Box::pin(async move {
        plant_patient(&prefix_for_step, "live", "late", "1").await;
        flag.store(true, Ordering::SeqCst);
    }));

    let stats = es
        .collect_orphans(&t, Some("Patient"))
        .await
        .expect("collect");
    assert!(landed.load(Ordering::SeqCst), "the step ran");
    assert_eq!(wire.live_mgets.load(Ordering::SeqCst), 1);
    assert_eq!(stats.orphans_deleted, 1, "{stats:?}");
    let left = contained_ids(&prefix).await;
    assert!(left.contains(&late), "the late container's child is kept");
    assert!(!left.contains(&gone), "the orphan in the same page goes");

    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_collector_waits_for_the_contained_sweep_gate() {
    let prefix = new_prefix();
    let (proxy, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = Arc::new(backend_via(&proxy, &prefix, 1).await);
    let t = tenant("orph-c");

    es.create(&t, "Patient", patient("live", "1", &["A"]), FhirVersion::R4)
        .await
        .expect("create live");
    let gone = plant_child(&prefix, "Patient", "gone", "a", "1").await;

    // A delete of an absent container sweeps; the proxy holds the sweep back,
    // so it keeps the only permit for that long.
    wire.sweep_delay_ms.store(1_500, Ordering::SeqCst);
    let sweeper = {
        let es = es.clone();
        let t = t.clone();
        tokio::spawn(async move { es.delete(&t, "Patient", "absent").await })
    };
    let waited = tokio::time::timeout(Duration::from_secs(10), async {
        while wire.sweeps_in_flight.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "the sweep went out");

    let collector = {
        let es = es.clone();
        let t = t.clone();
        tokio::spawn(async move { es.collect_orphans(&t, None).await })
    };
    let _ = sweeper.await.unwrap();
    let stats = collector.await.unwrap().expect("collect");
    wire.sweep_delay_ms.store(0, Ordering::SeqCst);

    assert!(wire.sweeps.load(Ordering::SeqCst) >= 1);
    assert_eq!(
        wire.others_during_sweep.load(Ordering::SeqCst),
        0,
        "no collector request while the sweep held the only permit"
    );
    assert_eq!(stats.orphans_deleted, 1);
    assert!(!contained_ids(&prefix).await.contains(&gone));

    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}

/// A writer that is the Elasticsearch backend, with the collector wired into
/// `collect_contained_orphans` the way the backend's own `ReindexTarget`
/// implementation is meant to.
struct CollectingTarget(Arc<ElasticsearchBackend>);

#[async_trait]
impl ReindexTarget for CollectingTarget {
    async fn delete_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<u64> {
        self.0
            .delete_search_entries(tenant, resource_type, resource_id)
            .await
    }

    async fn delete_search_entries_for_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_ids: &[String],
    ) -> StorageResult<u64> {
        self.0
            .delete_search_entries_for_resources(tenant, resource_type, resource_ids)
            .await
    }

    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource: &StoredResource,
    ) -> StorageResult<usize> {
        self.0.write_search_entries(tenant, resource).await
    }

    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
        self.0.clear_search_index(tenant).await
    }

    fn page_write_replaces_all_entries(&self) -> bool {
        self.0.page_write_replaces_all_entries()
    }

    async fn write_search_entries_page(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
    ) -> Vec<StorageResult<usize>> {
        self.0.write_search_entries_page(tenant, resources).await
    }

    async fn collect_contained_orphans(
        &self,
        tenant: &TenantContext,
        resource_types: &[String],
    ) -> StorageResult<u64> {
        let mut removed = 0;
        for resource_type in resource_types {
            removed += self
                .0
                .collect_orphans(tenant, Some(resource_type))
                .await?
                .orphans_deleted;
        }
        Ok(removed)
    }
}

async fn run_reindex(
    es: &Arc<ElasticsearchBackend>,
    t: &TenantContext,
    request: ReindexRequest,
) -> ReindexStatus {
    let op = ReindexOperation::with_parts(
        es.clone() as Arc<dyn ReindexSource>,
        vec![Arc::new(CollectingTarget(es.clone())) as Arc<dyn ReindexTarget>],
        search_registry(),
    );
    let job = op.start(t.clone(), request, None).await.expect("start");
    for _ in 0..600 {
        if let Some(progress) = op.get_progress(&job).await
            && progress.status.is_finished()
        {
            return progress.status;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("reindex did not finish");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reindex_without_clear_existing_collects_the_orphans_of_its_types() {
    let prefix = new_prefix();
    let es = Arc::new(backend_via(es_addr().await, &prefix, 16).await);
    let t = tenant("orph-d");

    es.create(&t, "Patient", patient("live", "1", &["A"]), FhirVersion::R4)
        .await
        .expect("create live");
    let current = contained_ids(&prefix).await;
    let gone = plant_child(&prefix, "Patient", "gone", "a", "1").await;
    let other = plant_child(&prefix, "Observation", "gone", "a", "1").await;

    // A run named by ids does not collect.
    let named = ReindexRequest::for_resources([helios_persistence::search::ResourceRef::new(
        "Patient", "live",
    )]);
    assert!(matches!(
        run_reindex(&es, &t, named).await,
        ReindexStatus::Completed
    ));
    assert!(contained_ids(&prefix).await.contains(&gone));

    // A whole-type run of `Patient` collects its orphans, not other types'.
    let status = run_reindex(&es, &t, ReindexRequest::for_types(["Patient"])).await;
    assert!(matches!(status, ReindexStatus::Completed), "{status:?}");
    let mut expected = current.clone();
    expected.push(other);
    assert_eq!(contained_ids(&prefix).await, sorted(expected));

    cleanup(&prefix).await;
}
