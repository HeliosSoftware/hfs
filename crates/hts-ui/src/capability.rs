//! Capability & Conformance page.
//!
//! **Shape of record (2026-09-01, #808).** This page and HFS's
//! `/ui/capability-statement` are no longer two implementations of one
//! document. The parser, the view model and the four cards they have in
//! common all live in [`helios_ui_chrome::capability`]; what stays here is
//! what is genuinely HTS's:
//!
//!   • two upstream fetches instead of one loopback self-call,
//!   • per-card degradation rather than a single page-level warning,
//!   • the **Terminology capabilities** card, which only a terminology
//!     server can declare,
//!   • a byte cap on the raw statement, because HTS's grows with the code
//!     systems it loads.
//!
//! It was previously called "Diagnostics" and lived at `/hts/diagnostics`;
//! both are kept working by a 308 redirect registered in
//! `crates/hts/src/server.rs`.
//!
//! Six cards: five rendered from the shared code HFS renders, plus
//! **Terminology capabilities**.
//!
//! Two upstream sources (`/metadata`, `/metadata?mode=terminology`) are
//! fetched and each feeds its own cards. A failure on one is isolated to
//! those cards, which render a `<p class="notice notice--warn">` carrying the
//! existing `hts-degraded-reason-*` sentence; the cards fed by the other
//! source are unaffected. The shared cards take that sentence directly
//! ([`CapabilityCards::notice`]), so the degraded state costs no duplicated
//! card headings.
//!
//! Three cards that used to live here were removed on 2026-08-27 because
//! each duplicated a surface that already served it better:
//!
//!   • **Health** — Home's status tile already renders `/health`.
//!   • **Prometheus raw** — Home's request-rate chart already reads
//!     `/metrics`. HFS folds the raw *CapabilityStatement* here instead,
//!     which is the artefact this page is actually about, so that is what
//!     this page now folds.
//!   • **Code systems** — `/ui/hts/code-systems` lists the same rows from
//!     the same table (`supported_systems()` is `SELECT url FROM
//!     code_systems`) with five columns instead of two, real paging instead
//!     of a 50-row cap, and a link into each system's detail page. Only the
//!     count survives here, where it reads as a capability.

use askama::Template;
use axum::{
    Router,
    extract::State,
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use axum_htmx::HxRequest;
use helios_ui_chrome::capability::{CapabilityCards, DocsVersion};
use std::sync::Arc;

use crate::i18n::{I18n, RequestLocale};
use crate::upstream::TerminologyCapabilitiesView;
use crate::{Chrome, HtsUiState};

// ── Routing ─────────────────────────────────────────────────────────────

pub(crate) fn routes() -> Router<Arc<HtsUiState>> {
    Router::new()
        .route("/hts/capability-statement", get(capability_page))
        // The page shipped as `/ui/hts/diagnostics` before it was renamed to
        // match HFS. Keep the old path working — it may be bookmarked, and
        // the docs and e2e specs referenced it. `Redirect::permanent` emits
        // 308 (preserves method + body), matching the trailing-slash
        // canonicalization in `home.rs`; GET is the only verb here.
        .route(
            "/hts/diagnostics",
            get(|| async { Redirect::permanent("/ui/hts/capability-statement") }),
        )
}

// ── Templates ───────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/capability-statement.html")]
struct CapabilityPageTemplate<'a> {
    chrome: Chrome<'a>,
    view: CapabilityPageView,
}

/// Everything the stacked cards render, gathered in one pass.
///
/// The four shared cards arrive here as finished HTML — rendered in the
/// handler rather than from the page template because [`CapabilityCards`] is
/// fallible and a template cannot decide what half a page should look like.
/// A card fed by a failed fetch is not absent: it holds the shared card's
/// heading over a `notice--warn` carrying the `hts-degraded-reason-*`
/// sentence produced by
/// [`crate::upstream::UpstreamError::degraded_reason`], so the warning reuses
/// catalog strings that already exist in all three locales rather than
/// minting a per-page "unavailable" string.
#[derive(Clone, Debug, Default)]
struct CapabilityPageView {
    summary_card: String,
    /// `None` when the server declares no system interactions, and when the
    /// fetch failed so we cannot know whether it would have.
    ///
    /// HTS serves `POST /` (batch) but does not advertise it in
    /// `rest[].interaction`, so this card is absent today rather than blank.
    /// It appears on its own the moment HTS declares them — the UI never
    /// invents the list.
    interactions_card: Option<String>,
    operations_card: String,
    resources_card: String,
    /// The raw statement, or `None` when `/metadata` could not be read —
    /// there is no half-document worth folding.
    raw: Option<RawStatement>,
    terminology: Option<TerminologyCapabilitiesView>,
    terminology_reason: Option<&'static str>,
}

