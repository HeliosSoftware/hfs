//! Capability & Conformance page.
//!
//! **Shape of record (2026-09-01, #808).** This page and HFS's
//! `/ui/capability-statement` are no longer two implementations of one
//! document. The parser, the view model, the four summary cards and — since
//! this page's #808 follow-up — the Raw CapabilityStatement fold all live in
//! [`helios_ui_chrome::capability`] / [`helios_ui_chrome::capability_json`];
//! what stays here is what is genuinely HTS's:
//!
//!   • two upstream fetches instead of one loopback self-call,
//!   • per-card degradation rather than a single page-level warning,
//!   • the **Terminology capabilities** card, which only a terminology
//!     server can declare.
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
    body::Bytes,
    extract::{Query, State},
    http::HeaderMap,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use axum_htmx::HxRequest;
use helios_ui_chrome::capability::{CapabilityCards, DocsVersion};
use helios_ui_chrome::capability_json::{self, FragmentEndpoint};
use serde::Deserialize;
use std::sync::Arc;

use crate::i18n::{I18n, RequestLocale};
use crate::upstream::TerminologyCapabilitiesView;
use crate::{Chrome, HtsUiState};

/// Where the router registers the fragment endpoint — relative to this
/// crate's `/hts` prefix, the same way every other route in [`routes`] is
/// spelled. [`JSON_FRAGMENT_URL`] is the *public* counterpart: what a browser
/// actually requests once [`router`](crate::router) mounts this at `/ui`.
const JSON_FRAGMENT_ROUTE: &str = "/hts/capability-statement/json-fragment";
/// The public URL the raw fold's `data-fragment-url` and pagination links
/// point at. Must stay `/ui` + [`JSON_FRAGMENT_ROUTE`] in sync with the mount
/// point [`crate::router`] documents.
const JSON_FRAGMENT_URL: &str = "/ui/hts/capability-statement/json-fragment";
const JSON_EXPAND_ROUTE: &str = "/hts/capability-statement/json-expand";
const JSON_EXPAND_URL: &str = "/ui/hts/capability-statement/json-expand";
/// The no-JS fallback link: the page itself, requested with the plain-text
/// query flag. HTS carries no filter or version query params to preserve, so
/// (unlike HFS's `capability_raw_url`) this needs no builder.
const PAGE_RAW_URL: &str = "/ui/hts/capability-statement?raw=1";

fn fragment_endpoint(state: &HtsUiState) -> FragmentEndpoint<'_> {
    FragmentEndpoint {
        base_path: JSON_FRAGMENT_URL,
        version: state.fhir_version,
    }
}

// ── Routing ─────────────────────────────────────────────────────────────

pub(crate) fn routes() -> Router<Arc<HtsUiState>> {
    Router::new()
        .route("/hts/capability-statement", get(capability_page))
        .route(JSON_FRAGMENT_ROUTE, get(capability_json_fragment))
        .route(JSON_EXPAND_ROUTE, post(capability_json_expand))
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
    /// The Raw CapabilityStatement fold, pre-rendered by
    /// [`CapabilityCards::raw`] — the same shell HFS renders, lazy-loading
    /// its tree from [`JSON_FRAGMENT_URL`] (#808 follow-up to #798). `None`
    /// when `/metadata` could not be read — there is no half-document worth
    /// folding.
    raw_card: Option<String>,
    terminology: Option<TerminologyCapabilitiesView>,
    terminology_reason: Option<&'static str>,
}

// ── View builder ────────────────────────────────────────────────────────

async fn build_view(
    state: &HtsUiState,
    i18n: &I18n,
    raw_requested: bool,
) -> Result<CapabilityPageView, askama::Error> {
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

    // Unbounded and only computed on explicit request — same trade HFS makes
    // for its own `?raw=1` no-JS fallback. The default path never serializes
    // the whole document; it hands the fold a fragment URL instead.
    let raw_text = if raw_requested {
        statement
            .map(|s| serde_json::to_string_pretty(&s.document).unwrap_or_default())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let initial_outline = if raw_requested {
        None
    } else {
        statement.and_then(|statement| {
            match capability_json::plan(
                &statement.document,
                "",
                0,
                capability_json::DEFAULT_PAGE_SIZE,
                fragment_endpoint(state),
            ) {
                Ok(capability_json::View::Outline(outline)) => Some(outline),
                Ok(capability_json::View::Full(_)) | Err(_) => None,
            }
        })
    };
    let raw_card = statement
        .map(|_| {
            cards.raw(
                raw_requested,
                &raw_text,
                PAGE_RAW_URL,
                JSON_EXPAND_URL,
                initial_outline.as_ref(),
            )
        })
        .transpose()?;

    let mut view = CapabilityPageView {
        summary_card: cards.summary()?,
        interactions_card: (statement.is_some() && !projection.interactions.is_empty())
            .then(|| cards.interactions())
            .transpose()?,
        operations_card: cards.operations()?,
        resources_card: cards.resources()?,
        raw_card,
        ..Default::default()
    };
    match terminology {
        Ok(v) => view.terminology = Some(v),
        Err(e) => view.terminology_reason = Some(e.degraded_reason()),
    }
    Ok(view)
}

#[derive(Deserialize, Default)]
struct CapabilityQuery {
    /// A string flag because the public query spelling is `raw=1`, not a
    /// Serde boolean literal such as `raw=true` — mirrors HFS's own
    /// `CapabilityQuery`.
    raw: Option<String>,
}

// ── GET /hts/capability-statement ───────────────────────────────────────

async fn capability_page(
    State(state): State<Arc<HtsUiState>>,
    // Taking the extractor is what arms `axum_htmx::AutoVaryLayer`, so the
    // response carries `Vary: HX-Request`. The page body is identical in
    // both modes; only the raw fold's own fragment endpoint is htmx-driven.
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
    Query(query): Query<CapabilityQuery>,
) -> Response {
    let raw_requested = query.raw.as_deref() == Some("1");
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
        build_view(&state, &i18n, raw_requested)
            .await
            .and_then(|view| CapabilityPageTemplate { chrome, view }.render()),
    )
}

