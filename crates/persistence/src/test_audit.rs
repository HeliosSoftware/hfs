//! Test-only audit sink for asserting on emitted `AuditEvent`s.
//!
//! This is `#[cfg(test)]` and therefore compiles only for `cargo test` on this
//! crate — it is not part of the public API and cannot be selected at runtime
//! via `HFS_AUDIT_BACKEND`. Deliberately not a cargo feature: CI builds the
//! workspace with `--all-features --release`, which would ship a
//! feature-gated in-memory sink in the released binary.
//!
//! The buffer is unbounded, which is fine for a test that records a handful of
//! events and would be a memory leak in a server.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use helios_audit::AuditSink;
use helios_fhir::r4::{AuditEvent, AuditEventEntityDetailValue};

/// Retains every recorded [`AuditEvent`] so tests can assert on them.
///
/// Clones share one buffer, so a handle registered as an `Arc<dyn AuditSink>`
/// and a handle kept for assertions observe the same events.
#[derive(Clone, Default)]
pub struct CollectorSink {
    events: Arc<Mutex<Vec<AuditEvent>>>,
}

impl CollectorSink {
    /// Creates an empty collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a snapshot of every event recorded so far.
    pub fn events(&self) -> Vec<AuditEvent> {
        self.events
            .lock()
            .expect("collector mutex poisoned")
            .clone()
    }
}

#[async_trait]
impl AuditSink for CollectorSink {
    async fn record(&self, event: AuditEvent) {
        self.events
            .lock()
            .expect("collector mutex poisoned")
            .push(event);
    }

    async fn flush(&self) {}

    fn name(&self) -> &str {
        "collector"
    }
}

/// Flattens an event's `entity[].detail[]` string values into a lookup map.
///
/// The builder spreads details across entities depending on whether a resource
/// was set, so addressing them positionally is brittle.
pub fn detail_map(event: &AuditEvent) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for entity in event.entity.as_ref().into_iter().flatten() {
        for detail in entity.detail.as_ref().into_iter().flatten() {
            let Some(key) = detail.r#type.value.clone() else {
                continue;
            };
            if let Some(AuditEventEntityDetailValue::String(s)) = &detail.value {
                map.insert(key, s.value.clone().unwrap_or_default());
            }
        }
    }
    map
}
