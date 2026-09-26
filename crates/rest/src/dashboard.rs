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
//! [`helios_observability::metrics`]). The authenticated
//! `/console/metrics/resource-counts` JSON handler reads its curves straight
//! from storage ([`resource_count_series`]); the provider builds the same
//! curves from the write counters with the same cumulative-bucketing helper
//! ([`cumulative_series`]), so those semantics live in exactly one place.
//!
//! # No storage query on a page load (#1078)
//!
//! Computing figures from storage means a `GROUP BY` over every live row of
//! the tenant plus bucketed history scans for the charted types. During a
//! multi-million-resource import those queries stop finishing within any page
//! budget, so the chart sat on "Waiting for the live figures…" exactly when an
//! operator most wanted to watch it move — and a page load that ran them
//! inline held a request, a connection and the backend's attention hostage.
//!
//! So [`DashboardProvider::snapshot`] never awaits storage. A page load gets
//! one of three snapshots, told apart by [`DashboardSnapshot::figures`]:
//!
//! - **Seeded tenant — served from memory.** Once a tenant's totals have been
//!   read from storage, every snapshot is built from the process-global
//!   [`DashboardCounters`] that the REST write handlers and `$bulk-submit`
//!   record into. When the totals and every charted series still match what
//!   storage last reported the figures are [`Figures::Exact`]; figures the
//!   counters cannot vouch for (writes recorded since the last reconcile, a
//!   change the storage write marker detected elsewhere, a purge not reseeded
//!   yet, or a window whose history ring has not been loaded from storage) are
//!   [`Figures::Approximate`] — measured, never invented (#956). A charted
//!   ring that lacks storage history is queued for a background seed.
//! - **Unseeded tenant — pending.** A tenant the counters have no storage base
//!   for gets [`Figures::Pending`] with empty figures, and a background seed of
//!   the tenant is queued (once, however many page loads ask). The page
//!   renders as waiting and is served from memory once the seed lands.
//! - **Backend that cannot count — unsupported.** When
//!   [`ResourceStorage::supports_type_counts`] is `false` (an S3 primary, for
//!   example) the count aggregates are empty defaults, so nothing can be
//!   measured: the snapshot is [`Figures::Unsupported`] with empty figures, and
//!   no tenant is ever seeded or reconciled.
//!
//! Every shape carries the bulk-export and bulk-submit job counts last read
//! for the tenant (`None` before the first read). Reading them is background
//! work too: a snapshot that finds them missing or older than
//! [`JOB_COUNTS_TTL`] queues a refresh, and the loop runs it.
//!
//! # Background seeding and reconcile
//!
//! [`spawn_reconcile_loop`] (wired next to the provider registration in
//! [`crate::build_app`]) runs every storage query the dashboard needs, one at
//! a time, on a single supervised task (a panicking pass is logged and the
//! task restarted with exponential back-off):
//!
//! - **Seeds.** A tenant's seed reads its totals (`count_all_types`) and then
//!   the history rings of its default charted types for every window, plus any
//!   ring charted recently that is not exact. Rings are loaded one window at a
//!   time: a single
//!   [`count_deltas_by_type_and_bucket`](ResourceStorage::count_deltas_by_type_and_bucket)
//!   query returns every type's buckets for that window, over the history
//!   log's `(tenant_id, last_updated)` range, so a seed costs one history
//!   query per window however many types it charts. A failed window query
//!   leaves all of that window's rings unloaded, to be retried with the next
//!   seed or re-seed of them. The default tenant is queued at
//!   startup, any other tenant when a page first asks for it, and a purged
//!   tenant as soon as [`DashboardCounters::invalidate_tenant`] marks it stale —
//!   its last figures stay on show, labelled approximate, until the reseed
//!   replaces them with storage's. A failed seed stays queued and is retried on
//!   the next pass; nothing a page does waits on it. The *first* seed of a
//!   tenant runs even while a bulk submit is active for it — seeds run one
//!   tenant at a time, and a tenant first viewed mid-import should leave its
//!   waiting state as soon as one grouping query finishes — while the reseed
//!   of an already seeded tenant, which has figures on show and whose counters
//!   follow the import, backs off like the periodic reconcile.
//! - **Reconciles.** Every reconcile interval
//!   (`HFS_DASHBOARD_RECONCILE_SECS`, [`ServerConfig::dashboard_reconcile_interval_secs`])
//!   each seeded tenant's totals are re-read when due, backing off while a
//!   bulk submit is active for the tenant and never spending more than about
//!   1/[`RECONCILE_DUTY_FACTOR`] of the time on one tenant's grouping query.
//!   The storage write marker
//!   ([`ResourceStorage::latest_write_marker`]) is read around every totals
//!   read, and probed once per pass for a tenant that is not due: a change
//!   this process did not record (a sibling instance, a direct database edit)
//!   labels the figures approximate and makes the totals due at once. The
//!   history rings of its charted types that are no longer exact are re-seeded
//!   when the tenant is quiet, or once their history is older than
//!   [`RESEED_MAX_AGE_FACTOR`] intervals even under continuous writes.
//! - **Ring seeds.** A page load that charts a ring without storage history
//!   queues it and wakes the loop, which loads it after a short debounce —
//!   deferred while a bulk submit is active for the tenant. The queued rings of
//!   a tenant are loaded with one grouped query per window, and a ring whose
//!   query failed stays queued for the next drain.
//! - **Bounded passes.** A pass visits tenants most recently viewed first
//!   ([`DashboardCounters::tenants_by_priority`]), stops after
//!   [`MAX_TENANTS_PER_PASS`] tenants or [`MAX_PASS_TIME`] (at most one
//!   interval), and starts the next pass with the tenants it skipped. The first
//!   seed of a tenant viewed within [`RECENT_VIEW`] is never skipped. Tenants
//!   nobody viewed for [`CHARTED_KEY_TTL`] are evicted (the default tenant is
//!   kept), and a deregistered tenant's state is dropped; either is seeded
//!   again on its next view.
//!
//! # Counters are process-local
//!
//! A multi-instance deployment sharing one PostgreSQL or MongoDB database only
//! records the writes that land on *this* instance. On backends with a write
//! marker, writes made elsewhere are detected by the next pass and labelled
//! approximate until the reconcile that follows; on the others (and for the
//! write paths that do not record, such as conformance seeding and history
//! deletes) they show up only after the next reconcile, and the snapshot may
//! be quietly behind until then.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration as StdDuration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use helios_observability::dashboard::{
    DashboardPoint, DashboardProvider, DashboardSeries, DashboardSnapshot, DashboardWindow,
    ExportJobCounts, Figures, ReindexActivity, TypeCount,
};
use helios_observability::dashboard_counters::{
    CountersSeries, DashboardCounters, ReconcileOutcome, StorageMarker,
};
use helios_observability::dashboard_metrics::{self, StorageQuery};
use helios_persistence::core::{
    BulkExportJobStore, BulkSubmitJobStore, ExportStatus, ResourceCountDelta, ResourceStorage,
    WriteMarker, bucket_floor,
};
use helios_persistence::error::StorageResult;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use tokio::sync::Notify;
use tracing::{debug, error, warn};

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
/// bounds the type list of the history reads. The default stays at three; this is how
/// far an explicit selection can go.
const MAX_CHARTED_TYPES: usize = 6;

/// How long a tenant's last read bulk-export and bulk-submit job counts are
/// served before a page load queues a background refresh of them (#1078).
/// They are small job-table reads, but a page load never awaits them: every
/// load (and every sibling snapshot key) asking for them is exactly the
/// traffic the database can least afford during an import, and a slow job
/// store must not hold a page. The loop refreshes a queued tenant within a
/// seed debounce, so the counts on show are at most a few seconds old while
/// someone watches.
const JOB_COUNTS_TTL: StdDuration = StdDuration::from_secs(5);

/// Upper bound on one job-store count or write-marker probe made by the
/// background loop. Both are meant to be index reads; one that overruns is
/// abandoned (and counted as a failed query) so a stuck job store or a
/// saturated connection pool cannot stall the whole pass. The previous value
/// is kept.
const PROBE_TIMEOUT: StdDuration = StdDuration::from_secs(2);

/// Duty-cycle bound on a tenant's totals reconcile: after a `count_all_types`
/// that took `t`, the tenant is not reconciled again for
/// `max(interval, t × RECONCILE_DUTY_FACTOR)`. A 30s grouping query at 6M rows
/// therefore runs at most every five minutes, so the reconcile never becomes a
/// standing load on a busy backend — including during a REST-driven load that
/// the bulk-submit back-off cannot see. Seeds are not held back by it (a
/// tenant with no figures, or stale ones after a purge, is read right away),
/// but they do record their read in it. A change the storage write marker
/// detects makes the totals due at once.
const RECONCILE_DUTY_FACTOR: u32 = 10;

/// How far before the previous totals reconcile began the write marker counts
/// recent writes. Covers writes committed with an earlier timestamp than the
/// newest history row (a lagging clock, a long transaction), which leave the
/// marker's `latest` unchanged.
const RECENT_WRITES_LOOKBACK: Duration = Duration::minutes(5);

/// A charted ring whose storage history is older than this many reconcile
/// intervals is re-seeded even while writes keep arriving, so a tenant under a
/// continuous load still converges instead of staying approximate forever.
const RESEED_MAX_AGE_FACTOR: u32 = 5;

/// How many tenants one pass (or one wake-up drain) visits at most. The rest
/// go first in the next pass.
const MAX_TENANTS_PER_PASS: usize = 64;

/// How long one pass (or one wake-up drain) keeps starting tenants' work,
/// capped by the reconcile interval. A tenant whose work already started
/// finishes it.
const MAX_PASS_TIME: StdDuration = StdDuration::from_secs(20);

/// A tenant viewed this recently has its *first* seed run even when the pass
/// budget is spent, so a page someone is looking at leaves its waiting state.
const RECENT_VIEW: StdDuration = StdDuration::from_secs(60);

/// How long the reconcile loop waits after being woken for a seed before
/// draining the queues, so a page that charts several types (or a burst of
/// window switches) is seeded in one batch.
const SEED_DEBOUNCE: StdDuration = StdDuration::from_millis(250);

/// How long a `(tenant, type, window)` stays on the re-seed list after it was
/// last charted, and how long a tenant nobody views keeps its counters before
/// it is evicted. Bounds both by recent user behaviour rather than by every
/// tenant and type anyone ever looked at.
const CHARTED_KEY_TTL: StdDuration = StdDuration::from_secs(60 * 60);

/// First pause before restarting a reconcile task that panicked; doubled on
/// each further panic up to [`RESTART_BACKOFF_MAX`].
const RESTART_BACKOFF_MIN: StdDuration = StdDuration::from_secs(1);

/// Longest pause before restarting a panicked reconcile task.
const RESTART_BACKOFF_MAX: StdDuration = StdDuration::from_secs(60);

/// A reconcile task that ran at least this long before panicking is
/// considered healthy: its restart starts over from [`RESTART_BACKOFF_MIN`].
const HEALTHY_RUN: StdDuration = StdDuration::from_secs(5 * 60);

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
/// This backs the console `resource-counts` JSON endpoint, which reads storage
/// on request. The web UI dashboard provider never calls it (#1078): it builds
/// the same curves from the write counters with the same
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
    let (first_bucket, last_bucket) = window.bounds(now);

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
        let deltas = storage
            .count_deltas_by_bucket(tenant, rt, first_bucket, window.bucket_seconds)
            .await?;

        // Keep only buckets inside the window (defensive against a
        // clock-skewed, future-dated `last_updated`).
        let in_window = deltas
            .iter()
            .filter(|d| d.bucket_start >= first_bucket && d.bucket_start <= last_bucket)
            .map(|d| (d.bucket_start, d.delta));

        series.push(cumulative_series(rt, total, window, now, in_window));
    }

    Ok(series)
}

/// Splits a grouped history read
/// ([`ResourceStorage::count_deltas_by_type_and_bucket`]) into each type's
/// `(bucket_start, delta)` list, keeping only buckets inside `window` as of
/// `now` (defensive against a clock-skewed, future-dated `last_updated`, as in
/// [`resource_count_series`]). A requested type with no rows has no entry;
/// callers seed it with an empty list.
fn history_deltas_by_type(
    rows: Vec<(String, ResourceCountDelta)>,
    window: SeriesWindow,
    now: DateTime<Utc>,
) -> HashMap<String, Vec<(DateTime<Utc>, i64)>> {
    let (first_bucket, last_bucket) = window.bounds(now);
    let mut by_type: HashMap<String, Vec<(DateTime<Utc>, i64)>> = HashMap::new();
    for (resource_type, delta) in rows {
        if delta.bucket_start >= first_bucket && delta.bucket_start <= last_bucket {
            by_type
                .entry(resource_type)
                .or_default()
                .push((delta.bucket_start, delta.delta));
        }
    }
    by_type
}

/// Groups `(type, window)` rings by window, in [`DashboardWindow::ALL`] order,
/// each with its distinct types sorted — one grouped history query per entry.
fn rings_by_window(
    rings: impl IntoIterator<Item = (String, DashboardWindow)>,
) -> Vec<(DashboardWindow, Vec<String>)> {
    let rings: Vec<(String, DashboardWindow)> = rings.into_iter().collect();
    DashboardWindow::ALL
        .iter()
        .filter_map(|window| {
            let types: BTreeSet<&String> = rings
                .iter()
                .filter(|(_, w)| w == window)
                .map(|(resource_type, _)| resource_type)
                .collect();
            (!types.is_empty()).then(|| (*window, types.into_iter().cloned().collect()))
        })
        .collect()
}

/// Turns one type's `total` and its bucketed net changes into the dense
/// cumulative curve [`resource_count_series`] documents — the single place the
/// baseline/cumulative arithmetic lives, shared by the console endpoint's
/// storage read and the dashboard's write-counter path so both chart identical
/// points for identical data.
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
/// totals.
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
    // grouped and counted — roughly doubling the cost of a read at 6M
    // resources. The counters' totals are still based on `count_all_types`.
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
    // Summed saturating so a pathological backend cannot panic the dashboard
    // on overflow.
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
/// else the largest few. Capped so the history-ring fan-out (one history read
/// per ring seed) and the palette stay bounded.
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

/// What the rebuild banner says for `tenant` among `jobs` (#1065, #1125).
///
/// Queued and in-progress rebuilds win, summed, so a retry that is running
/// shows its progress. With none running, the tenant's most recently finished
/// rebuild decides: one that failed, or completed with per-resource errors,
/// stays visible — searches keep missing those resources after the job stops —
/// until a later rebuild finishes cleanly or the job's status expires. A
/// cancelled rebuild shows nothing: stopping it was the operator's decision.
fn reindex_activity_of(
    jobs: &[helios_persistence::search::ReindexProgress],
    tenant: &str,
) -> Option<ReindexActivity> {
    use helios_persistence::search::ReindexStatus;

    let tenant_jobs = || {
        jobs.iter()
            .filter(move |job| job.tenant_id.as_deref() == Some(tenant))
    };
    let running: Vec<_> = tenant_jobs()
        .filter(|job| job.status.is_running())
        .collect();
    if !running.is_empty() {
        return Some(ReindexActivity::Running {
            jobs: running.len() as u64,
            processed: running.iter().map(|job| job.processed_resources).sum(),
            total: running.iter().map(|job| job.total_resources).sum(),
        });
    }
    // `completed_at` is RFC 3339 with a variable number of fractional digits,
    // so it is parsed rather than compared as text.
    let finished_at = |job: &helios_persistence::search::ReindexProgress| {
        job.completed_at
            .as_deref()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
    };
    let mut finished: Vec<_> = tenant_jobs()
        .filter(|job| job.status.is_finished())
        .collect();
    finished.sort_by_key(|job| std::cmp::Reverse(finished_at(job)));
    // A clean retry of named resources resolves the transient failures of the
    // rebuild before it, not that rebuild's permanent ones: look past it.
    let mut retried_cleanly = false;
    for job in finished {
        let clean = job.status == ReindexStatus::Completed && job.errors.is_empty();
        if job.resource_scoped && clean {
            retried_cleanly = true;
            continue;
        }
        let errors = if retried_cleanly {
            job.errors.iter().filter(|error| !error.retryable).count()
        } else {
            job.errors.len()
        };
        let unindexed = match job.status {
            ReindexStatus::Failed => true,
            ReindexStatus::Completed => errors > 0,
            _ => false,
        };
        return unindexed.then(|| ReindexActivity::Failed {
            job_id: job.job_id.clone(),
            errors: errors as u64,
        });
    }
    None
}

/// A tenant's last read job counts (see [`JOB_COUNTS_TTL`]).
#[derive(Clone, Copy, Default)]
struct JobCountsEntry {
    /// When the stores were last asked; `None` before the first read.
    fetched_at: Option<Instant>,
    /// Last successfully read export counts.
    export: Option<ExportJobCounts>,
    /// Last successfully read active-submission count.
    import: Option<u64>,
}

impl JobCountsEntry {
    /// Read within [`JOB_COUNTS_TTL`].
    fn is_fresh(&self) -> bool {
        self.fetched_at
            .is_some_and(|at| at.elapsed() < JOB_COUNTS_TTL)
    }
}

/// `(tenant, resource type, window)` — one history ring of the write counters.
type RingKey = (String, String, DashboardWindow);

