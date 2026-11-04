//! Random chaos against a real Elasticsearch cluster for the contained-document
//! bookkeeping of #2291.
//!
//! A model of the TRUTH (per resource: the last write that finished `Ok` and its
//! set of contained children, or "deleted") is kept next to a few hundred random
//! operations that run concurrently (some on the very same resource at once)
//! through a TCP proxy that injects faults into `_bulk` and `_delete_by_query`:
//!
//! * (a) forward the request, then drop the connection before the answer comes
//!   back (ES applied it, the client sees a transport error);
//! * (b) answer `503` without forwarding;
//! * (c) delay the answer 100..400 ms (to force overlaps);
//! * (d) answer `429` to the whole request.
//!
//! A fault tends to come in a short burst (the next requests get the same
//! fault) so that the client's own retries are sometimes exhausted and an error
//! surfaces. After quiescence, with the proxy in no-fault mode, these are
//! checked:
//!
//! * I1 `_contained` searches (both `_containedType`, both child types) never fail
//!   (also while the chaos runs: a reader task searches all the time);
//! * I2 no visible child belongs to a missing or deleted container;
//! * I3 a resource whose last operation finished `Ok` and that had no failed or
//!   overlapping operation since shows exactly the children of that write;
//! * I4 after one final fault-free write per resource ("healing"), everything is
//!   exactly as the model says, with no extra and no missing child;
//! * I5 after fault-free delete/purge, nothing is visible and a purge of all
//!   leaves no contained document physically.
//!
//! Two layouts of version numbers:
//!
//! * `primary` (the one of the server binary): the version of a write comes from
//!   a primary, strictly increasing per resource (restarting after a purge), and
//!   reaches ES through `write_search_entries` / the page writers;
//! * `standalone`: ES numbers its versions itself (`create_or_update`,
//!   `update`), which the code documents as a case where a retry can reuse the
//!   version of a failed attempt.
//!
//! Resources `.../1` and `.../2` of each type are hit freely (overlaps are
//! welcome, so they become "uncertain"); `.../3` and `.../4` are driven strictly
//! one operation at a time in plan order (I3 is asserted on those).
//!
//! The seed is printed; `HFS_CHAOS_SEED`, `HFS_CHAOS_OPS` and `HFS_CHAOS_PM`
//! (per-mille start rate of each fault, default 60) override the defaults. The
//! plan (operations, children, concurrency) is reproducible from the seed; the
//! exact interleaving with the faults is not.
//!
//! Set `HFS_TEST_ES_URL` to use a running cluster; otherwise a 7.17.29 container
//! is started.
//!
//! Run with:
//! `cargo test -p helios-persistence --features R4,elasticsearch,sqlite --test elasticsearch_contained_chaos_random_real -- --nocapture --test-threads=1`

#![cfg(all(feature = "elasticsearch", feature = "sqlite"))]

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use elasticsearch::Elasticsearch;
use elasticsearch::http::transport::Transport;
use helios_fhir::FhirVersion;
use helios_persistence::StoredResource;
use helios_persistence::backends::elasticsearch::{
    ElasticsearchBackend, ElasticsearchConfig, WriteRefreshPolicy,
};
use helios_persistence::core::{Backend, PurgableStorage, ResourceStorage, SearchProvider};
use helios_persistence::error::{ResourceError, StorageError};
use helios_persistence::search::{ReindexTarget, SearchParameterLoader, TenantSearchRegistries};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{ContainedMode, ContainedReturn, SearchQuery};
use serde_json::{Value, json};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::elastic_search::ElasticSearch;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OnceCell, Semaphore};

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

const ES_IMAGE_TAG: &str = "7.17.29";
const ES_STARTUP_TIMEOUT: Duration = Duration::from_secs(300);

// ---------------------------------------------------------------------------
// The cluster
// ---------------------------------------------------------------------------

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

async fn direct_client() -> Elasticsearch {
    Elasticsearch::new(Transport::single_node(&format!("http://{}", es_addr().await)).unwrap())
}

// ---------------------------------------------------------------------------
// A tiny deterministic generator (splitmix64)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn pct(&mut self, p: u64) -> bool {
        self.below(100) < p
    }
}

// ---------------------------------------------------------------------------
// The fault-injecting proxy
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Fault {
    None,
    /// Forward, then drop the connection instead of relaying the answer.
    DropAfter,
    /// Answer 503 without forwarding.
    Refuse503,
    /// Relay the answer after this many milliseconds.
    Delay(u64),
    /// Answer 429 without forwarding.
    Refuse429,
    /// `_bulk` only: forward only some of the operations and report the rest
    /// as rejected items (`429`), the way a saturated write queue does: the
    /// bulk is not atomic.
    Partial,
}

struct ChaosState {
    rng: Rng,
    burst: Fault,
    left: u32,
}

struct Chaos {
    /// Faults to apply to the next requests, in order, whatever `active` says
    /// (the deterministic reproducers use it).
    script: Mutex<std::collections::VecDeque<Fault>>,
    /// A partial bulk drops only its last operation (the container).
    last_only: AtomicBool,
    active: AtomicBool,
    /// Start rate of each fault, per mille.
    pm: AtomicU32,
    state: Mutex<ChaosState>,
}

impl Chaos {
    fn new(seed: u64, pm: u32) -> Self {
        Chaos {
            script: Mutex::new(Default::default()),
            last_only: AtomicBool::new(false),
            active: AtomicBool::new(false),
            pm: AtomicU32::new(pm),
            state: Mutex::new(ChaosState {
                rng: Rng(seed ^ 0xC4A0_5000_DEAD_BEEF),
                burst: Fault::None,
                left: 0,
            }),
        }
    }

    /// Which of `n` operations a partial bulk drops: at least one and not all.
    fn pick_drops(&self, n: usize) -> Vec<bool> {
        let mut s = self.state.lock().unwrap();
        let mut d: Vec<bool> = if self.last_only.load(Ordering::SeqCst) || s.rng.pct(50) {
            (0..n).map(|i| i + 1 == n).collect()
        } else {
            (0..n).map(|_| s.rng.pct(50)).collect()
        };
        if !d.iter().any(|x| *x) {
            let i = s.rng.below(n as u64) as usize;
            d[i] = true;
        }
        if d.iter().all(|x| *x) {
            let i = s.rng.below(n as u64) as usize;
            d[i] = false;
        }
        d
    }

