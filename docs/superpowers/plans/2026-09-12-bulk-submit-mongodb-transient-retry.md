# Bulk Submit MongoDB Transient Retry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A transient MongoDB driver error during a `$bulk-submit` batch is retried per command with bounded backoff and never duplicates what an earlier attempt landed; when retries are exhausted the batch's entries get per-line `processing-error` receipts and the file continues, instead of the rest of the file being silently abandoned (#1001).

**Architecture:** A shared `retry.rs` module (policy + cancel-aware bounded retry + error mapping) lifted out of `user_settings.rs`; every driver command in `bulk_ingest.rs` and the four bookkeeping commands in `process_entries` run through it, each with a replay rule chosen by the target collection's unique index; `ingest_batch` is split into `write_batch` (may fail) and a containment wrapper that always writes receipts. Integration tests inject faults with MongoDB's `failCommand` failpoint, scoped to one client by `appName`.

**Tech Stack:** Rust 1.90 / edition 2024, `mongodb` driver 3.7, `tokio` (paused-clock unit tests), `testcontainers-modules` `mongo:5.0.6` (standalone).

**Spec:** `docs/bulk-submit-mongodb-transient-retry.md` — read it first; the plan argues from it and the task briefs below reference its sections.

## Global Constraints

- **Work in the worktree** `C:\Users\DougC\Code\Helios\hfs\.claude\worktrees\fix-1001-transient-retry` on branch `fix/1001-bulk-submit-transient-retry`. Every path below is relative to that root. Never touch the main checkout at `C:\Users\DougC\Code\Helios\hfs`.
- **Share the build cache:** prefix every `cargo` command with `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target` (bash) so the worktree does not rebuild the workspace from scratch.
- **Feature flag:** all commands use `--features mongodb`. Unit tests: `cargo test -p helios-persistence --features mongodb --lib <filter>`. Integration tests: `cargo test -p helios-persistence --features mongodb --test mongodb_tests <filter> -- --nocapture`.
- **A skipped test is not a pass.** Every `mongodb_tests` test returns early (skips) when Docker/Mongo is unavailable, printing `Skipping …`. After running an integration test, confirm its output shows no `Skipping` line and that the assertions ran. If Mongo cannot be reached, stop and report — do not mark the step done.
- **Standalone server facts the tests rely on:** the driver performs **no retry of its own** for `insert`/`update`/`delete` on a standalone server, so failpoint `times` counts for those commands are exact. For `find`, the driver **may** retry once, so `find`-based tests assert recovery only, never attempt counts.
- **`failCommand` is one server-global failpoint.** Its configuration is replaced by every `configureFailPoint` call, so all failpoint tests hold the `FAILPOINT_LOCK` mutex (Task 3) for their whole duration. `appName` scoping keeps them from affecting the rest of the suite, which keeps running in parallel.
- **Per-task gates before committing:** `cargo fmt -p helios-persistence` (and `-p helios-hfs` when `crates/hfs` changed), `cargo clippy -p helios-persistence --features mongodb --tests -- -D warnings` clean on the touched files, and the task's tests passing.
- **Commit style:** `fix(persistence): <imperative summary>` (see `git log --oneline -20`), body explaining *why*, and this exact trailer as the last lines of every commit message:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01ECbBsNhgXwBo4dQtFyVBXv
  ```
- **Ambiguity rule:** if a step cannot be done as written (an API differs, a failpoint option is not honoured by `mongo:5.0.6`, a borrow does not compile the way the plan shows), stop and report back with the exact error rather than improvising a different design.
- **Comments:** only where the *why* is non-obvious (a replay rule, an accepted risk). No comments that restate the code or reference this plan or the issue except where the existing file already uses `#NNNN` references.
- **Unique index facts** (from `crates/persistence/src/backends/mongodb/schema.rs`): `resources` unique on `(tenant_id, resource_type, id)`; `resource_history` unique on `(tenant_id, resource_type, id, version_id)`; `bulk_submission_changes` unique on `(tenant_id, submitter, submission_id, change_id)`; `bulk_entry_results` unique on `(…manifest_id, file_url, line_number)`; `search_index` has **no** unique index.

---

## File structure

| File | Responsibility after this plan |
|---|---|
| `crates/persistence/src/backends/mongodb/retry.rs` (new) | `RetryPolicy`, the two policies, `Attempted`, `is_transient_mongo_error`, `retry_transient_with`, `retry_transient`, `exhausted` / `or_exhausted`; unit tests |
| `crates/persistence/src/backends/mongodb/user_settings.rs` | uses `super::retry::retry_transient`; loses its private copy |
| `crates/persistence/src/backends/mongodb/mod.rs` | declares `mod retry;` |
| `crates/persistence/src/backends/mongodb/backend.rs` | `MongoBackendConfig::app_name` |
| `crates/hfs/src/main.rs` | passes `app_name` in its exhaustive config literal |
| `crates/persistence/src/backends/mongodb/bulk_submit.rs` | `process_entries` bookkeeping commands retried; `touch_submission` retried |
| `crates/persistence/src/backends/mongodb/bulk_ingest.rs` | per-stage retry + replay rules, `confirm_landed`, `row_matches_plan`, `write_batch` / `ingest_batch` split, `all_failed`; module doc |
| `crates/persistence/tests/mongodb_tests.rs` | container flag, `FailPoint` helper, `create_backend_with_app_name`, `count_docs`, the new `bulk_submit` cases |
| `.claude/skills/bulk-data-submit/SKILL.md` | MongoDB behaviour note |

---

### Task 1: Shared retry module

**Files:**
- Create: `crates/persistence/src/backends/mongodb/retry.rs`
- Modify: `crates/persistence/src/backends/mongodb/mod.rs` (add `mod retry;` after `mod bulk_submit;`)
- Modify: `crates/persistence/src/backends/mongodb/user_settings.rs:19-20` (imports) and `:375-427` (remove the moved items)

**Interfaces:**
- Produces (all `pub(super)`):
  - `struct RetryPolicy { max_attempts: u32, base: Duration, cap: Duration }`
  - `const SETTINGS_RETRY: RetryPolicy` (4 attempts, 25 ms, cap 100 ms) and `const BULK_INGEST_RETRY: RetryPolicy` (6 attempts, 100 ms, cap 1 s)
  - `struct Attempted<T> { result: Result<T, MongoError>, attempts: u32 }`
  - `fn is_transient_mongo_error(err: &MongoError) -> bool`
  - `async fn retry_transient_with<T, F, Fut>(policy: &RetryPolicy, cancel: Option<&CancelToken>, what: &str, op: F) -> Attempted<T>`
  - `async fn retry_transient<T, F, Fut>(op: F) -> Result<T, MongoError>` (unchanged signature for `user_settings.rs`)
  - `fn exhausted(context: &str, attempts: u32, err: &MongoError) -> StorageError`
  - `fn or_exhausted<T>(context: &str, attempted: Attempted<T>) -> StorageResult<T>`

- [ ] **Step 1: Write the failing unit tests**

Create `crates/persistence/src/backends/mongodb/retry.rs` with only the test module for now (the items it references do not exist yet):

```rust
//! Bounded retry of transient MongoDB driver errors.
//!
//! The driver retries a retryable write once on a replica set and never on a
//! standalone server; a server that stays busy longer than that outlasts it.
//! This module adds a short, bounded, cancel-aware retry on top, shared by the
//! settings store and the `$bulk-submit` batch ingest.

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn io_error() -> MongoError {
        MongoError::from(std::io::Error::from(std::io::ErrorKind::TimedOut))
    }

    fn custom_error() -> MongoError {
        MongoError::custom("not transient")
    }

    #[test]
    fn io_and_pool_errors_are_transient_custom_is_not() {
        assert!(is_transient_mongo_error(&io_error()));
        assert!(!is_transient_mongo_error(&custom_error()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_transient_error_is_retried_with_the_policy_backoff() {
        let calls = Arc::new(AtomicU32::new(0));
        let started = tokio::time::Instant::now();
        let attempted = retry_transient_with(&BULK_INGEST_RETRY, None, "test", {
            let calls = calls.clone();
            move || {
                let calls = calls.clone();
                async move {
                    if calls.fetch_add(1, Ordering::SeqCst) < 2 {
                        Err(io_error())
                    } else {
                        Ok(42)
                    }
                }
            }
        })
        .await;
        assert_eq!(attempted.result.unwrap(), 42);
        assert_eq!(attempted.attempts, 3);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        // 100 ms before attempt 2, 200 ms before attempt 3.
        assert_eq!(started.elapsed(), Duration::from_millis(300));
    }

    #[tokio::test(start_paused = true)]
    async fn a_non_transient_error_is_returned_on_the_first_attempt() {
        let started = tokio::time::Instant::now();
        let attempted =
            retry_transient_with(&BULK_INGEST_RETRY, None, "test", || async { Err::<(), _>(custom_error()) })
                .await;
        assert!(attempted.result.is_err());
        assert_eq!(attempted.attempts, 1);
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn exhaustion_returns_the_last_error_after_max_attempts() {
        let started = tokio::time::Instant::now();
        let attempted =
            retry_transient_with(&BULK_INGEST_RETRY, None, "test", || async { Err::<(), _>(io_error()) })
                .await;
        assert!(attempted.result.is_err());
        assert_eq!(attempted.attempts, BULK_INGEST_RETRY.max_attempts);
        // 100 + 200 + 400 + 800 + 1000 (capped) ms.
        assert_eq!(started.elapsed(), Duration::from_millis(2500));
    }

    #[tokio::test(start_paused = true)]
    async fn the_settings_policy_sleeps_25_50_100() {
        let started = tokio::time::Instant::now();
        let attempted =
            retry_transient_with(&SETTINGS_RETRY, None, "test", || async { Err::<(), _>(io_error()) })
                .await;
        assert_eq!(attempted.attempts, 4);
        assert_eq!(started.elapsed(), Duration::from_millis(175));
    }

    #[tokio::test(start_paused = true)]
    async fn a_tripped_cancel_token_stops_before_the_next_sleep() {
        let cancel = CancelToken::new();
        let started = tokio::time::Instant::now();
        let attempted = retry_transient_with(&BULK_INGEST_RETRY, Some(&cancel), "test", {
            let cancel = cancel.clone();
            move || {
                cancel.cancel();
                async { Err::<(), _>(io_error()) }
            }
        })
        .await;
        assert!(attempted.result.is_err());
        assert_eq!(attempted.attempts, 1, "no second attempt once cancelled");
        assert_eq!(started.elapsed(), Duration::ZERO, "no backoff sleep once cancelled");
    }

    #[test]
    fn exhausted_maps_transient_to_unavailable_and_other_to_internal() {
        let transient = exhausted("insert batch resources", 6, &io_error());
        match transient {
            StorageError::Backend(BackendError::Unavailable { message, .. }) => {
                assert!(message.starts_with("insert batch resources: "));
                assert!(message.ends_with(" (after 6 attempts)"), "{message}");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
        let internal = exhausted("insert batch resources", 1, &custom_error());
        match internal {
            StorageError::Backend(BackendError::Internal { message, .. }) => {
                assert!(message.starts_with("insert batch resources: "));
                assert!(!message.contains("attempts"), "{message}");
            }
            other => panic!("expected Internal, got {other:?}"),
        }
    }
}
```

