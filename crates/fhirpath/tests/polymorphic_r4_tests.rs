use helios_fhir::r4;
use helios_fhirpath::{EvaluationContext, evaluate_expression};
use helios_fhirpath_support::EvaluationResult;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

/// Tests for polymorphic access to FHIR choice elements in R4 resources
/// This tests that expressions like "value.unit" correctly resolve to "valueQuantity.unit"
/// in observations with a valueQuantity element.
#[test]
fn test_polymorphic_value_unit() {
    // Load the Observation example
    let context = load_test_resource("observation-example.json").expect("Failed to load resource");

    // First verify simple path access works
    let expr_direct = "value.unit";
    let result_direct = run_expression(&context, expr_direct);
    assert_eq!(result_direct, EvaluationResult::string("lbs".to_string()));

    // For now, we use %context to work around the resource type prefix issue
    let expr_context = "%context.value.unit";
    let result_context = run_expression(&context, expr_context);
    assert_eq!(result_context, EvaluationResult::string("lbs".to_string()));
}

/// Tests that the 'is' operator correctly identifies choice element types
#[test]
fn test_polymorphic_is_quantity() {
    // Load the Observation example
    let context = load_test_resource("observation-example.json").expect("Failed to load resource");

    // Test using context to access value - use operator syntax
    let expr = "%context.value is Quantity";
    let result = run_expression(&context, expr);
    assert_eq!(result, EvaluationResult::boolean(true));

    // Also test negative case
    let expr_neg = "%context.value is Period";
    let result_neg = run_expression(&context, expr_neg);
    assert_eq!(result_neg, EvaluationResult::boolean(false));
}

/// Tests that the 'as' operator correctly filters choice elements by type
#[test]
fn test_polymorphic_as_quantity() {
    // Load the Observation example
    let context = load_test_resource("observation-example.json").expect("Failed to load resource");

    // Test with context
    let expr = "%context.value.as(Quantity).unit";
    let result = run_expression(&context, expr);
    assert_eq!(result, EvaluationResult::string("lbs".to_string()));
}

/// Evaluates a FHIRPath expression and returns the result
fn run_expression(context: &EvaluationContext, expression: &str) -> EvaluationResult {
    evaluate_expression(expression, context).expect("Failed to evaluate expression")
}

/// Loads a FHIR R4 resource from a JSON test file and creates an evaluation context
fn load_test_resource(json_filename: &str) -> Result<EvaluationContext, String> {
    // Get the path to the JSON file
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push(format!("tests/data/r4/input/{}", json_filename));

    // Load the JSON file
    let mut file =
        File::open(&path).map_err(|e| format!("Could not open JSON resource file: {:?}", e))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|e| format!("Failed to read JSON resource file: {:?}", e))?;

    // Parse the JSON into a FHIR resource
    let resource: r4::Resource =
        serde_json::from_str(&contents).map_err(|e| format!("Failed to parse JSON: {:?}", e))?;

    // Create an evaluation context with the resource
    let context = EvaluationContext::new(vec![helios_fhir::FhirResource::R4(Box::new(resource))]);
    Ok(context)
}
