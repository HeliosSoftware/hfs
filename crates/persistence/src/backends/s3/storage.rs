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
use crate::core::{ResourceStorage, VersionedStorage, normalize_etag};
use crate::error::{
    BackendError, ConcurrencyError, ResourceError, SearchError, StorageError, StorageResult,
};
use crate::tenant::TenantContext;
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

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
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
        let location = self.tenant_location(tenant)?;
        let resource_type = current.resource_type();
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

        let expected = normalize_etag(expected_version);
        let actual_version = actual.resource.version_id();
        if expected != actual_version {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: expected.to_string(),
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

        let expected = normalize_etag(expected_version);
        let actual_version = actual.resource.version_id();
        if expected != actual_version {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: expected.to_string(),
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

use crate::core::search::{SearchProvider, SearchResult};
use crate::core::storage::{
    ConditionalCreateResult, ConditionalDeleteResult, ConditionalStorage, ConditionalUpdateResult,
};
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
    ) -> &std::sync::Arc<parking_lot::RwLock<crate::search::SearchParameterRegistry>> {
        // S3 standalone does not implement search; an empty registry is
        // required only to satisfy the trait. In real deployments S3 is
        // composed with a search backend (e.g., Elasticsearch) and the
        // composite forwards to that backend's registry.
        use std::sync::OnceLock;
        static EMPTY: OnceLock<
            std::sync::Arc<parking_lot::RwLock<crate::search::SearchParameterRegistry>>,
        > = OnceLock::new();
        EMPTY.get_or_init(|| {
            std::sync::Arc::new(parking_lot::RwLock::new(
                crate::search::SearchParameterRegistry::new(),
            ))
        })
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
