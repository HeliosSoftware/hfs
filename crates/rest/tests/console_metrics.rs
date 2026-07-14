//! Integration tests for the management-console metrics HTTP handlers.
//!
//! Exercises every `/console/metrics/*` endpoint against an in-memory
//! SQLite-backed test server (auth disabled — the default — so all console
//! tiers, including the cross-tenant `tenants`/`traffic` endpoints, are
//! reachable). Assertions check response shape and the stable enum-string
//! fields (e.g. `scope`, `service`, `source`), not values that vary between
//! runs (uptime numbers, timestamps).
//!
//! The harness mirrors `tenant_resolution.rs::create_test_server`: same
//! `SqliteBackend::with_config(":memory:", ...)` + `init_schema()` +
//! `ServerConfig` construction and `AppState`. Because the console routers are
//! merged in `build_app` (not in `fhir_routes::create_routes`), this harness
//! reproduces the auth-disabled merge from `helios_rest::lib` by merging the
//! three console routers onto the FHIR routes with no auth layer.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};

const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

/// Creates an in-memory SQLite-backed console test server with auth disabled.
///
/// Mirrors `tenant_resolution.rs::create_test_server`, then additionally merges
/// the console-metrics routers (public + protected + admin) exactly as
/// `helios_rest::build_app` does when auth is disabled — a plain `merge` with no
/// auth layer — so all `/console/metrics/*` endpoints are reachable.
async fn create_test_server() -> (TestServer, Arc<SqliteBackend>) {
    // Configure with data directory to load spec SearchParameters.
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

    // FHIR routes plus the three console tiers, merged as in `build_app`'s
    // auth-disabled path (no auth/admin middleware layered on).
    let router = helios_rest::routing::fhir_routes::create_routes(state.clone());
    let router = router.merge(helios_rest::routing::console_metrics::public_routes(
        state.clone(),
    ));
    let router = router.merge(helios_rest::routing::console_metrics::protected_routes(
        state.clone(),
    ));
    let router = router.merge(helios_rest::routing::console_metrics::admin_routes(state));

    let server = TestServer::new(router).expect("Failed to create test server");

    (server, backend)
}

/// Seeds a resource through the normal REST create path (POST /[type]) so the
/// DB-backed console endpoints have non-trivial data (rows + write history).
async fn seed(server: &TestServer, resource_type: &str, resource: serde_json::Value) {
    let response = server
        .post(&format!("/{resource_type}"))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&resource)
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
}

/// Seeds a couple of Patients and an Observation for the default tenant.
async fn seed_default_tenant(server: &TestServer) {
    seed(
        server,
        "Patient",
        serde_json::json!({
            "resourceType": "Patient",
            "name": [{ "family": "Smith" }]
        }),
    )
    .await;
    seed(
        server,
        "Patient",
        serde_json::json!({
            "resourceType": "Patient",
            "name": [{ "family": "Jones" }]
        }),
    )
    .await;
    seed(
        server,
        "Observation",
        serde_json::json!({
            "resourceType": "Observation",
            "status": "final",
            "code": { "text": "test observation" }
        }),
    )
    .await;
}

// =============================================================================
// uptime (public)
// =============================================================================

#[tokio::test]
async fn test_uptime_shape_and_no_infra_leak() {
    let (server, _backend) = create_test_server().await;

    let response = server.get("/console/metrics/uptime").await;
    response.assert_status_ok();

    let body: serde_json::Value = response.json();
    assert_eq!(body["service"], "hfs");
    assert_eq!(body["scope"], "single-instance");
    assert!(body["version"].is_string(), "version should be present");
    assert!(
        body["uptime_seconds"].is_number(),
        "uptime_seconds should be a number"
    );
    assert!(
        body["started_at"].is_string(),
        "started_at should be present"
    );

    // Public, unauthenticated endpoint must not leak infrastructure identity.
    // Check the whole key set against an allowlist so a regression that adds a
    // hostname under ANY key (instance / backend / host / hostname / node / …)
    // fails, not just the two documented names.
    let allowed = [
        "service",
        "scope",
        "version",
        "started_at",
        "now",
        "uptime_seconds",
        "uptime_human",
        "availability",
    ];
    for key in body.as_object().expect("uptime body is an object").keys() {
        assert!(
            allowed.contains(&key.as_str()),
            "uptime exposed an unexpected field `{key}` (possible infra leak)"
        );
    }
}

