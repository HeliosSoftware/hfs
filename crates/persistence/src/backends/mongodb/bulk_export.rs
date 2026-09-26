//! Bulk export implementation for the MongoDB backend.
//!
//! Implements [`ExportDataProvider`], [`PatientExportProvider`], and
//! [`GroupExportProvider`] over the `resources` collection. Job-state storage
//! is not provided by MongoDB; deployments using MongoDB as the primary FHIR
//! backend pair it with an embedded SQLite sidecar for the
//! [`BulkExportJobStore`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mongodb::{
    Collection,
    bson::{self, Bson, DateTime as BsonDateTime, Document, doc},
    options::FindOptions,
};
use serde_json::Value;
use std::collections::HashSet;

use crate::core::bulk_export::{
    ExportDataProvider, ExportRequest, GroupExportProvider, NdjsonBatch, PatientExportProvider,
};
use crate::core::patient_compartment::PatientCompartmentMatcher;
use crate::error::{BackendError, BulkExportError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::StoredResource;

use super::MongoBackend;

fn internal_error(msg: impl Into<String>) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "mongodb".to_string(),
        message: msg.into(),
        source: None,
    })
}

fn chrono_to_bson(dt: DateTime<Utc>) -> BsonDateTime {
    BsonDateTime::from_millis(dt.timestamp_millis())
}

/// The `last_updated` range an export request asks for, or `None` when it is
/// unbounded.
///
/// Both bounds are inclusive, matching the S3 backend's
/// `last_modified() < since` / `> until` skips.
///
/// The two bounds must share one document: `last_updated` is a single key, so a
/// second `filter.insert("last_updated", ...)` would REPLACE the first rather
/// than narrow it, silently dropping whichever bound was written first.
fn last_updated_range(request: &ExportRequest) -> Option<Document> {
    let mut range = Document::new();
    if let Some(since) = request.since {
        range.insert("$gte", chrono_to_bson(since));
    }
    if let Some(until) = request.until {
        range.insert("$lte", chrono_to_bson(until));
    }
    (!range.is_empty()).then_some(range)
}

fn bson_to_chrono(dt: &BsonDateTime) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(dt.timestamp_millis()).unwrap_or_else(Utc::now)
}

fn document_to_value(doc: &Document) -> StorageResult<Value> {
    bson::from_bson::<Value>(Bson::Document(doc.clone()))
        .map_err(|e| internal_error(format!("deserialize resource: {e}")))
}

/// Parses a `{rfc3339}|{id}` keyset-pagination cursor.
fn parse_cursor(cursor: &str) -> Option<(DateTime<Utc>, String)> {
    let (ts, id) = cursor.split_once('|')?;
    let dt = DateTime::parse_from_rfc3339(ts).ok()?.with_timezone(&Utc);
    Some((dt, id.to_string()))
}

fn make_cursor(last_updated: &BsonDateTime, id: &str) -> String {
    format!("{}|{}", bson_to_chrono(last_updated).to_rfc3339(), id)
}

async fn collect_cursor(mut cursor: mongodb::Cursor<Document>) -> StorageResult<Vec<Document>> {
    let mut docs = Vec::new();
    while cursor
        .advance()
        .await
        .map_err(|e| internal_error(format!("cursor advance: {e}")))?
    {
        docs.push(
            cursor
                .deserialize_current()
                .map_err(|e| internal_error(format!("cursor deserialize: {e}")))?,
        );
    }
    Ok(docs)
}

fn ndjson_from_docs(docs: Vec<Document>, batch_size: u32) -> StorageResult<NdjsonBatch> {
    ndjson_from_docs_where(docs, batch_size, |_, _| true)
}

