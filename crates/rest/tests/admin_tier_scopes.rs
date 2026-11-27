//! The cross-tenant admin tier (`/admin/tenants`, `/console/metrics/tenants`)
//! demands the `system/*` permission bit the request's method implies: a
//! read-only `system/*.rs` can look, but registering a tenant takes `c` and
//! deregistering or purging one takes `d`.

use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, Principal, ScopeSet};
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_rest::{AuthMiddlewareState, ServerConfig};
use serde_json::json;

const READ_ONLY: &str = "read-only-token";
const FULL: &str = "full-token";
const DELETE_ONLY: &str = "delete-only-token";
const USER_WILDCARD: &str = "user-token";
const V1_READ: &str = "v1-read-token";
const V1_WRITE: &str = "v1-write-token";
const TENANT_BOUND_FULL: &str = "tenant-bound-full-token";
const BLANK_CLAIM_FULL: &str = "blank-claim-full-token";

/// One bearer per scope shape under test.
struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        let scopes = match authorization_header {
            h if h == format!("Bearer {READ_ONLY}") => "system/*.rs",
            h if h == format!("Bearer {FULL}") => "system/*.cruds",
            h if h == format!("Bearer {DELETE_ONLY}") => "system/*.d",
            h if h == format!("Bearer {USER_WILDCARD}") => "user/*.cruds",
            h if h == format!("Bearer {V1_READ}") => "system/*.read",
            h if h == format!("Bearer {V1_WRITE}") => "system/*.write",
            h if h == format!("Bearer {TENANT_BOUND_FULL}") => "system/*.cruds",
            h if h == format!("Bearer {BLANK_CLAIM_FULL}") => "system/*.cruds",
            _ => return Err(AuthError::InvalidSignature),
        };
        let tenant_id = match authorization_header {
            h if h == format!("Bearer {TENANT_BOUND_FULL}") => Some("tenanta".to_string()),
            h if h == format!("Bearer {BLANK_CLAIM_FULL}") => Some(" ".to_string()),
            _ => None,
        };
        Ok(Principal {
            subject: "demo-sub".to_string(),
            issuer: "https://idp.example.com".to_string(),
            tenant_id,
            scopes: ScopeSet::parse(scopes),
            jti: None,
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
            custom_claims: serde_json::Map::new(),
            ..Default::default()
        })
    }

    fn name(&self) -> &str {
        "stub"
    }
}

fn server() -> TestServer {
    let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
    backend.init_schema().expect("init schema");
    let auth = Arc::new(AuthMiddlewareState {
        provider: Arc::new(StubProvider),
        config: Arc::new(AuthConfig::default()),
        audit_sink: Arc::new(helios_audit::sinks::NullSink),
        audit_source_observer: "Device/test".to_string(),
        audit_exclusion_filter: ExclusionFilter::new(vec![]),
        tenant_url_routing: false,
        sessions: None,
    });
    let app = helios_rest::create_app_with_auth(
        backend,
        ServerConfig::for_testing(),
        AuthConfig::default(),
        Some(auth),
        None,
    );
    TestServer::new(app).expect("test server")
}

const AUTHORIZATION: HeaderName = HeaderName::from_static("authorization");

fn bearer(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("Bearer {token}")).unwrap()
}

fn assert_forbidden_as_tenant_bound(response: &axum_test::TestResponse) {
    assert_eq!(
        response.status_code(),
        StatusCode::FORBIDDEN,
        "{}",
        response.text()
    );
    assert!(
        response.text().contains("bound to a tenant"),
        "expected the refusal to name the tenant binding: {}",
        response.text()
    );
}

fn assert_forbidden_for_bit(response: &axum_test::TestResponse, letter: &str) {
    assert_eq!(
        response.status_code(),
        StatusCode::FORBIDDEN,
        "{}",
        response.text()
    );
    let expected = format!("system/*.{letter}");
    assert!(
        response.text().contains(&expected),
        "expected the refusal to name {expected}: {}",
        response.text()
    );
}

#[tokio::test]
async fn a_read_only_system_wildcard_can_look_but_not_register_or_deregister() {
    let server = server();

    let list = server
        .get("/admin/tenants")
        .add_header(AUTHORIZATION, bearer(READ_ONLY))
        .await;
    assert_eq!(list.status_code(), StatusCode::OK, "{}", list.text());

    let metrics = server
        .get("/console/metrics/tenants")
        .add_header(AUTHORIZATION, bearer(READ_ONLY))
        .await;
    assert_ne!(
        metrics.status_code(),
        StatusCode::FORBIDDEN,
        "{}",
        metrics.text()
    );

    let create = server
        .post("/admin/tenants")
        .add_header(AUTHORIZATION, bearer(READ_ONLY))
        .json(&json!({ "id": "acme" }))
        .await;
    assert_forbidden_for_bit(&create, "c");

    let delete = server
        .delete("/admin/tenants/acme?purge=true")
        .add_header(AUTHORIZATION, bearer(READ_ONLY))
        .await;
    assert_forbidden_for_bit(&delete, "d");

    // Nothing was registered behind the refusals.
    let list = server
        .get("/admin/tenants")
        .add_header(AUTHORIZATION, bearer(FULL))
        .await
        .json::<serde_json::Value>();
    let rows = list["tenants"].as_array().expect("tenants array");
    assert!(rows.iter().all(|r| r["id"] != "acme"), "{list}");
}

