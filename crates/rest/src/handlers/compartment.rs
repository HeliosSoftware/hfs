//! Compartment search handler.
//!
//! Implements FHIR [compartment search](https://hl7.org/fhir/compartmentdefinition.html):
//! `GET [base]/[compartment-type]/[id]/[resource-type]?params`
//!
//! Compartment search allows finding all resources related to a specific resource,
//! such as all Observations for a specific Patient.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirVersionExtractor, TenantExtractor, build_search_query_from_map};
use crate::state::AppState;

/// Returns compartment search parameters for a specific FHIR version.
///
/// This function dispatches to the version-specific generated compartment lookup
/// functions in the helios_fhir crate. The compartment definitions are generated
/// from the official FHIR CompartmentDefinition resources.
///
/// # Arguments
///
/// * `version` - The FHIR version to use for lookup
/// * `compartment_type` - The compartment type (e.g., "Patient", "Encounter")
/// * `resource_type` - The target resource type (e.g., "Observation")
///
/// # Returns
///
/// A static slice of search parameter names that link the resource to the compartment.
/// Returns an empty slice if the resource is not a member of the compartment.
fn get_compartment_params_for_version(
    version: FhirVersion,
    compartment_type: &str,
    resource_type: &str,
) -> &'static [&'static str] {
    match version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => helios_fhir::r4::get_compartment_params(compartment_type, resource_type),
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => {
            helios_fhir::r4b::get_compartment_params(compartment_type, resource_type)
        }
        #[cfg(feature = "R5")]
        FhirVersion::R5 => helios_fhir::r5::get_compartment_params(compartment_type, resource_type),
        #[cfg(feature = "R6")]
        FhirVersion::R6 => helios_fhir::r6::get_compartment_params(compartment_type, resource_type),
    }
}

/// Handler for compartment search.
///
/// Searches for resources within a specific compartment.
///
/// # HTTP Request
///
/// `GET [base]/[compartment-type]/[id]/[resource-type]?params`
///
/// # Examples
///
/// - `GET /Patient/123/Observation?code=8867-4` - Observations for patient 123
/// - `GET /Patient/123/Condition` - All conditions for patient 123
/// - `GET /Encounter/456/Procedure` - Procedures for encounter 456
///
/// # Response
///
/// Returns a Bundle of type "searchset" containing matching resources.
pub async fn compartment_search_handler<S>(
    State(state): State<AppState<S>>,
    Path((compartment_type, compartment_id, target_type)): Path<(String, String, String)>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    Query(mut params): Query<HashMap<String, String>>,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    debug!(
        compartment_type = %compartment_type,
        compartment_id = %compartment_id,
        target_type = %target_type,
        tenant = %tenant.tenant_id(),
        params = ?params,
        "Processing compartment search request"
    );

    // Get the reference parameters for this compartment/target combination
    let fhir_version = version.storage_version();
    let ref_params =
        get_compartment_params_for_version(fhir_version, &compartment_type, &target_type);

    // Check if the resource type is a member of the compartment
    if ref_params.is_empty() {
        return Err(RestError::BadRequest {
            message: format!(
                "Resource type '{}' is not a member of the '{}' compartment",
                target_type, compartment_type
            ),
        });
    }

    // Build the compartment reference
    let compartment_ref = format!("{}/{}", compartment_type, compartment_id);

    // Add the first compartment reference parameter to the search parameters
    // (the first parameter is typically the most specific one)
    params.insert(ref_params[0].to_string(), compartment_ref);

    // Apply pagination limits
    apply_pagination_limits(
        &mut params,
        state.default_page_size(),
        state.max_page_size(),
    );

    // Convert REST params to persistence SearchQuery
    let query = build_search_query_from_map(&target_type, &params)?;

    // Execute the search
    let result = state
        .storage()
        .search(tenant.context(), &query)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "Compartment search failed");
            RestError::from(e)
        })?;

    // Build the self link URL
    let self_link = build_compartment_search_url(
        state.base_url(),
        &compartment_type,
        &compartment_id,
        &target_type,
        &params,
    );

    // Convert result to FHIR Bundle
    let bundle = result.to_bundle(state.base_url(), &self_link);

    debug!(
        compartment_type = %compartment_type,
        compartment_id = %compartment_id,
        target_type = %target_type,
        results = result.resources.len(),
        "Compartment search completed"
    );

    Ok((StatusCode::OK, Json(bundle_to_json(bundle))).into_response())
}

/// Handler for compartment search across all types.
///
/// Returns all resources in a compartment.
///
/// # HTTP Request
///
/// `GET [base]/[compartment-type]/[id]/*`
///
/// Note: This is a less common operation and returns resources of various types.
#[allow(dead_code)]
pub async fn compartment_search_all_handler<S>(
    State(_state): State<AppState<S>>,
    Path((compartment_type, compartment_id)): Path<(String, String)>,
    _tenant: TenantExtractor,
    Query(_params): Query<HashMap<String, String>>,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    debug!(
        compartment_type = %compartment_type,
        compartment_id = %compartment_id,
        "Processing compartment search all request"
    );

    // For now, return an error - full implementation would search multiple types
    // and combine results
    Err(RestError::BadRequest {
        message: format!(
            "Searching all types in compartment '{}' is not yet implemented. \
             Please specify a resource type: GET /{}/{}/[type]",
            compartment_type, compartment_type, compartment_id
        ),
    })
}

