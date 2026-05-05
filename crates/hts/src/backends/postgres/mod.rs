//! PostgreSQL-backed terminology service backend.
//!
//! Gated by the `postgres` feature flag. Uses `deadpool-postgres` for async
//! connection pooling and `tokio-postgres` for query execution.

pub mod schema;

mod code_system;
mod concept_map;
mod value_set;

use async_trait::async_trait;
use deadpool_postgres::{Config, GenericClient, Pool, Runtime};
use tokio_postgres::NoTls;
use tracing::info;

use crate::error::HtsError;
use crate::import::bundle_parser::{self, ParsedCodeSystem, ParsedConceptMap, ParsedValueSet};
use crate::import::{BundleImportBackend, ImportStats};
use crate::traits::TerminologyMetadata;
use helios_persistence::tenant::TenantContext;

/// PostgreSQL-backed terminology service backend.
///
/// Wraps a `deadpool-postgres` connection pool. The schema is applied
/// automatically at construction time via [`schema::apply`].
///
/// All [`TerminologyBackend`] operations are implemented on this type,
/// enabling it to be placed directly in [`AppState`].
///
/// [`TerminologyBackend`]: crate::traits::TerminologyBackend
/// [`AppState`]: crate::state::AppState
#[derive(Clone)]
pub struct PostgresTerminologyBackend {
    pub(super) pool: Pool,
}

impl PostgresTerminologyBackend {
    /// Connect to PostgreSQL at `database_url` and apply the HTS schema.
    ///
    /// # Errors
    ///
    /// Returns [`HtsError::StorageError`] if the pool cannot be created or the
    /// schema migration fails.
    pub async fn new(database_url: &str) -> Result<Self, HtsError> {
        let pool = build_pool(database_url)?;

        {
            let client = pool.get().await.map_err(|e| {
                HtsError::StorageError(format!("Failed to acquire PG connection: {e}"))
            })?;

            schema::apply(&client)
                .await
                .map_err(|e| HtsError::StorageError(format!("Failed to apply HTS schema: {e}")))?;
        }

        info!(database_url, "PostgreSQL terminology backend initialized");

        Ok(Self { pool })
    }

    /// Borrow the underlying `deadpool-postgres` connection pool.
    pub fn pool(&self) -> &Pool {
        &self.pool
    }
}

fn build_pool(database_url: &str) -> Result<Pool, HtsError> {
    let mut cfg = Config::new();
    cfg.url = Some(database_url.to_string());
    cfg.pool = Some(deadpool_postgres::PoolConfig::new(16));
    cfg.create_pool(Some(Runtime::Tokio1), NoTls)
        .map_err(|e| HtsError::StorageError(format!("Failed to create PG pool: {e}")))
}

// ── TerminologyMetadata ────────────────────────────────────────────────────────

impl TerminologyMetadata for PostgresTerminologyBackend {
    fn backend_name(&self) -> &'static str {
        "postgres"
    }

    fn supported_systems(&self) -> Vec<String> {
        let pool = self.pool.clone();
        // Use block_in_place so we can safely block_on within a multi-threaded
        // tokio runtime (avoids "cannot start a runtime from within a runtime").
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let client = match pool.get().await {
                    Ok(c) => c,
                    Err(_) => return vec![],
                };
                match client
                    .query("SELECT url FROM code_systems ORDER BY url", &[])
                    .await
                {
                    Ok(rows) => rows.into_iter().map(|r| r.get::<_, String>(0)).collect(),
                    Err(_) => vec![],
                }
            })
        })
    }

    fn supports_subsumption(&self) -> bool {
        true
    }

    fn resource_url_by_id(&self, resource_type: &str, id: &str) -> Option<String> {
        let pool = self.pool.clone();
        let resource_type = resource_type.to_owned();
        let id = id.to_owned();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let client = pool.get().await.ok()?;
                match resource_type.as_str() {
                    "CodeSystem" => {
                        // Storage id may be the synthetic `<id>|<version>` form,
                        // so first try a direct hit, then fall back to matching
                        // the FHIR resource id captured in `resource_json` and
                        // pick the latest version.
                        if let Ok(rows) = client
                            .query("SELECT url FROM code_systems WHERE id = $1", &[&id])
                            .await
                        {
                            if let Some(row) = rows.into_iter().next() {
                                return Some(row.get::<_, String>(0));
                            }
                        }
                        let rows = client
                            .query(
                                "SELECT url FROM code_systems \
                                 WHERE (resource_json->>'id') = $1 \
                                 ORDER BY COALESCE(version, '') DESC \
                                 LIMIT 1",
                                &[&id],
                            )
                            .await
                            .ok()?;
                        rows.into_iter().next().map(|r| r.get::<_, String>(0))
                    }
                    "ValueSet" => {
                        let rows = client
                            .query("SELECT url FROM value_sets WHERE id = $1", &[&id])
                            .await
                            .ok()?;
                        rows.into_iter().next().map(|r| r.get::<_, String>(0))
                    }
                    "ConceptMap" => {
                        let rows = client
                            .query("SELECT url FROM concept_maps WHERE id = $1", &[&id])
                            .await
                            .ok()?;
                        rows.into_iter().next().map(|r| r.get::<_, String>(0))
                    }
                    _ => None,
                }
            })
        })
    }
}

