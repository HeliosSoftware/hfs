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

use std::borrow::Cow;
use std::time::Instant;

use axum::{extract::MatchedPath, extract::Request, middleware::Next, response::Response};
use tracing::Instrument;

/// Map an HTTP method to a metric label without allocating for the standard
/// verbs.
///
/// The `metrics` label value is a `SharedString` (`Cow<'static, str>`), so a
/// `Cow::Borrowed(&'static str)` clones for free — the always-on metrics path
/// records two metrics per request (a counter and a histogram) and would
/// otherwise heap-allocate the method string twice. Exotic methods keep their
/// exact spelling via an owned fallback, so the emitted label value is
/// byte-for-byte what `method.as_str()` produced before.
fn method_label(method: &axum::http::Method) -> Cow<'static, str> {
    match method.as_str() {
        "GET" => Cow::Borrowed("GET"),
        "POST" => Cow::Borrowed("POST"),
        "PUT" => Cow::Borrowed("PUT"),
        "DELETE" => Cow::Borrowed("DELETE"),
        "PATCH" => Cow::Borrowed("PATCH"),
        "HEAD" => Cow::Borrowed("HEAD"),
        "OPTIONS" => Cow::Borrowed("OPTIONS"),
        "CONNECT" => Cow::Borrowed("CONNECT"),
        "TRACE" => Cow::Borrowed("TRACE"),
        other => Cow::Owned(other.to_owned()),
    }
}

/// Map an HTTP status code to a metric label without allocating for the common
/// codes. Same rationale as [`method_label`]; the owned fallback for an
/// uncommon code renders identically to `status.to_string()`.
fn status_label(status: u16) -> Cow<'static, str> {
    match status {
        200 => Cow::Borrowed("200"),
        201 => Cow::Borrowed("201"),
        202 => Cow::Borrowed("202"),
        204 => Cow::Borrowed("204"),
        301 => Cow::Borrowed("301"),
        302 => Cow::Borrowed("302"),
        304 => Cow::Borrowed("304"),
        400 => Cow::Borrowed("400"),
        401 => Cow::Borrowed("401"),
        403 => Cow::Borrowed("403"),
        404 => Cow::Borrowed("404"),
        405 => Cow::Borrowed("405"),
        406 => Cow::Borrowed("406"),
        409 => Cow::Borrowed("409"),
        410 => Cow::Borrowed("410"),
        412 => Cow::Borrowed("412"),
        415 => Cow::Borrowed("415"),
        422 => Cow::Borrowed("422"),
        429 => Cow::Borrowed("429"),
        500 => Cow::Borrowed("500"),
        501 => Cow::Borrowed("501"),
        502 => Cow::Borrowed("502"),
        503 => Cow::Borrowed("503"),
        other => Cow::Owned(other.to_string()),
    }
}

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
        // Labels are `Cow<'static, str>`: the standard method/status values are
        // `Cow::Borrowed`, so cloning them for the second metric is free. Only
        // `route` (a matched-path template that is not `'static`) still owns a
        // clone. This keeps the always-on metrics path — recorded on every
        // request in every mode except `Off` — from heap-allocating the method
        // and status strings four times per request. Label values are unchanged.
        let method = method_label(&method);
        let status = status_label(status);
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

#[cfg(test)]
mod label_tests {
    use super::*;
    use axum::http::Method;

    // The label helpers must be byte-identical to the pre-optimization
    // `method.as_str().to_owned()` / `status.to_string()` for every value, so
    // the Prometheus series they feed are unchanged — only the allocation goes.
    #[test]
    fn method_label_matches_as_str_for_all_verbs() {
        for m in [
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
            Method::HEAD,
            Method::OPTIONS,
            Method::CONNECT,
            Method::TRACE,
        ] {
            assert_eq!(method_label(&m).as_ref(), m.as_str());
            assert!(matches!(method_label(&m), Cow::Borrowed(_)));
        }
        // Non-standard method keeps its exact spelling via the owned fallback.
        let custom = Method::from_bytes(b"REPORT").unwrap();
        assert_eq!(method_label(&custom).as_ref(), "REPORT");
        assert!(matches!(method_label(&custom), Cow::Owned(_)));
    }

    #[test]
    fn status_label_matches_to_string() {
        for code in [200u16, 201, 204, 304, 400, 404, 409, 412, 422, 500, 503] {
            assert_eq!(status_label(code).as_ref(), code.to_string());
            assert!(matches!(status_label(code), Cow::Borrowed(_)));
        }
        // Uncommon code falls back to an owned render, identical to to_string().
        assert_eq!(status_label(418).as_ref(), "418");
        assert!(matches!(status_label(418), Cow::Owned(_)));
    }
}
