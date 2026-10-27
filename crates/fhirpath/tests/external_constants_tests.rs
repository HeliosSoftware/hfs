//! Tests for `helios_fhirpath::external_constants` — the walker that locates
//! every `%name` reference in a parsed FHIRPath expression, which
//! `helios_sof::lint`'s undeclared-constant check (#821) is built on.

use helios_fhirpath::{ExternalConstantRef, external_constants, parse_expression_spanned};

/// Parses `source` and returns its external-constant references, panicking
/// (with a useful message) if `source` doesn't parse.
fn refs(source: &str) -> Vec<ExternalConstantRef> {
    let spanned = parse_expression_spanned(source)
        .unwrap_or_else(|e| panic!("expected {source:?} to parse, got {e:?}"));
    external_constants(&spanned, source)
}

/// The exact substring `source[span.position..span.position+span.length]`
/// covers, for asserting a reference's span underlines precisely the token
/// (including its leading `%`) and nothing else.
fn spanned_text<'a>(source: &'a str, r: &ExternalConstantRef) -> &'a str {
    &source[r.span.position..r.span.position + r.span.length]
}

#[test]
fn finds_a_bare_operand_reference() {
    let source = "name.where(system = %ucum)";
    let found = refs(source);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "ucum");
    assert_eq!(spanned_text(source, &found[0]), "%ucum");
}

#[test]
fn finds_a_reference_inside_a_function_argument() {
    let source = "extension(%myExtensionUrl)";
    let found = refs(source);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "myExtensionUrl");
    assert_eq!(spanned_text(source, &found[0]), "%myExtensionUrl");
}

#[test]
fn finds_a_reference_inside_a_where_lambda_argument() {
    let source = "telecom.where(system = %preferredSystem)";
    let found = refs(source);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "preferredSystem");
}

#[test]
fn finds_a_reference_inside_an_indexer() {
    let source = "name[%idx]";
    let found = refs(source);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "idx");
}

#[test]
fn finds_references_on_both_sides_of_a_union() {
    let source = "%left | %right";
    let found = refs(source);
    let names: Vec<&str> = found.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["left", "right"]);
}

#[test]
fn finds_a_reference_as_the_operand_of_is() {
    let source = "%maybeQuantity is Quantity";
    let found = refs(source);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "maybeQuantity");
}

#[test]
fn finds_a_reference_as_the_operand_of_as() {
    let source = "(%maybeQuantity as Quantity).value";
    let found = refs(source);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "maybeQuantity");
}

#[test]
fn finds_a_reference_inside_a_parenthesized_expression() {
    let source = "(1 + %offset) * 2";
    let found = refs(source);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "offset");
}

#[test]
fn no_reference_in_an_expression_without_one() {
    let source = "Patient.name.family";
    assert!(refs(source).is_empty());
}

#[test]
fn two_occurrences_of_the_same_constant_are_two_references() {
    let source = "%dup = 1 or %dup = 2";
    let found = refs(source);
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|r| r.name == "dup"));
    // Distinct positions, even though the name repeats.
    assert_ne!(found[0].span.position, found[1].span.position);
}

#[test]
fn bare_identifier_span_covers_exactly_percent_plus_name() {
    let source = "true and %foo";
    let found = refs(source);
    assert_eq!(found.len(), 1);
    assert_eq!(spanned_text(source, &found[0]), "%foo");
}

#[test]
fn trailing_whitespace_is_never_included_in_the_span() {
    // Regression: spanned_parser()'s token combinators all end in
    // `.padded()`, so a naive use of the raw ExprSpan would swallow the
    // trailing whitespace/comment here into what should be a 4-byte span.
    let source = "(  %foo  )";
    let found = refs(source);
    assert_eq!(found.len(), 1);
    assert_eq!(spanned_text(source, &found[0]), "%foo");
}

#[test]
fn backtick_delimited_identifier_span_includes_both_backticks() {
    let source = "%`a weird name`";
    let found = refs(source);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "a weird name");
    assert_eq!(spanned_text(source, &found[0]), "%`a weird name`");
}

#[test]
fn quoted_string_form_span_includes_both_quotes() {
    let source = "%'a weird name'";
    let found = refs(source);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "a weird name");
    assert_eq!(spanned_text(source, &found[0]), "%'a weird name'");
}

#[test]
fn span_is_correct_when_multibyte_characters_precede_it() {
    // "café" is 4 chars / 5 bytes; the reference's byte span must start
    // right after it, not after where it would start if é were 1 byte.
    let source = "'café' & %foo";
    let found = refs(source);
    assert_eq!(found.len(), 1);
    assert_eq!(spanned_text(source, &found[0]), "%foo");
    assert_eq!(found[0].span.position, "'café' & ".len());
}
