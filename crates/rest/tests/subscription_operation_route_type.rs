//! The Subscription operations ($status, $events, $get-ws-binding-token) are
//! mounted under a wildcard `/{resource_type}/{id}/$op` route. They exist on
//! `Subscription` only: another type segment is an unknown path, whatever the
//! token is granted on that type, and the Subscription form is judged by a
//! read grant on Subscription.

use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, Principal, ScopeSet};
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_rest::{AuthMiddlewareState, ServerConfig};

/// `user/Observation.r`: may read Observations and nothing else.
const OBSERVATION_READ: &str = "observation-read-token";

struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        if authorization_header != format!("Bearer {OBSERVATION_READ}") {
            return Err(AuthError::InvalidSignature);
        }
        Ok(Principal {
            subject: "demo-sub".to_string(),
            issuer: "https://idp.example.com".to_string(),
            scopes: ScopeSet::parse("user/Observation.r"),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
            ..Default::default()
        })
    }

    fn name(&self) -> &str {
        "stub"
    }
}

const AUTHORIZATION: HeaderName = HeaderName::from_static("authorization");

fn bearer(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("Bearer {token}")).unwrap()
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

#[tokio::test]
async fn the_operations_exist_on_subscription_only() {
    let server = server();
    for op in ["$status", "$events", "$get-ws-binding-token"] {
        let other = server
            .get(&format!("/Observation/sub-1/{op}"))
            .add_header(AUTHORIZATION, bearer(OBSERVATION_READ))
            .await;
        assert_eq!(
            other.status_code(),
            StatusCode::NOT_FOUND,
            "{op} under another type: {}",
            other.text()
        );

        let subscription = server
            .get(&format!("/Subscription/sub-1/{op}"))
            .add_header(AUTHORIZATION, bearer(OBSERVATION_READ))
            .await;
        assert_eq!(
            subscription.status_code(),
            StatusCode::FORBIDDEN,
            "{op} needs a read grant on Subscription: {}",
            subscription.text()
        );
    }
}
