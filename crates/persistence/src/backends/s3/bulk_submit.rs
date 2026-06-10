//! Bulk submission implementation for the S3 backend.
//!
//! Implements `BulkSubmitProvider`, `StreamingBulkSubmitProvider`, and
//! `BulkSubmitRollbackProvider`. All submission state — including manifests,
//! raw NDJSON lines, entry results, and change records — is persisted as
//! individual S3 objects keyed under the submission's prefix.

use async_trait::async_trait;
use chrono::Utc;
use helios_fhir::FhirVersion;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
use uuid::Uuid;

use crate::core::ResourceStorage;
use crate::core::VersionedStorage;
use crate::core::bulk_submit::{
    BulkEntryOutcome, BulkEntryResult, BulkProcessingOptions, BulkSubmitProvider,
    BulkSubmitRollbackProvider, ChangeType, EntryCountSummary, ManifestStatus, NdjsonEntry,
    StreamProcessingResult, StreamingBulkSubmitProvider, SubmissionChange, SubmissionId,
    SubmissionManifest, SubmissionStatus, SubmissionSummary,
};
use crate::error::{BulkSubmitError, ResourceError, StorageError, StorageResult};
use crate::tenant::TenantContext;

use super::backend::{S3Backend, TenantLocation};
use super::models::{SubmissionManifestState, SubmissionState};

