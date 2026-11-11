//! Process-level Prometheus metrics for MongoDB's deferred search indexing
//! (#1975, #2034, #2036).
//!
//! A large transaction Bundle commits without its search rows and writes them
//! right after the commit; whatever that post-commit step does not finish
//! (a crash, the Bundle deadline, a failed batch) stays in the
//! `search_index_pending` ledger until the repair loop finishes it. Until
//! then the Bundle is committed but unsearchable. These series are what an
//! operator alerts on and sizes the lag from:
//!
//! | Series | Type | Set by |
//! |--------|------|--------|
//! | `mongodb_deferred_search_index_pending_chunks` | gauge | each repair pass |
//! | `mongodb_deferred_search_index_oldest_pending_seconds` | gauge | each repair pass |
//! | `mongodb_deferred_search_index_repair_failed_chunks_total` | counter | each repair pass |
//! | `mongodb_deferred_search_index_step_seconds` | histogram | each post-commit step |
//! | `mongodb_deferred_search_index_step_rows_total` | counter | each post-commit step |
//! | `mongodb_deferred_search_index_steps_incomplete_total{reason}` | counter | each post-commit step |
//!
//! The two gauges describe the ledger as the latest repair pass left it, so
//! they are as fresh as that pass: they are not exported at all when the repair
//! is off (`HFS_MONGODB_DEFERRED_INDEX_REPAIR_INTERVAL=0`) or when the process
//! has no MongoDB backend. A ledger document that appears and is finished by
//! the post-commit step between two passes is never seen by them; the step
//! series cover that.
//!
//! The step's rows per second is `rate(..._step_rows_total) /
//! rate(..._step_seconds_sum)`: the seconds are the step's own wall time, not
//! the Bundle's.
//!
//! ## Tenant privacy and cardinality
//!
//! `/metrics` is public (see [`crate::metrics`]). The only label is `reason`
//! on the incomplete-steps counter, over the fixed set [`IncompleteReason`].
//! No tenant, resource type, Bundle id or database name: which Bundle is
//! owed is in the structured log events and the ledger documents, behind the
//! operator's own access control.
//!
//! Every function is safe to call without an installed recorder: the
//! [`metrics`] facade then records into its no-op recorder.

/// Gauge: ledger documents after the latest repair pass, due or not.
pub(crate) const PENDING_CHUNKS: &str = "mongodb_deferred_search_index_pending_chunks";
/// Gauge: age in seconds of the oldest ledger document (by `created_at`),
/// `0` when the ledger is empty.
pub(crate) const OLDEST_PENDING_SECONDS: &str =
    "mongodb_deferred_search_index_oldest_pending_seconds";
/// Counter: ledger documents a repair pass worked on and had to leave in place.
pub(crate) const REPAIR_FAILED_CHUNKS: &str =
    "mongodb_deferred_search_index_repair_failed_chunks_total";
/// Histogram: seconds each post-commit step took.
pub(crate) const STEP_SECONDS: &str = "mongodb_deferred_search_index_step_seconds";
/// Counter: search rows written by post-commit steps.
pub(crate) const STEP_ROWS: &str = "mongodb_deferred_search_index_step_rows_total";
/// Counter: post-commit steps that left work to the repair, by `reason`.
pub(crate) const STEPS_INCOMPLETE: &str = "mongodb_deferred_search_index_steps_incomplete_total";

/// Bucket bounds (seconds) of [`STEP_SECONDS`]: a step writes its Bundle's
/// rows in batches, so it takes from tens of milliseconds for a Bundle just
/// over the deferral threshold to minutes for one near the ceiling, and is cut
/// at the Bundle deadline (about 10 minutes at the default request timeout).
pub(crate) const STEP_SECONDS_BUCKETS: &[f64] = &[
    0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0,
];

/// Why a post-commit step left work to the repair: the `reason` label of
/// [`STEPS_INCOMPLETE`], a fixed set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IncompleteReason {
    /// The step stopped at the Bundle's deadline.
    Deadline,
    /// A batch failed for good, a resource was refused by the cache in a
    /// transaction of its own, or a finished ledger document could not be
    /// deleted.
    Failure,
}

impl IncompleteReason {
    /// The label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deadline => "deadline",
            Self::Failure => "failure",
        }
    }
}

/// Publish the ledger as a repair pass left it: how many documents it holds
/// and how old the oldest is (`0` for an empty ledger).
pub fn set_backlog(pending_chunks: u64, oldest_pending_seconds: f64) {
    metrics::gauge!(PENDING_CHUNKS).set(pending_chunks as f64);
    metrics::gauge!(OLDEST_PENDING_SECONDS).set(oldest_pending_seconds);
}

/// Count the ledger documents one repair pass worked on and left in place.
/// Called for every pass, with `0` when nothing failed, so the series exists
/// from the first pass and `increase()` sees the first failure.
pub fn record_repair_failed_chunks(failed: u64) {
    metrics::counter!(REPAIR_FAILED_CHUNKS).increment(failed);
}

