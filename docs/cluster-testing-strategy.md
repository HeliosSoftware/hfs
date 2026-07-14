# Cluster-capable state — testing strategy

**Status:** Testing plan (draft) for [discussion #223](https://github.com/HeliosSoftware/hfs/discussions/223)
**Branch:** `feat/cluster-capable-state` (off `main`)
**Companions:** [`docs/cluster-testing-methodology.md`](./cluster-testing-methodology.md) — *how to use* this strategy and the reasoning behind it;
[`docs/cluster-capable-state-design.md`](./cluster-capable-state-design.md) — the in-repo copy of the #223 design;
[`docs/cluster-capable-state-roadmap.md`](./cluster-capable-state-roadmap.md) — implementation status & what's next.
**Scope:** how we test every phase of the "Cluster-capable state" work — the
tiers of test, the harness contract, the per-subsystem definition-of-done, CI
integration, and the phase-by-phase test plan (Phases 0–4).
**Date:** 2026-07-08 (updated 2026-07-14 — T3 harness calibrated; D3 found & fixed, §10)

This document is the testing counterpart to the design in discussion #223. It
exists because the failure modes that work targets are, by construction,
invisible to the tests we run today. Read #223 first for the *what* and *why*;
this is the *how do we prove it*.

---

## 1. Why the current test suite cannot see any of these bugs

Every Class A–E defect in #223 is a **multi-observer** bug: state created against
one observer must be seen — or, for security, correctly *not* seen — by a
different observer. Our tests today have exactly one observer:

- HTTP behavior is validated **in-process** against a single composed Axum
  `Router` (`axum_test::TestServer`, `tower::ServiceExt::oneshot`) — one router
  is one instance.
- Backend behavior is validated against a **single** testcontainer
  (PostgreSQL / MongoDB / Elasticsearch / MinIO) via shared `OnceCell`
  containers.
- The only tests that exercise the real `hfs` binary over a network socket are
  the CI **smoke / Inferno** workflows (`bulk-export-smoke`,
  `subscriptions-smoke`, `inferno-*`), and each launches exactly **one**
  `./target/debug/hfs &`.

`cargo test --workspace --all-features` — the one CI test command — therefore
passes green while every cross-instance defect remains live. A test strategy for
this work is really a plan for **introducing a second observer**, as cheaply as
possible, at the lowest tier that still reproduces each bug.

## 2. The core insight — most cluster-correctness is a shared-backend *protocol*

You do **not** need two operating-system processes to prove most of this. A
cluster invariant like "a job created on instance A is visible to instance B" is
a statement about a **shared-infrastructure protocol**, and a protocol is
symmetric in the number of clients. If two independent trait handles — two
`worker_id`s, two `AppState`s, two `Arc<dyn JtiCache>` — point at **one** backing
store inside a single `#[tokio::test]`, that test drives the exact protocol two
real processes would:

- enqueue via handle A, poll / cancel / download via handle B → cross-instance
  visibility (A1);
- two workers race `claim_next` on one DB → exactly one wins
  (`FOR UPDATE SKIP LOCKED`);
- a zombie worker with a stale `fencing_token` mutates → `LeaseError::LeaseLost`
  (the fence);
- replay a `jti` on A → rejected on B (C1);
- import a CodeSystem on A → B's cache invalidated via the shared
  NOTIFY/epoch (C3).

The reference architecture already in the tree proves this is the right level:
the bulk-data job store (`crates/persistence/src/core/bulk_export_worker.rs` —
`ExportClaimStrategy`, `ExportWorkerStorage`, `ExportJobLease.fencing_token`) is
already cluster-safe, and its correctness is exactly these protocol properties.

**Consequence:** the large majority of the #223 surface is provable in a
single process against one testcontainer. Only WebSocket fan-out to a *live*
connection genuinely needs two processes.

## 3. The three test tiers

| Tier | Harness | Cost | Runs in |
|------|---------|------|---------|
| **T1 — unit** | `#[test]`, no container | ~ms | `cargo test`, every job |
| **T2 — shared-backend, multi-handle, one process** | 2+ freshly-constructed backend handles → **one** testcontainer | ~seconds (container amortized via `OnceCell`) | `cargo test --all-features` |
| **T3 — true multi-process E2E** | two `./hfs &` on separate ports sharing one backend, real HTTP/WS + round-robin front | ~tens of seconds | smoke workflow (`cluster-smoke`) |

### T1 — unit
Pure logic with no shared infrastructure: config fail-fast validation (Class F1–F4),
stateless HMAC/JWT binding tokens (B2 if we choose the stateless design), fence-token
arithmetic, job-state transitions against an in-memory fake `ClusterJobStore`.

### T2 — shared-backend, multi-handle, one process (the workhorse)
The tier that carries ~80% of the coverage. **Two independent handles, one
store, assert the protocol.** Critical harness rules:

1. **Two *fresh* handles, never a cloned `Arc`.** Cloning an `Arc<DashMap>`
   shares a heap and proves nothing. The handles must be independently
   constructed backend objects that only share the *backing store* (same
   Postgres URL / same Redis URL) — that is the faithful simulation of two
   `hfs` processes.
2. **Tenant dimension is mandatory.** Every suite includes a
   wrong-tenant → `404`/empty assertion (tenant-first contract), reusing the
   existing `is_cluster_shared` / secondary-tenant-context helpers from the
   persistence tests.
3. **Generic over backend.** Written once against the trait, parametrized over
   backends (the `backend_test!` / `TestableBackend` shape already in
   `crates/persistence/tests/common/`).
4. **`memory` backend proves the *unsafe* contract.** A `memory` two-handle test
   asserts the handles do **not** see each other — documenting exactly what
   `HFS_CLUSTER=true` refuses to run (Phase 0 fail-fast).

### T3 — true multi-process E2E
Only for what T2 structurally cannot reach: an event/socket that lives in one OS
process and must cross the network to another. Built by **extending the existing
smoke harness** — the workflows already build the binary, `docker run` the
backends, poll `/health`, and drive HTTP. T3 adds **one more `hfs &`** on a
second port sharing `HFS_DATABASE_URL`, with a trivial round-robin (nginx or a
shell `curl` alternator) in front. It asserts the two things only two processes
can show: (a) a job created via instance A is pollable/downloadable via B, and
(b) a WebSocket client on B receives an event triggered by a write to A.

## 4. The definition-of-done template (per subsystem)

Every Class A–E fix ships with a T2 cross-instance test that instantiates the
generic suite for its trait and asserts the failure mode is closed. The suite
contract:

```
given two fresh handles (H_a, H_b) over one shared store, and two tenants (T1, T2):
  visibility     : create via H_a(T1)  → observable via H_b(T1)
  isolation      : create via H_a(T1)  → NOT observable via H_b(T2)   // 404/empty
  exclusivity    : H_a and H_b both claim/redeem the same item → exactly one succeeds
  fencing        : stale-token write after lease moved → LeaseLost (where leased)
  durability     : drop & re-create a handle (simulated redeploy) → state survives (where durable)
  invalidation   : mutate via H_a → stale read via H_b is refreshed (where cached)
```

Not every row applies to every subsystem (a cache has no lease; a token store has
no durability guarantee) — the suite selects the applicable rows. A fix is not
done until its applicable rows are green against the **database** backend in CI.

## 5. Substrate decision and its effect on the matrix

Per the discussion, we **keep both substrates open** behind the env-var surface
(#223 §6, §9):

- **Database-only** (Postgres `LISTEN/NOTIFY` + tables for replay / fan-out /
  coherency epochs) is the **CI-tested default.** Every T2 test reuses the
  existing shared-Postgres `OnceCell` — **no new test container**, no CI change.
- **Redis** (`RedisJtiCache`, `JwksCoordinator`, Redis pub/sub) is a real,
  supported seam, so it gets the **identical** T2 assertions — but gated behind
  an env opt-in (`RUN_REDIS_CLUSTER_TESTS=1` + a Redis testcontainer), mirroring
  how `RUN_MINIO_S3_TESTS` / `RUN_MINIO_S3_ES_TESTS` gate MinIO today. This
  closes the current gap that Redis is the **only** distributed dependency in
  the tree with zero test coverage, without adding Redis to every CI run or
  making it a merge blocker.

Rule of thumb: **a cluster fix is CI-gated on the DB backend; the Redis backend
must have the same test written, runnable on demand.**

## 6. Per-subsystem → tier map

The full #223 inventory, annotated with the cheapest tier that reproduces the
bug and the DoD rows that apply. Locations abbreviated; see #223 §5 for the
exact file:line.

### Class A — in-memory job registries → unified job store

| Item | Location | Cheapest test | Tier | DoD rows |
|------|----------|---------------|------|----------|
| A1 SoF async export | `rest/src/export/in_memory.rs` `DashMap` | enqueue/poll/cancel/download across handles; race claim; per-instance semaphore → shared concurrency | **T2** (+T3 wiring) | visibility, isolation, exclusivity, durability |
| A2 reindex jobs | `persistence/src/search/reindex.rs` `RwLock<HashMap>` | status/cancel across handles; restart orphan | **T2** | visibility, isolation, durability |

### Class B — node-local registries & fan-out → shared pub/sub + shared state

| Item | Location | Cheapest test | Tier | DoD rows |
|------|----------|---------------|------|----------|
| B1 WS client registry | `subscriptions/.../ws_manager.rs` | pub/sub *layer* two-handle; live-socket delivery A→B | **T3** (T2 for the pub/sub layer) | visibility (layer) + E2E |
| B2 WS binding token | `subscriptions/.../ws_token.rs` | mint on A, redeem on B over shared KV; or pure-unit HMAC verify | **T1/T2** | exclusivity (single-use) |
| B3 subscription/topic registries | `subscriptions/src/manager,topics,engine` | startup reconciliation from DB; create-on-A visible-to-B | **T2** | visibility, isolation |
| B4 per-subscription counters | `manager/mod.rs` `eventNumber`, `consecutive_failures` | concurrent increments across handles → monotonic; scattered failures reach `off` | **T2** | exclusivity/atomicity |
| B5 delivery retry outbox | `engine/mod.rs` `dispatch_with_retry` | durable outbox survives handle drop; lease claim | **T2** | durability, exclusivity |

### Class C — shared caches / replay with local-only invalidation

| Item | Location | Cheapest test | Tier | DoD rows |
|------|----------|---------------|------|----------|
| C1 JWT `jti` replay | `auth/src/jti/` | replay on A → rejected on B (shared store); default `memory` → NOT rejected (unsafe contract) | **T2** | exclusivity (single-use), isolation |
| C2 JWKS refresh coordinator | `auth/src/jwks/coordinator.rs` (unwired) | N handles refresh under leader-lock → one IdP fetch | **T2** | exclusivity |
| C3 HTS terminology caches | `hts/src/state.rs`, `backends/*/mod.rs` | import via A → `$expand`/`$validate-code` via B refreshed | **T2** (+T3 real HTTP) | invalidation |

### Class D — once-per-instance background tasks → leasing / leader-election

| Item | Location | Cheapest test | Tier | DoD rows |
|------|----------|---------------|------|----------|
| D1 HTS bootstrap sync | `hts/src/main.rs` | N handles cold-start against shared DB under advisory lock → import runs once | **T2** | exclusivity |
| D2 bulk cleanup reapers | `hfs/src/main.rs` | already tolerable (idempotent deletes); optional single-owner lease | **T2** (opt) | exclusivity (opt) |
| D3 Postgres schema init — **fixed 2026-07-14** | `persistence/src/backends/postgres/schema.rs` | N fresh handles race init against an **empty** DB → all boot, exactly one `schema_version` row (advisory lock) | **T2** | exclusivity, visibility, isolation |

> **D3 provenance:** not in the original #223 inventory — found by the *first*
> dispatch of the T3 calibration harness (§7), reproduced at T2 (fails in ~2 s
> without the lock), and regression-locked there:
> `postgres_integration_cluster_concurrent_cold_start_schema_init`. The
> cheapest-reproducing-tier rule held: T3 discovered it only because the T2
> harness didn't exist yet.

### Class E — durability queues

| Item | Location | Cheapest test | Tier | DoD rows |
|------|----------|---------------|------|----------|
| E1 composite async sync | `persistence/src/composite/sync.rs` `mpsc::channel(1000)` | enqueue → drop worker (crash) → re-create → secondary write still lands | **T2** | durability |

### Class F — configuration caveats (documentation + one code fix)

| Item | Location | Cheapest test | Tier |
|------|----------|---------------|------|
| F1 SQLite can't cluster | `rest/src/config.rs` | `HFS_CLUSTER=true` + sqlite → boot refuses | **T1** |
| F2 bulk output local-fs | `hfs/src/main.rs` | `HFS_CLUSTER=true` + `local-fs` output → boot refuses | **T1** |
| F3 sidecar `bulk_export.db` under Mongo/S3 | `hfs/src/main.rs` `build_embedded_job_store` | documented; job-store-backend selection test | **T1** |
| F4 audit file sink node-local | `audit/src/sinks/file.rs` | `HFS_CLUSTER=true` + `file` audit → boot refuses | **T1** |
| F5 unconditional version-id increment race | `persistence/src/backends/postgres,mongodb/storage.rs` | two concurrent unconditional writers → no ETag collision / no lost history | **T2** |

**Tier tally:** T1 dominates Class F; T2 covers all of A, C, D, E and most of B;
**T3 is mandatory only for B1**, plus optional E2E wiring guards for A1/C1/C3.

## 7. CI integration

- **T1 + T2** land in the existing **`test-rust`** job (`cargo test --workspace
  --all-features`). The Postgres container is already provisioned via
  testcontainers on the self-hosted Docker host, so DB-backed cluster suites add
  **no new CI infrastructure** — only test code and container reuse. The
  leaked-container reaper (`cleanup-test-containers`) already covers any new
  labeled containers.
- **Redis opt-in suites** run behind `RUN_REDIS_CLUSTER_TESTS=1`. Off in the
  default `test-rust` run. **Enabled automatically on any PR that touches
  `crates/auth`** (a `paths:` filter — that is where `RedisJtiCache` /
  `JwksCoordinator` live, so an auth change gets Redis coverage immediately),
  **plus a nightly scheduled run** of the full Redis suite. This keeps the Redis
  seam continuously covered without gating unrelated PRs.
- **T3** is **`cluster-smoke.yml`** (landed on `main` via PR #256; dispatch with
  `ref=<branch>` to run that branch's workflow + smoke script), cloned from
  `bulk-export-smoke.yml`, differing only in launching a **second `hfs &`** on a
  separate port sharing `HFS_DATABASE_URL` and placing an **nginx sidecar** in
  front as the round-robin load balancer. nginx is chosen over a bash alternator
  so the same harness can also exercise sticky-session negatives for WebSockets
  (proving sticky routing is *not* sufficient, per #223 §7). It runs on the DB
  backend (Postgres primary), on the same matrix dimensions the smoke workflows
  already use where relevant. **Calibrated green 2026-07-14** (health A/B/front,
  round-robin via `X-Hfs-Upstream`, `PUT` on A → `GET` on B → `GET` via front);
  its first dispatch caught D3 (§6) and its second caught the
  readiness-probe-poisons-upstream harness bug (methodology §7 anti-patterns).
- **`coverage`** job (`cargo llvm-cov --workspace --features postgres,mongodb …
  -- --skip email_`, uploaded to Codecov) **includes the DB-backed T2 suites** —
  they already compile under its default-R4 feature set, so the metric honestly
  credits the cluster code. To keep it stable, timing-sensitive recovery
  assertions live in the nightly T3 tier (§10), not here; any T2 case that
  proves fragile under instrumentation is tagged and appended to the existing
  `--skip` list. Cluster suites must stay R4-safe and must not re-bloat lcov
  (the reason the job avoids `--all-features`). Redis suites are outside
  coverage automatically (not in its feature set).

## 8. Phase-by-phase test plan

Each phase is independently shippable and leaves the tree green (#223 §8). For
each: the code under test, the tests that gate it, and the tier.

### Phase 0 — framing & guardrails
**Code:** `HFS_CLUSTER` master switch with fail-fast validation (reject SQLite
primary, `memory` `jti`, `local-fs` bulk output, `file` audit); `HFS_JOB_STORE_BACKEND`
selector plumbed (not yet wired to a DB impl); the "Running HFS in a cluster"
operator doc.
**Tests:**
- **T1** — a validation table: for each `(HFS_CLUSTER, backend, jti, output,
  audit)` combination, assert boots vs refuses-with-the-specific-error. Pure
  `#[test]`, no container.
- **Scaffold (T2 groundwork)** — build the `cluster_harness` helper
  (`two_handles`, `assert_visible_across`, `assert_isolated`,
  `assert_claim_exclusive`, `assert_fence_rejected`, `assert_wrong_tenant_404`)
  in `persistence/tests/common/`, and **calibrate it against the already-safe
  bulk-export job store** — that suite must pass green immediately, proving the
  harness is faithful before any new subsystem depends on it.
**Gate:** validation table green; calibration suite green.

> **Progress (2026-07-14):** the T3 side of the calibration is done ahead of
> this phase — `cluster-smoke.yml` merged (PR #256), dispatched, and green
> end-to-end after catching D3 (§6) on its first run. The T2
> `cluster_harness` scaffold and its bulk-export calibration remain the Phase 0
> deliverable; D3's suite in `postgres_tests.rs` is the first two-fresh-handle
> test in the tree and a working template for it.

### Phase 1 — unified job store + SoF export (#169)
**Code:** `ClusterJobStore` (generalized from `BulkExportJobStore`); DB-backed
`ExportJobController` over it; wire `HFS_JOB_STORE_BACKEND` / `HFS_EXPORT_CONTROLLER`;
fold reindex (A2) onto the same `jobs` table. **Also lands the F5 fix**
(atomic version-id increment) — see below.

> **Scope decision (F3):** bulk export/submit do **not** migrate onto the
> unified `jobs` table in this phase. They are already cluster-safe on a
> Postgres primary and stay as-is; Mongo/S3-primary clusters are **documented as
> single-instance for bulk** (Phase 0 operator doc). `ClusterJobStore` is built
> generically so a future migration is possible, but no data-migration test is
> in scope here.

**Tests:**
- **T1** — job-state-machine transitions and the reaper against an **in-memory
  fake** `ClusterJobStore` (logic, no container).
- **T2 (DoD, A1)** — instantiate the suite for `ClusterJobStore` over Postgres:
  visibility, isolation, exclusivity (race two `claim_next`), fencing
  (stale-token → `LeaseLost`), durability (drop & re-create the store handle →
  job survives). Include the SoF export path end-to-end through
  `ExportJobController`.
- **T2 (DoD, A2)** — the same suite for reindex jobs on the shared table.
- **T2 (F5)** — two concurrent unconditional writers against one Postgres/Mongo
  store → no duplicate `version_id`, no ETag collision, no lost history version.
  (Folded forward from #223 §8 "continuous": the race exists single-instance
  too, the fix — `SET version = version+1 … RETURNING` or required `If-Match` —
  is cheap and independent of the cluster switch, and it's the first real
  product-code use of the two-handle harness built in Phase 0.)
- **T3 (wiring)** — `cluster-smoke`: `POST` a SoF `$export` to instance A, poll
  the status URL through the nginx front (lands on B), download the manifest via
  B.
- **T3 (nightly, kill-9, A1)** — start a streaming export on instance A,
  `kill -9` A mid-stream, assert instance B claims the orphaned lease after
  expiry and drives the job to completion (no lost/corrupt/torn job, no lock
  held by the corpse). Nightly cluster-smoke only.
**Gate:** A1 + A2 + F5 suites green on DB; cluster-smoke export round-trip green.
(Nightly: kill-9 recovery green.)

### Phase 2 — auth hardening
**Code:** make `jti` shared-mandatory under `HFS_CLUSTER` (fail-closed if
`memory`); wire the existing `JwksCoordinator` (leader-lock + shared key store).
**Tests:**
- **T1** — fail-closed check: `HFS_CLUSTER=true` + `HFS_AUTH_JTI_BACKEND=memory`
  → boot refuses.
- **T2 (DoD, C1)** — replay a one-time assertion on handle A → **rejected** on
  handle B (shared backend); assert the `memory` backend does **not** reject
  across handles (the unsafe contract). Include the existing secondary bug: a
  token honored past its `expires_at` under the flat 1 h TTL. Runs on the DB
  backend in CI; the **identical** suite runs against `RedisJtiCache` under
  `RUN_REDIS_CLUSTER_TESTS=1`.
- **T2 (DoD, C2)** — N handles trigger JWKS refresh under the coordinator lock →
  exactly one upstream fetch (assert via a `wiremock` IdP hit counter).
**Gate:** C1 + C2 suites green on DB; Redis variant green on demand.

### Phase 3 — subscriptions cluster delivery (#170)
**Code:** DB-backed subscription/topic load + startup reconciliation (B3);
shared pub/sub fan-out (B1); shared or stateless WS binding tokens (B2); shared
counters (B4); durable delivery outbox (B5).
**Tests:**
- **T2 (DoD, B3)** — a `Subscription`/`SubscriptionTopic` created via handle A is
  loaded and matchable via handle B after reconciliation; an instance that never
  saw the topic write still resolves it from the DB (no `TopicNotFound`).
- **T2 (DoD, B4)** — concurrent `eventNumber` increments across handles are
  monotonic and gap-free; failures scattered across handles still accumulate to
  the `off` threshold.
- **T2 (DoD, B5)** — a delivery enqueued then interrupted (drop the worker) is
  re-claimed and delivered after a fresh handle starts (outbox lease pattern).
- **T1/T2 (B2)** — if stateless: HMAC/JWT token verify is pure-unit; if shared
  KV: mint on A, redeem-once on B, second redeem fails.
- **T3 (DoD, B1 — the mandatory two-process test)** — `cluster-smoke`: open a
  WebSocket to instance B, `POST` a matching resource to instance A, assert B's
  socket receives the notification. This is the one bug no single-process tier
  can catch.
**Gate:** B3/B4/B5/B2 suites green on DB; **cluster-smoke WS fan-out A→B green.**

### Phase 4 — HTS cache coherency (C3) + bootstrap lock (D1) + composite outbox (E1)
**Code:** cross-instance terminology-cache invalidation (PG `LISTEN/NOTIFY` /
shared epoch / TTL); `pg_advisory_lock` around HTS bootstrap; durable composite
sync outbox.
**Tests:**
- **T2 (DoD, C3)** — import a CodeSystem/ValueSet/ConceptMap via handle A →
  `$expand` / `$validate-code` / `$lookup` / `$translate` / `$subsumes` via
  handle B returns the **updated** result (invalidation propagated). Covers the
  `AppState` caches and the backend response caches, including the
  `OnceLock` closure-count statics.
- **T3 (wiring, C3)** — `cluster-smoke`: import on A, `$validate-code` on B over
  HTTP returns fresh.
- **T2 (DoD, D1)** — N handles cold-start against a shared DB with a bootstrap
  dir → the heavy import runs **once** (assert a single import-ledger row / one
  import invocation), not N times.
- **T2 (DoD, E1)** — enqueue a secondary-backend propagation, drop the async
  worker mid-flight (simulated crash), re-create → the secondary write still
  lands (durable outbox), and search does not diverge.
- **T3 (nightly, kill-9, E1)** — write to instance A so events are buffered in
  the async worker, `kill -9` A, assert the durable outbox is drained by another
  instance and the secondary backend converges (no silently-lost propagation).
  Nightly cluster-smoke only.

  *(F5 — the version-id race — is fixed and tested in Phase 1, not here.)*
**Gate:** C3, D1, E1 suites green on DB; C3 cluster-smoke green.
(Nightly: kill-9 outbox-drain green.)

## 9. What this plan deliberately does *not* test (and why that's OK)

- **True N>2 topologies.** Two handles / two processes reproduce every listed
  bug; correctness of a protocol under 2 observers generalizes to N. We do not
  spin up 3+ instances.
- **Network partition / split-brain.** Out of scope for #223 (no consensus
  system is introduced — the shared DB/Redis *is* the coordination point). Not
  tested here.
- **Redis in the default CI run.** Covered on-demand (§5); DB-only is the
  CI-gated substrate.
- **SQLite in a cluster.** Explicitly rejected at boot (F1); the only "test" is
  that the fail-fast fires.

## 10. Decisions and remaining open questions

### Resolved (2026-07-08 discussion)

- **T3 front → nginx sidecar.** The two-instance smoke uses a real nginx reverse
  proxy so it can also exercise sticky-session negatives for WebSockets, not
  just round-robin (§7).
- **Redis-suite cadence → path filter on `crates/auth` + nightly.** Redis
  suites run automatically on PRs touching `crates/auth` and on a nightly
  scheduled job (§7).
- **F5 (version-id race) → folded into Phase 1.** Fixed and tested early rather
  than waiting for Phase 4; it's a live single-instance bug and the first
  product-code use of the two-handle harness (§8, Phase 1).
- **Bulk export/submit (F3) → stay Postgres-primary-only.** Not migrated onto
  the unified `jobs` table; Mongo/S3-primary bulk documented as single-instance
  (§8, Phase 1). `ClusterJobStore` is still built generically to keep a future
  migration open.

- **Fault injection → T2 drop-&-recreate everywhere + one nightly kill-9 T3
  each for A1 and E1.** The T2 drop-&-recreate handle simulation stays the
  mandatory durability DoD row for every durable subsystem (deterministic, in
  `cargo test`, proves the durability *contract*). On top of that, a **single
  targeted `kill -9`-mid-operation T3 case** is added for the two subsystems
  with the worst blast radius — A1 (SoF export: lost/corrupt job) and E1
  (composite sync: silent search divergence). Both have naturally hittable crash
  windows (a streaming export; the 100 ms-batched async worker), so **no
  test-only crash hook in product code is required**. These run in the
  **nightly** cluster-smoke only (they wait out a lease before asserting the
  steal → slow, timing-sensitive). Their unique value over T2 is catching
  non-transactional torn state and locks not released on a hard crash (§8,
  Phases 1 & 4).
- **Coverage accounting → include the T2 suites, push fragile timing to T3.**
  The DB-backed T2 suites feed the `coverage` job's `llvm-cov` (they already
  compile under its `--features postgres,mongodb` default-R4 build), so the
  metric honestly credits the cluster code this effort adds. To keep that job
  stable, the timing-sensitive recovery/lease-steal assertions live in the Q1
  nightly T3 tier (outside coverage), leaving the counted T2 suites
  deterministic (visibility, isolation, single-claim exclusivity, invalidation).
  Any T2 case that proves fragile under instrumentation is tagged and `--skip`ed
  in the coverage command only, reusing the existing `-- --skip email_`
  mechanism. Redis suites stay out of coverage automatically (not in its feature
  set) — fine, they're the opt-in seam.

### Findings from the T3 calibration (2026-07-14)

The first three dispatches of `cluster-smoke.yml` against
`feat/cluster-capable-state` validated the harness and produced two findings:

- **D3 — Postgres schema-init race (product bug, fixed).** Run 1: both
  instances cold-started against one empty Postgres; instance A died on
  `Failed to create schema_version table` (`pg_type` duplicate-key from racing
  `CREATE TABLE IF NOT EXISTS`). A Class D bug missing from the #223 inventory
  — added there and to §6. Fixed with `pg_advisory_lock`; T2 suite proves
  red-without/green-with.
- **nginx readiness probe poisoned upstream state (harness bug, fixed).**
  Run 2: the "is nginx up" probe proxied `/health` before the instances
  existed; nginx's default failure accounting (`max_fails=1 fail_timeout=10s`)
  then blacklisted both upstreams for 10 s and the smoke script's front check
  got `no live upstreams` → 502. Fixed with an nginx-local `/nginx-health`
  probe target + `max_fails=0` on the upstreams. Generalized as the
  "readiness probe through the system under test" anti-pattern (methodology §7).
- Run 3: all checks green — the harness is calibrated and ready to carry the
  Phase 1–4 assertions.

Net: the calibrate-first principle paid for itself before Phase 0 formally
started — one real Class D bug and one harness bug were found and fixed with
zero feature code at risk.

### Still open

*(none — all discussion items resolved as of 2026-07-08.)*
