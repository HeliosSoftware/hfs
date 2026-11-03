//! Failure paths of the Elasticsearch read side (`_search`, `_count`) and of
//! the startup mapping reconcile, against an in-process HTTP stub (#1335).
//!
//! A real cluster cannot be made to answer `503` twice and then recover, or to
//! sit behind a proxy that answers `404` by itself, and those are the answers
//! that decide whether a count is retried, reported as `0`, or reported as an
//! error. These tests need **no Docker**: the backend is pointed at a
//! `wiremock` server that plays the cluster, and each test asserts both the
//! outcome and the requests the backend actually sent.
//!
//! Run with:
//! `cargo test -p helios-persistence --features elasticsearch --test elasticsearch_search_wiremock`

#![cfg(feature = "elasticsearch")]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use helios_persistence::backends::elasticsearch::{
    ElasticsearchBackend, ElasticsearchConfig, SCHEMA_VERSION, SCHEMA_VERSION_META_KEY,
};
use helios_persistence::core::{Backend, SearchProvider};
use helios_persistence::error::{BackendError, SearchError, StorageError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchQuery, SortDirective, TotalMode};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// Attempts a retryable read gets, the first included (mirrors the private
/// `MAX_SEARCH_RETRIES + 1`).
const READ_ATTEMPTS: usize = 3;

const COUNT_PATH: &str = "/hfs_read-stub_patient/_count";
const SEARCH_PATH: &str = "/hfs_read-stub_patient/_search";

/// A real 7.17.29 / 8.15.0 answer for a read of an index that does not exist.
const INDEX_NOT_FOUND: &str = r#"{"error":{"root_cause":[{"type":"index_not_found_exception","reason":"no such index [hfs_read-stub_patient]","resource.type":"index_or_alias","resource.id":"hfs_read-stub_patient","index_uuid":"_na_","index":"hfs_read-stub_patient"}],"type":"index_not_found_exception","reason":"no such index [hfs_read-stub_patient]","resource.type":"index_or_alias","resource.id":"hfs_read-stub_patient","index_uuid":"_na_","index":"hfs_read-stub_patient"},"status":404}"#;

/// A real 7.17.29 answer for a query value Elasticsearch could not parse.
const BAD_QUERY: &str = r#"{"error":{"root_cause":[{"type":"parse_exception","reason":"failed to parse date field [x]"}],"type":"search_phase_execution_exception","reason":"all shards failed","phase":"query","grouped":true,"failed_shards":[{"shard":0,"index":"hfs_read-stub_patient","reason":{"type":"parse_exception","reason":"failed to parse date field [x]"}}]},"status":400}"#;

fn tenant() -> TenantContext {
    TenantContext::new(TenantId::new("read-stub"), TenantPermissions::full_access())
}

fn backend(server: &MockServer) -> ElasticsearchBackend {
    let config = ElasticsearchConfig {
        nodes: vec![server.uri()],
        request_timeout_ms: 2_000,
        ..Default::default()
    };
    ElasticsearchBackend::new(config).expect("client construction is lazy")
}

/// Answers `POST {url_path}` with `respond`.
async fn on_post(
    server: &MockServer,
    url_path: &str,
    respond: impl Fn(&Request) -> ResponseTemplate + Send + Sync + 'static,
) {
    Mock::given(method("POST"))
        .and(path(url_path))
        .respond_with(respond)
        .mount(server)
        .await;
}

async fn requests_to(server: &MockServer, http_method: &str, url_path: &str) -> usize {
    server
        .received_requests()
        .await
        .expect("request recording is on")
        .iter()
        .filter(|r| r.method.as_str() == http_method && r.url.path() == url_path)
        .count()
}

fn error_body(status: u16, error_type: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({
        "error": {
            "root_cause": [{ "type": error_type, "reason": "stubbed" }],
            "type": error_type,
            "reason": "stubbed"
        },
        "status": status
    }))
}

