// A FHIR primitive element that carries extensions but no value exists in the
// tree, but has no system value: value-consuming functions receive an empty
// collection and return empty, and ordering comparisons yield empty
// (tests-fhir-r5.xml `primitivesWithoutValue`).

use helios_fhir::FhirResource;
use helios_fhir::r4::Patient;
use helios_fhirpath::{EvaluationContext, evaluate_expression};
use helios_fhirpath_support::EvaluationResult;
use serde_json::json;

fn valueless_given_context() -> EvaluationContext {
    let patient_json = json!({
        "resourceType": "Patient",
        "id": "example",
        "name": [
            {
                "use": "maiden",
                "family": "Windsor",
                "given": [null, "James"],
                "_given": [
                    {
                        "extension": [
                            {
                                "url": "https://example.org/syllable-count",
                                "valueString": "five"
                            }
                        ]
                    }
                ]
            }
        ]
    });

    let patient: Patient = serde_json::from_value(patient_json).unwrap();
    let resource = FhirResource::R4(Box::new(helios_fhir::r4::Resource::Patient(Box::new(
        patient,
    ))));
    EvaluationContext::new(vec![resource])
}

#[test]
fn valueless_element_is_present_but_has_no_value() {
    let ctx = valueless_given_context();
    assert_eq!(
        evaluate_expression("Patient.name.given.first().exists()", &ctx).unwrap(),
        EvaluationResult::boolean(true)
    );
    assert_eq!(
        evaluate_expression("Patient.name.given.first().hasValue()", &ctx).unwrap(),
        EvaluationResult::boolean(false)
    );
    assert_eq!(
        evaluate_expression("Patient.name.given.last().length()", &ctx).unwrap(),
        EvaluationResult::integer(5)
    );
}

#[test]
fn value_consuming_functions_return_empty_for_valueless_element() {
    let ctx = valueless_given_context();
    let base = "Patient.name.given.first()";
    for call in [
        "length()",
        "toChars()",
        "substring(0, 1)",
        "upper()",
        "lower()",
        "indexOf('a')",
        "contains('a')",
        "startsWith('a')",
        "endsWith('a')",
        "matches('a')",
        "matchesFull('a')",
        "replace('a', 'b')",
        "replaceMatches('a', 'b')",
        "trim()",
        "split(',')",
        "encode('base64')",
        "decode('base64')",
    ] {
        let expr = format!("{base}.{call}");
        assert_eq!(
            evaluate_expression(&expr, &ctx).unwrap(),
            EvaluationResult::Empty,
            "{expr} must be empty"
        );
    }
}

#[test]
fn ordering_comparisons_with_valueless_element_return_empty() {
    let ctx = valueless_given_context();
    for expr in [
        "Patient.name.given.first() < 'x'",
        "Patient.name.given.first() <= 'x'",
        "Patient.name.given.first() > 'x'",
        "Patient.name.given.first() >= 'x'",
        "'x' < Patient.name.given.first()",
        "'x' >= Patient.name.given.first()",
    ] {
        assert_eq!(
            evaluate_expression(expr, &ctx).unwrap(),
            EvaluationResult::Empty,
            "{expr} must be empty"
        );
    }
}

#[test]
fn invariant_pattern_over_valueless_element_is_not_true() {
    let ctx = valueless_given_context();
    let expr = "Patient.name.given.first().exists() and Patient.name.given.first().length() = 1";
    assert_eq!(
        evaluate_expression(expr, &ctx).unwrap(),
        EvaluationResult::Empty
    );
}
