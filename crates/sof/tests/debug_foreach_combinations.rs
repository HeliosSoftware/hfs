use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

#[test]
fn debug_foreach_combinations() {
    // Create test data matching the failing forEach test
    let patient_json = serde_json::json!({
        "resourceType": "Patient",
        "id": "pt1",
        "name": [
            {"family": "F1.1"},
            {"family": "F1.2"}
        ],
        "contact": [
            {"name": {"family": "FC1.1"}},
            {"name": {"family": "FC1.2"}}
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
                "forEach": "contact",
                "column": [
                    {
                        "name": "cont_family",
                        "path": "name.family",
                        "type": "string"
                    }
                ]
            },
            {
                "forEach": "name",
                "column": [
                    {
                        "name": "pat_family",
                        "path": "family",
                        "type": "string"
                    }
                ]
            }
        ]
    });

    let view_definition: helios_fhir::r4::ViewDefinition =
        serde_json::from_value(view_definition_json).unwrap();
    let view_definition = SofViewDefinition::R4(view_definition);

    println!("=== Running forEach combination debug test ===");

    let result = run_view_definition(view_definition, bundle, ContentType::Json).unwrap();
    let result_str = String::from_utf8(result).unwrap();

    println!("Result: {}", result_str);

    // Parse and analyze the result
    let result_rows: Vec<serde_json::Value> = serde_json::from_str(&result_str).unwrap();

    println!("\nAnalysis:");
    println!("Number of rows: {}", result_rows.len());

    for (i, row) in result_rows.iter().enumerate() {
        println!("Row {}: {:?}", i + 1, row);
    }

    println!("\nExpected result:");
    println!("Row 1: {{\"cont_family\": \"FC1.1\", \"pat_family\": \"F1.1\"}}");
    println!("Row 2: {{\"cont_family\": \"FC1.2\", \"pat_family\": \"F1.1\"}}");
    println!("Row 3: {{\"cont_family\": \"FC1.1\", \"pat_family\": \"F1.2\"}}");
    println!("Row 4: {{\"cont_family\": \"FC1.2\", \"pat_family\": \"F1.2\"}}");
}
