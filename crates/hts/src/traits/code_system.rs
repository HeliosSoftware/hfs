//! Trait definition for FHIR CodeSystem operations.
//!
//! Backends implement [`CodeSystemOperations`] to provide concept lookup,
//! code validation, and subsumption testing over their stored code systems.
#![allow(dead_code)]

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::types::{
    LookupRequest, LookupResponse, ResourceSearchQuery, SubsumesRequest, SubsumesResponse,
    ValidateCodeRequest, ValidateCodeResponse,
};

/// Operations on FHIR CodeSystem resources.
///
/// Backends implement this trait to provide `$lookup`, `$validate-code`,
/// and `$subsumes` over their stored code systems.
#[async_trait]
pub trait CodeSystemOperations: Send + Sync {
    /// Look up a concept by code within a named code system.
    ///
    /// Returns the concept display, properties, and designations.
    /// Returns [`HtsError::NotFound`] if the code system or code does not exist.
    async fn lookup(
        &self,
        ctx: &TenantContext,
        req: LookupRequest,
    ) -> Result<LookupResponse, HtsError>;

    /// Check whether a code is valid in a code system.
    ///
    /// Returns `result = true` with display on success, or `result = false`
    /// with a diagnostic message when the code is absent.
    async fn validate_code(
        &self,
        ctx: &TenantContext,
        req: ValidateCodeRequest,
    ) -> Result<ValidateCodeResponse, HtsError>;

    /// Search for CodeSystem resources matching the given query parameters.
    ///
    /// Returns a list of FHIR CodeSystem JSON values (full resource if available,
    /// otherwise a minimal synthetic resource built from the stored columns).
    /// Returns an empty `Vec` when no resources match.
    async fn search(
        &self,
        ctx: &TenantContext,
        query: ResourceSearchQuery,
    ) -> Result<Vec<serde_json::Value>, HtsError>;

    /// Test the subsumption relationship between two codes.
    ///
    /// Returns one of: `equivalent`, `subsumes`, `subsumed-by`, `not-subsumed`.
    /// Returns [`HtsError::NotSupported`] if the backend does not implement
    /// hierarchy navigation (e.g., when hierarchy data was not imported).
    async fn subsumes(
        &self,
        ctx: &TenantContext,
        req: SubsumesRequest,
    ) -> Result<SubsumesResponse, HtsError>;

    /// Return the stored `version` value for the CodeSystem with the given URL.
    ///
    /// Used by `$expand` to populate `expansion.parameter[].used-codesystem`
    /// entries with the canonical `<url>|<version>` form. Returns `Ok(None)`
    /// when the system is unknown or carries no version.
    async fn code_system_version_for_url(
        &self,
        ctx: &TenantContext,
        url: &str,
    ) -> Result<Option<String>, HtsError>;

    /// Batch-fetch designations for a list of concept codes in the given
    /// CodeSystem URL. Returns a map from code → list of designations. Codes
    /// with no designations may be omitted from the result. Used by `$expand`
    /// to populate `expansion.contains[].designation` when the caller asks
    /// for `includeDesignations=true`.
    async fn concept_designations(
        &self,
        ctx: &TenantContext,
        system_url: &str,
        codes: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<ConceptDesignation>>, HtsError>;

    /// Look up the `(is_abstract, inactive)` flags for a batch of concept codes
    /// in the given CodeSystem URL.
    ///
    /// Used by `$expand` to populate `expansion.contains[].abstract` (driven
    /// by the FHIR `notSelectable` concept-property) and
    /// `expansion.contains[].inactive` (driven by the `status` property having
    /// a non-active value: `retired`, `deprecated`, `withdrawn`).
    ///
    /// Returns a map from code → flags. Codes with neither flag set may be
    /// omitted from the result.
    async fn concept_expansion_flags(
        &self,
        ctx: &TenantContext,
        system_url: &str,
        codes: &[String],
    ) -> Result<std::collections::HashMap<String, ConceptExpansionFlags>, HtsError>;
}

/// Per-concept flags surfaced in `expansion.contains[]`.
///
/// Both fields default to `false`; `Some(true)` means the flag should appear.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConceptExpansionFlags {
    pub is_abstract: bool,
    pub inactive: bool,
}

/// A single designation row for a concept (translation or alternate label).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConceptDesignation {
    pub language: Option<String>,
    pub use_system: Option<String>,
    pub use_code: Option<String>,
    pub value: String,
}