    fn decide(&self) -> Fault {
        if let Some(f) = self.script.lock().unwrap().pop_front() {
            return f;
        }
        if !self.active.load(Ordering::SeqCst) {
            return Fault::None;
        }
        let mut s = self.state.lock().unwrap();
        if s.left > 0 {
            s.left -= 1;
            return match s.burst {
                Fault::Delay(_) => Fault::Delay(100 + s.rng.below(301)),
                other => other,
            };
        }
        let pm = u64::from(self.pm.load(Ordering::SeqCst));
        let r = s.rng.below(1000);
        let fault = if r < pm {
            Fault::DropAfter
        } else if r < 2 * pm {
            Fault::Refuse503
        } else if r < 3 * pm {
            Fault::Delay(100 + s.rng.below(301))
        } else if r < 4 * pm {
            Fault::Refuse429
        } else if r < 5 * pm {
            Fault::Partial
        } else {
            Fault::None
        };
        if !matches!(fault, Fault::None | Fault::Delay(_)) {
            s.burst = fault;
            s.left = s.rng.below(9) as u32;
        }
        fault
    }
}

struct Wire {
    stop: AtomicBool,
    chaos: Chaos,
    t0: Instant,
    drops: AtomicUsize,
    f503: AtomicUsize,
    delays: AtomicUsize,
    f429: AtomicUsize,
    partials: AtomicUsize,
    organic_429: AtomicUsize,
    bulks: AtomicUsize,
    sweeps: AtomicUsize,
    log: Mutex<Vec<String>>,
}

impl Wire {
    fn summary(&self) -> String {
        format!(
            "requests(_bulk={} _dbq={}) injected(drop_after={} 503={} delay={} 429={} partial_bulk={}) organic429={}",
            self.bulks.load(Ordering::SeqCst),
            self.sweeps.load(Ordering::SeqCst),
            self.drops.load(Ordering::SeqCst),
            self.f503.load(Ordering::SeqCst),
            self.delays.load(Ordering::SeqCst),
            self.f429.load(Ordering::SeqCst),
            self.partials.load(Ordering::SeqCst),
            self.organic_429.load(Ordering::SeqCst),
        )
    }
}

/// Which operations of a `_bulk` the proxy lets through, and the items it
/// answers with a 429 for the others: `(let_through, (is_delete, index, id))`.
type PartialReply = (Vec<bool>, Vec<(bool, String, String)>);

/// The `_id`s named in the first chunk of a request (for the fault log).
fn ids_in(chunk: &[u8]) -> String {
    let text = String::from_utf8_lossy(chunk);
    let mut ids = Vec::new();
    let mut rest: &str = &text;
    while let Some(p) = rest.find("\"_id\":\"") {
        rest = &rest[p + 7..];
        if let Some(e) = rest.find('"') {
            ids.push(rest[..e].to_string());
            rest = &rest[e..];
        }
        if ids.len() >= 10 {
            break;
        }
    }
    ids.join(",")
}

struct Pending {
    fault: Fault,
    /// For a partial bulk: which operations were dropped, and what each one is
    /// (`is_delete`, `_index`, `_id`).
    partial: Option<PartialReply>,
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn content_length(head: &[u8]) -> Option<usize> {
    let text = String::from_utf8_lossy(head).to_lowercase();
    let p = text.find("content-length:")?;
    text[p + 15..].lines().next()?.trim().parse().ok()
}

/// Reads the rest of a request whose first chunk is `first`.
async fn read_full_request(
    cr: &mut tokio::net::tcp::OwnedReadHalf,
    first: &[u8],
) -> Option<Vec<u8>> {
    let mut req = first.to_vec();
    loop {
        if let Some(h) = find(&req, b"\r\n\r\n") {
            let cl = content_length(&req[..h])?;
            if req.len() >= h + 4 + cl {
                return Some(req);
            }
        }
        let mut tmp = vec![0u8; 64 * 1024];
        let n = cr.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        req.extend_from_slice(&tmp[..n]);
    }
}

type PartialPlan = (Vec<u8>, Vec<bool>, Vec<(bool, String, String)>);

/// The request without the dropped operations.
fn reduce_bulk(req: &[u8], chaos: &Chaos) -> Option<PartialPlan> {
    let h = find(req, b"\r\n\r\n")?;
    let head = &req[..h];
    let body = &req[h + 4..];
    let lines: Vec<&[u8]> = body
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    let mut ops: Vec<Vec<&[u8]>> = Vec::new();
    let mut meta: Vec<(bool, String, String)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let action: Value = serde_json::from_slice(lines[i]).ok()?;
        let (kind, spec) = action.as_object()?.iter().next()?;
        let is_delete = kind == "delete";
        meta.push((
            is_delete,
            spec["_index"].as_str()?.to_string(),
            spec["_id"].as_str()?.to_string(),
        ));
        if is_delete {
            ops.push(vec![lines[i]]);
            i += 1;
        } else {
            ops.push(vec![lines[i], *lines.get(i + 1)?]);
            i += 2;
        }
    }
    if ops.len() < 2 {
        return None;
    }
    let dropped = chaos.pick_drops(ops.len());
    let mut new_body: Vec<u8> = Vec::new();
    for (op, d) in ops.iter().zip(&dropped) {
        if !*d {
            for l in op {
                new_body.extend_from_slice(l);
                new_body.push(b'\n');
            }
        }
    }
    let head_text = String::from_utf8_lossy(head);
    let mut new_head = String::new();
    for line in head_text.split("\r\n") {
        if line.to_lowercase().starts_with("content-length:") {
            new_head.push_str(&format!("content-length: {}\r\n", new_body.len()));
        } else {
            new_head.push_str(line);
            new_head.push_str("\r\n");
        }
    }
    let mut out = new_head.into_bytes();
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&new_body);
    Some((out, dropped, meta))
}

/// Reads a whole content-length response whose first chunk is `first`.
async fn read_full_response(
    sr: &mut tokio::net::tcp::OwnedReadHalf,
    first: &[u8],
) -> Option<Value> {
    let mut resp = first.to_vec();
    loop {
        if let Some(h) = find(&resp, b"\r\n\r\n") {
            let cl = content_length(&resp[..h])?;
            if resp.len() >= h + 4 + cl {
                return serde_json::from_slice(&resp[h + 4..h + 4 + cl]).ok();
            }
        }
        let mut tmp = vec![0u8; 64 * 1024];
        let n = sr.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        resp.extend_from_slice(&tmp[..n]);
    }
}

/// The answer a client sees for a partial bulk: the cluster's own items for
/// the operations it ran, a `429` item for each dropped one.
fn fabricate(dropped: &[bool], meta: &[(bool, String, String)], real: &Value) -> Vec<u8> {
    let real_items = real["items"].as_array().cloned().unwrap_or_default();
    let mut next = real_items.into_iter();
    let items: Vec<Value> = dropped
        .iter()
        .zip(meta)
        .map(|(d, (is_delete, index, id))| {
            if *d {
                let key = if *is_delete { "delete" } else { "index" };
                json!({ key: {
                    "_index": index, "_id": id, "status": 429,
                    "error": { "type": "es_rejected_execution_exception",
                               "reason": "simulated rejected execution" },
                }})
            } else {
                next.next().unwrap_or(Value::Null)
            }
        })
        .collect();
    let body = serde_json::to_vec(&json!({ "took": 1, "errors": true, "items": items })).unwrap();
    let mut out = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json; charset=UTF-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(&body);
    out
}