/// The background work page loads ask for, drained by the reconcile loop.
#[derive(Default)]
struct SeedQueue {
    /// Tenants awaiting a seed: first viewed without a storage base, queued at
    /// startup, or marked stale by a purge. A set, so a tenant is queued (and
    /// the loop woken) once however many page loads ask; it leaves the set only
    /// when its seed succeeded.
    tenants: Mutex<HashSet<String>>,
    /// Rings a snapshot charted without storage history, awaiting their seed.
    /// A set for the same reason.
    rings: Mutex<HashSet<RingKey>>,
    /// Rings charted recently, with when — what a seed loads and a reconcile
    /// re-seeds once they stop being exact (see [`CHARTED_KEY_TTL`]).
    charted: Mutex<HashMap<RingKey, Instant>>,
    /// Tenants whose job counts a snapshot found missing or older than
    /// [`JOB_COUNTS_TTL`], awaiting a background refresh.
    job_counts: Mutex<HashSet<String>>,
    /// Wakes the reconcile loop when `tenants`, `rings` or `job_counts` gains
    /// a key.
    wake: Arc<Notify>,
}

/// What one pass of the background loop did, for its debug log and for tests.
#[derive(Debug, Default)]
pub(crate) struct ReconcileReport {
    /// Tenants seeded from storage: a first seed, or a reseed after a purge.
    pub(crate) seeded: Vec<String>,
    /// Tenants whose seed failed; they stay queued for the next pass.
    pub(crate) seed_failed: Vec<String>,
    /// Already seeded tenants whose reseed waits because a bulk submit is
    /// active for them; they stay stale until a later pass. A first seed is
    /// never deferred.
    pub(crate) seed_deferred: Vec<String>,
    /// Seeded tenants whose totals were re-read from storage.
    pub(crate) reconciled: Vec<String>,
    /// Seeded tenants skipped because a bulk submit is active for them.
    pub(crate) skipped_active_import: Vec<String>,
    /// Seeded tenants whose totals reconcile is not due yet (duty cycle).
    pub(crate) not_due: Vec<String>,
    /// Seeded tenants whose storage write marker showed a change this process
    /// did not record; their totals were made due at once.
    pub(crate) external_changes: Vec<String>,
    /// Tenants whose work waits for the next pass because this one spent its
    /// budget ([`MAX_TENANTS_PER_PASS`], [`MAX_PASS_TIME`]).
    pub(crate) budget_skipped: Vec<String>,
    /// Tenants whose provider state was dropped: evicted as idle, or no longer
    /// known to the counters (deregistered).
    pub(crate) dropped: Vec<String>,
    /// Tenants whose job counts were read from the job stores.
    pub(crate) job_counts_refreshed: Vec<String>,
    /// History rings loaded from storage during the pass.
    pub(crate) rings_seeded: usize,
}

/// The reconcile loop's own per-tenant bookkeeping: when each tenant's totals
/// may next be re-read (see [`RECONCILE_DUTY_FACTOR`]), which write-marker
/// bound its markers were read with, and which tenants the budget left for
/// the next pass. Owned by the loop.
pub(crate) struct ReconcileSchedule {
    interval: StdDuration,
    next_due: HashMap<String, Instant>,
    /// When each tenant's last totals reconcile began.
    last_begin: HashMap<String, DateTime<Utc>>,
    /// The `recent_since` bound the counters' current marker for the tenant
    /// was read with. Markers are only comparable when read with the same
    /// bound, so probes reuse it (see [`Self::reconcile_bound`]).
    marker_bound: HashMap<String, Option<DateTime<Utc>>>,
    /// Tenants a budget left out, oldest first: they go first next time.
    carried_over: Vec<String>,
    /// Tenants visited per pass at most ([`MAX_TENANTS_PER_PASS`]).
    max_tenants_per_pass: usize,
    /// Time after which a pass starts no further tenant (see
    /// [`MAX_PASS_TIME`]).
    pass_time: StdDuration,
    /// Idle time after which a tenant is evicted ([`CHARTED_KEY_TTL`]).
    idle_ttl: StdDuration,
}

impl ReconcileSchedule {
    pub(crate) fn new(interval: StdDuration) -> Self {
        Self {
            interval,
            next_due: HashMap::new(),
            last_begin: HashMap::new(),
            marker_bound: HashMap::new(),
            carried_over: Vec::new(),
            max_tenants_per_pass: MAX_TENANTS_PER_PASS,
            pass_time: interval.min(MAX_PASS_TIME),
            idle_ttl: CHARTED_KEY_TTL,
        }
    }

    fn is_due(&self, tenant: &str) -> bool {
        self.next_due
            .get(tenant)
            .is_none_or(|due| Instant::now() >= *due)
    }

    /// Makes `tenant`'s totals due now (an external change was detected).
    fn force_due(&mut self, tenant: &str) {
        self.next_due.remove(tenant);
    }

    /// Records a totals read of `elapsed` that just finished for `tenant`.
    fn record(&mut self, tenant: &str, elapsed: StdDuration) {
        let pause = self
            .interval
            .max(elapsed.saturating_mul(RECONCILE_DUTY_FACTOR));
        self.next_due
            .insert(tenant.to_string(), Instant::now() + pause);
    }

    /// The bound a between-reconcile marker probe of `tenant` uses: the one
    /// its reconciled marker was read with.
    fn probe_bound(&self, tenant: &str) -> Option<DateTime<Utc>> {
        self.marker_bound.get(tenant).copied().flatten()
    }

    /// The bound a totals reconcile of `tenant` reads its markers with, and
    /// notes it (with the reconcile's begin) for later probes.
    ///
    /// A marker's `recent_writes` depends on the bound, and the counters flag
    /// any marker that differs from the reconciled one as an external change.
    /// So the bound only moves forward (to [`RECENT_WRITES_LOOKBACK`] before
    /// the previous reconcile began) when `figures_exact` is `false`: the
    /// tenant is already approximate then, and the re-seed that follows covers
    /// its rings anyway. A tenant that stays exact keeps its bound, so a quiet
    /// tenant is never flagged by the bound moving. The first reconcile of a
    /// tenant asks for no recent-write count.
    fn reconcile_bound(
        &mut self,
        tenant: &str,
        figures_exact: bool,
        begin: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        let bound = match (self.marker_bound.get(tenant), figures_exact) {
            (Some(bound), true) => *bound,
            _ => self
                .last_begin
                .get(tenant)
                .map(|previous| *previous - RECENT_WRITES_LOOKBACK),
        };
        self.marker_bound.insert(tenant.to_string(), bound);
        self.last_begin.insert(tenant.to_string(), begin);
        bound
    }

    /// A fresh budget for one pass or drain.
    fn budget(&self) -> PassBudget {
        PassBudget {
            started: Instant::now(),
            visited: HashSet::new(),
        }
    }

    /// `candidates` in visiting order: the tenants a budget left out last time
    /// first, then by `priority` ([`DashboardCounters::tenants_by_priority`]),
    /// then the rest by name.
    fn order(&self, candidates: BTreeSet<String>, priority: &[String]) -> Vec<String> {
        let mut remaining = candidates;
        let mut ordered = Vec::with_capacity(remaining.len());
        for tenant in self.carried_over.iter().chain(priority) {
            if remaining.remove(tenant) {
                ordered.push(tenant.clone());
            }
        }
        ordered.extend(remaining);
        ordered
    }

    /// Whether `budget` lets the pass start work for `tenant`. A tenant
    /// already visited in this pass always may; an `exempt` one (a first seed
    /// someone is waiting for) may even when the budget is spent. A tenant
    /// turned away is carried over to go first next time.
    fn admit(&mut self, budget: &mut PassBudget, tenant: &str, exempt: bool) -> bool {
        if budget.visited.contains(tenant) {
            return true;
        }
        let spent = budget.visited.len() >= self.max_tenants_per_pass
            || budget.started.elapsed() >= self.pass_time;
        if spent && !exempt {
            if !self.carried_over.iter().any(|t| t == tenant) {
                self.carried_over.push(tenant.to_string());
            }
            return false;
        }
        budget.visited.insert(tenant.to_string());
        self.carried_over.retain(|t| t != tenant);
        true
    }

    /// Whether the pass's time budget is spent (for work not tied to a
    /// tenant's admission, such as job-count refreshes).
    fn out_of_time(&self, budget: &PassBudget) -> bool {
        budget.started.elapsed() >= self.pass_time
    }

    /// Drops everything kept for `tenant`.
    fn forget(&mut self, tenant: &str) {
        self.next_due.remove(tenant);
        self.last_begin.remove(tenant);
        self.marker_bound.remove(tenant);
        self.carried_over.retain(|t| t != tenant);
    }

    fn tenants(&self) -> impl Iterator<Item = &String> {
        self.next_due
            .keys()
            .chain(self.last_begin.keys())
            .chain(self.marker_bound.keys())
            .chain(self.carried_over.iter())
    }
}

/// The work one pass (or one wake-up drain) has started, against the limits
/// in [`ReconcileSchedule`].
struct PassBudget {
    started: Instant,
    /// Tenants whose work started in this pass.
    visited: HashSet<String>,
}

/// Why a snapshot carries no figures.
#[derive(Clone, Copy)]
enum NoFigures {
    /// The tenant's seed is queued ([`Figures::Pending`]).
    Pending,
    /// The backend cannot count ([`Figures::Unsupported`]).
    Unsupported,
}

/// `persistence`'s write marker in the counters' comparable form.
fn storage_marker(marker: WriteMarker) -> StorageMarker {
    StorageMarker {
        latest_millis: marker.latest.map(|at| at.timestamp_millis()),
        recent_writes: marker.recent_writes,
    }
}

/// Runs one small storage probe for the background loop with
/// [`PROBE_TIMEOUT`], recording it as a `query` storage query. `None` on an
/// error or a timeout, which is logged.
async fn timed_probe<T>(
    query: StorageQuery,
    tenant: &str,
    what: &'static str,
    probe: impl Future<Output = StorageResult<T>>,
) -> Option<T> {
    let started = Instant::now();
    let result = tokio::time::timeout(PROBE_TIMEOUT, probe).await;
    let elapsed = started.elapsed();
    match result {
        Ok(Ok(value)) => {
            dashboard_metrics::record_storage_query(query, elapsed, true);
            Some(value)
        }
        Ok(Err(error)) => {
            dashboard_metrics::record_storage_query(query, elapsed, false);
            warn!(
                %error,
                tenant = %tenant,
                probe = what,
                "dashboard reconcile: storage probe failed; keeping the previous value"
            );
            None
        }
        Err(_) => {
            dashboard_metrics::record_storage_query(query, elapsed, false);
            warn!(
                tenant = %tenant,
                probe = what,
                timeout_ms = PROBE_TIMEOUT.as_millis() as u64,
                "dashboard reconcile: storage probe timed out; keeping the previous value"
            );
            None
        }
    }
}

/// [`DashboardProvider`] backed by a live storage backend. Registered once in
/// [`crate::build_app`]; the tenant to chart arrives per call (#344), with the
/// server default as the fallback for an empty id.
pub(crate) struct StorageDashboardProvider<S> {
    default_tenant: String,
    fhir_version: String,
    storage: Arc<S>,
    /// Pause between reconcile passes
    /// ([`ServerConfig::dashboard_reconcile_interval_secs`]).
    reconcile_interval: StdDuration,
    /// Bulk-export job store, when the active backend provides one.
    export_jobs: Option<Arc<dyn BulkExportJobStore>>,
    /// Bulk-submit job store, when the active backend provides one.
    submit_jobs: Option<Arc<dyn BulkSubmitJobStore>>,
    /// The server's `$reindex` operation, when wired: its in-memory job
    /// registry is where a running search-index rebuild shows up (#1065).
    reindex: Option<Arc<helios_persistence::search::ReindexOperation>>,
    /// The live write counters every figure is served from (#1078): the set
    /// the server's write observer feeds, injected by `build_app`.
    counters: Arc<DashboardCounters>,
    /// Per-tenant last read job-store counts (see [`JOB_COUNTS_TTL`]).
    job_counts: Mutex<HashMap<String, JobCountsEntry>>,
    /// When each tenant was last viewed, for the budget exemption of a first
    /// seed (see [`RECENT_VIEW`]).
    viewed: Mutex<HashMap<String, Instant>>,
    /// Background seeds, drained by the reconcile loop.
    seeds: SeedQueue,
    /// The supervisor of the reconcile loop, aborted when the provider drops.
    reconcile_task: OnceLock<tokio::task::AbortHandle>,
}

