//! Opt-in `refers` target-type enforcement.

use helios_fhir_validator::{
    FhirSchema, SchemaRegistry, UnknownProfilePolicy, ValidationOptions, Validator,
};
use serde_json::json;
use std::sync::Arc;

#[test]
fn enforce_refers_rejects_disallowed_type() {
    let mut registry = SchemaRegistry::new();
    registry.insert_named(
        "string",
        serde_json::from_value::<FhirSchema>(json!({ "kind": "primitive-type" })).unwrap(),
    );
    registry.insert_named(
        "Reference",
        serde_json::from_value::<FhirSchema>(json!({
            "elements": { "reference": { "type": "string" } }
        }))
        .unwrap(),
    );
    registry.insert_named(
        "Resource",
        serde_json::from_value::<FhirSchema>(json!({
            "elements": {
                "resourceType": { "type": "string" },
                "subject": { "type": "Reference", "refers": ["Patient", "Group"] }
            }
        }))
        .unwrap(),
    );

    let validator = Validator::new(Arc::new(registry));
    let opts = ValidationOptions {
        profiles: vec![],
        use_meta_profiles: true,
        unknown_profile: UnknownProfilePolicy::Error,
        enforce_refers: true,
    };
    let outcome = validator.validate_sync(
        &json!({
            "resourceType": "Resource",
            "subject": { "reference": "Organization/1" }
        }),
        &opts,
    );
    assert!(
        outcome
            .errors
            .iter()
            .any(|e| e.message.contains("Organization") && e.message.contains("refers")),
        "{:?}",
        outcome.errors
    );
}

#[test]
fn enforce_refers_off_by_default() {
    let mut registry = SchemaRegistry::new();
    registry.insert_named(
        "string",
        serde_json::from_value::<FhirSchema>(json!({ "kind": "primitive-type" })).unwrap(),
    );
    registry.insert_named(
        "Reference",
        serde_json::from_value::<FhirSchema>(json!({
            "elements": { "reference": { "type": "string" } }
        }))
        .unwrap(),
    );
    registry.insert_named(
        "Resource",
        serde_json::from_value::<FhirSchema>(json!({
            "elements": {
                "resourceType": { "type": "string" },
                "subject": { "type": "Reference", "refers": ["Patient"] }
            }
        }))
        .unwrap(),
    );
    let validator = Validator::new(Arc::new(registry));
    let outcome = validator.validate_sync(
        &json!({
            "resourceType": "Resource",
            "subject": { "reference": "Organization/1" }
        }),
        &ValidationOptions::default(),
    );
    assert!(
        outcome
            .errors
            .iter()
            .all(|e| e.kind != helios_fhir_validator::ErrorKind::ReferenceTarget),
        "{:?}",
        outcome.errors
    );
}
