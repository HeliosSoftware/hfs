//! Per-request instrumentation middleware.
//!
//! Records request count and latency as Prometheus metrics, optionally opens a
//! tracing span, and optionally feeds the in-process request-log ring buffer.
//! Under the `otel` feature the span is exported as an OTLP trace span by the
//! `tracing-opentelemetry` layer (see [`crate::telemetry`]).
//!
//! What actually runs per request is decided by [`crate::mode`] plus two
//! runtime facts, so instrumentation nobody consumes is not paid for:
//!
//! - the **span** is opened only when a layer exports it
//!   ([`crate::telemetry::traces_live`]) — an unexported span is created,
//!   entered on every poll of the handler future, and dropped unread, which is
//!   pure overhead at tens of thousands of req/s;
//! - the **reqlog** push happens only when a server has registered a consumer
//!   ([`crate::reqlog::enabled`]) — `hts` writes no dashboard, so it should not
//!   pay for a ring buffer only `hfs`'s console reads.
//!
//! Metrics are the exception: `/metrics` is a public product surface, so they
//! are always recorded except under the [`crate::mode::ObsMode::Off`] floor arm.
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
    let arm = crate::mode::mode();
    let span_on = arm.span_enabled(crate::telemetry::traces_live());
    let metrics_on = arm.metrics_enabled();
    let reqlog_on = arm.reqlog_enabled(crate::reqlog::enabled());

    // Off arm — and any config where nothing consumes instrumentation — runs
    // the handler with no per-request work at all. No `MatchedPath` lookup, no
    // header scan, no allocation.
    if !span_on && !metrics_on && !reqlog_on {
        return next.run(req).await;
    }

    let method = req.method().clone();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_owned())
        .unwrap_or_else(|| "<unmatched>".to_owned());
    // Only the span and the reqlog rollup use the tenant; skip the header scan
    // and its allocation when neither is active.
    let tenant = if span_on || reqlog_on {
        req.headers()
            .get("x-tenant-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned()
    } else {
        String::new()
    };

    let start = Instant::now();
    let response = if span_on {
        let span = tracing::info_span!(
            "http.request",
            http.method = %method,
            http.route = %route,
            tenant = %tenant,
            http.status_code = tracing::field::Empty,
        );
        let response = next.run(req).instrument(span.clone()).await;
        span.record("http.status_code", response.status().as_u16());
        response
    } else {
        next.run(req).await
    };
    let elapsed = start.elapsed().as_secs_f64();
    let status = response.status().as_u16();

    if reqlog_on {
        // Feed the in-process rolling log that backs the dashboard traffic
        // widgets (windowed req/s, latency percentiles, per-tenant rollup).
        // Cheap: a single bounded-buffer push. Tenant is recorded here for the
        // per-tenant view but, as above, never becomes a Prometheus label.
        crate::reqlog::record(status, elapsed, &tenant);
    }

    if metrics_on {
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
    }

    response
}
