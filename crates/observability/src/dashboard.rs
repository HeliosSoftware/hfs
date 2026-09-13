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
    /// The instant this snapshot's figures were read — what the UI's "as of"
    /// label reports (#1078).
    ///
    /// A snapshot can be served well after it was computed: stale while a
    /// refresh runs, landed late by a compute that overran its time budget, or
    /// borrowed from a sibling cache entry while this window's own compute is
    /// still running. Without a timestamp, any of those reads as a live
    /// measurement, which #956 rules out. Providers set it to the moment they
    /// read storage; a provider that leaves it `None` has it filled in by the
    /// cache with the compute's completion time when the value is written.
    pub generated_at: Option<DateTime<Utc>>,
    /// Whether the figures are measured but not reconciled with storage — for
    /// example derived from in-memory write counters rather than a storage
    /// count (#1078).
    ///
    /// Unlike [`Self::partial`], nothing here was filled in: the numbers are
    /// real observations, just not an exact storage read. The UI labels them
    /// "approximate" so they are never presented as exact (#956).
    pub approximate: bool,
    /// Whether the per-type [`Self::series`] for [`Self::window`] are not
    /// available yet, even though the rest of the snapshot is (#1078).
    ///
    /// Set by the cache, never by a provider: when a window's first compute
    /// has not landed within the cold-load budget but the same tenant has a
    /// snapshot cached under another window or selection, that sibling's
    /// totals, type counts and job counts are served — they do not depend on
    /// the window — with `series` cleared and this flag raised. The chart must
    /// then render as waiting, never as an empty or flat chart: an empty
    /// `series` here is not a measured "nothing happened" (#956).
    pub series_pending: bool,
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
    /// the filled-in zeros are never read as measurements (#956). It should also
    /// stamp [`DashboardSnapshot::generated_at`] with the moment it read
    /// storage, and set [`DashboardSnapshot::approximate`] when its figures do
    /// not come from an exact storage read (#1078); it never sets
    /// [`DashboardSnapshot::series_pending`], which belongs to the cache.
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

/// A snapshot written to the cache, with what the freshness rules need.
struct CachedValue {
    /// When the value was written — what [`CACHE_TTL`] and [`LIVE_TTL`] are
    /// measured from, and how a request inside [`LIVE_WAIT`] tells its
    /// refresh from the stale value.
    written_at: Instant,
    /// Generation of the compute that produced the value. Generations are
    /// handed out under the cache lock as computes start, so comparing them
    /// compares start order: a late finisher only replaces a value whose
    /// compute started before its own (#1078).
    generation: u64,
    snapshot: DashboardSnapshot,
}

/// One cached key: the last computed snapshot (if any) and the bookkeeping for
/// the computes that refresh it.
#[derive(Default)]
struct CacheEntry {
    value: Option<CachedValue>,
    /// Whether a compute holds the refresh slot. Only the compute whose
    /// generation is still [`Self::generation`] may clear it, so a late
    /// finisher never frees a slot a newer compute has since claimed.
    computing: bool,
    /// Generation of the most recently started compute.
    generation: u64,
    /// Computes that overran [`COMPUTE_TIMEOUT`], gave up the refresh slot, and
    /// are still being awaited for a late write (see [`MAX_OVERRUNNING`]).
    overrunning: usize,
}

type SnapCache = Arc<RwLock<HashMap<CacheKey, CacheEntry>>>;

static CACHE: std::sync::LazyLock<SnapCache> = std::sync::LazyLock::new(SnapCache::default);

/// How long a computed snapshot is served without recomputing.
///
/// Applies to snapshots read from storage, whose computes can be expensive.
/// An [approximate](DashboardSnapshot::approximate) snapshot — served from
/// in-memory write counters, so it costs milliseconds to recompute — is only
/// fresh for [`LIVE_TTL`] instead (#1078).
const CACHE_TTL: Duration = Duration::from_secs(15);
/// How long an [approximate](DashboardSnapshot::approximate) snapshot is
/// served without recomputing (#1078).
///
/// Approximate snapshots come from in-memory write counters while an import
/// runs, and the dashboard polls them every few seconds so the operator can
/// watch the figures move. Held for the full [`CACHE_TTL`], every poll tick
/// would show the previous tick's figures — up to ~20s behind. Their computes
/// are O(1), so a short TTL costs nothing. Values that are not approximate
/// keep [`CACHE_TTL`].
const LIVE_TTL: Duration = Duration::from_secs(2);
/// How long a request that finds a stale
/// [approximate](DashboardSnapshot::approximate) snapshot waits for the
/// refresh it triggered before serving the stale value (#1078).
///
/// Stale-while-revalidate is right for storage-backed snapshots, but for
/// counter-backed ones it means each request shows what the *previous*
/// request computed. The refresh takes milliseconds, so waiting a short beat
/// serves the current figures on every poll; a refresh slower than this (a
/// backend under pressure) still falls back to the stale value rather than
/// holding the page.
const LIVE_WAIT: Duration = Duration::from_millis(500);
/// How often a request inside [`LIVE_WAIT`] re-checks the cache for its
/// refresh.
const LIVE_POLL: Duration = Duration::from_millis(20);
/// How long a cold request waits for the first compute before falling back to
/// a sibling snapshot or [`SnapshotState::Pending`]. Long enough for row-store
/// backends (milliseconds); deliberately far below what an object-store scan
/// can take.
const COLD_WAIT: Duration = Duration::from_millis(800);
/// How long a background compute may hold the refresh slot before the cache
/// frees the slot for another compute (#959).
///
/// Without this, a compute that never returns — a snapshot stuck behind a
/// 30s connection-pool acquire, say — leaves its cache entry pinned at
/// `computing: true` forever, and *no* later request ever spawns a refresh:
/// the entry goes permanently stale (or, if it was cold, permanently absent)
/// for the lifetime of the process.
///
/// The value is deliberately ≥ 2× [`CACHE_TTL`]. Anything shorter and a
/// slow-but-progressing compute would lose its slot on nearly every pass,
/// piling up detached computes behind each other.
///
/// Elapsing releases the slot but does **not** abandon the compute (#1078).
/// The provider future keeps running on its own task and, when it finishes,
/// writes its late value — unless a compute that started after it has already
/// written a value. Before #1078 the future was dropped here and wrote
/// nothing, so under sustained load (a large import) a key whose compute
/// always took longer than this never got a value at all: every retry
/// restarted the same scan from scratch and the 1h chart waited forever.
/// Late computes are bounded per key by [`MAX_OVERRUNNING`] and awaited for at
/// most [`LATE_WRITE_FACTOR`] × this budget.
const COMPUTE_TIMEOUT: Duration = Duration::from_secs(30);
/// How many overrun computes per key may still be running before the cache
/// stops starting new ones for that key (#1078).
///
/// Every compute that loses its slot keeps running for a late write, so
/// without a bound each retry past [`COMPUTE_TIMEOUT`] would add another scan
/// to the very load that made the previous one slow. With the bound a key runs
/// at most `1 + MAX_OVERRUNNING` computes at once; meanwhile requests are
/// served the stale value, a sibling snapshot, or
/// [`SnapshotState::Pending`]. The bound cannot lock a key out for good (the
/// failure #959 fixed): overrun computes stop counting as soon as they finish,
/// and one that never finishes is abandoned after [`LATE_WRITE_FACTOR`] ×
/// [`COMPUTE_TIMEOUT`].
const MAX_OVERRUNNING: usize = 2;
/// How many [`COMPUTE_TIMEOUT`]s, counted from its start, an overrun compute
/// is awaited for a late write before it is abandoned (10 minutes with the
/// production budget).
///
/// Abandoning drops the provider future and frees its [`MAX_OVERRUNNING`]
/// place, so a compute that never returns cannot hold the key's pile-up
/// budget forever. As with any dropped future, work already handed to
/// `spawn_blocking` (the SQLite backend) still runs to completion; this bounds
/// the cache's bookkeeping, not the backend's work.
const LATE_WRITE_FACTOR: u32 = 20;

