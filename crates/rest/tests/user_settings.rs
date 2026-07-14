//! End-to-end tests for the per-user UI settings endpoints
//! (`GET`/`PUT`/`PATCH /_user/settings`).
//!
//! These exercise the full REST stack — routing, the `UserKey` extractor, the
//! handlers, and the SQLite-backed [`SettingsStore`] — against an in-memory
//! database, with no authentication configured (so the caller resolves to the
//! `local|default` fallback key).

use std::sync::Arc;

use axum::body::Bytes;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::SettingsStore;
use helios_rest::ServerConfig;
use serde_json::{Value, json};

const IF_MATCH: HeaderName = HeaderName::from_static("if-match");
const IF_NONE_MATCH: HeaderName = HeaderName::from_static("if-none-match");
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

/// Builds a test server whose SQLite backend also hosts the settings store.
fn create_test_server() -> TestServer {
    let backend = SqliteBackend::in_memory().expect("create in-memory SQLite backend");
    backend.init_schema().expect("init schema");
    let backend = Arc::new(backend);

    let config = ServerConfig {
        base_url: "http://localhost:8080".to_string(),
        ..ServerConfig::for_testing()
    };

    let settings_store: Arc<dyn SettingsStore> = backend.clone();
    let state = helios_rest::AppState::new(backend, config).with_settings_store(settings_store);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    TestServer::new(app).expect("create test server")
}

/// Reads the `ETag` response header as an owned string.
fn etag(response: &axum_test::TestResponse) -> String {
    response
        .headers()
        .get("etag")
        .expect("ETag header present")
        .to_str()
        .expect("ETag is valid UTF-8")
        .to_string()
}

#[tokio::test]
async fn get_returns_empty_document_by_default() {
    let server = create_test_server();

    let response = server.get("/_user/settings").await;

    assert_eq!(response.status_code(), StatusCode::OK);
    assert_eq!(response.json::<Value>(), json!({}));
    assert_eq!(etag(&response), "W/\"0\"");
}

#[tokio::test]
async fn put_then_get_round_trips_document() {
    let server = create_test_server();
    let doc = json!({
        "theme": "dark",
        "defaultTenant": "acme",
        "activeFhirVersion": "R4",
        "recentQueries": {"Patient": ["name=smith"]}
    });

    let put = server.put("/_user/settings").json(&doc).await;
    assert_eq!(put.status_code(), StatusCode::OK);
    assert_eq!(etag(&put), "W/\"1\"");

    let get = server.get("/_user/settings").await;
    assert_eq!(get.status_code(), StatusCode::OK);
    assert_eq!(get.json::<Value>(), doc);
    assert_eq!(etag(&get), "W/\"1\"");
}

#[tokio::test]
async fn patch_merges_a_single_key_and_preserves_others() {
    let server = create_test_server();
    server
        .put("/_user/settings")
        .json(&json!({"theme": "dark", "defaultTenant": "acme"}))
        .await;

    // Toggle just the theme via JSON merge-patch.
    let patch = server
        .patch("/_user/settings")
        .json(&json!({"theme": "light"}))
        .await;
    assert_eq!(patch.status_code(), StatusCode::OK);
    assert_eq!(etag(&patch), "W/\"2\"");

    let get = server.get("/_user/settings").await;
    assert_eq!(
        get.json::<Value>(),
        json!({"theme": "light", "defaultTenant": "acme"})
    );
}

#[tokio::test]
async fn patch_null_deletes_a_key() {
    let server = create_test_server();
    server
        .put("/_user/settings")
        .json(&json!({"theme": "dark", "defaultTenant": "acme"}))
        .await;

    server
        .patch("/_user/settings")
        .json(&json!({"defaultTenant": null}))
        .await;

    let get = server.get("/_user/settings").await;
    assert_eq!(get.json::<Value>(), json!({"theme": "dark"}));
}

#[tokio::test]
async fn stale_if_match_is_rejected_with_412() {
    let server = create_test_server();
    server.put("/_user/settings").json(&json!({"a": 1})).await; // -> version 1

    // Precondition asserts "does not exist yet", but a document now exists.
    let conflict = server
        .put("/_user/settings")
        .add_header(IF_MATCH, HeaderValue::from_static("W/\"0\""))
        .json(&json!({"a": 2}))
        .await;

    assert_eq!(conflict.status_code(), StatusCode::PRECONDITION_FAILED);
}

#[tokio::test]
async fn matching_if_match_succeeds() {
    let server = create_test_server();
    server.put("/_user/settings").json(&json!({"a": 1})).await; // -> version 1

    let ok = server
        .put("/_user/settings")
        .add_header(IF_MATCH, HeaderValue::from_static("W/\"1\""))
        .json(&json!({"a": 2}))
        .await;

    assert_eq!(ok.status_code(), StatusCode::OK);
    assert_eq!(etag(&ok), "W/\"2\"");
}

#[tokio::test]
async fn non_object_body_is_rejected_with_400() {
    let server = create_test_server();

    let response = server
        .put("/_user/settings")
        .add_header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .bytes(Bytes::from_static(b"[1, 2, 3]"))
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
}

/// The exact sequence the web UI's theme toggle performs (#197): a merge-patch
/// of `{"theme": ...}` as the *first* write (no prior PUT — the document is
/// created on the fly), a reload's GET seeing the roamed value, and the
/// toggle back — all without touching any other settings key.
#[tokio::test]
async fn theme_toggle_round_trips_like_the_web_ui() {
    let server = create_test_server();
    // A pre-existing unrelated key, as another device may have written one.
    server
        .put("/_user/settings")
        .json(&json!({"defaultTenant": "acme"}))
        .await;

    // Toggle to dark: what theme.js sends on click.
    let patch = server
        .patch("/_user/settings")
        .json(&json!({"theme": "dark"}))
        .await;
    assert_eq!(patch.status_code(), StatusCode::OK);

    // Next page load: GET reconciles the cache with the server value.
    let get = server.get("/_user/settings").await;
    assert_eq!(
        get.json::<Value>(),
        json!({"theme": "dark", "defaultTenant": "acme"})
    );

    // Toggle back to light.
    server
        .patch("/_user/settings")
        .json(&json!({"theme": "light"}))
        .await;
    let get = server.get("/_user/settings").await;
    assert_eq!(get.json::<Value>()["theme"], json!("light"));
}

/// Same round-trip when no document exists at all yet: the very first
/// interaction a brand-new user has with the toggle must not 404/412.
#[tokio::test]
async fn theme_patch_creates_the_document_when_missing() {
    let server = create_test_server();

    let patch = server
        .patch("/_user/settings")
        .json(&json!({"theme": "dark"}))
        .await;
    assert_eq!(patch.status_code(), StatusCode::OK);

    let get = server.get("/_user/settings").await;
    assert_eq!(get.json::<Value>(), json!({"theme": "dark"}));
}

#[tokio::test]
async fn if_none_match_returns_304_for_unchanged_document() {
    let server = create_test_server();
    let put = server
        .put("/_user/settings")
        .json(&json!({"theme": "dark"}))
        .await;
    let current = etag(&put);

    let not_modified = server
        .get("/_user/settings")
        .add_header(
            IF_NONE_MATCH,
            HeaderValue::from_str(&current).expect("valid header"),
        )
        .await;

    assert_eq!(not_modified.status_code(), StatusCode::NOT_MODIFIED);
}
