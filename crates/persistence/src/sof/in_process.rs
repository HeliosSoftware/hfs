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
//! The chunk-processing loop runs in a `spawn_blocking` thread whose
//! `JoinHandle` is watched by [`watch_row_producer`](crate::core::sof_runner::watch_row_producer)
//! so a panic reaches the consumer as an `Err` item instead of a silent end
//! of stream.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use helios_fhir::FhirVersion;
use helios_sof::{
    CompartmentFilter, PreparedViewDefinition, ResourceChunk, compartment_reference_ids,
    parse_view_definition_for_version,
};
use serde_json::{Map, Value};
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;

use crate::core::sof_runner::{
    RowStream, SofError, SofRunner, ViewFilters, ViewRow, watch_row_producer,
};
use crate::sof::reference_resolver::{StorageReferenceResolver, collect_missing_references};
use crate::tenant::TenantContext;

/// Channel buffer depth (rows that can be queued ahead of the consumer).
const CHANNEL_BUFFER: usize = 256;

/// Number of resources handed to the engine per [`PreparedViewDefinition::process_chunk`]
/// call. Bounds peak memory when scanning large resource types.
const CHUNK_SIZE: usize = 1024;

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

/// Maps a `helios_sof` engine error onto the persistence-layer [`SofError`].
///
/// Structural/validation problems and the spec's absent-target case become
/// `InvalidViewDefinition` (surfaced as a 4xx); everything else is a runtime
/// backend error.
fn map_engine_error(e: helios_sof::SofError) -> SofError {
    use helios_sof::SofError as E;
    match e {
        E::InvalidViewDefinition(m) => SofError::InvalidViewDefinition(m),
        E::ReferencedResourceNotFound(m) => SofError::InvalidViewDefinition(m),
        other => SofError::Backend(other.to_string()),
    }
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
) -> Vec<Value> {
    let Some(r) = resolver else {
        return Vec::new();
    };
    let refs = collect_missing_references(resources);
    if refs.is_empty() {
        return Vec::new();
    }
    r.resolve(tenant, fhir_version, &refs).await
}

