//! Authentication and authorization middleware.
//!
//! Two middleware layers work together:
//!
//! 1. [`auth_middleware`] — Validates Bearer tokens and injects [`Principal`]
//!    into request extensions. Returns 401 for invalid/missing tokens.
//!
//! 2. [`authz_middleware`] — Checks the [`Principal`]'s SMART scopes against
//!    the requested FHIR operation. Returns 403 for insufficient scopes.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use helios_audit::{AuditAction, AuditAgent, AuditEventBuilder, AuditSink, ExclusionFilter};
use helios_auth::{
    AuthConfig, AuthError, AuthProvider, FhirOperation, Principal, SmartPermissions,
    SmartScopePolicy,
};
use helios_fhir::FhirVersion;
use tracing::{debug, warn};

use crate::middleware::tenant_prefix::extract_tenant_from_path;

/// Shared state for the auth middleware layers.
pub struct AuthMiddlewareState {
    /// The token validation provider.
    pub provider: Arc<dyn AuthProvider>,
    /// Auth configuration.
    pub config: Arc<AuthConfig>,
    /// Audit event sink.
    pub audit_sink: Arc<dyn AuditSink>,
    /// Source observer reference for audit events.
    pub audit_source_observer: String,
    /// Exclusion filter for audit events.
    pub audit_exclusion_filter: ExclusionFilter,
    /// Whether URL-path tenant routing (`url_path`/`both`) is enabled.
    ///
    /// When set, the tenant prefix is stripped from the request path *inside*
    /// the FHIR router, which runs downstream of the authorization layer — so
    /// `authz_middleware` sees the un-stripped `/{tenant}/{type}/{id}` path and
    /// must strip the leading tenant segment itself before classifying the
    /// operation. In `header_only` mode this is `false` and the path is used
    /// verbatim.
    pub tenant_url_routing: bool,
}

/// Paths that are exempt from authentication.
const EXEMPT_PATHS: &[&str] = &[
    "/metadata",
    "/health",
    "/_liveness",
    "/_readiness",
    "/.well-known/smart-configuration",
    "/$versions",
];

/// Check if a request path is exempt from authentication.
///
/// Only exact matches are allowed to prevent path traversal bypasses
/// (e.g., `/Patient/metadata` must NOT be treated as exempt).
fn is_exempt_path(path: &str) -> bool {
    let path = path.trim_end_matches('/');
    EXEMPT_PATHS.contains(&path)
}

/// Authentication middleware.
///
/// Validates the `Authorization: Bearer <token>` header using the configured
/// [`AuthProvider`]. On success, inserts a [`Principal`] into request extensions.
/// On failure, returns a 401 Unauthorized response with an OperationOutcome.
///
/// Exempt paths (health, metadata, SMART discovery) bypass authentication.
pub async fn auth_middleware(
    State(auth_state): State<Arc<AuthMiddlewareState>>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    let method = request.method().to_string();

    // Skip auth for exempt paths
    if is_exempt_path(&path) {
        debug!(path = %path, "Exempt path, skipping authentication");
        return next.run(request).await;
    }

    // Extract Authorization header
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let auth_header = match auth_header {
        Some(h) => h.to_string(),
        None => {
            if !auth_state
                .audit_exclusion_filter
                .is_excluded(&path, &method)
            {
                let event = AuditEventBuilder::new(&auth_state.audit_source_observer)
                    .action(AuditAction::Execute)
                    .outcome("8")
                    .outcome_desc(AuthError::MissingToken.to_string())
                    .build();
                auth_state.audit_sink.record(event).await;
            }
            return unauthorized_response("Missing Authorization header");
        }
    };

    // Validate token
    match auth_state.provider.authenticate(&auth_header).await {
        Ok(principal) => {
            debug!(
                sub = %principal.subject(),
                iss = %principal.issuer(),
                "Authentication successful"
            );
            if !auth_state
                .audit_exclusion_filter
                .is_excluded(&path, &method)
            {
                let event = AuditEventBuilder::new(&auth_state.audit_source_observer)
                    .action(AuditAction::Execute)
                    .outcome("0")
                    .agent(principal.subject(), None, true)
                    .build();
                auth_state.audit_sink.record(event).await;
            }
            request
                .extensions_mut()
                .insert(AuditAgent(principal.subject().to_string()));
            request.extensions_mut().insert(principal);
            next.run(request).await
        }
        Err(err) => {
            warn!(error = %err, path = %path, "Authentication failed");
            if !auth_state
                .audit_exclusion_filter
                .is_excluded(&path, &method)
            {
                let event = AuditEventBuilder::new(&auth_state.audit_source_observer)
                    .action(AuditAction::Execute)
                    .outcome("8")
                    .outcome_desc(err.to_string())
                    .build();
                auth_state.audit_sink.record(event).await;
            }
            unauthorized_response(&err.to_string())
        }
    }
}

