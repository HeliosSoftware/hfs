//! #862: conditional writes announce to the post-commit write observer.
//!
//! Every conditional success arm reported its live-count delta but passed
//! `notice: None`, so a subscriber never heard about a resource created by
//! `If-None-Exist`, written by `PUT [type]?[criteria]`, patched by
//! `PATCH [type]?[criteria]` or deleted by `DELETE [type]?[criteria]` — direct
//! or as a Bundle entry. These tests drive the real router and read what the
//! handlers hand the observer: the funnel subscribers (the subscriptions
//! engine, the dashboard counters) are attached to.
//!
//! What must NOT announce is pinned as hard as what must: a matched
//! `If-None-Exist` and a conditional delete that found nothing wrote nothing,
//! a refused precondition wrote nothing, and a rolled-back transaction commits
//! nothing, so none of them may reach an observer.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{ResourceWrite, WriteEvent, WriteKind, WriteObserver};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const IF_NONE_EXIST: HeaderName = HeaderName::from_static("if-none-exist");
const TENANT: &str = "test-tenant";

fn tenant() -> HeaderValue {
    HeaderValue::from_static(TENANT)
}

/// Every event reported to the state's observer, in order.
#[derive(Default)]
struct Recording {
    events: Mutex<Vec<WriteEvent>>,
}

impl WriteObserver for Recording {
    fn on_write(&self, event: &WriteEvent) {
        self.events.lock().unwrap().push(event.clone());
    }
}

impl Recording {
    /// The resource writes reported since the last call.
    fn take(&self) -> Vec<ResourceWrite> {
        std::mem::take(&mut *self.events.lock().unwrap())
            .into_iter()
            .map(|event| match event {
                WriteEvent::Resource(write) => write,
                other => panic!("unexpected non-resource event: {other:?}"),
            })
            .collect()
    }
}

struct Harness {
    server: TestServer,
    recording: Arc<Recording>,
}

/// A server over in-memory SQLite with the spec search parameters loaded —
/// without them `identifier` is an unknown parameter and every criteria a
/// `400` — and a recording observer on the state.
fn harness() -> Harness {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .canonicalize()
        .expect("repo data dir");
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("in-memory SQLite");
    backend.init_schema().expect("init schema");

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: TENANT.to_string(),
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::new(backend), config);
    let recording = Arc::new(Recording::default());
    state.write_observer().subscribe(recording.clone());
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    Harness {
        server: TestServer::new(app).expect("test server"),
        recording,
    }
}

fn patient(identifier: &str, active: bool) -> Value {
    json!({
        "resourceType": "Patient",
        "active": active,
        "identifier": [{"system": "http://example.org/mrn", "value": identifier}]
    })
}

impl Harness {
    /// Stores `Patient/{id}` (unconditionally) and discards the event.
    async fn seed(&self, id: &str, identifier: &str, active: bool) {
        let mut body = patient(identifier, active);
        body["id"] = json!(id);
        self.server
            .put(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&body)
            .await
            .assert_status(StatusCode::CREATED);
        self.recording.take();
    }

    async fn post_bundle(&self, bundle: &Value) -> axum_test::TestResponse {
        self.server
            .post("/")
            .add_header(X_TENANT_ID, tenant())
            .json(bundle)
            .await
    }
}

/// Asserts `write` is a single announced write and returns its notice.
fn announced(write: &ResourceWrite) -> &helios_persistence::core::WriteNotice {
    write
        .notice
        .as_ref()
        .unwrap_or_else(|| panic!("a conditional write must announce: {write:?}"))
}

/// The `Patient.identifier` value of a notice's content, else its id: a label
/// that names which entry a notice came from regardless of event order.
fn label(write: &ResourceWrite) -> String {
    let notice = announced(write);
    notice
        .resource
        .as_ref()
        .and_then(|resource| resource["identifier"][0]["value"].as_str())
        .map_or_else(|| notice.resource_id.clone(), str::to_string)
}