// =============================================================================
// resource-counts (tenant-scoped)
// =============================================================================

#[tokio::test]
async fn test_resource_counts_shape() {
    let (server, _backend) = create_test_server().await;
    seed_default_tenant(&server).await;

    let response = server.get("/console/metrics/resource-counts").await;
    response.assert_status_ok();

    let body: serde_json::Value = response.json();
    assert!(body["tenant"].is_string(), "tenant should be present");
    assert_eq!(body["scope"], "single-instance");
    assert!(
        body["total_resources"].is_number(),
        "total_resources should be a number"
    );
    assert!(body["series"].is_array(), "series should be an array");
    // At least the three seeded resources (2 Patient + 1 Observation).
    assert!(body["total_resources"].as_u64().unwrap() >= 3);
}

#[tokio::test]
async fn test_resource_counts_with_days_and_types_filter() {
    let (server, _backend) = create_test_server().await;
    seed_default_tenant(&server).await;

    let response = server
        .get("/console/metrics/resource-counts?days=7&types=Patient")
        .await;
    response.assert_status_ok();

    let body: serde_json::Value = response.json();
    let series = body["series"].as_array().expect("series is an array");
    // Only the requested type is charted.
    assert_eq!(series.len(), 1);
    assert_eq!(series[0]["resource_type"], "Patient");
    assert_eq!(body["window"]["days"].as_i64().unwrap(), 7);
}

#[tokio::test]
async fn test_resource_counts_days_clamped() {
    let (server, _backend) = create_test_server().await;

    // A stray oversized window must not force an unbounded scan; it is clamped.
    let response = server
        .get("/console/metrics/resource-counts?days=100000")
        .await;
    response.assert_status_ok();

    let body: serde_json::Value = response.json();
    assert_eq!(body["window"]["days"].as_i64().unwrap(), 365);
}

// =============================================================================
// activity (tenant-scoped)
// =============================================================================

#[tokio::test]
async fn test_activity_shape() {
    let (server, _backend) = create_test_server().await;
    seed_default_tenant(&server).await;

    let response = server.get("/console/metrics/activity").await;
    response.assert_status_ok();

    let body: serde_json::Value = response.json();
    assert_eq!(body["source"], "writes");
    assert_eq!(body["scope"], "single-instance");
    let cells = body["cells"].as_array().expect("cells is an array");
    // Dense 7 weekdays x 24 hours grid.
    assert_eq!(cells.len(), 168);
}

// =============================================================================
// resource-distribution (tenant-scoped)
// =============================================================================

#[tokio::test]
async fn test_resource_distribution_shape() {
    let (server, _backend) = create_test_server().await;
    seed_default_tenant(&server).await;

    let response = server.get("/console/metrics/resource-distribution").await;
    response.assert_status_ok();

    let body: serde_json::Value = response.json();
    assert!(body["types"].is_array(), "types should be an array");
    assert_eq!(body["scope"], "single-instance");
    assert!(body["total_resources"].as_u64().unwrap() >= 3);
}

#[tokio::test]
async fn test_resource_distribution_top_param() {
    let (server, _backend) = create_test_server().await;
    seed_default_tenant(&server).await;

    let response = server
        .get("/console/metrics/resource-distribution?top=3")
        .await;
    response.assert_status_ok();

    let body: serde_json::Value = response.json();
    assert!(body["types"].is_array());
}

