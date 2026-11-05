//! With no interactive login installed (issue #1449) the UI is exactly what it
//! was before: no session gate, and the login routes answer 404. Kept in its
//! own test binary because the login runtime is process-wide — the tests in
//! `login_http.rs` install one, and must not leak into this assertion.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn app() -> Router {
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        helios_ui::NlSearch {
            enabled: false,
            configured: false,
            model: "test-model".to_string(),
        },
        None,
        None,
        "default".to_string(),
        std::sync::Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    )
}

#[tokio::test]
async fn without_a_login_installed_pages_are_open_and_login_routes_are_absent() {
    let app = app();
    let page = app
        .clone()
        .oneshot(Request::get("/ui/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK, "no session gate");

    let login = app
        .clone()
        .oneshot(Request::get("/ui/login").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::NOT_FOUND);

    let logout = app
        .oneshot(Request::post("/ui/logout").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::NOT_FOUND);
}
