use std::sync::Arc;

use async_trait::async_trait;
use jsonwebtoken::{Algorithm, Validation, decode, decode_header};
use tracing::{debug, warn};

use super::AuthProvider;
use crate::config::AuthConfig;
use crate::error::AuthError;
use crate::jwks::JwksCache;
use crate::principal::{LaunchContext, Principal};
use crate::scope::ScopeSet;

/// Authentication provider that validates Bearer tokens as JWTs
/// using keys from a JWKS endpoint.
pub struct JwksBearerAuthProvider {
    jwks_cache: Arc<JwksCache>,
    expected_audience: Option<String>,
    expected_issuer: Option<String>,
    tenant_claim: String,
    patient_claim: String,
    encounter_claim: String,
    fhir_user_claim: String,
    allowed_algorithms: Vec<Algorithm>,
}

impl JwksBearerAuthProvider {
    /// Create a new JWKS Bearer auth provider.
    pub fn new(jwks_cache: Arc<JwksCache>, config: &AuthConfig) -> Self {
        let allowed_algorithms = config
            .allowed_algorithms
            .iter()
            .filter_map(|alg| parse_algorithm(alg))
            .collect();

        Self {
            jwks_cache,
            expected_audience: config.expected_audience.clone(),
            expected_issuer: config.expected_issuer.clone(),
            tenant_claim: config.tenant_claim.clone(),
            patient_claim: config.patient_claim.clone(),
            encounter_claim: config.encounter_claim.clone(),
            fhir_user_claim: config.fhir_user_claim.clone(),
            allowed_algorithms,
        }
    }
}

#[async_trait]
impl AuthProvider for JwksBearerAuthProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        // 1. Strip "Bearer " prefix
        let token = authorization_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| {
                AuthError::InvalidTokenFormat(
                    "Authorization header must start with 'Bearer '".to_string(),
                )
            })?;

        if token.is_empty() {
            return Err(AuthError::InvalidTokenFormat("Empty token".to_string()));
        }

        // 2. Decode JWT header to get kid and alg
        let header = decode_header(token).map_err(|e| {
            AuthError::InvalidTokenFormat(format!("Failed to decode JWT header: {}", e))
        })?;

        let alg = header.alg;

        // 3. Check algorithm is allowed
        if !self.allowed_algorithms.contains(&alg) {
            return Err(AuthError::UnsupportedAlgorithm {
                alg: format!("{:?}", alg),
            });
        }

        // 4. Look up key by kid
        let kid = header
            .kid
            .ok_or_else(|| AuthError::InvalidTokenFormat("JWT header missing 'kid'".to_string()))?;

        let decoding_key = self.jwks_cache.get_key(&kid).await?;

        // 5. Build validation
        let validation = build_validation(
            alg,
            self.expected_audience.as_deref(),
            self.expected_issuer.as_deref(),
        );

        // 6. Decode and validate
        let token_data =
            decode::<serde_json::Value>(token, &decoding_key, &validation).map_err(|e| match e
                .kind()
            {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::TokenExpired,
                jsonwebtoken::errors::ErrorKind::InvalidSignature => AuthError::InvalidSignature,
                jsonwebtoken::errors::ErrorKind::InvalidAudience => {
                    AuthError::ValidationError("Invalid audience".to_string())
                }
                jsonwebtoken::errors::ErrorKind::InvalidIssuer => {
                    AuthError::ValidationError("Invalid issuer".to_string())
                }
                jsonwebtoken::errors::ErrorKind::MissingRequiredClaim(claim) => {
                    AuthError::ValidationError(format!("Missing required claim: {claim}"))
                }
                _ => AuthError::ValidationError(format!("Token validation failed: {}", e)),
            })?;

        let claims = token_data.claims;

        // 7. Extract standard claims
        //
        // `sub` is in `required_spec_claims`, so a token without one never reaches
        // here — but the required-claim check only asserts presence, and an empty
        // `"sub": ""` would satisfy it while yielding an anonymous principal that
        // still flows into audit records and policy decisions. Reject that too.
        let subject = claims
            .get("sub")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AuthError::ValidationError("Missing 'sub' claim".to_string()))?
            .to_string();

        let issuer = claims
            .get("iss")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let jti = claims.get("jti").and_then(|v| v.as_str()).map(String::from);

        let exp = claims
            .get("exp")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| AuthError::ValidationError("Missing 'exp' claim".to_string()))?;

        let expires_at = chrono::DateTime::from_timestamp(exp, 0)
            .ok_or_else(|| AuthError::ValidationError("Invalid 'exp' timestamp".to_string()))?;

        // 8. Parse scopes from every claim an IdP may carry them in
        let scopes = scopes_from_claims(&claims);
        if scopes.raw().is_empty() {
            debug!(sub = %subject, "No scope, scp or roles claim found in token");
        }

        // 9. Extract tenant from configured claim
        let tenant_id = claims
            .get(&self.tenant_claim)
            .and_then(|v| v.as_str())
            .map(String::from);

        // 10. SMART launch context from the configured claims. The claims also
        // stay in `custom_claims` below, where callers found them before.
        let launch_context = claims.as_object().and_then(|map| {
            LaunchContext::from_claims(
                map,
                &self.patient_claim,
                &self.encounter_claim,
                &self.fhir_user_claim,
            )
        });

        // 11. Build custom claims map (excluding standard claims)
        let custom_claims = if let serde_json::Value::Object(map) = claims {
            let standard = [
                "sub", "iss", "exp", "iat", "nbf", "aud", "jti", "scope", "scp",
            ];
            map.into_iter()
                .filter(|(k, _)| !standard.contains(&k.as_str()) && k != &self.tenant_claim)
                .collect()
        } else {
            serde_json::Map::new()
        };

        debug!(sub = %subject, iss = %issuer, "Token validated successfully");

        Ok(Principal {
            subject,
            issuer,
            tenant_id,
            scopes,
            jti,
            expires_at,
            custom_claims,
            launch_context,
        })
    }

    fn name(&self) -> &str {
        "jwks-bearer"
    }
}

