//! $reindex Operation Implementation.
//!
//! Provides the ability to rebuild search indexes for existing resources
//! when new SearchParameters are added or when indexes need to be repaired.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::error::StorageResult;
use crate::tenant::TenantContext;
use crate::types::StoredResource;

use super::errors::ReindexError;
use super::extractor::SearchParameterExtractor;

/// Audit event helpers for reindex operations.
#[cfg(feature = "audit")]
pub mod audit {
    use helios_audit::{AuditAction, AuditEventBuilder, AuditSink};

    /// Record an audit event for a reindex lifecycle event.
    ///
    /// Call this at reindex start, completion, or cancellation.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_reindex_event(
        sink: &dyn AuditSink,
        source_observer: &str,
        agent: Option<&str>,
        job_id: &str,
        phase: &str,
        resource_types: &[String],
        resources_processed: u64,
        outcome: &str,
    ) {
        let mut builder = AuditEventBuilder::new(source_observer)
            .event_type(
                "http://terminology.hl7.org/CodeSystem/audit-event-type",
                "object",
            )
            .action(AuditAction::Execute)
            .outcome(outcome)
            .detail("audit-operation", "reindex")
            .detail("job-id", job_id)
            .detail("phase", phase)
            .detail("resources-processed", resources_processed.to_string());
        if !resource_types.is_empty() {
            builder = builder.detail("resource-types", resource_types.join(","));
        }
        if let Some(a) = agent {
            builder = builder.agent(a, None, true);
        }
        sink.record(builder.build()).await;
    }

    #[cfg(test)]
    mod tests {
        use helios_audit::{AuditAction, AuditEventBuilder};

        #[test]
        fn test_reindex_event_includes_resources_processed() {
            let event = AuditEventBuilder::new("Device/hfs")
                .event_type(
                    "http://terminology.hl7.org/CodeSystem/audit-event-type",
                    "object",
                )
                .action(AuditAction::Execute)
                .outcome("0")
                .detail("audit-operation", "reindex")
                .detail("job-id", "reindex-1")
                .detail("phase", "complete")
                .detail("resources-processed", "500")
                .build();
            let details = event.entity.as_ref().unwrap()[0].detail.as_ref().unwrap();
            assert_eq!(details.len(), 4);
            assert_eq!(
                details[3].r#type.value.as_deref(),
                Some("resources-processed")
            );
        }
    }
}

/// A page of resources for reindexing.
#[derive(Debug)]
pub struct ResourcePage {
    /// The resources in this page.
    pub resources: Vec<StoredResource>,
    /// Cursor for the next page (None if this is the last page).
    pub next_cursor: Option<String>,
}

/// Trait for storage backends that support reindexing.
///
/// This trait provides the methods needed to iterate through resources
/// for the $reindex operation.
#[async_trait]
pub trait ReindexableStorage: Send + Sync {
    /// Lists all resource types that have resources in the tenant.
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>>;

    /// Counts resources of a specific type.
    async fn count_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64>;

    /// Fetches a page of resources for reindexing.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `resource_type` - The resource type to fetch
    /// * `cursor` - Optional cursor from a previous page
    /// * `limit` - Maximum number of resources to return
    ///
    /// # Returns
    ///
    /// A page of resources with an optional cursor for the next page.
    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<ResourcePage>;

    /// Deletes search index entries for a resource.
    async fn delete_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<()>;

    /// Writes search index entries for a resource.
    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> StorageResult<usize>;

    /// Clears all search index entries for a tenant.
    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64>;
}

/// Request to start a reindex operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReindexRequest {
    /// Target resource types (None = all types).
    pub resource_types: Option<Vec<String>>,

    /// Specific SearchParameter URLs to reindex (None = all active).
    pub search_param_urls: Option<Vec<String>>,

    /// Batch size for processing resources.
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,

    /// Whether to clear existing indexes before reindexing.
    #[serde(default)]
    pub clear_existing: bool,
}

