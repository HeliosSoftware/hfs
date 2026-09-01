//! Version read (vread) interaction handler.
//!
//! Implements the FHIR [vread interaction](https://hl7.org/fhir/http.html#vread):
//! `GET [base]/[type]/[id]/_history/[vid]`
//!
//! Backed by the `VersionedStorage::vread` operation, which every first-class
//! storage backend implements and which `CompositeStorage` delegates to its
//! primary.

use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use helios_persistence::core::{ResourceStorage, VersionedStorage};
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirVersionExtractor, TenantExtractor};
use crate::middleware::content_type::{FhirContentType, negotiate_format};
use crate::responses::format_resource_response;
use crate::responses::headers::ResourceHeaders;
use crate::state::AppState;

/// Handler for the vread interaction.
///
/// Reads a specific version of a resource.
///
/// # HTTP Request
///
/// `GET [base]/[type]/[id]/_history/[vid]`
///
/// # Response
///
/// - `200 OK` - Version found, returns the resource
/// - `404 Not Found` - Resource or version does not exist
/// - `410 Gone` - The named version is the one that deleted the resource
///
/// # Example
///
/// ```http
/// GET /Patient/123/_history/2 HTTP/1.1
/// Host: fhir.example.com
/// Accept: application/fhir+json
/// ```
pub async fn vread_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id, version_id)): Path<(String, String, String)>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    req_headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> RestResult<Response>
where
    S: ResourceStorage + VersionedStorage + Send + Sync,
{
    debug!(
        resource_type = %resource_type,
        id = %id,
        version_id = %version_id,
        tenant = %tenant.tenant_id(),
        "Processing vread request"
    );

    let stored = state
        .storage()
        .vread(tenant.context(), &resource_type, &id, &version_id)
        .await?;

    match stored {
        Some(stored) if stored.is_deleted() => {
            // The named version is the tombstone — the interaction that deleted
            // the resource, not a state it ever had. FHIR's vread interaction
            // (https://hl7.org/fhir/http.html#vread) answers that with `410
            // Gone`, and the history Bundle has always agreed: a `Delete` entry
            // carries `request.method = DELETE` and no `resource` at all
            // (`history_entry_to_json`). Returning `200` with the body the
            // resource had *before* the delete, which is what this did,
            // contradicts both — a deleted version is not a readable version.
            //
            // `return_gone` is the same switch a deleted instance read uses, so
            // a deployment that prefers `404` over `410` for deleted content
            // keeps one answer for both interactions rather than two.
            //
            // The backend is free to stop storing that body once nothing can
            // ask for it, which the PostgreSQL delete path now does.
            if state.return_gone() {
                Err(RestError::Gone { resource_type, id })
            } else {
                Err(RestError::VersionNotFound {
                    resource_type,
                    id,
                    version_id,
                })
            }
        }
        Some(stored) => {
            // If the client requested a specific FHIR version, verify it matches.
            if let Some(requested) = version.accept_version() {
                if stored.fhir_version() != requested {
                    return Err(RestError::NotAcceptable {
                        message: format!(
                            "Resource is FHIR {} but {} was requested",
                            stored.fhir_version().as_mime_param(),
                            requested.as_mime_param()
                        ),
                    });
                }
            }

            // Negotiate response format.
            let format_param = params.get("_format").map(|s| s.as_str());
            let negotiated = negotiate_format(&req_headers, format_param);

            // Build response headers, including fhirVersion in Content-Type.
            let content_type =
                FhirContentType::with_version(negotiated.format, stored.fhir_version());
            let mut headers = ResourceHeaders::from_stored(&stored, &state)
                .with_content_type(content_type.to_header_value())
                .to_header_map();
            headers.insert(
                header::CONTENT_TYPE,
                content_type.to_header_value().parse().unwrap(),
            );

            debug!(
                resource_type = %resource_type,
                id = %id,
                version = %stored.version_id(),
                fhir_version = %stored.fhir_version(),
                "Returning resource version"
            );

            format_resource_response(StatusCode::OK, headers, stored.content(), negotiated.format)
                .map_err(|_| RestError::InternalError {
                    message: "Failed to serialize response".to_string(),
                })
        }
        None => Err(RestError::VersionNotFound {
            resource_type,
            id,
            version_id,
        }),
    }
}