Add `mod retry;` to `crates/persistence/src/backends/mongodb/mod.rs` directly after `mod bulk_submit;`.

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --lib backends::mongodb::retry`
Expected: compile errors — `RetryPolicy`, `retry_transient_with`, etc. not found.

- [ ] **Step 3: Implement the module**

Insert above the `#[cfg(test)]` module in `retry.rs`:

```rust
use std::future::Future;
use std::time::Duration;

use mongodb::error::{Error as MongoError, ErrorKind, RETRYABLE_ERROR, RETRYABLE_WRITE_ERROR};

use crate::core::bulk_submit::CancelToken;
use crate::error::{BackendError, StorageError, StorageResult};

/// How many times an operation runs and how long it waits in between.
#[derive(Debug, Clone, Copy)]
pub(super) struct RetryPolicy {
    /// Total attempts, the first included.
    pub max_attempts: u32,
    /// Sleep before the second attempt; doubles before each later one.
    pub base: Duration,
    /// Upper bound on any single sleep.
    pub cap: Duration,
}

impl RetryPolicy {
    /// Sleep after the failure of attempt `attempt` (1-based).
    fn backoff(&self, attempt: u32) -> Duration {
        let factor = 1u32 << (attempt - 1).min(16);
        self.base.saturating_mul(factor).min(self.cap)
    }
}

/// Settings-store policy: 25, 50, 100 ms. A brief blip, never a long stall
/// for an interactive caller.
pub(super) const SETTINGS_RETRY: RetryPolicy = RetryPolicy {
    max_attempts: 4,
    base: Duration::from_millis(25),
    cap: Duration::from_millis(100),
};

/// Bulk-ingest policy: 100, 200, 400, 800, 1000 ms — 2.5 s of sleep across
/// six attempts, enough for a cleared pool to reconnect under load (#1001).
pub(super) const BULK_INGEST_RETRY: RetryPolicy = RetryPolicy {
    max_attempts: 6,
    base: Duration::from_millis(100),
    cap: Duration::from_secs(1),
};

/// An operation's final result and how many times it ran.
pub(super) struct Attempted<T> {
    pub result: Result<T, MongoError>,
    pub attempts: u32,
}

/// True when a MongoDB error is transient and safe to retry: one the driver has
/// itself labelled retryable, or a fast network/connection failure (e.g. a
/// connection reset by a momentarily overloaded server). The driver retries such
/// errors once on a replica set and never on a standalone; a server that stays
/// busy longer than that outlasts the single retry, so we add a short bounded
/// retry on top. Non-transient errors (duplicate key, bad command, decode) are
/// never retried here.
///
/// A `ServerSelection` timeout is deliberately *not* treated as transient: it
/// already means the driver waited its full `server_selection_timeout` and found
/// no usable server, so a fast backoff-retry would just pay that wait again
/// (blocking the caller for minutes against a genuinely-down server) without
/// improving the odds. Such an error is surfaced promptly instead.
///
/// An `InsertMany` error carrying per-document `write_errors` is never
/// transient whatever labels ride on it: the server executed the command and
/// reported per-document outcomes, which the caller must attribute.
pub(super) fn is_transient_mongo_error(err: &MongoError) -> bool {
    if let ErrorKind::InsertMany(insert_many) = err.kind.as_ref()
        && insert_many.write_errors.is_some()
    {
        return false;
    }
    err.contains_label(RETRYABLE_ERROR)
        || err.contains_label(RETRYABLE_WRITE_ERROR)
        || matches!(
            err.kind.as_ref(),
            ErrorKind::Io(_) | ErrorKind::ConnectionPoolCleared { .. }
        )
}

/// Runs `op` until it succeeds, fails with a non-transient error, or
/// `policy.max_attempts` is reached. A tripped `cancel` token ends the loop
/// before the next sleep, so an aborted submission never pays the backoff.
pub(super) async fn retry_transient_with<T, F, Fut>(
    policy: &RetryPolicy,
    cancel: Option<&CancelToken>,
    what: &str,
    mut op: F,
) -> Attempted<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, MongoError>>,
{
    let mut attempts: u32 = 1;
    loop {
        match op().await {
            Ok(value) => {
                return Attempted {
                    result: Ok(value),
                    attempts,
                };
            }
            Err(err) if attempts < policy.max_attempts && is_transient_mongo_error(&err) => {
                if cancel.is_some_and(CancelToken::is_cancelled) {
                    return Attempted {
                        result: Err(err),
                        attempts,
                    };
                }
                let backoff = policy.backoff(attempts);
                tracing::warn!(
                    attempt = attempts,
                    max_attempts = policy.max_attempts,
                    backoff_ms = backoff.as_millis() as u64,
                    "transient mongodb error during {what}; retrying: {err}"
                );
                tokio::time::sleep(backoff).await;
                attempts += 1;
            }
            Err(err) => {
                return Attempted {
                    result: Err(err),
                    attempts,
                };
            }
        }
    }
}

/// The settings store's shape: the fast policy, no cancellation.
///
/// The settings-store writes are already safe to re-run: reads are pure, and a
/// re-executed insert/update is caught by the version-conditioned filter and the
/// duplicate-key path in `MongoBackend::write_settings`, so a retry after a
/// lost acknowledgement cannot double-apply.
pub(super) async fn retry_transient<T, F, Fut>(op: F) -> Result<T, MongoError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, MongoError>>,
{
    retry_transient_with(&SETTINGS_RETRY, None, "user_settings", op)
        .await
        .result
}

/// Maps an operation's final driver error to a `StorageError`: a transient
/// error that outlived its retries is `Unavailable`, anything else `Internal`.
pub(super) fn exhausted(context: &str, attempts: u32, err: &MongoError) -> StorageError {
    let message = if attempts == 1 {
        format!("{context}: {err}")
    } else {
        format!("{context}: {err} (after {attempts} attempts)")
    };
    StorageError::Backend(if is_transient_mongo_error(err) {
        BackendError::Unavailable {
            backend_name: "mongodb".to_string(),
            message,
        }
    } else {
        BackendError::Internal {
            backend_name: "mongodb".to_string(),
            message,
            source: None,
        }
    })
}

/// [`exhausted`] applied to an [`Attempted`].
pub(super) fn or_exhausted<T>(context: &str, attempted: Attempted<T>) -> StorageResult<T> {
    let Attempted { result, attempts } = attempted;
    result.map_err(|err| exhausted(context, attempts, &err))
}
```

Then in `user_settings.rs`:
1. Delete lines 375-427 (from `/// Bound on retries when a MongoDB operation fails with a *transient* error.` through the closing `}` of `retry_transient`). Keep `is_duplicate_key_error` (lines 371-373) and everything after 427.
2. Add `use super::retry::retry_transient;` next to `use super::MongoBackend;`.
3. Remove `use std::future::Future;` and `use std::time::Duration;` only if `cargo check` reports them unused after the move.

- [ ] **Step 4: Run the unit tests**

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --lib backends::mongodb::retry`
Expected: 7 tests PASS.

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --test mongodb_tests settings_ -- --nocapture`
Expected: the `settings_*` integration tests still pass (or print `Skipping` only if Mongo is genuinely unavailable — see Global Constraints).

- [ ] **Step 5: Gates and commit**

Run `cargo fmt -p helios-persistence` and `cargo clippy -p helios-persistence --features mongodb --tests -- -D warnings`.

```bash
git add crates/persistence/src/backends/mongodb/retry.rs crates/persistence/src/backends/mongodb/mod.rs crates/persistence/src/backends/mongodb/user_settings.rs
git commit -m "fix(persistence): lift the MongoDB transient retry into a shared, cancel-aware module

The bulk-submit ingest needs the same bounded retry the settings store
already had privately (#1001). Parameterise it by policy, thread a cancel
token through so an aborted submission never pays the backoff, and map an
exhausted transient error to Unavailable so callers can tell it apart.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01ECbBsNhgXwBo4dQtFyVBXv"
```

---

### Task 2: `app_name` on `MongoBackendConfig`

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/backend.rs:47` (connect_client), `:103-160` (struct + Default), `:162-180` (default fns)
- Modify: `crates/hfs/src/main.rs:204-214`

**Interfaces:**
- Produces: `MongoBackendConfig { pub app_name: String, .. }`, default `"helios-persistence"`.

- [ ] **Step 1: Write the failing test**

Find the existing `#[cfg(test)] mod tests` in `backend.rs` (grep `mod tests`); if none exists, add one at the end of the file. Add:

```rust
#[test]
fn app_name_defaults_to_the_historical_constant() {
    assert_eq!(MongoBackendConfig::default().app_name, "helios-persistence");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --lib app_name_defaults`
Expected: compile error — no field `app_name`.

- [ ] **Step 3: Implement**

In `backend.rs`, add to `MongoBackendConfig` after `max_included_resources`:

```rust
    /// `appName` the driver sends on every connection. Visible in the server's
    /// `currentOp`/logs and usable to scope a `failCommand` failpoint to one
    /// client in tests.
    #[serde(default = "default_app_name")]
    pub app_name: String,
```

Add next to the other default fns:

```rust
fn default_app_name() -> String {
    "helios-persistence".to_string()
}
```

In `impl Default for MongoBackendConfig`, add `app_name: default_app_name(),`.

In `connect_client`, replace `client_options.app_name = Some("helios-persistence".to_string());` with `client_options.app_name = Some(config.app_name.clone());`.

In `crates/hfs/src/main.rs`, in the struct literal at lines 204-214 add, after `max_included_resources,`:

```rust
        app_name: MongoBackendConfig::default().app_name,
```

- [ ] **Step 4: Run the test and the workspace check**

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --lib app_name_defaults`
Expected: PASS.

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo check --workspace --all-features`
Expected: clean (this is what catches the `main.rs` literal).

- [ ] **Step 5: Gates and commit**

`cargo fmt -p helios-persistence -p helios-hfs`; clippy as in Global Constraints.

```bash
git add crates/persistence/src/backends/mongodb/backend.rs crates/hfs/src/main.rs
git commit -m "fix(persistence): make the MongoDB client appName configurable

Tests need to scope a server failpoint to one client, which failCommand
does by appName. Default unchanged.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01ECbBsNhgXwBo4dQtFyVBXv"
```

---

### Task 3: Failpoint test infrastructure

**Files:**
- Modify: `crates/persistence/tests/mongodb_tests.rs:266` (container command), `:636-650` (next to `create_backend_with_search_offloaded`), and inside `mod bulk_submit` (after `MockFetcher`, ~line 5883)

**Interfaces:**
- Produces (top level of the test file): `async fn create_backend_with_app_name(test_name: &str, app_name: &str) -> Option<MongoBackend>`; `async fn count_docs(test_name: &str, collection: &str, filter: Document) -> u64`.
- Produces (inside `mod bulk_submit`): `struct FailPoint`, `FailPoint::enable(app_name: &str, data: Document, mode: Document) -> Option<FailPoint>`, `FailPoint::off(self)`; `static FAILPOINT_LOCK: tokio::sync::Mutex<()>`.

- [ ] **Step 1: Enable test commands on the shared container**

At `mongodb_tests.rs:266` change the command to:

