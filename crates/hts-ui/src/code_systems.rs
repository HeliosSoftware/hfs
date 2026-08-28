//! CodeSystem browser + detail with embedded workbench (design doc §7.2 / §7.3).
//!
//! The module registers nine routes under `/hts/code-systems`:
//!
//! - `GET /hts/code-systems` — full-page browser.
//! - `GET /hts/code-systems/rows` — filter-form target (rows partial).
//! - `GET /hts/code-systems/{id}` — 302 redirect to `/{id}/lookup` (design
//!   doc §8.3: operation-first landing; the former "Metadata" tab is gone
//!   and the facts block is always visible above the tab strip).
//! - `GET /hts/code-systems/{id}/{op}` — Lookup / Validate / Subsumes tabs
//!   (full page on hard nav, workbench input partial on htmx request).
//! - `POST /hts/code-systems/{id}/{op}` — runs the operation and returns
//!   the shared workbench-result partial (or a full page on hard nav).
//!
//! Every operation proxies to HTS **as POST** per design doc §7.6 proxy verb
//! rule, regardless of whether the source UI form was a GET (the rows
//! fragment) or a POST (workbench submit). Search / read stay GET.

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
use crate::upstream::{
    CodeSystemSummary, CsBrowserFilters, CsBrowserPage, LookupParams, LookupResult, OutcomeView,
    SubsumesParams, SubsumesResult, UpstreamError, ValidateCodeParams, ValidateCodeResult,
    ValidateInputMode,
};
use crate::{Chrome, HtsUiState};

// ── Routing ─────────────────────────────────────────────────────────────

pub(crate) fn routes() -> Router<Arc<HtsUiState>> {
    Router::new()
        .route("/hts/code-systems", get(browser_page))
        .route("/hts/code-systems/rows", get(browser_rows))
        .route("/hts/code-systems/{id}", get(detail_page))
        .route(
            "/hts/code-systems/{id}/lookup",
            get(lookup_input).post(lookup_run),
        )
        .route(
            "/hts/code-systems/{id}/validate",
            get(validate_input).post(validate_run),
        )
        .route(
            "/hts/code-systems/{id}/subsumes",
            get(subsumes_input).post(subsumes_run),
        )
}

/// Which workbench tab a detail-page render targets. `Lookup` is the
/// default (design doc §8.3 — operation-first landing); the former
/// `Metadata` variant is gone and the facts block is always rendered
/// above the tab strip regardless of which operation is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CsTab {
    Lookup,
    Validate,
    Subsumes,
}

impl CsTab {
    /// URL-slug rendering of the tab. Kept alongside the enum so future
    /// template additions (per-op heading, telemetry label) can reuse it
    /// without re-deriving the match; unused today because the templates
    /// branch on the concrete `view.lookup` / `view.validate` /
    /// `view.subsumes` Option arms.
    #[allow(dead_code)]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Lookup => "lookup",
            Self::Validate => "validate",
            Self::Subsumes => "subsumes",
        }
    }
}

// ── Browser page (§7.2) ─────────────────────────────────────────────────

/// Query shape accepted by the browser page and its rows fragment.
///
/// `_count` and `_offset` are `Option<String>` so a malformed value collapses
/// to the defaults rather than tripping axum's 400-on-deserialize-fail. This
/// matches the design doc §7.2 decision to clamp rather than reject when the
/// input is not `_count > 100` (see [`browser_rejects_over_max_count`] in
/// `tests/code_systems.rs`).
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
    // The language switcher hangs `?lang=` off any page; accept it silently.
    #[allow(dead_code)]
    lang: Option<String>,
}

