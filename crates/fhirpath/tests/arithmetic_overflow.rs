//! Arithmetic whose result is outside the representable range ends as an
//! ordinary evaluation result (empty or an overflow error), never a panic.
//! Decimal `+ - * div mod`, `power()`, Integer `div`/`mod` of the minimum
//! value by -1, date and time durations, `highBoundary` and `sum()` used
//! operations that panic on overflow, and a stored SearchParameter using one
//! failed every later write of its base type in the tenant.

use helios_fhirpath::evaluate_expression;
use helios_fhirpath::evaluator::EvaluationContext;

const DECIMAL_MAX: &str = "79228162514264337593543950335.0";

fn evaluates_without_panicking(expression: &str) {
    let outcome = std::panic::catch_unwind(|| {
        let context = EvaluationContext::new_empty_with_default_version();
        let _ = evaluate_expression(expression, &context);
    });
    assert!(outcome.is_ok(), "{expression} panicked");
}

#[test]
fn decimal_arithmetic_past_the_range_does_not_panic() {
    for expression in [
        format!("{DECIMAL_MAX} * 2"),
        format!("{DECIMAL_MAX} * 2.5"),
        format!("{DECIMAL_MAX} + 1.0"),
        format!("{DECIMAL_MAX} + 1"),
        format!("(-{DECIMAL_MAX}) - 1.0"),
        format!("(-{DECIMAL_MAX}) - 1"),
        format!("{DECIMAL_MAX} div 0.5"),
        format!("{DECIMAL_MAX} 'mg' * 2"),
        format!("{DECIMAL_MAX} 'mg' + {DECIMAL_MAX} 'mg'"),
        format!("({DECIMAL_MAX} | 1.5).sum()"),
        format!("{DECIMAL_MAX}.highBoundary(0)"),
        format!("(-{DECIMAL_MAX}).lowBoundary(0)"),
    ] {
        evaluates_without_panicking(&expression);
    }
}

#[test]
fn power_past_the_range_does_not_panic() {
    for expression in ["10.power(30)", "2.power(99)", "10.5.power(40)"] {
        evaluates_without_panicking(expression);
    }
}

#[test]
fn power_within_the_range_is_unchanged() {
    let context = EvaluationContext::new_empty_with_default_version();
    let result = evaluate_expression("2.power(10)", &context).expect("evaluates");
    assert!(format!("{result:?}").contains("1024"), "{result:?}");
    let result = evaluate_expression("10.power(28)", &context).expect("evaluates");
    assert!(
        format!("{result:?}").contains("10000000000000000000000000000"),
        "{result:?}"
    );
}

#[test]
fn integer_division_of_the_minimum_by_minus_one_does_not_panic() {
    for expression in [
        "(-9223372036854775807 - 1) div -1",
        "(-9223372036854775807 - 1) mod -1",
    ] {
        evaluates_without_panicking(expression);
    }
}

#[test]
fn durations_past_the_calendar_range_do_not_panic() {
    for expression in [
        "@2020-01-01 + 1000000000000000 days",
        "@2020-01-01 + 1000000000000000 weeks",
        "@2020-01-01T00:00:00 + 1000000000000000 days",
        "@2020-01-01T00:00:00 + 100000000000000000 years",
        "@2020-01-01T00:00:00 + 1000000000000 'ms'",
        "@T10:00 + 100000000000000000000 hours",
    ] {
        evaluates_without_panicking(expression);
    }
}
