//! Bulk submit types and traits.
//!
//! This module provides types and traits for implementing Bulk Data Submit
//! as specified in the [Bulk Submit](https://hackmd.io/@argonaut/rJoqHZrPle) specification.
//!
//! # Overview
//!
//! Bulk Submit allows clients to submit large amounts of FHIR resources in NDJSON format.
//! The submission process involves:
//!
//! 1. Creating a submission
//! 2. Adding manifests (files to process)
//! 3. Processing entries from each manifest
//! 4. Completing or aborting the submission
//!
//! # Rollback Support
//!
//! Submissions track changes for potential rollback. When a submission is aborted,
//! created resources are deleted and updated resources are reverted to their
//! previous state.
//!
//! # Example
//!
//! ```ignore
//! use helios_persistence::core::bulk_submit::{
//!     BulkSubmitProvider, SubmissionId, NdjsonEntry, BulkProcessingOptions,
//! };
//!
//! async fn submit_patients<S: BulkSubmitProvider>(storage: &S, tenant: &TenantContext) {
//!     // Create a submission
//!     let sub_id = SubmissionId::generate("my-system");
//!     let summary = storage.create_submission(tenant, &sub_id, None).await.unwrap();
//!
//!     // Add a manifest
//!     let manifest = storage.add_manifest(tenant, &sub_id, None, None).await.unwrap();
//!
//!     // Process entries
//!     let entries = vec![
//!         NdjsonEntry::new(1, "Patient", serde_json::json!({"resourceType": "Patient"})),
//!     ];
//!     let results = storage.process_entries(
//!         tenant, &sub_id, &manifest.manifest_id, entries, &BulkProcessingOptions::new()
//!     ).await.unwrap();
//!
//!     // Complete the submission
//!     storage.complete_submission(tenant, &sub_id).await.unwrap();
//! }
//! ```

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncBufRead;
use uuid::Uuid;

use crate::core::storage::ResourceStorage;
use crate::error::StorageResult;
use crate::tenant::TenantContext;

/// Audit event helpers for bulk submit operations.
#[cfg(feature = "audit")]
pub mod audit {
    use helios_audit::{AuditAction, AuditEventBuilder, AuditSink};

    use super::SubmissionId;

    /// Record an audit event for a bulk submit lifecycle event.
    ///
    /// Call this at submission start, completion, abort, or rollback.
    pub async fn record_submit_event(
        sink: &dyn AuditSink,
        source_observer: &str,
        agent: Option<&str>,
        submission_id: &SubmissionId,
        phase: &str,
        outcome: &str,
        outcome_desc: Option<&str>,
    ) {
        let mut builder = AuditEventBuilder::new(source_observer)
            .event_type(
                "http://terminology.hl7.org/CodeSystem/audit-event-type",
                "object",
            )
            .action(AuditAction::Execute)
            .outcome(outcome)
            .detail("audit-operation", "bulk-import")
            .detail("submission-id", &submission_id.submission_id)
            .detail("submitter", &submission_id.submitter)
            .detail("phase", phase);
        if let Some(a) = agent {
            builder = builder.agent(a, None, true);
        }
        if let Some(d) = outcome_desc {
            builder = builder.outcome_desc(d);
        }
        sink.record(builder.build()).await;
    }

    #[cfg(test)]
    mod tests {
        use helios_audit::sinks::NullSink;

        use super::*;
        use crate::core::bulk_submit::SubmissionId;

        #[tokio::test]
        async fn test_submit_event_includes_phase() {
            let sink = NullSink;
            let id = SubmissionId::new("client-a", "sub-123");
            record_submit_event(&sink, "Device/hfs", None, &id, "start", "0", None).await;
        }

