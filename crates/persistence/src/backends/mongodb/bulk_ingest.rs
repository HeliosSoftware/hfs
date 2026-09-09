//! Batched `$bulk-submit` ingest for the MongoDB backend (#1000).
//!
//! # Why this module exists
//!
//! The per-entry path this replaces called [`ResourceStorage::read`],
//! [`ResourceStorage::create`] or [`ResourceStorage::update`], `record_change`
//! and `store_entry_result` once per NDJSON line. Each of those is at least one
//! round trip, and `create` alone is five — existence probe, resource insert,
//! history insert, search-index delete, search-index insert — plus a transaction
//! commit. Nine round trips per resource against a server answering in ~2 ms is
//! ~60 resources/s no matter how idle the server is, which is exactly what
//! #1000 measured: `mongod` at 32 % CPU with HFS idle.
//!
//! So the fix is not to make any single operation faster. It is to stop paying
//! per resource for what can be paid per batch: this module plans a whole batch
//! in memory, then writes it with a fixed, small number of commands — one `find`
//! to resolve what already exists, then one `insert` or `update` per collection
//! — however many entries the batch holds.
//!
//! # Durability shape
//!
//! The flush is a sequence of independent commands, not one transaction. That is
//! deliberate:
//!
//! * The path it replaces was not atomic across a batch either. It opened a
//!   *best-effort* per-resource transaction — none at all on a standalone
//!   `mongod` — and wrote the rollback log and the receipt outside it.
//! * A batch-wide transaction would turn one transient error into a whole
//!   batch's worth of lost entries instead of one, which is the failure #1001 is
//!   already about.
//!
//! The commands are ordered so a crash mid-flush leaves a state that re-ingesting
//! the file converges from: resources, then history, then the derived search
//! index, then the rollback log, then the receipts. Receipts land last, so an
//! interrupted batch is re-processed rather than falsely reported done.
//!
//! # Working memory
//!
//! A batch is planned before any of it is written, so it holds the batch's
//! payloads several times over — the parsed entries, the rows the pre-read
//! found, the planned content, one `resource_history` document per version, and
//! the previous content each rollback record carries. The per-entry path already
//! buffered the entries themselves (`process_ndjson_stream` fills a
//! `Vec<NdjsonEntry>` before calling in), so this is a constant factor on top of
//! a batch that was always batch-sized; `HFS_BULK_SUBMIT_BATCH_SIZE` is the knob
//! if a corpus of very large resources makes that factor matter (#995).
//!
//! # Equivalence with the per-entry path
//!
//! Entries are planned in input order against an overlay carrying writes staged
//! earlier *in the same batch*, so a repeated id behaves as it did when each
//! entry ran on its own. Repeated ids are then coalesced into a single
//! `resources` write carrying the last version's content — the same end state
//! the per-entry path reached by overwriting the row once per entry — while
//! `resource_history`, the rollback log and the receipts still get one document
//! per entry.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use futures::stream::{StreamExt, TryStreamExt};
use helios_fhir::FhirVersion;
use mongodb::{
    Database,
    bson::{Bson, Document, doc},
    error::ErrorKind,
};
use serde_json::Value;

use crate::core::ResourceStorage;
use crate::core::{
    BulkEntryResult, BulkProcessingOptions, NdjsonEntry, SubmissionChange, SubmissionId,
};
use crate::error::{ConcurrencyError, ResourceError, StorageError, StorageResult};
use crate::tenant::{Operation, TenantContext};

use super::MongoBackend;
use super::storage::{
    chrono_to_bson, document_to_value, ensure_resource_identity, extract_created_at,
    extract_fhir_version, internal_error, next_version, value_to_document,
};

/// Receipt upserts per `update` command.
///
/// The server accepts 100 000 statements, but the command document is capped at
/// 16 MB and a receipt carries an `OperationOutcome` when the entry failed, so
/// the cap that binds is size. 500 keeps even an all-failed batch inside it with
/// room to spare, and any batch at or below that goes in a single command.
const UPDATE_STATEMENTS_PER_COMMAND: usize = 500;

/// Documents per `insert_many`.
///
/// `insert_many` splits oversized input itself, but a chunk here bounds how much
/// the driver serializes at once — a batch's `search_index` documents run to
/// ~21 per resource.
const INSERT_DOCS_PER_COMMAND: usize = 5_000;

