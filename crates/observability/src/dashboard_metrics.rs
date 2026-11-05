//! Process-level Prometheus metrics for the dashboard's background counter
//! reconcile (#1078).
//!
//! The Home dashboard answers from in-memory counters, and a background task
//! keeps them honest by reconciling them against storage. These metrics let an
//! operator see that loop working — how long a pass takes, when the last one
//! ran, how slow the storage reads behind it are, how much the counters had
//! drifted, and how much seeding is still queued — without the dashboard
//! itself ever touching storage on a page load.
//!
//! ## Tenant privacy
//!
//! `/metrics` is public (see [`crate::metrics`]). Every metric here is
//! **process-level**: no tenant label, no resource-type label, and nothing a
//! scraper could sum or count to learn how many tenants the server holds. The
//! only label is `query`, over the fixed [`StorageQuery`] set. Keep it that way:
//! per-tenant figures belong to the authenticated dashboard, never here.
//!
//! Every function is safe to call without an installed recorder: the
//! [`metrics`] facade then records into its no-op recorder.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Histogram: wall time of one reconcile pass, in seconds.
pub(crate) const RECONCILE_PASS_DURATION: &str = "dashboard_reconcile_pass_duration_seconds";
/// Gauge: Unix time (seconds) the last reconcile pass finished.
pub(crate) const RECONCILE_LAST_PASS_TIMESTAMP: &str =
    "dashboard_reconcile_last_pass_timestamp_seconds";
/// Gauge: the configured pause between reconcile passes, in seconds.
pub(crate) const RECONCILE_INTERVAL: &str = "dashboard_reconcile_interval_seconds";
/// Counter: times the reconcile task was restarted after it stopped.
pub(crate) const RECONCILE_RESTARTS: &str = "dashboard_reconcile_restarts_total";
/// Histogram: duration of one storage query made for the dashboard, in
/// seconds, labelled by `query`.
pub(crate) const STORAGE_QUERY_DURATION: &str = "dashboard_storage_query_duration_seconds";
/// Counter: storage queries made for the dashboard that failed, labelled by
/// `query`.
pub(crate) const STORAGE_QUERY_ERRORS: &str = "dashboard_storage_query_errors_total";
/// Gauge: history rings waiting in the seed queue.
pub(crate) const SEED_QUEUE_RINGS: &str = "dashboard_seed_queue_rings";
/// Histogram: resources a reconcile pass corrected the counters by.
pub(crate) const RECONCILE_CORRECTION: &str = "dashboard_reconcile_correction_resources";

/// Histogram buckets (seconds) for the dashboard's duration histograms: from a
/// millisecond-scale row-store read up to a ten-minute object-store scan.
pub(crate) const DURATION_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0,
];

/// Histogram buckets for [`record_reconcile_correction`], one per order of
/// magnitude; the `0` bucket separates passes that found nothing to correct.
pub(crate) const CORRECTION_BUCKETS: &[f64] = &[
    0.0,
    1.0,
    10.0,
    100.0,
    1_000.0,
    10_000.0,
    100_000.0,
    1_000_000.0,
];

/// The storage reads the dashboard makes, the fixed values of the `query`
/// label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StorageQuery {
    /// Per-type stored totals.
    Totals,
    /// History deltas behind a chart window.
    History,
    /// The marker that detects writes made outside this process.
    WriteMarker,
    /// Bulk export and import job counts.
    JobCounts,
}

impl StorageQuery {
    /// The `query` label value.
    pub fn as_str(self) -> &'static str {
        match self {
            StorageQuery::Totals => "totals",
            StorageQuery::History => "history",
            StorageQuery::WriteMarker => "write_marker",
            StorageQuery::JobCounts => "job_counts",
        }
    }
}

/// Record one finished reconcile pass: its duration, and now as the last pass
/// time.
pub fn record_reconcile_pass(duration: Duration) {
    metrics::histogram!(RECONCILE_PASS_DURATION).record(duration.as_secs_f64());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs_f64())
        .unwrap_or_default();
    metrics::gauge!(RECONCILE_LAST_PASS_TIMESTAMP).set(now);
}

/// Publish the configured pause between reconcile passes.
pub fn set_reconcile_interval(interval: Duration) {
    metrics::gauge!(RECONCILE_INTERVAL).set(interval.as_secs_f64());
}

