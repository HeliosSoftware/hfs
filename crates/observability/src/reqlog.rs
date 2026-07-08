//! In-process rolling request log for dashboard traffic widgets.
//!
//! The Prometheus pull exporter ([`crate::metrics`]) holds *cumulative* counters
//! and an all-time latency histogram — enough for Grafana to compute `rate()`
//! and `histogram_quantile()` over a range, but not enough for a self-contained
//! dashboard that wants "requests/sec, p95, and error rate over the last hour"
//! without a Prometheus deployment.
//!
//! This module keeps a bounded ring buffer of recent request samples and derives
//! windowed statistics from it on demand. It is intentionally lightweight:
//!
//! - Fixed capacity ([`CAPACITY`]); the oldest sample is evicted on overflow, so
//!   memory is bounded regardless of traffic.
//! - One `Mutex` guarding a `VecDeque`; the critical section is a push/pop, so
//!   contention stays negligible at dashboard-relevant request rates.
//! - No per-route retention — that would need windowed per-key aggregation;
//!   route-level stats remain Prometheus's job.
//!
//! [`record`] is called once per request by [`crate::middleware::track`];
//! [`snapshot`] and [`per_tenant`] back the console `traffic` and `tenants`
//! endpoints.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

/// Maximum number of request samples retained. At ~20 k samples the buffer
/// covers a comfortably long window for any realistic dashboard refresh while
/// staying well under a megabyte.
pub const CAPACITY: usize = 20_000;

/// Number of sub-buckets in a [`TrafficStats::series`] sparkline.
const SERIES_BUCKETS: usize = 30;

/// A single recorded request.
#[derive(Clone)]
struct Sample {
    /// Epoch milliseconds when the request completed.
    at_ms: i64,
    /// End-to-end latency in milliseconds.
    latency_ms: f32,
    /// HTTP status code.
    status: u16,
    /// Resolved tenant id (`X-Tenant-ID`), or empty when not supplied.
    tenant: Box<str>,
}

static LOG: OnceLock<Mutex<VecDeque<Sample>>> = OnceLock::new();

fn log() -> &'static Mutex<VecDeque<Sample>> {
    LOG.get_or_init(|| Mutex::new(VecDeque::with_capacity(1024)))
}

/// Record one completed request. Called from the request middleware.
///
/// `tenant` should be the raw `X-Tenant-ID` value (empty string if absent);
/// [`per_tenant`] normalises the empty case to `"default"`.
pub fn record(status: u16, latency_secs: f64, tenant: &str) {
    let sample = Sample {
        at_ms: chrono::Utc::now().timestamp_millis(),
        latency_ms: (latency_secs * 1000.0) as f32,
        status,
        tenant: tenant.into(),
    };
    if let Ok(mut q) = log().lock() {
        if q.len() >= CAPACITY {
            q.pop_front();
        }
        q.push_back(sample);
    }
}

/// Windowed traffic statistics, derived from the samples newer than
/// `now - window_seconds`.
#[derive(Debug, Clone)]
pub struct TrafficStats {
    /// The requested window length.
    pub window_seconds: i64,
    /// Span actually covered by the retained samples (≤ `window_seconds`; lower
    /// when the buffer does not reach back the full window).
    pub covered_seconds: f64,
    /// Number of samples in the window.
    pub sample_count: usize,
    /// Mean requests per second across the window.
    pub requests_per_second: f64,
    /// Latency percentiles in milliseconds.
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    /// Fraction of requests with a 5xx status, in `0.0..=1.0`.
    pub error_rate: f64,
    /// Status-class counts.
    pub status_2xx: u64,
    pub status_3xx: u64,
    pub status_4xx: u64,
    pub status_5xx: u64,
    /// Sparkline buckets, oldest first.
    pub series: Vec<TrafficBucket>,
}

/// One sparkline bucket within a [`TrafficStats`] window.
#[derive(Debug, Clone)]
pub struct TrafficBucket {
    /// Seconds before `now` at which this bucket starts (largest = oldest).
    pub offset_seconds: i64,
    /// Mean requests per second within the bucket.
    pub requests_per_second: f64,
    /// p95 latency (ms) within the bucket; `0.0` when the bucket is empty.
    pub p95_ms: f64,
}