fn default_batch_size() -> u32 {
    100
}

impl Default for ReindexRequest {
    fn default() -> Self {
        Self {
            resource_types: None,
            search_param_urls: None,
            batch_size: default_batch_size(),
            clear_existing: false,
        }
    }
}

impl ReindexRequest {
    /// Creates a new reindex request for all resources.
    pub fn all() -> Self {
        Self::default()
    }

    /// Creates a reindex request for specific resource types.
    pub fn for_types<I, S>(types: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            resource_types: Some(types.into_iter().map(Into::into).collect()),
            ..Self::default()
        }
    }

    /// Creates a reindex request for specific parameters.
    pub fn for_params<I, S>(urls: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            search_param_urls: Some(urls.into_iter().map(Into::into).collect()),
            ..Self::default()
        }
    }

    /// Sets the batch size.
    pub fn with_batch_size(mut self, size: u32) -> Self {
        self.batch_size = size;
        self
    }

    /// Enables clearing existing indexes.
    pub fn clear_existing(mut self) -> Self {
        self.clear_existing = true;
        self
    }
}

/// Status of a reindex operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReindexStatus {
    /// Reindex is queued but not started.
    Queued,
    /// Reindex is currently running.
    InProgress,
    /// Reindex completed successfully.
    Completed,
    /// Reindex failed with an error.
    Failed,
    /// Reindex was cancelled.
    Cancelled,
}

impl ReindexStatus {
    /// Returns true if the job is still running.
    pub fn is_running(&self) -> bool {
        matches!(self, ReindexStatus::Queued | ReindexStatus::InProgress)
    }

    /// Returns true if the job has finished (success, failure, or cancelled).
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            ReindexStatus::Completed | ReindexStatus::Failed | ReindexStatus::Cancelled
        )
    }
}

/// Progress information for a reindex job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReindexProgress {
    /// Unique job identifier.
    pub job_id: String,

    /// Current status.
    pub status: ReindexStatus,

    /// Total number of resources to process.
    pub total_resources: u64,

    /// Number of resources processed so far.
    pub processed_resources: u64,

    /// Number of index entries created.
    pub entries_created: u64,

    /// Errors encountered during processing.
    pub errors: Vec<ReindexProgressError>,

    /// When the job was started.
    pub started_at: Option<String>,

    /// When the job completed.
    pub completed_at: Option<String>,

    /// Error message if status is Failed.
    pub error_message: Option<String>,

    /// Current resource type being processed.
    pub current_resource_type: Option<String>,
}

/// An error encountered during reindexing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReindexProgressError {
    /// Resource type.
    pub resource_type: String,
    /// Resource ID.
    pub resource_id: String,
    /// Error message.
    pub error: String,
}

impl ReindexProgress {
    /// Creates a new progress tracker for a job.
    pub fn new(job_id: impl Into<String>) -> Self {
        Self {
            job_id: job_id.into(),
            status: ReindexStatus::Queued,
            total_resources: 0,
            processed_resources: 0,
            entries_created: 0,
            errors: Vec::new(),
            started_at: None,
            completed_at: None,
            error_message: None,
            current_resource_type: None,
        }
    }

    /// Returns the progress percentage (0-100).
    pub fn percentage(&self) -> f64 {
        if self.total_resources == 0 {
            0.0
        } else {
            (self.processed_resources as f64 / self.total_resources as f64) * 100.0
        }
    }

    /// Returns true if any errors occurred.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty() || self.error_message.is_some()
    }

    /// Converts to FHIR Parameters resource.
    pub fn to_parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "jobId", "valueString": self.job_id},
                {"name": "status", "valueCode": format!("{:?}", self.status).to_lowercase()},
                {"name": "total", "valueInteger": self.total_resources},
                {"name": "processed", "valueInteger": self.processed_resources},
                {"name": "entriesCreated", "valueInteger": self.entries_created},
                {"name": "errorCount", "valueInteger": self.errors.len()},
                {"name": "percentage", "valueDecimal": self.percentage()}
            ]
        })
    }
}

