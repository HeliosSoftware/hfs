//! Decimals beyond `rust_decimal`'s range (`1e999999999`) keep their value (#2716).
//!
//! The upstream `testLargeExponents` group (vendored in `data/r5/tests-fhir-r5.xml`)
//! covers comparison, `toString()`, arithmetic and boundaries. These cases cover
//! the rest of what the engine does with such a value.
#![cfg(feature = "R5")]

use chumsky::Parser;
use helios_fhir::r5;
use helios_fhirpath::{
    evaluator::{EvaluationContext, evaluate},
    parser::parser,
};
use helios_fhirpath_support::EvaluationResult;

const INPUT: &str = include_str!("data/r5/input/parameters-example-large-exponents.json");

fn eval(expression: &str) -> EvaluationResult {
    let resource: r5::Resource = serde_json::from_str(INPUT).expect("input parses");
    let context = EvaluationContext::new(vec![helios_fhir::FhirResource::R5(Box::new(resource))]);
    let expr = parser().parse(expression).unwrap();
    match evaluate(&expr, &context, None).unwrap() {
        EvaluationResult::Collection { mut items, .. } if items.len() == 1 => items.remove(0),
        other => other,
    }
}

fn string(expression: &str) -> String {
    match eval(expression) {
        EvaluationResult::String(s, _, _) => s,
        other => panic!("{expression}: expected a string, got {other:?}"),
    }
}

fn boolean(expression: &str) -> bool {
    match eval(expression) {
        EvaluationResult::Boolean(b, _, _) => b,
        other => panic!("{expression}: expected a boolean, got {other:?}"),
    }
}

#[test]
fn decimal_keeps_its_value_in_exponent_form() {
    assert_eq!(
        string("Parameters.parameter[2].value.toString()"),
        "1E+999999999"
    );
    assert_eq!(
        string("Parameters.parameter[3].value.toString()"),
        "1E-999999999"
    );
    assert_eq!(
        string("(-Parameters.parameter[2].value).toString()"),
        "-1E+999999999"
    );
    assert_eq!(
        string("(-Parameters.parameter[0].value).toString()"),
        "-1E+999999999 'kg'"
    );
    assert_eq!(
        string("Parameters.parameter[4].value.toString()"),
        "1E+999999999 'd'"
    );
    assert_eq!(
        string("'x' & Parameters.parameter[2].value"),
        "x1E+999999999"
    );
}

#[test]
fn decimal_is_typed_as_a_decimal() {
    assert!(boolean("Parameters.parameter[2].value is decimal"));
    assert!(boolean("Parameters.parameter[2].value.exists()"));
    assert!(boolean("Parameters.parameter[2].value.convertsToDecimal()"));
    assert!(!boolean(
        "Parameters.parameter[2].value.convertsToInteger()"
    ));
    assert!(boolean("Parameters.parameter[2].value.convertsToString()"));
    assert_eq!(
        string("Parameters.parameter[2].value.toDecimal().toString()"),
        "1E+999999999"
    );
}

#[test]
fn ordering_handles_signs_and_magnitudes() {
    assert!(boolean(
        "Parameters.parameter[2].value > 7922816251426433759354395033.5"
    ));
    assert!(boolean("-Parameters.parameter[2].value < -1"));
    assert!(boolean("Parameters.parameter[3].value > 0"));
    assert!(boolean(
        "Parameters.parameter[3].value < 0.0000000000000000000000000001"
    ));
    assert!(boolean(
        "Parameters.parameter[2].value = Parameters.parameter[2].value"
    ));
    assert!(boolean("Parameters.parameter[2].value != 1"));
    assert!(boolean("Parameters.parameter[3].value !~ 1"));
    assert!(boolean(
        "Parameters.parameter[0].value > Parameters.parameter[1].value"
    ));
}

#[test]
fn numeric_functions_give_empty() {
    for function in [
        "abs()",
        "ceiling()",
        "floor()",
        "truncate()",
        "sqrt()",
        "toInteger()",
    ] {
        let expression = format!("Parameters.parameter[2].value.{function}");
        assert_eq!(eval(&expression), EvaluationResult::Empty, "{expression}");
    }
    assert_eq!(
        eval("2.power(Parameters.parameter[2].value)"),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("Parameters.parameter[0].value.toQuantity()"),
        EvaluationResult::Empty
    );
}