/// The issue's case: a count that meets a transient `503` (then a `429`) is
/// retried and answers, instead of failing the search that asked for `_total`.
#[tokio::test]
async fn count_retries_transient_failures_then_answers() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    on_post(&server, COUNT_PATH, move |_| {
        match seen.fetch_add(1, Ordering::SeqCst) {
            0 => error_body(503, "unavailable_shards_exception"),
            1 => error_body(429, "es_rejected_execution_exception"),
            _ => ResponseTemplate::new(200).set_body_json(json!({ "count": 7 })),
        }
    })
    .await;

    let count = backend(&server)
        .search_count(&tenant(), &SearchQuery::new("Patient"))
        .await
        .expect("a transient failure must be retried, not surfaced");

    assert_eq!(count, 7);
    assert_eq!(
        requests_to(&server, "POST", COUNT_PATH).await,
        READ_ATTEMPTS
    );
}

/// Every retryable answer `search` retries, `search_count` retries too: the
/// statuses by themselves, and a rejected execution under any status.
#[tokio::test]
async fn count_retries_every_retryable_answer_and_then_reports_an_error() {
    for (status, error_type) in [
        (429, "es_rejected_execution_exception"),
        (502, "stubbed_exception"),
        (503, "stubbed_exception"),
        (504, "stubbed_exception"),
        (500, "es_rejected_execution_exception"),
    ] {
        let server = MockServer::start().await;
        on_post(&server, COUNT_PATH, move |_| error_body(status, error_type)).await;

        let error = backend(&server)
            .search_count(&tenant(), &SearchQuery::new("Patient"))
            .await
            .expect_err("an exhausted retry is an error, never a count of 0");

        assert!(
            matches!(error, StorageError::Backend(BackendError::Internal { .. })),
            "{status} {error_type}: {error:?}"
        );
        assert!(
            error.to_string().contains("after 3 attempts"),
            "{status} {error_type}: {error}"
        );
        assert_eq!(
            requests_to(&server, "POST", COUNT_PATH).await,
            READ_ATTEMPTS,
            "{status} {error_type}"
        );
    }
}

/// An unreachable cluster is retried and then reported as unavailable.
#[tokio::test]
async fn count_on_an_unreachable_cluster_is_unavailable() {
    // Nothing listens on port 1.
    let config = ElasticsearchConfig {
        nodes: vec!["http://127.0.0.1:1".to_string()],
        request_timeout_ms: 2_000,
        ..Default::default()
    };
    let backend = ElasticsearchBackend::new(config).expect("client construction is lazy");

    let error = backend
        .search_count(&tenant(), &SearchQuery::new("Patient"))
        .await
        .expect_err("no answer is not a count of 0");
    // The client text names the operation only (#2385): the transport error
    // names the node, the request path and the index, and goes to the log.
    let StorageError::Backend(BackendError::Unavailable { message, .. }) = &error else {
        panic!("expected Unavailable, got {error:?}");
    };
    assert_eq!(
        message,
        "Count failed after 3 attempts: the search index is temporarily unavailable; retry the request"
    );
    for leaked in ["127.0.0.1", "_count", "hfs_"] {
        assert!(!message.contains(leaked), "{leaked} leaked: {message}");
    }
}

/// A malformed query is the client's error and is not retried; any other 4xx
/// and a bare `500` are permanent internal errors, not retried either.
#[tokio::test]
async fn count_does_not_retry_permanent_failures() {
    let server = MockServer::start().await;
    on_post(&server, COUNT_PATH, |_| {
        ResponseTemplate::new(400).set_body_raw(BAD_QUERY, "application/json")
    })
    .await;
    let error = backend(&server)
        .search_count(&tenant(), &SearchQuery::new("Patient"))
        .await
        .expect_err("a malformed query is an error");
    assert!(
        matches!(
            error,
            StorageError::Search(SearchError::QueryParseError { .. })
        ),
        "{error:?}"
    );
    assert_eq!(requests_to(&server, "POST", COUNT_PATH).await, 1);

    for (status, error_type) in [(403, "security_exception"), (500, "null_pointer_exception")] {
        let server = MockServer::start().await;
        on_post(&server, COUNT_PATH, move |_| error_body(status, error_type)).await;
        let error = backend(&server)
            .search_count(&tenant(), &SearchQuery::new("Patient"))
            .await
            .expect_err("a permanent failure is an error");
        assert!(
            matches!(error, StorageError::Backend(BackendError::Internal { .. })),
            "{status}: {error:?}"
        );
        assert_eq!(
            requests_to(&server, "POST", COUNT_PATH).await,
            1,
            "{status}"
        );
    }
}

