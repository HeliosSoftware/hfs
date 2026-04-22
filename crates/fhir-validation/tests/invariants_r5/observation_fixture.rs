//! Complex `Observation` fixture mixing value, `dataAbsentReason`, contained `Provenance`, etc.

use crate::common::fhir_json_examples::is_invariant_like_issue;
use crate::common::fixtures::{load_resource, local_terminology_r5};
use crate::harness::r5_evaluator_for;
use fhir_validation::Validator;
use helios_fhir::FhirVersion;

#[test]
fn observation_fixture_emits_at_least_one_invariant_like_issue() {
    let r = load_resource(FhirVersion::R5, "invalid/obs-resource-element-test.json");
    let validator = Validator::default();
    let evaluator = r5_evaluator_for(&r);
    let term = local_terminology_r5();
    let issues = validator.validate_resource(&r, Some(&term), &evaluator);
    let n = issues.iter().filter(|i| is_invariant_like_issue(i)).count();
    assert!(
        n >= 1,
        "expected at least one invariant-like issue, got {n}: {issues:#?}"
    );
}
