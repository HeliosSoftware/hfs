//! Direct tests of R5 binding helper functions (`validate_*_binding`), mirroring the R4 unit tests.

use fhir_validation::LocalTerminologyService;
use fhir_validation::binding::common::BindingCheckContextSync;
use fhir_validation::r5::binding::{
    validate_codeable_concept_binding, validate_codeable_reference_binding,
    validate_coding_binding, validate_primitive_code_binding, validate_primitive_value_binding,
};
use fhir_validation::{ValidationConfig, Validator};
use fhir_validation_types::{BindingStrength, Severity};
use helios_fhir::Element;
use helios_fhir::FhirVersion;
use helios_fhir::TerminologyValidationError;
use helios_fhir::r5::{Code, CodeableConcept, CodeableReference, Coding, Uri};

fn validator() -> Validator {
    Validator::new(ValidationConfig::default())
}

fn code(value: &str) -> Code {
    Element {
        id: None,
        extension: None,
        value: Some(value.to_string()),
    }
}

fn uri(value: &str) -> Uri {
    Element {
        id: None,
        extension: None,
        value: Some(value.to_string()),
    }
}

fn coding(system: &str, code_value: &str, display: Option<&str>) -> Coding {
    Coding {
        id: None,
        extension: None,
        system: Some(uri(system)),
        version: None,
        code: Some(code(code_value)),
        display: display.map(|d| Element {
            id: None,
            extension: None,
            value: Some(d.to_string()),
        }),
        user_selected: None,
    }
}

fn cc_with_one_coding(system: &str, code_value: &str) -> CodeableConcept {
    CodeableConcept {
        id: None,
        extension: None,
        coding: Some(vec![coding(system, code_value, None)]),
        text: None,
    }
}

// --- primitive `code` ---------------------------------------------------------------------------

#[test]
fn primitive_code_absent_produces_no_issue() {
    let v = validator();
    let ctx = BindingCheckContextSync::new(
        &v,
        "Patient.gender",
        "http://hl7.org/fhir/ValueSet/administrative-gender",
        BindingStrength::Required,
        None,
    );
    let issues = validate_primitive_code_binding(&ctx, None, None, |_| Ok(()));
    assert!(issues.is_empty());
}

#[test]
fn primitive_code_local_success_produces_no_issue() {
    let v = validator();
    let ctx = BindingCheckContextSync::new(
        &v,
        "Patient.gender",
        "http://hl7.org/fhir/ValueSet/administrative-gender",
        BindingStrength::Required,
        None,
    );
    let issues = validate_primitive_code_binding(&ctx, Some("male"), None, |_| Ok(()));
    assert!(issues.is_empty());
}

#[test]
fn primitive_code_remote_false_extensible_is_warning_when_service_present() {
    let term = LocalTerminologyService::new(FhirVersion::R5);
    let v = validator();
    let ctx = BindingCheckContextSync::new(
        &v,
        "Patient.language",
        "http://hl7.org/fhir/ValueSet/languages",
        BindingStrength::Extensible,
        Some(&term),
    );
    let issues = validate_primitive_code_binding(&ctx, Some("xx"), Some("remote required"), |_| {
        Err(TerminologyValidationError::RemoteValidationRequired(
            "remote required".to_string(),
        ))
    });
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, Severity::Warning);
    assert_eq!(issues[0].code, "terminology");
}

// --- primitive `string` / `uri` (shared `validate_primitive_value_binding` path) --------------

#[test]
fn primitive_string_uri_shared_path_absent_ok() {
    let v = validator();
    let ctx = BindingCheckContextSync::new(
        &v,
        "Test.value",
        "http://hl7.org/fhir/ValueSet/defined-type",
        BindingStrength::Required,
        None,
    );
    let issues = validate_primitive_value_binding(&ctx, None, |_| Ok(()));
    assert!(issues.is_empty());
}