/// Pages `docs` like [`ndjson_from_docs`], emitting only the payloads
/// `keep(id, payload)` accepts. The cursor advances over every document of
/// the page, kept or not, so a page whose candidates all fail the test still
/// moves the export forward (an empty, non-final batch is legal).
fn ndjson_from_docs_where(
    docs: Vec<Document>,
    batch_size: u32,
    keep: impl Fn(&str, &Value) -> bool,
) -> StorageResult<NdjsonBatch> {
    let has_more = docs.len() > batch_size as usize;
    let slice = if has_more {
        &docs[..batch_size as usize]
    } else {
        &docs[..]
    };
    let mut lines = Vec::with_capacity(slice.len());
    let mut last_cursor = None;
    for d in slice {
        let last_updated = d
            .get_datetime("last_updated")
            .map_err(|e| internal_error(format!("missing last_updated: {e}")))?;
        let version_id = d
            .get_str("version_id")
            .map_err(|e| internal_error(format!("missing version_id: {e}")))?;
        let id = d
            .get_str("id")
            .map_err(|e| internal_error(format!("missing id: {e}")))?;
        last_cursor = Some(make_cursor(last_updated, id));
        let data = d
            .get_document("data")
            .map_err(|e| internal_error(format!("missing data payload: {e}")))?;
        let val = document_to_value(data)?;
        if !keep(id, &val) {
            continue;
        }
        // The payload is stored as submitted; versionId/lastUpdated live in their
        // own fields and must be merged back in (#1273).
        let val = StoredResource::merge_meta(val, version_id, bson_to_chrono(last_updated));
        let line =
            serde_json::to_string(&val).map_err(|e| internal_error(format!("serialize: {e}")))?;
        lines.push(line);
    }
    Ok(NdjsonBatch {
        lines,
        next_cursor: if has_more { last_cursor } else { None },
        is_last: !has_more,
    })
}

/// The `$in` candidates matching a stored reference to one of `patient_refs`:
/// the exact `Patient/{id}` and, as an anchored regex, its versioned form
/// `Patient/{id}/_history/{vid}`.
fn reference_candidates(patient_refs: &HashSet<String>) -> Vec<Bson> {
    let mut refs: Vec<&String> = patient_refs.iter().collect();
    refs.sort();
    let mut candidates = Vec::with_capacity(refs.len() * 2);
    for reference in refs {
        candidates.push(Bson::String(reference.clone()));
        candidates.push(Bson::RegularExpression(bson::Regex {
            pattern: format!("^{}/_history/", regex_literal(reference)),
            options: String::new(),
        }));
    }
    candidates
}

