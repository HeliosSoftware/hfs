//! Binding validation for FHIR R6.
//!
//! This module implements ValueSet binding validation for generated R6 resources.
//!
//! Supported binding target kinds (per FHIR `ElementDefinition.binding`):
//! - primitives `code`, `string`, `uri`
//! - `Coding`, `CodeableConcept`, `Quantity`, `CodeableReference`
//! - choice `[x]` elements ([`BindingTargetKind::Choice`]): handler is chosen from the instance JSON shape
//!
//! Validation flow:
//!
//! 1. Local generated ValueSet helpers are used first.
//! 2. If local validation returns `RemoteValidationRequired`, the validator
//!    calls the provided `TerminologyService` / `TerminologyServiceSync`.
//! 3. The result is converted into `ValidationIssue` according to binding strength.
//!
//! Sync vs async:
//!
//! - `*_binding` functions use `TerminologyServiceSync`
//! - `*_binding_async` functions use `TerminologyService`
//!
//! The async path is preferred for production validation where remote
//! terminology servers may be required.

use crate::binding::common::{
    BindingCheckContextAsync, BindingCheckContextSync, bindable_primitive_string_value,
    binding_issue_summary, choice_declared_allows_kind, get_json_values_with_instance_paths,
    prettify_remote_terminology_error, primitive_choice_target_kind, relative_binding_path,
    root_instance_path,
};
use crate::binding::engine::{
    BindingVersionAdapter, evaluate_local_codeable_concept_binding,
    evaluate_local_codeable_reference_binding, evaluate_local_coding_binding,
    evaluate_local_primitive_code_binding, evaluate_local_primitive_value_binding,
    evaluate_local_quantity_binding,
};
use crate::issue_code;
use crate::terminology::service::{TerminologyService, TerminologyServiceSync};
use crate::{ValidationIssue, Validator};
use fhir_validation_types::{BindingDef, BindingTargetKind};
use helios_fhir::TerminologyValidationError;
use helios_fhir::r6::terminology::index as terminology_index;
use helios_fhir::r6::{CodeableConcept, CodeableReference, Coding, Quantity};
use serde::Serialize;
use serde_json::Value;

struct R6BindingAdapter;

impl BindingVersionAdapter for R6BindingAdapter {
    type Coding = Coding;
    type CodeableConcept = CodeableConcept;
    type Quantity = Quantity;
    type CodeableReference = CodeableReference;
    type PrimitiveCode = String;

    fn primitive_code_value(value: &Self::PrimitiveCode) -> Option<&str> {
        Some(value.as_str()).filter(|v| !v.is_empty())
    }

    fn coding_system(coding: &Self::Coding) -> Option<&str> {
        coding_system(coding)
    }

    fn coding_code(coding: &Self::Coding) -> Option<&str> {
        coding_code(coding)
    }

    fn coding_display(coding: &Self::Coding) -> Option<&str> {
        coding_display(coding)
    }

    fn codeable_concept_codings(
        cc: &Self::CodeableConcept,
    ) -> Box<dyn Iterator<Item = &Self::Coding> + '_> {
        match cc.coding.as_ref() {
            Some(codings) => Box::new(codings.iter()),
            None => Box::new(std::iter::empty()),
        }
    }

    fn quantity_system(quantity: &Self::Quantity) -> Option<&str> {
        quantity_system(quantity)
    }

    fn quantity_code(quantity: &Self::Quantity) -> Option<&str> {
        quantity_code(quantity)
    }

    fn codeable_reference_concept(
        value: &Self::CodeableReference,
    ) -> Option<&Self::CodeableConcept> {
        value.concept.as_ref()
    }

    fn summarize_codeable_concept_codings(cc: &Self::CodeableConcept) -> String {
        summarize_codeable_concept_codings(cc)
    }
}

#[cfg(feature = "R6")]
pub fn coding_system(coding: &Coding) -> Option<&str> {
    coding.system.as_ref().and_then(|v| v.value.as_deref())
}
#[cfg(feature = "R6")]
pub fn coding_code(coding: &Coding) -> Option<&str> {
    coding.code.as_ref().and_then(|v| v.value.as_deref())
}
#[cfg(feature = "R6")]
pub fn coding_display(coding: &Coding) -> Option<&str> {
    coding.display.as_ref().and_then(|v| v.value.as_deref())
}
#[cfg(feature = "R6")]
pub fn quantity_system(quantity: &Quantity) -> Option<&str> {
    quantity.system.as_ref().and_then(|v| v.value.as_deref())
}
#[cfg(feature = "R6")]
pub fn quantity_code(quantity: &Quantity) -> Option<&str> {
    quantity.code.as_ref().and_then(|v| v.value.as_deref())
}

