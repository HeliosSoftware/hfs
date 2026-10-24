//! Allocation and indexed lookup of public terminology resource IDs.

use helios_persistence::tenant::SYSTEM_TENANT;
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;

use crate::backends::resource_identity::{
    ImportIdentity, LegacyIdentitySource, MigrationOwners, ResourceKind, ResourceRecord,
    matches_persisted_resource, valid_logical_id,
};
use crate::error::HtsError;

fn storage_error(error: rusqlite::Error) -> HtsError {
    HtsError::StorageError(error.to_string())
}

/// Runs after legacy table rebuilds, before the backend accepts requests.
pub(super) fn migrate(conn: &mut Connection) -> Result<(), HtsError> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let complete: bool = tx
        .query_row(
            "SELECT COUNT(*) = 2 FROM sqlite_master WHERE type = 'index' AND name \
             IN ('idx_code_systems_public_id', 'idx_value_sets_public_id')",
            [],
            |row| row.get(0),
        )
        .map_err(storage_error)?;
    if complete {
        return tx.commit().map_err(storage_error);
    }
    for kind in ResourceKind::VERSIONED_KINDS {
        let table = kind.table();
        let rows = tx
            .prepare(&format!(
                "SELECT id, url, version, CASE WHEN json_type(resource_json, '$.id') = 'text' \
                 THEN json_extract(resource_json, '$.id') END, resource_json IS NULL FROM {table} \
                 ORDER BY created_at, id"
            ))
            .map_err(storage_error)?
            .query_map([], |row| {
                Ok(ImportIdentity {
                    storage_id: row.get(0)?,
                    url: row.get(1)?,
                    version: row.get(2)?,
                    source_id: row.get(3)?,
                    resource_is_null: row.get(4)?,
                })
            })
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        let mut owners = MigrationOwners::default();
        let has_resources: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' \
                 AND name = 'resources')",
                [],
                |row| row.get(0),
            )
            .map_err(storage_error)?;
        if !rows.is_empty() && has_resources {
            let mut statement = tx
                .prepare(
                    "SELECT id, CASE WHEN json_type(data, '$.url') = 'text' \
                     THEN json_extract(data, '$.url') END, \
                     CASE WHEN json_type(data, '$.version') = 'text' \
                     THEN json_extract(data, '$.version') END, is_deleted FROM resources \
                     WHERE tenant_id = ?1 AND resource_type = ?2",
                )
                .map_err(storage_error)?;
            let persisted = statement
                .query_map(
                    rusqlite::params![SYSTEM_TENANT, kind.resource_type()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, bool>(3)?,
                        ))
                    },
                )
                .map_err(storage_error)?;
            for row in persisted {
                let (id, url, version, deleted) = row.map_err(storage_error)?;
                owners.insert(id, url, version, deleted);
            }
        }
        let extra_metadata = if matches!(kind, ResourceKind::CodeSystem) {
            "'content', content"
        } else {
            "'compose', json(compose_json)"
        };
        for row in rows {
            let id = owners.allocate(&row, LegacyIdentitySource::ResourceJson);
            if row.source_id.as_deref() == Some(&id)
                || (row.resource_is_null && row.storage_id == id)
            {
                continue;
            }
            tx.execute(
                &format!(
                    "UPDATE {table} SET resource_json = json_set(COALESCE(resource_json,
                   json_patch(json_object('resourceType', ?3, 'url', url, 'status', status),
                     json_object('version', version, 'name', name, 'title', title, \
                     {extra_metadata}))),
                   '$.id', ?1) WHERE id = ?2"
                ),
                rusqlite::params![id, row.storage_id, kind.resource_type()],
            )
            .map_err(storage_error)?;
        }
        let identity = kind.sqlite_identity_expression();
        tx.execute_batch(&format!(
            "CREATE UNIQUE INDEX idx_{table}_public_id ON {table}({identity});
             DROP INDEX IF EXISTS idx_{table}_meta;
             CREATE INDEX idx_{table}_meta ON {table}(created_at, id, {identity}, url, version, \
             name, title, status)"
        ))
        .map_err(storage_error)?;
    }
    tx.commit().map_err(storage_error)
}

