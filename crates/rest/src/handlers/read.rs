//! Read interaction handler.
//!
//! Implements the FHIR [read interaction](https://hl7.org/fhir/http.html#read):
//! `GET [base]/[type]/[id]`

use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use helios_persistence::core::ResourceStorage;
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirVersionExtractor, TenantExtractor};
use crate::middleware::conditional::ConditionalHeaders;
use crate::middleware::content_type::{FhirContentType, negotiate_format};
use crate::responses::format_resource_response;
use crate::responses::headers::ResourceHeaders;
use crate::responses::subsetting::{SummaryMode, apply_elements, apply_summary};
use crate::state::AppState;

/// Handler for the read interaction.
///
/// Reads a resource by type and ID, returning the current version.
///
/// # HTTP Request
///
/// `GET [base]/[type]/[id]`
///
/// # Headers
///
/// - `Accept` - Content type negotiation (default: application/fhir+json)
/// - `If-None-Match` - Return 304 Not Modified if ETag matches
/// - `If-Modified-Since` - Return 304 Not Modified if not modified since date
///
/// # Response
///
/// - `200 OK` - Resource found, returns the resource
/// - `304 Not Modified` - Resource unchanged (conditional read)
/// - `404 Not Found` - Resource does not exist
/// - `410 Gone` - Resource was deleted
///
/// # Example
///
/// ```http
/// GET /Patient/123 HTTP/1.1
/// Host: fhir.example.com
/// Accept: application/fhir+json
/// ```
pub async fn read_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    conditional: ConditionalHeaders,
    req_headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    debug!(
        resource_type = %resource_type,
        id = %id,
        tenant = %tenant.tenant_id(),
        "Processing read request"
    );

    // Read the resource
    let resource = state
        .storage()
        .read(tenant.context(), &resource_type, &id)
        .await?;

    match resource {
        Some(stored) => {
            // If client requested specific version, verify match
            if let Some(requested) = version.accept_version() {
                if stored.fhir_version() != requested {
                    return Err(RestError::NotAcceptable {
                        message: format!(
                            "Resource is FHIR {} but {} was requested",
                            stored.fhir_version().as_mime_param(),
                            requested.as_mime_param()
                        ),
                    });
                }
            }

            // Check conditional headers (If-None-Match)
            if let Some(etag) = conditional.if_none_match() {
                let resource_etag = format!("W/\"{}\"", stored.version_id());
                if etag == resource_etag || etag == "*" {
                    debug!(etag = %resource_etag, "Returning 304 Not Modified");
                    return Ok(StatusCode::NOT_MODIFIED.into_response());
                }
            }

            // Check If-Modified-Since
            if let Some(since) = conditional.if_modified_since() {
                let last_modified = stored.last_modified();
                if last_modified <= since {
                    debug!("Resource not modified since {}", since);
                    return Ok(StatusCode::NOT_MODIFIED.into_response());
                }
            }

            // Negotiate response format
            let format_param = params.get("_format").map(|s| s.as_str());
            let negotiated = negotiate_format(&req_headers, format_param);

            // Build response headers, including fhirVersion in Content-Type
            let content_type =
                FhirContentType::with_version(negotiated.format, stored.fhir_version());
            let mut headers = ResourceHeaders::from_stored(&stored, &state)
                .with_content_type(content_type.to_header_value())
                .to_header_map();
            headers.insert(
                header::CONTENT_TYPE,
                content_type.to_header_value().parse().unwrap(),
            );

            // Apply subsetting if _summary or _elements specified
            let summary_mode = params.get("_summary").and_then(|v| SummaryMode::parse(v));
            let elements: Option<Vec<&str>> = params
                .get("_elements")
                .map(|v| v.split(',').map(|s| s.trim()).collect());

            let mut content = stored.content().clone();
            let mut subsetted = false;

            if let Some(mode) = summary_mode {
                content = apply_summary(&content, mode, stored.fhir_version());
                if mode != SummaryMode::False {
                    subsetted = true;
                }
            }
            if let Some(ref elem_list) = elements {
                if !elem_list.is_empty() {
                    content = apply_elements(&content, elem_list);
                    subsetted = true;
                }
            }

            // Flag incomplete representations with the SUBSETTED tag (FHIR spec).
            if subsetted {
                crate::responses::subsetting::add_subsetted_tag(&mut content);
            }

            // Return the resource
            debug!(
                resource_type = %resource_type,
                id = %id,
                version = %stored.version_id(),
                fhir_version = %stored.fhir_version(),
                format = ?negotiated.format,
                summary = ?summary_mode,
                elements = ?elements,
                "Returning resource"
            );

            format_resource_response(StatusCode::OK, headers, &content, negotiated.format).map_err(
                |_| RestError::InternalError {
                    message: "Failed to serialize response".to_string(),
                },
            )
        }
        None => {
            debug!(
                resource_type = %resource_type,
                id = %id,
                "Resource not found"
            );
            Err(RestError::NotFound { resource_type, id })
        }
    }
}

