mod emit;
mod extract;
mod model;
mod versions;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use emit::emit_types_to_files;
use extract::{build_structure_definition_index, extract_type_validation_models_with_index};
use helios_fhir_gen::initial_fhir_model::StructureDefinition;
use serde_json::Value;
use versions::FhirVersion;

fn main() {
    if let Err(err) = run() {
        eprintln!("fhir-validation-gen: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 4 {
        return Err(usage(&args));
    }

    let version: FhirVersion = args[1]
        .parse()
        .map_err(|e: String| format!("invalid FHIR version '{}': {e}", args[1]))?;

    let output_dir = PathBuf::from(args.last().expect("output directory missing"));
    let input_paths: Vec<PathBuf> = args[2..args.len() - 1].iter().map(PathBuf::from).collect();

    let mut all_defs = Vec::new();
    for input_path in &input_paths {
        let defs = load_structure_definitions_from_bundle(input_path)?;
        all_defs.extend(defs);
    }

    let sd_index = build_structure_definition_index(&all_defs);

    let mut models = Vec::new();
    for def in &all_defs {
        let extracted = extract_type_validation_models_with_index(version, def, Some(&sd_index))
            .ok_or_else(|| {
                "could not extract validation models from one of the input bundles".to_string()
            })?;
        models.extend(extracted);
    }

    emit_types_to_files(version, &models, &output_dir, &input_paths, all_defs.len())?;

    println!(
        "Generated validation code for {} type(s) from {} bundle(s) ({}) -> {}",
        models.len(),
        input_paths.len(),
        version,
        output_dir.display()
    );

    Ok(())
}

fn usage(args: &[String]) -> String {
    let bin = args
        .first()
        .map(String::as_str)
        .unwrap_or("fhir-validation-gen");

    format!(
        "usage: {bin} <r4|r4b|r5|r6> <input-bundle-1.json> [<input-bundle-2.json> ...] <output_dir>\n\
\n\
Writes `all.rs` plus `parts/part_*.rs` and `parts/dispatch.rs` under <output_dir>.\n\
Example: .../crates/fhir-validation-gen/generated/r4"
    )
}

fn load_structure_definitions_from_bundle(path: &Path) -> Result<Vec<StructureDefinition>, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("failed to read '{}': {e}", path.display()))?;

    let json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse JSON '{}': {e}", path.display()))?;

    let resource_type = json
        .get("resourceType")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing resourceType in '{}'", path.display()))?;

    if resource_type != "Bundle" {
        return Err(format!(
            "expected Bundle input in '{}', found resourceType='{}'",
            path.display(),
            resource_type
        ));
    }

    let entries = json
        .get("entry")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("bundle '{}' is missing entry[]", path.display()))?;

    let mut defs = Vec::new();

    for entry in entries {
        let Some(resource) = entry.get("resource") else {
            continue;
        };

        let Some(resource_type) = resource.get("resourceType").and_then(Value::as_str) else {
            continue;
        };

        if resource_type != "StructureDefinition" {
            continue;
        }

        let def: StructureDefinition = serde_json::from_value(resource.clone()).map_err(|e| {
            format!(
                "failed to parse StructureDefinition entry in '{}': {e}",
                path.display()
            )
        })?;

        defs.push(def);
    }

    if defs.is_empty() {
        return Err(format!(
            "bundle '{}' did not contain any StructureDefinition entries",
            path.display()
        ));
    }

    Ok(defs)
}
