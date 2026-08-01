use crate::converter;
use crate::packages::error::PackageError;
use crate::packages::scan::scan_package_dir;
use crate::resolver::SchemaRegistry;
use serde_json::Value;
use std::path::Path;

/// Names of FHIR infrastructure roots excluded from package registries.
const ABSTRACT_ROOTS: &[&str] = &["Element", "BackboneElement", "Resource", "DomainResource"];

/// Outcome of converting a package's StructureDefinitions into a registry.
#[derive(Debug, Clone, Default)]
pub struct MaterializeReport {
    pub inserted: usize,
    pub skipped_abstract: usize,
    pub convert_errors: Vec<String>,
    pub warnings: Vec<String>,
    pub code_systems_seen: usize,
    pub value_sets_seen: usize,
}

/// Scan `package_root`, convert StructureDefinitions, insert into a new
/// [`SchemaRegistry`]. Soft-fails individual bad SDs (recorded in the report);
/// always returns a registry (possibly empty).
pub fn materialize_package(
    package_root: &Path,
) -> Result<(SchemaRegistry, MaterializeReport), PackageError> {
    let scanned = scan_package_dir(package_root)?;
    let mut registry = SchemaRegistry::new();
    let mut report = MaterializeReport {
        code_systems_seen: scanned.code_system_paths.len(),
        value_sets_seen: scanned.value_set_paths.len(),
        warnings: scanned.skipped_files,
        ..Default::default()
    };

    for sd in &scanned.structure_definitions {
        if should_skip_sd(sd) {
            report.skipped_abstract += 1;
            continue;
        }
        match converter::convert(sd) {
            Ok(conversion) => {
                for w in conversion.warnings {
                    report.warnings.push(w);
                }
                if registry.insert(conversion.schema) {
                    report.inserted += 1;
                } else {
                    report.warnings.push(format!(
                        "StructureDefinition has neither url nor name: {}",
                        sd_label(sd)
                    ));
                }
            }
            Err(e) => {
                report
                    .convert_errors
                    .push(format!("{}: {e}", sd_label(sd)));
            }
        }
    }

    Ok((registry, report))
}

fn should_skip_sd(sd: &Value) -> bool {
    let name = sd.get("name").and_then(Value::as_str).unwrap_or("");
    let id = sd.get("id").and_then(Value::as_str).unwrap_or("");
    if ABSTRACT_ROOTS.contains(&name) || ABSTRACT_ROOTS.contains(&id) {
        return true;
    }
    // Abstract infrastructure without derivation — same class of poison as Element.
    let is_abstract = sd.get("abstract").and_then(Value::as_bool) == Some(true);
    let has_derivation = sd
        .get("derivation")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    let has_base = sd
        .get("baseDefinition")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    is_abstract && !has_derivation && !has_base
}

fn sd_label(sd: &Value) -> String {
    sd.get("url")
        .and_then(Value::as_str)
        .or_else(|| sd.get("name").and_then(Value::as_str))
        .or_else(|| sd.get("id").and_then(Value::as_str))
        .unwrap_or("<unknown StructureDefinition>")
        .to_string()
}
