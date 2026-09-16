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
//! ## Where the figures come from
//!
//! The provider answers in constant time from in-memory counters
//! ([`crate::dashboard_counters`]) that a background task reconciles against
//! storage; no page load reads storage (#1078). Each snapshot says how far its
//! figures can be trusted through [`DashboardSnapshot::figures`]. The cache here
//! is therefore thin: it deduplicates concurrent reads of one key for a couple
//! of seconds and bounds a provider that stalls, nothing more.
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

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

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

/// A tenant's search-index rebuild (`$reindex`) that the UI must mention
/// (#1065, #1125).
///
/// Carried by [`DashboardSnapshot::reindex_active`]. While a rebuild runs,
/// stored resources stay readable by id but searches can miss them, so the UI
/// says so rather than letting an empty result read as lost data. When the
/// tenant's most recent rebuild ended without indexing everything, the same
/// holds indefinitely, so the UI keeps saying so — and where to find which
/// resources — instead of dropping the line the moment the job stops running.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ReindexActivity {
    /// Rebuilds queued or in progress for the tenant.
    Running {
        /// Rebuild jobs running (queued or in progress).
        jobs: u64,
        /// Resources processed so far, across those jobs.
        processed: u64,
        /// Resources to process, across those jobs; `0` while still being
        /// counted.
        total: u64,
    },
    /// The tenant's most recent rebuild ended, but left resources unindexed:
    /// it failed outright, or completed with per-resource errors.
    Failed {
        /// The job to look up with `$reindex-status/{job_id}`.
        job_id: String,
        /// Resources the job reported as not indexed; `0` when the job failed
        /// as a whole before attributing any error to a resource.
        errors: u64,
    },
}

impl ReindexActivity {
    /// Whether a rebuild is still running, so figures keep moving.
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }

    /// Whole percent done of a running rebuild; `None` while its total is
    /// still being counted, and for a rebuild that has ended.
    pub fn percent(&self) -> Option<u64> {
        match *self {
            Self::Running {
                processed, total, ..
            } => (total > 0).then(|| processed.min(total) * 100 / total),
            Self::Failed { .. } => None,
        }
    }
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
    /// Search-index rebuilds running for the tenant (#1065), or its most
    /// recent rebuild when that one left resources unindexed (#1125). `None`
    /// when none is running and the last one indexed everything, or the
    /// deployment has no `$reindex` operation: the rebuild banner is then
    /// simply absent, never a fabricated "0%".
    pub reindex_active: Option<ReindexActivity>,
    /// Where the figures come from and how far they can be trusted (#1078).
    /// Only [`Figures::Exact`] and [`Figures::Approximate`] carry figures; the
    /// other variants leave totals, `available` and `series` empty, and those
    /// empties are never measurements (#956).
    pub figures: Figures,
}

/// Where a [`DashboardSnapshot`]'s figures come from, and how far they can be
/// trusted (#956, #1078).
///
/// One value instead of independent flags, so a snapshot cannot claim to be
/// both waiting and approximate, or carry an "as of" time for figures nobody
/// read. Defaults to [`Figures::Pending`]: a snapshot built without saying
/// otherwise renders as waiting, never as zeros.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Figures {
    /// Read from counters that matched storage at the last reconcile, with no
    /// change to the tenant's data seen since.
    Exact {
        /// When the figures were read — the UI's "as of" time.
        read_at: DateTime<Utc>,
    },
    /// Measured, but storage changed since the counters last matched it —
    /// writes recorded by this process or detected elsewhere — or a charted
    /// window's history is not loaded yet.
    Approximate {
        /// When the figures were read — the UI's "as of" time.
        read_at: DateTime<Utc>,
        /// When the counters last matched storage.
        reconciled_at: DateTime<Utc>,
    },
    /// Not known yet: the tenant is being seeded in the background.
    #[default]
    Pending,
    /// The storage backend cannot count resources, so there are no figures —
    /// which is not the same as an empty tenant.
    Unsupported,
}

impl Figures {
    /// The "as of" time, for the variants that carry figures.
    pub fn read_at(&self) -> Option<DateTime<Utc>> {
        match self {
            Figures::Exact { read_at } | Figures::Approximate { read_at, .. } => Some(*read_at),
            Figures::Pending | Figures::Unsupported => None,
        }
    }

    /// Whether the snapshot carries figures at all.
    pub fn is_known(&self) -> bool {
        matches!(self, Figures::Exact { .. } | Figures::Approximate { .. })
    }