/// The outcome of a dashboard read. The two ways of not having a snapshot are
/// kept apart on purpose (#956): "this build has no metrics" and "the metrics
/// are not here yet" call for different pages, and collapsing them into one
/// `None` is what let a slow window render as invented sample data.
#[derive(Clone, Debug)]
pub enum SnapshotState {
    /// A snapshot from the registered provider — fresh, the previous one while
    /// a refresh runs (for an [approximate](DashboardSnapshot::approximate)
    /// snapshot, only when the refresh does not land within [`LIVE_WAIT`]),
    /// or (for a window whose first compute has not landed
    /// yet) a sibling's figures with [`DashboardSnapshot::series_pending`]
    /// set. Check [`DashboardSnapshot::partial`],
    /// [`DashboardSnapshot::approximate`] and
    /// [`DashboardSnapshot::series_pending`] before presenting its figures as
    /// complete, and [`DashboardSnapshot::generated_at`] for how old they are.
    Ready(DashboardSnapshot),
    /// A provider is registered, but the first compute for this key has not
    /// landed within the cold-load budget, and the tenant has no other cached
    /// snapshot to borrow figures from — a truly cold tenant. The compute is
    /// still running and will fill the cache, so the same request repeated
    /// shortly usually succeeds.
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
/// registered or nothing has been computed for the tenant yet. Prefer
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
/// request waits up to [`COLD_WAIT`] for its own compute. This keeps page
/// loads O(1) even on backends where computing the snapshot walks storage (the
/// S3 primary reads one object per resource — minutes once conformance seeding
/// has populated the store, #326).
///
/// [Approximate](DashboardSnapshot::approximate) snapshots take a fast path
/// (#1078): they come from in-memory write counters, so recomputing them is
/// O(1) while serving them stale would show each dashboard poll the previous
/// poll's figures. They are fresh only for [`LIVE_TTL`], and a stale request
/// waits up to [`LIVE_WAIT`] for its refresh to land — serving the stale value
/// only if it does not. Single-flight and the other refresh rules are the
/// same for both kinds.
///
/// A cold request that outlasts [`COLD_WAIT`] is answered from the freshest
/// snapshot cached for the same tenant and "View all resources" setting under
/// another window or selection (the same selection preferred), re-labelled
/// with the requested window, its `series` cleared and
/// [`DashboardSnapshot::series_pending`] set (#1078). Its totals and job counts
/// do not depend on the window, so they are real — as of the sibling's
/// [`DashboardSnapshot::generated_at`] — while the chart honestly waits. That
/// borrowed snapshot is never cached under the requested key; the real compute
/// keeps running and lands normally. Only a tenant with nothing cached at all
/// gets [`SnapshotState::Pending`].
///
/// A background refresh that overruns [`COMPUTE_TIMEOUT`] releases its refresh
/// slot, so a single stuck compute cannot freeze the entry forever (#959), but
/// keeps running and writes its late value when it finishes (#1078).
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
        LIVE_TTL,
        LIVE_WAIT,
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
    ttl: Duration,
    cold_wait: Duration,
    compute_timeout: Duration,
    live_ttl: Duration,
    live_wait: Duration,
) -> SnapshotState {
    // The charted set (and the "View all resources" toggle, #599) is part of
    // the cache identity: two selections — or the same selection with the
    // toggle flipped — are two different snapshots. Selections are short
    // (providers cap them), so the joined key stays small and the cache stays
    // bounded by user behavior.
    let key: CacheKey = (window, tenant.to_string(), types.join(","), include_empty);
    // One pass under the lock: serve fresh hits, note staleness, and claim the
    // compute slot if nobody holds it.
    let (cached, claimed) = {
        // A poisoned cache lock means a compute panicked mid-write: nothing is
        // readable and nothing can be spawned, which is exactly "not here yet".
        let Ok(mut guard) = cache.write() else {
            return SnapshotState::Pending;
        };
        let entry = guard.entry(key.clone()).or_default();
        if let Some(value) = &entry.value {
            // Counter-backed figures are cheap to recompute and watched live,
            // so they go stale much sooner than storage reads (#1078).
            let fresh_for = if value.snapshot.approximate {
                ttl.min(live_ttl)
            } else {
                ttl
            };
            if value.written_at.elapsed() < fresh_for {
                return SnapshotState::Ready(value.snapshot.clone());
            }
        }
        // Single-flight, with the computes that overran their budget still
        // counted against the key so retries under load cannot stampede.
        let claimed = if !entry.computing && entry.overrunning < MAX_OVERRUNNING {
            entry.computing = true;
            entry.generation += 1;
            Some(entry.generation)
        } else {
            None
        };
        (
            entry
                .value
                .as_ref()
                .map(|value| (value.written_at, value.snapshot.clone())),
            claimed,
        )
    };

    if let Some(generation) = claimed {
        tokio::spawn(run_compute(
            cache.clone(),
            provider,
            key.clone(),
            types.to_vec(),
            generation,
            compute_timeout,
        ));
    }

    if let Some((stale_written_at, stale)) = cached {
        // Stale beats absent: a storage-backed value is served now, and the
        // refresh lands for the next load.
        if !stale.approximate {
            return SnapshotState::Ready(stale);
        }
        // A counter-backed refresh takes milliseconds: give it a short beat so
        // this request shows current figures, not the previous request's
        // (#1078). Only a value written after the stale one counts; the lock
        // is never held across the sleep.
        let deadline = Instant::now() + live_wait;
        loop {
            if let Ok(guard) = cache.read()
                && let Some(value) = guard.get(&key).and_then(|e| e.value.as_ref())
                && value.written_at > stale_written_at
            {
                return SnapshotState::Ready(value.snapshot.clone());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::time::sleep(LIVE_POLL.min(remaining)).await;
        }
        // The refresh is slower than the beat: the stale value it is, and the
        // refresh still lands for the next load.
        return SnapshotState::Ready(stale);
    }

    // Cold: give a fast backend a beat to fill the cache before degrading.
    let deadline = Instant::now() + cold_wait;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Ok(guard) = cache.read()
            && let Some(value) = guard.get(&key).and_then(|e| e.value.as_ref())
        {
            return SnapshotState::Ready(value.snapshot.clone());
        }
    }
    if let Ok(guard) = cache.read() {
        if let Some(value) = guard.get(&key).and_then(|e| e.value.as_ref()) {
            return SnapshotState::Ready(value.snapshot.clone());
        }
        // The window's own series are not here yet, but the tenant's totals
        // may be: every window switch is a cold key, and during a large import
        // the finer windows can take longer than any wait (#1078).
        if let Some(sibling) = sibling_snapshot(&guard, &key) {
            return SnapshotState::Ready(sibling);
        }
    }
    // Not "no data" — the compute is still running and will land in the cache.
    // A tenant with nothing cached at all takes this path, so callers must
    // render waiting, not absence (#956).
    SnapshotState::Pending
}

