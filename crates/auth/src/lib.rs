//! # helios-auth — Authentication and Authorization for the Helios FHIR Server
//!
//! This crate provides SMART Backend Services authentication via JWT/JWKS
//! validation, SMART v2 scope-based authorization, and supporting infrastructure
//! (JTI replay prevention, JWKS key caching, audit event sinks).
//!
//! ## Architecture
//!
//! HFS does **not** act as an authorization server. Token issuance and client
//! registration remain external (Keycloak, Okta, Auth0, Entra ID, etc.).
//! This crate performs local JWT validation: signature verification, claim
//! checks (issuer, audience, expiry), and JTI replay prevention.
//!
//! ## Key Types
//!
//! - [`Principal`] — Authenticated identity extracted from a validated JWT
//! - [`ScopeSet`] — Parsed SMART v2 scopes with permission checking
//! - [`AuthProvider`] — Trait for token validation implementations
//! - [`JwksBearerAuthProvider`] — JWKS-based JWT validation
//! - [`SmartScopePolicy`] — Scope-based authorization checks
//! - [`AuthConfig`] — Configuration from environment variables

pub mod audit;
pub mod config;
pub mod discovery;
pub mod error;
pub mod jti;
pub mod jwks;
pub mod outbound;
pub mod policy;
pub mod principal;
pub mod provider;
pub mod scope;

// Re-export commonly used types
pub use config::AuthConfig;
pub use discovery::SmartConfiguration;
pub use error::{AuthError, FhirOperation};
pub use jti::{JtiCache, memory::InMemoryJtiCache};
pub use jwks::JwksCache;
pub use outbound::{
    NoOpOutboundAuthProvider, OutboundAuthProvider, StaticBearerOutboundAuthProvider,
    provider_from_token,
};
pub use policy::SmartScopePolicy;
pub use principal::Principal;
pub use provider::{AuthProvider, jwks_bearer::JwksBearerAuthProvider};
pub use scope::{ScopeSet, SmartPermissions};

#[cfg(feature = "redis")]
pub use jti::redis::RedisJtiCache;
