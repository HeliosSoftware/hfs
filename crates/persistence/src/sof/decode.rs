//! Per-column decoding of the text values the in-DB runners read back.
//!
//! Both the SQLite and PostgreSQL runners receive most column values as text
//! (PostgreSQL always does; SQLite does for strings, typed booleans and any
//! compound expression). Parsing every text value as JSON turns a `code` such
//! as `"44054006"` into a number and `"null"` into JSON `null`, so the
//! compiler records, per output column, how its text must be decoded and the
//! row mappers apply that decision through [`decode_text`].

use serde_json::Value;

/// How the text value of one output column is turned into a JSON value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnDecode {
    /// Collection or complex-typed column: the text is JSON and is parsed,
    /// falling back to a string when it is not valid JSON.
    Json,
    /// String-like column (`string`, `code`, `id`, `uri`, dates, ...): the
    /// text is always a JSON string, whatever it looks like.
    Text,
    /// `boolean` column: `'true'` / `'false'` become JSON booleans.
    Boolean,
    /// Integer column: numeric text becomes a JSON number.
    Integer,
    /// Decimal column: numeric text becomes a JSON number.
    Decimal,
    /// Untyped column whose type could not be inferred. Parses the text as
    /// JSON with a string fallback, the behaviour that predates per-column
    /// decoding.
    #[default]
    Auto,
}

impl ColumnDecode {
    /// Decode mode for a column from its declared `type` and `collection` flag.
    ///
    /// A collection column is always [`Json`](Self::Json). A column without a
    /// declared type is [`Auto`](Self::Auto); the compiler may refine it by
    /// inference. A declared type outside the FHIR primitives is treated as
    /// complex ([`Json`](Self::Json)).
    pub fn from_declared(declared: Option<&str>, collection: bool) -> Self {
        if collection {
            return Self::Json;
        }
        match declared {
            None => Self::Auto,
            Some(t) => match Self::from_fhir_type(t) {
                Self::Auto => Self::Json,
                other => other,
            },
        }
    }

    /// Decode mode for a FHIR type name as returned by the generated
    /// field-type tables.
    ///
    /// The tables mix vocabularies: FHIR primitive codes (`code`, `uri`),
    /// FHIRPath System names (`String`, `Boolean`) and generated names for
    /// complex, backbone and choice types (`CodeableConcept`, `PatientContact`).
    /// A name that is none of these returns [`Auto`](Self::Auto), never
    /// [`Text`](Self::Text).
    pub fn from_fhir_type(name: &str) -> Self {
        match name {
            "string" | "code" | "id" | "uri" | "url" | "canonical" | "oid" | "uuid"
            | "markdown" | "date" | "dateTime" | "instant" | "time" | "base64Binary"
            | "integer64" | "xhtml" | "String" => Self::Text,
            "boolean" | "Boolean" => Self::Boolean,
            "integer" | "positiveInt" | "unsignedInt" | "Integer" => Self::Integer,
            "decimal" | "Decimal" => Self::Decimal,
            // Remaining System names: leave them as they were.
            "Date" | "DateTime" | "Time" | "Quantity" => Self::Auto,
            other if other.chars().next().is_some_and(|c| c.is_ascii_uppercase()) => Self::Json,
            _ => Self::Auto,
        }
    }

    /// Reconciles the decode modes of the same column in two `unionAll`
    /// branches: equal modes are kept, different ones degrade to
    /// [`Auto`](Self::Auto).
    pub fn merge(self, other: Self) -> Self {
        if self == other { self } else { Self::Auto }
    }
}

/// Converts the text of a column value into JSON according to `decode`.
pub fn decode_text(decode: ColumnDecode, text: String) -> Value {
    match decode {
        ColumnDecode::Text => Value::String(text),
        ColumnDecode::Boolean => match text.as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => parse_or_string(text),
        },
        ColumnDecode::Integer | ColumnDecode::Decimal => match serde_json::from_str(&text) {
            Ok(v @ Value::Number(_)) => v,
            _ => Value::String(text),
        },
        ColumnDecode::Json | ColumnDecode::Auto => parse_or_string(text),
    }
}

