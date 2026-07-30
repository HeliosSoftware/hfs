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
        // These tests assert exact per-tenant resource counts, so don't let
        // provisioning seed the conformance set on top.
        seed_conformance: false,
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

/// Ids naming a control-plane namespace are refused at creation (issue #271).
///
/// Still defence in depth rather than what makes the S3 keyspace safe — that is
/// structural — but it is no longer admin-API-only. Since issue #385 the same
/// rule is applied by `TenantId::parse` at every ingress, including the JWT
/// claim, and again by `ResourceStorage::register_tenant` on all backends.
#[tokio::test]
async fn control_plane_namespace_ids_are_reserved() {
    let server = create_test_server().await;
    for reserved in ["tenants", "resources", "history", "bulk"] {
        let res = server
            .post("/admin/tenants")
            .json(&json!({ "id": reserved }))
            .await;
        res.assert_status(StatusCode::BAD_REQUEST);
    }
}

/// A reserved name is reserved in **every** segment, not just as the whole id
/// (issue #385).
///
/// `acme/resources` passed the old whole-id check. On S3 its objects land under
/// `acme/resources/…`, which is inside the prefix tenant `acme` lists — and that
/// `purge_tenant_data("acme")` deletes. Two tenants, one keyspace, with the
/// deletion crossing the boundary.
#[tokio::test]
async fn reserved_names_are_rejected_in_any_hierarchy_segment() {
    let server = create_test_server().await;
    for bad in [
        "acme/resources",
        "acme/history",
        "acme/bulk",
        "acme/tenants",
        "acme/__system__",
        "acme/../evil",
    ] {
        server
            .post("/admin/tenants")
            .json(&json!({ "id": bad }))
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }

    // Only whole segments are reserved — a substring must stay usable.
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme/resources-archive" }))
        .await
        .assert_status(StatusCode::CREATED);
}

/// The length cap is the canonical 64, not the 128 this handler used to apply
/// on its own (issue #385).
///
/// An id in 65..=128 bytes was registrable but unroutable: both the header and
/// URL-prefix validators capped at 64, so it could only ever be reached through
/// the JWT claim — the ingress that validated nothing at all.
#[tokio::test]
async fn ids_longer_than_the_canonical_cap_are_rejected() {
    let server = create_test_server().await;

    server
        .post("/admin/tenants")
        .json(&json!({ "id": "a".repeat(64) }))
        .await
        .assert_status(StatusCode::CREATED);

    server
        .post("/admin/tenants")
        .json(&json!({ "id": "a".repeat(65) }))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

/// Malformed hierarchy is rejected: `/` is a separator, so it cannot lead,
/// trail, or repeat.
#[tokio::test]
async fn malformed_hierarchy_is_rejected() {
    let server = create_test_server().await;
    for bad in ["/acme", "acme/", "acme//research"] {
        server
            .post("/admin/tenants")
            .json(&json!({ "id": bad }))
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
}

/// The rejection body names the reason, so an operator can act on it without
/// reading the source.
#[tokio::test]
async fn rejection_explains_why() {
    let server = create_test_server().await;
    let body = server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme corp" }))
        .await
        .text();
    assert!(
        body.contains("letters, digits"),
        "expected an actionable reason, got: {body}"
    );
}

/// Hierarchical ids stay valid — `TenantId` models `/` as the hierarchy
/// separator (`is_descendant_of`, `parent`, `ancestors`), so tightening the
/// charset here would contradict a shipped feature of the domain type.
///
/// This used to cite `SharedSchemaStrategy` as the second justification; that
/// type was unwired scaffolding and was removed in #370, so the domain model is
/// now the whole reason. Note what the removed code got wrong, because any
/// future per-tenant schema/bucket/database naming scheme must not repeat it:
/// its tenant→identifier mapping was **not injective** — it folded `/` and `-`
/// to `_` and lowercased, so `acme/research` and `acme_research` (and `Acme`
/// and `acme`) collided onto one identifier. Two distinct tenants sharing one
/// container is a cross-tenant data breach. Any such mapping must be injective
/// (percent-encode, as `S3Keyspace::registry_object_id` does) and must not use
/// `DefaultHasher`, whose output is not stable across Rust releases.
#[tokio::test]
async fn hierarchical_ids_are_still_accepted() {
    let server = create_test_server().await;
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme/research" }))
        .await
        .assert_status(StatusCode::CREATED);
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
