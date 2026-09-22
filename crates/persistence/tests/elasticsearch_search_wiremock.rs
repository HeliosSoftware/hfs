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
use std::sync::atomic::{AtomicUsize, Ordering};

use helios_persistence::backends::elasticsearch::{
    ElasticsearchBackend, ElasticsearchConfig, SCHEMA_VERSION, SCHEMA_VERSION_META_KEY,
};
use helios_persistence::core::{Backend, SearchProvider};
use helios_persistence::error::{BackendError, SearchError, StorageError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::SearchQuery;
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
    assert!(
        matches!(
            error,
            StorageError::Backend(BackendError::Unavailable { .. })
        ),
        "{error:?}"
    );
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
    let result = es.search(&tenant(), &query).await.unwrap();
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
