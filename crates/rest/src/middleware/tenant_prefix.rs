//! Tenant URL prefix middleware.
//!
//! Provides middleware for stripping tenant prefixes from URL paths
//! when using URL-based tenant routing.

use axum::{extract::Request, http::Uri, middleware::Next, response::Response};
use helios_fhir::{FhirResourceTypeProvider, FhirVersion};

/// Non-resource reserved paths (FHIR system endpoints, API prefixes).
/// Resource types are checked dynamically via helios-fhir's FhirResourceTypeProvider.
const RESERVED_SYSTEM_PATHS: &[&str] = &[
    "metadata",
    "health",
    "_history",
    "_liveness",
    "_readiness",
    "$versions",
    "api",
    "v1",
    "v2",
    "fhir",
    // Management-console namespace (`/console/metrics/*`). Reserved so a tenant
    // can never be named `console` under URL-path routing, keeping the console
    // paths unambiguous with the authz console guard (see `middleware::auth`).
    "console",
];

/// Checks if a path segment is a reserved path (not a tenant identifier).
///
/// A segment is reserved if it's either:
/// 1. A FHIR system endpoint or API prefix (from RESERVED_SYSTEM_PATHS)
/// 2. A valid FHIR resource type for the given version
fn is_reserved_path(segment: &str, fhir_version: &FhirVersion) -> bool {
    let lower = segment.to_lowercase();

    // Check system paths first (fast path)
    if RESERVED_SYSTEM_PATHS.iter().any(|&r| r == lower) {
        return true;
    }

    // Check if it's a valid FHIR resource type for the configured version
    is_fhir_resource_type(segment, fhir_version)
}

/// Checks if a string is a valid FHIR resource type for the given version.
/// Uses the FhirResourceTypeProvider trait for case-insensitive matching.
fn is_fhir_resource_type(type_name: &str, fhir_version: &FhirVersion) -> bool {
    match fhir_version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => helios_fhir::r4::Resource::is_resource_type(type_name),
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => helios_fhir::r4b::Resource::is_resource_type(type_name),
        #[cfg(feature = "R5")]
        FhirVersion::R5 => helios_fhir::r5::Resource::is_resource_type(type_name),
        #[cfg(feature = "R6")]
        FhirVersion::R6 => helios_fhir::r6::Resource::is_resource_type(type_name),
        #[allow(unreachable_patterns)]
        _ => false,
    }
}

/// Validates that a string could be a tenant ID.
fn is_valid_tenant_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Extracts tenant from URL path if present.
///
/// Returns `Some((tenant_id, remaining_path))` if a tenant prefix was found,
/// or `None` if the path doesn't start with a tenant prefix.
///
/// Uses the provided FHIR version for resource type detection.
pub fn extract_tenant_from_path(
    path: &str,
    fhir_version: &FhirVersion,
) -> Option<(String, String)> {
    // Skip leading slash and get first segment
    let path = path.strip_prefix('/').unwrap_or(path);

    // Get the first segment
    let first_segment = path.split('/').next()?;

    // Check if it's a valid tenant ID and not a reserved path
    if !is_reserved_path(first_segment, fhir_version) && is_valid_tenant_id(first_segment) {
        let tenant = first_segment.to_string();
        let remaining = path.strip_prefix(first_segment).unwrap_or("").to_string();
        // Ensure remaining path starts with / or is empty
        let remaining = if remaining.is_empty() || remaining.starts_with('/') {
            remaining
        } else {
            format!("/{}", remaining)
        };
        // If remaining is empty, use "/" for root
        let remaining = if remaining.is_empty() {
            "/".to_string()
        } else {
            remaining
        };
        Some((tenant, remaining))
    } else {
        None
    }
}

/// Middleware that strips tenant prefix from URL paths.
///
/// When URL-based tenant routing is enabled, this middleware:
/// 1. Extracts the tenant ID from the first path segment
/// 2. Stores the original path in a request extension
/// 3. Rewrites the URI to remove the tenant prefix
///
/// The TenantExtractor can then read the original path from the extension.
///
/// Uses the default FHIR version (R4) for resource type detection.
pub async fn strip_tenant_prefix_middleware(mut request: Request, next: Next) -> Response {
    let original_uri = request.uri().clone();
    let path = original_uri.path();

    // Use the default FHIR version for resource type checking
    let fhir_version = FhirVersion::default_enabled();

    // Try to extract tenant from path
    if let Some((tenant, remaining_path)) = extract_tenant_from_path(path, &fhir_version) {
        // Store original path and extracted tenant in extensions
        request
            .extensions_mut()
            .insert(OriginalPath(original_uri.path().to_string()));
        request
            .extensions_mut()
            .insert(ExtractedTenantFromUrl(tenant));

        // Build new URI with remaining path
        let new_uri = build_uri_with_new_path(&original_uri, &remaining_path);
        *request.uri_mut() = new_uri;
    }

    next.run(request).await
}

/// Extension type for storing the original request path.
#[derive(Clone, Debug)]
pub struct OriginalPath(pub String);

/// Extension type for storing the tenant extracted from URL.
#[derive(Clone, Debug)]
pub struct ExtractedTenantFromUrl(pub String);

