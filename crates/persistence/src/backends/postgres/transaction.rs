//! Transaction support for PostgreSQL backend.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use deadpool_postgres::Client;
use helios_fhir::FhirVersion;
use serde_json::Value;

use crate::core::{Transaction, TransactionOptions, TransactionProvider};
use crate::error::{
    BackendError, ConcurrencyError, ResourceError, StorageError, StorageResult, TransactionError,
};
use crate::search::SearchParameterExtractor;
use crate::tenant::{Operation, TenantContext};
use crate::types::StoredResource;

use super::PostgresBackend;
use super::cached::{execute_cached, query_cached, query_opt_cached};
use super::search::writer::{IndexRow, PostgresSearchIndexWriter};

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: None,
    })
}

#[allow(dead_code)]
fn serialization_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::SerializationError { message })
}

/// A PostgreSQL transaction.
///
/// Wraps a deadpool_postgres Client that has an active transaction.
/// The transaction is automatically rolled back on drop if not committed.
pub struct PostgresTransaction {
    /// The client with active transaction.
    /// Option so we can take it during commit/rollback.
    client: Option<Client>,
    /// Whether the transaction is still active.
    active: bool,
    /// The tenant context for this transaction.
    tenant: TenantContext,
    /// Search parameter extractor for indexing resources.
    search_extractor: Arc<SearchParameterExtractor>,
    /// When true, search indexing is offloaded to a secondary backend.
    search_offloaded: bool,
    /// The `search_index` layout, so writes in this transaction take the same
    /// composite shape the read path will look for.
    index_layout: super::schema::IndexLayout,
    /// The FHIR version writes in this transaction are stamped with.
    fhir_version: FhirVersion,
    /// Creates whose rows have been built but not yet sent.
    ///
    /// See [`PostgresTransaction::flush`] for why they are held back.
    pending: Vec<PendingCreate>,
    /// Index rows held in `pending`, to bound a flush by rows as well as by
    /// resources — one `Provenance` alone contributes 1,626.
    pending_index_rows: usize,
    /// How many times `create` has been called on this transaction. Doubles as
    /// the ordinal stamped on each [`PendingCreate`], so a conflict discovered
    /// at flush time can be traced back to the bundle entry that caused it.
    creates_seen: usize,
    /// Set when a flush discovers that a buffered create conflicted with an
    /// existing row. See [`PostgresTransaction::flush`].
    conflict: Option<DeferredConflict>,
}

/// A create whose `resources`/`resource_history`/`search_index` rows are built
/// and waiting to be sent with its neighbours'.
struct PendingCreate {
    /// Ordinal of the `create` call that produced this row.
    ordinal: usize,
    resource_type: String,
    id: String,
    version_id: String,
    data: Value,
    last_updated: DateTime<Utc>,
    fhir_version: String,
    /// Empty when search indexing is offloaded to a secondary backend.
    index_rows: Vec<IndexRow>,
}

/// A create that turned out to conflict, reported after the fact.
///
/// `ordinal` indexes the transaction's sequence of `create` calls; the caller
/// that drives a bundle knows which entry each of those came from and maps it
/// back (see `PostgresBackend::process_transaction`).
#[derive(Debug, Clone)]
pub(crate) struct DeferredConflict {
    pub ordinal: usize,
    pub resource_type: String,
    pub id: String,
}

/// Resources buffered before a flush is forced.
///
/// The benchmark's import bundles carry ~1,632 resources each, so this is the
/// knob that turns 3,264 statements per bundle into 26. It is a memory bound,
/// not a correctness one: 20 concurrent importers hold at most
/// 20 x 128 resources of JSON and index rows at a time.
const MAX_PENDING_RESOURCES: usize = 128;

/// Index rows buffered before a flush is forced.
///
/// At the import corpus's 24.2 rows per resource, [`MAX_PENDING_RESOURCES`]
/// binds first; this bounds the case where it does not.
const MAX_PENDING_INDEX_ROWS: usize = 4096;

impl std::fmt::Debug for PostgresTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresTransaction")
            .field("active", &self.active)
            .field("tenant", &self.tenant)
            .finish()
    }
}

