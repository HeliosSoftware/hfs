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

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use helios_sof::{
    PreparedViewDefinition, ResourceChunk, filter_resources_by_patient_and_group,
    filter_resources_by_since, parse_view_definition_for_version,
};
use serde_json::{Map, Value};
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;

use crate::core::sof_runner::{RowStream, SofError, SofRunner, ViewFilters, ViewRow};
use crate::tenant::TenantContext;

/// Channel buffer depth (rows that can be queued ahead of the consumer).
const CHANNEL_BUFFER: usize = 256;

/// Number of resources handed to the engine per [`PreparedViewDefinition::process_chunk`]
/// call. Bounds peak memory when scanning large resource types.
const CHUNK_SIZE: usize = 1024;

/// Streams the live resources of a single resource type for a tenant.
///
/// Implemented by backends that lack an in-DB SOF runner so they can reuse
/// [`InProcessSofRunner`]. Implementations must return the raw FHIR resource
/// JSON (the `content()` of each stored resource), excluding soft-deleted ones.
#[async_trait]
pub trait ResourceScan: Send + Sync {
    /// Returns every live resource of `resource_type` visible to `tenant` as
    /// raw FHIR JSON.
    async fn scan_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> Result<Vec<Value>, SofError>;
}

/// In-process SQL-on-FHIR runner backed by an arbitrary [`ResourceScan`].
pub struct InProcessSofRunner {
    scan: std::sync::Arc<dyn ResourceScan>,
    fhir_version: FhirVersion,
    runner_name: &'static str,
}

impl InProcessSofRunner {
    /// Creates a runner that scans resources via `scan` and evaluates views
    /// against `fhir_version`. `runner_name` is surfaced in logs/diagnostics
    /// (e.g. `"s3-in-process"`).
    pub fn new(
        scan: std::sync::Arc<dyn ResourceScan>,
        fhir_version: FhirVersion,
        runner_name: &'static str,
    ) -> Self {
        Self {
            scan,
            fhir_version,
            runner_name,
        }
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

        // Scan the view's target type, then apply the SoF run filters that the
        // engine itself does not handle (patient/group compartment and `since`).
        let mut resources = self.scan.scan_resources(tenant, &resource_type).await?;
        if let Some(since) = filters.since {
            resources = filter_resources_by_since(resources, since).map_err(map_engine_error)?;
        }
        if !filters.patient.is_empty() || !filters.group.is_empty() {
            resources = filter_resources_by_patient_and_group(
                resources,
                &filters.patient,
                &filters.group,
                self.fhir_version,
            )
            .map_err(map_engine_error)?;
        }

        let limit = filters.limit;
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<ViewRow, SofError>>(CHANNEL_BUFFER);

        // FHIRPath evaluation is CPU-bound (and `process_chunk` parallelises via
        // rayon), so run it off the async runtime.
        tokio::task::spawn_blocking(move || {
            let columns = prepared.columns().to_vec();
            let total = resources.len();
            let mut emitted = 0usize;
            let mut offset = 0usize;
            let mut chunk_index = 0usize;

            while offset < total {
                let end = (offset + CHUNK_SIZE).min(total);
                let chunk = ResourceChunk {
                    resources: resources[offset..end].to_vec(),
                    chunk_index,
                    is_last: end >= total,
                };
                offset = end;
                chunk_index += 1;

                let result = match prepared.process_chunk(chunk) {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = tx.blocking_send(Err(map_engine_error(e)));
                        return;
                    }
                };

                for row in &result.rows {
                    if let Some(cap) = limit
                        && emitted >= cap
                    {
                        return;
                    }
                    emitted += 1;
                    let view_row = row_to_view_row(&columns, &row.values);
                    if tx.blocking_send(Ok(view_row)).is_err() {
                        // Receiver dropped (client disconnected) — stop.
                        return;
                    }
                }
            }

            debug!(rows = emitted, "in-process view run complete");
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}
