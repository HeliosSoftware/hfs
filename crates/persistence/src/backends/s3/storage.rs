//! ResourceStorage, VersionedStorage, and history provider implementations
//! for the S3 backend, plus shared helper methods for JSON serialization,
//! object I/O, and history index maintenance.

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use uuid::Uuid;

use crate::core::history::{
    HistoryEntry, HistoryMethod, HistoryPage, HistoryParams, InstanceHistoryProvider,
    SystemHistoryProvider, TypeHistoryProvider,
};
use crate::core::{
    PurgableStorage, ResourceStorage, VersionedStorage, if_match_field_satisfied, normalize_etag,
};
use crate::error::{
    BackendError, ConcurrencyError, ResourceError, SearchError, StorageError, StorageResult,
};
use crate::search::reindex::{ReindexSource, ResourcePage};
use crate::tenant::{Operation, TenantContext, TenantId, TenantPermissions};
use crate::types::{
    CursorValue, Page, PageCursor, PageInfo, Pagination, PaginationMode, ResourceMethod,
    StoredResource,
};

use super::backend::{S3Backend, TenantLocation};
use super::client::{ListObjectItem, ObjectMetadata};
use super::models::HistoryIndexEvent;

/// A loaded current resource together with its S3 ETag.
///
/// The ETag is used as the optimistic concurrency token for subsequent
/// conditional writes (`If-Match` on update, `If-None-Match: *` on create).
#[derive(Debug, Clone)]
pub(crate) struct CurrentResourceWithMeta {
    /// The stored resource content and metadata.
    pub resource: StoredResource,
    /// S3 ETag of the object at the time it was fetched.
    pub etag: Option<String>,
}

impl S3Backend {
    /// Serialises `value` to a JSON byte vector.
    pub(crate) fn serialize_json<T: Serialize>(&self, value: &T) -> StorageResult<Vec<u8>> {
        serde_json::to_vec(value).map_err(|e| {
            StorageError::Backend(BackendError::SerializationError {
                message: format!("failed to serialize JSON payload: {e}"),
            })
        })
    }

    /// Deserialises a JSON byte slice into `T`.
    pub(crate) fn deserialize_json<T: DeserializeOwned>(&self, bytes: &[u8]) -> StorageResult<T> {
        serde_json::from_slice(bytes).map_err(|e| {
            StorageError::Backend(BackendError::SerializationError {
                message: format!("failed to deserialize JSON payload: {e}"),
            })
        })
    }

    /// Writes a JSON byte payload to `key` with optional ETag preconditions.
    ///
    /// - `if_match`: the object must exist with exactly this ETag.
    /// - `if_none_match`: typically `"*"` to prevent overwriting an existing
    ///   object.
    pub(crate) async fn put_json_object(
        &self,
        bucket: &str,
        key: &str,
        value: &[u8],
        if_match: Option<&str>,
        if_none_match: Option<&str>,
    ) -> StorageResult<ObjectMetadata> {
        self.client
            .put_object(
                bucket,
                key,
                value.to_vec(),
                Some("application/json"),
                if_match,
                if_none_match,
            )
            .await
            .map_err(|e| self.map_client_error(e))
    }

    /// Writes raw bytes to `key` with the given content type.
    ///
    /// No conditional preconditions are applied; used for bulk export NDJSON
    /// output parts and raw NDJSON archival.
    pub(crate) async fn put_bytes_object(
        &self,
        bucket: &str,
        key: &str,
        value: &[u8],
        content_type: Option<&str>,
    ) -> StorageResult<ObjectMetadata> {
        self.client
            .put_object(bucket, key, value.to_vec(), content_type, None, None)
            .await
            .map_err(|e| self.map_client_error(e))
    }

    /// Downloads and deserialises a JSON object, returning `None` if not found.
    pub(crate) async fn get_json_object<T: DeserializeOwned>(
        &self,
        bucket: &str,
        key: &str,
    ) -> StorageResult<Option<(T, ObjectMetadata)>> {
        match self.client.get_object(bucket, key).await {
            Ok(Some(object)) => {
                let value = self.deserialize_json::<T>(&object.bytes)?;
                Ok(Some((value, object.metadata)))
            }
            Ok(None) => Ok(None),
            Err(err) => Err(self.map_client_error(err)),
        }
    }

    /// Exhaustively lists all objects under `prefix`, auto-paginating through
    /// S3 continuation tokens until the full result set is collected.
    pub(crate) async fn list_objects_all(
        &self,
        bucket: &str,
        prefix: &str,
    ) -> StorageResult<Vec<ListObjectItem>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;

        loop {
            let page = self
                .client
                .list_objects(bucket, prefix, token.as_deref(), Some(1000))
                .await
                .map_err(|e| self.map_client_error(e))?;
            out.extend(page.items);
            token = page.next_continuation_token;
            if token.is_none() {
                break;
            }
        }

