# MongoDB PR1: id-order `$reindex` walk with catch-up rounds Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** MongoDB's `$reindex` walks each type in resource-`id` order (instead of `(last_updated, id)` order) with bounded catch-up rounds for anything written during the walk, so a full-corpus rebuild stops paying random-leaf cache misses on every Observation insert.

**Architecture:** `MongoBackend::fetch_resources_page` (the `ReindexSource` half of `$reindex`) is rewritten as a two-phase state machine carried in an opaque, versioned cursor: an id-order phase over everything live before the walk started (hinted to the unique `idx_resources_identity`), followed by up to three fixed-range catch-up rounds in today's `(last_updated, id)` order (hinted to `idx_resources_type_scan`) that heal anything written during the walk. A phase ends only on an empty query, so the driver — unchanged, still strictly serial — always gets a non-empty page or the type's one trailing empty page. No trait, index key, generation or persisted-state changes; the only new config surface is one clamped `MongoBackendConfig` field.

**Tech Stack:** Rust (edition 2024), `mongodb` driver 3.7.0, `bson` `doc!`, `chrono` `DateTime<Utc>`/`Duration`, `tracing` structured fields, testcontainers integration suite `crates/persistence/tests/mongodb_tests.rs` (MongoDB 5.0.6 in CI via `testcontainers-modules`).

