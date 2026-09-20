//! Patch interaction handler.
//!
//! Implements the FHIR [patch interaction](https://hl7.org/fhir/http.html#patch):
//! `PATCH [base]/[type]/[id]`, and its conditional form
//! `PATCH [base]/[type]?[search-params]`.
//!
//! Supports multiple patch formats:
//! - JSON Patch (RFC 6902) - application/json-patch+json
//! - JSON Merge Patch (RFC 7386) - application/merge-patch+json
//! - FHIRPath Patch - application/fhir+json with Parameters resource (recognised,
//!   answered `501 Not Implemented`)

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
use crate::handlers::extract_patient_from_resource;
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

    // Check the If-Match precondition (RFC 9110 §13.1.1).
    //
    // List-aware: satisfied when ANY supplied entity-tag matches. A malformed
    // value fails the precondition rather than being ignored. The resource is
    // known to exist here (the read above 404s otherwise), so `*` always
    // matches. See `helios_persistence::core::preconditions`.
    let if_match = conditional
        .if_match_tags()
        .map_err(|e| RestError::PreconditionFailed {
            message: format!("Malformed If-Match header: {e}"),
        })?;

    if !if_match.if_match_satisfied(Some(existing.version_id())) {
        return Err(RestError::PreconditionFailed {
            message: format!(
                "If-Match precondition failed: no supplied entity-tag matches the current version W/\"{}\"",
                existing.version_id()
            ),
        });
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

    super::write_event::report(
        &state,
        tenant.context(),
        stored.fhir_version(),
        &resource_type,
        0,
        Some(super::write_event::stored_notice(
            helios_persistence::core::WriteKind::Update,
            &stored,
        )),
    );

    build_patch_response(&stored, headers, &prefer).map(|mut response| {
        response
            .extensions_mut()
            .insert(helios_audit::AuditResponseContext {
                resource_type: Some(resource_type.clone()),
                resource_id: Some(stored.id().to_string()),
                patient_reference: extract_patient_from_resource(&resource_type, stored.content()),
            });
        response
    })
}

