//! Raw JSON fragment endpoints for incremental JSON rendering (#898).
//!
//! The CapabilityStatement page uses an incremental, bounded JSON renderer
//! (`helios_ui_chrome::capability_json`) that fetches one tree level at a time.
//! This module brings the same pattern to the raw response folds in operation
//! workbenches, Concept Explorer, and Import Status.
//!
//! Each fragment endpoint:
//! 1. Re-issues the upstream operation (e.g., `$lookup`, `$translate`) based on
//!    query parameters
//! 2. Plans one bounded level using `capability_json::plan()`
//! 3. Renders either a full view (small subtrees) or an outline (large containers)
//!
//! Every workbench endpoint exists twice: a GET `…/json-fragment` for one
//! bounded level, and a POST `…/json-expand` that takes the client's page
//! descriptors (form-encoded, same wire format as the CapabilityStatement
//! expand-all) and returns the whole tree in one response. Both accept a
//! `target=request|response` parameter selecting which half of the exchange
//! to render — the raw fold shows the POSTed `Parameters` and the response
//! through the same incremental viewer (#898).
//!
//! The endpoints are mounted at paths like:
//! - `/ui/hts/concepts/identity/json-fragment` (re-issues `$lookup`)
//! - `/ui/hts/concepts/mappings/json-fragment` (re-issues `$translate`)
//! - `/ui/hts/code-systems/workbench/lookup/json-fragment` (re-issues `$lookup`)
//! - `/ui/hts/code-systems/workbench/validate/json-fragment` (re-issues `$validate-code`)
//! - `/ui/hts/code-systems/workbench/subsumes/json-fragment` (re-issues `$subsumes`)
//! - `/ui/hts/value-sets/workbench/expand/json-fragment` (re-issues `$expand`)
//! - `/ui/hts/concept-maps/workbench/translate/json-fragment` (re-issues `$translate`)
//!
//! VS `$validate-code` and CM `$closure` are deferred to Slice E.

use axum::{
    Router,
    body::Bytes,
    extract::{Query, State},
    http::HeaderMap,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use helios_ui_chrome::capability_json::{self, FragmentEndpoint};
use serde::Deserialize;
use std::sync::Arc;

use crate::HtsUiState;
use crate::i18n::{I18n, RequestLocale};
use crate::upstream::{
    ConceptRef, ExpandParams, LookupParams, MappingDirection, SubsumesParams, TranslateParams,
    ValidateCodeParams, ValidateInputMode,
};

/// Concept identity (CodeSystem `$lookup`) fragment endpoint query.
#[derive(Debug, Deserialize, Default)]
pub struct ConceptIdentityFragmentQuery {
    // Concept reference parameters
    pub system: String,
    pub code: String,
    pub version: Option<String>,
    #[serde(rename = "displayLanguage")]
    pub display_language: Option<String>,
    // JSON fragment parameters
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub offset: usize,
    pub limit: Option<usize>,
}

/// Concept mappings (ConceptMap `$translate`) fragment endpoint query.
#[derive(Debug, Deserialize, Default)]
pub struct ConceptMappingsFragmentQuery {
    // Concept reference parameters
    pub system: String,
    pub code: String,
    pub version: Option<String>,
    #[serde(rename = "displayLanguage")]
    pub display_language: Option<String>,
    pub direction: Option<String>,
    // JSON fragment parameters
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub offset: usize,
    pub limit: Option<usize>,
}

/// Which half of the raw exchange a workbench fragment/expand request
/// addresses (#898). Carried as `target=` on both endpoint kinds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaneTarget {
    /// The `Parameters` resource the UI POSTed upstream.
    Request,
    /// The upstream response body.
    #[default]
    Response,
}

impl PaneTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Response => "response",
        }
    }

    /// Anything but a literal `request` is the response — a stale or
    /// hand-edited URL still lands on the payload the fold showed by default.
    fn from_query(value: Option<&str>) -> Self {
        if value == Some("request") {
            Self::Request
        } else {
            Self::Response
        }
    }

    fn pick(self, request_body: String, raw_body: String) -> String {
        match self {
            Self::Request => request_body,
            Self::Response => raw_body,
        }
    }
}

// ── Workbench query structs ─────────────────────────────────────────────

