use std::fmt;
use std::path::PathBuf;

/// Errors from package cache, resolution, or materialization.
#[derive(Debug)]
pub enum PackageError {
    /// I/O failure while reading or writing the cache.
    Io { path: PathBuf, source: std::io::Error },
    /// Archive extract or JSON parse failure.
    Invalid(String),
    /// Requested package is not present in the cache.
    NotInCache { name: String, version: String },
    /// `package.json` missing or malformed.
    Manifest(String),
    /// Dependency graph problem (missing dep, cycle, FHIR version mismatch).
    Resolve(String),
    /// StructureDefinition conversion failed hard enough to abort.
    Convert(String),
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "package I/O error at {}: {source}", path.display())
            }
            Self::Invalid(msg) | Self::Manifest(msg) | Self::Resolve(msg) | Self::Convert(msg) => {
                write!(f, "{msg}")
            }
            Self::NotInCache { name, version } => {
                write!(
                    f,
                    "package {name}@{version} not found in cache (offline resolve)"
                )
            }
        }
    }
}

impl std::error::Error for PackageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl PackageError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
