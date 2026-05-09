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
use crate::traits::TerminologyMetadata;
use crate::types::LookupResponse;
use helios_persistence::tenant::TenantContext;

// ─── Per-instance cache type aliases (see field docs on SqliteTerminologyBackend) ──
pub(crate) type PropCodesMap = HashMap<String, Arc<Vec<String>>>;
pub(crate) type ConceptFlagMap = HashMap<(String, String), bool>;
pub(crate) type ResolvedMeta = (String, String, Option<String>);
pub(crate) type ResolvedMetaMap = HashMap<(String, Option<String>), ResolvedMeta>;
pub(crate) type StringOptionMap = HashMap<String, Option<String>>;
pub(crate) type LookupResponseMap = HashMap<String, Arc<LookupResponse>>;

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

            // Clear the concept FTS index on every startup — it is always rebuilt
            // synchronously by prebuild_concepts_fts below, so stale rows from a
            // previous run must be removed first.
            // The implicit_expansion_cache is intentionally kept across restarts:
            // populate_implicit_cache runs inside a BEGIN EXCLUSIVE transaction, so
            // SQLite rolls back any partial write on crash — the entries are always
            // fully committed or fully absent.  Persisting the cache means repeated
            // server restarts (e.g. benchmark reruns) start warm rather than cold.
            // Cache entries are invalidated per-code-system when new data is imported
            // (see fhir_bundle::write_code_system).
            let _ = conn.execute_batch(
                "DELETE FROM concepts_fts;
                 DELETE FROM concepts_fts_built;
                 DELETE FROM concepts_word_fts;",
            );

            // Update query-planner statistics for large tables.
            let _ = conn.execute_batch(
                "ANALYZE concept_hierarchy; ANALYZE concepts; ANALYZE concept_closure; \
                 ANALYZE concept_properties; ANALYZE concept_designations; \
                 ANALYZE code_systems; ANALYZE value_sets; ANALYZE concept_maps;",
            );

            // Pre-populate the concepts_fts trigram index for every code system
            // so that text-filtered $expand requests always use the fast FTS path.
            // This runs synchronously before the server accepts requests; for large
            // systems (SNOMED 638K, LOINC 181K) it can take 10–25 s total.
            value_set::prebuild_concepts_fts(&conn);

            // Pre-warm the in-memory concept index from any implicit-expansion
            // entries that are already persisted in implicit_expansion_cache.
            // On a warm restart (e.g. repeated benchmark runs) this lets the
            // async hot path in expand() fire immediately without waiting for a
            // background build thread.  No-op on first run (empty cache).
            value_set::prebuild_implicit_index(&conn, &implicit_index);

            // Pre-warm the inline-compose in-memory index from any persisted
            // "inline-compose:*" entries.  Eliminates spawn_blocking contention
            // for repeated inline ValueSet $expand calls (e.g. EX06 benchmark).
            value_set::prebuild_inline_compose_index(&conn, &inline_compose_index);
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
        })
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
                conn.query_row(
                    "SELECT url FROM code_systems \
                     WHERE json_extract(resource_json, '$.id') = ?1 \
                     ORDER BY COALESCE(version, '') DESC \
                     LIMIT 1",
                    rusqlite::params![id],
                    |row| row.get::<_, String>(0),
                )
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
                conn.query_row(
                    "SELECT url FROM value_sets \
                     WHERE json_extract(resource_json, '$.id') = ?1 \
                     ORDER BY COALESCE(version, '') DESC \
                     LIMIT 1",
                    rusqlite::params![id],
                    |row| row.get::<_, String>(0),
                )
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
        let implicit_index = self.implicit_index.clone();
        let inline_compose_index = self.inline_compose_index.clone();
        let property_result_cache = self.property_result_cache.clone();
        let plain_fts_cache = self.plain_fts_cache.clone();

        let result = tokio::task::spawn_blocking(move || {
            crate::import::fhir_bundle::import_bundle_sync(&pool, &data_vec)
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?;

        // Evict all in-memory indexes so the next expand re-reads fresh data.
        if result.is_ok() {
            if let Ok(mut guard) = implicit_index.write() {
                guard.clear();
            }
            if let Ok(mut guard) = inline_compose_index.write() {
                guard.clear();
            }
            if let Ok(mut guard) = property_result_cache.write() {
                guard.clear();
            }
            if let Ok(mut guard) = plain_fts_cache.write() {
                guard.clear();
            }
        }

        result
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
    fn new_with_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("hts_test.db");
        let b = SqliteTerminologyBackend::new(db_path.to_str().unwrap())
            .expect("file-based backend should initialise");
        assert_eq!(b.backend_name(), "sqlite");
        assert!(b.supported_systems().is_empty());
    }
}
