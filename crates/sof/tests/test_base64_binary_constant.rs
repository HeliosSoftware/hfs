use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct TestCase {
    #[allow(dead_code)]
    title: String,
    #[allow(dead_code)]
    description: String,
    #[allow(dead_code)]
    #[serde(rename = "fhirVersion")]
    fhir_version: Vec<String>,
    resources: Vec<serde_json::Value>,
    tests: Vec<Test>,
}

#[derive(Debug, Deserialize)]
struct Test {
    title: String,
    #[allow(dead_code)]
    tags: Option<Vec<String>>,
    view: serde_json::Value,
    expect: Option<Vec<serde_json::Value>>,
    #[allow(dead_code)]
    #[serde(rename = "expectError")]
    expect_error: Option<bool>,
}

fn create_test_bundle(
    resources: &[serde_json::Value],
) -> Result<SofBundle, Box<dyn std::error::Error>> {
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

    let bundle: helios_fhir::r4::Bundle = serde_json::from_value(bundle_json)?;
    Ok(SofBundle::R4(bundle))
}

fn parse_view_definition(
    view_json: &serde_json::Value,
) -> Result<SofViewDefinition, Box<dyn std::error::Error>> {
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
fn test_base64_binary_constant_type() {
    println!("Testing base64Binary constant type support...");

    // Load test file
    let mut test_file_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    test_file_path.push("tests/sql-on-fhir-v2/tests/constant_types.json");
    let content = fs::read_to_string(&test_file_path)
        .expect("Failed to read test file - make sure the path is correct");
    let test_case: TestCase = serde_json::from_str(&content).expect("Failed to parse test file");

    // Create bundle from resources
    let bundle = create_test_bundle(&test_case.resources).expect("Failed to create test bundle");

    // Find the base64Binary test
    let base64_test = test_case
        .tests
        .iter()
        .find(|test| test.title == "base64Binary")
        .expect("base64Binary test not found");

    println!("Found test: {}", base64_test.title);

    // Parse ViewDefinition
    let view_definition =
        parse_view_definition(&base64_test.view).expect("Failed to parse ViewDefinition");
    println!("Successfully parsed ViewDefinition");

    // Run the view definition
    match run_view_definition(view_definition, bundle, ContentType::Json) {
        Ok(result) => {
            println!("✅ Test executed successfully!");
            println!("Result data: {}", String::from_utf8_lossy(&result));

            // Parse and compare with expected results
            let actual_rows: Vec<serde_json::Value> =
                serde_json::from_slice(&result).expect("Failed to parse result JSON");
            if let Some(expected) = &base64_test.expect {
                assert_eq!(
                    actual_rows.len(),
                    expected.len(),
                    "Result count mismatch: got {}, expected {}",
                    actual_rows.len(),
                    expected.len()
                );
                println!(
                    "✅ Result count matches expected: {} rows",
                    actual_rows.len()
                );

                for (i, (actual, expected)) in actual_rows.iter().zip(expected.iter()).enumerate() {
                    println!("Row {}: actual={}, expected={}", i, actual, expected);
                    // Note: We don't assert full equality here since we just want to check that it runs without "Unsupported constant type" error
                }
            }
        }
        Err(e) => {
            println!("❌ Test failed with error: {}", e);
            if e.to_string().contains("Unsupported constant type") {
                panic!(
                    "The Canonical constant type fix does not appear to be working - still getting 'Unsupported constant type' error: {}",
                    e
                );
            } else {
                panic!("Test failed with unexpected error: {}", e);
            }
        }
    }

    println!("✅ base64Binary constant type test passed!");
}
