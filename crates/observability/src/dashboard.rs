//! Process-global dashboard data provider.
//!
//! The web UI's landing dashboard renders a "FHIR resources over time" chart and
//! a few headline totals. Those figures come from the storage backend, which
//! lives behind `helios-rest`'s `AppState` — a layer this crate deliberately does
//! not depend on. To keep `helios-observability` storage-agnostic (and the UI
//! crate thin), the server registers a [`DashboardProvider`] here at startup, and
//! the UI reads the latest snapshot through [`snapshot`] without knowing anything
//! about persistence.
//!
//! This mirrors the process-global pattern already used by [`crate::uptime`] and
//! [`crate::metrics`]: install once at startup, read cheaply per request.
//!
//! ## Scope and trust
//!
//! The registered provider reports counts for a **single tenant** (the server's
//! default tenant) and is consumed only by the operator dashboard. Per-tenant
//! counts are deliberately never exported to the public Prometheus `/metrics`
//! endpoint (see [`crate::metrics`]); this snapshot is a separate, operator-facing
//! surface.
//!
//! ## Time resolution
//!
//! The chart is sampled over a [`DashboardWindow`], which pairs a span with the
//! bucket width used to sample it (1h/1min, 24h/30min, 30d/1day). Span and bucket
//! are coupled rather than independent, and the underlying series is built from
//! the immutable history log — see [`DashboardWindow`] for why both of those
//! matter.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};

/// A window the dashboard chart can be viewed over, pairing a span with the
/// bucket width that samples it.
///
/// The two are deliberately coupled: bucket width is not a free "precision"
/// knob, because a fine bucket over a long span produces a point count no chart
/// (or response body) can carry — a 30-day span at one-minute buckets is 43 200
/// points *per resource type*. Each variant below is therefore chosen to land in
/// the 30–60 point range, so every zoom level stays legible and cheap.
///
/// The series behind these is built from the immutable history log
/// (`count_deltas_by_bucket`), not from the current rows' `last_updated`, so
/// buckets do not shift when a resource is edited. That is what makes the
/// sub-day windows meaningful rather than merely finer-grained.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DashboardWindow {
    /// Last hour, in one-minute buckets (60 points).
    LastHour,
    /// Last 24 hours, in half-hour buckets (48 points).
    LastDay,
    /// Last 30 days, in daily buckets (30 points). The default view.
    #[default]
    LastMonth,
}

impl DashboardWindow {
    /// All windows, in the order the UI offers them (finest first).
    pub const ALL: [DashboardWindow; 3] = [Self::LastHour, Self::LastDay, Self::LastMonth];

    /// Stable slug used in the `?window=` query parameter and as the selector
    /// label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LastHour => "1h",
            Self::LastDay => "24h",
            Self::LastMonth => "30d",
        }
    }

    /// Parses a `?window=` slug, returning `None` for anything unrecognised so
    /// callers can fall back to the default rather than erroring.
    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.as_str() == slug)
    }

    /// Total span covered by the window, in seconds.
    pub fn span_seconds(self) -> i64 {
        self.bucket_seconds() * self.points() as i64
    }

    /// Width of one bucket, in seconds.
    pub fn bucket_seconds(self) -> i64 {
        match self {
            Self::LastHour => 60,
            Self::LastDay => 1_800,
            Self::LastMonth => 86_400,
        }
    }

    /// Number of buckets plotted across the span.
    pub fn points(self) -> usize {
        match self {
            Self::LastHour => 60,
            Self::LastDay => 48,
            Self::LastMonth => 30,
        }
    }

    /// Whether buckets are finer than a day, which is what decides between a
    /// clock-time and a calendar-date axis label.
    pub fn is_intraday(self) -> bool {
        self.bucket_seconds() < 86_400
    }
}

/// One bucket of a single resource type's cumulative growth curve.
#[derive(Clone, Debug)]
pub struct DashboardPoint {
    /// Inclusive start of the bucket (UTC), aligned to the Unix epoch.
    pub bucket_start: DateTime<Utc>,
    /// Net stored-resource change recorded in this bucket: creations minus
    /// deletions. May be negative.
    pub delta: i64,
    /// Running total through the end of this bucket (converges to the series
    /// `total` on the final point).
    pub cumulative: u64,
}

/// One resource type's series for the "resources over time" chart.
#[derive(Clone, Debug)]
pub struct DashboardSeries {
    /// FHIR resource type name (e.g. `"Observation"`).
    pub resource_type: String,
    /// Current stored total for this type — the final cumulative value.
    pub total: u64,
    /// Dense daily points, oldest first.
    pub points: Vec<DashboardPoint>,
}

