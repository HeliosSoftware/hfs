//! The auth middleware turns a web-UI session cookie into the session's bearer
//! (issue #1449) — only when the request sent no `Authorization` of its own,
//! and never for a cross-site request — and then validates it exactly like any
//! bearer. No IdP runs here: the session is seeded, and a stub provider stands
//! in for JWKS validation.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::routing::get;
use axum::{Extension, Router};
use axum_test::TestServer;
use helios_audit::ExclusionFilter;
use helios_auth::{
    AuthConfig, AuthError, AuthProvider, LoginConfig, Principal, ScopeSet, Session,
    SessionPrincipal, SessionStore,
};
use helios_rest::AuthMiddlewareState;
use helios_rest::middleware::auth::auth_middleware;

const GOOD_TOKEN: &str = "access-token-1";

/// Accepts exactly `Bearer access-token-1`; everything else is invalid.
struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        if authorization_header == format!("Bearer {GOOD_TOKEN}") {
            Ok(Principal {
                subject: "demo-sub".to_string(),
                issuer: "https://idp.example.com".to_string(),
                tenant_id: None,
                scopes: ScopeSet::parse("system/*.cruds"),
                jti: None,
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
                custom_claims: serde_json::Map::new(),
            })
        } else {
            Err(AuthError::InvalidSignature)
        }
    }

    fn name(&self) -> &str {
        "stub"
    }
}

fn store_with_session() -> (Arc<SessionStore>, String) {
    let store = Arc::new(SessionStore::new(LoginConfig {
        client_id: "hfs-web".to_string(),
        client_secret: None,
        redirect_uri: "http://localhost:8080/ui/callback".to_string(),
        scopes: "openid".to_string(),
        authorization_endpoint: "https://idp.example.com/auth".to_string(),
        token_endpoint: "https://idp.example.com/token".to_string(),
        end_session_endpoint: None,
        cookie_secure: true,
    }));
    let session = Session {
        id: "sess-1".to_string(),
        principal: SessionPrincipal {
            subject: "demo-sub".to_string(),
            issuer: "https://idp.example.com".to_string(),
            name: None,
            preferred_username: None,
            email: None,
            picture: None,
        },
        access_token: GOOD_TOKEN.to_string(),
        access_expires_at: Instant::now() + Duration::from_secs(300),
        refresh_token: None,
        id_token: None,
        last_seen: Instant::now(),
        created_at: chrono::Utc::now(),
    };
    store.insert(session.clone());
    (store, session.id)
}

/// A one-route API behind the auth middleware; the handler reports who the
/// middleware authenticated.
fn server(sessions: Option<Arc<SessionStore>>) -> TestServer {
    let auth = Arc::new(AuthMiddlewareState {
        provider: Arc::new(StubProvider),
        config: Arc::new(AuthConfig::default()),
        audit_sink: Arc::new(helios_audit::sinks::NullSink),
        audit_source_observer: "Device/test".to_string(),
        audit_exclusion_filter: ExclusionFilter::new(vec![]),
        tenant_url_routing: false,
        sessions,
    });
    let app = Router::new()
        .route(
            "/Patient",
            get(|Extension(principal): Extension<Principal>| async move {
                principal.subject().to_string()
            }),
        )
        .layer(axum::middleware::from_fn_with_state(auth, auth_middleware));
    TestServer::new(app).expect("test server")
}

const COOKIE: HeaderName = HeaderName::from_static("cookie");
const SEC_FETCH_SITE: HeaderName = HeaderName::from_static("sec-fetch-site");

#[tokio::test]
async fn a_session_cookie_alone_authenticates_as_the_session_user() {
    let (store, id) = store_with_session();
    let server = server(Some(store));
    let response = server
        .get("/Patient")
        .add_header(
            COOKIE,
            HeaderValue::from_str(&format!("hfs_lang=es; hfs_session={id}")).unwrap(),
        )
        .await;
    response.assert_status(StatusCode::OK);
    assert_eq!(response.text(), "demo-sub");
}

#[tokio::test]
async fn no_cookie_and_no_bearer_is_still_unauthorized() {
    let (store, _) = store_with_session();
    let response = server(Some(store)).get("/Patient").await;
    response.assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_unknown_session_cookie_is_unauthorized() {
    let (store, _) = store_with_session();
    let response = server(Some(store))
        .get("/Patient")
        .add_header(COOKIE, HeaderValue::from_static("hfs_session=nope"))
        .await;
    response.assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_cross_site_request_never_rides_the_session_cookie() {
    let (store, id) = store_with_session();
    let response = server(Some(store))
        .get("/Patient")
        .add_header(
            COOKIE,
            HeaderValue::from_str(&format!("hfs_session={id}")).unwrap(),
        )
        .add_header(SEC_FETCH_SITE, HeaderValue::from_static("cross-site"))
        .await;
    response.assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_explicit_bearer_wins_over_the_session_cookie() {
    let (store, id) = store_with_session();
    let response = server(Some(store))
        .get("/Patient")
        .add_header(
            COOKIE,
            HeaderValue::from_str(&format!("hfs_session={id}")).unwrap(),
        )
        .add_header(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer something-else"),
        )
        .await;
    // The request's own (invalid) bearer is what gets validated — the session
    // is not consulted to rescue it.
    response.assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn without_a_session_store_the_cookie_is_ignored() {
    let (_, id) = store_with_session();
    let response = server(None)
        .get("/Patient")
        .add_header(
            COOKIE,
            HeaderValue::from_str(&format!("hfs_session={id}")).unwrap(),
        )
        .await;
    response.assert_status(StatusCode::UNAUTHORIZED);
}
