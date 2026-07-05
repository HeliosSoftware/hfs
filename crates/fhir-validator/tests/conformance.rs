//! Runner for the vendored upstream FHIR Schema conformance suite
//! (`tests/fixtures/upstream/*.json` — see UPSTREAM.md there for provenance).
//!
//! Contract (mirrors `fhirschema-js/tests/tests.test.js`):
//! - the fixture's `schemas` map becomes the resolver, keyed by name/url
//! - each case validates `data` with the root schema auto-resolved from
//!   `data.resourceType`, plus the case's extra `schemas` as profiles
//! - the emitted error list must equal the expected `errors` (or `[]` when
//!   the key is absent) under **exact ordered deep-equality** — `type`,
//!   `path`, and `message` strings included
//! - `skip: true` cases are skipped; `focus: true` is ignored (all cases run)

use helios_fhir_validator::{
    FhirSchema, SchemaRegistry, UnknownProfilePolicy, ValidationOptions, Validator,
};
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Deserialize)]
struct FixtureFile {
    #[allow(dead_code)]
    desc: Option<String>,
    schemas: serde_json::Map<String, Value>,
    tests: Vec<FixtureCase>,
}

#[derive(Deserialize)]
struct FixtureCase {
    desc: Option<String>,
    /// Extra profile schema names to layer on top of the auto-resolved root.
    #[serde(default)]
    schemas: Vec<String>,
    data: Value,
    /// Absent ⇒ the case must validate clean.
    errors: Option<Value>,
    #[serde(default)]
    skip: bool,
}

fn fixture_dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(sub)
}

/// Runs every case in one fixture file; panics with a per-case diff report on
/// any mismatch.
fn run_fixture_file(sub: &str, name: &str) {
    let path = fixture_dir(sub).join(name);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()));
    let file: FixtureFile = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("cannot parse fixture {name}: {e}"));

    let mut registry = SchemaRegistry::new();
    for (key, def) in &file.schemas {
        let schema: FhirSchema = serde_json::from_value(def.clone())
            .unwrap_or_else(|e| panic!("fixture {name}: schema '{key}' failed to parse: {e}"));
        registry.insert_named(key.clone(), schema);
    }
    let validator = Validator::new(Arc::new(registry));

    let mut failures: Vec<String> = Vec::new();
    let mut ran = 0usize;
    for (i, case) in file.tests.iter().enumerate() {
        if case.skip {
            continue;
        }
        ran += 1;
        let desc = case.desc.clone().unwrap_or_else(|| format!("case #{i}"));
        let opts = ValidationOptions {
            profiles: case.schemas.clone(),
            // Fixtures never carry meta.profile, and unresolvable references
            // in fixtures are fixture bugs — surface them as errors.
            use_meta_profiles: true,
            unknown_profile: UnknownProfilePolicy::Error,
        };
        let outcome = validator.validate_sync(&case.data, &opts);
        let actual = serde_json::to_value(&outcome.errors).expect("errors serialize");
        let expected = case.errors.clone().unwrap_or_else(|| Value::Array(vec![]));
        if actual != expected {
            failures.push(format!(
                "  ✗ {desc}\n    expected: {expected}\n    actual:   {actual}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{name}: {}/{} cases failed:\n{}",
        failures.len(),
        ran,
        failures.join("\n")
    );
}

fn run_upstream(name: &str) {
    run_fixture_file("upstream", name)
}

#[test]
fn upstream_1_elements() {
    run_upstream("1_elements.json");
}

#[test]
fn upstream_2_base() {
    run_upstream("2_base.json");
}

#[test]
fn upstream_3_choices() {
    run_upstream("3_choices.json");
}

#[test]
fn upstream_4_required() {
    run_upstream("4_required.json");
}

#[test]
#[ignore = "Phase 2: slicing (engine/slicing.rs) not implemented yet"]
fn upstream_5_slices() {
    run_upstream("5_slices.json");
}

#[test]
#[ignore = "Phase 2: extensions→slicing compilation not implemented yet"]
fn upstream_6_extensions() {
    run_upstream("6_extensions.json");
}

#[test]
fn upstream_7_bundles() {
    run_upstream("7_bundles.json");
}
