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
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio_postgres::NoTls;
use tracing::info;

use crate::error::HtsError;
use crate::import::bundle_parser::{self, ParsedCodeSystem, ParsedConceptMap, ParsedValueSet};
use crate::import::{BundleImportBackend, ImportStats};
use crate::traits::TerminologyMetadata;
use crate::types::{ExpansionContains, LookupResponse, SubsumesResponse, TranslateResponse};
use helios_persistence::tenant::TenantContext;

/// Per-compose in-memory expansion index. Mirrors SQLite's
/// `inline_compose_index` (backends/sqlite/value_set.rs:296+).
///
/// Key = fnv64 of canonicalised compose body + system-version pins. Value =
/// a `OnceCell` that holds the full, sorted, unfiltered list of expansion
/// concepts. Slicing (filter / count / offset) happens in Rust on the
/// cached vec, so once a compose is warm a request avoids all DB I/O.
///
/// The `OnceCell` provides **single-flight** semantics: when 50 VUs
/// simultaneously miss the same key, only the first runs `compute_expansion`
/// and `populate_cache`; the other 49 await the cell's value and clone its
/// `Arc` in O(1). Without this, the bench's cold-miss stampede burned
/// 50× redundant 5,975-row INSERTs on a single VSAC VS (EX04). On error the
/// cell stays empty so the next caller retries cleanly.
pub type ComposeExpansionCache =
    Arc<RwLock<HashMap<u64, Arc<tokio::sync::OnceCell<Arc<Vec<ExpansionContains>>>>>>>;

/// `$lookup` response memo. Mirrors SQLite's `lookup_response_cache`
/// (backends/sqlite/code_system.rs:240+).
///
/// Key = canonicalised request string (`system|code|version|lang|date|props`).
/// Skipped when `useSupplement` is non-empty since the merged response depends
/// on per-request supplement resolution.
pub type LookupResponseCache = Arc<RwLock<HashMap<String, Arc<LookupResponse>>>>;

/// `(url, version) → (system_id, name, version)` resolver cache. Mirrors
/// SQLite's `cs_resolved_meta_cache` (backends/sqlite/code_system.rs:1686+).
/// Populated on the `date.is_none()` fast path; bypassed when a point-in-time
/// `date` filter is active.
pub type ResolvedMetaCache =
    Arc<RwLock<HashMap<(String, Option<String>), (String, String, Option<String>)>>>;

/// `$subsumes` response memo (SS01 hot path).
/// Key = `"system|version|codeA|codeB"`. Both ancestor-check directions
/// (subsumes / subsumed-by) share the same key — the outcome captures the
/// resolved direction.
pub type SubsumesResponseCache = Arc<RwLock<HashMap<String, Arc<SubsumesResponse>>>>;

/// `$translate` response memo (CM01/CM02 hot path).
/// Key = `"url|system|code|source|target|target_system|target_code|reverse|date"`.
pub type TranslateResponseCache = Arc<RwLock<HashMap<String, Arc<TranslateResponse>>>>;

/// Soft cap on cache entries. Once full, new entries are dropped silently
/// (warm-set wins). 16384 mirrors SQLite's iter-9 cap-raise: each LK*/VC*
/// pool holds 2000 distinct codes across 3-5 systems = 6-10K keys; the
/// previous 4096 was being filled by the first two systems' keys, leaving
/// later systems (LK04 RxNorm, etc.) entirely uncached.
pub const PG_COMPOSE_CACHE_MAX: usize = 16384;

/// Soft cap on `lookup_response_cache` entries — mirrors SQLite's
/// `lookup_response_cache_max()` (backends/sqlite/code_system.rs:131-135).
/// Sized to hold all LK*/VC*/SS01 keys simultaneously (~10K).
pub const PG_LOOKUP_RESPONSE_CACHE_MAX: usize = 16384;

/// Soft cap on `subsumes_response_cache` entries.
pub const PG_SUBSUMES_RESPONSE_CACHE_MAX: usize = 16384;