impl<S> StorageDashboardProvider<S> {
    /// Builds a provider for the server's default tenant, default FHIR version
    /// and reconcile interval. The window and the charted types are chosen per
    /// request by the UI, so neither is fixed here; the default selection is
    /// the tenant's largest stored types (#555). Job-store counts start
    /// unwired; call [`Self::with_job_stores`] to attach them. Starts with a
    /// private, empty counter set; the server injects the shared one with
    /// [`Self::with_counters`].
    pub(crate) fn new(storage: Arc<S>, config: &ServerConfig) -> Self {
        Self {
            default_tenant: config.default_tenant.clone(),
            fhir_version: config.default_fhir_version.to_string(),
            storage,
            // `validate` rejects 0; a hand-built config still never spins.
            reconcile_interval: StdDuration::from_secs(
                config.dashboard_reconcile_interval_secs.max(1),
            ),
            export_jobs: None,
            submit_jobs: None,
            reindex: None,
            counters: Arc::new(DashboardCounters::new()),
            job_counts: Mutex::new(HashMap::new()),
            viewed: Mutex::new(HashMap::new()),
            seeds: SeedQueue::default(),
            reconcile_task: OnceLock::new(),
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

    /// Attaches the `$reindex` operation, whose running jobs — or last
    /// unfinished one — become each snapshot's
    /// [`DashboardSnapshot::reindex_active`] (#1065, #1125). `None` leaves
    /// that field `None`, so no rebuild banner is ever shown.
    pub(crate) fn with_reindex(
        mut self,
        reindex: Option<Arc<helios_persistence::search::ReindexOperation>>,
    ) -> Self {
        self.reindex = reindex;
        self
    }

    /// The tenant's running or last failed search-index rebuild. Reads the operation's
    /// in-memory job registry, never storage, so a page load stays
    /// constant-time (#1078).
    fn reindex_activity(&self, tenant: &str) -> Option<ReindexActivity> {
        reindex_activity_of(&self.reindex.as_ref()?.list_jobs(), tenant)
    }

    /// Replaces the write counters this provider reads and seeds — the set the
    /// server's post-commit write observer records into.
    pub(crate) fn with_counters(mut self, counters: Arc<DashboardCounters>) -> Self {
        self.counters = counters;
        self
    }

    /// Resolves the empty tenant id to the server's default tenant.
    fn tenant_or_default<'a>(&'a self, tenant: &'a str) -> &'a str {
        if tenant.is_empty() {
            self.default_tenant.as_str()
        } else {
            tenant
        }
    }

    /// Notes a dashboard view of `tenant`: it keeps the tenant from eviction,
    /// puts it first in the reconcile order, and exempts its first seed from
    /// the pass budget for [`RECENT_VIEW`].
    fn note_viewed(&self, tenant: &str) {
        let now = Instant::now();
        self.counters.note_viewed(tenant, now);
        lock(&self.viewed).insert(tenant.to_string(), now);
    }

    /// Whether `tenant` was viewed within `within`.
    fn viewed_within(&self, tenant: &str, within: StdDuration) -> bool {
        lock(&self.viewed)
            .get(tenant)
            .is_some_and(|at| at.elapsed() < within)
    }

    /// Notes that `types` were charted over `window` for `tenant`, so a seed
    /// or a reconcile keeps their history rings exact.
    fn note_charted(&self, tenant: &str, types: &[&str], window: DashboardWindow) {
        if types.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut charted = lock(&self.seeds.charted);
        for rt in types {
            charted.insert((tenant.to_string(), (*rt).to_string(), window), now);
        }
    }

    /// Queues a background seed of one tenant, waking the reconcile loop the
    /// first time it is queued. Never reads storage.
    fn enqueue_tenant_seed(&self, tenant: &str) {
        if lock(&self.seeds.tenants).insert(tenant.to_string()) {
            self.seeds.wake.notify_one();
        }
    }

    /// Queues a background seed of one history ring, waking the reconcile loop
    /// the first time the key is queued. Never reads storage.
    fn enqueue_ring_seed(&self, tenant: &str, resource_type: &str, window: DashboardWindow) {
        let inserted =
            lock(&self.seeds.rings).insert((tenant.to_string(), resource_type.to_string(), window));
        if inserted {
            self.seeds.wake.notify_one();
        }
    }

    /// The tenant's last read job counts, `None` for a count never read (or
    /// with no store wired). Never reads storage: when the counts are missing
    /// or older than [`JOB_COUNTS_TTL`], a background refresh is queued, waking
    /// the loop the first time.
    ///
    /// Counts degrade to the last successfully read value rather than zero on
    /// a read error: a zero here would tell an operator "no jobs" when the
    /// truth is "could not ask". They carry their own unavailable state on the
    /// page, so they never affect [`DashboardSnapshot::figures`].
    fn cached_job_counts(&self, tenant: &str) -> (Option<ExportJobCounts>, Option<u64>) {
        if self.export_jobs.is_none() && self.submit_jobs.is_none() {
            return (None, None);
        }
        let entry = lock(&self.job_counts)
            .get(tenant)
            .copied()
            .unwrap_or_default();
        if !entry.is_fresh() && lock(&self.seeds.job_counts).insert(tenant.to_string()) {
            self.seeds.wake.notify_one();
        }
        (entry.export, entry.import)
    }

    /// Recently charted `(type, window)` rings of `tenant`, in a stable order.
    fn charted_for(&self, tenant: &str) -> Vec<(String, DashboardWindow)> {
        let mut rings: Vec<(String, DashboardWindow)> = lock(&self.seeds.charted)
            .keys()
            .filter(|(t, _, _)| t == tenant)
            .map(|(_, rt, window)| (rt.clone(), *window))
            .collect();
        rings.sort_by(|a, b| (&a.0, a.1.as_str()).cmp(&(&b.0, b.1.as_str())));
        rings
    }

    /// The recently charted rings of `tenant` that are not exact — what a seed
    /// or a quiet reconcile loads from storage.
    fn inexact_charted_rings(&self, tenant: &str) -> Vec<(String, DashboardWindow)> {
        self.charted_for(tenant)
            .into_iter()
            .filter(|(resource_type, window)| {
                !self
                    .ring_view(tenant, resource_type, *window)
                    .is_some_and(|view| view.exact)
            })
            .collect()
    }

    /// The counters' view of one ring, if the tenant is seeded.
    fn ring_view(
        &self,
        tenant: &str,
        resource_type: &str,
        window: DashboardWindow,
    ) -> Option<CountersSeries> {
        self.counters
            .series_view(tenant, window, &[resource_type], Utc::now())
            .and_then(|mut views| views.pop())
    }

    /// Publishes the seed queue depth.
    fn publish_seed_queue(&self) {
        let rings = lock(&self.seeds.rings).len();
        dashboard_metrics::set_seed_queue(rings);
    }

    /// Drops the provider's state for tenants the counters evicted as idle
    /// (not viewed for `schedule.idle_ttl`; the default tenant is kept) and
    /// for tenants the counters no longer hold at all — a deregistered tenant
    /// whose state [`DashboardCounters::remove_tenant`] dropped. Either is
    /// seeded again on its next view.
    fn drop_stale_tenants(
        &self,
        schedule: &mut ReconcileSchedule,
        evict: bool,
        report: &mut ReconcileReport,
    ) {
        let mut dropped: BTreeSet<String> = if evict {
            self.counters
                .evict_idle(
                    schedule.idle_ttl,
                    Instant::now(),
                    &[self.default_tenant.as_str()],
                )
                .into_iter()
                .collect()
        } else {
            BTreeSet::new()
        };

        let present: HashSet<String> = self.counters.tenants_by_priority().into_iter().collect();
        let mut held: BTreeSet<String> = schedule.tenants().cloned().collect();
        held.extend(lock(&self.seeds.tenants).iter().cloned());
        held.extend(lock(&self.seeds.rings).iter().map(|(t, _, _)| t.clone()));
        held.extend(lock(&self.seeds.charted).keys().map(|(t, _, _)| t.clone()));
        held.extend(lock(&self.seeds.job_counts).iter().cloned());
        held.extend(lock(&self.job_counts).keys().cloned());
        held.extend(lock(&self.viewed).keys().cloned());
        dropped.extend(
            held.into_iter()
                .filter(|tenant| *tenant != self.default_tenant && !present.contains(tenant)),
        );

        for tenant in &dropped {
            schedule.forget(tenant);
            lock(&self.seeds.tenants).remove(tenant);
            lock(&self.seeds.rings).retain(|(t, _, _)| t != tenant);
            lock(&self.seeds.charted).retain(|(t, _, _), _| t != tenant);
            lock(&self.seeds.job_counts).remove(tenant);
            lock(&self.job_counts).remove(tenant);
            lock(&self.viewed).remove(tenant);
        }
        if !dropped.is_empty() {
            debug!(
                tenants = dropped.len(),
                "dashboard reconcile: dropped the state of idle or deregistered tenants"
            );
        }
        report.dropped.extend(dropped);
    }
}

/// Stops the reconcile loop, if one runs, and wakes it so a pass in flight
/// notices at its next step that its provider is gone.
impl<S> Drop for StorageDashboardProvider<S> {
    fn drop(&mut self) {
        if let Some(task) = self.reconcile_task.get() {
            task.abort();
        }
        self.seeds.wake.notify_one();
    }
}

impl<S> StorageDashboardProvider<S>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    /// Reads `tenant`'s storage write marker with `recent_since`, for the
    /// background loop. `None` when the backend has none, or on an error or a
    /// timeout (logged, and recorded as a failed query).
    async fn read_marker(
        &self,
        tenant: &str,
        recent_since: Option<DateTime<Utc>>,
    ) -> Option<StorageMarker> {
        let context = tenant_context(tenant);
        let started = Instant::now();
        let result = tokio::time::timeout(
            PROBE_TIMEOUT,
            self.storage.latest_write_marker(&context, recent_since),
        )
        .await;
        let elapsed = started.elapsed();
        match result {
            // The backend has no marker: nothing was queried.
            Ok(Ok(None)) => None,
            Ok(Ok(Some(marker))) => {
                dashboard_metrics::record_storage_query(StorageQuery::WriteMarker, elapsed, true);
                Some(storage_marker(marker))
            }
            Ok(Err(error)) => {
                dashboard_metrics::record_storage_query(StorageQuery::WriteMarker, elapsed, false);
                warn!(%error, tenant = %tenant, "dashboard reconcile: write marker read failed");
                None
            }
            Err(_) => {
                dashboard_metrics::record_storage_query(StorageQuery::WriteMarker, elapsed, false);
                warn!(
                    tenant = %tenant,
                    timeout_ms = PROBE_TIMEOUT.as_millis() as u64,
                    "dashboard reconcile: write marker read timed out"
                );
                None
            }
        }
    }

    /// Reads `tenant`'s per-type totals from storage as a counters reconcile
    /// (#1078), recording the read in `schedule`. Only the background loop
    /// calls this.
    ///
    /// The storage write marker is read right before the reconcile begins and
    /// right after `count_all_types`, so the counters can tell whether storage
    /// changed while it was counted (see
    /// [`DashboardCounters::finish_reconcile`]). The reconcile is finished only
    /// with the count's successful result, so an error never marks the tenant
    /// seeded. `Ok(false)` means the tenant was invalidated (purged), evicted
    /// or removed while the query ran: those figures may describe erased data,
    /// so the counters dropped them and the tenant still needs a reseed.
    async fn reconcile_totals(
        &self,
        tenant: &str,
        schedule: &mut ReconcileSchedule,
    ) -> StorageResult<bool> {
        let context = tenant_context(tenant);
        let figures_exact = self
            .counters
            .totals_view(tenant)
            .is_some_and(|totals| totals.exact);
        let read_at = Utc::now();
        let bound = schedule.reconcile_bound(tenant, figures_exact, read_at);
        let before = self.read_marker(tenant, bound).await;
        let token = self.counters.begin_reconcile(tenant, before);

        // Timed, so an operator staring at a slow backend can tell from the
        // logs (and the query metrics) which query is responsible (#959).
        let started = Instant::now();
        let result = self.storage.count_all_types(&context).await;
        let elapsed = started.elapsed();
        schedule.record(tenant, elapsed);
        dashboard_metrics::record_storage_query(StorageQuery::Totals, elapsed, result.is_ok());
        debug!(
            tenant = %tenant,
            elapsed_ms = elapsed.as_millis() as u64,
            ok = result.is_ok(),
            "dashboard reconcile: count_all_types completed"
        );
        let counts = result?;
        let after = self.read_marker(tenant, bound).await;

        match self
            .counters
            .finish_reconcile(token, &counts, after, read_at)
        {
            ReconcileOutcome::Applied { correction } => {
                dashboard_metrics::record_reconcile_correction(correction);
                Ok(true)
            }
            ReconcileOutcome::Superseded => Ok(true),
            ReconcileOutcome::Invalidated => {
                debug!(
                    tenant = %tenant,
                    "dashboard: tenant invalidated during count_all_types; counters not seeded"
                );
                Ok(false)
            }
        }
    }

    /// Reads `tenant`'s job counts from the job stores — background only, each
    /// store call bounded by [`PROBE_TIMEOUT`] — and caches them. A count that
    /// could not be read keeps its previous value.
    async fn refresh_job_counts(&self, tenant: &str) -> JobCountsEntry {
        let context = tenant_context(tenant);
        let previous = lock(&self.job_counts)
            .get(tenant)
            .copied()
            .unwrap_or_default();

        let export = match &self.export_jobs {
            None => None,
            Some(store) => {
                let running = timed_probe(
                    StorageQuery::JobCounts,
                    tenant,
                    "running exports",
                    store.count_exports_by_status(&context, ExportStatus::InProgress),
                )
                .await;
                let queued = timed_probe(
                    StorageQuery::JobCounts,
                    tenant,
                    "queued exports",
                    store.count_exports_by_status(&context, ExportStatus::Accepted),
                )
                .await;
                match (running, queued) {
                    (Some(running), Some(queued)) => Some(ExportJobCounts { running, queued }),
                    _ => previous.export,
                }
            }
        };

        let import = match &self.submit_jobs {
            None => None,
            Some(store) => timed_probe(
                StorageQuery::JobCounts,
                tenant,
                "active submissions",
                store.count_active_submissions(&context),
            )
            .await
            .or(previous.import),
        };

        let entry = JobCountsEntry {
            fetched_at: Some(Instant::now()),
            export,
            import,
        };
        lock(&self.job_counts).insert(tenant.to_string(), entry);
        entry
    }

    /// Refreshes the job counts snapshots queued, skipping any another path
    /// refreshed since, until the pass's time budget is spent (the rest stay
    /// queued).
    async fn refresh_queued_job_counts(
        &self,
        schedule: &ReconcileSchedule,
        budget: &PassBudget,
        report: &mut ReconcileReport,
    ) {
        let queued: BTreeSet<String> = lock(&self.seeds.job_counts).iter().cloned().collect();
        for tenant in queued {
            if schedule.out_of_time(budget) {
                break;
            }
            lock(&self.seeds.job_counts).remove(&tenant);
            let fresh = lock(&self.job_counts)
                .get(&tenant)
                .is_some_and(JobCountsEntry::is_fresh);
            if !fresh {
                self.refresh_job_counts(&tenant).await;
                report.job_counts_refreshed.push(tenant);
            }
        }
    }

    /// Whether a bulk submit is active for `tenant` — the background loop's
    /// back-off signal, refreshing the job counts when they are stale. `false`
    /// when no submit store is wired or its count is unknown.
    async fn import_active(&self, tenant: &str) -> bool {
        if self.submit_jobs.is_none() {
            return false;
        }
        let cached = lock(&self.job_counts)
            .get(tenant)
            .copied()
            .filter(JobCountsEntry::is_fresh);
        let entry = match cached {
            Some(entry) => entry,
            None => self.refresh_job_counts(tenant).await,
        };
        entry.import.is_some_and(|active| active > 0)
    }

    /// A snapshot built entirely from the write counters, or `None` when the
    /// tenant has never been reconciled from storage (#1078).
    ///
    /// Awaits nothing: totals, the picker list, every charted series and the
    /// job counts come from memory. A tenant marked stale by a purge is served
    /// its last figures, labelled approximate, and its reseed is queued.
    fn snapshot_from_counters(
        &self,
        tenant_key: &str,
        window: DashboardWindow,
        types: &[String],
        include_empty: bool,
    ) -> Option<DashboardSnapshot> {
        let started = Instant::now();
        let now = Utc::now();

        let totals = self.counters.totals_view(tenant_key)?;
        if totals.needs_reseed {
            self.enqueue_tenant_seed(tenant_key);
        }
        let TypeSummary {
            total_resources,
            distinct_types,
            available,
        } = summarize_type_counts(totals.totals, include_empty);
        let selection = select_charted_types(&available, types, include_empty);
        // `totals_view` just returned figures, so this is `Some` unless the
        // tenant was evicted or removed in between; then it is pending again.
        let counted = resource_count_series_from_counters(
            &self.counters,
            tenant_key,
            window,
            &selection,
            now,
        )?;
        self.note_charted(tenant_key, &selection, window);
        let charted_types = selection.len();

        // Approximate when a write was recorded, a change detected through the
        // storage write marker, or a purge ran since the totals or a charted
        // ring were last read from storage, or when a ring has no storage
        // history at all yet (#956: measured, but not an exact storage read).
        let mut exact = totals.exact;
        let mut awaiting_history = 0usize;
        let mut series = Vec::with_capacity(counted.len());
        for entry in counted {
            exact &= entry.exact;
            if !entry.history_seeded {
                awaiting_history += 1;
                self.enqueue_ring_seed(tenant_key, &entry.series.resource_type, window);
            }
            series.push(entry.series);
        }
        let figures = if exact {
            Figures::Exact { read_at: now }
        } else {
            Figures::Approximate {
                read_at: now,
                reconciled_at: totals.reconciled_at,
            }
        };

        let (export_jobs, import_jobs_active) = self.cached_job_counts(tenant_key);

        // The acceptance signal for #1078: a page load logs this line (or the
        // pending one below) and never a storage-query timing.
        debug!(
            tenant = %tenant_key,
            window = window.as_str(),
            charted_types,
            exact,
            awaiting_history,
            needs_reseed = totals.needs_reseed,
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
            reindex_active: self.reindex_activity(tenant_key),
            figures,
        })
    }

    /// A snapshot with no figures — pending or unsupported — carrying only the
    /// job counts. Pending also queues the tenant's background seed.
    fn snapshot_without_figures(
        &self,
        tenant_key: &str,
        window: DashboardWindow,
        reason: NoFigures,
    ) -> DashboardSnapshot {
        let figures = match reason {
            NoFigures::Pending => {
                self.enqueue_tenant_seed(tenant_key);
                debug!(
                    tenant = %tenant_key,
                    window = window.as_str(),
                    "dashboard snapshot: tenant not seeded yet; background seed queued"
                );
                Figures::Pending
            }
            NoFigures::Unsupported => {
                debug!(
                    tenant = %tenant_key,
                    backend = self.storage.backend_name(),
                    "dashboard snapshot: storage backend cannot count resources"
                );
                Figures::Unsupported
            }
        };
        let (export_jobs, import_jobs_active) = self.cached_job_counts(tenant_key);
        DashboardSnapshot {
            fhir_version: self.fhir_version.clone(),
            total_resources: 0,
            distinct_types: 0,
            window,
            series: Vec::new(),
            available: Vec::new(),
            export_jobs,
            import_jobs_active,
            reindex_active: self.reindex_activity(tenant_key),
            figures,
        }
    }

    /// Loads the history rings of `types` over one `window` from storage in the
    /// background, with a single grouped history query for all of them
    /// (#1078).
    ///
    /// Every type's ring seed begins before the query and each finishes after
    /// it with that type's buckets — none when storage has no non-zero bucket
    /// for it, which is a valid seed of an empty history. Returns how many
    /// rings were seeded, or `None` when the query failed: that logs and leaves
    /// every one of those rings as it was (their begun seeds are simply never
    /// finished).
    async fn seed_window_rings(
        &self,
        tenant: &str,
        window: DashboardWindow,
        types: &[String],
    ) -> Option<usize> {
        if types.is_empty() {
            return Some(0);
        }
        let context = tenant_context(tenant);
        let series_window = SeriesWindow::from_dashboard_window(window);
        let now = Utc::now();
        let (first_bucket, _) = series_window.bounds(now);

        let tokens: Vec<_> = types
            .iter()
            .map(|resource_type| self.counters.begin_ring_seed(tenant, resource_type, window))
            .collect();
        let type_refs: Vec<&str> = types.iter().map(String::as_str).collect();
        let started = Instant::now();
        let result = self
            .storage
            .count_deltas_by_type_and_bucket(
                &context,
                &type_refs,
                first_bucket,
                series_window.bucket_seconds,
            )
            .await;
        let elapsed = started.elapsed();
        dashboard_metrics::record_storage_query(StorageQuery::History, elapsed, result.is_ok());
        debug!(
            tenant = %tenant,
            window = window.as_str(),
            types = types.len(),
            elapsed_ms = elapsed.as_millis() as u64,
            ok = result.is_ok(),
            "dashboard reconcile: grouped history ring read completed"
        );
        match result {
            Ok(rows) => {
                let mut by_type = history_deltas_by_type(rows, series_window, now);
                let seeded = types
                    .iter()
                    .zip(tokens)
                    .map(|(resource_type, token)| {
                        let deltas = by_type.remove(resource_type.as_str()).unwrap_or_default();
                        self.counters.finish_ring_seed(token, &deltas, now)
                    })
                    .filter(|seeded| *seeded)
                    .count();
                Some(seeded)
            }
            Err(error) => {
                warn!(
                    %error,
                    tenant = %tenant,
                    window = window.as_str(),
                    types = ?types,
                    "dashboard reconcile: grouped history ring read failed; keeping the previous rings"
                );
                None
            }
        }
    }

    /// Loads `rings` of `tenant` from storage: one
    /// [`seed_window_rings`](Self::seed_window_rings) query per window for all
    /// of that window's types. Returns how many rings were seeded; a window
    /// whose query failed seeds none.
    async fn seed_rings(&self, tenant: &str, rings: Vec<(String, DashboardWindow)>) -> usize {
        let mut seeded = 0usize;
        for (window, types) in rings_by_window(rings) {
            seeded += self
                .seed_window_rings(tenant, window, &types)
                .await
                .unwrap_or(0);
        }
        seeded
    }

    /// Seeds one tenant from storage: its totals, then every ring worth
    /// loading that is not exact — the default charted types' rings for every
    /// window (noted as charted, as a page load would) plus the rings charted
    /// recently. Returns how many rings were loaded, `Ok(None)` when the tenant
    /// was invalidated during the totals read, or the totals read's error (no
    /// ring is read then).
    async fn seed_tenant(
        &self,
        tenant: &str,
        schedule: &mut ReconcileSchedule,
    ) -> StorageResult<Option<usize>> {
        let started = Instant::now();
        if !self.reconcile_totals(tenant, schedule).await? {
            return Ok(None);
        }
        let Some(totals) = self.counters.totals_view(tenant) else {
            return Ok(None);
        };
        let summary = summarize_type_counts(totals.totals, false);
        for resource_type in select_charted_types(&summary.available, &[], false) {
            for window in DashboardWindow::ALL {
                self.note_charted(tenant, &[resource_type], window);
            }
        }
        let rings_seeded = self
            .seed_rings(tenant, self.inexact_charted_rings(tenant))
            .await;
        debug!(
            tenant = %tenant,
            rings_seeded,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "dashboard reconcile: tenant seeded"
        );
        Ok(Some(rings_seeded))
    }

