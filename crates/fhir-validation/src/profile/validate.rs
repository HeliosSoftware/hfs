//! Profile validation: [`ExtractedProfile`] instances against resource JSON (sync and async).
//!
//! # Element `constraint` evaluation (FHIRPath)
//!
//! [`StructureDefinition`](https://hl7.org/fhir/structuredefinition.html) constraints are
//! evaluated so that, where applicable, FHIRPath runs in the **same context as the
//! specification** (the value at the element’s path).
//!
//! ## [`ExtractedProfile::invariants`](crate::profile::types::ExtractedProfile::invariants) (profile / root row only)
//!
//! The extractor only fills this from differential elements whose `path` equals the resource
//! type (e.g. `Patient`, `Organization`). These are **resource-level** rules.
//!
//! They are evaluated with [`crate::Validator::apply_invariants`], which serializes the full resource
//! once and uses bulk [`FhirPathEvaluator::eval_invariants_on`]. That matches expressions written
//! for the resource root (e.g. “at least one name or identifier”, or
//! `active = true implies name.exists()`).
//!
//! ## [`ExtractedElementRule::constraints`](crate::profile::types::ExtractedElementRule::constraints) (per-element rules)
//!
//! For a rule whose [`ExtractedElementRule::path`](crate::profile::types::ExtractedElementRule::path) is **deeper than the resource type** (e.g.
//! `Patient.identifier`, `CapabilityStatement.rest.resource`), each
//! [`InvariantDef::expression`] is evaluated via [`FhirPathEvaluator::eval_invariant`], which
//! resolves the declared path to the correct focus (including each repeated node) and sets
//! `$this` accordingly. This supports **element-relative** FHIRPath (e.g. cpb-12’s
//! `searchParam.select(name).isDistinct()`, or `value.exists()` on an identifier slice).
//!
//! If a rule path is classified as **root-only** (resource type and nothing else), we still use
//! [`crate::Validator::apply_invariants`] for backward compatibility with expressions that assume the
//! whole resource as focus. See `apply_profile_element_rule_invariants` and
//! [`crate::profile::cardinality::is_root_profile_element_path`].
//!
//! **Note:** Generated resource validators (outside this module) pass a **nested** `self` into
//! [`crate::Validator::apply_invariants`]; that is unchanged and remains correct for datatype-local
//! rules such as `ele-1` on nested structures.
//!
//! # Full profile pipeline (`validate_profile`)
//!
//! The synchronous [`validate_profile`] / async [`validate_profile_async`] entry points delegate to
//! the internal `validate_profile_with_depth` (and `_async`), which applies **extracted** constraints in a
//! fixed order:
//!
//! 1. Profile-level invariants ([`ExtractedProfile::invariants`](crate::profile::types::ExtractedProfile::invariants)) — always evaluated at the
//!    resource root.
//! 2. Element invariants ([`ExtractedElementRule::constraints`](crate::profile::types::ExtractedElementRule::constraints)) — root vs nested focus per
//!    `apply_profile_element_rule_invariants`.
//! 3. Minimum cardinality, then mustSupport (if enabled), then maximum cardinality.
//! 4. [`crate::profile::slicing::validate_slicing`] — slice membership and per-slice cardinality.
//! 5. Fixed and pattern value constraints.
//! 6. `maxLength` and `minValue` / `maxValue` via [`crate::profile::element_bounds`].
//! 7. Narrowed choice types (`type.code` constraints on `[x]` elements).
//! 8. Reference and canonical metadata (`aggregation`, `versioning`, target profiles).
//! 9. `ElementDefinition.type.profile` — declared profile matching and optional **recursive**
//!    profile validation (guarded by [`crate::ValidationConfig::max_profile_recursion_depth`]
//!    and active-profile cycle detection in [`ValidationState`]).
//! 10. Terminology bindings (`ValueSet`/`CodeSystem`).
//!
//! When the high-level API merges **base structural** validation with **profile** validation,
//! duplicate issues (same severity, code, paths, diagnostics) may be removed by `dedupe_exact_issues`
//! to avoid noisy duplicates (e.g. shared invariants).
//!
//! # Declared profiles (`meta.profile`)
//!
//! [`validate_declared_profiles`] and [`validate_declared_profiles_async`] read `meta.profile`
//! canonical URLs, resolve each through [`ValidationContext::runtime_profile_registry`](crate::validation_context::ValidationContext::runtime_profile_registry)
//! / [`AsyncValidationContext::runtime_profile_registry`](crate::validation_context::AsyncValidationContext::runtime_profile_registry), and run [`validate_profile`] per URL.
//! Missing registry entries produce `not-found` issues at `Resource.meta.profile`.
//!
//! # Base definition lookup
//!
//! For profiles with `StructureDefinition.baseDefinition`, optional HTTP fetch (when enabled and
//! allowlisted) uses [`crate::profile::base_definition_fetch_url::structure_definition_json_fetch_url`]
//! to turn canonical pages into static JSON URLs, with in-memory caching.
use crate::issue_code;
use crate::profile::base_definition_fetch_url::structure_definition_json_fetch_url;
use crate::profile::cardinality::{
    is_root_profile_element_path, relative_profile_path, validate_max_cardinality,
    validate_min_cardinality, validate_must_support,
};
use crate::profile::element_bounds::validate_element_bounds;
use crate::profile::extract::extract_structure_definition_profile_from_json;
use crate::profile::helpers::{
    get_values_at_relative_path, get_values_with_paths_at_relative_path,
};
use crate::profile::profile_registry::ProfileRegistry;
use crate::profile::slicing::validate_slicing;
use crate::profile::types::{ExtractedProfile, ExtractedTypeConstraint, ExtractedValueConstraint};
use crate::validation_context::AsyncValidationContext;
pub use crate::validation_context::{ValidationContext, ValidationState};
use crate::validation_issue_detail::ValidationIssueDetailCode;
use crate::{FhirPathEvaluator, InvariantDef, TypeProfileMatchMode, ValidationIssue};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

static REMOTE_BASE_PROFILE_CACHE: OnceLock<
    Mutex<std::collections::HashMap<String, Option<ExtractedProfile>>>,
> = OnceLock::new();

fn dedupe_exact_issues(issues: Vec<ValidationIssue>) -> Vec<ValidationIssue> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(issues.len());
    for issue in issues {
        let k = format!(
            "{:?}|{}|{}|{:?}|{:?}|{:?}|{}|{}",
            issue.severity,
            issue.code,
            issue.fhir_path,
            issue.instance_path,
            issue.expression,
            issue.source_invariant_key,
            issue.summary.clone().unwrap_or_default(),
            issue.diagnostics
        );
        if seen.insert(k) {
            out.push(issue);
        }
    }
    out
}

/// Validate a resource instance against a single extracted profile.
///
/// This is the public entry point for profile-based validation. It initializes
/// recursion-cycle tracking and then delegates to the internal `validate_profile_with_depth`
/// for the actual validation pipeline.
///
/// **Invariants:** Profile-level and element-level FHIRPath constraints are applied as
/// described in the [`crate::profile::validate`] module documentation.
pub fn validate_profile<T: Serialize>(
    ctx: &ValidationContext<'_>,
    state: &mut ValidationState,
    resource: &T,
    resource_type: &str,
    profile: &ExtractedProfile,
) -> Vec<ValidationIssue> {
    validate_profile_with_depth(ctx, state, resource, resource_type, profile)
}

pub async fn validate_profile_async<T: Serialize>(
    ctx: &AsyncValidationContext<'_>,
    state: &mut ValidationState,
    resource: &T,
    resource_type: &str,
    profile: &ExtractedProfile,
) -> Vec<ValidationIssue> {
    validate_profile_with_depth_async(ctx, state, resource, resource_type, profile).await
}

/// Applies `ElementDefinition.constraint` entries from a single [`ExtractedElementRule`].
///
/// - **Root path** ([`is_root_profile_element_path`]): uses [`Validator::apply_invariants`] with
///   the full serialized `resource` and bulk [`FhirPathEvaluator::eval_invariants_on`].
/// - **Non-root path**: one [`FhirPathEvaluator::eval_invariant`] call per [`InvariantDef`], so
///   the declared [`InvariantDef::path`] drives focus resolution and each expression sees the
///   correct FHIRPath `$this` for that element.
fn apply_profile_element_rule_invariants<T: Serialize>(
    validator: &crate::Validator,
    resource: &T,
    evaluator: &dyn FhirPathEvaluator,
    resource_type: &str,
    rule_path: &str,
    constraints: &[InvariantDef],
) -> Vec<ValidationIssue> {
    if constraints.is_empty() {
        return Vec::new();
    }

    if is_root_profile_element_path(resource_type, rule_path) {
        return validator.apply_invariants(resource, constraints, evaluator, resource_type);
    }

    let mut issues = Vec::new();
    for inv in constraints {
        match evaluator.eval_invariant(inv.path.as_str(), inv.expression.as_str()) {
            Ok(true) => {}
            Ok(false) => {
                issues.push(ValidationIssue::from_invariant_def(inv).with_instance_path(rule_path));
            }
            Err(e) => {
                issues.push(
                    ValidationIssue::from_invariant_error(inv, e).with_instance_path(rule_path),
                );
            }
        }
    }
    issues
}

