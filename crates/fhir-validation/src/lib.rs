//! FHIR validation framework.
//!
//! This crate provides structural validation, invariant evaluation, and
//! terminology-aware binding validation for generated FHIR model types.
//!
//! # Layout
//!
//! Core framework types live at the crate root.
//! Version-specific validation lives under `r4`, `r4b`, `r5`, and `r6`.
//!
//! # Sync vs async validation
//!
//! The crate supports both synchronous and asynchronous validation flows.
//!
//! ## Synchronous validation
//!
//! The synchronous path is intended for local validation and backward-compatible
//! call sites. It performs:
//!
//! - structural checks
//! - invariant evaluation
//! - local terminology and binding validation
//!
//! When synchronous terminology is needed, the sync path uses
//! [`TerminologyServiceSync`].
//!
//! ## Asynchronous validation
//!
//! The asynchronous path is the preferred production path when remote
//! terminology may be required. It performs the same validation work as the
//! synchronous path, but can call remote terminology services without blocking
//! on ad hoc Tokio runtimes.
//!
//! When asynchronous terminology is needed, the async path uses
//! [`TerminologyService`].
//!
//! In general:
//!
//! - use sync validation for local-only validation, tests, and compatibility
//!   paths
//! - use async validation for server-side or production flows that may require
//!   remote terminology lookups
//!
//! # Terminology behavior
//!
//! Binding validation is local-first.
//!
//! Generated local terminology helpers are consulted first. If a binding can be
//! decided locally, the validator emits a normal binding result immediately.
//! Remote terminology is only consulted when the local helper reports
//! `RemoteValidationRequired`.
//!
//! This keeps validation fast and deterministic when terminology rules are fully
//! known locally, while still allowing remote FHIR terminology servers to be
//! used for more complex ValueSet semantics.
//!
//! ## Remote terminology outcomes
//!
//! Successful remote `ValueSet/$validate-code` responses are parsed into
//! [`TerminologyMembershipOutcome`], which preserves:
//!
//! - membership result
//! - optional server message
//! - optional terminology metadata such as code, system, and version
//!
//! Non-2xx remote responses are represented as structured remote terminology
//! errors and surfaced as terminology validation issues with readable
//! diagnostics. Unusable `$validate-code` JSON (for example missing `parameter`
//! or boolean `result`) is reported as [`ValidationError::RemoteTerminology`] with
//! [`RemoteTerminologyError::MalformedResponse`] ([`MalformedValidateCodeParameters`]).
//! Request assembly failures before any HTTP call use [`ValidationError::InvalidRequest`]
//! ([`TerminologyRequestInvalid`]).
//!
//! # Severity model
//!
//! Validation issues generally follow these rules:
//!
//! - required binding misses produce errors
//! - extensible binding misses produce warnings
//! - invariant failures produce invariant issues
//! - malformed or unavailable terminology responses produce terminology issues
//!
//! # Invariant evaluation: bulk focus vs path-rooted
//!
//! - [`Validator::apply_invariants`](crate::Validator::apply_invariants) serializes **`focus`**
//!   once, sets that value as the FHIRPath evaluation root, and runs
//!   [`FhirPathEvaluator::eval_invariants_on`]. Every `InvariantDef.expression` in the slice is
//!   evaluated against **the same** `$this`. Use this when **`focus` is already the node the
//!   expressions assume** (e.g. generated validators passing a nested `self`, or root-level
//!   profile rules on the full resource). The `InvariantDef.path` fields are **not** used to
//!   re-resolve focus in this path (they are still attached to issues for reporting).
//! - [`FhirPathEvaluator::eval_invariant`] resolves **`declared_path`** from the resource, then
//!   evaluates **`expression`** with `$this` at each resolved focus. Use this (or profile
//!   validation’s element rules) when constraints are **element-scoped** and expressions are
//!   **relative to that element** (see [`crate::profile::validate`]).
//! - Evaluation errors surface as [`ValidationIssue`] rows with code `exception` via
//!   [`ValidationIssue::from_invariant_error`]; logical failures use code `invariant`.
//!
//! # Versioned modules
//!
//! Version-specific modules expose the generated traits, dispatchers, and helper
//! functions for each supported FHIR release.
//!
//! # Errors and issues
//!
//! Pipeline failures use [`ValidationError`] ([`crate::error`] module). User-visible results are
//! [`ValidationIssue`] rows. For terminology binding paths, [`TerminologyIssueContext`] and
//! [`validation_error_to_issues`](crate::binding::common::validation_error_to_issues) (or
//! [`ValidationError::to_binding_issues`](crate::ValidationError::to_binding_issues)) map
//! [`ValidationError`] to issues. See **[`Errors.md`](./Errors.md)** in this crate for a full
//! description of the error model.

pub mod binding;
pub mod core;
pub mod error;
pub mod evaluators;
pub mod issue_code;
pub mod issue_to_op_outcome;
pub mod profile;
pub mod profile_manifest;
pub mod reference_resolution;
pub mod strict_properties;
pub mod terminology;
pub mod validation_context;
pub mod validation_issue_detail;

#[cfg(feature = "R5")]
pub mod questionnaire;

pub use binding::common::{TerminologyIssueContext, validation_error_to_issues};
pub use core::*;
pub use error::{
    MalformedValidateCodeParameters, RemoteTerminologyError, TerminologyRequestInvalid,
    ValidationError, malformed_validate_code_parameters_kind_label,
    remote_terminology_error_kind_label, validation_error_kind_label,
};
pub use evaluators::*;
pub use issue_to_op_outcome::VALIDATION_SOURCE_INVARIANT_KEY_URL;
pub use profile_manifest::{
    ProfileManifest, ProfileManifestPathStyle, ScannedIgResources,
    build_and_write_profile_manifest_for_ig, load_profile_registry_from_manifest,
    load_profile_registry_from_manifest_file, profile_manifest_from_scan,
    profile_manifest_from_scan_with_style, scan_ig_package_for_fhir_json,
    write_profile_manifest_to_file,
};
#[cfg(feature = "R5")]
pub use questionnaire::validate_questionnaire_response_against_questionnaire;
pub use reference_resolution::ReferenceResolver;
pub use strict_properties::{
    hl7_core_structure_definition_url, resolve_base_profile_in_registry,
    validate_json_against_extracted_profile,
};
use terminology::service::{TerminologyService, TerminologyServiceSync};
pub use terminology::*;
pub use validation_context::*;
pub use validation_issue_detail::{
    VALIDATION_ISSUE_DETAIL_SYSTEM, VALIDATION_ISSUE_DETAIL_VERSION, ValidationIssueDetailCode,
    ValidationSourceKind, classify_validation_source,
};

#[cfg(feature = "R4")]
pub mod r4;

#[cfg(feature = "R4B")]
pub mod r4b;

#[cfg(feature = "R5")]
pub mod r5;

#[cfg(feature = "R6")]
pub mod r6;
