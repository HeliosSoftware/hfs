//! Regression tests for #1338: system-level search is refused explicitly.
//!
//! `GET [base]?params` and `POST [base]/_search` answer `501` +
//! OperationOutcome (`not-supported`) — they used to fall through the router as
//! a bare `405` and as "`_search` is not a resource type" — and the
//! CapabilityStatement no longer lists `search-system`. `POST [base]` stays the
//! batch/transaction endpoint, and type-level search is untouched.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::{TestResponse, TestServer};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
const FHIR_JSON: HeaderValue = HeaderValue::from_static("application/fhir+json");

const ROUTING_MODES: [TenantRoutingMode; 3] = [
    TenantRoutingMode::HeaderOnly,
    TenantRoutingMode::UrlPath,
    TenantRoutingMode::Both,
];

fn sqlite_backend() -> SqliteBackend {
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
    backend
}

fn config(routing_mode: TenantRoutingMode) -> ServerConfig {
    ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    }
}

fn create_test_server(routing_mode: TenantRoutingMode) -> (TestServer, Arc<SqliteBackend>) {
    let backend = Arc::new(sqlite_backend());
    let state = helios_rest::AppState::new(Arc::clone(&backend), config(routing_mode));
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    (TestServer::new(app).expect("create test server"), backend)
}

fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

/// The base paths a tenant can be addressed by under `mode`, each with the
/// header (if any) that names the tenant: the default tenant at the bare root,
/// a header tenant, and a URL-prefix tenant.
fn bases(mode: TenantRoutingMode) -> Vec<(&'static str, Option<&'static str>)> {
    match mode {
        TenantRoutingMode::HeaderOnly => vec![("", None), ("", Some("acme"))],
        TenantRoutingMode::UrlPath => vec![("", None), ("/acme", None)],
        TenantRoutingMode::Both => vec![("", None), ("", Some("acme")), ("/acme", None)],
    }
}

async fn get(server: &TestServer, path: &str, header_tenant: Option<&str>) -> TestResponse {
    let mut request = server.get(path);
    if let Some(tenant) = header_tenant {
        request = request.add_header(X_TENANT_ID, HeaderValue::from_str(tenant).unwrap());
    }
    request.await
}

async fn post_form(
    server: &TestServer,
    path: &str,
    header_tenant: Option<&str>,
    body: &'static str,
) -> TestResponse {
    let mut request = server
        .post(path)
        .bytes(body.into())
        .content_type("application/x-www-form-urlencoded");
    if let Some(tenant) = header_tenant {
        request = request.add_header(X_TENANT_ID, HeaderValue::from_str(tenant).unwrap());
    }
    request.await
}

async fn post_bundle(
    server: &TestServer,
    path: &str,
    header_tenant: Option<&str>,
    bundle: &Value,
) -> TestResponse {
    let mut request = server
        .post(path)
        .json(bundle)
        .add_header(CONTENT_TYPE, FHIR_JSON);
    if let Some(tenant) = header_tenant {
        request = request.add_header(X_TENANT_ID, HeaderValue::from_str(tenant).unwrap());
    }
    request.await
}

/// `501` + a single `error` / `not-supported` issue that points at type-level
/// search.
fn assert_system_search_refused(response: &TestResponse, what: &str) {
    assert_eq!(
        response.status_code(),
        StatusCode::NOT_IMPLEMENTED,
        "{what}: {}",
        response.text()
    );
    let body: Value = response.json();
    assert_eq!(body["resourceType"], "OperationOutcome", "{what}: {body}");
    assert_eq!(body["issue"][0]["severity"], "error", "{what}: {body}");
    assert_eq!(body["issue"][0]["code"], "not-supported", "{what}: {body}");
    let text = body["issue"][0]["details"]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(
        text.contains("system-level search") && text.contains("GET [base]/[type]?"),
        "{what}: the outcome should say what to do instead, got {body}"
    );
}

