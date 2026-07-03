//! Compartment search handler.
//!
//! Implements FHIR [compartment search](https://hl7.org/fhir/compartmentdefinition.html):
//! `GET [base]/[compartment-type]/[id]/[resource-type]?params`
//!
//! Compartment search allows finding all resources related to a specific resource,
//! such as all Observations for a specific Patient.

use axum::{
    Json,
    extract::{Path, RawQuery, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_persistence::core::{ResourceStorage, SearchProvider};
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::extractors::query_pairs::parse_query_pairs;
use crate::extractors::{FhirVersionExtractor, SearchParams, TenantExtractor, build_search_query};
use crate::state::AppState;

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
    RawQuery(raw_query): RawQuery,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let pairs = parse_query_pairs(raw_query.as_deref());
    debug!(
        compartment_type = %compartment_type,
        compartment_id = %compartment_id,
        target_type = %target_type,
        tenant = %tenant.tenant_id(),
        params = ?pairs,
        "Processing compartment search request"
    );

    // `GET [base]/[compartment]/[id]/*` searches every resource type that is a
    // member of the compartment, rather than a single target type.
    if target_type == "*" {
        return compartment_search_all(
            state,
            compartment_type,
            compartment_id,
            tenant,
            version,
            pairs,
        )
        .await;
    }

    // Get the reference parameters for this compartment/target combination
    let fhir_version = version.storage_version();
    let ref_params =
        helios_fhir::get_compartment_params(fhir_version, &compartment_type, &target_type);

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

    let search_params = SearchParams::from_pairs(pairs);

    // Convert REST params to persistence SearchQuery. Scope the registry read
    // guard tightly so it doesn't span any await.
    let mut query = {
        let registry = state.storage().search_param_registry().read();
        build_search_query(&target_type, &search_params, &registry)?
    };

    // Restrict to compartment members. A resource joins a compartment if it
    // references the compartment through ANY of the membership reference params
    // (e.g. AllergyIntolerance via `patient`, `recorder`, OR `asserter`), so we
    // pass all of them — not just the first — and the backend ORs them. This
    // avoids the under-inclusive "first param only" behaviour.
    query.compartment = Some(helios_persistence::types::CompartmentMembership {
        params: ref_params.iter().map(|s| s.to_string()).collect(),
        reference: compartment_ref,
    });

    // Clamp page size to the configured default/maximum.
    let count = query
        .count
        .map(|c| c as usize)
        .unwrap_or(state.default_page_size())
        .min(state.max_page_size());
    query.count = Some(count as u32);

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
        &search_params,
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

/// Searches across all resource types in a compartment.
///
/// Implements `GET [base]/[compartment-type]/[id]/*`: returns every resource
/// that is a member of the compartment, regardless of type. Membership is
/// resolved per type from the bundled FHIR `CompartmentDefinition`s (via
/// [`helios_fhir::get_compartment_params`]) — the same source the single-type
/// compartment search uses.
///
/// Each member type is searched independently and the matches are merged into a
/// single `searchset` Bundle. Query parameters are applied per type on a
/// best-effort basis: a type for which a parameter is not a valid search
/// parameter is skipped (it cannot match that constraint anyway).
///
/// # Pagination
///
/// Cross-type keyset pagination is not supported. Results are capped at the
/// effective page size (`_count`, clamped to the server maximum) and no
/// `next`/`previous` links are emitted. `_count` therefore acts as an overall
/// ceiling across all member types.
async fn compartment_search_all<S>(
    state: AppState<S>,
    compartment_type: String,
    compartment_id: String,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    pairs: Vec<(String, String)>,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let fhir_version = version.storage_version();
    let compartment_ref = format!("{}/{}", compartment_type, compartment_id);
    let search_params = SearchParams::from_pairs(pairs.clone());

    // Effective overall page size, clamped to the server maximum.
    let count = search_params
        .count()
        .unwrap_or(state.default_page_size())
        .min(state.max_page_size());

    // Enumerate compartment member types and pre-build a SearchQuery for each,
    // restricted to the compartment. Building happens under the registry read
    // lock (no await); the lock is released before any search executes.
    let queries: Vec<helios_persistence::types::SearchQuery> = {
        let registry = state.storage().search_param_registry().read();
        crate::fhir_types::get_resource_type_names_for_version(fhir_version)
            .iter()
            .filter_map(|target_type| {
                let ref_params = helios_fhir::get_compartment_params(
                    fhir_version,
                    &compartment_type,
                    target_type,
                );
                if ref_params.is_empty() {
                    return None; // not a member of this compartment
                }

                // A parameter that is invalid for this member type means the type
                // cannot satisfy it — skip the type rather than failing the request.
                let mut query = match build_search_query(target_type, &search_params, &registry) {
                    Ok(q) => q,
                    Err(e) => {
                        debug!(
                            target_type = %target_type,
                            error = %e,
                            "Skipping compartment member type: query not applicable"
                        );
                        return None;
                    }
                };

                query.compartment = Some(helios_persistence::types::CompartmentMembership {
                    params: ref_params.iter().map(|s| s.to_string()).collect(),
                    reference: compartment_ref.clone(),
                });
                // Cross-type paging is unsupported; cap each type at the overall
                // ceiling and drop any client cursor/offset.
                query.count = Some(count as u32);
                query.cursor = None;
                query.offset = None;
                Some(query)
            })
            .collect()
    };

    // Run each member-type search, accumulating matches up to the page-size cap.
    let mut collected: Vec<helios_persistence::types::StoredResource> = Vec::new();
    for query in &queries {
        if collected.len() >= count {
            break;
        }
        match state.storage().search(tenant.context(), query).await {
            Ok(result) => collected.extend(result.resources.items),
            Err(e) => {
                tracing::warn!(
                    resource_type = %query.resource_type,
                    error = %e,
                    "Compartment all-types search failed for member type"
                );
                return Err(RestError::from(e));
            }
        }
    }
    collected.truncate(count);

    // Merge into a single searchset Bundle (no cross-type paging links).
    let page =
        helios_persistence::types::Page::new(collected, helios_persistence::types::PageInfo::end());
    let result = helios_persistence::core::SearchResult::new(page);

    let self_link = build_compartment_search_url(
        state.base_url(),
        &compartment_type,
        &compartment_id,
        "*",
        &search_params,
    );
    let bundle = result.to_bundle(state.base_url(), &self_link);

    debug!(
        compartment_type = %compartment_type,
        compartment_id = %compartment_id,
        results = result.resources.len(),
        "Compartment all-types search completed"
    );

    Ok((StatusCode::OK, Json(bundle_to_json(bundle))).into_response())
}

/// Builds a compartment search URL.
fn build_compartment_search_url(
    base_url: &str,
    compartment_type: &str,
    compartment_id: &str,
    target_type: &str,
    params: &SearchParams,
) -> String {
    let path = format!(
        "{}/{}/{}/{}",
        base_url, compartment_type, compartment_id, target_type
    );

    let query: String = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    if query.is_empty() {
        path
    } else {
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
    use helios_fhir::FhirVersion;

    #[test]
    fn test_get_compartment_params_patient_observation() {
        // Test that Patient compartment includes Observation with subject and performer params
        let params =
            helios_fhir::get_compartment_params(FhirVersion::default(), "Patient", "Observation");
        assert!(!params.is_empty());
        assert!(params.contains(&"subject"));
    }

    #[test]
    fn test_get_compartment_params_patient_immunization() {
        // Test that Patient compartment includes Immunization with patient param
        let params =
            helios_fhir::get_compartment_params(FhirVersion::default(), "Patient", "Immunization");
        assert!(!params.is_empty());
        assert!(params.contains(&"patient"));
    }

    #[test]
    fn test_get_compartment_params_encounter_procedure() {
        // Test that Encounter compartment includes Procedure with encounter param
        let params =
            helios_fhir::get_compartment_params(FhirVersion::default(), "Encounter", "Procedure");
        assert!(!params.is_empty());
        assert!(params.contains(&"encounter"));
    }

    #[test]
    fn test_get_compartment_params_unknown() {
        // Test that unknown resource types return an empty slice
        let params =
            helios_fhir::get_compartment_params(FhirVersion::default(), "Patient", "UnknownType");
        assert!(params.is_empty());
    }

    #[test]
    fn test_get_compartment_params_multiple() {
        // Test that some resources have multiple compartment params
        // AllergyIntolerance in Patient compartment has: patient, recorder, asserter
        let params = helios_fhir::get_compartment_params(
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
            &SearchParams::from_pairs(vec![]),
        );
        assert_eq!(url, "http://example.com/fhir/Patient/123/Observation");
    }

    #[test]
    fn test_build_compartment_search_url_with_params() {
        let params = SearchParams::from_pairs(vec![("code".to_string(), "8867-4".to_string())]);

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
