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
| Subscriptions (topics, delivery, WebSockets) | ✅ | ❌ **single-instance only** — registries are in-memory and only the instance that served a write reacts to it | Phase 3 (#170) |
| Terminology (HTS) response caches | ✅ | ⚠️ a terminology import on one instance leaves stale `$expand`/`$validate-code` answers on the others | Phase 4 |
| HTS bootstrap import | ✅ | ⚠️ N cold-starting instances all run the heavy import (correct but N× cost) | Phase 4 |
| Composite async search sync | ✅ | ⚠️ in-process queue; a crash loses queued secondary-index writes | Phase 4 (durable outbox) |
| Audit, metrics, health | ✅ | ✅ with shared sinks; `/metrics` is per-instance (aggregate in your scraper) | already safe |

**⚠️ single-instance-only features:** until their phase lands, pin
Subscriptions to one instance (or route their requests to one instance with
sticky sessions) — the same guidance issue #170 carries. SQL-on-FHIR async
exports and reindex are cluster-safe as of Phase 1 (`HFS_JOB_STORE_BACKEND=
database`, plus a shared `HFS_EXPORT_SINK` for exports).

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

---

## Load-balancer notes

- **Round-robin is fine for CRUD** — any instance answers any request against
  the shared database.
- **Sticky sessions are *not* sufficient for Subscriptions.** A WebSocket
  client can be pinned to instance B, but the resource write that triggers a
  notification can land on instance A — and today only the instance that
  served the write reacts. Cross-instance delivery requires the Phase 3
  pub/sub fan-out, not routing tricks.
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
through the front, download the shard via B. The same checks are easy to run
by hand against any deployment: create a resource through the balancer, then
`GET` it repeatedly and confirm every instance serves it; kick off an export
and poll/download it from a different instance than the one that accepted
it.
