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

/// The scan used to take everything after `@` that looked like part of a
/// literal (letters, digits, `.`, `-`, `:`, `+`) and to end a `//` comment
/// only at `\n`, while the parser takes a fixed-width literal and ends a
/// comment at any line terminator. Chains written after either were not
/// counted, and evaluating them overflowed the stack.
#[test]
fn chains_after_a_temporal_literal_or_a_line_comment_are_counted() {
    let n = MAX_EXPRESSION_DEPTH + 50;
    assert_rejected_everywhere(&format!("@2020-01-01{}", ".a".repeat(n)));
    assert_rejected_everywhere(&format!("@2020{}", "+1".repeat(n)));
    assert_rejected_everywhere(&format!("@2020-01-01T10:00:00.123Z{}", "-1".repeat(n)));
    assert_rejected_everywhere(&format!("@T10:30{}", ".a".repeat(n)));
    for terminator in ['\r', '\u{0B}', '\u{0C}', '\u{85}', '\u{2028}', '\u{2029}'] {
        assert_rejected_everywhere(&format!("1 // note{terminator} + {}", chain("1", " + ", n)));
    }
}

/// The parser allows a comment only around an integer, a date literal or a
/// type name. After anything else, `//*` is a division followed by a block
/// comment, so the rest of the line is syntax. The scan used to skip it as a
/// line comment.
#[test]
fn chains_after_a_division_and_a_block_comment_are_counted() {
    let n = MAX_EXPRESSION_DEPTH + 50;
    assert_eq!(
        parse_expression("a //* c */ 1"),
        parse_expression("a / 1"),
        "the parser reads `//*` after an identifier as a division"
    );
    for start in ["a", "'s'", "(a)", "and", "is", "a.is", "1.5", "1 'mg'"] {
        assert_rejected_everywhere(&format!("{start} //* c */ 1{}", " + 1".repeat(n)));
        assert_rejected_everywhere(&format!("{start} //* c */ @2020{}", ".a".repeat(n)));
    }
    assert_rejected_everywhere(&format!(
        "a //* c */ 1 + {}1{}",
        "(".repeat(MAX_NESTING_DEPTH + 50),
        ")".repeat(MAX_NESTING_DEPTH + 50)
    ));
    // Where the parser does read a line comment (after an integer, a type
    // name, or an operator), the scan must not read a block comment instead:
    // the quote here would then open a string over the chain.
    assert_rejected_everywhere(&format!("1 //* */ '\n{} + 'x'", " + 1".repeat(n)));
    assert_rejected_everywhere(&format!("- //* '\n1{} + 'x'", " + 1".repeat(n)));
    assert_rejected_everywhere(&format!(
        "a is /* c */ A //* */ '\n{} | 'x'",
        " | 1".repeat(n)
    ));
    // `///` is a division and then a line comment: the division is counted.
    assert_rejected_everywhere(&chain("a /// c\n1", " * ", n / 2 + 1));
}

#[test]
fn comments_that_start_with_a_slash_or_a_star_still_parse() {
    for expr in [
        "1 /// c\n + 1",
        "1 //* c\n + 1",
        "1 + /// c\n 1",
        "1 //// c",
        "- //* c\n1 + 'a'",
        "a is A //* c\n | 'a'",
        "a /// c\n1",
    ] {
        assert!(parse_expression(expr).is_ok(), "{expr}");
    }
}

#[test]
fn temporal_literals_still_parse() {
    for literal in [
        "@2020",
        "@2020-01",
        "@2020-01-01",
        "@2020T",
        "@2020-01-01T10",
        "@2020-01-01T10:30:15.123+05:30",
        "@2020-01-01T10:30:15Z",
        "@T10:30:15.5",
    ] {
        assert!(
            parse_expression(&format!("{literal} = {literal}")).is_ok(),
            "{literal}"
        );
    }
    assert!(parse_expression(&format!("@2020-01-01{}", ".a".repeat(100))).is_ok());
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

#[test]
fn fhirpath_server_handler_rejects_a_deep_expression_without_aborting() {
    // The fhirpath-server handler used to call the parser directly, bypassing
    // the cap: `validate` (spanned parse + AST walk) and `context` (main
    // expression parse + evaluation) each reached the walkers unguarded.
    let params = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            { "name": "expression", "valueString": chain("1", " + ", 2000) },
            { "name": "context", "valueString": "name" },
            { "name": "validate", "valueBoolean": true },
            { "name": "resource", "resource": {
                "resourceType": "Patient", "name": [{ "family": "Chalmers" }]
            } }
        ]
    });
    let params: helios_fhirpath::models::FhirPathParameters =
        serde_json::from_value(params).unwrap();
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let _ = rt.block_on(helios_fhirpath::handlers::evaluate_fhirpath(
                None,
                axum::Json(params),
            ));
        })
        .unwrap()
        .join()
        .expect("handler must not overflow the stack");
}

#[test]
fn nested_function_calls_count_toward_the_chain_depth() {
    // Nested calls sit under the bracket cap but each costs evaluator stack:
    // measured on a 2 MiB release thread, 100 nested `iif` around a 250-long
    // chain aborted while passing both caps separately.
    let nested_calls =
        |n: usize, inner: &str| format!("{}{inner}{}", "iif(true, ".repeat(n), ")".repeat(n));
    assert_rejected_everywhere(&nested_calls(100, &chain("1", " + ", 250)));
    assert_rejected_everywhere(&format!(
        "{}1{}",
        "1 + iif(true, ".repeat(100),
        ")".repeat(100)
    ));
    // A realistic amount of nesting is untouched.
    assert!(parse_expression(&nested_calls(10, "name.where(use = 'official').first()")).is_ok());
}
