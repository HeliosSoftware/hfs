//! Two Elasticsearch instances (two HFS processes) writing one resource, with a
//! late resend of a throttled child (#2291).
//!
//! Every `ElasticsearchBackend` has its own in-process lock per container. Two
//! instances against one cluster (two replicas of the server) do not share it.
//! The hypothesis these tests check: writer A (version 2) sends a `_bulk` of
//! `[child, container]`; the child item is answered `429` while the container
//! item succeeds; A resends the child after a back-off. If writer B (version 3,
//! another instance) writes its child and container in between, A's late
//! resend overwrites B's child with version 2 under a container in version 3,
//! and the `_contained` read filter (child version == container version)
//! hides the current child until the resource is written again.
//!
//! A recording and rewriting HTTP proxy sits in front of the cluster for the
//! instance(s) under test. It can answer `429` for the child items of a
//! `_bulk` (forwarding the rest) and hold back the resend until released.
//!
//! Set `HFS_TEST_ES_URL` to use a running cluster; otherwise a container starts.
//!
//! Run with:
//! `cargo test -p helios-persistence --features R4,elasticsearch,sqlite --test elasticsearch_contained_chaos_multiprocess_real -- --nocapture --include-ignored`

#![cfg(all(feature = "elasticsearch", feature = "sqlite"))]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use elasticsearch::Elasticsearch;
use elasticsearch::http::transport::Transport;
use helios_fhir::FhirVersion;
use helios_persistence::StoredResource;
use helios_persistence::backends::elasticsearch::{
    ElasticsearchBackend, ElasticsearchConfig, WriteRefreshPolicy,
};
use helios_persistence::core::{Backend, SearchProvider};
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
                    .with_env_var("ES_JAVA_OPTS", "-Xms512m -Xmx512m")
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
// The rewriting proxy
// ---------------------------------------------------------------------------

/// What the proxy does, and what it saw.
#[derive(Default)]
struct Chaos {
    stop: AtomicBool,
    /// Answer `429` for the child items of the next `_bulk` that has any (once),
    /// forwarding the other items.
    reject_next_child: AtomicBool,
    /// Hold every `_bulk` made only of child items until `gate_open`.
    hold_child_only: AtomicBool,
    gate_open: AtomicBool,
    /// `_bulk` requests held at the gate right now or released from it.
    held: AtomicUsize,
    /// Child items answered `429` by the proxy.
    child_429: AtomicUsize,
    /// `_bulk` requests seen.
    bulks: AtomicUsize,
    /// Random mode: per-mille chance that a child item is answered `429`.
    p429_permille: AtomicU64,
    /// Random mode: every `_bulk` is delayed by a random 0..=max_delay_ms.
    max_delay_ms: AtomicU64,
    rng: Mutex<u64>,
}