impl BrowserForm {
    fn into_filters(self) -> CsBrowserFilters {
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
        CsBrowserFilters {
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
#[template(path = "pages/cs-browser.html")]
struct BrowserPageTemplate<'a> {
    chrome: Chrome<'a>,
    view: BrowserRowsView,
}

#[derive(Template)]
#[template(path = "partials/hts-cs-rows.html")]
struct BrowserRowsTemplate<'a> {
    chrome: Chrome<'a>,
    view: BrowserRowsView,
}

/// The rows partial's data. Every arm is designed to render legibly on its
/// own — the browser page reuses the same partial after its initial full
/// render so the empty / error / degraded states stay identical between
/// hard nav and htmx swap (design doc §7.2 states matrix).
struct BrowserRowsView {
    filters: CsBrowserFilters,
    /// The rows result — `Ok` for a normal page (possibly empty), `Err` for
    /// an OperationOutcome / connection failure that the template renders
    /// above the (empty) table.
    result: Result<CsBrowserPage, UpstreamError>,
    /// True when `_count` exceeded the hard cap; the handler surfaces this
    /// as a pre-flight invalid-input OperationOutcome and does not call
    /// HTS.
    count_over_max: bool,
}

impl BrowserRowsView {
    fn degraded_reason(&self) -> Option<&'static str> {
        match &self.result {
            Err(e) if matches!(e, UpstreamError::Connect { .. } | UpstreamError::Timeout { .. } | UpstreamError::ClientBuild { .. }) => {
                Some(e.degraded_reason())
            }
            _ => None,
        }
    }

    fn outcome(&self) -> Option<OutcomeView> {
        if self.count_over_max {
            return Some(OutcomeView::invalid_input(format!(
                "_count must be between 1 and {}",
                CsBrowserFilters::MAX_COUNT
            )));
        }
        match &self.result {
            Err(UpstreamError::Outcome { outcome, .. }) => Some(outcome.clone()),
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

    fn page(&self) -> Option<&CsBrowserPage> {
        self.result.as_ref().ok()
    }

    /// Href for one status facet chip. The chips are plain links (they
    /// work with JS off and are the only filter that is not a form
    /// control), so each one has to carry the active text filters
    /// forward; `_offset` is deliberately dropped so switching status
    /// restarts paging. An empty `status` is the "any status" chip.
    ///
    /// Built with `form_urlencoded` rather than string concatenation so a
    /// filter value containing `&` / spaces / unicode cannot inject query
    /// parameters (same reasoning as `CsBrowserFilters::load_more_url`).
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
            "/ui/hts/code-systems".to_owned()
        } else {
            format!("/ui/hts/code-systems?{query}")
        }
    }

    /// Whether `status` is the chip currently selected — drives
    /// `aria-current="true"`. The empty string matches "no status filter".
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
        active_page: "code-systems",
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
    // hard nav (mirrors Slice A's dashboard-cards fragment).
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    let view = load_browser_view(&state, form).await;
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "code-systems",
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
            result: Ok(CsBrowserPage {
                rows: Vec::new(),
                filters: CsBrowserFilters::default(),
            }),
            count_over_max: true,
        };
    }
    let result = state.upstream.search_code_systems(&filters).await;
    BrowserRowsView {
        filters,
        result,
        count_over_max: false,
    }
}

// ── Detail page (§7.3) ──────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/cs-detail.html")]
struct DetailPageTemplate<'a> {
    chrome: Chrome<'a>,
    id: String,
    detail: Result<CodeSystemSummary, UpstreamError>,
    tab: CsTab,
    /// Populated only when this render is a workbench result. Kept
    /// separate from `tab` so the template can distinguish "tab pre-loaded
    /// with the input form" (`workbench = None`) from "tab pre-populated
    /// with a POST result" (`workbench = Some(view)`).
    workbench: Option<WorkbenchResultView>,
}