/// CodeSystem `$lookup` workbench fragment query.
///
/// Note: The CodeSystem version is renamed to `csVersion` to avoid collision
/// with the FHIR version parameter that `capability_json::root_fragment_url`
/// adds as `version=R4` etc.
#[derive(Debug, Deserialize, Default)]
pub struct CsLookupFragmentQuery {
    pub system: String,
    pub code: String,
    /// CodeSystem version (renamed from `version` to avoid FHIR version collision)
    #[serde(rename = "csVersion")]
    pub cs_version: Option<String>,
    #[serde(rename = "displayLanguage")]
    pub display_language: Option<String>,
    /// Comma-separated list of properties (Axum's default Query doesn't support
    /// repeated params for Vec, so we use a single comma-joined string).
    #[serde(default)]
    pub property: Option<String>,
    pub date: Option<String>,
    /// `request` or `response` — which half of the exchange to render.
    pub target: Option<String>,
    // JSON fragment parameters (version is the FHIR version, handled by
    // capability_json::root_fragment_url)
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub offset: usize,
    pub limit: Option<usize>,
}

/// CodeSystem `$validate-code` workbench fragment query.
#[derive(Debug, Deserialize, Default)]
pub struct CsValidateFragmentQuery {
    pub system: String,
    pub mode: Option<String>,
    pub code: Option<String>,
    pub display: Option<String>,
    #[serde(rename = "coding.system")]
    pub coding_system: Option<String>,
    #[serde(rename = "coding.code")]
    pub coding_code: Option<String>,
    #[serde(rename = "coding.display")]
    pub coding_display: Option<String>,
    #[serde(rename = "displayLanguage")]
    pub display_language: Option<String>,
    /// `request` or `response` — which half of the exchange to render.
    pub target: Option<String>,
    // JSON fragment parameters
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub offset: usize,
    pub limit: Option<usize>,
}

/// CodeSystem `$subsumes` workbench fragment query.
///
/// Note: CodeSystem version renamed to `csVersion` to avoid FHIR version collision.
#[derive(Debug, Deserialize, Default)]
pub struct CsSubsumesFragmentQuery {
    pub system: String,
    #[serde(rename = "codeA")]
    pub code_a: String,
    #[serde(rename = "codeB")]
    pub code_b: String,
    /// CodeSystem version (renamed from `version` to avoid FHIR version collision)
    #[serde(rename = "csVersion")]
    pub cs_version: Option<String>,
    /// `request` or `response` — which half of the exchange to render.
    pub target: Option<String>,
    // JSON fragment parameters
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub offset: usize,
    pub limit: Option<usize>,
}

/// ValueSet `$expand` workbench fragment query.
#[derive(Debug, Deserialize, Default)]
pub struct VsExpandFragmentQuery {
    pub url: String,
    pub filter: Option<String>,
    pub count: Option<String>,
    #[serde(rename = "_offset")]
    pub vs_offset: Option<String>,
    pub mode: Option<String>,
    /// `request` or `response` — which half of the exchange to render.
    pub target: Option<String>,
    // JSON fragment parameters
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub offset: usize,
    pub limit: Option<usize>,
}

/// ConceptMap `$translate` workbench fragment query.
#[derive(Debug, Deserialize, Default)]
pub struct CmTranslateFragmentQuery {
    pub url: String,
    pub direction: Option<String>,
    pub code: Option<String>,
    pub system: Option<String>,
    pub display: Option<String>,
    /// `request` or `response` — which half of the exchange to render.
    pub target: Option<String>,
    // JSON fragment parameters
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub offset: usize,
    pub limit: Option<usize>,
}

// ── Routing ─────────────────────────────────────────────────────────────

/// JSON fragment routes for Concept Explorer panels.
pub fn concept_fragment_routes() -> Router<Arc<HtsUiState>> {
    Router::new()
        .route(
            "/hts/concepts/identity/json-fragment",
            get(concept_identity_fragment),
        )
        .route(
            "/hts/concepts/mappings/json-fragment",
            get(concept_mappings_fragment),
        )
}

/// JSON fragment routes for operation workbenches (#898).
///
/// Note: VS `$validate-code` and CM `$closure` fragment endpoints are deferred
/// to Slice E (standalone workbenches) because their parameter structures are
/// more complex and they're not in the main detail-page workbenches.
pub fn workbench_fragment_routes() -> Router<Arc<HtsUiState>> {
    Router::new()
        // CodeSystem operations
        .route(
            "/hts/code-systems/workbench/lookup/json-fragment",
            get(cs_lookup_fragment),
        )
        .route(
            "/hts/code-systems/workbench/lookup/json-expand",
            post(cs_lookup_expand),
        )
        .route(
            "/hts/code-systems/workbench/validate/json-fragment",
            get(cs_validate_fragment),
        )
        .route(
            "/hts/code-systems/workbench/validate/json-expand",
            post(cs_validate_expand),
        )
        .route(
            "/hts/code-systems/workbench/subsumes/json-fragment",
            get(cs_subsumes_fragment),
        )
        .route(
            "/hts/code-systems/workbench/subsumes/json-expand",
            post(cs_subsumes_expand),
        )
        // ValueSet operations
        .route(
            "/hts/value-sets/workbench/expand/json-fragment",
            get(vs_expand_fragment),
        )
        .route(
            "/hts/value-sets/workbench/expand/json-expand",
            post(vs_expand_expand),
        )
        // ConceptMap operations
        .route(
            "/hts/concept-maps/workbench/translate/json-fragment",
            get(cm_translate_fragment),
        )
        .route(
            "/hts/concept-maps/workbench/translate/json-expand",
            post(cm_translate_expand),
        )
}