**Spec:** `docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md` §4.2, §9 — and its cited section file `C:\Users\DougC\Code\Helios\manual-test\archive\1403-run17-evidence\design\S2-pr1-id-walk.md` (SHA-256 `ad782e257989392cef99e2ecbea58a036b37caa3530c1da3710cc92c805e8a4e`, verified against the spec's §11 citation), which is the source of truth for every field name, log line and test in this plan.

## Global Constraints

- **No cargo while a bench arm is running (S4 §4.5.1, §4.12.3 "Orchestrator rule: no cargo during arms").** Before every cargo/test/build command anywhere in this plan, run:
  ```bash
  test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
  ```
  If it prints and exits, stop immediately and report back instead of running `cargo`, `rustc`, a test or a build — a `SMOKE-0`/`SMOKE-1`, `B0-s` or later arm may be mid-measurement, and running any of these can invalidate that arm or the host-quiet precondition its validity checks assume.
- **PR0 has already merged when this plan is implemented.** Branch `perf/1403-pr1-mongodb-id-walk` from `origin/main`, not from this design branch (Task 0). Every `file:line` anchor in this plan was verified at `c86d0f08b` (HEAD when this plan was written, before PR0 landed); PR0 (S1) adds always-on timing to `run_reindex` in `crates/persistence/src/search/reindex.rs` and a `_timed` writer variant in `crates/persistence/src/backends/mongodb/storage.rs` — renaming today's `write_search_entries_page` to `write_search_entries_page_timed` and moving its whole doc comment onto it — which **will shift line numbers in both files and, for that one method, its name**. Before Task 1, re-run every anchor grep in this plan (Task 0 Step 2: `tgrep -n -F -- "<anchor text>" C:/Users/DougC/Code/Helios/hfs`, root only — never add a single file, which forces `tgrep` to fall back to a full scan) against the actual `origin/main` you branched from, and use the function/struct/const names — not the line numbers — as the real anchor. The line numbers are hints only.
- **Docker-gated steps.** Every step that runs `reindex_id_walk` or any other MongoDB integration test needs Docker (testcontainers, MongoDB 5.0.6) or `HFS_TEST_MONGODB_URL`. A `Skipping` line in that test's own output, or `running 0 tests`, means the step did not run — it is never an acceptable substitute for the PASS the step asks for, and must not be committed as if it were one. Steps below write their log to `$LOGDIR/<name>.log`; before Task 1, set `LOGDIR` once for the shell session to your own scratchpad/temp directory — **never `/tmp` on Windows** — e.g. `export LOGDIR=/c/Users/<you>/AppData/Local/Temp/<your-scratchpad>` (Claude Code sessions already have one; use it). Read the log before deciding a step is done.
- PR1 changes only MongoDB's reindex source (`fetch_resources_page`), one config field (`MongoBackendConfig::reindex_catch_up_margin_ms`), two index-name constants in `schema.rs`, one doc comment on `write_search_entries_page`, and a `docs/mongodb/search-indexes.md` section. No `ReindexSource`/`ReindexTarget` trait change, no driver change, no index key/name/generation change, no persisted state (S2 §1).
- Never touch `crates/persistence/src/backends/mongodb/search_index_catalog.rs`, `crates/persistence/src/backends/mongodb/search_impl.rs`, or `docs/mongodb/*.mongosh.js`. `SCHEMA_VERSION` stays `10`; `SEARCH_INDEX_GENERATION` stays `3`.
- No `HFS_*` env var for the catch-up margin in PR1 — only tests change `reindex_catch_up_margin_ms`, by constructing `MongoBackendConfig` directly (S2 §3.10, D6).
- Catch-up margin default is `120_000` ms, clamped to `1_000..=86_400_000`; at most `REINDEX_CATCH_UP_MAX_ROUNDS = 3` catch-up rounds; round `k >= 2` runs only if round `k-1` took at least `margin/2` (spec §4.2).
- Cursor grammar is versioned `v2|...` (`v2|i|<floor>|<after_id>` for the id phase, `v2|c|<round>|<floor>|<ceiling>|<walked>|<after_lu>|<after_id>` for a catch-up round). Anything else — including HEAD's bare `<rfc3339>|<id>` — is `StorageError::Search(SearchError::InvalidCursor { .. })`, failing the run instead of silently restarting the type (spec §4.2, S2 §4.2 D5).
- **Never `git add -A` or `git commit -a`.** Building this workspace dirties ~3,500 generated R6 spec files; every commit in this plan stages explicit paths.
- Every commit message ends with a blank line, then exactly:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```
- CI runs MongoDB 5.0.6 only (`testcontainers-modules` 0.15.0 pins `TAG = "5.0.6"`); the MongoDB 7.0 gate (Task 9) is run manually in a throwaway container and its result recorded in the PR description, never against `hfs-mongo`.
- `cargo fmt` in a worktree must target explicit paths (`cargo fmt --all` fails there); redirect command output to `/dev/null`, never `nul`.
- Set `CARGO_BUILD_JOBS=4` first if the machine is memory-constrained; if `target/` fills the disk, delete `target/debug/incremental` first.

---

## File Structure

| File | Responsibility in this plan |
|---|---|
| `crates/persistence/src/backends/mongodb/backend.rs` | `MongoBackendConfig::reindex_catch_up_margin_ms` field and its default (Task 1) |
| `crates/hfs/src/main.rs` | adds the field to the one exhaustive `MongoBackendConfig` literal in `build_mongodb_config_with_env` (Task 1) |
| `crates/persistence/src/backends/mongodb/schema.rs` | `RESOURCES_IDENTITY_INDEX` / `RESOURCES_TYPE_SCAN_INDEX` `pub(crate)` constants replacing two literals; one comment paragraph (Task 2) |
| `crates/persistence/src/backends/mongodb/storage.rs` | cursor grammar (`ReindexWalkCursor`), pure helpers (margin/floor/ceiling/round decision), filter builders, dedupe (Task 3); the schema-constants import, the walk loop (`WalkStep`), private DB helpers, the rewritten `fetch_resources_page` (Task 4 Part A); the `write_search_entries_page_timed` precondition doc comment (Task 4 Part B) |
| `docs/mongodb/search-indexes.md` | new "How `$reindex` walks a type (#1403)" section (Task 4 Part B) |
| `crates/persistence/tests/mongodb_tests.rs` | `#[path]` include of the new test module; one config-default assertion (Tasks 1, 4 Part A) |
| `crates/persistence/tests/mongodb/reindex_id_walk.rs` | new — harness (fixture, snapshot, log capture), test doubles, T1–T12 (Tasks 4-8) |

---

### Task 0: Branch and re-verify anchors against the real `origin/main`

- [ ] **Step 1: Create the branch**

```bash
git fetch origin
git switch -c perf/1403-pr1-mongodb-id-walk origin/main
```

- [ ] **Step 2: Re-run every anchor grep**

Per the Global Constraints note: PR0 (S1) has already merged into `origin/main` by the time this branch is cut, and it shifts line numbers in `crates/persistence/src/search/reindex.rs` and `crates/persistence/src/backends/mongodb/storage.rs`. Before starting Task 1, re-run these greps against the branch just created and use the function/struct/const names they land on — not the line numbers printed in this plan — as the real anchor:

```
tgrep -n -F -- "fn fetch_resources_page" C:/Users/DougC/Code/Helios/hfs
tgrep -n -F -- "fn parse_reindex_cursor" C:/Users/DougC/Code/Helios/hfs
tgrep -n -F -- "Precondition:" C:/Users/DougC/Code/Helios/hfs
tgrep -n -F -- "mod versioned_write_race_suite" C:/Users/DougC/Code/Helios/hfs
tgrep -n -F -- "Ok(MongoBackendConfig {" C:/Users/DougC/Code/Helios/hfs
tgrep -n -F -- "## Composite parameters" C:/Users/DougC/Code/Helios/hfs
```
Always pass the repo root (`C:/Users/DougC/Code/Helios/hfs`), never a single file — `tgrep` on one file forces a full scan instead of using the index. Read each `file:line` hit before trusting it; the `Precondition:` grep specifically is expected to land on `write_search_entries_page_timed` post-PR0, not `write_search_entries_page` (see Task 4 Step 8).

---

### Task 1: Catch-up margin config field

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/backend.rs` — `MongoBackendConfig` struct (anchor: the `index_build` field, at HEAD c86d0f08b `:183-184`), the block of `default_*` free functions (anchor: `default_max_included_resources`/`default_app_name`, `:207-213`), `impl Default for MongoBackendConfig` (anchor: `:215-231`)
- Modify: `crates/hfs/src/main.rs` — the one exhaustive `MongoBackendConfig { .. }` literal inside `build_mongodb_config_with_env` (anchor: `Ok(MongoBackendConfig {`, at HEAD c86d0f08b `:346-358`)
- Test: `crates/persistence/tests/mongodb_tests.rs` — `test_mongodb_config_defaults` (anchor: `:99-111`)

**Interfaces:**
- Produces: `pub reindex_catch_up_margin_ms: u64` on `MongoBackendConfig`, `#[serde(default = "default_reindex_catch_up_margin_ms")]`, default `120_000`. Every other of the 19 `MongoBackendConfig { .. }` literals in the workspace uses `..Default::default()` and needs no change; `MongoBackend::from_env` (`backend.rs:341`) also uses struct-update syntax and needs no change.

- [ ] **Step 1: Write the failing test**

In `crates/persistence/tests/mongodb_tests.rs`, extend `test_mongodb_config_defaults`:

```rust
#[test]
fn test_mongodb_config_defaults() {
    let config = MongoBackendConfig::default();
    assert_eq!(config.connection_string, "mongodb://localhost:27017");
    assert_eq!(config.database_name, "helios");
    assert_eq!(config.max_connections, 10);
    assert_eq!(config.connect_timeout_ms, 5000);
    // Unchanged from when this was a hardcoded constant — making it configurable
    // must not change the default behaviour of an existing deployment.
    assert_eq!(config.server_selection_timeout_ms, 15_000);
    assert!(!config.search_offloaded);
    assert_eq!(config.fhir_version, FhirVersion::default());
    // #1403: the `$reindex` walk's clock-skew and commit-lag allowance.
    assert_eq!(config.reindex_catch_up_margin_ms, 120_000);
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests test_mongodb_config_defaults`
Expected: compile error — `no field \`reindex_catch_up_margin_ms\` on type \`MongoBackendConfig\`` (or `no method named \`reindex_catch_up_margin_ms\``).

- [ ] **Step 3: Implement**

In `crates/persistence/src/backends/mongodb/backend.rs`, add the field after `index_build`:

```rust
    /// When the generation-2 `search_index` indexes are built relative to
    /// boot: `background` (default) spawns the builder and serves at once,
    /// `inline` awaits it, `off` only warns about missing indexes so an
    /// operator can build them out of band (`HFS_MONGODB_INDEX_BUILD`).
    #[serde(default)]
    pub index_build: IndexBuildMode,

    /// Clock-skew and commit-lag allowance of the `$reindex` walk's catch-up
    /// rounds, in milliseconds (#1403). A resource stamped within this much of
    /// a walk's start, or written while it runs, is re-read in `last_updated`
    /// order after the id-order pass. Default 120 000 (two minutes); values
    /// are clamped to 1 000 ..= 86 400 000. No `HFS_*` variable reads this in
    /// PR1 — only tests construct a `MongoBackendConfig` with a shorter one.
    #[serde(default = "default_reindex_catch_up_margin_ms")]
    pub reindex_catch_up_margin_ms: u64,
}
```

Add the default function next to the other `default_*` functions:

```rust
fn default_reindex_catch_up_margin_ms() -> u64 {
    120_000
}
```

Add the field to `impl Default for MongoBackendConfig`:

```rust
impl Default for MongoBackendConfig {
    fn default() -> Self {
        Self {
            connection_string: default_connection_string(),
            database_name: default_database_name(),
            max_connections: default_max_connections(),
            connect_timeout_ms: default_connect_timeout_ms(),
            server_selection_timeout_ms: default_server_selection_timeout_ms(),
            fhir_version: FhirVersion::default_enabled(),
            data_dir: None,
            search_offloaded: false,
            max_included_resources: default_max_included_resources(),
            app_name: default_app_name(),
            index_build: IndexBuildMode::default(),
            reindex_catch_up_margin_ms: default_reindex_catch_up_margin_ms(),
        }
    }
}
```

In `crates/hfs/src/main.rs`, add one line to the exhaustive literal in `build_mongodb_config_with_env`:

```rust
    Ok(MongoBackendConfig {
        connection_string,
        database_name,
        max_connections,
        connect_timeout_ms,
        server_selection_timeout_ms,
        fhir_version: config.default_fhir_version,
        data_dir: config.data_dir.clone(),
        search_offloaded,
        max_included_resources,
        index_build,
        app_name: MongoBackendConfig::default().app_name,
        reindex_catch_up_margin_ms: MongoBackendConfig::default().reindex_catch_up_margin_ms,
    })
```

- [ ] **Step 4: Run it to verify it passes**

Run:
```
cargo test -p helios-persistence --features mongodb --test mongodb_tests test_mongodb_config
cargo check -p helios-hfs --features mongodb --tests
cargo test -p helios-hfs --features mongodb --bin hfs test_build_mongodb_config
```
Expected: `test_mongodb_config_defaults` and `test_mongodb_config_serialization` PASS (the latter is untouched — it uses `..Default::default()` and never lists this field); `helios-hfs` compiles clean, confirming the exhaustive literal is complete; the four `test_build_mongodb_config_*` tests in `crates/hfs/src/main.rs`'s `mod tests` (`test_build_mongodb_config_overlays_env_with_mongo_database_url`, `..._ignores_non_mongo_database_url`, `..._reads_index_build_and_rejects_invalid_values`, `..._uses_mongo_specific_url_before_database_url_fallback`) PASS — `4 passed`, confirming the new field did not break the exhaustive literal's construction from env.

- [ ] **Step 5: Commit**

```bash
git add crates/persistence/src/backends/mongodb/backend.rs crates/hfs/src/main.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "$(cat <<'EOF'
feat(mongodb): add the $reindex walk's catch-up margin config field (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Index-name constants and the walk's schema comment

This task is a verified refactor, not TDD: `schema` is `pub(crate) mod schema;` (`crates/persistence/src/backends/mongodb/mod.rs:26`), and every item inside it — including a new `pub(crate) const` — is invisible to `crates/persistence/tests/mongodb_tests.rs`, which is compiled as a separate crate. There is no visibility level that makes a `pub(crate)` constant reachable from an integration test, so a test asserting `RESOURCES_IDENTITY_INDEX == "idx_resources_identity"` from that file cannot compile, and one placed as a lib unit test would only assert a constant equals the literal used to define it — it does not show `ensure_resources_indexes` uses the constant. The real check is that the crate still builds with the literals replaced by named constants and that the live-server index-name tests still see the same index names, both already covered below.

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/schema.rs` — `ensure_resources_indexes` (anchor: `:108-160`), the two literal index-name strings (anchors: `"idx_resources_identity"` at `:114`, `"idx_resources_type_scan"` at `:145`), the comment block above the second `create_index` call (anchor: ends `"...examining 500 with this index."` at `:130`, blank comment line at `:131`, next paragraph starts `idx_resources_type_deleted` at `:132`)

**Interfaces:**
- Produces: `pub(crate) const RESOURCES_IDENTITY_INDEX: &str = "idx_resources_identity";` and `pub(crate) const RESOURCES_TYPE_SCAN_INDEX: &str = "idx_resources_type_scan";` in `schema.rs`, above `ensure_resources_indexes`. Consumed by Task 4's walk via `use super::schema::{RESOURCES_IDENTITY_INDEX, RESOURCES_TYPE_SCAN_INDEX};` in `storage.rs`. `search_impl.rs:1329`'s own `"idx_resources_identity"` literal is left alone (S2 §7.3, §13 D14) — this task's diff must not touch that file.

- [ ] **Step 1: Implement**

In `crates/persistence/src/backends/mongodb/schema.rs`, add above `ensure_resources_indexes`:

```rust
/// Name of the unique `(tenant_id, resource_type, id)` index on `resources`;
/// the `$reindex` id phase hints it (#1403).
pub(crate) const RESOURCES_IDENTITY_INDEX: &str = "idx_resources_identity";
/// Name of the `(tenant_id, resource_type, is_deleted, last_updated, id)`
/// index on `resources`; the `$reindex` catch-up rounds and newest-live
/// probe hint it (#1021, #1403).
pub(crate) const RESOURCES_TYPE_SCAN_INDEX: &str = "idx_resources_type_scan";

async fn ensure_resources_indexes(database: &Database) -> StorageResult<()> {
    let resources = database.collection::<Document>("resources");

    create_index(
        &resources,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "id": 1_i32 },
        RESOURCES_IDENTITY_INDEX,
        true,
    )
    .await?;
```

Replace the literal at the second `create_index` call:

```rust
    create_index(
        &resources,
        doc! {
            "tenant_id": 1_i32,
            "resource_type": 1_i32,
            "is_deleted": 1_i32,
            "last_updated": 1_i32,
            "id": 1_i32,
        },
        RESOURCES_TYPE_SCAN_INDEX,
        false,
    )
    .await?;
```

The comment above the second `create_index` call already has a lone blank `//` line at `:131`, between the paragraph ending "...examining 500 with this index." (`:130`) and the paragraph starting "`idx_resources_type_deleted`..." (`:132`). Replace that single `//` line with the following five lines, so the new paragraph is inserted in place of it rather than beside it — this avoids a double blank comment line on either side:

```rust
    //
    // Since #1403 the `$reindex` walk pages each type in `id` order on
    // `idx_resources_identity`; this index still serves the walk's catch-up
    // rounds (the `(last_updated, id)` keyset) and the newest-live probe that
    // sets its floor and ceilings.
    //
```

- [ ] **Step 2: Verify**

```
cargo check -p helios-persistence --features mongodb --tests
```
Expected: compiles clean — `ensure_resources_indexes` now references the two constants instead of the two literals, with no behavior change (the constants' values are exactly the strings they replace). Then run the existing live-server tests that observe these index names, to confirm the rename changed nothing externally:
```
cargo test -p helios-persistence --features mongodb --test mongodb_tests mongodb_integration_schema_v9_swaps_in_the_reindex_scan_index
cargo test -p helios-persistence --features mongodb --test mongodb_tests mongodb_integration_boot_creates_only_inline_search_indexes_and_keeps_generation_record
```
(Both require Docker or `HFS_TEST_MONGODB_URL`; a `Skipping` `eprintln!` and early return only counts as a pass if Docker is genuinely unavailable in this environment — see the Global Constraints note on Docker-gated steps. When Docker is available, expect both to PASS.)

- [ ] **Step 3: Commit**

```bash
git add crates/persistence/src/backends/mongodb/schema.rs
git commit -m "$(cat <<'EOF'
refactor(mongodb): name the two resources indexes with constants (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Cursor grammar and the walk's pure helpers

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/storage.rs` — imports (anchor: `chrono::{DateTime, Utc}` at `:6`, `mongodb::{.., options::FindOptions}` at `:8-13`, `crate::error::{.., StorageError, StorageResult, ..}` at `:23-26`); new constants and pure types/functions placed after `insert_search_entries_chunk` (anchor: the block starting `const SEARCH_INDEX_INSERT_CHUNK: usize = 5_000;` at `:5173`, ending with `parse_reindex_cursor` at `:5220-5225`, which this task's Task 4 sibling will delete); a new `#[cfg(test)] mod reindex_walk_tests` at the end of the file (anchor: follows the pattern of `mod history_query_tests` at `:5416`)

**Interfaces:**
- Produces (all private to `storage.rs`, unused outside `#[cfg(test)]` until Task 4 wires them in — expect harmless `dead_code` warnings, not errors, until then):
  ```rust
  const REINDEX_CATCH_UP_MAX_ROUNDS: u8 = 3;
  const REINDEX_CATCH_UP_MARGIN_MIN_MS: u64 = 1_000;
  const REINDEX_CATCH_UP_MARGIN_MAX_MS: u64 = 86_400_000;

  #[derive(Debug, Clone, PartialEq, Eq)]
  enum ReindexWalkCursor {
      Id { floor: DateTime<Utc>, after_id: String },
      Round {
          round: u8,
          floor: DateTime<Utc>,
          ceiling: DateTime<Utc>,
          walked: u64,
          after_last_updated: DateTime<Utc>,
          after_id: String,
      },
  }
  impl ReindexWalkCursor {
      fn encode(&self) -> String;
      fn parse(cursor: &str) -> StorageResult<Self>;
  }

  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  enum RoundStartDecision { Run, Complete, CapReached }

  fn truncate_to_millis(dt: DateTime<Utc>) -> DateTime<Utc>;
  fn format_walk_instant(dt: DateTime<Utc>) -> String;
  fn reindex_catch_up_margin(configured_ms: u64) -> chrono::Duration;
  fn reindex_catch_up_floor(t0: DateTime<Utc>, newest_live: Option<DateTime<Utc>>, margin: chrono::Duration) -> DateTime<Utc>;
  fn reindex_catch_up_ceiling(now: DateTime<Utc>, newest_live: Option<DateTime<Utc>>, margin: chrono::Duration) -> DateTime<Utc>;
  fn reindex_round_start_decision(round: u8, floor: DateTime<Utc>, now: DateTime<Utc>, margin: chrono::Duration) -> RoundStartDecision;
  fn reindex_id_page_filter(tenant_id: &str, resource_type: &str, floor: DateTime<Utc>, after_id: Option<&str>) -> Document;
  fn reindex_catch_up_page_filter(tenant_id: &str, resource_type: &str, floor: DateTime<Utc>, ceiling: DateTime<Utc>, after: Option<(DateTime<Utc>, &str)>) -> Document;
  fn dedupe_reindex_page_keep_last(docs: Vec<Document>) -> Vec<Document>;
  ```

- [ ] **Step 1: Write the failing tests**

Add at the end of `crates/persistence/src/backends/mongodb/storage.rs`, following the pattern of `mod history_query_tests`:

```rust
#[cfg(test)]
mod reindex_walk_tests {
    //! Docker-free unit tests for #1403's id-order `$reindex` walk: the
    //! cursor grammar, the pure floor/ceiling/round-decision rules, the
    //! filter builders, and the round-page dedupe. The walk itself
    //! (`fetch_resources_page`) is covered by the MongoDB integration suite
    //! in `tests/mongodb/reindex_id_walk.rs`, since it needs a live server.

    use super::*;

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    // --- Cursor grammar (§4) ---

    #[test]
    fn cursor_round_trips_every_state() {
        let id_cursor = ReindexWalkCursor::Id {
            floor: ts("2026-01-01T00:00:00.123Z"),
            after_id: "A-1.b".to_string(),
        };
        assert_eq!(ReindexWalkCursor::parse(&id_cursor.encode()).unwrap(), id_cursor);

        for round in [1u8, REINDEX_CATCH_UP_MAX_ROUNDS] {
            for walked in [0u64, u64::MAX] {
                let round_cursor = ReindexWalkCursor::Round {
                    round,
                    floor: ts("2026-01-01T00:00:00.000Z"),
                    ceiling: ts("2026-01-01T00:02:00.000Z"),
                    walked,
                    after_last_updated: ts("2026-01-01T00:01:00.500Z"),
                    after_id: "obs-017".to_string(),
                };
                assert_eq!(
                    ReindexWalkCursor::parse(&round_cursor.encode()).unwrap(),
                    round_cursor
                );
            }
        }
    }

    #[test]
    fn cursor_id_is_the_verbatim_remainder() {
        let id_cursor = ReindexWalkCursor::Id {
            floor: ts("2026-01-01T00:00:00.000Z"),
            after_id: "a|b".to_string(),
        };
        assert_eq!(ReindexWalkCursor::parse(&id_cursor.encode()).unwrap(), id_cursor);

        let round_cursor = ReindexWalkCursor::Round {
            round: 1,
            floor: ts("2026-01-01T00:00:00.000Z"),
            ceiling: ts("2026-01-01T00:02:00.000Z"),
            walked: 3,
            after_last_updated: ts("2026-01-01T00:01:00.000Z"),
            after_id: "a|b".to_string(),
        };
        assert_eq!(
            ReindexWalkCursor::parse(&round_cursor.encode()).unwrap(),
            round_cursor
        );
    }

    #[test]
    fn cursor_rejects_foreign_and_malformed_tokens() {
        let instant = "2026-01-01T00:00:00.000Z";
        let ceiling = "2026-01-01T00:02:00.000Z";
        let bad: Vec<String> = vec![
            "".to_string(),
            "2026-09-19T04:43:29.668+00:00|e357ce58-f379-216d-a369-99da40ff76ae".to_string(),
            format!("v1|i|{instant}|a"),
            format!("v3|i|{instant}|a"),
            format!("v2|x|{instant}|a"),
            format!("v2|s|1|{instant}"),
            format!("v2|i|{instant}|"),
            "v2|i|not-a-time|a".to_string(),
            // round 0 (below the 1..=MAX range)
            format!("v2|c|0|{instant}|{ceiling}|0|{instant}|a"),
            // round MAX + 1 (above the range)
            format!(
                "v2|c|{}|{instant}|{ceiling}|0|{instant}|a",
                REINDEX_CATCH_UP_MAX_ROUNDS + 1
            ),
            // five fields instead of six (missing after_lu)
            format!("v2|c|1|{instant}|{ceiling}|0|a"),
            // walked = -1
            format!("v2|c|1|{instant}|{ceiling}|-1|{instant}|a"),
            // floor == ceiling
            format!("v2|c|1|{instant}|{instant}|0|{instant}|a"),
            // after_lu < floor
            format!("v2|c|1|{instant}|{ceiling}|0|2025-12-31T23:59:59.000Z|a"),
            // after_lu == ceiling
            format!("v2|c|1|{instant}|{ceiling}|0|{ceiling}|a"),
        ];
        for cursor in bad {
            match ReindexWalkCursor::parse(&cursor) {
                Err(StorageError::Search(SearchError::InvalidCursor { .. })) => {}
                other => panic!("expected InvalidCursor for {cursor:?}, got {other:?}"),
            }
        }
    }

    // --- Floor / ceiling / margin / round decision (§3.2-3.4, §3.10) ---

    #[test]
    fn floor_is_newest_plus_one_ms_for_old_data() {
        let t0 = ts("2026-01-01T01:00:00.000Z");
        let newest = ts("2026-01-01T00:00:00.000Z"); // far older than t0 - margin
        let margin = chrono::Duration::seconds(120);
        assert_eq!(
            reindex_catch_up_floor(t0, Some(newest), margin),
            newest + chrono::Duration::milliseconds(1)
        );
    }

    #[test]
    fn floor_is_t0_minus_margin_for_fresh_data() {
        let t0 = ts("2026-01-01T01:00:00.000Z");
        let newest = t0 - chrono::Duration::seconds(1); // inside the margin
        let margin = chrono::Duration::seconds(120);
        assert_eq!(reindex_catch_up_floor(t0, Some(newest), margin), t0 - margin);
    }

    #[test]
    fn floor_without_live_resources_is_t0_minus_margin() {
        let t0 = ts("2026-01-01T01:00:00.000Z");
        let margin = chrono::Duration::seconds(120);
        assert_eq!(reindex_catch_up_floor(t0, None, margin), t0 - margin);
    }

    #[test]
    fn floor_truncates_to_milliseconds() {
        let t0 = Utc::now(); // sub-millisecond precision on most platforms
        let margin = chrono::Duration::seconds(120);
        let floor = reindex_catch_up_floor(t0, None, margin);
        assert_eq!(floor.timestamp_subsec_nanos() % 1_000_000, 0);
    }

    #[test]
    fn ceiling_is_now_plus_margin_for_past_stamps() {
        let now = ts("2026-01-01T01:00:00.000Z");
        let margin = chrono::Duration::seconds(120);
        assert_eq!(
            reindex_catch_up_ceiling(now, Some(now - chrono::Duration::seconds(1)), margin),
            now + margin
        );
        assert_eq!(reindex_catch_up_ceiling(now, None, margin), now + margin);
    }

    #[test]
    fn ceiling_passes_a_future_stamp() {
        let now = ts("2026-01-01T01:00:00.000Z");
        let margin = chrono::Duration::seconds(120);
        let newest = now + margin + chrono::Duration::seconds(5);
        assert_eq!(
            reindex_catch_up_ceiling(now, Some(newest), margin),
            newest + chrono::Duration::milliseconds(1)
        );
    }

    #[test]
    fn round_start_decision_round_one_always_runs() {
        let floor = ts("2026-01-01T00:00:00.000Z");
        let margin = chrono::Duration::seconds(120);
        assert_eq!(
            reindex_round_start_decision(1, floor, floor - chrono::Duration::hours(1), margin),
            RoundStartDecision::Run
        );
        assert_eq!(
            reindex_round_start_decision(1, floor, floor + chrono::Duration::hours(1), margin),
            RoundStartDecision::Run
        );
    }

    #[test]
    fn round_start_decision_round_two_completes_or_runs_at_the_half_margin_boundary() {
        let floor = ts("2026-01-01T00:02:00.000Z");
        let margin = chrono::Duration::seconds(120);
        let boundary = floor - margin / 2;
        assert_eq!(
            reindex_round_start_decision(2, floor, boundary, margin),
            RoundStartDecision::Run
        );
        assert_eq!(
            reindex_round_start_decision(2, floor, boundary - chrono::Duration::milliseconds(1), margin),
            RoundStartDecision::Complete
        );
    }

    #[test]
    fn round_start_decision_caps_or_completes_past_the_round_limit() {
        let floor = ts("2026-01-01T00:02:00.000Z");
        let margin = chrono::Duration::seconds(120);
        let boundary = floor - margin / 2;
        let round = REINDEX_CATCH_UP_MAX_ROUNDS + 1;
        assert_eq!(
            reindex_round_start_decision(round, floor, boundary, margin),
            RoundStartDecision::CapReached
        );
        assert_eq!(
            reindex_round_start_decision(round, floor, boundary - chrono::Duration::milliseconds(1), margin),
            RoundStartDecision::Complete
        );
    }

    #[test]
    fn margin_is_clamped() {
        assert_eq!(reindex_catch_up_margin(0), chrono::Duration::seconds(1));
        assert_eq!(reindex_catch_up_margin(120_000), chrono::Duration::seconds(120));
        assert_eq!(reindex_catch_up_margin(u64::MAX), chrono::Duration::hours(24));
    }

    // --- Filter shapes (§3.3, §3.4) ---

    #[test]
    fn id_page_filter_shape() {
        let floor = ts("2026-01-01T00:00:00.000Z");
        let first = reindex_id_page_filter("t1", "Observation", floor, None);
        assert!(!first.contains_key("id"));
        assert_eq!(first.get_bool("is_deleted"), Ok(false));
        assert_eq!(
            first.get_document("last_updated").unwrap().get("$lt"),
            Some(&Bson::from(chrono_to_bson(floor)))
        );

        let later = reindex_id_page_filter("t1", "Observation", floor, Some("obs-010"));
        assert_eq!(later.get_bool("is_deleted"), Ok(false));
        assert_eq!(
            later.get_document("last_updated").unwrap().get("$lt"),
            Some(&Bson::from(chrono_to_bson(floor)))
        );
        assert_eq!(
            later.get_document("id").unwrap().get_str("$gt"),
            Ok("obs-010")
        );
    }

    #[test]
    fn catch_up_filter_shape() {
        let floor = ts("2026-01-01T00:00:00.000Z");
        let ceiling = ts("2026-01-01T00:02:00.000Z");
        let first = reindex_catch_up_page_filter("t1", "Observation", floor, ceiling, None);
        assert!(!first.contains_key("$or"));
        let range = first.get_document("last_updated").unwrap();
        assert_eq!(range.get("$gte"), Some(&Bson::from(chrono_to_bson(floor))));
        assert_eq!(range.get("$lt"), Some(&Bson::from(chrono_to_bson(ceiling))));

        let after_lu = ts("2026-01-01T00:01:00.000Z");
        let continuation = reindex_catch_up_page_filter(
            "t1",
            "Observation",
            floor,
            ceiling,
            Some((after_lu, "obs-020")),
        );
        assert!(!continuation.contains_key("last_updated"));
        let or = continuation.get_array("$or").unwrap();
        assert_eq!(or.len(), 2);
        let first_arm_doc = or[0].as_document().unwrap();
        assert_eq!(first_arm_doc.len(), 1, "arm 0 must hold only `last_updated`: {first_arm_doc:?}");
        let first_arm = first_arm_doc.get_document("last_updated").unwrap();
        assert_eq!(first_arm.get("$gt"), Some(&Bson::from(chrono_to_bson(after_lu))));
        assert_eq!(first_arm.get("$lt"), Some(&Bson::from(chrono_to_bson(ceiling))));
        let second_arm = or[1].as_document().unwrap();
        assert_eq!(
            second_arm.len(),
            2,
            "arm 1 must hold exactly `last_updated` and `id`: {second_arm:?}"
        );
        assert_eq!(
            second_arm.get("last_updated"),
            Some(&Bson::from(chrono_to_bson(after_lu)))
        );
        assert_eq!(
            second_arm.get_document("id").unwrap().get_str("$gt"),
            Ok("obs-020")
        );
    }

    // --- Dedupe (§5) ---

    #[test]
    fn dedupe_keeps_the_last_occurrence_in_scan_order() {
        let docs = vec![
            doc! { "id": "a", "v": 1 },
            doc! { "note": "no id" },
            doc! { "id": "b", "v": 1 },
            doc! { "id": "a", "v": 2 },
        ];
        let deduped = dedupe_reindex_page_keep_last(docs);
        assert_eq!(deduped.len(), 3);
        assert!(!deduped[0].contains_key("id")); // "no id" doc kept in place
        assert_eq!(deduped[0].get_str("note"), Ok("no id"));
        assert_eq!(deduped[1].get_str("id"), Ok("b"));
        assert_eq!(deduped[2].get_str("id"), Ok("a"));
        assert_eq!(deduped[2].get_i32("v"), Ok(2));
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p helios-persistence --features mongodb --lib reindex_walk_tests`
Expected: compile errors for every undefined name — `ReindexWalkCursor`, `RoundStartDecision`, `REINDEX_CATCH_UP_MAX_ROUNDS`, `reindex_catch_up_floor`, `reindex_catch_up_ceiling`, `reindex_round_start_decision`, `reindex_catch_up_margin`, `reindex_id_page_filter`, `reindex_catch_up_page_filter`, `dedupe_reindex_page_keep_last`, and `SearchError` (not yet imported).

- [ ] **Step 3: Implement**

Extend the imports at the top of `storage.rs`:

```rust
use mongodb::{
    ClientSession, Collection, Cursor, SessionCursor,
    bson::{self, Bson, DateTime as BsonDateTime, Document, doc},
    error::{Error as MongoError, ErrorKind as MongoErrorKind},
    options::{FindOptions, Hint},
};
```

```rust
use crate::error::{
    BackendError, ConcurrencyError, QueryErrorExt, ResourceError, SearchError, StorageError,
    StorageResult, TransactionError,
};
```

Add, right after `insert_search_entries_chunk` and before `parse_reindex_cursor` (which Task 4 deletes):

```rust
/// Most catch-up rounds one type's `$reindex` walk runs (#1403).
const REINDEX_CATCH_UP_MAX_ROUNDS: u8 = 3;
/// Smallest catch-up margin honoured: below it every round would count as
/// "needed" and a quiescent type would run all rounds.
const REINDEX_CATCH_UP_MARGIN_MIN_MS: u64 = 1_000;
/// Largest catch-up margin honoured, so `t0 - margin` stays in range.
const REINDEX_CATCH_UP_MARGIN_MAX_MS: u64 = 86_400_000;

/// The walk position handed to the driver between calls (#1403). `v2|` and a
/// tag version the grammar; anything else is a foreign or corrupt cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReindexWalkCursor {
    Id {
        floor: DateTime<Utc>,
        after_id: String,
    },
    Round {
        round: u8,
        floor: DateTime<Utc>,
        ceiling: DateTime<Utc>,
        walked: u64,
        after_last_updated: DateTime<Utc>,
        after_id: String,
    },
}

impl ReindexWalkCursor {
    fn encode(&self) -> String {
        match self {
            ReindexWalkCursor::Id { floor, after_id } => {
                format!("v2|i|{}|{}", format_walk_instant(*floor), after_id)
            }
            ReindexWalkCursor::Round {
                round,
                floor,
                ceiling,
                walked,
                after_last_updated,
                after_id,
            } => format!(
                "v2|c|{}|{}|{}|{}|{}|{}",
                round,
                format_walk_instant(*floor),
                format_walk_instant(*ceiling),
                walked,
                format_walk_instant(*after_last_updated),
                after_id
            ),
        }
    }

    /// Anything that does not exactly match the grammar (including HEAD's
    /// `<rfc3339>|<id>` and an empty string) is `SearchError::InvalidCursor`.
    /// A cursor this process did not produce means there is a bug; restarting
    /// the type could loop forever, so the run fails instead (#1403 D5).
    fn parse(cursor: &str) -> StorageResult<Self> {
        let invalid = || {
            StorageError::Search(SearchError::InvalidCursor {
                cursor: cursor.to_string(),
            })
        };
        let rest = cursor.strip_prefix("v2|").ok_or_else(invalid)?;
        let (tag, rest) = rest.split_once('|').ok_or_else(invalid)?;
        match tag {
            "i" => {
                let (floor, after_id) = rest.split_once('|').ok_or_else(invalid)?;
                if after_id.is_empty() {
                    return Err(invalid());
                }
                let floor = DateTime::parse_from_rfc3339(floor)
                    .map_err(|_| invalid())?
                    .with_timezone(&Utc);
                Ok(ReindexWalkCursor::Id {
                    floor,
                    after_id: after_id.to_string(),
                })
            }
            "c" => {
                let fields: Vec<&str> = rest.splitn(6, '|').collect();
                let [round, floor, ceiling, walked, after_lu, after_id] = fields[..] else {
                    return Err(invalid());
                };
                if after_id.is_empty() {
                    return Err(invalid());
                }
                let round: u8 = round.parse().map_err(|_| invalid())?;
                if !(1..=REINDEX_CATCH_UP_MAX_ROUNDS).contains(&round) {
                    return Err(invalid());
                }
                let walked: u64 = walked.parse().map_err(|_| invalid())?;
                let floor = DateTime::parse_from_rfc3339(floor)
                    .map_err(|_| invalid())?
                    .with_timezone(&Utc);
                let ceiling = DateTime::parse_from_rfc3339(ceiling)
                    .map_err(|_| invalid())?
                    .with_timezone(&Utc);
                let after_last_updated = DateTime::parse_from_rfc3339(after_lu)
                    .map_err(|_| invalid())?
                    .with_timezone(&Utc);
                if !(floor < ceiling
                    && floor <= after_last_updated
                    && after_last_updated < ceiling)
                {
                    return Err(invalid());
                }
                Ok(ReindexWalkCursor::Round {
                    round,
                    floor,
                    ceiling,
                    walked,
                    after_last_updated,
                    after_id: after_id.to_string(),
                })
            }
            _ => Err(invalid()),
        }
    }
}

/// Whether round-start should run the round, declare the walk complete, or
/// stop at the round cap (#1403 §3.4 step 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoundStartDecision {
    Run,
    Complete,
    CapReached,
}

fn truncate_to_millis(dt: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(dt.timestamp_millis()).unwrap_or(dt)
}

fn format_walk_instant(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Clamps a configured margin to `[REINDEX_CATCH_UP_MARGIN_MIN_MS,
/// REINDEX_CATCH_UP_MARGIN_MAX_MS]` (#1403 §3.10).
fn reindex_catch_up_margin(configured_ms: u64) -> chrono::Duration {
    chrono::Duration::milliseconds(
        configured_ms.clamp(REINDEX_CATCH_UP_MARGIN_MIN_MS, REINDEX_CATCH_UP_MARGIN_MAX_MS) as i64,
    )
}

/// `min(newest_live + 1 ms, t0 - margin)` (#1403 §3.2).
fn reindex_catch_up_floor(
    t0: DateTime<Utc>,
    newest_live: Option<DateTime<Utc>>,
    margin: chrono::Duration,
) -> DateTime<Utc> {
    let fresh = truncate_to_millis(t0 - margin);
    match newest_live {
        Some(newest) => (newest + chrono::Duration::milliseconds(1)).min(fresh),
        None => fresh,
    }
}

/// `max(now + margin, newest_live + 1 ms)` (#1403 §3.4 step 3, decision D11).
fn reindex_catch_up_ceiling(
    now: DateTime<Utc>,
    newest_live: Option<DateTime<Utc>>,
    margin: chrono::Duration,
) -> DateTime<Utc> {
    let by_margin = truncate_to_millis(now + margin);
    match newest_live {
        Some(newest) => by_margin.max(newest + chrono::Duration::milliseconds(1)),
        None => by_margin,
    }
}

/// Whether the next round should run, or the walk is done (#1403 §3.4 step 1).
fn reindex_round_start_decision(
    round: u8,
    floor: DateTime<Utc>,
    now: DateTime<Utc>,
    margin: chrono::Duration,
) -> RoundStartDecision {
    if round == 1 {
        return RoundStartDecision::Run;
    }
    if now < floor - margin / 2 {
        return RoundStartDecision::Complete;
    }
    if round > REINDEX_CATCH_UP_MAX_ROUNDS {
        return RoundStartDecision::CapReached;
    }
    RoundStartDecision::Run
}

/// The id phase's filter: live resources older than `floor`, keyset on `id`
/// (#1403 §3.3).
fn reindex_id_page_filter(
    tenant_id: &str,
    resource_type: &str,
    floor: DateTime<Utc>,
    after_id: Option<&str>,
) -> Document {
    let mut filter = doc! {
        "tenant_id": tenant_id,
        "resource_type": resource_type,
        "is_deleted": false,
        "last_updated": { "$lt": chrono_to_bson(floor) },
    };
    if let Some(after_id) = after_id {
        filter.insert("id", doc! { "$gt": after_id });
    }
    filter
}

/// A catch-up round's filter over `[floor, ceiling)`, keyset on
/// `(last_updated, id)` once a page has been returned (#1403 §3.4).
fn reindex_catch_up_page_filter(
    tenant_id: &str,
    resource_type: &str,
    floor: DateTime<Utc>,
    ceiling: DateTime<Utc>,
    after: Option<(DateTime<Utc>, &str)>,
) -> Document {
    let mut filter = doc! {
        "tenant_id": tenant_id,
        "resource_type": resource_type,
        "is_deleted": false,
    };
    match after {
        None => {
            filter.insert(
                "last_updated",
                doc! { "$gte": chrono_to_bson(floor), "$lt": chrono_to_bson(ceiling) },
            );
        }
        Some((after_lu, after_id)) => {
            filter.insert(
                "$or",
                vec![
                    doc! { "last_updated": { "$gt": chrono_to_bson(after_lu), "$lt": chrono_to_bson(ceiling) } },
                    doc! { "last_updated": chrono_to_bson(after_lu), "id": { "$gt": after_id } },
                ],
            );
        }
    }
    filter
}

/// Keeps only the last (newest) occurrence of each `id` in `docs`, preserving
/// scan order otherwise; a document with no string `id` is kept in place and
/// left to fail parsing with HEAD's error (#1403 §5).
fn dedupe_reindex_page_keep_last(docs: Vec<Document>) -> Vec<Document> {
    let mut last_index_for_id: HashMap<String, usize> = HashMap::new();
    for (i, doc) in docs.iter().enumerate() {
        if let Ok(id) = doc.get_str("id") {
            last_index_for_id.insert(id.to_string(), i);
        }
    }
    docs.into_iter()
        .enumerate()
        .filter(|(i, doc)| match doc.get_str("id").ok() {
            Some(id) => last_index_for_id.get(id) == Some(i),
            None => true,
        })
        .map(|(_, doc)| doc)
        .collect()
}
```

Also add `chrono::SecondsFormat` to the `chrono` import:

```rust
use chrono::{DateTime, SecondsFormat, Utc};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run:
```
cargo test -p helios-persistence --features mongodb --lib reindex_walk_tests
cargo check -p helios-persistence --features mongodb --tests
```
Expected: every test in `reindex_walk_tests` passes; the crate compiles with `dead_code` warnings (not errors) on the new pure functions, since nothing calls them until Task 4.

- [ ] **Step 5: Commit**

```bash
git add crates/persistence/src/backends/mongodb/storage.rs
git commit -m "$(cat <<'EOF'
feat(mongodb): add the id-order walk's cursor grammar and pure helpers (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Wire the id-order walk into `fetch_resources_page`

This task ships in two commits so neither one is an unreviewable ~600-line unit: **Part A** (Steps 1-5) is the production walk plus the two tests that are genuinely red against unmodified HEAD (T2, T12). **Part B** (Steps 6-10) adds the legacy-parity harness and T1, the docs section, and the doc-comment relocation — all green on arrival once Part A lands, committed separately.

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/storage.rs` — imports (add `use super::schema::{RESOURCES_IDENTITY_INDEX, RESOURCES_TYPE_SCAN_INDEX};`, Part A); `impl ReindexSource for MongoBackend` block (anchor: `impl ReindexSource for MongoBackend {` `:4754`, `fetch_resources_page` `:4781-4864`, Part A); delete `parse_reindex_cursor` (anchor: `:5220-5225`, Part A); the `write_search_entries_page` precondition doc comment (Part B — see Step 8's note on where PR0 relocates it)
- Modify: `docs/mongodb/search-indexes.md` — new section before `## Composite parameters` (anchor: `:54`, Part B)
- Create: `crates/persistence/tests/mongodb/reindex_id_walk.rs` (harness + T2 + T12 in Part A; extended with the legacy-parity harness + T1 in Part B)
- Modify: `crates/persistence/tests/mongodb_tests.rs` — `#[path]` include directly after the `versioned_write_race_suite` include (anchor: `:816-817`, Part A)

**Interfaces:**
- Produces (private, `impl MongoBackend` block placed immediately before `impl ReindexSource for MongoBackend`, Part A):
  ```rust
  impl MongoBackend {
      async fn reindex_newest_live_last_updated(
          &self,
          resources: &Collection<Document>,
          tenant_id: &str,
          resource_type: &str,
      ) -> StorageResult<Option<DateTime<Utc>>>;

      async fn reindex_find_page(
          &self,
          resources: &Collection<Document>,
          filter: Document,
          sort: Document,
          hint: &str,
          limit: u32,
      ) -> StorageResult<Vec<Document>>;
  }
  ```
  and rewrites `impl ReindexSource for MongoBackend { async fn fetch_resources_page(..) -> StorageResult<ResourcePage> }` to run the two-phase walk (§3.5 loop), using `ReindexWalkCursor`/`RoundStartDecision`/the filter builders/`dedupe_reindex_page_keep_last` from Task 3.
- Consumes: `RESOURCES_IDENTITY_INDEX`, `RESOURCES_TYPE_SCAN_INDEX` (Task 2, via the `use super::schema::{..};` import above); every item from Task 3's Interfaces list.

- [ ] **Step 1 (Part A): Write the failing tests**

Create `crates/persistence/tests/mongodb/reindex_id_walk.rs`:

```rust
//! #1403: the id-order `$reindex` walk and its catch-up rounds.
//!
//! Uses `super::*` for the parent test crate's imports and private harness
//! helpers (`create_backend`, `create_tenant`, `build_test_database_name`,
//! etc.) — this file is a `#[path]`-included child module of
//! `mongodb_tests.rs`, not a standalone test binary.

use super::*;
use std::collections::{BTreeMap, BTreeSet};
use mongodb::bson::{DateTime as BsonDateTime, Document};
use helios_persistence::error::StorageResult;
use helios_persistence::search::{ReindexOperation, ReindexRequest, ReindexSource, ReindexTarget};

// `StorageResult` and `ReindexOperation`/`ReindexRequest` are used bare (not
// fully qualified) throughout this file's test doubles and tests below.
// `ReindexSource`/`ReindexTarget` must be in scope for their methods to be
// callable as `x.fetch_resources_page(..)` / `x.write_search_entries_page(..)`
// etc. — Rust only resolves a trait method by dot-call syntax when the trait
// itself is imported, even though the concrete type (`MongoBackend`) already
// implements it; without this import block, every such call is E0599 ("no
// method named ... found — the following trait is implemented but not in
// scope"). `ReindexStatus`, `ResourcePage` and `TenantSearchRegistries` are
// deliberately NOT imported here: every use of them in this file is already
// fully qualified (`helios_persistence::search::ReindexStatus::Completed`
// etc.), so importing the bare name would be an `unused_imports` error under
// `-D warnings` — they are plain types, not traits, so (unlike
// `ReindexSource`/`ReindexTarget`) a fully-qualified reference elsewhere does
// not count as "using" a bare import of them.

// ===========================================================================
// Harness
// ===========================================================================

/// Live and tombstoned ids per type, as seeded by [`seed_walk_fixture`].
struct WalkFixture {
    live: BTreeMap<String, BTreeSet<String>>,
    tombstones: BTreeMap<String, BTreeSet<String>>,
}

/// Seeds one tenant with the fixture #1403's tests share: a fixed set of
/// Patients exercising FHIR id ordering (`-` < `.` < digits < upper < lower),
/// `observations` Observations, and tombstones on two Patients and every
/// tenth Observation. Ids are NOT yet backdated — call [`backdate_fixture`]
/// separately so a test can inspect CRUD-time snapshots first.
async fn seed_walk_fixture(
    backend: &MongoBackend,
    tenant: &TenantContext,
    observations: usize,
    extra_patient: &str,
) -> WalkFixture {
    let mut live: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut tombstones: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    let mut patient_ids: Vec<String> = [
        "-lead", "0", "9.9", "A-1", "A.1", "Z", "a-1", "a.1", "aa", "z", "zz-9",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    patient_ids.push(extra_patient.to_string());

    for id in &patient_ids {
        backend
            .create(
                tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": id,
                    "name": [{ "family": id }],
                    "identifier": [{ "system": "urn:walk", "value": format!("p-{id}") }],
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    for i in 0..observations {
        let id = format!("obs-{i:03}");
        let mut body = json!({
            "resourceType": "Observation",
            "id": id,
            "status": "final",
            "code": { "coding": [{ "system": "http://loinc.org", "code": "8867-4" }] },
            "subject": { "reference": "Patient/A-1" },
            "effectiveDateTime": "2020-01-01",
            "valueQuantity": { "value": i, "unit": "/min" },
            "identifier": [{ "system": "urn:walk", "value": format!("o-{i}") }],
        });
        if i % 10 == 3 {
            body["contained"] = json!([{
                "resourceType": "Patient",
                "id": "p1",
                "name": [{ "family": format!("Contained{i}") }],
            }]);
            body["performer"] = json!([{ "reference": "#p1" }]);
        }
        backend
            .create(tenant, "Observation", body, FhirVersion::default())
            .await
            .unwrap();
    }

    for id in ["Z", "a.1"] {
        backend.delete(tenant, "Patient", id).await.unwrap();
        tombstones.entry("Patient".to_string()).or_default().insert(id.to_string());
    }
    for i in 0..observations {
        if i % 100 == 5 {
            let id = format!("obs-{i:03}");
            backend.delete(tenant, "Observation", &id).await.unwrap();
            tombstones
                .entry("Observation".to_string())
                .or_default()
                .insert(id);
        }
    }

    let patient_tombstones = tombstones.get("Patient").cloned().unwrap_or_default();
    live.insert(
        "Patient".to_string(),
        patient_ids
            .iter()
            .filter(|id| !patient_tombstones.contains(*id))
            .cloned()
            .collect(),
    );
    let obs_tombstones = tombstones.get("Observation").cloned().unwrap_or_default();
    live.insert(
        "Observation".to_string(),
        (0..observations)
            .map(|i| format!("obs-{i:03}"))
            .filter(|id| !obs_tombstones.contains(id))
            .collect(),
    );

    WalkFixture { live, tombstones }
}

/// Raw `update_many` on `resources` that backdates every id in `fixture`
/// (live and tombstoned) so the fast-load shape holds: three groups of equal
/// `last_updated`, interleaved with id order. Observation `i` goes to second
/// `i % 3`; every Patient goes to second 3.
async fn backdate_fixture(backend: &MongoBackend, tenant: &TenantContext, fixture: &WalkFixture) {
    let db = backend.get_database().await.unwrap();
    let resources = db.collection::<Document>("resources");
    let tenant_id = tenant.tenant_id().as_str();

    let mut patient_ids: Vec<String> = fixture
        .live
        .get("Patient")
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect();
    patient_ids.extend(fixture.tombstones.get("Patient").cloned().unwrap_or_default());
    if !patient_ids.is_empty() {
        resources
            .update_many(
                doc! {
                    "tenant_id": tenant_id,
                    "resource_type": "Patient",
                    "id": { "$in": &patient_ids },
                },
                doc! {
                    "$set": {
                        "last_updated": BsonDateTime::from_millis(
                            ts("2020-01-01T00:00:03.000Z").timestamp_millis(),
                        ),
                    },
                },
            )
            .await
            .unwrap();
    }

    let mut obs_by_group: [Vec<String>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut all_obs: BTreeSet<String> = fixture.live.get("Observation").cloned().unwrap_or_default();
    all_obs.extend(fixture.tombstones.get("Observation").cloned().unwrap_or_default());
    for id in &all_obs {
        let i: usize = id.trim_start_matches("obs-").parse().unwrap();
        obs_by_group[i % 3].push(id.clone());
    }
    for (group, ids) in obs_by_group.iter().enumerate() {
        if ids.is_empty() {
            continue;
        }
        resources
            .update_many(
                doc! {
                    "tenant_id": tenant_id,
                    "resource_type": "Observation",
                    "id": { "$in": ids },
                },
                doc! {
                    "$set": {
                        "last_updated": BsonDateTime::from_millis(
                            ts(&format!("2020-01-01T00:00:0{group}.000Z")).timestamp_millis(),
                        ),
                    },
                },
            )
            .await
            .unwrap();
    }
}

fn ts(s: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&chrono::Utc)
}

// ===========================================================================
// T2: order and isolation
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_returns_each_live_resource_once_in_byte_order() {
    let Some(backend) = create_backend("reindex_id_walk_order").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant_a = create_tenant("walk-ord-a");
    let tenant_b = create_tenant("walk-ord-b");
    let fixture_a = seed_walk_fixture(&backend, &tenant_a, 20, "only-in-a").await;
    let fixture_b = seed_walk_fixture(&backend, &tenant_b, 20, "only-in-b").await;
    backdate_fixture(&backend, &tenant_a, &fixture_a).await;
    backdate_fixture(&backend, &tenant_b, &fixture_b).await;

    async fn walk_all(backend: &MongoBackend, tenant: &TenantContext, resource_type: &str, limit: u32) -> Vec<String> {
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..40 {
            let page = backend
                .fetch_resources_page(tenant, resource_type, cursor.as_deref(), limit)
                .await
                .unwrap();
            let empty = page.resources.is_empty();
            ids.extend(page.resources.iter().map(|r| r.id().to_string()));
            match page.next_cursor {
                Some(next) => {
                    assert!(!empty, "every page but the trailing one must return at least one resource");
                    cursor = Some(next);
                }
                None => {
                    assert!(empty, "the trailing page must be empty");
                    return ids;
                }
            }
        }
        panic!("walk did not terminate within 40 pages");
    }

    let patient_ids = walk_all(&backend, &tenant_a, "Patient", 3).await;
    let expected_patients: Vec<String> = fixture_a.live.get("Patient").unwrap().iter().cloned().collect();
    assert_eq!(patient_ids, expected_patients);
    assert!(patient_ids.contains(&"only-in-a".to_string()));
    assert!(!patient_ids.contains(&"only-in-b".to_string()));
    assert!(!patient_ids.contains(&"Z".to_string()));
    assert!(!patient_ids.contains(&"a.1".to_string()));

    let obs_ids = walk_all(&backend, &tenant_a, "Observation", 7).await;
    let expected_obs: Vec<String> = fixture_a.live.get("Observation").unwrap().iter().cloned().collect();
    assert_eq!(obs_ids, expected_obs);
    assert_eq!(obs_ids.len(), 19);
    assert!(!obs_ids.contains(&"obs-005".to_string()));
    let _ = fixture_b; // seeded only to prove isolation via the assertions above
}

// ===========================================================================
// T12: a foreign or corrupt cursor is rejected
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_rejects_a_foreign_cursor() {
    let Some(backend) = create_backend("reindex_id_walk_bad_cursor").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("walk-bad-cursor");
    backend
        .create(
            &tenant,
            "Patient",
            json!({ "resourceType": "Patient", "id": "p1", "name": [{ "family": "X" }] }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let head_style = Some("2026-09-19T04:43:29.668+00:00|e357ce58-f379-216d-a369-99da40ff76ae");
    let err = backend
        .fetch_resources_page(&tenant, "Patient", head_style, 10)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        StorageError::Search(SearchError::InvalidCursor { .. })
    ));

    let err = backend
        .fetch_resources_page(&tenant, "Patient", Some("garbage"), 10)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        StorageError::Search(SearchError::InvalidCursor { .. })
    ));
}
```

Register the module in `crates/persistence/tests/mongodb_tests.rs`, directly after the `versioned_write_race_suite` include:

```rust
#[path = "search/versioned_write_race_suite.rs"]
mod versioned_write_race_suite;

/// #1403: the id-order `$reindex` walk and its catch-up rounds.
#[path = "mongodb/reindex_id_walk.rs"]
mod reindex_id_walk;
```

This is Part A of Task 4: the production walk itself, plus the two tests that are genuinely red against unmodified HEAD. Part B (Steps 6-10, below) adds the legacy-parity harness, T1, the docs section and the doc-comment relocation — all of it green on arrival once Part A lands, so it is committed separately rather than folded into this ~600-line step.

- [ ] **Step 2: Run the new tests to see them fail**

Docker or `HFS_TEST_MONGODB_URL` is required for this step — see the Global Constraints note on Docker-gated steps; do not accept a `Skipping` line as satisfying it.

Run:
```
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_id_walk 2>&1 \
  | tee "$LOGDIR/reindex_id_walk_task4a_red.log"
grep -q "Skipping" "$LOGDIR/reindex_id_walk_task4a_red.log" && { echo "NOT RUN: Docker/HFS_TEST_MONGODB_URL unavailable"; exit 1; }
```
Expected: both tests FAIL — `mongodb_reindex_id_walk_returns_each_live_resource_once_in_byte_order` (T2) because HEAD orders each type by `(last_updated, id)` over three interleaved groups rather than by `id`, so `patient_ids`/`obs_ids` do not equal the expected byte-order lists; `mongodb_reindex_id_walk_rejects_a_foreign_cursor` (T12) because HEAD's `parse_reindex_cursor` silently treats an unparseable cursor as "start over" and returns `Ok` instead of `Err(StorageError::Search(SearchError::InvalidCursor { .. }))`.

- [ ] **Step 3 (Part A): Implement**

In `storage.rs`, add `RESOURCES_IDENTITY_INDEX` and `RESOURCES_TYPE_SCAN_INDEX` to the import block, next to `use super::MongoBackend;`:

```rust
use super::MongoBackend;
use super::schema::{RESOURCES_IDENTITY_INDEX, RESOURCES_TYPE_SCAN_INDEX};
```

Without this import, `RESOURCES_IDENTITY_INDEX`/`RESOURCES_TYPE_SCAN_INDEX` below are unresolved (E0425) — `schema` is `pub(crate) mod schema;` in `mod.rs`, so the constants are reachable from `storage.rs` (a sibling module in the same crate) but are not brought into scope automatically.

Delete `parse_reindex_cursor` (`:5220-5225`).

Add, in a new `impl MongoBackend` block placed immediately before `impl ReindexSource for MongoBackend`:

```rust
impl MongoBackend {
    /// The newest-live probe (#1403 §3.2): a covered reverse scan of
    /// `idx_resources_type_scan` for the `last_updated` of the newest live
    /// resource of `resource_type`, or `None` if it has no live resource.
    async fn reindex_newest_live_last_updated(
        &self,
        resources: &Collection<Document>,
        tenant_id: &str,
        resource_type: &str,
    ) -> StorageResult<Option<DateTime<Utc>>> {
        let found = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "is_deleted": false,
            })
            .sort(doc! { "last_updated": -1, "id": -1 })
            .projection(doc! { "_id": 0, "last_updated": 1 })
            .hint(Hint::Name(RESOURCES_TYPE_SCAN_INDEX.to_string()))
            .await
            .map_err(|e| internal_error(format!("Failed to probe newest resource: {e}")))?;
        match found {
            Some(doc) => {
                let ts = doc
                    .get_datetime("last_updated")
                    .map_err(|e| internal_error(format!("Missing last_updated: {e}")))?;
                Ok(Some(bson_to_chrono(ts)))
            }
            None => Ok(None),
        }
    }

    /// One hinted, sorted, limited find, fully drained (#1403 §3.3, §3.4).
    async fn reindex_find_page(
        &self,
        resources: &Collection<Document>,
        filter: Document,
        sort: Document,
        hint: &str,
        limit: u32,
    ) -> StorageResult<Vec<Document>> {
        let mut stream = resources
            .find(filter)
            .sort(sort)
            .limit(limit as i64)
            .hint(Hint::Name(hint.to_string()))
            .await
            .map_err(|e| internal_error(format!("Failed to fetch resources: {e}")))?;

        let mut docs = Vec::new();
        while stream
            .advance()
            .await
            .map_err(|e| internal_error(format!("Failed to advance cursor: {e}")))?
        {
            docs.push(
                stream
                    .deserialize_current()
                    .map_err(|e| internal_error(format!("Failed to read resource: {e}")))?,
            );
        }
        Ok(docs)
    }
}

/// One step of the walk inside a single call (#1403 §3.5); never leaves the
/// call — only `ReindexWalkCursor::Id`/`Round` do, as an encoded cursor.
enum WalkStep {
    Start,
    IdPhase {
        floor: DateTime<Utc>,
        after_id: Option<String>,
    },
    RoundStart {
        round: u8,
        floor: DateTime<Utc>,
    },
    Round {
        round: u8,
        floor: DateTime<Utc>,
        ceiling: DateTime<Utc>,
        walked: u64,
        after: Option<(DateTime<Utc>, String)>,
    },
}

impl From<ReindexWalkCursor> for WalkStep {
    fn from(cursor: ReindexWalkCursor) -> Self {
        match cursor {
            ReindexWalkCursor::Id { floor, after_id } => WalkStep::IdPhase {
                floor,
                after_id: Some(after_id),
            },
            ReindexWalkCursor::Round {
                round,
                floor,
                ceiling,
                walked,
                after_last_updated,
                after_id,
            } => WalkStep::Round {
                round,
                floor,
                ceiling,
                walked,
                after: Some((after_last_updated, after_id)),
            },
        }
    }
}

/// Converts a returned page plus its next cursor into a [`ResourcePage`],
/// exactly as HEAD's `fetch_resources_page` did (`:4851-4863` at c86d0f08b).
fn reindex_page_from_docs(
    docs: &[Document],
    resource_type: &str,
    tenant: &TenantContext,
    next_cursor: ReindexWalkCursor,
) -> StorageResult<ResourcePage> {
    let resources = docs
        .iter()
        .map(|doc| {
            parse_history_row(doc, Some(resource_type), None)
                .map(|row| row.into_stored_resource(tenant))
        })
        .collect::<StorageResult<Vec<_>>>()?;
    Ok(ResourcePage {
        resources,
        next_cursor: Some(next_cursor.encode()),
        skipped: Vec::new(),
    })
}
```

Replace HEAD's `fetch_resources_page` (`:4781-4864`) with:

```rust
    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
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
                    let docs = self
                        .reindex_find_page(
                            &resources,
                            filter,
                            doc! { "id": 1 },
                            RESOURCES_IDENTITY_INDEX,
                            limit,
                        )
                        .await?;
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
                    let scanned = self
                        .reindex_find_page(
                            &resources,
                            filter,
                            doc! { "last_updated": 1, "id": 1 },
                            RESOURCES_TYPE_SCAN_INDEX,
                            limit,
                        )
                        .await?;
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

Add, right above the new method, a short overview comment replacing whatever comment HEAD's old keyset carried:

```rust
    /// Two phases per type (#1403): an id phase over live resources stamped
    /// before the floor, keyset on id and hinted to idx_resources_identity,
    /// then up to REINDEX_CATCH_UP_MAX_ROUNDS catch-up rounds over
    /// [floor, ceiling) in (last_updated, id) order on idx_resources_type_scan.
    /// A phase ends only on an empty query and the next phase starts in the
    /// same call, so the driver sees non-empty pages with Some(cursor) and one
    /// trailing empty page with None. The cursor is the versioned v2 grammar
    /// of ReindexWalkCursor.
```

- [ ] **Step 4 (Part A): Run the tests to verify they pass**

Docker or `HFS_TEST_MONGODB_URL` is required — see the Global Constraints note on Docker-gated steps.

Run:
```
cargo test -p helios-persistence --features mongodb --lib reindex_walk_tests
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_id_walk 2>&1 \
  | tee "$LOGDIR/reindex_id_walk_task4a_green.log"
grep -q "Skipping" "$LOGDIR/reindex_id_walk_task4a_green.log" && { echo "NOT RUN"; exit 1; }
```
Expected: `test result: ok. 2 passed` for `reindex_id_walk` (T2, T12 — this is all that is registered so far); every `reindex_walk_tests` unit test still passes. Also re-run the always-green Task 1-3 checks (`cargo check -p helios-hfs --features mongodb --tests`) to confirm nothing else broke.

- [ ] **Step 5 (Part A): Commit**

```bash
git add crates/persistence/src/backends/mongodb/storage.rs crates/persistence/tests/mongodb_tests.rs crates/persistence/tests/mongodb/reindex_id_walk.rs
git commit -m "$(cat <<'EOF'
feat(mongodb): walk $reindex in id order with catch-up rounds (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 6 (Part B): Add the legacy-parity harness and T1**

Everything below is a regression/pinning addition, green on arrival — Part A already implements the whole walk, so there is no new production code left to write. If this step's test fails, that is a Part A defect to reopen, not something to patch here.

Append to `crates/persistence/tests/mongodb/reindex_id_walk.rs`, after the `T12` section:

```rust
/// All rows of `search_index` and `search_index_contained` for `tenant_id`,
/// as canonical sorted JSON strings (order-independent, `_id`-independent).
async fn index_rows(
    db: &mongodb::Database,
    collection: &str,
    tenant_id: &str,
    strip_tenant: bool,
) -> Vec<String> {
    use futures::stream::TryStreamExt;
    let coll = db.collection::<Document>(collection);
    let mut rows: Vec<Document> = coll
        .find(doc! { "tenant_id": tenant_id })
        .projection(doc! { "_id": 0 })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    if strip_tenant {
        for row in &mut rows {
            row.remove("tenant_id");
        }
    }
    let mut lines: Vec<String> = rows
        .into_iter()
        .map(|d| canonical(mongodb::bson::Bson::Document(d).into_relaxed_extjson()).to_string())
        .collect();
    lines.sort();
    lines
}

/// Rebuilds `v` with every object's keys sorted, recursively, so two BSON
/// documents with the same content but different field insertion order
/// snapshot identically (#1403 §5). `serde_json`'s `preserve_order` feature is
/// enabled workspace-wide (`crates/sof/Cargo.toml`, and `helios-persistence`
/// depends on `helios-sof`), so without this a `Value::Object`'s iteration
/// order otherwise follows BSON insertion order rather than being sorted.
fn canonical(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<(String, serde_json::Value)> =
                map.into_iter().map(|(k, v)| (k, canonical(v))).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            serde_json::Value::Object(entries.into_iter().collect())
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(canonical).collect())
        }
        other => other,
    }
}

async fn snapshot(
    db: &mongodb::Database,
    tenant_id: &str,
    strip_tenant: bool,
) -> (Vec<String>, Vec<String>) {
    (
        index_rows(db, "search_index", tenant_id, strip_tenant).await,
        index_rows(db, "search_index_contained", tenant_id, strip_tenant).await,
    )
}

/// Routes `helios_persistence::backends::mongodb::storage` events at `debug`
/// and above into `tracing-test`'s global buffer, once per test binary. Every
/// walk test that asserts on log lines calls this before it starts its walk.
fn capture_walk_logs() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let writer =
            tracing_test::internal::MockWriter::new(tracing_test::internal::global_buf());
        let dispatch = tracing_test::internal::get_subscriber(
            writer,
            "helios_persistence::backends::mongodb::storage=debug",
        );
        tracing::dispatcher::set_global_default(dispatch)
            .expect("no other global tracing subscriber in this test binary");
    });
}

