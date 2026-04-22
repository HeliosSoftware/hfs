//! Custom OperationOutcome `issue.details` coding for validation.
//!
//! These codes are owned by Atrius (not HL7 `operation-outcome`) and are intended
//! to back a future `CodeSystem` / `ValueSet` for validation-specific detail.

use std::fmt;

/// Canonical URI for the custom CodeSystem used in `OperationOutcome.issue.details.coding`.
pub const VALIDATION_ISSUE_DETAIL_SYSTEM: &str =
    "https://atrius.health/fhir/CodeSystem/validation-issue-detail";
/// Placeholder until a published CodeSystem exists.
pub const VALIDATION_ISSUE_DETAIL_VERSION: &str = "0.1.0";

/// Detail subtype for [`crate::ValidationIssue`] → OperationOutcome `details.coding`.
///
/// Producers may set [`crate::ValidationIssue::detail_code`] to a specific variant; when
/// absent, [`ValidationIssueDetailCode::from_issue_category`] derives a coarse code from
/// [`crate::ValidationIssue::code`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValidationIssueDetailCode {
    RequiredElementMissing,
    MaximumCardinalityExceeded,
    ElementValueInvalid,
    ConstraintViolation,
    StructureInvalid,
    ReferenceNotFound,
    DuplicateDetected,
    EditConflict,
    InteractionNotSupported,
    TerminologyValidationFailed,
    TerminologyServiceUnavailable,
    RecursionDepthReached,
    ProfileCycleDetected,
    BusinessRuleViolation,
    ValidationException,
    /// Fallback when `issue.code` is unknown.
    ValidationFailure,
    CodeWithoutSystem,
    InvalidBindableValue,
    /// Required binding: code not in the bound ValueSet (typical `required` strength).
    RequiredBindingMiss,
    ExtensibleBindingMiss,
    PreferredBindingMiss,
    ExampleBindingMiss,
    /// Instance value does not match a profile `pattern` constraint.
    PatternConstraintMismatch,
    /// Instance value does not match a profile `fixed` constraint.
    FixedConstraintMismatch,
    /// `ordered` slicing: repeated items match slices out of declaration order.
    SliceOrderViolation,
    SliceMinCardinalityMissing,
    SliceMaxCardinalityExceeded,
    SliceOpenAtEndViolation,
    SlicingUnsupportedDiscriminator,
    SlicingNoDiscriminators,
}

impl ValidationIssueDetailCode {
    /// Map internal validator category (`ValidationIssue.code`) to a coarse detail code.
    pub fn from_issue_category(code: &str) -> Self {
        match code {
            "required" => Self::RequiredElementMissing,
            "value" => Self::ElementValueInvalid,
            "invariant" => Self::ConstraintViolation,
            "structure" => Self::StructureInvalid,
            "not-found" => Self::ReferenceNotFound,
            "duplicate" => Self::DuplicateDetected,
            "conflict" => Self::EditConflict,
            "not-supported" => Self::InteractionNotSupported,
            "terminology" => Self::TerminologyValidationFailed,
            "business-rule" => Self::BusinessRuleViolation,
            "exception" => Self::ValidationException,
            _ => Self::ValidationFailure,
        }
    }