// ── Fragment URL builders ───────────────────────────────────────────────

// Concept Explorer fragment URL constants
const IDENTITY_FRAGMENT_URL: &str = "/ui/hts/concepts/identity/json-fragment";
const MAPPINGS_FRAGMENT_URL: &str = "/ui/hts/concepts/mappings/json-fragment";

// Workbench fragment URL constants (#898)
const CS_LOOKUP_FRAGMENT_URL: &str = "/ui/hts/code-systems/workbench/lookup/json-fragment";
const CS_VALIDATE_FRAGMENT_URL: &str = "/ui/hts/code-systems/workbench/validate/json-fragment";
const CS_SUBSUMES_FRAGMENT_URL: &str = "/ui/hts/code-systems/workbench/subsumes/json-fragment";
const VS_EXPAND_FRAGMENT_URL: &str = "/ui/hts/value-sets/workbench/expand/json-fragment";
const CM_TRANSLATE_FRAGMENT_URL: &str = "/ui/hts/concept-maps/workbench/translate/json-fragment";

// Workbench expand-all URL constants (#898)
const CS_LOOKUP_EXPAND_URL: &str = "/ui/hts/code-systems/workbench/lookup/json-expand";
const CS_VALIDATE_EXPAND_URL: &str = "/ui/hts/code-systems/workbench/validate/json-expand";
const CS_SUBSUMES_EXPAND_URL: &str = "/ui/hts/code-systems/workbench/subsumes/json-expand";
const VS_EXPAND_EXPAND_URL: &str = "/ui/hts/value-sets/workbench/expand/json-expand";
const CM_TRANSLATE_EXPAND_URL: &str = "/ui/hts/concept-maps/workbench/translate/json-expand";

// ── Workbench fragment endpoints (#898) ──────────────────────────────────
//
// Each endpoint carries the operation's re-issue parameters (built by the
// matching `*_extra_query` below) so every nested fragment URL the planner
// mints can stand alone.

/// Returns the fragment endpoint for CodeSystem `$lookup` workbench.
pub fn cs_lookup_fragment_endpoint<'a>(
    fhir_version: &'a str,
    extra_query: &'a str,
) -> FragmentEndpoint<'a> {
    FragmentEndpoint {
        base_path: CS_LOOKUP_FRAGMENT_URL,
        version: fhir_version,
        extra_query,
    }
}

/// Returns the fragment endpoint for CodeSystem `$validate-code` workbench.
pub fn cs_validate_fragment_endpoint<'a>(
    fhir_version: &'a str,
    extra_query: &'a str,
) -> FragmentEndpoint<'a> {
    FragmentEndpoint {
        base_path: CS_VALIDATE_FRAGMENT_URL,
        version: fhir_version,
        extra_query,
    }
}

/// Returns the fragment endpoint for CodeSystem `$subsumes` workbench.
pub fn cs_subsumes_fragment_endpoint<'a>(
    fhir_version: &'a str,
    extra_query: &'a str,
) -> FragmentEndpoint<'a> {
    FragmentEndpoint {
        base_path: CS_SUBSUMES_FRAGMENT_URL,
        version: fhir_version,
        extra_query,
    }
}

/// Returns the fragment endpoint for ValueSet `$expand` workbench.
pub fn vs_expand_fragment_endpoint<'a>(
    fhir_version: &'a str,
    extra_query: &'a str,
) -> FragmentEndpoint<'a> {
    FragmentEndpoint {
        base_path: VS_EXPAND_FRAGMENT_URL,
        version: fhir_version,
        extra_query,
    }
}

/// Returns the fragment endpoint for ConceptMap `$translate` workbench.
pub fn cm_translate_fragment_endpoint<'a>(
    fhir_version: &'a str,
    extra_query: &'a str,
) -> FragmentEndpoint<'a> {
    FragmentEndpoint {
        base_path: CM_TRANSLATE_FRAGMENT_URL,
        version: fhir_version,
        extra_query,
    }
}

// ── Workbench expand-all URLs (#898) ─────────────────────────────────────