/// The freshest snapshot cached for the same tenant and "View all resources"
/// setting as `key` under a different window or selection, re-labelled as a
/// series-pending snapshot for `key`'s window (#1078).
///
/// Snapshots from the same selection are preferred — their `available` list
/// matches what was asked for — then the most recently generated. Everything
/// but the series is kept as the sibling reported it, including `partial`,
/// `approximate` and `generated_at`, so the borrowed figures stay exactly as
/// qualified as they were.
fn sibling_snapshot(
    entries: &HashMap<CacheKey, CacheEntry>,
    key: &CacheKey,
) -> Option<DashboardSnapshot> {
    let (window, tenant, types, include_empty) = key;
    entries
        .iter()
        .filter(|(k, _)| k != &key && &k.1 == tenant && k.3 == *include_empty)
        .filter_map(|(k, entry)| entry.value.as_ref().map(|value| (k, value)))
        .max_by_key(|(k, value)| (&k.2 == types, value.snapshot.generated_at, value.written_at))
        .map(|(_, value)| DashboardSnapshot {
            window: *window,
            series: Vec::new(),
            series_pending: true,
            ..value.snapshot.clone()
        })
}

/// Run `key`'s compute of `generation` to completion and write its value.
///
/// The provider runs on its own task, so the time-box below can stop *waiting*
/// without dropping the work (#1078), and a panicking provider surfaces as a
/// join error that still releases the slot instead of pinning it (#959).
async fn run_compute(
    cache: SnapCache,
    provider: Arc<dyn DashboardProvider>,
    key: CacheKey,
    types: Vec<String>,
    generation: u64,
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

    let mut overran = false;
    let joined = match tokio::time::timeout(compute_timeout, &mut work).await {
        Ok(joined) => Some(joined),
        Err(_elapsed) => {
            warn!(
                window = window.as_str(),
                tenant = %tenant,
                timeout_ms = compute_timeout.as_millis() as u64,
                "dashboard snapshot compute timed out; releasing the refresh slot \
                 and keeping the compute running for a late write"
            );
            // Free the slot so the next request may retry (#959) — unless a
            // newer compute has claimed it since, which is not ours to free.
            overran = true;
            with_entry(&cache, &key, |entry| {
                if entry.generation == generation {
                    entry.computing = false;
                }
                entry.overrunning += 1;
            });
            let late_budget = compute_timeout
                .saturating_mul(LATE_WRITE_FACTOR)
                .saturating_sub(compute_timeout);
            match tokio::time::timeout(late_budget, &mut work).await {
                Ok(joined) => Some(joined),
                Err(_elapsed) => {
                    work.abort();
                    warn!(
                        window = window.as_str(),
                        tenant = %tenant,
                        "dashboard snapshot compute still unfinished after its late-write \
                         budget; abandoning it"
                    );
                    None
                }
            }
        }
    };
    let snapshot = match joined {
        Some(Ok(snapshot)) => Some(snapshot),
        Some(Err(error)) => {
            warn!(
                window = window.as_str(),
                tenant = %tenant,
                %error,
                "dashboard snapshot compute failed"
            );
            None
        }
        None => None,
    };

    with_entry(&cache, &key, |entry| {
        if overran {
            entry.overrunning = entry.overrunning.saturating_sub(1);
        }
        if entry.generation == generation {
            entry.computing = false;
        }
        let Some(mut snapshot) = snapshot else {
            // Failed or abandoned: the entry keeps whatever it had.
            return;
        };
        // A late value must not replace one from a compute that started after
        // it: that value reflects storage more recently, whatever order the
        // two finished in.
        if entry
            .value
            .as_ref()
            .is_some_and(|value| value.generation > generation)
        {
            return;
        }
        if snapshot.generated_at.is_none() {
            snapshot.generated_at = Some(Utc::now());
        }
        entry.value = Some(CachedValue {
            written_at: Instant::now(),
            generation,
            snapshot,
        });
    });
}

