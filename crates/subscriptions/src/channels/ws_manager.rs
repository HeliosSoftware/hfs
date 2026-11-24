//! WebSocket client connection manager.
//!
//! Tracks connected WebSocket clients per subscription. The subscriptions
//! crate uses this to broadcast notifications, while the REST crate uses
//! it to register new connections from the WebSocket upgrade handler.
//!
//! Each client has a bounded queue of [`CLIENT_QUEUE_CAPACITY`]
//! notifications. A client that stops reading its socket stops draining the
//! queue; once the queue is full the client is dropped rather than letting
//! every further matching write retain another bundle in the process until
//! the socket closes. The client reconnects and replays through `$events`.

use dashmap::DashMap;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tracing::{debug, warn};
use uuid::Uuid;

/// Notifications a bound client may have waiting before it counts as not
/// draining and is dropped.
pub const CLIENT_QUEUE_CAPACITY: usize = 256;

/// Sender half given to a connected WebSocket client.
pub type WsClientSender = mpsc::Sender<serde_json::Value>;

/// Manages connected WebSocket clients for all active subscriptions.
pub struct WebSocketManager {
    /// Connected clients keyed by (tenant_id, subscription_id).
    /// Each entry is a vec of (client_id, sender).
    clients: DashMap<(String, String), Vec<(String, WsClientSender)>>,
}

impl WebSocketManager {
    /// Creates a new empty manager.
    pub fn new() -> Self {
        Self {
            clients: DashMap::new(),
        }
    }

    /// Registers a new WebSocket client for a subscription.
    ///
    /// Returns `(client_id, receiver)`. The caller (WebSocket handler) reads
    /// notifications from the receiver and forwards them over the socket.
    /// The receiver is closed ([`mpsc::Receiver::is_closed`]) as soon as the
    /// subscription is deregistered or the client let
    /// [`CLIENT_QUEUE_CAPACITY`] notifications pile up, while the queued ones
    /// are still buffered; the handler stops there instead of draining them.
    pub fn register_client(
        &self,
        tenant_id: &str,
        subscription_id: &str,
    ) -> (String, mpsc::Receiver<serde_json::Value>) {
        let (tx, rx) = mpsc::channel(CLIENT_QUEUE_CAPACITY);
        let client_id = Uuid::new_v4().to_string();

        self.clients
            .entry((tenant_id.to_string(), subscription_id.to_string()))
            .or_default()
            .push((client_id.clone(), tx));

        debug!(
            tenant_id,
            subscription_id,
            client_id = %client_id,
            "WebSocket client registered"
        );

        (client_id, rx)
    }

    /// Removes a specific client by ID (called on disconnect).
    pub fn remove_client(&self, tenant_id: &str, subscription_id: &str, client_id: &str) {
        let key = (tenant_id.to_string(), subscription_id.to_string());

        if let Some(mut entry) = self.clients.get_mut(&key) {
            entry.retain(|(id, _)| id != client_id);
            let is_empty = entry.is_empty();
            drop(entry);

            // Clean up empty entries.
            if is_empty {
                self.clients.remove(&key);
            }
        }

        debug!(
            tenant_id,
            subscription_id, client_id, "WebSocket client removed"
        );
    }

    /// Broadcasts a notification to all connected clients for a subscription.
    ///
    /// Automatically prunes disconnected clients (closed channels) and drops
    /// a client whose queue is full, so one client that stopped reading
    /// cannot retain a copy of every later notification. Returns the number
    /// of clients that received the message.
    pub fn send_to_subscription(
        &self,
        tenant_id: &str,
        subscription_id: &str,
        notification: &serde_json::Value,
    ) -> usize {
        let key = (tenant_id.to_string(), subscription_id.to_string());

        let Some(mut entry) = self.clients.get_mut(&key) else {
            return 0;
        };

        let mut delivered = 0;
        let initial_count = entry.len();

        // Send to all clients, removing those with closed or full channels.
        entry.retain(
            |(client_id, sender)| match sender.try_send(notification.clone()) {
                Ok(()) => {
                    delivered += 1;
                    true
                }
                Err(TrySendError::Full(_)) => {
                    warn!(
                        tenant_id,
                        subscription_id,
                        client_id = %client_id,
                        queued = CLIENT_QUEUE_CAPACITY,
                        "WebSocket client is not draining its notifications; dropping it"
                    );
                    false
                }
                Err(TrySendError::Closed(_)) => {
                    debug!(client_id = %client_id, "Pruning disconnected WebSocket client");
                    false
                }
            },
        );

        let pruned = initial_count - entry.len();
        if pruned > 0 {
            warn!(
                tenant_id,
                subscription_id, pruned, "Pruned disconnected or lagging WebSocket clients"
            );
        }

        // Clean up empty entries.
        if entry.is_empty() {
            drop(entry);
            self.clients.remove(&key);
        }

        delivered
    }

    /// Returns the number of connected clients for a subscription.
    pub fn client_count(&self, tenant_id: &str, subscription_id: &str) -> usize {
        let key = (tenant_id.to_string(), subscription_id.to_string());
        self.clients.get(&key).map(|e| e.len()).unwrap_or(0)
    }

