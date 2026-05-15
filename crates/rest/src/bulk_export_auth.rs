//! Authorization for bulk-export file downloads.
//!
//! The [`ExportFileAuth`] trait gates the HFS-served download path
//! (`requiresAccessToken = true`). Pre-signed-URL downloads bypass HFS
//! entirely and never reach this trait.

use async_trait::async_trait;
use helios_auth::Principal;
use helios_auth::scope::{ResourceTypeSpec, SmartPermissions};
use helios_persistence::core::ExportFileMetadata;
use helios_persistence::tenant::TenantContext;

/// Error returned when a download is not authorized.
#[derive(Debug, Clone)]
pub enum ExportAuthError {
    /// No authenticated principal was supplied.
    Unauthenticated,
    /// The principal is not permitted to download this file.
    Forbidden(String),
}

impl std::fmt::Display for ExportAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthenticated => write!(f, "authentication required"),
            Self::Forbidden(m) => write!(f, "forbidden: {m}"),
        }
    }
}

impl std::error::Error for ExportAuthError {}

/// Authorizes a bulk-export file download.
#[async_trait]
pub trait ExportFileAuth: Send + Sync {
    /// Decides whether `principal` may download the file described by
    /// `file_meta` for a job owned by `job_owner_subject`.
    async fn authorize_download(
        &self,
        principal: Option<&Principal>,
        tenant: &TenantContext,
        job_owner_subject: Option<&str>,
        file_meta: &ExportFileMetadata,
    ) -> Result<(), ExportAuthError>;
}

/// Returns true if the principal holds any `system/*` (wildcard) scope.
fn has_wildcard_scope(principal: &Principal) -> bool {
    principal
        .scopes
        .scopes()
        .iter()
        .any(|s| s.resource_type == ResourceTypeSpec::Wildcard)
}

/// The default [`ExportFileAuth`]: requires the kickoff Bearer token, the
/// job's owner-subject to match (or a `system/*` scope), and a
/// `system/{ResourceType}.rs` (read) scope covering the file's resource type.
#[derive(Debug, Clone, Default)]
pub struct BearerScopeAuth;

#[async_trait]
impl ExportFileAuth for BearerScopeAuth {
    async fn authorize_download(
        &self,
        principal: Option<&Principal>,
        _tenant: &TenantContext,
        job_owner_subject: Option<&str>,
        file_meta: &ExportFileMetadata,
    ) -> Result<(), ExportAuthError> {
        // When auth is disabled there is no principal — no enforcement, as
        // elsewhere in HFS.
        let Some(principal) = principal else {
            return Ok(());
        };

        let owns_job = job_owner_subject == Some(principal.subject.as_str());
        let is_wildcard = has_wildcard_scope(principal);
        if !owns_job && !is_wildcard {
            return Err(ExportAuthError::Forbidden(
                "principal does not own this export job".to_string(),
            ));
        }

        if !principal
            .scopes
            .is_permitted(&file_meta.resource_type, SmartPermissions::READ)
        {
            return Err(ExportAuthError::Forbidden(format!(
                "missing read scope for {}",
                file_meta.resource_type
            )));
        }

        Ok(())
    }
}
