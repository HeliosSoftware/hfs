//! Values the structural walk used to accept but the typed model refuses.
//!
//! The FHIRPath constraint evaluator works on the typed model. A resource the
//! typed model cannot load has none of its invariants evaluated, so each of
//! these let an enforce-mode write through with every invariant unchecked
//! (#2893). The walk now reports them where they are, and full validation of
//! any resource the typed model refuses is an error whatever the cause.
#![cfg(feature = "R4")]

use helios_fhir::FhirVersion;
use helios_fhir_validator::{Severity, ValidationOptions, Validator, packs};
use serde_json::{Value, json};

fn structural_errors(version: FhirVersion, resource: &Value) -> Vec<String> {
    Validator::new(packs::core_registry(version))
        .validate_sync(resource, &ValidationOptions::default())
        .errors
        .into_iter()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| format!("{}: {}", e.path, e.message))
        .collect()
}

fn assert_error(version: FhirVersion, resource: Value, expected: &str) {
    let errors = structural_errors(version, &resource);
    assert!(
        errors.iter().any(|e| e.contains(expected)),
        "{resource}: expected `{expected}` in {errors:#?}"
    );
}

fn assert_valid(version: FhirVersion, resource: Value) {
    let errors = structural_errors(version, &resource);
    assert!(errors.is_empty(), "{resource}: {errors:#?}");
}

#[test]
fn a_leap_second_is_an_error_in_date_time_instant_and_time() {
    let leap = "leap second";
    for issued in [
        "2015-06-30T23:59:60Z",
        "2015-06-30T23:59:60+01:00",
        "2015-06-30T23:59:60.5Z",
    ] {
        assert_error(
            FhirVersion::R4,
            json!({"resourceType": "Observation", "status": "final", "code": {"text": "x"}, "issued": issued}),
            leap,
        );
    }
    assert_error(
        FhirVersion::R4,
        json!({"resourceType": "Patient", "contact": [{"name": {"text": "x"}, "period": {"start": "2015-06-30T23:59:60Z"}}]}),
        leap,
    );
    assert_error(
        FhirVersion::R4,
        json!({"resourceType": "Patient", "deceasedDateTime": "2015-06-30T23:59:60Z"}),
        leap,
    );
    assert_error(
        FhirVersion::R4,
        json!({"resourceType": "Observation", "status": "final", "code": {"text": "x"}, "valueTime": "23:59:60"}),
        leap,
    );
}

#[test]
fn ordinary_times_are_accepted() {
    for issued in [
        "2015-06-30T23:59:59Z",
        "2015-06-30T23:00:59.600Z",
        "2015-06-30T10:06:00Z",
        "2060-06-30T00:00:00+06:00",
    ] {
        assert_valid(
            FhirVersion::R4,
            json!({"resourceType": "Observation", "status": "final", "code": {"text": "x"}, "issued": issued}),
        );
    }
    assert_valid(
        FhirVersion::R4,
        json!({"resourceType": "Observation", "status": "final", "code": {"text": "x"}, "valueTime": "06:00:59"}),
    );
}

#[test]
fn an_inline_resource_without_a_resource_type_is_an_error() {
    let required = "resourceType is required";
    assert_error(
        FhirVersion::R4,
        json!({"resourceType": "Patient", "contained": [{"id": "o"}]}),
        required,
    );
    assert_error(
        FhirVersion::R4,
        json!({"resourceType": "Patient", "contained": [{}]}),
        required,
    );
    assert_error(
        FhirVersion::R4,
        json!({"resourceType": "Bundle", "type": "collection", "entry": [{"resource": {"id": "x"}}]}),
        required,
    );
    assert_error(
        FhirVersion::R4,
        json!({"resourceType": "Bundle", "type": "batch-response", "entry": [{"response": {"status": "200", "outcome": {"id": "x"}}}]}),
        required,
    );
    assert_error(
        FhirVersion::R4,
        json!({"resourceType": "Parameters", "parameter": [{"name": "a", "resource": {"id": "x"}}]}),
        required,
    );
}

