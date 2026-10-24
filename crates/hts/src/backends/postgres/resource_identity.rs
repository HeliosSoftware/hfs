//! Allocation and indexed lookup of public terminology resource IDs.

use std::collections::{HashMap, HashSet};

use deadpool_postgres::GenericClient;
use helios_persistence::tenant::SYSTEM_TENANT;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::backends::resource_identity::{
    ImportIdentity, LegacyIdentitySource, MigrationOwners, ResourceKind, ResourceRecord,
    persisted_canonical_matches, valid_logical_id,
};
use crate::error::HtsError;

fn storage_error(error: tokio_postgres::Error) -> HtsError {
    HtsError::StorageError(super::error_chain(&error))
}

/// Identity keys hash into this many advisory locks. Every advisory lock held
/// takes a slot in PostgreSQL's shared lock table, which by default holds
/// `max_locks_per_transaction` (64) × (`max_connections` + prepared
/// transactions) entries. One lock per resource made a 5,000-resource
/// bootstrap batch fail with "out of shared memory". Bucketing caps any
/// transaction at this many locks, at the cost of unrelated identities
/// sharing a lock: a large import serializes with every REST write.
const IDENTITY_LOCK_BUCKETS: u64 = 256;

/// First key of the bucket range ("HTS_ID\0\0"), in the single-`bigint`
/// advisory key space, clear of `HTS_SCHEMA_LOCK` and the HFS migration lock.
const IDENTITY_LOCK_BASE: i64 = 0x4854_535f_4944_0000;

/// Only imports competing for a source ID or canonical version (or its lock
/// bucket) serialize.
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

