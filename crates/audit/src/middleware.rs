//! Axum middleware for recording FHIR operation audit events.
//!
//! Intercepts HTTP requests, executes the handler, then records an
//! `AuditEvent` asynchronously via `tokio::spawn` (fire-and-forget).

use std::sync::Arc;

use axum::extract::State;
use axum::middleware::Next;
use axum::response::Response;
use helios_auth::principal::Principal;

use crate::balp;
use crate::builder::AuditEventBuilder;
use crate::config::AuditConfig;
use crate::exclusion::ExclusionFilter;
use crate::patient::PatientResolver;
use crate::sink::AuditSink;

/// Shared state for the audit middleware layer.
pub struct AuditMiddlewareState {
    /// The active audit sink.
    pub sink: Arc<dyn AuditSink>,
    /// Audit configuration.
    pub config: AuditConfig,
    /// Pre-built exclusion filter.
    pub exclusion_filter: ExclusionFilter,
}

/// Axum middleware function that records audit events for FHIR operations.
///
/// Register with:
/// ```rust,ignore
/// router.layer(axum::middleware::from_fn_with_state(audit_state, audit_middleware))
/// ```
pub async fn audit_middleware(
    State(state): State<Arc<AuditMiddlewareState>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();

    // Skip excluded paths
    if state.exclusion_filter.is_excluded(&path, &method) {
        return next.run(request).await;
    }

    // Extract pre-request context
    let resource_type = extract_resource_type(&path);
    let resource_id = extract_resource_id(&path);
    let query_params = request.uri().query().map(parse_query_params);
    let principal = request.extensions().get::<Principal>().cloned();

    // Execute the actual handler
    let response = next.run(request).await;
    let status = response.status().as_u16();

    // Fire-and-forget audit recording
    let sink = Arc::clone(&state.sink);
    let source_observer = state.config.source_observer.clone();

    tokio::spawn(async move {
        let action = balp::action_for_method(&method);
        let outcome = if status < 400 { "0" } else { "8" };

        let patient_ref = PatientResolver::resolve(
            resource_type.as_deref().unwrap_or(""),
            resource_id.as_deref(),
            None, // body not available post-response
            query_params.as_deref(),
        );

        let mut builder = AuditEventBuilder::new(&source_observer)
            .action(action.to_code())
            .outcome(outcome);

        if let Some(rt) = &resource_type {
            if let Some(rid) = &resource_id {
                builder = builder.resource(rt, rid);
            }
        }
        if let Some(pr) = patient_ref {
            builder = builder.patient(pr);
        }
        if let Some(p) = &principal {
            builder = builder.agent(p.subject(), None, true);
        }

        sink.record(builder.build()).await;
    });

    response
}

/// Extract the FHIR resource type from a URL path.
///
/// Handles patterns like `/Patient`, `/Patient/123`, `/Patient/123/_history/1`,
/// and tenant-prefixed paths like `/tenant-a/Patient`.
fn extract_resource_type(path: &str) -> Option<String> {
    let segments: Vec<&str> = path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    for segment in &segments {
        // Resource types start with an uppercase letter
        if segment
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase())
        {
            return Some((*segment).to_string());
        }
    }
    None
}

/// Extract the resource ID from a URL path.
///
/// Returns the segment immediately following the resource type.
fn extract_resource_id(path: &str) -> Option<String> {
    let segments: Vec<&str> = path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    let mut found_type = false;
    for segment in &segments {
        if found_type {
            // Skip special segments
            if *segment == "_history" || *segment == "_search" || segment.starts_with('$') {
                return None;
            }
            return Some((*segment).to_string());
        }
        if segment
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase())
        {
            found_type = true;
        }
    }
    None
}

/// Parse a query string into key-value pairs.
fn parse_query_params(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next()?.to_string();
            let value = parts.next().unwrap_or("").to_string();
            Some((key, value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_resource_type_simple() {
        assert_eq!(
            extract_resource_type("/Patient"),
            Some("Patient".to_string())
        );
    }

    #[test]
    fn test_extract_resource_type_with_id() {
        assert_eq!(
            extract_resource_type("/Patient/123"),
            Some("Patient".to_string())
        );
    }

    #[test]
    fn test_extract_resource_type_with_history() {
        assert_eq!(
            extract_resource_type("/Patient/123/_history/1"),
            Some("Patient".to_string())
        );
    }

    #[test]
    fn test_extract_resource_type_tenant_prefixed() {
        assert_eq!(
            extract_resource_type("/tenant-a/Patient/123"),
            Some("Patient".to_string())
        );
    }

    #[test]
    fn test_extract_resource_type_none_for_system_paths() {
        assert_eq!(extract_resource_type("/health"), None);
        assert_eq!(extract_resource_type("/metadata"), None);
    }

    #[test]
    fn test_extract_resource_id_simple() {
        assert_eq!(extract_resource_id("/Patient/123"), Some("123".to_string()));
    }

    #[test]
    fn test_extract_resource_id_none_for_type_only() {
        assert_eq!(extract_resource_id("/Patient"), None);
    }

    #[test]
    fn test_extract_resource_id_none_for_search() {
        assert_eq!(extract_resource_id("/Patient/_search"), None);
    }

    #[test]
    fn test_parse_query_params() {
        let params = parse_query_params("patient=123&status=active");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], ("patient".to_string(), "123".to_string()));
        assert_eq!(params[1], ("status".to_string(), "active".to_string()));
    }

    #[test]
    fn test_parse_query_params_empty() {
        let params = parse_query_params("");
        // Empty string produces one entry with empty key
        assert!(params.is_empty() || params[0].0.is_empty());
    }
}
