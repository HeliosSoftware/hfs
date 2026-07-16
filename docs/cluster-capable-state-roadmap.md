# Cluster-capable state — implementation roadmap & status

**Status:** living tracker — update as each phase/PR lands
**Branch:** `feat/cluster-capable-state` (off `main`)
**Last updated:** 2026-07-16 (**Phase 4 COMPLETE** — gate met, full CI + cluster-smoke green; E1 nightly kill-9 case implemented, dry-run verified, and now dispatched green against real CI — see below)
**Companions:**
[`cluster-capable-state-design.md`](./cluster-capable-state-design.md) (the design, mirror of discussion #223) ·
[`cluster-testing-strategy.md`](./cluster-testing-strategy.md) (T1/T2/T3 tiers, DoD map, per-phase test plans) ·
[`cluster-testing-methodology.md`](./cluster-testing-methodology.md) (how to apply the strategy per PR)

**Detailed execution plans** (Claude Code plan files on the development
machine; also summarized in the project memory under
`cluster-testing-strategy` — this roadmap carries enough context to resume
without them):
- Phases 0–4 master plan: `~/.claude/plans/read-docs-cluster-testing-strategy-md-do-smooth-hamster.md`
- Phase 1 execution plan (F5 semantics decision, per-chunk verification): `~/.claude/plans/come-up-with-a-iridescent-spindle.md`
- Phase 4 execution plan (D1/C3/E1 file:line detail, the standalone-HTS-config decision, the E1 wiring/wake-hint reasoning): `~/.claude/plans/come-up-with-a-structured-llama.md`

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
| **Fail-fast validator** — refuses under `HFS_CLUSTER=true`: SQLite primary (F1), `HFS_AUTH_JTI_BACKEND=memory` with auth on (C1 — *check later removed with the jti subsystem itself, see #205 note below*), `local-fs` bulk export/submit output with the subsystem on (F2), `HFS_AUDIT_BACKEND=file` (F4), explicit `memory` job store. Pure fn over `ClusterConfigView` (T1 table needs no env mutation); lives in `hfs` because rest+auth+audit config only meet in the binary. Verified end-to-end (boot refusal with named env vars). | `crates/hfs/src/cluster.rs`, wired in `main()` after `storage_backend_mode()` | `8932b9b5` |
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

### ✅ Done — Phase 1: unified job store + SoF export (#169) + reindex (A2) + F5 (2026-07-15)

| What | Where | Commit |
|------|-------|--------|
| **PR 1.1 — F5 fix**: Postgres `update`/`delete` are a transactional version-guarded CAS (0 rows → re-select to disambiguate NotFound vs VersionConflict; history INSERT in the same txn); `create_or_update` absorbs transient CAS losses with a bounded retry (unconditional PUT stays last-writer-wins at the API surface). T2 races: update/update, PUT/PUT, update/delete + Mongo CAS-contract twin (`mongodb_tests.rs` gained the `#[path]` harness include and a shared-database factory — `create_backend` isolates per-call DBs, useless for cluster tests). Session decision: **CAS + retry**, not the unguarded atomic increment — preserves the trait contract asserted by sqlite/mongo. | `postgres/storage.rs`, `postgres_tests.rs`, `mongodb_tests.rs` | `d1204bf6` |
| **PR 1.2 — `ClusterJobStore`** on a `cluster_jobs` table (migration **v15** — main took v14 for the tenant registry). Claim = bulk-export shape (txn + `FOR UPDATE SKIP LOCKED` + fencing bump); fenced mutations require `status='running'` so cancel is final; `cancel` flips queued/running → cancelled immediately + sets `cancel_requested`. Server seam mirrors `sof_runner()`: `ResourceStorage::cluster_job_store()` default-None; Postgres returns a pool-sharing `PgClusterJobStore` handle (impl NOT on the backend — would collide with `ExportClaimStrategy`'s method names). `WorkerId` reused; `LeaseError` not (carries a bulk `ExportJobId`) → `ClusterLeaseError`. T1 state-machine suite vs `testing::InMemoryClusterJobStore`; T2 DoD suite (visibility/isolation/exclusivity/fencing/durability/cross-instance cancel) via the harness, driven through the accessor seam. | `core/cluster_job_store.rs`, `postgres/cluster_jobs.rs`, `postgres/schema.rs` | `6dfb7097` |
| **PR 1.3 — SoF export on the job store (#169)**: `ExportJobController` → `#[async_trait]`; execution engine shared between controllers via a `JobProgress` hook; new `DatabaseExportJobController` + per-instance claim/lease workers (heartbeat task, progress snapshots, cancel observed between work units, fenced-out terminal write deletes orphaned shards) + kind-scoped reaper (`delete_terminal_before` now returns reaped ids). `HFS_EXPORT_CONTROLLER` finally read (unset → follows job-store mode); validator refuses explicit `memory` controller + `fs` sink under cluster+SoF (T1 table grew to 8 collected violations — later 7 when C1 left with #205). T2 two-controller suite (`rest/tests/sof_export_cluster_pg.rs`, deterministic via `run_next_sof_export_job`); **T3 smoke check 4** (kickoff on A → poll via front → download via B) with `HFS_JOB_STORE_BACKEND=database` + shared export dir in the workflow (`HFS_CLUSTER` stays unset there — the validator rightly refuses `fs` sinks; S3 out of smoke scope). ch15 SoF row flipped. | `rest/src/export/{controller,in_memory,database}.rs`, `rest/src/lib.rs`, `hfs/src/cluster.rs`, smoke script + workflow | `7bf5bf35` |
| **Smoke check 4 fix** — the completion manifest's *first* `valueUri` is the top-level status URL (answers 303); the shard URL lives in the `output` part, so the script filters `/status$` endpoints before picking. First dispatch otherwise passed end-to-end. | `run_external_cluster_smoke.sh` | `e543bcdf` |
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

### ✅ Done — Phase 2: JWKS refresh coordination (C2) (2026-07-15)

**Session decision (2026-07-15):** the Redis coordinator (deleted by #205,
never wired) was **resurrected** as an opt-in second impl — user call: keep a
Redis scaffold for future cluster work. The DB-backed mode stays the default,
so a DB-only cluster still needs no Redis; standing decision (2) is resolved.

| What | Where | Commit |
|------|-------|--------|
| **PR 2.1 — `ClusterRefreshCache`** (persistence): generic string-keyed single-flight document refresh with **watermark** freshness (reuse iff stored doc is strictly newer than the caller's `fetched_at`, within `max_stale`, within its own `max_age` — what makes the rotation race deterministic with zero sleeps). `PgClusterRefreshCache`: per-key `pg_advisory_xact_lock` (seed "HFSRFRSH") in a txn, fetch closure runs under the lock, DB-clock ages; migration **v16** `cluster_refresh_cache` (server-global — no tenant column, isolation DoD row N/A by design). Seam: `ResourceStorage::cluster_refresh_cache()` default-None, mirroring `cluster_job_store()`. `testing::InMemoryClusterRefreshCache` = T1 reference. T2 in `postgres_tests.rs` (raced single-flight = exactly one fetch, durability, watermark refetch, error releases lock; **no** `CLUSTER_JOBS_TEST_LOCK` — per-test UUID keys are disjoint). | `core/cluster_refresh_cache.rs`, `postgres/cluster_refresh_cache.rs`, `schema.rs` | `b0fee864` |
| **PR 2.2 — auth seam**: `JwksFetcher` split into `fetch_raw` (preserves the raw body + Cache-Control — what the shared store holds) and `parse_document` (local per instance); `JwksCoordination` trait (`jwks/coordination.rs`) with `Fetch` (propagate) vs `Unavailable` (warn + **fall back to direct fetch**; watermark advances so a recovering store never installs older keys); `JwksCache` now `Clone`, background loop calls `refresh()` through a clone — the pre-existing duplicated inline fetch/swap is gone, ONE refresh seam; set-once `set_coordination()` (server Arc-wraps the cache before storage exists); reused docs get age-shortened TTL (floor 1s). Config: `HFS_AUTH_JWKS_COORDINATION` + re-added `HFS_AUTH_REDIS_URL` (both in `AUTH_ENV_KEYS`). | `auth/src/jwks/*`, `auth/src/config.rs` | `1554bd28` |
| **PR 2.3 — Redis resurrection**: `RedisJwksCoordination` behind the restored auth `redis` feature (redis 0.27, pre-#205 shape; moka stays gone) — `SET NX EX` lock with holder-token Lua compare-and-del, stored JSON doc with TTL, lock losers poll for the winner's doc, dead-holder bypass after lock TTL; every Redis error → `Unavailable` (never takes auth down). Gated T2 twin `auth/tests/jwks_cluster_redis.rs` (`RUN_REDIS_CLUSTER_TESTS=1`), identical assertions to the PG suite; new **`redis-cluster-tests.yml`** (auth-path PRs + nightly + dispatch; scheduled runs execute main's copy — effective on merge). | `auth/src/jwks/redis_coordination.rs`, workflow | `6673b00c` |
| **PR 2.4 — rest bridge + T2 suite**: `StoreJwksCoordination` adapts persistence's store to auth's trait (rest depends on both; no persistence↔auth edge). **The strategy §8 C2 suite**: `rest/tests/jwks_cluster_pg.rs` — two fresh `JwksCache`+backend pairs vs one wiremock IdP: barrier-raced boot herd → **exactly one upstream fetch**, late-joiner reuse → zero, rotation race → exactly one additional (loser adopts via watermark). wiremock added to rest dev-deps. | `rest/src/jwks_coordination.rs`, `rest/tests/jwks_cluster_pg.rs` | `949feee3` |
| **PR 2.5 — hfs wiring**: `resolve_jwks_coordination` (unset → `database` under `HFS_CLUSTER`; explicit `local` **warns, never refuses** — C2 is the one warn-only cluster concern, doc'd against the refusal table; `redis` needs the feature + URL; invalid values fail fast). `init_auth_with_audit` finishes local/redis boots itself and defers the database-mode initial fetch to `finish_auth_boot`, called on the **primary** handle in all 8 `start_*` fns (backends without a store warn + stay per-instance, the `reindex_cluster_store` posture). Verified end-to-end: two binaries + one Postgres + mock IdP → 1 IdP hit total, "Reused shared JWKS document" on B, refusal/warning cases exact. | `hfs/src/cluster.rs`, `hfs/src/main.rs` | `90c61cde` |

**Phase 2 gate (strategy §8, rescoped): MET 2026-07-15.** On the Phase 2 head
(`20056ff5` — an empty `ci:` commit; the docs head carried `[skip ci]`, the
known gotcha): full CI green
([run 29441746666](https://github.com/HeliosSoftware/hfs/actions/runs/29441746666)
— the C2 T2 suites ride `test-rust`), the new **Redis Cluster Tests**
workflow green on its first `pull_request` firing
([run 29441746418](https://github.com/HeliosSoftware/hfs/actions/runs/29441746418)
— gated twin against a real Redis container on the CI Docker host), and
cluster-smoke green
([run 29441743065](https://github.com/HeliosSoftware/hfs/actions/runs/29441743065)
— no regression; no new smoke check by design). Locally the same suites were
green first (6+4 persistence, 6 auth contract, 3 PG, 3 Redis twin, 6 hfs
resolution/validator), plus the two-binary end-to-end boot check.

**No T3 smoke addition** (deliberate): C2 changes no user-visible behavior —
same keys, same 401s; its only observable is the upstream IdP hit count,
which the strategy assigns to the T2 wiremock counter. A smoke case would
add flake surface without new coverage.

**Migration renumbering:** C2 took **v16** (`cluster_refresh_cache`), so the
Phase 3 items sketched below shift to **v17** and Phase 4 E1 to **v18**.

### ✅ Done — Phase 3: subscriptions cluster delivery (#170) (2026-07-15)

**Session decisions (2026-07-15):** the topic registry is now
**tenant-scoped** (`(tenant_id, canonical_url)`) in BOTH modes — user call,
closing the pre-existing cross-tenant topic-visibility leak; and
`CompositeStorage` now forwards **all** cluster seams to its primary,
including the Phase 1/2 `cluster_job_store`/`cluster_refresh_cache`
accessors it had silently dropped (latent gap: `postgres-elasticsearch`
modes reported no cluster stores).

| What | Where | Commit |
|------|-------|--------|
| **PR 3.1 — persistence substrate (migration v17)**: four core seams with `pub mod testing` T1 reference models + Postgres impls — `subscription_state` (lazy upsert-increment counters — an update can never reset them; cluster-visible status; `subscription_notification_events` bundle store; the cross-tenant registry-independent `SubscriptionHydrationSource` with the R4 `Basic` marker pre-filtered in SQL), `subscription_delivery` (dedicated jobs-shaped outbox with `attempts`/`next_attempt_at` schedule — claim bumps attempts; NOT a `cluster_jobs` kind), `ws_binding_tokens` (`DELETE … RETURNING` redeem-once, DB-clock expiry), `event_fanout` (`PgNotifyFanout`: pooled `pg_notify` publish, dedicated non-pooled LISTEN connection, capped-backoff reconnect + synthesized `Resync`, memoized per backend, stopped on drop, `ready()` for deterministic tests). Five `ResourceStorage` accessors (default-None). **All schedule/lease math on the DB clock** (client-clock compare flaked once against a container clock). 10-test T2 suite in `postgres_tests.rs` (`SUBSCRIPTION_OUTBOX_TEST_LOCK` for the cross-tenant claims); ran green 3×. | `persistence/src/core/{subscription_state,subscription_delivery,ws_binding_tokens,event_fanout}.rs`, `backends/postgres/subscription_*.rs`, `schema.rs` | `7ab9a074` |
| **PR 3.2 — engine seams (B2/B3/B4)**: `ClusterHandles` + `with_cluster_handles` (without them: byte-for-byte local behavior, full pre-existing suite unedited); shared `next_event_number` (store failure → local fallback with error — deliver-with-possible-dup beats dropping); `hydrate()` via non-handshaking `register_*_locally` helpers (topics→subs, stored runtime state overlays the resource status; live writes keep `event_number` but re-seed status + zero the failure streak — re-request intent wins); `generate_ws_token`/`redeem_ws_token` (fail-closed); `subscription_snapshot` overlay serving `$status`/`$events`/bind. Topic registry tenant-scoped. T1 `tests/cluster_engine.rs` (incl. the asserted-unsafe memory twin: duplicate eventNumbers) + T2 `tests/subscriptions_cluster_pg.rs` (new `postgres` marker feature, on in CI via `--all-features`). | `subscriptions/src/{engine,manager,topics,evaluator}`, `rest/src/handlers/{subscriptions,ws}.rs` | `0697ab79` |
| **PR 3.3 — fan-out delivery (B1) + durable outbox (B5) + rest wiring**: cluster mode splits per channel — websocket: persist bundle → lossless local delivery → `ws-event` envelope (origin skips its own; receivers load by key); push channels: fully outbox-driven from attempt zero, workers claim one attempt per cycle (`run_next_subscription_delivery` = the deterministic test seam), persisted backoff, shared thresholds, local-miss → full re-hydrate (topic + sub together — sub-only re-read fails topic validation); enqueue failure → inline fallback. Lifecycle/state envelopes + listener (vanished resource ⇒ delete; `Resync`/lag ⇒ re-hydrate). `HFS_SUBSCRIPTIONS_FANOUT` (unset → `pg-notify` under `HFS_CLUSTER`); `HFS_SUBSCRIPTIONS_ENABLED` parse hoisted onto `ServerConfig` (truthiness preserved); `build_app` wires the five seams (missing → warn + per-instance), starts listener before hydration, spawns workers. T1 + T2 grew ws-fan-out-over-real-LISTEN/NOTIFY, retry-requeue-then-deliver, lifecycle propagation incl. delete, B5 cross-instance claim. | `subscriptions/src/engine/mod.rs`, `rest/src/{config,lib}.rs` | `a2ea5e83` |
| **PR 3.4 — validator + docs**: two refusal rows under cluster+subscriptions — explicit `memory` fanout, non-Postgres primary (functional breakage, unlike C2's warn-only); feature-gated enabled resolution (a binary without the engine is exempt); T1 table = 9 collected violations worst-case. ch15 Phase 3 section (WS best-effort + gap detection, outbox at-least-once, **pgbouncer transaction-pooling breaks LISTEN** caveat, heartbeat still unwired), skills updated, design doc Class B `[amended]`. **Verified end-to-end, two binaries + one Postgres**: `$status` active via B for a subscription created on A; token minted on A bound on B; Encounter → A delivered to B's socket over LISTEN/NOTIFY; token re-use on A closed 1008; both refusal messages exact. | `hfs/src/{cluster,main}.rs`, `book/src/ch15-cluster-deployment.md`, skills, design doc | `7563a2c5` |
| **PR 3.5 — T3 smoke check 5 (the mandatory two-process B1 test)**: websocat client on **B**, matching Encounter written to **A**, event-notification frame asserted on B — plus lifecycle propagation (`$status` active via B), token mint-on-A/bind-on-B, and the sticky-session negative (consumed token rejected on A). Workflow: build gains `subscriptions`, `COMMON_ENV` += `HFS_SUBSCRIPTIONS_ENABLED=true` + explicit `HFS_SUBSCRIPTIONS_FANOUT=pg-notify` (`HFS_CLUSTER` still unset there — fs sink), pinned websocat install; the nginx WS plumbing was already in place. Check-5 section verified locally verbatim before dispatch. | `cluster-smoke.yml`, `run_external_cluster_smoke.sh` | `e9407064` |

**Phase 3 gate (strategy §8): MET 2026-07-15.** T3 **cluster-smoke green on
first dispatch**
([run 29459960052](https://github.com/HeliosSoftware/hfs/actions/runs/29459960052)
— all 5 checks incl. the new WS fan-out A→B and redeem-once negative); full
CI green on the Phase 3 head `e9407064`
([run 29459963541](https://github.com/HeliosSoftware/hfs/actions/runs/29459963541),
attempt 2 — the B2/B3/B4/B5 T2 suites ride `test-rust` via
`--all-features`); Redis Cluster Tests green on the same head. Locally the
same suites were green first (10 persistence + 8 T1 engine + 6 PG engine +
6 hfs validator + 2 rest config; the full pre-existing subscriptions suite
unedited and green), plus the two-binary end-to-end WS check.
**Infra note:** attempt 1's `Test Rust` failed twice with `No space left on
device` — specifically the self-hosted runner **github-agent5**
(`/Volumes/SN850X-1TB/_work/hfs/hfs/target` full); the rerun scheduled onto
agent6 and passed. Agent5 needs a manual `cargo clean`/target wipe or it
will keep poisoning whatever runs it picks up.

---

### ✅ Done — Phase 4: HTS coherency (C3) + bootstrap lock (D1) + composite outbox (E1) (2026-07-16)

| What | Where | Commit |
|------|-------|--------|
| **D1 — HTS bootstrap advisory lock**: `bootstrap_sync_postgres` wraps the whole directory-sync call in `schema::with_bootstrap_lock` (new dedicated key `HTS_BOOTSTRAP_LOCK` = "HTS_BOOT", distinct from schema-DDL's "HTS_SCHM"), unconditional — no `HFS_CLUSTER` gating (HTS reads no such flag). Per-file ledger check runs inside the lock, so a loser naturally skips what the winner already imported — no separate re-check needed. T2: `postgres_bootstrap_lock.rs`, a read-sleep-write race proving the lock serializes two independently constructed handles (verified red-without-fix / green-with-fix). | `crates/hts/src/main.rs`, `crates/hts/src/backends/postgres/schema.rs`, `crates/hts/tests/postgres_bootstrap_lock.rs` | `c76d0833` |
| **C3 — HTS terminology cache epoch**: new `terminology_epoch` single-row table (idempotent DDL in HTS's monolithic `SCHEMA` const, no versioned migration needed) + `EpochGuard` (`crates/hts/src/backends/postgres/epoch.rs`) shared between the `AppState`-layer handler caches and the `PostgresTerminologyBackend`-layer response caches, each independently tracking its own last-cleared epoch against one shared memoized fetch. Bump sits alongside the existing `clear_response_caches()` calls in `import_parsed`/`delete_normalized`. **Standalone HTS opt-in** (user decision this session): `HTS_TERMINOLOGY_CACHE_INVALIDATION = local \| epoch`, no `HFS_CLUSTER` coupling. T1: `EpochGuard` unit tests. T2: `postgres_epoch_cluster.rs` — two independent `AppState`/backend pairs over HTTP prove a pre-warmed stale lookup on B is invalidated by an update via A (both cache layers), plus the `local`-mode unsafe-contract negative. | `crates/hts/src/{main.rs,config.rs,state.rs,backends/postgres/{mod.rs,epoch.rs,schema.rs,code_system.rs,concept_map.rs,value_set.rs},operations/{expand.rs,lookup.rs,validate_code.rs}}`, `crates/hts/tests/postgres_epoch_cluster.rs` | `9c78e24c` |
| **C3 T3 smoke check**: two `hts` processes sharing the `hfs` instances' Postgres, `run_external_hts_cluster_smoke.sh` — import via A, pre-warm B's caches with the stale display, update via A, assert B self-corrects (not just a cold read, which would pass even without C3). | `.github/workflows/cluster-smoke.yml`, `crates/hfs/tests/cluster/run_external_hts_cluster_smoke.sh` | `ab6fd0a1` |
| **E1 — composite sync durable outbox**: new `CompositeSyncOutbox` trait (`crates/persistence/src/core/composite_sync_outbox.rs`, mirrors `SubscriptionDeliveryOutbox`'s shape) + Postgres impl (`backends/postgres/composite_sync_outbox.rs`, claim query cloned from `subscription_outbox.rs`) on migration **v18**; denormalized to **one row per `(event, backend_id)` pair**, `Create`/`Update`/`Delete` only — `SyncEvent::BulkSync` stays on the pre-existing in-memory channel. **Capability-based wiring, not `HFS_CLUSTER`-gated**: `CompositeStorage::new` wires the outbox unconditionally whenever the primary backend is Postgres (same reasoning as F5's unconditional fix) — no new env var, no `hfs`/`main.rs` changes needed for activation. `SyncManager` gained `run_next_composite_sync`/`spawn_composite_sync_workers`, a local `tokio::sync::Notify` same-process wake hint (no cross-instance fan-out — reasoned not to be needed, see design doc §5 E1 amendment), and a retry/backoff schedule mirroring `sync_event_to_backend`'s existing math. T1: state-machine tests against `InMemorySyncOutbox`. T2: `composite_sync_cluster_suite` in `postgres_tests.rs` — exclusivity, fencing+durability (reclaim after death), retry-schedule+isolation, and an end-to-end `CompositeStorage` wiring test (two independent composites, a genuinely separate throwaway "secondary" database so applies don't collide with the primary's own row) proving a write via A durably enqueues with no worker running on A, and B's `run_next_composite_sync` claims and applies it. | `crates/persistence/src/core/composite_sync_outbox.rs` (new), `crates/persistence/src/backends/postgres/composite_sync_outbox.rs` (new), `crates/persistence/src/backends/postgres/schema.rs` (v18), `crates/persistence/src/{core/storage.rs,composite/{storage.rs,sync.rs}}`, `crates/persistence/tests/postgres_tests.rs` | `6c322303` |
| **E1 warn-only validator**: `resolve_composite_sync_durability` (mirroring the warn-only `resolve_jwks_coordination`) — a composite secondary on a non-Postgres primary under `HFS_CLUSTER` keeps today's in-memory fallback and only logs a warning, not a refusal, contrast with subscriptions' hard refusal. | `crates/hfs/src/{cluster.rs,main.rs}` | `e7f16735` |
| Docs: ch15 matrix flipped, C3/E1 operator sections added; design doc `[amended 2026-07-16]` markers on §5 C3/D1/E1 + the `HFS_TERMINOLOGY_CACHE_INVALIDATION`→`HTS_TERMINOLOGY_CACHE_INVALIDATION` §6 correction; roadmap Phase 4 table. | `docs/cluster-*.md`, `book/src/ch15-cluster-deployment.md` | `f99eeec0` |

**Phase 4 gate (strategy §8): MET 2026-07-16.** T3 **cluster-smoke green on
first dispatch**
([run 29510624382](https://github.com/HeliosSoftware/hfs/actions/runs/29510624382)
— all checks incl. the new HTS C3 stage: health, warm-then-stale-then-fresh
lookup across two independent `hts` processes); full CI green on the Phase 4
head `864b8cad`
([run 29511136310](https://github.com/HeliosSoftware/hfs/actions/runs/29511136310)
— Test Rust incl. all new D1/C3/E1 T2 suites, Linting, Security Audit, Code
Coverage, Test Python, Test FHIRPath); Redis Cluster Tests green on the same
head. Locally the same suites were green first (1 D1 + 2 C3 + 9 E1 T2 tests,
the full pre-existing `helios-hts` 652 lib + 49+34 postgres integration and
`helios-persistence` 763 lib + 149 postgres_tests suites unedited and green,
run 3× to check for flakiness), plus a real two-process dry run of the C3
smoke script against a local Postgres before it ever touched CI.
**Infra note:** `Test Rust` failed twice on **github-agent5** (machine
`Stevens-Mini`) with a macOS dyld shared-cache mapping failure
(`SystemConfiguration.framework` failed to load, `SIGABRT`) — zero test
assertions failed either time; purely an OS-level runner fault (same runner
flagged for a disk-space issue during the Phase 3 push). Green on a later
retry once the runner recovered. **Gotcha (new): `[skip ci]` isn't scoped to
just the tip commit** — pushing a batch of commits where an EARLIER commit
in that batch carries `[skip ci]` suppresses `pull_request` CI for the whole
push, even when the tip commit's own message is clean. The Phase 0-3 fix
(one clean trailing commit) only works when that commit is pushed **on its
own**, not bundled with the skip-tagged commit in the same push.

**PR 4.4 — T3 nightly kill-9 recovery case (E1)**: `cluster-smoke.yml` gains
a dedicated `postgres-elasticsearch` instance pair (new ports, new
Elasticsearch container, nightly-gated) alongside the existing plain-Postgres
A/B pair — the main pair has no composite secondary, so no
`composite_sync_outbox` activity to crash-test. `run_nightly_e1_kill9_check.sh`
mirrors A1's shape adapted for E1's many-small-rows-not-one-big-job profile:
stop the pair's B, POST a 2000-Patient transaction Bundle via A (durably
enqueues one outbox row per resource without waiting on any Elasticsearch
apply), poll for a healthy queued/applying backlog (proving the single
worker hasn't already drained it — fail loudly and say "raise SEED_ROWS" if
it has, same discipline as A1), `kill -9` A, restart B, wait for the outbox
to fully drain, then verify via a `Patient?family=…&_summary=count` search
through B that Elasticsearch genuinely has every row — not just that the
outbox says so. **Passed on first local dry run** (500-row override against
real local Postgres + Elasticsearch containers: 459 rows still
queued/applying when the kill landed, 1 genuinely orphaned mid-`applying`
under A's lease, full recovery and convergence after restart). **Passed on
first real CI dispatch** (`-f nightly=true`, default `SEED_ROWS=2000`): the
"Nightly kill-9 recovery check (E1)" step completed in 71s with no failed
rows, and the whole run finished green end to end
([run 29524686368](https://github.com/HeliosSoftware/hfs/actions/runs/29524686368)).
| workflow + `run_nightly_e1_kill9_check.sh` | `99037703` |

**Not required for the Phase 4 gate proper** (the strategy's own gate
criteria list the nightly tier as an addendum, not a blocker) — implemented
as a fast-follow per the standing decision, and now confirmed green on real
CI self-hosted runners via `-f nightly=true` (scheduled runs will pick this
up automatically once this branch merges to `main`).

---

## What's next

Standing design decisions (do not re-litigate): **(1)** ~~`ExportJobController`
→ `#[async_trait]`~~ *done in 1.3*; **(2)** ~~JWKS coordination gets a
DB/advisory-lock variant alongside Redis so a DB-only cluster needs no Redis~~
*resolved in Phase 2: DB impl is the default, Redis resurrected as opt-in per
the 2026-07-15 session decision*; **(3)** ~~C3 uses a shared **epoch** table,
not LISTEN/NOTIFY~~ *landed in Phase 4 exactly as decided — see the design
doc §5 C3 amendment*. Decisions now baked into
code — Phase 1: bulk-export traits stay un-generalized (F3); the cluster job
store is Postgres-primary-only (other primaries warn and stay per-instance);
F5 is a CAS + bounded retry, not an unguarded atomic increment. Phase 2:
**watermark** freshness (not a pure staleness window) for coordinated
refreshes; C2 is **warn-only** under `HFS_CLUSTER` (per-instance JWKS is
functionally correct); coordination-layer failure falls back to direct IdP
fetch (auth availability > dedupe); the refresh store is server-global by
design (no tenant column). Phase 4: HTS's cache-invalidation toggle is a
**standalone `HTS_*` opt-in**, not coupled to `HFS_CLUSTER` (2026-07-16
session decision — HTS scales independently of the FHIR server); D1's
bootstrap lock is **unconditional**, no gating, mirroring D3; E1's outbox
wiring is **capability-based, not `HFS_CLUSTER`-gated** — `CompositeStorage`
wires it automatically whenever the primary is Postgres, same reasoning as
F5's unconditional fix, so there is no `hfs`/`main.rs` change and no new env
var for E1 at all.

### Phase 2 — auth hardening — ✅ COMPLETE 2026-07-15 (see the table above)

- C1 was obsoleted by #205 (jti subsystem removed; auth is stateless).
- C2 landed as PRs 2.1–2.5: `ClusterRefreshCache` (persistence, migration
  v16) + `JwksCoordination` (auth) + `StoreJwksCoordination` bridge (rest) +
  resurrected opt-in `RedisJwksCoordination` + hfs boot wiring, with the
  strategy §8 wiremock hit-counter suites at T2 on both backends.

### Phase 3 — subscriptions cluster delivery (#170) — ✅ COMPLETE 2026-07-15 (see the table above)

- Landed as PRs 3.1–3.5 on migration v17: the four persistence seams +
  `PgNotifyFanout`, the engine's `ClusterHandles` (hydration, shared
  counters, shared tokens), the channel-split delivery (WS fan-out +
  durable outbox), the validator refusal rows, and smoke check 5.
- Additional decisions baked in: fully outbox-driven push delivery
  (at-least-once, no inline first attempt), B2 as DB redeem-once (not
  stateless HMAC), lazy counter rows (updates never reset), tenant-scoped
  topic registry in both modes, WS-bundle store table
  (`subscription_notification_events`) so envelopes stay tiny.

### Phase 4 — HTS coherency (C3) + bootstrap lock (D1) + composite outbox (E1) — ✅ COMPLETE 2026-07-16 (see the table above)

- C3: one-row `terminology_epoch` table; every terminology write bumps it; an `EpochGuard` (memoized ~1s, 0 in tests) checks before serving from cache and calls the two existing clear seams (`hts/src/state.rs:280`, `backends/postgres/mod.rs:172`, incl. the `OnceLock` closure statics — turned out already covered, no separate handling needed). `HTS_TERMINOLOGY_CACHE_INVALIDATION = local | epoch`, standalone to the `hts` binary (not `HFS_*`, not `HFS_CLUSTER`-gated — session decision).
- D1: `pg_advisory_lock` around `bootstrap_sync` (dedicated key `HTS_BOOTSTRAP_LOCK` = "HTS_BOOT"), the per-file ledger check runs inside the lock so no separate re-check step was needed, unconditional (no gating).
- E1: `composite_sync_outbox` on migration **v18** (confirmed: v17 was Phase 3's head); `sync_asynchronous` durably enqueues one row per `(event, backend_id)` when the outbox is wired; a same-process `Notify` is the fast-path wake, not a cross-instance fan-out (reasoned unnecessary — see design doc §5 E1 amendment); workers claim with `FOR UPDATE SKIP LOCKED` + fencing. Wiring is capability-based (automatic on a Postgres primary), not `HFS_CLUSTER`-gated. T3 nightly kill-9 drain case landed (PR 4.4) — a dedicated `postgres-elasticsearch` pair + Elasticsearch container, gated `NIGHTLY`; dry-run verified, and green on its first real CI dispatch ([run 29524686368](https://github.com/HeliosSoftware/hfs/actions/runs/29524686368)).

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
   (`#[path]`-included from `postgres_tests.rs` and `mongodb_tests.rs`; do the
   same from other test binaries). Fresh-handle factory for Postgres is
   `create_backend()` in `postgres_tests.rs`; cluster-jobs suites must take
   `CLUSTER_JOBS_TEST_LOCK` (claiming is cross-tenant, so parallel tests steal
   each other's jobs). For deterministic worker execution in tests, drive one
   claim/run cycle via `run_next_sof_export_job` /
   `ReindexOperation::run_next_cluster_job` instead of spawning pollers.
5. T3: dispatch `cluster-smoke.yml` with `ref=feat/cluster-capable-state`
   (`gh workflow run cluster-smoke.yml --ref feat/cluster-capable-state`).
6. Local-toolchain gotcha: run clippy with **CI's exact flag set** (see
   `.github/workflows/ci.yml`, "Run clippy") — plain `-D warnings` reports
   `collapsible_if`/doc lints that CI deliberately allows; do not "fix" those.
7. Project hooks gate completion on `cargo fmt --all`, CI-style clippy, and an
   affected `cargo test`.
8. Gotcha: a **CONFLICTING PR silently stops `pull_request` CI** (GitHub can't
   build the merge ref, so pushes produce no runs and the PR shows "no
   checks"). If pushes stop getting checks, run
   `gh pr view 269 --json mergeable` before suspecting anything else — main
   moves fast on this repo; merge it in promptly.

## Update discipline

When a PR lands: move its row into the ✅ tables above with its commit hash,
tick the phase gate when the strategy §8 gate criteria are met, and update the
per-subsystem matrix in `book/src/ch15-cluster-deployment.md` (it promises to
track phase progress). Keep design-doc changes as inline `[amended <date>]`
markers.
