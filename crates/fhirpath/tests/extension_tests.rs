use helios_fhirpath::{EvaluationContext, evaluate_expression};
use helios_fhirpath_support::EvaluationResult;
use serde_json::{self, json};
use std::collections::HashMap;

// Helper function to parse and evaluate FHIRPath expressions
fn eval(expr: &str, context: &EvaluationContext) -> EvaluationResult {
    evaluate_expression(expr, context).unwrap()
}

// Helper to create a test Patient context with birthTime extension
fn create_patient_context() -> EvaluationContext {
    let mut ctx = EvaluationContext::new_empty_with_default_version();

    // Create a patient resource with a birthDate that has an extension
    let _patient = json!({
        "resourceType": "Patient",
        "id": "example",
        "birthDate": "1974-12-25",
        "_birthDate": {
            "extension": [
                {
                    "url": "http://hl7.org/fhir/StructureDefinition/patient-birthTime",
                    "valueDateTime": "1974-12-25T14:35:45-05:00"
                }
            ]
        }
    });

    // Convert to EvaluationResult
    let mut patient_obj = HashMap::new();

    // Add resourceType
    patient_obj.insert(
        "resourceType".to_string(),
        EvaluationResult::string("Patient".to_string()),
    );

    // Add birthDate
    patient_obj.insert(
        "birthDate".to_string(),
        EvaluationResult::string("1974-12-25".to_string()),
    );

    // Create the extension object
    let mut extension_obj = HashMap::new();
    extension_obj.insert(
        "url".to_string(),
        EvaluationResult::string(
            "http://hl7.org/fhir/StructureDefinition/patient-birthTime".to_string(),
        ),
    );
    extension_obj.insert(
        "valueDateTime".to_string(),
        EvaluationResult::string("1974-12-25T14:35:45-05:00".to_string()),
    );

    // Create _birthDate object
    let mut birthdate_ext_obj = HashMap::new();
    birthdate_ext_obj.insert(
        "extension".to_string(),
        EvaluationResult::Collection {
            items: vec![EvaluationResult::object(extension_obj.clone())],
            has_undefined_order: false,
            type_info: None,
        },
    );

    // Add _birthDate to patient
    patient_obj.insert(
        "_birthDate".to_string(),
        EvaluationResult::object(birthdate_ext_obj),
    );

    // Create a separate underscore_birthdate for testing direct access
    let mut underscore_birthdate = HashMap::new();
    underscore_birthdate.insert(
        "extension".to_string(),
        EvaluationResult::Collection {
            items: vec![EvaluationResult::object(extension_obj)],
            has_undefined_order: false,
            type_info: None,
        },
    );

    // Set as context - both the patient and the _birthDate object for different tests
    ctx.set_this(EvaluationResult::object(patient_obj));
    ctx.set_variable_result("_birthDate", EvaluationResult::object(underscore_birthdate));

    // Add a variable for the extension URL
    ctx.set_variable(
        "ext-patient-birthTime",
        "http://hl7.org/fhir/StructureDefinition/patient-birthTime".to_string(),
    );

    ctx
}

#[test]
fn test_direct_extension_access() {
    let ctx = create_patient_context();

    // Test accessing the extension directly on the _birthDate object from the variable context
    let result = eval("%_birthDate.extension[0].url", &ctx);
    assert_eq!(
        result,
        EvaluationResult::string(
            "http://hl7.org/fhir/StructureDefinition/patient-birthTime".to_string()
        )
    );

    let result = eval("%_birthDate.extension[0].valueDateTime", &ctx);
    assert_eq!(
        result,
        EvaluationResult::string("1974-12-25T14:35:45-05:00".to_string())
    );
}

#[test]
fn test_extension_function_with_hardcoded_url() {
    let ctx = create_patient_context();

    // Test the extension function with a direct URL
    let result = eval(
        "%_birthDate.extension('http://hl7.org/fhir/StructureDefinition/patient-birthTime').exists()",
        &ctx,
    );
    assert_eq!(result, EvaluationResult::boolean(true));

    let result = eval(
        "%_birthDate.extension('http://hl7.org/fhir/StructureDefinition/patient-birthTime').valueDateTime",
        &ctx,
    );
    assert_eq!(
        result,
        EvaluationResult::string("1974-12-25T14:35:45-05:00".to_string())
    );

    // Test with a URL that doesn't exist
    let result = eval(
        "%_birthDate.extension('http://example.org/non-existent').exists()",
        &ctx,
    );
    assert_eq!(result, EvaluationResult::boolean(false));
}

#[test]
fn test_extension_function_with_variable() {
    let ctx = create_patient_context();

    // Test the extension function with a variable reference
    let result = eval(
        "%_birthDate.extension(%`ext-patient-birthTime`).exists()",
        &ctx,
    );
    assert_eq!(result, EvaluationResult::boolean(true));

    let result = eval(
        "%_birthDate.extension(%`ext-patient-birthTime`).valueDateTime",
        &ctx,
    );
    assert_eq!(
        result,
        EvaluationResult::string("1974-12-25T14:35:45-05:00".to_string())
    );
}

// This test is specifically for the behavior we need to implement
// It depends on special handling in the extension_helpers that's not fully implemented
#[test]
fn test_underscore_property_access() {
    let ctx = create_patient_context();

    // Test the special case where we access extensions on birthDate
    // which should translate to _birthDate.extension
    let result = eval(
        "birthDate.extension('http://hl7.org/fhir/StructureDefinition/patient-birthTime').exists()",
        &ctx,
    );
    assert_eq!(result, EvaluationResult::boolean(true));

    let result = eval(
        "birthDate.extension(%`ext-patient-birthTime`).exists()",
        &ctx,
    );
    assert_eq!(result, EvaluationResult::boolean(true));
}
