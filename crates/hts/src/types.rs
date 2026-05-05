//! Domain types shared across HTS operations and storage backends.
//!
//! These structs model the request/response contracts for all FHIR terminology
//! operations: `$lookup`, `$validate-code`, `$subsumes`, `$expand`,
//! `$translate`, and `$closure`.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

// ─── Shared helpers ────────────────────────────────────────────────────────────

/// A property value attached to a concept (arbitrary FHIR property).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropertyValue {
    /// Property code (e.g. "parent", "inactive", "synonym")
    pub code: String,
    /// FHIR property type: code | string | boolean | integer | decimal | dateTime
    pub value_type: String,
    /// Serialised value (always a string for simplicity; callers cast as needed)
    pub value: String,
    /// Human-readable description of the property (optional)
    pub description: Option<String>,
}

/// An alternate name or translation for a concept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesignationValue {
    pub language: Option<String>,
    pub use_system: Option<String>,
    pub use_code: Option<String>,
    pub value: String,
}

// ─── $lookup ──────────────────────────────────────────────────────────────────

/// Request for `CodeSystem/$lookup`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LookupRequest {
    /// CodeSystem canonical URL.
    pub system: String,
    /// Concept code to look up.
    pub code: String,
    /// Optional version of the code system.
    pub version: Option<String>,
    /// Preferred language for display.
    pub display_language: Option<String>,
    /// SNOMED post-coordination expression — returns `NotSupported` in SQLite MVP.
    pub expression: Option<String>,
    /// Which properties to include in the response (empty = all).
    pub properties: Vec<String>,
    /// Point-in-time date for evaluation (ISO-8601). Only resources whose
    /// `$.date` field is ≤ this value are considered.
    #[serde(default)]
    pub date: Option<String>,
}

/// Response from `CodeSystem/$lookup`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LookupResponse {
    /// The canonical name of the code system.
    pub name: String,
    pub version: Option<String>,
    pub display: Option<String>,
    pub properties: Vec<PropertyValue>,
    pub designations: Vec<DesignationValue>,
}

// ─── $validate-code ───────────────────────────────────────────────────────────

/// Request for `CodeSystem/$validate-code` and `ValueSet/$validate-code`.
///
/// Either `url` (for a ValueSet) or `system` (for a CodeSystem) should be set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ValidateCodeRequest {
    /// ValueSet URL (used when validating against a value set).
    pub url: Option<String>,
    /// CodeSystem URL (used when validating directly against a code system).
    pub system: Option<String>,
    /// The code to validate.
    pub code: String,
    /// Optional version.
    pub version: Option<String>,
    /// Expected display; if provided the response includes whether it matches.
    pub display: Option<String>,
    /// FHIR `abstract` parameter — when explicitly false, abstract concepts
    /// (those with `notSelectable=true`) are rejected with a "code is
    /// abstract, and not allowed in this context" message. None / true mean
    /// abstract concepts pass when the VS otherwise contains them.
    #[serde(default)]
    pub include_abstract: Option<bool>,
    /// Point-in-time date for evaluation (ISO-8601).
    #[serde(default)]
    pub date: Option<String>,
}

/// Response from `$validate-code`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidateCodeResponse {
    /// `true` if the code is valid.
    pub result: bool,
    /// Explanation when `result` is `false`, or a confirmation message.
    pub message: Option<String>,
    /// The preferred display for the code (present on success).
    pub display: Option<String>,
    /// `Some(true)` when the matched concept is inactive (status in
    /// retired/deprecated/withdrawn/inactive). The IG fixtures expect this
    /// to surface as a top-level `inactive` parameter on the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inactive: Option<bool>,
}

// ─── $subsumes ────────────────────────────────────────────────────────────────

/// The four possible outcomes of `CodeSystem/$subsumes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubsumptionOutcome {
    /// code_a and code_b are the same concept.
    Equivalent,
    /// code_a subsumes code_b (code_a is an ancestor of code_b).
    Subsumes,
    /// code_a is subsumed by code_b (code_a is a descendant of code_b).
    SubsumedBy,
    /// Neither code subsumes the other.
    NotSubsumed,
}

impl SubsumptionOutcome {
    /// Returns the FHIR-specified string value for this outcome.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Equivalent => "equivalent",
            Self::Subsumes => "subsumes",
            Self::SubsumedBy => "subsumed-by",
            Self::NotSubsumed => "not-subsumed",
        }
    }
}

/// Request for `CodeSystem/$subsumes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubsumesRequest {
    /// CodeSystem canonical URL.
    pub system: String,
    /// Optional version.
    pub version: Option<String>,
    /// First code.
    pub code_a: String,
    /// Second code.
    pub code_b: String,
}

/// Response from `CodeSystem/$subsumes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubsumesResponse {
    pub outcome: SubsumptionOutcome,
}

// ─── $expand ──────────────────────────────────────────────────────────────────

