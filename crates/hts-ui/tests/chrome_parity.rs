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
        metrics_ring: Default::default(),
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
    // The hook is `.row-link`, not the old `.backlink`: the V3 layout pass
    // dropped every HTS-only class, and `.backlink` has no rule in
    // `crates/ui/assets/app.css`.
    // Template-source check because the backlink lives inside
    // `{% if let Some(summary) = self.summary() %}` and closed-loopback
    // summaries are `None`; a source check is both stricter and cheaper
    // than standing up a per-resource mock upstream.
    let needle = "<a class=\"row-link\" href=\"/ui/hts/code-systems\">\u{2039} {{ chrome.i18n.t(\"hts-cs-browser-title\") }}</a>";
    assert!(
        CS_DETAIL.contains(needle),
        "cs-detail.html must contain the backlink verbatim (chevron U+2039, hardcoded href): {needle}",
    );
}

#[test]
fn vs_detail_template_carries_backlink_to_value_sets_browser() {
    let needle = "<a class=\"row-link\" href=\"/ui/hts/value-sets\">\u{2039} {{ chrome.i18n.t(\"hts-vs-browser-title\") }}</a>";
    assert!(
        VS_DETAIL.contains(needle),
        "vs-detail.html must contain the backlink verbatim (chevron U+2039, hardcoded href): {needle}",
    );
}

#[test]
fn cm_detail_template_carries_backlink_to_concept_maps_browser() {
    let needle = "<a class=\"row-link\" href=\"/ui/hts/concept-maps\">\u{2039} {{ chrome.i18n.t(\"hts-cm-browser-title\") }}</a>";
    assert!(
        CM_DETAIL.contains(needle),
        "cm-detail.html must contain the backlink verbatim (chevron U+2039, hardcoded href): {needle}",
    );
}

// ── Track E: Home chart geometry is a guarded copy of HFS's ──────────

/// Pull `const NAME: i64 = <int>;` out of a Rust source file.
fn rust_const(source: &str, name: &str) -> i64 {
    let needle = format!("const {name}: i64 = ");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("`{needle}…` not found — did the constant get renamed?"))
        + needle.len();
    let rest = &source[start..];
    let end = rest
        .find(';')
        .unwrap_or_else(|| panic!("`const {name}` has no terminating `;`"));
    rest[..end]
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("`const {name}` is not an integer literal: {e}"))
}

/// Pull the plot width out of a `viewBox="0 0 <width> …"` attribute.
fn view_box_width(template: &str) -> i64 {
    let needle = "viewBox=\"0 0 ";
    let start = template
        .find(needle)
        .expect("template must carry a `viewBox=\"0 0 …\"` attribute")
        + needle.len();
    let rest = &template[start..];
    let end = rest.find(' ').expect("viewBox must have four components");
    rest[..end]
        .parse()
        .expect("viewBox width must be an integer")
}

