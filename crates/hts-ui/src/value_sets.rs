//! ValueSet browser + detail with embedded `$expand` workbench
//! (design doc §7.4).
//!
//! The module registers five routes under `/hts/value-sets`, mirroring the
//! CS Slice B naming (design doc §7.4 wireframe):
//!
//! - `GET  /hts/value-sets`                — full-page browser.
//! - `GET  /hts/value-sets/rows`           — filter-form target (rows partial).
//! - `GET  /hts/value-sets/{id}`           — 302 redirect to `/{id}/expand`
//!   (design doc §8.3: operation-first landing; the former "Metadata" tab
//!   is gone and the facts block is always visible above the tab strip).
//! - `GET  /hts/value-sets/{id}/expand`    — Expand tab input partial (or
//!   full page on hard nav).
//! - `POST /hts/value-sets/{id}/expand`    — runs `$expand` and returns the
//!   Expand result partial (or a full page on hard nav).
//!
//! `$expand` proxies to HTS as `POST /ValueSet/{id}/$expand` per design doc
//! §7.6 proxy verb rule, regardless of the source form verb. VS
//! `$validate-code` defers to Slice E's standalone workbench (§7.4.1 F9) —
//! there is no Validate tab in Slice C.

use askama::Template;
use axum::{
    Router,
    body::Bytes,
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use axum_htmx::HxRequest;
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};

use crate::i18n::{I18n, RequestLocale};
use crate::raw_fold::RawFold;
use crate::raw_json_fragment::{
    PaneTarget, vs_expand_expand_url, vs_expand_extra_query, vs_expand_fragment_endpoint,
};
use crate::upstream::{
    ExpandParams, ExpansionResult, HTS_UI_MAX_EXPANSION_SIZE_HINT, OpFailure, OutcomeView,
    UpstreamError, ValueSetSummary, VsBrowserFilters, VsBrowserPage,
};
use crate::{Chrome, HtsUiState};

// ── Routing ─────────────────────────────────────────────────────────────

pub(crate) fn routes() -> Router<Arc<HtsUiState>> {
    Router::new()
        .route("/hts/value-sets", get(browser_page))
        .route("/hts/value-sets/rows", get(browser_rows))
        .route("/hts/value-sets/{id}", get(detail_page))
        .route(
            "/hts/value-sets/{id}/expand",
            get(expand_input).post(expand_run),
        )
}

/// Which detail-page tab a render targets. `Expand` is the only variant
/// today (design doc §8.3 — operation-first landing; the former
/// `Metadata` variant is gone). `$validate-code` defers to Slice E's
/// standalone workbench (§7.4.1 F9); a Validate variant would land here
/// when that slice ships.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VsTab {
    Expand,
}

impl VsTab {
    /// URL-slug rendering of the tab. Mirrors [`crate::code_systems`]'s
    /// `CsTab::slug`; kept alongside the enum so future template additions
    /// can reuse it without re-deriving the match.
    #[allow(dead_code)]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Expand => "expand",
        }
    }
}

// ── Browser page (§7.4 mirror of §7.2) ──────────────────────────────────

/// Query shape accepted by the browser page and its rows fragment. `_count`
/// and `_offset` are `Option<String>` so a malformed value collapses to the
/// defaults rather than tripping axum's 400-on-deserialize-fail (design
/// doc §7.4.1 invariant #1 clamp).
#[derive(Debug, Deserialize, Default)]
struct BrowserForm {
    url: Option<String>,
    version: Option<String>,
    name: Option<String>,
    title: Option<String>,
    status: Option<String>,
    #[serde(rename = "_count")]
    count: Option<String>,
    #[serde(rename = "_offset")]
    offset: Option<String>,
    #[allow(dead_code)]
    lang: Option<String>,
}

impl BrowserForm {
    fn into_filters(self) -> VsBrowserFilters {
        let count = self
            .count
            .as_deref()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);
        let offset = self
            .offset
            .as_deref()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);
        VsBrowserFilters {
            url: non_empty(self.url),
            version: non_empty(self.version),
            name: non_empty(self.name),
            title: non_empty(self.title),
            status: non_empty(self.status),
            count,
            offset,
        }
    }
}

fn non_empty(v: Option<String>) -> Option<String> {
    v.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_owned())
        }
    })
}

#[derive(Template)]
#[template(path = "pages/vs-browser.html")]
struct BrowserPageTemplate<'a> {
    chrome: Chrome<'a>,
    view: BrowserRowsView,
}

