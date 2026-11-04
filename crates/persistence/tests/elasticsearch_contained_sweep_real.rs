//! The contained-document sweep against a real Elasticsearch cluster.
//!
//! A container document records the `_id`s of its contained documents
//! (`contained_doc_ids`, #2291), so an update or a delete removes them by id in
//! the same `_bulk` and runs no tenant-wide `_delete_by_query`. The sweep is
//! the repair path: it runs only for a container that is absent or that
//! predates the record. These tests prove, end to end:
//!
//! 1. an update or delete of a resource that records its contained documents
//!    (or none) runs no sweep, and no stale contained document survives, also
//!    after a lost sync, a lost sweep, or a Bundle-style page write;
//! 2. a burst of 100 concurrent sweeps (deletes of absent containers) keeps
//!    the sweeps in flight at or below `contained_sweep_concurrency` (default
//!    16) and gets no `429`;
//! 3. a sweep that stays unavailable fails the write loudly after its attempts
//!    and a repeat completes it.
//!
//! The backend talks to the cluster through a small recording TCP proxy that
//! counts `_delete_by_query` requests, the highest number in flight at once and
//! the `429` answers, so the claims are measured on the wire rather than
//! inferred.
//!
//! Set `HFS_TEST_ES_URL` (for example `http://127.0.0.1:9200`) to run against
//! a cluster that is already up; otherwise a 7.17.29 container is started.
//! `HFS_TEST_ES_INDEX_PREFIX` (default `hfs`) names the index prefix, so a
//! shared cluster can be kept tidy; the tests delete what they create.
//!
//! Run with:
//! `cargo test -p helios-persistence --features R4,elasticsearch,sqlite --test elasticsearch_contained_sweep_real`

#![cfg(all(feature = "elasticsearch", feature = "sqlite"))]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use elasticsearch::Elasticsearch;
use elasticsearch::http::transport::Transport;
use helios_fhir::FhirVersion;
use helios_persistence::StoredResource;
use helios_persistence::backends::elasticsearch::{
    ElasticsearchBackend, ElasticsearchConfig, WriteRefreshPolicy,
};
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
};
use helios_persistence::core::{
    Backend, BackendKind, ConditionalDeleteResult, ConditionalStorage, EntityTagPrecondition,
    PurgableStorage, ResourceStorage, SearchProvider,
};
use helios_persistence::search::{ReindexTarget, SearchParameterLoader, TenantSearchRegistries};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    ContainedMode, ContainedReturn, SearchParamType, SearchParameter, SearchQuery, SearchValue,
};
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
const ES_STARTUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

