//! Failure paths of the Elasticsearch backend's *write* requests — the
//! single-document index and delete, the contained-document maintenance around
//! them, and the delete-by-query purges — against an in-process HTTP stub
//! (#1382). The composite health probe's reading of a failed `count` is covered
//! here too, because a stubbed cluster is the easiest way to fail one.
//!
//! The sibling `elasticsearch_storage_read_wiremock.rs` covers the storage-side
//! reads and `elasticsearch_bulk_wiremock.rs` the `_bulk` path. As there, no
//! Docker is needed: the backend is pointed at a `wiremock` server that plays
//! the cluster, and each test asserts both the outcome and the requests the
//! backend actually sent.
//!
//! Run with:
//! `cargo test -p helios-persistence --features elasticsearch --test elasticsearch_storage_write_wiremock`

#![cfg(feature = "elasticsearch")]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use helios_fhir::FhirVersion;
use helios_persistence::backends::elasticsearch::{ElasticsearchBackend, ElasticsearchConfig};
use helios_persistence::composite::{HealthCheckResult, HealthMonitor};
use helios_persistence::core::{PurgableStorage, ResourceStorage};
use helios_persistence::error::{BackendError, ResourceError, StorageError};
use helios_persistence::search::{ReindexTarget, SearchParameterLoader, TenantSearchRegistries};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::StoredResource;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// Attempts a retryable write gets, the first included (mirrors the private
/// `MAX_SEARCH_RETRIES + 1` the reads use).
const WRITE_ATTEMPTS: usize = 3;

const TENANT: &str = "write-stub";
const INDEX_PATH: &str = "/hfs_write-stub_patient";
const DOC_PATH: &str = "/hfs_write-stub_patient/_doc/Patient_p1";
/// Where a contained Organization's document goes; its id carries a `#`, so
/// requests are matched by prefix rather than by the encoded path.
const CONTAINED_INDEX_PATH: &str = "/hfs_write-stub_organization";
const CONTAINED_DOC_PREFIX: &str = "/hfs_write-stub_organization/_doc/";
const TYPE_DBQ_PATH: &str = "/hfs_write-stub_patient/_delete_by_query";
const TENANT_DBQ_PATH: &str = "/hfs_write-stub_*/_delete_by_query";

/// A real 7.17.29 / 8.15.0 answer for a delete of an id that is not in an
/// index that exists.
const DELETE_NOT_FOUND: &str = r#"{"_index":"hfs_write-stub_patient","_id":"Patient_p1","_version":1,"result":"not_found","_shards":{"total":2,"successful":1,"failed":0},"_seq_no":3,"_primary_term":1}"#;

/// A real answer for a delete in an index that does not exist.
const INDEX_NOT_FOUND: &str = r#"{"error":{"root_cause":[{"type":"index_not_found_exception","reason":"no such index [hfs_write-stub_patient]","resource.type":"index_expression","resource.id":"hfs_write-stub_patient","index_uuid":"_na_","index":"hfs_write-stub_patient"}],"type":"index_not_found_exception","reason":"no such index [hfs_write-stub_patient]","resource.type":"index_expression","resource.id":"hfs_write-stub_patient","index_uuid":"_na_","index":"hfs_write-stub_patient"},"status":404}"#;

fn tenant() -> TenantContext {
    TenantContext::new(TenantId::new(TENANT), TenantPermissions::full_access())
}

/// A backend with the full R4 search parameter set: with only the embedded
/// handful, a contained resource extracts nothing and no contained document is
/// ever written, which would make every contained-document test pass vacuously.
fn backend(server: &MockServer) -> ElasticsearchBackend {
    let config = ElasticsearchConfig {
        nodes: vec![server.uri()],
        request_timeout_ms: 2_000,
        ..Default::default()
    };
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let loader = SearchParameterLoader::new(FhirVersion::default());
    let registries = Arc::new(TenantSearchRegistries::base_only());
    {
        let mut registry = registries.base().write();
        let embedded = loader.load_embedded().expect("embedded search parameters");
        let spec = loader
            .load_from_spec_file(&data_dir)
            .expect("data/search-parameters-r4.json");
        assert!(spec.len() > 1000, "the full parameter set must be loaded");
        for param in embedded.into_iter().chain(spec) {
            let _ = registry.register(param);
        }
    }
    ElasticsearchBackend::with_shared_registry(config, registries)
        .expect("client construction is lazy")
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

/// Answers every `{http_method}` request under `url_prefix` with `respond`.
async fn on_prefix(
    server: &MockServer,
    http_method: &str,
    url_prefix: &str,
    respond: impl Fn(&Request) -> ResponseTemplate + Send + Sync + 'static,
) {
    Mock::given(method(http_method))
        .and(path_regex(format!("^{}.+$", regex_escape(url_prefix))))
        .respond_with(respond)
        .mount(server)
        .await;
}

fn regex_escape(literal: &str) -> String {
    literal.replace('*', r"\*")
}

/// A responder that answers a `503`, then a `429`, then `ok`.
fn flaky(ok: ResponseTemplate) -> impl Fn(&Request) -> ResponseTemplate + Send + Sync + 'static {
    let calls = AtomicUsize::new(0);
    move |_| match calls.fetch_add(1, Ordering::SeqCst) {
        0 => error_body(503, "unavailable_shards_exception"),
        1 => error_body(429, "es_rejected_execution_exception"),
        _ => ok.clone(),
    }
}