```rust
                    .with_cmd([
                        "mongod",
                        "--bind_ip_all",
                        "--wiredTigerCacheSizeGB",
                        "0.25",
                        // `failCommand` (used by the bulk-submit retry tests)
                        // is only registered when test commands are enabled.
                        "--setParameter",
                        "enableTestCommands=1",
                    ])
```

- [ ] **Step 2: Add the backend and counting helpers**

After `create_backend_with_search_offloaded` (top level) add:

```rust
/// A backend whose driver connections carry `app_name`, so a `failCommand`
/// failpoint configured with `data.appName` hits only this backend.
async fn create_backend_with_app_name(test_name: &str, app_name: &str) -> Option<MongoBackend> {
    let connection_string = shared_mongo::connection_string().await?;
    let config = MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test_name),
        app_name: app_name.to_string(),
        ..Default::default()
    };
    build_backend(config).await
}

/// Counts documents in one of a test database's collections, through a plain
/// driver client (no failpoint `appName`, so never subject to one).
async fn count_docs(test_name: &str, collection: &str, filter: Document) -> u64 {
    let connection_string = shared_mongo::connection_string()
        .await
        .expect("count_docs is only called after a backend was created");
    let client = Client::with_uri_str(&connection_string).await.unwrap();
    client
        .database(&build_test_database_name(test_name))
        .collection::<Document>(collection)
        .count_documents(filter)
        .await
        .unwrap()
}
```

- [ ] **Step 3: Add the `FailPoint` helper inside `mod bulk_submit`**

Directly after the `impl SubmitInputFetcher for MockFetcher { .. }` block add:

```rust
    /// `failCommand` is one server-global failpoint: every `configureFailPoint`
    /// replaces its configuration. Tests that use it hold this lock for their
    /// whole duration; `data.appName` keeps them from touching the rest of the
    /// suite, which keeps running in parallel.
    static FAILPOINT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// A `failCommand` failpoint scoped to one client's `appName`.
    struct FailPoint {
        admin: mongodb::Database,
        _lock: tokio::sync::MutexGuard<'static, ()>,
    }

    impl FailPoint {
        /// Configures `failCommand` for connections whose `appName` is `app_name`.
        /// Returns `None`, after printing why, when the server was not started
        /// with `enableTestCommands=1` (an external `HFS_TEST_MONGODB_URL`).
        async fn enable(app_name: &str, mut data: Document, mode: Document) -> Option<FailPoint> {
            let lock = FAILPOINT_LOCK.lock().await;
            let connection_string = shared_mongo::connection_string().await?;
            let admin = Client::with_uri_str(&connection_string)
                .await
                .unwrap()
                .database("admin");
            let enabled = admin
                .run_command(doc! { "getParameter": 1, "enableTestCommands": 1 })
                .await
                .ok()
                .and_then(|r| r.get_bool("enableTestCommands").ok())
                .unwrap_or(false);
            if !enabled {
                eprintln!(
                    "Skipping failpoint test: mongod was not started with \
                     --setParameter enableTestCommands=1"
                );
                return None;
            }
            data.insert("appName", app_name);
            admin
                .run_command(doc! {
                    "configureFailPoint": "failCommand",
                    "mode": mode,
                    "data": data,
                })
                .await
                .expect("configureFailPoint failCommand");
            Some(FailPoint { admin, _lock: lock })
        }

        /// Turns the failpoint off and releases the lock. Call at the end of
        /// every test; a `times`-bounded failpoint that is never turned off
        /// still only affects its own `appName`.
        async fn off(self) {
            let _ = self
                .admin
                .run_command(doc! { "configureFailPoint": "failCommand", "mode": "off" })
                .await;
        }
    }
```

- [ ] **Step 4: Write the smoke test**

After the helper add:

```rust
    /// Pins the failpoint plumbing every retry test relies on: it fires for the
    /// scoped `appName`, is spent after `times`, and does not touch another client.
    #[tokio::test]
    async fn failpoint_hits_only_the_scoped_app_name() {
        let Some(connection_string) = shared_mongo::connection_string().await else {
            return;
        };
        let Some(fail_point) = FailPoint::enable(
            "fp-smoke-target",
            doc! { "failCommands": ["insert"], "closeConnection": true },
            doc! { "times": 1 },
        )
        .await
        else {
            return;
        };

        let client_for = |app_name: &str| {
            let connection_string = connection_string.clone();
            let app_name = app_name.to_string();
            async move {
                let mut options = mongodb::options::ClientOptions::parse(&connection_string)
                    .await
                    .unwrap();
                options.app_name = Some(app_name);
                Client::with_options(options).unwrap()
            }
        };
        let db_name = build_test_database_name("failpoint_smoke");

        let target = client_for("fp-smoke-target").await;
        let coll = target.database(&db_name).collection::<Document>("smoke");
        let first = coll.insert_one(doc! { "n": 1 }).await;
        assert!(
            matches!(
                first.as_ref().map_err(|e| e.kind.as_ref()),
                Err(mongodb::error::ErrorKind::Io(_))
            ),
            "the scoped client's first insert is dropped: {first:?}"
        );
        coll.insert_one(doc! { "n": 2 })
            .await
            .expect("the failpoint is spent after one use");

        let other = client_for("fp-smoke-other").await;
        other
            .database(&db_name)
            .collection::<Document>("smoke")
            .insert_one(doc! { "n": 3 })
            .await
            .expect("a client with another appName is unaffected");

        fail_point.off().await;
    }
```

- [ ] **Step 5: Run the smoke test**

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --test mongodb_tests failpoint_hits_only -- --nocapture`
Expected: PASS with no `Skipping` line. If the assertion on `ErrorKind::Io` fails because the driver surfaces the dropped connection as a different kind (e.g. `ConnectionPoolCleared`), widen the `matches!` to both kinds and note it in the commit body — both are transient for the retry helper.

- [ ] **Step 6: Gates and commit**

```bash
git add crates/persistence/tests/mongodb_tests.rs
git commit -m "test(persistence): failCommand failpoint helper for the MongoDB suite

Enables test commands on the shared standalone container and adds a
FailPoint guard scoped by appName and serialised by a mutex, since
failCommand is one server-global failpoint.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01ECbBsNhgXwBo4dQtFyVBXv"
```

---

### Task 4: Retry the bookkeeping commands in `process_entries`

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/bulk_submit.rs:310-324` (`touch_submission`), `:823-901` (`process_entries`)
- Test: `crates/persistence/tests/mongodb_tests.rs` (`mod bulk_submit`)

**Interfaces:**
- Consumes: `super::retry::{BULK_INGEST_RETRY, or_exhausted, retry_transient_with}` from Task 1; `FailPoint`, `create_backend_with_app_name` from Task 3.
- Produces: no signature changes.

- [ ] **Step 1: Write the failing tests**

Add to `mod bulk_submit`:

```rust
    fn three_patients(prefix: &str) -> Vec<NdjsonEntry> {
        (1..=3)
            .map(|i| {
                NdjsonEntry::new(
                    i,
                    "Patient",
                    json!({"resourceType": "Patient", "id": format!("{prefix}-{i}")}),
                )
            })
            .collect()
    }

    /// The manifest bookkeeping around a batch — the `processing` promotion,
    /// the counters and `touch_submission` — is retried like the batch itself.
    /// On a standalone server the driver adds no retry, so two dropped
    /// `update`s are exactly two of ours.
    #[tokio::test]
    async fn bookkeeping_updates_survive_dropped_connections() {
        let app = "fp-bookkeeping-update";
        let Some(backend) = create_backend_with_app_name("submit_fp_bookkeeping_update", app).await
        else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;
        let Some(fail_point) = FailPoint::enable(
            app,
            doc! { "failCommands": ["update"], "closeConnection": true },
            doc! { "times": 2 },
        )
        .await
        else {
            return;
        };

        let results = backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                three_patients("bk"),
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();
        fail_point.off().await;

        assert!(results.iter().all(|r| r.is_success()));
        let counts = backend.get_entry_counts(&tenant, &id, &manifest_id).await.unwrap();
        assert_eq!(counts.total, 3);
        assert_eq!(counts.success, 3);
    }

    /// The manifest existence check before a batch is a `find`; the driver may
    /// retry a read once on its own, so this asserts recovery, not attempts.
    #[tokio::test]
    async fn manifest_check_survives_dropped_connections() {
        let app = "fp-bookkeeping-find";
        let Some(backend) = create_backend_with_app_name("submit_fp_bookkeeping_find", app).await
        else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;
        let Some(fail_point) = FailPoint::enable(
            app,
            doc! { "failCommands": ["find"], "closeConnection": true },
            doc! { "times": 3 },
        )
        .await
        else {
            return;
        };

        let results = backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                three_patients("bkf"),
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();
        fail_point.off().await;

        assert!(results.iter().all(|r| r.is_success()));
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --test mongodb_tests bookkeeping_updates_survive -- --nocapture`
Expected: FAIL — `process_entries` returns `Err(internal error in mongodb: mark manifest processing: …)`.

- [ ] **Step 3: Implement**

At the top of `bulk_submit.rs` add `use super::retry::{BULK_INGEST_RETRY, or_exhausted, retry_transient_with};`.

Replace `touch_submission` (lines 310-324) with:

```rust
    /// Bumps a submission's `updated_at` (the TTL cleanup scan reads it).
    async fn touch_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        let submissions = self.submissions().await?;
        let filter = submission_filter(tenant, id);
        or_exhausted(
            "touch submission",
            retry_transient_with(&BULK_INGEST_RETRY, None, "touch submission", || async {
                submissions
                    .update_one(
                        filter.clone(),
                        doc! { "$set": { "updated_at": to_bson_time(Utc::now()) } },
                    )
                    .await
            })
            .await,
        )?;
        Ok(())
    }
```

In `process_entries`:

1. Replace the existence check (lines 831-840) with a retried `find_one` — the manifest's content is not used, only its presence:

```rust
        let manifests = self.manifests().await?;
        let cancel = options.cancel.as_ref();
        let filter = manifest_filter(tenant, submission_id, manifest_id);
        let present = or_exhausted(
            "load manifest",
            retry_transient_with(&BULK_INGEST_RETRY, cancel, "load manifest", || async {
                manifests.find_one(filter.clone()).await
            })
            .await,
        )?;
        if present.is_none() {
            return Err(StorageError::BulkSubmit(
                BulkSubmitError::ManifestNotFound {
                    submission_id: submission_id.submission_id.clone(),
                    manifest_id: manifest_id.to_string(),
                },
            ));
        }
```

and delete the later `let manifests = self.manifests().await?;` line that the promotion block declared.

2. Replace the promotion `update_one` (lines 858-864) with:

```rust
        or_exhausted(
            "mark manifest processing",
            retry_transient_with(&BULK_INGEST_RETRY, cancel, "mark manifest processing", || async {
                manifests
                    .update_one(
                        promote.clone(),
                        doc! { "$set": { "status": ManifestStatus::Processing.to_string() } },
                    )
                    .await
            })
            .await,
        )?;
```

3. Replace the counters `update_one` (lines 886-897) with:

