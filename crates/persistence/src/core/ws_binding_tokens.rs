//! Shared WebSocket binding tokens (design doc §Class B, B2; strategy §8
//! Phase 3).
//!
//! `$get-ws-binding-token` mints a single-use, short-lived (default 30s)
//! opaque token; the client presents it on a *separate* WebSocket-upgrade
//! connection that a load balancer may route to any instance. The
//! in-process `WsBindingTokenManager` map makes every cross-instance bind
//! fail; this trait is the shared, redeem-once replacement — mint on any
//! instance, redeem exactly once on any instance.
//!
//! `redeem` is deliberately NOT tenant-scoped: the token itself is the
//! credential (an unguessable UUID handed to the authenticated caller), and
//! the redeeming WebSocket connection has no tenant context of its own —
//! the token *returns* the tenant it was minted for. Same posture as the
//! cross-tenant `claim_next` on the cluster job store.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::StorageResult;
use crate::tenant::TenantContext;

/// Shared single-use WebSocket binding tokens.
#[async_trait]
pub trait WsBindingTokenStore: Send + Sync {
    /// Mints a new single-use token for the subscription, valid for `ttl`
    /// on the store's clock. Returns the token and its expiry.
    ///
    /// Implementations should opportunistically purge already-expired rows
    /// (mirroring the in-memory manager's lazy cleanup).
    async fn mint(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
        ttl: Duration,
    ) -> StorageResult<(String, DateTime<Utc>)>;

    /// Redeems a token, consuming it: returns `(tenant_id, subscription_id)`
    /// exactly once for a live token, `None` for an unknown, already-redeemed,
    /// or expired one. An expired token is still consumed.
    async fn redeem(&self, token: &str) -> StorageResult<Option<(String, String)>>;
}

/// Test support: an in-memory [`WsBindingTokenStore`] implementing the same
/// redeem-once contract as the database backends.
///
/// NOT a cluster-safe production backend — two instances would each have
/// their own map (exactly the B2 failure this trait exists to fix). It is
/// the T1 reference model.
pub mod testing {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    /// `(tenant_id, subscription_id, expires_at)` for one minted token.
    type TokenEntry = (String, String, DateTime<Utc>);

    /// See [module docs](self::super).
    #[derive(Default)]
    pub struct InMemoryWsTokenStore {
        tokens: Mutex<HashMap<String, TokenEntry>>,
    }

    impl InMemoryWsTokenStore {
        /// Creates an empty store.
        pub fn new() -> Self {
            Self::default()
        }

        /// Test-only: seeds a token with an explicit (typically past)
        /// expiry, so expiry paths can be exercised without sleeps.
        pub fn insert_backdated(
            &self,
            token: &str,
            tenant_id: &str,
            subscription_id: &str,
            expires_at: DateTime<Utc>,
        ) {
            self.tokens.lock().unwrap().insert(
                token.to_string(),
                (
                    tenant_id.to_string(),
                    subscription_id.to_string(),
                    expires_at,
                ),
            );
        }
    }

    #[async_trait]
    impl WsBindingTokenStore for InMemoryWsTokenStore {
        async fn mint(
            &self,
            tenant: &TenantContext,
            subscription_id: &str,
            ttl: Duration,
        ) -> StorageResult<(String, DateTime<Utc>)> {
            let now = Utc::now();
            let mut tokens = self.tokens.lock().unwrap();
            tokens.retain(|_, (_, _, expires_at)| *expires_at > now);
            let token = uuid::Uuid::new_v4().to_string();
            let expires_at = now + chrono::Duration::from_std(ttl).unwrap_or_default();
            tokens.insert(
                token.clone(),
                (
                    tenant.tenant_id().as_str().to_string(),
                    subscription_id.to_string(),
                    expires_at,
                ),
            );
            Ok((token, expires_at))
        }

        async fn redeem(&self, token: &str) -> StorageResult<Option<(String, String)>> {
            let removed = self.tokens.lock().unwrap().remove(token);
            Ok(
                removed.and_then(|(tenant_id, subscription_id, expires_at)| {
                    (expires_at > Utc::now()).then_some((tenant_id, subscription_id))
                }),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::tenant::{TenantId, TenantPermissions};

    fn tenant(id: &str) -> TenantContext {
        TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
    }

    #[tokio::test]
    async fn redeem_is_single_use_and_returns_the_minting_tenant() {
        let store = InMemoryWsTokenStore::new();
        let (token, expires_at) = store
            .mint(&tenant("tenant-a"), "sub-1", Duration::from_secs(30))
            .await
            .unwrap();
        assert!(expires_at > Utc::now());

        let redeemed = store.redeem(&token).await.unwrap();
        assert_eq!(
            redeemed,
            Some(("tenant-a".to_string(), "sub-1".to_string()))
        );
        assert!(
            store.redeem(&token).await.unwrap().is_none(),
            "second redeem must fail"
        );
    }

    #[tokio::test]
    async fn expired_tokens_redeem_as_none_and_are_consumed() {
        let store = InMemoryWsTokenStore::new();
        store.insert_backdated(
            "expired-token",
            "tenant-a",
            "sub-1",
            Utc::now() - chrono::Duration::seconds(1),
        );
        assert!(store.redeem("expired-token").await.unwrap().is_none());
        assert!(
            store.redeem("expired-token").await.unwrap().is_none(),
            "an expired token is consumed on first redeem attempt"
        );
    }

    #[tokio::test]
    async fn unknown_tokens_redeem_as_none() {
        let store = InMemoryWsTokenStore::new();
        assert!(store.redeem("never-minted").await.unwrap().is_none());
    }
}
