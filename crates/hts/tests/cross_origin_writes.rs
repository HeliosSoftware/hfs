//! A write from a page on another origin is refused, through the full
//! middleware stack `create_app` assembles: the HTS write routes have no
//! authentication, and a browser sends a `text/plain` POST without a preflight.

#![cfg(feature = "sqlite")]

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use helios_hts::{backends::SqliteTerminologyBackend, config::HtsConfig, state::AppState};
use tower::ServiceExt;

fn app(config: HtsConfig) -> Router {
    let backend =
        SqliteTerminologyBackend::in_memory().expect("failed to create in-memory HTS backend");
    helios_hts::server::create_app(&config, AppState::new(backend))
}

const BUNDLE: &str = r#"{"resourceType":"Bundle","type":"collection","entry":[{"resource":{"resourceType":"CodeSystem","url":"http://hts.test/xo","status":"active","content":"complete","concept":[{"code":"a"}]}}]}"#;

async fn import(router: Router, headers: &[(&str, &str)]) -> StatusCode {
    let mut request = Request::post("/import").header("content-type", "text/plain");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    router
        .oneshot(request.body(Body::from(BUNDLE)).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn a_cross_site_import_is_refused() {
    assert_eq!(
        import(
            app(HtsConfig::default()),
            &[("sec-fetch-site", "cross-site")]
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        import(
            app(HtsConfig::default()),
            &[("origin", "https://evil.example"), ("host", "hts.internal")]
        )
        .await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_cross_site_delete_is_refused_but_a_read_operation_is_not() {
    let router = app(HtsConfig::default());
    let delete = router
        .clone()
        .oneshot(
            Request::delete("/CodeSystem/anything")
                .header("sec-fetch-site", "cross-site")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::FORBIDDEN);

    let expand = router
        .oneshot(
            Request::get("/ValueSet/$expand?url=http://hts.test/none")
                .header("sec-fetch-site", "cross-site")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(expand.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn same_origin_listed_and_non_browser_imports_go_through() {
    assert_eq!(
        import(
            app(HtsConfig::default()),
            &[("sec-fetch-site", "same-origin")]
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(import(app(HtsConfig::default()), &[]).await, StatusCode::OK);

    let listed = HtsConfig {
        cors_origins: "https://admin.example".to_string(),
        ..HtsConfig::default()
    };
    assert_eq!(
        import(
            app(listed),
            &[
                ("origin", "https://admin.example"),
                ("sec-fetch-site", "cross-site")
            ]
        )
        .await,
        StatusCode::OK
    );
}
