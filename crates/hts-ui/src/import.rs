//! Standalone Import page (design doc §7.7), V3 "stepped" layout (#551).
//!
//! Two routes under `/hts/import`:
//!
//! - `GET /hts/import` — full-page shell with steps 1–2 (source + review)
//!   and an empty step-3 result card. On `HX-Request` returns only the
//!   form partial (dual-mode per design doc §7.6 F14 / §7.10 row 7.7 nojs
//!   contract).
//! - `POST /hts/import` — accepts the JSON Bundle from the form, proxies to
//!   HTS `POST /import`, and renders the status partial. On hard nav
//!   re-renders the full page with the status partial embedded.
//!
//! # Layout
//!
//! Three numbered `.card` steps, all direct children of `.content` so the
//! shared `.content > .card ~ .card` rule spaces them. The three result
//! states are told apart by shared HFS primitives (`.notice` /
//! `.notice--warn` plus `.tag--matched` / `.tag--muted` / `.tag--excluded`)
//! rather than the old `hts-import-status--*` modifiers, which had no CSS
//! rule at all and made success and partial-success identical.
//!
//! # Why step 2 shows no entry counts
//!
//! HTS returns per-resource counts **only** in the `POST /import` response
//! body (`upstream::ImportCounts`), so it cannot know them before writing.
//! Step 2 is therefore a genuine confirm step — target server, request
//! shape, accepted resource types, and the update-in-place semantics — and
//! never a fabricated pre-flight summary.
//!
//! # File input
//!
//! The `<input type="file">` is handled entirely client-side by
//! `assets/import.js`, which `FileReader`s the picked file into the
//! `#hts-import-bundle` textarea; the server still only ever reads the
//! urlencoded `bundle` field and never sees `bundle_file`.

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
    /// Absolute URL the Bundle will be POSTed to upstream. Rendered in
    /// step 2 ("Review") so the operator can see which terminology server
    /// is about to be written to before committing. Known server-side
    /// without a round-trip — unlike the entry counts, which HTS reports
    /// only in the `POST /import` *response*.
    target_url: String,
}

#[derive(Template)]
#[template(path = "partials/hts-import-form.html")]
struct ImportFormTemplate<'a> {
    chrome: Chrome<'a>,
    degraded_reason: Option<&'static str>,
    target_url: String,
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

    /// Non-fatal issues as one newline-joined block, for the
    /// `<pre class="detail__code">` inside the issues disclosure.
    fn issues_text(&self) -> String {
        self.issues.join("\n")
    }

    /// Machine-readable marker rendered as `data-import-status` on the
    /// step-3 panel. Replaces the old `hts-import-status--{ok,warn,error}`
    /// class markers, which carried no CSS rule and therefore made
    /// success and partial-success visually identical (#551). Tests key
    /// off this attribute; styling comes from `.notice` / `.tag` only.
    fn status_slug(&self) -> &'static str {
        if self.degraded_reason.is_some() {
            "degraded"
        } else if self.is_too_large {
            "too-large"
        } else if self.is_rejected {
            "rejected"
        } else if self.is_partial {
            "partial"
        } else if self.is_success {
            "success"
        } else {
            "unknown"
        }
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
    let target_url = import_target_url(&state);
    if is_htmx {
        return render(
            ImportFormTemplate {
                chrome,
                degraded_reason,
                target_url,
            }
            .render(),
        );
    }
    render(
        ImportPageTemplate {
            chrome,
            status: None,
            degraded_reason,
            target_url,
        }
        .render(),
    )
}

/// Upstream URL the Bundle is POSTed to. Shared by the review step and
/// the transport-failure status view so both name the same endpoint.
fn import_target_url(state: &HtsUiState) -> String {
    format!("{}/import", state.upstream.base_url())
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
    let target_url = import_target_url(&state);
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
        return respond(&chrome, view, is_htmx, target_url);
    }

    // Pre-flight gate #2 — invalid JSON. Same shape as gate #1 but a
    // different diagnostic so the operator knows which failure they
    // tripped without opening the network tab.
    if serde_json::from_str::<serde_json::Value>(bundle_trim).is_err() {
        let view = StatusView::from_outcome(OutcomeView::invalid_input(
            chrome.i18n.t("hts-import-invalid-json-error"),
        ));
        return respond(&chrome, view, is_htmx, target_url);
    }

    match state.upstream.import_bundle(bundle_trim).await {
        Ok(result) => {
            let view = StatusView::from_result(result);
            respond(&chrome, view, is_htmx, target_url)
        }
        Err(err) => {
            let view = StatusView::from_error(target_url.clone(), &err);
            respond(&chrome, view, is_htmx, target_url)
        }
    }
}

fn respond<'a>(
    chrome: &Chrome<'a>,
    view: StatusView,
    is_htmx: bool,
    target_url: String,
) -> Response {
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
            target_url,
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