/// Record one post-commit step: its wall time, the rows it wrote, and, when it
/// left work to the repair, why. Both `reason` series are touched on every
/// step (with `0` for the one that did not happen) so each exists from the
/// first step.
pub fn record_step(seconds: f64, rows: u64, incomplete: Option<IncompleteReason>) {
    metrics::histogram!(STEP_SECONDS).record(seconds);
    metrics::counter!(STEP_ROWS).increment(rows);
    for reason in [IncompleteReason::Deadline, IncompleteReason::Failure] {
        metrics::counter!(STEPS_INCOMPLETE, "reason" => reason.as_str())
            .increment(u64::from(incomplete == Some(reason)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_everything() {
        set_backlog(3, 12.5);
        record_repair_failed_chunks(1);
        record_repair_failed_chunks(0);
        record_repair_failed_chunks(2);
        record_step(0.5, 1_000, None);
        record_step(2.0, 500, Some(IncompleteReason::Deadline));
        record_step(1.0, 250, Some(IncompleteReason::Failure));
        record_step(1.5, 0, Some(IncompleteReason::Failure));
    }

    #[test]
    fn recording_without_a_recorder_does_not_panic() {
        record_everything();
    }

    /// The value of the series `name` with exactly the label `labels` (besides
    /// `service`), if it is rendered.
    fn value_of(text: &str, name: &str, labels: &[(&str, &str)]) -> Option<String> {
        text.lines()
            .filter(|line| !line.starts_with('#'))
            .find(|line| {
                let Some((series, _)) = line.rsplit_once(' ') else {
                    return false;
                };
                let (series_name, series_labels) = match series.split_once('{') {
                    Some((n, l)) => (n, l.trim_end_matches('}')),
                    None => (series, ""),
                };
                series_name == name
                    && labels
                        .iter()
                        .all(|(k, v)| series_labels.contains(&format!("{k}=\"{v}\"")))
            })
            .and_then(|line| line.rsplit_once(' ').map(|(_, v)| v.to_string()))
    }

    /// Renders through a recorder built exactly as [`crate::metrics::init`]
    /// builds the global one, installed only for this thread.
    #[test]
    fn metrics_render_with_the_reason_label_only() {
        let recorder = crate::metrics::builder("test").build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, record_everything);
        let text = handle.render();

        assert_eq!(
            value_of(&text, PENDING_CHUNKS, &[]).as_deref(),
            Some("3"),
            "pending chunks:\n{text}"
        );
        assert_eq!(
            value_of(&text, OLDEST_PENDING_SECONDS, &[]).as_deref(),
            Some("12.5"),
            "oldest pending:\n{text}"
        );
        assert_eq!(
            value_of(&text, REPAIR_FAILED_CHUNKS, &[]).as_deref(),
            Some("3"),
            "failed chunks are summed over passes:\n{text}"
        );
        assert_eq!(
            value_of(&text, STEP_ROWS, &[]).as_deref(),
            Some("1750"),
            "rows are summed over steps:\n{text}"
        );
        assert_eq!(
            value_of(&text, STEPS_INCOMPLETE, &[("reason", "deadline")]).as_deref(),
            Some("1"),
            "one deadline step:\n{text}"
        );
        assert_eq!(
            value_of(&text, STEPS_INCOMPLETE, &[("reason", "failure")]).as_deref(),
            Some("2"),
            "two failed steps:\n{text}"
        );

        // The step histogram is a real histogram with the seconds buckets.
        let seconds_name = format!("{STEP_SECONDS}_sum");
        let count_name = format!("{STEP_SECONDS}_count");
        assert_eq!(
            value_of(&text, &count_name, &[]).as_deref(),
            Some("4"),
            "four steps observed:\n{text}"
        );
        assert_eq!(
            value_of(&text, &seconds_name, &[]).as_deref(),
            Some("5"),
            "step seconds are summed:\n{text}"
        );
        let bucket_name = format!("{STEP_SECONDS}_bucket");
        assert_eq!(
            value_of(&text, &bucket_name, &[("le", "0.5")]).as_deref(),
            Some("1"),
            "one step took at most 0.5 s:\n{text}"
        );
        assert_eq!(
            value_of(&text, &bucket_name, &[("le", "600")]).as_deref(),
            Some("4"),
            "the 600 s bucket is configured:\n{text}"
        );

        // Every label key is `service` or `reason`; `le` is the histogram's
        // own bucket bound, not a data label.
        for line in text.lines().filter(|line| !line.starts_with('#')) {
            let Some((_, labels)) = line.split_once('{') else {
                continue;
            };
            let labels = labels.split_once('}').map(|(l, _)| l).unwrap_or(labels);
            for label in labels.split(',') {
                let key = label.split_once('=').map(|(k, _)| k).unwrap_or(label);
                assert!(
                    ["service", "reason", "le"].contains(&key),
                    "unexpected label {key} in: {line}"
                );
            }
        }
    }

    /// Both `reason` series exist after the first step, at `0` for the one
    /// that did not happen, so a rate over them sees the first increment.
    #[test]
    fn both_reasons_exist_after_a_complete_step() {
        let recorder = crate::metrics::builder("test").build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, || record_step(0.1, 10, None));
        let text = handle.render();
        for reason in ["deadline", "failure"] {
            assert_eq!(
                value_of(&text, STEPS_INCOMPLETE, &[("reason", reason)]).as_deref(),
                Some("0"),
                "{reason}:\n{text}"
            );
        }
    }

    #[test]
    fn the_reason_labels_are_the_fixed_set() {
        assert_eq!(IncompleteReason::Deadline.as_str(), "deadline");
        assert_eq!(IncompleteReason::Failure.as_str(), "failure");
    }
}
