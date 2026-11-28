//! The Subscription operations against a live engine and a registered
//! Subscription (#2818): a token that may read Observations only reaches none
//! of `$status`, `$events` and `$get-ws-binding-token` through
//! `/Observation/{id}/$op`, while a read grant on Subscription still does
//! through `/Subscription/{id}/$op`.
//!
//! `subscription_operation_route_type.rs` covers the same routes without an
//! engine. This file holds one test because it enables the engine through the
//! process environment.
#![cfg(all(feature = "subscriptions", feature = "R4"))]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, Principal, ScopeSet};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::{AuthMiddlewareState, ServerConfig};
use serde_json::{Value, json};

const TOPIC_URL: &str = "http://example.org/topics/encounter-start";
const OPERATIONS: [&str; 3] = ["$status", "$events", "$get-ws-binding-token"];

/// `system/*.cruds`: writes the topic and the Subscription.
const WRITER: &str = "writer-token";
/// `user/Observation.r`: may read Observations and nothing else.
const OBSERVATION_READ: &str = "observation-read-token";
/// `user/Subscription.r`: may read Subscriptions and nothing else.
const SUBSCRIPTION_READ: &str = "subscription-read-token";

struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        let scopes = match authorization_header.strip_prefix("Bearer ") {
            Some(WRITER) => "system/*.cruds",
            Some(OBSERVATION_READ) => "user/Observation.r",
            Some(SUBSCRIPTION_READ) => "user/Subscription.r",
            _ => return Err(AuthError::InvalidSignature),
        };
        Ok(Principal {
            subject: "demo-sub".to_string(),
            issuer: "https://idp.example.com".to_string(),
            scopes: ScopeSet::parse(scopes),
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
    // SAFETY: the only test in this binary, set before any thread is spawned
    // by the app, which reads it once while it is built.
    unsafe { std::env::set_var("HFS_SUBSCRIPTIONS_ENABLED", "true") };

    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"));
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir,
            ..Default::default()
        },
    )
    .expect("in-memory sqlite");
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

fn topic() -> Value {
    json!({
        "resourceType": "Basic", "id": "topic-1",
        "code": {"coding": [{"system": "http://hl7.org/fhir/fhir-types", "code": "SubscriptionTopic"}]},
        "extension": [{
            "url": "http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.url",
            "valueUri": TOPIC_URL
        }, {
            "url": "http://hl7.org/fhir/4.3/StructureDefinition/extension-SubscriptionTopic.resourceTrigger",
            "extension": [
                {"url": "resource", "valueUri": "http://hl7.org/fhir/StructureDefinition/Encounter"},
                {"url": "supportedInteraction", "valueCode": "create"}
            ]
        }]
    })
}

fn subscription() -> Value {
    json!({
        "resourceType": "Subscription", "id": "ws-sub", "status": "requested",
        "reason": "route type test", "criteria": TOPIC_URL,
        "channel": {"type": "websocket", "payload": "application/fhir+json"}
    })
}

#[tokio::test]
async fn a_grant_on_another_type_reaches_no_registered_subscription() {
    let server = server();
    for (path, body) in [
        ("/Basic/topic-1", topic()),
        ("/Subscription/ws-sub", subscription()),
    ] {
        let written = server
            .put(path)
            .add_header(AUTHORIZATION, bearer(WRITER))
            .json(&body)
            .await;
        assert_eq!(
            written.status_code(),
            StatusCode::CREATED,
            "{path}: {}",
            written.text()
        );
    }

    // The engine registers the Subscription after the write commits; a binding
    // token is only minted for a registered websocket Subscription.
    let mut registered = false;
    for _ in 0..100 {
        let token = server
            .get("/Subscription/ws-sub/$get-ws-binding-token")
            .add_header(AUTHORIZATION, bearer(SUBSCRIPTION_READ))
            .await;
        if token.status_code() == StatusCode::OK {
            registered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(registered, "the engine never registered the Subscription");

    for op in OPERATIONS {
        let other = server
            .get(&format!("/Observation/ws-sub/{op}"))
            .add_header(AUTHORIZATION, bearer(OBSERVATION_READ))
            .await;
        assert_eq!(
            other.status_code(),
            StatusCode::NOT_FOUND,
            "{op} under another type: {}",
            other.text()
        );

        let refused = server
            .get(&format!("/Subscription/ws-sub/{op}"))
            .add_header(AUTHORIZATION, bearer(OBSERVATION_READ))
            .await;
        assert_eq!(
            refused.status_code(),
            StatusCode::FORBIDDEN,
            "{op} needs a read grant on Subscription: {}",
            refused.text()
        );

        let granted = server
            .get(&format!("/Subscription/ws-sub/{op}"))
            .add_header(AUTHORIZATION, bearer(SUBSCRIPTION_READ))
            .await;
        assert_eq!(
            granted.status_code(),
            StatusCode::OK,
            "{op} with a read grant on Subscription: {}",
            granted.text()
        );
    }
}
