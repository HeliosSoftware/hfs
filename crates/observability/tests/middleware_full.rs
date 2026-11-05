//! Covers the span-enabled path of [`helios_observability::middleware::track`].
//!
//! The observability arm is a process-global `OnceLock` over `HELIOS_OBS_MODE`,
//! so this lives in its own test binary and forces the `full` arm before any
//! request is served. Under `full`, `span_enabled` is true regardless of
//! whether a trace exporter is installed, so the request opens the tracing span
//! and records the status onto it — the branch the default-arm test cannot reach.

use axum::{Router, body::Body, http::Request, routing::get};
use tower::ServiceExt;

#[tokio::test]
async fn track_opens_span_under_full_arm() {
    // SAFETY: set before any other thread in this single-test binary reads the
    // env; `mode()` caches on first use, which happens inside the request below.
    unsafe {
        std::env::set_var("HELIOS_OBS_MODE", "full");
    }

    let app = Router::new()
        .route("/ping/{id}", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn(
            helios_observability::middleware::track,
        ));

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ping/1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
}
