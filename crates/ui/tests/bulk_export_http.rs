//! End-to-end tests for the Bulk Export workspace (`/ui/bulk-export`, #537).
//!
//! The workspace drives the server's own `$export` API through self-calls
//! addressed by the request's Host header, so these tests mount the UI over a
//! mock FHIR export backend and serve the whole thing on a real socket: the
//! kick-off and status polls loop back into the mock.

use std::sync::{Arc, Mutex};

use axum::extract::State as AxState;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Json, Router};
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::SettingsStore;

#[derive(Clone, Default)]
struct MockExport {
    /// (path, query) of each kick-off received.
    kickoffs: Arc<Mutex<Vec<(String, String)>>>,
    /// Polls answered so far; the first responds 202, later ones 200.
    polls: Arc<Mutex<u32>>,
    /// When set, kick-offs answer 400 with this body.
    reject: Arc<Mutex<Option<String>>>,
    cancels: Arc<Mutex<u32>>,
}

fn mock_fhir_app(state: MockExport) -> Router {
    async fn kickoff(
        AxState(s): AxState<MockExport>,
        uri: axum::http::Uri,
    ) -> axum::response::Response {
        s.kickoffs.lock().unwrap().push((
            uri.path().to_string(),
            uri.query().unwrap_or("").to_string(),
        ));
        if let Some(body) = s.reject.lock().unwrap().clone() {
            return (StatusCode::BAD_REQUEST, body).into_response();
        }
        (
            StatusCode::ACCEPTED,
            [("content-location", "/export-status/job-1".to_string())],
            "",
        )
            .into_response()
    }
    async fn status(AxState(s): AxState<MockExport>) -> axum::response::Response {
        let mut polls = s.polls.lock().unwrap();
        *polls += 1;
        if *polls == 1 {
            (StatusCode::ACCEPTED, [("x-progress", "18% complete")], "").into_response()
        } else {
            Json(serde_json::json!({
                "transactionTime": "2026-08-11T10:00:00Z",
                "request": "http://x/$export",
                "requiresAccessToken": false,
                "output": [
                    {"type": "Patient", "url": "http://x/files/Patient.ndjson"},
                    {"type": "Observation", "url": "http://x/files/Observation.ndjson"}
                ],
                "error": []
            }))
            .into_response()
        }
    }
    async fn cancel(AxState(s): AxState<MockExport>) -> StatusCode {
        *s.cancels.lock().unwrap() += 1;
        StatusCode::ACCEPTED
    }
    Router::new()
        .route("/$export", axum::routing::get(kickoff))
        .route("/Patient/$export", axum::routing::get(kickoff))
        .route("/Group/{id}/$export", axum::routing::get(kickoff))
        .route(
            "/export-status/{id}",
            axum::routing::get(status).delete(cancel),
        )
        .with_state(state)
}

/// Serves the mounted UI (over the mock FHIR app) on a real port; returns the
/// base URL and the mock's state handles.
async fn serve() -> (String, MockExport) {
    let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
    backend.init_schema().expect("init schema");
    let settings: Arc<dyn SettingsStore> = Arc::new(backend);

    let mock = MockExport::default();
    let app = helios_ui::mount_with_conformance_source(
        mock_fhir_app(mock.clone()),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        helios_ui::NlSearch::default(),
        None,
        Some(settings),
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), mock)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

async fn get_text(base: &str, path: &str) -> (u16, String) {
    let res = client().get(format!("{base}{path}")).send().await.unwrap();
    (res.status().as_u16(), res.text().await.unwrap())
}

async fn post_form(base: &str, path: &str, form: &[(&str, &str)]) -> (u16, String) {
    let res = client()
        .post(format!("{base}{path}"))
        .form(form)
        .send()
        .await
        .unwrap();
    let location = res
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    (res.status().as_u16(), location)
}

