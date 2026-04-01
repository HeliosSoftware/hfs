use std::sync::Arc;

#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::SqliteBackend;
#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;

use crate::error::HtsError;
use crate::traits::TerminologyBackend;

/// Shared application state injected into every Axum handler.
///
/// `B` is the concrete terminology backend (e.g., `SqliteTerminologyBackend`).
/// The backend is wrapped in `Arc` so it can be cheaply cloned across threads.
///
/// Two optional fields support the CRUD API:
/// - `resource_store`: a `helios-persistence` `SqliteBackend` for raw FHIR JSON
///   storage (CRUD, versioning, ETag). Uses the same SQLite file as `backend`.
/// - `hts_pool`: a cloned r2d2 pool pointing at the HTS SQLite database, used
///   by CRUD handlers to keep the normalized terminology tables in sync.
#[derive(Clone)]
pub struct AppState<B: TerminologyBackend> {
    /// The backing terminology store.
    pub backend: Arc<B>,

    /// Raw FHIR resource store for versioned CRUD over the same SQLite file.
    #[cfg(feature = "sqlite")]
    pub resource_store: Option<Arc<SqliteBackend>>,

    /// r2d2 pool for the HTS normalized tables; used by CRUD handlers to
    /// re-index terminology after a create, update, or delete.
    #[cfg(feature = "sqlite")]
    pub hts_pool: Option<Arc<Pool<SqliteConnectionManager>>>,
}

impl<B: TerminologyBackend> AppState<B> {
    /// Wrap `backend` in an `Arc` and return a ready-to-use state.
    ///
    /// `resource_store` and `hts_pool` start as `None`; call
    /// [`with_resource_store`] and [`with_hts_pool`] to enable the CRUD API.
    pub fn new(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
            #[cfg(feature = "sqlite")]
            resource_store: None,
            #[cfg(feature = "sqlite")]
            hts_pool: None,
        }
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
}
