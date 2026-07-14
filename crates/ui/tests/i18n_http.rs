//! End-to-end locale negotiation through the mounted router: the same
//! requests a browser would make, exercising the middleware, the handlers,
//! and the templates together.

use axum::{
    Router,
    body::Body,
    http::{Request, header},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn app() -> Router {
    helios_ui::mount(Router::new(), "9.9.9", None)
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn accept_language_selects_the_ui_language() {
    let response = app()
        .oneshot(
            Request::get("/ui")
                .header(header::ACCEPT_LANGUAGE, "de-DE, de;q=0.9, en;q=0.7")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.headers().get(header::SET_COOKIE).is_none());
    let vary: Vec<_> = response
        .headers()
        .get_all(header::VARY)
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect();
    assert!(
        vary.iter().any(|v| v.contains("Accept-Language")),
        "language-dependent responses must vary on Accept-Language, got {vary:?}"
    );
    let html = body_text(response).await;
    assert!(html.contains(r#"<html lang="de">"#));
    assert!(html.contains("Startseite"));
}

#[tokio::test]
async fn lang_override_wins_and_persists_in_a_cookie() {
    let response = app()
        .oneshot(
            Request::get("/ui?lang=es")
                .header(header::ACCEPT_LANGUAGE, "de")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("explicit ?lang= must set the hfs_lang cookie")
        .to_str()
        .unwrap()
        .to_owned();
    assert!(cookie.starts_with("hfs_lang=es"));

    let html = body_text(response).await;
    assert!(html.contains(r#"<html lang="es">"#));
    assert!(html.contains("Inicio"));
}

#[tokio::test]
async fn cookie_keeps_the_choice_on_later_requests() {
    let response = app()
        .oneshot(
            Request::get("/ui")
                .header(header::COOKIE, "hfs_lang=es")
                .header(header::ACCEPT_LANGUAGE, "de")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let html = body_text(response).await;
    assert!(html.contains(r#"<html lang="es">"#));
}

#[tokio::test]
async fn htmx_fragment_is_localized_too() {
    let response = app()
        .oneshot(
            Request::get("/ui/status")
                .header("HX-Request", "true")
                .header(header::ACCEPT_LANGUAGE, "es")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let html = body_text(response).await;
    assert!(html.contains("Última comprobación:"));
    assert!(!html.contains("<html"), "fragment, not a full page");
}

#[tokio::test]
async fn default_is_english() {
    let response = app()
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let html = body_text(response).await;
    assert!(html.contains(r#"<html lang="en">"#));
    assert!(html.contains(">Home<"));
}