#[tokio::test]
async fn test_resource_distribution_rolls_up_tail_into_other() {
    let (server, _backend) = create_test_server().await;
    // Seed more distinct types than `top` so the handler must truncate the tail
    // into a synthetic `other` bucket (the `counts.len() > top` branch).
    seed_default_tenant(&server).await; // Patient (x2) + Observation
    seed(
        &server,
        "Encounter",
        serde_json::json!({ "resourceType": "Encounter", "status": "finished" }),
    )
    .await;

    // top=1 keeps only the busiest type; the other two collapse into `other`.
    let response = server
        .get("/console/metrics/resource-distribution?top=1")
        .await;
    response.assert_status_ok();

    let body: serde_json::Value = response.json();
    let types = body["types"].as_array().expect("types is an array");
    // One charted type plus the rolled-up `other` row.
    assert_eq!(types.len(), 2);
    let other = types
        .iter()
        .find(|t| t["resource_type"] == "other")
        .expect("an `other` roll-up row must be present");
    // Two distinct types were folded into the tail.
    assert_eq!(other["types"].as_u64().unwrap(), 2);
    assert!(other["count"].as_u64().unwrap() >= 2);
    // distinct_types still reflects the true count, not the truncated view.
    assert_eq!(body["distinct_types"].as_u64().unwrap(), 3);
}

// =============================================================================
// tenants (cross-tenant / admin; reachable with auth disabled)
// =============================================================================

#[tokio::test]
async fn test_tenants_shape() {
    let (server, _backend) = create_test_server().await;
    seed_default_tenant(&server).await;

    let response = server.get("/console/metrics/tenants").await;
    response.assert_status_ok();

    let body: serde_json::Value = response.json();
    assert!(body["tenants"].is_array(), "tenants should be an array");
    assert_eq!(body["resources_scope"], "single-instance");
    assert_eq!(body["traffic_scope"], "single-instance");
    assert!(body["instance"].is_string(), "instance should be present");
}

// =============================================================================
// traffic (cross-tenant / admin; reachable with auth disabled)
// =============================================================================

#[tokio::test]
async fn test_traffic_shape() {
    let (server, _backend) = create_test_server().await;

    // This test router has no request-tracking middleware, so the in-process
    // request log stays empty and the traffic fields default to 0 — assert shape,
    // not values.
    let response = server.get("/console/metrics/traffic").await;
    response.assert_status_ok();

    let body: serde_json::Value = response.json();
    assert!(
        body["requests_per_second"].is_number(),
        "requests_per_second should be a number"
    );
    assert!(
        body["latency_ms"].is_object(),
        "latency_ms should be an object"
    );
    assert!(
        body["latency_ms"]["p50"].is_number(),
        "latency_ms.p50 should be present"
    );
    assert!(
        body["status_classes"].is_object(),
        "status_classes should be an object"
    );
    assert_eq!(body["scope"], "single-instance");
    assert!(body["instance"].is_string(), "instance should be present");
}

#[tokio::test]
async fn test_tenants_includes_traffic_only_tenant() {
    let (server, _backend) = create_test_server().await;
    seed_default_tenant(&server).await;

    // A tenant that has recent traffic but no stored resources must still appear
    // in the roster (the "seen only in traffic" branch), reporting `resources: 0`
    // alongside its live rates. Inject the traffic directly into the in-process
    // request log the handler reads — the test router has no request-tracking
    // middleware, so HTTP calls alone would leave the log empty.
    let ghost = "ghost-traffic-tenant";
    for _ in 0..3 {
        helios_observability::reqlog::record(200, 0.010, ghost);
    }

    let response = server.get("/console/metrics/tenants").await;
    response.assert_status_ok();

    let body: serde_json::Value = response.json();
    let tenants = body["tenants"].as_array().expect("tenants is an array");
    let row = tenants
        .iter()
        .find(|t| t["tenant"] == ghost)
        .expect("a traffic-only tenant must still be listed");
    // No resources stored for this tenant, but traffic was observed.
    assert_eq!(row["resources"].as_u64().unwrap(), 0);
    assert!(row["requests_per_second"].as_f64().unwrap() > 0.0);
}

#[tokio::test]
async fn test_traffic_window_param() {
    let (server, _backend) = create_test_server().await;

    let response = server.get("/console/metrics/traffic?window=120").await;
    response.assert_status_ok();

    let body: serde_json::Value = response.json();
    assert_eq!(body["window_seconds"].as_i64().unwrap(), 120);
}
