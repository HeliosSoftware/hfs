# helios-audit

FHIR `AuditEvent` logging for the Helios FHIR Server, with [IHE BALP](https://profiles.ihe.net/ITI/BALP/) profile selection for REST, authentication, and persistence-layer events.

## Overview

This crate records security, privacy, and operational activity as typed FHIR `AuditEvent` resources. It plugs into HFS at four layers:

1. **REST pipeline** — FHIR interactions (CRUD, search, history)
2. **Auth pipeline** — token validation, authorization grants and denials
3. **Persistence operations** — bulk export/import, purge, reindex (via `helios-persistence` integration)
4. **Server lifecycle** — startup (with configuration snapshot) and graceful shutdown

Additionally, batch and transaction bundles produce **correlated per-entry audit events** linked by a shared bundle ID.

- **FHIR-Native Audit Records**: Builds real `AuditEvent` resources using `helios-fhir`
- **IHE BALP Profile Selection**: Chooses BALP read/create/update/delete/query/auth profiles based on action and patient context
- **Interaction-Aware Classification**: Detects FHIR query/search/history interactions instead of mapping by HTTP verb alone
- **Pluggable Sinks**: Choose where audit events go — local files, your FHIR database, or AWS CloudWatch Logs
- **Axum Middleware**: Captures FHIR REST interactions after the request completes
- **Auth Bridge**: Adapts `helios_auth::AuditEventSink` into full FHIR `AuditEvent` records, including authorization grants
- **Lifecycle Events**: Records server startup (with storage backend, FHIR version, auth/audit config) and shutdown
- **Rich Entity Context**: Attach arbitrary key-value details, custom entities, and non-REST event types to audit events
- **Bundle Correlation**: Link per-entry audit events within a batch or transaction by a shared correlation ID
- **Patient Resolution**: Resolves patient context from resource paths, request bodies, and search parameters
- **Write Context Enrichment**: Supports response-extension context for create/update/patch IDs and patient references
- **Durable File Writes**: Flushes file sink writes after each recorded event
- **Fire-and-Forget Semantics**: Audit failures are logged via `tracing`, never returned to the caller

## Audit Sinks

All audit events flow through an `AuditSink` implementation. Set `HFS_AUDIT_BACKEND` to choose which sink is active. Only one sink is active at a time.

### `NullSink` — disabled (`HFS_AUDIT_BACKEND=none`)

Discards all events. Zero runtime cost. This is the default when no backend is configured.

### `FileSink` — local NDJSON file (`HFS_AUDIT_BACKEND=file`)

Appends each `AuditEvent` as a single JSON line to a local file. Simple, human-readable, and easy to ingest into log aggregation pipelines.

```bash
HFS_AUDIT_BACKEND=file HFS_AUDIT_FILE_PATH=./audit/audit.ndjson cargo run --bin hfs
tail -f ./audit/audit.ndjson
```

| Variable | Required | Description |
|----------|----------|-------------|
| `HFS_AUDIT_FILE_PATH` | Yes | Path to the NDJSON audit file (parent dirs created automatically) |

### `DatabaseSink` — FHIR storage backend (`HFS_AUDIT_BACKEND=database`)

Persists `AuditEvent` resources through HFS storage backends (SQLite, PostgreSQL, MongoDB, S3) as first-class FHIR resources.

`database` mode supports both:

- **Shared audit store (default):** no audit backend overrides are set, so audit events are written to the same primary backend/database as clinical data.
- **Dedicated audit store:** one or more `HFS_AUDIT_*` backend override variables are set, so audit events are written to a separate backend target for independent management.

| Variable | Required | Description |
|----------|----------|-------------|
| `HFS_AUDIT_DATABASE_URL` | No | Dedicated database URL/path override (SQLite/PostgreSQL/MongoDB families) |
| `HFS_AUDIT_MONGODB_DATABASE` | No | Dedicated MongoDB database name (MongoDB family) |
| `HFS_AUDIT_S3_BUCKET` | No | Dedicated S3 bucket (S3 family) |
| `HFS_AUDIT_S3_PREFIX` | No | Dedicated S3 key prefix (S3 family) |
| `HFS_AUDIT_S3_REGION` | No | Dedicated S3 region override (S3 family) |
| `HFS_AUDIT_S3_VALIDATE_BUCKETS` | No | Dedicated S3 bucket validation toggle (`true`/`false`, default inherited from primary) |

`DatabaseSink` writes with `TenantContext::system()` (tenant ID `__system__`) to keep audit records isolated from tenant-scoped clinical writes.

Examples:

```bash
# Shared mode (SQLite example): audit events stored in the same DB as primary data
HFS_STORAGE_BACKEND=sqlite \
HFS_AUDIT_BACKEND=database \
cargo run --bin hfs -- --database-url ./fhir.db

# Dedicated mode (PostgreSQL example): audit events stored in separate DB
HFS_STORAGE_BACKEND=postgres \
HFS_AUDIT_BACKEND=database \
HFS_AUDIT_DATABASE_URL=postgresql://helios:helios@localhost:5432/helios_audit \
cargo run --bin hfs
```

### `CloudWatchLogsSink` — AWS CloudWatch Logs (`HFS_AUDIT_BACKEND=cloudwatch`)

Sends each event as a JSON log entry to an AWS CloudWatch Logs log group. Ideal for AWS deployments where CloudWatch is the standard audit and observability destination. The log group and stream are created automatically on startup if they don't exist.

Requires building with the `cloudwatch` feature flag:

```bash
HFS_AUDIT_BACKEND=cloudwatch \
  HFS_AUDIT_CLOUDWATCH_LOG_GROUP=/hfs/audit \
  cargo run --bin hfs --features cloudwatch
```

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `HFS_AUDIT_CLOUDWATCH_LOG_GROUP` | Yes | — | CloudWatch Logs log group name |
| `HFS_AUDIT_CLOUDWATCH_LOG_STREAM` | No | `hfs-audit` | Log stream name within the group |
| `HFS_AUDIT_CLOUDWATCH_REGION` | No | AWS default chain | AWS region override |

Standard AWS credential chain applies (`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`, instance profiles, ECS task roles, etc.).

The `record_batch()` method sends multiple events in a single `PutLogEvents` API call for efficiency.

### Implementing a Custom Sink

Implement the `AuditSink` trait to send events to any destination:

```rust,ignore
#[async_trait]
pub trait AuditSink: Send + Sync + 'static {
    async fn record(&self, event: AuditEvent);
    async fn record_batch(&self, events: Vec<AuditEvent>) { /* default: sequential */ }
    async fn flush(&self);
    fn name(&self) -> &str;
}
```

All methods are infallible — log errors internally via `tracing`, never propagate them to callers.

## Quick Start

```rust
use helios_audit::{AuditAction, AuditEventBuilder, AuditSink, FileSink};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let sink = FileSink::new("./audit/audit.ndjson").await?;

    let event = AuditEventBuilder::new("Device/hfs")
        .action(AuditAction::Read)
        .outcome("0")
        .resource("Patient", "123")
        .patient("Patient/123")
        .agent("Practitioner/example", Some("Dr. Smith".to_string()), true)
        .build();

    sink.record(event).await;
    sink.flush().await;

    Ok(())
}
```

The file sink writes newline-delimited JSON, one `AuditEvent` per line, and flushes each write in `record()`.

### Rich Event Context

The builder supports attaching arbitrary metadata via `detail()`, custom entities via `entity()`, and explicit event-type overrides via `event_type()`:

```rust,ignore
let event = AuditEventBuilder::new("Device/hfs")
    .event_type("http://terminology.hl7.org/CodeSystem/audit-event-type", "object")
    .action(AuditAction::Execute)
    .outcome("0")
    .detail("audit-operation", "bulk-export")
    .detail("job-id", "export-abc-123")
    .detail("export-level", "system")
    .detail("resource-types", "Patient,Observation")
    .agent("Practitioner/dr-smith", None, true)
    .build();
```

Details are serialized as `AuditEventEntityDetail` entries on the primary entity, using FHIR's standard tagged key-value structure.

### Terminology Validation Policy

HFS validates emitted audit codings against canonical **CodeSystems** as strict gates:

- `http://terminology.hl7.org/CodeSystem/audit-event-type`
- `http://hl7.org/fhir/restful-interaction`

Additional **ValueSets** are tracked as advisory context in external audit reports (not hard CI gates unless explicitly profiled for that use case):

- `http://hl7.org/fhir/ValueSet/audit-event-sub-type`
- `http://hl7.org/fhir/ValueSet/type-restful-interaction`
- `http://hl7.org/fhir/ValueSet/system-restful-interaction`
- `http://hl7.org/fhir/ValueSet/interaction-trigger`
- `http://hl7.org/fhir/ValueSet/testscript-operation-codes`

## Architecture

The crate is split into six core pieces:

- **`AuditEventBuilder`** builds typed FHIR `AuditEvent` structs with BALP profile selection, entity details, and custom event types
- **`AuditSink`** defines the backend contract (`record`, `record_batch`, `flush`, `name`)
- **`AuditBridge`** translates auth events (success, failure, grant, denial) into `AuditEvent` resources
- **`audit_middleware` + `AuditMiddlewareState`** records REST interactions from Axum and provides `record_bundle_entries` for batch/transaction correlation
- **`AuditCorrelation`** links per-entry audit events within a bundle by a shared ID
- **`lifecycle`** records server startup (with configuration details) and shutdown events

## How Events Are Mapped

### REST Interactions

The middleware uses interaction-aware detection (`detect_interaction`) instead of mapping by HTTP method alone:

| Request Pattern | Audit Action |
|-----------------|--------------|
| `GET`/`HEAD` type search or history (`/Patient`, `/Patient/_history`) | `Query` (`E`) |
| `GET`/`HEAD` system history (`/_history`) | `Query` (`E`) |
| `GET`/`HEAD` instance history (`/Patient/123/_history`) | `Query` (`E`) |
| `GET`/`HEAD` instance read (`/Patient/123`) | `Read` (`R`) |
| `POST` with `/_search` | `Query` (`E`) |
| `POST /[type]` create | `Create` (`C`) |
| `POST /` batch/transaction envelope | `Execute` (`E`) |
| `PUT`/`PATCH` | `Update` (`U`) |
| `DELETE` | `Delete` (`D`) |
| Other methods | `Execute` (`E`) |

Response status is translated into a coarse outcome:

- `status < 400` -> outcome `0`
- `status >= 400` -> outcome `8`

For query events, BALP profile selection uses query profiles (`IHE.BasicAudit.Query` / `IHE.BasicAudit.PatientQuery`) and RESTful interaction subtype `search`.

### Write Context Enrichment (`AuditResponseContext`)

`audit_middleware` can enrich events from `response.extensions()` using `AuditResponseContext`:

- `resource_type`: fallback resource type when route/path extraction is insufficient
- `resource_id`: post-handler ID (for create/upsert flows where ID is not known pre-request)
- `patient_reference`: post-handler patient context derived from the resulting resource

This lets create/update/patch audits include the final resource identity and patient linkage even when the request path or pre-request context is incomplete.

### Patient Resolution Waterfall

When the crate tries to attach patient context, it checks in this order:

1. Direct Patient resource access such as `/Patient/123`
2. `subject.reference` or `patient.reference` in the resource body
3. Search parameters named `patient` or `subject`
4. No patient entity if none of the above resolve

### Authentication and Authorization Events

`AuditBridge` records:

- **Authentication success** as action `E`, outcome `0`
- **Authentication failure** (invalid/missing token) as action `E`, outcome `8`
- **Authorization grant** (scope check passed) as action `E`, outcome `0`, with resource type and operation
- **Authorization denial** (insufficient scopes) as action `E`, outcome `8`, with resource type and operation

### Lifecycle Events

The `lifecycle` module records server startup and shutdown:

| Event | Type | Action | Details |
|-------|------|--------|---------|
| Startup | `object` | `E` | `audit-operation=lifecycle-startup`, `phase=startup`, `storage-backend`, `fhir-version`, `auth-enabled`, `audit-backend` |
| Shutdown | `object` | `E` | `audit-operation=lifecycle-shutdown`, `phase=shutdown` |

HFS emits a startup event immediately after the audit subsystem initializes, and a shutdown event during graceful termination (Ctrl-C / SIGINT).

### Persistence-Layer Events

When `helios-persistence` is built with the `audit` feature, each module provides its own audit helper that uses the enriched builder:

| Operation | Event Type | Action | Key Details |
|-----------|-----------|--------|-------------|
| Bulk export | `object` | `E` | `audit-operation=bulk-export`, `job-id`, `export-level`, `resource-types` |
| Bulk submit | `object` | `E` | `audit-operation=bulk-import`, `submission-id`, `submitter`, `phase` |
| Purge | `object` | `D` | `audit-operation=purge`, `resource-type`, `count`, patient entity |
| Reindex | `object` | `E` | `audit-operation=reindex`, `job-id`, `phase`, `resources-processed` |

These functions live in their respective persistence modules (e.g., `helios_persistence::core::bulk_export::audit::record_export_event`) and are called by the application layer at the appropriate lifecycle points.

### Batch and Transaction Correlation

When a batch or transaction bundle is processed, the handler can create an `AuditCorrelation` and call `AuditMiddlewareState::record_bundle_entries()` to emit one `AuditEvent` per entry. Each event carries:

- `bundle-id` — shared UUID linking all entries in the same bundle
- `bundle-type` — `"batch"` or `"transaction"`
- `entry-index` — position within the bundle

These per-entry events are additive to the outer HTTP-level event recorded by the middleware.

```rust,ignore
use helios_audit::{AuditCorrelation, BundleAuditEntry};

let correlation = AuditCorrelation::new("transaction");

let entries = vec![
    BundleAuditEntry {
        method: "POST".to_string(),
        resource_type: Some("Patient".to_string()),
        resource_id: Some("123".to_string()),
        status: 201,
        patient_ref: Some("Patient/123".to_string()),
    },
    BundleAuditEntry {
        method: "PUT".to_string(),
        resource_type: Some("Observation".to_string()),
        resource_id: Some("obs-1".to_string()),
        status: 200,
        patient_ref: Some("Patient/123".to_string()),
    },
];

audit_state.record_bundle_entries(&correlation, entries, Some("Practitioner/dr-1")).await;
```

## Axum Integration

Use the middleware when you want REST requests to emit `AuditEvent` records:

```rust,ignore
use std::sync::Arc;

use axum::Router;
use helios_audit::{
    middleware::audit_middleware, AuditConfig, AuditMiddlewareState, AuditSink, ExclusionFilter,
    FileSink,
};

let sink: Arc<dyn AuditSink> = Arc::new(FileSink::new("./audit/audit.ndjson").await?);
let config = AuditConfig::from_env();
let audit_state = Arc::new(AuditMiddlewareState {
    sink: Arc::clone(&sink),
    config: config.clone(),
    exclusion_filter: ExclusionFilter::default_exclusions(),
});

let app = Router::new().layer(axum::middleware::from_fn_with_state(
    audit_state,
    audit_middleware,
));
```

For authentication events, wrap the same sink with `AuditBridge` and pass it to `helios-auth`.

## Running with HFS

The `hfs` binary initializes this crate from `HFS_AUDIT_*` environment variables. See the [Audit Sinks](#audit-sinks) section above for backend-specific configuration.

HFS creates an `AuditBridge` for auth events and installs the audit middleware into the REST stack when audit is enabled.

When using `helios-rest` with batch/transaction handlers, the outer middleware skips `POST /` envelopes and batch/transaction handlers emit per-entry audit events.

## Configuration

All configuration is via `HFS_AUDIT_*` environment variables. Backend-specific variables are documented in the [Audit Sinks](#audit-sinks) section above.

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_AUDIT_BACKEND` | `none` | Active backend: `none`, `file`, `database`, or `cloudwatch` |
| `HFS_AUDIT_SOURCE_OBSERVER` | `Device/hfs` | Value used for `AuditEvent.source.observer.reference` |
| `HFS_AUDIT_EXCLUDE_PATHS` | *(none)* | Comma-separated request paths to exclude |

### Built-In Exclusions

`ExclusionFilter::default_exclusions()` skips endpoints that are usually too noisy or operational:

- `/health`
- `/metadata`
- `/.well-known/smart-configuration`
- `/$versions`

## Features

FHIR version support follows Cargo features:

- `R4` (default)
- `R4B`
- `R5`
- `R6`

Example:

```toml
[dependencies]
helios-audit = { version = "0.1", features = ["R4"] }
```

The `helios-persistence` crate has an optional `audit` feature that enables the persistence-layer audit functions:

```toml
[dependencies]
helios-persistence = { version = "0.1", features = ["sqlite", "audit"] }
```

## FHIR AuditEvent Spec Coverage

The [FHIR AuditEvent specification](https://hl7.org/fhir/auditevent.html) identifies several categories of auditable activity. Here is how HFS maps to each:

| Category | Status | Notes |
|----------|--------|-------|
| **Data manipulation** | Covered | REST CRUD audit via middleware |
| **Access control decisions** | Covered | Auth success/failure, authz grant/denial via bridge |
| **Software startup/shutdown** | Covered | Lifecycle events with config details |
| **Configuration events** | Covered | Config snapshot in startup event; no runtime config changes exist |
| **User login/logout** | N/A | HFS uses stateless JWT — no sessions to log in/out of |
| **Software installation** | N/A | Compiled binary — no runtime installation |
| **Policy rules changes** | N/A | Policy is determined by JWT scopes — no runtime policy API |

## Current Limitations

- Dedicated database audit stores are not automatically exposed through the primary REST API (they are intentionally separate storage targets)
- Middleware records events after the handler runs and does not directly inspect response bodies (enrichment comes from response extensions)
- Audit recording is intentionally infallible; write failures are logged, not surfaced to API clients
- Persistence-layer audit functions are available but not yet wired into REST handlers (the functions are ready for callers to use at the appropriate lifecycle points)