/// `index_not_found_exception` is a count of 0 and an empty search. A `404`
/// that does not carry it — nothing matched on a proxy, a wrong base path — is
/// not a statement about the data and must be an error on both paths.
#[tokio::test]
async fn a_missing_index_is_recognised_by_its_error_type_not_the_bare_status() {
    let server = MockServer::start().await;
    for url_path in [COUNT_PATH, SEARCH_PATH] {
        on_post(&server, url_path, |_| {
            ResponseTemplate::new(404).set_body_raw(INDEX_NOT_FOUND, "application/json")
        })
        .await;
    }
    let es = backend(&server);
    let query = SearchQuery::new("Patient");
    assert_eq!(es.search_count(&tenant(), &query).await.unwrap(), 0);
    let mut accurate = query.clone();
    accurate.total = Some(TotalMode::Accurate);
    let result = es.search(&tenant(), &accurate).await.unwrap();
    assert!(result.resources.items.is_empty());
    assert_eq!(result.total, Some(0));
    assert_eq!(requests_to(&server, "POST", COUNT_PATH).await, 1);

    // wiremock answers an unmatched request with a bare `404`.
    let proxy = MockServer::start().await;
    let es = backend(&proxy);
    let error = es
        .search_count(&tenant(), &query)
        .await
        .expect_err("a bare 404 is not a count of 0");
    assert!(
        matches!(error, StorageError::Backend(BackendError::Internal { .. })),
        "{error:?}"
    );
    es.search(&tenant(), &query)
        .await
        .expect_err("a bare 404 is not an empty result");
}

