# Cluster-capable state — testing methodology & reasoning

**Status:** Working methodology (draft) for [discussion #223](https://github.com/HeliosSoftware/hfs/discussions/223)
**Branch:** `feat/cluster-capable-state` (off `main`)
**Companion to:** [`docs/cluster-testing-strategy.md`](./cluster-testing-strategy.md);
design mirror: [`docs/cluster-capable-state-design.md`](./cluster-capable-state-design.md);
status tracker: [`docs/cluster-capable-state-roadmap.md`](./cluster-capable-state-roadmap.md)
**Scope:** *how to use* the cluster testing strategy day to day, and the
reasoning behind it — the decision procedure for picking a test tier, how to
write the tests, how the work should be sequenced, and what a reviewer demands
before a cluster fix merges.
**Date:** 2026-07-08 (updated 2026-07-14 — calibration proof point in §5; new anti-pattern in §7)

The companion document, `cluster-testing-strategy.md`, is the **reference**: the
tier definitions, the full Class A–F tier map, the per-phase (0–4) plans. This
document is the **method**: given a piece of cluster-affected state in front of
you, *how do you decide what to test, in what order, and why.* Read the
reference for the *what*; read this for the *how* and the *why*.

---

## 1. The one idea everything follows from

**A cluster bug is a multi-observer bug, and our default tests have exactly one
observer.**

State created against one observer must be seen — or, for security, correctly
*not* seen — by another. Every test we run by default (in-process
`axum_test::TestServer`, a single testcontainer, one `./hfs &` in smoke) has a
single observer, so it is structurally blind to this entire class of defect. A
suite can be 100% green and the server can still lose a job on redeploy, replay
a one-time token, or serve a stale clinical answer.

Two consequences drive the whole method:

1. **For cluster correctness, the test is the only evidence the feature works at
   all.** You cannot eyeball a diff and conclude "this is cluster-safe." The
   second observer has to actually exist in a test. This is why the
   definition-of-done rule (§4) is non-negotiable.
2. **You rarely need two processes to get a second observer.** A cluster
   invariant ("a job created on A is visible on B") is a statement about a
   *shared-infrastructure protocol*, and a protocol is symmetric in the number
   of clients. Two trait handles over one backing store, in a single test
   process, drive the same protocol two real instances would. This is why T2
   (not T3) carries most of the load.

Everything below is the practical consequence of those two facts.

## 2. The boundary: what needs a cluster test at all

Before reaching for a tier, decide whether the state even crosses the cluster
boundary. From #223 §1:

- **Process-local ephemera** — connection pools, tokio runtimes, immutable
  config loaded from disk, per-request scratch — may live in memory and needs
  **no cluster test**. It never outlives a request or is observed by another
  actor.
- **Correctness-bearing shared state** — anything that outlives a single request
  and must be observed by a poller, a worker, or a second instance — must be
  externalized when clustered, and **must carry a cluster test**.

The tell: *"if a second instance existed, would it need to see this, or be
prevented from re-using it?"* If yes, it crosses the boundary. If the honest
answer is "each instance keeps its own and that's fine" (e.g. a per-node JWKS
public-key cache, a health monitor), it's ephemera — note *why* it's safe (the
"confirmed benign" list in #223 §5 is the model) and move on.

## 3. Decision procedure: which tier do I write?

Work top-down. Stop at the cheapest tier that actually reproduces the failure.

```
Is the behavior pure logic / config with no shared store?
  └─ yes → T1 (unit). Done. (fail-fast validation, token HMAC, state transitions)

Does correctness depend on a second observer sharing one backing store?
  └─ yes → T2 (two fresh handles, one testcontainer). This is the default and
           carries ~80% of the surface. Pick the applicable DoD rows (§4).
           Ask: is that enough?

Does the bug require a real OS process — a live socket, or a hard crash that
leaves torn/locked state a clean handle-drop can't produce?
  └─ yes → add T3 (two `hfs &`, shared backend, nginx front). ONLY:
             • WebSocket fan-out to a live connection (B1) — mandatory.
             • kill-9 recovery for the worst-blast-radius durable paths
               (A1 export, E1 composite sync) — nightly only.
           Everything else stays at T2.
```

Two rules that keep this honest:

- **Never promote a tier for "realism" alone.** T3 is expensive and
  timing-sensitive. If T2 reproduces the bug, a T3 version of the same assertion
  adds cost, not confidence. Reserve T3 for what T2 *structurally cannot* reach
  (a socket in another process; a real abort mid-statement).
- **Never demote below the tier that reproduces the bug.** A single-observer
  test that "covers" a cluster path is worse than no test — it manufactures
  false confidence in exactly the code most likely to be wrong.

## 4. Writing a T2 test — the mechanics that matter

T2 is the workhorse; get these right and the rest follows.

**Two *fresh* handles, never a cloned `Arc`.** Cloning `Arc<DashMap>` shares a
heap and proves nothing. Construct two independent backend objects that share
only the *backing store* (same Postgres URL / same Redis URL). That is the
faithful simulation of two `hfs` processes:

```
let store = shared_pg_container();          // one backing store (OnceCell)
let a = ClusterJobStore::connect(&store).await;   // handle A  ("instance A")
let b = ClusterJobStore::connect(&store).await;   // handle B  ("instance B")
```

**Assert the applicable rows of the DoD contract** (the suite selects only the
rows that apply — a cache has no lease; a token store has no durability):

| Row | Assertion | Applies to |
|-----|-----------|------------|
| visibility | create via `a(T1)` → observable via `b(T1)` | jobs, subs, counters |
| isolation | create via `a(T1)` → **not** observable via `b(T2)` (404/empty) | **everything** |
| exclusivity | `a` and `b` both claim/redeem one item → exactly one wins | jobs, tokens, leader-locks |
| fencing | stale-token write after the lease moved → `LeaseLost` | leased workers |
| durability | drop & re-create a handle → state survives | jobs, outboxes |
| invalidation | mutate via `a` → stale read via `b` is refreshed | caches |

**The tenant row is mandatory, always.** The isolation assertion
(wrong-tenant → 404/empty) is required on every subsystem, per the tenant-first
contract. Reuse the existing `is_cluster_shared` / secondary-tenant-context
helpers.

**Write it generic over the backend.** One suite against the trait, parametrized
over backends (`backend_test!` / `TestableBackend` shape). CI runs it on the
database backend; the Redis variant runs the *identical* assertions behind
`RUN_REDIS_CLUSTER_TESTS=1`.

**Keep the counted-in-coverage T2 tests deterministic.** Timing-sensitive
recovery assertions (wait-out-lease-then-steal, race-under-load) belong in the
nightly T3 tier, not in `cargo test`. A T2 test that flakes under llvm-cov
instrumentation gets tagged and `--skip`ed from the coverage command (the
`email_` mechanism), but the goal is to design it not to.

## 5. Sequencing: what to build first, and why not the alternatives

The instinct is to pick one of two orders. Both are wrong.

- **"All the testing infra first, then the features."** Impossible for ~90% of
  it: a T2 suite needs the trait it tests, the Redis CI wiring is moot until jti
  is shared, the kill-9 case needs the streaming path. Building tests for code
  that doesn't exist is churn.
- **"All the features, then retrofit the tests."** This is the exact trap #223
  exists to avoid. A cluster feature with no second-observer test ships green
  and *looks* correct while nothing has exercised two observers. For this class,
  a trailing test isn't late — it's absent evidence dressed up as done.

So the method is **interleaved, with one genuine prerequisite:**

1. **First, as a standalone increment: the T2 `cluster_harness` scaffold,
   calibrated against code that is already cluster-safe.** Build the
   two-fresh-handles helpers and the assertion functions once, then point the
   suite at the already-correct bulk-export job store — it must pass green
   immediately. This is worth isolating because:
   - it is the shared prerequisite every later phase's tests reuse;
   - calibrating against known-good code is a **zero-feature-risk** way to prove
     the harness itself is faithful. If two-handle-over-one-container has a snag,
     or the "already safe" store isn't, you learn it now — before any new code
     leans on the harness.

   This is no longer hypothetical. The **first dispatch** of the T3 calibration
   smoke (2026-07-14) — which asserted only "already cluster-safe" behavior —
   failed before reaching a single assertion: two instances cold-starting
   against one empty database raced Postgres schema init (`CREATE TABLE IF NOT
   EXISTS` is not catalog-safe) and one died. That was D3, a Class D bug missing
   from the #223 inventory; the "already safe" boundary was drawn wrong, and the
   calibration run is what found it. The fix (advisory lock) and its T2 suite
   landed the same day. The **second dispatch** then found a bug in the harness
   itself (§7, readiness-probe anti-pattern) — exactly the two failure classes
   calibration exists to flush out.

   One mechanical exception to "CI plumbing lands in the first phase that needs
   it": a `workflow_dispatch` workflow is only dispatchable once it exists on
   the **default branch**, so the T3 smoke *skeleton* (asserting only
   already-safe behavior) merges to `main` ahead of the phases and is then
   dispatched with `ref=<feature-branch>` to run that branch's workflow and
   script. That early landing is the T3 counterpart of calibrate-first, not a
   violation of the sequencing rule.
2. **Then each phase = feature + its DoD tests in the same change.** The
   cross-instance test lands with the feature, never after. Shared CI plumbing
   (the `cluster-smoke` workflow, the Redis path-filter/nightly job, the kill-9
   nightly) lands in the first phase that needs it, not up front.

Stated as a rule: **the harness scaffold precedes the features; every other test
accompanies its feature; nothing trails.** There is no clean "finish the testing
strategy, then start the document" seam — Phase 0 deliberately fuses the scaffold
with the first feature slice (the `HFS_CLUSTER` fail-fast validator), and that
fusion *is* the method.

## 6. Definition-of-done checklist (per PR)

A cluster fix is not mergeable until, in the same PR:

- [ ] The applicable DoD rows (§4) are asserted at T2 against the **database**
      backend and pass in `cargo test --all-features`.
- [ ] The **isolation (wrong-tenant)** row is present, regardless of subsystem.
- [ ] The two handles are independently constructed, **not** a cloned `Arc`.
- [ ] If the subsystem has a Redis backend, the **same** suite is wired to run
      under `RUN_REDIS_CLUSTER_TESTS=1`.
- [ ] If the fix touches WebSocket fan-out (B1), a **T3 cluster-smoke** case
      proves delivery from instance A to a socket on instance B.
- [ ] If the fix is a worst-blast-radius durable path (A1, E1), the **nightly
      kill-9** case exists and passes.
- [ ] Counted-in-coverage T2 tests are deterministic (no timing-sensitive
      recovery assertions in `cargo test`; those live in nightly T3).
- [ ] The `memory` backend's behavior is asserted as the **unsafe contract**
      (two `memory` handles do *not* see each other) where relevant — this is
      what `HFS_CLUSTER=true` fail-fast refuses.

If a row does not apply, the PR says *why* (mirroring #223's "confirmed benign"
discipline), rather than silently omitting it.

## 7. Anti-patterns (and the smell that gives each away)

- **Cloned-`Arc` "two instances."** Smell: the test shares one `DashMap`/one
  object. It proves in-process concurrency, not cross-instance. → Two fresh
  handles over one store.
- **Single-observer test claiming cluster-safety.** Smell: an
  `axum_test::TestServer` test in a PR whose description says "makes X
  cluster-safe." One router is one instance. → T2 with a real second handle.
- **Trailing test.** Smell: "tests to follow in a later PR" on a cluster fix. →
  The test *is* the evidence; it ships with the feature.
- **T3-for-realism.** Smell: a two-process test asserting something a two-handle
  test already proved. → Keep it at T2; reserve T3 for sockets and real crashes.
- **kill-9 everywhere.** Smell: a fault-injection matrix across subsystems that
  are only visibility/counter bugs. → kill-9 only for A1 and E1; T2
  drop-&-recreate is the durability row everywhere else.
- **Timing-fragile test in `cargo test`.** Smell: `sleep`-and-hope, tight
  absolute timeouts, flakes under coverage instrumentation. → Move the timing
  assertion to nightly T3; keep `cargo test` deterministic.
- **Untested "safe" claim.** Smell: "this is fine per-instance" with no note on
  *why*. → Record the reasoning (the confirmed-benign model), so a future reader
  can re-verify rather than re-audit.
- **Readiness probe through the system under test.** Smell: the "is the harness
  up" check traverses a component it isn't checking — e.g. probing nginx by
  proxying `/health` to upstreams that aren't started yet. The refused connects
  tripped nginx's failure accounting (`max_fails=1 fail_timeout=10s`) and every
  front request for the next 10 s got `no live upstreams` → 502, a harness bug
  indistinguishable from a product failure (cluster-smoke run 2, 2026-07-14). →
  Probe each layer directly (an nginx-local `location = /nginx-health`), and
  disable upstream failure accounting in harness configs (`max_fails=0`).
  Debugging corollary: runner and Docker-host clocks skew (~2 s observed), so
  never reconstruct cross-host failure ordering from log timestamps.

## 8. Extending this when a new subsystem appears

When new process-local state is added later (a new async operation, a new
cache), classify it with the #223 Class A–F lens before writing code:

- **A** in-memory job registry → unified job store + T2 (visibility, isolation,
  exclusivity, durability).
- **B** node-local connection/fan-out → shared pub/sub; T2 for the layer, T3 if a
  live socket is involved.
- **C** shared cache/replay with local invalidation → shared store or
  cross-instance invalidation; T2 (invalidation / exclusivity).
- **D** once-per-instance background task → leasing/leader-election; T2
  (exclusivity — runs once across N handles).
- **E** durability queue → durable outbox; T2 (durability) + nightly kill-9 if
  the blast radius warrants.
- **F** configuration caveat → T1 fail-fast + operator-doc note.

Then run the §3 decision procedure and honor the §6 checklist. The strategy is
meant to absorb new state without a redesign — the tiers and the DoD contract
are the stable interface; the Class A–F map just grows.

## 9. Where to look

- **Reference (tiers, tier map, per-phase plans):**
  [`docs/cluster-testing-strategy.md`](./cluster-testing-strategy.md).
- **Design (what/why of the feature work):**
  [`docs/cluster-capable-state-design.md`](./cluster-capable-state-design.md)
  (in-repo copy of [discussion #223](https://github.com/HeliosSoftware/hfs/discussions/223),
  maintained in-repo since 2026-07-14).
- **The reference architecture to imitate (already cluster-safe):** the
  bulk-data job store — `crates/persistence/src/core/bulk_export_worker.rs`
  (`ExportClaimStrategy`, `ExportWorkerStorage`, `ExportJobLease.fencing_token`)
  and its Postgres claim query (`FOR UPDATE SKIP LOCKED`) in
  `crates/persistence/src/backends/postgres/bulk_export.rs`.
- **The pluggable-backend precedent:** the jti cache — `crates/auth/src/jti/`
  (`JtiCache` trait, `InMemoryJtiCache`, `RedisJtiCache`), selected by
  `HFS_AUTH_JTI_BACKEND`.
- **The first T2 cluster test in the tree (template for the harness):**
  `postgres_integration_cluster_concurrent_cold_start_schema_init` in
  `crates/persistence/tests/postgres_tests.rs` — four fresh handles, one
  container, exclusivity + visibility + wrong-tenant isolation.
- **The T3 harness:** `.github/workflows/cluster-smoke.yml` +
  `crates/hfs/tests/cluster/run_external_cluster_smoke.sh` (dispatch with
  `ref=<branch>`).