        #[test]
        fn test_submit_event_has_details() {
            let id = SubmissionId::new("client-a", "sub-123");
            let event = AuditEventBuilder::new("Device/hfs")
                .event_type(
                    "http://terminology.hl7.org/CodeSystem/audit-event-type",
                    "object",
                )
                .action(AuditAction::Execute)
                .outcome("0")
                .detail("audit-operation", "bulk-import")
                .detail("submission-id", &id.submission_id)
                .detail("submitter", &id.submitter)
                .detail("phase", "complete")
                .build();
            let entities = event.entity.as_ref().unwrap();
            let details = entities[0].detail.as_ref().unwrap();
            assert_eq!(details.len(), 4);
            assert_eq!(details[3].r#type.value.as_deref(), Some("phase"));
        }
    }
}

/// Unique identifier for a bulk submission.
///
/// A submission is identified by the combination of a submitter identifier
/// (typically the client system) and a submission ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubmissionId {
    /// The submitter identifier (e.g., client system name).
    pub submitter: String,
    /// The submission identifier.
    pub submission_id: String,
}

impl SubmissionId {
    /// Creates a new submission ID.
    pub fn new(submitter: impl Into<String>, submission_id: impl Into<String>) -> Self {
        Self {
            submitter: submitter.into(),
            submission_id: submission_id.into(),
        }
    }

    /// Generates a new submission ID with a random UUID.
    pub fn generate(submitter: impl Into<String>) -> Self {
        Self {
            submitter: submitter.into(),
            submission_id: Uuid::new_v4().to_string(),
        }
    }
}

impl std::fmt::Display for SubmissionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.submitter, self.submission_id)
    }
}

/// Status of a bulk submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubmissionStatus {
    /// Submission is in progress.
    InProgress,
    /// Submission has completed successfully.
    Complete,
    /// Submission was aborted.
    Aborted,
}

impl SubmissionStatus {
    /// Returns true if the submission is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Complete | Self::Aborted)
    }
}

impl std::fmt::Display for SubmissionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InProgress => write!(f, "in-progress"),
            Self::Complete => write!(f, "complete"),
            Self::Aborted => write!(f, "aborted"),
        }
    }
}

impl std::str::FromStr for SubmissionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "in-progress" | "in_progress" => Ok(Self::InProgress),
            "complete" => Ok(Self::Complete),
            "aborted" => Ok(Self::Aborted),
            _ => Err(format!("unknown submission status: {}", s)),
        }
    }
}

/// Status of a manifest within a submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManifestStatus {
    /// Manifest has been added but not yet processed.
    Pending,
    /// Manifest is currently being processed.
    Processing,
    /// Manifest has been fully processed.
    Completed,
    /// Manifest processing failed.
    Failed,
}

impl ManifestStatus {
    /// Returns true if the manifest is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

impl std::fmt::Display for ManifestStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Processing => write!(f, "processing"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

impl std::str::FromStr for ManifestStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "processing" => Ok(Self::Processing),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            _ => Err(format!("unknown manifest status: {}", s)),
        }
    }
}

/// A manifest within a submission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionManifest {
    /// Unique manifest ID within the submission.
    pub manifest_id: String,
    /// Optional URL where the manifest data was fetched from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_url: Option<String>,
    /// URL of the manifest this replaces, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces_manifest_url: Option<String>,
    /// Current processing status.
    pub status: ManifestStatus,
    /// When the manifest was added.
    pub added_at: DateTime<Utc>,
    /// Total number of entries in the manifest.
    pub total_entries: u64,
    /// Number of entries processed so far.
    pub processed_entries: u64,
    /// Number of entries that failed processing.
    pub failed_entries: u64,
}

impl SubmissionManifest {
    /// Creates a new manifest.
    pub fn new(manifest_id: impl Into<String>) -> Self {
        Self {
            manifest_id: manifest_id.into(),
            manifest_url: None,
            replaces_manifest_url: None,
            status: ManifestStatus::Pending,
            added_at: Utc::now(),
            total_entries: 0,
            processed_entries: 0,
            failed_entries: 0,
        }
    }

