//! FHIR route configuration.
//!
//! Defines all routes for the FHIR RESTful API, supporting multiple
//! tenant routing modes.

use axum::{
    Router,
    body::Body,
    extract::Request,
    routing::{delete, get, head, patch, post, put},
};
use helios_fhir::FhirVersion;
use helios_persistence::core::{
    BundleProvider, ConditionalStorage, IncludeProvider, InstanceHistoryProvider, ResourceStorage,
    RevincludeProvider, SearchProvider, SystemHistoryProvider, TypeHistoryProvider,
};
use tower::ServiceExt;

use crate::config::TenantRoutingMode;
use crate::handlers;
use crate::middleware::tenant_prefix::{
    ExtractedTenantFromUrl, OriginalPath, extract_tenant_from_path,
};
use crate::state::AppState;

/// Creates all FHIR REST API routes based on tenant routing configuration.
///
/// # Routing Modes
///
/// - `HeaderOnly` (default): Standard routes, tenant from X-Tenant-ID header
/// - `UrlPath`: Routes accept `/{tenant}/...` prefix, tenant extracted from URL
/// - `Both`: Both URL prefix and header supported; URL takes precedence
///
/// # Routes
///
/// ## System-level
/// - `GET /metadata` - CapabilityStatement
/// - `GET /$versions` - Supported FHIR versions
/// - `GET /health` - Health check
/// - `GET /_history` - System history
/// - `POST /` - Batch/Transaction
///
/// ## Type-level
/// - `GET /{type}` - Search
/// - `POST /{type}` - Create
/// - `POST /{type}/_search` - Search (POST)
/// - `GET /{type}/_history` - Type history
///
/// ## Instance-level
/// - `GET /{type}/{id}` - Read
/// - `PUT /{type}/{id}` - Update
/// - `PATCH /{type}/{id}` - Patch
/// - `DELETE /{type}/{id}` - Delete
/// - `GET /{type}/{id}/_history` - Instance history
/// - `GET /{type}/{id}/_history/{vid}` - Version read
pub fn create_routes<S>(state: AppState<S>) -> Router
where
    S: ResourceStorage
        + ConditionalStorage
        + SearchProvider
        + IncludeProvider
        + RevincludeProvider
        + InstanceHistoryProvider
        + TypeHistoryProvider
        + SystemHistoryProvider
        + BundleProvider
        + helios_persistence::core::ExportDataProvider
        + helios_persistence::core::PatientExportProvider
        + helios_persistence::core::GroupExportProvider
        + Send
        + Sync
        + 'static,
{
    match state.config().multitenancy.routing_mode {
        TenantRoutingMode::HeaderOnly => create_standard_routes(state),
        TenantRoutingMode::UrlPath => create_url_tenant_routes(state),
        TenantRoutingMode::Both => create_combined_routes(state),
    }
}

/// Creates standard routes (header-only tenant identification).
fn create_standard_routes<S>(state: AppState<S>) -> Router
where
    S: ResourceStorage
        + ConditionalStorage
        + SearchProvider
        + IncludeProvider
        + RevincludeProvider
        + InstanceHistoryProvider
        + TypeHistoryProvider
        + SystemHistoryProvider
        + BundleProvider
        + helios_persistence::core::ExportDataProvider
        + helios_persistence::core::PatientExportProvider
        + helios_persistence::core::GroupExportProvider
        + Send
        + Sync
        + 'static,
{
    create_fhir_router().with_state(state)
}

/// Creates routes with URL-based tenant identification.
///
/// Uses a request mapping layer to strip tenant prefix from URL paths BEFORE
/// route matching. The tenant is stored in request extensions.
fn create_url_tenant_routes<S>(state: AppState<S>) -> Router
where
    S: ResourceStorage
        + ConditionalStorage
        + SearchProvider
        + IncludeProvider
        + RevincludeProvider
        + InstanceHistoryProvider
        + TypeHistoryProvider
        + SystemHistoryProvider
        + BundleProvider
        + helios_persistence::core::ExportDataProvider
        + helios_persistence::core::PatientExportProvider
        + helios_persistence::core::GroupExportProvider
        + Send
        + Sync
        + 'static,
{
    let router = create_fhir_router().with_state(state);

    // Use tower's map_request to modify the request BEFORE routing
    let service = router.map_request(strip_tenant_prefix);

    Router::new().fallback_service(service)
}

