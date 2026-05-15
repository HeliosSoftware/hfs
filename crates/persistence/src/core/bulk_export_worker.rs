//! Worker-facing traits for bulk export job execution.
//!
//! These traits are *not* part of the REST-facing [`BulkExportStorage`] surface
//! — they are what the export worker uses to claim jobs and persist progress
//! under a heartbeated, fencing-token-guarded lease.
//!
//! [`BulkExportStorage`]: crate::core::bulk_export::BulkExportStorage

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::core::bulk_export::{
    BulkExportStorage, ExportDataProvider, ExportJobId, ExportLevel, ExportRequest, ExportStatus,
    GroupExportProvider, PatientExportProvider, TypeExportProgress,
};
use crate::core::bulk_export_output::{ExportOutputStore, ExportPartKey, FinalizedPart};
use crate::error::{StorageError, StorageResult};
use crate::tenant::TenantContext;

/// Identifier for an export worker instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkerId(String);

impl WorkerId {
    /// Creates a worker ID from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generates a fresh random worker ID.
    pub fn random() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// Returns the ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WorkerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A lease over a single export job, held by exactly one worker at a time.
///
/// Leases expire; if the holding worker does not heartbeat before
/// `lease_expiry`, the lease is reclaimable. The `fencing_token` is bumped on
/// every claim so a zombie worker cannot mutate a job another worker now owns.
#[derive(Debug, Clone)]
pub struct ExportJobLease {
    /// The leased job.
    pub job_id: ExportJobId,
    /// The tenant the job belongs to.
    pub tenant: TenantContext,
    /// The worker holding the lease.
    pub worker_id: WorkerId,
    /// When the lease expires if not renewed.
    pub lease_expiry: DateTime<Utc>,
    /// Monotonically increasing token, bumped on every claim.
    pub fencing_token: u64,
}

/// Error returned by fenced worker-storage operations.
#[derive(Debug)]
pub enum LeaseError {
    /// The lease was lost — another worker reclaimed the job. The caller MUST
    /// stop writing immediately.
    LeaseLost {
        /// The job whose lease was lost.
        job_id: ExportJobId,
    },
    /// An underlying storage error.
    Storage(StorageError),
}

impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LeaseLost { job_id } => {
                write!(
                    f,
                    "export job {job_id} lease lost (reclaimed by another worker)"
                )
            }
            Self::Storage(e) => write!(f, "storage error: {e}"),
        }
    }
}

impl std::error::Error for LeaseError {}

impl From<StorageError> for LeaseError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

/// The worker's view of a claimed job: everything needed to (re)run it.
#[derive(Debug, Clone)]
pub struct WorkerJobView {
    /// The original export request.
    pub request: ExportRequest,
    /// The export level.
    pub level: ExportLevel,
    /// Server wall-clock frozen at kickoff.
    pub transaction_time: DateTime<Utc>,
    /// The FHIR version the export runs against.
    pub fhir_version: helios_fhir::FhirVersion,
    /// Already-persisted per-type progress, for resuming after a crash.
    pub type_progress: Vec<TypeExportProgress>,
}

/// Strategy for atomically claiming the next available export job.
///
/// Each backend reaches for its native primitive — `SELECT … FOR UPDATE SKIP
/// LOCKED` on Postgres, a process-local mutex on SQLite.
#[async_trait]
pub trait ExportClaimStrategy: Send + Sync {
    /// Atomically transitions one eligible job (`accepted`, or `in_progress`
    /// with an expired lease) to held-by-this-worker, bumping the fencing
    /// token. Returns `Ok(None)` when no job is available.
    async fn claim_next(
        &self,
        worker_id: &WorkerId,
        lease_duration: Duration,
    ) -> StorageResult<Option<ExportJobLease>>;

    /// Renews a lease the worker still holds; returns the new expiry, or
    /// `LeaseError::LeaseLost` if the job was reclaimed.
    async fn heartbeat(&self, lease: &ExportJobLease) -> Result<DateTime<Utc>, LeaseError>;

