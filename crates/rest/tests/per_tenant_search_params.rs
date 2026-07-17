//! Integration tests: the in-memory SearchParameter registry is per-tenant, so
//! a custom `SearchParameter` POSTed under one tenant changes that tenant's FHIR
//! search API (and CapabilityStatement) without affecting another tenant.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"))
}

async fn server() -> TestServer {
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir()),
            ..Default::default()
        },
    )
    .expect("create SQLite backend");
    backend.init_schema().expect("init schema");
    let config = ServerConfig {
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "default".to_string(),
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::new(backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    TestServer::new(app).expect("create test server")
}

/// A custom SearchParameter (`Patient.name.where(use='nickname')`), POSTed under
/// one tenant, resolves searches and appears in `/metadata` for that tenant
/// only.
#[tokio::test]
async fn custom_search_parameter_is_tenant_scoped() {
    let server = server().await;

    let sp = json!({
        "resourceType": "SearchParameter",
        "id": "patient-nickname",
        "url": "http://acme.health/fhir/SearchParameter/patient-nickname",
        "name": "nickname",
        "status": "active",
        "code": "nickname",
        "base": ["Patient"],
        "type": "string",
        "expression": "Patient.name.where(use = 'nickname').given"
    });

    // POST the custom parameter under acme1 only.
    let created = server
        .post("/SearchParameter")
        .add_header(X_TENANT_ID, HeaderValue::from_static("acme1"))
        .json(&sp)
        .await;
    assert_eq!(created.status_code(), StatusCode::CREATED);

    // A Patient with a nickname, created under each tenant (indexed per-tenant).
    let patient = json!({
        "resourceType": "Patient",
        "id": "p1",
        "name": [{ "use": "nickname", "given": ["Ace"] }, { "family": "Adams" }]
    });
    for t in ["acme1", "acme2"] {
        let resp = server
            .put("/Patient/p1")
            .add_header(X_TENANT_ID, HeaderValue::from_str(t).unwrap())
            .json(&patient)
            .await;
        assert!(
            resp.status_code() == StatusCode::CREATED || resp.status_code() == StatusCode::OK,
            "{t} patient create: {}",
            resp.status_code()
        );
    }

    // acme1 resolves the custom search and matches the patient.
    let hit = server
        .get("/Patient?nickname=Ace")
        .add_header(X_TENANT_ID, HeaderValue::from_static("acme1"))
        .await;
    assert_eq!(hit.status_code(), StatusCode::OK);
    let body: Value = hit.json();
    assert_eq!(
        body["entry"].as_array().map(|e| e.len()).unwrap_or(0),
        1,
        "acme1 should resolve the custom nickname search"
    );

    // acme2 has no such parameter — under strict handling it is rejected.
    let strict = server
        .get("/Patient?nickname=Ace")
        .add_header(X_TENANT_ID, HeaderValue::from_static("acme2"))
        .add_header(
            HeaderName::from_static("prefer"),
            HeaderValue::from_static("handling=strict"),
        )
        .await;
    assert_eq!(
        strict.status_code(),
        StatusCode::BAD_REQUEST,
        "acme2 must not know the acme1-only 'nickname' parameter"
    );

    // GET /SearchParameter is likewise isolated.
    let acme1_sp: Value = server
        .get("/SearchParameter?code=nickname")
        .add_header(X_TENANT_ID, HeaderValue::from_static("acme1"))
        .await
        .json();
    assert_eq!(
        acme1_sp["entry"].as_array().map(|e| e.len()).unwrap_or(0),
        1
    );
    let acme2_sp: Value = server
        .get("/SearchParameter?code=nickname")
        .add_header(X_TENANT_ID, HeaderValue::from_static("acme2"))
        .await
        .json();
    assert_eq!(
        acme2_sp["entry"].as_array().map(|e| e.len()).unwrap_or(0),
        0
    );
}

/// `/metadata` (CapabilityStatement) advertises the custom parameter for the
/// owning tenant only.
#[tokio::test]
async fn capability_statement_is_tenant_scoped() {
    let server = server().await;

    let sp = json!({
        "resourceType": "SearchParameter",
        "id": "patient-nickname",
        "url": "http://acme.health/fhir/SearchParameter/patient-nickname",
        "name": "nickname",
        "status": "active",
        "code": "nickname",
        "base": ["Patient"],
        "type": "string",
        "expression": "Patient.name.where(use = 'nickname').given"
    });
    server
        .post("/SearchParameter")
        .add_header(X_TENANT_ID, HeaderValue::from_static("acme1"))
        .json(&sp)
        .await;

    let advertises_nickname = |cap: &Value| -> bool {
        cap["rest"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|r| r["resource"].as_array().into_iter().flatten())
            .filter(|res| res["type"] == "Patient")
            .flat_map(|res| res["searchParam"].as_array().into_iter().flatten())
            .any(|p| p["name"] == "nickname")
    };

    let acme1: Value = server
        .get("/metadata")
        .add_header(X_TENANT_ID, HeaderValue::from_static("acme1"))
        .await
        .json();
    assert!(
        advertises_nickname(&acme1),
        "acme1 metadata should list nickname"
    );

    let acme2: Value = server
        .get("/metadata")
        .add_header(X_TENANT_ID, HeaderValue::from_static("acme2"))
        .await
        .json();
    assert!(
        !advertises_nickname(&acme2),
        "acme2 metadata must not list nickname"
    );
}
