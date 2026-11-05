//! Primitive value validation: JSON type-class and regex checks.
//!
//! Runs when a primitive-type schema is a member of a node's cooperative
//! set. The type class comes from the primitive's declared `type` (falling
//! back to `name`); the regex is the FHIR spec pattern carried into the
//! packs by the converter. Upstream conformance fixtures declare primitives
//! as bare `{kind: "primitive-type"}` — no type, no regex — so neither
//! check can fire there and the exact-match contract is unaffected.
//!
//! FHIR spec regexes are implicitly anchored: they must match the whole
//! value, so patterns are compiled as `^(?:...)$`.
//!
//! # Why the shorthand classes are compiled as ASCII
//!
//! The regexes are not the authority on what a primitive may hold. FHIR says
//! so outright: "The regexes are provided to assist with tooling, but are
//! informative, not normative." The normative definition of `string` is "a
//! sequence of Unicode characters", capped at 1,048,576 characters, with prose
//! that it SHOULD NOT carry code points below 32 other than tab, CR and LF.
//!
//! Rust's `regex` makes `\s`/`\S` Unicode-aware, which put the engine at odds
//! with that normative text: U+00A0 (non-breaking space) is Unicode whitespace,
//! so it matched neither the literal `[ \r\n\t]` branch nor `\S` in the R4/R4B
//! `string`/`markdown` pattern `[ \r\n\t\S]+`, and a plainly valid FHIR string
//! was reported invalid (issue #425). `code` (`[^\s]+...`, every version) had
//! the same defect: FHIR's "no whitespace other than single spaces" means XML
//! whitespace, and U+00A0 is deliberately not in that set.
//!
//! Compiling with `unicode(false)` restores the intended reading. Because that
//! flag on the `&str` engine rejects any program that could match invalid UTF-8
//! (which `\S`/`[^\s]` can), the patterns are compiled on the `regex::bytes`
//! engine and matched against the value's UTF-8 bytes. The value always comes
//! from `Value::as_str()`, so it is always valid UTF-8.
//!
//! ## The invariant this relies on
//!
//! Byte matching with ASCII classes is only equivalent to codepoint matching
//! because of two properties, which hold across every primitive pattern shipped
//! in all four packs:
//!
//! - `\s` and `\S` are the only shorthand classes that appear. Nothing uses
//!   `.`, `\d`, `\w`, a backreference, or a lookaround.
//! - Every bounded quantifier (`id`'s `{1,64}`, `base64Binary`'s `{4}`, the
//!   date/time and uuid groups) is applied to an explicit ASCII class, where
//!   one byte is one character, so the counts cannot drift.
//!
//! Note that this is *not* strict XSD semantics, and should not be described as
//! such. XSD defines only `\s` as ASCII (`[#x20#x9#xD#xA]`); its `\d` is
//! `\p{Nd}` and its `\w` is Unicode-aware, both of which `unicode(false)`
//! narrows to ASCII. Rust's ASCII `\s` is also `[\t\n\v\f\r ]`, which is wider
//! than XSD's by VT and FF. Neither divergence is reachable through the shipped
//! patterns — that is exactly what the invariant above asserts — but a pack
//! carrying a `\d`/`\w` pattern in some future primitive would land outside it,
//! so the invariant has to be rechecked when the packs change.

use super::errors::{self, ErrorKind};
use super::walk::WalkCtx;
use crate::schema::FhirSchema;
use regex::bytes::{Regex, RegexBuilder};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

