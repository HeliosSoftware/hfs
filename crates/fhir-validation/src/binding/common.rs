//! Shared helpers for binding validation across FHIR versions.
//!
//! This module contains:
//! - helpers for constructing binding-related `ValidationIssue`s
//! - array-aware JSON path traversal utilities used to resolve generated binding
//!   paths to concrete instance paths
//! - utilities for mapping a generated binding path to the root resource/datatype
//!   instance path used during validation
//!
//! Version-specific binding modules (`r4/binding.rs`, `r5/binding.rs`, etc.) use these
//! helpers to validate ValueSet bindings on the
//! [FHIR bindable types](https://hl7.org/fhir/elementdefinition-definitions.html#ElementDefinition.binding):
//! primitive `code`, `string`, `uri`, `Coding`, `CodeableConcept`, `Quantity`,
//! and `CodeableReference` (where the version supports it), while preserving
//! precise instance locations.

use crate::binding::engine::LocalBindingOutcome;
use crate::issue_code;
use crate::service::{TerminologyService, TerminologyServiceSync};
use crate::types::TerminologyMembershipOutcome;
use crate::validation_issue_detail::{ValidationIssueDetailCode, ValidationSourceKind};
use crate::{ValidationError, ValidationIssue, Validator};
use fhir_validation_types::{
    BindingDef, BindingStrength, BindingTargetKind, Severity,
    binding_target_kind_for_element_type_code,
};
use helios_fhir::TerminologyValidationError;
use serde_json::Value;

/// Whether `kind` is among declared choice type codes (`None` / empty slice = unrestricted).
pub(crate) fn choice_declared_allows_kind(
    declared: Option<&[String]>,
    kind: BindingTargetKind,
) -> bool {
    match declared {
        None => true,
        Some([]) => true,
        Some(codes) => codes
            .iter()
            .filter_map(|c| binding_target_kind_for_element_type_code(c.as_str()))
            .any(|k| k == kind),
    }
}

/// Map string-ish instance JSON to a primitive [`BindingTargetKind`] using declared choice codes.
pub(crate) fn primitive_choice_target_kind(declared: Option<&[String]>) -> BindingTargetKind {
    match declared {
        None => BindingTargetKind::String,
        Some(codes) => {
            if codes.iter().any(|c| c.as_str() == "code") {
                BindingTargetKind::Code
            } else if codes.iter().any(|c| c.as_str() == "string") {
                BindingTargetKind::String
            } else if codes.iter().any(|c| c.as_str() == "uri") {
                BindingTargetKind::Uri
            } else {
                BindingTargetKind::String
            }
        }
    }
}

/// Extract primitive text for terminology binding from instance JSON.
///
/// Accepts a JSON string or the generated element shape `{"value": "..."}` used
/// for FHIR primitives such as `code`, `string`, and `uri`.
pub(crate) fn bindable_primitive_string_value(value: &Value) -> Option<&str> {
    match value {
        Value::String(s) => Some(s.as_str()),
        Value::Object(map) => map
            .get(issue_code::FHIR_JSON_VALUE)
            .and_then(|v| v.as_str()),
        _ => None,
    }
}

/// Convert a binding miss into a `ValidationIssue` using validator policy for
/// the supplied binding strength.
///
/// Summary text is intentionally generic: terminology may reject a binding for
/// missing membership, wrong system, or display/designation mismatch—see
/// `diagnostics` for the specific explanation (e.g. remote `$validate-code` message).
///
/// Returns `None` when the binding strength should not surface an issue.
pub fn issue_for_binding_miss(
    validator: &Validator,
    fhir_path: &str,
    valueset_url: &str,
    strength: BindingStrength,
    diagnostics: String,
) -> Option<ValidationIssue> {
    validator
        .binding_miss_severity(strength)
        .map(|severity| ValidationIssue {
            severity,
            code: issue_code::VALUE.to_string(),
            fhir_path: fhir_path.to_string(),
            instance_path: None,
            expression: Some(valueset_url.to_string()),
            expression_kind: Some(ValidationSourceKind::CanonicalUri),
            source_invariant_key: None,
            summary: Some(binding_miss_summary(strength).to_string()),
            detail_code: Some(binding_miss_detail_code(strength)),
            diagnostics,
        })
}

