//! Patch interaction handler.
//!
//! Implements the FHIR [patch interaction](https://hl7.org/fhir/http.html#patch):
//! `PATCH [base]/[type]/[id]`
//!
//! Supports multiple patch formats:
//! - JSON Patch (RFC 6902) - application/json-patch+json
//! - JSON Merge Patch (RFC 7386) - application/merge-patch+json
//! - FHIRPath Patch - application/fhir+json with Parameters resource

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use helios_persistence::core::{ConditionalStorage, PatchFormat, ResourceStorage};
use serde_json::Value;
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::extractors::TenantExtractor;
use crate::middleware::conditional::ConditionalHeaders;
use crate::middleware::prefer::PreferHeader;
use crate::responses::headers::ResourceHeaders;
use crate::state::AppState;

/// Handler for the patch interaction.
///
/// Applies a partial update to a resource.
///
/// # HTTP Request
///
/// `PATCH [base]/[type]/[id]`
///
/// # Headers
///
/// - `Content-Type` - Patch format:
///   - `application/json-patch+json` - JSON Patch (RFC 6902)
///   - `application/merge-patch+json` - JSON Merge Patch (RFC 7386)
///   - `application/fhir+json` - FHIRPath Patch (Parameters resource)
/// - `If-Match` - Optimistic locking (ETag)
///
/// # Response
///
/// - `200 OK` - Resource patched successfully
/// - `400 Bad Request` - Invalid patch document
/// - `404 Not Found` - Resource does not exist
/// - `412 Precondition Failed` - If-Match condition not met
/// - `415 Unsupported Media Type` - Unknown patch format
pub async fn patch_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    headers: HeaderMap,
    tenant: TenantExtractor,
    conditional: ConditionalHeaders,
    prefer: PreferHeader,
    body: Bytes,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    // AuditEvent resources are immutable — block write operations
    if resource_type == "AuditEvent" {
        return Err(RestError::MethodNotAllowed {
            method: "PATCH".to_string(),
            resource_type: resource_type.to_string(),
        });
    }

    debug!(
        resource_type = %resource_type,
        id = %id,
        tenant = %tenant.tenant_id(),
        "Processing patch request"
    );

    // Determine patch format from Content-Type
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json-patch+json");

    let patch_format = parse_patch_format(content_type, &body)?;

    // Read existing resource
    let existing = state
        .storage()
        .read(tenant.context(), &resource_type, &id)
        .await?
        .ok_or_else(|| RestError::NotFound {
            resource_type: resource_type.clone(),
            id: id.clone(),
        })?;

    // Check If-Match precondition
    if let Some(if_match) = conditional.if_match() {
        let current_etag = format!("W/\"{}\"", existing.version_id());
        if if_match != current_etag && if_match != "*" {
            return Err(RestError::PreconditionFailed {
                message: format!("ETag mismatch: expected {}, got {}", if_match, current_etag),
            });
        }
    }

    // Apply the patch
    let patched_content = apply_patch(existing.content(), &patch_format)?;

    // Validate that resourceType wasn't changed
    if let Some(body_type) = patched_content.get("resourceType").and_then(|v| v.as_str()) {
        if body_type != resource_type {
            return Err(RestError::BadRequest {
                message: "Cannot change resourceType via patch".to_string(),
            });
        }
    }

    // Update the resource
    let stored = state
        .storage()
        .update(tenant.context(), &existing, patched_content)
        .await?;

    let headers = ResourceHeaders::from_stored(&stored, &state);

    debug!(
        resource_type = %resource_type,
        id = %id,
        version = %stored.version_id(),
        "Resource patched"
    );

    build_patch_response(&stored, headers, &prefer)
}

