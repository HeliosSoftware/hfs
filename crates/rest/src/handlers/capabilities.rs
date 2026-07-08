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

use super::sof::capability::build_sof_capabilities;

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirVersionExtractor, TenantExtractor};
use crate::fhir_types::get_resource_type_names_for_version;
use crate::middleware::content_type::{FhirContentType, negotiate_format};
use crate::responses::format_resource_response;
use crate::state::AppState;

/// Extension URL used to advertise the search modifiers a parameter supports.
///
/// FHIR R4/R5 `CapabilityStatement.rest.resource.searchParam` has no native
/// `modifier` element, so the modifiers a backend actually honors are carried as
/// repeating `valueCode` extensions on the `searchParam` entry.
const SEARCH_MODIFIER_EXTENSION_URL: &str =
    "http://heliossoftware.com/fhir/StructureDefinition/capabilitystatement-search-modifier";

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
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
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
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    // Get resource types for the requested FHIR version
    let resource_types = get_resource_type_names_for_version(version);

    // Advertise each resource type's real search parameters from the loaded
    // SearchParameter registry (not just the seven common params). The read
    // guard is held only across this synchronous build (no await).
    // `_contained` / `_containedType` are only advertised when the backend can
    // evaluate contained search (others reject them with 501).
    let supports_contained = state.storage().supports_contained_search();

    // Per-parameter-type modifier sets the backend actually honors. Used to
    // advertise supported modifiers (as `valueCode` extensions) on each
    // resource-specific search param.
    let modifier_map: std::collections::HashMap<SearchParamType, Vec<&'static str>> = [
        SearchParamType::Number,
        SearchParamType::Date,
        SearchParamType::String,
        SearchParamType::Token,
        SearchParamType::Reference,
        SearchParamType::Composite,
        SearchParamType::Quantity,
        SearchParamType::Uri,
        SearchParamType::Special,
    ]
    .into_iter()
    .map(|t| (t, state.storage().modifiers_for_param_type(t)))
    .collect();

    let registry = state.storage().search_param_registry().read();

    // Reverse-include index (target type -> "Source:code" tokens) for advertising
    // each resource's real `searchRevInclude` targets.
    let revinclude_by_target = build_revinclude_index(&registry);

    let resources: Vec<serde_json::Value> = resource_types
        .iter()
        .map(|rt| {
            build_resource_capability(
                rt,
                &registry,
                supports_contained,
                &modifier_map,
                &revinclude_by_target,
            )
        })
        .collect();

    #[allow(unused_mut)]
    let mut formats = vec!["json", "application/fhir+json"];
    #[cfg(feature = "xml")]
    {
        formats.push("xml");
        formats.push("application/fhir+xml");
    }

    // Standard operations, extended with SOF operations
    let mut operations = build_rest_operations(state);

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
        ]
    });

    // Inject the SOF extension array when present
    if let Some(ext) = sof_extension {
        rest_entry["extension"] = ext;
    }

    let mut statement = serde_json::json!({
        "resourceType": "CapabilityStatement",
        "status": "active",
        "date": chrono::Utc::now().to_rfc3339(),
        "kind": "instance",
        "fhirVersion": version.full_version(),
        "format": formats,
        "implementation": {
            "description": "Helios FHIR Server",
            "url": base_url
        },
        "rest": [rest_entry]
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

    // Advertise Bulk Data Submit operations when enabled.
    if state.bulk_submit_config().enabled {
        operations.push(serde_json::json!({
            "name": "bulk-submit",
            "definition": "http://hl7.org/fhir/uv/bulkdata/OperationDefinition/bulk-submit"
        }));
        operations.push(serde_json::json!({
            "name": "bulk-submit-status",
            "definition": "http://hl7.org/fhir/uv/bulkdata/OperationDefinition/bulk-submit-status"
        }));
    }

    statement["rest"][0]["operation"] = serde_json::Value::Array(operations);
    statement
}

/// Builds the `rest[0].operation` list, including SOF operations.
///
/// `viewdefinition-run` and `sqlquery-run` are always declared when SOF is enabled.
/// `viewdefinition-export` and `sqlquery-export` are declared only when an
/// export controller is wired.
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
        ops.push(serde_json::json!({
            "name": "sqlquery-export",
            "definition": "http://sql-on-fhir.org/OperationDefinition/$sqlquery-export"
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
fn build_resource_capability(
    resource_type: &str,
    registry: &SearchParameterRegistry,
    supports_contained: bool,
    modifier_map: &std::collections::HashMap<SearchParamType, Vec<&'static str>>,
    revinclude_by_target: &std::collections::HashMap<String, std::collections::BTreeSet<String>>,
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
        "searchParam": build_search_params(resource_type, registry, supports_contained, modifier_map)
    });

    // Advertise real `_include` targets: one "Type:code" per reference param on
    // this type. Omitted entirely when the type has no reference params.
    let search_include = build_search_includes(resource_type, registry);
    if !search_include.is_empty() {
        entry["searchInclude"] = serde_json::Value::Array(search_include);
    }

    // Advertise real `_revinclude` targets: "Source:code" tokens from other
    // types' reference params that point at this type.
    if let Some(rev) = revinclude_by_target.get(resource_type) {
        if !rev.is_empty() {
            entry["searchRevInclude"] = serde_json::Value::Array(
                rev.iter().cloned().map(serde_json::Value::String).collect(),
            );
        }
    }
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
    supports_contained: bool,
    modifier_map: &std::collections::HashMap<SearchParamType, Vec<&'static str>>,
) -> Vec<serde_json::Value> {
    let mut params = build_common_search_params(supports_contained);
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
        // Advertise the modifiers this backend honors for the param's type.
        // R4/R5 `searchParam` has no `modifier` element, so they ride along as
        // repeating `valueCode` extensions.
        if let Some(modifiers) = modifier_map.get(&def.param_type) {
            if !modifiers.is_empty() {
                param["extension"] = serde_json::Value::Array(
                    modifiers
                        .iter()
                        .map(|m| {
                            serde_json::json!({
                                "url": SEARCH_MODIFIER_EXTENSION_URL,
                                "valueCode": m,
                            })
                        })
                        .collect(),
                );
            }
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

/// Builds the reverse-include index used for `searchRevInclude`: maps each target
/// resource type to the sorted set of "SourceType:code" tokens from every active
/// reference search param that targets it.
fn build_revinclude_index(
    registry: &SearchParameterRegistry,
) -> std::collections::HashMap<String, std::collections::BTreeSet<String>> {
    let mut index: std::collections::HashMap<String, std::collections::BTreeSet<String>> =
        std::collections::HashMap::new();
    for source_type in registry.resource_types() {
        for def in registry.get_active_params(&source_type) {
            if def.param_type != SearchParamType::Reference || def.code.starts_with('_') {
                continue;
            }
            if let Some(targets) = &def.target {
                let token = format!("{}:{}", source_type, def.code);
                for target in targets {
                    index
                        .entry(target.clone())
                        .or_default()
                        .insert(token.clone());
                }
            }
        }
    }
    index
}

/// Returns the `searchInclude` tokens ("Type:code") for a resource type: one per
/// active reference search param defined on it.
fn build_search_includes(
    resource_type: &str,
    registry: &SearchParameterRegistry,
) -> Vec<serde_json::Value> {
    let mut includes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for def in registry.get_active_params(resource_type) {
        if def.param_type == SearchParamType::Reference && !def.code.starts_with('_') {
            includes.insert(format!("{}:{}", resource_type, def.code));
        }
    }
    includes
        .into_iter()
        .map(serde_json::Value::String)
        .collect()
}

/// Builds common search parameters supported by all resources.
///
/// `_contained` / `_containedType` are only included when `supports_contained`
/// is set (the backend can evaluate contained search). `_list` is always
/// advertised (resolved application-side on every backend).
fn build_common_search_params(supports_contained: bool) -> Vec<serde_json::Value> {
    let mut params = vec![
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
        serde_json::json!({
            "name": "_list",
            "type": "special",
            "documentation": "Resources referenced by the given List resource"
        }),
    ];

    if supports_contained {
        params.push(serde_json::json!({
            "name": "_contained",
            "type": "token",
            "documentation": "Whether to search contained resources (false|true|both)"
        }));
        params.push(serde_json::json!({
            "name": "_containedType",
            "type": "token",
            "documentation": "Return the container or the contained resource (container|contained)"
        }));
    }

    params
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param_names(params: &[serde_json::Value]) -> Vec<String> {
        params
            .iter()
            .filter_map(|p| p["name"].as_str().map(str::to_string))
            .collect()
    }

    #[test]
    fn common_params_always_advertise_list() {
        let names = param_names(&build_common_search_params(false));
        assert!(names.contains(&"_list".to_string()));
        assert!(names.contains(&"_text".to_string()));
    }

    #[test]
    fn contained_params_gated_on_capability() {
        let without = param_names(&build_common_search_params(false));
        assert!(!without.contains(&"_contained".to_string()));
        assert!(!without.contains(&"_containedType".to_string()));

        let with = param_names(&build_common_search_params(true));
        assert!(with.contains(&"_contained".to_string()));
        assert!(with.contains(&"_containedType".to_string()));
    }

    fn ref_param(
        url: &str,
        code: &str,
        base: &str,
        targets: &[&str],
    ) -> helios_persistence::search::SearchParameterDefinition {
        helios_persistence::search::SearchParameterDefinition::new(
            url,
            code,
            SearchParamType::Reference,
            format!("{base}.{code}"),
        )
        .with_base([base])
        .with_targets(targets.iter().copied())
    }

    #[test]
    fn includes_and_revincludes_derived_from_reference_params() {
        use helios_persistence::search::SearchParameterRegistry;

        let mut reg = SearchParameterRegistry::new();
        reg.register(ref_param(
            "http://example.org/Patient-general-practitioner",
            "general-practitioner",
            "Patient",
            &["Practitioner", "Organization"],
        ))
        .unwrap();
        reg.register(ref_param(
            "http://example.org/Observation-subject",
            "subject",
            "Observation",
            &["Patient"],
        ))
        .unwrap();

        // searchInclude for a type = its own reference params as "Type:code".
        let includes: Vec<String> = build_search_includes("Patient", &reg)
            .into_iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(includes, vec!["Patient:general-practitioner".to_string()]);

        // searchRevInclude index: each target type maps to the "Source:code"
        // tokens that point at it.
        let rev = build_revinclude_index(&reg);
        assert!(rev.get("Patient").unwrap().contains("Observation:subject"));
        assert!(
            rev.get("Practitioner")
                .unwrap()
                .contains("Patient:general-practitioner")
        );
        // Observation has no reference param pointing at it.
        assert!(!rev.contains_key("Observation"));
    }

    #[test]
    fn resource_specific_params_carry_modifier_extension() {
        use helios_persistence::search::{SearchParameterDefinition, SearchParameterRegistry};
        use std::collections::HashMap;

        let mut reg = SearchParameterRegistry::new();
        reg.register(
            SearchParameterDefinition::new(
                "http://example.org/Patient-name",
                "name",
                SearchParamType::String,
                "Patient.name",
            )
            .with_base(["Patient"]),
        )
        .unwrap();

        let mut modifier_map: HashMap<SearchParamType, Vec<&'static str>> = HashMap::new();
        modifier_map.insert(SearchParamType::String, vec!["exact", "contains"]);

        let params = build_search_params("Patient", &reg, false, &modifier_map);
        let name = params
            .iter()
            .find(|p| p["name"] == "name")
            .expect("name param advertised");
        let ext = name["extension"]
            .as_array()
            .expect("modifier extension present");
        let codes: Vec<&str> = ext
            .iter()
            .map(|e| e["valueCode"].as_str().unwrap())
            .collect();
        assert_eq!(codes, vec!["exact", "contains"]);
        assert_eq!(ext[0]["url"], SEARCH_MODIFIER_EXTENSION_URL);

        // Common control params (e.g. _id) do not carry the modifier extension.
        let id = params.iter().find(|p| p["name"] == "_id").unwrap();
        assert!(id.get("extension").is_none());
    }
}