/// The exception's *name* inside another error (a search value echoed back in
/// a parse failure) must not turn a `400` into an empty result.
#[tokio::test]
async fn a_bad_query_that_mentions_index_not_found_is_still_a_bad_query() {
    let server = MockServer::start().await;
    let body = BAD_QUERY.replace("[x]", "[index_not_found_exception]");
    for url_path in [COUNT_PATH, SEARCH_PATH] {
        let body = body.clone();
        on_post(&server, url_path, move |_| {
            ResponseTemplate::new(400).set_body_raw(body.clone(), "application/json")
        })
        .await;
    }
    let es = backend(&server);
    let query = SearchQuery::new("Patient");

    for error in [
        es.search(&tenant(), &query).await.map(|_| ()).unwrap_err(),
        es.search_count(&tenant(), &query)
            .await
            .map(|_| ())
            .unwrap_err(),
    ] {
        assert!(
            matches!(
                error,
                StorageError::Search(SearchError::QueryParseError { .. })
            ),
            "{error:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// A read of an index that is still being created (#1402).
// ---------------------------------------------------------------------------

/// Attempts a read answered `no_shard_available_action_exception` gets, the
/// first included (mirrors the private `MAX_NO_SHARD_RETRIES + 1`).
const NO_SHARD_READ_ATTEMPTS: usize = 9;

/// A real 7.17.29 answer for a search (and, identically, a count) of an index
/// whose creation is still in flight: the index is in the cluster state, its
/// primary shard is not started.
const NO_SHARD_AVAILABLE: &str = r#"{"error":{"root_cause":[{"type":"no_shard_available_action_exception","reason":"[b25ac7417c31][172.17.0.2:9300][indices:data/read/search[phase/query]]","index_uuid":"EmpvuGepQaG0jYAwhcHI1A","shard":"0","index":"hfs_read-stub_patient"}],"type":"search_phase_execution_exception","reason":"all shards failed","phase":"query","grouped":true,"failed_shards":[{"shard":0,"index":"hfs_read-stub_patient","node":"O31KMMFTRTe1bsMsfGLEvg","reason":{"type":"no_shard_available_action_exception","reason":"[b25ac7417c31][172.17.0.2:9300][indices:data/read/search[phase/query]]","index_uuid":"EmpvuGepQaG0jYAwhcHI1A","shard":"0","index":"hfs_read-stub_patient"}}]},"status":503}"#;

fn no_shard_available() -> ResponseTemplate {
    ResponseTemplate::new(503).set_body_raw(NO_SHARD_AVAILABLE, "application/json")
}

/// The issue's sequence: someone else is creating the index, and a read meets
/// its unstarted primary for longer than the general retry budget lasts. The
/// read waits the creation out instead of failing.
#[tokio::test]
async fn a_read_outlasts_an_index_creation_longer_than_the_general_budget() {
    const UNSTARTED_ANSWERS: usize = READ_ATTEMPTS + 1;

    let server = MockServer::start().await;
    for url_path in [SEARCH_PATH, COUNT_PATH] {
        let calls = Arc::new(AtomicUsize::new(0));
        on_post(&server, url_path, move |_| {
            if calls.fetch_add(1, Ordering::SeqCst) < UNSTARTED_ANSWERS {
                return no_shard_available();
            }
            ResponseTemplate::new(200).set_body_json(json!({
                "count": 0,
                "hits": { "total": { "value": 0, "relation": "eq" }, "hits": [] }
            }))
        })
        .await;
    }
    let es = backend(&server);
    let query = SearchQuery::new("Patient");

    let result = es
        .search(&tenant(), &query)
        .await
        .expect("an index still being created is waited for");
    assert!(result.resources.items.is_empty());
    assert_eq!(
        requests_to(&server, "POST", SEARCH_PATH).await,
        UNSTARTED_ANSWERS + 1
    );

    assert_eq!(es.search_count(&tenant(), &query).await.unwrap(), 0);
    assert_eq!(
        requests_to(&server, "POST", COUNT_PATH).await,
        UNSTARTED_ANSWERS + 1
    );
}

/// The wait is bounded: a shard that never starts is an error after the
/// longer budget, never an empty result.
#[tokio::test]
async fn a_shard_that_never_starts_is_an_error_after_a_bounded_wait() {
    let server = MockServer::start().await;
    on_post(&server, SEARCH_PATH, |_| no_shard_available()).await;

    let error = backend(&server)
        .search(&tenant(), &SearchQuery::new("Patient"))
        .await
        .map(|_| ())
        .expect_err("a lost shard is not an empty result");

    assert!(
        matches!(error, StorageError::Backend(BackendError::Internal { .. })),
        "{error:?}"
    );
    assert!(
        error
            .to_string()
            .contains(&format!("after {NO_SHARD_READ_ATTEMPTS} attempts")),
        "{error}"
    );
    assert_eq!(
        requests_to(&server, "POST", SEARCH_PATH).await,
        NO_SHARD_READ_ATTEMPTS
    );
}

/// The longer budget belongs to the unstarted shard alone: once the cluster
/// answers with any other transient failure, a read that is already past the
/// general budget stops.
#[tokio::test]
async fn the_longer_budget_does_not_carry_over_to_other_transient_failures() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    on_post(&server, SEARCH_PATH, move |_| {
        if calls.fetch_add(1, Ordering::SeqCst) < READ_ATTEMPTS {
            no_shard_available()
        } else {
            error_body(503, "stubbed_exception")
        }
    })
    .await;

    let error = backend(&server)
        .search(&tenant(), &SearchQuery::new("Patient"))
        .await
        .map(|_| ())
        .expect_err("an overloaded cluster is still an error");

    assert!(error.to_string().contains("stubbed_exception"), "{error}");
    assert_eq!(
        requests_to(&server, "POST", SEARCH_PATH).await,
        READ_ATTEMPTS + 1
    );
}

// ---------------------------------------------------------------------------
// Startup mapping reconcile: which requests it sends.
// ---------------------------------------------------------------------------

/// A stub cluster that accepts everything `initialize` sends, reports the
/// given `_meta` for the one existing index, and answers `PUT _mapping` with
/// `put_mapping`.
async fn cluster_with_index(meta: Option<Value>, put_mapping: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/_template/hfs_template"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "acknowledged": true })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/hfs_\\*/_settings/.*$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let mut mappings = json!({ "properties": { "resource_type": { "type": "keyword" } } });
    if let Some(meta) = meta {
        mappings["_meta"] = meta;
    }
    Mock::given(method("GET"))
        .and(path("/hfs_*/_mapping"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "hfs_read-stub_patient": { "mappings": mappings } })),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path_regex("^/[^/]+/_mapping$"))
        .respond_with(put_mapping)
        .mount(&server)
        .await;
    server
}

