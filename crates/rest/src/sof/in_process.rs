//! In-process SQL-on-FHIR runner.
//!
//! This module provides [`InProcessRunner`], which evaluates ViewDefinitions using the
//! `helios-sof` FHIRPath engine running in-process. It is the universal fallback — every
//! storage backend can use it because it only requires a [`SearchProvider`] to page through
//! resources.
//!
//! ## How it works
//!
//! 1. Parse the raw ViewDefinition JSON into a [`SofViewDefinition`] and wrap it in a
//!    [`PreparedViewDefinition`] (validation happens here).
//! 2. Determine the `target_resource_type` from the prepared view.
//! 3. Page through matching resources via [`SearchProvider::search`] using cursor-based
//!    keyset pagination (default page size: 1000).
//! 4. Convert each page into a [`ResourceChunk`] and call
//!    [`PreparedViewDefinition::process_chunk`].
//! 5. Flatten `ProcessedRow` values into JSON objects (column name → cell value) and
//!    emit them on the output stream.
//!
//! The full result set is collected into memory before returning the stream. For TB-scale
//! workloads, use the in-DB runner (Phase 3) or the export endpoint (Phase 4) instead.

use std::sync::Arc;

use async_trait::async_trait;
use futures::stream;
use helios_fhir::FhirVersion;
use helios_persistence::core::search::SearchProvider;
use helios_persistence::core::sof_runner::{RowStream, SofError, SofRunner, ViewFilters, ViewRow};
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::SearchQuery;
use helios_sof::{PreparedViewDefinition, ResourceChunk, SofViewDefinition};
use serde_json::Value;
use tracing::debug;

/// Page size for fetching resources during in-process view evaluation.
const DEFAULT_PAGE_SIZE: u32 = 1000;

/// In-process SQL-on-FHIR runner backed by a [`SearchProvider`].
///
/// This runner is the universal fallback — it can be used with any storage backend
/// that implements [`SearchProvider`]. For backends that support in-DB compilation
/// (SQLite, PostgreSQL), prefer the in-DB runner instead.
pub struct InProcessRunner<S: SearchProvider + Send + Sync + 'static> {
    storage: Arc<S>,
    fhir_version: FhirVersion,
    page_size: u32,
}

impl<S: SearchProvider + Send + Sync + 'static> InProcessRunner<S> {
    /// Creates a new in-process runner wrapping the given search provider.
    ///
    /// # Arguments
    ///
    /// * `storage` — The search provider used to page through FHIR resources.
    /// * `fhir_version` — The FHIR version to assume when parsing ViewDefinition JSON.
    pub fn new(storage: Arc<S>, fhir_version: FhirVersion) -> Self {
        Self {
            storage,
            fhir_version,
            page_size: DEFAULT_PAGE_SIZE,
        }
    }

    /// Overrides the page size used when paging through resources.
    pub fn with_page_size(mut self, page_size: u32) -> Self {
        self.page_size = page_size;
        self
    }

    /// Parse the raw ViewDefinition JSON into a version-specific [`SofViewDefinition`].
    fn parse_view_definition(&self, json: Value) -> Result<SofViewDefinition, SofError> {
        match self.fhir_version {
            #[cfg(feature = "R4")]
            FhirVersion::R4 => {
                let vd: helios_fhir::r4::ViewDefinition =
                    serde_json::from_value(json).map_err(|e| {
                        SofError::InvalidViewDefinition(format!(
                            "failed to parse R4 ViewDefinition: {e}"
                        ))
                    })?;
                Ok(SofViewDefinition::R4(vd))
            }
            #[cfg(feature = "R4B")]
            FhirVersion::R4B => {
                let vd: helios_fhir::r4b::ViewDefinition =
                    serde_json::from_value(json).map_err(|e| {
                        SofError::InvalidViewDefinition(format!(
                            "failed to parse R4B ViewDefinition: {e}"
                        ))
                    })?;
                Ok(SofViewDefinition::R4B(vd))
            }
            #[cfg(feature = "R5")]
            FhirVersion::R5 => {
                let vd: helios_fhir::r5::ViewDefinition =
                    serde_json::from_value(json).map_err(|e| {
                        SofError::InvalidViewDefinition(format!(
                            "failed to parse R5 ViewDefinition: {e}"
                        ))
                    })?;
                Ok(SofViewDefinition::R5(vd))
            }
            #[cfg(feature = "R6")]
            FhirVersion::R6 => {
                let vd: helios_fhir::r6::ViewDefinition =
                    serde_json::from_value(json).map_err(|e| {
                        SofError::InvalidViewDefinition(format!(
                            "failed to parse R6 ViewDefinition: {e}"
                        ))
                    })?;
                Ok(SofViewDefinition::R6(vd))
            }
            #[allow(unreachable_patterns)]
            _ => Err(SofError::InvalidViewDefinition(format!(
                "FHIR version {:?} is not enabled in this build",
                self.fhir_version
            ))),
        }
    }

    /// Convert a [`helios_sof::SofError`] into the persistence-layer [`SofError`].
    fn map_sof_error(e: helios_sof::SofError) -> SofError {
        match e {
            helios_sof::SofError::InvalidViewDefinition(msg) => {
                SofError::InvalidViewDefinition(msg)
            }
            other => SofError::Backend(other.to_string()),
        }
    }

    /// Convert a `ProcessedRow` to a flat JSON object using the column name list.
    fn row_to_json(columns: &[String], values: Vec<Option<Value>>) -> ViewRow {
        let map: serde_json::Map<String, Value> = columns
            .iter()
            .zip(values)
            .filter_map(|(col, val)| val.map(|v| (col.clone(), v)))
            .collect();
        Value::Object(map)
    }
}

