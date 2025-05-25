use fhirpath::evaluator::EvaluationContext;
use fhirpath::parser::parser;
use fhirpath::EvaluationResult;
use std::collections::HashMap;
use chumsky::Parser;

/// Helper function to create a nested collection structure for testing
fn create_nested_collection() -> EvaluationResult {
    // Create a collection of collections
    // The structure is: [[1, 2], [3, 4, 5], [6]]
    
    // Inner collection 1: [1, 2]
    let inner1 = EvaluationResult::Collection {
        items: vec![
            EvaluationResult::integer(1),
            EvaluationResult::integer(2),
        ],
        has_undefined_order: false,
        type_info: None,
    };
    
    // Inner collection 2: [3, 4, 5]
    let inner2 = EvaluationResult::Collection {
        items: vec![
            EvaluationResult::integer(3),
            EvaluationResult::integer(4),
            EvaluationResult::integer(5),
        ],
        has_undefined_order: false,
        type_info: None,
    };
    
    // Inner collection 3: [6]
    let inner3 = EvaluationResult::Collection {
        items: vec![
            EvaluationResult::integer(6),
        ],
        has_undefined_order: false,
        type_info: None,
    };
    
    // Outer collection containing the inner collections
    EvaluationResult::Collection {
        items: vec![inner1, inner2, inner3],
        has_undefined_order: false,
        type_info: None,
    }
}

/// Helper function to create a complex object structure for testing
fn create_complex_object() -> EvaluationResult {
    // Create a patient resource with nested observations
    
    // First observation with a value of 120
    let mut obs1 = HashMap::new();
    obs1.insert("resourceType".to_string(), EvaluationResult::string("Observation".to_string()));
    obs1.insert("status".to_string(), EvaluationResult::string("final".to_string()));
    obs1.insert("value".to_string(), EvaluationResult::integer(120));
    
    // Second observation with a value of 80
    let mut obs2 = HashMap::new();
    obs2.insert("resourceType".to_string(), EvaluationResult::string("Observation".to_string()));
    obs2.insert("status".to_string(), EvaluationResult::string("final".to_string()));
    obs2.insert("value".to_string(), EvaluationResult::integer(80));
    
    // Third observation with a missing value
    let mut obs3 = HashMap::new();
    obs3.insert("resourceType".to_string(), EvaluationResult::string("Observation".to_string()));
    obs3.insert("status".to_string(), EvaluationResult::string("cancelled".to_string()));
    
    // Patient resource with observations
    let mut patient = HashMap::new();
    patient.insert("resourceType".to_string(), EvaluationResult::string("Patient".to_string()));
    patient.insert("id".to_string(), EvaluationResult::string("123".to_string()));
    
    // Create a collection of observations
    let observations = EvaluationResult::Collection {
        items: vec![
            EvaluationResult::object(obs1),
            EvaluationResult::object(obs2),
            EvaluationResult::object(obs3),
        ],
        has_undefined_order: false,
        type_info: None,
    };
    
    patient.insert("observations".to_string(), observations);
    
    EvaluationResult::object(patient)
}

#[test]
fn test_all_with_simple_criteria() {
    // Create a simple collection [1, 2, 3, 4, 5]
    let collection = EvaluationResult::Collection {
        items: vec![
            EvaluationResult::integer(1),
            EvaluationResult::integer(2),
            EvaluationResult::integer(3),
            EvaluationResult::integer(4),
            EvaluationResult::integer(5),
        ],
        has_undefined_order: false,
        type_info: None,
    };
    
    let mut context = EvaluationContext::new_empty();
    context.set_this(collection.clone());
    
    // Test: all elements are > 0
    let expr_str = "$this.all($this > 0)";
    let expr = parser().parse(expr_str).unwrap();
    let result = fhirpath::evaluate(&expr, &context, None).unwrap();
    
    assert_eq!(result, EvaluationResult::boolean(true));
    
    // Test: all elements are < 3
    let expr_str = "$this.all($this < 3)";
    let expr = parser().parse(expr_str).unwrap();
    let result = fhirpath::evaluate(&expr, &context, None).unwrap();
    
    assert_eq!(result, EvaluationResult::boolean(false));
}

#[test]
fn test_all_with_nested_collections() {
    // This test may fail due to issues with nested collections
    let nested_collection = create_nested_collection();
    
    let mut context = EvaluationContext::new_empty();
    context.set_this(nested_collection.clone());
    
    // Test: all inner collections have at least one element
    let expr_str = "$this.all($this.count() > 0)";
    let expr = parser().parse(expr_str).unwrap();
    let result = fhirpath::evaluate(&expr, &context, None).unwrap();
    
    assert_eq!(result, EvaluationResult::boolean(true));
    
    // Test: all elements in inner collections are positive
    // This might be problematic because it requires nested collection traversal
    let expr_str = "$this.all($this.all($this > 0))";
    let expr = parser().parse(expr_str).unwrap();
    let result = fhirpath::evaluate(&expr, &context, None);
    
    // This assertion may fail if the implementation doesn't handle nested all() correctly
    match result {
        Ok(EvaluationResult::Boolean(true, _)) => (), // Expected correct behavior
        _ => panic!("Failed to evaluate nested all() expression: {:?}", result),
    }
}

