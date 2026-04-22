//! Load FHIR JSON examples shipped with the `helios_fhir` crate (`crates/fhir/tests/data/json/`).
//!
//! Paths are resolved from `fhir-validation`’s manifest dir (`../fhir/tests/data/json/{R4,R5,R6}/`).
//!
//! Each integration test binary compiles this module independently; not every helper is used in
//! every binary, so suppress dead-code noise.
#![allow(dead_code)]

use fhir_validation::ValidationIssue;
use fhir_validation::ValidationIssueDetailCode;
use helios_fhir::FhirResource;
use std::fs;
use std::path::{Path, PathBuf};

pub fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `crates/fhir/tests/data/json/{version}` (e.g. `version` = `"R5"`).
pub fn fhir_json_dir(version: &str) -> PathBuf {
    manifest_dir().join("../fhir/tests/data/json").join(version)
}

pub fn read_fhir_example_json(version: &str, filename: &str) -> String {
    let path = fhir_json_dir(version).join(filename);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}
#[cfg(feature = "R5")]
pub fn load_r5_fhir_resource(relative: &str) -> FhirResource {
    let json = read_fhir_example_json("R5", relative);
    let r: helios_fhir::r5::Resource =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("parse R5 {}: {e}", relative));
    FhirResource::R5(Box::new(r))
}

/// Curated HL7-style examples (genomics / clinical narrative) — kept small for fast CI.
pub const R5_CURATED: &[&str] = &[
    "Patient-genomicPatient.json",
    "Patient-denovoChild.json",
    "Practitioner-practitioner01.json",
    "Practitioner-practitioner02.json",
    "Encounter-genomicEncounter.json",
    "Encounter-denovoEncounter.json",
    "ServiceRequest-genomicServiceRequest.json",
    "ServiceRequest-genomicSRProband.json",
    "DocumentReference-genomicVCFfile.json",
    "Group-denovoFamily.json",
    "RelatedPerson-denovoFather.json",
    "Requirements-example1.json",
    "coverageeligibilityrequest-example.json",
];

#[allow(dead_code)]
pub fn list_r5_json_filenames(max_files: usize) -> Vec<String> {
    let dir = fhir_json_dir("R5");
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    names.sort();
    names.truncate(max_files);
    names
}

pub fn count_severities(issues: &[ValidationIssue]) -> (usize, usize, usize) {
    use fhir_validation::Severity;
    let mut errors = 0usize;
    let mut warnings = 0usize;
    let mut infos = 0usize;
    for i in issues {
        match i.severity {
            Severity::Error | Severity::Fatal => errors += 1,
            Severity::Warning => warnings += 1,
            Severity::Information => infos += 1,
        }
    }
    (errors, warnings, infos)
}

pub fn is_binding_like_issue(i: &ValidationIssue) -> bool {
    match i.detail_code {
        Some(
            ValidationIssueDetailCode::RequiredBindingMiss
            | ValidationIssueDetailCode::ExtensibleBindingMiss
            | ValidationIssueDetailCode::PreferredBindingMiss
            | ValidationIssueDetailCode::ExampleBindingMiss
            | ValidationIssueDetailCode::TerminologyValidationFailed
            | ValidationIssueDetailCode::InvalidBindableValue
            | ValidationIssueDetailCode::CodeWithoutSystem,
        ) => true,
        _ => i.code == "terminology",
    }
}

pub fn is_invariant_like_issue(i: &ValidationIssue) -> bool {
    matches!(
        i.detail_code,
        Some(ValidationIssueDetailCode::ConstraintViolation)
    ) || i.code == "invariant"
}

pub fn resource_type_of_json(path: &Path, json: &str) -> String {
    let v: serde_json::Value =
        serde_json::from_str(json).unwrap_or_else(|e| panic!("JSON {}: {e}", path.display()));
    v.get("resourceType")
        .and_then(|x| x.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("{}: missing resourceType", path.display()))
}