pub fn cs_lookup_expand_url(extra_query: &str) -> String {
    format!("{CS_LOOKUP_EXPAND_URL}?{extra_query}")
}

pub fn cs_validate_expand_url(extra_query: &str) -> String {
    format!("{CS_VALIDATE_EXPAND_URL}?{extra_query}")
}

pub fn cs_subsumes_expand_url(extra_query: &str) -> String {
    format!("{CS_SUBSUMES_EXPAND_URL}?{extra_query}")
}

pub fn vs_expand_expand_url(extra_query: &str) -> String {
    format!("{VS_EXPAND_EXPAND_URL}?{extra_query}")
}

pub fn cm_translate_expand_url(extra_query: &str) -> String {
    format!("{CM_TRANSLATE_EXPAND_URL}?{extra_query}")
}

// ── Workbench extra-query builders (#898) ────────────────────────────────
//
// The percent-encoded operation parameters plus `target=`, appended to every
// fragment URL (via `FragmentEndpoint::extra_query`) and to the expand-all
// URL, so both endpoint kinds can re-issue the operation.

/// Extra query for the CodeSystem `$lookup` workbench.
///
/// The CodeSystem version is encoded as `csVersion` to avoid collision with
/// the FHIR version parameter (`version=R4`).
pub fn cs_lookup_extra_query(system: &str, params: &LookupParams, target: PaneTarget) -> String {
    let mut ser = form_urlencoded::Serializer::new(String::new());
    ser.append_pair("system", system);
    ser.append_pair("code", &params.code);
    if let Some(version) = &params.version {
        ser.append_pair("csVersion", version);
    }
    if let Some(display_language) = &params.display_language {
        ser.append_pair("displayLanguage", display_language);
    }
    // Join properties with comma (Axum Query doesn't support repeated params for Vec)
    if !params.properties.is_empty() {
        ser.append_pair("property", &params.properties.join(","));
    }
    if let Some(date) = &params.date {
        ser.append_pair("date", date);
    }
    ser.append_pair("target", target.as_str());
    ser.finish()
}

/// Extra query for the CodeSystem `$validate-code` workbench.
pub fn cs_validate_extra_query(
    system: &str,
    params: &ValidateCodeParams,
    target: PaneTarget,
) -> String {
    let mut ser = form_urlencoded::Serializer::new(String::new());
    ser.append_pair("system", system);
    ser.append_pair("mode", params.mode.as_str());
    if !params.code.is_empty() {
        ser.append_pair("code", &params.code);
    }
    if let Some(display) = &params.display {
        ser.append_pair("display", display);
    }
    if !params.coding_system.is_empty() {
        ser.append_pair("coding.system", &params.coding_system);
    }
    if !params.coding_code.is_empty() {
        ser.append_pair("coding.code", &params.coding_code);
    }
    if let Some(coding_display) = &params.coding_display {
        ser.append_pair("coding.display", coding_display);
    }
    if let Some(display_language) = &params.display_language {
        ser.append_pair("displayLanguage", display_language);
    }
    ser.append_pair("target", target.as_str());
    ser.finish()
}

/// Extra query for the CodeSystem `$subsumes` workbench.
pub fn cs_subsumes_extra_query(
    system: &str,
    params: &SubsumesParams,
    target: PaneTarget,
) -> String {
    let mut ser = form_urlencoded::Serializer::new(String::new());
    ser.append_pair("system", system);
    ser.append_pair("codeA", &params.code_a);
    ser.append_pair("codeB", &params.code_b);
    if let Some(version) = &params.version {
        ser.append_pair("csVersion", version);
    }
    ser.append_pair("target", target.as_str());
    ser.finish()
}

/// Extra query for the ValueSet `$expand` workbench.
///
/// `tree_mode` indicates whether the expand was done in tree mode
/// (`hierarchical=true`) or flat mode (`excludeNested=true`).
pub fn vs_expand_extra_query(
    url: &str,
    params: &ExpandParams,
    tree_mode: bool,
    target: PaneTarget,
) -> String {
    let mut ser = form_urlencoded::Serializer::new(String::new());
    ser.append_pair("url", url);
    if let Some(filter) = &params.filter {
        ser.append_pair("filter", filter);
    }
    if let Some(count) = params.count {
        ser.append_pair("count", &count.to_string());
    }
    if let Some(offset) = params.offset {
        ser.append_pair("_offset", &offset.to_string());
    }
    // Encode tree/flat mode
    ser.append_pair("mode", if tree_mode { "tree" } else { "flat" });
    ser.append_pair("target", target.as_str());
    ser.finish()
}

