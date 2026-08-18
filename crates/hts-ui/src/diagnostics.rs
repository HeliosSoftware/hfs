//! Slice G — standalone Diagnostics page (design doc §7.9).
//!
//! Two routes under `/hts/diagnostics`:
//!
//! - `GET /hts/diagnostics` — full-page shell with a 4-tab strip
//!   (Capability, TerminologyCap, /health, /metrics). The tab named by
//!   `?tab=` (default `capability`) is pre-rendered inside
//!   `#diag-panel` so nojs and deep-link deliveries work identically to
//!   the htmx-driven swap (design doc §7.10 row 7.9 nojs contract).
//! - `GET /hts/diagnostics/panel` — the tabpanel fragment target for the
//!   `hx-get` tab swap. Reads `?tab=` and renders the shared panel
//!   partial with the right variant flag set.
//!
//! **Per-tab isolation.** A 5xx / connect / decode failure on one tab
//! renders `partials/hts-outcome.html` *inside* `#diag-panel` — the
//! other tab links stay intact so the operator can navigate away from
//! the failing surface. The full-page shell still renders the shared
//! degraded banner (§7 preamble) when the *initial* `/health` probe on
//! GET `/hts/diagnostics` fails; the panel route deliberately does
//! not, so an htmx-driven tab swap can never blank the shell.

use askama::Template;
use axum::{
    Router,
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use axum_htmx::HxRequest;
use serde::Deserialize;
use std::sync::Arc;

use crate::i18n::{I18n, RequestLocale};
use crate::upstream::{
    CapabilityView, OutcomeView, TerminologyCapabilitiesView, UpstreamError, UpstreamHealth,
};
use crate::{Chrome, HtsUiState};

// ── Routing ─────────────────────────────────────────────────────────────

pub(crate) fn routes() -> Router<Arc<HtsUiState>> {
    Router::new()
        .route("/hts/diagnostics", get(diagnostics_page))
        .route("/hts/diagnostics/panel", get(diagnostics_panel))
}

// ── Tab enum + query ────────────────────────────────────────────────────

/// The four diagnostic surfaces the page exposes. Unknown / missing
/// `?tab=` collapses to `Capability` (design doc §7.9 — default tab).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Capability,
    TerminologyCapabilities,
    Health,
    Metrics,
}

