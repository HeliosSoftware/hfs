//! Slicing validation for extracted profile rules.
//!
//! Current scope:
//! - validates slicing on repeating elements described by [`ExtractedSlicing`](crate::profile::types::ExtractedSlicing)
//! - supports discriminator kinds: `value`, `type`, `exists`, `position`, and
//!   `profile` (v1, without `.resolve()`)
//! - supports multi-discriminator matching using logical AND semantics
//! - enforces slice min/max cardinality
//! - enforces `closed`, `open`, and `openAtEnd` slicing rules
//! - enforces ordered slicing when `ordered = true`
//!
//! Notes on FHIR semantics:
//! - `value` discriminator matching is implemented via fixed/pattern constraints
//!   extracted from the relevant slice element or slice child element.
//! - `type` discriminator matching is implemented by inferring candidate JSON/FHIR
//!   type codes from the runtime value and comparing them with extracted type
//!   constraints.
//! - `exists` discriminator matching is implemented by inferring expected presence
//!   from the slice discriminator child rule cardinality (`min > 0` => must exist,
//!   `max = 0` => must not exist).
//! - `position` discriminator matching is implemented using the item index within
//!   the repeating element and the declaration order of the slice names.
//!
//! Not yet implemented:
//! - full profile-based differentiation using FHIRPath `resolve()`
//!
//! For `profile` discriminators, the FHIR specification distinguishes between:
//! - conformance of the nominated element itself to one or more profiles, and
//! - when the discriminator path includes `.resolve()`, conformance of the
//!   referenced target resource to one or more target profiles.
//!
//! That second case requires the validator to resolve the reference target and
//! validate the resolved resource against the candidate profiles. This validator
//! does not yet implement FHIRPath `resolve()`, so profile-based discriminator
//! support is intentionally deferred until that capability is available or a
//! suitable equivalent resolution strategy is introduced.

use crate::issue_code;
use crate::profile::helpers::{
    get_values_at_relative_path, get_values_with_paths_at_relative_path, json_type_codes,
    parse_slice_max,
};
use crate::profile::types::{
    ExtractedDiscriminatorType, ExtractedElementRule, ExtractedProfile,
    ExtractedSliceDiscriminator, ExtractedSlicingRules, ExtractedTypeConstraint,
    ExtractedValueConstraint,
};
use crate::profile::validate::validate_profile_with_depth;
use crate::validation_context::{ValidationContext, ValidationState};
use crate::validation_issue_detail::ValidationIssueDetailCode;
use crate::{Severity, ValidationIssue};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

/// Validate slicing rules for a serialized resource instance against an extracted
/// profile.
///
/// This function operates on the JSON projection of the resource and performs:
/// - slice membership determination for each repeated item
/// - conflict detection when an item matches multiple slices
/// - rule enforcement for `closed`, `open`, and `openAtEnd`
/// - ordered slicing checks when `ordered = true`
/// - per-slice min/max cardinality checks after all items have been classified
///
/// Matching is driven entirely by the extracted differential metadata in
/// [`ExtractedProfile`]. This function does not evaluate general FHIRPath
/// expressions; it uses extracted discriminator metadata plus JSON path traversal.
pub fn validate_slicing<T: Serialize>(
    resource: &T,
    resource_type: &str,
    profile: &ExtractedProfile,
) -> Vec<ValidationIssue> {
    let mut state = ValidationState::default();
    validate_slicing_with_context(None, &mut state, resource, resource_type, profile)
}

