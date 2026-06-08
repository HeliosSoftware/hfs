//! Shared observability wiring for Helios servers.
//!
//! This crate centralizes the telemetry surface so the four Helios server
//! binaries (`hfs`, `hts`, `sof-server`, `fhirpath-server`) expose a uniform
//! set of endpoints and signals without duplicating wiring:
//!
//! - [`uptime`] — process start-time tracking, surfaced on `/health` and as the
//!   `uptime_seconds` metric.
//! - [`metrics`] — global Prometheus recorder + a `GET /metrics` router.
//! - [`middleware`] — per-request count/latency metrics and a tracing span.
//! - [`telemetry`] — `tracing-subscriber` init, optionally bridged to OTLP
//!   (feature `otel`).
//!
//! ## Typical wiring
//!
//! ```rust,ignore
//! // in main(), before serving:
//! helios_observability::uptime::init();
//! helios_observability::telemetry::init("hfs", &log_level);
//! helios_observability::metrics::init("hfs");
//!
//! let app = my_router()
//!     .merge(helios_observability::metrics::router())
//!     .layer(axum::middleware::from_fn(helios_observability::middleware::track));
//!
//! // on graceful shutdown:
//! helios_observability::telemetry::shutdown();
//! ```
//!
//! ## Design notes
//!
//! - `/metrics` uses the maintained `metrics` + `metrics-exporter-prometheus`
//!   stack, not `opentelemetry-prometheus` (unmaintained, RUSTSEC advisory).
//! - OTLP *metrics* are expected to be produced by an OpenTelemetry Collector
//!   scraping `/metrics`; the app itself only pushes OTLP *traces*.

pub mod metrics;
pub mod middleware;
pub mod telemetry;
pub mod uptime;