    /// Removes all clients for a subscription (called on deregistration).
    ///
    /// Dropping the senders causes receivers to return `None`, which
    /// triggers graceful close in the WebSocket handlers.
    pub fn remove_all_clients(&self, tenant_id: &str, subscription_id: &str) {
        let key = (tenant_id.to_string(), subscription_id.to_string());
        if let Some((_, clients)) = self.clients.remove(&key) {
            debug!(
                tenant_id,
                subscription_id,
                count = clients.len(),
                "Removed all WebSocket clients for subscription"
            );
        }
    }
}

impl Default for WebSocketManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_register_and_receive() {
        let mgr = WebSocketManager::new();
        let (_client_id, mut rx) = mgr.register_client("t1", "sub-1");

        let notification = json!({"resourceType": "Bundle", "type": "history"});
        let count = mgr.send_to_subscription("t1", "sub-1", &notification);

        assert_eq!(count, 1);
        let received = rx.recv().await.unwrap();
        assert_eq!(received, notification);
    }

    #[tokio::test]
    async fn test_multi_client_broadcast() {
        let mgr = WebSocketManager::new();
        let (_id1, mut rx1) = mgr.register_client("t1", "sub-1");
        let (_id2, mut rx2) = mgr.register_client("t1", "sub-1");

        let notification = json!({"type": "event-notification"});
        let count = mgr.send_to_subscription("t1", "sub-1", &notification);

        assert_eq!(count, 2);
        assert_eq!(rx1.recv().await.unwrap(), notification);
        assert_eq!(rx2.recv().await.unwrap(), notification);
    }

    #[tokio::test]
    async fn test_disconnected_client_pruned() {
        let mgr = WebSocketManager::new();
        let (_id1, rx1) = mgr.register_client("t1", "sub-1");
        let (_id2, mut rx2) = mgr.register_client("t1", "sub-1");

        // Drop rx1 to simulate disconnect.
        drop(rx1);

        let notification = json!({"type": "event-notification"});
        let count = mgr.send_to_subscription("t1", "sub-1", &notification);

        assert_eq!(count, 1);
        assert_eq!(rx2.recv().await.unwrap(), notification);
        assert_eq!(mgr.client_count("t1", "sub-1"), 1);
    }

    #[test]
    fn test_remove_client() {
        let mgr = WebSocketManager::new();
        let (id1, _rx1) = mgr.register_client("t1", "sub-1");
        let (_id2, _rx2) = mgr.register_client("t1", "sub-1");

        assert_eq!(mgr.client_count("t1", "sub-1"), 2);

        mgr.remove_client("t1", "sub-1", &id1);
        assert_eq!(mgr.client_count("t1", "sub-1"), 1);
    }

    #[test]
    fn test_remove_all_clients() {
        let mgr = WebSocketManager::new();
        let (_id1, _rx1) = mgr.register_client("t1", "sub-1");
        let (_id2, _rx2) = mgr.register_client("t1", "sub-1");

        mgr.remove_all_clients("t1", "sub-1");
        assert_eq!(mgr.client_count("t1", "sub-1"), 0);
    }

    #[tokio::test]
    async fn test_remove_all_closes_receivers() {
        let mgr = WebSocketManager::new();
        let (_id1, mut rx1) = mgr.register_client("t1", "sub-1");

        mgr.remove_all_clients("t1", "sub-1");

        // Receiver should return None (channel closed).
        assert!(rx1.recv().await.is_none());
    }

    /// A client that stopped reading keeps its queued notifications until the
    /// queue is full, then is dropped: nothing more is retained for it, and
    /// the handler sees the channel end after the queued ones.
    #[tokio::test]
    async fn test_a_client_that_stops_draining_is_dropped_once_its_queue_is_full() {
        let mgr = WebSocketManager::new();
        let (_stalled, mut stalled_rx) = mgr.register_client("t1", "sub-1");
        let (_draining, mut draining_rx) = mgr.register_client("t1", "sub-1");

        for i in 0..CLIENT_QUEUE_CAPACITY {
            let n = json!({"type": "event-notification", "n": i});
            assert_eq!(mgr.send_to_subscription("t1", "sub-1", &n), 2, "{i}");
            assert_eq!(draining_rx.recv().await.unwrap(), n);
        }
        assert_eq!(mgr.client_count("t1", "sub-1"), 2);

        let overflow = json!({"type": "event-notification", "n": "overflow"});
        assert_eq!(
            mgr.send_to_subscription("t1", "sub-1", &overflow),
            1,
            "only the draining client receives the one past the capacity"
        );
        assert_eq!(mgr.client_count("t1", "sub-1"), 1);
        assert_eq!(draining_rx.recv().await.unwrap(), overflow);
        // The dropped client's receiver reports closed while its queue is
        // still full: the signal the connection handler stops on, rather
        // than delivering the backlog (#2720).
        assert!(stalled_rx.is_closed());
        assert!(!draining_rx.is_closed());

        let mut queued = 0;
        while let Some(n) = stalled_rx.recv().await {
            assert_ne!(n, overflow, "nothing was queued past the capacity");
            queued += 1;
        }
        assert_eq!(queued, CLIENT_QUEUE_CAPACITY);
    }

    #[test]
    fn test_send_to_nonexistent_subscription() {
        let mgr = WebSocketManager::new();
        let notification = json!({"type": "event-notification"});
        let count = mgr.send_to_subscription("t1", "sub-1", &notification);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_client_count_empty() {
        let mgr = WebSocketManager::new();
        assert_eq!(mgr.client_count("t1", "sub-1"), 0);
    }
}
