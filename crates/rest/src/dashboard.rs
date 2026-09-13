//! Storage-backed dashboard data provider for the web UI.
//!
//! The web UI's landing page renders a "FHIR resources over time" chart plus a
//! few headline totals. The data lives behind [`ResourceStorage`], which the UI
//! crate deliberately does not depend on. Instead, this module implements
//! [`helios_observability::dashboard::DashboardProvider`] over the live backend
//! and registers it in [`crate::build_app`]; the UI reads the resulting snapshot
//! through the storage-agnostic `helios-observability` registry.
//!
//! Figures are scoped per snapshot call to the requesting tenant (#344; the
//! server's default tenant is only the empty-id fallback), and are never
//! exported to the public Prometheus `/metrics` endpoint (per the design in
//! [`helios_observability::metrics`]). The same cumulative-bucketing helper
//! backs the authenticated `/console/metrics/resource-counts` JSON handler
//! ([`resource_count_series`]) and both of the provider's read paths, so those
//! semantics live in exactly one place.
//!
//! # Two read paths (#1078)
//!
//! Computing a snapshot from storage means a `GROUP BY` over every live row of
//! the tenant plus one bucketed history scan per charted type. During a
//! multi-million-resource import those queries stop finishing within any page
//! budget, so the chart sat on "Waiting for the live figures…" exactly when an
//! operator most wanted to watch it move.
//!
//! - **Seeded tenant — served from memory.** Once a tenant's totals have been
//!   read from storage at least once, every snapshot is built from the
//!   process-global [`DashboardCounters`] that the REST write handlers and
//!   `$bulk-submit` record into. No storage aggregate runs on a page load; the
//!   only storage reads left are the small job-table counts, cached per tenant
//!   for [`JOB_COUNTS_TTL`]. Figures the counters cannot vouch for (writes
//!   recorded since the last reconcile, or a window whose history ring has not
//!   been loaded from storage yet) are flagged
//!   [`DashboardSnapshot::approximate`] — measured, never invented (#956). A
//!   charted ring that lacks storage history is queued for a background seed,
//!   never seeded inline.
//! - **Unseeded tenant — read from storage.** The first view of a tenant takes
//!   the original storage path, which doubles as the lazy seed: its
//!   `count_all_types` read reconciles the counters' totals and each per-type
//!   history read seeds that window's ring. Only successful reads seed; a failed
//!   read leaves the tenant unseeded, so the next view tries storage again.
//!
//! # Background reconcile
//!
//! [`spawn_reconcile_loop`] (wired next to the provider registration in
//! [`crate::build_app`]) keeps the counters honest: it seeds the default tenant
//! at startup (one background `GROUP BY` plus the history rings of its default
//! charted types — at a very large store that startup read takes as long as a
//! cold dashboard load used to, but nobody waits on it), then every
//! [`reconcile interval`](DEFAULT_RECONCILE_INTERVAL) re-reads each seeded
//! tenant's totals, backing off while a bulk submit is active for the tenant
//! and never spending more than about 1/[`RECONCILE_DUTY_FACTOR`] of the time
//! on one tenant's grouping query. When a tenant is quiet (no write recorded
//! since its totals reconcile began) the history rings of its charted types
//! that are no longer exact are re-seeded, after which the snapshot converges
//! to exact storage figures and drops its approximate label.
//!
//! # Counters are process-local
//!
//! A multi-instance deployment sharing one PostgreSQL or MongoDB database only
//! records the writes that land on *this* instance. Writes made through a
//! sibling instance, direct database edits, and the write paths that do not
//! record (conformance seeding, history deletes) show up only after the next
//! reconcile; until then the snapshot is labelled approximate whenever this
//! instance saw writes, and may be quietly behind when it saw none.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};
use std::time::{Duration as StdDuration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use helios_observability::dashboard::{
    DashboardPoint, DashboardProvider, DashboardSeries, DashboardSnapshot, DashboardWindow,
    ExportJobCounts, TypeCount,
};
use helios_observability::dashboard_counters::{self, DashboardCounters};
use helios_persistence::core::{
    BulkExportJobStore, BulkSubmitJobStore, ExportStatus, ResourceStorage, bucket_floor,
};
use helios_persistence::error::StorageResult;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use tokio::sync::Notify;
use tracing::{debug, warn};

use crate::config::ServerConfig;

/// How many series the dashboard charts when the user has not picked any:
/// the tenant's largest types, enough to compare without turning to spaghetti.
const DEFAULT_CHARTED_TYPES: usize = 3;
/// Definitional/infrastructure types the *default* selection skips — a server
/// that seeds its own SearchParameters would otherwise chart those instead of
/// the tenant's clinical data. They stay in `available`, so the picker still
/// offers them; this only shapes the out-of-the-box view.
const INFRASTRUCTURE_TYPES: &[&str] = &[
    "CapabilityStatement",
    "CodeSystem",
    "CompartmentDefinition",
    "ConceptMap",
    "ImplementationGuide",
    "OperationDefinition",
    "SearchParameter",
    "StructureDefinition",
    "Subscription",
    "ValueSet",
];
/// Hard cap on a user selection — matches the palette (six series colors) and
/// bounds the per-type delta queries. The default stays at three; this is how
/// far an explicit selection can go.
const MAX_CHARTED_TYPES: usize = 6;

/// How long a tenant's per-type totals (`count_all_types`) are reused before
/// the grouping query is re-run (#959).
///
/// The per-type totals are a single `GROUP BY` over every live row of the
/// tenant — the dominant cost of a dashboard load at scale (~30s at 6M rows on
/// SQLite) — and they do not depend on `window`, `types` or `include_empty` at
/// all. The observability layer, however, caches whole snapshots keyed on
/// `(window, tenant, types, include_empty)`, so the *same* figure is asked for
/// once per sibling key: window flips, picker changes, and the type rails on
/// `/ui/resources`, `/ui/search` and `/ui/queries` each used to pay for their
/// own scan. This cache collapses those duplicates into one.
///
/// Since #1078 it only matters on the storage path (an unseeded tenant); a
/// seeded tenant is served from the write counters and never reads it.
///
/// It must stay *below* the observability layer's 15s snapshot TTL, and that
/// bound is not a matter of taste. This entry is only ever refreshed as a side
/// effect of a snapshot recompute, and a given snapshot key recomputes at most
/// every 15s — so as long as this TTL is shorter than that, the entry is always
/// already expired by the time that key comes back, and the recompute sees the
/// current numbers. The cache is then invisible to freshness while still
/// absorbing every *sibling* key that asks in between.
///
/// Set it above 15s and the relationship inverts: a recompute starts serving
/// itself a value cached under some other key, and this becomes the term that
/// decides how long the headline "total resources" card, the "distinct types"
/// card and the type picker's option list keep showing pre-import numbers.
const TYPE_COUNTS_TTL: StdDuration = StdDuration::from_secs(5);

/// How long a tenant's bulk-export and bulk-submit job counts are reused
/// (#1078). They are small job-table reads, but every page load (and every
/// sibling snapshot key) asked for them, which during an import is exactly the
/// traffic the database can least afford. Below the snapshot layer's 15s TTL
/// for the same reason as [`TYPE_COUNTS_TTL`].
const JOB_COUNTS_TTL: StdDuration = StdDuration::from_secs(5);

/// Default pause between background reconcile passes (#1078).
///
/// Overridable with `HFS_DASHBOARD_RECONCILE_SECS` (whole seconds, > 0). A
/// pass only runs a tenant's grouping query when that tenant is due (see
/// [`RECONCILE_DUTY_FACTOR`]), so a short interval does not by itself make a
/// large store scan more often.
const DEFAULT_RECONCILE_INTERVAL: StdDuration = StdDuration::from_secs(30);

/// Environment variable overriding [`DEFAULT_RECONCILE_INTERVAL`].
const RECONCILE_INTERVAL_ENV: &str = "HFS_DASHBOARD_RECONCILE_SECS";

/// Duty-cycle bound on a tenant's totals reconcile: after a `count_all_types`
/// that took `t`, the tenant is not reconciled again for
/// `max(interval, t × RECONCILE_DUTY_FACTOR)`. A 30s grouping query at 6M rows
/// therefore runs at most every five minutes, so the reconcile never becomes a
/// standing load on a busy backend — including during a REST-driven load that
/// the bulk-submit back-off cannot see.
const RECONCILE_DUTY_FACTOR: u32 = 10;

/// How long the reconcile loop waits after being woken for a ring seed before
/// draining the queue, so a page that charts several types (or a burst of
/// window switches) is seeded in one batch.
const RING_SEED_DEBOUNCE: StdDuration = StdDuration::from_millis(250);

/// How long a `(tenant, type, window)` stays on the re-seed list after it was
/// last charted. Bounds the list by recent user behaviour rather than by every
/// type anyone ever picked.
const CHARTED_KEY_TTL: StdDuration = StdDuration::from_secs(60 * 60);

/// A span to chart, and the bucket width that samples it.
///
/// The UI builds one from a [`DashboardWindow`] preset; the console
/// `resource-counts` endpoint builds a daily one from its `days` parameter, so
/// its JSON keeps its calendar-day shape.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SeriesWindow {
    /// Width of one bucket, in seconds. Always positive.
    bucket_seconds: i64,
    /// Number of buckets plotted, ending with the one containing `now`.
    points: usize,
}

impl SeriesWindow {
    /// The window behind a UI preset.
    pub(crate) fn from_dashboard_window(window: DashboardWindow) -> Self {
        Self {
            bucket_seconds: window.bucket_seconds(),
            points: window.points(),
        }
    }

    /// `days` calendar-day buckets — the console endpoint's shape. Day-width
    /// buckets are epoch-aligned, so they coincide with UTC calendar days.
    pub(crate) fn days(days: i64) -> Self {
        Self {
            bucket_seconds: 86_400,
            points: days.max(1) as usize,
        }
    }

    /// Starts of the first and last buckets plotted as of `now`: the window
    /// ends with the (partial) bucket `now` falls in, and runs back
    /// `points - 1` whole buckets from there.
    fn bounds(self, now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
        let last_bucket = bucket_floor(now, self.bucket_seconds);
        let first_bucket = last_bucket
            - Duration::seconds(self.bucket_seconds) * (self.points.saturating_sub(1) as i32);
        (first_bucket, last_bucket)
    }
}

/// Builds a dense cumulative growth curve for each requested resource type, from
/// the immutable history log.
///
/// For each type it returns the current `total` plus `window.points` dense
/// buckets ending with the bucket containing `now`. Each point carries the net
/// change recorded in that bucket (`delta` — creations minus deletions, so it may
/// be negative) and the running total through it (`cumulative`).
///
/// # Why history, and not the current rows
///
/// The obvious source — bucketing the live `resources` rows by `last_updated`,
/// as [`ResourceStorage::count_by_day`] does — cannot support sub-day buckets
/// honestly: each resource sits in the bucket of its *most recent* edit, so
/// editing an old resource silently moves it into today's bucket and the past
/// changes shape under you. Aggregating `resource_history` instead counts the
/// write events themselves, which are immutable, so the curve is stable and
/// stays meaningful as the buckets get finer. See
/// [`ResourceStorage::count_deltas_by_bucket`].
///
/// The curve starts from a baseline of `total - (net change inside the window)`
/// and sums deltas forward, so its final point equals the live
/// [`ResourceStorage::count`] exactly. Anything history cannot reconstruct — a
/// resurrecting `PUT` after a delete, or resources predating the history log —
/// lands in that baseline instead of skewing the endpoint.
///
/// This is the shared implementation behind both the console `resource-counts`
/// JSON endpoint and the web UI dashboard provider's storage path; the
/// provider's counter path builds its curves with the same
/// [`cumulative_series`].
pub(crate) async fn resource_count_series<S>(
    storage: &S,
    tenant: &TenantContext,
    types: &[&str],
    window: SeriesWindow,
    now: DateTime<Utc>,
) -> StorageResult<Vec<DashboardSeries>>
where
    S: ResourceStorage + Sync,
{
    resource_count_series_seeding(storage, tenant, types, window, now, None).await
}

/// Where the storage path's history reads also seed the write counters: the
/// counters, and the dashboard preset whose ring the reads describe.
#[derive(Clone, Copy)]
struct RingSeeding<'a> {
    counters: &'a DashboardCounters,
    window: DashboardWindow,
}

