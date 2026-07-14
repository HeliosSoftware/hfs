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

/// Version-agnostic alias for the FHIR model module backing `AuditEvent`.
///
/// `helios-audit` emits IHE BALP `AuditEvent` resources, whose structure is
/// defined on the **R4** `AuditEvent` model (R4 and R4B share an identical
/// `AuditEvent` API; R5/R6 restructured it incompatibly). Rather than hard-pin
/// `helios_fhir::r4`, this aliases to the lowest enabled R4-family version so the
/// crate builds in single-version-minimal configurations (e.g. R4B-only) without
/// dragging in the R4 feature.
#[cfg(feature = "R4")]
pub(crate) use helios_fhir::r4 as fhir_model;
#[cfg(all(feature = "R4B", not(feature = "R4")))]
pub(crate) use helios_fhir::r4b as fhir_model;

#[cfg(all(not(feature = "R4"), not(feature = "R4B")))]
compile_error!(
    "helios-audit emits IHE BALP (R4-structured) AuditEvents and requires the \
     `R4` or `R4B` feature. For an R5/R6-only build, additionally enable `R4B`."
);

pub mod balp;
pub mod builder;
pub mod config;
pub mod correlation;
pub mod error;
pub mod exclusion;
pub mod helpers;
pub mod lifecycle;
pub mod middleware;
pub mod patient;
pub mod sink;
pub mod sinks;

// Re-exports for convenience
pub use balp::AuditAction;
pub use builder::AuditEventBuilder;
pub use config::{AuditBackend, AuditConfig};
pub use correlation::{AuditCorrelation, BundleAuditEntry};
pub use error::{AuditError, AuditResult};
pub use exclusion::ExclusionFilter;
pub use middleware::{AuditAgent, AuditMiddlewareState, AuditResponseContext};
pub use sink::AuditSink;
#[cfg(feature = "cloudwatch")]
pub use sinks::CloudWatchLogsSink;
pub use sinks::{DatabaseSink, FileSink, NullSink};
