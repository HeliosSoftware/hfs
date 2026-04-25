use axum::Json;
use helios_fhirpath::handlers::evaluate_fhirpath;
use helios_fhirpath::models::FhirPathParameters;
use serde_json::json;

#[tokio::test]
async fn test_context_with_trace_function() {
    // Create the test input matching the example
    let params = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "expression",
                "valueString": "trace('trc').given.join(' ').combine(family).join(', ')"
            },
            {
                "name": "context",
                "valueString": "name"
            },
            {
                "name": "validate",
                "valueBoolean": true
            },
            {
                "name": "variables"
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "example",
                    "name": [
                        {
                            "use": "official",
                            "family": "Chalmers",
                            "given": ["Peter", "James"]
                        },
                        {
                            "use": "usual",
                            "given": ["Jim"]
                        },
                        {
                            "use": "maiden",
                            "family": "Windsor",
                            "given": ["Peter", "James"],
                            "period": {
                                "end": "2002"
                            }
                        }
                    ]
                }
            }
        ]
    });

    // Convert to FhirPathParameters
    let fhir_params: FhirPathParameters = serde_json::from_value(params).unwrap();

    // Call the handler
    let response = evaluate_fhirpath(Json(fhir_params)).await;

    // Check that we got a successful response
    assert!(response.is_ok(), "Response should be successful");

    // Extract the response body
    let response = response.unwrap();
    let body = response.into_body();

    // The response should contain results for each name
    // For now, just verify we don't get an error
    println!("Response: {:?}", body);
}

#[test]
fn test_trace_with_context_full_expression() {
    use helios_fhir::FhirResource;
    use helios_fhir::r4::Patient;
    use helios_fhirpath::{EvaluationContext, evaluate_expression};

    // Create patient matching the user's example
    let patient_json = json!({
        "resourceType": "Patient",
        "id": "example",
        "name": [
            {
                "use": "official",
                "family": "Chalmers",
                "given": ["Peter", "James"]
            },
            {
                "use": "usual",
                "given": ["Jim"]
            },
            {
                "use": "maiden",
                "family": "Windsor",
                "given": ["Peter", "James"],
                "period": {
                    "end": "2002"
                }
            }
        ]
    });

    let patient: Patient = serde_json::from_value(patient_json.clone()).unwrap();
    let patient_resource: helios_fhir::r4::Resource =
        helios_fhir::r4::Resource::Patient(Box::new(patient));
    let resource = FhirResource::R4(Box::new(patient_resource));

    let context = EvaluationContext::new(vec![resource]);

    // First evaluate the context expression
    let context_result = evaluate_expression("name", &context).unwrap();

    // The full expression from the user
    let expression = "trace('trc').given.join(' ').combine(family).join(', ')";

    // Parse and evaluate with context
    use chumsky::Parser;
    let parsed = helios_fhirpath::parser::parser()
        .parse(expression)
        .into_result()
        .unwrap();

    // Get the name items
    let name_items = match context_result {
        helios_fhirpath::EvaluationResult::Collection { items, .. } => items,
        single => vec![single],
    };

    // Expected results
    let expected_results = ["Peter James, Chalmers", "Jim", "Peter James, Windsor"];

    for (i, name_item) in name_items.iter().enumerate() {
        println!("Processing name[{}]", i);
        let result =
            helios_fhirpath::evaluator::evaluate(&parsed, &context, Some(name_item)).unwrap();
        println!("Result: {:?}", result);

        // Should match expected
        match result {
            helios_fhirpath::EvaluationResult::String(s, _) => {
                assert_eq!(
                    s, expected_results[i],
                    "Result for name[{}] doesn't match",
                    i
                );
            }
            _ => panic!("Expected string result for name[{}]", i),
        }
    }
}

#[test]
fn test_trace_with_context_simple() {
    use helios_fhir::FhirResource;
    use helios_fhir::r4::Patient;
    use helios_fhirpath::{EvaluationContext, evaluate_expression};

    // Create a simple patient with name
    let patient_json = json!({
        "resourceType": "Patient",
        "name": [{
            "family": "Doe",
            "given": ["John"]
        }]
    });

    let patient: Patient = serde_json::from_value(patient_json.clone()).unwrap();
    let patient_resource: helios_fhir::r4::Resource =
        helios_fhir::r4::Resource::Patient(Box::new(patient));
    let resource = FhirResource::R4(Box::new(patient_resource));

    let context = EvaluationContext::new(vec![resource]);

    // First evaluate the context expression
    let context_result = evaluate_expression("name", &context).unwrap();
    println!("Context result: {:?}", context_result);

    // This should now work with our fix
    let expression = "trace('test').family";

    // Parse and evaluate with context
    use chumsky::Parser;
    let parsed = helios_fhirpath::parser::parser()
        .parse(expression)
        .into_result()
        .unwrap();

    // Get the first name as context
    let name_items = match context_result {
        helios_fhirpath::EvaluationResult::Collection { items, .. } => items,
        single => vec![single],
    };

    for name_item in &name_items {
        let result =
            helios_fhirpath::evaluator::evaluate(&parsed, &context, Some(name_item)).unwrap();
        println!("Result with context: {:?}", result);

        // Should contain "Doe"
        match result {
            helios_fhirpath::EvaluationResult::String(s, _) => {
                assert_eq!(s, "Doe");
            }
            _ => panic!("Expected string result"),
        }
    }
}