/// [`resource_count_series`], optionally seeding each type's history ring in
/// `seeding.counters` from the very read that builds its curve (#1078).
///
/// Each type's ring seed is begun right before its history read and finished
/// only when that read succeeded, so a failed read never loads a ring. The
/// returned series are identical with or without seeding.
async fn resource_count_series_seeding<S>(
    storage: &S,
    tenant: &TenantContext,
    types: &[&str],
    window: SeriesWindow,
    now: DateTime<Utc>,
    seeding: Option<RingSeeding<'_>>,
) -> StorageResult<Vec<DashboardSeries>>
where
    S: ResourceStorage + Sync,
{
    let (first_bucket, last_bucket) = window.bounds(now);
    let tenant_key = tenant.tenant_id().as_str();

    // One batched call for every per-type total, instead of a round-trip per
    // type. A type with no stored resources simply has no row (counts as 0).
    let totals_by_type: HashMap<String, u64> = storage
        .count_by_types(tenant, types)
        .await?
        .into_iter()
        .collect();

    let mut series = Vec::with_capacity(types.len());
    for &rt in types {
        let total = totals_by_type.get(rt).copied().unwrap_or(0);

        let token = seeding.map(|s| s.counters.begin_ring_seed(tenant_key, rt, s.window));
        let deltas = storage
            .count_deltas_by_bucket(tenant, rt, first_bucket, window.bucket_seconds)
            .await?;

        // Keep only buckets inside the window (defensive against a
        // clock-skewed, future-dated `last_updated`).
        let in_window: Vec<(DateTime<Utc>, i64)> = deltas
            .iter()
            .filter(|d| d.bucket_start >= first_bucket && d.bucket_start <= last_bucket)
            .map(|d| (d.bucket_start, d.delta))
            .collect();

        if let (Some(seeding), Some(token)) = (seeding, token) {
            seeding.counters.finish_ring_seed(token, &in_window, now);
        }

        series.push(cumulative_series(rt, total, window, now, in_window));
    }

    Ok(series)
}

/// Turns one type's `total` and its bucketed net changes into the dense
/// cumulative curve [`resource_count_series`] documents — the single place the
/// baseline/cumulative arithmetic lives, shared by the storage path and the
/// write-counter path so both chart identical points for identical data.
///
/// Deltas outside the window as of `now` are ignored; several deltas for one
/// bucket are summed.
fn cumulative_series(
    resource_type: &str,
    total: u64,
    window: SeriesWindow,
    now: DateTime<Utc>,
    deltas: impl IntoIterator<Item = (DateTime<Utc>, i64)>,
) -> DashboardSeries {
    let (first_bucket, last_bucket) = window.bounds(now);
    let bucket_span = Duration::seconds(window.bucket_seconds);

    // Collapse into a bucket -> delta map, keeping only buckets inside the
    // window.
    let mut by_bucket: HashMap<DateTime<Utc>, i64> = HashMap::new();
    let mut net_in_window: i64 = 0;
    for (bucket_start, delta) in deltas {
        if bucket_start >= first_bucket && bucket_start <= last_bucket {
            *by_bucket.entry(bucket_start).or_insert(0) += delta;
            net_in_window += delta;
        }
    }

    // Resources already stored when the window opened form the baseline. It is
    // clamped at zero: a history log that is missing creations (e.g. rows that
    // predate it) could otherwise imply a negative starting count.
    let baseline = (total as i64 - net_in_window).max(0);

    let mut points = Vec::with_capacity(window.points);
    let mut cumulative = baseline;
    for i in 0..window.points {
        let bucket_start = first_bucket + bucket_span * (i as i32);
        let delta = by_bucket.get(&bucket_start).copied().unwrap_or(0);
        cumulative = (cumulative + delta).max(0);
        points.push(DashboardPoint {
            bucket_start,
            delta,
            cumulative: cumulative as u64,
        });
    }

    DashboardSeries {
        resource_type: resource_type.to_string(),
        total,
        points,
    }
}

/// One charted series built from the write counters, with how far the
/// counters can vouch for it.
struct CountedSeries {
    series: DashboardSeries,
    /// The window's ring holds storage history, not just recorded writes.
    history_seeded: bool,
    /// Nothing was written to the type since its ring and totals were read.
    exact: bool,
}

/// The counter-path twin of [`resource_count_series`]: the same curves, built
/// from `counters` instead of storage. `None` when the tenant has never been
/// reconciled from storage (the counters have no trustworthy base, #956).
fn resource_count_series_from_counters(
    counters: &DashboardCounters,
    tenant: &str,
    window: DashboardWindow,
    types: &[&str],
    now: DateTime<Utc>,
) -> Option<Vec<CountedSeries>> {
    let series_window = SeriesWindow::from_dashboard_window(window);
    let views = counters.series_view(tenant, window, types, now)?;
    Some(
        views
            .into_iter()
            .map(|view| CountedSeries {
                series: cumulative_series(
                    &view.resource_type,
                    view.total,
                    series_window,
                    now,
                    view.buckets,
                ),
                history_seeded: view.history_seeded,
                exact: view.exact,
            })
            .collect(),
    )
}

/// The headline figures and picker list derived from a tenant's per-type
/// totals — identical whether the totals came from storage or the counters.
struct TypeSummary {
    total_resources: u64,
    distinct_types: usize,
    available: Vec<TypeCount>,
}

/// Derives [`TypeSummary`] from per-type live totals.
fn summarize_type_counts(raw_counts: Vec<(String, u64)>, include_empty: bool) -> TypeSummary {
    // The stat card counts only types the tenant actually stores —
    // `include_empty` (#599, "View all resources") never changes this
    // figure, so it must be taken before the flag relaxes the filter
    // below.
    let distinct_types = raw_counts.iter().filter(|(_, total)| *total > 0).count();
    // The headline total is *derived* from the per-type counts rather than
    // read with a second `storage.count(&tenant, None)` (#959). That call was
    // a full `COUNT(*)` over exactly the rows `count_all_types` had just
    // grouped and counted — roughly doubling the page's cost at 6M resources.
    //
    // The two figures agree by contract: `ResourceStorage::count` with `None`
    // returns "the count of non-deleted resources" for the tenant, and
    // `count_all_types` "counts non-deleted resources grouped by resource type
    // for `tenant`, returning one `(resource_type, count)` pair per type
    // present". The SQL backends use literally the same predicate for both
    // (`tenant_id = ? AND is_deleted = 0/FALSE`), and `CompositeStorage`
    // delegates both to its primary — so summing the groups reproduces the
    // ungrouped count exactly.
    //
    // When `count_all_types` fails with nothing cached, `total_resources` is
    // 0 and the snapshot is flagged partial (#956). Summed saturating so a
    // pathological backend cannot panic the dashboard on overflow.
    let total_resources: u64 = raw_counts
        .iter()
        .map(|(_, total)| *total)
        .fold(0u64, u64::saturating_add);

    // With `include_empty`, a type reported with a zero live count is kept
    // instead of dropped, coherent with the selection guard in
    // [`select_charted_types`] (which also stops requiring a requested type to
    // already be stored). Storage backends only ever return types with at
    // least one live row (`GROUP BY` over non-deleted rows); the write counters
    // may also know a type whose resources were all deleted. A type the tenant
    // has *never* stored still isn't known to this provider at all: the union
    // with the FHIR version's full type list (so the picker can offer those
    // too, at 0) is done on the UI side against `resource_type_names()`, the
    // same spec-derived source the other pickers use; see
    // `helios_ui::build_dashboard`.
    let mut available: Vec<TypeCount> = raw_counts
        .into_iter()
        .filter(|(_, total)| include_empty || *total > 0)
        .map(|(resource_type, total)| TypeCount {
            resource_type,
            total,
        })
        .collect();
    available.sort_by(|a, b| {
        b.total
            .cmp(&a.total)
            .then_with(|| a.resource_type.cmp(&b.resource_type))
    });

    TypeSummary {
        total_resources,
        distinct_types,
        available,
    }
}

/// The charted set: the caller's selection filtered to real stored types,
/// else the largest few. Capped so the query fan-out (one delta aggregate per
/// type on the storage path) and the palette stay bounded.
fn select_charted_types<'a>(
    available: &'a [TypeCount],
    types: &'a [String],
    include_empty: bool,
) -> Vec<&'a str> {
    if types.is_empty() {
        let mut defaults: Vec<&str> = available
            .iter()
            .filter(|t| !INFRASTRUCTURE_TYPES.contains(&t.resource_type.as_str()))
            .take(DEFAULT_CHARTED_TYPES)
            .map(|t| t.resource_type.as_str())
            .collect();
        // A store holding nothing but definitional resources still charts.
        if defaults.is_empty() {
            defaults = available
                .iter()
                .take(DEFAULT_CHARTED_TYPES)
                .map(|t| t.resource_type.as_str())
                .collect();
        }
        defaults
    } else {
        types
            .iter()
            // With `include_empty`, a requested type need not already be
            // stored: the UI only ever sends one it validated against the
            // version's real type list, and an unstored type still charts
            // cleanly (a flat zero series rather than an error or an omission).
            .filter(|t| include_empty || available.iter().any(|a| &a.resource_type == *t))
            .take(MAX_CHARTED_TYPES)
            .map(|t| t.as_str())
            .collect()
    }
}

/// A full-access context for `tenant_id`: the dashboard reads aggregate
/// figures on the server's own behalf.
fn tenant_context(tenant_id: &str) -> TenantContext {
    TenantContext::new(
        TenantId::new(tenant_id.to_string()),
        TenantPermissions::full_access(),
    )
}

/// Locks a mutex, recovering the data if a panicking holder poisoned it. Every
/// map guarded this way is a cache or a work queue whose worst torn state is a
/// redundant read.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Per-tenant `count_all_types` results, each stamped with the instant it was
/// computed so [`TYPE_COUNTS_TTL`] can be applied on read (#959).
///
/// A `std::sync::RwLock` rather than an async lock on purpose: it is only ever
/// taken for a synchronous map read or insert and released before the next
/// `.await`, so it never blocks the runtime (and never trips clippy's
/// `await_holding_lock`). Bounded by the number of tenants the dashboard is
/// viewed for.
type TypeCountCache = Arc<RwLock<HashMap<String, (Instant, Vec<(String, u64)>)>>>;

/// A tenant's cached job counts (see [`JOB_COUNTS_TTL`]).
#[derive(Clone, Copy, Default)]
struct JobCountsEntry {
    /// When the stores were last asked; `None` before the first read.
    fetched_at: Option<Instant>,
    /// Last successfully read export counts.
    export: Option<ExportJobCounts>,
    /// Last successfully read active-submission count.
    import: Option<u64>,
}

/// `(tenant, resource type, window)` — one history ring of the write counters.
type RingKey = (String, String, DashboardWindow);

/// The history rings waiting for, or worth, a background seed.
#[derive(Default)]
struct RingSeedQueue {
    /// Rings a snapshot charted without storage history, awaiting their first
    /// seed. A set, so a key is queued (and the loop woken) once however many
    /// page loads ask for it.
    pending: Mutex<HashSet<RingKey>>,
    /// Rings charted recently, with when — what a quiet reconcile re-seeds
    /// once they stop being exact (see [`CHARTED_KEY_TTL`]).
    charted: Mutex<HashMap<RingKey, Instant>>,
    /// Wakes the reconcile loop when `pending` gains a key.
    wake: Arc<Notify>,
}

/// What one reconcile pass did, for its debug log and for tests.
#[derive(Debug, Default)]
pub(crate) struct ReconcileReport {
    /// Tenants whose totals were re-read from storage.
    pub(crate) reconciled: Vec<String>,
    /// Seeded tenants skipped because a bulk submit is active for them.
    pub(crate) skipped_active_import: Vec<String>,
    /// Seeded tenants whose totals reconcile is not due yet (duty cycle).
    pub(crate) not_due: Vec<String>,
    /// History rings loaded from storage during the pass.
    pub(crate) rings_seeded: usize,
}

/// When each tenant's totals may next be re-read (see
/// [`RECONCILE_DUTY_FACTOR`]). Owned by the reconcile loop.
pub(crate) struct ReconcileSchedule {
    interval: StdDuration,
    next_due: HashMap<String, Instant>,
}

impl ReconcileSchedule {
    pub(crate) fn new(interval: StdDuration) -> Self {
        Self {
            interval,
            next_due: HashMap::new(),
        }
    }

    fn is_due(&self, tenant: &str) -> bool {
        self.next_due
            .get(tenant)
            .is_none_or(|due| Instant::now() >= *due)
    }

    /// Records a totals read of `elapsed` that just finished for `tenant`.
    fn record(&mut self, tenant: &str, elapsed: StdDuration) {
        let pause = self
            .interval
            .max(elapsed.saturating_mul(RECONCILE_DUTY_FACTOR));
        self.next_due
            .insert(tenant.to_string(), Instant::now() + pause);
    }
}

/// Which caller ran a totals read, for its debug log.
#[derive(Clone, Copy)]
enum TotalsReadSource {
    /// A page load on an unseeded tenant.
    Snapshot,
    /// The background reconcile loop.
    Reconcile,
}