/// Applies pagination limits from configuration to the params.
fn apply_pagination_limits(
    params: &mut HashMap<String, String>,
    default_page_size: usize,
    max_page_size: usize,
) {
    let count = params
        .get("_count")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default_page_size)
        .min(max_page_size);

    params.insert("_count".to_string(), count.to_string());
}

/// Builds a compartment search URL.
fn build_compartment_search_url(
    base_url: &str,
    compartment_type: &str,
    compartment_id: &str,
    target_type: &str,
    params: &HashMap<String, String>,
) -> String {
    let path = format!(
        "{}/{}/{}/{}",
        base_url, compartment_type, compartment_id, target_type
    );

    if params.is_empty() {
        path
    } else {
        let query: String = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        format!("{}?{}", path, query)
    }
}

/// Converts a SearchBundle to a serde_json::Value for response.
fn bundle_to_json(bundle: helios_persistence::types::SearchBundle) -> serde_json::Value {
    serde_json::json!({
        "resourceType": "Bundle",
        "type": bundle.bundle_type,
        "total": bundle.total,
        "link": bundle.link.iter().map(|l| {
            serde_json::json!({
                "relation": l.relation,
                "url": l.url
            })
        }).collect::<Vec<_>>(),
        "entry": bundle.entry.iter().map(|e| {
            let mut entry = serde_json::json!({});
            if let Some(ref full_url) = e.full_url {
                entry["fullUrl"] = serde_json::Value::String(full_url.clone());
            }
            if let Some(ref resource) = e.resource {
                entry["resource"] = resource.clone();
            }
            if let Some(ref search) = e.search {
                entry["search"] = serde_json::json!({
                    "mode": match search.mode {
                        helios_persistence::types::SearchEntryMode::Match => "match",
                        helios_persistence::types::SearchEntryMode::Include => "include",
                        helios_persistence::types::SearchEntryMode::Outcome => "outcome",
                    }
                });
            }
            entry
        }).collect::<Vec<_>>()
    })
}

// URL encoding helper
mod urlencoding {
    pub fn encode(s: &str) -> String {
        url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_compartment_params_patient_observation() {
        // Test that Patient compartment includes Observation with subject and performer params
        let params =
            get_compartment_params_for_version(FhirVersion::default(), "Patient", "Observation");
        assert!(!params.is_empty());
        assert!(params.contains(&"subject"));
    }

    #[test]
    fn test_get_compartment_params_patient_immunization() {
        // Test that Patient compartment includes Immunization with patient param
        let params =
            get_compartment_params_for_version(FhirVersion::default(), "Patient", "Immunization");
        assert!(!params.is_empty());
        assert!(params.contains(&"patient"));
    }

    #[test]
    fn test_get_compartment_params_encounter_procedure() {
        // Test that Encounter compartment includes Procedure with encounter param
        let params =
            get_compartment_params_for_version(FhirVersion::default(), "Encounter", "Procedure");
        assert!(!params.is_empty());
        assert!(params.contains(&"encounter"));
    }

    #[test]
    fn test_get_compartment_params_unknown() {
        // Test that unknown resource types return an empty slice
        let params =
            get_compartment_params_for_version(FhirVersion::default(), "Patient", "UnknownType");
        assert!(params.is_empty());
    }

    #[test]
    fn test_get_compartment_params_multiple() {
        // Test that some resources have multiple compartment params
        // AllergyIntolerance in Patient compartment has: patient, recorder, asserter
        let params = get_compartment_params_for_version(
            FhirVersion::default(),
            "Patient",
            "AllergyIntolerance",
        );
        assert!(
            params.len() >= 2,
            "Expected multiple params for AllergyIntolerance"
        );
        assert!(params.contains(&"patient"));
    }

    #[test]
    fn test_build_compartment_search_url_no_params() {
        let url = build_compartment_search_url(
            "http://example.com/fhir",
            "Patient",
            "123",
            "Observation",
            &HashMap::new(),
        );
        assert_eq!(url, "http://example.com/fhir/Patient/123/Observation");
    }

    #[test]
    fn test_build_compartment_search_url_with_params() {
        let mut params = HashMap::new();
        params.insert("code".to_string(), "8867-4".to_string());

        let url = build_compartment_search_url(
            "http://example.com/fhir",
            "Patient",
            "123",
            "Observation",
            &params,
        );

        assert!(url.starts_with("http://example.com/fhir/Patient/123/Observation?"));
        assert!(url.contains("code=8867-4"));
    }
}