/// Reject package replacement of content managed by the REST resource store.
/// The bulk caller owns an immediate write transaction and checked that the
/// resources table exists; normal CRUD uses its separate strict write path.
pub(crate) fn check_import_owner(
    conn: &Connection,
    kind: ResourceKind,
    source_id: &str,
    url: &str,
    version: Option<&str>,
) -> Result<(), HtsError> {
    if matches!(kind, ResourceKind::ConceptMap) {
        let reserved: bool = conn
            .prepare_cached(
                "SELECT EXISTS(SELECT 1 FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 \
                 AND id = ?3)",
            )
            .and_then(|mut statement| {
                statement.query_row(
                    rusqlite::params![SYSTEM_TENANT, kind.resource_type(), source_id],
                    |row| row.get(0),
                )
            })
            .map_err(storage_error)?;
        if reserved {
            return Err(HtsError::Conflict(format!(
                "ConceptMap/{source_id} is reserved by the REST resource store"
            )));
        }
    }

    let (version_column, version) = if matches!(kind, ResourceKind::ConceptMap) {
        ("''", None)
    } else {
        ("version", version)
    };
    let owner: Option<String> = conn
        .prepare_cached(&format!(
            "SELECT r.id FROM (SELECT {} AS public_id FROM {}
         WHERE url = ?1 AND COALESCE({version_column}, '') = COALESCE(?2, '')) AS normalized
         JOIN resources r ON r.id = normalized.public_id
         WHERE r.tenant_id = ?3 AND r.resource_type = ?4 AND r.is_deleted = 0",
            kind.sqlite_identity_expression(),
            kind.table(),
        ))
        .and_then(|mut statement| {
            statement.query_row(
                rusqlite::params![url, version, SYSTEM_TENANT, kind.resource_type()],
                |row| row.get(0),
            )
        })
        .optional()
        .map_err(storage_error)?;
    if let Some(id) = owner {
        return Err(HtsError::Conflict(format!(
            "{}/{id} is managed through REST; import cannot replace its content",
            kind.resource_type(),
        )));
    }
    Ok(())
}

/// The caller holds the write transaction through assignment and commit.
pub(crate) fn allocate(
    conn: &Connection,
    kind: ResourceKind,
    storage_id: &str,
    source_id: &str,
    url: &str,
    version: Option<&str>,
) -> Result<String, HtsError> {
    let table = kind.table();
    let existing = conn
        .prepare_cached(&format!(
            "SELECT COALESCE(json_extract(resource_json, '$.id'), id) FROM {table} \
             WHERE id = ?1"
        ))
        .and_then(|mut statement| {
            statement.query_row([storage_id], |row| row.get::<_, Option<String>>(0))
        })
        .optional()
        .map_err(storage_error)?
        .flatten();
    if let Some(id) = existing {
        if !valid_logical_id(&id) {
            return Err(HtsError::StorageError(
                "Stored public resource id is invalid".into(),
            ));
        }
        return Ok(id);
    }
    allocate_available(conn, kind, storage_id, source_id, url, version)
}