/// Like [`validate_slicing`] but shares [`ValidationState`] with surrounding validation (recursion
/// / [`ValidationContext`] when provided).
///
/// The optional [`ValidationContext`] is forwarded where nested slice validation may recursively
/// invoke profile checks (e.g. profile discriminators delegating into `validate_profile_with_depth`).
pub fn validate_slicing_with_context<T: Serialize>(
    ctx: Option<&ValidationContext<'_>>,
    state: &mut ValidationState,
    resource: &T,
    resource_type: &str,
    profile: &ExtractedProfile,
) -> Vec<ValidationIssue> {
    let root = match serde_json::to_value(resource) {
        Ok(value) => value,
        Err(err) => {
            return vec![ValidationIssue {
                severity: Severity::Error,
                code: "processing".to_string(),
                summary: Some(
                    "Resource could not be serialized for slicing validation".to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::ValidationException),
                diagnostics: format!(
                    "Failed to serialize resource while validating slicing: {}",
                    err
                ),
                expression: None,
                fhir_path: resource_type.to_string(),
                instance_path: Some(resource_type.to_string()),
            }];
        }
    };

    let mut issues = Vec::new();

    for base_rule in profile
        .element_rules
        .iter()
        .filter(|rule| rule.slicing.is_some())
    {
        let slicing = match &base_rule.slicing {
            Some(s) => s,
            None => continue,
        };

        if slicing.discriminators.is_empty() {
            issues.push(ValidationIssue {
                severity: Severity::Warning,
                code: "business-rule".to_string(),
                summary: Some("Slicing rule has no discriminators".to_string()),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::SlicingNoDiscriminators),
                diagnostics: format!(
                    "Slicing on '{}' has no discriminators; slicing validation was skipped.",
                    base_rule.path
                ),
                expression: None,
                fhir_path: base_rule.path.clone(),
                instance_path: Some(base_rule.path.clone()),
            });
            continue;
        }

        let unsupported_discriminators: Vec<_> = slicing
            .discriminators
            .iter()
            .filter(|d| {
                !matches!(
                    d.discriminator_type,
                    ExtractedDiscriminatorType::Value
                        | ExtractedDiscriminatorType::Pattern
                        | ExtractedDiscriminatorType::Type
                        | ExtractedDiscriminatorType::Exists
                        | ExtractedDiscriminatorType::Position
                        | ExtractedDiscriminatorType::Profile
                )
            })
            .collect();

        if !unsupported_discriminators.is_empty() {
            issues.push(ValidationIssue {
                severity: Severity::Warning,
                code: "business-rule".to_string(),
                summary: Some(
                    "Slicing uses discriminator types not supported by this validator".to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::SlicingUnsupportedDiscriminator),
                diagnostics: format!(
                    "Slicing on '{}' uses unsupported discriminator type(s) in v1; only value/pattern/type/exists/position/profile discriminators are currently supported, so slicing validation was skipped.",
                    base_rule.path
                ),
                expression: None,
                fhir_path: base_rule.path.clone(),
                instance_path: Some(base_rule.path.clone()),
            });
            continue;
        }

        if slicing.discriminators.iter().any(|d| {
            d.discriminator_type == ExtractedDiscriminatorType::Profile
                && d.path.contains("resolve()")
        }) {
            issues.push(ValidationIssue {
                severity: Severity::Warning,
                code: "business-rule".to_string(),
                summary: Some(
                    "Profile discriminator with resolve() is not implemented".to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::InteractionNotSupported),
                diagnostics: format!(
                    "Slicing on '{}' uses a profile discriminator path containing '.resolve()', but profile discriminator resolution is not yet implemented.",
                    base_rule.path
                ),
                expression: None,
                fhir_path: base_rule.path.clone(),
                instance_path: Some(base_rule.path.clone()),
            });
            continue;
        }

        if slicing.rules == ExtractedSlicingRules::OpenAtEnd && !slicing.ordered {
            issues.push(ValidationIssue {
                severity: Severity::Warning,
                code: "business-rule".to_string(),
                summary: Some("openAtEnd slicing requires ordered=true".to_string()),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::BusinessRuleViolation),
                diagnostics: format!(
                    "Slicing on '{}' uses openAtEnd but is not ordered. Per FHIR semantics, openAtEnd requires ordered slicing.",
                    base_rule.path
                ),
                expression: None,
                fhir_path: base_rule.path.clone(),
                instance_path: Some(base_rule.path.clone()),
            });
        }

        let slices: Vec<&ExtractedElementRule> = profile
            .element_rules
            .iter()
            .filter(|rule| rule.path == base_rule.path && rule.slice_name.is_some())
            .collect();

        if slices.is_empty() {
            continue;
        }

        let slice_order: HashMap<String, usize> = slices
            .iter()
            .enumerate()
            .filter_map(|(idx, slice)| slice.slice_name.as_ref().map(|name| (name.clone(), idx)))
            .collect();

        let Some(relative_path) = relative_profile_path(resource_type, &base_rule.path) else {
            continue;
        };

        let actual_values =
            get_values_with_paths_at_relative_path(&root, resource_type, relative_path);
        if actual_values.is_empty() {
            continue;
        }

        let mut slice_counts = HashMap::new();
        let mut seen_open_at_end_tail = false;
        let mut last_matched_slice_order: Option<usize> = None;

        for (item_index, (actual, actual_path)) in actual_values.into_iter().enumerate() {
            let matching_slice_names: Vec<String> = slices
                .iter()
                .filter(|slice| {
                    let mut eval = SliceEvaluationCtx {
                        actual,
                        item_index,
                        slice,
                        profile,
                        slice_order: &slice_order,
                        validation_ctx: ctx,
                        state,
                    };
                    matches_slice(&mut eval, &slicing.discriminators)
                })
                .filter_map(|slice| slice.slice_name.clone())
                .collect();

            if matching_slice_names.len() > 1 {
                issues.push(ValidationIssue {
                    severity: Severity::Error,
                    code: issue_code::STRUCTURE.to_string(),
                    summary: Some("Repeated element matches more than one slice".to_string()),
                    expression_kind: None,
                    source_invariant_key: None,
                    detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
                    diagnostics: format!(
                        "Element '{}' matches multiple declared slices on '{}': {}.",
                        actual_path,
                        base_rule.path,
                        matching_slice_names.join(", ")
                    ),
                    expression: None,
                    fhir_path: actual_path.clone(),
                    instance_path: Some(actual_path),
                });
                continue;
            }

            if let Some(slice_name) = matching_slice_names.first() {
                let current_slice_order = slice_order.get(slice_name).copied();

                if slicing.ordered {
                    if let (Some(last_order), Some(current_order)) =
                        (last_matched_slice_order, current_slice_order)
                    {
                        if current_order < last_order {
                            issues.push(ValidationIssue {
                                severity: Severity::Error,
                                code: issue_code::STRUCTURE.to_string(),
                                summary: Some(
                                    "Slice instances are not in declared order (ordered slicing)"
                                        .to_string(),
                                ),
                                expression_kind: None,
                                source_invariant_key: None,
                                detail_code: Some(ValidationIssueDetailCode::SliceOrderViolation),
                                diagnostics: format!(
                                    "Element '{}' matches declared slice '{}' on '{}', but ordered slicing requires slices to appear in declaration order.",
                                    actual_path, slice_name, base_rule.path
                                ),
                                expression: None,
                                fhir_path: actual_path.clone(),
                                instance_path: Some(actual_path),
                            });
                            continue;
                        }
                    }
                }

                if slicing.rules == ExtractedSlicingRules::OpenAtEnd && seen_open_at_end_tail {
                    issues.push(ValidationIssue {
                        severity: Severity::Error,
                        code: issue_code::STRUCTURE.to_string(),
                        summary: Some(
                            "Named slice matched after tail content (openAtEnd violation)".to_string(),
                        ),
                        expression_kind: None,
                        source_invariant_key: None,
                        detail_code: Some(ValidationIssueDetailCode::SliceOpenAtEndViolation),
                        diagnostics: format!(
                            "Element '{}' matches declared slice '{}' on '{}', but openAtEnd requires all unmatched content to appear only after the declared slices.",
                            actual_path, slice_name, base_rule.path
                        ),
                        expression: None,
                        fhir_path: actual_path.clone(),
                        instance_path: Some(actual_path),
                    });
                    continue;
                }

                if let Some(current_order) = current_slice_order {
                    last_matched_slice_order = Some(current_order);
                }

                *slice_counts.entry(slice_name.clone()).or_insert(0) += 1;
                continue;
            }

            match slicing.rules {
                ExtractedSlicingRules::Closed => {
                    issues.push(ValidationIssue {
                        severity: Severity::Error,
                        code: issue_code::STRUCTURE.to_string(),
                        summary: Some("Element does not match any slice (closed slicing)".to_string()),
                        expression_kind: None,
                        source_invariant_key: None,
                        detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
                        diagnostics: format!(
                            "Element '{}' does not match any declared slice on '{}', and slicing rules are closed.",
                            actual_path, base_rule.path
                        ),
                        expression: None,
                        fhir_path: actual_path.clone(),
                        instance_path: Some(actual_path),
                    });
                }
                ExtractedSlicingRules::Open => {}
                ExtractedSlicingRules::OpenAtEnd => {
                    seen_open_at_end_tail = true;
                }
            }
        }
        for slice in &slices {
            if let Some(slice_name) = &slice.slice_name {
                let count = slice_counts.get(slice_name).cloned().unwrap_or(0);

                if let Some(min) = slice.min {
                    if count < min as usize {
                        issues.push(ValidationIssue {
                            severity: Severity::Error,
                            code: "required".to_string(),
                            summary: Some("Slice does not meet minimum cardinality".to_string()),
                            expression_kind: None,
                            source_invariant_key: None,
                            detail_code: Some(
                                ValidationIssueDetailCode::SliceMinCardinalityMissing,
                            ),
                            diagnostics: format!(
                                "Slice '{}:{}' requires at least {} occurrence(s), but found {}.",
                                base_rule.path, slice_name, min, count
                            ),
                            expression: None,
                            fhir_path: format!("{}:{}", base_rule.path, slice_name),
                            instance_path: Some(base_rule.path.clone()),
                        });
                    }
                }

                if let Some(max) = &slice.max {
                    if let Some(max_value) = parse_slice_max(max) {
                        if count > max_value {
                            issues.push(ValidationIssue {
                                severity: Severity::Error,
                                code: issue_code::STRUCTURE.to_string(),
                                summary: Some("Slice exceeds maximum cardinality".to_string()),
                                expression_kind: None,
                                source_invariant_key: None,
                                detail_code: Some(
                                    ValidationIssueDetailCode::SliceMaxCardinalityExceeded,
                                ),
                                diagnostics: format!(
                                    "Slice '{}:{}' allows at most {} occurrence(s), but found {}.",
                                    base_rule.path, slice_name, max_value, count
                                ),
                                expression: None,
                                fhir_path: format!("{}:{}", base_rule.path, slice_name),
                                instance_path: Some(base_rule.path.clone()),
                            });
                        }
                    }
                }
            }
        }
    }

    issues
}