/// Internal profile validation pipeline with recursion-depth and cycle tracking.
///
/// The pipeline currently performs, in order:
/// - profile-level invariants ([`ExtractedProfile::invariants`], always root path — see module docs)
/// - element-level invariants ([`apply_profile_element_rule_invariants`] per rule with constraints)
/// - minimum cardinality
/// - mustSupport (when enabled)
/// - maximum cardinality
/// - slicing validation
/// - fixed/pattern value constraints
/// - `maxLength` and `minValue` / `maxValue` bounds (when extracted on the rule)
/// - narrowed choice-type constraints
/// - target profile constraints on references
/// - `ElementDefinition.type.aggregation` and `type.versioning` on references and
///   canonicals (when extracted)
/// - `type.profile` constraints, including optional recursive profile validation
/// - terminology bindings
///
/// Recursive profile validation is guarded by both a maximum recursion depth and
/// an active-profile set to prevent infinite cycles.
pub(crate) fn validate_profile_with_depth_async<'a, T: Serialize + 'a>(
    ctx: &'a AsyncValidationContext<'a>,
    state: &'a mut ValidationState,
    resource: &'a T,
    resource_type: &'a str,
    profile: &'a ExtractedProfile,
) -> Pin<Box<dyn Future<Output = Vec<ValidationIssue>> + 'a>> {
    Box::pin(async move {
        if state.recursion_depth >= ctx.validator.config.max_profile_recursion_depth {
            if ctx.validator.config.warn_on_profile_recursion_depth_reached {
                return vec![ValidationIssue {
                    severity: crate::Severity::Warning,
                    code: "business-rule".to_string(),
                    summary: Some(
                        "Recursive profile validation stopped: maximum depth reached".to_string(),
                    ),
                    expression_kind: None,
                    source_invariant_key: None,
                    detail_code: Some(ValidationIssueDetailCode::RecursionDepthReached),
                    diagnostics: format!(
                        "Skipping recursive profile validation for '{}' because the maximum recursion depth {} was reached.",
                        profile.url, ctx.validator.config.max_profile_recursion_depth
                    ),
                    expression: Some(profile.url.clone()),
                    fhir_path: resource_type.to_string(),
                    instance_path: Some(resource_type.to_string()),
                }];
            }
            return Vec::new();
        }

        if !state.active_profiles.insert(profile.url.clone()) {
            if ctx.validator.config.warn_on_profile_cycle {
                return vec![ValidationIssue {
                    severity: crate::Severity::Warning,
                    code: "business-rule".to_string(),
                    summary: Some(
                        "Recursive profile validation skipped: profile cycle detected".to_string(),
                    ),
                    expression_kind: None,
                    source_invariant_key: None,
                    detail_code: Some(ValidationIssueDetailCode::ProfileCycleDetected),
                    diagnostics: format!(
                        "Skipping recursive profile validation for '{}' because a validation cycle was detected.",
                        profile.url
                    ),
                    expression: Some(profile.url.clone()),
                    fhir_path: resource_type.to_string(),
                    instance_path: Some(resource_type.to_string()),
                }];
            }
            return Vec::new();
        }

        let mut issues = Vec::new();

        issues.extend(ctx.validator.apply_invariants(
            resource,
            profile.invariants.as_slice(),
            ctx.evaluator,
            resource_type,
        ));

        for rule in &profile.element_rules {
            if !rule.constraints.is_empty() {
                issues.extend(apply_profile_element_rule_invariants(
                    ctx.validator,
                    resource,
                    ctx.evaluator,
                    resource_type,
                    rule.path.as_str(),
                    rule.constraints.as_slice(),
                ));
            }
        }

        issues.extend(validate_min_cardinality(
            resource,
            resource_type,
            profile.element_rules.as_slice(),
        ));

        issues.extend(validate_must_support(
            resource,
            resource_type,
            profile.element_rules.as_slice(),
            &ctx.validator.config,
        ));

        issues.extend(validate_max_cardinality(
            resource,
            resource_type,
            profile.element_rules.as_slice(),
        ));

        issues.extend(validate_slicing(resource, resource_type, profile));

        issues.extend(validate_value_constraints(
            resource,
            resource_type,
            profile.element_rules.as_slice(),
        ));

        issues.extend(validate_element_bounds(
            resource,
            resource_type,
            profile.element_rules.as_slice(),
        ));

        issues.extend(validate_type_constraints(
            resource,
            resource_type,
            profile.element_rules.as_slice(),
        ));

        issues.extend(validate_target_profile_constraints(
            resource,
            resource_type,
            profile.element_rules.as_slice(),
        ));

        issues.extend(validate_reference_aggregate_versioning_constraints(
            resource,
            resource_type,
            profile.element_rules.as_slice(),
        ));

        issues.extend(
            validate_type_profile_constraints_async(
                ctx,
                state,
                resource,
                resource_type,
                profile.element_rules.as_slice(),
            )
            .await,
        );

        let bindings: Vec<_> = profile
            .element_rules
            .iter()
            .filter_map(|rule| rule.binding.clone())
            .collect();

        if !bindings.is_empty() {
            issues.extend(
                ctx.validator
                    .apply_bindings_for_version_async(
                        ctx.fhir_version,
                        resource,
                        bindings.as_slice(),
                        ctx.terminology,
                    )
                    .await,
            );
        }

        issues.extend(
            validate_base_definition_profile_async(ctx, state, resource, resource_type, profile)
                .await,
        );

        state.active_profiles.remove(&profile.url);
        dedupe_exact_issues(issues)
    })
}

pub(crate) fn validate_profile_with_depth<T: Serialize>(
    ctx: &ValidationContext<'_>,
    state: &mut ValidationState,
    resource: &T,
    resource_type: &str,
    profile: &ExtractedProfile,
) -> Vec<ValidationIssue> {
    if state.recursion_depth >= ctx.validator.config.max_profile_recursion_depth {
        if ctx.validator.config.warn_on_profile_recursion_depth_reached {
            return vec![ValidationIssue {
                severity: crate::Severity::Warning,
                code: "business-rule".to_string(),
                summary: Some(
                    "Recursive profile validation stopped: maximum depth reached".to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::RecursionDepthReached),
                diagnostics: format!(
                    "Skipping recursive profile validation for '{}' because the maximum recursion depth {} was reached.",
                    profile.url, ctx.validator.config.max_profile_recursion_depth
                ),
                expression: Some(profile.url.clone()),
                fhir_path: resource_type.to_string(),
                instance_path: Some(resource_type.to_string()),
            }];
        }
        return Vec::new();
    }

    if !state.active_profiles.insert(profile.url.clone()) {
        if ctx.validator.config.warn_on_profile_cycle {
            return vec![ValidationIssue {
                severity: crate::Severity::Warning,
                code: "business-rule".to_string(),
                summary: Some(
                    "Recursive profile validation skipped: profile cycle detected".to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::ProfileCycleDetected),
                diagnostics: format!(
                    "Skipping recursive profile validation for '{}' because a validation cycle was detected.",
                    profile.url
                ),
                expression: Some(profile.url.clone()),
                fhir_path: resource_type.to_string(),
                instance_path: Some(resource_type.to_string()),
            }];
        }
        return Vec::new();
    }

    let mut issues = Vec::new();

    issues.extend(ctx.validator.apply_invariants(
        resource,
        profile.invariants.as_slice(),
        ctx.evaluator,
        resource_type,
    ));

    for rule in &profile.element_rules {
        if !rule.constraints.is_empty() {
            issues.extend(apply_profile_element_rule_invariants(
                ctx.validator,
                resource,
                ctx.evaluator,
                resource_type,
                rule.path.as_str(),
                rule.constraints.as_slice(),
            ));
        }
    }

    issues.extend(validate_min_cardinality(
        resource,
        resource_type,
        profile.element_rules.as_slice(),
    ));

    issues.extend(validate_must_support(
        resource,
        resource_type,
        profile.element_rules.as_slice(),
        &ctx.validator.config,
    ));

    issues.extend(validate_max_cardinality(
        resource,
        resource_type,
        profile.element_rules.as_slice(),
    ));

    issues.extend(validate_slicing(resource, resource_type, profile));

    issues.extend(validate_value_constraints(
        resource,
        resource_type,
        profile.element_rules.as_slice(),
    ));

    issues.extend(validate_element_bounds(
        resource,
        resource_type,
        profile.element_rules.as_slice(),
    ));

    issues.extend(validate_type_constraints(
        resource,
        resource_type,
        profile.element_rules.as_slice(),
    ));

    issues.extend(validate_target_profile_constraints(
        resource,
        resource_type,
        profile.element_rules.as_slice(),
    ));

    issues.extend(validate_reference_aggregate_versioning_constraints(
        resource,
        resource_type,
        profile.element_rules.as_slice(),
    ));

    issues.extend(validate_type_profile_constraints(
        ctx,
        state,
        resource,
        resource_type,
        profile.element_rules.as_slice(),
    ));

    let bindings: Vec<_> = profile
        .element_rules
        .iter()
        .filter_map(|rule| rule.binding.clone())
        .collect();

    if !bindings.is_empty() {
        issues.extend(ctx.validator.apply_bindings_for_version_sync(
            ctx.fhir_version,
            resource,
            bindings.as_slice(),
            ctx.terminology,
        ));
    }

    issues.extend(validate_base_definition_profile(
        ctx,
        state,
        resource,
        resource_type,
        profile,
    ));

    state.active_profiles.remove(&profile.url);
    dedupe_exact_issues(issues)
}

fn validate_base_definition_profile<T: Serialize>(
    ctx: &ValidationContext<'_>,
    state: &ValidationState,
    resource: &T,
    resource_type: &str,
    profile: &ExtractedProfile,
) -> Vec<ValidationIssue> {
    if !ctx.validator.config.recurse_on_base_definition {
        return Vec::new();
    }

    let Some(base_url_raw) = profile.base_definition.as_deref() else {
        return Vec::new();
    };
    let base_url = canonical_profile_url(base_url_raw);
    if base_url.is_empty() || is_core_type_base_definition(base_url, &profile.resource_type) {
        return Vec::new();
    }

    let Some(base_profile) =
        resolve_base_profile_sync(ctx, base_url, profile.snapshot_base_version.as_deref())
    else {
        return Vec::new();
    };

    let mut child_state = ValidationState {
        recursion_depth: state.recursion_depth + 1,
        active_profiles: state.active_profiles.clone(),
    };
    validate_profile_with_depth(
        ctx,
        &mut child_state,
        resource,
        resource_type,
        &base_profile,
    )
}

fn resolve_base_profile_sync(
    ctx: &ValidationContext<'_>,
    base_url: &str,
    snapshot_base_version: Option<&str>,
) -> Option<ExtractedProfile> {
    if let Some(registry) = ctx.runtime_profile_registry {
        if let Some(profile) = registry
            .get(base_url)
            .or_else(|| registry.get(canonical_profile_url(base_url)))
        {
            return Some(profile.clone());
        }
    }

    if !ctx.validator.config.enable_base_definition_url_lookup {
        return None;
    }
    fetch_remote_profile_sync(ctx, base_url, snapshot_base_version)
}

async fn validate_base_definition_profile_async<'a, T: Serialize + 'a>(
    ctx: &'a AsyncValidationContext<'a>,
    state: &'a ValidationState,
    resource: &'a T,
    resource_type: &'a str,
    profile: &'a ExtractedProfile,
) -> Vec<ValidationIssue> {
    if !ctx.validator.config.recurse_on_base_definition {
        return Vec::new();
    }

    let Some(base_url_raw) = profile.base_definition.as_deref() else {
        return Vec::new();
    };
    let base_url = canonical_profile_url(base_url_raw);
    if base_url.is_empty() || is_core_type_base_definition(base_url, &profile.resource_type) {
        return Vec::new();
    }

    let Some(base_profile) =
        resolve_base_profile_async(ctx, base_url, profile.snapshot_base_version.as_deref()).await
    else {
        return Vec::new();
    };

    let mut child_state = ValidationState {
        recursion_depth: state.recursion_depth + 1,
        active_profiles: state.active_profiles.clone(),
    };
    validate_profile_with_depth_async(
        ctx,
        &mut child_state,
        resource,
        resource_type,
        &base_profile,
    )
    .await
}

async fn resolve_base_profile_async(
    ctx: &AsyncValidationContext<'_>,
    base_url: &str,
    snapshot_base_version: Option<&str>,
) -> Option<ExtractedProfile> {
    if let Some(registry) = ctx.runtime_profile_registry {
        if let Some(profile) = registry
            .get(base_url)
            .or_else(|| registry.get(canonical_profile_url(base_url)))
        {
            return Some(profile.clone());
        }
    }

    if !ctx.validator.config.enable_base_definition_url_lookup {
        return None;
    }
    fetch_remote_profile_async(ctx, base_url, snapshot_base_version).await
}

