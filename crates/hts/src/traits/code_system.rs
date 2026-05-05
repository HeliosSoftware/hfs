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

    /// Batch-fetch values of named properties for a list of concept codes.
    /// Used by `$expand` to populate `expansion.contains[].property[]` when
    /// the caller passed a `property` parameter naming which properties to
    /// surface. Returns a map from code → list of (property_name, value).
    async fn concept_property_values(
        &self,
        ctx: &TenantContext,
        system_url: &str,
        codes: &[String],
        properties: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<(String, String)>>, HtsError>;

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

    /// Resolve a supplement CS canonical URL to the (system_url, version)
    /// pair stored on the supplement resource itself.
    ///
    /// Returns `Ok(Some((url, Some(version))))` when the supplement is stored
    /// and is a `content=supplement` CodeSystem, `Ok(None)` when no such
    /// supplement exists. The default implementation returns `Ok(None)` so
    /// backends without supplement support degrade silently.
    async fn supplement_target(
        &self,
        _ctx: &TenantContext,
        _supplement_url: &str,
    ) -> Result<Option<SupplementInfo>, HtsError> {
        Ok(None)
    }

    /// Batch-fetch designations contributed by named CodeSystem supplements.
    ///
    /// Mirrors [`Self::concept_designations`] but reads from CSes whose URLs
    /// are listed in `supplement_urls`. Each returned designation is tagged
    /// with `source = "url|version"` so callers can emit the FHIR
    /// `designation.source` part on `$lookup` responses (per IG fixture
    /// `parameters-lookup-supplement-good`).
    ///
    /// Default implementation returns an empty map so backends without
    /// supplement support behave as if no supplement data exists.
    async fn supplement_designations(
        &self,
        _ctx: &TenantContext,
        _supplement_urls: &[String],
        _codes: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<ConceptDesignation>>, HtsError> {
        Ok(std::collections::HashMap::new())
    }

    /// Batch-fetch concept-property values contributed by named CodeSystem
    /// supplements. Mirrors [`Self::concept_property_values`] but for
    /// supplement CodeSystems. Returns `(property, value)` pairs grouped by
    /// concept code.
    async fn supplement_property_values(
        &self,
        _ctx: &TenantContext,
        _supplement_urls: &[String],
        _codes: &[String],
        _properties: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<(String, String)>>, HtsError> {
        Ok(std::collections::HashMap::new())
    }
}

/// Resolved supplement metadata returned by
/// [`CodeSystemOperations::supplement_target`].
///
/// `target_url` is the URL of the base CodeSystem the supplement modifies
/// (read from the supplement's `CodeSystem.supplements` field).
/// `supplement_canonical` is the supplement's own canonical, ready to drop
/// into a `used-supplement` parameter (`"url|version"` when version is
/// available).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplementInfo {
    pub target_url: String,
    pub supplement_canonical: String,
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
///
/// `source` is the CodeSystem URL (with optional `|version`) that contributed
/// this designation. For designations defined in the base CodeSystem itself
/// `source` is left as `None`; for designations supplied by an applied
/// CodeSystem supplement (`useSupplement`) `source` is set so the operations
/// layer can emit a `designation.source` part on `$lookup` responses (per IG
/// fixture `parameters-lookup-supplement-good`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConceptDesignation {
    pub language: Option<String>,
    pub use_system: Option<String>,
    pub use_code: Option<String>,
    pub value: String,
    /// CodeSystem URL (`url|version` when known) of the CS row that produced
    /// this designation. `None` for the base CodeSystem; populated when the
    /// designation comes from an applied supplement.
    pub source: Option<String>,
}
