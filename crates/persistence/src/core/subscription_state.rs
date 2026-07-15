//! Shared per-subscription runtime state (design doc §Class B, B3/B4;
//! strategy §8 Phase 3).
//!
//! In a cluster, the per-subscription counters that drive the FHIR
//! Subscriptions contract must live in one shared place: `eventNumber` must
//! be monotonic and gap-free across every instance (subscribers use it for
//! gap detection), and `consecutive_failures` scattered across N nodes must
//! still accumulate to the `error`/`off` thresholds. This module is that
//! place — a [`SubscriptionStateStore`] keyed `(tenant, subscription_id)` —
//! plus the boot-time read path ([`SubscriptionHydrationSource`]) that lets a
//! fresh instance rebuild its in-memory subscription/topic projections from
//! the resources that already persist (B3: today nothing loads them at
//! startup, so a restarted instance knows no subscriptions until their
//! resources are next written).
//!
//! Counter rows are created lazily by the first increment (`ON CONFLICT`
//! upsert), so re-registering a Subscription (any resource update re-runs
//! registration) can never reset shared counters — deliberately NOT
//! replicating the in-memory manager's reset-on-register behavior.
//!
//! The store also holds the built notification bundles for
//! websocket-channel events (`subscription_notification_events`): the
//! instance that matched the event persists the bundle it built, and the
//! instances holding the sockets load it by `(tenant, subscription_id,
//! event_number)` when the fan-out envelope arrives — the NOTIFY payload
//! cap (~8KB) means bundles never ride the fan-out itself.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::error::StorageResult;
use crate::tenant::TenantContext;

/// A snapshot of one subscription's shared runtime state.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscriptionStateRecord {
    /// Number of events delivered since the subscription started — the
    /// cluster-wide `eventsSinceSubscriptionStart` / `eventNumber` source.
    pub event_number: u64,
    /// Consecutive delivery failures across every instance.
    pub consecutive_failures: u32,
    /// Runtime status override (`active`, `error`, `off`), if any instance
    /// has recorded a transition. `None` until the first runtime transition;
    /// the FHIR resource's own `status` field is never written back.
    pub status: Option<String>,
    /// When the state row was last touched.
    pub updated_at: DateTime<Utc>,
}

/// Shared, cluster-visible per-subscription counters and status.
///
/// Tenancy contract: every method is tenant-scoped — one tenant's state rows
/// are invisible to another tenant's calls.
#[async_trait]
pub trait SubscriptionStateStore: Send + Sync {
    /// Atomically increments and returns the subscription's event number.
    ///
    /// Creates the row on first use (starting at 1). Cluster-wide monotonic
    /// and gap-free: two racing callers observe distinct consecutive values.
    async fn next_event_number(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
    ) -> StorageResult<u64>;

    /// Atomically increments and returns the consecutive-failure count.
    /// Creates the row on first use (count 1).
    async fn record_failure(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
    ) -> StorageResult<u32>;

    /// Resets the consecutive-failure count to zero (successful delivery).
    /// A missing row is not an error.
    async fn reset_failures(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
    ) -> StorageResult<()>;

    /// Records a runtime status transition (`error`, `off`, `active`) so
    /// every instance observes it. Creates the row if needed.
    async fn set_status(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
        status: &str,
    ) -> StorageResult<()>;

    /// Tenant-checked read of one subscription's state; `None` when no row
    /// exists *for this tenant*.
    async fn get(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
    ) -> StorageResult<Option<SubscriptionStateRecord>>;

    /// Deletes the state row and any stored notification events (the
    /// Subscription resource was deleted). A missing row is not an error.
    async fn delete(&self, tenant: &TenantContext, subscription_id: &str) -> StorageResult<()>;

    /// Stores a built notification bundle for cross-instance websocket
    /// delivery, keyed by the event number minted for it. Idempotent on key
    /// conflict (last write wins).
    async fn put_notification_event(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
        event_number: u64,
        bundle: &Value,
    ) -> StorageResult<()>;

    /// Tenant-checked read of a stored notification bundle.
    async fn get_notification_event(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
        event_number: u64,
    ) -> StorageResult<Option<Value>>;

    /// Deletes stored notification events older than `cutoff`, returning the
    /// number removed. Deliberately cross-tenant (a reaper duty, like
    /// [`ClusterJobStore::delete_terminal_before`](crate::core::cluster_job_store::ClusterJobStore::delete_terminal_before));
    /// idempotent — safe for every instance to run on a timer.
    async fn prune_notification_events_before(&self, cutoff: DateTime<Utc>) -> StorageResult<u64>;
}