        Ok(out)
    }

    /// Loads the current resource pointer together with its S3 ETag.
    ///
    /// Returns `None` if the resource has never been created. Does not check
    /// whether the resource is logically deleted — callers must check
    /// `StoredResource::is_deleted()` themselves.
    pub(crate) async fn load_current_with_meta(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<CurrentResourceWithMeta>> {
        let location = self.tenant_location(tenant)?;
        let key = location.keyspace.current_resource_key(resource_type, id);

        let loaded = self
            .get_json_object::<StoredResource>(&location.bucket, &key)
            .await?;

        Ok(loaded.map(|(resource, metadata)| CurrentResourceWithMeta {
            resource,
            etag: metadata.etag,
        }))
    }

    /// Writes the versioned history snapshot and both history index event keys
    /// for a resource mutation.
    ///
    /// Three objects are written per mutation:
    /// - The immutable history snapshot under `_history/<version>.json`.
    /// - A type-level history index event under `history/type/<type>/…`.
    /// - A system-level history index event under `history/system/…`.
    pub(crate) async fn put_history_and_indexes(
        &self,
        location: &TenantLocation,
        resource: &StoredResource,
        method: HistoryMethod,
    ) -> StorageResult<()> {
        let history_key = location.keyspace.history_version_key(
            resource.resource_type(),
            resource.id(),
            resource.version_id(),
        );
        let payload = self.serialize_json(resource)?;
        self.put_json_object(&location.bucket, &history_key, &payload, None, None)
            .await?;

        let event = HistoryIndexEvent {
            resource_type: resource.resource_type().to_string(),
            id: resource.id().to_string(),
            version_id: resource.version_id().to_string(),
            timestamp: resource.last_modified(),
            method,
            deleted: resource.is_deleted(),
        };
        let event_payload = self.serialize_json(&event)?;
        let suffix = Uuid::new_v4().simple().to_string();

        let type_key = location.keyspace.history_type_event_key(
            resource.resource_type(),
            resource.last_modified(),
            resource.id(),
            resource.version_id(),
            &suffix,
        );
        let system_key = location.keyspace.history_system_event_key(
            resource.resource_type(),
            resource.last_modified(),
            resource.id(),
            resource.version_id(),
            &suffix,
        );

        self.put_json_object(&location.bucket, &type_key, &event_payload, None, None)
            .await?;
        self.put_json_object(&location.bucket, &system_key, &event_payload, None, None)
            .await?;

        Ok(())
    }

    /// Derives the `HistoryMethod` for a stored resource from its own method
    /// field, falling back to `Delete` or `Put` based on the deletion flag.
    pub(crate) fn history_method_for(resource: &StoredResource) -> HistoryMethod {
        match resource.method() {
            Some(ResourceMethod::Post) => HistoryMethod::Post,
            Some(ResourceMethod::Put) => HistoryMethod::Put,
            Some(ResourceMethod::Patch) => HistoryMethod::Patch,
            Some(ResourceMethod::Delete) => HistoryMethod::Delete,
            None => {
                if resource.is_deleted() {
                    HistoryMethod::Delete
                } else {
                    HistoryMethod::Put
                }
            }
        }
    }

    /// Sorts entries by timestamp descending and returns a cursor-paginated page.
    ///
    /// The cursor encodes a simple offset into the sorted list; both forward
    /// and backward cursors are generated so callers can navigate in either
    /// direction.
    pub(crate) fn page_history(
        &self,
        mut entries: Vec<HistoryEntry>,
        pagination: &Pagination,
    ) -> StorageResult<HistoryPage> {
        entries.sort_by_key(|e| std::cmp::Reverse(e.timestamp));

        let total = entries.len();
        let offset = decode_pagination_offset(pagination)?;
        let count = pagination.count as usize;
        let end = offset.saturating_add(count).min(total);

        let items = if offset >= total {
            Vec::new()
        } else {
            entries[offset..end].to_vec()
        };

        let has_next = end < total;
        let has_previous = offset > 0;

        let next_cursor = if has_next {
            Some(PageCursor::new(vec![CursorValue::Number(end as i64)], end.to_string()).encode())
        } else {
            None
        };

        let previous_cursor = if has_previous {
            let prev = offset.saturating_sub(count);
            Some(PageCursor::new(vec![CursorValue::Number(prev as i64)], prev.to_string()).encode())
        } else {
            None
        };

        Ok(Page::new(
            items,
            PageInfo {
                next_cursor,
                previous_cursor,
                total: Some(total as u64),
                has_next,
                has_previous,
            },
        ))
    }

    /// Returns all keys ending with `/current.json` under the given resource
    /// type prefix (or the entire resource tree if `resource_type` is `None`).
    pub(crate) async fn list_current_keys(
        &self,
        location: &TenantLocation,
        resource_type: Option<&str>,
    ) -> StorageResult<Vec<String>> {
        let prefix = if let Some(resource_type) = resource_type {
            location.keyspace.resource_type_prefix(resource_type)
        } else {
            location.keyspace.resources_prefix()
        };

        let keys = self
            .list_objects_all(&location.bucket, &prefix)
            .await?
            .into_iter()
            .map(|i| i.key)
            .filter(|key| key.ends_with("/current.json"))
            .collect();

        Ok(keys)
    }

    /// Scans every live (non-deleted) resource of `resource_type` for `tenant`
    /// and returns their raw FHIR JSON content.
    ///
    /// Shares the `list_current_keys` + `get_json_object` walk used by bulk
    /// export ([`super::bulk_export`]); used by the in-process SQL-on-FHIR
    /// runner to feed the `helios-sof` engine.
    pub(crate) async fn scan_live_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<Vec<Value>> {
        let location = self.tenant_location(tenant)?;
        let keys = self
            .list_current_keys(&location, Some(resource_type))
            .await?;

        let mut resources = Vec::new();
        for key in keys {
            let Some((resource, _)) = self
                .get_json_object::<StoredResource>(&location.bucket, &key)
                .await?
            else {
                continue;
            };
            if resource.is_deleted() {
                continue;
            }
            resources.push(resource.content().clone());
        }
        Ok(resources)
    }

    /// Loads history entries by scanning all index event objects under `prefix`.
    ///
    /// For each event key found, the corresponding versioned history snapshot is
    /// fetched and assembled into a `HistoryEntry`. Objects that fail to parse
    /// are silently skipped.
    pub(crate) async fn load_history_event_entries(
        &self,
        location: &TenantLocation,
        prefix: &str,
    ) -> StorageResult<Vec<HistoryEntry>> {
        let mut entries = Vec::new();
        let objects = self.list_objects_all(&location.bucket, prefix).await?;

        for object in objects {
            let Some((event, _)) = self
                .get_json_object::<HistoryIndexEvent>(&location.bucket, &object.key)
                .await?
            else {
                continue;
            };

            let history_key = location.keyspace.history_version_key(
                &event.resource_type,
                &event.id,
                &event.version_id,
            );

            if let Some((resource, _)) = self
                .get_json_object::<StoredResource>(&location.bucket, &history_key)
                .await?
            {
                entries.push(HistoryEntry {
                    resource,
                    method: event.method,
                    timestamp: event.timestamp,
                });
            }
        }

        Ok(entries)
    }

    /// Ensures the resource JSON contains the correct `resourceType` and `id`
    /// fields, inserting them if they are absent or incorrect.
    pub(crate) fn ensure_resource_shape(
        &self,
        resource_type: &str,
        id: &str,
        mut resource: Value,
    ) -> Value {
        if let Some(object) = resource.as_object_mut() {
            object.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
            object.insert("id".to_string(), Value::String(id.to_string()));
        }
        resource
    }

    /// Brings a soft-deleted resource back to life with new content.
    ///
    /// FHIR permits a deleted resource to be restored by a subsequent update
    /// ([http.html#delete](https://hl7.org/fhir/http.html#delete)), so a `PUT`
    /// onto a deleted id must succeed instead of failing with `Gone`. The
    /// restored resource continues the existing version chain (the deletion
    /// record keeps its version, the restore gets the next one) and keeps the
    /// FHIR version the resource was originally stored under.
    ///
    /// Returns `NotFound` if no deleted object is present — the caller has
    /// already established one exists, so that only happens under a concurrent
    /// write.
    async fn restore_deleted(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        tenant.check_permission(Operation::Update, resource_type)?;

        let location = self.tenant_location(tenant)?;
        let current_key = location.keyspace.current_resource_key(resource_type, id);

        let Some(actual) = self
            .load_current_with_meta(tenant, resource_type, id)
            .await?
        else {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        };

        if !actual.resource.is_deleted() {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        }

        let deleted_version = actual.resource.version_id().to_string();
        let new_content = self.ensure_resource_shape(resource_type, id, resource);
        // `new_version` clears `deleted_at` — the exact inverse of the
        // `mark_deleted` performed by `delete` — while keeping the original
        // creation time and FHIR version and taking the next version number.
        let restored = actual
            .resource
            .new_version(new_content, ResourceMethod::Put);

        let payload = self.serialize_json(&restored)?;
        match self
            .put_json_object(
                &location.bucket,
                &current_key,
                &payload,
                actual.etag.as_deref(),
                None,
            )
            .await
        {
            Ok(_) => {
                self.put_history_and_indexes(&location, &restored, HistoryMethod::Put)
                    .await?;
                Ok(restored)
            }
            Err(StorageError::Backend(BackendError::QueryError { .. })) => {
                let latest = self
                    .load_current_with_meta(tenant, resource_type, id)
                    .await?
                    .map(|v| v.resource.version_id().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                Err(StorageError::Concurrency(
                    ConcurrencyError::VersionConflict {
                        resource_type: resource_type.to_string(),
                        id: id.to_string(),
                        expected_version: deleted_version,
                        actual_version: latest,
                    },
                ))
            }
            Err(err) => Err(err),
        }
    }

    /// Restores a resource snapshot as the latest version.
    ///
    /// If a current version exists (including tombstones), this writes a new
    /// version from that current pointer. If the resource is missing, this
    /// recreates version `1` from the snapshot content.
    pub(crate) async fn restore_resource_from_snapshot(
        &self,
        tenant: &TenantContext,
        snapshot: &StoredResource,
    ) -> StorageResult<StoredResource> {
        let location = self.tenant_location(tenant)?;
        let resource_type = snapshot.resource_type();
        let id = snapshot.id();
        let current_key = location.keyspace.current_resource_key(resource_type, id);

        let content = self.ensure_resource_shape(resource_type, id, snapshot.content().clone());

        if let Some(current) = self
            .load_current_with_meta(tenant, resource_type, id)
            .await?
        {
            let restored = current.resource.new_version(content, ResourceMethod::Put);
            let payload = self.serialize_json(&restored)?;
            self.put_json_object(
                &location.bucket,
                &current_key,
                &payload,
                current.etag.as_deref(),
                None,
            )
            .await?;
            self.put_history_and_indexes(&location, &restored, HistoryMethod::Put)
                .await?;
            Ok(restored)
        } else {
            let restored = StoredResource::new(
                resource_type,
                id,
                tenant.tenant_id().clone(),
                content,
                snapshot.fhir_version(),
            );
            let payload = self.serialize_json(&restored)?;
            self.put_json_object(&location.bucket, &current_key, &payload, None, Some("*"))
                .await?;
            self.put_history_and_indexes(&location, &restored, HistoryMethod::Post)
                .await?;
            Ok(restored)
        }
    }
}

#[async_trait]
impl ResourceStorage for S3Backend {
    fn backend_name(&self) -> &'static str {
        "s3"
    }

    async fn readiness_check(&self) -> Result<(), BackendError> {
        <Self as crate::core::Backend>::health_check(self).await
    }

    fn is_cluster_shared(&self) -> bool {
        // S3 is a networked object store shared by every instance; `count` (and
        // thus the console `count_by_types` totals) reads the shared bucket, so
        // the counts this backend produces are cluster-consistent.
        true
    }

    fn sof_runner(&self) -> Option<std::sync::Arc<dyn crate::core::sof_runner::SofRunner>> {
        use crate::sof::in_process::{InProcessSofRunner, ResourceScan};
        use crate::sof::reference_resolver::StorageBackedResolver;
        // S3 is object storage with no query engine, so SQL-on-FHIR runs
        // in-process over the scanned resources via the `helios-sof` engine.
        let scan: std::sync::Arc<dyn ResourceScan> = std::sync::Arc::new(self.clone());
        // Enable storage-backed `resolve()`: the same S3 backend serves as the
        // tenant-scoped reference resolver, so a view's `reference.resolve()` can
        // dereference a stored `Type/id` that is not in the scanned set.
        let resolver = std::sync::Arc::new(StorageBackedResolver::new(
            std::sync::Arc::new(self.clone()),
            StorageBackedResolver::DEFAULT_MAX_FANOUT,
        ));
        Some(std::sync::Arc::new(
            InProcessSofRunner::new(scan, FhirVersion::default_enabled(), "s3-in-process")
                .with_reference_resolver(resolver),
        ))
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        tenant.check_permission(Operation::Create, resource_type)?;

        let location = self.tenant_location(tenant)?;

        let id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        let current_key = location.keyspace.current_resource_key(resource_type, &id);

        if self
            .client
            .head_object(&location.bucket, &current_key)
            .await
            .map_err(|e| self.map_client_error(e))?
            .is_some()
        {
            return Err(StorageError::Resource(ResourceError::AlreadyExists {
                resource_type: resource_type.to_string(),
                id,
            }));
        }

        let content = self.ensure_resource_shape(resource_type, &id, resource);
        let stored = StoredResource::new(
            resource_type,
            &id,
            tenant.tenant_id().clone(),
            content,
            fhir_version,
        );

        let payload = self.serialize_json(&stored)?;
        match self
            .put_json_object(&location.bucket, &current_key, &payload, None, Some("*"))
            .await
        {
            Ok(_) => {
                self.put_history_and_indexes(&location, &stored, HistoryMethod::Post)
                    .await?;
                Ok(stored)
            }
            Err(StorageError::Backend(BackendError::QueryError { .. })) => {
                Err(StorageError::Resource(ResourceError::AlreadyExists {
                    resource_type: resource_type.to_string(),
                    id,
                }))
            }
            Err(e) => Err(e),
        }
    }

    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        match self.read(tenant, resource_type, id).await {
            Ok(Some(current)) => {
                let updated = self.update(tenant, &current, resource).await?;
                Ok((updated, false))
            }
            Ok(None) => {
                let created = self
                    .create(
                        tenant,
                        resource_type,
                        self.ensure_resource_shape(resource_type, id, resource),
                        fhir_version,
                    )
                    .await?;
                Ok((created, true))
            }
            // A deleted resource is brought back to life by a subsequent update
            // (FHIR http.html#delete), continuing the existing version chain
            // rather than being rejected with `Gone`.
            Err(StorageError::Resource(ResourceError::Gone { .. })) => {
                let restored = self
                    .restore_deleted(tenant, resource_type, id, resource)
                    .await?;
                Ok((restored, true))
            }
            Err(err) => Err(err),
        }
    }

    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let Some(current) = self
            .load_current_with_meta(tenant, resource_type, id)
            .await?
        else {
            return Ok(None);
        };

        if current.resource.is_deleted() {
            return Err(StorageError::Resource(ResourceError::Gone {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
                deleted_at: current.resource.deleted_at(),
            }));
        }

        Ok(Some(current.resource))
    }

    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        let resource_type = current.resource_type();
        tenant.check_permission(Operation::Update, resource_type)?;

        let location = self.tenant_location(tenant)?;
        let id = current.id();
        let current_key = location.keyspace.current_resource_key(resource_type, id);

        let Some(actual) = self
            .load_current_with_meta(tenant, resource_type, id)
            .await?
        else {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        };

        if actual.resource.is_deleted() {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        }

        if actual.resource.version_id() != current.version_id() {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: current.version_id().to_string(),
                    actual_version: actual.resource.version_id().to_string(),
                },
            ));
        }

        let new_content = self.ensure_resource_shape(resource_type, id, resource);
        let updated = actual
            .resource
            .new_version(new_content, ResourceMethod::Put);

        let payload = self.serialize_json(&updated)?;
        match self
            .put_json_object(
                &location.bucket,
                &current_key,
                &payload,
                actual.etag.as_deref(),
                None,
            )
            .await
        {
            Ok(_) => {
                self.put_history_and_indexes(&location, &updated, HistoryMethod::Put)
                    .await?;
                Ok(updated)
            }
            Err(StorageError::Backend(BackendError::QueryError { .. })) => {
                let latest = self
                    .load_current_with_meta(tenant, resource_type, id)
                    .await?
                    .map(|v| v.resource.version_id().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                Err(StorageError::Concurrency(
                    ConcurrencyError::VersionConflict {
                        resource_type: resource_type.to_string(),
                        id: id.to_string(),
                        expected_version: current.version_id().to_string(),
                        actual_version: latest,
                    },
                ))
            }
            Err(err) => Err(err),
        }
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        tenant.check_permission(Operation::Delete, resource_type)?;

        let location = self.tenant_location(tenant)?;
        let current_key = location.keyspace.current_resource_key(resource_type, id);

        let Some(actual) = self
            .load_current_with_meta(tenant, resource_type, id)
            .await?
        else {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        };

        if actual.resource.is_deleted() {
            return Err(StorageError::Resource(ResourceError::Gone {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
                deleted_at: actual.resource.deleted_at(),
            }));
        }

        let deleted = actual.resource.mark_deleted();
        let payload = self.serialize_json(&deleted)?;

        match self
            .put_json_object(
                &location.bucket,
                &current_key,
                &payload,
                actual.etag.as_deref(),
                None,
            )
            .await
        {
            Ok(_) => {
                self.put_history_and_indexes(&location, &deleted, HistoryMethod::Delete)
                    .await?;
                Ok(())
            }
            Err(StorageError::Backend(BackendError::QueryError { .. })) => Err(
                StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_etag: actual.etag.unwrap_or_default(),
                    actual_etag: None,
                }),
            ),
            Err(err) => Err(err),
        }
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        let location = self.tenant_location(tenant)?;
        let keys = self.list_current_keys(&location, resource_type).await?;

        let mut count = 0u64;
        for key in keys {
            if let Some((resource, _)) = self
                .get_json_object::<StoredResource>(&location.bucket, &key)
                .await?
            {
                if !resource.is_deleted() {
                    count += 1;
                }
            }
        }

        Ok(count)
    }

    async fn count_by_tenant(&self) -> StorageResult<Vec<(String, u64)>> {
        // Feeds tenant discovery on the maintenance page (#330): tenants whose
        // data outlives their registration must stay visible and purgeable.
        //
        // LIST-only by design — one delimiter LIST enumerates the tenant
        // prefixes, then paginated LISTs count each tenant's current-pointer
        // objects. No per-object GETs (see #326), which means delete tombstones
        // are counted too: a tombstone is still an object in the bucket, and
        // "this tenant still has purgeable data" is exactly what this count
        // exists to say.
        //
        // `BucketPerTenant` stays unsupported (empty result), matching the
        // tenant-registry carve-out for that mode: tenants live in a static
        // bucket map there, so there is no bucket to discover strays in.
        let super::config::S3TenancyMode::PrefixPerTenant { bucket } = &self.config.tenancy_mode
        else {
            return Ok(Vec::new());
        };

        let root = match self.global_prefix() {
            Some(prefix) => format!("{}/", prefix),
            None => String::new(),
        };

        let tenant_prefixes = self
            .client
            .list_common_prefixes(bucket, &root, "/")
            .await
            .map_err(|e| self.map_client_error(e))?;

        let mut out = Vec::new();
        for tenant_prefix in tenant_prefixes {
            let segment = tenant_prefix
                .strip_prefix(&root)
                .unwrap_or(&tenant_prefix)
                .trim_end_matches('/');
            if segment.is_empty() {
                continue;
            }

            // Only the resources namespace: non-tenant top-level groups (the
            // tenant registry, user settings, bulk-submit state) have no
            // `resources/` subtree and naturally count zero.
            let resources_prefix = format!("{}resources/", tenant_prefix);
            let mut count = 0u64;
            let mut continuation: Option<String> = None;
            loop {
                let page = self
                    .client
                    .list_objects(bucket, &resources_prefix, continuation.as_deref(), None)
                    .await
                    .map_err(|e| self.map_client_error(e))?;
                count += page
                    .items
                    .iter()
                    .filter(|item| item.key.ends_with("/current.json"))
                    .count() as u64;
                match page.next_continuation_token {
                    Some(token) => continuation = Some(token),
                    None => break,
                }
            }

            if count > 0 {
                out.push((segment.to_string(), count));
            }
        }

        Ok(out)
    }

    // ---- Tenant registry ----------------------------------------------------
    //
    // One JSON object per registered tenant at `[prefix/]tenants/<id>.json`,
    // outside any tenant prefix (see `S3Backend::registry_location`). In
    // bucket-per-tenant mode without a default system bucket there is nowhere
    // cross-tenant to keep the records, so the registry is unsupported there.

    fn bulk_write_concurrency(&self) -> usize {
        // One PUT per resource; S3 absorbs parallel writers trivially, and this
        // turns a ~1.4k-object conformance seed from minutes into seconds.
        32
    }

    fn supports_tenant_registry(&self) -> bool {
        self.registry_location().is_some()
    }

    async fn list_tenants(&self) -> StorageResult<Vec<crate::core::TenantRecord>> {
        let Some(location) = self.registry_location() else {
            return Ok(Vec::new());
        };
        let prefix = location.keyspace.tenant_registry_prefix();
        let items = self.list_objects_all(&location.bucket, &prefix).await?;
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            if !item.key.ends_with(".json") {
                continue;
            }
            // Registry records are direct children of the registry prefix; every
            // tenant-scoped key is nested at least one segment deeper (see
            // `S3Keyspace::tenant_registry_prefix`). S3 listings are recursive, so
            // without this a tenant named `tenants` has its own resource and
            // history objects read back as registry records (issue #271).
            let Some(relative) = item.key.strip_prefix(&prefix) else {
                continue;
            };
            if relative.contains('/') {
                continue;
            }
            match self
                .get_json_object::<crate::core::TenantRecord>(&location.bucket, &item.key)
                .await
            {
                Ok(Some((record, _))) => out.push(record),
                Ok(None) => {}
                // Degrade per record only for a payload we cannot parse — a
                // foreign object, a truncated write, a future schema change.
                // One such object must not make the whole tenant list fail
                // permanently, which is the unrecoverable half of #271.
                //
                // Transport and permission failures still propagate: silently
                // dropping a tenant because S3 hiccuped would under-report the
                // registry, which is worse than returning an error the caller
                // can retry.
                Err(StorageError::Backend(BackendError::SerializationError { message })) => {
                    tracing::warn!(
                        key = %item.key,
                        error = %message,
                        "skipping unparseable tenant registry record"
                    );
                }
                Err(e) => return Err(e),
            }
        }
        // A tenant registered before the `sanitize` fix and re-registered after it
        // would have a record under both key shapes; report it once, keeping the
        // earliest registration. Group by id first — `dedup_by` only collapses
        // adjacent entries, and the display order below is by timestamp.
        out.sort_by(|a, b| {
            a.id.cmp(&b.id)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });
        out.dedup_by(|a, b| a.id == b.id);
        out.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(out)
    }

    async fn get_tenant(&self, id: &str) -> StorageResult<Option<crate::core::TenantRecord>> {
        let Some(location) = self.registry_location() else {
            return Ok(None);
        };
        let key = location.keyspace.tenant_registry_key(id);
        if let Some((record, _)) = self
            .get_json_object::<crate::core::TenantRecord>(&location.bucket, &key)
            .await?
        {
            return Ok(Some(record));
        }
        // Fall back to the pre-escaping key so records written before the
        // `sanitize` fix stay readable. Only ids containing `/`, `\`, or space
        // differ between the two shapes, so this is a miss-only extra GET for
        // every other tenant.
        let legacy = location.keyspace.legacy_tenant_registry_key(id);
        if legacy == key {
            return Ok(None);
        }
        // The legacy key is ambiguous: `sanitize` mapped `a/b` to `a_b`, which is
        // also the *canonical* key of a tenant literally named `a_b`. So the key
        // alone cannot establish ownership — the record body decides. Without
        // this check the fallback would re-open the very cross-tenant read the
        // escaping change closes.
        Ok(self
            .get_json_object::<crate::core::TenantRecord>(&location.bucket, &legacy)
            .await?
            .map(|(record, _)| record)
            .filter(|record| record.id == id))
    }

    async fn register_tenant(
        &self,
        id: &str,
        display_name: Option<&str>,
    ) -> StorageResult<crate::core::TenantRecord> {
        let location = self
            .registry_location()
            .ok_or_else(|| self.tenant_registry_unsupported())?;
        let record = crate::core::TenantRecord {
            id: id.to_string(),
            display_name: display_name.map(str::to_string),
            // RFC 3339, matching the SQL registries' `created_at` format.
            created_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        };
        let key = location.keyspace.tenant_registry_key(id);
        let bytes = self.serialize_json(&record)?;
        // `If-None-Match: *` so a concurrent double-register surfaces as a
        // precondition failure; the admin handler pre-checks existence and
        // returns 409, so reaching here with a duplicate is a race and a 500
        // is acceptable.
        self.put_json_object(&location.bucket, &key, &bytes, None, Some("*"))
            .await?;
        Ok(record)
    }

    async fn deregister_tenant(&self, id: &str) -> StorageResult<bool> {
        let location = self
            .registry_location()
            .ok_or_else(|| self.tenant_registry_unsupported())?;
        // Delete both key shapes so deregistering a tenant registered before the
        // `sanitize` fix actually removes its record. S3 deletes are silently
        // idempotent, so probe each first to report whether anything was removed.
        //
        // The probe is not just a nicety here: the legacy key is ambiguous —
        // `sanitize` mapped `a/b` to `a_b`, which is also the *canonical* key of
        // a tenant named `a_b` — so the record body must confirm ownership
        // before deleting. Skipping that would make deregistering `a/b` silently
        // destroy `a_b`'s registration.
        let key = location.keyspace.tenant_registry_key(id);
        let legacy = location.keyspace.legacy_tenant_registry_key(id);
        let mut candidates = vec![key.clone()];
        if legacy != key {
            candidates.push(legacy);
        }
        let mut removed = false;
        for candidate in candidates {
            let Some((record, _)) = self
                .get_json_object::<crate::core::TenantRecord>(&location.bucket, &candidate)
                .await?
            else {
                continue;
            };
            if record.id != id {
                continue;
            }
            self.client
                .delete_object(&location.bucket, &candidate)
                .await
                .map_err(|e| self.map_client_error(e))?;
            removed = true;
        }
        Ok(removed)
    }

    async fn purge_tenant_data(&self, id: &str) -> StorageResult<u64> {
        // Resolve the tenant's data location exactly as request handling does.
        let tenant = TenantContext::new(TenantId::new(id), TenantPermissions::full_access());
        let location = self.tenant_location(&tenant)?;
        // Count current-version pointers first (tombstones included, mirroring
        // the SQLite purge) so we can report what was removed.
        let removed = self.list_current_keys(&location, None).await?.len() as u64;
        // Sweep resources and history, mirroring the SQLite purge — bulk
        // export/submit artifacts are left alone there too.
        for prefix in [
            location.keyspace.resources_prefix(),
            location.keyspace.history_root_prefix(),
        ] {
            for item in self.list_objects_all(&location.bucket, &prefix).await? {
                self.client
                    .delete_object(&location.bucket, &item.key)
                    .await
                    .map_err(|e| self.map_client_error(e))?;
            }
        }
        Ok(removed)
    }
}

