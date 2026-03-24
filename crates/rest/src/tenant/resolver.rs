//! Tenant resolution from multiple sources.
//!
//! Provides the [`TenantResolver`] which extracts tenant information from
//! requests using multiple configurable sources.

use axum::http::request::Parts;
use helios_fhir::{FhirResourceTypeProvider, FhirVersion};
use helios_persistence::tenant::TenantId;

use crate::config::{MultitenancyConfig, TenantRoutingMode};
use crate::middleware::tenant::X_TENANT_ID;
use crate::middleware::tenant_prefix::{ExtractedTenantFromUrl, OriginalPath};

use super::source::TenantSource;

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
];

/// Result of resolving a tenant from a request.
#[derive(Debug, Clone)]
pub struct ResolvedTenant {
    /// The resolved tenant ID.
    pub tenant_id: TenantId,
    /// The source from which the tenant was resolved.
    pub source: TenantSource,
    /// All sources that provided a tenant ID (for validation).
    pub all_sources: Vec<(TenantSource, TenantId)>,
}

impl ResolvedTenant {
    /// Returns true if the tenant was resolved from a URL path.
    pub fn is_url_based(&self) -> bool {
        self.source.is_url_based()
    }

    /// Returns true if the tenant was the default fallback.
    pub fn is_default(&self) -> bool {
        self.source.is_default()
    }

    /// Returns the tenant ID as a string reference.
    pub fn tenant_id_str(&self) -> &str {
        self.tenant_id.as_str()
    }
}

/// Trait for extracting tenant information from a specific source.
pub trait TenantSourceExtractor: Send + Sync {
    /// Attempts to extract a tenant ID from the request.
    fn extract(&self, parts: &Parts, config: &MultitenancyConfig) -> Option<TenantId>;

    /// Returns the source type this extractor handles.
    fn source_type(&self) -> TenantSource;
}

/// Extracts tenant from URL path prefix.
///
/// First checks for a tenant extracted by the middleware (stored in extensions).
/// If not found, falls back to parsing the original URL path.
#[derive(Debug, Default)]
pub struct UrlPathTenantExtractor;

impl TenantSourceExtractor for UrlPathTenantExtractor {
    fn extract(&self, parts: &Parts, _config: &MultitenancyConfig) -> Option<TenantId> {
        // First, check if middleware already extracted the tenant
        if let Some(ExtractedTenantFromUrl(tenant)) =
            parts.extensions.get::<ExtractedTenantFromUrl>()
        {
            return Some(TenantId::new(tenant));
        }

        // Fall back to checking the original path (if stored) or current path
        let path = if let Some(OriginalPath(original)) = parts.extensions.get::<OriginalPath>() {
            original.as_str()
        } else {
            parts.uri.path()
        };

        // Skip leading slash and get first segment
        let path = path.strip_prefix('/').unwrap_or(path);

        // Get the first segment (before the next slash or end of path)
        let tenant = path.split('/').next()?;

        // Skip reserved paths that are not tenant identifiers
        // Use the default FHIR version for resource type checking
        let fhir_version = FhirVersion::default();
        if is_reserved_path(tenant, &fhir_version) {
            return None;
        }

        // Validate tenant ID format (non-empty, reasonable characters)
        if tenant.is_empty() || !is_valid_tenant_id(tenant) {
            return None;
        }

        Some(TenantId::new(tenant))
    }

    fn source_type(&self) -> TenantSource {
        TenantSource::UrlPath
    }
}

/// Extracts tenant from X-Tenant-ID header.
#[derive(Debug, Default)]
pub struct HeaderTenantExtractor;

impl TenantSourceExtractor for HeaderTenantExtractor {
    fn extract(&self, parts: &Parts, _config: &MultitenancyConfig) -> Option<TenantId> {
        parts
            .headers
            .get(&X_TENANT_ID)
            .and_then(|v| v.to_str().ok())
            .filter(|s| !s.is_empty() && is_valid_tenant_id(s))
            .map(TenantId::new)
    }

    fn source_type(&self) -> TenantSource {
        TenantSource::Header
    }
}

/// Extracts tenant from a validated JWT's [`Principal`] in request extensions.
///
/// The auth middleware inserts a [`Principal`] into request extensions after
/// successful token validation. This extractor reads the tenant ID from
/// the principal's configured claim.
#[derive(Debug, Default)]
pub struct JwtTenantExtractor;

