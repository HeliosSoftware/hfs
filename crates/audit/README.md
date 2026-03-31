# helios-audit

FHIR `AuditEvent` logging for the Helios FHIR Server, with [IHE BALP](https://profiles.ihe.net/ITI/BALP/) profile selection for REST and authentication events.

## Overview

This crate records security, privacy, and operational activity as typed FHIR `AuditEvent` resources instead of introducing a separate audit schema. It is designed to plug into HFS at two layers: the REST pipeline (FHIR interactions) and the auth pipeline (token use, failures, authorization denials).

- **FHIR-Native Audit Records**: Builds real `AuditEvent` resources using `helios-fhir`
- **IHE BALP Profile Selection**: Chooses BALP read/create/update/delete/query/auth profiles based on action and patient context
- **Pluggable Sinks**: No-op, append-only NDJSON file, or database-backed persistence
- **Axum Middleware**: Captures FHIR REST interactions after the request completes
- **Auth Bridge**: Adapts `helios_auth::AuditEventSink` into full FHIR `AuditEvent` records
- **Patient Resolution**: Resolves patient context from resource paths, request bodies, and search parameters
- **Fire-and-Forget Semantics**: Audit failures are logged via `tracing`, never returned to the caller

## Quick Start

```rust
use helios_audit::{AuditEventBuilder, AuditSink, FileSink};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let sink = FileSink::new("./audit/audit.ndjson").await?;

    let event = AuditEventBuilder::new("Device/hfs")
        .action("R")
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

The file sink writes newline-delimited JSON, one `AuditEvent` per line.

## Architecture

The crate is split into four core pieces:

- **`AuditEventBuilder`** builds typed FHIR `AuditEvent` structs with BALP profile selection
- **`AuditSink`** defines the backend contract (`record`, `record_batch`, `flush`, `name`)
- **`AuditBridge`** translates narrow auth events into `AuditEvent` resources
- **`audit_middleware` + `AuditMiddlewareState`** records REST interactions from Axum

### Sink Implementations

| Sink | Purpose |
|------|---------|
| `NullSink` | Discards all events when audit logging is disabled |
| `FileSink` | Appends each event as NDJSON to a local file |
| `DatabaseSink` | Persists `AuditEvent` resources through an `AuditStorage` implementation |

## How Events Are Mapped

### REST Interactions

The middleware maps HTTP methods to FHIR audit actions:

| HTTP Method | Audit Action |
|-------------|--------------|
| `GET`, `HEAD` | `R` |
| `POST` | `C` |
| `PUT`, `PATCH` | `U` |
| `DELETE` | `D` |
| Other methods | `E` |

Response status is translated into a coarse outcome:

- `status < 400` -> outcome `0`
- `status >= 400` -> outcome `8`

### Patient Resolution Waterfall

When the crate tries to attach patient context, it checks in this order:

1. Direct Patient resource access such as `/Patient/123`
2. `subject.reference` or `patient.reference` in the resource body
3. Search parameters named `patient` or `subject`
4. No patient entity if none of the above resolve

### Authentication Events

`AuditBridge` records:

- Successful authentication as action `E`, outcome `0`
- Authentication failures as action `E`, outcome `8`
- Authorization denials as action `E`, outcome `8`

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

The `hfs` binary initializes this crate from environment variables.

```bash
HFS_AUDIT_BACKEND=file \
  HFS_AUDIT_FILE_PATH=./audit/audit.ndjson \
  HFS_AUDIT_SOURCE_OBSERVER=Device/hfs \
  cargo run --bin hfs

# Inspect the audit stream
tail -f ./audit/audit.ndjson
```

HFS creates an `AuditBridge` for auth events and installs the audit middleware into the REST stack when audit is enabled.

## Configuration

All configuration is via `HFS_AUDIT_*` environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_AUDIT_BACKEND` | `none` | Active backend: `none`, `file`, or `database` |
| `HFS_AUDIT_FILE_PATH` | *(none)* | Required when `HFS_AUDIT_BACKEND=file` |
| `HFS_AUDIT_DATABASE_URL` | *(none)* | Optional dedicated database URL for audit persistence |
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

## Current Limitations

- The `DatabaseSink` exists in this crate, but the current `hfs` startup wiring only enables `none` and `file`
- Middleware records events after the handler runs and does not inspect the response body
- Audit recording is intentionally infallible; write failures are logged, not surfaced to API clients
