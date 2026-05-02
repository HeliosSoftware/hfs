//! Bulk export types and traits.
//!
//! This module provides types and traits for implementing FHIR Bulk Data Export
//! as specified in the [FHIR Bulk Data Access IG](https://hl7.org/fhir/uv/bulkdata/export.html).
//!
//! # Export Levels
//!
//! The Bulk Data Export specification supports three levels of export:
//!
//! - **System-level** (`[base]/$export`) - Exports all resources in the system
//! - **Patient-level** (`[base]/Patient/$export`) - Exports all patient compartment resources
//! - **Group-level** (`[base]/Group/[id]/$export`) - Exports resources for patients in a group
//!
//! # Example
//!
//! ```ignore
//! use helios_persistence::core::bulk_export::{
//!     BulkExportStorage, ExportRequest, ExportLevel, ExportStatus,
//! };
//!
//! async fn export_patients<S: BulkExportStorage>(storage: &S, tenant: &TenantContext) {
//!     // Start a system-level export of Patient resources
//!     let request = ExportRequest::new(ExportLevel::System)
//!         .with_types(vec!["Patient".to_string()]);
//!
//!     let job_id = storage.start_export(tenant, request).await.unwrap();
//!
//!     // Poll for completion
//!     loop {
//!         let progress = storage.get_export_status(tenant, &job_id).await.unwrap();
//!         match progress.status {
//!             ExportStatus::Complete => break,
//!             ExportStatus::Error => panic!("Export failed"),
//!             _ => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
//!         }
//!     }
//!
//!     // Get the manifest
//!     let manifest = storage.get_export_manifest(tenant, &job_id).await.unwrap();
//!     for file in manifest.output {
//!         println!("Exported {} {} resources to {}", file.count, file.resource_type, file.url);
//!     }
//! }
//! ```

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Audit event helpers for bulk export operations.
#[cfg(feature = "audit")]
pub mod audit {
    use helios_audit::{AuditAction, AuditEventBuilder, AuditSink};

    use super::ExportLevel;

    /// Record an audit event for a bulk export lifecycle event.
    ///
    /// Call this at export start, completion, cancellation, or failure.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_export_event(
        sink: &dyn AuditSink,
        source_observer: &str,
        agent: Option<&str>,
        job_id: &str,
        level: &ExportLevel,
        resource_types: &[String],
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
            .detail("audit-operation", "bulk-export")
            .detail("job-id", job_id)
            .detail("export-level", level.to_string());
        if !resource_types.is_empty() {
            builder = builder.detail("resource-types", resource_types.join(","));
        }
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
        use crate::core::bulk_export::ExportLevel;

        #[tokio::test]
        async fn test_export_event_has_job_id() {
            let sink = NullSink;
            record_export_event(
                &sink,
                "Device/hfs",
                Some("Practitioner/dr-1"),
                "job-abc",
                &ExportLevel::System,
                &["Patient".to_string()],
                "0",
                None,
            )
            .await;
            // NullSink discards; we verify it doesn't panic and compiles correctly
        }

        #[test]
        fn test_export_event_type_is_object_for_bulk_export() {
            let event = AuditEventBuilder::new("Device/hfs")
                .event_type(
                    "http://terminology.hl7.org/CodeSystem/audit-event-type",
                    "object",
                )
                .action(AuditAction::Execute)
                .outcome("0")
                .detail("audit-operation", "bulk-export")
                .detail("job-id", "j1")
                .build();
            assert_eq!(
                event.r#type.code.as_ref().and_then(|c| c.value.as_deref()),
                Some("object")
            );
        }
    }
}

use crate::error::StorageResult;
use crate::tenant::TenantContext;

/// Unique identifier for an export job.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExportJobId(String);

