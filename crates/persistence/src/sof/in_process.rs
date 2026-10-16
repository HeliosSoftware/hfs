//! Generic in-process SQL-on-FHIR runner.
//!
//! Unlike the SQLite and PostgreSQL runners — which compile a ViewDefinition to
//! native SQL and execute it inside the database — this runner streams the
//! resources of the view's target type out of a backend and evaluates the
//! ViewDefinition with the in-process `helios-sof` FHIRPath engine. It exists
//! for backends that have no query engine to push the computation into (S3
//! object storage, and S3-primary composites such as `s3-elasticsearch`).
//!
//! The heavy lifting is delegated to [`helios_sof::PreparedViewDefinition`],
//! the same engine that serves inline `resource:` runs and the `sof-cli`/`sof-server`
//! tools, so this runner introduces no new SQL-on-FHIR semantics. Resource
//! access is abstracted behind [`ResourceScan`] so the runner is
//! backend-agnostic.
//!
//! A run has two tasks. An async scan task reads the [`ResourceScan`] (with
//! `_since` applied by the scan, see below), batches the resources and
//! pre-resolves each batch's references;
//! a `spawn_blocking` engine task evaluates the batches and sends the rows
//! through the producer core's batch channel
//! ([`row_batch_channel`](crate::core::sof_producer::row_batch_channel)).
//!
//! - **End marker.** The scan ends its batches with an explicit end marker.
//!   Its `JoinHandle` is supervised, so a scan that panics or fails reaches
//!   the consumer as an `Err` item; a scan that ends without the marker is a
//!   failure unless the run was stopped. The engine is supervised the same
//!   way, so a partial row set never looks complete.
//! - **Stopping.** The run stops on the consumer dropping the stream, on
//!   [`RunContext::cancel`] and at [`RunContext::deadline`]. The setup reads
//!   (the compartment filter's documents, the start of the scan), each scan
//!   step and each batch's reference reads are raced against the stop; the
//!   engine checks it per chunk, and a stop sets a per-run flag that
//!   [`PreparedViewDefinition::process_chunk_with_external_until`] reads before
//!   each resource, so every Rayon worker stops after the resource it is
//!   evaluating. The stream ends (with `Cancelled` or `Timeout` for a stop that
//!   is not a drop) only after the engine's evaluation has returned and the
//!   scan task has been joined, so no storage read of the run (an S3 GET the
//!   scan's stream would still start, say) follows the end the consumer sees.
//!   A read that runs on its own task ([`spawn_run_read`], the S3 GETs under
//!   the backend-wide read limit) is tracked by the run: the setup reads and
//!   the scan task wait until every such task is gone, aborted ones included,
//!   before the run reports its end, so none is still in flight or holds a
//!   permit once the consumer sees `Cancelled`, `Timeout` or a failure.
//! - **Bound.** The scan stops within the stop bound; the evaluation stops
//!   within one resource evaluation per Rayon worker. A FHIRPath terminology
//!   call in flight inside that evaluation is bounded by its own timeout
//!   (`FHIRPATH_TERMINOLOGY_TIMEOUT`), not by the stop.
//! - **`_since`.** A run with a cutoff reads
//!   [`ResourceScan::scan_resources_since`], so the comparison uses the instant
//!   the storage records: the default compares the JSON `meta.lastUpdated`
//!   (exact where storage keeps the same precision, as the MongoDB fallback
//!   does), and S3 overrides it with its stored nanosecond instant (#1864).
//!   `_since` is inclusive: a resource stored at the cutoff is kept.
//! - **References.** A `patient`/`group` reference whose target does not exist
//!   (or was deleted) is [`SofError::NotFound`] listing it, for callers that
//!   reach the runner without the REST reference check.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use helios_fhir::FhirVersion;
use helios_sof::{
    CompartmentFilter, PreparedViewDefinition, ResourceChunk, compartment_reference_ids,
    parse_view_definition_for_version,
};
use serde_json::{Map, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{debug, warn};

use crate::core::sof_producer::{
    BatchSender, RunStop, StopFlag, bridge_stop, row_batch_channel, supervise,
};
use crate::core::sof_runner::{
    RunCapabilities, RunContext, RunOutput, RunRequest, SofError, SofRunner, StopReason, ViewRow,
};
use crate::core::sof_schema::ViewSchema;
use crate::sof::reference_resolver::{StorageReferenceResolver, collect_missing_references};
use crate::tenant::TenantContext;

/// Number of resources handed to the engine per [`PreparedViewDefinition::process_chunk`]
/// call. Bounds peak memory when scanning large resource types.
const CHUNK_SIZE: usize = 1024;

/// Most resources [`JobScanCache`] keeps per resource type. A type larger than
/// this is streamed on every scan, so a job's memory stays bounded.
const JOB_SCAN_CACHE_MAX_RESOURCES: usize = 20_000;

/// Distinct references a storage-backed resolver may read for one scanned
/// batch of [`CHUNK_SIZE`] resources (#1858). A batch of Observations already
/// carries ~1,850 (subject and encounter), so the resolver's general
/// [`DEFAULT_MAX_FANOUT`](crate::sof::reference_resolver::StorageBackedResolver::DEFAULT_MAX_FANOUT)
/// of 1,000 silently left almost half of every batch unresolved.
pub const BATCH_REFERENCE_FANOUT: usize = 8 * CHUNK_SIZE;

/// Whether a ViewDefinition calls FHIRPath `resolve()` anywhere — a column,
/// a `forEach`, a `where` or a constant. Only such a view needs the scan to
/// pre-resolve the references in each batch; for every other view that is
/// one storage read per reference for nothing (#1858).
fn view_uses_resolve(view: &Value) -> bool {
    match view {
        Value::String(s) => s.contains("resolve("),
        Value::Array(items) => items.iter().any(view_uses_resolve),
        Value::Object(map) => map.values().any(view_uses_resolve),
        _ => false,
    }
}

/// Number of resource batches that can be queued between the async scan task
/// and the spawn_blocking engine task before the scan task applies backpressure.
///
/// The engine holds one batch back beyond this to compute `is_last`, so peak
/// residency is `(RESOURCE_CHANNEL_BUFFER + 1) * CHUNK_SIZE` resources plus the
/// batch the scan task is still filling. Together with [`CHUNK_SIZE`] this pair
/// sets the floor on the run's memory: measured on a 100k-Observation corpus,
/// 3/1024 peaks around 350 MiB and 1/256 around 175 MiB, for ~5% throughput.
const RESOURCE_CHANNEL_BUFFER: usize = 3;

/// A pinned, heap-allocated, `Send + 'static` stream of raw FHIR resource JSON.
///
/// Returned by [`ResourceScan::scan_resources`]; consumed by [`InProcessSofRunner`].
pub type ResourceStream = BoxStream<'static, Result<Value, SofError>>;

tokio::task_local! {
    /// The storage-read tasks of the in-process run whose setup reads or scan
    /// task is running on this task; see [`spawn_run_read`].
    static RUN_READS: TaskTracker;
}

/// Spawns `read`, a storage read that has to run on its own task (the S3 GET
/// under the backend-wide read limit, #2063: it must finish and return its
/// permit even while its caller is not polled).
///
/// Called from an in-process run's setup reads or scan task, the read is
/// tracked by that run (#1864): when the caller's future is dropped its owner
/// aborts the task, and the run waits until every tracked task is gone before
/// it reports its end. So a stopped run has no such read in flight, and holds
/// no permit, once its terminal result is seen. Anywhere else this is
/// [`tokio::spawn`].
#[cfg_attr(not(feature = "s3"), allow(dead_code))]
pub(crate) fn spawn_run_read<F>(read: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    match RUN_READS.try_with(TaskTracker::clone) {
        Ok(reads) => reads.spawn(read),
        Err(_) => tokio::spawn(read),
    }
}

/// Waits until every read task `reads` tracks is gone: finished, or aborted
/// and dropped (tokio drops an aborted task's future, and with it its permit,
/// before the tracker counts it gone). The tracker stays usable afterwards.
async fn run_reads_gone(reads: &TaskTracker) {
    reads.close();
    reads.wait().await;
    reads.reopen();
}

/// Streams the live resources of a single resource type for a tenant.
///
/// Implemented by backends that lack an in-DB SOF runner so they can reuse
/// [`InProcessSofRunner`]. Implementations must yield the raw FHIR resource
/// JSON (the `content()` of each stored resource, with server-populated
/// `meta.versionId`/`meta.lastUpdated`), excluding soft-deleted ones.
///
/// Resources are yielded one at a time; the runner accumulates them into
/// [`CHUNK_SIZE`]-sized batches internally.
#[async_trait]
pub trait ResourceScan: Send + Sync {
    /// Yields every live resource of `resource_type` visible to `tenant` as
    /// raw FHIR JSON, one at a time.
    async fn scan_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> Result<ResourceStream, SofError>;

    /// Reads the named resources of `resource_type` by id, as raw FHIR JSON
    /// in the same shape [`scan_resources`](Self::scan_resources) yields.
    ///
    /// Ids that name no live resource (absent or soft-deleted) are omitted
    /// rather than erroring, so the result holds at most `ids.len()` items in
    /// unspecified order. Implementations MUST fetch by id — the point of this
    /// method is to let the runner build a compartment filter without scanning
    /// the Patient/Group collections (#1453), so a scan-and-search fallback
    /// would defeat it.
    async fn read_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[String],
    ) -> Result<Vec<Value>, SofError>;

    /// Like [`scan_resources`](Self::scan_resources), keeping only the
    /// resources stored at or after `cutoff` (`_since`, inclusive).
    ///
    /// The default compares each resource's JSON `meta.lastUpdated` and drops
    /// one without a parseable stamp. That is exact when storage records the
    /// instant at the JSON stamp's precision (the MongoDB fallback: both in
    /// milliseconds). A storage that records a finer instant than the JSON
    /// stamp (S3: nanoseconds) overrides this to compare its own instant, so a
    /// cutoff equal to the stored instant keeps the resource (#1864). An
    /// override's stream is used as is: the runner does not filter it again.
    async fn scan_resources_since(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<ResourceStream, SofError> {
        let stream = self.scan_resources(tenant, resource_type).await?;
        Ok(stream
            .filter(move |item| {
                std::future::ready(match item {
                    Ok(resource) => last_updated_at_or_after(resource, cutoff),
                    // Errors pass through to the run.
                    Err(_) => true,
                })
            })
            .boxed())
    }
}

/// Whether `resource`'s JSON `meta.lastUpdated` is at or after `cutoff`. A
/// resource with no parseable stamp is not (it is dropped by `_since`).
fn last_updated_at_or_after(resource: &Value, cutoff: chrono::DateTime<chrono::Utc>) -> bool {
    resource
        .get("meta")
        .and_then(|m| m.get("lastUpdated"))
        .and_then(|lu| lu.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&chrono::Utc) >= cutoff)
        .unwrap_or(false)
}