/// Builds a new URI with a different path but same query/fragment.
fn build_uri_with_new_path(original: &Uri, new_path: &str) -> Uri {
    let mut parts = original.clone().into_parts();

    // Build path-and-query
    let path_and_query = if let Some(query) = original.query() {
        format!("{}?{}", new_path, query)
    } else {
        new_path.to_string()
    };

    parts.path_and_query = Some(path_and_query.parse().unwrap_or_else(|_| {
        // Fallback to just the path if parsing fails
        new_path.parse().unwrap()
    }));

    Uri::from_parts(parts).unwrap_or_else(|_| original.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_version() -> FhirVersion {
        FhirVersion::default()
    }

    #[test]
    fn test_extract_tenant_from_path() {
        let version = default_version();

        // Valid tenant paths
        let (tenant, remaining) = extract_tenant_from_path("/acme/Patient/123", &version).unwrap();
        assert_eq!(tenant, "acme");
        assert_eq!(remaining, "/Patient/123");

        let (tenant, remaining) = extract_tenant_from_path("/tenant-1/metadata", &version).unwrap();
        assert_eq!(tenant, "tenant-1");
        assert_eq!(remaining, "/metadata");

        let (tenant, remaining) = extract_tenant_from_path("/my_tenant/", &version).unwrap();
        assert_eq!(tenant, "my_tenant");
        assert_eq!(remaining, "/");

        let (tenant, remaining) = extract_tenant_from_path("/acme", &version).unwrap();
        assert_eq!(tenant, "acme");
        assert_eq!(remaining, "/");
    }

    #[test]
    fn test_extract_tenant_reserved_paths() {
        let version = default_version();

        // Reserved paths should not be extracted as tenants
        assert!(extract_tenant_from_path("/Patient/123", &version).is_none());
        assert!(extract_tenant_from_path("/metadata", &version).is_none());
        assert!(extract_tenant_from_path("/health", &version).is_none());
        assert!(extract_tenant_from_path("/_history", &version).is_none());

        // Previously missing resource types should also be reserved
        assert!(extract_tenant_from_path("/Provenance/123", &version).is_none());
        assert!(extract_tenant_from_path("/AuditEvent/456", &version).is_none());
        assert!(extract_tenant_from_path("/Binary/789", &version).is_none());

        // The management-console namespace must never be parsed as a tenant, so
        // `console` cannot shadow / be confused with a tenant literally named
        // "console" under URL-path routing.
        assert!(extract_tenant_from_path("/console/metrics/uptime", &version).is_none());
    }

    #[test]
    fn test_is_reserved_path() {
        let version = default_version();

        // System paths
        assert!(is_reserved_path("metadata", &version));
        assert!(is_reserved_path("health", &version));

        // Management-console namespace is reserved (case-insensitive), so a
        // tenant named "console" cannot exist under URL-path routing.
        assert!(is_reserved_path("console", &version));
        assert!(is_reserved_path("Console", &version));

        // FHIR resource types (case insensitive)
        assert!(is_reserved_path("Patient", &version));
        assert!(is_reserved_path("patient", &version));
        assert!(is_reserved_path("PATIENT", &version));

        // Previously missing resource types
        assert!(is_reserved_path("Provenance", &version));
        assert!(is_reserved_path("AuditEvent", &version));
        assert!(is_reserved_path("Binary", &version));
        assert!(is_reserved_path("OperationOutcome", &version));

        // Tenant IDs should NOT be reserved
        assert!(!is_reserved_path("acme", &version));
        assert!(!is_reserved_path("tenant-123", &version));
    }

    #[test]
    fn test_is_valid_tenant_id() {
        assert!(is_valid_tenant_id("acme"));
        assert!(is_valid_tenant_id("tenant-123"));
        assert!(is_valid_tenant_id("my_tenant"));
        assert!(is_valid_tenant_id("ABC123"));
        assert!(!is_valid_tenant_id("")); // empty
        assert!(!is_valid_tenant_id("tenant.com")); // dot
        assert!(!is_valid_tenant_id("tenant/path")); // slash
        assert!(!is_valid_tenant_id(&"a".repeat(100))); // too long
    }

    #[test]
    fn test_build_uri_with_new_path() {
        let uri: Uri = "/acme/Patient/123?_count=10".parse().unwrap();
        let new_uri = build_uri_with_new_path(&uri, "/Patient/123");
        assert_eq!(new_uri.path(), "/Patient/123");
        assert_eq!(new_uri.query(), Some("_count=10"));

        let uri: Uri = "/acme/Patient".parse().unwrap();
        let new_uri = build_uri_with_new_path(&uri, "/Patient");
        assert_eq!(new_uri.path(), "/Patient");
        assert_eq!(new_uri.query(), None);
    }

    #[test]
    fn test_all_fhir_resource_types_reserved() {
        let version = default_version();

        // Previously missing from hardcoded list - now dynamically checked
        assert!(is_reserved_path("Provenance", &version));
        assert!(is_reserved_path("provenance", &version)); // case insensitive
        assert!(is_reserved_path("AuditEvent", &version));
        assert!(is_reserved_path("Binary", &version));
        assert!(is_reserved_path("OperationOutcome", &version));
        assert!(is_reserved_path("Bundle", &version));
        assert!(is_reserved_path("Parameters", &version));
        assert!(is_reserved_path("RiskAssessment", &version));
        assert!(is_reserved_path("NutritionOrder", &version));
        assert!(is_reserved_path("MolecularSequence", &version));

        // Common resources still work
        assert!(is_reserved_path("Patient", &version));
        assert!(is_reserved_path("Observation", &version));
        assert!(is_reserved_path("Condition", &version));
        assert!(is_reserved_path("Encounter", &version));
        assert!(is_reserved_path("Medication", &version));

        // Tenant IDs should NOT be reserved
        assert!(!is_reserved_path("acme", &version));
        assert!(!is_reserved_path("tenant-123", &version));
        assert!(!is_reserved_path("my_tenant", &version));
    }
}