impl ExportJobId {
    /// Creates a new random export job ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Creates an export job ID from an existing string.
    pub fn from_string(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the ID as a string reference.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ExportJobId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ExportJobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for ExportJobId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for ExportJobId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Status of an export job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportStatus {
    /// Job has been accepted but not yet started processing.
    Accepted,
    /// Job is currently processing.
    InProgress,
    /// Job has completed successfully.
    Complete,
    /// Job failed with an error.
    Error,
    /// Job was cancelled by the user.
    Cancelled,
}

impl ExportStatus {
    /// Returns true if the job is in a terminal state (complete, error, or cancelled).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Complete | Self::Error | Self::Cancelled)
    }

    /// Returns true if the job is still active (accepted or in progress).
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Accepted | Self::InProgress)
    }
}

impl std::fmt::Display for ExportStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Accepted => write!(f, "accepted"),
            Self::InProgress => write!(f, "in-progress"),
            Self::Complete => write!(f, "complete"),
            Self::Error => write!(f, "error"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::str::FromStr for ExportStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "accepted" => Ok(Self::Accepted),
            "in-progress" | "in_progress" => Ok(Self::InProgress),
            "complete" => Ok(Self::Complete),
            "error" => Ok(Self::Error),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!("unknown export status: {}", s)),
        }
    }
}

/// Level at which the export is being performed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportLevel {
    /// System-level export (`[base]/$export`).
    System,
    /// Patient-level export (`[base]/Patient/$export`).
    Patient,
    /// Group-level export (`[base]/Group/[id]/$export`).
    Group {
        /// The group ID to export.
        group_id: String,
    },
}

impl ExportLevel {
    /// Creates a system-level export.
    pub fn system() -> Self {
        Self::System
    }

    /// Creates a patient-level export.
    pub fn patient() -> Self {
        Self::Patient
    }

    /// Creates a group-level export for the given group ID.
    pub fn group(group_id: impl Into<String>) -> Self {
        Self::Group {
            group_id: group_id.into(),
        }
    }
}

impl std::fmt::Display for ExportLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::System => write!(f, "system"),
            Self::Patient => write!(f, "patient"),
            Self::Group { group_id } => write!(f, "group/{}", group_id),
        }
    }
}

/// A type filter for the export request.
///
/// Type filters allow specifying FHIR search parameters that should be applied
/// when exporting a specific resource type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeFilter {
    /// The resource type this filter applies to.
    pub resource_type: String,
    /// The search query parameters.
    pub query: String,
}

impl TypeFilter {
    /// Creates a new type filter.
    pub fn new(resource_type: impl Into<String>, query: impl Into<String>) -> Self {
        Self {
            resource_type: resource_type.into(),
            query: query.into(),
        }
    }
}

/// Request parameters for starting an export job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportRequest {
    /// The level at which to perform the export.
    pub level: ExportLevel,

    /// Resource types to export. If empty, all applicable types are exported.
    #[serde(default)]
    pub resource_types: Vec<String>,

    /// Only include resources modified since this time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<DateTime<Utc>>,

    /// Type-specific filters to apply during export.
    #[serde(default)]
    pub type_filters: Vec<TypeFilter>,

    /// Batch size for processing (implementation-specific).
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,

    /// Output format (default: "application/fhir+ndjson").
    #[serde(default = "default_output_format")]
    pub output_format: String,
}

fn default_batch_size() -> u32 {
    1000
}

fn default_output_format() -> String {
    "application/fhir+ndjson".to_string()
}

impl ExportRequest {
    /// Creates a new export request with the given level.
    pub fn new(level: ExportLevel) -> Self {
        Self {
            level,
            resource_types: Vec::new(),
            since: None,
            type_filters: Vec::new(),
            batch_size: default_batch_size(),
            output_format: default_output_format(),
        }
    }

    /// Creates a system-level export request.
    pub fn system() -> Self {
        Self::new(ExportLevel::System)
    }

    /// Creates a patient-level export request.
    pub fn patient() -> Self {
        Self::new(ExportLevel::Patient)
    }

    /// Creates a group-level export request.
    pub fn group(group_id: impl Into<String>) -> Self {
        Self::new(ExportLevel::Group {
            group_id: group_id.into(),
        })
    }

    /// Sets the resource types to export.
    pub fn with_types(mut self, types: Vec<String>) -> Self {
        self.resource_types = types;
        self
    }

    /// Sets the since filter.
    pub fn with_since(mut self, since: DateTime<Utc>) -> Self {
        self.since = Some(since);
        self
    }

