//! Opt-in extensible-strength binding warnings.

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use helios_fhir_validator::{
    CodedValue, EffectHandlers, FhirSchema, SchemaRegistry, Severity, TerminologyError,
    TerminologyProvider, ValidationOptions, Validator,
};
use serde_json::json;
use std::sync::Arc;

struct RejectAll;

#[async_trait]
impl TerminologyProvider for RejectAll {
    async fn validate_code(
        &self,
        _value_set: &str,
        _coded: &CodedValue,
    ) -> Result<bool, TerminologyError> {
        Ok(false)
    }
}

fn validator() -> Validator {
    let mut registry = SchemaRegistry::new();
    registry.insert_named(
        "string",
        serde_json::from_value::<FhirSchema>(json!({ "kind": "primitive-type" })).unwrap(),
    );
    registry.insert_named(
        "CodeableConcept",
        serde_json::from_value::<FhirSchema>(json!({
            "elements": { "text": { "type": "string" } }
        }))
        .unwrap(),
    );
    registry.insert_named(
        "Patient",
        serde_json::from_value::<FhirSchema>(json!({
            "elements": {
                "resourceType": { "type": "string" },
                "maritalStatus": {
                    "type": "CodeableConcept",
                    "binding": {
                        "valueSet": "http://hl7.org/fhir/ValueSet/marital-status",
                        "strength": "extensible"
                    }
                }
            }
        }))
        .unwrap(),
    );
    Validator::new(Arc::new(registry))
}

#[tokio::test]
async fn extensible_unchecked_by_default() {
    let reject = RejectAll;
    let handlers = EffectHandlers {
        terminology: Some(&reject),
        ..Default::default()
    };
    let errors = validator()
        .validate(
            &json!({
                "resourceType": "Patient",
                "maritalStatus": { "text": "married" }
            }),
            FhirVersion::R4,
            &ValidationOptions::default(),
            &handlers,
        )
        .await;
    assert!(errors.is_empty(), "{errors:?}");
}

#[tokio::test]
async fn extensible_warns_when_opted_in() {
    let reject = RejectAll;
    let handlers = EffectHandlers {
        terminology: Some(&reject),
        check_extensible_bindings: true,
        ..Default::default()
    };
    let errors = validator()
        .validate(
            &json!({
                "resourceType": "Patient",
                "maritalStatus": { "text": "married" }
            }),
            FhirVersion::R4,
            &ValidationOptions::default(),
            &handlers,
        )
        .await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].severity, Severity::Warning);
    assert!(errors[0].message.contains("marital-status"));
}
