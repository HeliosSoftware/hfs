# Appendix D — Changelog

The Helios FHIR Server does not maintain a `CHANGELOG.md` file. Per-release notes are published as **GitHub Releases** and are the authoritative source for version history:

**[github.com/HeliosSoftware/hfs/releases](https://github.com/HeliosSoftware/hfs/releases)**

---

## What Has Shipped

The following capabilities are available in the current release. Source: [ROADMAP.md](https://github.com/HeliosSoftware/hfs/blob/main/ROADMAP.md).

### FHIR REST Server

- FHIR REST API with CRUD operations, search, history, and batch/transaction bundle support
- Multi-tenancy via header (`X-Tenant-ID`) or URL path
- ETag versioning and conditional updates
- CORS support and request ID tracking
- FHIR CapabilityStatement (`/metadata`)

### Persistence Backends

| Backend | Mode |
|---------|------|
| SQLite | Primary store (default) |
| SQLite + Elasticsearch | SQLite for CRUD, Elasticsearch for search |
| PostgreSQL | Primary store |
| PostgreSQL + Elasticsearch | PostgreSQL for CRUD, Elasticsearch for search |
| MongoDB | Primary store |
| MongoDB + Elasticsearch | MongoDB for CRUD, Elasticsearch for search |
| S3 | Primary store (CRUD, versioning, history, bulk ops; no search) |
| S3 + Elasticsearch | S3 for CRUD, Elasticsearch for search |

### Analytics and Tooling

- **SQL-on-FHIR** — `sof-cli` (CSV, JSON, NDJSON, Parquet) and `sof-server` HTTP API
- **FHIRPath engine** — `fhirpath-cli` and `fhirpath-server` (FHIRPath 3.0.0-ballot, 100+ functions)
- **Python bindings** — `pysof` on PyPI with streaming, chunked processing, and Parquet export
- **CDS Hooks types** — `helios-cds-hooks` with all 10 hooks and typed contexts

### Current Version

```bash
cargo pkgid helios-hfs   # shows current version
```

Or check the workspace `Cargo.toml`:

```bash
grep '^version' Cargo.toml
```

---

## What Is In Progress

| Area | Item |
|------|------|
| Compliance | Audit logging (AuditEvent resource support) |
| Standards | FHIR Validation engine |
| Standards | Authentication & Authorization (SMART on FHIR) |
| Documentation | Project documentation website |

---

## What Is Coming Next

| Area | Item |
|------|------|
| FHIR Server | Bulk Data API (`$export` / `$import`) |
| FHIR Server | FHIR Subscriptions (topic-based notifications) |
| FHIR Server | Terminology Server (`$lookup`, `$expand`, `$translate`) |
| FHIR Server | SMART on FHIR launch framework |
| Persistence | Cassandra as a primary store |
| Persistence | ClickHouse as a primary store |
| Developer Experience | Administrative UI |
| Developer Experience | MCP Server for FHIR API |
| Developer Experience | MCP Server for SQL-on-FHIR |
| Developer Experience | Deployment cookbooks (AWS, Azure, GCP) |

See the full [ROADMAP.md](https://github.com/HeliosSoftware/hfs/blob/main/ROADMAP.md) for details and discussion links.
