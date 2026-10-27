//! Shared SoF v2 `ViewDefinition.constant[]` parsing.
//!
//! Both the in-process FHIRPath evaluator (this crate's
//! [`crate::run_view_definition`]) and the in-DB SQL compiler in
//! `helios-persistence` walk `ViewDefinition.constant[]` and interpret the same
//! `value[X]` field family per the SoF v2 spec. This module is the single
//! source of truth for that field list and primitive recognition, so a new
//! primitive only needs to be added in one place.
//!
//! Engines convert from [`ConstantValue`] into their own value types:
//! - `helios-sof` builds an [`EvaluationResult`] via
//!   [`ConstantValue::to_evaluation_result`] for the in-process FHIRPath
//!   evaluator. The four per-version `ViewDefinitionConstantTrait` impls in
//!   [`crate::traits`] map their typed `ViewDefinitionConstantValue` variants
//!   into [`ConstantValue`] and call this method.
//! - `helios-persistence` walks `serde_json::Value` and calls
//!   [`parse_constant_from_json`], then lifts to its `LitValue`.

use helios_fhirpath_support::{EvaluationResult, TypeInfoResult};
use serde_json::Value;

use crate::SofError;

/// Neutral SoF constant value covering every `value[X]` primitive family
/// the SoF v2 spec allows for `ViewDefinition.constant[]`.
///
/// Stringly typed for the date/time/decimal families so engines can preserve
/// the lexical form (decimal precision; pre-prefixed `@`/`@T` literals).
#[derive(Debug, Clone, PartialEq)]
pub enum ConstantValue {
    /// `valueString`.
    String(String),
    /// `valueCode` — bound as text in FHIRPath/SQL.
    Code(String),
    /// `valueId`, `valueUri`, `valueUrl`, `valueOid`, `valueUuid`,
    /// `valueCanonical` — all bind as text.
    Identifier(String),
    /// `valueBase64Binary`.
    Base64Binary(String),
    /// `valueMarkdown` (currently only surfaced from the JSON path; no typed
    /// variant exists in any FHIR version's ViewDefinitionConstantValue yet).
    Markdown(String),
    /// `valueBoolean`.
    Boolean(bool),
    /// `valueInteger`.
    Integer(i64),
    /// `valuePositiveInt` (FHIR 1..*) — surfaces as Integer in FHIRPath.
    PositiveInt(i64),
    /// `valueUnsignedInt` (FHIR 0..*) — surfaces as Integer in FHIRPath.
    UnsignedInt(i64),
    /// `valueInteger64` (R5+). Surfaces as Integer64 in FHIRPath.
    Integer64(i64),
    /// `valueDecimal` — kept as its lexical form so precision survives the
    /// trip through FHIRPath / SQL parameter binding.
    Decimal(String),
    /// `valueDate`.
    Date(String),
    /// `valueDateTime` — may or may not be `@`-prefixed; normalised in
    /// [`Self::to_evaluation_result`].
    DateTime(String),
    /// `valueTime` — may or may not be `@T`-prefixed; normalised in
    /// [`Self::to_evaluation_result`].
    Time(String),
    /// `valueInstant` — surfaces as `EvaluationResult::DateTime` tagged with
    /// FHIR `instant`.
    Instant(String),
}

impl ConstantValue {
    /// Renders this constant as an [`EvaluationResult`] for the in-process
    /// FHIRPath evaluator. Handles `@` / `@T` literal prefixing and decimal
    /// precision parsing. Returns `Err` only when a [`Self::Decimal`] lexical
    /// form fails to parse.
    pub fn to_evaluation_result(&self) -> Result<EvaluationResult, SofError> {
        Ok(match self {
            ConstantValue::String(s)
            | ConstantValue::Code(s)
            | ConstantValue::Identifier(s)
            | ConstantValue::Base64Binary(s)
            | ConstantValue::Markdown(s) => EvaluationResult::String(s.clone(), None, None),
            ConstantValue::Boolean(b) => EvaluationResult::Boolean(*b, None, None),
            ConstantValue::Integer(i)
            | ConstantValue::PositiveInt(i)
            | ConstantValue::UnsignedInt(i) => EvaluationResult::Integer(*i, None, None),
            ConstantValue::Integer64(i) => EvaluationResult::Integer64(*i, None, None),
            ConstantValue::Decimal(s) => {
                let parsed = s.parse().map_err(|_| {
                    SofError::InvalidViewDefinition(format!("Invalid decimal value '{s}'"))
                })?;
                EvaluationResult::Decimal(parsed, None, None)
            }
            ConstantValue::Date(s) => EvaluationResult::Date(s.clone(), None, None),
            ConstantValue::DateTime(s) => EvaluationResult::DateTime(
                prefix_at(s),
                Some(TypeInfoResult::new("FHIR", "dateTime")),
                None,
            ),
            ConstantValue::Time(s) => EvaluationResult::Time(prefix_at_t(s), None, None),
            ConstantValue::Instant(s) => EvaluationResult::DateTime(
                prefix_at(s),
                Some(TypeInfoResult::new("FHIR", "instant")),
                None,
            ),
        })
    }
}

