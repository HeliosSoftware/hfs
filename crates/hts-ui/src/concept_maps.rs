//! ConceptMap browser + detail with embedded `$translate` workbench
//! (design doc §7.5).
//!
//! The module registers five routes under `/hts/concept-maps`, mirroring
//! Slice C's `value_sets` shape (design doc §7.5 wireframe):
//!
//! - `GET  /hts/concept-maps`                    — full-page browser.
//! - `GET  /hts/concept-maps/rows`               — filter-form target
//!   (rows partial).
//! - `GET  /hts/concept-maps/{id}`               — 302 redirect to
//!   `/{id}/translate` (design doc §8.3: operation-first landing; the
//!   former "Metadata" tab is gone and the facts block is always
//!   visible above the tab strip).
//! - `GET  /hts/concept-maps/{id}/translate`     — Translate tab input
//!   partial (or full page on hard nav).
//! - `POST /hts/concept-maps/{id}/translate`     — runs `$translate` and
//!   returns the Translate result partial (or a full page on hard nav).
//!
//! `$translate` proxies to HTS as `POST /ConceptMap/{id}/$translate` per
//! design doc §7.6 proxy verb rule regardless of the source form verb.
//! §7.5 explicitly forbids exposing `version` (of the ConceptMap),
//! `dependency`, and the lowercase `targetsystem` alias in the form; the
//! [`UpstreamClient::cm_translate_instance`] emitter mirrors that in the
//! wire body.
//!
//! # Inline validation gate
//!
//! In `reverse` direction a submit without `targetCode` renders a synthetic
//! `OperationOutcome` in the result region without touching HTS — mirrors
//! the Slice B pre-flight validation pattern (`_count > MAX`) so the
//! operator gets legible feedback and the mock upstream records zero
//! incoming requests. Forward-direction submits require both `code` and
//! `system`; missing either fires the same gate.

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
    PaneTarget, cm_translate_expand_url, cm_translate_extra_query, cm_translate_fragment_endpoint,
};
use crate::upstream::{
    CmBrowserFilters, CmBrowserPage, ConceptMapSummary, OpFailure, OutcomeView, TranslateDirection,
    TranslateParams, TranslateResult, UpstreamError,
};
use crate::{Chrome, HtsUiState};

// ── Routing ─────────────────────────────────────────────────────────────

pub(crate) fn routes() -> Router<Arc<HtsUiState>> {
    Router::new()
        .route("/hts/concept-maps", get(browser_page))
        .route("/hts/concept-maps/rows", get(browser_rows))
        .route("/hts/concept-maps/{id}", get(detail_page))
        .route(
            "/hts/concept-maps/{id}/translate",
            get(translate_input).post(translate_run),
        )
}

/// Which detail-page tab a render targets. `Translate` is the only
/// variant today (design doc §8.3 — operation-first landing; the former
/// `Metadata` variant is gone). The `Translate` variant swaps in the
/// workbench input partial and, on POST, the result partial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmTab {
    Translate,
}

impl CmTab {
    /// URL-slug rendering; parallel to Slice B / C.
    #[allow(dead_code)]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Translate => "translate",
        }
    }
}

// ── Browser page (§7.5 mirror of §7.4 mirror of §7.2) ───────────────────

/// Query shape accepted by the browser page and its rows fragment.
///
/// `_count` and `_offset` are `Option<String>` so a malformed value
/// collapses to the defaults rather than tripping axum's
/// 400-on-deserialize-fail. §7.5 inherits Slice B invariant #1: `_count >
/// MAX_COUNT` clamps to 100 and surfaces an `OperationOutcome` in the
/// results region, not an HTTP 400.
///
/// Version-of-CM is deliberately absent from the filter strip: HTS does
/// not accept a ConceptMap-version pin on `$translate` and the browser
/// facet is not one of the operator-usable knobs (§7.5). `source` /
/// `target` are the URL-scoped `source-uri` / `target-uri` searches per
/// the FHIR CM SearchParameter list.
#[derive(Debug, Deserialize, Default)]
struct BrowserForm {
    url: Option<String>,
    name: Option<String>,
    title: Option<String>,
    source: Option<String>,
    target: Option<String>,
    status: Option<String>,
    #[serde(rename = "_count")]
    count: Option<String>,
    #[serde(rename = "_offset")]
    offset: Option<String>,
    // The language switcher hangs `?lang=` off any page; accept it silently.
    #[allow(dead_code)]
    lang: Option<String>,
}