/// Render a CodeableConcept into a readable summary for diagnostics.
///
/// Produces strings like:
/// - system#code
/// - code
/// - <empty-coding>
fn summarize_codeable_concept_codings(cc: &CodeableConcept) -> String {
    let Some(codings) = cc.coding.as_ref() else {
        return "CodeableConcept has no codings".to_string();
    };

    let mut rendered = Vec::new();
    for coding in codings {
        let system = coding
            .system
            .as_ref()
            .and_then(|e| e.value.as_deref())
            .filter(|v| !v.is_empty());
        let code = coding
            .code
            .as_ref()
            .and_then(|e| e.value.as_deref())
            .filter(|v| !v.is_empty());

        match (system, code) {
            (Some(system), Some(code)) => rendered.push(format!("{}#{}", system, code)),
            (None, Some(code)) => rendered.push(code.to_string()),
            (Some(system), None) => rendered.push(format!("{}#<missing-code>", system)),
            (None, None) => rendered.push("<empty-coding>".to_string()),
        }
    }

    if rendered.is_empty() {
        "CodeableConcept has no codings".to_string()
    } else {
        rendered.join(", ")
    }
}
/// Validate a primitive FHIR `code` binding.
///
/// This is used for fields such as -
/// - `Patient.gender`
/// - `Observation.status`
/// - `Encounter.status`
///
/// The caller provides the already-extracted primitive string value (`code_value`) and
/// a generated local ValueSet wrapper function via `local_check`.
///
/// Validation flow:
/// 1. If the value is absent, do nothing. Presence/cardinality is handled elsewhere.
/// 2. Try local generated ValueSet validation.
/// 3. If local validation says remote terminology validation is required, use `TerminologyService`.
/// 4. Convert any miss into a `ValidationIssue` based on binding strength.
///
/// This is the synchronous variant.
/// See also: `validate_primitive_code_binding_async`.
pub fn validate_primitive_code_binding<F>(
    ctx: &BindingCheckContextSync<'_>,
    code_value: Option<&str>,
    implicit_system: Option<&str>,
    local_check: F,
) -> Vec<ValidationIssue>
where
    F: Fn(&str) -> Result<(), TerminologyValidationError>,
{
    let issues = Vec::new();

    let Some(code) = code_value else {
        // Missing primitive value is handled by structural/cardinality validation and/or invariants.
        return issues;
    };
    let local_outcome = evaluate_local_primitive_code_binding::<R6BindingAdapter, _>(
        ctx.valueset_url,
        &code.to_string(),
        |_, _, _| local_check(code),
    );

    match ctx.classify_local_outcome(local_outcome) {
        crate::binding::common::LocalBindingDisposition::Valid => vec![],
        crate::binding::common::LocalBindingDisposition::Done(issues) => issues,
        crate::binding::common::LocalBindingDisposition::NeedsRemote(req) => {
            let mut req = req;
            req.system = implicit_system.map(str::to_owned);
            ctx.execute_remote_sync(&req)
        }
    }
}
pub fn validate_primitive_value_binding<F>(
    ctx: &BindingCheckContextSync<'_>,
    value: Option<&str>,
    local_check: F,
) -> Vec<ValidationIssue>
where
    F: Fn(&str) -> Result<(), TerminologyValidationError>,
{
    let issues = Vec::new();

    let Some(value) = value else {
        // Missing primitive value is handled by structural/cardinality validation and/or invariants.
        return issues;
    };
    let local_outcome = evaluate_local_primitive_value_binding::<R6BindingAdapter, _>(
        ctx.valueset_url,
        &value.to_string(),
        |_, _, _| local_check(value),
    );

    match ctx.classify_local_outcome(local_outcome) {
        crate::binding::common::LocalBindingDisposition::Valid => vec![],
        crate::binding::common::LocalBindingDisposition::Done(issues) => issues,
        crate::binding::common::LocalBindingDisposition::NeedsRemote(req) => {
            ctx.execute_remote_sync(&req)
        }
    }
}

