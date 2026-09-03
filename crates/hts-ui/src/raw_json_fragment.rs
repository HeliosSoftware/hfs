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
//! The endpoints are mounted at paths like:
//! - `/ui/hts/concepts/identity/json-fragment` (re-issues `$lookup`)
//! - `/ui/hts/concepts/mappings/json-fragment` (re-issues `$translate`)
//! - `/ui/hts/code-systems/workbench/lookup/json-fragment` (re-issues `$lookup`)
//! - `/ui/hts/code-systems/workbench/validate/json-fragment` (re-issues `$validate-code`)
//! - `/ui/hts/code-systems/workbench/subsumes/json-fragment` (re-issues `$subsumes`)
//! - `/ui/hts/value-sets/workbench/expand/json-fragment` (re-issues `$expand`)
//! - `/ui/hts/value-sets/workbench/validate/json-fragment` (re-issues `$validate-code`)
//! - `/ui/hts/concept-maps/workbench/translate/json-fragment` (re-issues `$translate`)
//! - `/ui/hts/concept-maps/workbench/closure/json-fragment` (re-issues `$closure`)

use axum::{
    Router,
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
    routing::get,
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
            "/hts/code-systems/workbench/validate/json-fragment",
            get(cs_validate_fragment),
        )
        .route(
            "/hts/code-systems/workbench/subsumes/json-fragment",
            get(cs_subsumes_fragment),
        )
        // ValueSet operations
        .route(
            "/hts/value-sets/workbench/expand/json-fragment",
            get(vs_expand_fragment),
        )
        // ConceptMap operations
        .route(
            "/hts/concept-maps/workbench/translate/json-fragment",
            get(cm_translate_fragment),
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

// ── Workbench fragment URL builders (#898) ──────────────────────────────

/// Build a fragment URL for CodeSystem `$lookup` workbench.
///
/// The CodeSystem version is encoded as `csVersion` to avoid collision with
/// the FHIR version parameter (`version=R4`).
pub fn cs_lookup_fragment_url(system: &str, params: &LookupParams, fhir_version: &str) -> String {
    let endpoint = FragmentEndpoint {
        base_path: CS_LOOKUP_FRAGMENT_URL,
        version: fhir_version,
    };
    let root_url = capability_json::root_fragment_url(endpoint);
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
    format!("{}&{}", root_url, ser.finish())
}

/// Build a fragment URL for CodeSystem `$validate-code` workbench.
pub fn cs_validate_fragment_url(
    system: &str,
    params: &ValidateCodeParams,
    fhir_version: &str,
) -> String {
    let endpoint = FragmentEndpoint {
        base_path: CS_VALIDATE_FRAGMENT_URL,
        version: fhir_version,
    };
    let root_url = capability_json::root_fragment_url(endpoint);
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
    format!("{}&{}", root_url, ser.finish())
}

/// Build a fragment URL for CodeSystem `$subsumes` workbench.
///
/// The CodeSystem version is encoded as `csVersion` to avoid collision with
/// the FHIR version parameter (`version=R4`).
pub fn cs_subsumes_fragment_url(
    system: &str,
    params: &SubsumesParams,
    fhir_version: &str,
) -> String {
    let endpoint = FragmentEndpoint {
        base_path: CS_SUBSUMES_FRAGMENT_URL,
        version: fhir_version,
    };
    let root_url = capability_json::root_fragment_url(endpoint);
    let mut ser = form_urlencoded::Serializer::new(String::new());
    ser.append_pair("system", system);
    ser.append_pair("codeA", &params.code_a);
    ser.append_pair("codeB", &params.code_b);
    if let Some(version) = &params.version {
        ser.append_pair("csVersion", version);
    }
    format!("{}&{}", root_url, ser.finish())
}

/// Build a fragment URL for ValueSet `$expand` workbench.
///
/// `tree_mode` indicates whether the expand was done in tree mode (`hierarchical=true`)
/// or flat mode (`excludeNested=true`).
pub fn vs_expand_fragment_url(
    url: &str,
    params: &ExpandParams,
    tree_mode: bool,
    fhir_version: &str,
) -> String {
    let endpoint = FragmentEndpoint {
        base_path: VS_EXPAND_FRAGMENT_URL,
        version: fhir_version,
    };
    let root_url = capability_json::root_fragment_url(endpoint);
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
    format!("{}&{}", root_url, ser.finish())
}

/// Build a fragment URL for ConceptMap `$translate` workbench.
pub fn cm_translate_fragment_url(
    url: &str,
    params: &TranslateParams,
    fhir_version: &str,
) -> String {
    let endpoint = FragmentEndpoint {
        base_path: CM_TRANSLATE_FRAGMENT_URL,
        version: fhir_version,
    };
    let root_url = capability_json::root_fragment_url(endpoint);
    let mut ser = form_urlencoded::Serializer::new(String::new());
    ser.append_pair("url", url);
    ser.append_pair("direction", params.direction.as_str());
    if let Some(code) = &params.code {
        if !code.is_empty() {
            ser.append_pair("code", code);
        }
    }
    if let Some(system) = &params.system {
        if !system.is_empty() {
            ser.append_pair("system", system);
        }
    }
    if let Some(display) = &params.display {
        ser.append_pair("display", display);
    }
    format!("{}&{}", root_url, ser.finish())
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
    )
}