#[derive(Template)]
#[template(path = "partials/hts-vs-rows.html")]
struct BrowserRowsTemplate<'a> {
    chrome: Chrome<'a>,
    view: BrowserRowsView,
}

/// Rows-partial data. Every arm renders legibly on its own so empty /
/// error / degraded states stay identical between hard nav and htmx swap.
struct BrowserRowsView {
    filters: VsBrowserFilters,
    result: Result<VsBrowserPage, UpstreamError>,
    count_over_max: bool,
}

impl BrowserRowsView {
    fn degraded_reason(&self) -> Option<&'static str> {
        match &self.result {
            Err(e)
                if matches!(
                    e,
                    UpstreamError::Connect { .. }
                        | UpstreamError::Timeout { .. }
                        | UpstreamError::ClientBuild { .. }
                ) =>
            {
                Some(e.degraded_reason())
            }
            _ => None,
        }
    }

    fn outcome(&self) -> Option<OutcomeView> {
        if self.count_over_max {
            return Some(OutcomeView::invalid_input(format!(
                "_count must be between 1 and {}",
                VsBrowserFilters::MAX_COUNT
            )));
        }
        match &self.result {
            Err(UpstreamError::Outcome { outcome, .. }) => Some((**outcome).clone()),
            Err(UpstreamError::HttpStatus { status, .. }) => Some(OutcomeView {
                severity: "error".to_owned(),
                code: match *status {
                    400 => "invalid",
                    404 => "not-found",
                    422 => "too-costly",
                    _ => "unknown",
                }
                .to_owned(),
                ..OutcomeView::default()
            }),
            _ => None,
        }
    }

    fn page(&self) -> Option<&VsBrowserPage> {
        self.result.as_ref().ok()
    }

    /// See `code_systems::BrowserRowsView::status_url` — the status facet
    /// chips are plain links, so each carries the active text filters
    /// forward and drops `_offset`.
    fn status_url(&self, status: &str) -> String {
        let mut ser = form_urlencoded::Serializer::new(String::new());
        for (field, value) in [
            ("url", &self.filters.url),
            ("version", &self.filters.version),
            ("name", &self.filters.name),
            ("title", &self.filters.title),
        ] {
            if let Some(v) = value {
                if !v.is_empty() {
                    ser.append_pair(field, v);
                }
            }
        }
        if !status.is_empty() {
            ser.append_pair("status", status);
        }
        let query = ser.finish();
        if query.is_empty() {
            "/ui/hts/value-sets".to_owned()
        } else {
            format!("/ui/hts/value-sets?{query}")
        }
    }

    /// Whether `status` is the chip currently selected. Empty == "any".
    fn status_is(&self, status: &str) -> bool {
        match &self.filters.status {
            Some(active) => active == status,
            None => status.is_empty(),
        }
    }
}

async fn browser_page(
    State(state): State<Arc<HtsUiState>>,
    Query(form): Query<BrowserForm>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    let view = load_browser_view(&state, form).await;
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "value-sets",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    render(BrowserPageTemplate { chrome, view }.render())
}

async fn browser_rows(
    State(state): State<Arc<HtsUiState>>,
    Query(form): Query<BrowserForm>,
    // Extracted so `AutoVaryLayer` marks the response `Vary: HX-Request`
    // even in the branch where the body is identical between htmx and
    // hard nav (mirrors the CS rows fragment).
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    let view = load_browser_view(&state, form).await;
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "value-sets",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    render(BrowserRowsTemplate { chrome, view }.render())
}

async fn load_browser_view(state: &HtsUiState, form: BrowserForm) -> BrowserRowsView {
    let filters = form.into_filters();
    if filters.count_exceeds_cap() {
        return BrowserRowsView {
            filters,
            result: Ok(VsBrowserPage {
                rows: Vec::new(),
                filters: VsBrowserFilters::default(),
            }),
            count_over_max: true,
        };
    }
    let result = state.upstream.search_value_sets(&filters).await;
    BrowserRowsView {
        filters,
        result,
        count_over_max: false,
    }
}

// ── Detail page (§7.4) ──────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/vs-detail.html")]
struct DetailPageTemplate<'a> {
    chrome: Chrome<'a>,
    id: String,
    detail: Result<ValueSetSummary, UpstreamError>,
    tab: VsTab,
    /// Populated only when this render is an Expand result. Keeps the
    /// `tab` field independent of the workbench state, matching Slice B.
    workbench: Option<ExpandResultView>,
    /// Build-time threshold ceiling forwarded to the input / result
    /// partials so the ceiling tooltip and warning render without the
    /// template touching module-private state.
    ceiling: u64,
}