    /// Releases a lease early (graceful shutdown). Best-effort.
    async fn release(&self, lease: ExportJobLease) -> StorageResult<()>;
}

/// Worker-owned mutations of job state.
///
/// **Every method is fenced** by `worker_id` + `fencing_token`: a guarded
/// mutation affecting zero rows returns `LeaseError::LeaseLost`, so a zombie
/// worker cannot corrupt progress, file rows, or terminal status after its
/// job has been reclaimed.
#[async_trait]
pub trait ExportWorkerStorage: Send + Sync {
    /// Loads the claimed job's request, level, frozen metadata and persisted
    /// per-type progress (for resume). Fenced.
    async fn get_export_job_for_worker(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
    ) -> Result<WorkerJobView, LeaseError>;

    /// Marks the job `in_progress` (sets `started_at` if unset). Fenced.
    async fn mark_export_in_progress(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
    ) -> Result<(), LeaseError>;

    /// Idempotent upsert of per-type progress (cursor + counts). Fenced.
    async fn update_export_type_progress(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
        progress: &TypeExportProgress,
    ) -> Result<(), LeaseError>;

    /// Idempotent upsert of a finalized output/error file row. Fenced.
    async fn record_export_file(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
        part: &FinalizedPart,
        file_type: &str,
    ) -> Result<(), LeaseError>;

    /// Marks the job `complete` (sets `completed_at`). Fenced.
    async fn finish_export_job(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
    ) -> Result<(), LeaseError>;

    /// Marks the job `error` with a message (sets `completed_at`). Fenced.
    async fn fail_export_job(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
        error_message: &str,
    ) -> Result<(), LeaseError>;
}

/// Marker trait composing the three job-state surfaces a worker needs.
///
/// Only the SQLite and Postgres backends implement this; it is held as an
/// `Arc<dyn BulkExportJobStore>` and selected at bootstrap by
/// `HFS_BULK_EXPORT_BACKEND`.
pub trait BulkExportJobStore:
    BulkExportStorage + ExportWorkerStorage + ExportClaimStrategy
{
}

impl<T> BulkExportJobStore for T where
    T: BulkExportStorage + ExportWorkerStorage + ExportClaimStrategy
{
}

/// Marker trait for a resource-store that can feed every export level.
pub trait ExportResourceProvider:
    ExportDataProvider + PatientExportProvider + GroupExportProvider
{
}

impl<T> ExportResourceProvider for T where
    T: ExportDataProvider + PatientExportProvider + GroupExportProvider
{
}

/// The default in-process export worker.
///
/// Binds a [`BulkExportJobStore`] (job state + claim + worker storage), an
/// [`ExportResourceProvider`] (the resource store), and an
/// [`ExportOutputStore`] (where NDJSON bytes go), and drives a claimed job to
/// completion under its lease.
pub struct DefaultExportWorker<Js: ?Sized, Dp: ?Sized, Os: ?Sized> {
    /// Job-state store (claim, worker storage, lifecycle).
    pub jobs: Arc<Js>,
    /// Resource data provider.
    pub data: Arc<Dp>,
    /// Output store for NDJSON parts.
    pub output: Arc<Os>,
    /// This worker's identifier.
    pub worker_id: WorkerId,
    /// Group-export `_since` toggle: when `true`, exclude resources from
    /// before `_since` for patients added to the Group after `_since`
    /// (using `Group.member.period.start`).
    pub exclude_since_newly_added: bool,
}

