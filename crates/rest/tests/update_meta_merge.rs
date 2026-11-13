//! An update merges `meta.tag` and `meta.security` instead of replacing them
//! (#2562).
//!
//! FHIR R4 §2.26.3.9 / R5 §2.1.27.5.3.10: on update, servers merge existing
//! and new tags, merge existing and new security labels, and replace existing
//! profiles with the new profiles. Each test drives one write path through the
//! router against a real SQLite backend, then reads the resource back.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
const TAGS: &str = "http://example.org/tags";
const CONFIDENTIALITY: &str = "http://terminology.hl7.org/CodeSystem/v3-Confidentiality";

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

fn task(id: &str, meta: Option<Value>) -> Value {
    let mut task = json!({
        "resourceType": "Task",
        "id": id,
        "status": "requested",
        "intent": "order"
    });
    if let Some(meta) = meta {
        task["meta"] = meta;
    }
    task
}

async fn put(server: &TestServer, url: &str, body: &Value) -> u16 {
    server
        .put(url)
        .add_header(X_TENANT_ID, tenant())
        .add_header(CONTENT_TYPE, fhir_json())
        .json(body)
        .await
        .status_code()
        .as_u16()
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

/// The issue's reproduction: a second PUT with tag `b` keeps tag `a`.
#[tokio::test]
async fn put_merges_tags_and_security_and_replaces_profiles() {
    let server = create_test_server();
    let first = json!({
        "tag": [tag("a")],
        "security": [{"system": CONFIDENTIALITY, "code": "R"}],
        "profile": ["http://example.org/StructureDefinition/old"]
    });
    assert_eq!(
        put(&server, "/Task/t1", &task("t1", Some(first))).await,
        201
    );

    let second = json!({
        "tag": [tag("b")],
        "profile": ["http://example.org/StructureDefinition/new"]
    });
    assert_eq!(
        put(&server, "/Task/t1", &task("t1", Some(second))).await,
        200
    );

    let meta = read_meta(&server, "/Task/t1").await;
    assert_eq!(codes(&meta["tag"]), ["a", "b"]);
    assert_eq!(codes(&meta["security"]), ["R"]);
    assert_eq!(
        meta["profile"],
        json!(["http://example.org/StructureDefinition/new"]),
        "profiles are replaced"
    );
    assert_eq!(meta["versionId"], "2");

    // The history entry for the new version holds the merged labels too.
    let v2 = read_meta(&server, "/Task/t1/_history/2").await;
    assert_eq!(codes(&v2["tag"]), ["a", "b"]);
    let v1 = read_meta(&server, "/Task/t1/_history/1").await;
    assert_eq!(codes(&v1["tag"]), ["a"], "the old version is unchanged");
}

#[tokio::test]
async fn put_without_meta_keeps_every_label() {
    let server = create_test_server();
    let meta = json!({"tag": [tag("a")], "security": [{"system": CONFIDENTIALITY, "code": "R"}]});
    assert_eq!(put(&server, "/Task/t2", &task("t2", Some(meta))).await, 201);
    assert_eq!(put(&server, "/Task/t2", &task("t2", None)).await, 200);

    let meta = read_meta(&server, "/Task/t2").await;
    assert_eq!(codes(&meta["tag"]), ["a"]);
    assert_eq!(codes(&meta["security"]), ["R"]);
}

#[tokio::test]
async fn resending_a_tag_does_not_duplicate_it() {
    let server = create_test_server();
    let meta = json!({"tag": [tag("a")]});
    assert_eq!(
        put(&server, "/Task/t3", &task("t3", Some(meta.clone()))).await,
        201
    );
    assert_eq!(put(&server, "/Task/t3", &task("t3", Some(meta))).await, 200);

    let meta = read_meta(&server, "/Task/t3").await;
    assert_eq!(codes(&meta["tag"]), ["a"]);
}

#[tokio::test]
async fn merged_tags_are_searchable() {
    let server = create_test_server();
    assert_eq!(
        put(
            &server,
            "/Task/t4",
            &task("t4", Some(json!({"tag": [tag("a")]})))
        )
        .await,
        201
    );
    assert_eq!(
        put(
            &server,
            "/Task/t4",
            &task("t4", Some(json!({"tag": [tag("b")]})))
        )
        .await,
        200
    );

    for code in ["a", "b"] {
        let response = server
            .get(&format!("/Task?_tag={TAGS}|{code}"))
            .add_header(X_TENANT_ID, tenant())
            .await;
        response.assert_status_ok();
        assert_eq!(
            response.json::<Value>()["entry"]
                .as_array()
                .map_or(0, Vec::len),
            1,
            "_tag={code} must find the updated Task"
        );
    }
}

#[tokio::test]
async fn conditional_put_merges_tags() {
    let server = create_test_server();
    let mut first = task("t5", Some(json!({"tag": [tag("a")]})));
    first["identifier"] = json!([{"system": "urn:test", "value": "t5"}]);
    assert_eq!(put(&server, "/Task/t5", &first).await, 201);

    let mut second = task("t5", Some(json!({"tag": [tag("b")]})));
    second["identifier"] = json!([{"system": "urn:test", "value": "t5"}]);
    assert_eq!(
        put(&server, "/Task?identifier=urn:test|t5", &second).await,
        200
    );

    let meta = read_meta(&server, "/Task/t5").await;
    assert_eq!(codes(&meta["tag"]), ["a", "b"]);
}

async fn post_bundle(server: &TestServer, kind: &str, id: &str) {
    let bundle = json!({
        "resourceType": "Bundle",
        "type": kind,
        "entry": [{
            "fullUrl": format!("http://localhost:8080/Task/{id}"),
            "resource": task(id, Some(json!({"tag": [tag("b")]}))),
            "request": {"method": "PUT", "url": format!("Task/{id}")}
        }]
    });
    let response = server
        .post("/")
        .add_header(X_TENANT_ID, tenant())
        .add_header(CONTENT_TYPE, fhir_json())
        .json(&bundle)
        .await;
    response.assert_status_ok();
    let status = response.json::<Value>()["entry"][0]["response"]["status"].clone();
    assert!(
        status.as_str().is_some_and(|s| s.starts_with("200")),
        "{kind} PUT entry: {status}"
    );
}

#[tokio::test]
async fn batch_put_merges_tags() {
    let server = create_test_server();
    assert_eq!(
        put(
            &server,
            "/Task/t6",
            &task("t6", Some(json!({"tag": [tag("a")]})))
        )
        .await,
        201
    );
    post_bundle(&server, "batch", "t6").await;
    assert_eq!(
        codes(&read_meta(&server, "/Task/t6").await["tag"]),
        ["a", "b"]
    );
}

#[tokio::test]
async fn transaction_put_merges_tags() {
    let server = create_test_server();
    assert_eq!(
        put(
            &server,
            "/Task/t7",
            &task("t7", Some(json!({"tag": [tag("a")]})))
        )
        .await,
        201
    );
    post_bundle(&server, "transaction", "t7").await;
    assert_eq!(
        codes(&read_meta(&server, "/Task/t7").await["tag"]),
        ["a", "b"]
    );
}
