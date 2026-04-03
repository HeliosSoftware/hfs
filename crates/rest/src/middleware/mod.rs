//! HTTP middleware for the FHIR REST API.
//!
//! This module contains Axum middleware components:
//!
//! - [`tenant`] - Tenant identification and extraction
//! - [`tenant_prefix`] - URL prefix stripping for tenant-in-URL routing
//! - [`content_type`] - Content negotiation
//! - [`conditional`] - Conditional request headers (If-Match, etc.)
//! - [`prefer`] - Prefer header handling

pub mod auth;
pub mod conditional;
pub mod content_type;
pub mod prefer;
pub mod tenant;
pub mod tenant_prefix;

pub use conditional::ConditionalHeaders;
pub use prefer::PreferHeader;
pub use tenant_prefix::{ExtractedTenantFromUrl, OriginalPath};