impl BrowserForm {
    fn into_filters(self) -> CmBrowserFilters {
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
        CmBrowserFilters {
            url: non_empty(self.url),
            name: non_empty(self.name),
            title: non_empty(self.title),
            source: non_empty(self.source),
            target: non_empty(self.target),
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
#[template(path = "pages/cm-browser.html")]
struct BrowserPageTemplate<'a> {
    chrome: Chrome<'a>,
    view: BrowserRowsView,
}

#[derive(Template)]
#[template(path = "partials/hts-cm-rows.html")]
struct BrowserRowsTemplate<'a> {
    chrome: Chrome<'a>,
    view: BrowserRowsView,
}

/// Rows-partial data. Every arm renders legibly on its own so empty /
/// error / degraded states stay identical between hard nav and htmx swap.
struct BrowserRowsView {
    filters: CmBrowserFilters,
    result: Result<CmBrowserPage, UpstreamError>,
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
                CmBrowserFilters::MAX_COUNT
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

    fn page(&self) -> Option<&CmBrowserPage> {
        self.result.as_ref().ok()
    }

    /// See `code_systems::BrowserRowsView::status_url`. `source` /
    /// `target` are no longer offered as inputs (HTS drops them), but a
    /// hand-written URL that carries them still round-trips through the
    /// chips rather than being silently dropped here.
    fn status_url(&self, status: &str) -> String {
        let mut ser = form_urlencoded::Serializer::new(String::new());
        for (field, value) in [
            ("url", &self.filters.url),
            ("name", &self.filters.name),
            ("title", &self.filters.title),
            ("source", &self.filters.source),
            ("target", &self.filters.target),
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
            "/ui/hts/concept-maps".to_owned()
        } else {
            format!("/ui/hts/concept-maps?{query}")
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
        active_page: "concept-maps",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    render(BrowserPageTemplate { chrome, view }.render())
}

async fn browser_rows(
    State(state): State<Arc<HtsUiState>>,
    Query(form): Query<BrowserForm>,
    // Extracted so `AutoVaryLayer` marks the response `Vary: HX-Request`
    // even when the body is identical between htmx and hard nav (mirrors
    // Slice B / C rows fragments).
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    let view = load_browser_view(&state, form).await;
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "concept-maps",
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
            result: Ok(CmBrowserPage {
                rows: Vec::new(),
                filters: CmBrowserFilters::default(),
            }),
            count_over_max: true,
        };
    }
    let result = state.upstream.search_concept_maps(&filters).await;
    BrowserRowsView {
        filters,
        result,
        count_over_max: false,
    }
}

// ── Detail page (§7.5) ──────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/cm-detail.html")]
struct DetailPageTemplate<'a> {
    chrome: Chrome<'a>,
    id: String,
    detail: Result<ConceptMapSummary, UpstreamError>,
    tab: CmTab,
    /// Current direction seed for the Translate tab input, echoed back
    /// so a re-render preserves the operator's toggle position. `None`
    /// == no submit yet; the input falls back to `Forward`.
    direction: Option<TranslateDirection>,
    /// Populated only when this render is a Translate result. Keeps the
    /// `tab` field independent of the workbench state (Slice B / C).
    workbench: Option<TranslateResultView>,
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

    fn summary(&self) -> Option<&ConceptMapSummary> {
        self.detail.as_ref().ok()
    }

    fn effective_direction(&self) -> TranslateDirection {
        self.direction.unwrap_or_default()
    }
}

/// Base detail URL — permanent-redirects to the default operation tab
/// (§8.3 operation-first landing). The Translate handler renders the
/// full detail: facts block above the tab strip, Translate input as the
/// active tab.
async fn detail_page(Path(id): Path<String>) -> Response {
    Redirect::permanent(&format!("/ui/hts/concept-maps/{id}/translate")).into_response()
}

// ── Translate tab input (GET handler) ───────────────────────────────────

