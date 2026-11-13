//! `$meta`, `$meta-add` and `$meta-delete` over HTTP.
//!
//! An update merges tags and security labels instead of replacing them
//! (#2562), so these operations are how a client reads a resource's labels
//! and adds or removes them explicitly.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue};
use axum_test::{TestResponse, TestServer};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
const IF_MATCH: HeaderName = HeaderName::from_static("if-match");
const TAGS: &str = "http://example.org/tags";
const SEC: &str = "http://terminology.hl7.org/CodeSystem/v3-Confidentiality";

fn create_test_server() -> TestServer {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to init schema");

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::new(backend), config);
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
        .expect("Failed to create test server")
}

fn tenant() -> HeaderValue {
    HeaderValue::from_static("test-tenant")
}

fn fhir_json() -> HeaderValue {
    HeaderValue::from_static("application/fhir+json")
}

fn tag(code: &str) -> Value {
    json!({"system": TAGS, "code": code})
}

async fn seed(server: &TestServer, id: &str, meta: Value) {
    let response = server
        .put(&format!("/Task/{id}"))
        .add_header(X_TENANT_ID, tenant())
        .add_header(CONTENT_TYPE, fhir_json())
        .json(&json!({
            "resourceType": "Task", "id": id, "status": "requested", "intent": "order",
            "meta": meta
        }))
        .await;
    assert_eq!(response.status_code().as_u16(), 201, "{}", response.text());
}

fn parameters(meta: Value) -> Value {
    json!({"resourceType": "Parameters", "parameter": [{"name": "meta", "valueMeta": meta}]})
}

async fn invoke(server: &TestServer, path: &str, body: &Value) -> TestResponse {
    server
        .post(path)
        .add_header(X_TENANT_ID, tenant())
        .add_header(CONTENT_TYPE, fhir_json())
        .json(body)
        .await
}

/// The `return` Meta of an operation's `Parameters` response.
fn returned(response: &TestResponse) -> Value {
    let body = response.json::<Value>();
    assert_eq!(body["resourceType"], "Parameters", "{body}");
    assert_eq!(body["parameter"][0]["name"], "return", "{body}");
    body["parameter"][0]["valueMeta"].clone()
}

async fn read_meta(server: &TestServer, path: &str) -> Value {
    let response = server.get(path).add_header(X_TENANT_ID, tenant()).await;
    response.assert_status_ok();
    response.json::<Value>()["meta"].clone()
}