impl<Js, Dp, Os> DefaultExportWorker<Js, Dp, Os>
where
    Js: BulkExportJobStore + ?Sized,
    Dp: ExportResourceProvider + ?Sized,
    Os: ExportOutputStore + ?Sized,
{
    /// Creates a new worker (defaults to `since_newly_added=include`).
    pub fn new(jobs: Arc<Js>, data: Arc<Dp>, output: Arc<Os>, worker_id: WorkerId) -> Self {
        Self {
            jobs,
            data,
            output,
            worker_id,
            exclude_since_newly_added: false,
        }
    }

    /// Sets the `since_newly_added=exclude` toggle for Group exports.
    pub fn with_exclude_since_newly_added(mut self, exclude: bool) -> Self {
        self.exclude_since_newly_added = exclude;
        self
    }

    /// Runs the export job described by `lease` to completion.
    ///
    /// Every job-state mutation is fenced by `lease.worker_id` +
    /// `lease.fencing_token`; any `LeaseError::LeaseLost` aborts the run
    /// silently (the worker that reclaimed the job now owns it).
    pub async fn run_job(&self, lease: ExportJobLease) -> StorageResult<()> {
        match self.run_job_inner(&lease).await {
            Ok(()) => Ok(()),
            Err(LeaseError::LeaseLost { .. }) => {
                // Another worker owns the job now — stop silently.
                Ok(())
            }
            Err(LeaseError::Storage(e)) => {
                // Best-effort: mark the job failed (also fenced).
                let _ = self
                    .jobs
                    .fail_export_job(
                        &lease.tenant,
                        &lease.job_id,
                        &lease.worker_id,
                        lease.fencing_token,
                        &e.to_string(),
                    )
                    .await;
                Err(e)
            }
        }
    }

    async fn run_job_inner(&self, lease: &ExportJobLease) -> Result<(), LeaseError> {
        let tenant = &lease.tenant;
        let job_id = &lease.job_id;
        let wid = &lease.worker_id;
        let token = lease.fencing_token;

        let view = self
            .jobs
            .get_export_job_for_worker(tenant, job_id, wid, token)
            .await?;
        self.jobs
            .mark_export_in_progress(tenant, job_id, wid, token)
            .await?;

        let request = &view.request;

        // Resolve the resource types to export.
        let types = self
            .data
            .list_export_types(tenant, request)
            .await
            .map_err(LeaseError::Storage)?;

        // For Group exports, resolve the member patient IDs once.
        // When `exclude_since_newly_added` is set AND `_since` is provided,
        // filter out patients whose `Group.member.period.start` is *after*
        // `_since` (i.e., they joined the cohort after the client's last
        // sync) — the IG-recommended behavior under the `exclude` toggle.
        let group_patient_ids: Option<Vec<String>> = match &view.level {
            ExportLevel::Group { group_id } => {
                let ids = match (self.exclude_since_newly_added, view.request.since.as_ref()) {
                    (true, Some(since)) => {
                        let members = self
                            .data
                            .get_group_members_with_periods(tenant, group_id)
                            .await
                            .map_err(LeaseError::Storage)?;
                        members
                            .into_iter()
                            .filter_map(|(reference, period_start)| {
                                let pid = reference.strip_prefix("Patient/")?;
                                // Keep members whose period.start is unknown OR
                                // <= since (i.e., were already members at since).
                                match period_start {
                                    Some(start) if start > *since => None,
                                    _ => Some(pid.to_string()),
                                }
                            })
                            .collect()
                    }
                    _ => self
                        .data
                        .resolve_group_patient_ids(tenant, group_id)
                        .await
                        .map_err(LeaseError::Storage)?,
                };
                Some(ids)
            }
            _ => None,
        };

        let batch_size = request.batch_size.max(1);

        for resource_type in &types {
            // Resume from any persisted cursor for this type.
            let mut cursor: Option<String> = view
                .type_progress
                .iter()
                .find(|p| &p.resource_type == resource_type)
                .and_then(|p| p.cursor_state.clone());
            let mut exported: u64 = view
                .type_progress
                .iter()
                .find(|p| &p.resource_type == resource_type)
                .map(|p| p.exported_count)
                .unwrap_or(0);
            let mut part_index: u32 = 0;

            loop {
                // Cooperative cancellation check.
                if let Ok(progress) = self.jobs.get_export_status(tenant, job_id).await {
                    if progress.status == ExportStatus::Cancelled {
                        return Ok(());
                    }
                }

                let batch = match &group_patient_ids {
                    Some(pids) => self
                        .data
                        .fetch_patient_compartment_batch(
                            tenant,
                            request,
                            resource_type,
                            pids,
                            cursor.as_deref(),
                            batch_size,
                        )
                        .await
                        .map_err(LeaseError::Storage)?,
                    None if matches!(view.level, ExportLevel::Patient)
                        && !request.patient_refs.is_empty() =>
                    {
                        // Patient-level with specific patient filter: scope to
                        // exactly the requested patients' compartments.
                        let patient_ids: Vec<String> = request
                            .patient_refs
                            .iter()
                            .map(|r| r.strip_prefix("Patient/").unwrap_or(r).to_string())
                            .collect();
                        self.data
                            .fetch_patient_compartment_batch(
                                tenant,
                                request,
                                resource_type,
                                &patient_ids,
                                cursor.as_deref(),
                                batch_size,
                            )
                            .await
                            .map_err(LeaseError::Storage)?
                    }
                    None if matches!(view.level, ExportLevel::Patient) => {
                        // Patient-level without a patient filter: export all
                        // resources of this type across the patient compartment.
                        self.data
                            .fetch_export_batch(
                                tenant,
                                request,
                                resource_type,
                                cursor.as_deref(),
                                batch_size,
                            )
                            .await
                            .map_err(LeaseError::Storage)?
                    }
                    None => self
                        .data
                        .fetch_export_batch(
                            tenant,
                            request,
                            resource_type,
                            cursor.as_deref(),
                            batch_size,
                        )
                        .await
                        .map_err(LeaseError::Storage)?,
                };

                if !batch.lines.is_empty() {
                    let key = ExportPartKey::output(
                        tenant.tenant_id().as_str(),
                        job_id.clone(),
                        resource_type.clone(),
                        part_index,
                        token,
                    );
                    let mut writer = self
                        .output
                        .open_writer(&key)
                        .await
                        .map_err(LeaseError::Storage)?;
                    for line in &batch.lines {
                        let out_line = apply_elements(line, &request.elements);
                        writer.write_line(&out_line).await.map_err(|e| {
                            LeaseError::Storage(StorageError::Backend(
                                crate::error::BackendError::Internal {
                                    backend_name: "export-worker".to_string(),
                                    message: format!("write_line: {e}"),
                                    source: None,
                                },
                            ))
                        })?;
                    }
                    let finalized = self
                        .output
                        .finalize_part(&key, writer)
                        .await
                        .map_err(LeaseError::Storage)?;
                    exported += finalized.line_count;
                    self.jobs
                        .record_export_file(tenant, job_id, wid, token, &finalized, "output")
                        .await?;
                    part_index += 1;
                }

                cursor = batch.next_cursor.clone();

                // Persist progress + heartbeat after each batch.
                let mut progress = TypeExportProgress::new(resource_type.clone());
                progress.exported_count = exported;
                progress.cursor_state = cursor.clone();
                self.jobs
                    .update_export_type_progress(tenant, job_id, wid, token, &progress)
                    .await?;
                self.jobs.heartbeat(lease).await?;

                if batch.is_last {
                    break;
                }
            }
        }

        self.jobs
            .finish_export_job(tenant, job_id, wid, token)
            .await?;
        Ok(())
    }
}

