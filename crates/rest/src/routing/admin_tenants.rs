//! Router for the tenant-maintenance admin API (`/admin/tenants`).
//!
//! These are cross-tenant administrative operations, so `build_app` merges this
//! router into the console **admin** tier — the same tier as the cross-tenant
//! `console/metrics/tenants` endpoint — where it is wrapped in bearer-token
//! authentication plus [`crate::middleware::auth::admin_authz_middleware`],
//! which requires a system-context scope. An ordinary user-/patient-context
//! token, even a wildcard one, is rejected `403`; when auth is disabled
//! server-wide the tier is open like every other route (dev mode).

use axum::{
    Router,
    routing::{delete, get},
};
use helios_persistence::core::ResourceStorage;

use crate::handlers::admin_tenants;
use crate::state::AppState;

/// Tenant-maintenance routes. Merged into the admin tier by `build_app`, so the
/// system-context gate is applied there rather than here.
pub fn routes<S>(state: AppState<S>) -> Router
where
    S: ResourceStorage + Send + Sync + 'static,
{
    Router::new()
        .route(
            "/admin/tenants",
            get(admin_tenants::list_tenants_handler::<S>)
                .post(admin_tenants::create_tenant_handler::<S>),
        )
        .route(
            "/admin/tenants/{id}",
            delete(admin_tenants::delete_tenant_handler::<S>),
        )
        .with_state(state)
}
