//! Slice F — standalone Import page (design doc §7.7).
//!
//! Two routes under `/hts/import`:
//!
//! - `GET /hts/import` — full-page shell with upload form + empty status
//!   region. On `HX-Request` returns only the upload-form partial (dual-mode
//!   per design doc §7.6 F14 / §7.10 row 7.7 nojs contract).
//! - `POST /hts/import` — accepts the JSON Bundle from the form, proxies to
//!   HTS `POST /import`, and renders the status partial. On hard nav
//!   re-renders the full page with the status partial embedded.
//!
//! Slice F v1 ships **paste-only**: the design lists both paste and file
//! sources in §7.7 but the file-upload path requires multipart plumbing
//! that would inflate the diff without new test coverage the paste path
//! doesn't already provide. See `# TODO(F): file input follow-up` below.
//! The `<input type="file">` still renders for a11y symmetry with the
//! radio group, but its value is currently ignored by the server (a
//! stub input error surfaces via the same empty-bundle gate that
//! catches paste-mode misses).

use askama::Template;
use axum::{
    Router,
    body::Bytes,
    extract::State,
    response::{Html, IntoResponse, Response},
    routing::get,
};
use axum_htmx::HxRequest;
use std::{collections::HashMap, sync::Arc};

use crate::i18n::{I18n, RequestLocale};
use crate::upstream::{ImportResult, ImportStatus, OutcomeView, UpstreamError};
use crate::{Chrome, HtsUiState};

// ── Routing ─────────────────────────────────────────────────────────────

pub(crate) fn routes() -> Router<Arc<HtsUiState>> {
    Router::new().route("/hts/import", get(import_page).post(import_run))
}

// ── Page shell ──────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/import.html")]
struct ImportPageTemplate<'a> {
    chrome: Chrome<'a>,
    status: Option<StatusView>,
    /// True when the upstream `/health` probe failed on the initial GET —
    /// the shell then renders the shared `hts-degraded.html` banner and
    /// disables the submit button (design doc §7 preamble degraded state).
    degraded_reason: Option<&'static str>,
}

#[derive(Template)]
#[template(path = "partials/hts-import-form.html")]
struct ImportFormTemplate<'a> {
    chrome: Chrome<'a>,
    degraded_reason: Option<&'static str>,
}

#[derive(Template)]
#[template(path = "partials/hts-import-status.html")]
struct ImportStatusTemplate<'a> {
    chrome: Chrome<'a>,
    view: StatusView,
}

/// Data driving the four visual variants of the status partial. Askama
/// branches on the `is_*` booleans (matches the E1 `OpsFlags` idiom)
/// so the template never needs to import the `ImportStatus` enum.
#[derive(Clone, Debug)]
struct StatusView {
    is_success: bool,
    is_partial: bool,
    is_rejected: bool,
    is_too_large: bool,
    counts_code_systems: Option<u32>,
    counts_value_sets: Option<u32>,
    counts_concept_maps: Option<u32>,
    counts_concepts: Option<u32>,
    issues: Vec<String>,
    outcome: Option<OutcomeView>,
    request_url: String,
    raw_body: String,
    /// Reason key for the shared degraded partial when the upstream
    /// import round-trip failed at the transport layer (5xx / connect
    /// / timeout). `None` for the normal 200/207/400/413 arms.
    degraded_reason: Option<&'static str>,
}

impl StatusView {
    fn from_result(result: ImportResult) -> Self {
        let status = result.status;
        let (cs, vs, cm, cc) = match &result.counts {
            Some(c) => (
                Some(c.code_systems),
                Some(c.value_sets),
                Some(c.concept_maps),
                Some(c.concepts),
            ),
            None => (None, None, None, None),
        };
        Self {
            is_success: matches!(status, ImportStatus::Success),
            is_partial: matches!(status, ImportStatus::PartialSuccess),
            is_rejected: matches!(status, ImportStatus::Rejected),
            is_too_large: matches!(status, ImportStatus::TooLarge),
            counts_code_systems: cs,
            counts_value_sets: vs,
            counts_concept_maps: cm,
            counts_concepts: cc,
            issues: result.issues,
            outcome: result.outcome,
            request_url: result.request_url,
            raw_body: result.raw_body,
            degraded_reason: None,
        }
    }

