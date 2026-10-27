//! End-to-end coverage for [`helios_observability::middleware::track`].
//!
//! The middleware's decision table (which instrumentation runs per request) is
//! unit-tested on [`helios_observability::mode::ObsMode`]; this drives the async
//! body itself — path/tenant extraction, the metrics block, and the reqlog push
//! — through a real axum router.
//!
//! The observability arm is process-global (`OnceLock` over `HELIOS_OBS_MODE`),
//! so this integration binary can only observe one arm. It runs under the
//! default arm (env unset): metrics on, span off (no OTLP exporter installed),
//! reqlog gated on a registered consumer — which lets one test cover the
//! consumer-off path and another the consumer-on path within the same process.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::get,
};
use helios_observability::{middleware::track, reqlog};
use tower::ServiceExt; // for `oneshot`

fn app() -> Router {
    Router::new()
        .route("/ping/{id}", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn(track))
}

async fn call(uri: &str, tenant: Option<&str>) -> StatusCode {
    let mut builder = Request::builder().uri(uri);
    if let Some(t) = tenant {
        builder = builder.header("x-tenant-id", t);
    }
    app()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn track_passes_request_through_and_records_metrics() {
    // Default arm: the middleware runs its body (metrics are on) and forwards
    // the request to the handler. No metrics recorder is installed, so the
    // `metrics::` macros resolve to the global no-op recorder — the label-
    // building code still executes, which is what we want covered.
    assert_eq!(call("/ping/42", None).await, StatusCode::OK);
}

#[tokio::test]
async fn track_feeds_reqlog_once_a_consumer_is_registered() {
    // Before enabling, the default arm leaves reqlog off; after `enable()` the
    // middleware takes the tenant-extraction + reqlog-push path.
    reqlog::enable();
    assert!(reqlog::enabled());

    assert_eq!(call("/ping/7", Some("obs-mw-tenant")).await, StatusCode::OK);

    // The push should be visible in the ring buffer under the tenant we sent.
    let stats = reqlog::snapshot(3600, Some("obs-mw-tenant"));
    assert!(
        stats.sample_count >= 1,
        "an enabled reqlog must capture the request"
    );
}