/// Conditional patch handler.
///
/// Patches the one resource a search selects, instead of one named by id.
///
/// # HTTP Request
///
/// `PATCH [base]/[type]?[search-params]`
///
/// The criteria go through the pipeline every conditional interaction shares
/// (`helios_persistence::search::conditional`): the raw query, decoded once,
/// unknown parameters and empty values refused.
///
/// # Response
///
/// FHIR R4, R4B and R5 word the three outcomes identically
/// ([conditional patch](https://hl7.org/fhir/R4/http.html#patch)): "No matches:
/// The server returns a 404 Not Found"; "One Match: The server performs the
/// update against the matching resource"; "Multiple matches: The server returns
/// a 412 Precondition Failed error".
///
/// - `200 OK` - the single match was patched
/// - `400 Bad Request` - no criteria, criteria that cannot be evaluated, an
///   invalid patch document, or an `If-Match` header (see below)
/// - `404 Not Found` - nothing matched; nothing is created
/// - `405 Method Not Allowed` - `AuditEvent` resources are immutable
/// - `412 Precondition Failed` - more than one resource matched
/// - `415 Unsupported Media Type` - unknown patch format
/// - `501 Not Implemented` - FHIRPath Patch, as for [`patch_handler`]; or a
///   backend without conditional patch (MongoDB)
///
/// # `If-Match`
///
/// [`ConditionalStorage::conditional_patch`] searches and writes inside the
/// backend and never surfaces the version it is about to replace, so the header
/// cannot be honoured here (the same limit [`conditional_delete_handler`]
/// documents). A version precondition that is silently discarded is worse than
/// one that is refused, so the request is refused; a client that wants both
/// resolves the id first and sends `PATCH [type]/[id]` with `If-Match`.
///
/// [`conditional_delete_handler`]: super::delete::conditional_delete_handler
#[allow(clippy::too_many_arguments)]
pub async fn conditional_patch_handler<S>(
    State(state): State<AppState<S>>,
    Path(resource_type): Path<String>,
    headers: HeaderMap,
    tenant: TenantExtractor,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    conditional: ConditionalHeaders,
    prefer: PreferHeader,
    body: Bytes,
) -> RestResult<Response>
where
    S: ResourceStorage + ConditionalStorage + Send + Sync,
{
    // AuditEvent resources are immutable — block write operations
    if resource_type == "AuditEvent" {
        return Err(RestError::MethodNotAllowed {
            method: "PATCH".to_string(),
            resource_type: resource_type.to_string(),
        });
    }

    // The raw query, as written: every occurrence of a repeated parameter
    // (#1321), decoded once by the shared criteria builder (#1322).
    let search_params = raw_query.unwrap_or_default();

    debug!(
        resource_type = %resource_type,
        search_params = %search_params,
        tenant = %tenant.tenant_id(),
        "Processing conditional patch request"
    );

    // `PATCH /Patient` names neither an instance nor criteria. The backend
    // would answer "no match", and a 404 for a missing id or query string
    // sends the client looking for a resource that was never named.
    if helios_persistence::search::parse_conditional_criteria(&search_params).is_empty() {
        return Err(RestError::BadRequest {
            message: format!(
                "PATCH {resource_type} names no resource: use PATCH {resource_type}/[id], or \
                 PATCH {resource_type}?[search parameters] for a conditional patch"
            ),
        });
    }

    if conditional.has_if_match() {
        return Err(RestError::BadRequest {
            message: format!(
                "If-Match is not supported on a conditional patch (PATCH {resource_type}?…): \
                 the version precondition cannot be checked against a resource selected by \
                 criteria. Nothing was written; resolve the id and send PATCH \
                 {resource_type}/[id] with If-Match instead"
            ),
        });
    }

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json-patch+json");

    let patch_format = parse_patch_format(content_type, &body)?;

    // Hold the patch to what `patch_handler` accepts *before* the backend sees
    // it: the backend applies the document itself, and its FHIRPath Patch is a
    // stub that ignores every path but `Type.element` and still writes a new
    // version.
    check_conditional_patch(&resource_type, &patch_format)?;

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
            // Conditional writes announce nothing.
            super::write_event::report(
                &state,
                tenant.context(),
                stored.fhir_version(),
                &resource_type,
                0,
                None,
            );
            let headers = ResourceHeaders::from_stored(&stored, &state);
            build_patch_response(&stored, headers, &prefer).map(|mut response| {
                response
                    .extensions_mut()
                    .insert(helios_audit::AuditResponseContext {
                        resource_type: Some(resource_type.clone()),
                        resource_id: Some(stored.id().to_string()),
                        patient_reference: extract_patient_from_resource(
                            &resource_type,
                            stored.content(),
                        ),
                    });
                response
            })
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

/// The refusals [`patch_handler`] makes while or after applying a patch, made
/// up front for a conditional patch, where the backend does the applying.
///
/// * FHIRPath Patch is not implemented (`501`), exactly as on the instance
///   endpoint.
/// * `resourceType` cannot be patched (`400`). Backends re-assert the stored
///   type and id on every update, so a patch naming them could not corrupt the
///   row — it would be silently undone, and answered with a `200`.
fn check_conditional_patch(resource_type: &str, patch: &PatchFormat) -> RestResult<()> {
    let changes_type = match patch {
        PatchFormat::FhirPathPatch(_) => {
            return Err(RestError::NotImplemented {
                feature: "FHIRPath Patch".to_string(),
            });
        }
        PatchFormat::JsonPatch(operations) => operations.as_array().is_some_and(|ops| {
            ops.iter().any(|op| {
                // `test` and the source of a `copy` only read the element.
                let writes =
                    |key: &str| op.get(key).and_then(Value::as_str) == Some("/resourceType");
                match op.get("op").and_then(Value::as_str) {
                    Some("test") => false,
                    Some("move") => writes("path") || writes("from"),
                    _ => writes("path"),
                }
            })
        }),
        PatchFormat::MergePatch(merge_doc) => merge_doc
            .get("resourceType")
            .is_some_and(|t| t.as_str() != Some(resource_type)),
    };
    if changes_type {
        return Err(RestError::BadRequest {
            message: "Cannot change resourceType via patch".to_string(),
        });
    }
    Ok(())
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
        _ => Ok((StatusCode::OK, header_map, Json(stored.content_with_meta())).into_response()),
    }
}
