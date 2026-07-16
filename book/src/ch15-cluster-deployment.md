# Running HFS in a Cluster

This chapter describes what happens when you run more than one `hfs` instance
behind a load balancer, which subsystems are cluster-safe today, and how the
`HFS_CLUSTER` switch enforces a safe configuration at boot.

The full design and its status live in the repository under
[`docs/cluster-capable-state-design.md`](https://github.com/HeliosSoftware/hfs/blob/main/docs/cluster-capable-state-design.md)
(discussion #223). This chapter is the operator-facing summary.

---

## Prerequisites

A cluster is only as shared as its slowest-moving state. Before starting a
second instance, you need:

1. **A shared primary database** — PostgreSQL, MongoDB, or Elasticsearch-backed
   modes (`HFS_STORAGE_BACKEND=postgres`, `postgres-elasticsearch`, `mongodb`,
   …). **SQLite cannot be clustered**: it is a single-writer local file, and
   `HFS_CLUSTER=true` refuses to boot on it.
2. **Shared object storage for bulk output** — set
   `HFS_BULK_EXPORT_OUTPUT_BACKEND=s3` and `HFS_BULK_SUBMIT_OUTPUT_BACKEND=s3`.
   The default `local-fs` writes to node-local disk, so a download routed to a
   different node than the writer returns 404.
3. **A shared audit sink** — `HFS_AUDIT_BACKEND=database` or `cloudwatch`.
   The `file` sink writes node-local NDJSON and produces a fragmented,
   restart-lossy audit trail.

---

## The one-switch setup: `HFS_CLUSTER=true`

Setting `HFS_CLUSTER=true` declares "this process is one of N". It does two
things:

1. **Flips cluster-safe defaults** — e.g. `HFS_JOB_STORE_BACKEND` defaults to
   `database` instead of `memory` (takes effect as the unified job store
   lands; see the status matrix below).
2. **Fails fast on unsafe combinations** instead of silently degrading. The
   server refuses to boot, printing one `Configuration error:` line per
   violation:

| Check | Refused when clustered | Fix |
|-------|------------------------|-----|
| Primary backend | `HFS_STORAGE_BACKEND=sqlite` (or `sqlite-elasticsearch`) | Use `postgres`, `mongodb`, or an `*-elasticsearch` mode over them |
| Bulk export output | `HFS_BULK_EXPORT_OUTPUT_BACKEND=local-fs` (with bulk export enabled) | Use `s3` |
| Bulk submit output | `HFS_BULK_SUBMIT_OUTPUT_BACKEND=local-fs` (with bulk submit enabled) | Use `s3` |
| Audit sink | `HFS_AUDIT_BACKEND=file` (with audit enabled) | Use `database` or `cloudwatch` |
| Job store | explicit `HFS_JOB_STORE_BACKEND=memory` | Remove it or set `database` |
| SoF export controller | explicit `HFS_EXPORT_CONTROLLER=memory` (with SoF enabled) | Remove it (defaults to `database` when clustered) or set `database` |
| SoF export output | `HFS_EXPORT_SINK=fs` (with SoF enabled) | Use `s3` |
| Subscriptions fan-out | explicit `HFS_SUBSCRIPTIONS_FANOUT=memory` (with subscriptions enabled) | Remove it (defaults to `pg-notify` when clustered) or disable subscriptions |
| Subscriptions primary | subscriptions enabled on a non-PostgreSQL primary | Use a `postgres` primary (the fan-out and shared delivery state ride it) or disable subscriptions |

With `HFS_CLUSTER` unset (the default), nothing changes: a single instance
keeps its zero-configuration SQLite defaults.

A minimal two-instance setup on one shared PostgreSQL:

```bash
export HFS_CLUSTER=true
export HFS_STORAGE_BACKEND=postgres
export HFS_DATABASE_URL=postgresql://…      # same URL on every instance
export HFS_BULK_EXPORT_OUTPUT_BACKEND=s3
export HFS_BULK_SUBMIT_OUTPUT_BACKEND=s3
export HFS_AUDIT_BACKEND=database

hfs --port 8080 &   # instance A
hfs --port 8081 &   # instance B (in practice: another machine/pod)
```

---

## Per-subsystem behavior with 1 vs N instances

The cluster-capable-state work lands in phases; this matrix reflects the
current state and is updated as each phase ships.

| Subsystem | 1 instance | N instances today | Cluster-safe since / planned |
|-----------|------------|-------------------|------------------------------|
| FHIR CRUD, search, history | ✅ | ✅ shared-database backends are stateless per request | already safe |
| Bulk Data `$export` / `$bulk-submit` (jobs) | ✅ | ✅ DB-leased jobs with fencing tokens; every instance runs competing workers | already safe (Postgres primary) |
| Bulk output download | ✅ | ✅ with `s3` output; ❌ with `local-fs` | F2 — enforced by fail-fast |
| Concurrent cold-start schema init | ✅ | ✅ serialized by a Postgres advisory lock | fixed 2026-07-14 (D3) |
| SQL-on-FHIR `$viewdefinition-export` / `$sqlquery-export` (async jobs) | ✅ | ✅ with `HFS_EXPORT_CONTROLLER=database` (the default under `HFS_CLUSTER`): jobs on the shared `cluster_jobs` store, any instance polls/cancels/downloads, workers on every instance compete for work | Phase 1 (#169) — landed |
| Search reindex jobs | ✅ | ✅ with `HFS_JOB_STORE_BACKEND=database` (the default under `HFS_CLUSTER`): jobs on the shared `cluster_jobs` store, any instance answers `$reindex-status`/cancel, workers on every instance compete for the rebuild (Postgres primary) | Phase 1 (A2) — landed |
| JWT validation | ✅ | ✅ stateless per-request validation — the former `jti` replay cache was removed with #205 (access tokens are not one-time assertions), so auth holds no cross-instance state | already safe (#205) |
| JWKS refresh | ✅ | ✅ coordinated with `HFS_AUTH_JWKS_COORDINATION=database` (the default under `HFS_CLUSTER`, Postgres primary): cluster-wide single-flight over the shared `cluster_refresh_cache` table — one IdP fetch per boot herd / key rotation, every other instance reuses the stored document. `redis` (build feature) is an opt-in alternative; `local` keeps per-instance refresh (functionally correct — a warning, never a refusal) | Phase 2 (C2) — landed |
| Subscriptions (topics, delivery, WebSockets) | ✅ | ✅ with `HFS_SUBSCRIPTIONS_FANOUT=pg-notify` (the default under `HFS_CLUSTER`, Postgres primary): registries hydrate from the database at boot and stay in sync over LISTEN/NOTIFY, `eventNumber`/failure counters are shared (migration v17), WebSocket binding tokens mint/redeem on any instance, notifications reach sockets on every instance, and push-channel deliveries (rest-hook/email/message) run on a durable leased outbox that survives restarts | Phase 3 (#170) — landed |
| Terminology (HTS) response caches | ✅ | ✅ with `HTS_TERMINOLOGY_CACHE_INVALIDATION=epoch` (opt-in, HTS-only — see below): every instance checks a shared `terminology_epoch` counter and clears its caches on a detected import elsewhere; ⚠️ still stale under the `local` default | Phase 4 (C3) — landed |
| HTS bootstrap import | ✅ | ✅ serialized cluster-wide by a Postgres advisory lock (unconditional, no config) — N cold-starting instances run the heavy import once, not N times | Phase 4 (D1) — landed |
| Composite async search sync | ✅ | ✅ `Create`/`Update`/`Delete` events durably enqueue to a `composite_sync_outbox` table (automatic whenever the primary is Postgres — no config) and survive a crash; `BulkSync` and non-Postgres primaries stay on the pre-existing in-process queue | Phase 4 (E1) — landed |
| Audit, metrics, health | ✅ | ✅ with shared sinks; `/metrics` is per-instance (aggregate in your scraper) | already safe |

SQL-on-FHIR async exports and reindex are cluster-safe as of Phase 1
(`HFS_JOB_STORE_BACKEND=database`, plus a shared `HFS_EXPORT_SINK` for
exports); Subscriptions as of Phase 3 (`HFS_SUBSCRIPTIONS_FANOUT=pg-notify`,
Postgres primary); HTS terminology caches, HTS bootstrap import, and
composite async search sync as of Phase 4.

### JWKS refresh coordination (Phase 2)

With auth enabled, every instance caches the IdP's JWKS public keys and
refreshes on boot, on TTL expiry, and on an unknown `kid` (key rotation) —
so N instances all hit the IdP at the same moments. Per-instance refresh is
**functionally correct** (every node fetches the same public keys), which is
why this is the one cluster concern that only warns, never refuses.

`HFS_AUTH_JWKS_COORDINATION` selects the behavior:

| Value | Effect |
|-------|--------|
| *(unset)* | `database` under `HFS_CLUSTER=true`, `local` otherwise |
| `local` | Per-instance refresh (a warning is logged when clustered) |
| `database` | Cluster-wide single-flight over the primary backend's shared `cluster_refresh_cache` table (Postgres primary; per-URL advisory lock + stored document). Other primaries log a warning and stay per-instance |
| `redis` | Single-flight over Redis (`HFS_AUTH_REDIS_URL` required; needs an `hfs` build with the `redis` feature) |

Under coordination, exactly one instance fetches from the IdP per boot herd
or rotation; the rest adopt the stored document, shortening their local
cache lifetime by its age. If the coordination layer itself is unavailable
(database or Redis down), instances log a warning and fall back to direct
IdP fetches — auth availability outranks the dedupe optimization.

### Subscriptions cluster delivery (Phase 3)

`HFS_SUBSCRIPTIONS_FANOUT` selects the behavior (with subscriptions enabled):

| Value | Effect |
|-------|--------|
| *(unset)* | `pg-notify` under `HFS_CLUSTER=true`, `memory` otherwise |
| `memory` | Per-instance registries and delivery — single-instance only (refused under `HFS_CLUSTER=true`) |
| `pg-notify` | Cluster delivery over the PostgreSQL primary: shared counters/status/outbox tables (migration v17) plus a LISTEN/NOTIFY fan-out on a dedicated connection. Non-Postgres primaries log a warning and stay per-instance |

How the pieces behave, and the guarantees they carry:

- **Registries** (SubscriptionTopic + Subscription, including the R4
  backport `Basic` form) hydrate from the database at startup and stay in
  sync at runtime via lifecycle announcements — a Subscription created on
  any instance fires for matching writes served by every instance. Topics
  are tenant-scoped as of Phase 3: one tenant's SubscriptionTopic is no
  longer resolvable by another tenant's Subscription.
- **`eventNumber` is cluster-wide monotonic and gap-free** (a shared
  counter row per subscription); resource updates never reset it.
  Consecutive delivery failures accumulate across instances, so the
  `error`/`off` status thresholds work no matter which instance failed.
- **WebSockets**: `$get-ws-binding-token` mints into a shared single-use
  table, so the WebSocket upgrade may land on any instance. Notifications
  produced anywhere reach sockets everywhere via the fan-out. Delivery to
  *remote* sockets is best-effort: envelopes published while an instance's
  listen connection is down are lost, the instance re-syncs on reconnect,
  and clients detect gaps via `eventNumber` (the FHIR WebSocket channel
  contract).
- **Push channels** (rest-hook, email, message) deliver from a durable
  outbox with worker leases and persisted retry backoff — a redeploy no
  longer drops pending retries. Semantics are **at-least-once**: endpoints
  should tolerate a duplicate delivery after a worker crash (dedupe by
  `eventNumber`).
- **Pooler caveat**: the fan-out LISTENs on a dedicated session-level
  connection. A transaction-pooling proxy (e.g. pgbouncer in transaction
  mode) between HFS and PostgreSQL breaks LISTEN/NOTIFY — connect the
  instances directly, or use session pooling.
- The subscription heartbeat interval remains unwired; if it lands later it
  must be lease-guarded, not a per-instance timer.

### Terminology (HTS) cache coherency (Phase 4, C3)

**HTS is a separate binary from `hfs`** with its own configuration surface
(`HTS_*`, not `HFS_*`) and no awareness of `HFS_CLUSTER` — it can be scaled
independently of the FHIR server, so this is opt-in on its own terms rather
than inheriting the FHIR server's cluster switch.

`HTS_TERMINOLOGY_CACHE_INVALIDATION` selects the behavior (PostgreSQL storage
only — a cluster can't share a SQLite file anyway):

| Value | Effect |
|-------|--------|
| *(unset, `local`)* | Per-instance response caches, cleared only by writes served locally — a CodeSystem/ValueSet/ConceptMap import on instance A leaves B serving stale `$expand`/`$validate-code`/`$lookup`/`$translate`/`$subsumes` results indefinitely |
| `epoch` | Every terminology write bumps a shared `terminology_epoch` counter; every instance checks it (memoized, ~1s) before trusting its local caches and clears them on a detected transition |

Two separate cache layers are covered — the `AppState` handler-level caches
and the backend's response caches — since both must independently detect a
transition (a shared "have we cleared yet" flag would let whichever layer
checks first silently suppress the other's clear). The bump is a best-effort
statement issued right after the write commits, not transactional with it,
bounding cross-instance staleness to roughly the memo window rather than the
write's own transaction.

### Composite secondary-backend sync durability (Phase 4, E1)

Composite storage modes (`*-elasticsearch`) propagate writes to a secondary
search backend either synchronously (blocks the write) or asynchronously (an
in-process queue). In asynchronous mode, a crash or redeploy with events
still queued has always silently lost those secondary-index writes,
recoverable only by a full reindex — true single-instance, not just
clustered.

Unlike the other Phase 1–3 seams, this activates **automatically, with no
new environment variable**: whenever the primary backend is PostgreSQL,
`Create`/`Update`/`Delete` events durably enqueue to a `composite_sync_outbox`
table (one row per event per secondary backend) and are drained by
per-instance workers under a lease + fencing discipline, surviving a crash.
This applies whether or not `HFS_CLUSTER` is set — durable delivery is
strictly better than the in-memory queue even for a single instance, the
same reasoning behind F5's unconditional version-write fix. `SyncEvent::BulkSync`
(large batch resyncs) and non-Postgres primaries keep the pre-existing
in-process-queue behavior unchanged.

Under `HFS_CLUSTER=true` with a composite secondary configured on a
non-Postgres primary, boot logs a warning (never a refusal) — composite sync
falls back to the same in-memory behavior already shipped single-instance,
not a cluster-introduced regression, which is why this is warn-only rather
than in the refusal table above.

---

## Load-balancer notes

- **Round-robin is fine for CRUD** — any instance answers any request against
  the shared database.
- **Sticky sessions are no longer needed for Subscriptions** (Phase 3): a
  WebSocket client pinned to instance B receives notifications for writes
  served by instance A via the pg-notify fan-out, and binding tokens redeem
  on any instance.
- **WebSocket upgrades** need the usual `Upgrade`/`Connection` header
  pass-through on the proxy.
- **Health checks**: probe each instance's `/health` directly, not through
  the balancer, so one bad instance can't mask another.

---

## Verifying a cluster

The repository ships a two-instance smoke harness — two `hfs` processes
sharing one PostgreSQL behind an nginx round-robin front
(`.github/workflows/cluster-smoke.yml`). It asserts health on both instances
and the front, real round-robin distribution, cross-instance visibility
(write on A, read on B), and — since Phase 1 — a full SoF `$viewdefinition-
export` round-trip across instances: kick off on A, poll the status URL
through the front, download the shard via B. Since Phase 4, the same
workflow also runs two `hts` processes sharing the same PostgreSQL database:
a CodeSystem import via A, a stale read pre-warmed on B, an update via A, and
proof that B's `epoch`-mode cache invalidates and serves the fresh answer.
The same checks are easy to run by hand against any deployment: create a
resource through the balancer, then `GET` it repeatedly and confirm every
instance serves it; kick off an export and poll/download it from a different
instance than the one that accepted it.