/// User-Agent for remote `StructureDefinition` fetches. Hosts such as `hl7.org` have been
/// observed to respond with **405** and HTML to the default `reqwest` UA while accepting the same
/// request from browsers or `curl`.
#[doc(hidden)]
pub fn remote_structure_definition_fetch_user_agent() -> &'static str {
    concat!(
        "Mozilla/5.0 (compatible; helios-fhir-validation/",
        env!("CARGO_PKG_VERSION"),
        "; +https://github.com/HeliosSoftware/hfs)"
    )
}

fn fetch_remote_profile_sync(
    ctx: &ValidationContext<'_>,
    base_url: &str,
    snapshot_base_version: Option<&str>,
) -> Option<ExtractedProfile> {
    let fetch_url = structure_definition_json_fetch_url(base_url, snapshot_base_version);
    if !is_allowed_base_profile_url(&fetch_url, &ctx.validator.config) {
        return None;
    }
    let cache =
        REMOTE_BASE_PROFILE_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(base_url) {
            return cached.clone();
        }
    }

    let timeout = Duration::from_millis(ctx.validator.config.base_definition_url_lookup_timeout_ms);
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent(remote_structure_definition_fetch_user_agent())
        .build()
        .ok()?;
    let resp = client
        .get(fetch_url.as_str())
        .header(
            reqwest::header::ACCEPT,
            "application/fhir+json, application/json;q=0.9",
        )
        .send()
        .ok()?;
    if !resp.status().is_success() {
        if let Ok(mut guard) = cache.lock() {
            guard.insert(base_url.to_string(), None);
        }
        return None;
    }
    let body = resp.bytes().ok()?;
    if body.len() > ctx.validator.config.base_definition_url_lookup_max_bytes {
        if let Ok(mut guard) = cache.lock() {
            guard.insert(base_url.to_string(), None);
        }
        return None;
    }
    let value: Value = serde_json::from_slice(&body).ok()?;
    let extracted = extract_structure_definition_profile_from_json(&value).ok();
    if let Ok(mut guard) = cache.lock() {
        guard.insert(base_url.to_string(), extracted.clone());
    }
    extracted
}

async fn fetch_remote_profile_async(
    ctx: &AsyncValidationContext<'_>,
    base_url: &str,
    snapshot_base_version: Option<&str>,
) -> Option<ExtractedProfile> {
    let fetch_url = structure_definition_json_fetch_url(base_url, snapshot_base_version);
    if !is_allowed_base_profile_url(&fetch_url, &ctx.validator.config) {
        return None;
    }
    let cache =
        REMOTE_BASE_PROFILE_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(base_url) {
            return cached.clone();
        }
    }

    let timeout = Duration::from_millis(ctx.validator.config.base_definition_url_lookup_timeout_ms);
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(remote_structure_definition_fetch_user_agent())
        .build()
        .ok()?;
    let resp = client
        .get(fetch_url.as_str())
        .header(
            reqwest::header::ACCEPT,
            "application/fhir+json, application/json;q=0.9",
        )
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        if let Ok(mut guard) = cache.lock() {
            guard.insert(base_url.to_string(), None);
        }
        return None;
    }
    let body = resp.bytes().await.ok()?;
    if body.len() > ctx.validator.config.base_definition_url_lookup_max_bytes {
        if let Ok(mut guard) = cache.lock() {
            guard.insert(base_url.to_string(), None);
        }
        return None;
    }
    let value: Value = serde_json::from_slice(&body).ok()?;
    let extracted = extract_structure_definition_profile_from_json(&value).ok();
    if let Ok(mut guard) = cache.lock() {
        guard.insert(base_url.to_string(), extracted.clone());
    }
    extracted
}

fn canonical_profile_url(url: &str) -> &str {
    url.split('|').next().unwrap_or(url)
}

fn is_core_type_base_definition(base_url: &str, resource_type: &str) -> bool {
    base_url
        .rsplit('/')
        .next()
        .is_some_and(|tail| tail == resource_type)
        && (base_url.contains("hl7.org/fhir/StructureDefinition/")
            || base_url.contains("hl7.org/fhir/4.0/StructureDefinition/")
            || base_url.contains("hl7.org/fhir/5.0/StructureDefinition/"))
}

fn is_allowed_base_profile_url(url: &str, config: &crate::ValidationConfig) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return false;
    }
    if config.base_definition_url_lookup_allowed_hosts.is_empty() {
        return true;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    config
        .base_definition_url_lookup_allowed_hosts
        .iter()
        .any(|allowed| host == allowed || host.ends_with(&format!(".{allowed}")))
}

#[cfg(test)]
mod base_profile_url_allowlist_tests {
    use super::is_allowed_base_profile_url;
    use crate::ValidationConfig;

    #[test]
    fn allowlist_empty_allows_https_hosts() {
        let config = ValidationConfig::default();
        assert!(is_allowed_base_profile_url(
            "https://example.org/fhir/StructureDefinition/patient",
            &config
        ));
    }

    #[test]
    fn allowlist_blocks_hosts_not_listed() {
        let mut config = ValidationConfig::default();
        config.base_definition_url_lookup_allowed_hosts = vec!["hl7.org".to_string()];
        assert!(!is_allowed_base_profile_url(
            "https://evil.example/StructureDefinition/patient",
            &config
        ));
    }

    #[test]
    fn allowlist_accepts_exact_or_subdomain_match() {
        let mut config = ValidationConfig::default();
        config.base_definition_url_lookup_allowed_hosts = vec!["nrces.in".to_string()];
        assert!(is_allowed_base_profile_url(
            "https://nrces.in/ndhm/fhir/r4/StructureDefinition/Patient",
            &config
        ));
        assert!(is_allowed_base_profile_url(
            "https://api.nrces.in/ndhm/fhir/r4/StructureDefinition/Patient",
            &config
        ));
    }
}
/// Prefix issue paths produced during recursive nested profile validation so
/// they are reported relative to the parent element path in the outer resource.
fn prefix_nested_issue_paths(
    mut issues: Vec<ValidationIssue>,
    parent_path: &str,
    nested_resource_type: &str,
) -> Vec<ValidationIssue> {
    let nested_prefix = format!("{}.", nested_resource_type);
    let parent_prefix = format!("{}.", parent_path);

    for issue in &mut issues {
        issue.fhir_path = prefix_single_issue_path(
            &issue.fhir_path,
            parent_path,
            &parent_prefix,
            nested_resource_type,
            &nested_prefix,
        );

        if let Some(instance_path) = issue.instance_path.as_mut() {
            let updated = prefix_single_issue_path(
                instance_path,
                parent_path,
                &parent_prefix,
                nested_resource_type,
                &nested_prefix,
            );
            *instance_path = updated;
        }
    }

    issues
}

/// Prefix a single nested issue path with the parent path while stripping the
/// nested resource type prefix when present.
fn prefix_single_issue_path(
    path: &str,
    parent_path: &str,
    parent_prefix: &str,
    nested_resource_type: &str,
    nested_prefix: &str,
) -> String {
    if path.is_empty() || path == nested_resource_type {
        return parent_path.to_string();
    }

    if let Some(rest) = path.strip_prefix(nested_prefix) {
        return format!("{}{}", parent_prefix, rest);
    }

    format!("{}{}", parent_prefix, path)
}

