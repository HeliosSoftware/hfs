//! #2562: an update merges the stored tags and security labels with the
//! submitted ones and replaces the profiles, on every update path; a PATCH
//! removal, a create and a restore take the body's labels as they are.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use serde_json::{Value, json};

const TAGS: &str = "http://example.org/tags";

async fn server() -> TestServer {
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
    let state = helios_rest::AppState::new(
        Arc::new(backend),
        ServerConfig {
            base_url: "http://localhost:8080".to_string(),
            ..ServerConfig::for_testing()
        },
    );
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).expect("test server")
}

fn tag(code: &str) -> Value {
    json!({"system": TAGS, "code": code})
}

fn task(id: &str, meta: Value) -> Value {
    json!({
        "resourceType": "Task", "id": id, "status": "requested", "intent": "order",
        "identifier": [{"system": "http://example.org/ids", "value": id}],
        "meta": meta
    })
}

fn task_without_meta(id: &str) -> Value {
    json!({
        "resourceType": "Task", "id": id, "status": "requested", "intent": "order",
        "identifier": [{"system": "http://example.org/ids", "value": id}]
    })
}

async fn tag_codes(server: &TestServer, id: &str) -> Vec<String> {
    let read = server.get(&format!("/Task/{id}")).await;
    read.assert_status_ok();
    let body: Value = read.json();
    body["meta"]["tag"]
        .as_array()
        .map(|tags| {
            tags.iter()
                .map(|t| t["code"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

async fn put(server: &TestServer, id: &str, body: Value) -> StatusCode {
    let response = server.put(&format!("/Task/{id}")).json(&body).await;
    let status = response.status_code();
    assert!(status.is_success(), "{}", response.text());
    status
}

async fn found_by_tag(server: &TestServer, code: &str) -> Vec<String> {
    let bundle: Value = server
        .get(&format!("/Task?_tag={TAGS}|{code}"))
        .await
        .json();
    bundle["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|e| e["resource"]["id"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The reproduction from the issue.
#[tokio::test]
async fn put_keeps_a_tag_the_update_leaves_out() {
    let server = server().await;
    put(&server, "t1", task("t1", json!({"tag": [tag("a")]}))).await;
    put(&server, "t1", task("t1", json!({"tag": [tag("b")]}))).await;
    assert_eq!(tag_codes(&server, "t1").await, ["a", "b"]);
    assert_eq!(found_by_tag(&server, "a").await, ["t1"]);
    assert_eq!(found_by_tag(&server, "b").await, ["t1"]);
}

#[tokio::test]
async fn put_without_meta_keeps_labels_and_drops_profiles() {
    let server = server().await;
    put(
        &server,
        "t1",
        task(
            "t1",
            json!({
                "tag": [tag("a")],
                "security": [{"system": "http://terminology.hl7.org/CodeSystem/v3-Confidentiality", "code": "R"}],
                "profile": ["http://example.org/StructureDefinition/p"]
            }),
        ),
    )
    .await;
    put(&server, "t1", task_without_meta("t1")).await;

    let body: Value = server.get("/Task/t1").await.json();
    assert_eq!(body["meta"]["tag"], json!([tag("a")]));
    assert_eq!(body["meta"]["security"][0]["code"], "R");
    assert!(body["meta"].get("profile").is_none(), "{body}");
    assert_eq!(body["meta"]["versionId"], "2");
    let profiled: Value = server
        .get("/Task?_profile=http://example.org/StructureDefinition/p")
        .await
        .json();
    assert!(profiled["entry"].as_array().is_none_or(Vec::is_empty));
    let secured: Value = server
        .get("/Task?_security=http://terminology.hl7.org/CodeSystem/v3-Confidentiality|R")
        .await
        .json();
    assert_eq!(secured["entry"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn put_with_if_match_merges() {
    let server = server().await;
    put(&server, "t1", task("t1", json!({"tag": [tag("a")]}))).await;
    let response = server
        .put("/Task/t1")
        .add_header("If-Match", "W/\"1\"")
        .json(&task("t1", json!({"tag": [tag("b")]})))
        .await;
    response.assert_status_ok();
    assert_eq!(tag_codes(&server, "t1").await, ["a", "b"]);
}

#[tokio::test]
async fn conditional_put_merges() {
    let server = server().await;
    put(&server, "t1", task("t1", json!({"tag": [tag("a")]}))).await;
    let response = server
        .put("/Task?identifier=http://example.org/ids|t1")
        .json(&task("t1", json!({"tag": [tag("b")]})))
        .await;
    response.assert_status_ok();
    assert_eq!(tag_codes(&server, "t1").await, ["a", "b"]);
}

fn bundle(kind: &str, entries: Vec<Value>) -> Value {
    json!({"resourceType": "Bundle", "type": kind, "entry": entries})
}

fn put_entry(url: &str, resource: Value, if_match: Option<&str>) -> Value {
    let mut request = json!({"method": "PUT", "url": url});
    if let Some(if_match) = if_match {
        request["ifMatch"] = json!(if_match);
    }
    json!({"resource": resource, "request": request})
}

#[tokio::test]
async fn batch_and_transaction_puts_merge() {
    let server = server().await;
    for id in ["b1", "b2", "b3", "x1", "x2"] {
        put(&server, id, task(id, json!({"tag": [tag("a")]}))).await;
    }
    let batch = server
        .post("/")
        .json(&bundle(
            "batch",
            vec![
                put_entry("Task/b1", task("b1", json!({"tag": [tag("b")]})), None),
                put_entry(
                    "Task/b2",
                    task("b2", json!({"tag": [tag("b")]})),
                    Some("W/\"1\""),
                ),
                put_entry(
                    "Task?identifier=http://example.org/ids|b3",
                    task("b3", json!({"tag": [tag("b")]})),
                    None,
                ),
            ],
        ))
        .await;
    batch.assert_status_ok();
    for entry in batch.json::<Value>()["entry"].as_array().unwrap() {
        assert!(
            entry["response"]["status"]
                .as_str()
                .unwrap()
                .starts_with("200"),
            "{entry}"
        );
    }

    let transaction = server
        .post("/")
        .json(&bundle(
            "transaction",
            vec![
                put_entry("Task/x1", task("x1", json!({"tag": [tag("b")]})), None),
                put_entry(
                    "Task?identifier=http://example.org/ids|x2",
                    task("x2", json!({"tag": [tag("b")]})),
                    None,
                ),
            ],
        ))
        .await;
    transaction.assert_status_ok();

    for id in ["b1", "b2", "b3", "x1", "x2"] {
        assert_eq!(tag_codes(&server, id).await, ["a", "b"], "{id}");
    }
}

#[tokio::test]
async fn a_patch_removal_stays_removed() {
    let server = server().await;
    put(
        &server,
        "t1",
        task("t1", json!({"tag": [tag("a"), tag("b")]})),
    )
    .await;
    let response = server
        .patch("/Task/t1")
        .json(&json!([{"op": "remove", "path": "/meta/tag/0"}]))
        .content_type("application/json-patch+json")
        .await;
    response.assert_status_ok();
    assert_eq!(tag_codes(&server, "t1").await, ["b"]);
    assert!(found_by_tag(&server, "a").await.is_empty());
}

#[tokio::test]
async fn create_update_as_create_and_restore_take_the_body_labels() {
    let server = server().await;
    let created = server
        .post("/Task")
        .json(&task_without_meta("ignored"))
        .await;
    created.assert_status(StatusCode::CREATED);

    assert_eq!(
        put(&server, "new", task("new", json!({"tag": [tag("n")]}))).await,
        StatusCode::CREATED
    );
    assert_eq!(tag_codes(&server, "new").await, ["n"]);

    put(&server, "gone", task("gone", json!({"tag": [tag("old")]}))).await;
    server
        .delete("/Task/gone")
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(
        put(&server, "gone", task("gone", json!({"tag": [tag("new")]}))).await,
        StatusCode::CREATED
    );
    assert_eq!(tag_codes(&server, "gone").await, ["new"]);
}

#[tokio::test]
async fn an_empty_label_list_does_not_clear_stored_labels() {
    let server = server().await;
    put(&server, "t1", task("t1", json!({"tag": [tag("a")]}))).await;
    put(&server, "t1", task("t1", json!({"tag": []}))).await;
    assert_eq!(tag_codes(&server, "t1").await, ["a"]);
}
