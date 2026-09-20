//! Failure paths of the Elasticsearch backend's *storage-side* reads —
//! `ResourceStorage::count`, `read`, the existence check inside
//! `create_or_update`, and the `ReindexSource` reads — against an in-process
//! HTTP stub (#1364).
//!
//! The sibling `elasticsearch_search_wiremock.rs` covers `search` and
//! `search_count`; these are the reads that did not yet share their
//! retry/classify path. As there, no Docker is needed: the backend is pointed
//! at a `wiremock` server that plays the cluster, and each test asserts both
//! the outcome and the requests the backend actually sent.
//!
//! Run with:
//! `cargo test -p helios-persistence --features elasticsearch --test elasticsearch_storage_read_wiremock`

#![cfg(feature = "elasticsearch")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use helios_fhir::FhirVersion;
use helios_persistence::backends::elasticsearch::{ElasticsearchBackend, ElasticsearchConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::error::{BackendError, StorageError};
use helios_persistence::search::ReindexSource;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// Attempts a retryable read gets, the first included (mirrors the private
/// `MAX_SEARCH_RETRIES + 1`).
const READ_ATTEMPTS: usize = 3;

const INDEX: &str = "hfs_read-stub_patient";
const COUNT_PATH: &str = "/hfs_read-stub_patient/_count";
const TENANT_COUNT_PATH: &str = "/hfs_read-stub_*/_count";
const SEARCH_PATH: &str = "/hfs_read-stub_patient/_search";
const TENANT_SEARCH_PATH: &str = "/hfs_read-stub_*/_search";
const DOC_PATH: &str = "/hfs_read-stub_patient/_doc/Patient_p1";

/// A real 7.17.29 / 8.15.0 answer for a read of an index that does not exist.
const INDEX_NOT_FOUND: &str = r#"{"error":{"root_cause":[{"type":"index_not_found_exception","reason":"no such index [hfs_read-stub_patient]","resource.type":"index_or_alias","resource.id":"hfs_read-stub_patient","index_uuid":"_na_","index":"hfs_read-stub_patient"}],"type":"index_not_found_exception","reason":"no such index [hfs_read-stub_patient]","resource.type":"index_or_alias","resource.id":"hfs_read-stub_patient","index_uuid":"_na_","index":"hfs_read-stub_patient"},"status":404}"#;

/// A real 7.17.29 / 8.15.0 answer for a `GET _doc` of an id that is not in an
/// index that exists.
const DOC_NOT_FOUND: &str =
    r#"{"_index":"hfs_read-stub_patient","_type":"_doc","_id":"Patient_p1","found":false}"#;

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

/// Answers `{http_method} {url_path}` with `respond`.
async fn on(
    server: &MockServer,
    http_method: &str,
    url_path: &str,
    respond: impl Fn(&Request) -> ResponseTemplate + Send + Sync + 'static,
) {
    Mock::given(method(http_method))
        .and(path(url_path))
        .respond_with(respond)
        .mount(server)
        .await;
}

/// Answers `{http_method} {url_path}` with a `503`, then a `429`, then `ok`.
async fn flaky_then(server: &MockServer, http_method: &str, url_path: &str, ok: ResponseTemplate) {
    let calls = Arc::new(AtomicUsize::new(0));
    on(server, http_method, url_path, move |_| {
        match calls.fetch_add(1, Ordering::SeqCst) {
            0 => error_body(503, "unavailable_shards_exception"),
            1 => error_body(429, "es_rejected_execution_exception"),
            _ => ok.clone(),
        }
    })
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

fn raw_json(status: u16, body: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body, "application/json")
}

/// The `_source` of a stored Patient, as `build_es_document` lays it out.
fn patient_source() -> serde_json::Value {
    json!({
        "resource_type": "Patient",
        "resource_id": "p1",
        "tenant_id": "read-stub",
        "version_id": "4",
        "is_deleted": false,
        "last_updated": "2026-01-02T03:04:05Z",
        "fhir_version": "4.0",
        "content": { "resourceType": "Patient", "id": "p1" }
    })
}

fn found_doc() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "_index": INDEX, "_id": "Patient_p1", "_version": 4, "found": true,
        "_source": patient_source()
    }))
}

fn is_internal(error: &StorageError) -> bool {
    matches!(error, StorageError::Backend(BackendError::Internal { .. }))
}

// ---------------------------------------------------------------------------
// ResourceStorage::count
// ---------------------------------------------------------------------------

