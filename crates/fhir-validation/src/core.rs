//! Core validation engine for the `fhir-validation` crate.
//!
//! This module defines the shared validator types and orchestration layer used by
//! all version-specific validation code.
//!
//! Responsibilities:
//! - represent validation issues in a form that can later be mapped to
//!   `OperationOutcome.issue`
//! - provide configurable severity handling for binding misses
//! - dispatch validation by FHIR version (`R4`, `R5`, etc.)
//! - combine resource-level bindings and invariants into a single issue list
//! - provide shared helpers for rebasing instance paths and converting JSON into
//!   evaluation results for FHIRPath-based invariant evaluation
//!
//! Binding path resolution and typed terminology dispatch now live in the
//! version-specific binding modules (`r4/binding.rs`, `r5/binding.rs`)
//!
//! Version-specific logic such as binding extraction and terminology dispatch is
//! implemented in `r4/*`, `r5/*`, and generated validation modules.

pub use fhir_validation_types::{
    BindingDef, BindingStrength, BindingTargetKind, InvariantDef, Severity,
    StructureDefinitionKind, TypeDerivationRule,
};

use crate::error::ValidationError;
use crate::issue_code;
use crate::terminology::service::{TerminologyService, TerminologyServiceSync};
use crate::validation_issue_detail::{ValidationIssueDetailCode, ValidationSourceKind};
use crate::{FhirPathEvaluator, InvariantExprRef};

use crate::profile::cardinality;
use crate::profile::profile_registry::ProfileRegistry;
use crate::strict_properties;
use helios_fhirpath::handlers::json_value_to_evaluation_result;
use std::collections::HashMap;
use tracing::debug;

#[cfg(feature = "R4")]
use crate::r4::{
    validate_r4_resource, validate_r4_resource_async, validate_r4_resource_async_with_profiles,
    validate_r4_resource_async_with_profiles_and_validation_addons,
    validate_r4_resource_async_with_validation_addons, validate_r4_resource_with_profiles,
    validate_r4_resource_with_profiles_and_validation_addons,
    validate_r4_resource_with_validation_addons,
};
#[cfg(feature = "R4B")]
use crate::r4b::{
    validate_r4b_resource, validate_r4b_resource_async, validate_r4b_resource_async_with_profiles,
    validate_r4b_resource_async_with_profiles_and_validation_addons,
    validate_r4b_resource_async_with_validation_addons, validate_r4b_resource_with_profiles,
    validate_r4b_resource_with_profiles_and_validation_addons,
    validate_r4b_resource_with_validation_addons,
};
#[cfg(feature = "R5")]
use crate::r5::{
    validate_r5_resource, validate_r5_resource_async, validate_r5_resource_async_with_profiles,
    validate_r5_resource_async_with_profiles_and_validation_addons,
    validate_r5_resource_async_with_validation_addons, validate_r5_resource_with_profiles,
    validate_r5_resource_with_profiles_and_validation_addons,
    validate_r5_resource_with_validation_addons,
};
#[cfg(feature = "R6")]
use crate::r6::{
    validate_r6_resource, validate_r6_resource_async, validate_r6_resource_async_with_profiles,
    validate_r6_resource_async_with_profiles_and_validation_addons,
    validate_r6_resource_async_with_validation_addons, validate_r6_resource_with_profiles,
    validate_r6_resource_with_profiles_and_validation_addons,
    validate_r6_resource_with_validation_addons,
};

/// A single validation issue that can later be mapped to
/// `OperationOutcome.issue`.
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    pub severity: Severity,

    /// Internal validator category; see [`crate::issue_code`] for shared string constants.
    /// This is later mapped conservatively to FHIR `OperationOutcome.issue.code`.
    pub code: String,

    /// Declared logical FHIR path from the StructureDefinition metadata,
    /// for example "Reference" or "Patient.contact".
    pub fhir_path: String,

    /// Concrete instance path in the validated resource, for example
    /// "Patient.managingOrganization" or "Patient.identifier[0].assigner".
    ///
    /// This is optional for now because not all call sites/threaded recursion
    /// paths have been updated yet.
    pub instance_path: Option<String>,

    /// Optional expression (FHIRPath, ValueSet URL, etc.); see [`Self::expression_kind`].
    pub expression: Option<String>,

    /// When set, overrides heuristic classification of [`Self::expression`] when projecting
    /// to OperationOutcome source extensions (`canonical-uri`, `fhirpath`, …).
    pub expression_kind: Option<ValidationSourceKind>,

    /// Constraint / invariant key from metadata (e.g. [`InvariantDef::key`]), independent of
    /// [`Self::expression`] (which often holds the FHIRPath).
    pub source_invariant_key: Option<String>,

    /// Short headline for `OperationOutcome.issue.details.text` (e.g. UI, `$validate`).
    /// When absent, the issue-to-OperationOutcome mapper synthesizes text from
    /// [`Self::detail_code`] or [`Self::code`].
    pub summary: Option<String>,

    /// Optional fine-grained validation detail code for `details.coding`.
    /// When absent, [`ValidationIssueDetailCode::from_issue_category`] is used from [`Self::code`].
    pub detail_code: Option<ValidationIssueDetailCode>,

    /// Human-readable technical diagnostics for logs, debugging, and
    /// `OperationOutcome.issue.diagnostics`.
    pub diagnostics: String,
}

