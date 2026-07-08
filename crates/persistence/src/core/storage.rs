//! Core resource storage trait.
//!
//! This module defines the [`ResourceStorage`] trait, which provides the fundamental
//! CRUD operations for FHIR resources. All storage operations require a [`TenantContext`]
//! to ensure proper tenant isolation.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use helios_fhir::FhirVersion;
use serde_json::Value;

use crate::core::sof_runner::SofRunner;
use crate::error::{StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::StoredResource;

/// One UTC-day bucket of stored-resource counts.
///
/// Returned by [`ResourceStorage::count_by_day`] to back "FHIR resources over
/// time" dashboards. See that method for the precise counting semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyResourceCount {
    /// The UTC calendar day this bucket covers.
    pub day: NaiveDate,
    /// Number of non-deleted, current-version resources whose `last_updated`
    /// falls on `day`.
    pub count: u64,
}

/// One `(weekday, hour)` cell of write-activity, used by
/// [`ResourceStorage::activity_histogram`] to back the dashboard's "Activity"
/// heatmap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityCell {
    /// UTC day of week, `0` = Sunday … `6` = Saturday (matching JavaScript's
    /// `Date.getDay()`, so the frontend can index directly).
    pub weekday: u8,
    /// UTC hour of day, `0` … `23`.
    pub hour: u8,
    /// Number of write operations (resource versions) in this cell.
    pub count: u64,
}

/// Audit event helpers for purge operations.
#[cfg(feature = "audit")]
pub mod audit {
    use helios_audit::{AuditAction, AuditEventBuilder, AuditSink};

    /// Record an audit event for a purge (permanent deletion) operation.
    pub async fn record_purge_event(
        sink: &dyn AuditSink,
        source_observer: &str,
        agent: Option<&str>,
        resource_type: &str,
        resource_id: Option<&str>,
        count: u64,
        patient_ref: Option<&str>,
    ) {
        let mut builder = AuditEventBuilder::new(source_observer)
            .event_type(
                "http://terminology.hl7.org/CodeSystem/audit-event-type",
                "object",
            )
            .action(AuditAction::Delete)
            .outcome("0")
            .detail("audit-operation", "purge")
            .detail("count", count.to_string());
        if let Some(id) = resource_id {
            builder = builder.resource(resource_type, id);
        } else {
            builder = builder.detail("resource-type", resource_type);
        }
        if let Some(a) = agent {
            builder = builder.agent(a, None, true);
        }
        if let Some(p) = patient_ref {
            builder = builder.patient(p);
        }
        sink.record(builder.build()).await;
    }

    #[cfg(test)]
    mod tests {
        use helios_audit::{AuditAction, AuditEventBuilder};

        #[test]
        fn test_purge_event_action_is_delete() {
            let event = AuditEventBuilder::new("Device/hfs")
                .event_type(
                    "http://terminology.hl7.org/CodeSystem/audit-event-type",
                    "object",
                )
                .action(AuditAction::Delete)
                .outcome("0")
                .detail("audit-operation", "purge")
                .resource("Patient", "123")
                .detail("count", "1")
                .build();
            assert_eq!(
                event.action.as_ref().and_then(|a| a.value.as_deref()),
                Some("D")
            );
        }