/// Creates combined routes supporting both header and URL-based tenants.
///
/// URL-based routes take precedence. Uses request mapping to optionally strip
/// tenant prefix from URL paths.
fn create_combined_routes<S>(state: AppState<S>) -> Router
where
    S: ResourceStorage
        + ConditionalStorage
        + SearchProvider
        + IncludeProvider
        + RevincludeProvider
        + InstanceHistoryProvider
        + TypeHistoryProvider
        + SystemHistoryProvider
        + BundleProvider
        + helios_persistence::core::ExportDataProvider
        + helios_persistence::core::PatientExportProvider
        + helios_persistence::core::GroupExportProvider
        + Send
        + Sync
        + 'static,
{
    let router = create_fhir_router().with_state(state);

    // Use tower's map_request to modify the request BEFORE routing
    let service = router.map_request(strip_tenant_prefix);

    Router::new().fallback_service(service)
}

/// Strips tenant prefix from request URL and stores it in extensions.
fn strip_tenant_prefix(mut request: Request<Body>) -> Request<Body> {
    let path = request.uri().path().to_string();

    // Use the default FHIR version for resource type checking
    let fhir_version = FhirVersion::default_enabled();

    if let Some((tenant, remaining_path)) = extract_tenant_from_path(&path, &fhir_version) {
        // Store original path and extracted tenant in extensions
        request.extensions_mut().insert(OriginalPath(path));
        request
            .extensions_mut()
            .insert(ExtractedTenantFromUrl(tenant));

        // Build new URI with remaining path
        let new_uri = build_uri_with_new_path(request.uri(), &remaining_path);
        *request.uri_mut() = new_uri;
    }

    request
}

/// Builds a new URI with a different path but same query/fragment.
fn build_uri_with_new_path(original: &axum::http::Uri, new_path: &str) -> axum::http::Uri {
    let mut parts = original.clone().into_parts();

    // Build path-and-query
    let path_and_query = if let Some(query) = original.query() {
        format!("{}?{}", new_path, query)
    } else {
        new_path.to_string()
    };

    parts.path_and_query = Some(
        path_and_query
            .parse()
            .unwrap_or_else(|_| new_path.parse().unwrap()),
    );

    axum::http::Uri::from_parts(parts).unwrap_or_else(|_| original.clone())
}

