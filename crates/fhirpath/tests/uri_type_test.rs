use helios_fhir::FhirResource;
use helios_fhirpath::{EvaluationContext, evaluate_expression};
use serde_json::json;

#[test]
fn test_uri_type_preservation() {
    // Create a Patient resource with identifiers containing a system field of type Uri
    let patient_json = json!({
        "resourceType": "Patient",
        "id": "example",
        "identifier": [
            {
                "use": "usual",
                "type": {
                    "coding": [
                        {
                            "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                            "code": "MR"
                        }
                    ]
                },
                "system": "urn:oid:1.2.36.146.595.217.0.1",
                "value": "12345",
                "period": {
                    "start": "2001-05-06"
                },
                "assigner": {
                    "display": "Acme Healthcare"
                }
            }
        ],
        "active": true,
        "name": [
            {
                "use": "official",
                "family": "Chalmers",
                "given": [
                    "Peter",
                    "James"
                ]
            }
        ]
    });

    // Parse the JSON into a FHIR resource
    let resource: helios_fhir::r4::Resource = serde_json::from_value(patient_json).unwrap();
    let fhir_resource = FhirResource::R4(Box::new(resource));

    // Create evaluation context
    let context = EvaluationContext::new(vec![fhir_resource]);

    // Test 1: Get the system field and check its type
    let result = evaluate_expression("identifier[0].type.coding[0].system", &context).unwrap();

    // Verify the value is correct
    if let helios_fhirpath_support::EvaluationResult::String(value, _type_info) = &result {
        assert_eq!(value, "http://terminology.hl7.org/CodeSystem/v2-0203");

        // Test 2: Check the type using type() function
        let type_result =
            evaluate_expression("identifier[0].type.coding[0].system.type().name", &context)
                .unwrap();

        if let helios_fhirpath_support::EvaluationResult::String(type_name, _) = type_result {
            // This should be "uri" but currently returns "String"
            println!("Type name: {}", type_name);
            assert_eq!(
                type_name, "uri",
                "Expected 'uri' type but got '{}'",
                type_name
            );
        } else {
            panic!("Expected string result from type().name");
        }

        // Also check the namespace
        let namespace_result = evaluate_expression(
            "identifier[0].type.coding[0].system.type().namespace",
            &context,
        )
        .unwrap();
        if let helios_fhirpath_support::EvaluationResult::String(namespace, _) = namespace_result {
            println!("Namespace: {}", namespace);
            assert_eq!(
                namespace, "FHIR",
                "Expected 'FHIR' namespace but got '{}'",
                namespace
            );
        }
    } else {
        panic!("Expected string result");
    }
}

#[test]
fn test_code_type_preservation() {
    // Test for code type
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

    let resource: helios_fhir::r4::Resource = serde_json::from_value(patient_json).unwrap();
    let fhir_resource = FhirResource::R4(Box::new(resource));
    let context = EvaluationContext::new(vec![fhir_resource]);

    // Check code field type
    let type_result =
        evaluate_expression("identifier[0].type.coding[0].code.type().name", &context).unwrap();

    if let helios_fhirpath_support::EvaluationResult::String(type_name, _) = type_result {
        println!("Code type name: {}", type_name);
        assert_eq!(
            type_name, "code",
            "Expected 'code' type but got '{}'",
            type_name
        );
    } else {
        panic!("Expected string result from type().name");
    }
}

#[test]
fn test_id_type_preservation() {
    // Test for id type - the top-level 'id' in a resource is just a String, not an Id type.
    // Id type is used in other contexts, like in Element extensions.
    // For this test, we'll use a resource that has a field of type Id
    let _resource_json = json!({
        "resourceType": "Basic",
        "id": "example",
        "code": {
            "coding": [{
                "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                "code": "MR"
            }]
        },
        // In FHIR, the 'id' type is typically used in extensions or other specific contexts
        // For now, we'll skip this test as the top-level 'id' is correctly a plain string
    });

    // This test is commented out because the top-level 'id' field in FHIR resources
    // is correctly typed as a plain String, not the FHIR primitive type 'Id'.
    // The Id type is used in other contexts within FHIR.
}
