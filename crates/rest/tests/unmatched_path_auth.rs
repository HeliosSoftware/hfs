//! An unmatched path under an authenticated server answers `404`, never the
//! console-admin gate's `403` — the console tiers' middleware must wrap their
//! own routes only, not the app's fallback (found through #1571).

use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, Principal, ScopeSet};
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_rest::{AuthMiddlewareState, ServerConfig};

const USER_TOKEN: &str = "user-token";
const SYSTEM_TOKEN: &str = "system-token";

/// Accepts two bearers: a user-context one and a system-context one.
struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        let scopes = match authorization_header {
            h if h == format!("Bearer {USER_TOKEN}") => "user/*.cruds",
            h if h == format!("Bearer {SYSTEM_TOKEN}") => "system/*.cruds",
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

#[tokio::test]
async fn an_unmatched_path_is_a_404_for_a_user_token_not_the_admin_gate() {
    let server = server();
    let response = server
        .post("/ui/bulk-export/01bd4425-2b6b-440b-aba2-f6e7f088caeb/cancel")
        .add_header(AUTHORIZATION, bearer(USER_TOKEN))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::NOT_FOUND,
        "{}",
        response.text()
    );
    assert!(
        !response.text().contains("system-level scope"),
        "{}",
        response.text()
    );
}

#[tokio::test]
async fn an_unmatched_path_is_a_404_for_a_system_token_too() {
    let server = server();
    let response = server
        .get("/no/such/path/here")
        .add_header(AUTHORIZATION, bearer(SYSTEM_TOKEN))
        .await;
    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_admin_gate_still_guards_its_own_routes() {
    let server = server();
    let denied = server
        .get("/console/metrics/tenants")
        .add_header(AUTHORIZATION, bearer(USER_TOKEN))
        .await;
    assert_eq!(
        denied.status_code(),
        StatusCode::FORBIDDEN,
        "{}",
        denied.text()
    );
    assert!(
        denied.text().contains("system-level scope"),
        "{}",
        denied.text()
    );

    let unauthenticated = server.get("/console/metrics/tenants").await;
    assert_eq!(unauthenticated.status_code(), StatusCode::UNAUTHORIZED);

    let unauthenticated = server.get("/console/metrics/resource-counts").await;
    assert_eq!(unauthenticated.status_code(), StatusCode::UNAUTHORIZED);
}
