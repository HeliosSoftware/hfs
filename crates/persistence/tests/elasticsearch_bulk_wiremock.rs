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

/// The message of an `Unavailable` outcome: the text REST sends the client as the
/// 503 body and bulk ingest copies into receipts (#2385).
fn unavailable_message(outcome: &StorageResult<usize>) -> String {
    match outcome {
        Err(StorageError::Backend(BackendError::Unavailable { message, .. })) => message.clone(),
        other => panic!("expected an Unavailable outcome, got {other:?}"),
    }
}

/// The client text names no node, no `_bulk` path and nothing Elasticsearch
/// answered (`answers` are fragments of what the stub answered), only the
/// operation and the fixed advice (#2385).
fn assert_without_elasticsearch_answer(message: &str, server: &MockServer, answers: &[&str]) {
    let node = server.address().to_string();
    assert!(!message.contains(&node), "{message}");
    assert!(!message.contains("_bulk"), "{message}");
    for answer in answers {
        assert!(
            !message.contains(answer),
            "{answer} leaked into the client text: {message}"
        );
    }
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
        // The client text names the operation and the timeout only (#2385): the
        // transport error names the node and the `_bulk` path.
        let message = unavailable_message(outcome);
        assert_eq!(
            message,
            "Bulk index request for a single document timed out after 750 ms: the search index is temporarily unavailable; retry the request"
        );
        let node = server.address().to_string();
        assert!(!message.contains(&node), "{message}");
        assert!(!message.contains("_bulk"), "{message}");
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
        ResponseTemplate::new(429).set_body_string("es-body-429 index hfs_bulk-stub_patient")
    })
    .await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a"]))
        .await;

    assert!(is_transient(&outcomes[0]), "{:?}", outcomes[0]);
    assert_eq!(bulk_requests(&server).await.len(), BULK_MAX_ATTEMPTS);
    // Elasticsearch's answer goes to the log, not into the client text (#2385).
    let message = unavailable_message(&outcomes[0]);
    assert_eq!(
        message,
        "Bulk index request throttled (status 429) 5 times: the search index is temporarily unavailable; retry the request"
    );
    assert_without_elasticsearch_answer(&message, &server, &["es-body-429", "hfs_"]);
}

/// Elasticsearch's flood-stage block answers each item `429`
/// `cluster_block_exception`, naming the index, and does not clear until the disk
/// does. After the resends run out every receipt must carry the fixed text only
/// (#2385).
#[tokio::test]
async fn a_flood_stage_block_that_never_clears_keeps_the_index_out_of_the_receipts() {
    let server = cluster().await;
    on_bulk(&server, |request| {
        let items: Vec<Value> = bulk_ids(request)
            .iter()
            .map(|_| {
                json!({"index": {"status": 429, "error": {
                    "type": "cluster_block_exception",
                    "reason": "index [hfs_bulk-stub_patient] blocked by: [TOO_MANY_REQUESTS/12/disk usage exceeded flood-stage watermark, index has read-only-allow-delete block]"
                }}})
            })
            .collect();
        ResponseTemplate::new(200).set_body_json(json!({"errors": true, "items": items}))
    })
    .await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b", "c"]))
        .await;

    assert_eq!(outcomes.len(), 3);
    for outcome in &outcomes {
        assert!(is_transient(outcome), "{outcome:?}");
        let message = unavailable_message(outcome);
        assert_eq!(
            message,
            "Failed to index document (status 429, throttled 5 times): the search index is temporarily unavailable; retry the request"
        );
        assert_without_elasticsearch_answer(
            &message,
            &server,
            &["hfs_", "cluster_block_exception", "flood-stage"],
        );
    }
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
        ResponseTemplate::new(503).set_body_string("es-body-503 index hfs_bulk-stub_patient")
    })
    .await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b", "c", "d"]))
        .await;

    for outcome in &outcomes {
        assert!(is_transient(outcome), "{outcome:?}");
        // The status stays; Elasticsearch's answer goes to the log (#2385).
        let message = unavailable_message(outcome);
        assert_eq!(
            message,
            "Bulk index request failed (status 503): the search index is temporarily unavailable; retry the request"
        );
        assert_without_elasticsearch_answer(&message, &server, &["es-body-503", "hfs_"]);
    }
    assert_eq!(bulk_requests(&server).await.len(), 1);
}

#[tokio::test]
async fn a_single_document_answered_504_is_transient_without_the_response_body() {
    let server = cluster().await;
    on_bulk(&server, |_| {
        ResponseTemplate::new(504).set_body_string("es-body-504 index hfs_bulk-stub_patient")
    })
    .await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b"]))
        .await;

    for outcome in &outcomes {
        assert!(is_transient(outcome), "{outcome:?}");
        let message = unavailable_message(outcome);
        assert_eq!(
            message,
            "Bulk index request for a single document timed out (status 504): the search index is temporarily unavailable; retry the request"
        );
        assert_without_elasticsearch_answer(&message, &server, &["es-body-504", "hfs_"]);
    }
    // The pair, then each document alone.
    assert_eq!(bulk_requests(&server).await.len(), 3);
}