/// `(label, live_delta, kind)` of each write, sorted so the assertion does not
/// depend on the order entries complete in.
fn summary(writes: &[ResourceWrite]) -> Vec<(String, i64, WriteKind)> {
    let mut out: Vec<_> = writes
        .iter()
        .map(|write| {
            assert_eq!(write.resource_type, "Patient");
            assert_eq!(write.tenant.as_str(), TENANT);
            (label(write), write.live_delta, announced(write).kind)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn fhirpath_replace_active(value: bool) -> Value {
    json!({"resourceType":"Parameters","parameter":[{"name":"operation","part":[
        {"name":"type","valueCode":"replace"},
        {"name":"path","valueString":"Patient.active"},
        {"name":"value","valueBoolean":value}
    ]}]})
}

// =============================================================================
// Direct handlers
// =============================================================================

#[tokio::test]
async fn conditional_create_announces_a_create_notice() {
    let h = harness();
    let created = h
        .server
        .post("/Patient")
        .add_header(X_TENANT_ID, tenant())
        .add_header(IF_NONE_EXIST, HeaderValue::from_static("identifier=cc-1"))
        .json(&patient("cc-1", false))
        .await;
    created.assert_status(StatusCode::CREATED);
    let id = created.json::<Value>()["id"]
        .as_str()
        .expect("id")
        .to_string();

    let writes = h.recording.take();
    assert_eq!(writes.len(), 1, "exactly one event per committed write");
    assert_eq!(writes[0].live_delta, 1);
    let notice = announced(&writes[0]);
    assert_eq!(notice.kind, WriteKind::Create);
    assert_eq!(notice.resource_id, id);
    assert_eq!(notice.version_id, "1");
    let content = notice.resource.as_ref().expect("content");
    assert_eq!(content["id"], json!(id));
    assert_eq!(
        content["meta"]["versionId"],
        json!("1"),
        "content with meta"
    );
    assert_eq!(content["identifier"][0]["value"], json!("cc-1"));
    assert!(notice.previous.is_none());

    // A match answers 200 with the existing resource and writes nothing.
    h.server
        .post("/Patient")
        .add_header(X_TENANT_ID, tenant())
        .add_header(IF_NONE_EXIST, HeaderValue::from_static("identifier=cc-1"))
        .json(&patient("cc-1", false))
        .await
        .assert_status_ok();
    assert!(
        h.recording.take().is_empty(),
        "a matched If-None-Exist is not a write"
    );
}

#[tokio::test]
async fn conditional_update_announces_the_upsert_create_and_the_update() {
    let h = harness();

    // No match: the upsert creates (201).
    h.server
        .put("/Patient?identifier=cu-1")
        .add_header(X_TENANT_ID, tenant())
        .json(&patient("cu-1", false))
        .await
        .assert_status(StatusCode::CREATED);
    let writes = h.recording.take();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].live_delta, 1);
    let notice = announced(&writes[0]);
    assert_eq!(notice.kind, WriteKind::Create);
    assert_eq!(notice.version_id, "1");
    let id = notice.resource_id.clone();

    // One match: it updates (200).
    h.server
        .put("/Patient?identifier=cu-1")
        .add_header(X_TENANT_ID, tenant())
        .json(&patient("cu-1", true))
        .await
        .assert_status_ok();
    let writes = h.recording.take();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].live_delta, 0);
    let notice = announced(&writes[0]);
    assert_eq!(notice.kind, WriteKind::Update);
    assert_eq!(notice.resource_id, id);
    assert_eq!(notice.version_id, "2");
    assert_eq!(
        notice.resource.as_ref().expect("content")["active"],
        json!(true)
    );

    // A refused precondition writes nothing, so announces nothing.
    h.server
        .put("/Patient?identifier=cu-1")
        .add_header(X_TENANT_ID, tenant())
        .add_header(header::IF_MATCH, HeaderValue::from_static("W/\"1\""))
        .json(&patient("cu-1", false))
        .await
        .assert_status(StatusCode::PRECONDITION_FAILED);
    assert!(h.recording.take().is_empty());
}