#[async_trait]
impl VersionedStorage for S3Backend {
    async fn vread(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        version_id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let location = self.tenant_location(tenant)?;
        let key = location
            .keyspace
            .history_version_key(resource_type, id, version_id);

        let resource = self
            .get_json_object::<StoredResource>(&location.bucket, &key)
            .await?
            .map(|(r, _)| r);

        Ok(resource)
    }

    async fn update_with_match(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        let Some(actual) = self
            .load_current_with_meta(tenant, resource_type, id)
            .await?
        else {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        };

        if actual.resource.is_deleted() {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        }

        // `expected_version` is the client's `If-Match` field value, which is a
        // LIST and is satisfied when any listed tag matches (issue #311).
        let actual_version = actual.resource.version_id();
        if !if_match_field_satisfied(expected_version, actual_version) {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: normalize_etag(expected_version).to_string(),
                    actual_version: actual_version.to_string(),
                },
            ));
        }

        self.update(tenant, &actual.resource, resource).await
    }

    async fn delete_with_match(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
    ) -> StorageResult<()> {
        let Some(actual) = self
            .load_current_with_meta(tenant, resource_type, id)
            .await?
        else {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        };

        // `expected_version` is the client's `If-Match` field value, which is a
        // LIST and is satisfied when any listed tag matches (issue #311).
        let actual_version = actual.resource.version_id();
        if !if_match_field_satisfied(expected_version, actual_version) {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: normalize_etag(expected_version).to_string(),
                    actual_version: actual_version.to_string(),
                },
            ));
        }

        self.delete(tenant, resource_type, id).await
    }

    async fn list_versions(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Vec<String>> {
        let location = self.tenant_location(tenant)?;
        let prefix = location.keyspace.history_versions_prefix(resource_type, id);

        let mut versions = Vec::new();
        for object in self.list_objects_all(&location.bucket, &prefix).await? {
            let Some(version) = parse_version_from_history_key(&object.key) else {
                continue;
            };
            versions.push(version);
        }

        versions.sort_by_key(|v| v.parse::<u64>().unwrap_or_default());
        versions.dedup();
        Ok(versions)
    }
}