    /// Whether the figures are measured but not an exact storage match.
    pub fn is_approximate(&self) -> bool {
        matches!(self, Figures::Approximate { .. })
    }
}

/// Supplies [`DashboardSnapshot`]s on demand. Implemented in `helios-rest` over
/// the server's live resource counters and registered via [`set_provider`] at
/// startup.
#[async_trait]
pub trait DashboardProvider: Send + Sync {
    /// Build a snapshot over `window`, charting `types` (an empty slice asks
    /// for the provider's default selection — its top stored types).
    /// `include_empty` is the "View all resources" toggle (#599): when `true`,
    /// the implementation should also accept a requested type that is not
    /// among its stored types, charting it as a flat zero series, rather than
    /// silently dropping it.
    ///
    /// Called per dashboard page load and on every dashboard poll, so an
    /// implementation answers in constant time from what it already holds in
    /// memory and never reads storage here (#1078). It reports how far the
    /// figures can be trusted in [`DashboardSnapshot::figures`], stamping the
    /// moment it read them, and uses [`Figures::Pending`] or
    /// [`Figures::Unsupported`] rather than filling in zeros that would read as
    /// measurements (#956). The cache passes the snapshot through unchanged.
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

/// Cache identity: window, tenant, the joined charted types, and the "View all
/// resources" toggle (#599).
type CacheKey = (DashboardWindow, String, String, bool);

/// One cached key: the last snapshot written (with when it was written) and
/// whether a compute holds the refresh slot.
#[derive(Default)]
struct CacheEntry {
    value: Option<(Instant, DashboardSnapshot)>,
    computing: bool,
}

type SnapCache = Arc<RwLock<HashMap<CacheKey, CacheEntry>>>;

static CACHE: std::sync::LazyLock<SnapCache> = std::sync::LazyLock::new(SnapCache::default);

/// How long a written snapshot is served without recomputing.
///
/// The dashboard polls every few seconds so the operator can watch figures
/// move; the provider answers in constant time, so this only has to collapse
/// bursts of identical reads (several tabs, the rail counts) into one compute.
const TTL: Duration = Duration::from_secs(2);
/// How long a request that did not find a fresh value waits for a compute to
/// write one before serving the stale value, or [`SnapshotState::Pending`] when
/// there is none. A constant-time provider lands well inside it.
const WAIT: Duration = Duration::from_millis(500);
/// How long a compute may run before it is aborted and its refresh slot freed,
/// so a provider that never returns cannot pin a key (#959).
const COMPUTE_TIMEOUT: Duration = Duration::from_secs(10);
/// How often a waiting request re-checks the cache.
const POLL: Duration = Duration::from_millis(20);

/// The cache's timings, injected so tests run on fast clocks.
#[derive(Clone, Copy, Debug)]
struct Timings {
    /// See [`TTL`].
    ttl: Duration,
    /// See [`WAIT`].
    wait: Duration,
    /// See [`COMPUTE_TIMEOUT`].
    compute_timeout: Duration,
}

const TIMINGS: Timings = Timings {
    ttl: TTL,
    wait: WAIT,
    compute_timeout: COMPUTE_TIMEOUT,
};

/// The outcome of a dashboard read. The two ways of not having a snapshot are
/// kept apart on purpose (#956): "this build has no metrics" and "the metrics
/// are not here yet" call for different pages, and collapsing them into one
/// `None` is what let a slow window render as invented sample data.
#[derive(Clone, Debug)]
pub enum SnapshotState {
    /// A snapshot from the registered provider: fresh, freshly recomputed, or —
    /// when a refresh does not land within the wait — the previous one. Its
    /// [`DashboardSnapshot::figures`] says whether it carries figures at all,
    /// how far they can be trusted, and when they were read.
    Ready(DashboardSnapshot),
    /// A provider is registered, but its compute did not answer within the
    /// wait and nothing was cached for this key yet — a cold key under a
    /// stalled provider. The compute keeps running (up to its timeout) and
    /// fills the cache, so the same request repeated shortly usually succeeds.
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
/// registered or nothing has been computed for the key yet. Prefer
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
/// Snapshots are cached per (window, tenant, types, "View all resources") key.
/// A value written within [`TTL`] is served as is. Otherwise the request starts
/// the key's compute unless one is already running (single-flight), then waits
/// up to [`WAIT`] for a value written after the one it saw: it gets that value,
/// else the stale value (stale-while-revalidate), else
/// [`SnapshotState::Pending`]. A compute that outlasts [`COMPUTE_TIMEOUT`] is
/// aborted and frees the slot for the next request (#959).
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
        &CACHE,
        provider,
        window,
        tenant,
        types,
        include_empty,
        TIMINGS,
    )
    .await
}

