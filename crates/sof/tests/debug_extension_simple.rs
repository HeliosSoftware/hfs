use sof::{run_view_definition, SofViewDefinition, SofBundle, ContentType};
use serde_json;

#[test]
fn test_simple_extension_debug() {
    // Create test bundle
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
    
    // Test the simplest extension function call
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
                    "name": "ext_test",
                    "path": "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex')",
                    "type": "string"
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
    
    println!("Result: {:?}", actual_rows);
}