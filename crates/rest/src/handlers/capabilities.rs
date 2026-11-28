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
use helios_persistence::core::{
    BundleProvider, ConditionalStorage, ResourceStorage, SearchProvider,
};
use helios_persistence::search::{SearchParameterDefinition, SearchParameterRegistry};
use helios_persistence::types::SearchParamType;
use tracing::debug;

use crate::error::RestResult;
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
    S: ResourceStorage
        + SearchProvider
        + BundleProvider
        + ConditionalStorage
        + Send
        + Sync
        + 'static,
{
    // Determine which version to describe (from Accept header or default)
    let fhir_version = version.accept_version_or(state.config().default_fhir_version);

    debug!(
        fhir_version = %fhir_version,
        tenant = %tenant.tenant_id(),
        tenant_source = %tenant.source(),
        "Processing capabilities request"
    );

    let base_url = state.public_base_url_for_request(&tenant);

    let capability_statement =
        build_capability_statement(&state, tenant.context(), fhir_version, &base_url);

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
}

/// Builds a CapabilityStatement describing server capabilities for a specific FHIR version.
fn build_capability_statement<S>(
    state: &AppState<S>,
    tenant: &helios_persistence::tenant::TenantContext,
    version: FhirVersion,
    base_url: &str,
) -> serde_json::Value
where
    S: ResourceStorage
        + SearchProvider
        + BundleProvider
        + ConditionalStorage
        + Send
        + Sync
        + 'static,
{
    // Get resource types for the requested FHIR version
    let resource_types = get_resource_type_names_for_version(version);

    // Advertise each resource type's real search parameters from the loaded
    // SearchParameter registry (not just the seven common params). The read
    // guard is held only across this synchronous build (no await).
    // `_contained` / `_containedType` are only advertised when the backend can
    // evaluate contained search (others reject them with 501).
    let supports_contained = state.storage().supports_contained_search();

    // `search-type` and the `searchParam` / `searchInclude` / `searchRevInclude`
    // lists are the storage's to declare, not literals. They were listed for
    // every resource type on every backend, while a standalone S3 deployment
    // answers every search `501` (its `search` returns `UnsupportedCapability`),
    // so the statement promised an interaction no request could use (#514).
    let supports_search = state.storage().supports_search();

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

    // Which parameters the storage's `search` serves: a parameter it refuses
    // is left out of `searchParam`, and a reference parameter it refuses out
    // of `searchInclude` / `searchRevInclude` too (#2593). FHIR's
    // `searchParam` lists the "Search parameters for implementations to
    // support and/or make use of"
    // (https://hl7.org/fhir/R4/capabilitystatement-definitions.html#CapabilityStatement.rest.resource.searchParam).
    let storage = state.storage();
    let serves = |resource_type: &str, code: &str, param_type: SearchParamType| {
        storage.serves_search_param(resource_type, code, param_type)
    };

    let registry_arc = state.storage().search_param_registry(tenant);
    let registry = registry_arc.read();

    // Reverse-include index (target type -> "Source:code" tokens) for advertising
    // each resource's real `searchRevInclude` targets.
    let revinclude_by_target = build_revinclude_index(&registry, &serves);

    // The conditional interactions are the storage's to declare, not literals:
    // S3 serves only an identifier-scoped create on its own, and a composite's
    // answer depends on how it is composed (#1384). The conditional handlers
    // refuse with `501` from this same source.
    let conditionals = super::conditional_support::advertised(state.storage(), version);

    // Bulk Data export, at every level, is listed only where a kick-off can be
    // served: the handlers answer `501` when it is disabled by config and when
    // no job store is wired (standalone S3, and the backends whose job store
    // is a SQLite sidecar in a build without the `sqlite` feature), so the
    // statement reads the same two things (#3015).
    let serves_bulk_export = serves_bulk_export(state);

    let resources: Vec<serde_json::Value> = resource_types
        .iter()
        .map(|rt| {
            build_resource_capability(
                rt,
                &conditionals,
                &registry,
                supports_search,
                supports_contained,
                serves_bulk_export,
                &modifier_map,
                &revinclude_by_target,
                &serves,
                version,
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

    // Advertise `transaction` only when the storage backend can actually
    // honour all-or-nothing semantics. This was an unconditional literal, so an
    // s3-elasticsearch deployment published transaction support it had no way
    // to provide — a bundle that timed out left 466 of 473 entries committed
    // (#489). `batch` is unconditional: every bundle provider can process
    // entries independently, which is exactly what design discussion #28
    // prescribes for a backend without transaction support.
    let mut system_interactions = Vec::new();
    if state.storage().supports_atomic_transactions() {
        system_interactions.push(serde_json::json!({ "code": "transaction" }));
    }
    system_interactions.push(serde_json::json!({ "code": "batch" }));
    system_interactions.push(serde_json::json!({ "code": "history-system" }));
    // No `search-system`: `GET [base]?params` and `POST [base]/_search` are
    // refused with `501` (`search_system_not_supported_handler`). It was listed
    // here unconditionally while no route served it (#1338).

    // Standard operations, extended with the system-level SQL on FHIR
    // operations. Partial parameter support is advertised through the
    // OperationDefinitions those entries cite, not through an extension block —
    // the pre-ballot `sof-capabilities` extension has no counterpart in
    // 3.0.0-ballot.
    let mut operations = build_rest_operations(state);

    // When auth is on, a client must be able to discover SMART App Launch from
    // the CapabilityStatement itself, not only from
    // `/.well-known/smart-configuration`: SMART's discovery reads both, and the
    // security block was a `cors`-only literal regardless of auth (#1441). The
    // endpoints come from the same `AuthConfig` that backs the discovery
    // document, so the two never disagree. Advertised only when auth is enabled
    // *and* both required oauth-uris (authorize, token) are configured — a plain
    // bearer deployment with no SMART endpoints is not SMART-on-FHIR and stays
    // `cors`-only rather than publishing an incomplete profile.
    let mut security = serde_json::json!({
        "cors": state.config().enable_cors,
        "description": "This server supports CORS for cross-origin requests"
    });
    let auth = state.auth_config();
    if auth.enabled
        && auth.smart_authorize_endpoint.is_some()
        && auth.smart_token_endpoint.is_some()
    {
        security["service"] = serde_json::json!([{
            "coding": [{
                "system": "http://terminology.hl7.org/CodeSystem/restful-security-service",
                "code": "SMART-on-FHIR",
                "display": "SMART-on-FHIR"
            }],
            "text": "OAuth2 using SMART-on-FHIR profile (see http://docs.smarthealthit.org)"
        }]);

        // `oauth-uris` sub-extensions, in SMART's documented order. `authorize`
        // and `token` are always present here (guarded above); the rest are
        // added only when configured.
        let mut oauth_uris = vec![
            serde_json::json!({
                "url": "authorize",
                "valueUri": auth.smart_authorize_endpoint.as_deref().unwrap()
            }),
            serde_json::json!({
                "url": "token",
                "valueUri": auth.smart_token_endpoint.as_deref().unwrap()
            }),
        ];
        if let Some(uri) = auth.smart_introspection_endpoint.as_deref() {
            oauth_uris.push(serde_json::json!({ "url": "introspect", "valueUri": uri }));
        }
        if let Some(uri) = auth.smart_revocation_endpoint.as_deref() {
            oauth_uris.push(serde_json::json!({ "url": "revoke", "valueUri": uri }));
        }
        if let Some(uri) = auth.smart_registration_endpoint.as_deref() {
            oauth_uris.push(serde_json::json!({ "url": "register", "valueUri": uri }));
        }
        if let Some(uri) = auth.smart_management_endpoint.as_deref() {
            oauth_uris.push(serde_json::json!({ "url": "manage", "valueUri": uri }));
        }
        security["extension"] = serde_json::json!([{
            "url": "http://fhir-registry.smarthealthit.org/StructureDefinition/oauth-uris",
            "extension": oauth_uris
        }]);
    }

    let rest_entry = serde_json::json!({
        "mode": "server",
        "documentation": "Helios FHIR RESTful API",
        "security": security,
        "resource": resources,
        "interaction": system_interactions
    });

    let mut statement = serde_json::json!({
        "resourceType": "CapabilityStatement",
        "status": "active",
        "date": chrono::Utc::now().to_rfc3339(),
        "kind": "instance",
        "fhirVersion": version.full_version(),
        "format": formats,
        // `software` names the build a client is talking to; `implementation`
        // names this deployment of it. Without `software` the only place the
        // version appeared was `/health`, which FHIR clients do not read (#992).
        "software": crate::build_info::capability_software(),
        "implementation": {
            "description": crate::build_info::SOFTWARE_NAME,
            "url": base_url
        },
        "rest": [rest_entry]
    });

    // Advertise Bulk Data Export operations where they are served. The
    // bulk-data CapabilityStatement goes with them: `instantiates` says the
    // statement "must specify the full capability details"
    // (https://hl7.org/fhir/R4/capabilitystatement-definitions.html#CapabilityStatement.instantiates).
    if serves_bulk_export {
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

    // Advertise Bulk Data Submit operations where they are served: both
    // handlers answer `501` when submit is disabled by config and when no job
    // store is wired (an S3 deployment whose backend cannot run the submit
    // worker, such as bucket-per-tenant with no system bucket) (#3015).
    if serves_bulk_submit(state) {
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

/// Whether this deployment serves Bulk Data `$export`, at any level. It is
/// what every export handler checks before anything else: export enabled in
/// config, and a job store wired ([`AppState::with_bulk_export`] wires the
/// output store and download authorizer with it).
fn serves_bulk_export<S: ResourceStorage>(state: &AppState<S>) -> bool {
    state.bulk_export_config().enabled && state.bulk_export_jobs().is_some()
}

/// Whether this deployment serves `$bulk-submit` and `$bulk-submit-status`:
/// what both handlers check before anything else, submit enabled in config
/// and a job store wired.
fn serves_bulk_submit<S: ResourceStorage>(state: &AppState<S>) -> bool {
    state.bulk_submit_config().enabled && state.bulk_submit_jobs().is_some()
}

/// Builds the `rest[0].operation` list, including the SQL on FHIR operations.
///
/// Both data operations are invoked at the system level, so they are declared
/// here rather than under a resource type. Each is declared only when what its
/// handler needs is wired: `$sql-run` a SOF runner, `$sql-export` an export
/// controller. The server wires both only when SOF is enabled
/// (`HFS_SOF_ENABLED`). Without a runner `$sql-run` answers `501` for every
/// run over stored data, and serves only a ViewDefinition run over inline
/// `resource` data, so it is not declared (#3015).
///
/// `definition` points at *this server's* OperationDefinition rather than the
/// guide's. Citing the guide's would assert support for every parameter it
/// declares, and HFS supports a subset (no `context`, no `source`); per
/// operations-capability.html#partial-operation-support a server in that
/// position publishes its own definition with `base` naming the guide's. See
/// [`crate::handlers::sof::capability`].
fn build_rest_operations<S: ResourceStorage + Send + Sync + 'static>(
    state: &AppState<S>,
) -> Vec<serde_json::Value> {
    use crate::handlers::sof::capability::{
        REINDEX_DEFINITION_ID, SQL_EXPORT_DEFINITION_ID, SQL_RUN_DEFINITION_ID,
    };

    let mut ops = vec![
        serde_json::json!({
            "name": "validate",
            "definition": "http://hl7.org/fhir/OperationDefinition/Resource-validate"
        }),
        serde_json::json!({
            "name": "meta",
            "definition": "http://hl7.org/fhir/OperationDefinition/Resource-meta"
        }),
        serde_json::json!({
            "name": "meta-add",
            "definition": "http://hl7.org/fhir/OperationDefinition/Resource-meta-add"
        }),
        serde_json::json!({
            "name": "meta-delete",
            "definition": "http://hl7.org/fhir/OperationDefinition/Resource-meta-delete"
        }),
        serde_json::json!({
            "name": "versions",
            "definition": "http://hl7.org/fhir/OperationDefinition/CapabilityStatement-versions"
        }),
    ];

    if state.sof_runner().is_some() {
        ops.push(serde_json::json!({
            "name": "sql-run",
            "definition": format!("/OperationDefinition/{SQL_RUN_DEFINITION_ID}")
        }));
    }

    if state.export_controller().is_some() {
        ops.push(serde_json::json!({
            "name": "sql-export",
            "definition": format!("/OperationDefinition/{SQL_EXPORT_DEFINITION_ID}")
        }));
    }

    // `$reindex`, when this deployment has an index to rebuild. It is the
    // documented recovery for a search index that has fallen behind its
    // primary, and an operator who cannot find out it exists cannot use it —
    // which is how a composite deployment whose bulk-loaded data never reached
    // Elasticsearch had no discoverable way back (#1021). Declared only when
    // wired: on an S3 primary with no secondary there is no index at all and
    // the handler answers 501, so advertising it there would be a lie.
    //
    // `definition` names this server's own OperationDefinition, like the SQL on
    // FHIR operations above: `$reindex` is a HFS administrative operation with
    // no HL7 counterpart to cite.
    if state.reindex().is_some() {
        ops.push(serde_json::json!({
            "name": "reindex",
            "definition": format!("/OperationDefinition/{REINDEX_DEFINITION_ID}")
        }));
    }

    ops
}

/// Builds the capability entry for a resource type.
///
/// `supports_search` is the storage's [`SearchProvider::supports_search`].
/// When it is `false` the entry carries no `search-type` interaction and no
/// `searchParam`, `searchInclude` or `searchRevInclude`, for every type
/// (#514); everything else is unchanged.
///
/// `serves` is the storage's [`SearchProvider::serves_search_param`]: the
/// `searchParam`, `searchInclude` and `searchRevInclude` lists keep only what it
/// serves.
///
/// The per-resource operations are listed only where they are served (#3015):
/// Patient and Group `$export` when `serves_bulk_export`, and Patient
/// `$everything` when `supports_search`, since `$everything` is composed over
/// `search` and answers `501` where the storage cannot search. A type left with
/// no operation carries no `operation` element.
#[allow(clippy::too_many_arguments)]
fn build_resource_capability(
    resource_type: &str,
    conditionals: &serde_json::Map<String, serde_json::Value>,
    registry: &SearchParameterRegistry,
    supports_search: bool,
    supports_contained: bool,
    serves_bulk_export: bool,
    modifier_map: &std::collections::HashMap<SearchParamType, Vec<&'static str>>,
    revinclude_by_target: &std::collections::HashMap<String, std::collections::BTreeSet<String>>,
    serves: &dyn Fn(&str, &str, SearchParamType) -> bool,
    version: FhirVersion,
) -> serde_json::Value {
    let mut interactions = if resource_type == "AuditEvent" {
        serde_json::json!([
            { "code": "read" },
            { "code": "vread" },
            { "code": "history-instance" },
            { "code": "history-type" }
        ])
    } else {
        serde_json::json!([
            { "code": "read" },
            { "code": "vread" },
            { "code": "update" },
            { "code": "patch" },
            { "code": "delete" },
            { "code": "history-instance" },
            { "code": "history-type" },
            { "code": "create" }
        ])
    };
    if supports_search && let Some(list) = interactions.as_array_mut() {
        list.push(serde_json::json!({ "code": "search-type" }));
    }

    let mut entry = serde_json::json!({
        "type": resource_type,
        "profile": format!("http://hl7.org/fhir/StructureDefinition/{}", resource_type),
        "interaction": interactions,
        "versioning": "versioned",
        "readHistory": true,
        "conditionalRead": "full-support"
    });

    if supports_search {
        entry["searchParam"] = serde_json::Value::Array(build_search_params(
            resource_type,
            registry,
            supports_contained,
            modifier_map,
            serves,
            version,
        ));
    }

    if resource_type != "AuditEvent" {
        entry["updateCreate"] = serde_json::Value::Bool(true);
        for (element, value) in conditionals {
            entry[element.as_str()] = value.clone();
        }
    }

    // Advertise real `_include` targets: one "Type:code" per reference param on
    // this type. Omitted entirely when the type has no reference params, and
    // when the storage does not search (#514).
    if supports_search {
        let search_include = build_search_includes(resource_type, registry, serves);
        if !search_include.is_empty() {
            entry["searchInclude"] = serde_json::Value::Array(search_include);
        }
    }

    // Advertise real `_revinclude` targets: "Source:code" tokens from other
    // types' reference params that point at this type. Omitted when the
    // storage does not search (#514).
    if supports_search && let Some(rev) = revinclude_by_target.get(resource_type) {
        if !rev.is_empty() {
            entry["searchRevInclude"] = serde_json::Value::Array(
                rev.iter().cloned().map(serde_json::Value::String).collect(),
            );
        }
    }
    // Bulk Data Access IG: per-resource `$export` operation entries on Patient
    // and Group, in addition to the system-level `$export` advertised at
    // `rest[0].operation`, and Patient `$everything`. Each is listed only
    // where it is served (#3015).
    let mut operations = Vec::new();
    match resource_type {
        "Patient" => {
            if serves_bulk_export {
                operations.push(serde_json::json!({
                    "name": "export",
                    "definition": "http://hl7.org/fhir/uv/bulkdata/OperationDefinition/patient-export"
                }));
            }
            if supports_search {
                operations.push(serde_json::json!({
                    "name": "everything",
                    "definition": "http://hl7.org/fhir/OperationDefinition/Patient-everything"
                }));
            }
        }
        "Group" if serves_bulk_export => {
            operations.push(serde_json::json!({
                "name": "export",
                "definition": "http://hl7.org/fhir/uv/bulkdata/OperationDefinition/group-export"
            }));
        }
        _ => {}
    }
    if !operations.is_empty() {
        entry["operation"] = serde_json::Value::Array(operations);
    }
    entry
}

/// Builds the full `searchParam` list for a resource type: the seven common
/// params plus the resource-specific params from the loaded SearchParameter
/// registry. Control/common params (those starting with `_`) and duplicates are
/// skipped, and so is every parameter `serves` (the storage's
/// [`SearchProvider::serves_search_param`]) refuses, common ones included.
/// A composite is also skipped unless each of its components resolves and is
/// served ([`composite_components_served`]).
///
/// A common parameter is asked about with the type `/metadata` advertises for
/// it, except `_list` and `_in`: REST resolves both into an `_id` filter before
/// the storage is called (reading the List or Group itself), so they are asked
/// about as what the storage actually receives, `_id` of type token.
fn build_search_params(
    resource_type: &str,
    registry: &SearchParameterRegistry,
    supports_contained: bool,
    modifier_map: &std::collections::HashMap<SearchParamType, Vec<&'static str>>,
    serves: &dyn Fn(&str, &str, SearchParamType) -> bool,
    version: FhirVersion,
) -> Vec<serde_json::Value> {
    let mut params = build_common_search_params(supports_contained, version);
    params.retain(|param| {
        let name = param["name"].as_str().unwrap_or_default();
        if matches!(name, "_list" | "_in") {
            return serves(resource_type, "_id", SearchParamType::Token);
        }
        param["type"]
            .as_str()
            .and_then(|code| code.parse::<SearchParamType>().ok())
            .is_some_and(|param_type| serves(resource_type, name, param_type))
    });
    let mut seen: std::collections::HashSet<String> = params
        .iter()
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .collect();

    for def in registry.get_active_params(resource_type) {
        if def.code.starts_with('_')
            || !serves(resource_type, &def.code, def.param_type)
            || (def.param_type == SearchParamType::Composite
                && !composite_components_served(resource_type, &def, registry, serves))
            || !seen.insert(def.code.clone())
        {
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

/// Whether the composite `def` can be searched: it names at least one
/// component, each component resolves in `registry` by its definition URL (as
/// the search query builder resolves them), and `serves` serves each one.
///
/// A composite is searched through its components, so a component the storage
/// refuses makes every search for the composite fail (MongoDB refuses
/// `Composition.section-code-text`, whose `section-text` component is
/// `special`). A component that does not resolve leaves the composite with
/// fewer components than it defines, which the backends refuse or match
/// differently from the spec (some R5 and R6 composites name component
/// definitions the spec bundles do not publish).
fn composite_components_served(
    resource_type: &str,
    def: &SearchParameterDefinition,
    registry: &SearchParameterRegistry,
    serves: &dyn Fn(&str, &str, SearchParamType) -> bool,
) -> bool {
    def.component.as_deref().is_some_and(|components| {
        !components.is_empty()
            && components.iter().all(|component| {
                registry
                    .get_by_url(&component.definition)
                    .is_some_and(|sub| serves(resource_type, &sub.code, sub.param_type))
            })
    })
}

/// Builds the reverse-include index used for `searchRevInclude`: maps each target
/// resource type to the sorted set of "SourceType:code" tokens from every active
/// reference search param that targets it and that `serves` serves.
fn build_revinclude_index(
    registry: &SearchParameterRegistry,
    serves: &dyn Fn(&str, &str, SearchParamType) -> bool,
) -> std::collections::HashMap<String, std::collections::BTreeSet<String>> {
    let mut index: std::collections::HashMap<String, std::collections::BTreeSet<String>> =
        std::collections::HashMap::new();
    for source_type in registry.resource_types() {
        for def in registry.get_active_params(&source_type) {
            if def.param_type != SearchParamType::Reference
                || def.code.starts_with('_')
                || !serves(&source_type, &def.code, def.param_type)
            {
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
/// active reference search param defined on it that `serves` serves.
fn build_search_includes(
    resource_type: &str,
    registry: &SearchParameterRegistry,
    serves: &dyn Fn(&str, &str, SearchParamType) -> bool,
) -> Vec<serde_json::Value> {
    let mut includes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for def in registry.get_active_params(resource_type) {
        if def.param_type == SearchParamType::Reference
            && !def.code.starts_with('_')
            && serves(resource_type, &def.code, def.param_type)
        {
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
/// is set (the backend can evaluate contained search). `_list` is resolved
/// application-side into an `_id` filter, so [`build_search_params`] keeps it
/// wherever the storage serves `_id`. `_in` (List/Group membership) exists only
/// in R5 and later, so it is listed only there; it is likewise resolved into an
/// `_id` filter and kept on the same terms.
fn build_common_search_params(
    supports_contained: bool,
    version: FhirVersion,
) -> Vec<serde_json::Value> {
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

    if matches!(version.as_mime_param(), "5.0" | "6.0") {
        params.push(serde_json::json!({
            "name": "_in",
            "type": "reference",
            "documentation": "Resources that are active members of the given List or Group"
        }));
    }

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

    /// `SearchProvider::serves_search_param`'s default.
    fn serves_all(_: &str, _: &str, _: SearchParamType) -> bool {
        true
    }

    fn param_names(params: &[serde_json::Value]) -> Vec<String> {
        params
            .iter()
            .filter_map(|p| p["name"].as_str().map(str::to_string))
            .collect()
    }

    /// `$reindex` is advertised exactly where it can be served. A deployment
    /// with no index to rebuild answers 501 from the handler, so advertising it
    /// there would promise an operation that does not work; a deployment that
    /// has one had no discoverable way to learn it exists at all (#1021).
    #[cfg(feature = "sqlite")]
    #[test]
    fn reindex_is_advertised_only_where_an_index_can_be_rebuilt() {
        use crate::config::ServerConfig;
        use crate::handlers::sof::capability::REINDEX_DEFINITION_ID;
        use helios_persistence::backends::sqlite::SqliteBackend;
        use helios_persistence::search::{ReindexOperation, TenantSearchRegistries};
        use std::sync::Arc;

        let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite"));
        backend.init_schema().expect("init schema");

        let without = AppState::new(backend.clone(), ServerConfig::default());
        assert!(
            !operation_names(&build_rest_operations(&without)).contains(&"reindex".to_string()),
            "an unwired deployment must not advertise an operation its handler answers 501 to"
        );

        let registries = Arc::new(TenantSearchRegistries::base_only());
        let with = AppState::new(backend.clone(), ServerConfig::default())
            .with_reindex(Arc::new(ReindexOperation::new(backend, registries)));
        let names = operation_names(&build_rest_operations(&with));
        assert!(
            names.contains(&"reindex".to_string()),
            "a deployment with an index must advertise the rebuild, got {names:?}"
        );

        // The cited definition is the one this server actually serves.
        let reindex = build_rest_operations(&with)
            .into_iter()
            .find(|o| o["name"] == "reindex")
            .expect("just asserted present");
        assert_eq!(
            reindex["definition"],
            format!("/OperationDefinition/{REINDEX_DEFINITION_ID}")
        );
    }

    /// `$sql-run` is advertised exactly where a SOF runner is wired: without
    /// one (`HFS_SOF_ENABLED=false`) the handler answers 501 (#3015).
    #[cfg(feature = "sqlite")]
    #[test]
    fn sql_run_is_advertised_only_where_a_sof_runner_is_wired() {
        use crate::config::ServerConfig;
        use crate::handlers::sof::capability::SQL_RUN_DEFINITION_ID;
        use helios_persistence::backends::sqlite::SqliteBackend;
        use std::sync::Arc;

        let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite"));
        backend.init_schema().expect("init schema");

        let without = AppState::new(backend.clone(), ServerConfig::default());
        assert!(
            !operation_names(&build_rest_operations(&without)).contains(&"sql-run".to_string()),
            "a deployment with no SOF runner must not advertise $sql-run"
        );

        let runner = backend
            .sof_runner()
            .expect("SQLite has an in-DB SOF runner");
        let with = AppState::new(backend, ServerConfig::default()).with_sof_runner(runner);
        let sql_run = build_rest_operations(&with)
            .into_iter()
            .find(|o| o["name"] == "sql-run")
            .expect("a deployment with a SOF runner advertises $sql-run");
        assert_eq!(
            sql_run["definition"],
            format!("/OperationDefinition/{SQL_RUN_DEFINITION_ID}")
        );
    }

    /// `CapabilityStatement.software` identifies the build for every FHIR
    /// version the server can describe: the same binary answers `/metadata`
    /// for each of them, so the version must not depend on which one was
    /// asked for (#992).
    #[cfg(feature = "sqlite")]
    #[test]
    fn software_names_the_build_for_every_enabled_fhir_version() {
        use crate::build_info::{PKG_VERSION, SOFTWARE_NAME, git_sha};
        use crate::config::ServerConfig;
        use helios_persistence::backends::sqlite::SqliteBackend;
        use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
        use std::sync::Arc;

        let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite"));
        backend.init_schema().expect("init schema");
        let state = AppState::new(backend, ServerConfig::default());
        let tenant = TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        );

        assert!(!FhirVersion::enabled_versions().is_empty());
        for version in FhirVersion::enabled_versions() {
            let statement =
                build_capability_statement(&state, &tenant, *version, "http://localhost:8080");
            let software = &statement["software"];
            assert_eq!(
                software["name"], SOFTWARE_NAME,
                "software.name for {version:?}"
            );
            assert_eq!(
                software["version"], PKG_VERSION,
                "software.version must be the crate version for {version:?}"
            );
            assert_eq!(
                software["extension"][0]["valueString"].as_str(),
                git_sha(),
                "git sha rides as an extension exactly when the build knows it"
            );
            assert_eq!(statement["implementation"]["description"], SOFTWARE_NAME);
            assert_eq!(statement["fhirVersion"], version.full_version());
        }
    }

    fn operation_names(ops: &[serde_json::Value]) -> Vec<String> {
        ops.iter()
            .filter_map(|o| o["name"].as_str().map(str::to_string))
            .collect()
    }

    #[test]
    fn common_params_always_advertise_list() {
        let names = param_names(&build_common_search_params(
            false,
            FhirVersion::default_enabled(),
        ));
        assert!(names.contains(&"_list".to_string()));
        assert!(names.contains(&"_text".to_string()));
    }

    #[test]
    fn membership_param_advertised_only_on_r5_and_later() {
        for version in FhirVersion::enabled_versions() {
            let names = param_names(&build_common_search_params(false, *version));
            let expected = matches!(version.as_mime_param(), "5.0" | "6.0");
            assert_eq!(
                names.contains(&"_in".to_string()),
                expected,
                "_in advertisement for {version:?}"
            );
        }
    }

    #[test]
    fn contained_params_gated_on_capability() {
        let without = param_names(&build_common_search_params(
            false,
            FhirVersion::default_enabled(),
        ));
        assert!(!without.contains(&"_contained".to_string()));
        assert!(!without.contains(&"_containedType".to_string()));

        let with = param_names(&build_common_search_params(
            true,
            FhirVersion::default_enabled(),
        ));
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
        let includes: Vec<String> = build_search_includes("Patient", &reg, &serves_all)
            .into_iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(includes, vec!["Patient:general-practitioner".to_string()]);

        // searchRevInclude index: each target type maps to the "Source:code"
        // tokens that point at it.
        let rev = build_revinclude_index(&reg, &serves_all);
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

        let params = build_search_params(
            "Patient",
            &reg,
            false,
            &modifier_map,
            &serves_all,
            FhirVersion::default_enabled(),
        );
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

    /// The `rest.resource` entry of `resource_type`, built over a registry that
    /// gives `Patient` a search parameter (`name`), an `_include` target
    /// (`Patient:general-practitioner`) and a `_revinclude` source
    /// (`Observation:subject`).
    fn resource_entry(resource_type: &str, supports_search: bool) -> serde_json::Value {
        resource_entry_serving(resource_type, supports_search, true, &serves_all)
    }

    /// [`resource_entry`], with whether Bulk Data export is served and the
    /// storage's `serves_search_param` too.
    fn resource_entry_serving(
        resource_type: &str,
        supports_search: bool,
        serves_bulk_export: bool,
        serves: &dyn Fn(&str, &str, SearchParamType) -> bool,
    ) -> serde_json::Value {
        use helios_persistence::search::{SearchParameterDefinition, SearchParameterRegistry};

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
        reg.register(ref_param(
            "http://example.org/Patient-general-practitioner",
            "general-practitioner",
            "Patient",
            &["Practitioner"],
        ))
        .unwrap();
        reg.register(ref_param(
            "http://example.org/Observation-subject",
            "subject",
            "Observation",
            &["Patient"],
        ))
        .unwrap();

        let mut conditionals = serde_json::Map::new();
        conditionals.insert("conditionalCreate".to_string(), serde_json::json!(true));

        build_resource_capability(
            resource_type,
            &conditionals,
            &reg,
            supports_search,
            false,
            serves_bulk_export,
            &std::collections::HashMap::new(),
            &build_revinclude_index(&reg, serves),
            serves,
            FhirVersion::default_enabled(),
        )
    }

    fn interaction_codes(entry: &serde_json::Value) -> Vec<&str> {
        entry["interaction"]
            .as_array()
            .expect("interaction array")
            .iter()
            .filter_map(|i| i["code"].as_str())
            .collect()
    }

    /// With a storage that searches, the entry is what it always was:
    /// `search-type` last in the interaction list, and the three search
    /// blocks.
    #[test]
    fn search_is_advertised_when_the_storage_searches() {
        let patient = resource_entry("Patient", true);
        assert_eq!(
            interaction_codes(&patient),
            [
                "read",
                "vread",
                "update",
                "patch",
                "delete",
                "history-instance",
                "history-type",
                "create",
                "search-type"
            ]
        );
        assert!(
            param_names(patient["searchParam"].as_array().expect("searchParam"))
                .contains(&"name".to_string())
        );
        assert_eq!(
            patient["searchInclude"],
            serde_json::json!(["Patient:general-practitioner"])
        );
        assert_eq!(
            patient["searchRevInclude"],
            serde_json::json!(["Observation:subject"])
        );

        let audit = resource_entry("AuditEvent", true);
        assert_eq!(
            interaction_codes(&audit),
            [
                "read",
                "vread",
                "history-instance",
                "history-type",
                "search-type"
            ]
        );
        assert!(audit["searchParam"].is_array());
    }

    /// With a storage that does not search (standalone S3), no resource type
    /// — the `AuditEvent` branch included — lists `search-type` or any of the
    /// three search blocks. Every other element is untouched (#514).
    #[test]
    fn no_search_is_advertised_when_the_storage_does_not_search() {
        for resource_type in ["Patient", "Observation", "Practitioner", "AuditEvent"] {
            let entry = resource_entry(resource_type, false);
            assert!(
                !interaction_codes(&entry).contains(&"search-type"),
                "{resource_type}: {entry}"
            );
            for key in ["searchParam", "searchInclude", "searchRevInclude"] {
                assert!(
                    entry.get(key).is_none(),
                    "{resource_type}: {key} in {entry}"
                );
            }
            assert_eq!(entry["type"], resource_type);
            assert_eq!(entry["versioning"], "versioned");
            assert_eq!(entry["readHistory"], true);
            assert_eq!(entry["conditionalRead"], "full-support");
        }

        let patient = resource_entry("Patient", false);
        assert_eq!(
            interaction_codes(&patient),
            [
                "read",
                "vread",
                "update",
                "patch",
                "delete",
                "history-instance",
                "history-type",
                "create"
            ]
        );
        assert_eq!(patient["updateCreate"], true);
        assert_eq!(patient["conditionalCreate"], true);
        assert_eq!(
            resource_operation_names(&patient),
            ["export"],
            "Patient keeps `$export` and loses `$everything`"
        );

        assert_eq!(
            interaction_codes(&resource_entry("AuditEvent", false)),
            ["read", "vread", "history-instance", "history-type"]
        );
    }

    fn resource_operation_names(entry: &serde_json::Value) -> Vec<&str> {
        entry["operation"]
            .as_array()
            .map(|ops| ops.iter().filter_map(|o| o["name"].as_str()).collect())
            .unwrap_or_default()
    }

    /// Patient and Group `$export` are listed exactly when Bulk Data export is
    /// served, and Patient `$everything` exactly when the storage searches. A
    /// type left with no operation carries no `operation` element, rather than
    /// an empty array (#3015).
    #[test]
    fn resource_operations_follow_what_is_served() {
        for (export, search, patient, group) in [
            (true, true, vec!["export", "everything"], vec!["export"]),
            (false, true, vec!["everything"], vec![]),
            (true, false, vec!["export"], vec!["export"]),
            (false, false, vec![], vec![]),
        ] {
            let case = format!("export={export} search={search}");
            let patient_entry = resource_entry_serving("Patient", search, export, &serves_all);
            let group_entry = resource_entry_serving("Group", search, export, &serves_all);
            assert_eq!(resource_operation_names(&patient_entry), patient, "{case}");
            assert_eq!(resource_operation_names(&group_entry), group, "{case}");
            for entry in [&patient_entry, &group_entry] {
                if resource_operation_names(entry).is_empty() {
                    assert!(entry.get("operation").is_none(), "{case}: {entry}");
                }
            }
        }
        let observation = resource_entry_serving("Observation", true, true, &serves_all);
        assert!(observation.get("operation").is_none(), "{observation}");
    }

    /// Every common parameter's `type` code maps back to a type, so none is
    /// dropped for want of one.
    #[test]
    fn every_common_param_type_is_known() {
        for version in FhirVersion::enabled_versions() {
            for param in build_common_search_params(true, *version) {
                let code = param["type"].as_str().expect("type");
                let param_type: SearchParamType = code.parse().expect(code);
                assert_eq!(fhir_search_param_type(param_type), code);
            }
        }
    }

    /// `serves` filters `searchParam`, common parameters included, and the
    /// include lists: a storage that serves only token, date and URI keeps
    /// `_id` (token), `_lastUpdated` (date) and `_profile` (uri), keeps `_list`,
    /// which REST turns into an `_id` filter, drops `name` (string) and `_text`
    /// and `_content` (special), and lists no include, because every include
    /// names a reference parameter.
    #[test]
    fn unserved_params_and_their_includes_are_left_out() {
        fn token_date_uri(_: &str, _: &str, t: SearchParamType) -> bool {
            matches!(
                t,
                SearchParamType::Token | SearchParamType::Date | SearchParamType::Uri
            )
        }
        let patient = resource_entry_serving("Patient", true, true, &token_date_uri);
        let names = param_names(patient["searchParam"].as_array().expect("searchParam"));
        for kept in [
            "_id",
            "_lastUpdated",
            "_tag",
            "_profile",
            "_security",
            "_list",
        ] {
            assert!(names.contains(&kept.to_string()), "{kept}: {names:?}");
        }
        for dropped in ["name", "_text", "_content"] {
            assert!(
                !names.contains(&dropped.to_string()),
                "{dropped}: {names:?}"
            );
        }
        assert!(patient.get("searchInclude").is_none(), "{patient}");
        assert!(patient.get("searchRevInclude").is_none(), "{patient}");
        assert!(interaction_codes(&patient).contains(&"search-type"));

        // One reference parameter refused: its include and reverse include go,
        // the rest stays.
        fn not_subject(_: &str, code: &str, _: SearchParamType) -> bool {
            code != "subject"
        }
        let patient = resource_entry_serving("Patient", true, true, &not_subject);
        assert_eq!(
            patient["searchInclude"],
            serde_json::json!(["Patient:general-practitioner"])
        );
        assert!(patient.get("searchRevInclude").is_none(), "{patient}");
        assert!(
            param_names(patient["searchParam"].as_array().expect("searchParam"))
                .contains(&"name".to_string())
        );

        // `_list` reaches the storage as `_id`, so it goes with `_id`.
        fn not_id(_: &str, code: &str, _: SearchParamType) -> bool {
            code != "_id"
        }
        let patient = resource_entry_serving("Patient", true, true, &not_id);
        let names = param_names(patient["searchParam"].as_array().expect("searchParam"));
        for dropped in ["_id", "_list"] {
            assert!(
                !names.contains(&dropped.to_string()),
                "{dropped}: {names:?}"
            );
        }
        assert!(names.contains(&"_lastUpdated".to_string()), "{names:?}");
    }

    /// A composite is listed only when it names components, each resolves in
    /// the registry by its definition URL, and each is served: a composite with
    /// a component the storage refuses, or one the registry does not hold, is
    /// refused or matched on fewer components than it defines.
    #[test]
    fn composites_are_listed_only_when_every_component_is_served() {
        use helios_persistence::search::registry::CompositeComponentDef;

        let mut reg = SearchParameterRegistry::new();
        for (url, code, param_type) in [
            (
                "http://example.org/Obs-code",
                "code",
                SearchParamType::Token,
            ),
            (
                "http://example.org/Obs-value",
                "value",
                SearchParamType::Quantity,
            ),
            (
                "http://example.org/Obs-text",
                "text",
                SearchParamType::Special,
            ),
        ] {
            reg.register(
                SearchParameterDefinition::new(url, code, param_type, "Observation.id")
                    .with_base(["Observation"]),
            )
            .unwrap();
        }
        for (code, components) in [
            ("code-value", vec!["Obs-code", "Obs-value"]),
            ("code-text", vec!["Obs-code", "Obs-text"]),
            ("code-missing", vec!["Obs-code", "Obs-unpublished"]),
            ("no-components", vec![]),
        ] {
            let mut composite = SearchParameterDefinition::new(
                format!("http://example.org/Obs-{code}"),
                code,
                SearchParamType::Composite,
                "Observation",
            )
            .with_base(["Observation"]);
            composite.component = Some(
                components
                    .into_iter()
                    .map(|c| CompositeComponentDef {
                        definition: format!("http://example.org/{c}"),
                        expression: "Observation.id".to_string(),
                    })
                    .collect(),
            );
            reg.register(composite).unwrap();
        }
        let listed = |serves: &dyn Fn(&str, &str, SearchParamType) -> bool| {
            let params = build_search_params(
                "Observation",
                &reg,
                false,
                &std::collections::HashMap::new(),
                serves,
                FhirVersion::default(),
            );
            param_names(&params)
                .into_iter()
                .filter(|name| !name.starts_with('_'))
                .collect::<std::collections::BTreeSet<_>>()
        };
        let all = |_: &str, _: &str, _: SearchParamType| true;
        assert_eq!(
            listed(&all),
            ["code", "code-text", "code-value", "text", "value"]
                .map(String::from)
                .into()
        );
        let no_special = |_: &str, _: &str, t: SearchParamType| t != SearchParamType::Special;
        assert_eq!(
            listed(&no_special),
            ["code", "code-value", "value"].map(String::from).into()
        );
        let no_quantity = |_: &str, _: &str, t: SearchParamType| t != SearchParamType::Quantity;
        assert_eq!(
            listed(&no_quantity),
            ["code", "code-text", "text"].map(String::from).into()
        );
    }

    /// A storage that searches yields the same statement for the whole server:
    /// every resource entry lists `search-type` and a non-empty `searchParam`.
    #[cfg(feature = "sqlite")]
    #[test]
    fn statement_over_a_searching_storage_lists_search_on_every_type() {
        use crate::config::ServerConfig;
        use helios_persistence::backends::sqlite::SqliteBackend;
        use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
        use std::sync::Arc;

        let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite"));
        backend.init_schema().expect("init schema");
        assert!(backend.supports_search());
        let state = AppState::new(backend, ServerConfig::default());
        let tenant = TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        );

        let statement = build_capability_statement(
            &state,
            &tenant,
            FhirVersion::default_enabled(),
            "http://localhost:8080",
        );
        let resources = statement["rest"][0]["resource"]
            .as_array()
            .expect("resources");
        assert!(!resources.is_empty());
        for resource in resources {
            assert!(
                interaction_codes(resource).contains(&"search-type"),
                "{}",
                resource["type"]
            );
            assert!(
                !resource["searchParam"]
                    .as_array()
                    .expect("searchParam")
                    .is_empty(),
                "{}",
                resource["type"]
            );
        }
    }
}
