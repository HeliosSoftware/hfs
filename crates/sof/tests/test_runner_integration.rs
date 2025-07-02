use serde::{Deserialize, Serialize};
use sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

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

#[derive(Debug, Serialize)]
struct TestResult {
    passed: bool,
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct TestReport {
    name: String,
    result: TestResult,
}

#[derive(Debug, Serialize)]
struct TestSuiteReport {
    tests: Vec<TestReport>,
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

    let bundle: fhir::r4::Bundle = serde_json::from_value(bundle_json)?;
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

    let view_definition: fhir::r4::ViewDefinition = serde_json::from_value(view_def)?;
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
                        "Results don't match. Expected: {}, Got: {}",
                        serde_json::to_string_pretty(expected).unwrap_or_default(),
                        serde_json::to_string_pretty(&actual_rows).unwrap_or_default()
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

fn run_test_file(test_file: &Path) -> Result<TestSuiteReport, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(test_file)?;
    let test_case: TestCase = serde_json::from_str(&content)?;

    let mut test_reports = Vec::new();

    // Check if we support the FHIR version
    let supports_r4 = test_case.fhir_version.contains(&"4.0.1".to_string());
    if !supports_r4 {
        test_reports.push(TestReport {
            name: "version_check".to_string(),
            result: TestResult {
                passed: false,
                reason: Some("Only R4 (4.0.1) is currently supported".to_string()),
            },
        });
        return Ok(TestSuiteReport {
            tests: test_reports,
        });
    }

    let bundle = create_test_bundle(&test_case.resources)?;

    for test in test_case.tests {
        let test_result = run_single_test(&test, &bundle);
        test_reports.push(TestReport {
            name: test.title,
            result: test_result,
        });
    }

    Ok(TestSuiteReport {
        tests: test_reports,
    })
}

#[test]
fn run_comprehensive_test_suite() {
    let mut test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    test_dir.push("tests/sql-on-fhir-v2/tests");

    if !test_dir.exists() {
        println!("Test suite directory not found at: {:?}", test_dir);
        return;
    }

    let mut all_reports = HashMap::new();
    let mut total_tests = 0;
    let mut passed_tests = 0;

    // Run all test files
    for entry in fs::read_dir(test_dir).expect("Failed to read test directory") {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            let file_name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            println!("\n=== Running test file: {} ===", file_name);

            match run_test_file(&path) {
                Ok(suite_report) => {
                    for test_report in &suite_report.tests {
                        total_tests += 1;
                        if test_report.result.passed {
                            passed_tests += 1;
                            println!("✅ {}", test_report.name);
                        } else {
                            println!(
                                "❌ {}: {}",
                                test_report.name,
                                test_report
                                    .result
                                    .reason
                                    .as_deref()
                                    .unwrap_or("Unknown error")
                            );
                        }
                    }
                    all_reports.insert(file_name, suite_report);
                }
                Err(e) => {
                    println!("❌ Error running test file {}: {}", file_name, e);
                    all_reports.insert(
                        file_name,
                        TestSuiteReport {
                            tests: vec![TestReport {
                                name: "file_error".to_string(),
                                result: TestResult {
                                    passed: false,
                                    reason: Some(format!("Failed to load test file: {}", e)),
                                },
                            }],
                        },
                    );
                    total_tests += 1;
                }
            }
        }
    }

    println!("\n=== TEST SUMMARY ===");
    println!("Total tests: {}", total_tests);
    println!("Passed: {}", passed_tests);
    println!("Failed: {}", total_tests - passed_tests);
    println!(
        "Success rate: {:.1}%",
        (passed_tests as f64 / total_tests as f64) * 100.0
    );

    // Save the test report
    let report_json =
        serde_json::to_string_pretty(&all_reports).expect("Failed to serialize test report");
    fs::write("test_report.json", report_json).expect("Failed to write test report");
    println!("\nTest report saved to test_report.json");

    // Fail the test if any individual tests failed
    assert_eq!(
        passed_tests,
        total_tests,
        "Test suite failed: {} out of {} tests failed",
        total_tests - passed_tests,
        total_tests
    );
}
