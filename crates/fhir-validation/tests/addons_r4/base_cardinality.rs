use fhir_validation::profile::extract::extract_structure_definition_profile_from_json;
use fhir_validation::profile::profile_registry::ProfileRegistry;
use fhir_validation::{ValidationConfig, Validator};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

fn hl7_r4_patient_registry() -> ProfileRegistry {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fhir/tests/data/json/R4/patient.profile.json");
    let text = fs::read_to_string(&path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let p = extract_structure_definition_profile_from_json(&v).unwrap();
    let mut reg = ProfileRegistry::new();
    reg.insert(p);
    reg
}

fn validator_addons_on() -> Validator {
    let mut cfg = ValidationConfig::default();
    cfg.strict_json_properties = true;
    cfg.validate_base_snapshot_cardinality = true;
    Validator::new(cfg)
}

/// `Patient.communication.language` has `min: 1` when a `communication` entry exists (HL7 R4 snapshot).
#[test]
fn base_min_cardinality_nested_required_when_slice_present() {
    let reg = hl7_r4_patient_registry();
    let v = validator_addons_on();
    let root = json!({
        "resourceType": "Patient",
        "communication": [{}]
    });
    let issues = v.apply_validation_addons(&root, "Patient", &reg);
    assert!(
        issues
            .iter()
            .any(|i| i.code == "required" && i.fhir_path.contains("communication.language")),
        "{issues:?}"
    );
}

/// `Patient.active` has `max: "1"` in the HL7 R4 snapshot; two values via a JSON array exceeds it.
#[test]
fn base_max_cardinality_exceeded_for_active() {
    let reg = hl7_r4_patient_registry();
    let v = validator_addons_on();
    let root = json!({
        "resourceType": "Patient",
        "active": [true, false]
    });
    let issues = v.apply_validation_addons(&root, "Patient", &reg);
    assert!(
        issues.iter().any(|i| i.code == "structure"
            && i.diagnostics.contains("exceeds maximum cardinality")
            && i.fhir_path.contains("active")),
        "{issues:?}"
    );
}

#[test]
fn clean_patient_json_no_cardinality_errors_from_addons() {
    let reg = hl7_r4_patient_registry();
    let v = validator_addons_on();
    let root = json!({ "resourceType": "Patient" });
    let card_issues: Vec<_> = v
        .apply_validation_addons(&root, "Patient", &reg)
        .into_iter()
        .filter(|i| {
            i.code == "required"
                || i.code == "structure"
                    && i.summary.as_deref()
                        == Some("Element exceeds maximum cardinality for this profile")
        })
        .collect();
    assert!(card_issues.is_empty(), "{card_issues:?}");
}
