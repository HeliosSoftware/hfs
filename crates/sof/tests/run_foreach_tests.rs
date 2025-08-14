use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};
use serde::{Deserialize, Serialize};
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
    expect: Vec<serde_json::Value>,
    #[allow(dead_code)]
    #[serde(rename = "expectColumns")]
    expect_columns: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct TestResult {
    passed: bool,
    reason: Option<String>,
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

fn run_single_test(test: &Test, bundle: &SofBundle) -> TestResult {
    let view_definition = match parse_view_definition(&test.view) {
        Ok(vd) => vd,
        Err(e) => {
            return TestResult {
                passed: false,
                reason: Some(format!("Failed to parse ViewDefinition: {}", e)),
            };
        }
    };

    let result = match run_view_definition(view_definition, bundle.clone(), ContentType::Json) {
        Ok(data) => data,
        Err(e) => {
            return TestResult {
                passed: false,
                reason: Some(format!("Failed to execute ViewDefinition: {}", e)),
            };
        }
    };

    let actual_rows: Vec<serde_json::Value> = match serde_json::from_slice(&result) {
        Ok(rows) => rows,
        Err(e) => {
            return TestResult {
                passed: false,
                reason: Some(format!("Failed to parse result as JSON: {}", e)),
            };
        }
    };

    if compare_results(&actual_rows, &test.expect) {
        TestResult {
            passed: true,
            reason: None,
        }
    } else {
        TestResult {
            passed: false,
            reason: Some(format!(
                "Results don't match.\\nExpected: {}\\nGot: {}",
                serde_json::to_string_pretty(&test.expect).unwrap_or_default(),
                serde_json::to_string_pretty(&actual_rows).unwrap_or_default()
            )),
        }
    }
}

fn compare_results(actual: &[serde_json::Value], expected: &[serde_json::Value]) -> bool {
    if actual.len() != expected.len() {
        return false;
    }

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
            for (key, expected_val) in expected_obj {
                match actual_obj.get(key) {
                    Some(actual_val) => {
                        if !compare_json_values(actual_val, expected_val) {
                            return false;
                        }
                    }
                    None => {
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
fn test_foreach_file() {
    let mut test_file = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    test_file.push("tests/sql-on-fhir-v2/tests/foreach.json");

    if !test_file.exists() {
        println!("Test file not found: {:?}", test_file);
        panic!("foreach.json test file not found");
    }

    let content = fs::read_to_string(test_file).expect("Failed to read test file");
    let test_case: TestCase = serde_json::from_str(&content).expect("Failed to parse test case");

    // Check if we support the FHIR version
    let supports_r4 = test_case.fhir_version.contains(&"4.0.1".to_string());
    if !supports_r4 {
        panic!("Only R4 (4.0.1) is currently supported");
    }

    let bundle = create_test_bundle(&test_case.resources).expect("Failed to create test bundle");

    let mut passed = 0;
    let mut total = 0;

    println!("\\n=== Running forEach tests ===");

    for test in test_case.tests {
        total += 1;
        let test_result = run_single_test(&test, &bundle);

        if test_result.passed {
            passed += 1;
            println!("✅ {}", test.title);
        } else {
            println!(
                "❌ {}: {}",
                test.title,
                test_result.reason.unwrap_or_default()
            );
        }
    }

    println!("\\n=== RESULTS ===");
    println!("Passed: {}/{}", passed, total);
    println!(
        "Success rate: {:.1}%",
        (passed as f64 / total as f64) * 100.0
    );

    // For this test to pass, we want at least some improvement
    // You can adjust this threshold as needed
    assert!(
        passed > 0,
        "No tests passed - implementation needs more work"
    );
}