impl PostgresTransaction {
    /// Create a new transaction.
    async fn new(
        client: Client,
        tenant: TenantContext,
        search_extractor: Arc<SearchParameterExtractor>,
        search_offloaded: bool,
        fhir_version: FhirVersion,
        index_layout: super::schema::IndexLayout,
    ) -> StorageResult<Self> {
        // Start the transaction
        client.execute("BEGIN", &[]).await.map_err(|e| {
            StorageError::Transaction(TransactionError::RolledBack {
                reason: format!("Failed to begin transaction: {}", e),
            })
        })?;

        Ok(Self {
            client: Some(client),
            active: true,
            tenant,
            search_extractor,
            search_offloaded,
            fhir_version,
            index_layout,
            pending: Vec::new(),
            pending_index_rows: 0,
            creates_seen: 0,
            conflict: None,
        })
    }

    fn client(&self) -> StorageResult<&Client> {
        self.client
            .as_ref()
            .ok_or_else(|| StorageError::Transaction(TransactionError::InvalidTransaction))
    }

    /// Index a resource for search within the transaction.
    async fn index_resource(
        &self,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        last_updated: DateTime<Utc>,
        mode: super::storage::IndexWrite,
        resource: &Value,
    ) -> StorageResult<()> {
        if self.search_offloaded {
            return Ok(());
        }

        let client = self.client()?;

        if mode == super::storage::IndexWrite::Replace {
            execute_cached(
                    client,
                    "DELETE FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3",
                    &[&tenant_id, &resource_type, &resource_id],
                )
                .await
                .map_err(|e| internal_error(format!("Failed to clear search index: {}", e)))?;
        }

        // Extract values using the registry-driven extractor
        let values = self
            .search_extractor
            .extract(resource, resource_type)
            .map_err(|e| internal_error(format!("Search parameter extraction failed: {}", e)))?;

        // Write the extracted values, folding composites into the denormalized
        // one-row-per-instance layout (#279).
        PostgresSearchIndexWriter::write_values(
            client,
            tenant_id,
            resource_type,
            resource_id,
            last_updated,
            self.index_layout,
            values,
        )
        .await?;

        tracing::debug!(
            "Indexed resource {}/{} within transaction",
            resource_type,
            resource_id
        );

        Ok(())
    }
}

