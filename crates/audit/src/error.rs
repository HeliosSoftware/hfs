//! Error types for the audit subsystem.

/// Errors that can occur during audit operations.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    /// Configuration is invalid or incomplete.
    #[error("Audit configuration error: {0}")]
    Config(String),

    /// File I/O failure in the file sink.
    #[error("File sink error: {0}")]
    FileIo(#[from] std::io::Error),

    /// JSON serialization failure.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Database persistence failure.
    #[error("Database error: {0}")]
    Database(String),
}

/// Convenience alias for audit results.
pub type AuditResult<T> = Result<T, AuditError>;
