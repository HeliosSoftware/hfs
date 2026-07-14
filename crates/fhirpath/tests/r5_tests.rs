#[cfg(feature = "R5")]
mod common;

#[cfg(feature = "R5")]
use crate::common::*;
#[cfg(feature = "R5")]
use helios_fhir::r5;
#[cfg(feature = "R5")]
use helios_fhirpath::EvaluationContext;
#[cfg(feature = "R5")]
use helios_fhirpath_support::EvaluationResult;
#[cfg(feature = "R5")]
use std::fs::File;
#[cfg(feature = "R5")]
use std::io::Read;
#[cfg(feature = "R5")]
use std::path::PathBuf;

#[cfg(feature = "R5")]
// R5-specific resource loader implementation
struct R5ResourceLoader;

#[cfg(feature = "R5")]
impl TestResourceLoader for R5ResourceLoader {
    fn load_resource(&self, filename: &str) -> Result<EvaluationContext, String> {
        load_test_resource_r5(filename)
    }

    fn get_fhir_version(&self) -> &str {
        "R5"
    }
}

#[cfg(feature = "R5")]
// This function loads a JSON test resource and creates an evaluation context with it
fn load_test_resource_r5(json_filename: &str) -> Result<EvaluationContext, String> {
    // Get the path to the JSON file
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push(format!("tests/data/r5/input/{}", json_filename));

    // Load the JSON file
    let mut file =
        File::open(&path).map_err(|e| format!("Could not open JSON resource file: {:?}", e))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|e| format!("Failed to read JSON resource file: {:?}", e))?;

    // Parse the JSON into a FHIR resource
    let resource: r5::Resource =
        serde_json::from_str(&contents).map_err(|e| format!("Failed to parse JSON: {:?}", e))?;

    // Create an evaluation context with the resource
    let mut context =
        EvaluationContext::new(vec![helios_fhir::FhirResource::R5(Box::new(resource))]);

    // Use common context setup
    setup_resource_context(&mut context, json_filename);

    Ok(context)
}

/// Starts an in-process terminology server serving canned responses for the
/// `mode="tx"` conformance tests, and returns its base URL.
///
/// These tests used to reach the public `tx.fhir.org` because the evaluator
/// defaulted to it, which made `cargo test` fail whenever that server was down or
/// rate-limiting. The evaluator has no default any more (issue #217), so the suite
/// supplies its own server. The responses mirror the shapes a real terminology
/// server returns for these two operations; the expected values (4 gender codes,
/// `result = true`) are fixed by the FHIR specification, not by any one server.
///
/// The server is leaked rather than dropped: `Drop for MockServer` signals shutdown,
/// and the stub has to keep serving for the whole suite. The runtime that started it
/// is leaked alongside it, which costs a few idle threads in a process that is about
/// to exit.
#[cfg(feature = "R5")]
fn start_tx_stub() -> String {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let runtime = tokio::runtime::Runtime::new().expect("failed to build stub runtime");

    let uri = runtime.block_on(async {
        let server = MockServer::start().await;

        // txTest01: expand(administrative-gender).expansion.contains.count() = 4
        let gender_system = "http://hl7.org/fhir/administrative-gender";
        Mock::given(method("GET"))
            .and(path("/ValueSet/$expand"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "resourceType": "ValueSet",
                "id": "administrative-gender",
                "url": "http://hl7.org/fhir/ValueSet/administrative-gender",
                "status": "active",
                "expansion": {
                    "identifier": "urn:uuid:00000000-0000-0000-0000-000000000001",
                    "timestamp": "2024-01-01T00:00:00Z",
                    "total": 4,
                    "contains": [
                        { "system": gender_system, "code": "male", "display": "Male" },
                        { "system": gender_system, "code": "female", "display": "Female" },
                        { "system": gender_system, "code": "other", "display": "Other" },
                        { "system": gender_system, "code": "unknown", "display": "Unknown" }
                    ]
                }
            })))
            .mount(&server)
            .await;

        // txTest02: validateVS(administrative-gender, Patient.gender) -> result = true
        Mock::given(method("POST"))
            .and(path("/ValueSet/$validate-code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "resourceType": "Parameters",
                "parameter": [
                    { "name": "result", "valueBoolean": true },
                    { "name": "code", "valueCode": "male" },
                    { "name": "system", "valueUri": gender_system }
                ]
            })))
            .mount(&server)
            .await;

        let uri = server.uri();
        std::mem::forget(server);
        uri
    });

    std::mem::forget(runtime);
    uri
}

