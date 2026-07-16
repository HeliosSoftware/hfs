//! T2 cluster test for HTS terminology cache cross-instance invalidation
//! (C3): two independently constructed `AppState`/`PostgresTerminologyBackend`
//! handles over one shared PostgreSQL database, exercised over real HTTP
//! (`server::create_app`) so both the `AppState`-layer handler caches and the
//! `PostgresTerminologyBackend`-layer response caches are covered in one
//! request, per `docs/cluster-testing-strategy.md`'s T2 `invalidation` DoD
//! row.
//!
//! Run with:
//!   `cargo test -p helios-hts --features postgres --test postgres_epoch_cluster`

#![cfg(feature = "postgres")]

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use helios_hts::backends::PostgresTerminologyBackend;
use helios_hts::config::HtsConfig;
use helios_hts::state::AppState;
use helios_persistence::backends::postgres::PostgresBackend;
use serde_json::Value;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::OnceCell;
use tower::ServiceExt;

const LABEL_KEY: &str = "io.helios.hts.test-pool";
const LABEL_VALUE: &str = "hts-epoch-cluster-pg";

static CONTAINER: OnceLock<ContainerAsync<Postgres>> = OnceLock::new();
static DB_URL: OnceCell<String> = OnceCell::const_new();

#[ctor::dtor]
fn cleanup_container() {
    let filter = format!("label={LABEL_KEY}={LABEL_VALUE}");
    let Ok(listing) = std::process::Command::new("docker")
        .args(["ps", "-aq", "--filter", &filter])
        .output()
    else {
        return;
    };
    let ids = String::from_utf8_lossy(&listing.stdout);
    for id in ids.split_whitespace() {
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", id])
            .output();
    }
}

async fn db_url() -> &'static str {
    DB_URL
        .get_or_init(|| async {
            use testcontainers::{ImageExt, runners::AsyncRunner};
            let container = Postgres::default()
                .with_label(LABEL_KEY, LABEL_VALUE)
                .start()
                .await
                .expect("Failed to start Postgres container");
            let host = container.get_host().await.expect("get host");
            let port = container.get_host_port_ipv4(5432).await.expect("get port");
            let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
            let _ = CONTAINER.set(container);
            url
        })
        .await
}

/// Builds one independent instance's router: a fresh `PostgresTerminologyBackend`
/// + `AppState`, wired for epoch-mode cache invalidation with a zero memo
/// window (deterministic — no sleep-and-hope waiting out a real ~1s memo
/// window).
async fn fresh_router(cache_invalidation: &str) -> Router {
    let url = db_url().await;

    let backend = PostgresTerminologyBackend::new(url)
        .await
        .expect("backend should initialize")
        .with_epoch_guard(cache_invalidation == "epoch", Duration::ZERO);
    let epoch_guard = backend.epoch_guard();

    let resource_store = PostgresBackend::from_connection_string(url)
        .await
        .expect("resource store should open");
    resource_store
        .init_schema()
        .await
        .expect("resource store schema should initialize");

    let state = AppState::new(backend.clone())
        .with_resource_store_pg(resource_store)
        .with_terminology_importer(Arc::new(backend))
        .with_epoch_guard(epoch_guard);

    let config = HtsConfig {
        terminology_cache_invalidation: cache_invalidation.to_string(),
        ..HtsConfig::default()
    };
    helios_hts::server::create_app(&config, state)
}

async fn post_fhir(app: &Router, path: &str, body: String) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/fhir+json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn put_fhir(app: &Router, path: &str, body: String) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(path)
                .header("content-type", "application/fhir+json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn get_fhir(app: &Router, path: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn lookup_display(body: &Value) -> Option<&str> {
    body["parameter"]
        .as_array()?
        .iter()
        .find(|p| p["name"] == "display")?
        .get("valueString")?
        .as_str()
}

