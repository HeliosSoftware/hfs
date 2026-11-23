//! A transaction `GET` entry reads exactly the `[type]/[id]` its scope check
//! judged: a deeper path is refused, never read on its last two segments.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderValue, StatusCode, header::CONTENT_TYPE};
use axum::middleware::Next;
use axum_test::TestServer;
use chrono::Utc;
use helios_auth::{Principal, ScopeSet};
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use serde_json::{Value, json};

const TENANT: &str = "test-tenant";

fn tenant() -> TenantContext {
    TenantContext::new(TenantId::new(TENANT), TenantPermissions::full_access())
}

/// A server whose every request carries a principal with `scopes`, over a
/// tenant holding `Patient/p1` and `Observation/o1`.
async fn server_for(scopes: &str) -> TestServer {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));
    let backend = Arc::new(
        SqliteBackend::with_config(
            ":memory:",
            SqliteBackendConfig {
                data_dir: Some(data_dir),
                ..Default::default()
            },
        )
        .expect("create SQLite backend"),
    );
    backend.init_schema().expect("init schema");
    for resource in [
        json!({"resourceType": "Patient", "id": "p1"}),
        json!({"resourceType": "Observation", "id": "o1", "status": "final",
               "code": {"text": "x"}, "subject": {"reference": "Patient/p1"}}),
    ] {
        let resource_type = resource["resourceType"].as_str().unwrap().to_string();
        backend
            .create(&tenant(), &resource_type, resource, FhirVersion::default())
            .await
            .expect("seed");
    }

    let config = ServerConfig {
        base_url: "http://localhost:8080".to_string(),
        default_tenant: TENANT.to_string(),
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(backend, config);
    let principal = Principal {
        subject: "narrow-client".to_string(),
        issuer: "https://issuer.example".to_string(),
        tenant_id: Some(TENANT.to_string()),
        scopes: ScopeSet::parse(scopes),
        jti: None,
        expires_at: Utc::now() + chrono::Duration::hours(1),
        custom_claims: serde_json::Map::new(),
        launch_context: None,
    };
    let app = helios_rest::routing::fhir_routes::create_routes(state).layer(
        axum::middleware::from_fn(move |mut request: axum::extract::Request, next: Next| {
            let principal = principal.clone();
            async move {
                request.extensions_mut().insert(principal);
                next.run(request).await
            }
        }),
    );
    TestServer::new(app).expect("test server")
}

async fn transaction(server: &TestServer, urls: &[&str]) -> axum_test::TestResponse {
    let entries: Vec<Value> = urls
        .iter()
        .map(|url| json!({ "request": { "method": "GET", "url": url } }))
        .collect();
    server
        .post("/")
        .add_header("x-tenant-id", HeaderValue::from_static(TENANT))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&json!({ "resourceType": "Bundle", "type": "transaction", "entry": entries }))
        .await
}

#[tokio::test]
async fn a_read_entry_of_a_granted_type_is_served() {
    let server = server_for("user/Patient.rs").await;
    let response = transaction(&server, &["Patient/p1"]).await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let body: Value = response.json();
    assert_eq!(body["entry"][0]["resource"]["resourceType"], "Patient");
    assert_eq!(body["entry"][0]["resource"]["id"], "p1");
}

#[tokio::test]
async fn a_read_entry_of_an_ungranted_type_is_refused() {
    let server = server_for("user/Patient.rs").await;
    let response = transaction(&server, &["Observation/o1"]).await;
    response.assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_deeper_path_is_not_read_on_its_last_two_segments() {
    let server = server_for("user/Patient.rs").await;
    for url in ["Patient/any/Observation/o1", "Patient/Observation/o1"] {
        let response = transaction(&server, &[url]).await;
        assert_eq!(
            response.status_code(),
            StatusCode::BAD_REQUEST,
            "{url}: {}",
            response.text()
        );
        let outcome: Value = response.json();
        assert_eq!(outcome["resourceType"], "OperationOutcome");
        assert!(
            !response.text().contains("\"resourceType\":\"Observation\""),
            "{url} must not surface the Observation"
        );
    }
}

#[tokio::test]
async fn a_deeper_path_is_refused_whatever_the_grant() {
    let server = server_for("user/*.rs").await;
    let response = transaction(&server, &["Patient/p1/foo"]).await;
    response.assert_status(StatusCode::BAD_REQUEST);
}
