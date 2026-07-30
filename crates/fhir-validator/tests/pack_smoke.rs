//! Whole-spec smoke tests over the embedded packs. `#[ignore]`d in normal
//! runs (they parse full packs); run explicitly with
//! `cargo test -p helios-fhir-validator -- --ignored`
//! (add `--features R4B,R5,R6` for the other-version sweeps).

#![cfg(feature = "R4")]

use helios_fhir::FhirVersion;
use helios_fhir_validator::packs::core_registry;
use helios_fhir_validator::{SchemaResolver, ValidationOptions, Validator};
use serde_json::json;

/// Every enabled version's pack loads and validates a minimal Patient.
#[test]
#[ignore = "whole-pack parse; run with -- --ignored"]
fn all_enabled_packs_load_and_validate() {
    let versions = [
        FhirVersion::R4,
        #[cfg(feature = "R4B")]
        FhirVersion::R4B,
        #[cfg(feature = "R5")]
        FhirVersion::R5,
        #[cfg(feature = "R6")]
        FhirVersion::R6,
    ];
    for version in versions {
        let registry = core_registry(version);
        for name in [
            "Patient",
            "Observation",
            "Bundle",
            "Resource",
            "Element",
            "string",
        ] {
            assert!(
                registry.resolve(name).is_some(),
                "{version:?}: core schema '{name}' must resolve"
            );
        }
        let validator = Validator::new(registry);
        let outcome = validator.validate_sync(
            &json!({ "resourceType": "Patient", "active": true }),
            &ValidationOptions::default(),
        );
        assert_eq!(
            outcome.errors,
            vec![],
            "{version:?}: minimal Patient must be clean"
        );
        let outcome = validator.validate_sync(
            &json!({ "resourceType": "Patient", "bogus": 1 }),
            &ValidationOptions::default(),
        );
        assert!(
            !outcome.errors.is_empty(),
            "{version:?}: unknown element must be caught"
        );
    }
}

/// Rough structural-validation latency check (debug builds are far slower
/// than release; the plan's <5ms target refers to release).
#[test]
#[ignore = "timing; run with -- --ignored"]
fn structural_validation_latency_smoke() {
    let validator = Validator::new(core_registry(FhirVersion::R4));
    let opts = ValidationOptions::default();
    let patient = json!({
        "resourceType": "Patient",
        "id": "perf",
        "extension": [{ "url": "http://x", "valueString": "v" }],
        "identifier": [{ "system": "http://example.org/mrn", "value": "12345" }],
        "active": true,
        "name": [{ "use": "official", "family": "Chalmers", "given": ["Peter", "James"] }],
        "gender": "male",
        "birthDate": "1974-12-25",
        "deceasedBoolean": false,
        "contained": [{ "resourceType": "Organization", "id": "org1", "name": "ACME" }],
        "managingOrganization": { "reference": "#org1" }
    });

    // Warm the lazy pack parse before timing.
    let _ = validator.validate_sync(&patient, &opts);

    let iterations = 200u32;
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let outcome = validator.validate_sync(&patient, &opts);
        assert!(outcome.errors.is_empty());
    }
    let per_run = start.elapsed() / iterations;
    println!("structural validation: {per_run:?} per typical Patient");
    // Generous ceiling so debug builds pass; release comfortably beats the
    // 5ms plan target (verified manually).
    assert!(
        per_run.as_millis() < 50,
        "structural validation too slow: {per_run:?}"
    );
}

