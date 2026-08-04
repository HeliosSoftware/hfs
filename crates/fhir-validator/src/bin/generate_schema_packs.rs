//! Dev-time generator for the embedded core schema packs.
//!
//! Reads the FHIR spec bundles already vendored for code generation
//! (`crates/fhir-gen/resources/{R4,R4B,R5,R6}/profiles-{types,resources,others}.json`),
//! converts every StructureDefinition to FHIR Schema, and writes one sorted,
//! gzipped JSON array per version to `crates/fhir-validator/packs/`.
//!
//! The packs are **committed** (same workflow as the fhir-gen generated
//! models) and embedded via `include_bytes!` in `src/packs.rs`. Re-run after
//! spec updates:
//!
//! ```text
//! cargo run -p helios-fhir-validator --features gen-packs --bin generate-schema-packs
//! cargo run -p helios-fhir-validator --features gen-packs --bin generate-schema-packs -- R5 R6
//! ```
//!
//! Versions whose spec files are absent (R6 before fhir-gen's download has
//! run) are skipped with a warning.

use flate2::Compression;
use flate2::write::GzEncoder;
use helios_fhir_validator::converter::convert;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const SOURCE_FILES: [&str; 4] = [
    "profiles-types.json",
    "profiles-resources.json",
    "profiles-others.json",
    // Core extension definitions (#363) — vendored per version where the
    // profiled-extension picker is wanted; absent files are skipped.
    "extension-definitions.json",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let versions: Vec<&str> = if args.is_empty() {
        vec!["R4", "R4B", "R5", "R6"]
    } else {
        args.iter().map(String::as_str).collect()
    };

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let resources_root = manifest_dir.join("../fhir-gen/resources");
    let packs_dir = manifest_dir.join("packs");
    fs::create_dir_all(&packs_dir).expect("create packs dir");

    let mut failed = false;
    for version in versions {
        match generate_pack(&resources_root.join(version), &packs_dir, version) {
            Ok(report) => println!("{report}"),
            Err(e) => {
                eprintln!("{version}: {e}");
                failed = true;
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}

fn generate_pack(source_dir: &Path, packs_dir: &Path, version: &str) -> Result<String, String> {
    if !source_dir.exists() {
        return Ok(format!(
            "{version}: SKIPPED — spec directory {} not present (run a fhir-gen R6 build to download)",
            source_dir.display()
        ));
    }

    let mut schemas: Vec<Value> = Vec::new();
    let mut sd_count = 0usize;
    let mut warning_count = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for file in SOURCE_FILES {
        let path = source_dir.join(file);
        if !path.exists() {
            eprintln!("{version}: {file} missing; continuing without it");
            continue;
        }
        let raw = fs::read_to_string(&path).map_err(|e| format!("read {file}: {e}"))?;
        let bundle: Value = serde_json::from_str(&raw).map_err(|e| format!("parse {file}: {e}"))?;
        let entries = bundle
            .get("entry")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{file}: no Bundle.entry"))?;

        for entry in entries {
            let Some(resource) = entry.get("resource") else {
                continue;
            };
            if resource.get("resourceType").and_then(Value::as_str) != Some("StructureDefinition") {
                continue;
            }
            sd_count += 1;
            let sd_url = resource
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("<no url>");
            match convert(resource) {
                Ok(conversion) => {
                    warning_count += conversion.warnings.len();
                    for w in &conversion.warnings {
                        eprintln!("{version}: [warn] {sd_url}: {w}");
                    }
                    schemas
                        .push(serde_json::to_value(&conversion.schema).expect("schema serializes"));
                }
                Err(e) => failures.push(format!("{sd_url}: {e}")),
            }
        }
    }

    if !failures.is_empty() {
        return Err(format!(
            "{} of {} StructureDefinitions failed to convert:\n  {}",
            failures.len(),
            sd_count,
            failures.join("\n  ")
        ));
    }

    // Diff-stable output: sort by url.
    schemas.sort_by(|a, b| {
        a.get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(b.get("url").and_then(Value::as_str).unwrap_or_default())
    });

    let json = serde_json::to_vec(&schemas).expect("pack serializes");
    let out_path = packs_dir.join(format!("fhir_schemas_{}.json.gz", version.to_lowercase()));
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&json).expect("gzip write");
    let compressed = encoder.finish().expect("gzip finish");
    fs::write(&out_path, &compressed).map_err(|e| format!("write pack: {e}"))?;

    Ok(format!(
        "{version}: {} schemas from {} StructureDefinitions ({} converter warnings) → {} ({} KB raw, {} KB gz)",
        schemas.len(),
        sd_count,
        warning_count,
        out_path.display(),
        json.len() / 1024,
        compressed.len() / 1024
    ))
}