```rust
        // `$inc` is not idempotent: an attempt whose acknowledgement was lost
        // double-counts this batch on retry. Accepted — the receipts stay
        // authoritative and a manifest re-walk already over-counts the same way.
        let counters = doc! { "$inc": {
            "total_entries": results.len() as i64,
            "processed_entries": results.iter().filter(|r| r.is_success()).count() as i64,
            "failed_entries": error_count as i64,
            "last_processed_line": results.len() as i64,
        }};
        or_exhausted(
            "update manifest counts",
            retry_transient_with(&BULK_INGEST_RETRY, cancel, "update manifest counts", || async {
                manifests
                    .update_one(manifest_filter(tenant, submission_id, manifest_id), counters.clone())
                    .await
            })
            .await,
        )?;
```

`touch_submission` keeps its call unchanged.

- [ ] **Step 4: Run the tests**

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --test mongodb_tests bulk_submit:: -- --nocapture`
Expected: the two new tests PASS and every existing `bulk_submit::` test still passes.

- [ ] **Step 5: Gates and commit**

```bash
git add crates/persistence/src/backends/mongodb/bulk_submit.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "fix(persistence): retry the MongoDB bulk-submit manifest bookkeeping

A transient error on the processing promotion, the counters or the
submission touch aborted the whole file the same way the batch did
(#1001). Route them through the shared bounded retry, as sqlite already
does for its bookkeeping writes.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01ECbBsNhgXwBo4dQtFyVBXv"
```

---

### Task 5: Retry the resources stage with replay-aware attribution

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/bulk_ingest.rs:75-80` (imports), `:154-171` (`ingest_batch` call to `write_resources`), `:540-694` (`write_resources`)
- Test: `crates/persistence/tests/mongodb_tests.rs` (`mod bulk_submit`), plus an in-crate unit test in `bulk_ingest.rs`

**Interfaces:**
- Consumes: Task 1's `retry.rs` items; Task 3's helpers.
- Produces: `async fn write_resources(&self, db, tenant_id, planned: &PlannedBatch, cancel: Option<&CancelToken>) -> StorageResult<HashMap<usize, String>>`; `async fn confirm_landed(&self, db, tenant_id, planned: &PlannedBatch, plan_idxs: &[usize], cancel) -> StorageResult<HashSet<usize>>`; `fn row_matches_plan(row: &Document, plan: &ResourcePlan) -> StorageResult<bool>`.

- [ ] **Step 1: Write the failing unit test for `row_matches_plan`**

At the end of `bulk_ingest.rs` add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> ResourcePlan {
        let now = Utc::now();
        ResourcePlan {
            resource_type: "Patient".to_string(),
            id: "p1".to_string(),
            base_version: None,
            version: "1".to_string(),
            content: serde_json::json!({"resourceType": "Patient", "id": "p1", "active": true}),
            created_at: now,
            last_updated: now,
            fhir_version: FhirVersion::default_enabled(),
        }
    }

    fn row_for(plan: &ResourcePlan) -> Document {
        doc! {
            "id": &plan.id,
            "version_id": &plan.version,
            "last_updated": chrono_to_bson(plan.last_updated),
            "data": Bson::Document(value_to_document(&plan.content).unwrap()),
        }
    }

    #[test]
    fn a_row_with_the_planned_version_stamp_and_content_landed() {
        let plan = plan();
        assert!(row_matches_plan(&row_for(&plan), &plan).unwrap());
    }

    #[test]
    fn a_different_version_did_not_land() {
        let plan = plan();
        let mut row = row_for(&plan);
        row.insert("version_id", "2");
        assert!(!row_matches_plan(&row, &plan).unwrap());
    }

    #[test]
    fn a_different_timestamp_is_another_writer() {
        let plan = plan();
        let mut row = row_for(&plan);
        row.insert(
            "last_updated",
            chrono_to_bson(plan.last_updated + chrono::Duration::milliseconds(1)),
        );
        assert!(!row_matches_plan(&row, &plan).unwrap());
    }

    #[test]
    fn different_content_is_another_writer() {
        let plan = plan();
        let mut row = row_for(&plan);
        row.insert(
            "data",
            Bson::Document(doc! {"resourceType": "Patient", "id": "p1", "active": false}),
        );
        assert!(!row_matches_plan(&row, &plan).unwrap());
    }
}
```

- [ ] **Step 2: Write the failing integration tests**

Add to `mod bulk_submit` in `mongodb_tests.rs`:

```rust
    /// Asserts each id has exactly one resource, history and rollback row.
    async fn assert_one_row_each(test_name: &str, tenant: &TenantContext, ids: &[&str]) {
        let tenant_id = tenant.tenant_id().as_str();
        for id in ids {
            let by_id = doc! { "tenant_id": tenant_id, "resource_type": "Patient", "id": *id };
            assert_eq!(count_docs(test_name, "resources", by_id.clone()).await, 1, "resources {id}");
            assert_eq!(count_docs(test_name, "resource_history", by_id).await, 1, "history {id}");
            let change = doc! { "tenant_id": tenant_id, "resource_type": "Patient", "resource_id": *id };
            assert_eq!(count_docs(test_name, "bulk_submission_changes", change).await, 1, "changes {id}");
        }
    }

    /// Spec §5.2 case 1: a dropped `insert` is retried and every entry lands once.
    #[tokio::test]
    async fn dropped_insert_is_retried_and_lands_once() {
        let test = "submit_fp_dropped_insert";
        let app = "fp-dropped-insert";
        let Some(backend) = create_backend_with_app_name(test, app).await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;
        let Some(fail_point) = FailPoint::enable(
            app,
            doc! { "failCommands": ["insert"], "closeConnection": true },
            doc! { "times": 2 },
        )
        .await
        else {
            return;
        };

        let results = backend
            .process_entries(&tenant, &id, &manifest_id, three_patients("drop"), &BulkProcessingOptions::new())
            .await
            .unwrap();
        fail_point.off().await;

        assert!(results.iter().all(|r| r.is_success()), "{results:?}");
        assert_one_row_each(test, &tenant, &["drop-1", "drop-2", "drop-3"]).await;
    }

    /// Spec §5.2 case 2: the server executes the insert, then reports a
    /// retryable write-concern error. The retry finds its own rows (dup-key)
    /// and `confirm_landed` recognises them by version, timestamp and content.
    /// `times: 2` = the first insert (landed + error) and the retry (dup-key +
    /// error, which is attributed, not retried).
    #[tokio::test]
    async fn unacknowledged_insert_is_confirmed_not_duplicated() {
        let test = "submit_fp_unacked_insert";
        let app = "fp-unacked-insert";
        let Some(backend) = create_backend_with_app_name(test, app).await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;
        let Some(fail_point) = FailPoint::enable(
            app,
            doc! {
                "failCommands": ["insert"],
                "writeConcernError": {
                    "code": 91,
                    "errmsg": "Replication is being shut down",
                    "errorLabels": ["RetryableWriteError"],
                },
            },
            doc! { "times": 2 },
        )
        .await
        else {
            return;
        };

        let results = backend
            .process_entries(&tenant, &id, &manifest_id, three_patients("unack"), &BulkProcessingOptions::new())
            .await
            .unwrap();
        fail_point.off().await;

        assert!(results.iter().all(|r| r.is_success()), "{results:?}");
        assert_one_row_each(test, &tenant, &["unack-1", "unack-2", "unack-3"]).await;
    }

    /// The update path: one `update_one` per existing id, each its own retry unit.
    /// `times: 3` = the two dropped `processing` promotions plus one dropped update.
    #[tokio::test]
    async fn dropped_update_is_retried_and_versions_once() {
        let test = "submit_fp_dropped_update";
        let app = "fp-dropped-update";
        let Some(backend) = create_backend_with_app_name(test, app).await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;
        backend
            .process_entries(&tenant, &id, &manifest_id, three_patients("upd"), &BulkProcessingOptions::new())
            .await
            .unwrap();

        let Some(fail_point) = FailPoint::enable(
            app,
            doc! { "failCommands": ["update"], "closeConnection": true },
            doc! { "times": 3 },
        )
        .await
        else {
            return;
        };
        let updates: Vec<NdjsonEntry> = (1..=3)
            .map(|i| {
                NdjsonEntry::new(
                    i,
                    "Patient",
                    json!({"resourceType": "Patient", "id": format!("upd-{i}"), "active": true}),
                )
            })
            .collect();
        let results = backend
            .process_entries(&tenant, &id, &manifest_id, updates, &BulkProcessingOptions::new())
            .await
            .unwrap();
        fail_point.off().await;

        assert!(results.iter().all(|r| r.is_success()), "{results:?}");
        for i in 1..=3 {
            let versions = backend
                .list_versions(&tenant, "Patient", &format!("upd-{i}"))
                .await
                .unwrap();
            assert_eq!(versions.len(), 2, "upd-{i} has exactly versions 1 and 2");
        }
    }
