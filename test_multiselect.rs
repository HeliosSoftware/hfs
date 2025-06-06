use sof::{run_view_definition, SofViewDefinition, SofBundle, ContentType};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Test the "two selects with columns" case that was failing
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
        })
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
    
    let bundle: fhir::r4::Bundle = serde_json::from_value(bundle_json)?;
    let sof_bundle = SofBundle::R4(bundle);
    
    // Create ViewDefinition with two select clauses
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
    
    let view_definition: fhir::r4::ViewDefinition = serde_json::from_value(view)?;
    let sof_view = SofViewDefinition::R4(view_definition);
    
    // Run the view definition
    let result = run_view_definition(sof_view, sof_bundle, ContentType::Json)?;
    
    // Parse and display result
    let actual_rows: Vec<serde_json::Value> = serde_json::from_slice(&result)?;
    
    println!("Multi-select test result:");
    println!("{}", serde_json::to_string_pretty(&actual_rows)?);
    
    // Expected result
    let expected = vec![
        serde_json::json!({"id": "pt1", "last_name": "F1"}),
        serde_json::json!({"id": "pt2", "last_name": "F2"}),
        serde_json::json!({"id": "pt3", "last_name": null})
    ];
    
    println!("\nExpected result:");
    println!("{}", serde_json::to_string_pretty(&expected)?);
    
    // Check if they match
    let matches = actual_rows.len() == expected.len() && 
        actual_rows.iter().zip(expected.iter()).all(|(a, e)| {
            if let (Some(a_obj), Some(e_obj)) = (a.as_object(), e.as_object()) {
                e_obj.iter().all(|(key, expected_val)| {
                    match a_obj.get(key) {
                        Some(actual_val) => actual_val == expected_val,
                        None => expected_val.is_null()
                    }
                })
            } else {
                a == e
            }
        });
    
    if matches {
        println!("\n✅ Multi-select test PASSED!");
    } else {
        println!("\n❌ Multi-select test FAILED!");
    }
    
    Ok(())
}