async fn requests_to(server: &MockServer, http_method: &str, url_path: &str) -> usize {
    requests_under(server, http_method, url_path)
        .await
        .into_iter()
        .filter(|p| p == url_path)
        .count()
}

/// The paths of the `{http_method}` requests received under `url_prefix`.
async fn requests_under(server: &MockServer, http_method: &str, url_prefix: &str) -> Vec<String> {
    server
        .received_requests()
        .await
        .expect("request recording is on")
        .iter()
        .filter(|r| r.method.as_str() == http_method && r.url.path().starts_with(url_prefix))
        .map(|r| r.url.path().to_string())
        .collect()
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

fn indexed() -> ResponseTemplate {
    ResponseTemplate::new(201).set_body_json(json!({ "result": "created" }))
}

fn doc_deleted() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "result": "deleted" }))
}

fn swept(deleted: u64) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "took": 1, "timed_out": false, "total": deleted, "deleted": deleted,
        "version_conflicts": 0, "failures": []
    }))
}

/// Both indices exist, so `ensure_index` is one `HEAD` each.
async fn indices_exist(server: &MockServer) {
    on(server, "HEAD", INDEX_PATH, |_| ResponseTemplate::new(200)).await;
    on(server, "HEAD", CONTAINED_INDEX_PATH, |_| {
        ResponseTemplate::new(200)
    })
    .await;
}

fn plain_patient() -> Value {
    json!({ "resourceType": "Patient", "id": "p1", "name": [{ "family": "Stub" }] })
}

/// A Patient whose contained Organization has an indexable `name`.
fn patient_with_contained() -> Value {
    json!({
        "resourceType": "Patient",
        "id": "p1",
        "contained": [{ "resourceType": "Organization", "id": "org1", "name": "Acme" }],
        "managingOrganization": { "reference": "#org1" }
    })
}

fn stored(content: Value) -> StoredResource {
    let now = chrono::Utc::now();
    StoredResource::from_storage(
        "Patient",
        "p1",
        "1",
        TenantId::new(TENANT),
        content,
        now,
        now,
        None,
        FhirVersion::default(),
    )
}

fn is_internal(error: &StorageError) -> bool {
    matches!(error, StorageError::Backend(BackendError::Internal { .. }))
}

fn is_unavailable(error: &StorageError) -> bool {
    matches!(
        error,
        StorageError::Backend(BackendError::Unavailable { .. })
    )
}

fn is_not_found(error: &StorageError) -> bool {
    matches!(
        error,
        StorageError::Resource(ResourceError::NotFound { .. })
    )
}