/// Apply `f` to `key`'s entry under the write lock. A poisoned lock or a
/// missing entry is skipped: there is nothing safe to update.
fn with_entry(cache: &SnapCache, key: &CacheKey, f: impl FnOnce(&mut CacheEntry)) {
    if let Ok(mut guard) = cache.write()
        && let Some(entry) = guard.get_mut(key)
    {
        f(entry);
    }
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
            LIVE_TTL,
            LIVE_WAIT,
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
            LIVE_TTL,
            LIVE_WAIT,
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
            LIVE_TTL,
            LIVE_WAIT,
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
            LIVE_TTL,
            LIVE_WAIT,
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
            LIVE_TTL,
            LIVE_WAIT,
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
            LIVE_TTL,
            LIVE_WAIT,
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
            LIVE_TTL,
            LIVE_WAIT,
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

    /// Answers its n-th compute (1-based) after `delays[n - 1]` — instantly once
    /// the script runs out — with `n` as `total_resources`. Scripting each
    /// compute's duration is what makes the timeout paths observable: which
    /// compute overran, which landed late, and which value won.
    struct Scripted {
        hits: AtomicUsize,
        delays: Vec<Duration>,
    }

    impl Scripted {
        fn new(delays: &[Duration]) -> Arc<Self> {
            Arc::new(Scripted {
                hits: AtomicUsize::new(0),
                delays: delays.to_vec(),
            })
        }

        fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst)
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
            if let Some(delay) = self.delays.get(hit - 1) {
                tokio::time::sleep(*delay).await;
            }
            DashboardSnapshot {
                total_resources: hit as u64,
                window,
                ..DashboardSnapshot::default()
            }
        }
    }

    /// The injected timings, bundled so the timeout tests read as scripts.
    #[derive(Clone, Copy)]
    struct Timings {
        ttl: Duration,
        cold: Duration,
        compute: Duration,
        live_ttl: Duration,
        live_wait: Duration,
    }

    /// One read of the default tenant's default selection over `window`.
    async fn read(
        cache: &SnapCache,
        provider: Arc<dyn DashboardProvider>,
        window: DashboardWindow,
        timings: Timings,
    ) -> SnapshotState {
        snapshot_via(
            cache.clone(),
            provider,
            window,
            "default",
            &[],
            false,
            timings.ttl,
            timings.cold,
            timings.compute,
            timings.live_ttl,
            timings.live_wait,
        )
        .await
    }

    /// What a test can observe about one cache entry: whether a compute holds
    /// the slot, how many overrun computes are still awaited, and the cached
    /// value's `total_resources`.
    #[derive(Debug, PartialEq)]
    struct Observed {
        computing: bool,
        overrunning: usize,
        total: Option<u64>,
    }

    fn observe(cache: &SnapCache, key: &CacheKey) -> Observed {
        let guard = cache.read().expect("cache readable");
        let entry = guard.get(key).expect("the entry exists");
        Observed {
            computing: entry.computing,
            overrunning: entry.overrunning,
            total: entry
                .value
                .as_ref()
                .map(|value| value.snapshot.total_resources),
        }
    }

    fn default_key(window: DashboardWindow) -> CacheKey {
        (window, "default".to_string(), String::new(), false)
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

    /// #1078: a compute that overruns `compute_timeout` still lands its value.
    /// Before, the time-box dropped the provider future and wrote nothing, so
    /// under sustained load a key whose compute always took longer than the
    /// budget never got a value — the 1h chart waited forever. The slot is
    /// still released at the timeout (#959); only the result is kept.
    #[tokio::test]
    async fn timed_out_compute_releases_the_slot_and_its_late_value_still_lands() {
        let cache = SnapCache::default();
        let provider = Scripted::new(&[Duration::from_millis(400)]);
        let timings = Timings {
            ttl: Duration::from_secs(60),
            cold: Duration::from_millis(50),
            compute: Duration::from_millis(100),
            live_ttl: LIVE_TTL,
            live_wait: LIVE_WAIT,
        };
        let key = default_key(DashboardWindow::LastMonth);

        let first = read(
            &cache,
            provider.clone(),
            DashboardWindow::LastMonth,
            timings,
        )
        .await;
        assert!(
            matches!(first, SnapshotState::Pending),
            "the slow compute has not landed, and a registered provider that is \
             merely slow is pending, never absent (#956): {first:?}"
        );

        assert!(
            eventually(|| !observe(&cache, &key).computing).await,
            "the overrun compute must free the refresh slot"
        );
        assert_eq!(
            observe(&cache, &key),
            Observed {
                computing: false,
                overrunning: 1,
                total: None,
            },
            "released at the timeout, while the compute is still running"
        );

        assert!(
            eventually(|| observe(&cache, &key).total.is_some()).await,
            "the overrun compute writes its late value"
        );
        assert_eq!(
            observe(&cache, &key),
            Observed {
                computing: false,
                overrunning: 0,
                total: Some(1),
            }
        );

        let second = read(
            &cache,
            provider.clone(),
            DashboardWindow::LastMonth,
            timings,
        )
        .await
        .ready()
        .expect("the late value is served");
        assert_eq!(second.total_resources, 1);
        assert_eq!(
            provider.hits(),
            1,
            "served from the late write, no recompute"
        );
    }

    /// #959: a compute that overruns `compute_timeout` must release its refresh
    /// slot. Before the time-box, `computing` stayed `true` forever and the
    /// entry could never be refreshed again for the life of the process — this
    /// test would hang at [`SnapshotState::Pending`] on the second call.
    #[tokio::test]
    async fn timed_out_compute_releases_the_slot_so_a_later_request_retries() {
        let cache = SnapCache::default();
        // Effectively never returns within this test.
        let provider = Scripted::new(&[Duration::from_secs(30)]);
        let timings = Timings {
            ttl: Duration::from_secs(60),
            cold: Duration::from_millis(50),
            compute: Duration::from_millis(100),
            live_ttl: LIVE_TTL,
            live_wait: LIVE_WAIT,
        };
        let key = default_key(DashboardWindow::LastMonth);

        let first = read(
            &cache,
            provider.clone(),
            DashboardWindow::LastMonth,
            timings,
        )
        .await;
        assert!(matches!(first, SnapshotState::Pending), "{first:?}");
        assert!(
            eventually(|| !observe(&cache, &key).computing).await,
            "the overrun compute must free the refresh slot"
        );

        // With the slot free, the next request spawns a fresh compute — which
        // is instant this time — and a value lands.
        let second = read(
            &cache,
            provider.clone(),
            DashboardWindow::LastMonth,
            timings,
        )
        .await
        .ready()
        .expect("a new compute was spawned and landed");
        assert_eq!(second.total_resources, 2);
        assert_eq!(provider.hits(), 2, "exactly one retry, not a stampede");
    }

    /// A late value reflects storage as of when its compute *started*, so it
    /// must not replace a value from a compute that started after it, even
    /// though it finishes last (#1078).
    #[tokio::test]
    async fn a_late_result_does_not_overwrite_a_value_from_a_newer_compute() {
        let cache = SnapCache::default();
        let provider = Scripted::new(&[Duration::from_millis(400)]);
        let timings = Timings {
            ttl: Duration::from_secs(60),
            cold: Duration::from_millis(50),
            compute: Duration::from_millis(100),
            live_ttl: LIVE_TTL,
            live_wait: LIVE_WAIT,
        };
        let key = default_key(DashboardWindow::LastMonth);

        let first = read(
            &cache,
            provider.clone(),
            DashboardWindow::LastMonth,
            timings,
        )
        .await;
        assert!(matches!(first, SnapshotState::Pending), "{first:?}");
        assert!(eventually(|| !observe(&cache, &key).computing).await);

        let newer = read(
            &cache,
            provider.clone(),
            DashboardWindow::LastMonth,
            timings,
        )
        .await
        .ready()
        .expect("the retry lands first");
        assert_eq!(newer.total_resources, 2);

        assert!(
            eventually(|| observe(&cache, &key).overrunning == 0).await,
            "the first compute finishes"
        );
        assert_eq!(
            observe(&cache, &key).total,
            Some(2),
            "the older compute's late result was discarded"
        );
        let served = read(
            &cache,
            provider.clone(),
            DashboardWindow::LastMonth,
            timings,
        )
        .await
        .ready()
        .expect("cached");
        assert_eq!(served.total_resources, 2);
        assert_eq!(provider.hits(), 2);
    }

    /// A late finisher must not clear `computing` for a newer compute that
    /// claimed the slot after it overran: that would let a third compute start
    /// alongside the one still in flight (#1078).
    #[tokio::test]
    async fn a_late_finisher_does_not_free_a_slot_a_newer_compute_holds() {
        let cache = SnapCache::default();
        // The first overruns and lands at 450ms; the second claims the slot at
        // ~310ms and holds it until its own timeout at ~610ms.
        let provider = Scripted::new(&[Duration::from_millis(450), Duration::from_secs(10)]);
        let timings = Timings {
            ttl: Duration::from_secs(60),
            cold: Duration::from_millis(20),
            compute: Duration::from_millis(300),
            live_ttl: LIVE_TTL,
            live_wait: LIVE_WAIT,
        };
        let key = default_key(DashboardWindow::LastMonth);

        let first = read(
            &cache,
            provider.clone(),
            DashboardWindow::LastMonth,
            timings,
        )
        .await;
        assert!(matches!(first, SnapshotState::Pending), "{first:?}");
        assert!(eventually(|| !observe(&cache, &key).computing).await);

        let second = read(
            &cache,
            provider.clone(),
            DashboardWindow::LastMonth,
            timings,
        )
        .await;
        assert!(matches!(second, SnapshotState::Pending), "{second:?}");
        assert_eq!(provider.hits(), 2);

        assert!(
            eventually(|| observe(&cache, &key).total.is_some()).await,
            "the first compute lands late"
        );
        assert_eq!(
            observe(&cache, &key),
            Observed {
                computing: true,
                overrunning: 0,
                total: Some(1),
            },
            "the late write keeps the newer compute's claim on the slot"
        );
    }

    /// Overrun computes keep running, so each retry past the budget would add
    /// another scan to the load that made the last one slow. Two of them
    /// block a third from starting; as soon as one finishes, a new compute
    /// may start again (#1078).
    #[tokio::test]
    async fn overrun_computes_bound_the_pile_up_until_one_finishes() {
        let cache = SnapCache::default();
        let provider = Scripted::new(&[Duration::from_millis(400), Duration::from_millis(400)]);
        let timings = Timings {
            ttl: Duration::ZERO, // every landed value is instantly stale
            cold: Duration::from_millis(20),
            compute: Duration::from_millis(50),
            live_ttl: LIVE_TTL,
            live_wait: LIVE_WAIT,
        };
        let key = default_key(DashboardWindow::LastDay);

        for overrun in 1..=MAX_OVERRUNNING {
            let state = read(&cache, provider.clone(), DashboardWindow::LastDay, timings).await;
            assert!(matches!(state, SnapshotState::Pending), "{state:?}");
            assert!(
                eventually(|| {
                    let seen = observe(&cache, &key);
                    !seen.computing && seen.overrunning == overrun
                })
                .await,
                "compute {overrun} overran"
            );
        }

        let blocked = read(&cache, provider.clone(), DashboardWindow::LastDay, timings).await;
        assert!(matches!(blocked, SnapshotState::Pending), "{blocked:?}");
        assert_eq!(
            provider.hits(),
            MAX_OVERRUNNING,
            "no new compute while the pile-up budget is spent"
        );
        assert!(!observe(&cache, &key).computing);

        assert!(
            eventually(|| observe(&cache, &key).total.is_some()).await,
            "the first overrun compute lands"
        );
        let stale = read(&cache, provider.clone(), DashboardWindow::LastDay, timings).await;
        assert!(matches!(stale, SnapshotState::Ready(_)), "{stale:?}");
        assert!(
            eventually(|| provider.hits() == MAX_OVERRUNNING + 1).await,
            "a finished overrun compute frees room for a new one"
        );
    }

    /// The pile-up bound must not bring back the permanent lockout #959 fixed:
    /// overrun computes that never finish are abandoned after
    /// `LATE_WRITE_FACTOR` × the compute budget, and the key computes again.
    #[tokio::test]
    async fn never_finishing_overrun_computes_are_abandoned_so_the_key_recovers() {
        let cache = SnapCache::default();
        let provider = Scripted::new(&[Duration::from_secs(30), Duration::from_secs(30)]);
        let timings = Timings {
            ttl: Duration::from_secs(60),
            cold: Duration::from_millis(20),
            // Abandoned 20 × 20ms = 400ms after each start.
            compute: Duration::from_millis(20),
            live_ttl: LIVE_TTL,
            live_wait: LIVE_WAIT,
        };
        let key = default_key(DashboardWindow::LastHour);

        for overrun in 1..=MAX_OVERRUNNING {
            let state = read(&cache, provider.clone(), DashboardWindow::LastHour, timings).await;
            assert!(matches!(state, SnapshotState::Pending), "{state:?}");
            assert!(
                eventually(|| {
                    let seen = observe(&cache, &key);
                    !seen.computing && seen.overrunning == overrun
                })
                .await
            );
        }
        let blocked = read(&cache, provider.clone(), DashboardWindow::LastHour, timings).await;
        assert!(matches!(blocked, SnapshotState::Pending), "{blocked:?}");
        assert_eq!(provider.hits(), MAX_OVERRUNNING);

        assert!(
            eventually(|| observe(&cache, &key).overrunning < MAX_OVERRUNNING).await,
            "the hung compute is abandoned"
        );
        let recovered = read(&cache, provider.clone(), DashboardWindow::LastHour, timings)
            .await
            .ready()
            .expect("a new compute starts and lands");
        assert_eq!(recovered.total_resources, MAX_OVERRUNNING as u64 + 1);
    }

    /// Like [`Scripted`] — the n-th compute answers `n` after `delays[n - 1]`
    /// — with every snapshot's `approximate` flag fixed, standing in for a
    /// counter-backed provider (`true`) or a storage read (`false`).
    struct Counters {
        hits: AtomicUsize,
        delays: Vec<Duration>,
        approximate: bool,
    }

    impl Counters {
        fn new(approximate: bool, delays: &[Duration]) -> Arc<Self> {
            Arc::new(Counters {
                hits: AtomicUsize::new(0),
                delays: delays.to_vec(),
                approximate,
            })
        }

        fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl DashboardProvider for Counters {
        async fn snapshot(
            &self,
            window: DashboardWindow,
            _tenant: &str,
            _types: &[String],
            _include_empty: bool,
        ) -> DashboardSnapshot {
            let hit = self.hits.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some(delay) = self.delays.get(hit - 1)
                && !delay.is_zero()
            {
                tokio::time::sleep(*delay).await;
            }
            DashboardSnapshot {
                total_resources: hit as u64,
                window,
                approximate: self.approximate,
                ..DashboardSnapshot::default()
            }
        }
    }

    /// #1078: the dashboard polls counter-backed figures every few seconds. An
    /// approximate value past the live TTL — though well inside the storage
    /// TTL — is recomputed, and the request that noticed gets the *new*
    /// figures, not the previous poll's.
    #[tokio::test]
    async fn a_stale_approximate_value_is_recomputed_and_the_request_sees_the_new_figures() {
        let cache = SnapCache::default();
        let provider = Counters::new(true, &[]);
        let timings = Timings {
            ttl: Duration::from_secs(60),
            cold: Duration::from_millis(800),
            compute: TEST_COMPUTE_TIMEOUT,
            live_ttl: Duration::from_millis(100),
            live_wait: Duration::from_millis(500),
        };

        let first = read(&cache, provider.clone(), DashboardWindow::LastHour, timings)
            .await
            .ready()
            .expect("cold load");
        assert_eq!(first.total_resources, 1);
        let fresh = read(&cache, provider.clone(), DashboardWindow::LastHour, timings)
            .await
            .ready()
            .expect("fresh hit");
        assert_eq!(fresh.total_resources, 1, "inside the live TTL: cached");
        assert_eq!(provider.hits(), 1);

        tokio::time::sleep(Duration::from_millis(150)).await;
        let live = read(&cache, provider.clone(), DashboardWindow::LastHour, timings)
            .await
            .ready()
            .expect("refreshed");
        assert_eq!(
            live.total_resources, 2,
            "the refresh this request triggered, not the previous value"
        );
        assert_eq!(provider.hits(), 2);
    }

    /// The live TTL is for counter-backed figures only: a storage-backed value
    /// of the same age is still inside its TTL and served without a recompute.
    #[tokio::test]
    async fn a_non_approximate_value_keeps_the_full_ttl() {
        let cache = SnapCache::default();
        let provider = Counters::new(false, &[]);
        let timings = Timings {
            ttl: Duration::from_secs(60),
            cold: Duration::from_millis(800),
            compute: TEST_COMPUTE_TIMEOUT,
            live_ttl: Duration::from_millis(100),
            live_wait: Duration::from_millis(500),
        };

        let first = read(&cache, provider.clone(), DashboardWindow::LastHour, timings)
            .await
            .ready()
            .expect("cold load");
        assert_eq!(first.total_resources, 1);

        tokio::time::sleep(Duration::from_millis(150)).await;
        let cached = read(&cache, provider.clone(), DashboardWindow::LastHour, timings)
            .await
            .ready()
            .expect("fresh hit");
        assert_eq!(cached.total_resources, 1, "served from cache");
        assert_eq!(provider.hits(), 1, "no recompute inside the TTL");
    }

    /// A counter-backed refresh slower than the live wait does not hold the
    /// page: the stale value is served once the wait runs out, and the refresh
    /// still lands for the next request.
    #[tokio::test]
    async fn a_slow_approximate_refresh_serves_the_stale_value_then_lands() {
        let cache = SnapCache::default();
        let provider = Counters::new(true, &[Duration::ZERO, Duration::from_millis(400)]);
        let timings = Timings {
            ttl: Duration::from_secs(60),
            cold: Duration::from_millis(800),
            compute: TEST_COMPUTE_TIMEOUT,
            live_ttl: Duration::from_millis(300),
            live_wait: Duration::from_millis(100),
        };
        let key = default_key(DashboardWindow::LastHour);

        let first = read(&cache, provider.clone(), DashboardWindow::LastHour, timings)
            .await
            .ready()
            .expect("cold load");
        assert_eq!(first.total_resources, 1);

        tokio::time::sleep(Duration::from_millis(350)).await;
        let started = Instant::now();
        let stale = read(&cache, provider.clone(), DashboardWindow::LastHour, timings)
            .await
            .ready()
            .expect("stale beats absent");
        assert_eq!(stale.total_resources, 1, "the refresh has not landed");
        assert!(
            started.elapsed() >= timings.live_wait,
            "waited for the refresh first: {:?}",
            started.elapsed()
        );
        assert_eq!(provider.hits(), 2, "the refresh was started");

        assert!(
            eventually(|| observe(&cache, &key).total == Some(2)).await,
            "the slow refresh lands"
        );
        let refreshed = read(&cache, provider.clone(), DashboardWindow::LastHour, timings)
            .await
            .ready()
            .expect("cached");
        assert_eq!(refreshed.total_resources, 2);
        assert_eq!(provider.hits(), 2);
    }

    /// Waiting on the refresh must not undo single-flight: concurrent requests
    /// on a stale approximate key share one compute and all see its value.
    #[tokio::test]
    async fn concurrent_requests_on_a_stale_approximate_key_run_one_compute() {
        let cache = SnapCache::default();
        let provider = Counters::new(true, &[Duration::ZERO, Duration::from_millis(150)]);
        let timings = Timings {
            ttl: Duration::from_secs(60),
            cold: Duration::from_millis(800),
            compute: TEST_COMPUTE_TIMEOUT,
            live_ttl: Duration::from_millis(50),
            live_wait: Duration::from_millis(500),
        };

        let first = read(&cache, provider.clone(), DashboardWindow::LastHour, timings)
            .await
            .ready()
            .expect("cold load");
        assert_eq!(first.total_resources, 1);

        tokio::time::sleep(Duration::from_millis(80)).await;
        let requests: Vec<_> = (0..8)
            .map(|_| {
                let cache = cache.clone();
                let provider: Arc<dyn DashboardProvider> = provider.clone();
                tokio::spawn(async move {
                    read(&cache, provider, DashboardWindow::LastHour, timings).await
                })
            })
            .collect();
        for request in requests {
            let snapshot = request
                .await
                .expect("request task")
                .ready()
                .expect("served");
            assert_eq!(snapshot.total_resources, 2, "every waiter sees the refresh");
        }
        assert_eq!(provider.hits(), 2, "single-flight: one refresh compute");
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
                generated_at: DateTime::from_timestamp(1_752_454_800, 0),
                approximate: false,
                series_pending: false,
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
        // A provider's own "as of" stamp is kept, not overwritten by the cache.
        assert_eq!(
            snap.generated_at,
            DateTime::from_timestamp(1_752_454_800, 0)
        );
        assert!(!snap.series_pending);

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
            LIVE_TTL,
            LIVE_WAIT,
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
            LIVE_TTL,
            LIVE_WAIT,
        )
        .await
        .ready()
        .expect("cold load, flag on — a separate cache entry");
        assert_eq!(
            with.total_resources, 2,
            "not served from the flag-off entry"
        );
    }

    /// Slow only for one window — the 1h scan behind a large import — and
    /// instant for the others. Answers its n-th compute with `n` as
    /// `total_resources`, one charted series, and `approximate` set on the 24h
    /// window only, so a borrowed snapshot's provenance is traceable.
    struct SlowWindow {
        hits: AtomicUsize,
        slow: DashboardWindow,
        delay: Duration,
    }

    impl SlowWindow {
        fn new(slow: DashboardWindow, delay: Duration) -> Arc<Self> {
            Arc::new(SlowWindow {
                hits: AtomicUsize::new(0),
                slow,
                delay,
            })
        }
    }

    #[async_trait]
    impl DashboardProvider for SlowWindow {
        async fn snapshot(
            &self,
            window: DashboardWindow,
            _tenant: &str,
            _types: &[String],
            _include_empty: bool,
        ) -> DashboardSnapshot {
            let hit = self.hits.fetch_add(1, Ordering::SeqCst) + 1;
            if window == self.slow {
                tokio::time::sleep(self.delay).await;
            }
            DashboardSnapshot {
                total_resources: hit as u64,
                distinct_types: 1,
                window,
                series: vec![DashboardSeries {
                    resource_type: "Patient".to_string(),
                    total: hit as u64,
                    points: Vec::new(),
                }],
                available: vec![TypeCount {
                    resource_type: "Patient".to_string(),
                    total: hit as u64,
                }],
                approximate: window == DashboardWindow::LastDay,
                ..DashboardSnapshot::default()
            }
        }
    }

    /// One read with every cache-key component explicit.
    #[allow(clippy::too_many_arguments)]
    async fn read_key(
        cache: &SnapCache,
        provider: Arc<dyn DashboardProvider>,
        window: DashboardWindow,
        tenant: &str,
        types: &[&str],
        include_empty: bool,
        timings: Timings,
    ) -> SnapshotState {
        let types: Vec<String> = types.iter().map(|t| t.to_string()).collect();
        snapshot_via(
            cache.clone(),
            provider,
            window,
            tenant,
            &types,
            include_empty,
            timings.ttl,
            timings.cold,
            timings.compute,
            timings.live_ttl,
            timings.live_wait,
        )
        .await
    }

    /// #1078: switching to a window whose compute outlasts the cold wait no
    /// longer blanks the whole page. The tenant's totals are already cached
    /// under another window, so they are served — as of when they were read —
    /// with the series marked pending instead of drawn empty, and the real
    /// compute still lands under its own key.
    #[tokio::test]
    async fn cold_key_with_a_warm_sibling_serves_its_figures_with_the_series_pending() {
        let cache = SnapCache::default();
        let provider = SlowWindow::new(DashboardWindow::LastHour, Duration::from_millis(300));
        let timings = Timings {
            ttl: Duration::from_secs(60),
            cold: Duration::from_millis(50),
            compute: TEST_COMPUTE_TIMEOUT,
            live_ttl: LIVE_TTL,
            live_wait: LIVE_WAIT,
        };

        // Same selection as the cold read below, computed first.
        let same_types = read_key(
            &cache,
            provider.clone(),
            DashboardWindow::LastDay,
            "default",
            &["Patient"],
            false,
            timings,
        )
        .await
        .ready()
        .expect("instant window");
        assert_eq!(same_types.total_resources, 1);
        // Another selection, computed later: fresher, but a worse match.
        let fresher = read_key(
            &cache,
            provider.clone(),
            DashboardWindow::LastMonth,
            "default",
            &[],
            false,
            timings,
        )
        .await
        .ready()
        .expect("instant window");
        assert_eq!(fresher.total_resources, 2);

        let borrowed = read_key(
            &cache,
            provider.clone(),
            DashboardWindow::LastHour,
            "default",
            &["Patient"],
            false,
            timings,
        )
        .await
        .ready()
        .expect("a warm sibling answers the cold window");
        assert!(borrowed.series_pending, "the chart must render as waiting");
        assert!(borrowed.series.is_empty(), "no borrowed or invented series");
        assert_eq!(borrowed.window, DashboardWindow::LastHour);
        assert_eq!(
            borrowed.total_resources, 1,
            "the same selection is preferred over a fresher one"
        );
        assert_eq!(borrowed.distinct_types, same_types.distinct_types);
        assert_eq!(borrowed.available.len(), same_types.available.len());
        assert!(borrowed.approximate, "the sibling's qualifiers are kept");
        assert!(!borrowed.partial);
        assert_eq!(
            borrowed.generated_at, same_types.generated_at,
            "\"as of\" is the sibling's read, not now"
        );

        let hour_key = (
            DashboardWindow::LastHour,
            "default".to_string(),
            "Patient".to_string(),
            false,
        );
        assert_eq!(
            observe(&cache, &hour_key).total,
            None,
            "the borrowed snapshot is never cached as the window's own"
        );

        assert!(
            eventually(|| observe(&cache, &hour_key).total.is_some()).await,
            "the window's real compute still lands"
        );
        let real = read_key(
            &cache,
            provider.clone(),
            DashboardWindow::LastHour,
            "default",
            &["Patient"],
            false,
            timings,
        )
        .await
        .ready()
        .expect("cached");
        assert!(!real.series_pending);
        assert_eq!(real.series.len(), 1);
        assert_eq!(real.window, DashboardWindow::LastHour);
        assert_eq!(real.total_resources, 3);
    }

    /// A sibling has to describe the same tenant under the same "View all
    /// resources" setting; with none, a cold key is still honestly pending
    /// (#956).
    #[tokio::test]
    async fn cold_key_without_a_sibling_is_still_pending() {
        let cache = SnapCache::default();
        let provider = SlowWindow::new(DashboardWindow::LastHour, Duration::from_millis(300));
        let timings = Timings {
            ttl: Duration::from_secs(60),
            cold: Duration::from_millis(50),
            compute: TEST_COMPUTE_TIMEOUT,
            live_ttl: LIVE_TTL,
            live_wait: LIVE_WAIT,
        };

        let fresh_cache_cold = read_key(
            &cache,
            provider.clone(),
            DashboardWindow::LastHour,
            "default",
            &[],
            false,
            timings,
        )
        .await;
        assert!(
            matches!(fresh_cache_cold, SnapshotState::Pending),
            "nothing cached at all: {fresh_cache_cold:?}"
        );

        read_key(
            &cache,
            provider.clone(),
            DashboardWindow::LastMonth,
            "default",
            &[],
            false,
            timings,
        )
        .await
        .ready()
        .expect("instant window");

        let other_tenant = read_key(
            &cache,
            provider.clone(),
            DashboardWindow::LastHour,
            "other",
            &[],
            false,
            timings,
        )
        .await;
        assert!(
            matches!(other_tenant, SnapshotState::Pending),
            "another tenant's figures are never borrowed: {other_tenant:?}"
        );

        let other_toggle = read_key(
            &cache,
            provider.clone(),
            DashboardWindow::LastHour,
            "default",
            &[],
            true,
            timings,
        )
        .await;
        assert!(
            matches!(other_toggle, SnapshotState::Pending),
            "the other \"View all resources\" setting is not a sibling: {other_toggle:?}"
        );
    }

    /// A provider that leaves `generated_at` unset has it stamped by the cache
    /// when the value is written, so a served snapshot always says when its
    /// figures are from (#1078).
    #[tokio::test]
    async fn the_cache_stamps_generated_at_when_the_provider_leaves_it_unset() {
        let cache = SnapCache::default();
        let provider = Counting::new(Duration::ZERO);
        let before = Utc::now();

        let snap = snapshot_via(
            cache,
            provider,
            DashboardWindow::LastDay,
            "default",
            &[],
            false,
            Duration::from_secs(60),
            Duration::from_millis(800),
            TEST_COMPUTE_TIMEOUT,
            LIVE_TTL,
            LIVE_WAIT,
        )
        .await
        .ready()
        .expect("cold load fills within the wait");

        let stamped = snap.generated_at.expect("the cache fills generated_at");
        assert!(
            before <= stamped && stamped <= Utc::now(),
            "stamped at compute completion: {stamped}"
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
