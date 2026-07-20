//! Trait definition for FHIR ValueSet operations.
//!
//! Backends implement [`ValueSetOperations`] to provide value set expansion
//! (`$expand`) and code validation (`$validate-code`).
#![allow(dead_code)]

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::types::{
    ExpandRequest, ExpandResponse, ResourceSearchQuery, ValidateCodeRequest, ValidateCodeResponse,
};

/// Operations on FHIR ValueSet resources.
///
/// Backends implement this trait to provide `$expand` and `$validate-code`
/// over their stored value sets.
#[async_trait]
pub trait ValueSetOperations: Send + Sync {
    /// Search for ValueSet resources matching the given query parameters.
    ///
    /// Returns a list of FHIR ValueSet JSON values. Returns an empty `Vec` when
    /// no resources match.
    async fn search(
        &self,
        ctx: &TenantContext,
        query: ResourceSearchQuery,
    ) -> Result<Vec<serde_json::Value>, HtsError>;

    /// Expand a value set, returning all contained codes.
    ///
    /// The backend checks for a cached expansion first; on cache miss it
    /// evaluates the compose rules, caches the result, and returns it.
    /// Supports `count` + `offset` pagination.
    ///
    /// Returns [`HtsError::NotFound`] when the value set URL is unknown.
    async fn expand(
        &self,
        ctx: &TenantContext,
        req: ExpandRequest,
    ) -> Result<ExpandResponse, HtsError>;

    /// Check whether a code is valid in a value set.
    ///
    /// Triggers expansion if needed, then tests set membership.
    /// Returns `result = true` with display on success.
    /// Return the `version` of the ValueSet row this backend would resolve for
    /// `url` when the caller pins no version.
    ///
    /// The operations layer must not re-derive this by sorting the JSON returned
    /// from [`Self::search`]: same-URL precedence depends on `authority_rank`,
    /// which is a storage column and is deliberately absent from the FHIR
    /// resource. A Rust-side "highest version string wins" sort silently
    /// disagrees with the backend whenever a re-published copy carries a higher
    /// version than the original — exactly the shape of issue #200 — so the
    /// response would echo one ValueSet while the backend expanded another.
    ///
    /// Default returns `None`, meaning "no opinion"; callers then fall back to
    /// their previous behaviour.
    async fn value_set_version_for_url(
        &self,
        _ctx: &TenantContext,
        _url: &str,
    ) -> Result<Option<String>, HtsError> {
        Ok(None)
    }

    async fn validate_code(
        &self,
        ctx: &TenantContext,
        req: ValidateCodeRequest,
    ) -> Result<ValidateCodeResponse, HtsError>;
}
