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

**Analytics & Tooling**

- [SQL on FHIR](crates/sof/README.md) — CLI and HTTP server
- [FHIRPath expression engine](crates/fhirpath/README.md) — CLI and HTTP server
- [Python bindings (pysof)](crates/pysof/README.md)

---

## 🔨 Now — Actively In Progress

Work that is currently underway or planned for the near term.

| Area | Item | Status |
|------|------|--------|
| **Standards** | [Terminology](https://github.com/HeliosSoftware/hfs/discussions/54) | 🟡 In progress |
| **Standards** | FHIR Validation engine | 🔵 Design |
| **Standards** | [FHIR Subscriptions — Topic-based notification support](https://github.com/HeliosSoftware/hfs/discussions/59) | 🔵 Design |
| **Analytics** | [SQL on FHIR](https://sql-on-fhir.org/ig/latest/) — HFS integration and operations update | 🔵 Design |
| **Documentation** | [Project documentation website](https://github.com/HeliosSoftware/hfs/tree/docs/book-updates) | 🟡 In progress |

### Discussion Documents

We are actively developing community discussion documents on the following topics to gather feedback before implementation begins. These will be published as GitHub Discussions:

- **[Terminology](https://github.com/HeliosSoftware/hfs/discussions/54)** — Defining how code systems, value sets, and concept maps will be managed
- **Validation** — Establishing the strategy for StructureDefinition-based validation and profiles
- **[Subscriptions](https://github.com/HeliosSoftware/hfs/discussions/59)** — Defining the approach for topic-based notification support
- **SQL on FHIR** — Standalone `sof-cli` and `sof-server` already ship today; this effort integrates SQL on FHIR directly into HFS and brings all [SQL on FHIR operations](https://sql-on-fhir.org/ig/latest/operations.html) up to date with current spec progress

---

## 🗺️ Next — Up After Current Work

These items are well-understood and will be picked up once current work completes.

### FHIR Server Capabilities

- **Bulk Data API** — Import and export (`$export` / `$import` operations)
- **Persistence-layer audit events** — Wire audit logging for bulk export, purge, and reindex operations (audit functions exist, pending REST endpoints)

### Developer Experience

- **Administrative UI** — Web-based management console for server configuration and monitoring
- **MCP Server for FHIR API** — Model Context Protocol integration for the FHIR REST API
- **MCP Server for SQL on FHIR** — Model Context Protocol integration for analytics workflows
- **Deployment Cookbooks** — Reference architectures and deployment templates covering standalone development servers, single-node production setups, composite storage configurations, and full CQRS architectures on AWS, Azure, and GCP

---

## 🔭 Later — On the Horizon

Longer-term ideas we are exploring. These are not yet committed and may evolve significantly based on community input.

### FHIR Server Capabilities

- **SMART on FHIR** — Full launch framework (standalone launch, EHR launch) and fine-grained scoped access

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

---

## Status Legend

| Icon | Meaning |
|------|---------|
| 🟡 | In progress — actively being developed |
| 🔵 | Design — in planning or community discussion phase |

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
| **Bulk Data API not exposed via REST** | Appendix I "Bulk data processing" — persistence-layer traits exist across all backends but no `$export`/`$import` REST endpoints. The book notes bulk ingestion is important for hybrid architectures (Ch. 8). | 🗺️ Next |

#### Moderate

| Gap | Book Reference | Current Status |
|-----|---------------|----------------|
| **No `$everything` operation** | Ch. 6 — FHIR-native systems where "all queries are FHIR requests" expect standard patient-centric retrieval. | Not planned |
| **No interceptor/hook framework** | Ch. 1 "Platform Illusion," Appendix I "Proxy/intercept layer" — organizations need to inject business validation, governance rules, and custom logic into the CRUD pipeline. | Not planned |
| **No Provenance tracking** | Appendix I "Provenance tracking" — important for audit, trust, and multi-source systems. AuditEvent (who did what) is not the same as Provenance (where data came from). | Not planned |
| **No performance metrics** | Ch. 8 "Performance" — the book insists on POC benchmarking. No Prometheus/OpenTelemetry integration to support this. | Not planned |
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
