use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

#[test]
fn debug_column_ordering() {
    // Create test data matching the column ordering test
    let patient_json = serde_json::json!({
        "resourceType": "Patient",
        "id": "pt1",
        "name": [
            {"family": "TestFamily"}
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

    // Create ViewDefinition exactly like the failing test
    let view_definition_json = serde_json::json!({
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "status": "active",
        "select": [
            {
                "column": [
                    {
                        "path": "'A'",
                        "name": "a",
                        "type": "string"
                    },
                    {
                        "path": "'B'",
                        "name": "b",
                        "type": "string"
                    }
                ],
                "select": [
                    {
                        "forEach": "name",
                        "column": [
                            {
                                "path": "'C'",
                                "name": "c",
                                "type": "string"
                            },
                            {
                                "path": "'D'",
                                "name": "d",
                                "type": "string"
                            }
                        ]
                    }
                ],
                "unionAll": [
                    {
                        "column": [
                            {
                                "path": "'E1'",
                                "name": "e",
                                "type": "string"
                            },
                            {
                                "path": "'F1'",
                                "name": "f",
                                "type": "string"
                            }
                        ]
                    },
                    {
                        "column": [
                            {
                                "path": "'E2'",
                                "name": "e",
                                "type": "string"
                            },
                            {
                                "path": "'F2'",
                                "name": "f",
                                "type": "string"
                            }
                        ]
                    }
                ]
            },
            {
                "column": [
                    {
                        "path": "'G'",
                        "name": "g",
                        "type": "string"
                    },
                    {
                        "path": "'H'",
                        "name": "h",
                        "type": "string"
                    }
                ]
            }
        ]
    });

    let view_definition: helios_fhir::r4::ViewDefinition =
        serde_json::from_value(view_definition_json).unwrap();
    let view_definition = SofViewDefinition::R4(view_definition);

    println!("=== Testing column ordering ===");

    let result = run_view_definition(view_definition, bundle, ContentType::Json).unwrap();
    let result_str = String::from_utf8(result).unwrap();

    println!("Result: {}", result_str);

    let result_rows: Vec<serde_json::Value> = serde_json::from_str(&result_str).unwrap();

    println!("\nActual result:");
    for (i, row) in result_rows.iter().enumerate() {
        println!("Row {}: {:?}", i + 1, row);
    }

    println!("\nExpected result (first row):");
    println!(
        "{{\"a\": \"A\", \"b\": \"B\", \"c\": \"C\", \"d\": \"D\", \"e\": \"E1\", \"f\": \"F1\", \"g\": \"G\", \"h\": \"H\"}}"
    );
    println!(
        "{{\"a\": \"A\", \"b\": \"B\", \"c\": \"C\", \"d\": \"D\", \"e\": \"E2\", \"f\": \"F2\", \"g\": \"G\", \"h\": \"H\"}}"
    );
}
