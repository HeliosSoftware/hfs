//! PostgreSQL implementation of the [`SofTerminologyStore`] (#2185).
//!
//! Three tables, created by the v51 migration:
//!
//! - `sof_terminology_relations`: one row per (tenant, canonical) relation.
//! - `sof_terminology_chunks`: the relation's rows, as JSONB arrays of at most
//!   [`CHUNK_ROWS`] rows each, ordered by `seq`. Chunks cascade with their
//!   relation.
//! - `sof_terminology_refs`: one row per (tenant, Library, canonical)
//!   dependency.
//!
//! Unlike SQLite, which keeps a physical table per relation, the rows live in
//! one shared table, so a write needs no DDL. Every write runs in a single
//! transaction under a per-tenant advisory lock, which serializes it against
//! concurrent Library writes and the tenant purge the way SQLite's `IMMEDIATE`
//! transaction does: a release can never sweep a relation another writer is
//! about to reference.

use async_trait::async_trait;
use helios_sof::sqlquery::{TerminologyKind, TerminologyRelation};
use serde_json::Value;

use super::PostgresBackend;
use crate::core::sof_terminology::SofTerminologyStore;
use crate::error::{QueryErrorExt, StorageResult};
use crate::tenant::TenantContext;

/// Rows per `sof_terminology_chunks` row. A relation may hold up to
/// `HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD` rows; one JSONB value for all of
/// them could exceed PostgreSQL's 255 MiB `jsonb` limit.
const CHUNK_ROWS: usize = 1000;

/// First key of the two-part advisory lock serializing a tenant's terminology
/// writes; the second is `hashtext(tenant_id)`. See `USER_SETTINGS_LOCK_NAMESPACE`
/// in `user_settings.rs` for why the namespace is a fixed constant.
const SOF_TERMINOLOGY_LOCK_NAMESPACE: i32 = 0x4853_5454; // "HSTT" (HFS Sof TerminologY Tables)

/// The v50 -> v51 migration body.
pub(super) const CREATE_TABLES_SQL: &str = "
    CREATE TABLE IF NOT EXISTS sof_terminology_relations (
        id BIGSERIAL PRIMARY KEY,
        tenant_id TEXT NOT NULL,
        canonical TEXT NOT NULL,
        kind TEXT NOT NULL,
        row_count BIGINT NOT NULL,
        materialized_at TIMESTAMPTZ NOT NULL,
        UNIQUE (tenant_id, canonical)
    );
    CREATE TABLE IF NOT EXISTS sof_terminology_chunks (
        relation_id BIGINT NOT NULL
            REFERENCES sof_terminology_relations (id) ON DELETE CASCADE,
        seq INTEGER NOT NULL,
        rows JSONB NOT NULL,
        PRIMARY KEY (relation_id, seq)
    );
    CREATE TABLE IF NOT EXISTS sof_terminology_refs (
        tenant_id TEXT NOT NULL,
        library_id TEXT NOT NULL,
        canonical TEXT NOT NULL,
        PRIMARY KEY (tenant_id, library_id, canonical)
    );
    CREATE INDEX IF NOT EXISTS idx_sof_terminology_refs_canonical
        ON sof_terminology_refs (tenant_id, canonical);";

/// Takes the tenant's terminology advisory lock for the rest of `txn`.
async fn lock_tenant(
    txn: &deadpool_postgres::Transaction<'_>,
    tenant_id: &str,
) -> StorageResult<()> {
    txn.execute(
        "SELECT pg_advisory_xact_lock($1, hashtext($2))",
        &[&SOF_TERMINOLOGY_LOCK_NAMESPACE, &tenant_id],
    )
    .await
    .or_query_error("lock terminology tenant")?;
    Ok(())
}

/// Drops every relation of `tenant_id` and its references, inside the
/// caller's transaction (tenant purge).
pub(super) async fn purge_tenant_in_txn(
    txn: &deadpool_postgres::Transaction<'_>,
    tenant_id: &str,
) -> StorageResult<()> {
    lock_tenant(txn, tenant_id).await?;
    for sql in [
        "DELETE FROM sof_terminology_relations WHERE tenant_id = $1",
        "DELETE FROM sof_terminology_refs WHERE tenant_id = $1",
    ] {
        txn.execute(sql, &[&tenant_id])
            .await
            .or_query_error("purge terminology")?;
    }
    Ok(())
}