```

- [ ] **Step 3: Run them to verify they fail**

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --lib backends::mongodb::bulk_ingest`
Expected: compile error — `row_matches_plan` not found.

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --test mongodb_tests dropped_insert_is_retried -- --nocapture`
Expected: FAIL — `process_entries` returns `Err(… insert batch resources …)`.

- [ ] **Step 4: Implement**

Imports at the top of `bulk_ingest.rs`: add `use super::retry::{Attempted, BULK_INGEST_RETRY, exhausted, or_exhausted, retry_transient_with};` and `use crate::core::bulk_submit::CancelToken;`.

In `ingest_batch` change the call to `self.write_resources(&db, tenant_id, &planned, options.cancel.as_ref()).await?`.

Replace `write_resources` with:

```rust
    /// Writes the batch's `resources` rows: one `insert` for the creates, one
    /// `update` per update. Returns the plans neither applied to.
    ///
    /// Attempt 1 keeps the per-entry path's semantics: a duplicate id is the
    /// race the pre-read could not see, and an update that matched nothing is a
    /// version conflict. On a retry the same outcomes are ambiguous — the
    /// earlier attempt may have landed the row before its acknowledgement was
    /// lost — so they are settled by [`Self::confirm_landed`] instead.
    async fn write_resources(
        &self,
        db: &Database,
        tenant_id: &str,
        planned: &PlannedBatch,
        cancel: Option<&CancelToken>,
    ) -> StorageResult<HashMap<usize, String>> {
        let mut failed = HashMap::new();
        let mut create_docs = Vec::new();
        let mut create_plans = Vec::new();
        let mut update_ops = Vec::new();

        for (plan_idx, plan) in planned.plans.iter().enumerate() {
            let payload = Bson::Document(value_to_document(&plan.content)?);
            let last_updated = chrono_to_bson(plan.last_updated);
            match &plan.base_version {
                None => {
                    create_plans.push(plan_idx);
                    create_docs.push(doc! {
                        "tenant_id": tenant_id,
                        "resource_type": &plan.resource_type,
                        "id": &plan.id,
                        "version_id": &plan.version,
                        "data": payload,
                        "created_at": chrono_to_bson(plan.created_at),
                        "last_updated": last_updated,
                        "is_deleted": false,
                        "deleted_at": Bson::Null,
                        "fhir_version": plan.fhir_version.as_mime_param(),
                    });
                }
                Some(base_version) => {
                    update_ops.push((
                        plan_idx,
                        doc! {
                            "tenant_id": tenant_id,
                            "resource_type": &plan.resource_type,
                            "id": &plan.id,
                            "version_id": base_version,
                            "is_deleted": false,
                        },
                        doc! { "$set": {
                            "version_id": &plan.version,
                            "data": payload,
                            "last_updated": last_updated,
                            "is_deleted": false,
                            "deleted_at": Bson::Null,
                            "fhir_version": plan.fhir_version.as_mime_param(),
                        }},
                    ));
                }
            }
        }

        if !create_docs.is_empty() {
            let collection = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
            let mut offset = 0;
            for chunk in create_docs.chunks(INSERT_DOCS_PER_COMMAND) {
                // Unordered: one duplicate id must not stop the rest of the
                // batch, the way one failing entry did not stop the next.
                let Attempted { result, attempts } = retry_transient_with(
                    &BULK_INGEST_RETRY,
                    cancel,
                    "insert batch resources",
                    || async { collection.insert_many(chunk).ordered(false).await },
                )
                .await;
                if let Err(e) = result {
                    // Phase 1: attribute per document without holding `e`
                    // across an await.
                    let needs_confirm = {
                        let ErrorKind::InsertMany(insert_many) = e.kind.as_ref() else {
                            return Err(exhausted("insert batch resources", attempts, &e));
                        };
                        let Some(write_errors) = insert_many.write_errors.as_ref() else {
                            return Err(exhausted("insert batch resources", attempts, &e));
                        };
                        let mut needs_confirm = Vec::new();
                        for write_error in write_errors {
                            let plan_idx = create_plans[offset + write_error.index];
                            if write_error.code == 11000 && attempts > 1 {
                                needs_confirm.push(plan_idx);
                                continue;
                            }
                            let plan = &planned.plans[plan_idx];
                            // A duplicate id means the row appeared between
                            // this batch's pre-read and its insert; the
                            // per-entry path reported the same conflict from
                            // `create`'s existence probe.
                            let diagnostics = if write_error.code == 11000 {
                                StorageError::Resource(ResourceError::AlreadyExists {
                                    resource_type: plan.resource_type.clone(),
                                    id: plan.id.clone(),
                                })
                                .to_string()
                            } else {
                                format!(
                                    "Failed to insert {}/{}: {}",
                                    plan.resource_type, plan.id, write_error.message
                                )
                            };
                            failed.insert(plan_idx, diagnostics);
                        }
                        needs_confirm
                    };
                    // Phase 2: a duplicate on a retry is either our own earlier
                    // attempt or a concurrent writer.
                    if !needs_confirm.is_empty() {
                        let landed = self
                            .confirm_landed(db, tenant_id, planned, &needs_confirm, cancel)
                            .await?;
                        for plan_idx in needs_confirm {
                            if !landed.contains(&plan_idx) {
                                let plan = &planned.plans[plan_idx];
                                failed.insert(
                                    plan_idx,
                                    StorageError::Resource(ResourceError::AlreadyExists {
                                        resource_type: plan.resource_type.clone(),
                                        id: plan.id.clone(),
                                    })
                                    .to_string(),
                                );
                            }
                        }
                    }
                }
                offset += chunk.len();
            }
        }

        if !update_ops.is_empty() {
            let collection = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
            let concurrency = self.bulk_write_concurrency().clamp(1, 16);
            let cancel_owned = cancel.cloned();
            let applied: Vec<(usize, Attempted<mongodb::results::UpdateResult>)> =
                futures::stream::iter(update_ops)
                    .map(|(plan_idx, filter, update)| {
                        let collection = collection.clone();
                        let cancel_owned = cancel_owned.clone();
                        async move {
                            let attempted = retry_transient_with(
                                &BULK_INGEST_RETRY,
                                cancel_owned.as_ref(),
                                "update batch resource",
                                || {
                                    let filter = filter.clone();
                                    let update = update.clone();
                                    let collection = collection.clone();
                                    async move { collection.update_one(filter, update).await }
                                },
                            )
                            .await;
                            (plan_idx, attempted)
                        }
                    })
                    .buffer_unordered(concurrency)
                    .collect()
                    .await;

            let mut needs_confirm = Vec::new();
            for (plan_idx, Attempted { result, attempts }) in applied {
                let plan = &planned.plans[plan_idx];
                match result {
                    Ok(outcome) if outcome.matched_count == 1 => {}
                    Ok(_) if attempts > 1 => needs_confirm.push(plan_idx),
                    // The guard matched nothing: another writer moved the row
                    // between this batch's pre-read and its write. There is no
                    // way to see that from a batched `update`'s aggregate
                    // counts — two batches that both read version N both target
                    // N+1, so the row carrying N+1 afterwards does not say whose
                    // write put it there — which is why each update is its own
                    // statement. They still go out `bulk_write_concurrency` at a
                    // time, and a fresh import has none of them at all.
                    Ok(_) => {
                        failed.insert(
                            plan_idx,
                            StorageError::Concurrency(ConcurrencyError::VersionConflict {
                                resource_type: plan.resource_type.clone(),
                                id: plan.id.clone(),
                                expected_version: plan.base_version.clone().unwrap_or_default(),
                                actual_version: "unknown".to_string(),
                            })
                            .to_string(),
                        );
                    }
                    Err(e) => {
                        failed.insert(
                            plan_idx,
                            exhausted("update batch resource", attempts, &e).to_string(),
                        );
                    }
                }
            }
            if !needs_confirm.is_empty() {
                let landed = self
                    .confirm_landed(db, tenant_id, planned, &needs_confirm, cancel)
                    .await?;
                for plan_idx in needs_confirm {
                    if !landed.contains(&plan_idx) {
                        let plan = &planned.plans[plan_idx];
                        failed.insert(
                            plan_idx,
                            StorageError::Concurrency(ConcurrencyError::VersionConflict {
                                resource_type: plan.resource_type.clone(),
                                id: plan.id.clone(),
                                expected_version: plan.base_version.clone().unwrap_or_default(),
                                actual_version: "unknown".to_string(),
                            })
                            .to_string(),
                        );
                    }
                }
            }
        }

        Ok(failed)
    }

    /// Which of `plan_idxs` the batch's own earlier attempt already wrote.
    ///
    /// A row counts as ours only when its version, its `last_updated` and its
    /// content all match the plan. The timestamp is the batch's own, minted
    /// once in `plan_batch`, so an unrelated writer's identical-content write
    /// carries a different one — except within the same millisecond, which is
    /// accepted: that collision at attempt 1 reports the conflict, and the
    /// window only exists inside a retry that already needed a transient error.
    async fn confirm_landed(
        &self,
        db: &Database,
        tenant_id: &str,
        planned: &PlannedBatch,
        plan_idxs: &[usize],
        cancel: Option<&CancelToken>,
    ) -> StorageResult<HashSet<usize>> {
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let mut by_type: HashMap<&str, Vec<usize>> = HashMap::new();
        for &plan_idx in plan_idxs {
            by_type
                .entry(planned.plans[plan_idx].resource_type.as_str())
                .or_default()
                .push(plan_idx);
        }

        let mut landed = HashSet::new();
        for (resource_type, idxs) in by_type {
            let ids: Vec<Bson> = idxs
                .iter()
                .map(|&i| Bson::from(planned.plans[i].id.as_str()))
                .collect();
            let filter = doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": { "$in": ids },
            };
            let rows: Vec<Document> = or_exhausted(
                "confirm batch writes",
                retry_transient_with(&BULK_INGEST_RETRY, cancel, "confirm batch writes", || async {
                    resources
                        .find(filter.clone())
                        .projection(doc! { "id": 1, "version_id": 1, "last_updated": 1, "data": 1 })
                        .await?
                        .try_collect()
                        .await
                })
                .await,
            )?;
            let by_id: HashMap<&str, &Document> = rows
                .iter()
                .filter_map(|row| row.get_str("id").ok().map(|id| (id, row)))
                .collect();
            for plan_idx in idxs {
                let plan = &planned.plans[plan_idx];
                if let Some(row) = by_id.get(plan.id.as_str())
                    && row_matches_plan(row, plan)?
                {
                    landed.insert(plan_idx);
                }
            }
        }
        Ok(landed)
    }
```

And add, as a free function next to `insert_documents`:

```rust
/// True when `row` is exactly what `plan` meant to write: same version, the
/// batch's own `last_updated`, and identical content.
fn row_matches_plan(row: &Document, plan: &ResourcePlan) -> StorageResult<bool> {
    let version_ok = row.get_str("version_id").is_ok_and(|v| v == plan.version);
    let stamp_ok = row
        .get_datetime("last_updated")
        .is_ok_and(|t| *t == chrono_to_bson(plan.last_updated));
    if !(version_ok && stamp_ok) {
        return Ok(false);
    }
    let planned = value_to_document(&plan.content)?;
    Ok(row.get_document("data").is_ok_and(|data| *data == planned))
}
```

`HashSet` is already imported. If `insert_many(chunk)` with `chunk: &[Document]` does not satisfy the driver's `IntoIterator<Item = impl Borrow<Document>>` bound, use `chunk.iter()`.

- [ ] **Step 5: Run the tests**

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --lib backends::mongodb::bulk_ingest`
Expected: 4 unit tests PASS.

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --test mongodb_tests bulk_submit:: -- --nocapture`
Expected: the three new tests PASS; every existing `bulk_submit::` test still passes (especially `test_two_batches_racing_for_the_same_ids_stay_consistent`, which pins attempt-1 semantics).

If `unacknowledged_insert_is_confirmed_not_duplicated` fails because `mongo:5.0.6` does not honour `writeConcernError` (the first insert returns `Ok` and nothing is retried) or does not attach `errorLabels`, **stop and report back** with the observed driver error; do not weaken the assertion.

- [ ] **Step 6: Gates and commit**

```bash
git add crates/persistence/src/backends/mongodb/bulk_ingest.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "fix(persistence): retry the MongoDB batch resource writes and confirm replays