/// Construct a terminology-related validation issue.
///
/// Used when local validation is insufficient and remote terminology validation
/// is required or fails.
pub fn terminology_validation_issue(
    fhir_path: &str,
    valueset_url: &str,
    diagnostics: String,
) -> ValidationIssue {
    ValidationIssue {
        severity: Severity::Error,
        code: issue_code::TERMINOLOGY.to_string(),
        fhir_path: fhir_path.to_string(),
        instance_path: None,
        expression: Some(valueset_url.to_string()),
        expression_kind: Some(ValidationSourceKind::CanonicalUri),
        source_invariant_key: None,
        detail_code: Some(ValidationIssueDetailCode::TerminologyValidationFailed),
        summary: Some("Terminology validation failed".to_string()),
        diagnostics,
    }
}

pub fn terminology_unavailable_issue(
    fhir_path: &str,
    valueset_url: &str,
    diagnostics: String,
) -> ValidationIssue {
    ValidationIssue {
        severity: Severity::Error,
        code: issue_code::TERMINOLOGY.to_string(),
        fhir_path: fhir_path.to_string(),
        instance_path: None,
        expression: Some(valueset_url.to_string()),
        expression_kind: Some(ValidationSourceKind::CanonicalUri),
        source_invariant_key: None,
        detail_code: Some(ValidationIssueDetailCode::TerminologyServiceUnavailable),
        summary: Some(
            "Terminology validation could not be completed - Service unavailable".to_string(),
        ),
        diagnostics,
    }
}

/// Local terminology cannot prove or disprove ValueSet membership (e.g. composed ValueSets not
/// expanded in-process). This is **not** a binding-strength “miss” against a fully evaluated set.
pub fn terminology_membership_not_locally_verifiable_issue(
    fhir_path: &str,
    valueset_url: &str,
    strength: BindingStrength,
    diagnostics: String,
) -> ValidationIssue {
    let severity = match strength {
        BindingStrength::Required => Severity::Error,
        BindingStrength::Extensible | BindingStrength::Preferred | BindingStrength::Example => {
            Severity::Warning
        }
    };
    ValidationIssue {
        severity,
        code: issue_code::TERMINOLOGY.to_string(),
        fhir_path: fhir_path.to_string(),
        instance_path: None,
        expression: Some(valueset_url.to_string()),
        expression_kind: Some(ValidationSourceKind::CanonicalUri),
        source_invariant_key: None,
        detail_code: Some(ValidationIssueDetailCode::TerminologyValidationFailed),
        summary: Some(
            "ValueSet membership could not be verified with local terminology alone".to_string(),
        ),
        diagnostics,
    }
}

/// Tracks `member_of` scans over multiple codings in the CodeableConcept “NeedsRemote” path.
#[derive(Default)]
pub struct CodeableConceptRemoteScan {
    pub any_match: bool,
    pub any_remote_undecidable: bool,
    pub remote_undecidable_message: Option<String>,
    pub last_miss_diagnostics: Option<String>,
    /// Structured failure from [`TerminologyMembershipOutcome::local_failure`] (e.g. wrong display).
    pub last_local_failure: Option<TerminologyValidationError>,
}

