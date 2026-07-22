//! Tracing-subscriber initialization, optionally bridged to OTLP.
//!
//! [`init`] replaces the ad-hoc `tracing_subscriber` setup each server used to
//! do itself. It always installs an `fmt` + `EnvFilter` subscriber. When built
//! with the `otel` feature **and** `OTEL_EXPORTER_OTLP_ENDPOINT` is set, it also
//! exports spans over OTLP via `tracing-opentelemetry`. Call [`shutdown`] during
//! graceful shutdown to flush buffered spans.

use std::sync::atomic::{AtomicBool, Ordering};

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[cfg(feature = "otel")]
static PROVIDER: std::sync::OnceLock<opentelemetry_sdk::trace::SdkTracerProvider> =
    std::sync::OnceLock::new();

/// Set once, at [`init`], to whether a tracing layer is actually exporting the
/// per-request span. Without an exporter the span is created, entered on every
/// poll, and dropped with nobody reading it — pure overhead. The request
/// middleware consults [`traces_live`] to skip it in that case.
static TRACES_LIVE: AtomicBool = AtomicBool::new(false);

/// Whether a layer that consumes the per-request span is installed. `false`
/// until [`init`] decides otherwise, so the middleware defaults to the cheap
/// path. See [`crate::mode::ObsMode::span_enabled`].
pub fn traces_live() -> bool {
    TRACES_LIVE.load(Ordering::Relaxed)
}

/// Global default log directives when neither `RUST_LOG` nor the cli/env filter
/// is set. Applies `level` globally; deps remain overridable via `RUST_LOG`.
fn default_directives(level: &str) -> String {
    level.to_string()
}

/// Build the `EnvFilter`, honoring `RUST_LOG` only when it is set to a
/// non-empty value.
///
/// `EnvFilter::try_from_default_env()` treats a *set-but-empty* `RUST_LOG`
/// (`RUST_LOG=""`, as opposed to unset) as a valid filter that enables nothing,
/// silently silencing all output — including a server's startup line. That is a
/// footgun for operators, and it broke the obs-A/B benchmark harness, which
/// launches its non-probe arms with an empty `RUST_LOG` and then greps the
/// startup log to confirm which arm is active. Treat empty/whitespace-only
/// `RUST_LOG` as unset and fall back to the process default level; an unparseable
/// value also falls back, as before.
fn env_filter_from(rust_log: Option<&str>, log_level: &str) -> EnvFilter {
    match rust_log {
        Some(v) if !v.trim().is_empty() => {
            EnvFilter::try_new(v).unwrap_or_else(|_| EnvFilter::new(default_directives(log_level)))
        }
        _ => EnvFilter::new(default_directives(log_level)),
    }
}

/// Initialize logging/tracing for a server process. `service_name` is used as
/// the OTel resource service name (overridable by `OTEL_SERVICE_NAME`). Call
/// once per process.
pub fn init(service_name: &str, log_level: &str) {
    let filter = env_filter_from(std::env::var("RUST_LOG").ok().as_deref(), log_level);

    #[cfg(feature = "otel")]
    {
        if let Some(provider) = build_otlp_tracer(service_name) {
            use opentelemetry::trace::TracerProvider as _;
            let tracer = provider.tracer("helios");
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer())
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .init();
            let _ = PROVIDER.set(provider);
            // A layer now consumes the per-request span, so producing it earns
            // its keep. Everywhere else the middleware skips it.
            TRACES_LIVE.store(true, Ordering::Relaxed);
            tracing::info!(service = service_name, "OTLP trace export enabled");
            return;
        }
    }

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .init();
    let _ = service_name;
}

/// Flush and shut down the OTLP exporter, if one was configured. A no-op when
/// the `otel` feature is disabled or OTLP export was not configured.
pub fn shutdown() {
    #[cfg(feature = "otel")]
    if let Some(provider) = PROVIDER.get() {
        let _ = provider.shutdown();
    }
}

/// Build an OTLP-exporting tracer provider, or `None` when
/// `OTEL_EXPORTER_OTLP_ENDPOINT` is not set (export disabled).
#[cfg(feature = "otel")]
fn build_otlp_tracer(service_name: &str) -> Option<opentelemetry_sdk::trace::SdkTracerProvider> {
    use opentelemetry_otlp::SpanExporter;
    use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};

    if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_err() {
        return None;
    }

    let exporter = match SpanExporter::builder().with_tonic().build() {
        Ok(exporter) => exporter,
        Err(err) => {
            tracing::warn!(error = %err, "failed to build OTLP span exporter; tracing export disabled");
            return None;
        }
    };

    let service_name =
        std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| service_name.to_string());
    let resource = Resource::builder().with_service_name(service_name).build();

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();

    opentelemetry::global::set_tracer_provider(provider.clone());
    Some(provider)
}

#[cfg(test)]
mod tests {
    use super::{EnvFilter, env_filter_from};

    // Empty / whitespace / unset RUST_LOG must all fall back to the process
    // default level, so the startup line (and everything at that level) is
    // still emitted. A set-but-empty RUST_LOG previously enabled nothing.
    #[test]
    fn empty_or_unset_rust_log_falls_back_to_default_level() {
        for rust_log in [None, Some(""), Some("   ")] {
            assert_eq!(
                env_filter_from(rust_log, "info").to_string(),
                "info",
                "RUST_LOG={rust_log:?} should behave as unset"
            );
        }
    }

    #[test]
    fn non_empty_rust_log_is_honored() {
        assert_eq!(env_filter_from(Some("debug"), "info").to_string(), "debug");
        assert_eq!(
            env_filter_from(Some("warn,hts=trace"), "info").to_string(),
            EnvFilter::new("warn,hts=trace").to_string()
        );
    }

    #[test]
    fn unparseable_rust_log_falls_back_to_default_level() {
        // `@@@` is not a valid directive; fall back rather than panic.
        assert_eq!(env_filter_from(Some("@@@"), "info").to_string(), "info");
    }
}
