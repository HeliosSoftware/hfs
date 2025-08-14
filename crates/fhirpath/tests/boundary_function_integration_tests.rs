use helios_fhirpath::{EvaluationContext, evaluate_expression};
use helios_fhirpath_support::EvaluationResult;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromStr;

#[test]
fn test_low_boundary_decimal_integration() {
    let context = EvaluationContext::new_empty_with_default_version();

    // Test decimal 1.0 -> 0.95
    let result = evaluate_expression("1.0.lowBoundary()", &context).unwrap();
    assert_eq!(
        result,
        EvaluationResult::decimal(Decimal::from_str("0.95").unwrap())
    );
}

#[test]
fn test_high_boundary_decimal_integration() {
    let context = EvaluationContext::new_empty_with_default_version();

    // Test decimal 1.0 -> 1.05
    let result = evaluate_expression("1.0.highBoundary()", &context).unwrap();
    assert_eq!(
        result,
        EvaluationResult::decimal(Decimal::from_str("1.05").unwrap())
    );
}

#[test]
fn test_low_boundary_date_integration() {
    let context = EvaluationContext::new_empty_with_default_version();

    // Test date 1970-06 -> 1970-06-01
    let result = evaluate_expression("@1970-06.lowBoundary()", &context).unwrap();
    assert_eq!(result, EvaluationResult::date("1970-06-01".to_string()));
}

#[test]
fn test_high_boundary_date_integration() {
    let context = EvaluationContext::new_empty_with_default_version();

    // Test date 1970-06 -> 1970-06-30
    let result = evaluate_expression("@1970-06.highBoundary()", &context).unwrap();
    assert_eq!(result, EvaluationResult::date("1970-06-30".to_string()));
}

#[test]
fn test_low_boundary_time_integration() {
    let context = EvaluationContext::new_empty_with_default_version();

    // Test time 12:34 -> 12:34:00.000
    let result = evaluate_expression("@T12:34.lowBoundary()", &context).unwrap();
    assert_eq!(result, EvaluationResult::time("12:34:00.000".to_string()));
}

#[test]
fn test_high_boundary_time_integration() {
    let context = EvaluationContext::new_empty_with_default_version();

    // Test time 12:34 -> 12:34:59.999
    let result = evaluate_expression("@T12:34.highBoundary()", &context).unwrap();
    assert_eq!(result, EvaluationResult::time("12:34:59.999".to_string()));
}

#[test]
fn test_boundary_empty_integration() {
    let context = EvaluationContext::new_empty_with_default_version();

    // Test empty values
    let result = evaluate_expression("{}.lowBoundary()", &context).unwrap();
    assert_eq!(result, EvaluationResult::Empty);

    let result = evaluate_expression("{}.highBoundary()", &context).unwrap();
    assert_eq!(result, EvaluationResult::Empty);
}
