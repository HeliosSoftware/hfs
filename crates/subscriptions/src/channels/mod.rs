//! Channel dispatchers.
//!
//! Delivers notification bundles to subscriber endpoints via the configured
//! channel type (rest-hook, WebSocket, email, FHIR messaging).

pub mod email;
pub mod messaging;
pub mod rest_hook;
pub mod websocket;
pub mod ws_manager;
pub mod ws_token;

use async_trait::async_trait;

use crate::error::SubscriptionError;
use crate::manager::ActiveSubscription;

/// The result of attempting to deliver a notification.
#[derive(Debug)]
pub enum DispatchResult {
    /// The endpoint accepted the notification.
    Success,
    /// The delivery failed with a retryable error (e.g., 5xx, timeout).
    RetryableError(String),
    /// The delivery failed with a permanent error (e.g., 4xx).
    PermanentError(String),
}

/// Trait for channel-specific notification delivery.
#[async_trait]
pub trait ChannelDispatcher: Send + Sync {
    /// Deliver a notification bundle to the subscriber's endpoint.
    async fn dispatch(
        &self,
        subscription: &ActiveSubscription,
        notification_bundle: &serde_json::Value,
    ) -> Result<DispatchResult, SubscriptionError>;

    /// Perform the handshake sequence for a newly activated subscription.
    async fn handshake(
        &self,
        subscription: &ActiveSubscription,
        handshake_bundle: &serde_json::Value,
    ) -> Result<DispatchResult, SubscriptionError>;
}
