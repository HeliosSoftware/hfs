//! Allocation and indexed lookup of public terminology resource IDs.

use deadpool_postgres::GenericClient;
use helios_persistence::tenant::SYSTEM_TENANT;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::backends::resource_identity::{
    ImportIdentity, LegacyIdentitySource, MigrationOwners, ResourceKind, ResourceRecord,
    matches_persisted_resource, valid_logical_id,
};
use crate::error::HtsError;

fn storage_error(error: tokio_postgres::Error) -> HtsError {
    HtsError::StorageError(super::error_chain(&error))
}

/// Only imports competing for a source ID or canonical version serialize.
/// Acquire every key in sorted order before writing, so bundles with reversed
/// resource order cannot deadlock. One command acquires the whole bounded batch.
pub(crate) async fn lock_import(
    client: &impl GenericClient,
    parsed: &crate::import::bundle_parser::ParsedBundle,
) -> Result<(), HtsError> {
    let mut keys = Vec::new();
    for resource in &parsed.code_systems {
        append_lock_keys(
            &mut keys,
            ResourceKind::CodeSystem,
            &resource.id,
            &resource.url,
            resource.version.as_deref(),
        );
    }
    for resource in &parsed.value_sets {
        append_lock_keys(
            &mut keys,
            ResourceKind::ValueSet,
            &resource.id,
            &resource.url,
            resource.version.as_deref(),
        );
    }
    for resource in &parsed.concept_maps {
        append_lock_keys(
            &mut keys,
            ResourceKind::ConceptMap,
            &resource.id,
            &resource.url,
            resource.version.as_deref(),
        );
    }
    lock_keys(client, keys).await
}

async fn lock_keys(client: &impl GenericClient, mut keys: Vec<i64>) -> Result<(), HtsError> {
    keys.sort_unstable();
    keys.dedup();
    if !keys.is_empty() {
        client.query(
            "SELECT pg_advisory_xact_lock(lock_key) FROM (SELECT unnest($1::bigint[]) AS lock_key \
             ORDER BY lock_key) AS ordered_keys",
            &[&keys],
        ).await.map_err(storage_error)?;
    }
    Ok(())
}

fn append_lock_keys(
    keys: &mut Vec<i64>,
    kind: ResourceKind,
    id: &str,
    url: &str,
    version: Option<&str>,
) {
    keys.push(lock_key(kind, "id", id, ""));
    // ConceptMap has one normalized row per URL, regardless of business version.
    let version = if matches!(kind, ResourceKind::ConceptMap) {
        ""
    } else {
        version.unwrap_or("")
    };
    keys.push(lock_key(kind, "canonical", url, version));
}

fn lock_key(kind: ResourceKind, scope: &str, value: &str, version: &str) -> i64 {
    let mut hash = Sha256::new();
    for field in [
        "hts-logical-id",
        kind.resource_type(),
        scope,
        value,
        version,
    ] {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field.as_bytes());
    }
    let digest = hash.finalize();
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&digest[..8]);
    i64::from_be_bytes(bytes)
}

#[derive(PartialEq, Eq)]
struct CanonicalIdentity {
    url: String,
    version: Option<String>,
}

async fn canonical_by_id(
    client: &impl GenericClient,
    kind: ResourceKind,
    id: &str,
) -> Result<Option<CanonicalIdentity>, HtsError> {
    client
        .query_opt(
            &format!(
                "SELECT url, version FROM {} WHERE {} = $1",
                kind.table(),
                kind.postgres_identity_expression()
            ),
            &[&id],
        )
        .await
        .map_err(storage_error)
        .map(|row| {
            row.map(|row| CanonicalIdentity {
                url: row.get(0),
                version: row.get(1),
            })
        })
}

/// Acquire the complete old/new identity set before resource reads or writes.
/// The caller uses READ COMMITTED so revalidation sees the prior lock holder's
/// commit. A changed discovery aborts instead of extending locks out of order.
pub(crate) async fn lock_mutation(
    client: &impl GenericClient,
    kind: ResourceKind,
    id: &str,
    replacement: Option<&Value>,
) -> Result<(), HtsError> {
    let previous = canonical_by_id(client, kind, id).await?;
    let mut keys = vec![lock_key(kind, "id", id, "")];
    if let Some(previous) = &previous {
        append_lock_keys(
            &mut keys,
            kind,
            id,
            &previous.url,
            previous.version.as_deref(),
        );
    }
    if let Some(replacement) = replacement {
        let url = replacement
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| HtsError::InvalidRequest("Canonical URL is required".into()))?;
        append_lock_keys(
            &mut keys,
            kind,
            id,
            url,
            replacement.get("version").and_then(Value::as_str),
        );
    }
    lock_keys(client, keys).await?;
    if canonical_by_id(client, kind, id).await? != previous {
        return Err(HtsError::Conflict(
            "Resource identity changed while acquiring mutation locks; read it again and retry"
                .into(),
        ));
    }
    Ok(())
}

