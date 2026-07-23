use std::env;
use std::sync::Arc;

use crate::outbound::{OutboundAuthProvider, provider_from_token};

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

    /// Static bearer token attached to outbound server-to-server requests
    /// (e.g., subscription notification dispatch). When set, an
    /// `Authorization: Bearer <token>` header is added to outbound calls.
    /// Subscription-supplied headers take precedence.
    pub outbound_bearer_token: Option<String>,
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
            outbound_bearer_token: env::var("HFS_OUTBOUND_BEARER_TOKEN").ok(),
        }
    }

    /// Build an outbound auth provider from this config.
    ///
    /// Returns a [`StaticBearerOutboundAuthProvider`](crate::StaticBearerOutboundAuthProvider)
    /// when [`outbound_bearer_token`](Self::outbound_bearer_token) is set,
    /// otherwise a [`NoOpOutboundAuthProvider`](crate::NoOpOutboundAuthProvider).
    pub fn outbound_provider(&self) -> Arc<dyn OutboundAuthProvider> {
        provider_from_token(self.outbound_bearer_token.as_deref())
    }

    /// Validates that an enabled configuration is internally coherent,
    /// accumulating every problem rather than reporting only the first.
    ///
    /// A disabled configuration is always valid — nothing below is consulted
    /// when auth is bypassed.
    ///
    /// These invariants previously lived only in the `hfs` binary's startup
    /// path, which meant any *other* embedder of the library crates — the
    /// integration harnesses, downstream users — could build an enabled
    /// `AuthConfig` with no pinned issuer and never be told. Since the issuer is
    /// what qualifies a subject into a per-user identity (see
    /// `helios_rest::extractors::UserKey`), an unpinned issuer makes that
    /// identity depend on an unvalidated token claim. Keeping the check on the
    /// type means it holds for every caller.
    ///
    /// Audience is deliberately *not* required: an open demo deployment
    /// legitimately accepts any token from its issuer. Callers should warn.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        if !self.enabled {
            return Ok(());
        }

        let mut errors = Vec::new();
        if self.jwks_url.as_deref().unwrap_or("").trim().is_empty() {
            errors.push("HFS_AUTH_JWKS_URL is required when HFS_AUTH_ENABLED=true".to_string());
        }
        if self
            .expected_issuer
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            errors.push(
                "HFS_AUTH_ISSUER is required when HFS_AUTH_ENABLED=true (it pins the `iss` \
                 claim, which qualifies every per-user identity)"
                    .to_string(),
            );
        }
        if self.allowed_algorithms.is_empty() {
            errors.push("HFS_AUTH_ALGORITHMS must list at least one signing algorithm".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
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
            jwks_min_refresh_interval: 10,
            smart_token_endpoint: None,
            smart_authorize_endpoint: None,
            smart_jwks_url: None,
            smart_introspection_endpoint: None,
            smart_management_endpoint: None,
            smart_registration_endpoint: None,
            smart_revocation_endpoint: None,
            outbound_bearer_token: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{LazyLock, Mutex};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    const AUTH_ENV_KEYS: &[&str] = &[
        "HFS_AUTH_ENABLED",
        "HFS_AUTH_JWKS_URL",
        "HFS_AUTH_ISSUER",
        "HFS_AUTH_AUDIENCE",
        "HFS_AUTH_TENANT_CLAIM",
        "HFS_AUTH_ALGORITHMS",
        "HFS_AUTH_JWKS_MIN_REFRESH_INTERVAL",
        "HFS_SMART_TOKEN_ENDPOINT",
        "HFS_SMART_AUTHORIZE_ENDPOINT",
        "HFS_SMART_JWKS_URL",
        "HFS_SMART_INTROSPECTION_ENDPOINT",
        "HFS_SMART_MANAGEMENT_ENDPOINT",
        "HFS_SMART_REGISTRATION_ENDPOINT",
        "HFS_SMART_REVOCATION_ENDPOINT",
        "HFS_OUTBOUND_BEARER_TOKEN",
    ];

    fn clear_auth_env() {
        for key in AUTH_ENV_KEYS {
            unsafe { env::remove_var(key) };
        }
    }

    #[test]
    fn test_from_env_defaults() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_auth_env();

        let config = AuthConfig::from_env();
        assert!(!config.enabled);
        assert_eq!(config.tenant_claim, "tenant_id");
        assert_eq!(config.jwks_min_refresh_interval, 10);
        assert_eq!(
            config.allowed_algorithms,
            vec!["RS256", "RS384", "ES256", "ES384"]
        );
    }

    #[test]
    fn test_default_config() {
        let config = AuthConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.tenant_claim, "tenant_id");
        assert_eq!(config.jwks_min_refresh_interval, 10);
        assert_eq!(config.allowed_algorithms.len(), 4);
    }

    fn enabled_config() -> AuthConfig {
        AuthConfig {
            enabled: true,
            jwks_url: Some("https://idp.example.com/jwks".to_string()),
            expected_issuer: Some("https://idp.example.com".to_string()),
            ..AuthConfig::default()
        }
    }

    #[test]
    fn validate_accepts_a_complete_enabled_config() {
        assert!(enabled_config().validate().is_ok());
    }

    /// A disabled config never consults the other fields.
    #[test]
    fn validate_ignores_a_disabled_config() {
        let config = AuthConfig::default();
        assert!(!config.enabled);
        assert!(config.jwks_url.is_none());
        assert!(config.validate().is_ok());
    }

    /// The issuer pins `iss`, which qualifies every per-user identity — an
    /// enabled config without one must not start (#270).
    #[test]
    fn validate_requires_an_issuer_when_enabled() {
        let config = AuthConfig {
            expected_issuer: None,
            ..enabled_config()
        };
        let errors = config.validate().unwrap_err();
        assert!(
            errors.iter().any(|e| e.contains("HFS_AUTH_ISSUER")),
            "expected an issuer error, got {errors:?}"
        );
    }

    /// A blank value is as absent as a missing one.
    #[test]
    fn validate_rejects_whitespace_only_values() {
        let config = AuthConfig {
            expected_issuer: Some("   ".to_string()),
            jwks_url: Some("".to_string()),
            ..enabled_config()
        };
        let errors = config.validate().unwrap_err();
        assert_eq!(errors.len(), 2, "got {errors:?}");
    }

    /// Every problem is reported at once, not just the first.
    #[test]
    fn validate_accumulates_all_errors() {
        let config = AuthConfig {
            enabled: true,
            jwks_url: None,
            expected_issuer: None,
            allowed_algorithms: vec![],
            ..AuthConfig::default()
        };
        let errors = config.validate().unwrap_err();
        assert_eq!(errors.len(), 3, "got {errors:?}");
    }

    /// Audience stays optional by design — an open demo deployment accepts any
    /// token from its issuer. Callers warn rather than refuse.
    #[test]
    fn validate_does_not_require_an_audience() {
        let config = AuthConfig {
            expected_audience: None,
            ..enabled_config()
        };
        assert!(config.validate().is_ok());
    }
}