impl Chaos {
    fn next_rand(&self) -> u64 {
        let mut s = self.rng.lock().unwrap();
        if *s == 0 {
            *s = 0x9E37_79B9_7F4A_7C15 ^ (Instant::now().elapsed().as_nanos() as u64 | 1);
        }
        // xorshift64*
        *s ^= *s >> 12;
        *s ^= *s << 25;
        *s ^= *s >> 27;
        s.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn child_429(&self) -> usize {
        self.child_429.load(Ordering::SeqCst)
    }
    fn held(&self) -> usize {
        self.held.load(Ordering::SeqCst)
    }
}

struct Req {
    method: String,
    head_line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn find(buf: &[u8], pat: &[u8]) -> Option<usize> {
    buf.windows(pat.len()).position(|w| w == pat)
}

fn decode_chunked(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(p) = find(data, b"\r\n") {
        let size = usize::from_str_radix(
            std::str::from_utf8(&data[..p])
                .unwrap_or("0")
                .split(';')
                .next()
                .unwrap_or("0")
                .trim(),
            16,
        )
        .unwrap_or(0);
        data = &data[p + 2..];
        if size == 0 || data.len() < size {
            break;
        }
        out.extend_from_slice(&data[..size]);
        data = &data[(size + 2).min(data.len())..];
    }
    out
}

/// Reads one full request from the client; `None` on EOF.
async fn read_request(
    cr: &mut tokio::net::tcp::OwnedReadHalf,
    pending: &mut Vec<u8>,
) -> Option<Req> {
    let mut tmp = vec![0u8; 64 * 1024];
    let head_end = loop {
        if let Some(p) = find(pending, b"\r\n\r\n") {
            break p;
        }
        let n = cr.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        pending.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&pending[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let head_line = lines.next()?.to_string();
    let method = head_line.split(' ').next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let header = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    };
    let body_start = head_end + 4;
    let chunked =
        header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let body = if chunked {
        loop {
            if find(&pending[body_start..], b"0\r\n\r\n").is_some() {
                break;
            }
            let n = cr.read(&mut tmp).await.ok()?;
            if n == 0 {
                return None;
            }
            pending.extend_from_slice(&tmp[..n]);
        }
        let end = body_start + find(&pending[body_start..], b"0\r\n\r\n").unwrap() + 5;
        let decoded = decode_chunked(&pending[body_start..end]);
        pending.drain(..end);
        decoded
    } else {
        let len: usize = header("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        while pending.len() < body_start + len {
            let n = cr.read(&mut tmp).await.ok()?;
            if n == 0 {
                return None;
            }
            pending.extend_from_slice(&tmp[..n]);
        }
        let body = pending[body_start..body_start + len].to_vec();
        pending.drain(..body_start + len);
        body
    };
    Some(Req {
        method,
        head_line,
        headers,
        body,
    })
}

/// Sends `req` to the cluster on a fresh connection; returns (status line,
/// headers, body) of the answer.
async fn forward(
    upstream: &str,
    req: &Req,
    body: &[u8],
) -> Option<(String, Vec<(String, String)>, Vec<u8>)> {
    let mut s = TcpStream::connect(upstream).await.ok()?;
    let mut head = format!("{}\r\n", req.head_line);
    for (k, v) in &req.headers {
        let lk = k.to_ascii_lowercase();
        if matches!(
            lk.as_str(),
            "content-length" | "transfer-encoding" | "connection" | "accept-encoding" | "expect"
        ) {
            continue;
        }
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str(&format!(
        "content-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    ));
    s.write_all(head.as_bytes()).await.ok()?;
    s.write_all(body).await.ok()?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.ok()?;
    let p = find(&raw, b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..p]).to_string();
    let mut lines = head.split("\r\n");
    let status = lines.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let chunked = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked")
    });
    let rest = &raw[p + 4..];
    let body = if chunked {
        decode_chunked(rest)
    } else {
        rest.to_vec()
    };
    Some((status, headers, body))
}

/// A `_bulk` body split into items: (action line, optional document line).
fn parse_bulk(body: &[u8]) -> Vec<(Vec<u8>, Option<Vec<u8>>)> {
    let mut lines = body.split(|b| *b == b'\n').filter(|l| !l.is_empty());
    let mut items = Vec::new();
    while let Some(action) = lines.next() {
        let is_delete = find(action, b"{\"delete\"").is_some_and(|p| p < 3);
        let doc = if is_delete {
            None
        } else {
            lines.next().map(<[u8]>::to_vec)
        };
        items.push((action.to_vec(), doc));
    }
    items
}

fn is_child_index(action: &[u8]) -> bool {
    let v: Value = serde_json::from_slice(action).unwrap_or(Value::Null);
    v.get("index")
        .and_then(|i| i.get("_id"))
        .and_then(Value::as_str)
        .is_some_and(|id| id.contains('#'))
}

async fn start_proxy(upstream: String) -> (String, Arc<Chaos>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind proxy");
    let addr = listener.local_addr().expect("proxy addr").to_string();
    let chaos = Arc::new(Chaos::default());
    let accept = chaos.clone();
    tokio::spawn(async move {
        while !accept.stop.load(Ordering::SeqCst) {
            let Ok((client, _)) = listener.accept().await else {
                break;
            };
            let upstream = upstream.clone();
            let chaos = accept.clone();
            tokio::spawn(async move {
                let (mut cr, mut cw) = client.into_split();
                let mut pending = Vec::new();
                while let Some(req) = read_request(&mut cr, &mut pending).await {
                    let is_bulk = req.head_line.contains("_bulk");
                    let mut send_body = req.body.clone();
                    let mut items: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
                    let mut rejected: Vec<bool> = Vec::new();
                    if is_bulk {
                        chaos.bulks.fetch_add(1, Ordering::SeqCst);
                        items = parse_bulk(&req.body);
                        let has_child = items.iter().any(|(a, _)| is_child_index(a));
                        let only_children =
                            has_child && items.iter().all(|(a, _)| is_child_index(a));
                        // Random delay first: the interleaving window.
                        let max_delay = chaos.max_delay_ms.load(Ordering::SeqCst);
                        if max_delay > 0 {
                            let d = chaos.next_rand() % (max_delay + 1);
                            tokio::time::sleep(Duration::from_millis(d)).await;
                        }
                        // Which child items get a 429.
                        let p = chaos.p429_permille.load(Ordering::SeqCst);
                        rejected = items
                            .iter()
                            .map(|(a, _)| {
                                is_child_index(a) && p > 0 && chaos.next_rand() % 1000 < p
                            })
                            .collect();
                        if has_child
                            && chaos
                                .reject_next_child
                                .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
                                .is_ok()
                        {
                            rejected = items.iter().map(|(a, _)| is_child_index(a)).collect();
                            // From now on hold the resend of the rejected child.
                            chaos.hold_child_only.store(true, Ordering::SeqCst);
                        } else if only_children && chaos.hold_child_only.load(Ordering::SeqCst) {
                            chaos.held.fetch_add(1, Ordering::SeqCst);
                            while !chaos.gate_open.load(Ordering::SeqCst) {
                                tokio::time::sleep(Duration::from_millis(5)).await;
                            }
                        }
                        let n_rejected = rejected.iter().filter(|r| **r).count();
                        chaos.child_429.fetch_add(n_rejected, Ordering::SeqCst);
                        if n_rejected > 0 {
                            send_body.clear();
                            for ((a, d), rej) in items.iter().zip(&rejected) {
                                if *rej {
                                    continue;
                                }
                                send_body.extend_from_slice(a);
                                send_body.push(b'\n');
                                if let Some(d) = d {
                                    send_body.extend_from_slice(d);
                                    send_body.push(b'\n');
                                }
                            }
                        }
                    }
                    let all_rejected =
                        is_bulk && !rejected.is_empty() && rejected.iter().all(|r| *r);
                    let answer = if all_rejected {
                        Some((
                            "HTTP/1.1 200 OK".to_string(),
                            vec![],
                            serde_json::to_vec(&json!({"took": 1, "errors": true, "items": []}))
                                .unwrap(),
                        ))
                    } else {
                        forward(&upstream, &req, &send_body).await
                    };
                    let Some((status, headers, mut body)) = answer else {
                        break;
                    };
                    // Merge the real answers with fabricated 429 items, in order.
                    if is_bulk && rejected.iter().any(|r| *r) && status.contains(" 200") {
                        let real: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                        let mut real_items = real["items"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                            .into_iter();
                        let merged: Vec<Value> = items
                            .iter()
                            .zip(&rejected)
                            .map(|((a, _), rej)| {
                                if *rej {
                                    let act: Value = serde_json::from_slice(a).unwrap();
                                    let meta = &act["index"];
                                    json!({"index": {
                                        "_index": meta["_index"], "_id": meta["_id"],
                                        "status": 429,
                                        "error": {"type": "es_rejected_execution_exception",
                                                  "reason": "rejected execution (chaos proxy)"}
                                    }})
                                } else {
                                    real_items.next().unwrap_or(Value::Null)
                                }
                            })
                            .collect();
                        body = serde_json::to_vec(&json!({
                            "took": 1, "errors": true, "items": merged
                        }))
                        .unwrap();
                    }
                    let mut out = format!("{status}\r\n");
                    for (k, v) in &headers {
                        let lk = k.to_ascii_lowercase();
                        if matches!(
                            lk.as_str(),
                            "connection" | "transfer-encoding" | "content-length"
                        ) {
                            continue;
                        }
                        out.push_str(&format!("{k}: {v}\r\n"));
                    }
                    if req.method == "HEAD" {
                        // No body; keep the cluster's own length for the client.
                        let len = headers
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .map(|(_, v)| v.clone())
                            .unwrap_or_else(|| "0".to_string());
                        out.push_str(&format!("content-length: {len}\r\n\r\n"));
                        if cw.write_all(out.as_bytes()).await.is_err() {
                            break;
                        }
                    } else {
                        out.push_str(&format!("content-length: {}\r\n\r\n", body.len()));
                        if cw.write_all(out.as_bytes()).await.is_err()
                            || cw.write_all(&body).await.is_err()
                        {
                            break;
                        }
                    }
                }
            });
        }
    });
    (addr, chaos)
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
    let client =
        Elasticsearch::new(Transport::single_node(&format!("http://{}", es_addr().await)).unwrap());
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

/// Patient `id` in version `v` with `n` contained Organizations `org0..`, all
/// named `name` and carrying the version in `alias`.
fn versioned_patient(id: &str, name: &str, v: u32, n: usize) -> Value {
    let mut p = json!({
        "resourceType": "Patient", "id": id,
        "name": [{ "family": format!("Fam{v}") }],
    });
    if n > 0 {
        p["contained"] = Value::Array(
            (0..n)
                .map(|j| {
                    json!({ "resourceType": "Organization", "id": format!("org{j}"),
                            "name": name, "alias": [format!("v{v}")] })
                })
                .collect(),
        );
        p["managingOrganization"] = json!({ "reference": "#org0" });
    }
    p
}

fn stored(t: &TenantContext, id: &str, name: &str, v: u32, n: usize) -> StoredResource {
    let now = chrono::Utc::now();
    StoredResource::from_storage(
        "Patient".to_string(),
        id.to_string(),
        v.to_string(),
        t.tenant_id().clone(),
        versioned_patient(id, name, v, n),
        now,
        now,
        None,
        FhirVersion::default(),
    )
}

async fn write(
    es: &ElasticsearchBackend,
    t: &TenantContext,
    id: &str,
    name: &str,
    v: u32,
    n: usize,
) -> Result<(), String> {
    es.write_search_entries(t, &stored(t, id, name, v, n))
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The contained Organizations named `name` that a `_contained` search returns:
/// `(local id, version tag in alias)`, sorted.
async fn visible_children(
    es: &ElasticsearchBackend,
    t: &TenantContext,
    name: &str,
) -> Vec<(String, String)> {
    let mut q = SearchQuery::new("Organization");
    q.contained = ContainedMode::On;
    q.contained_return = ContainedReturn::Contained;
    q.parameters.push(SearchParameter {
        name: "name".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq(name)],
        chain: vec![],
        components: vec![],
    });
    let mut items: Vec<(String, String)> = es
        .search(t, &q)
        .await
        .expect("contained search")
        .resources
        .items
        .iter()
        .map(|r| {
            (
                r.id().to_string(),
                r.content()["alias"][0].as_str().unwrap_or("?").to_string(),
            )
        })
        .collect();
    items.sort();
    items
}

/// The container's `(version_id, contained_doc_ids)` read straight from the
/// cluster.
async fn container_state(prefix: &str, id: &str) -> Option<(String, Vec<String>)> {
    let client =
        Elasticsearch::new(Transport::single_node(&format!("http://{}", es_addr().await)).unwrap());
    let pattern = format!("{prefix}_*");
    let body = client
        .search(elasticsearch::SearchParts::Index(&[&pattern]))
        .body(json!({ "query": { "bool": {
            "filter": [
                { "term": { "resource_type": "Patient" } },
                { "term": { "resource_id": id } },
            ],
            "must_not": [ { "term": { "is_contained": true } } ],
        } } }))
        .send()
        .await
        .ok()?
        .json::<Value>()
        .await
        .ok()?;
    let src = body["hits"]["hits"][0]["_source"].clone();
    let version = src["version_id"].as_str()?.to_string();
    let ids = src["contained_doc_ids"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Some((version, ids))
}

async fn refresh_all(prefix: &str) {
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
}

async fn wait_for(what: &str, limit: Duration, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < limit, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

// ---------------------------------------------------------------------------
// 1 and 3: the deterministic late resend
// ---------------------------------------------------------------------------

/// What the deterministic scenario observed after the late resend.
struct LateResend {
    visible_v3: Vec<(String, String)>,
    visible_v2: Vec<(String, String)>,
    container_version: String,
    a_result: Result<(), String>,
    b_result: Result<(), String>,
    /// How long the state was still wrong when polled after the resend.
    still_wrong_for: Duration,
}

/// v1 is written, then writer A (v2, through a proxy that answers the child
/// `429` and holds the resend) and writer B (v3) race. `same_instance` makes
/// B the same backend as A (the in-process lock must serialize them); otherwise
/// B is another instance straight on the cluster.
async fn late_resend_scenario(same_instance: bool) -> LateResend {
    let prefix = new_prefix();
    let (via_a, chaos) = start_proxy(es_addr().await.to_string()).await;
    let a = Arc::new(backend_via(&via_a, &prefix).await);
    let b = if same_instance {
        a.clone()
    } else {
        Arc::new(backend_via(es_addr().await, &prefix).await)
    };
    let t = tenant("chaos-late");
    let name = "Late";

    write(&b, &t, "p", name, 1, 1).await.expect("v1");
    assert_eq!(
        visible_children(&b, &t, name).await,
        vec![("org0".into(), "v1".into())]
    );

    chaos.reject_next_child.store(true, Ordering::SeqCst);
    let a_task = {
        let a = a.clone();
        let t = t.clone();
        tokio::spawn(async move { write(&a, &t, "p", name, 2, 1).await })
    };
    // The resend of A's throttled child arrives and is held.
    wait_for("A's resend to be held", Duration::from_secs(20), || {
        chaos.held() >= 1
    })
    .await;
    assert_eq!(chaos.child_429(), 1, "the proxy throttled A's child once");

    // B writes v3 while A's child is still on hold.
    let b_task = {
        let b = b.clone();
        let t = t.clone();
        tokio::spawn(async move { write(&b, &t, "p", name, 3, 1).await })
    };
    let b_result = if same_instance {
        // The in-process lock keeps B waiting for A: B cannot finish before the
        // gate opens. Give it time to prove that, then release A.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(
            !b_task.is_finished(),
            "same instance: B waits on A's container lock"
        );
        chaos.gate_open.store(true, Ordering::SeqCst);
        b_task.await.expect("b task")
    } else {
        let r = b_task.await.expect("b task");
        // B is done: now release A's late resend.
        chaos.gate_open.store(true, Ordering::SeqCst);
        r
    };
    let a_result = a_task.await.expect("a task");
    let released = Instant::now();

    refresh_all(&prefix).await;
    let (container_version, ids) = container_state(&prefix, "p").await.expect("container");
    eprintln!("container version {container_version}, contained_doc_ids {ids:?}");
    let visible_v3 = visible_children(&b, &t, name).await;
    let mut still_wrong_for = Duration::ZERO;
    if visible_v3.is_empty() {
        // Poll: does it heal on its own?
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if !visible_children(&b, &t, name).await.is_empty() {
                break;
            }
            still_wrong_for = released.elapsed();
        }
    }
    let visible_v2 = visible_children(&a, &t, name).await;
    eprintln!(
        "same_instance={same_instance} a={a_result:?} b={b_result:?} visible={visible_v3:?} \
         wrong_for>={still_wrong_for:?}"
    );

    if !same_instance && visible_v3.is_empty() {
        // A single further write of the resource heals it.
        write(&b, &t, "p", name, 4, 1).await.expect("v4");
        assert_eq!(
            visible_children(&b, &t, name).await,
            vec![("org0".into(), "v4".into())],
            "one more write heals the resource"
        );
    }

    chaos.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;
    LateResend {
        visible_v3,
        visible_v2,
        container_version,
        a_result,
        b_result,
        still_wrong_for,
    }
}

/// Two instances: A's throttled child of v2 is resent after B wrote v3.
#[ignore = "known gap: multi-process late resend hides a current child"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_late_resend_from_another_instance_must_not_hide_the_current_child() {
    let r = late_resend_scenario(false).await;
    assert!(
        r.a_result.is_ok() && r.b_result.is_ok(),
        "both writes succeed: {:?} {:?}",
        r.a_result,
        r.b_result
    );
    assert_eq!(r.container_version, "3", "the container is B's v3");
    assert_eq!(
        r.visible_v3,
        vec![("org0".to_string(), "v3".to_string())],
        "the v3 child is visible (still hidden after {:?}); v2 search shows {:?}",
        r.still_wrong_for,
        r.visible_v2
    );
}

/// Control: one instance, two concurrent writers. The container lock makes B
/// wait for A's whole write, resend included, so v3 wins and is visible.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn control_one_instance_two_writers_keeps_the_current_child_visible() {
    let r = late_resend_scenario(true).await;
    assert!(
        r.a_result.is_ok() && r.b_result.is_ok(),
        "both writes succeed: {:?} {:?}",
        r.a_result,
        r.b_result
    );
    assert_eq!(r.container_version, "3");
    assert_eq!(r.visible_v3, vec![("org0".to_string(), "v3".to_string())]);
}

// ---------------------------------------------------------------------------
// 2: statistical, two instances with random 429s and delays
// ---------------------------------------------------------------------------

#[ignore = "known gap: multi-process late resend hides a current child"]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn statistical_two_instances_random_429_leave_no_hidden_current_child() {
    const N: usize = 200;
    const VERSIONS: u32 = 4;
    let prefix = new_prefix();
    let (via_a, chaos_a) = start_proxy(es_addr().await.to_string()).await;
    let (via_b, chaos_b) = start_proxy(es_addr().await.to_string()).await;
    for c in [&chaos_a, &chaos_b] {
        c.p429_permille.store(350, Ordering::SeqCst);
        c.max_delay_ms.store(250, Ordering::SeqCst);
    }
    let a = Arc::new(backend_via(&via_a, &prefix).await);
    let b = Arc::new(backend_via(&via_b, &prefix).await);
    // Reads and the healing write go straight to the cluster.
    let reader = Arc::new(backend_via(es_addr().await, &prefix).await);
    let t = tenant("chaos-stat");

    let gate = Arc::new(tokio::sync::Semaphore::new(24));
    let write_errors = Arc::new(AtomicUsize::new(0));
    let errored: Arc<Mutex<std::collections::HashSet<usize>>> = Default::default();
    let started = Instant::now();
    let tasks: Vec<_> = (0..N)
        .map(|i| {
            let (a, b) = (a.clone(), b.clone());
            let t = t.clone();
            let gate = gate.clone();
            let write_errors = write_errors.clone();
            let errored = errored.clone();
            let chaos = chaos_a.clone();
            tokio::spawn(async move {
                let _permit = gate.acquire().await.unwrap();
                let id = format!("r{i}");
                let name = format!("Res{i:04}z");
                let mut writes = Vec::new();
                for v in 1..=VERSIONS {
                    // Version v goes to a random instance, 0..=2 children, after
                    // a random head start so that writes overlap or not.
                    let on_a = chaos.next_rand() % 2 == 0;
                    let n = (chaos.next_rand() % 3) as usize;
                    let jitter = chaos.next_rand() % 400;
                    let es = if on_a { a.clone() } else { b.clone() };
                    let (t, id, name) = (t.clone(), id.clone(), name.clone());
                    writes.push(tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(jitter)).await;
                        write(&es, &t, &id, &name, v, n).await
                    }));
                }
                for w in writes {
                    if w.await.unwrap().is_err() {
                        write_errors.fetch_add(1, Ordering::SeqCst);
                        errored.lock().unwrap().insert(i);
                    }
                }
            })
        })
        .collect();
    for task in tasks {
        task.await.unwrap();
    }
    eprintln!(
        "writes done in {:?}: bulks A={} B={}, child 429s A={} B={}, failed writes={}",
        started.elapsed(),
        chaos_a.bulks.load(Ordering::SeqCst),
        chaos_b.bulks.load(Ordering::SeqCst),
        chaos_a.child_429(),
        chaos_b.child_429(),
        write_errors.load(Ordering::SeqCst)
    );

    // Quiescence: the writers are done and no resend is pending. No healing write.
    refresh_all(&prefix).await;
    let check = |label: &'static str| {
        let reader = reader.clone();
        let t = t.clone();
        let prefix = prefix.clone();
        let errored = errored.clone();
        async move {
            let mut hidden = Vec::new();
            let mut stale_visible = Vec::new();
            let mut ok = 0usize;
            for i in 0..N {
                let id = format!("r{i}");
                let Some((version, ids)) = container_state(&prefix, &id).await else {
                    continue;
                };
                let seen = visible_children(&reader, &t, &format!("Res{i:04}z")).await;
                let expected_children = ids.len();
                let current: Vec<_> = seen
                    .iter()
                    .filter(|(_, v)| *v == format!("v{version}"))
                    .collect();
                if seen.iter().any(|(_, v)| *v != format!("v{version}")) {
                    stale_visible.push(i);
                }
                if current.len() < expected_children {
                    hidden.push((i, version, expected_children, current.len()));
                } else {
                    ok += 1;
                }
            }
            let with_errors = hidden
                .iter()
                .filter(|(i, ..)| errored.lock().unwrap().contains(i))
                .count();
            eprintln!(
                "[{label}] of {N} resources: consistent={ok}, current child HIDDEN={} \
                 (of those, with a failed write on the resource: {with_errors}), \
                 stale child VISIBLE={}",
                hidden.len(),
                stale_visible.len()
            );
            for h in hidden.iter().take(5) {
                eprintln!(
                    "  hidden sample (resource, container version, children expected, visible): {h:?}"
                );
            }
            (hidden.len(), stale_visible.len())
        }
    };
    let (hidden, stale_visible) = check("quiescence").await;
    // Still the same a few seconds later? (Nothing heals by itself.)
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (hidden_later, _) = check("3 s later").await;
    eprintln!("hidden persists after 3 s: {hidden_later} of {hidden}");

    // One final write per resource, straight on the cluster, heals everything.
    for i in 0..N {
        let n = i % 3;
        write(
            &reader,
            &t,
            &format!("r{i}"),
            &format!("Res{i:04}z"),
            VERSIONS + 1,
            n,
        )
        .await
        .expect("healing write");
    }
    refresh_all(&prefix).await;
    let mut still_broken = 0;
    for i in 0..N {
        let n = i % 3;
        let seen = visible_children(&reader, &t, &format!("Res{i:04}z")).await;
        let expected: Vec<(String, String)> = (0..n)
            .map(|j| (format!("org{j}"), format!("v{}", VERSIONS + 1)))
            .collect();
        if seen != expected {
            still_broken += 1;
            eprintln!("not healed: r{i} saw {seen:?} expected {expected:?}");
        }
    }
    assert_eq!(still_broken, 0, "a final write heals every resource");

    chaos_a.stop.store(true, Ordering::SeqCst);
    chaos_b.stop.store(true, Ordering::SeqCst);
    cleanup(&prefix).await;

    assert_eq!(stale_visible, 0, "a stale child is never visible");
    assert_eq!(
        hidden, 0,
        "{hidden} of {N} resources keep a current child hidden after quiescence"
    );
}
