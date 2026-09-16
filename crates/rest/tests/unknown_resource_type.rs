//! Regression tests for #989: a path whose type segment is not a resource type
//! answers `404` + OperationOutcome (`not-supported`) on every type-scoped
//! route, instead of being served as an empty search or a storage miss.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::{TestResponse, TestServer};
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
#[cfg(all(feature = "R4", feature = "R5"))]
const ACCEPT: HeaderName = HeaderName::from_static("accept");

async fn create_test_server(
    routing_mode: TenantRoutingMode,
    default_fhir_version: FhirVersion,
) -> (TestServer, Arc<SqliteBackend>) {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(|path| path.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("create SQLite backend");
    backend.init_schema().expect("initialize SQLite schema");
    let backend = Arc::new(backend);

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        default_fhir_version,
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    (TestServer::new(app).expect("create test server"), backend)
}

fn tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
}

async fn seed_patient(backend: &SqliteBackend, id: &str) {
    backend
        .create(
            &tenant(),
            "Patient",
            json!({ "resourceType": "Patient", "id": id, "name": [{ "family": "Smith" }] }),
            FhirVersion::default_enabled(),
        )
        .await
        .expect("seed Patient");
}

/// The contract every unknown-type response must meet (#989).
fn assert_unknown_type_outcome(response: &TestResponse, resource_type: &str) {
    response.assert_status(StatusCode::NOT_FOUND);
    let body: Value = response.json();
    assert_eq!(body["resourceType"], "OperationOutcome", "{body}");
    assert_eq!(body["issue"][0]["severity"], "error", "{body}");
    assert_eq!(body["issue"][0]["code"], "not-supported", "{body}");
    let details = body["issue"][0]["details"]["text"]
        .as_str()
        .expect("details.text");
    assert!(
        details.contains(&format!("'{resource_type}'")),
        "details must name the offending segment: {details}"
    );
}

