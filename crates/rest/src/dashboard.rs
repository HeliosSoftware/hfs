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
    DashboardPoint, DashboardProvider, DashboardSeries, DashboardSnapshot,
};
use helios_persistence::core::ResourceStorage;
use helios_persistence::error::StorageResult;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use tracing::warn;

use crate::config::ServerConfig;

/// Resource types charted by the dashboard's "FHIR Resources over time" card.
/// Mirrors the console handler's default set
/// (`crate::handlers::console_metrics::DEFAULT_TYPES`).
pub(crate) const DEFAULT_DASHBOARD_TYPES: &[&str] = &[
    "Patient",
    "Observation",
    "Encounter",
    "Condition",
    "MedicationRequest",
    "DiagnosticReport",
    "Procedure",
    "AllergyIntolerance",
];

/// Number of daily buckets in the dashboard's default over-time window.
pub(crate) const DEFAULT_DASHBOARD_DAYS: i64 = 30;

/// Builds a dense daily cumulative series for each requested resource type.
///
/// For each type it returns the current `total` and `days` daily points ending
/// on the UTC day of `now`. Each point carries the resources whose most-recent
/// version landed that day (`count`) and the running total through that day
/// (`cumulative`). The cumulative curve starts from the count of resources last
/// updated *before* the window (`total - in_window`) and converges to `total` on
/// the final day, giving a growth curve whose endpoint matches
/// [`ResourceStorage::count`]. See [`ResourceStorage::count_by_day`] for the
/// exact bucketing semantics.
///
/// This is the shared implementation behind both the console
/// `resource-counts` JSON endpoint and the web UI dashboard provider.
pub(crate) async fn resource_count_series<S>(
    storage: &S,
    tenant: &TenantContext,
    types: &[&str],
    days: i64,
    now: DateTime<Utc>,
) -> StorageResult<Vec<DashboardSeries>>
where
    S: ResourceStorage + Sync,
{
    let today = now.date_naive();
    let start_date = today - Duration::days(days - 1);
    let since = start_date
        .and_hms_opt(0, 0, 0)
        .unwrap_or_default()
        .and_utc();

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

        let buckets = storage.count_by_day(tenant, rt, since).await?;

        // Collapse buckets into a day -> count map, summing only days inside the
        // window (defensive against any future-dated `last_updated`).
        let mut by_day: HashMap<chrono::NaiveDate, u64> = HashMap::new();
        let mut in_window: u64 = 0;
        for b in &buckets {
            if b.day >= start_date && b.day <= today {
                *by_day.entry(b.day).or_insert(0) += b.count;
                in_window += b.count;
            }
        }

        // Resources last updated before the window form the cumulative baseline.
        let baseline = total.saturating_sub(in_window);

        let mut points = Vec::with_capacity(days.max(0) as usize);
        let mut cumulative = baseline;
        for i in 0..days {
            let d = start_date + Duration::days(i);
            let count = by_day.get(&d).copied().unwrap_or(0);
            cumulative += count;
            points.push(DashboardPoint {
                date: d.format("%Y-%m-%d").to_string(),
                count,
                cumulative,
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

/// [`DashboardProvider`] backed by a live storage backend, scoped to the
/// server's default tenant. Registered once in [`crate::build_app`].
pub(crate) struct StorageDashboardProvider<S> {
    tenant: TenantContext,
    fhir_version: String,
    types: Vec<String>,
    days: i64,
    storage: Arc<S>,
}

impl<S> StorageDashboardProvider<S> {
    /// Builds a provider for the server's default tenant and default FHIR
    /// version, charting [`DEFAULT_DASHBOARD_TYPES`] over
    /// [`DEFAULT_DASHBOARD_DAYS`].
    pub(crate) fn new(storage: Arc<S>, config: &ServerConfig) -> Self {
        let tenant = TenantContext::new(
            TenantId::new(config.default_tenant.clone()),
            TenantPermissions::full_access(),
        );
        Self {
            tenant,
            fhir_version: config.default_fhir_version.to_string(),
            types: DEFAULT_DASHBOARD_TYPES
                .iter()
                .map(|t| t.to_string())
                .collect(),
            days: DEFAULT_DASHBOARD_DAYS,
            storage,
        }
    }
}

#[async_trait]
impl<S> DashboardProvider for StorageDashboardProvider<S>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    async fn snapshot(&self) -> DashboardSnapshot {
        let now = Utc::now();
        let type_refs: Vec<&str> = self.types.iter().map(|t| t.as_str()).collect();

        // Degrade to an empty/zeroed snapshot rather than surfacing an error —
        // the operator dashboard should render even if a count query hiccups.
        let series = match resource_count_series(
            self.storage.as_ref(),
            &self.tenant,
            &type_refs,
            self.days,
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

        let total_resources =
            self.storage
                .count(&self.tenant, None)
                .await
                .unwrap_or_else(|error| {
                    warn!(%error, "dashboard snapshot: total count query failed");
                    0
                });

        let distinct_types = self
            .storage
            .count_all_types(&self.tenant)
            .await
            .map(|types| types.len())
            .unwrap_or_else(|error| {
                warn!(%error, "dashboard snapshot: distinct-type query failed");
                0
            });

        DashboardSnapshot {
            fhir_version: self.fhir_version.clone(),
            total_resources,
            distinct_types,
            series,
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
        let snapshot = provider.snapshot().await;

        // One series per charted type, each a dense 30-day window of zeros.
        assert_eq!(snapshot.series.len(), DEFAULT_DASHBOARD_TYPES.len());
        assert!(snapshot.series.iter().all(|s| s.total == 0));
        assert!(
            snapshot
                .series
                .iter()
                .all(|s| s.points.len() == DEFAULT_DASHBOARD_DAYS as usize)
        );
        assert!(
            snapshot
                .series
                .iter()
                .all(|s| s.points.iter().all(|p| p.cumulative == 0))
        );
        assert_eq!(snapshot.total_resources, 0);
        assert_eq!(snapshot.distinct_types, 0);
        assert!(!snapshot.fhir_version.is_empty());
    }
}
