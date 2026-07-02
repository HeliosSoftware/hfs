# Console Metrics Endpoints

JSON endpoints that back the **Helios FHIR Server Console** dashboard
(uptime stat + "FHIR Resources over time" chart). They are served by the `hfs`
binary and are intended to be consumed directly by the management-console web
app.

Endpoints fall into three trust tiers:

| Tier | Endpoints | Requirement |
|---|---|---|
| **Public** | `uptime` | none — mounted outside auth (like `/health`) |
| **Tenant-scoped** | `resource-counts`, `activity`, `resource-distribution` | a valid `Authorization: Bearer <token>` (else `401`); returns only the caller's own tenant |
| **Admin (cross-tenant)** | `tenants`, `traffic` | a valid token **and** a system-context scope `system/*.r` (else `403`) |

All endpoints are covered by the server's CORS, timeout, body-limit, and tracing
middleware.

Behind auth, the tenant is taken **authoritatively from the JWT claim** (see
`TenantExtractor`); with `strict_validation` enabled a request whose `X-Tenant-ID`
disagrees with the token is rejected (`400`). So a spoofed `X-Tenant-ID` cannot
widen access beyond the caller's token.

The **admin** endpoints span every tenant — `tenants` returns the full tenant
roster and per-tenant sizes, `traffic` returns server-wide throughput/latency.
They are therefore gated by `admin_authz_middleware`, which requires a
system-context, all-resource scope (`system/*.r`, `system/*.rs`, `system/*.cruds`,
…). An ordinary `user/*` or `patient/*` token — **even a wildcard one** — is
rejected `403`.

> When auth is **disabled** server-wide, all endpoints (including the admin tier)
> are unprotected like every other route — matching existing server behaviour.

