use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

#[test]
fn debug_reference_key_wrong_type() {
    // Create test data matching the reference key test
    let patient_json = serde_json::json!({
        "resourceType": "Patient",
        "id": "p1",
        "link": [
            {
                "other": {
                    "reference": "Patient/p1"
                }
            }
        ]
    });

    // Create bundle
    let bundle_json = serde_json::json!({
        "resourceType": "Bundle",
        "id": "test-bundle",
        "type": "collection",
        "entry": [{"resource": patient_json}]
    });

    let bundle: helios_fhir::r4::Bundle = serde_json::from_value(bundle_json).unwrap();
    let bundle = SofBundle::R4(bundle);

    // Create ViewDefinition for the failing test case
    let view_definition_json = serde_json::json!({
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "status": "active",
        "select": [
            {
                "column": [
                    {
                        "path": "getResourceKey() = link.other.getReferenceKey(Observation)",
                        "name": "key_equal_ref",
                        "type": "boolean"
                    }
                ]
            }
        ]
    });

    let view_definition: helios_fhir::r4::ViewDefinition =
        serde_json::from_value(view_definition_json).unwrap();
    let view_definition = SofViewDefinition::R4(view_definition);

    println!("=== Testing getReferenceKey with wrong type specifier ===");

    let result = run_view_definition(view_definition, bundle.clone(), ContentType::Json).unwrap();
    let result_str = String::from_utf8(result).unwrap();

    println!("Result: {}", result_str);

    let result_rows: Vec<serde_json::Value> = serde_json::from_str(&result_str).unwrap();

    println!("\nActual result:");
    for (i, row) in result_rows.iter().enumerate() {
        println!("Row {}: {:?}", i + 1, row);
    }

    println!("\nExpected result:");
    println!("Row 1: {{\"key_equal_ref\": null}}");

    // Let's also test the individual function results
    println!("\n=== Testing individual function calls ===");

    let test_resource_key = serde_json::json!({
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "status": "active",
        "select": [
            {
                "column": [
                    {
                        "path": "getResourceKey()",
                        "name": "resource_key",
                        "type": "string"
                    }
                ]
            }
        ]
    });

    let test_ref_key_wrong_type = serde_json::json!({
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "status": "active",
        "select": [
            {
                "column": [
                    {
                        "path": "link.other.getReferenceKey(Observation)",
                        "name": "ref_key",
                        "type": "string"
                    }
                ]
            }
        ]
    });

    let test_ref_key_correct_type = serde_json::json!({
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "status": "active",
        "select": [
            {
                "column": [
                    {
                        "path": "link.other.getReferenceKey(Patient)",
                        "name": "ref_key",
                        "type": "string"
                    }
                ]
            }
        ]
    });

    // Test getResourceKey()
    let vd1: helios_fhir::r4::ViewDefinition = serde_json::from_value(test_resource_key).unwrap();
    let result1 = run_view_definition(
        SofViewDefinition::R4(vd1),
        bundle.clone(),
        ContentType::Json,
    )
    .unwrap();
    let result1_str = String::from_utf8(result1).unwrap();
    println!("getResourceKey(): {}", result1_str);

    // Test getReferenceKey(Observation) - wrong type
    let vd2: helios_fhir::r4::ViewDefinition =
        serde_json::from_value(test_ref_key_wrong_type).unwrap();
    let result2 = run_view_definition(
        SofViewDefinition::R4(vd2),
        bundle.clone(),
        ContentType::Json,
    )
    .unwrap();
    let result2_str = String::from_utf8(result2).unwrap();
    println!("getReferenceKey(Observation): {}", result2_str);

    // Test getReferenceKey(Patient) - correct type
    let vd3: helios_fhir::r4::ViewDefinition =
        serde_json::from_value(test_ref_key_correct_type).unwrap();
    let result3 = run_view_definition(
        SofViewDefinition::R4(vd3),
        bundle.clone(),
        ContentType::Json,
    )
    .unwrap();
    let result3_str = String::from_utf8(result3).unwrap();
    println!("getReferenceKey(Patient): {}", result3_str);
}
