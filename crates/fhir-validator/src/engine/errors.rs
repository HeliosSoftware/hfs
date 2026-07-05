//! The validation error model and — critically — every message template.
//!
//! The upstream FHIR Schema conformance suite compares error lists with exact
//! ordered deep-equality, message strings included. All message text lives in
//! this one file so the contract is pinned in a single place; the unit tests
//! at the bottom hold the literal strings copied from the fixtures and the
//! reference validator (`fhirschema-js/src/index.js`).

use serde::Serialize;
use serde_json::Value;

/// Stable error type strings — the cross-implementation contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorKind {
    /// A data key matched no element in any schema of the cooperative set.
    UnknownElement,
    /// A singular value was given where some layer declares `array: true`.
    NotArray,
    /// An array was given where every layer says singular.
    NotSingular,
    /// Wrong JSON container type (e.g. `required` applied to a non-object).
    Type,
    /// A key listed in `required` is absent.
    Required,
    /// A key listed in `excluded` is present.
    Excluded,
    /// Array shorter than `min`.
    Min,
    /// Array longer than `max`.
    Max,
    /// A slice's matched-item count violates its `min`/`max`.
    SliceCardinality,
    /// More than one branch of a choice group present at once.
    Choice,
    /// A present choice branch is not in an allowed `choices` list.
    ChoiceExcluded,
    /// Value not deeply equal to `fixed`.
    FixedValue,
    /// Value does not partially match `pattern`.
    PatternValue,
    /// A `severity: error` FHIRPath constraint evaluated to false.
    FhirpathConstraint,
    /// A coded value failed its required-strength ValueSet binding.
    TerminologyBinding,
    // ------------------------------------------------------------------
    // Helios extensions — not pinned by (and never triggered in) the
    // upstream conformance fixtures.
    // ------------------------------------------------------------------
    /// A primitive value failed its type-class or regex check.
    PrimitiveValue,
    /// An array item matched no slice under `rules: closed`, or an unmatched
    /// item preceded matched items under `rules: openAtEnd`.
    SliceUnmatched,
    /// Matched items violate the declared slice order (`ordered: true`).
    SliceOrder,
    /// A referenced schema (root, `base`, or complex `type`) could not be
    /// resolved. The reference validator throws here; we report and continue.
    UnknownSchema,
    /// A profile named in `meta.profile` or the caller's profile list could
    /// not be resolved.
    UnknownProfile,
}

/// Issue severity. Internal only — deliberately not serialized, so the
/// conformance error shape stays exactly `{type, path, message, ...extra}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Severity {
    #[default]
    Error,
    Warning,
}

/// One validation issue, shaped for the conformance contract.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValidationError {
    #[serde(rename = "type")]
    pub kind: ErrorKind,
    /// Dot-joined location rooted at the resource type, numeric array
    /// indices as bare numbers: `Bundle.entry.0.resource.extra`.
    pub path: String,
    pub message: String,
    /// Structured extras carried by some kinds (`constraint` id,
    /// `binding` object).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
    /// Internal severity; REST maps it onto OperationOutcome.
    #[serde(skip)]
    pub severity: Severity,
}

impl ValidationError {
    pub fn new(kind: ErrorKind, path: String, message: String) -> Self {
        Self { kind, path, message, extra: serde_json::Map::new(), severity: Severity::Error }
    }

    pub fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_extra(mut self, key: &str, value: Value) -> Self {
        self.extra.insert(key.to_string(), value);
        self
    }
}

// ---------------------------------------------------------------------
// Message templates — the single source of truth.
//
// Templates marked [pinned] are exercised verbatim by the upstream fixtures
// and must not change. Templates marked [reference] reproduce the reference
// validator's wording for keywords no fixture exercises yet. Templates
// marked [helios] are our own.
// ---------------------------------------------------------------------

/// [pinned] `{key} is unknown`
pub(crate) fn msg_unknown_element(key: &str) -> String {
    format!("{key} is unknown")
}

/// [pinned] `{key} is not array`
pub(crate) fn msg_not_array(key: &str) -> String {
    format!("{key} is not array")
}

/// [pinned] `{key} is not singular`
pub(crate) fn msg_not_singular(key: &str) -> String {
    format!("{key} is not singular")
}

