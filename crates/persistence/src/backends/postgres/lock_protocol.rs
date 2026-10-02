//! Transaction-scoped advisory lock protocol for PostgreSQL writes.
//!
//! Every write path (CRUD, transactions, bulk submit, reindex groups) locks a
//! bounded set of resources while other writers keep serving traffic for the
//! same tenant. These helpers give all sides a shared lock vocabulary so two
//! concurrent writers serialize on the same resource instead of racing to
//! leave the `current` search and FTS rows behind.
//!
//! # Protocol
//!
//! * The tenant gate key scopes everything to one tenant. Bounded write sets
//!   take it in `SHARED` mode, so disjoint resource sets can proceed
//!   concurrently. A SearchParameter write or tenant-wide operation takes it in
//!   `EXCLUSIVE` mode, which conflicts with every shared holder.
//! * Each resource in the group is then locked in `EXCLUSIVE` mode under the
//!   resource namespace, so two groups covering the same resource serialize,
//!   and a live writer that takes the same key serializes against the group.
//! * Keys are acquired in ascending signed order with duplicates removed, so
//!   concurrent groups holding overlapping sets cannot deadlock each other.
//!
//! All locks use the `pg_advisory_xact_lock` family and therefore live and die
//! with the caller's transaction: commit or rollback releases them. Callers
//! must already hold an open transaction on `client` before calling in.
//!
//! Key derivation is deterministic: signed FNV-1a-32 over the ASCII prefix
//! `HFS-PGI07-v1` plus a `NUL` byte, followed by each component as its UTF-8
//! bytes prefixed with its big-endian `u32` byte length. The gate hashes the
//! tenant id; a resource hashes tenant id, resource type, and logical id. The
//! two `0x4846534x` namespaces keep gate keys and resource keys in separate
//! advisory spaces even when the hashed values collide numerically.

use deadpool_postgres::GenericClient;
use tokio_postgres::error::SqlState;

use crate::error::{BackendError, StorageError, StorageResult, TransactionError};

/// Advisory-lock namespace for tenant gate keys (`HFSG` in ASCII).
pub(super) const TENANT_GATE_NAMESPACE: i32 = 0x4846_5347;

/// Advisory-lock namespace for per-resource keys (`HFSR` in ASCII).
pub(super) const RESOURCE_LOCK_NAMESPACE: i32 = 0x4846_5352;

/// Domain-separation prefix for key derivation: ASCII `HFS-PGI07-v1` plus NUL.
const HASH_PREFIX: &[u8] = b"HFS-PGI07-v1\0";

/// FNV-1a-32 offset basis.
const FNV_OFFSET_BASIS: u32 = 0x811c_9dc5;

/// FNV-1a-32 prime.
const FNV_PRIME: u32 = 0x0100_0193;

/// Builds the log text for a failed lock statement (#1637).
///
/// `tokio_postgres::Error`'s `Display` for a server error is the two words
/// "db error": the SQLSTATE and the server's message live in the `DbError` it
/// carries. `db` is that pair as `(sqlstate, message)`, and `None` for errors
/// that did not come from the server (I/O, connection closed, ...). A separate
/// function on plain parts because a `tokio_postgres::Error` with a server
/// error inside cannot be built outside the driver.
fn lock_error_message(context: &str, driver_text: &str, db: Option<(&str, &str)>) -> String {
    match db {
        Some((sqlstate, db_message)) => {
            format!("{context}: {driver_text} (SQLSTATE {sqlstate}: {db_message})")
        }
        None => format!("{context}: {driver_text}"),
    }
}

/// Wraps a failed lock statement, keeping the driver error as the source.
///
/// The source is what lets [`bundle_begin_error`] read the SQLSTATE back with
/// a typed downcast instead of parsing text, and the SQLSTATE and server
/// message are in the message so the log no longer says just "db error".
fn lock_error(context: &str, err: tokio_postgres::Error) -> StorageError {
    let message = lock_error_message(
        context,
        &err.to_string(),
        err.as_db_error().map(|db| (db.code().code(), db.message())),
    );
    StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: Some(Box::new(err)),
    })
}