#[test]
fn primitive_string_uri_shared_path_remote_required_without_service_errors() {
    let v = validator();
    let ctx = BindingCheckContextSync::new(
        &v,
        "Test.value",
        "http://hl7.org/fhir/ValueSet/defined-type",
        BindingStrength::Required,
        None,
    );
    let issues = validate_primitive_value_binding(&ctx, Some("any"), |_| {
        Err(TerminologyValidationError::RemoteValidationRequired(
            "remote required".to_string(),
        ))
    });
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, Severity::Error);
    assert_eq!(issues[0].code, "terminology");
}

// --- `Coding` -----------------------------------------------------------------------------------

#[test]
fn coding_absent_produces_no_issue() {
    let v = validator();
    let ctx = BindingCheckContextSync::new(
        &v,
        "Coding.test",
        "http://hl7.org/fhir/ValueSet/administrative-gender",
        BindingStrength::Required,
        None,
    );
    let issues = validate_coding_binding(&ctx, None, |_| Ok(()));
    assert!(issues.is_empty());
}

#[test]
fn coding_local_success_produces_no_issue() {
    let c = coding("http://hl7.org/fhir/administrative-gender", "male", None);
    let v = validator();
    let ctx = BindingCheckContextSync::new(
        &v,
        "Coding.test",
        "http://hl7.org/fhir/ValueSet/administrative-gender",
        BindingStrength::Required,
        None,
    );
    let issues = validate_coding_binding(&ctx, Some(&c), |_| Ok(()));
    assert!(issues.is_empty());
}

// --- `CodeableConcept` -------------------------------------------------------------------------

#[test]
fn codeable_concept_absent_produces_no_issue() {
    let v = validator();
    let ctx = BindingCheckContextSync::new(
        &v,
        "Patient.maritalStatus",
        "http://hl7.org/fhir/ValueSet/marital-status",
        BindingStrength::Extensible,
        None,
    );
    let issues = validate_codeable_concept_binding(&ctx, None, |_| Ok(()));
    assert!(issues.is_empty());
}

#[test]
fn codeable_concept_local_success_produces_no_issue() {
    let cc = cc_with_one_coding("http://terminology.hl7.org/CodeSystem/v3-NullFlavor", "UNK");
    let v = validator();
    let ctx = BindingCheckContextSync::new(
        &v,
        "Patient.maritalStatus",
        "http://hl7.org/fhir/ValueSet/marital-status",
        BindingStrength::Extensible,
        None,
    );
    let issues = validate_codeable_concept_binding(&ctx, Some(&cc), |_| Ok(()));
    assert!(issues.is_empty());
}

#[test]
fn codeable_concept_remote_required_without_service_errors() {
    let cc = cc_with_one_coding("http://example.org/system", "X");
    let v = validator();
    let ctx = BindingCheckContextSync::new(
        &v,
        "Patient.maritalStatus",
        "http://hl7.org/fhir/ValueSet/marital-status",
        BindingStrength::Required,
        None,
    );
    let issues = validate_codeable_concept_binding(&ctx, Some(&cc), |_| {
        Err(TerminologyValidationError::RemoteValidationRequired(
            "remote required".to_string(),
        ))
    });
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, Severity::Error);
    assert_eq!(issues[0].code, "terminology");
}

// --- `CodeableReference` ------------------------------------------------------------------------

#[test]
fn codeable_reference_absent_produces_no_issue() {
    let v = validator();
    let ctx = BindingCheckContextSync::new(
        &v,
        "ServiceRequest.code",
        "http://hl7.org/fhir/ValueSet/procedure-code",
        BindingStrength::Extensible,
        None,
    );
    let issues = validate_codeable_reference_binding(&ctx, None, |_| Ok(()));
    assert!(issues.is_empty());
}

#[test]
fn codeable_reference_concept_local_success_produces_no_issue() {
    let cr = CodeableReference {
        concept: Some(cc_with_one_coding(
            "http://terminology.hl7.org/CodeSystem/v3-NullFlavor",
            "UNK",
        )),
        ..Default::default()
    };
    let v = validator();
    let ctx = BindingCheckContextSync::new(
        &v,
        "ServiceRequest.code",
        "http://hl7.org/fhir/ValueSet/marital-status",
        BindingStrength::Extensible,
        None,
    );
    let issues = validate_codeable_reference_binding(&ctx, Some(&cr), |_| Ok(()));
    assert!(issues.is_empty());
}
