//! Search-specific error types.
//!
//! `LoaderError` and `RegistryError` moved to
//! [`helios_fhir::search::errors`] alongside the registry. `ExtractionError`
//! and `ReindexError` stay here — they're index-feed concerns.

use std::fmt;

use serde::{Deserialize, Serialize};

pub use helios_fhir::search::errors::{LoaderError, RegistryError};

/// Error during value extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExtractionError {
    /// FHIRPath evaluation failed.
    EvaluationFailed {
        /// The parameter name.
        param_name: String,
        /// The FHIRPath expression.
        expression: String,
        /// Error message.
        error: String,
    },

    /// Value conversion failed.
    ConversionFailed {
        /// The parameter name.
        param_name: String,
        /// The expected type.
        expected_type: String,
        /// What was actually found.
        actual_value: String,
    },

    /// Unsupported value type for indexing.
    UnsupportedType {
        /// The parameter name.
        param_name: String,
        /// The unsupported type.
        value_type: String,
    },

    /// Resource is not a valid JSON object.
    InvalidResource {
        /// Description of the problem.
        message: String,
    },

    /// FHIRPath expression evaluation error.
    FhirPathError {
        /// The FHIRPath expression that failed.
        expression: String,
        /// The error message from the evaluator.
        message: String,
    },

    /// Generic value conversion error.
    ConversionError {
        /// Description of the conversion error.
        message: String,
    },
}

impl fmt::Display for ExtractionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtractionError::EvaluationFailed {
                param_name,
                expression,
                error,
            } => {
                write!(
                    f,
                    "Failed to evaluate '{}' for parameter '{}': {}",
                    expression, param_name, error
                )
            }
            ExtractionError::ConversionFailed {
                param_name,
                expected_type,
                actual_value,
            } => {
                write!(
                    f,
                    "Cannot convert '{}' to {} for parameter '{}'",
                    actual_value, expected_type, param_name
                )
            }
            ExtractionError::UnsupportedType {
                param_name,
                value_type,
            } => {
                write!(
                    f,
                    "Unsupported value type '{}' for parameter '{}'",
                    value_type, param_name
                )
            }
            ExtractionError::InvalidResource { message } => {
                write!(f, "Invalid resource: {}", message)
            }
            ExtractionError::FhirPathError {
                expression,
                message,
            } => {
                write!(f, "FHIRPath error evaluating '{}': {}", expression, message)
            }
            ExtractionError::ConversionError { message } => {
                write!(f, "Conversion error: {}", message)
            }
        }
    }
}

impl std::error::Error for ExtractionError {}

/// Error during reindex operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReindexError {
    /// Reindex job not found.
    JobNotFound {
        /// The job ID.
        job_id: String,
    },

    /// Reindex job already running.
    AlreadyRunning {
        /// The existing job ID.
        existing_job_id: String,
    },

    /// Failed to process resource during reindex.
    ProcessingFailed {
        /// Resource type.
        resource_type: String,
        /// Resource ID.
        resource_id: String,
        /// Error message.
        error: String,
    },

    /// Storage error during reindex.
    StorageError {
        /// Error message.
        message: String,
    },

    /// Reindex was cancelled.
    Cancelled {
        /// Job ID that was cancelled.
        job_id: String,
    },
}

impl fmt::Display for ReindexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReindexError::JobNotFound { job_id } => {
                write!(f, "Reindex job '{}' not found", job_id)
            }
            ReindexError::AlreadyRunning { existing_job_id } => {
                write!(
                    f,
                    "Reindex already running with job ID '{}'",
                    existing_job_id
                )
            }
            ReindexError::ProcessingFailed {
                resource_type,
                resource_id,
                error,
            } => {
                write!(
                    f,
                    "Failed to reindex {}/{}: {}",
                    resource_type, resource_id, error
                )
            }
            ReindexError::StorageError { message } => {
                write!(f, "Storage error during reindex: {}", message)
            }
            ReindexError::Cancelled { job_id } => {
                write!(f, "Reindex job '{}' was cancelled", job_id)
            }
        }
    }
}

impl std::error::Error for ReindexError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extraction_error_display() {
        let err = ExtractionError::EvaluationFailed {
            param_name: "name".to_string(),
            expression: "Patient.name".to_string(),
            error: "syntax error".to_string(),
        };
        assert!(err.to_string().contains("name"));
        assert!(err.to_string().contains("Patient.name"));
    }

    #[test]
    fn test_reindex_error_display() {
        let err = ReindexError::ProcessingFailed {
            resource_type: "Patient".to_string(),
            resource_id: "123".to_string(),
            error: "database error".to_string(),
        };
        assert!(err.to_string().contains("Patient/123"));
    }
}
