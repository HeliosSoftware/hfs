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
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::search::SearchParameterRegistry;
use helios_persistence::types::SearchParamType;
use tracing::debug;

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
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    // Determine which version to describe (from Accept header or default)
    let fhir_version = version
        .accept_version()
        .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

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
    S: ResourceStorage + SearchProvider,
{
    let backend_name = state.storage().backend_name();

    // Get resource types for the requested FHIR version
    let resource_types = get_resource_type_names_for_version(version);

    // Advertise each resource type's real search parameters from the loaded
    // SearchParameter registry (not just the seven common params). The read
    // guard is held only across this synchronous build (no await).
    let registry = state.storage().search_param_registry().read();
    let resources: Vec<serde_json::Value> = resource_types
        .iter()
        .map(|rt| build_resource_capability(rt, &registry))
        .collect();

    #[allow(unused_mut)]
    let mut formats = vec!["json", "application/fhir+json"];
    #[cfg(feature = "xml")]
    {
        formats.push("xml");
        formats.push("application/fhir+xml");
    }

    let mut operations = vec![serde_json::json!({
        "name": "versions",
        "definition": "http://hl7.org/fhir/OperationDefinition/CapabilityStatement-versions"
    })];

    let mut statement = serde_json::json!({
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
        "rest": [{
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
        }]
    });

    // Advertise Bulk Data Export operations when enabled.
    if state.bulk_export_config().enabled {
        operations.push(serde_json::json!({
            "name": "export",
            "definition": "http://hl7.org/fhir/uv/bulkdata/OperationDefinition/export"
        }));
        operations.push(serde_json::json!({
            "name": "patient-export",
            "definition": "http://hl7.org/fhir/uv/bulkdata/OperationDefinition/patient-export"
        }));
        operations.push(serde_json::json!({
            "name": "group-export",
            "definition": "http://hl7.org/fhir/uv/bulkdata/OperationDefinition/group-export"
        }));
        statement["instantiates"] =
            serde_json::json!(["http://hl7.org/fhir/uv/bulkdata/CapabilityStatement/bulk-data"]);
    }

    statement["rest"][0]["operation"] = serde_json::Value::Array(operations);
    statement
}

/// Builds the capability entry for a resource type.
fn build_resource_capability(
    resource_type: &str,
    registry: &SearchParameterRegistry,
) -> serde_json::Value {
    let mut entry = serde_json::json!({
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
        "searchParam": build_search_params(resource_type, registry)
    });
    // Bulk Data Access IG: per-resource `$export` operation entries on Patient
    // and Group, in addition to the system-level `$export` advertised at
    // `rest[0].operation`.
    match resource_type {
        "Patient" => {
            entry["operation"] = serde_json::json!([{
                "name": "export",
                "definition": "http://hl7.org/fhir/uv/bulkdata/OperationDefinition/patient-export"
            }]);
        }
        "Group" => {
            entry["operation"] = serde_json::json!([{
                "name": "export",
                "definition": "http://hl7.org/fhir/uv/bulkdata/OperationDefinition/group-export"
            }]);
        }
        _ => {}
    }
    entry
}

/// Builds the full `searchParam` list for a resource type: the seven common
/// params plus the resource-specific params from the loaded SearchParameter
/// registry. Control/common params (those starting with `_`) and duplicates are
/// skipped.
fn build_search_params(
    resource_type: &str,
    registry: &SearchParameterRegistry,
) -> Vec<serde_json::Value> {
    let mut params = build_common_search_params();
    let mut seen: std::collections::HashSet<String> = params
        .iter()
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .collect();

    for def in registry.get_active_params(resource_type) {
        if def.code.starts_with('_') || !seen.insert(def.code.clone()) {
            continue;
        }
        let mut param = serde_json::json!({
            "name": def.code,
            "type": fhir_search_param_type(def.param_type),
            "definition": def.url,
        });
        if let Some(doc) = &def.description {
            param["documentation"] = serde_json::Value::String(doc.clone());
        }
        params.push(param);
    }
    params
}

/// Maps an internal search-parameter type to its FHIR CapabilityStatement
/// `searchParam.type` code.
fn fhir_search_param_type(t: SearchParamType) -> &'static str {
    match t {
        SearchParamType::Number => "number",
        SearchParamType::Date => "date",
        SearchParamType::String => "string",
        SearchParamType::Token => "token",
        SearchParamType::Reference => "reference",
        SearchParamType::Composite => "composite",
        SearchParamType::Quantity => "quantity",
        SearchParamType::Uri => "uri",
        SearchParamType::Special => "special",
    }
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
            "type": "special",
            "documentation": "Search on the narrative of the resource"
        }),
        serde_json::json!({
            "name": "_content",
            "type": "special",
            "documentation": "Search on the entire content of the resource"
        }),
    ]
}