/// Captured log lines containing every one of `needles`.
fn walk_log_lines(needles: &[&str]) -> Vec<String> {
    let buf = tracing_test::internal::global_buf().lock().unwrap();
    String::from_utf8_lossy(&buf)
        .lines()
        .filter(|line| needles.iter().all(|needle| line.contains(needle)))
        .map(str::to_string)
        .collect()
}

/// HEAD's walk, verbatim and test-only: `(last_updated, id)` keyset, no hint,
/// `"{rfc3339}|{id}"` cursor. Copied rather than reused because PR1 replaces
/// the production implementation.
struct LegacyWalkSource {
    backend: std::sync::Arc<MongoBackend>,
}

#[async_trait::async_trait]
impl helios_persistence::search::ReindexSource for LegacyWalkSource {
    async fn list_resource_types(
        &self,
        tenant: &TenantContext,
    ) -> StorageResult<Vec<String>> {
        self.backend.list_resource_types(tenant).await
    }

    async fn count_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        self.backend.count_resources(tenant, resource_type).await
    }

    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<helios_persistence::search::ResourcePage> {
        let db = self.backend.get_database().await?;
        let resources = db.collection::<Document>("resources");

        let mut stream = resources
            .find(legacy_filter(tenant, resource_type, cursor))
            .sort(doc! { "last_updated": 1, "id": 1 })
            .limit(limit as i64)
            .await
            .map_err(|e| StorageError::Backend(BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message: format!("legacy walk find: {e}"),
                source: None,
            }))?;
        let mut docs: Vec<Document> = Vec::new();
        while stream.advance().await.map_err(|e| {
            StorageError::Backend(BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message: format!("legacy walk advance: {e}"),
                source: None,
            })
        })? {
            docs.push(stream.deserialize_current().map_err(|e| {
                StorageError::Backend(BackendError::Internal {
                    backend_name: "mongodb".to_string(),
                    message: format!("legacy walk deserialize: {e}"),
                    source: None,
                })
            })?);
        }

        let full_page = docs.len() as u32 == limit;
        let next_cursor = match (full_page, docs.last()) {
            (true, Some(last)) => {
                let dt = last.get_datetime("last_updated").unwrap();
                let id = last.get_str("id").unwrap();
                let lu = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(dt.timestamp_millis()).unwrap();
                Some(format!("{}|{}", lu.to_rfc3339(), id))
            }
            _ => None,
        };

        let resources_out: StorageResult<Vec<_>> = docs
            .iter()
            .map(|d| {
                let dt = d.get_datetime("last_updated").unwrap();
                let lu = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(dt.timestamp_millis()).unwrap();
                let data = d.get_document("data").unwrap();
                let content: serde_json::Value =
                    mongodb::bson::from_document(data.clone()).unwrap();
                Ok(helios_persistence::types::StoredResource::from_storage(
                    resource_type,
                    d.get_str("id").unwrap(),
                    d.get_str("version_id").unwrap(),
                    tenant.tenant_id().clone(),
                    content,
                    lu,
                    lu,
                    None,
                    FhirVersion::default(),
                ))
            })
            .collect();

        Ok(helios_persistence::search::ResourcePage {
            resources: resources_out?,
            next_cursor,
            skipped: Vec::new(),
        })
    }
}