#[test]
fn inline_resources_with_a_resource_type_are_accepted() {
    assert_valid(
        FhirVersion::R4,
        json!({"resourceType": "Patient", "contained": [{"resourceType": "Organization", "id": "o", "name": "x"}]}),
    );
    assert_valid(
        FhirVersion::R4,
        json!({"resourceType": "Bundle", "type": "collection", "entry": [{"resource": {"resourceType": "Patient"}}]}),
    );
}

/// R5 `Attachment.size` is an `integer64`: a string in FHIR JSON.
#[cfg(feature = "R5")]
mod integer64 {
    use super::*;

    fn patient(size: Value) -> Value {
        json!({"resourceType": "Patient", "photo": [{"contentType": "image/png", "size": size}]})
    }

    #[test]
    fn the_string_form_is_accepted() {
        for size in [
            "0",
            "3000000000",
            "-5",
            "+5",
            "9223372036854775807",
            "-9223372036854775808",
        ] {
            assert_valid(FhirVersion::R5, patient(json!(size)));
        }
    }

    /// The typed model writes the number form, so it is tolerated.
    #[test]
    fn the_number_form_is_accepted_inside_the_range() {
        for size in [
            json!(0),
            json!(3_000_000_000u64),
            json!(i64::MAX),
            json!(i64::MIN),
        ] {
            assert_valid(FhirVersion::R5, patient(size));
        }
    }

    #[test]
    fn a_value_outside_the_range_is_an_error() {
        let range = "outside the range of integer64";
        assert_error(FhirVersion::R5, patient(json!(u64::MAX)), range);
        assert_error(
            FhirVersion::R5,
            patient(json!("9223372036854775808")),
            range,
        );
        assert_error(
            FhirVersion::R5,
            patient(json!("-9223372036854775809")),
            range,
        );
        assert_error(
            FhirVersion::R5,
            patient(json!("99999999999999999999")),
            range,
        );
    }

    #[test]
    fn a_malformed_value_is_an_error() {
        for size in [
            json!("abc"),
            json!(""),
            json!("05"),
            json!("-0"),
            json!("1.5"),
            json!("-"),
        ] {
            assert_error(FhirVersion::R5, patient(size), "is not a valid integer64");
        }
        assert_error(FhirVersion::R5, patient(json!(1.5)), "expected integer64");
        assert_error(FhirVersion::R5, patient(json!(true)), "expected integer64");
    }
}

/// Each of these used to reach the constraint evaluator with no structural
/// error, fail the typed parse there and come back as warnings only.
#[cfg(feature = "fhirpath")]
mod full_validation {
    use super::*;
    use helios_fhir_validator::EffectHandlers;
    use helios_fhir_validator::fhirpath_effects::FhirPathConstraintEvaluator;

    async fn error_count(version: FhirVersion, resource: Value) -> usize {
        let validator = Validator::new(packs::core_registry(version));
        let evaluator = FhirPathConstraintEvaluator::new();
        let effects = EffectHandlers {
            constraints: Some(&evaluator),
            ..Default::default()
        };
        validator
            .validate(&resource, version, &ValidationOptions::default(), &effects)
            .await
            .iter()
            .filter(|e| e.severity == Severity::Error)
            .count()
    }

    #[tokio::test]
    async fn resources_the_typed_model_refuses_are_errors() {
        // Each violates an invariant too (pat-1, obs-3): the write this
        // guards against is one that gets it stored.
        for resource in [
            json!({"resourceType": "Observation", "status": "final", "code": {"text": "x"},
                   "referenceRange": [{}], "issued": "2015-06-30T23:59:60Z"}),
            json!({"resourceType": "Patient", "contact": [{"gender": "male"}],
                   "contained": [{"id": "o"}]}),
            json!({"resourceType": "Patient", "contact": [{"gender": "male",
                   "period": {"start": "2015-06-30T23:59:60Z"}}]}),
        ] {
            assert!(
                error_count(FhirVersion::R4, resource.clone()).await > 0,
                "{resource}"
            );
        }
    }

    #[cfg(feature = "R5")]
    #[tokio::test]
    async fn an_integer64_outside_the_range_is_an_error() {
        let resource = json!({"resourceType": "Patient", "contact": [{"gender": "male"}],
                              "photo": [{"size": u64::MAX}]});
        assert!(error_count(FhirVersion::R5, resource).await > 0);
    }
}
