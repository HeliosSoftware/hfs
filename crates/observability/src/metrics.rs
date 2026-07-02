//! Prometheus metrics: global recorder installation and the `/metrics` endpoint.
//!
//! Uses the [`metrics`] facade with the maintained
//! [`metrics_exporter_prometheus`] pull exporter. We deliberately avoid
//! `opentelemetry-prometheus` (unmaintained, carries a RUSTSEC advisory via
//! `protobuf`). OTLP *metrics* are produced out-of-process by an OpenTelemetry
//! Collector scraping this `/metrics` endpoint.

use std::sync::OnceLock;

use axum::{Router, http::StatusCode, response::IntoResponse, routing::get};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};

use crate::uptime;

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Latency histogram buckets (seconds) for `http_request_duration_seconds`.
/// Configuring explicit buckets makes `metrics-exporter-prometheus` emit a real
/// Prometheus **histogram** (`_bucket` / `le` series) instead of a summary, so
/// `histogram_quantile()` works and quantiles aggregate correctly across
/// instances scraped into one backend. A standard HTTP-latency spread.
const LATENCY_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Install the global Prometheus recorder. A global `service="<service_name>"`
/// label is attached so the generic metric names (`http_requests_total`, …)
/// stay distinct across the different Helios server processes when scraped into
/// one backend. Idempotent across repeated calls within a process: the first
/// call installs the recorder, later calls are no-ops (useful for tests).
///
/// # Panics
///
/// Panics only if a *different* global metrics recorder was already installed by
/// something other than this function.
pub fn init(service_name: &str) {
    if HANDLE.get().is_some() {
        return;
    }
    let handle = PrometheusBuilder::new()
        .add_global_label("service", service_name)
        .set_buckets_for_metric(
            Matcher::Full("http_request_duration_seconds".to_string()),
            LATENCY_BUCKETS,
        )
        .expect("failed to set latency histogram buckets")
        .install_recorder()
        .expect("failed to install Prometheus recorder");
    let _ = HANDLE.set(handle);
}

/// A state-free [`Router`] exposing `GET /metrics`. Merge it into each server's
/// router with `router.merge(helios_observability::metrics::router())`.
pub fn router() -> Router {
    Router::new().route("/metrics", get(render))
}

// Note: per-tenant resource-count gauges are intentionally NOT exported here.
// The `/metrics` endpoint is public (unauthenticated, for Prometheus scraping),
// and tenant is never a metric label — exporting per-tenant counts would leak
// cross-tenant data to any anonymous scraper. Per-tenant stored-resource counts
// are served only via the authenticated console `resource-counts` JSON endpoint.

/// Render the Prometheus exposition text. The `uptime_seconds` gauge is set
/// immediately before rendering because the pull exporter has no scrape
/// callback.
async fn render() -> impl IntoResponse {
    metrics::gauge!("uptime_seconds").set(uptime::uptime_seconds());
    match HANDLE.get() {
        Some(handle) => handle.render().into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "metrics recorder not initialized\n",
        )
            .into_response(),
    }
}
