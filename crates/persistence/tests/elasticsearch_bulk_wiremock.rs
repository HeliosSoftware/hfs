//! `_bulk` failure paths of the Elasticsearch writer, against an in-process
//! HTTP stub (#1125).
//!
//! A real cluster cannot be made to answer `413`, `429` or a slow response on
//! demand, and those are exactly the answers that decide whether a rebuild
//! indexes a resource, retries it, or reports it. These tests need **no
//! Docker**: the backend is pointed at a `wiremock` server that plays the
//! cluster, and each test asserts both the outcome per resource and the
//! requests the writer actually sent.
//!
//! Run with:
//! `cargo test -p helios-persistence --features elasticsearch --test elasticsearch_bulk_wiremock`

#![cfg(feature = "elasticsearch")]

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use helios_fhir::FhirVersion;
use helios_persistence::backends::elasticsearch::{
    ElasticsearchBackend, ElasticsearchConfig, WriteRefreshPolicy,
};
use helios_persistence::error::{BackendError, StorageError, StorageResult};
use helios_persistence::search::ReindexTarget;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::StoredResource;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// Attempts the writer gives a throttled operation (mirrors the private
/// `BULK_MAX_ATTEMPTS`).
const BULK_MAX_ATTEMPTS: usize = 5;

fn tenant() -> TenantContext {
    TenantContext::new(TenantId::new("bulk-stub"), TenantPermissions::full_access())
}

fn patient(id: &str, padding: usize) -> StoredResource {
    StoredResource::from_storage(
        "Patient",
        id,
        "1",
        TenantId::new("bulk-stub"),
        json!({
            "resourceType": "Patient",
            "id": id,
            "name": [{"family": format!("Family{}", "x".repeat(padding))}]
        }),
        chrono::Utc::now(),
        chrono::Utc::now(),
        None,
        FhirVersion::default(),
    )
}

fn page(ids: &[&str]) -> Vec<StoredResource> {
    ids.iter().map(|id| patient(id, 0)).collect()
}

fn backend_with(
    server: &MockServer,
    configure: impl FnOnce(&mut ElasticsearchConfig),
) -> ElasticsearchBackend {
    let mut config = ElasticsearchConfig {
        nodes: vec![server.uri()],
        request_timeout_ms: 2_000,
        ..Default::default()
    };
    configure(&mut config);
    ElasticsearchBackend::new(config).expect("client construction is lazy")
}

/// A stub cluster on which every index already exists.
async fn cluster() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

/// Answers `_bulk` requests with `respond`.
async fn on_bulk(
    server: &MockServer,
    respond: impl Fn(&Request) -> ResponseTemplate + Send + Sync + 'static,
) {
    Mock::given(method("POST"))
        .and(path("/_bulk"))
        .respond_with(respond)
        .mount(server)
        .await;
}

/// The document ids a `_bulk` body carries, in order.
fn bulk_ids(request: &Request) -> Vec<String> {
    String::from_utf8_lossy(&request.body)
        .lines()
        .step_by(2)
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|action| action["index"]["_id"].as_str().map(str::to_string))
        .collect()
}

/// A `200` whose items report every one of `n` operations indexed.
fn indexed(n: usize) -> ResponseTemplate {
    let items: Vec<Value> = (0..n)
        .map(|_| json!({"index": {"status": 201, "result": "created"}}))
        .collect();
    ResponseTemplate::new(200).set_body_json(json!({"took": 1, "errors": false, "items": items}))
}

async fn bulk_requests(server: &MockServer) -> Vec<Request> {
    server
        .received_requests()
        .await
        .expect("request recording is on")
        .into_iter()
        .filter(|request| request.method.as_str() == "POST" && request.url.path() == "/_bulk")
        .collect()
}

fn is_transient(outcome: &StorageResult<usize>) -> bool {
    matches!(
        outcome,
        Err(StorageError::Backend(
            BackendError::Unavailable { .. }
                | BackendError::ConnectionFailed { .. }
                | BackendError::Timeout { .. }
                | BackendError::PoolExhausted { .. }
        ))
    )
}

fn is_permanent(outcome: &StorageResult<usize>) -> bool {
    matches!(
        outcome,
        Err(StorageError::Backend(BackendError::Internal { .. }))
    )
}