/// Creates the core FHIR router with all endpoints.
fn create_fhir_router<S>() -> Router<AppState<S>>
where
    S: ResourceStorage
        + ConditionalStorage
        + SearchProvider
        + IncludeProvider
        + RevincludeProvider
        + InstanceHistoryProvider
        + TypeHistoryProvider
        + SystemHistoryProvider
        + BundleProvider
        + helios_persistence::core::ExportDataProvider
        + helios_persistence::core::PatientExportProvider
        + helios_persistence::core::GroupExportProvider
        + Send
        + Sync
        + 'static,
{
    let router = Router::new()
        // System-level routes
        .route("/metadata", get(handlers::capabilities_handler::<S>))
        .route("/$versions", get(handlers::versions_handler::<S>))
        .route("/health", get(handlers::health_handler::<S>))
        .route("/_liveness", get(handlers::health::liveness_handler))
        .route("/_readiness", get(handlers::health::readiness_handler::<S>))
        .route(
            "/.well-known/smart-configuration",
            get(handlers::smart_discovery::smart_configuration_handler::<S>),
        )
        .route("/_history", get(handlers::history_system_handler::<S>))
        .route("/", post(handlers::batch_handler::<S>))
        // Bulk Data Export ($export) — operation routes precede the catch-all.
        .route(
            "/$export",
            get(handlers::system_export_kickoff_handler::<S>)
                .post(handlers::system_export_kickoff_handler::<S>),
        )
        .route(
            "/Patient/$export",
            get(handlers::patient_export_kickoff_handler::<S>)
                .post(handlers::patient_export_kickoff_handler::<S>),
        )
        .route(
            "/Group/{id}/$export",
            get(handlers::group_export_kickoff_handler::<S>)
                .post(handlers::group_export_kickoff_handler::<S>),
        )
        .route(
            "/export-status/{job_id}",
            get(handlers::export_status_handler::<S>).delete(handlers::export_cancel_handler::<S>),
        )
        .route(
            "/export-file/{job_id}/{part}",
            get(handlers::export_download_handler::<S>),
        )
        // Bulk Data Submit ($bulk-submit) — operation routes precede the catch-all.
        .route(
            "/$bulk-submit",
            post(handlers::bulk_submit_kickoff_handler::<S>),
        )
        .route(
            "/$bulk-submit-status",
            post(handlers::bulk_submit_status_kickoff_handler::<S>),
        )
        .route(
            "/bulk-submit-status/{poll_token}",
            get(handlers::bulk_submit_poll_handler::<S>)
                .delete(handlers::bulk_submit_cancel_handler::<S>),
        )
        .route(
            "/bulk-submit-file/{poll_token}/{part}",
            get(handlers::bulk_submit_file_handler::<S>),
        )
        // Type-level routes
        .route("/{resource_type}", get(handlers::search_get_handler::<S>))
        .route("/{resource_type}", post(handlers::create_handler::<S>))
        // Conditional update: PUT [base]/[type]?[search-params]
        .route(
            "/{resource_type}",
            put(handlers::conditional_update_handler::<S>),
        )
        // Conditional delete: DELETE [base]/[type]?[search-params]
        .route(
            "/{resource_type}",
            delete(handlers::conditional_delete_handler::<S>),
        )
        .route(
            "/{resource_type}/_search",
            post(handlers::search_post_handler::<S>),
        )
        .route(
            "/{resource_type}/_history",
            get(handlers::history_type_handler::<S>),
        )
        // Instance-level routes
        .route("/{resource_type}/{id}", get(handlers::read_handler::<S>))
        // HEAD for read - returns headers without body
        .route(
            "/{resource_type}/{id}",
            head(handlers::head_read_handler::<S>),
        )
        .route("/{resource_type}/{id}", put(handlers::update_handler::<S>))
        .route("/{resource_type}/{id}", patch(handlers::patch_handler::<S>))
        .route(
            "/{resource_type}/{id}",
            delete(handlers::delete_handler::<S>),
        )
        .route(
            "/{resource_type}/{id}/_history",
            get(handlers::history_instance_handler::<S>),
        )
        // Delete instance history (FHIR v6.0.0 Trial Use)
        .route(
            "/{resource_type}/{id}/_history",
            delete(handlers::delete_instance_history_handler::<S>),
        )
        .route(
            "/{resource_type}/{id}/_history/{version_id}",
            get(handlers::vread_handler::<S>),
        )
        // Delete specific version (FHIR v6.0.0 Trial Use)
        .route(
            "/{resource_type}/{id}/_history/{version_id}",
            delete(handlers::delete_version_handler::<S>),
        );

    // Subscription operations (feature-gated, before compartment search)
    #[cfg(feature = "subscriptions")]
    let router = router
        .route(
            "/{resource_type}/{id}/$status",
            get(handlers::subscriptions::subscription_status_handler::<S>),
        )
        .route(
            "/{resource_type}/{id}/$events",
            get(handlers::subscriptions::subscription_events_handler::<S>),
        )
        .route(
            "/{resource_type}/{id}/$get-ws-binding-token",
            get(handlers::subscriptions::get_ws_binding_token_handler::<S>),
        )
        .route(
            "/ws/subscriptions/bind",
            get(handlers::ws::ws_bind_handler::<S>),
        );

    // Compartment search: GET [base]/[compartment-type]/[id]/[target-type]?params
    router
        .route(
            "/{compartment_type}/{compartment_id}/{target_type}",
            get(handlers::compartment_search_handler::<S>),
        )
        // SQL-on-FHIR operations
        .merge(create_sof_routes::<S>())
}