A transient error on the resources insert or update aborted the whole
batch and the rest of the file (#1001). Retry per chunk / per update,
and on a retry settle a duplicate id or an unmatched version guard by
re-reading the row: ours (same version, batch timestamp, content) is a
success, anything else is still the conflict the per-entry path reported.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01ECbBsNhgXwBo4dQtFyVBXv"
```

---

### Task 6: Retry the pre-read, history, rollback log, search-index delete and receipts

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/bulk_ingest.rs` — `load_existing` (`:221-285`), `write_history` (`:697-712`), `write_search_index` delete (`:750-762`), `write_changes` (`:787-809`), `write_entry_results` (`:812-851`), `run_update_command` (`:866-899`), `insert_documents` (`:902-923`), and their call sites in `ingest_batch`
- Test: `crates/persistence/tests/mongodb_tests.rs` (`mod bulk_submit`)

**Interfaces:**
- Produces: every listed fn gains a trailing `cancel: Option<&CancelToken>` parameter (`load_existing` gains it too); `insert_documents(db, collection, documents, context, cancel)`; `run_update_command(db, collection, statements, context, cancel)`.

- [ ] **Step 1: Write the failing tests**

Add to `mod bulk_submit`:

```rust
    /// Spec §3.2: history and rollback-log inserts whose acknowledgement was
    /// lost find their own rows on retry (both collections have a unique key)
    /// and treat the duplicates as landed. `times: 4` = resources (landed +
    /// error, then dup-key) and history (landed + error, then dup-key).
    #[tokio::test]
    async fn unacknowledged_history_insert_is_not_duplicated() {
        let test = "submit_fp_unacked_history";
        let app = "fp-unacked-history";
        let Some(backend) = create_backend_with_app_name(test, app).await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;
        let Some(fail_point) = FailPoint::enable(
            app,
            doc! {
                "failCommands": ["insert"],
                "writeConcernError": {
                    "code": 91,
                    "errmsg": "Replication is being shut down",
                    "errorLabels": ["RetryableWriteError"],
                },
            },
            doc! { "times": 4 },
        )
        .await
        else {
            return;
        };

        let results = backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                three_patients("hist"),
                &BulkProcessingOptions::new().with_defer_indexing(true),
            )
            .await
            .unwrap();
        fail_point.off().await;

        assert!(results.iter().all(|r| r.is_success()), "{results:?}");
        assert_one_row_each(test, &tenant, &["hist-1", "hist-2", "hist-3"]).await;
    }

    /// Same for the rollback log: `times: 6` reaches the third insert stage
    /// (resources, history, changes — indexing deferred so no search insert).
    #[tokio::test]
    async fn unacknowledged_rollback_log_insert_is_not_duplicated() {
        let test = "submit_fp_unacked_changes";
        let app = "fp-unacked-changes";
        let Some(backend) = create_backend_with_app_name(test, app).await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;
        let Some(fail_point) = FailPoint::enable(
            app,
            doc! {
                "failCommands": ["insert"],
                "writeConcernError": {
                    "code": 91,
                    "errmsg": "Replication is being shut down",
                    "errorLabels": ["RetryableWriteError"],
                },
            },
            doc! { "times": 6 },
        )
        .await
        else {
            return;
        };

        let results = backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                three_patients("chg"),
                &BulkProcessingOptions::new().with_defer_indexing(true),
            )
            .await
            .unwrap();
        fail_point.off().await;

        assert!(results.iter().all(|r| r.is_success()), "{results:?}");
        assert_one_row_each(test, &tenant, &["chg-1", "chg-2", "chg-3"]).await;
    }

    /// The pre-read is a `find`; recovery only (the driver may retry reads).
    /// `times: 3` outlasts any single driver retry and the manifest check.
    #[tokio::test]
    async fn dropped_pre_read_is_retried() {
        let test = "submit_fp_dropped_find";
        let app = "fp-dropped-find";
        let Some(backend) = create_backend_with_app_name(test, app).await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;
        let Some(fail_point) = FailPoint::enable(
            app,
            doc! { "failCommands": ["find"], "closeConnection": true },
            doc! { "times": 3 },
        )
        .await
        else {
            return;
        };

        let results = backend
            .process_entries(&tenant, &id, &manifest_id, three_patients("find"), &BulkProcessingOptions::new())
            .await
            .unwrap();
        fail_point.off().await;

        assert!(results.iter().all(|r| r.is_success()), "{results:?}");
    }

    /// Spec §5.2 case 5: the receipt upsert is a raw `update` command with no
    /// driver retry at all. `times: 3` = two dropped `processing` promotions
    /// plus the first receipt command.
    #[tokio::test]
    async fn dropped_receipt_write_is_retried() {
        let test = "submit_fp_dropped_receipts";
        let app = "fp-dropped-receipts";
        let Some(backend) = create_backend_with_app_name(test, app).await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;
        let Some(fail_point) = FailPoint::enable(
            app,
            doc! { "failCommands": ["update"], "closeConnection": true },
            doc! { "times": 3 },
        )
        .await
        else {
            return;
        };

        let results = backend
            .process_entries(&tenant, &id, &manifest_id, three_patients("rcpt"), &BulkProcessingOptions::new())
            .await
            .unwrap();
        fail_point.off().await;

        assert!(results.iter().all(|r| r.is_success()), "{results:?}");
        let counts = backend.get_entry_counts(&tenant, &id, &manifest_id).await.unwrap();
        assert_eq!(counts.total, 3, "every receipt landed");
        assert_eq!(counts.success, 3);
    }

    /// The search-index delete is a multi-document `delete`, which the driver
    /// never retries. Updates with inline indexing issue it; `times: 2`.
    #[tokio::test]
    async fn dropped_search_index_delete_is_retried() {
        let test = "submit_fp_dropped_delete";
        let app = "fp-dropped-delete";
        let Some(backend) = create_backend_with_app_name(test, app).await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;
        backend
            .process_entries(&tenant, &id, &manifest_id, three_patients("del"), &BulkProcessingOptions::new())
            .await
            .unwrap();

        let Some(fail_point) = FailPoint::enable(
            app,
            doc! { "failCommands": ["delete"], "closeConnection": true },
            doc! { "times": 2 },
        )
        .await
        else {
            return;
        };
        let updates: Vec<NdjsonEntry> = (1..=3)
            .map(|i| {
                NdjsonEntry::new(
                    i,
                    "Patient",
                    json!({"resourceType": "Patient", "id": format!("del-{i}"), "name": [{"family": "Retried"}]}),
                )
            })
            .collect();
        let results = backend
            .process_entries(&tenant, &id, &manifest_id, updates, &BulkProcessingOptions::new())
            .await
            .unwrap();
        fail_point.off().await;

        assert!(results.iter().all(|r| r.is_success()), "{results:?}");
        let expected = search_index_entry_count(&backend, &tenant, "Patient", "del-1").await;
        assert!(expected > 0);
        for i in 2..=3 {
            assert_eq!(
                search_index_entry_count(&backend, &tenant, "Patient", &format!("del-{i}")).await,
                expected,
                "del-{i} indexed exactly once"
            );
        }
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --test mongodb_tests unacknowledged_history_insert -- --nocapture`
Expected: FAIL — `Err(… insert batch resource history …)`.

- [ ] **Step 3: Implement**

`load_existing` (add `cancel: Option<&CancelToken>` as the last parameter; call site in `ingest_batch` passes `options.cancel.as_ref()`). Replace the per-type loop body with:

```rust
        for (resource_type, ids) in ids_by_type {
            let ids: Vec<Bson> = ids.into_iter().map(Bson::from).collect();
            let filter = doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": { "$in": ids },
            };
            let rows: Vec<Document> = or_exhausted(
                "resolve batch ids",
                retry_transient_with(&BULK_INGEST_RETRY, cancel, "resolve batch ids", || async {
                    resources.find(filter.clone()).await?.try_collect().await
                })
                .await,
            )?;

            let now = Utc::now();
            for document in rows {
                let (Ok(id), Ok(version_id)) =
                    (document.get_str("id"), document.get_str("version_id"))
                else {
                    continue;
                };
                let (id, version_id) = (id.to_string(), version_id.to_string());
                let content = match document.get_document("data") {
                    Ok(payload) => document_to_value(payload)?,
                    Err(_) => Value::Null,
                };
                found.insert(
                    (resource_type.to_string(), id),
                    ExistingResource {
                        version_id,
                        content,
                        created_at: extract_created_at(&document, now),
                        fhir_version: extract_fhir_version(
                            &document,
                            FhirVersion::default_enabled(),
                        ),
                        is_deleted: document.get_bool("is_deleted").unwrap_or(false),
                    },
                );
            }
        }
```

`insert_documents`:

```rust
/// Inserts `documents` in chunked, unordered `insert` commands, each chunk its
/// own retry unit.
///
/// On a retry, duplicate-key errors are the chunk's own rows from the attempt
/// whose acknowledgement was lost — every collection this writes has a unique
/// key the batch minted itself — so a chunk that reports only duplicates has
/// landed.
async fn insert_documents(
    db: &Database,
    collection: &str,
    mut documents: Vec<Document>,
    context: &str,
    cancel: Option<&CancelToken>,
) -> StorageResult<()> {
    if documents.is_empty() {
        return Ok(());
    }
    let collection = db.collection::<Document>(collection);
    while !documents.is_empty() {
        let take = documents.len().min(INSERT_DOCS_PER_COMMAND);
        let chunk: Vec<Document> = documents.drain(..take).collect();
        let Attempted { result, attempts } =
            retry_transient_with(&BULK_INGEST_RETRY, cancel, context, || async {
                collection.insert_many(&chunk).ordered(false).await
            })
            .await;
        if let Err(err) = result {
            let only_duplicates = attempts > 1
                && matches!(
                    err.kind.as_ref(),
                    ErrorKind::InsertMany(insert_many)
                        if insert_many
                            .write_errors
                            .as_ref()
                            .is_some_and(|errors| errors.iter().all(|e| e.code == 11000))
                );
            if !only_duplicates {
                return Err(exhausted(context, attempts, &err));
            }
        }
    }
    Ok(())
}
```

`write_history` and `write_changes`: add `cancel: Option<&CancelToken>` as the last parameter and pass it through to `insert_documents`. Update their call sites in `ingest_batch` to pass `options.cancel.as_ref()`.

`write_search_index`, delete loop (only this part changes in this task):

```rust
            for (resource_type, ids) in stale_by_type {
                let filter = doc! {
                    "tenant_id": tenant_id,
                    "resource_type": resource_type,
                    "resource_id": { "$in": ids },
                };
                or_exhausted(
                    "clear batch search index",
                    retry_transient_with(
                        &BULK_INGEST_RETRY,
                        options.cancel.as_ref(),
                        "clear batch search index",
                        || async { collection.delete_many(filter.clone()).await },
                    )
                    .await,
                )?;
            }
```

and its trailing `insert_documents(.., "insert batch search index")` call gains `options.cancel.as_ref()`.

`write_entry_results`: pass `options.cancel.as_ref()` into `run_update_command`. `run_update_command`:

```rust
async fn run_update_command(
    db: &Database,
    collection: &str,
    statements: &[Document],
    context: &str,
    cancel: Option<&CancelToken>,
) -> StorageResult<UpdateOutcome> {
    let command = doc! {
        "update": collection,
        "updates": statements.to_vec(),
        "ordered": false,
    };
    let response = or_exhausted(
        context,
        retry_transient_with(&BULK_INGEST_RETRY, cancel, context, || async {
            db.run_command(command.clone()).await
        })
        .await,
    )?;
    // … the existing writeErrors parsing, unchanged …
}
```

- [ ] **Step 4: Run the tests**

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --test mongodb_tests bulk_submit:: -- --nocapture`
Expected: the five new tests PASS; all existing `bulk_submit::` tests still pass.

- [ ] **Step 5: Gates and commit**

```bash
git add crates/persistence/src/backends/mongodb/bulk_ingest.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "fix(persistence): retry every remaining MongoDB batch command

The pre-read, history and rollback-log inserts, the search-index delete
and the receipt upsert each aborted the batch on one transient error
(#1001). Retry them per command; a history or rollback chunk that finds
only its own duplicates on a retry has landed.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01ECbBsNhgXwBo4dQtFyVBXv"
```

---

### Task 7: Search-index insert with delete-all replay

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/bulk_ingest.rs` (`write_search_index`, the insert phase)
- Test: `crates/persistence/tests/mongodb_tests.rs` (`mod bulk_submit`)

**Interfaces:**
- Consumes: Task 6's `write_search_index` shape.
- Produces: no signature change.

- [ ] **Step 1: Write the failing test**

```rust
    /// Spec §3.2 search index: `search_index` has no unique key, so a replayed
    /// insert would duplicate rows. The retry deletes every batch id's rows
    /// first. `times: 6` with inline indexing = resources (2), history (2),
    /// then the search-index insert lands + errors twice before succeeding.
    #[tokio::test]
    async fn unacknowledged_search_index_insert_is_not_duplicated() {
        let test = "submit_fp_unacked_search";
        let app = "fp-unacked-search";
        let Some(backend) = create_backend_with_app_name(test, app).await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;
        // Control: the same shape ingested without a failpoint.
        backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                vec![NdjsonEntry::new(
                    1,
                    "Patient",
                    json!({"resourceType": "Patient", "id": "control", "name": [{"family": "Indexed"}]}),
                )],
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();
        let expected = search_index_entry_count(&backend, &tenant, "Patient", "control").await;
        assert!(expected > 0);

        let Some(fail_point) = FailPoint::enable(
            app,
            doc! {
                "failCommands": ["insert"],
                "writeConcernError": {
                    "code": 91,
                    "errmsg": "Replication is being shut down",
                    "errorLabels": ["RetryableWriteError"],
                },
            },
            doc! { "times": 6 },
        )
        .await
        else {
            return;
        };
        let entries: Vec<NdjsonEntry> = (1..=3)
            .map(|i| {
                NdjsonEntry::new(
                    i,
                    "Patient",
                    json!({"resourceType": "Patient", "id": format!("sidx-{i}"), "name": [{"family": "Indexed"}]}),
                )
            })
            .collect();
        let results = backend
            .process_entries(&tenant, &id, &manifest_id, entries, &BulkProcessingOptions::new())
            .await
            .unwrap();
        fail_point.off().await;

        assert!(results.iter().all(|r| r.is_success()), "{results:?}");
        for i in 1..=3 {
            assert_eq!(
                search_index_entry_count(&backend, &tenant, "Patient", &format!("sidx-{i}")).await,
                expected,
                "sidx-{i} indexed exactly once"
            );
        }
        assert_one_row_each(test, &tenant, &["sidx-1", "sidx-2", "sidx-3"]).await;
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --test mongodb_tests unacknowledged_search_index -- --nocapture`
Expected: FAIL on the `indexed exactly once` assertion (count is a multiple of `expected`), because Task 6's `insert_documents` treats the search-index insert like the others.

- [ ] **Step 3: Implement**

In `write_search_index`, replace everything after the stale-delete loop with:

```rust
        // Ids of every plan the batch wrote, for the replay delete below.
        let mut written_by_type: HashMap<&str, Vec<Bson>> = HashMap::new();
        let mut documents = Vec::new();
        for (plan_idx, plan) in planned.plans.iter().enumerate() {
            if failed.contains_key(&plan_idx) {
                continue;
            }
            written_by_type
                .entry(plan.resource_type.as_str())
                .or_default()
                .push(Bson::from(plan.id.as_str()));
            documents.extend(self.search_index_documents(
                tenant_id,
                &plan.resource_type,
                &plan.id,
                &plan.content,
            ));
        }
        if documents.is_empty() {
            return Ok(());
        }

        // `search_index` has no unique key, so a replayed insert would
        // duplicate rows. The retry unit is therefore the whole insert phase,
        // and every attempt after the first clears the batch's rows first —
        // creates included, since their rows may have partially landed.
        let mut first = true;
        or_exhausted(
            "insert batch search index",
            retry_transient_with(
                &BULK_INGEST_RETRY,
                options.cancel.as_ref(),
                "insert batch search index",
                || {
                    let replay = !std::mem::replace(&mut first, false);
                    let collection = collection.clone();
                    let written_by_type = &written_by_type;
                    let documents = &documents;
                    async move {
                        if replay {
                            for (resource_type, ids) in written_by_type {
                                collection
                                    .delete_many(doc! {
                                        "tenant_id": tenant_id,
                                        "resource_type": *resource_type,
                                        "resource_id": { "$in": ids.clone() },
                                    })
                                    .await?;
                            }
                        }
                        for chunk in documents.chunks(INSERT_DOCS_PER_COMMAND) {
                            collection.insert_many(chunk).ordered(false).await?;
                        }
                        Ok(())
                    }
                },
            )
            .await,
        )
```

Move `let collection = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);` above the stale-delete block so both phases share it. The stale-delete loop from Task 6 stays as the first-attempt clean-up of updated ids.

- [ ] **Step 4: Run the tests**

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --test mongodb_tests bulk_submit:: -- --nocapture`
Expected: PASS, including `test_defer_indexing_skips_the_search_index_but_stores_everything_else` and `mongodb_integration_standalone_search_writes_search_index` (run the latter separately: filter `standalone_search_writes`).

- [ ] **Step 5: Gates and commit**

```bash
git add crates/persistence/src/backends/mongodb/bulk_ingest.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "fix(persistence): replay the MongoDB batch search-index insert without duplicates

search_index has no unique key, so a retried insert re-deletes the batch's
rows first; the first attempt keeps its stale-only delete (#1001).

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01ECbBsNhgXwBo4dQtFyVBXv"
```

---

### Task 8: Containment — a failed flush becomes per-line receipts

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/bulk_ingest.rs` (`ingest_batch` `:154-215`, `write_entry_results`)
- Test: `crates/persistence/tests/mongodb_tests.rs` (`mod bulk_submit`)

**Interfaces:**
- Produces: `async fn write_batch(&self, db, tenant, submission_id, manifest_id, entries, options) -> StorageResult<PlannedBatch>`; `fn all_failed(entries: &[NdjsonEntry], err: &StorageError) -> Vec<BulkEntryResult>`; `write_entry_results(.., results: &[BulkEntryResult])`.

- [ ] **Step 1: Write the failing tests**

```rust
    fn six_lines(prefix: &str) -> Vec<u8> {
        (1..=6)
            .map(|i| format!("{{\"resourceType\":\"Patient\",\"id\":\"{prefix}-{i}\"}}\n"))
            .collect::<String>()
            .into_bytes()
    }

    fn cursor_reader(bytes: Vec<u8>) -> Box<dyn tokio::io::AsyncBufRead + Send + Unpin> {
        Box::new(tokio::io::BufReader::new(std::io::Cursor::new(bytes)))
    }

    /// Spec §5.2 case 3. `times: 6` is exact: batch 1 is one chunk, so its
    /// resources insert is one `insert` per attempt; the policy allows six;
    /// the standalone server adds no driver retry; and the exhausted error
    /// short-circuits `write_batch` before history or the rollback log issue
    /// any further `insert`. The failpoint is spent exactly when batch 1
    /// gives up, and batch 2's first insert succeeds.
    #[tokio::test]
    async fn exhausted_retries_contain_to_the_batch() {
        let test = "submit_fp_exhausted";
        let app = "fp-exhausted";
        let Some(backend) = create_backend_with_app_name(test, app).await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;
        let Some(fail_point) = FailPoint::enable(
            app,
            doc! { "failCommands": ["insert"], "closeConnection": true },
            doc! { "times": 6 },
        )
        .await
        else {
            return;
        };

        let options = BulkProcessingOptions::new().with_batch_size(3);
        let result = backend
            .process_ndjson_stream(&tenant, &id, &manifest_id, "Patient", cursor_reader(six_lines("ex")), &options)
            .await
            .unwrap();
        fail_point.off().await;

        assert!(!result.aborted, "{result:?}");
        assert_eq!(result.counts.processing_error, 3);
        assert_eq!(result.counts.success, 3);
        assert_eq!(result.lines_processed, 6, "the file was read to the end");

        let results = backend
            .get_entry_results(&tenant, &id, &manifest_id, None, None)
            .await
            .unwrap();
        let mut by_line: Vec<_> = results.iter().collect();
        by_line.sort_by_key(|r| r.line_number);
        for r in &by_line[..3] {
            assert_eq!(r.outcome, BulkEntryOutcome::ProcessingError, "{r:?}");
            let issue = &r.operation_outcome.as_ref().unwrap()["issue"][0];
            assert_eq!(issue["code"], "transient");
            let diagnostics = issue["diagnostics"].as_str().unwrap();
            assert!(diagnostics.contains("(after 6 attempts)"), "{diagnostics}");
            assert!(diagnostics.contains("re-ingesting this file"), "{diagnostics}");
        }
        for r in &by_line[3..] {
            assert_eq!(r.outcome, BulkEntryOutcome::Success, "{r:?}");
        }
        assert!(backend.read(&tenant, "Patient", "ex-1").await.unwrap().is_none());
        assert!(backend.read(&tenant, "Patient", "ex-6").await.unwrap().is_some());

        let manifest = backend.get_manifest(&tenant, &id, &manifest_id).await.unwrap().unwrap();
        assert_eq!(manifest.total_entries, 6);
        assert_eq!(manifest.failed_entries, 3);
        assert_eq!(manifest.processed_entries, 3);
    }

    /// Spec §5.2 case 4: with strict options the stream aborts on the failed
    /// batch and never attempts the next one.
    #[tokio::test]
    async fn max_errors_now_sees_backend_failures() {
        let test = "submit_fp_max_errors";
        let app = "fp-max-errors";
        let Some(backend) = create_backend_with_app_name(test, app).await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;
        let Some(fail_point) = FailPoint::enable(
            app,
            doc! { "failCommands": ["insert"], "closeConnection": true },
            doc! { "times": 6 },
        )
        .await
        else {
            return;
        };

        let options = BulkProcessingOptions {
            continue_on_error: false,
            max_errors: 3,
            ..BulkProcessingOptions::new().with_batch_size(3)
        };
        let result = backend
            .process_ndjson_stream(&tenant, &id, &manifest_id, "Patient", cursor_reader(six_lines("me")), &options)
            .await
            .unwrap();
        fail_point.off().await;

        assert!(result.aborted);
        assert_eq!(result.abort_reason.as_deref(), Some("max errors exceeded"));
        assert_eq!(result.counts.processing_error, 3);
        assert!(backend.read(&tenant, "Patient", "me-4").await.unwrap().is_none(), "batch 2 never ran");
    }

    /// Spec §5.2 case 6. This proves only that cancellation ends the batch well
    /// before the 2.5 s backoff budget is spent, not which backoff step it
    /// lands in: 150 ms is after attempt 1 fails and inside the first sleep.
    #[tokio::test]
    async fn cancel_during_backoff_returns_promptly() {
        let test = "submit_fp_cancel_backoff";
        let app = "fp-cancel-backoff";
        let Some(backend) = create_backend_with_app_name(test, app).await else {
            return;
        };
        let backend = std::sync::Arc::new(backend);
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;
        let Some(fail_point) = FailPoint::enable(
            app,
            doc! { "failCommands": ["insert"], "closeConnection": true },
            doc! { "times": 100 },
        )
        .await
        else {
            return;
        };

        let cancel = CancelToken::new();
        let options = BulkProcessingOptions::new()
            .with_batch_size(3)
            .with_cancel(cancel.clone());
        let started = std::time::Instant::now();
        let run = {
            let backend = backend.clone();
            let tenant = tenant.clone();
            let id = id.clone();
            let manifest_id = manifest_id.clone();
            tokio::spawn(async move {
                backend
                    .process_ndjson_stream(&tenant, &id, &manifest_id, "Patient", cursor_reader(six_lines("cb")), &options)
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(150)).await;
        cancel.cancel();
        let result = run.await.unwrap().unwrap();
        let elapsed = started.elapsed();
        fail_point.off().await;

        assert!(elapsed < Duration::from_secs(1), "took {elapsed:?}");
        assert!(result.aborted);
        assert_eq!(result.abort_reason.as_deref(), Some(CANCELLED_ABORT_REASON));
        assert_eq!(result.counts.processing_error, 3, "the batch in flight was recorded before the cancel check");
        let counts = backend.get_entry_counts(&tenant, &id, &manifest_id).await.unwrap();
        assert_eq!(counts.processing_error, 3);
    }
```

Check the names used above exist: `backend.get_entry_results(tenant, submission, manifest, outcome_filter, page)` and `BulkEntryOutcome` — grep `fn get_entry_results` in `crates/persistence/src/core/bulk_submit.rs` and use its actual parameters; add `BulkEntryOutcome` to the module's `use helios_persistence::core::{..}` list. `TenantContext: Clone` — if it is not, build a second `create_tenant("submit-tenant")` inside the spawned task instead of cloning.

- [ ] **Step 2: Run them to verify they fail**

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --test mongodb_tests exhausted_retries_contain -- --nocapture`
Expected: FAIL — `process_ndjson_stream` returns `Err(..)` (the `unwrap()` panics with `backend unavailable`).

- [ ] **Step 3: Implement**

Replace `ingest_batch` (from its doc comment through its closing brace) with:

```rust
    /// Ingests one batch of NDJSON entries in a fixed number of commands.
    ///
    /// See the [module docs](self) for the write order, the retry rules and
    /// why the batch is not one transaction.
    ///
    /// A flush that fails after its retries does not fail the batch: every
    /// entry gets a `processing-error` receipt naming the stage and the error,
    /// the manifest's counters charge them, and the file continues with its
    /// next batch. Only the receipt write itself still propagates — with it
    /// gone there is nothing left to record into.
    pub(super) async fn ingest_batch(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entries: &[NdjsonEntry],
        options: &BulkProcessingOptions,
    ) -> StorageResult<BatchOutcome> {
        let db = self.get_database().await?;

        let (results, error_count, aborted_on_max_errors, touched_search_parameters) =
            match self
                .write_batch(&db, tenant, submission_id, manifest_id, entries, options)
                .await
            {
                Ok(planned) => (
                    planned.results,
                    planned.error_count,
                    planned.aborted_on_max_errors,
                    planned.touched_search_parameters,
                ),
                Err(err) => {
                    tracing::warn!(
                        manifest_id,
                        entries = entries.len(),
                        "batch flush failed; recording every entry as processing-error: {err}"
                    );
                    (all_failed(entries, &err), entries.len() as u32, false, false)
                }
            };

        self.write_entry_results(&db, tenant, submission_id, manifest_id, options, &results)
            .await?;

        // A SearchParameter write may change a tenant's overlay. The per-entry
        // path reloaded the cache once per such resource; once per batch is the
        // same invalidation for a fraction of the reloads.
        if touched_search_parameters
            && let Err(e) = self.reload_stored_cache().await
        {
            tracing::warn!("SearchParameter cache reload failed: {e}");
        }

        Ok(BatchOutcome {
            results,
            error_count,
            aborted_on_max_errors,
        })
    }

    /// Everything from the pre-read through the rollback log. Fails as a whole
    /// when a stage exhausts its retries; [`Self::ingest_batch`] turns that
    /// into receipts.
    async fn write_batch(
        &self,
        db: &Database,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entries: &[NdjsonEntry],
        options: &BulkProcessingOptions,
    ) -> StorageResult<PlannedBatch> {
        let tenant_id = tenant.tenant_id().as_str();
        let cancel = options.cancel.as_ref();

        let existing = self.load_existing(db, tenant_id, entries, cancel).await?;
        let mut planned = self.plan_batch(tenant, manifest_id, entries, &existing, options)?;

        let failed = self.write_resources(db, tenant_id, &planned, cancel).await?;
        self.write_history(db, &mut planned, &failed, cancel).await?;
        self.write_search_index(db, tenant_id, &planned, &failed, options)
            .await?;

        // A resource the batch could not write takes its entry's result down
        // with it: the per-entry path reported the same failure per entry, and
        // its history, index, rollback and receipt rows were never written.
        for (plan_idx, result_idx, _) in &planned.changes {
            if let Some(diagnostics) = failed.get(plan_idx) {
                let plan = &planned.plans[*plan_idx];
                planned.results[*result_idx] = BulkEntryResult::processing_error(
                    planned.results[*result_idx].line_number,
                    &plan.resource_type,
                    serde_json::json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{
                            "severity": "error",
                            "code": "exception",
                            "diagnostics": diagnostics
                        }]
                    }),
                );
                planned.error_count += 1;
            }
        }

        self.write_changes(tenant, submission_id, db, &planned, &failed, cancel)
            .await?;
        Ok(planned)
    }
