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
use helios_persistence::error::{ResourceError, StorageError};
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirResource, FhirVersionExtractor, TenantExtractor};
use crate::fhir_types::admit_resource_type;
use crate::handlers::extract_patient_from_resource;
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
    // Determine FHIR version from header or use server default
    let fhir_version = version.storage_version_or(state.config().default_fhir_version);

    admit_resource_type(&resource_type, &resource, fhir_version).map_err(|error| {
        RestError::BadRequest {
            message: error.to_string(),
        }
    })?;

    // AuditEvent resources are immutable — block write operations
    if resource_type == "AuditEvent" {
        return Err(RestError::MethodNotAllowed {
            method: "PUT".to_string(),
            resource_type: resource_type.to_string(),
        });
    }

    // Negotiate response format from Accept header
    let negotiated = negotiate_format(&req_headers, None);

    debug!(
        resource_type = %resource_type,
        id = %id,
        tenant = %tenant.tenant_id(),
        fhir_version = %fhir_version,
        if_match = ?conditional.if_match_tags(),
        "Processing update request"
    );

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
    if state.require_if_match() && !conditional.has_if_match() {
        return Err(RestError::PreconditionFailed {
            message: "If-Match header is required for updates".to_string(),
        });
    }

    // Write-path validation (HFS_VALIDATION_MODE: off | log | enforce).
    state
        .validation()
        .check_write(tenant.tenant_id(), fhir_version, &resource_type, &resource)
        .await?;

    // Handle the If-Match precondition (RFC 9110 §13.1.1).
    //
    // `If-Match` is a comma-separated list and is satisfied when ANY listed tag
    // matches; comparing the field value as one string made every multi-valued
    // header a permanent 412 (issue #311). A malformed value is a *failed*
    // precondition, not an absent one — degrading it to "no precondition" would
    // turn a guarded update into an unconditional overwrite.
    //
    // `*` asserts that a current representation exists, so it does NOT license
    // an update-as-create: with no existing resource (or a deleted one, which
    // has no current representation) it fails like any other tag.
    //
    // Parsed before storage is touched, as `delete_handler` already does: a
    // malformed value is rejected without costing a round trip and cannot be
    // used as a probe.
    let if_match = conditional
        .if_match_tags()
        .map_err(|e| RestError::PreconditionFailed {
            message: format!("Malformed If-Match header: {e}"),
        })?;

    // Perform the update (or create).
    //
    // This handler used to read the resource unconditionally and then call
    // `create_or_update`, which reads it *again* to decide between update,
    // create and restore. Two reads of the same row, one per layer, on every
    // `PUT`. `patch_handler` has always done it the other way round — read
    // once, hand the row to `update` — and this brings `PUT` into line.
    //
    // The two branches below are not an optimisation of one path; they are two
    // genuinely different requests:
    //
    // * **No `If-Match`.** `EntityTagPrecondition::Absent` is satisfied by
    //   *everything* (`preconditions.rs`), so the read this handler did could
    //   not change the outcome by construction: `current_version` was computed
    //   and then compared against a precondition that returns `true` without
    //   looking at it. Nothing else in this function consulted `existing`. So
    //   the read is dropped outright and `create_or_update` does the single
    //   read it was always going to do. Last-writer-wins is preserved exactly:
    //   the decision is still made from the row `create_or_update` reads, at
    //   the same instant it read it before.
    //
    // * **`If-Match` present.** The read has to happen here, because the
    //   precondition is evaluated here. Handing the row it produced to `update`
    //   — rather than throwing it away and letting `create_or_update` fetch its
    //   own — removes the second read *and closes a lost update*:
    //
    //     handler reads v1 → `If-Match: W/"1"` is satisfied → a concurrent
    //     writer commits v2 → `create_or_update` reads v2 → `update` compares
    //     and swaps against v2, succeeds, and returns 200.
    //
    //   The client asked to write only if the resource was still v1; it was
    //   not, and its write silently overwrote v2 anyway. Passing the row the
    //   precondition was actually evaluated against makes the compare-and-swap
    //   in `update` test *that* version, so the race now ends in
    //   `VersionConflict` → 409 instead of a lost update. The window does not
    //   move; it is the same read-to-write gap, minus one round trip inside it.
    //
    //   Two consequences of that, stated rather than hidden. A racing writer
    //   that lands between the read and the write now yields 409 where the old
    //   code returned 200 — that is the bug being fixed, not a regression. And
    //   if the row is *soft-deleted* in that same window, the answer is 404
    //   (`update`'s CAS matches no live row) rather than the old 201 from
    //   `create_or_update`'s restore path: a client that conditioned its write
    //   on a specific live version should not silently resurrect a tombstone.
    //
    //   A satisfied *present* precondition implies a current representation
    //   — `Any` requires `current_version.is_some()` and `Tags` cannot match
    //   `None` — so `existing` is `Some` on that path. The `None` arm is
    //   unreachable and defers to the generic path rather than panicking.
    //
    // `Gone` is mapped to `None` exactly as before: a deleted resource is
    // brought back to life by a subsequent update
    // (https://hl7.org/fhir/http.html#delete), so it is not an error — it means
    // there is no current version to match `If-Match` against, and (on the
    // no-precondition path) the storage layer restores the resource on write.
    let (stored, created) = if if_match.is_present() {
        let existing = match state
            .storage()
            .read(tenant.context(), &resource_type, &id)
            .await
        {
            Ok(existing) => existing,
            Err(StorageError::Resource(ResourceError::Gone { .. })) => None,
            Err(e) => return Err(e.into()),
        };

        let current_version = existing.as_ref().map(|stored| stored.version_id());
        if !if_match.if_match_satisfied(current_version) {
            let message = match current_version {
                Some(current) => format!(
                    "If-Match precondition failed: no supplied entity-tag matches the current version W/\"{current}\""
                ),
                None => {
                    "If-Match precondition failed: the resource has no current version to match"
                        .to_string()
                }
            };
            return Err(RestError::PreconditionFailed { message });
        }

        match existing {
            Some(current) => (
                state
                    .storage()
                    .update(tenant.context(), &current, resource)
                    .await?,
                false,
            ),
            None => {
                state
                    .storage()
                    .create_or_update(
                        tenant.context(),
                        &resource_type,
                        &id,
                        resource,
                        fhir_version,
                    )
                    .await?
            }
        }
    } else {
        state
            .storage()
            .create_or_update(
                tenant.context(),
                &resource_type,
                &id,
                resource,
                fhir_version,
            )
            .await?
    };

    // Stored StructureDefinitions feed the tenant's profile registry.
    if resource_type == "StructureDefinition" {
        state.validation().upsert_stored_profile(
            tenant.tenant_id(),
            fhir_version,
            stored.content(),
        );
    }

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

    // Emit subscription event
    #[cfg(feature = "subscriptions")]
    if let Some(engine) = state.subscription_engine() {
        let event_type = if created {
            helios_subscriptions::ResourceEventType::Create
        } else {
            helios_subscriptions::ResourceEventType::Update
        };
        super::subscription_event::emit_subscription_event(
            engine,
            tenant.context(),
            &stored,
            fhir_version,
            event_type,
        );
    }

    let location = created
        .then(|| state.public_url_for_request(&tenant, [stored.resource_type(), stored.id()]));
    build_update_response(
        status,
        &stored,
        headers,
        location.as_deref(),
        created,
        &prefer,
        negotiated.format,
    )
    .map(|mut response| {
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
    let fhir_version = version.storage_version_or(state.config().default_fhir_version);

    admit_resource_type(&resource_type, &resource, fhir_version).map_err(|error| {
        RestError::BadRequest {
            message: error.to_string(),
        }
    })?;

    if resource_type == "AuditEvent" {
        return Err(RestError::MethodNotAllowed {
            method: "PUT".to_string(),
            resource_type: resource_type.to_string(),
        });
    }

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

    // Write-path validation (HFS_VALIDATION_MODE: off | log | enforce).
    state
        .validation()
        .check_write(tenant.tenant_id(), fhir_version, &resource_type, &resource)
        .await?;

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
                None,
                false,
                &prefer,
                negotiated.format,
            )
            .map(|mut response| {
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
        ConditionalUpdateResult::Created(stored) => {
            let headers = ResourceHeaders::from_stored(&stored, &state);
            let location =
                state.public_url_for_request(&tenant, [stored.resource_type(), stored.id()]);
            build_update_response(
                StatusCode::CREATED,
                &stored,
                headers,
                Some(&location),
                true,
                &prefer,
                negotiated.format,
            )
            .map(|mut response| {
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
    location: Option<&str>,
    created: bool,
    prefer: &PreferHeader,
    format: FhirFormat,
) -> RestResult<Response> {
    let mut header_map = headers.to_header_map();

    if let Some(location) = location {
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
