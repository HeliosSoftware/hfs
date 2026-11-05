//! Bulk export data provider for the S3 backend.
//!
//! The S3 backend is **output-only** for bulk export: it provides
//! [`ExportDataProvider`] (feeding export batches when S3 is the resource
//! store) but does not implement `BulkExportStorage` — job state lives in the
//! SQLite or Postgres job store, never S3.

use std::collections::{BTreeSet, HashSet};

use async_trait::async_trait;

use crate::core::ResourceStorage;
use crate::core::bulk_export::{
    ExportDataProvider, ExportRequest, GroupExportProvider, NdjsonBatch, PatientExportProvider,
};
use crate::core::patient_compartment::PatientCompartmentMatcher;
use crate::error::{BulkExportError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::StoredResource;

use super::backend::{ResourceLocation, S3Backend};

#[async_trait]
impl ExportDataProvider for S3Backend {
    async fn list_export_types(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
    ) -> StorageResult<Vec<String>> {
        let location = self.resource_location(tenant).await?;

        // A requested type is kept when it holds any live resource, as the
        // SQL and MongoDB backends decide it: the `_since` / `_until` window is
        // applied by the batches, and a type with nothing in it writes no
        // output. Counting here read every object of the type one at a time
        // before the export wrote anything (#2140).
        if !request.resource_types.is_empty() {
            let mut found = Vec::new();
            for resource_type in &request.resource_types {
                if self.has_live_current(&location, resource_type).await? {
                    found.push(resource_type.clone());
                }
            }
            return Ok(found);
        }

        let mut types = BTreeSet::new();
        for key in self.list_current_keys(&location, None).await? {
            if let Some(resource_type) = parse_resource_type_from_current_key(&key) {
                types.insert(resource_type);
            }
        }

        Ok(types.into_iter().collect())
    }

    async fn count_export_resources(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        resource_type: &str,
    ) -> StorageResult<u64> {
        let location = self.resource_location(tenant).await?;
        let keys = self
            .list_current_keys(&location, Some(resource_type))
            .await?;

        let mut count = 0u64;
        self.for_each_current(&location, keys, |resource| {
            if export_window_keeps(request, &resource) {
                count += 1;
            }
        })
        .await?;

        Ok(count)
    }

    async fn fetch_export_batch(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        resource_type: &str,
        cursor: Option<&str>,
        batch_size: u32,
    ) -> StorageResult<NdjsonBatch> {
        let location = self.resource_location(tenant).await?;
        self.export_page(&location, resource_type, cursor, batch_size, |resource| {
            export_window_keeps(request, resource)
        })
        .await
    }
}

/// Whether a stored resource is live and inside the request's `_since` /
/// `_until` window.
fn export_window_keeps(request: &ExportRequest, resource: &StoredResource) -> bool {
    if resource.is_deleted() {
        return false;
    }
    if let Some(since) = request.since
        && resource.last_modified() < since
    {
        return false;
    }
    if let Some(until) = request.until
        && resource.last_modified() > until
    {
        return false;
    }
    true
}

/// Prefix of a keyset export cursor: the object key of the last line already
/// handed out. A bare number is the legacy offset cursor (still accepted so an
/// export job started before an upgrade can finish).
const KEY_CURSOR_PREFIX: &str = "k:";

impl S3Backend {
    /// Whether `resource_type` holds at least one live (non-deleted)
    /// resource.
    ///
    /// Walks the type's listing a page at a time and reads its objects in
    /// rounds that start at one and double up to `scan_concurrency`, stopping
    /// at the first live one. A type whose first object is live costs one LIST
    /// and one GET; only a run of deleted objects at the start costs more.
    async fn has_live_current(
        &self,
        location: &ResourceLocation,
        resource_type: &str,
    ) -> StorageResult<bool> {
        let bucket = &location.bucket;
        let mut width = 1usize;
        let mut token: Option<String> = None;
        loop {
            let (mut keys, next) = self
                .list_current_keys_page(location, resource_type, None, token.as_deref())
                .await?;
            keys.sort();
            let mut rest = keys.as_slice();
            while !rest.is_empty() {
                let (round, tail) = rest.split_at(width.min(rest.len()));
                rest = tail;
                let fetched = futures::future::join_all(
                    round
                        .iter()
                        .map(|key| self.get_json_object_limited::<StoredResource>(bucket, key)),
                )
                .await;
                for object in fetched {
                    if let Some((resource, _)) = object?
                        && !resource.is_deleted()
                    {
                        return Ok(true);
                    }
                }
                width = (width * 2).min(self.scan_concurrency().max(1));
            }
            token = next;
            if token.is_none() {
                return Ok(false);
            }
        }
    }

    /// Reads the `current.json` objects at `keys` concurrently, under the
    /// backend-wide read limit (`scan_concurrency`, #2063), and hands each one
    /// that exists to `visit`, in completion order.
    async fn for_each_current(
        &self,
        location: &ResourceLocation,
        keys: Vec<String>,
        mut visit: impl FnMut(StoredResource),
    ) -> StorageResult<()> {
        use futures::stream::{self, StreamExt};

        let bucket = &location.bucket;
        let mut fetched = stream::iter(keys)
            .map(|key| async move {
                self.get_json_object_limited::<StoredResource>(bucket, &key)
                    .await
            })
            .buffer_unordered(self.scan_concurrency().max(1));
        while let Some(object) = fetched.next().await {
            if let Some((resource, _)) = object? {
                visit(resource);
            }
        }
        Ok(())
    }

    /// One export batch over the `current.json` objects of a type, in key
    /// order.
    ///
    /// The cursor is the key of the last line returned, so a batch lists the
    /// type from that key on (S3 `StartAfter`) and fetches only the objects
    /// it needs — `batch_size + 1` live ones (the extra one decides
    /// `is_last`) — one listing page at a time, objects concurrently.
    /// Listing and fetching the whole type on every batch made a full export
    /// O(n²/batch). A legacy numeric cursor still walks every key and slices
    /// by offset.
    async fn export_page(
        &self,
        location: &ResourceLocation,
        resource_type: &str,
        cursor: Option<&str>,
        batch_size: u32,
        keep: impl Fn(&StoredResource) -> bool,
    ) -> StorageResult<NdjsonBatch> {
        use futures::stream::{self, StreamExt};

        let (legacy_offset, after) = match cursor {
            None => (None, None),
            Some(raw) => match raw.strip_prefix(KEY_CURSOR_PREFIX) {
                Some(key) => (None, Some(key)),
                None => (Some(parse_export_cursor(Some(raw))?), None),
            },
        };
        let batch = (batch_size as usize).max(1);
        let limit = if legacy_offset.is_some() {
            usize::MAX
        } else {
            batch + 1
        };
        let bucket = &location.bucket;

        let mut lines: Vec<(String, String)> = Vec::new();
        let mut token: Option<String> = None;
        'pages: loop {
            let (mut keys, next) = self
                .list_current_keys_page(
                    location,
                    resource_type,
                    if token.is_none() { after } else { None },
                    token.as_deref(),
                )
                .await?;
            // A listing is already lexicographic; sorting only guards the
            // test doubles that list in insertion order.
            keys.sort();

            let mut fetched = stream::iter(keys)
                .map(|key| async move {
                    let object = self.get_json_object::<StoredResource>(bucket, &key).await;
                    (key, object)
                })
                .buffered(self.bulk_write_concurrency());
            while let Some((key, object)) = fetched.next().await {
                let Some((resource, _)) = object? else {
                    continue;
                };
                if !keep(&resource) {
                    continue;
                }
                let line = serde_json::to_string(&resource.content_with_meta()).map_err(|e| {
                    StorageError::BulkExport(BulkExportError::WriteError {
                        message: format!("failed to serialize NDJSON line: {e}"),
                    })
                })?;
                lines.push((key, line));
                if lines.len() >= limit {
                    break 'pages;
                }
            }
            token = next;
            if token.is_none() {
                break;
            }
        }

        if let Some(offset) = legacy_offset {
            let start = offset.min(lines.len());
            let end = start.saturating_add(batch).min(lines.len());
            let is_last = end >= lines.len();
            return Ok(NdjsonBatch {
                lines: lines[start..end].iter().map(|(_, l)| l.clone()).collect(),
                next_cursor: if is_last { None } else { Some(end.to_string()) },
                is_last,
            });
        }

        let is_last = lines.len() <= batch;
        lines.truncate(batch);
        let next_cursor = if is_last {
            None
        } else {
            lines
                .last()
                .map(|(key, _)| format!("{KEY_CURSOR_PREFIX}{key}"))
        };
        Ok(NdjsonBatch {
            lines: lines.into_iter().map(|(_, line)| line).collect(),
            next_cursor,
            is_last,
        })
    }
}

