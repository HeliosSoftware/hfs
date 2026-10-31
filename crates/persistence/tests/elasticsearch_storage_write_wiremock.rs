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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use helios_fhir::FhirVersion;
use helios_persistence::backends::elasticsearch::{ElasticsearchBackend, ElasticsearchConfig};
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynStorage, HealthCheckResult, HealthMonitor, SyncMode,
};
use helios_persistence::core::{
    BackendKind, BundleEntry, BundleMethod, BundleProvider, PurgableStorage, ResourceStorage,
    VersionedStorage,
};
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

const ORG_DOC_ID: &str = "Organization_p1#org1";
const ORG_INDEX: &str = "hfs_write-stub_organization";

/// The container is not in the index: the GET before a write finds nothing.
fn container_absent() -> ResponseTemplate {
    raw_json(
        404,
        r#"{"_index":"hfs_write-stub_patient","_id":"Patient_p1","found":false}"#,
    )
}

/// The container is indexed and records the `_id`s of its contained documents.
fn container_known(contained_doc_ids: &[&str]) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "_index": "hfs_write-stub_patient", "_id": "Patient_p1", "_version": 3,
        "_seq_no": 7, "_primary_term": 2, "found": true,
        "_source": {
            "tenant_id": TENANT, "version_id": "1",
            "contained_doc_ids": contained_doc_ids
        }
    }))
}

/// The container was indexed before `contained_doc_ids` existed.
fn container_legacy() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "_index": "hfs_write-stub_patient", "_id": "Patient_p1", "_version": 3,
        "_seq_no": 7, "_primary_term": 2, "found": true,
        "_source": { "tenant_id": TENANT, "version_id": "1" }
    }))
}

/// An `_mget` answer on which none of the requested documents exists.
fn mget_nothing_found(request: &Request) -> ResponseTemplate {
    let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
    let docs: Vec<Value> = body["docs"]
        .as_array()
        .map(|docs| {
            docs.iter()
                .map(|d| json!({"_index": d["_index"], "_id": d["_id"], "found": false}))
                .collect()
        })
        .unwrap_or_default();
    ResponseTemplate::new(200).set_body_json(json!({ "docs": docs }))
}

/// The operations of a `_bulk` body, in order: `(kind, action metadata)`.
/// An `index` action is followed by its document line, a `delete` is not.
fn bulk_actions(request: &Request) -> Vec<(String, Value)> {
    let body = String::from_utf8_lossy(&request.body).into_owned();
    let mut lines = body.lines();
    let mut actions = Vec::new();
    while let Some(line) = lines.next() {
        let action: Value = serde_json::from_str(line).expect("an action line");
        let (kind, meta) = action
            .as_object()
            .and_then(|o| o.iter().next())
            .map(|(k, v)| (k.clone(), v.clone()))
            .expect("an action");
        if kind == "index" {
            lines.next();
        }
        actions.push((kind, meta));
    }
    actions
}

/// A `_bulk` answer on which every operation succeeded.
fn bulk_ack(request: &Request) -> ResponseTemplate {
    let items: Vec<Value> = bulk_actions(request)
        .into_iter()
        .map(|(kind, _)| {
            let status = if kind == "delete" { 200 } else { 201 };
            json!({ kind: { "status": status } })
        })
        .collect();
    ResponseTemplate::new(200).set_body_json(json!({"took": 1, "errors": false, "items": items}))
}

/// The `_bulk` requests the stub received.
async fn bulk_requests(server: &MockServer) -> Vec<Request> {
    server
        .received_requests()
        .await
        .expect("request recording is on")
        .into_iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path() == "/_bulk")
        .collect()
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

/// The detail of an error: for `Unavailable`, its message without the backend
/// prefix.
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

/// The JSON bodies of the `POST {DOC_PATH}` index requests the stub received.
async fn indexed_bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .expect("request recording is on")
        .iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path() == DOC_PATH)
        .map(|r| serde_json::from_slice(&r.body).expect("an index request carries a JSON document"))
        .collect()
}