#[derive(Deserialize, Default)]
struct CapabilityJsonQuery {
    #[serde(default)]
    path: String,
    #[serde(default)]
    offset: usize,
    limit: Option<usize>,
}

// ── GET /hts/capability-statement/json-fragment ─────────────────────────

/// Mirrors HFS's `capability_json_fragment` handler: re-fetch, plan one
/// bounded level (or the whole subtree when it is small), render.
/// Re-fetching on every fragment click is the same cost model HFS's own
/// loopback self-call already pays per request — see
/// [`crate::upstream::UpstreamClient::capability_statement`].
async fn capability_json_fragment(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<CapabilityJsonQuery>,
) -> Response {
    let version = DocsVersion::from_code(state.fhir_version).unwrap_or_default();
    let document = match state.upstream.capability_statement(version).await {
        Ok(statement) => statement.document,
        Err(error) => {
            tracing::warn!("CapabilityStatement fragment fetch failed: {error}");
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "CapabilityStatement is unavailable",
            )
                .into_response();
        }
    };
    let limit = query.limit.unwrap_or(capability_json::DEFAULT_PAGE_SIZE);
    let i18n = I18n::new(locale);
    let endpoint = fragment_endpoint(&state);
    match capability_json::plan(&document, &query.path, query.offset, limit, endpoint) {
        Ok(capability_json::View::Full(json_lines)) => bounded_fragment(
            capability_json::render_full(&i18n, json_lines, query.path.is_empty()),
        ),
        Ok(capability_json::View::Outline(outline)) => {
            bounded_fragment(capability_json::render_outline(&i18n, &outline))
        }
        Err(capability_json::Error::NotFound) => {
            (axum::http::StatusCode::NOT_FOUND, "JSON path not found").into_response()
        }
        Err(capability_json::Error::InvalidPointer | capability_json::Error::InvalidPage) => (
            axum::http::StatusCode::BAD_REQUEST,
            "Invalid JSON fragment request",
        )
            .into_response(),
    }
}

async fn capability_json_expand(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| {
            value
                .trim()
                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
        })
    {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "Invalid JSON page state",
        )
            .into_response();
    }
    let pages = match capability_json::parse_page_descriptors(&body) {
        Ok(pages) => pages,
        Err(_) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                "Invalid JSON page state",
            )
                .into_response();
        }
    };
    let version = DocsVersion::from_code(state.fhir_version).unwrap_or_default();
    let document = match state.upstream.capability_statement(version).await {
        Ok(statement) => statement.document,
        Err(error) => {
            tracing::warn!("CapabilityStatement aggregate fetch failed: {error}");
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "CapabilityStatement is unavailable",
            )
                .into_response();
        }
    };
    let expanded =
        match capability_json::plan_expanded(&document, &pages, fragment_endpoint(&state)) {
            Ok(expanded) => expanded,
            Err(_) => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    "Invalid JSON page state",
                )
                    .into_response();
            }
        };
    let i18n = I18n::new(locale);
    match capability_json::render_expanded(&i18n, &expanded) {
        Ok(html) => Html(html).into_response(),
        Err(capability_json::ExpandedRenderError::TooLarge) => (
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            "CapabilityStatement expansion exceeds the rendering budget",
        )
            .into_response(),
        Err(capability_json::ExpandedRenderError::Template(error)) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("template render error: {error}"),
        )
            .into_response(),
    }
}

fn bounded_fragment(rendered: Result<String, askama::Error>) -> Response {
    match rendered {
        Ok(html) if html.len() <= capability_json::MAX_FRAGMENT_HTML_BYTES => {
            Html(html).into_response()
        }
        Ok(_) => (
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            "CapabilityStatement fragment exceeds the rendering budget",
        )
            .into_response(),
        Err(error) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("template render error: {error}"),
        )
            .into_response(),
    }
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