/// [`DashboardProvider`] backed by a live storage backend. Registered once in
/// [`crate::build_app`]; the tenant to chart arrives per call (#344), with the
/// server default as the fallback for an empty id.
pub(crate) struct StorageDashboardProvider<S> {
    default_tenant: String,
    fhir_version: String,
    storage: Arc<S>,
    /// Bulk-export job store, when the active backend provides one.
    export_jobs: Option<Arc<dyn BulkExportJobStore>>,
    /// Bulk-submit job store, when the active backend provides one.
    submit_jobs: Option<Arc<dyn BulkSubmitJobStore>>,
    /// Per-tenant cache of `count_all_types` (see [`TypeCountCache`] and
    /// [`TYPE_COUNTS_TTL`], #959).
    type_counts: TypeCountCache,
    /// The live write counters a seeded tenant is served from (#1078). The
    /// process-global set in production; tests pass an isolated one.
    counters: &'static DashboardCounters,
    /// One async lock per tenant around its `count_all_types` read, so sibling
    /// snapshot keys of an unseeded tenant and the reconcile loop never run
    /// the grouping query concurrently: a waiter finds the fresh result in
    /// [`Self::type_counts`] instead (#1078).
    totals_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Per-tenant cache of the job-store counts (see [`JOB_COUNTS_TTL`]).
    job_counts: Mutex<HashMap<String, JobCountsEntry>>,
    /// Background ring seeds, drained by the reconcile loop.
    ring_seeds: RingSeedQueue,
}

impl<S> StorageDashboardProvider<S> {
    /// Builds a provider for the server's default tenant and default FHIR
    /// version. The window and the charted types are chosen per request by the
    /// UI, so neither is fixed here; the default selection is the tenant's
    /// largest stored types (#555). Job-store counts start unwired; call
    /// [`Self::with_job_stores`] to attach them. Reads the process-global
    /// write counters; see [`Self::with_counters`].
    pub(crate) fn new(storage: Arc<S>, config: &ServerConfig) -> Self {
        Self {
            default_tenant: config.default_tenant.clone(),
            fhir_version: config.default_fhir_version.to_string(),
            storage,
            export_jobs: None,
            submit_jobs: None,
            type_counts: Arc::new(RwLock::new(HashMap::new())),
            counters: dashboard_counters::global(),
            totals_locks: Mutex::new(HashMap::new()),
            job_counts: Mutex::new(HashMap::new()),
            ring_seeds: RingSeedQueue::default(),
        }
    }

    /// Attaches the bulk-export and bulk-submit job stores, when the running
    /// deployment has them. Passing `None` for either leaves the
    /// corresponding snapshot field at `None` (unavailable) rather than a
    /// fabricated zero.
    pub(crate) fn with_job_stores(
        mut self,
        export_jobs: Option<Arc<dyn BulkExportJobStore>>,
        submit_jobs: Option<Arc<dyn BulkSubmitJobStore>>,
    ) -> Self {
        self.export_jobs = export_jobs;
        self.submit_jobs = submit_jobs;
        self
    }

    /// Replaces the write counters this provider reads and seeds (the
    /// process-global set by default). Tests use an isolated set, so figures
    /// recorded by other tests in the same process cannot leak in.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_counters(mut self, counters: &'static DashboardCounters) -> Self {
        self.counters = counters;
        self
    }

    /// Notes that `types` were charted over `window` for `tenant`, so a quiet
    /// reconcile keeps their history rings exact.
    fn note_charted(&self, tenant: &str, types: &[&str], window: DashboardWindow) {
        if types.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut charted = lock(&self.ring_seeds.charted);
        for rt in types {
            charted.insert((tenant.to_string(), (*rt).to_string(), window), now);
        }
    }

    /// Queues a background seed of one history ring, waking the reconcile loop
    /// the first time the key is queued. Never reads storage.
    fn enqueue_ring_seed(&self, tenant: &str, resource_type: &str, window: DashboardWindow) {
        let inserted = lock(&self.ring_seeds.pending).insert((
            tenant.to_string(),
            resource_type.to_string(),
            window,
        ));
        if inserted {
            self.ring_seeds.wake.notify_one();
        }
    }

    /// Recently charted `(type, window)` rings of `tenant`, in a stable order.
    fn charted_for(&self, tenant: &str) -> Vec<(String, DashboardWindow)> {
        let mut rings: Vec<(String, DashboardWindow)> = lock(&self.ring_seeds.charted)
            .keys()
            .filter(|(t, _, _)| t == tenant)
            .map(|(_, rt, window)| (rt.clone(), *window))
            .collect();
        rings.sort_by(|a, b| (&a.0, a.1.as_str()).cmp(&(&b.0, b.1.as_str())));
        rings
    }

    /// The async lock serializing `tenant`'s `count_all_types` reads.
    fn totals_lock(&self, tenant: &str) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(
            lock(&self.totals_locks)
                .entry(tenant.to_string())
                .or_default(),
        )
    }

    /// The tenant's cached per-type totals, if younger than
    /// [`TYPE_COUNTS_TTL`].
    fn fresh_type_counts(&self, tenant: &str) -> Option<Vec<(String, u64)>> {
        self.type_counts.read().ok().and_then(|guard| {
            guard
                .get(tenant)
                .and_then(|(at, counts)| (at.elapsed() < TYPE_COUNTS_TTL).then(|| counts.clone()))
        })
    }
}

/// Wakes the reconcile loop, if one runs, so it notices at once that its
/// provider is gone instead of at its next tick.
impl<S> Drop for StorageDashboardProvider<S> {
    fn drop(&mut self) {
        self.ring_seeds.wake.notify_one();
    }
}