fn prefix_at(s: &str) -> String {
    if s.starts_with('@') {
        s.to_string()
    } else {
        format!("@{s}")
    }
}

fn prefix_at_t(s: &str) -> String {
    if s.starts_with("@T") {
        s.to_string()
    } else {
        format!("@T{s}")
    }
}

/// Parses a raw JSON `ViewDefinition.constant[]` entry into `(name, value)`.
///
/// Used by the in-DB compiler which walks the ViewDefinition as
/// `serde_json::Value`. The in-process evaluator walks typed FHIR structs
/// instead and converts through the per-version trait impls.
///
/// Errors when `name` is missing or no recognised `value[X]` field is present.
pub fn parse_constant_from_json(c: &Value) -> Result<(String, ConstantValue), SofError> {
    let name = c
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            SofError::InvalidViewDefinition("ViewDefinition.constant.name is required".to_string())
        })?
        .to_string();
    let value = read_constant_value(c).ok_or_else(|| {
        SofError::InvalidViewDefinition(format!(
            "ViewDefinition.constant '{name}' must have exactly one supported value[X] field"
        ))
    })?;
    Ok((name, value))
}

fn read_constant_value(c: &Value) -> Option<ConstantValue> {
    if let Some(s) = c.get("valueString").and_then(|v| v.as_str()) {
        return Some(ConstantValue::String(s.to_string()));
    }
    if let Some(b) = c.get("valueBoolean").and_then(|v| v.as_bool()) {
        return Some(ConstantValue::Boolean(b));
    }
    if let Some(n) = c.get("valueInteger").and_then(|v| v.as_i64()) {
        return Some(ConstantValue::Integer(n));
    }
    if let Some(n) = c.get("valueInteger64").and_then(|v| v.as_i64()) {
        return Some(ConstantValue::Integer64(n));
    }
    if let Some(n) = c.get("valuePositiveInt").and_then(|v| v.as_i64()) {
        return Some(ConstantValue::PositiveInt(n));
    }
    if let Some(n) = c.get("valueUnsignedInt").and_then(|v| v.as_i64()) {
        return Some(ConstantValue::UnsignedInt(n));
    }
    if let Some(n) = c.get("valueDecimal") {
        // Preserve precision by going through the JSON string form.
        return Some(ConstantValue::Decimal(n.to_string()));
    }
    if let Some(s) = c.get("valueCode").and_then(|v| v.as_str()) {
        return Some(ConstantValue::Code(s.to_string()));
    }
    if let Some(s) = c.get("valueBase64Binary").and_then(|v| v.as_str()) {
        return Some(ConstantValue::Base64Binary(s.to_string()));
    }
    if let Some(s) = c.get("valueMarkdown").and_then(|v| v.as_str()) {
        return Some(ConstantValue::Markdown(s.to_string()));
    }
    for key in [
        "valueId",
        "valueUri",
        "valueUrl",
        "valueOid",
        "valueUuid",
        "valueCanonical",
    ] {
        if let Some(s) = c.get(key).and_then(|v| v.as_str()) {
            return Some(ConstantValue::Identifier(s.to_string()));
        }
    }
    if let Some(s) = c.get("valueDate").and_then(|v| v.as_str()) {
        return Some(ConstantValue::Date(s.to_string()));
    }
    if let Some(s) = c.get("valueDateTime").and_then(|v| v.as_str()) {
        return Some(ConstantValue::DateTime(s.to_string()));
    }
    if let Some(s) = c.get("valueTime").and_then(|v| v.as_str()) {
        return Some(ConstantValue::Time(s.to_string()));
    }
    if let Some(s) = c.get("valueInstant").and_then(|v| v.as_str()) {
        return Some(ConstantValue::Instant(s.to_string()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: serde_json::Value) -> ConstantValue {
        parse_constant_from_json(&v).expect("parse").1
    }

    #[test]
    fn each_value_field_lowers_to_matching_variant() {
        let cases: &[(serde_json::Value, ConstantValue)] = &[
            (
                json!({"name": "x", "valueString": "hello"}),
                ConstantValue::String("hello".to_string()),
            ),
            (
                json!({"name": "x", "valueBoolean": true}),
                ConstantValue::Boolean(true),
            ),
            (
                json!({"name": "x", "valueInteger": 7}),
                ConstantValue::Integer(7),
            ),
            (
                json!({"name": "x", "valueInteger64": 9_000_000_000i64}),
                ConstantValue::Integer64(9_000_000_000),
            ),
            (
                json!({"name": "x", "valuePositiveInt": 2}),
                ConstantValue::PositiveInt(2),
            ),
            (
                json!({"name": "x", "valueUnsignedInt": 0}),
                ConstantValue::UnsignedInt(0),
            ),
            (
                json!({"name": "x", "valueDecimal": 1.25}),
                ConstantValue::Decimal("1.25".to_string()),
            ),
            (
                json!({"name": "x", "valueCode": "active"}),
                ConstantValue::Code("active".to_string()),
            ),
            (
                json!({"name": "x", "valueBase64Binary": "QUJD"}),
                ConstantValue::Base64Binary("QUJD".to_string()),
            ),
            (
                json!({"name": "x", "valueMarkdown": "# h"}),
                ConstantValue::Markdown("# h".to_string()),
            ),
            (
                json!({"name": "x", "valueId": "abc-123"}),
                ConstantValue::Identifier("abc-123".to_string()),
            ),
            (
                json!({"name": "x", "valueUri": "http://example.org/"}),
                ConstantValue::Identifier("http://example.org/".to_string()),
            ),
            (
                json!({"name": "x", "valueUrl": "http://example.org/r"}),
                ConstantValue::Identifier("http://example.org/r".to_string()),
            ),
            (
                json!({"name": "x", "valueOid": "urn:oid:1.2.3"}),
                ConstantValue::Identifier("urn:oid:1.2.3".to_string()),
            ),
            (
                json!({"name": "x", "valueUuid": "urn:uuid:00000000-0000-0000-0000-000000000000"}),
                ConstantValue::Identifier(
                    "urn:uuid:00000000-0000-0000-0000-000000000000".to_string(),
                ),
            ),
            (
                json!({"name": "x", "valueCanonical": "http://x|1"}),
                ConstantValue::Identifier("http://x|1".to_string()),
            ),
            (
                json!({"name": "x", "valueDate": "2024-01-02"}),
                ConstantValue::Date("2024-01-02".to_string()),
            ),
            (
                json!({"name": "x", "valueDateTime": "2024-01-02T03:04:05Z"}),
                ConstantValue::DateTime("2024-01-02T03:04:05Z".to_string()),
            ),
            (
                json!({"name": "x", "valueTime": "03:04:05"}),
                ConstantValue::Time("03:04:05".to_string()),
            ),
            (
                json!({"name": "x", "valueInstant": "2024-01-02T03:04:05Z"}),
                ConstantValue::Instant("2024-01-02T03:04:05Z".to_string()),
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(&parse(input.clone()), expected, "input={input}");
        }
    }

    #[test]
    fn missing_name_errors() {
        let err = parse_constant_from_json(&json!({"valueString": "x"})).unwrap_err();
        assert!(matches!(err, SofError::InvalidViewDefinition(_)));
    }

    #[test]
    fn unknown_value_field_errors() {
        let err = parse_constant_from_json(&json!({"name": "x", "valueWhatever": 1})).unwrap_err();
        assert!(matches!(err, SofError::InvalidViewDefinition(_)));
    }

    #[test]
    fn datetime_prefixing_idempotent() {
        let cv = ConstantValue::DateTime("2024-01-02T03:04:05Z".to_string());
        match cv.to_evaluation_result().unwrap() {
            EvaluationResult::DateTime(s, _, _) => assert_eq!(s, "@2024-01-02T03:04:05Z"),
            other => panic!("unexpected: {other:?}"),
        }
        let cv = ConstantValue::DateTime("@2024-01-02T03:04:05Z".to_string());
        match cv.to_evaluation_result().unwrap() {
            EvaluationResult::DateTime(s, _, _) => assert_eq!(s, "@2024-01-02T03:04:05Z"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn time_prefixing_idempotent() {
        let cv = ConstantValue::Time("03:04:05".to_string());
        match cv.to_evaluation_result().unwrap() {
            EvaluationResult::Time(s, _, _) => assert_eq!(s, "@T03:04:05"),
            other => panic!("unexpected: {other:?}"),
        }
        let cv = ConstantValue::Time("@T03:04:05".to_string());
        match cv.to_evaluation_result().unwrap() {
            EvaluationResult::Time(s, _, _) => assert_eq!(s, "@T03:04:05"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn bad_decimal_errors() {
        let cv = ConstantValue::Decimal("not-a-number".to_string());
        assert!(matches!(
            cv.to_evaluation_result(),
            Err(SofError::InvalidViewDefinition(_))
        ));
    }
}