fn codes(list: &Value) -> Vec<String> {
    list.as_array()
        .map(|codings| {
            codings
                .iter()
                .map(|c| c["code"].as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default()
}

async fn tag_search_count(server: &TestServer, code: &str) -> usize {
    let response = server
        .get(&format!("/Task?_tag={TAGS}|{code}"))
        .add_header(X_TENANT_ID, tenant())
        .await;
    response.assert_status_ok();
    response.json::<Value>()["entry"]
        .as_array()
        .map_or(0, Vec::len)
}

#[tokio::test]
async fn meta_returns_the_current_meta_for_get_and_post() {
    let server = create_test_server();
    seed(&server, "m1", json!({"tag": [tag("a")]})).await;

    let get = server
        .get("/Task/m1/$meta")
        .add_header(X_TENANT_ID, tenant())
        .await;
    get.assert_status_ok();
    let meta = returned(&get);
    assert_eq!(codes(&meta["tag"]), ["a"]);
    assert_eq!(meta["versionId"], "1");
    assert!(meta["lastUpdated"].is_string());

    let post = invoke(
        &server,
        "/Task/m1/$meta",
        &json!({"resourceType": "Parameters"}),
    )
    .await;
    post.assert_status_ok();
    assert_eq!(returned(&post), meta);
}

#[tokio::test]
async fn meta_on_a_history_version_returns_that_versions_meta() {
    let server = create_test_server();
    seed(&server, "m2", json!({"tag": [tag("a")]})).await;
    invoke(
        &server,
        "/Task/m2/$meta-add",
        &parameters(json!({"tag": [tag("b")]})),
    )
    .await
    .assert_status_ok();

    let v1 = server
        .get("/Task/m2/_history/1/$meta")
        .add_header(X_TENANT_ID, tenant())
        .await;
    v1.assert_status_ok();
    let meta = returned(&v1);
    assert_eq!(codes(&meta["tag"]), ["a"]);
    assert_eq!(meta["versionId"], "1");
}

#[tokio::test]
async fn meta_add_adds_labels_and_writes_a_new_version() {
    let server = create_test_server();
    seed(&server, "m3", json!({"tag": [tag("a")]})).await;

    let add = parameters(json!({
        "tag": [tag("a"), tag("b")],
        "security": [{"system": SEC, "code": "R"}],
        "profile": ["http://example.org/StructureDefinition/p"]
    }));
    let response = invoke(&server, "/Task/m3/$meta-add", &add).await;
    response.assert_status_ok();
    let meta = returned(&response);
    assert_eq!(codes(&meta["tag"]), ["a", "b"]);
    assert_eq!(codes(&meta["security"]), ["R"]);
    assert_eq!(
        meta["profile"],
        json!(["http://example.org/StructureDefinition/p"])
    );
    assert_eq!(meta["versionId"], "2");
    assert_eq!(
        response.header("etag").to_str().unwrap(),
        "W/\"2\"",
        "the response carries the new version's ETag"
    );

    let stored = read_meta(&server, "/Task/m3").await;
    assert_eq!(codes(&stored["tag"]), ["a", "b"]);
    assert_eq!(
        tag_search_count(&server, "b").await,
        1,
        "the added tag is indexed"
    );

    // Adding what is already there writes nothing.
    let again = invoke(&server, "/Task/m3/$meta-add", &add).await;
    again.assert_status_ok();
    assert_eq!(returned(&again)["versionId"], "2");
}

#[tokio::test]
async fn meta_delete_removes_labels_that_an_update_would_keep() {
    let server = create_test_server();
    seed(
        &server,
        "m4",
        json!({
            "tag": [tag("a"), tag("b")],
            "security": [{"system": SEC, "code": "R"}],
            "profile": ["http://example.org/StructureDefinition/p"]
        }),
    )
    .await;

    let delete = parameters(json!({
        // display is ignored when matching
        "tag": [{"system": TAGS, "code": "a", "display": "Alpha"}, tag("absent")],
        "security": [{"system": SEC, "code": "R"}],
        "profile": ["http://example.org/StructureDefinition/p"]
    }));
    let response = invoke(&server, "/Task/m4/$meta-delete", &delete).await;
    response.assert_status_ok();
    let meta = returned(&response);
    assert_eq!(codes(&meta["tag"]), ["b"]);
    assert!(meta.get("security").is_none(), "{meta}");
    assert!(meta.get("profile").is_none(), "{meta}");
    assert_eq!(meta["versionId"], "2");

    let stored = read_meta(&server, "/Task/m4").await;
    assert_eq!(codes(&stored["tag"]), ["b"]);
    assert_eq!(
        tag_search_count(&server, "a").await,
        0,
        "the index forgets it"
    );
    assert_eq!(tag_search_count(&server, "b").await, 1);

    // A later update has nothing to merge back.
    let response = server
        .put("/Task/m4")
        .add_header(X_TENANT_ID, tenant())
        .add_header(CONTENT_TYPE, fhir_json())
        .json(
            &json!({"resourceType": "Task", "id": "m4", "status": "requested", "intent": "order"}),
        )
        .await;
    response.assert_status_ok();
    assert_eq!(codes(&read_meta(&server, "/Task/m4").await["tag"]), ["b"]);

    // Deleting what is absent is not an error, and writes nothing.
    let again = invoke(
        &server,
        "/Task/m4/$meta-delete",
        &parameters(json!({"tag": [tag("a")]})),
    )
    .await;
    again.assert_status_ok();
    assert_eq!(returned(&again)["versionId"], "3");
}

#[tokio::test]
async fn meta_add_honors_if_match() {
    let server = create_test_server();
    seed(&server, "m5", json!({"tag": [tag("a")]})).await;
    let response = server
        .post("/Task/m5/$meta-add")
        .add_header(X_TENANT_ID, tenant())
        .add_header(CONTENT_TYPE, fhir_json())
        .add_header(IF_MATCH, HeaderValue::from_static("W/\"9\""))
        .json(&parameters(json!({"tag": [tag("b")]})))
        .await;
    assert_eq!(response.status_code().as_u16(), 412);
    assert_eq!(codes(&read_meta(&server, "/Task/m5").await["tag"]), ["a"]);
}

#[tokio::test]
async fn malformed_requests_and_missing_resources_are_refused() {
    let server = create_test_server();
    seed(&server, "m6", json!({"tag": [tag("a")]})).await;

    for (body, why) in [
        (json!({"resourceType": "Task"}), "not Parameters"),
        (json!({"resourceType": "Parameters"}), "no meta parameter"),
        (
            json!({"resourceType": "Parameters", "parameter": [{"name": "meta", "valueString": "x"}]}),
            "no valueMeta",
        ),
        (parameters(json!({"tag": "x"})), "tag not a list"),
    ] {
        let response = invoke(&server, "/Task/m6/$meta-add", &body).await;
        assert_eq!(
            response.status_code().as_u16(),
            400,
            "{why}: {}",
            response.text()
        );
    }

    let missing = invoke(
        &server,
        "/Task/nope/$meta-delete",
        &parameters(json!({"tag": [tag("a")]})),
    )
    .await;
    assert_eq!(missing.status_code().as_u16(), 404);
    let missing = server
        .get("/Task/nope/$meta")
        .add_header(X_TENANT_ID, tenant())
        .await;
    assert_eq!(missing.status_code().as_u16(), 404);
}

#[tokio::test]
async fn system_and_type_level_meta_are_refused_with_a_4xx() {
    let server = create_test_server();
    for path in ["/$meta", "/Task/$meta"] {
        let response = server.get(path).add_header(X_TENANT_ID, tenant()).await;
        assert_eq!(
            response.status_code().as_u16(),
            400,
            "{path}: {}",
            response.text()
        );
        let outcome = response.json::<Value>();
        assert_eq!(
            outcome["issue"][0]["code"], "not-supported",
            "{path}: {outcome}"
        );
    }
}

#[tokio::test]
async fn capability_statement_advertises_the_meta_operations() {
    let server = create_test_server();
    let response = server
        .get("/metadata")
        .add_header(X_TENANT_ID, tenant())
        .await;
    response.assert_status_ok();
    let statement = response.json::<Value>();
    let names: Vec<&str> = statement["rest"][0]["operation"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|op| op["name"].as_str())
        .collect();
    for name in ["meta", "meta-add", "meta-delete"] {
        assert!(names.contains(&name), "{name} missing from {names:?}");
    }
}
