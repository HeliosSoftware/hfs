//! WAL upkeep outside `$bulk-submit` (#2025).
//!
//! `$bulk-submit` folds the WAL back at every file boundary (#978). Every other
//! write path relied on SQLite's passive auto-checkpoint, which cannot finish
//! while readers hold older snapshots, so concurrent transaction Bundles grew
//! the WAL to 36 GB. Nothing shrank it afterwards: auto-checkpoint only runs on
//! a commit, and the file keeps its size after a reset. Every later read went
//! through it, and every start rebuilt its index (~15 s).
//!
//! Under 20 concurrent transaction Bundles the WAL grows by 200–250 MB/s, so
//! the size is checked often and cheaply (a `stat`, no pooled connection) and
//! folded back in every pause of the load. The `TRUNCATE` waits only a second
//! while the server runs: with long Bundle transactions a longer wait seldom
//! truncated (118 of 143 passes still busy at 10 s) and held writers back,
//! raising the "database is locked" 503s from 4.6 % to 7.9 % (#1636). Only the
//! shutdown checkpoint, with no load left, waits longer.

use std::time::Duration;

use super::SqliteBackend;
use super::bulk_submit::checkpoint_wal_waiting;
use crate::error::{BackendError, StorageError, StorageResult};

/// A WAL file larger than this is folded back by [`SqliteBackend::maintain_wal`].
/// It matches the `wal_autocheckpoint` of 50,000 pages (~200 MiB), so a WAL
/// the auto-checkpoint keeps up with is never touched.
pub(super) const WAL_MAINTENANCE_BYTES: u64 = 256 * 1024 * 1024;

/// How long a maintenance pass's `TRUNCATE` waits for readers and writers
/// before leaving the rest to the next pass.
const RUNNING_TRUNCATE_WAIT: Duration = Duration::from_secs(1);

/// How long the shutdown checkpoint's `TRUNCATE` waits: the server has
/// drained, so only stragglers can be in the way.
const SHUTDOWN_TRUNCATE_WAIT: Duration = Duration::from_secs(10);

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
    /// grown past [`WAL_MAINTENANCE_BYTES`] (#2025). Meant to run every few
    /// seconds: below the threshold it only reads the file size, without a
    /// pooled connection. Past it, it copies passively first, then truncates,
    /// waiting at most a second for readers and writers. A busy pass leaves the
    /// rest to the next one.
    /// `None` when nothing was due, or for an in-memory database.
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
        let Some(wal_path) = self.wal_path().map(std::path::Path::to_path_buf) else {
            return Ok(None);
        };
        let before = file_bytes(&wal_path);
        if before == 0 || before < threshold {
            return Ok(None);
        }
        let wait = if threshold == 0 {
            SHUTDOWN_TRUNCATE_WAIT
        } else {
            RUNNING_TRUNCATE_WAIT
        };
        let report = self
            .run_blocking(move |conn| {
                checkpoint_wal_waiting(conn, wait).map_err(|e| {
                    StorageError::Backend(BackendError::Internal {
                        backend_name: "sqlite".to_string(),
                        message: format!("WAL checkpoint: {e}"),
                        source: None,
                    })
                })
            })
            .await?;
        let after = file_bytes(&wal_path);
        tracing::info!(
            wal_file_bytes = before,
            wal_file_bytes_after = after,
            wal_frames = report.wal_frames,
            passive_checkpointed = report.passive_checkpointed,
            truncate_busy = report.truncate_busy,
            truncate_wait_ms = wait.as_millis() as u64,
            duration_ms = report.duration.as_millis() as u64,
            "sqlite WAL folded back into the database"
        );
        Ok(Some(WalMaintenance {
            wal_file_bytes: before,
            wal_file_bytes_after: after,
            truncate_busy: report.truncate_busy,
        }))
    }
}

/// Size of a file, 0 when it does not exist.
fn file_bytes(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
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

    /// A reader holding a WAL snapshot makes the pass give up after its wait,
    /// without hanging; the next pass, once the reader is done, truncates.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reader_in_the_wal_defers_truncation_to_the_next_pass() {
        let dir = tempfile::tempdir().unwrap();
        let backend = backend_with_writes(dir.path()).await;
        let reader = rusqlite::Connection::open(dir.path().join("wal.db")).unwrap();
        reader.execute_batch("BEGIN").unwrap();
        let _: i64 = reader
            .query_row("SELECT COUNT(*) FROM resources", [], |row| row.get(0))
            .unwrap();
        // A write after the reader's snapshot, so the WAL holds frames it
        // still needs.
        let tenant = TenantContext::new(TenantId::new("t"), TenantPermissions::full_access());
        backend
            .create(
                &tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": "late" }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let started = std::time::Instant::now();
        let busy = backend.maintain_wal_over(1).await.unwrap().expect("due");
        assert!(busy.truncate_busy);
        assert!(busy.wal_file_bytes_after > 0);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );

        reader.execute_batch("COMMIT").unwrap();
        let done = backend.maintain_wal_over(1).await.unwrap().expect("due");
        assert!(!done.truncate_busy);
        assert_eq!(done.wal_file_bytes_after, 0);
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