    /// Sets the manifest URL.
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.manifest_url = Some(url.into());
        self
    }

    /// Sets the replaces URL.
    pub fn with_replaces(mut self, url: impl Into<String>) -> Self {
        self.replaces_manifest_url = Some(url.into());
        self
    }
}

/// Outcome of processing a single entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BulkEntryOutcome {
    /// Entry was processed successfully.
    Success,
    /// Entry failed validation.
    ValidationError,
    /// Entry encountered a processing error.
    ProcessingError,
    /// Entry was skipped (e.g., duplicate).
    Skipped,
}

impl std::fmt::Display for BulkEntryOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success => write!(f, "success"),
            Self::ValidationError => write!(f, "validation-error"),
            Self::ProcessingError => write!(f, "processing-error"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

impl std::str::FromStr for BulkEntryOutcome {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "success" => Ok(Self::Success),
            "validation-error" | "validation_error" => Ok(Self::ValidationError),
            "processing-error" | "processing_error" => Ok(Self::ProcessingError),
            "skipped" => Ok(Self::Skipped),
            _ => Err(format!("unknown entry outcome: {}", s)),
        }
    }
}

/// Result of processing a single NDJSON entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkEntryResult {
    /// Line number in the NDJSON file (1-indexed).
    pub line_number: u64,
    /// Resource type of the entry.
    pub resource_type: String,
    /// Resource ID if successfully processed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    /// Whether a new resource was created (vs updated).
    pub created: bool,
    /// Processing outcome.
    pub outcome: BulkEntryOutcome,
    /// OperationOutcome if there was an error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_outcome: Option<Value>,
}

impl BulkEntryResult {
    /// Creates a success result.
    pub fn success(
        line_number: u64,
        resource_type: impl Into<String>,
        resource_id: impl Into<String>,
        created: bool,
    ) -> Self {
        Self {
            line_number,
            resource_type: resource_type.into(),
            resource_id: Some(resource_id.into()),
            created,
            outcome: BulkEntryOutcome::Success,
            operation_outcome: None,
        }
    }

    /// Creates a validation error result.
    pub fn validation_error(
        line_number: u64,
        resource_type: impl Into<String>,
        outcome: Value,
    ) -> Self {
        Self {
            line_number,
            resource_type: resource_type.into(),
            resource_id: None,
            created: false,
            outcome: BulkEntryOutcome::ValidationError,
            operation_outcome: Some(outcome),
        }
    }

    /// Creates a processing error result.
    pub fn processing_error(
        line_number: u64,
        resource_type: impl Into<String>,
        outcome: Value,
    ) -> Self {
        Self {
            line_number,
            resource_type: resource_type.into(),
            resource_id: None,
            created: false,
            outcome: BulkEntryOutcome::ProcessingError,
            operation_outcome: Some(outcome),
        }
    }

    /// Creates a skipped result.
    pub fn skipped(line_number: u64, resource_type: impl Into<String>, reason: &str) -> Self {
        Self {
            line_number,
            resource_type: resource_type.into(),
            resource_id: None,
            created: false,
            outcome: BulkEntryOutcome::Skipped,
            operation_outcome: Some(serde_json::json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "information",
                    "code": "informational",
                    "diagnostics": reason
                }]
            })),
        }
    }

    /// Returns true if this was a successful outcome.
    pub fn is_success(&self) -> bool {
        self.outcome == BulkEntryOutcome::Success
    }

    /// Returns true if this was an error outcome.
    pub fn is_error(&self) -> bool {
        matches!(
            self.outcome,
            BulkEntryOutcome::ValidationError | BulkEntryOutcome::ProcessingError
        )
    }
}

