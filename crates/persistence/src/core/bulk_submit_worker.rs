//! Worker-facing traits for asynchronous Bulk Data **Submit** processing.
//!
//! The synchronous ingestion engine ([`BulkSubmitProvider`] and friends) does the
//! actual create/update/delete work. This module adds the missing async layer that
//! the FHIR Bulk Data Submit operation needs: a Data Consumer accepts a submission,
//! then a background worker **claims a pending manifest under a heartbeated,
//! fencing-token-guarded lease**, fetches the referenced files, and ingests them —
//! mirroring the bulk-export worker design ([`crate::core::bulk_export_worker`]).
//!
//! [`BulkSubmitProvider`]: crate::core::bulk_submit::BulkSubmitProvider

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use futures::stream::StreamExt;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use serde_json::{Value, json};

use crate::core::bulk_export_output::{ExportOutputStore, ExportPartKey};
use crate::core::bulk_export_worker::{LeaseError, WorkerId};
use crate::core::bulk_submit::{
    BulkEntryOutcome, BulkProcessingOptions, BulkSubmitProvider, BulkSubmitRollbackProvider,
    ByteProgress, CancelToken, EntryResultPage, ImportMode, ManifestPhase,
    StreamingBulkSubmitProvider, SubmissionId, SubmissionStatus, entry_result_pages,
};
use crate::core::bulk_submit_input::{RemoteFile, SubmitInputFetcher};
use crate::core::bulk_submit_output::submit_artifact_key;
use crate::core::bulk_submit_publication::{ManifestPublicationResult, ManifestPublicationStatus};
use crate::core::bulk_submit_receipts::{ReceiptSpools, SPOOL_BUFFER_BYTES, Spool};
use crate::error::{StorageError, StorageResult};
use crate::tenant::TenantContext;

/// What pushing a manifest's ingested resources into the deployment's
/// secondary search indexes achieved, reported before the receipt is written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IngestSyncReport {
    /// Resources every secondary accepted.
    pub synced: u64,
    /// Resources a secondary rejected after retries; their entry results
    /// are now `processing-error`.
    pub unindexed: u64,
    /// Resource types the manifest ingested whose tenant-wide count on the
    /// primary disagrees with a secondary's, checked right after the sync
    /// above. Always empty when secondaries are synced asynchronously (a
    /// count taken then would not reflect this manifest's sync) or when a
    /// backend's `count` call itself failed.
    pub drift: Vec<IndexDrift>,
}

/// A resource type whose tenant-wide count on the primary and on a
/// secondary search backend disagree after a manifest's sync.
///
/// This is a signal, not a fault attributable to any single entry: the
/// counts are tenant-wide, so a mismatch can also come from resources
/// outside this manifest (a concurrent write, a prior gap). It names a
/// backend and the two counts so an operator can decide whether to run
/// `$reindex`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexDrift {
    /// The resource type whose counts disagree.
    pub resource_type: String,
    /// The secondary backend whose count disagreed with the primary's.
    pub backend_id: String,
    /// The primary's tenant-wide count for `resource_type`.
    pub primary_count: u64,
    /// The secondary's tenant-wide count for `resource_type`.
    pub search_count: u64,
}

const MANIFEST_FETCH_ERROR_PART_INDEX: u32 = 1;

/// A lease over a single pending manifest, held by exactly one worker at a time.
///
/// Leases expire; if the holding worker does not heartbeat before `lease_expiry`,
/// the manifest is reclaimable. The `fencing_token` is bumped on every claim so a
/// zombie worker cannot mutate a manifest another worker now owns.
#[derive(Debug, Clone)]
pub struct ManifestLease {
    /// The tenant the submission belongs to.
    pub tenant: TenantContext,
    /// The submission this manifest belongs to.
    pub submission_id: SubmissionId,
    /// The leased manifest's ID (unique within the submission).
    pub manifest_id: String,
    /// The worker holding the lease.
    pub worker_id: WorkerId,
    /// When the lease expires if not renewed.
    pub lease_expiry: DateTime<Utc>,
    /// The duration the lease was claimed for; each heartbeat renews by this
    /// much, so short test leases stay short instead of jumping to a
    /// constant.
    pub lease_duration: Duration,
    /// Monotonically increasing token, bumped on every claim.
    pub fencing_token: u64,
}

impl ManifestLease {
    /// The expiry a successful heartbeat renews to: now plus the duration the
    /// lease was claimed with. Shared by every backend's `heartbeat`.
    pub fn renewed_expiry(&self) -> DateTime<Utc> {
        Utc::now()
            + chrono::Duration::from_std(self.lease_duration)
                .unwrap_or_else(|_| chrono::Duration::seconds(60))
    }
}

/// The worker's view of a claimed manifest: everything needed to fetch + ingest it.
#[derive(Debug, Clone)]
pub struct ManifestWorkerView {
    /// The manifest ID.
    pub manifest_id: String,
    /// The remote Bulk Export Manifest URL to fetch (None means nothing to do).
    pub manifest_url: Option<String>,
    /// Base URL for resolving relative references in ingested resources.
    pub fhir_base_url: Option<String>,
    /// The kickoff `outputFormat` (MIME), used to derive the FHIR version.
    pub output_format: Option<String>,
    /// HTTP headers the Data Provider asked us to include when fetching files.
    pub file_request_headers: Vec<(String, String)>,
    /// OAuth 2.0 metadata endpoints for acquiring file-retrieval tokens.
    pub oauth_metadata_urls: Vec<String>,
    /// JWE file-encryption key descriptor, if the provider encrypts files.
    pub file_encryption_key: Option<Value>,
    /// Pre-coordinated `import` processing directives as
    /// `(parameterUrl, parameterValue)` pairs. Resolve with
    /// [`ImportMode::from_directives`].
    pub import_directives: Vec<(String, String)>,
    /// Pre-coordinated `metadata` parts as `(parameterUrl, parameterValue)` pairs.
    ///
    /// The IG defines these as data the Data Provider passes to the Data Consumer
    /// without prescribing any processing; HFS retains them verbatim alongside the
    /// manifest and surfaces them to the worker (which logs them) so deployment- or
    /// IG-specific handling can be layered on without another migration.
    pub metadata: Vec<(String, String)>,
    /// Entries this manifest has already walked, across every run of it.
    ///
    /// Informational only — nothing resumes from it. A reclaimed manifest
    /// re-walks each of its files from the top and the re-ingested entries
    /// upsert idempotently; this cursor is the checkpoint a future per-line
    /// resume would read, but no such consumer exists yet. Counted like the
    /// other progress columns, so it only ever moves forward (#969).
    pub last_processed_line: u64,
    /// FHIR version this submission ingests against.
    pub fhir_version: FhirVersion,
}

/// The kickoff-supplied parameters a worker needs to fetch and ingest a manifest.
///
/// Bundled into a struct because they are written together by the kickoff handler
/// and read back together by [`SubmitWorkerStorage::get_manifest_for_worker`].
#[derive(Debug, Clone, Default)]
pub struct ManifestFetchParams<'a> {
    /// Base URL for resolving relative references in ingested resources.
    pub fhir_base_url: Option<&'a str>,
    /// The kickoff `outputFormat` (MIME), used to derive the FHIR version.
    pub output_format: Option<&'a str>,
    /// HTTP headers the Data Provider asked us to include when fetching files.
    pub file_request_headers: &'a [(String, String)],
    /// OAuth 2.0 metadata endpoints for acquiring file-retrieval tokens.
    pub oauth_metadata_urls: &'a [String],
    /// JWE file-encryption key descriptor, if the provider encrypts files.
    pub file_encryption_key: Option<&'a Value>,
    /// `import` directives as `(parameterUrl, parameterValue)` pairs.
    pub import_directives: &'a [(String, String)],
    /// `metadata` parts as `(parameterUrl, parameterValue)` pairs.
    pub metadata: &'a [(String, String)],
}

/// A finalized status-manifest artifact (output / error / deleted NDJSON part) to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitFileRecord {
    /// The submitted manifest this artifact relates to (for `output[].manifestUrl`).
    pub manifest_url: Option<String>,
    /// `output`, `error`, or `deleted`.
    pub file_type: String,
    /// FHIR resource type (for `output` entries).
    pub resource_type: Option<String>,
    /// 0-based part index within one manifest generation, file type, and
    /// optional resource type.
    pub part_index: u32,
    /// Encoded storage key / path in the output store.
    pub file_path: String,
    /// Number of NDJSON lines in the artifact.
    pub line_count: u64,
    /// Size of the artifact in bytes.
    pub byte_count: u64,
    /// `countSeverity` breakdown JSON (for `error` artifacts).
    pub count_severity: Option<Value>,
}

/// A persisted status-manifest artifact row, read back when building the status manifest.
#[derive(Debug, Clone)]
pub struct SubmitFileRow {
    /// The submitted manifest this artifact relates to.
    pub manifest_url: Option<String>,
    /// `output`, `error`, or `deleted`.
    pub file_type: String,
    /// FHIR resource type (for `output` entries).
    pub resource_type: Option<String>,
    /// 0-based part index.
    pub part_index: u32,
    /// Fencing token of the worker that wrote it.
    pub fencing_token: u64,
    /// Exact owning manifest when the persisted row is manifest-aware.
    ///
    /// Non-SQL rows that predate manifest-aware identities decode as `None`.
    /// SQL migration may backfill `Some` for a legacy row.
    pub manifest_id: Option<String>,
    /// True when the row uses the legacy pre-manifest identity/location.
    ///
    /// SQL backends identify migrated legacy rows by their absent publication
    /// worker; non-SQL backends identify them by an absent manifest ID.
    pub legacy_locator: bool,
    /// Encoded storage key / path in the output store.
    pub file_path: String,
    /// Number of NDJSON lines in the artifact.
    pub line_count: u64,
    /// Size of the artifact in bytes.
    pub byte_count: u64,
    /// `countSeverity` breakdown JSON (for `error` artifacts).
    pub count_severity: Option<Value>,
}

/// Resolution of a poll token to its owning submission, for REST status/cancel/file auth.
#[derive(Debug, Clone)]
pub struct PollTokenTarget {
    /// The tenant the submission belongs to.
    pub tenant: TenantContext,
    /// The submission identified by the token.
    pub submission_id: SubmissionId,
    /// The OAuth subject that kicked off the submission (for ownership checks).
    pub owner_subject: Option<String>,
}

/// Strategy for atomically claiming the next available pending manifest.
///
/// Each backend reaches for its native primitive — `SELECT … FOR UPDATE SKIP LOCKED`
/// on Postgres, a process-local mutex on SQLite.
#[async_trait]
pub trait SubmitClaimStrategy: Send + Sync {
    /// Atomically transitions one eligible manifest (`pending`, or `processing` with
    /// an expired lease) to held-by-this-worker, bumping the fencing token. Returns
    /// `Ok(None)` when no manifest is available.
    async fn claim_next_manifest(
        &self,
        worker_id: &WorkerId,
        lease_duration: Duration,
    ) -> StorageResult<Option<ManifestLease>>;

    /// Renews a lease the worker still holds; returns the new expiry, or
    /// `LeaseError::LeaseLost` if the manifest was reclaimed.
    async fn heartbeat(&self, lease: &ManifestLease) -> Result<DateTime<Utc>, LeaseError>;

    /// Releases a lease early (graceful shutdown). Best-effort.
    async fn release(&self, lease: ManifestLease) -> StorageResult<()>;
}

/// Worker-owned mutations of manifest/submission state.
///
/// The fenced methods (those taking a [`ManifestLease`]) verify `worker_id` +
/// `fencing_token`: a guarded mutation affecting zero rows returns
/// `LeaseError::LeaseLost`, so a zombie worker cannot corrupt progress, file rows,
/// or terminal status after its manifest has been reclaimed.
#[async_trait]
pub trait SubmitWorkerStorage: Send + Sync {
    /// Loads the claimed manifest's fetch parameters and resume cursor. Fenced.
    async fn get_manifest_for_worker(
        &self,
        lease: &ManifestLease,
    ) -> Result<ManifestWorkerView, LeaseError>;

    /// Marks the manifest `processing`. Fenced.
    async fn mark_manifest_processing(&self, lease: &ManifestLease) -> Result<(), LeaseError>;

    /// Adds to the manifest's progress counters. Fenced.
    ///
    /// The counters are **cumulative across every run of the manifest** and are
    /// owned by the ingestion engine: each committed batch adds its own entries
    /// atomically with the rows it wrote (`processed_entries += success +
    /// skipped`, `failed_entries += errors`, `last_processed_line +=
    /// entries`). The worker only contributes the deltas no batch can see —
    /// a file it could not fetch or could not ingest at all — so both writers
    /// share one semantics and a resumed manifest never walks its progress
    /// backwards (#969).
    ///
    /// Deltas are therefore *not* idempotent: re-ingesting an entry after a
    /// worker restart adds to `processed_entries` a second time, so the counts
    /// over-report on a resumed manifest until the re-walk is eliminated by a
    /// per-line resume. Over-reporting is the safe direction — a status poller sees
    /// progress that only ever moves forward.
    async fn add_manifest_progress(
        &self,
        lease: &ManifestLease,
        processed_delta: u64,
        failed_delta: u64,
        lines_delta: u64,
    ) -> Result<(), LeaseError>;

    /// Idempotent update of the manifest's byte progress — bytes consumed so
    /// far across its files, and the summed advertised size of the files
    /// opened so far (both monotonic within a run). What the status
    /// endpoint's percentage is computed from. Fenced.
    async fn update_manifest_bytes(
        &self,
        lease: &ManifestLease,
        bytes_processed: u64,
        bytes_total: u64,
    ) -> Result<(), LeaseError>;

    /// Records the coarse pre-ingest phase and its `files_done`/`files_total`
    /// pair, so the status endpoint has something to say before the byte and
    /// entry counters move (#953). Purely cosmetic — callers log and continue
    /// on storage errors — but fenced all the same, so a zombie worker cannot
    /// rewrite the phase of a manifest that was reclaimed under it.
    async fn update_manifest_phase(
        &self,
        lease: &ManifestLease,
        phase: ManifestPhase,
        files_done: u64,
        files_total: u64,
    ) -> Result<(), LeaseError>;

    /// Idempotent upsert of a finalized status-manifest artifact row. Fenced.
    ///
    /// SQL adapters stage the row and keep it hidden until publication; the
    /// `finish`/`fail` publication transaction makes the full generation visible.
    async fn record_submit_file(
        &self,
        lease: &ManifestLease,
        file: &SubmitFileRecord,
    ) -> Result<(), LeaseError>;

    /// Publishes the complete artifact set with the manifest's terminal state.
    ///
    /// SQL adapters perform canonical validation, full-set replacement, and the
    /// fenced terminal-state update in one transaction. A same-token replay must
    /// return [`ManifestPublicationResult::AlreadyPublished`] only when the exact
    /// artifact set and terminal state match; a stale lease is `LeaseLost`.
    ///
    /// MongoDB and S3 retain the existing fenced, non-atomic sequencing: validate
    /// the canonical set, record each artifact in turn, then write the terminal
    /// state. Those adapters do not provide atomic full-set publication or
    /// same-token replay markers.
    async fn publish_manifest_artifacts(
        &self,
        lease: &ManifestLease,
        files: &[SubmitFileRecord],
        terminal: ManifestPublicationStatus,
    ) -> Result<ManifestPublicationResult, LeaseError>;

    /// Marks the manifest `completed`. Fenced.
    async fn finish_manifest(&self, lease: &ManifestLease) -> Result<(), LeaseError>;

    /// Reclaims write-ahead-log space at a file boundary, when the backend
    /// keeps one (#978). SQLite's WAL grows without bound under a long ingest
    /// because its passive auto-checkpoint keeps yielding to the back-to-back
    /// batch writers; a multi-gigabyte WAL then slows every read (the status
    /// poll included) and doubles disk use. The worker calls this once per
    /// output file — a point where no batch holds the write lock — so the WAL
    /// is folded back into the database between files. Backends without a
    /// SQLite-style WAL (PostgreSQL, MongoDB, S3) leave the default no-op.
    async fn checkpoint_after_file(&self) {}

    /// Marks the manifest `failed` with a message. Fenced.
    async fn fail_manifest(
        &self,
        lease: &ManifestLease,
        error_message: &str,
    ) -> Result<(), LeaseError>;

    /// Pushes every resource this manifest ingested into the deployment's
    /// secondary search indexes, before the receipt is written, and marks the
    /// ones a secondary rejected as `processing-error`. Backends that index
    /// themselves have nothing to push and report an empty sync. Fenced.
    async fn sync_ingested(&self, lease: &ManifestLease) -> Result<IngestSyncReport, LeaseError> {
        let _ = lease;
        Ok(IngestSyncReport::default())
    }

    // ---- REST-facing (unfenced) submission/poll-token/artifact lifecycle ----

    /// Persists the remote-fetch parameters for a manifest that the kickoff handler
    /// added via [`crate::core::bulk_submit::BulkSubmitProvider::add_manifest`].
    ///
    /// `file_request_headers` / `oauth_metadata_urls` / `import_directives` /
    /// `metadata` are stored as JSON arrays; `file_encryption_key` as a JSON object.
    async fn set_manifest_fetch_params(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_id: &str,
        params: ManifestFetchParams<'_>,
    ) -> StorageResult<()>;

    /// Marks a previously-submitted manifest (identified by its submitted
    /// `manifest_url`) as `replaced`, for `replacesManifestUrl` handling. Returns the
    /// `manifest_id`s that were superseded (so the caller can roll back their changes).
    async fn replace_manifest_by_url(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_url: &str,
    ) -> StorageResult<Vec<String>>;

    /// Persists kickoff metadata (owner subject, request URL, access-token posture).
    async fn set_submission_kickoff_meta(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        owner_subject: Option<&str>,
        request_url: &str,
        requires_access_token: bool,
    ) -> StorageResult<()>;