/// Applies `_elements` projection to an NDJSON line.
///
/// When `elements` is non-empty, keeps `resourceType`, `id`, `meta` and the
/// listed top-level element names, and adds a `SUBSETTED` `meta.tag`. On any
/// parse failure the original line is returned unchanged.
fn apply_elements(line: &str, elements: &[String]) -> String {
    if elements.is_empty() {
        return line.to_string();
    }
    let Ok(serde_json::Value::Object(obj)) = serde_json::from_str::<serde_json::Value>(line) else {
        return line.to_string();
    };
    let mut out = serde_json::Map::new();
    // Always-included mandatory elements.
    for key in ["resourceType", "id"] {
        if let Some(v) = obj.get(key) {
            out.insert(key.to_string(), v.clone());
        }
    }
    // Requested top-level elements (strip a leading `ResourceType.` prefix).
    for el in elements {
        let name = el.rsplit('.').next().unwrap_or(el.as_str());
        if let Some(v) = obj.get(name) {
            out.insert(name.to_string(), v.clone());
        }
    }
    // meta + SUBSETTED tag.
    let mut meta = obj
        .get("meta")
        .and_then(|m| m.as_object().cloned())
        .unwrap_or_default();
    let tag = serde_json::json!({
        "system": "http://terminology.hl7.org/CodeSystem/v3-ObservationValue",
        "code": "SUBSETTED",
    });
    let tags = meta
        .entry("tag".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if let serde_json::Value::Array(arr) = tags {
        arr.push(tag);
    }
    out.insert("meta".to_string(), serde_json::Value::Object(meta));
    serde_json::to_string(&serde_json::Value::Object(out)).unwrap_or_else(|_| line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_elements_noop_when_empty() {
        let line = r#"{"resourceType":"Patient","id":"1","name":[]}"#;
        assert_eq!(apply_elements(line, &[]), line);
    }

    #[test]
    fn test_apply_elements_subsets_and_tags() {
        let line = r#"{"resourceType":"Patient","id":"1","name":[{"family":"X"}],"gender":"male"}"#;
        let out = apply_elements(line, &["name".to_string()]);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["resourceType"], "Patient");
        assert_eq!(v["id"], "1");
        assert!(v.get("name").is_some());
        assert!(v.get("gender").is_none());
        assert_eq!(v["meta"]["tag"][0]["code"], "SUBSETTED");
    }

    #[cfg(feature = "sqlite")]
    mod worker_integration {
        use super::*;
        use crate::backends::local_fs::LocalFsOutputStore;
        use crate::backends::sqlite::SqliteBackend;
        use crate::core::ResourceStorage;
        use crate::core::bulk_export::{ExportRequest, StartExportInput};
        use crate::tenant::{TenantContext, TenantId, TenantPermissions};
        use chrono::Utc;
        use std::sync::Arc;

        fn tenant() -> TenantContext {
            TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
        }

        #[tokio::test]
        async fn test_run_job_system_export_end_to_end() {
            let backend = Arc::new(SqliteBackend::in_memory().unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            for i in 0..3 {
                backend
                    .create(
                        &tenant,
                        "Patient",
                        serde_json::json!({"resourceType": "Patient", "id": format!("p{i}")}),
                        helios_fhir::FhirVersion::default(),
                    )
                    .await
                    .unwrap();
            }

            let tmp = tempfile::tempdir().unwrap();
            let output = Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080"));

            let job_id = backend
                .start_export(
                    &tenant,
                    StartExportInput {
                        request: ExportRequest::system()
                            .with_types(vec!["Patient".to_string()])
                            .with_batch_size(2),
                        transaction_time: Utc::now(),
                        request_url: "http://localhost/$export".to_string(),
                        owner_subject: Some("sub".to_string()),
                        fhir_version: helios_fhir::FhirVersion::default(),
                    },
                )
                .await
                .unwrap();

            let worker_id = WorkerId::new("w1");
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                worker_id.clone(),
            );

            let lease = backend
                .claim_next(&worker_id, Duration::from_secs(60))
                .await
                .unwrap()
                .expect("job claimable");
            assert_eq!(lease.job_id, job_id);

            worker.run_job(lease).await.unwrap();

            let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
            assert_eq!(progress.status, ExportStatus::Complete);

            let manifest = backend.get_export_manifest(&tenant, &job_id).await.unwrap();
            let total: u64 = manifest.output.iter().map(|e| e.count).sum();
            assert_eq!(total, 3);
        }
    }
}
