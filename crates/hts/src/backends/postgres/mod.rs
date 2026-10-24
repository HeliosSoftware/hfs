//! PostgreSQL-backed terminology service backend.
//!
//! Gated by the `postgres` feature flag. Uses `deadpool-postgres` for async
//! connection pooling and `tokio-postgres` for query execution.

pub mod schema;

mod code_system;
mod concept_map;
pub(crate) mod resource_identity;
mod value_set;

use async_trait::async_trait;
use deadpool_postgres::{Config, GenericClient, Pool, Runtime};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use tokio_postgres::NoTls;
use tracing::info;

use crate::error::HtsError;
use crate::import::bundle_parser::{self, ParsedCodeSystem, ParsedConceptMap, ParsedValueSet};
use crate::import::{BundleImportBackend, ImportStats};
use crate::string_search::{ResourceSearchRow, ResourceStringSearch, filter_rows};
use crate::traits::{TerminologyCaches, TerminologyMetadata};
use crate::types::{
    ExpansionContains, LookupResponse, ResourceSearchQuery, SubsumesResponse, TranslateResponse,
};
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

/// Count the resources in `table` matching `query`, ignoring `_count` and
/// `_offset`, so a searchset Bundle can report the full match count.
///
/// Uses the same predicates as the search paths: exact `url`/`version`/
/// `status` in SQL and, when present, FHIR string matching of `name`/`title`
/// over the metadata rows.
async fn count_resources<C>(
    client: &C,
    table: &str,
    query: &ResourceSearchQuery,
) -> Result<u64, HtsError>
where
    C: GenericClient + Sync,
{
    let search = ResourceStringSearch::new(query);
    let filter = "WHERE ($1::text IS NULL OR url = $1)
           AND ($2::text IS NULL OR version = $2)
           AND ($3::text IS NULL OR status = $3)";
    let params: [&(dyn tokio_postgres::types::ToSql + Sync); 3] =
        [&query.url, &query.version, &query.status];

    if search.is_empty() {
        let count: i64 = client
            .query_one(&format!("SELECT COUNT(*) FROM {table} {filter}"), &params)
            .await
            .map_err(|error| HtsError::StorageError(error_chain(&error)))?
            .get(0);
        return Ok(count.max(0) as u64);
    }

    let rows = client
        .query(
            &format!("SELECT name, title FROM {table} {filter}"),
            &params,
        )
        .await
        .map_err(|error| HtsError::StorageError(error_chain(&error)))?;
    Ok(rows
        .iter()
        .filter(|row| search.matches(row.get::<_, Option<&str>>(0), row.get::<_, Option<&str>>(1)))
        .count() as u64)
}

impl PostgresTerminologyBackend {
    /// Pooled wrapper around [`count_resources`] for the per-type search traits.
    pub(super) async fn count_search_matches(
        &self,
        table: &str,
        query: ResourceSearchQuery,
    ) -> Result<u64, HtsError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {}", error_chain(&e))))?;
        count_resources(&client, table, &query).await
    }
}

/// Search metadata using backend-neutral FHIR string matching, then hydrate
/// only the selected page. PostgreSQL search historically returns full
/// resources even for `_summary=true`, which this path preserves.
async fn search_resources(
    client: &mut deadpool_postgres::Object,
    table: &str,
    resource_type: &str,
    query: ResourceSearchQuery,
    search: ResourceStringSearch,
) -> Result<Vec<serde_json::Value>, HtsError> {
    // PostgreSQL's default READ COMMITTED level takes a fresh snapshot per
    // statement, so use REPEATABLE READ to keep metadata selection and JSON
    // hydration on one read snapshot without taking write locks.
    let transaction = client
        .build_transaction()
        .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
        .start()
        .await
        .map_err(|error| HtsError::StorageError(error_chain(&error)))?;
    let result =
        search_resources_in_transaction(&transaction, table, resource_type, query, search).await;
    match result {
        Ok(resources) => {
            transaction
                .commit()
                .await
                .map_err(|error| HtsError::StorageError(error_chain(&error)))?;
            Ok(resources)
        }
        Err(search_error) => match transaction.rollback().await {
            Ok(()) => Err(search_error),
            Err(rollback_error) => Err(HtsError::StorageError(format!(
                "{search_error}; failed to roll back search snapshot: {rollback_error}"
            ))),
        },
    }
}