async fn put_mapping_requests(server: &MockServer) -> Vec<Request> {
    server
        .received_requests()
        .await
        .expect("request recording is on")
        .into_iter()
        .filter(|r| r.method.as_str() == "PUT" && r.url.path().ends_with("/_mapping"))
        .collect()
}

fn acknowledged() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "acknowledged": true }))
}

/// An index with no version marker gets exactly one `PUT _mapping`, carrying
/// the mapping with `ignore_malformed` and the marker merged into its `_meta`.
#[tokio::test]
async fn reconcile_puts_the_current_mapping_on_an_unversioned_index() {
    let server = cluster_with_index(Some(json!({ "owner": "ops" })), acknowledged()).await;
    backend(&server).initialize().await.expect("initialize");

    let puts = put_mapping_requests(&server).await;
    assert_eq!(puts.len(), 1);
    assert_eq!(puts[0].url.path(), "/hfs_read-stub_patient/_mapping");
    let body: Value = serde_json::from_slice(&puts[0].body).unwrap();
    assert_eq!(
        body["_meta"],
        json!({ "owner": "ops", SCHEMA_VERSION_META_KEY: SCHEMA_VERSION })
    );
    let sp = &body["properties"]["search_params"]["properties"];
    assert_eq!(
        sp["date"]["properties"]["value"]["ignore_malformed"],
        json!(true)
    );
    assert_eq!(
        sp["composite"]["properties"]["date"]["ignore_malformed"],
        json!(true)
    );
    assert!(body.get("settings").is_none());
}

/// An index already at the current version — or at a newer one, written by a
/// newer build — is sent nothing.
#[tokio::test]
async fn reconcile_sends_nothing_to_a_current_or_newer_index() {
    for version in [SCHEMA_VERSION, SCHEMA_VERSION + 1] {
        let meta = json!({ SCHEMA_VERSION_META_KEY: version });
        let server = cluster_with_index(Some(meta), acknowledged()).await;
        backend(&server).initialize().await.expect("initialize");
        assert!(
            put_mapping_requests(&server).await.is_empty(),
            "version {version}"
        );
    }
}

/// #1749: startup sets the configured refresh interval on an existing index
/// that was created under another one, and a refusal does not fail startup.
#[tokio::test]
async fn initialize_sets_the_refresh_interval_on_existing_indices() {
    for put_status in [200, 403] {
        let server = cluster_with_index(
            Some(json!({ SCHEMA_VERSION_META_KEY: SCHEMA_VERSION })),
            acknowledged(),
        )
        .await;
        // Mounted ahead of the stub's own empty `_settings` answer.
        Mock::given(method("GET"))
            .and(path_regex("^/hfs_\\*/_settings/.*$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "hfs_read-stub_patient": { "settings": { "index.refresh_interval": "30s" } }
            })))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path_regex("^/[^/]+/_settings$"))
            .respond_with(ResponseTemplate::new(put_status).set_body_json(json!({})))
            .mount(&server)
            .await;
        backend(&server)
            .initialize()
            .await
            .expect("a refresh-interval failure must not fail startup");

        let puts: Vec<Request> = server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.method.as_str() == "PUT" && r.url.path().ends_with("/_settings"))
            .collect();
        assert_eq!(puts.len(), 1, "status {put_status}");
        assert_eq!(puts[0].url.path(), "/hfs_read-stub_patient/_settings");
        let body: Value = serde_json::from_slice(&puts[0].body).unwrap();
        assert_eq!(body, json!({ "index.refresh_interval": "1s" }));
    }
}