/// Writes one relation: upserts its registry row and replaces its chunks.
async fn write_relation(
    txn: &deadpool_postgres::Transaction<'_>,
    tenant_id: &str,
    canonical: &str,
    relation: &TerminologyRelation,
) -> StorageResult<()> {
    let row_count = relation.rows.len() as i64;
    let id: i64 = txn
        .query_one(
            "INSERT INTO sof_terminology_relations
                 (tenant_id, canonical, kind, row_count, materialized_at)
             VALUES ($1, $2, $3, $4, now())
             ON CONFLICT (tenant_id, canonical) DO UPDATE SET
                 kind = EXCLUDED.kind,
                 row_count = EXCLUDED.row_count,
                 materialized_at = EXCLUDED.materialized_at
             RETURNING id",
            &[&tenant_id, &canonical, &relation.kind.as_str(), &row_count],
        )
        .await
        .or_query_error("upsert terminology relation")?
        .get(0);
    txn.execute(
        "DELETE FROM sof_terminology_chunks WHERE relation_id = $1",
        &[&id],
    )
    .await
    .or_query_error("clear terminology chunks")?;
    let insert = txn
        .prepare("INSERT INTO sof_terminology_chunks (relation_id, seq, rows) VALUES ($1, $2, $3)")
        .await
        .or_query_error("prepare terminology chunk insert")?;
    for (seq, chunk) in relation.rows.chunks(CHUNK_ROWS).enumerate() {
        let rows = Value::Array(chunk.to_vec());
        txn.execute(&insert, &[&id, &(seq as i32), &rows])
            .await
            .or_query_error("insert terminology chunk")?;
    }
    Ok(())
}

#[async_trait]
impl SofTerminologyStore for PostgresBackend {
    async fn set_library_dependencies(
        &self,
        tenant: &TenantContext,
        library_id: &str,
        relations: Vec<(String, TerminologyRelation)>,
    ) -> StorageResult<()> {
        let tenant_id = tenant.tenant_id().as_str();
        let mut client = self.guarded_client().await?;
        let txn = client
            .transaction()
            .await
            .or_query_error("terminology begin")?;
        lock_tenant(&txn, tenant_id).await?;
        for (canonical, relation) in &relations {
            write_relation(&txn, tenant_id, canonical, relation).await?;
        }
        txn.execute(
            "DELETE FROM sof_terminology_refs WHERE tenant_id = $1 AND library_id = $2",
            &[&tenant_id, &library_id],
        )
        .await
        .or_query_error("clear terminology refs")?;
        for (canonical, _) in &relations {
            txn.execute(
                "INSERT INTO sof_terminology_refs (tenant_id, library_id, canonical)
                 VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                &[&tenant_id, &library_id, canonical],
            )
            .await
            .or_query_error("insert terminology ref")?;
        }
        txn.execute(
            "DELETE FROM sof_terminology_relations t
              WHERE t.tenant_id = $1
                AND NOT EXISTS (SELECT 1 FROM sof_terminology_refs r
                                 WHERE r.tenant_id = t.tenant_id AND r.canonical = t.canonical)",
            &[&tenant_id],
        )
        .await
        .or_query_error("drop unreferenced terminology relations")?;
        txn.commit().await.or_query_error("terminology commit")?;
        client.mark_settled();
        Ok(())
    }

    async fn load(
        &self,
        tenant: &TenantContext,
        canonical: &str,
    ) -> StorageResult<Option<TerminologyRelation>> {
        let tenant_id = tenant.tenant_id().as_str();
        let mut client = self.guarded_client().await?;
        // One snapshot for the registry row and its chunks, so a concurrent
        // rewrite cannot hand back a mix of two versions.
        let txn = client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .await
            .or_query_error("terminology read begin")?;
        let Some(entry) = txn
            .query_opt(
                "SELECT id, kind FROM sof_terminology_relations
                  WHERE tenant_id = $1 AND canonical = $2",
                &[&tenant_id, &canonical],
            )
            .await
            .or_query_error("read terminology relation")?
        else {
            txn.commit().await.or_query_error("terminology read end")?;
            client.mark_settled();
            return Ok(None);
        };
        let id: i64 = entry.get(0);
        let kind: String = entry.get(1);
        let chunks = txn
            .query(
                "SELECT rows FROM sof_terminology_chunks WHERE relation_id = $1 ORDER BY seq",
                &[&id],
            )
            .await
            .or_query_error("read terminology chunks")?;
        txn.commit().await.or_query_error("terminology read end")?;
        client.mark_settled();
        let Some(kind) = TerminologyKind::parse(&kind) else {
            return Ok(None);
        };
        let mut rows = Vec::new();
        for chunk in chunks {
            if let Value::Array(chunk) = chunk.get::<_, Value>(0) {
                rows.extend(chunk);
            }
        }
        Ok(Some(TerminologyRelation { kind, rows }))
    }
}