/// #1593: the document a `$reindex` writes carries the resource's own
/// `last_updated`, not the time of the rebuild. The two differ by however
/// long ago the resource was stored, so a rebuild used to make every resource
/// of the type answer `_lastUpdated` searches as if modified at the rebuild.
#[tokio::test]
async fn write_search_entries_indexes_the_resources_own_last_updated() {
    let server = MockServer::start().await;
    accept_everything_but_the_index_request(&server).await;
    on(&server, "POST", DOC_PATH, |_| indexed()).await;

    let stored_at = chrono::DateTime::parse_from_rfc3339("2026-09-28T19:39:53.005Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let resource = StoredResource::from_storage(
        "Patient",
        "p1",
        "4",
        TenantId::new(TENANT),
        plain_patient(),
        stored_at,
        stored_at,
        None,
        FhirVersion::default(),
    );
    backend(&server)
        .write_search_entries(&tenant(), &resource)
        .await
        .expect("the stub accepts the write");

    let bodies = indexed_bodies(&server).await;
    assert_eq!(bodies.len(), 1, "one index request");
    assert_eq!(
        bodies[0]["last_updated"],
        json!("2026-09-28T19:39:53.005+00:00"),
        "the resource's instant, not the rebuild's"
    );
    assert_eq!(bodies[0]["version_id"], json!("4"));
}

/// #1593, the write side: the instant a create indexes as `last_updated` is
/// the one it returns as the stored resource's `meta.lastUpdated`, so the
/// index and the resource never disagree even by the write's own duration.
#[tokio::test]
async fn create_indexes_the_instant_it_returns_as_last_modified() {
    let server = MockServer::start().await;
    accept_everything_but_the_index_request(&server).await;
    on(&server, "POST", DOC_PATH, |_| indexed()).await;

    let created = backend(&server)
        .create(
            &tenant(),
            "Patient",
            plain_patient(),
            FhirVersion::default(),
        )
        .await
        .expect("the stub accepts the write");

    let bodies = indexed_bodies(&server).await;
    assert_eq!(bodies.len(), 1, "one index request");
    assert_eq!(
        bodies[0]["last_updated"],
        json!(created.last_modified().to_rfc3339()),
        "the indexed instant is the one the caller is told"
    );
}

fn patient_from_primary(id: &str, version: &str, last_updated: &str) -> Value {
    json!({
        "resourceType": "Patient",
        "id": id,
        "name": [{ "family": "Stub" }],
        "meta": { "versionId": version, "lastUpdated": last_updated }
    })
}

const PRIMARY_INSTANT: &str = "2026-09-28T19:39:53.005Z";
const PRIMARY_INSTANT_INDEXED: &str = "2026-09-28T19:39:53.005+00:00";

/// create_or_update with the primary's meta skips the existence GET and
/// indexes that version and lastUpdated.
#[tokio::test]
async fn create_or_update_indexes_the_primarys_version_and_last_updated() {
    let server = MockServer::start().await;
    indices_exist(&server).await;
    on(&server, "GET", DOC_PATH, |_| container_absent()).await;
    on(&server, "POST", TENANT_DBQ_PATH, |_| swept(0)).await;
    on(&server, "POST", DOC_PATH, |_| indexed()).await;

    let es = backend(&server);
    let (stored, is_new) = es
        .create_or_update(
            &tenant(),
            "Patient",
            "p1",
            patient_from_primary("p1", "4", PRIMARY_INSTANT),
            FhirVersion::default(),
        )
        .await
        .expect("the stub accepts the write");

    assert_eq!(stored.version_id(), "4");
    assert!(!is_new, "the primary's version 4 is an update");
    assert_eq!(stored.last_modified().to_rfc3339(), PRIMARY_INSTANT_INDEXED);
    let body = &indexed_bodies(&server).await[0];
    assert_eq!(body["version_id"], json!("4"));
    assert_eq!(body["last_updated"], json!(PRIMARY_INSTANT_INDEXED));
    let (_, is_new) = es
        .create_or_update(
            &tenant(),
            "Patient",
            "p1",
            patient_from_primary("p1", "1", PRIMARY_INSTANT),
            FhirVersion::default(),
        )
        .await
        .expect("the stub accepts the write");
    assert!(is_new, "the primary's version 1 is a create");
    // The only GET of a write is the lookup of what the container records;
    // the primary's metadata makes a separate version lookup unnecessary.
    assert_eq!(requests_to(&server, "GET", DOC_PATH).await, 2);
    assert_eq!(requests_to(&server, "POST", TENANT_DBQ_PATH).await, 0);
}

