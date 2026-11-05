use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct TestCase {
    #[allow(dead_code)]
    title: String,
    #[allow(dead_code)]
    description: String,
    // Optional: some upstream fixtures (e.g. constant_types.json) omit `fhirVersion`, which the
    // spec treats as "applies to all versions".
    #[serde(rename = "fhirVersion", default)]
    fhir_version: Option<Vec<String>>,
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
    // Parsed but NOT asserted. `expectColumns` pins column *ordering*, which this
    // runner cannot see: it compares `serde_json::Value` rows, and without the
    // `preserve_order` feature `serde_json::Map` is unordered, so the order is
    // already lost by the time we compare. Checking it needs a raw-text compare
    // against the emitted JSON/NDJSON.
    //
    // Scope, measured: exactly 1 of the 144 corpus cases declares it
    // (`basic.json::column ordering`). Called out here rather than silently
    // dropped, since unasserted assertions are the subject of issue #307.
    #[allow(dead_code)]
    #[serde(rename = "expectColumns")]
    expect_columns: Option<Vec<String>>,
    #[serde(rename = "expectError")]
    expect_error: Option<bool>,
}

#[derive(Debug, Serialize)]
struct TestResult {
    passed: bool,
    error: Option<String>,
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
                    error: None,
                };
            } else {
                return TestResult {
                    passed: false,
                    error: Some(format!("Failed to parse ViewDefinition: {}", e)),
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
                    error: None,
                };
            } else {
                return TestResult {
                    passed: false,
                    error: Some(format!("Failed to execute ViewDefinition: {}", e)),
                };
            }
        }
    };

    // If we get here and expect_error is true, the test failed (no error occurred)
    if expect_error {
        return TestResult {
            passed: false,
            error: Some("Expected an error but ViewDefinition executed successfully".to_string()),
        };
    }

    // Parse the result as JSON
    let actual_rows: Vec<serde_json::Value> = match serde_json::from_slice(&result) {
        Ok(rows) => rows,
        Err(e) => {
            return TestResult {
                passed: false,
                error: Some(format!("Failed to parse result as JSON: {}", e)),
            };
        }
    };

    // Compare with expected results
    match &test.expect {
        Some(expected) => {
            if compare_results(&actual_rows, expected) {
                TestResult {
                    passed: true,
                    error: None,
                }
            } else {
                TestResult {
                    passed: false,
                    error: Some(format!(
                        "Results don't match. Expected: {}, Got: {}",
                        serde_json::to_string_pretty(expected).unwrap_or_default(),
                        serde_json::to_string_pretty(&actual_rows).unwrap_or_default()
                    )),
                }
            }
        }
        None => TestResult {
            passed: false,
            error: Some("Test has neither 'expect' nor 'expectError' field".to_string()),
        },
    }
}

