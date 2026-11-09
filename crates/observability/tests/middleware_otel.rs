//! Covers W3C trace-context propagation in
//! [`helios_observability::middleware::track`]: a request carrying a valid
//! `traceparent` must join the caller's trace rather than start a new one, so a
//! client can find the server's span under the trace id it sent.
//!
//! Like `middleware_full.rs`, this forces the `full` arm in its own test binary
//! so the request span opens without an OTLP endpoint. Every case runs inside
//! one test: the arm is read from the environment, which must not be written
//! while another test thread reads it.
#![cfg(feature = "otel")]

use axum::{Router, body::Body, http::Request, routing::get};
use opentelemetry::trace::{TraceContextExt as _, TraceId, TracerProvider as _};
use opentelemetry_sdk::trace::SdkTracerProvider;
use tower::ServiceExt;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;
use tracing_subscriber::prelude::*;

const TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";
const PARENT_ID: &str = "b7ad6b7169203331";

/// The trace id of the request span the handler runs in.
async fn handler_trace_id(traceparent: Option<&str>) -> String {
    let app = Router::new()
        .route(
            "/ping",
            get(|| async {
                tracing::Span::current()
                    .context()
                    .span()
                    .span_context()
                    .trace_id()
                    .to_string()
            }),
        )
        .layer(axum::middleware::from_fn(
            helios_observability::middleware::track,
        ));
    let mut req = Request::builder().uri("/ping");
    if let Some(tp) = traceparent {
        req = req.header("traceparent", tp);
    }
    let resp = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 64).await.unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

#[tokio::test]
async fn request_span_joins_only_a_valid_caller_trace() {
    // SAFETY: the only test in this binary, and `mode()` first reads the env
    // inside the first request below.
    unsafe {
        std::env::set_var("HELIOS_OBS_MODE", "full");
    }
    let provider = SdkTracerProvider::builder().build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
    let _guard = tracing::subscriber::set_default(subscriber);

    let sampled = format!("00-{TRACE_ID}-{PARENT_ID}-01");
    assert_eq!(handler_trace_id(Some(&sampled)).await, TRACE_ID);

    // The caller's sampling decision is honored, but the trace is still theirs.
    let unsampled = format!("00-{TRACE_ID}-{PARENT_ID}-00");
    assert_eq!(handler_trace_id(Some(&unsampled)).await, TRACE_ID);

    let fresh = handler_trace_id(None).await;
    assert_ne!(fresh, TRACE_ID);
    assert_ne!(
        fresh,
        TraceId::INVALID.to_string(),
        "a new root still gets a trace id"
    );

    // An all-zero parent id makes the whole header invalid.
    let bad_parent = format!("00-{TRACE_ID}-0000000000000000-01");
    assert_ne!(handler_trace_id(Some(&bad_parent)).await, TRACE_ID);

    assert_ne!(
        handler_trace_id(Some("not-a-traceparent")).await,
        TraceId::INVALID.to_string()
    );
}