/// Escapes `s` so a regex matches it literally.
fn regex_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\.^$|?*+()[]{}/".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[async_trait]
impl ExportDataProvider for MongoBackend {
    async fn list_export_types(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
    ) -> StorageResult<Vec<String>> {
        let db = self.get_database().await?;
        let resources: Collection<Document> = db.collection(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        if !request.resource_types.is_empty() {
            let mut valid = Vec::new();
            for rt in &request.resource_types {
                let exists = resources
                    .find_one(doc! {
                        "tenant_id": tenant_id,
                        "resource_type": rt.as_str(),
                        "is_deleted": false,
                    })
                    .await
                    .map_err(|e| internal_error(format!("validate type: {e}")))?
                    .is_some();
                if exists {
                    valid.push(rt.clone());
                }
            }
            return Ok(valid);
        }

        let types = resources
            .distinct(
                "resource_type",
                doc! { "tenant_id": tenant_id, "is_deleted": false },
            )
            .await
            .map_err(|e| internal_error(format!("distinct types: {e}")))?;
        let mut out: Vec<String> = types
            .into_iter()
            .filter_map(|b| match b {
                Bson::String(s) => Some(s),
                _ => None,
            })
            .collect();
        out.sort();
        Ok(out)
    }

    async fn count_export_resources(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        resource_type: &str,
    ) -> StorageResult<u64> {
        let db = self.get_database().await?;
        let resources: Collection<Document> = db.collection(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let mut filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "is_deleted": false,
        };
        if let Some(range) = last_updated_range(request) {
            filter.insert("last_updated", range);
        }

        resources
            .count_documents(filter)
            .await
            .map_err(|e| internal_error(format!("count: {e}")))
    }

    async fn fetch_export_batch(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        resource_type: &str,
        cursor: Option<&str>,
        batch_size: u32,
    ) -> StorageResult<NdjsonBatch> {
        let db = self.get_database().await?;
        let resources: Collection<Document> = db.collection(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let mut filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "is_deleted": false,
        };
        if let Some(range) = last_updated_range(request) {
            filter.insert("last_updated", range);
        }
        if let Some((cur_dt, cur_id)) = cursor.and_then(parse_cursor) {
            filter.insert(
                "$or",
                vec![
                    doc! { "last_updated": { "$gt": chrono_to_bson(cur_dt) } },
                    doc! {
                        "last_updated": chrono_to_bson(cur_dt),
                        "id": { "$gt": cur_id },
                    },
                ],
            );
        }

        let opts = FindOptions::builder()
            .sort(doc! { "last_updated": 1, "id": 1 })
            .limit((batch_size as i64) + 1)
            .build();
        let cursor_stream = resources
            .find(filter)
            .with_options(opts)
            .await
            .map_err(|e| internal_error(format!("find: {e}")))?;
        let docs = collect_cursor(cursor_stream).await?;
        ndjson_from_docs(docs, batch_size)
    }
}

#[async_trait]
impl PatientExportProvider for MongoBackend {
    async fn list_patient_ids(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        cursor: Option<&str>,
        batch_size: u32,
    ) -> StorageResult<(Vec<String>, Option<String>)> {
        let db = self.get_database().await?;
        let resources: Collection<Document> = db.collection(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let mut filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": "Patient",
            "is_deleted": false,
        };
        // `_since` only, deliberately: this selects WHICH patients are in scope,
        // not which of their resources are exported. Bounding it above by
        // `_until` would drop a patient whose own record was touched after the
        // window and take their in-window compartment resources with them. The
        // Patient resource itself is still bounded, by the compartment fetch.
        // S3 makes the same distinction (`backends/s3/bulk_export.rs:216`).
        if let Some(since) = request.since {
            filter.insert("last_updated", doc! { "$gte": chrono_to_bson(since) });
        }
        if let Some(c) = cursor {
            filter.insert("id", doc! { "$gt": c });
        }

        let opts = FindOptions::builder()
            .sort(doc! { "id": 1 })
            .limit((batch_size as i64) + 1)
            .projection(doc! { "id": 1 })
            .build();
        let cursor_stream = resources
            .find(filter)
            .with_options(opts)
            .await
            .map_err(|e| internal_error(format!("list patients: {e}")))?;
        let docs = collect_cursor(cursor_stream).await?;
        let mut ids: Vec<String> = docs
            .iter()
            .filter_map(|d| d.get_str("id").ok().map(|s| s.to_string()))
            .collect();

        let has_more = ids.len() > batch_size as usize;
        if has_more {
            ids.truncate(batch_size as usize);
        }
        let next = if has_more { ids.last().cloned() } else { None };
        Ok((ids, next))
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
        let tenant_id = tenant.tenant_id().as_str();
        let matcher = PatientCompartmentMatcher::new(
            self.tenant_registry(tenant_id),
            request.fhir_version,
            resource_type,
        );
        let is_patient = resource_type == "Patient";
        if !is_patient && matcher.is_empty() {
            return Ok(NdjsonBatch::empty());
        }
        let patient_refs: HashSet<String> = patient_ids
            .iter()
            .map(|id| format!("Patient/{id}"))
            .collect();

        // Membership is decided on the resource payload, not the search_index,
        // so this holds when search is offloaded (mongodb-elasticsearch leaves
        // the local index empty). The query narrows to documents whose payload
        // carries one of the patients under an element path the compartment
        // parameters read (`payload_paths`), or that are the patients
        // themselves; `matcher` then applies the parameters exactly to each
        // candidate. A parameter whose expression yields no element path
        // leaves the type without a prefilter: every document is a candidate.
        let db = self.get_database().await?;
        let resources: Collection<Document> = db.collection(MongoBackend::RESOURCES_COLLECTION);
        let mut filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "is_deleted": false,
        };
        if let Some(range) = last_updated_range(request) {
            filter.insert("last_updated", range);
        }

