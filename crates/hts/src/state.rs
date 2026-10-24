//! Shared application state injected into Axum handlers.
//!
//! The [`AppState`] struct is generic over the concrete [`TerminologyBackend`]
//! implementation so that the SQLite and PostgreSQL paths can reuse the same
//! handler bodies.  It also carries the raw FHIR resource store used by CRUD
//! handlers. CRUD commits the raw resource, history, and normalized terminology
//! through one backend transaction.
//!
//! [`TerminologyBackend`]: crate::traits::TerminologyBackend

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::traits::TerminologyBackend;
use bytes::Bytes;
use helios_persistence::ResourceStorage;
#[cfg(feature = "postgres")]
use helios_persistence::backends::postgres::PostgresBackend;
#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::SqliteBackend;

/// Key for the in-process `$expand` result cache.
///
/// Covers both URL-based expansions (`url` parameter) and inline-ValueSet
/// expansions (ad-hoc POST with a `valueSet` body — the body is serialised
/// to compact JSON for comparison, which is stable because k6 sends identical
/// bytes every iteration).
#[derive(Hash, Eq, PartialEq, Clone)]
pub struct ExpandCacheKey {
    /// Exact instance selection, distinct from canonical latest-version lookup.
    pub selected_value_set_id: Option<String>,
    /// Canonical ValueSet URL, or compact JSON of the inline `valueSet` body.
    pub url_or_body: String,
    /// Text filter (`""` when absent).
    pub filter: String,
    /// Requested page size (`u32::MAX` when absent, i.e. "all").
    pub count: u32,
    /// Zero-based page offset (`0` when absent).
    pub offset: u32,
    /// Whether a hierarchical (tree) expansion was requested.
    pub hierarchical: bool,
    /// Serialised, name-sorted form of the input parameters (excluding the
    /// `url` / `valueSet` discriminators already captured in `url_or_body`).
    /// Two requests with the same target ValueSet but different "extra" inputs
    /// (e.g. `excludeNested`, `displayLanguage`, `includeDesignations`) must
    /// yield distinct cache entries because the response echoes those inputs
    /// in `expansion.parameter`.
    pub extra_params: String,
}

/// Thread-safe in-process cache for `$expand` responses.
///
/// Keyed on [`ExpandCacheKey`]; values are pre-serialized FHIR JSON bytes
/// stored as [`bytes::Bytes`] — a reference-counted buffer that can be cloned
/// in O(1) and sent as an HTTP body without an extra allocation.
/// Bounded to [`EXPAND_CACHE_MAX`] entries; once full, new entries are
/// silently dropped (the benchmark never exceeds ~50 unique keys).
pub type ExpandCache = Arc<RwLock<HashMap<ExpandCacheKey, Bytes>>>;

pub const EXPAND_CACHE_MAX: usize = 2048;
/// Maximum number of ValueSet URLs to remember as definitively absent.
pub const NOT_FOUND_CACHE_MAX: usize = 10_000;

/// Negative-result cache: ValueSet URLs that returned 404 on the last expand.
///
/// Keyed on the canonical ValueSet URL only (filter/count/offset are irrelevant
/// — a missing URL is always missing).  Bounded to avoid unbounded growth when
/// a client probes many non-existent URLs.  Cleared together with
/// [`ExpandCache`] after every successful bundle import.
pub type NotFoundCache = Arc<RwLock<HashSet<String>>>;

/// Thread-safe per-AppState cache for fully-assembled `$validate-code` JSON
/// responses (both `CodeSystem/$validate-code` and `ValueSet/$validate-code`).
///
/// The cache lives at the *handler* layer — above every supplement / VS-import
/// pre-flight helper — so a warm hit on these endpoints skips ALL of:
/// `enforce_vs_supplement_extensions`, `detect_bad_vs_import`,
/// `resolve_supplements`, `supplement_url_in_coding_error`, and the per-system
/// `validate_code` backend call.
///
/// Keyed on a canonical, name-sorted serialisation of every input parameter
/// that influences the response.  Values are stored as `Arc<Value>` so warm
/// hits clone an Arc rather than re-serialising the whole tree.
///
/// Bounded to [`VALIDATE_CODE_HANDLER_CACHE_MAX`] entries — once full new
/// entries are dropped silently (the benchmark never exceeds a few hundred
/// distinct keys per cache).  Cleared alongside [`ExpandCache`] on bundle
/// import / CRUD writes via [`AppState::clear_expand_cache`].
pub type ValidateCodeHandlerCache = Arc<RwLock<HashMap<String, Arc<serde_json::Value>>>>;