/// [`snapshot_state`] with the cache, provider, and timings injected, so the
/// serve paths are testable against private caches and fast clocks. Never
/// returns [`SnapshotState::NoProvider`]: it is only reached with a provider
/// in hand.
async fn snapshot_via(
    cache: &SnapCache,
    provider: Arc<dyn DashboardProvider>,
    window: DashboardWindow,
    tenant: &str,
    types: &[String],
    include_empty: bool,
    timings: Timings,
) -> SnapshotState {
    // The charted set (and the "View all resources" toggle, #599) is part of
    // the cache identity: two selections — or the same selection with the
    // toggle flipped — are two different snapshots.
    let key: CacheKey = (window, tenant.to_string(), types.join(","), include_empty);
    // One pass under the lock: serve a fresh value, or remember what is there
    // and claim the compute slot if nobody holds it.
    let (seen, claimed) = {
        // A poisoned cache lock means a writer panicked mid-update: nothing is
        // readable and nothing can be spawned, which is "not here yet".
        let Ok(mut guard) = cache.write() else {
            return SnapshotState::Pending;
        };
        let entry = guard.entry(key.clone()).or_default();
        if let Some((written_at, snapshot)) = &entry.value
            && written_at.elapsed() < timings.ttl
        {
            return SnapshotState::Ready(snapshot.clone());
        }
        let claimed = !entry.computing;
        entry.computing = true;
        (entry.value.clone(), claimed)
    };

    if claimed {
        tokio::spawn(run_compute(
            cache.clone(),
            provider,
            key.clone(),
            types.to_vec(),
            timings.compute_timeout,
        ));
    }

    // Cold and stale requests wait alike, for a value written after the one
    // this request saw. The lock is never held across the sleep.
    let seen_at = seen.as_ref().map(|(written_at, _)| *written_at);
    let deadline = Instant::now() + timings.wait;
    loop {
        {
            let Ok(guard) = cache.read() else {
                return SnapshotState::Pending;
            };
            if let Some((written_at, snapshot)) = guard.get(&key).and_then(|e| e.value.as_ref())
                && seen_at.is_none_or(|seen_at| *written_at > seen_at)
            {
                return SnapshotState::Ready(snapshot.clone());
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        tokio::time::sleep(POLL.min(remaining)).await;
    }

    match seen {
        // Stale beats absent; the compute still lands for the next request.
        Some((_, stale)) => SnapshotState::Ready(stale),
        // Not "no data" — the compute is still running and will fill the
        // cache, so callers must render waiting, not absence (#956).
        None => SnapshotState::Pending,
    }
}

/// Run `key`'s compute and write its value, always freeing the slot.
///
/// The provider runs on its own task: a panic surfaces as a join error instead
/// of unwinding through the cache, and the timeout can abort the work rather
/// than leave it running unobserved. The snapshot is written exactly as the
/// provider built it.
async fn run_compute(
    cache: SnapCache,
    provider: Arc<dyn DashboardProvider>,
    key: CacheKey,
    types: Vec<String>,
    compute_timeout: Duration,
) {
    let (window, tenant, _, include_empty) = key.clone();
    let mut work = tokio::spawn({
        let tenant = tenant.clone();
        async move {
            provider
                .snapshot(window, &tenant, &types, include_empty)
                .await
        }
    });

    let snapshot = match tokio::time::timeout(compute_timeout, &mut work).await {
        Ok(Ok(snapshot)) => Some(snapshot),
        Ok(Err(error)) => {
            warn!(
                window = window.as_str(),
                tenant = %tenant,
                %error,
                "dashboard snapshot compute failed"
            );
            None
        }
        Err(_elapsed) => {
            work.abort();
            warn!(
                window = window.as_str(),
                tenant = %tenant,
                timeout_ms = compute_timeout.as_millis() as u64,
                "dashboard snapshot compute timed out; aborted it and freed the refresh slot"
            );
            None
        }
    };

    if let Ok(mut guard) = cache.write()
        && let Some(entry) = guard.get_mut(&key)
    {
        entry.computing = false;
        if let Some(snapshot) = snapshot {
            entry.value = Some((Instant::now(), snapshot));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Compute budget for the tests that are not about the timeout: far longer
    /// than any fake provider's delay, so it never fires.
    const NO_TIMEOUT: Duration = Duration::from_secs(30);

    /// Answers its n-th compute (1-based) with `n` as `total_resources`, after
    /// `delays[n - 1]` (instantly once the script runs out), panicking instead
    /// on the computes listed in `panics`. Counts computes started and computes
    /// whose future was dropped before finishing (aborted).
    struct Scripted {
        hits: AtomicUsize,
        dropped: Arc<AtomicUsize>,
        delays: Vec<Duration>,
        panics: Vec<usize>,
    }

    impl Scripted {
        fn new(delays: &[Duration]) -> Arc<Self> {
            Self::panicking(delays, &[])
        }

        fn panicking(delays: &[Duration], panics: &[usize]) -> Arc<Self> {
            Arc::new(Scripted {
                hits: AtomicUsize::new(0),
                dropped: Arc::new(AtomicUsize::new(0)),
                delays: delays.to_vec(),
                panics: panics.to_vec(),
            })
        }

        fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst)
        }

        fn dropped(&self) -> usize {
            self.dropped.load(Ordering::SeqCst)
        }
    }

    /// Counts a compute future dropped before it finished.
    struct DropGuard(Option<Arc<AtomicUsize>>);

    impl Drop for DropGuard {
        fn drop(&mut self) {
            if let Some(dropped) = &self.0 {
                dropped.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    #[async_trait]
    impl DashboardProvider for Scripted {
        async fn snapshot(
            &self,
            window: DashboardWindow,
            _tenant: &str,
            _types: &[String],
            _include_empty: bool,
        ) -> DashboardSnapshot {
            let hit = self.hits.fetch_add(1, Ordering::SeqCst) + 1;
            let mut guard = DropGuard(Some(self.dropped.clone()));
            if let Some(delay) = self.delays.get(hit - 1)
                && !delay.is_zero()
            {
                tokio::time::sleep(*delay).await;
            }
            guard.0 = None;
            if self.panics.contains(&hit) {
                panic!("scripted provider panic on compute {hit}");
            }
            DashboardSnapshot {
                total_resources: hit as u64,
                window,
                figures: Figures::Exact {
                    read_at: Utc::now(),
                },
                ..DashboardSnapshot::default()
            }
        }
    }

    fn timings(ttl: Duration, wait: Duration, compute_timeout: Duration) -> Timings {
        Timings {
            ttl,
            wait,
            compute_timeout,
        }
    }

    /// One read of the default tenant's default selection over the 1h window.
    async fn read(
        cache: &SnapCache,
        provider: &Arc<Scripted>,
        include_empty: bool,
        timings: Timings,
    ) -> SnapshotState {
        snapshot_via(
            cache,
            provider.clone(),
            DashboardWindow::LastHour,
            "default",
            &[],
            include_empty,
            timings,
        )
        .await
    }

    fn total(state: SnapshotState) -> u64 {
        match state {
            SnapshotState::Ready(snapshot) => snapshot.total_resources,
            other => panic!("expected a ready snapshot, got {other:?}"),
        }
    }

    fn default_key() -> CacheKey {
        (
            DashboardWindow::LastHour,
            "default".to_string(),
            String::new(),
            false,
        )
    }

    /// Whether a compute holds `key`'s slot, and the cached `total_resources`.
    fn observe(cache: &SnapCache, key: &CacheKey) -> (bool, Option<u64>) {
        let guard = cache.read().expect("cache readable");
        let entry = guard.get(key).expect("the entry exists");
        (
            entry.computing,
            entry.value.as_ref().map(|(_, s)| s.total_resources),
        )
    }

    /// Polls `cond` every 10ms for up to two seconds, reporting whether it
    /// became true.
    async fn eventually(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..200 {
            if cond() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cond()
    }

    #[test]
    fn reindex_activity_percent_is_whole_and_unknown_until_counted() {
        let activity = |processed, total| ReindexActivity::Running {
            jobs: 1,
            processed,
            total,
        };
        assert_eq!(activity(0, 0).percent(), None);
        assert_eq!(activity(1, 3).percent(), Some(33));
        assert_eq!(activity(18_957_456, 18_957_914).percent(), Some(99));
        // A counter that overshoots its total never reads above 100%.
        assert_eq!(activity(12, 10).percent(), Some(100));
        assert!(activity(0, 0).is_running());
    }

    #[test]
    fn a_failed_rebuild_is_not_running_and_has_no_percentage() {
        let failed = ReindexActivity::Failed {
            job_id: "job-1".to_string(),
            errors: 11_704,
        };
        assert!(!failed.is_running());
        assert_eq!(failed.percent(), None);
    }

    #[tokio::test]
    async fn cold_load_fills_the_cache_and_fresh_hits_reuse_it() {
        let cache = SnapCache::default();
        let provider = Scripted::new(&[]);
        let t = timings(Duration::from_secs(60), WAIT, NO_TIMEOUT);

        assert_eq!(total(read(&cache, &provider, false, t).await), 1);
        assert_eq!(
            total(read(&cache, &provider, false, t).await),
            1,
            "served from cache"
        );
        assert_eq!(provider.hits(), 1, "no recompute inside the TTL");
        assert_eq!(observe(&cache, &default_key()), (false, Some(1)));
    }

    /// Past the TTL the request that noticed waits for its refresh and gets the
    /// new figures, not the previous poll's.
    #[tokio::test]
    async fn a_stale_value_is_replaced_by_the_refresh_landing_within_the_wait() {
        let cache = SnapCache::default();
        let provider = Scripted::new(&[]);
        let t = timings(Duration::from_millis(50), WAIT, NO_TIMEOUT);

        assert_eq!(total(read(&cache, &provider, false, t).await), 1);
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(
            total(read(&cache, &provider, false, t).await),
            2,
            "the refresh this request triggered, not the stale value"
        );
        assert_eq!(provider.hits(), 2);
    }

    /// A refresh slower than the wait does not hold the page: the stale value
    /// is served once the wait runs out, and the refresh lands for the next
    /// request.
    #[tokio::test]
    async fn a_slow_refresh_serves_the_stale_value_then_lands() {
        let cache = SnapCache::default();
        let provider = Scripted::new(&[Duration::ZERO, Duration::from_millis(300)]);
        let t = timings(
            Duration::from_millis(50),
            Duration::from_millis(100),
            NO_TIMEOUT,
        );

        assert_eq!(total(read(&cache, &provider, false, t).await), 1);
        tokio::time::sleep(Duration::from_millis(80)).await;
        let started = Instant::now();
        assert_eq!(
            total(read(&cache, &provider, false, t).await),
            1,
            "stale beats absent"
        );
        assert!(
            started.elapsed() >= t.wait,
            "waited for the refresh first: {:?}",
            started.elapsed()
        );
        assert_eq!(provider.hits(), 2, "the refresh was started");

        assert!(eventually(|| observe(&cache, &default_key()) == (false, Some(2))).await);
    }

    /// #956: a cold key whose compute outlasts the wait is *pending*, not
    /// absent, and the compute still fills the cache.
    #[tokio::test]
    async fn slow_cold_compute_reports_pending_then_ready() {
        let cache = SnapCache::default();
        let provider = Scripted::new(&[Duration::from_millis(300)]);
        let t = timings(
            Duration::from_secs(60),
            Duration::from_millis(50),
            NO_TIMEOUT,
        );

        let first = read(&cache, &provider, false, t).await;
        assert!(matches!(first, SnapshotState::Pending), "{first:?}");

        let second = read(&cache, &provider, false, t).await;
        assert!(
            matches!(second, SnapshotState::Pending),
            "still computing: {second:?}"
        );
        assert_eq!(provider.hits(), 1, "single-flight: no second compute");

        assert!(eventually(|| observe(&cache, &default_key()).1.is_some()).await);
        assert_eq!(total(read(&cache, &provider, false, t).await), 1);
        assert_eq!(provider.hits(), 1);
    }

    /// The "View all resources" toggle (#599) is part of the cache identity.
    #[tokio::test]
    async fn include_empty_is_part_of_the_cache_key() {
        let cache = SnapCache::default();
        let provider = Scripted::new(&[]);
        let t = timings(Duration::from_secs(60), WAIT, NO_TIMEOUT);

        assert_eq!(total(read(&cache, &provider, false, t).await), 1);
        assert_eq!(
            total(read(&cache, &provider, true, t).await),
            2,
            "not served from the flag-off entry"
        );
        assert_eq!(total(read(&cache, &provider, false, t).await), 1);
        assert_eq!(provider.hits(), 2);
    }

    /// #959: a compute past its timeout is aborted — its future dropped, not
    /// left running — and frees the slot, so the next request computes again.
    #[tokio::test]
    async fn a_timed_out_compute_is_aborted_and_frees_the_slot() {
        let cache = SnapCache::default();
        let provider = Scripted::new(&[Duration::from_secs(30)]);
        let t = timings(
            Duration::from_secs(60),
            Duration::from_millis(20),
            Duration::from_millis(100),
        );
        let key = default_key();

        let first = read(&cache, &provider, false, t).await;
        assert!(matches!(first, SnapshotState::Pending), "{first:?}");
        assert!(observe(&cache, &key).0, "the compute holds the slot");

        assert!(
            eventually(|| provider.dropped() == 1).await,
            "the timed-out compute is aborted"
        );
        assert!(
            eventually(|| observe(&cache, &key) == (false, None)).await,
            "the slot is freed and nothing is written"
        );

        read(&cache, &provider, false, t).await;
        assert_eq!(provider.hits(), 2, "a new compute started");
        assert!(eventually(|| observe(&cache, &key) == (false, Some(2))).await);
    }

    /// A panicking provider surfaces as a join error: the slot is freed, the
    /// cache stays usable, and the next request computes again.
    #[tokio::test]
    async fn a_panicking_provider_frees_the_slot() {
        let cache = SnapCache::default();
        let provider = Scripted::panicking(&[], &[1]);
        let t = timings(
            Duration::from_secs(60),
            Duration::from_millis(100),
            NO_TIMEOUT,
        );
        let key = default_key();

        let first = read(&cache, &provider, false, t).await;
        assert!(matches!(first, SnapshotState::Pending), "{first:?}");
        assert_eq!(observe(&cache, &key), (false, None), "slot freed");

        assert_eq!(total(read(&cache, &provider, false, t).await), 2);
        assert_eq!(provider.hits(), 2);
    }

    /// Concurrent requests share one compute, on a cold key and on a stale one.
    #[tokio::test]
    async fn concurrent_requests_run_one_compute() {
        let cache = SnapCache::default();
        let provider = Scripted::new(&[Duration::from_millis(100), Duration::from_millis(100)]);
        let t = timings(Duration::from_millis(300), WAIT, NO_TIMEOUT);

        let burst = |expected: u64| {
            let requests: Vec<_> = (0..8)
                .map(|_| {
                    let cache = cache.clone();
                    let provider = provider.clone();
                    tokio::spawn(async move { read(&cache, &provider, false, t).await })
                })
                .collect();
            async move {
                for request in requests {
                    assert_eq!(
                        total(request.await.expect("request task")),
                        expected,
                        "every waiter sees the one compute"
                    );
                }
            }
        };

        burst(1).await;
        assert_eq!(provider.hits(), 1, "single-flight on a cold key");

        tokio::time::sleep(Duration::from_millis(350)).await;
        burst(2).await;
        assert_eq!(provider.hits(), 2, "single-flight on a stale key");
    }

    /// Echoes back the window it was asked for, with a fixed "as of" time.
    struct Fixed;

    const FIXED_READ_AT: i64 = 1_752_454_800;

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
                reindex_active: None,
                figures: Figures::Exact {
                    read_at: DateTime::from_timestamp(FIXED_READ_AT, 0).unwrap(),
                },
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
        // The provider's figures pass through the cache untouched.
        assert_eq!(
            snap.figures,
            Figures::Exact {
                read_at: DateTime::from_timestamp(FIXED_READ_AT, 0).unwrap(),
            }
        );

        // The same read through the three-state entry point reports Ready —
        // the state the UI needs to tell a real snapshot from a slow one.
        let state = snapshot_state(DashboardWindow::LastHour, "default", &[], false).await;
        assert!(matches!(state, SnapshotState::Ready(_)), "{state:?}");
    }

    #[test]
    fn figures_helpers_follow_the_variant() {
        let read_at = DateTime::from_timestamp(FIXED_READ_AT, 0).unwrap();
        let exact = Figures::Exact { read_at };
        let approximate = Figures::Approximate {
            read_at,
            reconciled_at: read_at,
        };
        assert_eq!(exact.read_at(), Some(read_at));
        assert!(exact.is_known() && !exact.is_approximate());
        assert_eq!(approximate.read_at(), Some(read_at));
        assert!(approximate.is_known() && approximate.is_approximate());
        for unknown in [Figures::Pending, Figures::Unsupported] {
            assert_eq!(unknown.read_at(), None);
            assert!(!unknown.is_known() && !unknown.is_approximate());
        }
        assert_eq!(DashboardSnapshot::default().figures, Figures::Pending);
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