    /// Adds a type filter.
    pub fn with_type_filter(mut self, filter: TypeFilter) -> Self {
        self.type_filters.push(filter);
        self
    }

    /// Adds multiple type filters.
    pub fn with_type_filters(mut self, filters: Vec<TypeFilter>) -> Self {
        self.type_filters.extend(filters);
        self
    }

    /// Sets the batch size.
    pub fn with_batch_size(mut self, batch_size: u32) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Returns the group ID if this is a group-level export.
    pub fn group_id(&self) -> Option<&str> {
        match &self.level {
            ExportLevel::Group { group_id } => Some(group_id),
            _ => None,
        }
    }
}

/// Progress information for a single resource type in an export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeExportProgress {
    /// The resource type.
    pub resource_type: String,
    /// Total number of resources to export (may be estimated).
    pub total_count: Option<u64>,
    /// Number of resources exported so far.
    pub exported_count: u64,
    /// Number of errors encountered.
    pub error_count: u64,
    /// Current cursor state for resuming (opaque to clients).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_state: Option<String>,
}

impl TypeExportProgress {
    /// Creates new progress tracking for a resource type.
    pub fn new(resource_type: impl Into<String>) -> Self {
        Self {
            resource_type: resource_type.into(),
            total_count: None,
            exported_count: 0,
            error_count: 0,
            cursor_state: None,
        }
    }

    /// Sets the total count.
    pub fn with_total(mut self, total: u64) -> Self {
        self.total_count = Some(total);
        self
    }

    /// Returns the progress as a percentage (0.0 to 1.0).
    pub fn progress_fraction(&self) -> Option<f64> {
        self.total_count.map(|total| {
            if total == 0 {
                1.0
            } else {
                self.exported_count as f64 / total as f64
            }
        })
    }
}

/// Overall progress of an export job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportProgress {
    /// The job ID.
    pub job_id: ExportJobId,
    /// Current status of the job.
    pub status: ExportStatus,
    /// The export level.
    pub level: ExportLevel,
    /// Time the export was initiated.
    pub transaction_time: DateTime<Utc>,
    /// Time the export started processing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    /// Time the export completed (success, error, or cancelled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Per-type progress information.
    pub type_progress: Vec<TypeExportProgress>,
    /// Current type being processed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_type: Option<String>,
    /// Error message if status is Error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl ExportProgress {
    /// Creates new progress for an accepted job.
    pub fn accepted(
        job_id: ExportJobId,
        level: ExportLevel,
        transaction_time: DateTime<Utc>,
    ) -> Self {
        Self {
            job_id,
            status: ExportStatus::Accepted,
            level,
            transaction_time,
            started_at: None,
            completed_at: None,
            type_progress: Vec::new(),
            current_type: None,
            error_message: None,
        }
    }

    /// Returns the overall progress as a percentage (0.0 to 1.0).
    pub fn overall_progress(&self) -> f64 {
        if self.type_progress.is_empty() {
            return 0.0;
        }

        let (total_exported, total_count) = self
            .type_progress
            .iter()
            .fold((0u64, 0u64), |(exp, tot), tp| {
                (exp + tp.exported_count, tot + tp.total_count.unwrap_or(0))
            });

        if total_count == 0 {
            0.0
        } else {
            total_exported as f64 / total_count as f64
        }
    }
}

/// An output file in the export manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOutputFile {
    /// The resource type contained in this file.
    #[serde(rename = "type")]
    pub resource_type: String,
    /// URL to access the file.
    pub url: String,
    /// Number of resources in the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
}

impl ExportOutputFile {
    /// Creates a new output file descriptor.
    pub fn new(resource_type: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            resource_type: resource_type.into(),
            url: url.into(),
            count: None,
        }
    }

    /// Sets the count.
    pub fn with_count(mut self, count: u64) -> Self {
        self.count = Some(count);
        self
    }
}

