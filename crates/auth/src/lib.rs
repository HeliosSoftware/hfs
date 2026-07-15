//! # helios-auth — Authentication and Authorization for the Helios FHIR Server
//!
//! This crate provides SMART Backend Services authentication via JWT/JWKS
//! validation, SMART v2 scope-based authorization, and supporting infrastructure
//! (JWKS key caching, audit event sinks).
//!
//! ## Architecture
//!
//! HFS does **not** act as an authorization server. Token issuance and client
//! registration remain external (Keycloak, Okta, Auth0, Entra ID, etc.).
//! This crate performs local JWT validation: signature verification and claim
//! checks (issuer, audience, expiry).
//!
//! HFS is a **resource server**, so it never receives single-use JWTs: the
//! `private_key_jwt` client assertions that `jti` replay caches exist to protect
//! (RFC 7523 §3) are consumed by the IdP's token endpoint, not here. Bearer
//! access tokens are reusable until they expire, so no replay cache is applied
//! to them.
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
pub use jwks::JwksCache;
pub use outbound::{
    NoOpOutboundAuthProvider, OutboundAuthProvider, StaticBearerOutboundAuthProvider,
    provider_from_token,
};
pub use policy::SmartScopePolicy;
pub use principal::Principal;
pub use provider::{AuthProvider, jwks_bearer::JwksBearerAuthProvider};
pub use scope::{ScopeSet, SmartPermissions};