/// Async variant of `validate_primitive_code_binding`.
///
/// Uses `TerminologyService` for remote terminology calls.
pub async fn validate_primitive_code_binding_async<F>(
    ctx: &BindingCheckContextAsync<'_>,
    code: Option<&str>,
    implicit_system: Option<&str>,
    local_check: F,
) -> Vec<ValidationIssue>
where
    F: Fn(&str) -> Result<(), TerminologyValidationError>,
{
    let issues = Vec::new();

    let Some(code) = code else {
        // Missing primitive value is handled by structural/cardinality validation and/or invariants.
        return issues;
    };
    let local_outcome = evaluate_local_primitive_code_binding::<R6BindingAdapter, _>(
        ctx.valueset_url,
        &code.to_string(),
        |_, _, _| local_check(code),
    );

    match ctx.classify_local_outcome(local_outcome) {
        crate::binding::common::LocalBindingDisposition::Valid => vec![],
        crate::binding::common::LocalBindingDisposition::Done(issues) => issues,
        crate::binding::common::LocalBindingDisposition::NeedsRemote(req) => {
            let mut req = req;
            req.system = implicit_system.map(str::to_owned);
            ctx.execute_remote_async(&req).await
        }
    }
}
pub async fn validate_primitive_value_binding_async<F>(
    ctx: &BindingCheckContextAsync<'_>,
    value: Option<&str>,
    local_check: F,
) -> Vec<ValidationIssue>
where
    F: Fn(&str) -> Result<(), TerminologyValidationError>,
{
    let issues = Vec::new();

    let Some(value) = value else {
        // Missing primitive value is handled by structural/cardinality validation and/or invariants.
        return issues;
    };
    let local_outcome = evaluate_local_primitive_value_binding::<R6BindingAdapter, _>(
        ctx.valueset_url,
        &value.to_string(),
        |_, _, _| local_check(value),
    );

    match ctx.classify_local_outcome(local_outcome) {
        crate::binding::common::LocalBindingDisposition::Valid => vec![],
        crate::binding::common::LocalBindingDisposition::Done(issues) => issues,
        crate::binding::common::LocalBindingDisposition::NeedsRemote(req) => {
            ctx.execute_remote_async(&req).await
        }
    }
}
/// Validate a `Quantity` binding.
///
/// For terminology purposes, `Quantity` bindings validate the `system` + `code`
/// pair carried by the quantity.
pub fn validate_quantity_binding<F>(
    ctx: &BindingCheckContextSync<'_>,
    quantity: Option<&Quantity>,
    local_check: F,
) -> Vec<ValidationIssue>
where
    F: Fn(&Quantity) -> Result<(), TerminologyValidationError>,
{
    let mut issues = Vec::new();

    let Some(quantity) = quantity else {
        return issues;
    };

    let system = quantity_system(quantity).filter(|s| !s.is_empty());
    let code = quantity_code(quantity).filter(|c| !c.is_empty());

    if code.is_some() && system.is_none() {
        issues.push(ValidationIssue {
            severity: fhir_validation_types::Severity::Warning,
            code: issue_code::TERMINOLOGY.to_string(),
            fhir_path: ctx.fhir_path.to_string(),
            instance_path: None,
            expression: Some(ctx.valueset_url.to_string()),
            expression_kind: Some(crate::ValidationSourceKind::CanonicalUri),
            source_invariant_key: None,
            summary: Some(binding_issue_summary::QUANTITY_CODE_WITHOUT_SYSTEM.to_string()),
            detail_code: Some(crate::ValidationIssueDetailCode::CodeWithoutSystem),
            diagnostics:
                "A quantity code with no system has no defined meaning, and it cannot be validated. A system should be provided"
                    .to_string(),
        });
        return issues;
    }
    let local_outcome = evaluate_local_quantity_binding::<R6BindingAdapter, _>(
        ctx.valueset_url,
        quantity,
        |_, _, _| local_check(quantity),
    );

    match ctx.classify_local_outcome(local_outcome) {
        crate::binding::common::LocalBindingDisposition::Valid => vec![],
        crate::binding::common::LocalBindingDisposition::Done(issues) => issues,
        crate::binding::common::LocalBindingDisposition::NeedsRemote(req) => {
            ctx.execute_remote_sync(&req)
        }
    }
}
/// Async variant of `validate_quantity_binding`.
pub async fn validate_quantity_binding_async<F>(
    ctx: &BindingCheckContextAsync<'_>,
    quantity: Option<&Quantity>,
    local_check: F,
) -> Vec<ValidationIssue>
where
    F: Fn(&Quantity) -> Result<(), TerminologyValidationError>,
{
    let mut issues = Vec::new();

    let Some(quantity) = quantity else {
        return issues;
    };

    let system = quantity_system(quantity).filter(|s| !s.is_empty());
    let code = quantity_code(quantity).filter(|c| !c.is_empty());

    if code.is_some() && system.is_none() {
        issues.push(ValidationIssue {
            severity: fhir_validation_types::Severity::Warning,
            code: issue_code::TERMINOLOGY.to_string(),
            fhir_path: ctx.fhir_path.to_string(),
            instance_path: None,
            expression: Some(ctx.valueset_url.to_string()),
            expression_kind: Some(crate::ValidationSourceKind::CanonicalUri),
            source_invariant_key: None,
            summary: Some(binding_issue_summary::QUANTITY_CODE_WITHOUT_SYSTEM.to_string()),
            detail_code: Some(crate::ValidationIssueDetailCode::CodeWithoutSystem),
            diagnostics:
                "A quantity code with no system has no defined meaning, and it cannot be validated. A system should be provided"
                    .to_string(),
        });
        return issues;
    }
    let local_outcome = evaluate_local_quantity_binding::<R6BindingAdapter, _>(
        ctx.valueset_url,
        quantity,
        |_, _, _| local_check(quantity),
    );

    match ctx.classify_local_outcome(local_outcome) {
        crate::binding::common::LocalBindingDisposition::Valid => vec![],
        crate::binding::common::LocalBindingDisposition::Done(issues) => issues,
        crate::binding::common::LocalBindingDisposition::NeedsRemote(req) => {
            ctx.execute_remote_async(&req).await
        }
    }
}
/// Validate a `CodeableReference` binding.
///
/// Binding semantics apply to the `concept` side when present. A reference-only
/// CodeableReference does not produce a terminology binding issue here.
pub fn validate_codeable_reference_binding<F>(
    ctx: &BindingCheckContextSync<'_>,
    codeable_reference: Option<&CodeableReference>,
    local_check: F,
) -> Vec<ValidationIssue>
where
    F: Fn(&CodeableConcept) -> Result<(), TerminologyValidationError>,
{
    let Some(cr) = codeable_reference else {
        return Vec::new();
    };

    let local_outcome = evaluate_local_codeable_reference_binding::<R6BindingAdapter, _>(
        ctx.valueset_url,
        cr,
        |system, code, display| {
            let mut local_cc = CodeableConcept::default();
            let mut local_coding = Coding::default();
            local_coding.system = system.map(|s| helios_fhir::r6::Code::from(s.to_string()));
            local_coding.code = Some(helios_fhir::r6::Code::from(code.to_string()));
            local_coding.display = display.map(|d| helios_fhir::r6::String::from(d.to_string()));
            local_cc.coding = Some(vec![local_coding]);
            local_check(&local_cc)
        },
    );

    match ctx.classify_local_outcome(local_outcome) {
        crate::binding::common::LocalBindingDisposition::Valid => vec![],
        crate::binding::common::LocalBindingDisposition::Done(issues) => issues,
        crate::binding::common::LocalBindingDisposition::NeedsRemote(_req) => {
            validate_codeable_concept_binding(ctx, cr.concept.as_ref(), local_check)
        }
    }
}
/// Async variant of `validate_codeable_reference_binding`.
pub async fn validate_codeable_reference_binding_async<F>(
    ctx: &BindingCheckContextAsync<'_>,
    codeable_reference: Option<&CodeableReference>,
    local_check: F,
) -> Vec<ValidationIssue>
where
    F: Fn(&CodeableConcept) -> Result<(), TerminologyValidationError>,
{
    let Some(cr) = codeable_reference else {
        return Vec::new();
    };

    let local_outcome = evaluate_local_codeable_reference_binding::<R6BindingAdapter, _>(
        ctx.valueset_url,
        cr,
        |system, code, display| {
            let mut local_cc = CodeableConcept::default();
            let mut local_coding = Coding::default();
            local_coding.system = system.map(|s| helios_fhir::r6::Code::from(s.to_string()));
            local_coding.code = Some(helios_fhir::r6::Code::from(code.to_string()));
            local_coding.display = display.map(|d| helios_fhir::r6::String::from(d.to_string()));
            local_cc.coding = Some(vec![local_coding]);
            local_check(&local_cc)
        },
    );

    match ctx.classify_local_outcome(local_outcome) {
        crate::binding::common::LocalBindingDisposition::Valid => vec![],
        crate::binding::common::LocalBindingDisposition::Done(issues) => issues,
        crate::binding::common::LocalBindingDisposition::NeedsRemote(_req) => {
            validate_codeable_concept_binding_async(ctx, cr.concept.as_ref(), local_check).await
        }
    }
}
/// Validate a `CodeableConcept` binding.
///
/// Semantics:
/// - If there is no `coding`, do nothing here. Cardinality / profile rules handle presence.
/// - Try local generated ValueSet validation first for the whole concept.
/// - If local validation succeeds, stop.
/// - If local validation says remote validation is required, try each usable coding remotely.
///   If any coding validates true, the concept is accepted.
/// - If local validation definitively says the concept is not in the ValueSet, surface an issue
///   based on binding strength.
///
/// The caller supplies the generated local wrapper function, typically something like:
/// `|cc| MaritalStatusCodes::validate_codeable_concept(cc)`
///
/// This is the synchronous variant.
/// See also: `validate_codeable_concept_binding_async`.
pub fn validate_codeable_concept_binding<F>(
    ctx: &BindingCheckContextSync<'_>,
    codeable_concept: Option<&CodeableConcept>,
    local_check: F,
) -> Vec<ValidationIssue>
where
    F: Fn(&CodeableConcept) -> Result<(), TerminologyValidationError>,
{
    let issues = Vec::new();

    let Some(cc) = codeable_concept else {
        return issues;
    };

    let codings = match cc.coding.as_ref() {
        Some(codings) if !codings.is_empty() => codings,
        _ => return issues,
    };
    let local_outcome = evaluate_local_codeable_concept_binding::<R6BindingAdapter, _>(
        ctx.valueset_url,
        cc,
        |system, code, display| {
            let mut local_cc = cc.clone();
            let mut local_coding = Coding::default();
            local_coding.system = system.map(|s| helios_fhir::r6::Code::from(s.to_string()));
            local_coding.code = Some(helios_fhir::r6::Code::from(code.to_string()));
            local_coding.display = display.map(|d| helios_fhir::r6::String::from(d.to_string()));
            local_cc.coding = Some(vec![local_coding]);
            local_check(&local_cc)
        },
    );

    match ctx.classify_local_outcome(local_outcome) {
        crate::binding::common::LocalBindingDisposition::Valid => vec![],
        crate::binding::common::LocalBindingDisposition::Done(issues) => issues,
        crate::binding::common::LocalBindingDisposition::NeedsRemote(_req) => {
            let Some(terminology) = ctx.terminology else {
                return vec![crate::binding::common::terminology_unavailable_issue(
                    ctx.fhir_path,
                    ctx.valueset_url,
                    "Remote terminology validation required but no TerminologyService was provided"
                        .to_string(),
                )];
            };

            let mut any_usable_coding = false;
            let mut scan = crate::binding::common::CodeableConceptRemoteScan::default();

            for coding in codings {
                let system = coding_system(coding);
                let code = coding_code(coding);
                let display = coding_display(coding);

                let Some(code) = code else {
                    continue;
                };

                any_usable_coding = true;

                let outcome = terminology.member_of(ctx.valueset_url, system, code, display);
                if let Err(e) = crate::binding::common::merge_remote_member_of_for_coding(
                    &mut scan,
                    outcome,
                    system,
                    code,
                    ctx.valueset_url,
                ) {
                    return vec![crate::binding::common::terminology_validation_issue(
                        ctx.fhir_path,
                        ctx.valueset_url,
                        prettify_remote_terminology_error(ctx.valueset_url, &e),
                    )];
                }
                if scan.any_match {
                    break;
                }
            }

            if scan.any_match {
                return vec![];
            }

            if scan.any_remote_undecidable {
                return vec![crate::binding::common::terminology_membership_not_locally_verifiable_issue(
                    ctx.fhir_path,
                    ctx.valueset_url,
                    ctx.strength,
                    scan.remote_undecidable_message.unwrap_or_else(|| {
                        "Local terminology cannot determine ValueSet membership; use a remote terminology service for a definitive validation result.".to_string()
                    }),
                )];
            }

            if !any_usable_coding {
                return vec![crate::binding::common::value_issue(
                    ctx.fhir_path,
                    ctx.valueset_url,
                    "CodeableConcept has no usable coding with a code value for terminology validation",
                    crate::ValidationIssueDetailCode::InvalidBindableValue,
                    "CodeableConcept has no usable coding with a code value for terminology validation"
                        .to_string(),
                )];
            }

            if let Some(err) = scan.last_local_failure {
                return crate::binding::common::local_error_to_issues(
                    ctx.validator,
                    ctx.fhir_path,
                    ctx.valueset_url,
                    ctx.strength,
                    err,
                );
            }

            let diagnostics = scan.last_miss_diagnostics.unwrap_or_else(|| {
                let coding_summary = summarize_codeable_concept_codings(cc);
                format!(
                    "The provided coding(s) {} were not found in ValueSet {}",
                    coding_summary, ctx.valueset_url
                )
            });

            match ctx.issue_for_binding_miss(diagnostics) {
                Some(issue) => vec![issue],
                None => vec![],
            }
        }
    }
}

