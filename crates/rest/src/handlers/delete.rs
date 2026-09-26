//! Delete interaction handler.
//!
//! Implements the FHIR [delete interaction](https://hl7.org/fhir/http.html#delete):
//! `DELETE [base]/[type]/[id]`

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use helios_persistence::core::{ConditionalStorage, ResourceStorage};
use helios_persistence::error::{ResourceError, StorageError};
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirVersionExtractor, TenantExtractor};
use crate::middleware::ConditionalHeaders;
use crate::middleware::content_type::negotiate_format;
use crate::middleware::prefer::PreferHeader;
use crate::responses::format_resource_response;
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
/// - `Prefer: return=OperationOutcome` — answer a successful delete with `200`
///   and an informational OperationOutcome instead of an empty `204` (#1343).
///
/// # Response
///
/// - `204 No Content` - Resource deleted successfully
/// - `200 OK` - Resource deleted, and `Prefer: return=OperationOutcome` was sent;
///   the body is an informational OperationOutcome
/// - `404 Not Found` - Resource does not exist, or is already deleted
/// - `412 Precondition Failed` - `If-Match` was supplied and is not satisfied
/// - `409 Conflict` - `If-Match` was satisfied, but another writer changed the
///   resource before the delete landed (#1404); nothing is deleted
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
/// [`conditional_delete_handler`] (`DELETE [base]/[type]?[search]`) honours the
/// header the same way; see its documentation.
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
    version: FhirVersionExtractor,
    conditional: ConditionalHeaders,
    prefer: PreferHeader,
    req_headers: HeaderMap,
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

    let fhir_version = existing_resource
        .as_ref()
        .map(|stored| stored.fhir_version())
        .unwrap_or_else(|| version.storage_version_or(state.config().default_fhir_version));

    // Perform the delete. Everything above this line is a refusal path; nothing
    // below it may run for a request that failed its precondition.
    //
    // With `If-Match` the delete is pinned to the version the precondition was
    // just evaluated against, inside storage's own compare-and-swap. A plain
    // `delete` here was check-then-act: a writer landing after the read above
    // was deleted along with the version the client named, one it never saw
    // (#1404). Losing that race is `VersionConflict` -> 409, what `PUT` with
    // `If-Match` answers for the same race. A satisfied precondition implies a
    // current resource, so the `None` arm is the precondition-less delete of
    // something absent: storage's own `NotFound` -> 404, as before.
    match existing_resource.as_ref() {
        Some(current) => {
            helios_persistence::core::delete_under_precondition(
                state.storage(),
                tenant.context(),
                if_match,
                current,
            )
            .await?
        }
        None => {
            state
                .storage()
                .delete(tenant.context(), &resource_type, &id)
                .await?
        }
    }

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

    // `delete` only succeeds against a live resource (an already-deleted or
    // missing one is `NotFound`), so a success always removes one.
    super::write_event::report(
        &state,
        tenant.context(),
        fhir_version,
        &resource_type,
        -1,
        Some(super::write_event::delete_notice(
            &id,
            existing_resource.map(|stored| stored.content().clone()),
        )),
    );

    // 204 No Content, or 200 with an OperationOutcome when the client asked.
    let mut response = delete_response(
        &prefer,
        &req_headers,
        &format!("Resource deleted: {resource_type}/{id}"),
    );
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
/// - `204 No Content` - the single match was deleted, **or nothing matched**
/// - `200 OK` - as `204`, but `Prefer: return=OperationOutcome` was sent: the
///   body is an informational OperationOutcome saying which of the two happened
/// - `400 Bad Request` - criteria that cannot be evaluated (unknown parameter,
///   empty value, …); nothing is deleted
/// - `405 Method Not Allowed` - `AuditEvent` resources are immutable
/// - `412 Precondition Failed` - more than one resource matched
///   (`conditionalDelete` is advertised as `single`), or `If-Match` was
///   supplied and is not satisfied
///
/// # `If-Match`
///
/// Honoured (#1381; FHIR R6 lists `O: If-Match` on `delete-conditional-single`,
/// R4–R5 are silent). [`ConditionalStorage::conditional_delete`] evaluates it
/// against the one resource the criteria resolve to, immediately before the
/// delete. With no match a supplied precondition fails — `412`, not the `204`
/// below — as it does on `DELETE [type]/[id]` for a resource that does not
/// exist: no current representation satisfies `If-Match` (RFC 9110 §13.1.1).
///
/// The check and the delete are one step (#1404): with `If-Match` the backend
/// deletes through `ResourceStorage::delete_versioned`, pinned to the version
/// it evaluated, so a writer landing between the two is answered `409` instead
/// of being deleted along with the version the client named — as on
/// [`delete_handler`].
///
/// # No match
///
/// FHIR R4, R4B and R5 word it identically
/// ([conditional delete](https://hl7.org/fhir/R4/http.html#delete)): "No
/// matches or One Match: The server performs an ordinary delete on the matching
/// resource", and an ordinary delete says: "Upon successful deletion, or if the
/// resource does not exist at all, the server should return either a 200 OK if
/// the response contains a payload, or a 204 No Content with no response
/// payload". No payload is sent, so no match is `204`, not `404` (#1361) — the
/// same answer a batch `DELETE [type]?criteria` entry gives. It differs from
/// conditional *patch*, where the same text says `404`: a delete that finds
/// nothing has reached its goal, a patch has not.
///
/// The bare `204` cannot tell "deleted" from "nothing matched" — which once hid
/// criteria that silently matched nothing (#1312). A client that needs to know
/// sends `Prefer: return=OperationOutcome` and gets `200` with an informational
/// OperationOutcome naming the deleted resource, or saying nothing matched
/// (#1343). That is the "200 OK if the response contains a payload" branch of
/// the same text, so the policy holds in every version.
pub async fn conditional_delete_handler<S>(
    State(state): State<AppState<S>>,
    Path(resource_type): Path<String>,
    tenant: TenantExtractor,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    conditional: ConditionalHeaders,
    prefer: PreferHeader,
    req_headers: HeaderMap,
) -> RestResult<Response>
where
    S: ResourceStorage + ConditionalStorage + Send + Sync,
{
    super::conditional_support::require_delete(state.storage())?;

    if resource_type == "AuditEvent" {
        return Err(RestError::MethodNotAllowed {
            method: "DELETE".to_string(),
            resource_type,
        });
    }

    // The raw query, handed over as written: a `HashMap` of it keeps only the
    // last occurrence of a repeated parameter — which, on a delete, widens
    // what is deleted (#1321) — and re-joining decoded pairs corrupts a value
    // containing `&` or `=` (#1322). The shared criteria builder splits, then
    // decodes, once.
    let search_params = raw_query.unwrap_or_default();

    debug!(
        resource_type = %resource_type,
        search_params = %search_params,
        tenant = %tenant.tenant_id(),
        "Processing conditional delete request"
    );

    let if_match = super::update::conditional_if_match(&conditional)?;

    let result = state
        .storage()
        .conditional_delete(tenant.context(), &resource_type, &search_params, if_match)
        .await
        .map_err(|e| super::update::conditional_write_error(e, &resource_type))?;

    use helios_persistence::core::ConditionalDeleteResult;
    match result {
        ConditionalDeleteResult::Deleted(deleted) => {
            // Counted, but conditional writes announce nothing.
            super::write_event::report(
                &state,
                tenant.context(),
                deleted.fhir_version(),
                &resource_type,
                -1,
                None,
            );
            debug!(
                resource_type = %resource_type,
                id = %deleted.id(),
                "Resource conditionally deleted"
            );
            // Name the entity in the audit trail, as the instance delete does.
            // Before the result carried the snapshot, a delete-by-criteria
            // produced an AuditEvent with no entity at all.
            let mut response = delete_response(
                &prefer,
                &req_headers,
                &format!("Resource deleted: {resource_type}/{}", deleted.id()),
            );
            response
                .extensions_mut()
                .insert(helios_audit::AuditResponseContext {
                    resource_type: Some(resource_type.clone()),
                    resource_id: Some(deleted.id().to_string()),
                    patient_reference: super::extract_patient_from_resource(
                        &resource_type,
                        deleted.content(),
                    ),
                });
            Ok(response)
        }
        ConditionalDeleteResult::NoMatch => {
            // "No matches or One Match: The server performs an ordinary
            // delete", which answers 204 "if the resource does not exist at
            // all" — see the handler doc.
            Ok(delete_response(
                &prefer,
                &req_headers,
                &format!("No {resource_type} matched the search criteria; nothing was deleted"),
            ))
        }
        ConditionalDeleteResult::MultipleMatches(count) => Err(RestError::MultipleMatches {
            operation: "delete".to_string(),
            count,
        }),
    }
}

/// The success response of a delete: an empty `204`, or — when the client sent
/// `Prefer: return=OperationOutcome` — `200` with an informational
/// OperationOutcome carrying `message`, in the negotiated format. The same
/// shape create, update and patch answer that preference with.
///
/// A format the server cannot produce (XML without the `xml` feature) is
/// answered with the formatter's own refusal, `406`, not a generic `500`.
fn delete_response(prefer: &PreferHeader, req_headers: &HeaderMap, message: &str) -> Response {
    if !prefer.is_operation_outcome() {
        return StatusCode::NO_CONTENT.into_response();
    }
    let outcome = serde_json::json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "information",
            "code": "informational",
            "details": { "text": message }
        }]
    });
    let format = negotiate_format(req_headers, None).format;
    format_resource_response(StatusCode::OK, HeaderMap::new(), &outcome, format)
        .unwrap_or_else(|refusal| refusal)
}