/// HEAD's exact `fetch_resources_page` filter (`storage.rs:4790-4809` at
/// c86d0f08b, before PR1 replaces it): `(last_updated, id)` keyset, no hint.
fn legacy_filter(tenant: &TenantContext, resource_type: &str, cursor: Option<&str>) -> Document {
    let mut filter = doc! {
        "tenant_id": tenant.tenant_id().as_str(),
        "resource_type": resource_type,
        "is_deleted": false,
    };
    if let Some(cursor) = cursor {
        if let Some((ts_str, id)) = cursor.split_once('|') {
            if let Ok(cur_dt) = chrono::DateTime::parse_from_rfc3339(ts_str) {
                let cur_dt = cur_dt.with_timezone(&chrono::Utc);
                filter.insert(
                    "$or",
                    vec![
                        doc! { "last_updated": { "$gt": BsonDateTime::from_millis(cur_dt.timestamp_millis()) } },
                        doc! {
                            "last_updated": BsonDateTime::from_millis(cur_dt.timestamp_millis()),
                            "id": { "$gt": id },
                        },
                    ],
                );
            }
        }
    }
    filter
}

// ===========================================================================
// T1: parity with HEAD's walk
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_matches_the_legacy_walk_row_for_row() {
    use std::sync::Arc;

    let Some(backend) = create_backend("reindex_id_walk_parity").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    capture_walk_logs();

    let tenant_a = create_tenant("walk-a");
    let tenant_b = create_tenant("walk-b");
    let fixture_a = seed_walk_fixture(&backend, &tenant_a, 300, "only-in-a").await;
    let fixture_b = seed_walk_fixture(&backend, &tenant_b, 300, "only-in-b").await;

    let db = backend.get_database().await.unwrap();
    let s_crud_a = snapshot(&db, "walk-a", false).await;
    let s_crud_b = snapshot(&db, "walk-b", false).await;

    backdate_fixture(&backend, &tenant_a, &fixture_a).await;
    backdate_fixture(&backend, &tenant_b, &fixture_b).await;

    let regs = backend.tenant_registries().clone();
    let request = || {
        ReindexRequest::for_types(["Observation", "Patient"])
            .clear_existing()
            .with_batch_size(40)
    };

    let legacy_source = Arc::new(LegacyWalkSource { backend: backend.clone() });
    let legacy_op = ReindexOperation::with_parts(legacy_source, vec![backend.clone()], regs.clone());
    let legacy_job = legacy_op.start(tenant_a.clone(), request(), None).await.unwrap();
    let legacy_progress = wait_for_terminal(&legacy_op, &legacy_job).await;
    let s_old = snapshot(&db, "walk-a", false).await;

    let new_op = ReindexOperation::new(backend.clone(), regs.clone());
    let new_job = new_op.start(tenant_a.clone(), request(), None).await.unwrap();
    let new_progress = wait_for_terminal(&new_op, &new_job).await;
    let s_new = snapshot(&db, "walk-a", false).await;

    assert_eq!(s_new, s_old, "new walk must match HEAD's walk row for row");
    assert_eq!(
        s_new, s_crud_a,
        "reindex must match CRUD indexing exactly (#1064) — if only this \
         assertion fails, stop and report; do not change the writer"
    );
    assert_eq!(snapshot(&db, "walk-b", false).await, s_crud_b);

    for progress in [&legacy_progress, &new_progress] {
        assert_eq!(progress.status, helios_persistence::search::ReindexStatus::Completed);
        assert!(progress.errors.is_empty());
        assert_eq!(progress.processed_resources, progress.total_resources);
        assert_eq!(progress.processed_resources, 297 + 10);
    }
    assert_eq!(legacy_progress.entries_created, new_progress.entries_created);

    let obs_started =
        walk_log_lines(&["tenant=walk-a", "resource_type=Observation", "mongodb reindex walk started"]);
    assert!(
        obs_started.iter().any(|l| l.contains("newest_live=2020-01-01T00:00:02.000Z")
            && l.contains("floor=2020-01-01T00:00:02.001Z")),
        "{obs_started:?}"
    );
    let patient_started =
        walk_log_lines(&["tenant=walk-a", "resource_type=Patient", "mongodb reindex walk started"]);
    assert!(
        patient_started.iter().any(|l| l.contains("newest_live=2020-01-01T00:00:03.000Z")
            && l.contains("floor=2020-01-01T00:00:03.001Z")),
        "{patient_started:?}"
    );
    for rt in ["Observation", "Patient"] {
        let finished = walk_log_lines(&[
            "tenant=walk-a",
            &format!("resource_type={rt}"),
            "mongodb reindex catch-up round finished",
            "round=1",
        ]);
        assert!(finished.iter().any(|l| l.contains("walked=0")), "{rt}: {finished:?}");
    }

    // Rerun without clear_existing: no duplicates should appear.
    let rerun_job = new_op.start(tenant_a.clone(), ReindexRequest::for_types(["Observation", "Patient"]).with_batch_size(40), None).await.unwrap();
    wait_for_terminal(&new_op, &rerun_job).await;
    assert_eq!(snapshot(&db, "walk-a", false).await, s_crud_a);
}

