//! CompartmentDefinition loader.
//!
//! Reads the FHIR spec `CompartmentDefinition` resources from the data
//! directory, as stored-shape JSON ready to seed into storage. This is the
//! compartment analogue of
//! [`SearchParameterLoader::load_spec_resources`](crate::search::loader::SearchParameterLoader::load_spec_resources):
//! the server seeds these into primary storage at startup so
//! `GET /CompartmentDefinition` and the web UI read the same definitions the
//! codegen'd membership table (`crate::get_compartment_params`) is built from.

use std::path::Path;

use serde_json::Value;

use crate::FhirVersion;
use crate::search::errors::LoaderError;

/// Loader for CompartmentDefinition spec resources.
pub struct CompartmentDefinitionLoader {
    fhir_version: FhirVersion,
}

impl CompartmentDefinitionLoader {
    /// Creates a new loader for the specified FHIR version.
    pub fn new(fhir_version: FhirVersion) -> Self {
        Self { fhir_version }
    }

    /// Returns the FHIR version.
    pub fn version(&self) -> FhirVersion {
        self.fhir_version
    }

    /// Returns the spec bundle filename for the configured FHIR version.
    ///
    /// Mirrors the `compartment-definitions-{version}.json` bundles shipped
    /// under the data directory, alongside `search-parameters-{version}.json`.
    #[allow(unreachable_patterns)]
    pub fn spec_filename(&self) -> &'static str {
        match self.fhir_version {
            #[cfg(feature = "R4")]
            FhirVersion::R4 => "compartment-definitions-r4.json",
            #[cfg(feature = "R4B")]
            FhirVersion::R4B => "compartment-definitions-r4b.json",
            #[cfg(feature = "R5")]
            FhirVersion::R5 => "compartment-definitions-r5.json",
            #[cfg(feature = "R6")]
            FhirVersion::R6 => "compartment-definitions-r6.json",
            _ => "compartment-definitions-r4.json",
        }
    }

    /// Returns the raw CompartmentDefinition resources from the spec bundle,
    /// as stored-shape JSON.
    ///
    /// The seeding companion the server hands to storage: each entry carries
    /// its bundle id (`patient`, `encounter`, …), so re-seeding is idempotent
    /// via `create`'s `AlreadyExists`.
    pub fn load_spec_resources(&self, data_dir: &Path) -> Result<Vec<Value>, LoaderError> {
        let path = data_dir.join(self.spec_filename());
        let content =
            std::fs::read_to_string(&path).map_err(|e| LoaderError::ConfigLoadFailed {
                path: path.display().to_string(),
                message: e.to_string(),
            })?;
        let json: Value =
            serde_json::from_str(&content).map_err(|e| LoaderError::ConfigLoadFailed {
                path: path.display().to_string(),
                message: format!("Invalid JSON: {}", e),
            })?;

        let mut resources = Vec::new();
        if let Some(entries) = json.get("entry").and_then(|e| e.as_array()) {
            for entry in entries {
                if let Some(resource) = entry.get("resource")
                    && resource.get("resourceType").and_then(|t| t.as_str())
                        == Some("CompartmentDefinition")
                {
                    resources.push(resource.clone());
                }
            }
        }
        Ok(resources)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Repo-root `data/` directory, from this crate's manifest dir.
    fn workspace_data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
    }

    #[cfg(feature = "R4")]
    #[test]
    fn loads_r4_compartment_definitions() {
        let loader = CompartmentDefinitionLoader::new(FhirVersion::R4);
        let resources = loader.load_spec_resources(&workspace_data_dir()).unwrap();
        // Device, Encounter, Patient, Practitioner, RelatedPerson (questionnaire
        // has no code and is excluded from the bundle).
        assert_eq!(resources.len(), 5);
        assert!(
            resources
                .iter()
                .all(|r| r["resourceType"] == "CompartmentDefinition")
        );
        assert!(resources.iter().any(|r| r["code"] == "Patient"));
        assert!(
            resources
                .iter()
                .all(|r| r.get("id").and_then(|i| i.as_str()).is_some())
        );
    }

    #[test]
    fn missing_dir_errors() {
        let loader = CompartmentDefinitionLoader::new(FhirVersion::default_enabled());
        assert!(
            loader
                .load_spec_resources(Path::new("/nonexistent/data/dir"))
                .is_err()
        );
    }

    /// The shipped `data/compartment-definitions-*.json` bundles must say
    /// exactly what the codegen'd `get_compartment_params` table says — every
    /// `(compartment, resource)` slot, every version. This is the guard against
    /// the seeded copies drifting from `crates/fhir-gen/resources/`.
    fn assert_parity(version: FhirVersion) {
        let loader = CompartmentDefinitionLoader::new(version);
        let defs = loader.load_spec_resources(&workspace_data_dir()).unwrap();
        assert!(!defs.is_empty(), "{version:?} bundle is empty");
        for def in &defs {
            let code = def["code"].as_str().unwrap();
            for resource in def["resource"].as_array().unwrap() {
                let rtype = resource["code"].as_str().unwrap();
                let params: Vec<&str> = resource
                    .get("param")
                    .and_then(|p| p.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                let runtime = crate::get_compartment_params(version, code, rtype);
                assert_eq!(
                    runtime,
                    params.as_slice(),
                    "{version:?} compartment {code} / {rtype}"
                );
            }
        }
    }

    #[cfg(feature = "R4")]
    #[test]
    fn r4_bundle_matches_codegen_table() {
        assert_parity(FhirVersion::R4);
    }

    #[cfg(feature = "R4B")]
    #[test]
    fn r4b_bundle_matches_codegen_table() {
        assert_parity(FhirVersion::R4B);
    }

    #[cfg(feature = "R5")]
    #[test]
    fn r5_bundle_matches_codegen_table() {
        assert_parity(FhirVersion::R5);
    }

    #[cfg(feature = "R6")]
    #[test]
    fn r6_bundle_matches_codegen_table() {
        assert_parity(FhirVersion::R6);
    }
}
