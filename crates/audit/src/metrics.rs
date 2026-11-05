//! Prometheus series for the audit pipeline (#2234).
//!
//! An audit sink never fails the request it audits, so a lost `AuditEvent`
//! used to leave only an `ERROR` log line. These series make the loss and the
//! backlog visible on `/metrics`:
//!
//! - `audit_events_dropped_total{sink,reason}`: events that were never
//!   persisted. `reason` is one of the `REASON_*` constants below.
//! - `audit_write_retries_total{sink}`: transient write failures that were
//!   retried.
//! - `audit_queue_depth{sink}`: events accepted by a
//!   [`QueuedSink`](crate::sinks::QueuedSink) and not yet handed back by its
//!   writers (queued or being written).
//!
//! `sink` is the sink's [`name`](crate::AuditSink::name), a fixed set chosen
//! at startup. No tenant, resource type or path appears in a label.
//!
//! Every function records through the [`metrics`] facade, so it is a no-op in
//! a process that installed no recorder.

/// Counter: audit events that were never persisted.
pub const EVENTS_DROPPED: &str = "audit_events_dropped_total";
/// Counter: transient audit write failures that were retried.
pub const WRITE_RETRIES: &str = "audit_write_retries_total";
/// Gauge: events accepted by a queued sink and not yet written.
pub const QUEUE_DEPTH: &str = "audit_queue_depth";

/// The queue was full and the overflow policy is `drop`.
pub const REASON_QUEUE_FULL: &str = "queue_full";
/// The queue's writers are gone (the sink is shutting down).
pub const REASON_QUEUE_CLOSED: &str = "queue_closed";
/// The event could not be serialized.
pub const REASON_SERIALIZE: &str = "serialize_failed";
/// The store refused the write with a non-transient error.
pub const REASON_WRITE_FAILED: &str = "write_failed";
/// The store kept failing transiently until the retries ran out.
pub const REASON_RETRIES_EXHAUSTED: &str = "retries_exhausted";

/// Count `count` events dropped by `sink` for `reason`.
pub fn record_dropped(sink: &str, reason: &'static str, count: u64) {
    metrics::counter!(EVENTS_DROPPED, "sink" => sink.to_string(), "reason" => reason)
        .increment(count);
}

/// Count one retried write on `sink`.
pub(crate) fn record_retry(sink: &str) {
    metrics::counter!(WRITE_RETRIES, "sink" => sink.to_string()).increment(1);
}

/// Publish the current backlog of `sink`.
pub(crate) fn set_queue_depth(sink: &str, depth: usize) {
    metrics::gauge!(QUEUE_DEPTH, "sink" => sink.to_string()).set(depth as f64);
}
