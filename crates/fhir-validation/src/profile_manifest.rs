//! Load [`ProfileRegistry`](crate::profile::profile_registry::ProfileRegistry) entries from
//! on-disk `StructureDefinition` JSON (single resource or Bundle).
//!
//! Used for **IG materialization**: expand NPM packages in CI, list canonical JSON paths in a
//! manifest, then call [`load_profile_registry_from_manifest`] at process startup.
//!
//! CapabilityStatement / ImplementationGuide resources do **not** become registry rows; publish a
//! manifest that lists only `StructureDefinition` JSON paths your deployment validates against.

use crate::ValidationError;
use crate::profile::extract::extract_structure_definition_profile_from_json;
use crate::profile::profile_registry::ProfileRegistry;
use crate::profile::types::ExtractedProfile;
use serde::Deserialize;
use std::fs;
use std::path::Path;

/// Paths (relative or absolute) to JSON files. Each file is either a `StructureDefinition`
/// resource or a FHIR `Bundle` containing `StructureDefinition` entries.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileManifest {
    #[serde(default)]
    pub structure_definition_files: Vec<String>,
}

/// Load and merge all profiles from [`ProfileManifest::structure_definition_files`].
pub fn load_profile_registry_from_manifest(
    manifest: &ProfileManifest,
) -> Result<ProfileRegistry, ValidationError> {
    let mut registry = ProfileRegistry::new();
    for path in &manifest.structure_definition_files {
        merge_structure_definition_file_into_registry(Path::new(path), &mut registry)?;
    }
    Ok(registry)
}

/// Read a manifest JSON file (`structure_definition_files` array) and build a registry.
pub fn load_profile_registry_from_manifest_file(
    path: &Path,
) -> Result<ProfileRegistry, ValidationError> {
    let text = fs::read_to_string(path).map_err(|e| {
        ValidationError::Internal(format!(
            "failed to read profile manifest '{}': {e}",
            path.display()
        ))
    })?;
    let manifest: ProfileManifest = serde_json::from_str(&text).map_err(|e| {
        ValidationError::Internal(format!(
            "failed to parse profile manifest '{}': {e}",
            path.display()
        ))
    })?;
    load_profile_registry_from_manifest(&manifest)
}

fn merge_structure_definition_file_into_registry(
    path: &Path,
    registry: &mut ProfileRegistry,
) -> Result<(), ValidationError> {
    let text = fs::read_to_string(path).map_err(|e| {
        ValidationError::Internal(format!(
            "failed to read StructureDefinition file '{}': {e}",
            path.display()
        ))
    })?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        ValidationError::Internal(format!("failed to parse JSON '{}': {e}", path.display()))
    })?;

    match v.get("resourceType").and_then(|x| x.as_str()) {
        Some("StructureDefinition") => {
            insert_extracted_profile(registry, &v)?;
        }
        Some("Bundle") => {
            let Some(entries) = v.get("entry").and_then(|e| e.as_array()) else {
                return Err(ValidationError::Internal(format!(
                    "Bundle '{}' has no entry array",
                    path.display()
                )));
            };
            for entry in entries {
                let Some(res) = entry.get("resource") else {
                    continue;
                };
                if res.get("resourceType").and_then(|x| x.as_str()) != Some("StructureDefinition") {
                    continue;
                }
                insert_extracted_profile(registry, res)?;
            }
        }
        Some(other) => {
            return Err(ValidationError::Internal(format!(
                "expected StructureDefinition or Bundle in '{}', got resourceType={other}",
                path.display()
            )));
        }
        None => {
            return Err(ValidationError::Internal(format!(
                "missing resourceType in '{}'",
                path.display()
            )));
        }
    }
    Ok(())
}

fn insert_extracted_profile(
    registry: &mut ProfileRegistry,
    sd_json: &serde_json::Value,
) -> Result<(), ValidationError> {
    let profile: ExtractedProfile = extract_structure_definition_profile_from_json(sd_json)?;
    registry.insert(profile);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_minimal_bundle_with_structure_definition() {
        let dir = std::env::temp_dir().join(format!("fv_manifest_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bundle_path = dir.join("bundle.json");
        let sd: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/r4/profiles/StructureDefinition-Patient.json"
        ))
        .unwrap();
        let bundle = serde_json::json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [{ "resource": sd }]
        });
        std::fs::write(&bundle_path, serde_json::to_string(&bundle).unwrap()).unwrap();

        let mf_path = dir.join("manifest.json");
        std::fs::write(
            &mf_path,
            serde_json::to_string(&serde_json::json!({
                "structure_definition_files": [bundle_path.to_str().unwrap()]
            }))
            .unwrap(),
        )
        .unwrap();

        let reg = load_profile_registry_from_manifest_file(&mf_path).expect("load");
        assert!(!reg.is_empty());
    }
}