/// The issue's case: a count that meets a transient `503` (then a `429`) is
/// retried and answers — for one type and for the whole tenant.
#[tokio::test]
async fn count_retries_transient_failures_then_answers() {
    let server = MockServer::start().await;
    let seven = ResponseTemplate::new(200).set_body_json(json!({ "count": 7 }));
    flaky_then(&server, "POST", COUNT_PATH, seven.clone()).await;
    flaky_then(&server, "POST", TENANT_COUNT_PATH, seven).await;
    let es = backend(&server);

    let count = es
        .count(&tenant(), Some("Patient"))
        .await
        .expect("a transient failure must be retried, not surfaced");
    assert_eq!(count, 7);
    assert_eq!(
        requests_to(&server, "POST", COUNT_PATH).await,
        READ_ATTEMPTS
    );

    assert_eq!(es.count(&tenant(), None).await.unwrap(), 7);
    assert_eq!(
        requests_to(&server, "POST", TENANT_COUNT_PATH).await,
        READ_ATTEMPTS
    );
}

/// Every retryable answer `search` retries, `count` retries too, and an
/// exhausted retry is an error.
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
        on(&server, "POST", COUNT_PATH, move |_| {
            error_body(status, error_type)
        })
        .await;

        let error = backend(&server)
            .count(&tenant(), Some("Patient"))
            .await
            .expect_err("an exhausted retry is an error, never a count of 0");

        assert!(is_internal(&error), "{status} {error_type}: {error:?}");
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

/// A 4xx that is not a throttle, and a bare `500`, are not retried.
#[tokio::test]
async fn count_does_not_retry_permanent_failures() {
    for (status, error_type) in [(403, "security_exception"), (500, "null_pointer_exception")] {
        let server = MockServer::start().await;
        on(&server, "POST", COUNT_PATH, move |_| {
            error_body(status, error_type)
        })
        .await;
        let error = backend(&server)
            .count(&tenant(), Some("Patient"))
            .await
            .expect_err("a permanent failure is an error");
        assert!(is_internal(&error), "{status}: {error:?}");
        assert_eq!(
            requests_to(&server, "POST", COUNT_PATH).await,
            1,
            "{status}"
        );
    }
}

/// An unreachable cluster is reported as unavailable, never as `0`.
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
        .count(&tenant(), Some("Patient"))
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