// ── BundleImportBackend ────────────────────────────────────────────────────────

#[async_trait]
impl BundleImportBackend for PostgresTerminologyBackend {
    /// Parse a FHIR Bundle via the shared [`bundle_parser`] and write all
    /// contained resources into the PostgreSQL normalized tables.
    async fn import_bundle(
        &self,
        _ctx: &TenantContext,
        data: &[u8],
    ) -> Result<ImportStats, HtsError> {
        let parsed = bundle_parser::parse_bundle(data)?;

        let mut client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let tx = client
            .transaction()
            .await
            .map_err(|e| HtsError::StorageError(format!("Begin transaction: {e}")))?;

        let mut stats = ImportStats::default();
        stats.errors.extend(parsed.parse_errors.iter().cloned());

        for cs in &parsed.code_systems {
            if let Err(e) = write_code_system(&tx, cs, &mut stats).await {
                stats
                    .errors
                    .push(format!("CodeSystem '{}' import failed: {e}", cs.url));
            }
        }
        for vs in &parsed.value_sets {
            if let Err(e) = write_value_set(&tx, vs, &mut stats).await {
                stats
                    .errors
                    .push(format!("ValueSet '{}' import failed: {e}", vs.url));
            }
        }
        for cm in &parsed.concept_maps {
            if let Err(e) = write_concept_map(&tx, cm, &mut stats).await {
                stats
                    .errors
                    .push(format!("ConceptMap '{}' import failed: {e}", cm.url));
            }
        }

        tx.commit()
            .await
            .map_err(|e| HtsError::StorageError(format!("Commit transaction: {e}")))?;

        Ok(stats)
    }

    /// Delete all HTS normalized rows for the resource identified by `resource_url`.
    ///
    /// For CodeSystem: purges expansion cache entries that include codes from
    /// this system, then deletes the code_system row (FK CASCADE handles
    /// concepts, hierarchy, properties, designations).
    ///
    /// For ValueSet: deletes the value_set row (FK CASCADE handles expansions).
    ///
    /// For ConceptMap: deletes the concept_map row (FK CASCADE handles elements).
    async fn delete_normalized(
        &self,
        resource_type: &str,
        resource_url: &str,
    ) -> Result<(), HtsError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        match resource_type {
            "CodeSystem" => {
                // Invalidate any cached ValueSet expansions that include codes
                // from this system (not covered by FK cascade).
                client
                    .execute(
                        "DELETE FROM value_set_expansions WHERE system_url = $1",
                        &[&resource_url],
                    )
                    .await
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                // Delete the system row; ON DELETE CASCADE removes concepts,
                // concept_hierarchy, concept_properties, concept_designations.
                client
                    .execute("DELETE FROM code_systems WHERE url = $1", &[&resource_url])
                    .await
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
            }
            "ValueSet" => {
                // ON DELETE CASCADE removes value_set_expansions.
                client
                    .execute("DELETE FROM value_sets WHERE url = $1", &[&resource_url])
                    .await
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
            }
            "ConceptMap" => {
                // ON DELETE CASCADE removes concept_map_elements.
                client
                    .execute("DELETE FROM concept_maps WHERE url = $1", &[&resource_url])
                    .await
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
            }
            _ => {} // Unknown resource type — nothing to clean up.
        }

