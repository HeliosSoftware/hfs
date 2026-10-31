//! Drops each test's database once its test has ended (#2311).
//!
//! `mongodb_tests` gives every test its own database (`build_test_database_name`)
//! and builds the full schema in it. Left alone, all ~560 of them stay in the one
//! shared mongod until the run ends, so WiredTiger's data handles and checkpoint
//! time grow through the run and every write and schema command slows down.
//!
//! # Contract
//!
//! A database named by `build_test_database_name` is dropped once the **thread
//! that named it** has ended. That thread is normally the libtest thread running
//! the test (libtest names it after the test), and `#[tokio::test]` bodies, also
//! the `multi_thread` ones, run on it. So:
//!
//! - name test databases from the test body, not from a spawned thread or task;
//! - a database lives as long as its test, however many backends the test opens
//!   on it, and a test may drop it earlier itself (a second `dropDatabase` is a
//!   no-op);
//! - threads without a name, named `main` or named `tokio-runtime-worker*` are
//!   not registered: their databases are left in place, as before this module,
//!   with a one-time warning on stderr.
//!
//! The `create_backend*`, `build_backend` and `raw_test_client` signatures are
//! unchanged, so no call site needs to know about this module.
//!
//! # How
//!
//! Each name is registered against a thread-local [`OwnerGuard`] of the calling
//! thread. Its destructor only stores `alive = false`; it never blocks. A
//! dedicated sweeper thread (own current-thread tokio runtime, one long-lived
//! `Client` with `appName` `hfs-test-db-cleanup`) takes the entries of finished
//! owners every [`SWEEP_INTERVAL`] and, on exactly those names, never on a prefix
//! pattern, so a developer's own `HFS_TEST_MONGODB_URL` server or another binary
//! sharing it is safe:
//!
//! 1. ends the sessions that still hold a lock on the database or run an
//!    operation in it ([`end_sessions_on`]: `lockInfo` and `$currentOp`, then
//!    `killSessions`), because the owner is gone and whatever still uses the
//!    database is abandoned;
//! 2. runs `dropDatabase` with a server-side bound of [`DROP_ATTEMPT_MAX_TIME`],
//!    and again, with step 1, when it ran out of that bound while queued for a
//!    lock, until [`DROP_TIMEOUT`].
//!
//! Step 1 matters for tests that end with a transaction open: a commit that a
//! `failCommand` failpoint refused leaves its transaction open, and a runtime that
//! shuts down before the driver's drop-time abort runs can do the same. Such a
//! transaction is idle, keeps an intent lock on its database and is expired by
//! mongod only after its 60 s lifetime and the reaper's next pass, 60-90 s later.
//! A plain `dropDatabase` queued for the exclusive lock behind it, holding a write
//! ticket for that whole time. On a mongod reused across rounds of a
//! five-module filter that includes `transaction_retry`, such drops were seen
//! together with the `createIndexes` of new tests and the transactions' own
//! commits and aborts holding every write ticket while the server made no
//! progress (#2436). Step 2 keeps a drop that still queues from holding a
//! ticket for more than a few seconds.
//!
//! When no shared mongo was started, finished entries are discarded without
//! connecting. Connection-level drop failures are retried up to [`DROP_ATTEMPTS`]
//! times, and giving up counts only drops that still fail after that. After
//! [`UNAVAILABLE_FAILURES_BEFORE_GIVING_UP`] consecutive "mongo is not
//! reachable" failures the sweeper stops dropping and says so once. A server
//! that refuses `lockInfo`, `$currentOp` or `killSessions` (for example an
//! authenticated server whose test user lacks the `serverStatus`, `inprog` or
//! `killAnySession` privileges) is dropped on as before, without step 1, with one
//! note on stderr.
//!
//! At process exit a `#[ctor::dtor]` waits up to [`EXIT_FLUSH_TIMEOUT`] for the
//! outstanding drops (only when `HFS_TEST_MONGODB_URL` is set, because the
//! harness's own container is removed at exit anyway) and prints one summary
//! line to stderr:
//! `[hfs-test-db-cleanup] registered=R unscoped=U dropped=D kept=K failed=F skipped=S ended=E outstanding=O peak_outstanding=P flush=<done|timeout|not-needed>`.
//! `ended` counts the sessions and operations step 1 ended for databases whose
//! drop then succeeded; each such drop is also named on stderr, with the test
//! thread that left it, as
//! `ended N session(s) still using <database> after test <name> ended`.
//!
//! `peak_outstanding` is the most test databases registered and not yet dropped
//! at one time (those of running tests plus drops in flight, counting names that
//! were never created), i.e. the mid-run bound #2311 asks for; with the default
//! test threads it should stay near the thread count, not near the number of
//! tests. `unscoped` counts names handed out on a thread that is not registered
//! (see the contract above).
//!
//! When the harness owns its container (no `HFS_TEST_MONGODB_URL`, as in CI),
//! that container is removed at exit while the last drops may still run, so
//! `failed=` or `outstanding=` can be non-zero on that line; this affects only
//! the stderr line, never the test result.
//!
//! # Knobs
//!
//! - `HFS_TEST_MONGODB_KEEP_FAILED=1` keeps the databases of tests that
//!   panicked, for debugging, up to [`KEEP_FAILED_MAX`] per process; each kept
//!   name is logged. Off by default. It installs a chained panic hook, so any
//!   panic on a test thread counts as a failure, including an intentional one
//!   caught by the test; the cap bounds that.
//! - `HFS_TEST_MONGODB_MAX_LEFTOVER_DATABASES=N` makes [`end_of_run_report`]
//!   assert that at most N test databases are left.
//!
//! # Measuring
//!
//! [`end_of_run_report`] is `#[ignore]`d. The runner removes its mongod when the
//! command ends, so run it in the same command, after the suite:
//!
//! ```text
//! hfs-test-mongo.sh <worktree> 'cargo test -p helios-persistence --features mongodb \
//!     --test mongodb_tests; cargo test -p helios-persistence --features mongodb \
//!     --test mongodb_tests -- --ignored --exact db_cleanup::end_of_run_report'
//! ```
//!
//! The report goes straight to stderr, so `--nocapture` is not needed. It prints
//! the number of `hfs_phase2_mongo_*` databases left, WiredTiger data handles,
//! drop-pending idents, checkpoint figures and average latencies. A dropped
//! database's WiredTiger tables are freed at the first checkpoint after the
//! server's snapshot history window (`minSnapshotHistoryWindowInSeconds`: 300 s
//! by default, 5 s on the harness's container and in the /test-hfs skill's
//! `docker run` line, #2326), so `wt_data_handles` and `drop_pending_idents`
//! trail the database count. A runner that starts its own mongod, such as the
//! one above, has the 300 s default unless it passes
//! `--setParameter minSnapshotHistoryWindowInSeconds=5` itself; add it before
//! comparing figures with a run on the harness's container.

use super::*;

