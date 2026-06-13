---
name: work-with-audit
description: Work on HFS audit logging. Use for FHIR AuditEvent, IHE BALP profiles, audit sinks (database/file/CloudWatch), path exclusions, and HFS_AUDIT_* configuration.
---

# Audit Logging

Use this when working in `helios-audit`. The crate emits FHIR `AuditEvent` records (IHE BALP profiles) for HFS interactions. It is consumed by `helios-rest`, `helios-hfs`, and `helios-persistence`; middleware records request/response activity and writes to a configured sink.

## Behavior

- Off by default (`HFS_AUDIT_BACKEND=none`). Choose a backend to enable auditing.
- BALP AuditEvents use the R4-family `AuditEvent` model. R5/R6 builds alias to R4B as the BALP baseline (R5/R6 restructured `AuditEvent`).
- Paths can be excluded (e.g. `/health`, `/metadata`) via `HFS_AUDIT_EXCLUDE_PATHS`.
- `AuditEvent.source.observer` defaults to `Device/hfs` (`HFS_AUDIT_SOURCE_OBSERVER`).

## Backends (`HFS_AUDIT_BACKEND`)

| Value | Sink |
|---|---|
| `none` (default) | disabled (`NullSink`) |
| `file` | NDJSON to `HFS_AUDIT_FILE_PATH` |
| `database` / `db` | `DatabaseSink` — `HFS_AUDIT_DATABASE_URL` (SQL); MongoDB via `HFS_AUDIT_MONGODB_DATABASE` |
| `cloudwatch` / `cloudwatch-logs` / `cwl` | CloudWatch Logs — requires `--features cloudwatch` |

## Environment

| Variable | Default | Description |
|---|---|---|
| `HFS_AUDIT_BACKEND` | `none` | Sink selection (see table) |
| `HFS_AUDIT_FILE_PATH` | none | File sink output path |
| `HFS_AUDIT_DATABASE_URL` | none | Database sink connection |
| `HFS_AUDIT_MONGODB_DATABASE` | none | MongoDB database name for the database sink |
| `HFS_AUDIT_EXCLUDE_PATHS` | none | Comma-separated paths to skip |
| `HFS_AUDIT_SOURCE_OBSERVER` | `Device/hfs` | `AuditEvent.source.observer` reference |
| `HFS_AUDIT_CLOUDWATCH_LOG_GROUP` / `_LOG_STREAM` / `_REGION` | none | CloudWatch Logs target |
| `HFS_AUDIT_S3_BUCKET` / `_PREFIX` / `_REGION` / `_VALIDATE_BUCKETS` | none | S3-related sink fields |

## Features

`default = ["R4"]`; `R4`/`R4B`/`R5`/`R6` select the FHIR `AuditEvent` model (R5/R6 pull the R4B baseline). `cloudwatch` enables the CloudWatch Logs sink.

## Key API

`AuditConfig` / `AuditBackend`, `AuditEventBuilder`, `AuditMiddlewareState` / `AuditAgent` / `AuditResponseContext`, `AuditSink` (`DatabaseSink`, `FileSink`, `CloudWatchLogsSink`, `NullSink`), `ExclusionFilter`, `AuditCorrelation` / `BundleAuditEntry`, `AuditAction`.

## Code map / tests

`config.rs`, `middleware.rs`, `balp.rs`, `builder.rs`, `sinks/` (database, file, cloudwatch, null), `exclusion.rs`, `correlation.rs`, `lifecycle.rs`, `patient.rs`. CI: `.github/workflows/audit-events.yml`. Unit tests inline in `src/`; integration tests under `crates/audit/tests/` where present.
