//! A pathologically nested FHIRPath expression is refused with a parse error
//! rather than overflowing the recursive-descent parser's stack and aborting
//! the process. Found by fuzzing the SQL-on-FHIR ViewDefinition column path on
//! 2026-09-30: a column `path` a few thousand parentheses deep crashed the
//! whole `hfs` process ("thread 'tokio-rt-worker' has overflowed its stack"),
//! taking every in-flight request with it — a stack overflow cannot be caught.

use helios_fhirpath::{
    MAX_NESTING_DEPTH, parse_expression, parse_expression_diagnostics, parse_expression_spanned,
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