fn assert_all_ok(outcomes: &[StorageResult<usize>]) {
    for (n, outcome) in outcomes.iter().enumerate() {
        assert!(outcome.is_ok(), "resource {n} failed: {outcome:?}");
    }
}

// ============================================================================
// 413 and timeouts: the request was too large, not its documents wrong
// ============================================================================

#[tokio::test]
async fn a_request_answered_413_is_split_down_to_single_documents() {
    let server = cluster().await;
    on_bulk(&server, |request| {
        let ops = bulk_ids(request).len();
        if ops > 1 {
            ResponseTemplate::new(413).set_body_string("Request Entity Too Large")
        } else {
            indexed(ops)
        }
    })
    .await;
    let backend = backend_with(&server, |_| {});
    let resources = page(&["a", "b", "c", "d", "e", "f", "g", "h"]);

    let outcomes = backend
        .write_search_entries_page(&tenant(), &resources)
        .await;

    assert_eq!(outcomes.len(), 8);
    assert_all_ok(&outcomes);
    let requests = bulk_requests(&server).await;
    // 8 -> 4 + 4 -> 2 * 4 -> 1 * 8: every halving is resent, nothing twice.
    assert_eq!(requests.len(), 1 + 2 + 4 + 8);
    let singles: HashSet<String> = requests
        .iter()
        .map(bulk_ids)
        .filter(|ids| ids.len() == 1)
        .flatten()
        .collect();
    assert_eq!(singles.len(), 8, "each document ends up sent on its own");
}

#[tokio::test]
async fn a_single_document_answered_413_is_rejected_permanently() {
    let server = cluster().await;
    on_bulk(&server, |_| {
        ResponseTemplate::new(413).set_body_string("Request Entity Too Large")
    })
    .await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b"]))
        .await;

    for outcome in &outcomes {
        assert!(is_permanent(outcome), "{outcome:?}");
        let message = outcome.as_ref().unwrap_err().to_string();
        assert!(message.contains("413"), "{message}");
    }
    // One pair, then each document alone — never the pair twice.
    assert_eq!(bulk_requests(&server).await.len(), 3);
}

#[tokio::test]
async fn a_timed_out_request_is_split_and_its_halves_succeed() {
    let server = cluster().await;
    on_bulk(&server, |request| {
        let ops = bulk_ids(request).len();
        if ops > 1 {
            indexed(ops).set_delay(Duration::from_secs(3))
        } else {
            indexed(ops)
        }
    })
    .await;
    let backend = backend_with(&server, |config| config.request_timeout_ms = 750);

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b", "c", "d"]))
        .await;

    assert_all_ok(&outcomes);
    // 4 (timed out) -> 2 + 2 (timed out) -> 4 singles.
    assert_eq!(bulk_requests(&server).await.len(), 1 + 2 + 4);
}

#[tokio::test]
async fn a_single_document_that_times_out_is_a_transient_failure() {
    let server = cluster().await;
    on_bulk(&server, |request| {
        indexed(bulk_ids(request).len()).set_delay(Duration::from_secs(3))
    })
    .await;
    let backend = backend_with(&server, |config| config.request_timeout_ms = 750);

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b"]))
        .await;

    for outcome in &outcomes {
        assert!(is_transient(outcome), "{outcome:?}");
    }
    // The pair, then `a` alone; `b` is failed without waiting out another timeout.
    assert_eq!(bulk_requests(&server).await.len(), 2);
}

#[tokio::test]
async fn a_stalled_cluster_fails_the_page_instead_of_splitting_it_to_single_documents() {
    let server = cluster().await;
    on_bulk(&server, |request| {
        indexed(bulk_ids(request).len()).set_delay(Duration::from_secs(3))
    })
    .await;
    let backend = backend_with(&server, |config| config.request_timeout_ms = 750);

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b", "c", "d", "e", "f"]))
        .await;

    for outcome in &outcomes {
        assert!(is_transient(outcome), "{outcome:?}");
    }
    // 6 -> 3 -> 1 timed out; splitting on would have cost 11 timeouts.
    assert_eq!(bulk_requests(&server).await.len(), 3);
}