use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Once, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// How often the sweeper looks for finished owners.
const SWEEP_INTERVAL: Duration = Duration::from_millis(250);
/// At most this many `dropDatabase` commands run at once.
const DROP_CONCURRENCY: usize = 8;
/// The bound on one database's whole drop: ending the sessions that still use it,
/// every [`DROP_ATTEMPT_MAX_TIME`]-bound attempt and the pauses between them.
const DROP_TIMEOUT: Duration = Duration::from_secs(120);
/// `dropDatabase` is idempotent, so a drop that fails with a connection-level
/// error ([`super::msg_is_mongo_unavailable`]: a reset or EOF under load, or a
/// pool cleared by another operation's failure) is sent again, at most this
/// many times in all, 1 s and then 2 s apart, all inside [`DROP_TIMEOUT`].
const DROP_ATTEMPTS: usize = 3;
/// The server-side `maxTimeMS` of one `dropDatabase`. A drop queues for the
/// database's exclusive lock behind every open transaction on it, holding a
/// write ticket while it waits; with this bound it gives the ticket back after
/// 5 s instead of keeping it, and the server stops it with its client (#2436).
const DROP_ATTEMPT_MAX_TIME: Duration = Duration::from_secs(5);
/// Pause between a drop attempt that ran out of time and the next one.
const DROP_QUEUED_PAUSE: Duration = Duration::from_millis(500);
/// Server error codes of a `dropDatabase` that ran out of its `maxTimeMS` while
/// it waited for a lock: `MaxTimeMSExpired`, `LockTimeout`, `ExceededTimeLimit`.
const QUEUED_DROP_CODES: [i32; 3] = [50, 24, 262];
/// Server selection and connect timeout of the cleanup client.
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the exit hook waits for outstanding drops.
const EXIT_FLUSH_TIMEOUT: Duration = Duration::from_secs(30);
const UNAVAILABLE_FAILURES_BEFORE_GIVING_UP: usize = 3;
const KEEP_FAILED_ENV: &str = "HFS_TEST_MONGODB_KEEP_FAILED";
/// At most this many databases of failed tests are kept per process.
const KEEP_FAILED_MAX: usize = 8;
/// `appName` of the cleanup client. `bulk_submit::FailPoint` scopes every
/// `failCommand` to its own `appName`, so a failpoint never matches this client.
const CLEANUP_APP_NAME: &str = "hfs-test-db-cleanup";

/// Shared state of one registering thread.
struct OwnerState {
    /// False once the thread has exited.
    alive: AtomicBool,
    /// Set by the opt-in panic hook when the thread panicked.
    failed: AtomicBool,
    thread: String,
}

/// Lives in a `thread_local!`, so it drops when its thread exits. The drop does
/// nothing but store an atomic, so a test thread never blocks or panics on it.
struct OwnerGuard(Arc<OwnerState>);

impl Drop for OwnerGuard {
    fn drop(&mut self) {
        self.0.alive.store(false, Ordering::Release);
    }
}

thread_local! {
    static OWNER: OwnerGuard = OwnerGuard(Arc::new(OwnerState {
        alive: AtomicBool::new(true),
        failed: AtomicBool::new(false),
        thread: std::thread::current().name().unwrap_or("<unnamed>").to_owned(),
    }));
}

struct Entry {
    database: String,
    owner: Arc<OwnerState>,
}

#[derive(Default)]
struct Counters {
    registered: AtomicUsize,
    /// Names handed out on a thread whose end is not the end of a test, so they
    /// are not registered and not dropped.
    unscoped: AtomicUsize,
    /// High-water mark of the registry's `outstanding` count.
    peak_outstanding: AtomicUsize,
    dropped: AtomicUsize,
    kept: AtomicUsize,
    failed: AtomicUsize,
    skipped: AtomicUsize,
    /// Sessions and operations ended because they still used the database of a
    /// finished test (#2436).
    ended: AtomicUsize,
}

struct Registry {
    /// Databases whose owner thread may still be running.
    pending: Mutex<Vec<Entry>>,
    /// Registered entries that are not yet dropped, kept or skipped.
    outstanding: Mutex<usize>,
    idle: Condvar,
    counters: Counters,
    given_up: AtomicBool,
    unavailable_streak: AtomicUsize,
}

/// Locks `mutex`, ignoring poisoning: nothing guarded here can be left
/// half-updated, and a panic must never turn into a second one.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Registry {
    fn new() -> Self {
        Self {
            pending: Mutex::new(Vec::new()),
            outstanding: Mutex::new(0),
            idle: Condvar::new(),
            counters: Counters::default(),
            given_up: AtomicBool::new(false),
            unavailable_streak: AtomicUsize::new(0),
        }
    }

    fn add(&self, entry: Entry) {
        self.counters.registered.fetch_add(1, Ordering::Relaxed);
        // `given_up` is checked under the `pending` lock, as `give_up` drains
        // under it: an entry is either refused here or drained there.
        let mut pending = lock(&self.pending);
        if self.given_up.load(Ordering::Acquire) {
            self.counters.skipped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        {
            let mut outstanding = lock(&self.outstanding);
            *outstanding += 1;
            self.counters
                .peak_outstanding
                .fetch_max(*outstanding, Ordering::Relaxed);
        }
        pending.push(entry);
    }

    /// Removes and returns the entries whose owner thread has ended.
    fn take_finished(&self) -> Vec<Entry> {
        let mut pending = lock(&self.pending);
        let (finished, running): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut *pending)
            .into_iter()
            .partition(|entry| !entry.owner.alive.load(Ordering::Acquire));
        *pending = running;
        finished
    }

    /// Marks `n` entries as dealt with (dropped, kept, failed or skipped).
    fn finish(&self, n: usize) {
        let mut outstanding = lock(&self.outstanding);
        *outstanding = outstanding.saturating_sub(n);
        if *outstanding == 0 {
            self.idle.notify_all();
        }
    }

    /// Counts `n` entries as skipped and marks them dealt with.
    fn skip(&self, n: usize) {
        self.counters.skipped.fetch_add(n, Ordering::Relaxed);
        self.finish(n);
    }

    /// True once `outstanding` reached 0 within `timeout`.
    fn wait_idle(&self, timeout: Duration) -> bool {
        let guard = lock(&self.outstanding);
        let (guard, _) = self
            .idle
            .wait_timeout_while(guard, timeout, |outstanding| *outstanding > 0)
            .unwrap_or_else(PoisonError::into_inner);
        *guard == 0
    }

    /// Stops dropping databases for the rest of the process. Says so once.
    fn give_up(&self, why: std::fmt::Arguments<'_>) {
        if !self.given_up.swap(true, Ordering::AcqRel) {
            log(format_args!("not dropping test databases any more: {why}"));
        }
        let drained = std::mem::take(&mut *lock(&self.pending));
        self.skip(drained.len());
    }

    fn outstanding(&self) -> usize {
        *lock(&self.outstanding)
    }
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();
static SWEEPER: Once = Once::new();
static UNSCOPED_WARNED: AtomicBool = AtomicBool::new(false);

/// One `[hfs-test-db-cleanup]` line on stderr.
///
/// Writes through `std::io::stderr()` rather than `eprintln!`, because the
/// sweeper thread inherits the output capture of the test that started it
/// (inferred), which would swallow the line. Write errors are ignored, so
/// logging never panics.
fn log(args: std::fmt::Arguments<'_>) {
    let _ = writeln!(std::io::stderr().lock(), "[hfs-test-db-cleanup] {args}");
}

