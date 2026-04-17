//! Subscription error types.

use thiserror::Error;

/// Errors that can occur during subscription operations.
#[derive(Debug, Error)]
pub enum SubscriptionError {
    /// The referenced SubscriptionTopic was not found.
    #[error("subscription topic not found: {url}")]
    TopicNotFound { url: String },

    /// A filter parameter is invalid or not supported by the topic.
    #[error("invalid filter: {message}")]
    InvalidFilter { message: String },

    /// The requested channel type is not supported.
    #[error("unsupported channel type: {channel_type}")]
    UnsupportedChannel { channel_type: String },

    /// The channel endpoint is invalid or missing.
    #[error("invalid channel endpoint: {message}")]
    InvalidEndpoint { message: String },

    /// Notification delivery failed.
    #[error("delivery failed: {message}")]
    DeliveryFailed { message: String },

    /// An invalid subscription status transition was attempted.
    #[error("invalid status transition from {from} to {to}")]
    InvalidStatusTransition { from: String, to: String },

    /// The subscription resource is malformed.
    #[error("invalid subscription resource: {message}")]
    InvalidSubscription { message: String },

    /// An error occurred in the persistence layer.
    #[error("storage error: {0}")]
    Storage(String),

    /// An internal error occurred.
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<helios_persistence::error::StorageError> for SubscriptionError {
    fn from(e: helios_persistence::error::StorageError) -> Self {
        Self::Storage(e.to_string())
    }
}
