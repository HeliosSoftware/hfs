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
//! [`helios_observability::metrics`]). The same [`resource_count_series`] helper
//! also backs the authenticated `/console/metrics/resource-counts` JSON handler,
//! so the cumulative-bucketing semantics live in exactly one place.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use helios_observability::dashboard::{
    DashboardPoint, DashboardProvider, DashboardSeries, DashboardSnapshot, DashboardWindow,
    ExportJobCounts, TypeCount,
};
use helios_persistence::core::{
    BulkExportJobStore, BulkSubmitJobStore, ExportStatus, ResourceStorage, bucket_floor,
};
use helios_persistence::error::StorageResult;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
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
const TYPE_COUNTS_TTL: std::time::Duration = std::time::Duration::from_secs(5);

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
/// JSON endpoint and the web UI dashboard provider.
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
    let bucket = window.bucket_seconds;
    let bucket_span = Duration::seconds(bucket);
    // The window ends with the (partial) bucket `now` falls in, and runs back
    // `points - 1` whole buckets from there.
    let last_bucket = bucket_floor(now, bucket);
    let first_bucket = last_bucket - bucket_span * (window.points.saturating_sub(1) as i32);

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
            .count_deltas_by_bucket(tenant, rt, first_bucket, bucket)
            .await?;

        // Collapse into a bucket -> delta map, keeping only buckets inside the
        // window (defensive against a clock-skewed, future-dated `last_updated`).
        let mut by_bucket: HashMap<DateTime<Utc>, i64> = HashMap::new();
        let mut net_in_window: i64 = 0;
        for d in &deltas {
            if d.bucket_start >= first_bucket && d.bucket_start <= last_bucket {
                *by_bucket.entry(d.bucket_start).or_insert(0) += d.delta;
                net_in_window += d.delta;
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

        series.push(DashboardSeries {
            resource_type: rt.to_string(),
            total,
            points,
        });
    }

    Ok(series)
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
}