#[async_trait]
impl BulkSubmitProvider for S3Backend {
    async fn create_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        metadata: Option<serde_json::Value>,
    ) -> StorageResult<SubmissionSummary> {
        let location = self.tenant_location(tenant)?;
        let state_key = location
            .keyspace
            .submit_state_key(&id.submitter, &id.submission_id);

        if self
            .client
            .head_object(&location.bucket, &state_key)
            .await
            .map_err(|e| self.map_client_error(e))?
            .is_some()
        {
            return Err(StorageError::BulkSubmit(
                BulkSubmitError::DuplicateSubmission {
                    submitter: id.submitter.clone(),
                    submission_id: id.submission_id.clone(),
                },
            ));
        }

        let mut summary = SubmissionSummary::new(id.clone());
        if let Some(metadata) = metadata {
            summary = summary.with_metadata(metadata);
        }

        let state = SubmissionState {
            summary: summary.clone(),
            abort_reason: None,
        };

        self.save_submission_state(&location, id, &state).await?;
        Ok(summary)
    }

    async fn get_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Option<SubmissionSummary>> {
        let location = self.tenant_location(tenant)?;
        Ok(self
            .load_submission_state_optional(&location, id)
            .await?
            .map(|s| s.summary))
    }

    async fn list_submissions(
        &self,
        tenant: &TenantContext,
        submitter: Option<&str>,
        status: Option<SubmissionStatus>,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<SubmissionSummary>> {
        let location = self.tenant_location(tenant)?;
        let prefix = location.keyspace.submit_root_prefix();

        let mut submissions = Vec::new();
        for object in self.list_objects_all(&location.bucket, &prefix).await? {
            if !object.key.ends_with("/state.json") {
                continue;
            }

            let Some((state, _)) = self
                .get_json_object::<SubmissionState>(&location.bucket, &object.key)
                .await?
            else {
                continue;
            };

            if let Some(submitter_filter) = submitter {
                if state.summary.id.submitter != submitter_filter {
                    continue;
                }
            }

            if let Some(status_filter) = status {
                if state.summary.status != status_filter {
                    continue;
                }
            }

            submissions.push(state.summary);
        }

        submissions.sort_by_key(|s| std::cmp::Reverse(s.created_at));

        let start = (offset as usize).min(submissions.len());
        let end = start.saturating_add(limit as usize).min(submissions.len());
        Ok(submissions[start..end].to_vec())
    }

    async fn complete_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<SubmissionSummary> {
        let location = self.tenant_location(tenant)?;
        let mut state = self.load_submission_state(&location, id).await?;

        if state.summary.status != SubmissionStatus::InProgress {
            return Err(StorageError::BulkSubmit(BulkSubmitError::AlreadyComplete {
                submission_id: id.submission_id.clone(),
            }));
        }

        let now = Utc::now();
        state.summary.status = SubmissionStatus::Complete;
        state.summary.updated_at = now;
        state.summary.completed_at = Some(now);

        self.save_submission_state(&location, id, &state).await?;
        Ok(state.summary)
    }

    async fn abort_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        reason: &str,
    ) -> StorageResult<u64> {
        let location = self.tenant_location(tenant)?;
        let mut state = self.load_submission_state(&location, id).await?;

        if state.summary.status != SubmissionStatus::InProgress {
            return Err(StorageError::BulkSubmit(BulkSubmitError::AlreadyComplete {
                submission_id: id.submission_id.clone(),
            }));
        }

        let mut pending_count = 0u64;
        let manifests = self.list_manifest_states(&location, id).await?;
        for mut manifest in manifests {
            if matches!(
                manifest.manifest.status,
                ManifestStatus::Pending | ManifestStatus::Processing
            ) {
                pending_count += 1;
                manifest.manifest.status = ManifestStatus::Failed;
                self.save_manifest_state(&location, id, &manifest).await?;
            }
        }

        let now = Utc::now();
        state.summary.status = SubmissionStatus::Aborted;
        state.summary.updated_at = now;
        state.summary.completed_at = Some(now);
        state.abort_reason = Some(reason.to_string());

        self.save_submission_state(&location, id, &state).await?;
        Ok(pending_count)
    }

    async fn add_manifest(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_url: Option<&str>,
        replaces_manifest_url: Option<&str>,
    ) -> StorageResult<SubmissionManifest> {
        let location = self.tenant_location(tenant)?;
        let mut submission = self.load_submission_state(&location, submission_id).await?;

        match submission.summary.status {
            SubmissionStatus::InProgress => {}
            SubmissionStatus::Complete => {
                return Err(StorageError::BulkSubmit(BulkSubmitError::InvalidState {
                    submission_id: submission_id.submission_id.clone(),
                    expected: "in-progress".to_string(),
                    actual: "complete".to_string(),
                }));
            }
            SubmissionStatus::Aborted => {
                return Err(StorageError::BulkSubmit(BulkSubmitError::Aborted {
                    submission_id: submission_id.submission_id.clone(),
                    reason: submission
                        .abort_reason
                        .clone()
                        .unwrap_or_else(|| "aborted".to_string()),
                }));
            }
        }

        let manifest_id = Uuid::new_v4().to_string();
        let mut manifest = SubmissionManifest::new(manifest_id);
        if let Some(manifest_url) = manifest_url {
            manifest = manifest.with_url(manifest_url);
        }
        if let Some(replaces_manifest_url) = replaces_manifest_url {
            manifest = manifest.with_replaces(replaces_manifest_url);
        }

        self.save_manifest_state(
            &location,
            submission_id,
            &SubmissionManifestState {
                manifest: manifest.clone(),
            },
        )
        .await?;

        submission.summary.manifest_count += 1;
        submission.summary.updated_at = Utc::now();
        self.save_submission_state(&location, submission_id, &submission)
            .await?;

        Ok(manifest)
    }

    async fn get_manifest(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<Option<SubmissionManifest>> {
        let location = self.tenant_location(tenant)?;
        Ok(self
            .load_manifest_state_optional(&location, submission_id, manifest_id)
            .await?
            .map(|state| state.manifest))
    }

    async fn list_manifests(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
    ) -> StorageResult<Vec<SubmissionManifest>> {
        let location = self.tenant_location(tenant)?;
        let mut manifests = self
            .list_manifest_states(&location, submission_id)
            .await?
            .into_iter()
            .map(|state| state.manifest)
            .collect::<Vec<_>>();

        manifests.sort_by_key(|m| m.added_at);
        Ok(manifests)
    }

    async fn process_entries(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entries: Vec<NdjsonEntry>,
        options: &BulkProcessingOptions,
    ) -> StorageResult<Vec<BulkEntryResult>> {
        let location = self.tenant_location(tenant)?;
        let mut submission = self.load_submission_state(&location, submission_id).await?;

        match submission.summary.status {
            SubmissionStatus::InProgress => {}
            SubmissionStatus::Complete => {
                return Err(StorageError::BulkSubmit(BulkSubmitError::AlreadyComplete {
                    submission_id: submission_id.submission_id.clone(),
                }));
            }
            SubmissionStatus::Aborted => {
                return Err(StorageError::BulkSubmit(BulkSubmitError::Aborted {
                    submission_id: submission_id.submission_id.clone(),
                    reason: submission
                        .abort_reason
                        .clone()
                        .unwrap_or_else(|| "aborted".to_string()),
                }));
            }
        }

        let mut manifest_state = self
            .load_manifest_state_optional(&location, submission_id, manifest_id)
            .await?
            .ok_or_else(|| {
                StorageError::BulkSubmit(BulkSubmitError::ManifestNotFound {
                    submission_id: submission_id.submission_id.clone(),
                    manifest_id: manifest_id.to_string(),
                })
            })?;

        manifest_state.manifest.status = ManifestStatus::Processing;
        self.save_manifest_state(&location, submission_id, &manifest_state)
            .await?;

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
                self.persist_entry_result(&location, submission_id, manifest_id, &skipped)
                    .await?;
                results.push(skipped);
                continue;
            }

            self.persist_raw_entry(&location, submission_id, manifest_id, &entry)
                .await?;

            let result = match self
                .process_single_entry(tenant, submission_id, manifest_id, &entry, options)
                .await
            {
                Ok(result) => result,
                Err(err) => BulkEntryResult::processing_error(
                    entry.line_number,
                    &entry.resource_type,
                    Self::bulk_submit_operation_outcome(&err),
                ),
            };

            if result.is_error() {
                error_count += 1;
            }

            self.persist_entry_result(&location, submission_id, manifest_id, &result)
                .await?;
            results.push(result);
        }

        let success_count = results.iter().filter(|r| r.is_success()).count() as u64;
        let failed_count = results.iter().filter(|r| r.is_error()).count() as u64;
        let skipped_count = results
            .iter()
            .filter(|r| r.outcome == BulkEntryOutcome::Skipped)
            .count() as u64;

        manifest_state.manifest.total_entries += results.len() as u64;
        manifest_state.manifest.processed_entries += results.len() as u64;
        manifest_state.manifest.failed_entries += failed_count;
        manifest_state.manifest.status = if failed_count > 0 {
            ManifestStatus::Failed
        } else {
            ManifestStatus::Completed
        };

        self.save_manifest_state(&location, submission_id, &manifest_state)
            .await?;

        submission.summary.total_entries += results.len() as u64;
        submission.summary.success_count += success_count;
        submission.summary.error_count += failed_count;
        submission.summary.skipped_count += skipped_count;
        submission.summary.updated_at = Utc::now();
        self.save_submission_state(&location, submission_id, &submission)
            .await?;

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
        let location = self.tenant_location(tenant)?;
        let mut results = self
            .load_entry_results(&location, submission_id, manifest_id)
            .await?;

        if let Some(filter) = outcome_filter {
            results.retain(|r| r.outcome == filter);
        }

        results.sort_by_key(|r| r.line_number);

        let start = (offset as usize).min(results.len());
        let end = start.saturating_add(limit as usize).min(results.len());
        Ok(results[start..end].to_vec())
    }

    async fn get_entry_counts(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<EntryCountSummary> {
        let location = self.tenant_location(tenant)?;
        let mut summary = EntryCountSummary::new();

        for result in self
            .load_entry_results(&location, submission_id, manifest_id)
            .await?
        {
            summary.increment(result.outcome);
        }

        Ok(summary)
    }
}

