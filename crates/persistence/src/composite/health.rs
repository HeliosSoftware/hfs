//! Health monitoring for composite storage backends.
//!
//! This module provides health checking and monitoring for all backends
//! in a composite storage configuration.
//!
//! # Features
//!
//! - Periodic health checks for all backends
//! - Failure detection with configurable thresholds
//! - Automatic failover tracking
//! - Health status reporting
//!
//! # Example
//!
//! ```ignore
//! use helios_persistence::composite::health::{HealthMonitor, HealthStatus};
//!
//! let monitor = HealthMonitor::new(config.health_config.clone());
//!
//! // Start background health checks
//! monitor.start(backends.clone());
//!
//! // Check current status
//! let status = monitor.status("primary");
//! if status.map(|s| s.is_healthy()).unwrap_or(false) {
//!     // Backend is healthy
//! }
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::core::ResourceStorage;
use crate::error::{BackendError, StorageError};

use super::config::HealthConfig;

/// Health status for a single backend.
#[derive(Debug, Clone)]
pub struct BackendHealthStatus {
    /// Backend identifier.
    pub backend_id: String,

    /// Whether the backend is currently healthy.
    pub is_healthy: bool,

    /// Last successful health check.
    pub last_success: Option<Instant>,

    /// Last failed health check.
    pub last_failure: Option<Instant>,

    /// Consecutive failure count.
    pub consecutive_failures: u32,

    /// Consecutive success count.
    pub consecutive_successes: u32,

    /// Last error message (if any).
    pub last_error: Option<String>,

    /// Average response time in milliseconds.
    pub avg_response_time_ms: f64,

    /// Response time samples for averaging.
    response_times: Vec<u64>,
}

impl BackendHealthStatus {
    /// Creates a new health status for a backend.
    pub fn new(backend_id: impl Into<String>) -> Self {
        Self {
            backend_id: backend_id.into(),
            is_healthy: true, // Assume healthy until proven otherwise
            last_success: None,
            last_failure: None,
            consecutive_failures: 0,
            consecutive_successes: 0,
            last_error: None,
            avg_response_time_ms: 0.0,
            response_times: Vec::with_capacity(10),
        }
    }

    /// Records a successful health check.
    pub fn record_success(&mut self, response_time_ms: u64) {
        self.last_success = Some(Instant::now());
        self.consecutive_successes += 1;
        self.consecutive_failures = 0;
        self.last_error = None;

        // Update rolling average
        self.response_times.push(response_time_ms);
        if self.response_times.len() > 10 {
            self.response_times.remove(0);
        }
        self.avg_response_time_ms =
            self.response_times.iter().sum::<u64>() as f64 / self.response_times.len() as f64;
    }

    /// Records a failed health check.
    pub fn record_failure(&mut self, error: String) {
        self.last_failure = Some(Instant::now());
        self.consecutive_failures += 1;
        self.consecutive_successes = 0;
        self.last_error = Some(error);
    }

    /// Updates healthy status based on thresholds.
    pub fn update_health(&mut self, failure_threshold: u32, success_threshold: u32) {
        if self.consecutive_failures >= failure_threshold {
            if self.is_healthy {
                warn!(
                    backend = %self.backend_id,
                    failures = self.consecutive_failures,
                    "Backend marked unhealthy"
                );
            }
            self.is_healthy = false;
        } else if self.consecutive_successes >= success_threshold {
            if !self.is_healthy {
                info!(
                    backend = %self.backend_id,
                    successes = self.consecutive_successes,
                    "Backend recovered"
                );
            }
            self.is_healthy = true;
        }
    }

    /// Returns how long since last successful check.
    pub fn time_since_success(&self) -> Option<Duration> {
        self.last_success.map(|t| t.elapsed())
    }

    /// Returns how long since last failed check.
    pub fn time_since_failure(&self) -> Option<Duration> {
        self.last_failure.map(|t| t.elapsed())
    }
}

/// Aggregate health status for all backends.
#[derive(Debug, Clone)]
pub struct CompositeHealthStatus {
    /// Status of each backend.
    pub backends: HashMap<String, BackendHealthStatus>,

    /// Whether the system as a whole is healthy.
    pub is_healthy: bool,

    /// Degraded backends (unhealthy but with failover available).
    pub degraded_backends: Vec<String>,

