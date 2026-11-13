//! #2562: `$meta`, `$meta-add` and `$meta-delete`.
//!
//! `$meta-add` / `$meta-delete` keep the version id and move `lastUpdated`
//! (the current row, that version's history row and the search index all
//! change together), work on an explicit history version, refuse a topology
//! that cannot rewrite in place, and are advertised exactly where they run.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{
    BackendEntry, BackendRole, CompositeConfig, CompositeStorage, DynStorage,
};
use helios_persistence::core::{
    BackendKind, ResourceStorage, WriteEvent, WriteKind, WriteObserver,
};
use helios_rest::ServerConfig;
use serde_json::{Value, json};

const TAGS: &str = "http://example.org/tags";

fn sqlite() -> SqliteBackend {
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
    backend
}

fn config() -> ServerConfig {
    ServerConfig {
        base_url: "http://localhost:8080".to_string(),
        ..ServerConfig::for_testing()
    }
}

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
    fn update_notices(&self) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| match event {
                WriteEvent::Resource(write) => write
                    .notice
                    .as_ref()
                    .is_some_and(|notice| notice.kind == WriteKind::Update),
                _ => false,
            })
            .count()
    }
}

fn server() -> (TestServer, Arc<Recording>) {
    let state = helios_rest::AppState::new(Arc::new(sqlite()), config());
    let writes = Arc::new(Recording::default());
    state.write_observer().subscribe(writes.clone());
    let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
        .expect("test server");
    (server, writes)
}

fn tag(code: &str) -> Value {
    json!({"system": TAGS, "code": code})
}

fn parameters(meta: Value) -> Value {
    json!({"resourceType": "Parameters", "parameter": [{"name": "meta", "valueMeta": meta}]})
}

fn task(tags: &[&str]) -> Value {
    let tags: Vec<Value> = tags.iter().map(|c| tag(c)).collect();
    json!({"resourceType": "Task", "id": "t1", "status": "requested", "intent": "order",
           "meta": {"tag": tags}})
}

