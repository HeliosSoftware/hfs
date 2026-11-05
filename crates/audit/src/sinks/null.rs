//! No-op audit sink for development and testing.

use crate::fhir_model::AuditEvent;
use async_trait::async_trait;

use crate::sink::AuditSink;

/// No-op sink that discards all audit events.
///
/// Used when audit logging is disabled (`HFS_AUDIT_BACKEND=none`).
/// Compiles to no-ops with zero runtime cost.
pub struct NullSink;

#[async_trait]
impl AuditSink for NullSink {
    async fn record(&self, _event: AuditEvent) {}

    async fn flush(&self) {}

    fn name(&self) -> &str {
        "null"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::AuditEventBuilder;

    #[tokio::test]
    async fn test_record_completes() {
        let sink = NullSink;
        let event = AuditEventBuilder::new("Device/hfs").build();
        sink.record(event).await;
    }

    #[tokio::test]
    async fn test_flush_completes() {
        let sink = NullSink;
        sink.flush().await;
    }

    #[test]
    fn test_name() {
        assert_eq!(NullSink.name(), "null");
    }
}