/// Async variant of `validate_codeable_concept_binding`.
///
/// Remote terminology is awaited using `TerminologyService`.
pub async fn validate_codeable_concept_binding_async<F>(
    ctx: &BindingCheckContextAsync<'_>,
    codeable_concept: Option<&CodeableConcept>,
    local_check: F,
) -> Vec<ValidationIssue>
where
    F: Fn(&CodeableConcept) -> Result<(), TerminologyValidationError>,
{
    let issues = Vec::new();

    let Some(cc) = codeable_concept else {
        return issues;
    };

    let codings = match cc.coding.as_ref() {
        Some(codings) if !codings.is_empty() => codings,
        _ => return issues,
    };
    let local_outcome = evaluate_local_codeable_concept_binding::<R6BindingAdapter, _>(
        ctx.valueset_url,
        cc,
        |system, code, display| {
            let mut local_cc = cc.clone();
            let mut local_coding = Coding::default();
            local_coding.system = system.map(|s| helios_fhir::r6::Code::from(s.to_string()));
            local_coding.code = Some(helios_fhir::r6::Code::from(code.to_string()));
            local_coding.display = display.map(|d| helios_fhir::r6::String::from(d.to_string()));
            local_cc.coding = Some(vec![local_coding]);
            local_check(&local_cc)
        },
    );

    match ctx.classify_local_outcome(local_outcome) {
        crate::binding::common::LocalBindingDisposition::Valid => vec![],
        crate::binding::common::LocalBindingDisposition::Done(issues) => issues,
        crate::binding::common::LocalBindingDisposition::NeedsRemote(_req) => {
            let Some(terminology) = ctx.terminology else {
                return vec![crate::binding::common::terminology_unavailable_issue(
                    ctx.fhir_path,
                    ctx.valueset_url,
                    "Remote terminology validation required but no TerminologyService was provided"
                        .to_string(),
                )];
            };

            let mut any_usable_coding = false;
            let mut scan = crate::binding::common::CodeableConceptRemoteScan::default();

            for coding in codings {
                let system = coding_system(coding);
                let code = coding_code(coding);
                let display = coding_display(coding);

                let Some(code) = code else {
                    continue;
                };

                any_usable_coding = true;

                let outcome = terminology
                    .member_of(ctx.valueset_url, system, code, display)
                    .await;
                if let Err(e) = crate::binding::common::merge_remote_member_of_for_coding(
                    &mut scan,
                    outcome,
                    system,
                    code,
                    ctx.valueset_url,
                ) {
                    return vec![crate::binding::common::terminology_validation_issue(
                        ctx.fhir_path,
                        ctx.valueset_url,
                        prettify_remote_terminology_error(ctx.valueset_url, &e),
                    )];
                }
                if scan.any_match {
                    break;
                }
            }

            if scan.any_match {
                return vec![];
            }

            if scan.any_remote_undecidable {
                return vec![crate::binding::common::terminology_membership_not_locally_verifiable_issue(
                    ctx.fhir_path,
                    ctx.valueset_url,
                    ctx.strength,
                    scan.remote_undecidable_message.unwrap_or_else(|| {
                        "Local terminology cannot determine ValueSet membership; use a remote terminology service for a definitive validation result.".to_string()
                    }),
                )];
            }

            if !any_usable_coding {
                return vec![crate::binding::common::value_issue(
                    ctx.fhir_path,
                    ctx.valueset_url,
                    "CodeableConcept has no usable coding with a code value for terminology validation",
                    crate::ValidationIssueDetailCode::InvalidBindableValue,
                    "CodeableConcept has no usable coding with a code value for terminology validation"
                        .to_string(),
                )];
            }

            if let Some(err) = scan.last_local_failure {
                return crate::binding::common::local_error_to_issues(
                    ctx.validator,
                    ctx.fhir_path,
                    ctx.valueset_url,
                    ctx.strength,
                    err,
                );
            }

            let diagnostics = scan.last_miss_diagnostics.unwrap_or_else(|| {
                let coding_summary = summarize_codeable_concept_codings(cc);
                format!(
                    "The provided coding(s) {} were not found in ValueSet {}",
                    coding_summary, ctx.valueset_url
                )
            });

            match ctx.issue_for_binding_miss(diagnostics) {
                Some(issue) => vec![issue],
                None => vec![],
            }
        }
    }
}
/// Validate a `Coding` binding.
///
/// This handles bindings declared on elements of type `Coding`.
///
/// Flow:
/// - local ValueSet validation first
/// - optional remote terminology
/// - diagnostics based on binding strength
///
/// This is the synchronous variant.
/// See also: `validate_coding_binding_async`.
pub fn validate_coding_binding<F>(
    ctx: &BindingCheckContextSync<'_>,
    coding: Option<&Coding>,
    local_check: F,
) -> Vec<ValidationIssue>
where
    F: Fn(&Coding) -> Result<(), TerminologyValidationError>,
{
    let Some(coding) = coding else {
        return vec![];
    };

    let local_outcome = evaluate_local_coding_binding::<R6BindingAdapter, _>(
        ctx.valueset_url,
        coding,
        |system, code, display| {
            let mut local_coding = coding.clone();
            local_coding.system = system.map(|s| helios_fhir::r6::Code::from(s.to_string()));
            local_coding.code = Some(helios_fhir::r6::Code::from(code.to_string()));
            local_coding.display = display.map(|d| helios_fhir::r6::String::from(d.to_string()));
            local_check(&local_coding)
        },
    );
    match ctx.classify_local_outcome(local_outcome) {
        crate::binding::common::LocalBindingDisposition::Valid => vec![],
        crate::binding::common::LocalBindingDisposition::Done(issues) => issues,
        crate::binding::common::LocalBindingDisposition::NeedsRemote(req) => {
            ctx.execute_remote_sync(&req)
        }
    }
}

