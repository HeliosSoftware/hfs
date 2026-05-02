//! WebSocket binding token manager.
//!
//! Generates and validates short-lived opaque tokens used by clients to bind
//! a WebSocket connection to a specific subscription. Tokens are single-use
//! and expire after a configurable lifetime (default 30 seconds per FHIR spec).

use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use uuid::Uuid;

/// Metadata associated with a binding token.
struct WsBindingToken {
    subscription_id: String,
    tenant_id: String,
    expires_at: DateTime<Utc>,
}

/// Manages short-lived WebSocket binding tokens.
///
/// Clients call `$get-ws-binding-token` to obtain a token, then present it
/// as a query parameter when connecting to the WebSocket endpoint. Tokens
/// are consumed on first use and automatically expire.
pub struct WsBindingTokenManager {
    tokens: DashMap<String, WsBindingToken>,
    token_lifetime_secs: i64,
}

impl WsBindingTokenManager {
    /// Creates a new token manager with the given token lifetime.
    pub fn new(token_lifetime_secs: i64) -> Self {
        Self {
            tokens: DashMap::new(),
            token_lifetime_secs,
        }
    }

    /// Generates a short-lived binding token for a subscription.
    ///
    /// Returns `(token_string, expiration_datetime)`.
    pub fn generate_token(
        &self,
        tenant_id: &str,
        subscription_id: &str,
    ) -> (String, DateTime<Utc>) {
        // Lazily clean up expired tokens.
        self.cleanup_expired();

        let token = Uuid::new_v4().to_string();
        let expires_at = Utc::now() + Duration::seconds(self.token_lifetime_secs);

        self.tokens.insert(
            token.clone(),
            WsBindingToken {
                subscription_id: subscription_id.to_string(),
                tenant_id: tenant_id.to_string(),
                expires_at,
            },
        );

        (token, expires_at)
    }

    /// Validates and consumes a token (single-use).
    ///
    /// Returns `Some((tenant_id, subscription_id))` on success, `None` if
    /// the token is invalid, expired, or already consumed.
    pub fn validate_and_consume(&self, token: &str) -> Option<(String, String)> {
        let (_, binding) = self.tokens.remove(token)?;

        if Utc::now() > binding.expires_at {
            return None;
        }

        Some((binding.tenant_id, binding.subscription_id))
    }

    /// Removes all expired tokens.
    pub fn cleanup_expired(&self) {
        let now = Utc::now();
        self.tokens.retain(|_, v| v.expires_at > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_token() {
        let mgr = WsBindingTokenManager::new(30);
        let (token, expiry) = mgr.generate_token("tenant-1", "sub-1");

        assert!(!token.is_empty());
        assert!(expiry > Utc::now());
    }

    #[test]
    fn test_validate_and_consume_success() {
        let mgr = WsBindingTokenManager::new(30);
        let (token, _) = mgr.generate_token("tenant-1", "sub-1");

        let result = mgr.validate_and_consume(&token);
        assert!(result.is_some());

        let (tenant, sub) = result.unwrap();
        assert_eq!(tenant, "tenant-1");
        assert_eq!(sub, "sub-1");
    }

    #[test]
    fn test_token_single_use() {
        let mgr = WsBindingTokenManager::new(30);
        let (token, _) = mgr.generate_token("tenant-1", "sub-1");

        assert!(mgr.validate_and_consume(&token).is_some());
        assert!(mgr.validate_and_consume(&token).is_none());
    }

    #[test]
    fn test_invalid_token() {
        let mgr = WsBindingTokenManager::new(30);
        assert!(mgr.validate_and_consume("nonexistent-token").is_none());
    }

    #[test]
    fn test_expired_token() {
        let mgr = WsBindingTokenManager::new(0);
        let (token, _) = mgr.generate_token("tenant-1", "sub-1");

        // Token expires immediately (0-second lifetime).
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(mgr.validate_and_consume(&token).is_none());
    }

    #[test]
    fn test_cleanup_expired() {
        let mgr = WsBindingTokenManager::new(0);
        mgr.generate_token("t1", "s1");
        mgr.generate_token("t1", "s2");

        std::thread::sleep(std::time::Duration::from_millis(10));
        mgr.cleanup_expired();

        assert_eq!(mgr.tokens.len(), 0);
    }
}