fn codes(meta: &Value) -> Vec<String> {
    meta["tag"]
        .as_array()
        .map(|tags| {
            tags.iter()
                .map(|t| t["code"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn returned_meta(response: &axum_test::TestResponse) -> Value {
    let body: Value = response.json();
    assert_eq!(body["resourceType"], "Parameters", "{body}");
    assert_eq!(body["parameter"][0]["name"], "return");
    body["parameter"][0]["valueMeta"].clone()
}

async fn found_by_tag(server: &TestServer, code: &str) -> usize {
    let bundle: Value = server
        .get(&format!("/Task?_tag={TAGS}|{code}"))
        .await
        .json();
    bundle["entry"].as_array().map(Vec::len).unwrap_or(0)
}

#[tokio::test]
async fn meta_reads_the_current_and_a_historical_version() {
    let (server, _) = server();
    server.put("/Task/t1").json(&task(&["a"])).await;
    server.put("/Task/t1").json(&task(&["b"])).await;

    for method in ["GET", "POST"] {
        let response = match method {
            "GET" => server.get("/Task/t1/$meta").await,
            _ => server.post("/Task/t1/$meta").await,
        };
        response.assert_status_ok();
        let meta = returned_meta(&response);
        assert_eq!(meta["versionId"], "2");
        assert!(meta["lastUpdated"].is_string());
        assert_eq!(codes(&meta), ["a", "b"]);
    }

    let response = server.get("/Task/t1/_history/1/$meta").await;
    response.assert_status_ok();
    let meta = returned_meta(&response);
    assert_eq!(meta["versionId"], "1");
    assert_eq!(codes(&meta), ["a"]);
}

#[tokio::test]
async fn system_and_type_level_meta_answer_400() {
    let (server, _) = server();
    for path in ["/$meta", "/Task/$meta"] {
        server
            .get(path)
            .await
            .assert_status(StatusCode::BAD_REQUEST);
        server
            .post(path)
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn missing_and_deleted_resources() {
    let (server, _) = server();
    server
        .get("/Task/none/$meta")
        .await
        .assert_status(StatusCode::NOT_FOUND);
    server
        .post("/Task/none/$meta-add")
        .json(&parameters(json!({"tag": [tag("a")]})))
        .await
        .assert_status(StatusCode::NOT_FOUND);

    server.put("/Task/t1").json(&task(&["a"])).await;
    server.delete("/Task/t1").await;
    server
        .get("/Task/t1/$meta")
        .await
        .assert_status(StatusCode::GONE);
    server
        .post("/Task/t1/$meta-add")
        .json(&parameters(json!({"tag": [tag("b")]})))
        .await
        .assert_status(StatusCode::GONE);
    // Version 2 is the tombstone, version 1 still has content.
    server
        .get("/Task/t1/_history/2/$meta")
        .await
        .assert_status(StatusCode::GONE);
    server
        .get("/Task/t1/_history/1/$meta")
        .await
        .assert_status_ok();
}

#[tokio::test]
async fn meta_add_keeps_the_version_and_moves_last_updated() {
    let (server, writes) = server();
    server.put("/Task/t1").json(&task(&["a"])).await;
    let before = server.get("/Task/t1").await;
    let before_body: Value = before.json();
    let before_modified = before.header("last-modified");
    let before_etag = before.header("etag");
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let notices = writes.update_notices();

    let response = server
        .post("/Task/t1/$meta-add")
        .json(&parameters(json!({"tag": [tag("b"), tag("a")]})))
        .await;
    response.assert_status_ok();
    let meta = returned_meta(&response);
    assert_eq!(meta["versionId"], "1");
    assert_eq!(codes(&meta), ["a", "b"]);
    assert!(
        meta["lastUpdated"].as_str() > before_body["meta"]["lastUpdated"].as_str(),
        "{meta} vs {before_body}"
    );

    let after = server.get("/Task/t1").await;
    let body: Value = after.json();
    assert_eq!(body["meta"]["versionId"], "1");
    assert_eq!(codes(&body["meta"]), ["a", "b"]);
    assert_eq!(after.header("etag"), before_etag);
    assert_ne!(after.header("last-modified"), before_modified);

    let version: Value = server.get("/Task/t1/_history/1").await.json();
    assert_eq!(codes(&version["meta"]), ["a", "b"]);
    assert_eq!(version["meta"]["lastUpdated"], body["meta"]["lastUpdated"]);

    assert_eq!(found_by_tag(&server, "b").await, 1);
    let since = before_body["meta"]["lastUpdated"].as_str().unwrap();
    let recent: Value = server
        .get(&format!(
            "/Task?_lastUpdated=gt{}",
            since.replace('+', "%2B")
        ))
        .await
        .json();
    assert_eq!(
        recent["entry"].as_array().map(Vec::len),
        Some(1),
        "{recent}"
    );
    assert_eq!(writes.update_notices(), notices + 1);
}

#[tokio::test]
async fn meta_delete_removes_and_an_absent_label_is_not_an_error() {
    let (server, _) = server();
    server.put("/Task/t1").json(&task(&["a", "b"])).await;
    let response = server
        .post("/Task/t1/$meta-delete")
        .json(&parameters(
            json!({"tag": [{"system": TAGS, "code": "a", "display": "ignored"}]}),
        ))
        .await;
    response.assert_status_ok();
    assert_eq!(codes(&returned_meta(&response)), ["b"]);
    assert_eq!(found_by_tag(&server, "a").await, 0);
    assert_eq!(found_by_tag(&server, "b").await, 1);

    let response = server
        .post("/Task/t1/$meta-delete")
        .json(&parameters(
            json!({"tag": [tag("never")], "profile": ["http://none"]}),
        ))
        .await;
    response.assert_status_ok();
    assert_eq!(codes(&returned_meta(&response)), ["b"]);
}

#[tokio::test]
async fn a_history_version_is_changed_alone() {
    let (server, writes) = server();
    server.put("/Task/t1").json(&task(&["a"])).await;
    server.put("/Task/t1").json(&task(&["b"])).await;
    let current_before: Value = server.get("/Task/t1").await.json();
    let notices = writes.update_notices();

    let response = server
        .post("/Task/t1/_history/1/$meta-add")
        .json(&parameters(json!({"tag": [tag("h")]})))
        .await;
    response.assert_status_ok();
    assert_eq!(codes(&returned_meta(&response)), ["a", "h"]);

    let version: Value = server.get("/Task/t1/_history/1").await.json();
    assert_eq!(codes(&version["meta"]), ["a", "h"]);
    let current: Value = server.get("/Task/t1").await.json();
    assert_eq!(current, current_before);
    assert_eq!(found_by_tag(&server, "h").await, 0);
    assert_eq!(writes.update_notices(), notices);

    server
        .post("/Task/t1/_history/9/$meta-add")
        .json(&parameters(json!({"tag": [tag("h")]})))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_current_version_named_by_history_changes_current_too() {
    let (server, _) = server();
    server.put("/Task/t1").json(&task(&["a"])).await;
    server
        .post("/Task/t1/_history/1/$meta-add")
        .json(&parameters(json!({"tag": [tag("c")]})))
        .await
        .assert_status_ok();
    let current: Value = server.get("/Task/t1").await.json();
    assert_eq!(codes(&current["meta"]), ["a", "c"]);
    assert_eq!(found_by_tag(&server, "c").await, 1);
}

#[tokio::test]
async fn invalid_input_changes_nothing() {
    let (server, _) = server();
    server.put("/Task/t1").json(&task(&["a"])).await;
    let before: Value = server.get("/Task/t1").await.json();
    for body in [
        json!({"resourceType": "Parameters", "parameter": [{"name": "meta", "valueCoding": tag("x")}]}),
        parameters(json!({"tag": [{"system": 1, "code": "x"}]})),
        parameters(json!({"tag": [{"code": "x", "userSelected": "true"}]})),
        parameters(json!({"tag": [{"code": "x", "extension": [{"url": "http://e"}]}]})),
    ] {
        server
            .post("/Task/t1/$meta-add")
            .json(&body)
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
    let after: Value = server.get("/Task/t1").await.json();
    assert_eq!(after, before);
}

#[tokio::test]
async fn audit_event_metadata_is_immutable() {
    let (server, _) = server();
    server
        .post("/AuditEvent/a1/$meta-add")
        .json(&parameters(json!({"tag": [tag("a")]})))
        .await
        .assert_status(StatusCode::METHOD_NOT_ALLOWED);
}

fn operation_names(statement: &Value) -> Vec<String> {
    statement["rest"][0]["operation"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn sqlite_advertises_the_operations_and_serves_the_definition() {
    let (server, _) = server();
    let statement: Value = server.get("/metadata").await.json();
    let names = operation_names(&statement);
    for name in ["meta", "meta-add", "meta-delete"] {
        assert!(names.contains(&name.to_string()), "{names:?}");
    }
    let meta = statement["rest"][0]["operation"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["name"] == "meta")
        .unwrap();
    assert_eq!(meta["definition"], "/OperationDefinition/hfs-meta");

    let definition: Value = server.get("/OperationDefinition/hfs-meta").await.json();
    assert_eq!(definition["code"], "meta");
    assert_eq!(definition["system"], false);
    assert_eq!(definition["type"], false);
    assert_eq!(definition["instance"], true);
    assert_eq!(
        definition["base"],
        "http://hl7.org/fhir/OperationDefinition/Resource-meta"
    );
}

/// A composite over SQLite: no secondary (`None`), or one supplied in the
/// backends map with this role and enabled flag.
fn composite_server(secondary: Option<(BackendRole, bool)>) -> TestServer {
    let mut builder = CompositeConfig::builder().primary("sqlite", BackendKind::Sqlite);
    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    let primary = Arc::new(sqlite());
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    if let Some((role, enabled)) = secondary {
        let kind = match role {
            BackendRole::Search => BackendKind::Elasticsearch,
            _ => BackendKind::Sqlite,
        };
        let mut entry = BackendEntry::new("secondary", role, kind);
        entry.enabled = enabled;
        builder = builder.with_backend(entry);
        backends.insert("secondary".to_string(), Arc::new(sqlite()) as DynStorage);
    }
    let composite_config = builder.build().expect("composite config");
    // Wired as the production constructors wire it, so versioned reads and
    // history reach the primary.
    let composite = CompositeStorage::new(composite_config, backends)
        .expect("composite")
        .with_full_primary(primary);
    assert_eq!(
        composite.supports_meta_rewrite(),
        secondary.is_none(),
        "capability and refusal share one condition"
    );
    assert_eq!(composite.tracks_meta_revision(), secondary.is_none());
    let state = helios_rest::AppState::new(Arc::new(composite), config());
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).expect("test server")
}

#[tokio::test]
async fn a_composite_with_a_supplied_secondary_refuses_metadata_writes() {
    for secondary in [
        (BackendRole::Archive, true),
        (BackendRole::Archive, false),
        (BackendRole::Search, true),
    ] {
        let enabled = secondary;
        let server = composite_server(Some(secondary));
        server.put("/Task/t1").json(&task(&["a"])).await;
        let before: Value = server.get("/Task/t1").await.json();
        let response = server
            .post("/Task/t1/$meta-add")
            .json(&parameters(json!({"tag": [tag("b")]})))
            .await;
        response.assert_status(StatusCode::NOT_IMPLEMENTED);
        let after: Value = server.get("/Task/t1").await.json();
        assert_eq!(after, before, "{enabled:?}");

        let names = operation_names(&server.get("/metadata").await.json());
        assert!(names.contains(&"meta".to_string()));
        assert!(!names.contains(&"meta-add".to_string()), "{names:?}");
        assert!(!names.contains(&"meta-delete".to_string()));

        server.get("/Task/t1/$meta").await.assert_status_ok();
        server.put("/Task/t1").json(&task(&["c"])).await;
        let merged: Value = server.get("/Task/t1").await.json();
        assert_eq!(codes(&merged["meta"]), ["a", "c"]);
    }
}

#[tokio::test]
async fn a_composite_without_secondaries_supports_metadata_writes() {
    let server = composite_server(None);
    server.put("/Task/t1").json(&task(&["a"])).await;
    server
        .post("/Task/t1/$meta-add")
        .json(&parameters(json!({"tag": [tag("b")]})))
        .await
        .assert_status_ok();
    let names = operation_names(&server.get("/metadata").await.json());
    assert!(names.contains(&"meta-add".to_string()), "{names:?}");

    // History-version reads and writes go through the composite too.
    server.put("/Task/t1").json(&task(&["c"])).await;
    let response = server.get("/Task/t1/_history/1/$meta").await;
    response.assert_status_ok();
    assert_eq!(codes(&returned_meta(&response)), ["a", "b"]);
    let response = server
        .post("/Task/t1/_history/1/$meta-add")
        .json(&parameters(json!({"tag": [tag("h")]})))
        .await;
    response.assert_status_ok();
    assert_eq!(codes(&returned_meta(&response)), ["a", "b", "h"]);
    let version: Value = server.get("/Task/t1/_history/1").await.json();
    assert_eq!(codes(&version["meta"]), ["a", "b", "h"]);
    let current: Value = server.get("/Task/t1").await.json();
    assert_eq!(codes(&current["meta"]), ["a", "b", "c"]);
    server
        .post("/Task/t1/_history/2/$meta-delete")
        .json(&parameters(json!({"tag": [tag("c")]})))
        .await
        .assert_status_ok();
    let current: Value = server.get("/Task/t1").await.json();
    assert_eq!(codes(&current["meta"]), ["a", "b"]);
}

#[tokio::test]
async fn a_second_value_choice_in_the_wrapper_changes_nothing() {
    let (server, _) = server();
    server.put("/Task/t1").json(&task(&["a"])).await;
    let before: Value = server.get("/Task/t1").await.json();
    server
        .post("/Task/t1/$meta-add")
        .json(&json!({"resourceType": "Parameters", "parameter": [{
            "name": "meta",
            "valueMeta": {"tag": [tag("b")]},
            "_valueString": {"extension": [{"url": "http://example.org/e", "valueCode": "x"}]}
        }]}))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    let after: Value = server.get("/Task/t1").await.json();
    assert_eq!(after, before);
}

#[tokio::test]
async fn responses_carry_the_fhir_content_type_and_honour_the_accept_version() {
    let (server, _) = server();
    server.put("/Task/t1").json(&task(&["a"])).await;
    for path in ["/Task/t1/$meta", "/Task/t1/_history/1/$meta"] {
        let response = server.get(path).await;
        response.assert_status_ok();
        assert_eq!(
            response.header("content-type"),
            "application/fhir+json; fhirVersion=4.0",
            "{path}"
        );
    }
    let response = server
        .post("/Task/t1/$meta-add")
        .json(&parameters(json!({"tag": [tag("b")]})))
        .await;
    assert_eq!(
        response.header("content-type"),
        "application/fhir+json; fhirVersion=4.0"
    );

    #[cfg(feature = "R5")]
    {
        let wrong = server
            .get("/Task/t1/$meta")
            .add_header("Accept", "application/fhir+json; fhirVersion=5.0")
            .await;
        wrong.assert_status(StatusCode::NOT_ACCEPTABLE);
    }

    #[cfg(feature = "xml")]
    {
        let response = server
            .get("/Task/t1/$meta")
            .add_header("Accept", "application/fhir+xml")
            .await;
        response.assert_status_ok();
        assert_eq!(
            response.header("content-type"),
            "application/fhir+xml; fhirVersion=4.0"
        );
        assert!(
            response.text().contains("<Parameters"),
            "{}",
            response.text()
        );
    }
}

fn integer64_tag() -> Value {
    json!({"tag": [{"system": TAGS, "code": "n", "extension": [
        {"url": "http://example.org/count", "valueInteger64": "9007199254740993"}
    ]}]})
}

/// Input in another FHIR version than the stored resource's could carry
/// elements that version does not define: refused for the current and for a
/// historical version, and nothing changes.
#[cfg(feature = "R5")]
#[tokio::test]
async fn input_in_another_fhir_version_than_the_resource_changes_nothing() {
    let (server, _) = server();
    server.put("/Task/t1").json(&task(&["a"])).await;
    let before: Value = server.get("/Task/t1").await.json();
    for path in ["/Task/t1/$meta-add", "/Task/t1/_history/1/$meta-add"] {
        let response = server
            .post(path)
            .json(&parameters(integer64_tag()))
            .content_type("application/fhir+json; fhirVersion=5.0")
            .await;
        response.assert_status(StatusCode::BAD_REQUEST);
    }
    // Without a declared version the input is checked against R4, where
    // valueInteger64 does not exist.
    server
        .post("/Task/t1/$meta-add")
        .json(&parameters(integer64_tag()))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    let after: Value = server.get("/Task/t1").await.json();
    assert_eq!(after, before);
}

/// An R5 resource keeps an R5-only element in its XML response.
#[cfg(all(feature = "R5", feature = "xml"))]
#[tokio::test]
async fn an_r5_xml_response_keeps_r5_elements() {
    let (server, _) = server();
    let r5 = "application/fhir+json; fhirVersion=5.0";
    server
        .put("/Task/t5")
        .json(
            &json!({"resourceType": "Task", "id": "t5", "status": "requested",
                      "intent": "order"}),
        )
        .content_type(r5)
        .await
        .assert_status(StatusCode::CREATED);
    server
        .post("/Task/t5/$meta-add")
        .json(&parameters(integer64_tag()))
        .content_type(r5)
        .await
        .assert_status_ok();
    let response = server
        .get("/Task/t5/$meta")
        .add_header("Accept", "application/fhir+xml; fhirVersion=5.0")
        .await;
    response.assert_status_ok();
    assert_eq!(
        response.header("content-type"),
        "application/fhir+xml; fhirVersion=5.0"
    );
    let xml = response.text();
    assert!(xml.contains("valueInteger64"), "{xml}");
    assert!(xml.contains("9007199254740993"), "{xml}");
}

fn tenant_server() -> TestServer {
    let state = helios_rest::AppState::new(
        Arc::new(sqlite()),
        ServerConfig {
            multitenancy: helios_rest::config::MultitenancyConfig {
                routing_mode: helios_rest::config::TenantRoutingMode::HeaderOnly,
                ..Default::default()
            },
            ..config()
        },
    );
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).expect("test server")
}

/// The same logical id in two tenants: each tenant's metadata operations see
/// and change only its own resource.
#[tokio::test]
async fn metadata_operations_stay_inside_their_tenant() {
    let server = tenant_server();
    let tenant = |name: &'static str| {
        (
            axum::http::HeaderName::from_static("x-tenant-id"),
            axum::http::HeaderValue::from_static(name),
        )
    };
    for (name, code) in [("tenant-a", "a"), ("tenant-b", "b")] {
        let (key, value) = tenant(name);
        server
            .put("/Task/t1")
            .add_header(key, value)
            .json(&task(&[code]))
            .await
            .assert_status(StatusCode::CREATED);
    }
    let (key, value) = tenant("tenant-a");
    server
        .post("/Task/t1/$meta-add")
        .add_header(key.clone(), value.clone())
        .json(&parameters(json!({"tag": [tag("added")]})))
        .await
        .assert_status_ok();
    server
        .post("/Task/t1/_history/1/$meta-delete")
        .add_header(key, value)
        .json(&parameters(json!({"tag": [tag("b")]})))
        .await
        .assert_status_ok();

    let meta_of = |name: &'static str, path: &'static str| {
        let (key, value) = tenant(name);
        let request = server.get(path).add_header(key, value);
        async move { returned_meta(&request.await) }
    };
    assert_eq!(
        codes(&meta_of("tenant-a", "/Task/t1/$meta").await),
        ["a", "added"]
    );
    assert_eq!(codes(&meta_of("tenant-b", "/Task/t1/$meta").await), ["b"]);
    assert_eq!(
        codes(&meta_of("tenant-b", "/Task/t1/_history/1/$meta").await),
        ["b"]
    );
}

/// The input gate works on the JSON the XML reader produces: a single `tag`
/// in XML arrives as an array and is accepted.
#[cfg(feature = "xml")]
#[tokio::test]
async fn an_xml_input_is_accepted() {
    let (server, _) = server();
    server.put("/Task/t1").json(&task(&["a"])).await;
    let xml = r#"<Parameters xmlns="http://hl7.org/fhir"><parameter><name value="meta"/><valueMeta><tag><system value="http://example.org/tags"/><code value="x"/></tag></valueMeta></parameter></Parameters>"#;
    let response = server
        .post("/Task/t1/$meta-add")
        .bytes(xml.as_bytes().to_vec().into())
        .content_type("application/fhir+xml")
        .add_header("Accept", "application/fhir+json")
        .await;
    response.assert_status_ok();
    assert_eq!(codes(&returned_meta(&response)), ["a", "x"]);
}

/// A response the request cannot accept is refused before anything changes:
/// a `$meta-add` / `$meta-delete` of the current or a historical version
/// that asks for another FHIR version (or for XML in a build without it)
/// answers 406, and the resource, its history, its index and the write
/// notices are untouched.
#[tokio::test]
async fn an_unacceptable_response_is_refused_before_the_rewrite() {
    let (server, writes) = server();
    server.put("/Task/t1").json(&task(&["a"])).await;
    let current_before: Value = server.get("/Task/t1").await.json();
    let version_before: Value = server.get("/Task/t1/_history/1").await.json();
    let notices = writes.update_notices();

    let unacceptable: &[&str] = &[
        #[cfg(feature = "R5")]
        "application/fhir+json; fhirVersion=5.0",
        #[cfg(not(feature = "xml"))]
        "application/fhir+xml",
    ];

    for &accept in unacceptable {
        for (path, meta) in [
            ("/Task/t1/$meta-add", json!({"tag": [tag("b")]})),
            ("/Task/t1/$meta-delete", json!({"tag": [tag("a")]})),
            ("/Task/t1/_history/1/$meta-add", json!({"tag": [tag("b")]})),
            (
                "/Task/t1/_history/1/$meta-delete",
                json!({"tag": [tag("a")]}),
            ),
        ] {
            let response = server
                .post(path)
                .add_header("Accept", accept)
                .json(&parameters(meta))
                .await;
            assert_eq!(
                response.status_code(),
                StatusCode::NOT_ACCEPTABLE,
                "{accept} {path}: {}",
                response.text()
            );
        }
    }
    let current_after: Value = server.get("/Task/t1").await.json();
    assert_eq!(current_after, current_before);
    let version_after: Value = server.get("/Task/t1/_history/1").await.json();
    assert_eq!(version_after, version_before);
    assert_eq!(found_by_tag(&server, "a").await, 1);
    assert_eq!(found_by_tag(&server, "b").await, 0);
    assert_eq!(writes.update_notices(), notices);
}
