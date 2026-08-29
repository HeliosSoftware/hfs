//! SQLite implementation of [`TerminologyBackend`].
//!
//! The backend owns an r2d2 connection pool and applies the schema from
//! [`schema::SCHEMA`] on construction.  Trait methods are split across private
//! sub-modules (`code_system`, `value_set`, `concept_map`) plus the
//! [`TerminologyMetadata`] impl in this file, so each source file stays
//! focused on one FHIR resource type.
//!
//! Synchronous `rusqlite` calls are dispatched through
//! [`tokio::task::spawn_blocking`] to avoid stalling the async runtime.
//!
//! [`TerminologyBackend`]: crate::traits::TerminologyBackend

pub mod schema;

mod code_system;
mod concept_map;
mod value_set;

pub(crate) use code_system::invalidate_cs_language_cache;
pub(crate) use value_set::invalidate_cs_id_cache;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use tracing::info;

use crate::error::HtsError;
use crate::import::{BundleImportBackend, ImportStats};
use crate::string_search::{ResourceSearchRow, ResourceStringSearch, filter_rows};
use crate::traits::{TerminologyCaches, TerminologyMetadata};
use crate::types::{LookupResponse, ResourceSearchQuery, ValidateCodeResponse};
use helios_persistence::tenant::TenantContext;

// ─── Per-instance cache type aliases (see field docs on SqliteTerminologyBackend) ──
pub(crate) type PropCodesMap = HashMap<String, Arc<Vec<String>>>;
pub(crate) type ConceptFlagMap = HashMap<(String, String), bool>;
pub(crate) type ResolvedMeta = (String, String, Option<String>);
pub(crate) type ResolvedMetaMap = HashMap<(String, Option<String>), ResolvedMeta>;
pub(crate) type StringOptionMap = HashMap<String, Option<String>>;
pub(crate) type BoolMap = HashMap<String, bool>;
pub(crate) type LookupResponseMap = HashMap<String, Arc<LookupResponse>>;
pub(crate) type ValidateCodeResponseMap = HashMap<String, Arc<ValidateCodeResponse>>;

/// Search metadata using backend-neutral FHIR string matching, then hydrate
/// only the selected page. Exact URL/version/status predicates stay in SQL.
fn search_resources(
    conn: &mut rusqlite::Connection,
    table: &str,
    resource_type: &str,
    query: ResourceSearchQuery,
    search: ResourceStringSearch,
    synthetic_summary: bool,
) -> Result<Vec<serde_json::Value>, HtsError> {
    // A deferred transaction acquires no write lock. It pins the read snapshot
    // so metadata selection and subsequent JSON hydration cannot observe
    // different resource revisions.
    let transaction = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
        .map_err(|error| HtsError::StorageError(error.to_string()))?;
    let result = search_resources_in_transaction(
        &transaction,
        table,
        resource_type,
        query,
        search,
        synthetic_summary,
    );
    match result {
        Ok(resources) => {
            transaction
                .commit()
                .map_err(|error| HtsError::StorageError(error.to_string()))?;
            Ok(resources)
        }
        Err(search_error) => match transaction.rollback() {
            Ok(()) => Err(search_error),
            Err(rollback_error) => Err(HtsError::StorageError(format!(
                "{search_error}; failed to roll back search snapshot: {rollback_error}"
            ))),
        },
    }
}

