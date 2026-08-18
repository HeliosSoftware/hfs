//! CodeSystem browser + detail + workbench HTTP tests (Slice B).
//!
//! Companion to `router_http.rs`. The test app points upstream at a closed
//! loopback port, so `search_code_systems` / `read_code_system` reliably
//! surface `UpstreamError::Connect`; the handlers must degrade gracefully
//! (banner + empty table on the browser, banner + explanatory shell on the
//! detail page) rather than 5xx. The htmx-aware `Vary: HX-Request` header
//! and pre-flight `_count > MAX` rejection are the two other invariants
//! specific to this slice.

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

fn app() -> Router {
    // Short timeouts so the whole ring finishes in a couple of seconds even
    // when handlers make 1–2 upstream calls each — see `route_enum.rs` for
    // the reqwest-on-Windows rationale.
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
async fn browser_renders_full_page_with_translated_heading() {
    let response = app()
        .oneshot(
            Request::get("/ui/hts/code-systems")
                .header(header::ACCEPT_LANGUAGE, "en")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains("<!doctype html>"),
        "hard nav must render a full HTML page",
    );
    assert!(
        html.contains(">CodeSystems<"),
        "browser heading must be Fluent-resolved (en value: CodeSystems)",
    );
    assert!(
        html.contains("hts-cs-browser__filters"),
        "filter form must render",
    );
    for key in [
        "hts-cs-browser-title",
        "hts-cs-browser-filter-search",
        "hts-cs-browser-column-url",
        "hts-cs-browser-load-more",
        "hts-workbench-run",
    ] {
        assert!(
            !html.contains(key),
            "raw catalog key `{key}` leaked (missing Fluent value?)",
        );
    }
    assert!(
        html.contains("Terminology backend not fully available"),
        "closed-loopback upstream must render the degraded banner (en)",
    );
}

#[tokio::test]
async fn browser_rows_fragment_vary_on_htmx_request() {
    let response = app()
        .oneshot(
            Request::get("/ui/hts/code-systems/rows")
                .header("HX-Request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let vary: Vec<String> = response
        .headers()
        .get_all(header::VARY)
        .iter()
        .map(|v| v.to_str().unwrap_or_default().to_ascii_lowercase())
        .collect();
    assert!(
        vary.iter().any(|v| v.contains("hx-request")),
        "rows fragment must add HX-Request to Vary; got: {vary:?}",
    );
    let html = body_text(response).await;
    assert!(
        html.contains("hts-cs-rows"),
        "rows fragment must render its stable outer id (found: {})",
        &html[..html.len().min(300)],
    );
}

#[tokio::test]
async fn browser_over_max_count_renders_invalid_input_outcome() {
    // Design decision: rather than reject with 400 (which would break the
    // Load-more affordance and the debounced filter form), the handler
    // renders an invalid-input OperationOutcome above an empty table with
    // the filters echoed back. This test pins that contract — the
    // response is 200 with the outcome partial's severity marker.
    let response = app()
        .oneshot(
            Request::get("/ui/hts/code-systems?_count=200")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("hts-outcome hts-outcome--error"),
        "over-max _count must render the outcome partial in error severity",
    );
}

#[tokio::test]
async fn browser_rejects_over_max_count_partial_shape_too() {
    // Same guarantee, but exercised through the rows-fragment path — the
    // partial swap on the filter form must also render the outcome.
    let response = app()
        .oneshot(
            Request::get("/ui/hts/code-systems/rows?_count=999")
                .header("HX-Request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("hts-outcome"),
        "rows fragment over-max _count must render the outcome partial",
    );
}

#[tokio::test]
async fn detail_renders_shell_and_outcome_on_upstream_failure() {
    // HTS is a closed loopback port here: read_code_system returns
    // `Connect`, which the detail handler translates into the degraded
    // banner. The important guarantee is that the request completes 200
    // with a full HTML page — not a 5xx or blank body — so operators can
    // read the banner and retry once HTS returns.
    let response = app()
        .oneshot(
            Request::get("/ui/hts/code-systems/example-system")
                .header(header::ACCEPT_LANGUAGE, "en")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("<!doctype html>"),
        "detail hard nav must render a full HTML page",
    );
    assert!(
        html.contains("Terminology backend not fully available"),
        "detail must render the degraded banner when upstream is unreachable",
    );
    assert!(
        html.contains("hts-cs-detail"),
        "detail scaffold section id must be present regardless of load result",
    );
}

#[tokio::test]
async fn detail_soft_deleted_would_render_outcome_not_page_404() {
    // Documented behavior contract (§7.3 states matrix): HTS returns 404
    // for both truly-missing and soft-deleted resources, and the UI
    // cannot tell them apart at the HTTP layer. The detail handler
    // therefore renders an OperationOutcome inside the page shell rather
    // than propagating an HTTP 404 to the browser. This test uses the
    // closed-loopback fixture, where the failure mode is `Connect` +
    // degraded banner; the parallel test in a wiremock ring (deferred to
    // the follow-up integration slice per docs update) covers the 404 →
    // outcome path directly. Either way, the response status stays 200.
    let response = app()
        .oneshot(
            Request::get("/ui/hts/code-systems/definitely-soft-deleted")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the detail page must never surface a page 404; the outcome/banner\
         partial is the operator-visible signal",
    );
}

#[tokio::test]
async fn lookup_input_hx_renders_input_partial_only() {
    let response = app()
        .oneshot(
            Request::get("/ui/hts/code-systems/example-system/lookup")
                .header("HX-Request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("hts-cs-workbench__input"),
        "htmx tab load must return only the workbench input partial",
    );
    assert!(
        !html.contains("<!doctype html>"),
        "htmx tab load must not include the full page shell",
    );
}

#[tokio::test]
async fn lookup_run_without_code_renders_invalid_input_outcome() {
    // The workbench pre-flight rejects a missing `code` locally so we
    // don't burn an HTS round-trip on invalid input; the outcome partial
    // is the operator-visible signal. Also covers the POST verb rule
    // from §7.6 — every operation proxy is POST.
    let response = app()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/code-systems/example-system/lookup")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(""))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("hts-outcome hts-outcome--error"),
        "empty POST must surface an invalid-input outcome",
    );
    assert!(
        html.contains("hts-workbench-result"),
        "the outcome must render inside the shared workbench-result panel",
    );
}
