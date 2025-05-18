use chumsky::Parser;
use fhirpath::evaluator::{EvaluationContext, evaluate};
use fhirpath::parser::parser;
use fhirpath_support::EvaluationResult;

#[test]
fn test_trace_function() {
    let context = EvaluationContext::new_empty();

    // Basic test cases
    let trace_cases = vec![
        // Basic trace with literal
        ("1.trace('test')", EvaluationResult::Integer(1)),
        // Trace with a chain
        ("1.trace('first').trace('second')", EvaluationResult::Integer(1)),
        // Trace with a collection
        ("(1 | 2 | 3).trace('collection')", EvaluationResult::Collection { items: vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2),
            EvaluationResult::Integer(3),
        ], has_undefined_order: false }),
        // Trace with a projection (second parameter)
        ("(1 | 2 | 3).trace('projection', $this + 1)", EvaluationResult::Collection { items: vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2),
            EvaluationResult::Integer(3),
        ], has_undefined_order: false }),
    ];

    // Run test cases
    for (expr, expected) in trace_cases {
        println!("Testing: {}", expr);
        
        let parsed = parser().parse(expr).unwrap();
        let result = evaluate(&parsed, &context, None).unwrap();
        
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
        
        let parsed = parser().parse(expr).unwrap();
        let result = evaluate(&parsed, &context, None);
        
        assert!(result.is_err(), "Expected error for expression: {}", expr);
    }
}
