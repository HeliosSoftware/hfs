//! Tests for FHIRPath string functions operating on Unicode scalar values
//! per FHIR-53554 (https://jira.hl7.org/browse/FHIR-53554).
//!
//! String functions such as `length()`, `indexOf()`, `lastIndexOf()`,
//! `substring()`, the indexer, `toChars()`, and `replace()` must operate on
//! characters (Unicode scalar values), not on UTF-8 bytes, UTF-16 code units,
//! or grapheme clusters. UTF-16 surrogate escape pairs (`\uD83D\uDD25`) must
//! combine into a single scalar value.

use chumsky::Parser;
use helios_fhirpath::evaluator::{EvaluationContext, evaluate};
use helios_fhirpath::parser::parser;
use helios_fhirpath_support::{EvaluationError, EvaluationResult};

fn eval(input: &str, context: &EvaluationContext) -> Result<EvaluationResult, EvaluationError> {
    let expr = parser()
        .parse(input)
        .into_result()
        .unwrap_or_else(|e| panic!("Parser error for input '{}': {:?}", input, e));
    evaluate(&expr, context, None)
}

#[test]
fn surrogate_pair_escape_combines_into_scalar_value() {
    let ctx = EvaluationContext::new_empty_with_default_version();
    // \uD83D\uDD25 is the UTF-16 surrogate pair for U+1F525 (🔥).
    assert_eq!(
        eval("'\\uD83D\\uDD25'", &ctx).unwrap(),
        EvaluationResult::string("🔥".to_string())
    );
}

#[test]
fn length_counts_scalar_values_not_utf8_bytes() {
    let ctx = EvaluationContext::new_empty_with_default_version();
    // Surrogate-pair escape is one scalar value.
    assert_eq!(
        eval("'\\uD83D\\uDD25'.length()", &ctx).unwrap(),
        EvaluationResult::integer(1)
    );
    // Combining form 'é' (U+0065 + U+0301) is two scalar values.
    assert_eq!(
        eval("'\\u0065\\u0301'.length()", &ctx).unwrap(),
        EvaluationResult::integer(2)
    );
    // Precomposed 'é' (U+00E9) is one scalar value.
    assert_eq!(
        eval("'\\u00E9'.length()", &ctx).unwrap(),
        EvaluationResult::integer(1)
    );
}

#[test]
fn index_of_returns_character_index_not_byte_offset() {
    let ctx = EvaluationContext::new_empty_with_default_version();
    // Literal emoji in source text.
    assert_eq!(
        eval("'a🔥b'.indexOf('🔥')", &ctx).unwrap(),
        EvaluationResult::integer(1)
    );
    // 'b' is at character index 2, not byte offset 5.
    assert_eq!(
        eval("'a🔥b'.indexOf('b')", &ctx).unwrap(),
        EvaluationResult::integer(2)
    );
    // Same string constructed via surrogate pair escape.
    assert_eq!(
        eval("'a\\uD83D\\uDD25b'.indexOf('b')", &ctx).unwrap(),
        EvaluationResult::integer(2)
    );
}

#[test]
fn last_index_of_returns_character_index() {
    let ctx = EvaluationContext::new_empty_with_default_version();
    assert_eq!(
        eval("'a🔥b🔥c'.lastIndexOf('🔥')", &ctx).unwrap(),
        EvaluationResult::integer(3)
    );
    assert_eq!(
        eval("'a🔥b🔥c'.lastIndexOf('c')", &ctx).unwrap(),
        EvaluationResult::integer(4)
    );
}

#[test]
fn substring_operates_on_characters() {
    let ctx = EvaluationContext::new_empty_with_default_version();
    // substring(start, length) in characters, not bytes.
    assert_eq!(
        eval("'a🔥b'.substring(1, 1)", &ctx).unwrap(),
        EvaluationResult::string("🔥".to_string())
    );
    assert_eq!(
        eval("'a🔥b'.substring(2, 1)", &ctx).unwrap(),
        EvaluationResult::string("b".to_string())
    );
}

#[test]
fn indexer_returns_scalar_value_character() {
    let ctx = EvaluationContext::new_empty_with_default_version();
    assert_eq!(
        eval("'a🔥b'[0]", &ctx).unwrap(),
        EvaluationResult::string("a".to_string())
    );
    assert_eq!(
        eval("'a🔥b'[1]", &ctx).unwrap(),
        EvaluationResult::string("🔥".to_string())
    );
    assert_eq!(
        eval("'a🔥b'[2]", &ctx).unwrap(),
        EvaluationResult::string("b".to_string())
    );
    assert_eq!(eval("'a🔥b'[3]", &ctx).unwrap(), EvaluationResult::Empty);
}

#[test]
fn to_chars_does_not_split_surrogate_pairs() {
    let ctx = EvaluationContext::new_empty_with_default_version();
    // 'a🔥b' (surrogate-pair form) should yield three single-char strings.
    let result = eval("'a\\uD83D\\uDD25b'.toChars()", &ctx).unwrap();
    match result {
        EvaluationResult::Collection { items, .. } => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], EvaluationResult::string("a".to_string()));
            assert_eq!(items[1], EvaluationResult::string("🔥".to_string()));
            assert_eq!(items[2], EvaluationResult::string("b".to_string()));
        }
        other => panic!("Expected Collection, got {:?}", other),
    }

    // Combining 'é' (U+0065 + U+0301) yields two entries.
    let result = eval("'\\u0065\\u0301'.toChars()", &ctx).unwrap();
    match result {
        EvaluationResult::Collection { items, .. } => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], EvaluationResult::string("e".to_string()));
            assert_eq!(items[1], EvaluationResult::string("\u{0301}".to_string()));
        }
        other => panic!("Expected Collection, got {:?}", other),
    }
}

#[test]
fn replace_with_empty_pattern_preserves_scalar_values() {
    let ctx = EvaluationContext::new_empty_with_default_version();
    // Inserting 'x' between every character must not split a surrogate pair.
    assert_eq!(
        eval("'a🔥c'.replace('', 'x')", &ctx).unwrap(),
        EvaluationResult::string("xax🔥xcx".to_string())
    );
}

#[test]
fn unpaired_high_surrogate_is_rejected() {
    // `\uD83D` alone is an unpaired high surrogate and must fail to parse.
    let result = parser().parse("'\\uD83D'").into_result();
    assert!(
        result.is_err(),
        "Parser unexpectedly accepted unpaired high surrogate: {:?}",
        result
    );
}

#[test]
fn unpaired_low_surrogate_is_rejected() {
    // `\uDD25` alone is an unpaired low surrogate and must fail to parse.
    let result = parser().parse("'\\uDD25'").into_result();
    assert!(
        result.is_err(),
        "Parser unexpectedly accepted unpaired low surrogate: {:?}",
        result
    );
}
