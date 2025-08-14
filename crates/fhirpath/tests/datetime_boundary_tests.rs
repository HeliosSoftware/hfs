use helios_fhirpath::{EvaluationContext, evaluate_expression};
use helios_fhirpath_support::EvaluationResult;

#[test]
fn test_datetime_boundary_timezone_handling() {
    let context = EvaluationContext::new_empty_with_default_version();

    // Test date "2010-10-10" lowBoundary -> "2010-10-10" (Date stays Date)
    let result = evaluate_expression("@2010-10-10.lowBoundary()", &context).unwrap();
    assert_eq!(result, EvaluationResult::date("2010-10-10".to_string()));

    // Test date "2010-10-10" highBoundary -> "2010-10-10" (Date stays Date)
    let result = evaluate_expression("@2010-10-10.highBoundary()", &context).unwrap();
    assert_eq!(result, EvaluationResult::date("2010-10-10".to_string()));
}

#[test]
fn test_datetime_with_time_boundary() {
    let context = EvaluationContext::new_empty_with_default_version();

    // Test datetime with time part
    let result = evaluate_expression("@2010-10-10T12:30.lowBoundary()", &context).unwrap();
    assert_eq!(
        result,
        EvaluationResult::datetime("2010-10-10T12:30:00.000+14:00".to_string())
    );

    let result = evaluate_expression("@2010-10-10T12:30.highBoundary()", &context).unwrap();
    assert_eq!(
        result,
        EvaluationResult::datetime("2010-10-10T12:30:59.999-12:00".to_string())
    );
}
