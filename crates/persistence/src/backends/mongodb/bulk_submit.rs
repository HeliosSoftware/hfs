//! Bulk Data Submit implementation for the MongoDB backend.
//!
//! MongoDB hosts the **whole** `$bulk-submit` surface natively: the synchronous
//! ingestion engine ([`BulkSubmitProvider`], [`StreamingBulkSubmitProvider`],
//! [`BulkSubmitRollbackProvider`]) *and* the REST worker's job store
//! ([`SubmitClaimStrategy`], [`SubmitWorkerStorage`]) — unlike bulk *export*,
//! whose job state rides an embedded SQLite sidecar.
//!
//! The difference is that a submit job store is not merely a queue: the ingestion
//! engine and the job state mutate the same submission/manifest records, so
//! splitting them across two stores would put one submission's truth in two
//! places. `findOneAndUpdate` is an atomic compare-and-swap, which is exactly
//! what claiming a manifest under a fencing token needs, so there is nothing the
//! sidecar would buy.
//!
//! # Collections
//!
//! | Collection | Holds |
//! |---|---|
//! | `bulk_submissions` | one document per submission: status, kickoff metadata, poll token |
//! | `bulk_manifests` | one per manifest: status, worker lease + fencing token, fetch parameters |
//! | `bulk_entry_results` | one per ingested NDJSON line, keyed by `(manifest, file_url, line)` |
//! | `bulk_submission_changes` | the rollback change log |
//! | `bulk_submit_files` | finalized status-manifest artifacts (`output` / `error` / `deleted`) |
//!
//! Values that are arbitrary JSON (resource payloads, `OperationOutcome`s,
//! `fileEncryptionKey`, `countSeverity`) are stored as JSON **strings**, not as
//! sub-documents: BSON forbids `.` and `$` in keys, and FHIR extension URLs and
//! JWE headers routinely contain both.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use mongodb::{
    Collection,
    bson::{Bson, DateTime as BsonDateTime, Document, doc},
    options::{FindOptions, ReturnDocument},
};
use serde_json::Value;
use std::time::Duration as StdDuration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
use uuid::Uuid;

use crate::core::ResourceStorage;
use crate::core::bulk_export::ExportJobId;
use crate::core::bulk_export_worker::{LeaseError, WorkerId};
use crate::core::bulk_submit::{
    BulkEntryOutcome, BulkEntryResult, BulkProcessingOptions, BulkSubmitProvider,
    BulkSubmitRollbackProvider, ChangeType, EntryCountSummary, ManifestStatus, NdjsonEntry,
    StreamProcessingResult, StreamingBulkSubmitProvider, SubmissionChange, SubmissionId,
    SubmissionManifest, SubmissionStatus, SubmissionSummary,
};
use crate::core::bulk_submit_worker::{
    ManifestFetchParams, ManifestLease, ManifestWorkerView, PollTokenTarget, SubmitClaimStrategy,
    SubmitFileRecord, SubmitFileRow, SubmitWorkerStorage,
};
use crate::error::{BackendError, BulkSubmitError, ResourceError, StorageError, StorageResult};
use crate::tenant::{TenantContext, TenantId, TenantPermissions};

use super::MongoBackend;

/// Submissions: one document per `(tenant, submitter, submission_id)`.
pub(crate) const SUBMISSIONS_COLLECTION: &str = "bulk_submissions";
/// Manifests within a submission, carrying the worker lease.
pub(crate) const MANIFESTS_COLLECTION: &str = "bulk_manifests";
/// Per-NDJSON-line ingestion results.
pub(crate) const ENTRY_RESULTS_COLLECTION: &str = "bulk_entry_results";
/// Rollback change log.
pub(crate) const CHANGES_COLLECTION: &str = "bulk_submission_changes";
/// Finalized status-manifest artifacts.
pub(crate) const SUBMIT_FILES_COLLECTION: &str = "bulk_submit_files";

/// How many eligible manifests a single claim attempt will race for before
/// giving up and reporting "nothing available".
///
/// A claim is a compare-and-swap that can lose to another worker; walking a few
/// candidates keeps a busy queue from starving a worker that keeps losing the
/// head of the queue.
const CLAIM_CANDIDATES: i64 = 16;

fn internal_error(message: impl Into<String>) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "mongodb".to_string(),
        message: message.into(),
        source: None,
    })
}

fn to_bson_time(dt: DateTime<Utc>) -> BsonDateTime {
    BsonDateTime::from_millis(dt.timestamp_millis())
}

fn from_bson_time(dt: &BsonDateTime) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(dt.timestamp_millis()).unwrap_or_else(Utc::now)
}

/// `Utc::now()` truncated to the millisecond precision BSON stores.
///
/// A timestamp that is *returned* to a caller as well as persisted must be
/// rounded first, or the value the caller sees differs from every later read of
/// the same field — which for `transaction_time` would make a status manifest's
/// `transactionTime` change between the first poll and the next.
fn now_at_bson_precision() -> DateTime<Utc> {
    from_bson_time(&to_bson_time(Utc::now()))
}

/// Reads an optional `DateTime` field, tolerating an absent or null value.
fn opt_time(doc: &Document, key: &str) -> Option<DateTime<Utc>> {
    doc.get_datetime(key).ok().map(from_bson_time)
}

/// Reads an optional string field, mapping BSON null to `None`.
fn opt_str(doc: &Document, key: &str) -> Option<String> {
    doc.get_str(key).ok().map(str::to_string)
}

/// Decodes a field written as a JSON string back into a value.
fn opt_json<T: serde::de::DeserializeOwned>(doc: &Document, key: &str) -> Option<T> {
    doc.get_str(key)
        .ok()
        .and_then(|s| serde_json::from_str(s).ok())
}

/// Encodes a value as a JSON string for storage (see the module docs on why
/// arbitrary JSON is not stored as a BSON sub-document).
fn json_string<T: serde::Serialize + ?Sized>(value: &T) -> StorageResult<String> {
    serde_json::to_string(value).map_err(|e| internal_error(format!("encode JSON field: {e}")))
}

/// Builds a `LeaseError::LeaseLost` for a submit manifest (the shared variant
/// carries an `ExportJobId`, so we encode `submission/manifest` into it).
fn lease_lost(lease: &ManifestLease) -> LeaseError {
    LeaseError::LeaseLost {
        job_id: ExportJobId::from_string(format!("{}/{}", lease.submission_id, lease.manifest_id)),
    }
}

/// Derives the ingest FHIR version from a stored `outputFormat` MIME string.
fn fhir_version_from_output_format(output_format: Option<&str>) -> FhirVersion {
    output_format
        .and_then(|fmt| {
            fmt.split(';').find_map(|part| {
                let part = part.trim();
                part.strip_prefix("fhirVersion=")
                    .and_then(FhirVersion::from_mime_param)
            })
        })
        .unwrap_or_else(FhirVersion::default_enabled)
}

