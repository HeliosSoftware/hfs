//! [`validate_profile`] on `Patient` with only root-level constraints (element rules cleared).

use crate::common::fixtures::{load_fixture, load_profile, load_resource};
use crate::harness::{r5_evaluator_for, run_validate_profile};
use fhir_validation::Validator;
use helios_fhir::FhirVersion;

#[test]
fn root_invariant_active_implies_name_evaluates_via_validate_profile() {
    let json_str = load_fixture(FhirVersion::R5, "profile/only-invariants.json");
    let json: serde_json::Value = serde_json::from_str(&json_str).expect("json");
    let fhir = load_resource(FhirVersion::R5, "profile/only-invariants.json");
    let mut profile = load_profile(FhirVersion::R5, "profile/atrius-profile.json");
    profile.element_rules.clear();

    let evaluator = r5_evaluator_for(&fhir);
    let issues = run_validate_profile(
        &Validator::default(),
        &json,
        "Patient",
        &profile,
        &evaluator,
    );
    assert!(
        issues
            .iter()
            .any(|i| { i.expression.as_deref() == Some("active = true implies name.exists()") }),
        "expected atrius root invariant expression, got: {issues:#?}"
    );
}
