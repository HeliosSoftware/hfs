# Cluster-capable state — discussion & design

**Status:** design — in-repo copy of [discussion #223](https://github.com/HeliosSoftware/hfs/discussions/223)
(authored by @smunini, 2026-07-08; mirrored 2026-07-14). Maintained here from
now on — in-repo amendments are marked **[amended <date>]** inline.
**Companions:** [`cluster-testing-strategy.md`](./cluster-testing-strategy.md) (how we prove it),
[`cluster-testing-methodology.md`](./cluster-testing-methodology.md) (how to apply the strategy),
[`cluster-capable-state-roadmap.md`](./cluster-capable-state-roadmap.md) (implementation status & what's next).
**Source:** [PR #180 review comment](https://github.com/HeliosSoftware/hfs/pull/180#issuecomment-4849199024),
which asks for a document describing the areas of `hfs` that must be modified —
behind an environment-variable config switch — to use a **unified,
cluster-capable job store**, plus operator documentation for running `hfs` in a
cluster. Relates to [#169](https://github.com/HeliosSoftware/hfs/issues/169),
[#170](https://github.com/HeliosSoftware/hfs/issues/170),
[#150](https://github.com/HeliosSoftware/hfs/pull/150), and the ROADMAP
*"Clustered / multi-instance deployment"* item.

This draft is the output of an exhaustive sweep of the workspace for state that
lives only in one process's memory and would break, diverge, or silently produce
wrong results if `hfs` ran as multiple instances behind a load balancer. It
covers the three areas already flagged (SoF export jobs, WebSocket Subscription
delivery, the JWT `jti` cache) **plus** everything else the sweep surfaced.

---

## 1. What "cluster-capable" means here

A single `hfs` process today holds a lot of correctness-bearing state on its own
heap: async job registries, WebSocket connections, replay caches, terminology
caches, and background reaper tasks. Behind a load balancer with ≥ 2 instances,
each request may land on a *different* instance than the one that created the
associated state. Four failure modes recur:

1. **Cross-instance invisibility** — a poll / cancel / download for something
   created on instance A returns 404 on instance B (SoF export jobs, reindex
   jobs, WebSocket binding tokens).
2. **Restart / redeploy loss** — a rolling deploy silently drops in-flight state
   with no durable record (all in-memory job registries, delivery retries).
3. **Silent divergence** — instances serve inconsistent answers because a cache
   was invalidated on one node only (HTS terminology caches), or a counter is
   maintained per node (Subscription `eventNumber`, failure counters).
4. **Security regression** — a one-time credential replayed against a second
   instance is accepted because the first instance's replay cache is not shared
   (JWT `jti`).

The boundary we are drawing: **process-local ephemera** (connection pools, tokio
runtimes, immutable config loaded from disk, per-request scratch) may stay in
memory. **Anything that outlives a single request and must be observed by
another actor** — a poller, a worker, a second instance — must be externalized
to shared infrastructure when clustered.

## 2. The good news — HFS already has two working precedents

We are not starting from zero. Two subsystems are already cluster-correct, and
they establish the two shapes every fix below should take.

### 2a. DB-backed job store with lease + fencing tokens (Bulk Data)

Bulk Data `$export` and `$bulk-submit` are fully cluster-safe. Their job state
(status, tenant ownership, per-type progress cursors, output-file rows) lives in
the **primary database**, and workers compete for jobs through an atomic
lease-and-fence protocol:

- `trait BulkExportJobStore` (`crates/persistence/src/core/bulk_export_worker.rs:215`)
  and `trait BulkSubmitJobStore` (`crates/persistence/src/core/bulk_submit_worker.rs:300`)
  compose job-state storage + a claim strategy + fenced worker storage.
- `ExportClaimStrategy::claim_next` (`bulk_export_worker.rs:124`) atomically
  claims one job (`SELECT … FOR UPDATE SKIP LOCKED` on Postgres; a process-local
  mutex on SQLite). Every worker mutation carries a `worker_id` + monotonic
  `fencing_token`; a zero-row guarded write returns `LeaseError::LeaseLost` and
  the worker aborts, so a zombie can never mutate a job another worker now owns.
- Workers are spawned per-instance (`spawn_export_workers`,
  `crates/hfs/src/main.rs:1035`); every replica runs a pool that competes for
  DB-leased jobs. `HFS_BULK_EXPORT_DISABLE_LOCAL_WORKER` turns a node API-only.

**This lease/fencing store is the reference architecture for the unified job
store in §4.**

### 2b. Pluggable shared backend selected by env var (auth `jti`)

The `jti` replay cache is already a trait with swappable backends chosen at
startup:

- `trait JtiCache` (`crates/auth/src/jti/mod.rs:15`) with `InMemoryJtiCache`
  (`memory.rs:14`), `RedisJtiCache` (`redis.rs:10`, atomic `SET NX EX`), and
  `DisabledJtiCache`.
- Selected by `HFS_AUTH_JTI_BACKEND` (`crates/auth/src/config.rs:73`, default
  `"memory"`), Redis URL from `HFS_AUTH_REDIS_URL`.

The trait boundary and env-var selection are exactly the pattern §4 generalizes.
The gap (see §5, class C) is only that the safe backend is opt-in.

A third, **already-written-but-never-wired** piece exists too: `JwksCoordinator`
(`crates/auth/src/jwks/coordinator.rs:11`) — a Redis leader-lock
(`hfs:jwks:refresh_lock`) plus shared key store for cluster JWKS refresh. It has
no call sites. *[amended 2026-07-15: #205 deleted it (never constructed) along
with the whole `redis` feature; Phase 2 replaced it with the `JwksCoordination`
trait — Postgres advisory-lock impl over `cluster_refresh_cache` (default under
`HFS_CLUSTER`) plus a resurrected, opt-in `RedisJwksCoordination` — see §5 C2.]*

## 3. Shared substrate — what "unified" backs onto

Three externalization primitives cover every finding. The proposal does **not**
introduce three separate dependencies; it selects among what a deployment
already runs:

| Primitive | Purpose | Backing options already present in HFS |
|-----------|---------|----------------------------------------|
| **Shared job store** | durable job lifecycle + lease/fencing | primary DB (Postgres; Mongo where transactional) — §2a |
| **Shared KV/replay cache** | replay tokens, coordination locks, coherency epochs | Redis (`redis` feature) — §2b; or a DB table |
| **Shared pub/sub fan-out** | deliver an event that originated on any node to sockets/caches on every node | Redis pub/sub, Postgres `LISTEN/NOTIFY`, or a message bus |

The governing principle: **when clustered, the primary backend must itself be a
shared database** (Postgres / Elasticsearch / MongoDB). SQLite — the zero-config
default (`HFS_STORAGE_BACKEND=sqlite`, `crates/rest/src/config.rs:739`) — is a
single-writer local file and **cannot be clustered at all**. Cluster guidance
starts there.

## 4. The unified cluster-capable job store

Generalize §2a into one `ClusterJobStore` seam that the in-memory async
subsystems plug into, selected by a single env var.

**Trait shape** (mirrors `BulkExportJobStore`, kind-agnostic):

```
trait ClusterJobStore {
    async fn enqueue(&self, tenant, kind: JobKind, payload: Value) -> JobId;
    async fn claim_next(&self, worker_id, lease) -> Option<ClaimedJob>;   // FOR UPDATE SKIP LOCKED
    async fn heartbeat(&self, lease) -> Result<Expiry, LeaseLost>;        // fencing-token guarded
    async fn get_status(&self, tenant, job_id) -> Option<JobStatus>;      // tenant-checked → 404
    async fn cancel(&self, tenant, job_id) -> bool;                       // cooperative, DB-visible
    async fn list_expired(&self, now) -> Vec<JobId>;                      // for a single reaper
}
```

- **One `jobs` table** in the shared DB, discriminated by `JobKind`
  (`bulk-export`, `bulk-submit`, `sof-export`, `reindex`), so the four
  subsystems share migrations, the claim query, lease/fence logic, tenant
  scoping, and the cleanup reaper.
- **Backend selection** via env var, following the `jti` precedent:
  `HFS_JOB_STORE_BACKEND = memory` (default; in-process `DashMap`, single
  instance) `| database` (shared, cluster-capable). A convenience master switch
  `HFS_CLUSTER=true` flips this and the other cluster-safe defaults
  (§6) at once and **fails fast** if the primary backend is SQLite.
- **Existing seams to retarget:** the SoF path already declares its trait
  (`ExportJobController`, `crates/rest/src/export/controller.rs:196`) and a
  reserved-but-unread selector (`HFS_EXPORT_CONTROLLER`,
  `crates/rest/src/config.rs:805`). The database backend implements
  `ExportJobController` over `ClusterJobStore` and is wired where
  `InMemoryController` is unconditionally constructed today
  (`crates/rest/src/lib.rs:517`). Bulk export/submit already *are* this pattern
  and can migrate onto the shared table opportunistically or stay as-is.

## 5. Inventory of affected areas

Grouped by the fix class they need. Each item: location · what breaks · severity.

### Class A — in-memory async job registries → move to the unified job store (§4)

- **A1 · SoF async export ([#169]) — CLUSTER-BREAKING.**
  `InMemoryController` (`crates/rest/src/export/in_memory.rs:55`) holds
  `jobs: DashMap<JobId, JobStatus>` (`:56`) and `job_tenants: DashMap` (`:59`);
  `submit()` runs each job in a detached `tokio::spawn` (`:168`) under a
  **per-instance** `Semaphore` (so N instances run N× the intended concurrency);
  a per-instance TTL reaper (`spawn_cleanup:117` / `reap_expired:323`) sees only
  its own jobs. The `202`/status URL, cancellation, download, and the completion
  manifest (`JobStatus::Completed { files, .. }`) all live only in one heap. A
  poll/cancel/download on another instance → 404; restart loses everything.
  **The single clear code defect; the whole `$…-export` async surface is
  single-instance today.** Severity: **functional breakage / result loss.**
- **A2 · Persistence reindex jobs — degradation.**
  `ReindexManager` (`crates/persistence/src/search/reindex.rs:363`) keeps
  `jobs: RwLock<HashMap<String, ReindexProgress>>` + `cancel_channels`; `start()`
  mints a UUID and `tokio::spawn`s the work (`:406`). Status poll / cancel on
  another instance sees no such job; restart orphans the reindex. Admin-triggered,
  no corruption. Severity: **degradation.**

### Class B — node-local connection registries & fan-out → shared pub/sub + shared state (Subscriptions, [#170])

*[amended 2026-07-15: Phase 3 landed B1–B5 on migration v17. Resolved
shapes: the fan-out is Postgres LISTEN/NOTIFY (`EventFanout` /
`PgNotifyFanout`, dedicated non-pooled listen connection, envelopes only —
websocket bundles persist in a `subscription_notification_events` table and
receivers rehydrate by `(tenant, sub, eventNumber)`); B2 chose the shared-KV
design (DB `DELETE … RETURNING` redeem-once), not stateless HMAC; B5 is
fully outbox-driven from attempt zero (at-least-once), not
inline-then-outbox; B4 counter rows are created lazily by upsert-increment,
so a Subscription update can NOT reset shared counters (deliberately not
replicating the in-memory register's reset); and the topic registry became
tenant-scoped (`(tenant_id, canonical_url)`) in both modes, closing a
pre-existing cross-tenant topic-visibility leak. `HFS_SUBSCRIPTIONS_FANOUT
= memory | pg-notify`; explicit `memory` (or a non-Postgres primary) with
subscriptions enabled is a refusal under `HFS_CLUSTER=true`. Original
inventory kept below for the record.]*

Systemic: subscription reaction is per-instance, fire-and-forget, in-memory.
`emit_subscription_event` (`crates/rest/src/handlers/subscription_event.rs:47`)
`tokio::spawn`s `on_resource_event` on whichever instance served the write,
against *that* process's registries. Subscriptions/topics are loaded **only
reactively** (nothing reads them from the DB at startup). So a `Subscription`
created on A produces **zero** notifications for a matching write served by B,
and all in-memory state vanishes on restart.

- **B1 · WebSocket client registry — CRITICAL.**
  `WebSocketManager.clients` (`crates/subscriptions/src/channels/ws_manager.rs:19`),
  a `DashMap<(tenant, subscription_id), Vec<WsClientSender>>` holding the
  mpsc-sender half of a live socket. Only the instance terminating the socket can
  deliver; `WebSocketChannel::dispatch` (`channels/websocket.rs:45`) hits the
  *local* manager and returns `Success` even with 0 local clients, so loss is
  invisible. Sockets are inherently node-local → needs a pub/sub fan-out where
  the event is broadcast to all instances and each delivers to its own sockets.
- **B2 · WebSocket binding-token store — HIGH.**
  `WsBindingTokenManager.tokens` (`channels/ws_token.rs:24`), single-use ~30 s
  tokens from `$get-ws-binding-token`. The token is minted on one connection and
  redeemed on a *separate* WS-upgrade connection the LB may route elsewhere →
  every cluster WS bind fails without sticky routing. Fix: shared KV (Redis TTL)
  or a signed/stateless (HMAC/JWT) token.
- **B3 · Subscription & topic registries — CRITICAL/HIGH.**
  `SubscriptionManager.subscriptions` (`crates/subscriptions/src/manager/mod.rs:137`),
  `InMemoryTopicRegistry.topics` (`crates/subscriptions/src/topics/mod.rs:94`),
  and `SubscriptionEngine.topic_resource_index` (`engine/mod.rs:37`) are all
  in-memory with no startup load. An instance that never saw the topic write
  rejects `register` with `TopicNotFound`. Fix: DB-backed load (the resources
  already persist) + startup reconciliation.
- **B4 · Per-subscription counters — HIGH/MEDIUM.**
  `events_since_start` → `Subscription.status.notificationEvent[].eventNumber`
  (`manager/mod.rs:124`, built in `notification/builder.rs:172`) and
  `consecutive_failures` driving `error`/`off` transitions (`manager/mod.rs:127`,
  `engine/mod.rs:854`) are per-instance. Result: non-monotonic/duplicated
  `eventNumber` across the cluster (breaks subscriber gap-detection); a dead
  endpoint whose failures scatter across nodes never reaches the `off` threshold.
  Fix: shared atomic counters (DB row / Redis `INCR`).
- **B5 · Delivery retry loop — MEDIUM.**
  `dispatch_with_retry` (`engine/mod.rs:722`) keeps the retry "queue" on a
  fire-and-forget task's stack (backoff up to 60 s × 10). A redeploy drops all
  pending retries with no record. Fix: a durable delivery **outbox** table
  processed by workers — i.e. the §2a lease pattern again.
- *Cluster-safe already:* rest-hook, messaging, and email channels
  (`channels/{rest_hook,messaging,email}.rs`) are stateless push-to-external —
  any instance delivers identically. No heartbeat/reaper loop exists yet
  (`heartbeat_check_interval`, `config.rs:33`, is unused) — but if one is added
  it must be lease-guarded, not a plain per-instance `interval`.

### Class C — shared caches / replay with local-only invalidation → shared store or cross-instance invalidation

- **C1 · JWT `jti` replay cache — OBSOLETE.** *[amended 2026-07-15: #205
  (merged via PRs #268/#230) removed the jti subsystem entirely — access
  tokens are not one-time assertions, so replay caching them was wrong-layer;
  token validation is now stateless per-request and auth holds no
  cross-instance state. The `HFS_AUTH_JTI_BACKEND` env var, its fail-fast
  check, and the Phase 2 C1 work items no longer exist. Original inventory
  kept below for the record.]*
  `InMemoryJtiCache` (`crates/auth/src/jti/memory.rs:14`) is the **default**
  (`config.rs:73`; built in `crates/hfs/src/main.rs:617`). A one-time client
  assertion accepted on A and replayed to B is honored again — B's cache never
  saw the `jti`. Blast radius scales with instance count. Secondary bug even
  single-instance: it ignores the token's `expires_at` and uses a flat 1 h TTL
  (`memory.rs:26/43`), so a longer-lived token is replayable after eviction. Fix
  already exists: `RedisJtiCache` (`redis.rs:10`). Recommend: when clustered,
  make a shared backend **mandatory / fail-closed**. Severity: **security.**
- **C2 · JWKS refresh — MEDIUM (dead code ready to wire).**
  `JwksCache` (`crates/auth/src/jwks/cache.rs:16`) is per-instance — functionally
  fine (every node fetches the same public keys) but each node independently
  hammers the IdP on a `kid` miss. `JwksCoordinator` (`jwks/coordinator.rs:11`)
  was built to fix this (Redis leader-lock + shared key store) and is **never
  wired**. Fix: wire it under the cluster switch.
  *[amended 2026-07-15 — landed in Phase 2, reshaped: the old coordinator was
  deleted by #205, so C2 is a new `JwksCoordination` trait in auth with
  **watermark** freshness (callers pass the `fetched_at` they hold; a stored
  document is reused only when strictly newer, within a staleness cap, and
  within its own `max_age`). Impls: Postgres — `ClusterRefreshCache` in
  persistence (`cluster_refresh_cache` table, migration v16, per-URL
  `pg_advisory_xact_lock`, fetch under the lock, DB-clock ages) bridged by
  `helios_rest::StoreJwksCoordination`; Redis — `RedisJwksCoordination`
  resurrected behind the auth `redis` feature (lock + stored doc + poll,
  opt-in). Selected by `HFS_AUTH_JWKS_COORDINATION` (unset → `database` under
  `HFS_CLUSTER`); coordination failures fall back to direct IdP fetches, and
  explicit `local` under cluster warns — C2 is warn-only, never a refusal.]*
- **C3 · HTS terminology response caches — HIGH (silent wrong clinical answers).**
  Per-process, no TTL, invalidated **only on the instance that received the
  write/import**:
  - `AppState` caches — `expand_cache`, `not_found_urls`, `*_validate_code_*`,
    `expand_handler_cache`, `lookup_handler_cache`, etc.
    (`crates/hts/src/state.rs:190`); cleared only by local `clear_expand_cache`
    (`state.rs:280`, called from `import_bundle.rs` / `crud.rs`).
  - Postgres backend caches — `inline_compose_cache`, `lookup_response_cache`,
    `cs_resolved_meta_cache`, `subsumes_response_cache`, `translate_response_cache`
    (`crates/hts/src/backends/postgres/mod.rs:97`; cleared `:172`), and
    **process-global `OnceLock` statics** `CLOSURE_COUNT_CACHE` /
    `CLOSURE_PREFIX_CACHE` (`postgres/value_set.rs:37,97`).

  A CodeSystem/ValueSet/ConceptMap update on instance A leaves B serving stale
  `$expand` / `$validate-code` / `$lookup` / `$translate` / `$subsumes` results
  indefinitely — a correctness hazard in a clinical system. Fix: cross-instance
  invalidation (PG `LISTEN/NOTIFY` on terminology writes, or a shared
  terminology-version epoch keyed into the caches, or short TTLs). The SQLite-side
  equivalents (`sqlite/value_set.rs:73`, `code_system.rs:119`) are **Low** — a
  cluster can't share a SQLite file anyway.
  *[amended 2026-07-16 — landed in Phase 4: a shared `terminology_epoch`
  single-row counter (not `LISTEN/NOTIFY`), bumped by `import_parsed`/
  `delete_normalized` and checked (memoized ~1s) by an `EpochGuard` shared
  between the `AppState`-layer handler caches and the backend-layer response
  caches (`crates/hts/src/backends/postgres/epoch.rs`). The two `OnceLock`
  closure statics turned out to already be covered by the existing
  `clear_response_caches()`, so no separate handling was needed for them.
  Opt-in and standalone to the `hts` crate — `HTS_TERMINOLOGY_CACHE_INVALIDATION
  = local | epoch`, **not** coupled to the `hfs` binary's `HFS_CLUSTER` switch
  (session decision: HTS scales independently of the FHIR server, so it gets
  its own toggle in its own `HTS_*` namespace rather than a cross-binary
  dependency).]*

### Class D — once-per-instance background tasks → leader-election / leasing

- **D1 · HTS bootstrap sync — MEDIUM.**
  `bootstrap_sync` runs on every boot (`crates/hts/src/main.rs:78,134,187`),
  deduped by the shared `bootstrap_imports` ledger — but the check-then-import-
  then-record sequence has **no lock**. N instances cold-booting together (rolling
  deploy, autoscale) each see "not imported" and import the same heavy
  SNOMED/LOINC/RxNorm/ICD-10 file concurrently. Idempotent upserts keep the end
  state correct, but you pay N× cost + write contention. Fix: `pg_advisory_lock`
  around bootstrap, or leader-election.
  *[amended 2026-07-16 — landed in Phase 4: `bootstrap_sync_postgres` wraps the
  whole directory-sync call in a session-scoped advisory lock
  (`schema::with_bootstrap_lock`, a new distinct key "HTS_BOOT" alongside
  the existing schema-DDL lock "HTS_SCHM") — one lock scope covers every file
  in one pass, and the per-file ledger check inside `bootstrap_sync` runs
  *inside* the locked section, so a loser that queues behind the winner
  naturally skips whatever the winner already imported. Unconditional, no
  gating (matches D3's precedent) — leader-election was not needed.]*
- **D2 · Bulk export/submit cleanup reapers — LOW (already tolerable).**
  Unleased per-instance reapers (`crates/hfs/src/main.rs:1063,1244`) scan the
  shared DB and race on deletes, but deletes are idempotent → duplicated work
  only. Optional: gate behind a single-owner lease for tidiness.
- **D3 · Postgres schema initialization — FIXED (2026-07-14). [amended 2026-07-14]**
  `initialize_schema` (`crates/persistence/src/backends/postgres/schema.rs`) ran
  unserialized on every boot: N instances cold-starting against one **empty**
  database raced `CREATE TABLE IF NOT EXISTS` — which is not concurrency-safe at
  the catalog level — and the loser aborted on a `pg_type` duplicate-key error.
  Not in the original sweep: it only fires on simultaneous cold-start against an
  empty database, which no single-observer test can produce. Discovered by the
  **first dispatch of the two-instance `cluster-smoke` calibration harness**
  (instance A died on "Failed to create schema_version table" while B won the
  race). Fixed by wrapping init/migrations in `pg_advisory_lock` (released on
  both success and failure paths; also serializes future rolling-upgrade
  migrations). T2 suite:
  `postgres_integration_cluster_concurrent_cold_start_schema_init` — four fresh
  handles race init from a barrier → all succeed, exactly one `schema_version`
  row, plus visibility and wrong-tenant isolation rows.

### Class E — durability queues

- **E1 · Composite async sync queue — HIGH (silent search divergence).**
  In async mode, `SyncManager` buffers secondary-backend (e.g. Elasticsearch)
  propagation in an in-process `mpsc::channel(1000)` with status in
  `RwLock<HashMap>` (`crates/persistence/src/composite/sync.rs:157,160`). A
  crash/redeploy with events still queued permanently loses those secondary
  writes → the search index silently diverges from the primary, recoverable only
  by a full reindex. Both a crash-durability and a cluster issue. Fix: a durable
  outbox (DB table) drained by workers.
  *[amended 2026-07-16 — landed in Phase 4: new `composite_sync_outbox` table
  (migration v18) + `CompositeSyncOutbox` trait
  (`crates/persistence/src/core/composite_sync_outbox.rs`), denormalized to
  **one row per `(event, backend_id)` pair** — not `QueuedEvent`'s prior
  one-row-fans-out-to-N-backends shape — under the same lease + fencing
  discipline as `cluster_jobs`/the subscription delivery outbox, cloned from
  `subscription_outbox.rs`'s claim query. `SyncEvent::BulkSync` stays on the
  pre-existing in-memory channel (doesn't denormalize cleanly to one row per
  resource without changing its batch semantics; bulk resync already rides
  the cluster-safe reindex job store). Wiring is **capability-based, not
  `HFS_CLUSTER`-gated**: `CompositeStorage::new` wires the outbox
  unconditionally whenever the primary backend is Postgres — durable delivery
  beats the in-memory channel even single-instance, the same reasoning behind
  F5's unconditional fix — so there is no new env var and no `hfs`-binary
  wiring needed. `crates/hfs/src/cluster.rs` gained a **warn-only**
  `resolve_composite_sync_durability` (not a refusal row): a non-Postgres
  primary with a composite secondary under `HFS_CLUSTER=true` keeps today's
  already-shipped in-memory fallback, not a cluster-introduced regression,
  unlike subscriptions' functionally-broken-without-Postgres refusal.]*

### Class F — configuration-level cluster caveats (no new code, must be documented)

- **F1 · SQLite default cannot cluster.** `HFS_STORAGE_BACKEND=sqlite`
  (`crates/rest/src/config.rs:739`) is single-writer local file. Clustering ⇒
  Postgres / Elasticsearch / MongoDB. HTS SQLite backend: same.
- **F2 · Bulk output stores default to node-local disk.** `LocalFsOutputStore`
  is the default for both bulk subsystems (`crates/hfs/src/main.rs:955,1124`); a
  download routed to a different node than the writer 404s. Set
  `HFS_BULK_EXPORT_OUTPUT_BACKEND=s3` / `HFS_BULK_SUBMIT_OUTPUT_BACKEND=s3` for
  shared, presigned output.
- **F3 · Sidecar `bulk_export.db` under Mongo/S3 primary.**
  When the primary backend can't host transactional job state (MongoDB, S3),
  bulk-export job state falls back to a per-process local SQLite file
  (`build_embedded_job_store`, `crates/hfs/src/main.rs:886`, worst case a
  per-PID temp path). Under those backends in a cluster, jobs are invisible
  across instances — the SoF failure mode. Fine for Postgres primary (shares the
  primary DB).
- **F4 · Audit file sink is node-local.** `FileSink`
  (`crates/audit/src/sinks/file.rs:19`) writes NDJSON to local (often ephemeral)
  disk → a fragmented, restart-lossy audit trail. Use `DatabaseSink` or
  `CloudWatchLogsSink` (both write immediately to shared infra) when clustered.
- **F5 · Unconditional version-ID increment race.** New `version_id` =
  `current.parse::<u64>() + 1` is a read-then-write with no lock on the
  *unconditional* update/delete path (`crates/persistence/src/backends/postgres/storage.rs:291,396`;
  `mongodb/storage.rs:110`). Two concurrent writers (more likely across
  instances) can both assign N+1, colliding ETags / losing a history version. The
  conditional path is safe (guards on `expected_version`,
  `postgres/storage.rs:946`). Fix: make the increment atomic (`… SET version =
  version+1 … RETURNING`) or require `If-Match`. Severity: **LOW-MEDIUM.**

### Confirmed benign (checked and cleared — for coverage confidence)

Server-assigned resource IDs are `Uuid::new_v4()` (collision-free, no shared
counter). The `AtomicU64`/`AtomicUsize` counters found (`sqlite/mod.rs:484`,
`persistence sqlite/backend.rs:23`, `sof/remote_fetch.rs:62`) name ephemeral
in-memory test DBs or count within a single operation. Per-backend
`search_registry` is immutable config loaded identically per instance. Composite
`HealthMonitor`, per-tenant pool LRU bookkeeping, fhirpath tracer/runtime
statics, outbound OAuth token cache (`crates/rest/src/bulk_submit_oauth.rs:30`),
and CDS Hooks types are per-instance-local or read-only and safe. Audit recording
is fire-and-forget with UUID IDs (no per-instance sequence), only a minor
crash-durability window.

## 6. Environment-variable configuration surface

One master switch plus per-subsystem overrides, all following the existing
`HFS_*` conventions.

| Variable | Values | Default | Effect |
|----------|--------|---------|--------|
| `HFS_CLUSTER` | `true` / `false` | `false` | Master switch: selects cluster-safe backends below and **fails fast** if the primary backend is SQLite or a required shared dependency is unset. |
| `HFS_JOB_STORE_BACKEND` | `memory` / `database` | `memory` (`database` when `HFS_CLUSTER`) | Unified job store (§4) — SoF export, reindex; bulk export/submit already DB-backed. |
| ~~`HFS_AUTH_JTI_BACKEND`~~ | — | — | *[amended 2026-07-15]* removed with the jti subsystem (#205); C1 is obsolete |
| `HFS_AUTH_JWKS_COORDINATION` | `local` / `database` / `redis` | `local` (`database` when `HFS_CLUSTER`) | Cluster single-flight JWKS refresh (C2). Warn-only: explicit `local` under cluster warns, never refuses. *(new — landed Phase 2)* **[amended 2026-07-15]** |
| `HFS_AUTH_REDIS_URL` | URL | — | Redis for ~~`jti` +~~ the JWKS coordinator (`redis` mode; needs the `redis` build feature). *[amended 2026-07-15: removed by #205, re-added in Phase 2 for C2 only]* |
| `HFS_SUBSCRIPTIONS_FANOUT` | `memory` / `redis` / `nats` / `pg-notify` | `memory` | Shared pub/sub for WS delivery + WS binding tokens + counters (class B). *(new)* |
| ~~`HFS_TERMINOLOGY_CACHE_INVALIDATION`~~ `HTS_TERMINOLOGY_CACHE_INVALIDATION` | `local` / `epoch` | `local` | Cross-instance HTS cache invalidation (C3), read by the **`hts` binary**, not `hfs` — HTS has its own `HTS_*` config surface and no `HFS_CLUSTER` coupling. *(new — landed Phase 4)* **[amended 2026-07-16: corrected the variable's prefix/name to match the code — it lives in `crates/hts`, not `hfs`/`rest`, so the `HFS_` prefix used above was wrong; also corrected values from the original `local / pg-notify / redis` sketch to the shape actually landed, `local / epoch`, a shared-counter table not `LISTEN/NOTIFY` or Redis]** |
| `HFS_BULK_EXPORT_OUTPUT_BACKEND` | `local-fs` / `s3` | `local-fs` | Set `s3` when clustered (F2). *(exists)* |
| `HFS_BULK_SUBMIT_OUTPUT_BACKEND` | `local-fs` / `s3` | `local-fs` | Set `s3` when clustered (F2). *(exists)* |
| `HFS_AUDIT_BACKEND` | `database` / `file` / `cloudwatch` / `none` | `none` | Avoid `file` when clustered (F4). *(exists — was misnamed `HFS_AUDIT_SINK` here; the code parses `HFS_AUDIT_BACKEND`, `crates/audit/src/config.rs`)* **[amended 2026-07-14]** |

Design intent: an operator sets `HFS_CLUSTER=true`, points at a shared primary
DB, and provides Redis (or accepts DB-notify equivalents); the switch turns the
individual backends to their cluster-safe variants and refuses to boot on an
unsafe combination rather than silently degrading.

## 7. Operator documentation deliverable

Ship a *"Running HFS in a cluster"* page (book chapter + skill note) covering:

1. **Prerequisites** — a shared primary backend (Postgres/ES/Mongo, **not**
   SQLite), shared object storage (S3) for bulk output, and Redis (or the
   `pg-notify` alternatives) for replay/fan-out/invalidation.
2. **The one-switch setup** — `HFS_CLUSTER=true` + the shared-infra URLs, and the
   fail-fast checks it enforces.
3. **Per-subsystem behavior matrix** — what each `$operation` / channel does with
   1 vs N instances, and which env var makes it cluster-safe.
4. **Known single-instance-only features** until their fix lands — with the same
   ⚠️ pinning guidance #169/#170 already carry (pin to one instance or use sticky
   sessions).
5. **Load-balancer notes** — WebSocket sticky sessions are *not* sufficient for
   Subscription delivery (the triggering event originates on an arbitrary node);
   pub/sub fan-out is required.

## 8. Suggested phasing

Each phase is independently shippable and leaves the tree green.

- **Phase 0 — framing & guardrails.** Add `HFS_CLUSTER` with fail-fast validation
  (reject SQLite primary, `memory` `jti`, `local-fs` bulk output, `file` audit)
  and publish the §7 operator doc describing current single-instance limits. No
  behavior change to the happy path. *(Highest value / lowest risk — documents
  and enforces the boundary immediately.)*
- **Phase 1 — unified job store + SoF export (#169).** Land `ClusterJobStore`
  (generalized from `BulkExportJobStore`), implement `ExportJobController` over
  it, wire `HFS_JOB_STORE_BACKEND`/`HFS_EXPORT_CONTROLLER`. Fold reindex jobs
  (A2) onto the same table.
- **Phase 2 — auth hardening.** Make `jti` shared-mandatory under `HFS_CLUSTER`
  ~~(C1)~~ *[amended 2026-07-15: C1 obsolete per #205]*; wire `JwksCoordinator` (C2). Small, mostly existing code.
  *[amended 2026-07-15: C2 landed — see the §5 C2 amendment for the shape it
  took (new trait + Postgres/Redis impls, not the deleted coordinator).]*
- **Phase 3 — Subscriptions cluster delivery (#170).** DB-backed
  subscription/topic load + startup reconciliation (B3), shared pub/sub fan-out
  (B1), shared/stateless WS binding tokens (B2), shared counters (B4), durable
  delivery outbox (B5).
- **Phase 4 — HTS cache coherency (C3) + bootstrap lock (D1)** and
  **composite durable sync outbox (E1).**
  *[amended 2026-07-16 — landed: D1 (unconditional advisory lock), C3
  (opt-in `HTS_TERMINOLOGY_CACHE_INVALIDATION=epoch`, standalone to the
  `hts` binary), and E1 (`composite_sync_outbox`, capability-based wiring,
  no `HFS_CLUSTER` gating needed) — see the §5 C3/D1/E1 amendments for the
  shapes actually landed. The T3 kill-9 nightly case for E1 (methodology
  §6/§7, mirroring A1's) remains outstanding.]*
- **Continuous — config caveats (F1–F5)** documented in Phase 0, code fixes
  (F5 atomic version increment) folded in where cheap.

## 9. Out of scope / open questions

- **Redis vs DB-only.** Can we keep the shared-infra footprint to *just the
  primary DB* (using `LISTEN/NOTIFY` + a DB table for replay/fan-out) so a
  cluster needs no Redis? Trade-off: DB load & `NOTIFY` fan-out limits vs one
  fewer dependency. The env-var surface above keeps both open.
  *[amended 2026-07-15 — resolved for C2: the DB-backed coordinator is the
  default, so a DB-only cluster needs no Redis; the Redis impl was
  resurrected as an explicit opt-in (session decision: keep a Redis scaffold
  for future cluster work). Its gated T2 twin runs via
  `redis-cluster-tests.yml` / `RUN_REDIS_CLUSTER_TESTS=1`.]*
- **Leader-elected singletons.** Reapers (D1/D2) and any future heartbeat loop
  could share one leader-election primitive rather than each rolling its own
  lease.
- **Observability in a cluster ([#150]).** Per-instance `/metrics` with tenant as
  a span attribute (never a metric label) is already correct; the open item is
  documenting cross-instance aggregation, not code.
- **Bulk export under Mongo/S3 primary (F3).** Decide whether to route its job
  state onto the unified `database` job store (needs a transactional home) or to
  document it as Postgres-primary-only for clustering.