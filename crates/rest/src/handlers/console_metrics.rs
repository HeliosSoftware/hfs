//! Management-console metrics endpoints.
//!
//! These return plain JSON (not FHIR resources) for the admin console
//! dashboard:
//!
//! - `GET /console/metrics/uptime` — process uptime + server identity.
//! - `GET /console/metrics/resource-counts` — per-type stored-resource counts
//!   and a daily "resources over time" series for charting.
//!
//! Trust tiers are assigned in [`crate::routing::console_metrics`]: `uptime` is
//! public (mounted outside auth, like `/metrics`); `resource-counts`, `activity`,
//! and `resource-distribution` require authentication and are tenant-scoped; and
//! the cross-tenant `tenants` / `traffic` endpoints additionally require a
//! system-context scope (`system/*.r`). Per-tenant resource counts are served
//! only in these authenticated JSON responses — they are deliberately NOT
//! exported to the public `/metrics` endpoint, because tenant is never a metric
//! label and `/metrics` is unauthenticated.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_persistence::core::ResourceStorage;
use serde_json::json;
use tracing::debug;

use crate::error::RestResult;
use crate::extractors::TenantExtractor;
use crate::state::AppState;

/// Resource types charted by the console dashboard's "FHIR Resources over time"
/// card when the caller does not specify an explicit `types` filter.
const DEFAULT_TYPES: &[&str] = &[
    "Patient",
    "Observation",
    "Encounter",
    "Condition",
    "MedicationRequest",
    "DiagnosticReport",
    "Procedure",
    "AllergyIntolerance",
];

/// Default number of daily buckets in the over-time series.
const DEFAULT_WINDOW_DAYS: i64 = 30;
/// Upper bound on the requested window, to keep the response and query bounded.
const MAX_WINDOW_DAYS: i64 = 365;
/// Cap on how many resource types a single `resource-counts` request may chart.
/// The list drives one batched `count_by_types` query plus one `count_by_day`
/// per type, so this bounds the work an authenticated caller can amplify via the
/// `types` parameter.
const MAX_REQUESTED_TYPES: usize = 50;

/// `GET /console/metrics/uptime`
///
/// Reports process uptime (seconds since start, plus a human-readable form),
/// the start timestamp, and basic server identity. `availability` is surfaced
/// as explicitly untracked rather than fabricated — a real availability
/// percentage (e.g. "99.98% over 30d") would require health-probe history that
/// HFS does not yet record.
pub async fn uptime_handler<S>(State(_state): State<AppState<S>>) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    debug!("Processing console uptime request");

    let seconds = helios_observability::uptime::uptime_seconds();

    let body = json!({
        "service": "hfs",
        "version": env!("CARGO_PKG_VERSION"),
        "started_at": helios_observability::uptime::started_at_rfc3339(),
        "now": chrono::Utc::now().to_rfc3339(),
        "uptime_seconds": seconds,
        "uptime_human": humanize_uptime(seconds),
        "availability": {
            "tracked": false,
            "note": "availability tracking is not enabled; uptime_seconds reflects \
                     time since the current process started"
        }
    });

    Ok((StatusCode::OK, Json(body)).into_response())
}

