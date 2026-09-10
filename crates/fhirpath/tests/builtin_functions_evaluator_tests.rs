//! Cross-check for `helios_fhirpath::builtin_functions` (#821): every
//! cataloged name must be a function the evaluator actually recognizes.
//!
//! The catalog itself (sortedness, no duplicates, category closed set) is
//! tested next to its definition in `src/functions.rs`; this integration
//! test is the one that needs `evaluate_expression` and a real
//! `EvaluationContext`, so it lives here instead.

use helios_fhirpath::{EvaluationContext, builtin_functions, evaluate_expression};

/// Functions whose evaluator dispatch is gated by argument count *before*
/// falling through to the generic "unknown function" arm — calling them with
/// zero arguments would misreport them as unimplemented. Each entry supplies
/// the minimum FHIRPath source needed to satisfy that arity gate; every
/// other cataloged function is safe to call as `{}.<name>()`.
fn minimal_call(name: &str) -> Option<String> {
    let call = match name {
        "where" => "{}.where(true)",
        "select" => "{}.select($this)",
        "coalesce" => "{}.coalesce(1)",
        "repeat" => "{}.repeat($this)",
        "repeatAll" => "{}.repeatAll($this)",
        "aggregate" => "{}.aggregate($total)",
        "iif" => "{}.iif(true, 1)",
        "duration" => "{}.duration(1, 'year')",
        "difference" => "{}.difference(1, 'year')",
        "ofType" => "{}.ofType(String)",
        _ => return None,
    };
    Some(call.to_string())
}

/// The substring the evaluator's own "unknown function" error contains
/// (`EvaluationError::UnsupportedFunction`'s `Display` impl formats it as
/// `"Unsupported Function: Function '<name>' is not implemented"`).
fn is_unknown_function_error(name: &str, error: &str) -> bool {
    error.contains("Unsupported Function") && error.contains(&format!("'{name}'"))
}

#[test]
fn every_cataloged_function_is_known_to_the_evaluator() {
    let context = EvaluationContext::new(vec![]);

    // Sanity: an actually-unknown function does produce the error this test
    // looks for, so a false negative below can't hide behind a broken check.
    let baseline = evaluate_expression("{}.thisFunctionDoesNotExist()", &context).unwrap_err();
    assert!(
        is_unknown_function_error("thisFunctionDoesNotExist", &baseline),
        "expected the baseline unknown function to fail as unsupported, got: {baseline}"
    );

    let mut unrecognized = Vec::new();
    for info in builtin_functions() {
        let expression = minimal_call(info.name).unwrap_or_else(|| format!("{{}}.{}()", info.name));
        if let Err(e) = evaluate_expression(&expression, &context)
            && is_unknown_function_error(info.name, &e)
        {
            unrecognized.push(info.name);
        }
    }

    assert!(
        unrecognized.is_empty(),
        "builtin_functions() lists names the evaluator does not recognize: {unrecognized:?}"
    );
}