/// Async variant of `validate_coding_binding`.
pub async fn validate_coding_binding_async<F>(
    ctx: &BindingCheckContextAsync<'_>,
    coding: Option<&Coding>,
    local_check: F,
) -> Vec<ValidationIssue>
where
    F: Fn(&Coding) -> Result<(), TerminologyValidationError>,
{
    let Some(coding) = coding else {
        return vec![];
    };

    let local_outcome = evaluate_local_coding_binding::<R6BindingAdapter, _>(
        ctx.valueset_url,
        coding,
        |system, code, display| {
            let mut local_coding = coding.clone();
            local_coding.system = system.map(|s| helios_fhir::r6::Code::from(s.to_string()));
            local_coding.code = Some(helios_fhir::r6::Code::from(code.to_string()));
            local_coding.display = display.map(|d| helios_fhir::r6::String::from(d.to_string()));
            local_check(&local_coding)
        },
    );
    match ctx.classify_local_outcome(local_outcome) {
        crate::binding::common::LocalBindingDisposition::Valid => vec![],
        crate::binding::common::LocalBindingDisposition::Done(issues) => issues,
        crate::binding::common::LocalBindingDisposition::NeedsRemote(req) => {
            ctx.execute_remote_async(&req).await
        }
    }
}
/// Convert a matched JSON instance path into a local binding instance path.
///
/// Binding paths are declared relative to the resource root,
/// but matches are found using absolute JSON paths.
/// This helper rebases the path so that the final `ValidationIssue`
/// points to the correct local element.
fn local_binding_instance_path(binding_path: &str, matched_instance_path: &str) -> String {
    let root = root_instance_path(binding_path);

    if let Some((_, local_root)) = root.rsplit_once('.') {
        if let Some(suffix) = matched_instance_path.strip_prefix(root) {
            return format!("{}{}", local_root, suffix);
        }
    }

    matched_instance_path.to_string()
}

