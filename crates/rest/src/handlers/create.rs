//! Create interaction handler.
//!
//! Implements the FHIR [create interaction](https://hl7.org/fhir/http.html#create):
//! `POST [base]/[type]`

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use helios_persistence::core::{ConditionalStorage, ResourceStorage};
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirResource, FhirVersionExtractor, TenantExtractor};
use crate::handlers::extract_patient_from_resource;
use crate::middleware::conditional::ConditionalHeaders;
use crate::middleware::content_type::{FhirFormat, negotiate_format};
use crate::middleware::prefer::PreferHeader;
use crate::responses::format_resource_response;
use crate::responses::headers::ResourceHeaders;
use crate::state::AppState;

/// Handler for the create interaction.
///
/// Creates a new resource. The server assigns the resource ID.
///
/// # HTTP Request
///
/// `POST [base]/[type]`
///
/// # Headers
///
/// - `Content-Type` - Must be application/fhir+json or application/fhir+xml
/// - `If-None-Exist` - Conditional create search parameters
/// - `Prefer` - Response preference (return=minimal, return=representation, return=OperationOutcome)
///
/// # Response
///
/// - `201 Created` - Resource created successfully
/// - `200 OK` - Conditional create matched existing resource
/// - `400 Bad Request` - Invalid resource
/// - `412 Precondition Failed` - Conditional create matched multiple resources
///
/// # Example
///
/// ```http
/// POST /Patient HTTP/1.1
/// Host: fhir.example.com
/// Content-Type: application/fhir+json
/// Prefer: return=representation
///
/// {"resourceType": "Patient", "name": [{"family": "Smith"}]}
/// ```
#[allow(clippy::too_many_arguments)]
pub async fn create_handler<S>(
    State(state): State<AppState<S>>,
    Path(resource_type): Path<String>,
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
            method: "POST".to_string(),
            resource_type: resource_type.to_string(),
        });
    }

    // Determine FHIR version from header or use server default
    let fhir_version = version.storage_version();

    // Negotiate response format from Accept header
    let negotiated = negotiate_format(&req_headers, None);

    debug!(
        resource_type = %resource_type,
        tenant = %tenant.tenant_id(),
        fhir_version = %fhir_version,
        conditional = ?conditional.if_none_exist(),
        "Processing create request"
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

    // Check for conditional create
    if let Some(search_params) = conditional.if_none_exist() {
        debug!(search_params = %search_params, "Processing conditional create");

        let result = state
            .storage()
            .conditional_create(
                tenant.context(),
                &resource_type,
                resource,
                search_params,
                fhir_version,
            )
            .await?;

        use helios_persistence::core::ConditionalCreateResult;
        return match result {
            ConditionalCreateResult::Created(stored) => {
                let headers = ResourceHeaders::from_stored(&stored, &state);
                let location = format!("{}/{}/{}", state.base_url(), resource_type, stored.id());

                debug!(
                    resource_type = %resource_type,
                    id = %stored.id(),
                    "Resource created (conditional)"
                );

                build_create_response(
                    StatusCode::CREATED,
                    &stored,
                    headers,
                    &location,
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
            ConditionalCreateResult::Exists(stored) => {
                let headers = ResourceHeaders::from_stored(&stored, &state);

                debug!(
                    resource_type = %resource_type,
                    id = %stored.id(),
                    "Existing resource matched conditional create"
                );

                // Return 200 OK with the existing resource
                build_existing_response(&stored, headers, &prefer, negotiated.format).map(
                    |mut response| {
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
                    },
                )
            }
            ConditionalCreateResult::MultipleMatches(count) => Err(RestError::MultipleMatches {
                operation: "create".to_string(),
                count,
            }),
        };
    }

    // Standard create
    let stored = state
        .storage()
        .create(tenant.context(), &resource_type, resource, fhir_version)
        .await?;

    let headers = ResourceHeaders::from_stored(&stored, &state);
    let location = format!("{}/{}/{}", state.base_url(), resource_type, stored.id());

    debug!(
        resource_type = %resource_type,
        id = %stored.id(),
        "Resource created"
    );

    // Emit subscription event
    #[cfg(feature = "subscriptions")]
    if let Some(engine) = state.subscription_engine() {
        super::subscription_event::emit_subscription_event(
            engine,
            tenant.context(),
            &stored,
            fhir_version,
            helios_subscriptions::ResourceEventType::Create,
        );
    }

    build_create_response(
        StatusCode::CREATED,
        &stored,
        headers,
        &location,
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

/// Builds the response for a successful create.
fn build_create_response(
    status: StatusCode,
    stored: &helios_persistence::types::StoredResource,
    headers: ResourceHeaders,
    location: &str,
    prefer: &PreferHeader,
    format: FhirFormat,
) -> RestResult<Response> {
    let mut header_map = headers.to_header_map();
    header_map.insert(header::LOCATION, location.parse().unwrap());

    match prefer.return_preference() {
        Some("minimal") => Ok((status, header_map).into_response()),
        Some("OperationOutcome") => {
            let outcome = serde_json::json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "information",
                    "code": "informational",
                    "details": {
                        "text": format!("Resource created: {}", location)
                    }
                }]
            });
            format_resource_response(status, header_map, &outcome, format).map_err(|_| {
                RestError::InternalError {
                    message: "Failed to serialize response".to_string(),
                }
            })
        }
        _ => {
            // Default: return=representation
            format_resource_response(status, header_map, stored.content(), format).map_err(|_| {
                RestError::InternalError {
                    message: "Failed to serialize response".to_string(),
                }
            })
        }
    }
}

/// Builds the response for an existing resource (conditional create match).
fn build_existing_response(
    stored: &helios_persistence::types::StoredResource,
    headers: ResourceHeaders,
    prefer: &PreferHeader,
    format: FhirFormat,
) -> RestResult<Response> {
    let header_map = headers.to_header_map();

    match prefer.return_preference() {
        Some("minimal") => Ok((StatusCode::OK, header_map).into_response()),
        _ => format_resource_response(StatusCode::OK, header_map, stored.content(), format)
            .map_err(|_| RestError::InternalError {
                message: "Failed to serialize response".to_string(),
            }),
    }
}
