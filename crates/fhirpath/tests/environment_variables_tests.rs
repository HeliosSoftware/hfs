//! Tests for `helios_fhirpath::environment_variables` and
//! `helios_fhirpath::is_environment_variable` — the fixed list of FHIRPath
//! environment variables `helios_sof::lint`'s undeclared-constant check
//! (#821) treats as always declared.

use helios_fhirpath::{EvaluationContext, environment_variables, is_environment_variable};

#[test]
fn returns_exactly_the_six_literal_names_the_evaluator_special_cases() {
    let mut vars: Vec<&str> = environment_variables().to_vec();
    vars.sort_unstable();
    assert_eq!(
        vars,
        vec![
            "context",
            "loinc",
            "resource",
            "rootResource",
            "sct",
            "ucum"
        ]
    );
}

#[test]
fn is_environment_variable_matches_the_list_exactly() {
    for name in environment_variables() {
        assert!(is_environment_variable(name));
    }
    assert!(!is_environment_variable("definitelyUnknown"));
    // %vs-* and %ext-* are patterns, not enumerable single names, and
    // %terminologies is a namespace object rather than a value — none of
    // these belong in the fixed list even though the evaluator also
    // special-cases them.
    assert!(!is_environment_variable("vs-administrative-gender"));
    assert!(!is_environment_variable("ext-patient-birthTime"));
    assert!(!is_environment_variable("terminologies"));
}

#[test]
fn is_environment_variable_is_case_sensitive() {
    assert!(is_environment_variable("ucum"));
    assert!(!is_environment_variable("UCUM"));
}

/// Cross-check: every name `environment_variables()` lists must evaluate as
/// `%<name>` against a minimal context without producing the evaluator's
/// "undefined variable" error — the same error an unknown name like
/// `%definitelyUnknown` produces.
#[test]
fn each_environment_variable_evaluates_without_an_undefined_variable_error() {
    let context = EvaluationContext::new(vec![]);

    let unknown_error =
        helios_fhirpath::evaluate_expression("%definitelyUnknown", &context).unwrap_err();
    assert!(
        unknown_error.contains("Undefined Variable") || unknown_error.contains("UndefinedVariable"),
        "expected %definitelyUnknown to fail as an undefined variable, got: {unknown_error}"
    );

    for name in environment_variables() {
        let expression = format!("%{name}");
        let result = helios_fhirpath::evaluate_expression(&expression, &context);
        match result {
            Ok(_) => {}
            Err(e) => assert!(
                !e.contains("Undefined Variable") && !e.contains("UndefinedVariable"),
                "expected %{name} to resolve as a known environment variable, got error: {e}"
            ),
        }
    }
}
