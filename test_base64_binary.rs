use sof::{run_view_definition, SofViewDefinition, SofBundle, ContentType};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Deserialize)]
struct TestCase {
    title: String,
    description: String,
    #[serde(rename = "fhirVersion")]
    fhir_version: Vec<String>,
    resources: Vec<serde_json::Value>,
    tests: Vec<Test>,
}

#[derive(Debug, Deserialize)]
struct Test {
    title: String,
    tags: Option<Vec<String>>,
    view: serde_json::Value,
    expect: Option<Vec<serde_json::Value>>,
    #[serde(rename = "expectError")]
    expect_error: Option<bool>,
}

fn create_test_bundle(resources: &[serde_json::Value]) -> Result<SofBundle, Box<dyn std::error::Error>> {
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
    Ok(SofBundle::R4(bundle))
}

fn parse_view_definition(view_json: &serde_json::Value) -> Result<SofViewDefinition, Box<dyn std::error::Error>> {
    let mut view_def = view_json.clone();
    if let Some(obj) = view_def.as_object_mut() {
        obj.insert("resourceType".to_string(), serde_json::Value::String("ViewDefinition".to_string()));
        obj.insert("status".to_string(), serde_json::Value::String("active".to_string()));
    }
    
    let view_definition: fhir::r4::ViewDefinition = serde_json::from_value(view_def)?;
    Ok(SofViewDefinition::R4(view_definition))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing base64Binary constant type support...");

    // Load test file
    let test_file_path = "/home/slm/git/hfs/tests/sql-on-fhir/sql-on-fhir-v2/tests/constant_types.json";
    let content = fs::read_to_string(test_file_path)?;
    let test_case: TestCase = serde_json::from_str(&content)?;
    
    // Create bundle from resources
    let bundle = create_test_bundle(&test_case.resources)?;
    
    // Find the base64Binary test
    let base64_test = test_case.tests.iter()
        .find(|test| test.title == "base64Binary")
        .ok_or("base64Binary test not found")?;
    
    println!("Found test: {}", base64_test.title);
    
    // Parse ViewDefinition
    let view_definition = parse_view_definition(&base64_test.view)?;
    println!("Successfully parsed ViewDefinition");
    
    // Run the view definition
    match run_view_definition(view_definition, bundle, ContentType::Json) {
        Ok(result) => {
            println!("✅ Test executed successfully!");
            println!("Result data: {}", String::from_utf8_lossy(&result));
            
            // Parse and compare with expected results
            let actual_rows: Vec<serde_json::Value> = serde_json::from_slice(&result)?;
            if let Some(expected) = &base64_test.expect {
                if actual_rows.len() == expected.len() {
                    println!("✅ Result count matches expected: {} rows", actual_rows.len());
                    for (i, (actual, expected)) in actual_rows.iter().zip(expected.iter()).enumerate() {
                        println!("Row {}: actual={}, expected={}", i, actual, expected);
                    }
                } else {
                    println!("❌ Result count mismatch: got {}, expected {}", actual_rows.len(), expected.len());
                }
            }
        }
        Err(e) => {
            println!("❌ Test failed with error: {}", e);
            if e.to_string().contains("Unsupported constant type") {
                println!("   This indicates the Canonical constant type fix may not be working");
            }
            return Err(e);
        }
    }
    
    println!("Test completed!");
    Ok(())
}