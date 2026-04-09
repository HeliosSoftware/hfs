//! Database audit sink.
//!
//! Persists `AuditEvent` resources via the FHIR storage backend by serializing
//! the typed struct to `serde_json::Value` and calling `ResourceStorage::create()`.

use std::sync::Arc;

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use helios_fhir::r4::AuditEvent;

use crate::sink::AuditSink;

/// Trait for the subset of storage operations the database sink needs.
///
/// This avoids a direct dependency on `helios-persistence` — the HFS binary
/// provides a concrete implementation that delegates to `ResourceStorage`.
#[async_trait]
pub trait AuditStorage: Send + Sync + 'static {
    /// Persist a FHIR resource as JSON.
    async fn create_resource(
        &self,
        resource_type: &str,
        resource: serde_json::Value,
        fhir_version: FhirVersion,
    ) -> Result<(), String>;
}

/// Database-backed audit sink.
///
/// Serializes `AuditEvent` to JSON and delegates to an [`AuditStorage`]
/// implementation for persistence.
pub struct DatabaseSink {
    storage: Arc<dyn AuditStorage>,
    fhir_version: FhirVersion,
}

impl DatabaseSink {
    /// Create a new database sink.
    pub fn new(storage: Arc<dyn AuditStorage>, fhir_version: FhirVersion) -> Self {
        Self {
            storage,
            fhir_version,
        }
    }
}

#[async_trait]
impl AuditSink for DatabaseSink {
    async fn record(&self, event: AuditEvent) {
        let value = match serde_json::to_value(&event) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "Failed to serialize audit event to JSON Value");
                return;
            }
        };

        if let Err(e) = self
            .storage
            .create_resource("AuditEvent", value, self.fhir_version)
            .await
        {
            tracing::error!(error = %e, "Failed to persist audit event to database");
        }
    }

    async fn flush(&self) {
        // Database writes are immediate — nothing to flush.
    }

    fn name(&self) -> &str {
        "database"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::balp::AuditAction;
    use crate::builder::AuditEventBuilder;
    use tokio::sync::Mutex;

    /// In-memory mock storage for testing.
    struct MockStorage {
        events: Mutex<Vec<serde_json::Value>>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl AuditStorage for MockStorage {
        async fn create_resource(
            &self,
            _resource_type: &str,
            resource: serde_json::Value,
            _fhir_version: FhirVersion,
        ) -> Result<(), String> {
            self.events.lock().await.push(resource);
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_record_persists_event() {
        let mock = Arc::new(MockStorage::new());
        let sink = DatabaseSink::new(
            Arc::clone(&mock) as Arc<dyn AuditStorage>,
            FhirVersion::default(),
        );

        let event = AuditEventBuilder::new("Device/hfs")
            .action(AuditAction::Read)
            .outcome("0")
            .build();
        sink.record(event).await;

        let events = mock.events.lock().await;
        assert_eq!(events.len(), 1);
        // Verify it's a valid AuditEvent by checking for key fields
        assert!(events[0]["id"].is_string());
        assert!(events[0]["recorded"].is_string());
    }

    #[tokio::test]
    async fn test_record_includes_correct_fields() {
        let mock = Arc::new(MockStorage::new());
        let sink = DatabaseSink::new(
            Arc::clone(&mock) as Arc<dyn AuditStorage>,
            FhirVersion::default(),
        );

        let event = AuditEventBuilder::new("Device/hfs")
            .action(AuditAction::Create)
            .outcome("0")
            .resource("Patient", "123")
            .build();
        sink.record(event).await;

        let events = mock.events.lock().await;
        let e = &events[0];
        assert!(e["id"].is_string());
        assert!(e["recorded"].is_string());
        assert!(e["source"]["observer"]["reference"].is_string());
        assert!(e["agent"].is_array());
    }

    #[tokio::test]
    async fn test_storage_error_is_logged_not_propagated() {
        struct FailingStorage;

        #[async_trait]
        impl AuditStorage for FailingStorage {
            async fn create_resource(
                &self,
                _: &str,
                _: serde_json::Value,
                _: FhirVersion,
            ) -> Result<(), String> {
                Err("connection refused".to_string())
            }
        }

        let sink = DatabaseSink::new(Arc::new(FailingStorage), FhirVersion::default());
        let event = AuditEventBuilder::new("Device/hfs").build();
        // Should not panic
        sink.record(event).await;
    }

    #[test]
    fn test_name() {
        let storage = Arc::new(MockStorage::new());
        let sink = DatabaseSink::new(storage, FhirVersion::default());
        assert_eq!(sink.name(), "database");
    }
}
