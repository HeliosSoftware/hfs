//! Tests for `helios_fhirpath::parse_expression_spanned` and
//! `helios_fhirpath::expr_span_to_char_offsets` — the spanned parser entry
//! point and byte→char span conversion that `helios_sof::lint`'s
//! undeclared-constant check (#821) builds on.

use helios_fhirpath::{expr_span_to_char_offsets, parse_expression_spanned};

#[test]
fn valid_expression_parses_ok() {
    let spanned = parse_expression_spanned("Patient.name.family").unwrap();
    assert_eq!(spanned.span.position, 0);
    assert_eq!(spanned.span.length, "Patient.name.family".len());
}

#[test]
fn syntax_error_reports_the_same_diagnostics_as_parse_expression_diagnostics() {
    let spanned_errors = parse_expression_spanned("Patient.name.").unwrap_err();
    let plain_errors = helios_fhirpath::parse_expression_diagnostics("Patient.name.").unwrap_err();
    assert_eq!(spanned_errors, plain_errors);
}

#[test]
fn empty_expression_is_a_parse_error() {
    assert!(parse_expression_spanned("").is_err());
}

#[test]
fn parse_expression_and_parse_expression_diagnostics_are_unchanged() {
    // NF1: parse_expression_spanned is purely additive.
    assert!(helios_fhirpath::parse_expression("Patient.name.family").is_ok());
    assert!(helios_fhirpath::parse_expression_diagnostics("Patient.name.family").is_ok());
}

#[test]
fn expr_span_to_char_offsets_matches_byte_offsets_for_ascii() {
    let expression = "Patient.name.family";
    let spanned = parse_expression_spanned(expression).unwrap();
    let (start, end) = expr_span_to_char_offsets(expression, &spanned.span);
    assert_eq!((start, end), (0, expression.len()));
}

#[test]
fn expr_span_to_char_offsets_accounts_for_multibyte_characters_before_the_span() {
    // "café" is 4 chars / 5 bytes (é is a 2-byte UTF-8 sequence), so any span
    // starting after it must differ between byte and char offsets.
    let expression = "'café' & %foo";
    let char_len = expression.chars().count();
    assert_eq!(char_len, 13);
    assert_eq!(expression.len(), 14); // sanity: bytes really do differ here.

    let spanned = parse_expression_spanned(expression).unwrap();
    // The raw (byte) span covers the whole 14-byte expression...
    assert_eq!(spanned.span.position + spanned.span.length, 14);
    // ...but converted to chars, it must never exceed the 13-char length.
    let (start, end) = expr_span_to_char_offsets(expression, &spanned.span);
    assert_eq!(start, 0);
    assert_eq!(end, char_len);
}