The same counts are also exported as Prometheus gauges on `/metrics` (see
[Prometheus](#prometheus) below).

> **Tenant** — like every HFS request, the active tenant is resolved from the
> `X-Tenant-ID` header (or the configured default tenant). Pass `X-Tenant-ID` to
> scope the counts to a specific tenant.

---

## `GET /console/metrics/uptime`

Process uptime and server identity.

### Response `200 OK`

```json
{
  "service": "hfs",
  "version": "0.1.47",
  "started_at": "2026-06-30T08:00:00.123456+00:00",
  "now": "2026-06-30T12:04:51.654321+00:00",
  "uptime_seconds": 14691.53,
  "uptime_human": "4h 4m 51s",
  "availability": {
    "tracked": false,
    "note": "availability tracking is not enabled; uptime_seconds reflects time since the current process started"
  }
}
```

| Field | Meaning |
|-------|---------|
| `uptime_seconds` | Seconds since the current process started (float). |
| `uptime_human` | Compact human form, e.g. `"4d 2h 13m 5s"`. |
| `started_at` / `now` | RFC 3339 timestamps. |
| `availability` | Honest placeholder. A real "99.98% over 30d" figure needs health-probe history that HFS does not yet record; this is surfaced as `tracked: false` rather than a fabricated number. |

---

## `GET /console/metrics/resource-counts`

Per-type stored-resource totals plus a dense daily series for the
"resources over time" chart.

### Query parameters (all optional)

| Param | Default | Notes |
|-------|---------|-------|
| `types` | The eight dashboard types¹ | Comma-separated FHIR resource types, e.g. `types=Patient,Observation`. |
| `days` | `30` | Daily window size, clamped to `1..=365`. |

¹ `Patient, Observation, Encounter, Condition, MedicationRequest,
DiagnosticReport, Procedure, AllergyIntolerance`.

### Example

```
GET /console/metrics/resource-counts?types=Patient,Observation&days=30
X-Tenant-ID: acme-health
```

### Response `200 OK`

```json
{
  "tenant": "acme-health",
  "generated_at": "2026-06-30T12:04:51.654321+00:00",
  "window": {
    "interval": "day",
    "days": 30,
    "since": "2026-06-01",
    "until": "2026-06-30"
  },
  "total_resources": 61423,
  "series": [
    {
      "resource_type": "Patient",
      "total": 1204,
      "points": [
        { "date": "2026-06-01", "count": 2,  "cumulative": 1180 },
        { "date": "2026-06-02", "count": 0,  "cumulative": 1180 },
        { "date": "2026-06-30", "count": 5,  "cumulative": 1204 }
      ]
    },
    {
      "resource_type": "Observation",
      "total": 38910,
      "points": [ "… one entry per day …" ]
    }
  ]
}
```

| Field | Meaning |
|-------|---------|
| `total_resources` | Count of all non-deleted resources for the tenant (matches the dashboard's "Stored resources" stat). |
| `series[].total` | Current count for that resource type. |
| `series[].points[]` | Exactly `days` entries, one per UTC day in `[since, until]`. |
| `points[].count` | Resources whose `meta.lastUpdated` falls on that day. |
| `points[].cumulative` | Running total; ends at `series[].total` on the final day. |

### Counting semantics (read this)

The series buckets resources by the day of their **most recent** version
(`meta.lastUpdated`), then presents a running cumulative total seeded with the
count of resources last updated *before* the window. Consequences:

- The cumulative curve **ends exactly at the current total** — good for a
  growth-style chart whose endpoint matches the "Stored resources" stat.
- It is **not** a true historical creation-date curve. A resource updated today
  contributes to today's bucket, not its original creation day. A
  creation-time-accurate series would require aggregating `resource_history` and
  is intentionally out of scope for v1.

Backends that cannot bucket by time (e.g. S3) return a flat series at the
current total (no time resolution available). SQLite, PostgreSQL, and MongoDB —
and composite configurations layered on them — return real daily buckets.

---

## `GET /console/metrics/activity`

Weekly-rhythm **activity heatmap** — write operations (resource versions:
creates, updates, deletes) bucketed by UTC weekday and hour, for the dashboard's
"Activity" card.

### Query parameters (all optional)

| Param | Default | Notes |
|-------|---------|-------|
| `days` | `30` | Rolling window, clamped to `1..=365`. Buckets aggregate across the window (e.g. all Mondays-at-09:00 in the last 30 days). |

### Response `200 OK`

```json
{
  "tenant": "acme-health",
  "generated_at": "2026-06-30T12:04:51.654321+00:00",
  "source": "writes",
  "window": { "days": 30, "since": "2026-06-01", "until": "2026-06-30" },
  "total": 14302,
  "max_cell": 412,
  "cells": [
    { "weekday": 0, "hour": 0, "count": 3 },
    { "weekday": 0, "hour": 1, "count": 0 },
    "… 168 entries total, dense and zero-filled …",
    { "weekday": 6, "hour": 23, "count": 11 }
  ]
}
```

| Field | Meaning |
|-------|---------|
| `cells` | **Dense** 7×24 = 168 entries, ascending by `weekday` then `hour` — no gap-filling needed client-side. |
| `weekday` | `0` = Sunday … `6` = Saturday (matches JS `Date.getDay()`). |
| `hour` | `0` … `23`, UTC. |
| `count` | Write operations in that weekday/hour bucket, summed across the window. |
| `max_cell` | Largest single-cell count, for colour scaling. |
| `source` | Where the data came from. Currently always `"writes"` (from `resource_history`). |

### Source semantics

This version reflects **writes only** — every row in `resource_history` is one
create/update/delete. Reads and searches are not recorded there. The `source`
field is part of the contract so an `AuditEvent`-backed `"all-operations"` source
(including reads, when the audit DB sink is enabled) can be added later without
changing the response shape. Backends that cannot bucket by time (e.g. S3) return
an all-zero grid.

---

## `GET /console/metrics/traffic`

Windowed request throughput, latency percentiles, error rate, and a sparkline —
the "Traffic & Latency" panel. Derived from an **in-process rolling request log**
fed by the request middleware, so it needs no Prometheus deployment.

> **🔒 Admin endpoint** — server-wide traffic across all tenants. Requires a valid
> token **and** a `system/*.r` scope; other tokens get `403`.

### Query parameters (all optional)

| Param | Default | Notes |
|-------|---------|-------|
| `window` | `3600` | Look-back in seconds, clamped to `60..=86_400`. |
| `tenant` | (all) | Restrict to a single tenant id. |

### Response `200 OK`

```json
{
  "generated_at": "2026-06-30T12:04:51+00:00",
  "window_seconds": 3600,
  "covered_seconds": 3600,
  "sample_count": 12483,
  "requests_per_second": 3.47,
  "latency_ms": { "p50": 5.2, "p95": 18.0, "p99": 42.1 },
  "error_rate": 0.0041,
  "status_classes": { "2xx": 12000, "3xx": 40, "4xx": 438, "5xx": 5 },
  "series": [ { "offset_seconds": 3600, "requests_per_second": 3.1, "p95_ms": 17.0 }, "… 30 buckets, oldest first …" ]
}
```

> **Caveat:** the window is in-process — it **resets on restart** and retains at
> most the most recent ~20 k requests (`covered_seconds` reports the true span).
> For long-horizon/durable history, scrape the Prometheus `/metrics` histogram
> instead. `error_rate` counts 5xx responses.

---

## `GET /console/metrics/resource-distribution`

Every stored resource type with its count, busiest first — the "resource
composition" treemap.

### Query parameters (all optional)

| Param | Default | Notes |
|-------|---------|-------|
| `top` | `20` | Keep the N largest types; the remainder collapses into a single `other` bucket (with a `types` count). Clamped to `1..=100`. |

### Response `200 OK`

```json
{
  "tenant": "acme-health",
  "generated_at": "2026-06-30T12:04:51+00:00",
  "total_resources": 61423,
  "distinct_types": 47,
  "types": [
    { "resource_type": "Observation", "count": 38910 },
    { "resource_type": "Claim",       "count": 8430 },
    { "resource_type": "other",       "count": 1240, "types": 27 }
  ]
}
```

---

## `GET /console/metrics/tenants`

Per-tenant comparison — authoritative stored-resource counts joined with windowed
traffic. Sorted by resource count, busiest first.

> **🔒 Admin endpoint** — this **enumerates every tenant** and its data volume, so
> it is the most sensitive console endpoint. Requires a valid token **and** a
> system-context scope `system/*.r`; ordinary `user/*` / `patient/*` tokens (even
> wildcard ones) get `403`.

### Query parameters (all optional)

| Param | Default | Notes |
|-------|---------|-------|
| `window` | `3600` | Traffic look-back in seconds, clamped to `60..=86_400`. |

### Response `200 OK`

```json
{
  "generated_at": "2026-06-30T12:04:51+00:00",
  "window_seconds": 3600,
  "tenant_count": 3,
  "tenants": [
    { "tenant": "acme-health", "resources": 48210, "requests_per_second": 2.9, "p95_ms": 19.0, "error_rate": 0.003 },
    { "tenant": "northwind",   "resources": 11013, "requests_per_second": 0.4, "p95_ms": 12.0, "error_rate": 0.0 },
    { "tenant": "sandbox",     "resources": 2200,  "requests_per_second": 0.0, "p95_ms": 0.0,  "error_rate": 0.0 }
  ]
}
```

| Field | Meaning |
|-------|---------|
| `resources` | Authoritative count from the backend (this aggregate **spans tenants** by design — an operator view). |
| `requests_per_second` / `p95_ms` / `error_rate` | Best-effort, from the in-process request log, keyed by `X-Tenant-ID` (empty → `default`). A tenant with stored data but no recent traffic reports zeroes; a tenant seen only in traffic appears with `resources: 0`. |

---

## Prometheus

The `/metrics` endpoint is public (unauthenticated, for Prometheus scraping) and
exposes only tenant-agnostic series:

```
http_requests_total{service="hfs",method="GET",route="/{resource_type}/{id}",status="200"}  4213
http_request_duration_seconds{service="hfs",...}                                            ...
uptime_seconds{service="hfs"}                                                                14691.53
```

Per-tenant stored-resource counts are **deliberately NOT exported to
`/metrics`**. Tenant is never a metric label (see the rule in
`crates/observability/src/middleware.rs` and `CLAUDE.md`): because `/metrics` is
unauthenticated, a tenant-labelled gauge would leak the identity and
resource counts of every tenant to any anonymous scraper — the same cross-tenant
data the `tenants` endpoint gates behind a `system/*.r` admin scope. Per-tenant
counts are available only via the authenticated `resource-counts` /
`resource-distribution` JSON endpoints above.

---

## Manual verification

> With auth enabled (`HFS_AUTH_*`), every call below except `uptime` needs an
> `Authorization: Bearer <token>` header, and the admin endpoints (`tenants`,
> `traffic`) additionally need that token to carry a `system/*.r` scope. The
> default dev server runs with auth disabled, so these work as-is.

```bash
# Start the server (default: R4, SQLite, port 8080)
cargo run --bin hfs

# Seed a couple of resources
curl -s -X POST localhost:8080/Patient -H 'Content-Type: application/fhir+json' \
  -d '{"resourceType":"Patient","name":[{"family":"Smith"}]}'

# Uptime
curl -s localhost:8080/console/metrics/uptime | jq

# Resource counts (default 8 types, 30 days)
curl -s 'localhost:8080/console/metrics/resource-counts?types=Patient&days=7' \
  -H 'X-Tenant-ID: default' | jq

# Activity heatmap (write ops by weekday/hour, last 30 days)
curl -s 'localhost:8080/console/metrics/activity?days=30' \
  -H 'X-Tenant-ID: default' | jq '{source, total, max_cell, cells: (.cells | length)}'

# Traffic & latency (last hour) — issue a few requests first so the log fills
curl -s 'localhost:8080/console/metrics/traffic?window=3600' | jq 'del(.series)'

# Resource composition treemap (top 15 + "other")
curl -s 'localhost:8080/console/metrics/resource-distribution?top=15' \
  -H 'X-Tenant-ID: default' | jq

# Per-tenant comparison
curl -s 'localhost:8080/console/metrics/tenants' | jq

# Prometheus (public, tenant-agnostic: request counters, latency, uptime)
curl -s localhost:8080/metrics | grep -E 'http_requests_total|uptime_seconds'
```