/// Query shape for `GET /translate`. `direction` swaps the visible field
/// group between forward (source `code` / `system` / `display`) and
/// reverse (`targetCode`); every other field renders in both modes.
#[derive(Debug, Deserialize, Default)]
struct TranslateInputForm {
    direction: Option<String>,
    #[allow(dead_code)]
    lang: Option<String>,
}

// Tabs are wrapped in `#hts-cm-detail-region` (design doc §8.1); a tab
// click uses `hx-select="#hts-cm-detail-region"` so we always render the
// full detail page and htmx picks the region out.

async fn translate_input(
    State(state): State<Arc<HtsUiState>>,
    Path(id): Path<String>,
    Query(form): Query<TranslateInputForm>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "concept-maps",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    let direction = TranslateDirection::from_form(form.direction.as_deref());
    let summary = state.upstream.read_concept_map(&id).await.ok();
    render_detail_with_tab(&state, chrome, id, CmTab::Translate, direction, summary).await
}

async fn render_detail_with_tab<'a>(
    state: &HtsUiState,
    chrome: Chrome<'a>,
    id: String,
    tab: CmTab,
    direction: TranslateDirection,
    prefetched: Option<ConceptMapSummary>,
) -> Response {
    let detail = match prefetched {
        Some(s) => Ok(s),
        None => state.upstream.read_concept_map(&id).await,
    };
    render(
        DetailPageTemplate {
            chrome,
            id,
            detail,
            tab,
            direction: Some(direction),
            workbench: None,
        }
        .render(),
    )
}

// ── Translate run (POST handler) ────────────────────────────────────────

#[derive(Template)]
#[template(path = "partials/hts-cm-translate-result.html")]
struct TranslateResultTemplate<'a> {
    chrome: Chrome<'a>,
    view: TranslateResultView,
}

/// Payload for the Translate result partial. Per-op (mirrors Slice C
/// `ExpandResultView`); the abstract concept renderer stays aspirational
/// until enough operations demand a cross-slice refactor.
#[derive(Clone, Debug)]
pub struct TranslateResultView {
    /// What went over the wire, for the "Raw request and response" fold.
    pub raw: RawFold,
    pub result: Option<TranslateResult>,
    pub outcome: Option<OutcomeView>,
    pub degraded_reason: Option<&'static str>,
    /// The direction the operator submitted, echoed for the input
    /// partial's radio state on the next re-render.
    pub direction: TranslateDirection,
}

impl TranslateResultView {
    fn empty(direction: TranslateDirection) -> Self {
        Self {
            raw: RawFold::default(),
            result: None,
            outcome: None,
            degraded_reason: None,
            direction,
        }
    }

    /// The view for a failed call. The exchange the proxy kept rides along, so
    /// the raw fold shows the payload of the failure rather than going blank
    /// on it (#803).
    fn from_error(direction: TranslateDirection, failure: &OpFailure) -> Self {
        let mut view = Self::empty(direction);
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
}

async fn translate_run(
    State(state): State<Arc<HtsUiState>>,
    Path(id): Path<String>,
    HxRequest(is_htmx): HxRequest,
    locale: RequestLocale,
    body: Bytes,
) -> Response {
    let form = parse_form(&body);
    let direction = TranslateDirection::from_form(opt(&form, "direction").as_deref());
    let params = TranslateParams {
        direction,
        code: opt(&form, "code"),
        system: opt(&form, "system"),
        display: opt(&form, "display"),
        target_code: opt(&form, "targetCode"),
        target_system: opt(&form, "targetSystem"),
        source_url: opt(&form, "source"),
        target_url: opt(&form, "target"),
        date: opt(&form, "date"),
    };
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "concept-maps",
        fhir_version: state.fhir_version,
        version: state.version,
    };

    // Pre-flight validation gate (§7.5 states matrix): render an inline
    // `OperationOutcome` without touching HTS. Mirrors Slice B's
    // `_count > MAX` pattern: the mock upstream records zero incoming
    // requests, and the operator gets a legible error above the form.
    if let Some(diagnostics) = validate_pre_flight(&params) {
        let mut view = TranslateResultView::empty(direction);
        view.outcome = Some(OutcomeView::invalid_input(diagnostics));
        return respond_workbench(&state, chrome, id, view, is_htmx).await;
    }