/// `GET /console/metrics/resource-counts`
///
/// Query parameters (all optional):
/// - `types` — comma-separated FHIR resource types (defaults to the eight
///   dashboard types in [`DEFAULT_TYPES`]).
/// - `days` — size of the daily window, clamped to `1..=365` (default 30).
///
/// For each requested type it returns the current `total` and a dense daily
/// `points` array (`date`, `count`, `cumulative`). The cumulative series starts
/// from the count of resources last updated *before* the window and converges
/// to `total` at the final day, giving a growth-style curve whose endpoint
/// matches the dashboard's "Stored resources" stat. See
/// [`ResourceStorage::count_by_day`] for the exact bucketing semantics.
pub async fn resource_counts_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    Query(params): Query<HashMap<String, String>>,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    let ctx = tenant.context();
    let tenant_id = tenant.tenant_id();
    debug!(tenant = %tenant_id, "Processing console resource-counts request");

    // Window size; clamp to a sane range so a stray `?days=100000` can't force an
    // unbounded scan or response. Unparseable values fall back to the default.
    let days = params
        .get("days")
        .and_then(|d| d.parse::<i64>().ok())
        .unwrap_or(DEFAULT_WINDOW_DAYS)
        .clamp(1, MAX_WINDOW_DAYS);

    // Resource types to chart: an explicit comma-separated `types` filter
    // (whitespace-trimmed, empty entries dropped), otherwise the dashboard set.
    // Capped at MAX_REQUESTED_TYPES so the per-type query fan-out stays bounded.
    let mut types: Vec<String> = match params.get("types") {
        Some(s) if !s.trim().is_empty() => s
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
        _ => DEFAULT_TYPES.iter().map(|t| t.to_string()).collect(),
    };
    types.truncate(MAX_REQUESTED_TYPES);

    // Window: `days` daily buckets ending today (all UTC, midnight-aligned).
    let now = chrono::Utc::now();
    let today = now.date_naive();
    let start_date = today - chrono::Duration::days(days - 1);
    let since = start_date
        .and_hms_opt(0, 0, 0)
        .unwrap_or_default()
        .and_utc();

    // Fetch all per-type totals in one batched call (backends may serve this
    // with a single grouped query), instead of one `count()` round-trip per
    // type. Build a type -> total lookup; a type with no row counts as 0.
    let type_refs: Vec<&str> = types.iter().map(|t| t.as_str()).collect();
    let totals_by_type: HashMap<String, u64> = state
        .storage()
        .count_by_types(ctx, &type_refs)
        .await?
        .into_iter()
        .collect();

    let mut series = Vec::with_capacity(types.len());
    for rt in &types {
        let total = totals_by_type.get(rt.as_str()).copied().unwrap_or(0);

        let buckets = state.storage().count_by_day(ctx, rt.as_str(), since).await?;

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

        let mut points = Vec::with_capacity(days as usize);
        let mut cumulative = baseline;
        for i in 0..days {
            let d = start_date + chrono::Duration::days(i);
            let count = by_day.get(&d).copied().unwrap_or(0);
            cumulative += count;
            points.push(json!({
                "date": d.format("%Y-%m-%d").to_string(),
                "count": count,
                "cumulative": cumulative,
            }));
        }

        series.push(json!({
            "resource_type": rt,
            "total": total,
            "points": points,
        }));
    }

    let total_resources = state.storage().count(ctx, None).await?;

    let body = json!({
        "tenant": tenant_id,
        "generated_at": now.to_rfc3339(),
        "window": {
            "interval": "day",
            "days": days,
            "since": start_date.format("%Y-%m-%d").to_string(),
            "until": today.format("%Y-%m-%d").to_string(),
        },
        "total_resources": total_resources,
        "series": series,
    });

    Ok((StatusCode::OK, Json(body)).into_response())
}

/// `GET /console/metrics/activity`
///
/// Weekly-rhythm activity heatmap: write operations (resource versions) bucketed
/// by UTC weekday (`0` = Sunday … `6` = Saturday) and hour (`0` … `23`) over a
/// rolling window.
///
/// Query parameters (all optional):
/// - `days` — window size, clamped to `1..=365` (default 30).
///
/// The `cells` array is **dense** — all 168 `weekday × hour` cells, zero-filled,
/// ascending by `weekday` then `hour` — so the frontend renders the grid without
/// gap-filling. `max_cell` is provided for colour scaling. `source` reports where
/// the data came from; this version always reports `"writes"` (from
/// `resource_history`), leaving room to add an `AuditEvent`-backed
/// `"all-operations"` source without changing the response shape.
pub async fn activity_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    Query(params): Query<HashMap<String, String>>,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    let ctx = tenant.context();
    let tenant_id = tenant.tenant_id();
    debug!(tenant = %tenant_id, "Processing console activity request");

    let days = params
        .get("days")
        .and_then(|d| d.parse::<i64>().ok())
        .unwrap_or(DEFAULT_WINDOW_DAYS)
        .clamp(1, MAX_WINDOW_DAYS);

    let now = chrono::Utc::now();
    let today = now.date_naive();
    let start_date = today - chrono::Duration::days(days - 1);
    let since = start_date
        .and_hms_opt(0, 0, 0)
        .unwrap_or_default()
        .and_utc();

    let histogram = state.storage().activity_histogram(ctx, since).await?;

    // Densify into a 7×24 grid, summing any duplicate cells defensively.
    let mut grid = [[0u64; 24]; 7];
    for cell in &histogram {
        let wd = cell.weekday.min(6) as usize;
        let hr = cell.hour.min(23) as usize;
        grid[wd][hr] = grid[wd][hr].saturating_add(cell.count);
    }

    let mut cells = Vec::with_capacity(7 * 24);
    let mut total: u64 = 0;
    let mut max_cell: u64 = 0;
    for (wd, hours) in grid.iter().enumerate() {
        for (hr, &count) in hours.iter().enumerate() {
            total += count;
            max_cell = max_cell.max(count);
            cells.push(json!({ "weekday": wd, "hour": hr, "count": count }));
        }
    }

    let body = json!({
        "tenant": tenant_id,
        "generated_at": now.to_rfc3339(),
        "source": "writes",
        "window": {
            "days": days,
            "since": start_date.format("%Y-%m-%d").to_string(),
            "until": today.format("%Y-%m-%d").to_string(),
        },
        "total": total,
        "max_cell": max_cell,
        "cells": cells,
    });

    Ok((StatusCode::OK, Json(body)).into_response())
}

