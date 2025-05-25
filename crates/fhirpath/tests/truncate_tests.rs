use chumsky::Parser;
use fhirpath::evaluator::{EvaluationContext, evaluate};
use fhirpath::parser::parser;
use fhirpath_support::{EvaluationError, EvaluationResult};

// Helper function to parse and evaluate
fn eval(input: &str) -> Result<EvaluationResult, EvaluationError> {
    let ctx = EvaluationContext::new_empty();
    let expr = parser().parse(input).unwrap_or_else(|e| {
        panic!("Parser error for input '{}': {:?}", input, e);
    });
    evaluate(&expr, &ctx, None)
}

#[test]
fn test_truncate_integer() {
    // Integer truncation (should return the same integer)
    assert_eq!(eval("101.truncate()").unwrap(), EvaluationResult::integer(101));
    assert_eq!(eval("0.truncate()").unwrap(), EvaluationResult::integer(0));
    assert_eq!(eval("(-42).truncate()").unwrap(), EvaluationResult::integer(-42));
}

#[test]
fn test_truncate_decimal() {
    // Decimal truncation (should return only the integer part)
    assert_eq!(eval("101.5.truncate()").unwrap(), EvaluationResult::integer(101));
    assert_eq!(eval("1.00000001.truncate()").unwrap(), EvaluationResult::integer(1));
    assert_eq!(eval("(-1.56).truncate()").unwrap(), EvaluationResult::integer(-1));
    assert_eq!(eval("(-0.99).truncate()").unwrap(), EvaluationResult::integer(0));
}

#[test]
fn test_truncate_collection() {
    // Collection with single item (should work like scalar)
    assert_eq!(eval("5.7.truncate()").unwrap(), EvaluationResult::integer(5));
    
    // Empty collection (should return empty)
    assert_eq!(eval("{}.truncate()").unwrap(), EvaluationResult::Empty);
}

#[test]
fn test_truncate_multiple_items() {
    // Collection with multiple items (should error)
    let result = eval("(1.1 | 2.2).truncate()");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("singleton"));
}

#[test]
fn test_truncate_invalid_type() {
    // String (should error)
    let result = eval("'abc'.truncate()");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("numeric"));
}