//! Subscription engine configuration.

use std::time::Duration;

/// Configuration for the subscription engine.
#[derive(Debug, Clone)]
pub struct SubscriptionConfig {
    /// Maximum number of retry attempts for failed deliveries.
    pub max_retries: u32,

    /// Initial delay before the first retry.
    pub retry_initial_delay: Duration,

    /// Maximum delay between retries.
    pub retry_max_delay: Duration,

    /// Backoff multiplier for exponential backoff.
    pub retry_backoff_factor: f64,

    /// How often to check for heartbeats that are due.
    pub heartbeat_check_interval: Duration,

    /// Number of consecutive failures before transitioning to `error` status.
    pub error_threshold: u32,

    /// Number of consecutive failures before transitioning to `off` status.
    pub off_threshold: u32,

    /// Supported channel types.
    pub supported_channel_types: Vec<String>,
}

impl Default for SubscriptionConfig {
    fn default() -> Self {
        Self {
            max_retries: 10,
            retry_initial_delay: Duration::from_secs(1),
            retry_max_delay: Duration::from_secs(60),
            retry_backoff_factor: 2.0,
            heartbeat_check_interval: Duration::from_secs(30),
            error_threshold: 3,
            off_threshold: 10,
            supported_channel_types: vec!["rest-hook".to_string()],
        }
    }
}