/// Summary of a submission's status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionSummary {
    /// The submission ID.
    pub id: SubmissionId,
    /// Current status.
    pub status: SubmissionStatus,
    /// When the submission was created.
    pub created_at: DateTime<Utc>,
    /// When the submission was last updated.
    pub updated_at: DateTime<Utc>,
    /// When the submission completed (if terminal).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Number of manifests in the submission.
    pub manifest_count: u32,
    /// Total entries across all manifests.
    pub total_entries: u64,
    /// Successfully processed entries.
    pub success_count: u64,
    /// Failed entries.
    pub error_count: u64,
    /// Skipped entries.
    pub skipped_count: u64,
    /// Optional metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl SubmissionSummary {
    /// Creates a new submission summary.
    pub fn new(id: SubmissionId) -> Self {
        let now = Utc::now();
        Self {
            id,
            status: SubmissionStatus::InProgress,
            created_at: now,
            updated_at: now,
            completed_at: None,
            manifest_count: 0,
            total_entries: 0,
            success_count: 0,
            error_count: 0,
            skipped_count: 0,
            metadata: None,
        }
    }

    /// Sets the metadata.
    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// A parsed NDJSON entry ready for processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NdjsonEntry {
    /// Line number in the source file (1-indexed).
    pub line_number: u64,
    /// The resource type.
    pub resource_type: String,
    /// The resource ID (if present in the resource).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    /// The resource content.
    pub resource: Value,
}

impl NdjsonEntry {
    /// Creates a new NDJSON entry.
    pub fn new(line_number: u64, resource_type: impl Into<String>, resource: Value) -> Self {
        let resource_type = resource_type.into();
        let resource_id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(String::from);
        Self {
            line_number,
            resource_type,
            resource_id,
            resource,
        }
    }

    /// Parses an NDJSON line into an entry.
    pub fn parse(line_number: u64, line: &str) -> Result<Self, String> {
        let resource: Value =
            serde_json::from_str(line).map_err(|e| format!("invalid JSON: {}", e))?;

        let resource_type = resource
            .get("resourceType")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing resourceType".to_string())?
            .to_string();

        Ok(Self::new(line_number, resource_type, resource))
    }
}

/// Options for bulk processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkProcessingOptions {
    /// Number of entries to process in a single batch/transaction.
    #[serde(default = "default_submit_batch_size")]
    pub batch_size: u32,
    /// Whether to continue processing after encountering errors.
    #[serde(default = "default_continue_on_error")]
    pub continue_on_error: bool,
    /// Maximum number of errors before aborting (0 = unlimited).
    #[serde(default)]
    pub max_errors: u32,
    /// Whether to allow updates to existing resources.
    #[serde(default = "default_allow_updates")]
    pub allow_updates: bool,
}

fn default_submit_batch_size() -> u32 {
    100
}

fn default_continue_on_error() -> bool {
    true
}

fn default_allow_updates() -> bool {
    true
}

impl Default for BulkProcessingOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl BulkProcessingOptions {
    /// Creates default processing options.
    pub fn new() -> Self {
        Self {
            batch_size: default_submit_batch_size(),
            continue_on_error: default_continue_on_error(),
            max_errors: 0,
            allow_updates: default_allow_updates(),
        }
    }

    /// Sets the batch size.
    pub fn with_batch_size(mut self, batch_size: u32) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Sets whether to continue on error.
    pub fn with_continue_on_error(mut self, continue_on_error: bool) -> Self {
        self.continue_on_error = continue_on_error;
        self
    }

    /// Sets the maximum number of errors.
    pub fn with_max_errors(mut self, max_errors: u32) -> Self {
        self.max_errors = max_errors;
        self
    }

    /// Sets whether updates are allowed.
    pub fn with_allow_updates(mut self, allow_updates: bool) -> Self {
        self.allow_updates = allow_updates;
        self
    }

    /// Creates options for strict processing (no errors allowed).
    pub fn strict() -> Self {
        Self {
            batch_size: default_submit_batch_size(),
            continue_on_error: false,
            max_errors: 1,
            allow_updates: true,
        }
    }