/// Infer a concrete [`BindingTargetKind`] from instance JSON for choice `[x]` elements.
///
/// `declared` is [`BindingDef::choice_type_codes`](fhir_validation_types::BindingDef::choice_type_codes)
/// when present; it restricts which shapes are considered.
fn infer_r6_choice_kind(value: &Value, declared: Option<&[String]>) -> Option<BindingTargetKind> {
    if bindable_primitive_string_value(value).is_some() {
        let k = primitive_choice_target_kind(declared);
        return choice_declared_allows_kind(declared, k).then_some(k);
    }
    if !matches!(value, Value::Object(_)) {
        return None;
    }
    let v = value.clone();
    if choice_declared_allows_kind(declared, BindingTargetKind::Quantity)
        && serde_json::from_value::<Quantity>(v.clone()).ok().is_some()
    {
        return Some(BindingTargetKind::Quantity);
    }
    if choice_declared_allows_kind(declared, BindingTargetKind::CodeableReference)
        && serde_json::from_value::<CodeableReference>(v.clone())
            .ok()
            .is_some()
    {
        return Some(BindingTargetKind::CodeableReference);
    }
    if choice_declared_allows_kind(declared, BindingTargetKind::CodeableConcept)
        && serde_json::from_value::<CodeableConcept>(v.clone())
            .ok()
            .is_some()
    {
        return Some(BindingTargetKind::CodeableConcept);
    }
    if choice_declared_allows_kind(declared, BindingTargetKind::Coding)
        && serde_json::from_value::<Coding>(v).ok().is_some()
    {
        return Some(BindingTargetKind::Coding);
    }
    None
}

