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
use helios_auth::{
    AuditEventSink, AuthConfig, AuthError, AuthProvider, FhirOperation, Principal, SmartScopePolicy,
};
use tracing::{debug, warn};

/// Shared state for the auth middleware layers.
pub struct AuthMiddlewareState {
    /// The token validation provider.
    pub provider: Arc<dyn AuthProvider>,
    /// Auth configuration.
    pub config: Arc<AuthConfig>,
    /// Audit event sink.
    pub audit_sink: Arc<dyn AuditEventSink>,
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
            auth_state
                .audit_sink
                .record_auth_failure(&AuthError::MissingToken, &path, &method)
                .await;
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
            auth_state
                .audit_sink
                .record_auth_success(&principal, &path, &method)
                .await;
            request.extensions_mut().insert(principal);
            next.run(request).await
        }
        Err(err) => {
            warn!(error = %err, path = %path, "Authentication failed");
            auth_state
                .audit_sink
                .record_auth_failure(&err, &path, &method)
                .await;
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

    // Determine the FHIR operation and resource type from the path
    if let Some((resource_type, operation)) = extract_operation(&path, method.as_str()) {
        match SmartScopePolicy::check(&principal, &resource_type, operation) {
            Ok(()) => {
                debug!(
                    sub = %principal.subject(),
                    resource_type = %resource_type,
                    operation = %operation,
                    "Authorization granted"
                );
                auth_state
                    .audit_sink
                    .record_authz_grant(&principal, &resource_type, &operation.to_string())
                    .await;
            }
            Err(err) => {
                warn!(
                    sub = %principal.subject(),
                    resource_type = %resource_type,
                    operation = %operation,
                    "Authorization denied"
                );
                auth_state
                    .audit_sink
                    .record_authz_denial(&principal, &resource_type, &operation.to_string())
                    .await;
                return forbidden_response(&err.to_string());
            }
        }
    }

    next.run(request).await
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

    // System-level paths
    let first = segments[0];
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
}