impl<'a> DetailPageTemplate<'a> {
    fn degraded_reason(&self) -> Option<&'static str> {
        match &self.detail {
            Err(e) if matches!(
                e,
                UpstreamError::Connect { .. } | UpstreamError::Timeout { .. } | UpstreamError::ClientBuild { .. }
            ) => Some(e.degraded_reason()),
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
            Err(UpstreamError::Outcome { outcome, .. }) => Some(outcome.clone()),
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

    fn summary(&self) -> Option<&CodeSystemSummary> {
        self.detail.as_ref().ok()
    }
}

/// Base detail URL — permanent-redirects to the default operation tab
/// (§8.3 operation-first landing). The Lookup handler renders the full
/// detail: facts block above the tab strip, Lookup input as the active
/// tab. Redirecting instead of rendering keeps the browser URL and the
/// `aria-current` tab always in sync.
async fn detail_page(Path(id): Path<String>) -> Response {
    Redirect::permanent(&format!("/ui/hts/code-systems/{id}/lookup")).into_response()
}

// ── Workbench inputs (GET tab handlers) ─────────────────────────────────
//
// The tabs are wrapped in `#hts-cs-detail-region` (design doc §8.1) and
// each tab click uses `hx-select="#hts-cs-detail-region"`, so the handler
// always renders the full detail page and htmx picks the region out. No
// separate htmx-fragment branch — that used to leave the tabs strip with
// a stale `aria-current` attribute (Bug 1 in the tab-active-state fix).

async fn lookup_input(
    State(state): State<Arc<HtsUiState>>,
    Path(id): Path<String>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "code-systems",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    let summary = state.upstream.read_code_system(&id).await.ok();
    render_detail_with_tab(&state, chrome, id, CsTab::Lookup, summary).await
}

async fn validate_input(
    State(state): State<Arc<HtsUiState>>,
    Path(id): Path<String>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "code-systems",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    let summary = state.upstream.read_code_system(&id).await.ok();
    render_detail_with_tab(&state, chrome, id, CsTab::Validate, summary).await
}

async fn subsumes_input(
    State(state): State<Arc<HtsUiState>>,
    Path(id): Path<String>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "code-systems",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    let summary = state.upstream.read_code_system(&id).await.ok();
    render_detail_with_tab(&state, chrome, id, CsTab::Subsumes, summary).await
}

async fn render_detail_with_tab<'a>(
    state: &HtsUiState,
    chrome: Chrome<'a>,
    id: String,
    tab: CsTab,
    prefetched: Option<CodeSystemSummary>,
) -> Response {
    let detail = match prefetched {
        Some(s) => Ok(s),
        None => state.upstream.read_code_system(&id).await,
    };
    render(
        DetailPageTemplate {
            chrome,
            id,
            detail,
            tab,
            workbench: None,
        }
        .render(),
    )
}

// ── Workbench runs (POST handlers) ──────────────────────────────────────

#[derive(Template)]
#[template(path = "partials/hts-cs-workbench-result.html")]
struct WorkbenchResultTemplate<'a> {
    chrome: Chrome<'a>,
    view: WorkbenchResultView,
}

/// Payload for the shared workbench result partial. Options rather than an
/// enum so the template can branch on `Some(_)` per op without importing
/// custom types (matches Slice A's `Result` matching style in the
/// dashboard-cards partial).
#[derive(Clone, Debug)]
pub struct WorkbenchResultView {
    /// The op that produced this result. Kept on the view so Slice E's
    /// standalone operations workbench can render a per-op heading over
    /// the shared partial; the CS detail template already knows which tab
    /// is active from its own `tab` field, so it does not read this
    /// today. Removing it would force Slice E to re-thread the op down
    /// through a wrapper struct — the doc-comment cost is smaller.
    #[allow(dead_code)]
    pub op: CsTab,
    pub request_url: String,
    pub raw_body: String,
    pub lookup: Option<LookupResult>,
    pub validate: Option<ValidateCodeResult>,
    pub subsumes: Option<SubsumesResult>,
    pub outcome: Option<OutcomeView>,
    pub degraded_reason: Option<&'static str>,
}

impl WorkbenchResultView {
    fn empty(op: CsTab) -> Self {
        Self {
            op,
            request_url: String::new(),
            raw_body: String::new(),
            lookup: None,
            validate: None,
            subsumes: None,
            outcome: None,
            degraded_reason: None,
        }
    }

    fn from_error(op: CsTab, request_url: String, err: &UpstreamError) -> Self {
        let mut view = Self::empty(op);
        view.request_url = request_url;
        match err {
            UpstreamError::Outcome { outcome, .. } => view.outcome = Some(outcome.clone()),
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
                view.degraded_reason = Some(err.degraded_reason());
            }
            UpstreamError::Decode { message, .. } => {
                view.outcome = Some(OutcomeView::invalid_input(message.clone()));
            }
        }
        view
    }
}

