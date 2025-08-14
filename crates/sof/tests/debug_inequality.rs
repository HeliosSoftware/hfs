use helios_fhirpath::{EvaluationContext, evaluate_expression};

#[test]
fn debug_inequality_operators() {
    // Create test data - Observation with valueInteger: 12
    let observation_json = serde_json::json!({
        "resourceType": "Observation",
        "id": "o1",
        "valueInteger": 12
    });

    // Parse into FHIR resource
    let observation: helios_fhir::r4::Observation =
        serde_json::from_value(observation_json).expect("Failed to parse observation");

    // Create context with the observation
    let context = EvaluationContext::new(vec![helios_fhir::FhirResource::R4(Box::new(
        helios_fhir::r4::Resource::Observation(observation),
    ))]);

    // Test parsing and evaluating the expressions
    println!("Testing comparison operators with Observation (valueInteger: 12)");

    // Test simple expressions first
    test_expression("valueInteger", &context);
    test_expression("value", &context);
    test_expression("value.ofType(integer)", &context);
    test_expression("value.ofType(Integer)", &context); // Try capitalized version
    test_expression("value.ofType(System.Integer)", &context); // Try explicit System
    test_expression("value.ofType(FHIR.integer)", &context); // Try explicit FHIR
    test_expression("value.is(integer)", &context); // Test 'is' function
    test_expression("value.is(Integer)", &context); // Test 'is' with capitalized
    test_expression("valueInteger > 11", &context);
    test_expression("value.ofType(integer) > 11", &context);

    // Test where expressions
    test_expression("where(valueInteger > 11)", &context);
    test_expression("where(value.ofType(integer) > 11)", &context);
    test_expression("where(value.ofType(integer) > 11).exists()", &context);

    // Test the exact expressions from the failing test
    test_expression("where(valueInteger < 11)", &context);
    test_expression("where(value.ofType(integer) < 11)", &context);
    test_expression("where(value.ofType(integer) < 11).exists()", &context);
}

fn test_expression(expr_str: &str, context: &EvaluationContext) {
    println!("\nTesting: {}", expr_str);

    match evaluate_expression(expr_str, context) {
        Ok(result) => {
            println!("  Result: {:?}", result);
        }
        Err(e) => {
            println!("  Error: {}", e);
        }
    }
}