/// What a batch does to one `(resource_type, id)`.
struct ResourcePlan {
    resource_type: String,
    id: String,
    /// `Some(v)`: the row existed at version `v` before the batch and the batch
    /// updates it. `None`: the batch creates the row.
    base_version: Option<String>,
    /// Version the stored row carries once the batch lands.
    version: String,
    /// Content the stored row carries once the batch lands.
    content: Value,
    created_at: DateTime<Utc>,
    last_updated: DateTime<Utc>,
    fhir_version: FhirVersion,
}

/// A resource row as the batch found it, projected to what planning needs.
struct ExistingResource {
    version_id: String,
    content: Value,
    created_at: DateTime<Utc>,
    fhir_version: FhirVersion,
    is_deleted: bool,
}

/// The whole batch, resolved but not yet written.
struct PlannedBatch {
    results: Vec<BulkEntryResult>,
    plans: Vec<ResourcePlan>,
    /// `(plan index, history document)` — one per written version.
    history: Vec<(usize, Document)>,
    /// `(plan index, result index, change)` — one per entry that wrote.
    changes: Vec<(usize, usize, SubmissionChange)>,
    error_count: u32,
    aborted_on_max_errors: bool,
    /// A `SearchParameter` was written, so the tenant overlay cache is stale.
    touched_search_parameters: bool,
}

/// Outcome of ingesting one batch.
pub(super) struct BatchOutcome {
    /// One result per entry, in input order.
    pub results: Vec<BulkEntryResult>,
    /// How many of them are errors.
    pub error_count: u32,
    /// The batch stopped early because `max_errors` was reached with
    /// `continue_on_error` off. Everything planned before that point was still
    /// written, exactly as the per-entry path left it.
    pub aborted_on_max_errors: bool,
}