impl ValidationIssue {
    /// Construct an error-level validation issue with a declared FHIR path.
    pub fn error(
        code: impl Into<String>,
        path: impl Into<String>,
        diag: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Error,
            code: code.into(),
            fhir_path: path.into(),
            instance_path: None,
            expression: None,
            expression_kind: None,
            source_invariant_key: None,
            summary: None,
            detail_code: None,
            diagnostics: diag.into(),
        }
    }
    /// Construct a warning-level validation issue with a declared FHIR path.
    pub fn warning(
        code: impl Into<String>,
        path: impl Into<String>,
        diag: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Warning,
            code: code.into(),
            fhir_path: path.into(),
            instance_path: None,
            expression: None,
            expression_kind: None,
            source_invariant_key: None,
            summary: None,
            detail_code: None,
            diagnostics: diag.into(),
        }
    }
    /// Convert a generated invariant definition into a validation issue for a failed check.
    pub fn from_invariant_def(invariant: &InvariantDef) -> Self {
        Self {
            severity: invariant.severity,
            code: issue_code::INVARIANT.to_string(),
            fhir_path: invariant.path.to_string(),
            instance_path: None,
            expression: Some(invariant.expression.to_string()),
            expression_kind: Some(ValidationSourceKind::FhirPath),
            source_invariant_key: Some(invariant.key.to_string()),
            summary: Some("Value does not satisfy an invariant constraint".to_string()),
            detail_code: None,
            diagnostics: if invariant.human.is_empty() {
                format!("Constraint failed: {}", invariant.key)
            } else {
                format!(
                    "Constraint failed: {}: '{}'",
                    invariant.key, invariant.human
                )
            },
        }
    }
    /// Convert an invariant evaluation failure into an exception-style validation issue
    pub fn from_invariant_error(invariant: &InvariantDef, err: ValidationError) -> Self {
        Self {
            severity: Severity::Error,
            code: issue_code::EXCEPTION.to_string(),
            fhir_path: invariant.path.to_string(),
            instance_path: None,
            expression: Some(invariant.expression.to_string()),
            expression_kind: Some(ValidationSourceKind::FhirPath),
            source_invariant_key: Some(invariant.key.to_string()),
            summary: Some("Invariant expression evaluation failed".to_string()),
            detail_code: None,
            diagnostics: if invariant.human.is_empty() {
                format!("Constraint evaluation error: {}: {err}", invariant.key)
            } else {
                format!(
                    "Constraint evaluation error: {}: '{}': {err}",
                    invariant.key, invariant.human
                )
            },
        }
    }
    /// Attach a concrete instance path to this issue.
    pub fn with_instance_path(mut self, instance_path: impl Into<String>) -> Self {
        self.instance_path = Some(instance_path.into());
        self
    }

    /// Attach a short summary for OperationOutcome `details.text`.
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    /// Detail code for OperationOutcome `details.coding`, using [`Self::detail_code`] or
    /// [`ValidationIssueDetailCode::from_issue_category`] on [`Self::code`].
    pub fn resolved_detail_code(&self) -> ValidationIssueDetailCode {
        self.detail_code
            .unwrap_or_else(|| ValidationIssueDetailCode::from_issue_category(&self.code))
    }
}
/// Configuration for validation behavior.
#[derive(Debug, Clone)]
pub struct ValidationConfig {
    /// Treat extensible bindings as errors
    pub strict_extensible_bindings: bool,

    /// Emit warnings for preferred bindings
    pub warn_on_preferred_bindings: bool,

    /// Emit warnings for example bindings
    pub warn_on_example_bindings: bool,

    /// Emit verbose internal traces (intended for development; off by default).
    pub debug_trace: bool,

    /// Maximum recursion depth for nested `type.profile` validation.
    pub max_profile_recursion_depth: usize,

    /// When a profile recursion cycle is detected, emit a warning issue.
    /// When false, cycles are silently skipped.
    pub warn_on_profile_cycle: bool,

