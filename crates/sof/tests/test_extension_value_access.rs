use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

#[test]
fn test_extension_value_access_patterns() {
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

    let bundle: helios_fhir::r4::Bundle =
        serde_json::from_value(bundle_json).expect("Failed to parse bundle");
    let sof_bundle = SofBundle::R4(bundle);

    // Test different access patterns
    let test_cases = vec![
        // Test 1: Direct extension function
        (
            "extension_func",
            "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex')",
        ),
        // Test 2: Extension function + value access
        (
            "extension_value",
            "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').value",
        ),
        // Test 3: Extension function + valueCode access (what should work)
        (
            "extension_valuecode",
            "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').valueCode",
        ),
        // Test 4: Extension function + value + ofType
        (
            "extension_value_oftype",
            "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').value.ofType(code)",
        ),
        // Test 5: Full path from failing test
        (
            "full_path",
            "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').value.ofType(code).first()",
        ),
        // Test 6: Alternative using valueCode
        (
            "valuecode_direct",
            "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').valueCode",
        ),
    ];

    for (name, path) in test_cases {
        println!("\n=== Testing: {} ===", name);
        println!("Path: {}", path);

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
                        "name": name,
                        "path": path,
                        "type": "code"
                    }
                ]
            }]
        });

        let view_definition: helios_fhir::r4::ViewDefinition =
            serde_json::from_value(view).expect("Failed to parse ViewDefinition");
        let sof_view = SofViewDefinition::R4(view_definition);

        match run_view_definition(sof_view, sof_bundle.clone(), ContentType::Json) {
            Ok(result) => {
                let actual_rows: Vec<serde_json::Value> =
                    serde_json::from_slice(&result).expect("Failed to parse result as JSON");

                println!("Result: {:?}", actual_rows);

                if let Some(row) = actual_rows.first() {
                    if let Some(value) = row.get(name) {
                        if value.is_null() {
                            println!("❌ FAILED: Got null");
                        } else {
                            println!("✅ SUCCESS: Got value: {}", value);
                        }
                    }
                }
            }
            Err(e) => {
                println!("❌ ERROR: {}", e);
            }
        }
    }
}