    /// Failed backends (unhealthy without failover).
    pub failed_backends: Vec<String>,

    /// Time of this status snapshot.
    pub timestamp: Instant,
}

impl CompositeHealthStatus {
    /// Creates a new composite health status.
    pub fn new(backends: HashMap<String, BackendHealthStatus>) -> Self {
        let degraded_backends: Vec<_> = backends
            .iter()
            .filter(|(_, status)| !status.is_healthy)
            .map(|(id, _)| id.clone())
            .collect();

        // For now, all unhealthy backends are considered degraded
        // A more sophisticated implementation would check failover status
        let failed_backends = Vec::new();

        let is_healthy = degraded_backends.is_empty() && failed_backends.is_empty();

        Self {
            backends,
            is_healthy,
            degraded_backends,
            failed_backends,
            timestamp: Instant::now(),
        }
    }

    /// Returns true if the primary backend is healthy.
    pub fn primary_healthy(&self, primary_id: &str) -> bool {
        self.backends
            .get(primary_id)
            .map(|s| s.is_healthy)
            .unwrap_or(false)
    }

    /// Returns the number of healthy backends.
    pub fn healthy_count(&self) -> usize {
        self.backends.values().filter(|s| s.is_healthy).count()
    }

    /// Returns the number of unhealthy backends.
    pub fn unhealthy_count(&self) -> usize {
        self.backends.values().filter(|s| !s.is_healthy).count()
    }
}

/// Health check result.
#[derive(Debug)]
pub enum HealthCheckResult {
    /// Health check passed.
    Healthy {
        /// Response time in milliseconds.
        response_time_ms: u64,
    },
    /// Health check failed.
    Unhealthy {
        /// Error message.
        error: String,
    },
    /// Health check timed out.
    Timeout,
}

/// Health monitor for composite storage.
pub struct HealthMonitor {
    /// Configuration.
    config: HealthConfig,

    /// Health status for each backend.
    status: Arc<RwLock<HashMap<String, BackendHealthStatus>>>,

    /// Shutdown channel.
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl HealthMonitor {
    /// Creates a new health monitor.
    pub fn new(config: HealthConfig) -> Self {
        Self {
            config,
            status: Arc::new(RwLock::new(HashMap::new())),
            shutdown_tx: None,
        }
    }

    /// Starts background health checking.
    pub fn start(
        &mut self,
        backends: HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
    ) -> tokio::task::JoinHandle<()> {
        let (tx, rx) = mpsc::channel(1);
        self.shutdown_tx = Some(tx);

        let config = self.config.clone();
        let status = self.status.clone();

        // Initialize status for all backends
        {
            let mut status_map = status.write();
            for id in backends.keys() {
                status_map.insert(id.clone(), BackendHealthStatus::new(id));
            }
        }

        tokio::spawn(async move {
            Self::health_check_loop(rx, backends, config, status).await;
        })
    }

    /// Stops the health monitor.
    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
    }

    /// Gets the health status for a specific backend.
    pub fn backend_status(&self, backend_id: &str) -> Option<BackendHealthStatus> {
        self.status.read().get(backend_id).cloned()
    }

    /// Gets the health status for all backends.
    pub fn all_status(&self) -> CompositeHealthStatus {
        CompositeHealthStatus::new(self.status.read().clone())
    }

    /// Returns true if a specific backend is healthy.
    pub fn is_healthy(&self, backend_id: &str) -> bool {
        self.status
            .read()
            .get(backend_id)
            .map(|s| s.is_healthy)
            .unwrap_or(false)
    }

    /// Returns true if all backends are healthy.
    pub fn all_healthy(&self) -> bool {
        self.status.read().values().all(|s| s.is_healthy)
    }

