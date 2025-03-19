use fhirpath::evaluator::{evaluate, EvaluationContext, EvaluationResult, FhirResource};
use fhirpath::parser::parser;
use std::collections::HashMap;
use chumsky::Parser;
use fhir::r4;

#[test]
fn test_simple_literals() {
    let test_cases = vec![
        ("true", EvaluationResult::Boolean(true)),
        ("false", EvaluationResult::Boolean(false)),
        ("123", EvaluationResult::Number(123.0)),
        ("'hello'", EvaluationResult::String("hello".to_string())),
    ];

    // Create a dummy R4 resource for testing
    let dummy_resource = r4::Resource::Account(r4::Account {
        id: None,
        meta: None,
        implicitly_included: false,
    });
    
    let context = EvaluationContext::new(FhirResource::R4(Box::new(dummy_resource)));

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

    // Create a dummy R4 resource for testing
    let dummy_resource = r4::Resource::Account(r4::Account {
        id: None,
        meta: None,
        implicitly_included: false,
    });
    
    let context = EvaluationContext::new(FhirResource::R4(Box::new(dummy_resource)));

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

    // Create a dummy R4 resource for testing
    let dummy_resource = r4::Resource::Account(r4::Account {
        id: None,
        meta: None,
        implicitly_included: false,
    });
    
    let context = EvaluationContext::new(FhirResource::R4(Box::new(dummy_resource)));

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

    // Create a dummy R4 resource for testing
    let dummy_resource = r4::Resource::Account(r4::Account {
        id: None,
        meta: None,
        implicitly_included: false,
    });
    
    let context = EvaluationContext::new(FhirResource::R4(Box::new(dummy_resource)));

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        let result = evaluate(&expr, &context);
        assert_eq!(result, expected);
    }
}

#[test]
fn test_object_access() {
    // Create a dummy R4 resource for testing
    let dummy_resource = r4::Resource::Account(r4::Account {
        id: None,
        meta: None,
        implicitly_included: false,
    });
    
    // We'll set up the context with our resource
    let mut context = EvaluationContext::new(FhirResource::R4(Box::new(dummy_resource)));
    
    // For testing property access, we'll add some variables to the context
    let mut patient = HashMap::new();
    patient.insert(
        "name".to_string(),
        EvaluationResult::String("John Doe".to_string()),
    );
    patient.insert(
        "age".to_string(),
        EvaluationResult::Integer(42),
    );
    
    context.set_variable("patient", EvaluationResult::Object(patient));
    
    let test_cases = vec![
        // Access through the variable
        ("patient.name", EvaluationResult::String("John Doe".to_string())),
        ("patient.age", EvaluationResult::Integer(42)),
        ("patient.address", EvaluationResult::Empty), // Non-existent property
    ];

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        let result = evaluate(&expr, &context);
        assert_eq!(result, expected);
    }
}

#[test]
fn test_functions() {
    // Create a dummy R4 resource for testing
    let dummy_resource = r4::Resource::Account(r4::Account {
        id: None,
        meta: None,
        implicitly_included: false,
    });
    
    // We'll set up the context with our resource
    let mut context = EvaluationContext::new(FhirResource::R4(Box::new(dummy_resource)));
    
    // For testing collection functions, we'll add a collection variable to the context
    let mut items = Vec::new();
    items.push(EvaluationResult::Integer(1));
    items.push(EvaluationResult::Integer(2));
    items.push(EvaluationResult::Integer(3));
    
    context.set_variable("numbers", EvaluationResult::Collection(items));
    
    let test_cases = vec![
        ("numbers.count()", EvaluationResult::Integer(3)),
        ("numbers.empty()", EvaluationResult::Boolean(false)),
        ("numbers.exists()", EvaluationResult::Boolean(true)),
        ("numbers.first()", EvaluationResult::Integer(1)),
        ("numbers.last()", EvaluationResult::Integer(3)),
    ];

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        let result = evaluate(&expr, &context);
        assert_eq!(result, expected);
    }
}