/// Shared fragment rendering logic.
fn render_json_fragment(
    state: &HtsUiState,
    locale: &RequestLocale,
    document: &serde_json::Value,
    path: &str,
    offset: usize,
    limit: Option<usize>,
    base_path: &str,
) -> Response {
    let limit = limit.unwrap_or(capability_json::DEFAULT_PAGE_SIZE);
    let i18n = I18n::new(*locale);
    let endpoint = FragmentEndpoint {
        base_path,
        version: state.fhir_version,
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

// ── Workbench fragment handlers (#898) ──────────────────────────────────

/// CodeSystem `$lookup` workbench fragment handler.
async fn cs_lookup_fragment(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<CsLookupFragmentQuery>,
) -> Response {
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

    if params.code.is_empty() || query.system.trim().is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "Missing system or code parameter",
        )
            .into_response();
    }

    let result = state
        .upstream
        .cs_lookup_type_level(query.system.trim(), params)
        .await;

    let document = match result {
        Ok(lookup) => match serde_json::from_str::<serde_json::Value>(&lookup.raw_body) {
            Ok(doc) => doc,
            Err(_) => {
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "Response is not valid JSON",
                )
                    .into_response();
            }
        },
        Err(err) => {
            tracing::warn!("CS lookup fragment fetch failed: {err:?}");
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "CodeSystem lookup is unavailable",
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
        CS_LOOKUP_FRAGMENT_URL,
    )
}

/// CodeSystem `$validate-code` workbench fragment handler.
async fn cs_validate_fragment(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<CsValidateFragmentQuery>,
) -> Response {
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

    if query.system.trim().is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "Missing system parameter",
        )
            .into_response();
    }

    let result = state
        .upstream
        .cs_validate_code(query.system.trim(), params)
        .await;

    let document = match result {
        Ok(validate) => match serde_json::from_str::<serde_json::Value>(&validate.raw_body) {
            Ok(doc) => doc,
            Err(_) => {
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "Response is not valid JSON",
                )
                    .into_response();
            }
        },
        Err(err) => {
            tracing::warn!("CS validate-code fragment fetch failed: {err:?}");
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "CodeSystem validate-code is unavailable",
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
        CS_VALIDATE_FRAGMENT_URL,
    )
}

/// CodeSystem `$subsumes` workbench fragment handler.
async fn cs_subsumes_fragment(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<CsSubsumesFragmentQuery>,
) -> Response {
    let params = SubsumesParams {
        code_a: query.code_a.trim().to_owned(),
        code_b: query.code_b.trim().to_owned(),
        version: query
            .cs_version
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty()),
    };

    if params.code_a.is_empty() || params.code_b.is_empty() || query.system.trim().is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "Missing system, codeA, or codeB parameter",
        )
            .into_response();
    }

    let result = state
        .upstream
        .cs_subsumes(query.system.trim(), params)
        .await;

    let document = match result {
        Ok(subsumes) => match serde_json::from_str::<serde_json::Value>(&subsumes.raw_body) {
            Ok(doc) => doc,
            Err(_) => {
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "Response is not valid JSON",
                )
                    .into_response();
            }
        },
        Err(err) => {
            tracing::warn!("CS subsumes fragment fetch failed: {err:?}");
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "CodeSystem subsumes is unavailable",
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
        CS_SUBSUMES_FRAGMENT_URL,
    )
}

/// ValueSet `$expand` workbench fragment handler.
async fn vs_expand_fragment(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<VsExpandFragmentQuery>,
) -> Response {
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

    if query.url.trim().is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST, "Missing url parameter").into_response();
    }

    let result = state
        .upstream
        .vs_expand_by_url(query.url.trim(), &params)
        .await;

    let document = match result {
        Ok(expand) => match serde_json::from_str::<serde_json::Value>(&expand.raw_body) {
            Ok(doc) => doc,
            Err(_) => {
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "Response is not valid JSON",
                )
                    .into_response();
            }
        },
        Err(err) => {
            tracing::warn!("VS expand fragment fetch failed: {err:?}");
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "ValueSet expand is unavailable",
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
        VS_EXPAND_FRAGMENT_URL,
    )
}

/// ConceptMap `$translate` workbench fragment handler.
async fn cm_translate_fragment(
    State(state): State<Arc<HtsUiState>>,
    locale: RequestLocale,
    Query(query): Query<CmTranslateFragmentQuery>,
) -> Response {
    use crate::upstream::TranslateDirection;

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

    if query.url.trim().is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST, "Missing url parameter").into_response();
    }

    let result = state
        .upstream
        .cm_translate_by_url(query.url.trim(), &params)
        .await;

    let document = match result {
        Ok(translate) => match serde_json::from_str::<serde_json::Value>(&translate.raw_body) {
            Ok(doc) => doc,
            Err(_) => {
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "Response is not valid JSON",
                )
                    .into_response();
            }
        },
        Err(err) => {
            tracing::warn!("CM translate fragment fetch failed: {err:?}");
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "ConceptMap translate is unavailable",
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
        CM_TRANSLATE_FRAGMENT_URL,
    )
}