impl<S> StorageDashboardProvider<S>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    /// The tenant's per-type live totals, served from the per-tenant cache
    /// when it is fresher than [`TYPE_COUNTS_TTL`], else recomputed (#959).
    ///
    /// This is the storage path's single most expensive query — a `GROUP BY`
    /// over every live row of the tenant — and its result depends on nothing
    /// but the tenant, so it is cached independently of the observability
    /// layer's `(window, tenant, types, include_empty)` snapshot cache. A
    /// recompute also reconciles the write counters (see [`Self::read_totals`]),
    /// which is what moves the tenant onto the counter path (#1078).
    ///
    /// Degrades like the rest of the snapshot: on a storage error it logs and
    /// falls back to the stale entry when there is one (stale beats absent —
    /// the same principle the observability cache already applies to whole
    /// snapshots), otherwise to an empty list.
    ///
    /// The returned flag is that last case: an empty list is indistinguishable
    /// from an empty tenant, and since #959 derives *both* `distinct_types` and
    /// `total_resources` from this call, a failure with nothing to fall back on
    /// zeroes both stat cards. The page has to say those zeros are not
    /// measurements (#956). A stale fallback is deliberately not flagged: those
    /// figures were read from storage, just not now, which is the same trade
    /// the snapshot cache already makes when it serves a stale snapshot.
    async fn cached_count_all_types(&self, tenant: &TenantContext) -> (Vec<(String, u64)>, bool) {
        let tenant_key = tenant.tenant_id().as_str().to_string();

        if let Some(counts) = self.fresh_type_counts(&tenant_key) {
            debug!(
                tenant = %tenant_key,
                types = counts.len(),
                "dashboard snapshot: per-type counts served from cache"
            );
            return (counts, false);
        }

        // Single-flight per tenant: a sibling key (or the reconcile loop) may
        // already be running this very query. Wait for it and reuse its
        // result rather than stacking a second scan on the backend.
        let totals_lock = self.totals_lock(&tenant_key);
        let _guard = totals_lock.lock().await;
        if let Some(counts) = self.fresh_type_counts(&tenant_key) {
            debug!(
                tenant = %tenant_key,
                types = counts.len(),
                "dashboard snapshot: per-type counts served from cache"
            );
            return (counts, false);
        }

        match self.read_totals(tenant, TotalsReadSource::Snapshot).await {
            Ok(counts) => (counts, false),
            Err(error) => {
                warn!(%error, "dashboard snapshot: distinct-type query failed");
                match self
                    .type_counts
                    .read()
                    .ok()
                    .and_then(|guard| guard.get(&tenant_key).map(|(_, counts)| counts.clone()))
                {
                    Some(stale) => (stale, false),
                    None => (Vec::new(), true),
                }
            }
        }
    }

    /// Runs `count_all_types` for `tenant`, bracketed as a counters reconcile
    /// (#1078), and refreshes [`Self::type_counts`] with the result.
    ///
    /// The reconcile is begun right before the query and finished only with
    /// its successful result, so an error never marks the tenant seeded. Must
    /// be called with the tenant's [`Self::totals_lock`] held.
    async fn read_totals(
        &self,
        tenant: &TenantContext,
        source: TotalsReadSource,
    ) -> StorageResult<Vec<(String, u64)>> {
        let tenant_key = tenant.tenant_id().as_str();
        let read_at = Utc::now();
        let token = self.counters.begin_reconcile(tenant_key);

        // Timed, so an operator staring at a slow dashboard can tell from the
        // logs *which* query is responsible (#959) — and whether a page load
        // or the background reconcile ran it (#1078).
        let started = Instant::now();
        let result = self.storage.count_all_types(tenant).await;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match source {
            TotalsReadSource::Snapshot => debug!(
                tenant = %tenant_key,
                elapsed_ms,
                ok = result.is_ok(),
                "dashboard snapshot: count_all_types completed"
            ),
            TotalsReadSource::Reconcile => debug!(
                tenant = %tenant_key,
                elapsed_ms,
                ok = result.is_ok(),
                "dashboard reconcile: count_all_types completed"
            ),
        }
        let counts = result?;

        if self.counters.finish_reconcile(token, &counts, read_at) {
            if let Ok(mut guard) = self.type_counts.write() {
                guard.insert(tenant_key.to_string(), (Instant::now(), counts.clone()));
            }
        } else {
            // The tenant was purged while the query ran: these figures may
            // describe erased data, so neither the counters nor the cache keep
            // them.
            debug!(
                tenant = %tenant_key,
                "dashboard: tenant invalidated during count_all_types; counters not seeded"
            );
        }
        Ok(counts)
    }

    /// The tenant's bulk-export and bulk-submit job counts, cached for
    /// [`JOB_COUNTS_TTL`].
    ///
    /// Counts degrade to the last successfully read value, or `None`
    /// (unavailable) when there is none, rather than zero on a read error: a
    /// zero here would tell an operator "no jobs" when the truth is "could not
    /// ask". `None` also covers the normal case of a deployment with no
    /// bulk-export/bulk-submit job store wired at all. They carry their own
    /// unavailable state on the page, so they never set `partial` — that flag
    /// is for figures with no honest rendering of their own.
    async fn job_counts(&self, tenant: &TenantContext) -> (Option<ExportJobCounts>, Option<u64>) {
        if self.export_jobs.is_none() && self.submit_jobs.is_none() {
            return (None, None);
        }
        let tenant_key = tenant.tenant_id().as_str();
        let cached = lock(&self.job_counts)
            .get(tenant_key)
            .copied()
            .unwrap_or_default();
        if cached
            .fetched_at
            .is_some_and(|at| at.elapsed() < JOB_COUNTS_TTL)
        {
            return (cached.export, cached.import);
        }

        let export = match &self.export_jobs {
            None => None,
            Some(store) => {
                let running = store
                    .count_exports_by_status(tenant, ExportStatus::InProgress)
                    .await;
                let queued = store
                    .count_exports_by_status(tenant, ExportStatus::Accepted)
                    .await;
                match (running, queued) {
                    (Ok(running), Ok(queued)) => Some(ExportJobCounts { running, queued }),
                    (Err(error), _) | (_, Err(error)) => {
                        warn!(%error, "dashboard snapshot: export job count query failed");
                        cached.export
                    }
                }
            }
        };

        let import = match &self.submit_jobs {
            None => None,
            Some(store) => match store.count_active_submissions(tenant).await {
                Ok(n) => Some(n),
                Err(error) => {
                    warn!(%error, "dashboard snapshot: import job count query failed");
                    cached.import
                }
            },
        };

        lock(&self.job_counts).insert(
            tenant_key.to_string(),
            JobCountsEntry {
                fetched_at: Some(Instant::now()),
                export,
                import,
            },
        );
        (export, import)
    }

    /// Whether a bulk submit is active for `tenant` — the reconcile's back-off
    /// signal. `false` when no submit store is wired or its count is unknown.
    async fn import_active(&self, tenant: &str) -> bool {
        if self.submit_jobs.is_none() {
            return false;
        }
        self.job_counts(&tenant_context(tenant))
            .await
            .1
            .is_some_and(|active| active > 0)
    }

    /// A snapshot built entirely from the write counters, or `None` when the
    /// tenant has never been reconciled from storage (#1078).
    ///
    /// Runs no storage aggregate: totals, the picker list and every charted
    /// series come from memory. Only the cached job counts may touch storage.
    async fn snapshot_from_counters(
        &self,
        tenant: &TenantContext,
        window: DashboardWindow,
        types: &[String],
        include_empty: bool,
    ) -> Option<DashboardSnapshot> {
        let started = Instant::now();
        let tenant_key = tenant.tenant_id().as_str();
        let now = Utc::now();

        let totals = self.counters.totals_view(tenant_key)?;
        let totals_exact = totals.exact;
        let TypeSummary {
            total_resources,
            distinct_types,
            available,
        } = summarize_type_counts(totals.totals, include_empty);
        let selection = select_charted_types(&available, types, include_empty);
        // `None` only if the tenant was invalidated since `totals_view`; the
        // caller then falls back to storage.
        let counted = resource_count_series_from_counters(
            self.counters,
            tenant_key,
            window,
            &selection,
            now,
        )?;
        self.note_charted(tenant_key, &selection, window);
        let charted_types = selection.len();

        // Approximate when a write was recorded since the totals or a charted
        // ring were last read from storage, or when a ring has no storage
        // history at all yet (#956: measured, but not an exact storage read).
        let mut approximate = !totals_exact;
        let mut awaiting_history = 0usize;
        let mut series = Vec::with_capacity(counted.len());
        for entry in counted {
            approximate |= !entry.exact;
            if !entry.history_seeded {
                awaiting_history += 1;
                self.enqueue_ring_seed(tenant_key, &entry.series.resource_type, window);
            }
            series.push(entry.series);
        }

        let (export_jobs, import_jobs_active) = self.job_counts(tenant).await;

        // The acceptance signal for #1078: a page load on a seeded tenant logs
        // this line and none of the storage-query timings.
        debug!(
            tenant = %tenant_key,
            window = window.as_str(),
            charted_types,
            approximate,
            awaiting_history,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "dashboard snapshot: served from write counters"
        );

        Some(DashboardSnapshot {
            fhir_version: self.fhir_version.clone(),
            total_resources,
            distinct_types,
            window,
            series,
            available,
            export_jobs,
            import_jobs_active,
            partial: false,
            // The counters are live: these are the figures as of this call.
            generated_at: Some(now),
            approximate,
            series_pending: false,
        })
    }

    /// A snapshot read from storage — the path of a tenant the counters have
    /// not been seeded for, and the read that seeds them (#1078).
    async fn snapshot_from_storage(
        &self,
        tenant: &TenantContext,
        window: DashboardWindow,
        types: &[String],
        include_empty: bool,
    ) -> DashboardSnapshot {
        let tenant_key = tenant.tenant_id().as_str();
        let now = Utc::now();

        // Set by every degradation below. A half-failed snapshot reads exactly
        // like a real one — empty series, zero totals — and is then cached as
        // truth, so it has to carry the fact that it is incomplete (#956).
        let mut partial = false;

        // What the tenant actually stores, largest first — the picker's option
        // list, and the pool defaults are drawn from (#555). Cached per tenant
        // (#959): this grouping query is the dashboard's dominant cost and does
        // not vary with the window or the selection. It reports whether it had
        // to fabricate its zeros, because both stat cards derive from it.
        let (raw_counts, counts_unavailable) = self.cached_count_all_types(tenant).await;
        partial |= counts_unavailable;
        let TypeSummary {
            total_resources,
            distinct_types,
            available,
        } = summarize_type_counts(raw_counts, include_empty);
        let selection = select_charted_types(&available, types, include_empty);
        self.note_charted(tenant_key, &selection, window);

        // Degrade to an empty/zeroed snapshot rather than surfacing an error —
        // the operator dashboard should render even if a count query hiccups —
        // but flag it, so the page says the figures are incomplete instead of
        // charting the fallback as data (#956).
        //
        // Timed at `debug!` alongside the per-type grouping so a slow dashboard
        // can be attributed to one query or the other without a profiler (#959).
        // Each type's history read also seeds that window's counter ring.
        let series_started = Instant::now();
        let series_result = resource_count_series_seeding(
            self.storage.as_ref(),
            tenant,
            &selection,
            SeriesWindow::from_dashboard_window(window),
            now,
            Some(RingSeeding {
                counters: self.counters,
                window,
            }),
        )
        .await;
        debug!(
            tenant = %tenant_key,
            window = window.as_str(),
            charted_types = selection.len(),
            elapsed_ms = series_started.elapsed().as_millis() as u64,
            ok = series_result.is_ok(),
            "dashboard snapshot: resource-count series completed"
        );
        let series = match series_result {
            Ok(series) => series,
            Err(error) => {
                warn!(%error, "dashboard snapshot: resource-count series query failed");
                partial = true;
                Vec::new()
            }
        };

        let (export_jobs, import_jobs_active) = self.job_counts(tenant).await;

        DashboardSnapshot {
            fhir_version: self.fhir_version.clone(),
            total_resources,
            distinct_types,
            window,
            series,
            available,
            export_jobs,
            import_jobs_active,
            partial,
            // Every figure above was read from storage starting at `now`, so
            // that is what the dashboard's "as of" reports (#1078).
            generated_at: Some(now),
            approximate: false,
            series_pending: false,
        }
    }

    /// Loads one history ring from storage in the background: the window's
    /// bucketed history read, bracketed as a ring seed. Returns whether the
    /// ring was seeded; a failed read logs and leaves the ring as it was.
    async fn seed_ring(&self, tenant: &str, resource_type: &str, window: DashboardWindow) -> bool {
        let context = tenant_context(tenant);
        let series_window = SeriesWindow::from_dashboard_window(window);
        let now = Utc::now();
        let (first_bucket, last_bucket) = series_window.bounds(now);

        let token = self.counters.begin_ring_seed(tenant, resource_type, window);
        let started = Instant::now();
        let result = self
            .storage
            .count_deltas_by_bucket(
                &context,
                resource_type,
                first_bucket,
                series_window.bucket_seconds,
            )
            .await;
        debug!(
            tenant = %tenant,
            resource_type,
            window = window.as_str(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            ok = result.is_ok(),
            "dashboard reconcile: history ring read completed"
        );
        match result {
            Ok(deltas) => {
                let in_window: Vec<(DateTime<Utc>, i64)> = deltas
                    .iter()
                    .filter(|d| d.bucket_start >= first_bucket && d.bucket_start <= last_bucket)
                    .map(|d| (d.bucket_start, d.delta))
                    .collect();
                self.counters.finish_ring_seed(token, &in_window, now)
            }
            Err(error) => {
                warn!(
                    %error,
                    tenant = %tenant,
                    resource_type,
                    window = window.as_str(),
                    "dashboard reconcile: history ring read failed; keeping the previous ring"
                );
                false
            }
        }
    }

    /// Reconciles `tenant`'s totals under its totals lock, recording the read
    /// in `schedule`.
    async fn reconcile_totals(
        &self,
        tenant: &str,
        schedule: &mut ReconcileSchedule,
    ) -> StorageResult<Vec<(String, u64)>> {
        let context = tenant_context(tenant);
        let totals_lock = self.totals_lock(tenant);
        let _guard = totals_lock.lock().await;
        let started = Instant::now();
        let result = self
            .read_totals(&context, TotalsReadSource::Reconcile)
            .await;
        schedule.record(tenant, started.elapsed());
        result
    }

    /// The reconcile loop's startup step: seeds the default tenant's totals
    /// and the history rings of its default charted types for every window,
    /// so the first dashboard view after a restart is already served from
    /// memory. At a very large store this is one background `GROUP BY` plus a
    /// few index-backed history reads; nobody waits on it.
    pub(crate) async fn seed_default_tenant(&self, schedule: &mut ReconcileSchedule) {
        let tenant = self.default_tenant.clone();
        let started = Instant::now();
        if let Err(error) = self.reconcile_totals(&tenant, schedule).await {
            warn!(
                %error,
                tenant = %tenant,
                "dashboard reconcile: startup seed of the default tenant failed; \
                 its first dashboard view will read storage instead"
            );
            return;
        }
        let Some(totals) = self.counters.totals_view(&tenant) else {
            return;
        };
        let summary = summarize_type_counts(totals.totals, false);
        let defaults: Vec<String> = select_charted_types(&summary.available, &[], false)
            .into_iter()
            .map(str::to_string)
            .collect();
        let mut rings_seeded = 0usize;
        for resource_type in &defaults {
            for window in DashboardWindow::ALL {
                self.note_charted(&tenant, &[resource_type.as_str()], window);
                if self.seed_ring(&tenant, resource_type, window).await {
                    rings_seeded += 1;
                }
            }
        }
        debug!(
            tenant = %tenant,
            charted_types = defaults.len(),
            rings_seeded,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "dashboard reconcile: default tenant seeded"
        );
    }

    /// Seeds the queued history rings, one at a time, leaving queued the rings
    /// of tenants with an active bulk submit. Returns how many were seeded.
    pub(crate) async fn drain_pending_ring_seeds(&self) -> usize {
        let mut keys: Vec<RingKey> = lock(&self.ring_seeds.pending).iter().cloned().collect();
        keys.sort_by(|a, b| (&a.0, &a.1, a.2.as_str()).cmp(&(&b.0, &b.1, b.2.as_str())));

        let mut import_active: HashMap<String, bool> = HashMap::new();
        let mut seeded = 0usize;
        for key in keys {
            let (tenant, resource_type, window) = &key;
            let active = match import_active.get(tenant) {
                Some(active) => *active,
                None => {
                    let active = self.import_active(tenant).await;
                    import_active.insert(tenant.clone(), active);
                    active
                }
            };
            if active {
                debug!(
                    tenant = %tenant,
                    resource_type = %resource_type,
                    window = window.as_str(),
                    "dashboard reconcile: bulk submit active; history ring seed deferred"
                );
                continue;
            }
            if self.seed_ring(tenant, resource_type, *window).await {
                seeded += 1;
            }
            // Dequeued only after the read, so a page load during it does not
            // queue the same ring again. A failed seed is dropped: the next
            // view of the ring queues it anew.
            lock(&self.ring_seeds.pending).remove(&key);
        }
        seeded
    }

    /// One reconcile pass over every seeded tenant in the counters (#1078).
    ///
    /// Sequential by design — one tenant, one query at a time — so the pass
    /// never competes much with writers. Per tenant:
    ///
    /// 1. **Back-off.** A tenant with an active bulk submit is skipped
    ///    entirely: its grouping query is exactly what the import keeps from
    ///    finishing, and the counters already follow the import.
    /// 2. **Totals.** When due (see [`RECONCILE_DUTY_FACTOR`]),
    ///    `count_all_types` is re-read as a counters reconcile. An error logs
    ///    and keeps the previous counters.
    /// 3. **Rings.** When the tenant is quiet — no write recorded since its
    ///    totals reconcile began — each recently charted ring that is not
    ///    exact is re-seeded, which is what lets the snapshot drop its
    ///    approximate label after an import.
    ///
    /// Finally the queued ring seeds are drained.
    pub(crate) async fn reconcile_pass(&self, schedule: &mut ReconcileSchedule) -> ReconcileReport {
        let started = Instant::now();
        let mut report = ReconcileReport::default();
        lock(&self.ring_seeds.charted).retain(|_, at| at.elapsed() < CHARTED_KEY_TTL);

        for tenant in self.counters.tenants() {
            if !self.counters.is_seeded(&tenant) {
                // Recorded writes but never viewed: its first view seeds it.
                continue;
            }
            if self.import_active(&tenant).await {
                debug!(
                    tenant = %tenant,
                    "dashboard reconcile: bulk submit active; skipping the tenant this pass"
                );
                report.skipped_active_import.push(tenant);
                continue;
            }

            if schedule.is_due(&tenant) {
                match self.reconcile_totals(&tenant, schedule).await {
                    Ok(_) => report.reconciled.push(tenant.clone()),
                    Err(error) => warn!(
                        %error,
                        tenant = %tenant,
                        "dashboard reconcile: count_all_types failed; keeping the previous counters"
                    ),
                }
            } else {
                debug!(
                    tenant = %tenant,
                    "dashboard reconcile: totals reconcile not due yet (duty-cycle back-off)"
                );
                report.not_due.push(tenant.clone());
            }

            let quiet = self
                .counters
                .totals_view(&tenant)
                .is_some_and(|totals| totals.exact);
            if !quiet {
                debug!(
                    tenant = %tenant,
                    "dashboard reconcile: writes in flight; history ring re-seed deferred"
                );
                continue;
            }
            for (resource_type, window) in self.charted_for(&tenant) {
                let exact = self
                    .counters
                    .series_view(&tenant, window, &[resource_type.as_str()], Utc::now())
                    .and_then(|mut views| views.pop())
                    .is_some_and(|view| view.exact);
                if !exact && self.seed_ring(&tenant, &resource_type, window).await {
                    report.rings_seeded += 1;
                }
            }
        }

        report.rings_seeded += self.drain_pending_ring_seeds().await;
        debug!(
            reconciled = report.reconciled.len(),
            skipped_active_import = report.skipped_active_import.len(),
            not_due = report.not_due.len(),
            rings_seeded = report.rings_seeded,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "dashboard reconcile: pass completed"
        );
        report
    }
}

