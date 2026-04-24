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
}
