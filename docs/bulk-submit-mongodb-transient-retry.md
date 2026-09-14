# Bulk submit on MongoDB: transient-error retry and batch-failure containment

Design for the retry half of #1001. Written 2026-09-12 against `main` at
`2fa9dfaee`. The "completed with N unrecoverable entries" half of #1001 is a
separate change and is out of scope here (see §7).

## 1. Problem, on current `main`

#1001 was filed against the per-entry MongoDB ingest path. PR #1024 replaced
that path with the batched one in
`crates/persistence/src/backends/mongodb/bulk_ingest.rs`, which changes the
bug's shape but not its substance:

- **Nothing retries a transient driver error.** A working, bounded retry helper
  exists (`retry_transient` / `is_transient_mongo_error`,
  `backends/mongodb/user_settings.rs:375-427`) but is module-private and unused
  by the bulk path. The driver's own `retryWrites` (default on) retries
  `insert_many` and `update_one` once on a replica set, never `delete_many`
  (multi) or the raw `run_command` used for receipts, and never on a
  standalone server.
- **One transient error now loses the rest of the file, not one entry.** Only
  two sites attribute a failure to a single entry: per-document `write_errors`
  on the create-side `insert_many` (`bulk_ingest.rs:599-629`) and the per-future
  `update_one` errors (`:686-688`). Every other fallible command —
  `load_existing`'s `find` (`:251,257`), a transport-level `insert_many` error
  (`:630-635`), `delete_many` (`:759-761`), the shared `insert_documents`
  (`:916-920`) and the receipt `run_command` (`:880`) — is wrapped with
  `internal_error(..)?`. That aborts `ingest_batch`, then `process_entries`
  (`mongodb/bulk_submit.rs:870`), then `process_ndjson_stream`'s read loop
  (`:1088` / `:1111`), so every remaining line of that NDJSON file is never
  read. The worker records that as one file-level `OperationOutcome` and
  `failed += 1` (`core/bulk_submit_worker.rs:1302-1318`), so the dropped
  resources get no per-line receipt at all.
- `max_errors` / `continue_on_error` never see a whole-batch `Err`; they only
  govern per-entry results a backend returns inside `Ok`.
- The `TransientTransactionError` label in the issue cannot occur any more:
  `ingest_batch` opens no session. The same pool-cleared / I/O timeout now
  surfaces as `ErrorKind::Io` or `ErrorKind::ConnectionPoolCleared`, possibly
  labelled `RetryableWriteError`.

## 2. Goals and non-goals

Goals:

1. A transient driver error during a batch flush is retried with bounded
   backoff, per command, and a retry never duplicates or double-versions what
   an earlier attempt already landed.
2. When retries are exhausted, or the error is not transient, the batch's
   entries get per-line `processing-error` receipts and ingest of the file
   continues with the next batch. `max_errors` / `continue_on_error` then
   govern backend failures the same way they govern validation failures.
3. The happy path keeps its current round-trip count.

Non-goals:

- A new terminal manifest status or status-manifest field for partial results
  (§7).
- Retry on the PostgreSQL, SQLite or S3 backends. PostgreSQL and SQLite already
  isolate per entry inside a batch transaction (`postgres/bulk_submit.rs:740-767`,
  `sqlite/bulk_submit.rs:809-836`); S3 catches every entry error inline
  (`s3/bulk_submit.rs:380-390`).
- Tuning the connection pool or the timeouts that cause the transient errors.
- The rollback log's non-idempotence across a *manifest re-walk* after lease
  reclaim (`change_id` is minted per plan; a re-walk plans afresh). This design
  only guarantees idempotence across retries *within one flush*, where the
  plan — and its `change_id`s — is reused.

## 3. Design

### 3.1 A shared retry helper: `backends/mongodb/retry.rs`

