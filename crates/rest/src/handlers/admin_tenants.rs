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
use helios_persistence::tenant::{SYSTEM_TENANT, TenantId};
use serde::Deserialize;
use serde_json::json;
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::state::AppState;

// The tenant-id length cap and reserved-name list that used to live here are
// now `helios_persistence::tenant::{MAX_TENANT_ID_LEN, RESERVED_TENANT_SEGMENTS}`,
// applied by `TenantId::parse` — see `validate_tenant_id` below (issue #385).

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

/// Seeds a newly-provisioned tenant with the spec conformance resources
/// (SearchParameters and CompartmentDefinitions), mirroring the per-tenant
/// startup seed. Best-effort: failures are logged, never surfaced to the caller
/// (the tenant is registered regardless; the next startup seed completes it).
async fn seed_new_tenant<S: ResourceStorage>(
    storage: &S,
    config: &crate::config::ServerConfig,
    tenant_id: &str,
) {
    let data_dir = config
        .data_dir
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("./data"));
    helios_persistence::search::seed_tenant_conformance(
        storage,
        config.default_fhir_version,
        &data_dir,
        tenant_id,
    )
    .await;
}

/// Validates a tenant id against the canonical definition.
///
/// This used to be a third, independent charset — wider than the two the REST
/// routing surfaces enforced (it permitted `.` and `/`) and with a different
/// length cap. Provisioning a tenant it accepted but routing did not produced a
/// registered tenant nobody could address. Validation now lives in one place
/// ([`TenantId::parse`], issue #385) and this handler only translates the
/// rejection into a `400`.
///
/// Two rules the old local version had are now *stronger*, not merely moved:
///
/// - The reserved-name check compares each `/`-separated **segment**, not the
///   whole id. `acme/resources` used to pass, and on S3 its objects land inside
///   the `acme/resources/` prefix that tenant `acme` lists and
///   `purge_tenant_data("acme")` deletes.
/// - The length cap is 64, not 128. An id in 65..=128 bytes was already
///   unroutable through both header and URL routing (each capped at 64), so it
///   was reachable only through the JWT claim — the ingress that validated
///   nothing.
fn validate_tenant_id(id: &str) -> RestResult<()> {
    TenantId::parse(id)
        .map(|_| ())
        .map_err(|e| RestError::BadRequest {
            message: e.to_string(),
        })
}

/// `GET /admin/tenants`
///
/// Returns every known tenant, newest registrations last. The list is the union
/// of the **registry** (tenants explicitly provisioned via `POST`, which carry a
/// `created_at` and optional `display_name`) and tenants merely **discovered**
/// from stored data (`registered: false`, `created_at: null`) so nothing with
/// data is hidden. Each row carries the current non-deleted `resources` count.
/// The internal system tenant is never listed.
///
/// Each row also carries `canonical` — whether the id satisfies
/// [`TenantId::parse`]. It is `true` for everything this API can create, so the
/// field only ever matters for ids that predate the canonical validator (issue
/// #385): those were stored under a wider (or absent) charset and are no longer
/// reachable through any ingress. This is how an operator finds them. Rows with
/// `canonical: false` and a non-zero `resources` count are the ones needing
/// attention — the data is still there, but requests naming that tenant are now
/// rejected. `DELETE /admin/tenants/{id}` still accepts them, deliberately, so
/// they can be cleaned up.
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

    // `new` (unchecked) then `is_canonical`, not `parse().is_ok()`, to say
    // plainly what this is: reporting on a value that is already stored, never
    // admitting one.
    let is_canonical = |id: &str| helios_persistence::tenant::TenantId::new(id).is_canonical();

    let mut seen = std::collections::HashSet::new();
    let mut tenants = Vec::new();
    for rec in &registered {
        seen.insert(rec.id.clone());
        tenants.push(json!({
            "id": rec.id,
            "display_name": rec.display_name,
            "created_at": rec.created_at,
            "registered": true,
            "canonical": is_canonical(&rec.id),
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
            "canonical": is_canonical(id),
            "resources": n,
        }));
    }

    let non_canonical = tenants
        .iter()
        .filter(|t| t["canonical"] == json!(false))
        .count();
    let body = json!({
        "tenant_count": tenants.len(),
        // Surfaced at the top level so an operator does not have to scan the
        // rows to learn whether the upgrade stranded anything.
        "non_canonical_count": non_canonical,
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

    // Seed the new tenant with the conformance resources (spec SearchParameters
    // and CompartmentDefinitions) so `GET /SearchParameter` and
    // `GET /CompartmentDefinition` are populated for it, matching what the
    // server seeds every provisioned tenant with at startup. Best-effort: a
    // failed seed logs and still returns the created tenant.
    if state.config().seed_conformance {
        seed_new_tenant(state.storage(), state.config(), &id).await;
    }

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
    // Validate BEFORE the existence probe below. `__system__` has data (the
    // AuditEvent trail), so probing first would let a reserved id past the 404
    // check and straight into `deregister_tenant` + `purge_tenant_data` — the
    // create path validated but this one never did (issue #317). Validating
    // first also means a reserved id reports 400 rather than a misleading 404.
    validate_tenant_id(&id)?;

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