    /// When the maximum recursion depth is reached, emit a warning issue.
    /// When false, depth overflow is silently skipped.
    pub warn_on_profile_recursion_depth_reached: bool,

    /// Allow `type.profile` validation to fall back to matching by `resourceType`
    /// when `meta.profile` is absent.
    pub allow_type_profile_resource_type_fallback: bool,

    /// Emit a warning when `type.profile` succeeds only by `resourceType` fallback.
    pub warn_on_type_profile_fallback: bool,

    /// When `type.profile` succeeds by `resourceType` fallback, recursively validate
    /// against the matched profile(s).
    pub recurse_on_type_profile_fallback: bool,

    pub warn_on_unknown_profile: bool,
    pub error_on_unknown_profile: bool,
    /// Recursively validate against `StructureDefinition.baseDefinition` when it
    /// points to a non-core profile.
    pub recurse_on_base_definition: bool,
    /// Allow HTTP(S) lookup of base profiles not present in the runtime
    /// `ProfileRegistry`.
    pub enable_base_definition_url_lookup: bool,
    /// Network timeout used for base profile URL lookups.
    pub base_definition_url_lookup_timeout_ms: u64,
    /// Maximum accepted payload size (bytes) for fetched base profiles.
    pub base_definition_url_lookup_max_bytes: usize,
    /// Optional hostname allowlist for remote base profile lookups.
    ///
    /// - Empty list means "allow any host" (backward compatible).
    /// - Non-empty list allows exact host match or subdomain match.
    // cfg.base_definition_url_lookup_allowed_hosts = vec![
    //     "hl7.org".to_string(),
    //     "nrces.in".to_string(),
    // ];
    pub base_definition_url_lookup_allowed_hosts: Vec<String>,

    pub type_profile_match_mode: TypeProfileMatchMode,

    /// When true, emit an issue for elements marked `mustSupport` in the profile
    /// when the instance has no value at that path (see `must_support_missing_severity`).
    pub validate_must_support: bool,

    /// Severity when a `mustSupport` element has no instance values (`validate_must_support`).
    pub must_support_missing_severity: Severity,

    /// When true, reject JSON property names not allowed by a **base** extracted profile loaded in
    /// the runtime [`ProfileRegistry`] (see [`strict_properties::validate_json_against_extracted_profile`]).
    pub strict_json_properties: bool,

    /// When true, enforce min/max cardinality from the **base** snapshot loaded for the resource
    /// type (same registry lookup as [`Self::strict_json_properties`]).
    pub validate_base_snapshot_cardinality: bool,

    /// Map `resourceType` → canonical `StructureDefinition.url` key used in [`ProfileRegistry`]
    /// when it differs from `http://hl7.org/fhir/StructureDefinition/{type}`.
    pub base_structure_definition_url_overrides: HashMap<String, String>,

