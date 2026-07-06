//! Resource validation service: the REST layer's bridge to
//! `helios-fhir-validator`.
//!
//! Owns the per-version core validators (embedded schema packs), the
//! FHIRPath constraint evaluator, the optional terminology provider, and
//! the mapping from validator issues onto FHIR `OperationOutcome` issues.
//! Used by the `$validate` operation (always available) and — when
//! `HFS_VALIDATION_MODE` enables it — by the write-path enforcement hooks.

use crate::responses::operation_outcome::{
    Issue, IssueSeverity, IssueType, OperationOutcomeBuilder,
};
use helios_fhir::FhirVersion;
use helios_fhir_validator::fhirpath_effects::FhirPathConstraintEvaluator;
use helios_fhir_validator::{
    dotted_to_fhirpath, EffectHandlers, ErrorKind, Severity, TerminologyProvider,
    UnknownProfilePolicy, ValidationError, ValidationOptions, Validator,
};
use serde_json::Value;
use std::sync::Arc;

/// The validation service. Cheap to construct (core registries load lazily,
/// once per process); hold one in `AppState` for the AST cache to pay off.
pub struct ValidationService {
    constraint_evaluator: FhirPathConstraintEvaluator,
    terminology: Option<Arc<dyn TerminologyProvider>>,
    /// Constraint ids never evaluated (default: `dom-6`, the narrative
    /// warning that fires on almost every machine-generated resource).
    suppress_constraints: Vec<String>,
    /// Escalate terminology-service failures to errors.
    terminology_fail_closed: bool,
    /// Validate against `meta.profile` claims.
    use_meta_profiles: bool,
    unknown_profile: UnknownProfilePolicy,
}

impl Default for ValidationService {
    fn default() -> Self {
        Self {
            constraint_evaluator: FhirPathConstraintEvaluator::new(),
            terminology: None,
            suppress_constraints: vec!["dom-6".to_string()],
            terminology_fail_closed: false,
            use_meta_profiles: true,
            unknown_profile: UnknownProfilePolicy::Warn,
        }
    }
}

impl ValidationService {
    /// A service with the default posture: constraints on (dom-6
    /// suppressed), meta.profile honored, unresolvable profiles warned,
    /// no terminology provider.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the terminology provider (bindings stay unchecked without one).
    pub fn with_terminology(mut self, provider: Arc<dyn TerminologyProvider>) -> Self {
        self.terminology = Some(provider);
        self
    }

    /// Escalate terminology-service failures from warnings to errors.
    pub fn set_terminology_fail_closed(&mut self, closed: bool) {
        self.terminology_fail_closed = closed;
    }

    /// Replace the set of constraint ids that are never evaluated.
    pub fn set_suppress_constraints(&mut self, ids: Vec<String>) {
        self.suppress_constraints = ids;
    }

    /// Toggle validation against `meta.profile` claims.
    pub fn set_use_meta_profiles(&mut self, on: bool) {
        self.use_meta_profiles = on;
    }

    /// Validate a resource against the core pack for `version` plus any
    /// extra profile canonicals. Structural issues first, then constraint
    /// issues, then binding issues.
    pub async fn validate_resource(
        &self,
        version: FhirVersion,
        resource: &Value,
        profiles: Vec<String>,
    ) -> Vec<ValidationError> {
        let validator = Validator::new(helios_fhir_validator::packs::core_registry(version));
        let opts = ValidationOptions {
            profiles,
            use_meta_profiles: self.use_meta_profiles,
            unknown_profile: self.unknown_profile,
        };
        let handlers = EffectHandlers {
            constraints: Some(&self.constraint_evaluator),
            terminology: self.terminology.as_deref(),
            suppress_constraints: &self.suppress_constraints,
            terminology_fail_closed: self.terminology_fail_closed,
        };
        validator.validate(resource, version, &opts, &handlers).await
    }
}

/// Map one validator issue onto an OperationOutcome issue.
pub fn to_outcome_issue(error: &ValidationError) -> Issue {
    let code = match error.kind {
        ErrorKind::Required => IssueType::Required,
        ErrorKind::FixedValue | ErrorKind::PatternValue | ErrorKind::PrimitiveValue => {
            IssueType::Value
        }
        ErrorKind::FhirpathConstraint => IssueType::Invariant,
        ErrorKind::TerminologyBinding => IssueType::CodeInvalid,
        ErrorKind::UnknownSchema | ErrorKind::UnknownProfile => IssueType::NotSupported,
        // Everything structural: unknown-element, shape, cardinality,
        // slicing, choices, wrong container type.
        ErrorKind::UnknownElement
        | ErrorKind::NotArray
        | ErrorKind::NotSingular
        | ErrorKind::Type
        | ErrorKind::Excluded
        | ErrorKind::Min
        | ErrorKind::Max
        | ErrorKind::SliceCardinality
        | ErrorKind::SliceUnmatched
        | ErrorKind::SliceOrder
        | ErrorKind::Choice
        | ErrorKind::ChoiceExcluded => IssueType::Structure,
    };
    let severity = match error.severity {
        Severity::Error => IssueSeverity::Error,
        Severity::Warning => IssueSeverity::Warning,
    };
    Issue::new(severity, code, error.message.clone())
        .with_expression(dotted_to_fhirpath(&error.path))
}

/// Build the `$validate` OperationOutcome: the mapped issues, or the
/// canonical all-clear information issue when there are none.
pub fn validation_outcome(errors: &[ValidationError]) -> Value {
    let mut builder = OperationOutcomeBuilder::new();
    if errors.is_empty() {
        builder = builder.information(IssueType::Informational, "Validation successful");
    } else {
        for error in errors {
            builder = builder.add_issue(to_outcome_issue(error));
        }
    }
    builder.build()
}

/// Whether any issue is error severity (drives `$validate` reporting and
/// enforce-mode rejection).
pub fn has_errors(errors: &[ValidationError]) -> bool {
    errors.iter().any(|e| e.severity == Severity::Error)
}