/// In-process SQL-on-FHIR runner backed by an arbitrary [`ResourceScan`].
pub struct InProcessSofRunner {
    scan: Arc<dyn ResourceScan>,
    fhir_version: FhirVersion,
    runner_name: &'static str,
    /// Optional storage-backed `resolve()` prefetch. When set, relative
    /// references in each scanned batch are dereferenced from storage
    /// (tenant-scoped) and folded into the FHIRPath resolution pool for that
    /// batch. When `None`, `resolve()` falls back to in-scope resolution only.
    resolver: Option<Arc<dyn StorageReferenceResolver>>,
}

impl InProcessSofRunner {
    /// Creates a runner that scans resources via `scan` and evaluates views
    /// against `fhir_version`. `runner_name` is surfaced in logs/diagnostics
    /// (e.g. `"s3-in-process"`). No storage-backed `resolve()` is configured;
    /// use [`with_reference_resolver`](Self::with_reference_resolver) to enable it.
    pub fn new(
        scan: Arc<dyn ResourceScan>,
        fhir_version: FhirVersion,
        runner_name: &'static str,
    ) -> Self {
        Self {
            scan,
            fhir_version,
            runner_name,
            resolver: None,
        }
    }

    /// Enables storage-backed `resolve()`: relative `Type/id` references in the
    /// resources under evaluation are dereferenced from storage via `resolver`
    /// (tenant-scoped, version-matched) and made available to FHIRPath
    /// `resolve()`. See [`crate::sof::reference_resolver`].
    pub fn with_reference_resolver(mut self, resolver: Arc<dyn StorageReferenceResolver>) -> Self {
        self.resolver = Some(resolver);
        self
    }
}

/// A [`ResourceScan`] that remembers each type's resources for the life of one
/// export job (see [`SofRunner::for_export_job`]).
///
/// The first scan of a type streams through to the inner scan and keeps what
/// it saw; only a scan that read the type to its end and stayed under
/// [`JOB_SCAN_CACHE_MAX_RESOURCES`] is kept, so one that was dropped part way
/// (cancelled, or capped by a row limit) or failed leaves nothing behind. Later
/// scans of the type replay the kept resources instead of reading storage.
///
/// A `_since` scan is forwarded to the inner
/// [`scan_resources_since`](ResourceScan::scan_resources_since), so the export
/// path reaches a storage's own stored-instant comparison (S3), and is kept
/// under its cutoff: the key is `(tenant, type, cutoff)`, so a scan with one
/// cutoff (or none) never replays the resources of another (#1864).
struct JobScanCache {
    inner: Arc<dyn ResourceScan>,
    kept: KeptScans,
}

/// The key of one kept scan: tenant id, resource type and `_since` cutoff.
type ScanKey = (String, String, Option<chrono::DateTime<chrono::Utc>>);

/// Resources kept per [`ScanKey`].
type KeptScans = Arc<parking_lot::Mutex<std::collections::HashMap<ScanKey, Arc<Vec<Value>>>>>;

#[async_trait]
impl ResourceScan for JobScanCache {
    async fn scan_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> Result<ResourceStream, SofError> {
        self.scan_kept(tenant, resource_type, None).await
    }

    async fn scan_resources_since(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<ResourceStream, SofError> {
        self.scan_kept(tenant, resource_type, Some(cutoff)).await
    }

    async fn read_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[String],
    ) -> Result<Vec<Value>, SofError> {
        self.inner.read_resources(tenant, resource_type, ids).await
    }
}

impl JobScanCache {
    /// Replays the kept scan of `(tenant, type, cutoff)`, or streams the inner
    /// scan (with the cutoff, when there is one) and keeps what it read.
    async fn scan_kept(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cutoff: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<ResourceStream, SofError> {
        let key: ScanKey = (
            tenant.tenant_id().as_str().to_string(),
            resource_type.to_string(),
            cutoff,
        );
        let hit = self.kept.lock().get(&key).cloned();
        if let Some(resources) = hit {
            return Ok(
                futures::stream::unfold((resources, 0usize), |(all, at)| async move {
                    let next = all.get(at).cloned()?;
                    Some((Ok(next), (all, at + 1)))
                })
                .boxed(),
            );
        }

        let inner = match cutoff {
            Some(cutoff) => {
                self.inner
                    .scan_resources_since(tenant, resource_type, cutoff)
                    .await?
            }
            None => self.inner.scan_resources(tenant, resource_type).await?,
        };
        let kept = Arc::clone(&self.kept);
        Ok(futures::stream::unfold(
            (inner, Some(Vec::<Value>::new()), kept, key),
            |(mut inner, mut buffer, kept, key)| async move {
                match inner.next().await {
                    Some(Ok(resource)) => {
                        if let Some(held) = buffer.as_mut() {
                            if held.len() < JOB_SCAN_CACHE_MAX_RESOURCES {
                                held.push(resource.clone());
                            } else {
                                buffer = None;
                            }
                        }
                        Some((Ok(resource), (inner, buffer, kept, key)))
                    }
                    Some(Err(e)) => Some((Err(e), (inner, None, kept, key))),
                    None => {
                        if let Some(held) = buffer {
                            kept.lock().insert(key, Arc::new(held));
                        }
                        None
                    }
                }
            },
        )
        .boxed())
    }
}

/// The suffix `helios_sof`'s absent-target error appends to its reference
/// list (`CompartmentFilter::build`).
const ABSENT_TARGET_SUFFIX: &str = " not found in supplied resources";

/// Maps a `helios_sof` engine error onto the persistence-layer [`SofError`].
///
/// Structural/validation problems become `InvalidViewDefinition` (422); the
/// spec's absent-target case becomes the typed [`SofError::NotFound`] (400
/// `not-found`; the runner normally reports it itself, before the engine sees
/// the filter, see [`missing_compartment_targets`]); everything else is a
/// runtime backend error.
fn map_engine_error(e: helios_sof::SofError) -> SofError {
    use helios_sof::SofError as E;
    match e {
        E::InvalidViewDefinition(m) => SofError::InvalidViewDefinition(m),
        E::ReferencedResourceNotFound(m) => SofError::NotFound {
            references: m
                .strip_suffix(ABSENT_TARGET_SUFFIX)
                .map(|list| list.split(", ").map(str::to_string).collect())
                .unwrap_or_else(|| vec![m]),
        },
        // A path the view's author wrote that FHIRPath rejects is a problem with
        // the ViewDefinition, not a backend failure (#640, main).
        E::FhirPathError(m) => SofError::InvalidViewDefinition(m),
        other => SofError::backend(other.to_string()),
    }
}

/// The `patient`/`group` references, as the caller wrote them, whose target is
/// not among `supporting` (the documents read by id; a deleted one is never
/// read). Mirrors the lookup of `CompartmentFilter::build`: a reference names
/// `{Type}/{id}` with or without its type prefix, and the id ends at the next
/// `/`.
fn missing_compartment_targets(
    patient: &[String],
    group: &[String],
    supporting: &[Value],
) -> Vec<String> {
    let mut missing = Vec::new();
    for (refs, resource_type) in [(patient, "Patient"), (group, "Group")] {
        for reference in refs {
            let named = reference
                .strip_prefix(resource_type)
                .and_then(|rest| rest.strip_prefix('/'))
                .unwrap_or(reference);
            let id = named.split('/').next().unwrap_or_default();
            let found = !id.is_empty()
                && supporting.iter().any(|resource| {
                    resource.get("resourceType").and_then(Value::as_str) == Some(resource_type)
                        && resource.get("id").and_then(Value::as_str) == Some(id)
                });
            if !found {
                missing.push(reference.clone());
            }
        }
    }
    missing
}

/// Zips one engine row (`columns` × `values`) into a flat JSON object, the
/// same shape the SQL runners emit. Missing/`None` values become JSON `null`.
fn row_to_view_row(columns: &[String], values: &[Option<Value>]) -> ViewRow {
    let mut obj = Map::with_capacity(columns.len());
    for (i, column) in columns.iter().enumerate() {
        let value = values.get(i).and_then(|v| v.as_ref()).cloned();
        obj.insert(column.clone(), value.unwrap_or(Value::Null));
    }
    Value::Object(obj)
}

/// Pre-resolves references found in `resources` against `resolver` (if any).
///
/// Called from the async scan task once per batch, before sending to the
/// blocking engine task. When `resolver` is `None` this is a no-op.
async fn resolve_batch_external(
    resolver: &Option<Arc<dyn StorageReferenceResolver>>,
    tenant: &TenantContext,
    fhir_version: FhirVersion,
    resources: &[Value],
) -> Result<Vec<Value>, SofError> {
    let Some(r) = resolver else {
        return Ok(Vec::new());
    };
    let refs = collect_missing_references(resources);
    if refs.is_empty() {
        return Ok(Vec::new());
    }
    r.resolve(tenant, fhir_version, &refs).await
}

#[async_trait]
impl SofRunner for InProcessSofRunner {
    fn runner_name(&self) -> &'static str {
        self.runner_name
    }

