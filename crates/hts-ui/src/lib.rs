//! Server-rendered, HTMX-first administrative UI for the Helios Terminology
//! Server (HTS).
//!
//! This crate follows the same rules of the road as [`helios_ui`]: handlers
//! parse the request, gather data over the HTS HTTP surface, and render an
//! [`askama`] template. All markup lives in `templates/`; static assets
//! (htmx, CSS, JS) are embedded at compile time via [`rust_embed`] and served
//! by [`axum_embed`], so there is no runtime CDN dependency (#551 D6).
//!
//! Handlers branch on the `HX-Request` header — read through the infallible
//! [`axum_htmx::HxRequest`] extractor — to return an HTML *fragment* for
//! htmx-driven swaps and a *full page* for hard navigations, so the UI
//! degrades to working full-page loads without JavaScript.
//!
//! # Mounting
//!
//! [`router`] returns an [`axum::Router`] meant to be mounted at `/ui` in the
//! HTS binary. The router uses the `/hts` prefix internally, so the resulting
//! URL space is `/ui/hts`, `/ui/hts/code-systems`, etc. — matching the design
//! document at `edson/docs/hts-ui-design.md` §5.1.
//!
//! # Upstream contract
//!
//! HTS-UI ships inside the `hts` binary but the handlers still speak HTTP to
//! the HTS REST surface. That keeps the UI honest (every card and cell is
//! what a browser sees), and makes the `HTS_UI_UPSTREAM_URL` override — the
//! canonical degraded-state trigger per design doc §7 — useful without a
//! rebuild. When neither is set the mount site derives a loopback URL from
//! the binary's own host:port. `HFS_TERMINOLOGY_SERVER` is HFS-side and has
//! no meaning here.
//!
//! # Assets during Phase 1
//!
//! The `Assets` embed points at `../ui/assets`. HTS-UI and HFS-UI share CSS
//! and vendored htmx during Phase 1; Phase 8 (post-#543) will extract those
//! into a `helios-ui-chrome` crate (see design doc §9.2).

mod code_systems;
mod concept_maps;
mod dashboard;
mod diagnostics;
mod i18n;
mod import;
mod operations;
mod upstream;
mod value_sets;

use axum::{
    Router,
    response::{Html, IntoResponse, Response},
};
use axum_embed::ServeEmbed;
use axum_htmx::AutoVaryLayer;
use i18n::I18n;
use rust_embed::RustEmbed;
use std::sync::Arc;

pub use i18n::{negotiate_locale, RequestLocale};
pub use operations::BatchJobs;
pub use upstream::{
    ClosureConcept, ClosureEdge, ClosureParams, ClosureResult, CmBrowserFilters, CmBrowserPage,
    CmBrowserRow, CodeSystemSummary, ConceptMapSummary, CsBrowserFilters, CsBrowserPage,
    CsBrowserRow, ExpandParams, ExpansionConcept, ExpansionDesignation, ExpansionResult,
    ImportCounts, ImportResult, ImportStatus, LookupDesignation, LookupParams, LookupProperty,
    LookupResult, MappingKind, OutcomeView, SubsumesParams, SubsumesResult, TranslateDirection,
    TranslateMatch, TranslateParams, TranslateResult, UpstreamCapabilitiesCodeSystem,
    UpstreamClient, UpstreamError, UpstreamHealth, UpstreamTerminologyCapabilities,
    ValidateCodeParams, ValidateCodeResult, ValidateInputMode, ValueSetSummary, VsBrowserFilters,
    VsBrowserPage, VsBrowserRow, VsValidateMode, VsValidateParams, VsValidateResult,
    VsValidateSource, HTS_UI_BATCH_FANOUT_CONCURRENCY, HTS_UI_MAX_EXPANSION_SIZE_HINT,
};
// Slice G additions (diagnostics, §7.9). Appended below Slice F's block to
// avoid touching the alphabetized list.
pub use upstream::{
    CapabilityRestResource, CapabilityView, TerminologyCapabilitiesView,
    TerminologyCodeSystemEntry,
};

