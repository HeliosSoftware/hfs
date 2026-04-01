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

    #[test]
    fn test_default_config() {
        let config = AuditConfig::default();
        assert_eq!(config.backend, AuditBackend::None);
        assert!(config.file_path.is_none());
        assert!(config.database_url.is_none());
        assert!(config.exclusions.is_empty());
        assert_eq!(config.source_observer, "Device/hfs");
    }

    #[test]
    fn test_backend_display() {
        assert_eq!(AuditBackend::None.to_string(), "none");
        assert_eq!(AuditBackend::File.to_string(), "file");
        assert_eq!(AuditBackend::Database.to_string(), "database");
    }

    #[test]
    fn test_from_env_file_backend() {
        unsafe {
            env::set_var("HFS_AUDIT_BACKEND", "file");
            env::set_var("HFS_AUDIT_FILE_PATH", "/tmp/audit.log");
        }
        let config = AuditConfig::from_env();
        assert_eq!(config.backend, AuditBackend::File);
        assert_eq!(config.file_path.as_deref(), Some("/tmp/audit.log"));
        unsafe {
            env::remove_var("HFS_AUDIT_BACKEND");
            env::remove_var("HFS_AUDIT_FILE_PATH");
        }
    }

    #[test]
    fn test_from_env_database_backend() {
        unsafe {
            env::set_var("HFS_AUDIT_BACKEND", "database");
        }
        let config = AuditConfig::from_env();
        assert_eq!(config.backend, AuditBackend::Database);
        unsafe {
            env::remove_var("HFS_AUDIT_BACKEND");
        }
    }

    #[test]
    fn test_from_env_db_alias() {
        unsafe {
            env::set_var("HFS_AUDIT_BACKEND", "db");
        }
        let config = AuditConfig::from_env();
        assert_eq!(config.backend, AuditBackend::Database);
        unsafe {
            env::remove_var("HFS_AUDIT_BACKEND");
        }
    }

    #[test]
    fn test_from_env_unset_defaults_to_none() {
        unsafe {
            env::remove_var("HFS_AUDIT_BACKEND");
        }
        let config = AuditConfig::from_env();
        assert_eq!(config.backend, AuditBackend::None);
    }

    #[test]
    fn test_from_env_custom_source_observer() {
        unsafe {
            env::set_var("HFS_AUDIT_SOURCE_OBSERVER", "Device/my-server");
        }
        let config = AuditConfig::from_env();
        assert_eq!(config.source_observer, "Device/my-server");
        unsafe {
            env::remove_var("HFS_AUDIT_SOURCE_OBSERVER");
        }
    }
}