#[tokio::test]
async fn conditional_delete_announces_a_delete_notice_with_previous_content() {
    let h = harness();
    h.seed("cd-1", "cd-1", true).await;

    h.server
        .delete("/Patient?identifier=cd-1")
        .add_header(X_TENANT_ID, tenant())
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let writes = h.recording.take();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].live_delta, -1);
    let notice = announced(&writes[0]);
    assert_eq!(notice.kind, WriteKind::Delete);
    assert_eq!(notice.resource_id, "cd-1");
    assert!(notice.version_id.is_empty());
    assert!(notice.resource.is_none());
    assert_eq!(
        notice.previous.as_ref().expect("pre-delete content")["active"],
        json!(true),
        "the instance DELETE hands the pre-delete content over; so does this"
    );

    // Nothing matches any more: 204, nothing deleted, nothing announced.
    h.server
        .delete("/Patient?identifier=cd-1")
        .add_header(X_TENANT_ID, tenant())
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert!(h.recording.take().is_empty());
}

#[tokio::test]
async fn conditional_patch_announces_an_update_notice() {
    let h = harness();
    h.seed("cp-1", "cp-1", false).await;

    h.server
        .patch("/Patient?identifier=cp-1")
        .add_header(X_TENANT_ID, tenant())
        .add_header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json-patch+json"),
        )
        .bytes(
            json!([{"op": "replace", "path": "/active", "value": true}])
                .to_string()
                .into(),
        )
        .await
        .assert_status_ok();
    let writes = h.recording.take();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].live_delta, 0);
    let notice = announced(&writes[0]);
    assert_eq!(notice.kind, WriteKind::Update);
    assert_eq!(notice.resource_id, "cp-1");
    assert_eq!(notice.version_id, "2");
    assert_eq!(
        notice.resource.as_ref().expect("content")["active"],
        json!(true)
    );

    // No match is a 404 and writes nothing.
    h.server
        .patch("/Patient?identifier=cp-none")
        .add_header(X_TENANT_ID, tenant())
        .add_header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json-patch+json"),
        )
        .bytes(
            json!([{"op": "replace", "path": "/active", "value": true}])
                .to_string()
                .into(),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
    assert!(h.recording.take().is_empty());
}

// =============================================================================
// Bundle entries
// =============================================================================

/// A batch's conditional entries announce like their unconditional siblings:
/// create, upsert-create, update, patch and delete each reach the observer; a
/// matched `ifNoneExist` and a delete that found nothing do not.
#[tokio::test]
async fn batch_conditional_entries_announce() {
    let h = harness();
    h.seed("b-upd", "b-upd", false).await;
    h.seed("b-del", "b-del", true).await;
    h.seed("b-patch", "b-patch", false).await;

    let bundle = json!({
        "resourceType": "Bundle",
        "type": "batch",
        "entry": [
            {   // created
                "resource": patient("b-new", false),
                "request": {"method": "POST", "url": "Patient", "ifNoneExist": "identifier=b-new"}
            },
            {   // matched: nothing written
                "resource": patient("b-upd", false),
                "request": {"method": "POST", "url": "Patient", "ifNoneExist": "identifier=b-upd"}
            },
            {   // updated
                "resource": patient("b-upd", true),
                "request": {"method": "PUT", "url": "Patient?identifier=b-upd"}
            },
            {   // upsert-created
                "resource": patient("b-up", false),
                "request": {"method": "PUT", "url": "Patient?identifier=b-up"}
            },
            {   // deleted
                "request": {"method": "DELETE", "url": "Patient?identifier=b-del"}
            },
            {   // nothing matched: nothing deleted
                "request": {"method": "DELETE", "url": "Patient?identifier=b-nothing"}
            },
            {   // patched
                "resource": fhirpath_replace_active(true),
                "request": {"method": "PATCH", "url": "Patient?identifier=b-patch"}
            }
        ]
    });
    let response = h.post_bundle(&bundle).await;
    response.assert_status_ok();
    let statuses: Vec<String> = response.json::<Value>()["entry"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|e| e["response"]["status"].as_str().unwrap_or("?").to_string())
        .collect();
    assert_eq!(
        statuses
            .iter()
            .map(|s| s.split(' ').next().unwrap_or("?"))
            .collect::<Vec<_>>(),
        ["201", "200", "200", "201", "204", "204", "200"],
        "{statuses:?}"
    );

    let writes = h.recording.take();
    assert_eq!(
        summary(&writes),
        vec![
            ("b-del".to_string(), -1, WriteKind::Delete),
            ("b-new".to_string(), 1, WriteKind::Create),
            ("b-patch".to_string(), 0, WriteKind::Update),
            ("b-up".to_string(), 1, WriteKind::Create),
            ("b-upd".to_string(), 0, WriteKind::Update),
        ],
        "one announcement per write; the matched POST and the no-match DELETE wrote nothing"
    );
    let deleted = writes
        .iter()
        .find(|w| announced(w).kind == WriteKind::Delete)
        .expect("delete event");
    assert_eq!(announced(deleted).resource_id, "b-del");
}