fn parse_or_string(text: String) -> Value {
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_keeps_json_looking_strings() {
        for s in [
            "44054006", "0123", "true", "false", "null", "1e3", "[1]", "{}",
        ] {
            assert_eq!(
                decode_text(ColumnDecode::Text, s.to_string()),
                Value::String(s.to_string()),
                "{s}"
            );
        }
    }

    #[test]
    fn json_and_auto_parse_with_string_fallback() {
        for d in [ColumnDecode::Json, ColumnDecode::Auto] {
            assert_eq!(decode_text(d, "[1,2]".into()), serde_json::json!([1, 2]));
            assert_eq!(
                decode_text(d, "44054006".into()),
                serde_json::json!(44054006)
            );
            assert_eq!(
                decode_text(d, "4548-4".into()),
                Value::String("4548-4".into())
            );
        }
    }

    #[test]
    fn boolean_decodes_true_false_only() {
        assert_eq!(
            decode_text(ColumnDecode::Boolean, "true".into()),
            Value::Bool(true)
        );
        assert_eq!(
            decode_text(ColumnDecode::Boolean, "false".into()),
            Value::Bool(false)
        );
    }

    #[test]
    fn numeric_decodes_only_numbers() {
        assert_eq!(
            decode_text(ColumnDecode::Integer, "42".into()),
            serde_json::json!(42)
        );
        assert!(decode_text(ColumnDecode::Decimal, "1.5".into()).is_number());
        assert_eq!(
            decode_text(ColumnDecode::Integer, "true".into()),
            Value::String("true".into())
        );
        assert_eq!(
            decode_text(ColumnDecode::Decimal, "null".into()),
            Value::String("null".into())
        );
    }

    #[test]
    fn declared_types_map_to_modes() {
        let d = |t| ColumnDecode::from_declared(Some(t), false);
        for t in [
            "string",
            "code",
            "id",
            "uri",
            "date",
            "dateTime",
            "integer64",
        ] {
            assert_eq!(d(t), ColumnDecode::Text, "{t}");
        }
        assert_eq!(d("boolean"), ColumnDecode::Boolean);
        assert_eq!(d("integer"), ColumnDecode::Integer);
        assert_eq!(d("positiveInt"), ColumnDecode::Integer);
        assert_eq!(d("decimal"), ColumnDecode::Decimal);
        assert_eq!(d("Coding"), ColumnDecode::Json);
        assert_eq!(d("somethingElse"), ColumnDecode::Json);
        assert_eq!(ColumnDecode::from_declared(None, false), ColumnDecode::Auto);
        assert_eq!(
            ColumnDecode::from_declared(Some("code"), true),
            ColumnDecode::Json
        );
        assert_eq!(ColumnDecode::from_declared(None, true), ColumnDecode::Json);
    }

    #[test]
    fn fhir_type_vocabulary() {
        assert_eq!(ColumnDecode::from_fhir_type("String"), ColumnDecode::Text);
        assert_eq!(ColumnDecode::from_fhir_type("code"), ColumnDecode::Text);
        assert_eq!(
            ColumnDecode::from_fhir_type("Boolean"),
            ColumnDecode::Boolean
        );
        assert_eq!(
            ColumnDecode::from_fhir_type("CodeableConcept"),
            ColumnDecode::Json
        );
        assert_eq!(
            ColumnDecode::from_fhir_type("PatientContact"),
            ColumnDecode::Json
        );
        assert_eq!(ColumnDecode::from_fhir_type("mystery"), ColumnDecode::Auto);
    }

    #[test]
    fn merge_degrades_to_auto() {
        assert_eq!(
            ColumnDecode::Text.merge(ColumnDecode::Text),
            ColumnDecode::Text
        );
        assert_eq!(
            ColumnDecode::Text.merge(ColumnDecode::Json),
            ColumnDecode::Auto
        );
    }
}
