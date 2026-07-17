//! `$purge` — permanent, irreversible deletion of a resource and its history.
//!
//! This is **not** the FHIR `delete` interaction. An ordinary `DELETE` is a soft
//! delete: it writes a tombstone version, the resource stays in `_history`, and
//! a `vread` of an earlier version still works. `$purge` removes the bytes.
//!
//! | Route | Effect |
//! |---|---|
//! | `DELETE /{type}/{id}/$purge` | Purges one resource and all its versions |
//! | `POST /{type}/$purge` | Purges every resource of that type in the tenant |
//!
//! `$purge` is not defined by the FHIR specification; it exists to serve
//! erasure requirements (GDPR Article 17 and similar) that a soft delete cannot.
//!
//! # Authorization
//!
//! Gated on the `system/purge` operation scope, following the same pattern as
//! `system/bulk-submit` — **not** on ordinary `Delete` scope. A token that may
//! soft-delete a Patient must not thereby be able to destroy it and its entire
//! history irrecoverably.
//!
//! # AuditEvent is never purgeable
//!
//! Purging an `AuditEvent` would let a privileged caller erase the record of
//! what they did. That is refused unconditionally, before the scope check even
//! matters — no scope grants it.
//!
//! # Composite deployments
//!
//! The purge target is the *composite* storage, not the primary, so the delete
//! reaches the Elasticsearch secondary too. Purging only the primary would leave
//! the resource in the search index, still searchable and still holding its full
//! content — which defeats the entire purpose of the operation.

use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_auth::Principal;
use helios_persistence::core::ResourceStorage;
use helios_persistence::core::storage::audit::record_purge_event;
use helios_persistence::error::{ResourceError, StorageError};
use serde_json::json;

use crate::error::{RestError, RestResult};
use crate::extractors::TenantExtractor;
use crate::state::AppState;

/// The operation scope a caller must hold to purge.
const PURGE_SCOPE: &str = "purge";

/// Resource types that can never be purged, whatever scope the caller holds.
///
/// The audit trail is append-only. A caller who can erase `AuditEvent` can erase
/// the evidence of their own actions, which makes every other audit control
/// decorative.
const UNPURGEABLE: [&str; 1] = ["AuditEvent"];

fn purge_unavailable() -> RestError {
    RestError::NotImplemented {
        feature: "$purge is not available: no purgable storage backend is configured".to_string(),
    }
}

/// Enforces the `system/purge` operation scope. Auth disabled → allowed.
fn check_purge_scope(principal: Option<&Principal>) -> RestResult<()> {
    if let Some(p) = principal
        && !p.scopes.grants_operation(PURGE_SCOPE)
    {
        return Err(RestError::Forbidden {
            message: "the `system/purge` scope is required".to_string(),
        });
    }
    Ok(())
}

/// Refuses to purge resource types whose whole value is being un-erasable.
fn check_purgeable_type(resource_type: &str) -> RestResult<()> {
    if UNPURGEABLE.contains(&resource_type) {
        return Err(RestError::Forbidden {
            message: format!(
                "{resource_type} resources cannot be purged: the audit trail is append-only"
            ),
        });
    }
    Ok(())
}

/// Emits the purge `AuditEvent`, including for a purge that failed.
///
/// A failed purge on a composite deployment means one backend still holds the
/// resource while another does not. That inconsistency is exactly what an
/// operator needs to see, so it is audited with outcome `8` rather than dropped.
#[allow(clippy::too_many_arguments)]
async fn emit_purge_audit<S>(
    state: &AppState<S>,
    principal: Option<&Principal>,
    resource_type: &str,
    resource_id: Option<&str>,
    count: u64,
    patient_ref: Option<&str>,
    outcome: &str,
    outcome_desc: Option<&str>,
) where
    S: ResourceStorage,
{
    let Some(sink) = state.audit_sink() else {
        return;
    };
    record_purge_event(
        sink.as_ref(),
        state.audit_source_observer(),
        principal.map(|p| p.subject.as_str()),
        resource_type,
        resource_id,
        count,
        patient_ref,
        outcome,
        outcome_desc,
    )
    .await;
}