    fn for_export_job(&self) -> Option<Arc<dyn SofRunner>> {
        Some(Arc::new(InProcessSofRunner {
            scan: Arc::new(JobScanCache {
                inner: Arc::clone(&self.scan),
                kept: Arc::default(),
            }),
            fhir_version: self.fhir_version,
            runner_name: self.runner_name,
            resolver: self.resolver.clone(),
        }))
    }

    async fn run_view(
        &self,
        tenant: &TenantContext,
        request: RunRequest,
        ctx: RunContext,
    ) -> Result<RunOutput, SofError> {
        let RunRequest {
            view: view_definition,
            filters,
        } = request;
        let schema = ViewSchema::from_view_definition(&view_definition);
        let uses_resolve = view_uses_resolve(&view_definition);
        let view = parse_view_definition_for_version(view_definition, self.fhir_version)
            .map_err(map_engine_error)?;
        let prepared = PreparedViewDefinition::new(view).map_err(map_engine_error)?;
        // The evaluator already emits document order, so this is the identity
        // (the walker mirrors the evaluator); checked once per run all the same.
        let permutation = schema.permutation_from(prepared.columns())?;
        let resource_type = prepared.target_resource_type().to_string();

        debug!(
            runner = self.runner_name,
            tenant = %tenant.tenant_id(),
            resource_type = %resource_type,
            "executing in-process ViewDefinition"
        );

        // The run's channel and stop signal exist before the setup reads, so
        // those reads are raced against the cancel and the deadline, and the
        // budget a timeout reports counts from here.
        let (out, stop, rows) = row_batch_channel(self.runner_name, &ctx);
        // The read tasks the setup reads and the scan spawn (`spawn_run_read`).
        let reads = TaskTracker::new();

        // Fetch the Patient/Group documents the compartment filter needs, by id.
        // `build` only ever looks at the resources its own references name —
        // absent-target validation is a point lookup per reference, and a
        // Group's members are read off that Group — so setup is proportional to
        // the number of filter references, not to the corpus. Scanning the two
        // types instead would be unbounded for a Patient-target view and would
        // read the target type twice (#1453).
        let compartment_filter: Option<CompartmentFilter> =
            if !filters.patient.is_empty() || !filters.group.is_empty() {
                let mut supporting: Vec<Value> = Vec::new();
                for (refs, supporting_type) in
                    [(&filters.patient, "Patient"), (&filters.group, "Group")]
                {
                    let ids = compartment_reference_ids(refs, supporting_type);
                    if !ids.is_empty() {
                        let read = self.scan.read_resources(tenant, supporting_type, &ids);
                        supporting.extend(race_setup(&stop, &reads, read).await?);
                    }
                }
                // A target that does not exist, or was deleted, is the typed
                // not-found (400 `not-found` on `$sql-run`), never a 422.
                let missing =
                    missing_compartment_targets(&filters.patient, &filters.group, &supporting);
                if !missing.is_empty() {
                    return Err(SofError::NotFound {
                        references: missing,
                    });
                }
                Some(
                    CompartmentFilter::build(
                        &filters.patient,
                        &filters.group,
                        &supporting,
                        self.fhir_version,
                    )
                    .map_err(map_engine_error)?,
                )
            } else {
                None
            };

        // `_since` is applied by the scan, against the instant the storage
        // records (see `ResourceScan::scan_resources_since`).
        let scan_stream = match filters.since {
            Some(cutoff) => {
                let scan = self
                    .scan
                    .scan_resources_since(tenant, &resource_type, cutoff);
                race_setup(&stop, &reads, scan).await?
            }
            None => {
                let scan = self.scan.scan_resources(tenant, &resource_type);
                race_setup(&stop, &reads, scan).await?
            }
        };

        // Resource channel: batches of (resources, pre-resolved external refs)
        // from the async scan task to the blocking engine task, ended by
        // `ScanFrame::End`.
        let (res_tx, res_rx) = mpsc::channel::<ScanFrame>(RESOURCE_CHANNEL_BUFFER);
        let scan = ScanTask {
            stream: scan_stream,
            limit: filters.limit,
            resolver: if uses_resolve {
                self.resolver.clone()
            } else {
                None
            },
            tenant: tenant.clone(),
            version: self.fhir_version,
        };
        let scan_handle =
            tokio::spawn(RUN_READS.scope(reads.clone(), scan.run(res_tx.clone(), stop.clone())));
        let scan_stopped = CancellationToken::new();
        let scan_watcher = supervise_scan(
            self.runner_name,
            scan_handle,
            reads,
            res_tx,
            scan_stopped.clone(),
        );

        // Blocking engine task (CPU-bound FHIRPath evaluation), with the
        // bridge that turns a stop into the evaluation's stop flag.
        let reporter = out.reporter();
        let flag = StopFlag::default();
        let evaluation_stop = Arc::new(AtomicBool::new(false));
        let done = CancellationToken::new();
        let bridge = {
            let evaluation_stop = Arc::clone(&evaluation_stop);
            bridge_stop(stop.clone(), flag.clone(), done.clone(), move || {
                evaluation_stop.store(true, Ordering::Relaxed)
            })
        };
        let engine = Engine {
            prepared,
            compartment_filter,
            limit: filters.limit,
            version: self.fhir_version,
            runner: self.runner_name,
            stop,
            flag,
            evaluation_stop,
            scan_stopped,
            handle: tokio::runtime::Handle::current(),
        };
        let producer = tokio::task::spawn_blocking(move || {
            let _done = done.drop_guard();
            engine.produce(res_rx, out);
        });
        // The watcher is joined too, so an engine panic is reported only once
        // the scan (which stops when the engine's receiver is gone) has stopped.
        supervise(
            self.runner_name,
            reporter,
            vec![producer, bridge, scan_watcher],
        );

        // Rows follow the scan's order, which no backend makes total; nothing
        // here runs on a database server (a stop ends the storage reads and the
        // evaluation, both in this process).
        let capabilities = RunCapabilities {
            total_order: false,
            declared_schema: true,
            cancels_server_side: false,
        };
        Ok(RunOutput::new(
            schema,
            capabilities,
            permutation.normalize(rows),
        ))
    }
}

/// Runs a setup read of a run (the compartment filter's documents, the start
/// of the scan) unless the run is cancelled or its deadline passes first.
///
/// The read tasks the setup spawns are tracked in `reads`; whatever the
/// outcome, this returns only once they are gone. A stop (or a failed read
/// among several in flight) drops the setup with GETs still running, which
/// aborts them, and the run's error must not be reported before those aborted
/// reads, and the permits they hold, are released (#1864).
async fn race_setup<T>(
    stop: &RunStop,
    reads: &TaskTracker,
    setup: impl std::future::Future<Output = Result<T, SofError>>,
) -> Result<T, SofError> {
    let result = tokio::select! {
        biased;
        reason = stop.stopped() => Err(stop.error_for(reason).unwrap_or(SofError::Cancelled)),
        result = RUN_READS.scope(reads.clone(), setup) => result,
    };
    run_reads_gone(reads).await;
    result
}

/// What the scan task sends to the engine.
enum ScanFrame {
    /// A batch of resources and the external resources its references need.
    Batch(Vec<Value>, Vec<Value>),
    /// The scan read its stream to the end; nothing follows.
    End,
    /// The scan failed; nothing follows.
    Failed(SofError),
}

/// The async half of a run: reads the scan (already narrowed by `_since`),
/// batches, and pre-resolves each batch's references.
struct ScanTask {
    stream: ResourceStream,
    limit: Option<usize>,
    resolver: Option<Arc<dyn StorageReferenceResolver>>,
    tenant: TenantContext,
    version: FhirVersion,
}