        #[test]
        fn test_purge_event_includes_count() {
            let event = AuditEventBuilder::new("Device/hfs")
                .event_type(
                    "http://terminology.hl7.org/CodeSystem/audit-event-type",
                    "object",
                )
                .action(AuditAction::Delete)
                .outcome("0")
                .detail("audit-operation", "purge")
                .resource("Observation", "obs-1")
                .detail("count", "15")
                .build();
            let details = event.entity.as_ref().unwrap()[0].detail.as_ref().unwrap();
            let count_detail = details
                .iter()
                .find(|d| d.r#type.value.as_deref() == Some("count"));
            assert!(count_detail.is_some());
        }

        #[test]
        fn test_purge_with_patient_has_patient_entity() {
            let event = AuditEventBuilder::new("Device/hfs")
                .action(AuditAction::Delete)
                .outcome("0")
                .resource("Observation", "obs-1")
                .patient("Patient/456")
                .build();
            let entities = event.entity.as_ref().unwrap();
            assert_eq!(entities.len(), 2);
            assert_eq!(
                entities[1]
                    .what
                    .as_ref()
                    .and_then(|w| w.reference.as_ref())
                    .and_then(|s| s.value.as_deref()),
                Some("Patient/456")
            );
        }
    }
}

/// Core storage trait for FHIR resources.
///
/// This trait defines the fundamental CRUD (Create, Read, Update, Delete) operations
/// for persisting FHIR resources. All operations require a [`TenantContext`] to ensure
/// proper tenant isolation - there is no escape hatch.
///
/// # Tenant Isolation
///
/// Every operation takes a `TenantContext` as its first parameter. This design ensures
/// that tenant isolation is enforced at the type level - it's impossible to perform
/// storage operations without specifying the tenant context.
///
/// # Versioning
///
/// All mutating operations (create, update, delete) create new versions of resources.
/// The version ID is monotonically increasing and is used for optimistic locking via
/// the `If-Match` HTTP header.
///
/// # Soft Deletes
///
/// The `delete` operation performs a soft delete by default, marking the resource as
/// deleted but retaining its history. Use `purge` for permanent deletion (if supported).
///
/// # Example
///
/// ```ignore
/// use helios_fhir::FhirVersion;
/// use helios_persistence::core::ResourceStorage;
/// use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
///
/// async fn example<S: ResourceStorage>(storage: &S) -> Result<(), StorageError> {
///     let tenant = TenantContext::new(
///         TenantId::new("acme"),
///         TenantPermissions::full_access(),
///     );
///
///     // Create a new patient with FHIR R4
///     let patient = serde_json::json!({
///         "resourceType": "Patient",
///         "name": [{"family": "Smith"}]
///     });
///     let stored = storage.create(&tenant, "Patient", patient, FhirVersion::default()).await?;
///     println!("Created: {}", stored.url());
///
///     // Read it back
///     let read = storage.read(&tenant, "Patient", stored.id()).await?;
///     assert!(read.is_some());
///
///     // Update it
///     let mut updated_content = stored.content().clone();
///     updated_content["active"] = serde_json::json!(true);
///     let updated = storage.update(&tenant, &stored, updated_content).await?;
///     assert_eq!(updated.version_id(), "2");
///
///     // Delete it
///     storage.delete(&tenant, "Patient", stored.id()).await?;
///
///     Ok(())
/// }
/// ```
#[async_trait]
pub trait ResourceStorage: Send + Sync {
    /// Returns a human-readable name for this storage backend.
    fn backend_name(&self) -> &'static str;

    /// Whether this backend's stored data is shared across all HFS instances in a
    /// cluster (a networked database) vs. local to one instance (e.g. an on-disk
    /// SQLite file). Used to label console metrics honestly in multi-instance
    /// deployments. Conservative default: `false` (never over-claims cluster
    /// consistency).
    fn is_cluster_shared(&self) -> bool {
        false
    }

    /// Creates a new resource.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The FHIR resource type (e.g., "Patient")
    /// * `resource` - The resource content as JSON
    /// * `fhir_version` - The FHIR specification version for this resource
    ///
    /// # Returns
    ///
    /// The stored resource with assigned ID, version, and metadata.
    ///
    /// # Errors
    ///
    /// * `StorageError::Validation` - If the resource is invalid
    /// * `StorageError::Resource(AlreadyExists)` - If a resource with the same ID exists
    /// * `StorageError::Tenant` - If the tenant doesn't have create permission
    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource>;

    /// Creates a resource with a specific ID (PUT semantics).
    ///
    /// If the resource doesn't exist, creates it with version "1".
    /// If it exists, this is equivalent to an update.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The FHIR resource type
    /// * `id` - The desired resource ID
    /// * `resource` - The resource content as JSON
    /// * `fhir_version` - The FHIR specification version for this resource
    ///
    /// # Returns
    ///
    /// A tuple of (StoredResource, created: bool) where created indicates
    /// whether a new resource was created (true) or an existing one updated (false).
    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)>;

    /// Reads a resource by type and ID.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The FHIR resource type
    /// * `id` - The resource's logical ID
    ///
    /// # Returns
    ///
    /// The stored resource if found and not deleted, or `None`.
    ///
    /// # Errors
    ///
    /// * `StorageError::Tenant` - If the tenant doesn't have read permission
    /// * `StorageError::Resource(Gone)` - If the resource was deleted (optional behavior)
    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>>;