#[async_trait]
impl InstanceHistoryProvider for S3Backend {
    async fn history_instance(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let versions = self.list_versions(tenant, resource_type, id).await?;
        let mut entries = Vec::new();

        for version in versions {
            let Some(resource) = self.vread(tenant, resource_type, id, &version).await? else {
                continue;
            };

            if !params.include_deleted && resource.is_deleted() {
                continue;
            }

            if let Some(since) = params.since {
                if resource.last_modified() < since {
                    continue;
                }
            }
            if let Some(before) = params.before {
                if resource.last_modified() >= before {
                    continue;
                }
            }

            entries.push(HistoryEntry {
                method: Self::history_method_for(&resource),
                timestamp: resource.last_modified(),
                resource,
            });
        }

        self.page_history(entries, &params.pagination)
    }

    async fn history_instance_count(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<u64> {
        Ok(self.list_versions(tenant, resource_type, id).await?.len() as u64)
    }
}

#[async_trait]
impl TypeHistoryProvider for S3Backend {
    async fn history_type(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let location = self.tenant_location(tenant)?;
        let prefix = location.keyspace.history_type_prefix(resource_type);
        let mut entries = self.load_history_event_entries(&location, &prefix).await?;

        entries.retain(|entry| {
            (params.include_deleted || !entry.resource.is_deleted())
                && params
                    .since
                    .map(|since| entry.timestamp >= since)
                    .unwrap_or(true)
                && params
                    .before
                    .map(|before| entry.timestamp < before)
                    .unwrap_or(true)
        });

        self.page_history(entries, &params.pagination)
    }