/// `GET /console/metrics/traffic`
///
/// Windowed request throughput, latency percentiles, error rate, and a sparkline
/// — derived from the in-process rolling request log
/// ([`helios_observability::reqlog`]). Unlike the Prometheus `/metrics` counters,
/// these arrive already windowed and ready to chart, with no PromQL.
///
/// Query parameters (all optional):
/// - `window` — look-back in seconds, clamped to `60..=86_400` (default 3600).
/// - `tenant` — restrict to a single tenant id (default: all traffic).
///
/// The window is in-process and resets on restart; it covers at most the most
/// recent [`helios_observability::reqlog::CAPACITY`] requests.
pub async fn traffic_handler<S>(
    State(_state): State<AppState<S>>,
    Query(params): Query<HashMap<String, String>>,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    debug!("Processing console traffic request");

    let window = params
        .get("window")
        .and_then(|w| w.parse::<i64>().ok())
        .unwrap_or(3600)
        .clamp(60, 86_400);
    let tenant_filter = params.get("tenant").map(|s| s.as_str());

    let stats = helios_observability::reqlog::snapshot(window, tenant_filter);

    let series: Vec<_> = stats
        .series
        .iter()
        .map(|b| {
            json!({
                "offset_seconds": b.offset_seconds,
                "requests_per_second": b.requests_per_second,
                "p95_ms": b.p95_ms,
            })
        })
        .collect();

    let body = json!({
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "window_seconds": stats.window_seconds,
        "covered_seconds": stats.covered_seconds,
        "sample_count": stats.sample_count,
        "requests_per_second": stats.requests_per_second,
        "latency_ms": { "p50": stats.p50_ms, "p95": stats.p95_ms, "p99": stats.p99_ms },
        "error_rate": stats.error_rate,
        "status_classes": {
            "2xx": stats.status_2xx,
            "3xx": stats.status_3xx,
            "4xx": stats.status_4xx,
            "5xx": stats.status_5xx,
        },
        "series": series,
    });

    Ok((StatusCode::OK, Json(body)).into_response())
}

/// `GET /console/metrics/resource-distribution`
///
/// Every stored resource type for the tenant with its count, busiest first — the
/// data behind the "resource composition" treemap.
///
/// Query parameters (all optional):
/// - `top` — keep the N largest types and roll the remainder into a single
///   `other` bucket, clamped to `1..=100` (default 20).
pub async fn resource_distribution_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    Query(params): Query<HashMap<String, String>>,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    let ctx = tenant.context();
    let tenant_id = tenant.tenant_id();
    debug!(tenant = %tenant_id, "Processing console resource-distribution request");

    let top = params
        .get("top")
        .and_then(|t| t.parse::<usize>().ok())
        .unwrap_or(20)
        .clamp(1, 100);

    let mut counts = state.storage().count_all_types(ctx).await?;
    // Busiest first; tie-break alphabetically for stable output.
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let total: u64 = counts.iter().map(|(_, n)| *n).sum();
    let distinct_types = counts.len();

    let mut types = Vec::new();
    if counts.len() > top {
        for (rt, n) in &counts[..top] {
            types.push(json!({ "resource_type": rt, "count": n }));
        }
        let tail = &counts[top..];
        let other_count: u64 = tail.iter().map(|(_, n)| *n).sum();
        types.push(json!({
            "resource_type": "other",
            "count": other_count,
            "types": tail.len(),
        }));
    } else {
        for (rt, n) in &counts {
            types.push(json!({ "resource_type": rt, "count": n }));
        }
    }

    let body = json!({
        "tenant": tenant_id,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "total_resources": total,
        "distinct_types": distinct_types,
        "types": types,
    });

    Ok((StatusCode::OK, Json(body)).into_response())
}