/// Manages reindex operations.
pub struct ReindexOperation<S: ReindexableStorage> {
    /// The storage backend.
    storage: Arc<S>,
    /// The search parameter extractor.
    extractor: Arc<SearchParameterExtractor>,
    /// Active jobs.
    jobs: Arc<RwLock<HashMap<String, ReindexProgress>>>,
    /// Cancellation channels.
    cancel_channels: Arc<RwLock<HashMap<String, mpsc::Sender<()>>>>,
}

impl<S: ReindexableStorage + 'static> ReindexOperation<S> {
    /// Creates a new reindex operation manager.
    pub fn new(storage: Arc<S>, extractor: Arc<SearchParameterExtractor>) -> Self {
        Self {
            storage,
            extractor,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            cancel_channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Starts a reindex operation.
    ///
    /// Returns immediately with a job ID. The reindex runs in the background.
    pub async fn start(
        &self,
        tenant: TenantContext,
        request: ReindexRequest,
    ) -> Result<String, ReindexError> {
        let job_id = Uuid::new_v4().to_string();
        let progress = ReindexProgress::new(&job_id);

        // Store the job
        self.jobs.write().insert(job_id.clone(), progress);

        // Create cancellation channel
        let (cancel_tx, cancel_rx) = mpsc::channel::<()>(1);
        self.cancel_channels
            .write()
            .insert(job_id.clone(), cancel_tx);

        // Clone references for the background task
        let storage = self.storage.clone();
        let extractor = self.extractor.clone();
        let jobs = self.jobs.clone();
        let job_id_clone = job_id.clone();

        // Spawn background task
        tokio::spawn(async move {
            Self::run_reindex(
                job_id_clone,
                tenant,
                request,
                storage,
                extractor,
                jobs,
                cancel_rx,
            )
            .await;
        });

        Ok(job_id)
    }

    /// Runs the reindex operation in the background.
    async fn run_reindex(
        job_id: String,
        tenant: TenantContext,
        request: ReindexRequest,
        storage: Arc<S>,
        extractor: Arc<SearchParameterExtractor>,
        jobs: Arc<RwLock<HashMap<String, ReindexProgress>>>,
        mut cancel_rx: mpsc::Receiver<()>,
    ) {
        // Mark as started
        {
            let mut jobs_guard = jobs.write();
            if let Some(progress) = jobs_guard.get_mut(&job_id) {
                progress.status = ReindexStatus::InProgress;
                progress.started_at = Some(chrono::Utc::now().to_rfc3339());
            }
        }

        // Determine resource types to process
        let resource_types = match request.resource_types {
            Some(types) => types,
            None => match storage.list_resource_types(&tenant).await {
                Ok(types) => types,
                Err(e) => {
                    Self::mark_failed(
                        &jobs,
                        &job_id,
                        format!("Failed to list resource types: {}", e),
                    );
                    return;
                }
            },
        };

        // Count total resources
        let mut total_resources: u64 = 0;
        for resource_type in &resource_types {
            match storage.count_resources(&tenant, resource_type).await {
                Ok(count) => total_resources += count,
                Err(e) => {
                    Self::mark_failed(
                        &jobs,
                        &job_id,
                        format!("Failed to count {}: {}", resource_type, e),
                    );
                    return;
                }
            }
        }

        // Update total
        {
            let mut jobs_guard = jobs.write();
            if let Some(progress) = jobs_guard.get_mut(&job_id) {
                progress.total_resources = total_resources;
            }
        }

        // Clear existing indexes if requested
        if request.clear_existing
            && let Err(e) = storage.clear_search_index(&tenant).await
        {
            Self::mark_failed(
                &jobs,
                &job_id,
                format!("Failed to clear search index: {}", e),
            );
            return;
        }

        // Process each resource type
        for resource_type in &resource_types {
            // Check for cancellation
            if cancel_rx.try_recv().is_ok() {
                Self::mark_cancelled(&jobs, &job_id);
                return;
            }

            // Update current resource type
            {
                let mut jobs_guard = jobs.write();
                if let Some(progress) = jobs_guard.get_mut(&job_id) {
                    progress.current_resource_type = Some(resource_type.clone());
                }
            }

            // Process resources in batches
            let mut cursor: Option<String> = None;
            loop {
                // Check for cancellation
                if cancel_rx.try_recv().is_ok() {
                    Self::mark_cancelled(&jobs, &job_id);
                    return;
                }

                // Fetch a page of resources
                let page = match storage
                    .fetch_resources_page(
                        &tenant,
                        resource_type,
                        cursor.as_deref(),
                        request.batch_size,
                    )
                    .await
                {
                    Ok(page) => page,
                    Err(e) => {
                        Self::mark_failed(
                            &jobs,
                            &job_id,
                            format!("Failed to fetch resources: {}", e),
                        );
                        return;
                    }
                };

                // Process each resource
                for resource in &page.resources {
                    // Delete existing index entries
                    if let Err(e) = storage
                        .delete_search_entries(&tenant, resource_type, resource.id())
                        .await
                    {
                        // Log error but continue
                        let mut jobs_guard = jobs.write();
                        if let Some(progress) = jobs_guard.get_mut(&job_id) {
                            progress.errors.push(ReindexProgressError {
                                resource_type: resource_type.clone(),
                                resource_id: resource.id().to_string(),
                                error: format!("Failed to delete index entries: {}", e),
                            });
                        }
                        continue;
                    }

                    // Extract and write new index entries
                    match extractor.extract(resource.content(), resource_type) {
                        Ok(values) => {
                            let entry_count = values.len();

                            // Write new index entries
                            match storage
                                .write_search_entries(
                                    &tenant,
                                    resource_type,
                                    resource.id(),
                                    resource.content(),
                                )
                                .await
                            {
                                Ok(_written) => {
                                    let mut jobs_guard = jobs.write();
                                    if let Some(progress) = jobs_guard.get_mut(&job_id) {
                                        progress.processed_resources += 1;
                                        progress.entries_created += entry_count as u64;
                                    }
                                }
                                Err(e) => {
                                    let mut jobs_guard = jobs.write();
                                    if let Some(progress) = jobs_guard.get_mut(&job_id) {
                                        progress.processed_resources += 1;
                                        progress.errors.push(ReindexProgressError {
                                            resource_type: resource_type.clone(),
                                            resource_id: resource.id().to_string(),
                                            error: format!("Failed to write index entries: {}", e),
                                        });
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            let mut jobs_guard = jobs.write();
                            if let Some(progress) = jobs_guard.get_mut(&job_id) {
                                progress.processed_resources += 1;
                                progress.errors.push(ReindexProgressError {
                                    resource_type: resource_type.clone(),
                                    resource_id: resource.id().to_string(),
                                    error: format!("Extraction failed: {}", e),
                                });
                            }
                        }
                    }
                }

                // Check if there are more pages
                match page.next_cursor {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
        }

        // Mark as completed
        {
            let mut jobs_guard = jobs.write();
            if let Some(progress) = jobs_guard.get_mut(&job_id) {
                progress.status = ReindexStatus::Completed;
                progress.completed_at = Some(chrono::Utc::now().to_rfc3339());
                progress.current_resource_type = None;
            }
        }
    }

    /// Marks a job as failed.
    fn mark_failed(
        jobs: &Arc<RwLock<HashMap<String, ReindexProgress>>>,
        job_id: &str,
        error: String,
    ) {
        let mut jobs_guard = jobs.write();
        if let Some(progress) = jobs_guard.get_mut(job_id) {
            progress.status = ReindexStatus::Failed;
            progress.error_message = Some(error);
            progress.completed_at = Some(chrono::Utc::now().to_rfc3339());
        }
    }

    /// Marks a job as cancelled.
    fn mark_cancelled(jobs: &Arc<RwLock<HashMap<String, ReindexProgress>>>, job_id: &str) {
        let mut jobs_guard = jobs.write();
        if let Some(progress) = jobs_guard.get_mut(job_id) {
            progress.status = ReindexStatus::Cancelled;
            progress.completed_at = Some(chrono::Utc::now().to_rfc3339());
        }
    }

    /// Gets the progress of a reindex job.
    pub async fn get_progress(&self, job_id: &str) -> Option<ReindexProgress> {
        self.jobs.read().get(job_id).cloned()
    }

    /// Cancels a running reindex job.
    pub async fn cancel(&self, job_id: &str) -> Result<(), ReindexError> {
        // Check if job exists and is running
        {
            let jobs = self.jobs.read();
            let progress = jobs.get(job_id).ok_or_else(|| ReindexError::JobNotFound {
                job_id: job_id.to_string(),
            })?;

            if !progress.status.is_running() {
                return Ok(()); // Already finished
            }
        }

        // Send cancellation signal
        let tx = self.cancel_channels.read().get(job_id).cloned();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }

        // Update status
        {
            let mut jobs = self.jobs.write();
            if let Some(progress) = jobs.get_mut(job_id) {
                progress.status = ReindexStatus::Cancelled;
                progress.completed_at = Some(chrono::Utc::now().to_rfc3339());
            }
        }

        Ok(())
    }

    /// Lists all jobs (active and recent).
    pub fn list_jobs(&self) -> Vec<ReindexProgress> {
        self.jobs.read().values().cloned().collect()
    }

    /// Removes completed jobs older than the specified duration.
    pub fn cleanup_old_jobs(&self, max_age_seconds: i64) {
        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(max_age_seconds);

        let mut jobs = self.jobs.write();
        let mut channels = self.cancel_channels.write();

        jobs.retain(|job_id, progress| {
            if progress.status.is_finished()
                && let Some(ref completed_at) = progress.completed_at
                && let Ok(completed) = chrono::DateTime::parse_from_rfc3339(completed_at)
                && completed.with_timezone(&chrono::Utc) < cutoff
            {
                channels.remove(job_id);
                return false;
            }
            true
        });
    }
}

impl<S: ReindexableStorage> std::fmt::Debug for ReindexOperation<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReindexOperation")
            .field("active_jobs", &self.jobs.read().len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reindex_request() {
        let req = ReindexRequest::for_types(vec!["Patient", "Observation"])
            .with_batch_size(50)
            .clear_existing();

        assert_eq!(req.resource_types.as_ref().unwrap().len(), 2);
        assert_eq!(req.batch_size, 50);
        assert!(req.clear_existing);
    }

    #[test]
    fn test_reindex_status() {
        assert!(ReindexStatus::InProgress.is_running());
        assert!(!ReindexStatus::Completed.is_running());
        assert!(ReindexStatus::Completed.is_finished());
        assert!(ReindexStatus::Failed.is_finished());
    }

    #[test]
    fn test_reindex_progress() {
        let mut progress = ReindexProgress::new("job-123");
        progress.total_resources = 100;
        progress.processed_resources = 50;

        assert_eq!(progress.percentage(), 50.0);
        assert!(!progress.has_errors());

        progress.errors.push(ReindexProgressError {
            resource_type: "Patient".to_string(),
            resource_id: "1".to_string(),
            error: "test error".to_string(),
        });

        assert!(progress.has_errors());
    }

    #[test]
    fn test_progress_to_parameters() {
        let progress = ReindexProgress::new("job-123");
        let params = progress.to_parameters();

        assert_eq!(params["resourceType"], "Parameters");
        assert!(params["parameter"].is_array());
    }
}
