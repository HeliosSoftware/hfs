//! Audit subsystem configuration.
//!
//! Loaded from environment variables prefixed with `HFS_AUDIT_`.

use std::env;
use std::fmt;

use crate::exclusion::ExclusionRule;
use crate::sinks::QueueConfig;

/// Which audit backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditBackend {
    /// Audit logging disabled (default).
    None,
    /// Append-only NDJSON file.
    File,
    /// Persist via the FHIR storage backend (SQLite / PostgreSQL / S3).
    Database,
    /// AWS CloudWatch Logs (requires `cloudwatch` feature).
    CloudWatch,
}

impl fmt::Display for AuditBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::File => write!(f, "file"),
            Self::Database => write!(f, "database"),
            Self::CloudWatch => write!(f, "cloudwatch"),
        }
    }
}

/// Top-level audit configuration.
///
/// `Debug` is hand-written (#2149): `database_url` prints only as
/// `[redacted]` when set. This crate has no driver to parse it with, and a
/// connection string can carry a password in several syntaxes.
#[derive(Clone)]
pub struct AuditConfig {
    /// Active backend.
    pub backend: AuditBackend,
    /// File path for the file sink (required when `backend = File`).
    pub file_path: Option<String>,
    /// Optional dedicated database URL for audit events.
    /// When absent the main HFS storage backend is reused.
    pub database_url: Option<String>,
    /// Optional dedicated MongoDB database name for audit events.
    /// Used when the primary backend family is MongoDB.
    pub mongodb_database: Option<String>,
    /// Optional dedicated S3 bucket for audit events.
    /// Used when the primary backend family is S3.
    pub s3_bucket: Option<String>,
    /// Optional dedicated S3 prefix for audit events.
    /// Used when the primary backend family is S3.
    pub s3_prefix: Option<String>,
    /// Optional dedicated S3 region for audit events.
    /// Used when the primary backend family is S3.
    pub s3_region: Option<String>,
    /// Optional dedicated S3 bucket validation toggle for audit events.
    /// Used when the primary backend family is S3.
    pub s3_validate_buckets: Option<bool>,
    /// Path/method pairs that should be excluded from audit logging.
    pub exclusions: Vec<ExclusionRule>,
    /// FHIR Reference used as `AuditEvent.source.observer`
    /// (e.g. `"Device/hfs"`).
    pub source_observer: String,
    /// CloudWatch Logs log group name (required when `backend = CloudWatch`).
    pub cloudwatch_log_group: Option<String>,
    /// CloudWatch Logs log stream name (defaults to `"hfs-audit"`).
    pub cloudwatch_log_stream: Option<String>,
    /// AWS region override for CloudWatch Logs.
    pub cloudwatch_region: Option<String>,
    /// Queue between the request path and the sink (`HFS_AUDIT_QUEUE_CAPACITY`,
    /// `HFS_AUDIT_WRITERS`, `HFS_AUDIT_OVERFLOW`).
    pub queue: QueueConfig,
    /// Retries of a transient database-sink write (`HFS_AUDIT_WRITE_RETRIES`).
    pub write_retries: u32,
}

/// Parse `key` with `FromStr`, falling back to `default` (with a warning)
/// when it is set but invalid.
fn env_parse<T>(key: &str, default: T) -> T
where
    T: std::str::FromStr + fmt::Display,
    T::Err: fmt::Display,
{
    match env::var(key) {
        Ok(raw) => raw.trim().parse().unwrap_or_else(|e| {
            tracing::warn!(%key, value = %raw, error = %e, %default, "Invalid audit setting; using the default");
            default
        }),
        Err(_) => default,
    }
}