async fn wait_for_terminal(
    op: &helios_persistence::search::ReindexOperation,
    job_id: &str,
) -> helios_persistence::search::ReindexProgress {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let progress = op.get_progress(job_id).await.unwrap();
            if progress.status.is_finished() {
                return progress;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("reindex status should become terminal")
}
```

- [ ] **Step 7 (Part B): Run — expect an immediate pass**

Docker or `HFS_TEST_MONGODB_URL` is required.

Run:
```
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_id_walk 2>&1 \
  | tee "$LOGDIR/reindex_id_walk_task4b.log"
grep -q "Skipping" "$LOGDIR/reindex_id_walk_task4b.log" && { echo "NOT RUN"; exit 1; }
```
Expected: `test result: ok. 3 passed` (T1, T2, T12). T1 is a regression/pinning test, not a red step — Part A already implemented the full walk, including every log line T1 checks, so it should pass on first run. If it does not, that is a Part A defect: stop and report the exact assertion and captured values rather than patching the walk here.

- [ ] **Step 8 (Part B): Doc comment relocation and the docs section**

`crates/persistence/src/backends/mongodb/storage.rs`'s `write_search_entries_page` doc comment (HEAD c86d0f08b `:4980-4985`, the "Precondition: ..." paragraph) is where this task's Files section originally anchored the precondition update. **PR0 (S1 §7.4) renames this method to `write_search_entries_page_timed`, moves this whole doc comment onto it verbatim, and adds a new, short-doc `write_search_entries_page` delegate in its place.** Since this plan branches from `origin/main` *after* PR0 merges (Global Constraints), the method actually carrying the "Precondition: ..." paragraph at implementation time is `write_search_entries_page_timed`, not `write_search_entries_page`. Find it with `tgrep -n -F -- "Precondition:" C:/Users/DougC/Code/Helios/hfs` rather than trusting the old line numbers or the pre-PR0 method name, and edit whichever method the grep shows carries it (expected: `write_search_entries_page_timed`). Replace that paragraph with:

```rust
    /// Precondition: `resources` must hold each `(resource_type, id)` at most
    /// once; unlike Elasticsearch's `_id`-keyed upsert, a repeated id here
    /// would double-insert, because the delete for the whole page runs once,
    /// up front. The production caller, `fetch_resources_page`, guarantees
    /// it: an id-phase page walks the unique `idx_resources_identity` in key
    /// order, and a catch-up page is de-duplicated by id (keeping the newest
    /// version) before it is returned (#1403). The same resource in two
    /// different calls is expected — a catch-up round rewrites what the id
    /// phase wrote.
```

Add to `docs/mongodb/search-indexes.md`, before `## Composite parameters`:

```markdown
## How `$reindex` walks a type (#1403)

`$reindex` and the rebuild that follows a fast-load import page each resource type in two phases:

1. **Id order.** Every live resource whose `last_updated` is older than the walk's *floor*, in `id` order on `idx_resources_identity`. Every `search_index` key contains the resource id, so this order keeps each index's inserts clustered instead of random. That is the difference between a cache-resident rebuild and the 17-hour Observation rebuild of #1403.
2. **Catch-up rounds.** Every live resource stamped at or after the floor, in `last_updated` order on `idx_resources_type_scan`. A round stops at a ceiling of *round start + margin*, or just past the newest live `last_updated` if a resource is stamped later than that. A round ends only when a query finds nothing more in its range. Another round runs, three at most, when the previous one took longer than half the margin.

The floor is the earlier of two times: the type's newest live `last_updated` at the start plus 1 ms, and the start time minus the margin (two minutes). On a type nobody writes to while it is walked, every resource is written exactly once.

A resource may be updated while the page holding its old version is being written. It is then read again after the update and indexed from its newest version. In the id-order phase that guarantee is exact, given two assumptions:

- the clocks of the HFS processes writing to one database agree within one minute;
- a write commits within one minute of its `last_updated`. MongoDB aborts a transaction after `transactionLifetimeLimitSeconds`, 60 s by default. A bulk-ingest batch is stamped when it is planned and can take longer under memory pressure, so do not run `$reindex` over a live non-deferred import of the same type. The walk reads from the primary.

Inside a catch-up round, the guarantee holds as long as the update is stamped later than the documents already on the page.

Outside those limits, a resource can keep stale search rows until it is next written or reindexed. The same holds when a resource is written again while the last round runs; the log then says `mongodb reindex catch-up stopped at its round limit`. That warning is expected and harmless when an import of the same type overlaps the rebuild: the follow-up generation (logged as `merged deferred reindex work into the pending generation`) walks the type again. A resource deleted while its page is being written can keep search rows. Searches never return it, because they only read live resources.

`mongodb reindex found live resources stamped in the future` means that some live resources carry a `last_updated` later than the walk's start plus the margin, usually from an HFS node whose clock ran ahead. They are still indexed.

The walk's position lives in memory: after a restart, a rebuild starts every type from the beginning.
```

- [ ] **Step 9 (Part B): Run the full suite to verify nothing broke**

```
cargo test -p helios-persistence --features mongodb --lib reindex_walk_tests
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_id_walk 2>&1 \
  | tee "$LOGDIR/reindex_id_walk_task4b_final.log"
grep -q "Skipping" "$LOGDIR/reindex_id_walk_task4b_final.log" && { echo "NOT RUN"; exit 1; }
```
Expected: `test result: ok. 3 passed` (T1, T2, T12); every `reindex_walk_tests` unit test still passes.

- [ ] **Step 10 (Part B): Commit**

```bash
git add crates/persistence/src/backends/mongodb/storage.rs docs/mongodb/search-indexes.md crates/persistence/tests/mongodb/reindex_id_walk.rs
git commit -m "$(cat <<'EOF'
test(mongodb): pin id-order $reindex walk parity with HEAD's walk (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Query-plan coverage (T9)

**Files:**
- Modify: `crates/persistence/tests/mongodb/reindex_id_walk.rs` (add T9; no changes to `mongodb_tests.rs` — T9's inline explain-key-list idiom is modeled on, but does not modify, the unrelated `mongodb_history_type_plan_is_a_bounded_index_walk` test's own inline copy)

**Interfaces:** none new; this test uses Task 4's harness (`seed_walk_fixture`, `backdate_fixture`, `create_backend`, `create_tenant`) and the production `backend.fetch_resources_page(..)` directly.

- [ ] **Step 1: Write the test**

T9 is a regression/pinning test, green on arrival — Task 4 already implements the whole walk. Append to `crates/persistence/tests/mongodb/reindex_id_walk.rs`:

```rust
// ===========================================================================
// T9: plans — hint honoured, no blocking sort, no from-floor re-scan (#1021)
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_pages_plan_without_a_blocking_sort() {
    use futures::stream::TryStreamExt;

    let Some(backend) = create_backend("reindex_id_walk_plan").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("walk-plan");
    let fixture = seed_walk_fixture(&backend, &tenant, 300, "walk-plan-extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;

    let db = backend.get_database().await.unwrap();
    let resources = db.collection::<Document>("resources");
    let tail_ids: Vec<String> = (250..300).map(|i| format!("obs-{i:03}")).collect();
    resources
        .update_many(
            doc! {
                "tenant_id": "walk-plan",
                "resource_type": "Observation",
                "id": { "$in": &tail_ids },
            },
            doc! {
                "$set": {
                    "last_updated": BsonDateTime::from_millis(
                        (chrono::Utc::now() - chrono::Duration::seconds(10)).timestamp_millis(),
                    ),
                },
            },
        )
        .await
        .unwrap();

    let profiling_enabled = db.run_command(doc! { "profile": 2_i32 }).await.is_ok();
    if !profiling_enabled {
        eprintln!(
            "mongodb_reindex_id_walk_pages_plan_without_a_blocking_sort: server refused \
             {{profile: 2}} (likely a managed/shared HFS_TEST_MONGODB_URL); skipping the \
             plan assertions"
        );
        return;
    }

    let mut cursor: Option<String> = None;
    loop {
        let page = backend
            .fetch_resources_page(&tenant, "Observation", cursor.as_deref(), 20)
            .await
            .unwrap();
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let _ = db.run_command(doc! { "profile": 0_i32 }).await;

    let profile: mongodb::Collection<Document> = db.collection("system.profile");
    let entries: Vec<Document> = profile
        .find(doc! {
            "ns": format!("{}.resources", db.name()),
            "op": "query",
            "command.find": "resources",
        })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();

    let mut id_queries = 0u32;
    let mut probes = 0u32;
    let mut round_queries = 0u32;
    let mut round_queries_with_or = 0u32;

    for entry in &entries {
        let command = match entry.get_document("command") {
            Ok(c) => c,
            Err(_) => continue,
        };
        let hint = command.get_str("hint").ok();
        let sort = command.get_document("sort").ok();
        let has_sort_stage = entry.get_bool("hasSortStage").unwrap_or(false);
        let keys_examined = entry
            .get_i64("keysExamined")
            .or_else(|_| entry.get_i32("keysExamined").map(i64::from))
            .unwrap_or(0);
        let docs_examined = entry
            .get_i64("docsExamined")
            .or_else(|_| entry.get_i32("docsExamined").map(i64::from))
            .unwrap_or(0);

        let mut inner = Document::new();
        for key in ["find", "filter", "sort", "limit", "projection", "hint"] {
            if let Some(v) = command.get(key) {
                inner.insert(key, v.clone());
            }
        }
        let explain = db
            .run_command(doc! { "explain": inner, "verbosity": "executionStats" })
            .await
            .unwrap();
        let winning_plan = match explain
            .get_document("queryPlanner")
            .and_then(|qp| qp.get_document("winningPlan"))
        {
            Ok(p) => p.clone(),
            Err(_) => continue,
        };
        let mut names = Vec::new();
        collect_index_names(&winning_plan, &mut names);
        let has_sort = contains_stage_named(&winning_plan, "SORT");

        let is_probe = sort == Some(&doc! { "last_updated": -1_i32, "id": -1_i32 });
        match hint {
            Some(h) if h == "idx_resources_identity" => {
                id_queries += 1;
                assert!(!has_sort_stage, "id query used a blocking sort");
                assert!(!has_sort, "id query plan has a SORT stage");
                assert!(
                    !names.is_empty() && names.iter().all(|n| n == "idx_resources_identity"),
                    "{names:?}"
                );
                assert!(keys_examined <= 20 + 3 + 50 + 1, "id query examined {keys_examined} keys");
            }
            Some(h) if h == "idx_resources_type_scan" && is_probe => {
                probes += 1;
                assert!(
                    !names.is_empty() && names.iter().all(|n| n == "idx_resources_type_scan"),
                    "{names:?}"
                );
                assert!(!has_sort);
                assert!(keys_examined <= 2, "probe examined {keys_examined} keys");
                assert_eq!(docs_examined, 0, "the probe must be covered");
            }
            Some(h) if h == "idx_resources_type_scan" => {
                round_queries += 1;
                // A round's continuation query plans as SORT_MERGE of two
                // IXSCANs of the same index (run 17's measured plan, S2 §4.3),
                // so `names` holds two equal entries here, not one —
                // `collect_index_names` does not de-duplicate.
                assert!(
                    !names.is_empty() && names.iter().all(|n| n == "idx_resources_type_scan"),
                    "{names:?}"
                );
                assert!(!has_sort, "round query plan has a SORT stage (SORT_MERGE is fine)");
                assert!(!has_sort_stage);
                assert!(
                    keys_examined <= 20 + 2,
                    "round query examined {keys_examined} keys (>= 40 would mean a from-floor rescan)"
                );
                let filter_has_or = command
                    .get_document("filter")
                    .map(|f| f.contains_key("$or"))
                    .unwrap_or(false);
                if filter_has_or {
                    round_queries_with_or += 1;
                }
            }
            _ => {}
        }
    }

    assert!(id_queries >= 13, "expected at least 13 id queries, got {id_queries}");
    assert!(probes >= 2, "expected at least 2 probes, got {probes}");
    assert!(round_queries >= 4, "expected at least 4 round queries, got {round_queries}");
    assert!(
        round_queries_with_or >= 3,
        "expected at least 3 round queries carrying $or, got {round_queries_with_or}"
    );
}
```

`collect_index_names` and `contains_stage_named` are the existing helpers already defined at file scope in `mongodb_tests.rs` (anchors `:4340`, `:4312` at HEAD c86d0f08b) and reachable here through `use super::*;` — do not redefine them. A find command's filter is always recorded at `command.filter` in `system.profile`, so `filter_has_or`'s extraction needs no runtime adjustment.

- [ ] **Step 2: Run the test — expect an immediate pass**

Docker or `HFS_TEST_MONGODB_URL` is required — see the Global Constraints note on Docker-gated steps.

Run:
```
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_id_walk 2>&1 \
  | tee "$LOGDIR/reindex_id_walk_task5.log"
grep -q "Skipping" "$LOGDIR/reindex_id_walk_task5.log" && { echo "NOT RUN"; exit 1; }
```
Expected: `test result: ok. 4 passed` (T1, T2, T9, T12) — this task adds no new production code, only T9, so treat any failure here as a bug surfaced in Task 4's walk, not an expected-red step.

- [ ] **Step 3: N/A — no implementation step**

This task adds only a test against Task 4's already-implemented walk. If T9 fails for a reason other than a missing harness helper, stop and report the failure with the exact assertion and captured values — do not patch the walk from inside this task; that is a Task 4 regression to re-open.

- [ ] **Step 4: Confirm all four tests are green**

Run the same command as Step 2. Expected: `test result: ok. 4 passed` (T1, T2, T9, T12).

- [ ] **Step 5: Commit**

```bash
git add crates/persistence/tests/mongodb/reindex_id_walk.rs
git commit -m "$(cat <<'EOF'
test(mongodb): id-order walk query-plan coverage (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: Self-healing under a racing update (T3, T4) — regression tests, green on arrival

**Files:**
- Modify: `crates/persistence/tests/mongodb/reindex_id_walk.rs` (add `MutatingSource`, `RecordingTarget`, T3, T4)

**Interfaces:**
- Produces (test-only types):
  ```rust
  struct MutatingSource {
      inner: std::sync::Arc<dyn helios_persistence::search::ReindexSource>,
      trigger_id: String,
      fired: std::sync::atomic::AtomicBool,
      mutation: Box<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>,
  }
  struct RecordingTarget {
      inner: std::sync::Arc<MongoBackend>,
      writes: std::sync::Mutex<Vec<(String, String)>>,
      pages_written: std::sync::atomic::AtomicUsize,
      page_written: tokio::sync::Notify,
      delay_after_page: std::time::Duration,
  }
  ```

- [ ] **Step 1: Write the failing tests**

Append to `crates/persistence/tests/mongodb/reindex_id_walk.rs`:

```rust
// ===========================================================================
// Test doubles for T3, T4, T7, T8
// ===========================================================================

/// Delegates to `inner`, and awaits `mutation()` once — after the inner call
/// returns a page containing `trigger_id`, before this call returns it — so
/// the mutation lands after the page's fetch and before its write,
/// deterministically, in either walk.
struct MutatingSource {
    inner: std::sync::Arc<dyn helios_persistence::search::ReindexSource>,
    trigger_id: String,
    fired: std::sync::atomic::AtomicBool,
    mutation: Box<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>,
}

#[async_trait::async_trait]
impl helios_persistence::search::ReindexSource for MutatingSource {
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        self.inner.list_resource_types(tenant).await
    }

    async fn count_resources(&self, tenant: &TenantContext, resource_type: &str) -> StorageResult<u64> {
        self.inner.count_resources(tenant, resource_type).await
    }

    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<helios_persistence::search::ResourcePage> {
        let page = self.inner.fetch_resources_page(tenant, resource_type, cursor, limit).await?;
        if !self.fired.load(std::sync::atomic::Ordering::SeqCst)
            && page.resources.iter().any(|r| r.id() == self.trigger_id)
        {
            self.fired.store(true, std::sync::atomic::Ordering::SeqCst);
            (self.mutation)().await;
        }
        Ok(page)
    }
}

/// Records each written resource's `(id, version_id)` and delegates every
/// write to `inner`; sleeps `delay_after_page` after each page (if nonzero)
/// and notifies `page_written`.
struct RecordingTarget {
    inner: std::sync::Arc<MongoBackend>,
    writes: std::sync::Mutex<Vec<(String, String)>>,
    pages_written: std::sync::atomic::AtomicUsize,
    page_written: tokio::sync::Notify,
    delay_after_page: std::time::Duration,
}

impl RecordingTarget {
    fn new(inner: std::sync::Arc<MongoBackend>) -> Self {
        Self {
            inner,
            writes: std::sync::Mutex::new(Vec::new()),
            pages_written: std::sync::atomic::AtomicUsize::new(0),
            page_written: tokio::sync::Notify::new(),
            delay_after_page: std::time::Duration::ZERO,
        }
    }

    fn with_delay(mut self, delay: std::time::Duration) -> Self {
        self.delay_after_page = delay;
        self
    }
}

#[async_trait::async_trait]
impl helios_persistence::search::ReindexTarget for RecordingTarget {
    async fn delete_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<u64> {
        self.inner.delete_search_entries(tenant, resource_type, resource_id).await
    }

    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource: &helios_persistence::types::StoredResource,
    ) -> StorageResult<usize> {
        self.inner.write_search_entries(tenant, resource).await
    }

    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
        self.inner.clear_search_index(tenant).await
    }

    async fn write_search_entries_page(
        &self,
        tenant: &TenantContext,
        resources: &[helios_persistence::types::StoredResource],
    ) -> Vec<StorageResult<usize>> {
        for r in resources {
            self.writes
                .lock()
                .unwrap()
                .push((r.id().to_string(), r.version_id().to_string()));
        }
        let results = self.inner.write_search_entries_page(tenant, resources).await;
        if !self.delay_after_page.is_zero() {
            tokio::time::sleep(self.delay_after_page).await;
        }
        self.pages_written.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.page_written.notify_one();
        results
    }
}

// ===========================================================================
// T3: an update between fetch and write of an id-phase page heals
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_heals_an_update_between_fetch_and_write() {
    use std::sync::Arc;

    async fn run_for(
        backend: Arc<MongoBackend>,
        source: Arc<dyn helios_persistence::search::ReindexSource>,
        tenant: &TenantContext,
        regs: Arc<helios_persistence::search::TenantSearchRegistries>,
    ) -> (Arc<RecordingTarget>, helios_persistence::search::ReindexProgress) {
        let target = Arc::new(RecordingTarget::new(backend));
        let op = ReindexOperation::with_parts(source, vec![target.clone()], regs);
        let job = op
            .start(tenant.clone(), ReindexRequest::for_types(["Observation"]).with_batch_size(10), None)
            .await
            .unwrap();
        let progress = wait_for_terminal(&op, &job).await;
        (target, progress)
    }

    let Some(backend) = create_backend("reindex_id_walk_heal_id").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant_legacy = create_tenant("walk-upd-legacy");
    let tenant_new = create_tenant("walk-upd-new");
    for tenant in [&tenant_legacy, &tenant_new] {
        let fixture = seed_walk_fixture(&backend, tenant, 60, "extra").await;
        backdate_fixture(&backend, tenant, &fixture).await;
    }

    let mutation_for = |backend: Arc<MongoBackend>, tenant: TenantContext| {
        Box::new(move || {
            let backend = backend.clone();
            let tenant = tenant.clone();
            Box::pin(async move {
                let current = backend.read(&tenant, "Observation", "obs-017").await.unwrap().unwrap();
                let mut content = current.content().clone();
                content["valueQuantity"]["value"] = json!(9999);
                content["identifier"][0]["value"] = json!("o-17-updated");
                backend.update(&tenant, &current, content).await.unwrap();
            }) as futures::future::BoxFuture<'static, ()>
        }) as Box<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>
    };

    let regs = backend.tenant_registries().clone();

    let legacy_source: Arc<dyn helios_persistence::search::ReindexSource> = Arc::new(MutatingSource {
        inner: Arc::new(LegacyWalkSource { backend: backend.clone() }),
        trigger_id: "obs-017".to_string(),
        fired: std::sync::atomic::AtomicBool::new(false),
        mutation: mutation_for(backend.clone(), tenant_legacy.clone()),
    });
    run_for(backend.clone(), legacy_source, &tenant_legacy, regs.clone()).await;

    let new_source: Arc<dyn helios_persistence::search::ReindexSource> = Arc::new(MutatingSource {
        inner: backend.clone(),
        trigger_id: "obs-017".to_string(),
        fired: std::sync::atomic::AtomicBool::new(false),
        mutation: mutation_for(backend.clone(), tenant_new.clone()),
    });
    let (target, progress) = run_for(backend.clone(), new_source, &tenant_new, regs.clone()).await;
    assert_eq!(progress.status, helios_persistence::search::ReindexStatus::Completed);
    assert!(progress.errors.is_empty());

    let writes = target.writes.lock().unwrap().clone();
    let v1_idx = writes.iter().position(|(id, v)| id == "obs-017" && v == "1");
    let v2_idx = writes.iter().position(|(id, v)| id == "obs-017" && v == "2");
    assert!(v1_idx.is_some() && v2_idx.is_some() && v1_idx < v2_idx, "{writes:?}");

    let db = backend.get_database().await.unwrap();
    let s_legacy = snapshot(&db, "walk-upd-legacy", true).await;
    let s_new = snapshot(&db, "walk-upd-new", true).await;
    assert_eq!(s_new, s_legacy);

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(Some("urn:walk"), "o-17-updated")],
        chain: vec![],
        components: vec![],
    });
    let found = backend.search(&tenant_new, &query).await.unwrap();
    assert_eq!(found.resources.items.len(), 1);
    let stale_query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(Some("urn:walk"), "o-17")],
        chain: vec![],
        components: vec![],
    });
    assert!(backend.search(&tenant_new, &stale_query).await.unwrap().resources.items.is_empty());

    let s_final = snapshot(&db, "walk-upd-new", false).await;
    let resource = backend.read(&tenant_new, "Observation", "obs-017").await.unwrap().unwrap();
    backend.write_search_entries(&tenant_new, &resource).await.unwrap();
    assert_eq!(snapshot(&db, "walk-upd-new", false).await, s_final);
}

// ===========================================================================
// T4: an update racing a catch-up page heals
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_heals_an_update_racing_a_catch_up_page() {
    use std::sync::Arc;

    let Some(backend) = create_backend("reindex_id_walk_heal_round").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("walk-rnd");
    let fixture = seed_walk_fixture(&backend, &tenant, 60, "extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;

    for i in [10, 20, 30] {
        let id = format!("obs-{i:03}");
        let current = backend.read(&tenant, "Observation", &id).await.unwrap().unwrap();
        let mut content = current.content().clone();
        content["identifier"][0]["value"] = json!(format!("o-{i}-v2"));
        backend.update(&tenant, &current, content).await.unwrap();
    }

    let trigger_tenant = tenant.clone();
    let backend_for_mutation = backend.clone();
    let mutation: Box<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync> =
        Box::new(move || {
            let backend = backend_for_mutation.clone();
            let tenant = trigger_tenant.clone();
            Box::pin(async move {
                let current = backend.read(&tenant, "Observation", "obs-020").await.unwrap().unwrap();
                let mut content = current.content().clone();
                content["identifier"][0]["value"] = json!("o-20-v3");
                backend.update(&tenant, &current, content).await.unwrap();
            }) as futures::future::BoxFuture<'static, ()>
        });

    let source: Arc<dyn helios_persistence::search::ReindexSource> = Arc::new(MutatingSource {
        inner: backend.clone(),
        trigger_id: "obs-020".to_string(),
        fired: std::sync::atomic::AtomicBool::new(false),
        mutation,
    });
    let target = Arc::new(RecordingTarget::new(backend.clone()));
    let op = ReindexOperation::with_parts(source, vec![target.clone()], backend.tenant_registries().clone());
    let job = op
        .start(tenant.clone(), ReindexRequest::for_types(["Observation"]).with_batch_size(10), None)
        .await
        .unwrap();
    let progress = wait_for_terminal(&op, &job).await;
    assert_eq!(progress.status, helios_persistence::search::ReindexStatus::Completed);
    assert!(progress.errors.is_empty());

    let writes = target.writes.lock().unwrap().clone();
    let v2_idx = writes.iter().position(|(id, v)| id == "obs-020" && v == "2");
    let v3_idx = writes.iter().position(|(id, v)| id == "obs-020" && v == "3");
    assert!(
        v2_idx.is_some() && v3_idx.is_some() && v2_idx < v3_idx,
        "under a design that ended the round on the short page, version 3 would never be written: {writes:?}"
    );

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(Some("urn:walk"), "o-20-v3")],
        chain: vec![],
        components: vec![],
    });
    assert_eq!(backend.search(&tenant, &query).await.unwrap().resources.items.len(), 1);
    let stale_query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(Some("urn:walk"), "o-20-v2")],
        chain: vec![],
        components: vec![],
    });
    assert!(backend.search(&tenant, &stale_query).await.unwrap().resources.items.is_empty());

    let db = backend.get_database().await.unwrap();
    let s_final = snapshot(&db, "walk-rnd", false).await;
    let resource = backend.read(&tenant, "Observation", "obs-020").await.unwrap().unwrap();
    backend.write_search_entries(&tenant, &resource).await.unwrap();
    assert_eq!(snapshot(&db, "walk-rnd", false).await, s_final);
}
```

`SearchParamType`, `SearchParameter`, `SearchQuery`, `SearchValue`, `json!`, `doc!`, `Bson`, `Document`, `Arc`, `TenantContext`, `FhirVersion`, `MongoBackend`, `MongoBackendConfig`, `BackendError`, `StorageError`, `SearchError`, `ResourceStorage` and `SearchProvider` all already come through the root's `use` lines (`mongodb_tests.rs:29-54`) via `use super::*;` at the top of `reindex_id_walk.rs` — no additional imports are needed here.

- [ ] **Step 2: Run the tests — expect an immediate pass**

Docker or `HFS_TEST_MONGODB_URL` is required — see the Global Constraints note on Docker-gated steps.

Run:
```
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_id_walk 2>&1 \
  | tee "$LOGDIR/reindex_id_walk_task6.log"
grep -q "Skipping" "$LOGDIR/reindex_id_walk_task6.log" && { echo "NOT RUN"; exit 1; }
```
Expected: `test result: ok. 6 passed` (T1-T4, T9, T12) — compiles against Task 4's already-correct walk, so both T3 and T4 should pass immediately (this task, like Task 5, adds no new production code). If either fails, capture the exact assertion and the `writes` vector's contents before deciding whether it is a real Task 4 defect (most likely: the id phase or a round did not re-read a resource updated mid-page) or a test-harness mistake (most likely: the mutation fired on the wrong page, or `content()` vs `content_with_meta()` confusion causing a `_lastUpdated` mismatch — S2 §9.2 warns fixture JSON must never carry `meta`).

- [ ] **Step 3: N/A — no implementation step**

Per Task 5's note: this task only adds tests against Task 4's walk. Do not patch the walk here; re-open Task 4 if it is wrong.

- [ ] **Step 4: Confirm all six tests are green**

Run the same command as Step 2. Expected: `test result: ok. 6 passed` (T1-T4, T9, T12).

- [ ] **Step 5: Commit**

```bash
git add crates/persistence/tests/mongodb/reindex_id_walk.rs
git commit -m "$(cat <<'EOF'
test(mongodb): self-healing coverage for a racing update in either phase (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: Deferred creates, deletes, import tails and future stamps (T5, T6, T10, T11) — regression tests, green on arrival

**Files:**
- Modify: `crates/persistence/tests/mongodb/reindex_id_walk.rs` (add a local `seed_submission` copy, T5, T6, T10, T11)

**Interfaces:**
- Produces: `async fn seed_submission(backend: &MongoBackend, tenant: &TenantContext) -> (helios_persistence::core::SubmissionId, String)` — a local copy of `mongodb_tests.rs`'s private `mod bulk_submit::seed` (anchor `:10054-10067`), because that function is private to its sibling module and unreachable through `use super::*;` from this file (S2 §9.2).
- Consumes: `MutatingSource`, `RecordingTarget` (Task 6, reused by T5 and T6 below); `wait_for_terminal`, `snapshot`, `capture_walk_logs`, `walk_log_lines`, `LegacyWalkSource`, `seed_walk_fixture`, `backdate_fixture` (Task 4); `search_index_entry_count` (existing helper, `mongodb_tests.rs:1320`, reachable via `use super::*;`).

- [ ] **Step 1: Write the failing tests**

Append to `crates/persistence/tests/mongodb/reindex_id_walk.rs`:

```rust
// ===========================================================================
// Harness: a local copy of `bulk_submit::seed` (private to its own module)
// ===========================================================================

/// Creates a submission with one fetchable manifest — the shape the REST
/// kickoff handler produces. A copy of `mongodb_tests.rs`'s private
/// `bulk_submit::seed`, since that module's items are not reachable from a
/// sibling `#[path]`-included file through `use super::*;`.
async fn seed_submission(
    backend: &MongoBackend,
    tenant: &TenantContext,
) -> (helios_persistence::core::SubmissionId, String) {
    use helios_persistence::core::{BulkSubmitProvider, SubmissionId};
    let id = SubmissionId::generate("data-provider");
    backend.create_submission(tenant, &id, None).await.unwrap();
    let manifest = backend
        .add_manifest(tenant, &id, Some("https://provider.example/manifest.json"), None)
        .await
        .unwrap();
    (id, manifest.manifest_id)
}

// ===========================================================================
// T5: a deferred create below the cursor is indexed
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_indexes_a_deferred_create_below_the_cursor() {
    use helios_persistence::core::{BulkProcessingOptions, BulkSubmitProvider, NdjsonEntry};
    use std::sync::Arc;

    let Some(backend) = create_backend("reindex_id_walk_deferred_create").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant_legacy = create_tenant("walk-crt-legacy");
    let tenant_new = create_tenant("walk-crt-new");

    let fixture_legacy = seed_walk_fixture(&backend, &tenant_legacy, 60, "extra").await;
    backdate_fixture(&backend, &tenant_legacy, &fixture_legacy).await;
    let (sid_legacy, mid_legacy) = seed_submission(&backend, &tenant_legacy).await;

    let fixture_new = seed_walk_fixture(&backend, &tenant_new, 60, "extra").await;
    backdate_fixture(&backend, &tenant_new, &fixture_new).await;
    let (sid_new, mid_new) = seed_submission(&backend, &tenant_new).await;

    // The create happens mid-walk, triggered by obs-030 (S2 §9.2 T5), so it
    // lands behind the cursor as a real non-inline write — not before the
    // walk starts, where nothing would exercise the id-order phase's healing.
    let mutation_for = |backend: Arc<MongoBackend>,
                         tenant: TenantContext,
                         sid: helios_persistence::core::SubmissionId,
                         mid: String| {
        Box::new(move || {
            let backend = backend.clone();
            let tenant = tenant.clone();
            let sid = sid.clone();
            let mid = mid.clone();
            Box::pin(async move {
                backend
                    .process_entries(
                        &tenant,
                        &sid,
                        &mid,
                        vec![NdjsonEntry::new(
                            1,
                            "Observation",
                            json!({
                                "resourceType": "Observation",
                                "id": "--below-cursor",
                                "status": "final",
                                "code": { "coding": [{ "system": "http://loinc.org", "code": "8867-4" }] },
                                "identifier": [{ "system": "urn:walk", "value": "o-below" }],
                            }),
                        )],
                        &BulkProcessingOptions::new().with_defer_indexing(true),
                    )
                    .await
                    .unwrap();
            }) as futures::future::BoxFuture<'static, ()>
        }) as Box<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>
    };

    let regs = backend.tenant_registries().clone();

    let legacy_source: Arc<dyn helios_persistence::search::ReindexSource> = Arc::new(MutatingSource {
        inner: Arc::new(LegacyWalkSource { backend: backend.clone() }),
        trigger_id: "obs-030".to_string(),
        fired: std::sync::atomic::AtomicBool::new(false),
        mutation: mutation_for(backend.clone(), tenant_legacy.clone(), sid_legacy, mid_legacy),
    });
    let legacy_op = ReindexOperation::with_parts(legacy_source, vec![backend.clone()], regs.clone());
    let job = legacy_op
        .start(tenant_legacy.clone(), ReindexRequest::for_types(["Observation"]).with_batch_size(10), None)
        .await
        .unwrap();
    wait_for_terminal(&legacy_op, &job).await;

    let new_source: Arc<dyn helios_persistence::search::ReindexSource> = Arc::new(MutatingSource {
        inner: backend.clone(),
        trigger_id: "obs-030".to_string(),
        fired: std::sync::atomic::AtomicBool::new(false),
        mutation: mutation_for(backend.clone(), tenant_new.clone(), sid_new, mid_new),
    });
    let new_op = ReindexOperation::with_parts(new_source, vec![backend.clone()], regs);
    let job = new_op
        .start(tenant_new.clone(), ReindexRequest::for_types(["Observation"]).with_batch_size(10), None)
        .await
        .unwrap();
    wait_for_terminal(&new_op, &job).await;

    // Only after both walks have run does the deferred create's absence from
    // the search index get resolved — it writes no rows itself (deferred
    // indexing), so asserting this any earlier would always pass vacuously.
    assert!(search_index_entry_count(&backend, &tenant_legacy, "Observation", "--below-cursor").await > 0);
    assert!(search_index_entry_count(&backend, &tenant_new, "Observation", "--below-cursor").await > 0);

    let db = backend.get_database().await.unwrap();
    assert_eq!(
        snapshot(&db, "walk-crt-legacy", true).await,
        snapshot(&db, "walk-crt-new", true).await
    );

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(Some("urn:walk"), "o-below")],
        chain: vec![],
        components: vec![],
    });
    assert_eq!(backend.search(&tenant_new, &query).await.unwrap().resources.items.len(), 1);
}

// ===========================================================================
// T6: a delete mid-walk behaves as on HEAD (orphan rows unchanged)
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_delete_mid_walk_behaves_as_the_legacy_walk() {
    use std::sync::Arc;

    let Some(backend) = create_backend("reindex_id_walk_delete_mid").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant_legacy = create_tenant("walk-del-legacy");
    let tenant_new = create_tenant("walk-del-new");
    for tenant in [&tenant_legacy, &tenant_new] {
        let fixture = seed_walk_fixture(&backend, tenant, 60, "extra").await;
        backdate_fixture(&backend, tenant, &fixture).await;
    }

    let mutation_for = |backend: Arc<MongoBackend>, tenant: TenantContext| {
        Box::new(move || {
            let backend = backend.clone();
            let tenant = tenant.clone();
            Box::pin(async move {
                backend.delete(&tenant, "Observation", "obs-042").await.unwrap();
            }) as futures::future::BoxFuture<'static, ()>
        }) as Box<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>
    };

    let regs = backend.tenant_registries().clone();
    let legacy_source: Arc<dyn helios_persistence::search::ReindexSource> = Arc::new(MutatingSource {
        inner: Arc::new(LegacyWalkSource { backend: backend.clone() }),
        trigger_id: "obs-042".to_string(),
        fired: std::sync::atomic::AtomicBool::new(false),
        mutation: mutation_for(backend.clone(), tenant_legacy.clone()),
    });
    let legacy_op = ReindexOperation::with_parts(legacy_source, vec![backend.clone()], regs.clone());
    let job = legacy_op
        .start(tenant_legacy.clone(), ReindexRequest::for_types(["Observation"]).with_batch_size(10), None)
        .await
        .unwrap();
    let legacy_progress = wait_for_terminal(&legacy_op, &job).await;

    let new_source: Arc<dyn helios_persistence::search::ReindexSource> = Arc::new(MutatingSource {
        inner: backend.clone(),
        trigger_id: "obs-042".to_string(),
        fired: std::sync::atomic::AtomicBool::new(false),
        mutation: mutation_for(backend.clone(), tenant_new.clone()),
    });
    let new_op = ReindexOperation::with_parts(new_source, vec![backend.clone()], regs);
    let job = new_op
        .start(tenant_new.clone(), ReindexRequest::for_types(["Observation"]).with_batch_size(10), None)
        .await
        .unwrap();
    let new_progress = wait_for_terminal(&new_op, &job).await;

    for p in [&legacy_progress, &new_progress] {
        assert_eq!(p.status, helios_persistence::search::ReindexStatus::Completed);
        assert!(p.errors.is_empty());
    }

    let db = backend.get_database().await.unwrap();
    assert_eq!(
        snapshot(&db, "walk-del-legacy", true).await,
        snapshot(&db, "walk-del-new", true).await,
        "orphan rows from the mid-walk delete must match HEAD exactly"
    );

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(Some("urn:walk"), "o-42")],
        chain: vec![],
        components: vec![],
    });
    assert!(backend.search(&tenant_new, &query).await.unwrap().resources.items.is_empty());
}