/// Compute traffic statistics over the last `window_seconds`.
///
/// Pass `tenant = Some(id)` to restrict to a single tenant, or `None` for all
/// traffic. The lock is held only long enough to copy the in-window samples.
pub fn snapshot(window_seconds: i64, tenant: Option<&str>) -> TrafficStats {
    let window = window_seconds.max(1);
    let now_ms = chrono::Utc::now().timestamp_millis();
    let start_ms = now_ms - window * 1000;

    let mut samples: Vec<(i64, f32, u16)> = Vec::new();
    if let Ok(q) = log().lock() {
        for s in q.iter() {
            if s.at_ms >= start_ms && tenant.is_none_or(|t| &*s.tenant == t) {
                samples.push((s.at_ms, s.latency_ms, s.status));
            }
        }
    }

    let sample_count = samples.len();
    let covered_seconds = samples
        .iter()
        .map(|s| s.0)
        .min()
        .map(|oldest| (now_ms - oldest) as f64 / 1000.0)
        .unwrap_or(0.0)
        .min(window as f64);

    let (mut s2, mut s3, mut s4, mut s5) = (0u64, 0u64, 0u64, 0u64);
    let mut latencies: Vec<f32> = Vec::with_capacity(sample_count);
    for &(_, latency, status) in &samples {
        latencies.push(latency);
        match status / 100 {
            2 => s2 += 1,
            3 => s3 += 1,
            4 => s4 += 1,
            5 => s5 += 1,
            _ => {}
        }
    }

    let requests_per_second = sample_count as f64 / window as f64;
    let error_rate = if sample_count > 0 {
        s5 as f64 / sample_count as f64
    } else {
        0.0
    };

    TrafficStats {
        window_seconds: window,
        covered_seconds,
        sample_count,
        requests_per_second,
        p50_ms: percentile_ms(&mut latencies, 0.50),
        p95_ms: percentile_ms(&mut latencies, 0.95),
        p99_ms: percentile_ms(&mut latencies, 0.99),
        error_rate,
        status_2xx: s2,
        status_3xx: s3,
        status_4xx: s4,
        status_5xx: s5,
        series: build_series(&samples, start_ms, now_ms, window),
    }
}

/// Per-tenant traffic rollup over the last `window_seconds`, busiest first.
pub fn per_tenant(window_seconds: i64) -> Vec<TenantTraffic> {
    let window = window_seconds.max(1);
    let now_ms = chrono::Utc::now().timestamp_millis();
    let start_ms = now_ms - window * 1000;

    // Hold the lock only long enough to copy the in-window samples into a local
    // Vec; build the per-tenant rollup (HashMap, key allocation, percentiles)
    // after releasing the lock so `record()` is never stalled by this work.
    let mut samples: Vec<(Box<str>, f32, u16)> = Vec::new();
    if let Ok(q) = log().lock() {
        for s in q.iter() {
            if s.at_ms >= start_ms {
                samples.push((s.tenant.clone(), s.latency_ms, s.status));
            }
        }
    }

    // tenant -> (latencies, 5xx count)
    let mut by_tenant: std::collections::HashMap<String, (Vec<f32>, u64)> =
        std::collections::HashMap::new();
    for (tenant, latency, status) in samples {
        let key = if tenant.is_empty() {
            "default".to_string()
        } else {
            String::from(tenant)
        };
        let entry = by_tenant.entry(key).or_insert_with(|| (Vec::new(), 0));
        entry.0.push(latency);
        if status / 100 == 5 {
            entry.1 += 1;
        }
    }

    let mut out: Vec<TenantTraffic> = by_tenant
        .into_iter()
        .map(|(tenant, (mut latencies, errors))| {
            let n = latencies.len();
            TenantTraffic {
                tenant,
                sample_count: n,
                requests_per_second: n as f64 / window as f64,
                p95_ms: percentile_ms(&mut latencies, 0.95),
                error_rate: if n > 0 { errors as f64 / n as f64 } else { 0.0 },
            }
        })
        .collect();
    out.sort_by_key(|b| std::cmp::Reverse(b.sample_count));
    out
}