async fn search_resources_in_transaction<C>(
    client: &C,
    table: &str,
    resource_type: &str,
    query: ResourceSearchQuery,
    search: ResourceStringSearch,
) -> Result<Vec<serde_json::Value>, HtsError>
where
    C: GenericClient + Sync,
{
    // Read large JSON bodies only after string filtering and pagination.
    let metadata_sql = format!(
        "SELECT id, url, version, name, title, status
         FROM {table}
         WHERE ($1::text IS NULL OR url = $1)
           AND ($2::text IS NULL OR version = $2)
           AND ($3::text IS NULL OR status = $3)
         ORDER BY created_at"
    );
    let rows = client
        .query(&metadata_sql, &[&query.url, &query.version, &query.status])
        .await
        .map_err(|error| HtsError::StorageError(error_chain(&error)))?
        .into_iter()
        .map(|row| ResourceSearchRow {
            id: row.get(0),
            #[cfg(feature = "sqlite")]
            public_id: None,
            url: row.get(1),
            version: row.get(2),
            name: row.get(3),
            title: row.get(4),
            status: row.get(5),
        })
        .collect::<Vec<_>>();
    let selected = filter_rows(rows, &query, &search);

    let stored_resources = if selected.is_empty() {
        HashMap::new()
    } else {
        let ids = selected
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>();
        let resource_sql = format!("SELECT id, resource_json FROM {table} WHERE id = ANY($1)");
        client
            .query(&resource_sql, &[&ids])
            .await
            .map_err(|error| HtsError::StorageError(error_chain(&error)))?
            .into_iter()
            .map(|row| {
                (
                    row.get::<_, String>(0),
                    row.get::<_, Option<serde_json::Value>>(1),
                )
            })
            .collect::<HashMap<_, _>>()
    };

    let mut results = Vec::with_capacity(selected.len());
    for row in selected {
        let mut resource = stored_resources
            .get(&row.id)
            .cloned()
            .flatten()
            .unwrap_or_else(|| {
                code_system::build_synthetic_resource(
                    resource_type,
                    &row.id,
                    &row.url,
                    row.version.as_deref(),
                    row.name.as_deref(),
                    row.title.as_deref(),
                    &row.status,
                )
            });
        // CodeSystem and ValueSet JSON already contains the assigned public ID.
        // ConceptMap continues to use its unchanged table identity.
        if table == "concept_maps"
            && let Some(object) = resource.as_object_mut()
        {
            object.insert("id".to_owned(), serde_json::Value::String(row.id));
        }
        results.push(resource);
    }
    Ok(results)
}

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
                HtsError::StorageError(format!(
                    "Failed to acquire PG connection: {}",
                    error_chain(&e)
                ))
            })?;

            schema::apply(&client).await.map_err(|e| {
                HtsError::StorageError(format!("Failed to apply HTS schema: {}", error_chain(&e)))
            })?;

            resource_identity::migrate(&mut client).await?;

            // Backfill `concept_closure` for any system that has hierarchy edges
            // but no closure rows yet. Idempotent and per-system, so existing
            // closures aren't rebuilt. For a fresh CI database, this is a no-op
            // because `hts import` already triggered the build via
            // `migrate_concept_closure_pg` at the end of the import phase.
            schema::migrate_concept_closure_pg(&mut client)
                .await
                .map_err(|e| {
                    HtsError::StorageError(format!(
                        "Failed to build concept_closure: {}",
                        error_chain(&e)
                    ))
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
    pub async fn rebuild_missing_closures(&self) -> Result<usize, HtsError> {
        let mut client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {}", error_chain(&e))))?;
        schema::migrate_concept_closure_pg(&mut client)
            .await
            .map_err(|e| {
                HtsError::StorageError(format!("concept_closure migration: {}", error_chain(&e)))
            })
    }
}

impl TerminologyCaches for PostgresTerminologyBackend {
    /// Drop every per-instance response cache plus the two process-global
    /// closure memos, so stale `$lookup` / `$translate` / `resolve_code_system`
    /// answers cannot shadow content that has just changed.
    ///
    /// Invoked after a successful `import_bundle` / `import_parsed`, from
    /// `delete_normalized`, and — since issue #304 — from the generic CRUD
    /// write seam. The CRUD create/update path reaches this twice (once through
    /// the importer, once through the seam); the method is idempotent, and a
    /// second clear of already-empty maps is not worth branching to avoid.
    ///
    /// # Totality is enforced by the compiler
    ///
    /// The destructuring below names every field with no `..` rest pattern, so
    /// adding a sixth cache to [`PostgresTerminologyBackend`] fails to compile
    /// (E0027) until the author names it here. This mirrors the SQLite backend,
    /// where the same shape closes a real six-of-sixteen eviction gap.
    fn invalidate_caches(&self) {
        // Exhaustive by construction: no `..`. See the doc comment above.
        let Self {
            // Not a cache: the connection pool itself.
            pool: _,
            inline_compose_cache,
            lookup_response_cache,
            cs_resolved_meta_cache,
            subsumes_response_cache,
            translate_response_cache,
        } = self;

        // Clear one `RwLock`-guarded map, ignoring a poisoned lock: a poisoned
        // cache is already unusable, and failing a write because some unrelated
        // reader panicked would be worse than leaving it be.
        macro_rules! clear {
            ($($cache:expr),* $(,)?) => {
                $(if let Ok(mut guard) = $cache.write() { guard.clear(); })*
            };
        }

        clear!(
            inline_compose_cache,
            lookup_response_cache,
            cs_resolved_meta_cache,
            subsumes_response_cache,
            translate_response_cache,
            // Iter 7k+: process-global closure COUNT(*) memo (see
            // backends/postgres/value_set.rs::CLOSURE_COUNT_CACHE).
            self::value_set::closure_count_cache(),
            // Iter 7n: process-global `?fhir_vs=isa/X` per-root prefix cache.
            self::value_set::root_prefix_cache(),
        );
    }
}

/// Render an error together with its full `source()` chain.
///
/// `tokio_postgres::Error`'s `Display` impl only prints the error *kind* —
/// for a server-side failure that is the bare string `"db error"`. The actual
/// SQLSTATE and `ERROR: …` message live in `source()`, so formatting such an
/// error with `{e}` discards the one piece of information needed to diagnose
/// it. Walking the chain preserves messages like
/// `db error: ERROR: could not resize shared memory segment …`.
pub(super) fn error_chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        out.push_str(": ");
        out.push_str(&s.to_string());
        src = s.source();
    }
    out
}

