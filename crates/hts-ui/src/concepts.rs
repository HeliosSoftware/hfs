//! The concept information plane — a concept as a top-level object with its
//! own permalink (design doc Direction B, "concept-first").
//!
//! Every other HTS-UI surface is resource-first: you find a CodeSystem, then
//! you ask it a question. This one inverts that. The address is
//! `system` + `code`, and the page answers the three questions an operator
//! actually has about a code they were handed:
//!
//! | Panel | Question | Upstream |
//! |---|---|---|
//! | Identity | What *is* this code? | `POST /CodeSystem/$lookup` (`property=*`) |
//! | Mappings | What does it map to, in **any** stored map? | `POST /ConceptMap/$translate`, `url` omitted |
//! | Subsumption | Does the hierarchy agree with itself? | `POST /CodeSystem/$subsumes`, one call per comparator |
//!
//! # Why the route is query-shaped
//!
//! `GET /ui/hts/concepts?system=…&code=…` rather than
//! `/ui/hts/concepts/{system}/{code}`. A canonical system URI
//! (`http://hl7.org/fhir/sid/icd-10-cm`) in a path segment needs a
//! double-encoded `%252F`; axum's `Path` decoder normalizes single-encoded
//! `%2F` back into a segment separator, and plenty of reverse proxies reject
//! the encoded form outright. The query string has no such problem, and
//! `form_urlencoded::Serializer` builds every outgoing link so the encoding is
//! never hand-rolled.
//!
//! # Panel loading
//!
//! Identity renders server-side in the page shell, so first paint is
//! meaningful even on a slow link. Mappings and Subsumption are lazy: their
//! skeletons self-fetch with `hx-trigger="load"` + `hx-swap="outerHTML"` and no
//! `hx-target`, and the returned fragment re-emits the `id` **without** the
//! trigger, so the swap terminates. Each skeleton carries a `<noscript>` link
//! to the panel's standalone route, which renders the full page around that one
//! panel — so the plane degrades to plain navigation without JavaScript.
//!
//! # Error contract
//!
//! HTS's 404s carry a JSON `OperationOutcome` but under `Content-Type:
//! application/json` (not `application/fhir+json`), so nothing here gates the
//! outcome partial on the FHIR content type. Extractor-level rejections
//! (400 / 415 / 408) bypass `HtsError` entirely and come back as plain text,
//! which is why the degraded banner still exists alongside the outcome partial.

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
    ConceptIdentity, ConceptMappings, ConceptRef, MappingDirection, OutcomeView, SubsumptionReport,
    UpstreamError,
};
use crate::{Chrome, HtsUiState};

// ── Routing ─────────────────────────────────────────────────────────────

pub(crate) fn routes() -> Router<Arc<HtsUiState>> {
    Router::new()
        .route("/hts/concepts", get(concept_page))
        .route("/hts/concepts/identity", get(identity_panel))
        .route("/hts/concepts/mappings", get(mappings_panel))
        .route("/hts/concepts/relations", get(relations_panel))
}

/// Which panel a request is asking for. The page shell asks for
/// [`Panel::Page`]; the three fragment routes ask for their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Panel {
    Page,
    Identity,
    Mappings,
    Relations,
}

// ── Query parsing ───────────────────────────────────────────────────────

/// The query shape shared by the page and all three panel routes.
///
/// Every field is `Option<String>` so a missing or malformed pair collapses to
/// a rendered `invalid` OperationOutcome rather than axum's bare 400 — the
/// permalink is something people paste out of tickets and chat logs, and a
/// half-typed one should explain itself.
#[derive(Debug, Default, Deserialize)]
struct ConceptForm {
    system: Option<String>,
    code: Option<String>,
    version: Option<String>,
    #[serde(rename = "displayLanguage")]
    display_language: Option<String>,
    /// Mappings panel: `forward` (default) or `reverse`.
    direction: Option<String>,
    /// Subsumption panel: the operator's free-text comparator code.
    compare: Option<String>,
    // The language switcher hangs `?lang=` off any page; accept it silently.
    #[allow(dead_code)]
    lang: Option<String>,
    /// No-JS fallback: `?raw=1` renders the full JSON in a plain `<pre>` (#898).
    raw: Option<String>,
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

impl ConceptForm {
    fn reference(&self) -> ConceptRef {
        ConceptRef {
            system: self.system.clone().unwrap_or_default().trim().to_owned(),
            code: self.code.clone().unwrap_or_default().trim().to_owned(),
            version: non_empty(self.version.clone()),
            display_language: non_empty(self.display_language.clone()),
        }
    }