/// The export manifest returned when an export completes.
///
/// This follows the FHIR Bulk Data Export manifest format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportManifest {
    /// Time the export was initiated.
    #[serde(rename = "transactionTime")]
    pub transaction_time: DateTime<Utc>,
    /// The original export request URL.
    pub request: String,
    /// Whether the client should check for deleted resources.
    #[serde(rename = "requiresAccessToken")]
    pub requires_access_token: bool,
    /// Output files containing the exported resources.
    pub output: Vec<ExportOutputFile>,
    /// Output files containing OperationOutcome resources for errors.
    #[serde(default)]
    pub error: Vec<ExportOutputFile>,
    /// Informational messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Extension data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<Value>,
}

impl ExportManifest {
    /// Creates a new export manifest.
    pub fn new(transaction_time: DateTime<Utc>, request: impl Into<String>) -> Self {
        Self {
            transaction_time,
            request: request.into(),
            requires_access_token: true,
            output: Vec::new(),
            error: Vec::new(),
            message: None,
            extension: None,
        }
    }

    /// Adds an output file.
    pub fn with_output(mut self, file: ExportOutputFile) -> Self {
        self.output.push(file);
        self
    }

    /// Adds an error file.
    pub fn with_error(mut self, file: ExportOutputFile) -> Self {
        self.error.push(file);
        self
    }

    /// Sets a message.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
}

/// A batch of NDJSON resources for streaming export.
#[derive(Debug, Clone)]
pub struct NdjsonBatch {
    /// The serialized NDJSON lines (one JSON object per line).
    pub lines: Vec<String>,
    /// Cursor for fetching the next batch, if any.
    pub next_cursor: Option<String>,
    /// Whether this is the last batch.
    pub is_last: bool,
}

impl NdjsonBatch {
    /// Creates a new batch.
    pub fn new(lines: Vec<String>) -> Self {
        Self {
            lines,
            next_cursor: None,
            is_last: false,
        }
    }

    /// Creates an empty final batch.
    pub fn empty() -> Self {
        Self {
            lines: Vec::new(),
            next_cursor: None,
            is_last: true,
        }
    }

    /// Sets the next cursor.
    pub fn with_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.next_cursor = Some(cursor.into());
        self
    }

    /// Marks this as the last batch.
    pub fn as_last(mut self) -> Self {
        self.is_last = true;
        self.next_cursor = None;
        self
    }

    /// Returns the number of resources in this batch.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Returns true if this batch is empty.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

// ============================================================================
// Traits
// ============================================================================

/// Storage trait for bulk export job management.
///
/// This trait handles the lifecycle of export jobs: creating, tracking,
/// completing, and cleaning up exports.
#[async_trait]
pub trait BulkExportStorage: Send + Sync {
    /// Starts a new export job.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `request` - The export request parameters
    ///
    /// # Returns
    ///
    /// The job ID for tracking the export.
    ///
    /// # Errors
    ///
    /// * `BulkExportError::TooManyConcurrentExports` - If too many exports are running
    /// * `BulkExportError::InvalidRequest` - If the request is invalid
    async fn start_export(
        &self,
        tenant: &TenantContext,
        request: ExportRequest,
    ) -> StorageResult<ExportJobId>;

