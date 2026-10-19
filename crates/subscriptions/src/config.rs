//! Subscription engine configuration.

use std::time::Duration;

/// Configuration for the subscription engine.
#[derive(Debug, Clone)]
pub struct SubscriptionConfig {
    /// Maximum resource events retained by the nonblocking write ingress.
    /// New data events are shed when this queue is full.
    pub event_queue_capacity: usize,

    /// Global budget for queued and active event notifications, including
    /// notifications sleeping between retry attempts.
    pub max_pending_notifications: usize,

    /// Maximum pending notifications for one tenant/subscription, including
    /// its active notification. This is capped at the global budget.
    pub per_subscription_queue_capacity: usize,

    /// Maximum channel attempts running simultaneously. Retry sleeps do not
    /// hold an attempt slot. A value of one serializes channel attempts.
    pub dispatch_concurrency: usize,

    /// Maximum activation flows running simultaneously, shared by live writes
    /// and startup rehydration.
    pub handshake_concurrency: usize,

    /// Maximum number of retry attempts for failed deliveries.
    pub max_retries: u32,

    /// Initial delay before the first retry.
    pub retry_initial_delay: Duration,

    /// Maximum delay between retries.
    pub retry_max_delay: Duration,

    /// Backoff multiplier for exponential backoff.
    pub retry_backoff_factor: f64,

    /// Delay before the first activation handshake attempt.
    pub handshake_initial_delay: Duration,

    /// Maximum number of activation handshake attempts.
    pub handshake_max_attempts: u32,

    /// Initial delay before retrying a failed activation handshake.
    pub handshake_retry_initial_delay: Duration,

    /// Maximum delay between activation handshake retries.
    pub handshake_retry_max_delay: Duration,

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

    /// Whether server-driven status transitions are written back into the
    /// stored `Subscription` resource (issue #357).
    ///
    /// `true` (the default) makes the engine's own decisions durable and
    /// visible: a subscription it activated reads back `active`, and one its
    /// delivery circuit breaker turned `off` stays `off` across a restart
    /// instead of resuming delivery to a dead endpoint.
    ///
    /// Set `HFS_SUBSCRIPTION_PERSIST_STATUS=false` to restore the previous
    /// in-memory-only behaviour. That is the kill switch for the case where
    /// this write-back is itself the incident: it costs at most three writes
    /// per subscription lifetime, but it does write to the primary backend from
    /// the delivery path.
    pub persist_status: bool,

    /// How long a single status write-back may take before it is abandoned.
    ///
    /// A hung backend must not pin the spawned dispatch task that is waiting on
    /// it. On timeout the transition stays in memory and is logged at `error`.
    pub status_write_timeout: Duration,

    /// Maximum status write-backs in flight at once.
    ///
    /// A subscriber outage can trip every subscription in a tenant at nearly the
    /// same instant; without a bound those transitions would each check out a
    /// backend connection and could starve ordinary reads.
    pub status_write_concurrency: usize,
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
            event_queue_capacity: 1024,
            max_pending_notifications: 1024,
            per_subscription_queue_capacity: 32,
            dispatch_concurrency: 64,
            handshake_concurrency: 8,
            max_retries: 10,
            retry_initial_delay: Duration::from_secs(1),
            retry_max_delay: Duration::from_secs(60),
            retry_backoff_factor: 2.0,
            handshake_initial_delay: Duration::ZERO,
            handshake_max_attempts: 1,
            handshake_retry_initial_delay: Duration::from_secs(1),
            handshake_retry_max_delay: Duration::from_secs(60),
            heartbeat_check_interval: Duration::from_secs(30),
            error_threshold: 3,
            off_threshold: 10,
            supported_channel_types: vec!["rest-hook".to_string()],
            ws_token_lifetime_secs: 30,
            smtp: None,
            messaging: None,
            persist_status: true,
            status_write_timeout: Duration::from_secs(5),
            status_write_concurrency: 4,
        }
    }
}