/// Merge one [`TerminologyServiceSync::member_of`] / async `member_of` result into [`CodeableConceptRemoteScan`].
pub fn merge_remote_member_of_for_coding(
    scan: &mut CodeableConceptRemoteScan,
    outcome: Result<TerminologyMembershipOutcome, ValidationError>,
    system: Option<&str>,
    code: &str,
    valueset_url: &str,
) -> Result<(), ValidationError> {
    match outcome {
        Ok(o) if o.is_member => {
            scan.any_match = true;
        }
        Ok(o) if o.remote_validation_required => {
            scan.any_remote_undecidable = true;
            if scan.remote_undecidable_message.is_none() {
                scan.remote_undecidable_message = o.message;
            }
        }
        Ok(o) => {
            if let Some(err) = o.local_failure.clone() {
                scan.last_local_failure = Some(err);
            }
            scan.last_miss_diagnostics = Some(o.message.unwrap_or_else(|| {
                if let Some(system) = system {
                    format!(
                        "The provided coding {}#{} was not found in ValueSet {}",
                        system, code, valueset_url
                    )
                } else {
                    format!(
                        "The provided code '{}' was not found in ValueSet {}",
                        code, valueset_url
                    )
                }
            }));
        }
        Err(e) => return Err(e),
    }
    Ok(())
}
/// Construct a value-shape validation issue.
///
/// Used when a bound field cannot be validated locally because the value is
/// malformed or missing the required structure for terminology validation.
pub fn value_issue(
    fhir_path: &str,
    valueset_url: &str,
    summary: &str,
    detail_code: ValidationIssueDetailCode,
    diagnostics: String,
) -> ValidationIssue {
    ValidationIssue {
        severity: Severity::Error,
        code: issue_code::VALUE.to_string(),
        fhir_path: fhir_path.to_string(),
        instance_path: None,
        expression: Some(valueset_url.to_string()),
        expression_kind: Some(ValidationSourceKind::CanonicalUri),
        source_invariant_key: None,
        summary: Some(summary.to_string()),
        detail_code: Some(detail_code),
        diagnostics,
    }
}

/// Convert a terminal local terminology validation error into concrete issues.
///
/// This helper assumes the caller has already separated control-flow outcomes
/// such as `Valid` and `NeedsRemote { .. }` from terminal local errors.
pub fn local_error_to_issues(
    validator: &Validator,
    fhir_path: &str,
    valueset_url: &str,
    strength: BindingStrength,
    err: TerminologyValidationError,
) -> Vec<ValidationIssue> {
    match err {
        TerminologyValidationError::NotInValueSet {
            valueset_url: _,
            system,
            code,
        } => {
            let diagnostics = match system {
                Some(system) => format!(
                    "Code '{}' from system '{}' not found in ValueSet '{}'",
                    code, system, valueset_url
                ),
                None => format!("Code '{}' not found in ValueSet '{}'", code, valueset_url),
            };

            issue_for_binding_miss(validator, fhir_path, valueset_url, strength, diagnostics)
                .into_iter()
                .collect()
        }

        TerminologyValidationError::MissingSystem(msg) => {
            vec![ValidationIssue {
                severity: Severity::Warning,
                code: issue_code::TERMINOLOGY.to_string(),
                fhir_path: fhir_path.to_string(),
                instance_path: None,
                expression: Some(valueset_url.to_string()),
                expression_kind: Some(ValidationSourceKind::CanonicalUri),
                source_invariant_key: None,
                summary: Some("Code cannot be validated without a system".to_string()),
                detail_code: Some(ValidationIssueDetailCode::CodeWithoutSystem),
                diagnostics: msg,
            }]
        }

        TerminologyValidationError::UnknownCode { system, code } => {
            vec![value_issue(
                fhir_path,
                valueset_url,
                "Code is not recognized in the declared code system",
                ValidationIssueDetailCode::InvalidBindableValue,
                format!("Unknown code '{}' in system '{}'", code, system),
            )]
        }

        TerminologyValidationError::WrongDisplay {
            system,
            code,
            expected,
            provided,
        } => {
            vec![value_issue(
                fhir_path,
                valueset_url,
                "Display does not match the code",
                ValidationIssueDetailCode::InvalidBindableValue,
                format!(
                    "Wrong display '{}' for code '{}' in system '{}'; expected '{}'",
                    provided, code, system, expected
                ),
            )]
        }

        TerminologyValidationError::InvalidInput(msg) => {
            vec![value_issue(
                fhir_path,
                valueset_url,
                "Value could not be validated against the bound value set",
                ValidationIssueDetailCode::InvalidBindableValue,
                msg,
            )]
        }

        TerminologyValidationError::RemoteValidationRequired(msg) => {
            tracing::warn!(
                fhir_path,
                valueset_url,
                "RemoteValidationRequired reached local_error_to_issues; treating as not locally verifiable"
            );
            vec![terminology_membership_not_locally_verifiable_issue(
                fhir_path,
                valueset_url,
                strength,
                msg,
            )]
        }
    }
}

