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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    pub inactive: Option<bool>,
    /// Nested contains for hierarchical expansions.
    #[serde(default)]
    pub contains: Vec<ExpansionContains>,
}

/// Request for `ValueSet/$expand`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
}

/// Response from `ValueSet/$expand`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpandResponse {
    pub total: Option<u32>,
    pub offset: Option<u32>,
    pub contains: Vec<ExpansionContains>,
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
}

/// Request for `ConceptMap/$translate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslateRequest {
    /// ConceptMap canonical URL (optional; if absent, all maps are searched).
    pub url: Option<String>,
    /// Source code system URL.
    pub system: Option<String>,
    /// Source code to translate.
    pub code: String,
    /// Source value set URL.
    pub source: Option<String>,
    /// Target value set URL.
    pub target: Option<String>,
    /// Target code system URL.
    pub target_system: Option<String>,
    /// If `true`, reverse the mapping direction (look up target → source).
    #[serde(default)]
    pub reverse: bool,
}

/// Response from `ConceptMap/$translate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslateResponse {
    /// `true` if at least one match was found.
    pub result: bool,
    pub message: Option<String>,
    pub matches: Vec<TranslationMatch>,
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
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: ValidateCodeRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn expand_request_roundtrip() {
        let req = ExpandRequest {
            url: Some("http://example.org/vs".into()),
            value_set: None,
            filter: None,
            count: Some(100),
            offset: Some(0),
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
            inactive: None,
            contains: vec![],
        };
        let parent = ExpansionContains {
            system: "http://example.org/cs".into(),
            code: "PARENT".into(),
            display: Some("Parent Concept".into()),
            inactive: None,
            contains: vec![child.clone()],
        };
        let json = serde_json::to_string(&parent).unwrap();
        let decoded: ExpansionContains = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.contains.len(), 1);
        assert_eq!(decoded.contains[0].code, "CHILD");
    }
}