/// One resource row as read back for boot-time hydration.
#[derive(Debug, Clone)]
pub struct HydratedResource {
    /// The tenant that owns the resource.
    pub tenant_id: String,
    /// The FHIR resource type (`Subscription`, `SubscriptionTopic`, `Basic`).
    pub resource_type: String,
    /// The resource id.
    pub resource_id: String,
    /// The stored FHIR version discriminator (mime-param form, e.g. `4.0`),
    /// so the caller can pick the right parser.
    pub fhir_version: String,
    /// The current resource content.
    pub content: Value,
}

/// Boot-time enumeration of the subscription-relevant resources that already
/// persist (B3 startup hydration).
///
/// `list_current` is deliberately cross-tenant and system-level: hydration
/// runs once at boot on behalf of the server, and the tenant registry is
/// opt-in metadata that may not cover every tenant with resources — so the
/// read must not iterate registered tenants. The per-row `tenant_id` lets
/// the caller rebuild correctly tenant-scoped projections.
#[async_trait]
pub trait SubscriptionHydrationSource: Send + Sync {
    /// Returns every current (non-deleted) resource of the given types,
    /// across all tenants. For `Basic`, implementations should pre-filter to
    /// R4 backport subscription topics where they can (the
    /// `http://hl7.org/fhir/fhir-types` / `SubscriptionTopic` code marker)
    /// rather than returning every `Basic` row.
    async fn list_current(&self, resource_types: &[&str]) -> StorageResult<Vec<HydratedResource>>;

    /// Tenant-checked read of one current resource — the runtime
    /// re-read used when a lifecycle fan-out envelope announces a change.
    /// `None` when the resource does not exist (or is deleted) *for this
    /// tenant*.
    async fn get_current(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<Option<HydratedResource>>;
}

/// Test support: in-memory reference implementations of the same contracts
/// as the database backends.
///
/// NOT cluster-safe production backends — two instances would each have
/// their own maps (the "unsafe contract" `HFS_CLUSTER=true` refuses). They
/// exist so trait consumers can be unit-tested without a database and so the
/// contracts have T1 reference models.
pub mod testing {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct StateEntry {
        event_number: u64,
        consecutive_failures: u32,
        status: Option<String>,
        updated_at: Option<DateTime<Utc>>,
    }

    /// `(tenant_id, subscription_id, event_number)` keying a stored bundle.
    type EventKey = (String, String, u64);
    /// A stored bundle and when it was stored (for pruning).
    type StoredEvent = (Value, DateTime<Utc>);

    /// See [module docs](self::super) — the T1 reference
    /// [`SubscriptionStateStore`].
    #[derive(Default)]
    pub struct InMemorySubscriptionStateStore {
        state: Mutex<HashMap<(String, String), StateEntry>>,
        events: Mutex<HashMap<EventKey, StoredEvent>>,
    }

    impl InMemorySubscriptionStateStore {
        /// Creates an empty store.
        pub fn new() -> Self {
            Self::default()
        }
    }

    fn key(tenant: &TenantContext, subscription_id: &str) -> (String, String) {
        (
            tenant.tenant_id().as_str().to_string(),
            subscription_id.to_string(),
        )
    }

    #[async_trait]
    impl SubscriptionStateStore for InMemorySubscriptionStateStore {
        async fn next_event_number(
            &self,
            tenant: &TenantContext,
            subscription_id: &str,
        ) -> StorageResult<u64> {
            let mut state = self.state.lock().unwrap();
            let entry = state.entry(key(tenant, subscription_id)).or_default();
            entry.event_number += 1;
            entry.updated_at = Some(Utc::now());
            Ok(entry.event_number)
        }

        async fn record_failure(
            &self,
            tenant: &TenantContext,
            subscription_id: &str,
        ) -> StorageResult<u32> {
            let mut state = self.state.lock().unwrap();
            let entry = state.entry(key(tenant, subscription_id)).or_default();
            entry.consecutive_failures += 1;
            entry.updated_at = Some(Utc::now());
            Ok(entry.consecutive_failures)
        }

        async fn reset_failures(
            &self,
            tenant: &TenantContext,
            subscription_id: &str,
        ) -> StorageResult<()> {
            let mut state = self.state.lock().unwrap();
            if let Some(entry) = state.get_mut(&key(tenant, subscription_id)) {
                entry.consecutive_failures = 0;
                entry.updated_at = Some(Utc::now());
            }
            Ok(())
        }