/// `GET /console/metrics/tenants`
///
/// Per-tenant comparison: authoritative stored-resource counts (cross-tenant
/// backend aggregate) joined with windowed traffic (best-effort, from the
/// in-process request log). Sorted by resource count, busiest first.
///
/// Query parameters (all optional):
/// - `window` — traffic look-back in seconds, clamped to `60..=86_400`
///   (default 3600).
///
/// Traffic is keyed by the `X-Tenant-ID` seen on requests (empty → `"default"`),
/// so a tenant with stored data but no recent traffic reports zero rates, and a
/// tenant with traffic but no stored resources still appears (with `resources:
/// 0`).
pub async fn tenants_handler<S>(
    State(state): State<AppState<S>>,
    Query(params): Query<HashMap<String, String>>,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    debug!("Processing console tenants request");

    let window = params
        .get("window")
        .and_then(|w| w.parse::<i64>().ok())
        .unwrap_or(3600)
        .clamp(60, 86_400);

    let mut resource_counts = state.storage().count_by_tenant().await?;
    resource_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let traffic = helios_observability::reqlog::per_tenant(window);
    let traffic_by_tenant: HashMap<&str, &helios_observability::reqlog::TenantTraffic> =
        traffic.iter().map(|t| (t.tenant.as_str(), t)).collect();

    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut tenants = Vec::new();
    for (tenant, resources) in &resource_counts {
        seen.insert(tenant.as_str());
        let tr = traffic_by_tenant.get(tenant.as_str());
        tenants.push(json!({
            "tenant": tenant,
            "resources": resources,
            "requests_per_second": tr.map(|t| t.requests_per_second).unwrap_or(0.0),
            "p95_ms": tr.map(|t| t.p95_ms).unwrap_or(0.0),
            "error_rate": tr.map(|t| t.error_rate).unwrap_or(0.0),
        }));
    }
    // Tenants seen only in traffic (no stored resources yet).
    for t in &traffic {
        if !seen.contains(t.tenant.as_str()) {
            tenants.push(json!({
                "tenant": t.tenant,
                "resources": 0,
                "requests_per_second": t.requests_per_second,
                "p95_ms": t.p95_ms,
                "error_rate": t.error_rate,
            }));
        }
    }

    let body = json!({
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "window_seconds": window,
        "tenant_count": tenants.len(),
        "tenants": tenants,
    });

    Ok((StatusCode::OK, Json(body)).into_response())
}

/// Formats a duration in seconds as a compact `"4d 2h 13m 5s"` string, omitting
/// leading zero units. Always shows at least seconds (`"0s"`).
fn humanize_uptime(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let days = total / 86_400;
    let hours = (total % 86_400) / 3_600;
    let mins = (total % 3_600) / 60;
    let secs = total % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 || !parts.is_empty() {
        parts.push(format!("{hours}h"));
    }
    if mins > 0 || !parts.is_empty() {
        parts.push(format!("{mins}m"));
    }
    parts.push(format!("{secs}s"));
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanize_uptime_formats_units() {
        assert_eq!(humanize_uptime(0.0), "0s");
        assert_eq!(humanize_uptime(45.0), "45s");
        assert_eq!(humanize_uptime(90.0), "1m 30s");
        assert_eq!(humanize_uptime(3_661.0), "1h 1m 1s");
        assert_eq!(humanize_uptime(90_061.0), "1d 1h 1m 1s");
    }

    #[test]
    fn humanize_uptime_clamps_negative() {
        assert_eq!(humanize_uptime(-5.0), "0s");
    }
}
