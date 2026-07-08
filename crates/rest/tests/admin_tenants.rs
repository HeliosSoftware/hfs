//! Integration tests for the tenant-maintenance admin API
//! (`GET`/`POST`/`DELETE /admin/tenants`).
//!
//! Exercises the full REST stack against an in-memory SQLite-backed test server
//! with auth disabled (the default), so the admin tier is reachable without a
//! system-context token. The harness mirrors `console_metrics.rs`: it merges the
//! FHIR routes with the admin-tenant router exactly as `build_app` does on the
//! auth-disabled path.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
const TENANT_HEADER: HeaderName = HeaderName::from_static("x-tenant-id");

async fn create_test_server() -> TestServer {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let backend_config = SqliteBackendConfig {
        data_dir: Some(data_dir),
        ..Default::default()
    };
    let backend = SqliteBackend::with_config(":memory:", backend_config)
        .expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to init schema");
    let backend = Arc::new(backend);

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "default-tenant".to_string(),
        ..ServerConfig::for_testing()
    };

    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let router = helios_rest::routing::fhir_routes::create_routes(state.clone())
        .merge(helios_rest::routing::admin_tenants::routes(state));
    TestServer::new(router).expect("Failed to create test server")
}

/// Seeds a resource for a specific tenant via the normal REST create path.
async fn seed_for(server: &TestServer, tenant: &str, resource_type: &str) {
    let response = server
        .post(&format!("/{resource_type}"))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .add_header(TENANT_HEADER, HeaderValue::from_str(tenant).unwrap())
        .json(&json!({ "resourceType": resource_type }))
        .await;
    response.assert_status(StatusCode::CREATED);
}

fn tenants(body: &Value) -> &Vec<Value> {
    body["tenants"].as_array().expect("tenants array")
}

fn find<'a>(body: &'a Value, id: &str) -> Option<&'a Value> {
    tenants(body).iter().find(|t| t["id"] == id)
}

#[tokio::test]
async fn list_is_empty_before_any_tenant() {
    let server = create_test_server().await;
    let res = server.get("/admin/tenants").await;
    res.assert_status(StatusCode::OK);
    let body = res.json::<Value>();
    assert_eq!(body["tenant_count"], 0);
    assert!(tenants(&body).is_empty());
}

#[tokio::test]
async fn create_then_list_round_trips() {
    let server = create_test_server().await;

    let created = server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme", "display_name": "Acme Health" }))
        .await;
    created.assert_status(StatusCode::CREATED);
    let rec = created.json::<Value>();
    assert_eq!(rec["id"], "acme");
    assert_eq!(rec["display_name"], "Acme Health");
    assert_eq!(rec["registered"], true);
    assert!(rec["created_at"].as_str().is_some_and(|s| !s.is_empty()));

    let list = server.get("/admin/tenants").await.json::<Value>();
    assert_eq!(list["tenant_count"], 1);
    let acme = find(&list, "acme").expect("acme present");
    assert_eq!(acme["display_name"], "Acme Health");
    assert_eq!(acme["resources"], 0);
}

#[tokio::test]
async fn duplicate_create_conflicts() {
    let server = create_test_server().await;
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme" }))
        .await
        .assert_status(StatusCode::CREATED);
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme" }))
        .await
        .assert_status(StatusCode::CONFLICT);
}

#[tokio::test]
async fn invalid_ids_are_rejected() {
    let server = create_test_server().await;
    for bad in ["", "has space", "__system__", "bad*char"] {
        let res = server
            .post("/admin/tenants")
            .json(&json!({ "id": bad }))
            .await;
        res.assert_status(StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn list_includes_data_only_tenants_with_counts() {
    let server = create_test_server().await;
    // A tenant that has data but was never registered.
    seed_for(&server, "beta", "Patient").await;
    seed_for(&server, "beta", "Observation").await;
    // A registered tenant with data.
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme" }))
        .await
        .assert_status(StatusCode::CREATED);
    seed_for(&server, "acme", "Patient").await;

    let list = server.get("/admin/tenants").await.json::<Value>();

    let acme = find(&list, "acme").expect("acme");
    assert_eq!(acme["registered"], true);
    assert_eq!(acme["resources"], 1);
    assert!(acme["created_at"].as_str().is_some());

    let beta = find(&list, "beta").expect("beta discovered from data");
    assert_eq!(beta["registered"], false);
    assert_eq!(beta["resources"], 2);
    assert!(beta["created_at"].is_null());
}

#[tokio::test]
async fn delete_deregisters_without_purge_by_default() {
    let server = create_test_server().await;
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme" }))
        .await
        .assert_status(StatusCode::CREATED);
    seed_for(&server, "acme", "Patient").await;

    let del = server.delete("/admin/tenants/acme").await;
    del.assert_status(StatusCode::OK);
    let body = del.json::<Value>();
    assert_eq!(body["deregistered"], true);
    assert_eq!(body["purged"], false);
    assert!(body["resources_removed"].is_null());

    // Deregistered, but the data survives, so it now shows as data-only.
    let list = server.get("/admin/tenants").await.json::<Value>();
    let acme = find(&list, "acme").expect("still discoverable via data");
    assert_eq!(acme["registered"], false);
    assert_eq!(acme["resources"], 1);
}

#[tokio::test]
async fn delete_with_purge_tears_down_data() {
    let server = create_test_server().await;
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme" }))
        .await
        .assert_status(StatusCode::CREATED);
    seed_for(&server, "acme", "Patient").await;
    seed_for(&server, "acme", "Patient").await;

    let del = server.delete("/admin/tenants/acme?purge=true").await;
    del.assert_status(StatusCode::OK);
    let body = del.json::<Value>();
    assert_eq!(body["purged"], true);
    assert_eq!(body["resources_removed"], 2);

    // Gone entirely: no registration and no data.
    let list = server.get("/admin/tenants").await.json::<Value>();
    assert!(find(&list, "acme").is_none());
}

#[tokio::test]
async fn delete_unknown_tenant_is_404() {
    let server = create_test_server().await;
    server
        .delete("/admin/tenants/ghost")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}