#[test]
fn chart_geometry_matches_hfs() {
    // Extracting a shared `helios-ui-chrome` crate is deferred as its own
    // piece of work, so `crates/hts-ui/src/chart.rs` is a *copy* of HFS's
    // chart geometry rather than a shared dependency. That copy is only
    // safe while it stays identical: both charts render against the same
    // `.chart` / `.grid-line` / `.axis-label` rules in the one shared
    // `crates/ui/assets/app.css`, so a plot box that drifts on one side
    // would silently mis-render on the other.
    //
    // Read both sides off disk rather than `include_str!`-ing only ours:
    // the point is to notice when *HFS* moves.
    let hfs_lib = std::fs::read_to_string("../ui/src/lib.rs")
        .expect("HFS lib.rs must be readable from the hts-ui crate directory");
    let hfs_index = std::fs::read_to_string("../ui/templates/pages/index.html")
        .expect("HFS index.html must be readable from the hts-ui crate directory");
    let hts_chart = include_str!("../src/chart.rs");
    let hts_template = include_str!("../templates/partials/hts-home-chart.html");

    for name in ["PLOT_LEFT", "PLOT_RIGHT", "PLOT_TOP"] {
        assert_eq!(
            rust_const(&hfs_lib, name),
            rust_const(hts_chart, name),
            "{name} drifted between crates/ui/src/lib.rs and crates/hts-ui/src/chart.rs",
        );
    }

    // HFS derives its plot bottom as `CHART_HEIGHT - 22` inside `build_chart`;
    // HTS spells the result out as `PLOT_BOTTOM` so the template and the
    // geometry share one literal. Recompute HFS's to compare like for like.
    let hfs_height = rust_const(&hfs_lib, "CHART_HEIGHT");
    let hfs_bottom_offset = {
        let needle = "let plot_bottom = height - ";
        let start = hfs_lib
            .find(needle)
            .expect("HFS build_chart must still derive plot_bottom from height")
            + needle.len();
        let rest = &hfs_lib[start..];
        let end = rest
            .find(';')
            .expect("plot_bottom expression must end in `;`");
        rest[..end].trim().parse::<i64>().expect("integer offset")
    };
    assert_eq!(
        hfs_height,
        rust_const(hts_chart, "CHART_HEIGHT"),
        "CHART_HEIGHT drifted between HFS and HTS",
    );
    assert_eq!(
        hfs_height - hfs_bottom_offset,
        rust_const(hts_chart, "PLOT_BOTTOM"),
        "HFS's plot bottom (CHART_HEIGHT - {hfs_bottom_offset}) no longer equals HTS's PLOT_BOTTOM",
    );

    // The viewBox width is the other half of the contract: HFS's index.html
    // and HTS's chart partial must open the same coordinate space, and it
    // must equal PLOT_RIGHT on both sides.
    let hfs_width = view_box_width(&hfs_index);
    let hts_width = view_box_width(hts_template);
    assert_eq!(hfs_width, hts_width, "chart viewBox width drifted");
    assert_eq!(
        hts_width,
        rust_const(hts_chart, "PLOT_RIGHT"),
        "the chart viewBox width must equal PLOT_RIGHT",
    );

    // Spelled-out expectations, so a *coordinated* drift on both sides still
    // has to be a deliberate edit to this test.
    assert_eq!(rust_const(hts_chart, "PLOT_LEFT"), 40);
    assert_eq!(rust_const(hts_chart, "PLOT_RIGHT"), 1060);
    assert_eq!(rust_const(hts_chart, "PLOT_TOP"), 10);
    assert_eq!(rust_const(hts_chart, "PLOT_BOTTOM"), 278);
    assert_eq!(rust_const(hts_chart, "CHART_HEIGHT"), 300);
}