impl ScanTask {
    /// Sends the scan's batches, then [`ScanFrame::End`]. Returns without the
    /// end marker when the run stops or the engine is gone: every await
    /// (the next resource, a batch's reference reads, a send) is raced
    /// against both.
    async fn run(self, res_tx: mpsc::Sender<ScanFrame>, stop: RunStop) {
        let ScanTask {
            mut stream,
            limit,
            resolver,
            tenant,
            version,
        } = self;
        /// Awaits `step` unless the run stops or the engine goes away first.
        async fn raced<T>(
            stop: &RunStop,
            res_tx: &mpsc::Sender<ScanFrame>,
            step: impl std::future::Future<Output = T>,
        ) -> Option<T> {
            tokio::select! {
                biased;
                _ = stop.stopped() => None,
                _ = res_tx.closed() => None,
                value = step => Some(value),
            }
        }
        async fn send(stop: &RunStop, res_tx: &mpsc::Sender<ScanFrame>, frame: ScanFrame) -> bool {
            raced(stop, res_tx, res_tx.send(frame))
                .await
                .is_some_and(|sent| sent.is_ok())
        }

        // A run with a row cap (a preview) starts with a batch no larger
        // than the cap, so a simple view yields its rows after reading
        // about `limit` resources rather than a full CHUNK_SIZE. Each
        // flushed batch doubles the target up to CHUNK_SIZE, so a view
        // whose `where` drops most inputs still reaches its cap.
        let mut batch_target = limit.map_or(CHUNK_SIZE, |cap| cap.clamp(1, CHUNK_SIZE));
        let mut batch: Vec<Value> = Vec::with_capacity(batch_target);

        loop {
            let Some(item) = raced(&stop, &res_tx, stream.next()).await else {
                return;
            };
            let Some(item) = item else { break };
            let resource = match item {
                Ok(v) => v,
                Err(e) => {
                    send(&stop, &res_tx, ScanFrame::Failed(e)).await;
                    return;
                }
            };

            batch.push(resource);

            if batch.len() >= batch_target {
                let resolved = resolve_batch_external(&resolver, &tenant, version, &batch);
                let Some(resolved) = raced(&stop, &res_tx, resolved).await else {
                    return;
                };
                let frame = match resolved {
                    Ok(external) => ScanFrame::Batch(std::mem::take(&mut batch), external),
                    Err(e) => {
                        send(&stop, &res_tx, ScanFrame::Failed(e)).await;
                        return;
                    }
                };
                if !send(&stop, &res_tx, frame).await {
                    return;
                }
                batch_target = (batch_target * 2).min(CHUNK_SIZE);
            }
        }

        if !batch.is_empty() {
            let resolved = resolve_batch_external(&resolver, &tenant, version, &batch);
            let Some(resolved) = raced(&stop, &res_tx, resolved).await else {
                return;
            };
            let frame = match resolved {
                Ok(external) => ScanFrame::Batch(batch, external),
                Err(e) => {
                    send(&stop, &res_tx, ScanFrame::Failed(e)).await;
                    return;
                }
            };
            if !send(&stop, &res_tx, frame).await {
                return;
            }
        }
        send(&stop, &res_tx, ScanFrame::End).await;
    }
}

/// Watches the scan task: if it panics or is cancelled, the engine receives
/// a [`ScanFrame::Failed`] (through `res_tx`, a sender kept outside the task)
/// in place of the end marker, so the rows read before the failure cannot
/// end the run as if they were complete.
///
/// Fires `stopped` once the scan task has been joined and every read task it
/// spawned (tracked in `reads`) is gone, however it ended (and if the watcher
/// itself is dropped). Tokio drops a task's future before its `JoinHandle`
/// resolves, so by then the scan's stream, and every storage read it polled
/// inline, is gone; dropping the stream aborts the read tasks it spawned
/// ([`spawn_run_read`]), and waiting on `reads` covers the time until tokio has
/// dropped them too. Returns the watcher's handle.
fn supervise_scan(
    runner: &'static str,
    scan: tokio::task::JoinHandle<()>,
    reads: TaskTracker,
    res_tx: mpsc::Sender<ScanFrame>,
    stopped: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let _stopped = stopped.drop_guard();
        let joined = scan.await;
        run_reads_gone(&reads).await;
        if let Err(e) = joined {
            warn!(runner, error = %e, "SoF resource scan ended before finishing");
            let _ = res_tx
                .send(ScanFrame::Failed(SofError::backend(format!(
                    "resource scan failed: {e}"
                ))))
                .await;
        }
    })
}

/// How a run ended before all its rows were sent.
enum RunEnd {
    Stopped(StopReason),
    Failed(SofError),
}

/// The blocking half of a run: applies the compartment filter and evaluates
/// each batch, stoppable per resource.
struct Engine {
    prepared: PreparedViewDefinition,
    compartment_filter: Option<CompartmentFilter>,
    limit: Option<usize>,
    version: FhirVersion,
    runner: &'static str,
    stop: RunStop,
    /// The stop reason, recorded by the bridge before it sets
    /// `evaluation_stop`.
    flag: StopFlag,
    /// Read by the evaluation before each resource.
    evaluation_stop: Arc<AtomicBool>,
    /// Fired by [`supervise_scan`] once the scan task has been joined and its
    /// read tasks are gone.
    scan_stopped: CancellationToken,
    handle: tokio::runtime::Handle,
}

impl Engine {
    /// Evaluates the scan's batches into `out`, then ends the stream. The
    /// stream ends here, after the evaluation (and its Rayon work) has
    /// returned and the scan task has been joined, so the consumer sees the
    /// end (a stop, a failure or the last row) only once nothing evaluates or
    /// reads storage for this run any more.
    fn produce(self, mut res_rx: mpsc::Receiver<ScanFrame>, mut out: BatchSender) {
        let result = self.evaluate(&mut res_rx, &mut out);
        // Dropping the receiver stops the scan: every await of the scan task
        // is raced against `res_tx.closed()` (and against the stop). Then wait
        // until it has stopped; a scan still polling its stream could start
        // another storage read after the end.
        drop(res_rx);
        let Engine {
            handle,
            stop,
            runner,
            scan_stopped,
            ..
        } = self;
        handle.block_on(scan_stopped.cancelled());
        match result {
            Ok(rows) => {
                debug!(runner, rows, "in-process view run complete");
                out.blocking_finish(&handle, &stop);
            }
            Err(RunEnd::Stopped(reason)) => {
                debug!(runner, ?reason, "in-process view run stopped");
                out.blocking_end_after_stop(&handle, &stop, reason);
            }
            Err(RunEnd::Failed(error)) => out.blocking_fail(&handle, &stop, error),
        }
    }

    /// The reason the run must stop now, if any.
    fn stopped(&self) -> Option<StopReason> {
        self.flag.get().or_else(|| self.stop.check())
    }

    /// The next frame of the scan, unless the run stops first. A scan that
    /// ended without its end marker is the stop's doing when the run
    /// stopped, and a failure otherwise.
    fn next_frame(&self, res_rx: &mut mpsc::Receiver<ScanFrame>) -> Result<ScanFrame, RunEnd> {
        let stop = &self.stop;
        let received = self.handle.block_on(async {
            tokio::select! {
                biased;
                reason = stop.stopped() => Err(reason),
                frame = res_rx.recv() => Ok(frame),
            }
        });
        match received {
            Err(reason) => Err(RunEnd::Stopped(reason)),
            Ok(Some(frame)) => Ok(frame),
            Ok(None) => Err(match self.stopped() {
                Some(reason) => RunEnd::Stopped(reason),
                None => RunEnd::Failed(SofError::backend(format!(
                    "{}: resource scan ended before its end marker",
                    self.runner
                ))),
            }),
        }
    }

    /// The batch loop. Returns the number of rows sent.
    fn evaluate(
        &self,
        res_rx: &mut mpsc::Receiver<ScanFrame>,
        out: &mut BatchSender,
    ) -> Result<usize, RunEnd> {
        let columns = self.prepared.columns().to_vec();
        let mut emitted = 0usize;
        let mut chunk_index = 0usize;

        // One-batch lookahead: `is_last` means "no further batches exist",
        // which only the end marker behind the current batch can tell.
        let mut next = self.next_frame(res_rx)?;
        loop {
            let (resources, external_json) = match next {
                ScanFrame::Batch(resources, external) => (resources, external),
                ScanFrame::End => break,
                ScanFrame::Failed(e) => return Err(RunEnd::Failed(e)),
            };
            next = self.next_frame(res_rx)?;
            let is_last = matches!(next, ScanFrame::End);

            // Per-chunk check: a cancel, the deadline or a dropped stream.
            if let Some(reason) = self.stopped() {
                return Err(RunEnd::Stopped(reason));
            }

            let resources: Vec<Value> = match &self.compartment_filter {
                Some(cf) => {
                    let mut filtered = Vec::with_capacity(resources.len());
                    for r in resources {
                        if self.evaluation_stop.load(Ordering::Relaxed) {
                            return Err(RunEnd::Stopped(
                                self.stopped().unwrap_or(StopReason::Dropped),
                            ));
                        }
                        match cf.apply(&r) {
                            Ok(true) => filtered.push(r),
                            Ok(false) => {}
                            Err(e) => return Err(RunEnd::Failed(map_engine_error(e))),
                        }
                    }
                    filtered
                }
                None => resources,
            };

            // An all-filtered-out batch produces no rows, but the final
            // batch still has to reach the engine so `is_last` is actually
            // delivered once per run.
            if resources.is_empty() && !is_last {
                continue;
            }

            let external: Vec<_> = external_json
                .into_iter()
                .filter_map(|json| {
                    helios_sof::parse_json_to_fhir_resource_pub(json, self.version).ok()
                })
                .collect();

            let chunk = ResourceChunk {
                resources,
                chunk_index,
                is_last,
            };
            chunk_index += 1;

            let result = match self.prepared.process_chunk_with_external_until(
                chunk,
                external,
                &self.evaluation_stop,
            ) {
                Ok(Some(result)) => result,
                // The bridge records the reason before it sets the flag.
                Ok(None) => {
                    return Err(RunEnd::Stopped(
                        self.stopped().unwrap_or(StopReason::Dropped),
                    ));
                }
                Err(e) => return Err(RunEnd::Failed(map_engine_error(e))),
            };

            for row in &result.rows {
                if self.limit.is_some_and(|cap| emitted >= cap) {
                    return Ok(emitted);
                }
                emitted += 1;
                if out.push(Ok(row_to_view_row(&columns, &row.values))) {
                    out.blocking_flush(&self.handle, &self.stop)
                        .map_err(RunEnd::Stopped)?;
                }
            }
            // A chunk's rows reach the consumer before the next chunk is
            // evaluated.
            out.blocking_flush(&self.handle, &self.stop)
                .map_err(RunEnd::Stopped)?;
            if self.limit.is_some_and(|cap| emitted >= cap) {
                return Ok(emitted);
            }
        }
        Ok(emitted)
    }
}