    /// Emit a warning when [`Self::strict_json_properties`] or
    /// [`Self::validate_base_snapshot_cardinality`] is enabled but no matching base profile exists
    /// in the registry (instead of silently skipping add-ons).
    pub warn_on_missing_base_profile_for_validation_addons: bool,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            strict_extensible_bindings: true,
            warn_on_preferred_bindings: true,
            warn_on_example_bindings: true,
            debug_trace: false,
            max_profile_recursion_depth: 3,
            warn_on_profile_cycle: true,
            warn_on_profile_recursion_depth_reached: true,
            allow_type_profile_resource_type_fallback: true,
            warn_on_type_profile_fallback: true,
            recurse_on_type_profile_fallback: true,
            warn_on_unknown_profile: false,
            error_on_unknown_profile: true,
            recurse_on_base_definition: true,
            enable_base_definition_url_lookup: true,
            base_definition_url_lookup_timeout_ms: 2_000,
            base_definition_url_lookup_max_bytes: 1_000_000,
            base_definition_url_lookup_allowed_hosts: Vec::new(),
            type_profile_match_mode: TypeProfileMatchMode::Any,
            validate_must_support: true,
            must_support_missing_severity: Severity::Warning,
            strict_json_properties: true,
            validate_base_snapshot_cardinality: true,
            base_structure_definition_url_overrides: HashMap::new(),
            warn_on_missing_base_profile_for_validation_addons: true,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub enum TypeProfileMatchMode {
    Any, // OR (current behavior)
    All, // AND (strict)
}

/// Shared validator entry point used by generated and handwritten validation code.
#[derive(Debug, Clone, Default)]
pub struct Validator {
    pub config: ValidationConfig,
}

impl Validator {
    /// Create a validator with the provided configuration
    pub fn new(config: ValidationConfig) -> Self {
        Self { config }
    }
    /// Map a binding strength to the severity that should be emitted when a value
    /// is not in the bound ValueSet.
    pub fn binding_miss_severity(&self, strength: BindingStrength) -> Option<Severity> {
        match strength {
            BindingStrength::Required => Some(Severity::Error),
            BindingStrength::Extensible => Some(if self.config.strict_extensible_bindings {
                Severity::Error
            } else {
                Severity::Warning
            }),
            BindingStrength::Preferred => Some(if self.config.warn_on_preferred_bindings {
                Severity::Warning
            } else {
                Severity::Information
            }),
            BindingStrength::Example => Some(if self.config.warn_on_example_bindings {
                Severity::Warning
            } else {
                Severity::Information
            }),
        }
    }
    /// Validate a versioned `FhirResource` by dispatching to the appropriate
    /// version-specific resource validator.
    pub fn validate_resource(
        &self,
        resource: &helios_fhir::FhirResource,
        terminology: Option<&dyn TerminologyServiceSync>,
        evaluator: &dyn FhirPathEvaluator,
    ) -> Vec<ValidationIssue> {
        match resource {
            #[cfg(feature = "R4")]
            helios_fhir::FhirResource::R4(res) => {
                validate_r4_resource(self, res.as_ref(), terminology, evaluator)
            }

            #[cfg(feature = "R4B")]
            helios_fhir::FhirResource::R4B(res) => {
                validate_r4b_resource(self, res.as_ref(), terminology, evaluator)
            }

            #[cfg(feature = "R5")]
            helios_fhir::FhirResource::R5(res) => {
                validate_r5_resource(self, res.as_ref(), terminology, evaluator)
            }

            #[cfg(feature = "R6")]
            helios_fhir::FhirResource::R6(res) => {
                validate_r6_resource(self, res.as_ref(), terminology, evaluator)
            }
        }
    }
    pub async fn validate_resource_async(
        &self,
        resource: &helios_fhir::FhirResource,
        terminology: Option<&dyn TerminologyService>,
        evaluator: &dyn FhirPathEvaluator,
    ) -> Vec<ValidationIssue> {
        match resource {
            #[cfg(feature = "R4")]
            helios_fhir::FhirResource::R4(res) => {
                validate_r4_resource_async(self, res.as_ref(), terminology, evaluator).await
            }

            #[cfg(feature = "R4B")]
            helios_fhir::FhirResource::R4B(res) => {
                validate_r4b_resource_async(self, res.as_ref(), terminology, evaluator).await
            }

            #[cfg(feature = "R5")]
            helios_fhir::FhirResource::R5(res) => {
                validate_r5_resource_async(self, res.as_ref(), terminology, evaluator).await
            }

            #[cfg(feature = "R6")]
            helios_fhir::FhirResource::R6(res) => {
                validate_r6_resource_async(self, res.as_ref(), terminology, evaluator).await
            }
        }
    }

    pub fn validate_resource_with_profiles(
        &self,
        resource: &helios_fhir::FhirResource,
        terminology: Option<&dyn TerminologyServiceSync>,
        evaluator: &dyn FhirPathEvaluator,
        profile_registry: &ProfileRegistry,
    ) -> Vec<ValidationIssue> {
        match resource {
            #[cfg(feature = "R4")]
            helios_fhir::FhirResource::R4(res) => validate_r4_resource_with_profiles(
                self,
                res.as_ref(),
                terminology,
                evaluator,
                profile_registry,
            ),

            #[cfg(feature = "R4B")]
            helios_fhir::FhirResource::R4B(res) => validate_r4b_resource_with_profiles(
                self,
                res.as_ref(),
                terminology,
                evaluator,
                profile_registry,
            ),

            #[cfg(feature = "R5")]
            helios_fhir::FhirResource::R5(res) => validate_r5_resource_with_profiles(
                self,
                res.as_ref(),
                terminology,
                evaluator,
                profile_registry,
            ),

            #[cfg(feature = "R6")]
            helios_fhir::FhirResource::R6(res) => validate_r6_resource_with_profiles(
                self,
                res.as_ref(),
                terminology,
                evaluator,
                profile_registry,
            ),
        }
    }

