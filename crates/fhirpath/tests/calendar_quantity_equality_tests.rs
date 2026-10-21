//! Calendar and UCUM year/month equality, including upstream testEquality36.
//! https://hl7.org/fhirpath/en/index.html#time-valued-quantities

use helios_fhirpath::{EvaluationContext, EvaluationResult, evaluate_expression};

fn assert_result(expression: &str, expected: EvaluationResult) {
    let context = EvaluationContext::new_empty_with_default_version();
    assert_eq!(
        evaluate_expression(expression, &context).unwrap(),
        expected,
        "{expression}",
    );
}

#[test]
fn calendar_years_and_months_are_not_equal_to_ucum_durations() {
    for expression in [
        "1 year = 1 'a'",
        "1 'a' = 1 year",
        "1 years = 1 'a'",
        "1 month = 1 'mo'",
        "1 'mo' = 1 months",
        "1 year = 12 'mo'",
        "12 'mo' = 1 year",
        "1 year != 1 'a'",
        "1 'mo' != 1 month",
        "'1 \\'a\\''.toQuantity() = 1 year",
        "'1 \\'mo\\''.toQuantity() = 1 month",
    ] {
        assert_result(expression, EvaluationResult::Empty);
    }
}

#[test]
fn calendar_and_ucum_years_and_months_remain_equivalent() {
    for expression in [
        "1 year ~ 1 'a'",
        "1 'a' ~ 1 years",
        "1 month ~ 1 'mo'",
        "1 'mo' ~ 1 months",
    ] {
        assert_result(expression, EvaluationResult::boolean(true));
    }
    assert_result("1 year !~ 1 'a'", EvaluationResult::boolean(false));
}

#[test]
fn comparable_duration_equality_is_preserved() {
    for expression in [
        "1 year = 1 year",
        "1 year = 1 years",
        "1 month = 1 months",
        "1 week = 1 'wk'",
        "1 day = 24 'h'",
        "1 hour = 3600 's'",
        "1 minute = 60 's'",
        "1 second = 1000 'ms'",
        "1 'm' = 100 'cm'",
    ] {
        assert_result(expression, EvaluationResult::boolean(true));
    }
    assert_result("1 year = 2 years", EvaluationResult::boolean(false));
}
