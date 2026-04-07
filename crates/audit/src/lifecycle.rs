//! Audit events for server lifecycle transitions.
//!
//! Emits `AuditEvent` records when the server starts up or shuts down,
//! capturing configuration details as entity metadata.

use helios_fhir::r4::AuditEvent;

use crate::balp::AuditAction;
use crate::builder::AuditEventBuilder;
use crate::sink::AuditSink;

/// Record a server startup audit event.
///
/// Captures the storage backend, FHIR version, and auth/audit status as
/// entity details so that configuration at startup is part of the audit trail.
pub async fn record_startup(
    sink: &dyn AuditSink,
    source_observer: &str,
    details: Vec<(&str, String)>,
) {
    let event = build_lifecycle_event(source_observer, "startup", "0", &details);
    sink.record(event).await;
}

/// Record a server shutdown audit event.
///
/// Should be called during graceful shutdown, before the audit sink is flushed
/// and dropped.
pub async fn record_shutdown(sink: &dyn AuditSink, source_observer: &str) {
    let event = build_lifecycle_event(source_observer, "shutdown", "0", &[]);
    sink.record(event).await;
}

fn build_lifecycle_event(
    source_observer: &str,
    phase: &str,
    outcome: &str,
    details: &[(&str, String)],
) -> AuditEvent {
    let operation = format!("lifecycle-{phase}");
    let mut builder = AuditEventBuilder::new(source_observer)
        .event_type(
            "http://terminology.hl7.org/CodeSystem/audit-event-type",
            "object",
        )
        .action(AuditAction::Execute)
        .outcome(outcome)
        .detail("audit-operation", operation)
        .detail("phase", phase);

    for (key, value) in details {
        builder = builder.detail(key, value);
    }

    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_startup_event_has_object_type() {
        let event = build_lifecycle_event("Device/hfs", "startup", "0", &[]);
        assert_eq!(
            event.r#type.code.as_ref().and_then(|c| c.value.as_deref()),
            Some("object")
        );
    }

    #[test]
    fn test_startup_event_has_phase_detail() {
        let event = build_lifecycle_event("Device/hfs", "startup", "0", &[]);
        let details = event.entity.as_ref().unwrap()[0].detail.as_ref().unwrap();
        assert_eq!(details[0].r#type.value.as_deref(), Some("audit-operation"));
        assert_eq!(
            details[0].value.as_ref().and_then(|v| match v {
                helios_fhir::r4::AuditEventEntityDetailValue::String(s) => s.value.as_deref(),
                _ => None,
            }),
            Some("lifecycle-startup")
        );
        assert_eq!(details[1].r#type.value.as_deref(), Some("phase"));
    }

    #[test]
    fn test_startup_event_carries_config_details() {
        let details = vec![
            ("storage-backend", "sqlite".to_string()),
            ("fhir-version", "R4".to_string()),
            ("auth-enabled", "true".to_string()),
        ];
        let event = build_lifecycle_event("Device/hfs", "startup", "0", &details);
        let entity_details = event.entity.as_ref().unwrap()[0].detail.as_ref().unwrap();
        // audit-operation + phase + 3 config details
        assert_eq!(entity_details.len(), 5);
        assert_eq!(
            entity_details[2].r#type.value.as_deref(),
            Some("storage-backend")
        );
        assert_eq!(
            entity_details[3].r#type.value.as_deref(),
            Some("fhir-version")
        );
        assert_eq!(
            entity_details[4].r#type.value.as_deref(),
            Some("auth-enabled")
        );
    }

    #[test]
    fn test_shutdown_event_has_phase() {
        let event = build_lifecycle_event("Device/hfs", "shutdown", "0", &[]);
        let details = event.entity.as_ref().unwrap()[0].detail.as_ref().unwrap();
        assert_eq!(details[0].r#type.value.as_deref(), Some("audit-operation"));
        assert_eq!(
            details[0].value.as_ref().and_then(|v| match v {
                helios_fhir::r4::AuditEventEntityDetailValue::String(s) => s.value.as_deref(),
                _ => None,
            }),
            Some("lifecycle-shutdown")
        );
        assert_eq!(details[1].r#type.value.as_deref(), Some("phase"));
    }

    #[tokio::test]
    async fn test_record_startup_completes() {
        let sink = crate::sinks::NullSink;
        record_startup(&sink, "Device/hfs", vec![("backend", "sqlite".to_string())]).await;
    }

    #[tokio::test]
    async fn test_record_shutdown_completes() {
        let sink = crate::sinks::NullSink;
        record_shutdown(&sink, "Device/hfs").await;
    }
}