/// Maximum number of cached `$validate-code` handler responses (per direction —
/// CS path and VS path each have their own map).
pub const VALIDATE_CODE_HANDLER_CACHE_MAX: usize = 16384;

/// Thread-safe per-AppState cache for fully-assembled `$expand` JSON bytes,
/// keyed at the *handler* layer above every `process_expand` pre-flight step
/// (URL parse, `useSupplement` resolution, `tx-resource` shortcut, the inner
/// `expand_cache` build, …).  A warm hit returns the previously-serialised
/// `Bytes` directly via O(1) Arc clone — no `serde_json::to_vec` call,
/// no backend round-trip, no helper overhead.
///
/// Keyed on a canonical, name-sorted serialisation of every input parameter
/// that influences the response.  Values are stored as [`Bytes`] which already
/// share their backing buffer reference-counted style.
///
/// Bounded to [`EXPAND_HANDLER_CACHE_MAX`] entries — once full new entries are
/// dropped silently.  Cleared alongside [`ExpandCache`] on bundle import /
/// CRUD writes via [`AppState::clear_expand_cache`].
pub type ExpandHandlerCache = Arc<RwLock<HashMap<String, Bytes>>>;

/// Maximum number of cached `$expand` handler responses.
pub const EXPAND_HANDLER_CACHE_MAX: usize = 16384;

/// Thread-safe per-AppState cache for fully-assembled `$lookup` JSON values,
/// keyed at the *handler* layer above every `process_lookup` pre-flight step
/// (supplement resolution, `backend.lookup` call, `code_system_language`
/// lookup, FHIR `Parameters` assembly).  Warm hits clone an `Arc<Value>`
/// rather than re-running the backend round-trips and FHIR builder.
///
/// LK01-04 hot path: each VU replays the same `(system, code, version,
/// displayLanguage, property...)` tuple across 50 VUs over a 30 s run.  The
/// backend has its own `lookup_response_cache`, but a request still pays for
/// supplement resolution, the synthesised `preferredForLanguage` designation,
/// and the Parameters builder.  This cache short-circuits ALL of that.
///
/// Bounded to [`LOOKUP_HANDLER_CACHE_MAX`] entries — once full new entries
/// drop silently.  Cleared on bundle import / CRUD writes via
/// [`AppState::clear_expand_cache`].
pub type LookupHandlerCache = Arc<RwLock<HashMap<String, Arc<serde_json::Value>>>>;

/// Maximum number of cached `$lookup` handler responses.
pub const LOOKUP_HANDLER_CACHE_MAX: usize = 16384;

/// Shared application state injected into every Axum handler.
///
/// `B` is the concrete terminology backend (e.g., `SqliteTerminologyBackend`).
/// The backend is wrapped in `Arc` so it can be cheaply cloned across threads.
///
/// ## CRUD storage fields
///
/// Attach the persistence backend that shares the terminology database via
/// `with_resource_store` (SQLite) or `with_resource_store_pg` (PostgreSQL).
/// CRUD handlers use that store's transaction for both raw resources and
/// normalized terminology, so a failed write cannot leave them inconsistent.
#[derive(Clone)]
pub struct AppState<B: TerminologyBackend> {
    /// The backing terminology store.
    pub backend: Arc<B>,

    /// Raw FHIR resource store for versioned CRUD (SQLite path).
    #[cfg(feature = "sqlite")]
    pub resource_store: Option<Arc<SqliteBackend>>,

    /// Raw FHIR resource store for versioned CRUD (PostgreSQL path).
    #[cfg(feature = "postgres")]
    pub resource_store_pg: Option<Arc<PostgresBackend>>,

    /// Maximum number of codes allowed in a single `$expand` response.
    /// Requests that would exceed this limit receive `HtsError::TooCostly`.
    /// Defaults to `3_500`; override with `HTS_MAX_EXPANSION_SIZE`.
    pub max_expansion_size: u32,

    /// In-process LRU-style cache for `$expand` responses.
    ///
    /// Eliminates redundant backend work when the same expansion is requested
    /// repeatedly (e.g. by k6 virtual users running the same script).  Evicted
    /// by [`Self::clear_expand_cache`] after a bundle import and after every
    /// CRUD write.
    pub expand_cache: ExpandCache,