impl SubscriptionConfig {
    /// Resolve invalid direct-API capacities before constructing channels or
    /// semaphores. This remains synchronous and needs no Tokio runtime.
    pub(crate) fn normalize_dispatch_limits(mut self) -> Self {
        let defaults = Self::default();
        for (name, capacity, fallback) in [
            (
                "event_queue_capacity",
                &mut self.event_queue_capacity,
                defaults.event_queue_capacity,
            ),
            (
                "max_pending_notifications",
                &mut self.max_pending_notifications,
                defaults.max_pending_notifications,
            ),
            (
                "per_subscription_queue_capacity",
                &mut self.per_subscription_queue_capacity,
                defaults.per_subscription_queue_capacity,
            ),
            (
                "dispatch_concurrency",
                &mut self.dispatch_concurrency,
                defaults.dispatch_concurrency,
            ),
            (
                "handshake_concurrency",
                &mut self.handshake_concurrency,
                defaults.handshake_concurrency,
            ),
        ] {
            if *capacity == 0 || *capacity > tokio::sync::Semaphore::MAX_PERMITS {
                tracing::warn!(
                    setting = name,
                    configured = *capacity,
                    effective = fallback,
                    "Invalid subscription dispatch limit; using default"
                );
                *capacity = fallback;
            }
        }
        if self.per_subscription_queue_capacity > self.max_pending_notifications {
            tracing::warn!(
                configured = self.per_subscription_queue_capacity,
                effective = self.max_pending_notifications,
                "Subscription queue capacity exceeds global notification budget; clamping"
            );
            self.per_subscription_queue_capacity = self.max_pending_notifications;
        }
        self
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
///
/// `Debug` redacts the password.
#[derive(Clone)]
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

impl std::fmt::Debug for SmtpSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            host,
            port,
            username,
            password,
            encryption,
            from_address,
            default_subject,
            timeout_secs,
        } = self;
        f.debug_struct("SmtpSettings")
            .field("host", host)
            .field("port", port)
            .field("username", username)
            .field("password", &password.as_ref().map(|_| "[redacted]"))
            .field("encryption", encryption)
            .field("from_address", from_address)
            .field("default_subject", default_subject)
            .field("timeout_secs", timeout_secs)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smtp_settings_debug_redacts_password() {
        let settings = SmtpSettings {
            host: "smtp.example.com".into(),
            port: 587,
            username: Some("mailer".into()),
            password: Some("SMTPSECRET".into()),
            encryption: SmtpEncryption::StartTls,
            from_address: "hfs@example.com".into(),
            default_subject: None,
            timeout_secs: 30,
        };
        let debug = format!("{settings:?}");
        assert!(!debug.contains("SMTPSECRET"), "{debug}");
        assert!(debug.contains("smtp.example.com"), "{debug}");
        assert!(debug.contains("[redacted]"), "{debug}");
    }

    #[test]
    fn invalid_dispatch_limits_use_defaults_without_a_runtime() {
        let default = SubscriptionConfig::default();
        for invalid in [0, tokio::sync::Semaphore::MAX_PERMITS + 1, usize::MAX] {
            let config = SubscriptionConfig {
                event_queue_capacity: invalid,
                max_pending_notifications: invalid,
                per_subscription_queue_capacity: invalid,
                dispatch_concurrency: invalid,
                handshake_concurrency: invalid,
                ..SubscriptionConfig::default()
            }
            .normalize_dispatch_limits();
            assert_eq!(config.event_queue_capacity, default.event_queue_capacity);
            assert_eq!(
                config.max_pending_notifications,
                default.max_pending_notifications
            );
            assert_eq!(
                config.per_subscription_queue_capacity,
                default.per_subscription_queue_capacity
            );
            assert_eq!(config.dispatch_concurrency, default.dispatch_concurrency);
            assert_eq!(config.handshake_concurrency, default.handshake_concurrency);
        }
    }

    #[test]
    fn subscriber_capacity_is_clamped_to_the_global_budget() {
        let config = SubscriptionConfig {
            max_pending_notifications: 3,
            per_subscription_queue_capacity: 9,
            event_queue_capacity: 2,
            dispatch_concurrency: 1,
            handshake_concurrency: 1,
            ..SubscriptionConfig::default()
        }
        .normalize_dispatch_limits();
        assert_eq!(config.per_subscription_queue_capacity, 3);
        assert_eq!(config.max_pending_notifications, 3);
        assert_eq!(config.event_queue_capacity, 2);
        assert_eq!(config.dispatch_concurrency, 1);
        assert_eq!(config.handshake_concurrency, 1);
    }
}