/// Whether failing to take a write lock under `code` is a contention outcome
/// rather than a defect (#1637): the caller lost a race for the lock, nothing
/// was written, and an unchanged retry can succeed.
///
/// - `57014 query_canceled` — the wait outlived `statement_timeout`
///   (`HFS_PG_STATEMENT_TIMEOUT_MS`), the usual outcome behind a long-held gate.
/// - `55P03 lock_not_available` — a `lock_timeout` or `NOWAIT` gave up.
/// - `40P01 deadlock_detected` — the server picked this transaction as the
///   deadlock victim.
///
/// SQLSTATE and never message text: the server localizes its messages through
/// `lc_messages`. A pure function on the code so it needs no database to test.
pub(super) fn is_transient_lock_sqlstate(code: &SqlState) -> bool {
    // Compared with `==`, not matched: `SqlState` is non-structural-match.
    *code == SqlState::QUERY_CANCELED
        || *code == SqlState::LOCK_NOT_AVAILABLE
        || *code == SqlState::T_R_DEADLOCK_DETECTED
}

/// The SQLSTATE of the server error behind a failed lock statement, if there
/// was one: [`lock_error`] keeps the driver error as the source.
fn lock_failure_sqlstate(err: &StorageError) -> Option<&SqlState> {
    match err {
        StorageError::Backend(BackendError::Internal {
            source: Some(source),
            ..
        }) => source
            .downcast_ref::<tokio_postgres::Error>()
            .and_then(tokio_postgres::Error::code),
        _ => None,
    }
}

/// Maps a failed `begin_transaction` to the error a transaction Bundle ends
/// with (#1637).
///
/// A bundle that cannot take the tenant write gate for a transient reason (see
/// [`is_transient_lock_sqlstate`]) ends as [`TransactionError::Transient`], a
/// retryable 503, carrying the full error text — SQLSTATE included — in its
/// log-only `reason`. Every other begin failure keeps the
/// [`TransactionError::RolledBack`] wrapping it has always had.
///
/// Applied by the bundle path only: single-resource writes and bulk-submit
/// batches reach the same lock statements and keep the errors they had. Within
/// `begin_transaction` the exclusive gate is the only lock taken, and any other
/// begin failure that carries one of those SQLSTATEs is retryable all the same.
pub(super) fn bundle_begin_error(err: StorageError) -> TransactionError {
    let code = lock_failure_sqlstate(&err).cloned();
    begin_error_for(err, code.as_ref())
}

/// [`bundle_begin_error`] with the SQLSTATE already read out of the error, so
/// the mapping is testable without a driver error.
fn begin_error_for(err: StorageError, sqlstate: Option<&SqlState>) -> TransactionError {
    if sqlstate.is_some_and(is_transient_lock_sqlstate) {
        return TransactionError::Transient {
            attempts: 1,
            reason: err.to_string(),
        };
    }
    TransactionError::RolledBack {
        reason: format!("Failed to begin transaction: {err}"),
    }
}

fn oversize_error(what: &str, len: usize) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message: format!("write lock {what} too long: {len} bytes exceeds u32::MAX"),
        source: None,
    })
}

