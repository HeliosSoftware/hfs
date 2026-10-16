//! SQLite implementation of the [`SofTerminologyStore`] (#1824).
//!
//! Two bookkeeping tables, created by the v40 migration:
//!
//! - `sof_terminology_tables`: one row per (tenant, canonical) relation, whose
//!   rowid names the physical table holding its rows (`sof_term_<id>`).
//! - `sof_terminology_refs`: one row per (tenant, Library, canonical)
//!   dependency.
//!
//! A relation's physical table has exactly the spec's columns for its kind
//! (see [`TerminologyKind::columns`]). It is dropped, with its registry row,
//! as soon as no reference to its canonical remains.

use std::time::Duration;

use async_trait::async_trait;
use helios_sof::sqlquery::{ColumnFhirType, TerminologyKind, TerminologyRelation};
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde_json::{Map, Value};

use super::SqliteBackend;
use super::bulk_submit::retry_bookkeeping_on_busy_async;
use crate::core::sof_terminology::SofTerminologyStore;
use crate::error::{QueryErrorExt, StorageResult};
use crate::tenant::TenantContext;

/// How long a write may retry a shared-cache lock conflict.
const LOCK_RETRY_BUDGET: Duration = Duration::from_secs(5);

/// The physical table holding the rows of registry entry `id`. Built only from
/// an integer, so it never needs quoting beyond the usual.
fn physical_table(id: i64) -> String {
    format!("sof_term_{id}")
}

/// Creates the bookkeeping tables. The body of the v39 -> v40 migration.
pub(crate) fn create_bookkeeping_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sof_terminology_tables (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             tenant_id TEXT NOT NULL,
             canonical TEXT NOT NULL,
             kind TEXT NOT NULL,
             row_count INTEGER NOT NULL,
             materialized_at TEXT NOT NULL,
             UNIQUE (tenant_id, canonical)
         );
         CREATE TABLE IF NOT EXISTS sof_terminology_refs (
             tenant_id TEXT NOT NULL,
             library_id TEXT NOT NULL,
             canonical TEXT NOT NULL,
             PRIMARY KEY (tenant_id, library_id, canonical)
         );
         CREATE INDEX IF NOT EXISTS idx_sof_terminology_refs_canonical
             ON sof_terminology_refs (tenant_id, canonical);",
    )
}

/// Drops every relation table of `tenant_id` and its bookkeeping, inside the
/// caller's transaction (tenant purge).
pub(crate) fn purge_tenant_in_txn(conn: &Connection, tenant_id: &str) -> StorageResult<()> {
    let ids = select_ids(
        conn,
        "SELECT id FROM sof_terminology_tables WHERE tenant_id = ?1",
        tenant_id,
    )?;
    for id in ids {
        conn.execute(
            &format!("DROP TABLE IF EXISTS \"{}\"", physical_table(id)),
            [],
        )
        .or_query_error("purge terminology table")?;
    }
    for sql in [
        "DELETE FROM sof_terminology_tables WHERE tenant_id = ?1",
        "DELETE FROM sof_terminology_refs WHERE tenant_id = ?1",
    ] {
        conn.execute(sql, params![tenant_id])
            .or_query_error("purge terminology bookkeeping")?;
    }
    Ok(())
}

fn select_ids(conn: &Connection, sql: &str, tenant_id: &str) -> StorageResult<Vec<i64>> {
    let mut stmt = conn
        .prepare(sql)
        .or_query_error("prepare terminology ids")?;
    let ids = stmt
        .query_map(params![tenant_id], |row| row.get::<_, i64>(0))
        .or_query_error("query terminology ids")?
        .collect::<Result<Vec<_>, _>>()
        .or_query_error("read terminology ids")?;
    Ok(ids)
}

