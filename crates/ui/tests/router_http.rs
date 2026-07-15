//! End-to-end tests over the mounted router: the same requests a browser
//! would make, exercising [`helios_ui::mount`], the handlers, the embedded
//! asset service, the `Vary` middleware, and the FHIR fallback together.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    routing::get,
};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn app() -> Router {
    helios_ui::mount(Router::new(), "9.9.9")
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn index_serves_the_full_landing_page() {
    let response = app()
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("<!doctype html>"));
    assert!(html.contains("9.9.9"));
}

#[tokio::test]
async fn status_is_a_full_page_on_hard_navigation() {
    let response = app()
        .oneshot(Request::get("/ui/status").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("<!doctype html>"),
        "without HX-Request the same URL must render the whole document"
    );
    assert!(html.contains("9.9.9"));
}

#[tokio::test]
async fn status_is_a_fragment_for_htmx_and_varies_on_the_header() {
    let response = app()
        .oneshot(
            Request::get("/ui/status")
                .header("HX-Request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let vary: Vec<_> = response
        .headers()
        .get_all(header::VARY)
        .iter()
        .map(|v| v.to_str().unwrap().to_ascii_lowercase())
        .collect();
    assert!(
        vary.iter().any(|v| v.contains("hx-request")),
        "AutoVaryLayer must emit Vary: HX-Request so caches never cross a \
         fragment with a full page, got {vary:?}"
    );

    let html = body_text(response).await;
    assert!(html.contains("Last checked:"));
    assert!(!html.contains("<html"), "fragment, not a full page");
}

#[tokio::test]
async fn embedded_assets_are_served() {
    for asset in ["/ui/assets/htmx.min.js", "/ui/assets/app.css"] {
        let response = app()
            .oneshot(Request::get(asset).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{asset}");
    }
}

#[tokio::test]
async fn non_ui_paths_fall_through_to_the_fhir_app() {
    // Stand-in for the FHIR REST router: proves /ui never shadows it.
    let fhir_app = Router::new().route("/Patient", get(|| async { "fhir handled" }));
    let response = helios_ui::mount(fhir_app, "9.9.9")
        .oneshot(Request::get("/Patient").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_text(response).await, "fhir handled");
}

/* History & Versions (#236). The diff is computed server-side; these post two
 * versions the way the browser does after fetching them from _history. */

#[tokio::test]
async fn history_page_renders_the_shell() {
    let response = app()
        .oneshot(Request::get("/ui/history").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("<!doctype html>"));
    // Brett's tabs and the version rail.
    assert!(html.contains(r#"data-tab="instance""#));
    assert!(html.contains(r#"id="history-versions""#));
    assert!(html.contains("history.js"));
}

async fn diff(form: &str) -> String {
    let response = app()
        .oneshot(
            Request::post("/ui/history/diff")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body_text(response).await
}

#[tokio::test]
async fn diff_shows_a_rename_in_both_layers_and_hides_metadata() {
    // v3 -> v4: family Smith -> Smythe, and the meta churn that should be hidden.
    let from = "%7B%22name%22%3A%5B%7B%22family%22%3A%22Smith%22%7D%5D%2C%22meta%22%3A%7B%22versionId%22%3A%223%22%7D%7D";
    let to = "%7B%22name%22%3A%5B%7B%22family%22%3A%22Smythe%22%7D%5D%2C%22meta%22%3A%7B%22versionId%22%3A%224%22%7D%7D";
    let html = diff(&format!(
        "from={from}&to={to}&from_label=v3&to_label=v4&show_metadata=false"
    ))
    .await;

    // Semantic layer: a field-level replace on family, with the old value.
    assert!(html.contains("/name/0/family"));
    assert!(html.contains("Smith"));
    assert!(html.contains("Smythe"));
    // Metadata is filtered, and the toggle says how much.
    assert!(!html.contains("/meta/versionId"));
    assert!(html.contains("metadata"));
    // Textual layer: word-level highlight of the changed run.
    assert!(html.contains("<mark>"));
}

#[tokio::test]
async fn diff_shows_metadata_when_asked() {
    let from = "%7B%22meta%22%3A%7B%22versionId%22%3A%223%22%7D%7D";
    let to = "%7B%22meta%22%3A%7B%22versionId%22%3A%224%22%7D%7D";
    let html = diff(&format!("from={from}&to={to}&show_metadata=true")).await;
    assert!(html.contains("/meta/versionId"));
}

#[tokio::test]
async fn a_deleted_version_is_a_banner_not_a_diff() {
    let html = diff("from=%7B%7D&to=%7B%7D&to_label=v5&deleted=true").await;
    assert!(html.contains("history__banner--deleted"));
    assert!(html.contains("v5"));
    // No diff table for a tombstone.
    assert!(!html.contains("diff-table"));
}

#[tokio::test]
async fn identical_versions_say_so() {
    let doc = "%7B%22name%22%3A%5B%7B%22family%22%3A%22Smith%22%7D%5D%7D";
    let html = diff(&format!("from={doc}&to={doc}&show_metadata=true")).await;
    assert!(html.contains("history__banner--same"));
}

#[tokio::test]
async fn unparseable_versions_report_an_error_not_an_empty_diff() {
    let html = diff("from=%7Bnope&to=%7B%7D").await;
    assert!(html.contains("history__banner--error"));
}