/// Shared inputs for evaluating whether a repeated instance item belongs to a slice.
///
/// Discriminators are passed separately to [`matches_slice`] so callers can iterate them
/// while holding mutable access to [`ValidationState`] (the slice rules and the state cannot
/// live in the same borrowed struct without splitting borrows).
struct SliceEvaluationCtx<'a, 'v> {
    actual: &'a Value,
    item_index: usize,
    slice: &'a ExtractedElementRule,
    profile: &'a ExtractedProfile,
    slice_order: &'a HashMap<String, usize>,
    validation_ctx: Option<&'v ValidationContext<'v>>,
    state: &'a mut ValidationState,
}

/// Return `true` if the current repeated item satisfies all discriminator rules
/// required for the given slice.
///
/// FHIR slicing uses AND semantics when multiple discriminators are declared,
/// so every discriminator must match for the item to belong to the slice.
fn matches_slice(
    eval: &mut SliceEvaluationCtx<'_, '_>,
    discriminators: &[ExtractedSliceDiscriminator],
) -> bool {
    discriminators
        .iter()
        .all(|discriminator| matches_discriminator(eval, discriminator))
}

/// Dispatch discriminator evaluation by discriminator kind.
///
/// Supported discriminator kinds are currently limited to:
/// - `value`
/// - `type`
/// - `exists`
/// - `position`
/// - `profile` (v1, without `.resolve()`)
///
/// `profile` support currently matches only against declared `meta.profile`
/// values on the nominated runtime element. When `.resolve()` is used in the
/// discriminator path, the resolved reference target would need profile
/// conformance checking, which is not yet implemented.
fn matches_discriminator(
    eval: &mut SliceEvaluationCtx<'_, '_>,
    discriminator: &ExtractedSliceDiscriminator,
) -> bool {
    match discriminator.discriminator_type {
        ExtractedDiscriminatorType::Value | ExtractedDiscriminatorType::Pattern => {
            matches_value_discriminator(eval.actual, eval.slice, discriminator, eval.profile)
        }
        ExtractedDiscriminatorType::Type => {
            matches_type_discriminator(eval.actual, eval.slice, discriminator, eval.profile)
        }
        ExtractedDiscriminatorType::Exists => {
            matches_exists_discriminator(eval.actual, eval.slice, discriminator, eval.profile)
        }
        ExtractedDiscriminatorType::Position => {
            matches_position_discriminator(eval.item_index, eval.slice, eval.slice_order)
        }
        ExtractedDiscriminatorType::Profile => matches_profile_discriminator(
            eval.actual,
            eval.slice,
            discriminator,
            eval.profile,
            eval.validation_ctx,
            eval.state,
        ),
    }
}

