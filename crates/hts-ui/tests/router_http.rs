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
/// `/ui`, so URLs resolve as `/ui/hts/...` exactly as they do in the
/// deployed binary. Pointed at a
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
        metrics_ring: Default::default(),
    });
    Router::new().nest("/ui", helios_hts_ui::router(state))
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Body text with escaped ampersands folded back to `&`.
///
/// Askama escapes `&` in attribute values, and *which* escape it emits is an
/// implementation detail of the escaper (`&#38;` today, `&amp;` in other
/// versions). Assertions about multi-parameter URLs should pin the URL, not
/// the escaper, so they run against the decoded text.
async fn body_text_urls(response: axum::response::Response) -> String {
    body_text(response)
        .await
        .replace("&#38;", "&")
        .replace("&amp;", "&")
}

#[tokio::test]
async fn home_serves_full_page_at_ui_hts() {
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
        "hts-nav-home",
        "hts-nav-code-systems",
        "hts-nav-value-sets",
        "hts-nav-concept-maps",
        "hts-nav-operations",
        "hts-nav-import",
        "hts-nav-diagnostics",
        "hts-home-title",
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
async fn home_trailing_slash_redirects_to_canonical() {
    // `/ui/hts/` (trailing slash) must 308-redirect to the canonical
    // `/ui/hts`. Axum matches paths exactly, so without an explicit route
    // the trailing-slash variant would 404. Locked here so any regression
    // fails the ring.
    let response = app()
        .oneshot(Request::get("/ui/hts/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::PERMANENT_REDIRECT,
        "GET /ui/hts/ must 308-redirect to the canonical path",
    );
    let location = response
        .headers()
        .get(header::LOCATION)
        .expect("308 must carry a Location header")
        .to_str()
        .expect("Location header must be ASCII");
    assert_eq!(
        location, "/ui/hts",
        "trailing-slash redirect must point at the browser-canonical /ui/hts",
    );
}

#[tokio::test]
async fn home_advertises_vary_hx_request_for_htmx_caching() {
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
async fn home_localizes_via_accept_language_when_no_query_or_cookie() {
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
    // The Spanish stub for hts-nav-home is "Inicio" — the collapsed key
    // that backs both the sidebar label and the h1 (HFS `nav-home` parity).
    assert!(
        html.contains("Inicio"),
        "Spanish translation of hts-nav-home must appear in the sidebar",
    );
}

// ── Home request-rate chart (design doc §7.1) ─────────────────────────

#[tokio::test]
async fn home_cards_fragment_renders_the_chart_for_a_selected_window_and_series() {
    // The fragment must honour `?window=&series=` — that is the whole
    // reason the poll lives on `#hts-home-cards` and carries the query.
    let response = app()
        .oneshot(
            Request::get("/ui/hts/home/cards?window=1h&series=4xx")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text_urls(response).await;

    // The chart frame renders even with nothing to plot: the upstream here
    // is a closed port, so this also pins the degraded arm.
    assert!(
        html.contains("class=\"chart\""),
        "the chart <svg> must render even when there is nothing to plot",
    );
    assert!(
        html.contains("viewBox=\"0 0 1060 300\""),
        "the chart must share HFS's coordinate space verbatim",
    );

    // Exactly one `hx-get`, and it is the region's own self-refresh. Two
    // would mean the old never-swapped wrapper came back, which would
    // revert the selection on the next 15 s tick.
    assert_eq!(
        html.matches("hx-get=").count(),
        1,
        "#hts-home-cards must carry exactly one hx-get (its own self-refresh)",
    );
    assert!(
        html.contains("hx-get=\"/ui/hts/home/cards?window=1h&series=4xx\""),
        "the self-refresh URL must carry the current selection so it survives the poll",
    );
    assert!(
        !html.contains("hx-target="),
        "the poll must have no hx-target — htmx defaults to the element, which is \
         what makes outerHTML replace the refreshed hx-get too",
    );

    // The selection is reflected in the controls.
    assert!(
        html.contains("window-picker__option--active"),
        "the active window must be marked in the picker",
    );
    assert!(
        html.contains("chart-legend__item--active"),
        "the active status class must be marked in the legend",
    );

    // No dead controls copied over from HFS's richer chart.
    assert!(
        !html.contains("pill--square"),
        "the expand pill has no HTS analogue and must not render",
    );
    assert!(
        !html.contains("chart-legend__type"),
        "chart-legend__type has no rule in app.css and must not render",
    );

    // Every chart string resolved through Fluent.
    for key in [
        "hts-home-chart-title",
        "hts-home-chart-window",
        "hts-home-chart-series",
        "hts-home-chart-window-1h",
        "hts-home-chart-series-4xx",
        "hts-home-chart-empty-unreachable",
    ] {
        assert!(
            !html.contains(key),
            "raw catalog key `{key}` leaked into the chart card",
        );
    }
}

#[tokio::test]
async fn home_page_carries_the_chart_selection_into_its_self_refresh() {
    // A window/legend click is a plain navigation back to `/ui/hts`; the
    // page it returns must hand the selection straight to the poll.
    let response = app()
        .oneshot(
            Request::get("/ui/hts?window=6h&series=5xx")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text_urls(response).await;
    assert!(
        html.contains("hx-get=\"/ui/hts/home/cards?window=6h&series=5xx\""),
        "the Home page's cards region must poll for the selected window and series",
    );
    assert_eq!(
        html.matches("hx-get=").count(),
        1,
        "the old never-swapped wrapper must stay gone from the page shell",
    );
    // Window and legend options are real links, so the chart works with
    // JavaScript disabled.
    assert!(
        html.contains("href=\"/ui/hts?window=15m&series=5xx\""),
        "window options must be plain <a href> links that preserve the series",
    );
    assert!(
        html.contains("href=\"/ui/hts?window=6h&series=2xx\""),
        "legend options must be plain <a href> links that preserve the window",
    );
}

#[tokio::test]
async fn an_unknown_chart_selection_falls_back_instead_of_erroring() {
    // A stale bookmark or a hand-edited URL must still render a chart.
    let response = app()
        .oneshot(
            Request::get("/ui/hts/home/cards?window=99y&series=nonsense")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text_urls(response).await;
    assert!(
        html.contains("hx-get=\"/ui/hts/home/cards?window=15m&series=all\""),
        "unknown values must fall back to the defaults, not 400",
    );
}

/// Mount with a handle on the state, so a test can seed the chart's sample
/// ring before rendering. Same closed-loopback upstream as [`app`].
fn app_with_state() -> (Router, Arc<helios_hts_ui::HtsUiState>) {
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
        metrics_ring: Default::default(),
    });
    (
        Router::new().nest("/ui", helios_hts_ui::router(state.clone())),
        state,
    )
}

#[tokio::test]
async fn a_populated_ring_renders_one_polyline_per_observed_run() {
    // The end-to-end shape the unit tests cannot see: real coordinates in
    // the markup, and a gap that becomes a second <polyline> rather than a
    // straight line bridging the unobserved stretch.
    let (app, state) = app_with_state();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let counts = |all: u64| helios_hts_ui::StatusCounts {
        all,
        s2xx: all,
        s4xx: 0,
        s5xx: 0,
    };
    // Two observed runs, 20 minutes apart, inside the 1 h window.
    for (offset, total) in [
        (2_400.0, 0u64),
        (2_385.0, 10),
        (2_370.0, 25),
        (300.0, 9_000), // resumed after ~35 unobserved minutes
        (285.0, 9_020),
        (270.0, 9_035),
    ] {
        state.metrics_ring.push(helios_hts_ui::MetricsSample {
            at_secs: now - offset,
            uptime_secs: 100_000.0 - offset,
            counts: counts(total),
        });
    }

    let response = app
        .oneshot(
            Request::get("/ui/hts/home/cards?window=1h&series=all")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text_urls(response).await;

    assert_eq!(
        html.matches("<polyline").count(),
        2,
        "the unobserved stretch must break the series into two polylines, \
         never one line drawn straight across it",
    );
    // Real geometry, not an empty points list.
    assert!(
        !html.contains("points=\"\""),
        "every rendered polyline must carry coordinates",
    );
    // The upstream here is a closed port, so *this* scrape failed even
    // though the seeded history is intact. The card states the live fact —
    // /metrics is unreachable — while still drawing what was genuinely
    // observed. That pairing is the honest one: the history is not erased,
    // and the fact that it is now stale is not hidden.
    assert!(
        html.contains("unreachable"),
        "an unreachable /metrics must be stated in the card's .stat__sub",
    );
    assert!(
        !html.contains("No samples collected yet"),
        "\"no samples\" must not render next to a plotted line",
    );
    // The line and its legend dot share the palette slot for the All series.
    assert!(html.contains("class=\"series series--1\""));
    assert!(html.contains("chart-legend__dot chart-legend__dot--1"));
}

#[tokio::test]
async fn an_upstream_restart_clears_the_ring_instead_of_plotting_a_cliff() {
    // `uptime_seconds` going backwards means the counters restarted from
    // zero. Differencing across that boundary would render a plunge to 0 —
    // "traffic stopped" — which is false. The ring drops the old samples and
    // the chart reports that it is collecting again.
    let (app, state) = app_with_state();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let counts = |all: u64| helios_hts_ui::StatusCounts {
        all,
        s2xx: all,
        s4xx: 0,
        s5xx: 0,
    };
    for (offset, uptime, total) in [
        (60.0, 50_000.0, 900_000u64),
        (45.0, 50_015.0, 900_100),
        (30.0, 3.0, 4), // restart
    ] {
        state.metrics_ring.push(helios_hts_ui::MetricsSample {
            at_secs: now - offset,
            uptime_secs: uptime,
            counts: counts(total),
        });
    }

    let response = app
        .oneshot(
            Request::get("/ui/hts/home/cards?window=15m&series=all")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text_urls(response).await;
    assert_eq!(
        html.matches("<polyline").count(),
        0,
        "the only surviving sample is the post-restart one, so nothing is \
         plottable — and certainly no cliff",
    );
    assert!(
        html.contains("unreachable"),
        "the card must state the live condition rather than draw a drop to zero",
    );
    // Nothing from the pre-restart process survived into the legend totals
    // either — 900 100 requests must not be attributed to this window.
    assert!(
        !html.contains(">900"),
        "pre-restart counters must not leak into the window totals",
    );
}