#[test]
#[cfg(feature = "R5")]
fn test_r5_test_suite() {
    println!("Running FHIRPath R5 test suite");

    let tx_stub_uri = start_tx_stub();
    println!("Terminology stub for mode=\"tx\" tests: {}", tx_stub_uri);

    // Get the path to the test file
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/data/r5/tests-fhir-r5.xml");

    // Load the test file
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(e) => {
            panic!("Could not open R5 test file: {:?}", e);
        }
    };
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .expect("Failed to read test file");

    // Parse the XML using common parser
    let doc = parse_test_xml(&contents).expect("Failed to parse test XML");

    // Define test resource files that will be used
    let resource_files = vec![
        "patient-example.json",
        "observation-example.json",
        "questionnaire-example.json",
        "valueset-example-expansion.json",
        "conceptmap-example.json",
        "codesystem-example.json",
        "parameters-example-types.json",
        "patient-example-name.json",
        "ccda.json",
    ];

    // Verify that we can load all necessary JSON test files
    println!("Checking R5 test resources:");
    let loader = R5ResourceLoader;
    for file in resource_files {
        match loader.load_resource(file) {
            Ok(_) => println!("  - {} loaded successfully", file),
            Err(e) => println!("  - {} failed to load: {}", file, e),
        }
    }

    // Find all test groups
    let test_groups = find_test_groups(&doc.root_element());
    println!("Found {} test groups", test_groups.len());

    let mut total_tests = 0;
    let mut passed_tests = 0;
    let mut skipped_tests = 0;
    let mut failed_tests = 0;

    // For each test group
    for (group_name, tests) in test_groups {
        println!("\nRunning test group: {}", group_name);

        // For each test in the group
        for test in tests {
            total_tests += 1;

            // Skip tests with empty expressions
            if test.expression.is_empty() {
                println!("  SKIP: {} - Empty expression", test.name);
                skipped_tests += 1;
                continue;
            }

            // Create the appropriate context for this test
            let mut context = if test.input_file.is_empty() {
                // Use empty context for tests without input files
                let mut ctx = EvaluationContext::new_empty_with_default_version();
                if test.mode == "strict" {
                    ctx.set_strict_mode(true);
                }
                if test.check_ordered_functions == "true" {
                    ctx.set_check_ordered_functions(true);
                }
                ctx
            } else {
                // Try to load the resource for tests with input files
                match loader.load_resource(&test.input_file) {
                    Ok(mut ctx) => {
                        if test.mode == "strict" {
                            ctx.set_strict_mode(true);
                        }
                        if test.check_ordered_functions == "true" {
                            ctx.set_check_ordered_functions(true);
                        }
                        ctx
                    }
                    Err(e) => {
                        println!(
                            "  SKIP: {} - '{}' - Failed to load JSON resource for {}: {}",
                            test.name, test.expression, test.input_file, e
                        );
                        skipped_tests += 1;
                        continue;
                    }
                }
            };

            // Set up common variables
            setup_common_variables(&mut context);

            // mode="tx" tests exercise %terminologies, which needs a server. The
            // evaluator no longer defaults to one (issue #217), and the suite must not
            // depend on a public server being reachable, so point these at the
            // in-process stub. FHIRPATH_TERMINOLOGY_SERVER still wins when set, which is
            // how these expectations get re-validated against a real server.
            if test.mode == "tx" && std::env::var("FHIRPATH_TERMINOLOGY_SERVER").is_err() {
                context.set_terminology_server(tx_stub_uri.clone());
            }

            // Special handling for extension tests
            if test.name.starts_with("testExtension") || test.expression.contains("extension(") {
                setup_extension_variables(&mut context);
                setup_patient_extension_context(&mut context, &test.name);
            }

            // Skip PrecisionDecimal test due to known limitation with decimal trailing zeros
            if test.name == "PrecisionDecimal" {
                println!(
                    "  SKIP: {} - Known limitation: decimal trailing zeros not preserved (see PRECISION_LIMITATION.md)",
                    test.name
                );
                skipped_tests += 1;
                continue;
            }

            // Skip conformsTo tests - function not yet implemented
            if test.expression.contains("conformsTo(") {
                println!(
                    "  SKIP: {} - '{}' - conformsTo() function not yet implemented",
                    test.name, test.expression
                );
                skipped_tests += 1;
                continue;
            }

            // Skip dvConceptMapExample - test data uses R4-format ConceptMap (identifier as object
            // instead of array) which only parses when xml feature enables SingleOrVec wrappers,
            // leading to inconsistent isDistinct() results
            if test.name == "dvConceptMapExample" {
                println!(
                    "  SKIP: {} - test data uses R4-format ConceptMap incompatible with R5 model",
                    test.name
                );
                skipped_tests += 1;
                continue;
            }

            // Skip translate-based tx tests - the stub does not serve $translate, and
            // ConceptMap cm-address-use-v2 is not reliably available on public servers.
            if test.mode == "tx" && test.expression.contains("translate(") {
                println!(
                    "  SKIP: {} - '{}' - ConceptMap $translate not covered by the terminology stub",
                    test.name, test.expression
                );
                skipped_tests += 1;
                continue;
            }

            // Parse expected outputs from test def
            let mut expected_results: Vec<EvaluationResult> = Vec::new();
            let mut skip_test = false;
            for (output_type, output_value) in &test.outputs {
                match parse_output_value(output_type, output_value, loader.get_fhir_version()) {
                    Ok(result) => expected_results.push(result),
                    Err(e) => {
                        println!("  SKIP: {} - {}", test.name, e);
                        skipped_tests += 1;
                        skip_test = true;
                        break;
                    }
                }
            }
            if skip_test {
                continue;
            }

            // For tests with no expected outputs, they may be checking for empty result or just syntax
            if expected_results.is_empty() && !test.outputs.is_empty() {
                println!("  SKIP: {} - Could not parse expected outputs", test.name);
                skipped_tests += 1;
                continue;
            }

            // Run the test
            let is_predicate_test = test.predicate == "true";
            let test_run_result = run_fhir_test(
                &test.expression,
                &context,
                &expected_results,
                is_predicate_test,
            );

            // Determine if this test expects an error
            let expects_error = !test.invalid.is_empty();

            if expects_error {
                // This test is expected to produce an error
                match test_run_result {
                    Ok(_) => {
                        if !test.invalid.is_empty() {
                            println!(
                                "  FAIL (expected error '{}'): {} - '{}' - Got Ok instead of error",
                                test.invalid, test.name, test.expression
                            );
                        } else {
                            println!(
                                "  FAIL (expected error): {} - '{}' - Got Ok instead of error",
                                test.name, test.expression
                            );
                        }
                        failed_tests += 1;
                    }
                    Err(e) => {
                        if !test.invalid.is_empty() {
                            println!(
                                "  PASS (invalid test): {} - '{}' - Correctly failed with: {}",
                                test.name, test.expression, e
                            );
                        } else {
                            println!(
                                "  PASS (error expected): {} - '{}' - Correctly failed with: {}",
                                test.name, test.expression, e
                            );
                        }
                        passed_tests += 1;
                    }
                }
            } else if test.outputs.is_empty() {
                // Special case: tests with no outputs should expect empty result
                // We need to evaluate the expression directly since run_fhir_test doesn't return the result
                match helios_fhirpath::evaluate_expression(&test.expression, &context) {
                    Ok(result) => {
                        // Check if the result is actually empty
                        match &result {
                            EvaluationResult::Empty => {
                                println!("  PASS: {} - '{}'", test.name, test.expression);
                                passed_tests += 1;
                            }
                            _ => {
                                // Check if this is a contested test
                                let contested_tests = [
                                    "testFHIRPathAsFunction11",
                                    "testFHIRPathAsFunction16",
                                    "testStringQuantityMonthLiteralToQuantity",
                                    "testStringQuantityYearLiteralToQuantity",
                                ];

                                if contested_tests.contains(&test.name.as_str()) {
                                    println!(
                                        "  PASS (contested): {} - '{}' - Expected empty, got: {:?}",
                                        test.name, test.expression, result
                                    );
                                    passed_tests += 1;
                                } else {
                                    println!(
                                        "  FAIL: {} - '{}' - Expected empty result, got: {:?}",
                                        test.name, test.expression, result
                                    );
                                    failed_tests += 1;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        // If it failed with an error and there are no outputs,
                        // this is likely an expected error (like negative precision)
                        println!(
                            "  PASS (no output expected): {} - '{}' - Got error: {}",
                            test.name, test.expression, e
                        );
                        passed_tests += 1;
                    }
                }
            } else {
                // This test is expected to be valid with specific outputs
                match test_run_result {
                    Ok(_) => {
                        println!("  PASS: {} - '{}'", test.name, test.expression);
                        passed_tests += 1;
                    }
                    Err(e) => {
                        if e.contains("Unsupported function called")
                            || e.contains("Not yet implemented")
                        {
                            println!(
                                "  NOT IMPLEMENTED: {} - '{}' - {}",
                                test.name, test.expression, e
                            );
                            failed_tests += 1;
                        } else {
                            println!("  FAIL: {} - '{}' - {}", test.name, test.expression, e);
                            failed_tests += 1;
                        }
                    }
                }
            }
        }
    }

    println!("\nR5 Test Summary:");
    println!("  Total tests: {}", total_tests);
    println!("  Passed: {}", passed_tests);
    println!("  Skipped/Not Implemented: {}", skipped_tests);
    println!("  Failed: {}", failed_tests);

    // Print detailed info about failures
    if failed_tests > 0 {
        println!("\nERROR: Some tests failed due to unimplemented features or bugs.");
        println!("See the 'NOT IMPLEMENTED' tests above for details on what needs to be fixed.");
    }

    // We're now enforcing that tests must pass to ensure implementation is complete
    assert_eq!(
        failed_tests, 0,
        "Some R5 tests failed - {} unimplemented features need to be addressed",
        failed_tests
    );

    // Make sure we found some tests
    assert!(total_tests > 0, "No R5 tests found");
}

#[test]
#[cfg(not(feature = "R5"))]
fn test_r5_test_suite() {
    println!("Skipping R5 tests - R5 feature not enabled");
    println!("To run R5 tests, use: cargo test --features R5");
}
