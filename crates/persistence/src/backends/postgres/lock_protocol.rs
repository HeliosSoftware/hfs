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
//! * A transaction Bundle may additionally take a *criteria* key, for a
//!   conditional create (`ifNoneExist`) whose search found nothing. The key is
//!   only ever taken `EXCLUSIVE`, to create under it, and is then held to
//!   `COMMIT`: the Bundle searches again and creates only if it still finds
//!   nothing, so two Bundles with the same criteria cannot both create (#1637).
//!   The Bundle first tries the key without waiting. When another Bundle holds
//!   it, it waits for that Bundle to end without keeping the key (`SHARE` mode,
//!   in a savepoint that is rolled back), so all waiters on one creator wake
//!   together and hold nothing (#1747). It searches again, and only on a miss
//!   takes the key `EXCLUSIVE`, blocking, searches a third time and creates.
//!   Criteria keys are taken *after* the gate and the resource keys, one at a
//!   time. Two Bundles that each hold a criteria key and wait for the other's,
//!   in either mode, deadlock; PostgreSQL detects that (`40P01`). A criteria
//!   lock wait that PostgreSQL ends -- as a deadlock victim, with a statement or
//!   lock timeout (`57014`, `55P03`) or because the lock table is full
//!   (`53200`) -- ends the Bundle as a retryable [`TransactionError::Transient`],
//!   see [`acquire_criteria_lock`] and [`transient_bundle_error`]. Only lock
//!   waits map to it: a timeout on any other statement of a Bundle, an index or
//!   full-text write say, is not a lock outcome and stays a `BundleError`.
//!
//! All locks use the `pg_advisory_xact_lock` family and therefore live and die
//! with the caller's transaction: commit or rollback releases them. Callers
//! must already hold an open transaction on `client` before calling in.
//!
//! Key derivation is deterministic: signed FNV-1a-32 over the ASCII prefix
//! `HFS-PGI07-v1` plus a `NUL` byte, followed by each component as its UTF-8
//! bytes prefixed with its big-endian `u32` byte length. The gate hashes the
//! tenant id; a resource hashes tenant id, resource type, and logical id; a
//! criteria key hashes a `criteria` kind tag, tenant id, resource type, the
//! number of criteria, and each name and value of the sorted, decoded
//! criteria. The three `0x4846534x` namespaces keep gate, resource and criteria
//! keys in separate advisory spaces even when the hashed values collide
//! numerically.

use deadpool_postgres::GenericClient;
use tokio_postgres::error::SqlState;

use crate::error::{BackendError, StorageError, StorageResult, TransactionError};

/// Advisory-lock namespace for tenant gate keys (`HFSG` in ASCII).
pub(super) const TENANT_GATE_NAMESPACE: i32 = 0x4846_5347;

/// Advisory-lock namespace for per-resource keys (`HFSR` in ASCII).
pub(super) const RESOURCE_LOCK_NAMESPACE: i32 = 0x4846_5352;

/// Advisory-lock namespace for conditional-create criteria keys (`HFSC` in
/// ASCII).
pub(super) const CRITERIA_LOCK_NAMESPACE: i32 = 0x4846_5343;