#[async_trait]
impl SofRunner for InProcessSofRunner {
    fn runner_name(&self) -> &'static str {
        self.runner_name
    }

    async fn run_view(
        &self,
        tenant: &TenantContext,
        view_definition: Value,
        filters: ViewFilters,
    ) -> Result<RowStream, SofError> {
        let view = parse_view_definition_for_version(view_definition, self.fhir_version)
            .map_err(map_engine_error)?;
        let prepared = PreparedViewDefinition::new(view).map_err(map_engine_error)?;
        let resource_type = prepared.target_resource_type().to_string();

        debug!(
            runner = self.runner_name,
            tenant = %tenant.tenant_id(),
            resource_type = %resource_type,
            "executing in-process ViewDefinition"
        );

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
                        supporting.extend(
                            self.scan
                                .read_resources(tenant, supporting_type, &ids)
                                .await?,
                        );
                    }
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

        let scan_stream = self.scan.scan_resources(tenant, &resource_type).await?;

        let limit = filters.limit;
        let version = self.fhir_version;
        let since = filters.since;
        let tenant_owned = tenant.clone();
        let resolver = self.resolver.clone();

        // Resource channel: batches of (resources, pre-resolved external refs)
        // from the async scan task to the blocking engine task.
        let (res_tx, mut res_rx) = tokio::sync::mpsc::channel::<
            Result<(Vec<Value>, Vec<Value>), SofError>,
        >(RESOURCE_CHANNEL_BUFFER);

        // Row channel: output rows from the blocking engine to the caller.
        let (row_tx, row_rx) =
            tokio::sync::mpsc::channel::<Result<ViewRow, SofError>>(CHANNEL_BUFFER);
        let guard_tx = row_tx.clone();

        // Async scan task: drive the cursor, apply the `since` filter per resource,
        // accumulate batches of CHUNK_SIZE, pre-resolve external refs per batch,
        // and forward to the blocking engine via the resource channel.
        tokio::spawn(async move {
            let mut stream = scan_stream;
            let mut batch: Vec<Value> = Vec::with_capacity(CHUNK_SIZE);

            while let Some(item) = stream.next().await {
                let resource = match item {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = res_tx.send(Err(e)).await;
                        return;
                    }
                };

                if let Some(cutoff) = since {
                    let passes = resource
                        .get("meta")
                        .and_then(|m| m.get("lastUpdated"))
                        .and_then(|lu| lu.as_str())
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|t| t.with_timezone(&chrono::Utc) > cutoff)
                        .unwrap_or(false);
                    if !passes {
                        continue;
                    }
                }

                batch.push(resource);

                if batch.len() == CHUNK_SIZE {
                    let external =
                        resolve_batch_external(&resolver, &tenant_owned, version, &batch).await;
                    if res_tx
                        .send(Ok((std::mem::take(&mut batch), external)))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }

            if !batch.is_empty() {
                let external =
                    resolve_batch_external(&resolver, &tenant_owned, version, &batch).await;
                let _ = res_tx.send(Ok((batch, external))).await;
            }
        });

        // Blocking engine task: apply the compartment filter (CPU-bound FHIRPath
        // evaluation) and drive PreparedViewDefinition::process_chunk_with_external.
        let producer = tokio::task::spawn_blocking(move || {
            let columns = prepared.columns().to_vec();
            let mut emitted = 0usize;
            let mut chunk_index = 0usize;

            // One-batch lookahead. `is_last` has to mean "no further batches
            // exist", which an `is_empty()` probe on the receiver cannot tell
            // you — under backpressure the channel is momentarily drained for
            // most batches whenever the engine outruns the scan. Holding the
            // next batch back is the only way to know the current one is final.
            let mut pending = res_rx.blocking_recv();

            while let Some(item) = pending.take() {
                let (resources, external_json) = match item {
                    Err(e) => {
                        let _ = row_tx.blocking_send(Err(e));
                        return;
                    }
                    Ok(batch) => batch,
                };
                pending = res_rx.blocking_recv();
                let is_last = pending.is_none();

                let resources: Vec<Value> = match &compartment_filter {
                    Some(cf) => {
                        let mut filtered = Vec::with_capacity(resources.len());
                        for r in resources {
                            match cf.apply(&r) {
                                Ok(true) => filtered.push(r),
                                Ok(false) => {}
                                Err(e) => {
                                    let _ = row_tx.blocking_send(Err(map_engine_error(e)));
                                    return;
                                }
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
                        helios_sof::parse_json_to_fhir_resource_pub(json, version).ok()
                    })
                    .collect();

                let chunk = ResourceChunk {
                    resources,
                    chunk_index,
                    is_last,
                };
                chunk_index += 1;

                let result = match prepared.process_chunk_with_external(chunk, external) {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = row_tx.blocking_send(Err(map_engine_error(e)));
                        return;
                    }
                };

                for row in &result.rows {
                    if let Some(cap) = limit {
                        if emitted >= cap {
                            return;
                        }
                    }
                    emitted += 1;
                    let view_row = row_to_view_row(&columns, &row.values);
                    if row_tx.blocking_send(Ok(view_row)).is_err() {
                        return;
                    }
                }
            }

            debug!(rows = emitted, "in-process view run complete");
        });
        watch_row_producer(self.runner_name(), guard_tx, producer);

        Ok(Box::pin(ReceiverStream::new(row_rx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        ) -> Vec<Value> {
            refs.iter()
                .filter_map(|(rt, id)| {
                    self.pool.iter().find(|r| {
                        r.get("resourceType").and_then(Value::as_str) == Some(rt.as_str())
                            && r.get("id").and_then(Value::as_str) == Some(id.as_str())
                    })
                })
                .cloned()
                .collect()
        }
    }

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
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
            .run_view(&tenant(), resolve_view(), ViewFilters::default())
            .await
            .expect("run_view");
        let mut rows = Vec::new();
        while let Some(row) = stream.next().await {
            rows.push(row.expect("row"));
        }
        rows
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

    async fn observation_ids(runner: &InProcessSofRunner, filters: ViewFilters) -> Vec<String> {
        let mut stream = runner
            .run_view(&tenant(), observation_ids_view(), filters)
            .await
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
                view,
                ViewFilters {
                    patient: vec!["Patient/p2".to_string()],
                    ..Default::default()
                },
            )
            .await
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
    /// spec's absent-target error, not an empty result.
    #[tokio::test]
    async fn absent_group_is_an_error() {
        let scan = StaticScan::of(compartment_pool());
        let runner = InProcessSofRunner::new(scan, FhirVersion::R4, "test");

        let err = runner
            .run_view(
                &tenant(),
                observation_ids_view(),
                ViewFilters {
                    group: vec!["Group/nope".to_string()],
                    ..Default::default()
                },
            )
            .await
            .err()
            .expect("an absent Group is refused");
        assert!(
            matches!(err, SofError::InvalidViewDefinition(ref m) if m.contains("Group/nope")),
            "{err:?}"
        );
    }
}
