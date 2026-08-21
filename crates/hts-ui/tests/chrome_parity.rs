//! Chrome / visual parity tests (design doc §14, added 2026-08-20).
//!
//! A reviewer flagged that HTS "did not look like HFS": Figtree did not
//! load, the nav lacked icons, the FHIR version was a bare badge instead
//! of a disclosure, and detail pages had no back-link. This ring pins
//! the fix so a future refactor cannot silently reintroduce the
//! divergence. It also guards the Import file-upload contract from
//! Track F: the file input is enabled and the form still POSTs
//! urlencoded — the backend handler stays untouched.
//!
//! Route tests reuse the closed-loopback fixture from `router_http.rs`
//! (see the `app()` helper below). The backlink assertions are
//! template-source checks via `include_str!`: the backlink lives inside
//! `{% if let Some(summary) = self.summary() %}`, and closed-loopback
//! summaries are `None`, so a source check is both cheaper and stricter
//! than standing up a per-test mock upstream.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

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

// ── Track A: Figtree fonts load from the shared stylesheet ─────────────

#[tokio::test]
async fn font_paths_are_relative_in_shared_css() {
    // §14.2 fix. The @font-face src must be relative so it resolves under
    // both `/ui/assets/app.css` (HFS) and `/ui/hts/assets/app.css` (HTS).
    // Absolute `/ui/assets/fonts/…` would 404 under the HTS mount.
    let response = app()
        .oneshot(
            Request::get("/ui/hts/assets/app.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let css = body_text(response).await;
    assert!(
        css.contains("url(\"fonts/figtree-latin.woff2\")"),
        "app.css must use relative font URL `fonts/figtree-latin.woff2` (see design §14.2)",
    );
    assert!(
        css.contains("url(\"fonts/figtree-latin-ext.woff2\")"),
        "app.css must use relative font URL `fonts/figtree-latin-ext.woff2` (see design §14.2)",
    );
    assert!(
        !css.contains("url(\"/ui/assets/fonts/"),
        "app.css must not carry the old absolute `/ui/assets/fonts/…` URL — it 404s under HTS",
    );
}

#[tokio::test]
async fn figtree_woff2_is_served_under_hts_assets() {
    // Guards that RustEmbed picked up `fonts/*.woff2` under the HTS mount.
    // Content-Type must be `font/woff2` for the browser to accept the
    // hint from `@font-face format("woff2")` without a MIME mismatch.
    let response = app()
        .oneshot(
            Request::get("/ui/hts/assets/fonts/figtree-latin.woff2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "figtree-latin.woff2 must be served under /ui/hts/assets/fonts/",
    );
    let ctype = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap_or_default().to_ascii_lowercase())
        .unwrap_or_default();
    assert!(
        ctype.starts_with("font/woff2") || ctype.starts_with("application/font-woff2"),
        "unexpected content-type for figtree-latin.woff2: {ctype:?}",
    );
}

// ── Track B: nav items render inline SVG icons ────────────────────────

#[tokio::test]
async fn sidebar_renders_brand_logo() {
    // Reviewer complaint: HTS sidebar was missing the brand logo. HFS
    // renders `<img class="brand__logo">`; HTS must render the same.
    let response = app()
        .oneshot(Request::get("/ui/hts").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("class=\"brand__logo\""),
        "sidebar must render <img class=\"brand__logo\"> (HFS parity)",
    );
    assert!(
        html.contains("/ui/hts/assets/logo.png"),
        "brand logo src must resolve under /ui/hts/assets/",
    );
}

#[tokio::test]
async fn sidebar_nav_items_render_inline_svg_icons() {
    // §14.3: HFS wraps every nav-item label with
    // `<span class="icon">{% include "icons/X.svg" %}</span>`. The
    // included SVG opens with `<svg ` — a substring that will never
    // appear from text content alone.
    let response = app()
        .oneshot(Request::get("/ui/hts").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // Seven nav items in the HTS sidebar (home, code-systems,
    // value-sets, concept-maps, operations, import, diagnostics). Each
    // opens with an <a class="nav-item" …> and must carry its own
    // <span class="icon">. Counting the class marker across the page is
    // sufficient: the FHIR selector adds a few more `<span class="icon">`
    // slots, so the total must be >= 7.
    let icon_count = html.matches("<span class=\"icon\">").count();
    assert!(
        icon_count >= 7,
        "expected at least 7 `<span class=\"icon\">` slots (one per nav item), got {icon_count}",
    );
    // The inline SVGs actually landed — includes resolved to markup.
    assert!(
        html.contains("<svg "),
        "nav icon <span> must contain an inlined <svg …> element",
    );
}

// ── Track C: FHIR selector is a details.menu.menu--up disclosure ──────

#[tokio::test]
async fn fhir_version_selector_uses_details_menu_disclosure() {
    // §14.4: the display-only FHIR selector must render as a
    // `<details class="menu menu--up">` disclosure with a `<summary
    // class="selector selector--outline">`, matching HFS. The old
    // `<span class="fhir-badge">` must be gone.
    let response = app()
        .oneshot(Request::get("/ui/hts").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains("<details class=\"menu menu--up\">"),
        "FHIR selector must be a <details class=\"menu menu--up\"> disclosure (HFS parity, §14.4)",
    );
    assert!(
        html.contains("class=\"selector selector--outline\""),
        "FHIR selector <summary> must carry `selector selector--outline` for HFS chrome parity",
    );
    assert!(
        !html.contains("class=\"fhir-badge\""),
        "old `fhir-badge` span must not render (replaced by the details disclosure, §14.4)",
    );
    // Single degenerate option: R4 marked as current with a check icon.
    assert!(
        html.contains("aria-current=\"true\""),
        "the sole FHIR-version option must be marked `aria-current=\"true\"` (display-only)",
    );
}

// ── Track D: backlink lives in the three detail templates ─────────────

const CS_DETAIL: &str = include_str!("../templates/pages/cs-detail.html");
const VS_DETAIL: &str = include_str!("../templates/pages/vs-detail.html");
const CM_DETAIL: &str = include_str!("../templates/pages/cm-detail.html");

#[test]
fn cs_detail_template_carries_backlink_to_code_systems_browser() {
    // §14.5: Category C clone of `crates/ui/templates/pages/bulk-import-detail.html`
    // — hardcoded href to the list page, chevron U+2039, Fluent title key.
    // Template-source check because the backlink lives inside
    // `{% if let Some(summary) = self.summary() %}` and closed-loopback
    // summaries are `None`; a source check is both stricter and cheaper
    // than standing up a per-resource mock upstream.
    let needle =
        "<a class=\"backlink\" href=\"/ui/hts/code-systems\">\u{2039} {{ chrome.i18n.t(\"hts-cs-browser-title\") }}</a>";
    assert!(
        CS_DETAIL.contains(needle),
        "cs-detail.html must contain the backlink verbatim (chevron U+2039, hardcoded href): {needle}",
    );
}

#[test]
fn vs_detail_template_carries_backlink_to_value_sets_browser() {
    let needle =
        "<a class=\"backlink\" href=\"/ui/hts/value-sets\">\u{2039} {{ chrome.i18n.t(\"hts-vs-browser-title\") }}</a>";
    assert!(
        VS_DETAIL.contains(needle),
        "vs-detail.html must contain the backlink verbatim (chevron U+2039, hardcoded href): {needle}",
    );
}

#[test]
fn cm_detail_template_carries_backlink_to_concept_maps_browser() {
    let needle =
        "<a class=\"backlink\" href=\"/ui/hts/concept-maps\">\u{2039} {{ chrome.i18n.t(\"hts-cm-browser-title\") }}</a>";
    assert!(
        CM_DETAIL.contains(needle),
        "cm-detail.html must contain the backlink verbatim (chevron U+2039, hardcoded href): {needle}",
    );
}

// ── Track F: Import file support (Batch-style, no backend change) ─────

#[tokio::test]
async fn import_form_enables_file_radio_and_input() {
    // §14.6: the `source=file` radio and `<input type="file"
    // name="bundle_file">` must both be enabled. If either regresses to
    // `disabled`, the file upload UX silently stops working.
    let response = app()
        .oneshot(Request::get("/ui/hts/import").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // The file radio input.
    let file_radio_marker = "<input type=\"radio\" name=\"source\" value=\"file\">";
    assert!(
        html.contains(file_radio_marker),
        "file radio must render *without* the `disabled` attribute (§14.6). Rendered form snippet: {}",
        html.get(0..html.len().min(4000)).unwrap_or_default(),
    );
    // The file input itself. The template renders it with `id=` first.
    assert!(
        html.contains("id=\"hts-import-file\""),
        "file input must render with stable id `hts-import-file`",
    );
    // Belt-and-suspenders: the file input must not carry `disabled`.
    // Simplest heuristic: locate the `<input id="hts-import-file"` slice
    // and confirm the containing tag has no `disabled` attribute.
    let anchor = "id=\"hts-import-file\"";
    let start = html.find(anchor).expect("id=hts-import-file must be present");
    // Walk back from the anchor to the enclosing `<input` and forward to
    // the closing `>`. A file input tag never spans more than ~500 chars.
    let window_start = start.saturating_sub(200);
    let window_end = (start + 500).min(html.len());
    let window = &html[window_start..window_end];
    let tag_open = window.rfind("<input").expect("file input must be an <input tag");
    let tag_end_rel = window[tag_open..]
        .find('>')
        .expect("<input tag must be closed");
    let tag = &window[tag_open..tag_open + tag_end_rel + 1];
    assert!(
        !tag.contains(" disabled"),
        "<input id=\"hts-import-file\"> must not carry `disabled` (§14.6). Rendered tag: {tag}",
    );
}

#[tokio::test]
async fn import_form_stays_urlencoded_for_paste_regression() {
    // §14.6 constraint: file support was added *without* touching the
    // backend. That means the wire format stays
    // `application/x-www-form-urlencoded` and `import.js` reads the file
    // into the textarea before submit. If someone flips the enctype to
    // `multipart/form-data` without also swapping the handler, both
    // paths (paste + file) break silently.
    let response = app()
        .oneshot(Request::get("/ui/hts/import").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("enctype=\"application/x-www-form-urlencoded\""),
        "Import form must stay urlencoded so the existing handler keeps working (§14.6)",
    );
    // And the FileReader sink script is wired in.
    assert!(
        html.contains("/ui/hts/assets/import.js"),
        "Import page must load import.js — the FileReader → textarea sink (§14.6)",
    );
}

#[tokio::test]
async fn import_js_is_served_under_hts_assets() {
    // The FileReader sink lives at `crates/ui/assets/import.js`; both
    // HFS and HTS binaries embed the same folder, so the file must be
    // reachable under /ui/hts/assets/ for the HTS Import page.
    let response = app()
        .oneshot(
            Request::get("/ui/hts/assets/import.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "import.js must be served under /ui/hts/assets/",
    );
    let js = body_text(response).await;
    assert!(
        js.contains("FileReader"),
        "import.js must use FileReader (Batch-style sink, §14.6)",
    );
    assert!(
        js.contains("hts-import-bundle"),
        "import.js must write into the shared textarea `#hts-import-bundle`",
    );
}
