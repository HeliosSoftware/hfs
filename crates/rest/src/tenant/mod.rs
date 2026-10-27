//! Multi-source tenant resolution.
//!
//! This module provides tenant identification from multiple sources:
//!
//! - **URL path prefix**: `/{tenant}/Patient/123` (FHIR spec approach)
//! - **X-Tenant-ID header**: Traditional header-based identification
//! - **JWT token claim**: Future support for authentication-based tenants
//! - **Default tenant**: Fallback from configuration
//!
//! # Resolution Priority
//!
//! When multiple sources provide a tenant ID, they are resolved in this
//! priority order (highest to lowest):
//!
//! 1. URL path prefix
//! 2. X-Tenant-ID header
//! 3. JWT token claim
//! 4. Default tenant from configuration
//!
//! # Configuration
//!
//! Tenant routing mode is configured via [`TenantRoutingMode`]:
//!
//! - `HeaderOnly` (default): Only use X-Tenant-ID header
//! - `UrlPath`: Only use URL path prefix
//! - `Both`: Support both, with URL taking precedence
//!
//! # Strict Validation
//!
//! When [`MultitenancyConfig::strict_validation`] is enabled, the resolver
//! will return an error if multiple sources provide different tenant IDs.
//! This helps catch configuration or client issues early.
//!
//! # Example
//!
//! ```rust,ignore
//! use helios_rest::tenant::{TenantResolver, TenantSource};
//! use helios_rest::config::MultitenancyConfig;
//!
//! let config = MultitenancyConfig::default();
//! let resolver = TenantResolver::new(&config);
//!
//! // In an Axum handler. `resolve` fails when a source *asserted* a tenant id
//! // that is not canonical (see `helios_persistence::tenant::TenantId::parse`);
//! // a request that asserts nothing resolves to the default tenant.
//! let resolved = resolver.resolve(&parts, &config, "default")?;
//! println!("Tenant: {} (from {})", resolved.tenant_id_str(), resolved.source);
//! ```

mod resolver;
mod source;
mod validation;

pub use resolver::{
    HeaderTenantExtractor, JwtTenantExtractor, ResolvedTenant, TenantResolver, TenantSourceError,
    TenantSourceExtractor, UrlPathTenantExtractor,
};
pub use source::TenantSource;
pub use validation::{TenantMismatchError, TenantValidator};
