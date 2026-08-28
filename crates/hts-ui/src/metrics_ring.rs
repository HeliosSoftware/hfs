//! In-process sample ring for the Home page request-rate chart (§7.1).
//!
//! # Why a ring at all
//!
//! HTS `/metrics` exposes only *cumulative* Prometheus counters
//! (`http_requests_total`), a latency histogram, and an `uptime_seconds`
//! gauge. There is no time series to read: a scrape answers "how many
//! requests since this process started", not "how busy is it right now".
//!
//! Plotting the cumulative counter directly would be wrong twice over. It is
//! a counter, not a level — the curve only ever climbs, so it says nothing
//! about load — and an upstream restart would drop it to zero, rendering as
//! a cliff that reads as "traffic stopped" when in fact traffic is fine and
//! the *process* restarted. So the chart plots a **rate**: requests per
//! minute, differenced between consecutive scrapes.
//!
//! Differencing needs memory, and this is that memory: the last
//! [`RING_CAPACITY`] scrapes, fed from the `/metrics` leg the Home cards
//! fetch already performs, so the chart costs **zero extra upstream
//! requests**.
//!
//! # Honesty constraints
//!
//! The ring only grows while somebody has the Home page open — the 15 s
//! htmx poll is the only thing that samples it. That makes the series
//! *sparse and irregular*, and everything downstream is built for that:
//!
//! - **Restarts clear the ring.** When `uptime_seconds` goes backwards the
//!   upstream process is not the one we sampled before; its counters restart
//!   from zero and no prior sample is comparable. The ring is emptied so the
//!   restart renders as a gap, never as a negative rate or a cliff.
//! - **Unusable intervals produce no point.** A counter that went backwards,
//!   a non-advancing clock, or an unobserved stretch longer than
//!   [`MAX_CONTINUOUS_GAP_SECS`] yields nothing, and flags the *next* point
//!   as starting a new polyline segment. An hour nobody watched must show as
//!   a visible break, not a straight line implying we knew what happened.
//! - **The first sample yields no point.** A rate needs two scrapes.
//!
//! # Concurrency
//!
//! [`MetricsRing`] guards its deque with a [`std::sync::RwLock`], not a
//! `tokio::sync::RwLock`, because every critical section here is a
//! `push_back`/`pop_front`/copy over `Copy` structs — microseconds, no
//! allocation-heavy work, and above all **no `.await` is ever executed
//! while a guard is held**. Callers must keep it that way: take the
//! snapshot, drop the guard, then do async work. Holding a std lock across
//! an await would let a suspended task pin the lock across a scheduler
//! switch and deadlock the executor.

use crate::metrics_parse::StatusCounts;
use std::collections::VecDeque;
use std::sync::RwLock;

/// Samples retained per process. At the Home page's 15 s poll this is six
/// hours of continuous observation — exactly the longest offered window —
/// and about 46 KB of `Copy` structs.
pub const RING_CAPACITY: usize = 1440;

/// The Home page's htmx refresh interval, in seconds. Mirrors
/// `hx-trigger="every 15s"` in `partials/hts-home-cards.html`; the two must
/// move together.
pub const POLL_INTERVAL_SECS: f64 = 15.0;

/// Longest interval between two scrapes that is still drawn as one
/// continuous line. Four poll intervals of headroom absorbs a slow upstream
/// or a briefly backgrounded tab; beyond it we genuinely did not observe the
/// server, and the chart says so with a break rather than interpolating.
pub const MAX_CONTINUOUS_GAP_SECS: f64 = POLL_INTERVAL_SECS * 4.0;

/// One `/metrics` scrape, reduced to the numbers the chart differences.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricsSample {
    /// Wall-clock time of the scrape, in seconds since the UNIX epoch. The
    /// chart maps x by *timestamp*, never by sample index: these samples are
    /// neither dense nor equal-width, so index mapping would silently
    /// compress an unobserved hour into one pixel step.
    pub at_secs: f64,
    /// The upstream's `uptime_seconds` gauge at scrape time. Strictly
    /// increasing within one process lifetime, so a decrease is a restart.
    pub uptime_secs: f64,
    /// Cumulative request counters, self-traffic already excluded.
    pub counts: StatusCounts,
}