impl AuditConfig {
    /// Load configuration from `HFS_AUDIT_*` environment variables.
    pub fn from_env() -> Self {
        let backend = match env::var("HFS_AUDIT_BACKEND")
            .unwrap_or_else(|_| "none".to_string())
            .to_lowercase()
            .as_str()
        {
            "file" => AuditBackend::File,
            "database" | "db" => AuditBackend::Database,
            "cloudwatch" | "cloudwatch-logs" | "cwl" => AuditBackend::CloudWatch,
            _ => AuditBackend::None,
        };

        let exclusions = env::var("HFS_AUDIT_EXCLUDE_PATHS")
            .ok()
            .map(|paths| {
                paths
                    .split(',')
                    .map(|p| ExclusionRule {
                        path: p.trim().to_string(),
                        method: None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let defaults = Self::default();
        Self {
            backend,
            file_path: env::var("HFS_AUDIT_FILE_PATH").ok(),
            database_url: env::var("HFS_AUDIT_DATABASE_URL").ok(),
            mongodb_database: env::var("HFS_AUDIT_MONGODB_DATABASE").ok(),
            s3_bucket: env::var("HFS_AUDIT_S3_BUCKET").ok(),
            s3_prefix: env::var("HFS_AUDIT_S3_PREFIX").ok(),
            s3_region: env::var("HFS_AUDIT_S3_REGION").ok(),
            s3_validate_buckets: env::var("HFS_AUDIT_S3_VALIDATE_BUCKETS").ok().map(|v| {
                let normalized = v.trim().to_ascii_lowercase();
                !(normalized == "false" || normalized == "0")
            }),
            exclusions,
            source_observer: env::var("HFS_AUDIT_SOURCE_OBSERVER")
                .unwrap_or_else(|_| "Device/hfs".to_string()),
            cloudwatch_log_group: env::var("HFS_AUDIT_CLOUDWATCH_LOG_GROUP").ok(),
            cloudwatch_log_stream: env::var("HFS_AUDIT_CLOUDWATCH_LOG_STREAM").ok(),
            cloudwatch_region: env::var("HFS_AUDIT_CLOUDWATCH_REGION").ok(),
            queue: QueueConfig {
                capacity: env_parse("HFS_AUDIT_QUEUE_CAPACITY", defaults.queue.capacity),
                writers: env_parse("HFS_AUDIT_WRITERS", defaults.queue.writers),
                overflow: env_parse("HFS_AUDIT_OVERFLOW", defaults.queue.overflow),
                ..defaults.queue
            },
            write_retries: env_parse("HFS_AUDIT_WRITE_RETRIES", defaults.write_retries),
        }
    }
}

impl fmt::Debug for AuditConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Destructured without `..` so a new field fails to compile until it
        // is classified as secret or not.
        let Self {
            backend,
            file_path,
            database_url,
            mongodb_database,
            s3_bucket,
            s3_prefix,
            s3_region,
            s3_validate_buckets,
            exclusions,
            source_observer,
            cloudwatch_log_group,
            cloudwatch_log_stream,
            cloudwatch_region,
            queue,
            write_retries,
        } = self;
        f.debug_struct("AuditConfig")
            .field("backend", backend)
            .field("file_path", file_path)
            .field("database_url", &database_url.as_ref().map(|_| "[redacted]"))
            .field("mongodb_database", mongodb_database)
            .field("s3_bucket", s3_bucket)
            .field("s3_prefix", s3_prefix)
            .field("s3_region", s3_region)
            .field("s3_validate_buckets", s3_validate_buckets)
            .field("exclusions", exclusions)
            .field("source_observer", source_observer)
            .field("cloudwatch_log_group", cloudwatch_log_group)
            .field("cloudwatch_log_stream", cloudwatch_log_stream)
            .field("cloudwatch_region", cloudwatch_region)
            .field("queue", queue)
            .field("write_retries", write_retries)
            .finish()
    }
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            backend: AuditBackend::None,
            file_path: None,
            database_url: None,
            mongodb_database: None,
            s3_bucket: None,
            s3_prefix: None,
            s3_region: None,
            s3_validate_buckets: None,
            exclusions: Vec::new(),
            source_observer: "Device/hfs".to_string(),
            cloudwatch_log_group: None,
            cloudwatch_log_stream: None,
            cloudwatch_region: None,
            queue: QueueConfig::default(),
            write_retries: crate::sinks::database::RetryPolicy::default().max_retries,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sinks::OverflowPolicy;
    use std::sync::{LazyLock, Mutex};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn debug_redacts_database_url() {
        let config = AuditConfig {
            database_url: Some("postgres://audit:audit-s3cret@db.internal/audit".to_string()),
            s3_bucket: Some("audit-bucket".to_string()),
            ..AuditConfig::default()
        };
        let out = format!("{config:?}");
        assert!(!out.contains("audit-s3cret"), "{out}");
        assert!(out.contains("[redacted]"), "{out}");
        assert!(out.contains("audit-bucket"), "{out}");
    }

    const AUDIT_ENV_KEYS: &[&str] = &[
        "HFS_AUDIT_BACKEND",
        "HFS_AUDIT_FILE_PATH",
        "HFS_AUDIT_DATABASE_URL",
        "HFS_AUDIT_MONGODB_DATABASE",
        "HFS_AUDIT_S3_BUCKET",
        "HFS_AUDIT_S3_PREFIX",
        "HFS_AUDIT_S3_REGION",
        "HFS_AUDIT_S3_VALIDATE_BUCKETS",
        "HFS_AUDIT_EXCLUDE_PATHS",
        "HFS_AUDIT_SOURCE_OBSERVER",
        "HFS_AUDIT_CLOUDWATCH_LOG_GROUP",
        "HFS_AUDIT_CLOUDWATCH_LOG_STREAM",
        "HFS_AUDIT_CLOUDWATCH_REGION",
        "HFS_AUDIT_QUEUE_CAPACITY",
        "HFS_AUDIT_WRITERS",
        "HFS_AUDIT_OVERFLOW",
        "HFS_AUDIT_WRITE_RETRIES",
    ];

    fn clear_audit_env() {
        for key in AUDIT_ENV_KEYS {
            unsafe { env::remove_var(key) };
        }
    }

    #[test]
    fn test_default_config() {
        let config = AuditConfig::default();
        assert_eq!(config.backend, AuditBackend::None);
        assert!(config.file_path.is_none());
        assert!(config.database_url.is_none());
        assert!(config.mongodb_database.is_none());
        assert!(config.s3_bucket.is_none());
        assert!(config.s3_prefix.is_none());
        assert!(config.s3_region.is_none());
        assert!(config.s3_validate_buckets.is_none());
        assert!(config.exclusions.is_empty());
        assert_eq!(config.source_observer, "Device/hfs");
    }

    #[test]
    fn test_backend_display() {
        assert_eq!(AuditBackend::None.to_string(), "none");
        assert_eq!(AuditBackend::File.to_string(), "file");
        assert_eq!(AuditBackend::Database.to_string(), "database");
        assert_eq!(AuditBackend::CloudWatch.to_string(), "cloudwatch");
    }

    #[test]
    fn test_from_env_file_backend() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_audit_env();
        unsafe {
            env::set_var("HFS_AUDIT_BACKEND", "file");
            env::set_var("HFS_AUDIT_FILE_PATH", "/tmp/audit.log");
        }
        let config = AuditConfig::from_env();
        assert_eq!(config.backend, AuditBackend::File);
        assert_eq!(config.file_path.as_deref(), Some("/tmp/audit.log"));
        clear_audit_env();
    }

    #[test]
    fn test_from_env_database_backend() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_audit_env();
        unsafe {
            env::set_var("HFS_AUDIT_BACKEND", "database");
        }
        let config = AuditConfig::from_env();
        assert_eq!(config.backend, AuditBackend::Database);
        clear_audit_env();
    }