        async fn set_status(
            &self,
            tenant: &TenantContext,
            subscription_id: &str,
            status: &str,
        ) -> StorageResult<()> {
            let mut state = self.state.lock().unwrap();
            let entry = state.entry(key(tenant, subscription_id)).or_default();
            entry.status = Some(status.to_string());
            entry.updated_at = Some(Utc::now());
            Ok(())
        }

        async fn get(
            &self,
            tenant: &TenantContext,
            subscription_id: &str,
        ) -> StorageResult<Option<SubscriptionStateRecord>> {
            let state = self.state.lock().unwrap();
            Ok(state
                .get(&key(tenant, subscription_id))
                .map(|e| SubscriptionStateRecord {
                    event_number: e.event_number,
                    consecutive_failures: e.consecutive_failures,
                    status: e.status.clone(),
                    updated_at: e.updated_at.unwrap_or_else(Utc::now),
                }))
        }

        async fn delete(&self, tenant: &TenantContext, subscription_id: &str) -> StorageResult<()> {
            self.state
                .lock()
                .unwrap()
                .remove(&key(tenant, subscription_id));
            let tenant_id = tenant.tenant_id().as_str();
            self.events
                .lock()
                .unwrap()
                .retain(|(t, s, _), _| !(t == tenant_id && s == subscription_id));
            Ok(())
        }

        async fn put_notification_event(
            &self,
            tenant: &TenantContext,
            subscription_id: &str,
            event_number: u64,
            bundle: &Value,
        ) -> StorageResult<()> {
            self.events.lock().unwrap().insert(
                (
                    tenant.tenant_id().as_str().to_string(),
                    subscription_id.to_string(),
                    event_number,
                ),
                (bundle.clone(), Utc::now()),
            );
            Ok(())
        }

        async fn get_notification_event(
            &self,
            tenant: &TenantContext,
            subscription_id: &str,
            event_number: u64,
        ) -> StorageResult<Option<Value>> {
            Ok(self
                .events
                .lock()
                .unwrap()
                .get(&(
                    tenant.tenant_id().as_str().to_string(),
                    subscription_id.to_string(),
                    event_number,
                ))
                .map(|(bundle, _)| bundle.clone()))
        }

        async fn prune_notification_events_before(
            &self,
            cutoff: DateTime<Utc>,
        ) -> StorageResult<u64> {
            let mut events = self.events.lock().unwrap();
            let before = events.len();
            events.retain(|_, (_, created_at)| *created_at >= cutoff);
            Ok((before - events.len()) as u64)
        }
    }

    /// See [module docs](self::super) — the T1 reference
    /// [`SubscriptionHydrationSource`], seeded by tests.
    #[derive(Default)]
    pub struct InMemoryHydrationSource {
        resources: Mutex<Vec<HydratedResource>>,
    }

    impl InMemoryHydrationSource {
        /// Creates an empty source.
        pub fn new() -> Self {
            Self::default()
        }

        /// Test-only: seeds a resource row.
        pub fn insert(&self, resource: HydratedResource) {
            self.resources.lock().unwrap().push(resource);
        }

        /// Test-only: removes a resource row (simulates a delete).
        pub fn remove(&self, tenant_id: &str, resource_type: &str, resource_id: &str) {
            self.resources.lock().unwrap().retain(|r| {
                !(r.tenant_id == tenant_id
                    && r.resource_type == resource_type
                    && r.resource_id == resource_id)
            });
        }
    }

    #[async_trait]
    impl SubscriptionHydrationSource for InMemoryHydrationSource {
        async fn list_current(
            &self,
            resource_types: &[&str],
        ) -> StorageResult<Vec<HydratedResource>> {
            Ok(self
                .resources
                .lock()
                .unwrap()
                .iter()
                .filter(|r| resource_types.contains(&r.resource_type.as_str()))
                .cloned()
                .collect())
        }

