//! Error types for SearchParameter loading and registry operations.
//!
//! Lifted from `helios-persistence` so `helios-sof` can do
//! compartment-aware filtering without a circular dep. Persistence's
//! `ExtractionError` and `ReindexError` stay in `helios-persistence` —
//! they're index-feed concerns, not spec concerns.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Error during SearchParameter loading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoaderError {
    /// Invalid SearchParameter resource structure.
    InvalidResource {
        /// Description of what was invalid.
        message: String,
        /// URL of the problematic parameter, if known.
        url: Option<String>,
    },

    /// Missing required field in SearchParameter.
    MissingField {
        /// Name of the missing field.
        field: String,
        /// URL of the parameter.
        url: Option<String>,
    },

    /// Invalid FHIRPath expression in SearchParameter.
    InvalidExpression {
        /// The invalid expression.
        expression: String,
        /// Parser error message.
        error: String,
    },

    /// Failed to read embedded parameters.
    EmbeddedLoadFailed {
        /// FHIR version attempted.
        version: String,
        /// Error message.
        message: String,
    },

    /// Failed to read config file.
    ConfigLoadFailed {
        /// Path to the config file.
        path: String,
        /// Error message.
        message: String,
    },

    /// Storage error when loading stored parameters.
    StorageError {
        /// Error message.
        message: String,
    },
}

impl fmt::Display for LoaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoaderError::InvalidResource { message, url } => {
                if let Some(url) = url {
                    write!(f, "Invalid SearchParameter '{}': {}", url, message)
                } else {
                    write!(f, "Invalid SearchParameter: {}", message)
                }
            }
            LoaderError::MissingField { field, url } => {
                if let Some(url) = url {
                    write!(
                        f,
                        "SearchParameter '{}' missing required field '{}'",
                        url, field
                    )
                } else {
                    write!(f, "SearchParameter missing required field '{}'", field)
                }
            }
            LoaderError::InvalidExpression { expression, error } => {
                write!(f, "Invalid FHIRPath expression '{}': {}", expression, error)
            }
            LoaderError::EmbeddedLoadFailed { version, message } => {
                write!(
                    f,
                    "Failed to load embedded {} parameters: {}",
                    version, message
                )
            }
            LoaderError::ConfigLoadFailed { path, message } => {
                write!(f, "Failed to load config from '{}': {}", path, message)
            }
            LoaderError::StorageError { message } => {
                write!(f, "Storage error loading parameters: {}", message)
            }
        }
    }
}

impl std::error::Error for LoaderError {}

/// Error during registry operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RegistryError {
    /// Parameter with this URL already exists.
    DuplicateUrl {
        /// The duplicate URL.
        url: String,
    },

    /// Parameter not found in registry.
    NotFound {
        /// The URL or code that was not found.
        identifier: String,
    },

    /// Invalid parameter definition.
    InvalidDefinition {
        /// Description of the problem.
        message: String,
    },

    /// Registry is locked/read-only.
    Locked {
        /// Reason for the lock.
        reason: String,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::DuplicateUrl { url } => {
                write!(f, "SearchParameter with URL '{}' already exists", url)
            }
            RegistryError::NotFound { identifier } => {
                write!(f, "SearchParameter '{}' not found", identifier)
            }
            RegistryError::InvalidDefinition { message } => {
                write!(f, "Invalid SearchParameter definition: {}", message)
            }
            RegistryError::Locked { reason } => {
                write!(f, "Registry is locked: {}", reason)
            }
        }
    }
}

impl std::error::Error for RegistryError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loader_error_display() {
        let err = LoaderError::MissingField {
            field: "expression".to_string(),
            url: Some("http://example.org/SearchParameter/test".to_string()),
        };
        assert!(err.to_string().contains("expression"));
        assert!(err.to_string().contains("test"));
    }

    #[test]
    fn test_registry_error_display() {
        let err = RegistryError::DuplicateUrl {
            url: "http://example.org/sp".to_string(),
        };
        assert!(err.to_string().contains("already exists"));
    }
}
