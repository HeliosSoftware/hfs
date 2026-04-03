use std::env;

/// Configuration for the authentication and authorization subsystem.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Master switch — when false, all auth is bypassed.
    pub enabled: bool,
    /// JWKS endpoint URL. Required when `enabled` is true.
    pub jwks_url: Option<String>,
    /// Expected JWT issuer (`iss` claim). Validated if set.
    pub expected_issuer: Option<String>,
    /// Expected JWT audience (`aud` claim). Validated if set.
    ///
    /// **Recommended for production.** Without audience validation, any valid
    /// token from the same issuer is accepted — even tokens intended for a
    /// different service. Set `HFS_AUTH_AUDIENCE` to restrict accepted tokens
    /// to those explicitly issued for this server.
    pub expected_audience: Option<String>,
    /// JWT claim name used to extract the tenant ID.
    pub tenant_claim: String,
    /// Comma-separated list of allowed JWT signing algorithms.
    pub allowed_algorithms: Vec<String>,
    /// JTI cache backend: `"memory"` or `"redis"`.
    pub jti_backend: String,
    /// Redis connection URL (required when `jti_backend` is `"redis"`).
    pub redis_url: Option<String>,
    /// Minimum interval (seconds) between JWKS refreshes.
    pub jwks_min_refresh_interval: u64,

    // SMART discovery endpoint fields
    /// Token endpoint URL for `/.well-known/smart-configuration`.
    pub smart_token_endpoint: Option<String>,
    /// Authorization endpoint URL.
    pub smart_authorize_endpoint: Option<String>,
    /// JWKS URL for the discovery document (may differ from `jwks_url`).
    pub smart_jwks_url: Option<String>,
    /// Introspection endpoint URL.
    pub smart_introspection_endpoint: Option<String>,
    /// Management endpoint URL.
    pub smart_management_endpoint: Option<String>,
    /// Registration endpoint URL.
    pub smart_registration_endpoint: Option<String>,
    /// Revocation endpoint URL.
    pub smart_revocation_endpoint: Option<String>,
}

impl AuthConfig {
    /// Load configuration from environment variables.
    pub fn from_env() -> Self {
        Self {
            enabled: env::var("HFS_AUTH_ENABLED")
                .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
                .unwrap_or(false),
            jwks_url: env::var("HFS_AUTH_JWKS_URL").ok(),
            expected_issuer: env::var("HFS_AUTH_ISSUER").ok(),
            expected_audience: env::var("HFS_AUTH_AUDIENCE").ok(),
            tenant_claim: env::var("HFS_AUTH_TENANT_CLAIM")
                .unwrap_or_else(|_| "tenant_id".to_string()),
            allowed_algorithms: env::var("HFS_AUTH_ALGORITHMS")
                .unwrap_or_else(|_| "RS256,RS384,ES256,ES384".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
            jti_backend: env::var("HFS_AUTH_JTI_BACKEND").unwrap_or_else(|_| "memory".to_string()),
            redis_url: env::var("HFS_AUTH_REDIS_URL").ok(),
            jwks_min_refresh_interval: env::var("HFS_AUTH_JWKS_MIN_REFRESH_INTERVAL")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            smart_token_endpoint: env::var("HFS_SMART_TOKEN_ENDPOINT").ok(),
            smart_authorize_endpoint: env::var("HFS_SMART_AUTHORIZE_ENDPOINT").ok(),
            smart_jwks_url: env::var("HFS_SMART_JWKS_URL").ok(),
            smart_introspection_endpoint: env::var("HFS_SMART_INTROSPECTION_ENDPOINT").ok(),
            smart_management_endpoint: env::var("HFS_SMART_MANAGEMENT_ENDPOINT").ok(),
            smart_registration_endpoint: env::var("HFS_SMART_REGISTRATION_ENDPOINT").ok(),
            smart_revocation_endpoint: env::var("HFS_SMART_REVOCATION_ENDPOINT").ok(),
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            jwks_url: None,
            expected_issuer: None,
            expected_audience: None,
            tenant_claim: "tenant_id".to_string(),
            allowed_algorithms: vec![
                "RS256".to_string(),
                "RS384".to_string(),
                "ES256".to_string(),
                "ES384".to_string(),
            ],
            jti_backend: "memory".to_string(),
            redis_url: None,
            jwks_min_refresh_interval: 10,
            smart_token_endpoint: None,
            smart_authorize_endpoint: None,
            smart_jwks_url: None,
            smart_introspection_endpoint: None,
            smart_management_endpoint: None,
            smart_registration_endpoint: None,
            smart_revocation_endpoint: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AuthConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.tenant_claim, "tenant_id");
        assert_eq!(config.jti_backend, "memory");
        assert_eq!(config.jwks_min_refresh_interval, 10);
        assert_eq!(config.allowed_algorithms.len(), 4);
    }
}