struct SharedEs {
    /// `host:port` of the cluster.
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

// ---------------------------------------------------------------------------
// The recording proxy
// ---------------------------------------------------------------------------

/// What crossed the proxy.
#[derive(Default)]
struct Wire {
    /// `_delete_by_query` requests seen.
    sweeps: AtomicUsize,
    /// `_delete_by_query` requests sent and not yet answered.
    in_flight: AtomicUsize,
    /// The highest `in_flight` reached.
    max_in_flight: AtomicUsize,
    /// Answers with status 429 (to any request).
    too_many: AtomicUsize,
    /// `_delete_by_query` answers that were not 2xx.
    sweep_failures: AtomicUsize,
    stop: AtomicBool,
    /// When set, a `_delete_by_query` is answered `503` by the proxy itself and
    /// never reaches the cluster.
    fail_sweeps: AtomicBool,
}

impl Wire {
    fn sweeps(&self) -> usize {
        self.sweeps.load(Ordering::SeqCst)
    }
    fn max_in_flight(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }
    fn too_many(&self) -> usize {
        self.too_many.load(Ordering::SeqCst)
    }
    fn sweep_failures(&self) -> usize {
        self.sweep_failures.load(Ordering::SeqCst)
    }
    /// Forgets the high-water mark and the 429 count (not the sweep total).
    fn reset_peaks(&self) {
        self.max_in_flight.store(0, Ordering::SeqCst);
        self.too_many.store(0, Ordering::SeqCst);
        self.sweep_failures.store(0, Ordering::SeqCst);
    }
}

/// Forwards connections to `upstream`, watching each one. The ES client does
/// not pipeline, so on a connection a chunk arriving while no request is
/// outstanding starts a request (its first line names the endpoint) and the
/// next chunk from the cluster starts its answer (its status line is `HTTP/1.1
/// NNN`).
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
                let (mut cr, cw) = client.into_split();
                let cw = Arc::new(tokio::sync::Mutex::new(cw));
                let refuse = cw.clone();
                let (mut sr, mut sw) = server.into_split();
                // Whether the request now outstanding on this connection is a sweep.
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
                        let mut refused = false;
                        {
                            let mut state = up_state.lock().unwrap();
                            if state.is_none() {
                                let head = &buf[..n.min(512)];
                                let first_line = head.split(|b| *b == b'\n').next().unwrap_or(head);
                                let is_sweep =
                                    first_line.windows(16).any(|w| w == b"_delete_by_query");
                                if is_sweep {
                                    up_wire.sweeps.fetch_add(1, Ordering::SeqCst);
                                    let now = up_wire.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                                    up_wire.max_in_flight.fetch_max(now, Ordering::SeqCst);
                                    if up_wire.fail_sweeps.load(Ordering::SeqCst) {
                                        up_wire.in_flight.fetch_sub(1, Ordering::SeqCst);
                                        up_wire.sweep_failures.fetch_add(1, Ordering::SeqCst);
                                        refused = true;
                                    }
                                }
                                *state = Some(is_sweep);
                            }
                        }
                        if refused {
                            // Refuse it here, then drop the connection so no tail
                            // of the request is mistaken for the next one.
                            let _ = refuse.lock().await.write_all(
                                b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                            ).await;
                            break;
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
                    if let Some(is_sweep) = answered {
                        let status_429 = buf[..n].starts_with(b"HTTP/1.1 429");
                        let ok = buf[..n].starts_with(b"HTTP/1.1 2");
                        if status_429 {
                            wire.too_many.fetch_add(1, Ordering::SeqCst);
                        }
                        if is_sweep {
                            wire.in_flight.fetch_sub(1, Ordering::SeqCst);
                            if !ok {
                                wire.sweep_failures.fetch_add(1, Ordering::SeqCst);
                            }
                        }
                    }
                    if cw.lock().await.write_all(&buf[..n]).await.is_err() {
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

/// An initialised backend on `prefix`, reaching the cluster through `via`.
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

/// Removes every index and template the test made under `prefix`.
async fn cleanup(prefix: &str) {
    let client =
        Elasticsearch::new(Transport::single_node(&format!("http://{}", es_addr().await)).unwrap());
    // A cluster refuses a wildcard delete by default
    // (`action.destructive_requires_name`), so list the names and delete those.
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

fn patient(id: &str, family: &str, contained_org: Option<&str>) -> Value {
    let mut p = json!({
        "resourceType": "Patient", "id": id,
        "name": [{ "family": family }],
    });
    if let Some(org) = contained_org {
        p["contained"] = json!([{ "resourceType": "Organization", "id": "org", "name": org }]);
        p["managingOrganization"] = json!({ "reference": "#org" });
    }
    p
}

/// A patient with one contained Organization per name in `orgs` (ids `org0`, ...).
fn patient_with_orgs(id: &str, family: &str, orgs: &[&str]) -> Value {
    let mut p = json!({
        "resourceType": "Patient", "id": id,
        "name": [{ "family": family }],
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

/// The containers (by `Patient/id`) of contained Organizations named `name`.
async fn containers_of(
    es: &ElasticsearchBackend,
    tenant: &TenantContext,
    name: &str,
) -> Vec<String> {
    es.refresh_index(tenant.tenant_id().as_str(), "Organization")
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
        .search(tenant, &q)
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

/// How many contained documents the cluster holds under `prefix`, whether or
/// not their container's own document is still there.
async fn contained_doc_count(prefix: &str) -> u64 {
    let client =
        Elasticsearch::new(Transport::single_node(&format!("http://{}", es_addr().await)).unwrap());
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

/// The container document of `rtype/id` under `prefix`: its index, `_id` and
/// `_source`, read straight from the cluster (not through the proxy).
async fn find_container(prefix: &str, rtype: &str, id: &str) -> (String, String, Value) {
    let client =
        Elasticsearch::new(Transport::single_node(&format!("http://{}", es_addr().await)).unwrap());
    let pattern = format!("{prefix}_*");
    let _ = client
        .indices()
        .refresh(elasticsearch::indices::IndicesRefreshParts::Index(&[
            &pattern,
        ]))
        .send()
        .await;
    let body = client
        .search(elasticsearch::SearchParts::Index(&[&pattern]))
        .body(json!({ "query": { "bool": {
            "filter": [
                { "term": { "resource_type": rtype } },
                { "term": { "resource_id": id } },
            ],
            "must_not": [ { "term": { "is_contained": true } } ],
        } } }))
        .send()
        .await
        .expect("find container")
        .json::<Value>()
        .await
        .expect("find container body");
    let hit = body["hits"]["hits"][0].clone();
    (
        hit["_index"].as_str().expect("container index").to_string(),
        hit["_id"].as_str().expect("container id").to_string(),
        hit["_source"].clone(),
    )
}

/// Rewrites the container of `rtype/id` without `contained_doc_ids`: what a
/// document indexed before the record existed looks like (`Prior::Legacy`).
async fn make_legacy(prefix: &str, rtype: &str, id: &str) {
    let (index, doc_id, mut source) = find_container(prefix, rtype, id).await;
    assert!(
        source.get("contained_doc_ids").is_some(),
        "precondition: the container records its contained documents"
    );
    source.as_object_mut().unwrap().remove("contained_doc_ids");
    let client =
        Elasticsearch::new(Transport::single_node(&format!("http://{}", es_addr().await)).unwrap());
    client
        .index(elasticsearch::IndexParts::IndexId(&index, &doc_id))
        .refresh(elasticsearch::params::Refresh::True)
        .body(source)
        .send()
        .await
        .expect("rewrite container")
        .error_for_status_code()
        .expect("rewrite container ok");
}

/// A composite with a SQLite primary and `es` as the search secondary, in
/// synchronous mode: a write returns once Elasticsearch has it.
fn composite_over(es: Arc<ElasticsearchBackend>) -> CompositeStorage {
    composite_over_with_primary(es).0
}

/// [`composite_over`], also handing back the SQLite primary so a test can write
/// to it behind the composite's back (a write whose secondary sync never ran).
fn composite_over_with_primary(
    es: Arc<ElasticsearchBackend>,
) -> (CompositeStorage, Arc<SqliteBackend>) {
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
    let composite = CompositeStorage::new(config, backends)
        .expect("composite")
        .with_search_providers(providers)
        .with_full_primary(sqlite.clone())
        .start_sync_workers();
    (composite, sqlite)
}

// ---------------------------------------------------------------------------
// 1: no sweep for containers that record their documents; nothing stale
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn updates_and_deletes_run_no_sweep_and_leave_no_stale_contained_documents() {
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = Arc::new(backend_via(&via, &prefix, 0).await);
    let composite = composite_over(es.clone());
    let t = tenant("sweep-real");

    // --- a resource without contained[]: no sweep ---
    let plain = composite
        .create(
            &t,
            "Patient",
            patient("plain", "Plainfam", None),
            FhirVersion::default(),
        )
        .await
        .expect("create plain");
    let updated = composite
        .update(&t, &plain, patient("plain", "Plainfam2", None))
        .await
        .expect("update plain");
    assert_eq!(
        wire.sweeps(),
        0,
        "an update of a resource without contained[] runs no sweep"
    );
    match composite
        .conditional_delete(
            &t,
            "Patient",
            "family=Plainfam2",
            &EntityTagPrecondition::Absent,
        )
        .await
        .expect("conditional delete")
    {
        ConditionalDeleteResult::Deleted(d) => assert_eq!(d.id(), updated.id()),
        other => panic!("expected Deleted, got {other:?}"),
    }
    assert_eq!(wire.sweeps(), 0, "a conditional delete runs no sweep");

    // --- a resource with contained[] ---
    let with = composite
        .create(
            &t,
            "Patient",
            patient("with", "Withfam", Some("Acme")),
            FhirVersion::default(),
        )
        .await
        .expect("create with contained");
    assert_eq!(containers_of(&es, &t, "Acme").await, vec!["Patient/with"]);

    let replaced = composite
        .update(&t, &with, patient("with", "Withfam", Some("Globex")))
        .await
        .expect("update with contained");
    assert!(
        containers_of(&es, &t, "Acme").await.is_empty(),
        "the stale contained document was deleted by id"
    );
    assert_eq!(containers_of(&es, &t, "Globex").await, vec!["Patient/with"]);

    // The array disappears.
    composite
        .update(&t, &replaced, patient("with", "Withfam", None))
        .await
        .expect("update dropping contained");
    assert!(
        containers_of(&es, &t, "Globex").await.is_empty(),
        "contained documents of a version that no longer has them are gone"
    );

    // A delete removes the contained documents of the container too.
    let withc = composite
        .create(
            &t,
            "Patient",
            patient("gone", "Gonefam", Some("Initech")),
            FhirVersion::default(),
        )
        .await
        .expect("create gone");
    assert_eq!(
        containers_of(&es, &t, "Initech").await,
        vec!["Patient/gone"]
    );
    composite
        .delete(&t, "Patient", withc.id())
        .await
        .expect("delete");
    assert!(containers_of(&es, &t, "Initech").await.is_empty());
    assert_eq!(contained_doc_count(&prefix).await, 0);
    assert_eq!(
        wire.sweeps(),
        0,
        "no tenant-wide _delete_by_query in the whole scenario"
    );
    assert_eq!(wire.too_many(), 0, "no 429 in the whole scenario");

    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}

// ---------------------------------------------------------------------------
// 3: 100 concurrent deletes stay under the sweep limiter and get no 429
// ---------------------------------------------------------------------------

/// Deletes `n` patients at once and reports what the wire saw during the
/// deletes: (sweeps, peak in flight, 429s, failed sweeps). With `absent` the
/// patients were never created, so each delete finds no container, cannot know
/// its contained documents, and sweeps; otherwise they exist and record none,
/// and no delete sweeps.
async fn burst_of_deletes(
    n: usize,
    sweep_concurrency: usize,
    absent: bool,
) -> (usize, usize, usize, usize) {
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = Arc::new(backend_via(&via, &prefix, sweep_concurrency).await);
    let t = tenant("sweep-burst");

    let created = if absent { 1 } else { n };
    let creates: Vec<_> = (0..created)
        .map(|i| {
            let es = es.clone();
            let t = t.clone();
            tokio::spawn(async move {
                es.create(
                    &t,
                    "Patient",
                    patient(
                        &format!("{}{i}", if absent { "seed" } else { "p" }),
                        "Burst",
                        None,
                    ),
                    FhirVersion::default(),
                )
                .await
            })
        })
        .collect();
    for c in creates {
        c.await.unwrap().expect("create");
    }
    wire.reset_peaks();
    let before = wire.sweeps();

    let deletes: Vec<_> = (0..n)
        .map(|i| {
            let es = es.clone();
            let t = t.clone();
            tokio::spawn(async move { es.delete(&t, "Patient", &format!("p{i}")).await })
        })
        .collect();
    for d in deletes {
        let outcome = d.await.unwrap();
        if absent {
            assert!(
                matches!(
                    outcome,
                    Err(helios_persistence::error::StorageError::Resource(
                        helios_persistence::error::ResourceError::NotFound { .. }
                    ))
                ),
                "an absent container is NotFound after its sweep: {outcome:?}"
            );
        } else {
            outcome.expect("every concurrent delete succeeds");
        }
    }
    let seen = (
        wire.sweeps() - before,
        wire.max_in_flight(),
        wire.too_many(),
        wire.sweep_failures(),
    );
    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
    seen
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hundred_concurrent_deletes_of_recorded_containers_run_no_sweep() {
    let (sweeps, _peak, too_many, failed) = burst_of_deletes(100, 0, false).await;
    eprintln!("recorded containers: sweeps={sweeps} 429={too_many} failed={failed}");
    assert_eq!(sweeps, 0, "no delete sweeps");
    assert_eq!(too_many, 0, "no 429 too_many_scroll_contexts");
    assert_eq!(failed, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hundred_concurrent_sweeps_stay_under_the_default_sweep_limit_without_429() {
    // 0 is the default: 16 sweeps in flight.
    let (sweeps, peak, too_many, failed) = burst_of_deletes(100, 0, true).await;
    eprintln!(
        "default limiter: sweeps={sweeps} peak_in_flight={peak} 429={too_many} failed={failed}"
    );
    assert_eq!(
        sweeps, 100,
        "a delete of an absent container sweeps, once each"
    );
    assert!(peak <= 16, "at most 16 sweeps at once, saw {peak}");
    assert!(peak >= 2, "the burst really was concurrent, saw {peak}");
    assert_eq!(too_many, 0, "no 429 too_many_scroll_contexts");
    assert_eq!(failed, 0, "no sweep failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_configured_sweep_limit_is_the_one_enforced() {
    let (sweeps, peak, too_many, failed) = burst_of_deletes(100, 3, true).await;
    eprintln!("limit 3: sweeps={sweeps} peak_in_flight={peak} 429={too_many} failed={failed}");
    assert_eq!(sweeps, 100);
    assert!(peak <= 3, "at most 3 sweeps at once, saw {peak}");
    assert!(peak >= 2, "the limit was reached, saw {peak}");
    assert_eq!(too_many, 0);
    assert_eq!(failed, 0);
}

// ---------------------------------------------------------------------------
// 3: regression, the gap scenario: stale documents are removed by recorded id
// ---------------------------------------------------------------------------

/// v1 has `contained[]`; v2 drops it but its sync to Elasticsearch never
/// happens (v2 is written to the primary alone, which is what a permanently
/// failed secondary sync leaves behind); v3 is a normal update. Elasticsearch
/// still holds v1's contained documents, and its container records them, so v3
/// deletes them by id: the old ids come from Elasticsearch's own container, not
/// from the primary's previous version (which no longer has them).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_sync_of_the_version_that_dropped_contained_does_not_strand_its_documents() {
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = Arc::new(backend_via(&via, &prefix, 0).await);
    let (composite, primary) = composite_over_with_primary(es.clone());
    let t = tenant("sweep-gap");

    let v1 = composite
        .create(
            &t,
            "Patient",
            patient("gap", "Gapfam", Some("Stale")),
            FhirVersion::default(),
        )
        .await
        .expect("create v1");
    assert_eq!(containers_of(&es, &t, "Stale").await, vec!["Patient/gap"]);

    let v2 = primary
        .update(&t, &v1, patient("gap", "Gapfam2", None))
        .await
        .expect("v2 on the primary only");
    assert_eq!(
        containers_of(&es, &t, "Stale").await,
        vec!["Patient/gap"],
        "precondition: Elasticsearch never saw v2"
    );

    composite
        .update(&t, &v2, patient("gap", "Gapfam3", None))
        .await
        .expect("v3");
    assert_eq!(wire.sweeps(), 0, "v3 deletes by id, no sweep");
    assert!(
        containers_of(&es, &t, "Stale").await.is_empty(),
        "v1's contained documents are gone after v3"
    );

    // The same for a delete after a lost sync.
    let v1 = composite
        .create(
            &t,
            "Patient",
            patient("del", "Delfam", Some("Ghost")),
            FhirVersion::default(),
        )
        .await
        .expect("create del v1");
    primary
        .update(&t, &v1, patient("del", "Delfam2", None))
        .await
        .expect("del v2 on the primary only");
    assert_eq!(containers_of(&es, &t, "Ghost").await, vec!["Patient/del"]);
    composite
        .delete(&t, "Patient", "del")
        .await
        .expect("delete");
    assert!(containers_of(&es, &t, "Ghost").await.is_empty());
    assert_eq!(contained_doc_count(&prefix).await, 0);
    assert_eq!(wire.sweeps(), 0, "the delete removed them by id too");

    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}

// ---------------------------------------------------------------------------
// 4: a sweep that stays unavailable fails the write, loudly, after 4 attempts
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sweep_that_exhausts_its_attempts_fails_the_write_and_a_repeat_completes_it() {
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = Arc::new(backend_via(&via, &prefix, 0).await);
    let t = tenant("sweep-exhaust");

    let stored = es
        .create(
            &t,
            "Patient",
            patient("ex", "Exfam", Some("Doomed")),
            FhirVersion::default(),
        )
        .await
        .expect("create");
    assert_eq!(containers_of(&es, &t, "Doomed").await, vec!["Patient/ex"]);
    // The sweep only runs for a container that does not record its contained
    // documents: make this one look like it predates the record.
    make_legacy(&prefix, "Patient", "ex").await;

    wire.fail_sweeps.store(true, Ordering::SeqCst);
    let before = wire.sweeps();
    es.write_search_entries(&t, &stored)
        .await
        .expect_err("a failing sweep fails the update's sync");
    // Each of the SWEEP_ATTEMPTS (4) rounds is itself sent up to 3 times by the
    // transport-level retry, so 12 requests reach the wire; at least 4 is the
    // contract, and the exact number is not asserted.
    let sent = wire.sweeps() - before;
    assert!(sent >= 4, "at least SWEEP_ATTEMPTS (4) sweeps, saw {sent}");

    let before = wire.sweeps();
    let err = es
        .delete(&t, "Patient", "ex")
        .await
        .expect_err("a sweep that never succeeds must fail the delete, not drop it silently");
    assert!(wire.sweeps() - before >= 4);
    let message = err.to_string();
    assert!(
        message.contains("contained"),
        "the error names the contained sweep: {message}"
    );
    assert_eq!(
        contained_doc_count(&prefix).await,
        1,
        "the stale contained document is still there, and the caller was told"
    );

    // Once the cluster answers, repeating the delete finishes the job even
    // though the container's own document is already gone (absent: sweep).
    wire.fail_sweeps.store(false, Ordering::SeqCst);
    let _ = es.delete(&t, "Patient", "ex").await;
    assert_eq!(
        contained_doc_count(&prefix).await,
        0,
        "the repeat swept the stale document"
    );

    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}

// ---------------------------------------------------------------------------
// 5: a lost sweep does not strand contained documents
// ---------------------------------------------------------------------------

/// The container predates the record, so v2 (dropping contained[]) must sweep,
/// and its sweep fails; the sweep works again for v3. The failed v2 left the
/// container as it was (still unrecorded), so v3 sweeps again. Nothing stale
/// may remain.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lost_sweep_does_not_strand_contained_documents() {
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = Arc::new(backend_via(&via, &prefix, 0).await);
    let composite = composite_over(es.clone());
    let t = tenant("sweep-lost");

    let v1 = composite
        .create(
            &t,
            "Patient",
            patient("lost", "Lostfam", Some("Ghost")),
            FhirVersion::default(),
        )
        .await
        .expect("create v1");
    assert_eq!(containers_of(&es, &t, "Ghost").await, vec!["Patient/lost"]);

    make_legacy(&prefix, "Patient", "lost").await;
    wire.fail_sweeps.store(true, Ordering::SeqCst);
    let _ = composite
        .update(&t, &v1, patient("lost", "Lostfam2", None))
        .await;
    assert!(wire.sweeps() >= 1, "v2 tried to sweep");
    wire.fail_sweeps.store(false, Ordering::SeqCst);

    let v2 = composite
        .read(&t, "Patient", "lost")
        .await
        .expect("read")
        .expect("v2 is in the primary");
    composite
        .update(&t, &v2, patient("lost", "Lostfam3", None))
        .await
        .expect("v3");
    assert_eq!(
        contained_doc_count(&prefix).await,
        0,
        "no stale contained document after v3"
    );

    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}

// ---------------------------------------------------------------------------
// 6: the sweep is the repair path: a legacy container sweeps once, then never
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_container_without_the_record_sweeps_once_and_is_recorded_afterwards() {
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = Arc::new(backend_via(&via, &prefix, 0).await);
    let t = tenant("sweep-legacy");

    let v1 = es
        .create(
            &t,
            "Patient",
            patient("old", "Oldfam", Some("Acme")),
            FhirVersion::default(),
        )
        .await
        .expect("create");
    make_legacy(&prefix, "Patient", "old").await;

    let v2 = es
        .update(&t, &v1, patient("old", "Oldfam2", Some("Globex")))
        .await
        .expect("update of a legacy container");
    assert_eq!(wire.sweeps(), 1, "the legacy container sweeps once");
    assert!(containers_of(&es, &t, "Acme").await.is_empty());
    assert_eq!(containers_of(&es, &t, "Globex").await, vec!["Patient/old"]);

    let v3 = es
        .update(&t, &v2, patient("old", "Oldfam3", Some("Initech")))
        .await
        .expect("second update");
    assert_eq!(wire.sweeps(), 1, "the container records its documents now");
    assert!(containers_of(&es, &t, "Globex").await.is_empty());
    es.delete(&t, "Patient", v3.id()).await.expect("delete");
    assert_eq!(wire.sweeps(), 1);
    assert_eq!(contained_doc_count(&prefix).await, 0);

    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}

// ---------------------------------------------------------------------------
// 7: Bundle sync (the page path): no stale contained document after an update
// ---------------------------------------------------------------------------

/// A transaction Bundle that updates a resource reaches Elasticsearch as a
/// page of "created" entries (`write_synced_entries_page`). The updated
/// resource drops one contained entry; the document of the dropped entry must
/// not stay behind, and nothing is swept.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bundle_update_that_drops_a_contained_entry_leaves_no_stale_document() {
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = Arc::new(backend_via(&via, &prefix, 0).await);
    let (composite, primary) = composite_over_with_primary(es.clone());
    let t = tenant("sweep-bundle");

    let v1 = composite
        .create(
            &t,
            "Patient",
            patient_with_orgs("b", "Bfam", &["Acme", "Globex"]),
            FhirVersion::default(),
        )
        .await
        .expect("create");
    assert_eq!(containers_of(&es, &t, "Acme").await, vec!["Patient/b"]);
    assert_eq!(containers_of(&es, &t, "Globex").await, vec!["Patient/b"]);

    // The Bundle's PUT lands on the primary, then syncs to the secondary as a page.
    let v2 = primary
        .update(&t, &v1, patient_with_orgs("b", "Bfam2", &["Acme"]))
        .await
        .expect("v2 on the primary");
    let results = es
        .write_synced_entries_page(&t, std::slice::from_ref(&v2))
        .await;
    results.into_iter().for_each(|r| {
        r.expect("page entry written");
    });
    assert!(
        containers_of(&es, &t, "Globex").await.is_empty(),
        "the dropped contained entry has no stale document"
    );
    assert_eq!(containers_of(&es, &t, "Acme").await, vec!["Patient/b"]);

    // A later Bundle drops the last one.
    let v3 = primary
        .update(&t, &v2, patient_with_orgs("b", "Bfam3", &[]))
        .await
        .expect("v3 on the primary");
    es.write_synced_entries_page(&t, std::slice::from_ref(&v3))
        .await
        .into_iter()
        .for_each(|r| {
            r.expect("page entry written");
        });
    assert_eq!(contained_doc_count(&prefix).await, 0);
    assert_eq!(wire.sweeps(), 0, "the page path deletes by id, no sweep");
    assert_eq!(wire.too_many(), 0);

    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}

// ---------------------------------------------------------------------------
// 8: B-prime: the read guarantee, the container type in the `_id`, races
// ---------------------------------------------------------------------------

/// The raw contained Organization documents named `name` under `prefix`:
/// `(index, _id, _source)`, read straight from the cluster.
async fn raw_contained_orgs(prefix: &str, name: &str) -> Vec<(String, String, Value)> {
    let client =
        Elasticsearch::new(Transport::single_node(&format!("http://{}", es_addr().await)).unwrap());
    let pattern = format!("{prefix}_*");
    let _ = client
        .indices()
        .refresh(elasticsearch::indices::IndicesRefreshParts::Index(&[
            &pattern,
        ]))
        .send()
        .await;
    let body = client
        .search(elasticsearch::SearchParts::Index(&[&pattern]))
        .size(100)
        .body(json!({ "query": { "bool": { "filter": [
            { "term": { "is_contained": true } },
            { "term": { "resource_type": "Organization" } },
        ] } } }))
        .send()
        .await
        .expect("raw contained search")
        .json::<Value>()
        .await
        .expect("raw contained body");
    body["hits"]["hits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|h| {
            h["_source"]["content"]["name"].as_str() == Some(name)
                || h["_source"]["content"].as_str().is_some_and(|s| {
                    serde_json::from_str::<Value>(s)
                        .ok()
                        .is_some_and(|c| c["name"].as_str() == Some(name))
                })
        })
        .map(|h| {
            (
                h["_index"].as_str().unwrap().to_string(),
                h["_id"].as_str().unwrap().to_string(),
                h["_source"].clone(),
            )
        })
        .collect()
}

/// Indexes `source` under `index`/`id` directly, behind the backend's back.
async fn index_raw(index: &str, id: &str, source: Value) {
    let client =
        Elasticsearch::new(Transport::single_node(&format!("http://{}", es_addr().await)).unwrap());
    client
        .index(elasticsearch::IndexParts::IndexId(index, id))
        .refresh(elasticsearch::params::Refresh::True)
        .body(source)
        .send()
        .await
        .expect("index raw")
        .error_for_status_code()
        .expect("index raw ok");
}

/// A `_contained=true` search for Organizations named `name`, returning what
/// `ret` asks for: the items as `(resource type, id)`, sorted.
async fn contained_hits(
    es: &ElasticsearchBackend,
    tenant: &TenantContext,
    name: &str,
    ret: ContainedReturn,
) -> Vec<(String, String)> {
    es.refresh_index(tenant.tenant_id().as_str(), "Organization")
        .await
        .ok();
    let mut q = SearchQuery::new("Organization");
    q.contained = ContainedMode::On;
    q.contained_return = ret;
    q.parameters.push(SearchParameter {
        name: "name".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq(name)],
        chain: vec![],
        components: vec![],
    });
    let mut items: Vec<(String, String)> = es
        .search(tenant, &q)
        .await
        .expect("contained search")
        .resources
        .items
        .iter()
        .map(|r| (r.resource_type().to_string(), r.id().to_string()))
        .collect();
    items.sort();
    items
}

/// Risk: a contained document the container's id list never recorded (an
/// old version, or a container that is gone) must not be returned, and a stale
/// hit must not hide the current one under the same key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_orphan_contained_document_is_hidden_and_does_not_hide_the_current_one() {
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = Arc::new(backend_via(&via, &prefix, 0).await);
    let t = tenant("sweep-orphan");

    let v1 = es
        .create(
            &t,
            "Patient",
            patient("orph", "Orphfam", Some("Acme")),
            FhirVersion::default(),
        )
        .await
        .expect("create");
    let v2 = es
        .update(&t, &v1, patient("orph", "Orphfam2", Some("Acme")))
        .await
        .expect("update");
    assert_ne!(v1.version_id(), v2.version_id());

    let live = raw_contained_orgs(&prefix, "Acme").await;
    assert_eq!(live.len(), 1, "precondition: one live contained document");
    let (index, live_id, live_source) = live[0].clone();
    assert_eq!(live_source["version_id"], json!(v2.version_id()));
    assert_eq!(
        live_id, "Organization_Patient_orph#org",
        "the contained `_id` carries the container's type"
    );
    assert_eq!(
        contained_hits(&es, &t, "Acme", ContainedReturn::Contained).await,
        vec![("Organization".to_string(), "org".to_string())]
    );

    // (a) an old version of a live container, under a different local id.
    let mut old_version = live_source.clone();
    old_version["version_id"] = json!(v1.version_id());
    old_version["contained_local_id"] = json!("old");
    old_version["resource_id"] = json!("Patient_orph#old");
    index_raw(&index, "Organization_Patient_orph#old", old_version).await;

    // (b) a container that no longer exists.
    let mut ghost = live_source.clone();
    ghost["container_id"] = json!("ghost");
    ghost["contained_local_id"] = json!("org");
    ghost["resource_id"] = json!("Patient_ghost#org");
    index_raw(&index, "Organization_Patient_ghost#org", ghost).await;

    // (c) stale hits under the live key (same container and local id), one
    // sorted before the live document and one after, with other content.
    for (id, stamp) in [
        ("Organization_a_stale#org", "2000-01-01T00:00:00+00:00"),
        ("Organization_z_stale#org", "2100-01-01T00:00:00+00:00"),
    ] {
        let mut stale = live_source.clone();
        stale["version_id"] = json!(v1.version_id());
        stale["last_updated"] = json!(stamp);
        if stale["content"].is_object() {
            stale["content"]["name"] = json!("StaleContent");
        }
        index_raw(&index, id, stale).await;
    }

    assert_eq!(
        contained_hits(&es, &t, "Acme", ContainedReturn::Contained).await,
        vec![("Organization".to_string(), "org".to_string())],
        "only the current document of the live container is returned"
    );
    assert_eq!(
        contained_hits(&es, &t, "Acme", ContainedReturn::Container).await,
        vec![("Patient".to_string(), "orph".to_string())],
        "container mode: no ghost, no duplicate from stale versions"
    );
    // The returned content is the current document's, not a stale hit's.
    let mut q = SearchQuery::new("Organization");
    q.contained = ContainedMode::On;
    q.contained_return = ContainedReturn::Contained;
    q.parameters.push(SearchParameter {
        name: "name".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("Acme")],
        chain: vec![],
        components: vec![],
    });
    let found = es.search(&t, &q).await.expect("search").resources.items;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].content()["name"], json!("Acme"));
    assert_eq!(found[0].version_id(), v2.version_id());

    // Physically they are all still there: the guarantee is on the read.
    assert_eq!(contained_doc_count(&prefix).await, 5);

    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}

/// A resource of `rtype` with one contained Organization `org` named `name`.
fn holder(rtype: &str, id: &str, name: Option<&str>) -> Value {
    let mut r = json!({ "resourceType": rtype, "id": id });
    if let Some(name) = name {
        r["contained"] = json!([{ "resourceType": "Organization", "id": "org", "name": name }]);
    }
    r
}

/// Risk: two containers of different types with the same id and the same local
/// id must not share one contained document, so that one dropping its contained
/// resource does not delete (or overwrite) the other's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn containers_of_different_types_with_the_same_id_and_local_id_do_not_collide() {
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = Arc::new(backend_via(&via, &prefix, 0).await);
    let t = tenant("sweep-collide");

    let p1 = es
        .create(
            &t,
            "Patient",
            holder("Patient", "1", Some("PatientOrg")),
            FhirVersion::default(),
        )
        .await
        .expect("create patient");
    es.create(
        &t,
        "Practitioner",
        holder("Practitioner", "1", Some("PractitionerOrg")),
        FhirVersion::default(),
    )
    .await
    .expect("create practitioner");
    assert_eq!(contained_doc_count(&prefix).await, 2, "one document each");
    assert_eq!(
        containers_of(&es, &t, "PatientOrg").await,
        vec!["Patient/1"]
    );
    assert_eq!(
        containers_of(&es, &t, "PractitionerOrg").await,
        vec!["Practitioner/1"]
    );

    // Patient/1 drops its contained resource: Practitioner/1 keeps its own.
    let p2 = es
        .update(&t, &p1, holder("Patient", "1", None))
        .await
        .expect("update patient");
    assert!(containers_of(&es, &t, "PatientOrg").await.is_empty());
    assert_eq!(
        containers_of(&es, &t, "PractitionerOrg").await,
        vec!["Practitioner/1"],
        "the other container's document survives the update"
    );

    // Same for a delete of Patient/1 after it had a contained resource again.
    let p3 = es
        .update(&t, &p2, holder("Patient", "1", Some("PatientOrg2")))
        .await
        .expect("update patient again");
    es.delete(&t, "Patient", p3.id()).await.expect("delete");
    assert!(containers_of(&es, &t, "PatientOrg2").await.is_empty());
    assert_eq!(
        containers_of(&es, &t, "PractitionerOrg").await,
        vec!["Practitioner/1"],
        "the other container's document survives the delete"
    );
    assert_eq!(contained_doc_count(&prefix).await, 1);
    assert_eq!(wire.sweeps(), 0, "all by id, no sweep");

    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}

/// Risk: concurrent writes of one resource, some with and some without
/// `contained`, leave contained documents that disagree with the container
/// version that won; the search must return exactly the winner's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writes_of_one_resource_leave_the_contained_of_the_winning_version() {
    const N: usize = 16;
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = Arc::new(backend_via(&via, &prefix, 0).await);
    let t = tenant("sweep-race");

    let base = es
        .create(
            &t,
            "Patient",
            patient("race", "Racefam", None),
            FhirVersion::default(),
        )
        .await
        .expect("create");

    // Version k+2 carries org "Race{k}" when k is even, none when k is odd.
    let name_of = |k: usize| k.is_multiple_of(2).then(|| format!("Race{k}"));
    let writes: Vec<_> = (0..N)
        .map(|k| {
            let es = es.clone();
            let t = t.clone();
            let name = name_of(k);
            let content = patient_with_orgs(
                "race",
                &format!("Racefam{k}"),
                &name.iter().map(String::as_str).collect::<Vec<_>>(),
            );
            let stored = StoredResource::from_storage(
                "Patient".to_string(),
                "race".to_string(),
                (k + 2).to_string(),
                t.tenant_id().clone(),
                content,
                base.created_at(),
                base.last_modified(),
                None,
                base.fhir_version(),
            );
            tokio::spawn(async move { es.write_search_entries(&t, &stored).await })
        })
        .collect();
    for w in writes {
        // A losing write may be refused; none may panic.
        let _ = w.await.expect("write task");
    }

    let (_, _, source) = find_container(&prefix, "Patient", "race").await;
    let version: usize = source["version_id"]
        .as_str()
        .expect("version")
        .parse()
        .expect("numeric version");
    assert!(version >= 2, "one of the concurrent versions won");
    let winner = name_of(version - 2);
    for k in 0..N {
        let name = format!("Race{k}");
        let expected: Vec<String> = if winner.as_deref() == Some(name.as_str()) {
            vec!["Patient/race".to_string()]
        } else {
            vec![]
        };
        assert_eq!(
            containers_of(&es, &t, &name).await,
            expected,
            "version {version} won (contained: {winner:?}); search for {name}"
        );
        let docs: Vec<(String, String)> =
            contained_hits(&es, &t, &name, ContainedReturn::Contained).await;
        assert_eq!(docs.len(), expected.len(), "contained mode for {name}");
    }

    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}

/// Risk: a rebuild page (which never sweeps) over a legacy container (no
/// `contained_doc_ids`) leaves the obsolete contained document of the previous
/// version in the index; the search must still not return it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rebuild_page_over_a_legacy_container_does_not_surface_its_obsolete_contained() {
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = Arc::new(backend_via(&via, &prefix, 0).await);
    let (composite, primary) = composite_over_with_primary(es.clone());
    let t = tenant("sweep-rebuild");

    let v1 = composite
        .create(
            &t,
            "Patient",
            patient_with_orgs("rb", "Rbfam", &["Acme", "Globex"]),
            FhirVersion::default(),
        )
        .await
        .expect("create");
    make_legacy(&prefix, "Patient", "rb").await;
    let v2 = primary
        .update(&t, &v1, patient_with_orgs("rb", "Rbfam2", &["Acme"]))
        .await
        .expect("v2 on the primary");

    es.write_search_entries_page(&t, std::slice::from_ref(&v2))
        .await
        .into_iter()
        .for_each(|r| {
            r.expect("rebuild page entry written");
        });

    assert_eq!(wire.sweeps(), 0, "a rebuild never sweeps");
    assert!(
        containers_of(&es, &t, "Globex").await.is_empty(),
        "the obsolete contained document is not returned"
    );
    assert!(
        contained_hits(&es, &t, "Globex", ContainedReturn::Contained)
            .await
            .is_empty()
    );
    assert_eq!(containers_of(&es, &t, "Acme").await, vec!["Patient/rb"]);

    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}

/// Risk: a purge that trusts the container's id list leaves a child the list
/// never named alive with the same version, and it resurfaces when the resource
/// is recreated in that version (a primary that restarts its versions).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_purge_removes_a_contained_document_the_list_does_not_name() {
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = Arc::new(backend_via(&via, &prefix, 0).await);
    let t = tenant("sweep-purge-orphan");