async fn lock_keys(client: &impl GenericClient, keys: Vec<i64>) -> Result<(), HtsError> {
    let mut keys: Vec<i64> = keys.into_iter().map(lock_bucket).collect();
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

fn lock_bucket(key: i64) -> i64 {
    IDENTITY_LOCK_BASE + (key as u64 % IDENTITY_LOCK_BUCKETS) as i64
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

/// Identity decisions for one import or REST write, read in a few set-based
/// queries per resource kind instead of several round trips per resource.
///
/// The caller holds the identity locks of every resource it will write, so
/// the rows read here cannot change under it. Each decision is recorded, so a
/// later entry in the same bundle sees the rows that earlier entries wrote,
/// exactly as per-resource queries would.
pub(crate) struct ImportIdentities {
    has_resources: bool,
    code_systems: KindIdentities,
    value_sets: KindIdentities,
    concept_maps: KindIdentities,
}

#[derive(Default)]
struct KindIdentities {
    /// `(url, version or "")` → `(storage id, public id)` of its normalized row.
    canonical: HashMap<(String, String), (String, String)>,
    /// Canonicals whose public id is a live REST resource.
    rest_owners: HashMap<(String, String), String>,
    /// Storage ids known to exist, complete for every queried candidate.
    storage_ids: HashSet<String>,
    /// Public id → storage id, complete for every queried candidate.
    public_ids: HashMap<String, String>,
    /// REST resources with a candidate id: `None` when deleted.
    persisted: HashMap<String, Option<PersistedCanonical>>,
}

/// The string `url` and `version` of a live REST resource.
struct PersistedCanonical {
    url: Option<String>,
    version: Option<String>,
}

impl ImportIdentities {
    /// Read the identity state for every resource in `parsed`. `has_resources`
    /// says whether the REST resource table exists in this database.
    pub(crate) async fn load(
        client: &impl GenericClient,
        parsed: &crate::import::bundle_parser::ParsedBundle,
        has_resources: bool,
    ) -> Result<Self, HtsError> {
        let code_systems = parsed
            .code_systems
            .iter()
            .map(|cs| (cs.id.as_str(), cs.url.as_str(), cs.version.as_deref()))
            .collect::<Vec<_>>();
        let value_sets = parsed
            .value_sets
            .iter()
            .map(|vs| (vs.id.as_str(), vs.url.as_str(), vs.version.as_deref()))
            .collect::<Vec<_>>();
        // ConceptMap has one normalized row per URL, regardless of version.
        let concept_maps = parsed
            .concept_maps
            .iter()
            .map(|cm| (cm.id.as_str(), cm.url.as_str(), None))
            .collect::<Vec<_>>();
        Ok(Self {
            has_resources,
            code_systems: KindIdentities::load(
                client,
                ResourceKind::CodeSystem,
                &code_systems,
                has_resources,
            )
            .await?,
            value_sets: KindIdentities::load(
                client,
                ResourceKind::ValueSet,
                &value_sets,
                has_resources,
            )
            .await?,
            concept_maps: KindIdentities::load(
                client,
                ResourceKind::ConceptMap,
                &concept_maps,
                has_resources,
            )
            .await?,
        })
    }

    fn kind(&mut self, kind: ResourceKind) -> &mut KindIdentities {
        match kind {
            ResourceKind::CodeSystem => &mut self.code_systems,
            ResourceKind::ValueSet => &mut self.value_sets,
            ResourceKind::ConceptMap => &mut self.concept_maps,
        }
    }

    /// Bulk imports cannot replace a REST-managed resource without also
    /// updating its history. Strict CRUD normalization deliberately does not
    /// use this guard.
    pub(crate) fn check_import_owner(
        &mut self,
        kind: ResourceKind,
        source_id: &str,
        url: &str,
        version: Option<&str>,
    ) -> Result<(), HtsError> {
        let identities = self.kind(kind);
        if matches!(kind, ResourceKind::ConceptMap) && identities.persisted.contains_key(source_id)
        {
            return Err(HtsError::Conflict(format!(
                "ConceptMap/{source_id} is reserved by the REST resource store"
            )));
        }
        let version = if matches!(kind, ResourceKind::ConceptMap) {
            None
        } else {
            version
        };
        if let Some(id) = identities.rest_owners.get(&canonical_key(url, version)) {
            return Err(HtsError::Conflict(format!(
                "{}/{id} is managed through REST; import cannot replace its content",
                kind.resource_type(),
            )));
        }
        Ok(())
    }

    /// Choose the storage key and public id for a versioned resource and record
    /// them as written. A canonical that already has a row keeps both.
    ///
    /// The storage key prefers `<source id>|<version>`, then a UUID. The public
    /// id prefers the source id unless another row or a REST resource for a
    /// different canonical holds it, then a UUID.
    pub(crate) fn assign(
        &mut self,
        kind: ResourceKind,
        source_id: &str,
        url: &str,
        version: Option<&str>,
    ) -> Result<(String, String), HtsError> {
        let has_resources = self.has_resources;
        let identities = self.kind(kind);
        let key = canonical_key(url, version);
        if let Some((storage_id, public_id)) = identities.canonical.get(&key) {
            if !valid_logical_id(public_id) {
                return Err(HtsError::StorageError(
                    "Stored public resource id is invalid".into(),
                ));
            }
            return Ok((storage_id.clone(), public_id.clone()));
        }

        let preferred = crate::import::fhir_bundle::storage_id_for(source_id, version);
        let storage_id = if identities.storage_ids.contains(&preferred)
            || identities.public_ids.contains_key(&preferred)
        {
            uuid::Uuid::new_v4().to_string()
        } else {
            preferred
        };
        // Only source ids were read from the database. A fresh UUID cannot
        // collide in practice, so it is checked against this import alone.
        let mut public_id = if valid_logical_id(source_id) {
            source_id.to_owned()
        } else {
            uuid::Uuid::new_v4().to_string()
        };
        loop {
            let taken = identities
                .public_ids
                .get(&public_id)
                .is_some_and(|owner| owner != &storage_id);
            let reserved = has_resources
                && identities
                    .persisted
                    .get(&public_id)
                    .is_some_and(|resource| {
                        !resource.as_ref().is_some_and(|persisted| {
                            persisted_canonical_matches(
                                persisted.url.as_deref(),
                                persisted.version.as_deref(),
                                false,
                                url,
                                version,
                            )
                        })
                    });
            if !taken && !reserved {
                break;
            }
            public_id = uuid::Uuid::new_v4().to_string();
        }

        identities.storage_ids.insert(storage_id.clone());
        identities
            .public_ids
            .insert(public_id.clone(), storage_id.clone());
        // A live REST resource with this id now owns the canonical.
        if identities
            .persisted
            .get(&public_id)
            .is_some_and(Option::is_some)
        {
            identities
                .rest_owners
                .insert(key.clone(), public_id.clone());
        }
        identities
            .canonical
            .insert(key, (storage_id.clone(), public_id.clone()));
        Ok((storage_id, public_id))
    }
}

impl KindIdentities {
    /// `resources` holds `(source id, url, version)` for every resource of this
    /// kind in the write.
    async fn load(
        client: &impl GenericClient,
        kind: ResourceKind,
        resources: &[(&str, &str, Option<&str>)],
        has_resources: bool,
    ) -> Result<Self, HtsError> {
        let mut identities = Self::default();
        if resources.is_empty() {
            return Ok(identities);
        }
        let table = kind.table();
        let identity = kind.postgres_identity_expression();
        let version = if matches!(kind, ResourceKind::ConceptMap) {
            "''"
        } else {
            "COALESCE(n.version, '')"
        };
        let urls = resources
            .iter()
            .map(|(_, url, _)| (*url).to_owned())
            .collect::<Vec<_>>();
        let versions = resources
            .iter()
            .map(|(_, _, version)| version.unwrap_or("").to_owned())
            .collect::<Vec<_>>();

        // Existing rows for these canonicals, with any live REST owner.
        let owner = if has_resources {
            "(SELECT r.id FROM resources r WHERE r.tenant_id = $3 AND r.resource_type = $4 \
             AND r.id = n.public_id AND NOT r.is_deleted)"
        } else {
            "NULL::text"
        };
        let sql = format!(
            "SELECT n.url, n.version, n.id, n.public_id, {owner} FROM ( \
             SELECT DISTINCT n.url, {version} AS version, n.id, {identity} AS public_id \
             FROM {table} n JOIN unnest($1::text[], $2::text[]) AS c(url, version) \
             ON n.url = c.url AND {version} = c.version) AS n"
        );
        let rows = if has_resources {
            client
                .query(
                    &sql,
                    &[&urls, &versions, &SYSTEM_TENANT, &kind.resource_type()],
                )
                .await
        } else {
            client.query(&sql, &[&urls, &versions]).await
        }
        .map_err(storage_error)?;
        for row in rows {
            let key: (String, String) = (row.get(0), row.get(1));
            let storage_id: String = row.get(2);
            let public_id: String = row.get(3);
            if let Some(owner) = row.get::<_, Option<String>>(4) {
                identities.rest_owners.insert(key.clone(), owner);
            }
            identities.storage_ids.insert(storage_id.clone());
            identities
                .public_ids
                .insert(public_id.clone(), storage_id.clone());
            identities.canonical.insert(key, (storage_id, public_id));
        }

        // ConceptMap ids are their storage keys; nothing else to allocate.
        let source_ids = resources
            .iter()
            .map(|(id, _, _)| (*id).to_owned())
            .filter(|id| matches!(kind, ResourceKind::ConceptMap) || valid_logical_id(id))
            .collect::<Vec<_>>();
        if !matches!(kind, ResourceKind::ConceptMap) {
            let mut candidates = resources
                .iter()
                .map(|(id, _, version)| crate::import::fhir_bundle::storage_id_for(id, *version))
                .chain(source_ids.iter().cloned())
                .collect::<Vec<_>>();
            candidates.sort_unstable();
            candidates.dedup();
            for row in client
                .query(
                    &format!(
                        "SELECT id, {identity} FROM {table} \
                         WHERE id = ANY($1) OR {identity} = ANY($1)"
                    ),
                    &[&candidates],
                )
                .await
                .map_err(storage_error)?
            {
                let storage_id: String = row.get(0);
                identities.storage_ids.insert(storage_id.clone());
                identities.public_ids.insert(row.get(1), storage_id);
            }
        }

        // REST resources whose ids the source ids would take.
        if has_resources && !source_ids.is_empty() {
            for row in client
                .query(
                    "SELECT id, is_deleted, \
                     CASE WHEN jsonb_typeof(data->'url') = 'string' THEN data->>'url' END, \
                     CASE WHEN jsonb_typeof(data->'version') = 'string' \
                     THEN data->>'version' END \
                     FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND id = ANY($3)",
                    &[&SYSTEM_TENANT, &kind.resource_type(), &source_ids],
                )
                .await
                .map_err(storage_error)?
            {
                let deleted: bool = row.get(1);
                identities.persisted.insert(
                    row.get(0),
                    (!deleted).then(|| PersistedCanonical {
                        url: row.get(2),
                        version: row.get(3),
                    }),
                );
            }
        }
        Ok(identities)
    }
}

fn canonical_key(url: &str, version: Option<&str>) -> (String, String) {
    (url.to_owned(), version.unwrap_or("").to_owned())
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
