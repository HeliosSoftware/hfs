//! A whole-request `_bulk` timeout in `SyncManager::sync_creates` (#1749).
//!
//! `write_search_entries_page` on Elasticsearch reports a timed-out `_bulk`
//! as one error per resource, and `sync_creates` retries them individually
//! through `sync_event_to_backend` (`max_retries + 1` attempts each, with backoff).
//! Retrying every one serially against a cluster that does not answer made a
//! batch of N block for N x (max_retries + 1) request timeouts; it now stops
//! after the first resource whose own retry fails because the backend is
//! unavailable. No Docker: the cluster is a `wiremock` stub.
//!
//! Run with:
//! `cargo test -p helios-persistence --features elasticsearch \
//!   --test elasticsearch_sync_creates_timeout_wiremock`

#![cfg(feature = "elasticsearch")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use helios_fhir::FhirVersion;
use helios_persistence::backends::elasticsearch::{ElasticsearchBackend, ElasticsearchConfig};
use helios_persistence::composite::{SyncManager, config::SyncConfig, config::SyncMode};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::TenantId;
use helios_persistence::types::StoredResource;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const TIMEOUT_MS: u64 = 200;
const DELAY: Duration = Duration::from_secs(2);

struct Measured {
    elapsed: Duration,
    bulk: usize,
    single: usize,
    failed: usize,
}

async fn run(n: usize, singles_hang: bool) -> Measured {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/_bulk"))
        .respond_with(|_: &Request| ResponseTemplate::new(200).set_delay(DELAY))
        .mount(&server)
        .await;
    // The lookup of what the containers already record: nothing is indexed
    // yet, for the page's `_mget` and for each single write's GET.
    Mock::given(method("POST"))
        .and(path("/_mget"))
        .respond_with(|request: &Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&request.body).unwrap_or(serde_json::Value::Null);
            let docs: Vec<serde_json::Value> = body["docs"]
                .as_array()
                .map(|docs| {
                    docs.iter()
                        .map(|d| json!({"_index": d["_index"], "_id": d["_id"], "found": false}))
                        .collect()
                })
                .unwrap_or_default();
            ResponseTemplate::new(200).set_body_json(json!({ "docs": docs }))
        })
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(|_: &Request| {
            ResponseTemplate::new(404).set_body_json(json!({"found": false}))
        })
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/hfs_t_*/_delete_by_query"))
        .respond_with(|_: &Request| {
            ResponseTemplate::new(200).set_body_json(json!({
                "took": 1, "timed_out": false, "total": 0, "deleted": 0,
                "version_conflicts": 0, "failures": []
            }))
        })
        .mount(&server)
        .await;
    // Single-document index: POST /{index}/_doc/{id} (after the `_bulk` mock).
    Mock::given(method("POST"))
        .respond_with(move |_: &Request| {
            let ok = ResponseTemplate::new(201).set_body_json(json!({"result": "created"}));
            if singles_hang {
                ok.set_delay(DELAY)
            } else {
                ok
            }
        })
        .mount(&server)
        .await;

    let backend = ElasticsearchBackend::new(ElasticsearchConfig {
        nodes: vec![server.uri()],
        request_timeout_ms: TIMEOUT_MS,
        ..Default::default()
    })
    .expect("client construction is lazy");
    let mut backends: HashMap<String, Arc<dyn ResourceStorage + Send + Sync>> = HashMap::new();
    backends.insert("es".to_string(), Arc::new(backend));

    let manager = SyncManager::new(SyncConfig {
        mode: SyncMode::Synchronous,
        ..Default::default()
    });
    let tenant = TenantId::new("t");
    let resources: Vec<StoredResource> = (0..n)
        .map(|i| {
            let id = format!("p{i}");
            StoredResource::new(
                "Patient",
                id.clone(),
                tenant.clone(),
                json!({"resourceType": "Patient", "id": id, "name": [{"family": "X"}]}),
                FhirVersion::default(),
            )
        })
        .collect();

    let start = Instant::now();
    let statuses = manager
        .sync_creates(&tenant, "Patient", resources, &backends)
        .await
        .expect("sync_creates");
    let elapsed = start.elapsed();

    let requests = server.received_requests().await.expect("recording on");
    let bulk = requests.iter().filter(|r| r.url.path() == "/_bulk").count();
    let single = requests
        .iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path().contains("/_doc/"))
        .count();
    Measured {
        elapsed,
        bulk,
        single,
        failed: statuses.iter().map(|s| s.failed_resource_ids.len()).sum(),
    }
}

/// When the cluster does not answer, only the first resource is retried on its
/// own; the rest of the batch is recorded as failed without further requests,
/// so the time no longer grows with the size of the batch.
#[tokio::test]
async fn bulk_timeout_stops_retrying_one_by_one_once_the_cluster_is_unavailable() {
    for n in [5usize, 10, 20] {
        let m = run(n, true).await;
        // One resource pays its `max_retries + 1` attempts; no other is retried.
        assert_eq!(m.single, 4, "N={n}: single POSTs");
        assert_eq!(m.failed, n, "N={n}: every resource is reported failed");
        // The `_bulk` halving plus one resource's attempts and backoff, and
        // nowhere near N x (4 timeouts + backoff).
        let one_by_one = Duration::from_millis(n as u64 * (4 * TIMEOUT_MS + 700));
        assert!(
            m.elapsed < one_by_one / 2 + Duration::from_secs(3),
            "N={n}: took {:?}",
            m.elapsed
        );
        assert!(m.bulk >= 3, "N={n}: {} _bulk requests", m.bulk);
    }
}

/// A slow `_bulk` that the single-document path survives is still retried
/// resource by resource: the rest of the batch is not given up on.
#[tokio::test]
async fn bulk_timeout_still_retries_each_resource_when_singles_succeed() {
    for n in [5usize, 10] {
        let m = run(n, false).await;
        assert_eq!(m.single, n, "N={n}: one single POST each");
        assert_eq!(m.failed, 0, "N={n}");
    }
}