/// Resolve all JSON values matching a relative binding path and return each
/// value together with its concrete indexed instance path.
///
/// This is array-aware and is used for repeated FHIR elements such as
/// `HumanName.use`, `Identifier.use`, and `ContactPoint.system`.
pub(crate) fn get_json_values_with_instance_paths<'a>(
    value: &'a Value,
    root_instance_path: &str,
    relative_path: &str,
) -> Vec<(&'a Value, String)> {
    if relative_path.is_empty() {
        return vec![(value, root_instance_path.to_string())];
    }

    let segments: Vec<&str> = relative_path.split('.').collect();
    let mut out = Vec::new();
    collect_json_values_with_paths(value, root_instance_path, &segments, &mut out);
    out
}

/// Recursive worker used by `get_json_values_with_instance_paths`.
///
/// Traverses objects and arrays, preserving indexed instance paths such as
/// `Patient.name[1].use` or `Patient.telecom[2].system`.
pub(crate) fn collect_json_values_with_paths<'a>(
    value: &'a Value,
    current_path: &str,
    remaining_segments: &[&str],
    out: &mut Vec<(&'a Value, String)>,
) {
    if remaining_segments.is_empty() {
        match value {
            Value::Array(items) => {
                for (idx, item) in items.iter().enumerate() {
                    out.push((item, format!("{current_path}[{idx}]")));
                }
            }
            _ => out.push((value, current_path.to_string())),
        }
        return;
    }

    let segment = remaining_segments[0];
    let rest = &remaining_segments[1..];

    match value {
        Value::Object(map) => {
            if let Some(child) = map.get(segment) {
                let next_path = format!("{current_path}.{segment}");
                collect_json_values_with_paths(child, &next_path, rest, out);
            }
        }

        Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                let indexed_path = format!("{current_path}[{idx}]");
                collect_json_values_with_paths(item, &indexed_path, remaining_segments, out);
            }
        }

        _ => {}
    }
}
/// Extract the current generated-type root from a generated binding path.
///
/// Binding definitions are attached to direct child fields of the current
/// generated type, so runtime path resolution needs the parent path of the
/// bound field rather than only the first segment.
///
/// Examples:
/// - `Patient.gender` -> `Patient`
/// - `HumanName.use` -> `HumanName`
/// - `Patient.contact.relationship` -> `Patient.contact`
/// - `Observation.component.code` -> `Observation.component`
///
/// Version-specific binding modules then resolve the final child segment
/// relative to this root to build concrete indexed instance paths.
pub(crate) fn root_instance_path(binding_path: &str) -> &str {
    binding_path
        .rsplit_once('.')
        .map(|(head, _)| head)
        .unwrap_or(binding_path)
}