/// Claims a token's granted scopes are read from, in order.
///
/// - `scope`: the OAuth 2.0 claim (Keycloak, Auth0), a space-delimited string.
/// - `scp`: an array on Okta, a space-delimited string on Microsoft Entra ID
///   delegated (user) tokens.
/// - `roles`: Microsoft Entra ID application permissions, granted to a client
///   as App Roles and issued on client-credentials tokens.
const SCOPE_CLAIMS: [&str; 3] = ["scope", "scp", "roles"];

/// Collects the scopes of every [`SCOPE_CLAIMS`] claim present, each as a
/// space-delimited string or an array of strings, without duplicates.
fn scopes_from_claims(claims: &serde_json::Value) -> ScopeSet {
    let mut raw: Vec<String> = Vec::new();
    let mut push = |scope: &str| {
        if !raw.iter().any(|seen| seen == scope) {
            raw.push(scope.to_string());
        }
    };
    for name in SCOPE_CLAIMS {
        match claims.get(name) {
            Some(serde_json::Value::String(s)) => s.split_whitespace().for_each(&mut push),
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .filter_map(|v| v.as_str())
                .flat_map(str::split_whitespace)
                .for_each(&mut push),
            _ => {}
        }
    }
    ScopeSet::parse_array(&raw)
}

fn parse_algorithm(alg: &str) -> Option<Algorithm> {
    match alg {
        "RS256" => Some(Algorithm::RS256),
        "RS384" => Some(Algorithm::RS384),
        "RS512" => Some(Algorithm::RS512),
        "ES256" => Some(Algorithm::ES256),
        "ES384" => Some(Algorithm::ES384),
        _ => {
            warn!(algorithm = alg, "Unknown JWT algorithm, ignoring");
            None
        }
    }
}

