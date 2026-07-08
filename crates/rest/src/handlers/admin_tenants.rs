//! Tenant-maintenance admin API: `GET`/`POST`/`DELETE /admin/tenants`.
//!
//! These make tenants **first-class managed entities**. Historically a tenant
//! was only ever an implicit identifier string, accepted the first time it was
//! used; there was no registry, no metadata, and no lifecycle API. These
//! handlers add one, backed by [`ResourceStorage`]'s tenant-registry methods:
//!
//! - `GET  /admin/tenants` — list tenants (registered ∪ data-discovered) with
//!   their optional display name, creation date, and current resource count.
//! - `POST /admin/tenants` — register (provision) a new tenant.
//! - `DELETE /admin/tenants/{id}` — deregister a tenant and, when asked, tear
//!   down its data.
//!
//! **Trust tier.** These are cross-tenant administrative operations. They are
//! mounted in the console *admin* tier (see [`crate::routing::console_metrics`]),
//! which requires a system-context scope on top of authentication — an ordinary
//! user-/patient-context token, even a wildcard one, is rejected `403`. Backends
//! that do not maintain a registry (see
//! [`ResourceStorage::supports_tenant_registry`]) return `501 Not Implemented`.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_audit::{AuditAction, AuditEventBuilder};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::SYSTEM_TENANT;
use serde::Deserialize;
use serde_json::json;
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::state::AppState;

/// Maximum accepted tenant-id length. Generous, but bounds the value that flows
/// into schema names / storage keys.
const MAX_TENANT_ID_LEN: usize = 128;

/// Request body for `POST /admin/tenants`.
#[derive(Debug, Deserialize)]
pub struct CreateTenantBody {
    /// The tenant id — the value used in the API (`X-Tenant-ID` / URL / JWT).
    pub id: String,
    /// Optional human-friendly display name (presentation only).
    #[serde(default)]
    pub display_name: Option<String>,
}

/// Rejects an endpoint when the active backend has no tenant registry, so the
/// caller gets an honest `501` instead of a silently-empty list.
fn require_registry<S: ResourceStorage>(storage: &S) -> RestResult<()> {
    if storage.supports_tenant_registry() {
        Ok(())
    } else {
        Err(RestError::NotImplemented {
            feature: format!(
                "tenant registry (backend '{}' does not maintain one)",
                storage.backend_name()
            ),
        })
    }
}

/// Validates a tenant id: non-empty, within length, no whitespace, a
/// conservative identifier charset, and never the internal system sentinel.
fn validate_tenant_id(id: &str) -> RestResult<()> {
    if id.is_empty() {
        return Err(RestError::BadRequest {
            message: "tenant id must not be empty".to_string(),
        });
    }
    if id.len() > MAX_TENANT_ID_LEN {
        return Err(RestError::BadRequest {
            message: format!("tenant id exceeds {MAX_TENANT_ID_LEN} characters"),
        });
    }
    if id == SYSTEM_TENANT {
        return Err(RestError::BadRequest {
            message: "'__system__' is reserved for internal shared resources".to_string(),
        });
    }
    // Allow the characters that are safe across routing (header / URL prefix /
    // JWT claim) and storage keys: alphanumerics plus `-`, `_`, `.`, and `/`
    // (the hierarchy separator). Reject everything else, including whitespace.
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        return Err(RestError::BadRequest {
            message: "tenant id may contain only letters, digits, '-', '_', '.', and '/'"
                .to_string(),
        });
    }
    Ok(())
}

/// `GET /admin/tenants`
///
/// Returns every known tenant, newest registrations last. The list is the union
/// of the **registry** (tenants explicitly provisioned via `POST`, which carry a
/// `created_at` and optional `display_name`) and tenants merely **discovered**
/// from stored data (`registered: false`, `created_at: null`) so nothing with
/// data is hidden. Each row carries the current non-deleted `resources` count.
/// The internal system tenant is never listed.
pub async fn list_tenants_handler<S>(State(state): State<AppState<S>>) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    require_registry(state.storage())?;
    debug!("Processing admin list-tenants request");

    let registered = state.storage().list_tenants().await?;
    let counts: HashMap<String, u64> = state
        .storage()
        .count_by_tenant()
        .await?
        .into_iter()
        .collect();

    let mut seen = std::collections::HashSet::new();
    let mut tenants = Vec::new();
    for rec in &registered {
        seen.insert(rec.id.clone());
        tenants.push(json!({
            "id": rec.id,
            "display_name": rec.display_name,
            "created_at": rec.created_at,
            "registered": true,
            "resources": counts.get(&rec.id).copied().unwrap_or(0),
        }));
    }
    // Tenants that have data but were never registered — surfaced so the roster
    // is complete. They have no creation date or name until registered.
    let mut discovered: Vec<(&String, &u64)> = counts
        .iter()
        .filter(|(id, _)| id.as_str() != SYSTEM_TENANT && !seen.contains(id.as_str()))
        .collect();
    discovered.sort_by(|a, b| a.0.cmp(b.0));
    for (id, n) in discovered {
        tenants.push(json!({
            "id": id,
            "display_name": null,
            "created_at": null,
            "registered": false,
            "resources": n,
        }));
    }

    let body = json!({
        "tenant_count": tenants.len(),
        "tenants": tenants,
    });
    Ok((StatusCode::OK, Json(body)).into_response())
}

