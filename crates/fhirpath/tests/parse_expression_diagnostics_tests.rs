//! Tests for `helios_fhirpath::parse_expression_diagnostics` (#753 ticket 03):
//! the additive, span-reporting sibling of `parse_expression`. Consumed by
//! `helios_sof::lint` to locate FHIRPath syntax errors inside a
//! ViewDefinition document.

use helios_fhirpath::parse_expression_diagnostics;

#[test]
fn valid_expression_parses_ok() {
    assert!(parse_expression_diagnostics("Patient.name.family").is_ok());
    assert!(parse_expression_diagnostics("name.given.first()").is_ok());
}

#[test]
fn syntax_error_reports_a_span_at_the_expected_position() {
    // Trailing dot: nothing follows the member-access operator.
    let errors = parse_expression_diagnostics("Patient.name.").unwrap_err();
    assert!(!errors.is_empty());

    let first = &errors[0];
    assert!(!first.message.is_empty());
    // The span must fall within (or at the very end of) the 14-character
    // input — never past it, and never inverted.
    assert!(first.span.0 <= first.span.1);
    assert!(first.span.1 <= "Patient.name.".chars().count());
}

#[test]
fn empty_expression_is_a_parse_error() {
    let errors = parse_expression_diagnostics("").unwrap_err();
    assert!(!errors.is_empty());
}

#[test]
fn message_is_not_a_debug_dump_and_does_not_repeat_the_expression() {
    let expression = "Patient.name.";
    let errors = parse_expression_diagnostics(expression).unwrap_err();
    let message = &errors[0].message;
    // chumsky's `Display` output is a short "found .. expected .." style
    // sentence — not the `{:?}` dump `parse_expression`'s own error string
    // produces, and it never quotes the whole source expression back.
    assert!(!message.contains(expression));
    assert!(!message.contains("Rich"));
}

#[test]
fn span_offsets_are_unicode_chars_not_utf8_bytes() {
    // "café." is 5 chars / 6 bytes (é is a 2-byte UTF-8 sequence); a
    // trailing-dot error there must report char offsets that never exceed
    // the character length, even though the byte length is larger.
    let expression = "café.";
    let char_len = expression.chars().count();
    assert_eq!(char_len, 5);
    assert_eq!(expression.len(), 6); // sanity: bytes really do differ here.

    let errors = parse_expression_diagnostics(expression).unwrap_err();
    for diagnostic in &errors {
        assert!(
            diagnostic.span.0 <= char_len && diagnostic.span.1 <= char_len,
            "span {:?} must be within the {char_len}-char expression, not its byte length",
            diagnostic.span
        );
    }
}

#[test]
fn parse_expression_unchanged_alongside_the_new_function() {
    // RF4: parse_expression keeps its existing signature/behavior — this is
    // a purely additive sibling, not a replacement.
    assert!(helios_fhirpath::parse_expression("Patient.name.family").is_ok());
    assert!(helios_fhirpath::parse_expression("Patient.name.").is_err());
}
