# MongoDB reindex batch-bytes (PR2a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `MongoBackend` honours `HFS_REINDEX_BATCH_BYTES` on the automatic `$reindex` rebuild — a byte-capped page never exceeds the cap unless it holds a single resource — and the server-side default of that variable becomes 32 MiB on every backend, closing #1499.

**Architecture:** MongoDB's PR1 walk (`fetch_resources_page`, S2 final) gains a byte-aware sibling, `fetch_reindex_page`, that both the uncapped path and the new `ReindexSource::fetch_resources_page_capped` override call; a byte-capped page stays non-empty and never ends a walk phase, so PR1's cursor and phase-transition logic needs no change beyond passing `max_bytes` through. Two unrelated batch-size bugs (a wrapping REST `batchSize` and an unclamped driver page limit) are fixed at their source. The `HFS_REINDEX_BATCH_BYTES` server default moves from `0` to `33554432` in three `crates/rest/src/config.rs` sites; the in-process library defaults (`ReindexRequest`, `AutomaticRunOptions`) stay `0`, so manual `$reindex` is unchanged, and a new test pins that the automatic hook still gets the new default when the operator never sets the variable.

**Tech Stack:** Rust (edition 2024), `mongodb` driver 3.7.0 (`bson::doc!`, `Cursor::current().as_bytes()`), `bson` 2.15.0, `async-trait`, testcontainers integration suite `crates/persistence/tests/mongodb_tests.rs` (feature `mongodb`, `mongo:5.0.6` via `testcontainers-modules`), `clap` env-backed config in `helios-rest`.

**Spec:** `docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md` §4.3 (PR2a) and §4.7 (sequence); design section `C:\Users\DougC\Code\Helios\manual-test\archive\1403-run17-evidence\design\S3-pr2-overlap.md` §4 (the whole of PR2a) and its `#4.1`–`#4.6` subsections; the PR2a-relevant consistency fixes are `S2-pr1-id-walk.md` §12's "PR2a (honour `batch_bytes`)" bullet, whose exact names (`WalkStep`, `RoundStartDecision`, `ReindexWalkCursor`, the filter builders) and the `fetch_resources_page` loop body this plan copies from are now also captured verbatim in PR1's own implementation plan, `docs/superpowers/plans/2026-09-23-1403-pr1-mongodb-id-walk.md` (Task 4) — that file exists in this repo (unlike when S3 was drafted) and is this plan's primary code source; S2/S3 remain the source for the *design rationale*. GitHub issue #1499 (`gh issue view 1499 --repo HeliosSoftware/hfs`) is the filed bug this PR closes; its six stale-doc-location list is Task 4's checklist.

## Global Constraints

- **Sequencing.** This plan assumes **PR0** (S1, always-on timing/`ReindexPageStats`) and **PR1** (S2 final, the id-order walk with catch-up rounds) have already merged to `main`. Work happens on `perf/1403-pr2a-mongodb-batch-bytes`, cut from `origin/main` **after** PR1 lands — not from `perf/1403-mongodb-reindex-rebuild`, which at the time this plan was written carries only the design doc commit (HEAD `0dbc51c82` / `c86d0f08b`), with no PR0/PR1 code yet.
- **No cargo while a bench arm is running (S4 §4.5.1, §4.12.3 "Orchestrator rule: no cargo during arms").** This plan is implemented while `B1-s`/`CU-1`/`B1-s-1G` (and possibly later arms) run on the orchestrator's bench host. Before every cargo/test/build command anywhere in this plan, run:
  ```bash
  test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
  ```
  If it prints and exits, stop immediately and report back instead of running `cargo`, `rustc`, a test or a build — an arm may be mid-measurement, and running any of these can invalidate that arm or the host-quiet precondition its validity checks assume. Task 6 shows this line inline, since it is the plan's final whole-suite verification step; every other cargo/test/build step in this plan does not repeat it inline, but it applies there identically — run it, unabbreviated, before each of those command blocks too.
