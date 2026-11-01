//! Process-level Prometheus metrics for composite storage's secondary sync
//! (#1334).
//!
//! A composite write succeeds once the primary has committed it; a secondary
//! (the Elasticsearch search index) that then refuses the change after retries
//! leaves the resource missing from search. These metrics are what an operator
//! alerts on: a counter of such failures, and a gauge of resources still
//! recorded as needing a reindex.
//!
//! ## Tenant privacy and cardinality
//!
//! `/metrics` is public (see [`crate::metrics`]). The only labels are
//! `backend` — the secondary's configured id, a handful of values fixed at
//! startup — and `operation`, over a fixed set. No tenant, resource type or
//! resource id: those are unbounded, and they identify data. Which resources
//! are affected is in the structured log event and the durable record, both
//! behind the operator's own access control.
//!
//! Every function is safe to call without an installed recorder: the
//! [`metrics`] facade then records into its no-op recorder.

/// Counter: secondary syncs that failed for good (after retries), labelled by
/// `backend` and `operation`.
pub(crate) const SECONDARY_SYNC_FAILURES: &str = "composite_secondary_sync_failures_total";
/// Gauge: resources currently recorded as needing a reindex on a secondary.
pub(crate) const SECONDARY_SYNC_NEEDS_REINDEX: &str = "composite_secondary_sync_needs_reindex";

/// Gauge: resources queued for asynchronous secondary sync and not yet
/// written or recorded as failed (#1545).
pub(crate) const SECONDARY_SYNC_QUEUE_DEPTH: &str = "composite_secondary_sync_queue_depth";

/// Gauge: documents missing from a secondary (on the primary only), as of the
/// last finished reconcile pass, labelled by `backend`.
pub(crate) const SECONDARY_RECONCILE_MISSING: &str = "composite_secondary_reconcile_missing";
/// Gauge: documents only on a secondary, as of the last finished reconcile pass.
pub(crate) const SECONDARY_RECONCILE_EXTRA: &str = "composite_secondary_reconcile_extra";
/// Gauge: documents on both sides with different versions, as of the last
/// finished reconcile pass.
pub(crate) const SECONDARY_RECONCILE_STALE: &str = "composite_secondary_reconcile_stale";
/// Histogram: seconds a whole reconcile pass took, labelled by `backend`.
pub(crate) const SECONDARY_RECONCILE_DURATION: &str =
    "composite_secondary_reconcile_duration_seconds";

/// Count one final secondary sync failure. `operation` is one of `create`,
/// `update`, `delete`.
pub fn record_secondary_sync_failure(backend: &str, operation: &'static str) {
    metrics::counter!(
        SECONDARY_SYNC_FAILURES,
        "backend" => backend.to_string(),
        "operation" => operation
    )
    .increment(1);
}

/// Publish how many resources are recorded as needing a reindex.
pub fn set_secondary_sync_needs_reindex(outstanding: u64) {
    metrics::gauge!(SECONDARY_SYNC_NEEDS_REINDEX).set(outstanding as f64);
}

/// Publish how many resources are waiting in the asynchronous sync queue.
pub fn set_secondary_sync_queue_depth(depth: u64) {
    metrics::gauge!(SECONDARY_SYNC_QUEUE_DEPTH).set(depth as f64);
}

/// Publish the drift a finished reconcile pass found on `backend`, summed
/// over every tenant and resource type it covered (per-type detail is in the
/// log: `/metrics` is public and carries no type labels).
pub fn set_secondary_reconcile_drift(backend: &str, missing: u64, extra: u64, stale: u64) {
    let backend = backend.to_string();
    metrics::gauge!(SECONDARY_RECONCILE_MISSING, "backend" => backend.clone()).set(missing as f64);
    metrics::gauge!(SECONDARY_RECONCILE_EXTRA, "backend" => backend.clone()).set(extra as f64);
    metrics::gauge!(SECONDARY_RECONCILE_STALE, "backend" => backend).set(stale as f64);
}

/// Record how long a whole reconcile pass on `backend` took.
pub fn record_secondary_reconcile_duration(backend: &str, duration: std::time::Duration) {
    metrics::histogram!(SECONDARY_RECONCILE_DURATION, "backend" => backend.to_string())
        .record(duration.as_secs_f64());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_everything() {
        record_secondary_sync_failure("es", "create");
        record_secondary_sync_failure("es", "create");
        record_secondary_sync_failure("es", "delete");
        set_secondary_sync_needs_reindex(2);
        set_secondary_sync_queue_depth(7);
        set_secondary_reconcile_drift("es", 3, 1, 2);
        record_secondary_reconcile_duration("es", std::time::Duration::from_millis(1500));
    }

    #[test]
    fn recording_without_a_recorder_does_not_panic() {
        record_everything();
    }

    /// Renders through a recorder built exactly as [`crate::metrics::init`]
    /// builds the global one, installed only for this thread.
    #[test]
    fn metrics_render_with_backend_and_operation_labels_only() {
        let recorder = crate::metrics::builder("test").build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, record_everything);
        let text = handle.render();

        let series = |operation: &str, value: u64| {
            text.lines().any(|line| {
                line.starts_with(SECONDARY_SYNC_FAILURES)
                    && line.contains("backend=\"es\"")
                    && line.contains(&format!("operation=\"{operation}\""))
                    && line.ends_with(&format!(" {value}"))
            })
        };
        assert!(series("create", 2), "create counted twice:\n{text}");
        assert!(series("delete", 1), "delete counted once:\n{text}");
        assert!(
            text.lines().any(|line| {
                line.starts_with(SECONDARY_SYNC_NEEDS_REINDEX) && line.ends_with(" 2")
            }),
            "gauge published:\n{text}"
        );

        for (name, value) in [
            (SECONDARY_RECONCILE_MISSING, 3),
            (SECONDARY_RECONCILE_EXTRA, 1),
            (SECONDARY_RECONCILE_STALE, 2),
        ] {
            assert!(
                text.lines().any(|line| line.starts_with(name)
                    && line.contains("backend=\"es\"")
                    && line.ends_with(&format!(" {value}"))),
                "{name} published:\n{text}"
            );
        }
        assert!(
            text.contains(SECONDARY_RECONCILE_DURATION),
            "duration recorded:\n{text}"
        );

        // Every label key is `service`, `backend` or `operation`.
        for line in text.lines().filter(|line| !line.starts_with('#')) {
            let Some((_, labels)) = line.split_once('{') else {
                continue;
            };
            let labels = labels.split_once('}').map(|(l, _)| l).unwrap_or(labels);
            for label in labels.split(',') {
                let key = label.split_once('=').map(|(k, _)| k).unwrap_or(label);
                assert!(
                    ["service", "backend", "operation", "quantile"].contains(&key),
                    "unexpected label {key} in: {line}"
                );
            }
        }
    }
}