#[tokio::test]
async fn a_gateway_timeout_is_split_like_a_client_timeout() {
    let server = cluster().await;
    on_bulk(&server, |request| {
        let ops = bulk_ids(request).len();
        if ops > 1 {
            ResponseTemplate::new(504).set_body_string("gateway timeout")
        } else {
            indexed(ops)
        }
    })
    .await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b", "c", "d"]))
        .await;

    assert_all_ok(&outcomes);
    assert_eq!(bulk_requests(&server).await.len(), 1 + 2 + 4);
}

// ============================================================================
// 429: back off and resend only what was rejected
// ============================================================================

#[tokio::test]
async fn a_throttled_item_is_resent_alone_after_backing_off() {
    let server = cluster().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    on_bulk(&server, move |request| {
        let ops = bulk_ids(request).len();
        if seen.fetch_add(1, Ordering::SeqCst) == 0 {
            ResponseTemplate::new(200).set_body_json(json!({
                "errors": true,
                "items": [
                    {"index": {"status": 201}},
                    {"index": {"status": 429, "error": {
                        "type": "es_rejected_execution_exception",
                        "reason": "rejected execution"
                    }}},
                    {"index": {"status": 201}}
                ]
            }))
        } else {
            indexed(ops)
        }
    })
    .await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b", "c"]))
        .await;

    assert_all_ok(&outcomes);
    let requests = bulk_requests(&server).await;
    assert_eq!(requests.len(), 2);
    let first = bulk_ids(&requests[0]);
    assert_eq!(
        bulk_ids(&requests[1]),
        vec![first[1].clone()],
        "only the throttled item is resent"
    );
}

#[tokio::test]
async fn a_throttled_request_is_resent_whole() {
    let server = cluster().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    on_bulk(&server, move |request| {
        if seen.fetch_add(1, Ordering::SeqCst) == 0 {
            ResponseTemplate::new(429).set_body_string("too many requests")
        } else {
            indexed(bulk_ids(request).len())
        }
    })
    .await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b", "c"]))
        .await;

    assert_all_ok(&outcomes);
    let requests = bulk_requests(&server).await;
    assert_eq!(requests.len(), 2);
    assert_eq!(bulk_ids(&requests[0]), bulk_ids(&requests[1]));
}

#[tokio::test]
async fn throttling_that_never_clears_is_a_transient_failure() {
    let server = cluster().await;
    on_bulk(&server, |_| {
        ResponseTemplate::new(429).set_body_string("too many requests")
    })
    .await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a"]))
        .await;

    assert!(is_transient(&outcomes[0]), "{:?}", outcomes[0]);
    assert_eq!(bulk_requests(&server).await.len(), BULK_MAX_ATTEMPTS);
}

// ============================================================================
// Rejections and whole-request failures
// ============================================================================

#[tokio::test]
async fn an_item_rejection_stays_permanent_and_names_its_type_and_reason() {
    let server = cluster().await;
    on_bulk(&server, |_| {
        ResponseTemplate::new(200).set_body_json(json!({
            "errors": true,
            "items": [{"index": {"status": 400, "error": {
                "type": "document_parsing_exception",
                "reason": "[1:2] failed to parse",
                "caused_by": {
                    "type": "illegal_argument_exception",
                    "reason": "The number of nested documents has exceeded the allowed limit of [10000]."
                }
            }}}]
        }))
    })
    .await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["big"]))
        .await;

    assert!(is_permanent(&outcomes[0]), "{:?}", outcomes[0]);
    let message = outcomes[0].as_ref().unwrap_err().to_string();
    assert!(message.contains("document_parsing_exception"), "{message}");
    assert!(
        message.contains("nested documents has exceeded"),
        "{message}"
    );
    assert_eq!(
        bulk_requests(&server).await.len(),
        1,
        "a rejection is not resent"
    );
}

#[tokio::test]
async fn a_server_error_on_the_whole_request_is_transient_and_not_split() {
    let server = cluster().await;
    on_bulk(&server, |_| {
        ResponseTemplate::new(503).set_body_string("unavailable")
    })
    .await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b", "c", "d"]))
        .await;

    for outcome in &outcomes {
        assert!(is_transient(outcome), "{outcome:?}");
    }
    assert_eq!(bulk_requests(&server).await.len(), 1);
}

// ============================================================================
// Request shape: operations and bytes
// ============================================================================