    /// Creates options for create-only processing.
    pub fn create_only() -> Self {
        Self {
            batch_size: default_submit_batch_size(),
            continue_on_error: true,
            max_errors: 0,
            allow_updates: false,
        }
    }
}

/// Type of change recorded for rollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeType {
    /// A new resource was created.
    Create,
    /// An existing resource was updated.
    Update,
}

impl std::fmt::Display for ChangeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Create => write!(f, "create"),
            Self::Update => write!(f, "update"),
        }
    }
}

impl std::str::FromStr for ChangeType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "create" => Ok(Self::Create),
            "update" => Ok(Self::Update),
            _ => Err(format!("unknown change type: {}", s)),
        }
    }
}

/// A change record for potential rollback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionChange {
    /// Unique change ID.
    pub change_id: String,
    /// Manifest this change is associated with.
    pub manifest_id: String,
    /// Type of change.
    pub change_type: ChangeType,
    /// Resource type.
    pub resource_type: String,
    /// Resource ID.
    pub resource_id: String,
    /// Version before the change (for updates).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<String>,
    /// Version after the change.
    pub new_version: String,
    /// Previous resource content (for updates).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_content: Option<Value>,
    /// When the change was made.
    pub changed_at: DateTime<Utc>,
}

impl SubmissionChange {
    /// Creates a change record for a create operation.
    pub fn create(
        manifest_id: impl Into<String>,
        resource_type: impl Into<String>,
        resource_id: impl Into<String>,
        new_version: impl Into<String>,
    ) -> Self {
        Self {
            change_id: Uuid::new_v4().to_string(),
            manifest_id: manifest_id.into(),
            change_type: ChangeType::Create,
            resource_type: resource_type.into(),
            resource_id: resource_id.into(),
            previous_version: None,
            new_version: new_version.into(),
            previous_content: None,
            changed_at: Utc::now(),
        }
    }

    /// Creates a change record for an update operation.
    pub fn update(
        manifest_id: impl Into<String>,
        resource_type: impl Into<String>,
        resource_id: impl Into<String>,
        previous_version: impl Into<String>,
        new_version: impl Into<String>,
        previous_content: Value,
    ) -> Self {
        Self {
            change_id: Uuid::new_v4().to_string(),
            manifest_id: manifest_id.into(),
            change_type: ChangeType::Update,
            resource_type: resource_type.into(),
            resource_id: resource_id.into(),
            previous_version: Some(previous_version.into()),
            new_version: new_version.into(),
            previous_content: Some(previous_content),
            changed_at: Utc::now(),
        }
    }
}

/// Summary of entry counts by outcome.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntryCountSummary {
    /// Total entries.
    pub total: u64,
    /// Successful entries.
    pub success: u64,
    /// Validation errors.
    pub validation_error: u64,
    /// Processing errors.
    pub processing_error: u64,
    /// Skipped entries.
    pub skipped: u64,
}

impl EntryCountSummary {
    /// Creates an empty summary.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the total number of errors.
    pub fn error_count(&self) -> u64 {
        self.validation_error + self.processing_error
    }

    /// Increments the count for an outcome.
    pub fn increment(&mut self, outcome: BulkEntryOutcome) {
        self.total += 1;
        match outcome {
            BulkEntryOutcome::Success => self.success += 1,
            BulkEntryOutcome::ValidationError => self.validation_error += 1,
            BulkEntryOutcome::ProcessingError => self.processing_error += 1,
            BulkEntryOutcome::Skipped => self.skipped += 1,
        }
    }
}

/// Result of streaming NDJSON processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamProcessingResult {
    /// Number of lines processed.
    pub lines_processed: u64,
    /// Entry count summary.
    pub counts: EntryCountSummary,
    /// Whether processing was aborted early.
    pub aborted: bool,
    /// Abort reason if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abort_reason: Option<String>,
}

