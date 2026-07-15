# Cluster-capable state — implementation roadmap & status

**Status:** living tracker — update as each phase/PR lands
**Branch:** `feat/cluster-capable-state` (off `main`)
**Last updated:** 2026-07-15 (**Phase 1 COMPLETE** — gate met)
**Companions:**
[`cluster-capable-state-design.md`](./cluster-capable-state-design.md) (the design, mirror of discussion #223) ·
[`cluster-testing-strategy.md`](./cluster-testing-strategy.md) (T1/T2/T3 tiers, DoD map, per-phase test plans) ·
[`cluster-testing-methodology.md`](./cluster-testing-methodology.md) (how to apply the strategy per PR)

**Detailed execution plan:** the approved implementation plan (full per-phase
design decisions, PR breakdown, file-level detail) lives at
`~/.claude/plans/read-docs-cluster-testing-strategy-md-do-smooth-hamster.md`
on the development machine (Claude Code plan file; also summarized in the
project memory under `cluster-testing-strategy`). This roadmap carries enough
context to resume without it; the plan file adds file:line-level detail.

---

## Where things stand

### ✅ Done — pre-Phase 0 (T3 calibration, 2026-07-14)

| What | Where | Commit |
|------|-------|--------|
| T3 two-instance smoke harness (two `hfs` + shared Postgres + nginx round-robin front, `X-Hfs-Upstream` proves distribution) | `.github/workflows/cluster-smoke.yml` + `crates/hfs/tests/cluster/run_external_cluster_smoke.sh` | PR #256 (merged to `main`; dispatch with `ref=feat/cluster-capable-state`) |
| **D3** Postgres schema-init race fix (`pg_advisory_lock`, key `0x4846_5353_4348_454D` "HFSSCHEM") — found by the harness's first dispatch | `crates/persistence/src/backends/postgres/schema.rs` | `2bb443d6` |
| D3 T2 regression suite (4 fresh handles, barrier-raced cold start) | `postgres_integration_cluster_concurrent_cold_start_schema_init` in `crates/persistence/tests/postgres_tests.rs` | `b89c6fe9` |
| Harness fix: nginx readiness probe must not proxy through not-yet-started upstreams (`/nginx-health` + `max_fails=0`) | workflow nginx config | `608842c6` |

### ✅ Done — Phase 0: framing & guardrails (2026-07-14)

| What | Where | Commit |
|------|-------|--------|
| The three governing docs committed; design §6 amended (`HFS_AUDIT_BACKEND`, not `HFS_AUDIT_SINK`) | `docs/cluster-*.md` | `d0b5eecc` |
| **T2 `cluster_harness` scaffold** — `two_handles` (fresh constructions, never a cloned `Arc`), `race2` (barrier-synchronized), DoD assertions (`assert_exactly_one` / `assert_visible` / `assert_wrong_tenant_hidden`) | `crates/persistence/tests/common/cluster_harness.rs`, included from `postgres_tests.rs` via `#[path]` (nothing else includes `tests/common/`; a wholesale `mod common;` trips dead-code under `-D warnings`) | `1a876e3f` |
| **Calibration suite** against the already-cluster-safe bulk-export job store — green on first run: visibility, wrong-tenant isolation, claim exclusivity (drain queue first!), fencing (deterministic release→reclaim→stale-token `LeaseLost`, no sleeps), durability (handle drop) | `postgres_integration_cluster_bulk_export_*` in `postgres_tests.rs` | `1a876e3f` |
| **`HFS_CLUSTER`** master switch + **`HFS_JOB_STORE_BACKEND`** selector (`JobStoreBackend { Memory, Database }`; unset → `Database` under cluster; parsed + validated, **not consumed yet**) | `crates/rest/src/config.rs` (+ re-export in `lib.rs`) | `8932b9b5` |
| **Fail-fast validator** — refuses under `HFS_CLUSTER=true`: SQLite primary (F1), `HFS_AUTH_JTI_BACKEND=memory` with auth on (C1), `local-fs` bulk export/submit output with the subsystem on (F2), `HFS_AUDIT_BACKEND=file` (F4), explicit `memory` job store. Pure fn over `ClusterConfigView` (T1 table needs no env mutation); lives in `hfs` because rest+auth+audit config only meet in the binary. Verified end-to-end (boot refusal with named env vars). | `crates/hfs/src/cluster.rs`, wired in `main()` after `storage_backend_mode()` | `8932b9b5` |
| Operator doc: book chapter *Running HFS in a Cluster* + `run-hfs-server` skill Clustering section | `book/src/ch15-cluster-deployment.md`, `book/src/SUMMARY.md`, `.claude/skills/run-hfs-server/SKILL.md` | `08fe9c08` |

**Phase 0 gate (strategy §8): met.** Validation table green (6 T1 tests),
calibration suite green on first run (4 T2 tests), full CI-style clippy and
`cargo fmt` clean.

**CI baseline (2026-07-14): all green.**
- Branch pushed; **draft PR [#269](https://github.com/HeliosSoftware/hfs/pull/269)**
  is the long-running CI vehicle for the effort — every push gets the full
  suite; phases get marked ready / peeled out when merge-worthy.
- Full CI on the Phase 0 head ([run 29363363507](https://github.com/HeliosSoftware/hfs/actions/runs/29363363507)):
  Test Rust ✅ (**all five T2 cluster suites ran green on the CI Docker
  host** — 4 calibration + D3 cold-start), Code Coverage ✅ (T2 suites now
  feed the metric per strategy §7), Linting / Security Audit / FHIRPath /
  Python / codecov-patch ✅. One transient failure along the way: the known
  `SystemOverloadedError` MongoDB flake on the shared Docker host
  (unrelated; passed on re-run of the failed job).
- T3 cluster-smoke re-dispatched on the Phase 0 head: ✅
  ([run 29361875226](https://github.com/HeliosSoftware/hfs/actions/runs/29361875226)).
- **Gotcha (cost two dead pushes):** GitHub skips `pull_request` workflows
  when the skip-CI directive appears *anywhere in the head commit message —
  including quoted in the body*. On this branch, never quote it in a commit
  that should trigger CI; after a docs-only skip-tagged head, the PR shows
  "no checks" until the next clean-message push.

### ✅ Done — Phase 1, PRs 1.1–1.3 (2026-07-15)

| What | Where | Commit |
|------|-------|--------|
| **F5 fix** — Postgres `update`/`delete` are a transactional version-guarded CAS (0 rows → re-select to disambiguate NotFound vs VersionConflict; history INSERT in the same txn); `create_or_update` absorbs transient CAS losses with a bounded retry (unconditional PUT stays last-writer-wins at the API surface). T2 races: update/update, PUT/PUT, update/delete + Mongo CAS-contract twin (`mongodb_tests.rs` gained the `#[path]` harness include and a shared-database factory — `create_backend` isolates per-call DBs, useless for cluster tests). Session decision: **CAS + retry**, not the unguarded atomic increment — preserves the trait contract asserted by sqlite/mongo. | `postgres/storage.rs`, `postgres_tests.rs`, `mongodb_tests.rs` | `d1204bf6` |
| **PR 1.2 — `ClusterJobStore`** on a `cluster_jobs` table (migration **v15** — main took v14 for the tenant registry). Claim = bulk-export shape (txn + `FOR UPDATE SKIP LOCKED` + fencing bump); fenced mutations require `status='running'` so cancel is final; `cancel` flips queued/running → cancelled immediately + sets `cancel_requested`. Server seam mirrors `sof_runner()`: `ResourceStorage::cluster_job_store()` default-None; Postgres returns a pool-sharing `PgClusterJobStore` handle (impl NOT on the backend — would collide with `ExportClaimStrategy`'s method names). `WorkerId` reused; `LeaseError` not (carries a bulk `ExportJobId`) → `ClusterLeaseError`. T1 state-machine suite vs `testing::InMemoryClusterJobStore`; T2 DoD suite (visibility/isolation/exclusivity/fencing/durability/cross-instance cancel) via the harness, driven through the accessor seam. | `core/cluster_job_store.rs`, `postgres/cluster_jobs.rs`, `postgres/schema.rs` | `6dfb7097` |
| **PR 1.3 — SoF export on the job store (#169)**: `ExportJobController` → `#[async_trait]`; execution engine shared between controllers via a `JobProgress` hook; new `DatabaseExportJobController` + per-instance claim/lease workers (heartbeat task, progress snapshots, cancel observed between work units, fenced-out terminal write deletes orphaned shards) + kind-scoped reaper (`delete_terminal_before` now returns reaped ids). `HFS_EXPORT_CONTROLLER` finally read (unset → follows job-store mode); validator refuses explicit `memory` controller + `fs` sink under cluster+SoF (T1 table now 8 rows). T2 two-controller suite (`rest/tests/sof_export_cluster_pg.rs`, deterministic via `run_next_sof_export_job`); **T3 smoke check 4** (kickoff on A → poll via front → download via B) with `HFS_JOB_STORE_BACKEND=database` + shared export dir in the workflow (`HFS_CLUSTER` stays unset there — the validator rightly refuses `fs` sinks; S3 out of smoke scope). ch15 SoF row flipped. | `rest/src/export/{controller,in_memory,database}.rs`, `rest/src/lib.rs`, `hfs/src/cluster.rs`, smoke script + workflow | `7bf5bf35` |

| **PR 1.4 — reindex on the job store (A2)**: `ReindexOperation::with_cluster_store` — `start` enqueues `JobKind::Reindex`, any instance answers status/cancel, per-instance workers claim and run the untouched `run_reindex` via a bridge task (local progress map → store snapshots; cross-instance cancel/lease-loss → local cancel channel). `get_progress`/`cancel`/`list_jobs` now tenant-checked in **both** modes (the in-memory map previously served any tenant's job by UUID). hfs wires the store through the ops bundles under the database job-store mode (backends without a store warn + stay per-instance). T2 suite: visibility+list, isolation, cross-instance execution via deterministic `run_next_cluster_job`, cross-instance cancel, durability. | `search/reindex.rs`, `handlers/reindex.rs`, `hfs/main.rs` | `24517f54` |
| **PR 1.5 — nightly kill-9 (A1)**: `cluster-smoke.yml` gains `schedule` + `nightly=true` dispatch input gating `run_nightly_kill9_check.sh` — stop B, seed 200k patients via psql, kick off on A, wait for the claim, `kill -9` A, restart B from the persisted env; assert reclaim after lease expiry under a new worker_id + bumped fencing token, completion, and a full-count shard via B. **Passed on first dispatch.** Scheduled runs execute main's copy — until merge, exercise via dispatch `-f nightly=true`. | workflow + `run_nightly_kill9_check.sh` | `9955e79c` |

**Phase 1 gate (strategy §8): MET 2026-07-15.** A1+A2+F5 T2 suites green in CI
on the merged head ([run 29430658948](https://github.com/HeliosSoftware/hfs/actions/runs/29430658948));
smoke check 4 (SoF export A→front→B) green ([run 29427174805](https://github.com/HeliosSoftware/hfs/actions/runs/29427174805)
and again on the merged head, [run 29430704358](https://github.com/HeliosSoftware/hfs/actions/runs/29430704358));
nightly kill-9 green ([run 29428583874](https://github.com/HeliosSoftware/hfs/actions/runs/29428583874)).

**Context (2026-07-15):** main broke on the #233 merge (semantic conflict —
`concepts_search_fts` never created; latent since `f6008b29`) and was fixed by
**PR #273** (not ours; we closed duplicate #276 — check open PRs before
authoring a fix). main was then merged into this branch (`9cfd120b`), bringing
the `$purge`/`$reindex` REST endpoints (`3177ec22`), which 1.4 built on.
A second main merge (`e0cb5198`) absorbed **#205 — the jti subsystem was
removed** (auth is stateless): the validator's C1 check was deleted with it
and Phase 2 rescoped to C2 only (see below).

---

## What's next

Confirmed design decisions (do not re-litigate): **(1)** `ExportJobController`
converts to `#[async_trait]`; **(2)** DB-backed jti + JWKS coordination
variants are built (a DB-only cluster needs no Redis); **(3)** C3 uses a
shared **epoch** table, not LISTEN/NOTIFY (amend design §6 when it lands).

### ▶ Phase 1 — unified job store + SoF export (#169) + reindex (A2) + F5

PR-by-PR (each independently shippable, feature + DoD tests together):

1. **PR 1.1 — F5 version-id race fix** *(start here; first product-code use of the harness)*
   - `crates/persistence/src/backends/postgres/storage.rs` `update` (~:251) and `delete` (~:363): replace the non-transactional SELECT→parse+1→UPDATE→history-INSERT with **one transaction** using `UPDATE … SET version_id = (version_id::bigint + 1)::text … RETURNING version_id`, history insert with the returned version.
   - Mongo is already a version-guarded CAS (`mongodb/storage.rs` ~:842) — add the T2 test only.
   - T2: `race2` two unconditional updates from two fresh backends → distinct version_ids, both history rows.
2. **PR 1.2 — `ClusterJobStore`** — new trait in `crates/persistence/src/core/cluster_job_store.rs` (`enqueue`/`claim_next`/`heartbeat`/`get_status`/`cancel`/`cancel_requested`/`update_progress`/`complete`/`fail`/`delete_terminal_before`; `JobKind { SofExport, Reindex }`; payloads as JSON; reuse `WorkerId`). New `cluster_jobs` table, migration v13→v14 in `postgres/schema.rs`; Postgres impl clones the claim shape from `bulk_export.rs:533-602` (`FOR UPDATE SKIP LOCKED` + fencing token). Do **not** generalize the bulk-export traits — bulk stays as-is (F3 decision). T1 in-memory fake + full T2 DoD suite via the harness.
3. **PR 1.3 — SoF export on it** — `ExportJobController` → `#[async_trait]`; new `DatabaseExportJobController` (`crates/rest/src/export/database.rs`); per-instance worker loop modeled on `spawn_export_workers` (`hfs/main.rs` ~:1048); wire in the `sof_enabled` block (`rest/src/lib.rs` ~:543-648) selected by `job_store_backend_mode()`, finally reading `HFS_EXPORT_CONTROLLER`; extend the cluster validation table (cluster + SoF needs `database` controller + non-fs `HFS_EXPORT_SINK`). T2 two-controller suite + **T3 smoke check 4** (`$export` on A → poll via front → download via B).
4. **PR 1.4 — reindex (A2)** onto the same table (store-backed mode for `ReindexOperation`, `persistence/src/search/reindex.rs` ~:357). T2 DoD suite for `JobKind::Reindex`.
5. **PR 1.5 — nightly kill-9 (A1)** — `on: schedule` for `cluster-smoke.yml` + `NIGHTLY`-gated check: kill -9 instance A mid-export, B claims the orphaned lease and completes.

### Phase 2 — auth hardening (~~C1~~, C2) — **rescoped 2026-07-15**

- **C1 is OBSOLETE**: main's #205 (merged 2026-07-15, PRs #268/#230) removed
  the jti replay-cache subsystem entirely — access tokens are not one-time
  assertions, so token validation is stateless and auth holds no
  cross-instance state. The `HFS_AUTH_JTI_BACKEND` fail-fast check was
  removed from the cluster validator in the same merge that absorbed #205
  into this branch. The planned `DatabaseJtiCache`, memory-jti unsafe-contract
  tests, and Redis jti twin are all moot.
- What remains of Phase 2 is **C2 only**: `JwksCoordination` trait in auth
  (dedupe concurrent IdP refreshes across instances); impls: Redis
  `JwksCoordinator` (verify it still exists post-#205) + Postgres
  advisory-lock impl; wire into `JwksCache`'s refresh seams. T2 C2 via
  `wiremock` mock-IdP hit counter. Re-ground line numbers before starting —
  #205 reshaped `crates/auth`.
- CI: `redis-cluster-tests.yml` only if the Redis coordinator survives;
  otherwise no new CI.

### Phase 3 — subscriptions cluster delivery (#170)

- `EventFanout` trait; `PgNotifyFanout` on a **dedicated non-pooled** `tokio_postgres` connection with reconnect/re-LISTEN loop; NOTIFY payload ≈8KB cap ⇒ envelopes `(tenant, subscription_id, event_id)`, receivers rehydrate from DB. `HFS_SUBSCRIPTIONS_FANOUT = memory | pg-notify`.
- B3 startup hydration (there is **zero** startup load of Subscription/SubscriptionTopic today — engine built empty at `rest/src/lib.rs` ~:713); B4 shared counters (`subscription_state` table, `UPDATE … event_number = event_number + 1 RETURNING`); B2 DB redeem-once binding tokens (`DELETE … RETURNING`); B5 durable delivery outbox (jobs-shaped lease columns), replacing the on-stack retry in `dispatch_with_retry` (`engine/mod.rs` ~:722). All migration v16.
- **T3 B1 is the one mandatory two-process test**: WS client on B, POST matching resource to A, socket receives; plus sticky-session negative. Reuse the FIFO WS client from `run_external_subscriptions_smoke.sh`.

### Phase 4 — HTS coherency (C3) + bootstrap lock (D1) + composite outbox (E1)

- C3: one-row `terminology_epoch` table; every terminology write bumps it; an `EpochGuard` (memoized ~1s, 0 in tests) checks before serving from cache and calls the two existing clear seams (`hts/src/state.rs:280`, `backends/postgres/mod.rs:172`, incl. the `OnceLock` closure statics). `HFS_TERMINOLOGY_CACHE_INVALIDATION = local | epoch`.
- D1: `pg_advisory_lock` around `bootstrap_sync` (new key, e.g. "HFSBSTRP"), ledger re-check after acquisition, dedicated connection (HTS owns its own deadpool pool).
- E1: `composite_sync_outbox` (migration v17); `sync_asynchronous` writes the row, mpsc becomes a wake-up hint, workers claim with SKIP LOCKED + fencing. T3 nightly kill-9 drain case.

---

## How to resume (checklist for a fresh session)

1. Read the three companion docs, then this file; the approved plan file (path
   above) has the full file:line detail if present.
2. `git log origin/main..HEAD --oneline` — confirm where the branch is vs this
   file's tables.
3. The per-PR bar is methodology §6: DoD rows at T2 on the DB backend, two
   independently constructed handles, wrong-tenant isolation row always,
   deterministic (no sleeps) in `cargo test`, memory backend asserted as the
   *unsafe* contract where relevant, Redis twin behind
   `RUN_REDIS_CLUSTER_TESTS=1` where a Redis backend exists.
4. Reuse the harness: `crates/persistence/tests/common/cluster_harness.rs`
   (`#[path]`-included from `postgres_tests.rs`; do the same from other test
   binaries). Fresh-handle factory for Postgres is `create_backend()` in
   `postgres_tests.rs`.
5. T3: dispatch `cluster-smoke.yml` with `ref=feat/cluster-capable-state`
   (`gh workflow run cluster-smoke.yml --ref feat/cluster-capable-state`).
6. Local-toolchain gotcha: run clippy with **CI's exact flag set** (see
   `.github/workflows/ci.yml`, "Run clippy") — plain `-D warnings` reports
   `collapsible_if`/doc lints that CI deliberately allows; do not "fix" those.
7. Project hooks gate completion on `cargo fmt --all`, CI-style clippy, and an
   affected `cargo test`.

## Update discipline

When a PR lands: move its row into the ✅ tables above with its commit hash,
tick the phase gate when the strategy §8 gate criteria are met, and update the
per-subsystem matrix in `book/src/ch15-cluster-deployment.md` (it promises to
track phase progress). Keep design-doc changes as inline `[amended <date>]`
markers.