/// The `(tenant, submitter, submission_id)` selector shared by every submission
/// -scoped query.
fn submission_filter(tenant: &TenantContext, id: &SubmissionId) -> Document {
    doc! {
        "tenant_id": tenant.tenant_id().as_str(),
        "submitter": &id.submitter,
        "submission_id": &id.submission_id,
    }
}

/// The submission selector plus a manifest ID.
fn manifest_filter(tenant: &TenantContext, id: &SubmissionId, manifest_id: &str) -> Document {
    let mut filter = submission_filter(tenant, id);
    filter.insert("manifest_id", manifest_id);
    filter
}

/// The manifest selector plus the lease's worker + fencing token — the guard
/// that makes a zombie worker's write affect zero documents.
fn fenced_filter(lease: &ManifestLease) -> Document {
    let mut filter = manifest_filter(&lease.tenant, &lease.submission_id, &lease.manifest_id);
    filter.insert("worker_id", lease.worker_id.as_str());
    filter.insert("fencing_token", lease.fencing_token as i64);
    filter
}

/// Rebuilds a full-access `TenantContext` from a stored `tenant_id`.
///
/// The worker and cleanup paths resolve documents before any request context
/// exists, exactly as the SQLite/Postgres job stores do.
fn tenant_from_id(tenant_id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access())
}

fn decode_manifest(doc: &Document) -> StorageResult<SubmissionManifest> {
    let manifest_id = doc
        .get_str("manifest_id")
        .map_err(|e| internal_error(format!("manifest missing manifest_id: {e}")))?
        .to_string();
    let status: ManifestStatus = doc
        .get_str("status")
        .unwrap_or("pending")
        .parse()
        .map_err(|e: String| internal_error(e))?;
    Ok(SubmissionManifest {
        manifest_id,
        manifest_url: opt_str(doc, "manifest_url"),
        replaces_manifest_url: opt_str(doc, "replaces_manifest_url"),
        status,
        added_at: opt_time(doc, "added_at").unwrap_or_else(Utc::now),
        lease_expiry: opt_time(doc, "lease_expiry"),
        total_entries: doc.get_i64("total_entries").unwrap_or(0).max(0) as u64,
        processed_entries: doc.get_i64("processed_entries").unwrap_or(0).max(0) as u64,
        failed_entries: doc.get_i64("failed_entries").unwrap_or(0).max(0) as u64,
        bytes_processed: doc.get_i64("bytes_processed").unwrap_or(0).max(0) as u64,
        bytes_total: doc.get_i64("bytes_total").unwrap_or(0).max(0) as u64,
    })
}

fn decode_entry_result(doc: &Document) -> BulkEntryResult {
    let outcome: BulkEntryOutcome = doc
        .get_str("outcome")
        .unwrap_or("processing-error")
        .parse()
        .unwrap_or(BulkEntryOutcome::ProcessingError);
    BulkEntryResult {
        line_number: doc.get_i64("line_number").unwrap_or(0).max(0) as u64,
        resource_type: doc
            .get_str("resource_type")
            .unwrap_or("Resource")
            .to_string(),
        resource_id: opt_str(doc, "resource_id"),
        created: doc.get_bool("created").unwrap_or(false),
        outcome,
        operation_outcome: opt_json(doc, "operation_outcome"),
    }
}

fn decode_change(doc: &Document) -> SubmissionChange {
    let change_type: ChangeType = doc
        .get_str("change_type")
        .unwrap_or("create")
        .parse()
        .unwrap_or(ChangeType::Create);
    SubmissionChange {
        change_id: doc.get_str("change_id").unwrap_or_default().to_string(),
        manifest_id: doc.get_str("manifest_id").unwrap_or_default().to_string(),
        change_type,
        resource_type: doc.get_str("resource_type").unwrap_or_default().to_string(),
        resource_id: doc.get_str("resource_id").unwrap_or_default().to_string(),
        previous_version: opt_str(doc, "previous_version"),
        new_version: doc.get_str("new_version").unwrap_or_default().to_string(),
        previous_content: opt_json(doc, "previous_content"),
        changed_at: opt_time(doc, "changed_at").unwrap_or_else(Utc::now),
    }
}

impl MongoBackend {
    async fn submissions(&self) -> StorageResult<Collection<Document>> {
        Ok(self
            .get_database()
            .await?
            .collection(SUBMISSIONS_COLLECTION))
    }

    async fn manifests(&self) -> StorageResult<Collection<Document>> {
        Ok(self.get_database().await?.collection(MANIFESTS_COLLECTION))
    }

    async fn entry_results(&self) -> StorageResult<Collection<Document>> {
        Ok(self
            .get_database()
            .await?
            .collection(ENTRY_RESULTS_COLLECTION))
    }

    async fn submission_changes(&self) -> StorageResult<Collection<Document>> {
        Ok(self.get_database().await?.collection(CHANGES_COLLECTION))
    }

    async fn submit_files(&self) -> StorageResult<Collection<Document>> {
        Ok(self
            .get_database()
            .await?
            .collection(SUBMIT_FILES_COLLECTION))
    }

    /// Loads a submission document, or `SubmissionNotFound`.
    async fn load_submission_doc(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Document> {
        self.submissions()
            .await?
            .find_one(submission_filter(tenant, id))
            .await
            .map_err(|e| internal_error(format!("read submission: {e}")))?
            .ok_or_else(|| {
                StorageError::BulkSubmit(BulkSubmitError::SubmissionNotFound {
                    submitter: id.submitter.clone(),
                    submission_id: id.submission_id.clone(),
                })
            })
    }

    /// Bumps a submission's `updated_at` (the TTL cleanup scan reads it).
    async fn touch_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        self.submissions()
            .await?
            .update_one(
                submission_filter(tenant, id),
                doc! { "$set": { "updated_at": to_bson_time(Utc::now()) } },
            )
            .await
            .map_err(|e| internal_error(format!("touch submission: {e}")))?;
        Ok(())
    }

    /// Counts entry results for a submission (optionally one manifest), grouped
    /// into the outcome buckets both `get_submission` and `get_entry_counts`
    /// report.
    async fn count_outcomes(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_id: Option<&str>,
    ) -> StorageResult<EntryCountSummary> {
        let results = self.entry_results().await?;
        let base = match manifest_id {
            Some(m) => manifest_filter(tenant, id, m),
            None => submission_filter(tenant, id),
        };

        let mut counts = [0u64; 4];
        for (slot, outcome) in [
            BulkEntryOutcome::Success,
            BulkEntryOutcome::ValidationError,
            BulkEntryOutcome::ProcessingError,
            BulkEntryOutcome::Skipped,
        ]
        .into_iter()
        .enumerate()
        {
            let mut filter = base.clone();
            filter.insert("outcome", outcome.to_string());
            counts[slot] = results
                .count_documents(filter)
                .await
                .map_err(|e| internal_error(format!("count entry results: {e}")))?;
        }

        let mut summary = EntryCountSummary::new();
        summary.success = counts[0];
        summary.validation_error = counts[1];
        summary.processing_error = counts[2];
        summary.skipped = counts[3];
        summary.total =
            summary.success + summary.validation_error + summary.processing_error + summary.skipped;
        Ok(summary)
    }