    fn direction(&self) -> MappingDirection {
        MappingDirection::from_query(self.direction.as_deref())
    }

    fn compare(&self) -> String {
        non_empty(self.compare.clone()).unwrap_or_default()
    }

    /// True when the no-JS fallback was requested (`?raw=1`).
    fn raw_requested(&self) -> bool {
        self.raw.as_deref() == Some("1")
    }
}

/// Pre-flight for the free-text comparator.
///
/// `$subsumes` rejects cross-system codings with a 400, but the system here is
/// pinned server-side from the concept address, so that arm is unreachable —
/// which means a pasted `system|code` or a bare URI would be silently sent as a
/// *code* and come back as a confusing 404. Catching the shape locally names
/// the real problem instead of round-tripping for a worse answer.
///
/// Returns the diagnostic string when the input is unusable.
fn comparator_problem(compare: &str) -> Option<String> {
    if compare.is_empty() {
        return None;
    }
    if compare.contains("://") || compare.contains('|') {
        return Some(format!(
            "`{compare}` looks like a system-qualified reference. \
             $subsumes compares two codes within one system, and the system is \
             already pinned to this concept's — enter the bare code only."
        ));
    }
    if compare.chars().any(char::is_whitespace) {
        return Some(format!(
            "`{compare}` contains whitespace; a FHIR code cannot."
        ));
    }
    None
}

// ── View model ──────────────────────────────────────────────────────────

/// Everything the page shell and all three panel partials read.
///
/// One struct for every template so the partials can be `{% include %}`d from
/// both the page and their own fragment wrapper without duplicating accessors.
/// A panel field is `None` when *this* render did not load it — that is what
/// makes the skeleton branch fire.
pub(crate) struct ConceptView {
    reference: ConceptRef,
    direction: MappingDirection,
    compare: String,
    /// Pre-flight complaint about `compare`, rendered instead of calling out.
    compare_problem: Option<String>,
    identity: Option<Result<ConceptIdentity, UpstreamError>>,
    mappings: Option<Result<ConceptMappings, UpstreamError>>,
    relations: Option<Result<SubsumptionReport, UpstreamError>>,
    /// FHIR version for building fragment URLs (#898).
    fhir_version: &'static str,
    /// True when the no-JS fallback was requested (`?raw=1`) (#898).
    raw_requested: bool,
}

/// Connection-class failures render the degraded banner; everything else
/// renders an OperationOutcome. Shared by all three panels.
fn degraded_reason(err: &UpstreamError) -> Option<&'static str> {
    match err {
        UpstreamError::Connect { .. }
        | UpstreamError::Timeout { .. }
        | UpstreamError::ClientBuild { .. } => Some(err.degraded_reason()),
        _ => None,
    }
}

/// Project an upstream failure into the outcome partial's shape. Returns
/// `None` for the connection-class errors that the degraded banner owns.
fn outcome_for(err: &UpstreamError) -> Option<OutcomeView> {
    match err {
        UpstreamError::Outcome { outcome, .. } => Some((**outcome).clone()),
        UpstreamError::NotFound { .. } => Some(OutcomeView {
            severity: "error".to_owned(),
            code: "not-found".to_owned(),
            ..OutcomeView::default()
        }),
        UpstreamError::HttpStatus { status, .. } => Some(OutcomeView {
            severity: "error".to_owned(),
            code: match *status {
                400 => "invalid".to_owned(),
                404 => "not-found".to_owned(),
                422 => "too-costly".to_owned(),
                _ => "unknown".to_owned(),
            },
            ..OutcomeView::default()
        }),
        UpstreamError::Decode { message, .. } => Some(OutcomeView::invalid_input(message.clone())),
        UpstreamError::Connect { .. }
        | UpstreamError::Timeout { .. }
        | UpstreamError::ClientBuild { .. } => None,
    }
}