impl<'a> DetailPageTemplate<'a> {
    fn degraded_reason(&self) -> Option<&'static str> {
        match &self.detail {
            Err(e)
                if matches!(
                    e,
                    UpstreamError::Connect { .. }
                        | UpstreamError::Timeout { .. }
                        | UpstreamError::ClientBuild { .. }
                ) =>
            {
                Some(e.degraded_reason())
            }
            _ => None,
        }
    }

    fn outcome(&self) -> Option<OutcomeView> {
        match &self.detail {
            Err(UpstreamError::NotFound { .. }) => Some(OutcomeView {
                severity: "error".to_owned(),
                code: "not-found".to_owned(),
                ..OutcomeView::default()
            }),
            Err(UpstreamError::Outcome { outcome, .. }) => Some((**outcome).clone()),
            Err(UpstreamError::HttpStatus { status, .. }) => Some(OutcomeView {
                severity: "error".to_owned(),
                code: match *status {
                    400 => "invalid".to_owned(),
                    404 => "not-found".to_owned(),
                    _ => "unknown".to_owned(),
                },
                ..OutcomeView::default()
            }),
            _ => None,
        }
    }

    fn summary(&self) -> Option<&ValueSetSummary> {
        self.detail.as_ref().ok()
    }

    /// Threshold echo for the Expand-input partial. Pulled off the
    /// carried workbench view (if any) so the numeric input keeps the
    /// operator's most-recent submitted value across a re-render.
    fn threshold(&self) -> Option<u64> {
        self.workbench.as_ref().and_then(|w| w.threshold)
    }

    /// Tree/flat toggle echo for the Expand-input partial. `None` when
    /// no workbench render has happened yet — the input then falls back
    /// to the `flat` default.
    fn tree_mode(&self) -> Option<bool> {
        self.workbench.as_ref().map(|w| w.tree_mode)
    }
}

/// Base detail URL — permanent-redirects to the default operation tab
/// (§8.3 operation-first landing). The Expand handler renders the full
/// detail: facts block above the tab strip, Expand input as the active
/// tab.
async fn detail_page(Path(id): Path<String>) -> Response {
    Redirect::permanent(&format!("/ui/hts/value-sets/{id}/expand")).into_response()
}

// ── Workbench input (GET Expand tab handler) ────────────────────────────
//
// Tabs are wrapped in `#hts-vs-detail-region` (design doc §8.1); a tab
// click uses `hx-select="#hts-vs-detail-region"` so we always render the
// full detail page and htmx picks the region out.

async fn expand_input(
    State(state): State<Arc<HtsUiState>>,
    Path(id): Path<String>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "value-sets",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    let summary = state.upstream.read_value_set(&id).await.ok();
    render_detail_with_tab(&state, chrome, id, VsTab::Expand, summary).await
}

async fn render_detail_with_tab<'a>(
    state: &HtsUiState,
    chrome: Chrome<'a>,
    id: String,
    tab: VsTab,
    prefetched: Option<ValueSetSummary>,
) -> Response {
    let detail = match prefetched {
        Some(s) => Ok(s),
        None => state.upstream.read_value_set(&id).await,
    };
    render(
        DetailPageTemplate {
            chrome,
            id,
            detail,
            tab,
            workbench: None,
            ceiling: HTS_UI_MAX_EXPANSION_SIZE_HINT,
        }
        .render(),
    )
}

// ── Workbench run (POST Expand handler) ─────────────────────────────────

#[derive(Template)]
#[template(path = "partials/hts-vs-expand-result.html")]
struct ExpandResultTemplate<'a> {
    chrome: Chrome<'a>,
    id: String,
    view: ExpandResultView,
    ceiling: u64,
}

/// Payload for the Expand result partial. Kept per-op (§7.4.1 F3): the
/// abstract "hts-concept" renderer stays aspirational until a second
/// operation demands the cross-slice refactor.
#[derive(Clone, Debug)]
pub struct ExpandResultView {
    /// What went over the wire, for the "Raw request and response" fold.
    pub raw: RawFold,
    pub result: Option<ExpansionResult>,
    pub outcome: Option<OutcomeView>,
    pub degraded_reason: Option<&'static str>,
    /// The tree/flat toggle position the operator submitted. Echoed
    /// back so the Load-more pager and re-render preserve the mode.
    pub tree_mode: bool,
    /// Echoed threshold seed for the numeric input on the next submit
    /// (§7.4.1 F1/F4 per-request store).
    pub threshold: Option<u64>,
}

