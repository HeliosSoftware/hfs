//! End-to-end HTTP tests over the mounted HTS UI router.
//!
//! These use `tower::ServiceExt::oneshot` to issue the same requests a
//! browser would make and assert the shape a Phase 1 blocker scaffold must
//! satisfy: a routed dashboard, an HTMX-safe `Vary` layer, and a served
//! embedded asset bundle. Route coverage grows with each Phase 2 slice.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

/// Test-only mount: identical to how the `hts` binary mounts the router at
/// `/ui`, so URLs match `edson/docs/hts-ui-design.md` §5.1. Pointed at a
/// closed loopback port so upstream fetches fail deterministically — the
/// dashboard renders the degraded banner alongside every card, which is
/// exactly what the Phase 1 shell-blocker test expects.
///
/// Short timeouts keep the ring under a couple of seconds on Windows,
/// where reqwest's default 2 s connect_timeout fires against a closed
/// loopback port instead of returning WSAECONNREFUSED immediately.
fn app() -> Router {
    let state = Arc::new(helios_hts_ui::HtsUiState {
        fhir_version: "R4",
        version: "9.9.9-test",
        upstream: helios_hts_ui::UpstreamClient::new_with_timeouts(
            "http://127.0.0.1:1",
            Duration::from_millis(250),
            Duration::from_millis(100),
        )
        .expect("closed loopback URL always parses"),
        bundled_data_bytes: None,
    });
    Router::new().nest("/ui", helios_hts_ui::router(state))
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn dashboard_serves_full_page_at_ui_hts() {
    let response = app()
        .oneshot(Request::get("/ui/hts").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("<!doctype html>"), "response must be a full HTML page");
    assert!(
        html.contains("9.9.9-test"),
        "sidebar must render the version we mounted with"
    );
    // Every catalog key must have been resolved to translated prose. A
    // stray key text would signal either a missing Fluent entry or a
    // template that renders the key literally.
    for key in [
        "hts-nav-dashboard",
        "hts-nav-code-systems",
        "hts-nav-value-sets",
        "hts-nav-concept-maps",
        "hts-nav-operations",
        "hts-nav-import",
        "hts-nav-diagnostics",
        "hts-dashboard-title",
        "hts-dialect-prefix",
        "hts-degraded-title",
    ] {
        assert!(
            !html.contains(key),
            "raw catalog key `{key}` leaked into the response (missing Fluent translation?)",
        );
    }
    // Degraded banner must render because the upstream is a closed port —
    // Slice A's guaranteed marker on the cards region.
    assert!(
        html.contains("Terminology backend not fully available"),
        "degraded banner (en) must render when upstream is unreachable",
    );
}

#[tokio::test]
async fn dashboard_advertises_vary_hx_request_for_htmx_caching() {
    // `AutoVaryLayer` (axum-htmx) appends `Vary: HX-Request` only when the
    // request carried the `HX-Request` header — that's what makes it safe
    // for shared caches: a hard navigation and an htmx swap of the same URL
    // never share a cache line.
    let response = app()
        .oneshot(
            Request::get("/ui/hts")
                .header("HX-Request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let vary: Vec<String> = response
        .headers()
        .get_all(header::VARY)
        .iter()
        .map(|v| v.to_str().unwrap_or_default().to_ascii_lowercase())
        .collect();
    assert!(
        vary.iter().any(|v| v.contains("hx-request")),
        "AutoVaryLayer must add HX-Request to Vary on htmx requests; got: {vary:?}",
    );
}

#[tokio::test]
async fn assets_serve_the_embedded_bundle_under_ui_hts_assets() {
    let response = app()
        .oneshot(
            Request::get("/ui/hts/assets/htmx.min.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the embedded htmx bundle must be served under /ui/hts/assets/*"
    );
    let ctype = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap_or_default().to_ascii_lowercase())
        .unwrap_or_default();
    assert!(
        ctype.starts_with("application/javascript") || ctype.starts_with("text/javascript"),
        "unexpected content-type for htmx.min.js: {ctype:?}",
    );
}

#[tokio::test]
async fn dashboard_localizes_via_accept_language_when_no_query_or_cookie() {
    // Spanish request: the sidebar heading must be translated. If the Fluent
    // catalog is not wired the key would leak; if the locale negotiator is not
    // installed the English string would leak — both are actionable failures.
    let response = app()
        .oneshot(
            Request::get("/ui/hts")
                .header(header::ACCEPT_LANGUAGE, "es-ES, es;q=0.9, en;q=0.5")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("<html lang=\"es\">"),
        "the html lang attribute must reflect the negotiated locale",
    );
    // The Spanish stub for hts-nav-dashboard.
    assert!(
        html.contains("Panel"),
        "Spanish translation of hts-nav-dashboard must appear in the sidebar",
    );
}