/// The batched form of the per-resource `WITH ins AS (INSERT INTO resources …)`
/// statement.
///
/// Row for row it does exactly what the single-resource statement did — insert
/// into `resources` with `ON CONFLICT (tenant_id, resource_type, id) DO
/// NOTHING`, then copy whatever that inserted into `resource_history` — and it
/// does it in array order, so the rows are inserted, and therefore locked, in
/// the same order as before.
///
/// Three details are load-bearing:
///
/// * `hist` is a data-modifying CTE that the outer query never selects from.
///   Postgres executes those "exactly once, and always to completion,
///   independently of whether the primary query reads any of their output", so
///   the history rows are written regardless.
/// * The outer `SELECT` returns the key of every row that was actually
///   inserted. A key that is absent is a key that conflicted, which is how a
///   per-entry 409 survives batching.
/// * `ON CONFLICT DO NOTHING` — not `DO UPDATE` — also makes duplicates *within
///   one batch* well defined: the second speculative insert of a key sees the
///   first and is skipped, rather than raising "cannot affect row a second
///   time". Two entries creating the same id therefore still resolve to one
///   success and one conflict.
const INSERT_RESOURCES_SQL: &str = "\
WITH input (resource_type, id, version_id, data, last_updated, fhir_version) AS (
    SELECT * FROM unnest($2::text[], $3::text[], $4::text[], $5::jsonb[], $6::timestamptz[], $7::text[])
), ins AS (
    INSERT INTO resources (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
    SELECT $1::text, resource_type, id, version_id, data, last_updated, FALSE, fhir_version FROM input
    ON CONFLICT (tenant_id, resource_type, id) DO NOTHING
    RETURNING resource_type, id, version_id, data, last_updated, is_deleted, fhir_version
), hist AS (
    INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
    SELECT $1::text, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version FROM ins
)
SELECT resource_type, id FROM ins";

impl PostgresTransaction {
    /// How many `create` calls this transaction has taken.
    ///
    /// The bundle driver samples this around each entry so it can map a
    /// [`DeferredConflict`]'s ordinal back to an entry index.
    pub(crate) fn creates_seen(&self) -> usize {
        self.creates_seen
    }

    /// The conflict a flush discovered, if any.
    pub(crate) fn deferred_conflict(&self) -> Option<&DeferredConflict> {
        self.conflict.as_ref()
    }

    /// Sends every buffered create.
    ///
    /// # Why buffer at all
    ///
    /// A transaction bundle is the import path, and run 33029355759's import
    /// wrote 1,630,685 resources through 1,630,685 executions of the
    /// single-resource insert (1,018 s of Postgres execution) and ~1.6M
    /// executions of the index insert. Each carried one bind, one executor
    /// start/stop and one round trip for a payload of one resource. Buffering
    /// [`MAX_PENDING_RESOURCES`] of them turns a 1,632-entry bundle's 3,264
    /// statements into 26 — the same rows, the same order, ~99.2% fewer
    /// statements.
    ///
    /// # Why a conflict can be reported late
    ///
    /// `create` used to learn on the spot that its row conflicted, because it
    /// sent the insert itself. Buffered, it cannot: the conflict is discovered
    /// when the batch lands. The two are equivalent for the only caller that
    /// exists — a transaction bundle is all-or-nothing, so an entry that
    /// conflicts rolls the whole bundle back either way, and the entry index is
    /// preserved through [`DeferredConflict::ordinal`].
    ///
    /// They are *not* equivalent for a hypothetical caller that catches the
    /// error and commits anyway: entries the unbatched path would never have
    /// attempted are already inserted by the time the conflict is seen. So a
    /// conflict poisons the transaction — [`Transaction::commit`] refuses and
    /// rolls back instead. That is a deliberate loss of a capability nothing
    /// uses, rather than a silent divergence.
    pub(crate) async fn flush(&mut self) -> StorageResult<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let pending = std::mem::take(&mut self.pending);
        self.pending_index_rows = 0;

        let conflict = {
            let client = self.client()?;
            let tenant_id = self.tenant.tenant_id().as_str();
            flush_pending(client, tenant_id, &pending).await?
        };

        if let Some(conflict) = conflict {
            let message = format!("{}/{} already exists", conflict.resource_type, conflict.id);
            let resource_type = conflict.resource_type.clone();
            let id = conflict.id.clone();
            self.conflict = Some(conflict);
            tracing::debug!("transaction flush found a conflicting create: {}", message);
            return Err(StorageError::Resource(ResourceError::AlreadyExists {
                resource_type,
                id,
            }));
        }

        Ok(())
    }

    /// Refuses to keep using a transaction that has already lost an entry.
    fn ensure_usable(&self) -> StorageResult<()> {
        if !self.active || self.conflict.is_some() {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }
        Ok(())
    }
}

