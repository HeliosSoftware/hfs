//! Covers the zero-work fast path of [`helios_observability::middleware::track`].
//!
//! Own test binary so it can force the `off` arm before the process-global
//! `mode()` caches. Under `off` every instrumentation flag is false, so `track`
//! takes the early return that forwards the request without touching the
//! `MatchedPath` extension, headers, or any allocation — the floor arm.

use axum::{Router, body::Body, http::Request, routing::get};
use tower::ServiceExt;

#[tokio::test]
async fn track_does_no_work_under_off_arm() {
    // SAFETY: set before any other thread in this single-test binary reads the
    // env; `mode()` caches on first use, which happens inside the request below.
    unsafe {
        std::env::set_var("HELIOS_OBS_MODE", "off");
    }

    let app = Router::new()
        .route("/ping/{id}", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn(
            helios_observability::middleware::track,
        ));

    // Still forwards to the handler — the request just carries no instrumentation.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ping/2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
}
