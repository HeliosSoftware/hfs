//! HL7 examples under `crates/fhir/tests/data/json/R5/`: explicit invariant expectations where useful.

use crate::common::fhir_json_examples::load_r5_fhir_resource;
use crate::common::fixtures::local_terminology_r5;
use crate::harness::r5_evaluator_for;
use fhir_validation::{Severity, Validator};

fn invariant_errors(issues: &[fhir_validation::ValidationIssue]) -> usize {
    issues
        .iter()
        .filter(|i| {
            i.code == "invariant" && matches!(i.severity, Severity::Error | Severity::Fatal)
        })
        .count()
}

#[test]
fn patient_genomic_example_has_no_invariant_errors() {
    let resource = load_r5_fhir_resource("Patient-genomicPatient.json");
    let evaluator = r5_evaluator_for(&resource);
    let term = local_terminology_r5();
    let issues = Validator::default().validate_resource(&resource, Some(&term), &evaluator);
    assert_eq!(
        invariant_errors(&issues),
        0,
        "unexpected invariant errors: {issues:#?}"
    );
}

#[test]
fn requirements_example1_has_no_invariant_errors() {
    let resource = load_r5_fhir_resource("Requirements-example1.json");
    let evaluator = r5_evaluator_for(&resource);
    let term = local_terminology_r5();
    let issues = Validator::default().validate_resource(&resource, Some(&term), &evaluator);
    assert_eq!(invariant_errors(&issues), 0, "{issues:#?}");
}