    async fn history_type_count(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        let location = self.tenant_location(tenant)?;
        let prefix = location.keyspace.history_type_prefix(resource_type);
        Ok(self
            .list_objects_all(&location.bucket, &prefix)
            .await?
            .len() as u64)
    }
}

#[async_trait]
impl SystemHistoryProvider for S3Backend {
    async fn history_system(
        &self,
        tenant: &TenantContext,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let location = self.tenant_location(tenant)?;
        let prefix = location.keyspace.history_system_prefix();
        let mut entries = self.load_history_event_entries(&location, &prefix).await?;

        entries.retain(|entry| {
            (params.include_deleted || !entry.resource.is_deleted())
                && params
                    .since
                    .map(|since| entry.timestamp >= since)
                    .unwrap_or(true)
                && params
                    .before
                    .map(|before| entry.timestamp < before)
                    .unwrap_or(true)
        });

        self.page_history(entries, &params.pagination)
    }

    async fn history_system_count(&self, tenant: &TenantContext) -> StorageResult<u64> {
        let location = self.tenant_location(tenant)?;
        let prefix = location.keyspace.history_system_prefix();
        Ok(self
            .list_objects_all(&location.bucket, &prefix)
            .await?
            .len() as u64)
    }
}

/// Extracts the numeric version string from a history key filename.
///
/// History keys have the form `…/_history/<version>.json`; the version is the
/// filename stem. Returns `None` for empty stems or non-`.json` extensions.
fn parse_version_from_history_key(key: &str) -> Option<String> {
    if !key.ends_with(".json") {
        return None;
    }
    let filename = key.rsplit('/').next()?;
    let version = filename.strip_suffix(".json")?;
    if version.is_empty() {
        None
    } else {
        Some(version.to_string())
    }
}

/// Decodes the numeric offset from a history pagination struct.
///
/// Handles both explicit `Offset` mode and `Cursor` mode, where the cursor
/// encodes the offset as a `CursorValue::Number`.
fn decode_pagination_offset(pagination: &Pagination) -> StorageResult<usize> {
    match &pagination.mode {
        PaginationMode::Offset(offset) => Ok(*offset as usize),
        PaginationMode::Cursor(None) => Ok(0),
        PaginationMode::Cursor(Some(cursor)) => {
            if let Some(CursorValue::Number(offset)) = cursor.sort_values().first() {
                return Ok((*offset).max(0) as usize);
            }

            if let Ok(parsed) = cursor.resource_id().parse::<usize>() {
                return Ok(parsed);
            }

            Err(StorageError::Search(SearchError::InvalidCursor {
                cursor: cursor.encode(),
            }))
        }
    }
}

// ---------------------------------------------------------------------------
// Stub trait impls: S3 does not support search or conditional operations
// ---------------------------------------------------------------------------

use crate::core::search::{IncludeProvider, RevincludeProvider, SearchProvider, SearchResult};
use crate::core::storage::{
    ConditionalCreateResult, ConditionalDeleteResult, ConditionalStorage, ConditionalUpdateResult,
};
use crate::types::IncludeDirective;
use crate::types::SearchQuery;

#[async_trait]
impl SearchProvider for S3Backend {
    async fn search(
        &self,
        _tenant: &TenantContext,
        _query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        Err(StorageError::Backend(BackendError::UnsupportedCapability {
            backend_name: "S3".to_string(),
            capability: "search".to_string(),
        }))
    }

