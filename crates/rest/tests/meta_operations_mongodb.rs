//! #2562 over a MongoDB-backed server: a PUT merges the stored tags and
//! security labels, `$meta` reads, and `$meta-add` / `$meta-delete` answer
//! `501` without writing and are not advertised.
//!
//! Needs `HFS_TEST_MONGODB_URL`; without it the test reports itself skipped.

#![cfg(feature = "mongodb")]

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use axum_test::TestServer;
use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};
use helios_persistence::core::Backend;
use helios_rest::ServerConfig;
use serde_json::{Value, json};

async fn server() -> Option<TestServer> {
    let url = std::env::var("HFS_TEST_MONGODB_URL").ok()?;
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .canonicalize()
        .expect("repo data dir");
    let backend = MongoBackend::new(MongoBackendConfig {
        connection_string: url,
        database_name: format!("rest_meta_ops_{}", uuid::Uuid::new_v4().simple()),
        data_dir: Some(data_dir),
        ..Default::default()
    })
    .expect("MongoBackend");
    backend
        .initialize()
        .await
        .expect("initialize MongoDB schema");
    let state = helios_rest::AppState::new(
        Arc::new(backend),
        ServerConfig {
            base_url: "http://localhost:8080".to_string(),
            ..ServerConfig::for_testing()
        },
    );
    Some(TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).expect("server"))
}

fn task(codes: &[&str]) -> Value {
    let tags: Vec<Value> = codes
        .iter()
        .map(|c| json!({"system": "http://example.org/tags", "code": c}))
        .collect();
    json!({"resourceType": "Task", "id": "t1", "status": "requested", "intent": "order",
           "meta": {"tag": tags}})
}

#[tokio::test]
async fn mongodb_merges_on_put_and_refuses_metadata_writes() {
    let Some(server) = server().await else {
        eprintln!(
            "Skipping mongodb_merges_on_put_and_refuses_metadata_writes (requires HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    server.put("/Task/t1").json(&task(&["a"])).await;
    server
        .put("/Task/t1")
        .json(&task(&["b"]))
        .await
        .assert_status_ok();
    let body: Value = server.get("/Task/t1").await.json();
    assert_eq!(
        body["meta"]["tag"].as_array().map(Vec::len),
        Some(2),
        "{body}"
    );

    let meta: Value = server.get("/Task/t1/$meta").await.json();
    assert_eq!(meta["parameter"][0]["valueMeta"]["versionId"], "2");

    server
        .post("/Task/t1/$meta-delete")
        .json(&json!({"resourceType": "Parameters", "parameter": [{
            "name": "meta",
            "valueMeta": {"tag": [{"system": "http://example.org/tags", "code": "a"}]}
        }]}))
        .await
        .assert_status(StatusCode::NOT_IMPLEMENTED);
    let after: Value = server.get("/Task/t1").await.json();
    assert_eq!(after, body);

    let statement: Value = server.get("/metadata").await.json();
    let names: Vec<&str> = statement["rest"][0]["operation"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|op| op["name"].as_str())
        .collect();
    assert!(names.contains(&"meta"), "{names:?}");
    assert!(!names.contains(&"meta-add"), "{names:?}");
    assert!(!names.contains(&"meta-delete"), "{names:?}");

    // A PATCH still removes a tag explicitly.
    server
        .patch("/Task/t1")
        .json(&json!([{"op": "remove", "path": "/meta/tag/0"}]))
        .content_type("application/json-patch+json")
        .await
        .assert_status_ok();
    let patched: Value = server.get("/Task/t1").await.json();
    assert_eq!(patched["meta"]["tag"].as_array().map(Vec::len), Some(1));
}