/// [pinned] `{key} is required`
pub(crate) fn msg_required(key: &str) -> String {
    format!("{key} is required")
}

/// [reference] `excluded property {key} is present`
pub(crate) fn msg_excluded(key: &str) -> String {
    format!("excluded property {key} is present")
}

/// [reference] `expected object got {json_type}` — the `required`/`excluded`
/// wording (no comma).
pub(crate) fn msg_expected_object_no_comma(json_type: &str) -> String {
    format!("expected object got {json_type}")
}

/// [reference] `expected object, got {json_type}` — the `elements` wording
/// (with comma). Yes, the two differ only by a comma; both are contract.
pub(crate) fn msg_expected_object_comma(json_type: &str) -> String {
    format!("expected object, got {json_type}")
}

/// [reference] `expected {min} > {actual_len}`
pub(crate) fn msg_min(min: u64, actual_len: usize) -> String {
    format!("expected {min} > {actual_len}")
}

/// [reference] `expected {max} < {actual_len}`
pub(crate) fn msg_max(max: u64, actual_len: usize) -> String {
    format!("expected {max} < {actual_len}")
}

/// [pinned] `only one choice for {group} allowed, but multiple found: {a, b}`
pub(crate) fn msg_choice(group: &str, branches: &[String]) -> String {
    format!(
        "only one choice for {group} allowed, but multiple found: {}",
        branches.join(", ")
    )
}

/// [pinned] `{key} is excluded choice`
pub(crate) fn msg_choice_excluded(key: &str) -> String {
    format!("{key} is excluded choice")
}

/// [pinned] `Slice defines the following min cardinality: '{min}', actual cardinality: '{n}'`
pub(crate) fn msg_slice_min(min: u64, actual: u64) -> String {
    format!("Slice defines the following min cardinality: '{min}', actual cardinality: '{actual}'")
}

/// [pinned] `Slice defines the following max cardinality: '{max}', actual cardinality: '{n}'`
pub(crate) fn msg_slice_max(max: u64, actual: u64) -> String {
    format!("Slice defines the following max cardinality: '{max}', actual cardinality: '{actual}'")
}

/// Render a value for message text: bare strings (like the reference
/// validator's template interpolation), compact JSON for everything else.
pub(crate) fn render_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// [helios] fixed-value mismatch. Matches the reference wording for scalars;
/// composites render as compact JSON (the reference coerces JS objects to
/// `[object Object]`, which no fixture pins).
pub(crate) fn msg_fixed_value(expected: &Value, actual: &Value) -> String {
    format!(
        "Expected value to be exactly equal to 'fixed' pattern '{}', but got: '{}'",
        render_value(expected),
        render_value(actual)
    )
}

/// [helios] pattern-value mismatch (same rendering rationale as fixed).
pub(crate) fn msg_pattern_value(expected: &Value, actual: &Value) -> String {
    format!(
        "Expected value to match 'pattern' '{}', but got: '{}'",
        render_value(expected),
        render_value(actual)
    )
}

/// [helios] `rules: closed` violation, at the unmatched item's path.
pub(crate) fn msg_slice_closed_unmatched() -> String {
    "item does not match any slice in closed slicing".to_string()
}

/// [helios] `rules: openAtEnd` violation, at the unmatched item's path.
pub(crate) fn msg_slice_open_at_end() -> String {
    "unmatched item must be at the end of the array (openAtEnd slicing)".to_string()
}

/// [helios] `ordered: true` violation, at the out-of-order item's path.
pub(crate) fn msg_slice_order(slice: &str) -> String {
    format!("slice '{slice}' is out of order (ordered slicing)")
}

/// [helios] a schema reference could not be resolved (reference validator
/// throws `could not resolve ["…"]`; we report and continue).
pub(crate) fn msg_unknown_schema(reference: &str) -> String {
    format!("could not resolve schema '{reference}'")
}

/// [helios] a profile reference could not be resolved.
pub(crate) fn msg_unknown_profile(reference: &str) -> String {
    format!("could not resolve profile '{reference}'")
}