    /// Idempotently mints + stores a poll token on the submission (UNIQUE index
    /// guards collisions). Returns the existing token if one is already set.
    async fn ensure_poll_token(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<String>;

    /// Resolves a poll token to its submission, or `None` after deletion/unknown.
    async fn resolve_poll_token(&self, token: &str) -> StorageResult<Option<PollTokenTarget>>;

    /// Clears the poll token so subsequent [`Self::resolve_poll_token`] returns `None`.
    async fn clear_poll_token(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()>;

    /// Lists the recorded status-manifest artifact rows for a submission.
    async fn list_submit_files(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Vec<SubmitFileRow>>;

    /// Deletes the `bulk_submit_files` rows for a submission (idempotent).
    ///
    /// **Note:** the persistence layer holds no reference to the output store, so
    /// removing the underlying NDJSON objects is orchestrated by the caller (REST
    /// DELETE handler / TTL cleanup task), which lists files via
    /// [`Self::list_submit_files`], deletes them from the output store, then calls
    /// this to drop the rows.
    async fn delete_submission_artifacts(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()>;

    /// Counts submissions with work in flight for a tenant (per-tenant
    /// concurrency cap).
    ///
    /// A submission counts while it is `in-progress` **and** still has work
    /// pending: either no manifests yet (opened, awaiting its first kick-off)
    /// or at least one non-terminal manifest. A drained submission — open per
    /// the spec so `replacesManifestUrl` can still land, but with every
    /// manifest terminal — holds no slot: the cap bounds concurrent
    /// ingestion, and a submitter that never sends the closing
    /// `submissionStatus=completed` must not leak its tenant's slots forever
    /// (#850). A new manifest on a drained submission makes it count again.
    async fn count_active_submissions(&self, tenant: &TenantContext) -> StorageResult<u64>;

    /// Lists submissions whose `updated_at` is older than `now - ttl`, across all
    /// tenants, for the periodic cleanup task. Returns `(tenant, submission_id)`
    /// pairs (bounded by `limit`) so the caller can delete their output-store
    /// artifacts and rows.
    async fn list_expired_submissions(
        &self,
        now: DateTime<Utc>,
        ttl: Duration,
        limit: u32,
    ) -> StorageResult<Vec<(TenantContext, SubmissionId)>>;

    /// Records a transaction time on the submission when its status manifest is first
    /// finalized (idempotent — only sets if currently unset).
    async fn ensure_transaction_time(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<DateTime<Utc>>;
}

/// Marker trait composing the submit job-state surfaces a worker + REST layer needs.
///
/// Only the SQLite and Postgres backends implement this; it is held as an
/// `Arc<dyn BulkSubmitJobStore>` and selected at bootstrap by `HFS_BULK_SUBMIT_BACKEND`.
pub trait BulkSubmitJobStore:
    BulkSubmitProvider
    + StreamingBulkSubmitProvider
    + BulkSubmitRollbackProvider
    + SubmitWorkerStorage
    + SubmitClaimStrategy
{
}

impl<T> BulkSubmitJobStore for T where
    T: BulkSubmitProvider
        + StreamingBulkSubmitProvider
        + BulkSubmitRollbackProvider
        + SubmitWorkerStorage
        + SubmitClaimStrategy
{
}

/// Rebuilds deferred search indexes once a manifest finishes ingesting
/// (bulk fast-load, #903). Implemented over the reindex machinery by the
/// server wiring; fire-and-forget from the worker's perspective.
#[async_trait]
pub trait DeferredReindexHook: Send + Sync {
    /// Kicks a reindex of the given resource types for the tenant.
    async fn reindex_types(&self, tenant: &TenantContext, resource_types: Vec<String>);
}

/// The default in-process submit worker.
///
/// Binds a [`BulkSubmitJobStore`] (job state + claim + worker storage + ingestion
/// engine), a [`SubmitInputFetcher`] (remote manifest + NDJSON fetch), and an
/// [`ExportOutputStore`] (where status-manifest artifacts go), and drives a claimed
/// manifest to completion: fetch → ingest each `output` file via the existing
/// `process_ndjson_stream` engine → sync ingested resources to secondary search
/// indexes ([`SubmitWorkerStorage::sync_ingested`], #1007) → emit `output`/`error`
/// artifacts → finish.
pub struct DefaultSubmitWorker<Js: ?Sized, Fetcher: ?Sized, Os: ?Sized> {
    jobs: Arc<Js>,
    fetcher: Arc<Fetcher>,
    output: Arc<Os>,
    #[allow(dead_code)]
    worker_id: WorkerId,
    /// Bulk fast-load (#903): ingest without search indexing, then reindex.
    defer_indexing: bool,
    /// Rebuilds the deferred indexes after each finished manifest. Without a
    /// hook, deferred mode still ingests and logs that $reindex is owed.
    reindex_hook: Option<Arc<dyn DeferredReindexHook>>,
    /// How many of a manifest's `output` files to ingest at once (#fan-out).
    /// `1` keeps the historical sequential behavior. Higher values overlap
    /// per-file fetch, parse, and write, which a concurrent-writer backend
    /// (PostgreSQL) turns into near-linear throughput; SQLite's single writer
    /// caps the gain but still benefits from overlapped fetch and extraction.
    file_concurrency: usize,
}

/// A pass-through [`AsyncBufRead`] that adds every consumed byte to a shared
/// counter — the live half of the status endpoint's percentage.
struct CountingReader {
    inner: Box<dyn tokio::io::AsyncBufRead + Send + Unpin>,
    consumed: Arc<std::sync::atomic::AtomicU64>,
}

impl tokio::io::AsyncRead for CountingReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &poll {
            let read = buf.filled().len() - before;
            self.consumed.fetch_add(read as u64, Ordering::Relaxed);
        }
        poll
    }
}

impl tokio::io::AsyncBufRead for CountingReader {
    fn poll_fill_buf(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<&[u8]>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_fill_buf(cx)
    }

    fn consume(mut self: std::pin::Pin<&mut Self>, amt: usize) {
        self.consumed.fetch_add(amt as u64, Ordering::Relaxed);
        std::pin::Pin::new(&mut self.inner).consume(amt);
    }
}

/// Renews a [`ManifestLease`] from a dedicated task for as long as it is alive.
///
/// The renewal must not share a future with the ingestion: a fast local stream
/// keeps the ingest future Ready-heavy enough that a sibling `select!` timer arm
/// is polled too rarely to ever fire, the lease silently expires mid-file, and
/// the manifest is reclaimed and restarted from its first file — an unbounded
/// loop, invisible in the counts because re-ingested entries upsert
/// idempotently. A separately spawned task renews on schedule no matter how the
/// ingest future behaves.
///
/// The keeper's task also flushes the shared byte progress every few seconds,
/// best-effort. On backends whose batch bookkeeping persists the counters
/// itself (SQLite) this is only a fallback — a standalone write can starve
/// for tens of seconds behind back-to-back batch transactions there, and the
/// value it finally lands is as old as the wait. All byte writes are
/// monotonic (`MAX`), so a stale flush can never walk progress backwards.
/// The heartbeat alone decides lease health.
///
/// A lease that cannot be renewed before it expires is fatal to the run that
/// holds it (#969). A heartbeat starved behind the ingest loop's writes used to
/// be retried indefinitely, so the manifest stayed claimable — eligible for
/// `claim_next_manifest` by a second worker — while this one kept committing
/// batches: silent duplicate ingestion on a cluster. Each renewal is now bounded
/// by what is left of the lease, and once `lease_expiry` passes unrenewed the
/// keeper declares the lease lost, which aborts the run mid-file.
///
/// On the same schedule the keeper re-reads the submission's status and trips
/// the ingest's [`CancelToken`] once it stops being ingestable (#968). The
/// keeper is the only part of a running job that touches the database on a
/// fixed schedule no matter what the ingest is doing, which makes it the one
/// place an abort can be noticed promptly; nothing else in the loop would look
/// until the current file ended.
///
/// Dropping the keeper stops the renewal task.
struct LeaseKeeper {
    /// Held rather than only subscribed to, so `run_job` can declare the lease
    /// lost too when a fenced write answers `LeaseLost`.
    lost: tokio::sync::watch::Sender<bool>,
    cancel: CancelToken,
    handle: tokio::task::JoinHandle<()>,
}

/// The slice of a job store the [`LeaseKeeper`] uses.
///
/// Narrow on purpose: a keeper that must be handed a whole [`BulkSubmitJobStore`]
/// can only be exercised against a real backend, and the failure this guards —
/// a heartbeat that never lands — is exactly the one a real backend will not
/// reproduce on demand.
#[async_trait]
trait LeaseRenewal: Send + Sync {
    /// Renews the lease, returning its new expiry.
    async fn heartbeat(&self, lease: &ManifestLease) -> Result<DateTime<Utc>, LeaseError>;

    /// Persists byte progress, best-effort — failures are not lease-relevant.
    async fn flush_bytes(&self, lease: &ManifestLease, consumed: u64, total: u64);

    /// Re-reads the status of the submission the lease belongs to (#968).
    ///
    /// Here rather than on the ingest path because the keeper is the only part
    /// of a running job that reaches storage on a fixed schedule, so it is the
    /// only place an abort can be noticed promptly. The three outcomes are kept
    /// apart deliberately — found, missing, and unreadable mean different
    /// things to [`watch_submission`], and only the first can stop a job.
    async fn submission_status(
        &self,
        lease: &ManifestLease,
    ) -> StorageResult<Option<SubmissionStatus>>;
}

/// Adapts a job store to the keeper's narrow surface.
struct JobStoreRenewal<Js: ?Sized>(Arc<Js>);

#[async_trait]
impl<Js> LeaseRenewal for JobStoreRenewal<Js>
where
    Js: BulkSubmitJobStore + ?Sized + 'static,
{
    async fn heartbeat(&self, lease: &ManifestLease) -> Result<DateTime<Utc>, LeaseError> {
        self.0.heartbeat(lease).await
    }

    async fn flush_bytes(&self, lease: &ManifestLease, consumed: u64, total: u64) {
        let _ = self.0.update_manifest_bytes(lease, consumed, total).await;
    }

    async fn submission_status(
        &self,
        lease: &ManifestLease,
    ) -> StorageResult<Option<SubmissionStatus>> {
        Ok(self
            .0
            .get_submission(&lease.tenant, &lease.submission_id)
            .await?
            .map(|summary| summary.status))
    }
}

impl LeaseKeeper {
    fn spawn<R>(
        jobs: Arc<R>,
        lease: ManifestLease,
        progress: ByteProgress,
        cancel: CancelToken,
    ) -> Self
    where
        R: LeaseRenewal + ?Sized + 'static,
    {
        const FLUSH_EVERY: Duration = Duration::from_secs(3);
        /// Pause between renewal attempts after a storage error. Short, because
        /// the whole retry window is capped by the lease's remaining life.
        const RETRY_AFTER: Duration = Duration::from_millis(500);
        let (lost, _) = tokio::sync::watch::channel(false);
        let flag = lost.clone();
        let watched = cancel.clone();
        let handle = tokio::spawn(async move {
            let mut expiry = lease.lease_expiry;
            let mut last_flushed: u64 = 0;
            loop {
                let remaining = (expiry - Utc::now())
                    .to_std()
                    .unwrap_or(Duration::from_secs(1));
                let wait = (remaining / 3).clamp(Duration::from_secs(1), Duration::from_secs(60));
                let heartbeat_at = tokio::time::Instant::now() + wait;
                loop {
                    let now = tokio::time::Instant::now();
                    if now >= heartbeat_at {
                        break;
                    }
                    tokio::time::sleep(FLUSH_EVERY.min(heartbeat_at - now)).await;
                    let total = progress.total.load(Ordering::Relaxed);
                    let consumed = progress.consumed.load(Ordering::Relaxed);
                    if total > 0 && consumed != last_flushed {
                        last_flushed = consumed;
                        jobs.flush_bytes(&lease, consumed, total).await;
                    }
                    // Watch for an abort on the flush cadence rather than the
                    // (up to a minute) heartbeat cadence: a lease renewal is
                    // cheap to defer, a user waiting for Abort to do something
                    // is not. One indexed row read every few seconds per
                    // running manifest.
                    if !watched.is_cancelled() {
                        watch_submission(jobs.as_ref(), &lease, &watched).await;
                    }
                }
                // Renew, retrying only for as long as the lease still covers
                // the writes the ingest loop is making in parallel. A renewal
                // that has not landed by `expiry` is indistinguishable from a
                // lost one: the manifest is claimable either way.
                let mut renewed = None;
                loop {
                    let left = (expiry - Utc::now())
                        .to_std()
                        .unwrap_or(Duration::from_secs(0));
                    if left.is_zero() {
                        break;
                    }
                    match tokio::time::timeout(left, jobs.heartbeat(&lease)).await {
                        Ok(Ok(new_expiry)) => {
                            renewed = Some(new_expiry);
                            break;
                        }
                        // Already reclaimed by another worker — expected, and
                        // the run aborts quietly.
                        Ok(Err(LeaseError::LeaseLost { .. })) => {
                            let _ = flag.send(true);
                            return;
                        }
                        Ok(Err(LeaseError::Storage(e))) => {
                            tracing::debug!(
                                submission = %lease.submission_id,
                                manifest = %lease.manifest_id,
                                error = %e,
                                "bulk-submit lease heartbeat failed; retrying"
                            );
                            tokio::time::sleep(RETRY_AFTER.min(left)).await;
                        }
                        // Starved behind the ingest loop's writer for the rest
                        // of the lease.
                        Err(_elapsed) => break,
                    }
                }
                let Some(new_expiry) = renewed else {
                    tracing::warn!(
                        submission = %lease.submission_id,
                        manifest = %lease.manifest_id,
                        worker = %lease.worker_id,
                        "bulk-submit lease could not be renewed before it expired; \
                         abandoning the manifest so it can be reclaimed"
                    );
                    let _ = flag.send(true);
                    return;
                };
                expiry = new_expiry;
            }
        });
        Self {
            lost,
            cancel,
            handle,
        }
    }

    /// Whether the job in flight should wind down: either the lease is gone or
    /// the submission stopped being ingestable (#968). Both exit the same way —
    /// leave the manifest alone and let whoever owns its outcome record it.
    fn should_stop(&self) -> bool {
        *self.lost.borrow() || self.cancel.is_cancelled()
    }

    /// Whether the stop is an abort rather than a lost lease. The two exit
    /// identically but read very differently in an operator's log.
    fn cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Marks the lease lost from outside the renewal task — used when a fenced
    /// write answers `LeaseLost`, which is as conclusive as a failed heartbeat.
    fn declare_lost(&self) {
        let _ = self.lost.send(true);
    }

    /// Resolves once the lease is lost, and never otherwise. Raced against the
    /// ingest so a loss aborts mid-file rather than only between files.
    ///
    /// Lease loss only: an abort needs no race here, because its token is
    /// checked between batches *inside* the ingest, so the files wind
    /// themselves down and the fan-out ends normally (#968). Resolving this on
    /// a cancel too would report an aborted run as one whose lease was lost.
    async fn lost(&self) {
        let mut rx = self.lost.subscribe();
        while !*rx.borrow_and_update() {
            if rx.changed().await.is_err() {
                // Only reachable if the sender is dropped, which cannot happen
                // while `self` is alive.
                return;
            }
        }
    }
}

impl Drop for LeaseKeeper {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Trips `cancel` when the submission behind `lease` has stopped being
/// ingestable, so an abort reaches the manifest already in flight and not only
/// future claims (#968).
///
/// The admitted set mirrors `claim_next_manifest`'s: `complete` means the
/// submitter will send no further manifests, not that the registered ones
/// should be dropped, so only `aborted` stops the ingest. A submission that
/// reads back as missing is left alone — the lease machinery already covers
/// deletion, and a transient read is not worth throwing away a running job
/// over. Storage errors likewise never cancel: an unreachable database must
/// not look like an abort.
async fn watch_submission<R>(jobs: &R, lease: &ManifestLease, cancel: &CancelToken)
where
    R: LeaseRenewal + ?Sized,
{
    match jobs.submission_status(lease).await {
        Ok(Some(status))
            if !matches!(
                status,
                SubmissionStatus::InProgress | SubmissionStatus::Complete
            ) =>
        {
            tracing::info!(
                submission = %lease.submission_id,
                manifest = %lease.manifest_id,
                %status,
                "bulk-submit submission is no longer ingestable; stopping the manifest in flight"
            );
            cancel.cancel();
        }
        Ok(_) => {}
        Err(e) => {
            tracing::debug!(
                submission = %lease.submission_id,
                manifest = %lease.manifest_id,
                error = %e,
                "bulk-submit submission status check failed; retrying"
            );
        }
    }
}

/// Logs a claimed manifest winding down before its natural end.
///
/// A cancelled manifest is deliberately left untouched: `abort_submission`
/// already moved it out of `processing`, and marking it here would either
/// fight that write or fabricate a terminal state for work that simply
/// stopped. Its partial counts stay recorded.
fn log_wind_down(lease: &ManifestLease, cancelled: bool) {
    if cancelled {
        tracing::info!(
            submission = %lease.submission_id,
            manifest = %lease.manifest_id,
            "bulk-submit manifest stopped by abort; partial counts kept, no result artifacts written"
        );
    }
}

impl<Js, Fetcher, Os> DefaultSubmitWorker<Js, Fetcher, Os>
where
    Js: BulkSubmitJobStore + ?Sized + 'static,
    Fetcher: SubmitInputFetcher + ?Sized,
    Os: ExportOutputStore + ?Sized,
{
    /// Creates a new worker bound to the given job store, fetcher, and output store.
    pub fn new(jobs: Arc<Js>, fetcher: Arc<Fetcher>, output: Arc<Os>, worker_id: WorkerId) -> Self {
        Self {
            jobs,
            fetcher,
            output,
            worker_id,
            defer_indexing: false,
            reindex_hook: None,
            file_concurrency: 1,
        }
    }

    /// Sets how many of a manifest's `output` files ingest concurrently
    /// (see [`Self::file_concurrency`]). Values below 1 are treated as 1.
    pub fn with_file_concurrency(mut self, concurrency: usize) -> Self {
        self.file_concurrency = concurrency.max(1);
        self
    }

    /// Enables bulk fast-load: entries ingest without search-index/FTS
    /// writes, and the hook (when set) rebuilds them per finished manifest.
    pub fn with_deferred_indexing(
        mut self,
        defer: bool,
        hook: Option<Arc<dyn DeferredReindexHook>>,
    ) -> Self {
        self.defer_indexing = defer;
        self.reindex_hook = hook;
        self
    }

    /// Drives a single claimed manifest to a terminal state.
    ///
    /// Returns `Ok(())` for both successful ingestion and *recorded* manifest-level
    /// failures (a bad remote manifest fails only that manifest, not the worker).
    /// Only `LeaseError::LeaseLost` aborts early; storage errors propagate.
    pub async fn run_job(&self, lease: ManifestLease) -> StorageResult<()> {
        let view = match self.jobs.get_manifest_for_worker(&lease).await {
            Ok(v) => v,
            Err(LeaseError::LeaseLost { .. }) => return Ok(()),
            Err(LeaseError::Storage(e)) => return Err(e),
        };
        match self.jobs.mark_manifest_processing(&lease).await {
            Ok(()) => {}
            Err(LeaseError::LeaseLost { .. }) => return Ok(()),
            Err(LeaseError::Storage(e)) => return Err(e),
        }

        let Some(manifest_url) = view.manifest_url.clone() else {
            // Nothing to fetch (status-only submission). Publish its empty
            // terminal generation with the live lease.
            let keeper = LeaseKeeper::spawn(
                Arc::new(JobStoreRenewal(Arc::clone(&self.jobs))),
                lease.clone(),
                ByteProgress::default(),
                CancelToken::new(),
            );
            let _ = self
                .publish_collected_artifacts(
                    &lease,
                    keeper,
                    Vec::new(),
                    ManifestPublicationStatus::Completed,
                )
                .await?;
            return Ok(());
        };

        let progress = ByteProgress::default();
        // Abort is cooperative and means "stop soon": the keeper trips this
        // token when the submission stops being ingestable, the ingest loops
        // check it between batches, and this job checks it between files. Every
        // per-file clone of `opts` shares the one token (#968).
        let cancel = CancelToken::new();
        // The lease must stay heartbeated *through* a file, not only between
        // files, and independently of how often the ingest future yields; the
        // keeper renews from its own task until dropped.
        //
        // Spawned here rather than after pre-sizing (#953): fetching the remote
        // manifest and HEAD-ing its output files can take minutes on a
        // file-heavy corpus, and that whole window used to pass without a
        // single heartbeat. The keeper only *flushes bytes* once
        // `progress.total` is non-zero, so starting it early adds heartbeats
        // and nothing else.
        //
        // It also starts the abort watch that much earlier, which those same
        // minutes are the reason to want: an operator who gives up during
        // `reading manifest` or `sizing` now has the token tripped before the
        // first file is opened, so the ingest stops at its first batch check
        // instead of running the whole corpus first (#968).
        let keeper = LeaseKeeper::spawn(
            Arc::new(JobStoreRenewal(Arc::clone(&self.jobs))),
            lease.clone(),
            progress.clone(),
            cancel.clone(),
        );

        // 1. Fetch the remote Bulk Export Manifest.
        self.report_phase(&lease, ManifestPhase::ReadingManifest, 0, 0)
            .await;
        let manifest = match self
            .fetcher
            .fetch_manifest(
                &manifest_url,
                &view.file_request_headers,
                &view.oauth_metadata_urls,
                view.file_encryption_key.as_ref(),
            )
            .await
        {
            Ok(m) => m,
            Err(e) => {
                let failure_message = e.to_string();
                // Whatever a prior run of this manifest already ingested must
                // stay searchable even though this run fails outright — sync
                // it before the manifest goes terminal, while the lease (and
                // this heartbeat) still covers the write. Best-effort: the
                // manifest fails either way, and a miss here is repaired by
                // $reindex.
                if let Err(e) = self.jobs.sync_ingested(&lease).await {
                    tracing::warn!(
                        submission = %lease.submission_id,
                        manifest = %lease.manifest_id,
                        error = %e,
                        "bulk-submit: failed to sync ingested resources before failing the \
                         manifest on a fetch error"
                    );
                }
                let fetch_error = self
                    .write_manifest_error(
                        &lease,
                        &manifest_url,
                        MANIFEST_FETCH_ERROR_PART_INDEX,
                        &format!("failed to fetch manifest: {failure_message}"),
                    )
                    .await?;
                let _ = self
                    .publish_collected_artifacts(
                        &lease,
                        keeper,
                        vec![fetch_error],
                        ManifestPublicationStatus::Failed {
                            error_message: failure_message,
                        },
                    )
                    .await?;
                return Ok(());
            }
        };

        // Apply the pre-coordinated `import` directives the Data Provider sent at
        // kickoff. `metadata` carries no processing semantics of its own — it is
        // retained with the manifest and surfaced here for operators.
        let import_mode = ImportMode::from_directives(&view.import_directives);
        if !view.metadata.is_empty() {
            tracing::info!(
                submission = %lease.submission_id,
                manifest = %lease.manifest_id,
                metadata = ?view.metadata,
                "ingesting manifest with submission metadata"
            );
        }
        let opts = BulkProcessingOptions::new()
            .with_import_mode(import_mode)
            .with_defer_indexing(self.defer_indexing)
            .with_byte_progress(progress.clone())
            .with_cancel(cancel.clone());
        let file_count = manifest.output.len() as u64;
        // Reserve 0 for the aggregated entry-error artifact and 1 for the
        // manifest-fetch failure. Per-file output/deleted indexes follow their
        // input ordinal so concurrent completion order cannot renumber them.
        let output_len = u64::try_from(manifest.output.len()).ok();
        let deleted_len = u64::try_from(manifest.deleted.len()).ok();
        let artifact_count = output_len
            .and_then(|output| deleted_len.and_then(|deleted| output.checked_add(deleted)))
            .and_then(|files| files.checked_add(u64::from(MANIFEST_FETCH_ERROR_PART_INDEX + 1)));
        match artifact_count {
            Some(count) if count <= i32::MAX as u64 => {}
            Some(count) => {
                return Err(internal_error(format!(
                    "bulk submit manifest has too many artifacts: {count}"
                )));
            }
            None => {
                return Err(internal_error(
                    "bulk submit manifest has too many artifacts".to_string(),
                ));
            }
        }
        // Pre-size the byte denominator: every output file's advertised size
        // up front, so the percentage never recomputes against a partial
        // total — learned lazily per file, each newly opened file yanked the
        // bar backwards on multi-file manifests (#874). Encrypted files are
        // skipped (wire length ≠ decrypted length); any unknown size falls
        // back to lazy accumulation below.
        //
        // The HEADs run `file_concurrency` at a time, like the ingest fan-out
        // below: serially, a manifest with hundreds of outputs spent hundreds
        // of sequential round trips here before the first byte was read (#953).
        let mut presized = false;
        if view.file_encryption_key.is_none() && !manifest.output.is_empty() {
            self.report_phase(&lease, ManifestPhase::Sizing, 0, file_count)
                .await;
            let sum = AtomicU64::new(0);
            let all_known = AtomicBool::new(true);
            let sized = AtomicU64::new(0);
            let sum_ref = &sum;
            let all_known_ref = &all_known;
            let sized_ref = &sized;
            let view_ref = &view;
            let lease_ref = &lease;
            let manifest_ref = &manifest;

            // Indexed like the ingest fan-out below, so the closure's argument
            // stays owned and the future may borrow `manifest.output[i]`.
            let mut sizing = futures::stream::iter(0..manifest.output.len())
                .map(|i| async move {
                    // One unknown size poisons the total anyway, so stop
                    // spending round trips on the queued remainder. The
                    // in-flight ones still finish; their results are discarded.
                    if !all_known_ref.load(Ordering::Relaxed) {
                        return;
                    }
                    let file = &manifest_ref.output[i];
                    match self
                        .fetcher
                        .file_size(
                            &file.url,
                            &view_ref.file_request_headers,
                            manifest_ref.requires_access_token,
                            &view_ref.oauth_metadata_urls,
                        )
                        .await
                    {
                        Ok(Some(len)) => {
                            sum_ref.fetch_add(len, Ordering::Relaxed);
                        }
                        _ => {
                            all_known_ref.store(false, Ordering::Relaxed);
                            return;
                        }
                    }
                    // A fenced UPDATE per HEAD, which is cheap next to the
                    // network round trip it follows — and it is the only thing
                    // the status endpoint can report during this window.
                    let done = sized_ref.fetch_add(1, Ordering::Relaxed) + 1;
                    self.report_phase(lease_ref, ManifestPhase::Sizing, done, file_count)
                        .await;
                })
                .buffer_unordered(self.file_concurrency.max(1));
            while sizing.next().await.is_some() {}
            drop(sizing);

            // A zero sum (every file empty, or a source misreporting sizes)
            // presizes nothing — the lazy path below stays the authority.
            let sum = sum.load(Ordering::Relaxed);
            if all_known.load(Ordering::Relaxed) && sum > 0 {
                progress.total.store(sum, Ordering::Relaxed);
                presized = true;
            }
        }
        // Percentages need every file's size; one sizeless file (e.g. a
        // gzip-decompressed stream) poisons the total for the whole manifest
        // and the status endpoint falls back to manifest-count progress.
        let totals_known = AtomicBool::new(true);
        // Files whose download has *started*, for the `downloading file N of M`
        // report. Approximate by construction when `file_concurrency > 1` —
        // several files are in flight at once — which is fine for a coarse
        // "it is moving" signal that the byte counters take over from.
        let opened = AtomicU64::new(0);

        // 2. Ingest the `output` files. Up to `file_concurrency` at a time run
        // concurrently (fan-out): each file's fetch, parse, and write overlaps
        // the others', which a concurrent-writer backend turns into throughput.
        // The manifest's files carry disjoint resource types, so their entry
        // receipts and rollback records never collide. A file's own failures
        // are recorded and counted without aborting the manifest, exactly as
        // the sequential loop did, and only a storage error on the bookkeeping
        // path aborts the job.
        //
        // This tally is run-local and feeds the status artifacts only. The
        // manifest's persisted counters are cumulative across runs and belong
        // to the ingestion engine's per-batch bookkeeping (#969).
        let failed_at = AtomicU64::new(0);
        // File-level fetch/ingest failures write their own finalized artifacts.
        // No staged row exists; all finalized records are collected and handed
        // to one publication call after the whole run.
        let error_records = Arc::new(tokio::sync::Mutex::new(Vec::<SubmitFileRecord>::new()));
        // Shared borrows for the concurrent per-file futures. Iterating by
        // index keeps the map closure's argument owned (a `usize`), so the
        // future it returns can borrow `manifest.output[i]` for the manifest's
        // lifetime without a higher-ranked-lifetime bound the closure can't name.
        let failed_ref = &failed_at;
        let opened_ref = &opened;
        let totals_ref = &totals_known;
        let progress_ref = &progress;
        let keeper_ref = &keeper;
        let view_ref = &view;
        let opts_ref = &opts;
        let lease_ref = &lease;
        let manifest_ref = &manifest;
        let manifest_url_ref = &manifest_url;
        let error_records_ref = &error_records;

        let mut ingest = futures::stream::iter(0..manifest.output.len())
            .map(|i| async move {
                if keeper_ref.should_stop() {
                    return Ok::<(), StorageError>(());
                }
                let file = &manifest_ref.output[i];
                let resource_type = file
                    .resource_type
                    .clone()
                    .unwrap_or_else(|| "Resource".into());
                // Opening a file can itself be a long wait (the remote has to
                // start streaming), and until its first batch commits nothing
                // else moves — so say which file is being pulled (#953).
                let nth = opened_ref.fetch_add(1, Ordering::Relaxed) + 1;
                self.report_phase(lease_ref, ManifestPhase::Downloading, nth, file_count)
                    .await;
                let (inner, file_bytes_total) = match self
                    .fetcher
                    .open_file_stream(
                        &file.url,
                        &view_ref.file_request_headers,
                        manifest_ref.requires_access_token,
                        &view_ref.oauth_metadata_urls,
                        view_ref.file_encryption_key.as_ref(),
                    )
                    .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        let file_error = self
                            .write_manifest_error(
                                lease_ref,
                                manifest_url_ref,
                                i as u32 + 2,
                                &format!("failed to fetch file {}: {e}", file.url),
                            )
                            .await?;
                        error_records_ref.lock().await.push(file_error);
                        failed_ref.fetch_add(1, Ordering::Relaxed);
                        // No batch ran for a file that never opened, so this
                        // failure is the worker's to add.
                        if let Err(e) = self.jobs.add_manifest_progress(lease_ref, 0, 1, 0).await {
                            return fenced_write_outcome(keeper_ref, e);
                        }
                        return Ok(());
                    }
                };
                if !presized {
                    match file_bytes_total {
                        Some(len) if totals_ref.load(Ordering::Relaxed) => {
                            progress_ref.total.fetch_add(len, Ordering::Relaxed);
                        }
                        Some(_) => {}
                        None => {
                            totals_ref.store(false, Ordering::Relaxed);
                            progress_ref.total.store(0, Ordering::Relaxed);
                        }
                    }
                }
                let stream: Box<dyn tokio::io::AsyncBufRead + Send + Unpin> =
                    Box::new(CountingReader {
                        inner,
                        consumed: Arc::clone(&progress_ref.consumed),
                    });

                // Per-file options: the file url is part of every entry result's
                // identity, since line numbers restart in each file (#457).
                let file_opts = opts_ref.clone().with_file_url(&file.url);
                match self
                    .jobs
                    .process_ndjson_stream(
                        &lease_ref.tenant,
                        &lease_ref.submission_id,
                        &lease_ref.manifest_id,
                        &resource_type,
                        stream,
                        &file_opts,
                    )
                    .await
                {
                    Ok(result) => {
                        failed_ref.fetch_add(result.counts.error_count(), Ordering::Relaxed);
                        // Every entry a batch committed was counted by that
                        // batch. The lines the stream threw out before they
                        // reached one — unparseable, or carrying the wrong
                        // resource type — were counted by nobody, so they are
                        // the worker's to add (#969).
                        if result.unbatched_errors > 0
                            && let Err(e) = self
                                .jobs
                                .add_manifest_progress(
                                    lease_ref,
                                    0,
                                    result.unbatched_errors,
                                    result.unbatched_errors,
                                )
                                .await
                        {
                            return fenced_write_outcome(keeper_ref, e);
                        }
                    }
                    Err(e) => {
                        let file_error = self
                            .write_manifest_error(
                                lease_ref,
                                manifest_url_ref,
                                i as u32 + 2,
                                &format!("failed to ingest file {}: {e}", file.url),
                            )
                            .await?;
                        error_records_ref.lock().await.push(file_error);
                        failed_ref.fetch_add(1, Ordering::Relaxed);
                        // Every entry this file did commit was already counted
                        // by its own batch; the file-level failure was not.
                        if let Err(e) = self.jobs.add_manifest_progress(lease_ref, 0, 1, 0).await {
                            return fenced_write_outcome(keeper_ref, e);
                        }
                    }
                }

                // File boundary: no batch holds the write lock here, so fold
                // the WAL back into the database before the next file (#978).
                self.jobs.checkpoint_after_file().await;

                let total = progress_ref.total.load(Ordering::Relaxed);
                if total > 0 {
                    let _ = self
                        .jobs
                        .update_manifest_bytes(
                            lease_ref,
                            progress_ref.consumed.load(Ordering::Relaxed),
                            total,
                        )
                        .await;
                }
                Ok(())
            })
            .buffer_unordered(self.file_concurrency.max(1));
        // Race the whole fan-out against the lease: losing it has to stop the
        // ingest *inside* a file, not merely between files. A batch already
        // committed stays committed and re-ingests idempotently when the
        // manifest is reclaimed; continuing to write past expiry would let a
        // second worker ingest the same manifest alongside this one (#969).
        let drain = async {
            while let Some(result) = ingest.next().await {
                result?;
            }
            Ok::<(), StorageError>(())
        };
        let completed = tokio::select! {
            biased;
            _ = keeper.lost() => false,
            result = drain => {
                result?;
                true
            }
        };
        drop(ingest);
        if !completed {
            tracing::warn!(
                submission = %lease.submission_id,
                manifest = %lease.manifest_id,
                worker = %lease.worker_id,
                "bulk-submit run abandoned mid-manifest: its lease is no longer held"
            );
            return Ok(());
        }
        let mut failed = failed_at.load(Ordering::Relaxed);

        // 2b. Process `deleted` files — transaction Bundles / resource refs to
        // remove. Successful deletions across every deleted file share one
        // part-0 receipt; a deleted-file fetch failure gets a source-ordinal
        // error artifact and does not abort the remaining files.
        let mut records = error_records.lock().await.clone();
        let mut deleted_refs = Vec::new();
        for (input_index, file) in manifest.deleted.iter().enumerate() {
            if keeper.should_stop() {
                log_wind_down(&lease, keeper.cancelled());
                return Ok(());
            }
            match self
                .fetcher
                .open_file_stream(
                    &file.url,
                    &view.file_request_headers,
                    manifest.requires_access_token,
                    &view.oauth_metadata_urls,
                    view.file_encryption_key.as_ref(),
                )
                .await
            {
                Ok((reader, _)) => {
                    self.process_deleted_stream(&lease, reader, &mut deleted_refs)
                        .await;
                }
                Err(e) => {
                    let deleted_error = self
                        .write_manifest_error(
                            &lease,
                            &manifest_url,
                            manifest.output.len() as u32 + input_index as u32 + 2,
                            &format!("failed to fetch deleted file {}: {e}", file.url),
                        )
                        .await?;
                    records.push(deleted_error);
                }
            }
        }
        if !deleted_refs.is_empty() {
            let deleted_record = self
                .write_deleted_artifact(&lease, &manifest_url, &deleted_refs)
                .await?;
            records.push(deleted_record);
        }

        // 2c. Push this manifest's ingested resources into the deployment's
        // secondary search indexes, before the receipt is written (#1007): a
        // resource a secondary rejects after retries must not read `success`
        // in the receipt or the status counts. Same wind-down gate as the
        // receipt step below: a lost lease or an abort must not sync into a
        // manifest another worker (or the abort itself) already owns.
        if keeper.should_stop() {
            log_wind_down(&lease, keeper.cancelled());
            return Ok(());
        }
        let sync = match self.jobs.sync_ingested(&lease).await {
            Ok(report) => report,
            Err(LeaseError::LeaseLost { .. }) => return Ok(()),
            Err(LeaseError::Storage(e)) => return Err(e),
        };
        if sync.unindexed > 0 {
            tracing::warn!(
                submission = %lease.submission_id,
                manifest = %lease.manifest_id,
                unindexed = sync.unindexed,
                synced = sync.synced,
                "bulk-submit: a secondary rejected ingested resources after retries; \
                 their entry results are now processing-error"
            );
            // `processed_entries` is not corrected: it is cumulative across
            // runs and over-reporting is the safe direction (see
            // `SubmitWorkerStorage::add_manifest_progress`'s docs).
            // `failed_entries` must still grow, so the status counts and this
            // manifest's own receipt agree.
            if let Err(e) = self
                .jobs
                .add_manifest_progress(&lease, 0, sync.unindexed, 0)
                .await
            {
                return fenced_write_outcome(&keeper, e);
            }
        }
        failed += sync.unindexed;
        for d in &sync.drift {
            tracing::warn!(
                submission = %lease.submission_id,
                manifest = %lease.manifest_id,
                resource_type = %d.resource_type,
                backend_id = %d.backend_id,
                primary_count = d.primary_count,
                search_count = d.search_count,
                "bulk-submit: primary and search index disagree on this resource type's count"
            );
        }

        // 3. Emit per-type `output` receipts and an aggregated `error` artifact.
        // A wound-down job writes neither these nor a terminal manifest status:
        // the receipts would claim a manifest that never finished, and an
        // aborted manifest's outcome is already recorded by the abort (#968).
        if keeper.should_stop() {
            log_wind_down(&lease, keeper.cancelled());
            return Ok(());
        }
        // Receipts spool to disk and replay part by part, so the lease has to
        // cover the whole step: a reclaimed manifest must not keep writing
        // artifacts a second worker is about to produce as well.
        let receipts = tokio::select! {
            biased;
            _ = keeper.lost() => None,
            receipts = self.write_result_artifacts(
                &lease,
                &manifest_url,
                view.fhir_version,
                failed,
                &sync.drift,
            ) => Some(receipts?),
        };
        let Some(receipts) = receipts else {
            tracing::warn!(
                submission = %lease.submission_id,
                manifest = %lease.manifest_id,
                worker = %lease.worker_id,
                "bulk-submit run abandoned while writing result receipts: its lease \
                 is no longer held"
            );
            return Ok(());
        };
        records.extend(receipts);

        // 4. Publish all finalized artifacts and the terminal state together
        // where the storage engine supports it.
        let published = self
            .publish_collected_artifacts(
                &lease,
                keeper,
                records,
                ManifestPublicationStatus::Completed,
            )
            .await?;

        // 5. Fast-load (#903): the manifest ingested without search indexing —
        // rebuild the indexes for its resource types now. Fire-and-forget:
        // the manifest is already terminal, and the hook drives the same
        // machinery $reindex does.
        if published {
            self.reindex_deferred(&lease, &manifest.output).await;
        }
        Ok(())
    }

    /// Reads back this manifest's entry results and writes `output` receipts
    /// (grouped by resource type) plus a single aggregated `error` artifact,
    /// spooling them to disk instead of holding the receipt set in memory (#982).
    ///
    /// `drift` adds one `warning` OperationOutcome per [`IndexDrift`] to the
    /// `error` artifact, naming the `$reindex` repair. A drift is a tenant-wide
    /// count disagreement, not a failed entry: it does not affect
    /// `failed_count` or any `output` line.
    async fn write_result_artifacts(
        &self,
        lease: &ManifestLease,
        manifest_url: &str,
        _fhir_version: FhirVersion,
        failed_count: u64,
        drift: &[IndexDrift],
    ) -> StorageResult<Vec<SubmitFileRecord>> {
        let pages = entry_result_pages(|continuation| async move {
            self.jobs
                .get_entry_results_page(
                    &lease.tenant,
                    &lease.submission_id,
                    &lease.manifest_id,
                    None,
                    1000,
                    continuation.as_ref(),
                )
                .await
        });
        let spools = ReceiptSpools::new()?;
        self.write_result_artifact_pages(spools, lease, manifest_url, failed_count, drift, pages)
            .await
    }

    /// Streams the entry-result pages into spools, then replays them into parts.
    ///
    /// The spools are owned by the caller so a test can watch the directory a
    /// run spools into; production callers hand in a fresh temp directory.
    async fn write_result_artifact_pages(
        &self,
        mut spools: ReceiptSpools,
        lease: &ManifestLease,
        manifest_url: &str,
        failed_count: u64,
        drift: &[IndexDrift],
        pages: impl futures::Stream<Item = StorageResult<EntryResultPage>>,
    ) -> StorageResult<Vec<SubmitFileRecord>> {
        use futures::TryStreamExt;
        let mut severity: std::collections::BTreeMap<String, u64> =
            std::collections::BTreeMap::new();

        // One page at a time: every entry is consumed, serialized, and dropped
        // before the next page is fetched, so no page's receipts outlive it.
        futures::pin_mut!(pages);
        while let Some(page) = pages.try_next().await? {
            for paged in page.entries {
                let mut entry = paged.result;
                match entry.outcome {
                    BulkEntryOutcome::Success => {
                        if let Some(id) = &entry.resource_id {
                            let line =
                                json!({"reference": format!("{}/{}", entry.resource_type, id)})
                                    .to_string();
                            spools.push_output(&entry.resource_type, &line).await?;
                        }
                    }
                    BulkEntryOutcome::ValidationError | BulkEntryOutcome::ProcessingError => {
                        // The stored outcome carries the error; a result without
                        // one still gets a receipt in the same shape.
                        let stored = entry.operation_outcome.take();
                        let oo = match stored {
                            Some(oo) => oo,
                            None => default_error_outcome(&entry),
                        };
                        tally_severity(&oo, &mut severity);
                        spools.push_error(&oo.to_string()).await?;
                    }
                    BulkEntryOutcome::Skipped => {}
                }
            }
        }

        // The streaming engine counts parse / wrong-type failures but does not
        // persist them as per-line entry results. Surface any such uncaptured
        // failures as a summary OperationOutcome so the status manifest's `error`
        // array reflects them (partial success).
        let recorded_errors = spools.error_rows();
        if failed_count > recorded_errors {
            let uncaptured = failed_count - recorded_errors;
            let oo = json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "error",
                    "code": "processing",
                    "diagnostics": format!(
                        "{uncaptured} submitted resource(s) could not be parsed or did not \
                         match the declared resource type"
                    )
                }]
            });
            tally_severity(&oo, &mut severity);
            spools.push_error(&oo.to_string()).await?;
        }

        // One `warning` OperationOutcome per index drift (#1007): a tenant-wide
        // count disagreement, not a failed entry, so it neither touches
        // `failed_count` nor removes anything already collected in the output
        // spools. Pushed before the spools close, same as every other error line.
        for d in drift {
            let oo = json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "warning",
                    "code": "incomplete",
                    "diagnostics": format!(
                        "Search index drift for {rt} (tenant-wide, not only this manifest): \
                         primary holds {pc} resources, {backend} holds {sc}. Run \
                         POST /{rt}/$reindex to rebuild. Without Elasticsearch \
                         refresh=wait_for a small difference can be writes not yet visible; \
                         confirm with GET /{rt}?_summary=count.",
                        rt = d.resource_type,
                        pc = d.primary_count,
                        backend = d.backend_id,
                        sc = d.search_count,
                    )
                }]
            });
            tally_severity(&oo, &mut severity);
            spools.push_error(&oo.to_string()).await?;
        }

        // Replay publishes the spool counters, so every writer has to be closed
        // first.
        spools.close_writers().await?;

        let mut records = Vec::new();

        // Replay one `output` part per observed resource type, alphabetically,
        // with a dense part index: a type that produced no success receipt has no
        // spool and consumes no index.
        for (idx, (resource_type, spool)) in spools.take_outputs().into_iter().enumerate() {
            let key = submit_artifact_key(
                &lease.tenant,
                &lease.submission_id,
                &lease.manifest_id,
                "output",
                Some(resource_type.as_str()),
                idx as u32,
                lease.fencing_token,
            );
            let (line_count, byte_count) = self.replay_receipt_spool(&spools, &key, &spool).await?;
            records.push(SubmitFileRecord {
                manifest_url: Some(manifest_url.to_string()),
                file_type: "output".to_string(),
                resource_type: Some(resource_type),
                part_index: idx as u32,
                file_path: key.resource_type,
                line_count,
                byte_count,
                count_severity: None,
            });
            // The bytes are never replayed twice: dropping the file now keeps the
            // disk peak lower while the remaining parts and any store scratch
            // coexist.
            spools.remove(&spool).await;
        }

        // Write a single aggregated `error` part (if any), last.
        if let Some(spool) = spools.take_error() {
            let key = submit_artifact_key(
                &lease.tenant,
                &lease.submission_id,
                &lease.manifest_id,
                "error",
                Some("OperationOutcome"),
                0,
                lease.fencing_token,
            );
            let (line_count, byte_count) = self.replay_receipt_spool(&spools, &key, &spool).await?;
            let count_severity = Value::Object(
                severity
                    .into_iter()
                    .map(|(k, v)| (k, Value::from(v)))
                    .collect(),
            );
            records.push(SubmitFileRecord {
                manifest_url: Some(manifest_url.to_string()),
                file_type: "error".to_string(),
                resource_type: Some("OperationOutcome".to_string()),
                part_index: 0,
                file_path: key.resource_type,
                line_count,
                byte_count,
                count_severity: Some(count_severity),
            });
            spools.remove(&spool).await;
        }
        Ok(records)
    }

    /// Copies one receipt spool into a finalized output part.
    ///
    /// The spool already holds exactly the serialized rows — one trailing
    /// newline each — so replay is a fixed-size byte copy into the part writer's
    /// sink: no row is re-serialized, parsed, or rematerialized, which is what
    /// keeps an oversized OperationOutcome from being allocated a second time.
    /// The part's counters come from the spool bookkeeping that counted those
    /// bytes. A spool that does not replay exactly that many bytes fails the
    /// manifest rather than publishing counts that disagree with the artifact.
    async fn replay_receipt_spool(
        &self,
        spools: &ReceiptSpools,
        key: &ExportPartKey,
        spool: &Spool,
    ) -> StorageResult<(u64, u64)> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut reader = spools.open_reader(spool).await?;
        let mut writer = self.output.open_writer(key).await?;
        let mut chunk = vec![0_u8; SPOOL_BUFFER_BYTES];
        let mut copied: u64 = 0;
        loop {
            let read = reader.read(&mut chunk).await.map_err(|e| {
                crate::error::StorageError::Backend(crate::error::BackendError::Internal {
                    backend_name: "bulk-submit-output".to_string(),
                    message: format!("read receipt spool: {e}"),
                    source: None,
                })
            })?;
            if read == 0 {
                break;
            }
            writer
                .writer
                .write_all(&chunk[..read])
                .await
                .map_err(artifact_write_error)?;
            copied += read as u64;
        }
        if copied != spool.bytes {
            return Err(internal_error(format!(
                "receipt spool for {} part {} replayed {copied} bytes, expected {}",
                key.file_type, key.part_index, spool.bytes
            )));
        }
        writer.line_count = spool.lines;
        writer.byte_count = spool.bytes;
        let finalized = self.output.finalize_part(key, writer).await?;
        Ok((finalized.line_count, finalized.size_bytes))
    }

    /// Applies deletions from a `deleted` NDJSON stream (transaction Bundles or
    /// bare resources), collecting `Type/id` references actually removed.
    async fn process_deleted_stream(
        &self,
        lease: &ManifestLease,
        reader: Box<dyn tokio::io::AsyncBufRead + Send + Unpin>,
        refs: &mut Vec<String>,
    ) {
        use tokio::io::AsyncBufReadExt;
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(val) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if val.get("resourceType").and_then(|v| v.as_str()) == Some("Bundle") {
                if let Some(entries) = val.get("entry").and_then(|e| e.as_array()) {
                    for e in entries {
                        if let Some(url) = e
                            .get("request")
                            .and_then(|r| r.get("url"))
                            .and_then(|u| u.as_str())
                        {
                            if let Some((ty, id)) = url.split_once('/') {
                                if self.jobs.delete(&lease.tenant, ty, id).await.is_ok() {
                                    refs.push(format!("{ty}/{id}"));
                                }
                            }
                        }
                    }
                }
            } else if let (Some(ty), Some(id)) = (
                val.get("resourceType").and_then(|v| v.as_str()),
                val.get("id").and_then(|v| v.as_str()),
            ) {
                if self.jobs.delete(&lease.tenant, ty, id).await.is_ok() {
                    refs.push(format!("{ty}/{id}"));
                }
            }
        }
    }

    /// Writes the `deleted` receipt artifact listing removed resource references.
    async fn write_deleted_artifact(
        &self,
        lease: &ManifestLease,
        manifest_url: &str,
        refs: &[String],
    ) -> StorageResult<SubmitFileRecord> {
        let lines: Vec<String> = refs
            .iter()
            .map(|r| json!({ "reference": r }).to_string())
            .collect();
        let key = submit_artifact_key(
            &lease.tenant,
            &lease.submission_id,
            &lease.manifest_id,
            "deleted",
            Some("Bundle"),
            0,
            lease.fencing_token,
        );
        let (line_count, byte_count) = self.write_part(&key, &lines).await?;
        Ok(SubmitFileRecord {
            manifest_url: Some(manifest_url.to_string()),
            file_type: "deleted".to_string(),
            resource_type: Some("Bundle".to_string()),
            part_index: 0,
            file_path: key.resource_type,
            line_count,
            byte_count,
            count_severity: None,
        })
    }

    /// Publishes the coarse pre-ingest phase for the status endpoint (#953).
    ///
    /// Best-effort by design: the phase is a cosmetic hint, so *no* failure
    /// here may change what the job does. That includes `LeaseLost` — a lost
    /// lease is already detected authoritatively by the [`LeaseKeeper`]'s
    /// heartbeat and checked at each file boundary, and letting a decorative
    /// write also decide control flow would add a way to abandon a healthy
    /// job without adding any way to notice a dead one.
    async fn report_phase(
        &self,
        lease: &ManifestLease,
        phase: ManifestPhase,
        files_done: u64,
        files_total: u64,
    ) {
        if let Err(e) = self
            .jobs
            .update_manifest_phase(lease, phase, files_done, files_total)
            .await
        {
            tracing::debug!(
                submission = %lease.submission_id,
                manifest = %lease.manifest_id,
                %phase,
                error = %e,
                "could not record bulk-submit pre-ingest phase"
            );
        }
    }

    /// Writes a single manifest-level `error` OperationOutcome artifact.
    async fn write_manifest_error(
        &self,
        lease: &ManifestLease,
        manifest_url: &str,
        part_index: u32,
        message: &str,
    ) -> StorageResult<SubmitFileRecord> {
        let oo = json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": "processing",
                "diagnostics": message
            }]
        })
        .to_string();
        let key = submit_artifact_key(
            &lease.tenant,
            &lease.submission_id,
            &lease.manifest_id,
            "error",
            Some("OperationOutcome"),
            part_index,
            lease.fencing_token,
        );
        let part = self.write_part(&key, std::slice::from_ref(&oo)).await?;
        Ok(SubmitFileRecord {
            manifest_url: Some(manifest_url.to_string()),
            file_type: "error".to_string(),
            resource_type: Some("OperationOutcome".to_string()),
            part_index,
            file_path: key.resource_type,
            line_count: part.0,
            byte_count: part.1,
            count_severity: Some(json!({"error": 1})),
        })
    }

    /// Writes NDJSON `lines` to a new output-store part, returning `(line_count, byte_count)`.
    async fn write_part(&self, key: &ExportPartKey, lines: &[String]) -> StorageResult<(u64, u64)> {
        let mut writer = self.output.open_writer(key).await?;
        for line in lines {
            writer
                .write_line(line)
                .await
                .map_err(artifact_write_error)?;
        }
        let finalized = self.output.finalize_part(key, writer).await?;
        Ok((finalized.line_count, finalized.size_bytes))
    }

    /// Publishes all collected finalized artifacts with the live lease.
    ///
    /// The keeper owns the heartbeat until publication has returned, then is
    /// dropped before any deferred indexing. A publication already committed
    /// by this same generation is quiet and does not trigger reindexing.
    async fn publish_collected_artifacts(
        &self,
        lease: &ManifestLease,
        keeper: LeaseKeeper,
        files: Vec<SubmitFileRecord>,
        terminal: ManifestPublicationStatus,
    ) -> StorageResult<bool> {
        let outcome = self
            .jobs
            .publish_manifest_artifacts(lease, &files, terminal)
            .await;
        drop(keeper);
        match outcome {
            Ok(ManifestPublicationResult::Published) => Ok(true),
            Ok(ManifestPublicationResult::AlreadyPublished) => Ok(false),
            Err(LeaseError::Storage(e)) => Err(e),
            Err(LeaseError::LeaseLost { .. }) => Ok(false),
        }
    }

    /// Rebuilds search indexes for the manifest's resource types after
    /// deferred ingestion. Fire-and-forget: only publication storage errors
    /// may affect the run.
    async fn reindex_deferred(&self, lease: &ManifestLease, output_files: &[RemoteFile]) {
        if !self.defer_indexing {
            return;
        }
        let mut types: Vec<String> = output_files
            .iter()
            .filter_map(|file| file.resource_type.clone())
            .collect();
        types.sort();
        types.dedup();
        match (&self.reindex_hook, types.is_empty()) {
            (Some(hook), false) => {
                tracing::info!(
                    submission = %lease.submission_id,
                    manifest = %lease.manifest_id,
                    types = ?types,
                    "bulk fast-load: rebuilding deferred search indexes"
                );
                hook.reindex_types(&lease.tenant, types).await;
            }
            _ => {
                tracing::warn!(
                    submission = %lease.submission_id,
                    manifest = %lease.manifest_id,
                    "bulk fast-load ingested without indexing and no reindex hook is wired — run $reindex to make the data searchable"
                );
            }
        }
    }
}