/// Creates SQL-on-FHIR operation routes.
fn create_sof_routes<S>() -> Router<AppState<S>>
where
    S: SearchProvider
        + ConditionalStorage
        + InstanceHistoryProvider
        + BundleProvider
        + ResourceStorage
        + Send
        + Sync
        + 'static,
{
    Router::new()
        // SQL-on-FHIR capabilities: GET /$sql-on-fhir-capabilities
        .route(
            "/$sql-on-fhir-capabilities",
            get(handlers::sof::sof_capabilities_handler::<S>),
        )
        // Run (system level): POST/GET /$viewdefinition-run
        // Spec lists system-level invocation at [base]/$viewdefinition-run
        // with no resource-type prefix, matching the export and sqlquery-run
        // operations.
        .route(
            "/$viewdefinition-run",
            post(handlers::sof::run_view_definition_handler::<S>)
                .get(handlers::sof::run_view_definition_handler::<S>),
        )
        // Anonymous run (type level): POST /ViewDefinition/$viewdefinition-run
        // GET is permitted per spec when the ViewDefinition is supplied via
        // `viewReference` query parameter (no `viewResource`/`resource` body).
        .route(
            "/ViewDefinition/$viewdefinition-run",
            post(handlers::sof::run_view_definition_handler::<S>)
                .get(handlers::sof::run_view_definition_handler::<S>),
        )
        // Instance run: POST /ViewDefinition/{id}/$viewdefinition-run
        // GET infers the ViewDefinition id from the URL path.
        .route(
            "/ViewDefinition/{id}/$viewdefinition-run",
            post(handlers::sof::run_stored_view_definition_handler::<S>)
                .get(handlers::sof::run_stored_view_definition_handler::<S>),
        )
        // Export (system level): POST /$viewdefinition-export
        // Spec defines this operation at all three levels (system, type,
        // instance); system-level lets callers submit multi-view exports
        // without nesting under /ViewDefinition.
        .route(
            "/$viewdefinition-export",
            post(handlers::sof::export_view_definition_handler::<S>),
        )
        // Export (type level): POST /ViewDefinition/$viewdefinition-export
        .route(
            "/ViewDefinition/$viewdefinition-export",
            post(handlers::sof::export_view_definition_handler::<S>),
        )
        // Export (instance level): POST /ViewDefinition/{id}/$viewdefinition-export
        .route(
            "/ViewDefinition/{id}/$viewdefinition-export",
            post(handlers::sof::export_stored_view_definition_handler::<S>),
        )
        // Export status: GET /export/{job-id}/status
        // (DELETE on the same URL cancels the job, per spec)
        .route(
            "/export/{job_id}/status",
            get(handlers::sof::get_export_status_handler::<S>)
                .delete(handlers::sof::cancel_export_handler::<S>),
        )
        // Export download: GET /export/{job-id}/{filename}
        // (The completion manifest is served by the status endpoint with
        // 200 OK per the spec's async pattern — there is no separate
        // /result route.)
        .route(
            "/export/{job_id}/{filename}",
            get(handlers::sof::download_export_file_handler::<S>),
        )
        // SQL-on-FHIR v2 `$sqlquery-run` — system, type, and instance levels.
        .route(
            "/$sqlquery-run",
            post(handlers::sof::sqlquery_run_handler::<S>),
        )
        .route(
            "/Library/$sqlquery-run",
            post(handlers::sof::sqlquery_run_handler::<S>),
        )
        .route(
            "/Library/{id}/$sqlquery-run",
            post(handlers::sof::sqlquery_run_instance_handler::<S>),
        )
        // SQL-on-FHIR v2 `$sqlquery-export` — async export of SQL query
        // results; system, type, and instance levels. Shares the
        // /export/{job-id}/* status, cancel, and download routes with
        // `$viewdefinition-export`.
        .route(
            "/$sqlquery-export",
            post(handlers::sof::sqlquery_export_handler::<S>),
        )
        .route(
            "/Library/$sqlquery-export",
            post(handlers::sof::sqlquery_export_handler::<S>),
        )
        .route(
            "/Library/{id}/$sqlquery-export",
            post(handlers::sof::sqlquery_export_stored_handler::<S>),
        )
}

/// Creates a minimal set of routes for testing.
///
/// This is useful for integration tests that only need a subset
/// of functionality.
pub fn create_minimal_routes<S>(state: AppState<S>) -> Router
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    Router::new()
        .route("/health", get(handlers::health_handler::<S>))
        .route("/metadata", get(handlers::capabilities_handler::<S>))
        .route("/{resource_type}/{id}", get(handlers::read_handler::<S>))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    // Route tests will be in integration tests
}
