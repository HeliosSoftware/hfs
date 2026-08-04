//! Delete interaction handler.
//!
//! Implements the FHIR [delete interaction](https://hl7.org/fhir/http.html#delete):
//! `DELETE [base]/[type]/[id]`

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_persistence::core::{ConditionalStorage, ResourceStorage};
use helios_persistence::error::{ResourceError, StorageError};
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirVersionExtractor, TenantExtractor};
use crate::middleware::ConditionalHeaders;
use crate::state::AppState;

/// Handler for the delete interaction.
///
/// Deletes a resource (soft delete - marks as deleted but preserves history).
///
/// # HTTP Request
///
/// `DELETE [base]/[type]/[id]`
///
/// # Headers
///
/// - `If-Match` — optional version precondition (RFC 9110 §13.1.1). The delete
///   is performed only if the supplied entity-tag matches the resource's current
///   version. See [`delete_handler`]'s precondition section below.
///
/// # Response
///
/// - `204 No Content` - Resource deleted successfully
/// - `200 OK` - Resource deleted, returning OperationOutcome
/// - `404 Not Found` - Resource does not exist, or is already deleted
/// - `412 Precondition Failed` - `If-Match` was supplied and is not satisfied
/// - `405 Method Not Allowed` - `AuditEvent` resources are immutable
///
/// # The `If-Match` precondition
///
/// `If-Match` is a comma-separated list and is satisfied when ANY listed tag
/// matches the current version (RFC 9110 §13.1.1). Before issue #312 this
/// handler did not read the header at all: a client asking to "delete this only
/// if it is still the version I saw" had its precondition silently discarded and
/// whatever was current was destroyed. That is the same defect class as issue
/// #270, on the one method where the consequence is not recoverable by simply
/// re-sending the request.
///
/// A malformed value is a *failed* precondition, not an absent one — degrading
/// it to "no precondition" is what turns a guarded delete into an unconditional
/// one. Parsing therefore fails closed; see
/// [`helios_persistence::core::preconditions`].
///
/// `*` asserts that a current representation exists. A soft-deleted resource has
/// none, so `*` does not match a tombstone.
///
/// Note that a conditional delete is not idempotent under retry: if the delete
/// succeeds but the response is lost, the retry sees a bumped version and
/// answers `412`. That is correct RFC 9110 behavior, not a regression.
///
/// # Deliberately NOT covered
///
/// [`conditional_delete_handler`] (`DELETE [base]/[type]?[search]`) does not
/// honor `If-Match`. That is a scope decision, not an oversight:
/// [`ConditionalStorage::conditional_delete`] searches and deletes inside the
/// backend and never surfaces a version to compare against, so honoring the
/// header there means threading a precondition through all four backends. FHIR
/// R6 does define it (`delete-conditional-single` lists `O: If-Match`); R4 and
/// R5 are silent. Tracked as a follow-up, not silently forgotten.
///
/// # Example
///
/// ```http
/// DELETE /Patient/123 HTTP/1.1
/// Host: fhir.example.com
/// If-Match: W/"3"
/// ```
pub async fn delete_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
    #[cfg_attr(not(feature = "subscriptions"), allow(unused_variables))]
    version: FhirVersionExtractor,
    conditional: ConditionalHeaders,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    // AuditEvent resources are immutable — block write operations
    if resource_type == "AuditEvent" {
        return Err(RestError::MethodNotAllowed {
            method: "DELETE".to_string(),
            resource_type: resource_type.to_string(),
        });
    }

    debug!(
        resource_type = %resource_type,
        id = %id,
        tenant = %tenant.tenant_id(),
        if_match = ?conditional.if_match_tags(),
        "Processing delete request"
    );

    // Parse the precondition before touching storage. A malformed value is
    // rejected without costing a round trip, and cannot be used as a probe.
    let if_match = conditional
        .if_match_tags()
        .map_err(|e| RestError::PreconditionFailed {
            message: format!("Malformed If-Match header: {e}"),
        })?;

    // `HFS_REQUIRE_IF_MATCH` is honored here as it is for updates. Before #312
    // it was consulted only by the update handler, so a deployment that had
    // explicitly opted into mandatory preconditions still got unconditional
    // deletes — the control silently did not apply to the destructive verb.
    //
    // 412 (not 428) is deliberate: it is what `update_handler` returns today,
    // and one status for one condition across PUT/PATCH/DELETE beats being
    // individually more correct here. Moving all three to 428 is a follow-up.
    if state.require_if_match() && !conditional.has_if_match() {
        return Err(RestError::PreconditionFailed {
            message: "If-Match header is required for deletes".to_string(),
        });
    }

    // Read the current state so the precondition has something to evaluate
    // against. This read used to be `#[cfg(feature = "subscriptions")]`-only.
    //
    // `Gone` maps to `None` — a soft-deleted resource has no current
    // representation (RFC 9110 §13.1.1), so every supplied precondition,
    // including `*`, must fail against a tombstone. Mapping it here rather than
    // propagating with `?` is load-bearing twice over: it produces that 412, and
    // it keeps a *precondition-less* delete of an already-deleted resource
    // falling through to `delete()`'s own `NotFound` -> 404, which is what stock
    // builds have always returned. Propagating `Gone` instead would have made
    // 410 the answer in every build, silently, as a side effect of this fix.
    let existing_resource = match state
        .storage()
        .read(tenant.context(), &resource_type, &id)
        .await
    {
        Ok(existing) => existing,
        Err(StorageError::Resource(ResourceError::Gone { .. })) => None,
        Err(e) => return Err(e.into()),
    };

    let current_version = existing_resource.as_ref().map(|stored| stored.version_id());

    if !if_match.if_match_satisfied(current_version) {
        // The current version is deliberately absent from the client-facing
        // message. DELETE is authorized by `FhirOperation::Delete` alone, so a
        // `system/Patient.d` principal holding no read scope reaches this path;
        // echoing the versionId back would tell it how many times the record has
        // been amended. Operators still get it from the debug log below.
        debug!(
            resource_type = %resource_type,
            id = %id,
            current_version = ?current_version,
            "If-Match precondition failed; refusing delete"
        );
        return Err(RestError::PreconditionFailed {
            message: format!(
                "If-Match precondition failed: no supplied entity-tag matches \
                 the current state of {resource_type}/{id}"
            ),
        });
    }

    #[cfg(feature = "subscriptions")]
    let fhir_version = existing_resource
        .as_ref()
        .map(|stored| stored.fhir_version())
        .unwrap_or_else(|| version.storage_version_or(state.config().default_fhir_version));

    // Perform the delete. Everything above this line is a refusal path; nothing
    // below it may run for a request that failed its precondition.
    state
        .storage()
        .delete(tenant.context(), &resource_type, &id)
        .await?;

    debug!(
        resource_type = %resource_type,
        id = %id,
        "Resource deleted"
    );

    // Compartment reference for the audit record, taken from the pre-delete
    // content while it is still in hand. `update_handler` and `patch_handler`
    // both attach this; delete never did, so delete AuditEvents carried no
    // patient reference and fell short of the IHE BALP profile. The read this
    // fix makes unconditional is what puts the resource within reach here.
    let patient_reference = existing_resource
        .as_ref()
        .and_then(|stored| super::extract_patient_from_resource(&resource_type, stored.content()));

    // Emit subscription event
    #[cfg(feature = "subscriptions")]
    if let Some(engine) = state.subscription_engine() {
        super::subscription_event::emit_delete_event(
            engine,
            tenant.context(),
            &resource_type,
            &id,
            fhir_version,
            existing_resource.map(|stored| stored.content().clone()),
        );
    }

    // Return 204 No Content (or 200 with OperationOutcome)
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .extensions_mut()
        .insert(helios_audit::AuditResponseContext {
            resource_type: Some(resource_type.clone()),
            resource_id: Some(id.clone()),
            patient_reference,
        });
    Ok(response)
}