    /// Performs a single health check for a backend.
    pub async fn check_backend(
        backend: &dyn ResourceStorage,
        timeout: Duration,
    ) -> HealthCheckResult {
        let start = Instant::now();

        // Use a simple existence check as health probe
        // A real implementation might use a dedicated health endpoint
        let check = async {
            backend
                .count(
                    &crate::tenant::TenantContext::system(),
                    Some("__health_check__"),
                )
                .await
        };

        match tokio::time::timeout(timeout, check).await {
            Ok(Ok(_)) => HealthCheckResult::Healthy {
                response_time_ms: start.elapsed().as_millis() as u64,
            },
            // The backend could not be reached or could not answer. Every
            // error used to read as healthy here, on the theory that an error
            // is still a response — but an unreachable or failing backend
            // answers the probe with exactly this (#1382).
            Ok(Err(e)) if Self::probe_error_is_outage(&e) => HealthCheckResult::Unhealthy {
                error: match &e {
                    // `Unavailable` does not display its detail.
                    StorageError::Backend(BackendError::Unavailable { message, .. }) => {
                        format!("{e}: {message}")
                    }
                    _ => e.to_string(),
                },
            },
            // Anything else is the backend (or its configuration) declining
            // this particular question, which takes a working backend: no
            // backend counts a type that does not exist as an error, but one
            // may have no place for the system tenant, or no `count` at all.
            Ok(Err(_)) => HealthCheckResult::Healthy {
                response_time_ms: start.elapsed().as_millis() as u64,
            },
            Err(_) => HealthCheckResult::Timeout,
        }
    }

    /// Whether a failed probe says the backend is down, rather than that it
    /// would not answer this question.
    ///
    /// Backends retry transient failures themselves before reporting one —
    /// Elasticsearch for up to ~300 ms of back-off over three attempts — which
    /// stays well inside the default 5 s probe timeout, and a backend is only
    /// marked unhealthy after `failure_threshold` consecutive failed probes, so
    /// a slow but working backend is not taken for a failed one.
    fn probe_error_is_outage(error: &StorageError) -> bool {
        match error {
            // A statement-level deadline or a missing capability is an answer
            // from a backend that is up.
            StorageError::Backend(
                BackendError::Timeout { .. } | BackendError::UnsupportedCapability { .. },
            ) => false,
            StorageError::Backend(_) => true,
            _ => false,
        }
    }