/// The HL7/sql-on-fhir PR #410 example views shared with the in-DB runner
/// integration tests.
#[cfg(all(test, feature = "R4"))]
#[path = "../../tests/common/sof_pr410_views.rs"]
mod sof_pr410_views;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::sof_runner::{RunKind, ViewFilters};
    use crate::sof::reference_resolver::StorageReferenceResolver;
    use crate::tenant::{TenantId, TenantPermissions};
    use serde_json::json;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio_stream::StreamExt;

    /// A `ResourceScan` that streams a fixed set of resources, filtered by
    /// type, counting how often each access path is taken.
    #[derive(Default)]
    struct StaticScan {
        resources: Vec<Value>,
        /// The resource type of every `scan_resources` call, in call order.
        scans: Mutex<Vec<String>>,
        /// Number of `read_resources` calls.
        reads: AtomicUsize,
    }

    impl StaticScan {
        fn of(resources: Vec<Value>) -> Arc<Self> {
            Arc::new(Self {
                resources,
                ..Default::default()
            })
        }

        /// The resource types handed to `scan_resources`, in call order.
        fn scanned_types(&self) -> Vec<String> {
            self.scans.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ResourceScan for StaticScan {
        async fn scan_resources(
            &self,
            _tenant: &TenantContext,
            resource_type: &str,
        ) -> Result<ResourceStream, SofError> {
            self.scans.lock().unwrap().push(resource_type.to_string());
            let items: Vec<Value> = self
                .resources
                .iter()
                .filter(|r| r.get("resourceType").and_then(Value::as_str) == Some(resource_type))
                .cloned()
                .collect();
            Ok(Box::pin(futures::stream::iter(items.into_iter().map(Ok))))
        }

        async fn read_resources(
            &self,
            _tenant: &TenantContext,
            resource_type: &str,
            ids: &[String],
        ) -> Result<Vec<Value>, SofError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .resources
                .iter()
                .filter(|r| {
                    r.get("resourceType").and_then(Value::as_str) == Some(resource_type)
                        && r.get("id")
                            .and_then(Value::as_str)
                            .is_some_and(|id| ids.iter().any(|want| want == id))
                })
                .cloned()
                .collect())
        }
    }

    /// A resolver that serves a fixed pool of resources by `(type, id)`.
    struct StaticResolver {
        pool: Vec<Value>,
    }

    #[async_trait]
    impl StorageReferenceResolver for StaticResolver {
        async fn resolve(
            &self,
            _tenant: &TenantContext,
            _fhir_version: FhirVersion,
            refs: &[(String, String)],
        ) -> Result<Vec<Value>, SofError> {
            Ok(refs
                .iter()
                .filter_map(|(rt, id)| {
                    self.pool.iter().find(|r| {
                        r.get("resourceType").and_then(Value::as_str) == Some(rt.as_str())
                            && r.get("id").and_then(Value::as_str) == Some(id.as_str())
                    })
                })
                .cloned()
                .collect())
        }
    }

    /// A resolver whose every call fails, as one over its fan-out cap does.
    struct LimitResolver;

    #[async_trait]
    impl StorageReferenceResolver for LimitResolver {
        async fn resolve(
            &self,
            _tenant: &TenantContext,
            _fhir_version: FhirVersion,
            _refs: &[(String, String)],
        ) -> Result<Vec<Value>, SofError> {
            Err(SofError::ResolutionLimit("over the cap".to_string()))
        }
    }

    /// #1870: a reference the resolver cannot resolve fails the run instead of
    /// yielding rows with the resolved columns silently empty.
    #[tokio::test]
    async fn a_failed_resolution_fails_the_run() {
        let runner =
            InProcessSofRunner::new(StaticScan::of(vec![observation()]), FhirVersion::R4, "test")
                .with_reference_resolver(Arc::new(LimitResolver));
        let mut stream = runner
            .run_view(
                &tenant(),
                RunRequest::new(resolve_view(), ViewFilters::default()),
                RunContext::detached(RunKind::Preview),
            )
            .await
            .map(|output| output.rows)
            .expect("run_view");
        let mut rows = 0;
        let mut error = None;
        while let Some(item) = stream.next().await {
            match item {
                Ok(_) => rows += 1,
                Err(e) => error = Some(e),
            }
        }
        assert_eq!(rows, 0, "no row may be emitted for an unresolved batch");
        assert!(
            matches!(error, Some(SofError::ResolutionLimit(_))),
            "got {error:?}"
        );
    }

    /// #640: a leading identifier that is neither a type nor an element fails
    /// the run as an invalid ViewDefinition (4xx), not a backend error (500).
    #[tokio::test]
    async fn unresolvable_leading_identifier_is_an_invalid_view_definition() {
        let runner =
            InProcessSofRunner::new(StaticScan::of(vec![patient()]), FhirVersion::R4, "test");
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"name": "x", "path": "Bogus.name.family"}]}]
        });
        let mut stream = runner
            .run_view(
                &tenant(),
                RunRequest::new(view, ViewFilters::default()),
                RunContext::detached(RunKind::Preview),
            )
            .await
            .map(|output| output.rows)
            .expect("run_view");
        let mut error = None;
        while let Some(item) = stream.next().await {
            if let Err(e) = item {
                error = Some(e);
            }
        }
        assert!(
            matches!(error, Some(SofError::InvalidViewDefinition(ref m)) if m.contains("Bogus")),
            "got {error:?}"
        );
    }

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
    }

    /// The PR #410 example views (and the in-DB runners' corner cases) give
    /// the same rows through the in-process runner as through `sof-cli`.
    #[cfg(feature = "R4")]
    #[tokio::test]
    async fn pr410_example_views_match_the_reference_rows() {
        use super::sof_pr410_views as pr410;
        let resources = pr410::resources().into_iter().map(|(_, r)| r).collect();
        let runner = InProcessSofRunner::new(StaticScan::of(resources), FhirVersion::R4, "test");
        let sorted = |mut rows: Vec<Value>| {
            rows.sort_by_key(|r| r.to_string());
            rows
        };
        for case in pr410::pr410_cases().into_iter().chain(pr410::extra_cases()) {
            if pr410::IN_DB_ONLY.contains(&case.name) {
                continue;
            }
            let mut stream = runner
                .run_view(
                    &tenant(),
                    RunRequest::new(case.view, ViewFilters::default()),
                    RunContext::detached(RunKind::Preview),
                )
                .await
                .map(|output| output.rows)
                .unwrap_or_else(|e| panic!("{}: {e}", case.name));
            let mut rows = Vec::new();
            while let Some(row) = stream.next().await {
                rows.push(row.unwrap_or_else(|e| panic!("{}: {e}", case.name)));
            }
            assert_eq!(sorted(rows), sorted(case.expected), "{}", case.name);
        }
    }

    /// A resolver that counts how many times the runner asks it for anything.
    struct CountingResolver {
        inner: StaticResolver,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl StorageReferenceResolver for CountingResolver {
        async fn resolve(
            &self,
            tenant: &TenantContext,
            fhir_version: FhirVersion,
            refs: &[(String, String)],
        ) -> Result<Vec<Value>, SofError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.resolve(tenant, fhir_version, refs).await
        }
    }

    /// #1858: a view that never calls `resolve()` does not pre-resolve the
    /// references in its batches; one that does still gets them.
    #[tokio::test]
    async fn references_are_pre_resolved_only_for_a_view_that_resolves() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let runner =
            InProcessSofRunner::new(StaticScan::of(vec![observation()]), FhirVersion::R4, "test")
                .with_reference_resolver(Arc::new(CountingResolver {
                    inner: StaticResolver {
                        pool: vec![patient()],
                    },
                    calls: Arc::clone(&calls),
                }));

        let ids = observation_ids(&runner, ViewFilters::default()).await;
        assert_eq!(ids, ["o1"]);
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a view without resolve() reads no referenced resources"
        );

        let rows = collect_rows(&runner).await;
        assert_eq!(rows[0]["patient_family"], "Smith");
        assert!(calls.load(std::sync::atomic::Ordering::SeqCst) > 0);
    }

    #[test]
    fn view_uses_resolve_finds_it_anywhere_in_the_view() {
        assert!(view_uses_resolve(&resolve_view()));
        assert!(!view_uses_resolve(&observation_ids_view()));
        assert!(view_uses_resolve(
            &json!({"select": [{"forEach": "subject.resolve()"}]})
        ));
        assert!(view_uses_resolve(
            &json!({"where": [{"path": "encounter.resolve().exists()"}]})
        ));
        assert!(!view_uses_resolve(&json!({"select": [{"column": [
            {"name": "patient_id", "path": "subject.getReferenceKey(Patient)"}
        ]}]})));
    }

    /// A view whose column dereferences `Observation.subject` to a Patient that
    /// is *not* in the scanned set — it can only come from the resolver.
    fn resolve_view() -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Observation",
            "status": "active",
            "select": [{"column": [
                {"name": "obs_id", "path": "id"},
                {"name": "patient_family",
                 "path": "subject.resolve().ofType(Patient).name.family.first()"}
            ]}]
        })
    }

    fn observation() -> Value {
        json!({
            "resourceType": "Observation",
            "id": "o1",
            "status": "final",
            "code": {"text": "test"},
            "subject": {"reference": "Patient/123"}
        })
    }

    fn patient() -> Value {
        json!({
            "resourceType": "Patient",
            "id": "123",
            "name": [{"family": "Smith", "given": ["Ann"]}]
        })
    }

    async fn collect_rows(runner: &InProcessSofRunner) -> Vec<Value> {
        let mut stream = runner
            .run_view(
                &tenant(),
                RunRequest::new(resolve_view(), ViewFilters::default()),
                RunContext::detached(RunKind::Preview),
            )
            .await
            .map(|output| output.rows)
            .expect("run_view");
        let mut rows = Vec::new();
        while let Some(row) = stream.next().await {
            rows.push(row.expect("row"));
        }
        rows
    }

    /// #1864 A2/A3, in-process cell (the SQL and MongoDB cells run the shared
    /// views of `tests/common/sof_prefix_matrix.rs` in the REST conformance
    /// tests): the declared schema is the walker's document order, which is
    /// the evaluator's column order, and every row carries exactly those keys
    /// in that order, empty cells as explicit nulls; a `select` after a
    /// `unionAll` included.
    #[tokio::test]
    async fn view_schema_matches_row_keys_in_document_order() {
        let patients = vec![
            json!({"resourceType": "Patient", "id": "a", "birthDate": "1990-01-01",
                "name": [{"family": "Delta", "use": "official"}]}),
            json!({"resourceType": "Patient", "id": "b",
                "contact": [{"name": {"family": "Alpha"}}]}),
            json!({"resourceType": "Patient", "id": "c"}),
        ];
        let views = [
            json!({"resourceType": "ViewDefinition", "resource": "Patient", "status": "active",
            "select": [
                {"unionAll": [
                    {"forEach": "name", "column": [
                        {"name": "family", "path": "family", "type": "string"},
                        {"name": "use", "path": "use", "type": "code"}]},
                    {"forEach": "contact.name", "column": [
                        {"name": "family", "path": "family", "type": "string"},
                        {"name": "use", "path": "use", "type": "code"}]}
                ]},
                {"column": [{"name": "born", "path": "birthDate", "type": "date"}]}
            ]}),
            json!({"resourceType": "ViewDefinition", "resource": "Patient", "status": "active",
            "select": [
                {"column": [{"name": "id", "path": "getResourceKey()"}],
                 "select": [{"forEachOrNull": "name",
                    "column": [{"name": "family", "path": "family", "type": "string"}]}]},
                {"column": [{"name": "born", "path": "birthDate", "type": "date"}]}
            ]}),
        ];
        let runner = InProcessSofRunner::new(StaticScan::of(patients), FhirVersion::R4, "test");
        for view in views {
            let walker: Vec<String> = ViewSchema::from_view_definition(&view)
                .names()
                .map(str::to_string)
                .collect();
            let evaluator = PreparedViewDefinition::new(
                parse_view_definition_for_version(view.clone(), FhirVersion::R4).unwrap(),
            )
            .unwrap()
            .columns()
            .to_vec();
            assert_eq!(walker, evaluator, "walker = evaluator order: {view}");
            let output = runner
                .run_view(
                    &tenant(),
                    RunRequest::new(view.clone(), ViewFilters::default()),
                    RunContext::detached(RunKind::Preview),
                )
                .await
                .expect("run_view");
            let schema: Vec<String> = output.schema.names().map(str::to_string).collect();
            assert_eq!(schema, walker, "{view}");
            let rows: Vec<Value> = output.rows.map(|row| row.expect("row")).collect().await;
            assert!(!rows.is_empty(), "{view}");
            for row in &rows {
                let keys: Vec<&String> = row.as_object().expect("object").keys().collect();
                assert_eq!(keys, schema.iter().collect::<Vec<_>>(), "{row}");
            }
            assert!(
                rows.iter().any(|row| row["born"].is_null()),
                "explicit nulls: {rows:?}"
            );
        }
    }

    /// With a resolver, `resolve()` dereferences the stored Patient and the
    /// view column projects its family name.
    #[tokio::test]
    async fn resolves_stored_reference_during_view_run() {
        let scan = StaticScan::of(vec![observation()]);
        let resolver = Arc::new(StaticResolver {
            pool: vec![patient()],
        });
        let runner = InProcessSofRunner::new(scan, FhirVersion::R4, "test")
            .with_reference_resolver(resolver);

        let rows = collect_rows(&runner).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["obs_id"], "o1");
        assert_eq!(
            rows[0]["patient_family"], "Smith",
            "resolve() should dereference the stored Patient: {:?}",
            rows[0]
        );
    }

    /// Without a resolver, behavior is unchanged: `resolve()` yields a typed
    /// stub (no `name`), so the projected family is null.
    #[tokio::test]
    async fn without_resolver_reference_is_not_dereferenced() {
        let scan = StaticScan::of(vec![observation()]);
        let runner = InProcessSofRunner::new(scan, FhirVersion::R4, "test");

        let rows = collect_rows(&runner).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["obs_id"], "o1");
        assert_eq!(
            rows[0]["patient_family"],
            Value::Null,
            "without a resolver the Patient must not resolve: {:?}",
            rows[0]
        );
    }

    fn compartment_pool() -> Vec<Value> {
        vec![
            json!({ "resourceType": "Patient", "id": "p1" }),
            json!({ "resourceType": "Patient", "id": "p2" }),
            json!({
                "resourceType": "Group",
                "id": "g1",
                "type": "person",
                "actual": true,
                "member": [{ "entity": { "reference": "Patient/p1" } }]
            }),
            json!({
                "resourceType": "Observation",
                "id": "o1",
                "status": "final",
                "code": { "text": "x" },
                "subject": { "reference": "Patient/p1" }
            }),
            json!({
                "resourceType": "Observation",
                "id": "o2",
                "status": "final",
                "code": { "text": "x" },
                "subject": { "reference": "Patient/p2" }
            }),
        ]
    }

    fn observation_ids_view() -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Observation",
            "status": "active",
            "select": [{ "column": [{ "path": "id", "name": "obs_id" }] }]
        })
    }

    async fn observation_ids(runner: &dyn SofRunner, filters: ViewFilters) -> Vec<String> {
        let mut stream = runner
            .run_view(
                &tenant(),
                RunRequest::new(observation_ids_view(), filters),
                RunContext::detached(RunKind::Preview),
            )
            .await
            .map(|output| output.rows)
            .expect("run_view");
        let mut ids = Vec::new();
        while let Some(row) = stream.next().await {
            ids.push(row.expect("row")["obs_id"].as_str().unwrap().to_string());
        }
        ids.sort();
        ids
    }

    /// A `patient` filter on a view whose target type is not Patient: the
    /// scan only yields Observations, so the referenced Patients have to be
    /// pulled in for the compartment filter to recognise them — by id, not by
    /// scanning the Patient collection (#1453).
    #[tokio::test]
    async fn patient_filter_reads_the_referenced_patients_by_id() {
        let scan = StaticScan::of(compartment_pool());
        let runner = InProcessSofRunner::new(scan.clone(), FhirVersion::R4, "test");

        let ids = observation_ids(
            &runner,
            ViewFilters {
                patient: vec!["Patient/p2".to_string()],
                ..Default::default()
            },
        )
        .await;
        assert_eq!(ids, vec!["o2"]);
        assert_eq!(
            scan.scanned_types(),
            vec!["Observation"],
            "only the target type may be scanned; supporting resources are read by id"
        );
        assert_eq!(scan.reads.load(Ordering::SeqCst), 1);
    }

    /// A `group` filter: the Group named by the filter is read by id, its
    /// members' compartment decides the rows, and neither the Group nor the
    /// Patients leak into the view's output. A group-only filter needs no
    /// Patient documents at all — members are read off the Group.
    #[tokio::test]
    async fn group_filter_reads_the_group_by_id() {
        let scan = StaticScan::of(compartment_pool());
        let runner = InProcessSofRunner::new(scan.clone(), FhirVersion::R4, "test");

        let ids = observation_ids(
            &runner,
            ViewFilters {
                group: vec!["Group/g1".to_string()],
                ..Default::default()
            },
        )
        .await;
        assert_eq!(ids, vec!["o1"]);
        assert_eq!(
            scan.scanned_types(),
            vec!["Observation"],
            "a group filter must not scan the Patient or Group collections"
        );
        assert_eq!(
            scan.reads.load(Ordering::SeqCst),
            1,
            "only the Group is fetched; nothing in the filter consumes Patient documents"
        );
    }

    /// A `Patient`-target view under a `patient` filter: the Patient
    /// collection is the corpus, so it must be streamed exactly once as the
    /// target type and never materialised as a supporting prefetch (#1453).
    #[tokio::test]
    async fn patient_target_view_scans_the_patient_collection_once() {
        let scan = StaticScan::of(compartment_pool());
        let runner = InProcessSofRunner::new(scan.clone(), FhirVersion::R4, "test");

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{ "column": [{ "path": "id", "name": "pid" }] }]
        });
        let mut stream = runner
            .run_view(
                &tenant(),
                RunRequest::new(
                    view,
                    ViewFilters {
                        patient: vec!["Patient/p2".to_string()],
                        ..Default::default()
                    },
                ),
                RunContext::detached(RunKind::Preview),
            )
            .await
            .map(|output| output.rows)
            .expect("run_view");
        let mut ids = Vec::new();
        while let Some(row) = stream.next().await {
            ids.push(row.expect("row")["pid"].as_str().unwrap().to_string());
        }
        assert_eq!(ids, vec!["p2"]);
        assert_eq!(
            scan.scanned_types(),
            vec!["Patient"],
            "the target type must be scanned exactly once, with no second pass for the filter"
        );
    }

    /// A `group` reference that resolves to no stored Group is still the
    /// spec's absent-target error, not an empty result: the typed
    /// [`SofError::NotFound`] naming it (400 `not-found` on `$sql-run`; it was
    /// `InvalidViewDefinition`, a 422, before #1864).
    #[tokio::test]
    async fn absent_group_is_an_error() {
        let scan = StaticScan::of(compartment_pool());
        let runner = InProcessSofRunner::new(scan, FhirVersion::R4, "test");

        let err = runner
            .run_view(
                &tenant(),
                RunRequest::new(
                    observation_ids_view(),
                    ViewFilters {
                        group: vec!["Group/nope".to_string()],
                        ..Default::default()
                    },
                ),
                RunContext::detached(RunKind::Preview),
            )
            .await
            .map(|output| output.rows)
            .err()
            .expect("an absent Group is refused");
        assert!(
            matches!(err, SofError::NotFound { ref references } if references == &["Group/nope"]),
            "{err:?}"
        );
    }

    /// #1864 (A19): every absent target of a run is listed, as the caller wrote
    /// it, in one typed not-found; present targets (relative or bare) are not.
    #[tokio::test]
    async fn absent_targets_are_one_typed_not_found() {
        let runner =
            InProcessSofRunner::new(StaticScan::of(compartment_pool()), FhirVersion::R4, "test");
        let err = runner
            .run_view(
                &tenant(),
                RunRequest::new(
                    observation_ids_view(),
                    ViewFilters {
                        patient: vec![
                            "Patient/p1".to_string(),
                            "absent".to_string(),
                            "Patient/gone".to_string(),
                        ],
                        group: vec!["g1".to_string(), "Group/nope".to_string()],
                        ..Default::default()
                    },
                ),
                RunContext::detached(RunKind::Preview),
            )
            .await
            .map(|output| output.rows)
            .err()
            .expect("absent targets are refused");
        match err {
            SofError::NotFound { references } => {
                assert_eq!(references, ["absent", "Patient/gone", "Group/nope"]);
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    /// The engine's own absent-target error, if it is ever reached, maps to the
    /// same typed not-found with the references it lists.
    #[test]
    fn the_engine_absent_target_error_maps_to_not_found() {
        let err = map_engine_error(helios_sof::SofError::ReferencedResourceNotFound(
            "Patient/a, Group/b not found in supplied resources".to_string(),
        ));
        assert!(
            matches!(&err, SofError::NotFound { references } if references == &["Patient/a", "Group/b"]),
            "{err:?}"
        );
    }

    /// A scan whose `scan_resources_since` override answers by a stored instant
    /// kept apart from the JSON stamp, as S3 does: each resource is
    /// `(resource, stored instant)`. It records every call.
    #[derive(Default)]
    struct StoredInstantScan {
        resources: Vec<(Value, chrono::DateTime<chrono::Utc>)>,
        /// `None` for `scan_resources`, the cutoff for `scan_resources_since`.
        calls: Mutex<Vec<Option<chrono::DateTime<chrono::Utc>>>>,
    }

    #[async_trait]
    impl ResourceScan for StoredInstantScan {
        async fn scan_resources(
            &self,
            _tenant: &TenantContext,
            resource_type: &str,
        ) -> Result<ResourceStream, SofError> {
            self.calls.lock().unwrap().push(None);
            let items: Vec<_> = self
                .resources
                .iter()
                .filter(|(r, _)| r["resourceType"] == resource_type)
                .map(|(r, _)| Ok(r.clone()))
                .collect();
            Ok(Box::pin(futures::stream::iter(items)))
        }

        async fn scan_resources_since(
            &self,
            _tenant: &TenantContext,
            resource_type: &str,
            cutoff: chrono::DateTime<chrono::Utc>,
        ) -> Result<ResourceStream, SofError> {
            self.calls.lock().unwrap().push(Some(cutoff));
            let items: Vec<_> = self
                .resources
                .iter()
                .filter(|(r, stored)| r["resourceType"] == resource_type && *stored >= cutoff)
                .map(|(r, _)| Ok(r.clone()))
                .collect();
            Ok(Box::pin(futures::stream::iter(items)))
        }

        async fn read_resources(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _ids: &[String],
        ) -> Result<Vec<Value>, SofError> {
            Ok(Vec::new())
        }
    }

    /// #1864 (R-l, A18; council C1/S7): a run with `_since` reads the scan's
    /// own `scan_resources_since` and does not filter its stream again on the
    /// (millisecond) JSON stamp: a resource whose stored instant is at the
    /// cutoff is kept although its JSON stamp is earlier. The export runner's
    /// per-job cache forwards the cutoff and keys what it keeps by it: the same
    /// cutoff replays, another cutoff (or none) reads the scan again and never
    /// sees the first cutoff's resources.
    #[tokio::test]
    async fn since_reaches_the_scan_override_through_the_job_cache() {
        let stored: chrono::DateTime<chrono::Utc> =
            "2024-05-01T12:00:00.123456789Z".parse().unwrap();
        let obs = |id: &str, stamp: &str| {
            json!({"resourceType": "Observation", "id": id, "status": "final",
                "code": {"text": "x"}, "meta": {"lastUpdated": stamp}})
        };
        let scan = Arc::new(StoredInstantScan {
            resources: vec![
                // JSON stamp truncated to the millisecond, before the cutoff.
                (obs("at", "2024-05-01T12:00:00.123Z"), stored),
                (
                    obs("before", "2024-05-01T11:00:00.000Z"),
                    "2024-05-01T11:00:00Z".parse().unwrap(),
                ),
            ],
            ..Default::default()
        });
        let runner = InProcessSofRunner::new(scan.clone(), FhirVersion::R4, "test");
        let since = |cutoff| ViewFilters {
            since: Some(cutoff),
            ..Default::default()
        };

        // The request runner: the override decides, unfiltered after it.
        assert_eq!(observation_ids(&runner, since(stored)).await, ["at"]);

        let job = runner.for_export_job().expect("an export job runner");
        let later = stored + chrono::Duration::nanoseconds(1);
        // Cache miss, then hit, for the same cutoff.
        assert_eq!(observation_ids(job.as_ref(), since(stored)).await, ["at"]);
        assert_eq!(observation_ids(job.as_ref(), since(stored)).await, ["at"]);
        // Another cutoff is its own scan.
        assert!(observation_ids(job.as_ref(), since(later)).await.is_empty());
        // No cutoff is its own scan too, and sees everything.
        assert_eq!(
            observation_ids(job.as_ref(), ViewFilters::default()).await,
            ["at", "before"]
        );
        // And the earlier cutoffs replay without reading storage again.
        assert!(observation_ids(job.as_ref(), since(later)).await.is_empty());
        assert_eq!(observation_ids(job.as_ref(), since(stored)).await, ["at"]);

        assert_eq!(
            *scan.calls.lock().unwrap(),
            [Some(stored), Some(stored), Some(later), None],
            "one request scan, then one job scan per distinct cutoff"
        );
    }

    /// `_since` is inclusive (#1707): a resource updated exactly at the cutoff is
    /// kept, as in the SQL runners (`last_updated >= ?`) and the Mongo runner
    /// (`$gte`). One with no `meta.lastUpdated` stays excluded, as everywhere.
    #[tokio::test]
    async fn since_keeps_a_resource_updated_exactly_at_the_cutoff() {
        fn obs(id: &str, last_updated: Option<&str>) -> Value {
            let mut r = json!({
                "resourceType": "Observation",
                "id": id,
                "status": "final",
                "code": {"text": "x"}
            });
            if let Some(lu) = last_updated {
                r["meta"] = json!({"lastUpdated": lu});
            }
            r
        }

        let scan = StaticScan::of(vec![
            obs("before", Some("2024-05-01T11:59:59.999Z")),
            obs("at", Some("2024-05-01T12:00:00Z")),
            // Same instant as the cutoff, written with a UTC offset.
            obs("at-offset", Some("2024-05-01T14:00:00+02:00")),
            obs("after", Some("2024-05-01T12:00:00.001Z")),
            obs("none", None),
        ]);
        let runner = InProcessSofRunner::new(scan, FhirVersion::R4, "test");

        let ids = observation_ids(
            &runner,
            ViewFilters {
                since: Some("2024-05-01T12:00:00Z".parse().unwrap()),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(ids, ["after", "at", "at-offset"]);
    }

    /// The runner an export job gets reads each type from storage once: later
    /// views over it replay what the first read. A row-capped run never leaves
    /// a partial type behind: the view that follows it still sees every row.
    #[tokio::test]
    async fn an_export_job_scans_each_type_once() {
        let resources: Vec<Value> = (0..30)
            .map(|i| {
                json!({
                    "resourceType": "Observation",
                    "id": format!("o{i}"),
                    "status": "final",
                    "code": {"text": "x"}
                })
            })
            .collect();
        let scan = StaticScan::of(resources);
        let runner = InProcessSofRunner::new(scan.clone(), FhirVersion::R4, "test");
        let job = runner.for_export_job().expect("in-process job runner");

        for _ in 0..3 {
            let ids = observation_ids(job.as_ref(), ViewFilters::default()).await;
            assert_eq!(ids.len(), 30);
        }
        assert_eq!(scan.scanned_types(), ["Observation"]);

        // A capped run must not poison the cache with a partial type.
        let capped = runner.for_export_job().expect("in-process job runner");
        let ids = observation_ids(
            capped.as_ref(),
            ViewFilters {
                limit: Some(1),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(ids.len(), 1);
        let ids = observation_ids(capped.as_ref(), ViewFilters::default()).await;
        assert_eq!(ids.len(), 30);
    }

    /// A row cap shrinks the first scan batch, but later batches still grow,
    /// so a cap larger than one batch is reached and one smaller than a
    /// batch returns exactly that many rows.
    #[tokio::test]
    async fn limit_caps_rows_across_growing_batches() {
        let resources: Vec<Value> = (0..3000)
            .map(|i| {
                json!({
                    "resourceType": "Observation",
                    "id": format!("o{i}"),
                    "status": "final",
                    "code": {"text": "x"}
                })
            })
            .collect();
        let runner = InProcessSofRunner::new(StaticScan::of(resources), FhirVersion::R4, "test");

        for cap in [1usize, 50, 1500] {
            let ids = observation_ids(
                &runner,
                ViewFilters {
                    limit: Some(cap),
                    ..Default::default()
                },
            )
            .await;
            assert_eq!(ids.len(), cap, "limit {cap}");
        }
    }

    /// A `ResourceScan` whose stream yields `ok_items` Observations and then
    /// panics while the runner reads it.
    struct PanickingScan {
        ok_items: usize,
    }

    #[async_trait]
    impl ResourceScan for PanickingScan {
        async fn scan_resources(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
        ) -> Result<ResourceStream, SofError> {
            let ok_items = self.ok_items;
            Ok(Box::pin(futures::stream::iter(0..).map(move |i| {
                assert!(
                    i < ok_items,
                    "scan double: storage failed after {ok_items} items"
                );
                Ok(json!({
                    "resourceType": "Observation",
                    "id": format!("o{i}"),
                    "status": "final",
                    "code": {"text": "x"}
                }))
            })))
        }

        async fn read_resources(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _ids: &[String],
        ) -> Result<Vec<Value>, SofError> {
            Ok(Vec::new())
        }
    }

    /// #1864 (A12): a scan that panics part way must fail the run. A run that
    /// ends `Ok` with the rows read before the panic looks complete and is not.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_panicking_scan_fails_the_run() {
        // More than one batch, so rows from before the panic reach the consumer.
        let ok_items = CHUNK_SIZE + 10;
        let runner = InProcessSofRunner::new(
            Arc::new(PanickingScan { ok_items }),
            FhirVersion::R4,
            "test",
        );
        let mut stream = runner
            .run_view(
                &tenant(),
                RunRequest::new(observation_ids_view(), ViewFilters::default()),
                RunContext::detached(RunKind::Preview),
            )
            .await
            .map(|output| output.rows)
            .expect("run_view");
        let mut rows = 0usize;
        let mut error = None;
        let drained = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while let Some(item) = stream.next().await {
                match item {
                    Ok(_) => rows += 1,
                    Err(e) => {
                        error = Some(e);
                        break;
                    }
                }
            }
        })
        .await;
        assert!(drained.is_ok(), "the run must end, not hang");
        assert!(
            error.is_some(),
            "a scan that panicked after {ok_items} items must fail the run; it ended Ok with \
             {rows} rows"
        );
    }
    /// #1864 (A12): with the end marker, a scan that ends normally still ends
    /// the run normally, with every row, whether it held no resource, one, a
    /// whole number of batches or a partial last batch.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn clean_scans_of_zero_one_and_many_batches_end_normally() {
        for count in [0, 1, CHUNK_SIZE, 2 * CHUNK_SIZE, 2 * CHUNK_SIZE + 7] {
            let resources: Vec<Value> = (0..count)
                .map(|i| {
                    json!({"resourceType": "Observation", "id": format!("o{i}"),
                        "status": "final", "code": {"text": "x"}})
                })
                .collect();
            let runner =
                InProcessSofRunner::new(StaticScan::of(resources), FhirVersion::R4, "test");
            let mut stream = runner
                .run_view(
                    &tenant(),
                    RunRequest::new(observation_ids_view(), ViewFilters::default()),
                    RunContext::detached(RunKind::Preview),
                )
                .await
                .map(|output| output.rows)
                .expect("run_view");
            let mut ids = std::collections::HashSet::new();
            let drained = tokio::time::timeout(std::time::Duration::from_secs(60), async {
                while let Some(item) = stream.next().await {
                    let row = item.unwrap_or_else(|e| panic!("{count} resources: {e}"));
                    ids.insert(row["obs_id"].as_str().expect("obs_id").to_string());
                }
            })
            .await;
            assert!(drained.is_ok(), "{count} resources: the run must end");
            assert_eq!(ids.len(), count, "{count} resources: every row, once");
        }
    }

    /// #1864 (S4): a run stopped while a chunk is evaluated ends with the
    /// stop's error item only after the evaluation returned, and the
    /// evaluation stops per resource rather than finishing the chunk.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_during_an_evaluation_ends_the_run_after_it_stopped() {
        // Patients with many names under a `forEach`: one chunk takes far
        // longer to evaluate than the stop bound allows.
        let resources: Vec<Value> = (0..CHUNK_SIZE)
            .map(|i| {
                let names: Vec<Value> = (0..400)
                    .map(|n| json!({"family": format!("f{i}-{n}")}))
                    .collect();
                json!({"resourceType": "Patient", "id": format!("p{i}"), "name": names})
            })
            .collect();
        let view = json!({
            "resourceType": "ViewDefinition", "resource": "Patient", "status": "active",
            "select": [{"forEach": "name", "column": [{"name": "family", "path": "family"}]}]
        });
        let runner = InProcessSofRunner::new(StaticScan::of(resources), FhirVersion::R4, "test");
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let mut stream = runner
            .run_view(
                &tenant(),
                RunRequest::new(view, ViewFilters::default()),
                ctx,
            )
            .await
            .map(|output| output.rows)
            .expect("run_view");
        // The engine is evaluating the only chunk.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        cancel.cancel();
        let cancelled_at = std::time::Instant::now();
        let mut rows = 0usize;
        let last = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while let Some(item) = stream.next().await {
                match item {
                    Ok(_) => rows += 1,
                    Err(e) => return Some(e),
                }
            }
            None
        })
        .await
        .expect("the run ends");
        let ended_after = cancelled_at.elapsed();
        eprintln!("SOF-CANCEL in-process evaluation: ended {ended_after:?} after the cancel");
        assert_eq!(rows, 0, "a stopped chunk yields no partial rows");
        assert!(
            matches!(last, Some(SofError::Cancelled)),
            "expected a Cancelled item, got {last:?}"
        );
        assert!(
            ended_after < std::time::Duration::from_secs(1),
            "the evaluation ran {ended_after:?} after the cancel"
        );
    }

    /// Set once the scan stream that owns it has been dropped; the drop is
    /// slow on purpose, so a run that ends without waiting for its scan
    /// reports the stop before the flag is set.
    struct DropProbe(Arc<AtomicBool>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            std::thread::sleep(std::time::Duration::from_millis(200));
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// A `ResourceScan` whose endless stream holds a [`DropProbe`].
    struct SlowDropScan {
        dropped: Arc<AtomicBool>,
    }

    #[async_trait]
    impl ResourceScan for SlowDropScan {
        async fn scan_resources(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
        ) -> Result<ResourceStream, SofError> {
            let probe = DropProbe(Arc::clone(&self.dropped));
            Ok(Box::pin(futures::stream::iter(0..).map(move |i| {
                let _probe = &probe;
                Ok(json!({"resourceType": "Observation", "id": format!("o{i}"),
                    "status": "final", "code": {"text": "x"}}))
            })))
        }

        async fn read_resources(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _ids: &[String],
        ) -> Result<Vec<Value>, SofError> {
            Ok(Vec::new())
        }
    }

    /// #1864 (BD-N5-2): a stopped run ends with its error item only after the
    /// scan task stopped, so the scan's stream (and any storage read it would
    /// still start, such as an S3 GET) is gone before the consumer sees the
    /// end. Before the scan was joined, the engine ended the stream while the
    /// scan could still be reading.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stopped_run_ends_only_after_its_scan_stopped() {
        let dropped = Arc::new(AtomicBool::new(false));
        let runner = InProcessSofRunner::new(
            Arc::new(SlowDropScan {
                dropped: Arc::clone(&dropped),
            }),
            FhirVersion::R4,
            "test",
        );
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let mut stream = runner
            .run_view(
                &tenant(),
                RunRequest::new(observation_ids_view(), ViewFilters::default()),
                ctx,
            )
            .await
            .map(|output| output.rows)
            .expect("run_view");
        stream.next().await.expect("a row").expect("a row");
        cancel.cancel();
        let last = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(item) = stream.next().await {
                if let Err(e) = item {
                    return Some(e);
                }
            }
            None
        })
        .await
        .expect("the run ends");
        let scan_gone = dropped.load(Ordering::SeqCst);
        assert!(
            matches!(last, Some(SofError::Cancelled)),
            "expected a Cancelled item, got {last:?}"
        );
        assert!(
            scan_gone,
            "the run ended before its scan task stopped (the scan stream was still alive)"
        );
    }
}