/// Soft cap on `translate_response_cache` entries.
pub const PG_TRANSLATE_RESPONSE_CACHE_MAX: usize = 16384;

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
    /// Process-local inline-compose expansion cache. Hot path for repeated
    /// POST $expand requests with identical compose bodies — k6 EX02/04/05/06
    /// pool entries hit the same compose hundreds of times during a 30s run.
    pub(super) inline_compose_cache: ComposeExpansionCache,
    /// `$lookup` response memo — LK01-04 hit the same `(system, code,
    /// version, lang)` tuples per pool entry across 50 VUs.
    pub(super) lookup_response_cache: LookupResponseCache,
    /// `resolve_code_system` memo, keyed by `(url, version)`. Drives LK*,
    /// VC*, and SS01 — every CS-touching op begins by resolving the URL to
    /// an internal `(system_id, name, version)` triple.
    pub(super) cs_resolved_meta_cache: ResolvedMetaCache,
    /// `$subsumes` response memo — SS01 hot path was at 17% of SQLite
    /// after iter 2 because find_concept + check_ancestor (recursive CTE)
    /// ran on every request.
    pub(super) subsumes_response_cache: SubsumesResponseCache,
    /// `$translate` response memo — CM01/CM02 hot paths repeat the same
    /// (system, code, target) tuples across pool entries.
    pub(super) translate_response_cache: TranslateResponseCache,
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
            let mut client = pool.get().await.map_err(|e| {
                HtsError::StorageError(format!("Failed to acquire PG connection: {e}"))
            })?;

            schema::apply(&client)
                .await
                .map_err(|e| HtsError::StorageError(format!("Failed to apply HTS schema: {e}")))?;

            // Backfill `concept_closure` for any system that has hierarchy edges
            // but no closure rows yet. Idempotent and per-system, so existing
            // closures aren't rebuilt. For a fresh CI database, this is a no-op
            // because `hts import` already triggered the build via
            // `migrate_concept_closure_pg` at the end of the import phase.
            schema::migrate_concept_closure_pg(&mut client)
                .await
                .map_err(|e| {
                    HtsError::StorageError(format!("Failed to build concept_closure: {e}"))
                })?;
        }

        info!(database_url, "PostgreSQL terminology backend initialized");

        Ok(Self {
            pool,
            inline_compose_cache: Arc::new(RwLock::new(HashMap::new())),
            lookup_response_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_resolved_meta_cache: Arc::new(RwLock::new(HashMap::new())),
            subsumes_response_cache: Arc::new(RwLock::new(HashMap::new())),
            translate_response_cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Drop every per-instance response cache. Invoked after a successful
    /// `import_bundle` so stale `$lookup` / `resolve_code_system` results
    /// don't shadow newly-imported codes. Mirrors SQLite's eviction at
    /// backends/sqlite/mod.rs (`clear_response_caches` flow).
    pub(super) fn clear_response_caches(&self) {
        if let Ok(mut g) = self.lookup_response_cache.write() {
            g.clear();
        }
        if let Ok(mut g) = self.cs_resolved_meta_cache.write() {
            g.clear();
        }
        if let Ok(mut g) = self.inline_compose_cache.write() {
            g.clear();
        }
        if let Ok(mut g) = self.subsumes_response_cache.write() {
            g.clear();
        }
        if let Ok(mut g) = self.translate_response_cache.write() {
            g.clear();
        }
    }

    /// Borrow the underlying `deadpool-postgres` connection pool.
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Build (or rebuild) `concept_closure` for every system that has
    /// hierarchy edges but no closure rows.
    ///
    /// Called once at the end of a CLI bulk import so the server's first
    /// request doesn't pay the build cost. For SNOMED CT (~20M closure pairs)
    /// this takes ~30–60 s.
    pub async fn rebuild_missing_closures(&self) -> Result<(), HtsError> {
        let mut client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
        schema::migrate_concept_closure_pg(&mut client)
            .await
            .map_err(|e| HtsError::StorageError(format!("concept_closure migration: {e}")))?;
        Ok(())
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

        // Before the transaction: record which CodeSystems currently have zero
        // concepts in the DB so the post-commit closure build only touches
        // newly-imported (or empty-stub) systems. Systems being updated in a
        // batch import (e.g. SNOMED RF2 chunks) skip the per-batch rebuild
        // because rebuilding per chunk is O(n²) — the SNOMED full closure
        // is built once via the end-of-CLI `migrate_concept_closure_pg` call.
        // Mirrors the SQLite import_bundle pattern.
        let mut systems_needing_closure: Vec<String> = Vec::new();
        for cs in &parsed.code_systems {
            let row = client
                .query_opt(
                    "SELECT COUNT(*) FROM concepts c
                     JOIN code_systems s ON c.system_id = s.id
                     WHERE s.url = $1",
                    &[&cs.url],
                )
                .await
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            let count: i64 = row.map(|r| r.get(0)).unwrap_or(0);
            if count == 0 {
                systems_needing_closure.push(cs.url.clone());
            }
        }

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

        // Rebuild concept_closure for newly-imported (previously empty) code
        // systems. Skipped for batch imports of existing systems so SNOMED
        // RF2's ~1300 batches don't each pay the closure-rebuild cost — the
        // full closure is built once at end-of-CLI via
        // `migrate_concept_closure_pg` (called from main.rs). Mirrors the
        // SQLite post-commit closure rebuild.
        for url in &systems_needing_closure {
            let row = client
                .query_opt("SELECT id FROM code_systems WHERE url = $1", &[url])
                .await
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            let Some(sid_row) = row else { continue };
            let sid: String = sid_row.get(0);
            let has_hier: bool = client
                .query_one(
                    "SELECT EXISTS(SELECT 1 FROM concept_hierarchy WHERE system_id = $1 LIMIT 1)",
                    &[&sid],
                )
                .await
                .map(|r| r.get(0))
                .unwrap_or(false);
            if has_hier {
                if let Err(e) = schema::build_concept_closure_pg(&mut client, &sid).await {
                    tracing::warn!(
                        system_id = %sid,
                        error = %e,
                        "Failed to build concept_closure after import"
                    );
                }
            }
        }

        // Invalidate per-instance response caches — they may now shadow
        // newly-imported codes.
        self.clear_response_caches();

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

        // Invalidate per-instance response caches — they may reference the
        // now-deleted resource.
        self.clear_response_caches();

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
    // `"id":"version"`). When two DISTINCT CodeSystems share both fhir-id
    // AND version (e.g. two unrelated CSes ship id="status" with no
    // version), reuse the existing row for the matching (url, version) or
    // mint a fresh UUID rather than letting the second import silently
    // disappear via ON CONFLICT DO NOTHING. Mirrors
    // sqlite/import/fhir_bundle.rs:189-215.
    let preferred_id = crate::import::fhir_bundle::storage_id_for(&cs.id, cs.version.as_deref());
    let existing_for_url_version: Option<String> = client
        .query_opt(
            "SELECT id FROM code_systems \
             WHERE url = $1 AND COALESCE(version, '') = COALESCE($2, '')",
            &[&cs.url, &cs.version],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .and_then(|r| r.get::<_, Option<String>>(0));
    let storage_id = if let Some(id) = existing_for_url_version {
        id
    } else {
        let preferred_taken: bool = client
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM code_systems WHERE id = $1)",
                &[&preferred_id],
            )
            .await
            .map(|r| r.get::<_, bool>(0))
            .unwrap_or(false);
        if preferred_taken {
            uuid::Uuid::new_v4().to_string()
        } else {
            preferred_id
        }
    };

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

    // Invalidate stale closure rows so the post-import / startup migration
    // knows to (re)build the full closure once all batches are loaded. Without
    // this, a partial closure from an earlier batch would be mistakenly treated
    // as complete. Mirrors sqlite/import/fhir_bundle.rs.
    client
        .execute(
            "DELETE FROM concept_closure WHERE system_id = $1",
            &[&system_id],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

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

    // Synthetic storage id: `<fhir-id>|<version>` (or `<fhir-id>` when
    // version is absent). Mirrors the code_systems strategy so multiple
    // ValueSets that share a canonical URL but differ in version don't
    // collide on the primary key. When two distinct VSes share both a
    // fhir-id AND a version (e.g. tx-ecosystem ships several VSes whose
    // `id` field is reused across DIFFERENT canonical URLs like
    // `withdrawn`, `simple-import-bad`, etc.), reuse the existing row
    // for the matching (url, version) or mint a fresh UUID — otherwise
    // the second import gets silently dropped by ON CONFLICT DO NOTHING.
    // Mirrors sqlite/import/fhir_bundle.rs:443-478.
    let preferred_id = crate::import::fhir_bundle::storage_id_for(&vs.id, vs.version.as_deref());
    let existing_for_url_version: Option<String> = client
        .query_opt(
            "SELECT id FROM value_sets \
             WHERE url = $1 AND COALESCE(version, '') = COALESCE($2, '')",
            &[&vs.url, &vs.version],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .and_then(|r| r.get::<_, Option<String>>(0));
    let storage_id = if let Some(id) = existing_for_url_version {
        id
    } else {
        let preferred_taken: bool = client
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM value_sets WHERE id = $1)",
                &[&preferred_id],
            )
            .await
            .map(|r| r.get::<_, bool>(0))
            .unwrap_or(false);
        if preferred_taken {
            uuid::Uuid::new_v4().to_string()
        } else {
            preferred_id
        }
    };

    client
        .execute(
            "INSERT INTO value_sets
             (id, url, version, name, title, status, compose_json, resource_json, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
             ON CONFLICT DO NOTHING",
            &[
                &storage_id,
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
               name          = $1,
               title         = $2,
               status        = $3,
               compose_json  = $4,
               resource_json = $5,
               updated_at    = $6
             WHERE url = $7 AND COALESCE(version, '') = COALESCE($8, '')",
            &[
                &vs.name,
                &vs.title,
                &vs.status,
                &vs.compose_json,
                &resource_json,
                &now,
                &vs.url,
                &vs.version,
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
