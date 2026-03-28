use chrono::{DateTime, Utc};

use crate::scope::ScopeSet;

/// Represents an authenticated identity extracted from a validated JWT.
///
/// Injected into Axum request extensions by the auth middleware after
/// successful token validation.
#[derive(Debug, Clone)]
pub struct Principal {
    /// The `sub` (subject) claim from the JWT.
    pub subject: String,
    /// The `iss` (issuer) claim from the JWT.
    pub issuer: String,
    /// The tenant ID extracted from the configured JWT claim.
    pub tenant_id: Option<String>,
    /// Parsed SMART v2 scopes granted to this principal.
    pub scopes: ScopeSet,
    /// The `jti` (JWT ID) claim, used for replay prevention.
    pub jti: Option<String>,
    /// Token expiration time.
    pub expires_at: DateTime<Utc>,
    /// Additional claims from the JWT not captured in other fields.
    pub custom_claims: serde_json::Map<String, serde_json::Value>,
}

impl Principal {
    /// Returns the client/subject identifier.
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// Returns the token issuer.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Returns the tenant ID if present in the token.
    pub fn tenant_id(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }
}
