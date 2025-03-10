use fhirpath::evaluator::{evaluate, EvaluationContext, EvaluationResult};
use fhirpath::parser::parser;
use std::collections::HashMap;

#[test]
fn test_simple_literals() {
    let test_cases = vec![
        ("true", EvaluationResult::Boolean(true)),
        ("false", EvaluationResult::Boolean(false)),
        ("123", EvaluationResult::Number(123.0)),
        ("'hello'", EvaluationResult::String("hello".to_string())),
    ];

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        let context = EvaluationContext::new(EvaluationResult::Empty);
        let result = evaluate(&expr, &context);
        assert_eq!(result, expected);
    }
}

#[test]
fn test_arithmetic_operations() {
    let test_cases = vec![
        ("1 + 2", EvaluationResult::Number(3.0)),
        ("5 - 3", EvaluationResult::Number(2.0)),
        ("2 * 3", EvaluationResult::Number(6.0)),
        ("6 / 2", EvaluationResult::Number(3.0)),
        ("7 div 2", EvaluationResult::Integer(3)),
        ("7 mod 2", EvaluationResult::Number(1.0)),
    ];

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        let context = EvaluationContext::new(EvaluationResult::Empty);
        let result = evaluate(&expr, &context);
        assert_eq!(result, expected);
    }
}

#[test]
fn test_boolean_operations() {
    let test_cases = vec![
        ("true and true", EvaluationResult::Boolean(true)),
        ("true and false", EvaluationResult::Boolean(false)),
        ("true or false", EvaluationResult::Boolean(true)),
        ("false or false", EvaluationResult::Boolean(false)),
        ("true xor false", EvaluationResult::Boolean(true)),
        ("true xor true", EvaluationResult::Boolean(false)),
        ("false implies true", EvaluationResult::Boolean(true)),
        ("true implies false", EvaluationResult::Boolean(false)),
    ];

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        let context = EvaluationContext::new(EvaluationResult::Empty);
        let result = evaluate(&expr, &context);
        assert_eq!(result, expected);
    }
}

#[test]
fn test_comparison_operations() {
    let test_cases = vec![
        ("1 < 2", EvaluationResult::Boolean(true)),
        ("2 <= 2", EvaluationResult::Boolean(true)),
        ("3 > 2", EvaluationResult::Boolean(true)),
        ("3 >= 3", EvaluationResult::Boolean(true)),
        ("1 = 1", EvaluationResult::Boolean(true)),
        ("1 != 2", EvaluationResult::Boolean(true)),
        ("'abc' ~ 'ABC'", EvaluationResult::Boolean(true)),
        ("'abc' !~ 'def'", EvaluationResult::Boolean(true)),
    ];

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        let context = EvaluationContext::new(EvaluationResult::Empty);
        let result = evaluate(&expr, &context);
        assert_eq!(result, expected);
    }
}

#[test]
fn test_object_access() {
    // Create a simple object to test property access
    let mut patient = HashMap::new();
    patient.insert(
        "name".to_string(),
        EvaluationResult::String("John Doe".to_string()),
    );
    patient.insert(
        "age".to_string(),
        EvaluationResult::Integer(42),
    );
    
    let context = EvaluationContext::new(EvaluationResult::Object(patient));
    
    let test_cases = vec![
        ("name", EvaluationResult::String("John Doe".to_string())),
        ("age", EvaluationResult::Integer(42)),
        ("address", EvaluationResult::Empty), // Non-existent property
    ];

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        let result = evaluate(&expr, &context);
        assert_eq!(result, expected);
    }
}

#[test]
fn test_functions() {
    // Test collection functions
    let mut items = Vec::new();
    items.push(EvaluationResult::Integer(1));
    items.push(EvaluationResult::Integer(2));
    items.push(EvaluationResult::Integer(3));
    
    let context = EvaluationContext::new(EvaluationResult::Collection(items));
    
    let test_cases = vec![
        ("count()", EvaluationResult::Integer(3)),
        ("empty()", EvaluationResult::Boolean(false)),
        ("exists()", EvaluationResult::Boolean(true)),
        ("first()", EvaluationResult::Integer(1)),
        ("last()", EvaluationResult::Integer(3)),
    ];

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        let result = evaluate(&expr, &context);
        assert_eq!(result, expected);
    }
}
