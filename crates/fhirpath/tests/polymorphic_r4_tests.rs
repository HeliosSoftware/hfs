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

/// Builds an R4 evaluation context from inline resource JSON.
fn context_from_json(json: serde_json::Value) -> EvaluationContext {
    let resource: r4::Resource = serde_json::from_value(json).expect("valid R4 resource");
    EvaluationContext::new(vec![helios_fhir::FhirResource::R4(Box::new(resource))])
}

/// Issue #2477: an absent element must not resolve to a camelCase sibling.
///
/// Choice lookup used to fire for any missing member, matching every field
/// named `<name><Uppercase>…`, so `conclusion` returned `conclusionCode`,
/// `status` returned `statusReason`, and the non-element `birth` returned
/// `birthDate`. Only declared `[x]` elements may resolve through a typed
/// variant.
#[test]
fn missing_element_does_not_resolve_to_a_camel_case_sibling() {
    let report = context_from_json(serde_json::json!({
        "resourceType": "DiagnosticReport",
        "id": "mri",
        "status": "final",
        "code": {"text": "MRI HEAD ORBITS"},
        "conclusionCode": [
            {"coding": [{"system": "http://snomed.info/sct", "code": "128208008"}]},
            {"coding": [{"system": "http://snomed.info/sct", "code": "1234"}]}
        ]
    }));
    assert_eq!(
        run_expression(&report, "conclusion"),
        EvaluationResult::Empty
    );
    assert_eq!(
        run_expression(&report, "DiagnosticReport.conclusion"),
        EvaluationResult::Empty
    );
    assert_eq!(
        run_expression(&report, "conclusion.exists()"),
        EvaluationResult::boolean(false)
    );
    // Control: the sibling itself is still reachable by its own name.
    assert_eq!(
        run_expression(&report, "conclusionCode.count()"),
        EvaluationResult::integer(2)
    );

    let patient = context_from_json(serde_json::json!({
        "resourceType": "Patient",
        "id": "p",
        "birthDate": "2000-01-01"
    }));
    assert_eq!(run_expression(&patient, "birth"), EvaluationResult::Empty);

    let no_status = context_from_json(serde_json::json!({
        "resourceType": "Procedure",
        "id": "proc",
        "subject": {"reference": "Patient/p"},
        "statusReason": {"text": "patient declined"}
    }));
    assert_eq!(
        run_expression(&no_status, "status"),
        EvaluationResult::Empty
    );
}

/// Control for #2477: real choice elements still resolve by their base name.
#[test]
fn declared_choice_elements_still_resolve() {
    let patient = context_from_json(serde_json::json!({
        "resourceType": "Patient",
        "id": "p",
        "deceasedBoolean": true,
        "multipleBirthInteger": 2
    }));
    assert_eq!(
        run_expression(&patient, "deceased"),
        EvaluationResult::boolean(true)
    );
    assert_eq!(
        run_expression(&patient, "multipleBirth"),
        EvaluationResult::integer(2)
    );

    let obs = context_from_json(serde_json::json!({
        "resourceType": "Observation",
        "id": "o",
        "status": "final",
        "code": {"text": "x"},
        "effectiveDateTime": "2020-01-01",
        "component": [{"code": {"text": "c"}, "valueString": "nested"}]
    }));
    assert_eq!(
        run_expression(&obs, "effective.exists()"),
        EvaluationResult::boolean(true)
    );
    assert_eq!(
        run_expression(&obs, "component.value"),
        EvaluationResult::string("nested".to_string())
    );
}
