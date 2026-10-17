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

#[cfg(test)]
mod tests {
    use super::*;

    fn record_everything() {
        record_secondary_sync_failure("es", "create");
        record_secondary_sync_failure("es", "create");
        record_secondary_sync_failure("es", "delete");
        set_secondary_sync_needs_reindex(2);
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

        // Every label key is `service`, `backend` or `operation`.
        for line in text.lines().filter(|line| !line.starts_with('#')) {
            let Some((_, labels)) = line.split_once('{') else {
                continue;
            };
            let labels = labels.split_once('}').map(|(l, _)| l).unwrap_or(labels);
            for label in labels.split(',') {
                let key = label.split_once('=').map(|(k, _)| k).unwrap_or(label);
                assert!(
                    ["service", "backend", "operation"].contains(&key),
                    "unexpected label {key} in: {line}"
                );
            }
        }
    }
}