impl ConceptView {
    // -- address --------------------------------------------------------

    pub(crate) fn reference(&self) -> &ConceptRef {
        &self.reference
    }

    /// The pre-flight `invalid` OperationOutcome for a half-typed permalink.
    /// Deliberately an outcome at HTTP 200, not a 400: the operator needs to
    /// read what is missing, and a bare status code in the network tab is not
    /// that.
    pub(crate) fn address_outcome(&self) -> Option<OutcomeView> {
        self.reference.missing_parameter().map(|missing| {
            OutcomeView::invalid_input(format!(
                "Missing required query parameter: {missing}. \
                 A concept permalink is /ui/hts/concepts?system=<canonical-uri>&code=<code>."
            ))
        })
    }

    /// Page heading: the concept's display once identity has loaded, the bare
    /// code before that.
    pub(crate) fn heading(&self) -> String {
        match self.identity.as_ref().and_then(|r| r.as_ref().ok()) {
            Some(identity) => identity.heading().to_owned(),
            None => self.reference.code.clone(),
        }
    }

    // -- panel URLs (built through form_urlencoded, never concatenated) --

    pub(crate) fn mappings_url(&self) -> String {
        self.reference
            .panel_url_with("mappings", &[("direction", self.direction.as_str())])
    }

    /// Where the direction toggle points — the same panel in the other
    /// direction. One URL serves both `href` (hard nav renders a full page)
    /// and `hx-get` (renders the fragment).
    pub(crate) fn mappings_toggle_url(&self) -> String {
        self.reference.panel_url_with(
            "mappings",
            &[("direction", self.direction.toggled().as_str())],
        )
    }

    pub(crate) fn relations_url(&self) -> String {
        self.reference
            .panel_url_with("relations", &[("compare", self.compare.as_str())])
    }

    pub(crate) fn direction_suffix(&self) -> &'static str {
        self.direction.as_str()
    }

    pub(crate) fn toggle_suffix(&self) -> &'static str {
        self.direction.toggled().as_str()
    }

    pub(crate) fn compare(&self) -> &str {
        &self.compare
    }

    pub(crate) fn compare_problem(&self) -> Option<OutcomeView> {
        self.compare_problem.clone().map(OutcomeView::invalid_input)
    }

    // -- identity panel --------------------------------------------------

    pub(crate) fn identity(&self) -> Option<&ConceptIdentity> {
        self.identity.as_ref().and_then(|r| r.as_ref().ok())
    }

    pub(crate) fn identity_degraded(&self) -> Option<&'static str> {
        self.identity
            .as_ref()
            .and_then(|r| r.as_ref().err())
            .and_then(degraded_reason)
    }

    pub(crate) fn identity_outcome(&self) -> Option<OutcomeView> {
        self.identity
            .as_ref()
            .and_then(|r| r.as_ref().err())
            .and_then(outcome_for)
    }

    // -- mappings panel --------------------------------------------------

    /// True when this render should emit the self-fetching skeleton rather
    /// than the loaded card.
    pub(crate) fn mappings_pending(&self) -> bool {
        self.mappings.is_none()
    }

    pub(crate) fn mappings(&self) -> Option<&ConceptMappings> {
        self.mappings.as_ref().and_then(|r| r.as_ref().ok())
    }

    pub(crate) fn mappings_degraded(&self) -> Option<&'static str> {
        self.mappings
            .as_ref()
            .and_then(|r| r.as_ref().err())
            .and_then(degraded_reason)
    }

    pub(crate) fn mappings_outcome(&self) -> Option<OutcomeView> {
        self.mappings
            .as_ref()
            .and_then(|r| r.as_ref().err())
            .and_then(outcome_for)
    }

    // -- subsumption panel -----------------------------------------------

    pub(crate) fn relations_pending(&self) -> bool {
        self.relations.is_none()
    }

    pub(crate) fn relations(&self) -> Option<&SubsumptionReport> {
        self.relations.as_ref().and_then(|r| r.as_ref().ok())
    }

    pub(crate) fn relations_degraded(&self) -> Option<&'static str> {
        self.relations
            .as_ref()
            .and_then(|r| r.as_ref().err())
            .and_then(degraded_reason)
    }

    pub(crate) fn relations_outcome(&self) -> Option<OutcomeView> {
        self.relations
            .as_ref()
            .and_then(|r| r.as_ref().err())
            .and_then(outcome_for)
    }

    // -- JSON fold (#898) -----------------------------------------------

    /// True when the no-JS fallback was requested (`?raw=1`).
    pub(crate) fn raw_requested(&self) -> bool {
        self.raw_requested
    }

    /// Fragment URL for the identity panel's incremental JSON fold.
    pub(crate) fn identity_json_fragment_url(&self) -> String {
        crate::raw_json_fragment::identity_fragment_url(&self.reference, self.fhir_version)
    }

    /// No-JS fallback URL for the identity panel's raw JSON (with `?raw=1`).
    pub(crate) fn identity_raw_url(&self) -> String {
        self.reference.panel_url_with("identity", &[("raw", "1")])
    }

    /// Fragment URL for the mappings panel's incremental JSON fold.
    pub(crate) fn mappings_json_fragment_url(&self) -> String {
        crate::raw_json_fragment::mappings_fragment_url(
            &self.reference,
            self.direction,
            self.fhir_version,
        )
    }

    /// No-JS fallback URL for the mappings panel's raw JSON (with `?raw=1`).
    pub(crate) fn mappings_raw_url(&self) -> String {
        self.reference.panel_url_with(
            "mappings",
            &[("direction", self.direction.as_str()), ("raw", "1")],
        )
    }
}