impl StreamProcessingResult {
    /// Creates a new stream processing result.
    pub fn new() -> Self {
        Self {
            lines_processed: 0,
            counts: EntryCountSummary::new(),
            aborted: false,
            abort_reason: None,
        }
    }

    /// Marks the result as aborted.
    pub fn aborted(mut self, reason: impl Into<String>) -> Self {
        self.aborted = true;
        self.abort_reason = Some(reason.into());
        self
    }
}

impl Default for StreamProcessingResult {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Traits
// ============================================================================

/// Provider for bulk submit operations.
///
/// This trait handles the complete lifecycle of bulk submissions including
/// creating submissions, adding manifests, processing entries, and completing
/// or aborting submissions.
#[async_trait]
pub trait BulkSubmitProvider: ResourceStorage {
    /// Creates a new submission.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `id` - The submission identifier
    /// * `metadata` - Optional metadata to attach to the submission
    ///
    /// # Returns
    ///
    /// The submission summary.
    ///
    /// # Errors
    ///
    /// * `BulkSubmitError::DuplicateSubmission` - If a submission with this ID exists
    async fn create_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        metadata: Option<Value>,
    ) -> StorageResult<SubmissionSummary>;

    /// Gets a submission by ID.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `id` - The submission identifier
    ///
    /// # Returns
    ///
    /// The submission summary if found.
    async fn get_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Option<SubmissionSummary>>;

    /// Lists submissions.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `submitter` - Optional filter by submitter
    /// * `status` - Optional filter by status
    /// * `limit` - Maximum number of results
    /// * `offset` - Offset for pagination
    ///
    /// # Returns
    ///
    /// List of submission summaries.
    async fn list_submissions(
        &self,
        tenant: &TenantContext,
        submitter: Option<&str>,
        status: Option<SubmissionStatus>,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<SubmissionSummary>>;

    /// Marks a submission as complete.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `id` - The submission identifier
    ///
    /// # Returns
    ///
    /// The updated submission summary.
    ///
    /// # Errors
    ///
    /// * `BulkSubmitError::SubmissionNotFound` - If the submission doesn't exist
    /// * `BulkSubmitError::AlreadyComplete` - If already completed
    async fn complete_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<SubmissionSummary>;

    /// Aborts a submission.
    ///
    /// This does NOT automatically roll back changes - use `BulkSubmitRollbackProvider`
    /// for that functionality.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `id` - The submission identifier
    /// * `reason` - Reason for aborting
    ///
    /// # Returns
    ///
    /// The number of pending manifests that were cancelled.
    ///
    /// # Errors
    ///
    /// * `BulkSubmitError::SubmissionNotFound` - If the submission doesn't exist
    /// * `BulkSubmitError::AlreadyComplete` - If already completed
    async fn abort_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        reason: &str,
    ) -> StorageResult<u64>;

    /// Adds a manifest to a submission.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `submission_id` - The submission identifier
    /// * `manifest_url` - Optional URL where the manifest data came from
    /// * `replaces_manifest_url` - Optional URL of manifest this replaces
    ///
    /// # Returns
    ///
    /// The created manifest.
    ///
    /// # Errors
    ///
    /// * `BulkSubmitError::SubmissionNotFound` - If the submission doesn't exist
    /// * `BulkSubmitError::InvalidState` - If the submission is not in progress
    async fn add_manifest(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_url: Option<&str>,
        replaces_manifest_url: Option<&str>,
    ) -> StorageResult<SubmissionManifest>;