#[async_trait]
impl<S> DashboardProvider for StorageDashboardProvider<S>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    async fn snapshot(
        &self,
        window: DashboardWindow,
        tenant: &str,
        types: &[String],
        include_empty: bool,
    ) -> DashboardSnapshot {
        let tenant_id = if tenant.is_empty() {
            self.default_tenant.as_str()
        } else {
            tenant
        };
        let tenant = tenant_context(tenant_id);

        // A seeded tenant never waits on storage aggregates (#1078); an
        // unseeded one reads storage, which seeds it for the next load.
        if let Some(snapshot) = self
            .snapshot_from_counters(&tenant, window, types, include_empty)
            .await
        {
            return snapshot;
        }
        self.snapshot_from_storage(&tenant, window, types, include_empty)
            .await
    }

    /// A seeded tenant's snapshots all come from the in-memory counters —
    /// exact or approximate — so the cache keeps them only briefly and a
    /// settled dashboard notices an import on its next poll (#1078).
    fn serves_in_constant_time(&self, tenant: &str) -> bool {
        let tenant_id = if tenant.is_empty() {
            self.default_tenant.as_str()
        } else {
            tenant
        };
        self.counters.is_seeded(tenant_id)
    }
}

/// The reconcile interval: `HFS_DASHBOARD_RECONCILE_SECS` when set to a
/// positive whole number of seconds, else [`DEFAULT_RECONCILE_INTERVAL`].
fn reconcile_interval_from_env() -> StdDuration {
    match std::env::var(RECONCILE_INTERVAL_ENV) {
        Err(_) => DEFAULT_RECONCILE_INTERVAL,
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(secs) if secs > 0 => StdDuration::from_secs(secs),
            _ => {
                warn!(
                    env = RECONCILE_INTERVAL_ENV,
                    value = %raw,
                    default_secs = DEFAULT_RECONCILE_INTERVAL.as_secs(),
                    "invalid dashboard reconcile interval; using the default"
                );
                DEFAULT_RECONCILE_INTERVAL
            }
        },
    }
}

/// Spawns the dashboard counters' background reconcile for `provider`
/// (#1078); see the [module documentation](self) for what it does.
///
/// Called from [`crate::build_app`] right after the provider is registered.
/// The loop holds only a [`Weak`] reference: once the provider is no longer
/// registered (a later `build_app` replaced it) and its last in-flight compute
/// has released it, the loop stops — so repeated app construction, as in the
/// test suites, never accumulates loops. Returns `None`, without spawning,
/// outside a Tokio runtime (`build_app` is synchronous and may be called from
/// non-async contexts); the provider still seeds tenants lazily on their first
/// view, but then never reconciles.
pub(crate) fn spawn_reconcile_loop<S>(
    provider: &Arc<StorageDashboardProvider<S>>,
) -> Option<tokio::task::JoinHandle<()>>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    spawn_reconcile_loop_every(provider, reconcile_interval_from_env())
}

/// [`spawn_reconcile_loop`] with the interval injected.
fn spawn_reconcile_loop_every<S>(
    provider: &Arc<StorageDashboardProvider<S>>,
    interval: StdDuration,
) -> Option<tokio::task::JoinHandle<()>>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    if tokio::runtime::Handle::try_current().is_err() {
        warn!(
            "No Tokio runtime available at app construction; skipping the dashboard \
             counters reconcile. Dashboard figures may stay approximate."
        );
        return None;
    }
    let wake = Arc::clone(&provider.ring_seeds.wake);
    Some(tokio::spawn(run_reconcile_loop(
        Arc::downgrade(provider),
        wake,
        interval,
    )))
}