/// Per-tenant traffic figures returned by [`per_tenant`].
#[derive(Debug, Clone)]
pub struct TenantTraffic {
    pub tenant: String,
    pub sample_count: usize,
    pub requests_per_second: f64,
    pub p95_ms: f64,
    pub error_rate: f64,
}

/// Nearest-rank percentile (in ms) over `latencies`, which is sorted in place.
/// Returns `0.0` for an empty slice.
fn percentile_ms(latencies: &mut [f32], q: f64) -> f64 {
    if latencies.is_empty() {
        return 0.0;
    }
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = (q * latencies.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(latencies.len() - 1);
    latencies[idx] as f64
}

/// Bucket the window into [`SERIES_BUCKETS`] equal slices and compute per-slice
/// req/s and p95 for the sparkline.
fn build_series(
    samples: &[(i64, f32, u16)],
    start_ms: i64,
    now_ms: i64,
    window: i64,
) -> Vec<TrafficBucket> {
    let span_ms = (now_ms - start_ms).max(1);
    let bucket_ms = (span_ms as f64 / SERIES_BUCKETS as f64).max(1.0);
    let bucket_secs = bucket_ms / 1000.0;

    let mut latencies: Vec<Vec<f32>> = vec![Vec::new(); SERIES_BUCKETS];
    for &(at_ms, latency, _) in samples {
        let mut b = ((at_ms - start_ms) as f64 / bucket_ms) as usize;
        if b >= SERIES_BUCKETS {
            b = SERIES_BUCKETS - 1;
        }
        latencies[b].push(latency);
    }

    latencies
        .into_iter()
        .enumerate()
        .map(|(i, mut lat)| {
            let count = lat.len();
            // Offset (seconds before now) at which this bucket *starts*.
            let offset_seconds = window - (i as f64 * bucket_secs).round() as i64;
            TrafficBucket {
                offset_seconds: offset_seconds.max(0),
                requests_per_second: count as f64 / bucket_secs.max(1.0),
                p95_ms: percentile_ms(&mut lat, 0.95),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_basic() {
        let mut v: Vec<f32> = (1..=100).map(|n| n as f32).collect();
        assert_eq!(percentile_ms(&mut v, 0.50), 50.0);
        assert_eq!(percentile_ms(&mut v, 0.95), 95.0);
        assert_eq!(percentile_ms(&mut v, 0.99), 99.0);
        assert_eq!(percentile_ms(&mut [], 0.95), 0.0);
    }

    #[test]
    fn snapshot_counts_recorded_requests() {
        // Records land in the shared buffer; a wide window should see them and
        // classify statuses. (Other tests share the process-global buffer, so we
        // assert on monotonic presence rather than exact totals.)
        record(200, 0.010, "tenant-a");
        record(503, 0.050, "tenant-a");
        let stats = snapshot(3600, Some("tenant-a"));
        assert!(stats.sample_count >= 2);
        assert!(stats.status_2xx >= 1);
        assert!(stats.status_5xx >= 1);
        assert!(stats.error_rate > 0.0);
        assert_eq!(stats.series.len(), SERIES_BUCKETS);
    }

    #[test]
    fn per_tenant_rolls_up_and_normalises_empty() {
        // Unique tenant so we can assert on it despite the shared buffer.
        record(200, 0.010, "tenant-pt");
        record(503, 0.050, "tenant-pt");
        // Empty tenant must normalise to "default".
        record(200, 0.010, "");

        let rows = per_tenant(3600);

        let pt = rows
            .iter()
            .find(|r| r.tenant == "tenant-pt")
            .expect("tenant-pt present");
        assert!(pt.sample_count >= 2);
        assert!(pt.error_rate > 0.0);

        assert!(
            rows.iter().any(|r| r.tenant == "default"),
            "empty tenant should roll up under \"default\""
        );
        // Busiest-first ordering preserved.
        assert!(
            rows.windows(2)
                .all(|w| w[0].sample_count >= w[1].sample_count)
        );
    }
}