    /// Gets a manifest by ID.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `submission_id` - The submission identifier
    /// * `manifest_id` - The manifest identifier
    ///
    /// # Returns
    ///
    /// The manifest if found.
    async fn get_manifest(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<Option<SubmissionManifest>>;

    /// Lists manifests in a submission.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `submission_id` - The submission identifier
    ///
    /// # Returns
    ///
    /// List of manifests.
    async fn list_manifests(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
    ) -> StorageResult<Vec<SubmissionManifest>>;

    /// Processes entries from a manifest.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `submission_id` - The submission identifier
    /// * `manifest_id` - The manifest identifier
    /// * `entries` - The entries to process
    /// * `options` - Processing options
    ///
    /// # Returns
    ///
    /// Results for each entry.
    ///
    /// # Errors
    ///
    /// * `BulkSubmitError::SubmissionNotFound` - If the submission doesn't exist
    /// * `BulkSubmitError::ManifestNotFound` - If the manifest doesn't exist
    /// * `BulkSubmitError::MaxErrorsExceeded` - If max errors was reached
    async fn process_entries(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entries: Vec<NdjsonEntry>,
        options: &BulkProcessingOptions,
    ) -> StorageResult<Vec<BulkEntryResult>>;

    /// Gets entry results for a manifest.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `submission_id` - The submission identifier
    /// * `manifest_id` - The manifest identifier
    /// * `outcome_filter` - Optional filter by outcome
    /// * `limit` - Maximum number of results
    /// * `offset` - Offset for pagination
    ///
    /// # Returns
    ///
    /// List of entry results.
    async fn get_entry_results(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        outcome_filter: Option<BulkEntryOutcome>,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<BulkEntryResult>>;

    /// Gets entry counts for a manifest.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `submission_id` - The submission identifier
    /// * `manifest_id` - The manifest identifier
    ///
    /// # Returns
    ///
    /// Entry count summary.
    async fn get_entry_counts(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<EntryCountSummary>;
}

/// Provider for streaming NDJSON processing.
///
/// This trait extends `BulkSubmitProvider` with the ability to process
/// NDJSON data from an async reader stream.
#[async_trait]
pub trait StreamingBulkSubmitProvider: BulkSubmitProvider {
    /// Processes NDJSON data from a stream.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `submission_id` - The submission identifier
    /// * `manifest_id` - The manifest identifier
    /// * `resource_type` - Expected resource type (for validation)
    /// * `reader` - Async reader providing NDJSON lines
    /// * `options` - Processing options
    ///
    /// # Returns
    ///
    /// Result of the streaming processing.
    async fn process_ndjson_stream(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        resource_type: &str,
        reader: Box<dyn AsyncBufRead + Send + Unpin>,
        options: &BulkProcessingOptions,
    ) -> StorageResult<StreamProcessingResult>;
}

/// Provider for rollback of bulk submissions.
///
/// This trait extends `BulkSubmitProvider` with the ability to track and
/// rollback changes made during a submission.
#[async_trait]
pub trait BulkSubmitRollbackProvider: BulkSubmitProvider {
    /// Records a change for potential rollback.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `submission_id` - The submission identifier
    /// * `change` - The change to record
    async fn record_change(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        change: &SubmissionChange,
    ) -> StorageResult<()>;

    /// Lists recorded changes for a submission.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `submission_id` - The submission identifier
    /// * `limit` - Maximum number of results
    /// * `offset` - Offset for pagination
    ///
    /// # Returns
    ///
    /// List of recorded changes.
    async fn list_changes(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<SubmissionChange>>;

    /// Rolls back a single change.
    ///
    /// For creates: deletes the resource.
    /// For updates: restores the previous content.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `submission_id` - The submission identifier
    /// * `change` - The change to rollback
    ///
    /// # Returns
    ///
    /// Whether the rollback was successful.
    async fn rollback_change(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        change: &SubmissionChange,
    ) -> StorageResult<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_submission_id() {
        let id = SubmissionId::new("my-system", "sub-123");
        assert_eq!(id.submitter, "my-system");
        assert_eq!(id.submission_id, "sub-123");
        assert_eq!(id.to_string(), "my-system/sub-123");
    }

    #[test]
    fn test_submission_id_generate() {
        let id = SubmissionId::generate("my-system");
        assert_eq!(id.submitter, "my-system");
        assert!(!id.submission_id.is_empty());
    }

    #[test]
    fn test_submission_status() {
        assert!(!SubmissionStatus::InProgress.is_terminal());
        assert!(SubmissionStatus::Complete.is_terminal());
        assert!(SubmissionStatus::Aborted.is_terminal());

        let status: SubmissionStatus = "in-progress".parse().unwrap();
        assert_eq!(status, SubmissionStatus::InProgress);
    }

    #[test]
    fn test_manifest_status() {
        assert!(!ManifestStatus::Pending.is_terminal());
        assert!(!ManifestStatus::Processing.is_terminal());
        assert!(ManifestStatus::Completed.is_terminal());
        assert!(ManifestStatus::Failed.is_terminal());
    }

    #[test]
    fn test_bulk_entry_result() {
        let success = BulkEntryResult::success(1, "Patient", "pat-123", true);
        assert!(success.is_success());
        assert!(!success.is_error());
        assert!(success.created);

        let error = BulkEntryResult::validation_error(
            2,
            "Patient",
            serde_json::json!({"resourceType": "OperationOutcome"}),
        );
        assert!(!error.is_success());
        assert!(error.is_error());
    }

    #[test]
    fn test_ndjson_entry_parse() {
        let line = r#"{"resourceType":"Patient","id":"123","name":[{"family":"Smith"}]}"#;
        let entry = NdjsonEntry::parse(1, line).unwrap();

        assert_eq!(entry.line_number, 1);
        assert_eq!(entry.resource_type, "Patient");
        assert_eq!(entry.resource_id, Some("123".to_string()));
    }

    #[test]
    fn test_ndjson_entry_parse_error() {
        let result = NdjsonEntry::parse(1, "not json");
        assert!(result.is_err());

        let result = NdjsonEntry::parse(1, r#"{"id":"123"}"#);
        assert!(result.is_err()); // Missing resourceType
    }

    #[test]
    fn test_bulk_processing_options() {
        let options = BulkProcessingOptions::new()
            .with_batch_size(50)
            .with_max_errors(10)
            .with_continue_on_error(false);

        assert_eq!(options.batch_size, 50);
        assert_eq!(options.max_errors, 10);
        assert!(!options.continue_on_error);
    }

    #[test]
    fn test_bulk_processing_options_strict() {
        let options = BulkProcessingOptions::strict();
        assert!(!options.continue_on_error);
        assert_eq!(options.max_errors, 1);
    }

    #[test]
    fn test_submission_change() {
        let create = SubmissionChange::create("manifest-1", "Patient", "pat-123", "1");
        assert_eq!(create.change_type, ChangeType::Create);
        assert!(create.previous_content.is_none());

        let update = SubmissionChange::update(
            "manifest-1",
            "Patient",
            "pat-123",
            "1",
            "2",
            serde_json::json!({"resourceType": "Patient"}),
        );
        assert_eq!(update.change_type, ChangeType::Update);
        assert!(update.previous_content.is_some());
    }

    #[test]
    fn test_entry_count_summary() {
        let mut counts = EntryCountSummary::new();
        counts.increment(BulkEntryOutcome::Success);
        counts.increment(BulkEntryOutcome::Success);
        counts.increment(BulkEntryOutcome::ValidationError);
        counts.increment(BulkEntryOutcome::ProcessingError);
        counts.increment(BulkEntryOutcome::Skipped);

        assert_eq!(counts.total, 5);
        assert_eq!(counts.success, 2);
        assert_eq!(counts.error_count(), 2);
        assert_eq!(counts.skipped, 1);
    }

    #[test]
    fn test_stream_processing_result() {
        let result = StreamProcessingResult::new().aborted("max errors exceeded");
        assert!(result.aborted);
        assert_eq!(result.abort_reason, Some("max errors exceeded".to_string()));
    }
}