    #[test]
    fn test_from_env_db_alias() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_audit_env();
        unsafe {
            env::set_var("HFS_AUDIT_BACKEND", "db");
        }
        let config = AuditConfig::from_env();
        assert_eq!(config.backend, AuditBackend::Database);
        clear_audit_env();
    }

    #[test]
    fn test_from_env_cloudwatch_backend_aliases() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_audit_env();
        unsafe {
            env::set_var("HFS_AUDIT_BACKEND", "cloudwatch-logs");
            env::set_var("HFS_AUDIT_CLOUDWATCH_LOG_GROUP", "/hfs/audit");
            env::set_var("HFS_AUDIT_CLOUDWATCH_LOG_STREAM", "ci-stream");
            env::set_var("HFS_AUDIT_CLOUDWATCH_REGION", "us-east-1");
        }

        let config = AuditConfig::from_env();
        assert_eq!(config.backend, AuditBackend::CloudWatch);
        assert_eq!(config.cloudwatch_log_group.as_deref(), Some("/hfs/audit"));
        assert_eq!(config.cloudwatch_log_stream.as_deref(), Some("ci-stream"));
        assert_eq!(config.cloudwatch_region.as_deref(), Some("us-east-1"));

        clear_audit_env();
    }

    #[test]
    fn test_from_env_unset_defaults_to_none() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_audit_env();
        let config = AuditConfig::from_env();
        assert_eq!(config.backend, AuditBackend::None);
    }

    #[test]
    fn test_from_env_queue_settings() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_audit_env();
        let config = AuditConfig::from_env();
        assert_eq!(config.queue, QueueConfig::default());
        assert_eq!(config.queue.overflow, OverflowPolicy::Block);

        unsafe {
            env::set_var("HFS_AUDIT_QUEUE_CAPACITY", "500");
            env::set_var("HFS_AUDIT_WRITERS", "2");
            env::set_var("HFS_AUDIT_OVERFLOW", "drop");
            env::set_var("HFS_AUDIT_WRITE_RETRIES", "0");
        }
        let config = AuditConfig::from_env();
        assert_eq!(config.queue.capacity, 500);
        assert_eq!(config.queue.writers, 2);
        assert_eq!(config.queue.overflow, OverflowPolicy::Drop);
        assert_eq!(config.write_retries, 0);

        // Invalid values fall back to the defaults.
        unsafe {
            env::set_var("HFS_AUDIT_WRITERS", "many");
            env::set_var("HFS_AUDIT_OVERFLOW", "spill");
        }
        let config = AuditConfig::from_env();
        assert_eq!(config.queue.writers, QueueConfig::default().writers);
        assert_eq!(config.queue.overflow, OverflowPolicy::Block);
        clear_audit_env();
    }

    #[test]
    fn test_from_env_custom_source_observer() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_audit_env();
        unsafe {
            env::set_var("HFS_AUDIT_SOURCE_OBSERVER", "Device/my-server");
        }
        let config = AuditConfig::from_env();
        assert_eq!(config.source_observer, "Device/my-server");
        clear_audit_env();
    }

    #[test]
    fn test_from_env_database_overrides() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_audit_env();
        unsafe {
            env::set_var(
                "HFS_AUDIT_DATABASE_URL",
                "postgresql://audit:pass@localhost:5432/audit",
            );
            env::set_var("HFS_AUDIT_MONGODB_DATABASE", "helios_audit");
            env::set_var("HFS_AUDIT_S3_BUCKET", "audit-bucket");
            env::set_var("HFS_AUDIT_S3_PREFIX", "audit/");
            env::set_var("HFS_AUDIT_S3_REGION", "us-east-1");
            env::set_var("HFS_AUDIT_S3_VALIDATE_BUCKETS", "false");
        }

        let config = AuditConfig::from_env();
        assert_eq!(
            config.database_url.as_deref(),
            Some("postgresql://audit:pass@localhost:5432/audit")
        );
        assert_eq!(config.mongodb_database.as_deref(), Some("helios_audit"));
        assert_eq!(config.s3_bucket.as_deref(), Some("audit-bucket"));
        assert_eq!(config.s3_prefix.as_deref(), Some("audit/"));
        assert_eq!(config.s3_region.as_deref(), Some("us-east-1"));
        assert_eq!(config.s3_validate_buckets, Some(false));

        clear_audit_env();
    }
}
