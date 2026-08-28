//! Route-enumerator + fixture matrix for the HTS UI, matching the
//! "shared guards extended to /ui/hts" charter of `edson/docs/hts-ui-design.md`
//! §12 Phase 1 acceptance.
//!
//! The purpose is not to duplicate per-handler tests but to catch the class
//! of regressions where a page is added, its route is registered, but the
//! integration ring never notices — so 5xx / template-render errors / missing
//! Fluent keys can ship undetected. Every Phase 2 slice adds one entry to
//! [`ROUTES`] with a lightweight assertion; the matrix in [`FIXTURES`] is the
//! same locale × HX-Request combinations we walk in `crates/ui`'s
//! `router_http.rs`, so the shape stays consistent across products.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

fn app() -> Router {
    // 100 ms connect + 250 ms request keep the matrix under 3 s wallclock:
    // Windows' localhost RST is technically immediate but reqwest's client
    // stack can burn the default 2 s connect_timeout per iteration when a
    // handful of proxy/dns lookups collude on top of it, and the matrix
    // multiplies that by 30. Production uses [`UpstreamClient::new`]; only
    // tests take this fast path.
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

/// One row per registered `/ui/hts/*` page. The `expect` fragment is a
/// substring the response body must contain in the `en` locale, chosen to
/// fail loudly when the template fails to render or the Fluent key is
/// missing. Phase 2 slices append new rows here as pages land.
struct Route {
    path: &'static str,
    /// A substring that must appear in the `en`, no-htmx rendering. Kept
    /// short and specific to the page shell (title / scaffold heading),
    /// never a translated string that a locale rebase could shift.
    expect: &'static str,
}

const ROUTES: &[Route] = &[
    Route {
        path: "/ui/hts",
        // Slice A: the Home page renders live cards. The page `<h1>` is the
        // most stable marker (an inaccessible upstream still renders the
        // shell). The Fluent stub for `hts-nav-home` in en is exactly
        // "Home" — the same key backs both the sidebar label and the h1,
        // mirroring HFS's `nav-home` collapse.
        expect: ">Home<",
    },
    Route {
        // Slice A refresh fragment endpoint. Not a "page", but registered
        // and worth guarding: an accidental removal of the polling target
        // would blank the Home cards on JS clients without any test noticing.
        path: "/ui/hts/home/cards",
        expect: "hts-home-cards",
    },
    // Slice B: CodeSystem browser + rows fragment. The workbench tab
    // routes deliberately live in `tests/code_systems.rs` instead of
    // this matrix because each carries a real upstream `read_code_system`
    // call — walking them through locale × HX-Request combinations turns
    // the matrix from ~10 requests into ~50, which noticeably slows the
    // Windows link-and-run cycle without adding coverage the CS test
    // ring does not already provide.
    Route {
        path: "/ui/hts/code-systems",
        // Fluent en value for `hts-cs-browser-title` is "CodeSystems".
        expect: ">CodeSystems<",
    },
    Route {
        // Rows fragment; the browser table body's id is stable across
        // states (empty / results / degraded), so it is the ring's oracle.
        path: "/ui/hts/code-systems/rows",
        expect: "hts-cs-rows",
    },
    Route {
        // Detail page for a known-missing id: upstream is a closed
        // loopback port in tests, so the handler renders the shared
        // degraded banner + the outer section wrapper. Marker is chosen
        // to be present in every state (loaded / degraded / outcome).
        //
        // §8.3: the naked `/{id}` URL 308-redirects to the default
        // operation tab; the matrix walks `/{id}/lookup` directly (the
        // effective landing) so the shell-marker assertion still fires.
        // The redirect itself is asserted in `tests/code_systems.rs`.
        path: "/ui/hts/code-systems/does-not-exist/lookup",
        expect: "hts-cs-detail",
    },
    // Slice C: ValueSet browser + rows fragment + detail page. The
    // Expand tab (GET /{id}/expand) and its POST live in
    // `tests/value_sets.rs` — walking each through the locale ×
    // HX-Request combinations doubles matrix requests without adding
    // coverage the VS ring does not already provide (mirrors the CS
    // scope decision in §7.3.1).
    Route {
        path: "/ui/hts/value-sets",
        // Fluent en value for `hts-vs-browser-title` is "ValueSets".
        expect: ">ValueSets<",
    },
    Route {
        path: "/ui/hts/value-sets/rows",
        expect: "hts-vs-rows",
    },
    Route {
        // §8.3: naked `/{id}` 308-redirects to `/expand`; matrix walks
        // the effective landing directly. Redirect asserted in
        // `tests/value_sets.rs::detail_base_url_redirects_to_expand`.
        path: "/ui/hts/value-sets/does-not-exist/expand",
        expect: "hts-vs-detail",
    },
    // Slice D: ConceptMap browser + rows fragment + detail page. The
    // Translate tab (GET /{id}/translate and its POST) lives in
    // `tests/concept_maps.rs` for the same reason Slice B/C keep their
    // workbench routes there: each carries an outbound HTS round-trip
    // and would inflate the matrix from ~10 requests into ~50 without
    // adding coverage the CM ring does not already provide.
    Route {
        path: "/ui/hts/concept-maps",
        // Fluent en value for `hts-cm-browser-title` is "ConceptMaps".
        expect: ">ConceptMaps<",
    },
    Route {
        path: "/ui/hts/concept-maps/rows",
        expect: "hts-cm-rows",
    },
    Route {
        // §8.3: naked `/{id}` 308-redirects to `/translate`; matrix walks
        // the effective landing directly. Redirect asserted in
        // `tests/concept_maps.rs::detail_base_url_redirects_to_translate`.
        path: "/ui/hts/concept-maps/does-not-exist/translate",
        expect: "hts-cm-detail",
    },
    // Slice F: standalone Import page. The shell renders a real
    // `<form>` (nojs contract) with a paste-mode textarea; the H1
    // string is the stable marker across states (loaded / degraded).
    // No `/ui/hts/import/*` subroutes: F ships one route pair (GET +
    // POST) on the same path, so this single entry covers the shell
    // walk; the POST arm is exercised by `tests/import.rs`.
    Route {
        path: "/ui/hts/import",
        // Fluent en value for `hts-import-heading` is "Import terminology".
        expect: ">Import terminology<",
    },
    // Capability & Conformance. One entry — the page mirrors HFS's own
    // capability page (stacked `.card` sections, no tab strip), so there is
    // no fragment endpoint. It was called "Diagnostics" at
    // `/ui/hts/diagnostics` until 2026-08-27; that path now 308s here and
    // is therefore **not** in this matrix, which asserts 200 on every row.
    // The redirect is covered in `tests/capability.rs`. Per-source failure
    // isolation lives there too.
    Route {
        path: "/ui/hts/capability-statement",
        // Fluent en value for `cap-title` — HFS's own key, shared via the
        // workspace catalog. It is on the H1, so it survives any state
        // (loaded / degraded); the sidebar carries the different
        // `nav-capability-conformance` label, exactly as HFS does.
        expect: ">Capability Statement<",
    },
];

/// Locales the fixture matrix walks. Matches the switcher order in
/// [`crates/hts-ui/src/i18n.rs`] and the workspace catalog set.
const LOCALES: &[&str] = &["en", "es", "de"];

/// HX-Request booleans the matrix walks. The Phase 1 dashboard renders the
/// same body in both modes (full-page + fragment collapse to identical
/// content). Phase 2 slices with distinct fragment shapes will need per-page
/// oracles; this test only insists on 200 + `Vary: HX-Request` for now.
const HX_MODES: &[bool] = &[false, true];

#[tokio::test]
async fn every_registered_route_walks_the_locale_hx_matrix_and_en_body_marker() {
    // Merged from two prior siblings (matrix + shell-marker walk) to keep
    // the whole enumerator inside a single `#[tokio::test]`. Two separate
    // `#[tokio::test]` functions each create their own tokio runtime, drop
    // it, then the next test's runtime starts — under that sequence,
    // reqwest's connection pool cleanup can leak a Windows socket handle
    // into the next test process, which aborts with STATUS_INVALID_HANDLE
    // (0xFFFFFFFF) before its first request completes. Merging the walks
    // sidesteps the drop-then-reinit dance and keeps the ring green
    // regardless of test ordering.
    //
    // The router (and its UpstreamClient) is hoisted once so the reqwest
    // client is built exactly once for the whole matrix.
    let router = app();
    for route in ROUTES {
        for &locale in LOCALES {
            for &hx in HX_MODES {
                let mut builder = Request::get(route.path)
                    .header(header::ACCEPT_LANGUAGE, format!("{locale};q=1.0, en;q=0.1"));
                if hx {
                    builder = builder.header("HX-Request", "true");
                }
                let response = router
                    .clone()
                    .oneshot(builder.body(Body::empty()).unwrap())
                    .await
                    .unwrap();

                assert_eq!(
                    response.status(),
                    StatusCode::OK,
                    "route `{}` failed in locale `{locale}` (hx-request={hx})",
                    route.path,
                );

                // `AutoVaryLayer` (axum-htmx) only appends `Vary: HX-Request`
                // to responses whose request carried the `HX-Request` header
                // — that's the layer's contract, and it means caches will
                // never cross a fragment with a full page. Assert it in that
                // arm; the hard-navigation arm is exercised by the response
                // status alone.
                if hx {
                    let vary: Vec<String> = response
                        .headers()
                        .get_all(header::VARY)
                        .iter()
                        .map(|v| v.to_str().unwrap_or_default().to_ascii_lowercase())
                        .collect();
                    assert!(
                        vary.iter().any(|v| v.contains("hx-request")),
                        "route `{}` must Vary on HX-Request for htmx requests (got: {vary:?})",
                        route.path,
                    );
                    continue;
                }

                // Shell-marker assertion runs only on the `en, no-hx` cell of
                // the matrix: the expected substring is authored in English
                // and asserted once per route without duplicating the
                // 30-request walk.
                if locale == "en" {
                    let html = body_text(response).await;
                    assert!(
                        html.contains(route.expect),
                        "route `{}` did not include the expected shell marker `{}` in its `en` rendering",
                        route.path,
                        route.expect,
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn unknown_route_under_ui_hts_returns_404() {
    // Guard: the router only owns paths it explicitly declares. Any URL
    // under `/ui/hts` that is not registered must 404, not fall through to
    // the dashboard or crash the process. Phase 2 slices that add real
    // routes turn this test into a spec: unknown-under-parent stays 404.
    //
    // Kept as its own `#[tokio::test]` — it never enters the loopback
    // path, so it does not participate in the reqwest-cross-test
    // poisoning that forced the matrix + body-marker merge above.
    let response = app()
        .oneshot(
            Request::get("/ui/hts/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "unregistered /ui/hts/* paths must 404",
    );
}