/// `invalidation` DoD row: import via A, warm both of B's cache layers
/// (handler-level `lookup_handler_cache` and backend-level
/// `lookup_response_cache`) with the pre-update display, update the same
/// resource via A, and assert B serves the fresh display — without B ever
/// having been told directly. Covers both disjoint cache layers in one
/// request per read, since `process_lookup` checks the AppState layer before
/// falling through to `PostgresTerminologyBackend::lookup`'s backend-layer
/// check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_integration_cluster_epoch_invalidates_stale_lookup_across_instances() {
    let a = fresh_router("epoch").await;
    let b = fresh_router("epoch").await;

    let uid = uuid::Uuid::new_v4().simple().to_string();
    let url = format!("http://epoch-cluster-test.example/{uid}");

    let cs_v1 = serde_json::json!({
        "resourceType": "CodeSystem",
        "url": url,
        "version": "1.0",
        "name": "EpochClusterCS",
        "status": "active",
        "content": "complete",
        "concept": [{"code": "widget", "display": "Old Display"}]
    })
    .to_string();
    let (status, body) = post_fhir(&a, "/CodeSystem", cs_v1).await;
    assert_eq!(status, StatusCode::CREATED, "create via A: {body}");
    let id = body["id"].as_str().expect("id field missing").to_owned();

    // Warm B's caches (both layers) with the pre-update display.
    let lookup_path = format!("/CodeSystem/$lookup?system={url}&code=widget&version=1.0");
    let (status, body) = get_fhir(&b, &lookup_path).await;
    assert_eq!(status, StatusCode::OK, "warm read via B: {body}");
    assert_eq!(lookup_display(&body), Some("Old Display"));

    // Update via A — same (url, version), different display, so this
    // overwrites the same code_systems/concepts rows rather than creating a
    // second version row.
    let cs_v1_updated = serde_json::json!({
        "resourceType": "CodeSystem",
        "id": id,
        "url": url,
        "version": "1.0",
        "name": "EpochClusterCS",
        "status": "active",
        "content": "complete",
        "concept": [{"code": "widget", "display": "New Display"}]
    })
    .to_string();
    let (status, body) = put_fhir(&a, &format!("/CodeSystem/{id}"), cs_v1_updated).await;
    assert_eq!(status, StatusCode::OK, "update via A: {body}");

    // B never saw the update directly — only the epoch check on its next
    // read should invalidate its stale caches.
    let (status, body) = get_fhir(&b, &lookup_path).await;
    assert_eq!(status, StatusCode::OK, "post-update read via B: {body}");
    assert_eq!(
        lookup_display(&body),
        Some("New Display"),
        "B should serve the fresh display after A's update, not its \
         pre-update cached value — the epoch check must have cleared both \
         B's handler-level and backend-level lookup caches"
    );
}

/// The `local` mode's unsafe contract: without the epoch mechanism, B keeps
/// serving the pre-update value indefinitely — documenting exactly what
/// `epoch` mode fixes above.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_integration_cluster_local_mode_serves_stale_lookup_across_instances() {
    let a = fresh_router("local").await;
    let b = fresh_router("local").await;

    let uid = uuid::Uuid::new_v4().simple().to_string();
    let url = format!("http://epoch-cluster-test.example/{uid}");

    let cs_v1 = serde_json::json!({
        "resourceType": "CodeSystem",
        "url": url,
        "version": "1.0",
        "name": "LocalModeCS",
        "status": "active",
        "content": "complete",
        "concept": [{"code": "widget", "display": "Old Display"}]
    })
    .to_string();
    let (status, body) = post_fhir(&a, "/CodeSystem", cs_v1).await;
    assert_eq!(status, StatusCode::CREATED, "create via A: {body}");
    let id = body["id"].as_str().expect("id field missing").to_owned();

    let lookup_path = format!("/CodeSystem/$lookup?system={url}&code=widget&version=1.0");
    let (status, body) = get_fhir(&b, &lookup_path).await;
    assert_eq!(status, StatusCode::OK, "warm read via B: {body}");
    assert_eq!(lookup_display(&body), Some("Old Display"));

    let cs_v1_updated = serde_json::json!({
        "resourceType": "CodeSystem",
        "id": id,
        "url": url,
        "version": "1.0",
        "name": "LocalModeCS",
        "status": "active",
        "content": "complete",
        "concept": [{"code": "widget", "display": "New Display"}]
    })
    .to_string();
    let (status, body) = put_fhir(&a, &format!("/CodeSystem/{id}"), cs_v1_updated).await;
    assert_eq!(status, StatusCode::OK, "update via A: {body}");

    let (status, body) = get_fhir(&b, &lookup_path).await;
    assert_eq!(status, StatusCode::OK, "post-update read via B: {body}");
    assert_eq!(
        lookup_display(&body),
        Some("Old Display"),
        "under local mode (the default), B has no way to learn about A's \
         update and must keep serving its stale cached value — this is the \
         unsafe contract epoch mode exists to fix"
    );
}