```

If the borrow checker rejects mutating `planned.results` / `planned.error_count` while iterating `&planned.changes`, collect first: `let attributed: Vec<(usize, usize)> = planned.changes.iter().map(|(p, r, _)| (*p, *r)).collect();` and iterate that.

Add next to `row_matches_plan`:

```rust
/// One `processing-error` per entry of a batch whose flush failed. `transient`
/// (FHIR issue-type: the sender may resubmit) when the stage outlived its
/// retries on a transient error, `exception` otherwise.
fn all_failed(entries: &[NdjsonEntry], err: &StorageError) -> Vec<BulkEntryResult> {
    let code = if matches!(err, StorageError::Backend(BackendError::Unavailable { .. })) {
        "transient"
    } else {
        "exception"
    };
    entries
        .iter()
        .map(|entry| {
            BulkEntryResult::processing_error(
                entry.line_number,
                &entry.resource_type,
                serde_json::json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{
                        "severity": "error",
                        "code": code,
                        "diagnostics": format!("{err}; re-ingesting this file will retry the entry"),
                    }]
                }),
            )
        })
        .collect()
}
```

Add `BackendError` to the `crate::error` import. Change `write_entry_results`'s last parameter from `planned: &PlannedBatch` to `results: &[BulkEntryResult]` and its body to iterate `results` (the early return checks `results.is_empty()`).

- [ ] **Step 4: Run the tests**

Run: `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --test mongodb_tests bulk_submit:: -- --nocapture`
Expected: the three new tests PASS; every existing `bulk_submit::` test passes — `test_two_batches_racing_for_the_same_ids_stay_consistent` asserts `failed_entries >= 1`, which is what catches `write_batch` returning an incomplete `error_count`.

- [ ] **Step 5: Gates and commit**

```bash
git add crates/persistence/src/backends/mongodb/bulk_ingest.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "fix(persistence): contain a failed MongoDB batch flush to per-line receipts