/// Registers `database` against the calling thread. False when the calling
/// thread is not one whose end marks the end of a test (see the module doc).
fn register_into(registry: &Registry, database: &str) -> bool {
    let thread = std::thread::current();
    let scoped = thread
        .name()
        .is_some_and(|name| name != "main" && !name.starts_with("tokio-runtime-worker"));
    let owner = if scoped {
        OWNER.try_with(|guard| Arc::clone(&guard.0)).ok()
    } else {
        None
    };
    let Some(owner) = owner else {
        return false;
    };
    registry.add(Entry {
        database: database.to_owned(),
        owner,
    });
    true
}

/// Called by `build_test_database_name` for every name it hands out.
pub(super) fn register(database: &str) {
    let registry = REGISTRY.get_or_init(Registry::new);
    SWEEPER.call_once(|| spawn_sweeper(registry));
    if keep_failed_enabled() {
        install_failure_hook();
    }
    if !register_into(registry, database) {
        registry.counters.unscoped.fetch_add(1, Ordering::Relaxed);
        if !UNSCOPED_WARNED.swap(true, Ordering::Relaxed) {
            log(format_args!(
                "{database} was named on thread {:?}, which does not end with its test: it and \
                 any later database named there are not dropped automatically (name test \
                 databases from the test body)",
                std::thread::current().name()
            ));
        }
    }
}

/// Whether `HFS_TEST_MONGODB_KEEP_FAILED` asks to keep failed tests' databases.
fn keep_failed_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var(KEEP_FAILED_ENV).is_ok_and(|v| !v.is_empty() && v != "0"))
}

/// Whether a finished owner's databases are kept instead of dropped.
fn should_keep(owner_failed: bool, keep_failed: bool, kept_so_far: usize) -> bool {
    keep_failed && owner_failed && kept_so_far < KEEP_FAILED_MAX
}

/// Chains a panic hook that flags the panicking thread's owner as failed. Only
/// installed when `HFS_TEST_MONGODB_KEEP_FAILED` is set.
fn install_failure_hook() {
    static FAILURE_HOOK: Once = Once::new();
    FAILURE_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = OWNER.try_with(|guard| guard.0.failed.store(true, Ordering::Release));
            previous(info);
        }));
    });
}

fn spawn_sweeper(registry: &'static Registry) {
    let spawned = std::thread::Builder::new()
        .name("hfs-test-db-sweeper".to_owned())
        .spawn(move || {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime.block_on(sweep_forever(registry)),
                Err(error) => registry.give_up(format_args!("no runtime for the sweeper: {error}")),
            }
        });
    if let Err(error) = spawned {
        registry.give_up(format_args!("cannot start the sweeper thread: {error}"));
    }
}

/// The one client the sweeper drops databases with: its own `appName`, a small
/// pool and short timeouts, so it neither disturbs the connection-count and
/// `$currentOp` assertions of other tests nor hangs on a dead server.
async fn cleanup_client(url: &str) -> mongodb::error::Result<Client> {
    let mut options = mongodb::options::ClientOptions::parse(url).await?;
    options.app_name = Some(CLEANUP_APP_NAME.to_owned());
    options.max_pool_size = Some(DROP_CONCURRENCY as u32);
    options.server_selection_timeout = Some(CLEANUP_TIMEOUT);
    options.connect_timeout = Some(CLEANUP_TIMEOUT);
    Client::with_options(options)
}

async fn sweep_forever(registry: &'static Registry) {
    let permits = Arc::new(tokio::sync::Semaphore::new(DROP_CONCURRENCY));
    let mut client: Option<Client> = None;
    loop {
        tokio::time::sleep(SWEEP_INTERVAL).await;
        let finished = registry.take_finished();
        if finished.is_empty() {
            continue;
        }
        // No shared mongo started (or none available): nothing to drop, and no
        // reason to wait for a server selection timeout.
        let url = if registry.given_up.load(Ordering::Acquire) {
            None
        } else {
            super::shared_mongo::started_connection_string()
        };
        let Some(url) = url else {
            registry.skip(finished.len());
            continue;
        };
        let handle = match &client {
            Some(existing) => existing.clone(),
            None => match cleanup_client(&url).await {
                Ok(created) => {
                    client = Some(created.clone());
                    created
                }
                Err(error) => {
                    registry.give_up(format_args!("cannot build the cleanup client: {error}"));
                    registry.skip(finished.len());
                    continue;
                }
            },
        };
        for entry in finished {
            // Decided here, not in the tasks, so the keep count cannot race.
            if should_keep(
                entry.owner.failed.load(Ordering::Acquire),
                keep_failed_enabled(),
                registry.counters.kept.load(Ordering::Relaxed),
            ) {
                registry.counters.kept.fetch_add(1, Ordering::Relaxed);
                log(format_args!(
                    "keeping {} of failed test thread {} ({KEEP_FAILED_ENV})",
                    entry.database, entry.owner.thread
                ));
                registry.finish(1);
                continue;
            }
            tokio::spawn(drop_database(
                registry,
                handle.clone(),
                Arc::clone(&permits),
                entry,
            ));
        }
    }
}

/// Whether a drop that failed with `error_debug` on attempt number `attempt`
/// (1-based) is sent again.
fn retry_drop(error_debug: &str, attempt: usize) -> bool {
    attempt < DROP_ATTEMPTS && super::msg_is_mongo_unavailable(error_debug)
}

/// Whether `code` is one a `dropDatabase` answers when it ran out of its
/// `maxTimeMS` waiting for a lock ([`QUEUED_DROP_CODES`]).
fn is_queued_drop_code(code: i32) -> bool {
    QUEUED_DROP_CODES.contains(&code)
}

/// Whether a drop failed only because it was still queued for a lock when its
/// [`DROP_ATTEMPT_MAX_TIME`] ended, which is worth sending again.
fn is_queued_drop(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Command(command) if is_queued_drop_code(command.code)
    )
}

/// The session id in a lock request's `debugInfo`. mongod stamps the locks of an
/// open transaction with the session that owns them, as
/// `lsid: { id: UUID("<uuid>"), uid: BinData(0, <hash>) }`; ordinary requests
/// carry an empty string.
fn lsid_from_debug_info(debug_info: &str) -> Option<uuid::Uuid> {
    const MARKER: &str = "UUID(\"";
    let start = debug_info.find(MARKER)? + MARKER.len();
    debug_info
        .get(start..start + 36)
        .and_then(|text| uuid::Uuid::parse_str(text).ok())
}

/// Whether a `lockInfo` `resourceId`, such as
/// `{<id>: Database, <hash>, <db>}` or `{<id>: Collection, <hash>, <db>.<coll>}`,
/// names exactly `database` or one of its collections. The comparison is on the
/// whole name, so `hfs_phase2_mongo_a_1` does not match `hfs_phase2_mongo_a_12`.
fn names_database(resource_id: &str, database: &str) -> bool {
    let name = resource_id
        .trim_end_matches('}')
        .rsplit(", ")
        .next()
        .unwrap_or_default()
        .trim();
    name == database
        || name
            .strip_prefix(database)
            .is_some_and(|rest| rest.starts_with('.'))
}

/// The session id of a `$currentOp` entry.
fn op_lsid(op: &Document) -> Option<uuid::Uuid> {
    match op.get_document("lsid").ok()?.get("id")? {
        Bson::Binary(binary) => uuid::Uuid::from_slice(&binary.bytes).ok(),
        _ => None,
    }
}