    /// Updates an existing resource.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `current` - The current version of the resource (for optimistic locking)
    /// * `resource` - The new resource content
    ///
    /// # Returns
    ///
    /// The updated resource with incremented version.
    ///
    /// # Errors
    ///
    /// * `StorageError::Resource(NotFound)` - If the resource doesn't exist
    /// * `StorageError::Concurrency(VersionConflict)` - If the resource was modified
    /// * `StorageError::Tenant` - If the tenant doesn't have update permission
    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource>;

    /// Deletes a resource (soft delete).
    ///
    /// The resource is marked as deleted but its history is preserved.
    /// Subsequent reads will return `None` (or `Gone` error depending on config).
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The FHIR resource type
    /// * `id` - The resource's logical ID
    ///
    /// # Errors
    ///
    /// * `StorageError::Resource(NotFound)` - If the resource doesn't exist
    /// * `StorageError::Resource(Gone)` - If already deleted
    /// * `StorageError::Tenant` - If the tenant doesn't have delete permission
    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()>;

    /// Checks if a resource exists.
    ///
    /// This is more efficient than `read` when you only need to check existence.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The FHIR resource type
    /// * `id` - The resource's logical ID
    ///
    /// # Returns
    ///
    /// `true` if the resource exists and is not deleted, `false` otherwise.
    async fn exists(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<bool> {
        Ok(self.read(tenant, resource_type, id).await?.is_some())
    }

    /// Reads multiple resources by their IDs.
    ///
    /// This is more efficient than multiple individual reads.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The FHIR resource type
    /// * `ids` - The resource IDs to read
    ///
    /// # Returns
    ///
    /// A vector of found resources (missing/deleted resources are omitted).
    async fn read_batch(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[&str],
    ) -> StorageResult<Vec<StoredResource>> {
        let mut results = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(resource) = self.read(tenant, resource_type, id).await? {
                results.push(resource);
            }
        }
        Ok(results)
    }

    /// Counts the total number of resources of a given type.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The FHIR resource type (or None for all types)
    ///
    /// # Returns
    ///
    /// The count of non-deleted resources.
    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64>;

    /// Returns the SQL-on-FHIR runner for this backend, if it supports in-DB execution.
    ///
    /// Backends that can compile ViewDefinitions to SQL and run them natively (SQLite,
    /// PostgreSQL) return `Some(runner)`. All other backends return `None`, causing the
    /// handler layer to fall back to the in-process runner.
    ///
    /// The default implementation returns `None`.
    fn sof_runner(&self) -> Option<Arc<dyn SofRunner>> {
        None
    }

    /// Counts non-deleted resources for several types in one call.
    ///
    /// Returns `(resource_type, count)` pairs in the same order as
    /// `resource_types`. The default implementation issues one
    /// [`count`](Self::count) query per type; backends may override with a
    /// single grouped query.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_types` - The FHIR resource types to count
    async fn count_by_types(
        &self,
        tenant: &TenantContext,
        resource_types: &[&str],
    ) -> StorageResult<Vec<(String, u64)>> {
        let mut counts = Vec::with_capacity(resource_types.len());
        for &rt in resource_types {
            counts.push((rt.to_string(), self.count(tenant, Some(rt)).await?));
        }
        Ok(counts)
    }

    /// Counts non-deleted, current-version resources of `resource_type`,
    /// grouped by the UTC calendar day of their `last_updated`, restricted to
    /// days on or after `since`.
    ///
    /// Only non-empty days are returned, in ascending day order. This powers
    /// "FHIR resources over time" dashboards.
    ///
    /// # Semantics
    ///
    /// A resource contributes to the day of its **most recent** version, so a
    /// running cumulative sum of these buckets converges to the current total
    /// reported by [`count`](Self::count); it does not reconstruct historical
    /// creation dates. A creation-time-accurate series would require
    /// aggregating `resource_history` and is intentionally out of scope here.
    ///
    /// The default implementation returns an empty series for backends that
    /// cannot bucket by time; callers should treat that as "no time resolution
    /// available" and fall back to the current total.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The FHIR resource type to bucket
    /// * `since` - Inclusive lower bound; only days on or after this instant
    async fn count_by_day(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _since: DateTime<Utc>,
    ) -> StorageResult<Vec<DailyResourceCount>> {
        Ok(Vec::new())
    }