/// Authorization middleware.
///
/// Checks the [`Principal`]'s SMART scopes against the requested FHIR operation
/// and resource type. Returns 403 Forbidden if the principal lacks sufficient scopes.
///
/// If no [`Principal`] is present in extensions (exempt path or auth disabled),
/// the request passes through.
pub async fn authz_middleware(
    State(auth_state): State<Arc<AuthMiddlewareState>>,
    request: Request,
    next: Next,
) -> Response {
    // If no principal, pass through (exempt path or auth disabled)
    let principal = match request.extensions().get::<Principal>() {
        Some(p) => p.clone(),
        None => return next.run(request).await,
    };

    let path = request.uri().path().to_string();
    let method = request.method().clone();

    // Determine the FHIR operation and resource type from the path. Under
    // URL-path tenant routing the tenant prefix has not been stripped yet (that
    // happens downstream, inside the FHIR router), so strip it here first.
    if let Some((resource_type, operation)) =
        extract_operation_for_routing(&path, method.as_str(), auth_state.tenant_url_routing)
    {
        match SmartScopePolicy::check(&principal, &resource_type, operation) {
            Ok(()) => {
                debug!(
                    sub = %principal.subject(),
                    resource_type = %resource_type,
                    operation = %operation,
                    "Authorization granted"
                );
                let event = AuditEventBuilder::new(&auth_state.audit_source_observer)
                    .action(AuditAction::Execute)
                    .outcome("0")
                    .outcome_desc(format!("Granted: {operation} on {resource_type}"))
                    .agent(principal.subject(), None, true)
                    .resource(&resource_type, "")
                    .build();
                auth_state.audit_sink.record(event).await;
            }
            Err(err) => {
                warn!(
                    sub = %principal.subject(),
                    resource_type = %resource_type,
                    operation = %operation,
                    "Authorization denied"
                );
                let event = AuditEventBuilder::new(&auth_state.audit_source_observer)
                    .action(AuditAction::Execute)
                    .outcome("8")
                    .outcome_desc(format!("Forbidden: {operation} on {resource_type}"))
                    .agent(principal.subject(), None, true)
                    .resource(&resource_type, "")
                    .build();
                auth_state.audit_sink.record(event).await;
                return forbidden_response(&err.to_string());
            }
        }
    }

    next.run(request).await
}