static SESSION_LOOKUP_WARNED: AtomicBool = AtomicBool::new(false);

/// Ends every session that still holds a lock on `database` (or one of its
/// collections) or runs an operation in it, and returns how many it ended
/// (#2436).
///
/// A database is dropped only once the thread that named it has ended, so
/// whatever still uses it is abandoned. An open transaction is the case that
/// matters: it is idle, holds an intent lock on the database, and is expired by
/// mongod only after `transactionLifetimeLimitSeconds` and the reaper's next
/// pass (60-90 s), because nothing aborts it: a commit that mongod refused
/// leaves its transaction open, and the driver's drop-time abort runs on the
/// test's runtime, which may already be gone. The sessions come from
/// `lockInfo` (an open transaction's locks name their session in `debugInfo`)
/// and from `$currentOp` (operations whose namespace or database is
/// `database`), by the exact database name only, so another test's sessions are
/// never touched.
async fn end_sessions_on(client: &Client, database: &str) -> mongodb::error::Result<usize> {
    use futures::TryStreamExt;

    let admin = client.database("admin");
    let mut sessions: Vec<uuid::Uuid> = Vec::new();

    let locks = admin.run_command(doc! { "lockInfo": 1_i32 }).await?;
    for entry in locks.get_array("lockInfo").into_iter().flatten() {
        let Some(entry) = entry.as_document() else {
            continue;
        };
        if !names_database(entry.get_str("resourceId").unwrap_or_default(), database) {
            continue;
        }
        for side in ["granted", "pending"] {
            for request in entry.get_array(side).into_iter().flatten() {
                let debug_info = request
                    .as_document()
                    .and_then(|request| request.get_str("debugInfo").ok())
                    .unwrap_or_default();
                if let Some(id) = lsid_from_debug_info(debug_info)
                    && !sessions.contains(&id)
                {
                    sessions.push(id);
                }
            }
        }
    }

    let running: Vec<Document> = admin
        .aggregate(vec![
            doc! { "$currentOp": { "allUsers": true, "idleSessions": false, "idleConnections": false } },
            doc! { "$match": {
                "appName": { "$ne": CLEANUP_APP_NAME },
                "$expr": { "$or": [
                    { "$eq": [ { "$ifNull": [ "$command.$db", "" ] }, database ] },
                    { "$eq": [ { "$indexOfCP": [ { "$ifNull": [ "$ns", "" ] }, format!("{database}.") ] }, 0 ] },
                ] },
            } },
            doc! { "$project": { "opid": 1, "lsid": 1, "op": 1 } },
        ])
        .await?
        .try_collect()
        .await?;
    let mut sessionless_ops: Vec<(Bson, String)> = Vec::new();
    for op in &running {
        match op_lsid(op) {
            Some(id) => {
                if !sessions.contains(&id) {
                    sessions.push(id);
                }
            }
            None => {
                if let Some(opid) = op.get("opid") {
                    sessionless_ops.push((
                        opid.clone(),
                        op.get_str("op").unwrap_or("operation").to_owned(),
                    ));
                }
            }
        }
    }

    if !sessions.is_empty() {
        for id in &sessions {
            log(format_args!("ending session {id} still using {database}"));
        }
        let ids: Vec<Bson> = sessions
            .iter()
            .map(|id| {
                Bson::Document(doc! {
                    "id": Bson::Binary(mongodb::bson::Binary {
                        subtype: mongodb::bson::spec::BinarySubtype::Uuid,
                        bytes: id.as_bytes().to_vec(),
                    }),
                })
            })
            .collect();
        admin
            .run_command(doc! { "killSessions": ids, "maxTimeMS": 10_000_i64 })
            .await?;
    }
    for (opid, op) in &sessionless_ops {
        log(format_args!(
            "killing {op} {opid} still running in {database}"
        ));
        admin
            .run_command(doc! { "killOp": 1_i32, "op": opid.clone() })
            .await?;
    }
    Ok(sessions.len() + sessionless_ops.len())
}

/// One `dropDatabase`, bounded on the server by [`DROP_ATTEMPT_MAX_TIME`], so a
/// drop queued for the database lock gives its write ticket back instead of
/// holding it (#2436).
async fn drop_attempt(client: &Client, database: &str) -> mongodb::error::Result<()> {
    client
        .database(database)
        .run_command(doc! {
            "dropDatabase": 1_i32,
            "maxTimeMS": DROP_ATTEMPT_MAX_TIME.as_millis() as i64,
        })
        .await
        .map(|_| ())
}