/// A resource type the tenant actually stores, with its current total —
/// what the chart's type picker offers (#555).
#[derive(Clone, Debug)]
pub struct TypeCount {
    pub resource_type: String,
    pub total: u64,
}

/// A snapshot of the figures the dashboard renders. Plain data — no storage or
/// FHIR types — so this crate stays dependency-light.
#[derive(Clone, Debug, Default)]
pub struct DashboardSnapshot {
    /// Default FHIR version the server serves (e.g. `"R4"`).
    pub fhir_version: String,
    /// Total non-deleted resources across all types for the default tenant.
    pub total_resources: u64,
    /// Number of distinct resource types with at least one stored resource.
    pub distinct_types: usize,
    /// The window the `series` were sampled over.
    pub window: DashboardWindow,
    /// Per-type series for the charted resource types, in display order.
    pub series: Vec<DashboardSeries>,
    /// Every type with at least one stored resource, largest first — the
    /// picker's option list, so a tenant charts what it actually has (#555).
    pub available: Vec<TypeCount>,
}

/// Supplies [`DashboardSnapshot`]s on demand. Implemented in `helios-rest` over
/// the live storage backend and registered via [`set_provider`] at startup.
#[async_trait]
pub trait DashboardProvider: Send + Sync {
    /// Compute a fresh snapshot over `window`, charting `types` (an empty slice
    /// asks for the provider's default selection — its top stored types).
    /// Called per dashboard page load, so implementations keep the query
    /// fan-out bounded (implementations cap the charted set) and degrade
    /// gracefully — returning zeros — rather than erroring.
    async fn snapshot(
        &self,
        window: DashboardWindow,
        tenant: &str,
        types: &[String],
    ) -> DashboardSnapshot;
}

static PROVIDER: RwLock<Option<Arc<dyn DashboardProvider>>> = RwLock::new(None);

/// Register (or replace) the process-global dashboard provider. Called once from
/// the server's app builder; the most recent registration wins, so a later real
/// server never reads a provider left behind by an earlier one.
pub fn set_provider(provider: Arc<dyn DashboardProvider>) {
    if let Ok(mut guard) = PROVIDER.write() {
        *guard = Some(provider);
    }
}

/// The registered provider, if any.
fn provider() -> Option<Arc<dyn DashboardProvider>> {
    PROVIDER.read().ok().and_then(|guard| guard.clone())
}

/// One cached window: the last computed snapshot (if any) and whether a
/// compute task is currently in flight for it.
struct CacheEntry {
    value: Option<(std::time::Instant, DashboardSnapshot)>,
    computing: bool,
}

type SnapCache =
    Arc<RwLock<std::collections::HashMap<(DashboardWindow, String, String), CacheEntry>>>;

static CACHE: std::sync::LazyLock<SnapCache> = std::sync::LazyLock::new(SnapCache::default);

/// How long a computed snapshot is served without recomputing.
const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(15);
/// How long a cold request waits for the first compute before falling back to
/// placeholder figures. Long enough for row-store backends (milliseconds);
/// deliberately far below what an object-store scan can take.
const COLD_WAIT: std::time::Duration = std::time::Duration::from_millis(800);

/// Fetch a dashboard snapshot over `window`, or `None` when no provider is
/// registered (e.g. a server build without persistence, or the standalone UI
/// example) or the first compute is still in flight. The UI falls back to
/// placeholder figures in that case.
///
/// Snapshots are cached per window and recomputed in the background: a request
/// inside [`CACHE_TTL`] returns the cached value, a stale request returns the
/// stale value immediately while one refresh task recomputes, and a cold
/// request waits up to [`COLD_WAIT`] before degrading to `None`. This keeps
/// page loads O(1) even on backends where computing the snapshot walks storage
/// (the S3 primary reads one object per resource — minutes once conformance
/// seeding has populated the store, #326).
pub async fn snapshot(
    window: DashboardWindow,
    tenant: &str,
    types: &[String],
) -> Option<DashboardSnapshot> {
    let provider = provider()?;
    snapshot_via(
        CACHE.clone(),
        provider,
        window,
        tenant,
        types,
        CACHE_TTL,
        COLD_WAIT,
    )
    .await
}