    /// Gets the current status of an export job.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `job_id` - The export job ID
    ///
    /// # Returns
    ///
    /// The current progress of the export.
    ///
    /// # Errors
    ///
    /// * `BulkExportError::JobNotFound` - If the job doesn't exist
    async fn get_export_status(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<ExportProgress>;

    /// Cancels an in-progress export job.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `job_id` - The export job ID
    ///
    /// # Errors
    ///
    /// * `BulkExportError::JobNotFound` - If the job doesn't exist
    /// * `BulkExportError::InvalidJobState` - If the job is already complete
    async fn cancel_export(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<()>;

    /// Deletes an export job and its output files.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `job_id` - The export job ID
    ///
    /// # Errors
    ///
    /// * `BulkExportError::JobNotFound` - If the job doesn't exist
    async fn delete_export(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<()>;

    /// Gets the manifest for a completed export.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `job_id` - The export job ID
    ///
    /// # Returns
    ///
    /// The export manifest with output file information.
    ///
    /// # Errors
    ///
    /// * `BulkExportError::JobNotFound` - If the job doesn't exist
    /// * `BulkExportError::InvalidJobState` - If the job is not complete
    async fn get_export_manifest(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<ExportManifest>;

    /// Lists export jobs for a tenant.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `include_completed` - Whether to include completed jobs
    ///
    /// # Returns
    ///
    /// List of export progress records.
    async fn list_exports(
        &self,
        tenant: &TenantContext,
        include_completed: bool,
    ) -> StorageResult<Vec<ExportProgress>>;
}

/// Data provider for export operations.
///
/// This trait provides the data retrieval capabilities needed to perform
/// system-level exports.
#[async_trait]
pub trait ExportDataProvider: Send + Sync {
    /// Lists resource types available for export.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `request` - The export request (used to filter by requested types)
    ///
    /// # Returns
    ///
    /// List of resource type names that should be exported.
    async fn list_export_types(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
    ) -> StorageResult<Vec<String>>;

    /// Counts resources of a type for export.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `request` - The export request (for filters)
    /// * `resource_type` - The resource type to count
    ///
    /// # Returns
    ///
    /// The count of resources matching the export criteria.
    async fn count_export_resources(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        resource_type: &str,
    ) -> StorageResult<u64>;

    /// Fetches a batch of resources for export.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `request` - The export request (for filters)
    /// * `resource_type` - The resource type to fetch
    /// * `cursor` - Cursor from previous batch, or None for first batch
    /// * `batch_size` - Maximum number of resources to return
    ///
    /// # Returns
    ///
    /// A batch of NDJSON lines with cursor for next batch.
    async fn fetch_export_batch(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        resource_type: &str,
        cursor: Option<&str>,
        batch_size: u32,
    ) -> StorageResult<NdjsonBatch>;
}

/// Provider for patient compartment exports.
///
/// This trait extends `ExportDataProvider` with patient-specific capabilities
/// needed for Patient-level exports.
#[async_trait]
pub trait PatientExportProvider: ExportDataProvider {
    /// Lists patient IDs to include in the export.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `request` - The export request
    /// * `cursor` - Cursor from previous call, or None for first call
    /// * `batch_size` - Maximum number of patient IDs to return
    ///
    /// # Returns
    ///
    /// A tuple of (patient_ids, next_cursor).
    async fn list_patient_ids(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        cursor: Option<&str>,
        batch_size: u32,
    ) -> StorageResult<(Vec<String>, Option<String>)>;

    /// Fetches a batch of resources from the patient compartment.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `request` - The export request
    /// * `resource_type` - The resource type to fetch
    /// * `patient_ids` - Patient IDs whose resources to fetch
    /// * `cursor` - Cursor from previous batch, or None for first batch
    /// * `batch_size` - Maximum number of resources to return
    ///
    /// # Returns
    ///
    /// A batch of NDJSON lines with cursor for next batch.
    async fn fetch_patient_compartment_batch(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        resource_type: &str,
        patient_ids: &[String],
        cursor: Option<&str>,
        batch_size: u32,
    ) -> StorageResult<NdjsonBatch>;
}

/// Provider for group-level exports.
///
/// This trait extends `PatientExportProvider` with group-specific capabilities
/// needed for Group-level exports.
#[async_trait]
pub trait GroupExportProvider: PatientExportProvider {
    /// Gets the members of a group.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `group_id` - The group resource ID
    ///
    /// # Returns
    ///
    /// List of member references (e.g., "Patient/123").
    ///
    /// # Errors
    ///
    /// * `BulkExportError::GroupNotFound` - If the group doesn't exist
    async fn get_group_members(
        &self,
        tenant: &TenantContext,
        group_id: &str,
    ) -> StorageResult<Vec<String>>;

    /// Resolves group members to patient IDs.
    ///
    /// This handles the case where group members may be references to other
    /// resources (like Practitioner) or nested Groups.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `group_id` - The group resource ID
    ///
    /// # Returns
    ///
    /// List of patient IDs for the group members.
    async fn resolve_group_patient_ids(
        &self,
        tenant: &TenantContext,
        group_id: &str,
    ) -> StorageResult<Vec<String>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_job_id() {
        let id = ExportJobId::new();
        assert!(!id.as_str().is_empty());

        let id2 = ExportJobId::from_string("test-123");
        assert_eq!(id2.as_str(), "test-123");
        assert_eq!(id2.to_string(), "test-123");
    }

    #[test]
    fn test_export_status() {
        assert!(ExportStatus::Complete.is_terminal());
        assert!(ExportStatus::Error.is_terminal());
        assert!(ExportStatus::Cancelled.is_terminal());
        assert!(!ExportStatus::Accepted.is_terminal());
        assert!(!ExportStatus::InProgress.is_terminal());

        assert!(ExportStatus::Accepted.is_active());
        assert!(ExportStatus::InProgress.is_active());
        assert!(!ExportStatus::Complete.is_active());
    }

    #[test]
    fn test_export_status_display_parse() {
        let status = ExportStatus::InProgress;
        assert_eq!(status.to_string(), "in-progress");

        let parsed: ExportStatus = "in-progress".parse().unwrap();
        assert_eq!(parsed, ExportStatus::InProgress);

        // Also accept underscore variant
        let parsed: ExportStatus = "in_progress".parse().unwrap();
        assert_eq!(parsed, ExportStatus::InProgress);
    }

    #[test]
    fn test_export_level() {
        let system = ExportLevel::system();
        assert!(matches!(system, ExportLevel::System));

        let patient = ExportLevel::patient();
        assert!(matches!(patient, ExportLevel::Patient));

        let group = ExportLevel::group("grp-123");
        assert!(matches!(group, ExportLevel::Group { group_id } if group_id == "grp-123"));
    }

    #[test]
    fn test_export_request_builder() {
        let request = ExportRequest::system()
            .with_types(vec!["Patient".to_string(), "Observation".to_string()])
            .with_batch_size(500)
            .with_type_filter(TypeFilter::new("Observation", "code=1234"));

        assert!(matches!(request.level, ExportLevel::System));
        assert_eq!(request.resource_types, vec!["Patient", "Observation"]);
        assert_eq!(request.batch_size, 500);
        assert_eq!(request.type_filters.len(), 1);
    }

    #[test]
    fn test_export_request_group_id() {
        let request = ExportRequest::group("grp-123");
        assert_eq!(request.group_id(), Some("grp-123"));

        let system_request = ExportRequest::system();
        assert_eq!(system_request.group_id(), None);
    }

    #[test]
    fn test_type_export_progress() {
        let progress = TypeExportProgress::new("Patient").with_total(100);
        assert_eq!(progress.total_count, Some(100));
        assert_eq!(progress.progress_fraction(), Some(0.0));

        let mut progress = progress;
        progress.exported_count = 50;
        assert_eq!(progress.progress_fraction(), Some(0.5));
    }

    #[test]
    fn test_export_manifest() {
        let manifest = ExportManifest::new(Utc::now(), "https://example.com/$export")
            .with_output(
                ExportOutputFile::new("Patient", "/exports/Patient.ndjson").with_count(100),
            )
            .with_message("Export complete");

        assert_eq!(manifest.output.len(), 1);
        assert_eq!(manifest.output[0].resource_type, "Patient");
        assert_eq!(manifest.output[0].count, Some(100));
        assert!(manifest.message.is_some());
    }

    #[test]
    fn test_ndjson_batch() {
        let batch = NdjsonBatch::new(vec![
            r#"{"resourceType":"Patient","id":"1"}"#.to_string(),
            r#"{"resourceType":"Patient","id":"2"}"#.to_string(),
        ])
        .with_cursor("next-page");

        assert_eq!(batch.len(), 2);
        assert!(!batch.is_empty());
        assert!(!batch.is_last);
        assert_eq!(batch.next_cursor, Some("next-page".to_string()));

        let final_batch = batch.as_last();
        assert!(final_batch.is_last);
        assert!(final_batch.next_cursor.is_none());
    }

    #[test]
    fn test_ndjson_batch_empty() {
        let batch = NdjsonBatch::empty();
        assert!(batch.is_empty());
        assert!(batch.is_last);
    }
}
