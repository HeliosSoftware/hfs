//! Index-free `search_index` rebuild for PostgreSQL (`ReindexRequest::bulk_index_rebuild`,
//! `HFS_BULK_SUBMIT_BULK_INDEX_REBUILD`).
//!
//! A run that asks for it drops the `search_index` value indexes before its
//! pages and builds them once, sorted, when it ends, instead of maintaining
//! ~20 b-trees row by row. What stays:
//!
//! * every unique / primary-key / constraint index (`ON CONFLICT` targets);
//! * `idx_search_resource` (tenant_id, resource_type, resource_id), which
//!   the writer's per-resource `DELETE` seeks — without it every reindex
//!   group would scan the table.
//!
//! # Bookkeeping and crash safety
//!
//! The definition of every dropped index (`pg_get_indexdef`) is recorded in
//! `search_index_deferred_indexes` in the same transaction as the `DROP`, so
//! the set to rebuild never depends on this binary's idea of the schema, and
//! a process that dies inside the window leaves a durable to-do list:
//! [`PostgresBackend::heal_deferred_search_indexes`] rebuilds it at the next
//! startup, and the next run's end rebuilds it too. Each index is rebuilt
//! with `CREATE INDEX IF NOT EXISTS` and its row deleted in one transaction,
//! so a rebuild that fails part-way leaves exactly the unfinished rows.
//!
//! # Concurrent runs, in this process and others
//!
//! Runs are counted with a session-level advisory lock held for the run on a
//! connection taken out of the pool (so the lock dies with the process, not
//! with a pooled connection's next borrower):
//!
//! * begin: `pg_try_advisory_lock` (exclusive). Granted means no other run
//!   holds the lock anywhere: drop the indexes, then downgrade to the shared
//!   lock. Not granted means another run is active (it dropped them) or is
//!   building them: wait for the shared lock and run with whatever exists.
//! * end: release the shared lock, then `pg_try_advisory_lock` again.
//!   Granted means this was the last run: build the recorded indexes while
//!   holding it (a run that begins meanwhile waits in its shared request and
//!   then runs with the indexes present). Not granted: another run is still
//!   active and its end builds them.
//!
//! Inside one process the runs share one session through a reference count,
//! as SQLite's `BULK_INDEX_REBUILDS` does.
//!
//! While the window is open, search on this database is unindexed for every
//! tenant and type (correct, slower), which is why the mode is opt-in.

use std::sync::Arc;

use futures::StreamExt;
use tokio_postgres::GenericClient;

use crate::error::{BackendError, StorageError, StorageResult};

use super::PostgresBackend;

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: None,
    })
}

/// Advisory-lock namespace of the bulk index rebuild session lock ("HFSI").
const BULK_REBUILD_LOCK_NAMESPACE: i32 = 0x4846_5349;
/// The one key in [`BULK_REBUILD_LOCK_NAMESPACE`]: the whole `search_index`.
const BULK_REBUILD_LOCK_KEY: i32 = 1;

/// The index the reindex writer's per-resource `DELETE` needs; never dropped.
const KEPT_LOOKUP_INDEX: &str = "idx_search_resource";

const DEFERRED_TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS search_index_deferred_indexes (
    index_name TEXT PRIMARY KEY,
    definition TEXT NOT NULL,
    dropped_at TIMESTAMPTZ NOT NULL DEFAULT now()
)";

/// Droppable indexes of `search_index`: not unique, not primary, not backing
/// a constraint, not the per-resource lookup index.
const DROPPABLE_INDEXES_SQL: &str = "SELECT c.relname::text, pg_get_indexdef(c.oid)
    FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid
    WHERE i.indrelid = to_regclass('search_index')
      AND NOT i.indisunique AND NOT i.indisprimary
      AND c.relname <> $1
      AND NOT EXISTS (SELECT 1 FROM pg_constraint k WHERE k.conindid = c.oid)
    ORDER BY c.relname";

/// Default `maintenance_work_mem` for each sorted index build
/// (`HFS_PG_BULK_INDEX_MAINTENANCE_WORK_MEM`).
pub(crate) const DEFAULT_BUILD_MAINTENANCE_WORK_MEM: &str = "256MB";
/// Default number of index builds run at once
/// (`HFS_PG_BULK_INDEX_BUILD_CONCURRENCY`).
pub(crate) const DEFAULT_BUILD_CONCURRENCY: usize = 4;