/// Evaluate a `profile` discriminator by comparing expected profile URLs from
/// the slice rules with declared `meta.profile` URLs on the nominated runtime
/// value.
///
/// Current scope:
/// - supports discriminator paths without `.resolve()`
/// - first matches against declared `meta.profile`
/// - if no declared `meta.profile` is present, may fall back to recursive
///   validation against registry-known extracted profiles
fn matches_profile_discriminator(
    actual: &Value,
    slice: &ExtractedElementRule,
    discriminator: &crate::profile::types::ExtractedSliceDiscriminator,
    profile: &ExtractedProfile,
    ctx: Option<&ValidationContext<'_>>,
    state: &mut ValidationState,
) -> bool {
    let expected_profile_urls = slice_discriminator_profile_urls(profile, slice, discriminator);
    if expected_profile_urls.is_empty() {
        return false;
    }

    let actual_values = actual_discriminator_values(actual, discriminator);
    if actual_values.is_empty() {
        return false;
    }

    actual_values.into_iter().any(|candidate| {
        let declared_profiles = declared_profiles_on_value(candidate);
        if !declared_profiles.is_empty() {
            return expected_profile_urls
                .iter()
                .any(|expected| declared_profiles.iter().any(|actual| actual == expected));
        }

        let Some(ctx) = ctx else {
            return false;
        };

        expected_profile_urls.iter().any(|expected| {
            let Some(expected_profile) = ctx.extracted_profile_map.get(expected) else {
                return false;
            };

            matches_profile_by_recursive_validation(candidate, expected_profile, ctx, state)
        })
    })
}