/// Authorization middleware for the console's **cross-tenant** admin endpoints
/// (`/console/metrics/tenants`, `/console/metrics/traffic`).
///
/// These surface data spanning every tenant — the full tenant roster and sizes,
/// server-wide traffic — so, unlike the per-tenant console endpoints, they must
/// not be reachable by ordinary user- or patient-context tokens. This layer
/// requires a **system-context, all-resource** scope (`system/*.r`); anything
/// less is rejected `403`.
///
/// Consistent with [`authz_middleware`], if no [`Principal`] is present (auth
/// disabled server-wide, or an exempt path) the request passes through, so this
/// tier stays open in the same dev-mode configurations as every other route.
pub async fn admin_authz_middleware(
    State(auth_state): State<Arc<AuthMiddlewareState>>,
    request: Request,
    next: Next,
) -> Response {
    // If no principal, pass through (auth disabled). Matches authz_middleware.
    let principal = match request.extensions().get::<Principal>() {
        Some(p) => p.clone(),
        None => return next.run(request).await,
    };

    let path = request.uri().path().to_string();

    if principal.scopes.has_system_scope(SmartPermissions::READ) {
        debug!(
            sub = %principal.subject(),
            path = %path,
            "Console admin authorization granted"
        );
        let event = AuditEventBuilder::new(&auth_state.audit_source_observer)
            .action(AuditAction::Execute)
            .outcome("0")
            .outcome_desc(format!("Granted: console admin access to {path}"))
            .agent(principal.subject(), None, true)
            .build();
        auth_state.audit_sink.record(event).await;
        next.run(request).await
    } else {
        warn!(
            sub = %principal.subject(),
            path = %path,
            "Console admin authorization denied: system-level scope required"
        );
        let event = AuditEventBuilder::new(&auth_state.audit_source_observer)
            .action(AuditAction::Execute)
            .outcome("8")
            .outcome_desc(format!(
                "Forbidden: console admin access to {path} requires a system-level scope"
            ))
            .agent(principal.subject(), None, true)
            .build();
        auth_state.audit_sink.record(event).await;
        forbidden_response("This endpoint requires a system-level scope (system/*.read)")
    }
}

/// Classify the FHIR operation for authorization, accounting for URL-path
/// tenant routing.
///
/// When `tenant_url_routing` is set (routing mode `url_path`/`both`), the
/// request path still carries its `/{tenant}` prefix at this layer — the router
/// strips it further downstream. Left in place, that leading segment shifts
/// every other segment right, so `extract_operation` would authorize
/// `/{tenant}/Patient/123` as `Search` on type `123` and `/{tenant}/Patient` as
/// `Read`/`Create` on type `{tenant}` — the wrong resource type entirely.
///
/// We therefore re-classify against the tenant-stripped path, reusing the exact
/// same [`extract_tenant_from_path`] logic the router uses (so reserved paths
/// and real FHIR resource types are never mistaken for tenants). System/console
/// paths that [`extract_operation`] already declines on the raw path (returning
/// `None`) are honoured as-is and never stripped — notably `/console/metrics/*`,
/// which the router serves verbatim rather than as a tenant-prefixed route.
///
/// In `header_only` mode `tenant_url_routing` is `false` and the raw path is
/// used verbatim, leaving behaviour byte-for-byte unchanged.
fn extract_operation_for_routing(
    path: &str,
    method: &str,
    tenant_url_routing: bool,
) -> Option<(String, FhirOperation)> {
    // Strip a URL-path tenant prefix BEFORE classifying, mirroring the router.
    // Classifying the raw path first would wrongly `None`-out (and thus skip the
    // scope check for) a path whose tenant segment happens to look system-like to
    // `extract_operation` — e.g. `/_x/Patient/123`, where `_x` is a valid,
    // non-reserved tenant the router strips, yet `extract_operation` sees the
    // leading `_` and returns `None`. `extract_tenant_from_path` returns `None`
    // for reserved first segments (real FHIR resource types, `metadata`,
    // `_history`, `console`, …), so those fall through to the raw classification
    // and behave exactly as before.
    if tenant_url_routing {
        if let Some((_tenant, remaining)) =
            extract_tenant_from_path(path, &FhirVersion::default_enabled())
        {
            // Classify against the tenant-stripped path the router will actually
            // dispatch to.
            return extract_operation(&remaining, method);
        }
    }

    extract_operation(path, method)
}

