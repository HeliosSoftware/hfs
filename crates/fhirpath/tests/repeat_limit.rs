//! `repeat()` stops with an error once it has produced 10,000 items, as
//! `repeatAll()` does. A projection that derives a new value from every item
//! (`$this + 1`, a growing string) never reaches a round without new items,
//! and used to loop until the evaluation was abandoned, which a write
//! indexing a tenant's SearchParameter never does.

use std::sync::mpsc;
use std::time::Duration;

use helios_fhirpath::evaluate_expression;
use helios_fhirpath::evaluator::EvaluationContext;

/// The outcome of `expression`, or a panic if it does not end within 60 s.
fn evaluate_bounded(expression: &'static str) -> Result<String, String> {
    let (done, outcome) = mpsc::channel();
    std::thread::spawn(move || {
        let context = EvaluationContext::new_empty_with_default_version();
        let _ = done.send(evaluate_expression(expression, &context).map(|r| format!("{r:?}")));
    });
    outcome
        .recv_timeout(Duration::from_secs(60))
        .unwrap_or_else(|_| panic!("{expression} did not finish"))
}

#[test]
fn a_projection_that_always_yields_a_new_value_is_stopped() {
    for expression in ["1.repeat($this + 1)", "'a'.repeat($this + 'a')"] {
        let error = evaluate_bounded(expression).expect_err("refused");
        assert!(error.contains("safety limit"), "{expression}: {error}");
    }
}

#[test]
fn a_projection_that_reaches_a_fixed_point_is_unchanged() {
    let result =
        evaluate_bounded("(1 | 2 | 3).repeat(iif($this < 3, $this + 1, {}))").expect("evaluates");
    for value in ["2", "3"] {
        assert!(result.contains(value), "{result}");
    }
}