    /// Runs the queued tenant seeds, plus the reseed of every seeded tenant a
    /// purge marked stale (so a purge is reseeded even while nobody looks at
    /// the dashboard), one tenant at a time, in [`ReconcileSchedule::order`].
    /// A tenant leaves the queue only when its seed succeeded; a failed,
    /// deferred or budget-skipped seed is retried on the next drain or pass.
    ///
    /// A first seed runs even while a bulk submit is active for the tenant:
    /// it has nothing on show until the seed lands, and one grouping query at
    /// a time is a bounded cost. For the same reason the first seed of a
    /// tenant viewed within [`RECENT_VIEW`] runs even when the budget is spent.
    /// A reseed of an already seeded tenant backs off during an import like
    /// the periodic reconcile: its last figures stay on show, labelled
    /// approximate, and its counters follow the import.
    async fn seed_pending_tenants(
        &self,
        schedule: &mut ReconcileSchedule,
        budget: &mut PassBudget,
        report: &mut ReconcileReport,
    ) {
        let mut candidates: BTreeSet<String> = lock(&self.seeds.tenants).iter().cloned().collect();
        candidates.extend(
            self.counters
                .tenants()
                .into_iter()
                .filter(|t| self.counters.is_seeded(t) && self.counters.needs_reseed(t)),
        );
        if candidates.is_empty() {
            return;
        }
        let order = schedule.order(candidates, &self.counters.tenants_by_priority());

        for tenant in order {
            let first_seed = !self.counters.is_seeded(&tenant);
            if !first_seed && !self.counters.needs_reseed(&tenant) {
                // Queued twice, and already seeded by the first.
                lock(&self.seeds.tenants).remove(&tenant);
                continue;
            }
            let exempt = first_seed && self.viewed_within(&tenant, RECENT_VIEW);
            if !schedule.admit(budget, &tenant, exempt) {
                report.budget_skipped.push(tenant);
                continue;
            }
            if !first_seed && self.import_active(&tenant).await {
                debug!(
                    tenant = %tenant,
                    "dashboard reconcile: bulk submit active; tenant reseed deferred"
                );
                report.seed_deferred.push(tenant);
                continue;
            }
            match self.seed_tenant(&tenant, schedule).await {
                Ok(Some(rings)) => {
                    report.rings_seeded += rings;
                    // Dequeued only after the read, so a page load during it
                    // does not queue the tenant again.
                    lock(&self.seeds.tenants).remove(&tenant);
                    report.seeded.push(tenant);
                }
                Ok(None) => debug!(
                    tenant = %tenant,
                    "dashboard reconcile: tenant purged during its seed; reseed stays queued"
                ),
                Err(error) => {
                    warn!(
                        %error,
                        tenant = %tenant,
                        first_seed,
                        "dashboard reconcile: tenant seed failed; retrying on the next pass"
                    );
                    report.seed_failed.push(tenant);
                }
            }
        }
    }

    /// Runs everything page loads queued: job-count refreshes, tenant seeds
    /// (see [`Self::seed_pending_tenants`]), then ring seeds, within one pass
    /// budget. What the loop does when woken.
    pub(crate) async fn drain_pending_seeds(
        &self,
        schedule: &mut ReconcileSchedule,
    ) -> ReconcileReport {
        let mut report = ReconcileReport::default();
        let mut budget = schedule.budget();
        self.refresh_queued_job_counts(schedule, &budget, &mut report)
            .await;
        if self.storage.supports_type_counts() {
            self.drop_stale_tenants(schedule, false, &mut report);
            self.seed_pending_tenants(schedule, &mut budget, &mut report)
                .await;
            report.rings_seeded += self
                .drain_ring_seeds(schedule, &mut budget, &mut report)
                .await;
        }
        self.publish_seed_queue();
        report
    }

    /// The reconcile loop's startup step: queues the default tenant's seed and
    /// runs the queue, so the first dashboard view after a restart is usually
    /// served from memory. At a very large store this is one background
    /// `GROUP BY` plus one grouped history read per window; no page waits on it
    /// (a page load meanwhile gets a pending snapshot), and a failure leaves
    /// the tenant queued for the next pass.
    pub(crate) async fn seed_default_tenant(
        &self,
        schedule: &mut ReconcileSchedule,
    ) -> ReconcileReport {
        if !self.storage.supports_type_counts() {
            return ReconcileReport::default();
        }
        // Queued without waking the loop: this call is the loop.
        lock(&self.seeds.tenants).insert(self.default_tenant.clone());
        let report = self.drain_pending_seeds(schedule).await;
        debug!(
            tenant = %self.default_tenant,
            seeded = !report.seeded.is_empty(),
            rings_seeded = report.rings_seeded,
            "dashboard reconcile: startup seed of the default tenant completed"
        );
        report
    }

    /// [`Self::drain_ring_seeds`] with a fresh schedule and budget.
    #[cfg(test)]
    pub(crate) async fn drain_pending_ring_seeds(&self) -> usize {
        let mut schedule = ReconcileSchedule::new(self.reconcile_interval);
        let mut budget = schedule.budget();
        let mut report = ReconcileReport::default();
        self.drain_ring_seeds(&mut schedule, &mut budget, &mut report)
            .await
    }

    /// Seeds the queued history rings, one tenant at a time (within `budget`)
    /// and one grouped query per window for all of a tenant's queued types in
    /// it, leaving queued the rings of tenants with an active bulk submit or
    /// beyond the budget, and the rings whose query failed (retried on the
    /// next drain). Returns how many were seeded.
    async fn drain_ring_seeds(
        &self,
        schedule: &mut ReconcileSchedule,
        budget: &mut PassBudget,
        report: &mut ReconcileReport,
    ) -> usize {
        let keys: Vec<RingKey> = lock(&self.seeds.rings).iter().cloned().collect();

        let mut by_tenant: BTreeMap<String, Vec<(String, DashboardWindow)>> = BTreeMap::new();
        for key in keys {
            let (tenant, resource_type, window) = &key;
            if self
                .ring_view(tenant, resource_type, *window)
                .is_some_and(|view| view.history_seeded)
            {
                // A tenant seed loaded it since it was queued.
                lock(&self.seeds.rings).remove(&key);
                continue;
            }
            let (tenant, resource_type, window) = key;
            by_tenant
                .entry(tenant)
                .or_default()
                .push((resource_type, window));
        }
        if by_tenant.is_empty() {
            return 0;
        }
        let order = schedule.order(
            by_tenant.keys().cloned().collect(),
            &self.counters.tenants_by_priority(),
        );

        let mut seeded = 0usize;
        for tenant in order {
            let Some(rings) = by_tenant.remove(&tenant) else {
                continue;
            };
            if !schedule.admit(budget, &tenant, false) {
                report.budget_skipped.push(tenant);
                continue;
            }
            if self.import_active(&tenant).await {
                debug!(
                    tenant = %tenant,
                    rings = rings.len(),
                    "dashboard reconcile: bulk submit active; history ring seeds deferred"
                );
                continue;
            }
            for (window, types) in rings_by_window(rings) {
                let Some(n) = self.seed_window_rings(&tenant, window, &types).await else {
                    // Still queued: the next drain retries the window.
                    continue;
                };
                seeded += n;
                // Dequeued only after the read, so a page load during it does
                // not queue the same rings again.
                let mut queued = lock(&self.seeds.rings);
                for resource_type in types {
                    queued.remove(&(tenant.clone(), resource_type, window));
                }
            }
        }
        seeded
    }

