use fhir_validation::{FhirPathEvaluator, InvariantExprRef, R5FhirPathEvaluator};
use helios_fhir::r5::Resource as R5Resource;
use helios_fhirpath::handlers::json_value_to_evaluation_result;
use helios_fhirpath_support::EvaluationResult;
use serde_json::json;

fn focus(value: serde_json::Value) -> EvaluationResult {
    json_value_to_evaluation_result(&value).expect("focus JSON should convert to EvaluationResult")
}

fn root_patient_resource() -> R5Resource {
    serde_json::from_value(json!({
        "resourceType": "Patient",
        "id": "p1",
        "active": true,
        "gender": "male",
        "name": [
            { "family": "Smith" }
        ]
    }))
    .expect("valid R5 Patient resource")
}
#[test]
fn bulk_single_matches_eval_invariant_on() {
    let resource = root_patient_resource();
    let focus = focus(json!({
        "resourceType": "Patient",
        "id": "p1",
        "active": true,
        "gender": "male"
    }));

    let evaluator = R5FhirPathEvaluator::new_with_focus(resource, focus.clone());

    let single = evaluator.eval_invariant_on(focus.clone(), "Patient", "active.exists()");

    let bulk = evaluator.eval_invariants_on(
        focus,
        &[InvariantExprRef {
            declared_path: "Patient",
            expression: "active.exists()",
        }],
    );

    assert_eq!(bulk.len(), 1);

    match (single, &bulk[0]) {
        (Ok(a), Ok(b)) => assert_eq!(a, *b),
        (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string()),
        _ => panic!("single and bulk results differed"),
    }
}
#[test]
fn bulk_results_preserve_input_order() {
    let resource = root_patient_resource();
    let focus = focus(json!({
        "resourceType": "Patient",
        "active": true,
        "gender": "male"
    }));

    let evaluator = R5FhirPathEvaluator::new_with_focus(resource, focus.clone());

    let results = evaluator.eval_invariants_on(
        focus,
        &[
            InvariantExprRef {
                declared_path: "Patient",
                expression: "active.exists()",
            },
            InvariantExprRef {
                declared_path: "Patient",
                expression: "gender.exists()",
            },
            InvariantExprRef {
                declared_path: "Patient",
                expression: "birthDate.exists()",
            },
        ],
    );

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].as_ref().unwrap(), &true);
    assert_eq!(results[1].as_ref().unwrap(), &true);
    assert_eq!(results[2].as_ref().unwrap(), &false);
}
#[test]
fn bulk_allows_mixed_success_false_and_error_results() {
    let resource = root_patient_resource();
    let focus = focus(json!({
        "resourceType": "Patient",
        "active": true
    }));

    let evaluator = R5FhirPathEvaluator::new_with_focus(resource, focus.clone());

    let results = evaluator.eval_invariants_on(
        focus,
        &[
            InvariantExprRef {
                declared_path: "Patient",
                expression: "active.exists()",
            },
            InvariantExprRef {
                declared_path: "Patient",
                expression: "birthDate.exists()",
            },
            InvariantExprRef {
                declared_path: "Patient",
                expression: "this is not valid fhirpath !!!",
            },
        ],
    );

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].as_ref().unwrap(), &true);
    assert_eq!(results[1].as_ref().unwrap(), &false);
    assert!(results[2].is_err());
}
#[test]
fn bulk_error_includes_declared_path() {
    let resource = root_patient_resource();
    let focus = focus(json!({
        "resourceType": "Patient",
        "active": true
    }));

    let evaluator = R5FhirPathEvaluator::new_with_focus(resource, focus.clone());

    let results = evaluator.eval_invariants_on(
        focus,
        &[InvariantExprRef {
            declared_path: "Patient.name[0].family",
            expression: "!!! not valid !!!",
        }],
    );

    assert_eq!(results.len(), 1);

    let err = results[0].as_ref().unwrap_err().to_string();
    assert!(
        err.contains("Patient.name[0].family"),
        "error did not contain declared path: {err}"
    );
}
#[test]
fn bulk_empty_input_returns_empty_results() {
    let resource = root_patient_resource();
    let focus = focus(json!({
        "resourceType": "Patient",
        "active": true
    }));

    let evaluator = R5FhirPathEvaluator::new_with_focus(resource, focus.clone());

    let results = evaluator.eval_invariants_on(focus, &[]);
    assert!(results.is_empty());
}
#[test]
fn eval_invariant_declared_path_still_works() {
    let evaluator = R5FhirPathEvaluator::new(root_patient_resource());

    let result = evaluator.eval_invariant("Patient", "active.exists()");
    assert!(result.unwrap());

    let result = evaluator.eval_invariant("Patient", "birthDate.exists()");
    assert!(!result.unwrap());
}