/// Extract the FHIR resource type and operation from a request path and method.
///
/// Returns `None` for system-level operations (batch, history) where
/// per-resource-type authorization doesn't apply.
fn extract_operation(path: &str, method: &str) -> Option<(String, FhirOperation)> {
    // Split path segments, filtering empty
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if segments.is_empty() {
        // POST / (batch/transaction) — authorization deferred to batch handler
        return None;
    }

    let first = segments[0];

    // The management console (`/console/metrics/*`) is not a FHIR resource
    // operation — mapping it onto a FHIR resource type here would authorize it
    // against a bogus type (e.g. Search on "tenants"), which is meaningless and,
    // worse, admits any wildcard-search token. Its cross-tenant admin routes are
    // gated separately by `admin_authz_middleware`. Scoped narrowly to the
    // `console/metrics` namespace so it cannot shadow a tenant named "console"
    // under URL-path tenant routing (where authz sees the un-stripped path).
    if first == "console" && segments.get(1) == Some(&"metrics") {
        return None;
    }

    // System-level paths
    if first.starts_with('_') || first.starts_with('$') || first == "metadata" || first == "health"
    {
        return None;
    }

    // The first segment is the resource type (or tenant — handled by prefix stripping)
    let resource_type = first.to_string();

    // Detect compartment search: GET /{compartment_type}/{id}/{target_type}
    // The third segment is the actual resource type being accessed, so the
    // authorization check must be against the target type, not the compartment.
    if segments.len() == 3
        && matches!(method, "GET" | "HEAD")
        && !segments[2].starts_with('_')
        && !segments[2].starts_with('$')
    {
        let target_type = segments[2].to_string();
        return Some((target_type, FhirOperation::Search));
    }

    // Determine operation from method and path structure
    let operation = match method {
        "GET" | "HEAD" => {
            if segments.len() == 1 {
                // GET /Patient → search
                FhirOperation::Search
            } else if segments.len() >= 2 && segments.get(1).is_some_and(|s| s.starts_with('_')) {
                // GET /Patient/_history → read (history)
                FhirOperation::Read
            } else {
                // GET /Patient/123 → read
                FhirOperation::Read
            }
        }
        "POST" => {
            if segments.len() >= 2 && segments.get(1) == Some(&"_search") {
                FhirOperation::Search
            } else {
                // POST /Patient → create
                FhirOperation::Create
            }
        }
        "PUT" => FhirOperation::Update,
        "PATCH" => FhirOperation::Update,
        "DELETE" => FhirOperation::Delete,
        _ => return None,
    };

    Some((resource_type, operation))
}

fn unauthorized_response(message: &str) -> Response {
    let body = serde_json::json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "error",
            "code": "login",
            "details": { "text": message }
        }]
    });

    (
        StatusCode::UNAUTHORIZED,
        [(
            header::WWW_AUTHENTICATE,
            axum::http::HeaderValue::from_static("Bearer"),
        )],
        Json(body),
    )
        .into_response()
}