impl ExpandResultView {
    fn empty() -> Self {
        Self {
            raw: RawFold::default(),
            result: None,
            outcome: None,
            degraded_reason: None,
            tree_mode: false,
            threshold: None,
        }
    }

    /// The view for a failed call. The exchange the proxy kept rides along, so
    /// the raw fold shows the payload of the failure rather than going blank
    /// on it (#803).
    fn from_error(failure: &OpFailure) -> Self {
        let mut view = Self::empty();
        view.raw = RawFold::from_exchange(&failure.exchange);
        match &failure.error {
            UpstreamError::Outcome { outcome, .. } => view.outcome = Some((**outcome).clone()),
            UpstreamError::NotFound { .. } => {
                view.outcome = Some(OutcomeView {
                    severity: "error".to_owned(),
                    code: "not-found".to_owned(),
                    ..OutcomeView::default()
                })
            }
            UpstreamError::HttpStatus { status, .. } => {
                view.outcome = Some(OutcomeView {
                    severity: "error".to_owned(),
                    code: match *status {
                        400 => "invalid".to_owned(),
                        404 => "not-found".to_owned(),
                        422 => "too-costly".to_owned(),
                        _ => "unknown".to_owned(),
                    },
                    ..OutcomeView::default()
                });
            }
            UpstreamError::Connect { .. }
            | UpstreamError::Timeout { .. }
            | UpstreamError::ClientBuild { .. } => {
                view.degraded_reason = Some(failure.error.degraded_reason());
            }
            UpstreamError::Decode { message, .. } => {
                view.outcome = Some(OutcomeView::invalid_input(message.clone()));
            }
        }
        view
    }

    /// The `expansion.total`-based pager guard. Returns `true` when the
    /// result partial should render `[Load more]` (flat mode only,
    /// non-terminal page). Wraps [`ExpansionResult::has_more_flat`]
    /// so the template can call a single accessor per §7.4.1 F6.
    pub fn has_more_flat(&self) -> bool {
        match &self.result {
            Some(r) => !r.is_tree && r.has_more_flat(),
            None => false,
        }
    }
}

async fn expand_run(
    State(state): State<Arc<HtsUiState>>,
    Path(id): Path<String>,
    HxRequest(is_htmx): HxRequest,
    locale: RequestLocale,
    body: Bytes,
) -> Response {
    let form = parse_form(&body);
    let tree_mode = form_flag(&form, "mode", "tree");
    let mut params = ExpandParams {
        filter: opt(&form, "filter"),
        count: parse_u32(&form, "count"),
        offset: parse_u32(&form, "offset"),
        display_language: opt(&form, "displayLanguage"),
        active_only: Some(form_checkbox(&form, "activeOnly")),
        include_designations: Some(form_checkbox(&form, "includeDesignations")),
        use_supplement: multi(&form, "useSupplement"),
        date: opt(&form, "date"),
        property: multi(&form, "property"),
        tx_resource: multi(&form, "tx-resource"),
        system_version: multi(&form, "system-version"),
        check_system_version: multi(&form, "check-system-version"),
        force_system_version: multi(&form, "force-system-version"),
        default_valueset_version: opt(&form, "default-valueset-version"),
        hierarchical: None,
        exclude_nested: None,
        threshold: parse_u64(&form, "threshold"),
    };
    // §7.4.1 F7: exactly one of hierarchical / excludeNested emits.
    if tree_mode {
        params.hierarchical = Some(true);
    } else {
        params.exclude_nested = Some(true);
    }
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "value-sets",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    // §8.2 canonical-url contract: composite-stored VS resources cannot
    // be reached by base fhir id via /ValueSet/{id}/$expand. Resolve the
    // canonical url first, then use the type-level $expand pinned by
    // `url=<canonical>`. When the read fails (canonical empty) we fall
    // back to the instance-level call so mock-upstream integration
    // fixtures that only wire `/ValueSet/{id}/$expand` keep passing.
    let vs = state.upstream.read_value_set(&id).await;
    let canonical = vs.as_ref().map(|s| s.url.clone()).unwrap_or_default();
    let expand_result = if !canonical.is_empty() {
        state.upstream.vs_expand_by_url(&canonical, &params).await
    } else {
        state.upstream.vs_expand_instance(&id, &params).await
    };
    let view = match expand_result {
        Ok(result) => {
            let mut raw = RawFold::new(&result.request_url, &result.request_body, &result.raw_body);
            // Plan both incremental JSON panes (#898). Only possible when the
            // canonical url resolved — the fragment endpoints pin the
            // ValueSet via `url=<canonical>`.
            if !canonical.is_empty() {
                let req =
                    vs_expand_extra_query(&canonical, &params, tree_mode, PaneTarget::Request);
                let resp =
                    vs_expand_extra_query(&canonical, &params, tree_mode, PaneTarget::Response);
                raw.plan_panes(
                    vs_expand_fragment_endpoint(state.fhir_version, &req),
                    vs_expand_expand_url(&req),
                    vs_expand_fragment_endpoint(state.fhir_version, &resp),
                    vs_expand_expand_url(&resp),
                    &chrome.i18n,
                );
            }
            ExpandResultView {
                raw,
                tree_mode,
                threshold: params.threshold,
                result: Some(result),
                outcome: None,
                degraded_reason: None,
            }
        }
        Err(err) => {
            let mut v = ExpandResultView::from_error(&err);
            v.tree_mode = tree_mode;
            v.threshold = params.threshold;
            v
        }
    };
    respond_workbench(&state, chrome, id, view, is_htmx).await
}

