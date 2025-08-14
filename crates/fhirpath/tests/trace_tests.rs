use helios_fhirpath::{EvaluationContext, evaluate_expression};
use helios_fhirpath_support::EvaluationResult;

#[test]
fn test_trace_function() {
    let context = EvaluationContext::new_empty_with_default_version();

    // Basic test cases
    let trace_cases = vec![
        // Basic trace with literal
        ("1.trace('test')", EvaluationResult::integer(1)),
        // Trace with a chain
        (
            "1.trace('first').trace('second')",
            EvaluationResult::integer(1),
        ),
        // Trace with a collection
        (
            "(1 | 2 | 3).trace('collection')",
            EvaluationResult::Collection {
                items: vec![
                    EvaluationResult::integer(1),
                    EvaluationResult::integer(2),
                    EvaluationResult::integer(3),
                ],
                has_undefined_order: true,
                type_info: None,
            },
        ), // Union operator implies undefined order
        // Trace with a projection (second parameter)
        (
            "(1 | 2 | 3).trace('projection', $this + 1)",
            EvaluationResult::Collection {
                items: vec![
                    EvaluationResult::integer(1),
                    EvaluationResult::integer(2),
                    EvaluationResult::integer(3),
                ],
                has_undefined_order: true,
                type_info: None,
            },
        ), // Input collection from union has undefined order
    ];

    // Run test cases
    for (expr, expected) in trace_cases {
        println!("Testing: {}", expr);

        let result = evaluate_expression(expr, &context).unwrap();

        assert_eq!(result, expected, "Expression: {}", expr);
    }

    // Test error cases
    let error_cases = vec![
        // Missing the required name parameter
        "1.trace()",
        // Name parameter is not a string
        "1.trace(123)",
    ];

    for expr in error_cases {
        println!("Testing error case: {}", expr);

        let result = evaluate_expression(expr, &context);

        assert!(result.is_err(), "Expected error for expression: {}", expr);
    }
}