fn build_pool(database_url: &str) -> Result<Pool, HtsError> {
    let mut cfg = Config::new();
    cfg.url = Some(database_url.to_string());
    // 32 connections: warm requests bypass the pool entirely (see the
    // closure-page / inline-compose warm-hit fast paths in value_set.rs), so
    // the pool only gates *cold* expansions. 16 was too few to warm a
    // ~6.5k-entry benchmark pool within a 30s vu50 stage; 32 doubles the
    // cold-warming throughput while staying well under PG's
    // `max_connections=100`.
    cfg.pool = Some(deadpool_postgres::PoolConfig::new(32));
    cfg.create_pool(Some(Runtime::Tokio1), NoTls).map_err(|e| {
        HtsError::StorageError(format!("Failed to create PG pool: {}", error_chain(&e)))
    })
}

// ── TerminologyMetadata ────────────────────────────────────────────────────────

impl TerminologyMetadata for PostgresTerminologyBackend {
    fn resource_identity_by_id(
        &self,
        resource_type: &str,
        id: &str,
    ) -> Result<Option<crate::types::ResourceIdentity>, HtsError> {
        let Some(kind) =
            crate::backends::resource_identity::ResourceKind::from_resource_type(resource_type)
        else {
            return Ok(None);
        };
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let client = self
                    .pool
                    .get()
                    .await
                    .map_err(|error| HtsError::StorageError(error_chain(&error)))?;
                let row = client
                    .query_opt(
                        &format!(
                            "SELECT url, version, resource_json->>'language' FROM {} WHERE {} = $1",
                            kind.table(),
                            kind.postgres_identity_expression()
                        ),
                        &[&id],
                    )
                    .await
                    .map_err(|error| HtsError::StorageError(error_chain(&error)))?;
                Ok(row.map(|row| crate::types::ResourceIdentity {
                    id: id.to_owned(),
                    url: row.get(0),
                    version: row.get(1),
                    language: row.get(2),
                }))
            })
        })
    }

    fn resource_by_id(
        &self,
        resource_type: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>, HtsError> {
        let Some(kind) =
            crate::backends::resource_identity::ResourceKind::from_resource_type(resource_type)
        else {
            return Ok(None);
        };
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let client = self
                    .pool
                    .get()
                    .await
                    .map_err(|error| HtsError::StorageError(error_chain(&error)))?;
                resource_identity::read(&client, kind, id).await
            })
        })
    }

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
        let kind =
            crate::backends::resource_identity::ResourceKind::from_resource_type(resource_type)?;
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let client = self.pool.get().await.ok()?;
                resource_identity::url(&client, kind, id)
                    .await
                    .ok()
                    .flatten()
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
        ctx: &TenantContext,
        data: &[u8],
    ) -> Result<ImportStats, HtsError> {
        let parsed = bundle_parser::parse_bundle(data)?;
        self.import_parsed(ctx, parsed).await
    }

    /// Delegates to the inherent [`Self::rebuild_missing_closures`], then drops
    /// the response caches when anything was rebuilt: a `$subsumes` answered
    /// from the closure-less state may be sitting in `subsumes_response_cache`.
    async fn rebuild_missing_closures(&self) -> Result<usize, HtsError> {
        let rebuilt = PostgresTerminologyBackend::rebuild_missing_closures(self).await?;
        if rebuilt > 0 {
            self.invalidate_caches();
        }
        Ok(rebuilt)
    }

    /// Write an already-parsed bundle, skipping the JSON parse step.
    async fn import_parsed(
        &self,
        _ctx: &TenantContext,
        parsed: bundle_parser::ParsedBundle,
    ) -> Result<ImportStats, HtsError> {
        let mut client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {}", error_chain(&e))))?;

        // Before the transaction: decide which CodeSystems get their closure
        // rebuilt once the bundle commits. `write_code_system` deletes a
        // system's closure on every write, so a system left out of this list
        // answers `$subsumes` / `is-a` from an empty closure until the next
        // `migrate_concept_closure_pg` (issue #2128).
        //
        // A plain import rebuilds every system it writes. A batch of a chunked
        // bulk load (`defer_closure`, e.g. SNOMED RF2 chunks) only rebuilds
        // systems that had no concepts before it: rebuilding per chunk is
        // O(n²), so the full closure is built once via the end-of-CLI /
        // post-bootstrap `rebuild_missing_closures` call. Mirrors the SQLite
        // import_parsed_sync pattern.
        // Probed with EXISTS rather than COUNT(*): the answer only
        // distinguishes empty from non-empty, and a COUNT over an
        // ever-growing concept set would make each batch of a chunked bulk
        // load scan all previously imported rows.
        let mut systems_needing_closure: Vec<(String, Option<String>)> = Vec::new();
        for cs in &parsed.code_systems {
            if parsed.defer_closure {
                let row = client
                    .query_opt(
                        "SELECT EXISTS(
                             SELECT 1 FROM concepts c
                             JOIN code_systems s ON c.system_id = s.id
                             WHERE s.url = $1
                         )",
                        &[&cs.url],
                    )
                    .await
                    .map_err(|e| HtsError::StorageError(error_chain(&e)))?;
                let has_concepts: bool = row.map(|r| r.get(0)).unwrap_or(false);
                if has_concepts {
                    continue;
                }
            }
            systems_needing_closure.push((cs.url.clone(), cs.version.clone()));
        }

        // Ownership reads after an advisory-lock wait must see its previous
        // holder's commit even when the server defaults to REPEATABLE READ.
        let tx = client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::ReadCommitted)
            .start()
            .await
            .map_err(|e| {
                HtsError::StorageError(format!("Begin transaction: {}", error_chain(&e)))
            })?;

        resource_identity::lock_import(&tx, &parsed).await?;
        let has_resources: bool = tx
            .query_one("SELECT to_regclass('resources') IS NOT NULL", &[])
            .await
            .map_err(|error| HtsError::StorageError(error_chain(&error)))?
            .get(0);
        let mut identities =
            resource_identity::ImportIdentities::load(&tx, &parsed, has_resources).await?;

        let mut stats = ImportStats::default();
        stats.errors.extend(parsed.parse_errors.iter().cloned());

        for cs in &parsed.code_systems {
            if has_resources {
                if let Err(error) = identities.check_import_owner(
                    crate::backends::resource_identity::ResourceKind::CodeSystem,
                    &cs.id,
                    &cs.url,
                    cs.version.as_deref(),
                ) {
                    stats
                        .errors
                        .push(format!("CodeSystem '{}' import failed: {error}", cs.url));
                    systems_needing_closure
                        .retain(|(url, version)| url != &cs.url || version != &cs.version);
                    continue;
                }
            }
            if let Err(e) =
                write_code_system(&tx, cs, &mut identities, &mut stats, parsed.fresh_load).await
            {
                stats
                    .errors
                    .push(format!("CodeSystem '{}' import failed: {e}", cs.url));
            }
        }
        for vs in &parsed.value_sets {
            if has_resources {
                if let Err(error) = identities.check_import_owner(
                    crate::backends::resource_identity::ResourceKind::ValueSet,
                    &vs.id,
                    &vs.url,
                    vs.version.as_deref(),
                ) {
                    stats
                        .errors
                        .push(format!("ValueSet '{}' import failed: {error}", vs.url));
                    continue;
                }
            }
            if let Err(e) = write_value_set(&tx, vs, &mut identities, &mut stats).await {
                stats
                    .errors
                    .push(format!("ValueSet '{}' import failed: {e}", vs.url));
            }
        }
        for cm in &parsed.concept_maps {
            if has_resources {
                if let Err(error) = identities.check_import_owner(
                    crate::backends::resource_identity::ResourceKind::ConceptMap,
                    &cm.id,
                    &cm.url,
                    cm.version.as_deref(),
                ) {
                    stats
                        .errors
                        .push(format!("ConceptMap '{}' import failed: {error}", cm.url));
                    continue;
                }
            }
            if let Err(e) = write_concept_map(&tx, cm, &mut stats).await {
                stats
                    .errors
                    .push(format!("ConceptMap '{}' import failed: {e}", cm.url));
            }
        }

        tx.commit().await.map_err(|e| {
            HtsError::StorageError(format!("Commit transaction: {}", error_chain(&e)))
        })?;

        // Rebuild the closure of exactly the (url, version) rows this bundle
        // wrote. A URL may map to several `code_systems` rows when versions
        // share a canonical (e.g. the hl7.terminology@7.0.1 SNOMED stub sharing
        // `http://snomed.info/sct` with an RF2 import), and re-importing the
        // small one must not rebuild the large one.
        for (url, version) in &systems_needing_closure {
            let row = client
                .query_opt(
                    "SELECT id FROM code_systems
                     WHERE url = $1 AND COALESCE(version, '') = COALESCE($2, '')",
                    &[url, version],
                )
                .await
                .map_err(|e| HtsError::StorageError(error_chain(&e)))?;
            let Some(row) = row else { continue };
            let sid: String = row.get(0);
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
                        error = %error_chain(&e),
                        "Failed to build concept_closure after import"
                    );
                }
            }
        }

        // Invalidate per-instance response caches — they may now shadow
        // newly-imported codes.
        self.invalidate_caches();

        Ok(stats)
    }

    async fn code_system_has_concepts(
        &self,
        _ctx: &TenantContext,
        url: &str,
    ) -> Result<bool, HtsError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {}", error_chain(&e))))?;
        let row = client
            .query_one(
                "SELECT EXISTS(
                     SELECT 1 FROM concepts c
                     JOIN code_systems s ON c.system_id = s.id
                     WHERE s.url = $1
                 )",
                &[&url],
            )
            .await
            .map_err(|e| HtsError::StorageError(error_chain(&e)))?;
        Ok(row.get(0))
    }

    async fn delete_normalized(
        &self,
        resource_type: &str,
        resource_url: &str,
    ) -> Result<(), HtsError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {}", error_chain(&e))))?;

        match resource_type {
            "CodeSystem" => {
                // Invalidate any cached ValueSet expansions that include codes
                // from this system (not covered by FK cascade).
                invalidate_expansion_cache_for_system(&client, resource_url).await?;
                // Delete the system row; ON DELETE CASCADE removes concepts,
                // concept_hierarchy, concept_properties, concept_designations.
                client
                    .execute("DELETE FROM code_systems WHERE url = $1", &[&resource_url])
                    .await
                    .map_err(|e| HtsError::StorageError(error_chain(&e)))?;
            }
            "ValueSet" => {
                // ON DELETE CASCADE removes value_set_expansions.
                client
                    .execute("DELETE FROM value_sets WHERE url = $1", &[&resource_url])
                    .await
                    .map_err(|e| HtsError::StorageError(error_chain(&e)))?;
            }
            "ConceptMap" => {
                // ON DELETE CASCADE removes concept_map_elements.
                client
                    .execute("DELETE FROM concept_maps WHERE url = $1", &[&resource_url])
                    .await
                    .map_err(|e| HtsError::StorageError(error_chain(&e)))?;
            }
            _ => {} // Unknown resource type — nothing to clean up.
        }

        // Invalidate per-instance response caches — they may reference the
        // now-deleted resource.
        self.invalidate_caches();

        Ok(())
    }
}