    pub fn as_code(self) -> &'static str {
        match self {
            Self::RequiredElementMissing => "required-element-missing",
            Self::ElementValueInvalid => "element-value-invalid",
            Self::ConstraintViolation => "constraint-violation",
            Self::StructureInvalid => "structure-invalid",
            Self::ReferenceNotFound => "reference-not-found",
            Self::DuplicateDetected => "duplicate-detected",
            Self::EditConflict => "edit-conflict",
            Self::InteractionNotSupported => "interaction-not-supported",
            Self::TerminologyValidationFailed => "terminology-validation-failed",
            Self::BusinessRuleViolation => "business-rule-violation",
            Self::ValidationException => "validation-exception",
            Self::ValidationFailure => "validation-failure",
            Self::RequiredBindingMiss => "required-binding-miss",
            Self::PatternConstraintMismatch => "pattern-constraint-mismatch",
            Self::FixedConstraintMismatch => "fixed-constraint-mismatch",
            Self::SliceOrderViolation => "slice-order-violation",
            Self::MaximumCardinalityExceeded => "maximum-cardinality-exceeded",
            Self::TerminologyServiceUnavailable => "terminology-service-unavailable",
            Self::RecursionDepthReached => "recursion-depth-reached",
            Self::ProfileCycleDetected => "profile-cycle-detected",
            Self::CodeWithoutSystem => "code-without-system",
            Self::InvalidBindableValue => "invalid-bindable-value",
            Self::ExtensibleBindingMiss => "extensible-binding-miss",
            Self::PreferredBindingMiss => "preferred-binding-miss",
            Self::ExampleBindingMiss => "example-binding-miss",
            Self::SliceMinCardinalityMissing => "slice-min-cardinality-missing",
            Self::SliceMaxCardinalityExceeded => "slice-max-cardinality-exceeded",
            Self::SliceOpenAtEndViolation => "slice-open-at-end-violation",
            Self::SlicingUnsupportedDiscriminator => "slicing-unsupported-discriminator",
            Self::SlicingNoDiscriminators => "slicing-no-discriminators",
        }
    }

    /// Short label for `details.coding.display`.
    pub fn display(self) -> &'static str {
        match self {
            Self::RequiredElementMissing => "Required element missing",
            Self::ElementValueInvalid => "Element value invalid",
            Self::ConstraintViolation => "Invariant constraint violation",
            Self::StructureInvalid => "Structural validation failure",
            Self::ReferenceNotFound => "Reference not found",
            Self::DuplicateDetected => "Duplicate content",
            Self::EditConflict => "Content conflict",
            Self::InteractionNotSupported => "Not supported",
            Self::TerminologyValidationFailed => "Terminology validation failure",
            Self::BusinessRuleViolation => "Business rule violation",
            Self::ValidationException => "Validation exception",
            Self::ValidationFailure => "Validation issue",
            Self::RequiredBindingMiss => "Required binding miss",
            Self::PatternConstraintMismatch => "Pattern constraint mismatch",
            Self::FixedConstraintMismatch => "Fixed constraint mismatch",
            Self::SliceOrderViolation => "Slice order violation",
            Self::MaximumCardinalityExceeded => "Maximum cardinality exceeded",
            Self::TerminologyServiceUnavailable => "Terminology service unavailable",
            Self::RecursionDepthReached => "Recursion depth reached",
            Self::ProfileCycleDetected => "Profile cycle detected",
            Self::CodeWithoutSystem => "Code without system",
            Self::InvalidBindableValue => "Invalid bindable value",
            Self::ExtensibleBindingMiss => "Extensible binding miss",
            Self::PreferredBindingMiss => "Preferred binding miss",
            Self::ExampleBindingMiss => "Example binding miss",
            Self::SliceMinCardinalityMissing => "Slice min cardinality missing",
            Self::SliceMaxCardinalityExceeded => "Slice max cardinality exceeded",
            Self::SliceOpenAtEndViolation => "Slice open at end violation",
            Self::SlicingUnsupportedDiscriminator => "Slicing unsupported discriminator",
            Self::SlicingNoDiscriminators => "Slicing no discriminators",
        }
    }

    /// Default `OperationOutcome.issue.details.text` when [`crate::ValidationIssue::summary`] is absent.
    ///
    /// Wording is aligned with historical synthesized summaries (full sentences where helpful).
    pub fn details_text_fallback(self) -> &'static str {
        match self {
            Self::RequiredElementMissing => "Required element is missing",
            Self::ElementValueInvalid => "Element value is invalid for the expected constraint",
            Self::ConstraintViolation => "Value does not satisfy an invariant constraint",
            Self::StructureInvalid => "Resource structure does not match profile requirements",
            Self::ReferenceNotFound => "Referenced element or resource was not found",
            Self::DuplicateDetected => "Duplicate element or resource was detected",
            Self::EditConflict => "Submitted content conflicts with current state",
            Self::InteractionNotSupported => "Feature or interaction is not supported",
            Self::TerminologyValidationFailed => "Terminology validation failed",
            Self::BusinessRuleViolation => "Business rule or profile constraint was not satisfied",
            Self::ValidationException => "Validation encountered an unexpected error",
            Self::ValidationFailure => "Validation failed for this element",
            Self::RequiredBindingMiss => {
                "Does not satisfy the required value set binding (verify system, code, and display)"
            }
            Self::PatternConstraintMismatch => {
                "Element value does not match the profile pattern constraint"
            }
            Self::FixedConstraintMismatch => "Element value does not match the profile fixed value",
            Self::SliceOrderViolation => {
                "Slice instances are not in declared order (ordered slicing)"
            }
            Self::MaximumCardinalityExceeded => {
                "Maximum cardinality exceeded (cardinality slicing)"
            }
            Self::TerminologyServiceUnavailable => "Terminology service is not available",
            Self::RecursionDepthReached => "Recursion depth exceeded (recursion slicing)",
            Self::ProfileCycleDetected => "Profile cycle detected",
            Self::CodeWithoutSystem => "Code is missing a system",
            Self::InvalidBindableValue => "Value is not valid for the data type or binding",
            Self::ExtensibleBindingMiss => {
                "Does not satisfy the extensible value set binding (verify system, code, and display)"
            }
            Self::PreferredBindingMiss => {
                "Does not satisfy the preferred value set binding (verify system, code, and display)"
            }
            Self::ExampleBindingMiss => {
                "Does not satisfy the example value set binding (verify system, code, and display)"
            }
            Self::SliceMinCardinalityMissing => "Slice min cardinality is missing",
            Self::SliceMaxCardinalityExceeded => "Slice max cardinality is exceeded",
            Self::SliceOpenAtEndViolation => "Slice open at end is not allowed",
            Self::SlicingUnsupportedDiscriminator => {
                "Slicing is not supported for this discriminator"
            }
            Self::SlicingNoDiscriminators => "Slicing is not supported for this resource",
        }
    }

    pub fn try_from_detail_code(code: &str) -> Option<Self> {
        Some(match code {
            "required-element-missing" => Self::RequiredElementMissing,
            "element-value-invalid" => Self::ElementValueInvalid,
            "constraint-violation" => Self::ConstraintViolation,
            "structure-invalid" => Self::StructureInvalid,
            "reference-not-found" => Self::ReferenceNotFound,
            "duplicate-detected" => Self::DuplicateDetected,
            "edit-conflict" => Self::EditConflict,
            "interaction-not-supported" => Self::InteractionNotSupported,
            "terminology-validation-failed" => Self::TerminologyValidationFailed,
            "business-rule-violation" => Self::BusinessRuleViolation,
            "validation-exception" => Self::ValidationException,
            "validation-failure" => Self::ValidationFailure,
            "required-binding-miss" => Self::RequiredBindingMiss,
            "pattern-constraint-mismatch" => Self::PatternConstraintMismatch,
            "fixed-constraint-mismatch" => Self::FixedConstraintMismatch,
            "slice-order-violation" => Self::SliceOrderViolation,
            "maximum-cardinality-exceeded" => Self::MaximumCardinalityExceeded,
            "terminology-service-unavailable" => Self::TerminologyServiceUnavailable,
            "recursion-depth-reached" => Self::RecursionDepthReached,
            "profile-cycle-detected" => Self::ProfileCycleDetected,
            "code-without-system" => Self::CodeWithoutSystem,
            "invalid-bindable-value" => Self::InvalidBindableValue,
            "extensible-binding-miss" => Self::ExtensibleBindingMiss,
            "preferred-binding-miss" => Self::PreferredBindingMiss,
            "example-binding-miss" => Self::ExampleBindingMiss,
            "slice-min-cardinality-missing" => Self::SliceMinCardinalityMissing,
            "slice-max-cardinality-exceeded" => Self::SliceMaxCardinalityExceeded,
            "slice-open-at-end-violation" => Self::SliceOpenAtEndViolation,
            "slicing-unsupported-discriminator" => Self::SlicingUnsupportedDiscriminator,
            "slicing-no-discriminators" => Self::SlicingNoDiscriminators,
            _ => return None,
        })
    }
}