/// Extract declared `meta.profile` URLs from a nominated runtime JSON value.
fn declared_profiles_on_value(value: &Value) -> Vec<String> {
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };

    let Some(meta) = obj.get("meta").and_then(Value::as_object) else {
        return Vec::new();
    };

    let Some(profiles) = meta.get("profile").and_then(Value::as_array) else {
        return Vec::new();
    };

    profiles
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// Resolve the expected profile URLs for a slice `profile` discriminator.
///
/// Profile constraints may be declared either:
/// - directly on the slice root, or
/// - on a slice-specific child rule for the discriminator path
///   (for example `Parameters.parameter:obs.resource`).
fn slice_discriminator_profile_urls(
    profile: &ExtractedProfile,
    slice: &ExtractedElementRule,
    discriminator: &crate::profile::types::ExtractedSliceDiscriminator,
) -> Vec<String> {
    let discriminator_rule_path = format!("{}.{}", slice.path, discriminator.path);
    let discriminator_rule_id = format!("{}.{}", slice.id, discriminator.path);

    if let Some(slice_name) = slice.slice_name.as_deref() {
        if let Some(rule) = profile.element_rules.iter().find(|rule| {
            rule.path == discriminator_rule_path
                && rule.slice_name.as_deref() == Some(slice_name)
                && rule
                    .type_constraints
                    .iter()
                    .any(|constraint| !constraint.profiles.is_empty())
        }) {
            return type_constraint_profile_urls(&rule.type_constraints);
        }
    }

    if let Some(rule) = profile.element_rules.iter().find(|rule| {
        rule.id == discriminator_rule_id
            && rule
                .type_constraints
                .iter()
                .any(|constraint| !constraint.profiles.is_empty())
    }) {
        return type_constraint_profile_urls(&rule.type_constraints);
    }

    type_constraint_profile_urls(&slice.type_constraints)
}

