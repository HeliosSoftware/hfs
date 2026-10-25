//! Database audit sink.
//!
//! Persists `AuditEvent` resources via the FHIR storage backend by serializing
//! the typed struct to `serde_json::Value` and calling `ResourceStorage::create()`.
//!
//! A write that fails with a transient error (pool exhausted, connection
//! refused, server unavailable) is retried with exponential backoff before the
//! event is given up on (#2234). Every event given up on is counted in
//! `audit_events_dropped_total`.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::fhir_model::AuditEvent;
use async_trait::async_trait;
use helios_fhir::FhirVersion;

use crate::metrics;
use crate::sink::AuditSink;

/// Why an [`AuditStorage`] write failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditStorageError {
    /// Failure detail, for the log.
    pub message: String,
    /// Whether the same write may succeed if tried again: the store is
    /// saturated, unreachable or briefly unavailable. Only transient failures
    /// are retried.
    pub transient: bool,
}

impl AuditStorageError {
    /// A failure worth retrying.
    pub fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transient: true,
        }
    }

    /// A failure that retrying will not fix.
    pub fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transient: false,
        }
    }
}

impl fmt::Display for AuditStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AuditStorageError {}

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
    ) -> Result<(), AuditStorageError>;
}

/// How [`DatabaseSink`] retries transient write failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Retries after the first attempt. Zero disables retrying.
    pub max_retries: u32,
    /// Wait before the first retry; doubled for each later one.
    pub initial_backoff: Duration,
    /// Ceiling on the wait between retries.
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 5,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(2),
        }
    }
}

/// Database-backed audit sink.
///
/// Serializes `AuditEvent` to JSON and delegates to an [`AuditStorage`]
/// implementation for persistence.
pub struct DatabaseSink {
    storage: Arc<dyn AuditStorage>,
    fhir_version: FhirVersion,
    retry: RetryPolicy,
}

impl DatabaseSink {
    /// Create a new database sink with the default [`RetryPolicy`].
    pub fn new(storage: Arc<dyn AuditStorage>, fhir_version: FhirVersion) -> Self {
        Self {
            storage,
            fhir_version,
            retry: RetryPolicy::default(),
        }
    }

    /// Replace the retry policy.
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }
}

#[async_trait]
impl AuditSink for DatabaseSink {
    async fn record(&self, event: AuditEvent) {
        let value = match serde_json::to_value(&event) {
            Ok(v) => v,
            Err(e) => {
                metrics::record_dropped(self.name(), metrics::REASON_SERIALIZE, 1);
                tracing::error!(error = %e, "Failed to serialize audit event to JSON Value");
                return;
            }
        };

        let mut backoff = self.retry.initial_backoff;
        let mut retries = 0;
        loop {
            let err = match self
                .storage
                .create_resource("AuditEvent", value.clone(), self.fhir_version)
                .await
            {
                Ok(()) => return,
                Err(err) => err,
            };
            if !err.transient {
                metrics::record_dropped(self.name(), metrics::REASON_WRITE_FAILED, 1);
                tracing::error!(error = %err, "Failed to persist audit event to database");
                return;
            }
            if retries >= self.retry.max_retries {
                metrics::record_dropped(self.name(), metrics::REASON_RETRIES_EXHAUSTED, 1);
                tracing::error!(
                    error = %err,
                    retries,
                    "Failed to persist audit event to database; retries exhausted"
                );
                return;
            }
            retries += 1;
            metrics::record_retry(self.name());
            tracing::warn!(
                error = %err,
                retry = retries,
                backoff_ms = backoff.as_millis() as u64,
                "Transient failure persisting audit event; retrying"
            );
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(self.retry.max_backoff);
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
        ) -> Result<(), AuditStorageError> {
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
            ) -> Result<(), AuditStorageError> {
                Err(AuditStorageError::permanent("constraint violation"))
            }
        }

        let sink = DatabaseSink::new(Arc::new(FailingStorage), FhirVersion::default());
        let event = AuditEventBuilder::new("Device/hfs").build();
        // Should not panic
        sink.record(event).await;
    }

    /// Fails transiently `failures` times, then stores.
    struct FlakyStorage {
        failures: std::sync::atomic::AtomicU32,
        attempts: std::sync::atomic::AtomicU32,
        stored: std::sync::atomic::AtomicU32,
    }

    impl FlakyStorage {
        fn new(failures: u32) -> Self {
            Self {
                failures: failures.into(),
                attempts: 0.into(),
                stored: 0.into(),
            }
        }
    }

    #[async_trait]
    impl AuditStorage for FlakyStorage {
        async fn create_resource(
            &self,
            _: &str,
            _: serde_json::Value,
            _: FhirVersion,
        ) -> Result<(), AuditStorageError> {
            use std::sync::atomic::Ordering;
            self.attempts.fetch_add(1, Ordering::SeqCst);
            // Each test drives one write at a time, so load-then-sub is exact.
            if self.failures.load(Ordering::SeqCst) > 0 {
                self.failures.fetch_sub(1, Ordering::SeqCst);
                return Err(AuditStorageError::transient(
                    "postgres: connection pool exhausted",
                ));
            }
            self.stored.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn quick_retry(max_retries: u32) -> RetryPolicy {
        RetryPolicy {
            max_retries,
            initial_backoff: std::time::Duration::from_millis(1),
            max_backoff: std::time::Duration::from_millis(4),
        }
    }

    #[tokio::test]
    async fn test_transient_error_is_retried_until_stored() {
        use std::sync::atomic::Ordering;
        let storage = Arc::new(FlakyStorage::new(3));
        let sink = DatabaseSink::new(
            Arc::clone(&storage) as Arc<dyn AuditStorage>,
            FhirVersion::default(),
        )
        .with_retry(quick_retry(5));

        sink.record(AuditEventBuilder::new("Device/hfs").build())
            .await;

        assert_eq!(storage.attempts.load(Ordering::SeqCst), 4);
        assert_eq!(storage.stored.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retries_are_bounded() {
        use std::sync::atomic::Ordering;
        let storage = Arc::new(FlakyStorage::new(u32::MAX));
        let sink = DatabaseSink::new(
            Arc::clone(&storage) as Arc<dyn AuditStorage>,
            FhirVersion::default(),
        )
        .with_retry(quick_retry(2));

        sink.record(AuditEventBuilder::new("Device/hfs").build())
            .await;

        assert_eq!(storage.attempts.load(Ordering::SeqCst), 3);
        assert_eq!(storage.stored.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_name() {
        let storage = Arc::new(MockStorage::new());
        let sink = DatabaseSink::new(storage, FhirVersion::default());
        assert_eq!(sink.name(), "database");
    }
}
