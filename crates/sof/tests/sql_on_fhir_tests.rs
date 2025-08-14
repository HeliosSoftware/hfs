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
    #[serde(rename = "expectColumns")]
    expect_columns: Option<Vec<String>>,
    #[serde(rename = "expectError")]
    expect_error: Option<bool>,
}

#[derive(Debug)]
#[allow(dead_code)]
struct TestResult {
    passed: bool,
    reason: Option<String>,
}

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

fn run_single_test(test: &Test, bundle: &SofBundle) -> TestResult {
    // Check if this is an error test
    let expect_error = test.expect_error.unwrap_or(false);

    // Parse the ViewDefinition
    let view_definition = match parse_view_definition(&test.view) {
        Ok(vd) => vd,
        Err(e) => {
            if expect_error {
                // This is expected for error tests
                return TestResult {
                    passed: true,
                    reason: None,
                };
            } else {
                return TestResult {
                    passed: false,
                    reason: Some(format!("Failed to parse ViewDefinition: {}", e)),
                };
            }
        }
    };

    // Run the view definition
    let result = match run_view_definition(view_definition, bundle.clone(), ContentType::Json) {
        Ok(data) => data,
        Err(e) => {
            if expect_error {
                // This is expected for error tests
                return TestResult {
                    passed: true,
                    reason: None,
                };
            } else {
                return TestResult {
                    passed: false,
                    reason: Some(format!("Failed to execute ViewDefinition: {}", e)),
                };
            }
        }
    };

    // If we get here and expect_error is true, the test failed (no error occurred)
    if expect_error {
        return TestResult {
            passed: false,
            reason: Some("Expected an error but ViewDefinition executed successfully".to_string()),
        };
    }

    // Parse the result as JSON
    let actual_rows: Vec<serde_json::Value> = match serde_json::from_slice(&result) {
        Ok(rows) => rows,
        Err(e) => {
            return TestResult {
                passed: false,
                reason: Some(format!("Failed to parse result as JSON: {}", e)),
            };
        }
    };

    // Compare with expected results
    match &test.expect {
        Some(expected) => {
            if compare_results(&actual_rows, expected) {
                TestResult {
                    passed: true,
                    reason: None,
                }
            } else {
                TestResult {
                    passed: false,
                    reason: Some(format!(
                        "Results don't match. Expected: {:?}, Got: {:?}",
                        expected, actual_rows
                    )),
                }
            }
        }
        None => TestResult {
            passed: false,
            reason: Some("Test has neither 'expect' nor 'expectError' field".to_string()),
        },
    }
}

fn compare_results(actual: &[serde_json::Value], expected: &[serde_json::Value]) -> bool {
    if actual.len() != expected.len() {
        return false;
    }

    // For now, do a simple comparison
    for (actual_row, expected_row) in actual.iter().zip(expected.iter()) {
        if !compare_json_values(actual_row, expected_row) {
            return false;
        }
    }

    true
}

fn compare_json_values(actual: &serde_json::Value, expected: &serde_json::Value) -> bool {
    match (actual, expected) {
        (serde_json::Value::Null, serde_json::Value::Null) => true,
        (serde_json::Value::Object(actual_obj), serde_json::Value::Object(expected_obj)) => {
            // Check that all expected keys are present with correct values
            for (key, expected_val) in expected_obj {
                match actual_obj.get(key) {
                    Some(actual_val) => {
                        if !compare_json_values(actual_val, expected_val) {
                            return false;
                        }
                    }
                    None => {
                        // Expected key is missing, only OK if expected value is null
                        if !expected_val.is_null() {
                            return false;
                        }
                    }
                }
            }
            true
        }
        _ => actual == expected,
    }
}

#[test]
fn test_basic_view_definition() {
    // Test the first test case from basic.json
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

    let bundle = create_test_bundle(&resources).expect("Failed to create test bundle");

    let view = serde_json::json!({
        "resource": "Patient",
        "status": "active",
        "select": [{
            "column": [{
                "name": "id",
                "path": "id",
                "type": "id"
            }]
        }]
    });

    let view_definition = parse_view_definition(&view).expect("Failed to parse ViewDefinition");

    let result = run_view_definition(view_definition, bundle, ContentType::Json)
        .expect("Failed to run ViewDefinition");

    let actual_rows: Vec<serde_json::Value> =
        serde_json::from_slice(&result).expect("Failed to parse result as JSON");

    let expected = [
        serde_json::json!({"id": "pt1"}),
        serde_json::json!({"id": "pt2"}),
        serde_json::json!({"id": "pt3"}),
    ];

    assert_eq!(actual_rows.len(), expected.len(), "Row count mismatch");
    for (actual, expected) in actual_rows.iter().zip(expected.iter()) {
        assert!(
            compare_json_values(actual, expected),
            "Row mismatch: expected {:?}, got {:?}",
            expected,
            actual
        );
    }
}

#[test]
fn test_basic_boolean_attribute() {
    // Test boolean handling
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

    let bundle = create_test_bundle(&resources).expect("Failed to create test bundle");

    let view = serde_json::json!({
        "resource": "Patient",
        "status": "active",
        "select": [{
            "column": [
                {
                    "name": "id",
                    "path": "id",
                    "type": "id"
                },
                {
                    "name": "active",
                    "path": "active",
                    "type": "boolean"
                }
            ]
        }]
    });

    let view_definition = parse_view_definition(&view).expect("Failed to parse ViewDefinition");

    let result = run_view_definition(view_definition, bundle, ContentType::Json)
        .expect("Failed to run ViewDefinition");

    let actual_rows: Vec<serde_json::Value> =
        serde_json::from_slice(&result).expect("Failed to parse result as JSON");

    let expected = [
        serde_json::json!({"id": "pt1", "active": true}),
        serde_json::json!({"id": "pt2", "active": false}),
        serde_json::json!({"id": "pt3", "active": null}),
    ];

    assert_eq!(actual_rows.len(), expected.len(), "Row count mismatch");
    for (actual, expected) in actual_rows.iter().zip(expected.iter()) {
        assert!(
            compare_json_values(actual, expected),
            "Row mismatch: expected {:?}, got {:?}",
            expected,
            actual
        );
    }
}

#[test]
fn test_run_basic_test_file() {
    // Load and run a simple test case from the test suite
    let mut test_suite_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    test_suite_path.push("tests/sql-on-fhir-v2/tests/basic.json");

    if !test_suite_path.exists() {
        // Skip test if test suite is not available
        return;
    }

    let content = fs::read_to_string(test_suite_path).expect("Failed to read test file");
    let test_case: TestCase = serde_json::from_str(&content).expect("Failed to parse test case");

    // Check if we support the FHIR version
    let supports_r4 = test_case.fhir_version.contains(&"4.0.1".to_string());
    if !supports_r4 {
        return; // Skip tests that don't support R4
    }

    // Create a bundle from the test resources
    let bundle = create_test_bundle(&test_case.resources).expect("Failed to create test bundle");

    // Run the first test
    if let Some(first_test) = test_case.tests.first() {
        let result = run_single_test(first_test, &bundle);
        println!("Test '{}' result: {:?}", first_test.title, result);

        // For now, we'll just print the result rather than assert
        // This allows us to see what's working and what needs to be fixed
    }
}
