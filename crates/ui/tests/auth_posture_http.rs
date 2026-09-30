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

    let spanish = html("/ui?lang=es").await;
    assert!(
        spanish.contains("no hay un inicio de sesión de navegador configurado"),
        "{spanish}"
    );

    helios_ui::set_bearer_only_auth(false);
    let off = html("/ui").await;
    assert!(!off.contains("auth-bearer-only"), "{off}");
}
