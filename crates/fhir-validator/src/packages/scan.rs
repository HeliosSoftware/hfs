use crate::packages::error::PackageError;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// JSON resources discovered under an expanded FHIR NPM package.
#[derive(Debug, Clone, Default)]
pub struct ScannedPackage {
    pub structure_definitions: Vec<Value>,
    /// Paths of CodeSystem JSON (not loaded into the schema registry).
    pub code_system_paths: Vec<PathBuf>,
    /// Paths of ValueSet JSON (not loaded into the schema registry).
    pub value_set_paths: Vec<PathBuf>,
    pub skipped_files: Vec<String>,
}

/// Walk `package_root` for `*.json`, collect StructureDefinition resources
/// (standalone or Bundle entries). Skips `.index.json` and `package.json`.
pub fn scan_package_dir(package_root: &Path) -> Result<ScannedPackage, PackageError> {
    if !package_root.is_dir() {
        return Err(PackageError::Invalid(format!(
            "package root is not a directory: {}",
            package_root.display()
        )));
    }

    let mut json_files = Vec::new();
    walk_json_files(package_root, &mut json_files)?;
    json_files.sort();
    json_files.dedup();

    let mut out = ScannedPackage::default();
    for path in json_files {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name == "package.json" || name == ".index.json" || name == ".sha256" {
            continue;
        }
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                out.skipped_files
                    .push(format!("{}: read error: {e}", path.display()));
                continue;
            }
        };
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                out.skipped_files
                    .push(format!("{}: JSON parse error: {e}", path.display()));
                continue;
            }
        };
        collect_from_value(&v, &path, &mut out);
    }
    Ok(out)
}

fn collect_from_value(v: &Value, path: &Path, out: &mut ScannedPackage) {
    match v.get("resourceType").and_then(Value::as_str) {
        Some("StructureDefinition") => out.structure_definitions.push(v.clone()),
        Some("CodeSystem") => out.code_system_paths.push(path.to_path_buf()),
        Some("ValueSet") => out.value_set_paths.push(path.to_path_buf()),
        Some("Bundle") => {
            if let Some(entries) = v.get("entry").and_then(Value::as_array) {
                for entry in entries {
                    if let Some(res) = entry.get("resource") {
                        match res.get("resourceType").and_then(Value::as_str) {
                            Some("StructureDefinition") => {
                                out.structure_definitions.push(res.clone());
                            }
                            Some("CodeSystem") => {
                                out.code_system_paths.push(path.to_path_buf());
                            }
                            Some("ValueSet") => {
                                out.value_set_paths.push(path.to_path_buf());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn walk_json_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), PackageError> {
    for ent in fs::read_dir(dir).map_err(|e| PackageError::io(dir, e))? {
        let ent = ent.map_err(|e| PackageError::io(dir, e))?;
        let p = ent.path();
        if p.is_dir() {
            walk_json_files(&p, out)?;
        } else if p
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        {
            out.push(p);
        }
    }
    Ok(())
}