    /// One pass of the background loop (#1078).
    ///
    /// Sequential by design — one tenant, one query at a time — so the pass
    /// never competes much with writers:
    ///
    /// 1. **Job counts** snapshots queued are refreshed.
    /// 2. **Housekeeping** (also when the backend cannot count). Tenants not viewed for [`CHARTED_KEY_TTL`] are
    ///    evicted (the default tenant is kept), and the provider forgets every
    ///    tenant the counters no longer hold (deregistered).
    /// 3. **Seeds.** Queued tenant seeds and purge reseeds run (see
    ///    [`Self::seed_pending_tenants`]); this is where a failed seed is
    ///    retried.
    /// 4. **Per seeded tenant**, most recently viewed first (tenants the last
    ///    pass's budget skipped before them), skipping one still awaiting its
    ///    reseed, until the budget is spent:
    ///    - **Back-off.** A tenant with an active bulk submit is skipped
    ///      entirely: its grouping query is exactly what the import keeps from
    ///      finishing, and the counters already follow the import.
    ///    - **External changes.** When its totals are not due, the storage
    ///      write marker is probed once; a change this process did not record
    ///      makes them due now.
    ///    - **Totals.** When due (see [`RECONCILE_DUTY_FACTOR`]),
    ///      `count_all_types` is re-read as a counters reconcile. An error logs
    ///      and keeps the previous counters.
    ///    - **Rings.** When the tenant is quiet — its totals exact — each
    ///      recently charted ring that is not exact is re-seeded (one grouped
    ///      history query per window), which is what lets the snapshot become
    ///      exact after an import. Under continuous writes the rings
    ///      [`DashboardCounters::rings_to_reseed`] names are re-seeded instead:
    ///      those whose history is older than [`RESEED_MAX_AGE_FACTOR`]
    ///      intervals, or that only an external change made inexact.
    /// 5. **Ring seeds.** The queued ring seeds are drained.
    ///
    /// Only the job counts are refreshed when the backend cannot count.
    pub(crate) async fn reconcile_pass(&self, schedule: &mut ReconcileSchedule) -> ReconcileReport {
        let started = Instant::now();
        let mut report = ReconcileReport::default();
        let mut budget = schedule.budget();
        self.refresh_queued_job_counts(schedule, &budget, &mut report)
            .await;
        self.drop_stale_tenants(schedule, true, &mut report);
        if !self.storage.supports_type_counts() {
            dashboard_metrics::record_reconcile_pass(started.elapsed());
            return report;
        }
        lock(&self.seeds.charted).retain(|_, at| at.elapsed() < CHARTED_KEY_TTL);

        self.seed_pending_tenants(schedule, &mut budget, &mut report)
            .await;

        let max_age = Duration::from_std(schedule.interval.saturating_mul(RESEED_MAX_AGE_FACTOR))
            .unwrap_or(Duration::MAX);
        let priority = self.counters.tenants_by_priority();
        let order = schedule.order(priority.iter().cloned().collect(), &priority);
        for tenant in order {
            if !self.counters.is_seeded(&tenant) {
                // Recorded writes or a view but no seed yet: its view queues it.
                continue;
            }
            if self.counters.needs_reseed(&tenant) {
                // Its reseed failed or was deferred above; retried next pass.
                continue;
            }
            if !schedule.admit(&mut budget, &tenant, false) {
                report.budget_skipped.push(tenant);
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

            let mut due = schedule.is_due(&tenant);
            if !due
                && let Some(marker) = self
                    .read_marker(&tenant, schedule.probe_bound(&tenant))
                    .await
                && self.counters.note_marker(&tenant, marker)
            {
                debug!(
                    tenant = %tenant,
                    "dashboard reconcile: storage changed outside this process; totals due now"
                );
                schedule.force_due(&tenant);
                report.external_changes.push(tenant.clone());
                due = true;
            }
            if due {
                match self.reconcile_totals(&tenant, schedule).await {
                    Ok(true) => report.reconciled.push(tenant.clone()),
                    // Invalidated or removed while the query ran: the next
                    // pass reseeds or forgets it.
                    Ok(false) => continue,
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
            let rings = if quiet {
                self.inexact_charted_rings(&tenant)
            } else {
                self.counters.rings_to_reseed(
                    &tenant,
                    &self.charted_for(&tenant),
                    max_age,
                    Utc::now(),
                )
            };
            if !rings.is_empty() {
                report.rings_seeded += self.seed_rings(&tenant, rings).await;
            }
        }

        report.rings_seeded += self
            .drain_ring_seeds(schedule, &mut budget, &mut report)
            .await;
        self.publish_seed_queue();
        let elapsed = started.elapsed();
        dashboard_metrics::record_reconcile_pass(elapsed);
        debug!(
            seeded = report.seeded.len(),
            seed_failed = report.seed_failed.len(),
            seed_deferred = report.seed_deferred.len(),
            reconciled = report.reconciled.len(),
            skipped_active_import = report.skipped_active_import.len(),
            not_due = report.not_due.len(),
            external_changes = report.external_changes.len(),
            budget_skipped = report.budget_skipped.len(),
            dropped = report.dropped.len(),
            job_counts_refreshed = report.job_counts_refreshed.len(),
            rings_seeded = report.rings_seeded,
            elapsed_ms = elapsed.as_millis() as u64,
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
    /// Never awaits storage (#1078): see the [module documentation](self) for
    /// the three shapes a snapshot takes.
    async fn snapshot(
        &self,
        window: DashboardWindow,
        tenant: &str,
        types: &[String],
        include_empty: bool,
    ) -> DashboardSnapshot {
        let tenant_key = self.tenant_or_default(tenant);
        self.note_viewed(tenant_key);

        if !self.storage.supports_type_counts() {
            return self.snapshot_without_figures(tenant_key, window, NoFigures::Unsupported);
        }
        if let Some(snapshot) =
            self.snapshot_from_counters(tenant_key, window, types, include_empty)
        {
            return snapshot;
        }
        self.snapshot_without_figures(tenant_key, window, NoFigures::Pending)
    }
}

/// Spawns the dashboard's background seeding and reconcile for `provider`
/// (#1078), every [`ServerConfig::dashboard_reconcile_interval_secs`]; see the
/// [module documentation](self) for what it does.
///
/// Called from [`crate::build_app`] right after the provider is registered.
/// The task is supervised (see [`supervise_reconcile_loop`]) and owned by the
/// provider, which aborts it when dropped; the loop itself holds only a
/// [`Weak`] reference, so once the provider is no longer registered (a later
/// `build_app` replaced it) and its last in-flight compute has released it,
/// the loop stops — repeated app construction, as in the test suites, never
/// accumulates loops. It also stops at once when the backend cannot count and
/// no job store is wired. Does nothing outside a Tokio runtime (`build_app` is
/// synchronous and may be called from non-async contexts); nothing seeds the
/// counters then, so every snapshot of that provider stays pending.
pub(crate) fn spawn_reconcile_loop<S>(provider: &Arc<StorageDashboardProvider<S>>)
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let interval = provider.reconcile_interval;
    // The supervisor's handle is only for tests: the provider owns the task.
    drop(spawn_reconcile_loop_every(provider, interval));
}

/// [`spawn_reconcile_loop`] with the interval injected, returning the
/// supervisor's handle.
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
             counters seeding and reconcile. Dashboard figures will stay pending."
        );
        return None;
    }
    dashboard_metrics::set_reconcile_interval(interval);
    let handle = tokio::spawn(supervise_reconcile_loop(
        Arc::downgrade(provider),
        Arc::clone(&provider.seeds.wake),
        interval,
    ));
    if provider.reconcile_task.set(handle.abort_handle()).is_err() {
        // Already running for this provider: keep the first loop.
        handle.abort();
    }
    Some(handle)
}

/// Aborts the task it holds when dropped, so aborting the supervisor also
/// stops the worker it awaits.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The text of a panic payload.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

/// Runs [`run_reconcile_loop`] as a worker task and restarts it when it
/// panics (#1078): the panic is logged and counted
/// (`dashboard_reconcile_restarts_total`), and the worker restarts after an
/// exponential back-off from [`RESTART_BACKOFF_MIN`] to
/// [`RESTART_BACKOFF_MAX`], reset once a worker ran for [`HEALTHY_RUN`].
/// Stops when the worker returns (its provider is gone, or there is nothing to
/// do), when it is cancelled, or when the provider is gone by restart time.
async fn supervise_reconcile_loop<S>(
    provider: Weak<StorageDashboardProvider<S>>,
    wake: Arc<Notify>,
    interval: StdDuration,
) where
    S: ResourceStorage + Send + Sync + 'static,
{
    let mut backoff = RESTART_BACKOFF_MIN;
    loop {
        let started = Instant::now();
        let mut worker = AbortOnDrop(tokio::spawn(run_reconcile_loop(
            provider.clone(),
            Arc::clone(&wake),
            interval,
        )));
        let error = match (&mut worker.0).await {
            Ok(()) => return,
            Err(error) if error.is_cancelled() => return,
            Err(error) => error,
        };
        if started.elapsed() >= HEALTHY_RUN {
            backoff = RESTART_BACKOFF_MIN;
        }
        let message = match error.try_into_panic() {
            Ok(payload) => panic_message(payload.as_ref()),
            Err(error) => error.to_string(),
        };
        error!(
            panic = %message,
            restart_in_ms = backoff.as_millis() as u64,
            "dashboard reconcile loop panicked; restarting it"
        );
        dashboard_metrics::record_reconcile_restart();
        tokio::time::sleep(backoff).await;
        backoff = backoff.saturating_mul(2).min(RESTART_BACKOFF_MAX);
        if provider.strong_count() == 0 {
            debug!("dashboard reconcile: provider gone; not restarting the loop");
            return;
        }
    }
}

/// The loop body: the startup seed, then a [`reconcile
/// pass`](StorageDashboardProvider::reconcile_pass) every `interval`, draining
/// queued seeds and job-count refreshes in between whenever a page load wakes
/// it.
async fn run_reconcile_loop<S>(
    provider: Weak<StorageDashboardProvider<S>>,
    wake: Arc<Notify>,
    interval: StdDuration,
) where
    S: ResourceStorage + Send + Sync + 'static,
{
    let mut schedule = ReconcileSchedule::new(interval);
    match provider.upgrade() {
        Some(provider)
            if !provider.storage.supports_type_counts()
                && provider.export_jobs.is_none()
                && provider.submit_jobs.is_none() =>
        {
            debug!(
                backend = provider.storage.backend_name(),
                "dashboard reconcile: storage backend cannot count resources and no job \
                 store is wired; not running"
            );
            return;
        }
        Some(provider) => {
            provider.seed_default_tenant(&mut schedule).await;
        }
        None => return,
    }

    let mut next_pass = tokio::time::Instant::now() + interval;
    loop {
        let woken = tokio::select! {
            () = tokio::time::sleep_until(next_pass) => false,
            () = wake.notified() => true,
        };
        if woken {
            tokio::time::sleep(SEED_DEBOUNCE).await;
        }
        let Some(provider) = provider.upgrade() else {
            debug!("dashboard reconcile: provider no longer registered; stopping");
            return;
        };
        if woken {
            provider.drain_pending_seeds(&mut schedule).await;
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
    use futures::FutureExt;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::WriteEvent;
    use helios_persistence::core::WriteObserver;
    use helios_persistence::error::{BackendError, StorageError};
    use helios_persistence::types::StoredResource;
    use serde_json::Value;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    fn reindex_job(
        tenant: &str,
        status: helios_persistence::search::ReindexStatus,
        processed: u64,
        total: u64,
    ) -> helios_persistence::search::ReindexProgress {
        let mut job = helios_persistence::search::ReindexProgress::new(format!(
            "{tenant}-{status:?}-{processed}-{total}"
        ));
        job.tenant_id = Some(tenant.to_string());
        job.status = status;
        job.processed_resources = processed;
        job.total_resources = total;
        job
    }

    /// `job`, finished `minutes_ago`.
    fn finished(
        mut job: helios_persistence::search::ReindexProgress,
        minutes_ago: i64,
    ) -> helios_persistence::search::ReindexProgress {
        job.completed_at =
            Some((chrono::Utc::now() - chrono::Duration::minutes(minutes_ago)).to_rfc3339());
        job
    }

    fn resource_error(id: &str) -> helios_persistence::search::reindex::ReindexProgressError {
        helios_persistence::search::reindex::ReindexProgressError {
            resource_type: "Provenance".to_string(),
            resource_id: id.to_string(),
            error: "backend unavailable: elasticsearch".to_string(),
            retryable: true,
        }
    }

    #[test]
    fn reindex_activity_sums_only_the_tenants_running_jobs() {
        use helios_persistence::search::ReindexStatus;

        let jobs = [
            reindex_job("acme", ReindexStatus::InProgress, 250, 1_000),
            reindex_job("acme", ReindexStatus::Queued, 0, 0),
            finished(reindex_job("acme", ReindexStatus::Completed, 500, 500), 1),
            reindex_job("other", ReindexStatus::InProgress, 9, 10),
        ];
        assert_eq!(
            reindex_activity_of(&jobs, "acme"),
            Some(ReindexActivity::Running {
                jobs: 2,
                processed: 250,
                total: 1_000,
            })
        );
        assert_eq!(reindex_activity_of(&jobs, "idle"), None);
        assert_eq!(
            reindex_activity_of(&jobs[2..3], "acme"),
            None,
            "a rebuild that finished cleanly shows nothing"
        );
    }

    #[test]
    fn reindex_activity_keeps_showing_the_tenants_last_rebuild_when_it_left_resources_unindexed() {
        use helios_persistence::search::ReindexStatus;

        // #1125: generation 0 ends `Completed` with 11,704 resource errors.
        let mut with_errors = finished(reindex_job("acme", ReindexStatus::Completed, 9, 9), 5);
        with_errors.errors = vec![resource_error("p1"), resource_error("p2")];
        assert_eq!(
            reindex_activity_of(std::slice::from_ref(&with_errors), "acme"),
            Some(ReindexActivity::Failed {
                job_id: with_errors.job_id.clone(),
                errors: 2,
            }),
            "a completion with resource errors stays visible after the job stops"
        );
        assert_eq!(
            reindex_activity_of(std::slice::from_ref(&with_errors), "other"),
            None,
            "only for its own tenant"
        );

        let failed = finished(reindex_job("acme", ReindexStatus::Failed, 0, 9), 3);
        assert_eq!(
            reindex_activity_of(&[with_errors.clone(), failed.clone()], "acme"),
            Some(ReindexActivity::Failed {
                job_id: failed.job_id.clone(),
                errors: 0,
            }),
            "a job that failed as a whole counts, and the most recent one wins"
        );

        // A retry running hides the old failure behind its progress…
        let retry = reindex_job("acme", ReindexStatus::InProgress, 3, 9);
        assert!(
            reindex_activity_of(&[failed.clone(), retry], "acme")
                .is_some_and(|activity| activity.is_running())
        );
        // …and a later clean rebuild clears it.
        let clean = finished(reindex_job("acme", ReindexStatus::Completed, 9, 9), 1);
        assert_eq!(
            reindex_activity_of(&[with_errors.clone(), failed.clone(), clean], "acme"),
            None
        );
        // So does a later cancellation: stopping a rebuild is the operator's call.
        let cancelled = finished(reindex_job("acme", ReindexStatus::Cancelled, 1, 9), 1);
        assert_eq!(reindex_activity_of(&[failed, cancelled], "acme"), None);
    }

    #[test]
    fn a_clean_retry_of_failed_resources_keeps_the_permanent_failures_visible() {
        use helios_persistence::search::ReindexStatus;

        let mut rejected = resource_error("p2");
        rejected.retryable = false;
        let mut generation = finished(reindex_job("acme", ReindexStatus::Completed, 9, 9), 5);
        generation.errors = vec![resource_error("p1"), rejected];
        let mut retry = finished(reindex_job("acme", ReindexStatus::Completed, 1, 1), 1);
        retry.resource_scoped = true;

        assert_eq!(
            reindex_activity_of(&[generation.clone(), retry.clone()], "acme"),
            Some(ReindexActivity::Failed {
                job_id: generation.job_id.clone(),
                errors: 1,
            }),
            "the retry fixed p1; p2 was rejected permanently and is still unindexed"
        );

        generation.errors.truncate(1);
        assert_eq!(
            reindex_activity_of(&[generation, retry], "acme"),
            None,
            "a retry that fixed every failure clears the banner"
        );
    }

    /// A private counter set per test.
    fn isolated_counters() -> Arc<DashboardCounters> {
        Arc::new(DashboardCounters::new())
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

    /// Creates `n` resources of each `(type, n)` in the default tenant.
    async fn populate<S: ResourceStorage>(storage: &S, counts: &[(&str, usize)]) {
        for (resource_type, n) in counts {
            for _ in 0..*n {
                create_in(storage, resource_type).await;
            }
        }
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

    fn schedule() -> ReconcileSchedule {
        ReconcileSchedule::new(StdDuration::from_secs(30))
    }

    /// The tenants waiting for a background seed, sorted.
    fn queued_tenants<S>(provider: &StorageDashboardProvider<S>) -> Vec<String> {
        let mut tenants: Vec<String> = lock(&provider.seeds.tenants).iter().cloned().collect();
        tenants.sort();
        tenants
    }

    /// Moves the default tenant onto the counter path the way production does
    /// (#1078): the first load is pending and queues the seed, the background
    /// drain runs it, and the next load — returned — is served from memory.
    async fn settle<S>(
        provider: &StorageDashboardProvider<S>,
        window: DashboardWindow,
        types: &[String],
        include_empty: bool,
    ) -> DashboardSnapshot
    where
        S: ResourceStorage + Send + Sync + 'static,
    {
        let pending = provider.snapshot(window, "", types, include_empty).await;
        assert_eq!(
            pending.figures,
            Figures::Pending,
            "an unseeded tenant's first load is pending"
        );
        let report = provider.drain_pending_seeds(&mut schedule()).await;
        assert_eq!(report.seeded, vec!["default".to_string()], "the seed ran");
        let snapshot = provider.snapshot(window, "", types, include_empty).await;
        assert!(snapshot.figures.is_known());
        snapshot
    }

    /// A storage double over SQLite that counts every aggregate the dashboard
    /// could run — and can make each one slow or fail, or claim it cannot
    /// count at all — so a test can prove which reads happened where. Plain
    /// CRUD passes straight through.
    struct InstrumentedStorage {
        inner: Arc<SqliteBackend>,
        aggregate_calls: AtomicUsize,
        /// Per-type `count_deltas_by_bucket` calls (also in `aggregate_calls`).
        per_type_history_calls: AtomicUsize,
        /// Grouped `count_deltas_by_type_and_bucket` calls (also in
        /// `aggregate_calls`).
        grouped_history_calls: AtomicUsize,
        delay_ms: AtomicU64,
        fail: AtomicBool,
        /// Fails only the history reads, leaving the totals read working.
        fail_history: AtomicBool,
        type_counts: AtomicBool,
        /// Panics the next `count_all_types`, once.
        panic_once: AtomicBool,
        /// A write marker to report instead of SQLite's own.
        marker: Mutex<Option<Option<WriteMarker>>>,
    }

    impl InstrumentedStorage {
        fn over(inner: Arc<SqliteBackend>) -> Arc<Self> {
            Arc::new(Self {
                inner,
                aggregate_calls: AtomicUsize::new(0),
                per_type_history_calls: AtomicUsize::new(0),
                grouped_history_calls: AtomicUsize::new(0),
                delay_ms: AtomicU64::new(0),
                fail: AtomicBool::new(false),
                fail_history: AtomicBool::new(false),
                type_counts: AtomicBool::new(true),
                panic_once: AtomicBool::new(false),
                marker: Mutex::new(None),
            })
        }

        /// Reports `marker` as the storage write marker from now on.
        fn set_marker(&self, marker: Option<WriteMarker>) {
            *lock(&self.marker) = Some(marker);
        }

        fn aggregate_calls(&self) -> usize {
            self.aggregate_calls.load(Ordering::SeqCst)
        }

        fn per_type_history_calls(&self) -> usize {
            self.per_type_history_calls.load(Ordering::SeqCst)
        }

        fn grouped_history_calls(&self) -> usize {
            self.grouped_history_calls.load(Ordering::SeqCst)
        }

        /// A history read: counted as an aggregate, then failed if
        /// `fail_history` is set.
        async fn history_read(&self) -> StorageResult<()> {
            self.aggregate().await?;
            if self.fail_history.load(Ordering::SeqCst) {
                return Err(StorageError::Backend(BackendError::Unavailable {
                    backend_name: "instrumented".to_string(),
                    message: "injected history read failure".to_string(),
                }));
            }
            Ok(())
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

        fn supports_type_counts(&self) -> bool {
            self.type_counts.load(Ordering::SeqCst) && self.inner.supports_type_counts()
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
            self.per_type_history_calls.fetch_add(1, Ordering::SeqCst);
            self.history_read().await?;
            ResourceStorage::count_deltas_by_bucket(
                self.inner.as_ref(),
                tenant,
                resource_type,
                since,
                bucket_seconds,
            )
            .await
        }

        async fn count_deltas_by_type_and_bucket(
            &self,
            tenant: &TenantContext,
            resource_types: &[&str],
            since: DateTime<Utc>,
            bucket_seconds: i64,
        ) -> StorageResult<Vec<(String, ResourceCountDelta)>> {
            self.grouped_history_calls.fetch_add(1, Ordering::SeqCst);
            self.history_read().await?;
            // SQLite's own grouped query, not the trait's per-type default.
            ResourceStorage::count_deltas_by_type_and_bucket(
                self.inner.as_ref(),
                tenant,
                resource_types,
                since,
                bucket_seconds,
            )
            .await
        }

        async fn count_all_types(
            &self,
            tenant: &TenantContext,
        ) -> StorageResult<Vec<(String, u64)>> {
            if self.panic_once.swap(false, Ordering::SeqCst) {
                panic!("injected count_all_types panic");
            }
            self.aggregate().await?;
            ResourceStorage::count_all_types(self.inner.as_ref(), tenant).await
        }

        async fn latest_write_marker(
            &self,
            tenant: &TenantContext,
            recent_since: Option<DateTime<Utc>>,
        ) -> StorageResult<Option<WriteMarker>> {
            let marker = *lock(&self.marker);
            match marker {
                Some(marker) => Ok(marker),
                None => {
                    ResourceStorage::latest_write_marker(self.inner.as_ref(), tenant, recent_since)
                        .await
                }
            }
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

    /// Every series' type, total and final cumulative point — what a counter
    /// snapshot and a storage read taken moments apart must agree on.
    fn ends(series: &[DashboardSeries]) -> Vec<(String, u64, u64)> {
        series
            .iter()
            .map(|s| {
                (
                    s.resource_type.clone(),
                    s.total,
                    s.points.last().map_or(0, |p| p.cumulative),
                )
            })
            .collect()
    }

    /// Loads `types`' history rings over `window` straight from `storage` into
    /// `counters`, bracketed exactly as the provider's ring seeds are: every
    /// token begun, one grouped history read, split per type, every token
    /// finished.
    async fn seed_rings_from<S: ResourceStorage + Sync>(
        counters: &DashboardCounters,
        storage: &S,
        tenant: &TenantContext,
        types: &[&str],
        window: DashboardWindow,
        now: DateTime<Utc>,
    ) {
        let series_window = SeriesWindow::from_dashboard_window(window);
        let (first_bucket, _) = series_window.bounds(now);
        let tokens: Vec<_> = types
            .iter()
            .map(|rt| counters.begin_ring_seed(tenant.tenant_id().as_str(), rt, window))
            .collect();
        let rows = storage
            .count_deltas_by_type_and_bucket(
                tenant,
                types,
                first_bucket,
                series_window.bucket_seconds,
            )
            .await
            .expect("grouped history read");
        let mut by_type = history_deltas_by_type(rows, series_window, now);
        for (rt, token) in types.iter().zip(tokens) {
            let deltas = by_type.remove(*rt).unwrap_or_default();
            assert!(counters.finish_ring_seed(token, &deltas, now));
        }
    }

    /// The provider builds a well-formed, zeroed snapshot over an empty store
    /// once the tenant is seeded: no series, zero totals, and a non-empty FHIR
    /// version — an empty grouping sums to zero (#959).
    #[tokio::test]
    async fn snapshot_over_empty_backend_is_zeroed_but_well_formed() {
        let provider = StorageDashboardProvider::new(sqlite(), &test_config())
            .with_counters(isolated_counters());
        let snapshot = settle(&provider, DashboardWindow::default(), &[], false).await;

        // An empty store has nothing to chart: no available types, no series —
        // the UI renders its explicit empty state from this (#555).
        assert!(snapshot.series.is_empty());
        assert!(snapshot.available.is_empty());
        assert_eq!(snapshot.total_resources, 0);
        assert_eq!(snapshot.distinct_types, 0);
        assert!(!snapshot.fhir_version.is_empty());
        // Every read answered: these zeros are measurements, not fallbacks,
        // and the page may present them as such (#956).
        assert!(matches!(snapshot.figures, Figures::Exact { .. }));
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
    /// is silently dropped and the selection falls back to nothing plotted for
    /// it.
    #[tokio::test]
    async fn unstored_type_is_dropped_without_the_flag() {
        let backend = sqlite();
        create_in(backend.as_ref(), "Patient").await;
        let provider = StorageDashboardProvider::new(backend, &test_config())
            .with_counters(isolated_counters());
        let snapshot = settle(
            &provider,
            DashboardWindow::default(),
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
        let backend = sqlite();
        create_in(backend.as_ref(), "Patient").await;
        let provider = StorageDashboardProvider::new(backend, &test_config())
            .with_counters(isolated_counters());
        let requested = vec!["Patient".to_string(), "Observation".to_string()];

        let without_flag = settle(&provider, DashboardWindow::default(), &requested, false).await;
        let with_flag = provider
            .snapshot(DashboardWindow::default(), "", &requested, true)
            .await;

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
        assert_eq!(
            observation.points.len(),
            DashboardWindow::default().points()
        );
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
        let backend = sqlite();
        let tenant = test_tenant();
        let doomed = create_in(backend.as_ref(), "Patient").await;
        populate(backend.as_ref(), &[("Observation", 2)]).await;
        // Deleted rows must not be counted by either figure.
        backend
            .delete(&tenant, "Patient", doomed.id())
            .await
            .expect("delete");

        let provider = StorageDashboardProvider::new(Arc::clone(&backend), &test_config())
            .with_counters(isolated_counters());
        let snapshot = settle(&provider, DashboardWindow::default(), &[], false).await;

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

    /// #1078: sibling snapshot keys of an unseeded tenant — other windows,
    /// selections, the "View all" toggle, the tenant spelled out — all answer
    /// pending without a storage aggregate and queue one seed between them.
    /// That seed is the only storage work, and serves every key afterwards.
    #[tokio::test]
    async fn unseeded_tenant_queues_one_seed_across_sibling_keys() {
        let storage = InstrumentedStorage::over(sqlite());
        create_in(storage.as_ref(), "Patient").await;
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(isolated_counters());

        let keys: [(DashboardWindow, Vec<String>, bool, &str); 5] = [
            (DashboardWindow::LastHour, vec![], false, ""),
            (DashboardWindow::LastDay, vec![], false, ""),
            (
                DashboardWindow::LastMonth,
                vec!["Patient".to_string()],
                false,
                "",
            ),
            (
                DashboardWindow::LastHour,
                vec!["Encounter".to_string()],
                true,
                "",
            ),
            (DashboardWindow::LastHour, vec![], false, "default"),
        ];
        for (window, types, include_empty, tenant) in &keys {
            let snapshot = provider
                .snapshot(*window, tenant, types, *include_empty)
                .await;
            assert_eq!(snapshot.figures, Figures::Pending);
        }
        assert_eq!(storage.aggregate_calls(), 0);
        assert_eq!(queued_tenants(&provider), vec!["default".to_string()]);

        let report = provider.drain_pending_seeds(&mut schedule()).await;
        assert_eq!(report.seeded, vec!["default".to_string()]);
        let calls = storage.aggregate_calls();
        assert_eq!(
            calls,
            1 + DashboardWindow::ALL.len(),
            "one totals read, plus the default type's ring for every window"
        );

        for (window, types, include_empty, tenant) in &keys {
            let snapshot = provider
                .snapshot(*window, tenant, types, *include_empty)
                .await;
            assert!(snapshot.figures.is_known());
            assert_eq!(snapshot.total_resources, 1);
        }
        assert_eq!(storage.aggregate_calls(), calls);

        // Seeding is per tenant.
        assert!(
            provider
                .snapshot(DashboardWindow::LastHour, "other", &[], false)
                .await
                .figures
                == Figures::Pending
        );
        assert_eq!(queued_tenants(&provider), vec!["other".to_string()]);
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
        assert!(
            lock(&provider.seeds.job_counts).is_empty(),
            "nothing to refresh without a store"
        );
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
        assert_eq!(snapshot.export_jobs, None, "never read yet");
        assert_eq!(snapshot.import_jobs_active, None);

        let report = provider.drain_pending_seeds(&mut schedule()).await;
        assert_eq!(report.job_counts_refreshed, vec!["default".to_string()]);
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
            .claim_next(&worker, std::time::Duration::from_secs(60), 3)
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
        provider
            .snapshot(DashboardWindow::default(), "", &[], false)
            .await;
        provider.drain_pending_seeds(&mut schedule()).await;
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

    /// Job counts are served from the last background read (#1078): a load
    /// inside [`JOB_COUNTS_TTL`] neither reads the stores nor queues a
    /// refresh; a load after it serves the previous counts and queues one,
    /// which the loop's next drain runs.
    #[tokio::test]
    async fn job_counts_are_served_from_the_last_background_read() {
        use helios_persistence::core::{BulkSubmitProvider, SubmissionId};

        let backend = sqlite();
        let submit_jobs = Arc::clone(&backend) as Arc<dyn BulkSubmitJobStore>;
        let provider = StorageDashboardProvider::new(Arc::clone(&backend), &test_config())
            .with_counters(isolated_counters())
            .with_job_stores(None, Some(submit_jobs));

        let first = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert_eq!(first.import_jobs_active, None);
        assert_eq!(
            lock(&provider.seeds.job_counts).len(),
            1,
            "a refresh is queued"
        );
        provider.drain_pending_seeds(&mut schedule()).await;
        assert!(lock(&provider.seeds.job_counts).is_empty());

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
            "inside the TTL the last read count is served"
        );
        assert!(
            lock(&provider.seeds.job_counts).is_empty(),
            "and nothing is queued"
        );

        // Age the entry past the TTL: the stale count is still served at once,
        // and a refresh is queued for the loop.
        if let Some(entry) = lock(&provider.job_counts).get_mut("default") {
            entry.fetched_at = Instant::now().checked_sub(JOB_COUNTS_TTL);
        }
        let stale = provider
            .snapshot(DashboardWindow::LastDay, "", &[], false)
            .await;
        assert_eq!(stale.import_jobs_active, Some(0));
        assert_eq!(lock(&provider.seeds.job_counts).len(), 1);

        let report = provider.drain_pending_seeds(&mut schedule()).await;
        assert_eq!(report.job_counts_refreshed, vec!["default".to_string()]);
        let fresh = provider
            .snapshot(DashboardWindow::LastDay, "", &[], false)
            .await;
        assert_eq!(fresh.import_jobs_active, Some(1));
    }

    /// #1078 decision (a): the job counts are never read on the request path.
    /// Every snapshot — pending, from the counters, or unsupported — completes
    /// on its first poll, so it cannot be waiting on a job store however slow
    /// one is (the SQLite stores answer through a blocking task, which is never
    /// ready on a first poll), and a submission the stores hold only shows
    /// once the background refresh has read it.
    #[tokio::test]
    async fn job_counts_are_never_awaited_on_the_request_path() {
        use helios_persistence::core::{BulkSubmitProvider, SubmissionId};

        let backend = sqlite();
        create_in(backend.as_ref(), "Patient").await;
        backend
            .create_submission(&test_tenant(), &SubmissionId::generate("test-system"), None)
            .await
            .expect("create submission");
        let storage = InstrumentedStorage::over(Arc::clone(&backend));
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(isolated_counters())
            .with_job_stores(
                Some(Arc::clone(&backend) as Arc<dyn BulkExportJobStore>),
                Some(Arc::clone(&backend) as Arc<dyn BulkSubmitJobStore>),
            );

        let pending = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .now_or_never()
            .expect("a pending snapshot completes on its first poll");
        assert_eq!(pending.figures, Figures::Pending);
        assert_eq!(pending.import_jobs_active, None, "not read on the request");

        let report = provider.drain_pending_seeds(&mut schedule()).await;
        assert_eq!(report.seeded, vec!["default".to_string()]);
        assert_eq!(report.job_counts_refreshed, vec!["default".to_string()]);

        let seeded = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .now_or_never()
            .expect("a counter snapshot completes on its first poll");
        assert!(seeded.figures.is_known());
        assert_eq!(seeded.import_jobs_active, Some(1));
        assert_eq!(
            seeded.export_jobs,
            Some(ExportJobCounts {
                running: 0,
                queued: 0
            })
        );

        storage.type_counts.store(false, Ordering::SeqCst);
        let unsupported = provider
            .snapshot(DashboardWindow::LastHour, "other", &[], false)
            .now_or_never()
            .expect("an unsupported snapshot completes on its first poll");
        assert_eq!(unsupported.figures, Figures::Unsupported);
        assert_eq!(unsupported.import_jobs_active, None);
    }

    /// The #1078 request-path guarantee for an unseeded tenant: every load
    /// answers at once with [`Figures::Pending`] and empty figures, running no
    /// storage aggregate — even over a backend whose aggregates take seconds,
    /// and while the seed itself is in flight. The background seed loads the
    /// totals and the default charted types' rings for every window, after
    /// which the counter path serves storage's figures with no aggregate.
    #[tokio::test]
    async fn unseeded_tenant_is_pending_at_once_and_seeded_in_the_background() {
        let storage = InstrumentedStorage::over(sqlite());
        populate(storage.as_ref(), &[("Patient", 2), ("Observation", 1)]).await;
        let counters = isolated_counters();
        let provider = Arc::new(
            StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
                .with_counters(Arc::clone(&counters)),
        );

        storage.delay_ms.store(1_500, Ordering::SeqCst);
        for window in DashboardWindow::ALL {
            let pending = tokio::time::timeout(
                StdDuration::from_millis(500),
                provider.snapshot(window, "", &[], false),
            )
            .await
            .expect("an unseeded tenant's load must not wait on storage");
            assert_eq!(pending.figures, Figures::Pending, "{}", window.as_str());
            assert_eq!(pending.total_resources, 0);
            assert_eq!(pending.distinct_types, 0);
            assert!(pending.series.is_empty() && pending.available.is_empty());
            assert!(pending.figures.read_at().is_none(), "nothing was measured");
            assert!(!pending.fhir_version.is_empty());
        }
        assert_eq!(
            storage.aggregate_calls(),
            0,
            "no aggregate ran on the request path"
        );
        assert_eq!(queued_tenants(&provider), vec!["default".to_string()]);
        assert!(!counters.is_seeded("default"));

        // The seed runs in the background; a load while it is in flight still
        // returns at once.
        let seeding = tokio::spawn({
            let provider = Arc::clone(&provider);
            async move { provider.drain_pending_seeds(&mut schedule()).await }
        });
        assert!(
            eventually(|| storage.aggregate_calls() > 0).await,
            "the seed started"
        );
        let during = tokio::time::timeout(
            StdDuration::from_millis(500),
            provider.snapshot(DashboardWindow::LastHour, "", &[], false),
        )
        .await
        .expect("a load never waits on a seed in flight");
        assert_eq!(during.figures, Figures::Pending);
        storage.delay_ms.store(0, Ordering::SeqCst);
        let report = seeding.await.expect("the seed task did not panic");
        assert_eq!(report.seeded, vec!["default".to_string()]);
        assert!(queued_tenants(&provider).is_empty());

        let totals = counters.totals_view("default").expect("seeded totals");
        assert!(totals.exact);
        assert_eq!(
            totals.totals,
            vec![("Patient".to_string(), 2), ("Observation".to_string(), 1)]
        );
        for window in DashboardWindow::ALL {
            let views = counters
                .series_view("default", window, &["Patient", "Observation"], Utc::now())
                .expect("seeded series");
            assert!(
                views.iter().all(|v| v.history_seeded && v.exact),
                "{}",
                window.as_str()
            );
        }

        let calls = storage.aggregate_calls();
        let tenant = test_tenant();
        for window in DashboardWindow::ALL {
            let snapshot = provider.snapshot(window, "", &[], false).await;
            assert!(matches!(snapshot.figures, Figures::Exact { .. }));
            assert_eq!(snapshot.total_resources, 3);
            assert_eq!(snapshot.distinct_types, 2);
            let from_storage = resource_count_series(
                storage.inner.as_ref(),
                &tenant,
                &["Patient", "Observation"],
                SeriesWindow::from_dashboard_window(window),
                Utc::now(),
            )
            .await
            .expect("storage series");
            assert_eq!(ends(&snapshot.series), ends(&from_storage));
        }
        assert_eq!(
            storage.aggregate_calls(),
            calls,
            "a seeded tenant's loads run no aggregate"
        );
    }

    /// #1078 item 2: a failed startup seed is not dropped. The tenant stays
    /// queued, pages meanwhile get a pending snapshot (not zeros, not
    /// partial) without touching storage, and the next pass retries the seed.
    #[tokio::test]
    async fn startup_seed_failure_is_retried_on_the_next_pass() {
        let storage = InstrumentedStorage::over(sqlite());
        create_in(storage.as_ref(), "Patient").await;
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(Arc::clone(&counters));
        let mut schedule = schedule();

        storage.fail.store(true, Ordering::SeqCst);
        let report = provider.seed_default_tenant(&mut schedule).await;
        assert_eq!(report.seed_failed, vec!["default".to_string()]);
        assert!(report.seeded.is_empty());
        assert!(!counters.is_seeded("default"));
        assert!(counters.tenants().is_empty(), "a failed read seeds nothing");
        assert_eq!(
            queued_tenants(&provider),
            vec!["default".to_string()],
            "the failed seed stays queued"
        );

        let calls = storage.aggregate_calls();
        let meanwhile = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert_eq!(meanwhile.figures, Figures::Pending);
        assert_eq!(storage.aggregate_calls(), calls);

        storage.fail.store(false, Ordering::SeqCst);
        let report = provider.reconcile_pass(&mut schedule).await;
        assert_eq!(report.seeded, vec!["default".to_string()]);
        assert!(report.seed_failed.is_empty());
        assert!(counters.is_seeded("default"));
        assert!(queued_tenants(&provider).is_empty());
        let seeded = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(matches!(seeded.figures, Figures::Exact { .. }));
        assert_eq!(seeded.total_resources, 1);
    }

    /// A failed seed must never seed the counters (#956): a tenant a page
    /// queued stays pending and queued until a seed succeeds.
    #[tokio::test]
    async fn failed_seed_of_a_viewed_tenant_stays_queued_until_it_succeeds() {
        let storage = InstrumentedStorage::over(sqlite());
        create_in(storage.as_ref(), "Patient").await;
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(Arc::clone(&counters));

        storage.fail.store(true, Ordering::SeqCst);
        let pending = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert_eq!(pending.figures, Figures::Pending);
        let report = provider.drain_pending_seeds(&mut schedule()).await;
        assert_eq!(report.seed_failed, vec!["default".to_string()]);
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
        assert_eq!(queued_tenants(&provider), vec!["default".to_string()]);
        assert!(
            provider
                .snapshot(DashboardWindow::LastHour, "", &[], false)
                .await
                .figures
                == Figures::Pending
        );

        storage.fail.store(false, Ordering::SeqCst);
        let report = provider.drain_pending_seeds(&mut schedule()).await;
        assert_eq!(report.seeded, vec!["default".to_string()]);
        let recovered = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(recovered.figures.is_known());
        assert_eq!(recovered.total_resources, 1);
        assert!(counters.is_seeded("default"));
    }

    /// #1078 item 16: over a backend whose count aggregates are only the
    /// trait's empty defaults, every snapshot says the counts are unsupported
    /// — empty figures, no timestamp, job counts as usual (read in the
    /// background) — without a storage aggregate, and nothing ever seeds or
    /// reconciles the tenant: not a page load, not the startup seed, not a
    /// pass, not the loop. The loop keeps running only to refresh job counts,
    /// and stops at once when no job store is wired either.
    #[tokio::test]
    async fn storage_that_cannot_count_is_unsupported_and_never_seeded() {
        use helios_persistence::core::{BulkSubmitProvider, SubmissionId};

        let storage = InstrumentedStorage::over(sqlite());
        storage.type_counts.store(false, Ordering::SeqCst);
        create_in(storage.as_ref(), "Patient").await;
        storage
            .inner
            .create_submission(&test_tenant(), &SubmissionId::generate("test-system"), None)
            .await
            .expect("create submission");
        let counters = isolated_counters();
        let provider = Arc::new(
            StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
                .with_counters(Arc::clone(&counters))
                .with_job_stores(
                    None,
                    Some(Arc::clone(&storage.inner) as Arc<dyn BulkSubmitJobStore>),
                ),
        );

        for window in DashboardWindow::ALL {
            for (types, include_empty) in [
                (Vec::<String>::new(), false),
                (vec!["Patient".to_string()], true),
            ] {
                let snapshot = provider.snapshot(window, "", &types, include_empty).await;
                assert_eq!(
                    snapshot.figures,
                    Figures::Unsupported,
                    "{}",
                    window.as_str()
                );
                assert_eq!(snapshot.total_resources, 0);
                assert_eq!(snapshot.distinct_types, 0);
                assert!(snapshot.series.is_empty() && snapshot.available.is_empty());
                assert!(snapshot.figures.read_at().is_none());
            }
        }
        assert!(queued_tenants(&provider).is_empty(), "no seed is queued");

        // A recorded write gives the tenant counter state; it must still never
        // be seeded.
        counters.record("default", "Patient", 1, Utc::now());
        let mut schedule = schedule();
        let report = provider.seed_default_tenant(&mut schedule).await;
        assert!(report.seeded.is_empty() && report.seed_failed.is_empty());
        let report = provider.reconcile_pass(&mut schedule).await;
        assert!(report.seeded.is_empty() && report.reconciled.is_empty());
        assert_eq!(
            report.job_counts_refreshed,
            vec!["default".to_string()],
            "job counts are still refreshed"
        );
        provider.drain_pending_seeds(&mut schedule).await;
        assert_eq!(storage.aggregate_calls(), 0);
        assert!(!counters.is_seeded("default"));
        assert_eq!(
            provider
                .snapshot(DashboardWindow::LastHour, "", &[], false)
                .await
                .import_jobs_active,
            Some(1),
            "job counts are reported as usual"
        );

        // The loop refreshes a tenant's job counts when a load asks.
        let handle = spawn_reconcile_loop_every(&provider, StdDuration::from_secs(3600))
            .expect("inside a runtime");
        let other = provider
            .snapshot(DashboardWindow::LastHour, "other", &[], false)
            .await;
        assert_eq!(other.import_jobs_active, None);
        assert!(
            eventually(|| lock(&provider.job_counts)
                .get("other")
                .is_some_and(|entry| entry.import == Some(0)))
            .await,
            "the loop refreshed the job counts a load queued"
        );
        assert!(!handle.is_finished());
        assert_eq!(storage.aggregate_calls(), 0);
        assert!(!counters.is_seeded("default"));

        // Without any job store there is nothing to do: the loop stops.
        let bare = Arc::new(
            StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
                .with_counters(isolated_counters()),
        );
        let handle = spawn_reconcile_loop_every(&bare, StdDuration::from_secs(3600))
            .expect("inside a runtime");
        tokio::time::timeout(StdDuration::from_secs(5), handle)
            .await
            .expect("the loop has nothing to do and stops")
            .expect("the loop did not panic");
        assert_eq!(storage.aggregate_calls(), 0);
    }

    /// #1078: the first seed of a never-seeded tenant runs even while a bulk
    /// submit is active for it, so a tenant first viewed mid-import is not
    /// stuck pending until the import ends. Once seeded, a reseed after a
    /// purge backs off during the import like the periodic reconcile, keeping
    /// the last figures on show.
    #[tokio::test]
    async fn first_seed_runs_during_an_active_import_but_a_reseed_backs_off() {
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
            .with_counters(Arc::clone(&counters))
            .with_job_stores(
                None,
                Some(Arc::clone(&storage.inner) as Arc<dyn BulkSubmitJobStore>),
            );
        let mut schedule = schedule();

        let pending = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert_eq!(pending.figures, Figures::Pending);
        assert_eq!(pending.import_jobs_active, None, "not read yet");
        let report = provider.drain_pending_seeds(&mut schedule).await;
        assert_eq!(
            report.seeded,
            vec!["default".to_string()],
            "a first seed does not wait for the import"
        );
        assert!(report.seed_deferred.is_empty());
        let seeded = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(seeded.figures.is_known());
        assert_eq!(seeded.total_resources, 1);
        assert_eq!(seeded.import_jobs_active, Some(1));

        counters.invalidate_tenant("default");
        let calls = storage.aggregate_calls();
        let report = provider.reconcile_pass(&mut schedule).await;
        assert_eq!(report.seed_deferred, vec!["default".to_string()]);
        assert!(report.seeded.is_empty() && report.reconciled.is_empty());
        assert_eq!(storage.aggregate_calls(), calls, "the reseed waits");
        assert!(counters.needs_reseed("default"));
        let kept = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(kept.figures.is_approximate());
        assert_eq!(kept.total_resources, 1);
    }

    /// #1078 item 3b: after a purge the dashboard keeps showing the last
    /// figures, labelled approximate, instead of going back to waiting — with
    /// no storage aggregate on the request path — and the background reseed
    /// then replaces them with storage's figures. A purge nobody is looking
    /// at is reseeded by the next pass all the same.
    #[tokio::test]
    async fn purge_keeps_approximate_figures_and_reseeds_to_storage() {
        let storage = InstrumentedStorage::over(sqlite());
        populate(storage.as_ref(), &[("Patient", 2), ("Observation", 1)]).await;
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(Arc::clone(&counters));
        let before = settle(&provider, DashboardWindow::LastHour, &[], false).await;
        assert_eq!(before.total_resources, 3);
        assert!(matches!(before.figures, Figures::Exact { .. }));

        // What the purge handlers do: erase storage, then invalidate.
        storage
            .inner
            .purge_tenant_data("default")
            .await
            .expect("purge");
        counters.invalidate_tenant("default");

        let calls = storage.aggregate_calls();
        for window in DashboardWindow::ALL {
            let kept = provider.snapshot(window, "", &[], false).await;
            assert!(
                kept.figures.is_approximate(),
                "the last figures stay on show, labelled approximate until the reseed"
            );
            assert_eq!(kept.total_resources, 3);
            assert_eq!(kept.distinct_types, 2);
            assert_eq!(kept.series.len(), 2);
        }
        assert_eq!(
            storage.aggregate_calls(),
            calls,
            "no aggregate on the request path"
        );
        assert_eq!(
            queued_tenants(&provider),
            vec!["default".to_string()],
            "the reseed is queued"
        );

        let report = provider.drain_pending_seeds(&mut schedule()).await;
        assert_eq!(report.seeded, vec!["default".to_string()]);
        assert!(!counters.needs_reseed("default"));
        assert!(queued_tenants(&provider).is_empty());
        let after = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(matches!(after.figures, Figures::Exact { .. }));
        assert_eq!(after.total_resources, 0);
        assert_eq!(after.distinct_types, 0);
        assert!(after.available.is_empty() && after.series.is_empty());

        // Unattended: no page load queues it, the pass's stale scan does.
        create_in(storage.inner.as_ref(), "Encounter").await;
        counters.invalidate_tenant("default");
        assert!(queued_tenants(&provider).is_empty());
        let report = provider.reconcile_pass(&mut schedule()).await;
        assert_eq!(report.seeded, vec!["default".to_string()]);
        let reseeded = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(matches!(reseeded.figures, Figures::Exact { .. }));
        assert_eq!(reseeded.total_resources, 1);
        assert_eq!(reseeded.series[0].resource_type, "Encounter");
    }

    /// The #1078 acceptance core: once seeded, no page load runs a storage
    /// aggregate — whatever the window, selection or "View all" toggle — and
    /// the figures are the counters' (a recorded REST write shows, a write
    /// the counters never saw does not), labelled approximate.
    #[tokio::test]
    async fn seeded_tenant_is_served_from_counters_without_storage_aggregates() {
        let storage = InstrumentedStorage::over(sqlite());
        populate(storage.as_ref(), &[("Patient", 2)]).await;
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(Arc::clone(&counters));

        settle(&provider, DashboardWindow::LastHour, &[], false).await;
        let after_seed = storage.aggregate_calls();
        assert!(after_seed > 0, "the background seed read storage");

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
                let Figures::Approximate {
                    read_at,
                    reconciled_at,
                } = snapshot.figures
                else {
                    panic!("a recorded write is not reconciled: {:?}", snapshot.figures);
                };
                assert_eq!(
                    reconciled_at,
                    counters
                        .totals_view("default")
                        .expect("seeded")
                        .reconciled_at,
                    "approximate figures say when the counters last matched storage"
                );
                assert!(read_at >= reconciled_at);
                assert_eq!(snapshot.total_resources, 3);
                assert_eq!(snapshot.distinct_types, 1);
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
    /// snapshot still returns at once, for every window — the seeded default
    /// types exact, and a stored type whose rings were never loaded labelled
    /// approximate and queued for a background seed rather than read inline.
    #[tokio::test]
    async fn seeded_tenant_renders_promptly_over_a_slow_backend() {
        let storage = InstrumentedStorage::over(sqlite());
        // Encounter is stored but not among the three default charted types.
        populate(
            storage.as_ref(),
            &[
                ("Patient", 2),
                ("Observation", 2),
                ("Condition", 2),
                ("Encounter", 1),
            ],
        )
        .await;
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(isolated_counters());
        settle(&provider, DashboardWindow::LastHour, &[], false).await;

        storage.delay_ms.store(5_000, Ordering::SeqCst);
        let calls = storage.aggregate_calls();
        let encounter = vec!["Encounter".to_string()];
        for window in DashboardWindow::ALL {
            let defaults = tokio::time::timeout(
                StdDuration::from_secs(1),
                provider.snapshot(window, "", &[], false),
            )
            .await
            .expect("a seeded tenant must not wait on the slow backend");
            assert!(
                matches!(defaults.figures, Figures::Exact { .. }),
                "{}",
                window.as_str()
            );
            assert_eq!(defaults.total_resources, 7);
            assert_eq!(defaults.series.len(), 3);

            let snapshot = tokio::time::timeout(
                StdDuration::from_secs(1),
                provider.snapshot(window, "", &encounter, false),
            )
            .await
            .expect("a seeded tenant must not wait on the slow backend");
            assert!(
                snapshot.figures.is_approximate(),
                "no storage history for Encounter's ring yet ({})",
                window.as_str()
            );
            assert_eq!(snapshot.series.len(), 1);
            assert_eq!(snapshot.series[0].points.len(), window.points());
            assert_eq!(snapshot.series[0].points.last().unwrap().cumulative, 1);
        }
        assert_eq!(storage.aggregate_calls(), calls);

        let mut pending: Vec<RingKey> = lock(&provider.seeds.rings).iter().cloned().collect();
        pending.sort_by_key(|(_, _, window)| window.bucket_seconds());
        assert_eq!(
            pending,
            DashboardWindow::ALL
                .iter()
                .map(|window| ("default".to_string(), "Encounter".to_string(), *window))
                .collect::<Vec<_>>()
        );
    }

    /// A ring charted without storage history is queued once however many
    /// loads ask, and the background drain loads it — after which the
    /// snapshot is exact.
    #[tokio::test]
    async fn queued_ring_seeds_are_single_flight_and_drained_in_the_background() {
        let backend = sqlite();
        populate(
            backend.as_ref(),
            &[
                ("Patient", 2),
                ("Observation", 2),
                ("Condition", 2),
                ("Encounter", 1),
            ],
        )
        .await;
        let provider = StorageDashboardProvider::new(Arc::clone(&backend), &test_config())
            .with_counters(isolated_counters());
        settle(&provider, DashboardWindow::LastHour, &[], false).await;

        let encounter = vec!["Encounter".to_string()];
        for _ in 0..3 {
            let snapshot = provider
                .snapshot(DashboardWindow::LastDay, "", &encounter, false)
                .await;
            assert!(snapshot.figures.is_approximate());
        }
        assert_eq!(lock(&provider.seeds.rings).len(), 1);

        assert_eq!(provider.drain_pending_ring_seeds().await, 1);
        assert!(lock(&provider.seeds.rings).is_empty());
        let snapshot = provider
            .snapshot(DashboardWindow::LastDay, "", &encounter, false)
            .await;
        assert!(matches!(snapshot.figures, Figures::Exact { .. }));
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
            .with_counters(Arc::clone(&counters));
        let seeded = settle(&provider, DashboardWindow::LastHour, &[], false).await;
        assert!(matches!(seeded.figures, Figures::Exact { .. }));

        let recorded = create_in(backend.as_ref(), "Patient").await;
        counters.record("default", "Patient", 1, recorded.last_modified());
        // Another instance, or a direct database edit: storage only.
        create_in(backend.as_ref(), "Observation").await;

        let during = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(during.figures.is_approximate());
        assert_eq!(during.total_resources, 2);

        let mut schedule = ReconcileSchedule::new(StdDuration::from_secs(30));
        let report = provider.reconcile_pass(&mut schedule).await;
        assert_eq!(report.reconciled, vec!["default".to_string()]);
        assert!(report.skipped_active_import.is_empty());
        assert!(report.seeded.is_empty());
        assert_eq!(
            report.rings_seeded,
            DashboardWindow::ALL.len(),
            "the written Patient's ring is re-seeded in every window the seed loaded"
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
        assert!(after.figures.is_approximate());
        assert_eq!(provider.drain_pending_ring_seeds().await, 1);

        let converged = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(
            matches!(converged.figures, Figures::Exact { .. }),
            "reconciled and loaded: exact again"
        );
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
            &counters,
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

        // Straight away, the next pass is not due (duty cycle): it probes the
        // unchanged write marker and runs no aggregate.
        let report = provider.reconcile_pass(&mut schedule).await;
        assert!(report.reconciled.is_empty());
        assert!(report.external_changes.is_empty());
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
            .with_counters(Arc::clone(&counters))
            .with_job_stores(
                None,
                Some(Arc::clone(&storage.inner) as Arc<dyn BulkSubmitJobStore>),
            );
        // The first seed runs despite the import (covered on its own above).
        let seeded = settle(&provider, DashboardWindow::LastHour, &[], false).await;
        assert_eq!(seeded.import_jobs_active, Some(1));
        assert!(counters.is_seeded("default"));

        // The import writes; a load charts a ring with no history yet.
        counters.record("default", "Patient", 5, Utc::now());
        provider
            .snapshot(
                DashboardWindow::LastDay,
                "",
                &["Encounter".to_string()],
                true,
            )
            .await;

        let calls = storage.aggregate_calls();
        let mut schedule = ReconcileSchedule::new(StdDuration::from_secs(30));
        let report = provider.reconcile_pass(&mut schedule).await;
        assert_eq!(report.skipped_active_import, vec!["default".to_string()]);
        assert!(report.reconciled.is_empty());
        assert_eq!(report.rings_seeded, 0);
        assert_eq!(storage.aggregate_calls(), calls, "no storage aggregate ran");
        assert_eq!(
            lock(&provider.seeds.rings).len(),
            1,
            "the ring seed stays queued"
        );

        let totals = counters.totals_view("default").expect("seeded");
        assert!(!totals.exact);
        assert_eq!(totals.totals, vec![("Patient".to_string(), 6)]);
    }

    /// The counter path and a storage read chart identical points for the
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
            let token = counters.begin_reconcile("default", None);
            let totals = ResourceStorage::count_all_types(backend.as_ref(), &tenant)
                .await
                .expect("count_all_types");
            assert!(matches!(
                counters.finish_reconcile(token, &totals, None, now),
                ReconcileOutcome::Applied { .. }
            ));
            seed_rings_from(&counters, backend.as_ref(), &tenant, &types, window, now).await;
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
                resource_count_series_from_counters(&counters, "default", window, &types, now)
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
                resource_count_series_from_counters(&counters, "default", window, &types, now)
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
                .with_counters(Arc::clone(&counters)),
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
        assert!(
            snapshot.figures.is_approximate(),
            "Encounter's ring has no history yet"
        );
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
        let stopped = tokio::time::timeout(StdDuration::from_secs(5), handle)
            .await
            .expect("the loop stops once its provider is gone");
        // Dropping the provider aborts its supervisor.
        assert!(
            stopped.is_ok() || stopped.as_ref().is_err_and(|error| error.is_cancelled()),
            "the loop did not panic: {stopped:?}"
        );
    }

    /// Every `(type, window)` ring of `types` in every window is loaded from
    /// storage history and exact.
    fn all_rings_exact(counters: &DashboardCounters, types: &[&str]) -> bool {
        DashboardWindow::ALL.iter().all(|window| {
            counters
                .series_view("default", *window, types, Utc::now())
                .is_some_and(|views| views.iter().all(|v| v.history_seeded && v.exact))
        })
    }

    /// #1078: a tenant seed loads its rings with one grouped history query per
    /// window — not one per type per window — and never the per-type read. A
    /// charted type with no stored rows is seeded all the same (an empty
    /// history), and the seeded rings chart what per-type storage reads chart.
    #[tokio::test]
    async fn tenant_seed_issues_one_grouped_history_query_per_window() {
        let storage = InstrumentedStorage::over(sqlite());
        populate(
            storage.as_ref(),
            &[("Patient", 3), ("Observation", 2), ("Condition", 1)],
        )
        .await;
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(Arc::clone(&counters));
        // Charted recently in one window: joins that window's query, and a type
        // with no rows at all.
        provider.note_charted("default", &["Procedure"], DashboardWindow::LastDay);

        let report = settle_report(&provider).await;
        assert_eq!(report.seeded, vec!["default".to_string()]);
        let default_types = ["Patient", "Observation", "Condition"];
        assert_eq!(
            report.rings_seeded,
            default_types.len() * DashboardWindow::ALL.len() + 1
        );
        assert_eq!(
            storage.grouped_history_calls(),
            DashboardWindow::ALL.len(),
            "one grouped history query per window"
        );
        assert_eq!(storage.per_type_history_calls(), 0, "no per-type read");
        assert_eq!(
            storage.aggregate_calls(),
            1 + DashboardWindow::ALL.len(),
            "the totals read plus one history read per window"
        );
        assert!(all_rings_exact(&counters, &default_types));
        let procedure = counters
            .series_view(
                "default",
                DashboardWindow::LastDay,
                &["Procedure"],
                Utc::now(),
            )
            .expect("seeded");
        assert!(procedure[0].history_seeded && procedure[0].exact);

        let tenant = test_tenant();
        for window in DashboardWindow::ALL {
            let now = Utc::now();
            let from_storage = resource_count_series(
                storage.inner.as_ref(),
                &tenant,
                &default_types,
                SeriesWindow::from_dashboard_window(window),
                now,
            )
            .await
            .expect("storage series");
            let from_counters = resource_count_series_from_counters(
                &counters,
                "default",
                window,
                &default_types,
                now,
            )
            .expect("counter series");
            assert_eq!(
                shape(&from_storage),
                shape(from_counters.iter().map(|c| &c.series)),
                "{}",
                window.as_str()
            );
        }
    }

    /// Queues the default tenant's seed the way a page load does and drains
    /// it, returning the drain's report.
    async fn settle_report<S>(provider: &StorageDashboardProvider<S>) -> ReconcileReport
    where
        S: ResourceStorage + Send + Sync + 'static,
    {
        let pending = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert_eq!(pending.figures, Figures::Pending);
        provider.drain_pending_seeds(&mut schedule()).await
    }

    /// #1078: when a seed's grouped history query fails, the tenant's totals
    /// still land but none of that query's rings are loaded — they stay
    /// approximate — and a later quiet pass re-seeds them, again one grouped
    /// query per window.
    #[tokio::test]
    async fn failed_grouped_history_query_in_a_seed_is_retried_by_a_later_pass() {
        let storage = InstrumentedStorage::over(sqlite());
        populate(storage.as_ref(), &[("Patient", 2), ("Observation", 1)]).await;
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(Arc::clone(&counters));
        let types = ["Patient", "Observation"];

        storage.fail_history.store(true, Ordering::SeqCst);
        let report = settle_report(&provider).await;
        assert_eq!(report.seeded, vec!["default".to_string()]);
        assert_eq!(report.rings_seeded, 0, "a failed query seeds no ring");
        assert_eq!(storage.grouped_history_calls(), DashboardWindow::ALL.len());
        assert_eq!(storage.per_type_history_calls(), 0);
        for window in DashboardWindow::ALL {
            let views = counters
                .series_view("default", window, &types, Utc::now())
                .expect("totals seeded");
            assert!(
                views.iter().all(|v| !v.history_seeded),
                "{}",
                window.as_str()
            );
        }
        let snapshot = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(
            snapshot.figures.is_approximate(),
            "rings without storage history"
        );
        assert_eq!(snapshot.total_resources, 3);

        storage.fail_history.store(false, Ordering::SeqCst);
        let grouped_before = storage.grouped_history_calls();
        let report = provider.reconcile_pass(&mut schedule()).await;
        assert_eq!(
            report.rings_seeded,
            types.len() * DashboardWindow::ALL.len(),
            "the charted rings are re-seeded"
        );
        // The pass seeds every window once, then drains what the snapshot
        // above queued: nothing left to read by then.
        assert_eq!(
            storage.grouped_history_calls() - grouped_before,
            DashboardWindow::ALL.len()
        );
        assert_eq!(storage.per_type_history_calls(), 0);
        assert!(all_rings_exact(&counters, &types));
        assert!(lock(&provider.seeds.rings).is_empty());
        let exact = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(matches!(exact.figures, Figures::Exact { .. }));
    }

    /// #1078: queued ring seeds are drained one grouped query per window per
    /// tenant; a failed query leaves every ring of that window queued (not
    /// dropped) and a later drain seeds them.
    #[tokio::test]
    async fn failed_grouped_ring_seed_stays_queued_until_a_later_drain() {
        let storage = InstrumentedStorage::over(sqlite());
        populate(
            storage.as_ref(),
            &[
                ("Patient", 2),
                ("Observation", 2),
                ("Condition", 2),
                ("Encounter", 1),
                ("Procedure", 1),
            ],
        )
        .await;
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(Arc::clone(&counters));
        settle(&provider, DashboardWindow::LastHour, &[], false).await;

        let extra = vec!["Encounter".to_string(), "Procedure".to_string()];
        for window in [DashboardWindow::LastHour, DashboardWindow::LastDay] {
            let snapshot = provider.snapshot(window, "", &extra, false).await;
            assert!(snapshot.figures.is_approximate());
        }
        assert_eq!(lock(&provider.seeds.rings).len(), 4);

        storage.fail_history.store(true, Ordering::SeqCst);
        let grouped_before = storage.grouped_history_calls();
        assert_eq!(provider.drain_pending_ring_seeds().await, 0);
        assert_eq!(
            storage.grouped_history_calls() - grouped_before,
            2,
            "one grouped query per queued window, not per ring"
        );
        assert_eq!(
            lock(&provider.seeds.rings).len(),
            4,
            "the failed rings stay queued"
        );

        storage.fail_history.store(false, Ordering::SeqCst);
        let grouped_before = storage.grouped_history_calls();
        assert_eq!(provider.drain_pending_ring_seeds().await, 4);
        assert_eq!(storage.grouped_history_calls() - grouped_before, 2);
        assert_eq!(storage.per_type_history_calls(), 0);
        assert!(lock(&provider.seeds.rings).is_empty());
        for window in [DashboardWindow::LastHour, DashboardWindow::LastDay] {
            let snapshot = provider.snapshot(window, "", &extra, false).await;
            assert!(
                matches!(snapshot.figures, Figures::Exact { .. }),
                "{}",
                window.as_str()
            );
            assert_eq!(snapshot.series.len(), 2);
        }
    }

    /// The window grouping behind the grouped seeds: every window once, in
    /// [`DashboardWindow::ALL`] order, with its distinct types sorted.
    #[test]
    fn rings_by_window_groups_distinct_types_per_window() {
        let grouped = rings_by_window([
            ("Patient".to_string(), DashboardWindow::LastDay),
            ("Encounter".to_string(), DashboardWindow::LastHour),
            ("Patient".to_string(), DashboardWindow::LastHour),
            ("Encounter".to_string(), DashboardWindow::LastHour),
        ]);
        assert_eq!(
            grouped,
            vec![
                (
                    DashboardWindow::LastHour,
                    vec!["Encounter".to_string(), "Patient".to_string()]
                ),
                (DashboardWindow::LastDay, vec!["Patient".to_string()]),
            ]
        );
        assert!(rings_by_window(Vec::new()).is_empty());
    }

    /// Creates one resource of `resource_type` in `tenant`.
    async fn create_for<S: ResourceStorage>(storage: &S, tenant: &str, resource_type: &str) {
        storage
            .create(
                &tenant_context(tenant),
                resource_type,
                serde_json::json!({ "resourceType": resource_type }),
                FhirVersion::R4,
            )
            .await
            .expect("create");
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn sorted(mut items: Vec<String>) -> Vec<String> {
        items.sort();
        items
    }

    /// The provider holds nothing for `tenant` any more.
    fn assert_forgotten<S>(
        provider: &StorageDashboardProvider<S>,
        schedule: &ReconcileSchedule,
        tenant: &str,
    ) {
        assert!(!queued_tenants(provider).iter().any(|t| t == tenant));
        assert!(
            !lock(&provider.seeds.rings)
                .iter()
                .any(|(t, _, _)| t == tenant)
        );
        assert!(
            !lock(&provider.seeds.charted)
                .keys()
                .any(|(t, _, _)| t == tenant)
        );
        assert!(!lock(&provider.seeds.job_counts).contains(tenant));
        assert!(!lock(&provider.job_counts).contains_key(tenant));
        assert!(!lock(&provider.viewed).contains_key(tenant));
        assert!(!schedule.tenants().any(|t| t == tenant));
    }

    /// #1078 decision (b): a write made outside this process (a sibling
    /// instance) is detected through the storage write marker. The next pass
    /// labels the figures approximate and makes the totals due at once, even
    /// inside the duty cycle; once a reconcile succeeds they are exact again,
    /// with the external write counted.
    #[tokio::test]
    async fn marker_detected_external_change_is_approximate_until_a_reconcile() {
        let storage = InstrumentedStorage::over(sqlite());
        create_in(storage.as_ref(), "Patient").await;
        let marker = |millis: i64| {
            Some(WriteMarker {
                latest: DateTime::from_timestamp_millis(millis),
                recent_writes: None,
            })
        };
        storage.set_marker(marker(1_000));
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(isolated_counters());
        let patient = strings(&["Patient"]);
        let load = || provider.snapshot(DashboardWindow::LastHour, "", &patient, false);
        let mut schedule = schedule();

        assert_eq!(load().await.figures, Figures::Pending);
        provider.drain_pending_seeds(&mut schedule).await;
        assert!(matches!(load().await.figures, Figures::Exact { .. }));

        // Nothing changed: inside the duty cycle, the probe finds nothing.
        let report = provider.reconcile_pass(&mut schedule).await;
        assert_eq!(report.not_due, strings(&["default"]));
        assert!(report.external_changes.is_empty() && report.reconciled.is_empty());

        // A sibling instance writes: storage only, and the marker moves. The
        // forced reconcile fails, so the figures stay as they were — labelled.
        create_in(storage.inner.as_ref(), "Patient").await;
        storage.set_marker(marker(2_000));
        storage.fail.store(true, Ordering::SeqCst);
        let calls = storage.aggregate_calls();
        let report = provider.reconcile_pass(&mut schedule).await;
        assert_eq!(report.external_changes, strings(&["default"]));
        assert!(
            report.not_due.is_empty(),
            "the change made the totals due now"
        );
        assert!(report.reconciled.is_empty());
        assert!(
            storage.aggregate_calls() > calls,
            "a reconcile was attempted"
        );
        let detected = load().await;
        assert!(detected.figures.is_approximate(), "{:?}", detected.figures);
        assert_eq!(detected.total_resources, 1);

        storage.fail.store(false, Ordering::SeqCst);
        let report = provider.reconcile_pass(&mut schedule).await;
        assert_eq!(report.external_changes, strings(&["default"]));
        assert_eq!(report.reconciled, strings(&["default"]));
        let reconciled = load().await;
        assert!(
            matches!(reconciled.figures, Figures::Exact { .. }),
            "{:?}",
            reconciled.figures
        );
        assert_eq!(reconciled.total_resources, 2);
        assert_eq!(reconciled.series[0].points.last().unwrap().cumulative, 2);

        // Settled: the next probe sees the reconciled marker again.
        let report = provider.reconcile_pass(&mut schedule).await;
        assert!(report.external_changes.is_empty());
        assert_eq!(report.not_due, strings(&["default"]));
    }

    /// #1078 decision (c): a tenant that keeps receiving writes never becomes
    /// quiet, yet its charted rings are still re-seeded from storage once
    /// their history is older than [`RESEED_MAX_AGE_FACTOR`] intervals — and
    /// not before.
    #[tokio::test]
    async fn charted_rings_are_reseeded_under_continuous_writes_after_the_max_age() {
        let backend = sqlite();
        create_in(backend.as_ref(), "Patient").await;
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&backend), &test_config())
            .with_counters(Arc::clone(&counters));
        let interval = StdDuration::from_millis(200);
        let mut schedule = ReconcileSchedule::new(interval);
        provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        let report = provider.drain_pending_seeds(&mut schedule).await;
        assert_eq!(report.seeded, strings(&["default"]));
        let seeded_at = || {
            counters
                .series_view(
                    "default",
                    DashboardWindow::LastHour,
                    &["Patient"],
                    Utc::now(),
                )
                .expect("seeded")[0]
                .history_seeded_at
        };
        let first_seed = seeded_at().expect("the seed loaded the ring");

        // An import keeps writing; the totals are held not due so only the
        // rings are in question.
        let write = || counters.record("default", "Patient", 1, Utc::now());
        schedule.next_due.insert(
            "default".to_string(),
            Instant::now() + StdDuration::from_secs(3600),
        );
        write();
        let report = provider.reconcile_pass(&mut schedule).await;
        assert_eq!(report.not_due, strings(&["default"]));
        assert_eq!(report.rings_seeded, 0, "younger than the max age");
        assert_eq!(seeded_at(), Some(first_seed));

        tokio::time::sleep(interval * RESEED_MAX_AGE_FACTOR + StdDuration::from_millis(100)).await;
        write();
        let report = provider.reconcile_pass(&mut schedule).await;
        assert_eq!(
            report.rings_seeded,
            DashboardWindow::ALL.len(),
            "every charted Patient ring is re-seeded despite the writes"
        );
        assert!(seeded_at().is_some_and(|at| at > first_seed));
        let snapshot = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert!(
            snapshot.figures.is_approximate(),
            "the totals still carry unreconciled writes"
        );
    }

    /// #1078 decision (e): a pass visits at most its budget of tenants, most
    /// recently viewed first, and the tenants it skipped go first next pass.
    /// Even with the budget spent, the first seed of a tenant someone just
    /// opened runs, while a purge reseed waits.
    #[tokio::test]
    async fn pass_budget_serves_recent_views_first_and_carries_skipped_tenants_over() {
        let backend = sqlite();
        for tenant in ["a", "b", "c", "d"] {
            create_for(backend.as_ref(), tenant, "Patient").await;
        }
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&backend), &test_config())
            .with_counters(Arc::clone(&counters));
        for tenant in ["a", "b", "c"] {
            provider
                .snapshot(DashboardWindow::LastHour, tenant, &[], false)
                .await;
        }
        let report = provider.drain_pending_seeds(&mut schedule()).await;
        assert_eq!(sorted(report.seeded), strings(&["a", "b", "c"]));

        // Viewed c, then a, then b: b is the most recent.
        for tenant in ["c", "a", "b"] {
            tokio::time::sleep(StdDuration::from_millis(2)).await;
            provider
                .snapshot(DashboardWindow::LastHour, tenant, &[], false)
                .await;
        }
        let mut schedule = schedule();
        schedule.max_tenants_per_pass = 2;
        let report = provider.reconcile_pass(&mut schedule).await;
        assert_eq!(report.reconciled, strings(&["b", "a"]));
        assert_eq!(report.budget_skipped, strings(&["c"]));

        let report = provider.reconcile_pass(&mut schedule).await;
        assert_eq!(
            report.reconciled,
            strings(&["c"]),
            "the skipped tenant goes first"
        );
        assert_eq!(report.not_due, strings(&["b"]));
        assert_eq!(report.budget_skipped, strings(&["a"]));

        schedule.max_tenants_per_pass = 0;
        counters.invalidate_tenant("a");
        let pending = provider
            .snapshot(DashboardWindow::LastHour, "d", &[], false)
            .await;
        assert_eq!(pending.figures, Figures::Pending);
        let report = provider.reconcile_pass(&mut schedule).await;
        assert_eq!(report.seeded, strings(&["d"]), "a recent view's first seed");
        assert!(report.budget_skipped.contains(&"a".to_string()));
        assert!(counters.needs_reseed("a"), "the purge reseed waits");
        assert!(counters.is_seeded("d"));
    }

    /// #1078 decision (d): a tenant nobody views for the idle TTL is evicted —
    /// counters and provider state alike, the default tenant excepted — and
    /// its next view seeds it again from storage.
    #[tokio::test]
    async fn idle_tenants_are_evicted_and_seeded_again_on_their_next_view() {
        let backend = sqlite();
        create_in(backend.as_ref(), "Patient").await;
        create_for(backend.as_ref(), "other", "Patient").await;
        let counters = isolated_counters();
        let provider = StorageDashboardProvider::new(Arc::clone(&backend), &test_config())
            .with_counters(Arc::clone(&counters))
            .with_job_stores(
                None,
                Some(Arc::clone(&backend) as Arc<dyn BulkSubmitJobStore>),
            );
        let mut schedule = schedule();
        for tenant in ["", "other"] {
            provider
                .snapshot(DashboardWindow::LastHour, tenant, &[], false)
                .await;
        }
        let report = provider.drain_pending_seeds(&mut schedule).await;
        assert_eq!(sorted(report.seeded), strings(&["default", "other"]));
        // A queued ring and read job counts for "other"; then nobody looks.
        provider
            .snapshot(
                DashboardWindow::LastDay,
                "other",
                &strings(&["Encounter"]),
                true,
            )
            .await;
        assert!(
            lock(&provider.seeds.rings)
                .iter()
                .any(|(t, _, _)| t == "other")
        );
        assert!(lock(&provider.job_counts).contains_key("other"));

        schedule.idle_ttl = StdDuration::from_millis(50);
        tokio::time::sleep(StdDuration::from_millis(100)).await;
        let report = provider.reconcile_pass(&mut schedule).await;
        assert_eq!(report.dropped, strings(&["other"]), "the default is kept");
        assert!(!counters.is_seeded("other"));
        assert!(counters.is_seeded("default"));
        assert_forgotten(&provider, &schedule, "other");

        let pending = provider
            .snapshot(DashboardWindow::LastHour, "other", &[], false)
            .await;
        assert_eq!(pending.figures, Figures::Pending);
        assert_eq!(queued_tenants(&provider), strings(&["other"]));
        let report = provider.drain_pending_seeds(&mut schedule).await;
        assert_eq!(report.seeded, strings(&["other"]));
        let back = provider
            .snapshot(DashboardWindow::LastHour, "other", &[], false)
            .await;
        assert!(matches!(back.figures, Figures::Exact { .. }));
        assert_eq!(back.total_resources, 1);
    }

    /// #1078 decision (d): `TenantRemoved` drops the tenant's counters at once
    /// (through the write observer), and the provider forgets its queued work,
    /// schedule and job counts when it next runs — a queued ring of the
    /// removed tenant is not loaded.
    #[tokio::test]
    async fn tenant_removed_drops_the_tenants_state() {
        let backend = sqlite();
        create_for(backend.as_ref(), "other", "Patient").await;
        let counters = isolated_counters();
        let observer =
            crate::handlers::dashboard_counts::DashboardCountsObserver::new(Arc::clone(&counters));
        let storage = InstrumentedStorage::over(Arc::clone(&backend));
        let provider = StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
            .with_counters(Arc::clone(&counters))
            .with_job_stores(
                None,
                Some(Arc::clone(&backend) as Arc<dyn BulkSubmitJobStore>),
            );
        let mut schedule = schedule();
        provider
            .snapshot(DashboardWindow::LastHour, "other", &[], false)
            .await;
        provider.drain_pending_seeds(&mut schedule).await;
        assert!(counters.is_seeded("other"));
        provider
            .snapshot(
                DashboardWindow::LastDay,
                "other",
                &strings(&["Encounter"]),
                true,
            )
            .await;
        assert_eq!(lock(&provider.seeds.rings).len(), 1);

        observer.on_write(&WriteEvent::TenantRemoved {
            tenant: TenantId::new("other"),
        });
        assert!(!counters.is_seeded("other"));
        let grouped = storage.grouped_history_calls();
        let report = provider.drain_pending_seeds(&mut schedule).await;
        assert_eq!(report.dropped, strings(&["other"]));
        assert_eq!(report.rings_seeded, 0);
        assert_eq!(
            storage.grouped_history_calls(),
            grouped,
            "the removed tenant's queued ring is not loaded"
        );
        assert_forgotten(&provider, &schedule, "other");

        // A later view starts over.
        let again = provider
            .snapshot(DashboardWindow::LastHour, "other", &[], false)
            .await;
        assert_eq!(again.figures, Figures::Pending);
        assert_eq!(queued_tenants(&provider), strings(&["other"]));
    }

    /// #1078 decision (g): the loop is supervised. A pass that panics (here
    /// the startup seed's totals read) is logged and the loop restarted after
    /// a back-off, and the restarted loop seeds the tenant; the supervisor
    /// stops with its provider.
    #[tokio::test]
    async fn reconcile_loop_is_restarted_after_a_panic_and_still_seeds() {
        let storage = InstrumentedStorage::over(sqlite());
        create_in(storage.as_ref(), "Patient").await;
        storage.panic_once.store(true, Ordering::SeqCst);
        let counters = isolated_counters();
        let provider = Arc::new(
            StorageDashboardProvider::new(Arc::clone(&storage), &test_config())
                .with_counters(Arc::clone(&counters)),
        );
        let handle = spawn_reconcile_loop_every(&provider, StdDuration::from_secs(3600))
            .expect("inside a runtime");

        assert!(
            eventually(|| !storage.panic_once.load(Ordering::SeqCst)).await,
            "the startup seed ran and panicked"
        );
        assert!(
            eventually(|| counters.is_seeded("default")).await,
            "the restarted loop seeded the default tenant"
        );
        assert!(
            !handle.is_finished(),
            "the supervisor keeps the loop running"
        );

        drop(provider);
        let stopped = tokio::time::timeout(StdDuration::from_secs(5), handle)
            .await
            .expect("the supervisor stops with its provider");
        assert!(
            stopped.is_ok() || stopped.as_ref().is_err_and(|error| error.is_cancelled()),
            "{stopped:?}"
        );
    }

    /// The write-marker bound only moves while the figures are already
    /// approximate, so a quiet tenant's markers stay comparable; a probe uses
    /// the bound of the reconcile that stored the marker.
    #[test]
    fn marker_bound_moves_only_while_figures_are_approximate() {
        let mut schedule = schedule();
        let t0 = Utc::now();
        assert_eq!(
            schedule.reconcile_bound("t", false, t0),
            None,
            "first reconcile"
        );
        assert_eq!(schedule.probe_bound("t"), None);

        let t1 = t0 + Duration::minutes(1);
        assert_eq!(
            schedule.reconcile_bound("t", true, t1),
            None,
            "exact figures keep the bound"
        );
        let t2 = t1 + Duration::minutes(1);
        let moved = Some(t1 - RECENT_WRITES_LOOKBACK);
        assert_eq!(schedule.reconcile_bound("t", false, t2), moved);
        assert_eq!(schedule.probe_bound("t"), moved);
        assert_eq!(
            schedule.reconcile_bound("t", true, t2 + Duration::minutes(1)),
            moved
        );

        schedule.forget("t");
        assert_eq!(schedule.probe_bound("t"), None);
    }
}