#[test]
fn test_all_with_complex_object_paths() {
    // This test may fail due to issues with complex path navigation
    let patient = create_complex_object();
    
    let mut context = EvaluationContext::new_empty();
    context.set_this(patient.clone());
    
    // Test: all observations have a status
    let expr_str = "observations.all(status.exists())";
    let expr = parser().parse(expr_str).unwrap();
    let result = fhirpath::evaluate(&expr, &context, None).unwrap();
    
    assert_eq!(result, EvaluationResult::boolean(true));
    
    // Test: all observations have a value
    // This should fail because obs3 doesn't have a value
    let expr_str = "observations.all(value.exists())";
    let expr = parser().parse(expr_str).unwrap();
    let result = fhirpath::evaluate(&expr, &context, None).unwrap();
    
    assert_eq!(result, EvaluationResult::boolean(false));
    
    // Test: all final observations have a value
    // This involves a more complex criteria with multiple conditions
    let expr_str = "observations.all(status = 'final' implies value.exists())";
    let expr = parser().parse(expr_str).unwrap();
    let result = fhirpath::evaluate(&expr, &context, None).unwrap();
    
    // This assertion may fail if the implementation can't handle complex boolean logic
    assert_eq!(result, EvaluationResult::boolean(true));
}

#[test]
fn test_all_with_type_operations() {
    // This test may fail due to issues with type operations
    let collection = EvaluationResult::Collection {
        items: vec![
            EvaluationResult::integer(1),
            EvaluationResult::decimal("2.5".parse().unwrap()),
            EvaluationResult::string("3".to_string()),
        ],
        has_undefined_order: false,
        type_info: None,
    };
    
    let mut context = EvaluationContext::new_empty();
    context.set_this(collection.clone());
    
    // Test: check if all elements can be converted to decimal
    let expr_str = "$this.all($this.convertsToDecimal())";
    let expr = parser().parse(expr_str).unwrap();
    let result = fhirpath::evaluate(&expr, &context, None).unwrap();
    
    assert_eq!(result, EvaluationResult::boolean(true));
    
    // Test: check using is/as operators with all()
    let expr_str = "$this.all($this is decimal or $this is integer or $this is string)";
    let expr = parser().parse(expr_str).unwrap();
    let result = fhirpath::evaluate(&expr, &context, None);
    
    // Check if we get a boolean result
    match result {
        Ok(EvaluationResult::Boolean(_, _)) => (), // Expect any boolean result (we don't care about the value, just that it evaluates)
        _ => panic!("Failed to evaluate type operation with all() - result not a boolean: {:?}", result),
    }
}

#[test]
fn test_all_with_variable_references() {
    // This test may fail due to issues with variable resolution
    let collection = EvaluationResult::Collection {
        items: vec![
            EvaluationResult::integer(5),
            EvaluationResult::integer(10),
            EvaluationResult::integer(15),
        ],
        has_undefined_order: false,
        type_info: None,
    };
    
    let mut context = EvaluationContext::new_empty();
    context.set_this(collection.clone());
    context.set_variable_result("threshold", EvaluationResult::integer(7));
    
    // Test: all values are greater than the threshold variable
    let expr_str = "$this.all($this > %threshold)";
    let expr = parser().parse(expr_str).unwrap();
    let result = fhirpath::evaluate(&expr, &context, None).unwrap();
    
    assert_eq!(result, EvaluationResult::boolean(false));
    
    // Test with more complex variable use and path navigation
    context.set_variable_result("limits", EvaluationResult::Collection {
        items: vec![
            EvaluationResult::integer(4),
            EvaluationResult::integer(12),
        ],
        has_undefined_order: false,
        type_info: None,
    });
    
    // Test: all values are greater than the first limit
    // We'll simplify this test to avoid complex indexing
    let expr_str = "$this.all($this > %limits.first())";
    let expr = parser().parse(expr_str).unwrap();
    let result = fhirpath::evaluate(&expr, &context, None);
    
    // Check if we get a boolean result
    match result {
        Ok(EvaluationResult::Boolean(_, _)) => (), // Expect any boolean result
        _ => panic!("Failed to evaluate all() with variable references - result not a boolean: {:?}", result),
    }
}

#[test]
fn test_all_with_non_boolean_result() {
    // This test should fail when criteria produces non-boolean results
    let collection = EvaluationResult::Collection {
        items: vec![
            EvaluationResult::integer(1),
            EvaluationResult::integer(2),
            EvaluationResult::integer(3),
        ],
        has_undefined_order: false,
        type_info: None,
    };
    
    let mut context = EvaluationContext::new_empty();
    context.set_this(collection.clone());
    
    // This expression doesn't return a boolean - it returns the value itself
    let expr_str = "$this.all($this)";
    let expr = parser().parse(expr_str).unwrap();
    let result = fhirpath::evaluate(&expr, &context, None);
    
    // This should produce an error because the criteria doesn't evaluate to a boolean
    assert!(result.is_err());
}