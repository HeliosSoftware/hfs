//! WebSocket channel dispatcher.
//!
//! Delivers notification bundles by broadcasting to all connected WebSocket
//! clients for a subscription. The actual WebSocket connections are managed
//! by the [`WebSocketManager`] and the REST layer's upgrade handler.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::debug;

use super::ws_manager::WebSocketManager;
use super::{ChannelDispatcher, DispatchResult};
use crate::error::SubscriptionError;
use crate::manager::ActiveSubscription;

/// WebSocket channel implementation.
///
/// Unlike rest-hook (which pushes to an external endpoint), this channel
/// broadcasts to clients that have connected to the server's WebSocket
/// endpoint and bound to a subscription via a binding token.
pub struct WebSocketChannel {
    manager: Arc<WebSocketManager>,
}

impl WebSocketChannel {
    /// Creates a new WebSocket channel backed by the given manager.
    pub fn new(manager: Arc<WebSocketManager>) -> Self {
        Self { manager }
    }

    /// Returns a reference to the underlying WebSocket manager.
    pub fn manager(&self) -> &Arc<WebSocketManager> {
        &self.manager
    }
}

#[async_trait]
impl ChannelDispatcher for WebSocketChannel {
    async fn dispatch(
        &self,
        subscription: &ActiveSubscription,
        notification_bundle: &serde_json::Value,
    ) -> Result<DispatchResult, SubscriptionError> {
        let count = self.manager.send_to_subscription(
            &subscription.tenant_id,
            &subscription.id,
            notification_bundle,
        );

        debug!(
            subscription_id = %subscription.id,
            clients = count,
            "WebSocket notification dispatched"
        );

        // WebSocket dispatch is best-effort. Even with zero connected clients
        // we return Success — treating it as an error would incorrectly trigger
        // the error/off status transitions in the engine.
        Ok(DispatchResult::Success)
    }

    async fn handshake(
        &self,
        subscription: &ActiveSubscription,
        _handshake_bundle: &serde_json::Value,
    ) -> Result<DispatchResult, SubscriptionError> {
        // WebSocket handshake is a no-op at subscription activation time.
        // The actual handshake notification is sent when a client connects
        // via the WebSocket upgrade handler and binds with a token.
        debug!(
            subscription_id = %subscription.id,
            "WebSocket handshake (no-op at activation)"
        );
        Ok(DispatchResult::Success)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{ChannelConfig, ChannelType, PayloadContent, SubscriptionStatusCode};
    use helios_fhir::FhirVersion;
    use serde_json::json;

    fn ws_subscription() -> ActiveSubscription {
        ActiveSubscription {
            id: "sub-ws-1".to_string(),
            topic_url: "http://example.org/topic/test".to_string(),
            status: SubscriptionStatusCode::Active,
            channel: ChannelConfig {
                channel_type: ChannelType::Websocket,
                endpoint: None,
                payload_mime_type: Some("application/fhir+json".to_string()),
                payload_content: PayloadContent::FullResource,
                headers: vec![],
                heartbeat_period: None,
                timeout: None,
                max_count: None,
            },
            filters: vec![],
            fhir_version: FhirVersion::default(),
            events_since_start: 0,
            consecutive_failures: 0,
            tenant_id: "test".to_string(),
        }
    }

    fn test_bundle() -> serde_json::Value {
        json!({"resourceType": "Bundle", "type": "history", "entry": []})
    }

    #[tokio::test]
    async fn test_dispatch_no_clients_returns_success() {
        let mgr = Arc::new(WebSocketManager::new());
        let channel = WebSocketChannel::new(mgr);

        let result = channel
            .dispatch(&ws_subscription(), &test_bundle())
            .await
            .unwrap();
        assert!(matches!(result, DispatchResult::Success));
    }

    #[tokio::test]
    async fn test_dispatch_delivers_to_clients() {
        let mgr = Arc::new(WebSocketManager::new());
        let (_client_id, mut rx) = mgr.register_client("test", "sub-ws-1");

        let channel = WebSocketChannel::new(mgr);
        let bundle = test_bundle();

        let result = channel.dispatch(&ws_subscription(), &bundle).await.unwrap();
        assert!(matches!(result, DispatchResult::Success));

        let received = rx.recv().await.unwrap();
        assert_eq!(received, bundle);
    }

    #[tokio::test]
    async fn test_handshake_returns_success() {
        let mgr = Arc::new(WebSocketManager::new());
        let channel = WebSocketChannel::new(mgr);

        let result = channel
            .handshake(&ws_subscription(), &test_bundle())
            .await
            .unwrap();
        assert!(matches!(result, DispatchResult::Success));
    }
}
