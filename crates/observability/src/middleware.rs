//! Per-request instrumentation middleware.
//!
//! Records request count and latency as Prometheus metrics and opens a tracing
//! span per request. Under the `otel` feature the span is exported as an OTLP
//! trace span by the `tracing-opentelemetry` layer (see [`crate::telemetry`]).
//!
//! Cardinality discipline: the `route` label uses axum's templated
//! [`MatchedPath`] (e.g. `/{resource_type}/{id}`), never the raw URI with
//! concrete IDs. The tenant is recorded as a *span attribute* only (useful for
//! per-tenant latency in traces) and is never a metric label, to avoid
//! unbounded Prometheus series.

use std::time::Instant;

use axum::{extract::MatchedPath, extract::Request, middleware::Next, response::Response};
use tracing::Instrument;

/// Tower/axum middleware (`axum::middleware::from_fn`) that instruments every
/// request. State-free, so it composes with any server's router.
pub async fn track(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_owned())
        .unwrap_or_else(|| "<unmatched>".to_owned());
    let tenant = req
        .headers()
        .get("x-tenant-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    let span = tracing::info_span!(
        "http.request",
        http.method = %method,
        http.route = %route,
        tenant = %tenant,
        http.status_code = tracing::field::Empty,
    );

    let start = Instant::now();
    let response = next.run(req).instrument(span.clone()).await;
    let elapsed = start.elapsed().as_secs_f64();
    let status = response.status().as_u16();

    span.record("http.status_code", status);

    // Feed the in-process rolling log that backs the dashboard traffic widgets
    // (windowed req/s, latency percentiles, per-tenant rollup). Cheap: a single
    // bounded-buffer push. Tenant is recorded here for the per-tenant view but,
    // as above, never becomes a Prometheus label.
    crate::reqlog::record(status, elapsed, &tenant);

    let method = method.as_str().to_owned();
    let status = status.to_string();
    metrics::counter!(
        "http_requests_total",
        "method" => method.clone(),
        "route" => route.clone(),
        "status" => status.clone(),
    )
    .increment(1);
    metrics::histogram!(
        "http_request_duration_seconds",
        "method" => method,
        "route" => route,
        "status" => status,
    )
    .record(elapsed);

    response
}