// ===========================================================================
// T10: an import tail is written exactly once
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_writes_an_import_tail_once() {
    use std::sync::Arc;

    let Some(backend) = create_backend("reindex_id_walk_tail").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    capture_walk_logs();
    let tenant = create_tenant("walk-tail");
    let fixture = seed_walk_fixture(&backend, &tenant, 300, "extra").await;
    let db = backend.get_database().await.unwrap();
    let s_crud = snapshot(&db, "walk-tail", false).await;
    backdate_fixture(&backend, &tenant, &fixture).await;

    let resources = db.collection::<Document>("resources");
    let tail_ids: Vec<String> = (250..300).map(|i| format!("obs-{i:03}")).collect();
    resources
        .update_many(
            doc! { "tenant_id": "walk-tail", "resource_type": "Observation", "id": { "$in": &tail_ids } },
            doc! {
                "$set": {
                    "last_updated": BsonDateTime::from_millis(
                        (chrono::Utc::now() - chrono::Duration::seconds(10)).timestamp_millis(),
                    ),
                },
            },
        )
        .await
        .unwrap();

    let target = Arc::new(RecordingTarget::new(backend.clone()));
    let op = ReindexOperation::with_parts(backend.clone(), vec![target.clone()], backend.tenant_registries().clone());
    let job = op
        .start(tenant.clone(), ReindexRequest::for_types(["Observation"]).with_batch_size(40), None)
        .await
        .unwrap();
    let progress = wait_for_terminal(&op, &job).await;
    assert_eq!(progress.status, helios_persistence::search::ReindexStatus::Completed);
    assert!(progress.errors.is_empty());
    assert_eq!(progress.processed_resources, progress.total_resources);
    assert_eq!(progress.processed_resources, 297);

    let writes = target.writes.lock().unwrap().clone();
    let written_ids: Vec<&str> = writes.iter().map(|(id, _)| id.as_str()).collect();
    let non_tail_written: std::collections::HashSet<&str> = written_ids
        .iter()
        .filter(|id| !tail_ids.iter().any(|t| t == *id))
        .cloned()
        .collect();
    let last_non_tail_pos = written_ids
        .iter()
        .enumerate()
        .filter(|(_, id)| non_tail_written.contains(*id))
        .map(|(i, _)| i)
        .max()
        .unwrap_or(0);
    let first_tail_pos = written_ids
        .iter()
        .enumerate()
        .find(|(_, id)| tail_ids.iter().any(|t| t == *id))
        .map(|(i, _)| i);
    if let Some(first_tail_pos) = first_tail_pos {
        assert!(first_tail_pos > last_non_tail_pos, "{written_ids:?}");
    }
    // Every live id is recorded exactly once (no duplicates from a re-visit).
    let mut sorted = written_ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), written_ids.len(), "{written_ids:?}");

    assert_eq!(snapshot(&db, "walk-tail", false).await, s_crud);

    let round1_finished = walk_log_lines(&[
        "tenant=walk-tail",
        "resource_type=Observation",
        "mongodb reindex catch-up round finished",
        "round=1",
    ]);
    assert!(round1_finished.iter().any(|l| l.contains("walked=50")), "{round1_finished:?}");
}

