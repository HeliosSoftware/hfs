use fhir_validation::profile::extract::extract_structure_definition_profile_from_json;
use fhir_validation::profile::profile_registry::ProfileRegistry;
use fhir_validation::profile::types::{ExtractedElementRule, ExtractedProfile};
use fhir_validation::{ValidationConfig, Validator};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

fn load_hl7_sd(path_under_data_json: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fhir/tests/data/json")
        .join(path_under_data_json);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("parse json")
}

fn registry_patient_and_organization() -> ProfileRegistry {
    let mut reg = ProfileRegistry::new();
    for p in ["R4/patient.profile.json", "R4/organization.profile.json"] {
        let v = load_hl7_sd(p);
        let prof = extract_structure_definition_profile_from_json(&v).unwrap_or_else(|e| {
            panic!("extract {p}: {e}");
        });
        reg.insert(prof);
    }
    reg
}

fn validator_addons_on() -> Validator {
    let mut cfg = ValidationConfig::default();
    cfg.strict_json_properties = true;
    cfg.validate_base_snapshot_cardinality = true;
    Validator::new(cfg)
}

/// Strict property checks only run when the snapshot lists child paths for the parent. For nested
/// behaviour, use a minimal base profile that includes `Patient.name` and `Patient.name.family`.
fn minimal_patient_base_for_nested_strict() -> ProfileRegistry {
    let profile = ExtractedProfile {
        url: "http://hl7.org/fhir/StructureDefinition/Patient".into(),
        resource_type: "Patient".into(),
        element_rules: vec![
            ExtractedElementRule {
                path: "Patient.name".into(),
                id: "Patient.name".into(),
                ..Default::default()
            },
            ExtractedElementRule {
                path: "Patient.name.family".into(),
                id: "Patient.name.family".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut reg = ProfileRegistry::new();
    reg.insert(profile);
    reg
}

#[test]
fn strict_rejects_unknown_nested_property() {
    let reg = minimal_patient_base_for_nested_strict();
    let v = validator_addons_on();
    let root = json!({
        "resourceType": "Patient",
        "name": [{
            "family": "Doe",
            "madeUpNestedField": 1
        }]
    });
    let issues = v.apply_validation_addons(&root, "Patient", &reg);
    assert!(
        issues
            .iter()
            .any(|i| i.diagnostics.contains("madeUpNestedField")),
        "{issues:?}"
    );
}

#[test]
fn strict_rejects_unknown_top_level_property() {
    let reg = registry_patient_and_organization();
    let v = validator_addons_on();
    let root = json!({
        "resourceType": "Patient",
        "notARealField": true
    });
    let issues = v.apply_validation_addons(&root, "Patient", &reg);
    assert!(
        issues
            .iter()
            .any(|i| i.diagnostics.contains("notARealField")),
        "{issues:?}"
    );
}

#[test]
fn strict_rejects_extension_with_disallowed_key() {
    let reg = registry_patient_and_organization();
    let v = validator_addons_on();
    let root = json!({
        "resourceType": "Patient",
        "extension": [{
            "url": "http://example.org/x",
            "disallowedExtKey": "nope"
        }]
    });
    let issues = v.apply_validation_addons(&root, "Patient", &reg);
    assert!(
        issues.iter().any(
            |i| i.diagnostics.contains("disallowedExtKey") || i.diagnostics.contains("unknown")
        ),
        "{issues:?}"
    );
}

#[test]
fn strict_rejects_unknown_property_in_contained() {
    let reg = registry_patient_and_organization();
    let v = validator_addons_on();
    let root = json!({
        "resourceType": "Patient",
        "id": "p1",
        "contained": [{
            "resourceType": "Organization",
            "id": "org1",
            "name": "Acme",
            "bogusContainedField": 1
        }]
    });
    let issues = v.apply_validation_addons(&root, "Patient", &reg);
    assert!(
        issues
            .iter()
            .any(|i| i.diagnostics.contains("bogusContainedField")),
        "{issues:?}"
    );
}

#[test]
fn strict_accepts_minimal_valid_patient_json() {
    let reg = registry_patient_and_organization();
    let v = validator_addons_on();
    let root = json!({ "resourceType": "Patient" });
    let strict_issues: Vec<_> = v
        .apply_validation_addons(&root, "Patient", &reg)
        .into_iter()
        .filter(|i| i.code == "structure" && i.diagnostics.contains("Unknown JSON property"))
        .collect();
    assert!(
        strict_issues.is_empty(),
        "unexpected strict property issues: {strict_issues:?}"
    );
}
