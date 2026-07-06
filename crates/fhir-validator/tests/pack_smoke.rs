//! Whole-spec smoke tests over the embedded R4 pack. `#[ignore]`d in normal
//! runs (they parse the full pack); run explicitly with
//! `cargo test -p helios-fhir-validator -- --ignored`.

#![cfg(feature = "R4")]

use helios_fhir::FhirVersion;
use helios_fhir_validator::packs::core_registry;
use helios_fhir_validator::{SchemaResolver, ValidationOptions, Validator};
use serde_json::json;

#[test]
#[ignore = "whole-pack parse; run with -- --ignored"]
fn r4_pack_loads_and_resolves_core_schemas() {
    let registry = core_registry(FhirVersion::R4);
    for name in ["Patient", "Observation", "Bundle", "Resource", "DomainResource", "Element",
        "Extension", "HumanName", "string", "boolean", "dateTime", "Questionnaire"]
    {
        assert!(registry.resolve(name).is_some(), "core schema '{name}' must resolve");
    }
    // Canonical URLs resolve to the same schemas.
    let by_name = registry.resolve("Patient").unwrap();
    let by_url = registry.resolve("http://hl7.org/fhir/StructureDefinition/Patient").unwrap();
    assert!(std::sync::Arc::ptr_eq(&by_name, &by_url));
    // Primitives carry their value regexes.
    assert!(registry.resolve("string").unwrap().regex.is_some());
    // Questionnaire.item recursion converted to an elementReference.
    let q = registry.resolve("Questionnaire").unwrap();
    let item = &q.elements.as_ref().unwrap()["item"];
    let nested = &item.elements.as_ref().unwrap()["item"];
    assert_eq!(
        nested.element_reference.as_deref(),
        Some(&["Questionnaire".to_string(), "elements".to_string(), "item".to_string()][..])
    );
}

#[test]
#[ignore = "whole-pack parse; run with -- --ignored"]
fn r4_pack_validates_known_good_and_bad_resources() {
    let registry = core_registry(FhirVersion::R4);
    let validator = Validator::new(registry);
    let opts = ValidationOptions::default();

    // A well-formed Patient with common shapes: choice type, arrays,
    // primitive sidecar, contained resource, extension.
    let good = json!({
        "resourceType": "Patient",
        "id": "example",
        "meta": { "versionId": "1" },
        "extension": [{
            "url": "http://example.org/unknown-extension",
            "valueString": "free-form"
        }],
        "identifier": [{ "system": "http://example.org/mrn", "value": "12345" }],
        "active": true,
        "name": [{ "use": "official", "family": "Chalmers", "given": ["Peter", "James"] }],
        "gender": "male",
        "birthDate": "1974-12-25",
        "_birthDate": {
            "extension": [{
                "url": "http://hl7.org/fhir/StructureDefinition/patient-birthTime",
                "valueDateTime": "1974-12-25T14:35:45-05:00"
            }]
        },
        "deceasedBoolean": false,
        "contained": [{
            "resourceType": "Organization",
            "id": "org1",
            "name": "ACME Healthcare"
        }],
        "managingOrganization": { "reference": "#org1" }
    });
    let outcome = validator.validate_sync(&good, &opts);
    assert_eq!(
        outcome.errors,
        vec![],
        "known-good Patient must validate clean, got: {}",
        serde_json::to_string_pretty(&outcome.errors).unwrap()
    );

    // Structural breakage must surface.
    let bad = json!({
        "resourceType": "Patient",
        "bogusElement": true,
        "gender": ["male"],
        "name": { "family": "NotAnArray" },
        "deceasedBoolean": false,
        "deceasedDateTime": "2020-01-01"
    });
    let outcome = validator.validate_sync(&bad, &opts);
    let kinds: Vec<String> = outcome
        .errors
        .iter()
        .map(|e| serde_json::to_value(e.kind).unwrap().as_str().unwrap().to_string())
        .collect();
    assert!(kinds.contains(&"unknown-element".to_string()), "kinds: {kinds:?}");
    assert!(kinds.contains(&"not-singular".to_string()), "kinds: {kinds:?}");
    assert!(kinds.contains(&"not-array".to_string()), "kinds: {kinds:?}");
    assert!(kinds.contains(&"choice".to_string()), "kinds: {kinds:?}");

    // A Bundle whose entry resource is dynamically resolved.
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [{ "resource": { "resourceType": "Patient", "wrong": 1 } }]
    });
    let outcome = validator.validate_sync(&bundle, &opts);
    assert!(
        outcome.errors.iter().any(|e| e.path == "Bundle.entry.0.resource.wrong"),
        "dynamic resolution must reach the nested Patient, got: {}",
        serde_json::to_string_pretty(&outcome.errors).unwrap()
    );
}
