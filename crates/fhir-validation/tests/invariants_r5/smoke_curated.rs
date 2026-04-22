//! Curated HL7 R5 JSON corpus: validation completes; invariant-like issues stay bounded.

use crate::common::fhir_json_examples::{
    R5_CURATED, count_severities, fhir_json_dir, is_invariant_like_issue, list_r5_json_filenames,
    load_r5_fhir_resource, read_fhir_example_json, resource_type_of_json,
};
use crate::common::fixtures::local_terminology_r5;
use crate::harness::r5_evaluator_for;
use fhir_validation::{R5FhirPathEvaluator, Validator};
use helios_fhir::FhirResource;

fn evaluator_for(resource: &FhirResource) -> R5FhirPathEvaluator {
    r5_evaluator_for(resource)
}

#[test]
fn curated_examples_produce_finite_invariant_issue_summary() {
    let validator = Validator::default();
    let term = local_terminology_r5();
    for name in R5_CURATED {
        let resource = load_r5_fhir_resource(name);
        let evaluator = evaluator_for(&resource);
        let issues = validator.validate_resource(&resource, Some(&term), &evaluator);
        let inv = issues.iter().filter(|i| is_invariant_like_issue(i)).count();
        let (e, _w, _i) = count_severities(&issues);
        assert!(
            inv < 500 && e < 500,
            "{name}: invariant_like issues={inv} errors={e}"
        );
    }
}

#[test]
fn practitioner_example_issues_have_diagnostics_or_summary() {
    let resource = load_r5_fhir_resource("Practitioner-practitioner01.json");
    let evaluator = evaluator_for(&resource);
    let term = local_terminology_r5();
    let issues = Validator::default().validate_resource(&resource, Some(&term), &evaluator);
    for issue in &issues {
        assert!(
            !issue.diagnostics.is_empty() || issue.summary.is_some(),
            "issue should carry diagnostics or summary: {issue:?}"
        );
    }
}

/// Broader sweep of the R5 example corpus (capped). Run manually when changing validation core.
#[test]
#[ignore = "slow: validates up to 100 JSON files from crates/fhir/tests/data/json/R5"]
fn smoke_many_r5_examples_validate_with_bounded_issues() {
    let names = list_r5_json_filenames(100);
    assert!(
        names.len() >= 10,
        "expected example corpus under {}",
        fhir_json_dir("R5").display()
    );
    let validator = Validator::default();
    let term = local_terminology_r5();
    for name in names {
        let path = fhir_json_dir("R5").join(&name);
        let json = read_fhir_example_json("R5", &name);
        let _rt = resource_type_of_json(&path, &json);
        let resource = load_r5_fhir_resource(&name);
        let evaluator = evaluator_for(&resource);
        let issues = validator.validate_resource(&resource, Some(&term), &evaluator);
        let (e, w, _i) = count_severities(&issues);
        assert!(
            e < 500 && w < 500 && issues.len() < 2000,
            "{}: too many issues (errors={e} warnings={w} total={})",
            name,
            issues.len()
        );
    }
}