/// Remove `{# … #}` Askama comments so a class named in prose is not mistaken
/// for one the template emits.
fn strip_askama_comments(template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find("{#") {
        out.push_str(&rest[..open]);
        match rest[open..].find("#}") {
            Some(close) => rest = &rest[open + close + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

#[test]
fn home_chart_uses_only_classes_that_exist_in_the_shared_stylesheet() {
    // The HTS UI adds no CSS of its own — it embeds `crates/ui/assets`
    // wholesale. A class with no rule there is dead markup, so every class
    // the chart partial emits must be greppable in app.css.
    let css = std::fs::read_to_string("../ui/assets/app.css").expect("shared app.css");
    // Strip `{# … #}` template comments first: this file documents *why* two
    // HFS classes are omitted, and naming them in prose must not read as
    // emitting them.
    let template = strip_askama_comments(include_str!("../templates/partials/hts-home-chart.html"));
    let template = template.as_str();
    for class in [
        "chart-card",
        "chart-card__head",
        "chart-card__tools",
        "window-picker",
        "window-picker__option",
        "window-picker__option--active",
        "chart-legend",
        "chart-legend__item",
        "chart-legend__item--active",
        "chart-legend__total",
        "chart-legend__dot",
        "stat__label",
        "stat__value",
        "stat__sub",
        "grid-line",
        "axis-label",
    ] {
        assert!(
            template.contains(class),
            "the chart partial must still emit `{class}`",
        );
        assert!(
            css.contains(&format!(".{class}")),
            "`{class}` has no rule in crates/ui/assets/app.css — no new CSS is allowed",
        );
    }

    // HFS emits `.chart-legend__type` but app.css has no rule for it; the
    // HTS copy deliberately drops it in favour of a bare <span>.
    assert!(
        !template.contains("chart-legend__type"),
        "`chart-legend__type` has no CSS rule — do not copy it from HFS",
    );
    // The type-picker pill and the expand button have no HTS analogue and
    // would render as controls that do nothing.
    assert!(
        !template.contains("pill--square"),
        "the expand pill is a dead control in HTS — it must not be rendered",
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
    let start = html
        .find(anchor)
        .expect("id=hts-import-file must be present");
    // Walk back from the anchor to the enclosing `<input` and forward to
    // the closing `>`. A file input tag never spans more than ~500 chars.
    let window_start = start.saturating_sub(200);
    let window_end = (start + 500).min(html.len());
    let window = &html[window_start..window_end];
    let tag_open = window
        .rfind("<input")
        .expect("file input must be an <input tag");
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

// ── Track G: the account menu is the shared component, not a copy ─────

// Every other track in this file works the same way: it notices, after the
// fact, that HTS's chrome has drifted from HFS's. The account menu is the one
// place where that job has been taken away from testing entirely. #799 moved
// the markup into `crates/ui-chrome`, and both layouts now render it with a
// single `{{ … user_menu(…)?|safe }}` call, so there is no second copy left to
// drift.
//
// That only holds while nobody pastes the markup back. This track defends the
// structure rather than the output: one test pins the page to the shared
// function's exact bytes, one records that the control HTS carried alone was
// dead markup (so it does not come back as a "missing feature"), and one fails
// the moment either product grows its own copy of the block again.

#[tokio::test]
async fn account_menu_is_the_shared_chrome_component() {
    // Rendered here through the same public entry point the layout uses, then
    // demanded verbatim from the page. A look-alike does not pass.
    //
    // The HFS twin — `the_account_menu_is_the_shared_component_verbatim` in
    // `crates/ui/tests/router_http.rs` — asserts this same function's output
    // against `/ui`. Neither test names the other crate, but together they pin
    // the two products to each other through `helios-ui-chrome`: HFS ==
    // user_menu(..) == HTS. That is the property #799 was for, and it needs no
    // cross-crate dev-dependency to state.
    //
    // `RequestLocale::default()` is `en`, which is what a request with no
    // `?lang=`, no `hts_lang` cookie and no `Accept-Language` negotiates.
    let i18n = helios_hts_ui::I18n::new(helios_hts_ui::RequestLocale::default());
    // `UserIdentity::default()` is the signed-out local-operator state both
    // products render today (#320): no display name, no photo, no Sign out.
    let expected = helios_ui_chrome::user_menu(&i18n, helios_ui_chrome::UserIdentity::default())
        .expect("the shared user-menu template has no fallible construct")
        .replace("\r\n", "\n");

    let response = app()
        .oneshot(Request::get("/ui/hts").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await.replace("\r\n", "\n");

    assert!(
        html.contains(&expected),
        "the HTS topbar does not contain `helios_ui_chrome::user_menu(..)` \
         verbatim — either the account menu was re-inlined into \
         `crates/hts-ui/templates/layouts/base.html`, or `Chrome::user_menu` \
         is passing something other than `UserIdentity::default()` (#799).\n\n\
         Expected to find:\n{expected}",
    );
}

#[tokio::test]
async fn dead_language_switcher_and_placeholder_avatar_are_gone() {
    // Two controls HTS carried that HFS never did. The `<nav
    // class="lang-switcher">` of three links and the hardcoded `K` initial in
    // the avatar were both HTS-only inventions; the language options now live
    // inside the shared menu, and the avatar falls back to the generic user
    // icon. Only the *markup* went — `?lang=` and the `hts_lang` cookie in
    // `src/i18n.rs` are untouched and still drive the options in their new home.
    let response = app()
        .oneshot(Request::get("/ui/hts").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        !html.contains("lang-switcher"),
        "the topbar `lang-switcher` nav must not render — the language options \
         moved inside the shared account menu (#799)",
    );
    assert!(
        !html.contains(">K<"),
        "the placeholder `K` avatar initial must not render — the shared menu \
         falls back to the generic user icon (#799)",
    );

    // Second leg: *why* it went. HTS adds no CSS of its own (it embeds
    // `crates/ui/assets` wholesale), and `.lang-switcher` has no rule there —
    // so those three links were rendering unstyled the whole time. This is the
    // record that removing them restored nothing and broke nothing.
    let css = std::fs::read_to_string("../ui/assets/app.css").expect("shared app.css");
    assert!(
        !css.contains(".lang-switcher"),
        "`.lang-switcher` has gained a rule in crates/ui/assets/app.css — if \
         the control is genuinely coming back it belongs in \
         `crates/ui-chrome`, rendered by both products, not in HTS alone",
    );
}

#[test]
fn neither_layout_carries_its_own_copy_of_the_account_menu() {
    // The anti-copy-paste guard, and the point of the whole track.
    //
    // Source check, not a rendered check: pasting the block back into a layout
    // would still render *something* that looks right, and might even satisfy
    // a substring assertion on the page. What must not exist is a second place
    // the markup can be edited.
    //
    // Read HFS's off disk (as the chart track does) rather than only ours —
    // the rule binds both products equally, and the point is to notice when
    // *HFS* re-inlines it.
    let hfs = std::fs::read_to_string("../ui/templates/layouts/base.html")
        .expect("HFS base.html must be readable from the hts-ui crate directory");
    let hts = include_str!("../templates/layouts/base.html");

    // Both layouts document in `{# … #}` prose why the markup left and where
    // it went, naming these very classes. Explaining the rule must not read as
    // breaking it.
    let hfs = strip_askama_comments(&hfs);
    let hts = strip_askama_comments(hts);

    for (crate_path, source) in [
        ("crates/ui/templates/layouts/base.html", hfs.as_str()),
        ("crates/hts-ui/templates/layouts/base.html", hts.as_str()),
    ] {
        for marker in [
            "menu--user",
            "user-menu__",
            "topbar__avatar",
            "icons/user.svg",
        ] {
            assert!(
                !source.contains(marker),
                "`{marker}` is back in {crate_path}.\n\n\
                 The account menu has exactly one home: `crates/ui-chrome`. \
                 Both layouts render it with a single \
                 `{{{{ … user_menu(…)?|safe }}}}` call and hold none of its \
                 markup themselves — that is the entire reason #799 exists, \
                 because the previous arrangement (a comment asserting the two \
                 copies were byte-for-byte identical) had already silently \
                 stopped being true. Change the partial in \
                 `crates/ui-chrome/templates/partials/user-menu.html` instead; \
                 if the two products genuinely need to differ, that difference \
                 belongs in `UserIdentity`, not in a second copy of the block.",
            );
        }
    }
}

#[tokio::test]
async fn favicon_link_resolves_under_the_hts_mount() {
    // HFS points its `rel="icon"` at `/ui/assets/logo.png`; HTS must point at
    // the same file under its own mount. Asserting only the markup would miss
    // a prefix typo, so the second leg fetches it — the same two-legged shape
    // `figtree_woff2_is_served_under_hts_assets` uses for the font.
    let response = app()
        .oneshot(Request::get("/ui/hts").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("<link rel=\"icon\" type=\"image/png\" href=\"/ui/hts/assets/logo.png\">"),
        "HTS must declare a favicon link to /ui/hts/assets/logo.png (HFS parity)",
    );

    let response = app()
        .oneshot(
            Request::get("/ui/hts/assets/logo.png")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the favicon the page links to must actually be served under \
         /ui/hts/assets/ — a mount-prefix typo shows up here, not in the markup",
    );
}

#[test]
fn workbench_input_group_wrappers_all_carry_the_field_class() {
    // `.field` (`display: block; margin-bottom: 14px` in the shared app.css)
    // is what gives a form group its trailing gap. A `role="group"` wrapper
    // without it contributes no margin of its own and inherits whatever its
    // last child happens to leave behind — in the case this pins (#807) that
    // was a `.field` whose own bottom margin collapsed out through the
    // unstyled wrapper, so the rendered geometry happened to match. That is a
    // coincidence of margin collapsing, not a design: give such a group a
    // non-`.field` last child (a `.facets--bare` row, say, as the Direction
    // group above it has) or put padding on the wrapper, and its spacing
    // silently drops to zero.
    //
    // Source check across all three workbench forms rather than one rendered
    // page: the omission was in the Forward *and* Reverse branches of the
    // translate form, and only one of those is in the DOM at a time.
    let templates = [
        (
            "crates/hts-ui/templates/partials/hts-cs-validate-input.html",
            include_str!("../templates/partials/hts-cs-validate-input.html"),
        ),
        (
            "crates/hts-ui/templates/partials/hts-cm-translate-input.html",
            include_str!("../templates/partials/hts-cm-translate-input.html"),
        ),
        (
            "crates/hts-ui/templates/partials/hts-vs-expand-input.html",
            include_str!("../templates/partials/hts-vs-expand-input.html"),
        ),
    ];

    let mut groups = 0usize;
    for (path, template) in templates {
        // `role="group"` is also discussed in these files' prose.
        let body = strip_askama_comments(template);
        for chunk in body.split('<').skip(1) {
            let Some(tag) = chunk.split('>').next() else {
                continue;
            };
            if !tag.contains(r#"role="group""#) {
                continue;
            }
            groups += 1;
            assert!(
                tag.contains(r#"class="field""#),
                "a `role=\"group\"` wrapper in {path} is missing `class=\"field\"`, \
                 so it drops out of the form's vertical rhythm:\n\n<{tag}>",
            );
        }
    }
    assert_eq!(
        groups, 8,
        "expected the scan to reach all eight workbench group wrappers",
    );
}