impl<S> StorageDashboardProvider<S> {
    /// Builds a provider for the server's default tenant and default FHIR
    /// version. The window and the charted types are chosen per request by the
    /// UI, so neither is fixed here; the default selection is the tenant's
    /// largest stored types (#555). Job-store counts start unwired; call
    /// [`Self::with_job_stores`] to attach them.
    pub(crate) fn new(storage: Arc<S>, config: &ServerConfig) -> Self {
        Self {
            default_tenant: config.default_tenant.clone(),
            fhir_version: config.default_fhir_version.to_string(),
            storage,
            export_jobs: None,
            submit_jobs: None,
            type_counts: Arc::new(RwLock::new(HashMap::new())),
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
}

impl<S> StorageDashboardProvider<S>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    /// The tenant's per-type live totals, served from the per-tenant cache
    /// when it is fresher than [`TYPE_COUNTS_TTL`], else recomputed (#959).
    ///
    /// This is the dashboard's single most expensive query — a `GROUP BY` over
    /// every live row of the tenant — and its result depends on nothing but
    /// the tenant, so it is cached independently of the observability layer's
    /// `(window, tenant, types, include_empty)` snapshot cache.
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

        // Fast path. The read guard is scoped to this block and dropped before
        // the `.await` below, so no lock is ever held across a suspension.
        {
            let fresh = self.type_counts.read().ok().and_then(|guard| {
                guard.get(&tenant_key).and_then(|(at, counts)| {
                    (at.elapsed() < TYPE_COUNTS_TTL).then(|| counts.clone())
                })
            });
            if let Some(counts) = fresh {
                debug!(
                    tenant = %tenant_key,
                    types = counts.len(),
                    "dashboard snapshot: per-type counts served from cache"
                );
                return (counts, false);
            }
        }

        // Cache miss: time the grouping query, so an operator staring at a slow
        // dashboard can tell from the logs *which* query is responsible (#959).
        let started = Instant::now();
        let result = self.storage.count_all_types(tenant).await;
        debug!(
            tenant = %tenant_key,
            elapsed_ms = started.elapsed().as_millis() as u64,
            ok = result.is_ok(),
            "dashboard snapshot: count_all_types completed"
        );

        match result {
            Ok(counts) => {
                if let Ok(mut guard) = self.type_counts.write() {
                    guard.insert(tenant_key, (Instant::now(), counts.clone()));
                }
                (counts, false)
            }
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
        let tenant = TenantContext::new(
            TenantId::new(tenant_id.to_string()),
            TenantPermissions::full_access(),
        );
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
        let (raw_counts, counts_unavailable) = self.cached_count_all_types(&tenant).await;
        partial |= counts_unavailable;
        // The stat card counts only types the tenant actually stores —
        // `include_empty` (#599, "View all resources") never changes this
        // figure, so it must be taken before the flag relaxes the filter
        // below.
        let distinct_types = raw_counts.iter().filter(|(_, total)| *total > 0).count();
        // The headline total is *derived* from the per-type counts rather than
        // read with a second `self.storage.count(&tenant, None)` (#959). That
        // call was a full `COUNT(*)` over exactly the rows `count_all_types`
        // had just grouped and counted — roughly doubling the page's cost at
        // 6M resources.
        //
        // The two figures agree by contract: `ResourceStorage::count` with
        // `None` returns "the count of non-deleted resources" for the tenant,
        // and `count_all_types` "counts non-deleted resources grouped by
        // resource type for `tenant`, returning one `(resource_type, count)`
        // pair per type present". The SQL backends use literally the same
        // predicate for both (`tenant_id = ? AND is_deleted = 0/FALSE`), and
        // `CompositeStorage` delegates both to its primary — so summing the
        // groups reproduces the ungrouped count exactly.
        //
        // Behaviour change: when `count_all_types` fails, `total_resources` is
        // now 0, where before the independent `count()` call might still have
        // succeeded. Acceptable — the failure is already logged by
        // `cached_count_all_types`, and this snapshot is explicitly designed to
        // degrade to zeros rather than to error. Summed saturating so a
        // pathological backend cannot panic the dashboard on overflow.
        //
        // Must be computed here, while `raw_counts` is still alive: the
        // `into_iter()` below consumes it to build `available`.
        let total_resources: u64 = raw_counts
            .iter()
            .map(|(_, total)| *total)
            .fold(0u64, u64::saturating_add);

        // With `include_empty`, a type storage reports with a zero live count
        // is kept instead of dropped, coherent with the selection guard below
        // (which also stops requiring a requested type to already be
        // stored). Backends currently only ever return types with at least
        // one live row (`GROUP BY` over non-deleted rows), so in practice
        // this filter alone rarely surfaces anything new — a type the tenant
        // has *never* stored still isn't known to this provider at all. The
        // union with the FHIR version's full type list (so the picker can
        // offer those too, at 0) is done on the UI side against
        // `resource_type_names()`, the same spec-derived source the other
        // pickers use; see `helios_ui::build_dashboard`.
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

        // The charted set: the caller's selection filtered to real stored
        // types, else the largest few. Capped so the query fan-out (one delta
        // aggregate per type) and the palette stay bounded.
        let selection: Vec<&str> = if types.is_empty() {
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
                // cleanly (`resource_count_series` degrades to a flat zero
                // series rather than erroring or omitting it).
                .filter(|t| include_empty || available.iter().any(|a| &a.resource_type == *t))
                .take(MAX_CHARTED_TYPES)
                .map(|t| t.as_str())
                .collect()
        };

        // Degrade to an empty/zeroed snapshot rather than surfacing an error —
        // the operator dashboard should render even if a count query hiccups —
        // but flag it, so the page says the figures are incomplete instead of
        // charting the fallback as data (#956).
        //
        // Timed at `debug!` alongside the per-type grouping so a slow dashboard
        // can be attributed to one query or the other without a profiler (#959).
        let series_started = Instant::now();
        let series_result = resource_count_series(
            self.storage.as_ref(),
            &tenant,
            &selection,
            SeriesWindow::from_dashboard_window(window),
            now,
        )
        .await;
        debug!(
            tenant = %tenant.tenant_id().as_str(),
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

        // Job counts degrade to `None` (unavailable) rather than zero on a read
        // error: a zero here would tell an operator "no jobs" when the truth is
        // "could not ask". `None` also covers the normal case of a deployment
        // with no bulk-export/bulk-submit job store wired at all. They carry
        // their own unavailable state on the page, so they do not set
        // `partial` — that flag is for figures with no honest rendering of
        // their own.
        let export_jobs = match &self.export_jobs {
            None => None,
            Some(store) => {
                let running = store
                    .count_exports_by_status(&tenant, ExportStatus::InProgress)
                    .await;
                let queued = store
                    .count_exports_by_status(&tenant, ExportStatus::Accepted)
                    .await;
                match (running, queued) {
                    (Ok(running), Ok(queued)) => Some(ExportJobCounts { running, queued }),
                    (Err(error), _) | (_, Err(error)) => {
                        warn!(%error, "dashboard snapshot: export job count query failed");
                        None
                    }
                }
            }
        };

        let import_jobs_active = match &self.submit_jobs {
            None => None,
            Some(store) => match store.count_active_submissions(&tenant).await {
                Ok(n) => Some(n),
                Err(error) => {
                    warn!(%error, "dashboard snapshot: import job count query failed");
                    None
                }
            },
        };

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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;
    use helios_persistence::backends::sqlite::SqliteBackend;

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

        let provider = StorageDashboardProvider::new(Arc::new(backend), &config);
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

        let provider = StorageDashboardProvider::new(Arc::new(backend), &config);
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

        let provider = StorageDashboardProvider::new(Arc::new(backend), &config);
        let requested = vec!["Patient".to_string(), "Observation".to_string()];

        let without_flag = provider
            .snapshot(DashboardWindow::default(), "", &requested, false)
            .await;
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

        let provider = StorageDashboardProvider::new(Arc::clone(&backend), &config);
        let first = provider
            .snapshot(DashboardWindow::LastHour, "", &[], false)
            .await;
        assert_eq!(first.total_resources, 1);
        assert_eq!(first.distinct_types, 1);

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

        let provider = StorageDashboardProvider::new(Arc::new(backend), &config);
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
}
