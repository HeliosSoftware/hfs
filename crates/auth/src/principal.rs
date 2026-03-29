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
    /// The SMART launch context patient ID, present when the token was issued
    /// with a `patient/*` scope context (SMART App Launch patient context).
    /// Used by request handlers to restrict results to the patient's compartment.
    pub patient_id: Option<String>,
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

    /// Returns the SMART launch context patient ID if present.
    ///
    /// Non-`None` when the token was issued in patient context (e.g., via SMART App Launch).
    /// Handlers use this to restrict FHIR search results to the patient's compartment.
    pub fn patient_id(&self) -> Option<&str> {
        self.patient_id.as_deref()
    }
}
