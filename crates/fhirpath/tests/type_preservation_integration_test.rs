use helios_fhir::FhirResource;
use helios_fhirpath::{EvaluationContext, evaluate_expression};
use serde_json::json;

#[test]
fn test_uri_type_preserved_in_evaluation() {
    // Create a Patient resource with identifiers containing a system field of type Uri
    let patient_json = json!({
        "resourceType": "Patient",
        "id": "example",
        "identifier": [
            {
                "type": {
                    "coding": [
                        {
                            "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                            "code": "MR"
                        }
                    ]
                }
            }
        ]
    });

    // Parse the JSON into a FHIR resource
    let resource: helios_fhir::r4::Resource = serde_json::from_value(patient_json).unwrap();
    let fhir_resource = FhirResource::R4(Box::new(resource));

    // Create evaluation context
    let context = EvaluationContext::new(vec![fhir_resource]);

    // Get the system field
    let result = evaluate_expression("identifier[0].type.coding[0].system", &context).unwrap();

    // Verify it has the correct type information
    if let helios_fhirpath_support::EvaluationResult::String(value, type_info) = result {
        assert_eq!(value, "http://terminology.hl7.org/CodeSystem/v2-0203");

        // Check that type info is preserved
        assert!(type_info.is_some(), "Type information should be present");
        let info = type_info.unwrap();
        assert_eq!(info.namespace, "FHIR");
        assert_eq!(info.name, "uri");
    } else {
        panic!("Expected String result");
    }
}

#[test]
fn test_code_type_preserved_in_evaluation() {
    // Create a Patient resource with identifiers containing a code field
    let patient_json = json!({
        "resourceType": "Patient",
        "id": "example",
        "identifier": [
            {
                "type": {
                    "coding": [
                        {
                            "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                            "code": "MR"
                        }
                    ]
                }
            }
        ]
    });

    // Parse the JSON into a FHIR resource
    let resource: helios_fhir::r4::Resource = serde_json::from_value(patient_json).unwrap();
    let fhir_resource = FhirResource::R4(Box::new(resource));

    // Create evaluation context
    let context = EvaluationContext::new(vec![fhir_resource]);

    // Get the code field
    let result = evaluate_expression("identifier[0].type.coding[0].code", &context).unwrap();

    // Verify it has the correct type information
    if let helios_fhirpath_support::EvaluationResult::String(value, type_info) = result {
        assert_eq!(value, "MR");

        // Check that type info is preserved
        assert!(type_info.is_some(), "Type information should be present");
        let info = type_info.unwrap();
        assert_eq!(info.namespace, "FHIR");
        assert_eq!(info.name, "code");
    } else {
        panic!("Expected String result");
    }
}