/// Writes one batch of buffered creates.
///
/// Returns the first create (in `create` order) whose row was not inserted
/// because its key was already taken. On that path the index rows are
/// deliberately *not* written: the caller poisons the transaction, so the only
/// way out is a rollback, and writing 3,000 index rows that are about to be
/// discarded would be pure cost.
async fn flush_pending(
    client: &deadpool_postgres::Client,
    tenant_id: &str,
    pending: &[PendingCreate],
) -> StorageResult<Option<DeferredConflict>> {
    let resource_types: Vec<&str> = pending.iter().map(|p| p.resource_type.as_str()).collect();
    let ids: Vec<&str> = pending.iter().map(|p| p.id.as_str()).collect();
    let version_ids: Vec<&str> = pending.iter().map(|p| p.version_id.as_str()).collect();
    let data: Vec<&Value> = pending.iter().map(|p| &p.data).collect();
    let last_updated: Vec<DateTime<Utc>> = pending.iter().map(|p| p.last_updated).collect();
    let fhir_versions: Vec<&str> = pending.iter().map(|p| p.fhir_version.as_str()).collect();

    let inserted = query_cached(
        client,
        INSERT_RESOURCES_SQL,
        &[
            &tenant_id,
            &resource_types,
            &ids,
            &version_ids,
            &data,
            &last_updated,
            &fhir_versions,
        ],
    )
    .await
    .map_err(|e| internal_error(format!("Failed to insert resources: {}", e)))?;

    let mut inserted_keys: HashSet<(String, String)> = HashSet::with_capacity(inserted.len());
    for row in &inserted {
        inserted_keys.insert((row.get::<_, String>(0), row.get::<_, String>(1)));
    }

    let attempted: Vec<(&str, &str)> = pending
        .iter()
        .map(|p| (p.resource_type.as_str(), p.id.as_str()))
        .collect();
    if let Some(position) = first_unaccounted_key(&attempted, &mut inserted_keys) {
        let entry = &pending[position];
        return Ok(Some(DeferredConflict {
            ordinal: entry.ordinal,
            resource_type: entry.resource_type.clone(),
            id: entry.id.clone(),
        }));
    }

    let batches: Vec<(&str, &str, &[IndexRow])> = pending
        .iter()
        .filter(|p| !p.index_rows.is_empty())
        .map(|p| {
            (
                p.resource_type.as_str(),
                p.id.as_str(),
                p.index_rows.as_slice(),
            )
        })
        .collect();
    PostgresSearchIndexWriter::insert_rows_multi(client, tenant_id, &batches).await?;

    Ok(None)
}

/// Position of the first attempted key that `inserted` cannot account for.
///
/// `inserted` is the set of keys the statement reported as inserted, and it is
/// *consumed* as it matches. A primary key can come back at most once, so when
/// the same key was attempted twice in one batch the first attempt takes the
/// single match and the second is reported as the conflict — which is exactly
/// what the unbatched path did, where the first `create` inserted the row and
/// the second saw `ON CONFLICT DO NOTHING` skip it.
fn first_unaccounted_key(
    attempted: &[(&str, &str)],
    inserted: &mut HashSet<(String, String)>,
) -> Option<usize> {
    attempted.iter().position(|(resource_type, id)| {
        !inserted.remove(&((*resource_type).to_string(), (*id).to_string()))
    })
}

#[async_trait]
impl Transaction for PostgresTransaction {
    async fn create(
        &mut self,
        resource_type: &str,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        self.tenant
            .check_permission(Operation::Create, resource_type)?;

        self.ensure_usable()?;

        // Get or generate ID
        let id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // Build the resource with id and resourceType
        let mut data = resource.clone();
        if let Some(obj) = data.as_object_mut() {
            obj.insert("id".to_string(), Value::String(id.clone()));
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
        }

        let now = Utc::now();
        let version_id = "1";
        let fhir_version_str = self.fhir_version.as_mime_param();

        // The index rows are built here rather than at flush time so that an
        // extraction failure is still raised by the `create` call that caused
        // it — it is the one per-entry error on this path that is not a
        // conflict, and it must keep naming its own entry.
        let index_rows = if self.search_offloaded {
            Vec::new()
        } else {
            let values = self
                .search_extractor
                .extract(&data, resource_type)
                .map_err(|e| {
                    internal_error(format!("Search parameter extraction failed: {}", e))
                })?;
            PostgresSearchIndexWriter::build_rows(
                resource_type,
                &id,
                now,
                self.index_layout,
                values,
            )
        };

        let ordinal = self.creates_seen;
        self.creates_seen += 1;
        self.pending_index_rows += index_rows.len();
        self.pending.push(PendingCreate {
            ordinal,
            resource_type: resource_type.to_string(),
            id: id.clone(),
            version_id: version_id.to_string(),
            data: data.clone(),
            last_updated: now,
            fhir_version: fhir_version_str.to_string(),
            index_rows,
        });

        if self.pending.len() >= MAX_PENDING_RESOURCES
            || self.pending_index_rows >= MAX_PENDING_INDEX_ROWS
        {
            self.flush().await?;
        }

        Ok(StoredResource::from_storage(
            resource_type,
            &id,
            version_id,
            self.tenant.tenant_id().clone(),
            data,
            now,
            now,
            None,
            self.fhir_version,
        ))
    }