/// Validate a data value against a primitive-type schema. `Null` is skipped:
/// legal as a sidecar-array gap, and shape errors are reported elsewhere.
pub(super) fn validate_primitive(ctx: &mut WalkCtx<'_>, schema: &FhirSchema, data: &Value) {
    if data.is_null() || data.is_array() {
        return;
    }
    let Some(type_name) = schema.type_.as_deref().or(schema.name.as_deref()) else {
        return; // bare fixture primitive — nothing to check against
    };

    if let Some(expected) = expected_json_class(type_name)
        && !expected.matches(data)
    {
        ctx.error(
            ErrorKind::PrimitiveValue,
            errors::msg_primitive_type(type_name, errors::json_type_name(data)),
        );
        return;
    }

    if let Some(pattern) = &schema.regex
        && let Some(s) = data.as_str()
        && let Some(re) = compiled(pattern)
        && !re.is_match(s.as_bytes())
    {
        ctx.error(
            ErrorKind::PrimitiveValue,
            errors::msg_primitive_regex(type_name, s),
        );
    }
}

/// The JSON type class a FHIR primitive requires.
#[derive(Clone, Copy)]
enum JsonClass {
    Boolean,
    Integer,
    Number,
    String,
}

impl JsonClass {
    fn matches(self, v: &Value) -> bool {
        match self {
            JsonClass::Boolean => v.is_boolean(),
            JsonClass::Integer => v.as_number().is_some_and(|n| n.is_i64() || n.is_u64()),
            JsonClass::Number => v.is_number(),
            JsonClass::String => v.is_string(),
        }
    }
}

fn expected_json_class(type_name: &str) -> Option<JsonClass> {
    Some(match type_name {
        "boolean" => JsonClass::Boolean,
        "integer" | "positiveInt" | "unsignedInt" | "integer64" => JsonClass::Integer,
        "decimal" => JsonClass::Number,
        // Everything else — string, uri, code, dateTime, base64Binary, ... —
        // is a JSON string. Unknown primitive names get no class check.
        "string" | "uri" | "url" | "canonical" | "oid" | "uuid" | "id" | "code" | "markdown"
        | "base64Binary" | "instant" | "date" | "dateTime" | "time" | "xhtml" => JsonClass::String,
        _ => return None,
    })
}