/// Per-backend (shared by clones) state of the bulk index rebuild mode.
#[derive(Default)]
pub(crate) struct BulkIndexRebuildState {
    /// Runs of this process currently inside the window.
    active: usize,
    /// The connection holding this process's shared session lock while
    /// `active > 0`. Out of the pool: dropping it closes the connection and
    /// with it every session lock.
    session: Option<deadpool_postgres::ClientWrapper>,
}

pub(crate) type BulkIndexRebuild = Arc<tokio::sync::Mutex<BulkIndexRebuildState>>;

/// `maintenance_work_mem` for the builds: `HFS_PG_BULK_INDEX_MAINTENANCE_WORK_MEM`
/// when it is a plain size (`512MB`, `2GB`, `65536kB`, digits only = kB), the
/// default otherwise. Validated because it is spliced into `SET LOCAL`.
pub(crate) fn build_maintenance_work_mem(raw: Option<&str>) -> String {
    let valid = |value: &str| {
        let digits = value.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        let unit = &value[digits.len()..];
        !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && matches!(unit, "" | "kB" | "MB" | "GB")
    };
    match raw.map(str::trim) {
        Some(value) if valid(value) => value.to_string(),
        _ => DEFAULT_BUILD_MAINTENANCE_WORK_MEM.to_string(),
    }
}

/// Index builds run at once: `HFS_PG_BULK_INDEX_BUILD_CONCURRENCY` (≥ 1).
pub(crate) fn build_concurrency(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value >= 1)
        .unwrap_or(DEFAULT_BUILD_CONCURRENCY)
}

/// `CREATE INDEX name ON …` → `CREATE INDEX IF NOT EXISTS name ON …`, so a
/// rebuild that races another (or repeats after a partial failure) is a no-op
/// for the indexes that already exist.
pub(crate) fn idempotent_index_definition(definition: &str) -> String {
    match definition.strip_prefix("CREATE INDEX ") {
        Some(rest) if !rest.starts_with("IF NOT EXISTS ") => {
            format!("CREATE INDEX IF NOT EXISTS {rest}")
        }
        _ => definition.to_string(),
    }
}

fn lock_sql(function: &str) -> String {
    format!("SELECT {function}({BULK_REBUILD_LOCK_NAMESPACE}, {BULK_REBUILD_LOCK_KEY})")
}

impl PostgresBackend {
    /// A connection out of the pool for a session lock, with no statement
    /// timeout (the shared request may wait for another process's build).
    async fn bulk_rebuild_session(&self) -> StorageResult<deadpool_postgres::ClientWrapper> {
        let session = deadpool_postgres::Object::take(self.get_client().await?);
        session
            .batch_execute("SET statement_timeout = 0")
            .await
            .map_err(|e| internal_error(format!("bulk index rebuild session: {e}")))?;
        Ok(session)
    }

    async fn try_exclusive_bulk_rebuild_lock<C>(session: &C) -> StorageResult<bool>
    where
        C: GenericClient + Sync,
    {
        Ok(session
            .query_one(&lock_sql("pg_try_advisory_lock"), &[])
            .await
            .map_err(|e| internal_error(format!("bulk index rebuild lock: {e}")))?
            .get(0))
    }

    /// Records and drops the droppable `search_index` indexes in one
    /// transaction; returns how many were dropped.
    async fn drop_search_value_indexes(
        session: &mut deadpool_postgres::ClientWrapper,
    ) -> StorageResult<usize> {
        let transaction = session
            .transaction()
            .await
            .map_err(|e| internal_error(format!("bulk index rebuild: {e}")))?;
        transaction
            .batch_execute(DEFERRED_TABLE_DDL)
            .await
            .map_err(|e| internal_error(format!("bulk index rebuild bookkeeping: {e}")))?;
        let indexes = transaction
            .query(DROPPABLE_INDEXES_SQL, &[&KEPT_LOOKUP_INDEX])
            .await
            .map_err(|e| internal_error(format!("bulk index rebuild: list indexes: {e}")))?;
        for row in &indexes {
            let name: String = row.get(0);
            let definition: String = row.get(1);
            transaction
                .execute(
                    "INSERT INTO search_index_deferred_indexes (index_name, definition)
                     VALUES ($1, $2) ON CONFLICT (index_name) DO NOTHING",
                    &[&name, &definition],
                )
                .await
                .map_err(|e| internal_error(format!("bulk index rebuild bookkeeping: {e}")))?;
            // The name comes from pg_class; quote it as an identifier.
            transaction
                .batch_execute(&format!(
                    "DROP INDEX IF EXISTS \"{}\"",
                    name.replace('"', "\"\"")
                ))
                .await
                .map_err(|e| internal_error(format!("bulk index rebuild: drop {name}: {e}")))?;
        }
        transaction
            .commit()
            .await
            .map_err(|e| internal_error(format!("bulk index rebuild: commit drop: {e}")))?;
        Ok(indexes.len())
    }