    async fn read(
        &mut self,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        self.ensure_usable()?;

        // Buffered creates must be visible to everything that is not another
        // create. This is what keeps `read`/`update`/`delete` — and therefore a
        // bundle's PUT and DELETE entries, conditional or not — seeing exactly
        // what they saw before: the batch is a wire-level optimisation, never a
        // change of visibility inside the transaction.
        self.flush().await?;

        let client = self.client()?;
        let tenant_id = self.tenant.tenant_id().as_str();

        let row = query_opt_cached(
            client,
            "SELECT version_id, data, last_updated, is_deleted, fhir_version
                 FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
            &[&tenant_id, &resource_type, &id],
        )
        .await
        .map_err(|e| internal_error(format!("Failed to read resource: {}", e)))?;

        match row {
            Some(row) => {
                let version_id: String = row.get(0);
                let data: serde_json::Value = row.get(1);
                let last_updated: chrono::DateTime<Utc> = row.get(2);
                let is_deleted: bool = row.get(3);
                let fhir_version_str: String = row.get(4);

                if is_deleted {
                    return Ok(None);
                }

                let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                    .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

                Ok(Some(StoredResource::from_storage(
                    resource_type,
                    id,
                    version_id,
                    self.tenant.tenant_id().clone(),
                    data,
                    last_updated,
                    last_updated,
                    None,
                    fhir_version,
                )))
            }
            None => Ok(None),
        }
    }