impl MongoBackend {
    /// Ingests one batch of NDJSON entries in a fixed number of commands.
    ///
    /// See the [module docs](self) for the write order and why the batch is not
    /// one transaction.
    pub(super) async fn ingest_batch(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entries: &[NdjsonEntry],
        options: &BulkProcessingOptions,
    ) -> StorageResult<BatchOutcome> {
        let db = self.get_database().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let existing = self.load_existing(&db, tenant_id, entries).await?;
        let mut planned = self.plan_batch(tenant, manifest_id, entries, &existing, options)?;

        let failed = self.write_resources(&db, tenant_id, &planned).await?;
        self.write_history(&db, &mut planned, &failed).await?;
        self.write_search_index(&db, tenant_id, &planned, &failed, options)
            .await?;

        // A resource the batch could not write takes its entry's result down
        // with it: the per-entry path reported the same failure per entry, and
        // its history, index, rollback and receipt rows were never written.
        let mut error_count = planned.error_count;
        for (plan_idx, result_idx, _) in &planned.changes {
            if let Some(diagnostics) = failed.get(plan_idx) {
                let plan = &planned.plans[*plan_idx];
                planned.results[*result_idx] = BulkEntryResult::processing_error(
                    planned.results[*result_idx].line_number,
                    &plan.resource_type,
                    serde_json::json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{
                            "severity": "error",
                            "code": "exception",
                            "diagnostics": diagnostics
                        }]
                    }),
                );
                error_count += 1;
            }
        }

        self.write_changes(tenant, submission_id, &db, &planned, &failed)
            .await?;
        self.write_entry_results(&db, tenant, submission_id, manifest_id, options, &planned)
            .await?;

        // A SearchParameter write may change a tenant's overlay. The per-entry
        // path reloaded the cache once per such resource; once per batch is the
        // same invalidation for a fraction of the reloads.
        if planned.touched_search_parameters
            && let Err(e) = self.reload_stored_cache().await
        {
            tracing::warn!("SearchParameter cache reload failed: {e}");
        }

        Ok(BatchOutcome {
            results: planned.results,
            error_count,
            aborted_on_max_errors: planned.aborted_on_max_errors,
        })
    }

    /// Resolves in one `find` per resource type which of the batch's ids already
    /// have a row. Replaces two round trips per entry: the `read` the per-entry
    /// path made to decide create-vs-update, and the existence probe `create`
    /// made again for the same id.
    async fn load_existing(
        &self,
        db: &Database,
        tenant_id: &str,
        entries: &[NdjsonEntry],
    ) -> StorageResult<HashMap<(String, String), ExistingResource>> {
        let mut ids_by_type: HashMap<&str, HashSet<&str>> = HashMap::new();
        for entry in entries {
            if let Some(id) = entry.resource_id.as_deref() {
                ids_by_type
                    .entry(entry.resource_type.as_str())
                    .or_default()
                    .insert(id);
            }
        }
        if ids_by_type.is_empty() {
            return Ok(HashMap::new());
        }

        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let mut found = HashMap::new();
        for (resource_type, ids) in ids_by_type {
            let ids: Vec<Bson> = ids.into_iter().map(Bson::from).collect();
            let mut cursor = resources
                .find(doc! {
                    "tenant_id": tenant_id,
                    "resource_type": resource_type,
                    "id": { "$in": ids },
                })
                .await
                .map_err(|e| internal_error(format!("Failed to resolve batch ids: {e}")))?;

            let now = Utc::now();
            while let Some(document) = cursor
                .try_next()
                .await
                .map_err(|e| internal_error(format!("Failed to resolve batch ids: {e}")))?
            {
                let (Ok(id), Ok(version_id)) =
                    (document.get_str("id"), document.get_str("version_id"))
                else {
                    continue;
                };
                let (id, version_id) = (id.to_string(), version_id.to_string());
                let content = match document.get_document("data") {
                    Ok(payload) => document_to_value(payload)?,
                    Err(_) => Value::Null,
                };
                found.insert(
                    (resource_type.to_string(), id),
                    ExistingResource {
                        version_id,
                        content,
                        created_at: extract_created_at(&document, now),
                        fhir_version: extract_fhir_version(
                            &document,
                            FhirVersion::default_enabled(),
                        ),
                        is_deleted: document.get_bool("is_deleted").unwrap_or(false),
                    },
                );
            }
        }
        Ok(found)
    }

    /// Decides every entry's outcome without touching the database.
    fn plan_batch(
        &self,
        tenant: &TenantContext,
        manifest_id: &str,
        entries: &[NdjsonEntry],
        existing: &HashMap<(String, String), ExistingResource>,
        options: &BulkProcessingOptions,
    ) -> StorageResult<PlannedBatch> {
        let mut planned = PlannedBatch {
            results: Vec::with_capacity(entries.len()),
            plans: Vec::new(),
            history: Vec::new(),
            changes: Vec::new(),
            error_count: 0,
            aborted_on_max_errors: false,
            touched_search_parameters: false,
        };
        /// The row's state as an entry sees it: a write staged earlier in the
        /// batch wins over what the pre-read found.
        enum Seen {
            Staged(usize),
            Stored,
            Absent,
        }
        // Ids this batch has already staged a write for.
        let mut plan_for: HashMap<(String, String), usize> = HashMap::new();
        let tenant_id = tenant.tenant_id().as_str();
        let now = Utc::now();

        for entry in entries {
            if options.max_errors > 0 && planned.error_count >= options.max_errors {
                if !options.continue_on_error {
                    planned.aborted_on_max_errors = true;
                    break;
                }
                planned.results.push(BulkEntryResult::skipped(
                    entry.line_number,
                    &entry.resource_type,
                    "max errors exceeded",
                ));
                continue;
            }

            if let Some(resource_type) = entry.resource.get("resourceType").and_then(|v| v.as_str())
                && resource_type != entry.resource_type
            {
                planned.results.push(BulkEntryResult::validation_error(
                    entry.line_number,
                    &entry.resource_type,
                    serde_json::json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{
                            "severity": "error",
                            "code": "invalid",
                            "diagnostics": format!(
                                "resourceType mismatch: entry={}, payload={}",
                                entry.resource_type, resource_type
                            )
                        }]
                    }),
                ));
                planned.error_count += 1;
                continue;
            }

            let key = entry
                .resource_id
                .as_deref()
                .map(|id| (entry.resource_type.clone(), id.to_string()));

            let seen = match key.as_ref().and_then(|k| plan_for.get(k)) {
                Some(idx) => Seen::Staged(*idx),
                None => match key.as_ref().and_then(|k| existing.get(k)) {
                    // A tombstoned row is not an update target, and `create`
                    // refuses it too: its existence probe does not filter
                    // `is_deleted`, so it reports AlreadyExists. Reproduced so a
                    // re-import of a deleted resource fails as it does today.
                    Some(row) if row.is_deleted => {
                        planned.results.push(BulkEntryResult::processing_error(
                            entry.line_number,
                            &entry.resource_type,
                            serde_json::json!({
                                "resourceType": "OperationOutcome",
                                "issue": [{
                                    "severity": "error",
                                    "code": "exception",
                                    "diagnostics": StorageError::Resource(
                                        ResourceError::AlreadyExists {
                                            resource_type: entry.resource_type.clone(),
                                            id: entry.resource_id.clone().unwrap_or_default(),
                                        },
                                    ).to_string()
                                }]
                            }),
                        ));
                        planned.error_count += 1;
                        continue;
                    }
                    Some(_) => Seen::Stored,
                    None => Seen::Absent,
                },
            };

            let is_update = !matches!(seen, Seen::Absent);
            if is_update && !options.allow_updates {
                planned.results.push(BulkEntryResult::skipped(
                    entry.line_number,
                    &entry.resource_type,
                    "updates not allowed",
                ));
                continue;
            }

            let operation = if is_update {
                Operation::Update
            } else {
                Operation::Create
            };
            if let Err(e) = tenant.check_permission(operation, &entry.resource_type) {
                planned.results.push(BulkEntryResult::processing_error(
                    entry.line_number,
                    &entry.resource_type,
                    serde_json::json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{
                            "severity": "error",
                            "code": "exception",
                            "diagnostics": e.to_string()
                        }]
                    }),
                ));
                planned.error_count += 1;
                continue;
            }

            let result_idx = planned.results.len();
            let (plan_idx, change, created) = match seen {
                Seen::Staged(plan_idx) => {
                    let (resource_type, id, previous_version, previous_content) = {
                        let plan = &planned.plans[plan_idx];
                        (
                            plan.resource_type.clone(),
                            plan.id.clone(),
                            plan.version.clone(),
                            plan.content.clone(),
                        )
                    };
                    let new_version = next_version(&previous_version)?;
                    let mut content =
                        options.content_for_update(&previous_content, &entry.resource);
                    ensure_resource_identity(&resource_type, &id, &mut content);

                    let change = SubmissionChange::update(
                        manifest_id,
                        &resource_type,
                        &id,
                        previous_version,
                        &new_version,
                        previous_content,
                    );
                    let plan = &mut planned.plans[plan_idx];
                    plan.version = new_version;
                    plan.content = content;
                    plan.last_updated = now;
                    (plan_idx, change, false)
                }
                Seen::Stored => {
                    let (resource_type, id) = key.clone().expect("an existing row has an id");
                    let row = &existing[&(resource_type.clone(), id.clone())];
                    let new_version = next_version(&row.version_id)?;
                    let mut content = options.content_for_update(&row.content, &entry.resource);
                    ensure_resource_identity(&resource_type, &id, &mut content);

                    let change = SubmissionChange::update(
                        manifest_id,
                        &resource_type,
                        &id,
                        row.version_id.clone(),
                        &new_version,
                        row.content.clone(),
                    );
                    let plan_idx = planned.plans.len();
                    planned.plans.push(ResourcePlan {
                        resource_type: resource_type.clone(),
                        id: id.clone(),
                        base_version: Some(row.version_id.clone()),
                        version: new_version,
                        content,
                        created_at: row.created_at,
                        last_updated: now,
                        fhir_version: row.fhir_version,
                    });
                    plan_for.insert((resource_type, id), plan_idx);
                    (plan_idx, change, false)
                }
                Seen::Absent => {
                    let resource_type = entry.resource_type.clone();
                    let id = entry
                        .resource_id
                        .clone()
                        .unwrap_or_else(crate::types::new_resource_id);
                    let mut content = entry.resource.clone();
                    ensure_resource_identity(&resource_type, &id, &mut content);

                    let change = SubmissionChange::create(manifest_id, &resource_type, &id, "1");
                    let plan_idx = planned.plans.len();
                    planned.plans.push(ResourcePlan {
                        resource_type: resource_type.clone(),
                        id: id.clone(),
                        base_version: None,
                        version: "1".to_string(),
                        content,
                        created_at: now,
                        last_updated: now,
                        fhir_version: FhirVersion::default_enabled(),
                    });
                    plan_for.insert((resource_type, id), plan_idx);
                    (plan_idx, change, true)
                }
            };

            let plan = &planned.plans[plan_idx];
            let history = doc! {
                "tenant_id": tenant_id,
                "resource_type": &plan.resource_type,
                "id": &plan.id,
                "version_id": &plan.version,
                "data": Bson::Document(value_to_document(&plan.content)?),
                "created_at": chrono_to_bson(plan.created_at),
                "last_updated": chrono_to_bson(plan.last_updated),
                "is_deleted": false,
                "deleted_at": Bson::Null,
                "fhir_version": plan.fhir_version.as_mime_param(),
            };
            planned.results.push(BulkEntryResult::success(
                entry.line_number,
                &plan.resource_type,
                &plan.id,
                created,
            ));
            if plan.resource_type == "SearchParameter" {
                planned.touched_search_parameters = true;
            }
            planned.history.push((plan_idx, history));
            planned.changes.push((plan_idx, result_idx, change));
        }

        Ok(planned)
    }

    /// Writes the batch's `resources` rows: one `insert` for the creates, one
    /// `update` for the updates. Returns the plans neither applied to.
    async fn write_resources(
        &self,
        db: &Database,
        tenant_id: &str,
        planned: &PlannedBatch,
    ) -> StorageResult<HashMap<usize, String>> {
        let mut failed = HashMap::new();
        let mut create_docs = Vec::new();
        let mut create_plans = Vec::new();
        let mut update_ops = Vec::new();

        for (plan_idx, plan) in planned.plans.iter().enumerate() {
            let payload = Bson::Document(value_to_document(&plan.content)?);
            let last_updated = chrono_to_bson(plan.last_updated);
            match &plan.base_version {
                None => {
                    create_plans.push(plan_idx);
                    create_docs.push(doc! {
                        "tenant_id": tenant_id,
                        "resource_type": &plan.resource_type,
                        "id": &plan.id,
                        "version_id": &plan.version,
                        "data": payload,
                        "created_at": chrono_to_bson(plan.created_at),
                        "last_updated": last_updated,
                        "is_deleted": false,
                        "deleted_at": Bson::Null,
                        "fhir_version": plan.fhir_version.as_mime_param(),
                    });
                }
                Some(base_version) => {
                    update_ops.push((
                        plan_idx,
                        doc! {
                            "tenant_id": tenant_id,
                            "resource_type": &plan.resource_type,
                            "id": &plan.id,
                            "version_id": base_version,
                            "is_deleted": false,
                        },
                        doc! { "$set": {
                            "version_id": &plan.version,
                            "data": payload,
                            "last_updated": last_updated,
                            "is_deleted": false,
                            "deleted_at": Bson::Null,
                            "fhir_version": plan.fhir_version.as_mime_param(),
                        }},
                    ));
                }
            }
        }

        if !create_docs.is_empty() {
            let collection = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
            let mut offset = 0;
            for chunk in create_docs.chunks(INSERT_DOCS_PER_COMMAND) {
                // Unordered: one duplicate id must not stop the rest of the
                // batch, the way one failing entry did not stop the next.
                match collection.insert_many(chunk).ordered(false).await {
                    Ok(_) => {}
                    Err(e) => match e.kind.as_ref() {
                        ErrorKind::InsertMany(insert_many) => {
                            let Some(write_errors) = insert_many.write_errors.as_ref() else {
                                return Err(internal_error(format!(
                                    "Failed to insert batch resources: {e}"
                                )));
                            };
                            for write_error in write_errors {
                                let plan_idx = create_plans[offset + write_error.index];
                                let plan = &planned.plans[plan_idx];
                                // A duplicate id means the row appeared between
                                // this batch's pre-read and its insert; the
                                // per-entry path reported the same conflict from
                                // `create`'s existence probe.
                                let diagnostics = if write_error.code == 11000 {
                                    StorageError::Resource(ResourceError::AlreadyExists {
                                        resource_type: plan.resource_type.clone(),
                                        id: plan.id.clone(),
                                    })
                                    .to_string()
                                } else {
                                    format!(
                                        "Failed to insert {}/{}: {}",
                                        plan.resource_type, plan.id, write_error.message
                                    )
                                };
                                failed.insert(plan_idx, diagnostics);
                            }
                        }
                        _ => {
                            return Err(internal_error(format!(
                                "Failed to insert batch resources: {e}"
                            )));
                        }
                    },
                }
                offset += chunk.len();
            }
        }

        if !update_ops.is_empty() {
            let collection = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
            let concurrency = self.bulk_write_concurrency().clamp(1, 16);
            let applied: Vec<(usize, StorageResult<bool>)> = futures::stream::iter(update_ops)
                .map(|(plan_idx, filter, update)| {
                    let collection = collection.clone();
                    async move {
                        let outcome = collection
                            .update_one(filter, update)
                            .await
                            .map(|result| result.matched_count == 1)
                            .map_err(|e| {
                                internal_error(format!("Failed to update batch resource: {e}"))
                            });
                        (plan_idx, outcome)
                    }
                })
                .buffer_unordered(concurrency)
                .collect()
                .await;

            for (plan_idx, outcome) in applied {
                let plan = &planned.plans[plan_idx];
                match outcome {
                    Ok(true) => {}
                    // The guard matched nothing: another writer moved the row
                    // between this batch's pre-read and its write. There is no
                    // way to see that from a batched `update`'s aggregate
                    // counts — two batches that both read version N both target
                    // N+1, so the row carrying N+1 afterwards does not say whose
                    // write put it there — which is why each update is its own
                    // statement. They still go out `bulk_write_concurrency` at a
                    // time, and a fresh import has none of them at all.
                    Ok(false) => {
                        failed.insert(
                            plan_idx,
                            StorageError::Concurrency(ConcurrencyError::VersionConflict {
                                resource_type: plan.resource_type.clone(),
                                id: plan.id.clone(),
                                expected_version: plan.base_version.clone().unwrap_or_default(),
                                actual_version: "unknown".to_string(),
                            })
                            .to_string(),
                        );
                    }
                    Err(e) => {
                        failed.insert(plan_idx, e.to_string());
                    }
                }
            }
        }

        Ok(failed)
    }

    /// Writes one `resource_history` document per written version.
    async fn write_history(
        &self,
        db: &Database,
        planned: &mut PlannedBatch,
        failed: &HashMap<usize, String>,
    ) -> StorageResult<()> {
        let documents: Vec<Document> = std::mem::take(&mut planned.history)
            .into_iter()
            .filter(|(plan_idx, _)| !failed.contains_key(plan_idx))
            .map(|(_, document)| document)
            .collect();
        insert_documents(
            db,
            MongoBackend::RESOURCE_HISTORY_COLLECTION,
            documents,
            "insert batch resource history",
        )
        .await
    }

    /// Rebuilds the `search_index` rows for everything the batch wrote: one
    /// `delete` for the ids that already had rows, one `insert` for all of them.
    ///
    /// Under `defer_indexing` (`HFS_BULK_SUBMIT_DEFER_INDEXING`, #903) this
    /// writes nothing and the worker's post-manifest reindex rebuilds the index
    /// instead — the switch was a no-op on this backend until #1000, because the
    /// per-entry path reached the index through `create`/`update`, which have no
    /// way to be told to skip it.
    async fn write_search_index(
        &self,
        db: &Database,
        tenant_id: &str,
        planned: &PlannedBatch,
        failed: &HashMap<usize, String>,
        options: &BulkProcessingOptions,
    ) -> StorageResult<()> {
        if options.defer_indexing || self.is_search_offloaded() {
            return Ok(());
        }

        // Only ids that carried rows before the batch need clearing; a create
        // has nothing to delete, which is the whole of a first import. The
        // per-entry path issued this delete unconditionally, once per resource.
        let mut stale_by_type: HashMap<&str, Vec<Bson>> = HashMap::new();
        for (plan_idx, plan) in planned.plans.iter().enumerate() {
            if plan.base_version.is_some() && !failed.contains_key(&plan_idx) {
                stale_by_type
                    .entry(plan.resource_type.as_str())
                    .or_default()
                    .push(Bson::from(plan.id.as_str()));
            }
        }
        if !stale_by_type.is_empty() {
            let collection = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
            for (resource_type, ids) in stale_by_type {
                collection
                    .delete_many(doc! {
                        "tenant_id": tenant_id,
                        "resource_type": resource_type,
                        "resource_id": { "$in": ids },
                    })
                    .await
                    .map_err(|e| {
                        internal_error(format!("Failed to clear batch search index: {e}"))
                    })?;
            }
        }

        let mut documents = Vec::new();
        for (plan_idx, plan) in planned.plans.iter().enumerate() {
            if failed.contains_key(&plan_idx) {
                continue;
            }
            documents.extend(self.search_index_documents(
                tenant_id,
                &plan.resource_type,
                &plan.id,
                &plan.content,
            ));
        }
        insert_documents(
            db,
            MongoBackend::SEARCH_INDEX_COLLECTION,
            documents,
            "insert batch search index",
        )
        .await
    }

    /// Writes the batch's rollback log in one `insert`.
    async fn write_changes(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        db: &Database,
        planned: &PlannedBatch,
        failed: &HashMap<usize, String>,
    ) -> StorageResult<()> {
        let mut documents = Vec::new();
        for (plan_idx, _, change) in &planned.changes {
            if failed.contains_key(plan_idx) {
                continue;
            }
            documents.push(self.change_document(tenant, submission_id, change)?);
        }
        insert_documents(
            db,
            super::bulk_submit::CHANGES_COLLECTION,
            documents,
            "insert batch rollback log",
        )
        .await
    }

    /// Upserts the batch's per-line receipts in one `update`.
    async fn write_entry_results(
        &self,
        db: &Database,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        options: &BulkProcessingOptions,
        planned: &PlannedBatch,
    ) -> StorageResult<()> {
        if planned.results.is_empty() {
            return Ok(());
        }
        let file_url = options.file_url.as_deref().unwrap_or("");
        let mut statements = Vec::with_capacity(planned.results.len());
        for result in &planned.results {
            statements.push(self.entry_result_statement(
                tenant,
                submission_id,
                manifest_id,
                file_url,
                result,
            )?);
        }

        for chunk in statements.chunks(UPDATE_STATEMENTS_PER_COMMAND) {
            let applied = run_update_command(
                db,
                super::bulk_submit::ENTRY_RESULTS_COLLECTION,
                chunk,
                "store batch entry results",
            )
            .await?;
            if let Some((index, message)) = applied.failures.first() {
                return Err(internal_error(format!(
                    "store batch entry results: statement {index} failed: {message}"
                )));
            }
        }
        Ok(())
    }
}