fn allocate_available(
    conn: &Connection,
    kind: ResourceKind,
    storage_id: &str,
    source_id: &str,
    url: &str,
    version: Option<&str>,
) -> Result<String, HtsError> {
    let table = kind.table();
    let identity = kind.sqlite_identity_expression();
    let has_resources: bool = conn
        .prepare_cached(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' \
             AND name = 'resources')",
        )
        .and_then(|mut statement| statement.query_row([], |row| row.get(0)))
        .map_err(storage_error)?;
    let mut candidate = if valid_logical_id(source_id) {
        source_id.to_owned()
    } else {
        uuid::Uuid::new_v4().to_string()
    };
    loop {
        let taken: bool = conn
            .prepare_cached(&format!(
                "SELECT EXISTS(SELECT 1 FROM {table} WHERE {identity} = ?1 AND id != ?2)"
            ))
            .and_then(|mut statement| {
                statement.query_row(rusqlite::params![candidate, storage_id], |row| row.get(0))
            })
            .map_err(storage_error)?;
        let reserved = if has_resources {
            conn.prepare_cached(
                "SELECT data, is_deleted FROM resources WHERE tenant_id = ?1 \
                 AND resource_type = ?2 AND id = ?3",
            )
            .and_then(|mut statement| {
                statement.query_row(
                    rusqlite::params![SYSTEM_TENANT, kind.resource_type(), candidate],
                    |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, bool>(1)?)),
                )
            })
            .optional()
            .map_err(storage_error)?
            .map(|(data, deleted)| {
                serde_json::from_slice::<Value>(&data)
                    .map(|resource| !matches_persisted_resource(&resource, deleted, url, version))
                    .map_err(|error| HtsError::StorageError(error.to_string()))
            })
            .transpose()?
            .unwrap_or(false)
        } else {
            false
        };
        if !taken && !reserved {
            break;
        }
        candidate = uuid::Uuid::new_v4().to_string();
    }
    Ok(candidate)
}

pub(crate) fn read(
    conn: &Connection,
    kind: ResourceKind,
    id: &str,
) -> Result<Option<Value>, HtsError> {
    let table = kind.table();
    // Synthetic legacy rows use their already-valid storage ID until migration.
    // The expression matches the unique index in both cases.
    let key = kind.sqlite_identity_expression();
    let sql = format!(
        "SELECT url, version, name, title, status, resource_json{} FROM {table} WHERE {key} = ?1",
        if matches!(kind, ResourceKind::ValueSet) {
            ", compose_json"
        } else {
            ""
        }
    );
    let record = conn
        .query_row(&sql, [id], |row| {
            Ok(ResourceRecord {
                url: row.get(0)?,
                version: row.get(1)?,
                name: row.get(2)?,
                title: row.get(3)?,
                status: row.get(4)?,
                resource: None,
                compose: if matches!(kind, ResourceKind::ValueSet) {
                    row.get(6)?
                } else {
                    None
                },
            })
            .and_then(|mut record| {
                record.resource = row
                    .get::<_, Option<String>>(5)?
                    .map(|json| serde_json::from_str(&json))
                    .transpose()
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            5,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                Ok(record)
            })
        })
        .optional()
        .map_err(storage_error)?;
    record
        .map(|record| record.into_resource(kind, id))
        .transpose()
}

