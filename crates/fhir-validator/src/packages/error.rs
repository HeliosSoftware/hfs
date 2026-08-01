use std::path::PathBuf;
use thiserror::Error;

/// Errors from package cache, resolution, or materialization.
#[derive(Debug, Error)]
pub enum PackageError {
    /// I/O failure while reading or writing the cache.
    #[error("package I/O error at {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Archive extract or JSON parse failure.
    #[error("{0}")]
    Invalid(String),
    /// Requested package is not present in the cache.
    #[error("package {name}@{version} not found in cache (offline resolve)")]
    NotInCache { name: String, version: String },
    /// `package.json` missing or malformed.
    #[error("{0}")]
    Manifest(String),
    /// Dependency graph problem (missing dep, cycle, FHIR version mismatch).
    #[error("{0}")]
    Resolve(String),
    /// StructureDefinition conversion failed hard enough to abort.
    #[error("{0}")]
    Convert(String),
}

impl PackageError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