/// The reconcile loop body: the startup seed, then a [`reconcile
/// pass`](StorageDashboardProvider::reconcile_pass) every `interval`, draining
/// queued ring seeds in between whenever a page load wakes it.
async fn run_reconcile_loop<S>(
    provider: Weak<StorageDashboardProvider<S>>,
    wake: Arc<Notify>,
    interval: StdDuration,
) where
    S: ResourceStorage + Send + Sync + 'static,
{
    let mut schedule = ReconcileSchedule::new(interval);
    match provider.upgrade() {
        Some(provider) => provider.seed_default_tenant(&mut schedule).await,
        None => return,
    }

    let mut next_pass = tokio::time::Instant::now() + interval;
    loop {
        let woken = tokio::select! {
            () = tokio::time::sleep_until(next_pass) => false,
            () = wake.notified() => true,
        };
        if woken {
            tokio::time::sleep(RING_SEED_DEBOUNCE).await;
        }
        let Some(provider) = provider.upgrade() else {
            debug!("dashboard reconcile: provider no longer registered; stopping");
            return;
        };
        if woken {
            provider.drain_pending_ring_seeds().await;
        } else {
            provider.reconcile_pass(&mut schedule).await;
            next_pass = tokio::time::Instant::now() + interval;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceCountDelta;
    use helios_persistence::error::{BackendError, StorageError};
    use helios_persistence::types::StoredResource;
    use serde_json::Value;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    /// A private counter set per test: the production default is the
    /// process-global one, which every test in this binary would share.
    fn isolated_counters() -> &'static DashboardCounters {
        Box::leak(Box::new(DashboardCounters::new()))
    }

    fn test_config() -> ServerConfig {
        ServerConfig {
            default_tenant: "default".to_string(),
            ..ServerConfig::for_testing()
        }
    }

    fn sqlite() -> Arc<SqliteBackend> {
        let backend = SqliteBackend::in_memory().expect("in-memory sqlite backend");
        backend.init_schema().expect("init schema");
        Arc::new(backend)
    }

    async fn create_in<S: ResourceStorage>(storage: &S, resource_type: &str) -> StoredResource {
        storage
            .create(
                &test_tenant(),
                resource_type,
                serde_json::json!({ "resourceType": resource_type }),
                FhirVersion::R4,
            )
            .await
            .expect("create")
    }

    /// Polls `cond` for up to five seconds.
    async fn eventually(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..250 {
            if cond() {
                return true;
            }
            tokio::time::sleep(StdDuration::from_millis(20)).await;
        }
        cond()
    }

    /// A storage double over SQLite that counts every aggregate the dashboard
    /// could run — and can make each one slow or fail — so a test can prove
    /// which read path a snapshot took. Plain CRUD passes straight through.
    struct InstrumentedStorage {
        inner: Arc<SqliteBackend>,
        aggregate_calls: AtomicUsize,
        delay_ms: AtomicU64,
        fail: AtomicBool,
    }

    impl InstrumentedStorage {
        fn over(inner: Arc<SqliteBackend>) -> Arc<Self> {
            Arc::new(Self {
                inner,
                aggregate_calls: AtomicUsize::new(0),
                delay_ms: AtomicU64::new(0),
                fail: AtomicBool::new(false),
            })
        }

        fn aggregate_calls(&self) -> usize {
            self.aggregate_calls.load(Ordering::SeqCst)
        }

        async fn aggregate(&self) -> StorageResult<()> {
            self.aggregate_calls.fetch_add(1, Ordering::SeqCst);
            let delay = self.delay_ms.load(Ordering::SeqCst);
            if delay > 0 {
                tokio::time::sleep(StdDuration::from_millis(delay)).await;
            }
            if self.fail.load(Ordering::SeqCst) {
                return Err(StorageError::Backend(BackendError::Unavailable {
                    backend_name: "instrumented".to_string(),
                    message: "injected aggregate failure".to_string(),
                }));
            }
            Ok(())
        }
    }

    #[async_trait]
    impl ResourceStorage for InstrumentedStorage {
        fn backend_name(&self) -> &'static str {
            "instrumented"
        }

        async fn create(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            resource: Value,
            fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            ResourceStorage::create(
                self.inner.as_ref(),
                tenant,
                resource_type,
                resource,
                fhir_version,
            )
            .await
        }

        async fn create_or_update(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            id: &str,
            resource: Value,
            fhir_version: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            ResourceStorage::create_or_update(
                self.inner.as_ref(),
                tenant,
                resource_type,
                id,
                resource,
                fhir_version,
            )
            .await
        }

        async fn read(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            id: &str,
        ) -> StorageResult<Option<StoredResource>> {
            ResourceStorage::read(self.inner.as_ref(), tenant, resource_type, id).await
        }

        async fn update(
            &self,
            tenant: &TenantContext,
            current: &StoredResource,
            resource: Value,
        ) -> StorageResult<StoredResource> {
            ResourceStorage::update(self.inner.as_ref(), tenant, current, resource).await
        }

        async fn delete(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            id: &str,
        ) -> StorageResult<()> {
            ResourceStorage::delete(self.inner.as_ref(), tenant, resource_type, id).await
        }

        async fn count(
            &self,
            tenant: &TenantContext,
            resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            self.aggregate().await?;
            ResourceStorage::count(self.inner.as_ref(), tenant, resource_type).await
        }

        async fn count_by_types(
            &self,
            tenant: &TenantContext,
            resource_types: &[&str],
        ) -> StorageResult<Vec<(String, u64)>> {
            self.aggregate().await?;
            ResourceStorage::count_by_types(self.inner.as_ref(), tenant, resource_types).await
        }

        async fn count_deltas_by_bucket(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            since: DateTime<Utc>,
            bucket_seconds: i64,
        ) -> StorageResult<Vec<ResourceCountDelta>> {
            self.aggregate().await?;
            ResourceStorage::count_deltas_by_bucket(
                self.inner.as_ref(),
                tenant,
                resource_type,
                since,
                bucket_seconds,
            )
            .await
        }

        async fn count_all_types(
            &self,
            tenant: &TenantContext,
        ) -> StorageResult<Vec<(String, u64)>> {
            self.aggregate().await?;
            ResourceStorage::count_all_types(self.inner.as_ref(), tenant).await
        }
    }

    /// `(resource_type, total, [(bucket_start, delta, cumulative)])` — a
    /// comparable projection of a series.
    type SeriesShape = (String, u64, Vec<(DateTime<Utc>, i64, u64)>);

    fn shape<'a>(series: impl IntoIterator<Item = &'a DashboardSeries>) -> Vec<SeriesShape> {
        series
            .into_iter()
            .map(|s| {
                (
                    s.resource_type.clone(),
                    s.total,
                    s.points
                        .iter()
                        .map(|p| (p.bucket_start, p.delta, p.cumulative))
                        .collect(),
                )
            })
            .collect()
    }

    /// The provider builds a well-formed, zeroed snapshot over an empty store:
    /// one dense per-type series, zero totals, and a non-empty FHIR version.
    /// Exercises `StorageDashboardProvider::new` and the `snapshot` success path
    /// (both backend queries succeed and return "nothing yet"), including the
    /// derived `total_resources` — an empty grouping sums to zero (#959).
    #[tokio::test]
    async fn snapshot_over_empty_backend_is_zeroed_but_well_formed() {
        let backend = SqliteBackend::in_memory().expect("in-memory sqlite backend");
        backend.init_schema().expect("init schema");
        let config = ServerConfig {
            default_tenant: "default".to_string(),
            ..ServerConfig::for_testing()
        };

        let provider = StorageDashboardProvider::new(Arc::new(backend), &config)
            .with_counters(isolated_counters());
        let snapshot = provider
            .snapshot(DashboardWindow::default(), "", &[], false)
            .await;

        // An empty store has nothing to chart: no available types, no series —
        // the UI renders its explicit empty state from this (#555).
        assert!(snapshot.series.is_empty());
        assert!(snapshot.available.is_empty());
        assert_eq!(snapshot.total_resources, 0);
        assert_eq!(snapshot.distinct_types, 0);
        assert!(!snapshot.fhir_version.is_empty());
        // Every query answered: these zeros are measurements, not fallbacks,
        // and the page may present them as such (#956).
        assert!(!snapshot.partial);
    }

    /// Every window yields a dense series of exactly its own length, on
    /// epoch-aligned bucket boundaries of its own width.
    #[tokio::test]
    async fn every_window_yields_a_dense_epoch_aligned_series() {
        let backend = SqliteBackend::in_memory().expect("in-memory sqlite backend");
        backend.init_schema().expect("init schema");
        let tenant = test_tenant();
        let now = Utc::now();

        for window in DashboardWindow::ALL {
            let series = resource_count_series(
                &backend,
                &tenant,
                &["Patient"],
                SeriesWindow::from_dashboard_window(window),
                now,
            )
            .await
            .expect("series");

            let points = &series[0].points;
            assert_eq!(points.len(), window.points(), "{}", window.as_str());
            let bucket = window.bucket_seconds();
            assert!(
                points
                    .iter()
                    .all(|p| p.bucket_start.timestamp() % bucket == 0),
                "{} produced unaligned buckets",
                window.as_str()
            );
            // Buckets are contiguous and ascending, and the last one contains `now`.
            for pair in points.windows(2) {
                assert_eq!(
                    (pair[1].bucket_start - pair[0].bucket_start).num_seconds(),
                    bucket
                );
            }
            let last = points.last().unwrap().bucket_start;
            assert!(last <= now && now < last + Duration::seconds(bucket));
        }
    }

    /// The point of going history-backed: a resource created inside the window
    /// stays in the bucket it was *created* in, even after it is edited later.
    /// The old `last_updated` bucketing would have moved it to the edit's bucket,
    /// rewriting the past — which is exactly what makes fine buckets unusable.
    #[tokio::test]
    async fn curve_is_stable_under_later_edits_and_ends_at_the_live_total() {
        let backend = SqliteBackend::in_memory().expect("in-memory sqlite backend");
        backend.init_schema().expect("init schema");
        let tenant = test_tenant();

        let created = backend
            .create(
                &tenant,
                "Patient",
                serde_json::json!({"resourceType": "Patient"}),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("create");
        let create_bucket = bucket_floor(created.last_modified(), 60);

        // A second resource, then an update to the first: the update writes a new
        // history version but must not shift the first resource's creation bucket.
        backend
            .create(
                &tenant,
                "Patient",
                serde_json::json!({"resourceType": "Patient"}),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("create");
        backend
            .update(
                &tenant,
                &created,
                serde_json::json!({"resourceType": "Patient", "active": true}),
            )
            .await
            .expect("update");

        let now = Utc::now();
        let series = resource_count_series(
            &backend,
            &tenant,
            &["Patient"],
            SeriesWindow::from_dashboard_window(DashboardWindow::LastHour),
            now,
        )
        .await
        .expect("series");
        let patients = &series[0];

        // Two creations and one update: the update contributes no delta, so the
        // creation bucket holds exactly +2 and the curve ends at the live total.
        assert_eq!(patients.total, 2);
        let charted: i64 = patients.points.iter().map(|p| p.delta).sum();
        assert_eq!(charted, 2, "the update must not add a delta");
        let at_create = patients
            .points
            .iter()
            .find(|p| p.bucket_start == create_bucket)
            .expect("creation bucket is inside the last-hour window");
        assert_eq!(at_create.delta, 2);
        assert_eq!(patients.points.last().unwrap().cumulative, 2);

        // Deleting one resource nets it back out: -1 in the delete's bucket, and
        // the endpoint tracks the live total down to 1.
        backend
            .delete(&tenant, "Patient", created.id())
            .await
            .expect("delete");

        let series = resource_count_series(
            &backend,
            &tenant,
            &["Patient"],
            SeriesWindow::from_dashboard_window(DashboardWindow::LastHour),
            Utc::now(),
        )
        .await
        .expect("series");
        let patients = &series[0];
        assert_eq!(patients.total, 1);
        assert_eq!(patients.points.iter().map(|p| p.delta).sum::<i64>(), 1);
        assert_eq!(patients.points.last().unwrap().cumulative, 1);
    }

    /// Resources created *before* the window open into the curve's baseline, so a
    /// short window over an old store still starts — and ends — at the live total
    /// rather than at zero.
    #[tokio::test]
    async fn resources_predating_the_window_form_the_baseline() {
        let backend = SqliteBackend::in_memory().expect("in-memory sqlite backend");
        backend.init_schema().expect("init schema");
        let tenant = test_tenant();

        for _ in 0..3 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    serde_json::json!({"resourceType": "Patient"}),
                    helios_fhir::FhirVersion::R4,
                )
                .await
                .expect("create");
        }

        // Chart a window that closed before those creations happened: they fall
        // outside it, so every delta is zero and the whole curve sits at the total.
        let long_ago = Utc::now() - Duration::days(365);
        let series = resource_count_series(
            &backend,
            &tenant,
            &["Patient"],
            SeriesWindow::from_dashboard_window(DashboardWindow::LastHour),
            long_ago,
        )
        .await
        .expect("series");
        let patients = &series[0];

        assert_eq!(patients.total, 3);
        assert!(patients.points.iter().all(|p| p.delta == 0));
        assert!(patients.points.iter().all(|p| p.cumulative == 3));
    }

    /// Without `include_empty`, requesting a type the tenant has never stored
    /// is silently dropped (today's behavior) and the selection falls back to
    /// nothing plotted for it — the guard at `:302` is untouched by the flag.
    #[tokio::test]
    async fn unstored_type_is_dropped_without_the_flag() {
        let backend = SqliteBackend::in_memory().expect("in-memory sqlite backend");
        backend.init_schema().expect("init schema");
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "Patient",
                serde_json::json!({"resourceType": "Patient"}),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("create");
        let config = ServerConfig {
            default_tenant: "default".to_string(),
            ..ServerConfig::for_testing()
        };

        let provider = StorageDashboardProvider::new(Arc::new(backend), &config)
            .with_counters(isolated_counters());
        let snapshot = provider
            .snapshot(
                DashboardWindow::default(),
                "",
                &["Observation".to_string()],
                false,
            )
            .await;

        assert!(
            snapshot.series.is_empty(),
            "an unstored type with no flag charts nothing"
        );
    }

    /// With `include_empty` (#599, "View all resources"), a type the tenant
    /// has never stored is still accepted and charted — a dense series of
    /// flat zeros, not an absent series or an error. `distinct_types` (the
    /// stat card) is unaffected by the flag either way.
    #[tokio::test]
    async fn unstored_type_charts_a_flat_zero_series_with_the_flag() {
        let backend = SqliteBackend::in_memory().expect("in-memory sqlite backend");
        backend.init_schema().expect("init schema");
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "Patient",
                serde_json::json!({"resourceType": "Patient"}),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("create");
        let config = ServerConfig {
            default_tenant: "default".to_string(),
            ..ServerConfig::for_testing()
        };

        let backend = Arc::new(backend);
        let provider = StorageDashboardProvider::new(Arc::clone(&backend), &config)
            .with_counters(isolated_counters());
        let requested = vec!["Patient".to_string(), "Observation".to_string()];

        let without_flag = provider
            .snapshot(DashboardWindow::default(), "", &requested, false)
            .await;
        // A fresh provider, so this snapshot also takes the storage path…
        let with_flag = StorageDashboardProvider::new(Arc::clone(&backend), &config)
            .with_counters(isolated_counters())
            .snapshot(DashboardWindow::default(), "", &requested, true)
            .await;
        // …while the first provider, seeded by its first snapshot, now
        // answers from the write counters (#1078), which must honour the flag
        // the same way.
        let with_flag_from_counters = provider
            .snapshot(DashboardWindow::default(), "", &requested, true)
            .await;
        assert_eq!(with_flag_from_counters.distinct_types, 1);
        let observation = with_flag_from_counters
            .series
            .iter()
            .find(|s| s.resource_type == "Observation")
            .expect("Observation is charted from the counters with the flag");
        assert_eq!(observation.total, 0);
        assert_eq!(
            observation.points.len(),
            DashboardWindow::default().points()
        );
        assert!(observation.points.iter().all(|p| p.cumulative == 0));

        // The stat card counts only what the tenant actually stores — the
        // flag never moves it.
        assert_eq!(without_flag.distinct_types, 1);
        assert_eq!(with_flag.distinct_types, 1);

        // Without the flag, the never-stored type is dropped from the
        // selection; only Patient is charted.
        assert_eq!(
            without_flag
                .series
                .iter()
                .map(|s| s.resource_type.as_str())
                .collect::<Vec<_>>(),
            vec!["Patient"]
        );

        // With the flag, both are charted: Observation's series is present,
        // dense (same point count as Patient's), and flat at zero.
        let observation = with_flag
            .series
            .iter()
            .find(|s| s.resource_type == "Observation")
            .expect("Observation is charted with the flag");
        assert_eq!(observation.total, 0);
        assert!(!observation.points.is_empty(), "series must not be absent");
        assert!(
            observation.points.iter().all(|p| p.cumulative == 0),
            "an unstored type is a flat line at 0, not invented data"
        );
        let patient_points = with_flag
            .series
            .iter()
            .find(|s| s.resource_type == "Patient")
            .expect("Patient still charted")
            .points
            .len();
        assert_eq!(
            observation.points.len(),
            patient_points,
            "every plotted series shares the window's dense bucket count"
        );
    }

    /// `total_resources` is the sum of the per-type counts, not a second
    /// full-table `COUNT(*)` (#959) — and it agrees with what the backend's
    /// own `count(tenant, None)` reports, including after a delete.
    #[tokio::test]
    async fn total_resources_is_the_sum_of_the_per_type_counts() {
        let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite backend"));
        backend.init_schema().expect("init schema");
        let tenant = test_tenant();
        let config = ServerConfig {
            default_tenant: "default".to_string(),
            ..ServerConfig::for_testing()
        };

        let doomed = backend
            .create(
                &tenant,
                "Patient",
                serde_json::json!({"resourceType": "Patient"}),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("create patient");
        for _ in 0..2 {
            backend
                .create(
                    &tenant,
                    "Observation",
                    serde_json::json!({"resourceType": "Observation"}),
                    helios_fhir::FhirVersion::R4,
                )
                .await
                .expect("create observation");
        }
        // Deleted rows must not be counted by either figure.
        backend
            .delete(&tenant, "Patient", doomed.id())
            .await
            .expect("delete");

        // A fresh provider per snapshot, so the per-tenant type-count cache
        // never masks the arithmetic under test.
        let snapshot = StorageDashboardProvider::new(Arc::clone(&backend), &config)
            .with_counters(isolated_counters())
            .snapshot(DashboardWindow::default(), "", &[], false)
            .await;

        let ungrouped = backend.count(&tenant, None).await.expect("count");
        assert_eq!(ungrouped, 2);
        assert_eq!(
            snapshot.total_resources, ungrouped,
            "the derived sum must equal the backend's ungrouped live count"
        );
        assert_eq!(
            snapshot.total_resources,
            snapshot.available.iter().map(|t| t.total).sum::<u64>()
        );
        assert_eq!(snapshot.distinct_types, 1, "Patient's only row is deleted");
    }

    /// The per-tenant type counts are cached independently of the snapshot
    /// key (#959), so flipping the window does not re-run the `GROUP BY` over
    /// every live row. Proven without a fake backend: mutate the store
    /// between two snapshots on the *same* provider and observe that the
    /// second still reports the first's cached figures, while the chart
    /// (which is not cached here) does see the new data.
    ///
    /// The two snapshots are back to back, so they land inside
    /// [`TYPE_COUNTS_TTL`] with seconds to spare. That TTL is short by design
    /// — it exists to absorb sibling keys asking for the same figure at the
    /// same time, not to hold numbers past the snapshot layer's own 15s
    /// freshness — which is exactly the reuse this test pins down.
    ///
    /// Since #1078 the first snapshot also seeds the write counters, which
    /// would serve the second from memory; the counters are invalidated in
    /// between (as a purge would) so the second snapshot takes the storage
    /// path again and meets the type-count cache.
    #[tokio::test]
    async fn per_tenant_type_counts_are_reused_across_windows() {
        let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite backend"));
        backend.init_schema().expect("init schema");
        let tenant = test_tenant();
        let config = ServerConfig {
            default_tenant: "default".to_string(),
            ..ServerConfig::for_testing()
        };
        backend
            .create(
                &tenant,
                "Patient",
                serde_json::json!({"resourceType": "Patient"}),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("create patient");

        let counters = isolated_counters();
        let provider =
            StorageDashboardProvider::new(Arc::clone(&backend), &config).with_counters(counters);
        let first = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert_eq!(first.total_resources, 1);
        assert_eq!(first.distinct_types, 1);
        counters.invalidate_tenant("default");

        // A brand-new type lands after the cache was filled.
        backend
            .create(
                &tenant,
                "Observation",
                serde_json::json!({"resourceType": "Observation"}),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("create observation");

        // A different window: a different observability-cache key, so the
        // provider is called again — but the type counts come from the
        // provider's own cache, well inside `TYPE_COUNTS_TTL`.
        let second = provider
            .snapshot(DashboardWindow::LastMonth, "", &[], false)
            .await;
        assert_eq!(
            second.total_resources, 1,
            "headline total came from the cached grouping, not a fresh scan"
        );
        assert_eq!(second.distinct_types, 1);
        assert!(
            second
                .available
                .iter()
                .all(|t| t.resource_type != "Observation"),
            "the picker list is the cached one too"
        );

        // A provider with a cold cache does see both types, which is what
        // makes the assertions above evidence of caching rather than of a
        // write that never happened.
        let uncached = StorageDashboardProvider::new(Arc::clone(&backend), &config)
            .with_counters(isolated_counters())
            .snapshot(DashboardWindow::LastMonth, "", &[], false)
            .await;
        assert_eq!(uncached.total_resources, 2);
        assert_eq!(uncached.distinct_types, 2);
    }

    fn test_tenant() -> TenantContext {
        TenantContext::new(TenantId::new("default"), TenantPermissions::full_access())
    }

    /// Wraps an `ExportRequest` in a `StartExportInput` with default kickoff
    /// metadata, mirroring the sqlite backend's own test helper.
    fn test_export_input(
        request: helios_persistence::core::ExportRequest,
    ) -> helios_persistence::core::StartExportInput {
        helios_persistence::core::StartExportInput {
            request,
            transaction_time: Utc::now(),
            request_url: "http://localhost/$export".to_string(),
            owner_subject: Some("test-subject".to_string()),
            fhir_version: helios_fhir::FhirVersion::default(),
        }
    }

    /// With no job stores wired (the default from `new`), both job-count
    /// fields stay `None` — this is a normal, unconfigured state, not an error.
    #[tokio::test]
    async fn job_counts_are_unavailable_when_no_job_stores_are_wired() {
        let backend = SqliteBackend::in_memory().expect("in-memory sqlite backend");
        backend.init_schema().expect("init schema");
        let config = ServerConfig {
            default_tenant: "default".to_string(),
            ..ServerConfig::for_testing()
        };

        let provider = StorageDashboardProvider::new(Arc::new(backend), &config)
            .with_counters(isolated_counters());
        let snapshot = provider
            .snapshot(DashboardWindow::default(), "", &[], false)
            .await;

        assert_eq!(snapshot.export_jobs, None);
        assert_eq!(snapshot.import_jobs_active, None);
    }

    /// With both job stores wired but nothing submitted yet, the counts are
    /// real zeros — distinct from the "not wired" `None` case above.
    #[tokio::test]
    async fn job_counts_are_zero_over_empty_job_stores() {
        let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite backend"));
        backend.init_schema().expect("init schema");
        let config = ServerConfig {
            default_tenant: "default".to_string(),
            ..ServerConfig::for_testing()
        };

        // SqliteBackend implements both BulkExportJobStore and BulkSubmitJobStore,
        // so the same Arc can stand in for storage and both job stores.
        let export_jobs = Arc::clone(&backend) as Arc<dyn BulkExportJobStore>;
        let submit_jobs = Arc::clone(&backend) as Arc<dyn BulkSubmitJobStore>;
        let provider = StorageDashboardProvider::new(Arc::clone(&backend), &config)
            .with_counters(isolated_counters())
            .with_job_stores(Some(export_jobs), Some(submit_jobs));
        let snapshot = provider
            .snapshot(DashboardWindow::default(), "", &[], false)
            .await;

        assert_eq!(
            snapshot.export_jobs,
            Some(ExportJobCounts {
                running: 0,
                queued: 0
            })
        );
        assert_eq!(snapshot.import_jobs_active, Some(0));
    }

    /// Real export jobs in `accepted` and `in-progress` state, plus a real
    /// active submission, are reflected exactly in the snapshot.
    #[tokio::test]
    async fn job_counts_reflect_accepted_and_in_progress_exports() {
        use helios_persistence::core::{
            BulkExportStorage, BulkSubmitProvider, ExportClaimStrategy, ExportRequest,
            ExportWorkerStorage, SubmissionId, WorkerId,
        };

        let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite backend"));
        backend.init_schema().expect("init schema");
        let config = ServerConfig {
            default_tenant: "default".to_string(),
            ..ServerConfig::for_testing()
        };
        let tenant = test_tenant();

        // Two export jobs: one stays `accepted`, the other is claimed and
        // driven to `in-progress` through the real worker path.
        backend
            .start_export(&tenant, test_export_input(ExportRequest::system()))
            .await
            .expect("start export 1");
        backend
            .start_export(&tenant, test_export_input(ExportRequest::system()))
            .await
            .expect("start export 2");

        let worker = WorkerId::new("worker-1");
        let lease = backend
            .claim_next(&worker, std::time::Duration::from_secs(60))
            .await
            .expect("claim_next succeeds")
            .expect("a job should be claimable");
        backend
            .mark_export_in_progress(&tenant, &lease.job_id, &worker, lease.fencing_token)
            .await
            .expect("mark in progress");

        // One active submission (freshly created submissions start `in-progress`).
        let submission_id = SubmissionId::generate("test-system");
        backend
            .create_submission(&tenant, &submission_id, None)
            .await
            .expect("create submission");

        let export_jobs = Arc::clone(&backend) as Arc<dyn BulkExportJobStore>;
        let submit_jobs = Arc::clone(&backend) as Arc<dyn BulkSubmitJobStore>;
        let provider = StorageDashboardProvider::new(Arc::clone(&backend), &config)
            .with_counters(isolated_counters())
            .with_job_stores(Some(export_jobs), Some(submit_jobs));
        let snapshot = provider
            .snapshot(DashboardWindow::default(), "", &[], false)
            .await;

        assert_eq!(
            snapshot.export_jobs,
            Some(ExportJobCounts {
                running: 1,
                queued: 1
            })
        );
        assert_eq!(snapshot.import_jobs_active, Some(1));
    }

    /// Job counts are small reads, but every page load asked for them; during
    /// an import they are cached per tenant for [`JOB_COUNTS_TTL`] (#1078).
    #[tokio::test]
    async fn job_counts_are_cached_briefly_per_tenant() {
        use helios_persistence::core::{BulkSubmitProvider, SubmissionId};

        let backend = sqlite();
        let submit_jobs = Arc::clone(&backend) as Arc<dyn BulkSubmitJobStore>;
        let provider = StorageDashboardProvider::new(Arc::clone(&backend), &test_config())
            .with_counters(isolated_counters())
            .with_job_stores(None, Some(submit_jobs));

        let first = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert_eq!(first.import_jobs_active, Some(0));

        backend
            .create_submission(&test_tenant(), &SubmissionId::generate("test-system"), None)
            .await
            .expect("create submission");

        let cached = provider
            .snapshot(DashboardWindow::LastDay, "", &[], false)
            .await;
        assert_eq!(
            cached.import_jobs_active,
            Some(0),
            "inside the TTL the cached count is served"
        );

        // A cold cache reads the new submission.
        let fresh = StorageDashboardProvider::new(Arc::clone(&backend), &test_config())
            .with_counters(isolated_counters())
            .with_job_stores(
                None,
                Some(Arc::clone(&backend) as Arc<dyn BulkSubmitJobStore>),
            )
            .snapshot(DashboardWindow::LastDay, "", &[], false)
            .await;
        assert_eq!(fresh.import_jobs_active, Some(1));
    }

    /// An unseeded tenant's first snapshot reads storage — and those very
    /// reads seed the write counters: the totals are reconciled and the
    /// charted window's history rings loaded (#1078). The next snapshot is
    /// then served from memory, which a write storage saw but the counters did
    /// not makes visible.
    #[tokio::test]
    async fn unseeded_snapshot_seeds_the_counters_from_its_storage_reads() {
        let backend = sqlite();
        create_in(backend.as_ref(), "Patient").await;
        create_in(backend.as_ref(), "Patient").await;
        create_in(backend.as_ref(), "Observation").await;
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&backend), &test_config())
            .with_counters(counters);
        assert!(!counters.is_seeded("default"));

        let first = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(!first.partial);
        assert!(!first.approximate, "a storage read is exact");
        assert_eq!(first.total_resources, 3);

        assert!(counters.is_seeded("default"));
        let totals = counters.totals_view("default").expect("seeded totals");
        assert!(totals.exact);
        assert_eq!(
            totals.totals,
            vec![("Patient".to_string(), 2), ("Observation".to_string(), 1)]
        );
        let now = Utc::now();
        let hour = counters
            .series_view(
                "default",
                DashboardWindow::LastHour,
                &["Patient", "Observation"],
                now,
            )
            .expect("seeded series");
        assert!(hour.iter().all(|s| s.history_seeded && s.exact));
        let day = counters
            .series_view("default", DashboardWindow::LastDay, &["Patient"], now)
            .expect("seeded series");
        assert!(
            !day[0].history_seeded,
            "only the charted window's ring was loaded"
        );

        // Storage gains a row the counters never hear about: the next load is
        // served from the counters, so it does not show it.
        create_in(backend.as_ref(), "Encounter").await;
        let second = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert_eq!(second.total_resources, 3);
        assert!(!second.approximate);
        let ends = |snapshot: &DashboardSnapshot| -> Vec<(String, u64, u64)> {
            snapshot
                .series
                .iter()
                .map(|s| {
                    (
                        s.resource_type.clone(),
                        s.total,
                        s.points.last().map_or(0, |p| p.cumulative),
                    )
                })
                .collect()
        };
        assert_eq!(ends(&second), ends(&first));
    }

    /// #1078: the provider reports constant-time snapshots exactly for seeded
    /// tenants, so the cache's live fast path covers a seeded tenant's exact
    /// snapshots too. `""` resolves to the default tenant, as in `snapshot`.
    #[tokio::test]
    async fn serves_in_constant_time_once_the_tenant_is_seeded() {
        let backend = sqlite();
        create_in(backend.as_ref(), "Patient").await;
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&backend), &test_config())
            .with_counters(counters);
        assert!(!provider.serves_in_constant_time("default"));
        assert!(!provider.serves_in_constant_time(""));

        let seeding = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(!seeding.partial);
        assert!(counters.is_seeded("default"));

        assert!(provider.serves_in_constant_time("default"));
        assert!(
            provider.serves_in_constant_time(""),
            "the empty tenant is the default tenant"
        );
        assert!(
            !provider.serves_in_constant_time("other"),
            "seeding is per tenant"
        );
    }

    /// A failed storage read must never seed the counters: the tenant stays
    /// on the storage path, flagged partial, until a read succeeds (#956).
    #[tokio::test]
    async fn failed_storage_read_leaves_the_tenant_unseeded() {
        let storage = InstrumentedStorage::over(sqlite());
        create_in(storage.as_ref(), "Patient").await;
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(counters);

        storage.fail.store(true, Ordering::SeqCst);
        let failed = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(failed.partial);
        assert!(!counters.is_seeded("default"));
        assert!(counters.totals_view("default").is_none());
        assert!(
            counters
                .series_view(
                    "default",
                    DashboardWindow::LastHour,
                    &["Patient"],
                    Utc::now()
                )
                .is_none()
        );
        assert!(counters.tenants().is_empty(), "no ring was loaded either");

        storage.fail.store(false, Ordering::SeqCst);
        let recovered = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(!recovered.partial);
        assert_eq!(recovered.total_resources, 1);
        assert!(counters.is_seeded("default"));
    }

    /// The #1078 acceptance core: once seeded, no page load runs a storage
    /// aggregate — whatever the window, selection or "View all" toggle — and
    /// the figures are the counters' (a recorded REST write shows, a write
    /// the counters never saw does not), labelled approximate.
    #[tokio::test]
    async fn seeded_tenant_is_served_from_counters_without_storage_aggregates() {
        let storage = InstrumentedStorage::over(sqlite());
        create_in(storage.as_ref(), "Patient").await;
        create_in(storage.as_ref(), "Patient").await;
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(counters);

        provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        let after_seed = storage.aggregate_calls();
        assert!(after_seed > 0, "the seeding load read storage");

        // A write storage has but the counters never heard of…
        create_in(storage.as_ref(), "Observation").await;
        // …and one recorded as the REST handlers record it.
        counters.record("default", "Patient", 1, Utc::now());

        let selections: [(Vec<String>, bool); 3] = [
            (vec![], false),
            (vec!["Patient".to_string()], false),
            (vec!["Encounter".to_string()], true),
        ];
        for window in DashboardWindow::ALL {
            for (types, include_empty) in &selections {
                let snapshot = provider.snapshot(window, "", types, *include_empty).await;
                assert!(!snapshot.partial);
                assert!(snapshot.approximate, "a recorded write is not reconciled");
                assert!(!snapshot.series_pending);
                assert_eq!(snapshot.total_resources, 3);
                assert_eq!(snapshot.distinct_types, 1);
                assert!(snapshot.generated_at.is_some());
            }
        }
        assert_eq!(
            storage.aggregate_calls(),
            after_seed,
            "a seeded tenant's page loads run no storage aggregate"
        );

        let snapshot = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        let patients = &snapshot.series[0];
        assert_eq!(patients.resource_type, "Patient");
        assert_eq!(patients.total, 3);
        assert_eq!(patients.points.last().unwrap().cumulative, 3);
    }

    /// "Tests cover a slow provider or backend rendering from counters"
    /// (#1078): with every storage aggregate taking seconds, a seeded tenant's
    /// snapshot still returns at once, for every window — with the windows
    /// whose history is not loaded yet labelled approximate and queued for a
    /// background seed rather than read inline.
    #[tokio::test]
    async fn seeded_tenant_renders_promptly_over_a_slow_backend() {
        let storage = InstrumentedStorage::over(sqlite());
        create_in(storage.as_ref(), "Patient").await;
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(isolated_counters());
        provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;

        storage.delay_ms.store(5_000, Ordering::SeqCst);
        let calls = storage.aggregate_calls();
        for window in DashboardWindow::ALL {
            let snapshot = tokio::time::timeout(
                StdDuration::from_secs(1),
                provider.snapshot(window, "", &[], false),
            )
            .await
            .expect("a seeded tenant must not wait on the slow backend");
            assert!(!snapshot.partial);
            assert_eq!(snapshot.total_resources, 1);
            assert_eq!(snapshot.series.len(), 1);
            assert_eq!(snapshot.series[0].points.len(), window.points());
            assert_eq!(snapshot.series[0].points.last().unwrap().cumulative, 1);
            assert_eq!(
                snapshot.approximate,
                window != DashboardWindow::LastHour,
                "only the seeding load's window has storage history ({})",
                window.as_str()
            );
        }
        assert_eq!(storage.aggregate_calls(), calls);

        let mut pending: Vec<RingKey> =
            lock(&provider.ring_seeds.pending).iter().cloned().collect();
        pending.sort_by_key(|(_, _, window)| window.bucket_seconds());
        assert_eq!(
            pending,
            vec![
                (
                    "default".to_string(),
                    "Patient".to_string(),
                    DashboardWindow::LastDay
                ),
                (
                    "default".to_string(),
                    "Patient".to_string(),
                    DashboardWindow::LastMonth
                ),
            ]
        );
    }

    /// A ring charted without storage history is queued once however many
    /// loads ask, and the background drain loads it — after which the
    /// snapshot is exact.
    #[tokio::test]
    async fn queued_ring_seeds_are_single_flight_and_drained_in_the_background() {
        let backend = sqlite();
        create_in(backend.as_ref(), "Patient").await;
        let provider = StorageDashboardProvider::new(Arc::clone(&backend), &test_config())
            .with_counters(isolated_counters());
        provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;

        for _ in 0..3 {
            let snapshot = provider
                .snapshot(DashboardWindow::LastDay, "", &[], false)
                .await;
            assert!(snapshot.approximate);
        }
        assert_eq!(lock(&provider.ring_seeds.pending).len(), 1);

        assert_eq!(provider.drain_pending_ring_seeds().await, 1);
        assert!(lock(&provider.ring_seeds.pending).is_empty());
        let snapshot = provider
            .snapshot(DashboardWindow::LastDay, "", &[], false)
            .await;
        assert!(!snapshot.approximate);
        assert_eq!(snapshot.series[0].points.last().unwrap().cumulative, 1);
    }

    /// Convergence (#1078): writes the counters recorded — and one they never
    /// saw — leave the snapshot approximate; a reconcile pass with no writes
    /// in flight brings the totals back to exactly what storage holds, and
    /// once the newly charted ring is loaded the approximate label is gone.
    #[tokio::test]
    async fn reconcile_converges_to_exact_storage_totals() {
        let backend = sqlite();
        create_in(backend.as_ref(), "Patient").await;
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&backend), &test_config())
            .with_counters(counters);
        let seeded = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(!seeded.approximate);

        let recorded = create_in(backend.as_ref(), "Patient").await;
        counters.record("default", "Patient", 1, recorded.last_modified());
        // Another instance, or a direct database edit: storage only.
        create_in(backend.as_ref(), "Observation").await;

        let during = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(during.approximate);
        assert_eq!(during.total_resources, 2);

        let mut schedule = ReconcileSchedule::new(StdDuration::from_secs(30));
        let report = provider.reconcile_pass(&mut schedule).await;
        assert_eq!(report.reconciled, vec!["default".to_string()]);
        assert!(report.skipped_active_import.is_empty());
        assert_eq!(
            report.rings_seeded, 1,
            "the written Patient ring is re-seeded"
        );

        let tenant = test_tenant();
        let live = ResourceStorage::count(backend.as_ref(), &tenant, None)
            .await
            .expect("count");
        assert_eq!(live, 3);
        let after = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert_eq!(after.total_resources, live);
        assert_eq!(after.distinct_types, 2);
        // Observation is charted for the first time, from counters only.
        assert!(after.approximate);
        assert_eq!(provider.drain_pending_ring_seeds().await, 1);

        let converged = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(!converged.approximate, "reconciled and loaded: exact again");
        assert_eq!(converged.total_resources, live);
        let now = Utc::now();
        let from_storage = resource_count_series(
            backend.as_ref(),
            &tenant,
            &["Patient", "Observation"],
            SeriesWindow::from_dashboard_window(DashboardWindow::LastHour),
            now,
        )
        .await
        .expect("storage series");
        let from_counters = resource_count_series_from_counters(
            counters,
            "default",
            DashboardWindow::LastHour,
            &["Patient", "Observation"],
            now,
        )
        .expect("counter series");
        assert_eq!(
            shape(&from_storage),
            shape(from_counters.iter().map(|c| &c.series))
        );

        // Straight away, the next pass is not due (duty cycle) and reads
        // nothing.
        let report = provider.reconcile_pass(&mut schedule).await;
        assert!(report.reconciled.is_empty());
        assert_eq!(report.not_due, vec!["default".to_string()]);
        assert_eq!(report.rings_seeded, 0);
    }

    /// Back-off (#1078): while a bulk submit is active for a tenant, the
    /// reconcile pass skips it entirely — no grouping query, no ring seeds —
    /// and the counters keep following the import.
    #[tokio::test]
    async fn reconcile_skips_tenants_with_an_active_bulk_submit() {
        use helios_persistence::core::{BulkSubmitProvider, SubmissionId};

        let storage = InstrumentedStorage::over(sqlite());
        create_in(storage.as_ref(), "Patient").await;
        storage
            .inner
            .create_submission(&test_tenant(), &SubmissionId::generate("test-system"), None)
            .await
            .expect("create submission");
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(counters)
            .with_job_stores(
                None,
                Some(Arc::clone(&storage.inner) as Arc<dyn BulkSubmitJobStore>),
            );
        let seeded = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert_eq!(seeded.import_jobs_active, Some(1));
        assert!(counters.is_seeded("default"));

        // The import writes; a load charts a window with no history yet.
        counters.record("default", "Patient", 5, Utc::now());
        provider
            .snapshot(DashboardWindow::LastDay, "", &[], false)
            .await;

        let calls = storage.aggregate_calls();
        let mut schedule = ReconcileSchedule::new(StdDuration::from_secs(30));
        let report = provider.reconcile_pass(&mut schedule).await;
        assert_eq!(report.skipped_active_import, vec!["default".to_string()]);
        assert!(report.reconciled.is_empty());
        assert_eq!(report.rings_seeded, 0);
        assert_eq!(storage.aggregate_calls(), calls, "no storage aggregate ran");
        assert_eq!(
            lock(&provider.ring_seeds.pending).len(),
            1,
            "the ring seed stays queued"
        );

        let totals = counters.totals_view("default").expect("seeded");
        assert!(!totals.exact);
        assert_eq!(totals.totals, vec![("Patient".to_string(), 6)]);
    }

    /// The counter path and the storage path chart identical points for the
    /// same data, in every window — both when the counters were just seeded
    /// from storage and after a write recorded the way the REST handlers
    /// record it.
    #[tokio::test]
    async fn counter_and_storage_series_agree_for_the_same_data() {
        let backend = sqlite();
        let tenant = test_tenant();
        let doomed = create_in(backend.as_ref(), "Patient").await;
        create_in(backend.as_ref(), "Patient").await;
        create_in(backend.as_ref(), "Patient").await;
        backend
            .delete(&tenant, "Patient", doomed.id())
            .await
            .expect("delete");
        create_in(backend.as_ref(), "Observation").await;
        create_in(backend.as_ref(), "Observation").await;

        let counters = isolated_counters();
        let types = ["Patient", "Observation", "Encounter"];
        for window in DashboardWindow::ALL {
            let now = Utc::now();
            let token = counters.begin_reconcile("default");
            let totals = ResourceStorage::count_all_types(backend.as_ref(), &tenant)
                .await
                .expect("count_all_types");
            assert!(counters.finish_reconcile(token, &totals, now));
            let from_storage = resource_count_series_seeding(
                backend.as_ref(),
                &tenant,
                &types,
                SeriesWindow::from_dashboard_window(window),
                now,
                Some(RingSeeding { counters, window }),
            )
            .await
            .expect("storage series");
            let from_counters =
                resource_count_series_from_counters(counters, "default", window, &types, now)
                    .expect("counter series");
            assert_eq!(
                shape(&from_storage),
                shape(from_counters.iter().map(|c| &c.series)),
                "{}",
                window.as_str()
            );
            assert!(
                from_counters.iter().all(|c| c.history_seeded && c.exact),
                "{}",
                window.as_str()
            );
        }

        let recorded = create_in(backend.as_ref(), "Observation").await;
        counters.record("default", "Observation", 1, recorded.last_modified());
        let now = Utc::now();
        for window in DashboardWindow::ALL {
            let from_storage = resource_count_series(
                backend.as_ref(),
                &tenant,
                &types,
                SeriesWindow::from_dashboard_window(window),
                now,
            )
            .await
            .expect("storage series");
            let from_counters =
                resource_count_series_from_counters(counters, "default", window, &types, now)
                    .expect("counter series");
            assert_eq!(
                shape(&from_storage),
                shape(from_counters.iter().map(|c| &c.series)),
                "{}",
                window.as_str()
            );
            let observation = &from_counters[1];
            assert!(!observation.exact, "a recorded write is not reconciled");
            assert_eq!(observation.series.total, 3);
        }
    }

    /// The reconcile loop seeds the default tenant at startup, loads a ring a
    /// page load queued promptly (woken, not at its next tick), and stops once
    /// its provider is gone — so repeated `build_app` calls never leave loops
    /// behind.
    #[tokio::test]
    async fn reconcile_loop_seeds_at_startup_and_stops_with_its_provider() {
        let backend = sqlite();
        create_in(backend.as_ref(), "Patient").await;
        let counters = isolated_counters();
        let provider = Arc::new(
            StorageDashboardProvider::new(Arc::clone(&backend), &test_config())
                .with_counters(counters),
        );
        // An hour between passes: anything seeded below came from the startup
        // step or a wake-up.
        let handle = spawn_reconcile_loop_every(&provider, StdDuration::from_secs(3600))
            .expect("inside a runtime");

        assert!(
            eventually(|| {
                counters.is_seeded("default")
                    && DashboardWindow::ALL.iter().all(|window| {
                        counters
                            .series_view("default", *window, &["Patient"], Utc::now())
                            .is_some_and(|views| views[0].history_seeded && views[0].exact)
                    })
            })
            .await,
            "the default tenant and its default charted type are seeded at startup"
        );

        let snapshot = provider
            .snapshot(
                DashboardWindow::LastHour,
                "",
                &["Encounter".to_string()],
                true,
            )
            .await;
        assert!(snapshot.approximate, "Encounter's ring has no history yet");
        assert!(
            eventually(|| {
                counters
                    .series_view(
                        "default",
                        DashboardWindow::LastHour,
                        &["Encounter"],
                        Utc::now(),
                    )
                    .is_some_and(|views| views[0].history_seeded)
            })
            .await,
            "a queued ring seed is drained on wake-up"
        );
        assert!(!handle.is_finished());

        drop(provider);
        tokio::time::timeout(StdDuration::from_secs(5), handle)
            .await
            .expect("the loop stops once its provider is gone")
            .expect("the loop did not panic");
    }
}