/// Flatten all declared `type.profile` URLs from a slice's extracted type
/// constraints.
fn type_constraint_profile_urls(type_constraints: &[ExtractedTypeConstraint]) -> Vec<String> {
    type_constraints
        .iter()
        .flat_map(|constraint| constraint.profiles.iter().cloned())
        .collect()
}

fn matches_profile_by_recursive_validation(
    candidate: &Value,
    expected_profile: &ExtractedProfile,
    ctx: &ValidationContext<'_>,
    state: &mut ValidationState,
) -> bool {
    let Some(resource_type) = candidate
        .as_object()
        .and_then(|obj| obj.get("resourceType"))
        .and_then(Value::as_str)
    else {
        return false;
    };

    let issues =
        validate_nested_profile_candidate(candidate, resource_type, expected_profile, ctx, state);

    !issues.iter().any(|issue| issue.severity == Severity::Error)
}

fn validate_nested_profile_candidate(
    candidate: &Value,
    resource_type: &str,
    expected_profile: &ExtractedProfile,
    ctx: &ValidationContext<'_>,
    state: &mut ValidationState,
) -> Vec<ValidationIssue> {
    let mut child_state = ValidationState {
        recursion_depth: state.recursion_depth + 1,
        active_profiles: state.active_profiles.clone(),
    };

    validate_profile_with_depth(
        ctx,
        &mut child_state,
        candidate,
        resource_type,
        expected_profile,
    )
}

/// Evaluate a `value` discriminator by comparing the actual discriminator values
/// with the extracted fixed/pattern constraint for the slice.
///
/// The expected value constraint may live either on the slice root element or on
/// a slice-specific child rule such as `component:systolic.code`.
fn matches_value_discriminator(
    actual: &Value,
    slice: &ExtractedElementRule,
    discriminator: &crate::profile::types::ExtractedSliceDiscriminator,
    profile: &ExtractedProfile,
) -> bool {
    let Some(value_constraint) =
        slice_discriminator_value_constraint(profile, slice, discriminator)
    else {
        return false;
    };

    let actual_values = actual_discriminator_values(actual, discriminator);
    if actual_values.is_empty() {
        return false;
    }

    actual_values
        .into_iter()
        .any(|candidate| matches_value_constraint(candidate, value_constraint))
}

/// Evaluate a `type` discriminator by comparing inferred runtime JSON/FHIR type
/// codes with the extracted type constraints for the slice.
///
/// This implementation supports common FHIR JSON shapes including:
/// - embedded resources with `resourceType`
/// - `Reference`-shaped objects
/// - `CodeableReference`-shaped objects
/// - choice elements such as `value[x]` resolved to concrete JSON keys
fn matches_type_discriminator(
    actual: &Value,
    slice: &ExtractedElementRule,
    discriminator: &crate::profile::types::ExtractedSliceDiscriminator,
    profile: &ExtractedProfile,
) -> bool {
    let expected_type_codes = slice_discriminator_type_codes(profile, slice, discriminator);
    if expected_type_codes.is_empty() {
        return false;
    }

    let actual_values = actual_discriminator_values(actual, discriminator);
    if actual_values.is_empty() {
        return false;
    }

    actual_values.into_iter().any(|candidate| {
        let actual_type_codes = json_type_codes(candidate);
        if actual_type_codes.is_empty() {
            return false;
        }

        expected_type_codes
            .iter()
            .any(|expected| actual_type_codes.iter().any(|actual| actual == expected))
    })
}

