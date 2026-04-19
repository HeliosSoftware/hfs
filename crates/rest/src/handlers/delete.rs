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
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::extractors::TenantExtractor;
use crate::state::AppState;

/// Handler for the delete interaction.
///
/// Deletes a resource (soft delete - marks as deleted but preserves history).
///
/// # HTTP Request
///
/// `DELETE [base]/[type]/[id]`
///
/// # Response
///
/// - `204 No Content` - Resource deleted successfully
/// - `200 OK` - Resource deleted, returning OperationOutcome
/// - `404 Not Found` - Resource does not exist
///
/// # Example
///
/// ```http
/// DELETE /Patient/123 HTTP/1.1
/// Host: fhir.example.com
/// ```
pub async fn delete_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
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
        "Processing delete request"
    );

    // Perform the delete
    state
        .storage()
        .delete(tenant.context(), &resource_type, &id)
        .await?;

    debug!(
        resource_type = %resource_type,
        id = %id,
        "Resource deleted"
    );

    // Emit subscription event
    #[cfg(feature = "subscriptions")]
    if let Some(engine) = state.subscription_engine() {
        super::subscription_event::emit_delete_event(
            engine,
            tenant.context(),
            &resource_type,
            &id,
            helios_fhir::FhirVersion::default(),
        );
    }

    // Return 204 No Content (or 200 with OperationOutcome)
    Ok(StatusCode::NO_CONTENT.into_response())
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