Move `is_transient_mongo_error`, `retry_transient` and `MAX_TRANSIENT_RETRIES`
(`user_settings.rs:375-427`) out of `user_settings.rs` into a new sibling module `retry.rs`
(`pub(super)` items; `mod retry;` in `backends/mongodb/mod.rs`).

```rust
pub(super) struct RetryPolicy {
    pub max_attempts: u32,   // total attempts including the first
    pub base: Duration,      // sleep before attempt 2; doubles each retry
    pub cap: Duration,       // sleep never exceeds this
}
pub(super) const SETTINGS_RETRY: RetryPolicy    // 4 attempts, 25 ms, cap 100 ms — sleeps 25, 50, 100 (unchanged behaviour)
pub(super) const BULK_INGEST_RETRY: RetryPolicy // 6 attempts, 100 ms, cap 1 s   — sleeps 100, 200, 400, 800, 1000 (2.5 s total)

pub(super) struct Attempted<T> { pub result: Result<T, MongoError>, pub attempts: u32 }

pub(super) async fn retry_transient_with<T, F, Fut>(
    policy: &RetryPolicy,
    cancel: Option<&CancelToken>,
    what: &str,                // logged on every retry
    op: F,
) -> Attempted<T>
where F: FnMut() -> Fut, Fut: Future<Output = Result<T, MongoError>>;
```

Semantics of `retry_transient_with`:

- Attempt `op` until it succeeds, until it fails with a non-transient error,
  or until `attempts == policy.max_attempts`. Only
  `is_transient_mongo_error(&err)` failures are retried.