/// Evaluate a `position` discriminator.
///
/// The current implementation interprets expected slice position using the slice
/// declaration order within the extracted profile. The item matches only when its
/// zero-based index equals the declaration index of the slice name.
fn matches_position_discriminator(
    item_index: usize,
    slice: &ExtractedElementRule,
    slice_order: &HashMap<String, usize>,
) -> bool {
    let Some(slice_name) = slice.slice_name.as_ref() else {
        return false;
    };

    let Some(expected_index) = slice_order.get(slice_name).copied() else {
        return false;
    };

    item_index == expected_index
}

/// Evaluate an `exists` discriminator by comparing actual presence of the
/// discriminator path with the expected presence inferred from the slice rules.
fn matches_exists_discriminator(
    actual: &Value,
    slice: &ExtractedElementRule,
    discriminator: &crate::profile::types::ExtractedSliceDiscriminator,
    profile: &ExtractedProfile,
) -> bool {
    let Some(expected_exists) =
        slice_discriminator_exists_expectation(profile, slice, discriminator)
    else {
        return false;
    };

    let actual_values = actual_discriminator_values(actual, discriminator);
    let actual_exists = !actual_values.is_empty();

    actual_exists == expected_exists
}

/// Infer the expected boolean presence for an `exists` discriminator.
///
/// Convention used here:
/// - `min > 0` on the slice discriminator child rule means the path must exist
/// - `max = 0` on the slice discriminator child rule means the path must not exist
///
/// This is a practical extracted-rule interpretation for slicing validation.
fn slice_discriminator_exists_expectation(
    profile: &ExtractedProfile,
    slice: &ExtractedElementRule,
    discriminator: &crate::profile::types::ExtractedSliceDiscriminator,
) -> Option<bool> {
    let discriminator_rule_path = format!("{}.{}", slice.path, discriminator.path);
    let discriminator_rule_id = format!("{}.{}", slice.id, discriminator.path);

    if let Some(slice_name) = slice.slice_name.as_deref() {
        if let Some(rule) = profile.element_rules.iter().find(|rule| {
            rule.path == discriminator_rule_path && rule.slice_name.as_deref() == Some(slice_name)
        }) {
            if let Some(min) = rule.min {
                return Some(min > 0);
            }
            if let Some(max) = &rule.max {
                if max == "0" {
                    return Some(false);
                }
            }
        }
    }

    if let Some(rule) = profile
        .element_rules
        .iter()
        .find(|rule| rule.id == discriminator_rule_id)
    {
        if let Some(min) = rule.min {
            return Some(min > 0);
        }
        if let Some(max) = &rule.max {
            if max == "0" {
                return Some(false);
            }
        }
    }

    if let Some(min) = slice.min {
        return Some(min > 0);
    }
    if let Some(max) = &slice.max {
        if max == "0" {
            return Some(false);
        }
    }

    None
}

/// Resolve the runtime values addressed by a discriminator path relative to the
/// current repeated item.
///
/// Choice paths such as `value[x]` are resolved by the shared profile helper
/// path traversal utilities.
fn actual_discriminator_values<'a>(
    actual: &'a Value,
    discriminator: &crate::profile::types::ExtractedSliceDiscriminator,
) -> Vec<&'a Value> {
    get_values_at_relative_path(actual, &discriminator.path)
}

