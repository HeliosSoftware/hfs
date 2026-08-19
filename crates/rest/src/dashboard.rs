//! Storage-backed dashboard data provider for the web UI.
//!
//! The web UI's landing page renders a "FHIR resources over time" chart plus a
//! few headline totals. The data lives behind [`ResourceStorage`], which the UI
//! crate deliberately does not depend on. Instead, this module implements
//! [`helios_observability::dashboard::DashboardProvider`] over the live backend
//! and registers it in [`crate::build_app`]; the UI reads the resulting snapshot
//! through the storage-agnostic `helios-observability` registry.
//!
//! The single-tenant scope matches the operator dashboard: figures reflect the
//! server's **default tenant** only, and are never exported to the public
//! Prometheus `/metrics` endpoint (per the design in
//! [`helios_observability::metrics`]). The same [`resource_count_series`] helper
//! also backs the authenticated `/console/metrics/resource-counts` JSON handler,
//! so the cumulative-bucketing semantics live in exactly one place.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use helios_observability::dashboard::{
    DashboardPoint, DashboardProvider, DashboardSeries, DashboardSnapshot, DashboardWindow,
    TypeCount,
};
use helios_persistence::core::{ResourceStorage, bucket_floor};
use helios_persistence::error::StorageResult;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use tracing::warn;

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

/// [`DashboardProvider`] backed by a live storage backend. Registered once in
/// [`crate::build_app`]; the tenant to chart arrives per call (#344), with the
/// server default as the fallback for an empty id.
pub(crate) struct StorageDashboardProvider<S> {
    default_tenant: String,
    fhir_version: String,
    storage: Arc<S>,
}

impl<S> StorageDashboardProvider<S> {
    /// Builds a provider for the server's default tenant and default FHIR
    /// version. The window and the charted types are chosen per request by the
    /// UI, so neither is fixed here; the default selection is the tenant's
    /// largest stored types (#555).
    pub(crate) fn new(storage: Arc<S>, config: &ServerConfig) -> Self {
        Self {
            default_tenant: config.default_tenant.clone(),
            fhir_version: config.default_fhir_version.to_string(),
            storage,
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

        // What the tenant actually stores, largest first — the picker's option
        // list, and the pool defaults are drawn from (#555).
        let mut available: Vec<TypeCount> = self
            .storage
            .count_all_types(&tenant)
            .await
            .unwrap_or_else(|error| {
                warn!(%error, "dashboard snapshot: distinct-type query failed");
                Vec::new()
            })
            .into_iter()
            .filter(|(_, total)| *total > 0)
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
        let distinct_types = available.len();

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
                .filter(|t| available.iter().any(|a| &a.resource_type == *t))
                .take(MAX_CHARTED_TYPES)
                .map(|t| t.as_str())
                .collect()
        };

        // Degrade to an empty/zeroed snapshot rather than surfacing an error —
        // the operator dashboard should render even if a count query hiccups.
        let series = match resource_count_series(
            self.storage.as_ref(),
            &tenant,
            &selection,
            SeriesWindow::from_dashboard_window(window),
            now,
        )
        .await
        {
            Ok(series) => series,
            Err(error) => {
                warn!(%error, "dashboard snapshot: resource-count series query failed");
                Vec::new()
            }
        };

        let total_resources = self
            .storage
            .count(&tenant, None)
            .await
            .unwrap_or_else(|error| {
                warn!(%error, "dashboard snapshot: total count query failed");
                0
            });

        DashboardSnapshot {
            fhir_version: self.fhir_version.clone(),
            total_resources,
            distinct_types,
            window,
            series,
            available,
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
    /// (all three backend queries succeed and return "nothing yet").
    #[tokio::test]
    async fn snapshot_over_empty_backend_is_zeroed_but_well_formed() {
        let backend = SqliteBackend::in_memory().expect("in-memory sqlite backend");
        backend.init_schema().expect("init schema");
        let config = ServerConfig {
            default_tenant: "default".to_string(),
            ..ServerConfig::for_testing()
        };

        let provider = StorageDashboardProvider::new(Arc::new(backend), &config);
        let snapshot = provider.snapshot(DashboardWindow::default(), "", &[]).await;

        // An empty store has nothing to chart: no available types, no series —
        // the UI renders its explicit empty state from this (#555).
        assert!(snapshot.series.is_empty());
        assert!(snapshot.available.is_empty());
        assert_eq!(snapshot.total_resources, 0);
        assert_eq!(snapshot.distinct_types, 0);
        assert!(!snapshot.fhir_version.is_empty());
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

    fn test_tenant() -> TenantContext {
        TenantContext::new(TenantId::new("default"), TenantPermissions::full_access())
    }
}
