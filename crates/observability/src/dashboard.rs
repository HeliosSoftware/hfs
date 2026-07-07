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

use std::sync::{Arc, RwLock};

use async_trait::async_trait;

/// One daily point of a single resource type's cumulative growth curve.
#[derive(Clone, Debug)]
pub struct DashboardPoint {
    /// `YYYY-MM-DD` (UTC) label for the bucket.
    pub date: String,
    /// Resources whose most-recent version landed on this day.
    pub count: u64,
    /// Running total through this day (converges to the series `total`).
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
    /// Per-type series for the charted resource types, in display order.
    pub series: Vec<DashboardSeries>,
}

/// Supplies [`DashboardSnapshot`]s on demand. Implemented in `helios-rest` over
/// the live storage backend and registered via [`set_provider`] at startup.
#[async_trait]
pub trait DashboardProvider: Send + Sync {
    /// Compute a fresh snapshot. Called per dashboard page load, so
    /// implementations keep the query fan-out bounded (a handful of resource
    /// types) and degrade gracefully — returning zeros — rather than erroring.
    async fn snapshot(&self) -> DashboardSnapshot;
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

/// Fetch a fresh dashboard snapshot, or `None` when no provider is registered
/// (e.g. a server build without persistence, or the standalone UI example). The
/// UI falls back to placeholder figures in that case.
pub async fn snapshot() -> Option<DashboardSnapshot> {
    let provider = provider()?;
    Some(provider.snapshot().await)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(DashboardSnapshot);

    #[async_trait]
    impl DashboardProvider for Fixed {
        async fn snapshot(&self) -> DashboardSnapshot {
            self.0.clone()
        }
    }

    #[tokio::test]
    async fn registered_provider_snapshot_round_trips() {
        set_provider(Arc::new(Fixed(DashboardSnapshot {
            fhir_version: "R4".to_string(),
            total_resources: 42,
            distinct_types: 3,
            series: vec![DashboardSeries {
                resource_type: "Patient".to_string(),
                total: 7,
                points: vec![DashboardPoint {
                    date: "2026-07-07".to_string(),
                    count: 7,
                    cumulative: 7,
                }],
            }],
        })));

        let snap = snapshot().await.expect("provider registered");
        assert_eq!(snap.total_resources, 42);
        assert_eq!(snap.distinct_types, 3);
        assert_eq!(snap.series.len(), 1);
        assert_eq!(snap.series[0].resource_type, "Patient");
        assert_eq!(snap.series[0].points.last().unwrap().cumulative, 7);
    }
}
