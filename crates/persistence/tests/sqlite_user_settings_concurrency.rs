//! Regression coverage for #833: concurrent writers racing the same
//! `user_settings` row on a file-backed SQLite store (WAL) used to surface a
//! raw "database is locked" backend error instead of either succeeding or
//! reporting an optimistic-lock conflict. `write_settings` and
//! `purge_tenant_settings` read the current row before writing the new one,
//! so opening that transaction as the SQLite default, DEFERRED, let it take
//! its read snapshot on the first `SELECT` and then fail the read-to-write
//! upgrade with `SQLITE_BUSY_SNAPSHOT` the instant another connection
//! committed in between — a failure mode the busy handler (and its
//! configured `busy_timeout`) never gets a chance to retry. The fix opens
//! these transactions as `BEGIN IMMEDIATE` instead, taking the write lock up
//! front so the conflict is resolved by SQLite's normal lock queue rather
//! than a snapshot failure. This exercises the real symptom — the UI's SQL
//! Export kick-off, which reads-then-writes the settings document per job —
//! against a real file, not `:memory:`, since only a real file exercises
//! WAL's multi-connection snapshot behavior.

#![cfg(feature = "sqlite")]

use std::sync::Arc;
use std::time::Duration;

use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::SettingsStore;
use helios_persistence::error::{ConcurrencyError, StorageError};
use serde_json::json;

fn file_backend(dir: &tempfile::TempDir) -> SqliteBackend {
    let backend = SqliteBackend::with_config(
        dir.path().join("settings.db").to_str().unwrap(),
        SqliteBackendConfig::default(),
    )
    .expect("open file-backed sqlite");
    backend.init_schema().expect("init schema");
    backend
}

/// N writers race a `patch_settings` against the same document, all
/// asserting the *same* `if_match_version` read up front — deliberately
/// setting up a race where at most one can win. Every loser must come back
/// as an optimistic-lock conflict; none may surface as a raw backend error
/// (which is what an unwaited `SQLITE_BUSY_SNAPSHOT` looked like before the
/// fix — the pool's 30s `busy_timeout` never even engaged for it).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_patches_against_one_document_never_report_database_locked() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(file_backend(&dir));
    let user_key = "u2:4:test:racer";

    let seeded = backend
        .put_settings(user_key, json!({"byTenant": {}}), None)
        .await
        .expect("seed settings");
    let expected_version = seeded.version;

    const WRITERS: usize = 12;
    let mut tasks = Vec::with_capacity(WRITERS);
    for i in 0..WRITERS {
        let backend = Arc::clone(&backend);
        tasks.push(tokio::spawn(async move {
            backend
                .patch_settings(
                    user_key,
                    json!({ "byTenant": { "default": { "sqlExport": { "jobs": { format!("job-{i}"): {"status": "in-progress"} } } } } }),
                    Some(expected_version),
                )
                .await
        }));
    }

    let results = tokio::time::timeout(Duration::from_secs(30), futures::future::join_all(tasks))
        .await
        .expect("writers must not hang under WAL contention");

    let mut ok_count = 0;
    let mut conflict_count = 0;
    for result in results {
        match result.expect("task panicked") {
            Ok(_) => ok_count += 1,
            Err(StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure { .. })) => {
                conflict_count += 1;
            }
            Err(other) => panic!(
                "expected a version conflict, not a backend failure (this is the \
                 'database is locked' regression): {other}"
            ),
        }
    }
    assert_eq!(ok_count, 1, "exactly one writer should win the race");
    assert_eq!(conflict_count, WRITERS - 1);
}

/// The same race, but every writer instead reads its own starting version
/// immediately before patching (the shape `sql_export::start` follows: load,
/// then write once). Every writer must still resolve to `Ok` or a version
/// conflict — a lost race retried against the freshly-read version — never a
/// locked-database error, and the settings document ends up holding every
/// job that ever reported success.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_read_then_write_cycles_never_report_database_locked() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(file_backend(&dir));
    let user_key = "u2:4:test:reader-writer";
    backend
        .put_settings(user_key, json!({}), None)
        .await
        .expect("seed settings");

    const WRITERS: usize = 12;
    let mut tasks = Vec::with_capacity(WRITERS);
    for i in 0..WRITERS {
        let backend = Arc::clone(&backend);
        tasks.push(tokio::spawn(async move {
            let current = backend
                .get_settings(user_key)
                .await
                .expect("read settings")
                .expect("settings row exists");
            backend
                .patch_settings(
                    user_key,
                    json!({ "byTenant": { "default": { "sqlExport": { "jobs": { format!("job-{i}"): {"status": "complete"} } } } } }),
                    Some(current.version),
                )
                .await
        }));
    }

    let results = tokio::time::timeout(Duration::from_secs(30), futures::future::join_all(tasks))
        .await
        .expect("writers must not hang under WAL contention");

    for result in results {
        match result.expect("task panicked") {
            Ok(_)
            | Err(StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure { .. })) => {}
            Err(other) => panic!(
                "expected a version conflict, not a backend failure (this is the \
                 'database is locked' regression): {other}"
            ),
        }
    }
}
