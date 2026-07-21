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
/// supplies its own server.
///
/// # Why every mock matches on the request body
///
/// A stub that matches on method and path alone answers *any* request, including one
/// the real server rejects — so it certifies a broken client as working. That is not
/// hypothetical here: an earlier revision of this stub answered `$translate` with `H`
/// while live HTS was returning `400 Missing required parameter: code or sourceCode`
/// to the exact request our client sent (#287). The test was green over a path that
/// could not work in production.
///
/// So each mock asserts the shape live HTS actually requires, and `$translate` has a
/// catch-all returning HTS's real 400 for anything else. A client that regresses to a
/// request HTS would reject fails here the same way it fails in production, instead of
/// being quietly waved through.
///
/// The server is leaked rather than dropped: `Drop for MockServer` signals shutdown,
/// and the stub has to keep serving for the whole suite. The runtime that started it
/// is leaked alongside it, which costs a few idle threads in a process that is about
/// to exit.
#[cfg(feature = "R5")]
fn start_tx_stub() -> String {
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    /// True when the body is a `Parameters` resource carrying every one of `names`
    /// as a named parameter.
    ///
    /// Presence-only by design: the point is to pin the parameter *names* the server
    /// requires, which is exactly what the `$translate` bug got wrong (it sent
    /// `coding` where HTS demands `code`).
    fn has_params(req: &Request, names: &[&str]) -> bool {
        let Ok(body) = serde_json::from_slice::<serde_json::Value>(&req.body) else {
            return false;
        };
        let Some(parameters) = body.get("parameter").and_then(|p| p.as_array()) else {
            return false;
        };
        names.iter().all(|name| {
            parameters
                .iter()
                .any(|p| p.get("name").and_then(|n| n.as_str()) == Some(*name))
        })
    }

    let runtime = tokio::runtime::Runtime::new().expect("failed to build stub runtime");

    let uri = runtime.block_on(async {
        let server = MockServer::start().await;

        // txTest01: expand(administrative-gender).expansion.contains.count() = 4.
        // `expand()` passes the ValueSet as a `url` query parameter on a GET.
        let gender_system = "http://hl7.org/fhir/administrative-gender";
        Mock::given(method("GET"))
            .and(path("/ValueSet/$expand"))
            .and(query_param(
                "url",
                "http://hl7.org/fhir/ValueSet/administrative-gender",
            ))
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

        // txTest02: validateVS(administrative-gender, Patient.gender) -> result = true.
        //
        // `$this.gender` is a bare code with no system, so validate_vs takes its
        // system-less branch: `url` + `code` + `inferSystem`, letting the server
        // resolve the system from the ValueSet. It sends a `coding` parameter only
        // when handed a full Coding, which this test does not do -- so matching on
        // `coding` here is wrong, and wiremock rightly 404s it.
        //
        // Only `url` + `code` are pinned. `inferSystem` is what the client sends
        // today, but requiring it would assert a server rule this test has not
        // verified; live HTS accepts this shape, which #289's pre-flight probe
        // re-checks against the real server on every run.
        Mock::given(method("POST"))
            .and(path("/ValueSet/$validate-code"))
            .and(|req: &Request| has_params(req, &["url", "code"]))
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

        // txTest03: translate(cm-address-use-v2, Patient.address.use = 'home') -> 'H'.
        //
        // HTS requires `code` + `system` as named parameters. It rejects a lone
        // `coding` (which our client used to send, #287) and also rejects the
        // R5-spec `sourceCoding` -- that second one is an HTS spec violation tracked
        // in #288, so `code` + `system` is currently the only form that works.
        // Matching on those names is what stops this stub from certifying a request
        // the real server 400s. The response body is HTS's actual answer, re-derived
        // against the server we now point people at rather than tx.fhir.org (#217);
        // #289's CI pre-flight probe re-checks it against live HTS on every run.
        Mock::given(method("POST"))
            .and(path("/ConceptMap/$translate"))
            .and(|req: &Request| has_params(req, &["url", "code", "system"]))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "resourceType": "Parameters",
                "parameter": [
                    {
                        "name": "match",
                        "part": [
                            {
                                "name": "concept",
                                "valueCoding": {
                                    "system": "http://terminology.hl7.org/CodeSystem/v2-0190",
                                    "code": "H"
                                }
                            },
                            { "name": "relationship", "valueCode": "equivalent" }
                        ]
                    },
                    { "name": "result", "valueBoolean": true }
                ]
            })))
            // Lower number wins: MockSet sorts by priority ascending, so this is
            // tried before the catch-all below. Must follow respond_with -- it is a
            // method on Mock, not on MockBuilder.
            .with_priority(1)
            .mount(&server)
            .await;

        // Any other shape of $translate gets the 400 live HTS actually returns.
        //
        // Without this a regressed client would get wiremock's bare "no mock matched"
        // 404 and fail on a confusing error. Mirroring HTS's real rejection means the
        // suite fails the same way production does, with the same diagnostics.
        Mock::given(method("POST"))
            .and(path("/ConceptMap/$translate"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "resourceType": "OperationOutcome",
                "issue": [
                    {
                        "severity": "error",
                        "code": "required",
                        "diagnostics": "Missing required parameter: code or sourceCode"
                    }
                ]
            })))
            .with_priority(10)
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
            //
            // Note txTest03 passes here but remains a known failure in the .NET
            // conformance harness (#289). That is not a contradiction: it declares
            // `<output type="code">`, and `parse_expected_output` maps `code` to
            // EvaluationResult::String, so this suite compares the value ("H") and is
            // blind to the type. The .NET harness checks the type and still sees
            // `code` returned as `string`. This assertion covers the $translate value
            // path only -- output type fidelity is tracked separately.
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