impl Tab {
    fn from_slug(slug: &str) -> Self {
        match slug {
            "terminology-capabilities" => Self::TerminologyCapabilities,
            "health" => Self::Health,
            "metrics" => Self::Metrics,
            _ => Self::Capability,
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Capability => "capability",
            Self::TerminologyCapabilities => "terminology-capabilities",
            Self::Health => "health",
            Self::Metrics => "metrics",
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            Self::Capability => "hts-diagnostics-tab-capability",
            Self::TerminologyCapabilities => "hts-diagnostics-tab-terminology-capabilities",
            Self::Health => "hts-diagnostics-tab-health",
            Self::Metrics => "hts-diagnostics-tab-metrics",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct TabQuery {
    tab: Option<String>,
}

// ── Templates ───────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/diagnostics.html")]
struct DiagnosticsPageTemplate<'a> {
    chrome: Chrome<'a>,
    tabs: Vec<TabEntry>,
    panel: PanelView,
    /// Reason key for the shared degraded banner. `Some(_)` when the
    /// upstream `/health` probe fails on the initial page GET —
    /// mirrors the guard used by every other §7 page.
    degraded_reason: Option<&'static str>,
}

#[derive(Template)]
#[template(path = "partials/hts-diagnostics-panel.html")]
struct DiagnosticsPanelTemplate<'a> {
    chrome: Chrome<'a>,
    panel: PanelView,
}

/// One entry in the tab strip. `active` drives `aria-selected="true"`;
/// the label is the Fluent key so the template can localise it.
#[derive(Clone, Debug)]
struct TabEntry {
    slug: &'static str,
    label_key: &'static str,
    active: bool,
}

fn tab_entries(active: Tab) -> Vec<TabEntry> {
    [
        Tab::Capability,
        Tab::TerminologyCapabilities,
        Tab::Health,
        Tab::Metrics,
    ]
    .iter()
    .map(|t| TabEntry {
        slug: t.slug(),
        label_key: t.label_key(),
        active: *t == active,
    })
    .collect()
}

/// Data driving `hts-diagnostics-panel.html`. Askama branches on the
/// `is_*` flags — same idiom as Slice E1's `OpsFlags` and Slice F's
/// `StatusView` — so the template never needs to import [`Tab`].
///
/// Every tab populates at most one of `capability` / `terminology` /
/// `health` / `metrics`. On upstream failure the tab still marks its
/// discriminator flag but sets `outcome` instead — the partial then
/// renders `hts-outcome.html` inside the panel without disturbing the
/// tab strip (per-tab isolation contract from §7.9).
#[derive(Clone, Debug, Default)]
struct PanelView {
    is_capability: bool,
    is_terminology_capabilities: bool,
    is_health: bool,
    is_metrics: bool,
    capability: Option<CapabilityView>,
    terminology: Option<TerminologyCapabilitiesView>,
    health: Option<UpstreamHealth>,
    /// Raw Prometheus text-format body. `Some("")` renders the neutral
    /// empty state (`hts-diagnostics-metrics-empty`) rather than an
    /// error; `None` here means the tab is not `/metrics`.
    metrics: Option<String>,
    outcome: Option<OutcomeView>,
}

impl PanelView {
    fn empty(tab: Tab) -> Self {
        Self {
            is_capability: matches!(tab, Tab::Capability),
            is_terminology_capabilities: matches!(tab, Tab::TerminologyCapabilities),
            is_health: matches!(tab, Tab::Health),
            is_metrics: matches!(tab, Tab::Metrics),
            ..Self::default()
        }
    }
}

// ── Panel builder (shared by page + panel handlers) ────────────────────

async fn build_panel(state: &HtsUiState, tab: Tab) -> PanelView {
    let mut view = PanelView::empty(tab);
    match tab {
        Tab::Capability => match state.upstream.capability_statement().await {
            Ok(cap) => view.capability = Some(cap),
            Err(err) => view.outcome = Some(outcome_from_error(&err)),
        },
        Tab::TerminologyCapabilities => {
            match state.upstream.terminology_capabilities_view().await {
                Ok(tc) => view.terminology = Some(tc),
                Err(err) => view.outcome = Some(outcome_from_error(&err)),
            }
        }
        Tab::Health => match state.upstream.health().await {
            Ok(h) => view.health = Some(h),
            Err(err) => view.outcome = Some(outcome_from_error(&err)),
        },
        Tab::Metrics => match state.upstream.metrics_text().await {
            Ok(text) => view.metrics = Some(text),
            Err(err) => view.outcome = Some(outcome_from_error(&err)),
        },
    }
    view
}

/// Build a synthetic [`OutcomeView`] from an [`UpstreamError`]. The
/// diagnostic string is the error's `Display` output (which already
/// carries the `op` + `url` + status/message from the [`UpstreamError`]
/// variants); the shared partial handles severity and code rendering.
///
/// Codes are constrained to the set the shared `hts-outcome-code-*`
/// Fluent block already covers (`not-found`, `invalid`, `too-costly`,
/// `unknown`) so no raw key leaks into the rendered banner. Slice G
/// adds no new outcome codes.
fn outcome_from_error(err: &UpstreamError) -> OutcomeView {
    let code = match err {
        UpstreamError::NotFound { .. } => "not-found",
        _ => "unknown",
    };
    OutcomeView {
        severity: "error".to_owned(),
        code: code.to_owned(),
        diagnostics: err.to_string(),
        location: Vec::new(),
        request_id: None,
    }
}

// ── GET /hts/diagnostics ────────────────────────────────────────────────

async fn diagnostics_page(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
    Query(query): Query<TabQuery>,
) -> Response {
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "diagnostics",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    let active_tab = Tab::from_slug(query.tab.as_deref().unwrap_or(""));
    // Shell-level degraded probe (§7 preamble). Runs *once* on the page
    // GET so a healthy upstream shows a normal shell even if the tab
    // itself fails downstream (the tab failure renders as an outcome
    // inside the panel, not as a page-wide banner).
    let degraded_reason = probe_degraded(&state).await;
    let panel = build_panel(&state, active_tab).await;
    let tabs = tab_entries(active_tab);
    render(
        DiagnosticsPageTemplate {
            chrome,
            tabs,
            panel,
            degraded_reason,
        }
        .render(),
    )
}

// ── GET /hts/diagnostics/panel ──────────────────────────────────────────

async fn diagnostics_panel(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
    Query(query): Query<TabQuery>,
) -> Response {
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "diagnostics",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    let active_tab = Tab::from_slug(query.tab.as_deref().unwrap_or(""));
    let panel = build_panel(&state, active_tab).await;
    render(DiagnosticsPanelTemplate { chrome, panel }.render())
}

async fn probe_degraded(state: &HtsUiState) -> Option<&'static str> {
    match state.upstream.health().await {
        Ok(_) => None,
        Err(e) => Some(e.degraded_reason()),
    }
}

fn render(rendered: Result<String, askama::Error>) -> Response {
    match rendered {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!(?err, "hts-ui diagnostics template render failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Html(String::from("<pre>hts-ui: template render error</pre>")),
            )
                .into_response()
        }
    }
}