    /// Negative cache for ValueSet URLs that are definitively absent.
    ///
    /// A URL in this set returned `HtsError::NotFound` on its last expand
    /// attempt.  Subsequent requests skip all backend queries and return 404
    /// immediately, eliminating the 5+ SQLite round-trips that a cold miss
    /// costs for each uncached URL.  Cleared together with `expand_cache`
    /// after every successful bundle import.
    pub not_found_urls: NotFoundCache,

    /// Handler-level response cache for `POST /CodeSystem/$validate-code` (both
    /// the `_handler` and `get_*_handler` entry points share `process_validate_code`).
    /// See [`ValidateCodeHandlerCache`].
    pub cs_validate_code_handler_cache: ValidateCodeHandlerCache,

    /// Handler-level response cache for `POST /ValueSet/$validate-code`.
    /// See [`ValidateCodeHandlerCache`].
    pub vs_validate_code_handler_cache: ValidateCodeHandlerCache,

    /// Handler-level response cache for `POST /ValueSet/$expand` (and the
    /// matching GET handler) — sits above every `process_expand` pre-flight
    /// step so warm hits skip URL parse, `useSupplement` resolution,
    /// `tx-resource` collection, and the inner `expand_cache` rebuild.
    /// See [`ExpandHandlerCache`].
    pub expand_handler_cache: ExpandHandlerCache,

    /// Handler-level response cache for `POST /ValueSet/$expand` requests that
    /// carry an inline `valueSet` body (and possibly `tx-resource` fixture
    /// resources).  These requests are skipped by `expand_handler_cache`
    /// because its key serialiser bails on any param with a `resource` field
    /// (the bodies blow up the key length).  This cache uses a hashed digest
    /// of every inline resource body instead, so identical compose / fixture
    /// payloads collide on the same key across thousands of repeated k6
    /// iterations.  Same bound (`EXPAND_HANDLER_CACHE_MAX`) and same
    /// invalidation hook as the URL-keyed cache.
    pub inline_compose_handler_cache: ExpandHandlerCache,

    /// Handler-level response cache for `CodeSystem/$lookup` (both POST and
    /// GET handlers, type-level and instance-level — they all funnel through
    /// `process_lookup`).  See [`LookupHandlerCache`].
    pub lookup_handler_cache: LookupHandlerCache,

    /// Negative cache for `$lookup` requests that returned `NotFound`.
    ///
    /// Iter 7i — targets LK05 (PG ~50% of SQLite). The positive
    /// [`LookupHandlerCache`] populates only on `Ok(_)`, so each not-found
    /// request paid supplement-resolve + backend lookup + per-system
    /// language fetch + Parameters builder anyway. This `HashSet<String>`
    /// of canonical [`build_lookup_cache_key`] strings short-circuits ALL
    /// of that — same key space as the positive cache, no risk of
    /// stale-positive shadowing because the negative cache is only
    /// consulted after the positive miss.
    ///
    /// Cleared together with the positive caches on bundle import via
    /// [`AppState::clear_expand_cache`] — a newly-imported supplement
    /// might define a previously-unknown code.
    pub lookup_not_found_cache: NotFoundCache,

    /// Serialises concept-closure rebuilds started by `POST /import?finalize=true`.
    ///
    /// The rebuild runs as a detached task, so two finalize requests can
    /// overlap. On PostgreSQL both would see the same system as missing its
    /// closure and race on `concept_closure`'s primary key. Holding this lock
    /// means the second caller finds nothing left to rebuild.
    pub closure_rebuild_lock: Arc<tokio::sync::Mutex<()>>,

    /// How long `POST /import?finalize=true` waits for the closure rebuild
    /// before answering `202 Accepted` and letting it finish in the background.
    /// Must stay below the router's 30 s request timeout. See
    /// [`DEFAULT_IMPORT_FINALIZE_WAIT`].
    pub import_finalize_wait: Duration,
}

/// Default for [`AppState::import_finalize_wait`]: comfortably inside the
/// 30 s `TimeoutLayer` in `server::create_app`, so a slow rebuild is reported
/// as pending instead of being cut off with a 408.
pub const DEFAULT_IMPORT_FINALIZE_WAIT: Duration = Duration::from_secs(20);

