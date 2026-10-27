//! Versioned storage trait.
//!
//! This module extends [`ResourceStorage`] with version-aware operations,
//! including version reads (vread) and optimistic locking with If-Match.

use async_trait::async_trait;
use serde_json::Value;

use crate::error::{ConcurrencyError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::StoredResource;

use super::storage::ResourceStorage;

/// Storage trait with version-aware operations.
///
/// This trait extends [`ResourceStorage`] with capabilities for reading specific
/// versions of resources and performing updates with optimistic locking.
///
/// # Versioning Model
///
/// Each resource has a version ID that is incremented on every update. The version
/// ID is a monotonically increasing string (typically a number). The first version
/// of a resource has version ID "1".
///
/// # Optimistic Locking
///
/// The `update_with_match` method implements HTTP If-Match semantics. The update
/// only succeeds if the current version matches the expected version. This prevents
/// lost updates in concurrent scenarios.
///
/// # Example
///
/// ```ignore
/// use helios_persistence::core::VersionedStorage;
///
/// async fn example<S: VersionedStorage>(storage: &S) -> Result<(), StorageError> {
///     let tenant = TenantContext::new(
///         TenantId::new("acme"),
///         TenantPermissions::full_access(),
///     );
///
///     // Read a specific version
///     let v1 = storage.vread(&tenant, "Patient", "123", "1").await?;
///
///     // Update with optimistic locking
///     if let Some(current) = storage.read(&tenant, "Patient", "123").await? {
///         let new_content = serde_json::json!({"name": [{"family": "Updated"}]});
///         let updated = storage.update_with_match(
///             &tenant,
///             "Patient",
///             "123",
///             current.version_id(),
///             new_content,
///         ).await?;
///     }
///
///     Ok(())
/// }
/// ```
#[async_trait]
pub trait VersionedStorage: ResourceStorage {
    /// Reads a specific version of a resource (vread).
    ///
    /// This corresponds to the FHIR vread interaction:
    /// `GET [base]/[type]/[id]/_history/[vid]`
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The FHIR resource type
    /// * `id` - The resource's logical ID
    /// * `version_id` - The version ID to read
    ///
    /// # Returns
    ///
    /// The stored resource at the specified version, or `None` if not found.
    /// Note that this returns the resource even if it was subsequently deleted,
    /// as long as the specific version exists.
    ///
    /// # Errors
    ///
    /// * `StorageError::Tenant` - If the tenant doesn't have read permission
    async fn vread(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        version_id: &str,
    ) -> StorageResult<Option<StoredResource>>;

    /// Updates a resource with optimistic locking (If-Match).
    ///
    /// The update only succeeds if the current version matches `expected_version`.
    /// This implements HTTP If-Match semantics for concurrent update protection.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The FHIR resource type
    /// * `id` - The resource's logical ID
    /// * `expected_version` - The expected current version (from ETag/version_id)
    /// * `resource` - The new resource content
    ///
    /// # Returns
    ///
    /// The updated resource with incremented version.
    ///
    /// # Errors
    ///
    /// * `StorageError::Resource(NotFound)` - If the resource doesn't exist
    /// * `StorageError::Concurrency(VersionConflict)` - If versions don't match
    /// * `StorageError::Concurrency(OptimisticLockFailure)` - If update races with another
    /// * `StorageError::Tenant` - If the tenant doesn't have update permission
    async fn update_with_match(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
        resource: Value,
    ) -> StorageResult<StoredResource>;

    /// Deletes a resource with optimistic locking (If-Match).
    ///
    /// The delete only succeeds if the current version matches `expected_version`.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The FHIR resource type
    /// * `id` - The resource's logical ID
    /// * `expected_version` - The expected current version
    ///
    /// # Errors
    ///
    /// * `StorageError::Resource(NotFound)` - If the resource doesn't exist
    /// * `StorageError::Concurrency(VersionConflict)` - If versions don't match,
    ///   including when a writer lands after the comparison: implementations
    ///   delete through [`ResourceStorage::delete_versioned`], pinned to the
    ///   version they compared (#1404)
    /// * `StorageError::Tenant` - If the tenant doesn't have delete permission
    async fn delete_with_match(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
    ) -> StorageResult<()>;

    /// Gets the current version ID of a resource without reading the full content.
    ///
    /// This is more efficient than `read` when you only need the version.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The FHIR resource type
    /// * `id` - The resource's logical ID
    ///
    /// # Returns
    ///
    /// The current version ID, or `None` if the resource doesn't exist or is deleted.
    async fn current_version(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<String>> {
        Ok(self
            .read(tenant, resource_type, id)
            .await?
            .map(|r| r.version_id().to_string()))
    }

    /// Lists all version IDs for a resource.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The FHIR resource type
    /// * `id` - The resource's logical ID
    ///
    /// # Returns
    ///
    /// A vector of version IDs in ascending order (oldest first).
    async fn list_versions(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Vec<String>>;
}

/// Information about a version conflict.
#[derive(Debug, Clone)]
pub struct VersionConflictInfo {
    /// The resource type.
    pub resource_type: String,
    /// The resource ID.
    pub id: String,
    /// The version that was expected.
    pub expected_version: String,
    /// The actual current version.
    pub actual_version: String,
    /// The current resource content (if available).
    pub current_content: Option<Value>,
}

impl VersionConflictInfo {
    /// Creates a new version conflict info.
    pub fn new(
        resource_type: impl Into<String>,
        id: impl Into<String>,
        expected_version: impl Into<String>,
        actual_version: impl Into<String>,
    ) -> Self {
        Self {
            resource_type: resource_type.into(),
            id: id.into(),
            expected_version: expected_version.into(),
            actual_version: actual_version.into(),
            current_content: None,
        }
    }

    /// Adds the current content to the conflict info.
    pub fn with_content(mut self, content: Value) -> Self {
        self.current_content = Some(content);
        self
    }

    /// Converts this info into a storage error.
    pub fn into_error(self) -> StorageError {
        StorageError::Concurrency(ConcurrencyError::VersionConflict {
            resource_type: self.resource_type,
            id: self.id,
            expected_version: self.expected_version,
            actual_version: self.actual_version,
        })
    }
}

/// Helper function to check version match.
///
/// Returns `Ok(())` if versions match, or an error if they don't.
pub fn check_version_match(
    resource_type: &str,
    id: &str,
    expected: &str,
    actual: &str,
) -> StorageResult<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(VersionConflictInfo::new(resource_type, id, expected, actual).into_error())
    }
}

/// Helper function to normalize ETag values for comparison.
///
/// ETags may be formatted as `W/"1"`, `"1"`, or just `1`.
/// This function extracts the version number for comparison.
pub fn normalize_etag(etag: &str) -> &str {
    etag.trim_start_matches("W/")
        .trim_start_matches('"')
        .trim_end_matches('"')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_conflict_info() {
        let info = VersionConflictInfo::new("Patient", "123", "1", "2");
        assert_eq!(info.resource_type, "Patient");
        assert_eq!(info.id, "123");
        assert_eq!(info.expected_version, "1");
        assert_eq!(info.actual_version, "2");
    }

    #[test]
    fn test_version_conflict_with_content() {
        let info = VersionConflictInfo::new("Patient", "123", "1", "2")
            .with_content(serde_json::json!({"name": "test"}));
        assert!(info.current_content.is_some());
    }

    #[test]
    fn test_version_conflict_into_error() {
        let info = VersionConflictInfo::new("Patient", "123", "1", "2");
        let error = info.into_error();
        assert!(matches!(error, StorageError::Concurrency(_)));
    }

    #[test]
    fn test_check_version_match_success() {
        let result = check_version_match("Patient", "123", "1", "1");
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_version_match_failure() {
        let result = check_version_match("Patient", "123", "1", "2");
        assert!(result.is_err());
    }

    #[test]
    fn test_normalize_etag() {
        assert_eq!(normalize_etag("W/\"1\""), "1");
        assert_eq!(normalize_etag("\"1\""), "1");
        assert_eq!(normalize_etag("1"), "1");
        assert_eq!(normalize_etag("W/\"abc\""), "abc");
    }
}