/// The single-document path a resource falls back to when its batch fails.
#[tokio::test]
async fn create_indexes_the_primarys_version_and_last_updated() {
    let server = MockServer::start().await;
    accept_everything_but_the_index_request(&server).await;
    on(&server, "POST", DOC_PATH, |_| indexed()).await;

    let stored = backend(&server)
        .create(
            &tenant(),
            "Patient",
            patient_from_primary("p1", "3", PRIMARY_INSTANT),
            FhirVersion::default(),
        )
        .await
        .expect("the stub accepts the write");

    assert_eq!(stored.version_id(), "3");
    let body = &indexed_bodies(&server).await[0];
    assert_eq!(body["version_id"], json!("3"));
    assert_eq!(body["last_updated"], json!(PRIMARY_INSTANT_INDEXED));
}

/// The batch a transaction bundle's results sync through.
#[tokio::test]
async fn write_search_entries_page_indexes_each_resources_primary_version() {
    let server = MockServer::start().await;
    indices_exist(&server).await;
    Mock::given(method("POST"))
        .and(path("/_mget"))
        .respond_with(mget_nothing_found)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/_bulk"))
        .respond_with(|request: &Request| {
            let n = String::from_utf8_lossy(&request.body).lines().count() / 2;
            let items: Vec<Value> = (0..n)
                .map(|_| json!({"index": {"status": 200, "result": "updated"}}))
                .collect();
            ResponseTemplate::new(200)
                .set_body_json(json!({"took": 1, "errors": false, "items": items}))
        })
        .mount(&server)
        .await;

    let first = StoredResource::from_storage(
        "Patient",
        "p1",
        "3",
        TenantId::new(TENANT),
        patient_from_primary("p1", "3", PRIMARY_INSTANT),
        chrono::DateTime::parse_from_rfc3339(PRIMARY_INSTANT)
            .unwrap()
            .with_timezone(&chrono::Utc),
        chrono::DateTime::parse_from_rfc3339(PRIMARY_INSTANT)
            .unwrap()
            .with_timezone(&chrono::Utc),
        None,
        FhirVersion::default(),
    );
    let later = chrono::DateTime::parse_from_rfc3339("2026-09-28T20:00:00.000Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let second = StoredResource::from_storage(
        "Patient",
        "p2",
        "6",
        TenantId::new(TENANT),
        patient_from_primary("p2", "6", "2026-09-28T20:00:00.000Z"),
        later,
        later,
        None,
        FhirVersion::default(),
    );
    let results = backend(&server)
        .write_search_entries_page(&tenant(), &[first, second])
        .await;

    assert!(results.iter().all(|r| r.is_ok()), "{results:?}");
    let request = server
        .received_requests()
        .await
        .expect("request recording is on")
        .into_iter()
        .find(|r| r.url.path() == "/_bulk")
        .expect("one _bulk request");
    let indexed: Vec<Value> = String::from_utf8_lossy(&request.body)
        .lines()
        .skip(1)
        .step_by(2)
        .map(|line| serde_json::from_str::<Value>(line).expect("a document line"))
        .collect();
    assert_eq!(
        indexed
            .iter()
            .map(|doc| doc["version_id"].clone())
            .collect::<Vec<_>>(),
        [json!("3"), json!("6")]
    );
    assert_eq!(indexed[0]["last_updated"], json!(PRIMARY_INSTANT_INDEXED));
    assert_eq!(
        indexed[1]["last_updated"],
        json!("2026-09-28T20:00:00+00:00")
    );
}

/// A search-index write with an empty primary version is refused.
#[tokio::test]
async fn write_search_entries_rejects_a_missing_primary_version() {
    let server = MockServer::start().await;
    let now = chrono::Utc::now();
    let resource = StoredResource::from_storage(
        "Patient",
        "p1",
        "",
        TenantId::new(TENANT),
        plain_patient(),
        now,
        now,
        None,
        FhirVersion::default(),
    );
    let error = backend(&server)
        .write_search_entries(&tenant(), &resource)
        .await
        .expect_err("a missing primary version must not be indexed as 1");
    assert!(error.to_string().contains("versionId"), "{error}");
    assert_eq!(requests_to(&server, "POST", DOC_PATH).await, 0);
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
        // The client text names the lookup that precedes the write (#2291, #2385): Elasticsearch's answer,
        // which names the error type and the index, goes to the log.
        assert!(
            detail(&error)
                .ends_with("the search index is temporarily unavailable; retry the request"),
            "{status} {error_type}: {}",
            detail(&error)
        );
        for leaked in [error_type, "hfs_write-stub_patient"] {
            assert!(
                !detail(&error).contains(leaked),
                "{status} {error_type}: {leaked} leaked: {}",
                detail(&error)
            );
        }
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
    // The client text names the write only (#2385): the transport error names
    // the node and the document path, and goes to the log.
    let message = detail(&error);
    assert_eq!(
        message,
        "Failed to look up the contained documents of Patient/p1 after 3 attempts: the search index is temporarily unavailable; retry the request"
    );
    for leaked in ["127.0.0.1", "hfs_write-stub"] {
        assert!(!message.contains(leaked), "{leaked} leaked: {message}");
    }
}

