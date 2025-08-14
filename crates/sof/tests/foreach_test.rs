use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

fn create_test_bundle(
    resources: &[serde_json::Value],
) -> Result<SofBundle, Box<dyn std::error::Error>> {
    // Create a Bundle with the test resources
    let mut bundle_json = serde_json::json!({
        "resourceType": "Bundle",
        "id": "test-bundle",
        "type": "collection",
        "entry": []
    });

    if let Some(entry_array) = bundle_json["entry"].as_array_mut() {
        for resource in resources {
            entry_array.push(serde_json::json!({
                "resource": resource
            }));
        }
    }

    // Parse as R4 Bundle
    let bundle: helios_fhir::r4::Bundle = serde_json::from_value(bundle_json)?;
    Ok(SofBundle::R4(bundle))
}

fn parse_view_definition(
    view_json: &serde_json::Value,
) -> Result<SofViewDefinition, Box<dyn std::error::Error>> {
    // Add resourceType to make it a valid ViewDefinition resource
    let mut view_def = view_json.clone();
    if let Some(obj) = view_def.as_object_mut() {
        obj.insert(
            "resourceType".to_string(),
            serde_json::Value::String("ViewDefinition".to_string()),
        );
        obj.insert(
            "status".to_string(),
            serde_json::Value::String("active".to_string()),
        );
    }

    let view_definition: helios_fhir::r4::ViewDefinition = serde_json::from_value(view_def)?;
    Ok(SofViewDefinition::R4(view_definition))
}

#[test]
fn test_simple_foreach() {
    // Test data similar to foreach.json
    let resources = vec![
        serde_json::json!({
            "resourceType": "Patient",
            "id": "pt1",
            "name": [
                {
                    "family": "F1.1"
                },
                {
                    "family": "F1.2"
                }
            ]
        }),
        serde_json::json!({
            "resourceType": "Patient",
            "id": "pt2",
            "name": [
                {
                    "family": "F2.1"
                },
                {
                    "family": "F2.2"
                }
            ]
        }),
    ];

    let bundle = create_test_bundle(&resources).expect("Failed to create test bundle");

    // View definition with forEach
    let view = serde_json::json!({
        "resource": "Patient",
        "status": "active",
        "select": [
            {
                "column": [
                    {
                        "name": "id",
                        "path": "id",
                        "type": "id"
                    }
                ]
            },
            {
                "forEach": "name",
                "column": [
                    {
                        "name": "family",
                        "path": "family",
                        "type": "string"
                    }
                ]
            }
        ]
    });

    let view_definition = parse_view_definition(&view).expect("Failed to parse ViewDefinition");

    let result = run_view_definition(view_definition, bundle, ContentType::Json)
        .expect("Failed to run ViewDefinition");

    let actual_rows: Vec<serde_json::Value> =
        serde_json::from_slice(&result).expect("Failed to parse result as JSON");

    println!("Actual result: {:#?}", actual_rows);

    // Expected: 4 rows (2 per patient, one for each name)
    let expected = [
        serde_json::json!({"id": "pt1", "family": "F1.1"}),
        serde_json::json!({"id": "pt1", "family": "F1.2"}),
        serde_json::json!({"id": "pt2", "family": "F2.1"}),
        serde_json::json!({"id": "pt2", "family": "F2.2"}),
    ];

    assert_eq!(
        actual_rows.len(),
        expected.len(),
        "Row count mismatch. Expected: {}, Got: {}",
        expected.len(),
        actual_rows.len()
    );
}