    async fn search_count(
        &self,
        _tenant: &TenantContext,
        _query: &SearchQuery,
    ) -> StorageResult<u64> {
        Err(StorageError::Backend(BackendError::UnsupportedCapability {
            backend_name: "S3".to_string(),
            capability: "search_count".to_string(),
        }))
    }

    fn search_param_registry(
        &self,
        _tenant: &crate::tenant::TenantContext,
    ) -> std::sync::Arc<parking_lot::RwLock<crate::search::SearchParameterRegistry>> {
        // S3 standalone does not implement search; an empty registry is
        // required only to satisfy the trait. In real deployments S3 is
        // composed with a search backend (e.g., Elasticsearch) and the
        // composite forwards to that backend's registry.
        use std::sync::OnceLock;
        static EMPTY: OnceLock<crate::search::TenantSearchRegistries> = OnceLock::new();
        EMPTY
            .get_or_init(crate::search::TenantSearchRegistries::base_only)
            .for_tenant("")
    }
}

#[async_trait]
impl IncludeProvider for S3Backend {
    async fn resolve_includes(
        &self,
        _tenant: &TenantContext,
        _resources: &[StoredResource],
        _includes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        Err(StorageError::Backend(BackendError::UnsupportedCapability {
            backend_name: "S3".to_string(),
            capability: "_include".to_string(),
        }))
    }
}

#[async_trait]
impl RevincludeProvider for S3Backend {
    async fn resolve_revincludes(
        &self,
        _tenant: &TenantContext,
        _resources: &[StoredResource],
        _revincludes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        Err(StorageError::Backend(BackendError::UnsupportedCapability {
            backend_name: "S3".to_string(),
            capability: "_revinclude".to_string(),
        }))
    }
}

#[async_trait]
impl ConditionalStorage for S3Backend {
    async fn conditional_create(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _resource: Value,
        _search_params: &str,
        _fhir_version: FhirVersion,
    ) -> StorageResult<ConditionalCreateResult> {
        Err(StorageError::Backend(BackendError::UnsupportedCapability {
            backend_name: "S3".to_string(),
            capability: "conditional_create".to_string(),
        }))
    }

