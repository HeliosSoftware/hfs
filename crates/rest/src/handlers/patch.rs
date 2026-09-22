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

    // Apply the patch: the applier `PATCH [type]?criteria` uses inside the
    // storage layer (#1406). It refuses a patch that changes `resourceType` or
    // `id`, and FHIRPath Patch (`501`).
    let patched_content = helios_persistence::core::apply_patch(existing.content(), &patch_format)?;

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
/// - `400 Bad Request` - no criteria, criteria that cannot be evaluated, or an
///   invalid patch document
/// - `404 Not Found` - nothing matched; nothing is created
/// - `405 Method Not Allowed` - `AuditEvent` resources are immutable
/// - `412 Precondition Failed` - more than one resource matched, or `If-Match`
///   was supplied and is not satisfied
/// - `415 Unsupported Media Type` - unknown patch format
/// - `501 Not Implemented` - FHIRPath Patch, as for [`patch_handler`]; or a
///   storage without conditional patch (S3 on its own)
///
/// # `If-Match`
///
/// Honoured, as on conditional update and delete (#1381; it was refused with
/// `400` before). [`ConditionalStorage::conditional_patch`] evaluates it
/// against the one resource the criteria resolve to and hands that same row to
/// the compare-and-swap that writes the patched content, so a writer landing in
/// between ends in `409`, never in a patch over a version the client did not
/// name. A malformed value fails the precondition. With no match the answer
/// stays `404` — what `PATCH [type]/[id]` answers for a missing resource,
/// `If-Match` or not — and nothing is written.
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
    super::conditional_support::require_patch(state.storage())?;

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

    let if_match = super::update::conditional_if_match(&conditional)?;

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json-patch+json");

    let patch_format = parse_patch_format(content_type, &body)?;

    // FHIRPath Patch is not implemented. The storage layer's applier says so
    // too, but only once the criteria have resolved to one resource; refused
    // here, the answer does not depend on what the criteria match.
    if matches!(patch_format, PatchFormat::FhirPathPatch(_)) {
        return Err(RestError::NotImplemented {
            feature: "FHIRPath Patch".to_string(),
        });
    }

    let result = state
        .storage()
        .conditional_patch(
            tenant.context(),
            &resource_type,
            &search_params,
            &patch_format,
            if_match,
        )
        .await
        .map_err(|e| super::update::conditional_write_error(e, &resource_type))?;

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
