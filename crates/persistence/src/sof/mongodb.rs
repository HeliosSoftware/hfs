//! MongoDB in-DB SQL-on-FHIR runner.
//!
//! [`MongoInDbRunner`] compiles a ViewDefinition to a MongoDB aggregation
//! pipeline (via [`compile_view_definition_mongo`](super::compiler::compile_view_definition_mongo))
//! and executes it directly against the `resources` collection, bypassing
//! in-process FHIRPath evaluation. Resources are stored as native BSON
//! sub-documents under `data`, so the emitted pipeline navigates them with
//! `$getField`/dotted paths.
//!
//! The runtime filters that depend on the request (tenant, `_since`, row limit)
//! are layered onto the compiled pipeline here: a leading `$match` constrains
//! the tenant (and `last_updated` for `since`), and a trailing `$limit` caps the
//! output.

use std::sync::Arc;

use mongodb::bson::{Bson, DateTime as BsonDateTime, Document, doc};
use mongodb::{Client, Collection};
use tokio::sync::OnceCell;
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;

use crate::backends::mongodb::backend::{MongoBackend, MongoBackendConfig, connect_client};
use crate::core::sof_runner::{RowStream, SofError, SofRunner, ViewFilters, ViewRow};
use crate::tenant::TenantContext;

use super::compiler::compile_view_definition_mongo;

/// Channel buffer depth (rows that can be queued ahead of the consumer).
const CHANNEL_BUFFER: usize = 256;

/// SQL-on-FHIR runner that compiles ViewDefinitions to MongoDB aggregation
/// pipelines.
pub struct MongoInDbRunner {
    /// Shared (lazily initialised) client cell — the same pool the backend uses.
    client: Arc<OnceCell<Client>>,
    config: MongoBackendConfig,
}

impl MongoInDbRunner {
    /// Creates a runner sharing the backend's client cell and configuration.
    pub fn new(client: Arc<OnceCell<Client>>, config: MongoBackendConfig) -> Self {
        Self { client, config }
    }

    /// Resolves the `resources` collection, initialising the shared client on
    /// first use.
    async fn resources(&self) -> Result<Collection<Document>, SofError> {
        let client = self
            .client
            .get_or_try_init(|| connect_client(&self.config))
            .await
            .map_err(|e| SofError::Storage(e.to_string()))?;
        Ok(client
            .database(&self.config.database_name)
            .collection(MongoBackend::RESOURCES_COLLECTION))
    }
}

#[async_trait::async_trait]
impl SofRunner for MongoInDbRunner {
    fn runner_name(&self) -> &'static str {
        "mongo-indb"
    }

    async fn run_view(
        &self,
        tenant: &TenantContext,
        view_definition: serde_json::Value,
        filters: ViewFilters,
    ) -> Result<RowStream, SofError> {
        if !filters.patient.is_empty() || !filters.group.is_empty() {
            // Compartment filtering for the Mongo runner lands in a later stage.
            return Err(SofError::Uncompilable {
                reason: "patient/group filters are not yet supported by the MongoDB runner"
                    .to_string(),
            });
        }

        let compiled = compile_view_definition_mongo(&view_definition, self.config.fhir_version)?;

        debug!(
            runner = "mongo-indb",
            tenant = %tenant.tenant_id(),
            "executing compiled ViewDefinition"
        );

        // Leading tenant (and optional `since`) predicate, prepended to the
        // compiled pipeline. Kept first so the `tenant_id + resource_type +
        // is_deleted` index can serve the scan.
        let mut tenant_match = doc! { "tenant_id": tenant.tenant_id().to_string() };
        if let Some(since) = filters.since {
            tenant_match.insert(
                "last_updated",
                doc! { "$gte": BsonDateTime::from_millis(since.timestamp_millis()) },
            );
        }

        let mut pipeline: Vec<Document> = Vec::with_capacity(compiled.pipeline.len() + 2);
        pipeline.push(doc! { "$match": tenant_match });
        pipeline.extend(compiled.pipeline);
        if let Some(limit) = filters.limit {
            pipeline.push(doc! { "$limit": limit as i64 });
        }

        let collection = self.resources().await?;
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<ViewRow, SofError>>(CHANNEL_BUFFER);

        tokio::spawn(async move {
            let mut cursor = match collection.aggregate(pipeline).await {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx
                        .send(Err(SofError::Backend(format!("aggregate failed: {e}"))))
                        .await;
                    return;
                }
            };

            let mut count = 0usize;
            loop {
                match cursor.advance().await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(e) => {
                        let _ = tx
                            .send(Err(SofError::Backend(format!("cursor advance: {e}"))))
                            .await;
                        return;
                    }
                }

                let doc = match cursor.deserialize_current() {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = tx
                            .send(Err(SofError::Backend(format!("cursor deserialize: {e}"))))
                            .await;
                        return;
                    }
                };

                let row = match mongodb::bson::from_bson::<ViewRow>(Bson::Document(doc)) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = tx
                            .send(Err(SofError::Backend(format!("row decode: {e}"))))
                            .await;
                        return;
                    }
                };

                count += 1;
                if tx.send(Ok(row)).await.is_err() {
                    // Receiver dropped (client disconnected) — stop iterating.
                    return;
                }
            }

            debug!(
                runner = "mongo-indb",
                rows = count,
                "in-DB view run complete"
            );
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}