- **Re-verify every anchor against the real `main` before editing.** Every `file:line` citation below that names MongoDB-walk code (`reindex_find_page`, `ReindexWalkCursor`, `dedupe_reindex_page_keep_last`, `insert_search_entries_chunk`, `fetch_resources_page`'s body) is a **hint**, not a promise — PR1's merged diff may differ in small ways from the code this plan quotes (which is copied from PR1's own plan, Task 4, as written; PR1's implementer may have adjusted it during that plan's execution). Before editing, `tgrep -n -F -- "<fn or struct name>" crates/persistence/src/backends/mongodb/storage.rs` (repo root, so the index is used) against the real `main` and use the struct/function **name**, not the line number, as the anchor. If any quoted PR1 code block below does not match what `main` actually has (different variable name, different error string, different field), keep `main`'s real wording and treat this plan's quoted block as a guide to *where* and *what shape* the edit is, not its literal text. Anchors that cite code already on `main` at plan-writing time (`crates/rest/src/config.rs`, `crates/persistence/src/search/reindex.rs`'s trait defaults and paging loop, `crates/rest/src/handlers/reindex.rs`) were confirmed against the real repo when this plan was written (line numbers verified 2026-09-23) and should not have moved, but re-check them too — PR0/PR1 may have touched neighbouring lines.
- **Byte-cap rule (S3 D2).** MongoDB uses PostgreSQL's strict rule, not SQLite's overshoot-by-one: a page never exceeds `max_bytes` unless it holds exactly one resource.
- **Byte measure (S3 D3).** Bytes are the raw BSON length of each `resources` row, read from the driver cursor (`Cursor::current().as_bytes().len() as u64`, `Cursor<T>::current` returns `&RawDocument`, mongodb-3.7.0 `src/cursor.rs:138`, `RawDocument::as_bytes` at bson-2.15.0 `src/raw/document.rs:509`) before the row is decoded — defined even for a row whose `data` fails to parse.
- **A capped page never ends a phase (S2 D12, S3 §4.1).** A byte-capped page is non-empty by construction (the first row is always admitted), so it always continues its current walk phase (`Id` or `Round`) with a cursor built from the last row it *took*; it never returns `None` and never logs a phase-transition line. `max_bytes == 0` is exactly PR1's existing uncapped page.
- **Server default (spec §4.3, S3 D4 — decided by the user 2026-09-23).** `HFS_REINDEX_BATCH_BYTES` becomes `33554432` (32 MiB, `32 * 1024 * 1024`) on every backend. `ReindexRequest::default()` and `AutomaticRunOptions::default()` stay `0`; manual `$reindex` and every existing unit test that builds a `ReindexRequest` directly are unaffected.
- **D12 batch-size clamps.** The REST handler saturates an out-of-range `batchSize` to `u32::MAX` instead of wrapping to `0` (`4294967296 as u32 == 0` today); the driver's paging loop passes `request.batch_size.max(1)` — confirmed at HEAD `crates/persistence/src/search/reindex.rs:2226` that the paging path passes `request.batch_size` completely unclamped today (only the named-resources path at `:2172` already clamps with `.max(1)`).
- **PR2a touches no writer, trait, index, catalog or generation, except:** `ReindexSource`'s `fetch_resources_page`/`fetch_resources_page_capped` on `MongoBackend`, the REST handler, the driver's paging loop, config defaults/docs, and `crates/hfs/src/main.rs` (a hook-builder split that adds no new behaviour, only a way for a test to read the concrete `ReindexOnFinish` it built) plus a read-only `ReindexOnFinish::batch_bytes()` getter.
- **This PR closes #1499.** The PR description must say `Closes #1499`.
- Never `git add -A` or `git commit -a` — a `cargo build`/`cargo check` dirties ~3,500 generated R6 spec files. Stage the explicit paths each task lists.
- Commit messages end with a blank line then `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- MongoDB integration tests: `cargo test -p helios-persistence --features mongodb --test mongodb_tests <filter>`. Docker must be running; the suite starts its own single-node-replica-set testcontainer (`mongo:5.0.6` via `testcontainers-modules` 0.15.0's default tag). **Never** set `HFS_TEST_MONGODB_URL` to the long-lived `hfs-mongo` corpus container, and never touch that container from this plan.
- CI clippy (persistence): `cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation`
- CI clippy (rest / hfs): **the same `-A` list as above**, not a bare `-D warnings` — confirmed at `.github/workflows/ci.yml:558` that CI's one repo-wide clippy invocation is `cargo clippy --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation`. Without those allowances, `-p helios-rest` fails on unrelated existing code: `crates/rest/src/handlers/reindex.rs:108-109` (`let mut request = ReindexRequest::default(); request.resource_types = resource_types;`, every field of `ReindexRequest` public) trips `clippy::field_reassign_with_default` today, at HEAD, with nothing this plan touches. Task 6 also runs CI's #1291 single-backend `hfs` matrix (`.github/workflows/ci.yml:579-583`), since Task 5 changes a feature-gated function in `main.rs`.
- `cargo fmt -- <explicit paths>` only — never `cargo fmt --all` from a worktree (it reformats unrelated generated/vendored files). Redirect command output to `/dev/null` in Git Bash, never `nul`. Set `CARGO_BUILD_JOBS=4` if memory is tight; if `target/` fills the disk, delete `target/debug/incremental` first.
- `crates/hfs` has no library target, only the `hfs` binary (`crates/hfs/src/main.rs`); its tests run via `cargo test -p helios-hfs --bin hfs <filter>`.

## File Structure

| File | Responsibility in this plan |
|---|---|
| `crates/persistence/src/backends/mongodb/storage.rs` | **Task 1a:** `reindex_page_admits` (pure admission rule), `ReindexFoundPage`, `max_bytes` added to PR1's `reindex_find_page` (now returns `ReindexFoundPage`), its two call sites inside `fetch_resources_page` updated to pass `max_bytes = 0` and read `found.docs`, `mod reindex_page_cap_tests`. **Task 1b:** new `fetch_reindex_page` (PR1's walk, byte-bounded, with the capped-page DEBUG line), `fetch_resources_page` delegates to it, new `fetch_resources_page_capped` override. |
| `crates/rest/src/handlers/reindex.rs` | `batch_size_param` (saturating `batchSize` clamp) next to `check_reindex_scope`, and its unit test (Task 3) |
| `crates/persistence/src/search/reindex.rs` | `page_limit` clamp in `run_reindex`'s paging loop; `page_limit_tests::paging_passes_at_least_one_as_the_page_limit` (Task 3); `ReindexOnFinish::batch_bytes()` getter (Task 5) |
| `crates/rest/src/config.rs` | `HFS_REINDEX_BATCH_BYTES` doc comment + `default_value` → `"33554432"`; the two `reindex_batch_bytes: 0,` struct literals → `32 * 1024 * 1024`; the defaults test assertion (Task 4) |
| `crates/hfs/src/main.rs` | `build_automatic_reindex_hook` extracted from `automatic_reindex_hook_with_ledger` (so a test can inspect the concrete hook); new test that the automatic hook gets the server default when `HFS_REINDEX_BATCH_BYTES` is unset (Task 5) |
| `crates/persistence/tests/mongodb/reindex_pipeline.rs` (new) | **Task 1b:** `create_backend_with`, `create_id_phase_backend`, `settle_into_id_phase`, `reindex_resource_row_sizes`, `walk_capped`, and the four boundary/zero-cap/limit-zero/larger-than-cap capped-fetch tests. **Task 2 (append):** `provenance_fixture`, `RecordingSource`/`RecordingWriter`, and the Provenance-shaped, driver-run and catch-up-boundary tests (S3 §4.5) |
| `crates/persistence/tests/mongodb_tests.rs` | One new `#[path]` include for `reindex_pipeline.rs` (Task 1b) |
| `README.md`, `crates/hfs/README.md`, `book/src/configuration/environment-variables.md`, `.claude/skills/bulk-data-submit/SKILL.md`, `.agents/skills/bulk-data-submit/SKILL.md` | The doc sweep named in #1499: cross-backend wording and the new default, in the five non-`config.rs` places the issue lists (Task 4) |
| `docs/mongodb/bulk-import-sizing.md` | Adds the "Knobs" section (item 6) that the PR-docs plan deliberately left for PR2a (spec §6: "The knobs section lands with PR2a/PR3") — conditional on that file already existing (Task 4) |

---

### Task 1a: The pure admission rule and a byte-aware `reindex_find_page`

This task is a behaviour-preserving refactor: after it, MongoDB's walk is byte-accounting-capable internally, but every caller still passes `max_bytes = 0`, so nothing observable changes yet. Task 1b wires the real cap through.

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/storage.rs` — add `reindex_page_admits`/`ReindexFoundPage` immediately before the `impl MongoBackend` block PR1 added for `reindex_newest_live_last_updated`/`reindex_find_page` (that block sits immediately before `impl ReindexSource for MongoBackend`; find it with `tgrep -n -F -- "async fn reindex_find_page" crates/persistence/src/backends/mongodb/storage.rs`); extend `reindex_find_page`'s signature and body; update its two call sites inside `fetch_resources_page` (the `IdPhase` and `Round` arms).

**Interfaces:**
- Consumes (from PR1, merged before this plan starts — signatures and code below are copied from `docs/superpowers/plans/2026-09-23-1403-pr1-mongodb-id-walk.md` Task 4, which is now the exact source of PR1's `storage.rs` additions; re-verify against real `main` per the Global Constraints anchor rule): `MongoBackend::reindex_find_page(&self, resources: &Collection<Document>, filter: Document, sort: Document, hint: &str, limit: u32) -> StorageResult<Vec<Document>>`; the `WalkStep` enum (`Start`, `IdPhase { floor, after_id }`, `RoundStart { round, floor }`, `Round { round, floor, ceiling, walked, after }`); `ReindexWalkCursor::{Id, Round}`; `RoundStartDecision`; `reindex_id_page_filter`, `reindex_catch_up_page_filter`, `reindex_catch_up_margin`, `reindex_catch_up_floor`, `reindex_catch_up_ceiling`, `reindex_round_start_decision`, `dedupe_reindex_page_keep_last`, `format_walk_instant`, `truncate_to_millis`, `bson_to_chrono`, `internal_error`, `reindex_page_from_docs(docs: &[Document], resource_type: &str, tenant: &TenantContext, next_cursor: ReindexWalkCursor) -> StorageResult<ResourcePage>`, `RESOURCES_IDENTITY_INDEX`, `RESOURCES_TYPE_SCAN_INDEX`; the existing `ReindexSource`/`ReindexTarget` traits (unchanged shape at HEAD: `fetch_resources_page(&self, tenant, resource_type, cursor: Option<&str>, limit: u32) -> StorageResult<ResourcePage>`, `fetch_resources_page_capped(&self, tenant, resource_type, cursor, limit, max_bytes: u64) -> StorageResult<ResourcePage>` with the ignoring default at `crates/persistence/src/search/reindex.rs:184-195`).
- Produces: `fn reindex_page_admits(taken: usize, bytes_taken: u64, row_bytes: u64, max_bytes: u64) -> bool` (private, pure); `struct ReindexFoundPage { docs: Vec<Document>, bytes: u64, capped: bool }` (private); `MongoBackend::reindex_find_page`'s new signature, `(&self, resources: &Collection<Document>, filter: Document, sort: Document, hint: &str, limit: u32, max_bytes: u64) -> StorageResult<ReindexFoundPage>`.

- [ ] **Step 1: Write the failing unit tests for the admission rule**

In `crates/persistence/src/backends/mongodb/storage.rs`, add a new module at the end of the file (S3 §4.5):

```rust
#[cfg(test)]
mod reindex_page_cap_tests {
    use super::*;

    #[test]
    fn admits_the_first_row_whatever_its_size() {
        assert!(reindex_page_admits(0, 0, 10_000, 1));
    }

    #[test]
    fn admits_up_to_and_including_the_cap() {
        assert!(reindex_page_admits(1, 100, 50, 150));
        assert!(!reindex_page_admits(1, 100, 50, 149));
    }

    #[test]
    fn zero_cap_admits_everything() {
        assert!(reindex_page_admits(7, u64::MAX, u64::MAX, 0));
    }

    #[test]
    fn saturating_sum_does_not_overflow() {
        assert!(reindex_page_admits(1, u64::MAX - 1, 10, u64::MAX));
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p helios-persistence --features mongodb --lib backends::mongodb::storage::reindex_page_cap_tests`
Expected: compile error, `cannot find function reindex_page_admits in this scope`.

- [ ] **Step 3: Implement the admission rule and `ReindexFoundPage`**

Immediately before the `impl MongoBackend` block PR1 added for `reindex_newest_live_last_updated`/`reindex_find_page` (locate it by name, not line number — PR1 may have shifted it), add:

```rust
/// Whether a reindex page that already holds `taken` rows totalling `bytes_taken`
/// admits a next row of `row_bytes` under `max_bytes` (`0` = no cap, #1499). The
/// first row is always admitted, so a page always advances; after it the page
/// never grows past the cap (PostgreSQL's rule, `postgres/storage.rs:3776-3834`).
fn reindex_page_admits(taken: usize, bytes_taken: u64, row_bytes: u64, max_bytes: u64) -> bool {
    max_bytes == 0 || taken == 0 || bytes_taken.saturating_add(row_bytes) <= max_bytes
}

/// What [`MongoBackend::reindex_find_page`] read (#1499): the rows it took, in scan
/// order, their raw BSON bytes, and whether the byte cap stopped it before `limit`.
struct ReindexFoundPage {
    docs: Vec<Document>,
    bytes: u64,
    capped: bool,
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p helios-persistence --features mongodb --lib backends::mongodb::storage::reindex_page_cap_tests`
Expected: 4 passed. (`ReindexFoundPage` is not yet constructed anywhere; an "unused struct" warning is expected and harmless until Step 5 below. Its `bytes`/`capped` fields stay unread until Task 1b's `fetch_reindex_page` logs them — a "field is never read" warning is expected and harmless until then too; clippy runs only in Task 6.)

- [ ] **Step 5: Write the failing compile check for the extended `reindex_find_page`**

There is no new externally observable behaviour in this step (both `max_bytes` values passed below are `0`, i.e. uncapped), so the test is that the crate still compiles and PR1's existing walk tests still pass unchanged — a refactor's regression check, not new TDD-red/green.

Run: `cargo check -p helios-persistence --features mongodb --lib`
Expected: compiles cleanly against the current (pre-edit) `reindex_find_page` — confirms the baseline before this step's edit, so a later compile failure is attributable to this step.

- [ ] **Step 6: Extend `reindex_find_page` and update its two call sites**

In `crates/persistence/src/backends/mongodb/storage.rs`, replace PR1's `reindex_find_page` (found by name inside the `impl MongoBackend` block from Step 3's anchor):

```rust
async fn reindex_find_page(
    &self,
    resources: &Collection<Document>,
    filter: Document,
    sort: Document,
    hint: &str,
    limit: u32,
    max_bytes: u64,
) -> StorageResult<ReindexFoundPage> {
    let mut stream = resources
        .find(filter)
        .sort(sort)
        .limit(limit as i64)
        .hint(Hint::Name(hint.to_string()))
        .await
        .map_err(|e| internal_error(format!("Failed to fetch resources: {e}")))?;
    let mut docs: Vec<Document> = Vec::new();
    let mut bytes: u64 = 0;
    let mut capped = false;
    while stream
        .advance()
        .await
        .map_err(|e| internal_error(format!("Failed to advance cursor: {e}")))?
    {
        let row_bytes = stream.current().as_bytes().len() as u64;
        if !reindex_page_admits(docs.len(), bytes, row_bytes, max_bytes) {
            capped = true;
            break;
        }
        bytes = bytes.saturating_add(row_bytes);
        docs.push(
            stream
                .deserialize_current()
                .map_err(|e| internal_error(format!("Failed to read resource: {e}")))?,
        );
    }
    drop(stream); // a capped read leaves server-side results; dropping kills the cursor
    Ok(ReindexFoundPage { docs, bytes, capped })
}
```

If `main`'s actual `reindex_find_page` (post-PR1) has a different error string or a different `.find(..).sort(..).limit(..).hint(..)` call shape than shown above, keep `main`'s exact wording for everything up to (and including) the `hint(..)` call and its `map_err`; only the body from `let mut docs: Vec<Document> = Vec::new();` onward, and the return type, are this step's change. (The error text `"Failed to fetch resources: {e}"` above is PR1's actual text, confirmed against `docs/superpowers/plans/2026-09-23-1403-pr1-mongodb-id-walk.md:1649`, not a placeholder guess.)

**Update `fetch_resources_page`'s two call sites.** Inside the same file's `impl ReindexSource for MongoBackend { async fn fetch_resources_page(..) }`, in the `WalkStep::IdPhase` arm, replace:

```rust
                WalkStep::IdPhase { floor, after_id } => {
                    let filter =
                        reindex_id_page_filter(tenant_id, resource_type, floor, after_id.as_deref());
                    let found = self
                        .reindex_find_page(
                            &resources,
                            filter,
                            doc! { "id": 1 },
                            RESOURCES_IDENTITY_INDEX,
                            limit,
                            0,
                        )
                        .await?;
                    let docs = found.docs;
                    if !docs.is_empty() {
                        let last_id = docs
                            .last()
                            .expect("non-empty")
                            .get_str("id")
                            .map_err(|e| internal_error(format!("Missing id: {e}")))?
                            .to_string();
                        return reindex_page_from_docs(
                            &docs,
                            resource_type,
                            tenant,
                            ReindexWalkCursor::Id { floor, after_id: last_id },
                        );
                    }
                    tracing::info!(
                        tenant = %tenant_id,
                        resource_type = %resource_type,
                        floor = %format_walk_instant(floor),
                        "mongodb reindex id phase finished"
                    );
                    WalkStep::RoundStart { round: 1, floor }
                }
```

(only the `let docs = self.reindex_find_page(..)` line changes shape — everything else in this arm is unchanged from PR1). And in the `WalkStep::Round` arm, replace:

```rust
                WalkStep::Round { round, floor, ceiling, walked, after } => {
                    let filter = reindex_catch_up_page_filter(
                        tenant_id,
                        resource_type,
                        floor,
                        ceiling,
                        after.as_ref().map(|(lu, id)| (*lu, id.as_str())),
                    );
                    let found = self
                        .reindex_find_page(
                            &resources,
                            filter,
                            doc! { "last_updated": 1, "id": 1 },
                            RESOURCES_TYPE_SCAN_INDEX,
                            limit,
                            0,
                        )
                        .await?;
                    let scanned = found.docs;
                    if scanned.is_empty() {
                        tracing::info!(
                            tenant = %tenant_id,
                            resource_type = %resource_type,
                            round,
                            floor = %format_walk_instant(floor),
                            ceiling = %format_walk_instant(ceiling),
                            walked,
                            "mongodb reindex catch-up round finished"
                        );
                        WalkStep::RoundStart { round: round + 1, floor: ceiling }
                    } else {
                        let last = scanned.last().expect("non-empty");
                        let scanned_lu = last
                            .get_datetime("last_updated")
                            .map_err(|e| internal_error(format!("Missing last_updated: {e}")))?;
                        let scanned_id = last
                            .get_str("id")
                            .map_err(|e| internal_error(format!("Missing id: {e}")))?
                            .to_string();
                        let scanned_last_updated = bson_to_chrono(scanned_lu);
                        let docs = dedupe_reindex_page_keep_last(scanned);
                        let walked = walked + docs.len() as u64;
                        return reindex_page_from_docs(
                            &docs,
                            resource_type,
                            tenant,
                            ReindexWalkCursor::Round {
                                round,
                                floor,
                                ceiling,
                                walked,
                                after_last_updated: scanned_last_updated,
                                after_id: scanned_id,
                            },
                        );
                    }
                }
```

`WalkStep::Start` and `WalkStep::RoundStart` are untouched by this step — they never call `reindex_find_page`.

- [ ] **Step 7: Run the tests to verify the refactor is behaviour-preserving**

Run: `cargo test -p helios-persistence --features mongodb --lib backends::mongodb::storage::reindex_page_cap_tests`
Expected: 4 passed (unchanged from Step 4).

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_id_walk` (requires Docker or `HFS_TEST_MONGODB_URL`)
Expected: unchanged, all passing — every call site still passes `max_bytes = 0`, so PR1's walk behaves exactly as before.

- [ ] **Step 8: Commit**

```bash
git add crates/persistence/src/backends/mongodb/storage.rs
git commit -m "$(cat <<'EOF'
refactor(mongodb): thread a byte budget through reindex_find_page (#1499)

reindex_find_page now returns ReindexFoundPage (rows, bytes read, and
whether the cap stopped it) instead of a bare Vec<Document>, and takes
a max_bytes parameter. Every call site still passes 0 (no cap), so the
walk's behaviour is unchanged; Task 1b wires the real cap through
fetch_reindex_page.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 1b: `fetch_reindex_page`, the byte-capped override, and its integration tests

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/storage.rs` — add `fetch_reindex_page` to the same `impl MongoBackend` block Task 1a's `reindex_find_page` lives in; make `fetch_resources_page` delegate to it; add the `fetch_resources_page_capped` override in `impl ReindexSource for MongoBackend`.
- Create: `crates/persistence/tests/mongodb/reindex_pipeline.rs`
- Modify: `crates/persistence/tests/mongodb_tests.rs` — one new `#[path]` include, placed directly after PR1's own `reindex_id_walk` include (S2 §7.5 puts that one right after the `versioned_write_race_suite` include; find PR1's include by name and add this one immediately after it).

**Interfaces:**
- Consumes: Task 1a's `ReindexFoundPage`, the extended `reindex_find_page`; PR1's `WalkStep`, `ReindexWalkCursor`, `RoundStartDecision`, filter builders, `reindex_page_from_docs`, `reindex_catch_up_margin`, `format_walk_instant`, `truncate_to_millis`, `bson_to_chrono`, `internal_error`, `RESOURCES_IDENTITY_INDEX`, `RESOURCES_TYPE_SCAN_INDEX` (all as in Task 1a); the test harness `create_tenant`, `raw_test_client`, `build_backend`, `repo_data_dir`, `build_test_database_name`, `shared_mongo::connection_string` from `mongodb_tests.rs`'s root (reached through `use super::*;`).
- Produces: `async fn fetch_reindex_page(&self, tenant: &TenantContext, resource_type: &str, cursor: Option<&str>, limit: u32, max_bytes: u64) -> StorageResult<ResourcePage>` (private inherent method on `MongoBackend`); the `ReindexSource::fetch_resources_page_capped` override on `MongoBackend`; test helpers `create_backend_with`, `create_id_phase_backend`, `settle_into_id_phase`, `reindex_resource_row_sizes`, `walk_capped` (all in `reindex_pipeline.rs`, consumed by Task 2 too).

- [ ] **Step 1: Write the failing integration tests**

Create `crates/persistence/tests/mongodb/reindex_pipeline.rs`:

```rust
//! #1499: MongoDB honours `HFS_REINDEX_BATCH_BYTES` on the automatic
//! `$reindex` rebuild. Child module of the `mongodb_tests` root — `use
//! super::*;` reaches its private harness (`create_tenant`, `raw_test_client`,
//! `build_backend`, `repo_data_dir`, `build_test_database_name`,
//! `shared_mongo`, plus `Bson`/`Document`/`doc`/`json`/`FhirVersion`/`Client`,
//! all imported at the test-crate root), the same arrangement as
//! `tests/mongodb/reindex_id_walk.rs`.

use super::*;

use futures::TryStreamExt;
use helios_persistence::search::{ReindexSource, ResourcePage};

/// Builds a `MongoBackend` for this file's tests: same shape as
/// `create_backend_with_search_offloaded` (`mongodb_tests.rs:1233-1247`), but
/// lets the caller tweak the config first — used by [`create_id_phase_backend`]
/// below, and directly by PR2b's own tests (S3 §5.13), so this helper stays a
/// plain pass-through with no default catch-up-margin override of its own.
async fn create_backend_with(
    test_name: &str,
    configure: impl FnOnce(&mut MongoBackendConfig),
) -> Option<Arc<MongoBackend>> {
    let connection_string = shared_mongo::connection_string().await?;
    let mut config = MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test_name),
        data_dir: Some(repo_data_dir()),
        ..Default::default()
    };
    configure(&mut config);
    build_backend(config).await.map(Arc::new)
}

/// Builds a `MongoBackend` with the catch-up margin shortened to 1 s, so a
/// walk over already-seeded fixture rows can be moved into its id phase by
/// [`settle_into_id_phase`] (S3 §4.5's "id-phase fixture rule"). Every test
/// that exercises the id phase specifically — as opposed to a resource just
/// created, which the default 120 s margin would fold into catch-up round 1
/// (S2 §3.2: `floor = min(newest_live + 1 ms, t0 - margin)`, which with a
/// 120 s margin and resources seeded moments ago sits in the past relative to
/// nothing, so every seeded row has `last_updated >= floor` and the id phase's
/// first query comes back empty) — must use this instead of
/// [`create_backend_with`] directly.
async fn create_id_phase_backend(test_name: &str) -> Option<Arc<MongoBackend>> {
    create_backend_with(test_name, |c| c.reindex_catch_up_margin_ms = 1_000).await
}

/// Sleeps past [`create_id_phase_backend`]'s shortened margin, so every row
/// seeded before this call has a `last_updated` older than any walk's floor
/// and is walked by the id phase rather than folded into catch-up round 1.
/// Call it after seeding, before the first `fetch_resources_page_capped` /
/// `fetch_resources_page` call of the test.
async fn settle_into_id_phase() {
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
}

/// Reads each row's exact stored size the same way PostgreSQL's boundary test
/// does, via `$bsonSize` (needs MongoDB 4.4+; the floor is 5.0.6), sorted by
/// `id` to match the id phase's scan order. `$bsonSize` returns a 32-bit
/// `NumberInt` (MongoDB computes it as `Value(doc.toBson().objsize())`, and
/// `objsize()` is a 32-bit `int`), so `get_i64` — which bson 2.15.0 accepts
/// only for an actual `Bson::Int64` — is not safe to call on it; accept
/// either width explicitly.
async fn reindex_resource_row_sizes(
    db: &mongodb::Database,
    tenant_id: &str,
    resource_type: &str,
) -> Vec<(String, u64)> {
    let mut cursor = db
        .collection::<Document>("resources")
        .aggregate(vec![
            doc! { "$match": { "tenant_id": tenant_id, "resource_type": resource_type, "is_deleted": false } },
            doc! { "$sort": { "id": 1 } },
            doc! { "$project": { "id": 1, "size": { "$bsonSize": "$$ROOT" } } },
        ])
        .await
        .unwrap();
    let mut rows = Vec::new();
    while let Some(doc) = cursor.try_next().await.unwrap() {
        let id = doc.get_str("id").unwrap().to_string();
        let size = match doc.get("size") {
            Some(Bson::Int32(n)) => *n as u64,
            Some(Bson::Int64(n)) => *n as u64,
            other => panic!("$bsonSize returned {other:?}"),
        };
        rows.push((id, size));
    }
    rows
}

/// Walks `resource_type` to completion through `fetch_resources_page_capped`,
/// returning every non-empty page in fetch order. PR1 guarantees exactly one
/// trailing empty page per type (S2 D12), so more than 20 fetches means the
/// walk is not terminating (S3 §4.5's guard).
async fn walk_capped(
    backend: &MongoBackend,
    tenant: &TenantContext,
    resource_type: &str,
    limit: u32,
    max_bytes: u64,
) -> Vec<ResourcePage> {
    let mut cursor: Option<String> = None;
    let mut pages = Vec::new();
    for _ in 0..20 {
        let page = backend
            .fetch_resources_page_capped(tenant, resource_type, cursor.as_deref(), limit, max_bytes)
            .await
            .unwrap();
        let done = page.next_cursor.is_none();
        cursor = page.next_cursor.clone();
        if !page.resources.is_empty() {
            pages.push(page);
        }
        if done {
            return pages;
        }
    }
    panic!("walk of {resource_type} did not reach its trailing empty page within 20 fetches");
}

#[tokio::test]
async fn mongodb_integration_reindex_fetch_capped_boundaries() {
    let Some(backend) = create_id_phase_backend("reindex_capped_boundaries").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-capped-boundaries");
    for (id, k) in [("p01", 1usize), ("p02", 2), ("p03", 3), ("p04", 4)] {
        backend
            .create_or_update(
                &tenant,
                "Patient",
                id,
                json!({"resourceType": "Patient", "id": id, "name": [{"family": "X".repeat(200 * k)}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    settle_into_id_phase().await;
    let db = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name);
    let sizes = reindex_resource_row_sizes(&db, "tenant-capped-boundaries", "Patient").await;
    let ids: Vec<_> = sizes.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["p01", "p02", "p03", "p04"]);
    let first = sizes[0].1;
    let two = sizes[0].1 + sizes[1].1;

    for (cap, expected_first) in [
        (two, vec!["p01", "p02"]),
        (two - 1, vec!["p01"]),
        (first - 1, vec!["p01"]),
        (two + 1, vec!["p01", "p02"]),
    ] {
        let mut cursor: Option<String> = None;
        let mut seen = Vec::new();
        let mut first_page = true;
        let mut finished = false;
        for _ in 0..20 {
            let page = backend
                .fetch_resources_page_capped(&tenant, "Patient", cursor.as_deref(), 4, cap)
                .await
                .unwrap();
            let page_ids: Vec<String> =
                page.resources.iter().map(|r| r.id().to_string()).collect();
            let page_bytes: u64 = page_ids
                .iter()
                .map(|id| sizes.iter().find(|(key, _)| key == id).unwrap().1)
                .sum();
            assert!(
                page_bytes <= cap || page_ids.len() == 1,
                "cap {cap}: page {page_ids:?} totalled {page_bytes} bytes"
            );
            if first_page && !page_ids.is_empty() {
                assert_eq!(page_ids, expected_first, "cap {cap}: first page");
                assert!(
                    page.next_cursor
                        .as_deref()
                        .is_some_and(|c| c.starts_with("v2|i|")),
                    "cap {cap}: a capped page must stay in the id phase"
                );
                first_page = false;
            }
            seen.extend(page_ids);
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => {
                    finished = true;
                    break;
                }
            }
        }
        assert!(finished, "cap {cap}: walk did not terminate within 20 fetches");
        assert_eq!(seen, ["p01", "p02", "p03", "p04"], "cap {cap}");
    }
}

#[tokio::test]
async fn mongodb_integration_reindex_fetch_capped_zero_cap_matches_uncapped() {
    let Some(backend) = create_id_phase_backend("reindex_zero_cap_matches").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-zero-cap-matches");
    for id in ["p01", "p02", "p03", "p04"] {
        backend
            .create_or_update(
                &tenant,
                "Patient",
                id,
                json!({"resourceType": "Patient", "id": id}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    settle_into_id_phase().await;
    for limit in [2u32, 4] {
        let capped = walk_capped(&backend, &tenant, "Patient", limit, 0).await;

        let mut cursor: Option<String> = None;
        let mut uncapped = Vec::new();
        for _ in 0..20 {
            let page = backend
                .fetch_resources_page(&tenant, "Patient", cursor.as_deref(), limit)
                .await
                .unwrap();
            let done = page.next_cursor.is_none();
            cursor = page.next_cursor.clone();
            if !page.resources.is_empty() {
                uncapped.push(page);
            }
            if done {
                break;
            }
        }

        assert_eq!(capped.len(), uncapped.len(), "limit {limit}: page count");
        for (c, u) in capped.iter().zip(uncapped.iter()) {
            let c_ids: Vec<_> = c.resources.iter().map(|r| r.id().to_string()).collect();
            let u_ids: Vec<_> = u.resources.iter().map(|r| r.id().to_string()).collect();
            assert_eq!(c_ids, u_ids, "limit {limit}");
            if c
                .next_cursor
                .as_deref()
                .is_some_and(|s| s.starts_with("v2|i|"))
            {
                assert_eq!(
                    c.next_cursor, u.next_cursor,
                    "limit {limit}: id-phase cursors must match exactly"
                );
            }
        }
    }
}

#[tokio::test]
async fn mongodb_integration_reindex_fetch_capped_limit_zero_reads_one_per_page() {
    let Some(backend) = create_id_phase_backend("reindex_limit_zero").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-limit-zero");
    for id in ["p01", "p02"] {
        backend
            .create_or_update(
                &tenant,
                "Patient",
                id,
                json!({"resourceType": "Patient", "id": id}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    settle_into_id_phase().await;
    for max_bytes in [0u64, u64::MAX] {
        let page = backend
            .fetch_resources_page_capped(&tenant, "Patient", None, 0, max_bytes)
            .await
            .unwrap();
        assert_eq!(page.resources.len(), 1, "max_bytes {max_bytes}");
        assert!(page.next_cursor.is_some(), "max_bytes {max_bytes}: must continue");
    }
}

#[tokio::test]
async fn mongodb_integration_reindex_fetch_capped_returns_a_resource_larger_than_the_cap() {
    let Some(backend) = create_id_phase_backend("reindex_larger_than_cap").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-larger-than-cap");
    for id in ["p01", "p02"] {
        backend
            .create_or_update(
                &tenant,
                "Patient",
                id,
                json!({"resourceType": "Patient", "id": id, "name": [{"family": "X".repeat(500)}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    settle_into_id_phase().await;
    let pages = walk_capped(&backend, &tenant, "Patient", 10, 2).await;
    for page in &pages {
        assert_eq!(
            page.resources.len(),
            1,
            "every page must hold exactly one over-cap resource"
        );
    }
    let seen: Vec<String> = pages
        .iter()
        .flat_map(|p| p.resources.iter().map(|r| r.id().to_string()))
        .collect();
    assert_eq!(seen, ["p01", "p02"]);
}
```

Add the include to `crates/persistence/tests/mongodb_tests.rs`, directly after PR1's `reindex_id_walk` include (find it by name — S2 places it right after `versioned_write_race_suite`):

```rust
/// #1499: MongoDB honours `HFS_REINDEX_BATCH_BYTES` (PR2a).
#[path = "mongodb/reindex_pipeline.rs"]
mod reindex_pipeline;
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_pipeline`
Expected: `mongodb_integration_reindex_fetch_capped_boundaries` and `_returns_a_resource_larger_than_the_cap` fail — every page holds all 4 (or 2) resources regardless of `cap`, because `fetch_resources_page_capped` still uses the trait default that ignores `max_bytes` (Task 1a did not add the override). `_limit_zero_reads_one_per_page` and `_zero_cap_matches_uncapped` pass vacuously today (an uncapped page with `limit=0` already returns one resource via PR1's `limit.max(1)`, and `max_bytes=0`/`u64::MAX` already behave like the uncapped path) — that's expected; Step 4 must not regress them.

- [ ] **Step 3: Implement `fetch_reindex_page` and wire it in**

All edits are in `crates/persistence/src/backends/mongodb/storage.rs`.

**3a. Add `fetch_reindex_page`,** in the same `impl MongoBackend` block Task 1a's `reindex_find_page` lives in (do not create a second `impl MongoBackend` block):

```rust
/// Pages `resource_type` in id order with catch-up rounds (PR1, #1403), bounded
/// by `max_bytes` as well as by `limit` (`max_bytes == 0` is PR1's uncapped
/// page, #1499). A page the byte cap stops before `limit` is still non-empty
/// (the first row is always admitted), so it continues its current walk phase
/// exactly as a full page would — it never ends a phase and never returns
/// `None` on its own account. `fetch_resources_page` and
/// `fetch_resources_page_capped` are both thin calls to this method.
async fn fetch_reindex_page(
    &self,
    tenant: &TenantContext,
    resource_type: &str,
    cursor: Option<&str>,
    limit: u32,
    max_bytes: u64,
) -> StorageResult<ResourcePage> {
    let db = self.get_database().await?;
    let resources: Collection<Document> = db.collection(MongoBackend::RESOURCES_COLLECTION);
    let tenant_id = tenant.tenant_id().as_str();
    let limit = limit.max(1); // MongoDB treats limit(0) as "no limit"
    let margin = reindex_catch_up_margin(self.config().reindex_catch_up_margin_ms);

    let mut step = match cursor {
        None => WalkStep::Start,
        Some(c) => WalkStep::from(ReindexWalkCursor::parse(c)?),
    };

    loop {
        step = match step {
            WalkStep::Start => {
                let t0 = Utc::now();
                let newest_live = self
                    .reindex_newest_live_last_updated(&resources, tenant_id, resource_type)
                    .await?;
                let floor = reindex_catch_up_floor(t0, newest_live, margin);
                tracing::info!(
                    tenant = %tenant_id,
                    resource_type = %resource_type,
                    t0 = %format_walk_instant(t0),
                    newest_live = %newest_live.map(format_walk_instant).unwrap_or_else(|| "none".to_string()),
                    floor = %format_walk_instant(floor),
                    "mongodb reindex walk started"
                );
                WalkStep::IdPhase { floor, after_id: None }
            }
            WalkStep::IdPhase { floor, after_id } => {
                let filter =
                    reindex_id_page_filter(tenant_id, resource_type, floor, after_id.as_deref());
                let found = self
                    .reindex_find_page(
                        &resources,
                        filter,
                        doc! { "id": 1 },
                        RESOURCES_IDENTITY_INDEX,
                        limit,
                        max_bytes,
                    )
                    .await?;
                if max_bytes > 0 {
                    tracing::debug!(
                        tenant = %tenant_id,
                        resource_type = %resource_type,
                        rows = found.docs.len(),
                        bytes = found.bytes,
                        capped = found.capped,
                        "mongodb reindex capped page read"
                    );
                }
                let docs = found.docs;
                if !docs.is_empty() {
                    let last_id = docs
                        .last()
                        .expect("non-empty")
                        .get_str("id")
                        .map_err(|e| internal_error(format!("Missing id: {e}")))?
                        .to_string();
                    return reindex_page_from_docs(
                        &docs,
                        resource_type,
                        tenant,
                        ReindexWalkCursor::Id { floor, after_id: last_id },
                    );
                }
                tracing::info!(
                    tenant = %tenant_id,
                    resource_type = %resource_type,
                    floor = %format_walk_instant(floor),
                    "mongodb reindex id phase finished"
                );
                WalkStep::RoundStart { round: 1, floor }
            }
            WalkStep::RoundStart { round, floor } => {
                let now = Utc::now();
                match reindex_round_start_decision(round, floor, now, margin) {
                    RoundStartDecision::Complete => {
                        tracing::debug!(
                            tenant = %tenant_id,
                            resource_type = %resource_type,
                            rounds = round - 1,
                            "mongodb reindex catch-up complete"
                        );
                        return Ok(ResourcePage {
                            resources: Vec::new(),
                            next_cursor: None,
                            skipped: Vec::new(),
                        });
                    }
                    RoundStartDecision::CapReached => {
                        tracing::warn!(
                            tenant = %tenant_id,
                            resource_type = %resource_type,
                            rounds = round - 1,
                            last_ceiling = %format_walk_instant(floor),
                            "mongodb reindex catch-up stopped at its round limit"
                        );
                        return Ok(ResourcePage {
                            resources: Vec::new(),
                            next_cursor: None,
                            skipped: Vec::new(),
                        });
                    }
                    RoundStartDecision::Run => {
                        let newest_live = self
                            .reindex_newest_live_last_updated(&resources, tenant_id, resource_type)
                            .await?;
                        let ceiling = reindex_catch_up_ceiling(now, newest_live, margin);
                        if let Some(newest_live) = newest_live {
                            let by_margin_only = truncate_to_millis(now + margin);
                            if ceiling > by_margin_only {
                                tracing::warn!(
                                    tenant = %tenant_id,
                                    resource_type = %resource_type,
                                    round,
                                    newest_live = %format_walk_instant(newest_live),
                                    ceiling = %format_walk_instant(ceiling),
                                    "mongodb reindex found live resources stamped in the future"
                                );
                            }
                        }
                        tracing::info!(
                            tenant = %tenant_id,
                            resource_type = %resource_type,
                            round,
                            floor = %format_walk_instant(floor),
                            ceiling = %format_walk_instant(ceiling),
                            "mongodb reindex catch-up round started"
                        );
                        WalkStep::Round { round, floor, ceiling, walked: 0, after: None }
                    }
                }
            }
            WalkStep::Round { round, floor, ceiling, walked, after } => {
                let filter = reindex_catch_up_page_filter(
                    tenant_id,
                    resource_type,
                    floor,
                    ceiling,
                    after.as_ref().map(|(lu, id)| (*lu, id.as_str())),
                );
                let found = self
                    .reindex_find_page(
                        &resources,
                        filter,
                        doc! { "last_updated": 1, "id": 1 },
                        RESOURCES_TYPE_SCAN_INDEX,
                        limit,
                        max_bytes,
                    )
                    .await?;
                if max_bytes > 0 {
                    tracing::debug!(
                        tenant = %tenant_id,
                        resource_type = %resource_type,
                        rows = found.docs.len(),
                        bytes = found.bytes,
                        capped = found.capped,
                        "mongodb reindex capped page read"
                    );
                }
                let scanned = found.docs;
                if scanned.is_empty() {
                    tracing::info!(
                        tenant = %tenant_id,
                        resource_type = %resource_type,
                        round,
                        floor = %format_walk_instant(floor),
                        ceiling = %format_walk_instant(ceiling),
                        walked,
                        "mongodb reindex catch-up round finished"
                    );
                    WalkStep::RoundStart { round: round + 1, floor: ceiling }
                } else {
                    let last = scanned.last().expect("non-empty");
                    let scanned_lu = last
                        .get_datetime("last_updated")
                        .map_err(|e| internal_error(format!("Missing last_updated: {e}")))?;
                    let scanned_id = last
                        .get_str("id")
                        .map_err(|e| internal_error(format!("Missing id: {e}")))?
                        .to_string();
                    let scanned_last_updated = bson_to_chrono(scanned_lu);
                    let docs = dedupe_reindex_page_keep_last(scanned);
                    let walked = walked + docs.len() as u64;
                    return reindex_page_from_docs(
                        &docs,
                        resource_type,
                        tenant,
                        ReindexWalkCursor::Round {
                            round,
                            floor,
                            ceiling,
                            walked,
                            after_last_updated: scanned_last_updated,
                            after_id: scanned_id,
                        },
                    );
                }
            }
        };
    }
}
```

Field order in both DEBUG lines (`tenant, resource_type, rows, bytes, capped`) matches S3 §4.2 item 3 exactly; both use `%` on the two string fields (`tenant_id`, `resource_type`) per S2 §3.9's formatting rule, matching every other log line in this method.

**3b. Make `fetch_resources_page` delegate to it,** replacing its entire body (the walk loop Task 1a left there) in `impl ReindexSource for MongoBackend`:

```rust
async fn fetch_resources_page(
    &self,
    tenant: &TenantContext,
    resource_type: &str,
    cursor: Option<&str>,
    limit: u32,
) -> StorageResult<ResourcePage> {
    self.fetch_reindex_page(tenant, resource_type, cursor, limit, 0)
        .await
}
```

**3c. Add the override,** in the same `impl ReindexSource for MongoBackend` block:

```rust
/// Pages by resource count and, when `max_bytes` is set, by the raw BSON
/// bytes of the `resources` rows the page reads: a page never exceeds
/// `max_bytes` unless it holds exactly one resource (PostgreSQL's strict
/// rule, not SQLite's overshoot-by-one, #1499). A byte-capped page continues
/// the walk's current phase — it is never empty, so it never ends a phase —
/// and its cursor names the last row it *took*, never a row the cap rejected.
async fn fetch_resources_page_capped(
    &self,
    tenant: &TenantContext,
    resource_type: &str,
    cursor: Option<&str>,
    limit: u32,
    max_bytes: u64,
) -> StorageResult<ResourcePage> {
    self.fetch_reindex_page(tenant, resource_type, cursor, limit, max_bytes)
        .await
}
```

No new imports are needed beyond what PR1 already added (`Document` at `:10` on HEAD before PR1; `Hint` from `mongodb::options`); no trait, index, catalog or generation change.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_pipeline` and `cargo test -p helios-persistence --features mongodb --lib backends::mongodb::storage::reindex_page_cap_tests`
Expected: all 4 unit tests and all 4 integration tests pass. If Docker is unavailable, the integration tests print "Skipping" and return `Ok` — that is not a substitute for a real pass; run them where Docker is available at least once before Step 5.

Also run PR1's own suite to confirm this task did not regress the walk: `cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_id_walk` and `cargo test -p helios-persistence --features mongodb --lib backends::mongodb::storage::reindex_walk_tests`.
Expected: unchanged, all passing.

- [ ] **Step 5: Commit**

```bash
git add crates/persistence/src/backends/mongodb/storage.rs crates/persistence/tests/mongodb/reindex_pipeline.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "$(cat <<'EOF'
feat(mongodb): honour HFS_REINDEX_BATCH_BYTES on the reindex source (#1499)

MongoBackend now overrides ReindexSource::fetch_resources_page_capped
with PostgreSQL's strict rule (a page never exceeds max_bytes unless it
holds one resource), measured as each resources row's raw BSON length.
A capped page stays non-empty, so it continues PR1's walk phase without
any change to phase-end or cursor logic.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Additional coverage — Provenance-shaped pages, the driver, and the catch-up boundary

This task adds test coverage only; it changes no production code. Because Task 1b already implements the full behaviour these tests exercise, each test is expected to **pass on its first run** — write it, run it, and if it fails, that is a real gap in Task 1b's implementation to go back and fix, not new code to write here.

**Files:**
- Modify: `crates/persistence/tests/mongodb/reindex_pipeline.rs` (append)

**Interfaces:**
- Consumes: `create_backend_with`, `create_id_phase_backend`, `settle_into_id_phase`, `walk_capped`, `reindex_resource_row_sizes` (Task 1b); `ReindexOperation::with_parts(source: Arc<dyn ReindexSource>, writers: Vec<Arc<dyn ReindexTarget>>, registries: Arc<TenantSearchRegistries>) -> Self`; `ReindexRequest::for_types(..).with_batch_size(..).with_batch_bytes(..)`; `operation.start(tenant: TenantContext, request: ReindexRequest, agent: Option<String>) -> Result<String, ReindexError>`; `operation.get_progress(&self, job_id: &str) -> Option<ReindexProgress>` (confirmed at `crates/persistence/src/search/reindex.rs:1241` — it is `Option`, not a `Result`).
- Produces: `provenance_fixture(n: usize) -> Vec<(String, serde_json::Value)>`, `RecordingSource`, `RecordingWriter` (test-only fakes, this file).

- [ ] **Step 1: Write the tests**

Append to `crates/persistence/tests/mongodb/reindex_pipeline.rs`, first adding these imports above the new code (the ones `use super::*;` at the top of the file does not already cover — `Mutex`, `AtomicU64`/`Ordering`, `async_trait`, `StorageResult`, and the `ReindexOperation`/`ReindexPageStats`/`ReindexRequest`/`ReindexStatus`/`ReindexTarget`/`StoredResource` types this task's fakes and driver test need that Task 1b's tests did not):

```rust
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use helios_persistence::error::StorageResult;
use helios_persistence::search::{
    ReindexOperation, ReindexPageStats, ReindexRequest, ReindexStatus, ReindexTarget,
};
use helios_persistence::types::StoredResource;

/// `n` Provenance resources shaped like the #1403 corpus's extreme case: one
/// agent and 1,600 `target` references each, ~70-100 KB of BSON per resource.
fn provenance_fixture(n: usize) -> Vec<(String, serde_json::Value)> {
    (0..n)
        .map(|i| {
            let id = format!("prov-{i:02}");
            let targets: Vec<serde_json::Value> = (0..1600)
                .map(|k| json!({ "reference": format!("Observation/{id}-{k:05}") }))
                .collect();
            (
                id.clone(),
                json!({
                    "resourceType": "Provenance",
                    "id": id,
                    "agent": [{ "who": { "reference": "Practitioner/example" } }],
                    "target": targets,
                }),
            )
        })
        .collect()
}

#[tokio::test]
async fn mongodb_integration_reindex_fetch_capped_provenance_shaped() {
    let Some(backend) = create_id_phase_backend("reindex_provenance_shaped").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-provenance-shaped");
    let fixture = provenance_fixture(24);
    for (id, resource) in &fixture {
        backend
            .create_or_update(&tenant, "Provenance", id, resource.clone(), FhirVersion::default())
            .await
            .unwrap();
    }
    settle_into_id_phase().await;
    let db = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name);
    let sizes = reindex_resource_row_sizes(&db, "tenant-provenance-shaped", "Provenance").await;
    let mut by_size = sizes.clone();
    by_size.sort_by_key(|(_, size)| *size);
    let cap: u64 = by_size.iter().take(3).map(|(_, size)| *size).sum();

    let pages = walk_capped(&backend, &tenant, "Provenance", 24, cap).await;
    for page in &pages {
        assert!(page.resources.len() <= 3, "page held {} resources", page.resources.len());
        let bytes: u64 = page
            .resources
            .iter()
            .map(|r| sizes.iter().find(|(id, _)| id == r.id()).unwrap().1)
            .sum();
        assert!(bytes <= cap || page.resources.len() == 1);
    }
    let seen: std::collections::BTreeSet<String> = pages
        .iter()
        .flat_map(|p| p.resources.iter().map(|r| r.id().to_string()))
        .collect();
    assert_eq!(seen.len(), 24, "every resource must come back exactly once");

    let unbounded = walk_capped(&backend, &tenant, "Provenance", 5, u64::MAX).await;
    let last_index = unbounded.len().saturating_sub(1);
    for (i, page) in unbounded.iter().enumerate() {
        assert!(
            page.next_cursor
                .as_deref()
                .is_some_and(|c| c.starts_with("v2|i|"))
                || i == last_index,
            "page {i} of an id-only limit-5 walk must stay in the id phase"
        );
        if i == last_index {
            assert!(page.resources.len() <= 5);
        } else {
            assert_eq!(page.resources.len(), 5, "page {i} of an id-only limit-5 walk");
        }
    }
}

struct RecordingSource {
    inner: Arc<MongoBackend>,
    sizes: std::collections::HashMap<String, u64>,
    pages: Mutex<Vec<(usize, u64)>>,
}

#[async_trait]
impl ReindexSource for RecordingSource {
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        self.inner.list_resource_types(tenant).await
    }
    async fn count_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        self.inner.count_resources(tenant, resource_type).await
    }
    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<ResourcePage> {
        self.inner
            .fetch_resources_page(tenant, resource_type, cursor, limit)
            .await
    }
    async fn fetch_resources_page_capped(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<ResourcePage> {
        let page = self
            .inner
            .fetch_resources_page_capped(tenant, resource_type, cursor, limit, max_bytes)
            .await?;
        let bytes: u64 = page
            .resources
            .iter()
            .map(|r| *self.sizes.get(r.id()).expect("fixture size"))
            .sum();
        self.pages.lock().unwrap().push((page.resources.len(), bytes));
        Ok(page)
    }
}

struct RecordingWriter {
    inner: Arc<MongoBackend>,
    written: AtomicU64,
}

#[async_trait]
impl ReindexTarget for RecordingWriter {
    async fn delete_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<u64> {
        self.inner
            .delete_search_entries(tenant, resource_type, resource_id)
            .await
    }
    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource: &StoredResource,
    ) -> StorageResult<usize> {
        self.inner.write_search_entries(tenant, resource).await
    }
    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
        self.inner.clear_search_index(tenant).await
    }
    async fn begin_bulk_index_rebuild(&self) -> StorageResult<()> {
        self.inner.begin_bulk_index_rebuild().await
    }
    async fn end_bulk_index_rebuild(&self) -> StorageResult<()> {
        self.inner.end_bulk_index_rebuild().await
    }
    async fn write_search_entries_page(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
    ) -> Vec<StorageResult<usize>> {
        self.inner.write_search_entries_page(tenant, resources).await
    }
    async fn write_search_entries_page_timed(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        stats: &mut ReindexPageStats,
    ) -> Vec<StorageResult<usize>> {
        let results = self
            .inner
            .write_search_entries_page_timed(tenant, resources, stats)
            .await;
        let ok: u64 = results.iter().filter_map(|r| r.as_ref().ok()).map(|n| *n as u64).sum();
        self.written.fetch_add(ok, Ordering::SeqCst);
        results
    }
}

#[tokio::test]
async fn mongodb_integration_reindex_capped_run_bounds_every_page() {
    let Some(backend) = create_id_phase_backend("reindex_capped_run_bounds").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant_id = "tenant-capped-run-bounds";
    let tenant = create_tenant(tenant_id);
    let fixture = provenance_fixture(24);
    for (id, resource) in &fixture {
        backend
            .create_or_update(&tenant, "Provenance", id, resource.clone(), FhirVersion::default())
            .await
            .unwrap();
    }
    settle_into_id_phase().await;
    let db = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name);
    let sizes = reindex_resource_row_sizes(&db, tenant_id, "Provenance").await;
    let mut by_size = sizes.clone();
    by_size.sort_by_key(|(_, size)| *size);
    let cap: u64 = by_size.iter().take(3).map(|(_, size)| *size).sum();

    let source = Arc::new(RecordingSource {
        inner: backend.clone(),
        sizes: sizes.into_iter().collect(),
        pages: Mutex::new(Vec::new()),
    });
    let writer = Arc::new(RecordingWriter {
        inner: backend.clone(),
        written: AtomicU64::new(0),
    });
    let operation = ReindexOperation::with_parts(
        source.clone(),
        vec![writer.clone()],
        backend.tenant_registries().clone(),
    );
    let request = ReindexRequest::for_types(["Provenance"])
        .with_batch_size(100)
        .with_batch_bytes(cap);
    let job_id = operation.start(tenant, request, None).await.unwrap();
    let progress = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let progress = operation.get_progress(&job_id).await.unwrap();
            if progress.status.is_finished() {
                break progress;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("reindex did not finish within 60s");

    assert_eq!(progress.status, ReindexStatus::Completed);
    assert!(progress.errors.is_empty(), "{:?}", progress.errors);
    assert_eq!(progress.processed_resources, 24);
    for (resources, bytes) in source.pages.lock().unwrap().iter() {
        assert!(*resources <= 3, "page held {resources} resources");
        assert!(
            *bytes <= cap || *resources == 1,
            "page held {resources} resources totalling {bytes} bytes, over cap {cap}"
        );
    }
    let own = db
        .collection::<Document>("search_index")
        .count_documents(doc! { "tenant_id": tenant_id, "resource_type": "Provenance" })
        .await
        .unwrap();
    let contained = db
        .collection::<Document>("search_index_contained")
        .count_documents(doc! { "tenant_id": tenant_id, "resource_type": "Provenance" })
        .await
        .unwrap();
    assert_eq!(progress.entries_created, own + contained);
    assert_eq!(writer.written.load(Ordering::SeqCst), progress.entries_created);
}

#[tokio::test]
async fn mongodb_integration_reindex_fetch_capped_page_never_spans_the_catch_up_boundary() {
    let Some(backend) = create_id_phase_backend("reindex_capped_catch_up_boundary").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant_id = "tenant-capped-catch-up-boundary";
    let tenant = create_tenant(tenant_id);
    for (id, k) in [("p01", 1usize), ("p02", 2), ("p03", 3), ("p04", 4), ("p05", 5), ("p06", 6)] {
        backend
            .create_or_update(
                &tenant,
                "Patient",
                id,
                json!({"resourceType": "Patient", "id": id, "name": [{"family": "X".repeat(100 * k)}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    // The id-phase fixture rule: sleep past the margin so every seeded row is
    // walked by the id phase, not folded into round 1.
    settle_into_id_phase().await;

    let db = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name);
    let sizes = reindex_resource_row_sizes(&db, tenant_id, "Patient").await;
    let cap: u64 = sizes
        .iter()
        .filter(|(id, _)| id != "p06")
        .map(|(_, size)| *size)
        .sum();

    let page1 = backend
        .fetch_resources_page_capped(&tenant, "Patient", None, 6, cap)
        .await
        .unwrap();
    let page1_ids: Vec<String> = page1.resources.iter().map(|r| r.id().to_string()).collect();
    assert_eq!(page1_ids, ["p01", "p02", "p03", "p04", "p05"]);
    assert!(page1
        .next_cursor
        .as_deref()
        .is_some_and(|c| c.starts_with("v2|i|")));

    // A racing update to a row already on page 1, after it was written.
    backend
        .create_or_update(
            &tenant,
            "Patient",
            "p02",
            json!({"resourceType": "Patient", "id": "p02", "name": [{"family": "Updated"}]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let mut cursor = page1.next_cursor;
    let mut saw_id_phase_p06 = false;
    let mut p02_round_hits = 0u32;
    let mut p02_round_family: Option<String> = None;
    let mut seen_ids: Vec<String> = page1_ids.clone();
    {
        let unique: std::collections::HashSet<&String> = page1_ids.iter().collect();
        assert_eq!(unique.len(), page1_ids.len(), "page 1 holds an id twice");
    }
    for _ in 0..20 {
        let page = backend
            .fetch_resources_page_capped(&tenant, "Patient", cursor.as_deref(), 6, cap)
            .await
            .unwrap();
        let ids: Vec<String> = page.resources.iter().map(|r| r.id().to_string()).collect();
        {
            let unique: std::collections::HashSet<&String> = ids.iter().collect();
            assert_eq!(unique.len(), ids.len(), "a page holds an id twice: {ids:?}");
        }
        if !ids.is_empty() {
            if let Some(next) = &page.next_cursor {
                if next.starts_with("v2|i|") {
                    assert_eq!(ids, ["p06"], "the id phase's last page must be exactly p06");
                    saw_id_phase_p06 = true;
                } else if next.starts_with("v2|c|") {
                    if let Some(p02) = page.resources.iter().find(|r| r.id() == "p02") {
                        p02_round_hits += 1;
                        p02_round_family = p02.content()["name"][0]["family"]
                            .as_str()
                            .map(str::to_string);
                    }
                }
            }
        }
        seen_ids.extend(ids);
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert!(saw_id_phase_p06, "p06 must be walked by the id phase, not folded into a capped page");
    assert_eq!(p02_round_hits, 1, "p02 must be re-walked by exactly one catch-up round page");
    assert_eq!(
        p02_round_family.as_deref(),
        Some("Updated"),
        "p02 must come back with its updated content"
    );

    let mut counts = std::collections::HashMap::new();
    for id in &seen_ids {
        *counts.entry(id.clone()).or_insert(0) += 1;
    }
    for id in ["p01", "p03", "p04", "p05", "p06"] {
        assert_eq!(counts.get(id), Some(&1), "{id} must appear exactly once");
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_pipeline`
Expected: all 7 tests in the file pass (the 4 from Task 1b plus these 3). If any of the 3 new tests fails, treat it as a defect in Task 1b's `fetch_reindex_page`/`reindex_find_page` — do not special-case the test to make it pass.

- [ ] **Step 3: Commit**

```bash
git add crates/persistence/tests/mongodb/reindex_pipeline.rs
git commit -m "$(cat <<'EOF'
test(mongodb): cover Provenance-shaped pages, the driver, and catch-up races (#1499)

Extra coverage for the byte-cap mechanism beyond Task 1b's boundary
tests: a large-fanout Provenance fixture (the corpus's actual worst
case), a full ReindexOperation run through fake source/writer wrappers
that record per-page counts and bytes, and a page the cap stops right
at a catch-up-round boundary while a covered row races an update.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Fix the two batch-size bugs (D12)

**Files:**
- Modify: `crates/rest/src/handlers/reindex.rs:71` (add `batch_size_param` next to `check_reindex_scope`), `:131-132` (`request.batch_size = size.max(1) as u32;` → the new helper), `mod tests` at `:260` (new test)
- Modify: `crates/persistence/src/search/reindex.rs:2211-2229` (`run_reindex`'s paging loop: add `page_limit`, use it in the capped-fetch call), new `page_limit_tests` module

**Interfaces:**
- Produces: `fn batch_size_param(size: u64) -> u32` (private, `crates/rest/src/handlers/reindex.rs`); `let page_limit: u32` local to `run_reindex`'s paging loop (`crates/persistence/src/search/reindex.rs`, no public signature change).

- [ ] **Step 1: Write the failing tests**

In `crates/rest/src/handlers/reindex.rs`, inside `mod tests` (starts at `:260`):

```rust
    #[test]
    fn batch_size_param_clamps_and_saturates() {
        assert_eq!(batch_size_param(0), 1);
        assert_eq!(batch_size_param(1), 1);
        assert_eq!(batch_size_param(1000), 1000);
        assert_eq!(batch_size_param(4_294_967_296), u32::MAX);
        assert_eq!(batch_size_param(u64::MAX), u32::MAX);
    }
```

In `crates/persistence/src/search/reindex.rs`, add a new module near the end of the file (after the existing `#[cfg(test)] mod tests` block, as a sibling — clippy's `-A clippy::items_after_test_module` allowance covers this):

```rust
#[cfg(all(test, feature = "sqlite"))]
mod page_limit_tests {
    use super::*;
    use crate::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
    use crate::core::ResourceStorage;
    use crate::tenant::{TenantId, TenantPermissions};
    use std::sync::Mutex;

    struct LimitRecordingSource {
        inner: Arc<SqliteBackend>,
        limits: Mutex<Vec<u32>>,
    }

    #[async_trait]
    impl ReindexSource for LimitRecordingSource {
        async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
            self.inner.list_resource_types(tenant).await
        }
        async fn count_resources(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
        ) -> StorageResult<u64> {
            self.inner.count_resources(tenant, resource_type).await
        }
        async fn fetch_resources_page(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            cursor: Option<&str>,
            limit: u32,
        ) -> StorageResult<ResourcePage> {
            self.limits.lock().unwrap().push(limit);
            self.inner
                .fetch_resources_page(tenant, resource_type, cursor, limit)
                .await
        }
        async fn fetch_resources_page_capped(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            cursor: Option<&str>,
            limit: u32,
            max_bytes: u64,
        ) -> StorageResult<ResourcePage> {
            self.limits.lock().unwrap().push(limit);
            self.inner
                .fetch_resources_page_capped(tenant, resource_type, cursor, limit, max_bytes)
                .await
        }
    }

    #[tokio::test]
    async fn paging_passes_at_least_one_as_the_page_limit() {
        let backend = Arc::new(
            SqliteBackend::with_config(":memory:", SqliteBackendConfig::default()).unwrap(),
        );
        backend.init_schema().unwrap();
        let tenant = TenantContext::new(TenantId::new("tenant-page-limit"), TenantPermissions::full_access());
        for i in 0..3 {
            backend
                .create_or_update(
                    &tenant,
                    "Patient",
                    &format!("p{i}"),
                    serde_json::json!({"resourceType": "Patient", "id": format!("p{i}")}),
                    helios_fhir::FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        let source = Arc::new(LimitRecordingSource {
            inner: backend.clone(),
            limits: Mutex::new(Vec::new()),
        });
        let operation = ReindexOperation::with_parts(
            source.clone(),
            vec![backend.clone() as Arc<dyn ReindexTarget>],
            backend.tenant_registries().clone(),
        );
        let request = ReindexRequest::for_types(["Patient"]).with_batch_size(0);
        let job_id = operation.start(tenant, request, None).await.unwrap();
        let progress = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let progress = operation.get_progress(&job_id).await.unwrap();
                if progress.status.is_finished() {
                    break progress;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("job did not finish");
        assert_eq!(progress.status, ReindexStatus::Completed);
        assert_eq!(progress.processed_resources, 3);
        let limits = source.limits.lock().unwrap();
        assert!(!limits.is_empty());
        assert!(limits.iter().all(|&l| l == 1), "{limits:?}");
    }
}
```

`crate::backends::sqlite::{SqliteBackend, SqliteBackendConfig}` and `crate::tenant::{TenantId, TenantPermissions}` are the paths this same crate's other test modules already use for this exact purpose (e.g. `crates/persistence/src/search/chain_resolver.rs:879`, `list_resolver.rs:153`, `seeder.rs:690`) — do not "adjust" them. `crate::core::ResourceStorage` is required because `create_or_update` is a `ResourceStorage` trait method; `crate::FhirVersion` does not exist (this crate's `lib.rs` re-exports `StorageError, StorageResult, TenantContext, TenantId, TenantPermissions, Pagination, SearchQuery, StoredResource`, not `FhirVersion`) — use `helios_fhir::FhirVersion::default()`, exactly as `reindex.rs`'s own existing tests already do at `:2618`, `:3997`, `:4181`, `:4368`. `async_trait` (the file imports `async_trait::async_trait` at `:13`, reached via `use super::*;`), `Arc`, `StorageResult` and `TenantContext` are already in scope the same way.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p helios-rest --lib handlers::reindex::tests::batch_size_param_clamps_and_saturates`
Expected: compile error, `batch_size_param` not found.

Run: `cargo test -p helios-persistence --features sqlite --lib search::reindex::page_limit_tests`
Expected: **FAIL at `assert_eq!(progress.processed_resources, 3)` (left: `0`).** At HEAD, `run_reindex`'s paging loop passes `request.batch_size` straight to `fetch_resources_page_capped` with no clamp (confirmed at `crates/persistence/src/search/reindex.rs:2226` — the named-resources path already clamps at `:2172`, but the paging path does not). `ReindexRequest::for_types(["Patient"]).with_batch_size(0)` therefore calls SQLite's `fetch_resources_page_capped` with `limit = 0`, which runs `LIMIT 0` (`sqlite/storage.rs:3806`, `params: vec![.., Box::new(limit as i64)]`) and returns zero rows; SQLite's own cursor logic then sets `next_cursor = None` because its guard is `if limit > 0 && (capped || scanned == limit as usize)` (`sqlite/storage.rs:3892`, false when `limit == 0`), so the page is empty with no continuation and the type ends immediately with `processed_resources == 0` and one recorded limit, `[0]`. This is the correct starting failure for TDD — do not "fix" it by weakening the assertion.

- [ ] **Step 3: Implement**

In `crates/rest/src/handlers/reindex.rs`, next to `check_reindex_scope` (`:71`):

```rust
/// `batchSize` as a page size: at least 1, and saturating instead of wrapping
/// (`4294967296 as u32` is 0, which reindexed nothing, #1499).
fn batch_size_param(size: u64) -> u32 {
    u32::try_from(size).unwrap_or(u32::MAX).max(1)
}
```

Replace (`:131-132`):

```rust
                        request.batch_size = size.max(1) as u32;
```

with:

```rust
                        request.batch_size = batch_size_param(size);
```

In `crates/persistence/src/search/reindex.rs`, in `run_reindex`'s paging loop, add immediately before `let mut cursor: Option<String> = None;` (currently `:2212`):

```rust
            let page_limit = request.batch_size.max(1);
            let mut cursor: Option<String> = None;
```

and change the capped-fetch call (currently `:2221-2229`) from `request.batch_size` to `page_limit`:

```rust
                let fetched = source
                    .fetch_resources_page_capped(
                        &tenant,
                        resource_type,
                        cursor.as_deref(),
                        page_limit,
                        request.batch_bytes,
                    )
                    .await;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p helios-rest --lib handlers::reindex::tests::batch_size_param_clamps_and_saturates`
Expected: pass.

Run: `cargo test -p helios-persistence --features sqlite --lib search::reindex::page_limit_tests`
Expected: pass — `processed_resources == 3`, and every recorded limit is exactly `1`.

Run the whole `run_reindex` paging suite to confirm no regression: `cargo test -p helios-persistence --features sqlite --lib search::reindex::`
Expected: unchanged pass count plus the new test.

- [ ] **Step 5: Commit**

```bash
git add crates/rest/src/handlers/reindex.rs crates/persistence/src/search/reindex.rs
git commit -m "$(cat <<'EOF'
fix(reindex): saturate batchSize instead of wrapping, clamp the page limit (#1499)

POST $reindex's batchSize=4294967296 silently became 0 (u32 wraps), which
reindexed nothing; it now saturates to u32::MAX. run_reindex's paging
loop now clamps request.batch_size to at least 1 before passing it to
fetch_resources_page_capped, matching the named-resources path, which
already clamped. Without this, batch_size=0 made SQLite's LIMIT 0
return an empty page and end the type with nothing processed.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Server default → 32 MiB, the six-place doc sweep, and the Knobs section

**Files:**
- Modify: `crates/rest/src/config.rs:1315-1321` (doc comment + `default_value`), `:1597`, `:1867` (the two `reindex_batch_bytes: 0,` literals), `mod tests` at `:2465-2475` (defaults test doc comment + assertion)
- Modify: `README.md:309`
- Modify: `crates/hfs/README.md:113`
- Modify: `book/src/configuration/environment-variables.md:65`
- Modify: `.claude/skills/bulk-data-submit/SKILL.md:145`
- Modify: `.agents/skills/bulk-data-submit/SKILL.md:141`
- Modify: `docs/mongodb/bulk-import-sizing.md` (Knobs section — conditional, see Step 6)

**Interfaces:**
- Produces: `ServerConfig::reindex_batch_bytes` defaults to `33554432` from every construction path (`try_parse_from` with no env var, `Default`, `for_testing`). No signature changes.

- [ ] **Step 1: Write the failing test**

In `crates/rest/src/config.rs`, update the existing `test_elasticsearch_rebuild_knob_defaults` (`:2465-2475`) — change its doc comment and its one assertion:

```rust
    /// Every default reproduces today's behavior, except the reindex byte cap
    /// (32 MiB, #1499 — the new default is intentional, not a regression): a
    /// 30 s client timeout, the 1000-row deferred rebuild page, and a rebuild
    /// refresh that follows `HFS_ELASTICSEARCH_WRITE_REFRESH`.
    #[test]
    fn test_elasticsearch_rebuild_knob_defaults() {
        let parsed = ServerConfig::try_parse_from(["rest-server"]).unwrap();
        for config in [parsed, ServerConfig::default(), ServerConfig::for_testing()] {
            assert_eq!(config.elasticsearch_request_timeout_ms, 30_000);
            assert_eq!(config.elasticsearch_bulk_max_bytes, 10 * 1024 * 1024);
            assert_eq!(config.elasticsearch_reindex_refresh, None);
            assert_eq!(config.reindex_batch_size, 1000);
            assert_eq!(config.reindex_batch_bytes, 32 * 1024 * 1024);
            assert_eq!(config.elasticsearch_bulk_concurrency, 1);
        }
    }
```

- [ ] **Step 2: Run the test to see it fail**

Run: `cargo test -p helios-rest --lib config::tests::test_elasticsearch_rebuild_knob_defaults`
Expected: FAIL — `assert_eq!(config.reindex_batch_bytes, 32 * 1024 * 1024)` sees `0` from all three construction paths.

- [ ] **Step 3: Implement**

In `crates/rest/src/config.rs`, replace the field's doc comment and `#[arg(..)]` (`:1315-1321`):

```rust
    /// Byte cap of one page of the automatic rebuild, on top of
    /// `HFS_REINDEX_BATCH_SIZE`. `0` means count only; with a cap set, a page
    /// of ~108 KB `Provenance` resources stays near the cap instead of
    /// holding ~108 MB in memory (#1125). Honoured by the SQLite source (a
    /// page may exceed the cap by one resource) and by the PostgreSQL and
    /// MongoDB sources (a page never exceeds the cap unless it holds a single
    /// resource, #1499); the Elasticsearch and S3 sources page by count only.
    /// Defaults to 32 MiB so an automatic rebuild is bounded even when an
    /// operator never sets it; `ReindexRequest` and `AutomaticRunOptions`
    /// keep a library default of `0`, so manual `$reindex` is unchanged.
    #[arg(long, env = "HFS_REINDEX_BATCH_BYTES", default_value = "33554432")]
    pub reindex_batch_bytes: u64,
```

Replace both struct-literal sites (`:1597`, `:1867`):

```rust
            reindex_batch_bytes: 32 * 1024 * 1024,
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p helios-rest --lib config::tests::test_elasticsearch_rebuild_knob_defaults`
Expected: pass.

Run the whole config test module to confirm no other default assertion regressed: `cargo test -p helios-rest --lib config::tests::`
Expected: unchanged pass count plus no new failures.

- [ ] **Step 5: Sweep the five documentation places**

`README.md:309`, replace:

```
| `HFS_REINDEX_BATCH_BYTES` | `0` | Byte cap on one page of the automatic rebuild (`0` = count only); honoured by the SQLite source |
```

with:

```
| `HFS_REINDEX_BATCH_BYTES` | `33554432` (32 MiB) | Byte cap on one page of the automatic rebuild (`0` = count only). Honoured by the SQLite source (a page may exceed the cap by one resource) and by the PostgreSQL and MongoDB sources (a page never exceeds the cap unless it holds a single resource); the Elasticsearch and S3 sources page by count only |
```

`book/src/configuration/environment-variables.md:65` — identical current text to the `README.md` row; apply the identical replacement.

`crates/hfs/README.md:113`, replace:

```
| `HFS_REINDEX_BATCH_BYTES` | `0` | Byte cap on one page of the automatic rebuild, on top of `HFS_REINDEX_BATCH_SIZE`. `0` means count only. With a cap set, a page of ~108 KB `Provenance` resources ends at the first one that crosses it instead of holding ~108 MB in memory. Honoured by the SQLite source; other sources page by count only. |
```

with:

```
| `HFS_REINDEX_BATCH_BYTES` | `33554432` (32 MiB) | Byte cap on one page of the automatic rebuild, on top of `HFS_REINDEX_BATCH_SIZE`. `0` means count only; with a cap set, a page of ~108 KB `Provenance` resources stays near the cap instead of holding ~108 MB in memory. Honoured by the SQLite source (a page may exceed the cap by one resource) and by the PostgreSQL and MongoDB sources (a page never exceeds the cap unless it holds a single resource); the Elasticsearch and S3 sources page by count only. |
```

`.claude/skills/bulk-data-submit/SKILL.md:145` and `.agents/skills/bulk-data-submit/SKILL.md:141` (identical text in both — `.claude` edits are always mirrored in `.agents`), replace the `HFS_REINDEX_BATCH_BYTES` clause inside the "Rebuild knobs (#1125)" bullet:

```
`HFS_REINDEX_BATCH_BYTES` (default `0` = off) caps a page by bytes so ~108 KB `Provenance` resources do not make a ~108 MB page.
```

with:

```
`HFS_REINDEX_BATCH_BYTES` (default `33554432` (32 MiB); `0` = count only) caps a page by bytes so ~108 KB `Provenance` resources do not make a ~108 MB page. Honoured by the SQLite source (a page may exceed the cap by one resource) and by the PostgreSQL and MongoDB sources (a page never exceeds the cap unless it holds a single resource); the Elasticsearch and S3 sources page by count only.
```

leaving the rest of that bullet (the `sqlite-es` refresh-pair sentence) unchanged. This is S3 §4.4's exact sentence, not a paraphrase — the previous draft of this step dropped the SQLite/PostgreSQL-vs-MongoDB distinction and said "off" where the design says "count only"; use the wording above.

- [ ] **Step 6: Add the Knobs section to `docs/mongodb/bulk-import-sizing.md` (spec §6)**

Spec §6 says "The knobs section lands with PR2a/PR3", and the PR-docs plan deliberately left item 6 ("Knobs") out of `docs/mongodb/bulk-import-sizing.md` for this reason (`docs/superpowers/plans/2026-09-23-1403-pr-docs.md`'s Self-Review: "S5 §3.2 items 1, 2, 3 (0.2.x rule only), 4, 5, 7, 8 | Task 2 (item 6 'Knobs' deliberately excluded, per S5 §3.4)").

Precondition — this file is created by the PR-docs plan, not this one:

```bash
test -f docs/mongodb/bulk-import-sizing.md && echo EXISTS || echo MISSING
```

Expected: `EXISTS`. If it prints `MISSING`, PR-docs has not merged yet — skip only this step (do not create the file here) and continue with Steps 7-8 without it; Step 7's Knobs-section check and Step 8's `git add` of this file are already conditional on Step 6 having applied. Note in Task 6 Step 8's PR body that the Knobs section follows in PR-docs (see that step).

Insert a new section between `## Windows / Docker Desktop (WSL2)` and `## Watching a rebuild` (both headings already exist verbatim in the file per the PR-docs plan):

```markdown
## Knobs

- `HFS_REINDEX_BATCH_BYTES`: the server default is `33554432` (32 MiB) on every backend. It bounds pages of large resources such as `Provenance` (~108 KB each); keep it. `0` means count-only pages.
- `HFS_REINDEX_BATCH_SIZE`: not the bottleneck on MongoDB; keep `1000`.
- `HFS_BULK_SUBMIT_BULK_INDEX_REBUILD`: SQLite only.
```

- [ ] **Step 7: Verify the sweep is complete**

Run:
```bash
grep -rniE 'honoured by the sqlite source(;| \|)|other sources page by count only|default `0` = off|\| `0` \| Byte cap|ends at the first one that crosses' README.md crates/hfs/README.md book/src/configuration/environment-variables.md .claude/skills/bulk-data-submit/SKILL.md .agents/skills/bulk-data-submit/SKILL.md crates/rest/src/config.rs
```

Expected: no matches (every stale phrase was replaced by the cross-backend sentence and the new default; the pattern is case-insensitive so it also catches `crates/hfs/README.md`'s capitalized "Honoured"). `crates/persistence/README.md:1528` only names the two variables and links to the hfs README, so it needs no edit — confirm with `grep -n "HFS_REINDEX_BATCH_BYTES" crates/persistence/README.md` that its line still reads that way.

Run (only if Step 6 applied):
```bash
grep -c "## Knobs" docs/mongodb/bulk-import-sizing.md
```
Expected: `1`.

- [ ] **Step 8: Commit**

```bash
git add crates/rest/src/config.rs README.md crates/hfs/README.md book/src/configuration/environment-variables.md .claude/skills/bulk-data-submit/SKILL.md .agents/skills/bulk-data-submit/SKILL.md
```

If Step 6 applied (the file existed), also: `git add docs/mongodb/bulk-import-sizing.md`

```bash
git commit -m "$(cat <<'EOF'
feat(config): default HFS_REINDEX_BATCH_BYTES to 32 MiB on every backend (#1499)

The server-side default moves from 0 (unbounded) to 33554432, so an
automatic rebuild is bounded even when an operator never sets the
variable. ReindexRequest and AutomaticRunOptions keep a library
default of 0, so manual $reindex is unchanged. Sweeps the six stale
doc locations #1499 listed, which described the knob as SQLite-only,
and adds the Knobs section spec §6 assigns to PR2a.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: The automatic hook gets the server default when unset

**Files:**
- Modify: `crates/persistence/src/search/reindex.rs` (`impl ReindexOnFinish`, add a getter after `with_bulk_index_rebuild`)
- Modify: `crates/hfs/src/main.rs:1916-1948` (extract `build_automatic_reindex_hook` from `automatic_reindex_hook_with_ledger`; new test in `mod tests`, `:3760`)

**Interfaces:**
- Produces: `pub fn batch_bytes(&self) -> u64` on `ReindexOnFinish`; `fn build_automatic_reindex_hook(op: Arc<ReindexOperation>, config: &ServerConfig, ledger: Option<Arc<dyn helios_persistence::search::DeferredReindexLedger>>) -> helios_persistence::search::ReindexOnFinish` (private, `crates/hfs/src/main.rs`, same `#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mongodb", feature = "elasticsearch"))]` gate as today's `automatic_reindex_hook_with_ledger`).

- [ ] **Step 1: Write the failing test**

In `crates/hfs/src/main.rs`, inside `mod tests` (`:3760`, which today opens with `use super::*; use helios_audit::AuditConfig; use helios_rest::ServerConfig;` — confirmed at HEAD, no `clap::Parser` import anywhere in the file), add a new subsection after `test_sqlite_es_reindex_skips_an_offloaded_primary` and before the `// ── create_sqlite_backend() ───` heading:

```rust
    // ── Automatic reindex hook wiring (#1499) ──────────────────────

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_automatic_reindex_hook_gets_the_server_default_batch_bytes_when_unset() {
        use clap::Parser;

        let config = ServerConfig::try_parse_from(["rest-server"]).unwrap();
        assert_eq!(
            config.reindex_batch_bytes,
            32 * 1024 * 1024,
            "HFS_REINDEX_BATCH_BYTES server default (#1499)"
        );

        let backend = Arc::new(
            create_sqlite_backend(&ServerConfig {
                database_url: Some(":memory:".to_string()),
                ..Default::default()
            })
            .unwrap(),
        );
        let registries = backend.tenant_registries().clone();
        let op = Arc::new(ReindexOperation::new(backend, registries));

        let hook = build_automatic_reindex_hook(op, &config, None);
        assert_eq!(hook.batch_bytes(), 32 * 1024 * 1024);
    }
```

`ServerConfig::try_parse_from` is a `clap::Parser` trait method; `main.rs` never imports `clap::Parser` (it builds its own config with `ServerConfig::try_from_env()` at `:1270`, a different method), so the `use clap::Parser;` inside this test's body is required — `clap = { version = "4.0", features = ["derive", "env"] }` is already a dependency of `helios-hfs` (`crates/hfs/Cargo.toml:83`). Do not switch to `try_from_env()`: it parses the test binary's own `argv`, not a fixed empty argument list, so its result is not deterministic under `cargo test`.

- [ ] **Step 2: Run the test to see it fail**

Run: `cargo test -p helios-hfs --bin hfs test_automatic_reindex_hook_gets_the_server_default_batch_bytes_when_unset`
Expected: compile error — `build_automatic_reindex_hook` does not exist, and `ReindexOnFinish` has no method `batch_bytes`.

- [ ] **Step 3: Implement**

In `crates/persistence/src/search/reindex.rs`, in `impl ReindexOnFinish`, immediately after `with_bulk_index_rebuild`:

```rust
    /// The byte cap this hook's rebuild runs use (`0` = count only). Exposed
    /// so a caller — or a test, without downcasting the `Arc<dyn
    /// DeferredReindexHook>` this type is usually erased behind — can confirm
    /// what `with_batch_bytes` actually set (#1499).
    pub fn batch_bytes(&self) -> u64 {
        self.options.batch_bytes
    }
```

In `crates/hfs/src/main.rs`, replace `automatic_reindex_hook_with_ledger` (`:1926-1948`) with:

```rust
/// Builds the deferred bulk-submit hook using the existing submit-worker
/// concurrency as the per-process automatic reindex limit, plus where to clear
/// the persisted "this manifest still owes a rebuild" marker when a generation
/// finishes (#1125). Without a ledger (`None`) nothing is recorded and a
/// restart cannot resume, as before.
///
/// Split into this function and [`automatic_reindex_hook_with_ledger`] so a
/// test can read the concrete `ReindexOnFinish`'s `batch_bytes()` (#1499)
/// without downcasting the trait object every other caller uses.
///
/// Gated exactly like [`wire_reindex`], which produces the `op` every caller
/// passes in: any build with a reindex target. Keep the two in step rather
/// than naming individual backends here — a narrower gate breaks the builds
/// that leave that backend out (#1291).
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "elasticsearch"
))]
fn build_automatic_reindex_hook(
    op: Arc<ReindexOperation>,
    config: &ServerConfig,
    ledger: Option<Arc<dyn helios_persistence::search::DeferredReindexLedger>>,
) -> helios_persistence::search::ReindexOnFinish {
    let hook = helios_persistence::search::ReindexOnFinish::with_max_concurrency(
        op,
        config.bulk_submit.worker_concurrency as usize,
    )
    .with_batch_size(config.reindex_batch_size)
    .with_batch_bytes(config.reindex_batch_bytes)
    .with_bulk_index_rebuild(config.bulk_submit.bulk_index_rebuild);
    match ledger {
        Some(ledger) => hook.with_ledger(ledger),
        None => hook,
    }
}

/// Builds [`build_automatic_reindex_hook`]'s hook and erases it behind
/// `Arc<dyn DeferredReindexHook>`, the shape every wiring site outside tests
/// needs. See that function's doc comment for what it configures and why the
/// two are split (#1499).
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "elasticsearch"
))]
fn automatic_reindex_hook_with_ledger(
    op: Arc<ReindexOperation>,
    config: &ServerConfig,
    ledger: Option<Arc<dyn helios_persistence::search::DeferredReindexLedger>>,
) -> Arc<dyn helios_persistence::core::DeferredReindexHook> {
    Arc::new(build_automatic_reindex_hook(op, config, ledger))
}
```

Re-read `:1916-1948` before editing to confirm this still matches `main`'s exact current wording (PR0/PR1 may have touched neighbouring lines) — the doc comment above reproduces today's first paragraph verbatim and gives `build_automatic_reindex_hook` and `automatic_reindex_hook_with_ledger` each their own complete comment, rather than splitting one comment awkwardly across both.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p helios-hfs --bin hfs test_automatic_reindex_hook_gets_the_server_default_batch_bytes_when_unset`
Expected: pass.

Run the whole `mod tests` in `main.rs` to confirm no regression: `cargo test -p helios-hfs --bin hfs`
Expected: unchanged pass count plus the new test; `test_sqlite_es_reindex_skips_an_offloaded_primary` and the other `automatic_reindex_hook_with_ledger` callers (the `standalone_ops`/`composite_ops` wiring) are unaffected since the wrapper's behaviour is identical.

- [ ] **Step 5: Commit**

```bash
git add crates/persistence/src/search/reindex.rs crates/hfs/src/main.rs
git commit -m "$(cat <<'EOF'
test(hfs): pin that the automatic reindex hook gets the server default (#1499)

automatic_reindex_hook_with_ledger's construction is split into
build_automatic_reindex_hook, which returns the concrete ReindexOnFinish
instead of an erased Arc<dyn DeferredReindexHook>, and ReindexOnFinish
gains a batch_bytes() getter. A new test confirms the automatic hook
receives HFS_REINDEX_BATCH_BYTES's new 32 MiB default when the operator
never sets the variable.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: Whole-suite verification and opening the PR

**Files:** none new.

- [ ] **Step 1: Full MongoDB integration suite**

Run:
```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
cargo test -p helios-persistence --features mongodb --test mongodb_tests
```
Expected: passes, with 7 new tests from Tasks 1b-2 on top of whatever PR0/PR1 left the count at.

- [ ] **Step 2: Full persistence lib tests, both feature sets touched by this plan**

Run:
```
cargo test -p helios-persistence --features mongodb --lib
cargo test -p helios-persistence --features sqlite --lib
```
Expected: pass, including `backends::mongodb::storage::reindex_page_cap_tests` and `search::reindex::page_limit_tests`.

- [ ] **Step 3: rest and hfs crates, plus the existing SQLite and PostgreSQL capped-fetch checks (spec §4.3)**

Run:
```
cargo test -p helios-rest --lib
cargo test -p helios-hfs --bin hfs
cargo test -p helios-persistence --lib backends::sqlite::storage::tests::fetch_resources_page_capped
```
Expected: pass, including `config::tests::test_elasticsearch_rebuild_knob_defaults`, `handlers::reindex::tests::batch_size_param_clamps_and_saturates`, `test_automatic_reindex_hook_gets_the_server_default_batch_bytes_when_unset`, and SQLite's own `fetch_resources_page_capped_pages_one_resource_at_a_time_under_a_byte_cap`/`fetch_resources_page_capped_returns_a_resource_larger_than_the_cap` (both matched by that filter).

Run (requires Docker):
```
cargo test -p helios-persistence --features postgres --test postgres_tests postgres_integration_reindex_fetch_capped_boundaries
```
Expected: pass. This plan does not modify PostgreSQL's capped fetch; this run only confirms nothing here regressed it.

- [ ] **Step 4: fmt**

Run:
```
cargo fmt -- crates/persistence/src/backends/mongodb/storage.rs crates/persistence/src/search/reindex.rs crates/rest/src/handlers/reindex.rs crates/rest/src/config.rs crates/hfs/src/main.rs crates/persistence/tests/mongodb_tests.rs crates/persistence/tests/mongodb/reindex_pipeline.rs
```
Expected: no diff, or a diff that only reformats this plan's own additions — review and re-stage if it changes anything.

- [ ] **Step 5: clippy, with CI's exact allowances, plus the #1291 single-backend `hfs` matrix**

Run:
```
cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation
cargo clippy -p helios-rest --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation
cargo clippy -p helios-hfs --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation
```
Expected: clean. `ReindexFoundPage`'s fields are read by `fetch_reindex_page` by this point, so no dead-code warning should remain from Task 1a. (Without the `-A` list, `-p helios-rest` fails on pre-existing `crates/rest/src/handlers/reindex.rs:108-109`'s `clippy::field_reassign_with_default` — that lint is not this plan's to fix.)

Run the #1291 single-backend `hfs` matrix (Task 5 changed a feature-gated function in `main.rs`):
```bash
for features in R4,sqlite,postgres R4,postgres R4,mongodb R4,s3 R4,s3,elasticsearch; do
  cargo clippy -p helios-hfs --no-default-features --features "$features" -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation
done
```
Expected: clean for every feature set — `build_automatic_reindex_hook`/`automatic_reindex_hook_with_ledger` are gated identically to the code they replace, so no combination should lose or gain a warning.

- [ ] **Step 6: Confirm the doc sweep and the issue link**

Run: `grep -rln "HFS_REINDEX_BATCH_BYTES" README.md crates/hfs/README.md book/src/configuration/environment-variables.md .claude/skills/bulk-data-submit/SKILL.md .agents/skills/bulk-data-submit/SKILL.md crates/rest/src/config.rs`
Expected: all six files listed; spot-check each still names the new default and the cross-backend behaviour (Task 4, Step 7 already checked the stale phrase is gone).

- [ ] **Step 7: Commit any fix-ups**

Only if Steps 1-6 required changes:

```bash
git add <the specific files fmt/clippy touched>
git commit -m "$(cat <<'EOF'
chore(mongodb): fmt/clippy fix-ups for the batch-bytes plan (#1499)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 8: Open the PR (do not merge)**

If Task 4 Step 6 printed `MISSING` and was skipped, add a fourth bullet to the `## Summary` section below, immediately after the "Sweeps the six doc locations..." bullet:

```markdown
- The `docs/mongodb/bulk-import-sizing.md` "Knobs" section spec §6 assigns
  to this PR was not added: that file does not exist on `main` yet.
  Knobs section follows in PR-docs.
```

Otherwise, leave the `## Summary` section exactly as written below.

```bash
git push -u origin perf/1403-pr2a-mongodb-batch-bytes
gh pr create --base main --head perf/1403-pr2a-mongodb-batch-bytes \
  --title "perf(mongodb): honour HFS_REINDEX_BATCH_BYTES; 32 MiB server default (#1403 PR2a)" \
  --body "$(cat <<'EOF'
## Summary
- `MongoBackend` now overrides `ReindexSource::fetch_resources_page_capped`
  with PostgreSQL's strict byte-cap rule (a page never exceeds `max_bytes`
  unless it holds one resource), measured as each `resources` row's raw BSON
  length read straight off the driver cursor before the row is decoded.
  A capped page stays non-empty, so it continues PR1's walk phase with no
  change to phase-end or cursor logic; a new DEBUG line,
  `mongodb reindex capped page read`, records `rows`/`bytes`/`capped` per page.
- Two unrelated batch-size bugs are fixed at their source: `POST $reindex`'s
  `batchSize` now saturates to `u32::MAX` instead of silently wrapping to `0`
  (`4294967296 as u32 == 0`), and `run_reindex`'s paging loop now clamps its
  page limit to at least 1, matching the named-resources path.
- `HFS_REINDEX_BATCH_BYTES`'s server-side default moves from `0` (unbounded)
  to `33554432` (32 MiB) on every backend; the in-process library defaults
  (`ReindexRequest`, `AutomaticRunOptions`) stay `0`, so manual `$reindex` is
  unchanged. A new test pins that the automatic rebuild hook gets the new
  default when an operator never sets the variable.
- Sweeps the six doc locations #1499 named (README, hfs README, the
  configuration book, both `bulk-data-submit` SKILL files, `config.rs`'s own
  doc comment), which described the knob as SQLite-only, and adds the
  `docs/mongodb/bulk-import-sizing.md` "Knobs" section spec §6 assigns here.

## Design
- Spec: [`docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md`](../blob/main/docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md) (§4.3, §4.7)
- File-level design: `manual-test/archive/1403-run17-evidence/design/S3-pr2-overlap.md` §4 — local evidence only, not committed to this repo; SHA-256 `3918083534a81ae613af7b0da159338ba947bbd026eb74b1bfa817260dae2289` (recorded in the plan this PR implements, `docs/superpowers/plans/2026-09-23-1403-pr2a-mongodb-batch-bytes.md`).

## Gate (PR2a)
Pending — this PR's merge gate is Gate PR2a (spec §5.4; S4 §4.10.7), the `P2a-0`/`P2a-cap` Provenance-only before/after pair run by the orchestrator on this branch. Its results table (`G2a.1`–`G2a.6`) is pasted here with `gh pr edit` once that run completes; this PR is not merged before then.

## Test plan
- [x] `cargo test -p helios-persistence --features mongodb --lib backends::mongodb::storage::reindex_page_cap_tests`
- [x] `cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_pipeline` (Docker)
- [x] `cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_id_walk` (Docker; PR1 regression)
- [x] `cargo test -p helios-rest --lib handlers::reindex::tests::batch_size_param_clamps_and_saturates`
- [x] `cargo test -p helios-persistence --features sqlite --lib search::reindex::page_limit_tests`
- [x] `cargo test -p helios-rest --lib config::tests::test_elasticsearch_rebuild_knob_defaults`
- [x] `cargo test -p helios-hfs --bin hfs test_automatic_reindex_hook_gets_the_server_default_batch_bytes_when_unset`
- [x] `cargo test -p helios-persistence --features postgres --test postgres_tests postgres_integration_reindex_fetch_capped_boundaries` (Docker; regression)
- [x] `cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings` (repo's standard `-A` allowances) — same for `-p helios-rest` and `-p helios-hfs`
- [x] The #1291 single-backend `hfs` clippy matrix

Closes #1499

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Do not merge. Gate PR2a runs on this branch after it opens.

- [ ] **Step 9: After the Gate PR2a verdict, append the benchmark rows**

Once the orchestrator has run the `P2a-0`/`P2a-cap` pair and posted Gate PR2a's `G2a.1`–`G2a.6` verdict to this PR (Step 8's Gate section), append one row per arm to `docs/mongodb-reindex-benchmark.md` §8.2–§8.4, in a follow-up commit on this branch, using the exact column headers those sections already have (from the PR-docs plan, `docs/superpowers/plans/2026-09-23-1403-pr-docs.md` Task 1b):

```
### 8.2 Arms: outcome
| Arm | Gate | Source SHA | Binary SHA256 | Cache / memory MiB | Ingest wall | Rebuild wall | Obs walk ms/res | Q1 res/s | Q2 res/s | Q3 res/s | Q4 res/s | Q4/Q1 |

### 8.3 Arms: mechanism
| Arm | Q4 bm KB read/row | Q4 si KB read/row | Q4 miss % | bm KB written/row | iowait cores | Checkpoint max s |

### 8.4 Arms: correctness and guardrails
| Arm | Procedure ms/res | entriesCreated | errorCount | HFS peak MiB | fg p99 ms | Valid | Verdict |
```

Add a `P2a-0` row and a `P2a-cap` row to each of the three tables, then:

```bash
git add docs/mongodb-reindex-benchmark.md
git commit -m "$(cat <<'EOF'
docs(mongodb): record Gate PR2a's P2a-0/P2a-cap results (#1499)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

If `docs/mongodb-reindex-benchmark.md` does not exist on this branch yet (PR-docs has not merged), stop this step and report to the orchestrator — do not create the file here.

## Self-Review

**Spec coverage.**
- Spec §4.3 "MongoDB overrides `fetch_resources_page_capped` with PostgreSQL's strict rule": Task 1b (Steps 3a/3c).
- Spec §4.3 "measured as each row's raw BSON length" / "a page never exceeds `max_bytes` unless it holds one resource" / "continuation from last row taken" / "`max_bytes==0` = PR1's page": Task 1a/1b, Global Constraints, and the `reindex_page_admits`/`reindex_find_page`/`fetch_reindex_page` code and comments.
- Spec §4.3 "DEBUG `mongodb reindex capped page read`": Task 1b Step 3a, with `%`-formatted string fields per S2 §3.9 and field order `tenant, resource_type, rows, bytes, capped` per S3 §4.2 item 3.
- Spec §4.3 "REST handler saturates `batchSize`... driver uses `batch_size.max(1)`": Task 3, with the paging-path bug (`reindex.rs:2226` passes `request.batch_size` unclamped today) confirmed by reading `main` rather than assumed.
- Spec §4.3 "`HFS_REINDEX_BATCH_BYTES` = `33554432`... `ReindexRequest`/`AutomaticRunOptions` stay 0": Task 4 (server default) and Global Constraints (library defaults explicitly untouched — no task edits `ReindexRequest::default()` or `AutomaticRunOptions::default()`).
- Spec §4.3 "checks: the existing SQLite and PostgreSQL capped-fetch tests" — Task 6 Step 3 now runs both explicitly (`backends::sqlite::storage::tests::fetch_resources_page_capped`, matching two existing tests by substring, and `postgres_integration_reindex_fetch_capped_boundaries`), not just implicitly via the whole-lib run.
- Spec §4.3 "a `crates/hfs` test that the automatic hook receives 33554432 when the variable is unset": Task 5.
- Spec §4.3 "Six stale doc descriptions are rewritten. PR2a closes issue A": Task 4, Steps 5-7; issue #1499 read via `gh issue view 1499` names the same six locations, matched one-for-one.
- Spec §6 "The knobs section lands with PR2a/PR3": Task 4 Step 6, conditional on the PR-docs file already existing (confirmed at plan-revision time that `docs/mongodb/bulk-import-sizing.md` does not yet exist, so the precondition check is real, not decorative).
- S3 §4.1 "Behaviour contract" table (limit==0, max_bytes==0, max_bytes>0, capped page never moves phase, row_bytes definition, duplicates-in-a-round, decode failure): encoded in `reindex_page_admits`, `reindex_find_page`'s extension, and `fetch_reindex_page`'s doc comment/code (Tasks 1a-1b).
- S3 §4.2 items 1-6 (the pure helper, the extended `reindex_find_page`, `fetch_reindex_page`, the delegation, the override, "nothing else changes"): Task 1a Steps 3/6 and Task 1b Step 3a-3c, in that order — now as complete, compiling Rust copied from PR1's own implementation plan rather than as edit-instructions against code that plan could not yet see.
- S3 §4.3 "Batch-size clamps (D12)": Task 3, code copied verbatim from S3, with the wrong "passes today by coincidence" claim corrected against the actual HEAD behaviour (SQLite's `LIMIT 0`).
- S3 §4.4 "Default and docs" (new sentence, config.rs phrase fix, D4 sites, library default stays 0, who the cap affects): Task 4, including the SKILL.md files now carrying S3's exact sentence instead of a paraphrase that dropped the SQLite/PostgreSQL-MongoDB distinction.
- S3 §4.5 "Tests (PR2a)" — all seven named tests plus the four `reindex_page_cap_tests` unit tests: Tasks 1a (unit tests), 1b (integration tests 1-4, now id-phase-settled) and 2 (integration tests 5-7, matching S3's numbering of the Provenance-shaped, driver-run, and catch-up-boundary tests, all now using the same id-phase fixture rule S3 §4.5 states for "these tests", not just the boundary test).
- S3 §4.6 "Gate" is the orchestrator's bench run (S4 G2a.1-G2a.6), not a coding task — out of scope for this plan; Task 6's PR body names the gate table and blocks merge on it.
- S2 §12 "PR2a (honour `batch_bytes`)" bullet (phase-end rule, cursor-from-last-row-taken, the three specific edits, no `more_in_phase` code, the server-default note, "Issue A... is filed now"): Task 1b's code implements this directly; the "issue filed now" is already satisfied — #1499 exists and this plan's Global Constraints and every commit message reference it.
- Issue #1499's six named doc locations: matched exactly to Task 4.

**Coverage gap closed since the last revision.** PR1's own implementation plan, `docs/superpowers/plans/2026-09-23-1403-pr1-mongodb-id-walk.md`, now exists in this repo (it did not when S3 was drafted, and the prior revision of this plan wrongly asserted it still did not exist). Its Task 4 gives PR1's exact `fetch_resources_page` body, `reindex_find_page`'s exact signature and error text, and the `WalkStep`/`ReindexWalkCursor`/`reindex_page_from_docs` names this plan's Task 1a/1b now copy verbatim, rather than reconstructing them from S2's prose pseudocode. Every code fact this revision relies on (bson 2.15.0's `get_i64` panicking on an Int32, `Cursor::current`/`RawDocument::as_bytes`'s existence and signatures, `run_reindex`'s unclamped paging-path `batch_size`, SQLite's `LIMIT 0` behaviour and its `next_cursor` guard, CI's exact clippy `-A` list, `main.rs`'s missing `clap::Parser` import, `parse_history_row`'s `get_document("data")` path, `StoredResource::id()`/`content()`, `ReindexOperation::get_progress`'s `Option` return) was independently re-verified by reading the real files at HEAD `c86d0f08b`, not carried over from the design text unchecked.

**Placeholder scan.** No "TBD", "TODO", "add error handling", "similar to Task N", or "write tests for the above" appears anywhere above. `fetch_reindex_page` (Task 1b Step 3a) is now a complete method body, not edit-instructions in a comment — the prior revision's Task 1 Step 7c was a comment-only body that did not compile; this revision replaces it with PR1's real loop, parameterized by `max_bytes`, with the three PR2a edits (thread `max_bytes` through, build cursors from the last row taken, log when capped) applied inline. Every other code block in every task is complete, compiling Rust.

**Type consistency.** `fetch_resources_page_capped(&self, tenant: &TenantContext, resource_type: &str, cursor: Option<&str>, limit: u32, max_bytes: u64) -> StorageResult<ResourcePage>` is the same signature the trait already declares at HEAD (`reindex.rs:184-195`, unchanged by this plan) and is what Task 1b's override, Task 2's `RecordingSource`, and Task 3's `LimitRecordingSource` all implement identically. `ReindexFoundPage { docs: Vec<Document>, bytes: u64, capped: bool }` (Task 1a Step 3) is exactly the return type Task 1a Step 6's `reindex_find_page` produces and Task 1b Step 3a's `fetch_reindex_page` consumes. `reindex_page_admits(taken: usize, bytes_taken: u64, row_bytes: u64, max_bytes: u64) -> bool` has one call site (Task 1a Step 6) and four unit tests (Task 1a Step 1) using that exact argument order. `batch_size_param(size: u64) -> u32` (Task 3) matches its one call site and its test's five cases exactly. `ReindexOnFinish::batch_bytes(&self) -> u64` (Task 5) reads the same private `options: AutomaticRunOptions` field `with_batch_bytes` writes, both in the same `impl` block. `get_progress(&self, job_id: &str) -> Option<ReindexProgress>` (Task 2's Interfaces line, corrected from a `Result` in the prior revision) matches `reindex.rs:1241` exactly; every call site's `.unwrap()` is valid for either type, so no test code needed to change. `build_automatic_reindex_hook`'s return type, `helios_persistence::search::ReindexOnFinish`, is a concrete type (not the `Arc<dyn DeferredReindexHook>` the old function returned), consumed directly by Task 5's test and re-wrapped by the unchanged-signature `automatic_reindex_hook_with_ledger` for every other caller — every existing call site of `automatic_reindex_hook_with_ledger` (`standalone_ops`, `composite_ops`, and the other `main.rs` sites) keeps compiling unchanged.

**Rejected review items.** None. Every blocker, major and minor issue in the review was independently confirmed against the real codebase (see the coverage-gap note above) and applied; none was judged incorrect.
