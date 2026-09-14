//! FHIR `Patient/$everything` operation.
//!
//! Composed in the REST layer over the per-type [`SearchProvider::search`]
//! and the query-time compartment predicate that `GET /Patient/{id}/*` uses.
//! See `docs/superpowers/specs/2026-09-14-patient-everything-design.md`.

pub(crate) mod cursor;
pub(crate) mod params;
pub(crate) mod scope;
pub(crate) mod walk;

use axum::{
    Json,
    extract::{Path, Request, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use helios_persistence::core::{ResourceStorage, SearchProvider, SearchResult};
use helios_persistence::types::{BundleEntry, Page, PageInfo};
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirVersionExtractor, TenantExtractor};
use crate::handlers::bulk_common::{pairs_from_parameters, parse_query_pairs};
use crate::state::AppState;
use cursor::EverythingCursor;
use params::EverythingParams;
use walk::{WalkLimits, WalkOutput};

/// `GET|POST /Patient/{id}/$everything`
pub async fn patient_everything_instance_handler<S>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    run(state, Some(id), tenant, version, request).await
}

/// `GET|POST /Patient/$everything`
pub async fn patient_everything_type_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    run(state, None, tenant, version, request).await
}

async fn decode_pairs(request: Request) -> RestResult<Vec<(String, String)>> {
    let method = request.method().clone();
    let mut pairs = parse_query_pairs(request.uri().query());
    if method == Method::POST {
        let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
            .await
            .map_err(|e| RestError::BadRequest {
                message: format!("Unreadable request body: {e}"),
            })?;
        if !bytes.is_empty() {
            let body: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| RestError::BadRequest {
                    message: format!("Invalid JSON body: {e}"),
                })?;
            if body.get("resourceType").and_then(|v| v.as_str()) != Some("Parameters") {
                return Err(RestError::BadRequest {
                    message: "POST $everything requires a Parameters resource body".to_string(),
                });
            }
            pairs.extend(pairs_from_parameters(&body));
        }
    }
    Ok(pairs)
}

async fn run<S>(
    state: AppState<S>,
    patient_id: Option<String>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let fhir_version = version.storage_version_or(state.config().default_fhir_version);
    let pairs = decode_pairs(request).await?;
    let params = EverythingParams::from_pairs(&pairs, fhir_version, state.max_page_size())?;
    let fp_input =
        params.fingerprint_input(patient_id.as_deref(), tenant.tenant_id(), fhir_version);
    let resume = match &params.cursor {
        Some(token) => Some(EverythingCursor::decode(token, &fp_input)?),
        None => None,
    };
    let limits = WalkLimits {
        page: params.count,
        unpaged_ceiling: state.everything_max_unpaged(),
        per_query: state.max_page_size(),
    };
    debug!(patient = ?patient_id, tenant = %tenant.tenant_id(), params = ?params, "Processing $everything");

    let out: WalkOutput = match &patient_id {
        Some(pid) => {
            walk::walk_patient(
                &state,
                tenant.context(),
                fhir_version,
                pid,
                &params,
                resume,
                limits,
            )
            .await?
        }
        None => {
            walk::walk_all_patients(
                &state,
                tenant.context(),
                fhir_version,
                &params,
                resume,
                limits,
            )
            .await?
        }
    };

    let public_base = state.public_base_url_for_request(&tenant);
    let self_link = build_self_link(&public_base, patient_id.as_deref(), &pairs);
    let paged = params.count.is_some() || params.cursor.is_some() || out.ceiling_hit;
    let total = if paged {
        None
    } else {
        Some(out.matches.len() as u64)
    };
    let next_token = out.next.as_ref().map(EverythingCursor::encode);
    let page_info = PageInfo {
        next_cursor: next_token.clone(),
        previous_cursor: None,
        total,
        has_next: next_token.is_some(),
        has_previous: false,
    };
    let result = SearchResult {
        resources: Page::new(out.matches, page_info),
        included: out.included,
        total,
        scores: Default::default(),
    };
    let mut bundle = result.into_bundle(&public_base, &self_link);
    if out.ceiling_hit {
        bundle.entry.push(BundleEntry::outcome_entry(serde_json::json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "information",
                "code": "informational",
                "diagnostics": format!(
                    "Result exceeded the server's unpaged limit of {} entries and was paged; follow the 'next' link for the remainder",
                    state.everything_max_unpaged()
                )
            }]
        })));
    }
    if out.includes_truncated {
        bundle
            .entry
            .push(BundleEntry::outcome_entry(serde_json::json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "information",
                    "code": "informational",
                    "diagnostics": format!(
                        "Supporting resources exceeded the server's limit of {} and were truncated",
                        state.everything_max_unpaged()
                    )
                }]
            })));
    }
    crate::public_url::rewrite_bundle_full_urls(&mut bundle, |resource_type, id| {
        state.public_url_for_request(&tenant, [resource_type, id])
    });

    let mut response = (
        StatusCode::OK,
        Json(crate::responses::bundle::searchset_to_json(
            bundle,
            |resource| resource,
        )),
    )
        .into_response();
    response
        .extensions_mut()
        .insert(helios_audit::AuditResponseContext {
            resource_type: Some("Patient".to_string()),
            resource_id: patient_id.clone(),
            patient_reference: patient_id.as_ref().map(|id| format!("Patient/{id}")),
        });
    Ok(response)
}

fn build_self_link(base_url: &str, patient_id: Option<&str>, pairs: &[(String, String)]) -> String {
    let query: String = pairs
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                k,
                url::form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    let url = crate::public_url::PublicUrl::parse(base_url)
        .expect("request public base was built from validated configuration");
    match patient_id {
        Some(id) => url.with_segments_and_query(["Patient", id, "$everything"], &query),
        None => url.with_segments_and_query(["Patient", "$everything"], &query),
    }
}
