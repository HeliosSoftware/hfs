//! Create interaction handler.
//!
//! Implements the FHIR [create interaction](https://hl7.org/fhir/http.html#create):
//! `POST [base]/[type]`

use axum::Extension;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use helios_auth::Principal;
use helios_persistence::core::{ConditionalStorage, ResourceStorage, SearchProvider, WriteKind};
use tracing::debug;

use super::sof::terminology_materialize::{
    MaterializeBudget, apply_for_direct_write, apply_materialization,
    created_unmaterialized_outcome, plan_for_direct_write, transform_for_direct_write,
};

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirResource, FhirVersionExtractor, TenantExtractor};
use crate::fhir_types::admit_resource_type;
use crate::handlers::extract_patient_from_resource;
use crate::middleware::conditional::ConditionalHeaders;
use crate::middleware::content_type::{FhirFormat, negotiate_format};
use crate::middleware::prefer::PreferHeader;
use crate::patient_compartment::PatientCompartment;
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
    compartment: PatientCompartment,
    conditional: ConditionalHeaders,
    prefer: PreferHeader,
    req_headers: HeaderMap,
    principal: Option<Extension<Principal>>,
    FhirResource(resource): FhirResource,
) -> RestResult<Response>
where
    S: ResourceStorage + ConditionalStorage + SearchProvider + Send + Sync + 'static,
{
    let principal = principal.map(|Extension(p)| p);
    // Determine FHIR version from header or use server default
    let fhir_version = version.storage_version_or(state.config().default_fhir_version);

    admit_resource_type(&resource_type, &resource, fhir_version)?;

    // A `patient/` scope creates inside the launch patient's compartment only
    // (#1618): the submitted payload must reference the launch patient through
    // a compartment parameter. A conditional create is refused outright — its
    // `If-None-Exist` match could be another patient's resource, which the
    // response would then disclose.
    if compartment.is_confined() {
        if conditional.if_none_exist().is_some() {
            return Err(compartment.outside_compartment(
                "a conditional create is not available under a patient/ scope: the If-None-Exist match cannot be confined to the launch patient's compartment".to_string(),
            ));
        }
        compartment.assert_member(
            || state.storage().search_param_registry(tenant.context()),
            fhir_version,
            &resource,
            "create",
        )?;
    }

    // AuditEvent resources are immutable — block write operations
    if resource_type == "AuditEvent" {
        return Err(RestError::MethodNotAllowed {
            method: "POST".to_string(),
            resource_type: resource_type.to_string(),
        });
    }

    // Negotiate response format from Accept header
    let negotiated = negotiate_format(&req_headers, None);

    debug!(
        resource_type = %resource_type,
        tenant = %tenant.tenant_id(),
        fhir_version = %fhir_version,
        conditional = ?conditional.if_none_exist(),
        "Processing create request"
    );

    // Per http.html#create "the server ignores the id provided in the
    // resource" — a create always assigns a fresh server id. Honoring a
    // client id made a second POST of the same document answer 409, which
    // broke standard transaction ingestion of bundles that repeat shared
    // resources (#647). Internal callers that rely on caller-supplied ids
    // for idempotency (spec seeding) go straight to storage, not through
    // this handler.
    let mut resource = resource;
    if let Some(obj) = resource.as_object_mut() {
        obj.remove("id");
    }

    // #1824 T2: a ValueSet some SQL Library depends on gains its expansion
    // in the candidate, so the one write below persists the final content
    // and validation sees it.
    let budget = MaterializeBudget::start(state.config());
    let resource =
        transform_for_direct_write(&state, tenant.context(), &resource_type, resource, &budget)
            .await?;

    // Write-path validation (HFS_VALIDATION_MODE: off | log | enforce).
    state
        .validation()
        .check_write(tenant.tenant_id(), fhir_version, &resource_type, &resource)
        .await?;

    // #1014: an unknown ViewDefinition.resource is rejected on every write.
    super::sof::reject_unknown_view_definition_resource(&resource_type, &resource)?;

    // Check for conditional create
    if let Some(search_params) = conditional.if_none_exist() {
        super::conditional_support::require_create(state.storage())?;
        debug!(search_params = %search_params, "Processing conditional create");

        // #1824 T1 on a conditional create (D13 c, d): a pre-search decides
        // whether phase A runs at all — a match returns the existing
        // resource without touching the terminology server — then the
        // atomic conditional create runs first and phase B only after
        // `Created`, so a lost race never writes anything.
        let plan = match state
            .storage()
            .resolve_conditional_matches(tenant.context(), &resource_type, search_params)
            .await
        {
            Ok(matches) if matches.is_empty() => {
                plan_for_direct_write(
                    &state,
                    tenant.context(),
                    principal.as_ref(),
                    &resource_type,
                    &resource,
                    &budget,
                )
                .await?
            }
            Ok(_) => None,
            // A backend that cannot pre-search decides the match inside the
            // create; phase A then runs before it, writes nothing, and is
            // discarded on `Exists`.
            Err(helios_persistence::error::StorageError::Backend(
                helios_persistence::error::BackendError::UnsupportedCapability { .. },
            )) => {
                plan_for_direct_write(
                    &state,
                    tenant.context(),
                    principal.as_ref(),
                    &resource_type,
                    &resource,
                    &budget,
                )
                .await?
            }
            Err(e) => return Err(e.into()),
        };

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
                // Counted, but conditional writes announce nothing.
                super::write_event::report(
                    &state,
                    tenant.context(),
                    fhir_version,
                    &resource_type,
                    1,
                    None,
                );
                // Phase B after the committed create. A failure here leaves
                // the resource stored and unmaterialized: the 422 carries
                // its Location and ETag and names the PUT that retries.
                if let Some(plan) = plan
                    && !plan.is_empty()
                    && let Err(failure) =
                        apply_materialization(&state, tenant.context(), plan).await
                {
                    let headers = ResourceHeaders::from_stored(&stored, &state);
                    let location = state
                        .public_url_for_request(&tenant, [resource_type.as_str(), stored.id()]);
                    let outcome = created_unmaterialized_outcome(
                        &resource_type,
                        stored.id(),
                        stored.version_id(),
                        &location,
                        &failure,
                    );
                    let mut header_map = headers.to_header_map();
                    header_map.insert(header::LOCATION, location.parse().unwrap());
                    return format_resource_response(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        header_map,
                        &outcome,
                        negotiated.format,
                    );
                }
                // Stored StructureDefinitions feed the tenant's profile
                // registry.
                if resource_type == "StructureDefinition" {
                    state.validation().upsert_stored_profile(
                        tenant.tenant_id(),
                        fhir_version,
                        stored.content(),
                    );
                }
                let headers = ResourceHeaders::from_stored(&stored, &state);
                let location =
                    state.public_url_for_request(&tenant, [resource_type.as_str(), stored.id()]);

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

    // #1824 T1: phase A (reads only), then phase B immediately before the
    // outer write (D13 b).
    let plan = plan_for_direct_write(
        &state,
        tenant.context(),
        principal.as_ref(),
        &resource_type,
        &resource,
        &budget,
    )
    .await?;
    apply_for_direct_write(&state, tenant.context(), plan).await?;

    // Standard create
    let stored = state
        .storage()
        .create(tenant.context(), &resource_type, resource, fhir_version)
        .await?;
    super::write_event::report(
        &state,
        tenant.context(),
        fhir_version,
        &resource_type,
        1,
        Some(super::write_event::stored_notice(
            WriteKind::Create,
            &stored,
        )),
    );

    // Stored StructureDefinitions feed the tenant's profile registry.
    if resource_type == "StructureDefinition" {
        state.validation().upsert_stored_profile(
            tenant.tenant_id(),
            fhir_version,
            stored.content(),
        );
    }

    let headers = ResourceHeaders::from_stored(&stored, &state);
    let location = state.public_url_for_request(&tenant, [resource_type.as_str(), stored.id()]);

    debug!(
        resource_type = %resource_type,
        id = %stored.id(),
        "Resource created"
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
            format_resource_response(status, header_map, &outcome, format)
        }
        _ => {
            // Default: return=representation
            format_resource_response(status, header_map, &stored.content_with_meta(), format)
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
        _ => format_resource_response(
            StatusCode::OK,
            header_map,
            &stored.content_with_meta(),
            format,
        ),
    }
}