async fn lookup_run(
    State(state): State<Arc<HtsUiState>>,
    Path(id): Path<String>,
    HxRequest(is_htmx): HxRequest,
    locale: RequestLocale,
    body: Bytes,
) -> Response {
    let form = parse_form(&body);
    let params = LookupParams {
        code: single(&form, "code"),
        version: opt(&form, "version"),
        display_language: opt(&form, "displayLanguage"),
        properties: multi(&form, "property"),
        date: opt(&form, "date"),
    };
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "code-systems",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    // §8.2 canonical-url contract: the HTS backend stores resources with
    // composite ids (`{fhir_id}|{version}`) so the instance route
    // `/CodeSystem/{id}/$lookup` misses when passed the base fhir id.
    // Resolve the canonical url first, then delegate to the type-level
    // `$lookup` that pins the CS via `system=<canonical>`. If the read
    // fails (composite unknown, closed-loopback test upstream, transient
    // 5xx) we fall back to the instance-level call so the operator still
    // sees an actionable outcome — mirrors validate/subsumes semantics
    // and preserves closed-loopback pre-flight tests that never touch
    // canonical resolution.
    let cs = state.upstream.read_code_system(&id).await;
    let canonical = cs.as_ref().map(|s| s.url.clone()).unwrap_or_default();
    let view = if params.code.trim().is_empty() {
        WorkbenchResultView {
            outcome: Some(OutcomeView::invalid_input("code is required".to_string())),
            ..WorkbenchResultView::empty(CsTab::Lookup)
        }
    } else {
        let result = if !canonical.is_empty() {
            state.upstream.cs_lookup_type_level(&canonical, params).await
        } else {
            state.upstream.cs_lookup(&id, params).await
        };
        match result {
            Ok(result) => WorkbenchResultView {
                op: CsTab::Lookup,
                request_url: result.request_url.clone(),
                raw_body: result.raw_body.clone(),
                lookup: Some(result),
                validate: None,
                subsumes: None,
                outcome: None,
                degraded_reason: None,
            },
            Err(err) => {
                let request_url = if !canonical.is_empty() {
                    format!("{}/CodeSystem/$lookup", state.upstream.base_url())
                } else {
                    format!("{}/CodeSystem/{}/$lookup", state.upstream.base_url(), id)
                };
                WorkbenchResultView::from_error(CsTab::Lookup, request_url, &err)
            }
        }
    };
    respond_workbench(&state, chrome, id, CsTab::Lookup, view, is_htmx).await
}

async fn validate_run(
    State(state): State<Arc<HtsUiState>>,
    Path(id): Path<String>,
    HxRequest(is_htmx): HxRequest,
    locale: RequestLocale,
    body: Bytes,
) -> Response {
    let form = parse_form(&body);
    let params = ValidateCodeParams {
        mode: ValidateInputMode::from_form(form.get("mode").and_then(|v| v.first()).map(String::as_str)),
        code: single(&form, "code"),
        display: opt(&form, "display"),
        coding_system: single(&form, "coding.system"),
        coding_code: single(&form, "coding.code"),
        coding_display: opt(&form, "coding.display"),
        display_language: opt(&form, "displayLanguage"),
    };
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "code-systems",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    // The Validate tab has no CS instance route (hts-details.md §CS
    // `$validate-code`), so we resolve the canonical URL from a fresh read
    // of the CS; this is the same call the tab's input partial performs.
    let cs = state.upstream.read_code_system(&id).await;
    let canonical = cs.as_ref().map(|s| s.url.clone()).unwrap_or_default();
    let view = if canonical.is_empty() {
        WorkbenchResultView {
            outcome: Some(OutcomeView::invalid_input(
                "CodeSystem canonical url unavailable".to_string(),
            )),
            ..WorkbenchResultView::empty(CsTab::Validate)
        }
    } else if matches!(params.mode, ValidateInputMode::Code) && params.code.trim().is_empty() {
        WorkbenchResultView {
            outcome: Some(OutcomeView::invalid_input("code is required".to_string())),
            ..WorkbenchResultView::empty(CsTab::Validate)
        }
    } else if matches!(params.mode, ValidateInputMode::Coding)
        && (params.coding_code.trim().is_empty() || params.coding_system.trim().is_empty())
    {
        WorkbenchResultView {
            outcome: Some(OutcomeView::invalid_input(
                "coding.system and coding.code are required".to_string(),
            )),
            ..WorkbenchResultView::empty(CsTab::Validate)
        }
    } else {
        match state.upstream.cs_validate_code(&canonical, params).await {
            Ok(result) => WorkbenchResultView {
                op: CsTab::Validate,
                request_url: result.request_url.clone(),
                raw_body: result.raw_body.clone(),
                lookup: None,
                validate: Some(result),
                subsumes: None,
                outcome: None,
                degraded_reason: None,
            },
            Err(err) => {
                let request_url = format!(
                    "{}/CodeSystem/$validate-code",
                    state.upstream.base_url()
                );
                WorkbenchResultView::from_error(CsTab::Validate, request_url, &err)
            }
        }
    };
    respond_workbench(&state, chrome, id, CsTab::Validate, view, is_htmx).await
}