One exhausted retry aborted the rest of the NDJSON file with a single
file-level error and no per-resource record (#1001). Record every entry
of the batch as processing-error (code transient when the error was)
and let the stream continue, so max_errors and continue_on_error now
govern backend failures like any other.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01ECbBsNhgXwBo4dQtFyVBXv"
```

---

### Task 9: Documentation and full gates

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/bulk_ingest.rs:20-35` (module doc, *Durability shape*)
- Modify: `.claude/skills/bulk-data-submit/SKILL.md` (the MongoDB bullet under *Behavior Notes*, the one beginning "MongoDB ingests a batch, not an entry")
- Check: `crates/rest/README.md` (`grep -n processing-error`; edit only if it describes receipt outcomes)

- [ ] **Step 1: Rewrite the module doc's *Durability shape* section**

Replace the section from `//! # Durability shape` through the line ending `interrupted batch is re-processed rather than falsely reported done.` with:

```rust
//! # Durability shape
//!
//! The flush is a sequence of independent commands, not one transaction. That is
//! deliberate:
//!
//! * The path it replaces was not atomic across a batch either. It opened a
//!   *best-effort* per-resource transaction — none at all on a standalone
//!   `mongod` — and wrote the rollback log and the receipt outside it.
//! * Multi-document transactions need a replica set, and a 1 000-entry batch is
//!   tens of thousands of documents — past what one transaction should carry.
//!
//! What keeps a transient error from losing work is retry plus replay rules,
//! not atomicity (#1001). Every command runs through the bounded, cancel-aware
//! retry in [`super::retry`], and a retry never duplicates what an earlier
//! attempt landed:
//!
//! * `resources`: a duplicate id or an unmatched version guard on a retry is
//!   settled by re-reading the row — ours (same version, the batch's own
//!   `last_updated`, same content) is a success, anything else is still the
//!   conflict the per-entry path reported.
//! * `resource_history` and the rollback log: both have a unique key the batch
//!   minted itself, so a chunk that reports only duplicates on a retry has
//!   landed.
//! * `search_index` has no unique key, so a retry clears the batch's rows
//!   before re-inserting.
//! * Receipts are upserts keyed by `(manifest, file_url, line)`.
//!
//! When a stage outlives its retries the batch does not take the file with it:
//! every entry gets a `processing-error` receipt (issue code `transient` when
//! the error was) and the next batch runs. Re-ingesting the file converges —
//! resources upsert by id and version guard, history and rollback rows dedupe
//! on their unique indexes. The commands are ordered so that is always true:
//! resources, then history, then the derived search index, then the rollback
//! log, then the receipts, which land last so an interrupted batch is
//! re-processed rather than falsely reported done.
```

- [ ] **Step 2: Update the skill doc**

In `.claude/skills/bulk-data-submit/SKILL.md`, replace the sentence in the MongoDB bullet beginning "The batch flush is a sequence of commands rather than one transaction, on purpose:" through "…re-processed rather than falsely reported done." with:

```
The batch flush is a sequence of commands rather than one transaction. Every command is retried on a transient driver error (`RetryableError`/`RetryableWriteError` label, I/O error, cleared pool — not a server-selection timeout) with 100 ms doubling backoff capped at 1 s over six attempts, checking the submission's cancel token before each sleep; a retry never duplicates what an earlier attempt landed (resources are re-read and matched on version + the batch's own `last_updated` + content, history and rollback rows dedupe on their unique keys, the search index is cleared before re-insert). When a stage outlives its retries the batch's entries get `processing-error` receipts with issue code `transient` and the file continues with its next batch, so `max_errors`/`continue_on_error` govern backend failures too (#1001); re-submitting the file converges. Only a receipt write that itself fails after retries still aborts the file. The manifest counters are a `$inc` and may over-count one batch if a retried attempt had actually landed — the receipts are authoritative.
```

- [ ] **Step 3: Full gates**

Run, in order:
- `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo fmt --all -- --check`
- `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo clippy -p helios-persistence --features mongodb --tests -- -D warnings`
- `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo check --workspace --all-features`
- `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --lib`
- `CARGO_TARGET_DIR=C:/Users/DougC/Code/Helios/hfs/target cargo test -p helios-persistence --features mongodb --test mongodb_tests -- --nocapture 2>&1 | tee /tmp/mongodb_tests.log` then `grep -c Skipping /tmp/mongodb_tests.log` must be `0` and the summary line must show `0 failed`.

Expected: all clean/green.

- [ ] **Step 4: Commit**

```bash
git add crates/persistence/src/backends/mongodb/bulk_ingest.rs .claude/skills/bulk-data-submit/SKILL.md
git commit -m "docs(persistence): describe the MongoDB bulk-submit retry and containment rules

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01ECbBsNhgXwBo4dQtFyVBXv"
```
