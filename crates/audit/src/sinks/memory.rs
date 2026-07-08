//! In-memory audit sink for tests.
//!
//! Buffers every recorded [`AuditEvent`] in an unbounded `Vec` behind a `Mutex`
//! so test code can assert on emitted events. Because the buffer never evicts,
//! this sink is test-only and is deliberately not reachable via
//! `HFS_AUDIT_BACKEND`; do not wire it into a running server.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::fhir_model::AuditEvent;
use crate::sink::AuditSink;

/// Test-only sink that retains every recorded [`AuditEvent`] in memory.
///
/// Clones share the same underlying buffer (events recorded through any clone
/// are visible to all of them), which makes it convenient to register the sink
/// in an [`AppState`] while still holding a handle for assertions.
#[derive(Clone, Default)]
pub struct InMemoryAuditSink {
    events: Arc<Mutex<Vec<AuditEvent>>>,
}

impl InMemoryAuditSink {
    /// Creates a new, empty in-memory sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a snapshot of every event recorded so far.
    pub fn events(&self) -> Vec<AuditEvent> {
        self.events
            .lock()
            .expect("audit sink mutex poisoned")
            .clone()
    }

    /// Returns the number of events currently buffered.
    pub fn len(&self) -> usize {
        self.events.lock().expect("audit sink mutex poisoned").len()
    }

    /// Returns `true` when no events have been recorded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Removes all buffered events.
    pub fn clear(&self) {
        self.events
            .lock()
            .expect("audit sink mutex poisoned")
            .clear();
    }
}

#[async_trait]
impl AuditSink for InMemoryAuditSink {
    async fn record(&self, event: AuditEvent) {
        self.events
            .lock()
            .expect("audit sink mutex poisoned")
            .push(event);
    }

    async fn flush(&self) {}

    fn name(&self) -> &str {
        "memory"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::AuditEventBuilder;

    #[tokio::test]
    async fn test_record_stores_event() {
        let sink = InMemoryAuditSink::new();
        let event = AuditEventBuilder::new("Device/hfs")
            .detail("audit-operation", "test")
            .build();
        sink.record(event).await;
        assert_eq!(sink.len(), 1);
    }

    #[tokio::test]
    async fn test_clones_share_buffer() {
        let sink = InMemoryAuditSink::new();
        let clone = sink.clone();
        clone
            .record(AuditEventBuilder::new("Device/hfs").build())
            .await;
        assert_eq!(sink.len(), 1);
        assert_eq!(clone.len(), 1);
    }

    #[tokio::test]
    async fn test_clear_resets_buffer() {
        let sink = InMemoryAuditSink::new();
        sink.record(AuditEventBuilder::new("Device/hfs").build())
            .await;
        sink.clear();
        assert!(sink.is_empty());
    }

    #[test]
    fn test_name() {
        assert_eq!(InMemoryAuditSink::new().name(), "memory");
    }
}