/// Return the direct child binding path relative to the serialized focus object
/// being validated.
///
/// Generated binding definitions are always attached to direct child fields of
/// the current generated type. For top-level types this means:
/// - `Patient.gender` → `gender`
/// - `HumanName.use` → `use`
///
/// For nested helper types such as `Patient.contact`, the generated binding path
/// still carries the full logical path:
/// - `Patient.contact.relationship` → `relationship`
/// - `Patient.contact.gender` → `gender`
///
/// Using the final path segment keeps runtime binding resolution aligned with
/// the local serialized helper object shape.
pub(crate) fn relative_binding_path(binding_path: &str) -> &str {
    binding_path
        .rsplit_once('.')
        .map(|(_, tail)| tail)
        .unwrap_or(binding_path)
}
pub(crate) fn prettify_remote_terminology_error(
    valueset_url: &str,
    err: &crate::ValidationError,
) -> String {
    err.remote_binding_failure_diagnostics(valueset_url)
}

/// Inputs needed to turn a [`ValidationError`] from a ValueSet binding / `member_of` path into
/// [`ValidationIssue`] rows (path, ValueSet URL, binding strength, and validator policy).
#[derive(Debug, Clone, Copy)]
pub struct TerminologyIssueContext<'a> {
    pub validator: &'a Validator,
    pub fhir_path: &'a str,
    pub valueset_url: &'a str,
    pub strength: BindingStrength,
}

impl<'a> TerminologyIssueContext<'a> {
    pub fn new(
        validator: &'a Validator,
        fhir_path: &'a str,
        valueset_url: &'a str,
        strength: BindingStrength,
    ) -> Self {
        Self {
            validator,
            fhir_path,
            valueset_url,
            strength,
        }
    }
}

/// Map any [`ValidationError`] from terminology binding / [`crate::TerminologyService::member_of`]
/// into [`ValidationIssue`] rows.
///
/// - [`ValidationError::LocalTerminology`] uses [`local_error_to_issues`] (binding strength and
///   structured [`TerminologyValidationError`] semantics).
/// - [`ValidationError::RemoteTerminology`] uses the same terminology issue shape as remote
///   `$validate-code` failures ([`terminology_validation_issue`]).
/// - Other variants are rare on this path; they are mapped to exception / structure / generic
///   terminology issues so callers always get a stable issue list.
pub fn validation_error_to_issues(
    ctx: &TerminologyIssueContext<'_>,
    err: &ValidationError,
) -> Vec<ValidationIssue> {
    match err {
        ValidationError::LocalTerminology(e) => local_error_to_issues(
            ctx.validator,
            ctx.fhir_path,
            ctx.valueset_url,
            ctx.strength,
            e.clone(),
        ),
        ValidationError::RemoteTerminology(_) => {
            vec![terminology_validation_issue(
                ctx.fhir_path,
                ctx.valueset_url,
                err.remote_binding_failure_diagnostics(ctx.valueset_url),
            )]
        }
        ValidationError::InvalidRequest(req) => {
            vec![terminology_validation_issue(
                ctx.fhir_path,
                ctx.valueset_url,
                req.message.clone(),
            )]
        }
        ValidationError::FhirPath(e) => {
            vec![ValidationIssue {
                severity: Severity::Error,
                code: issue_code::EXCEPTION.to_string(),
                fhir_path: ctx.fhir_path.to_string(),
                instance_path: None,
                expression: Some(ctx.valueset_url.to_string()),
                expression_kind: Some(ValidationSourceKind::CanonicalUri),
                source_invariant_key: None,
                summary: Some(
                    "FHIRPath evaluation failed during terminology validation".to_string(),
                ),
                detail_code: Some(ValidationIssueDetailCode::ValidationException),
                diagnostics: e.to_string(),
            }]
        }
        ValidationError::InvalidStructureDefinition(msg) => {
            vec![ValidationIssue {
                severity: Severity::Error,
                code: issue_code::STRUCTURE.to_string(),
                fhir_path: ctx.fhir_path.to_string(),
                instance_path: None,
                expression: Some(ctx.valueset_url.to_string()),
                expression_kind: Some(ValidationSourceKind::CanonicalUri),
                source_invariant_key: None,
                summary: Some("StructureDefinition extraction failed".to_string()),
                detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
                diagnostics: msg.to_string(),
            }]
        }
        ValidationError::Internal(msg) => {
            vec![terminology_validation_issue(
                ctx.fhir_path,
                ctx.valueset_url,
                msg.clone(),
            )]
        }
    }
}

