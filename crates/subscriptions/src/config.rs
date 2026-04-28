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

    /// Lifetime of WebSocket binding tokens in seconds.
    pub ws_token_lifetime_secs: i64,

    /// SMTP settings for the email channel. `None` disables the email dispatcher.
    pub smtp: Option<SmtpSettings>,

    /// FHIR Messaging channel settings. `None` disables the messaging dispatcher.
    pub messaging: Option<MessagingSettings>,
}

/// FHIR Messaging channel configuration.
#[derive(Debug, Clone)]
pub struct MessagingSettings {
    /// Endpoint URL placed in `MessageHeader.source.endpoint`. Typically the
    /// HFS base URL (`HFS_BASE_URL`).
    pub source_endpoint: String,

    /// When `true`, dispatch to private/loopback/link-local IPs is permitted.
    /// Default `false` (SSRF guard on). Set
    /// `HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS=true` to opt in.
    pub allow_private_endpoints: bool,
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
            ws_token_lifetime_secs: 30,
            smtp: None,
            messaging: None,
        }
    }
}

/// SMTP transport encryption mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpEncryption {
    /// Plain SMTP with no transport encryption.
    None,
    /// STARTTLS — begin in plaintext and upgrade before AUTH.
    StartTls,
    /// Implicit TLS from connect (SMTPS, typically port 465).
    Tls,
}

/// SMTP client configuration for the email channel.
#[derive(Debug, Clone)]
pub struct SmtpSettings {
    /// SMTP relay host.
    pub host: String,
    /// SMTP relay port.
    pub port: u16,
    /// Optional SMTP auth username.
    pub username: Option<String>,
    /// Optional SMTP auth password.
    pub password: Option<String>,
    /// Transport encryption mode.
    pub encryption: SmtpEncryption,
    /// Default `From:` mailbox (RFC 5322). Required when email is enabled;
    /// subscription headers may override on a per-message basis.
    pub from_address: String,
    /// Optional default subject template. If absent, a built-in template is used.
    pub default_subject: Option<String>,
    /// Per-send timeout in seconds.
    pub timeout_secs: u64,
}
