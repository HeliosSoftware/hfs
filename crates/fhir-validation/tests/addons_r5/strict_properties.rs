use fhir_validation::profile::extract::extract_structure_definition_profile_from_json;
use fhir_validation::profile::profile_registry::ProfileRegistry;
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

fn hl7_r5_patient_registry() -> ProfileRegistry {
    let mut reg = ProfileRegistry::new();
    let v = load_hl7_sd("R5/patient.profile.json");
    let prof = extract_structure_definition_profile_from_json(&v)
        .unwrap_or_else(|e| panic!("extract R5 patient.profile: {e}"));
    reg.insert(prof);
    reg
}

fn validator_addons_on() -> Validator {
    let mut cfg = ValidationConfig::default();
    cfg.strict_json_properties = true;
    cfg.validate_base_snapshot_cardinality = true;
    Validator::new(cfg)
}

#[test]
fn strict_rejects_unknown_top_level_property_r5() {
    let reg = hl7_r5_patient_registry();
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
fn strict_accepts_minimal_valid_patient_json_r5() {
    let reg = hl7_r5_patient_registry();
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
