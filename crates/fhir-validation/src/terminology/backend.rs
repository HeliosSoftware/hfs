//! Backend abstraction for remote terminology operations.
//!
//! This module defines the low-level async backend contract used by the
//! validation-facing terminology service. Concrete implementations execute
//! remote terminology operations such as ValueSet `$validate-code`, while
//! higher layers remain decoupled from transport and client-specific details.
use crate::ValidationError;
use crate::terminology::requests::ValidateVsRequest;
use async_trait::async_trait;

#[async_trait]
pub trait TerminologyBackend: Send + Sync {
    async fn validate_vs(
        &self,
        req: &ValidateVsRequest,
    ) -> Result<serde_json::Value, ValidationError>;

    // async fn validate_cs(
    //     &self,
    //     req: ValidateCsRequest,
    // ) -> Result<serde_json::Value, ValidationError>;
    //
    // async fn expand(
    //     &self,
    //     req: ExpandRequest,
    // ) -> Result<serde_json::Value, ValidationError>;
    //
    // async fn lookup(
    //     &self,
    //     req: LookupRequest,
    // ) -> Result<serde_json::Value, ValidationError>;
    //
    // async fn subsumes(
    //     &self,
    //     req: SubsumesRequest,
    // ) -> Result<serde_json::Value, ValidationError>;
}
