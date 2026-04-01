//! Trait definition for FHIR ValueSet operations.
//!
//! Backends implement [`ValueSetOperations`] to provide value set expansion
//! (`$expand`) and code validation (`$validate-code`).
#![allow(dead_code)]

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::types::{ExpandRequest, ExpandResponse, ValidateCodeRequest, ValidateCodeResponse};

/// Operations on FHIR ValueSet resources.
///
/// Backends implement this trait to provide `$expand` and `$validate-code`
/// over their stored value sets.
#[async_trait]
pub trait ValueSetOperations: Send + Sync {
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
    async fn validate_code(
        &self,
        ctx: &TenantContext,
        req: ValidateCodeRequest,
    ) -> Result<ValidateCodeResponse, HtsError>;
}
