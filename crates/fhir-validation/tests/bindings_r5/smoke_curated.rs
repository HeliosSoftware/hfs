//! Fast smoke tests over the curated HL7 R5 example list (binding workload + parse sanity).

use crate::common::fhir_json_examples::{
    R5_CURATED, count_severities, fhir_json_dir, is_binding_like_issue, load_r5_fhir_resource,
};
use crate::common::fixtures::local_terminology_r5;
use crate::harness::r5_evaluator_for;
use fhir_validation::Validator;

#[test]
fn curated_r5_examples_resolve_each_file() {
    let root = fhir_json_dir("R5");
    for name in R5_CURATED {
        let p = root.join(name);
        assert!(
            p.is_file(),
            "missing example {} (expected at {})",
            name,
            p.display()
        );
        let _ = load_r5_fhir_resource(name);
    }
}

#[test]
fn curated_examples_binding_validation_completes_without_excessive_issues() {
    let validator = Validator::default();
    let term = local_terminology_r5();
    for name in R5_CURATED {
        let resource = load_r5_fhir_resource(name);
        let evaluator = r5_evaluator_for(&resource);
        let issues = validator.validate_resource(&resource, Some(&term), &evaluator);
        assert!(
            issues.len() < 5000,
            "{} produced excessive issues (possible loop): {}",
            name,
            issues.len()
        );
        let (e, w, _i) = count_severities(&issues);
        let binding_n = issues.iter().filter(|i| is_binding_like_issue(i)).count();
        assert!(
            binding_n < 200,
            "{name}: binding-like issues = {binding_n}, errors={e} warnings={w}"
        );
    }
}