#[tokio::test]
async fn the_byte_cap_bounds_every_request_body() {
    const CAP: usize = 4_000;
    let server = cluster().await;
    on_bulk(&server, |request| indexed(bulk_ids(request).len())).await;
    let backend = backend_with(&server, |config| config.bulk_max_bytes = CAP);
    let ids: Vec<String> = (0..12).map(|n| format!("p{n}")).collect();
    let resources: Vec<StoredResource> = ids.iter().map(|id| patient(id, 400)).collect();

    let outcomes = backend
        .write_search_entries_page(&tenant(), &resources)
        .await;

    assert_all_ok(&outcomes);
    let requests = bulk_requests(&server).await;
    assert!(
        requests.len() > 1,
        "12 padded documents exceed one {CAP}-byte request"
    );
    for request in &requests {
        let ops = bulk_ids(request).len();
        assert!(
            request.body.len() <= CAP || ops == 1,
            "a {}-byte request carried {ops} documents",
            request.body.len()
        );
    }
    let sent: HashSet<String> = requests.iter().flat_map(bulk_ids).collect();
    assert_eq!(sent.len(), 12, "every document is sent exactly once");
    assert_eq!(
        requests.iter().map(|r| bulk_ids(r).len()).sum::<usize>(),
        12
    );
}

#[tokio::test]
async fn a_page_over_500_operations_takes_more_than_one_request() {
    let server = cluster().await;
    on_bulk(&server, |request| indexed(bulk_ids(request).len())).await;
    let backend = backend_with(&server, |config| config.bulk_max_bytes = 0);
    let ids: Vec<String> = (0..501).map(|n| format!("p{n}")).collect();
    let resources: Vec<StoredResource> = ids.iter().map(|id| patient(id, 0)).collect();

    let outcomes = backend
        .write_search_entries_page(&tenant(), &resources)
        .await;

    assert_all_ok(&outcomes);
    let sizes: Vec<usize> = bulk_requests(&server)
        .await
        .iter()
        .map(|r| bulk_ids(r).len())
        .collect();
    assert_eq!(sizes, vec![500, 1]);
}

#[tokio::test]
async fn a_rebuild_uses_its_own_refresh_policy() {
    let server = cluster().await;
    on_bulk(&server, |request| indexed(bulk_ids(request).len())).await;
    let follows = backend_with(&server, |config| {
        config.write_refresh = WriteRefreshPolicy::WaitFor;
    });
    let own = backend_with(&server, |config| {
        config.write_refresh = WriteRefreshPolicy::WaitFor;
        config.reindex_refresh = Some(WriteRefreshPolicy::False);
    });

    assert_all_ok(
        &follows
            .write_search_entries_page(&tenant(), &page(&["a"]))
            .await,
    );
    assert_all_ok(
        &own.write_search_entries_page(&tenant(), &page(&["b"]))
            .await,
    );

    let requests = bulk_requests(&server).await;
    assert_eq!(requests.len(), 2);
    let query = |request: &Request| request.url.query().unwrap_or_default().to_string();
    assert!(
        query(&requests[0]).contains("refresh=wait_for"),
        "{}",
        query(&requests[0])
    );
    assert!(
        !query(&requests[1]).contains("refresh"),
        "{}",
        query(&requests[1])
    );
}

// ============================================================================
// ensure_index: an outage is transient, a rejection is not
// ============================================================================

#[tokio::test]
async fn an_unreachable_cluster_while_ensuring_the_index_is_transient() {
    let backend = ElasticsearchBackend::new(ElasticsearchConfig {
        nodes: vec!["http://127.0.0.1:1".to_string()],
        request_timeout_ms: 500,
        ..Default::default()
    })
    .expect("client construction is lazy");

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b"]))
        .await;

    for outcome in &outcomes {
        assert!(is_transient(outcome), "{outcome:?}");
    }
}

#[tokio::test]
async fn an_index_check_answered_with_a_server_error_is_transient() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    on_bulk(&server, |request| indexed(bulk_ids(request).len())).await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b"]))
        .await;

    for outcome in &outcomes {
        assert!(is_transient(outcome), "{outcome:?}");
    }
    assert!(
        bulk_requests(&server).await.is_empty(),
        "nothing is written to an index that could not be ensured"
    );
}

