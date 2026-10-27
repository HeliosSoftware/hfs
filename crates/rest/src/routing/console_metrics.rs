//! Routers for the management-console metrics endpoints.
//!
//! Console routes split into three trust tiers:
//!
//! - [`public_routes`] — only the server-global, non-sensitive `uptime` endpoint
//!   (like `/health`). Merged **outside** the auth layer so the console's login
//!   screen can show liveness without a token. Still covered by the shared CORS /
//!   timeout / body-limit / tracing stack.
//! - [`protected_routes`] — **tenant-scoped** endpoints (`resource-counts`,
//!   `activity`, `resource-distribution`). These require authentication;
//!   `build_app` layers the same bearer-token auth used by the FHIR routes around
//!   this router. Behind auth, the tenant is taken from the JWT claim (see
//!   [`crate::extractors::TenantExtractor`]), so a spoofed `X-Tenant-ID` cannot
//!   widen access beyond the caller's own tenant.
//! - [`admin_routes`] — **cross-tenant** endpoints (`tenants`, `traffic`) that
//!   surface data spanning every tenant. `build_app` additionally requires a
//!   system-context scope (`system/*.r`) via
//!   [`crate::middleware::auth::admin_authz_middleware`], so an ordinary
//!   user-/patient-context token — even one with a broad wildcard — cannot read
//!   the tenant roster or server-wide traffic.
//!
//! All three are merged into the application by `build_app` in `crate::lib`.

use axum::{Router, routing::get};
use helios_persistence::core::ResourceStorage;

use crate::handlers::console_metrics;
use crate::state::AppState;

/// Console routes that may be served without authentication.
///
/// Only `uptime` — server-global and non-sensitive, like `/health`. Everything
/// that touches tenant data lives in [`protected_routes`].
pub fn public_routes<S>(state: AppState<S>) -> Router
where
    S: ResourceStorage + Send + Sync + 'static,
{
    Router::new()
        .route(
            "/console/metrics/uptime",
            get(console_metrics::uptime_handler::<S>),
        )
        .with_state(state)
}

/// Tenant-scoped console routes: they require authentication but return only the
/// caller's own tenant data.
///
/// `build_app` wraps this router in the FHIR routes' bearer-token authentication.
/// `resource-counts`, `activity`, and `resource-distribution` are tenant-scoped —
/// the tenant is taken from the JWT claim behind auth (see
/// [`crate::extractors::TenantExtractor`]), so any authenticated principal sees
/// only its own tenant. Cross-tenant endpoints live in [`admin_routes`].
pub fn protected_routes<S>(state: AppState<S>) -> Router
where
    S: ResourceStorage + Send + Sync + 'static,
{
    Router::new()
        .route(
            "/console/metrics/resource-counts",
            get(console_metrics::resource_counts_handler::<S>),
        )
        .route(
            "/console/metrics/activity",
            get(console_metrics::activity_handler::<S>),
        )
        .route(
            "/console/metrics/resource-distribution",
            get(console_metrics::resource_distribution_handler::<S>),
        )
        .with_state(state)
}

/// Cross-tenant console routes: they expose data spanning **every** tenant and so
/// require an administrative, system-context scope on top of authentication.
///
/// `build_app` wraps this router in bearer-token auth **and**
/// [`crate::middleware::auth::admin_authz_middleware`], which requires a
/// `system/*.r` scope. `tenants` returns the full tenant roster and per-tenant
/// sizes; `traffic` returns server-wide throughput and latency — neither is
/// scoped to a single tenant, so an ordinary `user/*` or `patient/*` token (even
/// a wildcard one) is rejected `403`.
pub fn admin_routes<S>(state: AppState<S>) -> Router
where
    S: ResourceStorage + Send + Sync + 'static,
{
    Router::new()
        .route(
            "/console/metrics/traffic",
            get(console_metrics::traffic_handler::<S>),
        )
        .route(
            "/console/metrics/tenants",
            get(console_metrics::tenants_handler::<S>),
        )
        .with_state(state)
}