/// Drops a finished test's database and returns how many sessions had to be
/// ended first (see [`end_sessions_on`]).
///
/// Every attempt ends the sessions that still use the database, then drops it
/// with a server-side time bound. A drop that ran out of that bound while queued
/// for a lock (the next session may have appeared, or its end may not have taken
/// effect yet) is sent again after [`DROP_QUEUED_PAUSE`], and a connection-level
/// error ([`retry_drop`]) up to [`DROP_ATTEMPTS`] times in all; the caller's
/// [`DROP_TIMEOUT`] bounds the loop. Failing to look for sessions does not stop
/// the drop: the server may refuse `lockInfo`, `$currentOp` or `killSessions`
/// (privileges), and the bounded drop then simply waits its turn as before.
async fn drop_finished_database(client: &Client, database: &str) -> mongodb::error::Result<usize> {
    let mut ended = 0;
    let mut attempt = 1;
    loop {
        match end_sessions_on(client, database).await {
            Ok(count) => ended += count,
            Err(error) => {
                if !SESSION_LOOKUP_WARNED.swap(true, Ordering::Relaxed) {
                    log(format_args!(
                        "cannot end the sessions still using {database} ({error}); drops go \
                         ahead without ending sessions (this is said once)"
                    ));
                }
            }
        }
        match drop_attempt(client, database).await {
            Ok(()) => return Ok(ended),
            Err(error) if is_queued_drop(&error) => {
                tokio::time::sleep(DROP_QUEUED_PAUSE).await;
            }
            Err(error) if retry_drop(&format!("{error:?}"), attempt) => {
                tokio::time::sleep(Duration::from_secs(attempt as u64)).await;
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Drops one database and records the outcome. Never panics. The sessions still
/// using it are ended and connection-level errors are retried by
/// [`drop_finished_database`], so only a drop that still fails after all
/// attempts counts as a failure or toward giving up.
async fn drop_database(
    registry: &'static Registry,
    client: Client,
    permits: Arc<tokio::sync::Semaphore>,
    entry: Entry,
) {
    if let Ok(_permit) = permits.acquire_owned().await {
        match tokio::time::timeout(
            DROP_TIMEOUT,
            drop_finished_database(&client, &entry.database),
        )
        .await
        {
            Ok(Ok(ended)) => {
                registry.counters.dropped.fetch_add(1, Ordering::Relaxed);
                registry.unavailable_streak.store(0, Ordering::Relaxed);
                if ended > 0 {
                    registry.counters.ended.fetch_add(ended, Ordering::Relaxed);
                    log(format_args!(
                        "ended {ended} session(s) still using {} after test {} ended (#2436)",
                        entry.database, entry.owner.thread
                    ));
                }
            }
            Ok(Err(error)) => {
                registry.counters.failed.fetch_add(1, Ordering::Relaxed);
                log(format_args!(
                    "dropping {} (test {}) failed: {error}",
                    entry.database, entry.owner.thread
                ));
                if super::msg_is_mongo_unavailable(&format!("{error:?}"))
                    && registry.unavailable_streak.fetch_add(1, Ordering::Relaxed) + 1
                        >= UNAVAILABLE_FAILURES_BEFORE_GIVING_UP
                {
                    registry.give_up(format_args!("MongoDB is unreachable"));
                }
            }
            Err(_elapsed) => {
                registry.counters.failed.fetch_add(1, Ordering::Relaxed);
                log(format_args!(
                    "dropping {} (test {}) timed out after {} s",
                    entry.database,
                    entry.owner.thread,
                    DROP_TIMEOUT.as_secs()
                ));
            }
        }
    }
    registry.finish(1);
}

/// Waits for the outstanding drops and prints the summary line.
#[ctor::dtor]
fn flush_test_databases_at_exit() {
    let Some(registry) = REGISTRY.get() else {
        return;
    };
    let flush = if super::test_mongo_url().is_some() {
        if registry.wait_idle(EXIT_FLUSH_TIMEOUT) {
            "done"
        } else {
            "timeout"
        }
    } else {
        "not-needed"
    };
    let counters = &registry.counters;
    log(format_args!(
        "registered={} unscoped={} dropped={} kept={} failed={} skipped={} ended={} \
         outstanding={} peak_outstanding={} flush={flush}",
        counters.registered.load(Ordering::Relaxed),
        counters.unscoped.load(Ordering::Relaxed),
        counters.dropped.load(Ordering::Relaxed),
        counters.kept.load(Ordering::Relaxed),
        counters.failed.load(Ordering::Relaxed),
        counters.skipped.load(Ordering::Relaxed),
        counters.ended.load(Ordering::Relaxed),
        registry.outstanding(),
        counters.peak_outstanding.load(Ordering::Relaxed),
    ));
}

#[test]
fn registry_releases_an_entry_only_after_its_thread_ends() {
    let registry = Registry::new();
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("db_cleanup_unit_probe".to_owned())
            .spawn_scoped(scope, || {
                assert!(register_into(&registry, "db_a"));
                assert!(
                    registry.take_finished().is_empty(),
                    "an entry must stay while its thread runs"
                );
            })
            .expect("spawn probe thread")
            .join()
            .expect("probe thread panicked");
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    let finished = loop {
        let finished = registry.take_finished();
        if !finished.is_empty() || Instant::now() >= deadline {
            break finished;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let names: Vec<&str> = finished
        .iter()
        .map(|entry| entry.database.as_str())
        .collect();
    assert_eq!(names, ["db_a"]);
    assert_eq!(registry.outstanding(), 1);
    assert_eq!(
        registry.counters.peak_outstanding.load(Ordering::Relaxed),
        1
    );
    registry.finish(1);
    assert!(registry.wait_idle(Duration::from_secs(1)));
    assert_eq!(
        registry.counters.peak_outstanding.load(Ordering::Relaxed),
        1,
        "the peak is a high-water mark: finishing an entry does not lower it"
    );
}

#[test]
fn drop_retry_is_bounded_to_connection_errors() {
    let reset = "Kind: I/O error: unexpected end of file";
    assert!(retry_drop(reset, 1));
    assert!(retry_drop(reset, 2));
    assert!(!retry_drop(reset, DROP_ATTEMPTS));
    assert!(!retry_drop("Command failed (Unauthorized)", 1));
}

#[test]
fn lsid_is_read_from_a_lock_debug_info() {
    // The shape `lockInfo` reports for the locks of an open transaction
    // (MongoDB 7.0, seen with a transaction opened from mongosh).
    let open_transaction = r#"lsid: { id: UUID("b7766088-61ce-408f-9677-91803d44c7ae"), uid: BinData(0, E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855) }"#;
    assert_eq!(
        lsid_from_debug_info(open_transaction),
        Some(uuid::Uuid::parse_str("b7766088-61ce-408f-9677-91803d44c7ae").expect("uuid"))
    );
    assert_eq!(lsid_from_debug_info(""), None, "an ordinary request");
    assert_eq!(lsid_from_debug_info("no lsid here"), None);
    assert_eq!(
        lsid_from_debug_info(r#"id: UUID("b7766088")"#),
        None,
        "too short"
    );
    assert_eq!(
        lsid_from_debug_info(r#"id: UUID("zzzzzzzz-61ce-408f-9677-91803d44c7ae")"#),
        None,
        "not a UUID"
    );
}

#[test]
fn lock_resources_are_matched_by_exact_database_name() {
    let database = "hfs_phase2_mongo_a_1";
    assert!(names_database(
        "{5773689864591596621: Database, 1162003846164208717, hfs_phase2_mongo_a_1}",
        database
    ));
    assert!(names_database(
        "{7428124524227842508: Collection, 510595496586760652, hfs_phase2_mongo_a_1.resources}",
        database
    ));
    assert!(
        !names_database(
            "{5773689864591596621: Database, 1162003846164208717, hfs_phase2_mongo_a_12}",
            database
        ),
        "a longer database name that starts with this one is another test's"
    );
    assert!(!names_database(
        "{7428124524227842508: Collection, 510595496586760652, hfs_phase2_mongo_a_1x.resources}",
        database
    ));
    assert!(!names_database("{1: Global, 1}", database));
    assert!(!names_database("", database));
}

#[test]
fn queued_drop_codes_are_lock_wait_timeouts() {
    for code in [50, 24, 262] {
        assert!(is_queued_drop_code(code), "{code}");
    }
    // Unauthorized, Interrupted, ShutdownInProgress, NamespaceNotFound.
    for code in [13, 11601, 91, 26] {
        assert!(!is_queued_drop_code(code), "{code}");
    }
}

#[test]
fn registration_from_an_unscoped_thread_is_ignored() {
    let registry = Registry::new();
    std::thread::scope(|scope| {
        let registered = scope
            .spawn(|| register_into(&registry, "db_b"))
            .join()
            .expect("unnamed thread panicked");
        assert!(!registered, "an unnamed thread must not register");
    });
    assert_eq!(registry.outstanding(), 0);
    assert_eq!(registry.counters.registered.load(Ordering::Relaxed), 0);
    assert!(registry.take_finished().is_empty());
}

#[test]
fn keep_failed_is_opt_in_and_bounded() {
    assert!(should_keep(true, true, 0));
    assert!(should_keep(true, true, KEEP_FAILED_MAX - 1));
    assert!(!should_keep(true, true, KEEP_FAILED_MAX));
    assert!(!should_keep(false, true, 0), "a passed test is dropped");
    assert!(!should_keep(true, false, 0), "off unless asked for");
}

async fn database_exists(client: &Client, name: &str) -> bool {
    client
        .list_database_names()
        .filter(doc! { "name": name })
        .await
        .expect("listDatabases")
        .iter()
        .any(|listed| listed == name)
}

async fn wait_until_dropped(client: &Client, name: &str, deadline: Duration) -> bool {
    let give_up_at = Instant::now() + deadline;
    loop {
        if !database_exists(client, name).await {
            return true;
        }
        if Instant::now() >= give_up_at {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn probe_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("probe runtime")
}

/// Runs on the probe thread of
/// [`a_finished_test_threads_databases_are_dropped`]: builds a backend (and so a
/// schema database), names a database it drops itself and one it never creates.
async fn finished_thread_probe() -> Option<(String, String, String)> {
    let backend = create_backend("cleanup_probe").await?;
    let tenant = create_tenant("cleanup-probe");
    backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .expect("create a Patient in the probe database");
    let schema_db = backend.config().database_name.clone();

    let raw = raw_test_client(&backend.config().connection_string)
        .await
        .expect("raw client");
    let self_dropped = build_test_database_name("cleanup_self_dropped");
    raw.database(&self_dropped)
        .collection::<Document>("probe")
        .insert_one(doc! { "probe": 1_i32 })
        .await
        .expect("insert into the self-dropped database");
    raw.database(&self_dropped)
        .drop()
        .await
        .expect("the test drops its own database");
    let never_created = build_test_database_name("cleanup_never_created");

    assert!(
        database_exists(&raw, &schema_db).await,
        "the probe's database must exist while its thread runs"
    );
    Some((schema_db, self_dropped, never_created))
}

/// How long the probe tests wait for a drop. One drop may legitimately take up
/// to [`DROP_TIMEOUT`] behind the [`DROP_CONCURRENCY`] permits, so the probes
/// only fail when cleanup is really broken.
const PROBE_DROP_DEADLINE: Duration = Duration::from_secs(DROP_TIMEOUT.as_secs() + 60);

/// Without the `register` call in `build_test_database_name` the probe's schema
/// database is never dropped and the [`PROBE_DROP_DEADLINE`] wait below fails.
#[tokio::test]
async fn a_finished_test_threads_databases_are_dropped() {
    let Some(url) = shared_mongo::connection_string().await else {
        eprintln!(
            "Skipping a_finished_test_threads_databases_are_dropped (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let client = raw_test_client(&url).await.expect("raw client");

    let probe = std::thread::Builder::new()
        .name("db_cleanup_probe".to_owned())
        .spawn(|| probe_runtime().block_on(finished_thread_probe()))
        .expect("spawn probe thread");
    let joined = tokio::task::spawn_blocking(move || probe.join())
        .await
        .expect("join task");
    let probed = match joined {
        Ok(probed) => probed,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    let Some((schema_db, self_dropped, never_created)) = probed else {
        eprintln!(
            "Skipping a_finished_test_threads_databases_are_dropped (the probe found no mongo)"
        );
        return;
    };

    assert!(
        wait_until_dropped(&client, &schema_db, PROBE_DROP_DEADLINE).await,
        "database {schema_db} of a finished test thread still exists after {} s: \
         test databases are not dropped (#2311)",
        PROBE_DROP_DEADLINE.as_secs()
    );
    assert!(
        !database_exists(&client, &self_dropped).await,
        "{self_dropped} was dropped by its own test and must not come back"
    );
    assert!(
        !database_exists(&client, &never_created).await,
        "{never_created} was never created and must not appear"
    );
}

#[tokio::test]
async fn a_failed_test_threads_database_is_dropped() {
    if keep_failed_enabled() {
        eprintln!(
            "Skipping a_failed_test_threads_database_is_dropped ({KEEP_FAILED_ENV} keeps failed databases)"
        );
        return;
    }
    let Some(url) = shared_mongo::connection_string().await else {
        eprintln!(
            "Skipping a_failed_test_threads_database_is_dropped (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let client = raw_test_client(&url).await.expect("raw client");

    let (sender, receiver) = std::sync::mpsc::channel::<String>();
    let probe = std::thread::Builder::new()
        .name("db_cleanup_failed_probe".to_owned())
        .spawn(move || {
            probe_runtime().block_on(async move {
                let Some(backend) = create_backend("cleanup_failed_probe").await else {
                    return;
                };
                sender
                    .send(backend.config().database_name.clone())
                    .expect("receiver is alive");
                // Unwinds the thread like a failed assertion, without running the
                // panic hook, so no "panicked at" block lands in a passing log.
                std::panic::resume_unwind(Box::new("intentional #2311 probe panic"));
            })
        })
        .expect("spawn probe thread");
    let joined = tokio::task::spawn_blocking(move || probe.join())
        .await
        .expect("join task");

    // A panic inside `create_backend` also unwinds the probe thread, so the
    // thread's outcome and whether it reported a database are checked together.
    let database = match (receiver.try_recv(), joined) {
        (Ok(database), Err(_)) => database,
        (Ok(_), Ok(())) => panic!("the probe thread was meant to panic"),
        (Err(_), Ok(())) => {
            eprintln!(
                "Skipping a_failed_test_threads_database_is_dropped (the probe found no mongo)"
            );
            return;
        }
        (Err(_), Err(panic)) => std::panic::resume_unwind(panic),
    };
    assert!(
        wait_until_dropped(&client, &database, PROBE_DROP_DEADLINE).await,
        "database {database} of a failed test thread still exists after {} s (#2311)",
        PROBE_DROP_DEADLINE.as_secs()
    );
}

#[tokio::test]
async fn a_database_outlives_its_backend_until_its_test_ends() {
    let Some(backend) = create_backend("cleanup_outlives_backend").await else {
        eprintln!(
            "Skipping a_database_outlives_its_backend_until_its_test_ends (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let tenant = create_tenant("cleanup-outlives-backend");
    backend
        .create_or_update(
            &tenant,
            "Patient",
            "p1",
            json!({"resourceType": "Patient", "id": "p1"}),
            FhirVersion::default(),
        )
        .await
        .expect("create Patient p1");

    let config = backend.config().clone();
    drop(backend);
    // Several sweep intervals: a drop tied to the backend would have run by now.
    tokio::time::sleep(Duration::from_secs(2)).await;

    let client = raw_test_client(&config.connection_string)
        .await
        .expect("raw client");
    assert!(
        database_exists(&client, &config.database_name).await,
        "{} was dropped while its test was still running",
        config.database_name
    );

    let reopened = build_backend(config)
        .await
        .expect("reopen a backend on the same database");
    assert!(
        reopened
            .read(&tenant, "Patient", "p1")
            .await
            .expect("read p1")
            .is_some(),
        "the reopened backend must see the data written before the first one was dropped"
    );
}

/// The `lsid.id` of `session`, as `$currentOp` reports it.
fn session_lsid(session: &mongodb::ClientSession) -> Bson {
    session
        .id()
        .get("id")
        .cloned()
        .expect("a session id carries an `id`")
}

/// Whether `$currentOp` lists a session or an operation of the session with
/// this `lsid.id`: an open (idle) transaction shows as an `idleSession`.
async fn session_is_listed(client: &Client, lsid: &Bson) -> bool {
    use futures::TryStreamExt;

    let listed: Vec<Document> = client
        .database("admin")
        .aggregate(vec![
            doc! { "$currentOp": { "allUsers": true, "idleSessions": true, "idleConnections": false } },
            doc! { "$match": { "lsid.id": lsid.clone() } },
        ])
        .await
        .expect("$currentOp aggregate")
        .try_collect()
        .await
        .expect("$currentOp cursor");
    !listed.is_empty()
}

/// Whether `$currentOp` lists a `dropDatabase` of `database`.
async fn drop_is_running(client: &Client, database: &str) -> bool {
    use futures::TryStreamExt;

    let listed: Vec<Document> = client
        .database("admin")
        .aggregate(vec![
            doc! { "$currentOp": { "allUsers": true, "idleConnections": false } },
            doc! { "$match": { "command.dropDatabase": { "$exists": true }, "command.$db": database } },
        ])
        .await
        .expect("$currentOp aggregate")
        .try_collect()
        .await
        .expect("$currentOp cursor");
    !listed.is_empty()
}

/// Opens a transaction on `database` that stays open and idle, the way a test
/// leaves one behind when mongod refuses its commit (the `failCommand` tests)
/// or, possibly, when its runtime ends before the driver's drop-time abort runs
/// (#2436). The database is created first, outside the session. `None` (after a
/// note on stderr) when the server cannot run transactions, which only an
/// externally supplied `HFS_TEST_MONGODB_URL` standalone may.
async fn open_transaction_on(client: &Client, database: &str) -> Option<mongodb::ClientSession> {
    let collection = client.database(database).collection::<Document>("probe");
    collection
        .insert_one(doc! { "probe": 0_i32 })
        .await
        .expect("create the probe database");
    let mut session = client.start_session().await.expect("start a session");
    session
        .start_transaction()
        .await
        .expect("start a transaction");
    match collection
        .insert_one(doc! { "probe": 1_i32 })
        .session(&mut session)
        .await
    {
        Ok(_) => Some(session),
        // IllegalOperation: "Transaction numbers are only allowed on a replica set member or mongos".
        Err(error)
            if !transactions_required()
                && matches!(
                    error.kind.as_ref(),
                    mongodb::error::ErrorKind::Command(command) if command.code == 20
                ) =>
        {
            eprintln!(
                "this MongoDB does not run transactions, so the open-transaction probe is skipped"
            );
            None
        }
        Err(error) => panic!("insert inside the probe transaction failed: {error}"),
    }
}

/// How long the #2436 probes wait for a drop: a hang guard, not a measurement.
/// mongod's reaper cannot end an open transaction before its 60 s
/// `transactionLifetimeLimitSeconds`, so a drop that waits for it cannot finish
/// inside this. The 15 s margin below 60 s leaves room for load on a shared
/// mongod: write-ticket stalls of 38 s and more were seen in the #2436 loops.
const OPEN_TRANSACTION_DROP_BOUND: Duration = Duration::from_secs(45);

/// Why this MongoDB refuses `lockInfo`, or `None` when it answers. The sweeper
/// finds the sessions to end with it, so the probes that depend on that skip
/// when it is refused (for example by an authenticated external server whose
/// test user lacks the `serverStatus` privilege). `lockInfo` is not a test
/// command: it needs no `enableTestCommands`.
async fn lock_info_refused(client: &Client) -> Option<String> {
    client
        .database("admin")
        .run_command(doc! { "lockInfo": 1_i32 })
        .await
        .err()
        .map(|error| error.to_string())
}

/// A test's open transaction on its database must not hold up the drop of that
/// database (#2436). The transaction holds an intent lock on the database, so a
/// plain `dropDatabase` queues for the exclusive lock, holding a write ticket,
/// until mongod expires the transaction 60-90 s later; on a reused mongod
/// those waits piled up until every write ticket was out.
#[tokio::test]
async fn an_open_transaction_does_not_hold_up_its_databases_drop() {
    let Some(url) = shared_mongo::connection_string().await else {
        eprintln!(
            "Skipping an_open_transaction_does_not_hold_up_its_databases_drop (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let raw = raw_test_client(&url).await.expect("raw client");
    if let Some(refused) = lock_info_refused(&raw).await {
        eprintln!(
            "Skipping an_open_transaction_does_not_hold_up_its_databases_drop: this MongoDB \
             refuses lockInfo ({refused}), so the sweeper cannot find the sessions to end"
        );
        return;
    }
    let database = build_test_database_name("db_cleanup_open_txn");
    let Some(mut session) = open_transaction_on(&raw, &database).await else {
        return;
    };
    let lsid = session_lsid(&session);
    let cleanup = cleanup_client(&url).await.expect("cleanup client");

    let dropped = tokio::time::timeout(
        OPEN_TRANSACTION_DROP_BOUND,
        drop_finished_database(&cleanup, &database),
    )
    .await;
    let ended = match dropped {
        Ok(Ok(ended)) => ended,
        other => panic!(
            "the drop of {database} did not finish within {} s while a transaction was open on \
             it (it would wait for mongod's 60 s transaction lifetime limit, #2436): {other:?}",
            OPEN_TRANSACTION_DROP_BOUND.as_secs()
        ),
    };
    assert!(
        ended >= 1,
        "the drop of {database} ended {ended} sessions: it must end the session that holds the \
         open transaction on it instead of waiting for mongod to expire it (#2436)"
    );
    assert!(
        !database_exists(&raw, &database).await,
        "{database} still exists after its drop"
    );
    assert!(
        !session_is_listed(&raw, &lsid).await,
        "the transaction on {database} is still open after the database was dropped"
    );
    let _ = session.abort_transaction().await;
}

/// The same through the real path: the owner thread ends with its transaction
/// still open and never aborted (`mem::forget` stands for an abort that never
/// runs: a refused commit, or a drop-time abort lost at runtime shutdown), and
/// the sweeper drops the database anyway (#2436).
#[tokio::test]
async fn a_finished_test_threads_database_is_dropped_past_its_abandoned_transaction() {
    let Some(url) = shared_mongo::connection_string().await else {
        eprintln!(
            "Skipping a_finished_test_threads_database_is_dropped_past_its_abandoned_transaction (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let raw = raw_test_client(&url).await.expect("raw client");
    if let Some(refused) = lock_info_refused(&raw).await {
        eprintln!(
            "Skipping a_finished_test_threads_database_is_dropped_past_its_abandoned_transaction: \
             this MongoDB refuses lockInfo ({refused}), so the sweeper cannot find the sessions \
             to end"
        );
        return;
    }

    let probe = std::thread::Builder::new()
        .name("db_cleanup_abandoned_txn_probe".to_owned())
        .spawn(move || probe_runtime().block_on(abandoned_transaction_probe(url)))
        .expect("spawn probe thread");
    let joined = tokio::task::spawn_blocking(move || probe.join())
        .await
        .expect("join task");
    let probed = match joined {
        Ok(probed) => probed,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    let Some((database, lsid)) = probed else {
        return;
    };

    assert!(
        wait_until_dropped(&raw, &database, OPEN_TRANSACTION_DROP_BOUND).await,
        "{database} of a finished test thread still exists {} s after the thread ended while its \
         transaction was open (#2436)",
        OPEN_TRANSACTION_DROP_BOUND.as_secs()
    );
    assert!(
        !session_is_listed(&raw, &lsid).await,
        "the abandoned transaction on {database} is still open after the database was dropped"
    );
}

/// Runs on the probe thread of
/// [`a_finished_test_threads_database_is_dropped_past_its_abandoned_transaction`]:
/// names a database, opens a transaction on it and walks away from it.
async fn abandoned_transaction_probe(url: String) -> Option<(String, Bson)> {
    let raw = raw_test_client(&url).await.expect("raw client");
    let database = build_test_database_name("db_cleanup_abandoned_txn");
    let session = open_transaction_on(&raw, &database).await?;
    let lsid = session_lsid(&session);
    // No abort ever runs: dropping the session would send one in the background.
    std::mem::forget(session);
    Some((database, lsid))
}

/// One drop attempt must end on the server within its own bound, instead of
/// queuing for the database lock behind an open transaction, so it gives its
/// write ticket back (#2436). Before, it waited as long as the transaction
/// lived, and kept running on the server after the client gave up.
#[tokio::test]
async fn a_queued_drop_attempt_ends_on_the_server() {
    let Some(url) = shared_mongo::connection_string().await else {
        eprintln!(
            "Skipping a_queued_drop_attempt_ends_on_the_server (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let raw = raw_test_client(&url).await.expect("raw client");
    let database = build_test_database_name("db_cleanup_queued_drop");
    let Some(mut session) = open_transaction_on(&raw, &database).await else {
        return;
    };
    let cleanup = cleanup_client(&url).await.expect("cleanup client");

    // The hang guard is far below the 60 s the old unbounded drop waited at least.
    let attempt =
        tokio::time::timeout(Duration::from_secs(30), drop_attempt(&cleanup, &database)).await;
    // MaxTimeMSExpired (50), LockTimeout (24) or ExceededTimeLimit (262).
    let queued = matches!(&attempt, Ok(Err(error)) if is_queued_drop(error));
    let server_code = match &attempt {
        Ok(Err(error)) => match error.kind.as_ref() {
            mongodb::error::ErrorKind::Command(command) => Some(command.code),
            _ => None,
        },
        _ => None,
    };
    assert!(
        queued,
        "a drop attempt queued behind an open transaction on {database} must end with a time \
         limit error within 30 s, got {attempt:?} (server code {server_code:?}) (#2436)"
    );
    // The server may take a moment to list the ended drop as gone: poll for 2 s.
    let mut still_running = drop_is_running(&raw, &database).await;
    for _ in 0..20 {
        if !still_running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        still_running = drop_is_running(&raw, &database).await;
    }
    assert!(
        !still_running,
        "the timed-out drop of {database} is still running on the server"
    );

    session
        .abort_transaction()
        .await
        .expect("abort the probe transaction");
    // `drop_finished_database` sends a queued attempt again, so a write-ticket stall right
    // after the abort cannot fail the test.
    let dropped = tokio::time::timeout(
        Duration::from_secs(30),
        drop_finished_database(&cleanup, &database),
    )
    .await;
    assert!(
        matches!(dropped, Ok(Ok(_))),
        "the drop of {database} must succeed once the transaction is gone: {dropped:?}"
    );
    assert!(
        !database_exists(&raw, &database).await,
        "{database} still exists after its drop"
    );
}

/// The value of the first of `keys` present in `doc`, whatever numeric BSON
/// type it has.
fn num(doc: Option<&Document>, keys: &[&str]) -> Option<f64> {
    let doc = doc?;
    keys.iter().find_map(|key| match doc.get(*key)? {
        Bson::Int32(n) => Some(f64::from(*n)),
        Bson::Int64(n) => Some(*n as f64),
        Bson::Double(n) => Some(*n),
        _ => None,
    })
}

fn sub<'a>(doc: Option<&'a Document>, key: &str) -> Option<&'a Document> {
    doc?.get_document(key).ok()
}

fn show(value: Option<f64>, decimals: usize) -> String {
    value.map_or_else(|| "n/a".to_owned(), |v| format!("{v:.decimals$}"))
}

/// Average latency in ms of one `opLatencies` section (`writes`, `commands`).
fn average_ms(op_latencies: Option<&Document>, kind: &str) -> Option<f64> {
    let section = sub(op_latencies, kind);
    let latency = num(section, &["latency"])?;
    let ops = num(section, &["ops"])?;
    (ops > 0.0).then(|| latency / ops / 1000.0)
}

/// One `[hfs-2311-report]` line on stderr. Like [`log`], it bypasses libtest's
/// output capture, so the report shows without `--nocapture`.
fn report_line(args: std::fmt::Arguments<'_>) {
    let _ = writeln!(std::io::stderr().lock(), "[hfs-2311-report] {args}");
}

/// #2311 end-of-run report. Prints what the shared mongod holds after the
/// suite: leftover test databases, WiredTiger data handles, checkpoint time and
/// average latencies (server-wide, since the mongod started).
#[tokio::test]
#[ignore = "#2311 end-of-run report: run with --ignored after the suite, against the same HFS_TEST_MONGODB_URL"]
async fn end_of_run_report() {
    let Some(url) = test_mongo_url() else {
        eprintln!("Skipping end_of_run_report (set HFS_TEST_MONGODB_URL to the suite's mongod)");
        return;
    };
    let client = raw_test_client(&url).await.expect("raw client");
    let leftovers = client
        .list_database_names()
        .filter(doc! { "name": { "$regex": format!("^{MONGODB_TEST_DB_PREFIX}") } })
        .await
        .expect("listDatabases");
    let status = client
        .database("admin")
        .run_command(doc! { "serverStatus": 1_i32 })
        .await
        .expect("serverStatus");

    let wiredtiger = sub(Some(&status), "wiredTiger");
    let transaction = sub(wiredtiger, "transaction");
    let checkpoint = sub(wiredtiger, "checkpoint");
    let data_handles = num(
        sub(wiredtiger, "data-handle"),
        &["connection data handles currently active"],
    );
    let drop_pending = num(sub(Some(&status), "storageEngine"), &["dropPendingIdents"]);
    let checkpoints = num(
        checkpoint,
        &["number of checkpoints started", "number of checkpoints"],
    )
    .or_else(|| num(transaction, &["transaction checkpoints"]));
    let checkpoint_total_ms = num(checkpoint, &["total time (msecs)"])
        .or_else(|| num(transaction, &["transaction checkpoint total time (msecs)"]));
    let checkpoint_recent_ms = num(checkpoint, &["most recent time (msecs)"]).or_else(|| {
        num(
            transaction,
            &["transaction checkpoint most recent time (msecs)"],
        )
    });
    let op_latencies = sub(Some(&status), "opLatencies");

    report_line(format_args!(
        "hfs_phase2_mongo_databases={} wt_data_handles={} \
         drop_pending_idents={} checkpoints={} checkpoint_total_ms={} \
         checkpoint_most_recent_ms={} avg_write_ms={} avg_command_ms={}",
        leftovers.len(),
        show(data_handles, 0),
        show(drop_pending, 0),
        show(checkpoints, 0),
        show(checkpoint_total_ms, 0),
        show(checkpoint_recent_ms, 0),
        show(average_ms(op_latencies, "writes"), 1),
        show(average_ms(op_latencies, "commands"), 1),
    ));
    for name in leftovers.iter().take(20) {
        report_line(format_args!("leftover {name}"));
    }

    if let Some(max) = std::env::var("HFS_TEST_MONGODB_MAX_LEFTOVER_DATABASES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
    {
        assert!(
            leftovers.len() <= max,
            "{} test databases are left on the mongod, more than \
             HFS_TEST_MONGODB_MAX_LEFTOVER_DATABASES={max}: {leftovers:?}",
            leftovers.len()
        );
    }
}
