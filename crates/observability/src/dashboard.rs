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
//! Each snapshot reports counts for the single tenant passed to [`snapshot`]
//! (#344) and is consumed only by the operator dashboard. Per-tenant counts
//! are deliberately never exported to the public Prometheus `/metrics`
//! endpoint (see [`crate::metrics`]); this snapshot is a separate,
//! operator-facing surface.
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
use tracing::warn;

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

/// Bulk-export jobs for one tenant, split by lifecycle stage.
///
/// Carried by [`DashboardSnapshot::export_jobs`]; both figures come from the
/// same storage read so they are always consistent with each other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportJobCounts {
    /// Jobs a worker has claimed and is executing (`in-progress`).
    pub running: u64,
    /// Jobs accepted and waiting for a worker slot (`accepted`).
    pub queued: u64,
}

/// A snapshot of the figures the dashboard renders. Plain data — no storage or
/// FHIR types — so this crate stays dependency-light.
#[derive(Clone, Debug, Default)]
pub struct DashboardSnapshot {
    /// Default FHIR version the server serves (e.g. `"R4"`), fixed when the
    /// provider is constructed. The UI's dashboard card does not render this:
    /// it shows the request's effective version instead (#553).
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
    /// A provider may also report a zero-total type here when asked to (see
    /// [`DashboardProvider::snapshot`]'s `include_empty`, #599); the UI's own
    /// picker widens this further, unioning in the FHIR version's full type
    /// list against a separate spec-derived source this crate does not know
    /// about.
    pub available: Vec<TypeCount>,
    /// Bulk-export jobs for the tenant.
    ///
    /// `None` when the running storage backend has no bulk-export job store,
    /// the subsystem is disabled, or the count could not be read — the UI
    /// renders an explicit "unavailable" state instead of a fabricated zero.
    pub export_jobs: Option<ExportJobCounts>,
    /// Non-terminal bulk-submit (import) jobs for the tenant. `None` under the
    /// same conditions as [`Self::export_jobs`].
    pub import_jobs_active: Option<u64>,
    /// Whether part of this snapshot could not be read and was filled in with
    /// an empty series or a zero (#956).
    ///
    /// Providers degrade rather than error — a failed count query becomes a
    /// zero, a failed series query an empty chart — which otherwise makes a
    /// half-failed snapshot indistinguishable from a real one, and caches it
    /// as truth for the whole cache TTL. Setting this lets the UI say so
    /// instead of presenting the degraded figures as complete.
    pub partial: bool,
}