/// A transaction announces its conditional entries once, after commit.
#[tokio::test]
async fn transaction_conditional_entries_announce_after_commit() {
    let h = harness();
    h.seed("t-upd", "t-upd", false).await;
    h.seed("t-del", "t-del", true).await;
    h.seed("t-patch", "t-patch", false).await;

    let bundle = json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            {   // created
                "fullUrl": "urn:uuid:9d8a0a3e-0000-4000-8000-0000000000c1",
                "resource": patient("t-new", false),
                "request": {"method": "POST", "url": "Patient", "ifNoneExist": "identifier=t-new"}
            },
            {   // updated
                "resource": patient("t-upd", true),
                "request": {"method": "PUT", "url": "Patient?identifier=t-upd"}
            },
            {   // upsert-created
                "resource": patient("t-up", false),
                "request": {"method": "PUT", "url": "Patient?identifier=t-up"}
            },
            {   // deleted
                "request": {"method": "DELETE", "url": "Patient?identifier=t-del"}
            },
            {   // nothing matched: nothing deleted
                "request": {"method": "DELETE", "url": "Patient?identifier=t-nothing"}
            },
            {   // patched
                "resource": fhirpath_replace_active(true),
                "request": {"method": "PATCH", "url": "Patient?identifier=t-patch"}
            }
        ]
    });
    h.post_bundle(&bundle).await.assert_status_ok();

    let writes = h.recording.take();
    assert_eq!(
        summary(&writes),
        vec![
            ("t-del".to_string(), -1, WriteKind::Delete),
            ("t-new".to_string(), 1, WriteKind::Create),
            ("t-patch".to_string(), 0, WriteKind::Update),
            ("t-up".to_string(), 1, WriteKind::Create),
            ("t-upd".to_string(), 0, WriteKind::Update),
        ],
        "one announcement per write; the no-match DELETE wrote nothing"
    );
    // A conditional DELETE's URL carries criteria, not an id: the notice names
    // the resource the backend resolved. The transaction result holds no
    // pre-delete content, as for an instance DELETE entry.
    let deleted = writes
        .iter()
        .find(|w| announced(w).kind == WriteKind::Delete)
        .expect("delete event");
    let notice = announced(deleted);
    assert_eq!(notice.resource_id, "t-del");
    assert!(notice.version_id.is_empty());
    assert!(notice.resource.is_none());
}

/// A transaction that rolls back commits nothing, so a conditional entry that
/// ran before the failing one must not have announced.
#[tokio::test]
async fn rolled_back_transaction_announces_nothing() {
    let h = harness();
    h.seed("r-del", "r-del", true).await;

    let bundle = json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            {
                "fullUrl": "urn:uuid:9d8a0a3e-0000-4000-8000-0000000000d1",
                "resource": patient("r-new", false),
                "request": {"method": "POST", "url": "Patient", "ifNoneExist": "identifier=r-new"}
            },
            {
                "request": {"method": "DELETE", "url": "Patient?identifier=r-del"}
            },
            {   // fails: nothing to delete, which a transaction refuses
                "request": {"method": "DELETE", "url": "Observation/never-existed"}
            }
        ]
    });
    let response = h.post_bundle(&bundle).await;
    assert!(!response.status_code().is_success(), "{}", response.text());
    assert!(
        h.recording.take().is_empty(),
        "nothing committed, nothing announced"
    );

    // The rollback really did leave the state alone.
    let remaining: Value = h
        .server
        .get("/Patient?_count=10")
        .add_header(X_TENANT_ID, tenant())
        .await
        .json();
    let ids: Vec<&str> = remaining["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| e["resource"]["id"].as_str())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(ids, ["r-del"]);
}
