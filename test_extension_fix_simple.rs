use sof::{run_view_definition, SofViewDefinition, SofBundle, ContentType};
use serde_json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create test data matching exactly the SQL-on-FHIR extension test
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
    
    // Create bundle with the patient
    let bundle_json = serde_json::json!({
        "resourceType": "Bundle",
        "id": "test-bundle",
        "type": "collection",
        "entry": [
            {
                "resource": patient_json
            }
        ]
    });
    
    // Parse bundle
    let bundle: SofBundle = match serde_json::from_value(bundle_json)? {
        sof::SofBundle::R4(bundle) => SofBundle::R4(bundle),
        #[allow(unreachable_patterns)]
        other => panic!("Unexpected bundle version: {:?}", other),
    };
    
    // Create ViewDefinition matching the failing test
    let view_definition_json = serde_json::json!({
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "select": [
            {
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
            }
        ]
    });
    
    let view_definition: SofViewDefinition = match serde_json::from_value(view_definition_json)? {
        sof::SofViewDefinition::R4(vd) => SofViewDefinition::R4(vd),
        #[allow(unreachable_patterns)]
        other => panic!("Unexpected view definition version: {:?}", other),
    };
    
    println!("Running SQL-on-FHIR extension test...");
    
    // Run the view definition
    let result = run_view_definition(&view_definition, &bundle, ContentType::Json)?;
    
    println!("Result: {}", result);
    
    // Check if the result contains "F" instead of null
    if result.contains("\"birthsex\": \"F\"") {
        println!("✅ SUCCESS! Extension function is working correctly!");
        println!("The fix for choice element type preservation and extension function is complete.");
    } else if result.contains("\"birthsex\": null") {
        println!("❌ Extension function still returning null - needs further investigation");
    } else {
        println!("⚠️  Unexpected result format");
    }
    
    Ok(())
}