/// A cluster that accepts the connection and then does not answer is not resent
/// (each resend would wait out another full request timeout). The write says so
/// with the operation and the timeout only: the transport error, which names the
/// node, the index and the document path, goes to the log (#2385).
#[tokio::test]
async fn an_index_write_that_gets_no_answer_is_unavailable_and_names_no_url() {
    let server = MockServer::start().await;
    accept_everything_but_the_index_request(&server).await;
    // `backend` allows 2000 ms; the answer takes longer.
    on(&server, "POST", DOC_PATH, |_| {
        ResponseTemplate::new(201).set_delay(Duration::from_secs(4))
    })
    .await;

    let error = run_index_write(&backend(&server), "create")
        .await
        .expect_err("no answer is not a successful write");
    assert!(is_unavailable(&error), "{error:?}");
    let message = detail(&error);
    assert!(message.starts_with("Failed to "), "{message}");
    assert!(message.contains("no answer within 2000 ms"), "{message}");
    assert!(
        message.ends_with(": the search index is temporarily unavailable; retry the request"),
        "{message}"
    );
    for leaked in [
        server.address().to_string().as_str(),
        "hfs_write-stub_patient",
        "_doc",
    ] {
        assert!(!message.contains(leaked), "{leaked} leaked: {message}");
    }
    // A request timeout is never resent.
    assert_eq!(requests_to(&server, "POST", DOC_PATH).await, 1);
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
    // A container that records no contained documents needs no sweep.
    let server = MockServer::start().await;
    on(&server, "GET", DOC_PATH, |_| container_known(&[])).await;
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
    assert_eq!(requests_to(&server, "POST", TENANT_DBQ_PATH).await, 0);

    // One that predates the record may have some: the sweep runs once.
    let server = MockServer::start().await;
    on(&server, "GET", DOC_PATH, |_| container_legacy()).await;
    on(&server, "DELETE", DOC_PATH, flaky(doc_deleted())).await;
    on(&server, "POST", TENANT_DBQ_PATH, |_| swept(0)).await;
    backend(&server)
        .delete(&tenant(), "Patient", "p1")
        .await
        .expect("a transient failure must be retried");
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
        on(&server, "GET", DOC_PATH, |_| container_absent()).await;
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
        on(&server, "GET", DOC_PATH, |_| container_absent()).await;
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
    on(&server, "GET", DOC_PATH, |_| container_absent()).await;
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
    on(&server, "GET", DOC_PATH, |_| container_absent()).await;
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
    on(&server, "GET", DOC_PATH, |_| container_absent()).await;
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
    on(&server, "GET", DOC_PATH, |_| container_absent()).await;
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
    // The container predates the record, so only a sweep can find its contained
    // documents.
    on(&server, "GET", DOC_PATH, |_| container_legacy()).await;
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
    // The sweep is repeated on its own: 4 rounds of the write retries.
    assert_eq!(
        requests_to(&server, "POST", TENANT_DBQ_PATH).await,
        WRITE_ATTEMPTS * 4
    );
}

