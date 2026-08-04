//! The FHIR Schema data model.
//!
//! FHIR Schema is a JSON-Schema-like, differential-by-design representation of
//! FHIR StructureDefinitions (see <https://fhir-schema.github.io/fhir-schema/>).
//! One struct serves both the schema level and the element level — the two are
//! the same shape in the format (confirmed by the upstream conformance suite).
//!
//! Everything is optional: a schema on disk is a differential fragment, and
//! the "complete picture" only exists transiently as a cooperative schema set
//! during validation (see [`crate::engine`]).
//!
//! Deserialization is tolerant: unknown keys are ignored (mirroring the
//! reference validator, which logs and skips unrecognized keywords).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// Schema `kind` values with meaning to the validation engine.
///
/// Kept as plain string constants (rather than an enum) because fixtures and
/// converted packs carry open-ended kinds (`resource`, `complex-type`,
/// `logical`, `extension`, ...) and only `primitive-type` alters behavior:
/// it stops complex-`type` expansion during schema-set assembly.
pub mod kind {
    pub const PRIMITIVE_TYPE: &str = "primitive-type";
    pub const RESOURCE: &str = "resource";
    pub const COMPLEX_TYPE: &str = "complex-type";
    pub const EXTENSION: &str = "extension";
}

/// A FHIR Schema — used both as a top-level (named) schema and as an element
/// schema nested under [`FhirSchema::elements`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirSchema {
    // ------------------------------------------------------------------
    // Identity (top-level schemas)
    // ------------------------------------------------------------------
    /// Canonical URL this schema is published under.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Machine-readable name (also usable as a resolution alias).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Schema this one layers onto; resolved into the same cooperative set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Schema kind — see [`kind`]. Only `primitive-type` alters assembly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// `specialization` (custom resources) or `constraint` (profiles).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivation: Option<String>,
    /// The type this schema (or element) constrains. On an element, naming a
    /// complex type pulls that type's schema into the set; the special value
    /// `Resource` resolves dynamically from the data's `resourceType`.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,

    // ------------------------------------------------------------------
    // Shape
    // ------------------------------------------------------------------
    /// Element is a collection: only JSON arrays are accepted. Array-ness is
    /// decided cooperatively — true if *any* layer says `array: true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub array: Option<bool>,
    /// Element is scalar: JSON arrays are rejected. Mutually exclusive with
    /// `array` within one schema (layers may still disagree; array wins).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scalar: Option<bool>,
    /// Minimum number of items (arrays only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<u64>,
    /// Maximum number of items (arrays only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<u64>,

    // ------------------------------------------------------------------
    // Structure
    // ------------------------------------------------------------------
    /// Nested element schemas, keyed by element name. Order is preserved —
    /// deterministic error emission is part of the conformance contract.
    /// Values are `Arc` so cooperative sets can share them without cloning.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elements: Option<IndexMap<String, Arc<FhirSchema>>>,
    /// Child keys that must be present on the node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
    /// Child keys that must be absent from the node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded: Option<Vec<String>>,
    /// Reference to another element's schema by path, for recursive
    /// structures (e.g. `Questionnaire.item`):
    /// `["Questionnaire", "elements", "item"]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_reference: Option<Vec<String>>,

    // ------------------------------------------------------------------
    // Choice types (value[x])
    // ------------------------------------------------------------------
    /// On a choice-group declarer: the allowed concrete branch names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<String>>,
    /// On a concrete branch: the name of the choice group it belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choice_of: Option<String>,

    // ------------------------------------------------------------------
    // Value constraints
    // ------------------------------------------------------------------
    /// Value must be deeply equal to this constant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed: Option<Value>,
    /// Value must partially match this constant (pattern keys present and
    /// equal; extra data keys permitted).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<Value>,
    /// Terminology binding. Only `required`-strength bindings are enforced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<Binding>,
    /// FHIRPath invariants, keyed by constraint id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<IndexMap<String, Constraint>>,
    /// Allowed reference target types (for Reference/canonical elements).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refers: Option<Vec<String>>,

    // ------------------------------------------------------------------
    // Slicing
    // ------------------------------------------------------------------
    /// Array slicing definition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slicing: Option<Slicing>,
    /// Extension sugar: named extensions keyed by slice name, compiled into
    /// `slicing` on the `extension` element (matched by `{url}`) at
    /// validation time. Entry values reuse [`FhirSchema`] (they carry `url`,
    /// `min`, `max`, plus any extra element constraints).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<IndexMap<String, Arc<FhirSchema>>>,

    // ------------------------------------------------------------------
    // Informational (carried, never enforced)
    // ------------------------------------------------------------------
    /// Mirrors `ElementDefinition.isModifier`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<bool>,
    /// Mirrors `ElementDefinition.mustSupport`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub must_support: Option<bool>,
    /// Mirrors `ElementDefinition.isSummary`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<bool>,
    /// For extension definitions: the element expressions the extension
    /// applies to (`StructureDefinition.context[].expression` where the type
    /// is `element`). Powers the editor's profiled-extension offers (#363).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<String>>,
    /// Mirrors `ElementDefinition.short` — the human label consumers render
    /// instead of the raw element name. The longer `definition` text is
    /// deliberately not carried (pack size); `short` is the label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short: Option<String>,

    // ------------------------------------------------------------------
    // Helios extension
    // ------------------------------------------------------------------
    /// Primitive value regex, carried from the FHIR spec's primitive type
    /// StructureDefinitions by our converter. Not part of upstream FHIR
    /// Schema; absent from the upstream conformance fixtures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regex: Option<String>,
}

