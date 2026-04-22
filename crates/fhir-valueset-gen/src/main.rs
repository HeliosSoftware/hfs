mod generator;
mod indexer;
mod models;
mod parser;

use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::generator::emit_all;
use indexer::build_index;
use parser::parse_valuesets_bundle;

fn main() -> Result<()> {
    let version = env::args().nth(1).unwrap_or_else(|| "R5".to_string());
    if !matches!(version.as_str(), "R4" | "R4B" | "R5" | "R6") {
        anyhow::bail!("Unsupported FHIR version: {version}. Use R4 | R4B | R5 | R6");
    }

    let ver_mod = match version.as_str() {
        "R4" => "r4",
        "R4B" => "r4b",
        "R5" => "r5",
        "R6" => "r6",
        _ => unreachable!(),
    };

    // Input: crates/fhir-gen/resources/<VERSION>/valuesets.json
    let input_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("expected fhir-valueset-gen to have a parent directory")?
        .join("fhir-gen")
        .join("resources")
        .join(&version)
        .join("valuesets.json");

    if !input_path.exists() {
        anyhow::bail!("valuesets.json not found at: {}", input_path.display());
    }

    println!("📦 Loading {}", input_path.display());

    let json = fs::read_to_string(&input_path)
        .with_context(|| format!("failed reading {}", input_path.display()))?;

    let (code_systems, value_sets) = parse_valuesets_bundle(&json)?;
    println!(
        "✔ Parsed {} CodeSystems and {} ValueSets",
        code_systems.len(),
        value_sets.len()
    );

    let index = build_index(code_systems, value_sets)?;
    println!(
        "✔ Indexed {} CodeSystems ({} finite) and {} ValueSets",
        index.code_systems_by_module.len(),
        index.cs_concepts_by_url.len(),
        index.value_sets_by_module.len()
    );

    // Output: crates/fhir/src/<ver_mod>/terminology
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("expected fhir-valueset-gen to have a parent directory")?
        .join("fhir")
        .join("src")
        .join(ver_mod)
        .join("terminology");

    emit_all(&index, &out_dir)?;
    println!("✅ Generated terminology into {}", out_dir.display());

    Ok(())
}