#[test]
#[ignore = "whole-pack parse; run with -- --ignored"]
fn r4_pack_loads_and_resolves_core_schemas() {
    let registry = core_registry(FhirVersion::R4);
    for name in [
        "Patient",
        "Observation",
        "Bundle",
        "Resource",
        "DomainResource",
        "Element",
        "Extension",
        "HumanName",
        "string",
        "boolean",
        "dateTime",
        "Questionnaire",
    ] {
        assert!(
            registry.resolve(name).is_some(),
            "core schema '{name}' must resolve"
        );
    }
    // Canonical URLs resolve to the same schemas.
    let by_name = registry.resolve("Patient").unwrap();
    let by_url = registry
        .resolve("http://hl7.org/fhir/StructureDefinition/Patient")
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&by_name, &by_url));
    // Primitives carry their value regexes.
    assert!(registry.resolve("string").unwrap().regex.is_some());
    // Questionnaire.item recursion converted to an elementReference.
    let q = registry.resolve("Questionnaire").unwrap();
    let item = &q.elements.as_ref().unwrap()["item"];
    let nested = &item.elements.as_ref().unwrap()["item"];
    assert_eq!(
        nested.element_reference.as_deref(),
        Some(
            &[
                "Questionnaire".to_string(),
                "elements".to_string(),
                "item".to_string()
            ][..]
        )
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
        .map(|e| {
            serde_json::to_value(e.kind)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert!(
        kinds.contains(&"unknown-element".to_string()),
        "kinds: {kinds:?}"
    );
    assert!(
        kinds.contains(&"not-singular".to_string()),
        "kinds: {kinds:?}"
    );
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
        outcome
            .errors
            .iter()
            .any(|e| e.path == "Bundle.entry.0.resource.wrong"),
        "dynamic resolution must reach the nested Patient, got: {}",
        serde_json::to_string_pretty(&outcome.errors).unwrap()
    );
}

/// Regression for issue #424: a StructureDefinition whose `element.id`s carry a
/// choice suffix (`[x]`) or a slice qualifier (`:`) must NOT trip a
/// primitive-value error. Before the converter fix, R4B/R5 typed
/// `ElementDefinition.id` as the constrained `id` primitive, whose regex
/// `[A-Za-z0-9\-\.]{1,64}` rejects exactly these ids — so the spec's own
/// profiles failed validation. R4 was always clean (typed `string`), so it
/// serves as the control that the assertion is meaningful.
///
/// Version-gated behind R4B/R5 features (that is where the bug lived) and
/// `#[ignore]`d like the other whole-pack tests.
#[cfg(any(feature = "R4B", feature = "R5"))]
#[test]
#[ignore = "whole-pack parse; run with -- --ignored --features R4B,R5"]
fn structuredefinition_element_ids_with_choice_or_slice_are_not_rejected() {
    // A trimmed but structurally-valid StructureDefinition whose element ids
    // include a choice base (`Observation.value[x]`) and a named slice
    // (`Observation.category:vital-signs`). These are the exact shapes the `id`
    // regex rejected.
    let sd = json!({
        "resourceType": "StructureDefinition",
        "url": "http://example.org/StructureDefinition/choice-and-slice",
        "name": "ChoiceAndSlice",
        "status": "active",
        "kind": "resource",
        "abstract": false,
        "type": "Observation",
        "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
        "derivation": "constraint",
        "differential": { "element": [
            { "id": "Observation.value[x]", "path": "Observation.value[x]", "min": 0, "max": "1" },
            {
                "id": "Observation.category:vital-signs",
                "path": "Observation.category",
                "sliceName": "vital-signs",
                "min": 0, "max": "1"
            }
        ]}
    });

    let versions = [
        #[cfg(feature = "R4B")]
        FhirVersion::R4B,
        #[cfg(feature = "R5")]
        FhirVersion::R5,
    ];
    for version in versions {
        let validator = Validator::new(core_registry(version));
        let outcome = validator.validate_sync(&sd, &ValidationOptions::default());

        // The precise #424 failure: a primitive-value error on an element id.
        let id_value_errors: Vec<_> = outcome
            .errors
            .iter()
            .filter(|e| {
                e.kind == helios_fhir_validator::ErrorKind::PrimitiveValue
                    && e.path.ends_with(".id")
            })
            .collect();
        assert!(
            id_value_errors.is_empty(),
            "{version:?}: element ids with [x]/: must not fail primitive-value validation, got: {}",
            serde_json::to_string_pretty(&id_value_errors).unwrap()
        );
    }
}