    /// Ingests one NDJSON entry: upserts the resource per the import mode and
    /// records a rollback change.
    async fn process_single_entry(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entry: &NdjsonEntry,
        options: &BulkProcessingOptions,
    ) -> StorageResult<BulkEntryResult> {
        if let Some(resource_type) = entry.resource.get("resourceType").and_then(|v| v.as_str())
            && resource_type != entry.resource_type
        {
            return Ok(BulkEntryResult::validation_error(
                entry.line_number,
                &entry.resource_type,
                serde_json::json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{
                        "severity": "error",
                        "code": "invalid",
                        "diagnostics": format!(
                            "resourceType mismatch: entry={}, payload={}",
                            entry.resource_type, resource_type
                        )
                    }]
                }),
            ));
        }

        let existing = match entry.resource_id.as_deref() {
            Some(id) => match self.read(tenant, &entry.resource_type, id).await {
                Ok(found) => found,
                // A tombstoned resource is a create target, not an error.
                Err(StorageError::Resource(ResourceError::Gone { .. })) => None,
                Err(e) => return Err(e),
            },
            None => None,
        };

        if let Some(current) = existing {
            if !options.allow_updates {
                return Ok(BulkEntryResult::skipped(
                    entry.line_number,
                    &entry.resource_type,
                    "updates not allowed",
                ));
            }
            let content = options.content_for_update(current.content(), &entry.resource);
            let updated = self.update(tenant, &current, content).await?;
            let change = SubmissionChange::update(
                manifest_id,
                &entry.resource_type,
                updated.id(),
                current.version_id(),
                updated.version_id(),
                current.content().clone(),
            );
            self.record_change(tenant, submission_id, &change).await?;
            return Ok(BulkEntryResult::success(
                entry.line_number,
                &entry.resource_type,
                updated.id(),
                false,
            ));
        }

        let created = self
            .create(
                tenant,
                &entry.resource_type,
                entry.resource.clone(),
                FhirVersion::default_enabled(),
            )
            .await?;
        let change = SubmissionChange::create(
            manifest_id,
            &entry.resource_type,
            created.id(),
            created.version_id(),
        );
        self.record_change(tenant, submission_id, &change).await?;
        Ok(BulkEntryResult::success(
            entry.line_number,
            &entry.resource_type,
            created.id(),
            true,
        ))
    }

    /// Upserts one entry result.
    ///
    /// Keyed by `(manifest, file_url, line_number)`: line numbers restart in
    /// every manifest output file, so without the file every file after the
    /// first collides with the first (#457). The upsert is what makes a worker
    /// re-fetching a whole file after a transient failure overwrite its own
    /// earlier rows instead of duplicating them.
    async fn store_entry_result(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        file_url: &str,
        result: &BulkEntryResult,
    ) -> StorageResult<()> {
        let mut key = manifest_filter(tenant, submission_id, manifest_id);
        key.insert("file_url", file_url);
        key.insert("line_number", result.line_number as i64);

        let operation_outcome = match &result.operation_outcome {
            Some(v) => Some(json_string(v)?),
            None => None,
        };
        let update = doc! {
            "$set": {
                "resource_type": &result.resource_type,
                "resource_id": result.resource_id.as_deref(),
                "created": result.created,
                "outcome": result.outcome.to_string(),
                "operation_outcome": operation_outcome,
            },
            "$setOnInsert": key.clone(),
        };

        self.entry_results()
            .await?
            .update_one(key, update)
            .upsert(true)
            .await
            .map_err(|e| internal_error(format!("store entry result: {e}")))?;
        Ok(())
    }

    /// Applies a fenced `$set`/`$inc` to the leased manifest, reporting
    /// `LeaseLost` when the guard matches nothing.
    async fn fenced_update(
        &self,
        lease: &ManifestLease,
        update: Document,
    ) -> Result<(), LeaseError> {
        let manifests = self.manifests().await.map_err(LeaseError::Storage)?;
        let result = manifests
            .update_one(fenced_filter(lease), update)
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("fenced update: {e}"))))?;
        if result.matched_count == 0 {
            Err(lease_lost(lease))
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl BulkSubmitProvider for MongoBackend {
    async fn create_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        metadata: Option<Value>,
    ) -> StorageResult<SubmissionSummary> {
        let submissions = self.submissions().await?;
        let filter = submission_filter(tenant, id);

        if submissions
            .find_one(filter.clone())
            .await
            .map_err(|e| internal_error(format!("check duplicate submission: {e}")))?
            .is_some()
        {
            return Err(StorageError::BulkSubmit(
                BulkSubmitError::DuplicateSubmission {
                    submitter: id.submitter.clone(),
                    submission_id: id.submission_id.clone(),
                },
            ));
        }

        let now = Utc::now();
        let metadata_json = match &metadata {
            Some(v) => Some(json_string(v)?),
            None => None,
        };
        let mut document = filter;
        document.insert("status", SubmissionStatus::InProgress.to_string());
        document.insert("created_at", to_bson_time(now));
        document.insert("updated_at", to_bson_time(now));
        document.insert("metadata", metadata_json);

        submissions
            .insert_one(document)
            .await
            .map_err(|e| internal_error(format!("create submission: {e}")))?;

        Ok(SubmissionSummary {
            id: id.clone(),
            status: SubmissionStatus::InProgress,
            created_at: now,
            updated_at: now,
            completed_at: None,
            manifest_count: 0,
            total_entries: 0,
            success_count: 0,
            error_count: 0,
            skipped_count: 0,
            metadata,
        })
    }

    async fn get_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Option<SubmissionSummary>> {
        let Some(document) = self
            .submissions()
            .await?
            .find_one(submission_filter(tenant, id))
            .await
            .map_err(|e| internal_error(format!("read submission: {e}")))?
        else {
            return Ok(None);
        };

        let status: SubmissionStatus = document
            .get_str("status")
            .unwrap_or("in-progress")
            .parse()
            .map_err(|e: String| internal_error(e))?;
        let manifest_count = self
            .manifests()
            .await?
            .count_documents(submission_filter(tenant, id))
            .await
            .map_err(|e| internal_error(format!("count manifests: {e}")))?;
        let counts = self.count_outcomes(tenant, id, None).await?;

        Ok(Some(SubmissionSummary {
            id: id.clone(),
            status,
            created_at: opt_time(&document, "created_at").unwrap_or_else(Utc::now),
            updated_at: opt_time(&document, "updated_at").unwrap_or_else(Utc::now),
            completed_at: opt_time(&document, "completed_at"),
            manifest_count: manifest_count as u32,
            total_entries: counts.total,
            success_count: counts.success,
            error_count: counts.error_count(),
            skipped_count: counts.skipped,
            metadata: opt_json(&document, "metadata"),
        }))
    }

    async fn list_submissions(
        &self,
        tenant: &TenantContext,
        submitter: Option<&str>,
        status: Option<SubmissionStatus>,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<SubmissionSummary>> {
        let mut filter = doc! { "tenant_id": tenant.tenant_id().as_str() };
        if let Some(submitter) = submitter {
            filter.insert("submitter", submitter);
        }
        if let Some(status) = status {
            filter.insert("status", status.to_string());
        }

        let options = FindOptions::builder()
            .sort(doc! { "created_at": -1_i32 })
            .skip(Some(offset as u64))
            .limit(Some(limit as i64))
            .build();
        let cursor = self
            .submissions()
            .await?
            .find(filter)
            .with_options(options)
            .await
            .map_err(|e| internal_error(format!("list submissions: {e}")))?;

        let mut out = Vec::new();
        for document in collect(cursor).await? {
            let (Ok(submitter), Ok(submission_id)) = (
                document.get_str("submitter"),
                document.get_str("submission_id"),
            ) else {
                continue;
            };
            let id = SubmissionId::new(submitter, submission_id);
            if let Some(summary) = self.get_submission(tenant, &id).await? {
                out.push(summary);
            }
        }
        Ok(out)
    }

    async fn complete_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<SubmissionSummary> {
        let document = self.load_submission_doc(tenant, id).await?;
        if document.get_str("status").unwrap_or_default()
            != SubmissionStatus::InProgress.to_string()
        {
            return Err(StorageError::BulkSubmit(BulkSubmitError::AlreadyComplete {
                submission_id: id.submission_id.clone(),
            }));
        }

        let now = to_bson_time(Utc::now());
        self.submissions()
            .await?
            .update_one(
                submission_filter(tenant, id),
                doc! { "$set": {
                    "status": SubmissionStatus::Complete.to_string(),
                    "completed_at": now,
                    "updated_at": now,
                }},
            )
            .await
            .map_err(|e| internal_error(format!("complete submission: {e}")))?;

        self.get_submission(tenant, id)
            .await?
            .ok_or_else(|| internal_error("submission disappeared while completing"))
    }

    async fn abort_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        reason: &str,
    ) -> StorageResult<u64> {
        let document = self.load_submission_doc(tenant, id).await?;
        if document.get_str("status").unwrap_or_default()
            != SubmissionStatus::InProgress.to_string()
        {
            return Err(StorageError::BulkSubmit(BulkSubmitError::AlreadyComplete {
                submission_id: id.submission_id.clone(),
            }));
        }

        let manifests = self.manifests().await?;
        let mut pending_filter = submission_filter(tenant, id);
        pending_filter.insert(
            "status",
            doc! { "$in": [
                ManifestStatus::Pending.to_string(),
                ManifestStatus::Processing.to_string(),
            ]},
        );
        let pending = manifests
            .count_documents(pending_filter.clone())
            .await
            .map_err(|e| internal_error(format!("count pending manifests: {e}")))?;
        manifests
            .update_many(
                pending_filter,
                doc! { "$set": { "status": ManifestStatus::Failed.to_string() } },
            )
            .await
            .map_err(|e| internal_error(format!("fail pending manifests: {e}")))?;

        let now = to_bson_time(Utc::now());
        self.submissions()
            .await?
            .update_one(
                submission_filter(tenant, id),
                doc! { "$set": {
                    "status": SubmissionStatus::Aborted.to_string(),
                    "abort_reason": reason,
                    "completed_at": now,
                    "updated_at": now,
                }},
            )
            .await
            .map_err(|e| internal_error(format!("abort submission: {e}")))?;

        Ok(pending)
    }

    async fn add_manifest(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_url: Option<&str>,
        replaces_manifest_url: Option<&str>,
    ) -> StorageResult<SubmissionManifest> {
        let document = self.load_submission_doc(tenant, submission_id).await?;
        let status = document.get_str("status").unwrap_or("in-progress");
        if status == SubmissionStatus::Aborted.to_string() {
            return Err(StorageError::BulkSubmit(BulkSubmitError::Aborted {
                submission_id: submission_id.submission_id.clone(),
                reason: opt_str(&document, "abort_reason").unwrap_or_else(|| "aborted".to_string()),
            }));
        }
        if status != SubmissionStatus::InProgress.to_string() {
            return Err(StorageError::BulkSubmit(BulkSubmitError::InvalidState {
                submission_id: submission_id.submission_id.clone(),
                expected: SubmissionStatus::InProgress.to_string(),
                actual: status.to_string(),
            }));
        }

        let manifest = SubmissionManifest {
            manifest_id: Uuid::new_v4().to_string(),
            manifest_url: manifest_url.map(str::to_string),
            replaces_manifest_url: replaces_manifest_url.map(str::to_string),
            status: ManifestStatus::Pending,
            added_at: Utc::now(),
            total_entries: 0,
            processed_entries: 0,
            failed_entries: 0,
            lease_expiry: None,
            bytes_processed: 0,
            bytes_total: 0,
        };

        let mut document = manifest_filter(tenant, submission_id, &manifest.manifest_id);
        document.insert("manifest_url", manifest.manifest_url.as_deref());
        document.insert(
            "replaces_manifest_url",
            manifest.replaces_manifest_url.as_deref(),
        );
        document.insert("status", manifest.status.to_string());
        document.insert("added_at", to_bson_time(manifest.added_at));
        document.insert("total_entries", 0_i64);
        document.insert("processed_entries", 0_i64);
        document.insert("failed_entries", 0_i64);
        document.insert("fencing_token", 0_i64);
        document.insert("last_processed_line", 0_i64);
        document.insert("bytes_processed", 0_i64);
        document.insert("bytes_total", 0_i64);

        self.manifests()
            .await?
            .insert_one(document)
            .await
            .map_err(|e| internal_error(format!("add manifest: {e}")))?;
        self.touch_submission(tenant, submission_id).await?;

        Ok(manifest)
    }

    async fn get_manifest(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<Option<SubmissionManifest>> {
        let found = self
            .manifests()
            .await?
            .find_one(manifest_filter(tenant, submission_id, manifest_id))
            .await
            .map_err(|e| internal_error(format!("read manifest: {e}")))?;
        found.as_ref().map(decode_manifest).transpose()
    }

    async fn list_manifests(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
    ) -> StorageResult<Vec<SubmissionManifest>> {
        let options = FindOptions::builder()
            .sort(doc! { "added_at": 1_i32 })
            .build();
        let cursor = self
            .manifests()
            .await?
            .find(submission_filter(tenant, submission_id))
            .with_options(options)
            .await
            .map_err(|e| internal_error(format!("list manifests: {e}")))?;
        collect(cursor).await?.iter().map(decode_manifest).collect()
    }

    async fn process_entries(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entries: Vec<NdjsonEntry>,
        options: &BulkProcessingOptions,
    ) -> StorageResult<Vec<BulkEntryResult>> {
        if self
            .get_manifest(tenant, submission_id, manifest_id)
            .await?
            .is_none()
        {
            return Err(StorageError::BulkSubmit(
                BulkSubmitError::ManifestNotFound {
                    submission_id: submission_id.submission_id.clone(),
                    manifest_id: manifest_id.to_string(),
                },
            ));
        }

        let manifests = self.manifests().await?;
        manifests
            .update_one(
                manifest_filter(tenant, submission_id, manifest_id),
                doc! { "$set": { "status": ManifestStatus::Processing.to_string() } },
            )
            .await
            .map_err(|e| internal_error(format!("mark manifest processing: {e}")))?;

        let file_url = options.file_url.as_deref().unwrap_or("");
        let mut results = Vec::new();
        let mut error_count = 0u32;

        for entry in entries {
            if options.max_errors > 0 && error_count >= options.max_errors {
                if !options.continue_on_error {
                    return Err(StorageError::BulkSubmit(
                        BulkSubmitError::MaxErrorsExceeded {
                            submission_id: submission_id.submission_id.clone(),
                            max_errors: options.max_errors,
                        },
                    ));
                }
                let skipped = BulkEntryResult::skipped(
                    entry.line_number,
                    &entry.resource_type,
                    "max errors exceeded",
                );
                self.store_entry_result(tenant, submission_id, manifest_id, file_url, &skipped)
                    .await?;
                results.push(skipped);
                continue;
            }

            let result = match self
                .process_single_entry(tenant, submission_id, manifest_id, &entry, options)
                .await
            {
                Ok(result) => result,
                Err(e) => BulkEntryResult::processing_error(
                    entry.line_number,
                    &entry.resource_type,
                    serde_json::json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{
                            "severity": "error",
                            "code": "exception",
                            "diagnostics": e.to_string()
                        }]
                    }),
                ),
            };
            if result.is_error() {
                error_count += 1;
            }

            self.store_entry_result(tenant, submission_id, manifest_id, file_url, &result)
                .await?;
            results.push(result);
        }

        manifests
            .update_one(
                manifest_filter(tenant, submission_id, manifest_id),
                doc! { "$inc": {
                    "total_entries": results.len() as i64,
                    "processed_entries": results.iter().filter(|r| r.is_success()).count() as i64,
                    "failed_entries": error_count as i64,
                }},
            )
            .await
            .map_err(|e| internal_error(format!("update manifest counts: {e}")))?;
        self.touch_submission(tenant, submission_id).await?;

        Ok(results)
    }

    async fn get_entry_results(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        outcome_filter: Option<BulkEntryOutcome>,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<BulkEntryResult>> {
        let mut filter = manifest_filter(tenant, submission_id, manifest_id);
        if let Some(outcome) = outcome_filter {
            filter.insert("outcome", outcome.to_string());
        }
        let options = FindOptions::builder()
            .sort(doc! { "line_number": 1_i32, "file_url": 1_i32 })
            .skip(Some(offset as u64))
            .limit(Some(limit as i64))
            .build();
        let cursor = self
            .entry_results()
            .await?
            .find(filter)
            .with_options(options)
            .await
            .map_err(|e| internal_error(format!("query entry results: {e}")))?;
        Ok(collect(cursor)
            .await?
            .iter()
            .map(decode_entry_result)
            .collect())
    }

    async fn get_entry_counts(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<EntryCountSummary> {
        self.count_outcomes(tenant, submission_id, Some(manifest_id))
            .await
    }
}

#[async_trait]
impl StreamingBulkSubmitProvider for MongoBackend {
    async fn process_ndjson_stream(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        resource_type: &str,
        mut reader: Box<dyn AsyncBufRead + Send + Unpin>,
        options: &BulkProcessingOptions,
    ) -> StorageResult<StreamProcessingResult> {
        let mut result = StreamProcessingResult::new();
        let mut line_number = 0u64;
        let mut batch = Vec::new();

        loop {
            let mut line = String::new();
            let bytes_read = reader.read_line(&mut line).await.map_err(|e| {
                StorageError::BulkSubmit(BulkSubmitError::ParseError {
                    line: line_number,
                    message: format!("failed to read line: {e}"),
                })
            })?;
            if bytes_read == 0 {
                break;
            }

            line_number += 1;
            result.lines_processed = line_number;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            match NdjsonEntry::parse(line_number, line) {
                Ok(entry) => {
                    if entry.resource_type != resource_type {
                        result.counts.increment(BulkEntryOutcome::ValidationError);
                        if !options.continue_on_error
                            && (options.max_errors == 0
                                || result.counts.error_count() >= options.max_errors as u64)
                        {
                            return Ok(result.aborted("max errors exceeded"));
                        }
                        continue;
                    }
                    batch.push(entry);
                }
                Err(parse_err) => {
                    result.counts.increment(BulkEntryOutcome::ValidationError);
                    if !options.continue_on_error
                        && (options.max_errors == 0
                            || result.counts.error_count() >= options.max_errors as u64)
                    {
                        return Ok(result.aborted(format!("parse error: {parse_err}")));
                    }
                }
            }

            if batch.len() >= options.batch_size as usize {
                let batch_results = self
                    .process_entries(
                        tenant,
                        submission_id,
                        manifest_id,
                        std::mem::take(&mut batch),
                        options,
                    )
                    .await?;
                for entry_result in batch_results {
                    result.counts.increment(entry_result.outcome);
                }
                if !options.continue_on_error
                    && options.max_errors > 0
                    && result.counts.error_count() >= options.max_errors as u64
                {
                    return Ok(result.aborted("max errors exceeded"));
                }
            }
        }

        if !batch.is_empty() {
            let batch_results = self
                .process_entries(tenant, submission_id, manifest_id, batch, options)
                .await?;
            for entry_result in batch_results {
                result.counts.increment(entry_result.outcome);
            }
        }

        Ok(result)
    }
}

#[async_trait]
impl BulkSubmitRollbackProvider for MongoBackend {
    async fn record_change(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        change: &SubmissionChange,
    ) -> StorageResult<()> {
        let previous_content = match &change.previous_content {
            Some(v) => Some(json_string(v)?),
            None => None,
        };
        let mut document = submission_filter(tenant, submission_id);
        document.insert("change_id", &change.change_id);
        document.insert("manifest_id", &change.manifest_id);
        document.insert("change_type", change.change_type.to_string());
        document.insert("resource_type", &change.resource_type);
        document.insert("resource_id", &change.resource_id);
        document.insert("previous_version", change.previous_version.as_deref());
        document.insert("new_version", &change.new_version);
        document.insert("previous_content", previous_content);
        document.insert("changed_at", to_bson_time(change.changed_at));

        self.submission_changes()
            .await?
            .insert_one(document)
            .await
            .map_err(|e| internal_error(format!("record change: {e}")))?;
        Ok(())
    }

    async fn list_changes(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<SubmissionChange>> {
        let options = FindOptions::builder()
            .sort(doc! { "changed_at": -1_i32 })
            .skip(Some(offset as u64))
            .limit(Some(limit as i64))
            .build();
        let cursor = self
            .submission_changes()
            .await?
            .find(submission_filter(tenant, submission_id))
            .with_options(options)
            .await
            .map_err(|e| internal_error(format!("list changes: {e}")))?;
        Ok(collect(cursor).await?.iter().map(decode_change).collect())
    }

    async fn rollback_change(
        &self,
        tenant: &TenantContext,
        _submission_id: &SubmissionId,
        change: &SubmissionChange,
    ) -> StorageResult<bool> {
        match change.change_type {
            ChangeType::Create => match self
                .delete(tenant, &change.resource_type, &change.resource_id)
                .await
            {
                Ok(())
                | Err(StorageError::Resource(ResourceError::NotFound { .. }))
                | Err(StorageError::Resource(ResourceError::Gone { .. })) => Ok(true),
                Err(e) => Err(e),
            },
            ChangeType::Update => {
                let Some(previous_content) = &change.previous_content else {
                    return Ok(false);
                };
                match self
                    .read(tenant, &change.resource_type, &change.resource_id)
                    .await?
                {
                    Some(current) => {
                        self.update(tenant, &current, previous_content.clone())
                            .await?;
                        Ok(true)
                    }
                    None => Ok(false),
                }
            }
        }
    }
}

#[async_trait]
impl SubmitClaimStrategy for MongoBackend {
    async fn claim_next_manifest(
        &self,
        worker_id: &WorkerId,
        lease_duration: StdDuration,
    ) -> StorageResult<Option<ManifestLease>> {
        let manifests = self.manifests().await?;
        let now = Utc::now();
        // Rounded to what BSON stores, so the expiry the worker reasons about is
        // exactly the one a reclaiming worker compares against.
        let lease_expiry = now_at_bson_precision()
            + chrono::Duration::from_std(lease_duration)
                .unwrap_or_else(|_| chrono::Duration::seconds(60));

        // Eligible: has something to fetch, and is either untouched or held by a
        // worker that stopped heartbeating.
        let eligible = doc! {
            "manifest_url": { "$ne": null },
            "$or": [
                { "status": ManifestStatus::Pending.to_string() },
                {
                    "status": ManifestStatus::Processing.to_string(),
                    "$or": [
                        { "lease_expiry": null },
                        { "lease_expiry": { "$lt": to_bson_time(now) } },
                    ],
                },
            ],
        };
        let options = FindOptions::builder()
            .sort(doc! { "added_at": 1_i32 })
            .limit(Some(CLAIM_CANDIDATES))
            .build();
        let cursor = manifests
            .find(eligible)
            .with_options(options)
            .await
            .map_err(|e| internal_error(format!("scan claimable manifests: {e}")))?;

        for candidate in collect(cursor).await? {
            let (Ok(tenant_id), Ok(submitter), Ok(submission_id), Ok(manifest_id)) = (
                candidate.get_str("tenant_id"),
                candidate.get_str("submitter"),
                candidate.get_str("submission_id"),
                candidate.get_str("manifest_id"),
            ) else {
                continue;
            };
            let tenant = tenant_from_id(tenant_id);
            let id = SubmissionId::new(submitter, submission_id);

            // `complete` is admitted alongside `in-progress`: it means the
            // submitter will send no further manifests, not that already
            // registered ones should be dropped. `aborted` stays excluded.
            let submission_status = self
                .submissions()
                .await?
                .find_one(submission_filter(&tenant, &id))
                .await
                .map_err(|e| internal_error(format!("read submission for claim: {e}")))?
                .and_then(|d| opt_str(&d, "status"));
            let admissible = matches!(
                submission_status.as_deref(),
                Some("in-progress") | Some("complete")
            );
            if !admissible {
                continue;
            }

            // CAS on the exact document we read: another worker that claimed it
            // in between bumped `fencing_token`, so our filter matches nothing.
            let token = candidate.get_i64("fencing_token").unwrap_or(0);
            let status = candidate.get_str("status").unwrap_or("pending").to_string();
            let mut guard = manifest_filter(&tenant, &id, manifest_id);
            guard.insert("fencing_token", token);
            guard.insert("status", status);

            let claimed = manifests
                .find_one_and_update(
                    guard,
                    doc! {
                        "$set": {
                            "status": ManifestStatus::Processing.to_string(),
                            "worker_id": worker_id.as_str(),
                            "lease_expiry": to_bson_time(lease_expiry),
                        },
                        "$inc": { "fencing_token": 1_i64 },
                    },
                )
                .return_document(ReturnDocument::After)
                .await
                .map_err(|e| internal_error(format!("claim manifest: {e}")))?;

            let Some(claimed) = claimed else {
                continue;
            };
            return Ok(Some(ManifestLease {
                tenant,
                submission_id: id,
                manifest_id: manifest_id.to_string(),
                worker_id: worker_id.clone(),
                lease_expiry,
                lease_duration,
                fencing_token: claimed.get_i64("fencing_token").unwrap_or(token + 1).max(0) as u64,
            }));
        }

        Ok(None)
    }

    async fn heartbeat(&self, lease: &ManifestLease) -> Result<DateTime<Utc>, LeaseError> {
        let new_expiry = from_bson_time(&to_bson_time(lease.renewed_expiry()));
        self.fenced_update(
            lease,
            doc! { "$set": { "lease_expiry": to_bson_time(new_expiry) } },
        )
        .await?;
        Ok(new_expiry)
    }

    async fn release(&self, lease: ManifestLease) -> StorageResult<()> {
        let mut filter = fenced_filter(&lease);
        filter.insert("status", ManifestStatus::Processing.to_string());
        self.manifests()
            .await?
            .update_one(
                filter,
                doc! { "$set": {
                    "status": ManifestStatus::Pending.to_string(),
                    "worker_id": null,
                    "lease_expiry": null,
                }},
            )
            .await
            .map_err(|e| internal_error(format!("release manifest lease: {e}")))?;
        Ok(())
    }
}

#[async_trait]
impl SubmitWorkerStorage for MongoBackend {
    async fn get_manifest_for_worker(
        &self,
        lease: &ManifestLease,
    ) -> Result<ManifestWorkerView, LeaseError> {
        let manifests = self.manifests().await.map_err(LeaseError::Storage)?;
        let document = manifests
            .find_one(fenced_filter(lease))
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("read leased manifest: {e}"))))?
            .ok_or_else(|| lease_lost(lease))?;

        let output_format = opt_str(&document, "output_format");
        Ok(ManifestWorkerView {
            manifest_id: lease.manifest_id.clone(),
            manifest_url: opt_str(&document, "manifest_url"),
            fhir_base_url: opt_str(&document, "fhir_base_url"),
            fhir_version: fhir_version_from_output_format(output_format.as_deref()),
            output_format,
            file_request_headers: opt_json(&document, "file_request_headers").unwrap_or_default(),
            oauth_metadata_urls: opt_json(&document, "oauth_metadata_urls").unwrap_or_default(),
            file_encryption_key: opt_json(&document, "file_encryption_key"),
            import_directives: opt_json(&document, "import_directives").unwrap_or_default(),
            metadata: opt_json(&document, "submission_metadata").unwrap_or_default(),
            last_processed_line: document.get_i64("last_processed_line").unwrap_or(0).max(0) as u64,
        })
    }

    async fn mark_manifest_processing(&self, lease: &ManifestLease) -> Result<(), LeaseError> {
        self.fenced_update(
            lease,
            doc! { "$set": { "status": ManifestStatus::Processing.to_string() } },
        )
        .await
    }

    async fn update_manifest_progress(
        &self,
        lease: &ManifestLease,
        processed_entries: u64,
        failed_entries: u64,
        last_processed_line: u64,
    ) -> Result<(), LeaseError> {
        self.fenced_update(
            lease,
            doc! { "$set": {
                "processed_entries": processed_entries as i64,
                "failed_entries": failed_entries as i64,
                "last_processed_line": last_processed_line as i64,
            }},
        )
        .await
    }

    async fn update_manifest_bytes(
        &self,
        lease: &ManifestLease,
        bytes_processed: u64,
        bytes_total: u64,
    ) -> Result<(), LeaseError> {
        self.fenced_update(
            lease,
            doc! { "$max": {
                "bytes_processed": bytes_processed as i64,
                "bytes_total": bytes_total as i64,
            }},
        )
        .await
    }

    async fn record_submit_file(
        &self,
        lease: &ManifestLease,
        file: &SubmitFileRecord,
    ) -> Result<(), LeaseError> {
        let manifests = self.manifests().await.map_err(LeaseError::Storage)?;
        let holds = manifests
            .find_one(fenced_filter(lease))
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("fence check: {e}"))))?
            .is_some();
        if !holds {
            return Err(lease_lost(lease));
        }

        let count_severity = match &file.count_severity {
            Some(v) => Some(json_string(v).map_err(LeaseError::Storage)?),
            None => None,
        };
        // Keyed by the artifact's identity plus the writing worker's fencing
        // token, so a retried write replaces its own row rather than adding a
        // duplicate entry to the status manifest.
        let mut key = submission_filter(&lease.tenant, &lease.submission_id);
        key.insert("file_type", &file.file_type);
        key.insert("resource_type", file.resource_type.as_deref());
        key.insert("part_index", file.part_index as i64);
        key.insert("fencing_token", lease.fencing_token as i64);

        let update = doc! {
            "$set": {
                "manifest_url": file.manifest_url.as_deref(),
                "file_path": &file.file_path,
                "line_count": file.line_count as i64,
                "byte_count": file.byte_count as i64,
                "count_severity": count_severity,
                "created_at": to_bson_time(Utc::now()),
            },
            "$setOnInsert": key.clone(),
        };
        self.submit_files()
            .await
            .map_err(LeaseError::Storage)?
            .update_one(key, update)
            .upsert(true)
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("record submit file: {e}"))))?;
        Ok(())
    }

    async fn finish_manifest(&self, lease: &ManifestLease) -> Result<(), LeaseError> {
        self.fenced_update(
            lease,
            doc! { "$set": {
                "status": ManifestStatus::Completed.to_string(),
                "worker_id": null,
                "lease_expiry": null,
            }},
        )
        .await
    }

    async fn fail_manifest(
        &self,
        lease: &ManifestLease,
        error_message: &str,
    ) -> Result<(), LeaseError> {
        self.fenced_update(
            lease,
            doc! { "$set": {
                "status": ManifestStatus::Failed.to_string(),
                "error_message": error_message,
                "worker_id": null,
                "lease_expiry": null,
            }},
        )
        .await
    }

    async fn set_manifest_fetch_params(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_id: &str,
        params: ManifestFetchParams<'_>,
    ) -> StorageResult<()> {
        let file_encryption_key = match params.file_encryption_key {
            Some(v) => Some(json_string(v)?),
            None => None,
        };
        self.manifests()
            .await?
            .update_one(
                manifest_filter(tenant, id, manifest_id),
                doc! { "$set": {
                    "fhir_base_url": params.fhir_base_url,
                    "output_format": params.output_format,
                    "file_request_headers": json_string(params.file_request_headers)?,
                    "oauth_metadata_urls": json_string(params.oauth_metadata_urls)?,
                    "file_encryption_key": file_encryption_key,
                    "import_directives": json_string(params.import_directives)?,
                    "submission_metadata": json_string(params.metadata)?,
                }},
            )
            .await
            .map_err(|e| internal_error(format!("set manifest fetch params: {e}")))?;
        Ok(())
    }

    async fn replace_manifest_by_url(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_url: &str,
    ) -> StorageResult<Vec<String>> {
        let manifests = self.manifests().await?;
        let mut filter = submission_filter(tenant, id);
        filter.insert("manifest_url", manifest_url);

        let mut superseded_filter = filter.clone();
        superseded_filter.insert(
            "status",
            doc! { "$ne": ManifestStatus::Replaced.to_string() },
        );
        let cursor = manifests
            .find(superseded_filter)
            .await
            .map_err(|e| internal_error(format!("find replaced manifests: {e}")))?;
        let ids: Vec<String> = collect(cursor)
            .await?
            .iter()
            .filter_map(|d| opt_str(d, "manifest_id"))
            .collect();

        manifests
            .update_many(
                filter,
                doc! { "$set": { "status": ManifestStatus::Replaced.to_string() } },
            )
            .await
            .map_err(|e| internal_error(format!("mark manifests replaced: {e}")))?;
        Ok(ids)
    }

    async fn set_submission_kickoff_meta(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        owner_subject: Option<&str>,
        request_url: &str,
        requires_access_token: bool,
    ) -> StorageResult<()> {
        self.submissions()
            .await?
            .update_one(
                submission_filter(tenant, id),
                doc! { "$set": {
                    "owner_subject": owner_subject,
                    "request_url": request_url,
                    "requires_access_token": requires_access_token,
                }},
            )
            .await
            .map_err(|e| internal_error(format!("set kickoff meta: {e}")))?;
        Ok(())
    }

    async fn ensure_poll_token(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<String> {
        let submissions = self.submissions().await?;
        let filter = submission_filter(tenant, id);
        if let Some(existing) = submissions
            .find_one(filter.clone())
            .await
            .map_err(|e| internal_error(format!("read poll token: {e}")))?
            .and_then(|d| opt_str(&d, "poll_token"))
        {
            return Ok(existing);
        }

        let token = Uuid::new_v4().to_string();
        // Only mint a token if none appeared since the read; on a lost race the
        // winner's token is what every caller must see. A `null` equality match
        // covers both an explicit null and an absent field.
        let mut guarded = filter.clone();
        guarded.insert("poll_token", Bson::Null);
        let updated = submissions
            .update_one(guarded, doc! { "$set": { "poll_token": &token } })
            .await
            .map_err(|e| internal_error(format!("set poll token: {e}")))?;
        if updated.matched_count == 1 {
            return Ok(token);
        }

        submissions
            .find_one(filter)
            .await
            .map_err(|e| internal_error(format!("reread poll token: {e}")))?
            .and_then(|d| opt_str(&d, "poll_token"))
            .ok_or_else(|| {
                StorageError::BulkSubmit(BulkSubmitError::SubmissionNotFound {
                    submitter: id.submitter.clone(),
                    submission_id: id.submission_id.clone(),
                })
            })
    }

    async fn resolve_poll_token(&self, token: &str) -> StorageResult<Option<PollTokenTarget>> {
        let Some(document) = self
            .submissions()
            .await?
            .find_one(doc! { "poll_token": token })
            .await
            .map_err(|e| internal_error(format!("resolve poll token: {e}")))?
        else {
            return Ok(None);
        };
        let (Some(tenant_id), Some(submitter), Some(submission_id)) = (
            opt_str(&document, "tenant_id"),
            opt_str(&document, "submitter"),
            opt_str(&document, "submission_id"),
        ) else {
            return Ok(None);
        };
        Ok(Some(PollTokenTarget {
            tenant: tenant_from_id(&tenant_id),
            submission_id: SubmissionId::new(submitter, submission_id),
            owner_subject: opt_str(&document, "owner_subject"),
        }))
    }

    async fn clear_poll_token(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        self.submissions()
            .await?
            .update_one(
                submission_filter(tenant, id),
                doc! { "$set": { "poll_token": null } },
            )
            .await
            .map_err(|e| internal_error(format!("clear poll token: {e}")))?;
        Ok(())
    }

    async fn list_submit_files(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Vec<SubmitFileRow>> {
        let options = FindOptions::builder()
            .sort(doc! {
                "created_at": 1_i32,
                "file_type": 1_i32,
                "resource_type": 1_i32,
                "part_index": 1_i32,
            })
            .build();
        let cursor = self
            .submit_files()
            .await?
            .find(submission_filter(tenant, id))
            .with_options(options)
            .await
            .map_err(|e| internal_error(format!("list submit files: {e}")))?;

        Ok(collect(cursor)
            .await?
            .iter()
            .map(|d| SubmitFileRow {
                manifest_url: opt_str(d, "manifest_url"),
                file_type: d.get_str("file_type").unwrap_or_default().to_string(),
                resource_type: opt_str(d, "resource_type"),
                part_index: d.get_i64("part_index").unwrap_or(0).max(0) as u32,
                fencing_token: d.get_i64("fencing_token").unwrap_or(0).max(0) as u64,
                file_path: d.get_str("file_path").unwrap_or_default().to_string(),
                line_count: d.get_i64("line_count").unwrap_or(0).max(0) as u64,
                byte_count: d.get_i64("byte_count").unwrap_or(0).max(0) as u64,
                count_severity: opt_json(d, "count_severity"),
            })
            .collect())
    }

    async fn delete_submission_artifacts(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        self.submit_files()
            .await?
            .delete_many(submission_filter(tenant, id))
            .await
            .map_err(|e| internal_error(format!("delete submit files: {e}")))?;
        Ok(())
    }

    async fn count_active_submissions(&self, tenant: &TenantContext) -> StorageResult<u64> {
        self.submissions()
            .await?
            .count_documents(doc! {
                "tenant_id": tenant.tenant_id().as_str(),
                "status": SubmissionStatus::InProgress.to_string(),
            })
            .await
            .map_err(|e| internal_error(format!("count active submissions: {e}")))
    }

    async fn list_expired_submissions(
        &self,
        now: DateTime<Utc>,
        ttl: StdDuration,
        limit: u32,
    ) -> StorageResult<Vec<(TenantContext, SubmissionId)>> {
        let cutoff = now
            - chrono::Duration::from_std(ttl).unwrap_or_else(|_| chrono::Duration::seconds(86_400));
        let options = FindOptions::builder()
            .sort(doc! { "updated_at": 1_i32 })
            .limit(Some(limit as i64))
            .build();
        let cursor = self
            .submissions()
            .await?
            .find(doc! { "updated_at": { "$lt": to_bson_time(cutoff) } })
            .with_options(options)
            .await
            .map_err(|e| internal_error(format!("list expired submissions: {e}")))?;

        Ok(collect(cursor)
            .await?
            .iter()
            .filter_map(|d| {
                let tenant_id = opt_str(d, "tenant_id")?;
                let submitter = opt_str(d, "submitter")?;
                let submission_id = opt_str(d, "submission_id")?;
                Some((
                    tenant_from_id(&tenant_id),
                    SubmissionId::new(submitter, submission_id),
                ))
            })
            .collect())
    }

    async fn ensure_transaction_time(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<DateTime<Utc>> {
        let submissions = self.submissions().await?;
        let filter = submission_filter(tenant, id);
        if let Some(existing) = submissions
            .find_one(filter.clone())
            .await
            .map_err(|e| internal_error(format!("read transaction time: {e}")))?
            .and_then(|d| opt_time(&d, "transaction_time"))
        {
            return Ok(existing);
        }

        let now = now_at_bson_precision();
        let mut guarded = filter.clone();
        guarded.insert("transaction_time", Bson::Null);
        let updated = submissions
            .update_one(
                guarded,
                doc! { "$set": { "transaction_time": to_bson_time(now) } },
            )
            .await
            .map_err(|e| internal_error(format!("set transaction time: {e}")))?;
        if updated.matched_count == 1 {
            return Ok(now);
        }

        Ok(submissions
            .find_one(filter)
            .await
            .map_err(|e| internal_error(format!("reread transaction time: {e}")))?
            .and_then(|d| opt_time(&d, "transaction_time"))
            .unwrap_or(now))
    }
}

/// Drains a cursor into a vector of documents.
async fn collect(mut cursor: mongodb::Cursor<Document>) -> StorageResult<Vec<Document>> {
    let mut out = Vec::new();
    while cursor
        .advance()
        .await
        .map_err(|e| internal_error(format!("cursor advance: {e}")))?
    {
        out.push(
            cursor
                .deserialize_current()
                .map_err(|e| internal_error(format!("cursor deserialize: {e}")))?,
        );
    }
    Ok(out)
}