/// A refused or failing reconcile never blocks startup.
#[tokio::test]
async fn reconcile_failure_does_not_fail_startup() {
    for response in [
        error_body(400, "illegal_argument_exception"),
        error_body(403, "security_exception"),
        error_body(409, "version_conflict_engine_exception"),
        error_body(503, "stubbed_exception"),
    ] {
        let server = cluster_with_index(None, response).await;
        backend(&server)
            .initialize()
            .await
            .expect("a reconcile failure must not fail startup");
        assert_eq!(put_mapping_requests(&server).await.len(), 1);
    }

    // Not even the listing answering.
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/_template/hfs_template"))
        .respond_with(acknowledged())
        .mount(&server)
        .await;
    backend(&server).initialize().await.expect("initialize");
}

// ---------------------------------------------------------------------------
// `search_ids` (#2295): ids only, no `_source`, no total, same paging as search.
// ---------------------------------------------------------------------------

/// Answers `POST {url_path}` with `answer`, recording every request body sent.
async fn record_posts(
    server: &MockServer,
    url_path: &str,
    status: u16,
    answer: Value,
) -> Arc<Mutex<Vec<Value>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    on_post(server, url_path, move |request| {
        record
            .lock()
            .unwrap()
            .push(serde_json::from_slice(&request.body).unwrap_or(Value::Null));
        ResponseTemplate::new(status).set_body_json(answer.clone())
    })
    .await;
    seen
}

/// One canned hit: `_source` for `search`, `sort` (last_updated, id) for both.
fn hit(id: &str, millis: i64) -> Value {
    json!({
        "_id": format!("Patient_{id}"),
        "_score": null,
        "_source": {
            "resource_type": "Patient",
            "resource_id": id,
            "version_id": "1",
            "content": { "resourceType": "Patient", "id": id },
            "fhir_version": "4.0",
            "last_updated": "2026-01-01T00:00:00Z",
            "is_deleted": false
        },
        "sort": [millis, id]
    })
}

/// A page of `n` hits (descending `last_updated`, as the default sort).
fn answer_with(n: usize) -> Value {
    let hits: Vec<Value> = (0..n)
        .map(|i| hit(&format!("p-{i}"), 1_700_000_000_000 - i as i64))
        .collect();
    json!({ "hits": { "total": { "value": 99, "relation": "eq" }, "hits": hits } })
}

#[tokio::test]
async fn search_ids_asks_for_ids_only_and_no_total() {
    let server = MockServer::start().await;
    let sent = record_posts(&server, SEARCH_PATH, 200, answer_with(4)).await;
    let es = backend(&server);

    let page = es
        .search_ids(&tenant(), &SearchQuery::new("Patient").with_count(3))
        .await
        .expect("search_ids");
    let mut accurate = SearchQuery::new("Patient").with_count(3);
    accurate.total = Some(TotalMode::Accurate);
    es.search(&tenant(), &accurate).await.expect("search");

    let sent = sent.lock().unwrap();
    let (ids_body, full_body) = (&sent[0], &sent[1]);
    assert_eq!(ids_body["_source"], json!(false));
    assert_eq!(ids_body["track_total_hits"], json!(false));
    assert!(ids_body.get("track_scores").is_none());
    assert_eq!(
        ids_body["size"],
        json!(4),
        "count + 1 to detect a next page"
    );
    assert_eq!(ids_body["sort"], full_body["sort"], "same order as search");
    assert_eq!(
        ids_body["query"], full_body["query"],
        "same filters as search"
    );
    // The control: the regular search still loads the bodies, and counts when
    // `_total` asks it to (#2283).
    assert!(full_body.get("_source").is_none());
    assert_eq!(full_body["track_total_hits"], json!(true));
    assert_eq!(page.page_info.total, None);
}

#[tokio::test]
async fn search_ids_matches_search_ids_has_next_and_cursor() {
    let server = MockServer::start().await;
    record_posts(&server, SEARCH_PATH, 200, answer_with(4)).await;
    let es = backend(&server);
    let query = SearchQuery::new("Patient").with_count(3);

    let ids = es.search_ids(&tenant(), &query).await.expect("search_ids");
    let full = es.search(&tenant(), &query).await.expect("search");

    let expected: Vec<String> = full
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect();
    assert_eq!(ids.items, expected);
    assert_eq!(ids.items, ["p-0", "p-1", "p-2"]);
    assert!(ids.page_info.has_next);
    assert_eq!(ids.page_info.has_next, full.resources.page_info.has_next);
    assert!(ids.page_info.next_cursor.is_some());
    assert_eq!(
        ids.page_info.next_cursor,
        full.resources.page_info.next_cursor
    );

    // A short page ends the walk: no next page, no cursor.
    let server = MockServer::start().await;
    record_posts(&server, SEARCH_PATH, 200, answer_with(2)).await;
    let last = backend(&server)
        .search_ids(&tenant(), &query)
        .await
        .expect("search_ids");
    assert_eq!(last.items, ["p-0", "p-1"]);
    assert!(!last.page_info.has_next);
    assert_eq!(last.page_info.next_cursor, None);
}

