//! The initialized half of the dashboard Uptime card (#540). The uptime
//! tracker is process-global (`OnceCell` in `helios_observability::uptime`),
//! so this lives in its own test binary: `router_http.rs` asserts the
//! never-initialized fallback, this file asserts the real read path.

use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn app() -> Router {
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        helios_ui::NlSearch {
            enabled: false,
            configured: false,
            model: String::new(),
        },
        None,
        None,
        "default".to_string(),
        std::sync::Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        helios_fhir::FhirVersion::R4,
        None,
    )
}

#[tokio::test]
async fn uptime_card_shows_the_process_uptime_once_initialized() {
    helios_observability::uptime::init();

    let response = app()
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8(bytes.to_vec()).unwrap();

    // The sub-label only renders on the initialized branch, and the value is
    // a duration, never the old hardcoded percentage.
    assert!(html.contains("since process start"));
    assert!(!html.contains("99.98"));
}
