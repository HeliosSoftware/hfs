use async_trait::async_trait;

use crate::error::AuthError;
use crate::principal::Principal;

/// Trait for recording authentication and authorization events.
///
/// Implementations can log to structured tracing, persist FHIR AuditEvent
/// resources, or forward to external systems. The default `NoopAuditEventSink`
/// discards all events.
#[async_trait]
pub trait AuditEventSink: Send + Sync + 'static {
    /// Called after a token is successfully validated.
    async fn record_auth_success(&self, principal: &Principal, path: &str, method: &str);

    /// Called when authentication fails (invalid/missing token).
    async fn record_auth_failure(&self, error: &AuthError, path: &str, method: &str);

    /// Called when an authenticated principal is denied access by policy.
    async fn record_authz_denial(
        &self,
        principal: &Principal,
        resource_type: &str,
        operation: &str,
    );
}

/// No-op audit sink that discards all events.
///
/// This is the default implementation used until a real audit
/// backend is configured.
pub struct NoopAuditEventSink;

#[async_trait]
impl AuditEventSink for NoopAuditEventSink {
    async fn record_auth_success(&self, _principal: &Principal, _path: &str, _method: &str) {}

    async fn record_auth_failure(&self, _error: &AuthError, _path: &str, _method: &str) {}

    async fn record_authz_denial(
        &self,
        _principal: &Principal,
        _resource_type: &str,
        _operation: &str,
    ) {
    }
}
