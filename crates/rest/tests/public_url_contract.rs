//! Public URL contract for synchronous FHIR interactions.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum_test::TestServer;
use chrono::Utc;
use helios_auth::{Principal, ScopeSet};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use helios_rest::{AppState, ServerConfig};
use serde_json::{Value, json};

const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

async fn server(
    routing_mode: TenantRoutingMode,
    base_url: &str,
    principal_tenant: Option<&str>,
) -> (TestServer, Arc<SqliteBackend>) {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(|path| path.join("data"));
    let backend = Arc::new(
        SqliteBackend::with_config(
            ":memory:",
            SqliteBackendConfig {
                data_dir,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    backend.init_schema().unwrap();

    let state = AppState::new(
        Arc::clone(&backend),
        ServerConfig {
            base_url: base_url.to_string(),
            default_tenant: "default".to_string(),
            multitenancy: MultitenancyConfig {
                routing_mode,
                strict_validation: principal_tenant.is_some(),
                ..Default::default()
            },
            ..ServerConfig::for_testing()
        },
    );
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let app = if let Some(tenant_id) = principal_tenant {
        let principal = Principal {
            subject: "authenticated-client".to_string(),
            issuer: "https://issuer.example".to_string(),
            tenant_id: Some(tenant_id.to_string()),
            scopes: ScopeSet::parse("system/*.rs"),
            jti: None,
            expires_at: Utc::now() + chrono::Duration::hours(1),
            custom_claims: serde_json::Map::new(),
        };
        app.layer(axum::middleware::from_fn(
            move |mut request: axum::extract::Request, next: Next| {
                let principal = principal.clone();
                async move {
                    request.extensions_mut().insert(principal);
                    next.run(request).await
                }
            },
        ))
    } else {
        app
    };
    (TestServer::new(app).unwrap(), backend)
}

fn assert_public_url(value: &str, suffix: &str) {
    assert_eq!(value, format!("https://public.example/fhir/acme{suffix}"));
    assert!(!value.contains("spoofed.example"));
}

#[tokio::test]
async fn url_path_requests_advertise_prefix_and_tenant() {
    let (server, _) = server(
        TenantRoutingMode::UrlPath,
        "https://public.example/fhir/",
        None,
    )
    .await;

    let metadata = server
        .get("/acme/metadata")
        .add_header(
            HeaderName::from_static("host"),
            HeaderValue::from_static("spoofed.example"),
        )
        .add_header(
            HeaderName::from_static("forwarded"),
            HeaderValue::from_static("host=spoofed.example;proto=http"),
        )
        .add_header(
            HeaderName::from_static("x-forwarded-host"),
            HeaderValue::from_static("spoofed.example"),
        )
        .await;
    metadata.assert_status_ok();
    assert_public_url(
        metadata.json::<Value>()["implementation"]["url"]
            .as_str()
            .unwrap(),
        "",
    );

    let created = server
        .post("/acme/Patient")
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&json!({
            "resourceType": "Patient",
            "name": [{"family": "Public URL"}]
        }))
        .await;
    created.assert_status(StatusCode::CREATED);
    let location = created.headers()["location"].to_str().unwrap();
    assert!(location.starts_with("https://public.example/fhir/acme/Patient/"));
    let patient: Value = created.json();
    let id = patient["id"].as_str().unwrap();

    let updated = server
        .put("/acme/Observation/public-url-observation")
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&json!({
            "resourceType": "Observation",
            "id": "public-url-observation",
            "status": "final",
            "code": {"text": "URL contract"},
            "subject": {"reference": format!("Patient/{id}")}
        }))
        .await;
    updated.assert_status(StatusCode::CREATED);
    assert_public_url(
        updated.headers()["location"].to_str().unwrap(),
        "/Observation/public-url-observation",
    );

    let search = server.get("/acme/Patient?_count=1").await;
    search.assert_status_ok();
    let search: Value = search.json();
    assert_public_url(
        search["link"][0]["url"].as_str().unwrap(),
        "/Patient?_count=1",
    );
    assert_public_url(
        search["entry"][0]["fullUrl"].as_str().unwrap(),
        &format!("/Patient/{id}"),
    );

    let compartment = server
        .get(&format!("/acme/Patient/{id}/Observation?_count=1"))
        .await;
    compartment.assert_status_ok();
    let compartment: Value = compartment.json();
    assert_public_url(
        compartment["link"][0]["url"].as_str().unwrap(),
        &format!("/Patient/{id}/Observation?_count=1"),
    );
    assert_public_url(
        compartment["entry"][0]["fullUrl"].as_str().unwrap(),
        "/Observation/public-url-observation",
    );

    let history = server.get(&format!("/acme/Patient/{id}/_history")).await;
    history.assert_status_ok();
    assert_public_url(
        history.json::<Value>()["entry"][0]["fullUrl"]
            .as_str()
            .unwrap(),
        &format!("/Patient/{id}"),
    );

    let batch = server
        .post("/acme/")
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "resource": {"resourceType": "Patient"},
                "request": {"method": "POST", "url": "Patient"}
            }]
        }))
        .await;
    batch.assert_status_ok();
    assert!(
        batch.json::<Value>()["entry"][0]["fullUrl"]
            .as_str()
            .unwrap()
            .starts_with("https://public.example/fhir/acme/Patient/")
    );

    let transaction = server
        .post("/acme/")
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [{
                "resource": {"resourceType": "Patient"},
                "request": {"method": "POST", "url": "Patient"}
            }]
        }))
        .await;
    transaction.assert_status_ok();
    assert!(
        transaction.json::<Value>()["entry"][0]["fullUrl"]
            .as_str()
            .unwrap()
            .starts_with("https://public.example/fhir/acme/Patient/")
    );
}

#[tokio::test]
async fn header_tenant_does_not_change_the_public_path() {
    let (server, _) = server(
        TenantRoutingMode::HeaderOnly,
        "https://public.example/fhir/",
        None,
    )
    .await;

    let response = server
        .get("/metadata")
        .add_header(X_TENANT_ID, HeaderValue::from_static("acme"))
        .await;
    response.assert_status_ok();
    assert_eq!(
        response.json::<Value>()["implementation"]["url"],
        "https://public.example/fhir"
    );
}

#[tokio::test]
async fn authenticated_url_routes_preserve_tenant_in_public_base() {
    for routing_mode in [TenantRoutingMode::UrlPath, TenantRoutingMode::Both] {
        let (server, _) = server(routing_mode, "https://public.example/fhir/", Some("acme")).await;

        let response = server.get("/acme/metadata").await;
        response.assert_status_ok();
        assert_public_url(
            response.json::<Value>()["implementation"]["url"]
                .as_str()
                .unwrap(),
            "",
        );
    }
}

#[tokio::test]
async fn authenticated_header_routes_keep_unprefixed_public_base() {
    for routing_mode in [TenantRoutingMode::HeaderOnly, TenantRoutingMode::Both] {
        let (server, _) = server(routing_mode, "https://public.example/fhir/", Some("acme")).await;

        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, HeaderValue::from_static("acme"))
            .await;
        response.assert_status_ok();
        assert_eq!(
            response.json::<Value>()["implementation"]["url"],
            "https://public.example/fhir"
        );
    }
}