/// Strict normalized replacement inside the active persistence transaction.
pub(crate) async fn write_resource_in_transaction(
    client: &deadpool_postgres::Client,
    resource_type: &str,
    content: &serde_json::Value,
    replace_id: Option<&str>,
) -> Result<(), HtsError> {
    let parsed = crate::import::parse_crud_resource(resource_type, content)?;
    let kind = crate::backends::resource_identity::ResourceKind::from_resource_type(resource_type)
        .ok_or_else(|| {
            HtsError::InvalidRequest(format!("Unsupported resource type: {resource_type}"))
        })?;
    let id = content["id"]
        .as_str()
        .ok_or_else(|| HtsError::InvalidRequest("Resource id is required".into()))?;
    // The CRUD adapter acquired all old/new identity locks before raw writes.
    let version_column = if matches!(
        kind,
        crate::backends::resource_identity::ResourceKind::ConceptMap
    ) {
        "''"
    } else {
        "version"
    };
    let canonical_version = if matches!(
        kind,
        crate::backends::resource_identity::ResourceKind::ConceptMap
    ) {
        None
    } else {
        content.get("version").and_then(serde_json::Value::as_str)
    };
    let owner = client
        .query_opt(
            &format!(
                "SELECT {} FROM {} WHERE url = $1 AND COALESCE({version_column}, '') = \
                 COALESCE($2, '')",
                kind.postgres_identity_expression(),
                kind.table()
            ),
            &[&content["url"].as_str(), &canonical_version],
        )
        .await
        .map_err(|error| HtsError::StorageError(error_chain(&error)))?;
    if owner.is_some_and(|row| row.get::<_, String>(0) != id) {
        return Err(HtsError::InvalidRequest(
            "Canonical URL and version already belong to another resource id".into(),
        ));
    }
    let public_owner = client
        .query_opt(
            &format!(
                "SELECT url, version FROM {} WHERE {} = $1",
                kind.table(),
                kind.postgres_identity_expression()
            ),
            &[&id],
        )
        .await
        .map_err(|error| HtsError::StorageError(error_chain(&error)))?;
    if replace_id.is_none() && public_owner.is_some() {
        return Err(HtsError::InvalidRequest(
            "Resource id already identifies an imported resource".into(),
        ));
    }
    if let Some(id) = replace_id {
        delete_resource_in_transaction(client, kind, id).await?;
    }
    // Read after the delete: a replacement may reuse the identity it frees.
    let mut identities = resource_identity::ImportIdentities::load(client, &parsed, true).await?;
    let mut stats = ImportStats::default();
    for cs in &parsed.code_systems {
        write_code_system(client, cs, &mut identities, &mut stats, false).await?;
        let row = client
            .query_one(
                "SELECT id FROM code_systems WHERE url = $1 AND \
                 COALESCE(version, '') = COALESCE($2, '')",
                &[&cs.url, &cs.version],
            )
            .await
            .map_err(|error| HtsError::StorageError(error_chain(&error)))?;
        let sid: String = row.get(0);
        schema::build_concept_closure_in_transaction(&***client, &sid)
            .await
            .map_err(|error| HtsError::StorageError(error_chain(&error)))?;
    }
    for vs in &parsed.value_sets {
        write_value_set(client, vs, &mut identities, &mut stats).await?;
    }
    for cm in &parsed.concept_maps {
        write_concept_map(client, cm, &mut stats).await?;
    }
    let assigned: String = client
        .query_one(
            &format!(
                "SELECT {} FROM {} WHERE url = $1 AND COALESCE({version_column}, '') = \
                 COALESCE($2, '')",
                kind.postgres_identity_expression(),
                kind.table()
            ),
            &[&content["url"].as_str(), &canonical_version],
        )
        .await
        .map_err(|error| HtsError::StorageError(error_chain(&error)))?
        .get(0);
    if assigned != id {
        return Err(HtsError::InvalidRequest(
            "Resource id is reserved by another terminology resource".into(),
        ));
    }
    Ok(())
}