/// Bulk imports cannot replace a REST-managed resource without also updating
/// its history. Strict CRUD normalization deliberately does not use this guard.
pub(crate) async fn check_import_owner(
    client: &impl GenericClient,
    kind: ResourceKind,
    source_id: &str,
    url: &str,
    version: Option<&str>,
) -> Result<(), HtsError> {
    if matches!(kind, ResourceKind::ConceptMap) {
        let reserved: bool = client
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM resources WHERE tenant_id = $1 AND resource_type = $2 \
                 AND id = $3)",
                &[&SYSTEM_TENANT, &kind.resource_type(), &source_id],
            )
            .await
            .map_err(storage_error)?
            .get(0);
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
    let owner: Option<String> = client
        .query_opt(
            &format!(
                "SELECT r.id FROM (SELECT {} AS public_id FROM {}
         WHERE url = $1 AND COALESCE({version_column}, '') = COALESCE($2, '')) AS normalized
         JOIN resources r ON r.id = normalized.public_id
         WHERE r.tenant_id = $3 AND r.resource_type = $4 AND NOT r.is_deleted",
                kind.postgres_identity_expression(),
                kind.table(),
            ),
            &[&url, &version, &SYSTEM_TENANT, &kind.resource_type()],
        )
        .await
        .map_err(storage_error)?
        .map(|row| row.get(0));
    if let Some(id) = owner {
        return Err(HtsError::Conflict(format!(
            "{}/{id} is managed through REST; import cannot replace its content",
            kind.resource_type(),
        )));
    }
    Ok(())
}

pub(super) async fn migrate(client: &mut deadpool_postgres::Object) -> Result<(), HtsError> {
    let tx = client.transaction().await.map_err(storage_error)?;
    // Serialize with base schema DDL before acquiring any table locks.
    tx.execute(
        "SELECT pg_advisory_xact_lock($1)",
        &[&super::schema::HTS_SCHEMA_LOCK],
    )
    .await
    .map_err(storage_error)?;
    let complete: bool = tx
        .query_one(
            "SELECT to_regclass('idx_code_systems_public_id') IS NOT NULL \
             AND to_regclass('idx_value_sets_public_id') IS NOT NULL",
            &[],
        )
        .await
        .map_err(storage_error)?
        .get(0);
    if complete {
        return tx.commit().await.map_err(storage_error);
    }
    tx.batch_execute("LOCK TABLE code_systems, value_sets IN ACCESS EXCLUSIVE MODE")
        .await
        .map_err(storage_error)?;
    for kind in ResourceKind::VERSIONED_KINDS {
        let table = kind.table();
        let mut rows = tx.query(&format!(
            "SELECT id, url, version, CASE WHEN jsonb_typeof(resource_json->'id') = 'string' \
             THEN resource_json->>'id' END, resource_json IS NULL FROM {table} \
             ORDER BY created_at, id"
        ), &[]).await.map_err(storage_error)?.into_iter().map(|row| ImportIdentity {
            storage_id: row.get(0), url: row.get(1), version: row.get(2),
            source_id: row.get(3), resource_is_null: row.get(4),
        }).collect::<Vec<_>>();
        // PostgreSQL previously published valid storage keys; preserve them
        // before assigning source IDs to invalid version-suffixed keys.
        rows.sort_by_key(|row| !valid_logical_id(&row.storage_id));
        let mut owners = MigrationOwners::default();
        if !rows.is_empty()
            && tx
                .query_one("SELECT to_regclass('resources') IS NOT NULL", &[])
                .await
                .map_err(storage_error)?
                .get::<_, bool>(0)
        {
            for row in tx.query(
                "SELECT id, CASE WHEN jsonb_typeof(data->'url') = 'string' THEN data->>'url' END, \
                 CASE WHEN jsonb_typeof(data->'version') = 'string' THEN data->>'version' \
                 END, is_deleted FROM resources WHERE tenant_id = $1 AND resource_type = $2",
                &[&SYSTEM_TENANT, &kind.resource_type()],
            ).await.map_err(storage_error)? {
                owners.insert(row.get(0), row.get(1), row.get(2), row.get(3));
            }
        }
        let extra_metadata = if matches!(kind, ResourceKind::CodeSystem) {
            "'content', content"
        } else {
            "'compose', compose_json::jsonb"
        };
        for row in rows {
            let id = owners.allocate(&row, LegacyIdentitySource::StorageKey);
            if row.source_id.as_deref() == Some(&id)
                || (row.resource_is_null && row.storage_id == id)
            {
                continue;
            }
            tx.execute(
                &format!(
                    "UPDATE {table} SET resource_json = jsonb_set(COALESCE(resource_json,
                   jsonb_strip_nulls(jsonb_build_object('resourceType', $3::text, 'url', url,
                     'version', version, 'name', name, 'title', title, 'status', status, \
                     {extra_metadata}))),
                   '{{id}}', to_jsonb($1::text), true) WHERE id = $2"
                ),
                &[&id, &row.storage_id, &kind.resource_type()],
            )
            .await
            .map_err(storage_error)?;
        }
        let identity = kind.postgres_identity_expression();
        tx.batch_execute(&format!(
            "CREATE UNIQUE INDEX idx_{table}_public_id ON {table}(({identity}))"
        ))
        .await
        .map_err(storage_error)?;
    }
    tx.commit().await.map_err(storage_error)
}

