//! `$meta`, `$meta-add` and `$meta-delete` (FHIR `Resource-meta`,
//! `Resource-meta-add`, `Resource-meta-delete`).
//!
//! - `GET|POST [base]/[type]/[id]/$meta` — the resource's current `meta`.
//! - `GET|POST [base]/[type]/[id]/_history/[vid]/$meta` — the `meta` of one
//!   version.
//! - `POST [base]/[type]/[id]/$meta-add` — add profiles, tags and security
//!   labels.
//! - `POST [base]/[type]/[id]/$meta-delete` — remove them.
//! - `GET|POST [base]/$meta`, `[base]/[type]/$meta` — the label summary across
//!   a type or the system is not implemented. The operation definition lets a
//!   server refuse it with a 4xx; this one answers `400 not-supported`.
//!
//! Every form answers a `Parameters` whose `return` is a `Meta`.
//!
//! An update merges tags and security labels instead of replacing them
//! (#2562), so `$meta-delete` is the only way a client can remove one.
//!
//! # Versions
//!
//! The specification does not say whether `$meta-add` and `$meta-delete`
//! create a new version. Here a change to `meta` writes a new version, like
//! any other update: history, the ETag, subscriptions and the audit trail all
//! see it. An operation that changes nothing writes nothing and returns the
//! current `meta`.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use helios_persistence::core::{ResourceStorage, SearchProvider, VersionedStorage};
use helios_persistence::types::{MetaOperation, StoredResource, apply_meta_operation};
use serde_json::{Value, json};
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirResource, TenantExtractor};
use crate::handlers::extract_patient_from_resource;
use crate::middleware::conditional::ConditionalHeaders;
use crate::middleware::content_type::negotiate_format;
use crate::patient_compartment::PatientCompartment;
use crate::responses::format_resource_response;
use crate::responses::headers::ResourceHeaders;
use crate::state::AppState;

/// `GET|POST [base]/[type]/[id]/$meta`
pub async fn meta_instance_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
    compartment: PatientCompartment,
    headers: HeaderMap,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let stored = read_current(&state, &tenant, &compartment, &resource_type, &id).await?;
    respond(&state, &stored, &headers)
}

/// `GET|POST [base]/[type]/[id]/_history/[vid]/$meta`
pub async fn meta_version_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id, version_id)): Path<(String, String, String)>,
    tenant: TenantExtractor,
    compartment: PatientCompartment,
    headers: HeaderMap,
) -> RestResult<Response>
where
    S: ResourceStorage + VersionedStorage + SearchProvider + Send + Sync,
{
    let stored = super::vread::execute_vread(
        &state,
        &tenant,
        &compartment,
        &resource_type,
        &id,
        &version_id,
    )
    .await?;
    respond(&state, &stored, &headers)
}

/// `GET|POST [base]/$meta` and `[base]/[type]/$meta`: not supported.
pub async fn meta_summary_not_supported_handler() -> RestResult<Response> {
    Err(RestError::NotSupported {
        feature: "$meta at the system or type level (a summary of the labels in use); \
                  invoke it on a resource instance: [type]/[id]/$meta"
            .to_string(),
    })
}

/// `POST [base]/[type]/[id]/$meta-add`
#[allow(clippy::too_many_arguments)]
pub async fn meta_add_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
    compartment: PatientCompartment,
    conditional: ConditionalHeaders,
    headers: HeaderMap,
    FhirResource(body): FhirResource,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    change_meta(
        &state,
        MetaChange {
            resource_type,
            id,
            operation: MetaOperation::Add,
        },
        &tenant,
        &compartment,
        &conditional,
        &headers,
        &body,
    )
    .await
}

/// `POST [base]/[type]/[id]/$meta-delete`
#[allow(clippy::too_many_arguments)]
pub async fn meta_delete_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
    compartment: PatientCompartment,
    conditional: ConditionalHeaders,
    headers: HeaderMap,
    FhirResource(body): FhirResource,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    change_meta(
        &state,
        MetaChange {
            resource_type,
            id,
            operation: MetaOperation::Delete,
        },
        &tenant,
        &compartment,
        &conditional,
        &headers,
        &body,
    )
    .await
}

/// The target and kind of a `$meta-add` / `$meta-delete`.
struct MetaChange {
    resource_type: String,
    id: String,
    operation: MetaOperation,
}

impl MetaChange {
    fn name(&self) -> &'static str {
        match self.operation {
            MetaOperation::Add => "$meta-add",
            MetaOperation::Delete => "$meta-delete",
        }
    }
}