    async fn update(
        &mut self,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        self.tenant
            .check_permission(Operation::Update, current.resource_type())?;

        self.ensure_usable()?;

        // Buffered creates must be visible to everything that is not another
        // create. This is what keeps `read`/`update`/`delete` — and therefore a
        // bundle's PUT and DELETE entries, conditional or not — seeing exactly
        // what they saw before: the batch is a wire-level optimisation, never a
        // change of visibility inside the transaction.
        self.flush().await?;

        let client = self.client()?;
        let tenant_id = self.tenant.tenant_id().as_str();
        let resource_type = current.resource_type();
        let id = current.id();

        // Verify current version still matches (optimistic locking)
        let row = query_opt_cached(
            client,
            "SELECT version_id FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND is_deleted = FALSE",
            &[&tenant_id, &resource_type, &id],
        )
        .await
        .map_err(|e| internal_error(format!("Failed to get current version: {}", e)))?;

        let db_version = match row {
            Some(row) => row.get::<_, String>(0),
            None => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
        };

        if db_version != current.version_id() {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: current.version_id().to_string(),
                    actual_version: db_version,
                },
            ));
        }

        // Calculate new version
        let new_version: u64 = db_version.parse().unwrap_or(0) + 1;
        let new_version_str = new_version.to_string();

        // Build the resource with id and resourceType
        let mut data = resource.clone();
        if let Some(obj) = data.as_object_mut() {
            obj.insert("id".to_string(), Value::String(id.to_string()));
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
        }

        let now = Utc::now();
        let fhir_version = current.fhir_version();
        let fhir_version_str = fhir_version.as_mime_param();
        let is_deleted = false;

        // Update the resource
        execute_cached(
            client,
            "UPDATE resources SET version_id = $1, data = $2, last_updated = $3
                 WHERE tenant_id = $4 AND resource_type = $5 AND id = $6",
            &[
                &new_version_str,
                &data,
                &now,
                &tenant_id,
                &resource_type,
                &id,
            ],
        )
        .await
        .map_err(|e| internal_error(format!("Failed to update resource: {}", e)))?;

        // Insert into history
        execute_cached(
                client,
                "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&tenant_id, &resource_type, &id, &new_version_str, &data, &now, &is_deleted, &fhir_version_str],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to insert history: {}", e)))?;

        // Re-index the resource for search
        self.index_resource(
            tenant_id,
            resource_type,
            id,
            now,
            super::storage::IndexWrite::Replace,
            &data,
        )
        .await?;

        Ok(StoredResource::from_storage(
            resource_type,
            id,
            new_version_str,
            self.tenant.tenant_id().clone(),
            data,
            now,
            now,
            None,
            fhir_version,
        ))
    }

    async fn delete(&mut self, resource_type: &str, id: &str) -> StorageResult<()> {
        self.tenant
            .check_permission(Operation::Delete, resource_type)?;

        self.ensure_usable()?;

        // Buffered creates must be visible to everything that is not another
        // create. This is what keeps `read`/`update`/`delete` — and therefore a
        // bundle's PUT and DELETE entries, conditional or not — seeing exactly
        // what they saw before: the batch is a wire-level optimisation, never a
        // change of visibility inside the transaction.
        self.flush().await?;

        let client = self.client()?;
        let tenant_id = self.tenant.tenant_id().as_str();

        // Check if resource exists
        let row = query_opt_cached(
            client,
            "SELECT version_id, data, fhir_version FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND is_deleted = FALSE",
            &[&tenant_id, &resource_type, &id],
        )
        .await
        .map_err(|e| internal_error(format!("Failed to check resource: {}", e)))?;

        let (current_version, data, fhir_version_str) = match row {
            Some(row) => {
                let v: String = row.get(0);
                let d: serde_json::Value = row.get(1);
                let f: String = row.get(2);
                (v, d, f)
            }
            None => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
        };

        let now = Utc::now();
        let new_version: u64 = current_version.parse().unwrap_or(0) + 1;
        let new_version_str = new_version.to_string();
        let is_deleted = true;

        // Soft delete the resource
        execute_cached(
                client,
                "UPDATE resources SET is_deleted = TRUE, deleted_at = $1, version_id = $2, last_updated = $1
                 WHERE tenant_id = $3 AND resource_type = $4 AND id = $5",
                &[&now, &new_version_str, &tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to delete resource: {}", e)))?;

        // Insert deletion record into history
        execute_cached(
                client,
                "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&tenant_id, &resource_type, &id, &new_version_str, &data, &now, &is_deleted, &fhir_version_str],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to insert deletion history: {}", e)))?;

        Ok(())
    }

    async fn commit(mut self: Box<Self>) -> StorageResult<()> {
        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }

        // A transaction that lost an entry to a conflict cannot be committed:
        // see `flush`. Roll it back here rather than leaving it to `Drop`, so
        // the caller gets a definite answer.
        if self.conflict.is_some() {
            if let Some(client) = self.client.as_ref() {
                let _ = client.execute("ROLLBACK", &[]).await;
            }
            self.active = false;
            self.pending.clear();
            return Err(StorageError::Transaction(TransactionError::RolledBack {
                reason: "a bundle entry conflicted with an existing resource".to_string(),
            }));
        }

        // Anything still buffered is part of this commit. A failure here leaves
        // `active` set, so `Drop` still rolls the transaction back.
        self.flush().await?;

        if let Some(client) = self.client.as_ref() {
            client.execute("COMMIT", &[]).await.map_err(|e| {
                StorageError::Transaction(TransactionError::RolledBack {
                    reason: format!("Commit failed: {}", e),
                })
            })?;
        }

        self.active = false;
        Ok(())
    }

    async fn rollback(mut self: Box<Self>) -> StorageResult<()> {
        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }

        // Buffered creates were never sent, so a rollback simply drops them.
        self.pending.clear();
        self.pending_index_rows = 0;

        if let Some(client) = self.client.as_ref() {
            client.execute("ROLLBACK", &[]).await.map_err(|e| {
                StorageError::Transaction(TransactionError::RolledBack {
                    reason: format!("Rollback failed: {}", e),
                })
            })?;
        }

        self.active = false;
        Ok(())
    }

    fn tenant(&self) -> &TenantContext {
        &self.tenant
    }

    fn is_active(&self) -> bool {
        self.active
    }
}