    pub async fn validate_resource_with_profiles_async(
        &self,
        resource: &helios_fhir::FhirResource,
        terminology: Option<&dyn TerminologyService>,
        evaluator: &dyn FhirPathEvaluator,
        profile_registry: &ProfileRegistry,
    ) -> Vec<ValidationIssue> {
        match resource {
            #[cfg(feature = "R4")]
            helios_fhir::FhirResource::R4(res) => {
                validate_r4_resource_async_with_profiles(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
                .await
            }

            #[cfg(feature = "R4B")]
            helios_fhir::FhirResource::R4B(res) => {
                validate_r4b_resource_async_with_profiles(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
                .await
            }

            #[cfg(feature = "R5")]
            helios_fhir::FhirResource::R5(res) => {
                validate_r5_resource_async_with_profiles(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
                .await
            }

            #[cfg(feature = "R6")]
            helios_fhir::FhirResource::R6(res) => {
                validate_r6_resource_async_with_profiles(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
                .await
            }
        }
    }

    /// Structural add-ons (strict JSON keys, base snapshot cardinality) using a [`ProfileRegistry`]
    /// that includes the **base** `StructureDefinition` for `resource_type`.
    pub fn apply_validation_addons<T: serde::Serialize>(
        &self,
        resource: &T,
        resource_type: &str,
        profile_registry: &ProfileRegistry,
    ) -> Vec<ValidationIssue> {
        let cfg = &self.config;
        if !cfg.strict_json_properties && !cfg.validate_base_snapshot_cardinality {
            return Vec::new();
        }

        let base = strict_properties::resolve_base_profile_in_registry(
            resource_type,
            profile_registry,
            &cfg.base_structure_definition_url_overrides,
        );
        if base.is_none() {
            let want = cfg.strict_json_properties || cfg.validate_base_snapshot_cardinality;
            if want && cfg.warn_on_missing_base_profile_for_validation_addons {
                return vec![ValidationIssue::warning(
                    "incomplete",
                    resource_type,
                    format!(
                        "Validation add-ons require a base StructureDefinition for '{resource_type}' in the \
                         ProfileRegistry (HL7 canonical URL or base_structure_definition_url_overrides)."
                    ),
                )
                .with_summary(
                    "Base profile missing for strict JSON / snapshot cardinality validation",
                )];
            }
            return Vec::new();
        }
        let base = base.expect("checked");

        let mut issues = Vec::new();
        if cfg.strict_json_properties {
            let root = match serde_json::to_value(resource) {
                Ok(v) => v,
                Err(err) => {
                    return vec![
                        ValidationIssue::error(
                            "processing",
                            resource_type,
                            format!(
                                "Could not serialize resource for strict JSON validation: {err}"
                            ),
                        )
                        .with_summary("Serialization failed during strict property validation"),
                    ];
                }
            };
            issues.extend(strict_properties::validate_json_against_extracted_profile(
                &root,
                base,
                Some(profile_registry),
            ));
        }
        if cfg.validate_base_snapshot_cardinality {
            issues.extend(cardinality::validate_min_cardinality(
                resource,
                resource_type,
                &base.element_rules,
            ));
            issues.extend(cardinality::validate_max_cardinality(
                resource,
                resource_type,
                &base.element_rules,
            ));
        }
        issues
    }

    pub fn validate_resource_with_validation_addons(
        &self,
        resource: &helios_fhir::FhirResource,
        terminology: Option<&dyn TerminologyServiceSync>,
        evaluator: &dyn FhirPathEvaluator,
        profile_registry: &ProfileRegistry,
    ) -> Vec<ValidationIssue> {
        match resource {
            #[cfg(feature = "R4")]
            helios_fhir::FhirResource::R4(res) => validate_r4_resource_with_validation_addons(
                self,
                res.as_ref(),
                terminology,
                evaluator,
                profile_registry,
            ),

            #[cfg(feature = "R4B")]
            helios_fhir::FhirResource::R4B(res) => validate_r4b_resource_with_validation_addons(
                self,
                res.as_ref(),
                terminology,
                evaluator,
                profile_registry,
            ),

            #[cfg(feature = "R5")]
            helios_fhir::FhirResource::R5(res) => validate_r5_resource_with_validation_addons(
                self,
                res.as_ref(),
                terminology,
                evaluator,
                profile_registry,
            ),

            #[cfg(feature = "R6")]
            helios_fhir::FhirResource::R6(res) => validate_r6_resource_with_validation_addons(
                self,
                res.as_ref(),
                terminology,
                evaluator,
                profile_registry,
            ),
        }
    }

    pub fn validate_resource_with_profiles_and_validation_addons(
        &self,
        resource: &helios_fhir::FhirResource,
        terminology: Option<&dyn TerminologyServiceSync>,
        evaluator: &dyn FhirPathEvaluator,
        profile_registry: &ProfileRegistry,
    ) -> Vec<ValidationIssue> {
        match resource {
            #[cfg(feature = "R4")]
            helios_fhir::FhirResource::R4(res) => {
                validate_r4_resource_with_profiles_and_validation_addons(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
            }

            #[cfg(feature = "R4B")]
            helios_fhir::FhirResource::R4B(res) => {
                validate_r4b_resource_with_profiles_and_validation_addons(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
            }

            #[cfg(feature = "R5")]
            helios_fhir::FhirResource::R5(res) => {
                validate_r5_resource_with_profiles_and_validation_addons(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
            }

            #[cfg(feature = "R6")]
            helios_fhir::FhirResource::R6(res) => {
                validate_r6_resource_with_profiles_and_validation_addons(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
            }
        }
    }

    pub async fn validate_resource_async_with_validation_addons(
        &self,
        resource: &helios_fhir::FhirResource,
        terminology: Option<&dyn TerminologyService>,
        evaluator: &dyn FhirPathEvaluator,
        profile_registry: &ProfileRegistry,
    ) -> Vec<ValidationIssue> {
        match resource {
            #[cfg(feature = "R4")]
            helios_fhir::FhirResource::R4(res) => {
                validate_r4_resource_async_with_validation_addons(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
                .await
            }

            #[cfg(feature = "R4B")]
            helios_fhir::FhirResource::R4B(res) => {
                validate_r4b_resource_async_with_validation_addons(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
                .await
            }

            #[cfg(feature = "R5")]
            helios_fhir::FhirResource::R5(res) => {
                validate_r5_resource_async_with_validation_addons(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
                .await
            }

            #[cfg(feature = "R6")]
            helios_fhir::FhirResource::R6(res) => {
                validate_r6_resource_async_with_validation_addons(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
                .await
            }
        }
    }

    pub async fn validate_resource_async_with_profiles_and_validation_addons(
        &self,
        resource: &helios_fhir::FhirResource,
        terminology: Option<&dyn TerminologyService>,
        evaluator: &dyn FhirPathEvaluator,
        profile_registry: &ProfileRegistry,
    ) -> Vec<ValidationIssue> {
        match resource {
            #[cfg(feature = "R4")]
            helios_fhir::FhirResource::R4(res) => {
                validate_r4_resource_async_with_profiles_and_validation_addons(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
                .await
            }

            #[cfg(feature = "R4B")]
            helios_fhir::FhirResource::R4B(res) => {
                validate_r4b_resource_async_with_profiles_and_validation_addons(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
                .await
            }

            #[cfg(feature = "R5")]
            helios_fhir::FhirResource::R5(res) => {
                validate_r5_resource_async_with_profiles_and_validation_addons(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
                .await
            }

            #[cfg(feature = "R6")]
            helios_fhir::FhirResource::R6(res) => {
                validate_r6_resource_async_with_profiles_and_validation_addons(
                    self,
                    res.as_ref(),
                    terminology,
                    evaluator,
                    profile_registry,
                )
                .await
            }
        }
    }

    #[cfg(any(feature = "R4", feature = "R5"))]
    pub fn apply_bindings_for_version_sync<T: serde::Serialize>(
        &self,
        fhir_version: helios_fhir::FhirVersion,
        focus: &T,
        bindings: &[BindingDef],
        terminology: Option<&dyn TerminologyServiceSync>,
    ) -> Vec<ValidationIssue> {
        match fhir_version {
            #[cfg(feature = "R4")]
            helios_fhir::FhirVersion::R4 => self.apply_r4_bindings(focus, bindings, terminology),
            #[cfg(feature = "R5")]
            helios_fhir::FhirVersion::R5 => self.apply_r5_bindings(focus, bindings, terminology),
            #[cfg(feature = "R4B")]
            helios_fhir::FhirVersion::R4B => self.apply_r4b_bindings(focus, bindings, terminology),
            #[cfg(feature = "R6")]
            helios_fhir::FhirVersion::R6 => self.apply_r6_bindings(focus, bindings, terminology),
        }
    }
    #[cfg(any(feature = "R4", feature = "R5"))]
    pub async fn apply_bindings_for_version_async<T: serde::Serialize>(
        &self,
        fhir_version: helios_fhir::FhirVersion,
        focus: &T,
        bindings: &[BindingDef],
        terminology: Option<&dyn TerminologyService>,
    ) -> Vec<ValidationIssue> {
        match fhir_version {
            #[cfg(feature = "R4")]
            helios_fhir::FhirVersion::R4 => {
                self.apply_r4_bindings_async(focus, bindings, terminology)
                    .await
            }
            #[cfg(feature = "R5")]
            helios_fhir::FhirVersion::R5 => {
                self.apply_r5_bindings_async(focus, bindings, terminology)
                    .await
            }
            #[cfg(feature = "R4B")]
            helios_fhir::FhirVersion::R4B => {
                self.apply_r4b_bindings_async(focus, bindings, terminology)
                    .await
            }
            #[cfg(feature = "R6")]
            helios_fhir::FhirVersion::R6 => {
                self.apply_r6_bindings_async(focus, bindings, terminology)
                    .await
            }
        }
    }
    /// Apply R4 binding definitions to the current focus value.
    ///
    /// This delegates to the R4 binding module, which resolves binding paths,
    /// validates primitive codes / `Coding` / `CodeableConcept`, and falls back
    /// to remote terminology when local validation is insufficient.
    #[cfg(feature = "R4")]
    pub fn apply_r4_bindings<T>(
        &self,
        focus: &T,
        bindings: &[BindingDef],
        terminology: Option<&dyn TerminologyServiceSync>,
    ) -> Vec<ValidationIssue>
    where
        T: serde::Serialize,
    {
        crate::r4::binding::apply_r4_bindings(self, focus, bindings, terminology)
    }

    /// Apply R4B binding definitions to the current focus value.
    ///
    /// This delegates to the R4B binding module, which resolves binding paths,
    /// validates primitive codes / `Coding` / `CodeableConcept`, and falls back
    /// to remote terminology when local validation is insufficient.
    #[cfg(feature = "R4B")]
    pub fn apply_r4b_bindings<T>(
        &self,
        focus: &T,
        bindings: &[BindingDef],
        terminology: Option<&dyn TerminologyServiceSync>,
    ) -> Vec<ValidationIssue>
    where
        T: serde::Serialize,
    {
        crate::r4b::binding::apply_r4b_bindings(self, focus, bindings, terminology)
    }

    /// Apply R5 binding definitions to the current focus value.
    ///
    /// This delegates to the R5 binding module, which resolves binding paths,
    /// validates primitive codes / `Coding` / `CodeableConcept`, and falls back
    /// to remote terminology when local validation is insufficient.
    #[cfg(feature = "R5")]
    pub fn apply_r5_bindings<T>(
        &self,
        focus: &T,
        bindings: &[BindingDef],
        terminology: Option<&dyn TerminologyServiceSync>,
    ) -> Vec<ValidationIssue>
    where
        T: serde::Serialize,
    {
        crate::r5::binding::apply_r5_bindings(self, focus, bindings, terminology)
    }

    /// Apply R6 binding definitions to the current focus value.
    ///
    /// This delegates to the R6 binding module, which resolves binding paths,
    /// validates primitive codes / `Coding` / `CodeableConcept`, and falls back
    /// to remote terminology when local validation is insufficient.
    #[cfg(feature = "R6")]
    pub fn apply_r6_bindings<T>(
        &self,
        focus: &T,
        bindings: &[BindingDef],
        terminology: Option<&dyn TerminologyServiceSync>,
    ) -> Vec<ValidationIssue>
    where
        T: serde::Serialize,
    {
        crate::r6::binding::apply_r6_bindings(self, focus, bindings, terminology)
    }

    #[cfg(feature = "R4")]
    pub async fn apply_r4_bindings_async<T>(
        &self,
        focus: &T,
        bindings: &[BindingDef],
        terminology: Option<&dyn TerminologyService>,
    ) -> Vec<ValidationIssue>
    where
        T: serde::Serialize,
    {
        crate::r4::binding::apply_r4_bindings_async(self, focus, bindings, terminology).await
    }

    #[cfg(feature = "R4B")]
    pub async fn apply_r4b_bindings_async<T>(
        &self,
        focus: &T,
        bindings: &[BindingDef],
        terminology: Option<&dyn TerminologyService>,
    ) -> Vec<ValidationIssue>
    where
        T: serde::Serialize,
    {
        crate::r4b::binding::apply_r4b_bindings_async(self, focus, bindings, terminology).await
    }

    #[cfg(feature = "R5")]
    pub async fn apply_r5_bindings_async<T>(
        &self,
        focus: &T,
        bindings: &[BindingDef],
        terminology: Option<&dyn TerminologyService>,
    ) -> Vec<ValidationIssue>
    where
        T: serde::Serialize,
    {
        crate::r5::binding::apply_r5_bindings_async(self, focus, bindings, terminology).await
    }

    #[cfg(feature = "R6")]
    pub async fn apply_r6_bindings_async<T>(
        &self,
        focus: &T,
        bindings: &[BindingDef],
        terminology: Option<&dyn TerminologyService>,
    ) -> Vec<ValidationIssue>
    where
        T: serde::Serialize,
    {
        crate::r6::binding::apply_r6_bindings_async(self, focus, bindings, terminology).await
    }
    /// Apply generated invariants to a focused value using the supplied
    /// `FhirPathEvaluator`.
    ///
    /// # Contract (bulk evaluation)
    ///
    /// `focus` is serialized to JSON and converted to a single FHIRPath root; **all** invariant
    /// expressions are evaluated with that same root as `$this`. This is efficient when many
    /// rules share one focus (e.g. validating `ele-1` on a nested datatype where `focus` is that
    /// datatype instance, or root profile rules where `focus` is the whole resource).
    ///
    /// Do **not** rely on this method to implement FHIR’s “context = element at
    /// `InvariantDef.path`” rule when `focus` is the **full resource** and paths point at nested
    /// elements: expressions written **relative to the element** need
    /// [`FhirPathEvaluator::eval_invariant`] instead (see [`crate::profile::validate`] for profile
    /// constraints).
    ///
    /// `InvariantExprRef::declared_path` is passed through to the evaluator for diagnostics; in
    /// the bulk path the evaluator does **not** use it to change `$this` (see
    /// [`FhirPathEvaluator::eval_invariants_on`](crate::FhirPathEvaluator::eval_invariants_on)).
    ///
    /// The `instance_root_path` is used to stamp concrete instance paths on all emitted issues
    /// so callers can map a failure back to the logical location in the validated resource.
    pub fn apply_invariants<T>(
        &self,
        focus: &T,
        invariants: &[fhir_validation_types::InvariantDef],
        evaluator: &dyn FhirPathEvaluator,
        instance_root_path: &str,
    ) -> Vec<ValidationIssue>
    where
        T: serde::Serialize,
    {
        let mut issues = Vec::new();

        let focus_value = match serde_json::to_value(focus) {
            Ok(value) => match json_value_to_evaluation_result(&value) {
                Ok(result) => result,
                Err(err) => {
                    for invariant in invariants {
                        issues.push(ValidationIssue::from_invariant_error(
                            invariant,
                            ValidationError::Internal(format!(
                                "Failed to convert focus into evaluation result: {}",
                                err
                            )),
                        ));
                    }
                    return issues;
                }
            },
            Err(err) => {
                for invariant in invariants {
                    issues.push(ValidationIssue::from_invariant_error(
                        invariant,
                        ValidationError::Internal(format!(
                            "Failed to serialize focus for invariant evaluation: {}",
                            err
                        )),
                    ));
                }
                return issues;
            }
        };

        // Build the focused evaluation context once, then batch all invariant
        // expressions against it to avoid cloning the same evaluation tree for
        // every invariant.
        let invariant_refs: Vec<InvariantExprRef<'_>> = invariants
            .iter()
            .map(|inv| InvariantExprRef {
                declared_path: inv.path.as_str(),
                expression: inv.expression.as_str(),
            })
            .collect();

        let results = evaluator.eval_invariants_on(focus_value, &invariant_refs);

        for (invariant, result) in invariants.iter().zip(results.into_iter()) {
            match result {
                Ok(true) => {}
                Ok(false) => {
                    issues.push(
                        ValidationIssue::from_invariant_def(invariant)
                            .with_instance_path(instance_root_path),
                    );
                }
                Err(err) => {
                    issues.push(
                        ValidationIssue::from_invariant_error(invariant, err)
                            .with_instance_path(instance_root_path),
                    );
                }
            }
        }

        issues
    }
    /// Rebase a list of child issues from their local root to the caller's actual
    /// instance path.
    ///
    /// This is used heavily by recursive validation of nested datatypes and
    /// backbone elements.
    pub fn rebase_instance_paths(
        &self,
        issues: Vec<ValidationIssue>,
        actual_root_path: &str,
    ) -> Vec<ValidationIssue> {
        issues
            .into_iter()
            .map(|issue| {
                let rebased = match issue.instance_path.as_deref() {
                    Some(path) => rebase_instance_path(path, actual_root_path),
                    None => actual_root_path.to_string(),
                };
                issue.with_instance_path(rebased)
            })
            .collect()
    }
    pub fn debug_trace_enabled(&self) -> bool {
        self.config.debug_trace
    }
    pub fn trace(&self, message: impl AsRef<str>) {
        if self.config.debug_trace {
            debug!("debug: {}", message.as_ref());
        }
    }
}

/// Rebase a single instance path from a local validation root to the caller's
/// actual root path.
fn rebase_instance_path(current: &str, actual_root_path: &str) -> String {
    if current.is_empty() {
        return actual_root_path.to_string();
    }

    let split_at = current.find(['.', '[']).unwrap_or(current.len());

    let suffix = &current[split_at..];
    if suffix.is_empty() {
        actual_root_path.to_string()
    } else {
        format!("{}{}", actual_root_path, suffix)
    }
}