/// Resolves what a failed fenced write means for the file being ingested.
///
/// A storage error aborts the whole job, as it always has. A lost lease is not
/// an error at all: the manifest belongs to someone else now, so the keeper is
/// told, which stops the sibling files too, and this one ends quietly (#969).
fn fenced_write_outcome(keeper: &LeaseKeeper, e: LeaseError) -> StorageResult<()> {
    match e {
        LeaseError::Storage(e) => Err(e),
        LeaseError::LeaseLost { .. } => {
            keeper.declare_lost();
            Ok(())
        }
    }
}

fn internal_error(message: impl Into<String>) -> StorageError {
    StorageError::Backend(crate::error::BackendError::Internal {
        backend_name: "bulk-submit".to_string(),
        message: message.into(),
        source: None,
    })
}

/// Maps a failure writing bytes into an output-store part.
fn artifact_write_error(e: std::io::Error) -> StorageError {
    StorageError::Backend(crate::error::BackendError::Internal {
        backend_name: "bulk-submit-output".to_string(),
        message: format!("write artifact: {e}"),
        source: None,
    })
}

/// Builds a fallback OperationOutcome when an entry result lacks one.
fn default_error_outcome(entry: &crate::core::bulk_submit::BulkEntryResult) -> Value {
    json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "error",
            "code": "processing",
            "diagnostics": format!(
                "{} error on {} line {}",
                entry.outcome, entry.resource_type, entry.line_number
            )
        }]
    })
}