    async fn conditional_update(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _resource: Value,
        _search_params: &str,
        _upsert: bool,
        _fhir_version: FhirVersion,
    ) -> StorageResult<ConditionalUpdateResult> {
        Err(StorageError::Backend(BackendError::UnsupportedCapability {
            backend_name: "S3".to_string(),
            capability: "conditional_update".to_string(),
        }))
    }

    async fn conditional_delete(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _search_params: &str,
    ) -> StorageResult<ConditionalDeleteResult> {
        Err(StorageError::Backend(BackendError::UnsupportedCapability {
            backend_name: "S3".to_string(),
            capability: "conditional_delete".to_string(),
        }))
    }
}

#[async_trait]
impl crate::sof::in_process::ResourceScan for S3Backend {
    async fn scan_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> Result<Vec<Value>, crate::core::sof_runner::SofError> {
        self.scan_live_resources(tenant, resource_type)
            .await
            .map_err(|e| crate::core::sof_runner::SofError::Storage(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_key() {
        assert_eq!(
            parse_version_from_history_key("a/b/3.json"),
            Some("3".to_string())
        );
        assert_eq!(parse_version_from_history_key("a/b/.json"), None);
        assert_eq!(parse_version_from_history_key("a/b/3"), None);
    }
}

// ============================================================================
// PurgableStorage
//
// A resource's bytes are spread across four key families in S3:
//
//   resources/{Type}/{id}/current.json            — the current version
//   resources/{Type}/{id}/_history/{version}.json — every prior version
//   history/type/{Type}/{ts}_{id}_{ver}_{sfx}.json — type-history index events
//   history/system/{ts}_{Type}_{id}_{ver}_{sfx}.json — system-history events
//
// The first two are id-prefixed and delete cleanly. The history *index event*
// objects are not: their keys are timestamp-ordered, and the id is embedded in
// a `sanitize()`d filename segment that is not injective, so two different ids
// can produce the same segment. Matching on the key text would therefore purge
// the wrong resource's events (or miss the right one's). They must be resolved
// by reading each event object and comparing the parsed resource_type/id.
//
// That makes purge O(history events for the tenant). It is the correct cost:
// leaving those objects behind means `_history` keeps returning entries for a
// purged resource, which is silent PHI retention.
// ============================================================================

impl S3Backend {
    /// Deletes every object under `prefix`, returning how many were removed.
    async fn delete_prefix(&self, bucket: &str, prefix: &str) -> StorageResult<u64> {
        let objects = self.list_objects_all(bucket, prefix).await?;
        let mut deleted = 0;
        // The S3Api trait exposes only single-key deletes (no DeleteObjects
        // batch), so this is one round trip per object.
        for object in objects {
            self.client
                .delete_object(bucket, &object.key)
                .await
                .map_err(|e| self.map_client_error(e))?;
            deleted += 1;
        }
        Ok(deleted)
    }

    /// Deletes the history index-event objects belonging to `resource_type`,
    /// optionally narrowed to a single `id`.
    ///
    /// Each candidate object is read and matched on its parsed contents rather
    /// than on its key — see the module comment above for why.
    async fn purge_history_events(
        &self,
        location: &TenantLocation,
        resource_type: &str,
        id: Option<&str>,
    ) -> StorageResult<()> {
        let prefixes = [
            location.keyspace.history_type_prefix(resource_type),
            location.keyspace.history_system_prefix(),
        ];

        for prefix in prefixes {
            for object in self.list_objects_all(&location.bucket, &prefix).await? {
                let Some((event, _)) = self
                    .get_json_object::<HistoryIndexEvent>(&location.bucket, &object.key)
                    .await?
                else {
                    continue;
                };

                if event.resource_type != resource_type {
                    continue;
                }
                if let Some(id) = id
                    && event.id != id
                {
                    continue;
                }

                self.client
                    .delete_object(&location.bucket, &object.key)
                    .await
                    .map_err(|e| self.map_client_error(e))?;
            }
        }

        Ok(())
    }
}

#[async_trait]
impl PurgableStorage for S3Backend {
    async fn purge(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        let location = self.tenant_location(tenant)?;

        let current_key = location.keyspace.current_resource_key(resource_type, id);
        let history_prefix = location.keyspace.history_versions_prefix(resource_type, id);

        let has_current = self
            .get_json_object::<StoredResource>(&location.bucket, &current_key)
            .await?
            .is_some();
        let history = self
            .list_objects_all(&location.bucket, &history_prefix)
            .await?;

        if !has_current && history.is_empty() {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        }

        if has_current {
            self.client
                .delete_object(&location.bucket, &current_key)
                .await
                .map_err(|e| self.map_client_error(e))?;
        }
        self.delete_prefix(&location.bucket, &history_prefix)
            .await?;
        self.purge_history_events(&location, resource_type, Some(id))
            .await?;

        Ok(())
    }

    async fn purge_all(&self, tenant: &TenantContext, resource_type: &str) -> StorageResult<u64> {
        let location = self.tenant_location(tenant)?;

        // Counted over `current.json` keys, so the figure is "resources purged",
        // not "objects purged".
        let count = self
            .list_current_keys(&location, Some(resource_type))
            .await?
            .len() as u64;

        // The type prefix covers both current.json and the nested _history
        // snapshots for every id of this type.
        let type_prefix = location.keyspace.resource_type_prefix(resource_type);
        self.delete_prefix(&location.bucket, &type_prefix).await?;
        self.purge_history_events(&location, resource_type, None)
            .await?;

        Ok(count)
    }
}

// ============================================================================
// ReindexSource
//
// S3 implements the read half only. It has no search index of any kind — no
// index namespace in S3Keyspace, and its SearchProvider returns
// UnsupportedCapability — so it is deliberately NOT a ReindexTarget. On an
// s3+elasticsearch deployment it is the source and Elasticsearch is the target;
// on s3-standalone there is no target at all and `$reindex` has nothing to do.
// ============================================================================

#[async_trait]
impl ReindexSource for S3Backend {
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        let location = self.tenant_location(tenant)?;

        let mut types = std::collections::BTreeSet::new();
        for key in self.list_current_keys(&location, None).await? {
            // `…/resources/<type>/<id>/current.json` — the segment after
            // `resources` is the type.
            let parts: Vec<&str> = key.split('/').collect();
            if let Some(pos) = parts.iter().position(|p| *p == "resources")
                && let Some(resource_type) = parts.get(pos + 1)
            {
                types.insert((*resource_type).to_string());
            }
        }

        Ok(types.into_iter().collect())
    }

    async fn count_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        ResourceStorage::count(self, tenant, Some(resource_type)).await
    }

    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<ResourcePage> {
        let location = self.tenant_location(tenant)?;

        // S3 has no server-side sort or keyset cursor, so the cursor is an
        // offset into the lexicographically sorted key list — the same approach
        // the bulk-export batcher takes. Sorting makes it stable across pages.
        let mut keys = self
            .list_current_keys(&location, Some(resource_type))
            .await?;
        keys.sort();

        let offset: usize = match cursor {
            None => 0,
            Some(raw) => raw.parse().map_err(|_| {
                StorageError::Backend(BackendError::QueryError {
                    message: format!("Invalid reindex cursor: {raw}"),
                })
            })?,
        };

        let start = offset.min(keys.len());
        let end = start.saturating_add(limit as usize).min(keys.len());

        let mut resources = Vec::new();
        for key in &keys[start..end] {
            let Some((resource, _)) = self
                .get_json_object::<StoredResource>(&location.bucket, key)
                .await?
            else {
                continue;
            };
            if resource.is_deleted() {
                continue;
            }
            resources.push(resource);
        }

        let next_cursor = (end < keys.len()).then(|| end.to_string());

        Ok(ResourcePage {
            resources,
            next_cursor,
        })
    }
}
