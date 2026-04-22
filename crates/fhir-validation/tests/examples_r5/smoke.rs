//! Smoke validation on curated files under `crates/fhir/tests/data/json/R5/`.

use crate::common::fhir_json_examples::load_r5_fhir_resource;
use crate::common::fixtures::local_terminology_r5;
use fhir_validation::{R5FhirPathEvaluator, Validator};
use helios_fhir::FhirResource;

fn r5_evaluator(resource: &FhirResource) -> R5FhirPathEvaluator {
    let FhirResource::R5(r) = resource else {
        unreachable!()
    };
    R5FhirPathEvaluator::new((**r).clone())
}

#[test]
fn patient_genomic_example_issues_are_well_formed() {
    let resource = load_r5_fhir_resource("Patient-genomicPatient.json");
    let evaluator = r5_evaluator(&resource);
    let term = local_terminology_r5();
    let issues = Validator::default().validate_resource(&resource, Some(&term), &evaluator);
    for issue in &issues {
        assert!(
            !issue.diagnostics.is_empty() || issue.summary.is_some(),
            "issue should carry diagnostics or summary: {issue:?}"
        );
    }
}

#[test]
fn practitioner_example_validation_stays_bounded() {
    let resource = load_r5_fhir_resource("Practitioner-practitioner01.json");
    let evaluator = r5_evaluator(&resource);
    let term = local_terminology_r5();
    let issues = Validator::default().validate_resource(&resource, Some(&term), &evaluator);
    assert!(
        issues.len() < 10_000,
        "absurd issue count suggests a loop bug: {}",
        issues.len()
    );
}