impl ValidationError {
    /// Converts this orchestration error into [`ValidationIssue`] rows for ValueSet binding /
    /// [`crate::TerminologyService::member_of`] paths.
    ///
    /// Equivalent to [`validation_error_to_issues`] with this error. There is no single-issue or
    /// context-free conversion: diagnostics and codes depend on [`TerminologyIssueContext`].
    pub fn to_binding_issues(&self, ctx: &TerminologyIssueContext<'_>) -> Vec<ValidationIssue> {
        validation_error_to_issues(ctx, self)
    }
}

#[derive(Debug, Clone)]
pub struct RemoteMembershipRequest {
    pub valueset_url: String,
    pub system: Option<String>,
    pub code: String,
    pub display: Option<String>,
}

/// Shared inputs for ValueSet binding checks (path, ValueSet, strength, terminology).
///
/// Version-specific `validate_*_binding` helpers take this instead of repeating
/// `validator`, `fhir_path`, `valueset_url`, `strength`, and `terminology`.
pub struct BindingCheckContextSync<'a> {
    pub validator: &'a Validator,
    pub fhir_path: &'a str,
    pub valueset_url: &'a str,
    pub strength: BindingStrength,
    pub terminology: Option<&'a dyn TerminologyServiceSync>,
}

impl<'a> BindingCheckContextSync<'a> {
    pub fn new(
        validator: &'a Validator,
        fhir_path: &'a str,
        valueset_url: &'a str,
        strength: BindingStrength,
        terminology: Option<&'a dyn TerminologyServiceSync>,
    ) -> Self {
        Self {
            validator,
            fhir_path,
            valueset_url,
            strength,
            terminology,
        }
    }

    pub fn terminology_issue_context(&self) -> TerminologyIssueContext<'a> {
        TerminologyIssueContext::new(
            self.validator,
            self.fhir_path,
            self.valueset_url,
            self.strength,
        )
    }

    pub fn from_binding(
        validator: &'a Validator,
        binding: &'a BindingDef,
        terminology: Option<&'a dyn TerminologyServiceSync>,
    ) -> Self {
        Self {
            validator,
            fhir_path: binding.path.as_str(),
            valueset_url: binding.value_set.as_str(),
            strength: binding.strength,
            terminology,
        }
    }

    pub fn classify_local_outcome(&self, outcome: LocalBindingOutcome) -> LocalBindingDisposition {
        classify_local_outcome(
            self.validator,
            self.fhir_path,
            self.valueset_url,
            self.strength,
            outcome,
        )
    }

    pub fn execute_remote_sync(&self, req: &RemoteMembershipRequest) -> Vec<ValidationIssue> {
        execute_remote_sync(
            self.validator,
            self.fhir_path,
            self.strength,
            self.terminology,
            req,
        )
    }

    pub fn issue_for_binding_miss(&self, diagnostics: String) -> Option<ValidationIssue> {
        issue_for_binding_miss(
            self.validator,
            self.fhir_path,
            self.valueset_url,
            self.strength,
            diagnostics,
        )
    }
}

/// Async terminology variant of [`BindingCheckContextSync`].
pub struct BindingCheckContextAsync<'a> {
    pub validator: &'a Validator,
    pub fhir_path: &'a str,
    pub valueset_url: &'a str,
    pub strength: BindingStrength,
    pub terminology: Option<&'a dyn TerminologyService>,
}

impl<'a> BindingCheckContextAsync<'a> {
    pub fn new(
        validator: &'a Validator,
        fhir_path: &'a str,
        valueset_url: &'a str,
        strength: BindingStrength,
        terminology: Option<&'a dyn TerminologyService>,
    ) -> Self {
        Self {
            validator,
            fhir_path,
            valueset_url,
            strength,
            terminology,
        }
    }