impl Drop for PostgresTransaction {
    fn drop(&mut self) {
        // If the transaction wasn't explicitly committed or rolled back, we must
        // still ROLLBACK before the connection is returned to the pool. deadpool's
        // default recycling does NOT reset session state, so a connection handed
        // back with an open transaction poisons the pool: the next checkout fails
        // with "there is already a transaction in progress", cascading across every
        // subsequent request that reuses it. (The previous code assumed recycling
        // auto-rolls-back, which is false — this was the cause of the import-suite
        // failure cascade under concurrent load.)
        //
        // Drop can't be async, so move the client into a spawned task that issues
        // the ROLLBACK and only then drops it — the connection returns to the pool
        // clean, and only after the rollback completes.
        if !self.active {
            return;
        }
        self.active = false;
        self.pending.clear();
        let Some(client) = self.client.take() else {
            return;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                tracing::warn!(
                    "PostgreSQL transaction dropped without explicit commit/rollback; rolling back before pool return"
                );
                handle.spawn(async move {
                    // Ignore the result: the connection drops (returns to the pool)
                    // when this task ends regardless, and a failed rollback here
                    // just means a broken connection deadpool will discard anyway.
                    let _ = client.execute("ROLLBACK", &[]).await;
                });
            }
            Err(_) => {
                // No async runtime (e.g. a synchronous test drop): can't roll back.
                tracing::warn!(
                    "PostgreSQL transaction dropped without explicit commit/rollback and no runtime available to roll back; connection may re-enter the pool with an open transaction"
                );
            }
        }
    }
}

#[async_trait]
impl TransactionProvider for PostgresBackend {
    type Transaction = PostgresTransaction;

    async fn begin_transaction(
        &self,
        tenant: &TenantContext,
        options: TransactionOptions,
    ) -> StorageResult<Self::Transaction> {
        let client = self.get_client().await?;
        PostgresTransaction::new(
            client,
            tenant.clone(),
            std::sync::Arc::new(self.tenant_extractor(tenant.tenant_id().as_str())),
            self.is_search_offloaded(),
            options.fhir_version.unwrap_or(self.config().fhir_version),
            self.index_layout(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_set(keys: &[(&str, &str)]) -> HashSet<(String, String)> {
        keys.iter()
            .map(|(t, i)| ((*t).to_string(), (*i).to_string()))
            .collect()
    }

    #[test]
    fn every_key_inserted_is_no_conflict() {
        let attempted = [("Patient", "a"), ("Observation", "b")];
        let mut inserted = key_set(&attempted);
        assert_eq!(first_unaccounted_key(&attempted, &mut inserted), None);
    }

    /// The batch's whole point: one bad entry must still be identifiable, not
    /// collapse the batch into an anonymous failure.
    #[test]
    fn a_key_the_statement_skipped_is_named_by_position() {
        let attempted = [("Patient", "a"), ("Patient", "taken"), ("Patient", "c")];
        let mut inserted = key_set(&[("Patient", "a"), ("Patient", "c")]);
        assert_eq!(first_unaccounted_key(&attempted, &mut inserted), Some(1));
    }

    /// Two entries creating the same id: `ON CONFLICT DO NOTHING` inserts the
    /// first and skips the second, and the key comes back once. Attributing the
    /// conflict to the *second* attempt is what the unbatched path did.
    #[test]
    fn a_duplicate_within_one_batch_is_attributed_to_the_later_attempt() {
        let attempted = [("Patient", "dup"), ("Patient", "other"), ("Patient", "dup")];
        let mut inserted = key_set(&[("Patient", "dup"), ("Patient", "other")]);
        assert_eq!(first_unaccounted_key(&attempted, &mut inserted), Some(2));
    }

    /// A key that exists in another resource type is a different key.
    #[test]
    fn the_key_includes_the_resource_type() {
        let attempted = [("Patient", "x")];
        let mut inserted = key_set(&[("Observation", "x")]);
        assert_eq!(first_unaccounted_key(&attempted, &mut inserted), Some(0));
    }
}
