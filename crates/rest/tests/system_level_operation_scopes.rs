//! System-level operations under SMART scopes: `GET /_history` needs an
//! all-resource read grant, and `$sql-run` / `$sql-export` need a Search grant
//! on every resource type the subject reads. A token whose grants do not
//! cover them is refused with 403, as it is for the equivalent plain read.

use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, Principal, ScopeSet};
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_rest::{AuthMiddlewareState, ServerConfig};
use serde_json::json;

/// `user/Observation.rs`
const OBSERVATION_TOKEN: &str = "observation-token";
/// `user/Appointment.rs`
const APPOINTMENT_TOKEN: &str = "appointment-token";
/// `user/*.rs`
const WILDCARD_TOKEN: &str = "wildcard-token";
/// `openid`: a valid token with no resource scope at all.
const NO_RESOURCE_SCOPE_TOKEN: &str = "no-resource-scope-token";
/// `patient/*.rs` with no launch context.
const PATIENT_TOKEN: &str = "patient-token";

struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        let token = authorization_header
            .strip_prefix("Bearer ")
            .ok_or(AuthError::InvalidSignature)?;
        let scopes = match token {
            OBSERVATION_TOKEN => "user/Observation.rs",
            APPOINTMENT_TOKEN => "user/Appointment.rs",
            WILDCARD_TOKEN => "user/*.rs",
            NO_RESOURCE_SCOPE_TOKEN => "openid",
            PATIENT_TOKEN => "patient/*.rs",
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
const PREFER: HeaderName = HeaderName::from_static("prefer");

fn bearer(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("Bearer {token}")).unwrap()
}

fn observation_view() -> serde_json::Value {
    json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Observation",
        "select": [{"column": [{"name": "id", "path": "id"}]}]
    })
}

async fn system_history(token: &str) -> StatusCode {
    server()
        .get("/_history")
        .add_header(AUTHORIZATION, bearer(token))
        .await
        .status_code()
}

async fn sql_run(token: &str) -> (StatusCode, String) {
    let response = server()
        .post("/$sql-run")
        .add_header(AUTHORIZATION, bearer(token))
        .json(&observation_view())
        .await;
    (response.status_code(), response.text())
}

async fn sql_export(token: &str) -> (StatusCode, String) {
    let response = server()
        .post("/$sql-export")
        .add_header(AUTHORIZATION, bearer(token))
        .add_header(PREFER, HeaderValue::from_static("respond-async"))
        .json(&observation_view())
        .await;
    (response.status_code(), response.text())
}

#[tokio::test]
async fn system_history_needs_an_all_resource_read_grant() {
    assert_eq!(system_history(WILDCARD_TOKEN).await, StatusCode::OK);
    assert_eq!(
        system_history(OBSERVATION_TOKEN).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        system_history(NO_RESOURCE_SCOPE_TOKEN).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(system_history(PATIENT_TOKEN).await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn sql_run_needs_a_search_grant_on_the_type_the_view_reads() {
    let (status, _) = sql_run(WILDCARD_TOKEN).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = sql_run(OBSERVATION_TOKEN).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = sql_run(APPOINTMENT_TOKEN).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("Observation"), "{body}");
    let (status, _) = sql_run(NO_RESOURCE_SCOPE_TOKEN).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = sql_run(PATIENT_TOKEN).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn sql_export_needs_a_search_grant_on_the_type_the_view_reads() {
    let (status, body) = sql_export(APPOINTMENT_TOKEN).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("Observation"), "{body}");
    let (status, _) = sql_export(NO_RESOURCE_SCOPE_TOKEN).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = sql_export(PATIENT_TOKEN).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = sql_export(OBSERVATION_TOKEN).await;
    assert_ne!(status, StatusCode::FORBIDDEN, "{body}");
}
