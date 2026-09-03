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
//! - `/ui/hts/code-systems/{id}/lookup/json-fragment` (re-issues `$lookup`)
//! - etc.

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
use crate::upstream::{ConceptRef, MappingDirection};

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

// ── Fragment URL builders ───────────────────────────────────────────────

/// Base URL for concept identity JSON fragments.
const IDENTITY_FRAGMENT_URL: &str = "/ui/hts/concepts/identity/json-fragment";
/// Base URL for concept mappings JSON fragments.
const MAPPINGS_FRAGMENT_URL: &str = "/ui/hts/concepts/mappings/json-fragment";

/// Build a fragment URL for concept identity that includes the concept reference.
pub fn identity_fragment_url(reference: &ConceptRef, fhir_version: &str) -> String {
    let endpoint = FragmentEndpoint {
        base_path: IDENTITY_FRAGMENT_URL,
        version: fhir_version,
    };
    let root_url = capability_json::root_fragment_url(endpoint);
    // Append the concept reference parameters using form_urlencoded for correct escaping
    let mut ser = form_urlencoded::Serializer::new(String::new());
    ser.append_pair("system", &reference.system);
    ser.append_pair("code", &reference.code);
    if let Some(version) = &reference.version {
        ser.append_pair("version", version);
    }
    if let Some(display_language) = &reference.display_language {
        ser.append_pair("displayLanguage", display_language);
    }
    format!("{}&{}", root_url, ser.finish())
}

/// Build a fragment URL for concept mappings that includes the concept reference and direction.
pub fn mappings_fragment_url(
    reference: &ConceptRef,
    direction: MappingDirection,
    fhir_version: &str,
) -> String {
    let endpoint = FragmentEndpoint {
        base_path: MAPPINGS_FRAGMENT_URL,
        version: fhir_version,
    };
    let root_url = capability_json::root_fragment_url(endpoint);
    // Append the concept reference and direction parameters using form_urlencoded
    let mut ser = form_urlencoded::Serializer::new(String::new());
    ser.append_pair("system", &reference.system);
    ser.append_pair("code", &reference.code);
    if let Some(version) = &reference.version {
        ser.append_pair("version", version);
    }
    if let Some(display_language) = &reference.display_language {
        ser.append_pair("displayLanguage", display_language);
    }
    ser.append_pair("direction", direction.as_str());
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
