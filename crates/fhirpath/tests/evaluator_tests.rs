use chumsky::Parser;
use fhir::FhirResource;
use fhirpath::evaluator::{EvaluationContext, evaluate};
use fhirpath::parser::parser;
use fhirpath_support::EvaluationResult;

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
        println!("Variable access parsed expression: {:?}", expr);
        let result = evaluate(&expr, &context);
        println!(
            "Variable access result: {:?}, Expected: {:?}",
            result, expected
        );
        assert_eq!(result, expected, "Failed for input: {}", input);
    }
}

#[test]
fn test_string_operations() {
    // We'll set up the context without any resources
    let mut context = EvaluationContext::new_empty();

    // For testing string operations, we'll add a string variable
    context.set_variable("message", "Hello, World!".to_string());

    let test_cases = vec![
        // String contains operation with function call syntax
        (
            "'Hello, World!'.contains('World')",
            EvaluationResult::Boolean(true),
        ),
        (
            "'Hello, World!'.contains('Goodbye')",
            EvaluationResult::Boolean(false),
        ),
        (
            "%message.contains('World')",
            EvaluationResult::Boolean(true),
        ),
    ];

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        println!("String operation parsed expression: {:?}", expr);
        let result = evaluate(&expr, &context);
        println!(
            "String operation result: {:?}, Expected: {:?}",
            result, expected
        );
        assert_eq!(result, expected, "Failed for input: {}", input);
    }
}

#[test]
fn test_functions() {
    // We'll set up the context without any resources
    let context = EvaluationContext::new_empty();

    // Test collection functions
    let test_cases = vec![
        // Empty collection
        (
            "{}".to_string(),
            "count()".to_string(),
            EvaluationResult::Integer(0),
        ),
        (
            "{}".to_string(),
            "empty()".to_string(),
            EvaluationResult::Boolean(true),
        ),
        (
            "{}".to_string(),
            "exists()".to_string(),
            EvaluationResult::Boolean(false),
        ),
        // Single item
        (
            "'test'".to_string(),
            "count()".to_string(),
            EvaluationResult::Integer(1),
        ),
        (
            "'test'".to_string(),
            "empty()".to_string(),
            EvaluationResult::Boolean(false),
        ),
        (
            "'test'".to_string(),
            "exists()".to_string(),
            EvaluationResult::Boolean(true),
        ),
        // String functions
        (
            "'Hello'".to_string(),
            "count()".to_string(),
            EvaluationResult::Integer(1),
        ),
        (
            "'Hello'".to_string(),
            "length()".to_string(),
            EvaluationResult::Integer(5),
        ),
        (
            "'Hello, World!'".to_string(),
            "contains('World')".to_string(),
            EvaluationResult::Boolean(true),
        ),
        (
            "'Hello, World!'".to_string(),
            "contains('Goodbye')".to_string(),
            EvaluationResult::Boolean(false),
        ),
    ];

    for (base, func, expected) in test_cases {
        let full_expr = if base == "{}" {
            func.clone()
        } else {
            format!("{}.{}", base, func)
        };

        println!("Testing expression: {}", full_expr);
        let expr = parser().parse(&*full_expr).unwrap();
        let result = evaluate(&expr, &context);
        println!("Function result: {:?}, Expected: {:?}", result, expected);
        assert_eq!(result, expected, "Failed for input: {}", full_expr);
    }
}
#[test]
fn test_direct_string_operations() {
    // We'll set up the context without any resources
    let context = EvaluationContext::new_empty();

    // Test string operations through the parser instead of direct function calls
    let expr = parser().parse("'Hello, World!'.contains('World')").unwrap();
    let result = evaluate(&expr, &context);
    assert_eq!(result, EvaluationResult::Boolean(true));

    let expr = parser()
        .parse("'Hello, World!'.contains('Goodbye')")
        .unwrap();
    let result = evaluate(&expr, &context);
    assert_eq!(result, EvaluationResult::Boolean(false));
}

#[test]
fn test_resource_access() {
    use fhir::r4;
    // Create a dummy R4 resource for testing
    let dummy_resource = r4::Resource::Account(r4::Account {
        id: Some("theid".to_string().into()), // Convert String to Id
        meta: None,
        implicit_rules: None,
        language: None,
        text: None,
        contained: None,
        extension: None,
        modifier_extension: None,
        identifier: None,
        status: r4::Code {
            id: None,
            extension: None,
            value: None,
        },
        r#type: None,
        name: None,
        subject: None,
        service_period: None,
        coverage: None,
        owner: None,
        description: None,
        guarantor: None,
        part_of: None,
    });

    // Create a context with a resource
    let mut resources = Vec::new();
    resources.push(FhirResource::R4(Box::new(dummy_resource)));
    let context = EvaluationContext::new(resources);
    // Test accessing the resource type
    let expr = parser().parse("theid").unwrap();
    let result = evaluate(&expr, &context);
    assert_eq!(result, EvaluationResult::String("theid".to_string()));
}
