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

- [FHIR Validation engine](crates/fhir-validator/) — FHIR Schema based structural and profile validation with a StructureDefinition→schema converter, embedded core packages (R4–R6), FHIRPath invariant evaluation, and terminology binding checks. Exposed as `$validate` on the REST API, as opt-in write-path validation (`HFS_VALIDATION_MODE=log|enforce`), and as the `validator-cli` tool
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
| **Developer Experience** | Administrative UI — web-based management console for server configuration and monitoring | 🟡 In progress |
| **Terminology** | [Administrative UI for HTS](crates/hts/README.md) — web-based management console for the terminology server: CodeSystem / ValueSet / ConceptMap browsing, terminology import and bootstrap sync monitoring, and operation testing | 🟡 In progress |
| **Deployment** | Cluster support — multi-instance deployment behind a load balancer, with cluster-safe state | 🟡 In progress |
| **Documentation** | [Project documentation website](https://github.com/HeliosSoftware/hfs/tree/docs/book-updates) | 🟡 In progress |

### Discussion Documents

We are actively developing community discussion documents on the following topics to gather feedback before implementation begins. These are published as GitHub Discussions:

- **[Validation](https://github.com/HeliosSoftware/hfs/discussions/215)** — Establishing the strategy for StructureDefinition-based validation and profiles
- **[Clustered / multi-instance deployment](https://github.com/HeliosSoftware/hfs/discussions/223)** — How HFS should behave when run as multiple instances behind a load balancer, and where the boundary lies between state that can safely stay in process memory and state that must be externalized to shared infrastructure. Considerations include: cluster-aware WebSocket Subscription delivery, where connected clients are tracked in process memory today ([#170](https://github.com/HeliosSoftware/hfs/issues/170)); database-backed SQL-on-FHIR export job state, so status URLs survive restarts and are visible across instances ([#169](https://github.com/HeliosSoftware/hfs/issues/169)); and per-instance observability (Prometheus `/metrics`, OTLP traces) with correct tenant isolation when resource data comes from a shared database but traffic metrics are per-instance ([#150](https://github.com/HeliosSoftware/hfs/issues/150)). Authentication is *not* on this list: `helios-auth` validates tokens locally and holds no cross-instance state ([#205](https://github.com/HeliosSoftware/hfs/issues/205))

---

## 🗺️ Next — Up After Current Work

These items are well-understood and will be picked up once current work completes.

### FHIR Server Capabilities

- **Persistence-layer audit events** ([#168](https://github.com/HeliosSoftware/hfs/issues/168)) — Wire audit logging for bulk export, purge, and reindex operations (audit functions exist, pending REST endpoints)
- **Database-backed SQL-on-FHIR export job state** ([#169](https://github.com/HeliosSoftware/hfs/issues/169)) — Replace the in-memory job controller (`InMemoryController` in `crates/rest/src/export/`) behind the async `$sql-export` operation with database-backed job state, following the pattern already used by Bulk Data `$export` (whose job store shares the primary database). Today, job status, tenant ownership, progress, and cancellation state live only in process memory: a server restart invalidates every in-flight and completed status URL (undermining the spec's 24-hour manifest validity), and a second instance cannot see jobs submitted to the first. ⚠️ **Until this change is complete, deployments serving SQL-on-FHIR async operations must not be clustered** — run a single HFS instance for these endpoints, or pin them to one instance behind the load balancer. Synchronous `$sql-run` and Bulk Data `$export` are unaffected.
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
- **PostgreSQL + Citus for horizontal scale** ([#816](https://github.com/HeliosSoftware/hfs/issues/816)) — Distribute the PostgreSQL backend's tables by `tenant_id` using [Citus](https://www.citusdata.com/), so tenant-sharded deployments can scale out across workers instead of vertically. Community testing against a small Citus cluster (discussion [#705](https://github.com/HeliosSoftware/hfs/discussions/705)) found the existing schema already tenant-aware enough for colocation and single-shard routing; the known work is changing `search_index`'s primary key from `(id)` to `(tenant_id, id)` and providing a Citus-aware migration for the `resource_fts` trigger. Also under consideration alongside the ClickHouse item above: PostgreSQL with [pg_clickhouse](https://clickhouse.com/docs/products/managed-postgres/extensions/pg_clickhouse/introduction). Neither is committed — 👍 the issue to signal interest.

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

> **Note:** This section was generated by AI analysis of the HFS codebase evaluated against the criteria in [*FHIR Architecture Decisions*](https://darrendevitt.com/fhir-architecture-decisions-book/) (v1.0, 2026) by Darren Devitt — an independent guide to selecting FHIR servers and choosing between facade, hybrid, and FHIR-native architectural approaches. The analysis was first produced in April 2026 and last refreshed against the codebase in September 2026; it should be updated as features ship.

### Context

Devitt's book defines nine key questions organizations must answer before choosing an architectural model and FHIR server (Chapter 3), describes three architectural models with variants (Chapters 4–6), and catalogues the components required beyond the server itself (Appendix I). The analysis below maps HFS capabilities against these criteria.

### Strengths

| Area | Assessment |
|------|------------|
| **Multi-tenancy** | Tenant-first design with logical isolation at the query level, header- or URL-path-based tenant routing, and a tenant-maintenance page in the web UI. The book identifies multi-tenancy as a potential deal breaker (Ch. 3). |
| **Multi-backend persistence** | SQLite, PostgreSQL, MongoDB, S3, Elasticsearch — supports the on-prem and cloud deployment flexibility the book emphasizes (Ch. 8 "Data location"). |
| **Profile validation on write** | Ch. 1 Fig 1.1 shows "FHIR validation + Profile validation" as a core server responsibility on create; Ch. 6 warns that without it, data quality degrades in FHIR-native systems. [`helios-fhir-validator`](crates/fhir-validator/) validates structure, `meta.profile` claims, FHIRPath invariants, and required bindings against embedded R4–R6 core packages. Exposed as `$validate` (type and instance level), as opt-in write-path enforcement (`HFS_VALIDATION_MODE=off|log|enforce`, default `off`), and as the `validator-cli` tool. |
| **Audit logging** | IHE BALP-compliant AuditEvent logging with pluggable sinks (file, database, CloudWatch, S3), covering REST, auth, and the `$export`, `$purge`, and `$reindex` operations. The book lists audit logging as "required for compliance" (Appendix I). |
| **SMART on FHIR authentication** | JWT/JWKS validation, SMART v2 scope enforcement at resource-type level, dedicated operation scopes for `$purge`, `$reindex`, and `$bulk-submit`, and IdP integration guides (Keycloak, Okta, Auth0, Entra ID). The book calls SMART on FHIR "the expected authorization method" (Ch. 3). See the patient-level gap below. |
| **Terminology service** | Complete standalone HTS with `$lookup`, `$expand`, `$validate-code`, `$subsumes`, `$translate`, `$closure`, bulk import (HL7 packages, SNOMED CT, LOINC, ICD-10-CM, RxNorm), and its own web UI ([`helios-hts-ui`](crates/hts-ui)) for browsing CodeSystems / ValueSets / ConceptMaps, exploring concepts, and running imports. HFS wires to it for `:in` / `:not-in` search and FHIRPath `memberOf()` / `subsumes()`. The book notes terminology services are "often required and commonly missed in early product releases" (Appendix I). |
| **Bulk data processing** | Appendix I "Bulk data processing". `$export` at system / Patient / Group level with `_typeFilter`, `_since`, and pre-signed S3 downloads; `$bulk-submit` for bulk ingestion (HFS as Data Consumer, with OAuth `private_key_jwt` and JWE-encrypted files); asynchronous `$sql-export` for tabular output. |
| **Subscriptions / eventing** | Topic-based FHIR Subscriptions (R5 backport) with rest-hook, WebSocket, email, and FHIR Messaging channels, plus startup rehydration of stored subscriptions. This is the change-notification mechanism the hybrid sync models (Ch. 5) depend on. |
| **SQL-on-FHIR / Analytics** | Full SQL-on-FHIR (3.0.0-ballot) with Parquet, CSV, NDJSON, JSON output, Python bindings, and `$sql-run` / `$sql-export` on the server. Addresses the "analytical queries" deal breaker (Ch. 3) and secondary use cases the book highlights. |
| **Observability / performance measurement** | Ch. 8 "Performance" — the book insists on POC benchmarking. [`helios-observability`](crates/observability) exposes a Prometheus `GET /metrics` endpoint (request counts, latency histograms, uptime) across all servers, with optional OTLP trace export. Per-type stored-resource counts back the web UI dashboard chart; tenant is never a metric label. |
| **Administrative web UI** | Server-rendered management console under `/ui` ([`helios-ui`](crates/ui)): dashboard, resource browser and schema-driven editor, visual search builder and saved queries, SearchParameter and CompartmentDefinition viewers, version history with diff, batch/transaction workspace, bulk export builder, SQL-on-FHIR query / view / export pages, and tenant maintenance. Reduces the "tooling around the server" burden the book catalogues in Appendix I. |
| **AI-assisted search** | `$nl-search` translates natural-language requests into FHIR search queries through a configurable LLM endpoint (`HFS_NL_SEARCH_*`), with per-user/tenant rate limits, a daily cap, and a UI pane. Supports the AI-driven secondary use the book identifies as a growing adoption driver (Ch. 3). |
| **Open-source flexibility** | Full source access and customization — the book identifies this as a key advantage of the open-source server category (Ch. 8). |

### Gaps

#### Critical

No outstanding critical gaps. Profile validation on write, flagged as critical in the April 2026 analysis, has shipped (see Strengths).

#### Significant

| Gap | Book Reference | Current Status |
|-----|---------------|----------------|
| **No patient-level access control** | Ch. 3 "Authorization" — `patient/*` and `user/*` scope contexts are parsed but treated the same as `system/*`: enforcement is at resource-type level only, and search results are not filtered by patient compartment. The `CompartmentDefinition` machinery needed for enforcement now exists (compartment search `GET /Patient/{id}/{type}`, a compartment membership tester in the UI). | 🔭 Later |

#### Moderate

| Gap | Book Reference | Current Status |
|-----|---------------|----------------|
| **No `$everything` operation** | Ch. 6 — FHIR-native systems where "all queries are FHIR requests" expect standard patient-centric retrieval. Partial substitutes exist: compartment search (`GET /Patient/{id}/Observation`) and `Patient/$export`. | Not planned |
| **No interceptor/hook framework** | Ch. 1 "Platform Illusion," Appendix I "Proxy/intercept layer" — organizations need to inject business validation, governance rules, and custom logic into the CRUD pipeline. HFS has internal Axum middleware and configurable write-path validation, but no pluggable extension point; reacting to writes is possible out-of-band via Subscriptions, and CDS Hooks types are provided as a library only. | Not planned |
| **No Provenance tracking** | Appendix I "Provenance tracking" — important for audit, trust, and multi-source systems. `Provenance` resources can be stored and searched like any other resource, but the server does not generate them; AuditEvent (who did what) is not the same as Provenance (where data came from). | Not planned |
| **No general rate limiting** | Ch. 3 Q5 "Data consumers" — external consumers with SLAs require throttling and burst protection. Only `$nl-search` is rate-limited (per user/tenant window plus a daily cap); the FHIR REST surface has no throttling and relies on an upstream gateway. | Not planned |
| **Single-instance state in some operations** | Ch. 8 "Scalability" — clustered deployment is supported for CRUD, search, and Bulk Data `$export`, but `$sql-export` job state ([#169](https://github.com/HeliosSoftware/hfs/issues/169)), WebSocket subscription delivery ([#170](https://github.com/HeliosSoftware/hfs/issues/170)), and `$reindex` status are held in process memory on one node. | 🟡 In progress ([discussion #223](https://github.com/HeliosSoftware/hfs/discussions/223)) |

#### Minor

| Gap | Book Reference | Current Status |
|-----|---------------|----------------|
| **No StructureMap / data mapping** | Appendix I "Data mapping layer" — essential for facade and hybrid architectures. FHIRPath extraction and SQL-on-FHIR flattening exist but no bidirectional transformation. | Not planned |
| **No de-identification** | Appendix I "Secondary use and testing" — needed for AI/analytics secondary use, a growing driver for FHIR adoption (Ch. 3). | Not planned |
| **No GraphQL API** | Ch. 6 — alternative query surface for FHIR-native consumers. | 🔭 Later |
| **No MCP server** | Not covered by the book; relevant to the AI secondary-use driver (Ch. 3). Design is converging in [RFC discussion #231](https://github.com/HeliosSoftware/hfs/discussions/231). | 🔵 Design |

### Architectural Fit

Using Devitt's framework (Chapters 4–7), HFS is currently best suited for:

- **FHIR-native Variant A** (single source of truth, greenfield) — strong fit: profile validation on the write path, multi-tenancy, audit, terminology, and an administrative UI
- **Hybrid Variant A** (sync-only, read-only FHIR server) — good fit: Subscriptions (rest-hook, WebSocket, email, messaging) and Bulk Data (`$export`, `$bulk-submit`) provide the sync pipeline in both directions
- **Analytics / AI secondary use** — strong fit: SQL-on-FHIR, Parquet, Python bindings, async `$sql-export`, and natural-language search

HFS is not yet ready for:

- **Hybrid Variant B** (sync with write-back facade) — no interceptor hooks to route writes to the upstream system; Subscriptions can notify the facade after the fact but cannot veto or redirect a write
- **FHIR-native Variant B/C** (mixed or distributed source of truth) — no server-generated Provenance to track which source each resource came from
- **Patient-facing applications** — no patient-level access control enforcement; a patient-context token can read any patient's data its resource-type scopes allow