#[tokio::test]
async fn type_level_search_of_an_unknown_type_is_404_not_an_empty_searchset() {
    let (server, backend) = create_test_server(
        TenantRoutingMode::HeaderOnly,
        FhirVersion::default_enabled(),
    )
    .await;
    seed_patient(&backend, "p1").await;

    // The issue's reproduction: a made-up name and a plausible typo of a type
    // that has data.
    for unknown in ["Nonsense123", "Patinet"] {
        let response = server
            .get(&format!("/{unknown}"))
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        assert_unknown_type_outcome(&response, unknown);
    }

    // Case matters: `patient` is not `Patient`.
    let response = server
        .get("/patient")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await;
    assert_unknown_type_outcome(&response, "patient");

    // The real type still searches.
    let response = server
        .get("/Patient")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await;
    response.assert_status_ok();
    let bundle: Value = response.json();
    assert_eq!(bundle["resourceType"], "Bundle");
    assert_eq!(bundle["type"], "searchset");
    assert_eq!(bundle["entry"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn every_type_scoped_route_refuses_an_unknown_type_the_same_way() {
    let (server, _) = create_test_server(
        TenantRoutingMode::HeaderOnly,
        FhirVersion::default_enabled(),
    )
    .await;
    let tenant_header = || (X_TENANT_ID, HeaderValue::from_static("test-tenant"));
    let fhir_json = || {
        (
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
    };
    let body = json!({ "resourceType": "Patinet", "id": "1" });

    let responses: Vec<(&str, TestResponse)> = vec![
        (
            "GET /Patinet/1",
            server
                .get("/Patinet/1")
                .add_header(tenant_header().0, tenant_header().1)
                .await,
        ),
        (
            "GET /Patinet/1/_history",
            server
                .get("/Patinet/1/_history")
                .add_header(tenant_header().0, tenant_header().1)
                .await,
        ),
        (
            "GET /Patinet/1/_history/1",
            server
                .get("/Patinet/1/_history/1")
                .add_header(tenant_header().0, tenant_header().1)
                .await,
        ),
        (
            "GET /Patinet/_history",
            server
                .get("/Patinet/_history")
                .add_header(tenant_header().0, tenant_header().1)
                .await,
        ),
        (
            "POST /Patinet/_search",
            server
                .post("/Patinet/_search")
                .add_header(tenant_header().0, tenant_header().1)
                .add_header(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/x-www-form-urlencoded"),
                )
                .text("name=smith")
                .await,
        ),
        (
            "POST /Patinet",
            server
                .post("/Patinet")
                .add_header(tenant_header().0, tenant_header().1)
                .add_header(fhir_json().0, fhir_json().1)
                .json(&body)
                .await,
        ),
        (
            "PUT /Patinet/1",
            server
                .put("/Patinet/1")
                .add_header(tenant_header().0, tenant_header().1)
                .add_header(fhir_json().0, fhir_json().1)
                .json(&body)
                .await,
        ),
        (
            "PATCH /Patinet/1",
            server
                .patch("/Patinet/1")
                .add_header(tenant_header().0, tenant_header().1)
                .add_header(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/json-patch+json"),
                )
                .json(&json!([{ "op": "add", "path": "/active", "value": true }]))
                .await,
        ),
        (
            "DELETE /Patinet/1",
            server
                .delete("/Patinet/1")
                .add_header(tenant_header().0, tenant_header().1)
                .await,
        ),
        (
            "DELETE /Patinet?name=smith",
            server
                .delete("/Patinet?name=smith")
                .add_header(tenant_header().0, tenant_header().1)
                .await,
        ),
        (
            "POST /Patinet/$validate",
            server
                .post("/Patinet/$validate")
                .add_header(tenant_header().0, tenant_header().1)
                .add_header(fhir_json().0, fhir_json().1)
                .json(&body)
                .await,
        ),
        (
            "GET /Patinet/1/Observation (compartment)",
            server
                .get("/Patinet/1/Observation")
                .add_header(tenant_header().0, tenant_header().1)
                .await,
        ),
    ];

    for (label, response) in &responses {
        assert_eq!(
            response.status_code(),
            StatusCode::NOT_FOUND,
            "{label}: {}",
            response.text()
        );
        assert_unknown_type_outcome(response, "Patinet");
    }

    // HEAD carries no body, but the status still says so.
    let response = server
        .method(axum::http::Method::HEAD, "/Patinet/1")
        .add_header(tenant_header().0, tenant_header().1)
        .await;
    response.assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn non_type_paths_are_not_judged_as_resource_types() {
    let (server, _) = create_test_server(
        TenantRoutingMode::HeaderOnly,
        FhirVersion::default_enabled(),
    )
    .await;

    for path in ["/metadata", "/health", "/$versions", "/_history"] {
        let response = server
            .get(path)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{path}: {}",
            response.text()
        );
    }

    // A path that matches no route at all is still the router's plain 404,
    // not an unknown-type outcome — the gate only judges type-scoped routes.
    let response = server
        .get("/Patient/1/_history/1/extra")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await;
    response.assert_status(StatusCode::NOT_FOUND);
    assert!(response.text().is_empty(), "{}", response.text());

    // A server-level namespace used without its required id segments has no
    // static route of its own, so it *is* judged as a type — and refused as
    // one, rather than answered with an empty searchset as before.
    let response = server
        .get("/export-status")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await;
    assert_unknown_type_outcome(&response, "export-status");
}

#[tokio::test]
async fn a_percent_encoded_real_type_is_still_admitted() {
    let (server, _) = create_test_server(
        TenantRoutingMode::HeaderOnly,
        FhirVersion::default_enabled(),
    )
    .await;
    let response = server
        .get("/Pat%69ent")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await;
    response.assert_status_ok();
}

#[tokio::test]
async fn url_path_tenant_routing_gates_the_stripped_path() {
    for mode in [TenantRoutingMode::UrlPath, TenantRoutingMode::Both] {
        let (server, backend) = create_test_server(mode, FhirVersion::default_enabled()).await;
        seed_patient(&backend, "p1").await;

        let response = server.get("/test-tenant/Patinet").await;
        assert_unknown_type_outcome(&response, "Patinet");

        let response = server.get("/test-tenant/Patinet/1").await;
        assert_unknown_type_outcome(&response, "Patinet");

        let response = server.get("/test-tenant/Patient").await;
        response.assert_status_ok();
        let bundle: Value = response.json();
        assert_eq!(
            bundle["entry"].as_array().map(Vec::len),
            Some(1),
            "{mode:?}"
        );
    }
}

#[tokio::test]
async fn bundle_search_entries_refuse_an_unknown_type_like_a_direct_request() {
    let (server, backend) = create_test_server(
        TenantRoutingMode::HeaderOnly,
        FhirVersion::default_enabled(),
    )
    .await;
    seed_patient(&backend, "p1").await;

    for bundle_type in ["batch", "transaction"] {
        let response = server
            .post("/")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&json!({
                "resourceType": "Bundle",
                "type": bundle_type,
                "entry": [
                    { "request": { "method": "GET", "url": "Patient?name=smith" } },
                    { "request": { "method": "GET", "url": "Patinet?name=smith" } }
                ]
            }))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();
        assert_eq!(
            body["entry"][0]["response"]["status"], "200 OK",
            "{bundle_type}: {body}"
        );
        assert_eq!(
            body["entry"][1]["response"]["status"], "404 Not Found",
            "{bundle_type}: {body}"
        );
        assert_eq!(
            body["entry"][1]["response"]["outcome"]["issue"][0]["code"], "not-supported",
            "{bundle_type}: {body}"
        );
    }
}

#[cfg(all(feature = "R4", any(feature = "R5", feature = "R6")))]
#[tokio::test]
async fn a_type_from_another_compiled_version_is_judged_by_the_effective_version() {
    let (server, _) = create_test_server(TenantRoutingMode::HeaderOnly, FhirVersion::R4).await;

    // `ActorDefinition` exists from R5 on. Under the R4 default it is unknown…
    let response = server
        .get("/ActorDefinition")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await;
    assert_unknown_type_outcome(&response, "ActorDefinition");
    let body: Value = response.json();
    assert!(
        body["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("FHIR R4"),
        "{body}"
    );

    // …and admitted when the client negotiates a version that has it.
    #[cfg(feature = "R5")]
    {
        let response = server
            .get("/ActorDefinition/1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                ACCEPT,
                HeaderValue::from_static("application/fhir+json; fhirVersion=5.0"),
            )
            .await;
        // Past the gate: the instance simply does not exist.
        response.assert_status(StatusCode::NOT_FOUND);
        let body: Value = response.json();
        assert_eq!(body["issue"][0]["code"], "not-found", "{body}");
    }
}