/// Exact public-ID deletion with all derived rows in the same transaction.
pub(crate) async fn delete_resource_in_transaction(
    client: &impl GenericClient,
    kind: crate::backends::resource_identity::ResourceKind,
    id: &str,
) -> Result<(), HtsError> {
    // Delete the parent first, matching schema and import lock order.
    // RETURNING also avoids a separate read and a later lock upgrade.
    let deleted = client
        .query_opt(
            &format!(
                "DELETE FROM {} WHERE {} = $1 RETURNING url",
                kind.table(),
                kind.postgres_identity_expression()
            ),
            &[&id],
        )
        .await
        .map_err(|error| HtsError::StorageError(error_chain(&error)))?;
    if let Some(row) = deleted
        && matches!(
            kind,
            crate::backends::resource_identity::ResourceKind::CodeSystem
        )
    {
        let url: String = row.get(0);
        invalidate_expansion_cache_for_system(client, &url).await?;
    }
    Ok(())
}

// ── PostgreSQL write helpers ───────────────────────────────────────────────────

/// Invalidate complete expansions containing this CodeSystem's concepts.
/// A remaining row would otherwise mark a partial expansion as fully cached.
async fn invalidate_expansion_cache_for_system(
    client: &impl GenericClient,
    system_url: &str,
) -> Result<(), HtsError> {
    client
        .execute(
            "DELETE FROM value_set_expansions WHERE value_set_id IN (
                 SELECT value_set_id FROM value_set_expansions WHERE system_url = $1
             )",
            &[&system_url],
        )
        .await
        .map_err(|error| HtsError::StorageError(error_chain(&error)))?;
    Ok(())
}