#[tokio::test]
async fn the_export_page_offers_scopes_types_and_filters() {
    let (base, _) = serve().await;
    let (status, html) = get_text(&base, "/ui/bulk-export").await;
    assert_eq!(status, 200);
    assert!(html.contains("What are you exporting?"));
    assert!(html.contains("Everything"));
    assert!(html.contains(r#"name="types" value="Patient""#));
    assert!(html.contains("Narrow it down"));
    assert!(html.contains("Start Export"));
}

#[tokio::test]
async fn the_export_page_uses_form_panels_with_name_and_hint_up_top() {
    let (base, _) = serve().await;
    let (status, html) = get_text(&base, "/ui/bulk-export").await;
    assert_eq!(status, 200);

    // The scope choices are the designed radio-card row (#735), and nothing
    // borrows the sticky .detail sidebar layout (#608).
    assert_eq!(html.matches(r#"class="choice-card""#).count(), 3);
    assert!(!html.contains("card detail"));

    // The Name field comes before the scope radios in the first panel.
    let name_pos = html.find(r#"name="name""#).expect("name field present");
    let scope_pos = html.find(r#"name="scope""#).expect("scope radios present");
    assert!(name_pos < scope_pos, "Name should precede the scope radios");

    // The types hint sits above the checkbox grid, not below it.
    let hint_pos = html
        .find("Leave everything unchecked to export every type.")
        .expect("types hint present");
    let grid_pos = html
        .find(r#"class="typegrid""#)
        .expect("types grid present");
    assert!(hint_pos < grid_pos, "hint should precede the types grid");
}

#[tokio::test]
async fn the_active_exports_page_uses_the_shared_back_link() {
    let (base, _) = serve().await;
    let (status, html) = get_text(&base, "/ui/bulk-export/active").await;
    assert_eq!(status, 200);
    assert!(html.contains(r#"<a class="back-link" href="/ui/bulk-export">"#));
}

#[tokio::test]
async fn starting_a_system_export_kicks_off_and_tracks_the_job() {
    let (base, mock) = serve().await;

    let (status, location) = post_form(
        &base,
        "/ui/bulk-export",
        &[
            ("name", "Everything"),
            ("scope", "system"),
            ("types", "Patient"),
            ("types", "Observation"),
            ("elements", "id,meta"),
            ("since_preset", "week"),
        ],
    )
    .await;
    assert_eq!(status, 303);
    assert_eq!(location, "/ui/bulk-export/active");

    // The mock saw one kick-off with the narrowed parameters.
    let kickoffs = mock.kickoffs.lock().unwrap().clone();
    assert_eq!(kickoffs.len(), 1);
    assert_eq!(kickoffs[0].0, "/$export");
    let q = &kickoffs[0].1;
    assert!(q.contains("_type=Patient%2CObservation"), "{q}");
    assert!(q.contains("_elements=id%2Cmeta"), "{q}");
    assert!(q.contains("_since="), "{q}");

    // The Active Exports page shows it in progress.
    let (_, html) = get_text(&base, "/ui/bulk-export/active").await;
    assert!(html.contains("Everything"));
    assert!(html.contains("In progress"));
    // The card's own poll URL (not the layout's tenant-menu hx-get).
    let card_path = html
        .split("hx-get=\"")
        .map(|s| s.split('"').next().unwrap_or(""))
        .find(|s| s.starts_with("/ui/bulk-export/active/"))
        .expect("card poll url")
        .to_string();

    // First card fetch: one poll -> 202 with progress, still polling.
    let (_, html) = get_text(&base, &card_path).await;
    assert!(html.contains("18% complete"), "{html}");
    assert!(html.contains("every 5s"));

    // Second: the mock flips to 200 -> complete with two files, no polling.
    let (_, html) = get_text(&base, &card_path).await;
    assert!(html.contains("Complete"), "{html}");
    assert!(html.contains("Patient.ndjson"));
    assert!(html.contains("Observation.ndjson"));
    assert!(!html.contains("every 5s"));
}

#[tokio::test]
async fn patient_and_group_scopes_hit_their_export_paths() {
    let (base, mock) = serve().await;

    post_form(&base, "/ui/bulk-export", &[("scope", "patient")]).await;
    post_form(
        &base,
        "/ui/bulk-export",
        &[("scope", "group"), ("group_id", "cohort-7")],
    )
    .await;

    let kickoffs = mock.kickoffs.lock().unwrap().clone();
    let paths: Vec<&str> = kickoffs.iter().map(|(p, _)| p.as_str()).collect();
    assert!(paths.contains(&"/Patient/$export"), "{paths:?}");
    assert!(paths.contains(&"/Group/cohort-7/$export"), "{paths:?}");
}

#[tokio::test]
async fn a_rejected_kickoff_lands_as_failed_and_retry_reruns_it() {
    let (base, mock) = serve().await;
    *mock.reject.lock().unwrap() =
        Some("The server ran out of time building Observation.ndjson".to_string());

    post_form(
        &base,
        "/ui/bulk-export",
        &[("name", "Diabetes registry 2024"), ("scope", "system")],
    )
    .await;

    let (_, html) = get_text(&base, "/ui/bulk-export/active").await;
    assert!(html.contains("Failed"));
    assert!(html.contains("ran out of time"));
    assert!(html.contains("Retry"));

    // Clear the failure and retry through the card's form action.
    *mock.reject.lock().unwrap() = None;
    let retry_path = html
        .split("action=\"")
        .find(|s| s.starts_with("/ui/bulk-export/active/"))
        .and_then(|s| s.split('"').next())
        .expect("retry action")
        .to_string();
    let (status, _) = post_form(&base, &retry_path, &[]).await;
    assert_eq!(status, 303);

    let (_, html) = get_text(&base, "/ui/bulk-export/active").await;
    assert!(html.contains("In progress"), "{html}");
    assert_eq!(mock.kickoffs.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn cancelling_deletes_the_job_server_side() {
    let (base, mock) = serve().await;
    post_form(&base, "/ui/bulk-export", &[("scope", "system")]).await;

    let (_, html) = get_text(&base, "/ui/bulk-export/active").await;
    let cancel_path = html
        .split("action=\"")
        .find(|s| s.starts_with("/ui/bulk-export/active/") && s.contains("/cancel"))
        .and_then(|s| s.split('"').next())
        .expect("cancel action")
        .to_string();
    let (status, _) = post_form(&base, &cancel_path, &[]).await;
    assert_eq!(status, 303);

    assert_eq!(*mock.cancels.lock().unwrap(), 1, "DELETE reached the API");
    let (_, html) = get_text(&base, "/ui/bulk-export/active").await;
    assert!(html.contains("Cancelled"));
}