/// Handler for HEAD read interaction.
///
/// Returns headers for a resource without the body.
///
/// # HTTP Request
///
/// `HEAD [base]/[type]/[id]`
///
/// # Response
///
/// - `200 OK` - Resource exists, headers returned
/// - `304 Not Modified` - Resource unchanged (conditional read)
/// - `404 Not Found` - Resource does not exist
///
/// This is useful for checking resource existence and metadata without
/// transferring the full resource content.
pub async fn head_read_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    conditional: ConditionalHeaders,
    req_headers: HeaderMap,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    debug!(
        resource_type = %resource_type,
        id = %id,
        tenant = %tenant.tenant_id(),
        "Processing HEAD read request"
    );

    // Read the resource
    let resource = state
        .storage()
        .read(tenant.context(), &resource_type, &id)
        .await?;

    match resource {
        Some(stored) => {
            // If client requested specific version, verify match
            if let Some(requested) = version.accept_version() {
                if stored.fhir_version() != requested {
                    return Err(RestError::NotAcceptable {
                        message: format!(
                            "Resource is FHIR {} but {} was requested",
                            stored.fhir_version().as_mime_param(),
                            requested.as_mime_param()
                        ),
                    });
                }
            }

            // Check conditional headers (If-None-Match)
            if let Some(etag) = conditional.if_none_match() {
                let resource_etag = format!("W/\"{}\"", stored.version_id());
                if etag == resource_etag || etag == "*" {
                    debug!(etag = %resource_etag, "Returning 304 Not Modified");
                    return Ok(StatusCode::NOT_MODIFIED.into_response());
                }
            }

            // Check If-Modified-Since
            if let Some(since) = conditional.if_modified_since() {
                let last_modified = stored.last_modified();
                if last_modified <= since {
                    debug!("Resource not modified since {}", since);
                    return Ok(StatusCode::NOT_MODIFIED.into_response());
                }
            }

            // Negotiate response format
            let negotiated = negotiate_format(&req_headers, None);

            // Build response headers
            let content_type =
                FhirContentType::with_version(negotiated.format, stored.fhir_version());
            let mut headers = ResourceHeaders::from_stored(&stored, &state)
                .with_content_type(content_type.to_header_value())
                .to_header_map();
            headers.insert(
                header::CONTENT_TYPE,
                content_type.to_header_value().parse().unwrap(),
            );

            // Return headers only (no body)
            debug!(
                resource_type = %resource_type,
                id = %id,
                version = %stored.version_id(),
                "Returning HEAD response"
            );

            Ok((StatusCode::OK, headers).into_response())
        }
        None => {
            debug!(
                resource_type = %resource_type,
                id = %id,
                "Resource not found"
            );
            Err(RestError::NotFound { resource_type, id })
        }
    }
}

#[cfg(test)]
mod tests {
    // Tests will be added in the integration test suite
}
