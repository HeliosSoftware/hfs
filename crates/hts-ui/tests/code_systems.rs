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
        metrics_ring: Default::default(),
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
        html.contains("id=\"hts-cs-filters\""),
        "filter form must render (stable id anchor for tests)",
    );
    for key in [
        "hts-cs-browser-title",
        "hts-cs-browser-filter-reset",
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
        html.contains(r#"data-severity="error""#),
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
        html.contains("notice--warn"),
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
    //
    // §8.3: the naked `/{id}` URL now 308-redirects to `/{id}/lookup`,
    // so this test hits the effective landing directly. The redirect
    // itself is covered by `detail_base_url_redirects_to_lookup` below.
    let response = app()
        .oneshot(
            Request::get("/ui/hts/code-systems/example-system/lookup")
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
async fn detail_base_url_redirects_to_lookup() {
    // §8.3 operation-first landing: the naked `/ui/hts/code-systems/{id}`
    // URL 308-redirects to the default operation tab (`/{id}/lookup`).
    // This keeps the browser URL and the `aria-current` tab always in
    // sync — the workbench never renders at a URL that doesn't name the
    // active operation.
    let response = app()
        .oneshot(
            Request::get("/ui/hts/code-systems/example-system")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/ui/hts/code-systems/example-system/lookup"),
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
    //
    // §8.3: the request targets the default landing tab directly
    // (`/{id}/lookup`) rather than the naked `/{id}` URL, since the
    // latter now 308-redirects (see `detail_base_url_redirects_to_lookup`).
    let response = app()
        .oneshot(
            Request::get("/ui/hts/code-systems/definitely-soft-deleted/lookup")
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
async fn lookup_input_hx_returns_full_page_for_region_swap() {
    // Region-wrap contract (design doc §8.1): a tab-click GET now returns
    // the full detail page; htmx uses `hx-select="#hts-cs-detail-region"`
    // to pick the tabs+workbench region out of the response so the
    // aria-current attribute on the tab strip stays in sync with the
    // panel the operator sees. This test runs against the closed-loopback
    // upstream so read_code_system fails and the outcome banner renders
    // in place of the region — the important assertion is that the
    // response is a full HTML page (previously it was the input partial
    // only), which unlocks htmx to `hx-select` the region when the read
    // succeeds against a real HTS.
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
        html.contains("<!doctype html>") || html.contains("<!DOCTYPE html>"),
        "htmx tab load now returns the full page so htmx can hx-select the region",
    );
    // The degraded banner now wears the shared `.notice.notice--warn`
    // skin (the old `.hts-degraded*` hooks had no rule in app.css), so
    // match on that or on the outcome partial's own class.
    assert!(
        html.contains("notice--warn"),
        "closed-loopback upstream must surface the degraded banner (Connect) or outcome banner (NotFound) in place of the workbench",
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
        html.contains(r#"data-severity="error""#),
        "empty POST must surface an invalid-input outcome",
    );
    assert!(
        html.contains("hts-workbench-result"),
        "the outcome must render inside the shared workbench-result panel",
    );
}

// ── V2 "top strip" browser layout (#551 browser redesign) ───────────────
//
// The browser used to be a two-column `.filter-layout--two` with a sticky
// left `.filter-rail`. It is now a horizontal filter strip over a
// full-width table: a `.toolbar` of `.toolbar__search` inputs plus a
// `.facets.facets--bare` status chip row, inside one `.card.table-card`.
// These tests pin the shell, the chip semantics, and the fact that no
// invented CSS hook (`.hts-degraded`, `.btn--ghost`, `.content--wide`)
// creeps back in — every class on these pages must have a real rule in
// `crates/ui/assets/app.css`.

#[tokio::test]
async fn browser_renders_v2_top_strip_shell() {
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

    for hook in [
        r#"<body class="app-shell">"#,
        r#"class="content content--app""#,
        r#"class="card table-card""#,
        r#"class="toolbar""#,
        r#"class="toolbar__search""#,
        r#"class="facets facets--bare""#,
        r#"class="facet-label""#,
        r#"class="chip""#,
    ] {
        assert!(
            html.contains(hook),
            "V2 top-strip layout must render `{hook}`",
        );
    }
    // Retired / never-existing hooks. `.content--wide` was removed from
    // app.css by a recent merge; the rail grammar is gone by design.
    for dead in [
        "filter-rail",
        "filter-layout",
        "content--wide",
        "btn--ghost",
        "btn--secondary",
        r#"class="hts-degraded""#,
    ] {
        assert!(
            !html.contains(dead),
            "`{dead}` has no rule in app.css and must not be rendered",
        );
    }
}

#[tokio::test]
async fn browser_degraded_banner_uses_the_shared_notice_primitive() {
    // Closed loopback upstream => `UpstreamError::Connect` => banner.
    let response = app()
        .oneshot(
            Request::get("/ui/hts/code-systems")
                .header(header::ACCEPT_LANGUAGE, "en")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(
        html.contains(r#"<aside class="notice notice--warn""#),
        "the degraded banner must use the shared `.notice.notice--warn` skin",
    );
    assert!(
        html.contains("Terminology backend not fully available"),
        "and still carry its Fluent-resolved title",
    );
}

#[tokio::test]
async fn status_chips_mark_the_active_facet_and_carry_text_filters() {
    let response = app()
        .oneshot(
            Request::get("/ui/hts/code-systems?name=icd&status=retired")
                .header(header::ACCEPT_LANGUAGE, "en")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // The selected chip is the only `aria-current` one. (Askama escapes
    // `&` as the numeric entity `&#38;`, not `&amp;`.)
    assert!(
        html.contains(
            r#"href="/ui/hts/code-systems?name=icd&#38;status=retired" aria-current="true""#
        ),
        "the active status chip must carry aria-current=\"true\"",
    );
    assert_eq!(
        html.matches(r#"class="chip" href"#).count(),
        5,
        "five chips: any + draft/active/retired/unknown",
    );
    // ... and every other chip keeps the text filters so switching status
    // does not silently discard what the operator typed.
    assert!(
        html.contains(r#"href="/ui/hts/code-systems?name=icd""#),
        "the `any status` chip must drop only `status`, keeping `name`",
    );
    assert!(
        html.contains(r#"href="/ui/hts/code-systems?name=icd&#38;status=active""#),
        "the other chips must keep `name` alongside their own status",
    );
    // The chips are links, so the form needs the selection as a hidden
    // field or the debounced htmx swap would drop it.
    assert!(
        html.contains(r#"<input type="hidden" name="status" value="retired">"#),
        "the active status must ride along with the filter form",
    );
}

// ── Mock upstream: row-link encoding ────────────────────────────────────

/// Minimal in-process upstream that answers `GET /CodeSystem` with a
/// searchset Bundle. Only the browser's search leg is needed here, so
/// this is deliberately much smaller than the `concept_maps.rs` mock.
async fn start_search_mock() -> String {
    let router: Router = Router::new()
        .route(
            "/__mock_ready",
            axum::routing::get(|| async { (StatusCode::OK, "ok") }),
        )
        .route(
            "/CodeSystem",
            axum::routing::get(|| async {
                (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({
                        "resourceType": "Bundle",
                        "type": "searchset",
                        "entry": [
                            {
                                // HTS stores versioned CodeSystems under a
                                // composite `{fhir_id}|{version}` id (§8.2);
                                // `upstream::base_id` trims the version off
                                // before the row reaches the template.
                                "resource": {
                                    "resourceType": "CodeSystem",
                                    "id": "icd9cm|2015",
                                    "url": "http://hl7.org/fhir/sid/icd-9-cm",
                                    "version": "2015",
                                    "name": "ICD-9-CM",
                                    "title": "International Classification of Diseases, Ninth Revision",
                                    "status": "retired"
                                }
                            },
                            {
                                // An id that survives `base_id` intact and
                                // still needs escaping in a path segment.
                                "resource": {
                                    "resourceType": "CodeSystem",
                                    "id": "urn:oid:2.16.840.1.113883.6.238",
                                    "url": "urn:oid:2.16.840.1.113883.6.238",
                                    "name": "CDCREC",
                                    "status": "active"
                                }
                            }
                        ]
                    })),
                )
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock upstream listener");
    let addr: std::net::SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let base = format!("http://{addr}");
    let probe = reqwest::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .expect("build ready-probe client");
    let ready_url = format!("{base}/__mock_ready");
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match probe.get(&ready_url).send().await {
            Ok(r) if r.status().is_success() => break,
            _ => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
    base
}

fn app_pointing_at(base_url: &str) -> Router {
    let state = Arc::new(helios_hts_ui::HtsUiState {
        fhir_version: "R4",
        version: "9.9.9-test",
        upstream: helios_hts_ui::UpstreamClient::new_with_timeouts(
            base_url,
            Duration::from_secs(5),
            Duration::from_secs(2),
        )
        .expect("test upstream base URL parses"),
        bundled_data_bytes: None,
        metrics_ring: Default::default(),
    });
    Router::new().nest("/ui", helios_hts_ui::router(state))
}

#[tokio::test]
async fn row_link_percent_encodes_the_id_path_segment() {
    let base = start_search_mock().await;
    let response = app_pointing_at(&base)
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

    // `base_id` strips the `|version` suffix upstream, so the pipe never
    // reaches the href …
    assert!(
        html.contains(r#"href="/ui/hts/code-systems/icd9cm""#),
        "the composite id's version suffix is trimmed by `upstream::base_id`",
    );
    assert!(
        !html.contains("icd9cm|2015"),
        "a raw `|` in the path segment would not round-trip",
    );
    // … and whatever is left is percent-encoded, so an id carrying any
    // reserved character still produces a URL that decodes back to it.
    assert!(
        html.contains(r#"href="/ui/hts/code-systems/urn%3Aoid%3A2.16.840.1.113883.6.238""#),
        "the id path segment must be percent-encoded",
    );
    // Phase 5 column set survives the relayout: Name . Title . URL .
    // Version . Status, with the canonical URL in a `.url` span.
    assert!(
        html.contains(r#"<span class="url">http://hl7.org/fhir/sid/icd-9-cm</span>"#),
        "canonical URLs render in a `.url` span",
    );
    assert!(
        html.contains(r#"<span class="tag tag--retired">"#),
        "status renders as a `.tag--<status>` pill",
    );
}

// ── Static guard: the CS detail surface introduces no CSS of its own ────

/// Strip Askama comments (`{# … #}`) from a template source.
///
/// The V3 templates document their own class choices in prose — including
/// class names they deliberately do NOT use, like `.addbox` — so a naive
/// scan of the raw source would flag rules that are only ever quoted.
fn strip_template_comments(template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{#") {
        out.push_str(&rest[..start]);
        match rest[start..].find("#}") {
            Some(end) => rest = &rest[start + end + 2..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Every class name the CodeSystem detail templates use must already have
/// a rule in the shared `crates/ui/assets/app.css` (HTS-UI serves that
/// exact file — see the `Assets` embed in `src/lib.rs`). The V3 layout
/// pass ships zero new CSS, so a class with no rule is either a typo or a
/// reintroduced HTS-only style hook.
///
/// Interpolated values (`class="tag tag--{{ summary.status }}"`) are
/// skipped: the concrete status tokens are enumerated in app.css, but the
/// template source cannot be matched literally.
#[test]
fn cs_detail_templates_only_use_classes_that_exist_in_app_css() {
    const APP_CSS: &str = include_str!("../../ui/assets/app.css");
    let templates = [
        (
            "pages/cs-detail.html",
            include_str!("../templates/pages/cs-detail.html"),
        ),
        (
            "partials/hts-cs-lookup-input.html",
            include_str!("../templates/partials/hts-cs-lookup-input.html"),
        ),
        (
            "partials/hts-cs-validate-input.html",
            include_str!("../templates/partials/hts-cs-validate-input.html"),
        ),
        (
            "partials/hts-cs-subsumes-input.html",
            include_str!("../templates/partials/hts-cs-subsumes-input.html"),
        ),
        (
            "partials/hts-cs-workbench-result.html",
            include_str!("../templates/partials/hts-cs-workbench-result.html"),
        ),
        // Included by the workbench result and by cs-detail directly. It is
        // the one template allowed its own hooks (`.hts-outcome*`, #805) —
        // scanning it here is what keeps the rule and the class together.
        (
            "partials/hts-outcome.html",
            include_str!("../templates/partials/hts-outcome.html"),
        ),
    ];

    let mut checked = 0usize;
    for (name, template) in templates {
        let body = strip_template_comments(template);
        for chunk in body.split(r#"class=""#).skip(1) {
            let value = chunk.split('"').next().unwrap_or_default();
            if value.contains('{') {
                continue;
            }
            for class in value.split_whitespace() {
                assert!(
                    APP_CSS.contains(&format!(".{class}")),
                    "class `{class}` used by {name} has no rule in crates/ui/assets/app.css",
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 30,
        "expected the scan to reach the templates' class attributes, only saw {checked}",
    );
}

/// The V3 compact header: facts collapse into a `.facets.facets--bare`
/// chip row plus one `.detail__field--wide` canonical URL, and the full
/// fact set lives behind a collapsed `.disclosure` fold — never `.addbox`,
/// which is the Add-tenant dropdown and renders as a floating popover.
/// Since #801 the head also takes the HFS back-link idiom: a
/// `.page-head--back-link` modifier, a leading `.back-link`, and the rest
/// of the head wrapped in `.page-head__copy`.
///
/// Since #806 the fold is the shared `.disclosure` pattern rather than a
/// `<summary class="field__label">`: that class is `display: block`, which
/// suppresses the native `::marker` (it only paints at
/// `display: list-item`) and sets no `cursor`, so the fold looked inert.
#[test]
fn cs_detail_page_uses_the_v3_compact_header_shape() {
    const PAGE: &str = include_str!("../templates/pages/cs-detail.html");
    let body = strip_template_comments(PAGE);

    for hook in [
        r#"<header class="page-head page-head--back-link">"#,
        r#"class="back-link""#,
        r#"class="page-head__copy""#,
        r#"class="page-head__title""#,
        r#"class="stat__label""#,
        r#"class="facets facets--bare""#,
        r#"class="facet-label""#,
        r#"class="detail__field detail__field--wide""#,
        r#"<details class="disclosure">"#,
        r#"<summary class="disclosure__summary">"#,
        r#"<span class="icon disclosure__chevron" aria-hidden="true">"#,
    ] {
        assert!(
            body.contains(hook),
            "V3 compact header must render `{hook}`"
        );
    }
    // The summary now spans several lines (chevron span, then the label),
    // so the label text is asserted separately from the markup shape.
    assert!(
        body.contains(r#"{{ chrome.i18n.t("hts-cs-detail-facts-summary") }}"#),
        "the facts fold must still be labelled by `hts-cs-detail-facts-summary`",
    );
    for dead in [
        "page-header",
        "addbox",
        "hts-cs-detail__",
        "backlink",
        "row-link",
        "<dl",
    ] {
        assert!(
            !body.contains(dead),
            "`{dead}` belongs to the pre-V3 stacked layout and must be gone",
        );
    }
    // The disclosure must be collapsed by default so the workbench starts
    // high on the page — an `open` attribute would defeat the whole point.
    assert!(
        !body.contains("<details open"),
        "the facts disclosure must render collapsed",
    );
}

// ── The "Raw request and response" fold (#803) ──────────────────────────
//
// Three defects, all reproduced on `$lookup` and all fixed in the shared
// `partials/hts-raw-fold.html`:
//
// 1. The fold had no expand affordance — `<summary class="field__label">`
//    sets `display: block`, and a `<summary>` only draws the native
//    disclosure marker at `display: list-item`, so the triangle was gone in
//    every engine and the cursor never changed. (#806 fixed this for every
//    HTS fold with the shared `.disclosure` idiom; the shared partial takes
//    the same shape.)
// 2. The payload was an unhighlighted `<pre>` blob.
// 3. On a failed lookup there was no response JSON at all — `raw_body` was
//    populated only on the success path — and the request body was never
//    shown on any path, despite the heading promising it.

/// A `$lookup` mock that answers with `canned` and seeds the search legs
/// `read_code_system` needs so the run takes the §8.2 canonical path.
async fn start_lookup_mock(status: StatusCode, canned: serde_json::Value) -> String {
    let canned = Arc::new(canned);
    let router: Router = Router::new()
        .route(
            "/__mock_ready",
            axum::routing::get(|| async { (StatusCode::OK, "ok") }),
        )
        .route(
            "/CodeSystem",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "resourceType": "Bundle",
                    "type": "searchset",
                    "entry": [{
                        "resource": {
                            "resourceType": "CodeSystem",
                            "id": "icd9cm",
                            "url": "http://hl7.org/fhir/sid/icd-9-cm",
                            "name": "ICD-9-CM",
                            "status": "active"
                        }
                    }]
                }))
            }),
        )
        .route(
            "/CodeSystem/$lookup",
            axum::routing::post(move || {
                let canned = Arc::clone(&canned);
                async move { (status, axum::Json((*canned).clone())) }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock upstream listener");
    let addr: std::net::SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let base = format!("http://{addr}");
    let probe = reqwest::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .expect("build ready-probe client");
    let ready_url = format!("{base}/__mock_ready");
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match probe.get(&ready_url).send().await {
            Ok(r) if r.status().is_success() => break,
            _ => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
    base
}

async fn run_lookup(base: &str, form: &'static str) -> String {
    let response = app_pointing_at(base)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/code-systems/icd9cm/lookup")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::ACCEPT_LANGUAGE, "en")
                .header("HX-Request", "true")
                .body(Body::from(form))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body_text(response).await
}

#[tokio::test]
async fn raw_fold_reads_as_a_control_and_highlights_both_payloads() {
    let base = start_lookup_mock(
        StatusCode::OK,
        serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "display", "valueString": "Cholera" },
                { "name": "system", "valueUri": "http://hl7.org/fhir/sid/icd-9-cm" }
            ]
        }),
    )
    .await;
    let html = run_lookup(&base, "code=001").await;

    // 1. The card-shaped JSON fold idiom (#898): a `.card .json-fold` with
    //    `.card-head` summary carrying a chevron icon.
    assert!(
        html.contains(r#"<details class="card json-fold""#)
            && html.contains(r#"<summary class="card-head">"#)
            && html.contains(r#"<span class="icon">"#),
        "the fold must render the card json-fold idiom; got:\n{html}",
    );
    assert!(
        !html.contains(r#"<summary class="field__label">"#),
        "`.field__label` on a summary is what removed the disclosure marker",
    );

    // 2. Both payloads go through the shared highlighted JSON view.
    assert_eq!(
        html.matches(r#"class="json-view""#).count(),
        2,
        "request and response must each render a JSON view; got:\n{html}",
    );
    assert!(
        html.contains(r#"class="jt--key""#),
        "tokens must be coloured"
    );

    // 3. The request body is shown at all — the heading has promised it
    //    since it was written.
    assert!(
        html.contains("valueCode") && html.contains("001"),
        "the POSTed Parameters must be visible; got:\n{html}",
    );
    assert!(html.contains("/CodeSystem/$lookup"), "and the request URL");
}

#[tokio::test]
async fn a_failed_lookup_still_shows_the_response_payload() {
    // The body was never unavailable on this path — `status_to_error`
    // parsed it for its OperationOutcome and dropped the JSON, so a 404
    // showed the request URL and nothing else, precisely when the raw
    // payload is most wanted.
    let base = start_lookup_mock(
        StatusCode::NOT_FOUND,
        serde_json::json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": "not-found",
                "diagnostics": "code 999 not found in http://hl7.org/fhir/sid/icd-9-cm"
            }]
        }),
    )
    .await;
    let html = run_lookup(&base, "code=999").await;

    assert!(
        html.contains(r#"data-severity="error""#),
        "the structured outcome must still render above the fold",
    );
    assert!(
        html.contains(r#"<details class="card json-fold""#),
        "a failed lookup must still render the fold; got:\n{html}",
    );
    assert!(
        html.contains("code 999 not found"),
        "the response payload must survive the error path; got:\n{html}",
    );
    assert!(
        html.contains("999") && html.contains("valueCode"),
        "so must the request body that provoked it",
    );
}

#[tokio::test]
async fn a_submit_rejected_before_sending_renders_no_fold() {
    // Nothing went over the wire, so there is no exchange to disclose and
    // an empty fold would be worse than none.
    let response = app()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/code-systems/icd9cm/lookup")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(""))
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;

    assert!(
        !html.contains(r#"<details class="card json-fold""#),
        "got:\n{html}"
    );
}

/// Regression guard for #806 / #898: no CodeSystem-surface fold may go back to
/// `<summary class="field__label">`. `.field__label` is `display: block`,
/// which strips the native `::marker` (a `<summary>` only draws one at
/// `display: list-item`) and sets no `cursor`, so such a fold renders as an
/// inert grey label with no disclosure affordance at all.
///
/// There are two fold types in the CodeSystem surface:
/// - Fact-set folds (`.disclosure` pattern) for collapsing CodeSystem metadata
/// - Raw JSON folds (`.card .json-fold` pattern, #898) for raw response payloads
///
/// Both must ship an explicit chevron. The raw fold template must use the
/// card-shaped json-fold pattern.
#[test]
fn cs_detail_folds_never_reuse_the_markerless_field_label_summary() {
    // (template, fold type). The workbench result includes the raw fold
    // from `partials/hts-raw-fold.html` (#803), so the shape is asserted on
    // the partial rather than on the includer.
    //
    // Fold types: "disclosure" = fact-set, "json-fold" = raw JSON (#898)
    let templates: &[(&str, &str, Option<&str>)] = &[
        (
            "pages/cs-detail.html",
            include_str!("../templates/pages/cs-detail.html"),
            Some("disclosure"), // fact-set fold, not raw JSON
        ),
        (
            "partials/hts-cs-workbench-result.html",
            include_str!("../templates/partials/hts-cs-workbench-result.html"),
            None, // includes hts-raw-fold.html, does not define its own fold
        ),
        (
            "partials/hts-raw-fold.html",
            include_str!("../templates/partials/hts-raw-fold.html"),
            Some("json-fold"), // raw JSON fold, must use card pattern (#898)
        ),
    ];

    for (name, template, fold_type) in templates {
        let body = strip_template_comments(template);
        assert!(
            !body.contains(r#"<summary class="field__label""#),
            "{name} must not use markerless `<summary class=\"field__label\">`",
        );
        match *fold_type {
            Some("disclosure") => {
                // Fact-set fold: expects `.disclosure` pattern with chevron
                assert!(
                    body.contains(r#"<summary class="disclosure__summary">"#),
                    "{name} must render the shared `.disclosure__summary` fold",
                );
                assert!(
                    body.contains(r#"class="icon disclosure__chevron""#),
                    "{name}'s fold must render the explicit `.disclosure__chevron`",
                );
            }
            Some("json-fold") => {
                // Raw JSON fold: expects card `.json-fold` pattern (#898)
                assert!(
                    body.contains(r#"<summary class="card-head">"#),
                    "{name} must render the card-head summary for the json-fold",
                );
                assert!(
                    body.contains(r#"class="icon">"#),
                    "{name}'s fold must render the chevron icon",
                );
            }
            _ => {
                // No fold of its own — just checking it doesn't use field__label
            }
        }
    }
}