#[tokio::test]
async fn get_base_with_search_parameters_is_refused_in_every_routing_mode() {
    const QUERIES: [&str; 5] = [
        "",
        "?_type=Patient",
        "?_type=Patient,Observation&_id=1",
        "?_lastUpdated=gt2020",
        "?_format=json",
    ];
    for mode in ROUTING_MODES {
        let (server, _backend) = create_test_server(mode);
        for (base, header_tenant) in bases(mode) {
            for query in QUERIES {
                // `[base]` and `[base]/` are the same endpoint.
                for root in [format!("{base}/{query}"), format!("{base}{query}")] {
                    if root.is_empty() || root.starts_with('?') {
                        continue;
                    }
                    let response = get(&server, &root, header_tenant).await;
                    assert_system_search_refused(
                        &response,
                        &format!("{mode:?} GET {root} (header tenant {header_tenant:?})"),
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn search_at_base_underscore_search_is_refused_in_every_routing_mode() {
    for mode in ROUTING_MODES {
        let (server, _backend) = create_test_server(mode);
        for (base, header_tenant) in bases(mode) {
            let path = format!("{base}/_search");
            let what = format!("{mode:?} {path} (header tenant {header_tenant:?})");

            let response = post_form(&server, &path, header_tenant, "_type=Patient").await;
            assert_system_search_refused(&response, &format!("POST {what}"));

            let response = post_form(&server, &path, header_tenant, "").await;
            assert_system_search_refused(&response, &format!("POST (empty form) {what}"));

            let response = get(&server, &format!("{path}?_type=Patient"), header_tenant).await;
            assert_system_search_refused(&response, &format!("GET {what}"));
        }
    }
}

/// Under URL-path routing `_search` used to parse as a tenant id, so an
/// unprefixed `POST /_search` became `POST /` — the batch endpoint — for a
/// tenant named `_search`, and a Bundle sent there was committed into it.
#[tokio::test]
async fn bundle_posted_to_underscore_search_is_not_processed_as_a_batch() {
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [{
            "resource": {"resourceType": "Patient", "id": "leak", "name": [{"family": "Leak"}]},
            "request": {"method": "PUT", "url": "Patient/leak"}
        }]
    });
    for mode in ROUTING_MODES {
        let (server, backend) = create_test_server(mode);

        let response = post_bundle(&server, "/_search", None, &bundle).await;
        assert_system_search_refused(&response, &format!("{mode:?} POST /_search with a Bundle"));

        for tenant_id in ["_search", "test-tenant"] {
            let stored = backend
                .read(&tenant(tenant_id), "Patient", "leak")
                .await
                .expect("read");
            assert!(
                stored.is_none(),
                "{mode:?}: the Bundle was committed into tenant {tenant_id}"
            );
        }
    }
}

#[tokio::test]
async fn post_base_is_still_batch_and_transaction_in_every_routing_mode() {
    for mode in ROUTING_MODES {
        let (server, backend) = create_test_server(mode);
        for (index, (base, header_tenant)) in bases(mode).into_iter().enumerate() {
            let what = format!("{mode:?} POST {base}/ (header tenant {header_tenant:?})");
            let tenant_id = match (base, header_tenant) {
                ("", None) => "test-tenant",
                _ => "acme",
            };
            let id = format!("tx-{index}");

            let transaction = json!({
                "resourceType": "Bundle",
                "type": "transaction",
                "entry": [{
                    "resource": {"resourceType": "Patient", "id": id, "name": [{"family": "Kept"}]},
                    "request": {"method": "PUT", "url": format!("Patient/{id}")}
                }]
            });
            let response =
                post_bundle(&server, &format!("{base}/"), header_tenant, &transaction).await;
            assert_eq!(response.status_code(), StatusCode::OK, "{what}");
            let body: Value = response.json();
            assert_eq!(body["type"], "transaction-response", "{what}: {body}");
            assert!(
                backend
                    .read(&tenant(tenant_id), "Patient", &id)
                    .await
                    .expect("read")
                    .is_some(),
                "{what}: the transaction entry was not stored for tenant {tenant_id}"
            );

            // A batch whose one entry is a type-level search: the positive
            // control that search itself still runs, and finds the Patient.
            let batch = json!({
                "resourceType": "Bundle",
                "type": "batch",
                "entry": [{"request": {"method": "GET", "url": "Patient?family=Kept"}}]
            });
            let response = post_bundle(&server, &format!("{base}/"), header_tenant, &batch).await;
            assert_eq!(response.status_code(), StatusCode::OK, "{what}");
            let body: Value = response.json();
            assert_eq!(body["type"], "batch-response", "{what}: {body}");
            let found = &body["entry"][0]["resource"]["entry"];
            assert!(
                found
                    .as_array()
                    .is_some_and(|entries| entries.iter().any(|e| e["resource"]["id"] == id)),
                "{what}: batch search entry did not find Patient/{id}: {body}"
            );
        }
    }
}

/// `POST [base]` is the batch/transaction endpoint whatever the Content-Type;
/// the POST form of system-level search is `[base]/_search`. A form body there
/// is a malformed Bundle, as before.
#[tokio::test]
async fn form_post_to_base_is_a_malformed_bundle_not_a_search() {
    let (server, _backend) = create_test_server(TenantRoutingMode::HeaderOnly);
    let response = post_form(&server, "/", None, "_type=Patient").await;
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    let body: Value = response.json();
    assert_eq!(body["issue"][0]["code"], "invalid", "{body}");
}

#[tokio::test]
async fn type_level_search_is_unaffected() {
    for mode in ROUTING_MODES {
        let (server, backend) = create_test_server(mode);
        for (base, header_tenant) in bases(mode) {
            let what = format!("{mode:?} {base}/Patient (header tenant {header_tenant:?})");
            let tenant_id = match (base, header_tenant) {
                ("", None) => "test-tenant",
                _ => "acme",
            };
            // Idempotent across the two ways `Both` can address `acme`.
            backend
                .create_or_update(
                    &tenant(tenant_id),
                    "Patient",
                    "p1",
                    json!({"resourceType": "Patient", "id": "p1", "name": [{"family": "Findable"}]}),
                    helios_fhir::FhirVersion::default(),
                )
                .await
                .expect("seed patient");

            let response = get(
                &server,
                &format!("{base}/Patient?family=Findable"),
                header_tenant,
            )
            .await;
            assert_eq!(response.status_code(), StatusCode::OK, "{what}");
            let body: Value = response.json();
            assert_eq!(
                body["entry"][0]["resource"]["id"], "p1",
                "GET {what}: {body}"
            );

            let response = post_form(
                &server,
                &format!("{base}/Patient/_search"),
                header_tenant,
                "family=Findable",
            )
            .await;
            assert_eq!(response.status_code(), StatusCode::OK, "{what}");
            let body: Value = response.json();
            assert_eq!(
                body["entry"][0]["resource"]["id"], "p1",
                "POST {what}: {body}"
            );
        }
    }
}

#[tokio::test]
async fn capability_statement_does_not_advertise_search_system() {
    for mode in ROUTING_MODES {
        let (server, _backend) = create_test_server(mode);
        let response = server.get("/metadata").await;
        assert_eq!(response.status_code(), StatusCode::OK);
        let body: Value = response.json();
        let codes: Vec<&str> = body["rest"][0]["interaction"]
            .as_array()
            .expect("rest.interaction")
            .iter()
            .filter_map(|interaction| interaction["code"].as_str())
            .collect();
        assert!(!codes.contains(&"search-system"), "{mode:?}: {codes:?}");
        // What the server does serve at system level is still listed. SQLite
        // supports atomic transactions, so `transaction` is there too.
        for served in ["transaction", "batch", "history-system"] {
            assert!(
                codes.contains(&served),
                "{mode:?}: {served} missing from {codes:?}"
            );
        }
        // ... and `history-system` is truthful.
        let response = server.get("/_history").await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{mode:?} GET /_history"
        );

        // Type-level search is still advertised.
        let patient = body["rest"][0]["resource"]
            .as_array()
            .expect("rest.resource")
            .iter()
            .find(|resource| resource["type"] == "Patient")
            .expect("Patient entry");
        assert!(
            patient["interaction"]
                .as_array()
                .is_some_and(|list| list.iter().any(|i| i["code"] == "search-type")),
            "{mode:?}: Patient lost search-type"
        );
    }
}

#[tokio::test]
async fn other_system_routes_are_unaffected() {
    let (server, _backend) = create_test_server(TenantRoutingMode::HeaderOnly);
    for path in [
        "/health",
        "/_liveness",
        "/metadata",
        "/$versions",
        "/_history",
    ] {
        let response = server.get(path).await;
        assert_eq!(response.status_code(), StatusCode::OK, "GET {path}");
    }
}

/// The refusal sits behind authentication like every other FHIR route: an
/// unauthenticated client gets `401`, not a `501` that reveals routing.
mod with_auth {
    use super::*;
    use helios_audit::{ExclusionFilter, NullSink};
    use helios_auth::{AuthConfig, AuthError, AuthProvider, Principal};
    use helios_rest::middleware::auth::AuthMiddlewareState;

    /// Never reached: the requests below carry no `Authorization` header.
    struct RejectEverything;

    #[async_trait::async_trait]
    impl AuthProvider for RejectEverything {
        async fn authenticate(&self, _authorization_header: &str) -> Result<Principal, AuthError> {
            Err(AuthError::MissingToken)
        }

        fn name(&self) -> &str {
            "reject-everything"
        }
    }

    #[tokio::test]
    async fn unauthenticated_system_search_is_401_not_501() {
        let auth_config = AuthConfig::default();
        let auth_state = Arc::new(AuthMiddlewareState {
            provider: Arc::new(RejectEverything),
            config: Arc::new(auth_config.clone()),
            audit_sink: Arc::new(NullSink),
            audit_source_observer: "test".to_string(),
            audit_exclusion_filter: ExclusionFilter::new(Vec::new()),
            tenant_url_routing: false,
            sessions: None,
        });
        let app = helios_rest::create_app_with_auth(
            sqlite_backend(),
            config(TenantRoutingMode::HeaderOnly),
            auth_config,
            Some(auth_state),
            None,
        );
        let server = TestServer::new(app).expect("create test server");

        let response = server.get("/?_type=Patient").await;
        assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
        let response = post_form(&server, "/_search", None, "_type=Patient").await;
        assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
        // Same as the type-level search it is compared against.
        let response = server.get("/Patient?family=x").await;
        assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
    }
}
