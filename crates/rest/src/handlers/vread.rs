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