/// Static UI assets (htmx, CSS, JS) embedded into the binary at compile time.
///
/// Points at the sibling `crates/ui/assets` directory: during Phase 1 the two
/// products share bytes to avoid duplication (see design doc §9.2). Phase 8
/// extracts these to a shared `helios-ui-chrome` crate.
#[derive(Clone, RustEmbed)]
#[folder = "../ui/assets"]
struct Assets;

/// Shared router state: values that are constant for the process lifetime.
///
/// Cheap to `Arc<HtsUiState>::clone`.
#[derive(Clone)]
pub struct HtsUiState {
    /// The FHIR version the `hts` binary was built for (`R4`, `R4B`, `R5`,
    /// or `R6`). Rendered in the sidebar as a metadata chip — compile-time
    /// constant, not an interactive selector (design doc §7.1 HTS binary
    /// chrome contract).
    pub fhir_version: &'static str,

    /// The `hts` binary version string, wired to `env!("CARGO_PKG_VERSION")`
    /// at the mount site and shown next to the product name.
    pub version: &'static str,

    /// Upstream HTS HTTP client. Base URL comes from `HTS_UI_UPSTREAM_URL`
    /// when set, otherwise loopback to the same binary (design doc §7
    /// degraded state contract).
    pub upstream: UpstreamClient,

    /// Total on-disk size of the configured `HTS_BOOTSTRAP_DIR` in bytes.
    /// `None` when no bootstrap directory was set — the dashboard tile then
    /// renders an em-dash rather than a misleading zero.
    pub bundled_data_bytes: Option<u64>,
}

/// Build the HTS UI router.
///
/// Mount this at `/ui` in the HTS binary so the routes below become
/// `/ui/hts`, `/ui/hts/assets/*`, etc.
pub fn router(state: Arc<HtsUiState>) -> Router {
    Router::new()
        .merge(dashboard::routes())
        .merge(code_systems::routes())
        .merge(value_sets::routes())
        .merge(concept_maps::routes())
        .merge(operations::routes())
        .merge(import::routes())
        .merge(diagnostics::routes())
        .nest_service("/hts/assets", ServeEmbed::<Assets>::new())
        .with_state(state)
        .layer(axum::middleware::from_fn(i18n::negotiate_locale))
        .layer(AutoVaryLayer)
}

// ── Page context ────────────────────────────────────────────────────────────

/// Values every HTS-UI page needs for the sidebar/topbar chrome.
///
/// Kept as a plain struct (not a template macro) so each page template can
/// embed it via `{% include %}` or destructure fields, and so tests can
/// build one without going through a full request.
#[derive(Clone, Copy)]
pub(crate) struct Chrome<'a> {
    pub i18n: I18n,
    pub active_page: &'a str,
    pub fhir_version: &'a str,
    pub version: &'a str,
}

/// Askama render helper for full-page and fragment responses that already
/// carry the entire body content. Wraps the result in [`axum::response::Html`]
/// with a diagnostic 500 fallback so template failures fail loudly instead of
/// serving blank pages.
pub(crate) fn render_page(rendered: Result<String, askama::Error>) -> Response {
    match rendered {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!(?err, "hts-ui template render failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Html(String::from("<pre>hts-ui: template render error</pre>")),
            )
                .into_response()
        }
    }
}

/// Cheap constructor for local mount tests: an [`HtsUiState`] pointed at a
/// closed loopback port so upstream calls fail deterministically.
#[cfg(test)]
#[allow(dead_code)]
fn test_state() -> Arc<HtsUiState> {
    Arc::new(HtsUiState {
        fhir_version: "R4",
        version: "0.0.0-test",
        upstream: UpstreamClient::new("http://127.0.0.1:1").expect("test client"),
        bundled_data_bytes: None,
    })
}