fn utc_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

async fn write_code_system(
    client: &impl GenericClient,
    cs: &ParsedCodeSystem,
    identities: &mut resource_identity::ImportIdentities,
    stats: &mut ImportStats,
    // When `true` the caller guarantees this code system had no concepts before
    // the import session, so the per-concept delete-before-reinsert of
    // properties/designations is skipped (fast path for first-time bulk loads).
    // See [`crate::import::bundle_parser::ParsedBundle::fresh_load`].
    skip_child_delete: bool,
) -> Result<(), HtsError> {
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
    // [`resource_identity::ImportIdentities::assign`] makes the choice from
    // rows read once for the whole write.
    let (storage_id, public_id) = identities.assign(
        crate::backends::resource_identity::ResourceKind::CodeSystem,
        &cs.id,
        &cs.url,
        cs.version.as_deref(),
    )?;
    let mut stored_resource = cs.resource_json.clone();
    stored_resource["id"] = serde_json::Value::String(public_id);
    let resource_json = Some(stored_resource);

    // Upsert keyed on (url, version). The composite UNIQUE index
    // `idx_code_systems_url_version` is the conflict arbiter; the legacy
    // `ON CONFLICT (url)` is no longer applicable now that two rows can share
    // a URL. `ON CONFLICT DO NOTHING` keeps a prior row in place; the UPDATE
    // below then refreshes its mutable columns by (url, version).
    client
        .execute(
            "INSERT INTO code_systems
             (id, url, version, name, title, status, content, resource_json, created_at, updated_at,
              authority_rank)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9, $10)
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
                &cs.authority_rank,
            ],
        )
        .await
        .map_err(|e| HtsError::StorageError(error_chain(&e)))?;

    // authority_rank keeps the strongest claim ever asserted for this row; the
    // COALESCE sentinel (9 — above any real rank) means a row that predates the
    // column ("never claimed") is overwritten by the first source to claim it.
    // See the SQLite twin in `import/fhir_bundle.rs::write_code_system`.
    let cs_update = format!(
        "UPDATE code_systems SET
           name           = $1,
           title          = $2,
           status         = $3,
           content        = $4,
           resource_json  = $5,
           updated_at     = $6,
           authority_rank = LEAST(COALESCE(authority_rank, {unclaimed}), $9)
         WHERE url = $7 AND COALESCE(version, '') = COALESCE($8, '')
         RETURNING id",
        unclaimed = bundle_parser::AUTHORITY_UNCLAIMED
    );
    let cs_rows = client
        .query(
            &cs_update,
            &[
                &cs.name,
                &cs.title,
                &cs.status,
                &cs.content,
                &resource_json,
                &now,
                &cs.url,
                &cs.version,
                &cs.authority_rank,
            ],
        )
        .await
        .map_err(|e| HtsError::StorageError(error_chain(&e)))?;

    let system_id: String = cs_rows
        .into_iter()
        .next()
        .ok_or_else(|| {
            HtsError::StorageError("UPDATE code_systems RETURNING id got no row".into())
        })?
        .get(0);

    // Codes whose old parent edges this write has already cleared. A bundle
    // can list the same code more than once, and the second occurrence must
    // not wipe the edges the first one just inserted.
    let mut edges_cleared: HashSet<&str> = HashSet::new();

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
            .map_err(|e| HtsError::StorageError(error_chain(&e)))?;

        let concept_id: i64 = rows
            .into_iter()
            .next()
            .ok_or_else(|| {
                HtsError::StorageError("INSERT concepts RETURNING id got no row".into())
            })?
            .get(0);

        stats.concepts += 1;

        // Replace this concept's parent edges rather than adding to them, so a
        // re-import that moves a concept to a new parent (or reverses an edge)
        // does not leave the old edge behind, which can form a cycle (#2129).
        // A concept and all its parents arrive in one batch, so this is safe
        // across chunked loads. Skipped on a fresh load (nothing to delete).
        // Mirrors import/fhir_bundle.rs (SQLite).
        if !skip_child_delete && edges_cleared.insert(concept.code.as_str()) {
            client
                .execute(
                    "DELETE FROM concept_hierarchy WHERE system_id = $1 AND child_code = $2",
                    &[&system_id, &concept.code],
                )
                .await
                .map_err(|e| HtsError::StorageError(error_chain(&e)))?;
        }

        // Hierarchy from nesting or "parent" property.
        if let Some(ref parent) = concept.parent_code {
            insert_hierarchy(client, &system_id, parent, &concept.code).await?;
        }

        // Replace any prior child rows for this concept so a re-import fully
        // supersedes the previous set rather than duplicating rows or leaving
        // filtered-out languages behind (the concept row itself is upserted, so
        // its id is stable). A concept that now yields zero designations after a
        // narrower language filter must still lose its old ones. Stub
        // "content=not-present" imports carry an empty concepts array, so this
        // loop never runs for them. Skipped on a fresh load (nothing to
        // delete). Mirrors import/fhir_bundle.rs (SQLite).
        if !skip_child_delete {
            client
                .execute(
                    "DELETE FROM concept_properties WHERE concept_id = $1",
                    &[&concept_id],
                )
                .await
                .map_err(|e| HtsError::StorageError(error_chain(&e)))?;
            client
                .execute(
                    "DELETE FROM concept_designations WHERE concept_id = $1",
                    &[&concept_id],
                )
                .await
                .map_err(|e| HtsError::StorageError(error_chain(&e)))?;
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
                .map_err(|e| HtsError::StorageError(error_chain(&e)))?;

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
                .map_err(|e| HtsError::StorageError(error_chain(&e)))?;
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
        .map_err(|e| HtsError::StorageError(error_chain(&e)))?;

    invalidate_expansion_cache_for_system(client, &cs.url).await?;
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
        .map_err(|e| HtsError::StorageError(error_chain(&e)))?;
    Ok(())
}