    fn from_outcome(outcome: OutcomeView) -> Self {
        Self {
            is_success: false,
            is_partial: false,
            is_rejected: true,
            is_too_large: false,
            counts_code_systems: None,
            counts_value_sets: None,
            counts_concept_maps: None,
            counts_concepts: None,
            issues: Vec::new(),
            outcome: Some(outcome),
            request_url: String::new(),
            raw_body: String::new(),
            degraded_reason: None,
        }
    }

    fn from_error(request_url: String, err: &UpstreamError) -> Self {
        Self {
            is_success: false,
            is_partial: false,
            is_rejected: false,
            is_too_large: false,
            counts_code_systems: None,
            counts_value_sets: None,
            counts_concept_maps: None,
            counts_concepts: None,
            issues: Vec::new(),
            outcome: None,
            request_url,
            raw_body: String::new(),
            degraded_reason: Some(err.degraded_reason()),
        }
    }

    fn issue_count(&self) -> usize {
        self.issues.len()
    }
}

// ── GET /hts/import ─────────────────────────────────────────────────────

async fn import_page(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "import",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    // The Import shell exposes the operator-visible degraded banner if
    // the upstream `/health` probe fails — same trigger as the
    // dashboard and browser pages (design doc §7 preamble).
    let degraded_reason = probe_degraded(&state).await;
    if is_htmx {
        return render(
            ImportFormTemplate {
                chrome,
                degraded_reason,
            }
            .render(),
        );
    }
    render(
        ImportPageTemplate {
            chrome,
            status: None,
            degraded_reason,
        }
        .render(),
    )
}

async fn probe_degraded(state: &HtsUiState) -> Option<&'static str> {
    match state.upstream.health().await {
        Ok(_) => None,
        Err(e) => Some(e.degraded_reason()),
    }
}

// ── POST /hts/import ────────────────────────────────────────────────────

async fn import_run(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(is_htmx): HxRequest,
    locale: RequestLocale,
    body: Bytes,
) -> Response {
    let form = parse_form(&body);
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "import",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    let bundle = single(&form, "bundle");
    let bundle_trim = bundle.trim();

    // Pre-flight gate #1 — empty paste (and, until Slice F+1 wires up
    // multipart, empty file too). Mirrors the CM `$translate` empty-
    // code gate from Slice D: render an OperationOutcome and skip the
    // HTS round-trip entirely.
    if bundle_trim.is_empty() {
        let view = StatusView::from_outcome(OutcomeView::invalid_input(
            chrome.i18n.t("hts-import-empty-bundle-error"),
        ));
        return respond(&chrome, view, is_htmx);
    }

    // Pre-flight gate #2 — invalid JSON. Same shape as gate #1 but a
    // different diagnostic so the operator knows which failure they
    // tripped without opening the network tab.
    if serde_json::from_str::<serde_json::Value>(bundle_trim).is_err() {
        let view = StatusView::from_outcome(OutcomeView::invalid_input(
            chrome.i18n.t("hts-import-invalid-json-error"),
        ));
        return respond(&chrome, view, is_htmx);
    }

    match state.upstream.import_bundle(bundle_trim).await {
        Ok(result) => {
            let view = StatusView::from_result(result);
            respond(&chrome, view, is_htmx)
        }
        Err(err) => {
            let request_url = format!("{}/import", state.upstream.base_url());
            let view = StatusView::from_error(request_url, &err);
            respond(&chrome, view, is_htmx)
        }
    }
}

fn respond<'a>(chrome: &Chrome<'a>, view: StatusView, is_htmx: bool) -> Response {
    if is_htmx {
        return render(
            ImportStatusTemplate {
                chrome: *chrome,
                view,
            }
            .render(),
        );
    }
    render(
        ImportPageTemplate {
            chrome: *chrome,
            status: Some(view),
            // Post-submit the shell does not re-probe /health; if we
            // just reached HTS to import we would have surfaced the
            // failure via `from_error` and its `degraded_reason`
            // renders inside the status region.
            degraded_reason: None,
        }
        .render(),
    )
}

// ── Small helpers (paralleling the ones in code_systems.rs) ─────────────

fn render(rendered: Result<String, askama::Error>) -> Response {
    match rendered {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!(?err, "hts-ui import template render failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Html(String::from("<pre>hts-ui: template render error</pre>")),
            )
                .into_response()
        }
    }
}

/// Percent-decoded form body → multi-map. Slice F uses this only for
/// the `bundle` textarea; kept as a general helper so a future file /
/// multipart addition can layer on top without a rewrite.
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
