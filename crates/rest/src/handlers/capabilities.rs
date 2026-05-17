//! Capabilities (CapabilityStatement) handler.
//!
//! Implements the FHIR [capabilities interaction](https://hl7.org/fhir/http.html#capabilities):
//! `GET [base]/metadata`
//!
//! Per FHIR spec, the CapabilityStatement.fhirVersion is 1..1 (single value).
//! Multi-version servers return a version-specific CapabilityStatement based on the
//! `fhirVersion` parameter in the Accept header.
//!
//! # Tenant-Aware Base URL
//!
//! When using URL-based tenant routing, the CapabilityStatement's implementation.url
//! includes the tenant prefix. For example:
//! - Header-based: `http://fhir.example.com/`
//! - URL-based: `http://fhir.example.com/acme/`

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;
use tracing::debug;

use super::sof::capability::build_sof_capabilities;

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirVersionExtractor, TenantExtractor};
use crate::fhir_types::get_resource_type_names_for_version;
use crate::middleware::content_type::{FhirContentType, negotiate_format};
use crate::responses::format_resource_response;
use crate::state::AppState;

/// Handler for the capabilities interaction.
///
/// Returns a CapabilityStatement describing the server's capabilities.
///
/// Per FHIR spec, the CapabilityStatement.fhirVersion is a single value.
/// If the Accept header includes a `fhirVersion` parameter, the server returns
/// a CapabilityStatement for that specific version. Otherwise, the default
/// FHIR version is used.
///
/// # Tenant-Aware Base URL
///
/// When the tenant is resolved from a URL path (e.g., `/acme/metadata`), the
/// CapabilityStatement's `implementation.url` includes the tenant prefix to
/// ensure clients use the correct base URL for subsequent requests.
///
/// # HTTP Request
///
/// `GET [base]/metadata`
///
/// # Headers
///
/// - `Accept: application/fhir+json; fhirVersion=4.0` - Request R4 capabilities
/// - `Accept: application/fhir+json; fhirVersion=5.0` - Request R5 capabilities
///
/// # Response
///
/// Returns a CapabilityStatement resource (200 OK) with Content-Type including
/// the fhirVersion parameter.
pub async fn capabilities_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    req_headers: HeaderMap,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    // Determine which version to describe (from Accept header or default)
    let fhir_version = version.accept_version().unwrap_or_default();

    debug!(
        fhir_version = %fhir_version,
        tenant = %tenant.tenant_id(),
        tenant_source = %tenant.source(),
        "Processing capabilities request"
    );

    // Build tenant-aware base URL
    let base_url = if tenant.is_url_based() {
        format!(
            "{}/{}",
            state.base_url().trim_end_matches('/'),
            tenant.tenant_id()
        )
    } else {
        state.base_url().to_string()
    };

    let capability_statement = build_capability_statement(&state, fhir_version, &base_url);

    // Negotiate response format
    let negotiated = negotiate_format(&req_headers, None);

    // Build response with fhirVersion in Content-Type
    let content_type = FhirContentType::with_version(negotiated.format, fhir_version);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        content_type.to_header_value().parse().unwrap(),
    );

    format_resource_response(
        StatusCode::OK,
        headers,
        &capability_statement,
        negotiated.format,
    )
    .map_err(|_| RestError::InternalError {
        message: "Failed to serialize response".to_string(),
    })
}

/// Builds a CapabilityStatement describing server capabilities for a specific FHIR version.
fn build_capability_statement<S>(
    state: &AppState<S>,
    version: FhirVersion,
    base_url: &str,
) -> serde_json::Value
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let backend_name = state.storage().backend_name();

    // Get resource types for the requested FHIR version
    let resource_types = get_resource_type_names_for_version(version);

    let resources: Vec<serde_json::Value> = resource_types
        .iter()
        .map(|rt| build_resource_capability(rt))
        .collect();

    #[allow(unused_mut)]
    let mut formats = vec!["json", "application/fhir+json"];
    #[cfg(feature = "xml")]
    {
        formats.push("xml");
        formats.push("application/fhir+xml");
    }

    // Standard operations, extended with SOF operations
    let operations = build_rest_operations(state);

    // Optional SOF extension block on the rest[0] element
    let sof_extension = build_sof_rest_extension(state);

    let mut rest_entry = serde_json::json!({
        "mode": "server",
        "documentation": "Helios FHIR RESTful API",
        "security": {
            "cors": state.config().enable_cors,
            "description": "This server supports CORS for cross-origin requests"
        },
        "resource": resources,
        "interaction": [
            { "code": "transaction" },
            { "code": "batch" },
            { "code": "history-system" },
            { "code": "search-system" }
        ],
        "operation": operations
    });

    // Inject the SOF extension array when present
    if let Some(ext) = sof_extension {
        rest_entry["extension"] = ext;
    }

    serde_json::json!({
        "resourceType": "CapabilityStatement",
        "status": "active",
        "date": chrono::Utc::now().to_rfc3339(),
        "kind": "instance",
        "fhirVersion": version.full_version(),
        "format": formats,
        "implementation": {
            "description": format!("Helios FHIR Server ({})", backend_name),
            "url": base_url
        },
        "rest": [rest_entry]
    })
}