pub(crate) async fn allocate(
    client: &impl GenericClient,
    kind: ResourceKind,
    storage_id: &str,
    source_id: &str,
    url: &str,
    version: Option<&str>,
) -> Result<String, HtsError> {
    let table = kind.table();
    let existing = client
        .query_opt(
            &format!("SELECT COALESCE(resource_json->>'id', id) FROM {table} WHERE id = $1"),
            &[&storage_id],
        )
        .await
        .map_err(storage_error)?
        .and_then(|row| row.get::<_, Option<String>>(0));
    if let Some(id) = existing {
        if !valid_logical_id(&id) {
            return Err(HtsError::StorageError(
                "Stored public resource id is invalid".into(),
            ));
        }
        return Ok(id);
    }
    allocate_available(client, kind, storage_id, source_id, url, version).await
}

async fn allocate_available(
    client: &impl GenericClient,
    kind: ResourceKind,
    storage_id: &str,
    source_id: &str,
    url: &str,
    version: Option<&str>,
) -> Result<String, HtsError> {
    let table = kind.table();
    let identity = kind.postgres_identity_expression();
    let has_resources: bool = client
        .query_one("SELECT to_regclass('resources') IS NOT NULL", &[])
        .await
        .map_err(storage_error)?
        .get(0);
    let mut candidate = if valid_logical_id(source_id) {
        source_id.to_owned()
    } else {
        uuid::Uuid::new_v4().to_string()
    };
    loop {
        let taken: bool = client
            .query_one(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {identity} = $1 AND id != $2)"),
                &[&candidate, &storage_id],
            )
            .await
            .map_err(storage_error)?
            .get(0);
        let reserved = if has_resources {
            client
                .query_opt(
                    "SELECT data, is_deleted FROM resources WHERE tenant_id = $1 \
                     AND resource_type = $2 AND id = $3",
                    &[&SYSTEM_TENANT, &kind.resource_type(), &candidate],
                )
                .await
                .map_err(storage_error)?
                .is_some_and(|row| {
                    !matches_persisted_resource(&row.get::<_, Value>(0), row.get(1), url, version)
                })
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

pub(crate) async fn read(
    client: &impl GenericClient,
    kind: ResourceKind,
    id: &str,
) -> Result<Option<Value>, HtsError> {
    let table = kind.table();
    let key = kind.postgres_identity_expression();
    let sql = format!(
        "SELECT url, version, name, title, status, resource_json{} FROM {table} WHERE {key} = $1",
        if matches!(kind, ResourceKind::ValueSet) {
            ", compose_json"
        } else {
            ""
        }
    );
    client
        .query_opt(&sql, &[&id])
        .await
        .map_err(storage_error)?
        .map(|row| {
            ResourceRecord {
                url: row.get(0),
                version: row.get(1),
                name: row.get(2),
                title: row.get(3),
                status: row.get(4),
                resource: row.get(5),
                compose: if matches!(kind, ResourceKind::ValueSet) {
                    row.get(6)
                } else {
                    None
                },
            }
            .into_resource(kind, id)
        })
        .transpose()
}

/// Instance operations that only need a URL must not hydrate large CodeSystems.
pub(super) async fn url(
    client: &impl GenericClient,
    kind: ResourceKind,
    id: &str,
) -> Result<Option<String>, HtsError> {
    client
        .query_opt(
            &format!(
                "SELECT url FROM {} WHERE {} = $1",
                kind.table(),
                kind.postgres_identity_expression()
            ),
            &[&id],
        )
        .await
        .map_err(storage_error)
        .map(|row| row.map(|row| row.get(0)))
}