fn search_resources_in_transaction(
    conn: &rusqlite::Connection,
    table: &str,
    resource_type: &str,
    query: ResourceSearchQuery,
    search: ResourceStringSearch,
    synthetic_summary: bool,
) -> Result<Vec<serde_json::Value>, HtsError> {
    let metadata_sql = format!(
        "SELECT id, url, version, name, title, status
         FROM {table}
         WHERE (?1 IS NULL OR url = ?1)
           AND (?2 IS NULL OR version = ?2)
           AND (?3 IS NULL OR status = ?3)
         ORDER BY created_at"
    );
    let mut statement = conn
        .prepare(&metadata_sql)
        .map_err(|error| HtsError::StorageError(error.to_string()))?;
    let rows = statement
        .query_map(
            rusqlite::params![
                query.url.as_deref(),
                query.version.as_deref(),
                query.status.as_deref()
            ],
            |row| {
                Ok(ResourceSearchRow {
                    id: row.get(0)?,
                    url: row.get(1)?,
                    version: row.get(2)?,
                    name: row.get(3)?,
                    title: row.get(4)?,
                    status: row.get(5)?,
                })
            },
        )
        .map_err(|error| HtsError::StorageError(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| HtsError::StorageError(error.to_string()))?;
    let selected = filter_rows(rows, &query, &search);

    let use_synthetic = synthetic_summary && query.summary.as_deref() == Some("true");
    let mut stored_resources = HashMap::<String, Option<String>>::new();
    if !use_synthetic {
        // Stay below SQLite's common 999-variable ceiling regardless of an
        // arbitrary u32 `_count`. Chunking also avoids loading unselected JSON.
        for chunk in selected.chunks(900) {
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(", ");
            let resource_sql =
                format!("SELECT id, resource_json FROM {table} WHERE id IN ({placeholders})");
            let mut resource_statement = conn
                .prepare(&resource_sql)
                .map_err(|error| HtsError::StorageError(error.to_string()))?;
            let resource_rows = resource_statement
                .query_map(
                    rusqlite::params_from_iter(chunk.iter().map(|row| row.id.as_str())),
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .map_err(|error| HtsError::StorageError(error.to_string()))?;
            for resource in resource_rows {
                let (id, json) =
                    resource.map_err(|error| HtsError::StorageError(error.to_string()))?;
                stored_resources.insert(id, json);
            }
        }
    }

    let mut results = Vec::with_capacity(selected.len());
    for row in selected {
        let resource = stored_resources
            .remove(&row.id)
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
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
        results.push(resource);
    }
    Ok(results)
}

/// Insert `(key, value)` into a bounded cache, evicting one existing entry when
/// the map is already at `max` capacity.
///
/// The response and concept-flag caches are read under a shared (`read`) lock
/// on the `$lookup` / `$validate-code` hot paths, so we deliberately do **not**
/// track per-entry recency: true LRU would require taking a write lock on every
/// read to bump a recency counter, serializing the very lookups the cache
/// exists to speed up. Instead, when the map is full we evict a single
/// arbitrary entry (std `HashMap` iteration order is randomized per process, so
/// this is effectively random replacement) and admit the new key.
///
/// This replaces the previous "insert only while `len < max`" policy, which
/// *froze* each cache once it filled: the first `max` distinct keys held their
/// slots permanently and every later key missed forever, making the cache
/// useless for any working set larger than `max` (e.g. diverse-code `$lookup`
/// traffic against a 600 K-concept SNOMED system). Random replacement keeps the
/// same memory ceiling while letting hot keys re-enter the cache.
pub(crate) fn bounded_cache_insert<K, V, S>(
    map: &mut HashMap<K, V, S>,
    key: K,
    value: V,
    max: usize,
) where
    K: std::hash::Hash + Eq + Clone,
    S: std::hash::BuildHasher,
{
    if max == 0 {
        return;
    }
    // Only evict when inserting a genuinely new key would exceed the bound;
    // overwriting an existing key does not grow the map.
    if map.len() >= max && !map.contains_key(&key) {
        // Clone one key to end the immutable borrow before removing. The keys
        // here are small (`String` / `(String, String)`) and this only runs on
        // a cache miss, so the clone is negligible next to the SQLite work that
        // produced `value`.
        if let Some(evict) = map.keys().next().cloned() {
            map.remove(&evict);
        }
    }
    map.insert(key, value);
}

/// Shared in-memory index for text-filtered implicit ValueSet expansions.
///
/// Keyed by the implicit ValueSet URL.  Values are the combined entry list
/// plus a trigram inverted index that enables O(k) filtered queries.
pub(crate) type ImplicitIndex = Arc<RwLock<HashMap<String, Arc<value_set::ImplicitConceptIndex>>>>;

/// Shared in-memory index for inline-compose ValueSet expansions.
///
/// Keyed by the DB-level cache key (`"inline-compose:{fnv64-hex}"`).  After
/// the first expansion for a given compose body the full result set is loaded
/// into this map so that subsequent requests bypass `spawn_blocking` entirely,
/// eliminating r2d2 pool contention under high concurrency (EX06 optimisation).
pub(crate) type InlineComposeIndex =
    Arc<RwLock<HashMap<String, Arc<value_set::ImplicitConceptIndex>>>>;

/// Shared in-memory index for property-filtered inline ValueSet expansions.
///
/// Keyed by `"prop-result:{fnv64-hex}"` of the compose body.  Populated on
/// the first expansion of a compose that uses property= + hierarchy filters
/// (EX08 pattern).  Stores the FULL property-matched concept set (no text
/// filter); subsequent requests apply the text filter in Rust, eliminating
/// `spawn_blocking` and r2d2 pool contention under high concurrency.
pub(crate) type PropertyResultCache =
    Arc<RwLock<HashMap<String, Arc<value_set::ImplicitConceptIndex>>>>;

/// Shared in-memory corpus index for plain multi-system text-filter expansions.
///
/// Keyed by `"plain-fts:{fnv64-hex}"` of the compose body.  Populated on the
/// first filtered expansion where every include is a plain full-system include
/// (no compose filters, no explicit concept list, no nested valueSets) — the
/// EX07 pattern.  Stores ALL concepts from the included systems (no text
/// filter); subsequent requests for the same compose (any filter term) apply
/// the text filter in Rust via the trigram index, eliminating `spawn_blocking`
/// and r2d2 pool contention under high concurrency.
pub(crate) type PlainFtsCache = Arc<RwLock<HashMap<String, Arc<value_set::ImplicitConceptIndex>>>>;

/// SQLite-backed terminology service backend.
///
/// Wraps an r2d2 connection pool. Schema migrations are applied automatically
/// at construction time via [`schema::apply`].
///
/// All [`TerminologyBackend`] operations are implemented on this type, enabling
/// it to be placed directly in [`AppState`].
///
/// [`TerminologyBackend`]: crate::traits::TerminologyBackend
/// [`AppState`]: crate::state::AppState
#[derive(Clone)]
pub struct SqliteTerminologyBackend {
    pool: Pool<SqliteConnectionManager>,
    /// In-process concept index for text-filtered implicit ValueSet expansions.
    ///
    /// Keyed by the implicit ValueSet URL; values are pre-sorted slices of
    /// all concepts for that URL loaded from `implicit_expansion_cache`.
    /// Filtering is done with pure-Rust `contains()` instead of SQLite FTS5,
    /// eliminating pool contention at high concurrency (EX03 optimisation).
    pub(crate) implicit_index: ImplicitIndex,

    /// Deduplication guard for background index-population threads.
    ///
    /// When the BFS fast path serves an EX03 request it spawns exactly one
    /// `std::thread` per URL to populate `implicit_expansion_cache` and then
    /// build the in-memory `implicit_index`.  This set prevents multiple
    /// concurrent VUs from each spawning their own thread for the same URL.
    pub(crate) bg_index_pending: Arc<Mutex<HashSet<String>>>,

    /// In-process concept index for inline-compose ValueSet expansions.
    ///
    /// Keyed by `"inline-compose:{fnv64-hex}"` — the same key used by the
    /// DB-level `implicit_expansion_cache` table.  Populated during the first
    /// expansion for each unique compose body and pre-warmed at startup from any
    /// existing cache rows, so that repeated requests (e.g. k6 benchmark VUs)
    /// bypass `spawn_blocking` entirely once the index is warm.
    pub(crate) inline_compose_index: InlineComposeIndex,

    /// In-process concept index for property-filtered inline ValueSet expansions.
    ///
    /// Keyed by `"prop-result:{fnv64-hex}"` of the compose body.  Populated on
    /// the first expansion that has property= + hierarchy compose filters (e.g.
    /// EX08: SNOMED finding-site + is-a + text).  The cached set contains ALL
    /// property-matched concepts (no text filter); subsequent requests with the
    /// same compose but a different text filter apply the filter in Rust,
    /// bypassing `spawn_blocking` and r2d2 pool contention entirely.
    pub(crate) property_result_cache: PropertyResultCache,

    /// In-process corpus index for plain multi-system text-filter expansions.
    ///
    /// Keyed by `"plain-fts:{fnv64-hex}"` of the compose body.  Populated on
    /// the first filtered expansion where every include is a plain full-system
    /// include (EX07 pattern: multi-system text filter, no compose filters).
    /// The cached set contains ALL concepts from the included systems; any
    /// subsequent request for the same compose body (regardless of text filter)
    /// is served entirely from process memory via the trigram index, bypassing
    /// `spawn_blocking` and r2d2 pool contention.
    pub(crate) plain_fts_cache: PlainFtsCache,

    // ── Per-instance perf caches (iter3) ─────────────────────────────────────
    //
    // These were originally global `OnceLock<RwLock<HashMap<...>>>` statics,
    // but cargo runs tests in parallel across threads in the same binary;
    // distinct in-memory backends sharing the globals leaked entries across
    // tests (e.g. `is_concept_abstract` for `(http://example.org/cs, A)`
    // returning a stale `true` from another test). Per-instance caches make
    // every backend self-contained.
    /// CodeSystem URL → local property codes mapping for `notSelectable`.
    pub(crate) cs_abstract_prop_cache: Arc<RwLock<PropCodesMap>>,
    /// CodeSystem URL → local property codes mapping for `inactive`.
    pub(crate) cs_inactive_prop_cache: Arc<RwLock<PropCodesMap>>,
    /// `(system_url, code) → bool` result of `is_concept_abstract`.
    pub(crate) cs_concept_abstract_cache: Arc<RwLock<ConceptFlagMap>>,
    /// `(system_url, code) → bool` result of `is_concept_inactive`.
    pub(crate) cs_concept_inactive_cache: Arc<RwLock<ConceptFlagMap>>,
    /// CodeSystem URL → highest stored version (used in error messages).
    pub(crate) cs_version_for_msg_cache: Arc<RwLock<StringOptionMap>>,
    /// CodeSystem URL → `content` column value (e.g. `Some("fragment")`).
    pub(crate) cs_content_cache: Arc<RwLock<StringOptionMap>>,
    /// ValueSet URL → highest stored version (used in error messages).
    pub(crate) vs_version_for_msg_cache: Arc<RwLock<StringOptionMap>>,
    /// `(url, version) → resolved meta` for `resolve_code_system`.
    pub(crate) cs_resolved_meta_cache: Arc<RwLock<ResolvedMetaMap>>,
    /// Cache key → assembled `LookupResponse` for `$lookup`.
    pub(crate) lookup_response_cache: Arc<RwLock<LookupResponseMap>>,
    /// Cache key → assembled `ValidateCodeResponse` for `ValueSet/$validate-code`.
    ///
    /// Same shape and motivation as `lookup_response_cache`: VC01-03 hammer the
    /// same `(url, system, code)` tuples across 50 VUs and the entire validate
    /// pipeline (resolve VS, expand, search expansion, version-mismatch checks,
    /// finish_validate_code_response) is pure-functional in the request → so a
    /// per-instance memo skips spawn_blocking, pool acquisition, and the
    /// resolve+expand SQL roundtrips. Invalidated by
    /// [`TerminologyCaches::invalidate_caches`] on bundle import and on every
    /// CRUD write.
    ///
    /// It previously carried the justification "no explicit invalidation
    /// required because hot-path bench loops reuse one backend, and tests
    /// instantiate fresh ones per case". That reasoning holds for the benchmark
    /// and for tests, and fails for a server: a `PUT` that adds or retires a
    /// code left this memo answering `$validate-code` from the superseded
    /// content indefinitely (issue #304).
    pub(crate) validate_code_response_cache: Arc<RwLock<ValidateCodeResponseMap>>,
    /// CodeSystem URL → highest stored version, used by `$validate-code` for
    /// `x-unknown-system` detection (`build_validate_response_async`). Same
    /// shape as `cs_language_cache` but per-instance to keep tests isolated.
    /// Invalidated alongside the in-memory implicit/inline indexes when
    /// `import_bundle` succeeds.
    pub(crate) cs_version_for_url_cache: Arc<RwLock<StringOptionMap>>,
    /// CodeSystem URL → existence flag, used by the per-coding existence checks
    /// in `process_vs_validate_code_inner` (VC03 hot path).  Replaces a
    /// `search(url=Some(sys), count=Some(1))` round-trip that loaded
    /// `resource_json` only to discard everything except `is_empty()`.
    /// Invalidated alongside the other per-instance caches on `import_bundle`.
    pub(crate) cs_exists_cache: Arc<RwLock<BoolMap>>,
}

impl SqliteTerminologyBackend {
    /// Open (or create) the SQLite database at `db_path` and apply the schema.
    ///
    /// - Enables WAL journal mode for improved write concurrency.
    /// - Enables foreign key enforcement.
    /// - Creates all HTS tables if they do not yet exist.
    ///
    /// # Errors
    ///
    /// Returns [`HtsError::StorageError`] if the pool cannot be created or the
    /// schema migration fails.
    pub fn new(db_path: &str) -> Result<Self, HtsError> {
        Self::new_inner(db_path, true)
    }

    /// Like [`Self::new`] but defers the expensive `concepts_fts` prebuild and
    /// in-memory index pre-warm.
    ///
    /// The server startup path uses this so that bootstrap re-imports run
    /// *before* the FTS index is materialized: building FTS in `new` would
    /// (a) pay the 10–25 s prebuild on data that bootstrap is about to change,
    /// and (b) leave the index stale for any re-imported system. Callers MUST
    /// invoke [`Self::finalize_after_bootstrap`] before serving requests.
    pub fn new_without_fts_prebuild(db_path: &str) -> Result<Self, HtsError> {
        Self::new_inner(db_path, false)
    }

    fn new_inner(db_path: &str, prebuild_fts: bool) -> Result<Self, HtsError> {
        // Apply per-connection pragmas on every new connection from the pool.
        // journal_mode is file-level (WAL persists); the rest are per-connection.
        // `synchronous=NORMAL` is crash-safe under WAL (the journal mode set
        // at bootstrap below) and avoids an fsync on every commit — a
        // meaningful speed-up for bulk imports that commit one transaction
        // per batch of ~500 concepts.
        // PRAGMAs tuned for the benchmark sequence (EX01-03 populate large
        // implicit-cache rows that thrash the page cache; EX04/EX07/EX08 then
        // suffer from cold reads and WAL traversal).
        //   cache_size=-200000        — 200 MB per connection (was 32 MB)
        //   mmap_size=2 GiB           — read paths bypass syscalls (was 256 MB)
        //   wal_autocheckpoint=5000   — let WAL grow before checkpoint, reducing
        //                               reader/writer contention during bg writes
        let manager = SqliteConnectionManager::file(db_path).with_init(|conn| {
            conn.execute_batch(
                "PRAGMA foreign_keys=ON;
                 PRAGMA cache_size=-200000;
                 PRAGMA temp_store=MEMORY;
                 PRAGMA busy_timeout=30000;
                 PRAGMA synchronous=NORMAL;
                 PRAGMA mmap_size=2147483648;
                 PRAGMA wal_autocheckpoint=5000;",
            )
        });

        // Create and WAL-initialize the database file with a single connection
        // before building the pool. `Pool::build()` eagerly opens `max_size`
        // connections at once; against a not-yet-existent file that is a
        // concurrent-create race which logs spurious SQLITE_IOERR ("disk I/O
        // error") from r2d2's connection-retry path. Once the file exists in
        // WAL mode, the concurrent opens below are safe.
        rusqlite::Connection::open(db_path)
            .and_then(|c| c.execute_batch("PRAGMA journal_mode=WAL;"))
            .map_err(|e| {
                HtsError::StorageError(format!("Failed to initialize SQLite database: {e}"))
            })?;

        // Pool size sized for the benchmark's 50-VU sustained load plus
        // background implicit-cache populate threads (uncapped fan-out across
        // ~100 distinct `?fhir_vs=isa/<X>` URLs in EX01). At max_size=20,
        // tokio request tasks block on `pool.get()` once the bg writers are
        // active, which dominates EX04/EX07/EX08 latency in sequential
        // benchmarks. WAL handles concurrent readers fine; only one writer
        // at a time is enforced by SQLite, so adding read slack is safe.
        let pool = Pool::builder()
            .max_size(64)
            .build(manager)
            .map_err(|e| HtsError::StorageError(format!("Failed to create SQLite pool: {e}")))?;

        // Declare early so the init block can pre-warm the in-memory indexes.
        let implicit_index: ImplicitIndex = Arc::new(RwLock::new(HashMap::new()));
        let inline_compose_index: InlineComposeIndex = Arc::new(RwLock::new(HashMap::new()));
        let property_result_cache: PropertyResultCache = Arc::new(RwLock::new(HashMap::new()));
        let plain_fts_cache: PlainFtsCache = Arc::new(RwLock::new(HashMap::new()));
        // Bootstrap: apply WAL + schema on a single connection.
        {
            let mut conn = pool.get().map_err(|e| {
                HtsError::StorageError(format!("Failed to acquire connection for init: {e}"))
            })?;

            conn.execute_batch("PRAGMA journal_mode=WAL;")
                .map_err(|e| {
                    HtsError::StorageError(format!("Failed to configure SQLite pragmas: {e}"))
                })?;

            schema::apply(&conn)
                .map_err(|e| HtsError::StorageError(format!("Failed to apply HTS schema: {e}")))?;
            schema::migrate_search_columns(&conn).map_err(|e| {
                HtsError::StorageError(format!("Failed to apply search column migration: {e}"))
            })?;
            schema::migrate_concept_closure(&conn).map_err(|e| {
                HtsError::StorageError(format!("Failed to apply concept closure migration: {e}"))
            })?;
            // Drop legacy column-level UNIQUE on code_systems.url so multi-version
            // CodeSystems can share a canonical URL. Idempotent — no-op when the
            // table was already created without that constraint.
            schema::migrate_code_systems_drop_url_unique(&mut conn).map_err(|e| {
                HtsError::StorageError(format!(
                    "Failed to drop legacy code_systems.url UNIQUE: {e}"
                ))
            })?;
            schema::migrate_value_sets_drop_url_unique(&mut conn).map_err(|e| {
                HtsError::StorageError(format!("Failed to drop legacy value_sets.url UNIQUE: {e}"))
            })?;
            schema::migrate_bootstrap_imports_columns(&conn).map_err(|e| {
                HtsError::StorageError(format!(
                    "Failed to apply bootstrap_imports column migration: {e}"
                ))
            })?;
            // MUST run after the drop_url_unique rebuilds above: those recreate
            // code_systems / value_sets from a fixed column list and would drop
            // `authority_rank` if it had been added first.
            schema::migrate_authority_rank(&conn).map_err(|e| {
                HtsError::StorageError(format!("Failed to apply authority_rank migration: {e}"))
            })?;

            // FTS prebuild + index pre-warm. Skipped when `prebuild_fts` is
            // false (server startup path): bootstrap re-imports run first and
            // [`Self::finalize_after_bootstrap`] performs these steps once on
            // the final data. The implicit_expansion_cache is intentionally kept
            // across restarts: populate_implicit_cache runs inside a BEGIN
            // EXCLUSIVE transaction, so SQLite rolls back any partial write on
            // crash — the entries are always fully committed or fully absent.
            // Persisting the cache means repeated server restarts (e.g. benchmark
            // reruns) start warm rather than cold. Cache entries are invalidated
            // per-code-system when new data is imported (see
            // fhir_bundle::write_code_system).
            if prebuild_fts {
                // Incrementally build the concepts_fts trigram index for any
                // system not already tracked in concepts_fts_built (issue #295).
                // No blanket wipe: per-system invalidation on (re)import keeps
                // the tracker authoritative, so this only builds what is
                // genuinely missing and is a no-op on a warm reopen. For a first
                // load of large systems (SNOMED 638K, LOINC 181K) it still
                // tokenises the full corpus once. Runs synchronously before the
                // server accepts requests.
                let built = value_set::prebuild_concepts_fts(&conn);
                if built > 0 {
                    // Update query-planner statistics only when a build ran;
                    // stats persist on disk across reopens.
                    let _ = conn.execute_batch(
                        "ANALYZE concept_hierarchy; ANALYZE concepts; ANALYZE concept_closure; \
                         ANALYZE concept_properties; ANALYZE concept_designations; \
                         ANALYZE code_systems; ANALYZE value_sets; ANALYZE concept_maps;",
                    );
                }

                // Pre-warm the in-memory concept index from any implicit-expansion
                // entries that are already persisted in implicit_expansion_cache.
                // On a warm restart (e.g. repeated benchmark runs) this lets the
                // async hot path in expand() fire immediately without waiting for
                // a background build thread.  No-op on first run (empty cache).
                value_set::prebuild_implicit_index(&conn, &implicit_index);

                // Pre-warm the inline-compose in-memory index from any persisted
                // "inline-compose:*" entries.  Eliminates spawn_blocking contention
                // for repeated inline ValueSet $expand calls (e.g. EX06 benchmark).
                value_set::prebuild_inline_compose_index(&conn, &inline_compose_index);
            }
        }

        info!(db_path, "SQLite terminology backend initialized");

        Ok(Self {
            pool,
            implicit_index,
            bg_index_pending: Arc::new(Mutex::new(HashSet::new())),
            inline_compose_index,
            property_result_cache,
            plain_fts_cache,
            cs_abstract_prop_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_inactive_prop_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_concept_abstract_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_concept_inactive_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_version_for_msg_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_content_cache: Arc::new(RwLock::new(HashMap::new())),
            vs_version_for_msg_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_resolved_meta_cache: Arc::new(RwLock::new(HashMap::new())),
            lookup_response_cache: Arc::new(RwLock::new(HashMap::new())),
            validate_code_response_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_version_for_url_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_exists_cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Perform the post-bootstrap preparation that [`Self::new`] would normally
    /// do inline, but on the final post-import data.
    ///
    /// Intended to be called once after [`Self::new_without_fts_prebuild`] and
    /// the bootstrap directory sync, before the server begins accepting
    /// requests. It:
    ///
    /// 1. Rebuilds any concept closures that bootstrap re-imports invalidated.
    ///    `migrate_concept_closure` only touches systems that have hierarchy
    ///    edges but no closure rows, so unchanged systems cost nothing while a
    ///    re-imported SNOMED/LOINC system is rebuilt.
    /// 2. Incrementally builds the `concepts_fts` index for any system not yet
    ///    recorded in `concepts_fts_built` (missing or invalidated by a
    ///    bootstrap re-import), and refreshes the in-memory
    ///    implicit/inline-compose indexes — plus planner statistics only when a
    ///    build actually ran. Unchanged systems cost nothing, so a warm restart
    ///    does no FTS work before the server binds (issue #295).
    ///
    /// # Errors
    ///
    /// Returns [`HtsError::StorageError`] if a pooled connection cannot be
    /// acquired or the closure rebuild fails. FTS/index pre-warm failures are
    /// non-fatal (logged via the underlying helpers) and do not error.
    pub fn finalize_after_bootstrap(&self) -> Result<(), HtsError> {
        let conn = self
            .pool
            .get()
            .map_err(|e| HtsError::StorageError(format!("Failed to acquire connection: {e}")))?;

        // Rebuild closures wiped by bootstrap re-import (idempotent — only
        // systems missing closure rows are recomputed).
        schema::migrate_concept_closure(&conn).map_err(|e| {
            HtsError::StorageError(format!("Failed to rebuild concept closure: {e}"))
        })?;

        // Incrementally (re)build FTS for exactly the systems whose index is
        // missing or was invalidated by a bootstrap re-import — NOT a blanket
        // wipe + full rebuild on every boot (issue #295). When nothing changed
        // (a warm restart, or an `hts import`-prepared DB) this is a near-instant
        // no-op, so the server binds without re-tokenising the whole corpus.
        // Only refresh planner statistics when we actually built something:
        // ANALYZE stats persist on disk across restarts, so a no-op boot needs
        // none.
        let built = value_set::prebuild_concepts_fts(&conn);
        if built > 0 {
            let _ = conn.execute_batch(
                "ANALYZE concept_hierarchy; ANALYZE concepts; ANALYZE concept_closure; \
                 ANALYZE concept_properties; ANALYZE concept_designations; \
                 ANALYZE code_systems; ANALYZE value_sets; ANALYZE concept_maps;",
            );
        }
        value_set::prebuild_implicit_index(&conn, &self.implicit_index);
        value_set::prebuild_inline_compose_index(&conn, &self.inline_compose_index);
        Ok(())
    }

    /// Open an **in-memory** SQLite database (useful for tests).
    ///
    /// Each call creates a fresh, isolated database.
    #[allow(dead_code)]
    pub fn in_memory() -> Result<Self, HtsError> {
        // Use a shared-cache URI so all pool connections share the same in-memory DB.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let uri = format!("file:hts_mem_{id}?mode=memory&cache=shared");

        let manager = SqliteConnectionManager::file(&uri);
        let pool = Pool::builder()
            .max_size(4)
            .build(manager)
            .map_err(|e| HtsError::StorageError(format!("Failed to create in-memory pool: {e}")))?;

        {
            let mut conn = pool.get().map_err(|e| {
                HtsError::StorageError(format!("Failed to acquire in-memory connection: {e}"))
            })?;
            conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;")
                .map_err(|e| {
                    HtsError::StorageError(format!("Failed to configure in-memory pragmas: {e}"))
                })?;
            schema::apply(&conn).map_err(|e| {
                HtsError::StorageError(format!("Failed to apply in-memory schema: {e}"))
            })?;
            schema::migrate_search_columns(&conn).map_err(|e| {
                HtsError::StorageError(format!(
                    "Failed to apply in-memory search column migration: {e}"
                ))
            })?;
            schema::migrate_code_systems_drop_url_unique(&mut conn).map_err(|e| {
                HtsError::StorageError(format!(
                    "Failed to drop legacy code_systems.url UNIQUE: {e}"
                ))
            })?;
            schema::migrate_value_sets_drop_url_unique(&mut conn).map_err(|e| {
                HtsError::StorageError(format!("Failed to drop legacy value_sets.url UNIQUE: {e}"))
            })?;
            // See the on-disk path: must follow the drop_url_unique rebuilds.
            schema::migrate_authority_rank(&conn).map_err(|e| {
                HtsError::StorageError(format!("Failed to apply authority_rank migration: {e}"))
            })?;
        }

        Ok(Self {
            pool,
            implicit_index: Arc::new(RwLock::new(HashMap::new())),
            bg_index_pending: Arc::new(Mutex::new(HashSet::new())),
            inline_compose_index: Arc::new(RwLock::new(HashMap::new())),
            property_result_cache: Arc::new(RwLock::new(HashMap::new())),
            plain_fts_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_abstract_prop_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_inactive_prop_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_concept_abstract_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_concept_inactive_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_version_for_msg_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_content_cache: Arc::new(RwLock::new(HashMap::new())),
            vs_version_for_msg_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_resolved_meta_cache: Arc::new(RwLock::new(HashMap::new())),
            lookup_response_cache: Arc::new(RwLock::new(HashMap::new())),
            validate_code_response_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_version_for_url_cache: Arc::new(RwLock::new(HashMap::new())),
            cs_exists_cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Borrow the underlying r2d2 connection pool.
    pub fn pool(&self) -> &Pool<SqliteConnectionManager> {
        &self.pool
    }
}

// ── TerminologyMetadata ────────────────────────────────────────────────────────

impl TerminologyMetadata for SqliteTerminologyBackend {
    fn backend_name(&self) -> &'static str {
        "sqlite"
    }

    /// Query the `code_systems` table and return all stored canonical URLs.
    fn supported_systems(&self) -> Vec<String> {
        let conn = match self.pool.get() {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let mut stmt = match conn.prepare("SELECT url FROM code_systems ORDER BY url") {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        stmt.query_map([], |row| row.get::<_, String>(0))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    }

    /// The SQLite backend pre-materialises `concept_hierarchy` at import time,
    /// so `$subsumes` lookups are O(1) — subsumption is fully supported.
    fn supports_subsumption(&self) -> bool {
        true
    }

    /// Look up the canonical URL for a ValueSet or ConceptMap by its FHIR `id`.
    ///
    /// CodeSystem rows can share a FHIR id across versions (the synthetic
    /// storage id encodes the version), so for CodeSystem we also try matching
    /// `resource_json.id` and pick the latest version when several rows match.
    ///
    /// Returns `None` when the ID is unknown.
    fn resource_url_by_id(&self, resource_type: &str, id: &str) -> Option<String> {
        let conn = self.pool.get().ok()?;
        match resource_type {
            "CodeSystem" => {
                if let Ok(url) = conn.query_row(
                    "SELECT url FROM code_systems WHERE id = ?1",
                    rusqlite::params![id],
                    |row| row.get::<_, String>(0),
                ) {
                    return Some(url);
                }
                let sql = format!(
                    "SELECT url FROM code_systems \
                     WHERE json_extract(resource_json, '$.id') = ?1 \
                     ORDER BY {} LIMIT 1",
                    crate::backends::cs_precedence_order_by("code_systems")
                );
                conn.query_row(&sql, rusqlite::params![id], |row| row.get::<_, String>(0))
                    .ok()
            }
            "ValueSet" => {
                if let Ok(url) = conn.query_row(
                    "SELECT url FROM value_sets WHERE id = ?1",
                    rusqlite::params![id],
                    |row| row.get::<_, String>(0),
                ) {
                    return Some(url);
                }
                // Multi-version path: storage rows are keyed `<fhir-id>|<version>`,
                // so when the URL-path id is the bare FHIR id, fall back to a
                // resource_json scan and pick the latest version (matches how
                // CodeSystem reads handle the same case).
                let sql = format!(
                    "SELECT url FROM value_sets \
                     WHERE json_extract(resource_json, '$.id') = ?1 \
                     ORDER BY {} LIMIT 1",
                    crate::backends::vs_precedence_order_by("value_sets")
                );
                conn.query_row(&sql, rusqlite::params![id], |row| row.get::<_, String>(0))
                    .ok()
            }
            "ConceptMap" => conn
                .query_row(
                    "SELECT url FROM concept_maps WHERE id = ?1",
                    rusqlite::params![id],
                    |row| row.get::<_, String>(0),
                )
                .ok(),
            _ => None,
        }
    }
}

// ── Multi-version helpers (shared by code_system.rs + value_set.rs) ───────────

/// `true` for versions like `"1"` or `"2"` that should match any version
/// starting with that segment.
pub(super) fn code_system_version_is_short(ver: &str) -> bool {
    !ver.contains('.') && ver.chars().all(|c| c.is_ascii_digit())
}

/// Pick the highest-version row that matches `pattern`.
///
/// Each `x` segment in the pattern is a wildcard.  Bare numeric prefixes
/// (e.g. `"1"`) match any version starting with that segment.  Returns
/// `None` when no candidate matches.
pub(super) fn code_system_select_version_match(
    candidates: &[(String, Option<String>)],
    pattern: &str,
) -> Option<(String, Option<String>)> {
    let segments: Vec<&str> = pattern.split('.').collect();
    candidates
        .iter()
        .filter(|(_, v)| match v {
            Some(actual) => code_system_version_matches(actual, &segments),
            None => false,
        })
        .max_by(|a, b| a.1.cmp(&b.1))
        .cloned()
}

fn code_system_version_matches(actual: &str, pattern_segments: &[&str]) -> bool {
    let actual_segments: Vec<&str> = actual.split('.').collect();
    if pattern_segments.len() > actual_segments.len() {
        return false;
    }
    pattern_segments
        .iter()
        .zip(actual_segments.iter())
        .all(|(p, a)| *p == "x" || *p == *a)
}

// ── BundleImportBackend ────────────────────────────────────────────────────────

impl TerminologyCaches for SqliteTerminologyBackend {
    /// Evict every in-memory index and response memo so the next read re-derives
    /// from SQLite.
    ///
    /// Called after a bundle import *and* after every CRUD write (issue #304).
    /// Previously this cleared only the four concept indexes plus two CS
    /// metadata caches, on the argument that the response memos needed no
    /// invalidation because "hot-path bench loops reuse one backend, and tests
    /// instantiate fresh ones per case". That is true of the benchmark and
    /// false of a server: a `PUT /CodeSystem/{id}` that changes a display, or
    /// adds a code, left `lookup_response_cache` and
    /// `validate_code_response_cache` answering from the superseded content
    /// indefinitely.
    ///
    /// # Totality is enforced by the compiler
    ///
    /// The destructuring below names every field with no `..` rest pattern, so
    /// adding a seventeenth cache to [`SqliteTerminologyBackend`] fails to
    /// compile (E0027) until the author names it here and decides whether it
    /// needs clearing. The previous drift — six of sixteen caches cleared, the
    /// gap invisible — is not reachable from this shape.
    fn invalidate_caches(&self) {
        // Exhaustive by construction: no `..`. See the doc comment above.
        let Self {
            // Not a cache: the connection pool itself.
            pool: _,
            // Not a cache: a dedup guard for in-flight background index builds.
            // Clearing it would permit duplicate populate threads, not fresher
            // data — the threads it guards write to caches that this method is
            // about to empty anyway.
            bg_index_pending: _,
            implicit_index,
            inline_compose_index,
            property_result_cache,
            plain_fts_cache,
            cs_abstract_prop_cache,
            cs_inactive_prop_cache,
            cs_concept_abstract_cache,
            cs_concept_inactive_cache,
            cs_version_for_msg_cache,
            cs_content_cache,
            vs_version_for_msg_cache,
            cs_resolved_meta_cache,
            lookup_response_cache,
            validate_code_response_cache,
            cs_version_for_url_cache,
            cs_exists_cache,
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
            implicit_index,
            inline_compose_index,
            property_result_cache,
            plain_fts_cache,
            cs_abstract_prop_cache,
            cs_inactive_prop_cache,
            cs_concept_abstract_cache,
            cs_concept_inactive_cache,
            cs_version_for_msg_cache,
            cs_content_cache,
            vs_version_for_msg_cache,
            cs_resolved_meta_cache,
            lookup_response_cache,
            validate_code_response_cache,
            cs_version_for_url_cache,
            cs_exists_cache,
        );

        // Process-global caches shared by every backend instance in this
        // process. `import_code_system` / `delete_code_system` already drop
        // these on the paths they cover; doing it here too makes the hook total
        // regardless of which resource type was written, and both are cheap.
        invalidate_cs_id_cache();
        invalidate_cs_language_cache();
    }
}

#[async_trait]
impl BundleImportBackend for SqliteTerminologyBackend {
    /// Parse a FHIR Bundle from raw JSON bytes and insert all contained
    /// terminology resources into SQLite.
    ///
    /// Delegates to `crate::import::fhir_bundle::import_bundle_sync` on a
    /// blocking thread to avoid holding the async executor while performing
    /// synchronous SQLite I/O.
    async fn import_bundle(
        &self,
        _ctx: &TenantContext,
        data: &[u8],
    ) -> Result<ImportStats, HtsError> {
        let pool = self.pool.clone();
        let data_vec = data.to_vec();

        let result = tokio::task::spawn_blocking(move || {
            crate::import::fhir_bundle::import_bundle_sync(&pool, &data_vec)
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?;

        if result.is_ok() {
            self.invalidate_caches();
        }

        result
    }

    /// Insert an already-parsed bundle, skipping the JSON parse step.
    async fn import_parsed(
        &self,
        _ctx: &TenantContext,
        parsed: crate::import::bundle_parser::ParsedBundle,
    ) -> Result<ImportStats, HtsError> {
        let pool = self.pool.clone();

        let result = tokio::task::spawn_blocking(move || {
            crate::import::fhir_bundle::import_parsed_sync(&pool, &parsed)
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?;

        if result.is_ok() {
            self.invalidate_caches();
        }

        result
    }

    async fn code_system_has_concepts(
        &self,
        _ctx: &TenantContext,
        url: &str,
    ) -> Result<bool, HtsError> {
        let pool = self.pool.clone();
        let url = url.to_string();
        tokio::task::spawn_blocking(move || -> Result<bool, HtsError> {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
            conn.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM concepts c
                     JOIN code_systems s ON c.system_id = s.id
                     WHERE s.url = ?1
                 )",
                rusqlite::params![url],
                |r| r.get(0),
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::TerminologyMetadata;

    fn backend() -> SqliteTerminologyBackend {
        SqliteTerminologyBackend::in_memory().expect("in-memory backend should initialise")
    }

    #[test]
    fn backend_name_is_sqlite() {
        assert_eq!(backend().backend_name(), "sqlite");
    }

    #[test]
    fn supported_systems_empty_initially() {
        let b = backend();
        assert!(
            b.supported_systems().is_empty(),
            "no systems should exist in a fresh DB"
        );
    }

    #[test]
    fn supported_systems_after_insert() {
        let b = backend();

        let conn = b.pool().get().unwrap();
        conn.execute(
            "INSERT INTO code_systems (id, url, status, content, created_at, updated_at)
             VALUES ('cs1', 'http://example.org/cs', 'active', 'complete', '2024-01-01', '2024-01-01')",
            [],
        )
        .unwrap();
        drop(conn);

        let systems = b.supported_systems();
        assert_eq!(systems, vec!["http://example.org/cs".to_string()]);
    }

    #[test]
    fn supports_subsumption_is_true() {
        assert!(backend().supports_subsumption());
    }

    #[test]
    fn bounded_cache_insert_evicts_instead_of_freezing() {
        // The pre-fix policy froze the cache once full: keys beyond `max` were
        // never admitted. With eviction, the map stays at `max` but always
        // admits the newest key (the regression we are guarding against).
        let mut map: HashMap<i32, i32> = HashMap::new();
        let max = 3;
        for i in 0..100 {
            bounded_cache_insert(&mut map, i, i * 10, max);
            assert!(map.len() <= max, "cache must never exceed its bound");
            // The just-inserted key is always present (random replacement only
            // evicts a *different*, pre-existing entry to make room).
            assert_eq!(map.get(&i), Some(&(i * 10)), "newest key must be admitted");
        }
        assert_eq!(map.len(), max, "a saturated cache stays at its bound");
    }

    #[test]
    fn bounded_cache_insert_overwrite_does_not_evict() {
        // Re-inserting an existing key updates in place without evicting, so a
        // full cache of distinct keys is preserved on a refresh.
        let mut map: HashMap<&str, i32> = HashMap::new();
        let max = 2;
        bounded_cache_insert(&mut map, "a", 1, max);
        bounded_cache_insert(&mut map, "b", 2, max);
        bounded_cache_insert(&mut map, "a", 99, max); // overwrite, not grow
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("a"), Some(&99));
        assert_eq!(map.get("b"), Some(&2));
    }

    #[test]
    fn bounded_cache_insert_zero_max_is_noop() {
        let mut map: HashMap<i32, i32> = HashMap::new();
        bounded_cache_insert(&mut map, 1, 1, 0);
        assert!(map.is_empty(), "max=0 must never store anything");
    }

    #[test]
    fn new_with_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("hts_test.db");
        let b = SqliteTerminologyBackend::new(db_path.to_str().unwrap())
            .expect("file-based backend should initialise");
        assert_eq!(b.backend_name(), "sqlite");
        assert!(b.supported_systems().is_empty());
    }
}
