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
            _ => return Err(AuthError::InvalidSignature),
        };
        Ok(Principal {
            subject: "demo-sub".to_string(),
            issuer: "https://idp.example.com".to_string(),
            tenant_id: None,
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
    assert!(
        list.as_array()
            .map(|rows| rows.iter().all(|r| r["id"] != "acme"))
            .unwrap_or(true),
        "{list}"
    );
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