/// The reference validator's JSON type vocabulary (`getType`).
pub(crate) fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Bool(_) => "boolean",
        Value::Null => "null",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Literal strings copied from the upstream fixtures — the [pinned] set.
    #[test]
    fn pinned_messages_match_fixture_literals() {
        assert_eq!(msg_unknown_element("unknown"), "unknown is unknown");
        assert_eq!(msg_not_array("multielement"), "multielement is not array");
        assert_eq!(msg_not_singular("status"), "status is not singular");
        assert_eq!(msg_required("anotherRequiredElement"), "anotherRequiredElement is required");
        assert_eq!(
            msg_choice("choice", &["choiceString".into(), "choiceInteger".into()]),
            "only one choice for choice allowed, but multiple found: choiceString, choiceInteger"
        );
        assert_eq!(msg_choice_excluded("choiceInteger"), "choiceInteger is excluded choice");
        assert_eq!(
            msg_slice_min(1, 0),
            "Slice defines the following min cardinality: '1', actual cardinality: '0'"
        );
        assert_eq!(
            msg_slice_max(1, 2),
            "Slice defines the following max cardinality: '1', actual cardinality: '2'"
        );
    }

    /// Literal strings copied from fhirschema-js/src/index.js — the
    /// [reference] set (no fixture exercises them yet).
    #[test]
    fn reference_messages_match_source_literals() {
        assert_eq!(msg_expected_object_no_comma("string"), "expected object got string");
        assert_eq!(msg_expected_object_comma("string"), "expected object, got string");
        assert_eq!(msg_excluded("gender"), "excluded property gender is present");
        assert_eq!(msg_min(1, 0), "expected 1 > 0");
        assert_eq!(msg_max(2, 3), "expected 2 < 3");
    }

    /// Helios-owned wording, pinned here and by the extended fixtures.
    #[test]
    fn helios_messages_match_extended_fixture_literals() {
        assert_eq!(
            msg_fixed_value(&json!("male"), &json!("female")),
            "Expected value to be exactly equal to 'fixed' pattern 'male', but got: 'female'"
        );
        assert_eq!(
            msg_pattern_value(&json!({"system": "http://x"}), &json!({"text": "M"})),
            "Expected value to match 'pattern' '{\"system\":\"http://x\"}', but got: '{\"text\":\"M\"}'"
        );
        assert_eq!(
            msg_slice_closed_unmatched(),
            "item does not match any slice in closed slicing"
        );
        assert_eq!(
            msg_slice_open_at_end(),
            "unmatched item must be at the end of the array (openAtEnd slicing)"
        );
        assert_eq!(msg_slice_order("home"), "slice 'home' is out of order (ordered slicing)");
        assert_eq!(
            serde_json::to_value(ErrorKind::SliceUnmatched).unwrap(),
            json!("slice-unmatched")
        );
        assert_eq!(serde_json::to_value(ErrorKind::SliceOrder).unwrap(), json!("slice-order"));
    }

    #[test]
    fn serializes_to_conformance_shape() {
        let err = ValidationError::new(
            ErrorKind::UnknownElement,
            "Patient.unknown".into(),
            msg_unknown_element("unknown"),
        );
        assert_eq!(
            serde_json::to_value(&err).unwrap(),
            json!({
                "type": "unknown-element",
                "path": "Patient.unknown",
                "message": "unknown is unknown"
            })
        );
    }

    #[test]
    fn kebab_case_kind_strings() {
        for (kind, s) in [
            (ErrorKind::UnknownElement, "unknown-element"),
            (ErrorKind::NotArray, "not-array"),
            (ErrorKind::NotSingular, "not-singular"),
            (ErrorKind::Type, "type"),
            (ErrorKind::SliceCardinality, "slice-cardinality"),
            (ErrorKind::ChoiceExcluded, "choice-excluded"),
            (ErrorKind::FixedValue, "fixed-value"),
            (ErrorKind::PatternValue, "pattern-value"),
            (ErrorKind::FhirpathConstraint, "fhirpath-constraint"),
            (ErrorKind::TerminologyBinding, "terminology-binding"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(s));
        }
    }

    #[test]
    fn extra_fields_flatten() {
        let err = ValidationError::new(
            ErrorKind::FhirpathConstraint,
            "Patient".into(),
            "FHIRPath constraint pat-1 error: must have a name".into(),
        )
        .with_extra("constraint", json!("pat-1"));
        let v = serde_json::to_value(&err).unwrap();
        assert_eq!(v["constraint"], json!("pat-1"));
        assert!(v.get("severity").is_none(), "severity must not serialize");
    }
}