/// [`snapshot`] with the cache, provider, and timings injected, so the serve
/// paths are testable against private caches and fast clocks.
async fn snapshot_via(
    cache: SnapCache,
    provider: Arc<dyn DashboardProvider>,
    window: DashboardWindow,
    tenant: &str,
    types: &[String],
    ttl: std::time::Duration,
    cold_wait: std::time::Duration,
) -> Option<DashboardSnapshot> {
    // The charted set is part of the cache identity: two selections are two
    // different snapshots. Selections are short (providers cap them), so the
    // joined key stays small and the cache stays bounded by user behavior.
    let key = (window, tenant.to_string(), types.join(","));
    // One pass under the lock: serve fresh hits, note staleness, and claim the
    // compute slot if nobody holds it.
    let (cached, spawn_compute) = {
        let mut guard = cache.write().ok()?;
        let entry = guard.entry(key.clone()).or_insert(CacheEntry {
            value: None,
            computing: false,
        });
        if let Some((at, value)) = &entry.value
            && at.elapsed() < ttl
        {
            return Some(value.clone());
        }
        let spawn_compute = !entry.computing;
        if spawn_compute {
            entry.computing = true;
        }
        (entry.value.clone(), spawn_compute)
    };

    if spawn_compute {
        let cache = cache.clone();
        let key = key.clone();
        let tenant = tenant.to_string();
        let types = types.to_vec();
        tokio::spawn(async move {
            let value = provider.snapshot(window, &tenant, &types).await;
            if let Ok(mut guard) = cache.write()
                && let Some(entry) = guard.get_mut(&key)
            {
                entry.value = Some((std::time::Instant::now(), value));
                entry.computing = false;
            }
        });
    }

    // Stale beats absent: serve it now, the refresh lands for the next load.
    if let Some((_, value)) = cached {
        return Some(value);
    }

    // Cold: give a fast backend a beat to fill the cache before degrading.
    let deadline = std::time::Instant::now() + cold_wait;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Ok(guard) = cache.read()
            && let Some(value) = guard.get(&key).and_then(|e| e.value.as_ref())
        {
            return Some(value.1.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    /// Counts computes and answers with that count as `total_resources`, after
    /// an optional delay — enough to tell cached from recomputed values apart.
    struct Counting {
        hits: AtomicUsize,
        delay: Duration,
    }

    impl Counting {
        fn new(delay: Duration) -> Arc<Self> {
            Arc::new(Counting {
                hits: AtomicUsize::new(0),
                delay,
            })
        }
    }

    #[async_trait]
    impl DashboardProvider for Counting {
        async fn snapshot(
            &self,
            window: DashboardWindow,
            _tenant: &str,
            _types: &[String],
        ) -> DashboardSnapshot {
            let hit = self.hits.fetch_add(1, Ordering::SeqCst) + 1;
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            DashboardSnapshot {
                total_resources: hit as u64,
                window,
                ..DashboardSnapshot::default()
            }
        }
    }

    #[tokio::test]
    async fn cold_load_fills_the_cache_and_fresh_hits_reuse_it() {
        let cache = SnapCache::default();
        let provider = Counting::new(Duration::ZERO);
        let ttl = Duration::from_secs(60);
        let cold = Duration::from_millis(800);

        let first = snapshot_via(
            cache.clone(),
            provider.clone(),
            DashboardWindow::LastHour,
            "default",
            &[],
            ttl,
            cold,
        )
        .await
        .expect("cold load fills within the wait");
        assert_eq!(first.total_resources, 1);

        let second = snapshot_via(
            cache,
            provider.clone(),
            DashboardWindow::LastHour,
            "default",
            &[],
            ttl,
            cold,
        )
        .await
        .expect("fresh hit");
        assert_eq!(second.total_resources, 1, "served from cache");
        assert_eq!(
            provider.hits.load(Ordering::SeqCst),
            1,
            "no recompute inside the TTL"
        );
    }

    #[tokio::test]
    async fn stale_hits_serve_immediately_and_refresh_in_the_background() {
        let cache = SnapCache::default();
        let provider = Counting::new(Duration::ZERO);
        let ttl = Duration::ZERO; // everything is instantly stale
        let cold = Duration::from_millis(800);

        let first = snapshot_via(
            cache.clone(),
            provider.clone(),
            DashboardWindow::LastDay,
            "default",
            &[],
            ttl,
            cold,
        )
        .await
        .expect("cold load");
        assert_eq!(first.total_resources, 1);

        let stale = snapshot_via(
            cache.clone(),
            provider.clone(),
            DashboardWindow::LastDay,
            "default",
            &[],
            ttl,
            cold,
        )
        .await
        .expect("stale value served without waiting");
        assert_eq!(stale.total_resources, 1, "the old value, not the refresh");

        // The background refresh lands; a later hit sees the new compute.
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            if provider.hits.load(Ordering::SeqCst) >= 2 {
                break;
            }
        }
        let refreshed = snapshot_via(
            cache,
            provider.clone(),
            DashboardWindow::LastDay,
            "default",
            &[],
            ttl,
            cold,
        )
        .await
        .expect("refreshed value");
        assert!(
            refreshed.total_resources >= 2,
            "got {}",
            refreshed.total_resources
        );
    }

    #[tokio::test]
    async fn slow_cold_compute_degrades_to_none_then_lands() {
        let cache = SnapCache::default();
        let provider = Counting::new(Duration::from_millis(400));
        let ttl = Duration::from_secs(60);
        let cold = Duration::from_millis(120);

        let first = snapshot_via(
            cache.clone(),
            provider.clone(),
            DashboardWindow::LastMonth,
            "default",
            ttl,
            cold,
        )
        .await;
        assert!(
            first.is_none(),
            "cold load past the wait degrades to placeholder"
        );

        tokio::time::sleep(Duration::from_millis(600)).await;
        let second = snapshot_via(
            cache,
            provider.clone(),
            DashboardWindow::LastMonth,
            "default",
            ttl,
            cold,
        )
        .await
        .expect("the detached compute landed");
        assert_eq!(second.total_resources, 1);
        assert_eq!(
            provider.hits.load(Ordering::SeqCst),
            1,
            "single-flight: no compute stampede"
        );
    }

    /// Echoes back the window it was asked for, so the test can assert the
    /// requested window reaches the provider.
    struct Fixed;

    #[async_trait]
    impl DashboardProvider for Fixed {
        async fn snapshot(
            &self,
            window: DashboardWindow,
            _tenant: &str,
            _types: &[String],
        ) -> DashboardSnapshot {
            DashboardSnapshot {
                fhir_version: "R4".to_string(),
                total_resources: 42,
                distinct_types: 3,
                window,
                series: vec![DashboardSeries {
                    resource_type: "Patient".to_string(),
                    total: 7,
                    points: vec![DashboardPoint {
                        bucket_start: DateTime::from_timestamp(1_752_451_200, 0).unwrap(),
                        delta: 7,
                        cumulative: 7,
                    }],
                }],
                available: Vec::new(),
            }
        }
    }

    #[tokio::test]
    async fn registered_provider_snapshot_round_trips() {
        set_provider(Arc::new(Fixed));

        let snap = snapshot(DashboardWindow::LastHour, "default", &[])
            .await
            .expect("provider registered");
        assert_eq!(snap.total_resources, 42);
        assert_eq!(snap.distinct_types, 3);
        // The requested window reaches the provider and is echoed on the snapshot,
        // so the UI can render its selector from the snapshot alone.
        assert_eq!(snap.window, DashboardWindow::LastHour);
        assert_eq!(snap.series.len(), 1);
        assert_eq!(snap.series[0].resource_type, "Patient");
        assert_eq!(snap.series[0].points.last().unwrap().cumulative, 7);
    }

    /// Every window stays inside a legible point budget, and its span is exactly
    /// the buckets it plots — the invariant the chart's x-axis relies on.
    #[test]
    fn windows_pair_span_with_a_bounded_point_count() {
        for window in DashboardWindow::ALL {
            assert!(
                (30..=60).contains(&window.points()),
                "{} plots {} points, outside the legible range",
                window.as_str(),
                window.points()
            );
            assert_eq!(
                window.span_seconds(),
                window.bucket_seconds() * window.points() as i64
            );
            assert_eq!(DashboardWindow::from_slug(window.as_str()), Some(window));
        }

        assert_eq!(DashboardWindow::LastHour.span_seconds(), 3_600);
        assert_eq!(DashboardWindow::LastDay.span_seconds(), 86_400);
        assert_eq!(DashboardWindow::LastMonth.span_seconds(), 30 * 86_400);

        assert!(DashboardWindow::LastHour.is_intraday());
        assert!(DashboardWindow::LastDay.is_intraday());
        assert!(!DashboardWindow::LastMonth.is_intraday());

        // Unknown slugs fall back rather than erroring.
        assert_eq!(DashboardWindow::from_slug("7d"), None);
        assert_eq!(DashboardWindow::default(), DashboardWindow::LastMonth);
    }
}