    /// Buckets write activity into a weekly-rhythm grid by UTC weekday and hour.
    ///
    /// Every resource version (create, update, or delete) recorded in
    /// `resource_history` on or after `since` counts as one write operation; the
    /// result is the per-`(weekday, hour)` operation count aggregated across the
    /// whole window. Only non-empty cells are returned, so callers densify into
    /// the full 7×24 grid themselves.
    ///
    /// This powers the dashboard "Activity" heatmap. It reflects **writes only**
    /// — reads and searches are not in `resource_history`; an
    /// all-operations variant would draw from `AuditEvent` instead.
    ///
    /// The default implementation returns an empty grid for backends that cannot
    /// bucket by time.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `since` - Inclusive lower bound; only versions on or after this instant
    async fn activity_histogram(
        &self,
        _tenant: &TenantContext,
        _since: DateTime<Utc>,
    ) -> StorageResult<Vec<ActivityCell>> {
        Ok(Vec::new())
    }

    /// Counts non-deleted resources grouped by resource type for `tenant`,
    /// returning one `(resource_type, count)` pair per type present. Order is
    /// unspecified; callers sort as needed.
    ///
    /// Powers the dashboard "resource composition" treemap. Unlike
    /// [`count_by_types`](Self::count_by_types), the caller does not supply the
    /// type list — every stored type is discovered via a single `GROUP BY`. The
    /// default implementation returns an empty list (it cannot enumerate types
    /// without such a query); the SQL and document backends override it.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    async fn count_all_types(&self, _tenant: &TenantContext) -> StorageResult<Vec<(String, u64)>> {
        Ok(Vec::new())
    }

    /// Counts non-deleted resources grouped by tenant across the entire backend.
    ///
    /// **This intentionally spans tenants** and exists for operator/admin
    /// dashboards (the per-tenant comparison widget), not for tenant-scoped
    /// request handling — hence it takes no [`TenantContext`]. The default
    /// implementation returns an empty list; the SQL and document backends
    /// override it with a single `GROUP BY tenant_id`.
    async fn count_by_tenant(&self) -> StorageResult<Vec<(String, u64)>> {
        Ok(Vec::new())
    }
}

/// Extension trait for storage backends that support permanent deletion.
#[async_trait]
pub trait PurgableStorage: ResourceStorage {
    /// Permanently deletes a resource and all its history.
    ///
    /// This is an irreversible operation. Use with caution.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The FHIR resource type
    /// * `id` - The resource's logical ID
    ///
    /// # Errors
    ///
    /// * `StorageError::Resource(NotFound)` - If the resource doesn't exist
    /// * `StorageError::Tenant` - If the tenant doesn't have purge permission
    async fn purge(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()>;

    /// Permanently deletes all resources of a type for a tenant.
    ///
    /// This is an irreversible operation. Use with extreme caution.
    async fn purge_all(&self, tenant: &TenantContext, resource_type: &str) -> StorageResult<u64>;
}

/// Result of a conditional create operation.
#[derive(Debug, Clone)]
pub enum ConditionalCreateResult {
    /// Resource was created (no match found).
    Created(StoredResource),
    /// An existing resource matched the condition.
    Exists(StoredResource),
    /// Multiple resources matched (error condition).
    MultipleMatches(usize),
}

/// Result of a conditional update operation.
#[derive(Debug, Clone)]
pub enum ConditionalUpdateResult {
    /// Resource was updated.
    Updated(StoredResource),
    /// Resource was created (no match found, upsert mode).
    Created(StoredResource),
    /// No resource matched the condition.
    NoMatch,
    /// Multiple resources matched (error condition).
    MultipleMatches(usize),
}

/// Result of a conditional delete operation.
#[derive(Debug, Clone)]
pub enum ConditionalDeleteResult {
    /// Resource was deleted.
    Deleted,
    /// No resource matched the condition.
    NoMatch,
    /// Multiple resources matched (error condition).
    MultipleMatches(usize),
}

/// Result of a conditional patch operation.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum ConditionalPatchResult {
    /// Resource was patched successfully.
    Patched(StoredResource),
    /// No resource matched the condition.
    NoMatch,
    /// Multiple resources matched (error condition).
    MultipleMatches(usize),
}

/// Patch format for conditional patch operations.
#[derive(Debug, Clone)]
pub enum PatchFormat {
    /// JSON Patch (RFC 6902) - application/json-patch+json
    ///
    /// Example:
    /// ```json
    /// [
    ///   {"op": "replace", "path": "/name/0/family", "value": "NewName"},
    ///   {"op": "add", "path": "/active", "value": true}
    /// ]
    /// ```
    JsonPatch(Value),