// ── Templates ───────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/concept.html")]
struct ConceptPageTemplate<'a> {
    chrome: Chrome<'a>,
    view: ConceptView,
}

#[derive(Template)]
#[template(path = "partials/hts-concept-identity.html")]
struct IdentityFragmentTemplate<'a> {
    chrome: Chrome<'a>,
    view: ConceptView,
}

#[derive(Template)]
#[template(path = "partials/hts-concept-mappings.html")]
struct MappingsFragmentTemplate<'a> {
    chrome: Chrome<'a>,
    view: ConceptView,
}

#[derive(Template)]
#[template(path = "partials/hts-concept-relations.html")]
struct RelationsFragmentTemplate<'a> {
    chrome: Chrome<'a>,
    view: ConceptView,
}

// ── Handlers ────────────────────────────────────────────────────────────

async fn concept_page(
    State(state): State<Arc<HtsUiState>>,
    Query(form): Query<ConceptForm>,
    // Extracted so `AutoVaryLayer` marks the response `Vary: HX-Request` even
    // on the shell, whose body is identical in both modes.
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    respond(&state, form, locale, Panel::Page, false).await
}

async fn identity_panel(
    State(state): State<Arc<HtsUiState>>,
    Query(form): Query<ConceptForm>,
    HxRequest(is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    respond(&state, form, locale, Panel::Identity, is_htmx).await
}

async fn mappings_panel(
    State(state): State<Arc<HtsUiState>>,
    Query(form): Query<ConceptForm>,
    HxRequest(is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    respond(&state, form, locale, Panel::Mappings, is_htmx).await
}

async fn relations_panel(
    State(state): State<Arc<HtsUiState>>,
    Query(form): Query<ConceptForm>,
    HxRequest(is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    respond(&state, form, locale, Panel::Relations, is_htmx).await
}

/// Load exactly what `panel` needs, then render a fragment (htmx) or the whole
/// page around it (hard navigation).
async fn respond(
    state: &HtsUiState,
    form: ConceptForm,
    locale: RequestLocale,
    panel: Panel,
    is_htmx: bool,
) -> Response {
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "concepts",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    let view = load(state, form, panel).await;

    if is_htmx {
        return match panel {
            Panel::Identity => render(IdentityFragmentTemplate { chrome, view }.render()),
            Panel::Mappings => render(MappingsFragmentTemplate { chrome, view }.render()),
            Panel::Relations => render(RelationsFragmentTemplate { chrome, view }.render()),
            // The shell has no fragment form; htmx navigating to it gets the
            // page, which is what `hx-boost`-style navigation expects.
            Panel::Page => render(ConceptPageTemplate { chrome, view }.render()),
        };
    }
    render(ConceptPageTemplate { chrome, view }.render())
}

/// The whole data-loading policy for the plane, in one place.
///
/// Identity is fetched for every panel — the shell needs it for the heading,
/// and the subsumption panel needs its `parent` / `child` properties to derive
/// comparators at all. Mappings and Subsumption are fetched only when they are
/// the requested panel, which is what keeps the shell's first paint to a single
/// upstream round-trip.
async fn load(state: &HtsUiState, form: ConceptForm, panel: Panel) -> ConceptView {
    let reference = form.reference();
    let direction = form.direction();
    let compare = form.compare();
    let compare_problem = comparator_problem(&compare);
    let raw_requested = form.raw_requested();

    let mut view = ConceptView {
        reference,
        direction,
        compare,
        compare_problem,
        identity: None,
        mappings: None,
        relations: None,
        fhir_version: state.fhir_version,
        raw_requested,
    };

    // A half-typed permalink never reaches upstream: there is nothing to ask.
    if !view.reference.is_addressable() {
        return view;
    }

    let identity = state.upstream.concept_identity(&view.reference).await;

    match panel {
        Panel::Page | Panel::Identity => {}
        Panel::Mappings => {
            view.mappings = Some(
                state
                    .upstream
                    .concept_mappings(&view.reference, view.direction)
                    .await,
            );
        }
        Panel::Relations => {
            view.relations = Some(match identity.as_ref() {
                Ok(identity) => Ok(state
                    .upstream
                    .concept_subsumption(
                        &view.reference,
                        identity,
                        // A comparator that failed pre-flight is reported, not
                        // sent — see `comparator_problem`.
                        if view.compare_problem.is_some() {
                            None
                        } else {
                            Some(view.compare.as_str())
                        },
                    )
                    .await),
                // Without `$lookup` there are no derived comparators, and the
                // panel's whole value is the derived rows. Propagate the same
                // failure rather than rendering a table that silently lost
                // them.
                Err(err) => Err(err.clone()),
            });
        }
    }

    view.identity = Some(identity);
    view
}

// ── Small helpers ───────────────────────────────────────────────────────

fn render(rendered: Result<String, askama::Error>) -> Response {
    match rendered {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!(?err, "hts-ui concepts template render failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Html(String::from("<pre>hts-ui: template render error</pre>")),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparator_preflight_rejects_system_qualified_input() {
        assert!(comparator_problem("").is_none());
        assert!(comparator_problem("A01").is_none());
        assert!(comparator_problem("http://example.org/cs|A01").is_some());
        assert!(comparator_problem("http://example.org/cs").is_some());
        assert!(comparator_problem("A01 B02").is_some());
    }

    #[test]
    fn form_defaults_to_forward_and_trims_the_address() {
        let form = ConceptForm {
            system: Some("  http://example.org/cs  ".into()),
            code: Some(" A01 ".into()),
            version: Some("   ".into()),
            ..ConceptForm::default()
        };
        let reference = form.reference();
        assert_eq!(reference.system, "http://example.org/cs");
        assert_eq!(reference.code, "A01");
        assert_eq!(reference.version, None);
        assert_eq!(form.direction(), MappingDirection::Forward);
    }

    #[test]
    fn unknown_direction_collapses_to_forward() {
        let form = ConceptForm {
            direction: Some("sideways".into()),
            ..ConceptForm::default()
        };
        assert_eq!(form.direction(), MappingDirection::Forward);
        let reverse = ConceptForm {
            direction: Some("REVERSE".into()),
            ..ConceptForm::default()
        };
        assert_eq!(reverse.direction(), MappingDirection::Reverse);
    }
}
