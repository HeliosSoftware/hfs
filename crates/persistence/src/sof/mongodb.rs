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
//!
//! The cursor loop runs in a `tokio::spawn` task whose `JoinHandle` is watched
//! by [`watch_row_producer`](crate::core::sof_runner::watch_row_producer) so a
//! panic or cancellation reaches the consumer as an `Err` item instead of a
//! silent end of stream.

use std::sync::Arc;

use mongodb::bson::{DateTime as BsonDateTime, Document, doc};
use mongodb::{Client, Collection};
use tokio::sync::OnceCell;
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;

use crate::backends::mongodb::backend::{MongoBackend, MongoBackendConfig, connect_client};
use crate::core::sof_runner::{
    RowStream, SofError, SofRunner, ViewFilters, ViewRow, watch_row_producer,
};
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
    /// Where a run with `patient`/`group` filters goes: the pipeline compiler
    /// has no compartment predicate yet, so those runs are handed to another
    /// runner (the backend wires the in-process engine over a scan of the
    /// same collection) instead of failing as uncompilable.
    compartment_fallback: Option<Arc<dyn SofRunner>>,
}

impl MongoInDbRunner {
    /// Creates a runner sharing the backend's client cell and configuration.
    pub fn new(client: Arc<OnceCell<Client>>, config: MongoBackendConfig) -> Self {
        Self {
            client,
            config,
            compartment_fallback: None,
        }
    }

    /// Routes runs that carry `patient`/`group` filters to `runner`. Without
    /// one, such a run is refused as uncompilable.
    pub fn with_compartment_fallback(mut self, runner: Arc<dyn SofRunner>) -> Self {
        self.compartment_fallback = Some(runner);
        self
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
            // Compartment filtering has no pipeline form yet: hand the run to
            // the fallback runner when the backend wired one.
            return match &self.compartment_fallback {
                Some(runner) => {
                    debug!(
                        runner = runner.runner_name(),
                        tenant = %tenant.tenant_id(),
                        "patient/group filters: delegating the view run"
                    );
                    runner.run_view(tenant, view_definition, filters).await
                }
                None => Err(SofError::Uncompilable {
                    reason: "patient/group filters are not yet supported by the MongoDB runner"
                        .to_string(),
                }),
            };
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
        let guard_tx = tx.clone();

        let batch_size = self.config.sof_cursor_batch_size;
        let producer = tokio::spawn(async move {
            let mut cursor = match collection
                .aggregate(pipeline)
                .batch_size(batch_size)
                .with_type::<ViewRow>()
                .await
            {
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

                // Decoded straight from the raw BSON reply: no intermediate `Document`.
                let row = match cursor.deserialize_current() {
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
        watch_row_producer(self.runner_name(), guard_tx, producer);

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{self, Bson, doc};

    use super::ViewRow;

    /// The cursor decodes each reply document straight from its raw bytes. For
    /// the JSON-origin values a view column can hold, that must produce the
    /// same `serde_json::Value` as the `Document` -> `from_bson` route it
    /// replaced.
    #[test]
    fn raw_row_decode_matches_the_document_route() {
        let row = doc! {
            "id": "obs-1",
            "text": "a \u{e9} string",
            "int": 42_i32,
            "long": 9_007_199_254_740_993_i64,
            "double": 1.5_f64,
            "flag": true,
            "nothing": Bson::Null,
            "nested": [[1_i32, [2_i32, Bson::Null]], [], ["x"]],
            "object": { "k": [ { "a": 1_i32 }, { "b": Bson::Null } ], "empty": {} },
            // serde_json arbitrary-precision number wrapper, as stored for decimals.
            "decimal": { "$serde_json::private::Number": "0.10" },
        };
        let bytes = bson::to_vec(&row).unwrap();

        let raw: ViewRow = bson::from_slice(&bytes).unwrap();
        let via_document: ViewRow = bson::from_bson(Bson::Document(row)).unwrap();

        assert_eq!(raw, via_document);
        assert_eq!(raw["nested"][0][1][1], serde_json::Value::Null);
        assert_eq!(raw["long"], serde_json::json!(9_007_199_254_740_993_i64));
    }

    /// BSON-only types never come out of the pipeline today (the emitter only
    /// produces JSON-derived values). Pin how the two routes treat them: a
    /// `DateTime` keeps a different extended-JSON spelling on the raw route,
    /// and `Decimal128` is rejected by both.
    #[test]
    fn raw_row_decode_of_bson_only_types_is_pinned() {
        use mongodb::bson::{DateTime, Decimal128};

        let row = doc! { "v": DateTime::from_millis(1_700_000_000_000) };
        let raw: ViewRow = bson::from_slice(&bson::to_vec(&row).unwrap()).unwrap();
        let via_document: ViewRow = bson::from_bson(Bson::Document(row)).unwrap();
        assert_eq!(
            raw["v"],
            serde_json::json!({"$date": {"$numberLong": "1700000000000"}})
        );
        assert_eq!(
            via_document["v"],
            serde_json::json!({"$date": "2023-11-14T22:13:20Z"})
        );

        let row = doc! { "v": "1.5".parse::<Decimal128>().unwrap() };
        assert!(bson::from_slice::<ViewRow>(&bson::to_vec(&row).unwrap()).is_err());
        assert!(bson::from_bson::<ViewRow>(Bson::Document(row)).is_err());
    }
}