/// Process-wide compiled-regex cache (patterns repeat across every node of
/// every resource). Invalid patterns are cached as misses so they are only
/// reported... never: they are simply skipped — the converter emits spec
/// patterns, and a bad pattern must not fail validation.
///
/// Patterns are compiled on the byte engine with `unicode(false)`, so `\s`/`\S`
/// are ASCII; see the module docs for why, and for the invariant over the
/// shipped patterns that makes byte matching equivalent. The cache key is the
/// raw pattern string: every entry is compiled under the same fixed flag set,
/// so the pattern alone is a total key.
fn compiled(pattern: &str) -> Option<Arc<Regex>> {
    static CACHE: OnceLock<RwLock<HashMap<String, Option<Arc<Regex>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| RwLock::new(HashMap::new()));

    if let Some(hit) = cache.read().expect("regex cache lock").get(pattern) {
        return hit.clone();
    }
    let compiled = RegexBuilder::new(&format!("^(?:{pattern})$"))
        .unicode(false)
        .build()
        .ok()
        .map(Arc::new);
    cache
        .write()
        .expect("regex cache lock")
        .insert(pattern.to_string(), compiled.clone());
    compiled
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Convenience: does the compiled, anchored pattern match the whole value?
    /// Mirrors the engine, which matches the value's UTF-8 bytes.
    fn matches(pattern: &str, value: &str) -> bool {
        compiled(pattern).unwrap().is_match(value.as_bytes())
    }

    #[test]
    fn anchoring_is_whole_value() {
        let re = compiled("[0-9]+").unwrap();
        assert!(re.is_match(b"123"));
        assert!(!re.is_match(b"a123"));
        assert!(!re.is_match(b"123b"));
    }

    #[test]
    fn invalid_pattern_is_skipped() {
        assert!(compiled("([unclosed").is_none());
    }

    // Rust's `regex` makes `\s`/`\S` Unicode-aware by default, which wrongly
    // rejected values containing U+00A0 (non-breaking space) — a valid FHIR
    // string under the normative "a sequence of Unicode characters" definition.
    // #425. The engine now compiles the shorthand classes as ASCII; these pin
    // that behaviour on the patterns the packs actually ship.

    /// The R4/R4B `string`/`markdown` pattern. `\S` must be ASCII, so U+00A0 is
    /// a non-whitespace codepoint and a valid string. This is the exact case
    /// from the issue (`...Program\u{a0}`).
    #[test]
    fn string_markdown_r4_accepts_non_breaking_space() {
        let p = r"[ \r\n\t\S]+";
        assert!(matches(p, "Acquired Brain Injury (ABI) Program\u{a0}"));
        assert!(matches(p, "a\u{a0}b"));
        assert!(matches(p, "\u{a0}")); // U+00A0 on its own is a valid string
        assert!(matches(p, "a b")); // an ordinary space is still allowed
        assert!(matches(p, "a\tb"));
        assert!(!matches(p, "")); // but empty is not a valid string
    }

    /// The R5/R6 `string`/`markdown` pattern (`[\s\S]` = any codepoint) was
    /// never affected; behaviour must be unchanged either way.
    #[test]
    fn string_markdown_r5_unchanged() {
        let p = r"^[\s\S]+$";
        assert!(matches(p, "anything\u{a0}"));
        assert!(matches(p, "plain"));
        assert!(!matches(p, ""));
    }

    /// `code` (all versions) also used `[^\s]`, which under Unicode excluded
    /// U+00A0. With ASCII semantics U+00A0 is content, so a token containing it
    /// is one token; real ASCII whitespace still delimits/rejects as before.
    #[test]
    fn code_treats_non_breaking_space_as_content() {
        for p in [r"[^\s]+(\s[^\s]+)*", r"[^\s]+( [^\s]+)*"] {
            assert!(matches(p, "a\u{a0}b"), "U+00A0 is content in {p}");
            assert!(matches(p, "abc"));
            assert!(matches(p, "ab cd")); // space-separated tokens are valid
            assert!(!matches(p, " abc")); // leading real space is not
            assert!(!matches(p, "abc ")); // trailing real space is not
        }
    }

    /// `uri`/`url`/`canonical` is `\S*`: a real space is rejected, U+00A0 is
    /// accepted (not ASCII whitespace), and empty is allowed by `*`. The regex
    /// was never RFC 3986 enforcement — it admits any space-free string — so
    /// the widening here costs nothing that was being caught.
    #[test]
    fn uri_star_s_semantics() {
        let p = r"\S*";
        assert!(matches(p, "http://example.org/a\u{a0}b"));
        assert!(!matches(p, "a b"));
        assert!(matches(p, ""));
    }

    /// The fix must NOT blanket-accept non-ASCII: patterns built from explicit
    /// ASCII classes still reject a stray U+00A0. Guards against over-permissive
    /// byte matching.
    #[test]
    fn explicit_ascii_patterns_still_reject_non_breaking_space() {
        // date (R4): U+00A0 is not part of any date class.
        let date = r"([0-9]([0-9]([0-9][1-9]|[1-9]0)|[1-9]00)|[1-9]000)(-(0[1-9]|1[0-2])(-(0[1-9]|[1-2][0-9]|3[0-1]))?)?";
        assert!(matches(date, "2013-06-08"));
        assert!(!matches(date, "2013-06-08\u{a0}"));
        // id: `[A-Za-z0-9\-\.]{1,64}` — U+00A0 is not allowed.
        let id = r"[A-Za-z0-9\-\.]{1,64}";
        assert!(matches(id, "abc-1.2"));
        assert!(!matches(id, "abc\u{a0}"));
    }

    /// `base64Binary` (R4/R4B) uses a `{4}` quantifier over an ASCII class; byte
    /// counting equals char counting here, so grouping is unaffected.
    #[test]
    fn base64_quantifier_unaffected() {
        let p = r"(\s*([0-9a-zA-Z\+/=]){4}\s*)+";
        assert!(matches(p, "AAAA"));
        assert!(matches(p, "aGVsbG8="));
        assert!(!matches(p, "AA")); // not a multiple of 4
    }
}
