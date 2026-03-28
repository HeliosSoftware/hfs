//! Tenant context extractor.
//!
//! Extracts tenant information from multiple sources and creates
//! a TenantContext for use in handlers.
//!
//! # Sources
//!
//! Tenant can be identified from (in priority order):
//! 1. URL path prefix: `/{tenant}/Patient/123`
//! 2. X-Tenant-ID header
//! 3. JWT token claim (future)
//! 4. Default tenant from configuration
//!
//! # Configuration
//!
//! The routing mode is controlled by [`TenantRoutingMode`]:
//! - `HeaderOnly`: Only use header (default, backward compatible)
//! - `UrlPath`: Only use URL path prefix
//! - `Both`: Support both, URL takes precedence

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

use crate::state::AppState;
use crate::tenant::{ResolvedTenant, TenantResolver, TenantSource, TenantValidator};

/// Axum extractor for tenant context.
///
/// Extracts tenant information from multiple sources based on configuration
/// and creates a TenantContext with appropriate permissions.
///
/// # Example
///
/// ```rust,ignore
/// use helios_rest::extractors::TenantExtractor;
///
/// async fn handler(tenant: TenantExtractor) {
///     println!("Tenant ID: {}", tenant.tenant_id());
///     println!("Source: {}", tenant.source());
///     if tenant.is_url_based() {
///         println!("Tenant was extracted from URL path");
///     }
/// }
/// ```
#[derive(Debug, Clone)]
pub struct TenantExtractor {
    context: TenantContext,
    source: TenantSource,
    resolved: ResolvedTenant,
}

impl TenantExtractor {
    /// Creates a new TenantExtractor with the given tenant ID and source.
    pub fn new(tenant_id: &str, source: TenantSource) -> Self {
        let tenant_id_obj = TenantId::new(tenant_id);
        Self {
            context: TenantContext::new(tenant_id_obj.clone(), TenantPermissions::full_access()),
            source,
            resolved: ResolvedTenant {
                tenant_id: tenant_id_obj.clone(),
                source,
                all_sources: vec![(source, tenant_id_obj)],
            },
        }
    }

    /// Creates a TenantExtractor from a resolved tenant.
    pub fn from_resolved(resolved: ResolvedTenant) -> Self {
        Self {
            context: TenantContext::new(
                resolved.tenant_id.clone(),
                TenantPermissions::full_access(),
            ),
            source: resolved.source,
            resolved,
        }
    }

    /// Creates a TenantExtractor with the default tenant.
    pub fn default_tenant() -> Self {
        Self::new("default", TenantSource::Default)
    }

    /// Returns a reference to the tenant context.
    pub fn context(&self) -> &TenantContext {
        &self.context
    }

    /// Returns the tenant ID as a string.
    pub fn tenant_id(&self) -> &str {
        self.context.tenant_id().as_str()
    }

    /// Returns the source from which the tenant was resolved.
    pub fn source(&self) -> TenantSource {
        self.source
    }

    /// Returns true if the tenant was resolved from a URL path.
    pub fn is_url_based(&self) -> bool {
        self.source.is_url_based()
    }

    /// Returns true if the tenant is the default fallback.
    pub fn is_default(&self) -> bool {
        self.source.is_default()
    }

    /// Returns a reference to the full resolution information.
    pub fn resolved(&self) -> &ResolvedTenant {
        &self.resolved
    }

    /// Consumes the extractor and returns the tenant context.
    pub fn into_context(self) -> TenantContext {
        self.context
    }
}

impl std::fmt::Display for TenantExtractor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.tenant_id())
    }
}

impl<S> FromRequestParts<AppState<S>> for TenantExtractor
where
    S: helios_persistence::core::ResourceStorage + Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState<S>,
    ) -> Result<Self, Self::Rejection> {
        let config = state.config();

        // If auth is enabled and a Principal with a tenant_id is present,
        // use the JWT tenant authoritatively. When strict validation is enabled,
        // verify that the JWT tenant matches any header/URL tenant to prevent
        // confusion between URL context and actual data access.
        if let Some(principal) = parts.extensions.get::<helios_auth::Principal>() {
            if let Some(jwt_tenant) = principal.tenant_id() {
                if config.multitenancy.strict_validation {
                    let resolver = TenantResolver::new(&config.multitenancy);
                    let resolved =
                        resolver.resolve(parts, &config.multitenancy, &config.default_tenant);
                    // If a non-default source provided a tenant, it must match the JWT
                    if !resolved.is_default() && resolved.tenant_id_str() != jwt_tenant {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            format!(
                                "Tenant mismatch: JWT tenant '{}' does not match request tenant '{}'",
                                jwt_tenant,
                                resolved.tenant_id_str()
                            ),
                        ));
                    }
                }
                return Ok(TenantExtractor::new(
                    jwt_tenant,
                    crate::tenant::TenantSource::JwtClaim,
                ));
            }
        }

        // Create resolver based on configuration
        let resolver = TenantResolver::new(&config.multitenancy);

        // Resolve tenant from request
        let resolved = resolver.resolve(parts, &config.multitenancy, &config.default_tenant);

        // Validate consistency if strict mode is enabled
        if config.multitenancy.strict_validation {
            TenantValidator::validate_consistency(&resolved).map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("Tenant validation error: {}", e),
                )
            })?;
        }

        // Validate tenant ID format
        if resolved.tenant_id_str().is_empty() {
            return Err((StatusCode::BAD_REQUEST, "Invalid tenant ID".to_string()));
        }

        Ok(TenantExtractor::from_resolved(resolved))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let extractor = TenantExtractor::new("test-tenant", TenantSource::Header);
        assert_eq!(extractor.tenant_id(), "test-tenant");
        assert_eq!(extractor.source(), TenantSource::Header);
        assert!(!extractor.is_url_based());
    }

    #[test]
    fn test_url_based() {
        let extractor = TenantExtractor::new("test-tenant", TenantSource::UrlPath);
        assert!(extractor.is_url_based());
    }

    #[test]
    fn test_default_tenant() {
        let extractor = TenantExtractor::default_tenant();
        assert_eq!(extractor.tenant_id(), "default");
        assert!(extractor.is_default());
    }

    #[test]
    fn test_display() {
        let extractor = TenantExtractor::new("my-tenant", TenantSource::Header);
        assert_eq!(format!("{}", extractor), "my-tenant");
    }
}