/// A single concept in a ValueSet expansion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpansionContains {
    /// Code system URL.
    pub system: String,
    pub code: String,
    pub display: Option<String>,
    /// FHIR `abstract` flag — mirrors the concept's `notSelectable` property.
    /// Populated by the operations layer post-expansion via a batch lookup.
    #[serde(default, rename = "abstract")]
    pub is_abstract: Option<bool>,
    pub inactive: Option<bool>,
    /// Designations attached to this concept (translations, alternate
    /// labels). Populated post-expansion when the caller asked for
    /// `includeDesignations=true`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub designations: Vec<ExpansionContainsDesignation>,
    /// Properties attached to this concept (FHIR concept properties).
    /// Populated post-expansion when the caller passed a `property`
    /// parameter naming one or more property codes to surface.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<ExpansionContainsProperty>,
    /// Nested contains for hierarchical expansions.
    #[serde(default)]
    pub contains: Vec<ExpansionContains>,
}

/// One property entry on an `ExpansionContains` — mirrors the FHIR
/// `expansion.contains[].property[]` shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpansionContainsProperty {
    pub code: String,
    /// FHIR `value[x]` type label (e.g. "Code", "String", "Boolean").
    pub value_type: String,
    /// Serialised value (always a string; the serializer routes it to the
    /// correct FHIR `value[x]` field based on `value_type`).
    pub value: String,
}

/// One designation entry on an `ExpansionContains`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpansionContainsDesignation {
    pub language: Option<String>,
    /// `{system, code}` of the designation use; both optional.
    pub use_system: Option<String>,
    pub use_code: Option<String>,
    pub value: String,
}

/// Request for `ValueSet/$expand`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ExpandRequest {
    /// ValueSet canonical URL.
    pub url: Option<String>,
    /// Inline ValueSet resource (used when no `url` is provided).
    pub value_set: Option<serde_json::Value>,
    /// Free-text filter applied to code + display.
    pub filter: Option<String>,
    /// Maximum number of codes to return.
    pub count: Option<u32>,
    /// Zero-based offset for pagination.
    pub offset: Option<u32>,
    /// Server-side ceiling: if the full (unfiltered) expansion exceeds this
    /// value the backend returns `HtsError::TooCostly`. `None` means no limit.
    #[serde(skip)]
    pub max_expansion_size: Option<u32>,
    /// Point-in-time date for evaluation (ISO-8601).
    #[serde(default)]
    pub date: Option<String>,
    /// When `true`, return a tree-structured expansion using the CodeSystem
    /// hierarchy instead of a flat list. Pagination is not applied in tree mode.
    #[serde(default)]
    pub hierarchical: Option<bool>,
}

/// Response from `ValueSet/$expand`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpandResponse {
    pub total: Option<u32>,
    pub offset: Option<u32>,
    pub contains: Vec<ExpansionContains>,
    /// FHIR `expansion.parameter[].name = "warning"` messages emitted when
    /// one or more systems in an inline compose were not loaded and were
    /// silently excluded from the expansion.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

// ─── $translate ───────────────────────────────────────────────────────────────

/// A single translation match returned by `ConceptMap/$translate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslationMatch {
    /// FHIR equivalence code (e.g. "equivalent", "wider", "narrower").
    pub equivalence: String,
    pub concept_system: String,
    pub concept_code: String,
    pub concept_display: Option<String>,
    /// Reference to the source of the mapping (ConceptMap URL).
    pub source: Option<String>,
    /// Optional ConceptMap version, used to build the `originMap` canonical
    /// reference (`url|version`) in the response.
    #[serde(default)]
    pub map_version: Option<String>,
    /// The source-side Coding of this mapping. Populated for reverse
    /// translations so the response can include a `source` part identifying
    /// the original code that was reverse-mapped from.
    #[serde(default)]
    pub source_system: Option<String>,
    #[serde(default)]
    pub source_code: Option<String>,
}

/// Request for `ConceptMap/$translate`.
///
/// Supports both R4 parameter names (`code`, `system`) and R5 names
/// (`sourceCode`, `sourceSystem`, `targetCode`, `targetSystem`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TranslateRequest {
    /// ConceptMap canonical URL (optional; if absent, all maps are searched).
    pub url: Option<String>,
    /// Source code system URL (R4 `system` / R5 `sourceSystem`).
    pub system: Option<String>,
    /// Source code to translate (R4 `code` / R5 `sourceCode`).
    /// Empty string when reverse mode is driven by `target_code` instead.
    pub code: String,
    /// Source value set URL.
    pub source: Option<String>,
    /// Target value set URL.
    pub target: Option<String>,
    /// Target code system URL.
    pub target_system: Option<String>,
    /// Target code (R5 `targetCode`) — used to drive reverse translations
    /// without an explicit `reverse=true` flag.
    #[serde(default)]
    pub target_code: Option<String>,
    /// If `true`, reverse the mapping direction (look up target → source).
    #[serde(default)]
    pub reverse: bool,
    /// Point-in-time date for evaluation (ISO-8601).
    #[serde(default)]
    pub date: Option<String>,
}

/// Response from `ConceptMap/$translate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslateResponse {
    /// `true` if at least one match was found.
    pub result: bool,
    pub message: Option<String>,
    pub matches: Vec<TranslationMatch>,
}

