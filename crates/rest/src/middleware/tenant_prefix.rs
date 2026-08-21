//! Tenant URL prefix middleware.
//!
//! Provides middleware for stripping tenant prefixes from URL paths
//! when using URL-based tenant routing.

use axum::{extract::Request, http::Uri, middleware::Next, response::Response};
use helios_fhir::{FhirResourceTypeProvider, FhirVersion};
use helios_persistence::tenant::TenantId;

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
    // SMART discovery (`/.well-known/smart-configuration`). Reserved since issue
    // #385: the canonical charset permits `.`, which the old private validator
    // here rejected, so `.well-known` would otherwise parse as a tenant and this
    // middleware would rewrite the path to `/smart-configuration` — a 404 for
    // SMART discovery under `url_path`/`both` routing.
    ".well-known",
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

/// Validates that a path segment could be a tenant ID.
///
/// Delegates to [`TenantId::parse`], the canonical validator (issue #385). This
/// used to be a private copy of the charset, one of three that disagreed; the
/// copy in `tenant::resolver` is gone for the same reason. Keeping the two in
/// agreement is load-bearing: `authz_middleware` calls
/// [`extract_tenant_from_path`] to decide which path it is authorizing, so if
/// this accepted a segment the resolver rejected, authorization and data access
/// would disagree about which tenant the request is for.
///
/// The delegation is total: whatever `parse` accepts, this accepts. In
/// particular it does **not** additionally reject `/`, even though a tenant
/// prefix is by definition a single path segment. That is deliberate — the only
/// caller, [`extract_tenant_from_path`], passes `path.split('/').next()`, so a
/// string containing `/` never reaches here. Adding a local `/` check would look
/// like a guard while guarding nothing reachable, and would reintroduce exactly
/// what issue #385 removed: a second, subtly different charset alongside the
/// canonical one.
///
/// The one addition on top of `parse` is a whole-id reserved tenant, which
/// `parse` refuses (it is a reserved *segment*). Refusing it here would be a
/// security regression, not a hardening: `authz_middleware` calls
/// [`extract_tenant_from_path`] to decide what it is authorizing, so a `None`
/// for `/__system__/Patient/123` makes it classify the raw path, decline the
/// leading `_` segment, and skip the SMART scope check altogether — see
/// `middleware::auth::test_url_routing_reserved_tenant_still_classifies_for_scope_check`.
/// The reservation is enforced once, in `TenantExtractor`, as a `403`
/// (issue #317). Only the exact reserved ids get this pass; a hierarchical
/// `acme/__system__` cannot appear in a single path segment anyway.
fn is_valid_tenant_id(s: &str) -> bool {
    TenantId::parse(s).is_ok() || TenantId::is_reserved(s)
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

    /// Regression for a break this change would otherwise have introduced.
    ///
    /// The canonical charset (issue #385) permits `.`, which this module's old
    /// private validator rejected. Without reserving `.well-known`,
    /// `/.well-known/smart-configuration` would parse `.well-known` as a tenant
    /// and rewrite the path to `/smart-configuration`, which matches no route —
    /// turning SMART discovery into a 404 under `url_path`/`both` routing.
    #[test]
    fn smart_discovery_is_not_parsed_as_a_tenant_prefix() {
        let version = default_version();
        assert!(extract_tenant_from_path("/.well-known/smart-configuration", &version).is_none());
        assert!(is_reserved_path(".well-known", &version));

        // A dotted id that is *not* a reserved route still routes as a tenant —
        // the point of widening the charset.
        let (tenant, remaining) =
            extract_tenant_from_path("/tenant.example/Patient", &version).unwrap();
        assert_eq!(tenant, "tenant.example");
        assert_eq!(remaining, "/Patient");
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
        // `.` is now accepted: the canonical charset is the union of what the
        // three old validators accepted, and the admin API always allowed it.
        // A tenant provisioned as `tenant.example` was previously unroutable.
        assert!(is_valid_tenant_id("tenant.example"));
        assert!(!is_valid_tenant_id("")); // empty
        assert!(!is_valid_tenant_id(&"a".repeat(100))); // too long
        assert!(!is_valid_tenant_id("tenant corp")); // whitespace
        // A *whole-id* reserved tenant is accepted here so the router and the
        // authz classifier still strip the prefix; the request is then refused
        // with a 403 in `TenantExtractor` (issue #317). Accepting it is what
        // keeps the SMART scope check running for `/__system__/Patient/123`.
        assert!(is_valid_tenant_id("resources"));
        assert!(is_valid_tenant_id("__system__"));
        // A reserved segment *inside* a hierarchical id stays invalid (#385) —
        // that is the S3 prefix-collision shape, and nothing refuses it later.
        assert!(!is_valid_tenant_id("acme/resources"));

        // A hierarchical id *is* valid — this is a total delegation to
        // `TenantId::parse`, not a stricter single-segment check. The earlier
        // version of this test asserted the opposite and failed, which is the
        // useful thing it did: the constraint that a tenant prefix is one
        // segment lives in the caller (`path.split('/').next()`), not here.
        // `hierarchical_id_is_accepted_here_but_unreachable_from_a_url` shows
        // why that is safe.
        assert!(is_valid_tenant_id("tenant/path"));
    }

    /// A `/` never reaches [`is_valid_tenant_id`] from a real request, so its
    /// accepting one costs nothing — and a URL still cannot address a
    /// hierarchical tenant, because only the first segment is taken.
    #[test]
    fn hierarchical_id_is_accepted_here_but_unreachable_from_a_url() {
        let version = default_version();

        // Accepted in isolation…
        assert!(is_valid_tenant_id("acme/research"));

        // …but the URL form yields the first segment only, so `acme/research`
        // is addressable by header or JWT claim and never by URL prefix.
        let (tenant, remaining) =
            extract_tenant_from_path("/acme/research/Patient", &version).unwrap();
        assert_eq!(tenant, "acme");
        assert_eq!(remaining, "/research/Patient");
    }

    /// `authz_middleware` and the tenant resolver must classify a path segment
    /// identically, or the request would be authorized as one tenant and served
    /// as another. Both now call `TenantId::parse`; this pins that they agree.
    ///
    /// The whole-id reserved names are excluded because both sides make the
    /// same deliberate exception for them (see `is_valid_tenant_id` and
    /// `tenant::resolver::reserved_id_passthrough`) — they are covered by
    /// `reserved_ids_are_extracted_here_and_refused_in_the_extractor` instead.
    #[test]
    fn tenant_prefix_and_canonical_validator_agree() {
        for candidate in [
            "acme",
            "ACME",
            "tenant.example",
            "my_tenant",
            "tenant-1",
            // Included so the delegation stays *total*: a future local `/`
            // check here would diverge from the canonical validator and fail
            // this test, which is the point.
            "acme/research",
            "",
            "acme/resources",
            "acme/__system__",
            "tenant corp",
            "tenant%2F",
        ] {
            assert_eq!(
                is_valid_tenant_id(candidate),
                TenantId::parse(candidate).is_ok(),
                "{candidate:?} classified differently from the canonical validator"
            );
        }
    }

    /// The reserved-id exception, stated once: the prefix is still stripped, so
    /// the authz classifier sees `/Patient/123` and runs the scope check, and
    /// the reservation is enforced later as a `403` (issue #317).
    #[test]
    fn reserved_ids_are_extracted_here_and_refused_in_the_extractor() {
        let version = default_version();

        for reserved in helios_persistence::tenant::RESERVED_TENANT_IDS {
            assert!(TenantId::parse(reserved).is_err());
            let (tenant, remaining) =
                extract_tenant_from_path(&format!("/{reserved}/Patient/123"), &version)
                    .unwrap_or_else(|| panic!("{reserved} must still strip as a tenant prefix"));
            assert_eq!(&tenant, reserved);
            assert_eq!(remaining, "/Patient/123");
        }
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