    let v1 = es
        .create(
            &t,
            "Observation",
            holder("Observation", "o1", Some("Acme")),
            FhirVersion::default(),
        )
        .await
        .expect("create");

    let live = raw_contained_orgs(&prefix, "Acme").await;
    assert_eq!(live.len(), 1, "precondition: one registered child");
    let (index, _, live_source) = live[0].clone();
    let (_, _, container) = find_container(&prefix, "Observation", "o1").await;
    assert_eq!(
        container["contained_doc_ids"].as_array().map(Vec::len),
        Some(1),
        "precondition: the list names only the registered child"
    );

    // The orphan carries the container's current version; the list omits it.
    let orphan_id = "Organization_Observation_o1#orphan";
    let mut orphan = live_source.clone();
    orphan["contained_local_id"] = json!("orphan");
    orphan["resource_id"] = json!("Observation_o1#orphan");
    assert_eq!(orphan["version_id"], json!(v1.version_id()));
    index_raw(&index, orphan_id, orphan).await;
    assert_eq!(
        contained_hits(&es, &t, "Acme", ContainedReturn::Contained)
            .await
            .len(),
        2,
        "precondition: the orphan is visible while its container is live"
    );

    es.purge(&t, "Observation", "o1").await.expect("purge");

    // Recreated in version 1, without contained.
    let again = es
        .create(
            &t,
            "Observation",
            holder("Observation", "o1", None),
            FhirVersion::default(),
        )
        .await
        .expect("recreate");
    assert_eq!(again.version_id(), v1.version_id(), "same version again");

    for ret in [ContainedReturn::Contained, ContainedReturn::Container] {
        assert!(
            contained_hits(&es, &t, "Acme", ret).await.is_empty(),
            "the orphan does not resurface ({ret:?})"
        );
    }
    assert!(raw_contained_orgs(&prefix, "Acme").await.is_empty());
    assert_eq!(
        contained_doc_count(&prefix).await,
        0,
        "none left physically"
    );
    let client =
        Elasticsearch::new(Transport::single_node(&format!("http://{}", es_addr().await)).unwrap());
    let got = client
        .get(elasticsearch::GetParts::IndexId(&index, orphan_id))
        .send()
        .await
        .expect("get orphan");
    assert_eq!(got.status_code().as_u16(), 404, "the orphan is gone by _id");

    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}
