//! Update interaction handler.
//!
//! Implements the FHIR [update interaction](https://hl7.org/fhir/http.html#update):
//! `PUT [base]/[type]/[id]`

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use helios_persistence::core::{ConditionalStorage, ResourceStorage};
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirResource, FhirVersionExtractor, TenantExtractor};
use crate::middleware::conditional::ConditionalHeaders;
use crate::middleware::content_type::{FhirFormat, negotiate_format};
use crate::middleware::prefer::PreferHeader;
use crate::responses::format_resource_response;
use crate::responses::headers::ResourceHeaders;
use crate::state::AppState;

/// Handler for the update interaction.
///
/// Updates an existing resource, or creates it if it doesn't exist (upsert).
///
/// # HTTP Request
///
/// `PUT [base]/[type]/[id]`
///
/// # Headers
///
/// - `Content-Type` - Must be application/fhir+json or application/fhir+xml
/// - `If-Match` - Optimistic locking (ETag of current version)
/// - `Prefer` - Response preference
///
/// # Response
///
/// - `200 OK` - Resource updated successfully
/// - `201 Created` - Resource created (upsert)
/// - `400 Bad Request` - Invalid resource
/// - `409 Conflict` - Version conflict (concurrent modification)
/// - `412 Precondition Failed` - If-Match condition not met
///
/// # Example
///
/// ```http
/// PUT /Patient/123 HTTP/1.1
/// Host: fhir.example.com
/// Content-Type: application/fhir+json
/// If-Match: W/"1"
///
/// {"resourceType": "Patient", "id": "123", "name": [{"family": "Smith"}]}
/// ```
#[allow(clippy::too_many_arguments)]
pub async fn update_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    conditional: ConditionalHeaders,
    prefer: PreferHeader,
    req_headers: HeaderMap,
    FhirResource(resource): FhirResource,
) -> RestResult<Response>
where
    S: ResourceStorage + ConditionalStorage + Send + Sync,
{
    // AuditEvent resources are immutable — block write operations
    if resource_type == "AuditEvent" {
        return Err(RestError::MethodNotAllowed {
            method: "PUT".to_string(),
            resource_type: resource_type.to_string(),
        });
    }

    // Determine FHIR version from header or use server default
    let fhir_version = version.storage_version();

    // Negotiate response format from Accept header
    let negotiated = negotiate_format(&req_headers, None);

    debug!(
        resource_type = %resource_type,
        id = %id,
        tenant = %tenant.tenant_id(),
        fhir_version = %fhir_version,
        if_match = ?conditional.if_match(),
        "Processing update request"
    );

    // Validate resourceType in body matches URL
    if let Some(body_type) = resource.get("resourceType").and_then(|v| v.as_str()) {
        if body_type != resource_type {
            return Err(RestError::BadRequest {
                message: format!(
                    "Resource type in body ({}) does not match URL ({})",
                    body_type, resource_type
                ),
            });
        }
    } else {
        return Err(RestError::BadRequest {
            message: "Resource must contain resourceType".to_string(),
        });
    }

    // Validate ID in body matches URL (if present)
    if let Some(body_id) = resource.get("id").and_then(|v| v.as_str()) {
        if body_id != id {
            return Err(RestError::BadRequest {
                message: format!(
                    "Resource ID in body ({}) does not match URL ({})",
                    body_id, id
                ),
            });
        }
    }

    // Check if If-Match is required
    if state.require_if_match() && conditional.if_match().is_none() {
        return Err(RestError::PreconditionFailed {
            message: "If-Match header is required for updates".to_string(),
        });
    }

    // Try to read existing resource for version check
    let existing = state
        .storage()
        .read(tenant.context(), &resource_type, &id)
        .await?;

    // Handle If-Match precondition
    if let Some(if_match) = conditional.if_match() {
        match &existing {
            Some(stored) => {
                let current_etag = format!("W/\"{}\"", stored.version_id());
                if if_match != current_etag && if_match != "*" {
                    return Err(RestError::PreconditionFailed {
                        message: format!(
                            "ETag mismatch: expected {}, got {}",
                            if_match, current_etag
                        ),
                    });
                }
            }
            None => {
                // If-Match with no existing resource is a precondition failure
                // (unless If-Match: * which means "any version")
                if if_match != "*" {
                    return Err(RestError::PreconditionFailed {
                        message: "Resource does not exist".to_string(),
                    });
                }
            }
        }
    }

    // Perform the update (or create)
    let (stored, created) = state
        .storage()
        .create_or_update(
            tenant.context(),
            &resource_type,
            &id,
            resource,
            fhir_version,
        )
        .await?;

    let headers = ResourceHeaders::from_stored(&stored, &state);
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    debug!(
        resource_type = %resource_type,
        id = %id,
        version = %stored.version_id(),
        created = created,
        "Resource updated"
    );

    build_update_response(
        status,
        &stored,
        headers,
        &state,
        created,
        &prefer,
        negotiated.format,
    )
}

