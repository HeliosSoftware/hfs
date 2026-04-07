use std::sync::Arc;

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

/// Shared application state injected into every Axum handler.
///
/// `B` is the concrete terminology backend (e.g., `SqliteTerminologyBackend`).
/// The backend is wrapped in `Arc` so it can be cheaply cloned across threads.
///
/// ## CRUD storage fields
///
/// - **SQLite path**: `resource_store` (SQLite persistence) + `hts_pool` (SQLite
///   re-index pool) are set via [`with_resource_store`] / [`with_hts_pool`].
/// - **PostgreSQL path**: `resource_store_pg` (persistence) + `terminology_importer`
///   (async re-index via `BundleImportBackend`) are set via the
///   [`with_resource_store_pg`] / [`with_terminology_importer`] methods.
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
    /// Defaults to `10_000`; override with `HTS_MAX_EXPANSION_SIZE`.
    pub max_expansion_size: u32,
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