impl fmt::Display for ValidationIssueDetailCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display())
    }
}

/// How to interpret [`crate::ValidationIssue::expression`] for OperationOutcome extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationSourceKind {
    /// Absolute or canonical URI (`http(s)://…`, `urn:…`, or `url|version`).
    CanonicalUri,
    /// FHIRPath (or simple path) expression text.
    FhirPath,
    /// Constraint / invariant id (e.g. `ele-1`, `dom-2`).
    InvariantId,
    Unclassified,
}

impl ValidationSourceKind {
    pub fn as_code(self) -> &'static str {
        match self {
            Self::CanonicalUri => "canonical-uri",
            Self::FhirPath => "fhirpath",
            Self::InvariantId => "invariant-id",
            Self::Unclassified => "other",
        }
    }
}

/// Classify a stored `expression` string for serialization (`valueUri` vs `valueString`, etc.).
pub fn classify_validation_source(s: &str) -> ValidationSourceKind {
    let t = s.trim();
    if t.is_empty() {
        return ValidationSourceKind::Unclassified;
    }
    if t.starts_with("http://") || t.starts_with("https://") || t.starts_with("urn:") {
        return ValidationSourceKind::CanonicalUri;
    }
    if let Some((left, _)) = t.split_once('|') {
        if left.starts_with("http://") || left.starts_with("https://") || left.starts_with("urn:") {
            return ValidationSourceKind::CanonicalUri;
        }
    }
    if looks_like_invariant_id(t) {
        return ValidationSourceKind::InvariantId;
    }
    if looks_like_fhirpath(t) {
        return ValidationSourceKind::FhirPath;
    }
    ValidationSourceKind::Unclassified
}

