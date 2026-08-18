//! Cluster event fan-out for subscriptions (design doc §Class B, B1;
//! strategy §8 Phase 3).
//!
//! WebSocket sockets are inherently node-local: only the instance
//! terminating a socket can deliver to it. [`EventFanout`] broadcasts small
//! envelopes to every instance so each can deliver to *its* local sockets,
//! and carries the subscription/topic lifecycle announcements that keep
//! every instance's in-memory projections in sync with the resources in the
//! database.
//!
//! Envelopes are deliberately tiny references, never payloads: the Postgres
//! implementation rides `NOTIFY`, whose payload caps at ~8KB — notification
//! bundles live in the shared store
//! ([`subscription_state`](crate::core::subscription_state)) and receivers
//! rehydrate by key.
//!
//! The fan-out is a **best-effort wake/refresh signal, never a correctness
//! dependency**: envelopes published while a receiver is disconnected are
//! lost. Receivers get a locally synthesized [`FanoutKind::Resync`] after a
//! reconnect and respond with a full re-hydration; websocket notifications
//! missed during the gap are gap-detectable by the client via
//! `eventNumber`; and the durable delivery outbox polls on a floor interval
//! regardless of wake hints.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::error::StorageResult;

/// The lifecycle operation a [`FanoutKind::Lifecycle`] envelope announces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LifecycleOp {
    /// The resource was created or updated — receivers re-read it and
    /// re-register locally.
    Upsert,
    /// The resource was deleted — receivers deregister locally.
    Delete,
}

/// The kind-discriminated body of a fan-out envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FanoutKind {
    /// A websocket-channel notification was produced: receivers load the
    /// stored bundle by `(tenant, sub, event)` and deliver to their local
    /// sockets. `origin` is the publishing instance's worker id — the
    /// origin already delivered locally and skips its own envelopes.
    WsEvent {
        /// Publishing instance's worker id.
        origin: String,
        /// Tenant that owns the subscription.
        tenant: String,
        /// The subscription id.
        sub: String,
        /// The event number keying the stored bundle.
        event: u64,
    },
    /// A Subscription / SubscriptionTopic / R4 `Basic` topic resource
    /// changed: receivers re-read it from storage and patch their local
    /// projections.
    Lifecycle {
        /// Tenant that owns the resource.
        tenant: String,
        /// The resource type (`Subscription`, `SubscriptionTopic`, `Basic`).
        rtype: String,
        /// The resource id.
        rid: String,
        /// Whether to re-register or deregister.
        op: LifecycleOp,
    },
    /// A subscription's shared runtime state changed (status flip, e.g.
    /// `error`/`off`): receivers refresh their local snapshot from the
    /// state store.
    State {
        /// Tenant that owns the subscription.
        tenant: String,
        /// The subscription id.
        sub: String,
    },
    /// Synthesized locally by the fan-out implementation after a listen
    /// reconnect (never published): envelopes may have been missed —
    /// receivers should run a full re-hydration.
    Resync,
}

/// One fan-out envelope: a version marker plus the kind-discriminated body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FanoutEnvelope {
    /// Envelope schema version (currently 1). Receivers ignore envelopes
    /// with a version they don't understand.
    #[serde(default = "FanoutEnvelope::current_version")]
    pub v: u32,
    /// The envelope body.
    #[serde(flatten)]
    pub kind: FanoutKind,
}

impl FanoutEnvelope {
    /// The current envelope schema version.
    pub fn current_version() -> u32 {
        1
    }

    /// Wraps a body in a current-version envelope.
    pub fn new(kind: FanoutKind) -> Self {
        Self {
            v: Self::current_version(),
            kind,
        }
    }
}

/// Cluster-wide broadcast of subscription envelopes and outbox wake hints.
///
/// Every instance both publishes and subscribes; a published envelope
/// reaches every *currently connected* subscriber, including the publisher
/// itself (origin filtering is the consumer's job, via
/// [`FanoutKind::WsEvent::origin`]).
#[async_trait]
pub trait EventFanout: Send + Sync {
    /// Resolves once this instance is actually receiving envelopes (e.g.
    /// the Postgres implementation's `LISTEN` session is established).
    /// Callers that must not miss the next envelope await this after
    /// subscribing; in-process implementations resolve immediately (the
    /// default).
    async fn ready(&self) {}

    /// Publishes an envelope to every instance.
    async fn publish(&self, envelope: &FanoutEnvelope) -> StorageResult<()>;

    /// Subscribes to envelopes published from now on (including locally
    /// synthesized [`FanoutKind::Resync`] markers after reconnects).
    fn subscribe(&self) -> broadcast::Receiver<FanoutEnvelope>;