/// Parses the numeric offset encoded in an export batch cursor.
///
/// A `None` cursor is treated as offset `0` (start of the result set).
fn parse_export_cursor(cursor: Option<&str>) -> StorageResult<usize> {
    match cursor {
        None => Ok(0),
        Some(raw) => raw.parse::<usize>().map_err(|_| {
            StorageError::BulkExport(BulkExportError::InvalidRequest {
                message: format!("invalid export cursor: {raw}"),
            })
        }),
    }
}

/// Extracts the resource type from a `current.json` object key.
///
/// Keys follow the pattern `…/resources/<type>/<id>/current.json` (`V1`) or
/// `…/resources/<type>/<h2>/<id>/current.json` (`V2`); in both the segment
/// immediately after `resources` is the resource type.
fn parse_resource_type_from_current_key(key: &str) -> Option<String> {
    let parts: Vec<&str> = key.split('/').collect();
    let resources_idx = parts.iter().position(|segment| *segment == "resources")?;
    parts.get(resources_idx + 1).map(|s| s.to_string())
}

#[async_trait]
impl PatientExportProvider for S3Backend {
    async fn list_patient_ids(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        cursor: Option<&str>,
        batch_size: u32,
    ) -> StorageResult<(Vec<String>, Option<String>)> {
        let location = self.resource_location(tenant).await?;
        let keys = self.list_current_keys(&location, Some("Patient")).await?;

        let mut ids: Vec<String> = Vec::new();
        self.for_each_current(&location, keys, |resource| {
            if resource.is_deleted() {
                return;
            }
            if let Some(since) = request.since
                && resource.last_modified() < since
            {
                return;
            }
            ids.push(resource.id().to_string());
        })
        .await?;
        ids.sort();

        let start = match cursor {
            None => 0,
            Some(c) => ids.iter().position(|i| i.as_str() > c).unwrap_or(ids.len()),
        };
        let end = start.saturating_add(batch_size as usize).min(ids.len());
        let page = ids[start..end].to_vec();
        let next = if end < ids.len() {
            page.last().cloned()
        } else {
            None
        };
        Ok((page, next))
    }