    pub fn terminology_issue_context(&self) -> TerminologyIssueContext<'a> {
        TerminologyIssueContext::new(
            self.validator,
            self.fhir_path,
            self.valueset_url,
            self.strength,
        )
    }

    pub fn from_binding(
        validator: &'a Validator,
        binding: &'a BindingDef,
        terminology: Option<&'a dyn TerminologyService>,
    ) -> Self {
        Self {
            validator,
            fhir_path: binding.path.as_str(),
            valueset_url: binding.value_set.as_str(),
            strength: binding.strength,
            terminology,
        }
    }

    pub fn classify_local_outcome(&self, outcome: LocalBindingOutcome) -> LocalBindingDisposition {
        classify_local_outcome(
            self.validator,
            self.fhir_path,
            self.valueset_url,
            self.strength,
            outcome,
        )
    }

    pub async fn execute_remote_async(
        &self,
        req: &RemoteMembershipRequest,
    ) -> Vec<ValidationIssue> {
        execute_remote_async(
            self.validator,
            self.fhir_path,
            self.strength,
            self.terminology,
            req,
        )
        .await
    }

    pub fn issue_for_binding_miss(&self, diagnostics: String) -> Option<ValidationIssue> {
        issue_for_binding_miss(
            self.validator,
            self.fhir_path,
            self.valueset_url,
            self.strength,
            diagnostics,
        )
    }
}

pub enum LocalBindingDisposition {
    Done(Vec<ValidationIssue>),
    NeedsRemote(RemoteMembershipRequest),
    Valid,
}

pub fn remote_request_from_outcome(
    outcome: &LocalBindingOutcome,
) -> Option<RemoteMembershipRequest> {
    match outcome {
        LocalBindingOutcome::NeedsRemote {
            valueset_url,
            system,
            code,
            display,
        } => Some(RemoteMembershipRequest {
            valueset_url: valueset_url.clone(),
            system: system.clone(),
            code: code.clone(),
            display: display.clone(),
        }),
        _ => None,
    }
}
pub fn classify_local_outcome(
    validator: &Validator,
    fhir_path: &str,
    valueset_url: &str,
    strength: BindingStrength,
    outcome: LocalBindingOutcome,
) -> LocalBindingDisposition {
    match outcome {
        LocalBindingOutcome::Valid => LocalBindingDisposition::Valid,
        LocalBindingOutcome::NeedsRemote {
            valueset_url,
            system,
            code,
            display,
        } => LocalBindingDisposition::NeedsRemote(RemoteMembershipRequest {
            valueset_url,
            system,
            code,
            display,
        }),
        LocalBindingOutcome::Error(err) => LocalBindingDisposition::Done(local_error_to_issues(
            validator,
            fhir_path,
            valueset_url,
            strength,
            err,
        )),
    }
}