#[async_trait]
impl<S: SearchProvider + Send + Sync + 'static> SofRunner for InProcessRunner<S> {
    fn runner_name(&self) -> &'static str {
        "inprocess"
    }

    async fn run_view<'a>(
        &'a self,
        tenant: &'a TenantContext,
        view_definition: Value,
        filters: ViewFilters,
    ) -> Result<RowStream<'a>, SofError> {
        // Step 1 — parse and prepare the ViewDefinition
        let sof_view = self.parse_view_definition(view_definition)?;
        let prepared = PreparedViewDefinition::new(sof_view).map_err(Self::map_sof_error)?;
        let resource_type = prepared.target_resource_type().to_string();

        debug!(
            runner = "inprocess",
            resource_type = %resource_type,
            "starting in-process view run"
        );

        // Step 2 — page through resources and collect all output rows
        let mut all_rows: Vec<Result<ViewRow, SofError>> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut chunk_index: usize = 0;
        let mut total_emitted: usize = 0;
        let limit = filters.limit;

        loop {
            let mut query = SearchQuery::new(&resource_type);
            query.count = Some(self.page_size);
            query.cursor = cursor.clone();

            let result = self
                .storage
                .search(tenant, &query)
                .await
                .map_err(|e| SofError::Storage(e.to_string()))?;

            let next_cursor = result.resources.page_info.next_cursor.clone();
            let is_last = next_cursor.is_none();

            let resources: Vec<Value> = result
                .resources
                .items
                .into_iter()
                .map(|r| r.into_content())
                .collect();

            if !resources.is_empty() {
                let chunk = ResourceChunk {
                    resources,
                    chunk_index,
                    is_last,
                };

                let chunked = prepared.process_chunk(chunk).map_err(Self::map_sof_error)?;

                let columns = chunked.columns.clone();
                for row in chunked.rows {
                    if let Some(cap) = limit {
                        if total_emitted >= cap {
                            break;
                        }
                    }
                    let json = Self::row_to_json(&columns, row.values);
                    all_rows.push(Ok(json));
                    total_emitted += 1;
                }
            }

            chunk_index += 1;

            // Stop if we've hit the limit or there are no more pages
            if is_last {
                break;
            }
            if let Some(cap) = limit {
                if total_emitted >= cap {
                    break;
                }
            }

            cursor = next_cursor;
        }

        debug!(
            runner = "inprocess",
            rows = total_emitted,
            "in-process view run complete"
        );

        Ok(Box::pin(stream::iter(all_rows)))
    }
}
