//! Trait definition for FHIR ConceptMap operations.
//!
//! Backends implement [`ConceptMapOperations`] to provide concept translation
//! (`$translate`) and transitive closure computation (`$closure`).
#![allow(dead_code)]

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::types::{ClosureRequest, ClosureResponse, TranslateRequest, TranslateResponse};

/// Operations on FHIR ConceptMap resources.
///
/// Backends implement this trait to provide `$translate` and `$closure`.
#[async_trait]
pub trait ConceptMapOperations: Send + Sync {
    /// Translate a code from one system to another using a ConceptMap.
    ///
    /// If `url` is provided, only that ConceptMap is consulted; otherwise all
    /// stored maps are searched.  Returns `result = false` (not an error) when
    /// no mapping exists.
    async fn translate(
        &self,
        ctx: &TenantContext,
        req: TranslateRequest,
    ) -> Result<TranslateResponse, HtsError>;

    /// Compute the transitive closure for a set of concepts.
    ///
    /// Returns a ConceptMap resource (as JSON) representing the set of
    /// relationships discovered through hierarchy traversal and forward
    /// translations.
    async fn closure(
        &self,
        ctx: &TenantContext,
        req: ClosureRequest,
    ) -> Result<ClosureResponse, HtsError>;
}