/// Instance operations that only need a URL must not hydrate large CodeSystems.
pub(super) fn url(
    conn: &Connection,
    kind: ResourceKind,
    id: &str,
) -> Result<Option<String>, HtsError> {
    conn.query_row(
        &format!(
            "SELECT url FROM {} WHERE {} = ?1",
            kind.table(),
            kind.sqlite_identity_expression()
        ),
        [id],
        |row| row.get(0),
    )
    .optional()
    .map_err(storage_error)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::*;

    fn seed(
        conn: &Connection,
        storage_id: &str,
        url: &str,
        version: Option<&str>,
        source_id: Option<&str>,
    ) {
        let resource = source_id.map(|id| {
            json!({
                "resourceType": "ValueSet", "id": id, "url": url,
                "version": version, "status": "active",
            })
            .to_string()
        });
        conn.execute(
            "INSERT INTO value_sets(id, url, version, status, resource_json, compose_json, \
             created_at, updated_at)
             VALUES (?1, ?2, ?3, 'active', ?4, '{\"include\":[]}', 'now', 'now')",
            rusqlite::params![storage_id, url, version, resource],
        )
        .unwrap();
    }

    fn identities(conn: &Connection) -> Vec<(String, String)> {
        conn.prepare(
            "SELECT id, COALESCE(json_extract(resource_json, '$.id'), id) FROM value_sets \
             ORDER BY id",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    }

    #[test]
    fn legacy_migration_preserves_crud_ids_and_uses_indexed_reads() {
        let mut conn = Connection::open_in_memory().unwrap();
        super::super::schema::apply(&conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE resources(tenant_id TEXT, resource_type TEXT, id TEXT, data BLOB, \
             is_deleted INTEGER,
                PRIMARY KEY(tenant_id,resource_type,id))",
        )
        .unwrap();
        for (id, url, deleted) in [
            ("shared", "http://example.org/primary", false),
            ("deleted", "http://example.org/deleted", true),
            ("drift-owner", "http://example.org/drift", false),
        ] {
            let data = serde_json::to_vec(&json!({"id": id, "url": url, "version": "1"})).unwrap();
            conn.execute(
                "INSERT INTO resources VALUES (?1, 'ValueSet', ?2, ?3, ?4)",
                rusqlite::params![SYSTEM_TENANT, id, data, deleted],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO resources VALUES (?1, 'ValueSet', 'malformed-owner', ?2, 0)",
            rusqlite::params![
                SYSTEM_TENANT,
                serde_json::to_vec(&json!({"url":123,"version":false})).unwrap()
            ],
        )
        .unwrap();
        seed(
            &conn,
            "malformed-owner|1",
            "http://example.org/malformed-owner",
            Some("1"),
            Some("malformed-owner"),
        );
        seed(
            &conn,
            "shared",
            "http://example.org/other",
            None,
            Some("shared"),
        );
        seed(
            &conn,
            "shared|1",
            "http://example.org/primary",
            Some("1"),
            Some("shared"),
        );
        seed(
            &conn,
            "shared|2",
            "http://example.org/primary",
            Some("2"),
            Some("shared"),
        );
        seed(
            &conn,
            "deleted|1",
            "http://example.org/deleted",
            Some("1"),
            Some("deleted"),
        );
        seed(
            &conn,
            "synthetic",
            "http://example.org/synthetic",
            None,
            None,
        );
        seed(
            &conn,
            "invalid|1",
            "http://example.org/invalid",
            None,
            Some("invalid|source"),
        );
        seed(
            &conn,
            "long|1",
            "http://example.org/long",
            None,
            Some(&"a".repeat(65)),
        );
        seed(
            &conn,
            "old-public",
            "http://example.org/old",
            None,
            Some("new-source"),
        );
        seed(
            &conn,
            "drift-owner|1",
            "http://example.org/drift",
            Some("1"),
            Some("renamed-source"),
        );
        seed(
            &conn,
            "swap-a",
            "http://example.org/swap/a",
            None,
            Some("swap-b"),
        );
        seed(
            &conn,
            "swap-b",
            "http://example.org/swap/b",
            None,
            Some("swap-a"),
        );
        seed(
            &conn,
            "numeric",
            "http://example.org/numeric",
            None,
            Some("numeric"),
        );
        conn.execute(
            "UPDATE value_sets SET resource_json = json_set(resource_json, '$.id', 123) \
             WHERE id = 'numeric'",
            [],
        )
        .unwrap();
        seed(&conn, "null|1", "http://example.org/null", Some("1"), None);
        conn.execute(
            "INSERT INTO \
             code_systems(id,url,version,status,content,created_at,updated_at) \
             VALUES ('null-cs|1','http://example.org/null-cs','1','active','fragment','now','now')",
            [],
        )
        .unwrap();
        let uuid_id = "859c0215-d26e-4ce2-a4a8-c3d80217b223";
        seed(
            &conn,
            uuid_id,
            "http://example.org/uuid",
            None,
            Some(uuid_id),
        );
        for index in 0..2_000 {
            seed(
                &conn,
                &format!("seed-{index}|1"),
                &format!("http://example.org/seed/{index}"),
                Some("1"),
                Some(&format!("seed-{index}")),
            );
        }

        migrate(&mut conn).unwrap();
        let before = identities(&conn);
        assert!(before.iter().all(|(_, id)| valid_logical_id(id)));
        assert_eq!(
            before
                .iter()
                .map(|(_, id)| id)
                .collect::<BTreeSet<_>>()
                .len(),
            before.len()
        );
        assert_eq!(
            before
                .iter()
                .find(|(storage, _)| storage == "shared|1")
                .unwrap()
                .1,
            "shared"
        );
        assert_ne!(
            before
                .iter()
                .find(|(storage, _)| storage == "shared")
                .unwrap()
                .1,
            "shared"
        );
        assert_ne!(
            before
                .iter()
                .find(|(storage, _)| storage == "deleted|1")
                .unwrap()
                .1,
            "deleted"
        );
        assert_eq!(
            before
                .iter()
                .find(|(storage, _)| storage == uuid_id)
                .unwrap()
                .1,
            uuid_id
        );
        for (storage, expected) in [
            ("old-public", "new-source"),
            ("swap-a", "swap-b"),
            ("swap-b", "swap-a"),
            ("drift-owner|1", "drift-owner"),
        ] {
            assert_eq!(
                before.iter().find(|(key, _)| key == storage).unwrap().1,
                expected
            );
        }
        for table in ["code_systems", "value_sets"] {
            let columns = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(!columns.iter().any(|name| name == "logical_id"));
        }
        let numeric: String = conn
            .query_row(
                "SELECT json_type(resource_json, '$.id') FROM value_sets WHERE id = 'numeric'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(numeric, "text");
        let null_json: Option<String> = conn
            .query_row(
                "SELECT resource_json FROM value_sets WHERE id = 'synthetic'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            null_json.is_none(),
            "valid synthetic IDs do not require JSON rewrites"
        );
        let repaired: String = conn
            .query_row(
                "SELECT resource_json FROM value_sets WHERE id = 'null|1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let repaired: Value = serde_json::from_str(&repaired).unwrap();
        assert_eq!(repaired["compose"], json!({"include": []}));
        assert_eq!(repaired["version"], "1");
        assert!(valid_logical_id(repaired["id"].as_str().unwrap()));
        let repaired: String = conn
            .query_row(
                "SELECT resource_json FROM code_systems WHERE id = 'null-cs|1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let repaired: Value = serde_json::from_str(&repaired).unwrap();
        assert_eq!(repaired["content"], "fragment");
        assert_ne!(
            before
                .iter()
                .find(|(id, _)| id == "malformed-owner|1")
                .unwrap()
                .1,
            "malformed-owner"
        );
        let selected = read(&conn, ResourceKind::ValueSet, "shared")
            .unwrap()
            .unwrap();
        assert_eq!(selected["url"], "http://example.org/primary");
        assert_eq!(selected["version"], "1");
        let synthetic = read(&conn, ResourceKind::ValueSet, "synthetic")
            .unwrap()
            .unwrap();
        assert_eq!(synthetic["id"], "synthetic");
        assert_eq!(synthetic["compose"], json!({"include": []}));
        assert!(
            read(&conn, ResourceKind::ValueSet, "deleted")
                .unwrap()
                .is_none()
        );
        migrate(&mut conn).unwrap();
        assert_eq!(identities(&conn), before);

        conn.execute_batch("ANALYZE").unwrap();
        let plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN SELECT url FROM value_sets WHERE \
                 COALESCE(json_extract(resource_json, '$.id'), id) = ?1",
                ["shared"],
                |row| row.get(3),
            )
            .unwrap();
        assert!(plan.contains("idx_value_sets_public_id"), "{plan}");
        let plan: String = conn.query_row(
            "EXPLAIN QUERY PLAN SELECT COALESCE(json_extract(resource_json, '$.id'), id), url, \
             version, name, title, status
             FROM value_sets ORDER BY created_at LIMIT 20", [], |row| row.get(3),
        ).unwrap();
        assert!(
            plan.contains("COVERING INDEX idx_value_sets_meta"),
            "{plan}"
        );
    }
}
