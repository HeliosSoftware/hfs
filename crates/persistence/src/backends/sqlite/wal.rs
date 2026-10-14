//! WAL upkeep outside `$bulk-submit` (#2025).
//!
//! `$bulk-submit` folds the WAL back at every file boundary (#978). Every other
//! write path relied on SQLite's passive auto-checkpoint, which cannot finish
//! while readers hold older snapshots, so concurrent transaction Bundles grew
//! the WAL to 36 GB. Nothing shrank it afterwards: auto-checkpoint only runs on
//! a commit, and the file keeps its size after a reset. Every later read went
//! through it, and every start rebuilt its index (~15 s).

use super::SqliteBackend;
use super::bulk_submit::checkpoint_wal;
use crate::error::{BackendError, StorageError, StorageResult};

/// A WAL file larger than this is folded back by [`SqliteBackend::maintain_wal`].
/// It matches the `wal_autocheckpoint` of 50,000 pages (~200 MiB), so a WAL
/// the auto-checkpoint keeps up with is never touched.
pub(super) const WAL_MAINTENANCE_BYTES: u64 = 256 * 1024 * 1024;

/// What one maintenance pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalMaintenance {
    /// Size of the WAL file before the pass.
    pub wal_file_bytes: u64,
    /// Size of the WAL file after the pass (0 once truncated).
    pub wal_file_bytes_after: u64,
    /// Whether the `TRUNCATE` step gave up on a reader or writer; the next pass
    /// retries.
    pub truncate_busy: bool,
}

impl SqliteBackend {
    /// Folds the WAL back into the database and truncates the file when it has
    /// grown past [`WAL_MAINTENANCE_BYTES`] (#2025). Meant to run periodically:
    /// it copies passively first and waits at most a second for readers and
    /// writers before truncating, so it never stalls the server; a busy pass
    /// leaves the rest to the next one. `None` when nothing was due, or for an
    /// in-memory database.
    pub async fn maintain_wal(&self) -> StorageResult<Option<WalMaintenance>> {
        self.maintain_wal_over(WAL_MAINTENANCE_BYTES).await
    }

    /// Folds the WAL back and truncates it whatever its size, for a clean
    /// shutdown, so a restart does not inherit it (#2025).
    pub async fn checkpoint_wal_at_shutdown(&self) -> StorageResult<Option<WalMaintenance>> {
        self.maintain_wal_over(0).await
    }

    pub(super) async fn maintain_wal_over(
        &self,
        threshold: u64,
    ) -> StorageResult<Option<WalMaintenance>> {
        if self.is_memory() {
            return Ok(None);
        }
        let outcome = self
            .run_blocking(move |conn| {
                let before = wal_file_bytes(conn);
                if before == 0 || before < threshold {
                    return Ok(None);
                }
                let report = checkpoint_wal(conn).map_err(|e| {
                    StorageError::Backend(BackendError::Internal {
                        backend_name: "sqlite".to_string(),
                        message: format!("WAL checkpoint: {e}"),
                        source: None,
                    })
                })?;
                Ok(Some((report, before, wal_file_bytes(conn))))
            })
            .await?;
        Ok(outcome.map(|(report, before, after)| {
            tracing::info!(
                wal_file_bytes = before,
                wal_file_bytes_after = after,
                wal_frames = report.wal_frames,
                passive_checkpointed = report.passive_checkpointed,
                truncate_busy = report.truncate_busy,
                duration_ms = report.duration.as_millis() as u64,
                "sqlite WAL folded back into the database"
            );
            WalMaintenance {
                wal_file_bytes: before,
                wal_file_bytes_after: after,
                truncate_busy: report.truncate_busy,
            }
        }))
    }
}

/// Size of the connection's `-wal` file, 0 when there is none.
fn wal_file_bytes(conn: &rusqlite::Connection) -> u64 {
    conn.path()
        .filter(|path| !path.is_empty())
        .and_then(|path| std::fs::metadata(format!("{path}-wal")).ok())
        .map(|meta| meta.len())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ResourceStorage;
    use crate::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_fhir::FhirVersion;
    use serde_json::json;

    async fn backend_with_writes(dir: &std::path::Path) -> SqliteBackend {
        let backend = SqliteBackend::open(dir.join("wal.db")).unwrap();
        backend.init_schema().unwrap();
        let tenant = TenantContext::new(TenantId::new("t"), TenantPermissions::full_access());
        for i in 0..50 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({ "resourceType": "Patient", "id": format!("p{i}") }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        backend
    }

    #[tokio::test]
    async fn a_wal_past_the_threshold_is_folded_back_and_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let backend = backend_with_writes(dir.path()).await;
        let wal = dir.path().join("wal.db-wal");
        assert!(std::fs::metadata(&wal).unwrap().len() > 0);

        let done = backend.maintain_wal_over(1).await.unwrap().expect("due");
        assert!(done.wal_file_bytes > 0);
        assert!(!done.truncate_busy);
        assert_eq!(done.wal_file_bytes_after, 0);
        assert_eq!(std::fs::metadata(&wal).unwrap().len(), 0);

        // Nothing left to do: the next pass is a no-op.
        assert_eq!(backend.maintain_wal_over(1).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_small_wal_is_left_to_the_auto_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let backend = backend_with_writes(dir.path()).await;
        assert_eq!(backend.maintain_wal().await.unwrap(), None);
        assert!(
            std::fs::metadata(dir.path().join("wal.db-wal"))
                .unwrap()
                .len()
                > 0
        );
    }

    #[tokio::test]
    async fn shutdown_truncates_whatever_the_size() {
        let dir = tempfile::tempdir().unwrap();
        let backend = backend_with_writes(dir.path()).await;
        let done = backend
            .checkpoint_wal_at_shutdown()
            .await
            .unwrap()
            .expect("due");
        assert_eq!(done.wal_file_bytes_after, 0);
    }

    #[tokio::test]
    async fn every_file_connection_caps_the_wal_it_keeps_after_a_reset() {
        let dir = tempfile::tempdir().unwrap();
        let backend = backend_with_writes(dir.path()).await;
        let conn = backend.get_connection().unwrap();
        let limit: i64 = conn
            .query_row("PRAGMA journal_size_limit", [], |row| row.get(0))
            .unwrap();
        assert_eq!(limit, 268_435_456);
    }

    #[tokio::test]
    async fn an_in_memory_database_has_no_wal_to_maintain() {
        let backend = SqliteBackend::in_memory().unwrap();
        backend.init_schema().unwrap();
        assert_eq!(backend.checkpoint_wal_at_shutdown().await.unwrap(), None);
    }
}