/// `DELETE /{resource_type}/{id}/$purge`
pub async fn purge_instance_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    Path((resource_type, id)): Path<(String, String)>,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let principal = request.extensions().get::<Principal>().cloned();

    check_purgeable_type(&resource_type)?;
    check_purge_scope(principal.as_ref())?;

    let purge = state.purge().ok_or_else(purge_unavailable)?;

    // Read before purging so the audit event can carry the patient compartment
    // reference. After the purge the resource is gone and it cannot be resolved.
    let patient_ref = state
        .storage()
        .read(tenant.context(), &resource_type, &id)
        .await
        .ok()
        .flatten()
        .and_then(|r| super::extract_patient_from_resource(&resource_type, r.content()));

    match purge.purge(tenant.context(), &resource_type, &id).await {
        Ok(()) => {
            emit_purge_audit(
                &state,
                principal.as_ref(),
                &resource_type,
                Some(&id),
                1,
                patient_ref.as_deref(),
                "0",
                None,
            )
            .await;

            Ok((
                StatusCode::OK,
                axum::Json(operation_outcome_information(&format!(
                    "Purged {resource_type}/{id} and all of its history"
                ))),
            )
                .into_response())
        }
        Err(StorageError::Resource(ResourceError::NotFound { .. })) => Err(RestError::NotFound {
            resource_type: resource_type.clone(),
            id: id.clone(),
        }),
        Err(e) => {
            emit_purge_audit(
                &state,
                principal.as_ref(),
                &resource_type,
                Some(&id),
                0,
                patient_ref.as_deref(),
                "8",
                Some(&e.to_string()),
            )
            .await;
            Err(RestError::from(e))
        }
    }
}

/// `POST /{resource_type}/$purge`
pub async fn purge_type_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    Path(resource_type): Path<String>,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let principal = request.extensions().get::<Principal>().cloned();

    check_purgeable_type(&resource_type)?;
    check_purge_scope(principal.as_ref())?;

    let purge = state.purge().ok_or_else(purge_unavailable)?;

    match purge.purge_all(tenant.context(), &resource_type).await {
        Ok(count) => {
            emit_purge_audit(
                &state,
                principal.as_ref(),
                &resource_type,
                None,
                count,
                None,
                "0",
                None,
            )
            .await;

            Ok((
                StatusCode::OK,
                axum::Json(operation_outcome_information(&format!(
                    "Purged {count} {resource_type} resource(s) and all of their history"
                ))),
            )
                .into_response())
        }
        Err(e) => {
            emit_purge_audit(
                &state,
                principal.as_ref(),
                &resource_type,
                None,
                0,
                None,
                "8",
                Some(&e.to_string()),
            )
            .await;
            Err(RestError::from(e))
        }
    }
}

/// Builds an `information`-severity `OperationOutcome`.
fn operation_outcome_information(diagnostics: &str) -> serde_json::Value {
    json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "information",
            "code": "informational",
            "diagnostics": diagnostics,
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use helios_auth::scope::ScopeSet;

    fn principal(scopes: &str) -> Principal {
        Principal {
            subject: "client-1".to_string(),
            issuer: "https://idp.example.com".to_string(),
            tenant_id: Some("t1".to_string()),
            scopes: ScopeSet::parse(scopes),
            jti: None,
            expires_at: Utc::now() + chrono::Duration::hours(1),
            custom_claims: serde_json::Map::new(),
        }
    }

    /// Ordinary delete scope must NOT authorize a purge. A client that may
    /// soft-delete a Patient must not thereby be able to obliterate it.
    #[test]
    fn test_delete_scope_does_not_grant_purge() {
        let p = principal("system/Patient.d user/Patient.cruds");
        assert!(check_purge_scope(Some(&p)).is_err());
    }

    #[test]
    fn test_explicit_purge_scope_grants_purge() {
        let p = principal("system/purge");
        assert!(check_purge_scope(Some(&p)).is_ok());
    }

    /// Consistent with `system/bulk-submit`: a system-level wildcard is treated
    /// as granting every operation scope.
    #[test]
    fn test_system_wildcard_grants_purge() {
        let p = principal("system/*.crud");
        assert!(check_purge_scope(Some(&p)).is_ok());
    }

    /// With auth disabled there is no principal and no scope to check, matching
    /// every other endpoint.
    #[test]
    fn test_no_principal_allows_purge() {
        assert!(check_purge_scope(None).is_ok());
    }

    /// No scope — not even `system/purge`, not even a system wildcard — may
    /// erase the audit trail.
    #[test]
    fn test_audit_event_is_never_purgeable() {
        assert!(check_purgeable_type("AuditEvent").is_err());
        assert!(check_purgeable_type("Patient").is_ok());
    }
}