#[tokio::test]
async fn an_item_answered_with_a_server_error_is_transient_without_its_error() {
    let server = cluster().await;
    on_bulk(&server, |_| {
        let unavailable = json!({"index": {"status": 503, "error": {
            "type": "unavailable_shards_exception",
            "reason": "[hfs_bulk-stub_patient][0] primary shard is not active"
        }}});
        ResponseTemplate::new(200).set_body_json(json!({
            "errors": true,
            "items": [{"index": {"status": 201}}, unavailable, unavailable]
        }))
    })
    .await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b", "c"]))
        .await;

    assert!(outcomes[0].is_ok(), "{:?}", outcomes[0]);
    for outcome in &outcomes[1..] {
        assert!(is_transient(outcome), "{outcome:?}");
        let message = unavailable_message(outcome);
        assert_eq!(
            message,
            "Failed to index document (status 503): the search index is temporarily unavailable; retry the request"
        );
        assert_without_elasticsearch_answer(&message, &server, &["hfs_", "shard"]);
    }
    assert_eq!(
        bulk_requests(&server).await.len(),
        1,
        "a server error on an item is not resent"
    );
}

/// A `200` that is not JSON makes reqwest fail to decode it, and its error text
/// ends with ` for url (http://<node>/_bulk)`. That must not reach the client.
#[tokio::test]
async fn an_unreadable_bulk_response_is_transient_without_the_url() {
    let server = cluster().await;
    on_bulk(&server, |_| {
        ResponseTemplate::new(200).set_body_string("not json")
    })
    .await;
    let backend = backend_with(&server, |_| {});

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b"]))
        .await;

    for outcome in &outcomes {
        assert!(is_transient(outcome), "{outcome:?}");
        let message = unavailable_message(outcome);
        assert_eq!(
            message,
            "Failed to read bulk index response: the search index is temporarily unavailable; retry the request"
        );
        assert_without_elasticsearch_answer(&message, &server, &["not json", "hfs_"]);
    }
    assert_eq!(bulk_requests(&server).await.len(), 1);
}

/// A node that answers `HEAD` with `head_status` and drops the connection on any
/// other request, so a `_bulk` or an index creation fails in the transport rather
/// than with a status. wiremock always answers, so it cannot play this node.
/// Returns the node's URI.
async fn node_that_drops_writes(head_status: u16) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a local port");
    let uri = format!("http://{}", listener.local_addr().expect("local address"));
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 8192];
                loop {
                    let read = match socket.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => read,
                    };
                    if !buffer[..read].starts_with(b"HEAD ") {
                        // Dropping the socket closes the connection mid-request.
                        return;
                    }
                    let reply =
                        format!("HTTP/1.1 {head_status} Status\r\ncontent-length: 0\r\n\r\n");
                    if socket.write_all(reply.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    uri
}

fn backend_at(node: &str) -> ElasticsearchBackend {
    ElasticsearchBackend::new(ElasticsearchConfig {
        nodes: vec![node.to_string()],
        request_timeout_ms: 2_000,
        ..Default::default()
    })
    .expect("client construction is lazy")
}

#[tokio::test]
async fn a_bulk_request_that_never_reaches_the_cluster_is_transient_without_the_node() {
    let node = node_that_drops_writes(200).await;
    let backend = backend_at(&node);

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b"]))
        .await;

    for outcome in &outcomes {
        assert!(is_transient(outcome), "{outcome:?}");
        let message = unavailable_message(outcome);
        assert_eq!(
            message,
            "Failed to send bulk index request: the search index is temporarily unavailable; retry the request"
        );
        let port = node.rsplit(':').next().expect("a port");
        for leaked in ["127.0.0.1", port, "_bulk", "hfs_"] {
            assert!(!message.contains(leaked), "{leaked} leaked: {message}");
        }
    }
}