    // §8.2 canonical-url contract: composite-stored CM resources cannot
    // be reached by base fhir id via /ConceptMap/{id}/$translate. Resolve
    // the canonical url first, then use the type-level $translate pinned
    // by `url=<canonical>`. When the read fails (canonical empty) we
    // fall back to the instance-level call so mock-upstream fixtures
    // that only wire `/ConceptMap/{id}/$translate` keep passing.
    let cm = state.upstream.read_concept_map(&id).await;
    let canonical = cm.as_ref().map(|s| s.url.clone()).unwrap_or_default();
    let translate_result = if !canonical.is_empty() {
        state
            .upstream
            .cm_translate_by_url(&canonical, &params)
            .await
    } else {
        state.upstream.cm_translate_instance(&id, &params).await
    };
    let view = match translate_result {
        Ok(result) => {
            let mut raw = RawFold::new(&result.request_url, &result.request_body, &result.raw_body);
            // Plan both incremental JSON panes (#898). Only possible when the
            // canonical url resolved — the fragment endpoints pin the
            // ConceptMap via `url=<canonical>`.
            if !canonical.is_empty() {
                let req = cm_translate_extra_query(&canonical, &params, PaneTarget::Request);
                let resp = cm_translate_extra_query(&canonical, &params, PaneTarget::Response);
                raw.plan_panes(
                    cm_translate_fragment_endpoint(state.fhir_version, &req),
                    cm_translate_expand_url(&req),
                    cm_translate_fragment_endpoint(state.fhir_version, &resp),
                    cm_translate_expand_url(&resp),
                    &chrome.i18n,
                );
            }
            TranslateResultView {
                raw,
                direction,
                result: Some(result),
                outcome: None,
                degraded_reason: None,
            }
        }
        Err(err) => TranslateResultView::from_error(direction, &err),
    };
    respond_workbench(&state, chrome, id, view, is_htmx).await
}

/// Pre-flight validation (§7.5 states matrix). Returns `Some(diagnostics)`
/// when the submit is missing a required field for its direction — the
/// caller renders the inline validation outcome and never touches HTS.
/// Returns `None` when the request is safe to forward.
fn validate_pre_flight(params: &TranslateParams) -> Option<String> {
    match params.direction {
        TranslateDirection::Forward => {
            let code = params.code.as_deref().unwrap_or_default().trim();
            let system = params.system.as_deref().unwrap_or_default().trim();
            if code.is_empty() || system.is_empty() {
                Some("Forward translation requires both `code` and `system`.".to_owned())
            } else {
                None
            }
        }
        TranslateDirection::Reverse => {
            let target = params.target_code.as_deref().unwrap_or_default().trim();
            if target.is_empty() {
                Some("Reverse translation requires `targetCode`.".to_owned())
            } else {
                None
            }
        }
    }
}

async fn respond_workbench<'a>(
    state: &HtsUiState,
    chrome: Chrome<'a>,
    id: String,
    mut view: TranslateResultView,
    is_htmx: bool,
) -> Response {
    // Both response shapes render the same raw fold, so the payloads are
    // highlighted once, here, where the request locale is in hand.
    view.raw.highlight(&chrome.i18n);
    if is_htmx {
        return render(TranslateResultTemplate { chrome, view }.render());
    }
    let detail = state.upstream.read_concept_map(&id).await;
    let direction = view.direction;
    render(
        DetailPageTemplate {
            chrome,
            id,
            detail,
            tab: CmTab::Translate,
            direction: Some(direction),
            workbench: Some(view),
        }
        .render(),
    )
}

// ── Small helpers ───────────────────────────────────────────────────────

fn render(rendered: Result<String, askama::Error>) -> Response {
    match rendered {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!(?err, "hts-ui concept-maps template render failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Html(String::from("<pre>hts-ui: template render error</pre>")),
            )
                .into_response()
        }
    }
}

/// Percent-decoded form body → multi-map. Slice B invariant #2 / Slice C
/// invariant: repeated keys must survive (design doc §7.5 nojs contract
/// keeps repeatable inputs on the form).
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
