//! WebSocket client connection manager.
//!
//! Tracks connected WebSocket clients per subscription. The subscriptions
//! crate uses this to broadcast notifications, while the REST crate uses
//! it to register new connections from the WebSocket upgrade handler.

use dashmap::DashMap;
use tokio::sync::mpsc;
use tracing::{debug, warn};
use uuid::Uuid;

/// Sender half given to a connected WebSocket client.
pub type WsClientSender = mpsc::UnboundedSender<serde_json::Value>;

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
    pub fn register_client(
        &self,
        tenant_id: &str,
        subscription_id: &str,
    ) -> (String, mpsc::UnboundedReceiver<serde_json::Value>) {
        let (tx, rx) = mpsc::unbounded_channel();
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
    /// Automatically prunes disconnected clients (closed channels).
    /// Returns the number of clients that received the message.
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

        // Send to all clients, removing those with closed channels.
        entry.retain(|(client_id, sender)| {
            if sender.send(notification.clone()).is_ok() {
                delivered += 1;
                true
            } else {
                debug!(client_id = %client_id, "Pruning disconnected WebSocket client");
                false
            }
        });

        let pruned = initial_count - entry.len();
        if pruned > 0 {
            warn!(
                tenant_id,
                subscription_id, pruned, "Pruned disconnected WebSocket clients"
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