/// One plotted point: a rate derived from the interval ending at `at_secs`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RatePoint {
    /// Timestamp of the *later* scrape of the differenced pair.
    pub at_secs: f64,
    /// Requests per minute over that interval, rounded to a whole request.
    pub per_min: u64,
    /// `true` when this point must start a new `<polyline>`: either it is
    /// the first point, or the interval before it was unusable (gap, reset,
    /// or restart) and connecting it to the previous point would assert
    /// continuity we never observed.
    pub break_before: bool,
}

/// Fixed-capacity, newest-at-the-back ring of `/metrics` scrapes.
///
/// Lives on `HtsUiState` — deliberately **not** a module-level `static`.
/// A process-global would be shared by every `#[tokio::test]` in the crate,
/// so one test's pushes would leak into another's assertions.
#[derive(Debug)]
pub struct MetricsRing {
    capacity: usize,
    samples: RwLock<VecDeque<MetricsSample>>,
}

impl Default for MetricsRing {
    fn default() -> Self {
        Self::with_capacity(RING_CAPACITY)
    }
}

impl MetricsRing {
    /// A ring holding [`RING_CAPACITY`] samples.
    pub fn new() -> Self {
        Self::default()
    }

    /// A ring holding `capacity` samples. `0` is clamped to `1` so `push`
    /// always leaves at least the newest sample behind.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            samples: RwLock::new(VecDeque::with_capacity(capacity.min(RING_CAPACITY))),
        }
    }

    /// Record one scrape.
    ///
    /// Clears the ring first when `sample.uptime_secs` is *below* the last
    /// recorded uptime: the upstream restarted, its counters are back at
    /// zero, and differencing across the boundary would produce a negative
    /// (or, worse, a plausible-looking wrong) rate. Dropping the history
    /// makes the restart render as a gap.
    ///
    /// The write guard covers only the deque mutation; no `.await` runs
    /// inside it.
    pub fn push(&self, sample: MetricsSample) {
        let mut samples = self.samples.write().unwrap_or_else(|e| e.into_inner());
        if let Some(last) = samples.back()
            && sample.uptime_secs < last.uptime_secs
        {
            samples.clear();
        }
        samples.push_back(sample);
        while samples.len() > self.capacity {
            samples.pop_front();
        }
    }

    /// Copy the ring out, oldest first. Returns an owned `Vec` so the caller
    /// holds no guard while it renders (or awaits).
    pub fn snapshot(&self) -> Vec<MetricsSample> {
        self.samples
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .copied()
            .collect()
    }

    /// Number of retained samples.
    pub fn len(&self) -> usize {
        self.samples.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether no scrape has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Classify the interval between two consecutive scrapes.
///
/// `Some((dt, delta))` when the pair can be differenced into a rate;
/// `None` when it cannot, which is always for one of three reasons:
///
/// - the clock did not advance (`dt <= 0`) — nothing to divide by;
/// - the gap exceeds [`MAX_CONTINUOUS_GAP_SECS`] — we did not observe the
///   server across it, so any average would be invented detail;
/// - the counter went backwards — a reset. Rendering `cb - ca` here would
///   underflow, and rendering it as a signed dip would claim negative
///   traffic. The honest answer is no point at all.
fn usable_interval(
    a: &MetricsSample,
    b: &MetricsSample,
    pick: &impl Fn(&StatusCounts) -> u64,
) -> Option<(f64, u64)> {
    let dt = b.at_secs - a.at_secs;
    // NaN is checked first and explicitly: every float comparison against it
    // is false, so `dt <= 0.0` alone would wave it through and the division
    // below would yield a NaN rate that casts silently to 0.
    if dt.is_nan() || dt <= 0.0 || dt > MAX_CONTINUOUS_GAP_SECS {
        return None;
    }
    let (before, after) = (pick(&a.counts), pick(&b.counts));
    let delta = after.checked_sub(before)?;
    Some((dt, delta))
}

/// Difference consecutive scrapes into requests-per-minute points.
///
/// `pick` selects the counter to plot (all traffic, or one status class).
/// The output is at most `samples.len() - 1` long: the first scrape
/// establishes a baseline and yields no point.
pub fn rates(samples: &[MetricsSample], pick: impl Fn(&StatusCounts) -> u64) -> Vec<RatePoint> {
    let mut out = Vec::with_capacity(samples.len().saturating_sub(1));
    // The very first point always opens a segment.
    let mut pending_break = true;
    for pair in samples.windows(2) {
        match usable_interval(&pair[0], &pair[1], &pick) {
            Some((dt, delta)) => {
                let per_min = (delta as f64 / dt * 60.0).round().max(0.0);
                out.push(RatePoint {
                    at_secs: pair[1].at_secs,
                    per_min: per_min as u64,
                    break_before: pending_break,
                });
                pending_break = false;
            }
            // Unusable interval: emit nothing, and make the next point start
            // a fresh polyline so the break is visible.
            None => pending_break = true,
        }
    }
    out
}

/// Total requests observed in `[start, end]` for one counter, as the sum of
/// the usable per-interval deltas. Feeds the legend's `.chart-legend__total`.
///
/// Summing deltas rather than subtracting the endpoints keeps the number
/// honest across gaps and restarts: an unobserved stretch contributes zero
/// instead of silently attributing its whole backlog to the window.
pub fn window_total(
    samples: &[MetricsSample],
    start: f64,
    end: f64,
    pick: impl Fn(&StatusCounts) -> u64,
) -> u64 {
    let mut total = 0u64;
    for pair in samples.windows(2) {
        if pair[1].at_secs < start || pair[1].at_secs > end {
            continue;
        }
        if let Some((_, delta)) = usable_interval(&pair[0], &pair[1], &pick) {
            total = total.saturating_add(delta);
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(all: u64) -> StatusCounts {
        StatusCounts {
            all,
            s2xx: all,
            s4xx: 0,
            s5xx: 0,
        }
    }

    fn sample(at: f64, uptime: f64, all: u64) -> MetricsSample {
        MetricsSample {
            at_secs: at,
            uptime_secs: uptime,
            counts: counts(all),
        }
    }

    fn all(c: &StatusCounts) -> u64 {
        c.all
    }

    #[test]
    fn capacity_evicts_the_oldest_sample_first() {
        let ring = MetricsRing::with_capacity(3);
        for i in 0..5u64 {
            ring.push(sample(i as f64 * 15.0, i as f64 * 15.0, i * 10));
        }
        let held = ring.snapshot();
        assert_eq!(held.len(), 3, "ring must never exceed its capacity");
        assert_eq!(
            held.iter().map(|s| s.counts.all).collect::<Vec<_>>(),
            vec![20, 30, 40],
            "eviction must drop the oldest samples, keeping the newest",
        );
    }

    #[test]
    fn zero_capacity_is_clamped_to_one() {
        let ring = MetricsRing::with_capacity(0);
        ring.push(sample(0.0, 1.0, 1));
        ring.push(sample(15.0, 16.0, 2));
        assert_eq!(ring.len(), 1);
        assert_eq!(ring.snapshot()[0].counts.all, 2);
    }

    #[test]
    fn uptime_going_backwards_clears_the_ring() {
        // A restart: counters go back to zero and uptime resets. Prior
        // samples describe a different process and cannot be differenced
        // against the new ones.
        let ring = MetricsRing::new();
        ring.push(sample(0.0, 3600.0, 5_000));
        ring.push(sample(15.0, 3615.0, 5_100));
        assert_eq!(ring.len(), 2);

        ring.push(sample(30.0, 2.0, 7));
        let held = ring.snapshot();
        assert_eq!(
            held.len(),
            1,
            "an uptime regression must clear every pre-restart sample",
        );
        assert_eq!(
            held[0].counts.all, 7,
            "only the post-restart sample survives"
        );
    }

    #[test]
    fn a_restart_renders_as_a_gap_not_a_cliff() {
        // End to end through `rates`: the pre-restart samples are gone, so
        // the first post-restart interval is the only point, and it opens a
        // fresh segment. No point plots the 5100 -> 7 drop.
        let ring = MetricsRing::new();
        ring.push(sample(0.0, 3600.0, 5_000));
        ring.push(sample(15.0, 3615.0, 5_100));
        ring.push(sample(30.0, 2.0, 7));
        ring.push(sample(45.0, 17.0, 22));

        let points = rates(&ring.snapshot(), all);
        assert_eq!(
            points.len(),
            1,
            "only the post-restart interval is plottable"
        );
        assert!(
            points[0].break_before,
            "the surviving point opens a new segment"
        );
        assert_eq!(points[0].per_min, 60, "15 requests over 15 s is 60/min");
    }

    #[test]
    fn the_first_sample_yields_no_point() {
        let one = [sample(0.0, 1.0, 100)];
        assert!(
            rates(&one, all).is_empty(),
            "a rate needs two scrapes; one sample must plot nothing",
        );
        assert!(rates(&[], all).is_empty());
    }

    #[test]
    fn consecutive_samples_difference_into_requests_per_minute() {
        let s = [
            sample(0.0, 100.0, 0),
            sample(15.0, 115.0, 5),  // 5 in 15 s -> 20/min
            sample(30.0, 130.0, 35), // 30 in 15 s -> 120/min
        ];
        let points = rates(&s, all);
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].per_min, 20);
        assert_eq!(points[1].per_min, 120);
        assert!(points[0].break_before, "first point opens the segment");
        assert!(
            !points[1].break_before,
            "an observed interval stays connected"
        );
    }

    #[test]
    fn a_sampling_gap_splits_the_series_into_segments() {
        // Nobody had the page open between t=30 and t=3630. That hour must
        // be a visible break, not a straight line across the chart.
        let s = [
            sample(0.0, 100.0, 0),
            sample(15.0, 115.0, 5),
            sample(30.0, 130.0, 10),
            sample(3_630.0, 3_730.0, 9_000), // one unobserved hour
            sample(3_645.0, 3_745.0, 9_010),
        ];
        let points = rates(&s, all);
        assert_eq!(
            points.len(),
            3,
            "the hour-long interval must yield no point of its own: {points:?}",
        );
        assert_eq!(
            points.iter().map(|p| p.break_before).collect::<Vec<_>>(),
            vec![true, false, true],
            "the point after the gap must open a new segment",
        );
        assert_eq!(points[2].per_min, 40, "10 requests over 15 s is 40/min");
    }

    #[test]
    fn a_counter_reset_yields_a_gap_not_a_negative() {
        // Same process (uptime keeps climbing) but the counter went
        // backwards — a recorder reset. Subtracting would underflow; the
        // honest rendering is no point plus a segment break.
        let s = [
            sample(0.0, 100.0, 1_000),
            sample(15.0, 115.0, 1_050),
            sample(30.0, 130.0, 4), // reset without a restart
            sample(45.0, 145.0, 19),
        ];
        let points = rates(&s, all);
        assert_eq!(points.len(), 2, "the reset interval must plot nothing");
        assert_eq!(points[0].per_min, 200);
        assert_eq!(points[1].per_min, 60);
        assert!(
            points[1].break_before,
            "the first post-reset point must open a new segment",
        );
        assert!(
            points.iter().all(|p| p.per_min <= 200),
            "no point may carry a bogus value from the reset boundary",
        );
    }

    #[test]
    fn a_non_advancing_clock_yields_no_point() {
        // Wall clock stepped backwards (NTP correction) or two scrapes
        // landed on the same instant: dividing by zero or a negative dt
        // would be nonsense.
        let s = [
            sample(100.0, 100.0, 10),
            sample(100.0, 115.0, 20),
            sample(90.0, 130.0, 30),
            sample(105.0, 145.0, 45),
        ];
        let points = rates(&s, all);
        assert_eq!(points.len(), 1);
        assert!(points[0].break_before);
        assert_eq!(points[0].per_min, 60);
    }

    #[test]
    fn rates_select_the_requested_status_class() {
        let mk = |at: f64, s2xx: u64, s5xx: u64| MetricsSample {
            at_secs: at,
            uptime_secs: at,
            counts: StatusCounts {
                all: s2xx + s5xx,
                s2xx,
                s4xx: 0,
                s5xx,
            },
        };
        let s = [mk(0.0, 0, 0), mk(15.0, 15, 3)];
        assert_eq!(rates(&s, |c| c.s2xx)[0].per_min, 60);
        assert_eq!(rates(&s, |c| c.s5xx)[0].per_min, 12);
        assert_eq!(rates(&s, all)[0].per_min, 72);
    }

    #[test]
    fn window_total_sums_only_usable_intervals_inside_the_window() {
        let s = [
            sample(0.0, 100.0, 0),
            sample(15.0, 115.0, 10),         // +10, inside
            sample(30.0, 130.0, 25),         // +15, inside
            sample(3_630.0, 3_730.0, 9_000), // gap -> contributes nothing
            sample(3_645.0, 3_745.0, 9_007), // +7, outside a [0,100] window
        ];
        assert_eq!(window_total(&s, 0.0, 100.0, all), 25);
        assert_eq!(
            window_total(&s, 0.0, 10_000.0, all),
            32,
            "the unobserved hour's 8975-request backlog must not be attributed \
             to the window",
        );
    }
}
