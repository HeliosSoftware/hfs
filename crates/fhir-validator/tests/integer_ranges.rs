//! `integer`, `positiveInt` and `unsignedInt` values outside their 32-bit
//! range are structural errors. They used to pass the structural walk, then
//! fail the typed parse the constraint evaluator needs, which turned every
//! FHIRPath invariant of the resource into a warning: in enforce mode the
//! write was accepted without any invariant being checked.
#![cfg(feature = "R4")]

use helios_fhir::FhirVersion;
use helios_fhir_validator::{Severity, ValidationOptions, Validator, packs};
use serde_json::{Value, json};

fn patient_with_photo_size(size: Value) -> Value {
    json!({
        "resourceType": "Patient",
        "photo": [{"contentType": "image/png", "size": size}]
    })
}

fn structural_errors(resource: &Value) -> Vec<String> {
    let validator = Validator::new(packs::core_registry(FhirVersion::R4));
    validator
        .validate_sync(resource, &ValidationOptions::default())
        .errors
        .into_iter()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| serde_json::to_string(&e).unwrap())
        .collect()
}

#[test]
fn an_unsigned_int_outside_its_range_is_an_error() {
    for size in [json!(3_000_000_000u64), json!(-1)] {
        let errors = structural_errors(&patient_with_photo_size(size.clone()));
        assert!(
            errors
                .iter()
                .any(|e| e.contains("outside the range of unsignedInt")),
            "{size}: {errors:#?}"
        );
    }
}

#[test]
fn values_inside_the_range_are_accepted() {
    for size in [json!(0), json!(2_147_483_647)] {
        let errors = structural_errors(&patient_with_photo_size(size.clone()));
        assert!(
            !errors.iter().any(|e| e.contains("outside the range")),
            "{size}: {errors:#?}"
        );
    }
}

/// With constraints evaluated, the out-of-range value is an error, so an
/// enforce-mode write is refused rather than accepted on warnings only.
#[cfg(feature = "fhirpath")]
#[tokio::test]
async fn full_validation_reports_an_error_not_only_warnings() {
    use helios_fhir_validator::EffectHandlers;
    use helios_fhir_validator::fhirpath_effects::FhirPathConstraintEvaluator;

    let validator = Validator::new(packs::core_registry(FhirVersion::R4));
    let evaluator = FhirPathConstraintEvaluator::new();
    let effects = EffectHandlers {
        constraints: Some(&evaluator),
        ..Default::default()
    };
    let issues = validator
        .validate(
            &patient_with_photo_size(json!(3_000_000_000u64)),
            FhirVersion::R4,
            &ValidationOptions::default(),
            &effects,
        )
        .await;
    assert!(
        issues.iter().any(|e| e.severity == Severity::Error),
        "{issues:#?}"
    );
}