fn compare_results(actual: &[serde_json::Value], expected: &[serde_json::Value]) -> bool {
    if actual.len() != expected.len() {
        return false;
    }

    // Row order is not significant in SQL-on-FHIR (the official compliance runner canonicalizes
    // both sides before comparing). Match as a multiset: every expected row must consume a distinct
    // actual row.
    let mut used = vec![false; actual.len()];
    for expected_row in expected {
        let matched = actual
            .iter()
            .enumerate()
            .find(|(idx, actual_row)| !used[*idx] && compare_json_values(actual_row, expected_row));
        match matched {
            Some((idx, _)) => used[idx] = true,
            None => return false,
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

    // Check if we support the FHIR version. A missing `fhirVersion` applies to all versions.
    let supports_r4 = test_case
        .fhir_version
        .as_ref()
        .map(|v| v.contains(&"4.0.1".to_string()))
        .unwrap_or(true);
    if !supports_r4 {
        test_reports.push(TestReport {
            name: "version_check".to_string(),
            result: TestResult {
                passed: false,
                error: Some("Only R4 (4.0.1) is currently supported".to_string()),
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

/// Lower bounds on the vendored SQL-on-FHIR v2 corpus.
///
/// The corpus is git-tracked (22 fixture files, 144 test cases at time of
/// writing), so these are not guesses — they are a floor that catches a
/// half-checked-out or silently emptied fixture tree. Without them, an empty
/// directory yields `0 == 0` passes, prints `NaN%`, and reports success
/// (issue #307). Raise them when the corpus grows.
const MIN_FIXTURE_FILES: usize = 22;
const MIN_TEST_CASES: usize = 144;

#[test]
fn run_comprehensive_test_suite() {
    let mut test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    test_dir.push("tests/sql-on-fhir/tests");

    // Previously an early `return`, i.e. a silent pass. The fixtures are tracked
    // in git, so a missing directory means a broken checkout, not an optional
    // extra — and this test is the one that feeds the published conformance
    // report, so a vacuous pass here becomes a vacuous public number.
    assert!(
        test_dir.is_dir(),
        "SQL-on-FHIR conformance fixtures not found at {}. These are tracked in git; \
         a missing directory means a broken checkout, not an optional test suite.",
        test_dir.display()
    );

    let mut all_reports = HashMap::new();
    // Explicitly `usize` so they compare directly against the corpus floors below
    // (an inferred `i32` would not).
    let mut total_tests: usize = 0;
    let mut passed_tests: usize = 0;
    let mut fixture_files: usize = 0;

    // Run all test files
    for entry in fs::read_dir(&test_dir).expect("Failed to read test directory") {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            fixture_files += 1;
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
                                    .error
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
                                    error: Some(format!("Failed to load test file: {}", e)),
                                },
                            }],
                        },
                    );
                    total_tests += 1;
                }
            }
        }
    }

    // Corpus floors come BEFORE the pass/total comparison: with an empty corpus
    // that comparison is `0 == 0` and passes, after printing a NaN success rate.
    assert!(
        fixture_files >= MIN_FIXTURE_FILES,
        "only {fixture_files} fixture file(s) found in {} (expected at least \
         {MIN_FIXTURE_FILES}). The conformance corpus has shrunk or failed to check out; \
         refusing to report a green run over a corpus this small.",
        test_dir.display(),
    );
    assert!(
        total_tests >= MIN_TEST_CASES,
        "only {total_tests} conformance case(s) executed (expected at least {MIN_TEST_CASES}). \
         The fixture files are present but nearly empty, or parsing silently dropped their \
         `tests` arrays.",
    );

    println!("\n=== TEST SUMMARY ===");
    println!("Total tests: {}", total_tests);
    println!("Passed: {}", passed_tests);
    println!("Failed: {}", total_tests - passed_tests);
    println!(
        "Success rate: {:.1}%",
        (passed_tests as f64 / total_tests as f64) * 100.0
    );
    // Machine-readable line for the CI freshness/non-vacuity check.
    println!("SOF-CONFORMANCE: files={fixture_files} total={total_tests} passed={passed_tests}");

    // Save the test report.
    //
    // This file is a published deliverable, not a scratch artifact: CI's `build`
    // job uploads `crates/sof/test_report.json` and `publish-report` deploys it to
    // GitHub Pages on tagged releases. It is therefore written to a path derived
    // from CARGO_MANIFEST_DIR rather than the process CWD — the previous relative
    // `fs::write("test_report.json", ...)` only landed in the right place because
    // cargo happens to set CWD to the package root, so any change to how the test
    // is invoked would have silently relocated a release artifact.
    //
    // Deliberately NOT moved to OUT_DIR (as issue #307 suggested): OUT_DIR is not
    // set for integration-test binaries, and its hashed path could not be named by
    // the workflow's `path:` upload.
    let report_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_report.json");
    let report_json =
        serde_json::to_string_pretty(&all_reports).expect("Failed to serialize test report");
    fs::write(&report_path, report_json).unwrap_or_else(|e| {
        panic!(
            "Failed to write test report to {}: {e}",
            report_path.display()
        )
    });
    println!("\nTest report saved to {}", report_path.display());

    // Fail the test if any individual tests failed
    assert_eq!(
        passed_tests,
        total_tests,
        "Test suite failed: {} out of {} tests failed",
        total_tests - passed_tests,
        total_tests
    );
}
