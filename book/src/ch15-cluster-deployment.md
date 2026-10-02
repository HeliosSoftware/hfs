# Running HFS in a Cluster

This chapter describes what happens when you run more than one `hfs` instance
behind a load balancer, which subsystems are cluster-safe today, which are
still single-instance, and how the `HFS_CLUSTER` switch enforces a safe
configuration at boot.

See discussion [#223](https://github.com/HeliosSoftware/hfs/discussions/223)
for the design background. This chapter is the operator-facing summary and is
updated as each cluster-capable subsystem lands.

---

## Prerequisites

A cluster is only as shared as its slowest-moving state. Before starting a
second instance, you need:

1. **A shared primary database** — PostgreSQL, MongoDB, or an
   Elasticsearch-backed mode over them (`HFS_STORAGE_BACKEND=postgres`,
   `postgres-elasticsearch`, `mongodb`, …). **SQLite cannot be clustered**: it
   is a single-writer local file, and `HFS_CLUSTER=true` refuses to boot on it.
   Prefer PostgreSQL: it is the primary every cluster-capable subsystem is
   built on first, and bulk export needs it (see below).
2. **Shared object storage for bulk output** — set
   `HFS_BULK_EXPORT_OUTPUT_BACKEND=s3` and `HFS_BULK_SUBMIT_OUTPUT_BACKEND=s3`.
   The default `local-fs` writes to node-local disk, so a download routed to a
   different node than the writer returns 404.
3. **A shared audit sink** — `HFS_AUDIT_BACKEND=database` or `cloudwatch`.
   The `file` sink writes node-local NDJSON and produces a fragmented,
   restart-lossy audit trail.
4. **A load balancer that forwards SIGTERM-friendly deploys.** Every instance
   drains in-flight requests and flushes its audit sink on SIGTERM as well as
   Ctrl-C, so a rolling restart can stop an instance the way container
   runtimes and process supervisors do.

---

## The one-switch setup: `HFS_CLUSTER=true`

Setting `HFS_CLUSTER=true` declares "this process is one of N". It does two
things:

1. **Flips cluster-safe defaults** — `HFS_JOB_STORE_BACKEND` defaults to
   `database` instead of `memory`. (The unified job store that reads it is
   the next subsystem to land; until then the selector is parsed and
   validated but not consumed.)
2. **Fails fast on unsafe combinations** instead of silently degrading. The
   server refuses to boot, printing one `Configuration error:` line per
   violation so a deployment can be fixed in one pass:

| Check | Refused when clustered | Fix |
|-------|------------------------|-----|
| Primary backend | `HFS_STORAGE_BACKEND=sqlite` (or `sqlite-elasticsearch`) | Use `postgres`, `mongodb`, or an `*-elasticsearch` mode over them |
| Bulk export output | `HFS_BULK_EXPORT_OUTPUT_BACKEND=local-fs` (with bulk export enabled) | Use `s3` |
| Bulk submit output | `HFS_BULK_SUBMIT_OUTPUT_BACKEND=local-fs` (with bulk submit enabled) | Use `s3` |
| Bulk export job store | bulk export enabled on a `mongodb` or `s3` primary — its job store is a node-local SQLite sidecar (worst case a per-process temp file), so job state is invisible to other instances | Use a `postgres` primary, or `HFS_BULK_EXPORT_ENABLED=false` |
| Audit sink | `HFS_AUDIT_BACKEND=file` | Use `database` or `cloudwatch` |
| Job store | explicit `HFS_JOB_STORE_BACKEND=memory` | Remove it (defaults to `database` when clustered) or set `database` |

Two configurations run but log a warning at boot:

| Warning | Why | What to do |
|---------|-----|------------|
| SQL-on-FHIR `$sql-export` enabled | Async export job state is held in-process by the instance that accepted the export, so a poll, cancel, or download routed elsewhere returns 404 | Pin export clients to one instance or use sticky sessions until the database-backed controller lands; or `HFS_SOF_ENABLED=false` |
| `HFS_EXPORT_SINK=fs` | Export output under `HFS_EXPORT_DIR` is only reachable from every instance if that directory is shared (e.g. NFS) | Share the directory, or use `s3` |

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

The cluster-capable-state work lands one subsystem at a time; this matrix is
the current state and is updated as each one ships.

| Subsystem | 1 instance | N instances today | Status |
|-----------|------------|-------------------|--------|
| FHIR CRUD, search, history | ✅ | ✅ shared-database backends are stateless per request; every version bump is a version-guarded compare-and-swap, so two instances racing an unconditional `PUT` (including onto a deleted id) both succeed with distinct versions and a complete history | safe |
| Concurrent cold-start schema init | ✅ | ✅ serialized by a PostgreSQL advisory lock | safe |
| Bulk Data `$export` / `$bulk-submit` (jobs) | ✅ | ✅ DB-leased jobs with fencing tokens on a PostgreSQL primary; every instance runs competing workers. ❌ bulk export on a MongoDB/S3 primary uses a node-local sidecar job store — refused by `HFS_CLUSTER` | safe on PostgreSQL |
| Bulk output download | ✅ | ✅ with `s3` output; ❌ with `local-fs` — refused by `HFS_CLUSTER` | safe with `s3` |
| Graceful shutdown | ✅ | ✅ SIGTERM and Ctrl-C both drain and flush | safe |
| JWT validation | ✅ | ✅ stateless per-request validation; auth holds no cross-instance state. Each instance refreshes the IdP's JWKS independently (N fetches per boot or key rotation — a cost, not a correctness issue) | safe |
| SQL-on-FHIR `$sql-export` (async jobs) | ✅ | ⚠️ job state is per-instance: poll, cancel and download must reach the instance that accepted the export (`HFS_CLUSTER` warns) | **planned** — unified job store + database controller (#169) |
| Search `$reindex` jobs | ✅ | ⚠️ job state is per-instance: `$reindex-status` and cancel must reach the instance that started the job | **planned** — unified job store |
| Subscriptions (topics, delivery, WebSockets) | ✅ | ⚠️ single-instance: a Subscription created on instance A fires only for writes served by A until B restarts (registries hydrate at boot only); a WebSocket must be bound to the instance that minted its token and only receives notifications for writes served there; `eventNumber` and failure counters are per-instance; pending retries are lost on redeploy | **planned** — shared state + fan-out (#170) |
| Terminology (HTS) response caches | ✅ | ⚠️ `hts` is a separate binary; its response caches are per-instance and cleared only by writes served locally, so an import on one instance leaves the others stale | **planned** — HTS opt-in epoch invalidation |
| HTS bootstrap import | ✅ | ⚠️ N cold-starting `hts` instances all run the same bootstrap import (idempotent, but N× the cost) | **planned** — advisory lock |
| Composite async search sync (`*-elasticsearch`) | ⚠️ in-process queue; a crash loses queued secondary writes | ⚠️ same, per instance | **planned** — durable outbox |
| Audit, metrics, health | ✅ | ✅ with shared sinks; `/metrics` is per-instance (aggregate in your scraper) | safe |

---

## Load-balancer notes

- **Round-robin is fine for CRUD** — any instance answers any request against
  the shared database.
- **Sticky sessions are required today for SQL-on-FHIR async exports and for
  Subscriptions.** An export's status, cancel, and download URLs must reach
  the instance that accepted it, and a WebSocket subscriber must bind to the
  instance that minted its token. Note that sticky sessions are not
  sufficient for Subscriptions: the write that triggers a notification may be
  served by any instance, and only the instance holding the socket can
  deliver it. Until the fan-out lands, run Subscriptions on one instance.
- **WebSocket upgrades** need the usual `Upgrade`/`Connection` header
  pass-through on the proxy.
- **Health checks**: probe each instance's `/health` directly, not through
  the balancer, so one bad instance can't mask another.
- **Rolling deploys**: send SIGTERM and wait for the instance to exit; it
  drains in-flight requests and flushes the audit sink before it does.

---

## Verifying a cluster

The repository ships a two-instance smoke harness — two `hfs` processes
sharing one PostgreSQL behind an nginx round-robin front
(`.github/workflows/cluster-smoke.yml`, driven by
`crates/hfs/tests/cluster/run_external_cluster_smoke.sh`). It asserts health
on both instances and the front, real round-robin distribution
(`X-Hfs-Upstream` alternates), cross-instance visibility (write on A, read on
B), and that both instances drain on SIGTERM. Each subsystem that becomes
cluster-safe adds its own cross-instance check to the same harness.

The same checks are easy to run by hand against any deployment: create a
resource through the balancer, then `GET` it repeatedly and confirm every
instance serves it; stop one instance with SIGTERM and confirm it logs
`Shutdown signal received` and exits cleanly.

How the cluster-capable work is tested is described in the `test-hfs`
project skill: pure unit tables for the boot-time validation, two-handle
tests that run two independently constructed backends over one database to
prove a protocol across instances, and this two-process smoke for what only a
real socket or a real process can show.
