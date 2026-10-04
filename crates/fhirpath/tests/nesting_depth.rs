//! A pathologically nested FHIRPath expression is refused with a parse error
//! rather than overflowing the recursive-descent parser's stack and aborting
//! the process. Found by fuzzing the SQL-on-FHIR ViewDefinition column path on
//! 2026-09-30: a column `path` a few thousand parentheses deep crashed the
//! whole `hfs` process ("thread 'tokio-rt-worker' has overflowed its stack"),
//! taking every in-flight request with it — a stack overflow cannot be caught.

use helios_fhirpath::{
    MAX_EXPRESSION_DEPTH, MAX_NESTING_DEPTH, parse_expression, parse_expression_diagnostics,
    parse_expression_spanned,
};

/// `id` wrapped in `depth` layers of parentheses — valid FHIRPath that parses
/// to the same thing as `id`, but drives the parser `depth` levels deep.
fn nested(depth: usize) -> String {
    format!("{}id{}", "(".repeat(depth), ")".repeat(depth))
}

#[test]
fn depth_at_the_limit_still_parses() {
    assert!(parse_expression(&nested(MAX_NESTING_DEPTH)).is_ok());
    assert!(parse_expression_diagnostics(&nested(MAX_NESTING_DEPTH)).is_ok());
    assert!(parse_expression_spanned(&nested(MAX_NESTING_DEPTH)).is_ok());
}

#[test]
fn depth_past_the_limit_is_a_parse_error_not_a_crash() {
    // Deep enough to overflow the stack without the guard; the guard turns it
    // into an ordinary error on every entry point.
    let expr = nested(20_000);

    let err = parse_expression(&expr).unwrap_err();
    assert!(err.contains("nesting"), "{err}");

    let diags = parse_expression_diagnostics(&expr).unwrap_err();
    assert!(
        diags.iter().any(|d| d.message.contains("nesting")),
        "{diags:?}"
    );

    let diags = parse_expression_spanned(&expr).unwrap_err();
    assert!(
        diags.iter().any(|d| d.message.contains("nesting")),
        "{diags:?}"
    );
}

#[test]
fn brackets_inside_a_string_literal_do_not_count_toward_depth() {
    // A string literal full of parentheses nests the parser one level, not 500,
    // so it must parse rather than trip the guard.
    let expr = format!("'{}'", "(".repeat(500));
    assert!(
        parse_expression(&expr).is_ok(),
        "string literal was miscounted"
    );
}

fn chain(term: &str, sep: &str, n: usize) -> String {
    vec![term; n + 1].join(sep)
}

fn assert_rejected_everywhere(expr: &str) {
    assert!(parse_expression(expr).unwrap_err().contains("nesting"));
    assert!(parse_expression_diagnostics(expr).is_err());
    assert!(parse_expression_spanned(expr).is_err());
}

#[test]
fn two_hundred_or_terms_still_parse() {
    let ok = chain("a", " or ", 200);
    assert!(parse_expression(&ok).is_ok());
    assert!(parse_expression_diagnostics(&ok).is_ok());
    assert!(parse_expression_spanned(&ok).is_ok());
}

#[test]
fn long_operator_chains_are_a_parse_error() {
    let n = MAX_EXPRESSION_DEPTH + 50;
    assert_rejected_everywhere(&chain("1", " + ", n));
    assert_rejected_everywhere(&chain("a", " or ", n));
    assert_rejected_everywhere(&chain("a", ".", n));
    assert_rejected_everywhere(&format!("{}1", "-".repeat(n)));
    assert_rejected_everywhere(&format!("a{}", "[0]".repeat(n)));
}

#[test]
fn operators_inside_literals_do_not_count() {
    let n = MAX_EXPRESSION_DEPTH + 50;
    assert!(parse_expression(&format!("'{}'", " + ".repeat(n))).is_ok());
    assert!(parse_expression(&format!("`a{}`", ".b".repeat(n))).is_ok());
}

#[test]
fn moderate_chains_repeated_across_parentheses_accumulate() {
    let inner = chain("1", " + ", 40);
    let mut expr = inner.clone();
    for _ in 0..8 {
        expr = format!("({expr}) + {inner}");
    }
    assert_rejected_everywhere(&expr);
}

#[test]
fn comment_and_backtick_evasions_are_rejected() {
    let n = MAX_EXPRESSION_DEPTH + 50;
    let long = chain("1", " + ", n);
    assert_rejected_everywhere(&format!("/*'*/ {long} /*'*/"));
    assert_rejected_everywhere(&format!("`a\\`` + {long}"));
}

#[test]
fn function_chains_and_type_operators_count() {
    let n = MAX_EXPRESSION_DEPTH + 50;
    assert_rejected_everywhere(&format!("a{}", ".first()".repeat(n)));
    assert_rejected_everywhere(&format!("1{}", " is Integer".repeat(n)));
    assert!(parse_expression(&format!("a{}", ".first()".repeat(100))).is_ok());
}

#[test]
fn keywords_after_a_dot_decimals_and_two_char_operators_are_lexed() {
    // `or`/`and`/`in` after `.` are member names, not operators, and `1.5`
    // is one token — a long chain of them must be counted as plain dots only.
    let members = chain("a", ".or.", 100); // 200 dots
    assert!(parse_expression(&members).is_ok());
    let decimals = chain("1.5", " <= ", 100);
    assert!(parse_expression(&decimals).is_ok());
    // `!=` is one operator, not two.
    assert!(parse_expression(&chain("a", " != ", 200)).is_ok());
    assert_rejected_everywhere(&chain("a", " != ", MAX_EXPRESSION_DEPTH + 10));
}

#[test]
fn evaluating_a_rejected_chain_on_a_small_stack_is_an_error_not_an_abort() {
    // The #1219 repro: ~2000 chained `+` evaluated on a 2 MiB worker thread.
    let expr = chain("1", " + ", 2000);
    let result = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || {
            let ctx = helios_fhirpath::EvaluationContext::new_empty(helios_fhir::FhirVersion::R4);
            helios_fhirpath::evaluate_expression(&expr, &ctx)
        })
        .unwrap()
        .join()
        .expect("evaluation must not overflow the stack");
    assert!(result.unwrap_err().contains("nesting"));
}