#[tokio::test]
async fn an_index_creation_answered_with_a_server_error_is_transient() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a"]))
        .await;

    assert!(is_transient(&outcomes[0]), "{:?}", outcomes[0]);
}

#[tokio::test]
async fn a_rejected_index_creation_stays_permanent() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {"type": "illegal_argument_exception", "reason": "bad mapping"},
            "status": 400
        })))
        .mount(&server)
        .await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a"]))
        .await;

    assert!(is_permanent(&outcomes[0]), "{:?}", outcomes[0]);
    let message = outcomes[0].as_ref().unwrap_err().to_string();
    assert!(message.contains("bad mapping"), "{message}");
}

// ============================================================================
// Concurrency: the chains of one page never overlap on a document
// ============================================================================

/// `HFS_ELASTICSEARCH_BULK_CONCURRENCY` lets the root chunks of one page go
/// out as independent chains. The page's documents are partitioned between
/// those chains, so more chains must not mean a document sent twice, skipped,
/// or reported by the wrong chain. Nothing here may be asserted by request
/// index — which chain finishes first is not fixed (#1125).
#[tokio::test]
async fn concurrent_chains_index_every_document_of_a_page_exactly_once() {
    let server = cluster().await;
    on_bulk(&server, |request| indexed(bulk_ids(request).len())).await;
    let backend = backend_with(&server, |config| {
        config.bulk_max_bytes = 0;
        config.bulk_concurrency = 4;
    });
    let ids: Vec<String> = (0..1_200).map(|n| format!("p{n}")).collect();
    let resources: Vec<StoredResource> = ids.iter().map(|id| patient(id, 0)).collect();

    let outcomes = backend
        .write_search_entries_page(&tenant(), &resources)
        .await;

    assert_eq!(outcomes.len(), 1_200);
    assert_all_ok(&outcomes);

    let requests = bulk_requests(&server).await;
    // 1200 operations at 500 per request: three root chunks, hence three
    // chains however much concurrency was asked for. Sorted, because the
    // chains answer in whatever order the cluster serves them.
    let mut sizes: Vec<usize> = requests.iter().map(|r| bulk_ids(r).len()).collect();
    sizes.sort_unstable();
    assert_eq!(sizes, vec![200, 500, 500]);

    let mut sent: Vec<String> = requests.iter().flat_map(bulk_ids).collect();
    assert_eq!(sent.len(), 1_200, "no document is sent twice");
    let unique: HashSet<&String> = sent.iter().collect();
    assert_eq!(unique.len(), 1_200, "no document is sent twice");
    sent.sort();
    // One document per resource, named `<type>_<id>`.
    let mut expected: Vec<String> = ids.iter().map(|id| format!("Patient_{id}")).collect();
    expected.sort();
    assert_eq!(sent, expected, "every document of the page is sent");
}

/// A cluster that answers nothing in time stalls every chain, not just the one
/// that noticed: the shared flag stops the others before they send. Without it
/// each chain would halve its own chunk down to single documents and wait out
/// a timeout for each of them.
#[tokio::test]
async fn a_stalled_cluster_stops_every_concurrent_chain_instead_of_timing_out_per_document() {
    let server = cluster().await;
    on_bulk(&server, |request| {
        indexed(bulk_ids(request).len()).set_delay(Duration::from_secs(5))
    })
    .await;
    let backend = backend_with(&server, |config| {
        config.bulk_max_bytes = 0;
        config.bulk_concurrency = 4;
        config.request_timeout_ms = 250;
    });
    let resources: Vec<StoredResource> = (0..1_200).map(|n| patient(&format!("p{n}"), 0)).collect();

    let outcomes = backend
        .write_search_entries_page(&tenant(), &resources)
        .await;

    assert_eq!(outcomes.len(), 1_200);
    for outcome in &outcomes {
        assert!(is_transient(outcome), "{outcome:?}");
    }
    // A chain halves 500 -> 250 -> ... -> 1 in nine requests before a lone
    // document times out, and that stops the rest: a few dozen requests at
    // most, nowhere near one per document.
    let requests = bulk_requests(&server).await;
    assert!(
        requests.len() < 50,
        "{} requests for 1200 documents",
        requests.len()
    );
}