/// The detail of an error, including the message `Unavailable` does not display.
fn detail(error: &StorageError) -> String {
    match error {
        StorageError::Backend(BackendError::Unavailable { message, .. }) => message.clone(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Single-document index requests: create, update, create_or_update,
// write_search_entries
// ---------------------------------------------------------------------------

/// Every single-document write path, by name, run against `es`.
async fn run_index_write(es: &ElasticsearchBackend, which: &str) -> Result<(), StorageError> {
    let version = FhirVersion::default();
    match which {
        "create" => es
            .create(&tenant(), "Patient", plain_patient(), version)
            .await
            .map(|_| ()),
        "update" => es
            .update(&tenant(), &stored(plain_patient()), plain_patient())
            .await
            .map(|_| ()),
        "create_or_update" => es
            .create_or_update(&tenant(), "Patient", "p1", plain_patient(), version)
            .await
            .map(|_| ()),
        "write_search_entries" => es
            .write_search_entries(&tenant(), &stored(plain_patient()))
            .await
            .map(|_| ()),
        other => panic!("unknown write path {other}"),
    }
}

const INDEX_WRITES: [&str; 4] = [
    "create",
    "update",
    "create_or_update",
    "write_search_entries",
];

/// What every index write needs besides the index request itself.
async fn accept_everything_but_the_index_request(server: &MockServer) {
    indices_exist(server).await;
    // `create_or_update`'s existence check: the document is new.
    on(server, "GET", DOC_PATH, |_| {
        raw_json(
            404,
            r#"{"_index":"hfs_write-stub_patient","_id":"Patient_p1","found":false}"#,
        )
    })
    .await;
    on(server, "POST", TENANT_DBQ_PATH, |_| swept(0)).await;
}

/// The issue's case (2): a single write that meets a `503` (then a `429`) is
/// retried and succeeds. It used to fail after one attempt.
#[tokio::test]
async fn index_writes_retry_transient_failures_then_succeed() {
    for which in INDEX_WRITES {
        let server = MockServer::start().await;
        accept_everything_but_the_index_request(&server).await;
        on(&server, "POST", DOC_PATH, flaky(indexed())).await;

        run_index_write(&backend(&server), which)
            .await
            .unwrap_or_else(|e| panic!("{which}: a transient failure must be retried: {e:?}"));
        assert_eq!(
            requests_to(&server, "POST", DOC_PATH).await,
            WRITE_ATTEMPTS,
            "{which}"
        );
    }
}

/// #1402: a create-index request that timed out waiting for the primary shard
/// (`shards_acknowledged: false`) still created the index. The write goes
/// ahead — Elasticsearch makes the index request wait for the primary — and
/// the index is not created a second time.
#[tokio::test]
async fn a_created_index_whose_primary_has_not_started_does_not_fail_the_write() {
    let server = MockServer::start().await;
    on(&server, "HEAD", INDEX_PATH, |_| ResponseTemplate::new(404)).await;
    on(&server, "PUT", INDEX_PATH, |_| {
        ResponseTemplate::new(200).set_body_json(json!({
            "acknowledged": true,
            "shards_acknowledged": false,
            "index": "hfs_write-stub_patient"
        }))
    })
    .await;
    on(&server, "POST", TENANT_DBQ_PATH, |_| swept(0)).await;
    on(&server, "POST", DOC_PATH, |_| indexed()).await;

    run_index_write(&backend(&server), "create")
        .await
        .expect("an unstarted primary must not fail the write");
    assert_eq!(requests_to(&server, "PUT", INDEX_PATH).await, 1);
    assert_eq!(requests_to(&server, "POST", DOC_PATH).await, 1);
}

/// Every retryable answer is retried, and an exhausted retry is an
/// *unavailable* error: the document was never judged, so a later attempt (the
/// composite's own retry, a `$reindex`) may well succeed.
#[tokio::test]
async fn index_writes_retry_every_retryable_answer_and_then_report_unavailable() {
    for (status, error_type) in [
        (429, "es_rejected_execution_exception"),
        (502, "stubbed_exception"),
        (503, "stubbed_exception"),
        (504, "stubbed_exception"),
        (500, "es_rejected_execution_exception"),
    ] {
        let server = MockServer::start().await;
        accept_everything_but_the_index_request(&server).await;
        on(&server, "POST", DOC_PATH, move |_| {
            error_body(status, error_type)
        })
        .await;

        let error = run_index_write(&backend(&server), "create")
            .await
            .expect_err("an exhausted retry is an error");
        assert!(is_unavailable(&error), "{status} {error_type}: {error:?}");
        assert!(
            detail(&error).contains("after 3 attempts"),
            "{status} {error_type}: {}",
            detail(&error)
        );
        assert_eq!(
            requests_to(&server, "POST", DOC_PATH).await,
            WRITE_ATTEMPTS,
            "{status} {error_type}"
        );
    }
}

/// A rejection of the document itself, an authorization failure, a version
/// conflict and a bare `500` are answered deterministically: one attempt.
#[tokio::test]
async fn index_writes_do_not_retry_permanent_failures() {
    for (status, error_type) in [
        (400, "mapper_parsing_exception"),
        (403, "security_exception"),
        (404, "stubbed_exception"),
        (409, "version_conflict_engine_exception"),
        (500, "null_pointer_exception"),
    ] {
        for which in INDEX_WRITES {
            let server = MockServer::start().await;
            accept_everything_but_the_index_request(&server).await;
            on(&server, "POST", DOC_PATH, move |_| {
                error_body(status, error_type)
            })
            .await;

            let error = run_index_write(&backend(&server), which)
                .await
                .expect_err("a permanent failure is an error");
            assert!(is_internal(&error), "{which} {status}: {error:?}");
            assert!(
                error.to_string().contains(&format!("status {status}")),
                "{which} {status}: {error}"
            );
            assert_eq!(
                requests_to(&server, "POST", DOC_PATH).await,
                1,
                "{which} {status}"
            );
        }
    }
}

/// A cluster nothing listens on: `127.0.0.1:1`.
///
/// What this must produce is a *refused connection* — retried, then reported
/// as unavailable — not a *request timeout*, which is deliberately never
/// retried (each resend would wait out another full timeout). Which of the two
/// happens first depends on the platform: Linux refuses a closed local port at
/// once, while Windows retries the connect for about two seconds before
/// reporting the refusal. With a 2 s request timeout the timeout won that race
/// on the Windows runner, so the write was not retried and the health probe —
/// three slow refusals — outlived its own 5 s budget. The request timeout here
/// only has to be comfortably longer than the slowest platform's refusal.
fn unreachable_config() -> ElasticsearchConfig {
    ElasticsearchConfig {
        nodes: vec!["http://127.0.0.1:1".to_string()],
        request_timeout_ms: 15_000,
        ..Default::default()
    }
}

/// An unreachable cluster is retried like the reads retry it, then reported as
/// unavailable.
#[tokio::test]
async fn index_write_on_an_unreachable_cluster_is_unavailable() {
    // Nothing listens on port 1. (`update` skips nothing: `ensure_index` is the
    // first request and fails the same way.)
    let es = ElasticsearchBackend::new(unreachable_config()).expect("client construction is lazy");
    let error = es
        .delete(&tenant(), "Patient", "p1")
        .await
        .expect_err("no answer is not a successful delete");
    assert!(is_unavailable(&error), "{error:?}");
    assert!(detail(&error).contains("after 3 attempts"), "{error:?}");
}

/// HFS indexes by explicit id with plain index semantics — no
/// `op_type=create`, no external version, no `if_seq_no` — so resending a write
/// whose first response was lost overwrites the document with itself instead
/// of answering `409`. This pins the request shape that makes the retry safe.
#[tokio::test]
async fn index_requests_carry_nothing_that_makes_a_resend_conflict() {
    for which in INDEX_WRITES {
        let server = MockServer::start().await;
        accept_everything_but_the_index_request(&server).await;
        on(&server, "POST", DOC_PATH, |_| indexed()).await;
        run_index_write(&backend(&server), which).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        let write = requests
            .iter()
            .find(|r| r.method.as_str() == "POST" && r.url.path() == DOC_PATH)
            .expect("the index request");
        let query = write.url.query().unwrap_or_default();
        for forbidden in ["op_type", "version", "if_seq_no", "if_primary_term"] {
            assert!(!query.contains(forbidden), "{which}: {query}");
        }
        assert!(!write.url.path().contains("_create"), "{which}");
    }
}

// ---------------------------------------------------------------------------
// delete
// ---------------------------------------------------------------------------

/// The issue's case (2) for delete: retried, then the contained sweep runs.
#[tokio::test]
async fn delete_retries_transient_failures_then_succeeds() {
    let server = MockServer::start().await;
    on(&server, "DELETE", DOC_PATH, flaky(doc_deleted())).await;
    on(&server, "POST", TENANT_DBQ_PATH, |_| swept(0)).await;

    backend(&server)
        .delete(&tenant(), "Patient", "p1")
        .await
        .expect("a transient failure must be retried");
    assert_eq!(
        requests_to(&server, "DELETE", DOC_PATH).await,
        WRITE_ATTEMPTS
    );
    assert_eq!(requests_to(&server, "POST", TENANT_DBQ_PATH).await, 1);
}

/// The issue's case (3): "this resource does not exist" is a claim only the
/// cluster can make. Elasticsearch makes it with `"result":"not_found"` or an
/// `index_not_found_exception`; a bare `404` (a proxy, a wrong base path) is an
/// error, and used to be reported as `NotFound`.
#[tokio::test]
async fn delete_recognises_not_found_by_the_body_not_the_bare_status() {
    for body in [DELETE_NOT_FOUND, INDEX_NOT_FOUND] {
        let server = MockServer::start().await;
        on(&server, "DELETE", DOC_PATH, move |_| raw_json(404, body)).await;
        on(&server, "POST", TENANT_DBQ_PATH, |_| swept(0)).await;
        let error = backend(&server)
            .delete(&tenant(), "Patient", "p1")
            .await
            .expect_err("the document is not there");
        assert!(is_not_found(&error), "{error:?}");
        assert_eq!(requests_to(&server, "DELETE", DOC_PATH).await, 1);
    }

    for bare in [
        ResponseTemplate::new(404),
        ResponseTemplate::new(404).set_body_string("<html>404 Not Found</html>"),
        raw_json(404, r#"{"message":"no route"}"#),
    ] {
        let server = MockServer::start().await;
        on(&server, "DELETE", DOC_PATH, move |_| bare.clone()).await;
        on(&server, "POST", TENANT_DBQ_PATH, |_| swept(0)).await;
        let error = backend(&server)
            .delete(&tenant(), "Patient", "p1")
            .await
            .expect_err("a bare 404 says nothing about the document");
        assert!(is_internal(&error), "{error:?}");
        assert_eq!(requests_to(&server, "DELETE", DOC_PATH).await, 1);
    }
}

/// A delete that is repeated because its *contained sweep* failed finds the
/// parent document already gone. The sweep must still run on that repeat, or
/// the stale contained documents would outlive every retry.
#[tokio::test]
async fn delete_of_an_absent_document_still_sweeps_its_contained_documents() {
    let server = MockServer::start().await;
    on(&server, "DELETE", DOC_PATH, |_| {
        raw_json(404, DELETE_NOT_FOUND)
    })
    .await;
    on(&server, "POST", TENANT_DBQ_PATH, |_| swept(2)).await;

    let error = backend(&server)
        .delete(&tenant(), "Patient", "p1")
        .await
        .expect_err("still reported as not found");
    assert!(is_not_found(&error), "{error:?}");
    assert_eq!(requests_to(&server, "POST", TENANT_DBQ_PATH).await, 1);
}

/// The lost-response case: the first delete was applied but its answer never
/// arrived (here: a gateway's `503`), so the resend is answered `not_found`.
/// The document is gone, which is what was asked for — not a `NotFound` error.
#[tokio::test]
async fn delete_answered_not_found_on_a_resend_is_a_success() {
    let server = MockServer::start().await;
    let calls = AtomicUsize::new(0);
    on(&server, "DELETE", DOC_PATH, move |_| {
        match calls.fetch_add(1, Ordering::SeqCst) {
            0 => error_body(503, "stubbed_exception"),
            _ => raw_json(404, DELETE_NOT_FOUND),
        }
    })
    .await;
    on(&server, "POST", TENANT_DBQ_PATH, |_| swept(0)).await;

    backend(&server)
        .delete(&tenant(), "Patient", "p1")
        .await
        .expect("absent after a resend is what the delete asked for");
    assert_eq!(requests_to(&server, "DELETE", DOC_PATH).await, 2);
}

/// `purge` stays idempotent: an absent document is nothing to undo, but a bare
/// `404` is still not evidence of that.
#[tokio::test]
async fn purge_is_idempotent_only_on_a_real_not_found() {
    let server = MockServer::start().await;
    on(&server, "DELETE", DOC_PATH, |_| {
        raw_json(404, DELETE_NOT_FOUND)
    })
    .await;
    on(&server, "POST", TENANT_DBQ_PATH, |_| swept(0)).await;
    backend(&server)
        .purge(&tenant(), "Patient", "p1")
        .await
        .expect("already absent");

    let server = MockServer::start().await;
    on(&server, "DELETE", DOC_PATH, |_| ResponseTemplate::new(404)).await;
    let error = backend(&server)
        .purge(&tenant(), "Patient", "p1")
        .await
        .expect_err("a bare 404 is not an absent document");
    assert!(is_internal(&error), "{error:?}");
}

// ---------------------------------------------------------------------------
// Contained-document maintenance
// ---------------------------------------------------------------------------

/// The issue's case (1): the sweep of a deleted container's contained documents
/// is answered `503`. The status used to be discarded (`let _ = …`), so the
/// delete reported success while the stale contained documents stayed in the
/// index, still matching `_contained` searches.
#[tokio::test]
async fn delete_surfaces_a_failed_contained_sweep() {
    let server = MockServer::start().await;
    on(&server, "DELETE", DOC_PATH, |_| doc_deleted()).await;
    on(&server, "POST", TENANT_DBQ_PATH, |_| {
        error_body(503, "unavailable_shards_exception")
    })
    .await;

    let error = backend(&server)
        .delete(&tenant(), "Patient", "p1")
        .await
        .expect_err("stale contained documents remain; that is not a successful delete");
    assert!(is_unavailable(&error), "{error:?}");
    assert!(
        detail(&error).contains("contained"),
        "the error names what is out of sync: {}",
        detail(&error)
    );
    assert_eq!(
        requests_to(&server, "POST", TENANT_DBQ_PATH).await,
        WRITE_ATTEMPTS
    );
}

/// The same sweep runs before an update re-indexes the contained documents;
/// there it is retried, and a permanent rejection fails the write.
#[tokio::test]
async fn the_contained_sweep_before_a_rewrite_is_retried_and_never_ignored() {
    let server = MockServer::start().await;
    accept_everything_but_the_index_request(&server).await;
    on(&server, "POST", DOC_PATH, |_| indexed()).await;
    on_prefix(&server, "POST", CONTAINED_DOC_PREFIX, |_| indexed()).await;
    run_index_write(&backend(&server), "create_or_update")
        .await
        .expect("control: the happy path works against these stubs");

    let server = MockServer::start().await;
    indices_exist(&server).await;
    on(&server, "GET", DOC_PATH, |_| {
        raw_json(
            404,
            r#"{"_index":"hfs_write-stub_patient","_id":"Patient_p1","found":false}"#,
        )
    })
    .await;
    on(&server, "POST", DOC_PATH, |_| indexed()).await;
    on(&server, "POST", TENANT_DBQ_PATH, flaky(swept(1))).await;
    run_index_write(&backend(&server), "create_or_update")
        .await
        .expect("a transient sweep failure must be retried");
    assert_eq!(
        requests_to(&server, "POST", TENANT_DBQ_PATH).await,
        WRITE_ATTEMPTS
    );

    for which in ["update", "create_or_update", "write_search_entries"] {
        let server = MockServer::start().await;
        indices_exist(&server).await;
        on(&server, "GET", DOC_PATH, |_| {
            raw_json(
                404,
                r#"{"_index":"hfs_write-stub_patient","_id":"Patient_p1","found":false}"#,
            )
        })
        .await;
        on(&server, "POST", DOC_PATH, |_| indexed()).await;
        on(&server, "POST", TENANT_DBQ_PATH, |_| {
            error_body(403, "security_exception")
        })
        .await;
        let error = run_index_write(&backend(&server), which)
            .await
            .expect_err("a rejected sweep leaves stale contained documents");
        assert!(is_internal(&error), "{which}: {error:?}");
        assert!(error.to_string().contains("contained"), "{which}: {error}");
        assert_eq!(
            requests_to(&server, "POST", TENANT_DBQ_PATH).await,
            1,
            "{which}"
        );
    }
}

/// Runs `which` with a Patient that contains an Organization.
async fn write_with_contained(es: &ElasticsearchBackend, which: &str) -> Result<(), StorageError> {
    let version = FhirVersion::default();
    let resource = patient_with_contained();
    match which {
        "create" => es
            .create(&tenant(), "Patient", resource, version)
            .await
            .map(|_| ()),
        "update" => es
            .update(&tenant(), &stored(plain_patient()), resource)
            .await
            .map(|_| ()),
        "create_or_update" => es
            .create_or_update(&tenant(), "Patient", "p1", resource, version)
            .await
            .map(|_| ()),
        "write_search_entries" => es
            .write_search_entries(&tenant(), &stored(resource))
            .await
            .map(|_| ()),
        other => panic!("unknown write path {other}"),
    }
}

/// Every write path keeps the contained documents in step with the parent:
/// each indexes the contained Organization (positive control for the tests
/// below — without the full parameter set nothing would be written), and each
/// retries a transient failure of that request. `update` used to skip contained
/// maintenance altogether.
#[tokio::test]
async fn contained_documents_are_indexed_by_every_write_path_with_retries() {
    for which in INDEX_WRITES {
        let server = MockServer::start().await;
        accept_everything_but_the_index_request(&server).await;
        on(&server, "POST", DOC_PATH, |_| indexed()).await;
        on_prefix(&server, "POST", CONTAINED_DOC_PREFIX, flaky(indexed())).await;

        write_with_contained(&backend(&server), which)
            .await
            .unwrap_or_else(|e| panic!("{which}: {e:?}"));

        let contained = requests_under(&server, "POST", CONTAINED_DOC_PREFIX).await;
        assert_eq!(contained.len(), WRITE_ATTEMPTS, "{which}: {contained:?}");
        assert!(
            contained[0].contains("Organization_p1") && contained[0].contains("org1"),
            "{which}: {contained:?}"
        );
        // A rewrite sweeps the container's previous contained documents first;
        // a create has none.
        let sweeps = requests_to(&server, "POST", TENANT_DBQ_PATH).await;
        assert_eq!(sweeps, usize::from(which != "create"), "{which}");
    }
}

/// A contained document that cannot be indexed fails the write, and the error
/// says the parent is already in the index.
#[tokio::test]
async fn a_failed_contained_document_fails_the_write_and_says_so() {
    for which in INDEX_WRITES {
        let server = MockServer::start().await;
        accept_everything_but_the_index_request(&server).await;
        on(&server, "POST", DOC_PATH, |_| indexed()).await;
        on_prefix(&server, "POST", CONTAINED_DOC_PREFIX, |_| {
            error_body(503, "unavailable_shards_exception")
        })
        .await;

        let error = write_with_contained(&backend(&server), which)
            .await
            .expect_err("parent and contained documents are out of step");
        assert!(is_unavailable(&error), "{which}: {error:?}");
        assert!(
            detail(&error).contains("contained"),
            "{which}: {}",
            detail(&error)
        );
        assert_eq!(
            requests_under(&server, "POST", CONTAINED_DOC_PREFIX)
                .await
                .len(),
            WRITE_ATTEMPTS,
            "{which}"
        );
    }
}

// ---------------------------------------------------------------------------
// delete-by-query: purge_all, clear_search_index, purge_tenant_data
// ---------------------------------------------------------------------------

/// The three tenant/type purges, by name. Each returns the deleted count.
async fn run_purge(es: &ElasticsearchBackend, which: &str) -> Result<u64, StorageError> {
    match which {
        "purge_all" => es.purge_all(&tenant(), "Patient").await,
        "clear_search_index" => es.clear_search_index(&tenant()).await,
        "purge_tenant_data" => es.purge_tenant_data(TENANT).await,
        other => panic!("unknown purge {other}"),
    }
}

/// The path whose answer becomes the purge's return value.
fn counted_path(which: &str) -> &'static str {
    match which {
        "purge_all" => TYPE_DBQ_PATH,
        _ => TENANT_DBQ_PATH,
    }
}

const PURGES: [&str; 3] = ["purge_all", "clear_search_index", "purge_tenant_data"];

#[tokio::test]
async fn purges_retry_transient_failures_then_report_the_count() {
    for which in PURGES {
        let server = MockServer::start().await;
        on(&server, "POST", counted_path(which), flaky(swept(7))).await;
        if which == "purge_all" {
            on(&server, "POST", TENANT_DBQ_PATH, |_| swept(1)).await;
        }
        let deleted = run_purge(&backend(&server), which)
            .await
            .unwrap_or_else(|e| panic!("{which}: a transient failure must be retried: {e:?}"));
        assert_eq!(deleted, 7, "{which}");
        assert_eq!(
            requests_to(&server, "POST", counted_path(which)).await,
            WRITE_ATTEMPTS,
            "{which}"
        );
    }
}

/// The issue's case (4): a `200` that does not say how many documents were
/// deleted is not a purge of `0` documents — and `purge_tenant_data` did not
/// look at the status at all, so any failure was a purge of `0` too.
#[tokio::test]
async fn purges_never_report_a_failure_or_an_unreadable_answer_as_zero() {
    for which in PURGES {
        for answer in [
            ResponseTemplate::new(200).set_body_json(json!({ "took": 1 })),
            ResponseTemplate::new(200).set_body_string("<html>ok</html>"),
            error_body(403, "security_exception"),
            ResponseTemplate::new(404),
        ] {
            let server = MockServer::start().await;
            let respond = answer.clone();
            on(&server, "POST", counted_path(which), move |_| {
                respond.clone()
            })
            .await;
            if which == "purge_all" {
                // Its second sweep succeeds, so only the first can fail it.
                on(&server, "POST", TENANT_DBQ_PATH, |_| swept(0)).await;
            }
            let error = run_purge(&backend(&server), which)
                .await
                .expect_err("not a purge of 0 documents");
            assert!(is_internal(&error), "{which}: {error:?}");
            assert_eq!(
                requests_to(&server, "POST", counted_path(which)).await,
                1,
                "{which}"
            );
        }

        let server = MockServer::start().await;
        on(&server, "POST", counted_path(which), |_| {
            error_body(503, "unavailable_shards_exception")
        })
        .await;
        let error = run_purge(&backend(&server), which)
            .await
            .expect_err("an exhausted retry is an error");
        assert!(is_unavailable(&error), "{which}: {error:?}");
        assert_eq!(
            requests_to(&server, "POST", counted_path(which)).await,
            WRITE_ATTEMPTS,
            "{which}"
        );
    }
}

/// A delete-by-query that skipped documents (a concurrent write changed them
/// under it, or a shard failed) answers `200`. It is repeat-safe, so it is run
/// again; the counts add up, and documents still left behind are an error.
#[tokio::test]
async fn an_incomplete_delete_by_query_is_run_again() {
    let server = MockServer::start().await;
    let calls = AtomicUsize::new(0);
    on(&server, "POST", TENANT_DBQ_PATH, move |_| {
        match calls.fetch_add(1, Ordering::SeqCst) {
            0 => ResponseTemplate::new(200).set_body_json(json!({
                "deleted": 5, "version_conflicts": 2, "failures": []
            })),
            _ => swept(2),
        }
    })
    .await;
    let deleted = backend(&server)
        .clear_search_index(&tenant())
        .await
        .expect("the second pass removes what the first skipped");
    assert_eq!(deleted, 7);
    assert_eq!(requests_to(&server, "POST", TENANT_DBQ_PATH).await, 2);

    let server = MockServer::start().await;
    on(&server, "POST", TENANT_DBQ_PATH, |_| {
        ResponseTemplate::new(200).set_body_json(json!({
            "deleted": 0, "version_conflicts": 0,
            "failures": [{ "index": "hfs_write-stub_patient", "status": 500 }]
        }))
    })
    .await;
    let error = backend(&server)
        .clear_search_index(&tenant())
        .await
        .expect_err("documents were left behind");
    assert!(is_unavailable(&error), "{error:?}");
    assert_eq!(
        requests_to(&server, "POST", TENANT_DBQ_PATH).await,
        WRITE_ATTEMPTS
    );
}

// ---------------------------------------------------------------------------
// HealthMonitor::check_backend
// ---------------------------------------------------------------------------

/// The issue's case (5): a probe whose `count` fails used to read as healthy.
#[tokio::test]
async fn a_failed_health_probe_reads_as_unhealthy() {
    // Control: the probe's index never exists, and Elasticsearch saying so is
    // an answer from a working cluster.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex("^/.*__health_check__/_count$"))
        .respond_with(|_: &Request| raw_json(404, INDEX_NOT_FOUND))
        .mount(&server)
        .await;
    let result = HealthMonitor::check_backend(&backend(&server), Duration::from_secs(5)).await;
    assert!(
        matches!(result, HealthCheckResult::Healthy { .. }),
        "{result:?}"
    );

    // A cluster that keeps answering 503 is not healthy. The three attempts and
    // their ~300 ms of back-off fit the 5 s default probe timeout with room.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex("^/.*__health_check__/_count$"))
        .respond_with(|_: &Request| error_body(503, "unavailable_shards_exception"))
        .mount(&server)
        .await;
    let result = HealthMonitor::check_backend(&backend(&server), Duration::from_secs(5)).await;
    assert!(
        matches!(&result, HealthCheckResult::Unhealthy { error } if error.contains("503")),
        "{result:?}"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 3);

    // Nor is one that cannot be reached. The probe timeout leaves room for
    // three refused connects on a platform that is slow to refuse one (see
    // `unreachable_config`): a probe that runs out of time is `Timeout`, a
    // different answer from the one asserted here.
    let unreachable = ElasticsearchBackend::new(unreachable_config()).unwrap();
    let result = HealthMonitor::check_backend(&unreachable, Duration::from_secs(30)).await;
    assert!(
        matches!(result, HealthCheckResult::Unhealthy { .. }),
        "{result:?}"
    );
}