    /// Background health check loop.
    async fn health_check_loop(
        mut shutdown_rx: mpsc::Receiver<()>,
        backends: HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
        config: HealthConfig,
        status: Arc<RwLock<HashMap<String, BackendHealthStatus>>>,
    ) {
        let mut interval = tokio::time::interval(config.check_interval);

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    debug!("Health monitor shutting down");
                    break;
                }
                _ = interval.tick() => {
                    for (id, backend) in &backends {
                        let result = Self::check_backend(backend.as_ref(), config.timeout).await;

                        let mut status_map = status.write();
                        if let Some(backend_status) = status_map.get_mut(id) {
                            match result {
                                HealthCheckResult::Healthy { response_time_ms } => {
                                    backend_status.record_success(response_time_ms);
                                }
                                HealthCheckResult::Unhealthy { error } => {
                                    backend_status.record_failure(error);
                                }
                                HealthCheckResult::Timeout => {
                                    backend_status.record_failure("Health check timed out".to_string());
                                }
                            }

                            backend_status.update_health(
                                config.failure_threshold,
                                config.success_threshold,
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Health check endpoint response (for HTTP APIs).
#[derive(Debug, Clone, serde::Serialize)]
pub struct HealthCheckResponse {
    /// Overall status.
    pub status: String,

    /// Status of individual components.
    pub components: HashMap<String, ComponentHealth>,

    /// Timestamp.
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Health status of a component.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ComponentHealth {
    /// Status: "healthy", "unhealthy", "degraded".
    pub status: String,

    /// Additional details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,

    /// Response time in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_time_ms: Option<u64>,
}

impl From<CompositeHealthStatus> for HealthCheckResponse {
    fn from(status: CompositeHealthStatus) -> Self {
        let overall_status = if status.is_healthy {
            "healthy"
        } else if status.failed_backends.is_empty() {
            "degraded"
        } else {
            "unhealthy"
        };

        let components = status
            .backends
            .into_iter()
            .map(|(id, backend_status)| {
                let component_status = if backend_status.is_healthy {
                    "healthy"
                } else {
                    "unhealthy"
                };

                (
                    id,
                    ComponentHealth {
                        status: component_status.to_string(),
                        details: backend_status.last_error,
                        response_time_ms: if backend_status.avg_response_time_ms > 0.0 {
                            Some(backend_status.avg_response_time_ms as u64)
                        } else {
                            None
                        },
                    },
                )
            })
            .collect();

        Self {
            status: overall_status.to_string(),
            components,
            timestamp: chrono::Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_health_status_new() {
        let status = BackendHealthStatus::new("test-backend");
        assert!(status.is_healthy);
        assert_eq!(status.consecutive_failures, 0);
        assert_eq!(status.consecutive_successes, 0);
    }

    #[test]
    fn test_record_success() {
        let mut status = BackendHealthStatus::new("test");
        status.record_success(100);

        assert!(status.last_success.is_some());
        assert_eq!(status.consecutive_successes, 1);
        assert_eq!(status.consecutive_failures, 0);
        assert!((status.avg_response_time_ms - 100.0).abs() < 0.1);
    }

    #[test]
    fn test_record_failure() {
        let mut status = BackendHealthStatus::new("test");
        status.record_failure("Connection refused".to_string());

        assert!(status.last_failure.is_some());
        assert_eq!(status.consecutive_failures, 1);
        assert_eq!(status.last_error, Some("Connection refused".to_string()));
    }

    #[test]
    fn test_update_health_becomes_unhealthy() {
        let mut status = BackendHealthStatus::new("test");

        // Record 3 failures
        for _ in 0..3 {
            status.record_failure("Error".to_string());
        }

        // With threshold of 3, should become unhealthy
        status.update_health(3, 2);
        assert!(!status.is_healthy);
    }

    #[test]
    fn test_update_health_recovers() {
        let mut status = BackendHealthStatus::new("test");
        status.is_healthy = false;

        // Record 2 successes
        for _ in 0..2 {
            status.record_success(50);
        }

        // With threshold of 2, should recover
        status.update_health(3, 2);
        assert!(status.is_healthy);
    }

    #[test]
    fn test_composite_health_status() {
        let mut backends = HashMap::new();

        let mut healthy = BackendHealthStatus::new("healthy");
        healthy.is_healthy = true;

        let mut unhealthy = BackendHealthStatus::new("unhealthy");
        unhealthy.is_healthy = false;

        backends.insert("healthy".to_string(), healthy);
        backends.insert("unhealthy".to_string(), unhealthy);

        let status = CompositeHealthStatus::new(backends);

        assert!(!status.is_healthy);
        assert_eq!(status.healthy_count(), 1);
        assert_eq!(status.unhealthy_count(), 1);
        assert!(status.degraded_backends.contains(&"unhealthy".to_string()));
    }

    /// What a failed probe says about the backend (#1382). The probe itself is
    /// exercised against a stubbed cluster in
    /// `tests/elasticsearch_storage_write_wiremock.rs`.
    #[test]
    fn test_probe_error_is_outage() {
        let backend = |e: BackendError| StorageError::Backend(e);
        let name = || "test".to_string();

        assert!(HealthMonitor::probe_error_is_outage(&backend(
            BackendError::Unavailable {
                backend_name: name(),
                message: "connection refused".to_string(),
            }
        )));
        assert!(HealthMonitor::probe_error_is_outage(&backend(
            BackendError::Internal {
                backend_name: name(),
                message: "Count failed after 3 attempts (status 503)".to_string(),
                source: None,
            }
        )));
        assert!(HealthMonitor::probe_error_is_outage(&backend(
            BackendError::PoolExhausted {
                backend_name: name(),
            }
        )));

        // The backend is up; it declined the question.
        assert!(!HealthMonitor::probe_error_is_outage(&backend(
            BackendError::UnsupportedCapability {
                backend_name: name(),
                capability: "count".to_string(),
            }
        )));
        assert!(!HealthMonitor::probe_error_is_outage(&backend(
            BackendError::Timeout {
                backend_name: name(),
                message: "statement timeout".to_string(),
            }
        )));
        // S3 in bucket-per-tenant mode with no system bucket configured.
        assert!(!HealthMonitor::probe_error_is_outage(
            &StorageError::Tenant(crate::error::TenantError::InvalidTenant {
                tenant_id: crate::tenant::TenantId::system(),
            })
        ));
    }

    #[test]
    fn test_health_check_response_from_status() {
        let mut backends = HashMap::new();
        backends.insert("primary".to_string(), BackendHealthStatus::new("primary"));

        let status = CompositeHealthStatus::new(backends);
        let response: HealthCheckResponse = status.into();

        assert_eq!(response.status, "healthy");
        assert!(response.components.contains_key("primary"));
    }
}