/// Builds the `rest[0].operation` list, including SOF operations.
///
/// `viewdefinition-run` and `sqlquery-run` are always declared when SOF is enabled.
/// `viewdefinition-export` is declared only when an export controller is wired.
fn build_rest_operations<S: ResourceStorage + Send + Sync + 'static>(
    state: &AppState<S>,
) -> Vec<serde_json::Value> {
    let mut ops = vec![
        serde_json::json!({
            "name": "validate",
            "definition": "http://hl7.org/fhir/OperationDefinition/Resource-validate"
        }),
        serde_json::json!({
            "name": "versions",
            "definition": "http://hl7.org/fhir/OperationDefinition/CapabilityStatement-versions"
        }),
        serde_json::json!({
            "name": "viewdefinition-run",
            "definition": "http://sql-on-fhir.org/OperationDefinition/$viewdefinition-run"
        }),
        serde_json::json!({
            "name": "sqlquery-run",
            "definition": "http://sql-on-fhir.org/OperationDefinition/$sqlquery-run"
        }),
    ];

    if state.export_controller().is_some() {
        ops.push(serde_json::json!({
            "name": "viewdefinition-export",
            "definition": "http://sql-on-fhir.org/OperationDefinition/$viewdefinition-export"
        }));
    }

    ops
}

/// Builds the `extension` array on `rest[0]` advertising SOF-specific flags.
fn build_sof_rest_extension<S: ResourceStorage + Send + Sync + 'static>(
    state: &AppState<S>,
) -> Option<serde_json::Value> {
    let caps = build_sof_capabilities(state);
    // Inline the SOF Parameters as a contained extension value so consumers
    // that understand the SOF spec can discover the flags without an extra request.
    Some(serde_json::json!([
        {
            "url": "https://build.fhir.org/ig/FHIR/sql-on-fhir-v2/StructureDefinition-sof-capabilities.html",
            "valueReference": {
                "reference": "/$sql-on-fhir-capabilities",
                "display": "SQL-on-FHIR Capabilities"
            }
        },
        {
            "url": "https://build.fhir.org/ig/FHIR/sql-on-fhir-v2/StructureDefinition-sof-capabilities-inline.html",
            "valueAttachment": {
                "contentType": "application/json",
                "data": serde_json::to_string(&caps).unwrap_or_default()
            }
        }
    ]))
}

/// Builds the capability entry for a resource type.
fn build_resource_capability(resource_type: &str) -> serde_json::Value {
    serde_json::json!({
        "type": resource_type,
        "profile": format!("http://hl7.org/fhir/StructureDefinition/{}", resource_type),
        "interaction": [
            { "code": "read" },
            { "code": "vread" },
            { "code": "update" },
            { "code": "patch" },
            { "code": "delete" },
            { "code": "history-instance" },
            { "code": "history-type" },
            { "code": "create" },
            { "code": "search-type" }
        ],
        "versioning": "versioned",
        "readHistory": true,
        "updateCreate": true,
        "conditionalCreate": true,
        "conditionalRead": "full-support",
        "conditionalUpdate": true,
        "conditionalDelete": "single",
        "searchInclude": ["*"],
        "searchRevInclude": ["*"],
        "searchParam": build_common_search_params()
    })
}

/// Builds common search parameters supported by all resources.
fn build_common_search_params() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "_id",
            "type": "token",
            "documentation": "Logical id of this artifact"
        }),
        serde_json::json!({
            "name": "_lastUpdated",
            "type": "date",
            "documentation": "When the resource version last changed"
        }),
        serde_json::json!({
            "name": "_tag",
            "type": "token",
            "documentation": "Tags applied to this resource"
        }),
        serde_json::json!({
            "name": "_profile",
            "type": "uri",
            "documentation": "Profiles this resource claims to conform to"
        }),
        serde_json::json!({
            "name": "_security",
            "type": "token",
            "documentation": "Security Labels applied to this resource"
        }),
        serde_json::json!({
            "name": "_text",
            "type": "string",
            "documentation": "Search on the narrative of the resource"
        }),
        serde_json::json!({
            "name": "_content",
            "type": "string",
            "documentation": "Search on the entire content of the resource"
        }),
    ]
}
