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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use helios_auth::{Principal, ScopeSet};
    use helios_persistence::core::{ExportFileMetadata, ExportJobId, ExportPartKey};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

    fn make_principal(subject: &str, scopes: &str) -> Principal {
        Principal {
            subject: subject.to_string(),
            issuer: "test-issuer".to_string(),
            tenant_id: None,
            scopes: ScopeSet::parse(scopes),
            jti: None,
            expires_at: Utc::now() + chrono::Duration::hours(1),
            custom_claims: serde_json::Map::new(),
        }
    }

    fn make_file_meta(resource_type: &str, owner_sub: Option<&str>) -> ExportFileMetadata {
        ExportFileMetadata {
            key: ExportPartKey::output("t1", ExportJobId::new(), resource_type, 0, 1),
            resource_type: resource_type.to_string(),
            file_type: "output".to_string(),
            line_count: 0,
            job_owner_subject: owner_sub.map(str::to_string),
        }
    }

    fn test_tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
    }

    #[tokio::test]
    async fn no_principal_bypasses_auth() {
        let auth = BearerScopeAuth;
        let result = auth
            .authorize_download(
                None,
                &test_tenant(),
                Some("owner"),
                &make_file_meta("Patient", Some("owner")),
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn owner_with_read_scope_passes() {
        let auth = BearerScopeAuth;
        let p = make_principal("owner", "system/Patient.rs");
        let result = auth
            .authorize_download(
                Some(&p),
                &test_tenant(),
                Some("owner"),
                &make_file_meta("Patient", Some("owner")),
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn non_owner_without_wildcard_is_forbidden() {
        let auth = BearerScopeAuth;
        let p = make_principal("other", "system/Patient.rs");
        let result = auth
            .authorize_download(
                Some(&p),
                &test_tenant(),
                Some("owner"),
                &make_file_meta("Patient", Some("owner")),
            )
            .await;
        assert!(matches!(result, Err(ExportAuthError::Forbidden(_))));
    }

    #[tokio::test]
    async fn wildcard_scope_overrides_ownership() {
        let auth = BearerScopeAuth;
        let p = make_principal("other", "system/*.rs");
        let result = auth
            .authorize_download(
                Some(&p),
                &test_tenant(),
                Some("owner"),
                &make_file_meta("Patient", Some("owner")),
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn owner_missing_read_scope_is_forbidden() {
        let auth = BearerScopeAuth;
        // No SMART read scope — empty scope string → ScopeSet::empty()
        let p = make_principal("owner", "openid profile");
        let result = auth
            .authorize_download(
                Some(&p),
                &test_tenant(),
                Some("owner"),
                &make_file_meta("Patient", Some("owner")),
            )
            .await;
        assert!(matches!(result, Err(ExportAuthError::Forbidden(_))));
    }
}

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
