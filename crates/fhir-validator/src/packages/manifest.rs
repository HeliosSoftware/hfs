use crate::packages::error::PackageError;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;

/// A pinned FHIR NPM package identity (`name@version`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackageId {
    pub name: String,
    pub version: String,
}

impl PackageId {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version)
    }
}

/// Operator-facing package reference — same as [`PackageId`] for v1
/// (exact versions only; no ranges).
pub type PackageRef = PackageId;

impl PackageRef {
    /// Parse `name@version`. Uses `rsplit_once('@')` so names may contain dots.
    pub fn parse(s: &str) -> Result<Self, PackageError> {
        let s = s.trim();
        let (name, version) = s.rsplit_once('@').ok_or_else(|| {
            PackageError::Invalid(format!(
                "package ref '{s}' must be name@version (exact version)"
            ))
        })?;
        if name.is_empty() || version.is_empty() {
            return Err(PackageError::Invalid(format!(
                "package ref '{s}' has empty name or version"
            )));
        }
        Ok(Self::new(name, version))
    }
}

/// Subset of FHIR NPM `package.json` fields used for materialization.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    /// Declared FHIR versions, e.g. `["4.0.1"]`.
    #[serde(default)]
    pub fhir_versions: Vec<String>,
    #[serde(default)]
    pub canonical: Option<String>,
}

impl PackageManifest {
    pub fn load(path: &Path) -> Result<Self, PackageError> {
        let text = fs::read_to_string(path).map_err(|e| PackageError::io(path, e))?;
        let manifest: Self = serde_json::from_str(&text).map_err(|e| {
            PackageError::Manifest(format!("failed to parse {}: {e}", path.display()))
        })?;
        if manifest.name.is_empty() || manifest.version.is_empty() {
            return Err(PackageError::Manifest(format!(
                "{} missing name or version",
                path.display()
            )));
        }
        Ok(manifest)
    }

    pub fn id(&self) -> PackageId {
        PackageId::new(&self.name, &self.version)
    }

    /// Dependency pins as exact [`PackageId`]s (version strings used as-is).
    pub fn dependency_ids(&self) -> Vec<PackageId> {
        self.dependencies
            .iter()
            .map(|(name, version)| PackageId::new(name, version))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ref() {
        let r = PackageRef::parse("hl7.fhir.r4.core@4.0.1").unwrap();
        assert_eq!(r.name, "hl7.fhir.r4.core");
        assert_eq!(r.version, "4.0.1");
    }
}