impl TenantSourceExtractor for JwtTenantExtractor {
    fn extract(&self, parts: &Parts, _config: &MultitenancyConfig) -> Option<TenantId> {
        parts
            .extensions
            .get::<helios_auth::Principal>()
            .and_then(|p| p.tenant_id())
            .map(TenantId::new)
    }

    fn source_type(&self) -> TenantSource {
        TenantSource::JwtClaim
    }
}

/// Resolves tenant information from multiple sources.
pub struct TenantResolver {
    extractors: Vec<Box<dyn TenantSourceExtractor>>,
}

impl TenantResolver {
    /// Creates a new TenantResolver based on the multitenancy configuration.
    pub fn new(config: &MultitenancyConfig) -> Self {
        let mut extractors: Vec<Box<dyn TenantSourceExtractor>> = Vec::new();

        // Add extractors based on routing mode (in priority order)
        match config.routing_mode {
            TenantRoutingMode::HeaderOnly => {
                extractors.push(Box::new(HeaderTenantExtractor));
            }
            TenantRoutingMode::UrlPath => {
                extractors.push(Box::new(UrlPathTenantExtractor));
            }
            TenantRoutingMode::Both => {
                // URL path has higher priority, so it's checked first
                extractors.push(Box::new(UrlPathTenantExtractor));
                extractors.push(Box::new(HeaderTenantExtractor));
            }
        }

        // Always add JWT extractor (for future use)
        extractors.push(Box::new(JwtTenantExtractor));

        Self { extractors }
    }

    /// Creates a resolver with all extractors (for testing).
    #[cfg(test)]
    pub fn with_all_extractors() -> Self {
        Self {
            extractors: vec![
                Box::new(UrlPathTenantExtractor),
                Box::new(HeaderTenantExtractor),
                Box::new(JwtTenantExtractor),
            ],
        }
    }

    /// Resolves the tenant from the request.
    ///
    /// Returns a [`ResolvedTenant`] with the tenant ID and source information.
    pub fn resolve(
        &self,
        parts: &Parts,
        config: &MultitenancyConfig,
        default_tenant: &str,
    ) -> ResolvedTenant {
        let mut all_sources = Vec::new();

        // Try each extractor in priority order
        for extractor in &self.extractors {
            if let Some(tenant_id) = extractor.extract(parts, config) {
                all_sources.push((extractor.source_type(), tenant_id));
            }
        }

        // Select the highest priority source that provided a tenant
        if let Some((source, tenant_id)) = all_sources.first().cloned() {
            ResolvedTenant {
                tenant_id,
                source,
                all_sources,
            }
        } else {
            // Fall back to default tenant
            ResolvedTenant {
                tenant_id: TenantId::new(default_tenant),
                source: TenantSource::Default,
                all_sources,
            }
        }
    }
}

impl Default for TenantResolver {
    fn default() -> Self {
        Self::new(&MultitenancyConfig::default())
    }
}

/// Checks if a path segment is reserved (not a tenant identifier).
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