async fn respond_workbench<'a>(
    state: &HtsUiState,
    chrome: Chrome<'a>,
    id: String,
    mut view: ExpandResultView,
    is_htmx: bool,
) -> Response {
    // Both response shapes render the same raw fold, so the payloads are
    // highlighted once, here, where the request locale is in hand.
    view.raw.highlight(&chrome.i18n);
    if is_htmx {
        return render(
            ExpandResultTemplate {
                chrome,
                id,
                view,
                ceiling: HTS_UI_MAX_EXPANSION_SIZE_HINT,
            }
            .render(),
        );
    }
    let detail = state.upstream.read_value_set(&id).await;
    render(
        DetailPageTemplate {
            chrome,
            id,
            detail,
            tab: VsTab::Expand,
            workbench: Some(view),
            ceiling: HTS_UI_MAX_EXPANSION_SIZE_HINT,
        }
        .render(),
    )
}

// ── Small helpers ───────────────────────────────────────────────────────

fn render(rendered: Result<String, askama::Error>) -> Response {
    match rendered {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!(?err, "hts-ui value-sets template render failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Html(String::from("<pre>hts-ui: template render error</pre>")),
            )
                .into_response()
        }
    }
}

/// Percent-decoded form body → multi-map. Standard HTML forms encode
/// repeated fields (e.g. `useSupplement=A&useSupplement=B`) as duplicate
/// keys; `serde_urlencoded` collapses those, so we go direct through
/// `form_urlencoded::parse` (§7.4.1 invariant #2 / §7.3.1 property
/// multi-map form-parsing note).
fn parse_form(body: &[u8]) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for (k, v) in form_urlencoded::parse(body) {
        map.entry(k.into_owned()).or_default().push(v.into_owned());
    }
    map
}

fn opt(form: &HashMap<String, Vec<String>>, key: &str) -> Option<String> {
    form.get(key)
        .and_then(|v| v.first())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn multi(form: &HashMap<String, Vec<String>>, key: &str) -> Vec<String> {
    form.get(key)
        .map(|v| v.iter().filter(|s| !s.trim().is_empty()).cloned().collect())
        .unwrap_or_default()
}

fn parse_u32(form: &HashMap<String, Vec<String>>, key: &str) -> Option<u32> {
    opt(form, key).and_then(|s| s.parse::<u32>().ok())
}

fn parse_u64(form: &HashMap<String, Vec<String>>, key: &str) -> Option<u64> {
    opt(form, key).and_then(|s| s.parse::<u64>().ok())
}

/// `true` when a form field equals a given expected value. Used to detect
/// the `mode=tree` toggle from the workbench form; the `flat` alternative
/// falls through as `false`.
fn form_flag(form: &HashMap<String, Vec<String>>, key: &str, expected: &str) -> bool {
    form.get(key)
        .and_then(|v| v.first())
        .map(|s| s.trim().eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

/// `true` when a checkbox is present in the form body (HTML forms only
/// submit checked boxes). Accepts any value; the mere presence of the
/// key is the signal.
fn form_checkbox(form: &HashMap<String, Vec<String>>, key: &str) -> bool {
    form.get(key)
        .and_then(|v| v.first())
        .map(|s| {
            let t = s.trim();
            !t.is_empty() && !t.eq_ignore_ascii_case("false") && !t.eq_ignore_ascii_case("off")
        })
        .unwrap_or(false)
}