pub fn remote_result_to_issues(
    validator: &Validator,
    fhir_path: &str,
    valueset_url: &str,
    strength: BindingStrength,
    req: &RemoteMembershipRequest,
    outcome: Result<TerminologyMembershipOutcome, ValidationError>,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    match outcome {
        Ok(outcome) if outcome.is_member => issues,
        Ok(outcome) if outcome.remote_validation_required => {
            let diagnostics = outcome.message.unwrap_or_else(|| {
                "Local terminology cannot determine ValueSet membership; use a remote terminology service for a definitive validation result.".to_string()
            });
            issues.push(terminology_membership_not_locally_verifiable_issue(
                fhir_path,
                valueset_url,
                strength,
                diagnostics,
            ));
            issues
        }
        Ok(outcome) => {
            if let Some(err) = outcome.local_failure.clone() {
                return local_error_to_issues(validator, fhir_path, valueset_url, strength, err);
            }

            let diagnostics = outcome.message.unwrap_or_else(|| {
                if let Some(system) = &req.system {
                    format!(
                        "The provided coding {}#{} was not found in ValueSet {}",
                        system, req.code, valueset_url
                    )
                } else {
                    format!(
                        "The provided code '{}' was not found in ValueSet {}",
                        req.code, valueset_url
                    )
                }
            });

            if let Some(issue) = crate::binding::common::issue_for_binding_miss(
                validator,
                fhir_path,
                valueset_url,
                strength,
                diagnostics,
            ) {
                issues.push(issue);
            }
            issues
        }
        Err(e) => {
            let ctx = TerminologyIssueContext::new(validator, fhir_path, valueset_url, strength);
            validation_error_to_issues(&ctx, &e)
        }
    }
}
pub fn execute_remote_sync(
    validator: &Validator,
    fhir_path: &str,
    strength: BindingStrength,
    terminology: Option<&dyn TerminologyServiceSync>,
    req: &RemoteMembershipRequest,
) -> Vec<ValidationIssue> {
    let Some(terminology) = terminology else {
        return vec![crate::binding::common::terminology_unavailable_issue(
            fhir_path,
            &req.valueset_url,
            "Remote terminology validation required but no TerminologyService was provided"
                .to_string(),
        )];
    };

    let outcome = terminology.member_of(
        &req.valueset_url,
        req.system.as_deref(),
        &req.code,
        req.display.as_deref(),
    );

    remote_result_to_issues(
        validator,
        fhir_path,
        &req.valueset_url,
        strength,
        req,
        outcome,
    )
}

pub async fn execute_remote_async(
    validator: &Validator,
    fhir_path: &str,
    strength: BindingStrength,
    terminology: Option<&dyn TerminologyService>,
    req: &RemoteMembershipRequest,
) -> Vec<ValidationIssue> {
    let Some(terminology) = terminology else {
        return vec![crate::binding::common::terminology_unavailable_issue(
            fhir_path,
            &req.valueset_url,
            "Remote terminology validation required but no TerminologyService was provided"
                .to_string(),
        )];
    };

    let outcome = terminology
        .member_of(
            &req.valueset_url,
            req.system.as_deref(),
            &req.code,
            req.display.as_deref(),
        )
        .await;

    remote_result_to_issues(
        validator,
        fhir_path,
        &req.valueset_url,
        strength,
        req,
        outcome,
    )
}

/// Shared [`ValidationIssue::summary`] text for binding validation across FHIR versions.
pub mod binding_issue_summary {
    pub const QUANTITY_CODE_WITHOUT_SYSTEM: &str = "Quantity code is present without a code system";
    pub const RESOURCE_SERIALIZATION_FAILED: &str =
        "Resource serialization failed during binding validation";
}

fn binding_miss_summary(strength: BindingStrength) -> &'static str {
    match strength {
        BindingStrength::Required => {
            "Does not satisfy the required value set binding (verify system, code, and display)"
        }
        BindingStrength::Extensible => {
            "Does not satisfy the extensible value set binding (verify system, code, and display)"
        }
        BindingStrength::Preferred => {
            "Does not satisfy the preferred value set binding (verify system, code, and display)"
        }
        BindingStrength::Example => {
            "Does not satisfy the example value set binding (verify system, code, and display)"
        }
    }
}

fn binding_miss_detail_code(strength: BindingStrength) -> ValidationIssueDetailCode {
    match strength {
        BindingStrength::Required => ValidationIssueDetailCode::RequiredBindingMiss,
        BindingStrength::Extensible => ValidationIssueDetailCode::ExtensibleBindingMiss,
        BindingStrength::Preferred => ValidationIssueDetailCode::PreferredBindingMiss,
        BindingStrength::Example => ValidationIssueDetailCode::ExampleBindingMiss,
    }
}