/// Validate narrowed polymorphic choice-element type constraints.
///
/// This currently applies only to `[x]` elements and checks that the concrete
/// JSON choice representation matches one of the allowed extracted type codes.
fn validate_type_constraints<T: Serialize>(
    resource: &T,
    resource_type: &str,
    rules: &[crate::profile::types::ExtractedElementRule],
) -> Vec<ValidationIssue> {
    let root = match serde_json::to_value(resource) {
        Ok(value) => value,
        Err(err) => {
            return vec![ValidationIssue {
                severity: crate::Severity::Error,
                code: "processing".to_string(),
                summary: Some(
                    "Resource could not be serialized for type constraint validation".to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
                diagnostics: format!(
                    "Failed to serialize resource while validating type constraints: {}",
                    err
                ),
                expression: None,
                fhir_path: "".to_string(),
                instance_path: None,
            }];
        }
    };

    let mut issues = Vec::new();

    for rule in rules {
        if rule.type_constraints.is_empty() {
            continue;
        }

        // First pass: only enforce narrowed polymorphic choice elements like value[x], deceased[x], etc.
        if !rule.path.contains("[x]") {
            continue;
        }

        let Some(relative_path) = relative_profile_path(resource_type, &rule.path) else {
            continue;
        };

        let Some(choice_info) = actual_choice_type_codes(&root, relative_path) else {
            continue;
        };

        if choice_info.has_multiple_in_same_parent {
            issues.push(ValidationIssue {
                severity: crate::Severity::Error,
                code: issue_code::STRUCTURE.to_string(),
                summary: Some(
                    "Polymorphic [x] element has multiple type representations at once".to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
                diagnostics: format!(
                    "Element '{}' has multiple [x] representations present in the same object: {}.",
                    rule.path,
                    choice_info
                        .actual_type_codes
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                expression: None,
                fhir_path: rule.path.clone(),
                instance_path: Some(rule.path.clone()),
            });
            continue;
        }

        let allowed_codes: Vec<&str> = rule
            .type_constraints
            .iter()
            .map(|constraint| constraint.code.as_str())
            .collect();

        let disallowed_actual_types: Vec<&str> = choice_info
            .actual_type_codes
            .iter()
            .map(String::as_str)
            .filter(|actual_type_code| {
                !allowed_codes
                    .iter()
                    .any(|allowed| type_code_matches_choice_suffix(allowed, actual_type_code))
            })
            .collect();

        if disallowed_actual_types.is_empty() {
            continue;
        }

        issues.push(ValidationIssue {
            severity: crate::Severity::Error,
            code: issue_code::STRUCTURE.to_string(),
            summary: Some("Choice element type is not allowed by the profile".to_string()),
            expression_kind: None,
            source_invariant_key: None,
            detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
            diagnostics: format!(
                "Element '{}' uses disallowed type(s) '{}'. Allowed types: {}.",
                rule.path,
                disallowed_actual_types.join(", "),
                rule.type_constraints
                    .iter()
                    .map(|constraint| constraint.code.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            expression: None,
            fhir_path: rule.path.clone(),
            instance_path: Some(rule.path.clone()),
        });
    }

    issues
}

/// Summary of the concrete choice-type representations found for a single `[x]`
/// element path.
struct ChoiceTypeInfo {
    actual_type_codes: Vec<String>,
    has_multiple_in_same_parent: bool,
}

/// Discover the concrete JSON suffixes present for a polymorphic `[x]` element.
///
/// For example, `value[x]` may resolve to concrete keys such as `valueString`
/// or `valueCodeableConcept`.
fn actual_choice_type_codes(root: &Value, relative_path: &str) -> Option<ChoiceTypeInfo> {
    let choice_stem = relative_path.strip_suffix("[x]")?;

    let (parent, last_segment) = split_parent_path(choice_stem);
    let parent_values = get_values_at_relative_path(root, parent);
    if parent_values.is_empty() {
        return None;
    }

    let mut actual_type_codes = BTreeSet::new();
    let mut has_multiple_in_same_parent = false;

    for parent_value in parent_values {
        if let Value::Object(map) = parent_value {
            let matches: Vec<String> = map
                .keys()
                .filter_map(|key| key.strip_prefix(last_segment))
                .filter(|suffix| !suffix.is_empty())
                .map(str::to_owned)
                .collect();

            if matches.len() > 1 {
                has_multiple_in_same_parent = true;
            }

            for matched in matches {
                actual_type_codes.insert(matched);
            }
        }
    }

    if actual_type_codes.is_empty() {
        return None;
    }

    Some(ChoiceTypeInfo {
        actual_type_codes: actual_type_codes.into_iter().collect(),
        has_multiple_in_same_parent,
    })
}

/// Split a dotted relative path into `(parent_path, last_segment)`.
fn split_parent_path(path: &str) -> (&str, &str) {
    match path.rsplit_once('.') {
        Some((parent, last)) => (parent, last),
        None => ("", path),
    }
}

/// Return `true` when an extracted FHIR type code corresponds to the concrete
/// JSON suffix used for a choice element representation.
fn type_code_matches_choice_suffix(allowed_type_code: &str, actual_suffix: &str) -> bool {
    normalize_choice_type_code(allowed_type_code) == actual_suffix
}

/// Normalize a FHIR type code to the suffix convention used by generated JSON
/// keys for choice elements.
fn normalize_choice_type_code(type_code: &str) -> &str {
    match type_code {
        "boolean" => "Boolean",
        "integer" => "Integer",
        "integer64" => "Integer64",
        "decimal" => "Decimal",
        "base64Binary" => "Base64Binary",
        "instant" => "Instant",
        "string" => "String",
        "uri" => "Uri",
        "url" => "Url",
        "canonical" => "Canonical",
        "date" => "Date",
        "dateTime" => "DateTime",
        "time" => "Time",
        "code" => "Code",
        "oid" => "Oid",
        "id" => "Id",
        "markdown" => "Markdown",
        "unsignedInt" => "UnsignedInt",
        "positiveInt" => "PositiveInt",
        "xhtml" => "Xhtml",
        other => other,
    }
}

/// Validate `targetProfile`-style restrictions on reference targets.
///
/// This checks whether a reference points to an allowed resource type, including
/// inline contained references of the form `#id` when the contained resource is
/// present in the current JSON instance.
fn validate_target_profile_constraints<T: Serialize>(
    resource: &T,
    resource_type: &str,
    rules: &[crate::profile::types::ExtractedElementRule],
) -> Vec<ValidationIssue> {
    let root = match serde_json::to_value(resource) {
        Ok(value) => value,
        Err(err) => {
            return vec![ValidationIssue {
                severity: crate::Severity::Error,
                code: "processing".to_string(),
                summary: Some(
                    "Resource could not be serialized for reference target validation".to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::ValidationException),
                diagnostics: format!(
                    "Failed to serialize resource while validating targetProfile constraints: {}",
                    err
                ),
                expression: None,
                fhir_path: "".to_string(),
                instance_path: None,
            }];
        }
    };

    let mut issues = Vec::new();

    for rule in rules {
        let allowed_target_types: Vec<String> = rule
            .type_constraints
            .iter()
            .flat_map(|constraint| constraint.target_profiles.iter())
            .filter_map(|url| target_profile_resource_type(url))
            .collect();

        if allowed_target_types.is_empty() {
            continue;
        }

        let Some(relative_path) = relative_profile_path(resource_type, &rule.path) else {
            continue;
        };

        let actual_values = get_values_at_relative_path(&root, relative_path);
        if actual_values.is_empty() {
            continue;
        }

        for actual in actual_values {
            let Some(actual_target_type) = actual_reference_target_type(actual, &root) else {
                continue;
            };

            if allowed_target_types
                .iter()
                .any(|allowed| allowed == actual_target_type)
            {
                continue;
            }

            issues.push(ValidationIssue {
                severity: crate::Severity::Error,
                code: issue_code::STRUCTURE.to_string(),
                summary: Some(
                    "Reference target resource type is not allowed by targetProfile".to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
                diagnostics: format!(
                    "Element '{}' references resource type '{}', which is not allowed by the profile. Allowed target types: {}.",
                    rule.path,
                    actual_target_type,
                    allowed_target_types.join(", ")
                ),
                expression: None,
                fhir_path: rule.path.clone(),
                instance_path: Some(rule.path.clone()),
            });
        }
    }
    issues
}

/// Enforce `ElementDefinition.type.aggregation` and `type.versioning` when those
/// fields are present on extracted type rows for `Reference`, `CodeableReference`,
/// or `canonical`.
///
/// Multiple `type` entries are combined with **OR**: an instance satisfies the
/// element if it matches **at least one** applicable constraint row.
fn validate_reference_aggregate_versioning_constraints<T: Serialize>(
    resource: &T,
    resource_type: &str,
    rules: &[crate::profile::types::ExtractedElementRule],
) -> Vec<ValidationIssue> {
    let root = match serde_json::to_value(resource) {
        Ok(value) => value,
        Err(err) => {
            return vec![ValidationIssue {
                severity: crate::Severity::Error,
                code: "processing".to_string(),
                summary: Some(
                    "Resource could not be serialized for reference aggregation/versioning validation"
                        .to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::ValidationException),
                diagnostics: format!(
                    "Failed to serialize resource while validating reference aggregation/versioning: {}",
                    err
                ),
                expression: None,
                fhir_path: "".to_string(),
                instance_path: None,
            }];
        }
    };

    let mut issues = Vec::new();

    for rule in rules {
        let ref_constraints: Vec<&ExtractedTypeConstraint> = rule
            .type_constraints
            .iter()
            .filter(|c| {
                type_constraint_has_aggregate_or_versioning(c)
                    && type_constraint_supports_reference_semantics(c)
            })
            .collect();

        if ref_constraints.is_empty() {
            continue;
        }

        let Some(relative_path) = relative_profile_path(resource_type, &rule.path) else {
            continue;
        };

        let actual_values = get_values_at_relative_path(&root, relative_path);
        if actual_values.is_empty() {
            continue;
        }

        let has_canonical_row = ref_constraints
            .iter()
            .any(|c| c.code.eq_ignore_ascii_case("canonical"));

        for actual in actual_values {
            let ref_url = reference_url_for_aggregation(actual);
            let canonical_str = actual.as_str();

            let should_validate =
                ref_url.is_some() || (canonical_str.is_some() && has_canonical_row);

            if !should_validate {
                continue;
            }

            if ref_constraints
                .iter()
                .any(|c| instance_matches_aggregate_version_constraint(c, actual, &root))
            {
                continue;
            }

            issues.push(ValidationIssue {
                severity: crate::Severity::Error,
                code: issue_code::STRUCTURE.to_string(),
                summary: Some(
                    "Reference or canonical value does not satisfy type aggregation/versioning rules"
                        .to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
                diagnostics: format!(
                    "Element '{}' does not match any allowed ElementDefinition.type aggregation/versioning alternative. Profile declares {:?} on this path; instance reference: {:?}.",
                    rule.path,
                    ref_constraints
                        .iter()
                        .map(|c| {
                            (
                                c.code.as_str(),
                                c.aggregation.as_slice(),
                                c.versioning.as_deref(),
                            )
                        })
                        .collect::<Vec<_>>(),
                    ref_url.or(canonical_str)
                ),
                expression: None,
                fhir_path: rule.path.clone(),
                instance_path: Some(rule.path.clone()),
            });
        }
    }

    issues
}

fn type_constraint_supports_reference_semantics(c: &ExtractedTypeConstraint) -> bool {
    c.code.eq_ignore_ascii_case("Reference")
        || c.code.eq_ignore_ascii_case("CodeableReference")
        || c.code.eq_ignore_ascii_case("canonical")
}

fn type_constraint_has_aggregate_or_versioning(c: &ExtractedTypeConstraint) -> bool {
    !c.aggregation.is_empty() || c.versioning.is_some()
}

fn instance_matches_aggregate_version_constraint(
    c: &ExtractedTypeConstraint,
    value: &Value,
    root: &Value,
) -> bool {
    if c.code.eq_ignore_ascii_case("canonical") {
        let Value::String(s) = value else {
            return false;
        };
        // `aggregation` on `canonical` is uncommon; versioning uses `|` in the URI.
        return versioning_matches_canonical(c.versioning.as_deref(), s);
    }

    if !c.code.eq_ignore_ascii_case("Reference")
        && !c.code.eq_ignore_ascii_case("CodeableReference")
    {
        return false;
    }

    let Some(ref_url) = reference_url_for_aggregation(value) else {
        return false;
    };

    aggregation_matches_reference(&c.aggregation, ref_url, root)
        && versioning_matches_reference(c.versioning.as_deref(), ref_url)
}

fn aggregation_matches_reference(allowed: &[String], ref_url: &str, root: &Value) -> bool {
    if allowed.is_empty() {
        return true;
    }
    let modes = applicable_aggregation_modes(ref_url, root);
    allowed.iter().any(|a| modes.iter().any(|m| m == a))
}

fn root_is_bundle(root: &Value) -> bool {
    root.as_object()
        .and_then(|m| m.get("resourceType"))
        .and_then(Value::as_str)
        == Some("Bundle")
}

/// Classify how a reference participates in aggregation (FHIR
/// `resource-aggregation-mode`).
///
/// **`bundled`** is only assigned when the instance under validation is a
/// [`Bundle`](https://hl7.org/fhir/bundle.html) **and** the reference resolves to
/// an entry in that bundle (via `fullUrl` or `resourceType`/`id`). Opaque
/// `urn:uuid` / `urn:oid` references are **not** treated as bundled outside of a
/// bundle; they are classified as **`referenced`** so server-side logical
/// references still match `aggregation: referenced` on non-Bundle resources.
fn applicable_aggregation_modes(ref_url: &str, root: &Value) -> Vec<String> {
    let mut set = HashSet::new();
    if ref_url.starts_with('#') {
        set.insert("contained".to_string());
        return set.into_iter().collect();
    }

    let in_bundle = root_is_bundle(root);

    if in_bundle && bundle_entry_matches_reference(root, ref_url) {
        set.insert("bundled".to_string());
    }

    if is_referenced_style_reference(ref_url) {
        set.insert("referenced".to_string());
    }

    if !in_bundle && (ref_url.starts_with("urn:uuid:") || ref_url.starts_with("urn:oid:")) {
        set.insert("referenced".to_string());
    }

    if set.is_empty() {
        if !ref_url.is_empty()
            && !ref_url.starts_with('#')
            && !(in_bundle && (ref_url.starts_with("urn:uuid:") || ref_url.starts_with("urn:oid:")))
        {
            set.insert("referenced".to_string());
        }
    }

    set.into_iter().collect()
}

fn is_referenced_style_reference(ref_url: &str) -> bool {
    !ref_url.starts_with('#')
        && !ref_url.starts_with("urn:uuid:")
        && !ref_url.starts_with("urn:oid:")
}

/// FHIR primitive / string-like JSON either as a JSON string or as `{"value": "..."}`
/// (generated model serialization).
fn json_stringish_value(value: &Value) -> Option<&str> {
    match value {
        Value::String(s) => Some(s.as_str()),
        Value::Object(map) => map
            .get(issue_code::FHIR_JSON_VALUE)
            .and_then(|v| v.as_str()),
        _ => None,
    }
}

fn bundle_entry_matches_reference(root: &Value, ref_url: &str) -> bool {
    let Value::Object(root_map) = root else {
        return false;
    };
    if root_map.get("resourceType").and_then(Value::as_str) != Some("Bundle") {
        return false;
    }
    let Some(entries) = root_map.get("entry").and_then(Value::as_array) else {
        return false;
    };
    for e in entries {
        if let Some(fu) = e.get("fullUrl").and_then(json_stringish_value) {
            if fu == ref_url {
                return true;
            }
        }
        let Some(res) = e.get("resource") else {
            continue;
        };
        let Value::Object(rm) = res else {
            continue;
        };
        let rt = rm.get("resourceType").and_then(Value::as_str);
        let id = rm.get("id").and_then(Value::as_str);
        let Some(rt) = rt else {
            continue;
        };
        let Some(id) = id else {
            continue;
        };
        let composed = format!("{rt}/{id}");
        if composed == ref_url || ref_url.ends_with(&composed) {
            return true;
        }
    }
    false
}

fn versioning_matches_reference(versioning: Option<&str>, ref_url: &str) -> bool {
    let Some(v) = versioning else {
        return true;
    };
    let version_specific = reference_url_is_version_specific(ref_url);
    match v {
        "either" => true,
        "independent" => !version_specific,
        "specific" => version_specific,
        _ => true,
    }
}

fn reference_url_is_version_specific(ref_url: &str) -> bool {
    ref_url.contains("/_history/")
}

fn versioning_matches_canonical(versioning: Option<&str>, canonical: &str) -> bool {
    let Some(v) = versioning else {
        return true;
    };
    let version_specific = canonical_has_explicit_version(canonical);
    match v {
        "either" => true,
        "independent" => !version_specific,
        "specific" => version_specific,
        _ => true,
    }
}

/// `canonical` datatype version is expressed with `|` after the base URI.
fn canonical_has_explicit_version(canonical: &str) -> bool {
    canonical
        .rsplit_once('|')
        .is_some_and(|(base, ver)| !base.is_empty() && !ver.is_empty())
}

/// Extract `Reference.reference` for aggregation/versioning, including nested
/// `CodeableReference.reference`.
fn reference_url_for_aggregation(value: &Value) -> Option<&str> {
    let Value::Object(map) = value else {
        return None;
    };
    match map.get("reference")? {
        Value::String(s) => Some(s.as_str()),
        Value::Object(inner) => inner.get("reference").and_then(json_stringish_value),
        _ => None,
    }
}

/// Determine the referenced target resource type from a reference-like JSON value.
fn actual_reference_target_type<'a>(value: &'a Value, root: &'a Value) -> Option<&'a str> {
    let Value::Object(map) = value else {
        return None;
    };

    let reference = match map.get("reference")? {
        Value::String(s) => s.as_str(),
        Value::Object(inner) => inner.get("reference").and_then(json_stringish_value)?,
        _ => return None,
    };
    if let Some(contained_id) = reference.strip_prefix('#') {
        return contained_resource_type_by_id(root, contained_id);
    }

    parse_reference_resource_type(reference)
}

/// Resolve the resource type of a contained resource by its local `#id` target.
fn contained_resource_type_by_id<'a>(root: &'a Value, contained_id: &str) -> Option<&'a str> {
    let Value::Object(root_map) = root else {
        return None;
    };

    let contained = root_map.get("contained")?;
    let Value::Array(items) = contained else {
        return None;
    };

    for item in items {
        let Value::Object(resource_map) = item else {
            continue;
        };

        let Some(id) = resource_map.get("id").and_then(Value::as_str) else {
            continue;
        };

        if id == contained_id {
            return resource_map.get("resourceType").and_then(Value::as_str);
        }
    }

    None
}

/// Infer the target resource type name from a non-local FHIR reference string.
fn parse_reference_resource_type(reference: &str) -> Option<&str> {
    if reference.starts_with('#') {
        return None;
    }

    let path = reference
        .split_once("://")
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.split_once('/').map(|(_, path)| path))
        .unwrap_or(reference);

    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    let first = segments.next()?;

    if first == "_history" {
        return None;
    }

    Some(first)
}

/// Best-effort mapping from a target profile URL to its implied resource type.
///
/// Used when validating nested content under `type.profile` / `targetProfile` but the runtime
/// [`ProfileRegistry`] does not contain an entry for that canonical URL—so we
/// cannot read [`ExtractedProfile::resource_type`] directly.
///
/// The heuristic recognizes common HL7 core tail segments (`Patient`, `Observation`, …) and
/// several `…-{lowercaseresource}` naming patterns from published IGs. Unknown shapes return
/// [`None`] and callers may skip type-based fallbacks.
pub fn target_profile_resource_type(url: &str) -> Option<String> {
    let tail = url.rsplit('/').next()?;

    let candidate = match tail {
        "Patient" => "Patient",
        "Practitioner" => "Practitioner",
        "PractitionerRole" => "PractitionerRole",
        "RelatedPerson" => "RelatedPerson",
        "Person" => "Person",
        "Group" => "Group",
        "Organization" => "Organization",
        "Location" => "Location",
        "Device" => "Device",
        "Observation" => "Observation",
        "Encounter" => "Encounter",
        "Condition" => "Condition",
        "Procedure" => "Procedure",
        "MedicationRequest" => "MedicationRequest",
        "Medication" => "Medication",
        "Substance" => "Substance",
        "Specimen" => "Specimen",
        "ServiceRequest" => "ServiceRequest",
        "CarePlan" => "CarePlan",
        "DiagnosticReport" => "DiagnosticReport",
        "ImagingStudy" => "ImagingStudy",
        "AllergyIntolerance" => "AllergyIntolerance",
        "Immunization" => "Immunization",
        other if other.ends_with("-patient") => "Patient",
        other if other.ends_with("-practitioner") => "Practitioner",
        other if other.ends_with("-practitionerrole") => "PractitionerRole",
        other if other.ends_with("-relatedperson") => "RelatedPerson",
        other if other.ends_with("-group") => "Group",
        other if other.ends_with("-organization") => "Organization",
        other if other.ends_with("-location") => "Location",
        other if other.ends_with("-observation") => "Observation",
        other if other.ends_with("-encounter") => "Encounter",
        _ => return None,
    };

    Some(candidate.to_string())
}

/// Validate `type.profile` constraints on nested resource-valued elements.
///
/// This supports two matching strategies:
/// - explicit declared `meta.profile` matching on the nested resource
/// - optional fallback to resource type matching when enabled by configuration
///
/// When matching profiles are known in the registry, recursive validation of the
/// nested resource against the matched profile(s) may also be performed.
fn validate_type_profile_constraints<T: Serialize>(
    ctx: &ValidationContext<'_>,
    state: &mut ValidationState,
    resource: &T,
    resource_type: &str,
    rules: &[crate::profile::types::ExtractedElementRule],
) -> Vec<ValidationIssue> {
    let root = match serde_json::to_value(resource) {
        Ok(value) => value,
        Err(err) => {
            return vec![ValidationIssue {
                severity: crate::Severity::Error,
                code: "processing".to_string(),
                summary: Some(
                    "Resource could not be serialized for type.profile validation".to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::ValidationException),
                diagnostics: format!(
                    "Failed to serialize resource while validating type.profile constraints: {}",
                    err
                ),
                expression: None,
                fhir_path: "".to_string(),
                instance_path: None,
            }];
        }
    };

    let mut issues = Vec::new();

    for rule in rules {
        let required_profiles: Vec<&str> = rule
            .type_constraints
            .iter()
            .flat_map(|constraint| constraint.profiles.iter().map(String::as_str))
            .collect();

        if required_profiles.is_empty() {
            continue;
        }

        let Some(relative_path) = relative_profile_path(resource_type, &rule.path) else {
            continue;
        };

        let actual_values =
            get_values_with_paths_at_relative_path(&root, resource_type, relative_path);
        if actual_values.is_empty() {
            continue;
        }

        for (actual, actual_path) in actual_values {
            let mut unknown_required_profiles: Vec<&str> = Vec::new();
            let mut known_required_profiles: Vec<&str> = Vec::new();

            if let Some(registry) = ctx.runtime_profile_registry {
                for profile_url in &required_profiles {
                    if registry.get(profile_url).is_some() {
                        known_required_profiles.push(*profile_url);
                    } else {
                        unknown_required_profiles.push(*profile_url);
                    }
                }
            } else {
                unknown_required_profiles.extend(required_profiles.iter().copied());
            }

            if !unknown_required_profiles.is_empty() {
                if ctx.validator.config.error_on_unknown_profile {
                    issues.push(ValidationIssue {
                        severity: crate::Severity::Error,
                        code: "not-found".to_string(),
                        summary: Some(
                            "Required StructureDefinition profile URL is not in the profile registry"
                                .to_string(),
                        ),
                        expression_kind: None,
                        source_invariant_key: None,
                        detail_code: Some(ValidationIssueDetailCode::ReferenceNotFound),
                        diagnostics: format!(
                            "Element '{}' requires unknown profile(s): {}.",
                            rule.path,
                            unknown_required_profiles.join(", ")
                        ),
                        expression: None,
                        fhir_path: actual_path.clone(),
                        instance_path: Some(actual_path.clone()),
                    });
                    continue;
                }

                if ctx.validator.config.warn_on_unknown_profile {
                    issues.push(ValidationIssue {
                        severity: crate::Severity::Warning,
                        code: "not-found".to_string(),
                        summary: Some(
                            "Referenced profile URL is not available in the profile registry"
                                .to_string(),
                        ),
                        expression_kind: None,
                        source_invariant_key: None,
                        detail_code: Some(ValidationIssueDetailCode::ReferenceNotFound),
                        diagnostics: format!(
                            "Element '{}' references unknown profile(s): {}.",
                            rule.path,
                            unknown_required_profiles.join(", ")
                        ),
                        expression: None,
                        fhir_path: actual_path.clone(),
                        instance_path: Some(actual_path.clone()),
                    });
                }
            }

            let declared_profiles = declared_profiles_for_type_profile_instance(actual);
            if !declared_profiles.is_empty() {
                let matching_required_profiles: Vec<&str> = known_required_profiles
                    .iter()
                    .copied()
                    .filter(|required| {
                        declared_profiles
                            .iter()
                            .any(|declared| declared == required)
                    })
                    .collect();

                let declared_match_ok = match ctx.validator.config.type_profile_match_mode {
                    TypeProfileMatchMode::Any => !matching_required_profiles.is_empty(),
                    TypeProfileMatchMode::All => {
                        !known_required_profiles.is_empty()
                            && known_required_profiles.iter().all(|required| {
                                declared_profiles
                                    .iter()
                                    .any(|declared| declared == required)
                            })
                    }
                };

                if !declared_match_ok {
                    issues.push(ValidationIssue {
                        severity: crate::Severity::Error,
                        code: issue_code::STRUCTURE.to_string(),
                        summary: Some(
                            "Declared profiles do not satisfy type.profile requirement"
                                .to_string(),
                        ),
                        expression_kind: None,
                        source_invariant_key: None,
                        detail_code: Some(ValidationIssueDetailCode::BusinessRuleViolation),
                        diagnostics: format!(
                            "Element '{}' does not declare the required profile match. Required profiles: {}. Declared profiles: {}. Match mode: {:?}.",
                            rule.path,
                            known_required_profiles.join(", "),
                            declared_profiles.join(", "),
                            ctx.validator.config.type_profile_match_mode
                        ),
                        expression: None,
                        fhir_path: actual_path.clone(),
                        instance_path: Some(actual_path.clone()),
                    });
                    continue;
                }

                if let Some(registry) = ctx.runtime_profile_registry {
                    let profiles_to_recurse: Vec<&str> =
                        match ctx.validator.config.type_profile_match_mode {
                            TypeProfileMatchMode::Any => matching_required_profiles,
                            TypeProfileMatchMode::All => known_required_profiles.clone(),
                        };

                    for profile_url in profiles_to_recurse {
                        if let Some(nested_profile) = registry.get(profile_url) {
                            let nested_resource_type = nested_profile.resource_type.as_str();
                            let mut child_state = ValidationState {
                                recursion_depth: state.recursion_depth + 1,
                                active_profiles: state.active_profiles.clone(),
                            };
                            let nested_issues = validate_profile_with_depth(
                                ctx,
                                &mut child_state,
                                actual,
                                nested_resource_type,
                                nested_profile,
                            );
                            issues.extend(prefix_nested_issue_paths(
                                nested_issues,
                                &actual_path,
                                nested_resource_type,
                            ));
                        }
                    }
                }
                continue;
            }

            if let Some(registry) = ctx.runtime_profile_registry {
                let actual_resource_type = resource_type_name_from_value(actual);
                if let Some(actual_resource_type) = actual_resource_type {
                    if !ctx
                        .validator
                        .config
                        .allow_type_profile_resource_type_fallback
                    {
                        issues.push(ValidationIssue {
                            severity: crate::Severity::Error,
                            code: issue_code::STRUCTURE.to_string(),
                            summary: Some(
                                "meta.profile is missing and resourceType fallback is disabled for type.profile"
                                    .to_string(),
                            ),
                            expression_kind: None,
                            source_invariant_key: None,
                            detail_code: Some(ValidationIssueDetailCode::BusinessRuleViolation),
                            diagnostics: format!(
                                "Element '{}' does not explicitly declare any of the required profiles, and resourceType fallback is disabled. Expected profiles: {}.",
                                rule.path,
                                known_required_profiles.join(", ")
                            ),
                            expression: None,
                            fhir_path: actual_path.clone(),
                            instance_path: Some(actual_path.clone()),
                        });
                        continue;
                    }

                    let matching_profiles: Vec<&str> = known_required_profiles
                        .iter()
                        .copied()
                        .filter(|url| {
                            profile_resource_type(url, Some(registry))
                                .as_deref()
                                .map(|allowed| allowed == actual_resource_type)
                                .unwrap_or(false)
                        })
                        .collect();

                    let fallback_match_ok = match ctx.validator.config.type_profile_match_mode {
                        TypeProfileMatchMode::Any => !matching_profiles.is_empty(),
                        TypeProfileMatchMode::All => {
                            !known_required_profiles.is_empty()
                                && known_required_profiles.iter().all(|url| {
                                    profile_resource_type(url, Some(registry))
                                        .as_deref()
                                        .map(|allowed| allowed == actual_resource_type)
                                        .unwrap_or(false)
                                })
                        }
                    };

                    if !fallback_match_ok {
                        let allowed_resource_types: Vec<String> = known_required_profiles
                            .iter()
                            .filter_map(|url| profile_resource_type(url, Some(registry)))
                            .collect();

                        if !allowed_resource_types.is_empty() {
                            issues.push(ValidationIssue {
                                severity: crate::Severity::Error,
                                code: issue_code::STRUCTURE.to_string(),
                                summary: Some(
                                    "Nested resource type does not match type.profile expectation"
                                        .to_string(),
                                ),
                                expression_kind: None,
                                source_invariant_key: None,
                                detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
                                diagnostics: format!(
                                    "Element '{}' has resource type '{}', which does not match the required profiled type(s): {}. Match mode: {:?}.",
                                    rule.path,
                                    actual_resource_type,
                                    allowed_resource_types.join(", "),
                                    ctx.validator.config.type_profile_match_mode
                                ),
                                expression: None,
                                fhir_path: actual_path.clone(),
                                instance_path: Some(actual_path.clone()),
                            });
                        }
                        continue;
                    }

                    if ctx.validator.config.warn_on_type_profile_fallback {
                        issues.push(ValidationIssue {
                            severity: crate::Severity::Warning,
                            code: "business-rule".to_string(),
                            summary: Some(
                                "type.profile validation used resourceType fallback (meta.profile missing)"
                                    .to_string(),
                            ),
                            expression_kind: None,
                            source_invariant_key: None,
                            detail_code: Some(ValidationIssueDetailCode::BusinessRuleViolation),
                            diagnostics: format!(
                                "Element '{}' does not explicitly declare any of the required profiles. Falling back to resourceType match '{}'. Expected profiles: {}. Match mode: {:?}.",
                                rule.path,
                                actual_resource_type,
                                known_required_profiles.join(", "),
                                ctx.validator.config.type_profile_match_mode
                            ),
                            expression: None,
                            fhir_path: actual_path.clone(),
                            instance_path: Some(actual_path.clone()),
                        });
                    }

                    if ctx.validator.config.recurse_on_type_profile_fallback {
                        let profiles_to_recurse: Vec<&str> =
                            match ctx.validator.config.type_profile_match_mode {
                                TypeProfileMatchMode::Any => matching_profiles,
                                TypeProfileMatchMode::All => known_required_profiles.clone(),
                            };

                        for profile_url in profiles_to_recurse {
                            if let Some(nested_profile) = registry.get(profile_url) {
                                let nested_resource_type = nested_profile.resource_type.as_str();
                                let mut child_state = ValidationState {
                                    recursion_depth: state.recursion_depth + 1,
                                    active_profiles: state.active_profiles.clone(),
                                };
                                let nested_issues = validate_profile_with_depth(
                                    ctx,
                                    &mut child_state,
                                    actual,
                                    nested_resource_type,
                                    nested_profile,
                                );
                                issues.extend(prefix_nested_issue_paths(
                                    nested_issues,
                                    &actual_path,
                                    nested_resource_type,
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    issues
}
fn validate_type_profile_constraints_async<'a, T: Serialize + 'a>(
    ctx: &'a AsyncValidationContext<'a>,
    state: &'a mut ValidationState,
    resource: &'a T,
    resource_type: &'a str,
    rules: &'a [crate::profile::types::ExtractedElementRule],
) -> Pin<Box<dyn Future<Output = Vec<ValidationIssue>> + 'a>> {
    Box::pin(async move {
        let root = match serde_json::to_value(resource) {
            Ok(value) => value,
            Err(err) => {
                return vec![ValidationIssue {
                    severity: crate::Severity::Error,
                    code: "processing".to_string(),
                    summary: Some(
                        "Resource could not be serialized for type.profile validation".to_string(),
                    ),
                    expression_kind: None,
                    source_invariant_key: None,
                    detail_code: Some(ValidationIssueDetailCode::ValidationException),
                    diagnostics: format!(
                        "Failed to serialize resource while validating type.profile constraints: {}",
                        err
                    ),
                    expression: None,
                    fhir_path: "".to_string(),
                    instance_path: None,
                }];
            }
        };

        let mut issues = Vec::new();

        for rule in rules {
            let required_profiles: Vec<&str> = rule
                .type_constraints
                .iter()
                .flat_map(|constraint| constraint.profiles.iter().map(String::as_str))
                .collect();

            if required_profiles.is_empty() {
                continue;
            }

            let Some(relative_path) = relative_profile_path(resource_type, &rule.path) else {
                continue;
            };

            let actual_values =
                get_values_with_paths_at_relative_path(&root, resource_type, relative_path);
            if actual_values.is_empty() {
                continue;
            }

            for (actual, actual_path) in actual_values {
                let mut unknown_required_profiles: Vec<&str> = Vec::new();
                let mut known_required_profiles: Vec<&str> = Vec::new();

                if let Some(registry) = ctx.runtime_profile_registry {
                    for profile_url in &required_profiles {
                        if registry.get(profile_url).is_some() {
                            known_required_profiles.push(*profile_url);
                        } else {
                            unknown_required_profiles.push(*profile_url);
                        }
                    }
                } else {
                    unknown_required_profiles.extend(required_profiles.iter().copied());
                }

                if !unknown_required_profiles.is_empty() {
                    if ctx.validator.config.error_on_unknown_profile {
                        issues.push(ValidationIssue {
                            severity: crate::Severity::Error,
                            code: "not-found".to_string(),
                            summary: Some(
                                "Required StructureDefinition profile URL is not in the profile registry"
                                    .to_string(),
                            ),
                            expression_kind: None,
                            source_invariant_key: None,
                            detail_code: Some(ValidationIssueDetailCode::ReferenceNotFound),
                            diagnostics: format!(
                                "Element '{}' requires unknown profile(s): {}.",
                                rule.path,
                                unknown_required_profiles.join(", ")
                            ),
                            expression: None,
                            fhir_path: actual_path.clone(),
                            instance_path: Some(actual_path.clone()),
                        });
                        continue;
                    }

                    if ctx.validator.config.warn_on_unknown_profile {
                        issues.push(ValidationIssue {
                            severity: crate::Severity::Warning,
                            code: "not-found".to_string(),
                            summary: Some(
                                "Referenced profile URL is not available in the profile registry"
                                    .to_string(),
                            ),
                            expression_kind: None,
                            source_invariant_key: None,
                            detail_code: Some(ValidationIssueDetailCode::ReferenceNotFound),
                            diagnostics: format!(
                                "Element '{}' references unknown profile(s): {}.",
                                rule.path,
                                unknown_required_profiles.join(", ")
                            ),
                            expression: None,
                            fhir_path: actual_path.clone(),
                            instance_path: Some(actual_path.clone()),
                        });
                    }
                }

                let declared_profiles = declared_profiles_for_type_profile_instance(actual);
                if !declared_profiles.is_empty() {
                    let matching_required_profiles: Vec<&str> = known_required_profiles
                        .iter()
                        .copied()
                        .filter(|required| {
                            declared_profiles
                                .iter()
                                .any(|declared| declared == required)
                        })
                        .collect();

                    let declared_match_ok = match ctx.validator.config.type_profile_match_mode {
                        TypeProfileMatchMode::Any => !matching_required_profiles.is_empty(),
                        TypeProfileMatchMode::All => {
                            !known_required_profiles.is_empty()
                                && known_required_profiles.iter().all(|required| {
                                    declared_profiles
                                        .iter()
                                        .any(|declared| declared == required)
                                })
                        }
                    };

                    if !declared_match_ok {
                        issues.push(ValidationIssue {
                            severity: crate::Severity::Error,
                            code: issue_code::STRUCTURE.to_string(),
                            summary: Some(
                                "Declared profiles do not satisfy type.profile requirement"
                                    .to_string(),
                            ),
                        expression_kind: None,
                        source_invariant_key: None,
                            detail_code: Some(ValidationIssueDetailCode::BusinessRuleViolation),
                        diagnostics: format!(
                            "Element '{}' does not declare the required profile match. Required profiles: {}. Declared profiles: {}. Match mode: {:?}.",
                            rule.path,
                            known_required_profiles.join(", "),
                            declared_profiles.join(", "),
                            ctx.validator.config.type_profile_match_mode
                        ),
                        expression: None,
                        fhir_path: actual_path.clone(),
                        instance_path: Some(actual_path.clone()),
                    });
                        continue;
                    }

                    if let Some(registry) = ctx.runtime_profile_registry {
                        let profiles_to_recurse: Vec<&str> =
                            match ctx.validator.config.type_profile_match_mode {
                                TypeProfileMatchMode::Any => matching_required_profiles,
                                TypeProfileMatchMode::All => known_required_profiles.clone(),
                            };

                        for profile_url in profiles_to_recurse {
                            if let Some(nested_profile) = registry.get(profile_url) {
                                let nested_resource_type = nested_profile.resource_type.as_str();
                                let mut child_state = ValidationState {
                                    recursion_depth: state.recursion_depth + 1,
                                    active_profiles: state.active_profiles.clone(),
                                };
                                let nested_issues = validate_profile_with_depth_async(
                                    ctx,
                                    &mut child_state,
                                    actual,
                                    nested_resource_type,
                                    nested_profile,
                                )
                                .await;
                                issues.extend(prefix_nested_issue_paths(
                                    nested_issues,
                                    &actual_path,
                                    nested_resource_type,
                                ));
                            }
                        }
                    }
                    continue;
                }

                if let Some(registry) = ctx.runtime_profile_registry {
                    let actual_resource_type = resource_type_name_from_value(actual);
                    if let Some(actual_resource_type) = actual_resource_type {
                        if !ctx
                            .validator
                            .config
                            .allow_type_profile_resource_type_fallback
                        {
                            issues.push(ValidationIssue {
                            severity: crate::Severity::Error,
                            code: issue_code::STRUCTURE.to_string(),
                            summary: Some(
                                "meta.profile is missing and resourceType fallback is disabled for type.profile"
                                    .to_string(),
                            ),
                            expression_kind: None,
                            source_invariant_key: None,
                                detail_code: Some(ValidationIssueDetailCode::BusinessRuleViolation),
                            diagnostics: format!(
                                "Element '{}' does not explicitly declare any of the required profiles, and resourceType fallback is disabled. Expected profiles: {}.",
                                rule.path,
                                known_required_profiles.join(", ")
                            ),
                            expression: None,
                            fhir_path: actual_path.clone(),
                            instance_path: Some(actual_path.clone()),
                        });
                            continue;
                        }

                        let matching_profiles: Vec<&str> = known_required_profiles
                            .iter()
                            .copied()
                            .filter(|url| {
                                profile_resource_type(url, Some(registry))
                                    .as_deref()
                                    .map(|allowed| allowed == actual_resource_type)
                                    .unwrap_or(false)
                            })
                            .collect();

                        let fallback_match_ok = match ctx.validator.config.type_profile_match_mode {
                            TypeProfileMatchMode::Any => !matching_profiles.is_empty(),
                            TypeProfileMatchMode::All => {
                                !known_required_profiles.is_empty()
                                    && known_required_profiles.iter().all(|url| {
                                        profile_resource_type(url, Some(registry))
                                            .as_deref()
                                            .map(|allowed| allowed == actual_resource_type)
                                            .unwrap_or(false)
                                    })
                            }
                        };

                        if !fallback_match_ok {
                            let allowed_resource_types: Vec<String> = known_required_profiles
                                .iter()
                                .filter_map(|url| profile_resource_type(url, Some(registry)))
                                .collect();

                            if !allowed_resource_types.is_empty() {
                                issues.push(ValidationIssue {
                                severity: crate::Severity::Error,
                                code: issue_code::STRUCTURE.to_string(),
                                summary: Some(
                                    "Nested resource type does not match type.profile expectation"
                                        .to_string(),
                                ),
                                expression_kind: None,
                                source_invariant_key: None,
                                    detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
                                diagnostics: format!(
                                    "Element '{}' has resource type '{}', which does not match the required profiled type(s): {}. Match mode: {:?}.",
                                    rule.path,
                                    actual_resource_type,
                                    allowed_resource_types.join(", "),
                                    ctx.validator.config.type_profile_match_mode
                                ),
                                expression: None,
                                fhir_path: actual_path.clone(),
                                instance_path: Some(actual_path.clone()),
                            });
                            }
                            continue;
                        }

                        if ctx.validator.config.warn_on_type_profile_fallback {
                            issues.push(ValidationIssue {
                            severity: crate::Severity::Warning,
                            code: "business-rule".to_string(),
                            summary: Some(
                                "type.profile validation used resourceType fallback (meta.profile missing)"
                                    .to_string(),
                            ),
                            expression_kind: None,
                            source_invariant_key: None,
                                detail_code: Some(ValidationIssueDetailCode::BusinessRuleViolation),
                            diagnostics: format!(
                                "Element '{}' does not explicitly declare any of the required profiles. Falling back to resourceType match '{}'. Expected profiles: {}. Match mode: {:?}.",
                                rule.path,
                                actual_resource_type,
                                known_required_profiles.join(", "),
                                ctx.validator.config.type_profile_match_mode
                            ),
                            expression: None,
                            fhir_path: actual_path.clone(),
                            instance_path: Some(actual_path.clone()),
                        });
                        }

                        if ctx.validator.config.recurse_on_type_profile_fallback {
                            let profiles_to_recurse: Vec<&str> =
                                match ctx.validator.config.type_profile_match_mode {
                                    TypeProfileMatchMode::Any => matching_profiles,
                                    TypeProfileMatchMode::All => known_required_profiles.clone(),
                                };

                            for profile_url in profiles_to_recurse {
                                if let Some(nested_profile) = registry.get(profile_url) {
                                    let nested_resource_type =
                                        nested_profile.resource_type.as_str();
                                    let mut child_state = ValidationState {
                                        recursion_depth: state.recursion_depth + 1,
                                        active_profiles: state.active_profiles.clone(),
                                    };
                                    let nested_issues = validate_profile_with_depth_async(
                                        ctx,
                                        &mut child_state,
                                        actual,
                                        nested_resource_type,
                                        nested_profile,
                                    )
                                    .await;
                                    issues.extend(prefix_nested_issue_paths(
                                        nested_issues,
                                        &actual_path,
                                        nested_resource_type,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        issues
    })
}

/// Extract declared `meta.profile` URLs from a JSON object value.
fn declared_profiles_on_value(value: &Value) -> Vec<String> {
    let Value::Object(map) = value else {
        return Vec::new();
    };

    let Some(meta) = map.get("meta") else {
        return Vec::new();
    };

    let Value::Object(meta_map) = meta else {
        return Vec::new();
    };

    let Some(profile) = meta_map.get("profile") else {
        return Vec::new();
    };

    let Value::Array(items) = profile else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| item.as_str().map(str::to_owned))
        .collect()
}

/// Declared profiles from `meta.profile`, or — when absent — the Extension `url`,
/// which identifies the extension definition the same way as a StructureDefinition URL.
fn declared_profiles_for_type_profile_instance(actual: &Value) -> Vec<String> {
    let from_meta = declared_profiles_on_value(actual);
    if !from_meta.is_empty() {
        return from_meta;
    }
    let Value::Object(map) = actual else {
        return Vec::new();
    };
    map.get("url")
        .and_then(|v| v.as_str())
        .map(|url| vec![url.to_owned()])
        .unwrap_or_default()
}

/// Extract `resourceType` from a JSON object value, if present.
fn resource_type_name_from_value(value: &Value) -> Option<&str> {
    let Value::Object(map) = value else {
        return None;
    };

    map.get("resourceType")?.as_str()
}

/// Resolve the resource type associated with a profile URL, using the profile
/// registry first and then falling back to heuristic URL-based mapping.
fn profile_resource_type(url: &str, profile_registry: Option<&ProfileRegistry>) -> Option<String> {
    if let Some(registry) = profile_registry {
        if let Some(profile) = registry.get(url) {
            return Some(profile.resource_type.clone());
        }
    }

    target_profile_resource_type(url)
}

/// Validate every canonical URL in `meta.profile` using the async profile pipeline.
///
/// Each URL resolved via [`crate::validation_context::AsyncValidationContext::runtime_profile_registry`]
/// is validated with [`validate_profile_async`]. Missing registry entries produce `not-found`
/// [`ValidationIssue`]s at `{resourceType}.meta.profile`.
pub async fn validate_declared_profiles_async<T: Serialize>(
    ctx: &AsyncValidationContext<'_>,
    state: &mut ValidationState,
    resource: &T,
    resource_type: &str,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let declared_profiles = declared_profile_urls(resource);
    for profile_url in declared_profiles {
        match ctx
            .runtime_profile_registry
            .and_then(|registry| registry.get(&profile_url))
        {
            Some(profile) => {
                issues.extend(
                    validate_profile_async(ctx, state, resource, resource_type, profile).await,
                );
            }
            None => {
                issues.push(ValidationIssue {
                    severity: crate::Severity::Error,
                    code: "not-found".to_string(),
                    summary: Some(
                        "Declared meta.profile URL is not available in the profile registry"
                            .to_string(),
                    ),
                    expression_kind: None,
                    source_invariant_key: None,
                    detail_code: Some(ValidationIssueDetailCode::ReferenceNotFound),
                    diagnostics: format!(
                        "Declared profile '{}' was not found in the profile registry.",
                        profile_url
                    ),
                    expression: Some(profile_url.clone()),
                    fhir_path: format!("{}.meta.profile", resource_type),
                    instance_path: Some(format!("{}.meta.profile", resource_type)),
                });
            }
        }
    }

    issues
}

/// Validate every canonical URL in `meta.profile` using the synchronous profile pipeline.
///
/// Each URL resolved via [`crate::validation_context::ValidationContext::runtime_profile_registry`]
/// is validated with [`validate_profile`]. Missing registry entries produce `not-found`
/// [`ValidationIssue`]s at `{resourceType}.meta.profile`.
pub fn validate_declared_profiles<T: Serialize>(
    ctx: &ValidationContext<'_>,
    state: &mut ValidationState,
    resource: &T,
    resource_type: &str,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let declared_profiles = declared_profile_urls(resource);
    for profile_url in declared_profiles {
        match ctx
            .runtime_profile_registry
            .and_then(|registry| registry.get(&profile_url))
        {
            Some(profile) => {
                issues.extend(validate_profile(
                    ctx,
                    state,
                    resource,
                    resource_type,
                    profile,
                ));
            }
            None => {
                issues.push(ValidationIssue {
                    severity: crate::Severity::Error,
                    code: "not-found".to_string(),
                    summary: Some(
                        "Declared meta.profile URL is not available in the profile registry"
                            .to_string(),
                    ),
                    expression_kind: None,
                    source_invariant_key: None,
                    detail_code: Some(ValidationIssueDetailCode::ReferenceNotFound),
                    diagnostics: format!(
                        "Declared profile '{}' was not found in the profile registry.",
                        profile_url
                    ),
                    expression: Some(profile_url.clone()),
                    fhir_path: format!("{}.meta.profile", resource_type),
                    instance_path: Some(format!("{}.meta.profile", resource_type)),
                });
            }
        }
    }

    issues
}

/// Serialize a resource and extract all declared `meta.profile` URLs.
fn declared_profile_urls<T: Serialize>(resource: &T) -> Vec<String> {
    let value = match serde_json::to_value(resource) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };

    extract_declared_profile_urls(&value)
}

/// Extract all declared `meta.profile` URLs from a serialized JSON value.
fn extract_declared_profile_urls(value: &Value) -> Vec<String> {
    let Some(meta) = value.get("meta") else {
        return Vec::new();
    };

    let Some(profile) = meta.get("profile") else {
        return Vec::new();
    };

    let Some(items) = profile.as_array() else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| item.as_str().map(str::to_owned))
        .collect()
}

/// Validate fixed/pattern constraints extracted from the profile differential.
fn validate_value_constraints<T: Serialize>(
    resource: &T,
    resource_type: &str,
    rules: &[crate::profile::types::ExtractedElementRule],
) -> Vec<ValidationIssue> {
    let root = match serde_json::to_value(resource) {
        Ok(value) => value,
        Err(err) => {
            return vec![ValidationIssue {
                severity: crate::Severity::Error,
                code: "processing".to_string(),
                summary: Some(
                    "Resource could not be serialized for fixed/pattern validation".to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::ValidationException),
                diagnostics: format!(
                    "Failed to serialize resource while validating fixed/pattern constraints: {}",
                    err
                ),
                expression: None,
                fhir_path: "".to_string(),
                instance_path: None,
            }];
        }
    };

    let mut issues = Vec::new();

    for rule in rules {
        let Some(value_constraint) = &rule.value_constraint else {
            continue;
        };

        let Some(relative_path) = relative_profile_path(resource_type, &rule.path) else {
            continue;
        };

        let actual = get_relative_path(&root, relative_path);
        let matched = match value_constraint {
            ExtractedValueConstraint::Fixed(expected) => actual
                .map(|value| values_equal(value, expected))
                .unwrap_or(false),
            ExtractedValueConstraint::Pattern(expected) => actual
                .map(|value| value_matches_pattern(value, expected))
                .unwrap_or(false),
        };

        if matched {
            continue;
        }
        let (kind, expected, detail_code, summary) = match value_constraint {
            ExtractedValueConstraint::Fixed(expected) => (
                "fixed",
                expected,
                Some(ValidationIssueDetailCode::FixedConstraintMismatch),
                Some("Element value does not match fixed constraint".to_string()),
            ),
            ExtractedValueConstraint::Pattern(expected) => (
                "pattern",
                expected,
                Some(ValidationIssueDetailCode::PatternConstraintMismatch),
                Some("Element value does not match pattern constraint".to_string()),
            ),
        };

        issues.push(ValidationIssue {
            severity: crate::Severity::Error,
            code: issue_code::VALUE.to_string(),
            summary,
            expression_kind: None,
            source_invariant_key: None,
            detail_code,
            diagnostics: format!(
                "Element '{}' does not satisfy {} constraint. Expected pattern/value: {}",
                rule.path, kind, expected
            ),
            expression: None,
            fhir_path: rule.path.clone(),
            instance_path: Some(rule.path.clone()),
        });
    }

    issues
}

/// Resolve a single non-repeating relative dotted path from the given root.
fn get_relative_path<'a>(root: &'a Value, relative_path: &str) -> Option<&'a Value> {
    if relative_path.is_empty() {
        return Some(root);
    }

    let mut current = root;
    for segment in relative_path.split('.') {
        match current {
            Value::Object(map) => {
                current = map.get(segment)?;
            }
            _ => return None,
        }
    }

    Some(current)
}

/// Compare an actual JSON value with an extracted fixed JSON value.
fn values_equal(actual: &Value, expected: &Value) -> bool {
    actual == expected
}

/// Compare an actual JSON value against an extracted pattern JSON value.
///
/// Objects are matched by subset semantics, while arrays are matched by checking
/// that each pattern element matches at least one actual array element.
fn value_matches_pattern(actual: &Value, pattern: &Value) -> bool {
    match (actual, pattern) {
        (Value::Object(actual_map), Value::Object(pattern_map)) => {
            pattern_map.iter().all(|(key, pattern_value)| {
                actual_map
                    .get(key)
                    .map(|actual_value| value_matches_pattern(actual_value, pattern_value))
                    .unwrap_or(false)
            })
        }
        (Value::Array(actual_items), Value::Array(pattern_items)) => {
            if pattern_items.is_empty() {
                return true;
            }

            pattern_items.iter().all(|pattern_item| {
                actual_items
                    .iter()
                    .any(|actual_item| value_matches_pattern(actual_item, pattern_item))
            })
        }
        _ => actual == pattern,
    }
}

#[cfg(test)]
mod reference_aggregate_versioning_tests {
    use super::*;
    use crate::profile::types::ExtractedElementRule;
    use serde_json::json;

    fn rule(path: &str, tc: ExtractedTypeConstraint) -> ExtractedElementRule {
        ExtractedElementRule {
            id: path.to_string(),
            path: path.to_string(),
            type_constraints: vec![tc],
            ..Default::default()
        }
    }

    #[test]
    fn aggregation_referenced_only_rejects_contained_reference() {
        let resource = json!({
            "resourceType": "Patient",
            "id": "x",
            "generalPractitioner": { "reference": "#pr1" },
            "contained": [{ "resourceType": "Practitioner", "id": "pr1" }]
        });
        let issues = validate_reference_aggregate_versioning_constraints(
            &resource,
            "Patient",
            &[rule(
                "Patient.generalPractitioner",
                ExtractedTypeConstraint {
                    code: "Reference".into(),
                    aggregation: vec!["referenced".into()],
                    ..Default::default()
                },
            )],
        );
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn aggregation_referenced_only_accepts_literal_reference() {
        let resource = json!({
            "resourceType": "Patient",
            "generalPractitioner": { "reference": "Practitioner/1" }
        });
        let issues = validate_reference_aggregate_versioning_constraints(
            &resource,
            "Patient",
            &[rule(
                "Patient.generalPractitioner",
                ExtractedTypeConstraint {
                    code: "Reference".into(),
                    aggregation: vec!["referenced".into()],
                    ..Default::default()
                },
            )],
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn versioning_independent_rejects_history_url() {
        let resource = json!({
            "resourceType": "Patient",
            "generalPractitioner": { "reference": "Practitioner/1/_history/2" }
        });
        let issues = validate_reference_aggregate_versioning_constraints(
            &resource,
            "Patient",
            &[rule(
                "Patient.generalPractitioner",
                ExtractedTypeConstraint {
                    code: "Reference".into(),
                    versioning: Some("independent".into()),
                    ..Default::default()
                },
            )],
        );
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn versioning_specific_requires_history_segment() {
        let resource = json!({
            "resourceType": "Patient",
            "generalPractitioner": { "reference": "Practitioner/1" }
        });
        let issues = validate_reference_aggregate_versioning_constraints(
            &resource,
            "Patient",
            &[rule(
                "Patient.generalPractitioner",
                ExtractedTypeConstraint {
                    code: "Reference".into(),
                    versioning: Some("specific".into()),
                    ..Default::default()
                },
            )],
        );
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn type_rows_combine_with_or() {
        let resource = json!({
            "resourceType": "Patient",
            "generalPractitioner": { "reference": "#pr1" },
            "contained": [{ "resourceType": "Practitioner", "id": "pr1" }]
        });
        let issues = validate_reference_aggregate_versioning_constraints(
            &resource,
            "Patient",
            &[ExtractedElementRule {
                id: "Patient.generalPractitioner".into(),
                path: "Patient.generalPractitioner".into(),
                type_constraints: vec![
                    ExtractedTypeConstraint {
                        code: "Reference".into(),
                        aggregation: vec!["referenced".into()],
                        ..Default::default()
                    },
                    ExtractedTypeConstraint {
                        code: "Reference".into(),
                        aggregation: vec!["contained".into()],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn canonical_versioning_specific_requires_pipe_version() {
        let resource = json!({
            "resourceType": "StructureDefinition",
            "url": "http://example.org/foo"
        });
        let issues = validate_reference_aggregate_versioning_constraints(
            &resource,
            "StructureDefinition",
            &[rule(
                "StructureDefinition.url",
                ExtractedTypeConstraint {
                    code: "canonical".into(),
                    versioning: Some("specific".into()),
                    ..Default::default()
                },
            )],
        );
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn urn_on_non_bundle_counts_as_referenced_not_bundled() {
        let resource = json!({
            "resourceType": "Patient",
            "generalPractitioner": { "reference": "urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee" }
        });
        let issues = validate_reference_aggregate_versioning_constraints(
            &resource,
            "Patient",
            &[rule(
                "Patient.generalPractitioner",
                ExtractedTypeConstraint {
                    code: "Reference".into(),
                    aggregation: vec!["referenced".into()],
                    ..Default::default()
                },
            )],
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn bundled_only_requires_matching_bundle_entry() {
        let bundle = json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [
                {
                    "resource": {
                        "resourceType": "Observation",
                        "status": "final",
                        "code": { "text": "x" },
                        "subject": { "reference": "Patient/999" }
                    }
                }
            ]
        });
        let issues = validate_reference_aggregate_versioning_constraints(
            &bundle,
            "Bundle",
            &[rule(
                "Bundle.entry.resource.subject",
                ExtractedTypeConstraint {
                    code: "Reference".into(),
                    aggregation: vec!["bundled".into()],
                    ..Default::default()
                },
            )],
        );
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn bundled_aggregation_passes_when_reference_matches_bundle_full_url() {
        let bundle = json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [
                {
                    "fullUrl": "urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                    "resource": {
                        "resourceType": "Patient",
                        "id": "p1",
                        "name": [{ "family": "Test" }]
                    }
                },
                {
                    "resource": {
                        "resourceType": "Observation",
                        "status": "final",
                        "code": { "text": "x" },
                        "subject": { "reference": "urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee" }
                    }
                }
            ]
        });
        let issues = validate_reference_aggregate_versioning_constraints(
            &bundle,
            "Bundle",
            &[rule(
                "Bundle.entry.resource.subject",
                ExtractedTypeConstraint {
                    code: "Reference".into(),
                    aggregation: vec!["bundled".into()],
                    ..Default::default()
                },
            )],
        );
        assert!(issues.is_empty());
    }
}