/// Supplies [`DashboardSnapshot`]s on demand. Implemented in `helios-rest` over
/// the live storage backend and registered via [`set_provider`] at startup.
#[async_trait]
pub trait DashboardProvider: Send + Sync {
    /// Compute a fresh snapshot over `window`, charting `types` (an empty slice
    /// asks for the provider's default selection — its top stored types).
    /// `include_empty` is the "View all resources" toggle (#599): when `true`,
    /// the implementation should also accept a requested type that is not
    /// among its stored types, charting it as a flat zero series, rather than
    /// silently dropping it. Called per dashboard page load, so implementations
    /// keep the query fan-out bounded (implementations cap the charted set) and
    /// degrade gracefully — returning zeros — rather than erroring. An
    /// implementation that degrades must set [`DashboardSnapshot::partial`], so
    /// the filled-in zeros are never read as measurements (#956).
    async fn snapshot(
        &self,
        window: DashboardWindow,
        tenant: &str,
        types: &[String],
        include_empty: bool,
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
    Arc<RwLock<std::collections::HashMap<(DashboardWindow, String, String, bool), CacheEntry>>>;

static CACHE: std::sync::LazyLock<SnapCache> = std::sync::LazyLock::new(SnapCache::default);

/// How long a computed snapshot is served without recomputing.
const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(15);
/// How long a cold request waits for the first compute before falling back to
/// placeholder figures. Long enough for row-store backends (milliseconds);
/// deliberately far below what an object-store scan can take.
const COLD_WAIT: std::time::Duration = std::time::Duration::from_millis(800);
/// How long a background compute may run before the cache stops waiting on it
/// and frees the refresh slot (#959).
///
/// Without this, a compute that never returns — a snapshot stuck behind a
/// 30s connection-pool acquire, say — leaves its cache entry pinned at
/// `computing: true` forever, and *no* later request ever spawns a refresh:
/// the entry goes permanently stale (or, if it was cold, permanently absent)
/// for the lifetime of the process.
///
/// The value is deliberately ≥ 2× [`CACHE_TTL`]. Anything shorter and a
/// slow-but-progressing compute would be abandoned on nearly every pass, so
/// the cache would never land a value while detached computes piled up behind
/// each other — replacing a stuck entry with a stampede.
///
/// Honest limitation: elapsing only releases the `computing` flag. It does
/// **not** cancel the underlying work — for the SQLite backend that work runs
/// on `spawn_blocking` and is not cancellable at all, and even for async
/// backends the detached task is free to finish and write its (late) value.
/// So this prevents permanent lockout, not wasted work.
const COMPUTE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The outcome of a dashboard read. The two ways of not having a snapshot are
/// kept apart on purpose (#956): "this build has no metrics" and "the metrics
/// are not here yet" call for different pages, and collapsing them into one
/// `None` is what let a slow window render as invented sample data.
#[derive(Clone, Debug)]
pub enum SnapshotState {
    /// A snapshot from the registered provider — fresh, or the previous one
    /// while a refresh runs. Check [`DashboardSnapshot::partial`] before
    /// presenting its figures as complete.
    Ready(DashboardSnapshot),
    /// A provider is registered, but the first compute for this key has not
    /// landed within the cold-load budget. It is still running and will fill
    /// the cache, so the same request repeated shortly usually succeeds.
    Pending,
    /// No provider is registered: this build has no live metrics at all (a
    /// server without persistence, or the standalone UI example).
    NoProvider,
}

impl SnapshotState {
    /// The snapshot, if one was available. Callers that have nothing useful to
    /// say about *why* a snapshot is missing (the rail counts, which simply
    /// omit the count) use this; the dashboard matches on the state instead.
    pub fn ready(self) -> Option<DashboardSnapshot> {
        match self {
            SnapshotState::Ready(snapshot) => Some(snapshot),
            SnapshotState::Pending | SnapshotState::NoProvider => None,
        }
    }
}

/// Fetch a dashboard snapshot over `window`, or `None` when no provider is
/// registered or the first compute is still in flight. Prefer
/// [`snapshot_state`] where the difference between those two matters.
pub async fn snapshot(
    window: DashboardWindow,
    tenant: &str,
    types: &[String],
    include_empty: bool,
) -> Option<DashboardSnapshot> {
    snapshot_state(window, tenant, types, include_empty)
        .await
        .ready()
}

/// Fetch a dashboard snapshot over `window`, reporting which of the three
/// outcomes in [`SnapshotState`] occurred.
///
/// Snapshots are cached per window and recomputed in the background: a request
/// inside [`CACHE_TTL`] returns the cached value, a stale request returns the
/// stale value immediately while one refresh task recomputes, and a cold
/// request waits up to [`COLD_WAIT`] before reporting
/// [`SnapshotState::Pending`]. This keeps page loads O(1) even on backends
/// where computing the snapshot walks storage (the S3 primary reads one object
/// per resource — minutes once conformance seeding has populated the store,
/// #326).
///
/// A background refresh that overruns [`COMPUTE_TIMEOUT`] releases its refresh
/// slot, so a single stuck compute cannot freeze the entry forever (#959).
pub async fn snapshot_state(
    window: DashboardWindow,
    tenant: &str,
    types: &[String],
    include_empty: bool,
) -> SnapshotState {
    let Some(provider) = provider() else {
        return SnapshotState::NoProvider;
    };
    snapshot_via(
        CACHE.clone(),
        provider,
        window,
        tenant,
        types,
        include_empty,
        CACHE_TTL,
        COLD_WAIT,
        COMPUTE_TIMEOUT,
    )
    .await
}

/// [`snapshot_state`] with the cache, provider, and timings injected, so the
/// serve paths are testable against private caches and fast clocks. Never
/// returns [`SnapshotState::NoProvider`]: it is only reached with a provider
/// in hand.
#[allow(clippy::too_many_arguments)]
async fn snapshot_via(
    cache: SnapCache,
    provider: Arc<dyn DashboardProvider>,
    window: DashboardWindow,
    tenant: &str,
    types: &[String],
    include_empty: bool,
    ttl: std::time::Duration,
    cold_wait: std::time::Duration,
    compute_timeout: std::time::Duration,
) -> SnapshotState {
    // The charted set (and the "View all resources" toggle, #599) is part of
    // the cache identity: two selections — or the same selection with the
    // toggle flipped — are two different snapshots. Selections are short
    // (providers cap them), so the joined key stays small and the cache stays
    // bounded by user behavior.
    let key = (window, tenant.to_string(), types.join(","), include_empty);
    // One pass under the lock: serve fresh hits, note staleness, and claim the
    // compute slot if nobody holds it.
    let (cached, spawn_compute) = {
        // A poisoned cache lock means a compute panicked mid-write: nothing is
        // readable and nothing can be spawned, which is exactly "not here yet".
        let Ok(mut guard) = cache.write() else {
            return SnapshotState::Pending;
        };
        let entry = guard.entry(key.clone()).or_insert(CacheEntry {
            value: None,
            computing: false,
        });
        if let Some((at, value)) = &entry.value
            && at.elapsed() < ttl
        {
            return SnapshotState::Ready(value.clone());
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
            // Time-boxed: a compute that never returns must not pin
            // `computing: true` forever, which would stop every later request
            // from ever spawning a refresh and freeze the entry permanently
            // (#959). Note this releases the *slot*, not the work: the inner
            // future is dropped here, but a `spawn_blocking` query behind it
            // (the SQLite backend) keeps running to completion regardless.
            let computed = tokio::time::timeout(
                compute_timeout,
                provider.snapshot(window, &tenant, &types, include_empty),
            )
            .await;
            match computed {
                Ok(value) => {
                    if let Ok(mut guard) = cache.write()
                        && let Some(entry) = guard.get_mut(&key)
                    {
                        entry.value = Some((std::time::Instant::now(), value));
                        entry.computing = false;
                    }
                }
                Err(_elapsed) => {
                    warn!(
                        window = window.as_str(),
                        tenant = %tenant,
                        timeout_ms = compute_timeout.as_millis() as u64,
                        "dashboard snapshot compute timed out; releasing the refresh slot"
                    );
                    // Clear the flag but write no value: the entry keeps
                    // whatever (stale) snapshot it had, and the next request
                    // is free to retry.
                    if let Ok(mut guard) = cache.write()
                        && let Some(entry) = guard.get_mut(&key)
                    {
                        entry.computing = false;
                    }
                }
            }
        });
    }

    // Stale beats absent: serve it now, the refresh lands for the next load.
    if let Some((_, value)) = cached {
        return SnapshotState::Ready(value);
    }

    // Cold: give a fast backend a beat to fill the cache before degrading.
    let deadline = std::time::Instant::now() + cold_wait;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Ok(guard) = cache.read()
            && let Some(value) = guard.get(&key).and_then(|e| e.value.as_ref())
        {
            return SnapshotState::Ready(value.1.clone());
        }
    }
    // Not "no data" — the compute is still running and will land in the cache.
    // Every window switch takes this path (the window is part of the key), so
    // callers must render waiting, not absence (#956).
    SnapshotState::Pending
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    /// Compute budget for the tests that are not about the timeout: far longer
    /// than any fake provider's delay, so it never fires.
    const TEST_COMPUTE_TIMEOUT: Duration = Duration::from_secs(30);

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
            _include_empty: bool,
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
            false,
            ttl,
            cold,
            TEST_COMPUTE_TIMEOUT,
        )
        .await
        .ready()
        .expect("cold load fills within the wait");
        assert_eq!(first.total_resources, 1);

        let second = snapshot_via(
            cache,
            provider.clone(),
            DashboardWindow::LastHour,
            "default",
            &[],
            false,
            ttl,
            cold,
            TEST_COMPUTE_TIMEOUT,
        )
        .await
        .ready()
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
            false,
            ttl,
            cold,
            TEST_COMPUTE_TIMEOUT,
        )
        .await
        .ready()
        .expect("cold load");
        assert_eq!(first.total_resources, 1);

        let stale = snapshot_via(
            cache.clone(),
            provider.clone(),
            DashboardWindow::LastDay,
            "default",
            &[],
            false,
            ttl,
            cold,
            TEST_COMPUTE_TIMEOUT,
        )
        .await
        .ready()
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
            false,
            ttl,
            cold,
            TEST_COMPUTE_TIMEOUT,
        )
        .await
        .ready()
        .expect("refreshed value");
        assert!(
            refreshed.total_resources >= 2,
            "got {}",
            refreshed.total_resources
        );
    }

    /// #956: a cold load past the wait is *pending*, not absent. Every window
    /// switch is a cold key, so this is the path the dashboard took when it
    /// swapped in invented sample data mid-import; the state it reports has to
    /// keep "still computing" apart from "this build has no provider".
    #[tokio::test]
    async fn slow_cold_compute_reports_pending_then_lands() {
        let cache = SnapCache::default();
        let provider = Counting::new(Duration::from_millis(400));
        let ttl = Duration::from_secs(60);
        let cold = Duration::from_millis(120);

        let first = snapshot_via(
            cache.clone(),
            provider.clone(),
            DashboardWindow::LastMonth,
            "default",
            &[],
            false,
            ttl,
            cold,
            TEST_COMPUTE_TIMEOUT,
        )
        .await;
        assert!(
            matches!(first, SnapshotState::Pending),
            "cold load past the wait is pending, not missing: {first:?}"
        );

        tokio::time::sleep(Duration::from_millis(600)).await;
        let second = snapshot_via(
            cache,
            provider.clone(),
            DashboardWindow::LastMonth,
            "default",
            &[],
            false,
            ttl,
            cold,
            TEST_COMPUTE_TIMEOUT,
        )
        .await
        .ready()
        .expect("the detached compute landed");
        assert_eq!(second.total_resources, 1);
        assert_eq!(
            provider.hits.load(Ordering::SeqCst),
            1,
            "single-flight: no compute stampede"
        );
    }

    /// Hangs on its first compute — past any timeout a test would use — and
    /// answers instantly afterwards. That asymmetry is what makes the timeout
    /// observable: only a *second* spawned compute can land a value, and a
    /// second compute is only spawned if the first one's refresh slot was
    /// released.
    struct HangsOnce {
        hits: AtomicUsize,
        first_delay: Duration,
    }

    impl HangsOnce {
        fn new(first_delay: Duration) -> Arc<Self> {
            Arc::new(HangsOnce {
                hits: AtomicUsize::new(0),
                first_delay,
            })
        }
    }

    #[async_trait]
    impl DashboardProvider for HangsOnce {
        async fn snapshot(
            &self,
            window: DashboardWindow,
            _tenant: &str,
            _types: &[String],
            _include_empty: bool,
        ) -> DashboardSnapshot {
            let hit = self.hits.fetch_add(1, Ordering::SeqCst) + 1;
            if hit == 1 {
                tokio::time::sleep(self.first_delay).await;
            }
            DashboardSnapshot {
                total_resources: hit as u64,
                window,
                ..DashboardSnapshot::default()
            }
        }
    }

    /// A compute that overruns `compute_timeout` must release its refresh slot
    /// (#959). Before the time-box, `computing` stayed `true` forever and the
    /// entry could never be refreshed again for the life of the process — this
    /// test would hang at [`SnapshotState::Pending`] on the second call.
    #[tokio::test]
    async fn timed_out_compute_releases_the_slot_so_a_later_request_retries() {
        let cache = SnapCache::default();
        // Effectively never returns within this test.
        let provider = HangsOnce::new(Duration::from_secs(30));
        let ttl = Duration::from_secs(60);
        let cold = Duration::from_millis(80);
        let compute_timeout = Duration::from_millis(150);

        let first = snapshot_via(
            cache.clone(),
            provider.clone(),
            DashboardWindow::LastMonth,
            "default",
            &[],
            false,
            ttl,
            cold,
            compute_timeout,
        )
        .await;
        assert!(
            matches!(first, SnapshotState::Pending),
            "the stuck compute cannot fill the cache, and a registered provider \
             that is merely slow is pending, never absent (#956)"
        );
        assert_eq!(provider.hits.load(Ordering::SeqCst), 1);

        // Let the time-box elapse and the spawned task clear `computing`.
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            if cache
                .read()
                .ok()
                .and_then(|g| g.values().next().map(|e| !e.computing))
                .unwrap_or(false)
            {
                break;
            }
        }
        {
            let guard = cache.read().expect("cache readable");
            let entry = guard.values().next().expect("the entry exists");
            assert!(
                !entry.computing,
                "the elapsed compute must free the refresh slot"
            );
            assert!(
                entry.value.is_none(),
                "a timed-out compute writes no value, it only frees the slot"
            );
        }

        // With the slot free, the next request spawns a fresh compute — which
        // is instant this time — and a value finally lands.
        let second = snapshot_via(
            cache,
            provider.clone(),
            DashboardWindow::LastMonth,
            "default",
            &[],
            false,
            ttl,
            cold,
            compute_timeout,
        )
        .await
        .ready()
        .expect("a new compute was spawned and landed");
        assert_eq!(second.total_resources, 2);
        assert_eq!(
            provider.hits.load(Ordering::SeqCst),
            2,
            "exactly one retry, not a stampede"
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
            _include_empty: bool,
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
                export_jobs: None,
                import_jobs_active: None,
                partial: false,
            }
        }
    }

    #[tokio::test]
    async fn registered_provider_snapshot_round_trips() {
        set_provider(Arc::new(Fixed));

        let snap = snapshot(DashboardWindow::LastHour, "default", &[], false)
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

        // The same read through the three-state entry point reports Ready —
        // the state the UI needs to tell a real snapshot from a slow one.
        let state = snapshot_state(DashboardWindow::LastHour, "default", &[], false).await;
        assert!(matches!(state, SnapshotState::Ready(_)), "{state:?}");
    }

    /// The "View all resources" toggle (#599) is part of the cache identity:
    /// the same window/tenant/types with `include_empty` flipped must not
    /// reuse a value computed under the other setting.
    #[tokio::test]
    async fn include_empty_is_part_of_the_cache_key() {
        let cache = SnapCache::default();
        let provider = Counting::new(Duration::ZERO);
        let ttl = Duration::from_secs(60);
        let cold = Duration::from_millis(800);

        let without = snapshot_via(
            cache.clone(),
            provider.clone(),
            DashboardWindow::LastHour,
            "default",
            &[],
            false,
            ttl,
            cold,
            TEST_COMPUTE_TIMEOUT,
        )
        .await
        .ready()
        .expect("cold load, flag off");
        assert_eq!(without.total_resources, 1);

        let with = snapshot_via(
            cache,
            provider.clone(),
            DashboardWindow::LastHour,
            "default",
            &[],
            true,
            ttl,
            cold,
            TEST_COMPUTE_TIMEOUT,
        )
        .await
        .ready()
        .expect("cold load, flag on — a separate cache entry");
        assert_eq!(
            with.total_resources, 2,
            "not served from the flag-off entry"
        );
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