#[async_trait]
impl StreamingBulkSubmitProvider for S3Backend {
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
impl BulkSubmitRollbackProvider for S3Backend {
    async fn record_change(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        change: &SubmissionChange,
    ) -> StorageResult<()> {
        let location = self.tenant_location(tenant)?;
        let key = location.keyspace.submit_change_key(
            &submission_id.submitter,
            &submission_id.submission_id,
            &change.change_id,
        );

        let payload = self.serialize_json(change)?;
        self.put_json_object(&location.bucket, &key, &payload, None, None)
            .await?;
        Ok(())
    }

    async fn list_changes(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<SubmissionChange>> {
        let location = self.tenant_location(tenant)?;
        let mut changes = self.load_changes(&location, submission_id).await?;
        changes.sort_by_key(|c| std::cmp::Reverse(c.changed_at));

        let start = (offset as usize).min(changes.len());
        let end = start.saturating_add(limit as usize).min(changes.len());
        Ok(changes[start..end].to_vec())
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
                Err(err) => Err(err),
            },
            ChangeType::Update => {
                if let Some(previous_version) = change.previous_version.as_deref() {
                    if let Some(snapshot) = self
                        .vread(
                            tenant,
                            &change.resource_type,
                            &change.resource_id,
                            previous_version,
                        )
                        .await?
                    {
                        self.restore_resource_from_snapshot(tenant, &snapshot)
                            .await?;
                        return Ok(true);
                    }
                }

                if let Some(previous_content) = &change.previous_content {
                    if let Some(current) = self
                        .read(tenant, &change.resource_type, &change.resource_id)
                        .await?
                    {
                        self.update(tenant, &current, previous_content.clone())
                            .await?;
                        return Ok(true);
                    }
                }

                Ok(false)
            }
        }
    }
}