/// Resolve the expected type codes for a slice `type` discriminator.
///
/// Type constraints may be declared either:
/// - directly on the slice root, or
/// - on a slice-specific child rule for the discriminator path
///   (for example `Parameters.parameter:obs.resource`).
fn slice_discriminator_type_codes(
    profile: &ExtractedProfile,
    slice: &ExtractedElementRule,
    discriminator: &crate::profile::types::ExtractedSliceDiscriminator,
) -> Vec<String> {
    let discriminator_rule_path = format!("{}.{}", slice.path, discriminator.path);
    let discriminator_rule_id = format!("{}.{}", slice.id, discriminator.path);

    if let Some(slice_name) = slice.slice_name.as_deref() {
        if let Some(rule) = profile.element_rules.iter().find(|rule| {
            rule.path == discriminator_rule_path
                && rule.slice_name.as_deref() == Some(slice_name)
                && !rule.type_constraints.is_empty()
        }) {
            return rule
                .type_constraints
                .iter()
                .map(|constraint| constraint.code.clone())
                .collect();
        }
    }

    if let Some(rule) = profile
        .element_rules
        .iter()
        .find(|rule| rule.id == discriminator_rule_id && !rule.type_constraints.is_empty())
    {
        return rule
            .type_constraints
            .iter()
            .map(|constraint| constraint.code.clone())
            .collect();
    }

    slice
        .type_constraints
        .iter()
        .map(|constraint| constraint.code.clone())
        .collect()
}

/// Resolve the expected value constraint for a slice `value` discriminator.
///
/// Value constraints may be declared either:
/// - directly on the slice root, or
/// - on a slice-specific child rule for the discriminator path
///   (for example `Observation.component:systolic.code`).
fn slice_discriminator_value_constraint<'a>(
    profile: &'a ExtractedProfile,
    slice: &'a ExtractedElementRule,
    discriminator: &crate::profile::types::ExtractedSliceDiscriminator,
) -> Option<&'a ExtractedValueConstraint> {
    let discriminator_rule_path = format!("{}.{}", slice.path, discriminator.path);
    let discriminator_rule_id = format!("{}.{}", slice.id, discriminator.path);

    if let Some(slice_name) = slice.slice_name.as_deref() {
        if let Some(rule) = profile.element_rules.iter().find(|rule| {
            rule.path == discriminator_rule_path
                && rule.slice_name.as_deref() == Some(slice_name)
                && rule.value_constraint.is_some()
        }) {
            return rule.value_constraint.as_ref();
        }
    }

    if let Some(rule) = profile
        .element_rules
        .iter()
        .find(|rule| rule.id == discriminator_rule_id && rule.value_constraint.is_some())
    {
        return rule.value_constraint.as_ref();
    }

    slice.value_constraint.as_ref()
}

fn matches_value_constraint(actual: &Value, constraint: &ExtractedValueConstraint) -> bool {
    match constraint {
        ExtractedValueConstraint::Fixed(expected) => actual == expected,
        ExtractedValueConstraint::Pattern(expected) => pattern_matches(expected, actual),
    }
}

fn pattern_matches(pattern: &Value, actual: &Value) -> bool {
    match (pattern, actual) {
        (Value::Object(pattern_map), Value::Object(actual_map)) => {
            pattern_map.iter().all(|(key, pattern_value)| {
                actual_map
                    .get(key)
                    .map(|actual_value| pattern_matches(pattern_value, actual_value))
                    .unwrap_or(false)
            })
        }
        (Value::Array(pattern_items), Value::Array(actual_items)) => {
            pattern_items.len() <= actual_items.len()
                && pattern_items.iter().zip(actual_items.iter()).all(
                    |(pattern_value, actual_value)| pattern_matches(pattern_value, actual_value),
                )
        }
        _ => pattern == actual,
    }
}

/// Convert a full extracted profile path such as `Observation.component.code`
/// into a path relative to the current resource root such as `component.code`.
fn relative_profile_path<'a>(resource_type: &str, full_path: &'a str) -> Option<&'a str> {
    full_path
        .strip_prefix(resource_type)
        .and_then(|rest| rest.strip_prefix('.'))
        .or_else(|| (full_path == resource_type).then_some(""))
}