        Ok(())
    }
}

// ── PostgreSQL write helpers ───────────────────────────────────────────────────

fn utc_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

async fn write_code_system(
    client: &impl GenericClient,
    cs: &ParsedCodeSystem,
    stats: &mut ImportStats,
) -> Result<(), HtsError> {
    let resource_json = Some(cs.resource_json.clone());
    let now = utc_now();

    // Synthetic storage id encodes the version so multiple versions of the
    // same canonical URL coexist even when they share the same FHIR `id`
    // (tx-ecosystem `version/codesystem-version-1.json` + `-2.json` both ship
    // `"id":"version"`). See `crate::import::fhir_bundle::storage_id_for`.
    let storage_id = crate::import::fhir_bundle::storage_id_for(&cs.id, cs.version.as_deref());

    // Upsert keyed on (url, version). The composite UNIQUE index
    // `idx_code_systems_url_version` is the conflict arbiter; the legacy
    // `ON CONFLICT (url)` is no longer applicable now that two rows can share
    // a URL. `ON CONFLICT DO NOTHING` keeps a prior row in place; the UPDATE
    // below then refreshes its mutable columns by (url, version).
    client
        .execute(
            "INSERT INTO code_systems
             (id, url, version, name, title, status, content, resource_json, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
             ON CONFLICT DO NOTHING",
            &[
                &storage_id,
                &cs.url,
                &cs.version,
                &cs.name,
                &cs.title,
                &cs.status,
                &cs.content,
                &resource_json,
                &now,
            ],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let cs_rows = client
        .query(
            "UPDATE code_systems SET
               name          = $1,
               title         = $2,
               status        = $3,
               content       = $4,
               resource_json = $5,
               updated_at    = $6
             WHERE url = $7 AND COALESCE(version, '') = COALESCE($8, '')
             RETURNING id",
            &[
                &cs.name,
                &cs.title,
                &cs.status,
                &cs.content,
                &resource_json,
                &now,
                &cs.url,
                &cs.version,
            ],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let system_id: String = cs_rows
        .into_iter()
        .next()
        .ok_or_else(|| {
            HtsError::StorageError("UPDATE code_systems RETURNING id got no row".into())
        })?
        .get(0);

    for concept in &cs.concepts {
        let rows = client
            .query(
                "INSERT INTO concepts (system_id, code, display, definition)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (system_id, code) DO UPDATE SET
                   display    = EXCLUDED.display,
                   definition = EXCLUDED.definition
                 RETURNING id",
                &[
                    &system_id,
                    &concept.code,
                    &concept.display,
                    &concept.definition,
                ],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        let concept_id: i64 = rows
            .into_iter()
            .next()
            .ok_or_else(|| {
                HtsError::StorageError("INSERT concepts RETURNING id got no row".into())
            })?
            .get(0);

        stats.concepts += 1;

        // Hierarchy from nesting or "parent" property.
        if let Some(ref parent) = concept.parent_code {
            insert_hierarchy(client, &system_id, parent, &concept.code).await?;
        }

        for prop in &concept.properties {
            if prop.value.is_empty() {
                continue;
            }
            client
                .execute(
                    "INSERT INTO concept_properties
                     (concept_id, property, value_type, value)
                     VALUES ($1, $2, $3, $4)",
                    &[&concept_id, &prop.code, &prop.value_type, &prop.value],
                )
                .await
                .map_err(|e| HtsError::StorageError(e.to_string()))?;

            // Extra hierarchy edge from a "parent" property.
            if prop.is_parent_edge {
                if let Some(ref pv) = prop.parent_code_value {
                    insert_hierarchy(client, &system_id, pv, &concept.code).await?;
                }
            }
        }

        for desig in &concept.designations {
            client
                .execute(
                    "INSERT INTO concept_designations
                     (concept_id, language, use_system, use_code, value)
                     VALUES ($1, $2, $3, $4, $5)",
                    &[
                        &concept_id,
                        &desig.language,
                        &desig.use_system,
                        &desig.use_code,
                        &desig.value,
                    ],
                )
                .await
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
        }
    }

    stats.code_systems += 1;
    Ok(())
}

async fn insert_hierarchy(
    client: &impl GenericClient,
    system_id: &str,
    parent_code: &str,
    child_code: &str,
) -> Result<(), HtsError> {
    client
        .execute(
            "INSERT INTO concept_hierarchy (system_id, parent_code, child_code)
             VALUES ($1, $2, $3)
             ON CONFLICT DO NOTHING",
            &[&system_id, &parent_code, &child_code],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    Ok(())
}

async fn write_value_set(
    client: &impl GenericClient,
    vs: &ParsedValueSet,
    stats: &mut ImportStats,
) -> Result<(), HtsError> {
    let resource_json = Some(vs.resource_json.clone());
    let now = utc_now();

    // Invalidate expansion cache on re-import.
    let existing = client
        .query("SELECT id FROM value_sets WHERE url = $1", &[&vs.url])
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    if let Some(row) = existing.into_iter().next() {
        let existing_id: String = row.get(0);
        client
            .execute(
                "DELETE FROM value_set_expansions WHERE value_set_id = $1",
                &[&existing_id],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
    }

    // Two-step upsert — see `write_code_system` for why `ON CONFLICT (url)`
    // alone is not safe under concurrent imports that share `vs.id`.
    client
        .execute(
            "INSERT INTO value_sets
             (id, url, version, name, title, status, compose_json, resource_json, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
             ON CONFLICT DO NOTHING",
            &[
                &vs.id,
                &vs.url,
                &vs.version,
                &vs.name,
                &vs.title,
                &vs.status,
                &vs.compose_json,
                &resource_json,
                &now,
            ],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    client
        .execute(
            "UPDATE value_sets SET
               version       = $1,
               name          = $2,
               title         = $3,
               status        = $4,
               compose_json  = $5,
               resource_json = $6,
               updated_at    = $7
             WHERE url = $8",
            &[
                &vs.version,
                &vs.name,
                &vs.title,
                &vs.status,
                &vs.compose_json,
                &resource_json,
                &now,
                &vs.url,
            ],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    stats.value_sets += 1;
    Ok(())
}

async fn write_concept_map(
    client: &impl GenericClient,
    cm: &ParsedConceptMap,
    stats: &mut ImportStats,
) -> Result<(), HtsError> {
    let resource_json = Some(cm.resource_json.clone());
    let now = utc_now();

    // Two-step upsert — see `write_code_system` for why `ON CONFLICT (url)`
    // alone is not safe under concurrent imports that share `cm.id`.
    client
        .execute(
            "INSERT INTO concept_maps
             (id, url, version, name, title, source_uri, target_uri, status, resource_json, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT DO NOTHING",
            &[
                &cm.id,
                &cm.url,
                &cm.version,
                &cm.name,
                &cm.title,
                &cm.source_uri,
                &cm.target_uri,
                &cm.status,
                &resource_json,
                &now,
            ],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let rows = client
        .query(
            "UPDATE concept_maps SET
               version       = $1,
               name          = $2,
               title         = $3,
               source_uri    = $4,
               target_uri    = $5,
               status        = $6,
               resource_json = $7
             WHERE url = $8
             RETURNING id",
            &[
                &cm.version,
                &cm.name,
                &cm.title,
                &cm.source_uri,
                &cm.target_uri,
                &cm.status,
                &resource_json,
                &cm.url,
            ],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let map_id: String = rows
        .into_iter()
        .next()
        .ok_or_else(|| {
            HtsError::StorageError("UPDATE concept_maps RETURNING id got no row".into())
        })?
        .get(0);

    // Remove stale elements before re-inserting.
    client
        .execute(
            "DELETE FROM concept_map_elements WHERE map_id = $1",
            &[&map_id],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    for el in &cm.elements {
        client
            .execute(
                "INSERT INTO concept_map_elements
                 (map_id, source_system, source_code, target_system, target_code, equivalence)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT DO NOTHING",
                &[
                    &map_id,
                    &el.source_system,
                    &el.source_code,
                    &el.target_system,
                    &el.target_code,
                    &el.equivalence,
                ],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
    }

    stats.concept_maps += 1;
    Ok(())
}