/// Kind tag hashed first into a criteria key, so a criteria key can never equal
/// a gate or resource key built from the same strings.
const CRITERIA_KIND: &[u8] = b"criteria";

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
/// - `53200 out_of_memory` — "out of shared memory ... You might need to
///   increase max_locks_per_transaction": the server's lock table is full. An
///   advisory lock takes one of its slots (they never use the per-backend fast
///   path), so a lock statement can be the one that finds it full. The table
///   frees as the transactions holding it end, so an unchanged retry can
///   succeed; nothing in the request was the problem.
///
/// Only the SQLSTATE of a *lock statement* is read with this (see
/// [`bundle_begin_error`] and [`acquire_criteria_lock`]). The same codes on any
/// other statement of a Bundle are not lock outcomes and keep the error they
/// had.
///
/// SQLSTATE and never message text: the server localizes its messages through
/// `lc_messages`. A pure function on the code so it needs no database to test.
pub(super) fn is_transient_lock_sqlstate(code: &SqlState) -> bool {
    // Compared with `==`, not matched: `SqlState` is non-structural-match.
    *code == SqlState::QUERY_CANCELED
        || *code == SqlState::LOCK_NOT_AVAILABLE
        || *code == SqlState::T_R_DEADLOCK_DETECTED
        || *code == SqlState::OUT_OF_MEMORY
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

/// Maps a failed begin of a transaction Bundle to the error it ends with
/// (#1637).
///
/// A bundle that cannot take the tenant write gate, or the resource keys of its
/// lock plan, for a transient reason (see [`is_transient_lock_sqlstate`]) ends
/// as [`TransactionError::Transient`], a retryable 503, carrying the full error
/// text — SQLSTATE included — in its log-only `reason`. Every other begin
/// failure keeps the [`TransactionError::RolledBack`] wrapping it has always
/// had.
///
/// Applied by the bundle path only: single-resource writes and bulk-submit
/// batches reach the same lock statements and keep the errors they had.
///
/// Only lock statements are classified, because they are the only step of a
/// begin whose failure keeps a driver error as its source ([`lock_error`]): a
/// failed `BEGIN`, or a failed read of the registry, carries none, so a
/// timeout on those is no lock outcome and stays `RolledBack`.
pub(super) fn bundle_begin_error(err: StorageError) -> TransactionError {
    let code = lock_failure_sqlstate(&err).cloned();
    begin_error_for(err, code.as_ref())
}

/// [`bundle_begin_error`] with the SQLSTATE already read out of the error, so
/// the mapping is testable without a driver error.
fn begin_error_for(err: StorageError, sqlstate: Option<&SqlState>) -> TransactionError {
    transient_error_for(&err, sqlstate).unwrap_or_else(|| TransactionError::RolledBack {
        reason: format!("Failed to begin transaction: {err}"),
    })
}

/// Recognises the failure of a transaction Bundle's *criteria lock* statement
/// that [`try_criteria_lock`], [`wait_for_criteria_lock`] or
/// [`acquire_criteria_lock`] marked as retryable, and returns the
/// [`TransactionError::Transient`] the Bundle ends with (#1637).
///
/// A Bundle that holds the tenant gate shared can wait for a criteria lock (see
/// the module docs), and PostgreSQL can end that wait as a deadlock victim
/// (`40P01`), as a statement timeout (`57014`), as `55P03` or because the lock
/// table is full (`53200`). Nothing was committed and an unchanged retry can
/// succeed, which is exactly what [`bundle_begin_error`] already says of the
/// same SQLSTATEs at begin time.
///
/// Only that marked failure qualifies. The marker is an explicit one — the
/// [`TransactionError::Transient`] that the criteria lock statements put in the
/// [`StorageError`] — and not a test of the error's shape, because a search
/// index or full-text write (`search/writer.rs`, `storage.rs`) fails with the
/// very same shape, a [`BackendError::Internal`] with the driver error as its
/// source. A statement timeout on one of those is not a lock wait: a Bundle too
/// big to index within the timeout would time out on every retry, so it keeps
/// the `BundleError` it has always had, wherever in the Bundle the write
/// happens to be flushed.
pub(super) fn transient_bundle_error(err: &StorageError) -> Option<TransactionError> {
    match err {
        StorageError::Transaction(TransactionError::Transient { attempts, reason }) => {
            Some(TransactionError::Transient {
                attempts: *attempts,
                reason: reason.clone(),
            })
        }
        _ => None,
    }
}

/// Marks the failure of a criteria lock statement for
/// [`transient_bundle_error`], with the SQLSTATE already read out of the error
/// so the mapping is testable without a driver error.
///
/// A transient SQLSTATE (see [`is_transient_lock_sqlstate`]) turns the failure
/// into [`TransactionError::Transient`] carrying the full error text — SQLSTATE
/// included — as its log-only `reason`. Any other failure of the statement is
/// returned as it was.
fn criteria_lock_failure_for(err: StorageError, sqlstate: Option<&SqlState>) -> StorageError {
    match transient_error_for(&err, sqlstate) {
        Some(transient) => StorageError::Transaction(transient),
        None => err,
    }
}

/// The [`TransactionError::Transient`] for a lock statement that failed with
/// `sqlstate`, when that is a transient one.
fn transient_error_for(
    err: &StorageError,
    sqlstate: Option<&SqlState>,
) -> Option<TransactionError> {
    sqlstate
        .is_some_and(is_transient_lock_sqlstate)
        .then(|| TransactionError::Transient {
            attempts: 1,
            reason: err.to_string(),
        })
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

/// Derives the criteria key for a conditional create of `resource_type` in
/// `tenant_id` with the decoded `criteria` pairs (#1637).
///
/// The pairs are sorted and deduplicated first, so the parameter order, and a
/// parameter repeated verbatim, do not change the key. Two creators whose
/// criteria differ only in such spelling therefore contend on one lock. Criteria
/// that are the same search spelt differently in other ways (an OR list in
/// another order, say) get different keys, and only lose the serialisation: the
/// two can both create. A hash collision only causes false contention.
pub(super) fn criteria_key(
    tenant_id: &str,
    resource_type: &str,
    criteria: &[(String, String)],
) -> StorageResult<i32> {
    let mut pairs: Vec<&(String, String)> = criteria.iter().collect();
    pairs.sort_unstable();
    pairs.dedup();
    // The pair count is a component of its own, so a different number of pairs
    // can never hash the same bytes.
    let count = u32::try_from(pairs.len())
        .map_err(|_| oversize_error("criteria count", pairs.len()))?
        .to_be_bytes();
    let mut components: Vec<&[u8]> = Vec::with_capacity(4 + pairs.len() * 2);
    components.extend([
        CRITERIA_KIND,
        tenant_id.as_bytes(),
        resource_type.as_bytes(),
        &count,
    ]);
    for (name, value) in pairs {
        components.push(name.as_bytes());
        components.push(value.as_bytes());
    }
    hash_components(&components)
}

/// The criteria key of the `ifNoneExist` text `criteria`, or `None` when it
/// names nothing a search could match on, so there is nothing to serialise.
///
/// Parsed with [`crate::search::parse_conditional_criteria`], the parser the
/// search the criteria drive uses, so `a%7Cb` and `a|b` are one key. Result
/// parameters (`_count`, `_format`, ...) are no criteria and are left out, as
/// the search leaves them out.
pub(super) fn criteria_key_for(
    tenant_id: &str,
    resource_type: &str,
    criteria: &str,
) -> StorageResult<Option<i32>> {
    let mut pairs = crate::search::parse_conditional_criteria(criteria);
    pairs.retain(|(name, _)| !crate::search::conditional::RESULT_PARAMS.contains(&name.as_str()));
    if pairs.is_empty() {
        return Ok(None);
    }
    criteria_key(tenant_id, resource_type, &pairs).map(Some)
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

/// Maps a failed criteria lock statement for [`transient_bundle_error`]: a
/// transient SQLSTATE (see [`is_transient_lock_sqlstate`]) becomes
/// [`TransactionError::Transient`], any other failure keeps the lock error it
/// was.
fn criteria_lock_failure(err: StorageError) -> StorageError {
    let code = lock_failure_sqlstate(&err).cloned();
    criteria_lock_failure_for(err, code.as_ref())
}

/// Name of the savepoint [`wait_for_criteria_lock`] takes and releases.
const CRITERIA_WAIT_SAVEPOINT: &str = "hfs_criteria_wait";

/// The statements of [`wait_for_criteria_lock`], as one simple-query batch.
///
/// The `RELEASE` after the `ROLLBACK TO` is load-bearing: `ROLLBACK TO` leaves
/// the savepoint open, and an open subtransaction would wrap the rest of the
/// Bundle. The text contains only the locally derived i32 `key` and fixed
/// names, never tenant strings, as in [`acquire_shared_write_locks`].
fn criteria_wait_sql(key: i32) -> String {
    format!(
        "SAVEPOINT {CRITERIA_WAIT_SAVEPOINT}; \
         SELECT pg_advisory_xact_lock_shared({CRITERIA_LOCK_NAMESPACE}, {key}); \
         ROLLBACK TO SAVEPOINT {CRITERIA_WAIT_SAVEPOINT}; \
         RELEASE SAVEPOINT {CRITERIA_WAIT_SAVEPOINT}"
    )
}

/// Tries to take the criteria lock `key` in `EXCLUSIVE` mode without waiting.
///
/// Returns `true` when the key was taken, and it is then held to `COMMIT`.
/// `false` means another transaction holds the key or waits for it, and nothing
/// was taken.
///
/// A failure is marked for [`transient_bundle_error`] like the other criteria
/// lock statements.
pub(super) async fn try_criteria_lock(
    client: &impl GenericClient,
    key: i32,
) -> StorageResult<bool> {
    let row = client
        .query_one(
            "SELECT pg_try_advisory_xact_lock($1, $2)",
            &[&CRITERIA_LOCK_NAMESPACE, &key],
        )
        .await
        .map_err(|e| criteria_lock_failure(lock_error("failed to try criteria lock", e)))?;
    Ok(row.get::<_, bool>(0))
}

/// Waits until no other transaction holds the criteria lock `key` `EXCLUSIVE`,
/// and returns holding nothing (#1747).
///
/// The wait takes the key in `SHARE` mode inside a savepoint and gives it
/// straight back by rolling back to the savepoint, so every transaction waiting
/// on one holder is granted together when the holder ends, and none of them
/// keeps the key. Pinned by
/// `postgres_integration_bundle_lock_rollback_to_savepoint_releases_an_advisory_xact_lock`.
/// The caller has buffered rows only in memory, so the rollback undoes nothing
/// but the lock.
///
/// A deadlock, timeout, cancel or full lock table here is
/// [`TransactionError::Transient`], as for the other criteria lock statements.
pub(super) async fn wait_for_criteria_lock(
    client: &impl GenericClient,
    key: i32,
) -> StorageResult<()> {
    client
        .batch_execute(&criteria_wait_sql(key))
        .await
        .map_err(|e| criteria_lock_failure(lock_error("failed to wait for criteria lock", e)))
}

/// Takes the criteria lock `key` in `EXCLUSIVE` mode, waiting for its holder.
///
/// Called by a transaction Bundle that already holds the shared tenant gate and
/// its resource keys, for a conditional create whose search found nothing, after
/// [`try_criteria_lock`] failed and the search that followed
/// [`wait_for_criteria_lock`] still found nothing. It blocks, so it can deadlock
/// against another Bundle taking criteria keys in the opposite order, or wait
/// out `statement_timeout`.
///
/// A failure for a transient reason (see [`is_transient_lock_sqlstate`]: a
/// deadlock victim, a timeout, a full lock table) is returned as
/// [`StorageError::Transaction`] holding [`TransactionError::Transient`], which
/// is what [`transient_bundle_error`] recognises. Only the criteria lock
/// statements ([`try_criteria_lock`], [`wait_for_criteria_lock`] and this one)
/// make one mid-Bundle, so no other failing statement can become `Transient` by
/// being mistaken for it. Any other failure keeps the lock error it was.
pub(super) async fn acquire_criteria_lock(
    client: &impl GenericClient,
    key: i32,
) -> StorageResult<()> {
    advisory_xact_lock(
        client,
        CRITERIA_LOCK_NAMESPACE,
        key,
        false,
        "failed to acquire criteria lock",
    )
    .await
    .map_err(criteria_lock_failure)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn criteria_wait_sql_takes_the_key_shared_in_a_savepoint_and_gives_it_back() {
        assert_eq!(
            criteria_wait_sql(-594198615),
            "SAVEPOINT hfs_criteria_wait; \
             SELECT pg_advisory_xact_lock_shared(1212568387, -594198615); \
             ROLLBACK TO SAVEPOINT hfs_criteria_wait; \
             RELEASE SAVEPOINT hfs_criteria_wait"
        );
    }

    #[test]
    fn gate_key_vectors() {
        assert_eq!(gate_key("acme").unwrap(), -2026577464);
    }

    #[test]
    fn resource_key_vectors() {
        assert_eq!(resource_key("acme", "Patient", "123").unwrap(), 261144589);
        assert_eq!(resource_key("beta", "Patient", "123").unwrap(), 999185681);
    }

    fn pairs(raw: &[(&str, &str)]) -> Vec<(String, String)> {
        raw.iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect()
    }

    /// Pinned, like the gate and resource keys: the key is a cross-process
    /// protocol, so a change to its derivation is a fleet-wide change. The
    /// values were computed independently of this module (FNV-1a-32 over the
    /// documented byte layout).
    #[test]
    fn criteria_key_vectors() {
        assert_eq!(
            criteria_key(
                "acme",
                "Organization",
                &pairs(&[("identifier", "http://example.org|1")])
            )
            .unwrap(),
            -594198615
        );
        assert_eq!(
            criteria_key("acme", "Organization", &pairs(&[("a", "1"), ("b", "2")])).unwrap(),
            -1824396254
        );
    }

    /// The order of the parameters, and a parameter repeated verbatim, do not
    /// change which lock two creators contend on.
    #[test]
    fn criteria_key_ignores_pair_order_and_verbatim_repeats() {
        let one = criteria_key("acme", "Organization", &pairs(&[("a", "1"), ("b", "2")])).unwrap();
        let swapped =
            criteria_key("acme", "Organization", &pairs(&[("b", "2"), ("a", "1")])).unwrap();
        let repeated = criteria_key(
            "acme",
            "Organization",
            &pairs(&[("a", "1"), ("b", "2"), ("a", "1")]),
        )
        .unwrap();
        assert_eq!(one, swapped);
        assert_eq!(one, repeated);
    }

    #[test]
    fn criteria_key_differs_by_tenant_type_and_criteria() {
        let base = criteria_key("acme", "Organization", &pairs(&[("a", "1")])).unwrap();
        for other in [
            criteria_key("beta", "Organization", &pairs(&[("a", "1")])).unwrap(),
            criteria_key("acme", "Practitioner", &pairs(&[("a", "1")])).unwrap(),
            criteria_key("acme", "Organization", &pairs(&[("a", "2")])).unwrap(),
            criteria_key("acme", "Organization", &pairs(&[("b", "1")])).unwrap(),
            criteria_key("acme", "Organization", &pairs(&[("a", "1"), ("a", "2")])).unwrap(),
            // Name and value boundaries are length-prefixed, not concatenated.
            criteria_key("acme", "Organization", &pairs(&[("a1", "")])).unwrap(),
            criteria_key("acme", "Organization", &pairs(&[("", "a1")])).unwrap(),
        ] {
            assert_ne!(base, other);
        }
    }

    /// The key is built from what the search would parse, so spellings the
    /// search treats as one are one key.
    #[test]
    fn criteria_key_for_decodes_and_drops_result_parameters() {
        let plain = criteria_key_for("acme", "Organization", "identifier=http://example.org|1")
            .unwrap()
            .unwrap();
        assert_eq!(
            plain,
            criteria_key(
                "acme",
                "Organization",
                &pairs(&[("identifier", "http://example.org|1")])
            )
            .unwrap()
        );
        for spelling in [
            "identifier=http%3A%2F%2Fexample.org%7C1",
            "identifier=http://example.org|1&_count=5",
            "_format=json&identifier=http://example.org|1",
        ] {
            assert_eq!(
                criteria_key_for("acme", "Organization", spelling)
                    .unwrap()
                    .unwrap(),
                plain,
                "{spelling}"
            );
        }
        // Nothing to match on: no lock, as no search is run on it either.
        assert_eq!(criteria_key_for("acme", "Organization", "").unwrap(), None);
        assert_eq!(
            criteria_key_for("acme", "Organization", "_count=5").unwrap(),
            None
        );
    }

    #[test]
    fn keys_differ_across_tenants_namespaces_and_kinds() {
        assert_ne!(CRITERIA_LOCK_NAMESPACE, TENANT_GATE_NAMESPACE);
        assert_ne!(CRITERIA_LOCK_NAMESPACE, RESOURCE_LOCK_NAMESPACE);
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

    /// #1637: statement timeout, lock timeout, deadlock victim and a full lock
    /// table are contention; everything else — constraint violations, a
    /// serialization failure (a different question, see
    /// `classify_postgres_error`), a server going away, a missing table, other
    /// resource exhaustion — is not.
    #[test]
    fn transient_lock_sqlstates_are_exactly_cancel_lock_unavailable_deadlock_and_lock_table_full() {
        assert!(is_transient_lock_sqlstate(&SqlState::QUERY_CANCELED));
        assert!(is_transient_lock_sqlstate(&SqlState::LOCK_NOT_AVAILABLE));
        assert!(is_transient_lock_sqlstate(&SqlState::T_R_DEADLOCK_DETECTED));
        assert!(is_transient_lock_sqlstate(&SqlState::OUT_OF_MEMORY));

        // The same codes as the server sends them. 53200 is `out of shared
        // memory ... You might need to increase max_locks_per_transaction`, the
        // lock table running out of slots (an advisory lock takes one).
        for code in ["57014", "55P03", "40P01", "53200"] {
            assert!(
                is_transient_lock_sqlstate(&SqlState::from_code(code)),
                "{code}"
            );
        }
        for code in [
            "23505", // unique_violation
            "40001", // serialization_failure
            "57P01", // admin_shutdown
            "53100", // disk_full
            "53300", // too_many_connections
            "53400", // configuration_limit_exceeded
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
            // The lock table is full: the gate or a resource key of the plan
            // could not get a slot.
            SqlState::OUT_OF_MEMORY,
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

    /// The failure of a lock statement, as a Bundle meets it: an internal
    /// postgres error whose text names the SQLSTATE the server sent.
    fn lock_statement_failure(context: &str, sqlstate: &str, message: &str) -> StorageError {
        StorageError::Backend(BackendError::Internal {
            backend_name: "postgres".to_string(),
            message: lock_error_message(context, "db error", Some((sqlstate, message))),
            source: None,
        })
    }

    /// #1637: the failure of a criteria lock wait is marked `Transient` for the
    /// four SQLSTATEs a lock wait can end with -- timeout or cancel, lock
    /// timeout, deadlock victim, lock table full -- with the SQLSTATE in the
    /// reason, and for nothing else.
    #[test]
    fn criteria_lock_failure_is_transient_only_for_a_transient_sqlstate() {
        let failure = || {
            lock_statement_failure(
                "failed to acquire criteria lock",
                "40P01",
                "deadlock detected",
            )
        };

        for code in [
            SqlState::QUERY_CANCELED,
            SqlState::LOCK_NOT_AVAILABLE,
            SqlState::T_R_DEADLOCK_DETECTED,
            SqlState::OUT_OF_MEMORY,
        ] {
            let marked = criteria_lock_failure_for(failure(), Some(&code));
            match transient_bundle_error(&marked) {
                Some(TransactionError::Transient { attempts, reason }) => {
                    assert_eq!(attempts, 1);
                    assert!(reason.contains("criteria lock"), "{reason}");
                    assert!(reason.contains("SQLSTATE 40P01"), "{reason}");
                }
                other => panic!("{code:?} must be Transient, got {other:?}"),
            }
        }

        // Any other failure of the statement is returned as it was, and is not
        // recognised.
        for code in [Some(&SqlState::UNIQUE_VIOLATION), None] {
            let unmarked = criteria_lock_failure_for(failure(), code);
            assert!(
                matches!(
                    unmarked,
                    StorageError::Backend(BackendError::Internal { .. })
                ),
                "{code:?} must keep the lock error: {unmarked:?}"
            );
            assert!(
                transient_bundle_error(&unmarked).is_none(),
                "{code:?} must stay a BundleError"
            );
        }
    }

    /// #1637: only the marked criteria-lock failure is `Transient`. Every
    /// other error a Bundle's entry can raise -- among them the failure of a
    /// search-index or full-text write, which has the shape of a lock
    /// statement's failure (an internal postgres error with a driver error as
    /// its source) -- is not recognised, whatever SQLSTATE sits behind it.
    #[test]
    fn nothing_but_the_marked_criteria_lock_failure_is_transient() {
        // A write of the search index that timed out: `internal_postgres_error`
        // keeps the driver error as the source, as `lock_error` does.
        let index_write = StorageError::Backend(BackendError::Internal {
            backend_name: "postgres".to_string(),
            message: "Failed to insert search index rows: canceling statement due to \
                      statement timeout"
                .to_string(),
            source: Some(Box::new(tokio_postgres::Error::__private_api_timeout())),
        });
        // The same text as a lock statement's, from a statement that is not one.
        let look_alike = lock_statement_failure(
            "Failed to write resource_fts rows",
            "57014",
            "canceling statement due to statement timeout",
        );
        for other in [
            index_write,
            look_alike,
            StorageError::Transaction(TransactionError::RolledBack {
                reason: "resource Patient/x was not in the transaction lock plan".to_string(),
            }),
            StorageError::Transaction(TransactionError::InvalidTransaction),
        ] {
            assert!(transient_bundle_error(&other).is_none(), "{other:?}");
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