// ===========================================================================
// T11: a future-stamped resource is indexed
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_indexes_a_future_stamped_resource() {
    use std::sync::Arc;

    let Some(backend) = create_backend("reindex_id_walk_future_stamp").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    capture_walk_logs();
    let tenant = create_tenant("walk-future");
    let fixture = seed_walk_fixture(&backend, &tenant, 60, "extra").await;
    let db = backend.get_database().await.unwrap();
    let s_crud = snapshot(&db, "walk-future", false).await;
    backdate_fixture(&backend, &tenant, &fixture).await;

    let resources = db.collection::<Document>("resources");
    resources
        .update_many(
            doc! { "tenant_id": "walk-future", "resource_type": "Observation", "id": "obs-033" },
            doc! {
                "$set": {
                    "last_updated": BsonDateTime::from_millis(
                        (chrono::Utc::now() + chrono::Duration::days(1)).timestamp_millis(),
                    ),
                },
            },
        )
        .await
        .unwrap();

    let target = Arc::new(RecordingTarget::new(backend.clone()));
    let op = ReindexOperation::with_parts(backend.clone(), vec![target.clone()], backend.tenant_registries().clone());
    let job = op
        .start(tenant.clone(), ReindexRequest::for_types(["Observation"]).with_batch_size(10), None)
        .await
        .unwrap();
    let progress = wait_for_terminal(&op, &job).await;
    assert_eq!(progress.status, helios_persistence::search::ReindexStatus::Completed);
    assert!(progress.errors.is_empty());
    assert_eq!(progress.processed_resources, progress.total_resources);
    assert_eq!(progress.processed_resources, 59);

    let writes = target.writes.lock().unwrap().clone();
    let count_033 = writes.iter().filter(|(id, _)| id == "obs-033").count();
    assert_eq!(count_033, 1, "{writes:?}");

    // Every other live Observation is also recorded exactly once — the future
    // stamp must not cause it, or anything else, to be walked twice.
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for (id, _) in &writes {
        *counts.entry(id.as_str()).or_insert(0) += 1;
    }
    let live_ids: std::collections::BTreeSet<&str> =
        fixture.live.get("Observation").unwrap().iter().map(String::as_str).collect();
    let written_ids: std::collections::BTreeSet<&str> = counts.keys().copied().collect();
    assert_eq!(written_ids, live_ids, "written ids must equal the 59 live ids");
    assert!(counts.values().all(|&n| n == 1), "{counts:?}");

    assert_eq!(snapshot(&db, "walk-future", false).await, s_crud);

    let future_lines = walk_log_lines(&[
        "tenant=walk-future",
        "resource_type=Observation",
        "mongodb reindex found live resources stamped in the future",
    ]);
    assert_eq!(future_lines.len(), 1, "{future_lines:?}");
}
```

- [ ] **Step 2: Run the tests — expect an immediate pass**

Docker or `HFS_TEST_MONGODB_URL` is required — see the Global Constraints note on Docker-gated steps.

Run:
```
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_id_walk 2>&1 \
  | tee "$LOGDIR/reindex_id_walk_task7.log"