    /// FHIRPath Patch - application/fhir+json with Parameters resource
    ///
    /// Uses a Parameters resource with operation parts containing:
    /// - type: add, insert, delete, replace, move
    /// - path: FHIRPath expression
    /// - name: element name (for add)
    /// - value: new value
    FhirPathPatch(Value),

    /// JSON Merge Patch (RFC 7386) - application/merge-patch+json
    ///
    /// Simpler format where the patch document mirrors the structure
    /// of the resource with only changed fields.
    MergePatch(Value),
}

/// Extension trait for conditional operations based on search criteria.
#[async_trait]
pub trait ConditionalStorage: ResourceStorage {
    /// Creates a resource only if no matching resource exists.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `resource_type` - The FHIR resource type
    /// * `resource` - The resource to create
    /// * `search_params` - Search parameters to check for existing match
    /// * `fhir_version` - The FHIR specification version for this resource
    ///
    /// # Returns
    ///
    /// * `Created` - If no match was found and resource was created
    /// * `Exists` - If exactly one matching resource was found
    /// * `MultipleMatches` - If multiple matching resources were found (error)
    async fn conditional_create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        fhir_version: FhirVersion,
    ) -> StorageResult<ConditionalCreateResult>;

    /// Updates a resource based on search criteria.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `resource_type` - The FHIR resource type
    /// * `resource` - The new resource content
    /// * `search_params` - Search parameters to find the resource
    /// * `upsert` - If true, create if no match found
    /// * `fhir_version` - The FHIR specification version for this resource (used if creating)
    ///
    /// # Returns
    ///
    /// * `Updated` - If exactly one match was found and updated
    /// * `Created` - If no match was found and upsert is true
    /// * `NoMatch` - If no match was found and upsert is false
    /// * `MultipleMatches` - If multiple matches were found (error)
    async fn conditional_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        upsert: bool,
        fhir_version: FhirVersion,
    ) -> StorageResult<ConditionalUpdateResult>;

    /// Deletes a resource based on search criteria.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `resource_type` - The FHIR resource type
    /// * `search_params` - Search parameters to find the resource
    ///
    /// # Returns
    ///
    /// * `Deleted` - If exactly one match was found and deleted
    /// * `NoMatch` - If no match was found
    /// * `MultipleMatches` - If multiple matches were found (error)
    async fn conditional_delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
    ) -> StorageResult<ConditionalDeleteResult>;

    /// Patches a resource based on search criteria.
    ///
    /// This implements conditional patch as defined in FHIR:
    /// `PATCH [base]/[type]?[search-params]`
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `resource_type` - The FHIR resource type
    /// * `search_params` - Search parameters to find the resource
    /// * `patch` - The patch to apply (JSON Patch, FHIRPath Patch, or Merge Patch)
    ///
    /// # Returns
    ///
    /// * `Patched` - If exactly one match was found and patched
    /// * `NoMatch` - If no match was found
    /// * `MultipleMatches` - If multiple matches were found (error)
    ///
    /// # Errors
    ///
    /// * `StorageError::Validation` - If the patch is invalid or would create invalid resource
    /// * `StorageError::Backend(NotSupported)` - If conditional patch is not supported
    async fn conditional_patch(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
        patch: &PatchFormat,
    ) -> StorageResult<ConditionalPatchResult> {
        // Default implementation returns NotSupported
        let _ = (tenant, resource_type, search_params, patch);
        Err(StorageError::Backend(
            crate::error::BackendError::UnsupportedCapability {
                backend_name: "unknown".to_string(),
                capability: "conditional_patch".to_string(),
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conditional_create_result_debug() {
        let result = ConditionalCreateResult::MultipleMatches(3);
        let debug = format!("{:?}", result);
        assert!(debug.contains("MultipleMatches"));
        assert!(debug.contains("3"));
    }

    #[test]
    fn test_conditional_update_result_variants() {
        let _created = ConditionalUpdateResult::Created(StoredResource::new(
            "Patient",
            "123",
            crate::tenant::TenantId::new("t1"),
            serde_json::json!({}),
            FhirVersion::default(),
        ));
        let _no_match = ConditionalUpdateResult::NoMatch;
        let _multiple = ConditionalUpdateResult::MultipleMatches(2);
    }

    #[test]
    fn test_conditional_delete_result_variants() {
        let _deleted = ConditionalDeleteResult::Deleted;
        let _no_match = ConditionalDeleteResult::NoMatch;
        let _multiple = ConditionalDeleteResult::MultipleMatches(5);
    }
}
