pub mod schema;

mod code_system;
mod concept_map;
mod value_set;

use async_trait::async_trait;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use tracing::info;

use crate::error::HtsError;
use crate::import::{BundleImportBackend, ImportStats};
use crate::traits::TerminologyMetadata;
use helios_persistence::tenant::TenantContext;

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
    // Shared across all operation impls and the metadata trait.
    #[allow(dead_code)]
    pool: Pool<SqliteConnectionManager>,
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
        let manager = SqliteConnectionManager::file(db_path);

        let pool = Pool::builder()
            .max_size(8)
            .build(manager)
            .map_err(|e| HtsError::StorageError(format!("Failed to create SQLite pool: {e}")))?;

        // Bootstrap: apply pragmas + schema on a single connection.
        {
            let conn = pool.get().map_err(|e| {
                HtsError::StorageError(format!("Failed to acquire connection for init: {e}"))
            })?;

            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
                .map_err(|e| {
                    HtsError::StorageError(format!("Failed to configure SQLite pragmas: {e}"))
                })?;

            schema::apply(&conn)
                .map_err(|e| HtsError::StorageError(format!("Failed to apply HTS schema: {e}")))?;
        }

        info!(db_path, "SQLite terminology backend initialized");

        Ok(Self { pool })
    }

    /// Open an **in-memory** SQLite database (useful for tests).
    #[allow(dead_code)]
    ///
    /// Each call creates a fresh, isolated database.
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
            let conn = pool.get().map_err(|e| {
                HtsError::StorageError(format!("Failed to acquire in-memory connection: {e}"))
            })?;
            conn.execute_batch("PRAGMA foreign_keys=ON;").map_err(|e| {
                HtsError::StorageError(format!("Failed to configure in-memory pragmas: {e}"))
            })?;
            schema::apply(&conn).map_err(|e| {
                HtsError::StorageError(format!("Failed to apply in-memory schema: {e}"))
            })?;
        }

        Ok(Self { pool })
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
}

// ── BundleImportBackend ────────────────────────────────────────────────────────

#[async_trait]
impl BundleImportBackend for SqliteTerminologyBackend {
    /// Parse a FHIR Bundle from raw JSON bytes and insert all contained
    /// terminology resources into SQLite.
    ///
    /// Delegates to [`crate::import::fhir_bundle::import_bundle_sync`] on a
    /// blocking thread to avoid holding the async executor while performing
    /// synchronous SQLite I/O.
    async fn import_bundle(
        &self,
        _ctx: &TenantContext,
        data: &[u8],
    ) -> Result<ImportStats, HtsError> {
        let pool = self.pool.clone();
        let data_vec = data.to_vec();

        tokio::task::spawn_blocking(move || {
            crate::import::fhir_bundle::import_bundle_sync(&pool, &data_vec)
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
    fn new_with_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("hts_test.db");
        let b = SqliteTerminologyBackend::new(db_path.to_str().unwrap())
            .expect("file-based backend should initialise");
        assert_eq!(b.backend_name(), "sqlite");
        assert!(b.supported_systems().is_empty());
    }
}