#[tokio::test]
async fn an_index_creation_that_never_reaches_the_cluster_is_transient_without_the_node() {
    let node = node_that_drops_writes(404).await;
    let backend = backend_at(&node);

    let outcomes = backend
        .write_search_entries_page(&tenant(), &page(&["a", "b"]))
        .await;

    for outcome in &outcomes {
        assert!(is_transient(outcome), "{outcome:?}");
        let message = unavailable_message(outcome);
        assert_eq!(
            message,
            "Failed to create index: the search index is temporarily unavailable; retry the request"
        );
        let port = node.rsplit(':').next().expect("a port");
        for leaked in ["127.0.0.1", port, "hfs_"] {
            assert!(!message.contains(leaked), "{leaked} leaked: {message}");
        }
    }
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

/// Composite sync writes the same `_bulk` page as a rebuild, but under the
/// ordinary write policy: a rebuild that skips the refresh wait must not cost
/// a transaction bundle its read-your-writes search.
#[tokio::test]
async fn a_synced_page_uses_the_write_refresh_policy_not_the_rebuilds() {
    let server = cluster().await;
    on_bulk(&server, |request| indexed(bulk_ids(request).len())).await;
    let backend = backend_with(&server, |config| {
        config.write_refresh = WriteRefreshPolicy::WaitFor;
        config.reindex_refresh = Some(WriteRefreshPolicy::False);
    });

    assert_all_ok(
        &backend
            .write_synced_entries_page(&tenant(), &page(&["a"]))
            .await,
    );

    let requests = bulk_requests(&server).await;
    assert_eq!(requests.len(), 1);
    let query = requests[0].url.query().unwrap_or_default().to_string();
    assert!(query.contains("refresh=wait_for"), "{query}");
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
        // The transport error names the node and the index (#2385).
        let message = unavailable_message(outcome);
        assert_eq!(
            message,
            "Failed to check index existence: the search index is temporarily unavailable; retry the request"
        );
        assert!(!message.contains("127.0.0.1"), "{message}");
        assert!(!message.contains("hfs_bulk-stub"), "{message}");
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
        // The status stays; the index name goes to the log (#2385).
        let message = unavailable_message(outcome);
        assert!(
            message.starts_with("Failed to check index existence (status 503"),
            "{message}"
        );
        assert!(!message.contains("hfs_"), "{message}");
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
    // The status stays; the index name and Elasticsearch's answer go to the log
    // (#2385).
    let message = unavailable_message(&outcomes[0]);
    assert!(
        message.starts_with("Failed to create index (status 500"),
        "{message}"
    );
    assert!(!message.contains("boom"), "{message}");
    assert!(!message.contains("hfs_"), "{message}");
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

/// The prepare pool (#1250) changes where a page's documents are built, not
/// what is built: a page large enough for the pool sends the same `_bulk`
/// operations, in input order, as the inline path, and every outcome is
/// still reported against its own resource. Multi-thread runtime, since the
/// pooled path is gated on one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pooled_extraction_sends_the_page_in_input_order_with_one_outcome_per_resource() {
    use helios_persistence::search::reindex::ReindexPageStats;

    let ids: Vec<String> = (0..24).map(|i| format!("p{i:02}")).collect();
    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();

    let mut bodies = Vec::new();
    for threads in [2usize, 1usize] {
        let server = cluster().await;
        on_bulk(&server, |request| indexed(bulk_ids(request).len())).await;
        let backend = backend_with(&server, |c| c.reindex_prepare_threads = threads);

        let mut stats = ReindexPageStats::default();
        let results = backend
            .write_search_entries_page_timed(&tenant(), &page(&id_refs), &mut stats)
            .await;
        assert_eq!(results.len(), 24, "threads={threads}");
        assert!(
            results.iter().all(Result::is_ok),
            "threads={threads}: {results:?}"
        );
        assert_eq!(stats.sub_batches, 1, "threads={threads}");
        assert_eq!(
            stats.pool_sub_batches,
            u64::from(threads >= 2),
            "threads={threads}: a width of 2 runs the page on the pool, 1 runs it inline"
        );
        assert_eq!(stats.inserted_entries, 24, "threads={threads}");
        assert!(stats.insert > Duration::ZERO, "threads={threads}");

        let requests = bulk_requests(&server).await;
        let sent: Vec<String> = requests.iter().flat_map(bulk_ids).collect();
        let expected: Vec<String> = ids.iter().map(|id| format!("Patient_{id}")).collect();
        assert_eq!(
            sent, expected,
            "threads={threads}: `_bulk` keeps input order"
        );
        bodies.push(
            requests
                .iter()
                .map(|r| String::from_utf8_lossy(&r.body).into_owned())
                .collect::<Vec<_>>(),
        );
    }
    // Same documents either way, apart from nothing: the two runs index the
    // same resources, so the `_bulk` bodies agree line for line once the
    // write instant (which the builder stamps) is masked.
    let mask = |body: &str| -> String {
        body.lines()
            .map(|line| {
                let mut v: Value = serde_json::from_str(line).unwrap_or(Value::Null);
                if let Some(o) = v.as_object_mut() {
                    o.remove("last_updated");
                }
                v.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let pooled: Vec<String> = bodies[0].iter().map(|b| mask(b)).collect();
    let inline: Vec<String> = bodies[1].iter().map(|b| mask(b)).collect();
    assert_eq!(pooled, inline, "the pool must not change what a page sends");
}
