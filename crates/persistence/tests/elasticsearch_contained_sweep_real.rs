//! The contained-document sweep against a real Elasticsearch cluster.
//!
//! Every update and delete of a resource runs a `_delete_by_query` over the
//! tenant's indices to remove the container's stale `contained[]` documents.
//! These tests prove, end to end, what the sweep limiter and back-off add:
//!
//! 1. the sweep ALWAYS runs on update and delete (no skip), so stale contained
//!    documents are removed, including after a lost sync or a lost sweep;
//! 2. a burst of 100 concurrent deletes keeps the sweeps in flight at or below
//!    `contained_sweep_concurrency` (default 16) and gets no `429`;
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
use helios_persistence::backends::elasticsearch::{
    ElasticsearchBackend, ElasticsearchConfig, WriteRefreshPolicy,
};
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
};
use helios_persistence::core::{
    Backend, BackendKind, ConditionalDeleteResult, ConditionalStorage, EntityTagPrecondition,
    ResourceStorage, SearchProvider,
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
// 1: the sweep always runs, and removes what is stale
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_update_and_delete_sweeps_and_stale_contained_documents_are_removed() {
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = Arc::new(backend_via(&via, &prefix, 0).await);
    let composite = composite_over(es.clone());
    let t = tenant("sweep-real");

    // --- a resource without contained[]: still swept, as on main ---
    let plain = composite
        .create(
            &t,
            "Patient",
            patient("plain", "Plainfam", None),
            FhirVersion::default(),
        )
        .await
        .expect("create plain");
    let before = wire.sweeps();
    let updated = composite
        .update(&t, &plain, patient("plain", "Plainfam2", None))
        .await
        .expect("update plain");
    assert!(
        wire.sweeps() - before >= 1,
        "an update of a resource without contained[] still sweeps"
    );
    let before = wire.sweeps();
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
    assert!(
        wire.sweeps() - before >= 1,
        "a conditional delete still sweeps"
    );

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
        "the stale contained document was swept"
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
    assert_eq!(wire.too_many(), 0, "no 429 in the whole scenario");

    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}

// ---------------------------------------------------------------------------
// 3: 100 concurrent deletes stay under the sweep limiter and get no 429
// ---------------------------------------------------------------------------

/// Creates `n` patients, then deletes them all at once, and reports what the
/// wire saw during the deletes: (sweeps, peak in flight, 429s, failed sweeps).
async fn burst_of_deletes(n: usize, sweep_concurrency: usize) -> (usize, usize, usize, usize) {
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string()).await;
    let es = Arc::new(backend_via(&via, &prefix, sweep_concurrency).await);
    let t = tenant("sweep-burst");

    let creates: Vec<_> = (0..n)
        .map(|i| {
            let es = es.clone();
            let t = t.clone();
            tokio::spawn(async move {
                es.create(
                    &t,
                    "Patient",
                    patient(&format!("p{i}"), "Burst", None),
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
        d.await.unwrap().expect("every concurrent delete succeeds");
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
async fn a_hundred_concurrent_deletes_stay_under_the_default_sweep_limit_without_429() {
    // 0 is the default: 16 sweeps in flight.
    let (sweeps, peak, too_many, failed) = burst_of_deletes(100, 0).await;
    eprintln!(
        "default limiter: sweeps={sweeps} peak_in_flight={peak} 429={too_many} failed={failed}"
    );
    assert_eq!(sweeps, 100, "a delete sweeps, once each");
    assert!(peak <= 16, "at most 16 sweeps at once, saw {peak}");
    assert!(peak >= 2, "the burst really was concurrent, saw {peak}");
    assert_eq!(too_many, 0, "no 429 too_many_scroll_contexts");
    assert_eq!(failed, 0, "no sweep failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_configured_sweep_limit_is_the_one_enforced() {
    let (sweeps, peak, too_many, failed) = burst_of_deletes(100, 3).await;
    eprintln!("limit 3: sweeps={sweeps} peak_in_flight={peak} 429={too_many} failed={failed}");
    assert_eq!(sweeps, 100);
    assert!(peak <= 3, "at most 3 sweeps at once, saw {peak}");
    assert!(peak >= 2, "the limit was reached, saw {peak}");
    assert_eq!(too_many, 0);
    assert_eq!(failed, 0);
}

// ---------------------------------------------------------------------------
// 3: regression, the gap scenario: stale documents ARE removed (sweep always runs)
// ---------------------------------------------------------------------------

/// v1 has `contained[]`; v2 drops it but its sync to Elasticsearch never
/// happens (v2 is written to the primary alone, which is what a permanently
/// failed secondary sync leaves behind); v3 is a normal update. Elasticsearch
/// still holds v1's contained documents. With the sweep always running they
/// do not outlive v3.
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

    let before = wire.sweeps();
    composite
        .update(&t, &v2, patient("gap", "Gapfam3", None))
        .await
        .expect("v3");
    assert!(wire.sweeps() - before >= 1, "v3 sweeps");
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

    wire.fail_sweeps.store(true, Ordering::SeqCst);
    let before = wire.sweeps();
    let err = es
        .delete(&t, "Patient", "ex")
        .await
        .expect_err("a sweep that never succeeds must fail the delete, not drop it silently");
    // Each of the SWEEP_ATTEMPTS (4) rounds is itself sent up to 3 times by the
    // transport-level retry, so 12 requests reach the wire; at least 4 is the
    // contract, and the exact number is not asserted.
    let sent = wire.sweeps() - before;
    assert!(sent >= 4, "at least SWEEP_ATTEMPTS (4) sweeps, saw {sent}");
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

    // The same for an update replacing a resource that carried contained[].
    let before = wire.sweeps();
    es.write_search_entries(&t, &stored)
        .await
        .expect_err("a failing sweep fails the update's sync too");
    assert!(wire.sweeps() - before >= 4);

    // Once the cluster answers, repeating the delete finishes the job even
    // though the container's own document is already gone.
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

/// v2 drops contained[] but its SWEEP fails; the sweep works again for v3.
/// Nothing stale may remain.
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

    wire.fail_sweeps.store(true, Ordering::SeqCst);
    let _ = composite
        .update(&t, &v1, patient("lost", "Lostfam2", None))
        .await;
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
