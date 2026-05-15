//! Bulk export data provider for the S3 backend.
//!
//! The S3 backend is **output-only** for bulk export: it provides
//! [`ExportDataProvider`] (feeding export batches when S3 is the resource
//! store) but does not implement `BulkExportStorage` — job state lives in the
//! SQLite or Postgres job store, never S3.

use std::collections::BTreeSet;

use async_trait::async_trait;

use crate::core::bulk_export::{
    ExportDataProvider, ExportRequest, GroupExportProvider, NdjsonBatch, PatientExportProvider,
};
use crate::error::{BackendError, BulkExportError, StorageError, StorageResult};
use crate::tenant::TenantContext;

use super::backend::S3Backend;

/// Error for export-level operations S3 does not support as a primary.
fn s3_export_unsupported() -> StorageError {
    StorageError::Backend(BackendError::UnsupportedCapability {
        backend_name: "s3".to_string(),
        capability: "patient/group bulk export".to_string(),
    })
}

#[async_trait]
impl ExportDataProvider for S3Backend {
    async fn list_export_types(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
    ) -> StorageResult<Vec<String>> {
        let location = self.tenant_location(tenant)?;

        if !request.resource_types.is_empty() {
            let mut found = Vec::new();
            for resource_type in &request.resource_types {
                let count = self
                    .count_export_resources(tenant, request, resource_type)
                    .await?;
                if count > 0 {
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
        let location = self.tenant_location(tenant)?;
        let keys = self
            .list_current_keys(&location, Some(resource_type))
            .await?;

        let mut count = 0u64;
        for key in keys {
            let Some((resource, _)) = self
                .get_json_object::<crate::types::StoredResource>(&location.bucket, &key)
                .await?
            else {
                continue;
            };

            if resource.is_deleted() {
                continue;
            }

            if let Some(since) = request.since {
                if resource.last_modified() < since {
                    continue;
                }
            }
            if let Some(until) = request.until {
                if resource.last_modified() > until {
                    continue;
                }
            }

            count += 1;
        }

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
        let location = self.tenant_location(tenant)?;
        let mut keys = self
            .list_current_keys(&location, Some(resource_type))
            .await?;
        keys.sort();

        let mut lines = Vec::new();
        for key in keys {
            let Some((resource, _)) = self
                .get_json_object::<crate::types::StoredResource>(&location.bucket, &key)
                .await?
            else {
                continue;
            };

            if resource.is_deleted() {
                continue;
            }

            if let Some(since) = request.since {
                if resource.last_modified() < since {
                    continue;
                }
            }
            if let Some(until) = request.until {
                if resource.last_modified() > until {
                    continue;
                }
            }

            lines.push(serde_json::to_string(resource.content()).map_err(|e| {
                StorageError::BulkExport(BulkExportError::WriteError {
                    message: format!("failed to serialize NDJSON line: {e}"),
                })
            })?);
        }

        let offset = parse_export_cursor(cursor)?;
        let start = offset.min(lines.len());
        let end = start.saturating_add(batch_size as usize).min(lines.len());

        let batch_lines = lines[start..end].to_vec();
        let is_last = end >= lines.len();
        let next_cursor = if is_last { None } else { Some(end.to_string()) };

        Ok(NdjsonBatch {
            lines: batch_lines,
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
/// Keys follow the pattern `…/resources/<type>/<id>/current.json`; the
/// segment immediately after `resources` is the resource type.
fn parse_resource_type_from_current_key(key: &str) -> Option<String> {
    let parts: Vec<&str> = key.split('/').collect();
    let resources_idx = parts.iter().position(|segment| *segment == "resources")?;
    parts.get(resources_idx + 1).map(|s| s.to_string())
}

// S3 is output-only for bulk export; patient/group compartment enumeration is
// not supported when S3 is the resource store. These stub impls satisfy the
// trait hierarchy so S3 can be a primary backend.

#[async_trait]
impl PatientExportProvider for S3Backend {
    async fn list_patient_ids(
        &self,
        _tenant: &TenantContext,
        _request: &ExportRequest,
        _cursor: Option<&str>,
        _batch_size: u32,
    ) -> StorageResult<(Vec<String>, Option<String>)> {
        Err(s3_export_unsupported())
    }

    async fn fetch_patient_compartment_batch(
        &self,
        _tenant: &TenantContext,
        _request: &ExportRequest,
        _resource_type: &str,
        _patient_ids: &[String],
        _cursor: Option<&str>,
        _batch_size: u32,
    ) -> StorageResult<NdjsonBatch> {
        Err(s3_export_unsupported())
    }
}

#[async_trait]
impl GroupExportProvider for S3Backend {
    async fn get_group_members(
        &self,
        _tenant: &TenantContext,
        _group_id: &str,
    ) -> StorageResult<Vec<String>> {
        Err(s3_export_unsupported())
    }

    async fn resolve_group_patient_ids(
        &self,
        _tenant: &TenantContext,
        _group_id: &str,
    ) -> StorageResult<Vec<String>> {
        Err(s3_export_unsupported())
    }
}