        let mut clauses: Vec<Document> = Vec::new();
        let by_id = doc! { "id": { "$in": patient_ids.to_vec() } };
        if matcher.is_empty() {
            clauses.push(by_id);
        } else if let Some(paths) = matcher.payload_paths() {
            let candidates = reference_candidates(&patient_refs);
            let mut arms: Vec<Document> = paths
                .iter()
                .map(|path| {
                    let mut arm = Document::new();
                    arm.insert(
                        format!("data.{path}.reference"),
                        doc! { "$in": candidates.clone() },
                    );
                    arm
                })
                .collect();
            if is_patient {
                arms.push(by_id);
            }
            clauses.push(doc! { "$or": arms });
        }
        if let Some((cur_dt, cur_id)) = cursor.and_then(parse_cursor) {
            clauses.push(doc! { "$or": vec![
                doc! { "last_updated": { "$gt": chrono_to_bson(cur_dt) } },
                doc! {
                    "last_updated": chrono_to_bson(cur_dt),
                    "id": { "$gt": cur_id },
                },
            ]});
        }
        if !clauses.is_empty() {
            filter.insert("$and", clauses);
        }

        let opts = FindOptions::builder()
            .sort(doc! { "last_updated": 1, "id": 1 })
            .limit((batch_size as i64) + 1)
            .build();
        let cursor_stream = resources
            .find(filter)
            .with_options(opts)
            .await
            .map_err(|e| internal_error(format!("compartment fetch: {e}")))?;
        let docs = collect_cursor(cursor_stream).await?;
        ndjson_from_docs_where(docs, batch_size, |id, value| {
            (is_patient && patient_ids.iter().any(|p| p == id))
                || matcher.is_member(value, &patient_refs)
        })
    }
}

#[async_trait]
impl GroupExportProvider for MongoBackend {
    async fn get_group_members(
        &self,
        tenant: &TenantContext,
        group_id: &str,
    ) -> StorageResult<Vec<String>> {
        let db = self.get_database().await?;
        let resources: Collection<Document> = db.collection(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let doc_opt = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": "Group",
                "id": group_id,
                "is_deleted": false,
            })
            .await
            .map_err(|e| internal_error(format!("get group: {e}")))?;
        let d = doc_opt.ok_or_else(|| {
            StorageError::BulkExport(BulkExportError::GroupNotFound {
                group_id: group_id.to_string(),
            })
        })?;
        let data = d
            .get_document("data")
            .map_err(|e| internal_error(format!("missing data payload: {e}")))?;
        let group: Value = document_to_value(data)?;
        let mut refs = Vec::new();
        if let Some(members) = group.get("member").and_then(|m| m.as_array()) {
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
        use std::collections::HashSet;
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
    ) -> StorageResult<Vec<(String, Option<DateTime<Utc>>)>> {
        let db = self.get_database().await?;
        let resources: Collection<Document> = db.collection(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let doc_opt = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": "Group",
                "id": group_id,
                "is_deleted": false,
            })
            .await
            .map_err(|e| internal_error(format!("get group: {e}")))?;
        let d = doc_opt.ok_or_else(|| {
            StorageError::BulkExport(BulkExportError::GroupNotFound {
                group_id: group_id.to_string(),
            })
        })?;
        let data = d
            .get_document("data")
            .map_err(|e| internal_error(format!("missing data payload: {e}")))?;
        let group: Value = document_to_value(data)?;
        let mut out = Vec::new();
        if let Some(arr) = group.get("member").and_then(|m| m.as_array()) {
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
                        DateTime::parse_from_rfc3339(s)
                            .ok()
                            .map(|dt| dt.with_timezone(&Utc))
                    });
                out.push((reference.to_string(), period_start));
            }
        }
        Ok(out)
    }
}