/// `index_not_found_exception` is a count of 0. A `404` that does not carry it
/// — nothing matched on a proxy, a wrong base path — says nothing about the
/// data and must be an error.
#[tokio::test]
async fn count_recognises_a_missing_index_by_its_error_type_not_the_bare_status() {
    let server = MockServer::start().await;
    on(&server, "POST", COUNT_PATH, |_| {
        raw_json(404, INDEX_NOT_FOUND)
    })
    .await;
    assert_eq!(
        backend(&server)
            .count(&tenant(), Some("Patient"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(requests_to(&server, "POST", COUNT_PATH).await, 1);

    // wiremock answers an unmatched request with a bare `404`.
    let proxy = MockServer::start().await;
    let error = backend(&proxy)
        .count(&tenant(), Some("Patient"))
        .await
        .expect_err("a bare 404 is not a count of 0");
    assert!(is_internal(&error), "{error:?}");
}

/// A `200` that is not a count response is not a count of 0.
#[tokio::test]
async fn count_without_a_count_field_is_an_error() {
    for body in [json!({}), json!({ "count": "many" }), json!({ "took": 3 })] {
        let server = MockServer::start().await;
        let answer = body.clone();
        on(&server, "POST", COUNT_PATH, move |_| {
            ResponseTemplate::new(200).set_body_json(answer.clone())
        })
        .await;
        let error = backend(&server)
            .count(&tenant(), Some("Patient"))
            .await
            .expect_err("a body without `count` is not a count of 0");
        assert!(is_internal(&error), "{body}: {error:?}");
    }

    // Not JSON at all (an HTML page from something in between).
    let server = MockServer::start().await;
    on(&server, "POST", COUNT_PATH, |_| {
        ResponseTemplate::new(200).set_body_string("<html>ok</html>")
    })
    .await;
    backend(&server)
        .count(&tenant(), Some("Patient"))
        .await
        .expect_err("an unparseable body is not a count of 0");
}

// ---------------------------------------------------------------------------
// ResourceStorage::read (and `exists` / `read_batch`, which are built on it)
// ---------------------------------------------------------------------------

/// Positive control plus the retry: a document that is there is returned, also
/// after a transient `503` and `429`.
#[tokio::test]
async fn read_retries_transient_failures_then_returns_the_document() {
    let server = MockServer::start().await;
    flaky_then(&server, "GET", DOC_PATH, found_doc()).await;

    let stored = backend(&server)
        .read(&tenant(), "Patient", "p1")
        .await
        .expect("a transient failure must be retried, not surfaced")
        .expect("the document is there");
    assert_eq!(stored.id(), "p1");
    assert_eq!(stored.version_id(), "4");
    assert_eq!(requests_to(&server, "GET", DOC_PATH).await, READ_ATTEMPTS);
}

/// Elasticsearch's own two "not there" answers — `found: false` and
/// `index_not_found_exception` — are `None`, after one request.
#[tokio::test]
async fn read_of_a_missing_document_or_index_is_none() {
    for body in [DOC_NOT_FOUND, INDEX_NOT_FOUND] {
        let server = MockServer::start().await;
        on(&server, "GET", DOC_PATH, move |_| raw_json(404, body)).await;
        let es = backend(&server);
        assert!(es.read(&tenant(), "Patient", "p1").await.unwrap().is_none());
        assert!(!es.exists(&tenant(), "Patient", "p1").await.unwrap());
        assert_eq!(requests_to(&server, "GET", DOC_PATH).await, 2);
    }
}

/// A `404` that Elasticsearch did not write is not "this resource does not
/// exist": Elasticsearch always says `found: false` or names the missing index.
#[tokio::test]
async fn read_of_a_bare_404_is_an_error() {
    // wiremock answers an unmatched request with a bare `404`.
    let proxy = MockServer::start().await;
    let es = backend(&proxy);
    let error = es
        .read(&tenant(), "Patient", "p1")
        .await
        .expect_err("a bare 404 is not a missing resource");
    assert!(is_internal(&error), "{error:?}");
    es.exists(&tenant(), "Patient", "p1")
        .await
        .expect_err("a bare 404 is not `false`");
}

/// A `200` without `_source` is not a missing resource either.
#[tokio::test]
async fn read_of_a_200_without_a_source_is_an_error() {
    let server = MockServer::start().await;
    on(&server, "GET", DOC_PATH, |_| {
        ResponseTemplate::new(200).set_body_json(json!({ "took": 1 }))
    })
    .await;
    let error = backend(&server)
        .read(&tenant(), "Patient", "p1")
        .await
        .expect_err("an unexpected body is not a missing resource");
    assert!(is_internal(&error), "{error:?}");
}

/// Exhausted retries and permanent failures are errors, with the same attempt
/// counts as every other read.
#[tokio::test]
async fn read_failures_are_errors() {
    for (status, error_type, attempts) in [
        (503, "stubbed_exception", READ_ATTEMPTS),
        (429, "es_rejected_execution_exception", READ_ATTEMPTS),
        (403, "security_exception", 1),
        (500, "null_pointer_exception", 1),
    ] {
        let server = MockServer::start().await;
        on(&server, "GET", DOC_PATH, move |_| {
            error_body(status, error_type)
        })
        .await;
        let error = backend(&server)
            .read(&tenant(), "Patient", "p1")
            .await
            .expect_err("a failed read is an error, never `None`");
        assert!(is_internal(&error), "{status}: {error:?}");
        assert_eq!(
            requests_to(&server, "GET", DOC_PATH).await,
            attempts,
            "{status}"
        );
    }
}

// ---------------------------------------------------------------------------
// The existence check inside create_or_update
// ---------------------------------------------------------------------------

/// Stubs what a successful write needs after the existence check.
async fn accept_the_write(server: &MockServer) {
    on(server, "HEAD", "/hfs_read-stub_patient", |_| {
        ResponseTemplate::new(200)
    })
    .await;
    on(server, "POST", DOC_PATH, |_| {
        ResponseTemplate::new(201).set_body_json(json!({ "result": "created" }))
    })
    .await;
}

/// "New, start at version 1" needs Elasticsearch to say the document is
/// absent. A bare `404` does not say that, and must not reset the version of a
/// resource that may well exist: the write fails before anything is indexed.
#[tokio::test]
async fn create_or_update_does_not_take_a_bare_404_for_a_new_resource() {
    let server = MockServer::start().await;
    accept_the_write(&server).await;

    let error = backend(&server)
        .create_or_update(
            &tenant(),
            "Patient",
            "p1",
            json!({ "resourceType": "Patient" }),
            FhirVersion::default(),
        )
        .await
        .expect_err("a bare 404 does not establish that the resource is new");
    assert!(is_internal(&error), "{error:?}");
    assert_eq!(requests_to(&server, "POST", DOC_PATH).await, 0);
}

/// The existence check is retried, and both real answers still work: an absent
/// document starts at version 1, an existing one gets the next version.
#[tokio::test]
async fn create_or_update_retries_the_existence_check() {
    let server = MockServer::start().await;
    accept_the_write(&server).await;
    flaky_then(&server, "GET", DOC_PATH, raw_json(404, DOC_NOT_FOUND)).await;
    let (stored, is_new) = backend(&server)
        .create_or_update(
            &tenant(),
            "Patient",
            "p1",
            json!({ "resourceType": "Patient" }),
            FhirVersion::default(),
        )
        .await
        .expect("a transient failure of the existence check must be retried");
    assert!(is_new);
    assert_eq!(stored.version_id(), "1");
    assert_eq!(requests_to(&server, "GET", DOC_PATH).await, READ_ATTEMPTS);
    assert_eq!(requests_to(&server, "POST", DOC_PATH).await, 1);

    let server = MockServer::start().await;
    accept_the_write(&server).await;
    flaky_then(&server, "GET", DOC_PATH, found_doc()).await;
    let (stored, is_new) = backend(&server)
        .create_or_update(
            &tenant(),
            "Patient",
            "p1",
            json!({ "resourceType": "Patient" }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert!(!is_new);
    assert_eq!(stored.version_id(), "5");
}

// ---------------------------------------------------------------------------
// ReindexSource reads
// ---------------------------------------------------------------------------

/// A failed listing is an error. It used to be "this tenant has no resource
/// types", which makes a `$reindex` finish successfully having done nothing.
#[tokio::test]
async fn list_resource_types_retries_and_never_reports_a_failure_as_empty() {
    let server = MockServer::start().await;
    flaky_then(
        &server,
        "POST",
        TENANT_SEARCH_PATH,
        ResponseTemplate::new(200).set_body_json(json!({
            "hits": { "hits": [] },
            "aggregations": { "types": { "buckets": [
                { "key": "Patient", "doc_count": 2 },
                { "key": "Observation", "doc_count": 1 }
            ]}}
        })),
    )
    .await;
    let types = backend(&server)
        .list_resource_types(&tenant())
        .await
        .expect("a transient failure must be retried, not surfaced");
    assert_eq!(types, ["Patient", "Observation"]);
    assert_eq!(
        requests_to(&server, "POST", TENANT_SEARCH_PATH).await,
        READ_ATTEMPTS
    );

    for (status, error_type) in [(503, "stubbed_exception"), (403, "security_exception")] {
        let server = MockServer::start().await;
        on(&server, "POST", TENANT_SEARCH_PATH, move |_| {
            error_body(status, error_type)
        })
        .await;
        let error = backend(&server)
            .list_resource_types(&tenant())
            .await
            .expect_err("a failed listing is not an empty tenant");
        assert!(is_internal(&error), "{status}: {error:?}");
    }

    // No index under the tenant's prefix: Elasticsearch answers a wildcard
    // with an empty `200`, and a concrete missing index by name.
    let server = MockServer::start().await;
    on(&server, "POST", TENANT_SEARCH_PATH, |_| {
        raw_json(404, INDEX_NOT_FOUND)
    })
    .await;
    assert!(
        backend(&server)
            .list_resource_types(&tenant())
            .await
            .unwrap()
            .is_empty()
    );
}

/// A failed page is an error. It used to be an empty last page, which ends the
/// walk early and silently truncates whatever is being rebuilt from it.
#[tokio::test]
async fn fetch_resources_page_retries_and_never_reports_a_failure_as_the_last_page() {
    let server = MockServer::start().await;
    flaky_then(
        &server,
        "POST",
        SEARCH_PATH,
        ResponseTemplate::new(200).set_body_json(json!({
            "hits": { "hits": [
                { "_id": "Patient_p1", "_source": patient_source(),
                  "sort": [1767323045000u64, "p1"] }
            ]}
        })),
    )
    .await;
    let page = backend(&server)
        .fetch_resources_page(&tenant(), "Patient", None, 1)
        .await
        .expect("a transient failure must be retried, not surfaced");
    assert_eq!(page.resources.len(), 1);
    assert_eq!(page.resources[0].id(), "p1");
    assert!(page.next_cursor.is_some());
    assert_eq!(
        requests_to(&server, "POST", SEARCH_PATH).await,
        READ_ATTEMPTS
    );

    for (status, error_type) in [(503, "stubbed_exception"), (403, "security_exception")] {
        let server = MockServer::start().await;
        on(&server, "POST", SEARCH_PATH, move |_| {
            error_body(status, error_type)
        })
        .await;
        let error = backend(&server)
            .fetch_resources_page(&tenant(), "Patient", None, 10)
            .await
            .expect_err("a failed page is not the end of the data");
        assert!(is_internal(&error), "{status}: {error:?}");
    }

    let server = MockServer::start().await;
    on(&server, "POST", SEARCH_PATH, |_| {
        raw_json(404, INDEX_NOT_FOUND)
    })
    .await;
    let page = backend(&server)
        .fetch_resources_page(&tenant(), "Patient", None, 10)
        .await
        .unwrap();
    assert!(page.resources.is_empty() && page.next_cursor.is_none());
}
