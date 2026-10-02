//! #1636: a transaction Bundle that cannot BEGIN because SQLite is busy.
//!
//! Under concurrent imports the single SQLite write lock is held for seconds at
//! a time, and a Bundle that waits out `busy_timeout` used to fail as
//! `TransactionError::RolledBack` (twice-wrapped: "Failed to begin
//! transaction: transaction rolled back: Failed to begin transaction: database
//! is locked"), which the REST layer answers with a 500. A busy database is
//! transient: the bundle must end as `TransactionError::Transient` so the
//! client gets a retryable 503.
//!
//! The write lock is held by a second, plain `rusqlite` connection on the same
//! WAL database file — the situation a concurrent importer creates — and the
//! backend under test has a short `busy_timeout`, so the failure is fast.

#![cfg(feature = "sqlite")]

use std::path::Path;

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{
    BundleEntry, BundleMethod, BundleProvider, Transaction, TransactionOptions, TransactionProvider,
};
use helios_persistence::error::TransactionError;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

fn tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("busy-begin-tenant"),
        TenantPermissions::full_access(),
    )
}

fn patient_bundle() -> Vec<BundleEntry> {
    vec![BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "name": [{"family": "BusyBegin"}]
        })),
        full_url: Some("urn:uuid:busy-begin".to_string()),
        ..Default::default()
    }]
}

/// A file-backed (WAL) backend. `busy_timeout_ms` is how long BEGIN waits for
/// the write lock; `max_connections` / `connection_timeout_ms` bound the pool.
fn file_backend(
    path: &Path,
    busy_timeout_ms: u32,
    max_connections: u32,
    connection_timeout_ms: u64,
) -> SqliteBackend {
    let backend = SqliteBackend::with_config(
        path.to_str().unwrap(),
        SqliteBackendConfig {
            busy_timeout_ms,
            max_connections,
            connection_timeout_ms,
            ..Default::default()
        },
    )
    .expect("file-backed SQLite backend");
    backend.init_schema().expect("init schema");
    backend
}

/// Takes SQLite's write lock from a connection the backend does not own and
/// keeps it until the returned connection is dropped.
fn hold_write_lock(path: &Path) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open(path).expect("second connection");
    conn.busy_timeout(std::time::Duration::from_millis(0))
        .expect("busy_timeout");
    conn.execute_batch("BEGIN IMMEDIATE")
        .expect("the second connection takes the write lock");
    conn
}

/// The headline: a held write lock ends the bundle as `Transient`, with the
/// SQLite error text in the reason exactly once.
#[tokio::test]
async fn bundle_that_cannot_begin_for_a_held_write_lock_is_transient() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("busy.db");
    let backend = file_backend(&path, 200, 10, 30_000);
    let tenant = tenant();

    let holder = hold_write_lock(&path);
    let err = backend
        .process_transaction(&tenant, patient_bundle(), FhirVersion::default())
        .await
        .expect_err("BEGIN IMMEDIATE cannot get the write lock");

    match err {
        TransactionError::Transient { attempts, reason } => {
            assert_eq!(attempts, 1, "begin is not retried");
            assert_eq!(
                reason.matches("database is locked").count(),
                1,
                "the SQLite error must appear once, not wrapped twice: {reason:?}"
            );
        }
        other => panic!("a busy BEGIN must be Transient, got {other:?}"),
    }

    // Nothing leaked: once the lock is released the same bundle goes through.
    drop(holder);
    let result = backend
        .process_transaction(&tenant, patient_bundle(), FhirVersion::default())
        .await
        .expect("the bundle succeeds once the lock is free");
    assert_eq!(result.entries[0].status, 201);
}

/// The other half of "can't BEGIN": the pool cannot hand out a connection
/// within its timeout. One pooled connection, held by an open transaction.
#[tokio::test]
async fn bundle_that_times_out_acquiring_a_pooled_connection_is_transient() {
    let tmp = tempfile::tempdir().unwrap();
    let backend = file_backend(&tmp.path().join("pool.db"), 200, 1, 200);
    let tenant = tenant();

    let held = backend
        .begin_transaction(&tenant, TransactionOptions::new())
        .await
        .expect("the first transaction takes the only connection");

    let err = backend
        .process_transaction(&tenant, patient_bundle(), FhirVersion::default())
        .await
        .expect_err("the pool has no connection to give");

    match err {
        TransactionError::Transient { attempts, reason } => {
            assert_eq!(attempts, 1);
            assert!(
                reason.contains("timed out waiting for connection"),
                "the pool's error text is kept: {reason:?}"
            );
        }
        other => panic!("a pool timeout at BEGIN must be Transient, got {other:?}"),
    }

    Box::new(held).rollback().await.expect("rollback");
}

/// The direct `begin_transaction` path (used by bulk-submit and other callers)
/// reports the same typed error.
#[tokio::test]
async fn begin_transaction_reports_transient_for_a_held_write_lock() {
    use helios_persistence::error::StorageError;

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("direct.db");
    let backend = file_backend(&path, 200, 10, 30_000);

    let _holder = hold_write_lock(&path);
    let err = backend
        .begin_transaction(&tenant(), TransactionOptions::new())
        .await
        .expect_err("BEGIN IMMEDIATE cannot get the write lock");

    match err {
        StorageError::Transaction(TransactionError::Transient { attempts, reason }) => {
            assert_eq!(attempts, 1);
            assert_eq!(
                reason.matches("database is locked").count(),
                1,
                "{reason:?}"
            );
        }
        other => panic!("expected Transaction(Transient), got {other:?}"),
    }
}