#[tokio::test]
async fn search_ids_pages_with_the_cursor_search_produced() {
    let server = MockServer::start().await;
    let sent = record_posts(&server, SEARCH_PATH, 200, answer_with(4)).await;
    let es = backend(&server);
    let first = SearchQuery::new("Patient").with_count(3);

    let cursor = es
        .search_ids(&tenant(), &first)
        .await
        .expect("first page")
        .page_info
        .next_cursor
        .expect("a next cursor");
    let next = first.clone().with_cursor(cursor);
    es.search_ids(&tenant(), &next).await.expect("second page");
    es.search(&tenant(), &next)
        .await
        .expect("search, same cursor");

    let sent = sent.lock().unwrap();
    let (ids_body, full_body) = (&sent[1], &sent[2]);
    let after = ids_body["search_after"].as_array().expect("search_after");
    assert_eq!(after, &[json!(1_700_000_000_000_i64 - 2), json!("p-2")]);
    assert_eq!(ids_body["search_after"], full_body["search_after"]);
    assert_eq!(ids_body["_source"], json!(false));
}

#[tokio::test]
async fn search_ids_reads_ids_from_hits_without_a_source() {
    let server = MockServer::start().await;
    let hits: Vec<Value> = ["a", "b"]
        .iter()
        .enumerate()
        .map(|(i, id)| json!({ "_id": format!("Patient_{id}"), "sort": [i as i64, id] }))
        .collect();
    record_posts(
        &server,
        SEARCH_PATH,
        200,
        json!({ "hits": { "hits": hits } }),
    )
    .await;

    let page = backend(&server)
        .search_ids(&tenant(), &SearchQuery::new("Patient").with_count(5))
        .await
        .expect("search_ids");
    assert_eq!(page.items, ["a", "b"]);
}

#[tokio::test]
async fn search_ids_keeps_the_full_path_for_sort_offset_and_total() {
    let mut sorted = SearchQuery::new("Patient").with_count(3);
    sorted = sorted.with_sort(SortDirective::parse("-_lastUpdated"));
    let mut offset = SearchQuery::new("Patient").with_count(3);
    offset.offset = Some(2);
    let mut total = SearchQuery::new("Patient").with_count(3);
    total.total = Some(TotalMode::Accurate);

    for query in [sorted, offset, total] {
        // `track_total_hits` follows `_total` on the full path (#2283).
        let counts = json!(query.total == Some(TotalMode::Accurate));
        let server = MockServer::start().await;
        let sent = record_posts(&server, SEARCH_PATH, 200, answer_with(2)).await;
        let page = backend(&server)
            .search_ids(&tenant(), &query)
            .await
            .expect("search_ids");
        assert_eq!(page.items, ["p-0", "p-1"]);
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(
            sent[0].get("_source").is_none(),
            "the full body is requested"
        );
        assert_eq!(sent[0]["track_total_hits"], counts);
    }
}

#[tokio::test]
async fn search_ids_on_a_missing_index_is_an_empty_page() {
    let server = MockServer::start().await;
    let missing: Value = serde_json::from_str(INDEX_NOT_FOUND).unwrap();
    record_posts(&server, SEARCH_PATH, 404, missing).await;

    let page = backend(&server)
        .search_ids(&tenant(), &SearchQuery::new("Patient"))
        .await
        .expect("a missing index is not an error");
    assert!(page.items.is_empty());
    assert!(!page.page_info.has_next);
    assert_eq!(page.page_info.next_cursor, None);
}
