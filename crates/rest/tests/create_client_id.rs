//! #647: per http.html#create "the server ignores the id provided in the
//! resource" — a POST always gets a fresh server id. Honoring the client id
//! made a second POST of the same document 409, which failed whole Synthea
//! transaction bundles repeating shared Organizations/Practitioners.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

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
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(backend, config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    TestServer::new(app).expect("Failed to create test server")
}

fn org(id: &str) -> Value {
    json!({
        "resourceType": "Organization",
        "id": id,
        "name": "Shared Org",
    })
}

#[tokio::test]
async fn post_assigns_a_server_id_and_repeats_never_conflict() {
    let server = create_test_server().await;

    let first = server
        .post("/Organization")
        .add_header(X_TENANT_ID, "test-tenant")
        .json(&org("dup-test"))
        .await;
    first.assert_status(StatusCode::CREATED);
    let first_id = first.json::<Value>()["id"].as_str().unwrap().to_string();
    assert_ne!(
        first_id, "dup-test",
        "the client id is ignored, not honored"
    );

    // The exact POST that used to answer 409.
    let second = server
        .post("/Organization")
        .add_header(X_TENANT_ID, "test-tenant")
        .json(&org("dup-test"))
        .await;
    second.assert_status(StatusCode::CREATED);
    let second_id = second.json::<Value>()["id"].as_str().unwrap().to_string();
    assert_ne!(second_id, first_id, "every create gets its own id");

    // Nothing was ever stored under the client-supplied id.
    server
        .get("/Organization/dup-test")
        .add_header(X_TENANT_ID, "test-tenant")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn transaction_bundles_can_repeat_shared_post_entries() {
    let server = create_test_server().await;
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [{
            "fullUrl": "urn:uuid:00000000-0000-4000-8000-000000000001",
            "resource": org("shared-org"),
            "request": { "method": "POST", "url": "Organization" },
        }],
    });

    // The Synthea shape: consecutive patient bundles each POST the same
    // shared Organization under its fixed id. Both transactions must commit.
    for _ in 0..2 {
        let response = server
            .post("/")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&bundle)
            .await;
        response.assert_status(StatusCode::OK);
        let body = response.json::<Value>();
        let status = body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(status.starts_with("201"), "entry status: {status}");
        // #1223: the client id is ignored — the created location must not carry it.
        let location = body["entry"][0]["response"]["location"]
            .as_str()
            .unwrap_or_default();
        assert!(
            !location.contains("shared-org"),
            "transaction POST must get a server id, not the client id; location: {location}"
        );
    }

    // Nothing was ever stored under the client-supplied id.
    server
        .get("/Organization/shared-org")
        .add_header(X_TENANT_ID, "test-tenant")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

/// #1223: a POST entry inside a **batch** bundle must get a server-assigned id,
/// exactly as a standalone POST and a transaction entry do. The batch executor
/// reads the raw entry resource, so a regression here silently created the
/// resource under the client id — and a later import of the same id overwrote
/// it as v2 instead of creating a second copy.
#[tokio::test]
async fn batch_post_entries_get_a_server_id_not_the_client_id() {
    let server = create_test_server().await;
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "batch",
        "entry": [{
            "fullUrl": "urn:uuid:00000000-0000-4000-8000-0000000000aa",
            "resource": org("client-batch-id"),
            "request": { "method": "POST", "url": "Organization" },
        }],
    });

    let response = server
        .post("/")
        .add_header(X_TENANT_ID, "test-tenant")
        .json(&bundle)
        .await;
    response.assert_status(StatusCode::OK);
    let body = response.json::<Value>();
    let status = body["entry"][0]["response"]["status"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(status.starts_with("201"), "entry status: {status}");
    let location = body["entry"][0]["response"]["location"]
        .as_str()
        .unwrap_or_default();
    assert!(
        !location.contains("client-batch-id"),
        "batch POST must get a server id, not the client id; location: {location}"
    );

    // Nothing was ever stored under the client-supplied id.
    server
        .get("/Organization/client-batch-id")
        .add_header(X_TENANT_ID, "test-tenant")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}