- Before each sleep: if `cancel.is_some_and(|c| c.is_cancelled())`, return the
  current error immediately without sleeping. A cancelled submission must not
  pay the backoff budget (#968 keeps abort at "one keeper tick plus one batch").
- Sleep uses `tokio::time::sleep`, so tests run under `start_paused`.
- Each retry logs at `warn` with `what`, `attempt`, `backoff_ms`, and the
  error, matching `sqlite/bulk_submit.rs:137-144`.
- `attempts` in the returned `Attempted` is the number of times `op` ran.

`is_transient_mongo_error` is unchanged: a `RetryableError` /
`RetryableWriteError` label, or `ErrorKind::Io` / `ErrorKind::ConnectionPoolCleared`;
`ServerSelection` is deliberately excluded (its doc comment explains why and
moves with it). Note the driver copies the original labels onto the
`ErrorKind::InsertMany` wrapper (`mongodb-3.7.0/src/action/insert_many.rs:135-181`),
so an `insert_many` that landed its documents and then reported a retryable
write-concern error is classified transient by the same check — that is the
"landed but acknowledgement lost" case §3.2 has to survive.

The existing `retry_transient(op)` keeps its signature as a thin wrapper
(`SETTINGS_RETRY`, no cancel, returns `.result`) so the eight `user_settings.rs`
call sites do not change.

### 3.2 Retry at every command in `bulk_ingest.rs`, with replay rules

Every driver command in the flush runs through `retry_transient_with(
&BULK_INGEST_RETRY, options.cancel.as_ref(), <context>, ..)`. The retry unit
and the replay rule differ per stage because what a second attempt can safely
do depends on the collection's unique index:

| Stage | Command | Retry unit | Replay rule on attempt ≥ 2 |
|---|---|---|---|
| `load_existing` | `find` + cursor drain | the whole find-and-collect | none needed (pure) |
| `write_resources` creates | `insert_many` (unordered) per 5 000-doc chunk | one chunk | dup-key (`11000`) write errors → `confirm_landed` (§3.3); landed ⇒ success, else `AlreadyExists` as today |
| `write_resources` updates | `update_one` (version-guarded) | one `update_one` | `matched_count == 0` → `confirm_landed`; landed ⇒ success, else `VersionConflict` as today |
| `write_history` | `insert_many` per chunk | one chunk | dup-key write errors ⇒ those documents already landed ⇒ success (`idx_history_identity` is unique on the version this batch minted) |
| `write_search_index` delete | `delete_many` per type | one command | none needed (idempotent) |
| `write_search_index` insert | `insert_many` per chunk | **delete-all + insert-all** (see below) | replay re-deletes first (`search_index` has no unique index) |
| `write_changes` | `insert_many` per chunk | one chunk | dup-key write errors ⇒ already landed ⇒ success (`idx_bulk_changes_id` is unique on `change_id`, minted at plan time) |
| `write_entry_results` | `run_command` `update` per 500-statement chunk | one command | none needed (upsert keyed by `(manifest, file_url, line)`) |

Details:

- **Attempt 1 keeps today's semantics everywhere.** A dup-key on the *first*
  create attempt is still the race the pre-read could not see and still reports
  `AlreadyExists`; `matched_count == 0` on the first update attempt is still
  `VersionConflict`; a dup-key on the first history/changes attempt is still a
  hard error. The replay rules apply only when `attempts > 1`, because only
  then can the row have been written by *this* batch's earlier attempt.
- **The retry wraps one loop iteration, never the loop.** In every chunked
  stage the closure passed to `retry_transient_with` captures only that
  iteration's chunk (and, for creates, its `offset`); the surrounding
  `for chunk in ..` loop is outside the retry. A chunk that returned `Ok` is
  never resent, and each chunk's `attempts` is independent of every other
  chunk's.
- **Chunks are borrowed, not moved.** `insert_documents` today drains a chunk
  out of the owned `Vec<Document>` and moves it into `insert_many(chunk)`
  (`bulk_ingest.rs:915-919`), so a second attempt would have nothing to
  resend. Build each chunk once as an owned `Vec<Document>` *outside* the
  retry closure and call `insert_many(&chunk)` inside it — `insert_many` takes
  `impl IntoIterator<Item = impl Borrow<T>>`, so a `&Vec<Document>` works
  without cloning. The `drain` happens once per chunk, before its retry loop.
- **`update_one` takes its `Document`s by value**, so the retried closure
  clones `filter` and `update` per call
  (`|| { let f = filter.clone(); let u = update.clone(); async move { collection.update_one(f, u).await } }`),
  captured by the same outer `async move` per future as today. The clone only
  runs on the retry path; do not introduce `Arc` for this.
- **`load_existing`** currently accumulates every type's cursor into one shared
  `found` map with `internal_error(..)?` at the `find` and each `try_next`
  (`bulk_ingest.rs:241-257`). Restructure so the per-type closure builds and
  returns its own owned map from a fresh `find` + full drain each attempt,
  returning the raw `MongoError`; merge into `found` only from the returned
  `Attempted.result`, and map exhaustion once with `exhausted("resolve batch ids", ..)`.
- **Create path (bespoke, not `insert_documents`).** `write_resources`' create
  loop keeps its own per-`write_error` attribution (`bulk_ingest.rs:608-628`).
  It becomes two-phase so no borrow of the driver error is held across an
  `.await`: first, iterate `write_errors` and partition into owned
  `(plan_idx, &ResourcePlan)` groups — `needs_confirm` (code `11000` and
  `attempts > 1`) and `immediate` (everything else, handled exactly as today:
  `AlreadyExists` for `11000`, the message otherwise); then, after the `match`
  ends, `.await` a single `confirm_landed` for `needs_confirm` and insert
  `AlreadyExists` into `failed` only for the indices it did **not** return.
- **`insert_documents` (history, rollback log)** gains one `attempts`-aware
  branch: when the result is `ErrorKind::InsertMany` whose `write_errors` are
  *all* code `11000` and `attempts > 1`, return `Ok(())`. Any other write
  error, or an `InsertMany` error with `write_errors: None` that is not
  transient, stays a hard error as today. (An `InsertMany` error with
  `write_errors: None` that *is* transient — a retryable write-concern error —
  is retried like any transient error; its documents landed, so the retry
  takes the dup-key path.)
- **Search index (bespoke orchestration).** `write_search_index` does not use
  `insert_documents`' per-chunk retry. Its insert phase is one retried unit:
  `retry_transient_with(&BULK_INGEST_RETRY, cancel, "insert batch search index", op)`
  where `op` is an `FnMut` closure with a captured `first: bool` flag —
  on the first call it deletes only `stale_by_type` (updates, as today) and
  inserts every chunk with no inner retry; on every later call it first
  deletes the rows of **every plan not in `failed`, creates included**
  (`resource_id $in [..]` per type — a second grouping distinct from
  `stale_by_type`, because a create's rows may have partially landed), then
  inserts every chunk from the start. This is the only stage whose retry unit
  is larger than one command, because `search_index` has no unique key to make
  a replayed insert converge. It is unreachable under the default
  `HFS_BULK_SUBMIT_DEFER_INDEXING=true`, where the stage is a no-op
  (`bulk_ingest.rs:733-735`).
- **Updates keep per-entry attribution.** An `update_one` that still fails
  after retries goes into `failed` for that one plan (as today), with the
  attempt count in the diagnostics.
- **Receipts** are retried blindly; the per-statement `writeErrors` check after
  a successful command is unchanged.

#### Manifest bookkeeping in `process_entries` (`mongodb/bulk_submit.rs`)

The same failure class hits the four commands `process_entries` issues around
`ingest_batch`, and today each of them aborts the file the same way
(`sqlite`'s `retry_bookkeeping_on_busy` retries exactly this class on that
backend). Wrap all four with `retry_transient_with(&BULK_INGEST_RETRY, cancel, ..)`:

| Write | `bulk_submit.rs` | Replay safety |
|---|---|---|
| `get_manifest` existence check | `:831-833` | pure |
| mark manifest `processing` | `:858-864` | idempotent `$set`, guarded on `pending`/`processing` |
| counters `$inc` (`total_entries` …) | `:886-897` | **not idempotent**: a landed-but-unacknowledged attempt double-counts one batch on retry. Accepted — the counters already over-count on a manifest re-walk after lease reclaim, and the receipts (`bulk_entry_results`) remain authoritative. Deriving the counters from receipts, as PostgreSQL does (`postgres/bulk_submit.rs:202`), is a follow-up (§7). |
| `touch_submission` | `:310-324` | idempotent `$set updated_at` |

Exhaustion here still returns `Err` from `process_entries` (there is no
per-entry receipt that could describe "the manifest row could not be
updated"), so the worker's file-level branch applies; the retry just makes
that far rarer.

### 3.3 `confirm_landed`

```rust
async fn confirm_landed(&self, db: &Database, tenant_id: &str, plans: &[(usize, &ResourcePlan)])
    -> StorageResult<HashSet<usize>>
```

One `find` on `resources` per resource type present in `plans`, filtered
`tenant_id` + `resource_type` + `id $in [..]`, projecting `id`, `version_id`,
`last_updated`, `data`. A plan index is in the returned set iff a row exists
with all three of:

1. `version_id == plan.version`,
2. `last_updated == chrono_to_bson(plan.last_updated)` — the batch's own
   timestamp, minted once in `plan_batch` (`bulk_ingest.rs:315`, `Utc::now()`)
   and written by both the create insert and the update `$set`. This is what
   makes a positive match attributable to *this* batch: a second attempt of the
   same plan reproduces it, an unrelated concurrent writer's identical-content
   write carries its own `now`,
3. `data == value_to_document(&plan.content)` (BSON `Document` equality).

A row that matches on version but not on timestamp or content is the race the
per-entry path reported, and still reports it (`AlreadyExists` /
`VersionConflict`). The `find` itself runs through the retry helper.

Residual, accepted: BSON datetimes are millisecond precision, so two writers
creating the same `(tenant, type, id)` with byte-identical content, the same
version and the same millisecond would both be treated as landed; the second
would then own a rollback-log record for a resource it did not write. Today
that same collision at attempt 1 reports `AlreadyExists`. The window is one
millisecond inside a retry that already required a transient failure; it is
documented in the `confirm_landed` doc comment rather than closed.

This is called only from the replay branches in §3.2, so the happy path pays
nothing for it.

### 3.4 Containment: `ingest_batch` never fails a batch on a flush error

Split the current `ingest_batch` body:

- `write_batch(..) -> StorageResult<PlannedBatch>` — everything from
  `load_existing` through `write_changes`, i.e. today's `ingest_batch` minus the
  receipt write and the `SearchParameter` cache reload.
- `ingest_batch(..)` becomes:

```rust
let (results, error_count, aborted_on_max_errors, touched_search_parameters) =
    match self.write_batch(..).await {
        Ok(planned) => (
            planned.results,
            planned.error_count,
            planned.aborted_on_max_errors,
            planned.touched_search_parameters,
        ),
        Err(err) => {
            tracing::warn!(manifest_id, entries = entries.len(),
                "batch flush failed; recording every entry as processing-error: {err}");
            (all_failed(entries, &err), entries.len() as u32, false, false)
        }
    };
self.write_entry_results(&db, tenant, submission_id, manifest_id, options, &results).await?;
if touched_search_parameters && let Err(e) = self.reload_stored_cache().await {
    tracing::warn!("SearchParameter cache reload failed: {e}");
}
Ok(BatchOutcome { results, error_count, aborted_on_max_errors })
```

- **`write_batch` must return a complete `error_count`.** Today the
  post-write attribution loop (`bulk_ingest.rs:174-194`) copies
  `planned.error_count` into a local shadow variable and increments only that
  copy, which `ingest_batch` then returns. In `write_batch` that loop
  increments `planned.error_count` (the struct field) directly, so the
  returned `PlannedBatch` already counts both planning-time and write-stage
  failures. The existing `test_two_batches_racing_for_the_same_ids_stay_consistent`
  asserts `failed_entries >= 1` after a write-stage race and would catch the
  omission.
- `all_failed(entries, err)` builds one `BulkEntryResult::processing_error(
  entry.line_number, &entry.resource_type, outcome)` per entry in `entries` —
  every entry the batch was asked to ingest, regardless of what `plan_batch`
  would have said about it (a validation error the batch never got to record is
  re-evaluated when the file is re-ingested).
- `write_entry_results` changes its last parameter from `&PlannedBatch` to
  `&[BulkEntryResult]`.
- The receipt write is the one stage whose failure still propagates: if it
  fails after retries there is nothing left to record into, and the file-level
  handling in the worker (`bulk_submit_worker.rs:1302-1318`) applies as today.
- `process_entries` (`mongodb/bulk_submit.rs:823-901`) is unchanged: its
  existing `$inc` of `total_entries` / `processed_entries` / `failed_entries`
  already does the right thing for an all-failed batch, and
  `process_ndjson_stream` is unchanged, so the `max_errors` /
  `continue_on_error` check after each batch now sees backend failures.
- A batch that fails before `plan_batch` ran (`load_existing`) has written
  nothing. A batch that fails at a later stage may have landed resources and
  history for some entries; their receipts still say `processing-error`,
  because the entry's ingest — rollback record, receipt, index — did not
  complete. Re-ingesting the file converges: resources upsert by id and
  version guard, history and rollback rows dedupe on their unique indexes.
  The receipt's diagnostics say so (§3.5).

### 3.5 Error classification and the receipt's `OperationOutcome`

When a stage exhausts its retries, the `Attempted` error is mapped with a
helper in `retry.rs`:

```rust
pub(super) fn exhausted<T>(context: &str, attempted: Attempted<T>) -> StorageResult<T>
```

- transient (`is_transient_mongo_error`) ⇒
  `StorageError::Backend(BackendError::Unavailable { backend_name: "mongodb", message, source: None })`
- otherwise ⇒ `BackendError::Internal { .. }` (what `internal_error` builds today)
- `message` = `"{context}: {driver error}"` when `attempts == 1`, else
  `"{context}: {driver error} (after {attempts} attempts)"`.

`all_failed` maps that into the receipt:

```json
{ "resourceType": "OperationOutcome", "issue": [{
    "severity": "error",
    "code": "transient",
    "diagnostics": "<err.to_string()>; re-ingesting this file will retry the entry"
}] }
```

`code` is `transient` (FHIR issue-type: "the system receiving the message may
be able to resubmit the same content") when the mapped error is
`BackendError::Unavailable`, and `exception` (today's value) otherwise. The
per-entry `failed` diagnostics for a create/update that failed after retries
(§3.2, updates) use `exception` with the same `(after N attempts)` suffix.

### 3.6 `app_name` on `MongoBackendConfig`

Add `pub app_name: String` (default `"helios-persistence"`, the value
`backend.rs:47` hard-codes today) and use it in `connect_client`. No env
variable. This exists so an integration test can scope a server failpoint to
one client (§5); it is otherwise inert.

`crates/hfs/src/main.rs:204-214` (`build_mongodb_config_with_env`) builds
`MongoBackendConfig` with an exhaustive struct literal and no
`..Default::default()`, unlike every other construction site in the workspace,
so it must gain `app_name: MongoBackendConfig::default().app_name` or the
workspace check fails with E0063.

### 3.7 Documentation

- Rewrite the *Durability shape* section of the `bulk_ingest.rs` module doc:
  drop the sentence claiming the no-transaction design limits a transient
  error to one entry (it did not — `?` propagation lost the batch and the rest
  of the file), and describe the retry policy, the per-stage replay rules, and
  containment instead.
- `.claude/skills/bulk-data-submit/SKILL.md`, MongoDB paragraph under
  *Behavior Notes*: replace the "one transient error into a whole batch of lost
  entries" sentence with the new behaviour — retry budget, `transient` receipt
  code, "re-ingest converges", and that `max_errors` now covers backend
  failures.
- `crates/rest/README.md` only if it describes bulk-submit receipt outcomes
  (grep `processing-error`); otherwise untouched.

## 4. Behavioural consequences worth stating

- **Worst-case added latency per failing batch** is 2.5 s of sleep plus six
  attempts' own time. A genuinely dead server: the first attempt fails fast
  (`Io` / pool cleared), the loop sleeps 2.5 s, then the batch is recorded
  failed. With strict options (`max_errors` reached) the stream aborts on that
  batch; with `continue_on_error` every subsequent batch pays the same 2.5 s
  until the lease keeper's own heartbeat fails and the worker stops. A
  `ServerSelection` timeout is not retried at all, so the 15 s
  `server_selection_timeout` is never multiplied.
- **Abort** stays cooperative and prompt: the retry loop checks the cancel
  token before every sleep.
- **The lease** is unaffected: the keeper heartbeats on its own task, and 2.5 s
  is far inside the 60 s default lease.
- **Composite deployments** (`mongodb-elasticsearch`) are unaffected: the
  secondary sync has its own retry (`composite/sync.rs:595-707`).

## 5. Testing

### 5.1 Unit tests in `retry.rs` (`#[cfg(test)]`, `#[tokio::test(start_paused = true)]`)

Transient errors are built with
`mongodb::error::Error::from(std::io::Error::from(std::io::ErrorKind::TimedOut))`
(`ErrorKind::Io`); non-transient ones with `mongodb::error::Error::custom(..)`.

1. a transient error is retried with the policy's exact sleep sequence and the
   op's success on attempt *k* returns `attempts == k`
2. a non-transient error returns after attempt 1 with no sleep
3. exhaustion returns the last error with `attempts == max_attempts`
4. a cancel token tripped between attempts returns immediately, without the
   pending sleep
5. `exhausted` maps transient ⇒ `Unavailable` / other ⇒ `Internal`, with and
   without the `(after N attempts)` suffix

### 5.2 Integration tests in `mongodb_tests.rs`, `mod bulk_submit`

Infrastructure:

- Add `"--setParameter", "enableTestCommands=1"` to the shared container's
  `with_cmd` (`mongodb_tests.rs:266`). `failCommand` works on a standalone
  server.
- A helper `failpoint(backend, doc)` that runs `configureFailPoint` on `admin`
  with `data.appName` set to the test's unique `app_name`, so the failpoint
  affects only that test's client while the rest of the suite shares the
  container; and a guard that turns it `off` on drop.
- Each test builds its backend with a unique `app_name`.
- When the server reports `enableTestCommands != 1` (`getParameter`) — an
  external `HFS_TEST_MONGODB_URL` — these tests skip with a message naming the
  parameter, the way the suite already skips without Docker.
- Because the test server is standalone, the driver performs no retry of its
  own, so every recovery observed is this change's.

Cases (a 3-entry batch of creates unless stated; assertions are on receipts,
`resources`, `resource_history`, `bulk_submission_changes` and the manifest
counters):

1. **Dropped `insert` recovers** — `failCommand { failCommands: ["insert"],
   closeConnection: true, mode: { times: 2 } }`: all three receipts `success`;
   exactly one resource, one history row and one rollback record each.
2. **Landed-but-unacknowledged `insert` does not duplicate** — `failCommand {
   failCommands: ["insert"], writeConcernError: { code: 91, errmsg: "..",
   errorLabels: ["RetryableWriteError"] }, mode: { times: 1 } }`: the command
   executes, then reports a retryable error; the retry takes the dup-key path.
   Same assertions as case 1. If the standalone server does not honour
   `writeConcernError` in `failCommand`, the implementer reports back rather
   than weakening the assertion.
3. **Exhausted retries contain to the batch** — `failCommands: ["insert"],
   closeConnection: true, mode: { times: 6 }` while ingesting a file of two
   batches (`batch_size: 3`, six lines of creates). `times: 6` is exact, not
   approximate, and the test's comment states the invariant it relies on:
   batch 1 is one chunk, so `write_resources` issues one `insert` per attempt;
   `BULK_INGEST_RETRY` allows six attempts; the standalone server adds no
   driver retry; and the exhausted `Err` short-circuits `write_batch` before
   history/search-index/changes issue any further `insert`. The failpoint is
   therefore spent exactly when batch 1 gives up, and batch 2's first `insert`
   succeeds. Assert: first three receipts are `processing-error` with
   `issue[0].code == "transient"` and diagnostics containing
   `(after 6 attempts)`; the last three are `success`; manifest
   `total_entries == 6`, `failed_entries == 3`, `processed_entries == 3`; the
   worker's file-level error branch was **not** taken (no "failed to ingest
   file" artifact). If any of those assumptions changes, this test fails
   loudly rather than flaking.
4. **`max_errors` now sees backend failures** — case 3 with strict options
   (`continue_on_error: false`, `max_errors: 3`): `process_ndjson_stream`
   returns `aborted("max errors exceeded")` after the first batch and the
   second batch is never attempted.
5. **Receipt path** — `failCommands: ["update"]`, `closeConnection: true,
   times: 2` on a batch of updates (seed the resources first): receipts land,
   each resource has exactly one new version.
6. **Cancel during backoff** — `times: 100`; spawn the ingest, sleep 150 ms
   (long enough for attempt 1 to fail and the first 100 ms backoff to begin,
   short enough to precede the 2.5 s budget by a wide margin), then trip the
   `CancelToken`. Wall-clock elapsed is not asserted beyond a loose `< 10 s`
   hang guard: a `closeConnection` failpoint makes each attempt pay a real
   driver reconnect, and how long that costs is environment-dependent, not
   something cancellation controls (observed ~1 s per reconnect on Windows +
   Docker Desktop, stacking to ~2 s once containment's receipt write also
   needs a fresh connection after the cancelled flush). What the test proves
   deterministically instead: cancellation ends the retry loop before its
   6-attempt budget. Read the three `processing-error` receipts
   (`get_entry_results_page`) and assert each `issue[0].diagnostics` contains
   `(after N attempts)` with `N < 6` — 2 or 3 is expected, but only the bound
   is asserted, since it depends on reconnect speed. Also assert
   `process_ndjson_stream` reports the cancelled abort reason and
   `counts.processing_error == 3` (the batch was recorded before the
   between-batch cancel check ran), both from the stream result and a fresh
   `get_entry_counts`.
7. **`confirm_landed`** directly: a row with the planned version and content
   ⇒ landed; same version, different content ⇒ not landed; absent ⇒ not landed.

Existing `mod bulk_submit` tests must keep passing unchanged; in particular
`test_a_mixed_batch_keeps_results_aligned_to_lines` and
`test_two_batches_racing_for_the_same_ids_stay_consistent` pin the
attempt-1 semantics this design preserves.

### 5.3 Gates

`cargo test -p helios-persistence --features mongodb` (lib + integration),
`cargo check --workspace --all-features`, `cargo clippy` clean on touched
files, `cargo fmt`.

## 6. Files touched

| File | Change |
|---|---|
| `crates/persistence/src/backends/mongodb/retry.rs` | new: policy, `retry_transient_with`, `Attempted`, `exhausted`, moved classifier, unit tests |
| `crates/persistence/src/backends/mongodb/mod.rs` | `mod retry;` |
| `crates/persistence/src/backends/mongodb/user_settings.rs` | remove moved items; `use super::retry::retry_transient` |
| `crates/persistence/src/backends/mongodb/bulk_ingest.rs` | per-stage retry + replay rules, `confirm_landed`, `write_batch` / `ingest_batch` split, `all_failed`, `write_entry_results` signature, module doc |
| `crates/persistence/src/backends/mongodb/bulk_submit.rs` | retry around the four bookkeeping commands in `process_entries` |
| `crates/persistence/src/backends/mongodb/backend.rs` | `app_name` config field |
| `crates/hfs/src/main.rs` | `app_name` in the exhaustive `MongoBackendConfig` literal (`:204-214`) |
| `crates/persistence/tests/mongodb_tests.rs` | container flag, failpoint helper, seven cases |
| `.claude/skills/bulk-data-submit/SKILL.md` | MongoDB behaviour note |

## 7. Follow-ups filed separately

- **Partial-status signal** (the second ask of #1001): a manifest that ends
  with `failed_entries > 0` still publishes `Completed`
  (`bulk_submit_worker.rs:1500-1509`); `ManifestPublicationStatus` has no third
  state; the REST status manifest has no count. S3 already carries a dormant
  `Failed if failed_count > 0` (`s3/bulk_submit.rs:421-427`) the worker
  bypasses. Needs its own design across the four backends and the REST/UI
  surface.
- **Rollback-log duplication on manifest re-walk**: `change_id` is minted at
  plan time, so a re-walk after lease reclaim appends a second record per
  entry. Independent of this change.
- **Per-document error handling for the file-level branch**: even after this
  change a receipt-write failure still records one file-level error with
  `count_severity {error: 1}` regardless of how many entries were in flight
  (`bulk_submit_worker.rs:1905`).
- **Derive MongoDB manifest counters from receipts.** `process_entries`'
  `$inc` over-counts on a re-walk today and can over-count one batch on a
  landed-but-unacknowledged retry after this change; PostgreSQL computes the
  same counters with a `SUM` over `bulk_entry_results` outcomes
  (`postgres/bulk_submit.rs:202`). Doing the same on MongoDB removes the
  non-idempotent write entirely.