/// `POST /admin/tenants`
///
/// Registers (provisions) a new tenant. Body: `{ "id": "...", "display_name"?:
/// "..." }`. Returns `201` with the created record. `409` if the tenant is
/// already registered; `400` for an invalid id.
pub async fn create_tenant_handler<S>(
    State(state): State<AppState<S>>,
    Json(body): Json<CreateTenantBody>,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    require_registry(state.storage())?;
    let id = body.id.trim().to_string();
    validate_tenant_id(&id)?;
    let display_name = body
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    // Pre-check so a duplicate is a clean 409 rather than a constraint 500.
    if state.storage().get_tenant(&id).await?.is_some() {
        return Err(RestError::Conflict {
            message: format!("tenant '{id}' is already registered"),
        });
    }

    let record = state.storage().register_tenant(&id, display_name).await?;
    debug!(tenant = %id, "Registered tenant");
    audit(
        &state,
        AuditAction::Create,
        "0",
        format!("Registered tenant {id}"),
    )
    .await;

    let body = json!({
        "id": record.id,
        "display_name": record.display_name,
        "created_at": record.created_at,
        "registered": true,
        "resources": 0,
    });
    Ok((StatusCode::CREATED, Json(body)).into_response())
}

/// Query parameters for `DELETE /admin/tenants/{id}`.
#[derive(Debug, Deserialize)]
pub struct DeleteTenantQuery {
    /// When `true`, also permanently delete the tenant's stored data (resources,
    /// history, search index). Defaults to `false` — deregister only.
    #[serde(default)]
    pub purge: bool,
}

/// `DELETE /admin/tenants/{id}`
///
/// Removes a tenant's registration. With `?purge=true`, additionally tears down
/// the tenant's stored data (destructive, irreversible). Returns `200` with what
/// happened; `404` if the tenant was neither registered nor had any data.
pub async fn delete_tenant_handler<S>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
    Query(query): Query<DeleteTenantQuery>,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    require_registry(state.storage())?;

    // Establish what exists before mutating so we can 404 on a no-op and report
    // accurately. A tenant may be registered, have data, both, or neither.
    let was_registered = state.storage().get_tenant(&id).await?.is_some();
    let had_data = state
        .storage()
        .count_by_tenant()
        .await?
        .into_iter()
        .any(|(t, n)| t == id && n > 0);

    if !was_registered && !had_data {
        return Err(RestError::NotFound {
            resource_type: "Tenant".to_string(),
            id: id.clone(),
        });
    }

    let deregistered = state.storage().deregister_tenant(&id).await?;
    let resources_removed = if query.purge {
        Some(state.storage().purge_tenant_data(&id).await?)
    } else {
        None
    };

    debug!(tenant = %id, purge = query.purge, "Deleted tenant");
    audit(
        &state,
        AuditAction::Delete,
        "0",
        format!(
            "Deleted tenant {id} (deregistered={deregistered}, purged={})",
            query.purge
        ),
    )
    .await;

    let body = json!({
        "id": id,
        "deregistered": deregistered,
        "purged": query.purge,
        "resources_removed": resources_removed,
    });
    Ok((StatusCode::OK, Json(body)).into_response())
}

/// Emits a handler-level audit event when an audit sink is configured. Tenant
/// maintenance is administrative and (for delete) destructive, so every mutation
/// is recorded.
async fn audit<S>(state: &AppState<S>, action: AuditAction, outcome: &str, desc: String)
where
    S: ResourceStorage + Send + Sync,
{
    if let Some(sink) = state.audit_sink() {
        let event = AuditEventBuilder::new(state.audit_source_observer())
            .action(action)
            .outcome(outcome)
            .outcome_desc(desc)
            .build();
        sink.record(event).await;
    }
}