/// The capped copy of the statement behind the raw fold.
#[derive(Clone, Debug)]
struct RawStatement {
    text: String,
    truncated: bool,
    full_bytes: usize,
}

// ── View builder ────────────────────────────────────────────────────────

async fn build_view(state: &HtsUiState, i18n: &I18n) -> Result<CapabilityPageView, askama::Error> {
    // Sequential, deliberately. Firing the probes with `tokio::join!` opens
    // simultaneous upstream connections per page load; under the crate's
    // parallel test harness (several `#[tokio::test]`s, each with its own
    // current-thread runtime and its own in-process mock) that reliably
    // stalls on Windows until the request timeout fires. Every other
    // handler in this crate makes its upstream calls in sequence for the
    // same reason, and two localhost round-trips are not the page's cost
    // centre.
    //
    // `fhir_version` is the release code the `hts` binary was built for. An
    // unrecognised value falls back to R4 — the workspace default and the
    // only release a build can be certain to carry — rather than dropping
    // every specification link on the page.
    let version = DocsVersion::from_code(state.fhir_version).unwrap_or_default();
    let capability = state.upstream.capability_statement(version).await;
    let terminology = state.upstream.terminology_capabilities_view().await;

    let statement = capability.as_ref().ok();
    let reason = capability
        .as_ref()
        .err()
        .map(|e| i18n.t(&format!("hts-degraded-reason-{}", e.degraded_reason())));
    let projection = statement.map(|s| s.cards.clone()).unwrap_or_default();

    // HTS lists exactly three resource types, so HFS's `filter-rail__search`
    // form is deliberately not taken: a search box over three rows is noise,
    // not parity. Nor are HFS's `Includes` / `Revincludes` columns — HTS
    // emits no `searchInclude` / `searchRevInclude`, and a column of zeroes
    // would read as a measurement rather than an absence.
    let cards = CapabilityCards::new(i18n, &projection)
        .notice(reason.as_deref())
        .operations_empty_key(Some("hts-capability-operations-empty"))
        .resources_empty_key("hts-capability-rest-empty");

    let mut view = CapabilityPageView {
        summary_card: cards.summary()?,
        interactions_card: (statement.is_some() && !projection.interactions.is_empty())
            .then(|| cards.interactions())
            .transpose()?,
        operations_card: cards.operations()?,
        resources_card: cards.resources()?,
        raw: statement.map(|s| RawStatement {
            text: s.raw.clone(),
            truncated: s.raw_truncated,
            full_bytes: s.raw_full_bytes,
        }),
        ..Default::default()
    };
    match terminology {
        Ok(v) => view.terminology = Some(v),
        Err(e) => view.terminology_reason = Some(e.degraded_reason()),
    }
    Ok(view)
}

// ── GET /hts/capability-statement ───────────────────────────────────────

async fn capability_page(
    State(state): State<Arc<HtsUiState>>,
    // Taking the extractor is what arms `axum_htmx::AutoVaryLayer`, so the
    // response carries `Vary: HX-Request`. The page body is identical in
    // both modes (HFS's capability page has no fragment endpoint either).
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    let i18n = I18n::new(locale);
    let chrome = Chrome {
        i18n,
        active_page: "capability-statement",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    // Both legs are `askama::Error`: rendering the shared cards and rendering
    // the page around them fail the same way and take the same 500 path.
    render(
        build_view(&state, &i18n)
            .await
            .and_then(|view| CapabilityPageTemplate { chrome, view }.render()),
    )
}

fn render(rendered: Result<String, askama::Error>) -> Response {
    match rendered {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!(?err, "hts-ui capability template render failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Html(String::from("<pre>hts-ui: template render error</pre>")),
            )
                .into_response()
        }
    }
}