async fn change_meta<S>(
    state: &AppState<S>,
    change: MetaChange,
    tenant: &TenantExtractor,
    compartment: &PatientCompartment,
    conditional: &ConditionalHeaders,
    headers: &HeaderMap,
    body: &Value,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let MetaChange {
        resource_type, id, ..
    } = &change;
    let operation = change.name();

    // AuditEvent resources are immutable, as on update and patch.
    if resource_type == "AuditEvent" {
        return Err(RestError::MethodNotAllowed {
            method: operation.to_string(),
            resource_type: resource_type.clone(),
        });
    }

    let meta = meta_parameter(body, operation)?;
    let existing = read_current(state, tenant, compartment, resource_type, id).await?;

    // `If-Match`, as on patch: list-aware, malformed fails the precondition.
    let if_match = conditional
        .if_match_tags()
        .map_err(|e| RestError::PreconditionFailed {
            message: format!("Malformed If-Match header: {e}"),
        })?;
    if !if_match.if_match_satisfied(Some(existing.version_id())) {
        return Err(RestError::PreconditionFailed {
            message: format!(
                "If-Match precondition failed: no supplied entity-tag matches the current version W/\"{}\"",
                existing.version_id()
            ),
        });
    }

    let mut content = existing.content().clone();
    let changed =
        apply_meta_operation(&mut content, meta, change.operation).map_err(|message| {
            RestError::BadRequest {
                message: format!("{operation}: {message}"),
            }
        })?;
    if !changed {
        debug!(%resource_type, %id, operation, "meta unchanged; nothing written");
        return respond(state, &existing, headers);
    }

    // A profile added is a profile the resource now claims; check the write
    // as update and patch do.
    state
        .validation()
        .check_write(
            tenant.tenant_id(),
            existing.fhir_version(),
            resource_type,
            &content,
        )
        .await?;

    let stored = match change.operation {
        // Everything already present is still present, so the merge an update
        // applies is a no-op here.
        MetaOperation::Add => {
            state
                .storage()
                .update(tenant.context(), &existing, content)
                .await?
        }
        // A plain update would merge the deleted labels straight back (#2562).
        MetaOperation::Delete => {
            state
                .storage()
                .update_verbatim(tenant.context(), &existing, content)
                .await?
        }
    };

    if resource_type == "StructureDefinition" {
        state.validation().upsert_stored_profile(
            tenant.tenant_id(),
            stored.fhir_version(),
            stored.content(),
        );
    }

    debug!(
        %resource_type,
        %id,
        operation,
        version = %stored.version_id(),
        "meta changed"
    );

    super::write_event::report(
        state,
        tenant.context(),
        stored.fhir_version(),
        resource_type,
        0,
        Some(super::write_event::stored_notice(
            helios_persistence::core::WriteKind::Update,
            &stored,
        )),
    );

    let mut response = respond(state, &stored, headers)?;
    response
        .extensions_mut()
        .insert(helios_audit::AuditResponseContext {
            resource_type: Some(resource_type.clone()),
            resource_id: Some(stored.id().to_string()),
            patient_reference: extract_patient_from_resource(resource_type, stored.content()),
        });
    Ok(response)
}

/// Reads the live resource, confined to a `patient/` scope's compartment.
async fn read_current<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    compartment: &PatientCompartment,
    resource_type: &str,
    id: &str,
) -> RestResult<StoredResource>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let stored = state
        .storage()
        .read(tenant.context(), resource_type, id)
        .await?
        .ok_or_else(|| RestError::NotFound {
            resource_type: resource_type.to_string(),
            id: id.to_string(),
        })?;
    compartment.assert_member(
        || state.storage().search_param_registry(tenant.context()),
        stored.fhir_version(),
        stored.content(),
        "meta",
    )?;
    Ok(stored)
}

/// The `meta` input parameter (1..1, `Meta`) of a `Parameters` body.
fn meta_parameter<'a>(body: &'a Value, operation: &str) -> RestResult<&'a Value> {
    if body.get("resourceType").and_then(Value::as_str) != Some("Parameters") {
        return Err(RestError::BadRequest {
            message: format!("{operation} takes a Parameters resource with a 'meta' parameter"),
        });
    }
    let mut metas = body
        .get("parameter")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|p| p.get("name").and_then(Value::as_str) == Some("meta"));
    let meta = metas.next().ok_or_else(|| RestError::BadRequest {
        message: format!("{operation} requires the 'meta' parameter"),
    })?;
    if metas.next().is_some() {
        return Err(RestError::BadRequest {
            message: format!("{operation} takes exactly one 'meta' parameter"),
        });
    }
    meta.get("valueMeta").ok_or_else(|| RestError::BadRequest {
        message: format!("{operation}: the 'meta' parameter must carry a valueMeta"),
    })
}

/// `Parameters { return: <stored.meta> }`, with the version's ETag and
/// Last-Modified.
fn respond<S>(
    state: &AppState<S>,
    stored: &StoredResource,
    headers: &HeaderMap,
) -> RestResult<Response>
where
    S: ResourceStorage,
{
    let negotiated = negotiate_format(headers, None);
    let meta = stored
        .content_with_meta()
        .get("meta")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let parameters = json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "return", "valueMeta": meta}]
    });
    let header_map = ResourceHeaders::from_stored(stored, state)
        .with_content_type(negotiated.format.mime_type())
        .to_header_map();
    format_resource_response(StatusCode::OK, header_map, &parameters, negotiated.format)
}