    /// Publishes an outbox wake hint: "a delivery row was enqueued, claim
    /// now instead of waiting out the poll interval". Purely advisory.
    async fn publish_outbox_wake(&self) -> StorageResult<()>;

    /// Subscribes to outbox wake hints.
    fn subscribe_outbox_wake(&self) -> broadcast::Receiver<()>;
}

/// Test support: an in-process [`EventFanout`] over shared broadcast
/// channels.
///
/// NOT a cluster-safe production backend — it only reaches subscribers in
/// the same process (exactly the B1 failure the fan-out exists to fix). It
/// exists so two engine handles in one test can share a bus and simulate
/// two instances, and as the T1 reference model.
pub mod testing {
    use super::*;

    /// See [module docs](self::super).
    pub struct InMemoryEventFanout {
        envelopes: broadcast::Sender<FanoutEnvelope>,
        wakes: broadcast::Sender<()>,
    }

    impl InMemoryEventFanout {
        /// Creates a fan-out bus with room for `capacity` in-flight
        /// envelopes per subscriber.
        pub fn new(capacity: usize) -> Self {
            Self {
                envelopes: broadcast::channel(capacity).0,
                wakes: broadcast::channel(capacity).0,
            }
        }

        /// Test-only: injects a locally-synthesized envelope (e.g.
        /// [`FanoutKind::Resync`]) as a real fan-out impl would after a
        /// reconnect.
        pub fn inject(&self, envelope: FanoutEnvelope) {
            let _ = self.envelopes.send(envelope);
        }
    }

    impl Default for InMemoryEventFanout {
        fn default() -> Self {
            Self::new(256)
        }
    }

    #[async_trait]
    impl EventFanout for InMemoryEventFanout {
        async fn publish(&self, envelope: &FanoutEnvelope) -> StorageResult<()> {
            // No subscribers is not an error (matches NOTIFY semantics).
            let _ = self.envelopes.send(envelope.clone());
            Ok(())
        }

        fn subscribe(&self) -> broadcast::Receiver<FanoutEnvelope> {
            self.envelopes.subscribe()
        }

        async fn publish_outbox_wake(&self) -> StorageResult<()> {
            let _ = self.wakes.send(());
            Ok(())
        }

        fn subscribe_outbox_wake(&self) -> broadcast::Receiver<()> {
            self.wakes.subscribe()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn envelope_json_shape_is_stable() {
        let envelope = FanoutEnvelope::new(FanoutKind::WsEvent {
            origin: "w-1".into(),
            tenant: "t1".into(),
            sub: "s1".into(),
            event: 42,
        });
        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "v": 1,
                "kind": "ws-event",
                "origin": "w-1",
                "tenant": "t1",
                "sub": "s1",
                "event": 42
            })
        );

        let lifecycle = FanoutEnvelope::new(FanoutKind::Lifecycle {
            tenant: "t1".into(),
            rtype: "SubscriptionTopic".into(),
            rid: "topic-1".into(),
            op: LifecycleOp::Upsert,
        });
        let json = serde_json::to_string(&lifecycle).unwrap();
        let parsed: FanoutEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, lifecycle);

        // A missing "v" defaults to the current version (older publishers).
        let parsed: FanoutEnvelope =
            serde_json::from_str(r#"{"kind":"state","tenant":"t1","sub":"s1"}"#).unwrap();
        assert_eq!(parsed.v, 1);
        assert_eq!(
            parsed.kind,
            FanoutKind::State {
                tenant: "t1".into(),
                sub: "s1".into()
            }
        );
    }

    #[tokio::test]
    async fn publish_reaches_every_subscriber_including_the_publisher() {
        let bus = InMemoryEventFanout::default();
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        let envelope = FanoutEnvelope::new(FanoutKind::State {
            tenant: "t1".into(),
            sub: "s1".into(),
        });
        bus.publish(&envelope).await.unwrap();
        assert_eq!(a.recv().await.unwrap(), envelope);
        assert_eq!(b.recv().await.unwrap(), envelope);
    }

    #[tokio::test]
    async fn publish_without_subscribers_is_not_an_error() {
        let bus = InMemoryEventFanout::default();
        bus.publish(&FanoutEnvelope::new(FanoutKind::Resync))
            .await
            .unwrap();
        bus.publish_outbox_wake().await.unwrap();
    }

    #[tokio::test]
    async fn wake_hints_flow_on_their_own_channel() {
        let bus = InMemoryEventFanout::default();
        let mut envelopes = bus.subscribe();
        let mut wakes = bus.subscribe_outbox_wake();
        bus.publish_outbox_wake().await.unwrap();
        wakes.recv().await.unwrap();
        assert!(
            matches!(
                envelopes.try_recv(),
                Err(broadcast::error::TryRecvError::Empty)
            ),
            "wake hints must not appear on the envelope channel"
        );
    }
}