/// Extra query for the ConceptMap `$translate` workbench.
pub fn cm_translate_extra_query(url: &str, params: &TranslateParams, target: PaneTarget) -> String {
    let mut ser = form_urlencoded::Serializer::new(String::new());
    ser.append_pair("url", url);
    ser.append_pair("direction", params.direction.as_str());
    if let Some(code) = &params.code
        && !code.is_empty()
    {
        ser.append_pair("code", code);
    }
    if let Some(system) = &params.system
        && !system.is_empty()
    {
        ser.append_pair("system", system);
    }
    if let Some(display) = &params.display {
        ser.append_pair("display", display);
    }
    ser.append_pair("target", target.as_str());
    ser.finish()
}

// ── Fragment handlers ───────────────────────────────────────────────────

/// Concept identity JSON fragment handler.
///
/// Re-issues `POST /CodeSystem/$lookup` with `property=*` and returns one
/// bounded level of the response JSON.
async fn concept_identity_fragment(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<ConceptIdentityFragmentQuery>,
) -> Response {
    let reference = ConceptRef {
        system: query.system.trim().to_owned(),
        code: query.code.trim().to_owned(),
        version: query
            .version
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty()),
        display_language: query
            .display_language
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty()),
    };

    if !reference.is_addressable() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "Missing system or code parameter",
        )
            .into_response();
    }

    // Re-issue $lookup to get the response
    let identity_result = state.upstream.concept_identity(&reference).await;
    let document = match identity_result {
        Ok(identity) => {
            // Parse the raw_body as JSON
            match serde_json::from_str::<serde_json::Value>(&identity.raw_body) {
                Ok(doc) => doc,
                Err(_) => {
                    return (
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        "Response is not valid JSON",
                    )
                        .into_response();
                }
            }
        }
        Err(error) => {
            tracing::warn!("Concept identity fragment fetch failed: {error}");
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "Concept lookup is unavailable",
            )
                .into_response();
        }
    };

    render_json_fragment(
        &state,
        &locale,
        &document,
        &query.path,
        query.offset,
        query.limit,
        IDENTITY_FRAGMENT_URL,
        "",
    )
}

/// Concept mappings JSON fragment handler.
///
/// Re-issues `POST /ConceptMap/$translate` and returns one bounded level of
/// the response JSON.
async fn concept_mappings_fragment(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<ConceptMappingsFragmentQuery>,
) -> Response {
    let reference = ConceptRef {
        system: query.system.trim().to_owned(),
        code: query.code.trim().to_owned(),
        version: query
            .version
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty()),
        display_language: query
            .display_language
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty()),
    };
    let direction = MappingDirection::from_query(query.direction.as_deref());

    if !reference.is_addressable() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "Missing system or code parameter",
        )
            .into_response();
    }

    // Re-issue $translate to get the response
    let mappings_result = state.upstream.concept_mappings(&reference, direction).await;
    let document = match mappings_result {
        Ok(mappings) => {
            // Parse the raw_body as JSON
            match serde_json::from_str::<serde_json::Value>(&mappings.raw_body) {
                Ok(doc) => doc,
                Err(_) => {
                    return (
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        "Response is not valid JSON",
                    )
                        .into_response();
                }
            }
        }
        Err(error) => {
            tracing::warn!("Concept mappings fragment fetch failed: {error}");
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "Concept mappings lookup is unavailable",
            )
                .into_response();
        }
    };

    render_json_fragment(
        &state,
        &locale,
        &document,
        &query.path,
        query.offset,
        query.limit,
        MAPPINGS_FRAGMENT_URL,
        "",
    )
}

