//! The audit middleware tags each `AuditEvent` with the tenant the request
//! acted on (#1515).
//!
//! The database sink stores every event in the reserved system tenant, so the
//! tag is the only record of which tenant an event describes. The tenant is
//! resolved inside the handler, after the middleware has handed the request on,
//! so this drives the real middleware + router + extractor chain rather than
//! the builder alone.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::StatusCode;
use axum_test::TestServer;
use helios_audit::balp::code_systems::HFS_TENANT;
use helios_audit::{AuditConfig, AuditMiddlewareState, AuditSink, ExclusionFilter};
use helios_fhir::r4::AuditEvent;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_rest::ServerConfig;
use serde_json::json;

/// Retains every recorded [`AuditEvent`].
#[derive(Clone, Default)]
struct CollectorSink {
    events: Arc<Mutex<Vec<AuditEvent>>>,
}

#[async_trait]
impl AuditSink for CollectorSink {
    async fn record(&self, event: AuditEvent) {
        self.events.lock().expect("collector poisoned").push(event);
    }
    async fn flush(&self) {}
    fn name(&self) -> &str {
        "collector"
    }
}

fn tenant_tags(event: &AuditEvent) -> Vec<String> {
    event
        .meta
        .as_ref()
        .and_then(|m| m.tag.as_ref())
        .into_iter()
        .flatten()
        .filter(|c| c.system.as_ref().and_then(|s| s.value.as_deref()) == Some(HFS_TENANT))
        .filter_map(|c| c.code.as_ref().and_then(|c| c.value.clone()))
        .collect()
}

fn server() -> (TestServer, CollectorSink) {
    let backend = SqliteBackend::in_memory().expect("in-memory SQLite");
    backend.init_schema().expect("init schema");
    let sink = CollectorSink::default();
    let state = helios_rest::AppState::new(Arc::new(backend), ServerConfig::for_testing());
    let audit = Arc::new(AuditMiddlewareState {
        sink: Arc::new(sink.clone()),
        config: AuditConfig::default(),
        exclusion_filter: ExclusionFilter::new(Vec::new()),
    });
    let app = helios_rest::routing::fhir_routes::create_routes(state).layer(
        axum::middleware::from_fn_with_state(audit, helios_audit::middleware::audit_middleware),
    );
    (TestServer::new(app).expect("create test server"), sink)
}

#[tokio::test]
async fn event_is_tagged_with_the_header_tenant() {
    let (server, sink) = server();
    let response = server
        .post("/Patient")
        .add_header("X-Tenant-ID", "acme")
        .json(&json!({"resourceType": "Patient", "active": true}))
        .await;
    assert_eq!(response.status_code(), StatusCode::CREATED);

    let events = sink.events.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(tenant_tags(&events[0]), vec!["acme".to_string()]);
}

#[tokio::test]
async fn event_is_tagged_with_the_default_tenant() {
    let (server, sink) = server();
    server.get("/Patient").await.assert_status_ok();

    let events = sink.events.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(tenant_tags(&events[0]), vec!["test-tenant".to_string()]);
}

#[tokio::test]
async fn request_naming_a_reserved_tenant_is_left_untagged() {
    let (server, sink) = server();
    let response = server
        .get("/Patient")
        .add_header("X-Tenant-ID", "__system__")
        .await;
    assert_eq!(response.status_code(), StatusCode::FORBIDDEN);

    let events = sink.events.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert!(tenant_tags(&events[0]).is_empty());
}
