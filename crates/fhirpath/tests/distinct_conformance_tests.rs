//! `distinct()` / `isDistinct()` cases from the upstream HL7 `testDistinct`
//! conformance group (FHIR/fhir-test-cases#302, testDistinct7..36).
//!
//! Items are duplicates when `=` returns true (so `1 = 1.0`, `1.0 = 1.00`, a code
//! equals a string with the same value, and the same instant in two timezones is
//! equal); `distinct()` keeps the last of each set of equal items, in order; and
//! date/time values of differing precision cannot be compared, so both functions
//! return empty.

use helios_fhir::FhirResource;
use helios_fhirpath::{EvaluationContext, EvaluationResult, evaluate_expression};
use rust_decimal::Decimal;
use std::str::FromStr;

fn patient_context() -> EvaluationContext {
    let patient_json = serde_json::json!({
        "resourceType": "Patient",
        "id": "example",
        "gender": "male",
        "name": [
            { "given": ["Peter", "James"] },
            { "given": ["Jim"] },
            { "given": ["Peter", "James"] }
        ]
    });
    let patient: helios_fhir::r4::Patient = serde_json::from_value(patient_json).unwrap();
    EvaluationContext::new(vec![FhirResource::R4(Box::new(
        helios_fhir::r4::Resource::Patient(Box::new(patient)),
    ))])
}

fn eval(expr: &str) -> EvaluationResult {
    evaluate_expression(expr, &patient_context())
        .unwrap_or_else(|e| panic!("{expr} failed to evaluate: {e:?}"))
}

fn items(result: EvaluationResult) -> Vec<EvaluationResult> {
    match result {
        EvaluationResult::Empty => vec![],
        EvaluationResult::Collection { items, .. } => items,
        single => vec![single],
    }
}

fn assert_is_distinct(expr: &str, expected: bool) {
    assert_eq!(
        eval(expr),
        EvaluationResult::boolean(expected),
        "{expr} should be {expected}"
    );
}

fn assert_empty(expr: &str) {
    assert_eq!(
        eval(expr),
        EvaluationResult::Empty,
        "{expr} should be empty"
    );
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

#[test]
fn is_distinct_string_like_values() {
    assert_is_distinct("Patient.name.given.isDistinct()", false); // testDistinct7
    assert_is_distinct("Patient.name.given.distinct().isDistinct()", true); // testDistinct8
    assert_is_distinct("Patient.gender.combine('male').isDistinct()", false); // testDistinct9
    assert_is_distinct("Patient.gender.combine('female').isDistinct()", true); // testDistinct10
    assert_is_distinct("'1'.combine(2).isDistinct()", true); // testDistinct12
}

#[test]
fn is_distinct_numbers_and_booleans() {
    assert_is_distinct("1.0.combine(1.00).isDistinct()", false); // testDistinct13
    assert_is_distinct("1.combine(2).combine(1).isDistinct()", false); // testDistinct14
    assert_is_distinct("true.combine(false).isDistinct()", true); // testDistinct15
    assert_is_distinct("true.combine(false).combine(true).isDistinct()", false); // testDistinct16
    assert_is_distinct("1.combine('a').combine(true).isDistinct()", true); // testDistinct19
    assert_is_distinct(
        "1.combine('a').combine(true).combine('a').isDistinct()",
        false,
    ); // testDistinct20
    assert_is_distinct("1.combine(1.0).isDistinct()", false); // testDistinct21
    assert_is_distinct("1.5.combine(2.5).combine(10.0).isDistinct()", true); // testDistinct22
    assert_is_distinct("1.5.combine(2.0).combine(1.50).isDistinct()", false); // testDistinct23
    assert_is_distinct("'1.0'.combine('1.00').combine(5.0).isDistinct()", true); // testDistinct24
    assert_is_distinct("10.combine(1.5).isDistinct()", true); // testDistinct25
}

#[test]
fn is_distinct_dates_and_times() {
    assert_is_distinct("@T10:00:00.combine(@T11:00:00).isDistinct()", true); // testDistinct17
    assert_is_distinct("@T10:00:00.combine(@T10:00:00).isDistinct()", false); // testDistinct18
    assert_is_distinct("@2020-01-01.combine(@2020-01-02).isDistinct()", true); // testDistinct26
    assert_is_distinct("@2020-01-01.combine(@2020-01-01).isDistinct()", false); // testDistinct27
    assert_is_distinct(
        "@2020-01-01T10:00:00+10:00.combine(@2020-01-01T00:00:00Z).isDistinct()",
        false,
    ); // testDistinct28
    assert_empty("@2020.combine(@2020-01-01).isDistinct()"); // testDistinct29
}

#[test]
fn is_distinct_quantities_use_unit_conversion() {
    // Spec example: both quantities are the same via unit conversion.
    assert_is_distinct("1 'm'.combine(100 'cm').isDistinct()", false);
    assert_is_distinct("1 'm'.combine(2 'm').isDistinct()", true);
}

#[test]
fn distinct_keeps_last_of_each_equal_set() {
    // testDistinct30
    assert_eq!(
        items(eval("'a'.combine('b').combine('a').distinct()")),
        vec![
            EvaluationResult::string("b".into()),
            EvaluationResult::string("a".into())
        ]
    );
    // testDistinct31: the plain string (last) is kept, not the code
    let kept = items(eval("Patient.gender.combine('male').distinct()"));
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0], EvaluationResult::string("male".into()));
    assert_eq!(
        items(eval(
            "Patient.gender.combine('male').distinct().type().name"
        )),
        vec![EvaluationResult::string("String".into())]
    );
    // testDistinct32
    assert_eq!(
        items(eval("1.combine(2).combine(1).combine(3).distinct()")),
        vec![
            EvaluationResult::integer(2),
            EvaluationResult::integer(1),
            EvaluationResult::integer(3)
        ]
    );
    // testDistinct33: integer 1 and decimal 1.0 are equal; the decimal is last
    assert_eq!(
        items(eval("1.combine(2).combine(1.0).distinct()")),
        vec![
            EvaluationResult::integer(2),
            EvaluationResult::decimal(dec("1.0"))
        ]
    );
    // testDistinct34
    assert_eq!(
        items(eval("true.combine(false).combine(true).distinct()")),
        vec![
            EvaluationResult::boolean(false),
            EvaluationResult::boolean(true)
        ]
    );
}

#[test]
fn distinct_dates() {
    // testDistinct35
    let kept = items(eval(
        "@2020-01-01.combine(@2020-01-02).combine(@2020-01-01).distinct()",
    ));
    assert_eq!(kept.len(), 2);
    assert!(matches!(&kept[0], EvaluationResult::Date(d, _, _) if d == "2020-01-02"));
    assert!(matches!(&kept[1], EvaluationResult::Date(d, _, _) if d == "2020-01-01"));
    // testDistinct36: differing precision cannot be compared
    assert_empty("@2020.combine(@2020-01-01).distinct()");
}

#[test]
fn distinct_times_and_timezones() {
    let kept = items(eval("@T10:00:00.combine(@T10:00:00).distinct()"));
    assert_eq!(kept.len(), 1);
    let kept = items(eval(
        "@2020-01-01T10:00:00+10:00.combine(@2020-01-01T00:00:00Z).distinct()",
    ));
    assert_eq!(kept.len(), 1);
    assert!(
        matches!(&kept[0], EvaluationResult::DateTime(d, _, _) if d.ends_with('Z')),
        "last of the equal pair is kept: {kept:?}"
    );
}