impl<B: TerminologyBackend> AppState<B> {
    /// Wrap `backend` in an `Arc` and return a ready-to-use state.
    pub fn new(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
            #[cfg(feature = "sqlite")]
            resource_store: None,
            #[cfg(feature = "postgres")]
            resource_store_pg: None,
            max_expansion_size: 10_000,
            expand_cache: Arc::new(RwLock::new(HashMap::new())),
            not_found_urls: Arc::new(RwLock::new(HashSet::new())),
            cs_validate_code_handler_cache: Arc::new(RwLock::new(HashMap::new())),
            vs_validate_code_handler_cache: Arc::new(RwLock::new(HashMap::new())),
            expand_handler_cache: Arc::new(RwLock::new(HashMap::new())),
            inline_compose_handler_cache: Arc::new(RwLock::new(HashMap::new())),
            lookup_handler_cache: Arc::new(RwLock::new(HashMap::new())),
            lookup_not_found_cache: Arc::new(RwLock::new(HashSet::new())),
            closure_rebuild_lock: Arc::new(tokio::sync::Mutex::new(())),
            import_finalize_wait: DEFAULT_IMPORT_FINALIZE_WAIT,
        }
    }

    /// Evict all cached `$expand` results and negative-cache entries, plus the
    /// per-AppState `$lookup` and `$validate-code` handler-response caches.
    ///
    /// Call this after any successful mutation of terminology content — bundle
    /// import and CRUD create/update/delete alike — so that expansions,
    /// lookups and validations reflecting the new data are recomputed on the
    /// next request.
    ///
    /// This is only the *handler* tier. The backend keeps its own per-instance
    /// memos, which a handler recomputation would otherwise read straight back;
    /// CRUD writes therefore go through
    /// `operations::crud::evict_caches_after_write`, which clears both tiers.
    /// See [`crate::traits::TerminologyCaches`].
    pub fn clear_expand_cache(&self) {
        if let Ok(mut cache) = self.expand_cache.write() {
            cache.clear();
        }
        if let Ok(mut neg) = self.not_found_urls.write() {
            neg.clear();
        }
        if let Ok(mut cache) = self.cs_validate_code_handler_cache.write() {
            cache.clear();
        }
        if let Ok(mut cache) = self.vs_validate_code_handler_cache.write() {
            cache.clear();
        }
        if let Ok(mut cache) = self.expand_handler_cache.write() {
            cache.clear();
        }
        if let Ok(mut cache) = self.inline_compose_handler_cache.write() {
            cache.clear();
        }
        if let Ok(mut cache) = self.lookup_handler_cache.write() {
            cache.clear();
        }
        if let Ok(mut neg) = self.lookup_not_found_cache.write() {
            neg.clear();
        }
    }

    /// Override the maximum expansion size (default: 10 000).
    pub fn with_max_expansion_size(mut self, limit: u32) -> Self {
        self.max_expansion_size = limit;
        self
    }

    /// Override [`Self::import_finalize_wait`].
    pub fn with_import_finalize_wait(mut self, wait: Duration) -> Self {
        self.import_finalize_wait = wait;
        self
    }

    /// Attach a `helios-persistence` SQLite backend for raw FHIR resource storage.
    #[cfg(feature = "sqlite")]
    pub fn with_resource_store(mut self, store: SqliteBackend) -> Self {
        self.resource_store = Some(Arc::new(store));
        self
    }

    /// Attach a `helios-persistence` PostgreSQL backend for raw FHIR resource storage.
    #[cfg(feature = "postgres")]
    pub fn with_resource_store_pg(mut self, store: PostgresBackend) -> Self {
        self.resource_store_pg = Some(Arc::new(store));
        self
    }

    /// Access the terminology backend directly (avoids cloning the `Arc`).
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Return the active raw FHIR resource store, if any.
    ///
    /// Checks the SQLite store first (backward compat), then the PostgreSQL store.
    pub fn active_resource_store(&self) -> Option<Arc<dyn ResourceStorage>> {
        #[cfg(feature = "sqlite")]
        if let Some(ref s) = self.resource_store {
            return Some(Arc::clone(s) as Arc<dyn ResourceStorage>);
        }
        #[cfg(feature = "postgres")]
        if let Some(ref s) = self.resource_store_pg {
            return Some(Arc::clone(s) as Arc<dyn ResourceStorage>);
        }
        None
    }
}