fn apply_r6_binding_sync_single(
    validator: &Validator,
    binding: &BindingDef,
    field_value: &Value,
    kind: BindingTargetKind,
    terminology: Option<&dyn TerminologyServiceSync>,
) -> Vec<ValidationIssue> {
    let ctx = BindingCheckContextSync::from_binding(validator, binding, terminology);
    match kind {
        BindingTargetKind::Code => {
            let code_value =
                bindable_primitive_string_value(field_value).or_else(|| field_value.as_str());
            let implicit_system = terminology_index::implicit_system(binding.value_set.as_str());
            validate_primitive_code_binding(&ctx, code_value, implicit_system, |code| {
                terminology_index::validate_code(binding.value_set.as_str(), code)
            })
        }
        BindingTargetKind::String | BindingTargetKind::Uri => {
            let text =
                bindable_primitive_string_value(field_value).or_else(|| field_value.as_str());
            validate_primitive_value_binding(&ctx, text, |code| {
                terminology_index::validate_code(binding.value_set.as_str(), code)
            })
        }
        BindingTargetKind::Coding => {
            let coding = serde_json::from_value::<Coding>(field_value.clone()).ok();
            validate_coding_binding(&ctx, coding.as_ref(), |coding| {
                terminology_index::validate_coding(binding.value_set.as_str(), coding)
            })
        }
        BindingTargetKind::CodeableConcept => {
            let codeable_concept =
                serde_json::from_value::<CodeableConcept>(field_value.clone()).ok();
            validate_codeable_concept_binding(&ctx, codeable_concept.as_ref(), |cc| {
                terminology_index::validate_codeable_concept(binding.value_set.as_str(), cc)
            })
        }
        BindingTargetKind::Quantity => {
            let quantity = serde_json::from_value::<Quantity>(field_value.clone()).ok();
            validate_quantity_binding(&ctx, quantity.as_ref(), |quantity| {
                terminology_index::validate_quantity(binding.value_set.as_str(), quantity)
            })
        }
        BindingTargetKind::CodeableReference => {
            let codeable_reference =
                serde_json::from_value::<CodeableReference>(field_value.clone()).ok();
            validate_codeable_reference_binding(&ctx, codeable_reference.as_ref(), |cc| {
                terminology_index::validate_codeable_concept(binding.value_set.as_str(), cc)
            })
        }
        BindingTargetKind::Choice | BindingTargetKind::Unsupported => vec![],
    }
}

async fn apply_r6_binding_async_single(
    validator: &Validator,
    binding: &BindingDef,
    field_value: &Value,
    kind: BindingTargetKind,
    terminology: Option<&dyn TerminologyService>,
) -> Vec<ValidationIssue> {
    let ctx = BindingCheckContextAsync::from_binding(validator, binding, terminology);
    match kind {
        BindingTargetKind::Code => {
            let code_value =
                bindable_primitive_string_value(field_value).or_else(|| field_value.as_str());
            let implicit_system = terminology_index::implicit_system(binding.value_set.as_str());
            validate_primitive_code_binding_async(&ctx, code_value, implicit_system, |code| {
                terminology_index::validate_code(binding.value_set.as_str(), code)
            })
            .await
        }
        BindingTargetKind::String | BindingTargetKind::Uri => {
            let text =
                bindable_primitive_string_value(field_value).or_else(|| field_value.as_str());
            validate_primitive_value_binding_async(&ctx, text, |code| {
                terminology_index::validate_code(binding.value_set.as_str(), code)
            })
            .await
        }
        BindingTargetKind::Coding => {
            let coding = serde_json::from_value::<Coding>(field_value.clone()).ok();
            validate_coding_binding_async(&ctx, coding.as_ref(), |coding| {
                terminology_index::validate_coding(binding.value_set.as_str(), coding)
            })
            .await
        }
        BindingTargetKind::CodeableConcept => {
            let codeable_concept =
                serde_json::from_value::<CodeableConcept>(field_value.clone()).ok();
            validate_codeable_concept_binding_async(&ctx, codeable_concept.as_ref(), |cc| {
                terminology_index::validate_codeable_concept(binding.value_set.as_str(), cc)
            })
            .await
        }
        BindingTargetKind::Quantity => {
            let quantity = serde_json::from_value::<Quantity>(field_value.clone()).ok();
            validate_quantity_binding_async(&ctx, quantity.as_ref(), |quantity| {
                terminology_index::validate_quantity(binding.value_set.as_str(), quantity)
            })
            .await
        }
        BindingTargetKind::CodeableReference => {
            let codeable_reference =
                serde_json::from_value::<CodeableReference>(field_value.clone()).ok();
            validate_codeable_reference_binding_async(&ctx, codeable_reference.as_ref(), |cc| {
                terminology_index::validate_codeable_concept(binding.value_set.as_str(), cc)
            })
            .await
        }
        BindingTargetKind::Choice | BindingTargetKind::Unsupported => vec![],
    }
}

