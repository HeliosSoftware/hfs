//! Bridge between the auth crate's `AuditEventSink` trait and the audit
//! crate's `AuditSink`.
//!
//! Translates narrow auth events (success/failure/denial) into full FHIR
//! `AuditEvent` resources and forwards them to the configured sink.

use std::sync::Arc;

use async_trait::async_trait;
use helios_auth::audit::AuditEventSink as AuthAuditEventSink;
use helios_auth::error::AuthError;
use helios_auth::principal::Principal;

use crate::builder::AuditEventBuilder;
use crate::config::AuditConfig;
use crate::exclusion::ExclusionFilter;
use crate::sink::AuditSink;

/// Bridges `helios_auth::AuditEventSink` to the audit subsystem.
pub struct AuditBridge {
    sink: Arc<dyn AuditSink>,
    source_observer: String,
    exclusion_filter: ExclusionFilter,
}

impl AuditBridge {
    /// Create a new bridge.
    pub fn new(sink: Arc<dyn AuditSink>, config: &AuditConfig) -> Self {
        Self {
            sink,
            source_observer: config.source_observer.clone(),
            exclusion_filter: ExclusionFilter::new(config.exclusions.clone()),
        }
    }
}

#[async_trait]
impl AuthAuditEventSink for AuditBridge {
    async fn record_auth_success(&self, principal: &Principal, path: &str, method: &str) {
        if self.exclusion_filter.is_excluded(path, method) {
            return;
        }
        let event = AuditEventBuilder::new(&self.source_observer)
            .action("E")
            .outcome("0")
            .agent(principal.subject(), None, true)
            .build();
        self.sink.record(event).await;
    }

    async fn record_auth_failure(&self, error: &AuthError, path: &str, method: &str) {
        if self.exclusion_filter.is_excluded(path, method) {
            return;
        }
        let event = AuditEventBuilder::new(&self.source_observer)
            .action("E")
            .outcome("8")
            .outcome_desc(error.to_string())
            .build();
        self.sink.record(event).await;
    }

    async fn record_authz_denial(
        &self,
        principal: &Principal,
        resource_type: &str,
        operation: &str,
    ) {
        let event = AuditEventBuilder::new(&self.source_observer)
            .action("E")
            .outcome("8")
            .outcome_desc(format!("Forbidden: {operation} on {resource_type}"))
            .agent(principal.subject(), None, true)
            .resource(resource_type, "")
            .build();
        self.sink.record(event).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helios_auth::scope::ScopeSet;
    use helios_fhir::r4::AuditEvent;
    use tokio::sync::Mutex;

    /// Test sink that collects recorded events.
    struct CollectorSink {
        events: Mutex<Vec<AuditEvent>>,
    }

    impl CollectorSink {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl AuditSink for CollectorSink {
        async fn record(&self, event: AuditEvent) {
            self.events.lock().await.push(event);
        }
        async fn flush(&self) {}
        fn name(&self) -> &str {
            "collector"
        }
    }

    fn test_principal() -> Principal {
        Principal {
            subject: "user-123".to_string(),
            issuer: "https://idp.example.com".to_string(),
            tenant_id: Some("tenant-a".to_string()),
            scopes: ScopeSet::empty(),
            jti: None,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            custom_claims: serde_json::Map::new(),
        }
    }

    #[tokio::test]
    async fn test_auth_success_records_event() {
        let sink = Arc::new(CollectorSink::new());
        let config = AuditConfig::default();
        let bridge = AuditBridge::new(Arc::clone(&sink) as Arc<dyn AuditSink>, &config);

        bridge
            .record_auth_success(&test_principal(), "/Patient", "GET")
            .await;

        let events = sink.events.lock().await;
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].outcome.as_ref().and_then(|o| o.value.as_deref()),
            Some("0")
        );
    }

    #[tokio::test]
    async fn test_auth_failure_records_event_with_outcome_8() {
        let sink = Arc::new(CollectorSink::new());
        let config = AuditConfig::default();
        let bridge = AuditBridge::new(Arc::clone(&sink) as Arc<dyn AuditSink>, &config);

        let error = AuthError::MissingToken;
        bridge.record_auth_failure(&error, "/Patient", "GET").await;

        let events = sink.events.lock().await;
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].outcome.as_ref().and_then(|o| o.value.as_deref()),
            Some("8")
        );
        assert!(events[0].outcome_desc.is_some());
    }

    #[tokio::test]
    async fn test_authz_denial_records_event() {
        let sink = Arc::new(CollectorSink::new());
        let config = AuditConfig::default();
        let bridge = AuditBridge::new(Arc::clone(&sink) as Arc<dyn AuditSink>, &config);

        bridge
            .record_authz_denial(&test_principal(), "Patient", "read")
            .await;

        let events = sink.events.lock().await;
        assert_eq!(events.len(), 1);
        assert!(
            events[0]
                .outcome_desc
                .as_ref()
                .and_then(|s| s.value.as_deref())
                .unwrap()
                .contains("Forbidden")
        );
    }

    #[tokio::test]
    async fn test_excluded_path_skips_auth_success() {
        let sink = Arc::new(CollectorSink::new());

        // Test with an explicit exclusion
        let config_with_exclusion = AuditConfig {
            exclusions: vec![crate::exclusion::ExclusionRule {
                path: "/metadata".to_string(),
                method: None,
            }],
            ..AuditConfig::default()
        };
        let bridge = AuditBridge::new(
            Arc::clone(&sink) as Arc<dyn AuditSink>,
            &config_with_exclusion,
        );

        bridge
            .record_auth_success(&test_principal(), "/metadata", "GET")
            .await;

        let events = sink.events.lock().await;
        assert_eq!(events.len(), 0);
    }
}
