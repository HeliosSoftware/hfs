# Helios FHIR Server — Roadmap

> This document outlines the development direction for the Helios FHIR Server. It is organized into three horizons — **Now**, **Next**, and **Later** — to set expectations without overpromising timelines. Items may shift between horizons as priorities evolve based on community feedback, production needs, and contributor availability.
>
> Want to influence the roadmap? Join our [weekly developer meeting](#community) or comment on a [GitHub Discussion](https://github.com/HeliosSoftware/hfs/discussions).

---

## ✅ Shipped

These capabilities are available today in the current release.

**Helios FHIR Server**

- [FHIR REST API server](crates/hfs/README.md) with CRUD operations, search, history, and batch/transaction support

**Persistence**

- [SQLite as a primary store](crates/persistence/README.md#sqlite-default)
- [SQLite as a primary store with Elasticsearch as a query secondary](crates/persistence/README.md#sqlite--elasticsearch)
- [PostgreSQL as a primary store](crates/persistence/README.md#postgresql)
- [PostgreSQL as a primary store with Elasticsearch as a query secondary](crates/persistence/README.md#postgresql--elasticsearch)
- [MongoDB as a primary store](crates/persistence/README.md#mongodb)
- [MongoDB as a primary store with Elasticsearch as a query secondary](crates/persistence/README.md#mongodb--elasticsearch)
- [S3 as a primary store](crates/persistence/README.md#s3)
- [S3 as a primary store with Elasticsearch as a query secondary](crates/persistence/README.md#s3--elasticsearch)

**Authentication & Authorization**

- [SMART on FHIR authentication, authorization, and IdP integration](https://github.com/HeliosSoftware/hfs/discussions/45)

**Clinical Decision Support**

- [CDS Hooks](crates/cds-hooks/README.md) — HL7 CDS Hooks v3.0.0 protocol types, all 10 hook contexts, and async `CdsHooksService` trait

**Compliance**

- [Audit logging (AuditEvent resource support)](crates/audit/README.md) — REST, auth, and lifecycle audit events with IHE BALP profiles

**Terminology**

- [Terminology service (HTS) — SQLite and PostgreSQL backends](crates/hts/README.md) — Standalone FHIR Terminology Server with `$lookup`, `$expand`, `$validate-code`, `$subsumes`, `$translate`, `$closure`, and bulk import for HL7 NPM packages, SNOMED CT RF2, LOINC, ICD-10-CM, and RxNorm

**Standards**

- [Bulk Data API — `$export`](crates/rest/README.md) — System / patient / group export with pre-signed S3 downloads
- [Bulk Data API — `$bulk-submit`](crates/rest/README.md) — Bulk ingestion

**Messaging**

- [FHIR Subscriptions](crates/subscriptions/README.md) — Topic-based notification support (R5 backport)

**Analytics & Tooling**

- [SQL on FHIR](crates/sof/README.md) — CLI and HTTP server
- [FHIRPath expression engine](crates/fhirpath/README.md) — CLI and HTTP server
- [Python bindings (pysof)](crates/pysof/README.md)

---

## 🔨 Now — Actively In Progress

Work that is currently underway or planned for the near term.

| Area | Item | Status |
|------|------|--------|
| **Standards** | FHIR Validation engine | 🔵 Design |
| **Developer Experience** | Administrative UI — web-based management console for server configuration and monitoring | 🔵 Design |
| **Documentation** | [Project documentation website](https://github.com/HeliosSoftware/hfs/tree/docs/book-updates) | 🟡 In progress |

### Discussion Documents

We are actively developing community discussion documents on the following topics to gather feedback before implementation begins. These will be published as GitHub Discussions:

- **Validation** — Establishing the strategy for StructureDefinition-based validation and profiles

---

## 🗺️ Next — Up After Current Work

These items are well-understood and will be picked up once current work completes.

### FHIR Server Capabilities

- **Persistence-layer audit events** ([#168](https://github.com/HeliosSoftware/hfs/issues/168)) — Wire audit logging for bulk export, purge, and reindex operations (audit functions exist, pending REST endpoints)
- **Database-backed SQL-on-FHIR export job state** ([#169](https://github.com/HeliosSoftware/hfs/issues/169)) — Replace the in-memory job controller (`InMemoryController` in `crates/rest/src/export/`) behind the async `$viewdefinition-export` / `$sqlquery-export` operations with database-backed job state, following the pattern already used by Bulk Data `$export` (whose job store shares the primary database). Today, job status, tenant ownership, progress, and cancellation state live only in process memory: a server restart invalidates every in-flight and completed status URL (undermining the spec's 24-hour manifest validity), and a second instance cannot see jobs submitted to the first. ⚠️ **Until this change is complete, deployments serving SQL-on-FHIR async operations must not be clustered** — run a single HFS instance for these endpoints, or pin them to one instance behind the load balancer. Synchronous `$viewdefinition-run` / `$sqlquery-run` and Bulk Data `$export` are unaffected.
- **Cluster-aware Subscription notification delivery** ([#170](https://github.com/HeliosSoftware/hfs/issues/170)) — Identified by the same review: the Subscriptions engine tracks connected WebSocket clients in process memory (`crates/subscriptions/src/channels/ws_manager.rs`), so in a multi-instance deployment only the instance holding a client's connection can deliver its notifications. Subscription resources themselves are database-backed; what's needed is a shared delivery channel (e.g., a message bus or pub/sub fan-out) so resource events on any instance reach clients connected to any other. Until then, WebSocket subscription delivery is single-instance / sticky-session only (rest-hook channels are unaffected).

### Developer Experience

- **MCP Server for FHIR API** — Model Context Protocol integration for the FHIR REST API
- **MCP Server for SQL on FHIR** — Model Context Protocol integration for analytics workflows
- **Deployment Cookbooks** — Reference architectures and deployment templates covering standalone development servers, single-node production setups, composite storage configurations, and full CQRS architectures on AWS, Azure, and GCP
- **Point internal build tools at hosted HTS** — Once `https://hts.heliossoftware.com` is available, switch `.github/workflows/inferno.yml` and any other internal build/CI tooling that needs a terminology service to point at the hosted endpoint (via `HFS_TERMINOLOGY_SERVER` / `FHIRPATH_TERMINOLOGY_SERVER`) 

---

## 🔭 Later — On the Horizon

Longer-term ideas we are exploring. These are not yet committed and may evolve significantly based on community input.

### FHIR Server Capabilities

- **SMART on FHIR** — Full launch framework (standalone launch, EHR launch) and fine-grained scoped access
- **GraphQL API** — [FHIR GraphQL](https://hl7.org/fhir/graphql.html) support for resource retrieval, search, and graph traversal as an alternative to the REST API

### SQL-on-FHIR & FHIRPath

Follow-ups to `resolve()` in SQL-on-FHIR, which today dereferences references against the input bundle (and their `contained` resources) and can optionally fetch from explicitly allowlisted, trusted remote servers:

- **Logical (identifier-based) reference resolution** — `resolve()` handles literal `Reference.reference` strings (relative `Type/id`, absolute URLs, and `#contained` fragments). References that identify their target only by a business `identifier` (no `reference` string) are not yet resolved.
- **Storage-backed and cross-run resolution cache for `resolve()`** ([#167](https://github.com/HeliosSoftware/hfs/issues/167)) — Resolve against server-stored resources, and persist fetched resources across runs, as a layer over the current in-bundle pool and per-run in-memory prefetch cache.
- **Async FHIRPath evaluator** — The FHIRPath engine is synchronous (executed under Rayon by the SQL-on-FHIR core), so remote `resolve()` is handled by an up-front prefetch pass rather than inline I/O. A general async evaluator would let effectful functions (inline remote resolution, terminology lookups) run during evaluation.
- **OAuth client-credentials for remote `resolve()`** — Trusted-server authentication is per-host bearer tokens today; a client-credentials grant flow would better suit servers fronted by OAuth.

### Advanced Persistence

- Cassandra as a primary store
- Cassandra with Elasticsearch as a query secondary
- ClickHouse as a primary store
- Neo4j as a primary store
- PostgreSQL with Neo4j as a graph query secondary

### Persistence Advisor

An intelligent recommendation engine for storage configuration:

- Analyze a FHIR query and recommend an optimal persistence configuration
- Leverage historical benchmark data to inform recommendations
- Web UI for interactive configuration guidance

### Build & Dependencies

- **Upgrade `lru` to the latest version** — `helios-sof` uses `lru` for the
  streaming remote-`resolve()` cross-chunk cache. It is currently pinned to `0.12`
  to match `aws-sdk-s3`, which pins `lru = "^0.12"` as a non-optional dependency
  (pulled by the optional `s3` feature in `helios-persistence` / `helios-rest`).
  Matching the SDK keeps a single `lru` version compiled across all feature
  combinations. The latest `lru` is 0.18; raising `helios-sof`'s pin would compile
  two `lru` versions whenever `s3` is enabled, because the AWS SDK's `^0.12` pin (a
  third-party crate we don't control) is semver-incompatible with 0.18. Bump
  `helios-sof` to the latest `lru` once the AWS SDK updates its own pin, collapsing
  back to a single version. Tracked so the pin is re-evaluated on the next AWS SDK
  upgrade.

---

## Status Legend

| Icon | Meaning |
|------|---------|
| 🟡 | In progress — actively being developed |
| 🔵 | Design — in planning or community discussion phase |

---

## 🐛 Open GitHub Issues

Tracked work items currently open on the [issue tracker](https://github.com/HeliosSoftware/hfs/issues):

| Issue | Title |
|-------|-------|
| [#170](https://github.com/HeliosSoftware/hfs/issues/170) | Cluster-aware Subscription notification delivery |
| [#169](https://github.com/HeliosSoftware/hfs/issues/169) | Database-backed SQL-on-FHIR export job state |
| [#168](https://github.com/HeliosSoftware/hfs/issues/168) | Persistence-layer audit events for bulk export, purge, and reindex |
| [#167](https://github.com/HeliosSoftware/hfs/issues/167) | FHIRPath resolve(): add storage-backed resolution for server-stored resources |
| [#145](https://github.com/HeliosSoftware/hfs/issues/145) | SoF export: re-sign S3 pre-signed download URLs on each manifest poll |
| [#144](https://github.com/HeliosSoftware/hfs/issues/144) | SoF export: clean up partial results when a job is cancelled |

---

## Community

We welcome contributors and feedback at every level — from opening issues to joining design discussions.

- **📋 GitHub Discussions:** [github.com/HeliosSoftware/hfs/discussions](https://github.com/HeliosSoftware/hfs/discussions)
- **🐛 Issues:** [github.com/HeliosSoftware/hfs/issues](https://github.com/HeliosSoftware/hfs/issues)
- **🗓️ Weekly Developer Meeting:** — Open to all. We review roadmap progress, discuss design decisions, and plan upcoming work. Details and updates are posted to [this GitHub Discussion](https://github.com/HeliosSoftware/hfs/discussions/40).
- **💼 LinkedIn Group:** [linkedin.com/groups/8618077](https://www.linkedin.com/groups/8618077/) — Over 2,500 members

### How to Get Involved

1. **Comment on a discussion document** — Help shape the design of upcoming features
2. **Claim a roadmap item** — Open an issue or comment on an existing one to signal interest
3. **Join the weekly call** — Introduce yourself and find out where help is needed
4. **Contribute code** — See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines

---

*This roadmap is a living document. It does not represent a commitment or guarantee to deliver any feature by any particular date. Items may be reprioritized based on community needs, production feedback, and resource availability.*

---

## 📊 Gap Analysis — "FHIR Architecture Decisions" by Darren Devitt

> **Note:** This section was generated by AI analysis of the HFS codebase evaluated against the criteria in [*FHIR Architecture Decisions*](https://darrendevitt.com/fhir-architecture-decisions-book/) (v1.0, 2026) by Darren Devitt — an independent guide to selecting FHIR servers and choosing between facade, hybrid, and FHIR-native architectural approaches. The analysis reflects the state of the codebase as of April 2026 and should be updated as features ship.

### Context

Devitt's book defines nine key questions organizations must answer before choosing an architectural model and FHIR server (Chapter 3), describes three architectural models with variants (Chapters 4–6), and catalogues the components required beyond the server itself (Appendix I). The analysis below maps HFS capabilities against these criteria.

### Strengths

| Area | Assessment |
|------|------------|
| **Multi-tenancy** | Tenant-first design with logical isolation at the query level. The book identifies multi-tenancy as a potential deal breaker (Ch. 3). |
| **Multi-backend persistence** | SQLite, PostgreSQL, MongoDB, S3, Elasticsearch — supports the on-prem and cloud deployment flexibility the book emphasizes (Ch. 8 "Data location"). |
| **Audit logging** | IHE BALP-compliant AuditEvent logging with pluggable sinks (file, database, CloudWatch). The book lists audit logging as "required for compliance" (Appendix I). |
| **SMART on FHIR authentication** | JWT/JWKS validation, SMART v2 scope enforcement at resource-type level, IdP integration guides (Keycloak, Okta, Auth0, Entra ID). The book calls SMART on FHIR "the expected authorization method" (Ch. 3). |
| **Terminology service** | Complete standalone HTS with `$lookup`, `$expand`, `$validate-code`, `$subsumes`, `$translate`, `$closure`. The book notes terminology services are "often required and commonly missed in early product releases" (Appendix I). |
| **SQL-on-FHIR / Analytics** | Full SQL-on-FHIR v2 with Parquet, CSV, NDJSON, JSON output and Python bindings. Addresses the "analytical queries" deal breaker (Ch. 3) and secondary use cases the book highlights. |
| **Open-source flexibility** | Full source access and customization — the book identifies this as a key advantage of the open-source server category (Ch. 8). |

### Gaps

#### Critical

| Gap | Book Reference | Current Status |
|-----|---------------|----------------|
| **No profile validation on write** | Ch. 1 Fig 1.1 shows "FHIR validation + Profile validation" as a core server responsibility on create. Ch. 6 warns that without it, data quality degrades in FHIR-native systems. | 🔵 Design |

#### Significant

| Gap | Book Reference | Current Status |
|-----|---------------|----------------|
| **No patient-level access control** | Ch. 3 "Authorization" — SMART scopes are parsed but `patient/*` and `user/*` contexts are not enforced. Search results are not filtered by patient compartment. | 🔭 Later |
| **Bulk Data API** | Appendix I "Bulk data processing" — `$export` (system / patient / group) is exposed via the REST layer with an embedded SQLite-backed worker pool by default and an optional Postgres + S3 multi-instance topology. `$bulk-submit` (ingestion) is also available. | ✅ Shipped |

#### Moderate

| Gap | Book Reference | Current Status |
|-----|---------------|----------------|
| **No `$everything` operation** | Ch. 6 — FHIR-native systems where "all queries are FHIR requests" expect standard patient-centric retrieval. | Not planned |
| **No interceptor/hook framework** | Ch. 1 "Platform Illusion," Appendix I "Proxy/intercept layer" — organizations need to inject business validation, governance rules, and custom logic into the CRUD pipeline. | Not planned |
| **No Provenance tracking** | Appendix I "Provenance tracking" — important for audit, trust, and multi-source systems. AuditEvent (who did what) is not the same as Provenance (where data came from). | Not planned |
| **Performance metrics** | Ch. 8 "Performance" — the book insists on POC benchmarking. | **Implemented.** `helios-observability` exposes a Prometheus `GET /metrics` endpoint (request counts/latency histograms, uptime) across all servers, with optional OTLP trace export (`otel` feature). Per-type stored-resource counts (default tenant) back the web UI's "FHIR resources over time" dashboard chart via an authenticated console endpoint; tenant is never a metric label. |
| **No rate limiting** | Ch. 3 Q5 "Data consumers" — external consumers with SLAs require throttling and burst protection. | Not planned |

#### Minor

| Gap | Book Reference | Current Status |
|-----|---------------|----------------|
| **No StructureMap / data mapping** | Appendix I "Data mapping layer" — essential for facade and hybrid architectures. FHIRPath extraction exists but no bidirectional transformation. | Not planned |
| **No de-identification** | Appendix I "Secondary use and testing" — needed for AI/analytics secondary use, a growing driver for FHIR adoption (Ch. 3). | Not planned |

### Architectural Fit

Using Devitt's framework (Chapters 4–7), HFS is currently best suited for:

- **FHIR-native Variant A** (single source of truth, greenfield) — strong fit once profile validation ships
- **Hybrid Variant A** (sync-only, read-only FHIR server) — requires Subscriptions or Bulk Data REST endpoints for the sync pipeline
- **Analytics / AI secondary use** — strong SQL-on-FHIR and Parquet support today

HFS is not yet ready for:

- **Hybrid Variant B** (sync with write-back facade) — no interceptor hooks, no Subscriptions
- **FHIR-native Variant B/C** (mixed or distributed source of truth) — no Provenance, no Subscriptions
- **Patient-facing applications** — no patient-level access control enforcement