fn forbidden_response(message: &str) -> Response {
    let body = serde_json::json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "error",
            "code": "forbidden",
            "details": { "text": message }
        }]
    });

    (StatusCode::FORBIDDEN, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exempt_paths() {
        assert!(is_exempt_path("/metadata"));
        assert!(is_exempt_path("/health"));
        assert!(is_exempt_path("/_liveness"));
        assert!(is_exempt_path("/_readiness"));
        assert!(is_exempt_path("/.well-known/smart-configuration"));
        assert!(is_exempt_path("/$versions"));

        assert!(!is_exempt_path("/Patient"));
        assert!(!is_exempt_path("/Patient/123"));
        assert!(!is_exempt_path("/"));

        // Path traversal attempts must NOT be exempt
        assert!(!is_exempt_path("/Patient/metadata"));
        assert!(!is_exempt_path("/Observation/health"));
        assert!(!is_exempt_path("/Device/_liveness"));
        assert!(!is_exempt_path("/Patient/_readiness"));
        assert!(!is_exempt_path("/Patient/.well-known/smart-configuration"));
        assert!(!is_exempt_path("/tenant/metadata"));
        assert!(!is_exempt_path("/tenant/$versions"));
    }

    #[test]
    fn test_extract_operation_search() {
        let (rt, op) = extract_operation("/Patient", "GET").unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(op, FhirOperation::Search);
    }

    #[test]
    fn test_extract_operation_read() {
        let (rt, op) = extract_operation("/Patient/123", "GET").unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(op, FhirOperation::Read);
    }

    #[test]
    fn test_extract_operation_create() {
        let (rt, op) = extract_operation("/Patient", "POST").unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(op, FhirOperation::Create);
    }

    #[test]
    fn test_extract_operation_update() {
        let (rt, op) = extract_operation("/Patient/123", "PUT").unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(op, FhirOperation::Update);
    }

    #[test]
    fn test_extract_operation_delete() {
        let (rt, op) = extract_operation("/Patient/123", "DELETE").unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(op, FhirOperation::Delete);
    }

    #[test]
    fn test_extract_operation_search_post() {
        let (rt, op) = extract_operation("/Patient/_search", "POST").unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(op, FhirOperation::Search);
    }

    #[test]
    fn test_extract_operation_batch() {
        assert!(extract_operation("/", "POST").is_none());
    }

    #[test]
    fn test_extract_operation_system_history() {
        assert!(extract_operation("/_history", "GET").is_none());
    }

    #[test]
    fn test_extract_operation_console_is_not_a_fhir_op() {
        // Console endpoints must not be mapped onto FHIR resource operations —
        // otherwise `/console/metrics/tenants` would be authorized as a Search on
        // resource type "tenants", admitting any wildcard-search token. They are
        // gated separately by `admin_authz_middleware`.
        assert!(extract_operation("/console/metrics/tenants", "GET").is_none());
        assert!(extract_operation("/console/metrics/traffic", "GET").is_none());
        assert!(extract_operation("/console/metrics/resource-counts", "GET").is_none());
        assert!(extract_operation("/console/metrics/uptime", "GET").is_none());
    }

    #[test]
    fn test_extract_operation_metadata() {
        assert!(extract_operation("/metadata", "GET").is_none());
    }

    #[test]
    fn test_extract_operation_compartment_search() {
        // GET /Patient/123/Observation → authz must check Observation (target), not Patient
        let (rt, op) = extract_operation("/Patient/123/Observation", "GET").unwrap();
        assert_eq!(rt, "Observation");
        assert_eq!(op, FhirOperation::Search);
    }

    #[test]
    fn test_extract_operation_compartment_search_different_types() {
        let (rt, op) = extract_operation("/Encounter/456/Procedure", "GET").unwrap();
        assert_eq!(rt, "Procedure");
        assert_eq!(op, FhirOperation::Search);
    }

    #[test]
    fn test_extract_operation_instance_history_not_compartment() {
        // GET /Patient/123/_history has 3 segments but third starts with '_'
        // so it should NOT be treated as compartment search
        let (rt, op) = extract_operation("/Patient/123/_history", "GET").unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(op, FhirOperation::Read);
    }

    // ── URL-path tenant routing: authz must classify the TENANT-STRIPPED path ──

    #[test]
    fn test_url_routing_read_strips_tenant() {
        // GET /{tenant}/Patient/123 must authorize Read on Patient, NOT the id.
        let (rt, op) = extract_operation_for_routing("/acme/Patient/123", "GET", true).unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(op, FhirOperation::Read);
    }

    #[test]
    fn test_url_routing_underscore_tenant_strips_and_classifies() {
        // Regression: an authenticated caller sends GET /_x/Patient/123 under
        // URL-path routing. `_x` is a valid, non-reserved tenant that the router
        // strips, but the leading `_` makes `extract_operation` decline the raw
        // path (`None`). Classifying the raw path first would skip the scope
        // check entirely — a full SMART scope-enforcement bypass. Stripping first
        // must yield Read Patient so the scope check runs.
        let (rt, op) = extract_operation_for_routing("/_x/Patient/123", "GET", true).unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(op, FhirOperation::Read);
    }

    #[test]
    fn test_url_routing_search_strips_tenant() {
        // GET /{tenant}/Patient must authorize Search on Patient, NOT the tenant.
        let (rt, op) = extract_operation_for_routing("/acme/Patient", "GET", true).unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(op, FhirOperation::Search);
    }

    #[test]
    fn test_url_routing_create_strips_tenant() {
        let (rt, op) = extract_operation_for_routing("/acme/Patient", "POST", true).unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(op, FhirOperation::Create);
    }

    #[test]
    fn test_url_routing_update_strips_tenant() {
        let (rt, op) = extract_operation_for_routing("/acme/Patient/123", "PUT", true).unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(op, FhirOperation::Update);
    }

    #[test]
    fn test_url_routing_compartment_search_strips_tenant() {
        // GET /{tenant}/Patient/123/Observation → Search on the target type.
        let (rt, op) =
            extract_operation_for_routing("/acme/Patient/123/Observation", "GET", true).unwrap();
        assert_eq!(rt, "Observation");
        assert_eq!(op, FhirOperation::Search);
    }

    #[test]
    fn test_url_routing_untenanted_path_unchanged() {
        // A resource-type first segment is not a tenant, so nothing is stripped.
        let (rt, op) = extract_operation_for_routing("/Patient/123", "GET", true).unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(op, FhirOperation::Read);
    }

    #[test]
    fn test_url_routing_console_not_stripped() {
        // /console/metrics/* is served verbatim by the router (never as a tenant
        // prefix) and must stay unauthorized-as-FHIR even with URL routing on.
        assert!(extract_operation_for_routing("/console/metrics/uptime", "GET", true).is_none());
        assert!(extract_operation_for_routing("/console/metrics/tenants", "GET", true).is_none());
        assert!(
            extract_operation_for_routing("/console/metrics/resource-counts", "GET", true)
                .is_none()
        );
    }

    #[test]
    fn test_url_routing_system_paths_none() {
        // `metadata` and `_history` are reserved first segments (never stripped as
        // tenants), so they fall through to raw classification → system-level None.
        assert!(extract_operation_for_routing("/metadata", "GET", true).is_none());
        assert!(extract_operation_for_routing("/_history", "GET", true).is_none());
    }

    #[test]
    fn test_url_routing_tenant_system_history_none() {
        // GET /{tenant}/_history strips to /_history → system-level, no per-type authz.
        assert!(extract_operation_for_routing("/acme/_history", "GET", true).is_none());
    }

    #[test]
    fn test_header_only_mode_does_not_strip() {
        // With tenant_url_routing=false, the path is used verbatim: this must be
        // identical to calling extract_operation directly (header_only unchanged).
        let (rt, op) = extract_operation_for_routing("/Patient/123", "GET", false).unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(op, FhirOperation::Read);

        // A path that *looks* tenant-prefixed is NOT stripped in header_only mode,
        // matching the raw extract_operation compartment classification.
        let (rt, op) = extract_operation_for_routing("/acme/Patient/123", "GET", false).unwrap();
        assert_eq!(rt, extract_operation("/acme/Patient/123", "GET").unwrap().0);
        assert_eq!(op, extract_operation("/acme/Patient/123", "GET").unwrap().1);

        // A leading `_`-tenant path must NOT be stripped in header_only mode: it
        // matches raw extract_operation, which declines the `_`-prefixed first
        // segment as a system path (`None`). (In url_path mode it strips — see
        // `test_url_routing_underscore_tenant_strips_and_classifies`.)
        assert!(extract_operation_for_routing("/_x/Patient/123", "GET", false).is_none());
        assert_eq!(extract_operation("/_x/Patient/123", "GET"), None);
    }
}
