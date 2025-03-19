use chumsky::Parser;
use fhir::FhirResource;
use fhirpath::evaluator::{EvaluationContext, EvaluationResult, evaluate};
use fhirpath::parser::parser;

#[test]
fn test_simple_literals() {
    let test_cases = vec![
        ("true", EvaluationResult::Boolean(true)),
        ("false", EvaluationResult::Boolean(false)),
        ("123", EvaluationResult::Number(123.0)),
        ("'hello'", EvaluationResult::String("hello".to_string())),
    ];

    // For simple literals, we don't need any resources
    let context = EvaluationContext::new_empty();

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        println!("Parsed expression: {:?}", expr);
        let result = evaluate(&expr, &context);
        println!("Result: {:?}, Expected: {:?}", result, expected);
        assert_eq!(result, expected, "Failed for input: {}", input);
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

    // For arithmetic operations, we don't need any resources
    let context = EvaluationContext::new_empty();

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
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

    // For boolean operations, we don't need any resources
    let context = EvaluationContext::new_empty();

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        println!("Boolean op parsed expression: {:?}", expr);
        let result = evaluate(&expr, &context);
        println!("Boolean op result: {:?}, Expected: {:?}", result, expected);
        assert_eq!(result, expected, "Failed for input: {}", input);
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

    // For comparison operations, we don't need any resources
    let context = EvaluationContext::new_empty();

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        let result = evaluate(&expr, &context);
        assert_eq!(result, expected);
    }
}

#[test]
fn test_variable_access() {
    // We'll set up the context without any resources
    let mut context = EvaluationContext::new_empty();

    // For testing variable access, we'll add some variables to the context
    context.set_variable("name", "John Doe".to_string());
    context.set_variable("age", "42".to_string());

    let test_cases = vec![
        // Access variables directly
        ("%name", EvaluationResult::String("John Doe".to_string())),
        ("%age", EvaluationResult::String("42".to_string())),
        ("%address", EvaluationResult::Empty), // Non-existent variable
    ];

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        let result = evaluate(&expr, &context);
        assert_eq!(result, expected);
    }
}

#[test]
fn test_functions() {
    // We'll set up the context without any resources
    let mut context = EvaluationContext::new_empty();

    // For testing string functions, we'll add a string variable
    context.set_variable("message", "Hello, World!".to_string());

    let test_cases = vec![
        // String functions
        ("%message.length()", EvaluationResult::Integer(13)),
        (
            "%message.substring(0, 5)",
            EvaluationResult::String("Hello".to_string()),
        ),
        (
            "%message.contains('World')",
            EvaluationResult::Boolean(true),
        ),
    ];

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        let result = evaluate(&expr, &context);
        assert_eq!(result, expected);
    }
}
#[test]
fn test_resource_access() {
    use fhir::r4;
    // Create a dummy R4 resource for testing
    let dummy_resource = r4::Resource::Account(r4::Account {
        id: None,
        meta: None,
    });

    // Create a context with a resource
    let mut resources = Vec::new();
    resources.push(FhirResource::R4(Box::new(dummy_resource)));
    let context = EvaluationContext::new(resources);
    // Test accessing the resource type
    let expr = parser().parse("resourceType").unwrap();
    let result = evaluate(&expr, &context);
    assert_eq!(result, EvaluationResult::String("R4Resource".to_string()));
}
