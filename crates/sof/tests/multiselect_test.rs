use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

#[test]
fn test_two_selects_with_columns() {
    // Test the "two selects with columns" case that was previously failing
    let resources = vec![
        serde_json::json!({
            "resourceType": "Patient",
            "id": "pt1",
            "name": [{"family": "F1"}],
            "active": true
        }),
        serde_json::json!({
            "resourceType": "Patient",
            "id": "pt2",
            "name": [{"family": "F2"}],
            "active": false
        }),
        serde_json::json!({
            "resourceType": "Patient",
            "id": "pt3"
        }),
    ];

    // Create bundle
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

    let bundle: helios_fhir::r4::Bundle =
        serde_json::from_value(bundle_json).expect("Failed to create bundle");
    let sof_bundle = SofBundle::R4(bundle);

    // Create ViewDefinition with two select clauses (this was failing before)
    let view = serde_json::json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
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
                "column": [
                    {
                        "name": "last_name",
                        "path": "name.family.first()",
                        "type": "string"
                    }
                ]
            }
        ]
    });

    let view_definition: helios_fhir::r4::ViewDefinition =
        serde_json::from_value(view).expect("Failed to create ViewDefinition");
    let sof_view = SofViewDefinition::R4(view_definition);

    // Run the view definition
    let result = run_view_definition(sof_view, sof_bundle, ContentType::Json)
        .expect("Failed to run ViewDefinition");

    // Parse result
    let actual_rows: Vec<serde_json::Value> =
        serde_json::from_slice(&result).expect("Failed to parse result");

    println!("Multi-select test result:");
    println!("{}", serde_json::to_string_pretty(&actual_rows).unwrap());

    // Expected result - each row should have BOTH id and last_name columns
    let expected = vec![
        serde_json::json!({"id": "pt1", "last_name": "F1"}),
        serde_json::json!({"id": "pt2", "last_name": "F2"}),
        serde_json::json!({"id": "pt3", "last_name": null}),
    ];

    println!("\nExpected result:");
    println!("{}", serde_json::to_string_pretty(&expected).unwrap());

    // Verify we have the correct number of rows
    assert_eq!(actual_rows.len(), expected.len(), "Row count mismatch");

    // Check that each row has both columns
    for (i, actual_row) in actual_rows.iter().enumerate() {
        let expected_row = &expected[i];

        if let (Some(actual_obj), Some(expected_obj)) =
            (actual_row.as_object(), expected_row.as_object())
        {
            // Check that both id and last_name columns are present
            assert!(
                actual_obj.contains_key("id"),
                "Row {} missing 'id' column",
                i
            );
            assert!(
                actual_obj.contains_key("last_name"),
                "Row {} missing 'last_name' column",
                i
            );

            // Check that values match
            for (key, expected_val) in expected_obj {
                match actual_obj.get(key) {
                    Some(actual_val) => {
                        assert_eq!(
                            actual_val, expected_val,
                            "Row {} column '{}' value mismatch",
                            i, key
                        );
                    }
                    None => {
                        if !expected_val.is_null() {
                            panic!("Row {} missing expected column '{}'", i, key);
                        }
                    }
                }
            }
        } else {
            panic!("Row {} is not an object", i);
        }
    }

    println!("\n✅ Multi-select test PASSED! Both columns are properly combined in each row.");
}