impl FhirSchema {
    /// Whether this schema is a primitive-type leaf: registered in the set
    /// but never expanded as a complex type.
    pub fn is_primitive(&self) -> bool {
        self.kind.as_deref() == Some(kind::PRIMITIVE_TYPE)
    }
}

/// Terminology binding: the ValueSet a coded value must belong to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    /// Canonical URL of the bound ValueSet.
    pub value_set: String,
    /// FHIR binding strength (`required`, `extensible`, `preferred`,
    /// `example`). Only `required` is validated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<String>,
}

/// A FHIRPath invariant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Constraint {
    /// FHIRPath expression that must evaluate truthy on the node.
    pub expression: String,
    /// `error` | `warning` | `guideline`. Only `error` fails validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    /// Human-readable description, used in the emitted message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human: Option<String>,
}

/// Array slicing: partitions a repeating element into named slices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slicing {
    /// Named slices. Order preserved for deterministic sweep/error order.
    pub slices: IndexMap<String, Slice>,
    /// `open` (default) | `closed` | `openAtEnd`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<String>,
    /// When true, matched items must appear in slice `order`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordered: Option<bool>,
}

/// One slice of a sliced array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Slice {
    /// How items are matched into this slice. Optional: constraining slices
    /// (`sliceIsConstraining`) reference a parent slice instead.
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    pub match_: Option<Match>,
    /// Minimum number of matched items across the whole array.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<u64>,
    /// Maximum number of matched items across the whole array.
    /// `Some(0)` means the slice is prohibited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<u64>,
    /// Position for `ordered` slicing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<u64>,
    /// Name of the parent slice being re-sliced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reslice: Option<String>,
    /// Further constrains a slice of the same name from a parent schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slice_is_constraining: Option<bool>,
    /// Element schema applied cooperatively to items matched into the slice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Arc<FhirSchema>>,
}

/// A slice match rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    /// `pattern` | `binding` | `profile` | `type` (the reference validator
    /// only implements `pattern`; we start there too).
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
    /// The pattern / binding / profile / type payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// Match against the resolved reference target instead of the reference
    /// element itself.
    #[serde(rename = "resolve-ref", skip_serializing_if = "Option::is_none")]
    pub resolve_ref: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_primitive_schema() {
        let s: FhirSchema = serde_json::from_str(r#"{ "kind": "primitive-type" }"#).unwrap();
        assert!(s.is_primitive());
        assert!(s.elements.is_none());
    }

    #[test]
    fn parses_elements_preserving_order() {
        let s: FhirSchema = serde_json::from_str(
            r#"{ "elements": { "z": {"type": "string"}, "a": {"type": "string"} } }"#,
        )
        .unwrap();
        let keys: Vec<&str> = s
            .elements
            .as_ref()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        assert_eq!(
            keys,
            vec!["z", "a"],
            "element declaration order must be preserved"
        );
    }

    #[test]
    fn tolerates_unknown_keys() {
        let s: FhirSchema =
            serde_json::from_str(r#"{ "type": "string", "somethingNew": 42 }"#).unwrap();
        assert_eq!(s.type_.as_deref(), Some("string"));
    }

    #[test]
    fn parses_slicing_shape() {
        let s: FhirSchema = serde_json::from_str(
            r#"{
              "slicing": {
                "slices": {
                  "home": {
                    "match": { "type": "pattern", "value": { "use": "home" } },
                    "min": 1, "max": 1,
                    "schema": { "required": ["city"] }
                  }
                }
              }
            }"#,
        )
        .unwrap();
        let slicing = s.slicing.unwrap();
        let home = &slicing.slices["home"];
        assert_eq!(home.min, Some(1));
        let m = home.match_.as_ref().unwrap();
        assert_eq!(m.type_.as_deref(), Some("pattern"));
        assert_eq!(m.value.as_ref().unwrap()["use"], "home");
        assert_eq!(
            home.schema.as_ref().unwrap().required.as_ref().unwrap(),
            &vec!["city".to_string()]
        );
    }
}