    /// Builds every index recorded in `search_index_deferred_indexes`, up to
    /// `HFS_PG_BULK_INDEX_BUILD_CONCURRENCY` at once, each in a transaction
    /// that also deletes its bookkeeping row. Returns how many were built.
    /// The caller holds the exclusive bulk-rebuild lock.
    pub(crate) async fn rebuild_deferred_search_indexes(&self) -> StorageResult<usize> {
        let client = self.get_client().await?;
        let exists: bool = client
            .query_one(
                "SELECT to_regclass('search_index_deferred_indexes') IS NOT NULL",
                &[],
            )
            .await
            .map_err(|e| internal_error(format!("bulk index rebuild: {e}")))?
            .get(0);
        if !exists {
            return Ok(0);
        }
        let pending: Vec<(String, String)> = client
            .query(
                "SELECT index_name, definition FROM search_index_deferred_indexes
                 ORDER BY index_name",
                &[],
            )
            .await
            .map_err(|e| internal_error(format!("bulk index rebuild: {e}")))?
            .into_iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect();
        drop(client);
        if pending.is_empty() {
            return Ok(0);
        }
        let work_mem = build_maintenance_work_mem(
            std::env::var("HFS_PG_BULK_INDEX_MAINTENANCE_WORK_MEM")
                .ok()
                .as_deref(),
        );
        let concurrency = build_concurrency(
            std::env::var("HFS_PG_BULK_INDEX_BUILD_CONCURRENCY")
                .ok()
                .as_deref(),
        );
        let total = pending.len();
        let results: Vec<StorageResult<()>> = futures::stream::iter(pending)
            .map(|(name, definition)| {
                let work_mem = work_mem.clone();
                async move {
                    let started = std::time::Instant::now();
                    let mut client = self.get_client().await?;
                    let transaction = client
                        .transaction()
                        .await
                        .map_err(|e| internal_error(format!("build {name}: {e}")))?;
                    transaction
                        .batch_execute(&format!(
                            "SET LOCAL statement_timeout = 0; \
                             SET LOCAL maintenance_work_mem = '{work_mem}'; {}",
                            idempotent_index_definition(&definition)
                        ))
                        .await
                        .map_err(|e| internal_error(format!("build {name}: {e}")))?;
                    transaction
                        .execute(
                            "DELETE FROM search_index_deferred_indexes WHERE index_name = $1",
                            &[&name],
                        )
                        .await
                        .map_err(|e| internal_error(format!("build {name}: {e}")))?;
                    transaction
                        .commit()
                        .await
                        .map_err(|e| internal_error(format!("build {name}: {e}")))?;
                    tracing::debug!(
                        index = %name,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "bulk index rebuild: index built"
                    );
                    Ok(())
                }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await;
        let mut built = 0;
        let mut first_error = None;
        for result in results {
            match result {
                Ok(()) => built += 1,
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        match first_error {
            Some(error) => {
                tracing::warn!(
                    built,
                    total,
                    "bulk index rebuild: some search_index indexes were not rebuilt; \
                     they stay recorded and are rebuilt at the next run end or startup: {error}"
                );
                Err(error)
            }
            None => Ok(built),
        }
    }

    /// Startup self-heal: rebuilds the indexes a bulk rebuild that never
    /// reached its end (a crashed process) left recorded — unless a run in
    /// another process is inside its window right now, whose end will.
    pub(crate) async fn heal_deferred_search_indexes(&self) -> StorageResult<usize> {
        let session = self.bulk_rebuild_session().await?;
        if !Self::try_exclusive_bulk_rebuild_lock(&*session).await? {
            return Ok(0);
        }
        let started = std::time::Instant::now();
        let result = self.rebuild_deferred_search_indexes().await;
        if let Ok(built) = &result {
            if *built > 0 {
                tracing::warn!(
                    built,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "bulk index rebuild: rebuilt search_index indexes left dropped by an \
                     interrupted run"
                );
            }
        }
        // Dropping the session closes the connection and releases the lock.
        drop(session);
        result
    }

    /// `ReindexTarget::begin_bulk_index_rebuild` for PostgreSQL.
    pub(crate) async fn begin_bulk_index_rebuild_pg(&self) -> StorageResult<()> {
        if self.is_search_offloaded() {
            return Ok(());
        }
        let mut state = self.bulk_index_rebuild.lock().await;
        if state.active > 0 {
            state.active += 1;
            return Ok(());
        }
        let mut session = self.bulk_rebuild_session().await?;
        if Self::try_exclusive_bulk_rebuild_lock(&*session).await? {
            let started = std::time::Instant::now();
            let dropped = Self::drop_search_value_indexes(&mut session).await?;
            session
                .batch_execute(&format!(
                    "{}; {}",
                    lock_sql("pg_advisory_lock_shared"),
                    lock_sql("pg_advisory_unlock")
                ))
                .await
                .map_err(|e| internal_error(format!("bulk index rebuild lock: {e}")))?;
            tracing::info!(
                dropped,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "bulk index rebuild: search_index value indexes dropped for the run"
            );
        } else {
            session
                .batch_execute(&lock_sql("pg_advisory_lock_shared"))
                .await
                .map_err(|e| internal_error(format!("bulk index rebuild lock: {e}")))?;
            tracing::info!(
                "bulk index rebuild: another run holds the search_index window; joining it"
            );
        }
        state.session = Some(session);
        state.active = 1;
        Ok(())
    }

    /// `ReindexTarget::end_bulk_index_rebuild` for PostgreSQL.
    pub(crate) async fn end_bulk_index_rebuild_pg(&self) -> StorageResult<()> {
        if self.is_search_offloaded() {
            return Ok(());
        }
        let mut state = self.bulk_index_rebuild.lock().await;
        state.active = state.active.saturating_sub(1);
        if state.active > 0 {
            return Ok(());
        }
        let Some(session) = state.session.take() else {
            return Ok(());
        };
        session
            .batch_execute(&lock_sql("pg_advisory_unlock_shared"))
            .await
            .map_err(|e| internal_error(format!("bulk index rebuild unlock: {e}")))?;
        if !Self::try_exclusive_bulk_rebuild_lock(&*session).await? {
            tracing::info!(
                "bulk index rebuild: another run is still inside the window; it rebuilds"
            );
            return Ok(());
        }
        let started = std::time::Instant::now();
        let result = self.rebuild_deferred_search_indexes().await;
        if let Ok(built) = &result {
            tracing::info!(
                built,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "bulk index rebuild: search_index value indexes rebuilt"
            );
        }
        drop(session);
        result.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maintenance_work_mem_accepts_plain_sizes_only() {
        assert_eq!(build_maintenance_work_mem(Some("1GB")), "1GB");
        assert_eq!(build_maintenance_work_mem(Some(" 512MB ")), "512MB");
        assert_eq!(build_maintenance_work_mem(Some("65536")), "65536");
        assert_eq!(build_maintenance_work_mem(Some("65536kB")), "65536kB");
        for bad in ["", "MB", "1 GB", "1TB", "1GB'; DROP TABLE x; --", "-1MB"] {
            assert_eq!(
                build_maintenance_work_mem(Some(bad)),
                DEFAULT_BUILD_MAINTENANCE_WORK_MEM,
                "{bad:?}"
            );
        }
        assert_eq!(
            build_maintenance_work_mem(None),
            DEFAULT_BUILD_MAINTENANCE_WORK_MEM
        );
    }

    #[test]
    fn build_concurrency_defaults_and_floor() {
        assert_eq!(build_concurrency(None), DEFAULT_BUILD_CONCURRENCY);
        assert_eq!(build_concurrency(Some("0")), DEFAULT_BUILD_CONCURRENCY);
        assert_eq!(build_concurrency(Some("x")), DEFAULT_BUILD_CONCURRENCY);
        assert_eq!(build_concurrency(Some("8")), 8);
    }

    #[test]
    fn index_definitions_become_idempotent() {
        assert_eq!(
            idempotent_index_definition(
                "CREATE INDEX idx_search_date ON public.search_index USING btree (tenant_id)"
            ),
            "CREATE INDEX IF NOT EXISTS idx_search_date ON public.search_index USING btree (tenant_id)"
        );
        let already = "CREATE INDEX IF NOT EXISTS x ON search_index (a)";
        assert_eq!(idempotent_index_definition(already), already);
    }
}
