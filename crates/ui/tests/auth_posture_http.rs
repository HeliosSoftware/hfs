//! The shell's bearer-only notice (issue #1560): with authentication enabled
//! and no interactive login installed, every page says so once and names the
//! setting that enables the sign-in. The flag is process-wide, so this binary
//! owns it: one test flips it on, asserts, and flips it back.

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

async fn status(method: &str, uri: &str) -> (StatusCode, String) {
    let response = app()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("id=probe"))
                .unwrap(),
        )
        .await
        .unwrap();
    let code = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (code, String::from_utf8(bytes.to_vec()).unwrap())
}

/// The routes whose handlers act on storage themselves (#1619).
const STORAGE_ROUTES: &[(&str, &str)] = &[
    ("GET", "/ui/tenants"),
    ("GET", "/ui/tenants/rows"),
    ("POST", "/ui/tenants"),
    ("DELETE", "/ui/tenants/other"),
    ("DELETE", "/ui/tenants/other?purge=true"),
    ("GET", "/ui/tenant/options"),
    ("POST", "/ui/tenant"),
    ("GET", "/ui/bulk-import"),
    ("GET", "/ui/bulk-import/keys"),
    ("GET", "/ui/bulk-import/some-id"),
    ("GET", "/ui/bulk-import/some-id/status"),
    ("POST", "/ui/bulk-import"),
    ("POST", "/ui/bulk-import/some-id/delete"),
    ("POST", "/ui/bulk-import/some-id/abort"),
    ("POST", "/ui/bulk-import/some-id/complete"),
    ("POST", "/ui/bulk-import/some-id/edit"),
];

async fn html(uri: &str) -> String {
    let response = app()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{uri}");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn the_shell_says_once_that_no_browser_sign_in_is_configured() {
    helios_ui::set_bearer_only_auth(true);

    let home = html("/ui").await;
    assert_eq!(
        home.matches(r#"id="auth-bearer-only""#).count(),
        1,
        "one notice, in the shell"
    );
    assert!(home.contains("HFS_UI_LOGIN_CLIENT_ID"), "{home}");

    let batch = html("/ui/batch").await;
    assert!(batch.contains(r#"id="auth-bearer-only""#));
    assert!(
        batch.contains("data-msg-sign-in-required=\"This server has no browser sign-in"),
        "{batch}"
    );
    // #1662: in this posture batch.js answers from the shell before sending,
    // so a bundle of any size gets the sign-in message, never a dropped
    // upload's "Failed to fetch".
    let script = html("/ui/assets/batch.js").await;
    let precheck = script
        .find(r#"if (document.getElementById("auth-bearer-only"))"#)
        .expect("batch.js checks the posture before executing");
    let send = script
        .find(r#"fetch("/""#)
        .expect("batch.js posts to the root");
    assert!(precheck < send, "the posture check runs before the POST");

    let spanish = html("/ui?lang=es").await;
    assert!(
        spanish.contains("no hay un inicio de sesión de navegador configurado"),
        "{spanish}"
    );

    // #1619: the routes that act on storage directly refuse an anonymous
    // caller in this posture — with the shell's own words — while the shell
    // itself still renders.
    for (method, uri) in STORAGE_ROUTES {
        let (code, body) = status(method, uri).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED, "{method} {uri}: {body}");
        assert!(
            body.contains("HFS_UI_LOGIN_CLIENT_ID"),
            "{method} {uri}: the refusal names the setting: {body}"
        );
    }
    let (code, body) = status("GET", "/ui/tenants?lang=es").await;
    assert_eq!(code, StatusCode::UNAUTHORIZED);
    assert!(
        body.contains("no hay un inicio de sesión de navegador configurado"),
        "{body}"
    );
    assert_eq!(status("GET", "/ui").await.0, StatusCode::OK);
    assert_eq!(status("GET", "/ui/batch").await.0, StatusCode::OK);

    helios_ui::set_bearer_only_auth(false);
    let off = html("/ui").await;
    assert!(!off.contains("auth-bearer-only"), "{off}");
    // Without the posture the same routes answer as before: the pages render
    // (with no registry or store behind this test app, their "unavailable"
    // state) and the fragments and mutations reach their handlers.
    for (method, uri) in STORAGE_ROUTES {
        let (code, body) = status(method, uri).await;
        assert_ne!(code, StatusCode::UNAUTHORIZED, "{method} {uri}: {body}");
    }
}