/// Builds the JWT [`Validation`] for `alg`, requiring the audience and issuer
/// claims when they are configured.
///
/// `set_audience` / `set_issuer` only validate their claim when it is *present*
/// in the token, and `Validation::required_spec_claims` defaults to just
/// `{"exp"}`. A token that omits `aud` (or `iss`) would therefore slip past a
/// configured restriction — the bug in issue #206. Adding the claim to
/// `required_spec_claims` makes a missing one fail validation.
///
/// The same presence check also rejects a claim of the wrong JSON type (e.g.
/// `"aud": 42`), which otherwise fails to deserialize and is skipped as if absent.
fn build_validation(
    alg: Algorithm,
    expected_audience: Option<&str>,
    expected_issuer: Option<&str>,
) -> Validation {
    let mut validation = Validation::new(alg);

    if let Some(aud) = expected_audience {
        validation.set_audience(&[aud]);
        validation.required_spec_claims.insert("aud".to_string());
    } else {
        validation.validate_aud = false;
    }

    if let Some(iss) = expected_issuer {
        validation.set_issuer(&[iss]);
        validation.required_spec_claims.insert("iss".to_string());
    }

    // Every token must identify a subject: it becomes the `Principal` that audit
    // records and policy decisions are attributed to.
    validation.required_spec_claims.insert("sub".to_string());

    validation.validate_exp = true;
    // `nbf` is only enforced when the claim is present, so this rejects a
    // not-yet-valid token without requiring the claim of tokens that omit it.
    validation.validate_nbf = true;
    validation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::SmartPermissions;

    #[test]
    fn audience_and_issuer_become_required_claims() {
        // Regression for #206: a configured audience/issuer must be *required*,
        // not merely checked-when-present, or a token omitting the claim bypasses
        // the restriction.
        let v = build_validation(Algorithm::RS256, Some("hfs-api"), Some("https://idp"));
        assert!(v.required_spec_claims.contains("aud"));
        assert!(v.required_spec_claims.contains("iss"));
        assert!(v.validate_aud);
    }

    #[test]
    fn no_audience_disables_aud_validation() {
        let v = build_validation(Algorithm::RS256, None, Some("https://idp"));
        assert!(!v.validate_aud);
        assert!(!v.required_spec_claims.contains("aud"));
        assert!(v.required_spec_claims.contains("iss"));
    }

    #[test]
    fn no_issuer_leaves_iss_unrequired() {
        let v = build_validation(Algorithm::RS256, Some("hfs-api"), None);
        assert!(v.required_spec_claims.contains("aud"));
        assert!(!v.required_spec_claims.contains("iss"));
    }

    #[test]
    fn subject_is_always_required() {
        // The subject is what audit records and policy decisions are attributed to,
        // so it is required regardless of what else is configured.
        let v = build_validation(Algorithm::RS256, None, None);
        assert!(v.required_spec_claims.contains("sub"));
    }

    fn raw_scopes(claims: serde_json::Value) -> Vec<String> {
        scopes_from_claims(&claims).raw().to_vec()
    }

    #[test]
    fn scope_string_is_split_on_whitespace() {
        assert_eq!(
            raw_scopes(serde_json::json!({ "scope": "openid system/Patient.rs" })),
            ["openid", "system/Patient.rs"]
        );
    }

    #[test]
    fn scp_is_read_as_an_array_or_a_string() {
        assert_eq!(
            raw_scopes(serde_json::json!({ "scp": ["user/*.rs", "openid"] })),
            ["user/*.rs", "openid"]
        );
        assert_eq!(
            raw_scopes(serde_json::json!({ "scp": "user/*.rs openid" })),
            ["user/*.rs", "openid"]
        );
    }

    #[test]
    fn roles_grant_scopes() {
        let scopes = scopes_from_claims(&serde_json::json!({ "roles": ["system/*.cruds"] }));
        assert!(scopes.is_permitted("Patient", SmartPermissions::CREATE));
        assert!(scopes.has_system_wildcard());
    }

    #[test]
    fn every_scope_claim_is_merged_without_duplicates() {
        assert_eq!(
            raw_scopes(serde_json::json!({
                "scope": "openid",
                "scp": "user/Patient.rs openid",
                "roles": ["system/Observation.rs", "user/Patient.rs"],
            })),
            ["openid", "user/Patient.rs", "system/Observation.rs"]
        );
    }

    #[test]
    fn non_string_scope_values_are_ignored() {
        assert!(
            raw_scopes(serde_json::json!({ "scope": 1, "scp": [true, null], "roles": {} }))
                .is_empty()
        );
    }

    #[test]
    fn nbf_is_validated_but_not_required() {
        // Enforced when present (a not-yet-valid token is rejected) without forcing
        // tokens that omit `nbf` to carry it.
        let v = build_validation(Algorithm::RS256, None, None);
        assert!(v.validate_nbf);
        assert!(!v.required_spec_claims.contains("nbf"));
    }
}