/// Shared fragment rendering logic.
///
/// `extra_query` rides along on every nested fragment URL the planner mints
/// (empty for the Concept Explorer endpoints, the operation's re-issue
/// parameters for the workbenches).
#[allow(clippy::too_many_arguments)]
fn render_json_fragment(
    state: &HtsUiState,
    locale: &RequestLocale,
    document: &serde_json::Value,
    path: &str,
    offset: usize,
    limit: Option<usize>,
    base_path: &str,
    extra_query: &str,
) -> Response {
    let limit = limit.unwrap_or(capability_json::DEFAULT_PAGE_SIZE);
    let i18n = I18n::new(*locale);
    let endpoint = FragmentEndpoint {
        base_path,
        version: state.fhir_version,
        extra_query,
    };

    match capability_json::plan(document, path, offset, limit, endpoint) {
        Ok(capability_json::View::Full(json_lines)) => bounded_fragment(
            capability_json::render_full(&i18n, json_lines, path.is_empty()),
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

fn bounded_fragment(rendered: Result<String, askama::Error>) -> Response {
    match rendered {
        Ok(html) if html.len() <= capability_json::MAX_FRAGMENT_HTML_BYTES => {
            Html(html).into_response()
        }
        Ok(_) => (
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            "JSON fragment exceeds the rendering budget",
        )
            .into_response(),
        Err(error) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("template render error: {error}"),
        )
            .into_response(),
    }
}

/// Shared expand-all rendering logic (#898), mirroring the HFS
/// CapabilityStatement `json-expand` handler: validate the form Content-Type,
/// parse the client's page descriptors, plan the aggregate expansion, and
/// render it whole (or refuse with 413 when it exceeds the budget).
fn render_json_expand(
    state: &HtsUiState,
    locale: &RequestLocale,
    document: &serde_json::Value,
    headers: &HeaderMap,
    body: &[u8],
    base_path: &str,
    extra_query: &str,
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
        return bad_request("Invalid JSON page state");
    }
    let pages = match capability_json::parse_page_descriptors(body) {
        Ok(pages) => pages,
        Err(_) => return bad_request("Invalid JSON page state"),
    };
    let endpoint = FragmentEndpoint {
        base_path,
        version: state.fhir_version,
        extra_query,
    };
    let expanded = match capability_json::plan_expanded(document, &pages, endpoint) {
        Ok(expanded) => expanded,
        Err(_) => return bad_request("Invalid JSON page state"),
    };
    let i18n = I18n::new(*locale);
    match capability_json::render_expanded(&i18n, &expanded) {
        Ok(html) => Html(html).into_response(),
        Err(capability_json::ExpandedRenderError::TooLarge) => (
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            "JSON expansion exceeds the rendering budget",
        )
            .into_response(),
        Err(capability_json::ExpandedRenderError::Template(error)) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("template render error: {error}"),
        )
            .into_response(),
    }
}

fn bad_request(message: &'static str) -> Response {
    (axum::http::StatusCode::BAD_REQUEST, message).into_response()
}

fn unavailable(message: &'static str) -> Response {
    (axum::http::StatusCode::SERVICE_UNAVAILABLE, message).into_response()
}

/// Parses the selected half of the exchange as JSON, or the 500 that tells
/// the viewer nothing renderable came back.
fn parse_payload(payload: &str) -> Result<serde_json::Value, Response> {
    serde_json::from_str(payload).map_err(|_| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "Payload is not valid JSON",
        )
            .into_response()
    })
}

// ── Workbench fragment handlers (#898) ──────────────────────────────────
//
// Each operation has one `*_document` helper that re-issues the upstream
// call and returns the requested half of the exchange (parsed) plus the
// canonical extra query, and two thin handlers on top of it: the GET
// fragment and the POST expand-all.

/// Re-issues CodeSystem `$lookup` and returns the requested payload half.
async fn cs_lookup_document(
    state: &HtsUiState,
    query: CsLookupFragmentQuery,
) -> Result<(serde_json::Value, String), Response> {
    let target = PaneTarget::from_query(query.target.as_deref());
    // Parse comma-separated properties into Vec
    let properties: Vec<String> = query
        .property
        .map(|s| s.split(',').map(|p| p.trim().to_owned()).collect())
        .unwrap_or_default();

    let params = LookupParams {
        code: query.code.trim().to_owned(),
        version: query
            .cs_version
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty()),
        display_language: query
            .display_language
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty()),
        properties,
        date: query.date.filter(|v| !v.trim().is_empty()),
    };
    let system = query.system.trim().to_owned();

    if params.code.is_empty() || system.is_empty() {
        return Err(bad_request("Missing system or code parameter"));
    }

    let extra_query = cs_lookup_extra_query(&system, &params, target);
    let payload = match state.upstream.cs_lookup_type_level(&system, params).await {
        Ok(lookup) => target.pick(lookup.request_body, lookup.raw_body),
        Err(err) => {
            tracing::warn!("CS lookup fragment fetch failed: {err:?}");
            return Err(unavailable("CodeSystem lookup is unavailable"));
        }
    };
    Ok((parse_payload(&payload)?, extra_query))
}

/// CodeSystem `$lookup` workbench fragment handler.
async fn cs_lookup_fragment(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<CsLookupFragmentQuery>,
) -> Response {
    let (path, offset, limit) = (query.path.clone(), query.offset, query.limit);
    match cs_lookup_document(&state, query).await {
        Ok((document, extra_query)) => render_json_fragment(
            &state,
            &locale,
            &document,
            &path,
            offset,
            limit,
            CS_LOOKUP_FRAGMENT_URL,
            &extra_query,
        ),
        Err(response) => response,
    }
}

