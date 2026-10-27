//! Core audit sink trait.
//!
//! All audit backends implement [`AuditSink`]. The trait is intentionally
//! infallible — failures are logged via `tracing` and never propagated to
//! callers, preserving the fire-and-forget contract.

use crate::fhir_model::AuditEvent;
use async_trait::async_trait;

/// Pluggable backend for recording FHIR `AuditEvent` resources.
///
/// Implementations must be `Send + Sync + 'static` so they can be shared
/// across Axum handlers via `Arc`.
#[async_trait]
pub trait AuditSink: Send + Sync + 'static {
    /// Record a single audit event.
    ///
    /// This method **must not** return errors to the caller. Failures are
    /// logged internally via `tracing::error!`.
    async fn record(&self, event: AuditEvent);

    /// Record a batch of audit events.
    ///
    /// The default implementation calls [`record`](Self::record) for each
    /// event sequentially. Backends that support batch writes can override
    /// this for better throughput.
    async fn record_batch(&self, events: Vec<AuditEvent>) {
        for event in events {
            self.record(event).await;
        }
    }

    /// Flush any buffered events to the underlying store.
    async fn flush(&self);

    /// Human-readable name for this sink (e.g. `"null"`, `"file"`, `"database"`).
    fn name(&self) -> &str;
}
