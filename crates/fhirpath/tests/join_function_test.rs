use helios_fhir::FhirResource;
use helios_fhirpath::{EvaluationContext, EvaluationResult, evaluate_expression};

#[test]
fn test_join_function_basic() {
    // Test basic join functionality
    let patient_json = serde_json::json!({
        "resourceType": "Patient",
        "id": "p1",
        "name": [{
            "given": ["John", "James"]
        }]
    });

    let patient: helios_fhir::r4::Patient = serde_json::from_value(patient_json).unwrap();
    let fhir_resource = FhirResource::R4(Box::new(helios_fhir::r4::Resource::Patient(Box::new(
        patient,
    ))));
    let context = EvaluationContext::new(vec![fhir_resource]);

    // Test joining given names with comma
    let result = evaluate_expression("name.given.join(',')", &context).unwrap();

    match result {
        EvaluationResult::String(s, _) => {
            assert_eq!(s, "John,James");
        }
        _ => panic!("Expected string result, got: {:?}", result),
    }
}

#[test]
fn test_join_function_with_space() {
    // Test join with space separator
    let patient_json = serde_json::json!({
        "resourceType": "Patient",
        "id": "p1",
        "name": [{
            "given": ["John", "James"]
        }]
    });

    let patient: helios_fhir::r4::Patient = serde_json::from_value(patient_json).unwrap();
    let fhir_resource = FhirResource::R4(Box::new(helios_fhir::r4::Resource::Patient(Box::new(
        patient,
    ))));
    let context = EvaluationContext::new(vec![fhir_resource]);

    // Test joining given names with space
    let result = evaluate_expression("name.given.join(' ')", &context).unwrap();

    match result {
        EvaluationResult::String(s, _) => {
            assert_eq!(s, "John James");
        }
        _ => panic!("Expected string result, got: {:?}", result),
    }
}

#[test]
fn test_join_function_empty_separator() {
    // Test join with empty separator
    let patient_json = serde_json::json!({
        "resourceType": "Patient",
        "id": "p1",
        "name": [{
            "given": ["John", "James"]
        }]
    });

    let patient: helios_fhir::r4::Patient = serde_json::from_value(patient_json).unwrap();
    let fhir_resource = FhirResource::R4(Box::new(helios_fhir::r4::Resource::Patient(Box::new(
        patient,
    ))));
    let context = EvaluationContext::new(vec![fhir_resource]);

    // Test joining given names with no separator
    let result = evaluate_expression("name.given.join('')", &context).unwrap();

    match result {
        EvaluationResult::String(s, _) => {
            assert_eq!(s, "JohnJames");
        }
        _ => panic!("Expected string result, got: {:?}", result),
    }
}

#[test]
fn test_join_function_empty_collection() {
    // Test join with empty collection
    let patient_json = serde_json::json!({
        "resourceType": "Patient",
        "id": "p1"
        // No name field
    });

    let patient: helios_fhir::r4::Patient = serde_json::from_value(patient_json).unwrap();
    let fhir_resource = FhirResource::R4(Box::new(helios_fhir::r4::Resource::Patient(Box::new(
        patient,
    ))));
    let context = EvaluationContext::new(vec![fhir_resource]);

    // Test joining non-existent given names
    let result = evaluate_expression("name.given.join(',')", &context).unwrap();

    match result {
        EvaluationResult::String(s, _) => {
            assert_eq!(s, ""); // Empty collection should produce empty string
        }
        _ => panic!("Expected string result, got: {:?}", result),
    }
}

#[test]
fn test_join_function_no_separator() {
    // Test join with no separator (should default to empty string)
    let patient_json = serde_json::json!({
        "resourceType": "Patient",
        "id": "p1",
        "name": [{
            "given": ["John", "James"]
        }]
    });

    let patient: helios_fhir::r4::Patient = serde_json::from_value(patient_json).unwrap();
    let fhir_resource = FhirResource::R4(Box::new(helios_fhir::r4::Resource::Patient(Box::new(
        patient,
    ))));
    let context = EvaluationContext::new(vec![fhir_resource]);

    // Test joining given names with no separator (should default to empty separator)
    let result = evaluate_expression("name.given.join()", &context).unwrap();

    match result {
        EvaluationResult::String(s, _) => {
            assert_eq!(s, "JohnJames"); // Should join with no separator
        }
        _ => panic!("Expected string result, got: {:?}", result),
    }
}
