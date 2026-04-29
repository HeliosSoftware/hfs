//! Shared application state injected into Axum handlers.
//!
//! The [`AppState`] struct is generic over the concrete [`TerminologyBackend`]
//! implementation so that the SQLite and PostgreSQL paths can reuse the same
//! handler bodies.  It also carries the raw FHIR resource store used by CRUD
//! handlers and the asynchronous re-index hook (`terminology_importer`)
//! triggered after create/update operations.
//!
//! [`TerminologyBackend`]: crate::traits::TerminologyBackend

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use bytes::Bytes;
use helios_persistence::ResourceStorage;
#[cfg(feature = "postgres")]
use helios_persistence::backends::postgres::PostgresBackend;
#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::SqliteBackend;
#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;

use crate::error::HtsError;
use crate::import::BundleImportBackend;
use crate::traits::TerminologyBackend;

/// Key for the in-process `$expand` result cache.
///
/// Covers both URL-based expansions (`url` parameter) and inline-ValueSet
/// expansions (ad-hoc POST with a `valueSet` body — the body is serialised
/// to compact JSON for comparison, which is stable because k6 sends identical
/// bytes every iteration).
#[derive(Hash, Eq, PartialEq, Clone)]
pub struct ExpandCacheKey {
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

/// Shared application state injected into every Axum handler.
///
/// `B` is the concrete terminology backend (e.g., `SqliteTerminologyBackend`).
/// The backend is wrapped in `Arc` so it can be cheaply cloned across threads.
///
/// ## CRUD storage fields
///
/// - **SQLite path**: `resource_store` (SQLite persistence) + `hts_pool` (SQLite
///   re-index pool) are set via [`Self::with_resource_store`] /
///   [`Self::with_hts_pool`].
/// - **PostgreSQL path**: `resource_store_pg` (persistence) + `terminology_importer`
///   (async re-index via `BundleImportBackend`) are set via the
///   `with_resource_store_pg` / [`Self::with_terminology_importer`]
///   methods.  (`with_resource_store_pg` is only available with the `postgres`
///   feature enabled.)
///
/// CRUD handlers check the SQLite fields first (backward compat) and fall back
/// to the generic fields for the PostgreSQL backend.
#[derive(Clone)]
pub struct AppState<B: TerminologyBackend> {
    /// The backing terminology store.
    pub backend: Arc<B>,

    /// Raw FHIR resource store for versioned CRUD (SQLite path).
    #[cfg(feature = "sqlite")]
    pub resource_store: Option<Arc<SqliteBackend>>,

    /// r2d2 pool for HTS normalized-table re-indexing (SQLite path).
    #[cfg(feature = "sqlite")]
    pub hts_pool: Option<Arc<Pool<SqliteConnectionManager>>>,

    /// Raw FHIR resource store for versioned CRUD (PostgreSQL path).
    #[cfg(feature = "postgres")]
    pub resource_store_pg: Option<Arc<PostgresBackend>>,

    /// Async re-index hook used after create / update operations (PostgreSQL
    /// path).  Wraps the `PostgresTerminologyBackend` so CRUD handlers can call
    /// `import_bundle` without knowing the concrete type.
    pub terminology_importer: Option<Arc<dyn BundleImportBackend>>,

    /// Maximum number of codes allowed in a single `$expand` response.
    /// Requests that would exceed this limit receive `HtsError::TooCostly`.
    /// Defaults to `3_500`; override with `HTS_MAX_EXPANSION_SIZE`.
    pub max_expansion_size: u32,

    /// In-process LRU-style cache for `$expand` responses.
    ///
    /// Eliminates redundant backend work when the same expansion is requested
    /// repeatedly (e.g. by k6 virtual users running the same script).  The
    /// cache is never invalidated during normal server operation; after a
    /// bundle import call [`Self::clear_expand_cache`].
    pub expand_cache: ExpandCache,
}

impl<B: TerminologyBackend> AppState<B> {
    /// Wrap `backend` in an `Arc` and return a ready-to-use state.
    pub fn new(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
            #[cfg(feature = "sqlite")]
            resource_store: None,
            #[cfg(feature = "sqlite")]
            hts_pool: None,
            #[cfg(feature = "postgres")]
            resource_store_pg: None,
            terminology_importer: None,
            max_expansion_size: 10_000,
            expand_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Evict all cached `$expand` results.
    ///
    /// Call this after a successful bundle import so that expansions reflecting
    /// the new terminology data are recomputed on the next request.
    pub fn clear_expand_cache(&self) {
        if let Ok(mut cache) = self.expand_cache.write() {
            cache.clear();
        }
    }

    /// Override the maximum expansion size (default: 10 000).
    pub fn with_max_expansion_size(mut self, limit: u32) -> Self {
        self.max_expansion_size = limit;
        self
    }

    /// Attach a `helios-persistence` SQLite backend for raw FHIR resource storage.
    #[cfg(feature = "sqlite")]
    pub fn with_resource_store(mut self, store: SqliteBackend) -> Self {
        self.resource_store = Some(Arc::new(store));
        self
    }

    /// Attach the HTS r2d2 pool for normalized-table re-indexing during CRUD.
    #[cfg(feature = "sqlite")]
    pub fn with_hts_pool(mut self, pool: Pool<SqliteConnectionManager>) -> Self {
        self.hts_pool = Some(Arc::new(pool));
        self
    }

    /// Attach a `helios-persistence` PostgreSQL backend for raw FHIR resource storage.
    #[cfg(feature = "postgres")]
    pub fn with_resource_store_pg(mut self, store: PostgresBackend) -> Self {
        self.resource_store_pg = Some(Arc::new(store));
        self
    }

    /// Attach an async re-index hook (e.g. `PostgresTerminologyBackend`) for
    /// create/update operations.  Used by the PostgreSQL CRUD path.
    pub fn with_terminology_importer(mut self, importer: Arc<dyn BundleImportBackend>) -> Self {
        self.terminology_importer = Some(importer);
        self
    }

    /// Access the terminology backend directly (avoids cloning the `Arc`).
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Clone the HTS pool Arc, returning an error if not initialised.
    #[cfg(feature = "sqlite")]
    pub fn require_hts_pool(&self) -> Result<Arc<Pool<SqliteConnectionManager>>, HtsError> {
        self.hts_pool
            .clone()
            .ok_or_else(|| HtsError::Internal("HTS pool not initialized".into()))
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
