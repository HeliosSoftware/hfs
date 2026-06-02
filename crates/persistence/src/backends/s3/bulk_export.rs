//! Bulk export data provider for the S3 backend.
//!
//! The S3 backend is **output-only** for bulk export: it provides
//! [`ExportDataProvider`] (feeding export batches when S3 is the resource
//! store) but does not implement `BulkExportStorage` — job state lives in the
//! SQLite or Postgres job store, never S3.

use std::collections::{BTreeSet, HashSet};

use async_trait::async_trait;
use serde_json::Value;

use crate::core::bulk_export::{
    ExportDataProvider, ExportRequest, GroupExportProvider, NdjsonBatch, PatientExportProvider,
};
use crate::error::{BulkExportError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::StoredResource;

use super::backend::S3Backend;

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

/// Returns true if `value` (a FHIR resource JSON) has `subject.reference` or
/// `patient.reference` matching one of the supplied `Patient/<id>` references.
fn resource_in_patient_compartment(value: &Value, patient_refs: &HashSet<String>) -> bool {
    let check = |key: &str| -> bool {
        value
            .get(key)
            .and_then(|v| v.get("reference"))
            .and_then(|r| r.as_str())
            .is_some_and(|r| patient_refs.contains(r))
    };
    check("subject") || check("patient")
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
        let location = self.tenant_location(tenant)?;
        let keys = self.list_current_keys(&location, Some("Patient")).await?;

        let mut ids: Vec<String> = Vec::new();
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
            if let Some(since) = request.since
                && resource.last_modified() < since
            {
                continue;
            }
            ids.push(resource.id().to_string());
        }
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
        let location = self.tenant_location(tenant)?;
        let patient_id_set: HashSet<String> = patient_ids.iter().cloned().collect();
        let patient_ref_set: HashSet<String> = patient_ids
            .iter()
            .map(|id| format!("Patient/{id}"))
            .collect();

        let mut keys = self
            .list_current_keys(&location, Some(resource_type))
            .await?;
        keys.sort();

        let mut lines: Vec<String> = Vec::new();
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
            if let Some(since) = request.since
                && resource.last_modified() < since
            {
                continue;
            }
            if let Some(until) = request.until
                && resource.last_modified() > until
            {
                continue;
            }
            let in_compartment = if resource_type == "Patient" {
                patient_id_set.contains(resource.id())
            } else {
                resource_in_patient_compartment(resource.content(), &patient_ref_set)
            };
            if !in_compartment {
                continue;
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

#[async_trait]
impl GroupExportProvider for S3Backend {
    async fn get_group_members(
        &self,
        tenant: &TenantContext,
        group_id: &str,
    ) -> StorageResult<Vec<String>> {
        let location = self.tenant_location(tenant)?;
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
        let location = self.tenant_location(tenant)?;
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
