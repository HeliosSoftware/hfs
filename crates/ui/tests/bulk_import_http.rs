//! End-to-end tests for the Bulk Import workspace (`/ui/bulk-import`, #527),
//! driving the mounted router against a real in-memory SQLite settings store
//! and, for submission, a loopback stand-in for the Data Recipient.

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::SettingsStore;
use http_body_util::BodyExt;
use tower::ServiceExt;

fn settings_store() -> Arc<dyn SettingsStore> {
    let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
    backend.init_schema().expect("init schema");
    Arc::new(backend)
}

fn app(settings: &Arc<dyn SettingsStore>) -> Router {
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        None,
        helios_ui::NlSearch::default(),
        None,
        Some(Arc::clone(settings)),
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::empty()),
        FhirVersion::R4,
        None,
    )
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn get(settings: &Arc<dyn SettingsStore>, uri: &str) -> (StatusCode, String) {
    let res = app(settings)
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    (status, body_text(res).await)
}

/// POSTs a form and returns `(status, Location header, body)`.
async fn post_form(
    settings: &Arc<dyn SettingsStore>,
    uri: &str,
    form: &str,
) -> (StatusCode, String, String) {
    let res = app(settings)
        .oneshot(
            Request::post(uri)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(form.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let location = res
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    (status, location, body_text(res).await)
}

/// Creates a submission and returns its detail path (`/ui/bulk-import/{id}`).
async fn create_submission(settings: &Arc<dyn SettingsStore>) -> String {
    let (status, location, _) = post_form(
        settings,
        "/ui/bulk-import",
        "name=BrettTest&recipient_base_url=http%3A%2F%2Flocalhost%3A9%2F&auth=none",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(location.starts_with("/ui/bulk-import/"), "{location}");
    location
}

#[tokio::test]
async fn the_list_page_renders_and_offers_creation() {
    let settings = settings_store();
    let (status, html) = get(&settings, "/ui/bulk-import").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Bulk Import"));
    assert!(html.contains("New submission"));
    assert!(html.contains("No submissions yet"));
}

#[tokio::test]
async fn creating_a_submission_lands_on_its_detail_page() {
    let settings = settings_store();
    let detail_path = create_submission(&settings).await;

    let (status, html) = get(&settings, &detail_path).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("BrettTest"));
    assert!(html.contains("Not Started"));
    assert!(
        html.contains("http://localhost:9"),
        "trailing slash trimmed"
    );
    assert!(html.contains("Add Manifest"));

    // And the list now shows it.
    let (_, list) = get(&settings, "/ui/bulk-import").await;
    assert!(list.contains("BrettTest"));
}

#[tokio::test]
async fn manifests_can_be_added_and_removed() {
    let settings = settings_store();
    let detail_path = create_submission(&settings).await;

    let (status, location, _) = post_form(
        &settings,
        &format!("{detail_path}/manifests"),
        "manifest_url=http%3A%2F%2F10.2.1.890%2Fmanifest.local",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, detail_path);

    let (_, html) = get(&settings, &detail_path).await;
    assert!(html.contains("http://10.2.1.890/manifest.local"));
    assert!(html.contains("Submit All"));

    // Pull the manifest id out of the rendered submit form action.
    let marker = format!("{detail_path}/manifests/");
    let mid = html
        .split(&marker)
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .expect("manifest id in form action")
        .to_string();

    let (status, _, _) = post_form(
        &settings,
        &format!("{detail_path}/manifests/{mid}/delete"),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (_, html) = get(&settings, &detail_path).await;
    assert!(html.contains("No manifests yet"));
}

#[tokio::test]
async fn deleting_a_submission_returns_to_the_list() {
    let settings = settings_store();
    let detail_path = create_submission(&settings).await;

    let (status, location, _) = post_form(&settings, &format!("{detail_path}/delete"), "").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, "/ui/bulk-import");

    // The detail page for a deleted submission redirects back to the list.
    let res = app(&settings)
        .oneshot(Request::get(&detail_path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
}

/// A loopback Data Recipient that records the kick-off body it receives.
async fn mock_recipient(
    status: StatusCode,
) -> (String, Arc<std::sync::Mutex<Vec<serde_json::Value>>>) {
    use axum::extract::State;
    let received: Arc<std::sync::Mutex<Vec<serde_json::Value>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::clone(&received);
    let recipient = Router::new()
        .route(
            "/$bulk-submit",
            axum::routing::post(
                move |State(seen): State<Arc<std::sync::Mutex<Vec<serde_json::Value>>>>,
                      axum::Json(body): axum::Json<serde_json::Value>| async move {
                    seen.lock().unwrap().push(body);
                    (
                        status,
                        axum::Json(serde_json::json!({"resourceType": "Parameters"})),
                    )
                },
            ),
        )
        .with_state(state);
    tokio::spawn(async move { axum::serve(listener, recipient).await.unwrap() });
    (format!("http://{addr}"), received)
}

#[tokio::test]
async fn submitting_a_manifest_posts_the_kickoff_and_logs_the_outcome() {
    let settings = settings_store();
    let (recipient_url, received) = mock_recipient(StatusCode::OK).await;

    let (_, detail_path, _) = post_form(
        &settings,
        "/ui/bulk-import",
        &format!(
            "name=Alice&recipient_base_url={}&auth=none",
            urlencode(&recipient_url)
        ),
    )
    .await;
    post_form(
        &settings,
        &format!("{detail_path}/manifests"),
        "manifest_url=http%3A%2F%2Fexample.org%2Fexports%2Fmanifest.json&output_format=application%2Ffhir%2Bndjson",
    )
    .await;

    let (status, _, _) = post_form(&settings, &format!("{detail_path}/submit-all"), "").await;
    assert_eq!(status, StatusCode::SEE_OTHER);

    // The recipient saw a spec-shaped kick-off.
    let bodies = received.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1);
    let params = &bodies[0];
    assert_eq!(params["resourceType"], "Parameters");
    let names: Vec<&str> = params["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["name"].as_str())
        .collect();
    assert!(names.contains(&"submitter"));
    assert!(names.contains(&"submissionId"));
    assert!(names.contains(&"submissionStatus"));
    assert!(names.contains(&"manifestUrl"));
    assert!(names.contains(&"fhirBaseUrl"));
    assert!(names.contains(&"outputFormat"));
    // fhirBaseUrl fell back to the manifest's own base.
    let fhir_base = params["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "fhirBaseUrl")
        .and_then(|p| p["valueUrl"].as_str())
        .unwrap();
    assert_eq!(fhir_base, "http://example.org/exports");

    // The log recorded the attempt and the submission moved to In Progress.
    let (_, html) = get(&settings, &detail_path).await;
    assert!(html.contains("Submitting manifest"));
    assert!(html.contains("accepted by the recipient (200)"));
    assert!(html.contains("In Progress"));
}

#[tokio::test]
async fn a_rejected_kickoff_is_logged_as_a_failure() {
    let settings = settings_store();
    let (recipient_url, _) = mock_recipient(StatusCode::NOT_FOUND).await;

    let (_, detail_path, _) = post_form(
        &settings,
        "/ui/bulk-import",
        &format!(
            "name=Alice&recipient_base_url={}&auth=none",
            urlencode(&recipient_url)
        ),
    )
    .await;
    post_form(
        &settings,
        &format!("{detail_path}/manifests"),
        "manifest_url=http%3A%2F%2Ftest.com",
    )
    .await;
    post_form(&settings, &format!("{detail_path}/submit-all"), "").await;

    let (_, html) = get(&settings, &detail_path).await;
    assert!(html.contains("Bulk Submit request failed!"));
    assert!(html.contains("404"));
    assert!(html.contains("Not Started"), "status unchanged on failure");
}

#[tokio::test]
async fn test_auth_without_a_server_key_reports_the_gap() {
    let settings = settings_store();
    let (status, _, html) = post_form(
        &settings,
        "/ui/bulk-import/test-auth",
        "client_id=alice&token_url=http%3A%2F%2Flocalhost%3A9%2Ftoken",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("HFS_BULK_SUBMIT_PRIVATE_KEY"));
}

fn urlencode(s: &str) -> String {
    s.replace(':', "%3A").replace('/', "%2F")
}