/// Typical HL7 constraint keys: short lowercase prefix, hyphen, numeric suffix (`ele-1`, `dom-2`).
fn looks_like_invariant_id(t: &str) -> bool {
    if t.len() > 32 || t.len() < 3 {
        return false;
    }
    if t.contains(' ') || t.contains('.') || t.contains('/') {
        return false;
    }
    let Some((head, tail)) = t.rsplit_once('-') else {
        return false;
    };
    if head.is_empty() || tail.is_empty() {
        return false;
    }
    if !(2..=12).contains(&head.len()) {
        return false;
    }
    if !head.chars().all(|c| c.is_ascii_lowercase()) {
        return false;
    }
    tail.chars().all(|c| c.is_ascii_digit())
}

fn looks_like_fhirpath(t: &str) -> bool {
    if t.contains(" where") || t.contains("where(") || t.contains(".where") {
        return true;
    }
    if t.contains("exists(") || t.contains("empty(") || t.contains("hasValue(") {
        return true;
    }
    if t.contains("->") {
        return true;
    }
    if t.contains(" = ") || t.contains("!=") || t.contains(">=") || t.contains("<=") {
        return true;
    }
    if t.contains('`') {
        return true;
    }
    // Simple path starting with ResourceName.
    if t.contains('.')
        && t.chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false)
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Every enum variant; keep in sync when adding new [`ValidationIssueDetailCode`] variants.
    fn all_detail_codes() -> [ValidationIssueDetailCode; 30] {
        [
            ValidationIssueDetailCode::RequiredElementMissing,
            ValidationIssueDetailCode::MaximumCardinalityExceeded,
            ValidationIssueDetailCode::ElementValueInvalid,
            ValidationIssueDetailCode::ConstraintViolation,
            ValidationIssueDetailCode::StructureInvalid,
            ValidationIssueDetailCode::ReferenceNotFound,
            ValidationIssueDetailCode::DuplicateDetected,
            ValidationIssueDetailCode::EditConflict,
            ValidationIssueDetailCode::InteractionNotSupported,
            ValidationIssueDetailCode::TerminologyValidationFailed,
            ValidationIssueDetailCode::TerminologyServiceUnavailable,
            ValidationIssueDetailCode::RecursionDepthReached,
            ValidationIssueDetailCode::ProfileCycleDetected,
            ValidationIssueDetailCode::BusinessRuleViolation,
            ValidationIssueDetailCode::ValidationException,
            ValidationIssueDetailCode::ValidationFailure,
            ValidationIssueDetailCode::CodeWithoutSystem,
            ValidationIssueDetailCode::InvalidBindableValue,
            ValidationIssueDetailCode::RequiredBindingMiss,
            ValidationIssueDetailCode::ExtensibleBindingMiss,
            ValidationIssueDetailCode::PreferredBindingMiss,
            ValidationIssueDetailCode::ExampleBindingMiss,
            ValidationIssueDetailCode::PatternConstraintMismatch,
            ValidationIssueDetailCode::FixedConstraintMismatch,
            ValidationIssueDetailCode::SliceOrderViolation,
            ValidationIssueDetailCode::SliceMinCardinalityMissing,
            ValidationIssueDetailCode::SliceMaxCardinalityExceeded,
            ValidationIssueDetailCode::SliceOpenAtEndViolation,
            ValidationIssueDetailCode::SlicingUnsupportedDiscriminator,
            ValidationIssueDetailCode::SlicingNoDiscriminators,
        ]
    }

    #[test]
    fn detail_code_as_codes_are_unique() {
        let mut seen = HashSet::new();
        for v in all_detail_codes() {
            assert!(seen.insert(v.as_code()), "duplicate as_code for {:?}", v);
        }
    }

    #[test]
    fn detail_code_round_trip() {
        for v in all_detail_codes() {
            assert_eq!(
                ValidationIssueDetailCode::try_from_detail_code(v.as_code()),
                Some(v),
                "try_from_detail_code missing for {:?}",
                v
            );
        }
    }

    #[test]
    fn try_from_detail_code_rejects_unknown() {
        assert_eq!(ValidationIssueDetailCode::try_from_detail_code(""), None);
        assert_eq!(
            ValidationIssueDetailCode::try_from_detail_code("not-a-real-code"),
            None
        );
    }

    #[test]
    fn from_issue_category_maps_coarse_codes() {
        assert_eq!(
            ValidationIssueDetailCode::from_issue_category("required"),
            ValidationIssueDetailCode::RequiredElementMissing
        );
        assert_eq!(
            ValidationIssueDetailCode::from_issue_category("value"),
            ValidationIssueDetailCode::ElementValueInvalid
        );
        assert_eq!(
            ValidationIssueDetailCode::from_issue_category("invariant"),
            ValidationIssueDetailCode::ConstraintViolation
        );
        assert_eq!(
            ValidationIssueDetailCode::from_issue_category("unknown-internal"),
            ValidationIssueDetailCode::ValidationFailure
        );
    }

    #[test]
    fn display_and_fallback_are_non_empty() {
        for v in all_detail_codes() {
            assert!(!v.display().is_empty(), "{v:?}");
            assert!(!v.details_text_fallback().is_empty(), "{v:?}");
        }
    }

    #[test]
    fn classify_valueset_url() {
        assert_eq!(
            classify_validation_source("http://hl7.org/fhir/ValueSet/administrative-gender|4.0.1"),
            ValidationSourceKind::CanonicalUri
        );
    }

    #[test]
    fn classify_fhirpath() {
        assert_eq!(
            classify_validation_source("Patient.name.where(use='official').exists()"),
            ValidationSourceKind::FhirPath
        );
        assert_eq!(
            classify_validation_source("Patient.gender"),
            ValidationSourceKind::FhirPath
        );
    }

    #[test]
    fn classify_invariant_id() {
        assert_eq!(
            classify_validation_source("ele-1"),
            ValidationSourceKind::InvariantId
        );
        assert_eq!(
            classify_validation_source("dom-2"),
            ValidationSourceKind::InvariantId
        );
    }
}