/// Conditional delete handler.
///
/// Deletes a resource based on search criteria instead of ID.
///
/// # HTTP Request
///
/// `DELETE [base]/[type]?[search-params]`
///
/// # Response
///
/// - `204 No Content` - Resource(s) deleted
/// - `404 Not Found` - No resources matched
/// - `412 Precondition Failed` - Multiple resources matched
pub async fn conditional_delete_handler<S>(
    State(state): State<AppState<S>>,
    Path(resource_type): Path<String>,
    tenant: TenantExtractor,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> RestResult<Response>
where
    S: ResourceStorage + ConditionalStorage + Send + Sync,
{
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
        "Processing conditional delete request"
    );

    let result = state
        .storage()
        .conditional_delete(tenant.context(), &resource_type, &search_params)
        .await?;

    use helios_persistence::core::ConditionalDeleteResult;
    match result {
        ConditionalDeleteResult::Deleted => {
            debug!(
                resource_type = %resource_type,
                "Resource conditionally deleted"
            );
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        ConditionalDeleteResult::NoMatch => {
            // Per FHIR spec, no match on conditional delete is success
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        ConditionalDeleteResult::MultipleMatches(count) => Err(RestError::MultipleMatches {
            operation: "delete".to_string(),
            count,
        }),
    }
}