/// A collection `update` command's outcome.
struct UpdateOutcome {
    /// Statements the server reported a write error for, with its message.
    failures: Vec<(usize, String)>,
}

/// Runs `statements` as one collection `update` command.
///
/// The Rust driver's `bulk_write` is a MongoDB 8.0 command and this backend
/// supports 7.0, so the multi-statement `update` command is how a batch's
/// receipt upserts go over the wire in one round trip. It suits them because
/// they need no per-statement attribution: they are keyed by
/// `(manifest, file_url, line)`, which no other writer contends for.
async fn run_update_command(
    db: &Database,
    collection: &str,
    statements: &[Document],
    context: &str,
) -> StorageResult<UpdateOutcome> {
    let response = db
        .run_command(doc! {
            "update": collection,
            "updates": statements.to_vec(),
            "ordered": false,
        })
        .await
        .map_err(|e| internal_error(format!("{context}: {e}")))?;

    let mut failures = Vec::new();
    if let Ok(write_errors) = response.get_array("writeErrors") {
        for write_error in write_errors {
            if let Some(document) = write_error.as_document() {
                let index = document
                    .get_i32("index")
                    .map(|i| i as usize)
                    .unwrap_or_default();
                let message = document
                    .get_str("errmsg")
                    .unwrap_or("unknown error")
                    .to_string();
                tracing::warn!("{context}: statement {index} failed: {message}");
                failures.push((index, message));
            }
        }
    }
    Ok(UpdateOutcome { failures })
}

/// Inserts `documents` in chunked, unordered `insert` commands.
async fn insert_documents(
    db: &Database,
    collection: &str,
    mut documents: Vec<Document>,
    context: &str,
) -> StorageResult<()> {
    if documents.is_empty() {
        return Ok(());
    }
    let collection = db.collection::<Document>(collection);
    while !documents.is_empty() {
        let take = documents.len().min(INSERT_DOCS_PER_COMMAND);
        let chunk: Vec<Document> = documents.drain(..take).collect();
        collection
            .insert_many(chunk)
            .ordered(false)
            .await
            .map_err(|e| internal_error(format!("{context}: {e}")))?;
    }
    Ok(())
}