/// Validates that a string is a valid tenant ID.
fn is_valid_tenant_id(s: &str) -> bool {
    // Tenant IDs should be alphanumeric with hyphens and underscores
    // and have a reasonable length
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, Request, Uri};

    fn make_parts(path: &str, tenant_header: Option<&str>) -> Parts {
        let mut builder = Request::builder().uri(Uri::try_from(path).unwrap());

        if let Some(tenant) = tenant_header {
            builder = builder.header(&X_TENANT_ID, HeaderValue::from_str(tenant).unwrap());
        }

        let request = builder.body(()).unwrap();
        request.into_parts().0
    }

    #[test]
    fn test_url_path_extractor() {
        let extractor = UrlPathTenantExtractor;
        let config = MultitenancyConfig::default();

        // Valid tenant in URL
        let parts = make_parts("/acme/Patient/123", None);
        assert_eq!(
            extractor
                .extract(&parts, &config)
                .map(|t| t.as_str().to_string()),
            Some("acme".to_string())
        );

        // Reserved path (should not extract)
        let parts = make_parts("/Patient/123", None);
        assert_eq!(extractor.extract(&parts, &config), None);

        // System endpoint (should not extract)
        let parts = make_parts("/metadata", None);
        assert_eq!(extractor.extract(&parts, &config), None);
    }

    #[test]
    fn test_header_extractor() {
        let extractor = HeaderTenantExtractor;
        let config = MultitenancyConfig::default();

        // Valid header
        let parts = make_parts("/Patient/123", Some("acme"));
        assert_eq!(
            extractor
                .extract(&parts, &config)
                .map(|t| t.as_str().to_string()),
            Some("acme".to_string())
        );

        // Missing header
        let parts = make_parts("/Patient/123", None);
        assert_eq!(extractor.extract(&parts, &config), None);

        // Empty header
        let parts = make_parts("/Patient/123", Some(""));
        assert_eq!(extractor.extract(&parts, &config), None);
    }

    #[test]
    fn test_resolver_header_only() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        };
        let resolver = TenantResolver::new(&config);

        // Header provided
        let parts = make_parts("/Patient/123", Some("acme"));
        let resolved = resolver.resolve(&parts, &config, "default");
        assert_eq!(resolved.tenant_id_str(), "acme");
        assert_eq!(resolved.source, TenantSource::Header);

        // No header - falls back to default
        let parts = make_parts("/Patient/123", None);
        let resolved = resolver.resolve(&parts, &config, "default");
        assert_eq!(resolved.tenant_id_str(), "default");
        assert_eq!(resolved.source, TenantSource::Default);
    }

    #[test]
    fn test_resolver_url_path() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::UrlPath,
            ..Default::default()
        };
        let resolver = TenantResolver::new(&config);

        // Tenant in URL
        let parts = make_parts("/acme/Patient/123", None);
        let resolved = resolver.resolve(&parts, &config, "default");
        assert_eq!(resolved.tenant_id_str(), "acme");
        assert_eq!(resolved.source, TenantSource::UrlPath);

        // No tenant in URL (reserved path) - falls back to default
        let parts = make_parts("/Patient/123", None);
        let resolved = resolver.resolve(&parts, &config, "default");
        assert_eq!(resolved.tenant_id_str(), "default");
        assert_eq!(resolved.source, TenantSource::Default);
    }

    #[test]
    fn test_resolver_both_url_precedence() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::Both,
            ..Default::default()
        };
        let resolver = TenantResolver::new(&config);

        // Both URL and header - URL wins
        let parts = make_parts("/acme/Patient/123", Some("other"));
        let resolved = resolver.resolve(&parts, &config, "default");
        assert_eq!(resolved.tenant_id_str(), "acme");
        assert_eq!(resolved.source, TenantSource::UrlPath);
        assert_eq!(resolved.all_sources.len(), 2);

        // Only header (reserved URL path)
        let parts = make_parts("/Patient/123", Some("acme"));
        let resolved = resolver.resolve(&parts, &config, "default");
        assert_eq!(resolved.tenant_id_str(), "acme");
        assert_eq!(resolved.source, TenantSource::Header);
    }

    #[test]
    fn test_is_reserved_path() {
        let version = FhirVersion::default();

        // System endpoints
        assert!(is_reserved_path("metadata", &version));
        assert!(is_reserved_path("health", &version));

        // FHIR resource types (case insensitive)
        assert!(is_reserved_path("Patient", &version));
        assert!(is_reserved_path("PATIENT", &version));
        assert!(is_reserved_path("patient", &version));

        // Tenant IDs should NOT be reserved
        assert!(!is_reserved_path("acme", &version));
        assert!(!is_reserved_path("tenant-123", &version));
    }

    #[test]
    fn test_reserved_paths_includes_all_resource_types() {
        let version = FhirVersion::default();

        // These were missing from the old hardcoded list
        assert!(is_reserved_path("Provenance", &version));
        assert!(is_reserved_path("provenance", &version)); // case insensitive
        assert!(is_reserved_path("AuditEvent", &version));
        assert!(is_reserved_path("Binary", &version));
        assert!(is_reserved_path("OperationOutcome", &version));
        assert!(is_reserved_path("Bundle", &version));
        assert!(is_reserved_path("Parameters", &version));

        // Common resources from the old list still work
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
}
