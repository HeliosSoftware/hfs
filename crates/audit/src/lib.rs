//! # helios-audit — FHIR AuditEvent Logging for HFS
//!
//! Records security, privacy, and operational events as FHIR `AuditEvent`
//! resources conforming to the [IHE BALP v1.1.4](https://profiles.ihe.net/ITI/BALP/)
//! Implementation Guide.
//!
//! ## Architecture
//!
//! - **[`AuditSink`]** — Pluggable backend trait (null, file, database)
//! - **[`AuditEventBuilder`]** — Fluent builder for typed `AuditEvent` structs
//! - **[`AuditBridge`]** — Bridges `helios_auth::AuditEventSink` to `AuditSink`
//! - **[`AuditMiddlewareState`]** — Axum middleware for FHIR REST audit logging
//!
//! ## Design Principles
//!
//! 1. **Zero-cost when disabled** — `NullSink` compiles to no-ops
//! 2. **Uses the FHIR model directly** — No custom intermediate representation
//! 3. **Async-first** — All sink implementations are async
//! 4. **Fire-and-forget** — Audit failures are logged, never propagated
//! 5. **Single active backend** — One configured sink at a time
//! 6. **Immutable audit trail** — No UPDATE or DELETE on AuditEvent via REST
//!
//! ## Configuration
//!
//! | Variable | Default | Description |
//! |----------|---------|-------------|
//! | `HFS_AUDIT_BACKEND` | `none` | `none`, `file`, `database` |
//! | `HFS_AUDIT_FILE_PATH` | — | File path for the file sink |
//! | `HFS_AUDIT_DATABASE_URL` | — | Dedicated DB URL (optional) |
//! | `HFS_AUDIT_SOURCE_OBSERVER` | `Device/hfs` | AuditEvent.source.observer |
//! | `HFS_AUDIT_EXCLUDE_PATHS` | — | Comma-separated paths to skip |

pub mod balp;
pub mod bridge;
pub mod builder;
pub mod config;
pub mod error;
pub mod exclusion;
pub mod helpers;
pub mod middleware;
pub mod patient;
pub mod sink;
pub mod sinks;

// Re-exports for convenience
pub use bridge::AuditBridge;
pub use builder::AuditEventBuilder;
pub use config::{AuditBackend, AuditConfig};
pub use error::{AuditError, AuditResult};
pub use exclusion::ExclusionFilter;
pub use middleware::AuditMiddlewareState;
pub use sink::AuditSink;
pub use sinks::{DatabaseSink, FileSink, NullSink};