/// CodeSystem `$lookup` workbench expand-all handler.
async fn cs_lookup_expand(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<CsLookupFragmentQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match cs_lookup_document(&state, query).await {
        Ok((document, extra_query)) => render_json_expand(
            &state,
            &locale,
            &document,
            &headers,
            &body,
            CS_LOOKUP_FRAGMENT_URL,
            &extra_query,
        ),
        Err(response) => response,
    }
}

/// Re-issues CodeSystem `$validate-code` and returns the requested payload half.
async fn cs_validate_document(
    state: &HtsUiState,
    query: CsValidateFragmentQuery,
) -> Result<(serde_json::Value, String), Response> {
    let target = PaneTarget::from_query(query.target.as_deref());
    let mode = ValidateInputMode::from_form(query.mode.as_deref());
    let params = ValidateCodeParams {
        mode,
        code: query.code.unwrap_or_default(),
        display: query.display,
        coding_system: query.coding_system.unwrap_or_default(),
        coding_code: query.coding_code.unwrap_or_default(),
        coding_display: query.coding_display,
        display_language: query.display_language,
    };
    let system = query.system.trim().to_owned();

    if system.is_empty() {
        return Err(bad_request("Missing system parameter"));
    }

    let extra_query = cs_validate_extra_query(&system, &params, target);
    let payload = match state.upstream.cs_validate_code(&system, params).await {
        Ok(validate) => target.pick(validate.request_body, validate.raw_body),
        Err(err) => {
            tracing::warn!("CS validate-code fragment fetch failed: {err:?}");
            return Err(unavailable("CodeSystem validate-code is unavailable"));
        }
    };
    Ok((parse_payload(&payload)?, extra_query))
}

/// CodeSystem `$validate-code` workbench fragment handler.
async fn cs_validate_fragment(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<CsValidateFragmentQuery>,
) -> Response {
    let (path, offset, limit) = (query.path.clone(), query.offset, query.limit);
    match cs_validate_document(&state, query).await {
        Ok((document, extra_query)) => render_json_fragment(
            &state,
            &locale,
            &document,
            &path,
            offset,
            limit,
            CS_VALIDATE_FRAGMENT_URL,
            &extra_query,
        ),
        Err(response) => response,
    }
}

/// CodeSystem `$validate-code` workbench expand-all handler.
async fn cs_validate_expand(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<CsValidateFragmentQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match cs_validate_document(&state, query).await {
        Ok((document, extra_query)) => render_json_expand(
            &state,
            &locale,
            &document,
            &headers,
            &body,
            CS_VALIDATE_FRAGMENT_URL,
            &extra_query,
        ),
        Err(response) => response,
    }
}

/// Re-issues CodeSystem `$subsumes` and returns the requested payload half.
async fn cs_subsumes_document(
    state: &HtsUiState,
    query: CsSubsumesFragmentQuery,
) -> Result<(serde_json::Value, String), Response> {
    let target = PaneTarget::from_query(query.target.as_deref());
    let params = SubsumesParams {
        code_a: query.code_a.trim().to_owned(),
        code_b: query.code_b.trim().to_owned(),
        version: query
            .cs_version
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty()),
    };
    let system = query.system.trim().to_owned();

    if params.code_a.is_empty() || params.code_b.is_empty() || system.is_empty() {
        return Err(bad_request("Missing system, codeA, or codeB parameter"));
    }

    let extra_query = cs_subsumes_extra_query(&system, &params, target);
    let payload = match state.upstream.cs_subsumes(&system, params).await {
        Ok(subsumes) => target.pick(subsumes.request_body, subsumes.raw_body),
        Err(err) => {
            tracing::warn!("CS subsumes fragment fetch failed: {err:?}");
            return Err(unavailable("CodeSystem subsumes is unavailable"));
        }
    };
    Ok((parse_payload(&payload)?, extra_query))
}

/// CodeSystem `$subsumes` workbench fragment handler.
async fn cs_subsumes_fragment(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<CsSubsumesFragmentQuery>,
) -> Response {
    let (path, offset, limit) = (query.path.clone(), query.offset, query.limit);
    match cs_subsumes_document(&state, query).await {
        Ok((document, extra_query)) => render_json_fragment(
            &state,
            &locale,
            &document,
            &path,
            offset,
            limit,
            CS_SUBSUMES_FRAGMENT_URL,
            &extra_query,
        ),
        Err(response) => response,
    }
}

/// CodeSystem `$subsumes` workbench expand-all handler.
async fn cs_subsumes_expand(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<CsSubsumesFragmentQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match cs_subsumes_document(&state, query).await {
        Ok((document, extra_query)) => render_json_expand(
            &state,
            &locale,
            &document,
            &headers,
            &body,
            CS_SUBSUMES_FRAGMENT_URL,
            &extra_query,
        ),
        Err(response) => response,
    }
}

