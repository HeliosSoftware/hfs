use sof::*;
use fhir::r4::*;
use std::collections::HashMap;

fn main() {
    // Test ViewDefinition with base64Binary constant
    let view_def_json = r#"{
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Device",
        "constant": [
            {
                "name": "aidc",
                "valueBase64Binary": "aGVsbG8K"
            }
        ],
        "select": [
            {
                "column": [
                    {
                        "name": "id",
                        "path": "id",
                        "type": "id"
                    }
                ]
            }
        ]
    }"#;

    // Test Bundle with simple Device resource
    let bundle_json = r#"{
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [
            {
                "resource": {
                    "resourceType": "Device",
                    "id": "d1"
                }
            }
        ]
    }"#;

    // Parse ViewDefinition
    let view_def: ViewDefinition = match serde_json::from_str(view_def_json) {
        Ok(vd) => vd,
        Err(e) => {
            println!("Failed to parse ViewDefinition: {}", e);
            return;
        }
    };

    // Parse Bundle
    let bundle: Bundle = match serde_json::from_str(bundle_json) {
        Ok(b) => b,
        Err(e) => {
            println!("Failed to parse Bundle: {}", e);
            return;
        }
    };

    // Test our fix
    match run_view_definition(view_def, bundle, ContentType::Json) {
        Ok(result) => {
            println!("SUCCESS: ViewDefinition with base64Binary constant executed successfully!");
            println!("Result: {}", String::from_utf8_lossy(&result));
        }
        Err(e) => {
            println!("FAILED: {}", e);
        }
    }
}