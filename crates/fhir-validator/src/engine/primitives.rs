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

use super::errors::{self, ErrorKind};
use super::walk::WalkCtx;
use crate::schema::FhirSchema;
use regex::Regex;
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
        && !re.is_match(s)
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
fn compiled(pattern: &str) -> Option<Arc<Regex>> {
    static CACHE: OnceLock<RwLock<HashMap<String, Option<Arc<Regex>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| RwLock::new(HashMap::new()));

    if let Some(hit) = cache.read().expect("regex cache lock").get(pattern) {
        return hit.clone();
    }
    let compiled = Regex::new(&format!("^(?:{pattern})$")).ok().map(Arc::new);
    cache
        .write()
        .expect("regex cache lock")
        .insert(pattern.to_string(), compiled.clone());
    compiled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchoring_is_whole_value() {
        let re = compiled("[0-9]+").unwrap();
        assert!(re.is_match("123"));
        assert!(!re.is_match("a123"));
        assert!(!re.is_match("123b"));
    }

    #[test]
    fn invalid_pattern_is_skipped() {
        assert!(compiled("([unclosed").is_none());
    }
}