// ─── Search ───────────────────────────────────────────────────────────────────

/// Query parameters for searching CodeSystem, ValueSet, or ConceptMap resources.
///
/// All fields are optional — omitting a field means "no filter on that field".
/// Multiple fields are ANDed together.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ResourceSearchQuery {
    /// Filter by canonical URL (exact match).
    pub url: Option<String>,
    /// Filter by version string (exact match).
    pub version: Option<String>,
    /// Filter by computer-friendly name (exact match).
    pub name: Option<String>,
    /// Filter by human-friendly title (exact match).
    pub title: Option<String>,
    /// Filter by status: `draft`, `active`, `retired`, or `unknown`.
    pub status: Option<String>,
    /// Maximum number of results to return (default: 20).
    #[serde(rename = "_count")]
    pub count: Option<u32>,
    /// Zero-based offset for pagination (default: 0).
    #[serde(rename = "_offset")]
    pub offset: Option<u32>,
    /// When `"true"`, return a summary representation without large data arrays.
    /// Avoids reading the `resource_json` blob; returns a synthetic summary instead.
    #[serde(rename = "_summary")]
    pub summary: Option<String>,
}

// ─── $closure ─────────────────────────────────────────────────────────────────

/// A coded concept supplied to `ConceptMap/$closure`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodingConcept {
    pub system: String,
    pub code: String,
    pub display: Option<String>,
}

/// Request for `ConceptMap/$closure`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClosureRequest {
    /// Name of the closure table to maintain.
    pub name: String,
    /// Concepts to add to the closure table.
    #[serde(default)]
    pub concept: Vec<CodingConcept>,
    pub version: Option<String>,
}

/// Response from `ConceptMap/$closure`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClosureResponse {
    /// Name of the closure table.
    pub name: String,
    pub version: Option<String>,
    /// A ConceptMap resource (as JSON) representing the computed closure.
    pub concept_map: Option<serde_json::Value>,
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsumption_outcome_as_str() {
        assert_eq!(SubsumptionOutcome::Equivalent.as_str(), "equivalent");
        assert_eq!(SubsumptionOutcome::Subsumes.as_str(), "subsumes");
        assert_eq!(SubsumptionOutcome::SubsumedBy.as_str(), "subsumed-by");
        assert_eq!(SubsumptionOutcome::NotSubsumed.as_str(), "not-subsumed");
    }

    #[test]
    fn subsumption_outcome_roundtrip() {
        let outcomes = [
            SubsumptionOutcome::Equivalent,
            SubsumptionOutcome::Subsumes,
            SubsumptionOutcome::SubsumedBy,
            SubsumptionOutcome::NotSubsumed,
        ];
        for outcome in &outcomes {
            let json = serde_json::to_string(outcome).unwrap();
            let decoded: SubsumptionOutcome = serde_json::from_str(&json).unwrap();
            assert_eq!(outcome, &decoded);
        }
    }

    #[test]
    fn lookup_request_roundtrip() {
        let req = LookupRequest {
            system: "http://example.org/cs".into(),
            code: "ABC".into(),
            version: Some("1.0".into()),
            display_language: None,
            expression: None,
            properties: vec!["display".into()],
            ..Default::default()
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: LookupRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn validate_code_request_roundtrip() {
        let req = ValidateCodeRequest {
            url: None,
            system: Some("http://example.org/cs".into()),
            code: "ABC".into(),
            version: None,
            display: Some("Alpha Beta Charlie".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: ValidateCodeRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn expand_request_roundtrip() {
        let req = ExpandRequest {
            url: Some("http://example.org/vs".into()),
            count: Some(100),
            offset: Some(0),
            ..Default::default()
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: ExpandRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn translate_request_default_reverse() {
        // `reverse` defaults to false when absent from JSON
        let json = r#"{"code":"A","system":"http://cs.org"}"#;
        let req: TranslateRequest = serde_json::from_str(json).unwrap();
        assert!(!req.reverse);
    }

    #[test]
    fn closure_request_default_concepts() {
        // `concept` defaults to empty vec when absent from JSON
        let json = r#"{"name":"my-closure"}"#;
        let req: ClosureRequest = serde_json::from_str(json).unwrap();
        assert!(req.concept.is_empty());
    }

    #[test]
    fn expansion_contains_nested() {
        let child = ExpansionContains {
            system: "http://example.org/cs".into(),
            code: "CHILD".into(),
            display: Some("Child Concept".into()),
            is_abstract: None,
            inactive: None,
            designations: vec![],
            properties: vec![],
            contains: vec![],
        };
        let parent = ExpansionContains {
            system: "http://example.org/cs".into(),
            code: "PARENT".into(),
            display: Some("Parent Concept".into()),
            is_abstract: None,
            inactive: None,
            designations: vec![],
            properties: vec![],
            contains: vec![child.clone()],
        };
        let json = serde_json::to_string(&parent).unwrap();
        let decoded: ExpansionContains = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.contains.len(), 1);
        assert_eq!(decoded.contains[0].code, "CHILD");
    }
}