async fn subsumes_run(
    State(state): State<Arc<HtsUiState>>,
    Path(id): Path<String>,
    HxRequest(is_htmx): HxRequest,
    locale: RequestLocale,
    body: Bytes,
) -> Response {
    let form = parse_form(&body);
    let params = SubsumesParams {
        code_a: single(&form, "codeA"),
        code_b: single(&form, "codeB"),
        version: opt(&form, "version"),
    };
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "code-systems",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    let cs = state.upstream.read_code_system(&id).await;
    let canonical = cs.as_ref().map(|s| s.url.clone()).unwrap_or_default();
    let view = if canonical.is_empty() {
        WorkbenchResultView {
            outcome: Some(OutcomeView::invalid_input(
                "CodeSystem canonical url unavailable".to_string(),
            )),
            ..WorkbenchResultView::empty(CsTab::Subsumes)
        }
    } else if params.code_a.trim().is_empty() || params.code_b.trim().is_empty() {
        WorkbenchResultView {
            outcome: Some(OutcomeView::invalid_input(
                "codeA and codeB are required".to_string(),
            )),
            ..WorkbenchResultView::empty(CsTab::Subsumes)
        }
    } else {
        match state.upstream.cs_subsumes(&canonical, params).await {
            Ok(result) => WorkbenchResultView {
                op: CsTab::Subsumes,
                request_url: result.request_url.clone(),
                raw_body: result.raw_body.clone(),
                lookup: None,
                validate: None,
                subsumes: Some(result),
                outcome: None,
                degraded_reason: None,
            },
            Err(err) => {
                let request_url = format!("{}/CodeSystem/$subsumes", state.upstream.base_url());
                WorkbenchResultView::from_error(CsTab::Subsumes, request_url, &err)
            }
        }
    };
    respond_workbench(&state, chrome, id, CsTab::Subsumes, view, is_htmx).await
}

async fn respond_workbench<'a>(
    state: &HtsUiState,
    chrome: Chrome<'a>,
    id: String,
    tab: CsTab,
    view: WorkbenchResultView,
    is_htmx: bool,
) -> Response {
    if is_htmx {
        return render(WorkbenchResultTemplate { chrome, view }.render());
    }
    let detail = state.upstream.read_code_system(&id).await;
    render(
        DetailPageTemplate {
            chrome,
            id,
            detail,
            tab,
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
            tracing::error!(?err, "hts-ui code-systems template render failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Html(String::from("<pre>hts-ui: template render error</pre>")),
            )
                .into_response()
        }
    }
}

/// Percent-decoded form body → multi-map. Standard HTML forms encode
/// repeated fields (e.g. `property=parent&property=child`) as duplicate
/// keys; `serde_urlencoded` collapses those, so we go direct through
/// `form_urlencoded::parse`.
fn parse_form(body: &[u8]) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for (k, v) in form_urlencoded::parse(body) {
        map.entry(k.into_owned()).or_default().push(v.into_owned());
    }
    map
}

fn single(form: &HashMap<String, Vec<String>>, key: &str) -> String {
    form.get(key)
        .and_then(|v| v.first())
        .cloned()
        .unwrap_or_default()
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