        async fn get_current(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            resource_id: &str,
        ) -> StorageResult<Option<HydratedResource>> {
            let tenant_id = tenant.tenant_id().as_str();
            Ok(self
                .resources
                .lock()
                .unwrap()
                .iter()
                .find(|r| {
                    r.tenant_id == tenant_id
                        && r.resource_type == resource_type
                        && r.resource_id == resource_id
                })
                .cloned())
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
    async fn event_numbers_are_monotonic_and_start_at_one() {
        let store = InMemorySubscriptionStateStore::new();
        let t = tenant("t1");
        assert_eq!(store.next_event_number(&t, "s1").await.unwrap(), 1);
        assert_eq!(store.next_event_number(&t, "s1").await.unwrap(), 2);
        assert_eq!(store.next_event_number(&t, "s1").await.unwrap(), 3);
        // A different subscription counts independently.
        assert_eq!(store.next_event_number(&t, "s2").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn failures_accumulate_and_reset() {
        let store = InMemorySubscriptionStateStore::new();
        let t = tenant("t1");
        assert_eq!(store.record_failure(&t, "s1").await.unwrap(), 1);
        assert_eq!(store.record_failure(&t, "s1").await.unwrap(), 2);
        store.reset_failures(&t, "s1").await.unwrap();
        assert_eq!(store.record_failure(&t, "s1").await.unwrap(), 1);
        // Resetting a missing row is not an error.
        store.reset_failures(&t, "nope").await.unwrap();
    }

    #[tokio::test]
    async fn state_is_tenant_scoped() {
        let store = InMemorySubscriptionStateStore::new();
        let a = tenant("tenant-a");
        let b = tenant("tenant-b");
        store.next_event_number(&a, "s1").await.unwrap();
        assert!(store.get(&b, "s1").await.unwrap().is_none());
        // Tenant B's increment starts its own row, not tenant A's.
        assert_eq!(store.next_event_number(&b, "s1").await.unwrap(), 1);
        assert_eq!(store.get(&a, "s1").await.unwrap().unwrap().event_number, 1);
    }

    #[tokio::test]
    async fn status_and_delete_lifecycle() {
        let store = InMemorySubscriptionStateStore::new();
        let t = tenant("t1");
        store.set_status(&t, "s1", "error").await.unwrap();
        assert_eq!(
            store
                .get(&t, "s1")
                .await
                .unwrap()
                .unwrap()
                .status
                .as_deref(),
            Some("error")
        );
        store
            .put_notification_event(&t, "s1", 1, &serde_json::json!({"resourceType":"Bundle"}))
            .await
            .unwrap();
        store.delete(&t, "s1").await.unwrap();
        assert!(store.get(&t, "s1").await.unwrap().is_none());
        assert!(
            store
                .get_notification_event(&t, "s1", 1)
                .await
                .unwrap()
                .is_none(),
            "delete must remove stored notification events too"
        );
    }

    #[tokio::test]
    async fn notification_events_roundtrip_and_are_tenant_checked() {
        let store = InMemorySubscriptionStateStore::new();
        let a = tenant("tenant-a");
        let b = tenant("tenant-b");
        let bundle =
            serde_json::json!({"resourceType": "Bundle", "type": "subscription-notification"});
        store
            .put_notification_event(&a, "s1", 7, &bundle)
            .await
            .unwrap();
        assert_eq!(
            store.get_notification_event(&a, "s1", 7).await.unwrap(),
            Some(bundle)
        );
        assert!(
            store
                .get_notification_event(&b, "s1", 7)
                .await
                .unwrap()
                .is_none(),
            "another tenant's event must be invisible"
        );
        let pruned = store
            .prune_notification_events_before(Utc::now() + chrono::Duration::seconds(1))
            .await
            .unwrap();
        assert_eq!(pruned, 1);
    }

    #[tokio::test]
    async fn hydration_source_filters_by_type_and_tenant_checks_get() {
        let source = InMemoryHydrationSource::new();
        source.insert(HydratedResource {
            tenant_id: "tenant-a".into(),
            resource_type: "SubscriptionTopic".into(),
            resource_id: "topic-1".into(),
            fhir_version: "5.0".into(),
            content: serde_json::json!({"resourceType": "SubscriptionTopic"}),
        });
        source.insert(HydratedResource {
            tenant_id: "tenant-b".into(),
            resource_type: "Subscription".into(),
            resource_id: "sub-1".into(),
            fhir_version: "5.0".into(),
            content: serde_json::json!({"resourceType": "Subscription"}),
        });

        let topics = source.list_current(&["SubscriptionTopic"]).await.unwrap();
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].tenant_id, "tenant-a");

        let both = source
            .list_current(&["SubscriptionTopic", "Subscription"])
            .await
            .unwrap();
        assert_eq!(both.len(), 2);

        assert!(
            source
                .get_current(&tenant("tenant-a"), "Subscription", "sub-1")
                .await
                .unwrap()
                .is_none(),
            "get_current must be tenant-checked"
        );
        assert!(
            source
                .get_current(&tenant("tenant-b"), "Subscription", "sub-1")
                .await
                .unwrap()
                .is_some()
        );
    }
}