/// Conditional update handler.
///
/// Updates a resource based on search criteria instead of ID.
///
/// # HTTP Request
///
/// `PUT [base]/[type]?[search-params]`
#[allow(clippy::too_many_arguments)]
pub async fn conditional_update_handler<S>(
    State(state): State<AppState<S>>,
    Path(resource_type): Path<String>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
    prefer: PreferHeader,
    req_headers: HeaderMap,
    FhirResource(resource): FhirResource,
) -> RestResult<Response>
where
    S: ResourceStorage + ConditionalStorage + Send + Sync,
{
    // Determine FHIR version from header or use server default
    let fhir_version = version.storage_version();

    // Negotiate response format from Accept header
    let negotiated = negotiate_format(&req_headers, None);

    // Build search params string
    let search_params: String = query
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");

    debug!(
        resource_type = %resource_type,
        search_params = %search_params,
        tenant = %tenant.tenant_id(),
        fhir_version = %fhir_version,
        "Processing conditional update request"
    );

    // Validate resourceType
    if let Some(body_type) = resource.get("resourceType").and_then(|v| v.as_str()) {
        if body_type != resource_type {
            return Err(RestError::BadRequest {
                message: format!(
                    "Resource type in body ({}) does not match URL ({})",
                    body_type, resource_type
                ),
            });
        }
    }

    let result = state
        .storage()
        .conditional_update(
            tenant.context(),
            &resource_type,
            resource,
            &search_params,
            true, // upsert
            fhir_version,
        )
        .await?;

    use helios_persistence::core::ConditionalUpdateResult;
    match result {
        ConditionalUpdateResult::Updated(stored) => {
            let headers = ResourceHeaders::from_stored(&stored, &state);
            build_update_response(
                StatusCode::OK,
                &stored,
                headers,
                &state,
                false,
                &prefer,
                negotiated.format,
            )
        }
        ConditionalUpdateResult::Created(stored) => {
            let headers = ResourceHeaders::from_stored(&stored, &state);
            build_update_response(
                StatusCode::CREATED,
                &stored,
                headers,
                &state,
                true,
                &prefer,
                negotiated.format,
            )
        }
        ConditionalUpdateResult::NoMatch => {
            // With upsert=true, this shouldn't happen, but handle it
            Err(RestError::NotFound {
                resource_type,
                id: "conditional".to_string(),
            })
        }
        ConditionalUpdateResult::MultipleMatches(count) => Err(RestError::MultipleMatches {
            operation: "update".to_string(),
            count,
        }),
    }
}

/// Builds the response for a successful update.
fn build_update_response(
    status: StatusCode,
    stored: &helios_persistence::types::StoredResource,
    headers: ResourceHeaders,
    state: &AppState<impl ResourceStorage>,
    created: bool,
    prefer: &PreferHeader,
    format: FhirFormat,
) -> RestResult<Response> {
    let mut header_map = headers.to_header_map();

    if created {
        let location = format!(
            "{}/{}/{}",
            state.base_url(),
            stored.resource_type(),
            stored.id()
        );
        header_map.insert(header::LOCATION, location.parse().unwrap());
    }

    match prefer.return_preference() {
        Some("minimal") => Ok((status, header_map).into_response()),
        Some("OperationOutcome") => {
            let action = if created { "created" } else { "updated" };
            let outcome = serde_json::json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "information",
                    "code": "informational",
                    "details": {
                        "text": format!("Resource {}: {}/{}", action, stored.resource_type(), stored.id())
                    }
                }]
            });
            format_resource_response(status, header_map, &outcome, format).map_err(|_| {
                RestError::InternalError {
                    message: "Failed to serialize response".to_string(),
                }
            })
        }
        _ => format_resource_response(status, header_map, stored.content(), format).map_err(|_| {
            RestError::InternalError {
                message: "Failed to serialize response".to_string(),
            }
        }),
    }
}