/// Conditional patch handler.
///
/// Patches a resource based on search criteria.
///
/// # HTTP Request
///
/// `PATCH [base]/[type]?[search-params]`
pub async fn conditional_patch_handler<S>(
    State(state): State<AppState<S>>,
    Path(resource_type): Path<String>,
    headers: HeaderMap,
    tenant: TenantExtractor,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
    prefer: PreferHeader,
    body: Bytes,
) -> RestResult<Response>
where
    S: ResourceStorage + ConditionalStorage + Send + Sync,
{
    let search_params: String = query
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");

    debug!(
        resource_type = %resource_type,
        search_params = %search_params,
        tenant = %tenant.tenant_id(),
        "Processing conditional patch request"
    );

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json-patch+json");

    let patch_format = parse_patch_format(content_type, &body)?;

    let result = state
        .storage()
        .conditional_patch(
            tenant.context(),
            &resource_type,
            &search_params,
            &patch_format,
        )
        .await?;

    use helios_persistence::core::ConditionalPatchResult;
    match result {
        ConditionalPatchResult::Patched(stored) => {
            let headers = ResourceHeaders::from_stored(&stored, &state);
            build_patch_response(&stored, headers, &prefer)
        }
        ConditionalPatchResult::NoMatch => Err(RestError::NotFound {
            resource_type,
            id: "conditional".to_string(),
        }),
        ConditionalPatchResult::MultipleMatches(count) => Err(RestError::MultipleMatches {
            operation: "patch".to_string(),
            count,
        }),
    }
}

/// Parses the patch format from Content-Type and body.
fn parse_patch_format(content_type: &str, body: &Bytes) -> RestResult<PatchFormat> {
    let patch_value: Value = serde_json::from_slice(body).map_err(|e| RestError::BadRequest {
        message: format!("Invalid JSON in patch body: {}", e),
    })?;

    if content_type.contains("json-patch+json") {
        Ok(PatchFormat::JsonPatch(patch_value))
    } else if content_type.contains("merge-patch+json") {
        Ok(PatchFormat::MergePatch(patch_value))
    } else if content_type.contains("fhir+json") {
        // FHIRPath Patch uses a Parameters resource
        if patch_value.get("resourceType") == Some(&Value::String("Parameters".to_string())) {
            Ok(PatchFormat::FhirPathPatch(patch_value))
        } else {
            Err(RestError::BadRequest {
                message: "FHIRPath patch must be a Parameters resource".to_string(),
            })
        }
    } else {
        Err(RestError::UnsupportedMediaType {
            content_type: content_type.to_string(),
        })
    }
}

/// Applies a patch to a resource.
fn apply_patch(resource: &Value, patch: &PatchFormat) -> RestResult<Value> {
    match patch {
        PatchFormat::JsonPatch(operations) => {
            let patch: json_patch::Patch =
                serde_json::from_value(operations.clone()).map_err(|e| RestError::BadRequest {
                    message: format!("Invalid JSON Patch: {}", e),
                })?;

            let mut resource = resource.clone();
            json_patch::patch(&mut resource, &patch).map_err(|e| RestError::BadRequest {
                message: format!("Failed to apply JSON Patch: {}", e),
            })?;

            Ok(resource)
        }
        PatchFormat::MergePatch(merge_doc) => {
            let mut resource = resource.clone();
            json_patch::merge(&mut resource, merge_doc);
            Ok(resource)
        }
        PatchFormat::FhirPathPatch(_params) => {
            // FHIRPath Patch is more complex and requires FHIRPath evaluation
            Err(RestError::NotImplemented {
                feature: "FHIRPath Patch".to_string(),
            })
        }
    }
}

/// Builds the response for a successful patch.
fn build_patch_response(
    stored: &helios_persistence::types::StoredResource,
    headers: ResourceHeaders,
    prefer: &PreferHeader,
) -> RestResult<Response> {
    let header_map = headers.to_header_map();

    match prefer.return_preference() {
        Some("minimal") => Ok((StatusCode::OK, header_map).into_response()),
        Some("OperationOutcome") => {
            let outcome = serde_json::json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "information",
                    "code": "informational",
                    "details": {
                        "text": format!("Resource patched: {}/{}", stored.resource_type(), stored.id())
                    }
                }]
            });
            Ok((StatusCode::OK, header_map, Json(outcome)).into_response())
        }
        _ => Ok((StatusCode::OK, header_map, Json(stored.content().clone())).into_response()),
    }
}