/// Writes one relation: upserts its registry row and replaces its table.
fn write_relation(
    conn: &Connection,
    tenant_id: &str,
    canonical: &str,
    relation: &TerminologyRelation,
) -> StorageResult<()> {
    let kind = relation.kind;
    conn.execute(
        "INSERT INTO sof_terminology_tables (tenant_id, canonical, kind, row_count, materialized_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (tenant_id, canonical) DO UPDATE SET
             kind = ?3, row_count = ?4, materialized_at = ?5",
        params![
            tenant_id,
            canonical,
            kind.as_str(),
            relation.rows.len() as i64,
            chrono::Utc::now().to_rfc3339(),
        ],
    )
    .or_query_error("upsert terminology registry")?;
    let id: i64 = conn
        .query_row(
            "SELECT id FROM sof_terminology_tables WHERE tenant_id = ?1 AND canonical = ?2",
            params![tenant_id, canonical],
            |row| row.get(0),
        )
        .or_query_error("read terminology registry id")?;
    let table = physical_table(id);
    let columns = kind.columns();
    let ddl = columns
        .iter()
        .map(|(name, ty)| {
            format!(
                "\"{name}\" {}",
                ColumnFhirType::from_code(ty).sqlite_affinity()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute(&format!("DROP TABLE IF EXISTS \"{table}\""), [])
        .or_query_error("drop terminology table")?;
    conn.execute(&format!("CREATE TABLE \"{table}\" ({ddl})"), [])
        .or_query_error("create terminology table")?;
    let names = columns
        .iter()
        .map(|(name, _)| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = vec!["?"; columns.len()].join(", ");
    let mut insert = conn
        .prepare(&format!(
            "INSERT INTO \"{table}\" ({names}) VALUES ({placeholders})"
        ))
        .or_query_error("prepare terminology insert")?;
    for row in &relation.rows {
        let values = columns.iter().map(|(name, _)| match row.get(*name) {
            Some(Value::String(s)) => rusqlite::types::Value::Text(s.clone()),
            Some(Value::Bool(b)) => rusqlite::types::Value::Integer(i64::from(*b)),
            _ => rusqlite::types::Value::Null,
        });
        insert
            .execute(params_from_iter(values))
            .or_query_error("insert terminology row")?;
    }
    Ok(())
}

/// Drops every table of `tenant_id` no Library references any more.
fn drop_unreferenced(conn: &Connection, tenant_id: &str) -> StorageResult<()> {
    let orphans = select_ids(
        conn,
        "SELECT t.id FROM sof_terminology_tables t
          WHERE t.tenant_id = ?1
            AND NOT EXISTS (SELECT 1 FROM sof_terminology_refs r
                             WHERE r.tenant_id = t.tenant_id AND r.canonical = t.canonical)",
        tenant_id,
    )?;
    for id in orphans {
        conn.execute(
            &format!("DROP TABLE IF EXISTS \"{}\"", physical_table(id)),
            [],
        )
        .or_query_error("drop unreferenced terminology table")?;
        conn.execute(
            "DELETE FROM sof_terminology_tables WHERE id = ?1",
            params![id],
        )
        .or_query_error("delete terminology registry row")?;
    }
    Ok(())
}

#[async_trait]
impl SofTerminologyStore for SqliteBackend {
    async fn set_library_dependencies(
        &self,
        tenant: &TenantContext,
        library_id: &str,
        relations: Vec<(String, TerminologyRelation)>,
    ) -> StorageResult<()> {
        let tenant_id = tenant.tenant_id().as_str().to_string();
        let library_id = library_id.to_string();
        let relations = std::sync::Arc::new(relations);
        retry_bookkeeping_on_busy_async(
            self,
            "set_library_dependencies",
            LOCK_RETRY_BUDGET,
            || {
                let tenant_id = tenant_id.clone();
                let library_id = library_id.clone();
                let relations = std::sync::Arc::clone(&relations);
                move |conn: &Connection| {
                    // IMMEDIATE: the orphan sweep reads before it writes (see
                    // `purge_tenant_data` for the WAL snapshot-upgrade hazard).
                    let tx = rusqlite::Transaction::new_unchecked(
                        conn,
                        rusqlite::TransactionBehavior::Immediate,
                    )
                    .or_query_error("terminology begin")?;
                    for (canonical, relation) in relations.iter() {
                        write_relation(&tx, &tenant_id, canonical, relation)?;
                    }
                    tx.execute(
                        "DELETE FROM sof_terminology_refs WHERE tenant_id = ?1 AND library_id = ?2",
                        params![tenant_id, library_id],
                    )
                    .or_query_error("clear terminology refs")?;
                    for (canonical, _) in relations.iter() {
                        tx.execute(
                            "INSERT OR IGNORE INTO sof_terminology_refs (tenant_id, library_id, canonical)
                             VALUES (?1, ?2, ?3)",
                            params![tenant_id, library_id, canonical],
                        )
                        .or_query_error("insert terminology ref")?;
                    }
                    drop_unreferenced(&tx, &tenant_id)?;
                    tx.commit().or_query_error("terminology commit")
                }
            },
        )
        .await
    }

    async fn load(
        &self,
        tenant: &TenantContext,
        canonical: &str,
    ) -> StorageResult<Option<TerminologyRelation>> {
        let tenant_id = tenant.tenant_id().as_str().to_string();
        let canonical = canonical.to_string();
        self.run_blocking(move |conn: &Connection| {
            let entry: Option<(i64, String)> = conn
                .query_row(
                    "SELECT id, kind FROM sof_terminology_tables
                      WHERE tenant_id = ?1 AND canonical = ?2",
                    params![tenant_id, canonical],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .or_query_error("read terminology registry")?;
            let Some((id, kind)) = entry else {
                return Ok(None);
            };
            let Some(kind) = TerminologyKind::parse(&kind) else {
                return Ok(None);
            };
            let columns = kind.columns();
            let names = columns
                .iter()
                .map(|(name, _)| format!("\"{name}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {names} FROM \"{}\" ORDER BY rowid",
                    physical_table(id)
                ))
                .or_query_error("prepare terminology read")?;
            let rows = stmt
                .query_map([], |row| {
                    let mut obj = Map::new();
                    for (i, (name, ty)) in columns.iter().enumerate() {
                        let value = if *ty == "boolean" {
                            row.get::<_, Option<i64>>(i)?
                                .map_or(Value::Null, |v| Value::Bool(v != 0))
                        } else {
                            row.get::<_, Option<String>>(i)?
                                .map_or(Value::Null, Value::String)
                        };
                        obj.insert((*name).to_string(), value);
                    }
                    Ok(Value::Object(obj))
                })
                .or_query_error("query terminology rows")?
                .collect::<Result<Vec<_>, _>>()
                .or_query_error("read terminology rows")?;
            Ok(Some(TerminologyRelation { kind, rows }))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenant::{TenantId, TenantPermissions};
    use serde_json::json;

    fn tenant(id: &str) -> TenantContext {
        TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
    }

    fn vs(codes: &[&str]) -> TerminologyRelation {
        TerminologyRelation {
            kind: TerminologyKind::ValueSet,
            rows: codes
                .iter()
                .map(|c| {
                    json!({"system": "http://s", "version": null, "code": c,
                           "display": null, "inactive": c.starts_with('x')})
                })
                .collect(),
        }
    }

    fn table_count(backend: &SqliteBackend) -> i64 {
        backend
            .get_connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name GLOB 'sof_term_[0-9]*'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn backend() -> SqliteBackend {
        let backend = SqliteBackend::in_memory().expect("in-memory backend");
        backend.init_schema().expect("init schema");
        backend
    }

    #[tokio::test]
    async fn shared_table_is_dropped_with_its_last_reference() {
        let backend = backend();
        let t = tenant("acme");
        let url = "http://example.org/vs";

        backend
            .set_library_dependencies(&t, "lib-a", vec![(url.into(), vs(&["a", "xb"]))])
            .await
            .unwrap();
        backend
            .set_library_dependencies(&t, "lib-b", vec![(url.into(), vs(&["a", "xb", "c"]))])
            .await
            .unwrap();
        assert_eq!(table_count(&backend), 1, "one shared table per canonical");

        // The latest write refreshes the shared table.
        let loaded = backend.load(&t, url).await.unwrap().unwrap();
        assert_eq!(loaded, vs(&["a", "xb", "c"]));

        backend
            .set_library_dependencies(&t, "lib-a", vec![])
            .await
            .unwrap();
        assert!(
            backend.load(&t, url).await.unwrap().is_some(),
            "lib-b still refers"
        );

        backend
            .set_library_dependencies(&t, "lib-b", vec![])
            .await
            .unwrap();
        assert!(backend.load(&t, url).await.unwrap().is_none());
        assert_eq!(table_count(&backend), 0);
    }

    #[tokio::test]
    async fn dropping_a_dependency_on_update_releases_it() {
        let backend = backend();
        let t = tenant("acme");
        backend
            .set_library_dependencies(
                &t,
                "lib",
                vec![
                    ("http://vs/1".into(), vs(&["a"])),
                    ("http://vs/2".into(), vs(&["b"])),
                ],
            )
            .await
            .unwrap();
        backend
            .set_library_dependencies(&t, "lib", vec![("http://vs/2".into(), vs(&["b"]))])
            .await
            .unwrap();
        assert!(backend.load(&t, "http://vs/1").await.unwrap().is_none());
        assert!(backend.load(&t, "http://vs/2").await.unwrap().is_some());
        assert_eq!(table_count(&backend), 1);
    }

    #[tokio::test]
    async fn tenants_are_isolated() {
        let backend = backend();
        let url = "http://example.org/vs";
        backend
            .set_library_dependencies(&tenant("a"), "lib", vec![(url.into(), vs(&["a"]))])
            .await
            .unwrap();
        assert!(backend.load(&tenant("b"), url).await.unwrap().is_none());
        // Releasing the same Library id in another tenant leaves tenant a's alone.
        backend
            .set_library_dependencies(&tenant("b"), "lib", vec![])
            .await
            .unwrap();
        assert!(backend.load(&tenant("a"), url).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn concept_map_rows_round_trip() {
        let backend = backend();
        let t = tenant("acme");
        let row = json!({
            "source_system": "http://a", "source_version": null, "source_code": "1",
            "source_display": "One", "target_system": "http://b", "target_version": "2",
            "target_code": null, "target_display": null, "relationship": null
        });
        let rel = TerminologyRelation {
            kind: TerminologyKind::ConceptMap,
            rows: vec![row],
        };
        backend
            .set_library_dependencies(&t, "lib", vec![("http://cm".into(), rel.clone())])
            .await
            .unwrap();
        assert_eq!(backend.load(&t, "http://cm").await.unwrap(), Some(rel));
    }

    #[tokio::test]
    async fn purge_drops_the_tenants_tables() {
        use crate::core::ResourceStorage;
        let backend = backend();
        let t = tenant("acme");
        backend
            .set_library_dependencies(&t, "lib", vec![("http://vs".into(), vs(&["a"]))])
            .await
            .unwrap();
        backend.purge_tenant_data("acme").await.unwrap();
        assert!(backend.load(&t, "http://vs").await.unwrap().is_none());
        assert_eq!(table_count(&backend), 0);
    }
}
