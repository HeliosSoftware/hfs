//! Tracing-subscriber initialization, optionally bridged to OTLP.
//!
//! [`init`] replaces the ad-hoc `tracing_subscriber` setup each server used to
//! do itself. It always installs an `fmt` + `EnvFilter` subscriber. When built
//! with the `otel` feature **and** `OTEL_EXPORTER_OTLP_ENDPOINT` is set, it also
//! exports spans over OTLP via `tracing-opentelemetry`. Call [`shutdown`] during
//! graceful shutdown to flush buffered spans.

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[cfg(feature = "otel")]
static PROVIDER: std::sync::OnceLock<opentelemetry_sdk::trace::SdkTracerProvider> =
    std::sync::OnceLock::new();

/// Global default log directives when neither `RUST_LOG` nor the cli/env filter
/// is set. Applies `level` globally; deps remain overridable via `RUST_LOG`.
fn default_directives(level: &str) -> String {
    level.to_string()
}

/// Initialize logging/tracing for a server process. `service_name` is used as
/// the OTel resource service name (overridable by `OTEL_SERVICE_NAME`). Call
/// once per process.
pub fn init(service_name: &str, log_level: &str) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_directives(log_level)));

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