/// Tallies issue severities from an OperationOutcome into `acc` (for `countSeverity`).
fn tally_severity(oo: &Value, acc: &mut std::collections::BTreeMap<String, u64>) {
    if let Some(issues) = oo.get("issue").and_then(|v| v.as_array()) {
        for issue in issues {
            let sev = issue
                .get("severity")
                .and_then(|v| v.as_str())
                .unwrap_or("error");
            *acc.entry(sev.to_string()).or_insert(0) += 1;
        }
    } else {
        *acc.entry("error".to_string()).or_insert(0) += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::local_fs::LocalFsOutputStore;
    use crate::backends::sqlite::SqliteBackend;
    use crate::core::ManifestStatus;
    use crate::core::bulk_submit::{
        BulkEntryResult, BulkSubmitProvider, EntryResultContinuation, PagedEntryResult,
    };
    use crate::core::bulk_submit_input::{RemoteFile, RemoteManifest, submission_output_job_id};
    use crate::core::storage::ResourceStorage;
    use crate::tenant::{TenantContext, TenantId, TenantPermissions};
    use rusqlite::params;
    use std::time::Duration as StdDuration;

    mod scripted_pages {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/bulk_submit/scripted_pages.rs"
        ));
    }

    #[tokio::test]
    async fn artifact_consumer_reads_beyond_an_empty_page_with_continuation() {
        use tokio::io::AsyncReadExt;
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost",
        ));
        let tenant = tenant();
        let sub = SubmissionId::generate("scripted-artifacts");
        backend
            .create_submission(&tenant, &sub, None)
            .await
            .unwrap();
        backend
            .add_manifest(&tenant, &sub, Some("http://provider/m.json"), None)
            .await
            .unwrap();
        let lease = backend
            .claim_next_manifest(&WorkerId::new("scripted"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            patient_fetcher(""),
            output.clone(),
            WorkerId::new("scripted"),
        );
        let (pages, calls) = scripted_pages::pages();
        let spool_dir = tempfile::tempdir().unwrap();
        let spool_path = spool_dir.path().to_path_buf();
        let records = worker
            .write_result_artifact_pages(
                ReceiptSpools::in_dir(spool_dir),
                &lease,
                "http://provider/m.json",
                0,
                &[],
                pages,
            )
            .await
            .unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert!(
            !spool_path.exists(),
            "receipt spools must not outlive the run that built them"
        );
        assert_eq!(records.len(), 1);
        backend
            .publish_manifest_artifacts(
                &lease,
                &records,
                crate::core::bulk_submit_publication::ManifestPublicationStatus::Completed,
            )
            .await
            .unwrap();
        let rows = backend.list_submit_files(&tenant, &sub).await.unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.line_count, 3);
        let key = ExportPartKey {
            tenant_id: tenant.tenant_id().as_str().to_string(),
            job_id: submission_output_job_id(&sub),
            resource_type: row.file_path.clone(),
            file_type: row.file_type.clone(),
            part_index: row.part_index,
            fencing_token: row.fencing_token,
        };
        let mut reader = output.open_reader(&key).await.unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(row.byte_count, bytes.len() as u64);
        let references: Vec<Value> = std::str::from_utf8(&bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            references,
            vec![
                json!({"reference":"Patient/after-empty"}),
                json!({"reference":"Patient/after-empty"}),
                json!({"reference":"Patient/exclusive-late"}),
            ]
        );
    }

    /// The consumer has to reach the spool between pages instead of passing the
    /// whole traversal into memory first: the second fetch fails unless the first
    /// page's oversized row has already put a full writer buffer's worth of bytes
    /// on disk. The previous in-memory construction spooled nothing at all.
    #[tokio::test]
    async fn receipts_reach_the_spool_before_the_next_page_is_fetched() {
        use tokio::io::AsyncReadExt;

        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let tenant = tenant();
        let sub = SubmissionId::generate("lazy-receipts");
        backend
            .create_submission(&tenant, &sub, None)
            .await
            .unwrap();
        backend
            .add_manifest(&tenant, &sub, Some("http://provider/lazy.json"), None)
            .await
            .unwrap();
        let lease = backend
            .claim_next_manifest(&WorkerId::new("lazy-receipts"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost",
        ));
        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            patient_fetcher(""),
            output.clone(),
            WorkerId::new("lazy-receipts"),
        );

        let oversized_id = "p".repeat(3 * SPOOL_BUFFER_BYTES + 4096);
        let oversized = json!({"reference": format!("Patient/{oversized_id}")}).to_string();
        let after = json!({"reference": "Patient/after-oversized"}).to_string();
        let spool_dir = tempfile::tempdir().unwrap();
        let spool_path = spool_dir.path().to_path_buf();
        // Both `BufWriter` and Tokio's file staging can retain a full buffer.
        // File::write_all may return while its blocking write is still pending,
        // so a row just larger than one buffer need not be visible on disk yet.
        // This row exceeds both buffers by more than another full buffer: at
        // least that much must have finished writing before the next fetch.
        // The tail and newline can still be buffered. Requiring the whole row
        // or flushing per page would change the production buffering to suit
        // the test.
        let expected_on_disk = SPOOL_BUFFER_BYTES as u64;
        let spooled_id = oversized_id.clone();
        let directory = spool_path.clone();
        let mut script = std::collections::VecDeque::from([
            (
                None,
                EntryResultPage {
                    entries: vec![PagedEntryResult {
                        result: BulkEntryResult::success(1, "Patient", spooled_id, true),
                        stored_identity: None,
                    }],
                    next: Some(EntryResultContinuation::Offset(1)),
                },
            ),
            (
                Some(EntryResultContinuation::Offset(1)),
                EntryResultPage {
                    entries: vec![PagedEntryResult {
                        result: BulkEntryResult::success(2, "Patient", "after-oversized", true),
                        stored_identity: None,
                    }],
                    next: None,
                },
            ),
        ]);
        let pages = entry_result_pages(move |continuation| {
            let (expected, page) = script.pop_front().expect("must not fetch after EOF");
            assert_eq!(continuation, expected, "the opaque token is passed back");
            let result: StorageResult<EntryResultPage> = match continuation {
                // Nothing can be on disk before the first page is handed over.
                None => Ok(page),
                // A continuation means a previous page was consumed, and its
                // receipts have to be on disk by now.
                Some(_) => {
                    // `fs::metadata` on the path, not `DirEntry::metadata`: on
                    // Windows the listing's size can lag a file still open for
                    // writing and read 0 despite the bytes being on disk.
                    let spooled: u64 = std::fs::read_dir(&directory)
                        .expect("the spool directory exists while receipts are building")
                        .map(|entry| std::fs::metadata(entry.unwrap().path()).unwrap().len())
                        .sum();
                    if spooled < expected_on_disk {
                        Err(internal_error(format!(
                            "the first page must be spooled before the second is fetched: \
                             {spooled} bytes on disk, expected at least {expected_on_disk}"
                        )))
                    } else {
                        Ok(page)
                    }
                }
            };
            std::future::ready(result)
        });

        let records = worker
            .write_result_artifact_pages(
                ReceiptSpools::in_dir(spool_dir),
                &lease,
                "http://provider/lazy.json",
                0,
                &[],
                pages,
            )
            .await
            .unwrap();
        assert!(
            !spool_path.exists(),
            "receipt spools must not outlive the run that built them"
        );
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(
            (
                record.file_type.as_str(),
                record.resource_type.as_deref(),
                record.part_index
            ),
            ("output", Some("Patient"), 0)
        );
        assert_eq!(record.line_count, 2);
        assert_eq!(
            record.byte_count,
            (oversized.len() + after.len() + 2) as u64
        );

        let key = ExportPartKey {
            tenant_id: tenant.tenant_id().as_str().to_string(),
            job_id: submission_output_job_id(&sub),
            resource_type: record.file_path.clone(),
            file_type: record.file_type.clone(),
            part_index: record.part_index,
            fencing_token: lease.fencing_token,
        };
        let mut reader = output.open_reader(&key).await.unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes.len() as u64, record.byte_count);
        assert_eq!(bytes, format!("{oversized}\n{after}\n").into_bytes());
    }

    /// One mixed run over two pages: both sort orders, every outcome that emits
    /// no receipt, a stored OperationOutcome, a fallback one, and the summary
    /// for failures the ingestion engine never persisted.
    #[tokio::test]
    async fn mixed_outcomes_publish_exact_parts_and_one_summary_error() {
        use tokio::io::AsyncReadExt;

        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let tenant = tenant();
        let sub = SubmissionId::generate("mixed-receipts");
        backend
            .create_submission(&tenant, &sub, None)
            .await
            .unwrap();
        backend
            .add_manifest(&tenant, &sub, Some("http://provider/mixed.json"), None)
            .await
            .unwrap();
        let lease = backend
            .claim_next_manifest(&WorkerId::new("mixed-receipts"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost",
        ));
        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            patient_fetcher(""),
            output.clone(),
            WorkerId::new("mixed-receipts"),
        );

        let stored = json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "warning",
                "code": "informational",
                "diagnostics": "stored outcome"
            }]
        });
        let fallback = BulkEntryResult {
            line_number: 5,
            resource_type: "Observation".to_string(),
            resource_id: None,
            created: false,
            outcome: BulkEntryOutcome::ValidationError,
            operation_outcome: None,
        };
        let without_id = BulkEntryResult {
            line_number: 4,
            resource_type: "Patient".to_string(),
            resource_id: None,
            created: false,
            outcome: BulkEntryOutcome::Success,
            operation_outcome: None,
        };
        let mut script = std::collections::VecDeque::from([
            (
                None,
                EntryResultPage {
                    entries: vec![
                        PagedEntryResult {
                            result: BulkEntryResult::success(1, "Patient", "kept", true),
                            stored_identity: None,
                        },
                        PagedEntryResult {
                            result: BulkEntryResult::success(2, "Observation", "obs", true),
                            stored_identity: None,
                        },
                        PagedEntryResult {
                            result: BulkEntryResult::skipped(3, "Patient", "duplicate"),
                            stored_identity: None,
                        },
                        PagedEntryResult {
                            result: without_id,
                            stored_identity: None,
                        },
                        PagedEntryResult {
                            result: fallback,
                            stored_identity: None,
                        },
                    ],
                    next: Some(EntryResultContinuation::Offset(1)),
                },
            ),
            (
                Some(EntryResultContinuation::Offset(1)),
                EntryResultPage {
                    entries: vec![PagedEntryResult {
                        result: BulkEntryResult::processing_error(6, "Patient", stored.clone()),
                        stored_identity: None,
                    }],
                    next: None,
                },
            ),
        ]);
        let pages = entry_result_pages(move |continuation| {
            let (expected, page) = script.pop_front().expect("must not fetch after EOF");
            assert_eq!(continuation, expected, "the opaque token is passed back");
            std::future::ready(Ok(page))
        });

        let spool_dir = tempfile::tempdir().unwrap();
        let spool_path = spool_dir.path().to_path_buf();
        // Two stored errors are recorded, three parse failures are not.
        let records = worker
            .write_result_artifact_pages(
                ReceiptSpools::in_dir(spool_dir),
                &lease,
                "http://provider/mixed.json",
                5,
                &[],
                pages,
            )
            .await
            .unwrap();
        assert!(!spool_path.exists());
        assert_eq!(
            records.len(),
            3,
            "Observation, Patient and the aggregated error"
        );

        // Alphabetical types, dense part index: Observation is part 0 even
        // though Patient's receipt was stored first.
        let expected_outputs = [
            (0, "Observation", "{\"reference\":\"Observation/obs\"}"),
            (1, "Patient", "{\"reference\":\"Patient/kept\"}"),
        ];
        for (index, resource_type, row) in expected_outputs {
            let record = &records[index];
            assert_eq!(record.file_type, "output");
            assert_eq!(record.resource_type.as_deref(), Some(resource_type));
            assert_eq!(record.part_index, index as u32);
            assert_eq!(record.count_severity, None);
            assert_eq!(record.line_count, 1);
            assert_eq!(record.byte_count, (row.len() + 1) as u64);
            let expected_key = submit_artifact_key(
                &tenant,
                &sub,
                &lease.manifest_id,
                "output",
                Some(resource_type),
                index as u32,
                lease.fencing_token,
            );
            assert_eq!(record.file_path, expected_key.resource_type);
            let key = ExportPartKey {
                tenant_id: tenant.tenant_id().as_str().to_string(),
                job_id: submission_output_job_id(&sub),
                resource_type: record.file_path.clone(),
                file_type: record.file_type.clone(),
                part_index: record.part_index,
                fencing_token: lease.fencing_token,
            };
            let mut reader = output.open_reader(&key).await.unwrap();
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await.unwrap();
            assert_eq!(bytes.len() as u64, record.byte_count);
            assert_eq!(bytes, format!("{row}\n").into_bytes());
        }

        let error = &records[2];
        assert_eq!(
            (
                error.file_type.as_str(),
                error.resource_type.as_deref(),
                error.part_index
            ),
            ("error", Some("OperationOutcome"), 0)
        );
        assert_eq!(error.line_count, 3);
        assert_eq!(
            error.count_severity,
            Some(json!({"error": 2, "warning": 1}))
        );
        let expected_key = submit_artifact_key(
            &tenant,
            &sub,
            &lease.manifest_id,
            "error",
            Some("OperationOutcome"),
            0,
            lease.fencing_token,
        );
        assert_eq!(error.file_path, expected_key.resource_type);
        let key = ExportPartKey {
            tenant_id: tenant.tenant_id().as_str().to_string(),
            job_id: submission_output_job_id(&sub),
            resource_type: error.file_path.clone(),
            file_type: error.file_type.clone(),
            part_index: error.part_index,
            fencing_token: lease.fencing_token,
        };
        let mut reader = output.open_reader(&key).await.unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes.len() as u64, error.byte_count);
        let rows: Vec<Value> = std::str::from_utf8(&bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            rows,
            vec![
                json!({"resourceType": "OperationOutcome", "issue": [{
                    "severity": "error",
                    "code": "processing",
                    "diagnostics": "validation-error error on Observation line 5"
                }]}),
                stored,
                json!({"resourceType": "OperationOutcome", "issue": [{
                    "severity": "error",
                    "code": "processing",
                    "diagnostics": "3 submitted resource(s) could not be parsed or did not match the declared resource type"
                }]}),
            ]
        );
    }

    /// An empty manifest — one listing no output files — stores no entry
    /// results, so its terminal generation carries no receipts at all.
    #[tokio::test]
    async fn a_manifest_without_output_files_publishes_no_artifacts() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let tenant = tenant();
        let sub = SubmissionId::generate("empty-receipts");
        backend
            .create_submission(&tenant, &sub, None)
            .await
            .unwrap();
        backend
            .add_manifest(&tenant, &sub, Some("http://provider/empty.json"), None)
            .await
            .unwrap();
        let lease = backend
            .claim_next_manifest(&WorkerId::new("empty-receipts"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        let fetcher = Arc::new(MockFetcher {
            files: std::collections::HashMap::new(),
            manifest: RemoteManifest {
                requires_access_token: false,
                output: Vec::new(),
                deleted: Vec::new(),
            },
        });
        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            fetcher,
            Arc::new(LocalFsOutputStore::new(
                tmp.path().to_path_buf(),
                "http://localhost",
            )),
            WorkerId::new("empty-receipts"),
        );
        worker.run_job(lease).await.unwrap();

        assert_eq!(
            backend.list_manifests(&tenant, &sub).await.unwrap()[0].status,
            ManifestStatus::Completed
        );
        assert!(
            backend
                .list_submit_files(&tenant, &sub)
                .await
                .unwrap()
                .is_empty(),
            "an empty manifest publishes neither output nor error parts"
        );
    }

    /// Results that are all errors publish the aggregated `error` part and
    /// nothing else, and `countSeverity` tallies what the OperationOutcomes
    /// carry: an unusual severity as itself, an issue with no severity as
    /// `error`, an empty issue array as nothing, and an outcome with no issue
    /// array at all as one `error`.
    #[tokio::test]
    async fn error_only_results_publish_one_error_part_and_tally_severities() {
        use tokio::io::AsyncReadExt;

        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let tenant = tenant();
        let sub = SubmissionId::generate("error-only-receipts");
        backend
            .create_submission(&tenant, &sub, None)
            .await
            .unwrap();
        backend
            .add_manifest(&tenant, &sub, Some("http://provider/error-only.json"), None)
            .await
            .unwrap();
        let lease = backend
            .claim_next_manifest(
                &WorkerId::new("error-only-receipts"),
                StdDuration::from_secs(60),
            )
            .await
            .unwrap()
            .unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost",
        ));
        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            patient_fetcher(""),
            output.clone(),
            WorkerId::new("error-only-receipts"),
        );

        let vendor = json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "vendor-severity",
                "code": "processing",
                "diagnostics": "a severity this build does not know"
            }]
        });
        let unlabelled = json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "code": "processing",
                "diagnostics": "an issue without a severity"
            }]
        });
        let no_issues = json!({"resourceType": "OperationOutcome", "issue": []});
        let no_issue_array = json!({"resourceType": "OperationOutcome"});
        // A result that stored no OperationOutcome still gets a receipt.
        let fallback = BulkEntryResult {
            line_number: 1,
            resource_type: "Observation".to_string(),
            resource_id: None,
            created: false,
            outcome: BulkEntryOutcome::ValidationError,
            operation_outcome: None,
        };
        let expected_rows = vec![
            json!({"resourceType": "OperationOutcome", "issue": [{
                "severity": "error",
                "code": "processing",
                "diagnostics": "validation-error error on Observation line 1"
            }]}),
            vendor.clone(),
            unlabelled.clone(),
            no_issues.clone(),
            no_issue_array.clone(),
        ];
        let mut script = std::collections::VecDeque::from([(
            None,
            EntryResultPage {
                entries: vec![
                    PagedEntryResult {
                        result: fallback,
                        stored_identity: None,
                    },
                    PagedEntryResult {
                        result: BulkEntryResult::processing_error(2, "Patient", vendor),
                        stored_identity: None,
                    },
                    PagedEntryResult {
                        result: BulkEntryResult::processing_error(3, "Patient", unlabelled),
                        stored_identity: None,
                    },
                    PagedEntryResult {
                        result: BulkEntryResult::processing_error(4, "Patient", no_issues),
                        stored_identity: None,
                    },
                    PagedEntryResult {
                        result: BulkEntryResult::processing_error(5, "Patient", no_issue_array),
                        stored_identity: None,
                    },
                ],
                next: None,
            },
        )]);
        let pages = entry_result_pages(move |continuation| {
            let (expected, page) = script.pop_front().expect("must not fetch after EOF");
            assert_eq!(continuation, expected, "the opaque token is passed back");
            std::future::ready(Ok(page))
        });

        let spool_dir = tempfile::tempdir().unwrap();
        let spool_path = spool_dir.path().to_path_buf();
        // Five recorded errors against a failure count of five: no summary row.
        let records = worker
            .write_result_artifact_pages(
                ReceiptSpools::in_dir(spool_dir),
                &lease,
                "http://provider/error-only.json",
                5,
                &[],
                pages,
            )
            .await
            .unwrap();
        assert!(!spool_path.exists());
        assert_eq!(
            records.len(),
            1,
            "error-only results publish the error part and no output part"
        );
        let error = &records[0];
        assert_eq!(
            (
                error.file_type.as_str(),
                error.resource_type.as_deref(),
                error.part_index
            ),
            ("error", Some("OperationOutcome"), 0)
        );
        assert_eq!(error.line_count, 5);
        assert_eq!(
            error.count_severity,
            Some(json!({"error": 3, "vendor-severity": 1}))
        );

        let expected_key = submit_artifact_key(
            &tenant,
            &sub,
            &lease.manifest_id,
            "error",
            Some("OperationOutcome"),
            0,
            lease.fencing_token,
        );
        assert_eq!(error.file_path, expected_key.resource_type);
        let key = ExportPartKey {
            tenant_id: tenant.tenant_id().as_str().to_string(),
            job_id: submission_output_job_id(&sub),
            resource_type: error.file_path.clone(),
            file_type: error.file_type.clone(),
            part_index: error.part_index,
            fencing_token: lease.fencing_token,
        };
        let mut reader = output.open_reader(&key).await.unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes.len() as u64, error.byte_count);
        let rows: Vec<Value> = std::str::from_utf8(&bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows, expected_rows);
    }

    /// A spool that vanishes under the run must fail the replay rather than
    /// publish a part from whatever else is still on disk. The second fetch
    /// deletes the first type's spool file, so replay reaches the missing file
    /// after it has already published the type that sorts first. The deletion
    /// unlinks a file its writer still holds open, which only Unix allows.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_spool_deleted_while_results_stream_fails_the_replay() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let tenant = tenant();
        let sub = SubmissionId::generate("vanished-receipts");
        backend
            .create_submission(&tenant, &sub, None)
            .await
            .unwrap();
        backend
            .add_manifest(&tenant, &sub, Some("http://provider/vanished.json"), None)
            .await
            .unwrap();
        let lease = backend
            .claim_next_manifest(
                &WorkerId::new("vanished-receipts"),
                StdDuration::from_secs(60),
            )
            .await
            .unwrap()
            .unwrap();
        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            patient_fetcher(""),
            Arc::new(LocalFsOutputStore::new(
                tmp.path().to_path_buf(),
                "http://localhost",
            )),
            WorkerId::new("vanished-receipts"),
        );

        let spool_dir = tempfile::tempdir().unwrap();
        let spool_path = spool_dir.path().to_path_buf();
        let directory = spool_path.clone();
        let mut script = std::collections::VecDeque::from([
            (
                None,
                EntryResultPage {
                    entries: vec![PagedEntryResult {
                        result: BulkEntryResult::success(1, "Patient", "p1", true),
                        stored_identity: None,
                    }],
                    next: Some(EntryResultContinuation::Offset(1)),
                },
            ),
            (
                Some(EntryResultContinuation::Offset(1)),
                EntryResultPage {
                    entries: vec![PagedEntryResult {
                        result: BulkEntryResult::success(2, "Observation", "o1", true),
                        stored_identity: None,
                    }],
                    next: None,
                },
            ),
        ]);
        let pages = entry_result_pages(move |continuation| {
            let (expected, page) = script.pop_front().expect("must not fetch after EOF");
            assert_eq!(continuation, expected, "the opaque token is passed back");
            if continuation.is_some() {
                // Patient's spool is the first one allocated and is on disk by
                // now; the writer still holds it open, as a run that is
                // streaming results does. POSIX unlinks the open file.
                std::fs::remove_file(directory.join("000001.ndjson"))
                    .expect("the first type's spool file exists");
            }
            std::future::ready(Ok(page))
        });

        let error = worker
            .write_result_artifact_pages(
                ReceiptSpools::in_dir(spool_dir),
                &lease,
                "http://provider/vanished.json",
                0,
                &[],
                pages,
            )
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("open receipt spool"),
            "a vanished spool must fail the run: {error}"
        );
        assert!(
            !spool_path.exists(),
            "a failed replay still removes the spool directory"
        );
        assert!(
            backend
                .list_submit_files(&tenant, &sub)
                .await
                .unwrap()
                .is_empty(),
            "nothing is published when a spool cannot be replayed"
        );
    }

    /// Replay is fail-closed: a spool holding fewer bytes than its counters
    /// describe aborts the run instead of publishing a part whose counts
    /// disagree with the artifact.
    #[tokio::test]
    async fn a_truncated_spool_fails_replay_instead_of_publishing_counts() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost",
        ));
        let worker = DefaultSubmitWorker::new(
            backend,
            patient_fetcher(""),
            output,
            WorkerId::new("truncated-spool"),
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let mut spools = ReceiptSpools::in_dir(dir);
        let row = "{\"reference\":\"Patient/truncated\"}";
        spools.push_output("Patient", row).await.unwrap();
        spools.close_writers().await.unwrap();
        let (_, spool) = spools.take_outputs().pop().unwrap();
        assert_eq!(spool.bytes, (row.len() + 1) as u64);

        // The file loses the newline its counters counted. One spool was
        // allocated, so it is the first file the run named.
        tokio::fs::write(path.join("000001.ndjson"), row)
            .await
            .unwrap();
        let tenant = tenant();
        let sub = SubmissionId::generate("truncated-spool");
        let key = submit_artifact_key(&tenant, &sub, "m1", "output", Some("Patient"), 0, 1);
        let error = worker
            .replay_receipt_spool(&spools, &key, &spool)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("expected {}", spool.bytes)),
            "replay must fail on a byte-count mismatch: {error}"
        );
        drop(spools);
        assert!(
            !path.exists(),
            "a failed replay still removes the spool directory"
        );
    }

    /// The replay's own byte copy is the reader the counters came from, so a
    /// spool that opens and then refuses the read has to fail the run too. A
    /// directory in the spool's place reaches that branch deterministically on
    /// Unix; Windows refuses the open instead, so the case is Unix-only like
    /// the deleted-spool one.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_spool_that_cannot_be_read_fails_the_replay() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost",
        ));
        let worker = DefaultSubmitWorker::new(
            backend,
            patient_fetcher(""),
            output,
            WorkerId::new("unreadable-spool"),
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let mut spools = ReceiptSpools::in_dir(dir);
        let row = "{\"reference\":\"Patient/unreadable\"}";
        spools.push_output("Patient", row).await.unwrap();
        spools.close_writers().await.unwrap();
        let (_, spool) = spools.take_outputs().pop().unwrap();
        assert!(spool.bytes > 0);

        // A directory where the spool was: the open succeeds, the read is the
        // call that fails (`EISDIR`).
        tokio::fs::remove_file(path.join("000001.ndjson"))
            .await
            .unwrap();
        tokio::fs::create_dir(path.join("000001.ndjson"))
            .await
            .unwrap();
        let tenant = tenant();
        let sub = SubmissionId::generate("unreadable-spool");
        let key = submit_artifact_key(&tenant, &sub, "m1", "output", Some("Patient"), 0, 1);
        let error = worker
            .replay_receipt_spool(&spools, &key, &spool)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("read receipt spool"),
            "the replay must surface its own read failure: {error}"
        );
        drop(spools);
        assert!(
            !path.exists(),
            "a failed replay still removes the spool directory"
        );
    }

    /// The output store is where the replay's own bytes go, so a failure there
    /// — the part refusing to open, or the sink refusing the first byte the
    /// copy writes — has to reach the caller. Nothing is published, and the
    /// spool directory is removed either way.
    #[tokio::test]
    async fn a_failing_output_store_still_cleans_the_spool_directory() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let tenant = tenant();
        let sub = SubmissionId::generate("store-failure-receipts");
        backend
            .create_submission(&tenant, &sub, None)
            .await
            .unwrap();
        backend
            .add_manifest(
                &tenant,
                &sub,
                Some("http://provider/store-failure.json"),
                None,
            )
            .await
            .unwrap();
        let lease = backend
            .claim_next_manifest(
                &WorkerId::new("store-failure-receipts"),
                StdDuration::from_secs(60),
            )
            .await
            .unwrap()
            .unwrap();
        let objects = tmp.path().join("objects");

        for (fault, expected) in [
            (OutputFault::Write, "write artifact"),
            (OutputFault::Open, "refused the part"),
        ] {
            let worker = DefaultSubmitWorker::new(
                backend.clone(),
                patient_fetcher(""),
                Arc::new(FaultOutputStore::new(
                    Arc::new(LocalFsOutputStore::new(objects.clone(), "http://localhost")),
                    fault,
                )),
                WorkerId::new("store-failure-receipts"),
            );
            let pages = entry_result_pages(|_continuation| {
                std::future::ready(Ok(EntryResultPage {
                    entries: vec![PagedEntryResult {
                        result: BulkEntryResult::success(1, "Patient", "p1", true),
                        stored_identity: None,
                    }],
                    next: None,
                }))
            });
            let spool_dir = tempfile::tempdir().unwrap();
            let spool_path = spool_dir.path().to_path_buf();
            let error = worker
                .write_result_artifact_pages(
                    ReceiptSpools::in_dir(spool_dir),
                    &lease,
                    "http://provider/store-failure.json",
                    0,
                    &[],
                    pages,
                )
                .await
                .unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "expected a {expected} failure, got {error}"
            );
            assert!(
                !spool_path.exists(),
                "a failed replay still removes the spool directory"
            );
            assert!(
                backend
                    .list_submit_files(&tenant, &sub)
                    .await
                    .unwrap()
                    .is_empty(),
                "a failed replay publishes nothing"
            );
        }
    }

    /// The spool directory belongs to the receipt-writing future. Dropping that
    /// future mid-stream — what a taken-over lease does to the run — removes
    /// the directory along with the page the consumer was holding.
    #[tokio::test]
    async fn dropping_the_receipt_future_removes_its_spool_directory() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost",
        ));
        let worker = DefaultSubmitWorker::new(
            backend,
            patient_fetcher(""),
            output,
            WorkerId::new("dropped-receipts"),
        );
        let lease = ManifestLease {
            tenant: tenant(),
            submission_id: SubmissionId::generate("dropped-receipts"),
            manifest_id: "m1".to_string(),
            worker_id: WorkerId::new("dropped-receipts"),
            lease_expiry: Utc::now() + chrono::Duration::seconds(60),
            lease_duration: StdDuration::from_secs(60),
            fencing_token: 1,
        };

        let spool_dir = tempfile::tempdir().unwrap();
        let spool_path = spool_dir.path().to_path_buf();
        let spooled = spool_path.clone();
        let fetched = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let mut script = std::collections::VecDeque::from([
            (
                None,
                EntryResultPage {
                    entries: vec![PagedEntryResult {
                        result: BulkEntryResult::success(1, "Patient", "p1", true),
                        stored_identity: None,
                    }],
                    next: Some(EntryResultContinuation::Offset(1)),
                },
            ),
            (
                Some(EntryResultContinuation::Offset(1)),
                EntryResultPage {
                    entries: Vec::new(),
                    next: None,
                },
            ),
        ]);
        let fetched_signal = Arc::clone(&fetched);
        let release_signal = Arc::clone(&release);
        let pages = entry_result_pages(move |continuation| {
            let (expected, page) = script.pop_front().expect("must not fetch after EOF");
            assert_eq!(continuation, expected, "the opaque token is passed back");
            let signal = Arc::clone(&fetched_signal);
            let held = Arc::clone(&release_signal);
            let directory = spooled.clone();
            async move {
                if continuation.is_some() {
                    // The first page is spooled by now; hold the consumer here
                    // so the future is dropped with its directory in use.
                    assert!(directory.join("000001.ndjson").exists());
                    signal.notify_one();
                    held.notified().await;
                }
                Ok(page)
            }
        });

        let task = tokio::spawn(async move {
            worker
                .write_result_artifact_pages(
                    ReceiptSpools::in_dir(spool_dir),
                    &lease,
                    "http://provider/dropped.json",
                    0,
                    &[],
                    pages,
                )
                .await
        });
        tokio::time::timeout(StdDuration::from_secs(10), fetched.notified())
            .await
            .expect("the consumer has to reach the second fetch");
        assert!(spool_path.join("000001.ndjson").exists());

        task.abort();
        let cancelled = task
            .await
            .expect_err("the receipt task must not finish while it is held");
        assert!(
            cancelled.is_cancelled(),
            "the receipt task must be cancelled, got {cancelled}"
        );
        assert!(
            !spool_path.exists(),
            "dropping the receipt future removes its spool directory"
        );
    }

    /// Captures the deferred-reindex callbacks the worker fires.
    struct MockReindexHook {
        calls: std::sync::Mutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl DeferredReindexHook for MockReindexHook {
        async fn reindex_types(&self, _tenant: &TenantContext, resource_types: Vec<String>) {
            self.calls.lock().unwrap().push(resource_types);
        }
    }

    /// A fetcher that returns a fixed manifest and serves NDJSON from memory.
    struct MockFetcher {
        files: std::collections::HashMap<String, Vec<u8>>,
        manifest: RemoteManifest,
    }

    #[async_trait]
    impl SubmitInputFetcher for MockFetcher {
        async fn fetch_manifest(
            &self,
            _url: &str,
            _headers: &[(String, String)],
            _oauth: &[String],
            _encryption_key: Option<&Value>,
        ) -> StorageResult<RemoteManifest> {
            Ok(self.manifest.clone())
        }

        async fn open_file_stream(
            &self,
            url: &str,
            _headers: &[(String, String)],
            _requires_access_token: bool,
            _oauth: &[String],
            _encryption_key: Option<&Value>,
        ) -> StorageResult<(Box<dyn tokio::io::AsyncBufRead + Send + Unpin>, Option<u64>)> {
            let data = self.files.get(url).cloned().unwrap_or_default();
            let len = data.len() as u64;
            Ok((
                Box::new(tokio::io::BufReader::new(std::io::Cursor::new(data))),
                Some(len),
            ))
        }

        async fn file_size(
            &self,
            url: &str,
            _headers: &[(String, String)],
            _requires_access_token: bool,
            _oauth: &[String],
        ) -> StorageResult<Option<u64>> {
            Ok(self.files.get(url).map(|d| d.len() as u64))
        }
    }

    /// Which step of the output store a [`FaultOutputStore`] breaks.
    #[derive(Clone, Copy)]
    enum OutputFault {
        /// `open_writer` refuses every part.
        Open,
        /// Every part writer is a broken pipe: the first byte written into it
        /// fails, deterministically and without a full device.
        Write,
        /// `finalize_part` fails its given 0-based attempt.
        Finalize(u32),
    }

    /// The local-filesystem store with one step broken and everything else
    /// delegated, so a test can fail a part's open, its bytes, or its
    /// finalization without another copy of the store.
    struct FaultOutputStore {
        inner: Arc<LocalFsOutputStore>,
        fault: OutputFault,
        finalized: std::sync::atomic::AtomicU32,
    }

    impl FaultOutputStore {
        fn new(inner: Arc<LocalFsOutputStore>, fault: OutputFault) -> Self {
            Self {
                inner,
                fault,
                finalized: std::sync::atomic::AtomicU32::new(0),
            }
        }
    }

    fn fault_error(message: &str) -> StorageError {
        StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "bulk-submit-worker-test".to_string(),
            message: message.to_string(),
            source: None,
        })
    }

    #[async_trait]
    impl ExportOutputStore for FaultOutputStore {
        async fn open_writer(
            &self,
            key: &ExportPartKey,
        ) -> StorageResult<crate::core::bulk_export_output::ExportPartWriter> {
            match self.fault {
                OutputFault::Open => Err(fault_error("the output store refused the part")),
                OutputFault::Write => {
                    let (sink, peer) = tokio::io::duplex(1);
                    // The peer is gone, so the first write into the sink fails
                    // with `BrokenPipe` instead of staging the bytes anywhere.
                    drop(peer);
                    let sink: std::pin::Pin<Box<dyn tokio::io::AsyncWrite + Send>> = Box::pin(sink);
                    Ok(crate::core::bulk_export_output::ExportPartWriter::new(sink))
                }
                OutputFault::Finalize(_) => self.inner.open_writer(key).await,
            }
        }

        async fn finalize_part(
            &self,
            key: &ExportPartKey,
            writer: crate::core::bulk_export_output::ExportPartWriter,
        ) -> StorageResult<crate::core::bulk_export_output::FinalizedPart> {
            if let OutputFault::Finalize(ordinal) = self.fault {
                let attempt = self
                    .finalized
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if attempt == ordinal {
                    return Err(fault_error(&format!(
                        "finalize attempt {ordinal} forced failure"
                    )));
                }
            }
            self.inner.finalize_part(key, writer).await
        }

        async fn download_url(
            &self,
            key: &ExportPartKey,
            ttl: Duration,
        ) -> StorageResult<crate::core::bulk_export_output::DownloadUrl> {
            self.inner.download_url(key, ttl).await
        }

        async fn open_reader(
            &self,
            key: &ExportPartKey,
        ) -> StorageResult<std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>> {
            self.inner.open_reader(key).await
        }

        async fn delete_job_outputs(
            &self,
            tenant: &TenantContext,
            job_id: &crate::core::ExportJobId,
        ) -> StorageResult<()> {
            self.inner.delete_job_outputs(tenant, job_id).await
        }
    }

    /// Deliberately fails a compact set of input URLs while the manifest still
    /// fetches normally, so source-ordinal error receipts can be checked.
    struct SelectiveFailureFetcher {
        inner: MockFetcher,
        failing_urls: std::collections::BTreeSet<String>,
    }

    #[async_trait]
    impl SubmitInputFetcher for SelectiveFailureFetcher {
        async fn fetch_manifest(
            &self,
            url: &str,
            headers: &[(String, String)],
            oauth: &[String],
            encryption_key: Option<&Value>,
        ) -> StorageResult<RemoteManifest> {
            self.inner
                .fetch_manifest(url, headers, oauth, encryption_key)
                .await
        }

        async fn open_file_stream(
            &self,
            url: &str,
            headers: &[(String, String)],
            requires_access_token: bool,
            oauth: &[String],
            encryption_key: Option<&Value>,
        ) -> StorageResult<(Box<dyn tokio::io::AsyncBufRead + Send + Unpin>, Option<u64>)> {
            if self.failing_urls.contains(url) {
                return Err(StorageError::Backend(
                    crate::error::BackendError::Internal {
                        backend_name: "bulk-submit-worker-test".to_string(),
                        message: "unavailable input".to_string(),
                        source: None,
                    },
                ));
            }
            self.inner
                .open_file_stream(url, headers, requires_access_token, oauth, encryption_key)
                .await
        }

        async fn file_size(
            &self,
            url: &str,
            headers: &[(String, String)],
            requires_access_token: bool,
            oauth: &[String],
        ) -> StorageResult<Option<u64>> {
            self.inner
                .file_size(url, headers, requires_access_token, oauth)
                .await
        }
    }

    /// Slows only artifact finalization, exercising the worker's live keeper.
    struct SlowFinalize {
        inner: Arc<LocalFsOutputStore>,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl ExportOutputStore for SlowFinalize {
        async fn open_writer(
            &self,
            key: &ExportPartKey,
        ) -> StorageResult<crate::core::bulk_export_output::ExportPartWriter> {
            self.inner.open_writer(key).await
        }

        async fn finalize_part(
            &self,
            key: &ExportPartKey,
            writer: crate::core::bulk_export_output::ExportPartWriter,
        ) -> StorageResult<crate::core::bulk_export_output::FinalizedPart> {
            self.entered.notify_one();
            self.release.notified().await;
            self.inner.finalize_part(key, writer).await
        }

        async fn download_url(
            &self,
            key: &ExportPartKey,
            ttl: Duration,
        ) -> StorageResult<crate::core::bulk_export_output::DownloadUrl> {
            self.inner.download_url(key, ttl).await
        }

        async fn open_reader(
            &self,
            key: &ExportPartKey,
        ) -> StorageResult<std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>> {
            self.inner.open_reader(key).await
        }

        async fn delete_job_outputs(
            &self,
            tenant: &TenantContext,
            job_id: &crate::core::ExportJobId,
        ) -> StorageResult<()> {
            self.inner.delete_job_outputs(tenant, job_id).await
        }
    }

    /// One phase reading: the counters a status poll would have seen, without
    /// the label saying which fetcher call it was taken during.
    type PhaseReading = (Option<ManifestPhase>, u64, u64);

    /// A [`PhaseReading`] tagged with that label.
    type LabelledPhaseReading = (String, Option<ManifestPhase>, u64, u64);

    /// Wraps a [`MockFetcher`] and, on every call the worker makes, reads back
    /// the phase the worker persisted just before it — so a test can assert
    /// what a *concurrent status poll* would have seen, rather than only the
    /// residue left behind at the end of the run.
    struct PhaseSpyFetcher {
        inner: MockFetcher,
        backend: Arc<SqliteBackend>,
        tenant: TenantContext,
        sub_id: SubmissionId,
        seen: std::sync::Mutex<Vec<LabelledPhaseReading>>,
    }

    impl PhaseSpyFetcher {
        async fn observe(&self, during: &str) {
            let manifests = self
                .backend
                .list_manifests(&self.tenant, &self.sub_id)
                .await
                .unwrap();
            let m = &manifests[0];
            self.seen.lock().unwrap().push((
                during.to_string(),
                m.phase,
                m.files_done,
                m.files_total,
            ));
        }

        fn phases_during(&self, during: &str) -> Vec<PhaseReading> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .filter(|(w, _, _, _)| w == during)
                .map(|(_, p, d, t)| (*p, *d, *t))
                .collect()
        }
    }

    #[async_trait]
    impl SubmitInputFetcher for PhaseSpyFetcher {
        async fn fetch_manifest(
            &self,
            url: &str,
            headers: &[(String, String)],
            oauth: &[String],
            encryption_key: Option<&Value>,
        ) -> StorageResult<RemoteManifest> {
            self.observe("fetch_manifest").await;
            self.inner
                .fetch_manifest(url, headers, oauth, encryption_key)
                .await
        }

        async fn open_file_stream(
            &self,
            url: &str,
            headers: &[(String, String)],
            requires_access_token: bool,
            oauth: &[String],
            encryption_key: Option<&Value>,
        ) -> StorageResult<(Box<dyn tokio::io::AsyncBufRead + Send + Unpin>, Option<u64>)> {
            self.observe("open_file_stream").await;
            self.inner
                .open_file_stream(url, headers, requires_access_token, oauth, encryption_key)
                .await
        }

        async fn file_size(
            &self,
            url: &str,
            headers: &[(String, String)],
            requires_access_token: bool,
            oauth: &[String],
        ) -> StorageResult<Option<u64>> {
            let size = self
                .inner
                .file_size(url, headers, requires_access_token, oauth)
                .await;
            self.observe("file_size").await;
            size
        }
    }

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
    }

    /// #953: everything before the first NDJSON batch used to poll as a flat
    /// `0%`. The worker must now publish a phase at each pre-ingest step, so
    /// the status endpoint has something to report during that window.
    #[tokio::test]
    async fn test_worker_reports_its_pre_ingest_phases() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost:8080",
        ));

        let tenant = tenant();
        let sub_id = SubmissionId::generate("mock-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        backend
            .add_manifest(
                &tenant,
                &sub_id,
                Some("http://provider/manifest.json"),
                None,
            )
            .await
            .unwrap();

        // Distinct resource types: the fan-out assumes a manifest's files do
        // not collide on type.
        let types = ["Patient", "Observation", "Condition"];
        let mut files = std::collections::HashMap::new();
        let mut output_files = Vec::new();
        for ty in types {
            let url = format!("http://provider/{ty}.ndjson");
            files.insert(
                url.clone(),
                format!("{{\"resourceType\":\"{ty}\",\"id\":\"x1\"}}\n").into_bytes(),
            );
            output_files.push(RemoteFile {
                resource_type: Some(ty.to_string()),
                url,
                count: None,
            });
        }
        let fetcher = Arc::new(PhaseSpyFetcher {
            inner: MockFetcher {
                files,
                manifest: RemoteManifest {
                    requires_access_token: false,
                    output: output_files,
                    deleted: vec![],
                },
            },
            backend: backend.clone(),
            tenant: tenant.clone(),
            sub_id: sub_id.clone(),
            seen: std::sync::Mutex::new(Vec::new()),
        });

        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            fetcher.clone(),
            output,
            WorkerId::new("phase-worker"),
        );
        let lease = backend
            .claim_next_manifest(&WorkerId::new("phase-worker"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .expect("claimable manifest");
        worker.run_job(lease).await.unwrap();

        // While the remote manifest downloads, the phase says so — and carries
        // no file counts, because the `output` array is still unread.
        assert_eq!(
            fetcher.phases_during("fetch_manifest"),
            vec![(Some(ManifestPhase::ReadingManifest), 0, 0)],
            "the manifest fetch must report `reading-manifest`"
        );

        // Each HEAD advances `files_done` against the now-known denominator,
        // which is what turns a silent pre-size into `sizing N of M files`.
        // The counts run 0..2 rather than 1..3 because the spy reads the row
        // from inside the HEAD, i.e. just *before* the worker records that
        // file's own increment: what it captures is the run of distinct values
        // a status poll could have landed on mid-pre-size.
        assert_eq!(
            fetcher.phases_during("file_size"),
            vec![
                (Some(ManifestPhase::Sizing), 0, 3),
                (Some(ManifestPhase::Sizing), 1, 3),
                (Some(ManifestPhase::Sizing), 2, 3),
            ],
            "pre-sizing must count files against the manifest's output total"
        );

        // Opening a file reports `downloading`, restarting the counter — which
        // is why the phase write overwrites rather than accumulating.
        let downloads = fetcher.phases_during("open_file_stream");
        assert_eq!(downloads.len(), 3);
        for (i, (phase, done, total)) in downloads.iter().enumerate() {
            assert_eq!(*phase, Some(ManifestPhase::Downloading));
            assert_eq!(*done, i as u64 + 1);
            assert_eq!(*total, 3);
        }

        // The phase outlives the run; the status endpoint is what keeps a
        // stale one harmless, by letting the byte/entry counters outrank it.
        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        assert_eq!(manifests[0].phase, Some(ManifestPhase::Downloading));
        assert_eq!(manifests[0].files_total, 3);
    }

    /// The pre-size HEADs run `file_concurrency` at a time (#953). Correctness
    /// must not depend on that: the denominator is the sum the serial loop
    /// produced, no matter what order the responses land in.
    #[tokio::test]
    async fn test_presizing_is_correct_with_file_concurrency() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost:8080",
        ));

        let tenant = tenant();
        let sub_id = SubmissionId::generate("mock-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        backend
            .add_manifest(
                &tenant,
                &sub_id,
                Some("http://provider/manifest.json"),
                None,
            )
            .await
            .unwrap();

        let types = [
            "Patient",
            "Observation",
            "Condition",
            "Encounter",
            "Procedure",
            "Immunization",
        ];
        let mut files = std::collections::HashMap::new();
        let mut output_files = Vec::new();
        let mut expected: u64 = 0;
        for (i, ty) in types.iter().enumerate() {
            let url = format!("http://provider/{ty}.ndjson");
            // Different lengths per file, so a dropped or double-counted HEAD
            // cannot coincidentally still sum to `expected`.
            let body = format!("{{\"resourceType\":\"{ty}\",\"id\":\"c{i}\"}}\n");
            expected += body.len() as u64;
            files.insert(url.clone(), body.into_bytes());
            output_files.push(RemoteFile {
                resource_type: Some((*ty).to_string()),
                url,
                count: None,
            });
        }
        let fetcher = Arc::new(MockFetcher {
            files,
            manifest: RemoteManifest {
                requires_access_token: false,
                output: output_files,
                deleted: vec![],
            },
        });

        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            fetcher,
            output,
            WorkerId::new("concurrent-presize"),
        )
        .with_file_concurrency(4);
        let lease = backend
            .claim_next_manifest(
                &WorkerId::new("concurrent-presize"),
                StdDuration::from_secs(60),
            )
            .await
            .unwrap()
            .expect("claimable manifest");
        worker.run_job(lease).await.unwrap();

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        assert_eq!(
            manifests[0].bytes_total, expected,
            "concurrent HEADs must sum to the same denominator as the serial loop"
        );
        assert_eq!(manifests[0].bytes_processed, expected);
        assert_eq!(manifests[0].files_total, types.len() as u64);
    }

    #[tokio::test]
    async fn test_worker_ingests_output_and_records_artifacts() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost:8080",
        ));

        let tenant = tenant();
        let sub_id = SubmissionId::generate("mock-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        backend
            .add_manifest(
                &tenant,
                &sub_id,
                Some("http://provider/manifest.json"),
                None,
            )
            .await
            .unwrap();

        let ndjson = concat!(
            "{\"resourceType\":\"Patient\",\"id\":\"p1\",\"name\":[{\"family\":\"A\"}]}\n",
            "{\"resourceType\":\"Patient\",\"name\":[{\"family\":\"B\"}]}\n"
        );
        let mut files = std::collections::HashMap::new();
        files.insert(
            "http://provider/patient.ndjson".to_string(),
            ndjson.as_bytes().to_vec(),
        );
        let fetcher = Arc::new(MockFetcher {
            files,
            manifest: RemoteManifest {
                requires_access_token: false,
                output: vec![RemoteFile {
                    resource_type: Some("Patient".to_string()),
                    url: "http://provider/patient.ndjson".to_string(),
                    count: Some(2),
                }],
                deleted: vec![],
            },
        });

        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            fetcher,
            output,
            WorkerId::new("test-worker"),
        );

        let lease = backend
            .claim_next_manifest(&WorkerId::new("test-worker"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .expect("claimable manifest");
        worker.run_job(lease).await.unwrap();

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        let manifest_id = manifests[0].manifest_id.clone();

        // Both Patients ingested (one with id, one assigned a new id).
        let counts = backend
            .get_entry_counts(&tenant, &sub_id, &manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.success, 2);

        // Manifest marked completed.
        assert_eq!(
            manifests[0].status,
            crate::core::bulk_submit::ManifestStatus::Completed
        );

        // Byte progress reached the file's full advertised size.
        assert_eq!(manifests[0].bytes_total, ndjson.len() as u64);
        assert_eq!(manifests[0].bytes_processed, ndjson.len() as u64);
    }

    /// A minimal secondary that rejects `create`/`create_many` for one
    /// hard-coded id, and accepts everything else — the composite-sync half
    /// of the worker tests below.
    ///
    /// `received` tracks every id `create`/`create_or_update` was called
    /// with, by resource type, whether or not it was accepted — the
    /// primary really does hold a rejected resource (only its *search*
    /// indexing failed), so `count` reporting what was received, not just
    /// what was accepted, keeps the rejection test's own index-drift check
    /// from misreporting the rejection `mark_entries_unindexed` already
    /// names. `count_override`, when set, replaces that with a fixed value —
    /// for forcing a drift scenario deterministically.
    struct RejectingSecondary {
        reject_id: &'static str,
        received:
            std::sync::Mutex<std::collections::HashMap<String, std::collections::HashSet<String>>>,
        count_override: Option<u64>,
    }

    impl RejectingSecondary {
        /// A secondary that only rejects `reject_id` and otherwise reports
        /// its real received count.
        fn new(reject_id: &'static str) -> Self {
            Self {
                reject_id,
                received: std::sync::Mutex::new(std::collections::HashMap::new()),
                count_override: None,
            }
        }

        /// A secondary that accepts everything but always reports
        /// `count_override`, to force an index-drift scenario.
        fn with_count_override(count_override: u64) -> Self {
            Self {
                reject_id: "",
                received: std::sync::Mutex::new(std::collections::HashMap::new()),
                count_override: Some(count_override),
            }
        }
    }

    #[async_trait]
    impl ResourceStorage for RejectingSecondary {
        fn backend_name(&self) -> &'static str {
            "rejecting-secondary"
        }

        async fn create(
            &self,
            _tenant: &TenantContext,
            resource_type: &str,
            resource: Value,
            fhir_version: FhirVersion,
        ) -> StorageResult<crate::types::StoredResource> {
            let id = resource
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            self.received
                .lock()
                .unwrap()
                .entry(resource_type.to_string())
                .or_default()
                .insert(id.clone());
            if id == self.reject_id {
                return Err(crate::error::StorageError::Backend(
                    crate::error::BackendError::Internal {
                        backend_name: "rejecting-secondary".to_string(),
                        message: format!("secondary rejected {resource_type}/{id}"),
                        source: None,
                    },
                ));
            }
            Ok(crate::types::StoredResource::new(
                resource_type,
                id,
                TenantId::new("t1"),
                resource,
                fhir_version,
            ))
        }

        async fn create_or_update(
            &self,
            _tenant: &TenantContext,
            resource_type: &str,
            id: &str,
            resource: Value,
            fhir_version: FhirVersion,
        ) -> StorageResult<(crate::types::StoredResource, bool)> {
            self.received
                .lock()
                .unwrap()
                .entry(resource_type.to_string())
                .or_default()
                .insert(id.to_string());
            Ok((
                crate::types::StoredResource::new(
                    resource_type,
                    id,
                    TenantId::new("t1"),
                    resource,
                    fhir_version,
                ),
                true,
            ))
        }

        async fn read(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<Option<crate::types::StoredResource>> {
            Ok(None)
        }

        async fn update(
            &self,
            _tenant: &TenantContext,
            current: &crate::types::StoredResource,
            resource: Value,
        ) -> StorageResult<crate::types::StoredResource> {
            Ok(crate::types::StoredResource::new(
                current.resource_type(),
                current.id(),
                TenantId::new("t1"),
                resource,
                current.fhir_version(),
            ))
        }

        async fn delete(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<()> {
            Ok(())
        }

        async fn count(
            &self,
            _tenant: &TenantContext,
            resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            if let Some(n) = self.count_override {
                return Ok(n);
            }
            let ty = resource_type.unwrap_or("");
            Ok(self
                .received
                .lock()
                .unwrap()
                .get(ty)
                .map(|ids| ids.len())
                .unwrap_or(0) as u64)
        }
    }

    /// Reads back a finalized status-manifest artifact's NDJSON lines
    /// directly from [`LocalFsOutputStore`]'s layout (mirrors
    /// `LocalFsOutputStore::part_path`, which is private to that module).
    fn read_submit_file_lines(
        tmp_root: &std::path::Path,
        tenant_id: &str,
        job_id: &str,
        row: &crate::core::bulk_submit_worker::SubmitFileRow,
    ) -> Vec<String> {
        // The on-disk name is keyed by `row.file_path`, the opaque locator
        // `submit_artifact_key` derives from the artifact's full identity
        // (#1045) — not the human-readable `resource_type`, which several
        // rows of the same manifest can share.
        let path = tmp_root.join(tenant_id).join(job_id).join(format!(
            "{}-{}-{}-{}.ndjson",
            row.file_type, row.file_path, row.part_index, row.fencing_token
        ));
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect()
    }

    /// #1007: a resource the primary ingests but a secondary search index
    /// rejects (after retries) must not read `success` in the manifest's
    /// receipt — its entry result becomes `processing-error` with an
    /// OperationOutcome naming the resource and the `$reindex` repair, and
    /// the manifest still reaches a terminal state.
    #[tokio::test]
    async fn test_worker_reports_unindexed_resources_in_the_receipt() {
        use crate::composite::{CompositeConfig, CompositeStorage, CompositeSubmitJobs};
        use crate::core::BackendKind;
        use crate::core::bulk_submit::BulkEntryOutcome;
        use crate::core::bulk_submit_worker::BulkSubmitJobStore;

        let sqlite = Arc::new(SqliteBackend::in_memory().unwrap());
        sqlite.init_schema().unwrap();
        let config = CompositeConfig::builder()
            .primary("sqlite", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .sync_mode(crate::composite::config::SyncMode::Synchronous)
            .build()
            .unwrap();
        let mut backends: std::collections::HashMap<String, crate::composite::DynStorage> =
            std::collections::HashMap::new();
        backends.insert(
            "sqlite".to_string(),
            sqlite.clone() as crate::composite::DynStorage,
        );
        backends.insert(
            "es".to_string(),
            Arc::new(RejectingSecondary::new("reject-1")) as crate::composite::DynStorage,
        );
        let composite = Arc::new(CompositeStorage::new(config, backends).unwrap());
        let jobs = Arc::new(CompositeSubmitJobs::new(
            sqlite.clone() as Arc<dyn BulkSubmitJobStore>,
            composite,
        ));

        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost:8080",
        ));

        let tenant = tenant();
        let sub_id = SubmissionId::generate("mock-system");
        jobs.create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        jobs.add_manifest(
            &tenant,
            &sub_id,
            Some("http://provider/manifest.json"),
            None,
        )
        .await
        .unwrap();

        let ndjson = concat!(
            "{\"resourceType\":\"Patient\",\"id\":\"ok-1\"}\n",
            "{\"resourceType\":\"Patient\",\"id\":\"reject-1\"}\n"
        );
        let mut files = std::collections::HashMap::new();
        files.insert(
            "http://provider/patient.ndjson".to_string(),
            ndjson.as_bytes().to_vec(),
        );
        let fetcher = Arc::new(MockFetcher {
            files,
            manifest: RemoteManifest {
                requires_access_token: false,
                output: vec![RemoteFile {
                    resource_type: Some("Patient".to_string()),
                    url: "http://provider/patient.ndjson".to_string(),
                    count: Some(2),
                }],
                deleted: vec![],
            },
        });

        let worker = DefaultSubmitWorker::new(
            jobs.clone(),
            fetcher,
            output,
            WorkerId::new("unindexed-worker"),
        );
        let lease = jobs
            .claim_next_manifest(
                &WorkerId::new("unindexed-worker"),
                StdDuration::from_secs(60),
            )
            .await
            .unwrap()
            .expect("claimable manifest");
        worker.run_job(lease).await.unwrap();

        let manifests = jobs.list_manifests(&tenant, &sub_id).await.unwrap();
        assert!(
            manifests[0].status.is_terminal(),
            "the manifest must still reach a terminal state"
        );

        let job_id = submission_output_job_id(&sub_id);
        let tenant_id = tenant.tenant_id().as_str();
        let submit_files = jobs.list_submit_files(&tenant, &sub_id).await.unwrap();

        let output_row = submit_files
            .iter()
            .find(|f| f.file_type == "output" && f.resource_type.as_deref() == Some("Patient"))
            .expect("output receipt for Patient");
        let output_lines =
            read_submit_file_lines(tmp.path(), tenant_id, job_id.as_str(), output_row);
        assert_eq!(
            output_lines,
            vec![serde_json::json!({"reference": "Patient/ok-1"}).to_string()],
            "only the accepted resource is a success receipt"
        );

        let error_row = submit_files
            .iter()
            .find(|f| f.file_type == "error")
            .expect("error receipt for the rejected resource");
        let error_lines = read_submit_file_lines(tmp.path(), tenant_id, job_id.as_str(), error_row);
        assert_eq!(error_lines.len(), 1, "exactly one rejected resource");
        let oo: Value = serde_json::from_str(&error_lines[0]).unwrap();
        assert_eq!(oo["issue"][0]["code"], "incomplete");
        let diagnostics = oo["issue"][0]["diagnostics"].as_str().unwrap();
        assert!(diagnostics.contains("Patient/reject-1"));
        assert!(diagnostics.contains("$reindex"));
        assert_eq!(
            error_row.count_severity,
            Some(serde_json::json!({"error": 1}))
        );

        let counts = jobs
            .get_entry_counts(&tenant, &sub_id, &manifests[0].manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.success, 1);
        assert_eq!(counts.processing_error, 1);
        let page = jobs
            .get_entry_results_page(&tenant, &sub_id, &manifests[0].manifest_id, None, 10, None)
            .await
            .unwrap();
        let rejected = page
            .entries
            .iter()
            .map(|e| &e.result)
            .find(|r| r.resource_id.as_deref() == Some("reject-1"))
            .expect("rejected entry present");
        assert_eq!(rejected.outcome, BulkEntryOutcome::ProcessingError);
    }

    /// #1007: a tenant-wide count mismatch between the primary and a
    /// secondary search index, discovered right after a manifest's sync, is
    /// written to the receipt as a `warning` OperationOutcome naming the
    /// `$reindex` repair — it must not be mistaken for a failed entry: every
    /// ingested resource still reads `success`, and the manifest's
    /// `failed_entries` does not grow.
    #[tokio::test]
    async fn test_worker_writes_drift_as_a_warning_outcome() {
        use crate::composite::{CompositeConfig, CompositeStorage, CompositeSubmitJobs};
        use crate::core::BackendKind;
        use crate::core::bulk_submit_worker::BulkSubmitJobStore;

        let sqlite = Arc::new(SqliteBackend::in_memory().unwrap());
        sqlite.init_schema().unwrap();
        let config = CompositeConfig::builder()
            .primary("sqlite", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .sync_mode(crate::composite::config::SyncMode::Synchronous)
            .build()
            .unwrap();
        let mut backends: std::collections::HashMap<String, crate::composite::DynStorage> =
            std::collections::HashMap::new();
        backends.insert(
            "sqlite".to_string(),
            sqlite.clone() as crate::composite::DynStorage,
        );
        // Accepts everything but always reports a count of 1, disagreeing
        // with the primary's real count of 2 — forces drift deterministically.
        backends.insert(
            "es".to_string(),
            Arc::new(RejectingSecondary::with_count_override(1)) as crate::composite::DynStorage,
        );
        let composite = Arc::new(CompositeStorage::new(config, backends).unwrap());
        let jobs = Arc::new(CompositeSubmitJobs::new(
            sqlite.clone() as Arc<dyn BulkSubmitJobStore>,
            composite,
        ));

        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost:8080",
        ));

        let tenant = tenant();
        let sub_id = SubmissionId::generate("drift-worker");
        jobs.create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        jobs.add_manifest(
            &tenant,
            &sub_id,
            Some("http://provider/manifest.json"),
            None,
        )
        .await
        .unwrap();

        let ndjson = concat!(
            "{\"resourceType\":\"Patient\",\"id\":\"d-1\"}\n",
            "{\"resourceType\":\"Patient\",\"id\":\"d-2\"}\n"
        );
        let mut files = std::collections::HashMap::new();
        files.insert(
            "http://provider/patient.ndjson".to_string(),
            ndjson.as_bytes().to_vec(),
        );
        let fetcher = Arc::new(MockFetcher {
            files,
            manifest: RemoteManifest {
                requires_access_token: false,
                output: vec![RemoteFile {
                    resource_type: Some("Patient".to_string()),
                    url: "http://provider/patient.ndjson".to_string(),
                    count: Some(2),
                }],
                deleted: vec![],
            },
        });

        let worker =
            DefaultSubmitWorker::new(jobs.clone(), fetcher, output, WorkerId::new("drift-worker"));
        let lease = jobs
            .claim_next_manifest(&WorkerId::new("drift-worker"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .expect("claimable manifest");
        worker.run_job(lease).await.unwrap();

        let manifests = jobs.list_manifests(&tenant, &sub_id).await.unwrap();
        assert!(manifests[0].status.is_terminal());
        assert_eq!(
            manifests[0].failed_entries, 0,
            "a drift is a warning, not a failed entry"
        );

        let job_id = submission_output_job_id(&sub_id);
        let tenant_id = tenant.tenant_id().as_str();
        let submit_files = jobs.list_submit_files(&tenant, &sub_id).await.unwrap();

        let output_row = submit_files
            .iter()
            .find(|f| f.file_type == "output" && f.resource_type.as_deref() == Some("Patient"))
            .expect("output receipt for Patient");
        let output_lines =
            read_submit_file_lines(tmp.path(), tenant_id, job_id.as_str(), output_row);
        assert_eq!(
            output_lines.len(),
            2,
            "a drift warning does not remove anything from the receipt"
        );

        let error_row = submit_files
            .iter()
            .find(|f| f.file_type == "error")
            .expect("error artifact carrying the drift warning");
        let error_lines = read_submit_file_lines(tmp.path(), tenant_id, job_id.as_str(), error_row);
        assert_eq!(error_lines.len(), 1, "exactly one drift warning");
        let oo: Value = serde_json::from_str(&error_lines[0]).unwrap();
        assert_eq!(oo["issue"][0]["severity"], "warning");
        assert_eq!(oo["issue"][0]["code"], "incomplete");
        let diagnostics = oo["issue"][0]["diagnostics"].as_str().unwrap();
        assert!(diagnostics.contains("Patient"));
        assert!(diagnostics.contains("primary holds 2"));
        assert!(diagnostics.contains("$reindex"));
        assert_eq!(
            error_row.count_severity,
            Some(serde_json::json!({"warning": 1}))
        );

        let counts = jobs
            .get_entry_counts(&tenant, &sub_id, &manifests[0].manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.success, 2);
        assert_eq!(counts.processing_error, 0);
    }

    /// #903 fast-load: deferred ingestion stores readable resources that are
    /// invisible to search, fires the reindex hook with the manifest's types,
    /// and the real reindex machinery then makes them searchable.
    #[tokio::test]
    async fn test_deferred_indexing_defers_search_and_fires_the_hook() {
        use crate::search::{ReindexOnFinish, ReindexOperation};
        use crate::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

        // A data dir gives the tenant a real SearchParameter registry — the
        // reindex half of this test extracts through it.
        let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap();
        let backend = Arc::new(
            SqliteBackend::with_config(
                ":memory:",
                crate::backends::sqlite::SqliteBackendConfig {
                    data_dir: Some(data_dir),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost:8080",
        ));

        let tenant = tenant();
        let sub_id = SubmissionId::generate("mock-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        backend
            .add_manifest(
                &tenant,
                &sub_id,
                Some("http://provider/manifest.json"),
                None,
            )
            .await
            .unwrap();

        let ndjson = "{\"resourceType\":\"Patient\",\"id\":\"defer-1\",\"name\":[{\"family\":\"Deferred\"}]}\n";
        let mut files = std::collections::HashMap::new();
        files.insert(
            "http://provider/patient.ndjson".to_string(),
            ndjson.as_bytes().to_vec(),
        );
        let fetcher = Arc::new(MockFetcher {
            files,
            manifest: RemoteManifest {
                requires_access_token: false,
                output: vec![RemoteFile {
                    resource_type: Some("Patient".to_string()),
                    url: "http://provider/patient.ndjson".to_string(),
                    count: Some(1),
                }],
                deleted: vec![],
            },
        });

        let hook = Arc::new(MockReindexHook {
            calls: std::sync::Mutex::new(Vec::new()),
        });
        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            fetcher,
            output,
            WorkerId::new("defer-worker"),
        )
        .with_deferred_indexing(true, Some(hook.clone()));

        let lease = backend
            .claim_next_manifest(&WorkerId::new("defer-worker"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .expect("claimable manifest");
        worker.run_job(lease).await.unwrap();

        // Readable immediately…
        assert!(
            backend
                .read(&tenant, "Patient", "defer-1")
                .await
                .unwrap()
                .is_some()
        );

        // …but invisible to search: the index writes were deferred.
        let by_name = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "family".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Deferred")],
            chain: vec![],
            components: vec![],
        });
        use crate::core::search::SearchProvider;
        let found = backend.search(&tenant, &by_name).await.unwrap();
        assert_eq!(
            found.resources.items.len(),
            0,
            "deferred ingest must not index"
        );

        // The hook received the manifest's resource types.
        assert_eq!(
            hook.calls.lock().unwrap().clone(),
            vec![vec!["Patient".to_string()]]
        );

        // The real hook + reindex machinery restores searchability.
        let op = Arc::new(ReindexOperation::new(
            backend.clone(),
            backend.tenant_registries().clone(),
        ));
        let real_hook = ReindexOnFinish::new(op);
        DeferredReindexHook::reindex_types(&real_hook, &tenant, vec!["Patient".to_string()]).await;
        // The reindex runs as a spawned background job — poll briefly.
        let mut visible = false;
        for _ in 0..50 {
            tokio::time::sleep(StdDuration::from_millis(100)).await;
            if backend
                .search(&tenant, &by_name)
                .await
                .unwrap()
                .resources
                .items
                .len()
                == 1
            {
                visible = true;
                break;
            }
        }
        assert!(
            visible,
            "reindex must make the deferred resources searchable"
        );
    }

    /// #874: with every file's size known up front, the byte denominator is
    /// the full manifest total from the start — the percentage never
    /// recomputes against a partial sum as later files open.
    #[tokio::test]
    async fn test_multi_file_manifest_presizes_the_byte_total() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost:8080",
        ));

        let tenant = tenant();
        let sub_id = SubmissionId::generate("mock-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        backend
            .add_manifest(
                &tenant,
                &sub_id,
                Some("http://provider/manifest.json"),
                None,
            )
            .await
            .unwrap();

        let f1 = "{\"resourceType\":\"Patient\",\"id\":\"m1\"}\n";
        let f2 = "{\"resourceType\":\"Patient\",\"id\":\"m2\"}\n{\"resourceType\":\"Patient\",\"id\":\"m3\"}\n";
        let mut files = std::collections::HashMap::new();
        files.insert(
            "http://provider/a.ndjson".to_string(),
            f1.as_bytes().to_vec(),
        );
        files.insert(
            "http://provider/b.ndjson".to_string(),
            f2.as_bytes().to_vec(),
        );
        let fetcher = Arc::new(MockFetcher {
            files,
            manifest: RemoteManifest {
                requires_access_token: false,
                output: vec![
                    RemoteFile {
                        resource_type: Some("Patient".to_string()),
                        url: "http://provider/a.ndjson".to_string(),
                        count: None,
                    },
                    RemoteFile {
                        resource_type: Some("Patient".to_string()),
                        url: "http://provider/b.ndjson".to_string(),
                        count: None,
                    },
                ],
                deleted: vec![],
            },
        });

        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            fetcher,
            output,
            WorkerId::new("presize-worker"),
        );
        let lease = backend
            .claim_next_manifest(&WorkerId::new("presize-worker"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .expect("claimable manifest");
        worker.run_job(lease).await.unwrap();

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        let expected = (f1.len() + f2.len()) as u64;
        assert_eq!(manifests[0].bytes_total, expected);
        assert_eq!(manifests[0].bytes_processed, expected);

        // An `output` artifact for Patient was recorded.
        let files = backend.list_submit_files(&tenant, &sub_id).await.unwrap();
        assert!(
            files
                .iter()
                .any(|f| f.file_type == "output" && f.resource_type.as_deref() == Some("Patient"))
        );
    }

    #[tokio::test]
    async fn test_fan_out_ingests_every_file_of_a_multi_file_manifest() {
        // Fan-out (file_concurrency > 1) must ingest all of a manifest's files —
        // each carrying a distinct resource type — with the same result as the
        // sequential path: every resource stored, and the counts complete.
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost:8080",
        ));

        let tenant = tenant();
        let sub_id = SubmissionId::generate("mock-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        backend
            .add_manifest(
                &tenant,
                &sub_id,
                Some("http://provider/manifest.json"),
                None,
            )
            .await
            .unwrap();

        // Six files of six distinct types, two resources each.
        let types = [
            "Patient",
            "Observation",
            "Condition",
            "Encounter",
            "Procedure",
            "Immunization",
        ];
        let mut files = std::collections::HashMap::new();
        let mut output_files = Vec::new();
        for t in types {
            let url = format!("http://provider/{t}.ndjson");
            let body = format!(
                "{{\"resourceType\":\"{t}\",\"id\":\"{t}-1\"}}\n{{\"resourceType\":\"{t}\",\"id\":\"{t}-2\"}}\n"
            );
            files.insert(url.clone(), body.into_bytes());
            output_files.push(RemoteFile {
                resource_type: Some(t.to_string()),
                url,
                count: None,
            });
        }
        let fetcher = Arc::new(MockFetcher {
            files,
            manifest: RemoteManifest {
                requires_access_token: false,
                output: output_files,
                deleted: vec![],
            },
        });

        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            fetcher,
            output,
            WorkerId::new("fanout-worker"),
        )
        .with_file_concurrency(4);
        let lease = backend
            .claim_next_manifest(&WorkerId::new("fanout-worker"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .expect("claimable manifest");
        worker.run_job(lease).await.unwrap();

        // Every resource of every type is stored.
        for t in types {
            let count = backend.count(&tenant, Some(t)).await.unwrap();
            assert_eq!(count, 2, "expected 2 {t} resources after fan-out ingest");
        }

        // The manifest's terminal counts cover all 12 entries with no failures.
        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        assert_eq!(manifests[0].processed_entries, 12);
        assert_eq!(manifests[0].failed_entries, 0);

        // One `output` receipt per type.
        let receipts = backend.list_submit_files(&tenant, &sub_id).await.unwrap();
        for t in types {
            assert!(
                receipts
                    .iter()
                    .any(|f| f.file_type == "output" && f.resource_type.as_deref() == Some(t)),
                "missing output receipt for {t}"
            );
        }
    }

    /// A fetcher whose `fetch_manifest` always fails (unreachable / bad manifest).
    struct FailingManifestFetcher;
    #[async_trait]
    impl SubmitInputFetcher for FailingManifestFetcher {
        async fn fetch_manifest(
            &self,
            _url: &str,
            _h: &[(String, String)],
            _o: &[String],
            _k: Option<&Value>,
        ) -> StorageResult<RemoteManifest> {
            Err(crate::error::StorageError::Backend(
                crate::error::BackendError::Internal {
                    backend_name: "test".into(),
                    message: "unreachable manifest".into(),
                    source: None,
                },
            ))
        }
        async fn open_file_stream(
            &self,
            _url: &str,
            _h: &[(String, String)],
            _r: bool,
            _o: &[String],
            _k: Option<&Value>,
        ) -> StorageResult<(Box<dyn tokio::io::AsyncBufRead + Send + Unpin>, Option<u64>)> {
            Ok((
                Box::new(tokio::io::BufReader::new(std::io::Cursor::new(Vec::new()))),
                Some(0),
            ))
        }
    }

    async fn seed(backend: &Arc<SqliteBackend>, tenant: &TenantContext) -> SubmissionId {
        let sub_id = SubmissionId::generate("mock-system");
        backend
            .create_submission(tenant, &sub_id, None)
            .await
            .unwrap();
        backend
            .add_manifest(tenant, &sub_id, Some("http://provider/manifest.json"), None)
            .await
            .unwrap();
        sub_id
    }

    #[tokio::test]
    async fn test_worker_fails_manifest_on_fetch_error() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://x",
        ));
        let tenant = tenant();
        let sub_id = seed(&backend, &tenant).await;

        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            Arc::new(FailingManifestFetcher),
            output,
            WorkerId::new("w"),
        );
        let lease = backend
            .claim_next_manifest(&WorkerId::new("w"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        // A bad manifest fails only that manifest (worker returns Ok).
        worker.run_job(lease).await.unwrap();

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        assert_eq!(
            manifests[0].status,
            crate::core::bulk_submit::ManifestStatus::Failed
        );
        // A manifest-level error artifact was recorded.
        let files = backend.list_submit_files(&tenant, &sub_id).await.unwrap();
        assert!(files.iter().any(|f| f.file_type == "error"));
    }

    /// Seeds a submission whose single manifest carries the given `import` directives.
    async fn seed_with_import(
        backend: &Arc<SqliteBackend>,
        tenant: &TenantContext,
        directives: &[(String, String)],
    ) -> SubmissionId {
        let sub_id = SubmissionId::generate("mock-system");
        backend
            .create_submission(tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(tenant, &sub_id, Some("http://provider/manifest.json"), None)
            .await
            .unwrap();
        backend
            .set_manifest_fetch_params(
                tenant,
                &sub_id,
                &manifest.manifest_id,
                ManifestFetchParams {
                    import_directives: directives,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        sub_id
    }

    /// A fetcher serving one NDJSON file of `Patient` resources.
    fn patient_fetcher(ndjson: &str) -> Arc<MockFetcher> {
        let mut files = std::collections::HashMap::new();
        files.insert(
            "http://provider/p.ndjson".to_string(),
            ndjson.as_bytes().to_vec(),
        );
        Arc::new(MockFetcher {
            files,
            manifest: RemoteManifest {
                requires_access_token: false,
                output: vec![RemoteFile {
                    resource_type: Some("Patient".to_string()),
                    url: "http://provider/p.ndjson".to_string(),
                    count: Some(1),
                }],
                deleted: vec![],
            },
        })
    }

    /// Ingests a partial Patient over an existing one and returns the stored content.
    async fn ingest_over_existing(directives: &[(String, String)]) -> Value {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://x",
        ));
        let tenant = tenant();

        // An existing Patient carrying a name the submission never mentions.
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "p1",
                    "name": [{"family": "Original"}],
                    "gender": "female"
                }),
                helios_fhir::FhirVersion::default_enabled(),
            )
            .await
            .unwrap();

        let sub_id = seed_with_import(&backend, &tenant, directives).await;
        let fetcher =
            patient_fetcher("{\"resourceType\":\"Patient\",\"id\":\"p1\",\"gender\":\"male\"}\n");
        let worker = DefaultSubmitWorker::new(backend.clone(), fetcher, output, WorkerId::new("w"));
        let lease = backend
            .claim_next_manifest(&WorkerId::new("w"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        worker.run_job(lease).await.unwrap();

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        assert_eq!(
            backend
                .get_entry_counts(&tenant, &sub_id, &manifests[0].manifest_id)
                .await
                .unwrap()
                .success,
            1
        );
        backend
            .read(&tenant, "Patient", "p1")
            .await
            .unwrap()
            .expect("patient still stored")
            .content()
            .clone()
    }

    #[tokio::test]
    async fn test_import_mode_merge_retains_unsubmitted_elements() {
        let stored = ingest_over_existing(&[(
            crate::core::bulk_submit::IMPORT_MODE_PARAMETER_URL.to_string(),
            "merge".to_string(),
        )])
        .await;
        assert_eq!(stored["gender"], json!("male"));
        assert_eq!(stored["name"], json!([{"family": "Original"}]));
    }

    #[tokio::test]
    async fn test_import_mode_replace_is_the_default() {
        // Explicit `replace` and no directive at all must behave identically:
        // the submitted resource wins wholesale.
        for directives in [
            vec![(
                crate::core::bulk_submit::IMPORT_MODE_PARAMETER_URL.to_string(),
                "replace".to_string(),
            )],
            vec![],
        ] {
            let stored = ingest_over_existing(&directives).await;
            assert_eq!(stored["gender"], json!("male"));
            assert!(
                stored.get("name").is_none(),
                "replace must not retain unsubmitted elements, got {stored}"
            );
        }
    }

    #[tokio::test]
    async fn test_worker_partial_success_on_invalid_ndjson() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://x",
        ));
        let tenant = tenant();
        let sub_id = seed(&backend, &tenant).await;

        // One valid Patient, one malformed JSON line.
        let ndjson = "{\"resourceType\":\"Patient\",\"id\":\"ok\"}\nnot-json\n";
        let mut files = std::collections::HashMap::new();
        files.insert(
            "http://provider/p.ndjson".to_string(),
            ndjson.as_bytes().to_vec(),
        );
        let fetcher = Arc::new(MockFetcher {
            files,
            manifest: RemoteManifest {
                requires_access_token: false,
                output: vec![RemoteFile {
                    resource_type: Some("Patient".to_string()),
                    url: "http://provider/p.ndjson".to_string(),
                    count: Some(2),
                }],
                deleted: vec![],
            },
        });
        let worker = DefaultSubmitWorker::new(backend.clone(), fetcher, output, WorkerId::new("w"));
        let lease = backend
            .claim_next_manifest(&WorkerId::new("w"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        worker.run_job(lease).await.unwrap();

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        let counts = backend
            .get_entry_counts(&tenant, &sub_id, &manifests[0].manifest_id)
            .await
            .unwrap();
        // Partial success: one ingested; the malformed line is counted as a
        // failure on the manifest and surfaced as a summary error artifact, and
        // the manifest still completes.
        assert_eq!(counts.success, 1);
        // Exactly one: the malformed line, charged by the worker because no
        // batch saw it, and by nobody else (#969).
        assert_eq!(manifests[0].failed_entries, 1);
        assert_eq!(manifests[0].processed_entries, 1);
        assert_eq!(
            manifests[0].status,
            crate::core::bulk_submit::ManifestStatus::Completed
        );
        let files = backend.list_submit_files(&tenant, &sub_id).await.unwrap();
        assert!(files.iter().any(|f| f.file_type == "error"));
        assert!(files.iter().any(|f| f.file_type == "output"));
    }

    /// A manifest reclaimed after a worker died keeps the progress its earlier
    /// run recorded (#969).
    ///
    /// The worker used to overwrite the counters with the current run's
    /// absolute totals, so a manifest six million entries in reported a single
    /// entry the moment it was re-walked.
    #[tokio::test]
    async fn test_worker_progress_survives_a_reclaimed_manifest() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://x",
        ));
        let tenant = tenant();
        let sub_id = seed(&backend, &tenant).await;

        let lease = backend
            .claim_next_manifest(&WorkerId::new("w"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        // Stands in for everything an earlier, interrupted run had ingested.
        backend
            .add_manifest_progress(&lease, 6_034_873, 7, 5_564_073)
            .await
            .unwrap();
        assert_eq!(
            backend
                .get_manifest_for_worker(&lease)
                .await
                .unwrap()
                .last_processed_line,
            5_564_073
        );

        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            patient_fetcher("{\"resourceType\":\"Patient\",\"id\":\"p1\"}\n"),
            output,
            WorkerId::new("w"),
        );
        worker.run_job(lease).await.unwrap();

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        assert_eq!(
            manifests[0].processed_entries, 6_034_874,
            "the re-walked entry must add to the earlier run's total, not replace it"
        );
        assert_eq!(manifests[0].failed_entries, 7);
    }

    #[tokio::test]
    async fn concurrent_file_errors_use_deterministic_source_indexes() {
        use tokio::io::AsyncReadExt;

        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let tenant = tenant();
        let sub_id = SubmissionId::generate("concurrent-errors");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let _manifest = backend
            .add_manifest(
                &tenant,
                &sub_id,
                Some("http://provider/concurrent-errors.json"),
                None,
            )
            .await
            .unwrap();
        let lease = backend
            .claim_next_manifest(
                &WorkerId::new("concurrent-errors-worker"),
                StdDuration::from_secs(60),
            )
            .await
            .unwrap()
            .unwrap();

        let mut failing_urls = std::collections::BTreeSet::new();
        let mut output_files = Vec::new();
        for index in 0..3 {
            let url = format!("http://provider/fail-{index}.ndjson");
            output_files.push(RemoteFile {
                resource_type: Some("Patient".to_string()),
                url: url.clone(),
                count: Some(1),
            });
            failing_urls.insert(url);
        }
        let deleted_url = "http://provider/deleted.ndjson".to_string();
        failing_urls.insert(deleted_url.clone());
        let fetcher = Arc::new(SelectiveFailureFetcher {
            inner: MockFetcher {
                files: std::collections::HashMap::new(),
                manifest: RemoteManifest {
                    requires_access_token: false,
                    output: output_files,
                    deleted: vec![RemoteFile {
                        resource_type: Some("Bundle".to_string()),
                        url: deleted_url.clone(),
                        count: Some(1),
                    }],
                },
            },
            failing_urls,
        });
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().join("objects"),
            "http://localhost",
        ));
        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            fetcher,
            output.clone(),
            WorkerId::new("concurrent-errors-worker"),
        )
        .with_file_concurrency(3);
        worker.run_job(lease.clone()).await.unwrap();

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        assert_eq!(manifests[0].status, ManifestStatus::Completed);
        let errors = backend
            .list_submit_files(&tenant, &sub_id)
            .await
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>();
        assert_eq!(errors.len(), 5);
        assert!(errors.iter().all(|row| row.file_type == "error"
            && row.resource_type.as_deref() == Some("OperationOutcome")
            && row.line_count == 1));
        let part_indexes = errors
            .iter()
            .map(|row| row.part_index)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            part_indexes.into_iter().collect::<Vec<_>>(),
            vec![0, 2, 3, 4, 5]
        );
        let locators = errors
            .iter()
            .map(|row| row.file_path.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(locators.len(), 5);

        for row in errors {
            let expected_url = match row.part_index {
                0 => "",
                2 => "http://provider/fail-0.ndjson",
                3 => "http://provider/fail-1.ndjson",
                4 => "http://provider/fail-2.ndjson",
                5 => deleted_url.as_str(),
                _ => panic!("unexpected error part {}", row.part_index),
            };
            assert_eq!(row.count_severity, Some(json!({"error": 1})));

            let key = ExportPartKey {
                tenant_id: tenant.tenant_id().as_str().to_string(),
                job_id: submission_output_job_id(&sub_id),
                resource_type: row.file_path.clone(),
                file_type: row.file_type.clone(),
                part_index: row.part_index,
                fencing_token: row.fencing_token,
            };
            let mut reader = output.open_reader(&key).await.unwrap();
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await.unwrap();
            assert_eq!(row.byte_count, bytes.len() as u64);
            let outcome: Value =
                serde_json::from_str(std::str::from_utf8(&bytes).unwrap()).unwrap();
            let diagnostic = outcome["issue"][0]["diagnostics"].as_str().unwrap();
            if row.part_index == 0 {
                assert!(diagnostic.contains("3 submitted resource(s)"));
                assert!(diagnostic.contains("could not be parsed"));
            } else {
                assert!(diagnostic.contains(expected_url));
                assert!(diagnostic.contains("unavailable input"));
            }
        }
    }

    #[tokio::test]
    async fn already_published_helper_replays_return_false() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let tenant = tenant();
        let _sub_id = seed(&backend, &tenant).await;
        let lease = backend
            .claim_next_manifest(
                &WorkerId::new("already-published"),
                StdDuration::from_secs(60),
            )
            .await
            .unwrap()
            .unwrap();
        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            patient_fetcher(""),
            Arc::new(LocalFsOutputStore::new(
                tmp.path().to_path_buf(),
                "http://localhost",
            )),
            WorkerId::new("already-published"),
        );

        let keeper = LeaseKeeper::spawn(
            Arc::new(JobStoreRenewal(Arc::clone(&backend))),
            lease.clone(),
            ByteProgress::default(),
            CancelToken::new(),
        );
        let first = worker
            .publish_collected_artifacts(
                &lease,
                keeper,
                Vec::new(),
                ManifestPublicationStatus::Completed,
            )
            .await;
        assert!(first.unwrap());

        let keeper = LeaseKeeper::spawn(
            Arc::new(JobStoreRenewal(Arc::clone(&backend))),
            lease.clone(),
            ByteProgress::default(),
            CancelToken::new(),
        );
        let second = worker
            .publish_collected_artifacts(
                &lease,
                keeper,
                Vec::new(),
                ManifestPublicationStatus::Completed,
            )
            .await;
        assert!(!second.unwrap());
    }

    #[tokio::test]
    async fn slow_finalize_keeps_lease_alive_for_reclaimer() {
        use tokio::io::AsyncReadExt;

        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let tenant = tenant();
        let sub_id = SubmissionId::generate("slow-finalize");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(
                &tenant,
                &sub_id,
                Some("http://provider/slow-finalize.json"),
                None,
            )
            .await
            .unwrap();
        let lease = backend
            .claim_next_manifest(&WorkerId::new("slow-finalize"), StdDuration::from_secs(2))
            .await
            .unwrap()
            .unwrap();

        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let output = Arc::new(SlowFinalize {
            inner: Arc::new(LocalFsOutputStore::new(
                tmp.path().join("objects"),
                "http://localhost",
            )),
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
        });
        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            patient_fetcher("{\"resourceType\":\"Patient\",\"id\":\"slow-finalize\"}\n"),
            output.clone(),
            WorkerId::new("slow-finalize"),
        );
        let run = {
            let lease = lease.clone();
            tokio::spawn(async move { worker.run_job(lease).await })
        };
        tokio::time::timeout(StdDuration::from_secs(10), entered.notified())
            .await
            .unwrap();
        tokio::time::sleep(StdDuration::from_millis(2200)).await;
        assert_eq!(
            backend.list_manifests(&tenant, &sub_id).await.unwrap()[0].status,
            ManifestStatus::Processing
        );
        assert!(
            backend
                .claim_next_manifest(
                    &WorkerId::new("slow-finalize-reclaimer"),
                    StdDuration::from_secs(2),
                )
                .await
                .unwrap()
                .is_none()
        );

        release.notify_one();

        let result = tokio::time::timeout(StdDuration::from_secs(10), run)
            .await
            .unwrap()
            .unwrap();
        result.unwrap();
        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        assert_eq!(manifests[0].status, ManifestStatus::Completed);
        let rows = backend.list_submit_files(&tenant, &sub_id).await.unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(
            (
                row.file_type.as_str(),
                row.resource_type.as_deref(),
                row.part_index,
                row.fencing_token
            ),
            ("output", Some("Patient"), 0, lease.fencing_token)
        );
        let expected_key = submit_artifact_key(
            &tenant,
            &sub_id,
            &manifest.manifest_id,
            "output",
            Some("Patient"),
            0,
            lease.fencing_token,
        );
        assert_eq!(row.file_path, expected_key.resource_type);
        let key = ExportPartKey {
            tenant_id: tenant.tenant_id().as_str().to_string(),
            job_id: submission_output_job_id(&sub_id),
            resource_type: row.file_path.clone(),
            file_type: row.file_type.clone(),
            part_index: row.part_index,
            fencing_token: row.fencing_token,
        };
        let mut reader = output.open_reader(&key).await.unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(row.line_count, 1);
        assert_eq!(row.byte_count, bytes.len() as u64);
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            "{\"reference\":\"Patient/slow-finalize\"}\n"
        );
    }

    /// Losing the lease while receipts are being finalized is quiet, exactly
    /// like losing it mid-file: the run abandons the manifest without
    /// publishing anything, and the worker that took the lease over recovers
    /// the manifest on its own.
    #[tokio::test]
    async fn receipt_finalize_abandons_the_run_when_its_lease_is_taken_over() {
        use tokio::io::AsyncReadExt;

        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let tenant = tenant();
        let sub_id = SubmissionId::generate("receipt-lease-loss");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(
                &tenant,
                &sub_id,
                Some("http://provider/receipt-lease-loss.json"),
                None,
            )
            .await
            .unwrap();
        let lease = backend
            .claim_next_manifest(
                &WorkerId::new("receipt-lease-loss"),
                StdDuration::from_secs(2),
            )
            .await
            .unwrap()
            .unwrap();

        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let slow_output = Arc::new(SlowFinalize {
            inner: Arc::new(LocalFsOutputStore::new(
                tmp.path().join("objects"),
                "http://localhost",
            )),
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
        });
        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            patient_fetcher("{\"resourceType\":\"Patient\",\"id\":\"receipt-lease-loss\"}\n"),
            Arc::clone(&slow_output),
            WorkerId::new("receipt-lease-loss"),
        );
        let run = {
            let lease = lease.clone();
            tokio::spawn(async move { worker.run_job(lease).await })
        };
        tokio::time::timeout(StdDuration::from_secs(10), entered.notified())
            .await
            .expect("the run has to reach receipt finalization");

        // Expire the lease under the blocked finalize and hand it over, as a
        // reclaimer would once the heartbeat window has passed. Rewriting the
        // worker also fails the abandoned run's next heartbeat.
        backend
            .get_connection()
            .unwrap()
            .execute(
                "UPDATE bulk_manifests
                 SET worker_id = 'receipt-lease-loss-reclaimer',
                     lease_expiry = '1970-01-01T00:00:00Z'
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                   AND manifest_id = ?4",
                params![
                    tenant.tenant_id().as_str(),
                    sub_id.submitter,
                    sub_id.submission_id,
                    manifest.manifest_id,
                ],
            )
            .unwrap();
        let replacement = backend
            .claim_next_manifest(
                &WorkerId::new("receipt-lease-loss-reclaimer"),
                StdDuration::from_secs(60),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(replacement.manifest_id, manifest.manifest_id);
        assert!(replacement.fencing_token > lease.fencing_token);

        let abandoned = tokio::time::timeout(StdDuration::from_secs(5), run)
            .await
            .expect("the run must abandon a manifest it no longer holds")
            .unwrap();
        abandoned.unwrap();
        assert_eq!(
            backend.list_manifests(&tenant, &sub_id).await.unwrap()[0].status,
            ManifestStatus::Processing,
            "the reclaimer owns the manifest now"
        );
        assert!(
            backend
                .list_submit_files(&tenant, &sub_id)
                .await
                .unwrap()
                .is_empty(),
            "a lease-lost run publishes neither output nor error parts"
        );
        // The abandoned run never finalized its part. A scratch file under the
        // output store may survive the cancellation — that is the store's
        // pre-existing behavior — but the finalized artifact must not.
        let abandoned_key = submit_artifact_key(
            &tenant,
            &sub_id,
            &manifest.manifest_id,
            "output",
            Some("Patient"),
            0,
            lease.fencing_token,
        );
        assert!(
            slow_output.open_reader(&abandoned_key).await.is_err(),
            "the abandoned run must not leave a finalized part behind"
        );

        // The reclaimer recovers the manifest normally: exact references, its
        // own fencing token. It shares the abandoned run's store root on
        // purpose — the fencing token in the part locator is what keeps the two
        // runs' artifacts apart. The slow store never finished a part, so
        // nothing has to release it.
        let live_output = Arc::clone(&slow_output.inner);
        let live_worker = DefaultSubmitWorker::new(
            backend.clone(),
            patient_fetcher("{\"resourceType\":\"Patient\",\"id\":\"receipt-lease-loss\"}\n"),
            live_output.clone(),
            WorkerId::new("receipt-lease-loss-reclaimer"),
        );
        live_worker.run_job(replacement.clone()).await.unwrap();

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        assert_eq!(manifests[0].status, ManifestStatus::Completed);
        let rows = backend.list_submit_files(&tenant, &sub_id).await.unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(
            (
                row.file_type.as_str(),
                row.resource_type.as_deref(),
                row.part_index,
                row.fencing_token
            ),
            ("output", Some("Patient"), 0, replacement.fencing_token)
        );
        assert_eq!(row.line_count, 1);
        let expected_key = submit_artifact_key(
            &tenant,
            &sub_id,
            &manifest.manifest_id,
            "output",
            Some("Patient"),
            0,
            replacement.fencing_token,
        );
        assert_eq!(row.file_path, expected_key.resource_type);
        let key = ExportPartKey {
            tenant_id: tenant.tenant_id().as_str().to_string(),
            job_id: submission_output_job_id(&sub_id),
            resource_type: row.file_path.clone(),
            file_type: row.file_type.clone(),
            part_index: row.part_index,
            fencing_token: row.fencing_token,
        };
        let mut reader = live_output.open_reader(&key).await.unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(row.byte_count, bytes.len() as u64);
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            "{\"reference\":\"Patient/receipt-lease-loss\"}\n"
        );
    }

    #[tokio::test]
    async fn second_finalize_failure_hides_the_set_until_live_recovery() {
        use tokio::io::AsyncReadExt;

        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let tenant = tenant();
        let sub_id = SubmissionId::generate("finalize-failure");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(
                &tenant,
                &sub_id,
                Some("http://provider/finalize-failure.json"),
                None,
            )
            .await
            .unwrap();
        let old = backend
            .claim_next_manifest(
                &WorkerId::new("finalize-failure-old"),
                StdDuration::from_secs(60),
            )
            .await
            .unwrap()
            .unwrap();

        let mut files = std::collections::HashMap::new();
        let mut output_files = Vec::new();
        for resource_type in ["Condition", "Patient"] {
            let url = format!("http://provider/{resource_type}.ndjson");
            files.insert(
                url.clone(),
                format!("{{\"resourceType\":\"{resource_type}\",\"id\":\"one\"}}\n").into_bytes(),
            );
            output_files.push(RemoteFile {
                resource_type: Some(resource_type.to_string()),
                url,
                count: Some(1),
            });
        }
        let fetcher = Arc::new(MockFetcher {
            files,
            manifest: RemoteManifest {
                requires_access_token: false,
                output: output_files,
                deleted: vec![],
            },
        });
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().join("objects"),
            "http://localhost",
        ));
        let failing_output = Arc::new(FaultOutputStore::new(
            Arc::clone(&output),
            OutputFault::Finalize(1),
        ));
        let failing_worker = DefaultSubmitWorker::new(
            backend.clone(),
            fetcher.clone(),
            failing_output,
            WorkerId::new("finalize-failure-old"),
        );
        let failed_result = failing_worker.run_job(old.clone()).await;
        match failed_result {
            Err(StorageError::Backend(crate::error::BackendError::Internal {
                backend_name,
                message,
                source: None,
            })) => {
                assert_eq!(backend_name, "bulk-submit-worker-test");
                assert_eq!(message, "finalize attempt 1 forced failure");
            }
            other => panic!("expected the forced second-finalize error, got {other:?}"),
        }
        assert!(
            backend
                .list_submit_files(&tenant, &sub_id)
                .await
                .unwrap()
                .is_empty()
        );

        backend
            .get_connection()
            .unwrap()
            .execute(
                "UPDATE bulk_manifests SET lease_expiry = '1970-01-01T00:00:00Z'
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                   AND manifest_id = ?4",
                params![
                    tenant.tenant_id().as_str(),
                    sub_id.submitter,
                    sub_id.submission_id,
                    manifest.manifest_id,
                ],
            )
            .unwrap();
        let replacement = backend
            .claim_next_manifest(
                &WorkerId::new("finalize-failure-live"),
                StdDuration::from_secs(60),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(replacement.manifest_id, manifest.manifest_id);
        assert!(replacement.fencing_token > old.fencing_token);

        let live_worker = DefaultSubmitWorker::new(
            backend.clone(),
            fetcher,
            Arc::clone(&output),
            WorkerId::new("finalize-failure-live"),
        );
        live_worker.run_job(replacement.clone()).await.unwrap();

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        assert_eq!(manifests[0].status, ManifestStatus::Completed);
        let mut outputs = backend
            .list_submit_files(&tenant, &sub_id)
            .await
            .unwrap()
            .into_iter()
            .map(|row| {
                (
                    row.resource_type.clone().unwrap(),
                    row.part_index,
                    row.fencing_token,
                    row.file_path,
                    row.line_count,
                    row.byte_count,
                )
            })
            .collect::<Vec<_>>();
        outputs.sort_by_key(|row| (row.0.clone(), row.1));
        let expected_outputs = ["Condition", "Patient"]
            .into_iter()
            .enumerate()
            .map(|(part_index, resource_type)| {
                let key = submit_artifact_key(
                    &tenant,
                    &sub_id,
                    &manifest.manifest_id,
                    "output",
                    Some(resource_type),
                    part_index as u32,
                    replacement.fencing_token,
                );
                (
                    resource_type.to_string(),
                    part_index as u32,
                    replacement.fencing_token,
                    key.resource_type,
                    1,
                    format!("{{\"reference\":\"{resource_type}/one\"}}\n").len() as u64,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(outputs, expected_outputs);
        for (resource_type, part_index, fencing_token, file_path, line_count, byte_count) in outputs
        {
            let key = ExportPartKey {
                tenant_id: tenant.tenant_id().as_str().to_string(),
                job_id: submission_output_job_id(&sub_id),
                resource_type: file_path,
                file_type: "output".to_string(),
                part_index,
                fencing_token,
            };
            let mut reader = output.open_reader(&key).await.unwrap();
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await.unwrap();
            let expected_bytes = format!("{{\"reference\":\"{resource_type}/one\"}}\n");
            assert_eq!(line_count, 1);
            assert_eq!(byte_count, expected_bytes.len() as u64);
            assert_eq!(std::str::from_utf8(&bytes).unwrap(), expected_bytes);
            let references = std::str::from_utf8(&bytes)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                references,
                vec![json!({"reference": format!("{resource_type}/one")})]
            );
        }
    }

    #[tokio::test]
    async fn publication_takeover_is_quiet_then_original_lease_retry_reindexes() {
        use tokio::io::AsyncReadExt;

        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let tenant = tenant();
        let sub_id = SubmissionId::generate("publication-takeover");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(
                &tenant,
                &sub_id,
                Some("http://provider/publication-takeover.json"),
                None,
            )
            .await
            .unwrap();
        let lease = backend
            .claim_next_manifest(
                &WorkerId::new("publication-takeover"),
                StdDuration::from_secs(60),
            )
            .await
            .unwrap()
            .unwrap();

        let hook = Arc::new(MockReindexHook {
            calls: std::sync::Mutex::new(Vec::new()),
        });
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().join("objects"),
            "http://localhost",
        ));
        let guarded_worker = DefaultSubmitWorker::new(
            backend.clone(),
            patient_fetcher("{\"resourceType\":\"Patient\",\"id\":\"guarded\"}\n"),
            Arc::clone(&output),
            WorkerId::new("publication-takeover"),
        )
        .with_deferred_indexing(true, Some(hook.clone()));

        backend
            .get_connection()
            .unwrap()
            .execute(
                "CREATE TRIGGER lose_lease_after_output
                 AFTER INSERT ON bulk_submit_files
                 BEGIN
                   UPDATE bulk_manifests
                   SET worker_id = 'interloper', fencing_token = 999
                   WHERE tenant_id = NEW.tenant_id
                     AND submitter = NEW.submitter
                     AND submission_id = NEW.submission_id
                     AND manifest_id = NEW.manifest_id;
                 END",
                [],
            )
            .unwrap();
        guarded_worker.run_job(lease.clone()).await.unwrap();
        assert_eq!(
            backend.list_manifests(&tenant, &sub_id).await.unwrap()[0].status,
            ManifestStatus::Processing
        );
        assert!(
            backend
                .list_submit_files(&tenant, &sub_id)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            hook.calls.lock().unwrap().is_empty(),
            "lease lost publication must not fire reindex"
        );

        backend
            .get_connection()
            .unwrap()
            .execute("DROP TRIGGER lose_lease_after_output", [])
            .unwrap();
        let retry_worker = DefaultSubmitWorker::new(
            backend.clone(),
            patient_fetcher("{\"resourceType\":\"Patient\",\"id\":\"guarded\"}\n"),
            Arc::clone(&output),
            WorkerId::new("publication-takeover"),
        )
        .with_deferred_indexing(true, Some(hook.clone()));
        retry_worker.run_job(lease.clone()).await.unwrap();

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        assert_eq!(manifests[0].status, ManifestStatus::Completed);
        assert_eq!(
            hook.calls.lock().unwrap().clone(),
            vec![vec!["Patient".to_string()]]
        );
        let rows = backend.list_submit_files(&tenant, &sub_id).await.unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(
            (
                row.file_type.as_str(),
                row.resource_type.as_deref(),
                row.part_index,
                row.fencing_token
            ),
            ("output", Some("Patient"), 0, lease.fencing_token)
        );
        let expected_key = submit_artifact_key(
            &tenant,
            &sub_id,
            &manifest.manifest_id,
            "output",
            Some("Patient"),
            0,
            lease.fencing_token,
        );
        assert_eq!(row.file_path, expected_key.resource_type);
        let key = ExportPartKey {
            tenant_id: tenant.tenant_id().as_str().to_string(),
            job_id: submission_output_job_id(&sub_id),
            resource_type: row.file_path.clone(),
            file_type: "output".to_string(),
            part_index: row.part_index,
            fencing_token: row.fencing_token,
        };
        let mut reader = output.open_reader(&key).await.unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(row.line_count, 1);
        assert_eq!(row.byte_count, bytes.len() as u64);
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            "{\"reference\":\"Patient/guarded\"}\n"
        );
    }

    /// How a stubbed heartbeat behaves, for the [`LeaseKeeper`] tests.
    enum Renewal {
        /// Never answers — a heartbeat starved behind the ingest loop's writer,
        /// which is what let the lease expire in #969.
        Hangs,
        /// Answers, but always with a storage error.
        Fails,
        /// Renews normally.
        Lands,
    }

    struct StubRenewal {
        renewal: Renewal,
        /// What the submission behind the lease reads back as (#968).
        status: SubmissionStatus,
    }

    #[async_trait]
    impl LeaseRenewal for StubRenewal {
        async fn heartbeat(&self, lease: &ManifestLease) -> Result<DateTime<Utc>, LeaseError> {
            match self.renewal {
                Renewal::Hangs => std::future::pending().await,
                Renewal::Fails => Err(LeaseError::Storage(StorageError::Backend(
                    crate::error::BackendError::Internal {
                        backend_name: "stub".to_string(),
                        message: "writer busy".to_string(),
                        source: None,
                    },
                ))),
                Renewal::Lands => Ok(lease.renewed_expiry()),
            }
        }

        async fn flush_bytes(&self, _lease: &ManifestLease, _consumed: u64, _total: u64) {}

        async fn submission_status(
            &self,
            _lease: &ManifestLease,
        ) -> StorageResult<Option<SubmissionStatus>> {
            Ok(Some(self.status))
        }
    }

    /// A two-second lease, short enough that a keeper's timings play out inside
    /// a test.
    fn keeper_lease() -> ManifestLease {
        ManifestLease {
            tenant: tenant(),
            submission_id: SubmissionId::generate("mock-system"),
            manifest_id: "m1".to_string(),
            worker_id: WorkerId::new("w"),
            lease_expiry: Utc::now() + chrono::Duration::seconds(2),
            lease_duration: StdDuration::from_secs(2),
            fencing_token: 1,
        }
    }

    /// Spawns a keeper over a two-second lease and reports whether it declared
    /// that lease lost within `wait`.
    async fn keeper_loses_lease(renewal: Renewal, wait: StdDuration) -> bool {
        let keeper = LeaseKeeper::spawn(
            Arc::new(StubRenewal {
                renewal,
                status: SubmissionStatus::InProgress,
            }),
            keeper_lease(),
            ByteProgress::default(),
            CancelToken::new(),
        );
        tokio::time::timeout(wait, keeper.lost()).await.is_ok()
    }

    /// Spawns a keeper over a healthy lease whose submission reads back as
    /// `status`, and reports whether it tripped the ingest's cancel token
    /// within `wait`.
    async fn keeper_cancels(status: SubmissionStatus, wait: StdDuration) -> bool {
        let cancel = CancelToken::new();
        let _keeper = LeaseKeeper::spawn(
            Arc::new(StubRenewal {
                renewal: Renewal::Lands,
                status,
            }),
            keeper_lease(),
            ByteProgress::default(),
            cancel.clone(),
        );
        let deadline = tokio::time::Instant::now() + wait;
        while tokio::time::Instant::now() < deadline && !cancel.is_cancelled() {
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
        cancel.is_cancelled()
    }

    /// A heartbeat that cannot land before the lease expires is fatal (#969).
    ///
    /// Retrying it indefinitely left the manifest claimable by a second worker
    /// while this one kept committing batches under a dead lease.
    #[tokio::test]
    async fn test_lease_keeper_gives_up_on_a_starved_heartbeat() {
        assert!(keeper_loses_lease(Renewal::Hangs, StdDuration::from_secs(15)).await);
    }

    #[tokio::test]
    async fn test_lease_keeper_gives_up_when_heartbeats_keep_failing() {
        assert!(keeper_loses_lease(Renewal::Fails, StdDuration::from_secs(15)).await);
    }

    /// The converse: a lease that is being renewed is never declared lost, so
    /// the new expiry check cannot abort a healthy run.
    #[tokio::test]
    async fn test_lease_keeper_holds_a_renewable_lease() {
        assert!(!keeper_loses_lease(Renewal::Lands, StdDuration::from_secs(5)).await);
    }

    /// An aborted submission trips the ingest's cancel token (#968).
    ///
    /// This is the whole mechanism behind Abort reaching a manifest that is
    /// already in flight: nothing else in a running job re-reads the
    /// submission, so without this the ingest runs to the end of the file and
    /// the UI reports "Stopped" over a job still writing rows.
    #[tokio::test]
    async fn test_lease_keeper_cancels_an_aborted_submission() {
        assert!(keeper_cancels(SubmissionStatus::Aborted, StdDuration::from_secs(15)).await);
    }

    /// The converse, and the reason `complete` is not treated as terminal here:
    /// it means the submitter will send no further manifests, not that the
    /// registered ones should be dropped mid-ingest.
    #[tokio::test]
    async fn test_lease_keeper_leaves_an_ingestable_submission_running() {
        assert!(!keeper_cancels(SubmissionStatus::InProgress, StdDuration::from_secs(5)).await);
        assert!(!keeper_cancels(SubmissionStatus::Complete, StdDuration::from_secs(5)).await);
    }
}