impl S3Backend {
    /// Processes a single NDJSON entry: validates it, upserts the resource,
    /// and records a change log entry for rollback.
    ///
    /// Returns a `BulkEntryResult` describing the outcome. Storage errors are
    /// promoted to entry-level processing errors rather than aborting the whole
    /// batch.
    async fn process_single_entry(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entry: &NdjsonEntry,
        options: &BulkProcessingOptions,
    ) -> StorageResult<BulkEntryResult> {
        if let Some(resource_type) = entry.resource.get("resourceType").and_then(|v| v.as_str()) {
            if resource_type != entry.resource_type {
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
        }

        if let Some(id) = entry.resource_id.as_deref() {
            match self.read(tenant, &entry.resource_type, id).await {
                Ok(Some(current)) => {
                    if !options.allow_updates {
                        return Ok(BulkEntryResult::skipped(
                            entry.line_number,
                            &entry.resource_type,
                            "updates not allowed",
                        ));
                    }

                    let updated = self
                        .update(tenant, &current, entry.resource.clone())
                        .await?;

                    let change = SubmissionChange::update(
                        manifest_id,
                        &entry.resource_type,
                        updated.id(),
                        current.version_id(),
                        updated.version_id(),
                        current.content().clone(),
                    );
                    self.record_change(tenant, submission_id, &change).await?;

                    Ok(BulkEntryResult::success(
                        entry.line_number,
                        &entry.resource_type,
                        updated.id(),
                        false,
                    ))
                }
                Ok(None) | Err(StorageError::Resource(ResourceError::Gone { .. })) => {
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
                Err(err) => Err(err),
            }
        } else {
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
    }

    /// Archives the raw NDJSON payload for a single entry to S3.
    ///
    /// Stored under `raw/<manifest>/<line>.ndjson` so that the original data
    /// is preserved for auditing after ingestion.
    async fn persist_raw_entry(
        &self,
        location: &TenantLocation,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entry: &NdjsonEntry,
    ) -> StorageResult<()> {
        let key = location.keyspace.submit_raw_line_key(
            &submission_id.submitter,
            &submission_id.submission_id,
            manifest_id,
            entry.line_number,
        );

        let mut line = serde_json::to_string(&entry.resource).map_err(|e| {
            StorageError::BulkSubmit(BulkSubmitError::ParseError {
                line: entry.line_number,
                message: format!("failed to serialize raw NDJSON entry: {e}"),
            })
        })?;
        line.push('\n');

        self.put_bytes_object(
            &location.bucket,
            &key,
            line.as_bytes(),
            Some("application/fhir+ndjson"),
        )
        .await?;

        Ok(())
    }

    /// Persists the processing result for a single entry to S3.
    async fn persist_entry_result(
        &self,
        location: &TenantLocation,
        submission_id: &SubmissionId,
        manifest_id: &str,
        result: &BulkEntryResult,
    ) -> StorageResult<()> {
        let key = location.keyspace.submit_result_line_key(
            &submission_id.submitter,
            &submission_id.submission_id,
            manifest_id,
            result.line_number,
        );
        let payload = self.serialize_json(result)?;
        self.put_json_object(&location.bucket, &key, &payload, None, None)
            .await?;
        Ok(())
    }

    /// Loads all entry results for a manifest from S3.
    async fn load_entry_results(
        &self,
        location: &TenantLocation,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<Vec<BulkEntryResult>> {
        let prefix = format!(
            "{}results/{}/",
            location
                .keyspace
                .submit_prefix(&submission_id.submitter, &submission_id.submission_id),
            manifest_id
        );

        let mut results = Vec::new();
        for object in self.list_objects_all(&location.bucket, &prefix).await? {
            if !object.key.ends_with(".json") {
                continue;
            }

            if let Some((result, _)) = self
                .get_json_object::<BulkEntryResult>(&location.bucket, &object.key)
                .await?
            {
                results.push(result);
            }
        }

        Ok(results)
    }

    /// Loads all change log records for a submission from S3.
    async fn load_changes(
        &self,
        location: &TenantLocation,
        submission_id: &SubmissionId,
    ) -> StorageResult<Vec<SubmissionChange>> {
        let prefix = format!(
            "{}changes/",
            location
                .keyspace
                .submit_prefix(&submission_id.submitter, &submission_id.submission_id)
        );

        let mut changes = Vec::new();
        for object in self.list_objects_all(&location.bucket, &prefix).await? {
            if !object.key.ends_with(".json") {
                continue;
            }

            if let Some((change, _)) = self
                .get_json_object::<SubmissionChange>(&location.bucket, &object.key)
                .await?
            {
                changes.push(change);
            }
        }

        Ok(changes)
    }

    /// Loads the submission state, returning `SubmissionNotFound` if absent.
    async fn load_submission_state(
        &self,
        location: &TenantLocation,
        id: &SubmissionId,
    ) -> StorageResult<SubmissionState> {
        self.load_submission_state_optional(location, id)
            .await?
            .ok_or_else(|| {
                StorageError::BulkSubmit(BulkSubmitError::SubmissionNotFound {
                    submitter: id.submitter.clone(),
                    submission_id: id.submission_id.clone(),
                })
            })
    }

    /// Loads the submission state, returning `None` if it does not exist.
    async fn load_submission_state_optional(
        &self,
        location: &TenantLocation,
        id: &SubmissionId,
    ) -> StorageResult<Option<SubmissionState>> {
        let key = location
            .keyspace
            .submit_state_key(&id.submitter, &id.submission_id);
        Ok(self
            .get_json_object::<SubmissionState>(&location.bucket, &key)
            .await?
            .map(|(state, _)| state))
    }

    /// Serialises and writes the submission state to S3.
    async fn save_submission_state(
        &self,
        location: &TenantLocation,
        id: &SubmissionId,
        state: &SubmissionState,
    ) -> StorageResult<()> {
        let key = location
            .keyspace
            .submit_state_key(&id.submitter, &id.submission_id);
        let payload = self.serialize_json(state)?;
        self.put_json_object(&location.bucket, &key, &payload, None, None)
            .await?;
        Ok(())
    }

    /// Loads a manifest state from S3, returning `None` if it does not exist.
    async fn load_manifest_state_optional(
        &self,
        location: &TenantLocation,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<Option<SubmissionManifestState>> {
        let key = location.keyspace.submit_manifest_key(
            &submission_id.submitter,
            &submission_id.submission_id,
            manifest_id,
        );

        Ok(self
            .get_json_object::<SubmissionManifestState>(&location.bucket, &key)
            .await?
            .map(|(state, _)| state))
    }

    /// Serialises and writes a manifest state to S3.
    async fn save_manifest_state(
        &self,
        location: &TenantLocation,
        submission_id: &SubmissionId,
        state: &SubmissionManifestState,
    ) -> StorageResult<()> {
        let key = location.keyspace.submit_manifest_key(
            &submission_id.submitter,
            &submission_id.submission_id,
            &state.manifest.manifest_id,
        );

        let payload = self.serialize_json(state)?;
        self.put_json_object(&location.bucket, &key, &payload, None, None)
            .await?;
        Ok(())
    }

    /// Lists all manifest state objects for a submission.
    async fn list_manifest_states(
        &self,
        location: &TenantLocation,
        submission_id: &SubmissionId,
    ) -> StorageResult<Vec<SubmissionManifestState>> {
        let prefix = format!(
            "{}manifests/",
            location
                .keyspace
                .submit_prefix(&submission_id.submitter, &submission_id.submission_id)
        );

        let mut manifests = Vec::new();
        for object in self.list_objects_all(&location.bucket, &prefix).await? {
            if !object.key.ends_with(".json") {
                continue;
            }

            if let Some((state, _)) = self
                .get_json_object::<SubmissionManifestState>(&location.bucket, &object.key)
                .await?
            {
                manifests.push(state);
            }
        }

        Ok(manifests)
    }

    /// Builds a minimal OperationOutcome from a storage error for use in
    /// per-entry failure records.
    fn bulk_submit_operation_outcome(err: &StorageError) -> serde_json::Value {
        let code = match err {
            StorageError::Validation(_) => "invalid",
            StorageError::Tenant(_) => "forbidden",
            StorageError::Resource(ResourceError::NotFound { .. }) => "not-found",
            StorageError::Resource(ResourceError::Gone { .. }) => "deleted",
            StorageError::Resource(ResourceError::AlreadyExists { .. }) => "conflict",
            StorageError::Concurrency(_) => "conflict",
            _ => "exception",
        };

        serde_json::json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": code,
                "diagnostics": err.to_string()
            }]
        })
    }
}