/// The sweep is the repair path now: a rewrite runs it only for a container
/// indexed before `contained_doc_ids` existed, whose contained documents can
/// only be found that way. There it is retried, and a permanent rejection fails
/// the write.
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
    on(&server, "GET", DOC_PATH, |_| container_legacy()).await;
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
        on(&server, "GET", DOC_PATH, |_| container_legacy()).await;
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
/// each writes the contained Organization (positive control for the tests
/// below — without the full parameter set nothing would be written), and each
/// retries a transient failure of that request. A create writes it with a
/// request of its own; a rewrite puts it in the same `_bulk` as the container,
/// and finds the previous ones from the container, not with a sweep.
#[tokio::test]
async fn contained_documents_are_indexed_by_every_write_path_with_retries() {
    for which in INDEX_WRITES {
        let server = MockServer::start().await;
        accept_everything_but_the_index_request(&server).await;
        on(&server, "POST", DOC_PATH, |_| indexed()).await;
        on_prefix(&server, "POST", CONTAINED_DOC_PREFIX, flaky(indexed())).await;
        let throttled_once = AtomicUsize::new(0);
        Mock::given(method("POST"))
            .and(path("/_bulk"))
            .respond_with(move |request: &Request| {
                if throttled_once.fetch_add(1, Ordering::SeqCst) == 0 {
                    error_body(429, "es_rejected_execution_exception")
                } else {
                    bulk_ack(request)
                }
            })
            .mount(&server)
            .await;

        write_with_contained(&backend(&server), which)
            .await
            .unwrap_or_else(|e| panic!("{which}: {e:?}"));

        if which == "create" {
            let contained = requests_under(&server, "POST", CONTAINED_DOC_PREFIX).await;
            assert_eq!(contained.len(), WRITE_ATTEMPTS, "{which}: {contained:?}");
            assert!(
                contained[0].contains("Organization_p1") && contained[0].contains("org1"),
                "{which}: {contained:?}"
            );
        } else {
            let bulks = bulk_requests(&server).await;
            assert_eq!(bulks.len(), 2, "{which}: throttled once, then resent");
            let actions = bulk_actions(&bulks[1]);
            assert!(
                actions
                    .iter()
                    .any(|(kind, meta)| kind == "index" && meta["_id"] == json!(ORG_DOC_ID)),
                "{which}: {actions:?}"
            );
            assert_eq!(
                actions.last().map(|(_, meta)| meta["_id"].clone()),
                Some(json!("Patient_p1")),
                "{which}: the container is written after its contained documents"
            );
        }
        // Nothing here needs the sweep: the container is new or records `[]`.
        assert_eq!(
            requests_to(&server, "POST", TENANT_DBQ_PATH).await,
            0,
            "{which}"
        );
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
        on(&server, "POST", "/_bulk", |_| {
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
        if which == "create" {
            assert_eq!(
                requests_under(&server, "POST", CONTAINED_DOC_PREFIX)
                    .await
                    .len(),
                WRITE_ATTEMPTS,
                "{which}"
            );
        } else {
            assert!(!bulk_requests(&server).await.is_empty(), "{which}");
        }
    }
}

/// The point of recording the contained documents: an update or a delete of a
/// resource that has none runs no tenant-wide `_delete_by_query`, whether the
/// container is new to the index or records an empty list.
#[tokio::test]
async fn a_write_of_a_resource_without_contained_runs_no_sweep() {
    for which in ["update", "create_or_update", "write_search_entries"] {
        for known in [false, true] {
            let server = MockServer::start().await;
            indices_exist(&server).await;
            on(&server, "GET", DOC_PATH, move |_| {
                if known {
                    container_known(&[])
                } else {
                    container_absent()
                }
            })
            .await;
            on(&server, "POST", DOC_PATH, |_| indexed()).await;
            on(&server, "POST", TENANT_DBQ_PATH, |_| swept(0)).await;

            run_index_write(&backend(&server), which)
                .await
                .unwrap_or_else(|e| panic!("{which}: {e:?}"));

            assert_eq!(
                requests_to(&server, "POST", TENANT_DBQ_PATH).await,
                0,
                "{which} (known: {known})"
            );
            assert_eq!(requests_to(&server, "POST", DOC_PATH).await, 1, "{which}");
            assert!(bulk_requests(&server).await.is_empty(), "{which}");
            let body = &indexed_bodies(&server).await[0];
            assert_eq!(
                body["contained_doc_ids"],
                json!([]),
                "{which}: the container records that it has none"
            );
        }
    }
}

/// A container indexed before the record existed is swept once; the document
/// it is rewritten as records its contained documents from then on.
#[tokio::test]
async fn a_container_that_predates_the_record_is_swept_once() {
    for which in ["update", "create_or_update", "write_search_entries"] {
        let server = MockServer::start().await;
        indices_exist(&server).await;
        on(&server, "GET", DOC_PATH, |_| container_legacy()).await;
        on(&server, "POST", DOC_PATH, |_| indexed()).await;
        on(&server, "POST", TENANT_DBQ_PATH, |_| swept(1)).await;

        run_index_write(&backend(&server), which)
            .await
            .unwrap_or_else(|e| panic!("{which}: {e:?}"));

        assert_eq!(
            requests_to(&server, "POST", TENANT_DBQ_PATH).await,
            1,
            "{which}"
        );
        assert_eq!(
            indexed_bodies(&server).await[0]["contained_doc_ids"],
            json!([]),
            "{which}"
        );
    }
}

/// An update that drops a contained resource deletes its document by id, in
/// the same `_bulk` that rewrites the container under the sequence number it
/// was read at. No sweep.
#[tokio::test]
async fn an_update_deletes_the_contained_documents_it_dropped_by_id_in_one_bulk() {
    for which in ["update", "create_or_update", "write_search_entries"] {
        let server = MockServer::start().await;
        indices_exist(&server).await;
        on(&server, "GET", DOC_PATH, |_| container_known(&[ORG_DOC_ID])).await;
        on(&server, "POST", TENANT_DBQ_PATH, |_| swept(0)).await;
        on(&server, "POST", "/_bulk", bulk_ack).await;

        // The resource no longer contains the Organization.
        run_index_write(&backend(&server), which)
            .await
            .unwrap_or_else(|e| panic!("{which}: {e:?}"));

        let bulks = bulk_requests(&server).await;
        assert_eq!(bulks.len(), 1, "{which}");
        let actions = bulk_actions(&bulks[0]);
        assert_eq!(actions.len(), 2, "{which}: {actions:?}");
        assert_eq!(actions[0].0, "delete", "{which}");
        assert_eq!(actions[0].1["_id"], json!(ORG_DOC_ID), "{which}");
        assert_eq!(actions[0].1["_index"], json!(ORG_INDEX), "{which}");
        assert_eq!(actions[1].0, "index", "{which}");
        assert_eq!(actions[1].1["_id"], json!("Patient_p1"), "{which}");
        assert_eq!(actions[1].1["if_seq_no"], json!(7), "{which}");
        assert_eq!(actions[1].1["if_primary_term"], json!(2), "{which}");
        assert_eq!(
            requests_to(&server, "POST", TENANT_DBQ_PATH).await,
            0,
            "{which}"
        );
    }
}

/// A delete of a container that records contained documents removes them and
/// the container in one `_bulk`, with no sweep and no single-document delete.
#[tokio::test]
async fn a_delete_removes_the_recorded_contained_documents_in_one_bulk() {
    let server = MockServer::start().await;
    on(&server, "GET", DOC_PATH, |_| container_known(&[ORG_DOC_ID])).await;
    on(&server, "POST", TENANT_DBQ_PATH, |_| swept(0)).await;
    on(&server, "POST", "/_bulk", bulk_ack).await;

    backend(&server)
        .delete(&tenant(), "Patient", "p1")
        .await
        .expect("the stub accepts the bulk");

    let bulks = bulk_requests(&server).await;
    assert_eq!(bulks.len(), 1);
    let actions = bulk_actions(&bulks[0]);
    assert_eq!(
        actions
            .iter()
            .map(|(kind, meta)| (kind.as_str(), meta["_id"].as_str().unwrap_or_default()))
            .collect::<Vec<_>>(),
        [("delete", ORG_DOC_ID), ("delete", "Patient_p1")]
    );
    assert_eq!(requests_to(&server, "DELETE", DOC_PATH).await, 0);
    assert_eq!(requests_to(&server, "POST", TENANT_DBQ_PATH).await, 0);
}

/// Someone rewrote the container since it was read (`409` on the guarded
/// index operation), so what it records is unknown: sweep, then write it again
/// without the guard.
#[tokio::test]
async fn a_conflict_on_the_container_sweeps_and_rewrites_it_unguarded() {
    let server = MockServer::start().await;
    indices_exist(&server).await;
    on(&server, "GET", DOC_PATH, |_| container_known(&[ORG_DOC_ID])).await;
    on(&server, "POST", TENANT_DBQ_PATH, |_| swept(1)).await;
    let calls = AtomicUsize::new(0);
    on(&server, "POST", "/_bulk", move |request| {
        if calls.fetch_add(1, Ordering::SeqCst) > 0 {
            return bulk_ack(request);
        }
        // The delete succeeds; the guarded container is refused.
        ResponseTemplate::new(200).set_body_json(json!({
            "took": 1, "errors": true,
            "items": [
                { "delete": { "status": 200 } },
                { "index": { "status": 409, "error": {
                    "type": "version_conflict_engine_exception",
                    "reason": "version conflict, required seqNo [7]"
                } } }
            ]
        }))
    })
    .await;

    run_index_write(&backend(&server), "update")
        .await
        .expect("the conflict is repaired and the write completed");

    assert_eq!(requests_to(&server, "POST", TENANT_DBQ_PATH).await, 1);
    let bulks = bulk_requests(&server).await;
    assert_eq!(bulks.len(), 2);
    let retry = bulk_actions(&bulks[1]);
    let container = retry.last().expect("the container");
    assert_eq!(container.1["_id"], json!("Patient_p1"));
    assert!(
        container.1.get("if_seq_no").is_none() && container.1.get("if_primary_term").is_none(),
        "{container:?}"
    );
}

/// `_bulk` is not atomic: if the container was rewritten but the delete of a
/// recorded document failed, the container no longer lists it. The write
/// sweeps before it reports the error, so the document cannot be stranded.
#[tokio::test]
async fn a_failed_delete_of_a_recorded_document_is_swept_before_the_error() {
    let server = MockServer::start().await;
    indices_exist(&server).await;
    on(&server, "GET", DOC_PATH, |_| container_known(&[ORG_DOC_ID])).await;
    on(&server, "POST", TENANT_DBQ_PATH, |_| swept(1)).await;
    on(&server, "POST", "/_bulk", |_| {
        ResponseTemplate::new(200).set_body_json(json!({
            "took": 1, "errors": true,
            "items": [
                { "delete": { "status": 500, "error": {
                    "type": "unavailable_shards_exception", "reason": "stubbed"
                } } },
                { "index": { "status": 200 } }
            ]
        }))
    })
    .await;

    let error = run_index_write(&backend(&server), "update")
        .await
        .expect_err("a recorded document could not be deleted");
    assert!(error.to_string().contains("contained"), "{error}");
    assert_eq!(requests_to(&server, "POST", TENANT_DBQ_PATH).await, 1);
}

/// If that sweep fails too, the container is rewritten without its record, so
/// the next write of it takes the sweep path instead of trusting a list that
/// no longer matches the index.
#[tokio::test]
async fn when_the_repair_sweep_fails_the_container_is_unlisted() {
    let server = MockServer::start().await;
    indices_exist(&server).await;
    on(&server, "GET", DOC_PATH, |_| container_known(&[ORG_DOC_ID])).await;
    on(&server, "POST", TENANT_DBQ_PATH, |_| {
        error_body(403, "security_exception")
    })
    .await;
    on(&server, "POST", DOC_PATH, |_| indexed()).await;
    on(&server, "POST", "/_bulk", |_| {
        ResponseTemplate::new(200).set_body_json(json!({
            "took": 1, "errors": true,
            "items": [
                { "delete": { "status": 500, "error": {
                    "type": "unavailable_shards_exception", "reason": "stubbed"
                } } },
                { "index": { "status": 200 } }
            ]
        }))
    })
    .await;

    run_index_write(&backend(&server), "update")
        .await
        .expect_err("a recorded document could not be deleted");

    let rewrites = indexed_bodies(&server).await;
    assert_eq!(rewrites.len(), 1, "the container is rewritten once");
    assert!(
        rewrites[0].get("contained_doc_ids").is_none(),
        "{}",
        rewrites[0]
    );
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
    let HealthCheckResult::Unhealthy { error } = result else {
        panic!("expected an unhealthy backend, got {result:?}");
    };
    assert!(
        error.starts_with("backend unavailable: elasticsearch: "),
        "{error}"
    );
    // The reason appears once: the health result does not append it again. It
    // is the client text (#2385), so it names no node.
    assert_eq!(
        error
            .matches("the search index is temporarily unavailable")
            .count(),
        1,
        "{error}"
    );
    assert!(!error.contains("127.0.0.1"), "{error}");
}

/// SQLite primary + Elasticsearch search secondary: a transaction that
/// updates the same Patient several times indexes the primary's version and
/// lastUpdated. The hit's version is one `If-Match` accepts.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_es_transaction_indexes_primary_version_and_accepts_if_match() {
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::error::ConcurrencyError;
    let server = MockServer::start().await;
    indices_exist(&server).await;
    Mock::given(method("POST"))
        .and(path("/_mget"))
        .respond_with(mget_nothing_found)
        .mount(&server)
        .await;
    on(&server, "GET", DOC_PATH, |_| container_absent()).await;
    Mock::given(method("POST"))
        .and(path("/_bulk"))
        .respond_with(|request: &Request| {
            let n = String::from_utf8_lossy(&request.body).lines().count() / 2;
            let items: Vec<Value> = (0..n)
                .map(|_| json!({"index": {"status": 200, "result": "updated"}}))
                .collect();
            ResponseTemplate::new(200)
                .set_body_json(json!({"took": 1, "errors": false, "items": items}))
        })
        .mount(&server)
        .await;
    on(&server, "POST", TENANT_DBQ_PATH, |_| swept(0)).await;
    on(&server, "POST", DOC_PATH, |_| indexed()).await;

    let sqlite = Arc::new(SqliteBackend::in_memory().expect("sqlite"));
    sqlite.init_schema().expect("schema");
    let es = Arc::new(backend(&server));
    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("es", BackendKind::Elasticsearch)
        .sync_mode(SyncMode::Synchronous)
        .build()
        .expect("composite config");
    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), sqlite.clone() as DynStorage);
    backends.insert("es".to_string(), es.clone() as DynStorage);
    let composite = CompositeStorage::new(config, backends)
        .expect("composite")
        .with_full_primary(sqlite);

    composite
        .create(
            &tenant(),
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "p1",
                "name": [{ "family": "One" }]
            }),
            FhirVersion::default(),
        )
        .await
        .expect("create");

    let result = composite
        .process_transaction(
            &tenant(),
            vec![BundleEntry {
                method: BundleMethod::Put,
                url: "Patient/p1".to_string(),
                resource: Some(json!({
                    "resourceType": "Patient",
                    "id": "p1",
                    "name": [{ "family": "Two" }]
                })),
                ..Default::default()
            }],
            FhirVersion::default(),
        )
        .await
        .expect("transaction");
    assert!(result.entries.iter().all(|e| e.status < 300), "{result:?}");

    let stored = composite
        .read(&tenant(), "Patient", "p1")
        .await
        .expect("read")
        .expect("present");
    assert_eq!(stored.version_id(), "2");

    let bulk = server
        .received_requests()
        .await
        .expect("request recording is on")
        .into_iter()
        .find(|r| r.url.path() == "/_bulk")
        .expect("bundle sync is one _bulk page");
    let docs: Vec<Value> = String::from_utf8_lossy(&bulk.body)
        .lines()
        .skip(1)
        .step_by(2)
        .map(|line| serde_json::from_str::<Value>(line).expect("document line"))
        .collect();
    let last = docs
        .iter()
        .rev()
        .find(|doc| doc["resource_id"] == json!("p1"))
        .expect("Patient/p1 was indexed");
    assert_eq!(last["version_id"], json!("2"));
    let indexed_at = chrono::DateTime::parse_from_rfc3339(
        last["last_updated"]
            .as_str()
            .expect("last_updated is a string"),
    )
    .expect("indexed last_updated is RFC3339")
    .with_timezone(&chrono::Utc);
    assert_eq!(
        indexed_at.timestamp_millis(),
        stored.last_modified().timestamp_millis(),
        "search last_updated is the primary's last_modified, not the sync clock"
    );

    composite
        .update_with_match(
            &tenant(),
            "Patient",
            "p1",
            "2",
            json!({
                "resourceType": "Patient",
                "id": "p1",
                "name": [{ "family": "Four" }]
            }),
        )
        .await
        .expect("If-Match of the indexed version succeeds");

    let conflict = composite
        .update_with_match(
            &tenant(),
            "Patient",
            "p1",
            "1",
            json!({
                "resourceType": "Patient",
                "id": "p1",
                "name": [{ "family": "Stale" }]
            }),
        )
        .await
        .expect_err("a stale search-hit version must 412");
    assert!(
        matches!(
            conflict,
            StorageError::Concurrency(ConcurrencyError::VersionConflict { .. })
        ),
        "{conflict:?}"
    );
}