/// Count one restart of the reconcile task.
pub fn record_reconcile_restart() {
    metrics::counter!(RECONCILE_RESTARTS).increment(1);
}

/// Record one storage query made for the dashboard: its duration and, when it
/// failed, an error.
pub fn record_storage_query(query: StorageQuery, duration: Duration, ok: bool) {
    metrics::histogram!(STORAGE_QUERY_DURATION, "query" => query.as_str())
        .record(duration.as_secs_f64());
    if !ok {
        metrics::counter!(STORAGE_QUERY_ERRORS, "query" => query.as_str()).increment(1);
    }
}

/// Publish the seed queue depth: history rings still waiting. Queued tenants
/// are deliberately not exported: right after startup every tenant is queued,
/// so that depth would reveal how many tenants the server holds.
pub fn set_seed_queue(rings: usize) {
    metrics::gauge!(SEED_QUEUE_RINGS).set(rings as f64);
}

/// Record how many resources one reconcile pass corrected the counters by.
pub fn record_reconcile_correction(resources: u64) {
    metrics::histogram!(RECONCILE_CORRECTION).record(resources as f64);
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_QUERIES: [StorageQuery; 4] = [
        StorageQuery::Totals,
        StorageQuery::History,
        StorageQuery::WriteMarker,
        StorageQuery::JobCounts,
    ];

    fn record_everything() {
        record_reconcile_pass(Duration::from_millis(250));
        set_reconcile_interval(Duration::from_secs(30));
        record_reconcile_restart();
        for query in ALL_QUERIES {
            record_storage_query(query, Duration::from_millis(20), true);
            record_storage_query(query, Duration::from_secs(2), false);
        }
        set_seed_queue(12);
        record_reconcile_correction(0);
        record_reconcile_correction(1_500);
    }

    #[test]
    fn recording_without_a_recorder_does_not_panic() {
        record_everything();
    }

    #[test]
    fn label_values_are_stable() {
        let labels: Vec<_> = ALL_QUERIES.iter().map(|q| q.as_str()).collect();
        assert_eq!(labels, ["totals", "history", "write_marker", "job_counts"]);
    }

    /// Renders through a recorder built exactly as [`crate::metrics::init`]
    /// builds the global one, installed only for this thread.
    #[test]
    fn metrics_render_with_names_histogram_buckets_and_no_tenant_labels() {
        let recorder = crate::metrics::builder("test").build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, record_everything);
        let text = handle.render();

        for name in [
            RECONCILE_LAST_PASS_TIMESTAMP,
            RECONCILE_INTERVAL,
            RECONCILE_RESTARTS,
            STORAGE_QUERY_ERRORS,
            SEED_QUEUE_RINGS,
        ] {
            assert!(text.contains(name), "{name} missing from:\n{text}");
        }
        // Explicit buckets make real histograms, not summaries.
        for name in [
            RECONCILE_PASS_DURATION,
            STORAGE_QUERY_DURATION,
            RECONCILE_CORRECTION,
        ] {
            assert!(
                text.contains(&format!("{name}_bucket")),
                "{name} is not a histogram:\n{text}"
            );
        }
        assert!(
            text.contains(
                "dashboard_reconcile_pass_duration_seconds_bucket{service=\"test\",le=\"600\"}"
            ),
            "duration buckets applied:\n{text}"
        );
        assert!(
            text.contains(
                "dashboard_reconcile_correction_resources_bucket{service=\"test\",le=\"1000000\"}"
            ),
            "correction buckets applied:\n{text}"
        );
        for query in ALL_QUERIES {
            assert!(
                text.contains(&format!("query=\"{}\"", query.as_str())),
                "{} label missing:\n{text}",
                query.as_str()
            );
        }
        // Every label key is `service`, `query` or the histogram's `le`: no
        // tenant or resource-type label can slip in.
        for line in text.lines().filter(|line| !line.starts_with('#')) {
            let Some((_, labels)) = line.split_once('{') else {
                continue;
            };
            let labels = labels.split_once('}').map_or(labels, |(inner, _)| inner);
            for pair in labels.split(',') {
                let key = pair.split_once('=').map_or(pair, |(key, _)| key);
                assert!(
                    ["service", "query", "le"].contains(&key),
                    "unexpected label {key:?} in {line:?}"
                );
            }
        }
    }
}
