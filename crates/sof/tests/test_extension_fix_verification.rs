use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

#[test]
fn test_extension_fix_verification() {
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
    let bundle: helios_fhir::r4::Bundle = serde_json::from_value(bundle_json).unwrap();
    let bundle = SofBundle::R4(bundle);

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

    let view_definition: helios_fhir::r4::ViewDefinition =
        serde_json::from_value(view_definition_json).unwrap();
    let view_definition = SofViewDefinition::R4(view_definition);

    println!("Running SQL-on-FHIR extension test...");

    // Run the view definition
    let result = run_view_definition(view_definition, bundle, ContentType::Json).unwrap();

    let result_str = String::from_utf8(result).unwrap();
    println!("Result: {}", result_str);

    // Check if the result contains "F" instead of null
    if result_str.contains("\"birthsex\": \"F\"") {
        println!("✅ SUCCESS! Extension function is working correctly!");
        println!(
            "The fix for choice element type preservation and extension function is complete."
        );
    } else if result_str.contains("\"birthsex\": null") {
        println!("❌ Extension function still returning null - needs further investigation");
        panic!("Extension function fix failed - still returning null instead of 'F'");
    } else {
        println!("⚠️  Unexpected result format: {}", result_str);
        panic!("Unexpected result format from extension test");
    }
}