async fn write_value_set(
    client: &impl GenericClient,
    vs: &ParsedValueSet,
    identities: &mut resource_identity::ImportIdentities,
    stats: &mut ImportStats,
) -> Result<(), HtsError> {
    let now = utc_now();

    // Invalidate expansion cache on re-import.
    let existing = client
        .query("SELECT id FROM value_sets WHERE url = $1", &[&vs.url])
        .await
        .map_err(|e| HtsError::StorageError(error_chain(&e)))?;

    if let Some(row) = existing.into_iter().next() {
        let existing_id: String = row.get(0);
        client
            .execute(
                "DELETE FROM value_set_expansions WHERE value_set_id = $1",
                &[&existing_id],
            )
            .await
            .map_err(|e| HtsError::StorageError(error_chain(&e)))?;
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
    // [`resource_identity::ImportIdentities::assign`] makes the choice from
    // rows read once for the whole write.
    let (storage_id, public_id) = identities.assign(
        crate::backends::resource_identity::ResourceKind::ValueSet,
        &vs.id,
        &vs.url,
        vs.version.as_deref(),
    )?;
    let mut stored_resource = vs.resource_json.clone();
    stored_resource["id"] = serde_json::Value::String(public_id);
    let resource_json = Some(stored_resource);

    client
        .execute(
            "INSERT INTO value_sets
             (id, url, version, name, title, status, compose_json, resource_json, created_at, updated_at,
              authority_rank)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9, $10)
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
                &vs.authority_rank,
            ],
        )
        .await
        .map_err(|e| HtsError::StorageError(error_chain(&e)))?;

    // See `write_code_system` for the LEAST/COALESCE-sentinel rationale.
    let vs_update = format!(
        "UPDATE value_sets SET
           name           = $1,
           title          = $2,
           status         = $3,
           compose_json   = $4,
           resource_json  = $5,
           updated_at     = $6,
           authority_rank = LEAST(COALESCE(authority_rank, {unclaimed}), $9)
         WHERE url = $7 AND COALESCE(version, '') = COALESCE($8, '')",
        unclaimed = bundle_parser::AUTHORITY_UNCLAIMED
    );
    client
        .execute(
            &vs_update,
            &[
                &vs.name,
                &vs.title,
                &vs.status,
                &vs.compose_json,
                &resource_json,
                &now,
                &vs.url,
                &vs.version,
                &vs.authority_rank,
            ],
        )
        .await
        .map_err(|e| HtsError::StorageError(error_chain(&e)))?;

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
        .map_err(|e| HtsError::StorageError(error_chain(&e)))?;

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
        .map_err(|e| HtsError::StorageError(error_chain(&e)))?;

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
        .map_err(|e| HtsError::StorageError(error_chain(&e)))?;

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
            .map_err(|e| HtsError::StorageError(error_chain(&e)))?;
    }

    stats.concept_maps += 1;
    Ok(())
}