grep -q "Skipping" "$LOGDIR/reindex_id_walk_task7.log" && { echo "NOT RUN"; exit 1; }
```
Expected: `test result: ok. 10 passed` (T1-T6, T9-T12) — compiles against Task 4's walk; all four new tests pass immediately (no new production code in this task). As in Tasks 5-6, treat a genuine failure as a Task 4 regression to investigate and re-open, not something to patch here.

- [ ] **Step 3: N/A — no implementation step**

- [ ] **Step 4: Confirm all ten tests are green**

Run the same command as Step 2. Expected: `test result: ok. 10 passed` (T1-T6, T9-T12).

- [ ] **Step 5: Commit**

```bash
git add crates/persistence/tests/mongodb/reindex_id_walk.rs
git commit -m "$(cat <<'EOF'
test(mongodb): deferred creates, mid-walk deletes, import tails and future stamps (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: Cancellation, rerun and termination under continuous writes (T7, T8) — regression tests, green on arrival

**Files:**
- Modify: `crates/persistence/tests/mongodb/reindex_id_walk.rs` (add `PausingSource`, `create_backend_with_catch_up_margin`, T7, T8)

**Interfaces:**
- Produces:
  ```rust
  struct PausingSource {
      inner: std::sync::Arc<MongoBackend>,
      pause_on_call: usize,
      calls: std::sync::atomic::AtomicUsize,
      reached: tokio::sync::Notify,
      resume: tokio::sync::Semaphore,
  }
  async fn create_backend_with_catch_up_margin(test_name: &str, margin_ms: u64) -> Option<MongoBackend>;
  ```
- Consumes: `RecordingTarget` (Task 6); `wait_for_terminal`, `snapshot`, `capture_walk_logs`, `walk_log_lines`, `seed_walk_fixture`, `backdate_fixture` (Task 4); `build_test_database_name`, `repo_data_dir`, `build_backend`, `shared_mongo::connection_string` (existing helpers, reachable via `use super::*;`).

- [ ] **Step 1: Write the failing tests**

Append to `crates/persistence/tests/mongodb/reindex_id_walk.rs`:

```rust
// ===========================================================================
// Harness: a backend with a shortened catch-up margin, and a pausing source
// ===========================================================================

/// A copy of `create_backend_with_search_offloaded` (anchor `:1233-1247` at
/// HEAD c86d0f08b) that also sets `reindex_catch_up_margin_ms`, so a
/// termination test does not have to wait out the real 120 s margin.
async fn create_backend_with_catch_up_margin(test_name: &str, margin_ms: u64) -> Option<MongoBackend> {
    let connection_string = shared_mongo::connection_string().await?;
    let config = MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test_name),
        data_dir: Some(repo_data_dir()),
        reindex_catch_up_margin_ms: margin_ms,
        ..Default::default()
    };
    build_backend(config).await
}

/// Pauses at its `pause_on_call`-th call, after fetching the inner page but
/// before returning it, so a test can observe an in-flight page and then
/// cancel while it is still in flight.
struct PausingSource {
    inner: std::sync::Arc<MongoBackend>,
    pause_on_call: usize,
    calls: std::sync::atomic::AtomicUsize,
    reached: tokio::sync::Notify,
    resume: tokio::sync::Semaphore,
}

impl PausingSource {
    fn new(inner: std::sync::Arc<MongoBackend>, pause_on_call: usize) -> Self {
        Self {
            inner,
            pause_on_call,
            calls: std::sync::atomic::AtomicUsize::new(0),
            reached: tokio::sync::Notify::new(),
            resume: tokio::sync::Semaphore::new(0),
        }
    }
}

#[async_trait::async_trait]
impl helios_persistence::search::ReindexSource for PausingSource {
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        self.inner.list_resource_types(tenant).await
    }

    async fn count_resources(&self, tenant: &TenantContext, resource_type: &str) -> StorageResult<u64> {
        self.inner.count_resources(tenant, resource_type).await
    }

    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<helios_persistence::search::ResourcePage> {
        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let page = self.inner.fetch_resources_page(tenant, resource_type, cursor, limit).await?;
        if call == self.pause_on_call {
            self.reached.notify_one();
            self.resume.acquire().await.unwrap().forget();
        }
        Ok(page)
    }
}

// ===========================================================================
// T7: cancel while a page is in flight, then rerun without duplicates
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_cancel_then_rerun_leaves_no_duplicates() {
    use std::sync::Arc;

    let Some(backend) = create_backend("reindex_id_walk_cancel").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("walk-cancel");
    let fixture = seed_walk_fixture(&backend, &tenant, 300, "extra").await;
    let db = backend.get_database().await.unwrap();
    let s_crud = snapshot(&db, "walk-cancel", false).await;
    backdate_fixture(&backend, &tenant, &fixture).await;

    let pausing = Arc::new(PausingSource::new(backend.clone(), 3));
    let recording = Arc::new(RecordingTarget::new(backend.clone()));
    let op = Arc::new(ReindexOperation::with_parts(
        pausing.clone(),
        vec![recording.clone()],
        backend.tenant_registries().clone(),
    ));
    let job = op
        .start(tenant.clone(), ReindexRequest::for_types(["Observation"]).with_batch_size(20), None)
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(30), pausing.reached.notified())
        .await
        .expect("PausingSource never reached its pause point");
    op.cancel(&job).await.unwrap();
    pausing.resume.add_permits(1);

    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if recording.pages_written.load(std::sync::atomic::Ordering::SeqCst) >= 3 {
                return;
            }
            recording.page_written.notified().await;
        }
    })
    .await
    .expect("page 3 was never written");

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(pausing.calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(recording.pages_written.load(std::sync::atomic::Ordering::SeqCst), 3);

    let progress = op.get_progress(&job).await.unwrap();
    assert_eq!(progress.status, helios_persistence::search::ReindexStatus::Cancelled);

    let rerun_op = ReindexOperation::new(backend.clone(), backend.tenant_registries().clone());
    let rerun_job = rerun_op
        .start(tenant.clone(), ReindexRequest::for_types(["Observation"]).with_batch_size(20), None)
        .await
        .unwrap();
    wait_for_terminal(&rerun_op, &rerun_job).await;
    assert_eq!(snapshot(&db, "walk-cancel", false).await, s_crud);
}

// ===========================================================================
// T8: termination and the round cap under continuous writes
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_terminates_under_continuous_writes() {
    use std::sync::Arc;

    let Some(backend) = create_backend_with_catch_up_margin("reindex_id_walk_churn", 2_000).await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    capture_walk_logs();
    let tenant = create_tenant("walk-churn");
    let fixture = seed_walk_fixture(&backend, &tenant, 60, "only-in-churn").await;
    backdate_fixture(&backend, &tenant, &fixture).await;

    let live_ids: Vec<String> = fixture.live.get("Observation").unwrap().iter().cloned().collect();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let updates = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let writer = {
        let backend = backend.clone();
        let tenant = tenant.clone();
        let stop = stop.clone();
        let updates = updates.clone();
        tokio::spawn(async move {
            let mut i = 0usize;
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                let id = &live_ids[i % live_ids.len()];
                if let Ok(Some(current)) = backend.read(&tenant, "Observation", id).await {
                    let mut content = current.content().clone();
                    let bumped = content["valueQuantity"]["value"].as_i64().unwrap_or(0) + 1;
                    content["valueQuantity"]["value"] = json!(bumped);
                    if backend.update(&tenant, &current, content).await.is_ok() {
                        updates.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                i += 1;
            }
        })
    };

    while updates.load(std::sync::atomic::Ordering::SeqCst) < 20 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let u0 = updates.load(std::sync::atomic::Ordering::SeqCst);

    let target = Arc::new(RecordingTarget::new(backend.clone()).with_delay(std::time::Duration::from_millis(250)));
    let op = ReindexOperation::with_parts(backend.clone(), vec![target], backend.tenant_registries().clone());
    let job = op
        .start(tenant.clone(), ReindexRequest::for_types(["Observation"]).with_batch_size(5), None)
        .await
        .unwrap();
    let progress = tokio::time::timeout(std::time::Duration::from_secs(45), async {
        loop {
            let p = op.get_progress(&job).await.unwrap();
            if p.status.is_finished() {
                return p;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("reindex did not terminate within 45s");
    let u1 = updates.load(std::sync::atomic::Ordering::SeqCst);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    writer.await.unwrap();

    assert!(u1 - u0 >= 50, "writer made only {} updates during the walk; the test's timing assumptions do not hold", u1 - u0);
    assert_eq!(progress.status, helios_persistence::search::ReindexStatus::Completed);
    assert!(progress.errors.is_empty());

    let round3_started = walk_log_lines(&[
        "tenant=walk-churn",
        "resource_type=Observation",
        "mongodb reindex catch-up round started",
        "round=3",
    ]);
    assert!(!round3_started.is_empty(), "expected round 3 to start under sustained writes");
    let capped = walk_log_lines(&[
        "tenant=walk-churn",
        "resource_type=Observation",
        "mongodb reindex catch-up stopped at its round limit",
    ]);
    assert_eq!(capped.len(), 1, "{capped:?}");
    assert!(progress.processed_resources >= progress.total_resources);
    // Final-row correctness is not asserted: residual (a) is expected under
    // continuous writes that outpace the walk.
}
```

- [ ] **Step 2: Run the tests — expect an immediate pass**

Docker or `HFS_TEST_MONGODB_URL` is required — see the Global Constraints note on Docker-gated steps.

Run:
```
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_id_walk -- --test-threads=1 2>&1 \
  | tee "$LOGDIR/reindex_id_walk_task8.log"
grep -q "Skipping" "$LOGDIR/reindex_id_walk_task8.log" && { echo "NOT RUN"; exit 1; }
```
Expected: `test result: ok. 12 passed` (T1-T12) — compiles against Task 4's already-correct walk and `ReindexOperation::cancel`/`get_progress` (both pre-existing, unmodified by this plan), so T7 and T8 should pass immediately. `--test-threads=1` is recommended for T8 specifically, since it spawns a background writer task and asserts on wall-clock-relative counts; if the whole suite is normally run with default parallelism, confirm T8 alone is not flaky under load before relying on it in CI (S2 R10). Treat a genuine assertion failure as a Task 4 regression (most likely in `RoundStartDecision`/the round loop) to re-open, not something to patch here.

- [ ] **Step 3: N/A — no implementation step**

- [ ] **Step 4: Confirm all twelve tests are green**

Run:
```
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_id_walk
cargo test -p helios-persistence --features mongodb --lib reindex_walk_tests
```
Expected: `test result: ok. 12 passed` for `reindex_id_walk` (T1-T12), and every `reindex_walk_tests` unit test still passes.

- [ ] **Step 5: Commit**

```bash
git add crates/persistence/tests/mongodb/reindex_id_walk.rs
git commit -m "$(cat <<'EOF'
test(mongodb): cancel/rerun and round-cap termination under continuous writes (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: Whole-PR verification and the manual MongoDB 7.0 gate

**Files:** none new — verification only.

- [ ] **Step 1: Existing tests that must stay green**

Run each by name (all require Docker or `HFS_TEST_MONGODB_URL` — see the Global Constraints note on Docker-gated steps; do not accept a `Skipping` line as a pass for this step). First, per the Global Constraints "No cargo while a bench arm is running" rule:
```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
```
```
cargo test -p helios-persistence --features mongodb --test mongodb_tests mongodb_integration_reindex_page_batches_index_writes
cargo test -p helios-persistence --features mongodb --test mongodb_tests mongodb_integration_reindex_page_counts_contained_entries
cargo test -p helios-persistence --features mongodb --test mongodb_tests mongodb_integration_reindex_page_is_a_no_op_when_search_offloaded
cargo test -p helios-persistence --features mongodb --test mongodb_tests mongodb_integration_schema_v9_swaps_in_the_reindex_scan_index
cargo test -p helios-persistence --features mongodb --test mongodb_tests test_defer_indexing_skips_the_search_index_but_stores_everything_else
cargo test -p helios-persistence --features mongodb --lib search::reindex
cargo test -p helios-hfs --features mongodb --bin hfs test_build_mongodb_config
```
Expected: all PASS unchanged (this plan touched none of their production code paths). The last line expects `4 passed` — the hfs config tests are named `test_build_mongodb_config_overlays_env_with_mongo_database_url`, `..._ignores_non_mongo_database_url`, `..._reads_index_build_and_rejects_invalid_values` and `..._uses_mongo_specific_url_before_database_url_fallback` (`crates/hfs/src/main.rs`'s `mod tests`); there is no test named `build_mongodb_config_with_env` (that is the *production* function under test, at `main.rs:299`), so a filter on that exact string would silently run zero tests.

- [ ] **Step 2: The whole MongoDB integration suite, once, on 5.0.6**

Docker or `HFS_TEST_MONGODB_URL` is required for this step to mean anything; if neither is available, this step cannot be completed locally — say so in the PR description and rely on CI rather than skipping silently.

Run:
```
cargo test -p helios-persistence --features mongodb --test mongodb_tests 2>&1 | tee "$LOGDIR/mongodb_tests_full.log"
grep -qE "running 0 tests|Skipping" "$LOGDIR/mongodb_tests_full.log" && { echo "NOT RUN: Docker/HFS_TEST_MONGODB_URL unavailable"; exit 1; }
```
Expected: PASS in full (the harness default `Mongo::default()` pins 5.0.6 via `testcontainers-modules` 0.15.0's `TAG` constant).

- [ ] **Step 3: MongoDB 7.0, manually, in a throwaway container — never `hfs-mongo`**

```bash
docker run -d --name hfs-mongo70-walktest -p 27070:27017 mongo:7.0 --replSet rs0 --bind_ip_all
docker exec hfs-mongo70-walktest mongosh --quiet --eval 'rs.initiate({_id:"rs0",members:[{_id:0,host:"localhost:27017"}]})'
until docker exec hfs-mongo70-walktest mongosh --quiet --eval 'db.hello().isWritablePrimary' 2>/dev/null | grep -q true; do
  sleep 1
done
HFS_TEST_MONGODB_URL='mongodb://localhost:27070/?directConnection=true' \
  cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_id_walk
docker rm -f hfs-mongo70-walktest
```
Expected: `test result: ok. 12 passed` (T1-T12) on 7.0. Record the result (pass/fail, and T9's plan assertions specifically, since `{profile: 2}` support can differ) in the PR description — CI only exercises 5.0.6.

- [ ] **Step 4: Lint, build, format**

```bash
cargo clippy -p helios-persistence --features mongodb --all-targets -- -D warnings \
  -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark \
  -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default \
  -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation
cargo check -p helios-hfs --features mongodb --tests
rustfmt --edition 2024 crates/persistence/src/backends/mongodb/storage.rs crates/persistence/src/backends/mongodb/backend.rs crates/persistence/src/backends/mongodb/schema.rs crates/persistence/tests/mongodb_tests.rs crates/persistence/tests/mongodb/reindex_id_walk.rs crates/hfs/src/main.rs > /dev/null
git diff --stat -- crates/persistence/src/backends/mongodb/storage.rs crates/persistence/src/backends/mongodb/backend.rs crates/persistence/src/backends/mongodb/schema.rs crates/persistence/tests/mongodb_tests.rs crates/persistence/tests/mongodb/reindex_id_walk.rs crates/hfs/src/main.rs
```
The allow-list matches CI's own clippy invocation exactly (`.github/workflows/ci.yml:558`) — omitting it would surface pre-existing lints in code this plan does not touch (e.g. the nested `if let` at `mongodb_tests.rs:4325-4330`) and invite unrelated edits. `--all-targets` (not `--tests`) so the lib's `reindex_walk_tests` module and the new integration test module are both linted.
Expected: clippy clean; `helios-hfs` compiles; `rustfmt` makes no further changes (or, if it does, `git add` the reformatted files and fold that into this task's commit).

- [ ] **Step 5: Diff gate**

```bash
git diff origin/main -- crates/persistence/src/backends/mongodb/search_index_catalog.rs crates/persistence/src/backends/mongodb/search_impl.rs 'docs/mongodb/*.mongosh.js'
```
Expected: empty output.

```bash
grep -n "pub const SCHEMA_VERSION" crates/persistence/src/backends/mongodb/schema.rs
grep -n "const SEARCH_INDEX_GENERATION" crates/persistence/src/backends/mongodb/search_index_catalog.rs
```
Expected: `SCHEMA_VERSION` still `= 10;`, `SEARCH_INDEX_GENERATION` still `= 3;` (unchanged from `origin/main`). Matching on `"SCHEMA_VERSION"` alone would print the doc comment at `schema.rs:29` ("`SCHEMA_VERSION` does ...") instead of the constant at `:31`, so the check must anchor on `pub const SCHEMA_VERSION`.

```bash
git diff origin/main -- crates/persistence/src/backends/mongodb/schema.rs
```
Expected: exactly three things — the two new constants, the two literal replacements at the two `create_index` calls, and one appended comment paragraph. No index key, name-string-outside-the-constant, or option change.

- [ ] **Step 6: Commit any fix-ups**

If Steps 4-5 required changes (formatting, a clippy fix, a stray warning), commit them:

```bash
git add crates/persistence/src/backends/mongodb/storage.rs crates/persistence/src/backends/mongodb/backend.rs crates/persistence/src/backends/mongodb/schema.rs crates/persistence/tests/mongodb_tests.rs crates/persistence/tests/mongodb/reindex_id_walk.rs crates/hfs/src/main.rs
git commit -m "$(cat <<'EOF'
chore(mongodb): fmt/clippy tidy-up for the id-order $reindex walk (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

If nothing needed changing, skip the commit — do not create an empty one.

---

### Task 10: Open the PR (do not merge) and, after the verdict, append the benchmark rows

**Files:** none new — PR metadata and, after the gate verdict lands, `docs/mongodb-reindex-benchmark.md`.

- [ ] **Step 1: Push and open the PR**

Per the Global Constraints "No cargo while a bench arm is running" rule, `git push`/`gh pr create` are not cargo/test/build commands and need no lock check.

```bash
git push -u origin perf/1403-pr1-mongodb-id-walk
gh pr create --base main --head perf/1403-pr1-mongodb-id-walk \
  --title "perf(mongodb): id-order \$reindex walk with catch-up rounds (#1403 PR1)" \
  --body "$(cat <<'EOF'
## Summary
- Rewrites MongoDB's `$reindex` source (`fetch_resources_page`) as a
  two-phase walk: an id-order pass over everything live before the walk
  started (hinted to `idx_resources_identity`), followed by up to three
  bounded catch-up rounds in `(last_updated, id)` order (hinted to
  `idx_resources_type_scan`) that heal anything written during the walk.
- No `ReindexSource`/`ReindexTarget` trait change, no driver change, no
  index key/name/generation change, no persisted state. New: one clamped
  `MongoBackendConfig::reindex_catch_up_margin_ms` field (default 120 000 ms,
  no `HFS_*` env var in this PR) and a versioned `v2|...` cursor grammar.

## Design
- Spec: [`docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md`](../blob/main/docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md) (§4.2, §9)
- File-level design: `manual-test/archive/1403-run17-evidence/design/S2-pr1-id-walk.md` — local evidence only, not committed to this repo; SHA-256 `ad782e257989392cef99e2ecbea58a036b37caa3530c1da3710cc92c805e8a4e` (recorded in the plan this PR implements, `docs/superpowers/plans/2026-09-23-1403-pr1-mongodb-id-walk.md`).

## Gate (PR1 + CU-1)
Pending — this PR's merge gate is Gate PR1 (B1-s vs B0-s at C_bench, spec §4.7/§7.5) plus CU-1 (correctness under concurrent writes, §7.6), run by the orchestrator on this branch. The result tables are pasted here with `gh pr edit` once that run completes; they are not part of this PR's test plan below, which the implementer runs directly.

## Test plan
- [x] `cargo test -p helios-persistence --features mongodb --lib reindex_walk_tests`
- [x] `cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_id_walk` (Docker; T1-T12, see Task 9 Step 2)
- [x] MongoDB 7.0 manual gate: `12 passed` (Task 9 Step 3)
- [x] `cargo clippy -p helios-persistence --features mongodb --all-targets -- -D warnings` (repo's standard `-A` allowances)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Do not merge before Gate PR1 and CU-1 pass.

- [ ] **Step 2: After the verdict, append the benchmark rows**

Once Gate PR1 and CU-1 have a verdict, in one commit append the run-17 reference to §8.1 and the `B0-s`/`B1-s`/`CU-1` rows to §8.2–§8.4 of `docs/mongodb-reindex-benchmark.md`, using the exact column headers already in that file:

```
### 8.1 Run-17 reference

| Window | min | rows/s | res/s | ms/res | bm KB read/row | bm KB written/row | miss % | iowait cores |
|---|---:|---:|---:|---:|---:|---:|---:|---:|

### 8.2 Arms: outcome

| Arm | Gate | Source SHA | Binary SHA256 | Cache / memory MiB | Ingest wall | Rebuild wall | Obs walk ms/res | Q1 res/s | Q2 res/s | Q3 res/s | Q4 res/s | Q4/Q1 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|

### 8.3 Arms: mechanism

| Arm | Q4 bm KB read/row | Q4 si KB read/row | Q4 miss % | bm KB written/row | iowait cores | Checkpoint max s |
|---|---|---|---|---|---|---|

### 8.4 Arms: correctness and guardrails

| Arm | Procedure ms/res | entriesCreated | errorCount | HFS peak MiB | fg p99 ms | Valid | Verdict |
|---|---|---|---|---|---|---|---|
```

Fill one data row per arm (run-17 into §8.1; `B0-s`, `B1-s`, `CU-1` into each of §8.2–§8.4) from the gate's own `results/<gate>__*.md` files — never from memory or estimate. Commit:

```bash
git add docs/mongodb-reindex-benchmark.md
git commit -m "$(cat <<'EOF'
docs(mongodb): record the run-17 reference, B0-s, B1-s and CU-1 rows (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**Spec/design coverage** (spec §4.2 and S2, mapped to the task that implements or tests it):

| Requirement | Task |
|---|---|
| Branch cut from `origin/main`; anchors re-verified against it | Task 0 |
| `MongoBackendConfig::reindex_catch_up_margin_ms` + exhaustive literal in `crates/hfs/src/main.rs` | Task 1 |
| `schema.rs` index-name constants (`RESOURCES_IDENTITY_INDEX`, `RESOURCES_TYPE_SCAN_INDEX`) + comment | Task 2 |
| v2 cursor grammar: parse/encode, rejection as `SearchError::InvalidCursor` | Task 3 (pure logic) + T12 (Task 4 Part A, live) |
| Floor/ceiling/margin rules (§3.2, §3.4 step 3, §3.10) | Task 3 |
| Round-start decision / round cap (§3.4 step 1, §3.7) | Task 3 (pure logic) + T8 (Task 8, live) |
| Id phase (hint `idx_resources_identity`, keyset on `id`, `last_updated < floor`) | Task 3 (filter) + Task 4 Part A (wiring) + T1 (Task 4 Part B)/T2 (Task 4 Part A)/T9 (Task 5) |
| Newest-live covered probe | Task 4 Part A (`reindex_newest_live_last_updated`) + T9 (Task 5, covered-scan assertion) |
| Catch-up rounds 1-3 (hint `idx_resources_type_scan`) | Task 3 (filter) + Task 4 Part A (wiring) + T8 (Task 8) |
| Round de-dup keeping newest (`dedupe_reindex_page_keep_last`) | Task 3 |
| Walk log lines (exact messages/fields/levels, §3.9) | Task 4 Part A (emission) + T1 (Task 4 Part B: `walk started`/`round finished`), T8 (Task 8: `round started` round=3, cap WARN), T11 (Task 7: future-stamp WARN), T10 (Task 7: `round finished walked=50`) |
| `write_search_entries_page`/`write_search_entries_page_timed` precondition doc comment (PR0 relocates it; see Task 4 Part B Step 8) | Task 4 Part B |
| `docs/mongodb/search-indexes.md` walk section | Task 4 Part B |
| `mod reindex_walk_tests` (cursor, floor, ceiling, margin, round decision, filters, dedupe) | Task 3 |
| T1 (parity) | Task 4 Part B |
| T2 (order/isolation) | Task 4 Part A |
| T3 (id-phase heal) | Task 6 |
| T4 (round heal) | Task 6 |
| T5 (deferred create below the cursor, mid-walk via `MutatingSource` trigger `obs-030`) | Task 7 |
| T6 (delete mid-walk) | Task 7 |
| T7 (cancel/rerun) | Task 8 |
| T8 (termination/round cap) | Task 8 |
| T9 (plans, explain assertion) | Task 5 |
| T10 (import tail) | Task 7 |
| T11 (future stamp, every other live id written exactly once) | Task 7 |
| T12 (foreign cursor rejected) | Task 4 Part A |
| Schema-constants import (`use super::schema::{..}`) into `storage.rs` | Task 4 Part A Step 3 |
| Module-level `ReindexSource`/`ReindexTarget`/`StorageResult`/etc. imports in `reindex_id_walk.rs` | Task 4 Part A Step 1 |
| MongoDB 5.0.6 (CI) + manual 7.0 run | Task 9 |
| Existing tests stay green; diff gate on untouched files; `SCHEMA_VERSION`/`SEARCH_INDEX_GENERATION` unchanged | Task 9 |

**Placeholder scan:** every step above shows the actual test code and the actual implementation code (no "TBD", "add error handling", "similar to Task N", or "write tests for the above"). `LegacyWalkSource::fetch_resources_page` (Task 4 Part B) is final code — one `find`, an `advance`/`deserialize_current` drain loop matching HEAD's own idiom, no throwaway duplicate block. T9's `filter_has_or` extraction (Task 5) needs no runtime adjustment: a find command's filter is always recorded at `command.filter` in `system.profile`. No adaptation points remain.

**Type consistency:** `ReindexWalkCursor`, `WalkStep`, `RoundStartDecision`, and all eight pure functions introduced in Task 3 are defined once and reused without modification through Task 4; Task 4 Part A is the only place that defines `reindex_page_from_docs`, `reindex_newest_live_last_updated`, `reindex_find_page`, and `legacy_filter`, and every later task calls the public trait method `fetch_resources_page` rather than any of these internals. `RESOURCES_IDENTITY_INDEX`/`RESOURCES_TYPE_SCAN_INDEX` (Task 2) are consumed only in Task 4 Part A, via the `use super::schema::{..};` import that Part A's Step 3 adds. `ReindexSource`/`ReindexTarget`/`StorageResult`/`ReindexOperation`/`ReindexRequest` are imported once, at `reindex_id_walk.rs`'s module scope (Task 4 Part A Step 1), and every later task's test relies on that scope rather than re-importing; `ReindexStatus`/`ResourcePage`/`TenantSearchRegistries` are deliberately left out of that import and referenced fully qualified everywhere, since none of the three is ever named bare in this file and importing an unused plain type (not a trait) would be an `unused_imports` error. Test doubles are introduced exactly once each and reused by name in later tasks: `MutatingSource`/`RecordingTarget` (Task 6) are reused by T5/T6 (Task 7) and by T7/T8 (Task 8, `RecordingTarget` only); `PausingSource`/`create_backend_with_catch_up_margin` (Task 8) are new to that task since no earlier task needed pausing or a shortened margin. `seed_walk_fixture`/`backdate_fixture`/`ts` (Task 4 Part A) and `snapshot`/`capture_walk_logs`/`walk_log_lines`/`LegacyWalkSource`/`legacy_filter`/`wait_for_terminal` (Task 4 Part B) are the harness every later task builds on without redefinition; `seed_submission` (Task 7) is its own local copy of a function private to `mongodb_tests.rs`'s `bulk_submit` module, not a duplicate of anything already in scope.

**Verified against the codebase at `c86d0f08b`** (read-only; nothing in this repo was built, run or committed while revising this plan): `storage.rs`'s import block has no `schema` import (blocker fix 1); `mongodb_tests.rs` imports `StorageResult`/`ReindexSource`/`ReindexTarget`/etc. only inside function/module bodies, never at file scope (blocker fix 2); `schema` is `pub(crate) mod schema;` (`mod.rs:26`), so no integration test can reach a `pub(crate)` constant in it (Task 2's rewrite); `collect_index_names` (`mongodb_tests.rs:4340`) does not de-duplicate, and the design's own measured plan for a round's continuation query is `SORT_MERGE` of two `idx_resources_type_scan` `IXSCAN`s (S2 §2, run 17), confirming T9's `names` fix; `test_defer_indexing_skips_the_search_index_but_stores_everything_else` (`mongodb_tests.rs:10361-10365`) asserts a deferred create's `search_index_entry_count` is `0`, confirming T5 needed to move its create to mid-walk; S1-pr0-instrumentation.md §7.4 (lines 875-879) confirms PR0 renames `write_search_entries_page` to `write_search_entries_page_timed` and relocates its doc comment, which Task 4 Part B Step 8 now accounts for; `.github/workflows/ci.yml:558` gives the exact clippy allow-list now used in Task 9 Step 4; `main.rs:3897,3933,3953,3980` give the four real `test_build_mongodb_config_*` names used in Tasks 1 and 9. One additional issue was found and fixed beyond the review above: the review's own suggested module-level import list for `reindex_id_walk.rs` (`ReindexStatus`, `ResourcePage`, `TenantSearchRegistries` alongside `ReindexOperation`/`ReindexRequest`/`ReindexSource`/`ReindexTarget`) would itself have failed `-D warnings`, because none of those three plain types (as opposed to the two traits) is ever referenced by its bare name anywhere in this file — every use is fully qualified. They are left out of the import (Task 4 Part A Step 1) accordingly.

**Not verified** (would require building/running, which this read-only revision could not do): that the plan's Rust compiles end-to-end (in particular the `let [round, floor, ceiling, walked, after_lu, after_id] = fields[..] else { .. }` slice pattern in `ReindexWalkCursor::parse`, and the two `clippy::manual_range_contains`/`clippy::let_unit_value` fixes applied by inspection); that MongoDB 5.0.6 and 7.0 both plan a round's continuation query as `SORT_MERGE` rather than a single filtered `IXSCAN` (T9's `names` fix assumes this, per the design's own run-17 measurement, not a fresh run); T7/T8's timing assumptions about `ReindexOperation::cancel`/the round loop under load. These carry over from the original review and are unchanged by this revision.