async fn start_proxy(upstream: String, seed: u64, pm: u32) -> (String, Arc<Wire>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind proxy");
    let addr = listener.local_addr().expect("proxy addr").to_string();
    let wire = Arc::new(Wire {
        stop: AtomicBool::new(false),
        chaos: Chaos::new(seed, pm),
        t0: Instant::now(),
        drops: AtomicUsize::new(0),
        f503: AtomicUsize::new(0),
        delays: AtomicUsize::new(0),
        f429: AtomicUsize::new(0),
        partials: AtomicUsize::new(0),
        organic_429: AtomicUsize::new(0),
        bulks: AtomicUsize::new(0),
        sweeps: AtomicUsize::new(0),
        log: Mutex::new(Vec::new()),
    });
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
                let outstanding: Arc<Mutex<Option<Pending>>> = Default::default();
                let up_state = outstanding.clone();
                let up_wire = wire.clone();
                let up = tokio::spawn(async move {
                    let mut buf = vec![0u8; 64 * 1024];
                    loop {
                        let n = match cr.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        let mut refusal: Option<&'static [u8]> = None;
                        {
                            let mut state = up_state.lock().unwrap();
                            if state.is_none() {
                                let head = &buf[..n.min(512)];
                                let first_line = head.split(|b| *b == b'\n').next().unwrap_or(head);
                                let is_bulk = first_line.windows(5).any(|w| w == b"_bulk");
                                let is_dbq =
                                    first_line.windows(16).any(|w| w == b"_delete_by_query");
                                let mut fault = Fault::None;
                                if is_bulk || is_dbq {
                                    if is_bulk {
                                        up_wire.bulks.fetch_add(1, Ordering::SeqCst);
                                    } else {
                                        up_wire.sweeps.fetch_add(1, Ordering::SeqCst);
                                    }
                                    fault = up_wire.chaos.decide();
                                    if fault == Fault::Partial && !is_bulk {
                                        fault = Fault::None;
                                    }
                                    if fault != Fault::None {
                                        let counter = match fault {
                                            Fault::DropAfter => &up_wire.drops,
                                            Fault::Refuse503 => &up_wire.f503,
                                            Fault::Delay(_) => &up_wire.delays,
                                            Fault::Refuse429 => &up_wire.f429,
                                            Fault::Partial => &up_wire.partials,
                                            Fault::None => unreachable!(),
                                        };
                                        counter.fetch_add(1, Ordering::SeqCst);
                                        up_wire.log.lock().unwrap().push(format!(
                                            "[{:>6}ms] {} {:?} ids={}",
                                            up_wire.t0.elapsed().as_millis(),
                                            if is_bulk { "_bulk" } else { "_dbq" },
                                            fault,
                                            ids_in(&buf[..n.min(8192)])
                                        ));
                                    }
                                }
                                match fault {
                                    Fault::Refuse503 => {
                                        refusal = Some(
                                            b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                                        );
                                    }
                                    Fault::Refuse429 => {
                                        refusal = Some(
                                            b"HTTP/1.1 429 Too Many Requests\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                                        );
                                    }
                                    _ => {}
                                }
                                *state = Some(Pending {
                                    fault,
                                    partial: None,
                                });
                            }
                        }
                        if let Some(bytes) = refusal {
                            let _ = refuse.lock().await.write_all(bytes).await;
                            break;
                        }
                        let is_partial = up_state
                            .lock()
                            .unwrap()
                            .as_ref()
                            .is_some_and(|p| p.fault == Fault::Partial && p.partial.is_none());
                        if is_partial {
                            let Some(req) = read_full_request(&mut cr, &buf[..n]).await else {
                                break;
                            };
                            match reduce_bulk(&req, &up_wire.chaos) {
                                Some((reduced, dropped, meta)) => {
                                    up_state.lock().unwrap().as_mut().unwrap().partial =
                                        Some((dropped.clone(), meta));
                                    up_wire
                                        .log
                                        .lock()
                                        .unwrap()
                                        .push(format!("    partial drop pattern {dropped:?}"));
                                    if sw.write_all(&reduced).await.is_err() {
                                        break;
                                    }
                                }
                                None => {
                                    up_state.lock().unwrap().as_mut().unwrap().fault = Fault::None;
                                    if sw.write_all(&req).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            continue;
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
                    if let Some(p) = answered {
                        if buf[..n].starts_with(b"HTTP/1.1 429") {
                            wire.organic_429.fetch_add(1, Ordering::SeqCst);
                        }
                        if let Some((dropped, meta)) = &p.partial {
                            let Some(real) = read_full_response(&mut sr, &buf[..n]).await else {
                                break;
                            };
                            let out = fabricate(dropped, meta, &real);
                            let _ = cw.lock().await.write_all(&out).await;
                            break;
                        }
                        match p.fault {
                            Fault::DropAfter => break,
                            Fault::Delay(ms) => {
                                tokio::time::sleep(Duration::from_millis(ms)).await;
                            }
                            _ => {}
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

async fn backend_via(via: &str, prefix: &str) -> ElasticsearchBackend {
    let config = ElasticsearchConfig {
        nodes: vec![format!("http://{via}")],
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

async fn cleanup(prefix: &str) {
    let client = direct_client().await;
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

/// Every contained document under `prefix`, visible or not (direct, no proxy).
async fn contained_doc_count(prefix: &str) -> u64 {
    let client = direct_client().await;
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
    body["count"].as_u64().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// The universe: 3 container types x 4 ids (same ids in every type)
// ---------------------------------------------------------------------------

const TYPES: [&str; 3] = ["Patient", "Observation", "Practitioner"];
const IDS: [&str; 4] = ["1", "2", "3", "4"];
const CHILD_TYPES: [&str; 2] = ["Organization", "Location"];
const POOL: [&str; 5] = ["o0", "o1", "o2", "l0", "l1"];
const KEYS: usize = 12;

fn key_type(key: usize) -> &'static str {
    TYPES[key / IDS.len()]
}
fn key_id(key: usize) -> &'static str {
    IDS[key % IDS.len()]
}
fn key_name(key: usize) -> String {
    format!("{}/{}", key_type(key), key_id(key))
}
/// Ids `3` and `4` are driven one operation at a time, in plan order.
fn serialized(key: usize) -> bool {
    key % IDS.len() >= 2
}
fn child_type(local: &str) -> &'static str {
    if local.starts_with('o') {
        "Organization"
    } else {
        "Location"
    }
}

/// The resource: `language` carries the write's tag, every child's `name`
/// carries `<container>#<local>@<tag>`, so a visible child says exactly which
/// write it came from.
fn content(key: usize, tag: &str, children: &BTreeSet<String>) -> Value {
    let mut r = json!({
        "resourceType": key_type(key), "id": key_id(key), "language": tag,
    });
    if !children.is_empty() {
        r["contained"] = Value::Array(
            children
                .iter()
                .map(|local| {
                    json!({
                        "resourceType": child_type(local), "id": local,
                        "name": format!("{}#{}@{}", key_name(key), local, tag),
                    })
                })
                .collect(),
        );
    }
    r
}

#[derive(Clone, Debug, PartialEq, Default)]
enum Truth {
    #[default]
    Unwritten,
    Deleted,
    Written {
        tag: String,
        children: BTreeSet<String>,
    },
}

#[derive(Default)]
struct ResState {
    truth: Truth,
    /// A failed operation, or an overlap, since the last clean `Ok`.
    dirty: bool,
    inflight: Vec<u64>,
    overlapped: HashSet<u64>,
    ok: usize,
    failed: usize,
    overlaps: usize,
}

struct Resource {
    state: Mutex<ResState>,
    order: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Primary,
    Standalone,
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Write,
    Page { synced: bool },
    Upsert,
    Update,
    Delete,
    Purge,
}

#[derive(Clone)]
struct Item {
    key: usize,
    version: u64,
    tag: String,
    children: BTreeSet<String>,
}

#[derive(Clone)]
struct Op {
    id: u64,
    kind: Kind,
    items: Vec<Item>,
}

fn random_children(rng: &mut Rng) -> BTreeSet<String> {
    let count = rng.below(4) as usize;
    let mut pool: Vec<&str> = POOL.to_vec();
    let mut out = BTreeSet::new();
    for _ in 0..count {
        let i = rng.below(pool.len() as u64) as usize;
        out.insert(pool.remove(i).to_string());
    }
    out
}

struct Plan {
    ops: Vec<Op>,
    /// The version the next write of each key gets.
    next: [u64; KEYS],
    /// The highest version ever issued per key.
    high: [u64; KEYS],
}

fn pick_key(rng: &mut Rng, avoid: &[usize], last_key: usize) -> usize {
    loop {
        // 25 %: the same resource as the previous operation (they will overlap).
        let k = if avoid.is_empty() && rng.pct(25) {
            last_key
        } else {
            rng.below(KEYS as u64) as usize
        };
        if !avoid.contains(&k) {
            return k;
        }
    }
}

fn make_plan(rng: &mut Rng, n: usize, mode: Mode) -> Plan {
    let mut next = [1u64; KEYS];
    let mut high = [0u64; KEYS];
    let mut ops = Vec::with_capacity(n);
    let mut last_key = 0usize;
    for id in 0..n as u64 {
        let kind = {
            let r = rng.below(100);
            match mode {
                Mode::Primary => match r {
                    0..=39 => Kind::Write,
                    40..=51 => Kind::Page { synced: false },
                    52..=67 => Kind::Page { synced: true },
                    68..=83 => Kind::Delete,
                    _ => Kind::Purge,
                },
                Mode::Standalone => match r {
                    0..=39 => Kind::Upsert,
                    40..=74 => Kind::Update,
                    75..=89 => Kind::Delete,
                    _ => Kind::Purge,
                },
            }
        };
        let item_count = match kind {
            Kind::Page { .. } => 1 + rng.below(4) as usize,
            _ => 1,
        };
        let mut keys: Vec<usize> = Vec::new();
        for _ in 0..item_count {
            let k = pick_key(rng, &keys, last_key);
            keys.push(k);
        }
        last_key = keys[0];
        let mut items = Vec::new();
        for (i, key) in keys.into_iter().enumerate() {
            let writes = matches!(
                kind,
                Kind::Write | Kind::Page { .. } | Kind::Upsert | Kind::Update
            );
            let version = if writes && mode == Mode::Primary {
                let v = next[key];
                next[key] += 1;
                high[key] = high[key].max(v);
                v
            } else {
                0
            };
            if matches!(kind, Kind::Purge) && mode == Mode::Primary {
                // A primary that purges restarts its versions.
                next[key] = 1;
            }
            items.push(Item {
                key,
                version,
                tag: format!("w{id}x{i}"),
                children: if writes {
                    random_children(rng)
                } else {
                    BTreeSet::new()
                },
            });
        }
        ops.push(Op { id, kind, items });
    }
    Plan { ops, next, high }
}

// ---------------------------------------------------------------------------
// Running the operations
// ---------------------------------------------------------------------------

struct Ctx {
    es: ElasticsearchBackend,
    t: TenantContext,
    res: Vec<Resource>,
    #[allow(dead_code)]
    wire: Arc<Wire>,
    t0: Instant,
    oplog: Mutex<Vec<String>>,
    first_errors: Mutex<Vec<String>>,
}

enum Outcome {
    Ok,
    Err(String),
    /// Nothing was attempted against the cluster (an `update` of an absent
    /// resource).
    Skip,
}

fn stored_of(ctx: &Ctx, it: &Item) -> StoredResource {
    let now = chrono::Utc::now();
    StoredResource::from_storage(
        key_type(it.key).to_string(),
        key_id(it.key).to_string(),
        it.version.to_string(),
        ctx.t.tenant_id().clone(),
        content(it.key, &it.tag, &it.children),
        now,
        now,
        None,
        FhirVersion::default(),
    )
}

fn short(e: &impl std::fmt::Display) -> String {
    let s = e.to_string();
    s.chars().take(200).collect()
}

fn is_not_found(e: &StorageError) -> bool {
    matches!(e, StorageError::Resource(ResourceError::NotFound { .. }))
}

impl Ctx {
    fn begin(&self, op: u64, key: usize) {
        let mut s = self.res[key].state.lock().unwrap();
        if !s.inflight.is_empty() {
            let running: Vec<u64> = s.inflight.clone();
            s.overlapped.extend(running);
            s.overlapped.insert(op);
            s.overlaps += 1;
            s.dirty = true;
        }
        s.inflight.push(op);
    }

    fn end(&self, op: u64, key: usize, truth_if_ok: Truth, outcome: &Outcome) {
        let mut s = self.res[key].state.lock().unwrap();
        s.inflight.retain(|x| *x != op);
        let was_overlapped = s.overlapped.remove(&op);
        match outcome {
            Outcome::Ok => {
                s.ok += 1;
                s.truth = truth_if_ok;
                s.dirty = was_overlapped;
            }
            Outcome::Err(_) => {
                s.failed += 1;
                s.dirty = true;
            }
            Outcome::Skip => {
                // Nothing happened; an overlap still blurs the picture.
                if was_overlapped {
                    s.dirty = true;
                }
            }
        }
    }

    async fn run(&self, op: &Op, mode: Mode) {
        for it in &op.items {
            self.begin(op.id, it.key);
        }
        let started = self.t0.elapsed().as_millis();
        let outcomes: Vec<Outcome> = match op.kind {
            Kind::Write => {
                let it = &op.items[0];
                let stored = stored_of(self, it);
                vec![match self.es.write_search_entries(&self.t, &stored).await {
                    Ok(_) => Outcome::Ok,
                    Err(e) => Outcome::Err(short(&e)),
                }]
            }
            Kind::Page { synced } => {
                let stored: Vec<StoredResource> =
                    op.items.iter().map(|it| stored_of(self, it)).collect();
                let results = if synced {
                    self.es.write_synced_entries_page(&self.t, &stored).await
                } else {
                    self.es.write_search_entries_page(&self.t, &stored).await
                };
                (0..op.items.len())
                    .map(|i| match results.get(i) {
                        Some(Ok(_)) => Outcome::Ok,
                        Some(Err(e)) => Outcome::Err(short(e)),
                        None => Outcome::Err("page result missing".to_string()),
                    })
                    .collect()
            }
            Kind::Upsert => {
                let it = &op.items[0];
                let value = content(it.key, &it.tag, &it.children);
                vec![match self
                    .es
                    .create_or_update(
                        &self.t,
                        key_type(it.key),
                        key_id(it.key),
                        value,
                        FhirVersion::default(),
                    )
                    .await
                {
                    Ok(_) => Outcome::Ok,
                    Err(e) => Outcome::Err(short(&e)),
                }]
            }
            Kind::Update => {
                let it = &op.items[0];
                let value = content(it.key, &it.tag, &it.children);
                vec![match self
                    .es
                    .read(&self.t, key_type(it.key), key_id(it.key))
                    .await
                {
                    Ok(Some(current)) => match self.es.update(&self.t, &current, value).await {
                        Ok(_) => Outcome::Ok,
                        Err(e) => Outcome::Err(short(&e)),
                    },
                    Ok(None) | Err(_) => Outcome::Skip,
                }]
            }
            Kind::Delete => {
                let it = &op.items[0];
                vec![match self
                    .es
                    .delete(&self.t, key_type(it.key), key_id(it.key))
                    .await
                {
                    Ok(()) => Outcome::Ok,
                    Err(e) if is_not_found(&e) => Outcome::Ok,
                    Err(e) => Outcome::Err(short(&e)),
                }]
            }
            Kind::Purge => {
                // The composite fan-out repeats a failed purge; so do we.
                let it = &op.items[0];
                let mut last = Outcome::Skip;
                for attempt in 0..8 {
                    match self
                        .es
                        .purge(&self.t, key_type(it.key), key_id(it.key))
                        .await
                    {
                        Ok(()) => {
                            last = Outcome::Ok;
                            break;
                        }
                        Err(e) => {
                            last = Outcome::Err(short(&e));
                            tokio::time::sleep(Duration::from_millis(30 * (attempt + 1))).await;
                        }
                    }
                }
                vec![last]
            }
        };
        let finished = self.t0.elapsed().as_millis();
        let _ = mode;
        for (it, outcome) in op.items.iter().zip(outcomes.iter()) {
            let truth = match op.kind {
                Kind::Delete | Kind::Purge => Truth::Deleted,
                _ => Truth::Written {
                    tag: it.tag.clone(),
                    children: it.children.clone(),
                },
            };
            if let Outcome::Err(msg) = outcome {
                let mut first = self.first_errors.lock().unwrap();
                if first.len() < 12 {
                    first.push(format!("op#{} {:?}: {msg}", op.id, op.kind));
                }
            }
            self.oplog.lock().unwrap().push(format!(
                "[{started:>6}..{finished:>6}ms] op#{} {:?} {} v{} children={:?} -> {}",
                op.id,
                op.kind,
                key_name(it.key),
                it.version,
                it.children,
                match outcome {
                    Outcome::Ok => "OK".to_string(),
                    Outcome::Skip => "skip".to_string(),
                    Outcome::Err(m) => format!("ERR {m}"),
                }
            ));
            self.end(op.id, it.key, truth, outcome);
        }
    }
}

// ---------------------------------------------------------------------------
// Observing
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Vis {
    container: String,
    rtype: String,
    local: String,
    tag: String,
    version: String,
}

struct Obs {
    vis: Vec<Vis>,
    container_mode: BTreeSet<String>,
    errors: Vec<String>,
}

async fn search_items(
    es: &ElasticsearchBackend,
    t: &TenantContext,
    child: &str,
    ret: ContainedReturn,
) -> Result<Vec<StoredResource>, String> {
    let mut q = SearchQuery::new(child);
    q.contained = ContainedMode::On;
    q.contained_return = ret;
    q.count = Some(1000);
    es.search(t, &q)
        .await
        .map(|r| r.resources.items)
        .map_err(|e| format!("{child}/{ret:?}: {}", short(&e)))
}

async fn observe(ctx: &Ctx) -> Obs {
    for ty in TYPES.iter().chain(CHILD_TYPES.iter()) {
        let _ = ctx.es.refresh_index(ctx.t.tenant_id().as_str(), ty).await;
    }
    let mut obs = Obs {
        vis: Vec::new(),
        container_mode: BTreeSet::new(),
        errors: Vec::new(),
    };
    for child in CHILD_TYPES {
        match search_items(&ctx.es, &ctx.t, child, ContainedReturn::Contained).await {
            Ok(items) => {
                for item in items {
                    let name = item.content()["name"].as_str().unwrap_or("").to_string();
                    let (identity, tag) = name.split_once('@').unwrap_or((&name, ""));
                    let (container, local) = identity.split_once('#').unwrap_or(("?", identity));
                    obs.vis.push(Vis {
                        container: container.to_string(),
                        rtype: item.resource_type().to_string(),
                        local: local.to_string(),
                        tag: tag.to_string(),
                        version: item.version_id().to_string(),
                    });
                }
            }
            Err(e) => obs.errors.push(e),
        }
        match search_items(&ctx.es, &ctx.t, child, ContainedReturn::Container).await {
            Ok(items) => {
                for item in items {
                    obs.container_mode
                        .insert(format!("{}/{}", item.resource_type(), item.id()));
                }
            }
            Err(e) => obs.errors.push(e),
        }
    }
    obs
}

#[derive(Default)]
struct Findings {
    i1: Vec<String>,
    i2: Vec<String>,
    i3: Vec<String>,
    /// Resources (clean, dirty) in each shape of inconsistency.
    shapes: BTreeMap<&'static str, (usize, usize)>,
    clean: usize,
    dirty: usize,
}

async fn analyze(ctx: &Ctx, obs: &Obs) -> Findings {
    let mut f = Findings::default();
    f.i1.extend(obs.errors.iter().cloned());

    // The two views of the same data must name the same containers.
    let from_contained: BTreeSet<String> = obs.vis.iter().map(|v| v.container.clone()).collect();
    if from_contained != obs.container_mode {
        f.i2.push(format!(
            "container-mode and contained-mode searches disagree: contained says {from_contained:?}, container says {:?}",
            obs.container_mode
        ));
    }
    let known: BTreeSet<String> = (0..KEYS).map(key_name).collect();
    for v in &obs.vis {
        if !known.contains(&v.container) {
            f.i2.push(format!("visible child of unknown container: {v:?}"));
        }
    }

    for key in 0..KEYS {
        let (truth, dirty) = {
            let s = ctx.res[key].state.lock().unwrap();
            (s.truth.clone(), s.dirty)
        };
        let name = key_name(key);
        if dirty {
            f.dirty += 1;
        } else {
            f.clean += 1;
        }
        let mine: Vec<&Vis> = obs.vis.iter().filter(|v| v.container == name).collect();
        let container = match ctx.es.read(&ctx.t, key_type(key), key_id(key)).await {
            Ok(c) => c,
            Err(e) => {
                f.i1.push(format!("GET {name}: {}", short(&e)));
                continue;
            }
        };
        let present = container.is_some();
        let mut shape = |label: &'static str| {
            let e = f.shapes.entry(label).or_insert((0, 0));
            if dirty {
                e.1 += 1;
            } else {
                e.0 += 1;
            }
        };
        let mut seen = BTreeSet::new();
        let mut dup = false;
        for v in &mine {
            if !seen.insert((v.rtype.clone(), v.local.clone())) {
                dup = true;
            }
        }
        if dup {
            f.i2.push(format!("{name}: the same child is listed twice: {mine:?}"));
        }
        if let Some(c) = &container {
            for v in &mine {
                if v.version != c.version_id() {
                    f.i2.push(format!(
                        "{name}: visible child {v:?} has version {} but the container has {}",
                        v.version,
                        c.version_id()
                    ));
                }
            }
        }
        if !present && !mine.is_empty() {
            shape("child of a deleted/missing container");
            f.i2.push(format!(
                "{name}: container is gone but children are visible: {mine:?}"
            ));
        }
        match &truth {
            Truth::Written { tag, children } => {
                let extra: Vec<String> = mine
                    .iter()
                    .filter(|v| !children.contains(&v.local) || &v.tag != tag)
                    .map(|v| format!("{}@{}", v.local, v.tag))
                    .collect();
                let hidden: Vec<&String> = children
                    .iter()
                    .filter(|c| !mine.iter().any(|v| &v.local == *c))
                    .collect();
                if !extra.is_empty() {
                    shape("extra child visible");
                }
                if !hidden.is_empty() {
                    shape("current child hidden");
                }
                if !children.is_empty() && mine.is_empty() && present {
                    shape("container without children");
                }
                if !present {
                    shape("container missing");
                }
                if !dirty && (!extra.is_empty() || !hidden.is_empty() || !present) {
                    f.i3.push(format!(
                        "{name}: last clean write {tag} has children {children:?}; \
                         visible {:?}; extra={extra:?} hidden={hidden:?} container_present={present}",
                        mine.iter()
                            .map(|v| format!("{}@{}", v.local, v.tag))
                            .collect::<Vec<_>>()
                    ));
                }
            }
            Truth::Deleted | Truth::Unwritten => {
                if present {
                    shape("deleted container resurrected");
                }
                if !dirty && (present || !mine.is_empty()) {
                    f.i2.push(format!(
                        "{name}: deleted/purged cleanly but container_present={present}, visible={mine:?}"
                    ));
                }
            }
        }
    }
    f
}

// ---------------------------------------------------------------------------
// The campaign
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Report {
    seed: u64,
    mode: Option<Mode>,
    ops: usize,
    conc: usize,
    wire: String,
    surfaced_errors: usize,
    overlaps: usize,
    clean_before: usize,
    dirty_before: usize,
    shapes_before: BTreeMap<&'static str, (usize, usize)>,
    physical_before: u64,
    visible_before: usize,
    physical_after_heal: u64,
    visible_after_heal: usize,
    i1: Vec<String>,
    i2: Vec<String>,
    i3: Vec<String>,
    i4: Vec<String>,
    i5: Vec<String>,
    reader_searches: usize,
    first_errors: Vec<String>,
    trace: Vec<String>,
}

impl Report {
    fn print(&self) {
        eprintln!("==================== CHAOS REPORT ====================");
        eprintln!(
            "seed={} mode={:?} ops={} concurrency={}",
            self.seed,
            self.mode.unwrap(),
            self.ops,
            self.conc
        );
        eprintln!("wire: {}", self.wire);
        eprintln!(
            "operations that surfaced an error: {} ; overlapping starts: {} ; reader searches during chaos: {}",
            self.surfaced_errors, self.overlaps, self.reader_searches
        );
        for e in &self.first_errors {
            eprintln!("  first error: {e}");
        }
        eprintln!(
            "BEFORE healing: {} resources clean, {} uncertain (failed or overlapped); \
             visible children={} physical contained docs={}",
            self.clean_before, self.dirty_before, self.visible_before, self.physical_before
        );
        if self.shapes_before.is_empty() {
            eprintln!("  no resource diverges from the last-Ok model");
        }
        for (shape, (clean, dirty)) in &self.shapes_before {
            eprintln!("  shape '{shape}': {clean} clean resources, {dirty} uncertain resources");
        }
        eprintln!(
            "AFTER healing: visible children={} physical contained docs={} (the rest are hidden by the version filter)",
            self.visible_after_heal, self.physical_after_heal
        );
        eprintln!(
            "violations: I1={} I2={} I3={} I4={} I5={}",
            self.i1.len(),
            self.i2.len(),
            self.i3.len(),
            self.i4.len(),
            self.i5.len()
        );
        for (name, list) in [
            ("I1", &self.i1),
            ("I2", &self.i2),
            ("I3", &self.i3),
            ("I4", &self.i4),
            ("I5", &self.i5),
        ] {
            for v in list.iter().take(8) {
                eprintln!("  {name}: {v}");
            }
        }
        if !self.trace.is_empty() {
            eprintln!("-- operation trace of the resources in violation --");
            for line in self.trace.iter().take(80) {
                eprintln!("  {line}");
            }
        }
        eprintln!("======================================================");
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

async fn campaign(seed: u64, ops: usize, mode: Mode) -> Report {
    let pm = env_u64("HFS_CHAOS_PM", 60) as u32;
    eprintln!("chaos seed={seed} ops={ops} mode={mode:?} pm={pm}");
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string(), seed, pm).await;
    let es = backend_via(&via, &prefix).await;
    let ctx = Arc::new(Ctx {
        es,
        t: tenant(&format!("chaos-{seed}")),
        res: (0..KEYS)
            .map(|_| Resource {
                state: Mutex::new(ResState::default()),
                order: Arc::new(tokio::sync::Mutex::new(())),
            })
            .collect(),
        wire: wire.clone(),
        t0: Instant::now(),
        oplog: Mutex::new(Vec::new()),
        first_errors: Mutex::new(Vec::new()),
    });

    let mut rng = Rng(seed);
    let conc = 4 + rng.below(5) as usize;
    let plan = make_plan(&mut rng, ops, mode);
    let mut report = Report {
        seed,
        mode: Some(mode),
        ops,
        conc,
        ..Default::default()
    };

    // A reader that searches all the time while the chaos runs (I1).
    let reader_stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let ctx = ctx.clone();
        let stop = reader_stop.clone();
        tokio::spawn(async move {
            let mut errors = Vec::new();
            let mut searches = 0usize;
            while !stop.load(Ordering::SeqCst) {
                for child in CHILD_TYPES {
                    for ret in [ContainedReturn::Contained, ContainedReturn::Container] {
                        searches += 1;
                        if let Err(e) = search_items(&ctx.es, &ctx.t, child, ret).await
                            && errors.len() < 20
                        {
                            errors.push(format!("during chaos: {e}"));
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            (errors, searches)
        })
    };

    // The chaos.
    wire.chaos.active.store(true, Ordering::SeqCst);
    let sem = Arc::new(Semaphore::new(conc));
    let mut handles = Vec::new();
    for op in plan.ops.iter().cloned() {
        let permit = sem.clone().acquire_owned().await.unwrap();
        // Serialized resources are taken here, in plan order.
        let mut keys: Vec<usize> = op
            .items
            .iter()
            .map(|i| i.key)
            .filter(|k| serialized(*k))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        let mut guards = Vec::new();
        for k in keys {
            guards.push(ctx.res[k].order.clone().lock_owned().await);
        }
        let ctx2 = ctx.clone();
        handles.push(tokio::spawn(async move {
            ctx2.run(&op, mode).await;
            drop(guards);
            drop(permit);
        }));
        if rng.pct(30) {
            tokio::time::sleep(Duration::from_millis(rng.below(25))).await;
        }
    }
    for h in handles {
        h.await.expect("operation task");
    }
    wire.chaos.active.store(false, Ordering::SeqCst);
    reader_stop.store(true, Ordering::SeqCst);
    let (reader_errors, searches) = reader.await.unwrap();
    report.reader_searches = searches;
    report.i1.extend(reader_errors);
    report.wire = wire.summary();
    report.first_errors = ctx.first_errors.lock().unwrap().clone();
    for r in &ctx.res {
        let s = r.state.lock().unwrap();
        report.surfaced_errors += s.failed;
        report.overlaps += s.overlaps;
    }

    // Quiescence: what the model says before any healing.
    let obs = observe(&ctx).await;
    let f = analyze(&ctx, &obs).await;
    report.clean_before = f.clean;
    report.dirty_before = f.dirty;
    report.shapes_before = f.shapes.clone();
    report.visible_before = obs.vis.len();
    report.physical_before = contained_doc_count(&prefix).await;
    report.i1.extend(f.i1);
    report.i2.extend(f.i2);
    report.i3.extend(f.i3);

    // Healing: one final fault-free write per resource.
    let mut heal_rng = Rng(seed ^ 0x4EA1);
    for key in 0..KEYS {
        let children = if key % 3 == 0 {
            POOL.iter().take(3).map(|s| s.to_string()).collect()
        } else if key % 3 == 1 {
            BTreeSet::new()
        } else {
            random_children(&mut heal_rng)
        };
        let version = if serialized(key) {
            plan.next[key]
        } else {
            plan.high[key] + 1
        };
        let it = Item {
            key,
            version,
            tag: format!("heal{key}"),
            children,
        };
        let op = Op {
            id: 1_000_000 + key as u64,
            kind: match mode {
                Mode::Primary => Kind::Write,
                Mode::Standalone => Kind::Upsert,
            },
            items: vec![it],
        };
        ctx.run(&op, mode).await;
        let ok = ctx.res[key].state.lock().unwrap().dirty;
        // `dirty` is false after a clean Ok; a failure leaves it true.
        if ok {
            report.i4.push(format!(
                "heal write of {} failed or was not clean",
                key_name(key)
            ));
        }
    }
    let obs = observe(&ctx).await;
    let f = analyze(&ctx, &obs).await;
    report.i1.extend(f.i1);
    report.i4.extend(f.i2);
    report.i4.extend(f.i3);
    report.visible_after_heal = obs.vis.len();
    report.physical_after_heal = contained_doc_count(&prefix).await;

    // I5: clean deletes and purges.
    for key in 0..KEYS {
        let kind = if key % 2 == 0 {
            Kind::Delete
        } else {
            Kind::Purge
        };
        let op = Op {
            id: 2_000_000 + key as u64,
            kind,
            items: vec![Item {
                key,
                version: 0,
                tag: String::new(),
                children: BTreeSet::new(),
            }],
        };
        ctx.run(&op, mode).await;
    }
    let obs = observe(&ctx).await;
    let f = analyze(&ctx, &obs).await;
    report.i1.extend(f.i1);
    report.i5.extend(f.i2);
    report.i5.extend(f.i3);
    if !obs.vis.is_empty() {
        report.i5.push(format!(
            "children visible after deleting everything: {:?}",
            obs.vis
        ));
    }
    // Purge the ones that were only deleted: a purge always sweeps, so nothing
    // may be left physically.
    for key in 0..KEYS {
        let _ = ctx.es.purge(&ctx.t, key_type(key), key_id(key)).await;
    }
    let left = contained_doc_count(&prefix).await;
    if left != 0 {
        report.i5.push(format!(
            "{left} contained documents physically left after purging everything"
        ));
    }

    // The trace of the resources that broke something.
    let mut bad: BTreeSet<String> = BTreeSet::new();
    for list in [&report.i2, &report.i3, &report.i4, &report.i5] {
        for line in list {
            for key in 0..KEYS {
                if line.starts_with(&key_name(key)) {
                    bad.insert(key_name(key));
                }
            }
        }
    }
    if !bad.is_empty() {
        let oplog = ctx.oplog.lock().unwrap().clone();
        let mut ordered: Vec<String> = oplog
            .into_iter()
            .filter(|l| bad.iter().any(|b| l.contains(&format!(" {b} "))))
            .collect();
        ordered.sort();
        report.trace = ordered;
        let faults = wire.log.lock().unwrap().clone();
        report
            .trace
            .push("-- injected faults (time ids) --".to_string());
        report.trace.extend(faults.into_iter().take(40));
    }

    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
    report.print();
    report
}

fn seed() -> u64 {
    env_u64("HFS_CHAOS_SEED", 20291)
}
fn ops() -> usize {
    env_u64("HFS_CHAOS_OPS", 400) as usize
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The server binary's layout: versions come from a primary. Everything that
/// holds must keep holding with a fixed seed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chaos_campaign_primary_versions() {
    let r = campaign(seed(), ops(), Mode::Primary).await;
    assert!(r.i1.is_empty(), "I1: searches failed: {:?}", r.i1);
    assert!(r.i2.is_empty(), "I2: {:?}", r.i2);
    assert!(r.i3.is_empty(), "I3: {:?}", r.i3);
    assert!(r.i4.is_empty(), "I4: {:?}", r.i4);
    assert!(r.i5.is_empty(), "I5: {:?}", r.i5);
}

/// ES numbers its own versions. What holds regardless of who numbers them: the
/// searches never fail, nothing is visible under a missing container, and a
/// purge leaves nothing behind.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chaos_campaign_standalone_versions() {
    let r = campaign(seed(), ops(), Mode::Standalone).await;
    assert!(r.i1.is_empty(), "I1: searches failed: {:?}", r.i1);
    assert!(r.i2.is_empty(), "I2: {:?}", r.i2);
    assert!(r.i5.is_empty(), "I5: {:?}", r.i5);
}

/// The same campaign asserting I3 and I4 too: with ES-numbered versions a retry
/// after a rejected container reuses the version of the documents the failed
/// attempt wrote, which then show up as children of the container (see
/// `a_failed_standalone_update_leaves_a_child_that_a_retry_makes_visible`).
/// Fails on several seeds (for example `HFS_CHAOS_SEED=3 HFS_CHAOS_PM=60`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "bug: standalone ES reuses the version of a failed attempt, so its unlisted children become visible (I3/I4)"]
async fn chaos_campaign_standalone_versions_strict() {
    let r = campaign(env_u64("HFS_CHAOS_SEED", 3), ops(), Mode::Standalone).await;
    assert!(r.i3.is_empty(), "I3: {:?}", r.i3);
    assert!(r.i4.is_empty(), "I4: {:?}", r.i4);
}

/// Minimal reproducer. `create_or_update` makes v1 without children. An
/// `update` with child `o1` indexes the child with version 2, but the container
/// item of the `_bulk` is rejected (`429` item, then `429` for every resend)
/// and the write fails. The container is still v1 and lists nothing, so the
/// child is hidden. A second `update` back to no children numbers itself v2
/// again (`current.version + 1`): nothing is stale (the list is empty), so
/// nothing is deleted, and the orphan carries the container's new version.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "bug: a failed standalone update leaves an unlisted child at version N+1 that the retry (also N+1) makes visible; storage.rs commit_container / update (new_version = current + 1)"]
async fn a_failed_standalone_update_leaves_a_child_that_a_retry_makes_visible() {
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string(), 1, 0).await;
    let ctx = reproducer_ctx(&via, &prefix, wire.clone(), "repro-standalone").await;
    let (t, es) = (&ctx.t, &ctx.es);
    let kids =
        |names: &[&str]| -> BTreeSet<String> { names.iter().map(|s| s.to_string()).collect() };

    es.create_or_update(
        t,
        "Patient",
        "r",
        content(0, "a", &kids(&[])),
        FhirVersion::default(),
    )
    .await
    .expect("v1");
    let current = es.read(t, "Patient", "r").await.unwrap().unwrap();

    wire.chaos.last_only.store(true, Ordering::SeqCst);
    {
        let mut script = wire.chaos.script.lock().unwrap();
        script.push_back(Fault::Partial);
        for _ in 0..40 {
            script.push_back(Fault::Refuse429);
        }
    }
    let failed = es
        .update(t, &current, content(0, "b", &kids(&["o1"])))
        .await;
    wire.chaos.script.lock().unwrap().clear();
    assert!(
        failed.is_err(),
        "the container item was rejected every time"
    );
    assert!(
        observe(&ctx).await.vis.is_empty(),
        "precondition: the orphan child is hidden"
    );

    // The retry: back to no children.
    let current = es.read(t, "Patient", "r").await.unwrap().unwrap();
    es.update(t, &current, content(0, "c", &kids(&[])))
        .await
        .expect("retry");
    let seen = observe(&ctx).await.vis;
    assert!(
        seen.is_empty(),
        "the container has no children, but these are visible: {seen:?}"
    );
    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}

async fn reproducer_ctx(via: &str, prefix: &str, wire: Arc<Wire>, tenant_id: &str) -> Ctx {
    Ctx {
        es: backend_via(via, prefix).await,
        t: tenant(tenant_id),
        res: (0..KEYS)
            .map(|_| Resource {
                state: Mutex::new(ResState::default()),
                order: Arc::new(tokio::sync::Mutex::new(())),
            })
            .collect(),
        wire,
        t0: Instant::now(),
        oplog: Mutex::new(Vec::new()),
        first_errors: Mutex::new(Vec::new()),
    }
}

/// The same sequence with a primary's versions (strictly increasing, one per
/// attempt, as the server binary has them): the orphan of the failed v2 stays
/// hidden when the next write is v3.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_primary_write_leaves_an_orphan_that_stays_hidden() {
    let prefix = new_prefix();
    let (via, wire) = start_proxy(es_addr().await.to_string(), 1, 0).await;
    let ctx = reproducer_ctx(&via, &prefix, wire.clone(), "repro-primary").await;
    let (t, es) = (&ctx.t, &ctx.es);
    let kids =
        |names: &[&str]| -> BTreeSet<String> { names.iter().map(|s| s.to_string()).collect() };
    let item = |version: u64, tag: &str, names: &[&str]| Item {
        key: 0,
        version,
        tag: tag.to_string(),
        children: kids(names),
    };

    es.write_search_entries(t, &stored_of(&ctx, &item(1, "a", &[])))
        .await
        .expect("v1");
    wire.chaos.last_only.store(true, Ordering::SeqCst);
    {
        let mut script = wire.chaos.script.lock().unwrap();
        script.push_back(Fault::Partial);
        for _ in 0..40 {
            script.push_back(Fault::Refuse429);
        }
    }
    let failed = es
        .write_search_entries(t, &stored_of(&ctx, &item(2, "b", &["o1"])))
        .await;
    wire.chaos.script.lock().unwrap().clear();
    assert!(failed.is_err());
    es.write_search_entries(t, &stored_of(&ctx, &item(3, "c", &[])))
        .await
        .expect("v3");
    assert!(observe(&ctx).await.vis.is_empty());
    es.write_search_entries(t, &stored_of(&ctx, &item(4, "d", &["o1", "l0"])))
        .await
        .expect("v4");
    let seen = observe(&ctx).await.vis;
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(seen.iter().all(|v| v.tag == "d"), "{seen:?}");
    wire.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
}