/// Re-issues ValueSet `$expand` and returns the requested payload half.
async fn vs_expand_document(
    state: &HtsUiState,
    query: VsExpandFragmentQuery,
) -> Result<(serde_json::Value, String), Response> {
    let target = PaneTarget::from_query(query.target.as_deref());
    // Convert mode string to hierarchical/exclude_nested flags
    let tree_mode = query.mode.as_deref() == Some("tree");
    let params = ExpandParams {
        filter: query.filter.filter(|v| !v.trim().is_empty()),
        count: query.count.as_deref().and_then(|s| s.trim().parse().ok()),
        offset: query
            .vs_offset
            .as_deref()
            .and_then(|s| s.trim().parse().ok()),
        hierarchical: if tree_mode { Some(true) } else { None },
        exclude_nested: if !tree_mode { Some(true) } else { None },
        ..Default::default()
    };
    let url = query.url.trim().to_owned();

    if url.is_empty() {
        return Err(bad_request("Missing url parameter"));
    }

    let extra_query = vs_expand_extra_query(&url, &params, tree_mode, target);
    let payload = match state.upstream.vs_expand_by_url(&url, &params).await {
        Ok(expand) => target.pick(expand.request_body, expand.raw_body),
        Err(err) => {
            tracing::warn!("VS expand fragment fetch failed: {err:?}");
            return Err(unavailable("ValueSet expand is unavailable"));
        }
    };
    Ok((parse_payload(&payload)?, extra_query))
}

/// ValueSet `$expand` workbench fragment handler.
async fn vs_expand_fragment(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<VsExpandFragmentQuery>,
) -> Response {
    let (path, offset, limit) = (query.path.clone(), query.offset, query.limit);
    match vs_expand_document(&state, query).await {
        Ok((document, extra_query)) => render_json_fragment(
            &state,
            &locale,
            &document,
            &path,
            offset,
            limit,
            VS_EXPAND_FRAGMENT_URL,
            &extra_query,
        ),
        Err(response) => response,
    }
}

/// ValueSet `$expand` workbench expand-all handler.
async fn vs_expand_expand(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<VsExpandFragmentQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match vs_expand_document(&state, query).await {
        Ok((document, extra_query)) => render_json_expand(
            &state,
            &locale,
            &document,
            &headers,
            &body,
            VS_EXPAND_FRAGMENT_URL,
            &extra_query,
        ),
        Err(response) => response,
    }
}

/// Re-issues ConceptMap `$translate` and returns the requested payload half.
async fn cm_translate_document(
    state: &HtsUiState,
    query: CmTranslateFragmentQuery,
) -> Result<(serde_json::Value, String), Response> {
    use crate::upstream::TranslateDirection;

    let target = PaneTarget::from_query(query.target.as_deref());
    let direction = TranslateDirection::from_form(query.direction.as_deref());
    let params = TranslateParams {
        direction,
        code: query.code,
        system: query.system,
        display: query.display,
        target_code: None,
        target_system: None,
        source_url: None,
        target_url: None,
        date: None,
    };
    let url = query.url.trim().to_owned();

    if url.is_empty() {
        return Err(bad_request("Missing url parameter"));
    }

    let extra_query = cm_translate_extra_query(&url, &params, target);
    let payload = match state.upstream.cm_translate_by_url(&url, &params).await {
        Ok(translate) => target.pick(translate.request_body, translate.raw_body),
        Err(err) => {
            tracing::warn!("CM translate fragment fetch failed: {err:?}");
            return Err(unavailable("ConceptMap translate is unavailable"));
        }
    };
    Ok((parse_payload(&payload)?, extra_query))
}

/// ConceptMap `$translate` workbench fragment handler.
async fn cm_translate_fragment(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<CmTranslateFragmentQuery>,
) -> Response {
    let (path, offset, limit) = (query.path.clone(), query.offset, query.limit);
    match cm_translate_document(&state, query).await {
        Ok((document, extra_query)) => render_json_fragment(
            &state,
            &locale,
            &document,
            &path,
            offset,
            limit,
            CM_TRANSLATE_FRAGMENT_URL,
            &extra_query,
        ),
        Err(response) => response,
    }
}

/// ConceptMap `$translate` workbench expand-all handler.
async fn cm_translate_expand(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<CmTranslateFragmentQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match cm_translate_document(&state, query).await {
        Ok((document, extra_query)) => render_json_expand(
            &state,
            &locale,
            &document,
            &headers,
            &body,
            CM_TRANSLATE_FRAGMENT_URL,
            &extra_query,
        ),
        Err(response) => response,
    }
}