fn fnv1a_feed(hash: &mut u32, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u32::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn checked_len(len: usize, what: &str) -> StorageResult<u32> {
    u32::try_from(len).map_err(|_| oversize_error(what, len))
}

fn hash_components(components: &[&[u8]]) -> StorageResult<i32> {
    let mut hash = FNV_OFFSET_BASIS;
    fnv1a_feed(&mut hash, HASH_PREFIX);
    for component in components {
        let len = checked_len(component.len(), "key component")?;
        fnv1a_feed(&mut hash, &len.to_be_bytes());
        fnv1a_feed(&mut hash, component);
    }
    Ok(hash as i32)
}

/// Derives the tenant gate key for `tenant_id`.
///
/// Component lengths are UTF-8 byte lengths and must fit in a `u32`.
pub(super) fn gate_key(tenant_id: &str) -> StorageResult<i32> {
    hash_components(&[tenant_id.as_bytes()])
}

/// Derives the per-resource key for `tenant_id` / `resource_type` / `id`.
///
/// Component lengths are UTF-8 byte lengths and must fit in a `u32`.
pub(super) fn resource_key(tenant_id: &str, resource_type: &str, id: &str) -> StorageResult<i32> {
    hash_components(&[
        tenant_id.as_bytes(),
        resource_type.as_bytes(),
        id.as_bytes(),
    ])
}

/// Derives the per-resource keys for a group, deduplicated and sorted ascending.
///
/// Acquiring the returned keys in order is what keeps overlapping groups from
/// deadlocking each other.
pub(super) fn sorted_resource_keys<T, U>(
    tenant_id: &str,
    resources: &[(T, U)],
) -> StorageResult<Vec<i32>>
where
    T: AsRef<str>,
    U: AsRef<str>,
{
    let mut keys = Vec::with_capacity(resources.len());
    for (resource_type, id) in resources {
        keys.push(resource_key(
            tenant_id,
            resource_type.as_ref(),
            id.as_ref(),
        )?);
    }
    keys.sort_unstable();
    keys.dedup();
    Ok(keys)
}

async fn advisory_xact_lock(
    client: &impl GenericClient,
    namespace: i32,
    key: i32,
    shared: bool,
    context: &str,
) -> StorageResult<()> {
    let sql = if shared {
        "SELECT pg_advisory_xact_lock_shared($1, $2)"
    } else {
        "SELECT pg_advisory_xact_lock($1, $2)"
    };
    client
        .query_one(sql, &[&namespace, &key])
        .await
        .map_err(|e| lock_error(context, e))?;
    Ok(())
}

/// Locks a bounded write set: shared tenant gate, then exclusive resource keys.
///
/// This is the general PostgreSQL write-protocol path shared by CRUD,
/// transactions, bulk submit, and reindex groups: the shared gate lets write
/// sets for the same tenant run concurrently while still conflicting with an
/// exclusive whole-tenant gate, and the exclusive resource keys serialize
/// overlapping write sets against each other. Keys are acquired in ascending
/// order.
///
/// The caller must hold an open transaction on `client`; the locks release
/// when it commits or rolls back.
pub(super) async fn acquire_shared_write_locks<T, U>(
    client: &impl GenericClient,
    tenant_id: &str,
    resources: &[(T, U)],
) -> StorageResult<()>
where
    T: AsRef<str> + Sync,
    U: AsRef<str> + Sync,
{
    let gate = gate_key(tenant_id)?;
    let keys = sorted_resource_keys(tenant_id, resources)?;
    advisory_xact_lock(
        client,
        TENANT_GATE_NAMESPACE,
        gate,
        true,
        "failed to acquire shared write gate lock",
    )
    .await?;
    if keys.len() == 1 {
        advisory_xact_lock(
            client,
            RESOURCE_LOCK_NAMESPACE,
            keys[0],
            false,
            "failed to acquire write resource lock",
        )
        .await?;
    } else if !keys.is_empty() {
        // `batch_execute` sends these independent simple-query statements in
        // one message. PostgreSQL executes them in text order, so locks remain
        // sorted even if a hash collision or opposing write set is present.
        // The text contains only locally derived i32 keys and a fixed
        // namespace, never tenant or resource strings.
        let mut sql = String::with_capacity(keys.len() * 58);
        for key in keys {
            use std::fmt::Write;
            writeln!(
                sql,
                "SELECT pg_advisory_xact_lock({RESOURCE_LOCK_NAMESPACE}, {key});"
            )
            .expect("writing to String cannot fail");
        }
        client
            .batch_execute(&sql)
            .await
            .map_err(|e| lock_error("failed to acquire write resource locks", e))?;
    }
    Ok(())
}

/// Locks the whole tenant: exclusive tenant gate, no resource keys.
///
/// The exclusive gate conflicts with every shared gate holder, so this waits
/// until no group is running for the tenant and blocks new groups until the
/// caller's transaction ends.
pub(super) async fn acquire_exclusive_write_gate(
    client: &impl GenericClient,
    tenant_id: &str,
) -> StorageResult<()> {
    let gate = gate_key(tenant_id)?;
    advisory_xact_lock(
        client,
        TENANT_GATE_NAMESPACE,
        gate,
        false,
        "failed to acquire exclusive write gate lock",
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_key_vectors() {
        assert_eq!(gate_key("acme").unwrap(), -2026577464);
    }

    #[test]
    fn resource_key_vectors() {
        assert_eq!(resource_key("acme", "Patient", "123").unwrap(), 261144589);
        assert_eq!(resource_key("beta", "Patient", "123").unwrap(), 999185681);
    }

    #[test]
    fn keys_differ_across_tenants_namespaces_and_kinds() {
        assert_ne!(TENANT_GATE_NAMESPACE, RESOURCE_LOCK_NAMESPACE);
        assert_ne!(gate_key("acme").unwrap(), gate_key("beta").unwrap());
        assert_ne!(
            resource_key("acme", "Patient", "123").unwrap(),
            resource_key("beta", "Patient", "123").unwrap()
        );
        assert_ne!(
            resource_key("acme", "Patient", "123").unwrap(),
            resource_key("acme", "Observation", "123").unwrap()
        );
        assert_ne!(
            resource_key("acme", "Patient", "123").unwrap(),
            resource_key("acme", "Patient", "456").unwrap()
        );
    }

    #[test]
    fn sorted_resource_keys_dedup_and_sort_ascending() {
        let pairs = vec![
            ("Patient".to_string(), "123".to_string()),
            ("Observation".to_string(), "abc".to_string()),
            ("Patient".to_string(), "123".to_string()),
            ("Patient".to_string(), "456".to_string()),
            ("Observation".to_string(), "abc".to_string()),
        ];
        let keys = sorted_resource_keys("acme", &pairs).unwrap();
        assert_eq!(keys.len(), 3);
        assert!(keys.windows(2).all(|w| w[0] < w[1]));

        let mut expected = vec![
            resource_key("acme", "Patient", "123").unwrap(),
            resource_key("acme", "Observation", "abc").unwrap(),
            resource_key("acme", "Patient", "456").unwrap(),
        ];
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(keys, expected);

        let empty: Vec<(String, String)> = Vec::new();
        assert!(sorted_resource_keys("acme", &empty).unwrap().is_empty());
    }

    #[test]
    fn checked_len_rejects_lengths_beyond_u32() {
        assert_eq!(checked_len(0, "key component").unwrap(), 0);
        assert_eq!(checked_len(3, "key component").unwrap(), 3);
        assert_eq!(
            checked_len(u32::MAX as usize, "key component").unwrap(),
            u32::MAX
        );
        assert!(checked_len(u32::MAX as usize + 1, "key component").is_err());
        assert!(checked_len(usize::MAX, "key component").is_err());
    }

    /// #1637: statement timeout, lock timeout and deadlock victim are
    /// contention; everything else — constraint violations, a serialization
    /// failure (a different question, see `classify_postgres_error`), a server
    /// going away, a missing table — is not.
    #[test]
    fn transient_lock_sqlstates_are_exactly_cancel_lock_unavailable_and_deadlock() {
        assert!(is_transient_lock_sqlstate(&SqlState::QUERY_CANCELED));
        assert!(is_transient_lock_sqlstate(&SqlState::LOCK_NOT_AVAILABLE));
        assert!(is_transient_lock_sqlstate(&SqlState::T_R_DEADLOCK_DETECTED));

        // The same codes as the server sends them.
        for code in ["57014", "55P03", "40P01"] {
            assert!(
                is_transient_lock_sqlstate(&SqlState::from_code(code)),
                "{code}"
            );
        }
        for code in [
            "23505", // unique_violation
            "40001", // serialization_failure
            "57P01", // admin_shutdown
            "53300", // too_many_connections
            "42P01", // undefined_table
            "42501", // insufficient_privilege
            "25P02", // in_failed_sql_transaction
            "XX000", // internal_error
            "00000", // successful_completion
        ] {
            assert!(
                !is_transient_lock_sqlstate(&SqlState::from_code(code)),
                "{code} must not be transient"
            );
        }
    }

    /// The log text for a server error names its SQLSTATE and message instead
    /// of the driver's bare "db error".
    #[test]
    fn lock_error_message_includes_sqlstate_and_the_server_message() {
        assert_eq!(
            lock_error_message(
                "failed to acquire exclusive write gate lock",
                "db error",
                Some(("57014", "canceling statement due to statement timeout")),
            ),
            "failed to acquire exclusive write gate lock: db error \
             (SQLSTATE 57014: canceling statement due to statement timeout)"
        );
        // Not a server error: nothing to add.
        assert_eq!(
            lock_error_message(
                "failed to acquire write resource locks",
                "connection closed",
                None
            ),
            "failed to acquire write resource locks: connection closed"
        );
    }

    /// `lock_error` keeps the driver error as the source — the typed handle
    /// the bundle path reads the SQLSTATE through. (A driver error carrying a
    /// server error cannot be built outside the driver; the integration test
    /// `postgres_integration_transaction_bundle_gate_timeout_is_transient`
    /// covers that half against a live server.)
    #[test]
    fn lock_error_keeps_the_driver_error_as_source() {
        let wrapped = lock_error(
            "failed to acquire shared write gate lock",
            tokio_postgres::Error::__private_api_timeout(),
        );
        match &wrapped {
            StorageError::Backend(BackendError::Internal {
                backend_name,
                message,
                source: Some(source),
            }) => {
                assert_eq!(backend_name, "postgres");
                assert_eq!(
                    message,
                    "failed to acquire shared write gate lock: timeout waiting for server"
                );
                assert!(source.downcast_ref::<tokio_postgres::Error>().is_some());
            }
            other => panic!("expected Internal with a source, got {other:?}"),
        }
        // No server error behind it, so no SQLSTATE to classify on.
        assert!(lock_failure_sqlstate(&wrapped).is_none());
    }

    /// #1637: a transient SQLSTATE ends the bundle as `Transient`
    /// (`attempts: 1`, full text in the reason); any other begin failure keeps
    /// its `RolledBack` wrapping.
    #[test]
    fn begin_error_is_transient_only_for_a_transient_sqlstate() {
        let gate_failure = || {
            StorageError::Backend(BackendError::Internal {
                backend_name: "postgres".to_string(),
                message: lock_error_message(
                    "failed to acquire exclusive write gate lock",
                    "db error",
                    Some(("57014", "canceling statement due to statement timeout")),
                ),
                source: None,
            })
        };

        for code in [
            SqlState::QUERY_CANCELED,
            SqlState::LOCK_NOT_AVAILABLE,
            SqlState::T_R_DEADLOCK_DETECTED,
        ] {
            match begin_error_for(gate_failure(), Some(&code)) {
                TransactionError::Transient { attempts, reason } => {
                    assert_eq!(attempts, 1);
                    assert_eq!(
                        reason,
                        "internal error in postgres: failed to acquire exclusive write gate \
                         lock: db error (SQLSTATE 57014: canceling statement due to \
                         statement timeout)"
                    );
                }
                other => panic!("{code:?} must be Transient, got {other:?}"),
            }
        }

        // Not transient: the flattening it has always had.
        for code in [Some(&SqlState::UNIQUE_VIOLATION), None] {
            match begin_error_for(gate_failure(), code) {
                TransactionError::RolledBack { reason } => assert!(
                    reason.starts_with("Failed to begin transaction: internal error in postgres:"),
                    "{reason:?}"
                ),
                other => panic!("{code:?} must stay RolledBack, got {other:?}"),
            }
        }
    }

    /// A begin failure that is not a lock error — the BEGIN statement itself
    /// failed, say — is passed through the public entry point as `RolledBack`.
    #[test]
    fn bundle_begin_error_without_a_driver_source_is_rolled_back() {
        let err = StorageError::Transaction(TransactionError::RolledBack {
            reason: "Failed to begin transaction: connection closed".to_string(),
        });
        match bundle_begin_error(err) {
            TransactionError::RolledBack { reason } => assert_eq!(
                reason,
                "Failed to begin transaction: transaction rolled back: \
                 Failed to begin transaction: connection closed"
            ),
            other => panic!("expected RolledBack, got {other:?}"),
        }
    }
}
