//! `$purge` operation handlers.
//!
//! Permanent deletion of FHIR resources (and their history) — a destructive,
//! irreversible operation distinct from the soft-delete [`delete_handler`]
//! exposed by the standard FHIR REST API. Purge is opt-in: when no
//! [`PurgableStorage`] provider is wired into [`AppState`], the handlers return
//! `501 Not Implemented`.
//!
//! Each invocation emits a BALP-compliant `AuditEvent` (action `Delete`) via
//! [`helios_persistence::core::storage::audit::record_purge_event`].
//!
//! # Routes
//!
//! - `DELETE /{resource_type}/{id}/$purge` — purge a single resource and its
//!   complete version history.
//! - `POST /{resource_type}/$purge` — purge every resource of a type within
//!   the current tenant. Returns the number of resources purged.
//!
//! # Authorization
//!
//! A SMART-on-FHIR `*.write` (or wildcard) scope on the target resource type is
//! required when authentication is enabled. Operations targeting `AuditEvent`
//! are rejected: the audit trail is immutable.
//!
//! [`delete_handler`]: super::delete::delete_handler
//! [`PurgableStorage`]: helios_persistence::core::storage::PurgableStorage

use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::StatusCode,
    response::Response,
};
use helios_auth::Principal;
use helios_persistence::core::ResourceStorage;

use crate::error::{RestError, RestResult};
use crate::extractors::TenantExtractor;
use crate::state::AppState;

fn not_implemented() -> RestError {
    RestError::NotImplemented {
        feature: "$purge is not supported by the configured storage backend".to_string(),
    }
}

fn reject_audit_event(resource_type: &str) -> Option<RestError> {
    if resource_type == "AuditEvent" {
        Some(RestError::MethodNotAllowed {
            method: "PURGE".to_string(),
            resource_type: resource_type.to_string(),
        })
    } else {
        None
    }
}

/// `DELETE /{resource_type}/{id}/$purge` — permanently delete a single resource.
pub async fn purge_instance_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    if let Some(e) = reject_audit_event(&resource_type) {
        return Err(e);
    }
    let provider = state.purge_provider().ok_or_else(not_implemented)?;
    let principal = request.extensions().get::<Principal>().cloned();

    if let Some(p) = principal.as_ref() {
        if let Err(e) = helios_auth::SmartScopePolicy::check(
            p,
            &resource_type,
            helios_auth::FhirOperation::Delete,
        ) {
            let msg = e.to_string();
            emit_purge_audit(
                &state,
                principal.as_ref(),
                &resource_type,
                Some(&id),
                0,
                "8",
                Some(&msg),
            )
            .await;
            return Err(RestError::Forbidden { message: msg });
        }
    }

    match provider.purge(tenant.context(), &resource_type, &id).await {
        Ok(()) => {
            emit_purge_audit(
                &state,
                principal.as_ref(),
                &resource_type,
                Some(&id),
                1,
                "0",
                None,
            )
            .await;
            Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(Body::empty())
                .map_err(|e| RestError::InternalError {
                    message: e.to_string(),
                })
        }
        Err(e) => {
            let msg = e.to_string();
            emit_purge_audit(
                &state,
                principal.as_ref(),
                &resource_type,
                Some(&id),
                0,
                "8",
                Some(&msg),
            )
            .await;
            Err(e.into())
        }
    }
}

/// `POST /{resource_type}/$purge` — permanently delete every resource of a type
/// within the current tenant.
pub async fn purge_type_handler<S>(
    State(state): State<AppState<S>>,
    Path(resource_type): Path<String>,
    tenant: TenantExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    if let Some(e) = reject_audit_event(&resource_type) {
        return Err(e);
    }
    let provider = state.purge_provider().ok_or_else(not_implemented)?;
    let principal = request.extensions().get::<Principal>().cloned();

    if let Some(p) = principal.as_ref() {
        if let Err(e) = helios_auth::SmartScopePolicy::check(
            p,
            &resource_type,
            helios_auth::FhirOperation::Delete,
        ) {
            let msg = e.to_string();
            emit_purge_audit(
                &state,
                principal.as_ref(),
                &resource_type,
                None,
                0,
                "8",
                Some(&msg),
            )
            .await;
            return Err(RestError::Forbidden { message: msg });
        }
    }

    match provider.purge_all(tenant.context(), &resource_type).await {
        Ok(count) => {
            emit_purge_audit(
                &state,
                principal.as_ref(),
                &resource_type,
                None,
                count,
                "0",
                None,
            )
            .await;
            let body = serde_json::json!({
                "resourceType": "Parameters",
                "parameter": [
                    {"name": "resourceType", "valueString": resource_type},
                    {"name": "purged", "valueInteger": count},
                ]
            });
            let bytes = serde_json::to_vec(&body).map_err(|e| RestError::InternalError {
                message: e.to_string(),
            })?;
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/fhir+json")
                .body(Body::from(bytes))
                .map_err(|e| RestError::InternalError {
                    message: e.to_string(),
                })
        }
        Err(e) => {
            let msg = e.to_string();
            emit_purge_audit(
                &state,
                principal.as_ref(),
                &resource_type,
                None,
                0,
                "8",
                Some(&msg),
            )
            .await;
            Err(e.into())
        }
    }
}

/// Emits a `$purge` `AuditEvent` when an audit sink is configured.
async fn emit_purge_audit<S>(
    state: &AppState<S>,
    principal: Option<&Principal>,
    resource_type: &str,
    resource_id: Option<&str>,
    count: u64,
    outcome: &str,
    outcome_desc: Option<&str>,
) where
    S: ResourceStorage,
{
    let Some(sink) = state.audit_sink() else {
        return;
    };
    let mut builder = helios_audit::AuditEventBuilder::new(state.audit_source_observer())
        .event_type(
            "http://terminology.hl7.org/CodeSystem/audit-event-type",
            "object",
        )
        .action(helios_audit::AuditAction::Delete)
        .outcome(outcome)
        .detail("audit-operation", "purge")
        .detail("count", count.to_string());
    if let Some(id) = resource_id {
        builder = builder.resource(resource_type, id);
    } else {
        builder = builder.detail("resource-type", resource_type);
    }
    if let Some(desc) = outcome_desc {
        builder = builder.outcome_desc(desc);
    }
    if let Some(p) = principal {
        builder = builder.agent(&p.subject, None, true);
    }
    sink.record(builder.build()).await;
}
