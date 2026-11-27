//! An `elementReference` that leads back to an element already in the
//! schema set ends the closure instead of recursing. A tenant profile can
//! give an untyped element a `contentReference` to itself, or two elements
//! references to each other; each hop used to wrap the target in a fresh
//! `Arc`, so the identity dedup never matched and the walk overflowed the
//! stack.

use std::sync::Arc;

use helios_fhir_validator::{
    FhirSchema, SchemaRegistry, UnknownProfilePolicy, ValidationOptions, Validator,
};
use serde_json::{Value, json};

const PROFILE_URL: &str = "https://example.invalid/StructureDefinition/loop";

fn schema(value: Value) -> FhirSchema {
    serde_json::from_value(value).expect("schema parses")
}

fn validator(profile_elements: Value) -> Validator {
    let mut registry = SchemaRegistry::new();
    registry.insert_named(
        "Patient",
        schema(json!({
            "kind": "resource", "type": "Patient",
            "elements": { "resourceType": { "type": "code" } }
        })),
    );
    registry.insert_named("code", schema(json!({ "kind": "primitive-type" })));
    registry.insert_named(
        PROFILE_URL,
        schema(json!({
            "kind": "resource", "type": "Patient", "base": "Patient",
            "elements": profile_elements
        })),
    );
    Validator::new(Arc::new(registry))
}

fn validate(validator: &Validator, data: &Value) -> usize {
    let opts = ValidationOptions {
        profiles: vec![PROFILE_URL.to_string()],
        unknown_profile: UnknownProfilePolicy::Error,
        ..Default::default()
    };
    validator.validate_sync(data, &opts).errors.len()
}

#[test]
fn an_element_referencing_itself_is_validated_once() {
    let validator = validator(json!({
        "loop": { "elementReference": [PROFILE_URL, "elements", "loop"] }
    }));
    validate(
        &validator,
        &json!({ "resourceType": "Patient", "loop": {} }),
    );
}

#[test]
fn two_elements_referencing_each_other_are_validated_once() {
    let validator = validator(json!({
        "a": { "elementReference": [PROFILE_URL, "elements", "b"] },
        "b": { "elementReference": [PROFILE_URL, "elements", "a"] }
    }));
    validate(
        &validator,
        &json!({ "resourceType": "Patient", "a": {}, "b": {} }),
    );
}