/// Apply binding validation to a serialized R6 resource.
///
/// This function:
/// - walks JSON using generated binding paths
/// - extracts values with instance paths
/// - dispatches to the correct binding validator
/// - stamps `instance_path` on all produced issues
///
/// Local-first terminology validation is used.
/// Remote validation is performed only if required.
///
/// This is the synchronous binding dispatcher.
/// See also: `apply_r6_bindings_async`.
pub fn apply_r6_bindings<T>(
    validator: &Validator,
    focus: &T,
    bindings: &[BindingDef],
    terminology: Option<&dyn TerminologyServiceSync>,
) -> Vec<ValidationIssue>
where
    T: Serialize,
{
    let mut issues = Vec::new();

    let focus_json = match serde_json::to_value(focus) {
        Ok(value) => value,
        Err(err) => {
            issues.push(ValidationIssue {
                severity: fhir_validation_types::Severity::Error,
                code: issue_code::STRUCTURE.to_string(),
                fhir_path: "binding".to_string(),
                instance_path: None,
                expression: None,
                expression_kind: None,
                source_invariant_key: None,
                summary: Some(binding_issue_summary::RESOURCE_SERIALIZATION_FAILED.to_string()),
                detail_code: Some(crate::ValidationIssueDetailCode::ValidationException),
                diagnostics: format!("Failed to serialize focus for binding validation: {}", err),
            });
            return issues;
        }
    };

    for binding in bindings {
        let relative_path = relative_binding_path(&binding.path);
        let field_values = get_json_values_with_instance_paths(
            &focus_json,
            root_instance_path(&binding.path),
            relative_path,
        );
        for (field_value, instance_path) in &field_values {
            match binding.target_kind {
                BindingTargetKind::Unsupported => {}
                BindingTargetKind::Choice => {
                    if let Some(kind) =
                        infer_r6_choice_kind(field_value, binding.choice_type_codes.as_deref())
                    {
                        let mut child_issues = apply_r6_binding_sync_single(
                            validator,
                            binding,
                            field_value,
                            kind,
                            terminology,
                        );
                        let stamped_instance_path =
                            local_binding_instance_path(&binding.path, instance_path);
                        for issue in &mut child_issues {
                            issue.instance_path = Some(stamped_instance_path.clone());
                        }
                        issues.extend(child_issues);
                    }
                }
                kind => {
                    let mut child_issues = apply_r6_binding_sync_single(
                        validator,
                        binding,
                        field_value,
                        kind,
                        terminology,
                    );
                    let stamped_instance_path =
                        local_binding_instance_path(&binding.path, instance_path);
                    for issue in &mut child_issues {
                        issue.instance_path = Some(stamped_instance_path.clone());
                    }
                    issues.extend(child_issues);
                }
            }
        }
    }

    issues
}

/// Async binding dispatcher for R6 resources.
///
/// Same semantics as `apply_r6_bindings`, but uses
/// `TerminologyService` for remote terminology calls.
pub async fn apply_r6_bindings_async<T>(
    validator: &Validator,
    focus: &T,
    bindings: &[BindingDef],
    terminology: Option<&dyn TerminologyService>,
) -> Vec<ValidationIssue>
where
    T: Serialize,
{
    let mut issues = Vec::new();
    let focus_json = match serde_json::to_value(focus) {
        Ok(v) => v,
        Err(e) => {
            issues.push(ValidationIssue {
                severity: fhir_validation_types::Severity::Error,
                code: issue_code::STRUCTURE.to_string(),
                fhir_path: "binding".to_string(),
                instance_path: None,
                expression: None,
                expression_kind: None,
                source_invariant_key: None,
                summary: Some(binding_issue_summary::RESOURCE_SERIALIZATION_FAILED.to_string()),
                detail_code: Some(crate::ValidationIssueDetailCode::ValidationException),
                diagnostics: format!("Failed to serialize focus for binding validation: {}", e),
            });
            return issues;
        }
    };

    for binding in bindings {
        let relative_path = relative_binding_path(&binding.path);
        let field_values = get_json_values_with_instance_paths(
            &focus_json,
            root_instance_path(&binding.path),
            relative_path,
        );
        for (field_value, instance_path) in &field_values {
            match binding.target_kind {
                BindingTargetKind::Unsupported => {}
                BindingTargetKind::Choice => {
                    if let Some(kind) =
                        infer_r6_choice_kind(field_value, binding.choice_type_codes.as_deref())
                    {
                        let mut child_issues = apply_r6_binding_async_single(
                            validator,
                            binding,
                            field_value,
                            kind,
                            terminology,
                        )
                        .await;
                        let stamped_instance_path =
                            local_binding_instance_path(&binding.path, instance_path);
                        for issue in &mut child_issues {
                            issue.instance_path = Some(stamped_instance_path.clone());
                        }
                        issues.extend(child_issues);
                    }
                }
                kind => {
                    let mut child_issues = apply_r6_binding_async_single(
                        validator,
                        binding,
                        field_value,
                        kind,
                        terminology,
                    )
                    .await;
                    let stamped_instance_path =
                        local_binding_instance_path(&binding.path, instance_path);
                    for issue in &mut child_issues {
                        issue.instance_path = Some(stamped_instance_path.clone());
                    }
                    issues.extend(child_issues);
                }
            }
        }
    }

    issues
}
