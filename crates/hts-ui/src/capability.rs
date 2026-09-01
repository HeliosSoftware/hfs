//! Capability & Conformance page.
//!
//! **Shape of record (2026-08-27).** This page mirrors HFS's
//! `crates/ui/templates/pages/capability-statement.html`: same name in the
//! sidebar (`nav-capability-conformance`), same icon (`icons/shield.svg`),
//! same route shape (`/ui/hts/capability-statement` beside HFS's
//! `/ui/capability-statement`), and the same stacked `<section class="card">`
//! blocks in the same order. It was previously called "Diagnostics" and
//! lived at `/hts/diagnostics`; both are kept working by a 308 redirect
//! registered in `crates/hts/src/server.rs`.
//!
//! Six cards: five mirroring HFS one-for-one, plus **Terminology
//! capabilities** — the one thing only a terminology server can declare.
//!
//! Two upstream sources (`/metadata`, `/metadata?mode=terminology`) are
//! fetched and each renders into its own card. A failure on one is isolated
//! to that card, which renders a `<p class="notice notice--warn">` carrying
//! the existing `hts-degraded-reason-*` sentence; the other cards are
//! unaffected.
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
use std::sync::Arc;

use crate::i18n::{I18n, RequestLocale};
use crate::upstream::{CapabilityView, TerminologyCapabilitiesView};
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
/// Each source carries a `Some(view)` **or** a `Some(reason)` — never
/// both. `reason` is the `hts-degraded-reason-*` suffix produced by
/// [`crate::upstream::UpstreamError::degraded_reason`], so the card's
/// warning notice reuses catalog strings that already exist in all three
/// locales rather than minting a per-page "unavailable" string.
#[derive(Clone, Debug, Default)]
struct CapabilityPageView {
    capability: Option<CapabilityView>,
    capability_reason: Option<&'static str>,
    terminology: Option<TerminologyCapabilitiesView>,
    terminology_reason: Option<&'static str>,
}

// ── View builder ────────────────────────────────────────────────────────

async fn build_view(state: &HtsUiState) -> CapabilityPageView {
    // Sequential, deliberately. Firing the probes with `tokio::join!` opens
    // simultaneous upstream connections per page load; under the crate's
    // parallel test harness (several `#[tokio::test]`s, each with its own
    // current-thread runtime and its own in-process mock) that reliably
    // stalls on Windows until the request timeout fires. Every other
    // handler in this crate makes its upstream calls in sequence for the
    // same reason, and two localhost round-trips are not the page's cost
    // centre.
    let capability = state.upstream.capability_statement().await;
    let terminology = state.upstream.terminology_capabilities_view().await;

    let mut view = CapabilityPageView::default();
    match capability {
        Ok(v) => view.capability = Some(v),
        Err(e) => view.capability_reason = Some(e.degraded_reason()),
    }
    match terminology {
        Ok(v) => view.terminology = Some(v),
        Err(e) => view.terminology_reason = Some(e.degraded_reason()),
    }
    view
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
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "capability-statement",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    render(
        CapabilityPageTemplate {
            chrome,
            view: build_view(&state).await,
        }
        .render(),
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
