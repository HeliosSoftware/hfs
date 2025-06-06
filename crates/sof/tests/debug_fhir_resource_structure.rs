use sof::{run_view_definition, SofViewDefinition, SofBundle, ContentType};
use fhirpath::{evaluate_expression, EvaluationContext};
use fhir::FhirResource;
use serde_json;

fn create_test_patient_with_extension() -> fhir::r4::Patient {
    let patient_json = serde_json::json!({
        "resourceType": "Patient",
        "id": "pt1",
        "extension": [
            {
                "id": "birthsex",
                "url": "http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex",
                "valueCode": "F"
            }
        ]
    });
    
    serde_json::from_value(patient_json).expect("Failed to parse patient")
}

#[test]
fn test_fhir_resource_structure_debug() {
    // Create a patient with extension
    let patient = create_test_patient_with_extension();
    
    println!("Original Patient structure:");
    println!("{}", serde_json::to_string_pretty(&patient).unwrap());
    
    // Test 1: Convert to FhirResource and back to JSON to see what changes
    let fhir_resource = FhirResource::R4(Box::new(fhir::r4::Resource::Patient(patient.clone())));
    
    // Serialize the FhirResource back to JSON to see if extensions are preserved
    if let FhirResource::R4(boxed_resource) = &fhir_resource {
        if let fhir::r4::Resource::Patient(p) = boxed_resource.as_ref() {
            println!("\nAfter FhirResource conversion:");
            println!("{}", serde_json::to_string_pretty(p).unwrap());
        }
    }
    
    // Test 2: Create evaluation context and test direct paths
    let context = EvaluationContext::new(vec![fhir_resource]);
    
    // Test basic navigation
    println!("\n=== Testing FHIRPath Navigation ===");
    
    let resource_type_result = evaluate_expression("Patient.resourceType", &context);
    println!("Patient.resourceType: {:?}", resource_type_result);
    
    let id_result = evaluate_expression("Patient.id", &context);
    println!("Patient.id: {:?}", id_result);
    
    // Test extension field access
    let extension_field_result = evaluate_expression("Patient.extension", &context);
    println!("Patient.extension: {:?}", extension_field_result);
    
    // Test extension function
    let extension_func_result = evaluate_expression("Patient.extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex')", &context);
    println!("Patient.extension('...'): {:?}", extension_func_result);
    
    // Test value access
    let extension_value_result = evaluate_expression("Patient.extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').value", &context);
    println!("Patient.extension('...').value: {:?}", extension_value_result);
    
    // Test full path
    let full_path_result = evaluate_expression("Patient.extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').value.ofType(code).first()", &context);
    println!("Patient.extension('...').value.ofType(code).first(): {:?}", full_path_result);
    
    // Test 3: Try direct Bundle approach like in SQL-on-FHIR
    println!("\n=== Testing Bundle Approach ===");
    let bundle_json = serde_json::json!({
        "resourceType": "Bundle",
        "id": "test-bundle",
        "type": "collection",
        "entry": [
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "pt1",
                    "extension": [
                        {
                            "id": "birthsex",
                            "url": "http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex",
                            "valueCode": "F"
                        }
                    ]
                }
            }
        ]
    });
    
    let bundle: fhir::r4::Bundle = serde_json::from_value(bundle_json).expect("Failed to parse bundle");
    let sof_bundle = SofBundle::R4(bundle);
    
    // Test the same path through SQL-on-FHIR
    let view = serde_json::json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [
                {
                    "path": "id",
                    "name": "id",
                    "type": "id"
                },
                {
                    "name": "extension_debug",
                    "path": "extension",
                    "type": "string"
                },
                {
                    "name": "birthsex",
                    "path": "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').value.ofType(code).first()",
                    "type": "code"
                }
            ]
        }]
    });
    
    let view_definition: fhir::r4::ViewDefinition = serde_json::from_value(view).expect("Failed to parse ViewDefinition");
    let sof_view = SofViewDefinition::R4(view_definition);
    
    let result = run_view_definition(sof_view, sof_bundle, ContentType::Json)
        .expect("Failed to run ViewDefinition");
    
    let actual_rows: Vec<serde_json::Value> = serde_json::from_slice(&result)
        .expect("Failed to parse result as JSON");
    
    println!("SQL-on-FHIR result: {:?}", actual_rows);
}