    async fn fetch_patient_compartment_batch(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        resource_type: &str,
        patient_ids: &[String],
        cursor: Option<&str>,
        batch_size: u32,
    ) -> StorageResult<NdjsonBatch> {
        if patient_ids.is_empty() {
            return Ok(NdjsonBatch::empty());
        }
        let location = self.resource_location(tenant).await?;
        let matcher = PatientCompartmentMatcher::new(
            self.registries.for_tenant(tenant.tenant_id().as_str()),
            request.fhir_version,
            resource_type,
        );
        let is_patient = resource_type == "Patient";
        if !is_patient && matcher.is_empty() {
            return Ok(NdjsonBatch::empty());
        }
        let patient_id_set: HashSet<String> = patient_ids.iter().cloned().collect();
        let patient_ref_set: HashSet<String> = patient_ids
            .iter()
            .map(|id| format!("Patient/{id}"))
            .collect();

        self.export_page(&location, resource_type, cursor, batch_size, |resource| {
            export_window_keeps(request, resource)
                && ((is_patient && patient_id_set.contains(resource.id()))
                    || matcher.is_member(resource.content(), &patient_ref_set))
        })
        .await
    }
}

#[async_trait]
impl GroupExportProvider for S3Backend {
    async fn get_group_members(
        &self,
        tenant: &TenantContext,
        group_id: &str,
    ) -> StorageResult<Vec<String>> {
        let location = self.resource_location(tenant).await?;
        let key = location.keyspace.current_resource_key("Group", group_id);
        let group = self
            .get_json_object::<StoredResource>(&location.bucket, &key)
            .await?
            .map(|(r, _)| r);
        let group = match group {
            Some(g) if !g.is_deleted() => g,
            _ => {
                return Err(StorageError::BulkExport(BulkExportError::GroupNotFound {
                    group_id: group_id.to_string(),
                }));
            }
        };
        let mut refs = Vec::new();
        if let Some(members) = group.content().get("member").and_then(|m| m.as_array()) {
            for member in members {
                if let Some(reference) = member
                    .get("entity")
                    .and_then(|e| e.get("reference"))
                    .and_then(|r| r.as_str())
                {
                    refs.push(reference.to_string());
                }
            }
        }
        Ok(refs)
    }

    async fn resolve_group_patient_ids(
        &self,
        tenant: &TenantContext,
        group_id: &str,
    ) -> StorageResult<Vec<String>> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut patient_ids: Vec<String> = Vec::new();
        let mut worklist: Vec<String> = vec![group_id.to_string()];
        while let Some(gid) = worklist.pop() {
            if !visited.insert(gid.clone()) {
                continue;
            }
            let members = self.get_group_members(tenant, &gid).await?;
            for r in members {
                if let Some(pid) = r.strip_prefix("Patient/") {
                    if seen.insert(pid.to_string()) {
                        patient_ids.push(pid.to_string());
                    }
                } else if let Some(nested) = r.strip_prefix("Group/") {
                    worklist.push(nested.to_string());
                }
            }
        }
        Ok(patient_ids)
    }

    async fn get_group_members_with_periods(
        &self,
        tenant: &TenantContext,
        group_id: &str,
    ) -> StorageResult<Vec<(String, Option<chrono::DateTime<chrono::Utc>>)>> {
        let location = self.resource_location(tenant).await?;
        let key = location.keyspace.current_resource_key("Group", group_id);
        let group = self
            .get_json_object::<StoredResource>(&location.bucket, &key)
            .await?
            .map(|(r, _)| r);
        let group = match group {
            Some(g) if !g.is_deleted() => g,
            _ => {
                return Err(StorageError::BulkExport(BulkExportError::GroupNotFound {
                    group_id: group_id.to_string(),
                }));
            }
        };
        let mut out = Vec::new();
        if let Some(arr) = group.content().get("member").and_then(|m| m.as_array()) {
            for member in arr {
                let Some(reference) = member
                    .get("entity")
                    .and_then(|e| e.get("reference"))
                    .and_then(|r| r.as_str())
                else {
                    continue;
                };
                let period_start = member
                    .get("period")
                    .and_then(|p| p.get("start"))
                    .and_then(|s| s.as_str())
                    .and_then(|s| {
                        chrono::DateTime::parse_from_rfc3339(s)
                            .ok()
                            .map(|dt| dt.with_timezone(&chrono::Utc))
                    });
                out.push((reference.to_string(), period_start));
            }
        }
        Ok(out)
    }
}
