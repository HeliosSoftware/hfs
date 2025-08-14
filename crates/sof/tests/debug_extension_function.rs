use helios_fhirpath::{EvaluationContext, evaluate_expression};
use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

fn create_test_bundle() -> SofBundle {
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

    let bundle: helios_fhir::r4::Bundle =
        serde_json::from_value(bundle_json).expect("Failed to parse bundle");
    SofBundle::R4(bundle)
}

#[test]
fn test_extension_function_debug() {
    let bundle = create_test_bundle();

    // Test the view definition that should extract extension
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
                    "name": "birthsex",
                    "path": "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').value.ofType(code).first()",
                    "type": "code"
                }
            ]
        }]
    });

    let view_definition: helios_fhir::r4::ViewDefinition =
        serde_json::from_value(view).expect("Failed to parse ViewDefinition");
    let sof_view = SofViewDefinition::R4(view_definition);

    let result = run_view_definition(sof_view, bundle, ContentType::Json)
        .expect("Failed to run ViewDefinition");

    let actual_rows: Vec<serde_json::Value> =
        serde_json::from_slice(&result).expect("Failed to parse result as JSON");

    println!("Result: {:?}", actual_rows);

    // Test direct FHIRPath evaluation
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

    println!("Testing direct FHIRPath evaluation:");
    let patient: helios_fhir::r4::Patient =
        serde_json::from_value(patient_json).expect("Failed to parse patient");
    let resources = vec![helios_fhir::FhirResource::R4(Box::new(
        helios_fhir::r4::Resource::Patient(patient),
    ))];
    let context = EvaluationContext::new(resources);

    // Test basic extension access
    let extension_result = evaluate_expression("Patient.extension", &context);
    println!("Patient.extension: {:?}", extension_result);

    // Test extension function call
    let extension_func_result = evaluate_expression(
        "Patient.extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex')",
        &context,
    );
    println!("Patient.extension('...'): {:?}", extension_func_result);

    // Test full path
    let full_path_result = evaluate_expression(
        "Patient.extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').value.ofType(code).first()",
        &context,
    );
    println!("full path: {:?}", full_path_result);
}