#[tokio::test]
async fn the_implied_bit_admits_each_method() {
    let server = server();

    let create = server
        .post("/admin/tenants")
        .add_header(AUTHORIZATION, bearer(FULL))
        .json(&json!({ "id": "acme" }))
        .await;
    assert_eq!(
        create.status_code(),
        StatusCode::CREATED,
        "{}",
        create.text()
    );

    // `d` alone deregisters but cannot look.
    let list = server
        .get("/admin/tenants")
        .add_header(AUTHORIZATION, bearer(DELETE_ONLY))
        .await;
    assert_forbidden_for_bit(&list, "r");

    let delete = server
        .delete("/admin/tenants/acme")
        .add_header(AUTHORIZATION, bearer(DELETE_ONLY))
        .await;
    assert_eq!(delete.status_code(), StatusCode::OK, "{}", delete.text());
}

#[tokio::test]
async fn a_user_context_wildcard_is_refused_on_every_method() {
    let server = server();
    for (method, response) in [
        (
            "GET",
            server
                .get("/admin/tenants")
                .add_header(AUTHORIZATION, bearer(USER_WILDCARD))
                .await,
        ),
        (
            "POST",
            server
                .post("/admin/tenants")
                .add_header(AUTHORIZATION, bearer(USER_WILDCARD))
                .json(&json!({ "id": "acme" }))
                .await,
        ),
        (
            "DELETE",
            server
                .delete("/admin/tenants/acme")
                .add_header(AUTHORIZATION, bearer(USER_WILDCARD))
                .await,
        ),
    ] {
        assert_eq!(
            response.status_code(),
            StatusCode::FORBIDDEN,
            "{method}: {}",
            response.text()
        );
    }
}

#[tokio::test]
async fn head_is_held_to_the_read_bit() {
    let server = server();
    let response = server
        .method(axum::http::Method::HEAD, "/admin/tenants")
        .add_header(AUTHORIZATION, bearer(READ_ONLY))
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
}

#[tokio::test]
async fn an_unmounted_method_is_held_to_the_update_bit_before_routing() {
    let server = server();
    // Authorization runs before routing: a read-only token is refused on
    // the `u` bit rather than told the method is not allowed.
    let refused = server
        .method(axum::http::Method::PUT, "/admin/tenants")
        .add_header(AUTHORIZATION, bearer(READ_ONLY))
        .await;
    assert_forbidden_for_bit(&refused, "u");

    let full = server
        .method(axum::http::Method::PUT, "/admin/tenants")
        .add_header(AUTHORIZATION, bearer(FULL))
        .await;
    assert_eq!(full.status_code(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn smart_v1_scopes_grant_nothing_on_the_admin_tier() {
    // HFS parses SMART v2 scopes only, so `.read` / `.write` are not
    // permission bits and the admin tier admits neither.
    let server = server();
    for token in [V1_READ, V1_WRITE] {
        let list = server
            .get("/admin/tenants")
            .add_header(AUTHORIZATION, bearer(token))
            .await;
        assert_forbidden_for_bit(&list, "r");
        let create = server
            .post("/admin/tenants")
            .add_header(AUTHORIZATION, bearer(token))
            .json(&json!({ "id": "acme" }))
            .await;
        assert_forbidden_for_bit(&create, "c");
    }
}

#[tokio::test]
async fn a_tenant_bound_token_cannot_administer_other_tenants() {
    let server = server();
    let setup = server
        .post("/admin/tenants")
        .add_header(AUTHORIZATION, bearer(FULL))
        .json(&json!({ "id": "tenantb" }))
        .await;
    assert_eq!(setup.status_code(), StatusCode::CREATED, "{}", setup.text());

    let list = server
        .get("/admin/tenants")
        .add_header(AUTHORIZATION, bearer(TENANT_BOUND_FULL))
        .await;
    assert_forbidden_as_tenant_bound(&list);

    for path in ["/console/metrics/tenants", "/console/metrics/traffic"] {
        let metrics = server
            .get(path)
            .add_header(AUTHORIZATION, bearer(TENANT_BOUND_FULL))
            .await;
        assert_forbidden_as_tenant_bound(&metrics);
    }

    let create = server
        .post("/admin/tenants")
        .add_header(AUTHORIZATION, bearer(TENANT_BOUND_FULL))
        .json(&json!({ "id": "tenantc" }))
        .await;
    assert_forbidden_as_tenant_bound(&create);

    let purge = server
        .delete("/admin/tenants/tenantb?purge=true")
        .add_header(AUTHORIZATION, bearer(TENANT_BOUND_FULL))
        .await;
    assert_forbidden_as_tenant_bound(&purge);

    let list = server
        .get("/admin/tenants")
        .add_header(AUTHORIZATION, bearer(FULL))
        .await
        .json::<serde_json::Value>();
    let rows = list["tenants"].as_array().expect("tenants array");
    assert!(rows.iter().any(|r| r["id"] == "tenantb"), "{list}");
    assert!(rows.iter().all(|r| r["id"] != "tenantc"), "{list}");
}

#[tokio::test]
async fn a_blank_tenant_claim_is_not_read_as_a_system_token() {
    // The FHIR routes refuse a blank claim as invalid, so the admin tier
    // must not treat the same token as claimless.
    let server = server();
    let fhir = server
        .get("/Patient")
        .add_header(AUTHORIZATION, bearer(BLANK_CLAIM_FULL))
        .await;
    assert_eq!(fhir.status_code(), StatusCode::FORBIDDEN, "{}", fhir.text());

    let list = server
        .get("/admin/tenants")
        .add_header(AUTHORIZATION, bearer(BLANK_CLAIM_FULL))
        .await;
    assert_forbidden_as_tenant_bound(&list);

    let create = server
        .post("/admin/tenants")
        .add_header(AUTHORIZATION, bearer(BLANK_CLAIM_FULL))
        .json(&json!({ "id": "tenantc" }))
        .await;
    assert_forbidden_as_tenant_bound(&create);
}
