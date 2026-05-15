//! Audit subsystem configuration.
//!
//! Loaded from environment variables prefixed with `HFS_AUDIT_`.

use std::env;
use std::fmt;

use crate::exclusion::ExclusionRule;

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
#[derive(Debug, Clone)]
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
        }
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{LazyLock, Mutex};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

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
