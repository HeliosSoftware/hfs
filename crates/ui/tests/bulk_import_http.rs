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

/// One test's world: the settings store and the recipient base the router is
/// mounted with — the recipient is fixed server-side now (#689), so tests
/// that need the kickoff to reach their mock recipient mount with its URL.
struct Ctx {
    settings: Arc<dyn SettingsStore>,
    recipient: String,
}

fn ctx(recipient: &str) -> Ctx {
    Ctx {
        settings: settings_store(),
        recipient: recipient.to_string(),
    }
}

fn app(ctx: &Ctx) -> Router {
    let settings = &ctx.settings;
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
        ctx.recipient.clone(),
    )
}

fn app_with_path_tenant(ctx: &Ctx, tenant: &str) -> Router {
    helios_ui::mount_with_conformance_source_and_body_limit_and_tenant_routing(
        Router::new(),
        "9.9.9",
        None,
        helios_ui::NlSearch::default(),
        None,
        Some(Arc::clone(&ctx.settings)),
        tenant.to_string(),
        Arc::new(helios_ui::StaticConformanceSource::empty()),
        FhirVersion::R4,
        None,
        ctx.recipient.clone(),
        10 * 1024 * 1024,
        true,
    )
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn get(ctx: &Ctx, uri: &str) -> (StatusCode, String) {
    let res = app(ctx)
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    (status, body_text(res).await)
}

/// POSTs a form and returns `(status, Location header, body)`.
async fn post_form(ctx: &Ctx, uri: &str, form: &str) -> (StatusCode, String, String) {
    let res = app(ctx)
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
async fn create_submission(ctx: &Ctx) -> String {
    let (status, location, _) = post_form(ctx, "/ui/bulk-import", "name=BrettTest&auth=none").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(location.starts_with("/ui/bulk-import/"), "{location}");
    location
}

#[tokio::test]
async fn the_list_page_renders_and_offers_creation() {
    let ctx = ctx("http://localhost:9/");
    let (status, html) = get(&ctx, "/ui/bulk-import").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Bulk Import"));
    assert!(html.contains("New Submission"));
    assert!(html.contains("No submissions yet"));
}

#[tokio::test]
async fn recipient_includes_public_prefix_and_selected_path_tenant() {
    let ctx = ctx("https://public.example/fhir/");

    // The recipient is fixed server-side at create time, so a spoofed Host must
    // not reach it: the stored value is the configured public base plus the
    // path tenant. The detail page is where it surfaces (#721 dropped the row
    // from the create dialog).
    let created = app_with_path_tenant(&ctx, "acme")
        .oneshot(
            Request::post("/ui/bulk-import")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("host", "spoofed.example")
                .body(Body::from("name=BrettTest&auth=none"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::SEE_OTHER);
    let detail_path = created
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("redirect to the detail page")
        .to_string();

    let response = app_with_path_tenant(&ctx, "acme")
        .oneshot(
            Request::get(&detail_path)
                .header("host", "spoofed.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("https://public.example/fhir/acme"), "{html}");
    assert!(!html.contains("spoofed.example"));
}

#[tokio::test]
async fn creating_a_submission_lands_on_its_detail_page() {
    let ctx = ctx("http://localhost:9/");
    let detail_path = create_submission(&ctx).await;

    let (status, html) = get(&ctx, &detail_path).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("BrettTest"));
    assert!(html.contains("Not Started"));
    assert!(
        html.contains("http://localhost:9"),
        "trailing slash trimmed"
    );
    assert!(html.contains("Add Manifest"));

    // And the list now shows it.
    let (_, list) = get(&ctx, "/ui/bulk-import").await;
    assert!(list.contains("BrettTest"));
}

#[tokio::test]
async fn the_detail_page_uses_the_shared_full_width_components() {
    let ctx = ctx("http://localhost:9/");
    let detail_path = create_submission(&ctx).await;
    let submission_id = detail_path.rsplit('/').next().unwrap();

    let (status, html) = get(&ctx, &detail_path).await;
    assert_eq!(status, StatusCode::OK);

    assert!(html.contains(r#"<section class="card panel bulk-import-section">"#));
    assert!(html.contains(r#"<section class="card table-card bulk-import-section">"#));
    assert_eq!(html.matches("bulk-import-section").count(), 2);
    assert!(html.contains(r#"<div class="kv-grid">"#));
    assert!(!html.contains(r#"class="card detail""#));

    // Machine-readable values stay mono while human-readable labels remain
    // ordinary proportional text.
    assert!(html.contains(r#"<span>Data Recipient</span><code>http://localhost:9</code>"#));
    assert!(html.contains(&format!(
        r#"<span>Submission ID</span><code>{submission_id}</code>"#
    )));
    assert!(html.contains(r#"<span>Submitter</span><code>"#));
    assert!(html.contains(r#"<span>Created</span><code>"#));
    assert!(html.contains(r#"<span>Status</span><div>Not Started</div>"#));
    assert!(html.contains(r#"<span>Authentication</span><div>"#));

    assert!(html.contains(r#"<a class="back-link" href="/ui/bulk-import">"#));
    assert_eq!(html.matches("btn btn--danger").count(), 1);
    assert_eq!(html.matches(r#"<th scope="col">"#).count(), 4);
    for heading in ["Manifest URL", "Last Submitted", "Submit", "Actions"] {
        assert!(html.contains(&format!(r#"<th scope="col">{heading}</th>"#)));
    }
    assert!(html.contains(r#"class="data-table__empty""#));
    assert!(html.contains(r#"class="empty-state""#));

    // The initial HTMX load is owned directly by the fragment host, so its
    // outerHTML replacement does not leave a redundant wrapper behind.
    assert!(html.contains(&format!(
        r#"<div id="bulk-status" hx-get="{detail_path}/status" hx-trigger="load" hx-swap="outerHTML"></div>"#
    )));
}

#[tokio::test]
async fn manifests_can_be_added_and_removed() {
    let ctx = ctx("http://localhost:9/");
    let detail_path = create_submission(&ctx).await;

    let (status, location, _) = post_form(
        &ctx,
        &format!("{detail_path}/manifests"),
        "manifest_url=http%3A%2F%2F10.2.1.890%2Fmanifest.local",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, detail_path);

    let (_, html) = get(&ctx, &detail_path).await;
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

    let (status, _, _) =
        post_form(&ctx, &format!("{detail_path}/manifests/{mid}/delete"), "").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (_, html) = get(&ctx, &detail_path).await;
    assert!(html.contains("No manifests yet"));
}

#[tokio::test]
async fn deleting_a_submission_returns_to_the_list() {
    let ctx = ctx("http://localhost:9/");
    let detail_path = create_submission(&ctx).await;

    let (status, location, _) = post_form(&ctx, &format!("{detail_path}/delete"), "").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, "/ui/bulk-import");

    // The detail page for a deleted submission redirects back to the list.
    let res = app(&ctx)
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
    let (recipient_url, received) = mock_recipient(StatusCode::OK).await;
    let ctx = ctx(&recipient_url);

    let (_, detail_path, _) = post_form(&ctx, "/ui/bulk-import", "name=Alice&auth=none").await;
    post_form(
        &ctx,
        &format!("{detail_path}/manifests"),
        "manifest_url=http%3A%2F%2Fexample.org%2Fexports%2Fmanifest.json&output_format=application%2Ffhir%2Bndjson",
    )
    .await;

    let (status, _, _) = post_form(&ctx, &format!("{detail_path}/submit-all"), "").await;
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
    assert_eq!(
        fhir_base, "http://example.org",
        "origin, not parent directory"
    );

    // The log recorded the attempt and the submission moved to In Progress.
    let (_, html) = get(&ctx, &detail_path).await;
    assert!(html.contains("Submitting manifest"));
    assert!(html.contains("accepted by the recipient (200)"));
    assert!(html.contains("In Progress"));
}

#[tokio::test]
async fn a_rejected_kickoff_is_logged_as_a_failure() {
    let (recipient_url, _) = mock_recipient(StatusCode::NOT_FOUND).await;
    let ctx = ctx(&recipient_url);

    let (_, detail_path, _) = post_form(&ctx, "/ui/bulk-import", "name=Alice&auth=none").await;
    post_form(
        &ctx,
        &format!("{detail_path}/manifests"),
        "manifest_url=http%3A%2F%2Ftest.com",
    )
    .await;
    post_form(&ctx, &format!("{detail_path}/submit-all"), "").await;

    let (_, html) = get(&ctx, &detail_path).await;
    // The log names the request that failed — the kick-off POST, with the
    // manifest as context — and the submission reads Failed (#686).
    assert!(html.contains("$bulk-submit → 404"));
    assert!(html.contains("(manifest http://test.com)"));
    assert!(html.contains("Failed"), "status shows the failure");
    assert!(!html.contains("Not Started"));
}

fn urlencode(s: &str) -> String {
    s.replace(':', "%3A").replace('/', "%2F")
}

#[tokio::test]
async fn a_single_manifest_can_be_submitted_from_its_row() {
    let (recipient_url, received) = mock_recipient(StatusCode::OK).await;
    let ctx = ctx(&recipient_url);

    let (_, detail_path, _) = post_form(&ctx, "/ui/bulk-import", "name=Solo&auth=none").await;
    for url in [
        "http%3A%2F%2Fone.example%2Fm.json",
        "http%3A%2F%2Ftwo.example%2Fm.json",
    ] {
        post_form(
            &ctx,
            &format!("{detail_path}/manifests"),
            &format!("manifest_url={url}"),
        )
        .await;
    }
    let (_, html) = get(&ctx, &detail_path).await;
    let marker = format!("{detail_path}/manifests/");
    let mid = html
        .split(&marker)
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .expect("manifest id")
        .to_string();

    let (status, _, _) =
        post_form(&ctx, &format!("{detail_path}/manifests/{mid}/submit"), "").await;
    assert_eq!(status, StatusCode::SEE_OTHER);

    // Only the one manifest went out.
    assert_eq!(received.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn abort_and_complete_send_status_only_kickoffs() {
    let (recipient_url, received) = mock_recipient(StatusCode::OK).await;
    let ctx = ctx(&recipient_url);

    let (_, detail_path, _) = post_form(&ctx, "/ui/bulk-import", "name=Lifecycle&auth=none").await;

    post_form(&ctx, &format!("{detail_path}/abort"), "").await;
    let (_, html) = get(&ctx, &detail_path).await;
    assert!(html.contains("Stopped"));
    assert!(html.contains("Recipient acknowledged (200)"));

    post_form(&ctx, &format!("{detail_path}/complete"), "").await;
    let (_, html) = get(&ctx, &detail_path).await;
    assert!(html.contains("Completed"));

    // Status-only kick-offs carry no manifestUrl.
    let bodies = received.lock().unwrap().clone();
    assert_eq!(bodies.len(), 2);
    let statuses: Vec<&str> = bodies
        .iter()
        .map(|b| {
            b["parameter"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == "submissionStatus")
                .and_then(|p| p["valueCoding"]["code"].as_str())
                .unwrap()
        })
        .collect();
    assert_eq!(statuses, vec!["stopped", "completed"]);
    assert!(bodies.iter().all(|b| {
        b["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["name"] != "manifestUrl")
    }));
}

#[tokio::test]
async fn a_rejected_status_change_keeps_the_status_and_logs_it() {
    let (recipient_url, _) = mock_recipient(StatusCode::CONFLICT).await;
    let ctx = ctx(&recipient_url);

    let (_, detail_path, _) = post_form(&ctx, "/ui/bulk-import", "name=Reject&auth=none").await;
    post_form(&ctx, &format!("{detail_path}/abort"), "").await;

    let (_, html) = get(&ctx, &detail_path).await;
    assert!(html.contains("Recipient rejected the status change: 409"));
    assert!(html.contains("Not Started"));
}

#[tokio::test]
async fn an_unreachable_recipient_fails_the_status_change() {
    let ctx = ctx("http://localhost:9/");
    let (_, detail_path, _) = post_form(&ctx, "/ui/bulk-import", "name=Gone&auth=none").await;
    post_form(&ctx, &format!("{detail_path}/abort"), "").await;

    let (_, html) = get(&ctx, &detail_path).await;
    assert!(html.contains("Status change failed:"));
    assert!(html.contains("Not Started"));
}

#[tokio::test]
async fn manifest_options_ride_the_kickoff() {
    let (recipient_url, received) = mock_recipient(StatusCode::OK).await;
    let ctx = ctx(&recipient_url);

    let (_, detail_path, _) = post_form(&ctx, "/ui/bulk-import", "name=Options&auth=none").await;
    post_form(
        &ctx,
        &format!("{detail_path}/manifests"),
        "manifest_url=http%3A%2F%2Fone.example%2Fm.json\
         &fhir_base_url=http%3A%2F%2Fbase.example%2Ffhir\
         &output_format=application%2Ffhir%2Bndjson\
         &file_request_headers=Authorization%3A%20Bearer%20abc%0AX-Trace%3A%201",
    )
    .await;
    post_form(&ctx, &format!("{detail_path}/submit-all"), "").await;

    let bodies = received.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1);
    let params = bodies[0]["parameter"].as_array().unwrap().clone();
    let value_of = |name: &str| {
        params
            .iter()
            .find(|p| p["name"] == name)
            .cloned()
            .unwrap_or_default()
    };
    // The explicit FHIR base wins over the manifest-derived fallback.
    assert_eq!(
        value_of("fhirBaseUrl")["valueUrl"],
        "http://base.example/fhir"
    );
    assert_eq!(
        value_of("outputFormat")["valueString"],
        "application/fhir+ndjson"
    );
    let headers: Vec<(String, String)> = params
        .iter()
        .filter(|p| p["name"] == "fileRequestHeader")
        .map(|p| {
            let part = p["part"].as_array().unwrap();
            (
                part[0]["valueString"].as_str().unwrap().to_string(),
                part[1]["valueString"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        headers,
        vec![
            ("Authorization".to_string(), "Bearer abc".to_string()),
            ("X-Trace".to_string(), "1".to_string()),
        ]
    );
}

#[tokio::test]
async fn the_page_degrades_without_a_settings_store() {
    let app = helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        None,
        helios_ui::NlSearch::default(),
        None,
        None,
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::empty()),
        FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
    );
    let res = app
        .clone()
        .oneshot(Request::get("/ui/bulk-import").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let html = body_text(res).await;
    assert!(html.contains("settings store"));

    // Creating fails loudly rather than dropping the submission.
    let res = app
        .oneshot(
            Request::post("/ui/bulk-import")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("name=X"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_GATEWAY);
}

/// A P-384 key generated for this test alone; it protects nothing.
const TEST_EC_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIG2AgEAMBAGByqGSM49AgEGBSuBBAAiBIGeMIGbAgEBBDBUhaaxv3+geGibr3l1
4zsp7xiv25gKhynl2UEOwaEy/IubzysZPBLq/D/UFqR2Mn+hZANiAAR578fV49hI
eGeRazDFYoj+Q7VBe0Eu60f9CorENX8+mpJzUDFB4p48cH1tgS1KvQgjqMbnR/RA
P53KqB3RqJVeovdzKPFaCZjbwHj1fZMcKr08BqYQo6GgRAHpjMFY8o8=
-----END PRIVATE KEY-----";

/// A stub authorization server: `/token` answers with the given body.
async fn mock_token_endpoint(status: StatusCode, body: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/token",
        axum::routing::post(move || async move {
            (status, [("content-type", "application/json")], body)
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/token")
}

/// One sequential test for everything that reads the process-wide signing-key
/// environment variable — phases must not run concurrently with each other,
/// and nothing else in this binary reads the variable.
#[tokio::test]
async fn backend_services_auth_mints_and_attaches_tokens() {
    let ctx = ctx("http://localhost:9/");
    let token_url = mock_token_endpoint(StatusCode::OK, r#"{"access_token":"tok-123"}"#).await;

    // Phase 1 — no key configured: the gap is reported, not swallowed.
    unsafe { std::env::remove_var("HFS_BULK_SUBMIT_PRIVATE_KEY") };
    let (status, _, html) = post_form(
        &ctx,
        "/ui/bulk-import/test-auth",
        &format!("client_id=alice&token_url={}", urlencode(&token_url)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("HFS_BULK_SUBMIT_PRIVATE_KEY"));

    unsafe { std::env::set_var("HFS_BULK_SUBMIT_PRIVATE_KEY", TEST_EC_KEY) };

    // Phase 2 — missing client id fails before any signing.
    let (_, _, html) = post_form(
        &ctx,
        "/ui/bulk-import/test-auth",
        &format!("client_id=&token_url={}", urlencode(&token_url)),
    )
    .await;
    assert!(html.contains("missing client_id"));

    // Phase 3 — a working endpoint: the mint succeeds.
    let (_, _, html) = post_form(
        &ctx,
        "/ui/bulk-import/test-auth",
        &format!("client_id=alice&token_url={}", urlencode(&token_url)),
    )
    .await;
    assert!(html.contains("✓"), "success fragment: {html}");

    // Phase 4 — the endpoint rejects: status is surfaced.
    let denied =
        mock_token_endpoint(StatusCode::BAD_REQUEST, r#"{"error":"invalid_client"}"#).await;
    let (_, _, html) = post_form(
        &ctx,
        "/ui/bulk-import/test-auth",
        &format!("client_id=alice&token_url={}", urlencode(&denied)),
    )
    .await;
    assert!(html.contains("token endpoint answered 400"));

    // Phase 5 — a token response without access_token is an error.
    let empty = mock_token_endpoint(StatusCode::OK, r#"{"scope":"none"}"#).await;
    let (_, _, html) = post_form(
        &ctx,
        "/ui/bulk-import/test-auth",
        &format!("client_id=alice&token_url={}", urlencode(&empty)),
    )
    .await;
    assert!(html.contains("no access_token"));

    // Phase 6 — a backend-services submission attaches the Bearer token to
    // the kick-off it POSTs at the recipient.
    let (recipient_url, seen_auth) = mock_recipient_capturing_auth().await;
    // Same settings (the registered signing key lives there), phase-local
    // recipient: the kick-off must reach this phase's capturing mock.
    let ctx = Ctx {
        settings: Arc::clone(&ctx.settings),
        recipient: recipient_url.clone(),
    };
    let (_, detail_path, _) = post_form(
        &ctx,
        "/ui/bulk-import",
        &format!(
            "name=Authy&auth=backend-services&client_id=alice&token_url={}",
            urlencode(&token_url)
        ),
    )
    .await;
    post_form(
        &ctx,
        &format!("{detail_path}/manifests"),
        "manifest_url=http%3A%2F%2Fone.example%2Fm.json",
    )
    .await;
    post_form(&ctx, &format!("{detail_path}/submit-all"), "").await;

    assert_eq!(
        seen_auth.lock().unwrap().as_deref(),
        Some("Bearer tok-123"),
        "the kick-off carried the minted token"
    );
    let (_, html) = get(&ctx, &detail_path).await;
    assert!(html.contains("accepted by the recipient (200)"));

    // Phase 7 — the RS384 signing path works with an RSA key.
    unsafe { std::env::set_var("HFS_BULK_SUBMIT_PRIVATE_KEY", TEST_RSA_KEY) };
    unsafe { std::env::set_var("HFS_BULK_SUBMIT_SIGNING_ALG", "RS384") };
    let (_, _, html) = post_form(
        &ctx,
        "/ui/bulk-import/test-auth",
        &format!("client_id=alice&token_url={}", urlencode(&token_url)),
    )
    .await;
    assert!(html.contains("✓"), "RS384 mint: {html}");
    unsafe { std::env::remove_var("HFS_BULK_SUBMIT_SIGNING_ALG") };

    unsafe { std::env::remove_var("HFS_BULK_SUBMIT_PRIVATE_KEY") };
}

/// An RSA-2048 key generated for this test alone; it protects nothing.
const TEST_RSA_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDH1Fo0WDU06NHP
2JGiQSqMn40gjDALiqnTgxBFi/9i+Th4cTKXCYMmcpzQKCyWIhgtNF8no672XQsD
O3zixg7YdGNJPdaO90lJmY8k7jm/jfGqzgnEYunmD1W1RY0OqkUo+Ni/4FVo/6uJ
lZFrP7lTE5ZsY90CZc1/isVbYWgwZnhcOCNQfm5EkeMgNPShizoJJpEgGadnVlP1
9sWEckO1lEY5Ed5fl6Rhb1EO+c7cCSug4WRATWSEP5pUhoulN6JUyxy7BAe9YZ5d
Lx7Du6SBDmG8K8lOzqvAA1vVqcA85JYBZeMd+sTYCmTsW//bnBAymZRKMcbosyjk
UdDZxR7VAgMBAAECggEAAPBerSqLwBHk5eSflh9zJ/1niYZTszqABi2/p6p/Xqpl
yxg79garxc0wTYAHQsC9RNt7yKQTj0aSRNWFMtzGHOK78rdXfYBnPpXczZePnw1u
j/Dv8LZpCtNwDksptJfWC6IcYD1IPl4ye5HZHe1CJuooCIJ3nYCOV3zhM31pszjo
b4r+cm6SDxFnuAYtH6ngvvsN+pGht/DyN6u1KY5d06JNQ0aDCuGsRayZ55jHJubU
1VRvqeBmcn4xkiWkvcfQuE72rbojoTCtupOaVmLjiOxd7F1IvNKKbbJIBbMlc7VX
dnzMTLF2MCCYYtRzZUIlbZ7qSVSSOh89SUA8aNg0iQKBgQD7PFUU910UfQ77wIEr
gOl062yskJMT97hfBl6HGTGfpmnHqS8L7ZZmTybNGznS4rZuiPC/uUEGMr01tnRD
GlVBpugfe/8AO6J2nUY9e1IBplvhwB7uuy8MpwmkL1CzbDcat+s5f7GDvlLK6u18
XKKtnK8GVaHzXXjSjyaFrE6iHwKBgQDLnnWw7SqW4+0BaenhWC51iHd2GeIQJQ0s
cXFs5N6+i87UVaqSErAdFMRt08ocieO5t2v03YT2hiYSFxxgdpGDEG18e4/T/sQW
T18fqo9iFaFmZ2Hzk3uRI2HG4FfYetwyv97nVEH0EibrKd/dSnZ8JbB87VR0LfhO
c4srqPnoiwKBgF3Db5GKnE+IOO5WMx8UVozPTFi/AFVEb6fvTZooGfAWgIYGq0tN
WYNHaRjFX3hIKoPoUcmMDyuMBjekp5Ffo5AEBb+yXEIu/3w7SDqr6rg46TPAqwq4
C2AyexOuoPTFn282UvC7qnmbr3SR5x4xyHj48A1yKiYUrYIP8PWUkChLAoGAWHkV
sjaa1s1aYc7fbKagKTmOjqZYb6NpwfHY0vPvROQCjohagPXVyA0J/J6VpyjS5hMo
uVC3QVawnBOmpNNgDo7Iw9n8eKSuFvON5Xh6rKexZYluKiPfAQVaqss34DwiCXsN
I36c2aw5dNzRBJoiOXc25FFK7OA8j/nscqANVlkCgYEAhgRQc/atd/GYETp2a8sn
8nYahjwwC4lU753b9/akn3GQcE9T65yRoeqFi63v7AAwxwIUFZnxv4CuI/sgSYEL
t+xX6yYqCQTVR3Y9XZgUWohxnpky0zdqat1BAJDzgRCWnZ75B++gDcLgbTYeqaVI
CbOPbIKiVSNWN6XfYk/ZDEU=
-----END PRIVATE KEY-----";

/// A recipient that records the Authorization header of the kick-off.
async fn mock_recipient_capturing_auth() -> (String, Arc<std::sync::Mutex<Option<String>>>) {
    let seen: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::clone(&seen);
    let recipient = Router::new()
        .route(
            "/$bulk-submit",
            axum::routing::post(
                move |axum::extract::State(seen): axum::extract::State<
                    Arc<std::sync::Mutex<Option<String>>>,
                >,
                      headers: axum::http::HeaderMap| async move {
                    *seen.lock().unwrap() = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(String::from);
                    axum::Json(serde_json::json!({"resourceType": "Parameters"}))
                },
            ),
        )
        .with_state(state);
    tokio::spawn(async move { axum::serve(listener, recipient).await.unwrap() });
    (format!("http://{addr}"), seen)
}

#[tokio::test]
async fn unknown_ids_redirect_instead_of_crashing() {
    let ctx = ctx("http://localhost:9/");
    let ghost = "/ui/bulk-import/00000000-0000-4000-8000-000000000000";

    for uri in [
        format!("{ghost}/manifests"),
        format!("{ghost}/manifests/also-missing/delete"),
        format!("{ghost}/manifests/also-missing/submit"),
        format!("{ghost}/submit-all"),
        format!("{ghost}/abort"),
    ] {
        let (status, _, _) = post_form(&ctx, &uri, "manifest_url=http%3A%2F%2Fx").await;
        assert_eq!(status, StatusCode::SEE_OTHER, "{uri}");
    }
}

#[tokio::test]
async fn a_missing_manifest_id_submits_nothing() {
    let (recipient_url, received) = mock_recipient(StatusCode::OK).await;
    let ctx = ctx(&recipient_url);
    let (_, detail_path, _) = post_form(&ctx, "/ui/bulk-import", "name=Ghost&auth=none").await;
    post_form(
        &ctx,
        &format!("{detail_path}/manifests/no-such-manifest/submit"),
        "",
    )
    .await;
    assert_eq!(received.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn an_unreachable_recipient_fails_the_manifest_submit() {
    let ctx = ctx("http://localhost:9/");
    let (_, detail_path, _) = post_form(&ctx, "/ui/bulk-import", "name=Down&auth=none").await;
    post_form(
        &ctx,
        &format!("{detail_path}/manifests"),
        "manifest_url=http%3A%2F%2Fone.example%2Fm.json",
    )
    .await;
    post_form(&ctx, &format!("{detail_path}/submit-all"), "").await;

    let (_, html) = get(&ctx, &detail_path).await;
    assert!(html.contains("Bulk Submit request failed:"));
    assert!(html.contains("$bulk-submit"), "the failed target is named");
    assert!(html.contains("Failed"), "status shows the failure");
}

/// A recipient implementing the full status flow: kick-off returns a
/// Content-Location poll URL; the first poll answers 202 + X-Progress, the
/// second 200 with a status manifest.
async fn mock_recipient_with_status() -> (String, Arc<std::sync::Mutex<Vec<serde_json::Value>>>) {
    use axum::extract::State as AxState;
    #[derive(Clone)]
    struct S {
        kickoffs: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
        polls: Arc<std::sync::Mutex<u32>>,
        base: Arc<std::sync::Mutex<String>>,
    }
    let state = S {
        kickoffs: Arc::new(std::sync::Mutex::new(Vec::new())),
        polls: Arc::new(std::sync::Mutex::new(0)),
        base: Arc::new(std::sync::Mutex::new(String::new())),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    *state.base.lock().unwrap() = format!("http://{addr}");
    let kickoffs = Arc::clone(&state.kickoffs);
    let app = Router::new()
        .route(
            "/$bulk-submit",
            axum::routing::post(
                |AxState(s): AxState<S>, axum::Json(b): axum::Json<serde_json::Value>| async move {
                    s.kickoffs.lock().unwrap().push(b);
                    axum::Json(serde_json::json!({"resourceType": "OperationOutcome"}))
                },
            ),
        )
        .route(
            "/$bulk-submit-status",
            axum::routing::post(|AxState(s): AxState<S>| async move {
                let base = s.base.lock().unwrap().clone();
                (
                    StatusCode::ACCEPTED,
                    [("content-location", format!("{base}/poll"))],
                    "",
                )
            }),
        )
        .route(
            "/poll",
            axum::routing::get(|AxState(s): AxState<S>| async move {
                let mut polls = s.polls.lock().unwrap();
                *polls += 1;
                if *polls == 1 {
                    (
                        StatusCode::ACCEPTED,
                        [
                            ("x-progress", "processing 0% complete".to_string()),
                            ("content-type", "text/plain".to_string()),
                        ],
                        String::new(),
                    )
                        .into_response()
                } else {
                    axum::Json(serde_json::json!({
                        "output": [{"type": "Patient", "url": "http://x/1.ndjson"}],
                        "error": []
                    }))
                    .into_response()
                }
            }),
        )
        .with_state(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), kickoffs)
}

use axum::response::IntoResponse;

#[tokio::test]
async fn status_polling_tracks_progress_and_lands_the_result() {
    let (recipient_url, kickoffs) = mock_recipient_with_status().await;
    let ctx = ctx(&recipient_url);

    let (_, detail_path, _) = post_form(
        &ctx,
        "/ui/bulk-import",
        "name=Polling&auth=none&submitter_system=http%3A%2F%2Fexample.org%2Fsubmitters&submitter_value=acme&submission_id=pinned-42",
    )
    .await;
    assert!(detail_path.ends_with("/pinned-42"), "{detail_path}");

    post_form(
        &ctx,
        &format!("{detail_path}/manifests"),
        "manifest_url=http%3A%2F%2Fone.example%2Fm.json",
    )
    .await;
    post_form(&ctx, &format!("{detail_path}/submit-all"), "").await;

    // The submit kick-off carried the pinned id and the custom submitter,
    // and the status kick-off went out with the same identity.
    let bodies = kickoffs.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1, "one manifest kick-off");
    let submitter = bodies[0]["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "submitter")
        .unwrap()["valueIdentifier"]
        .clone();
    assert_eq!(submitter["system"], "http://example.org/submitters");
    assert_eq!(submitter["value"], "acme");

    // First status fetch: one poll happens -> 202 progress recorded, and the
    // fragment keeps polling.
    let (status, html) = get(&ctx, &format!("{detail_path}/status")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("processing 0% complete"), "{html}");
    assert!(html.contains("every 5s"), "keeps polling: {html}");
    assert!(html.contains(r#"id="bulk-status" class="card panel bulk-import-section""#));
    assert!(html.contains(r#"class="kv-grid""#));
    assert!(!html.contains(r#"class="card detail""#));

    // Second fetch: the mock flips to 200 -> result summary, polling stops.
    let (_, html) = get(&ctx, &format!("{detail_path}/status")).await;
    assert!(!html.contains("every 5s"), "polling stopped: {html}");
    assert!(html.contains("Output files"), "{html}");
    assert!(html.contains(r#"id="bulk-status" class="card panel bulk-import-section""#));
    assert!(html.contains(r#"class="kv-grid""#));
    assert!(html.contains("Processing finished at <code>"), "{html}");
    assert!(!html.contains(r#"class="card detail""#));

    // The log recorded the whole journey and the detail shows the summary.
    let (_, detail) = get(&ctx, &detail_path).await;
    assert!(detail.contains("Bulk status kick-off request"));
    assert!(detail.contains("got 200 OK"));
}

/// The UI route became a permanent redirect to the server-level JWKS when
/// `/.well-known/bulk-submit-jwks.json` landed; existing registrations keep
/// working through it.
#[tokio::test]
async fn the_keys_endpoint_redirects_to_the_well_known_jwks() {
    let ctx = ctx("http://localhost:9/");
    let res = app(&ctx)
        .oneshot(
            Request::get("/ui/bulk-import/keys")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        res.headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/.well-known/bulk-submit-jwks.json")
    );
}

#[tokio::test]
async fn the_empty_manifest_is_served() {
    let ctx = ctx("http://localhost:9/");
    let (status, body) = get(&ctx, "/ui/bulk-import/empty-manifest.json").await;
    assert_eq!(status, StatusCode::OK);
    let m: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(m["output"].as_array().unwrap().len(), 0);
    assert_eq!(m["requiresAccessToken"], false);
}

#[tokio::test]
async fn replacing_a_manifest_sends_replaces_manifest_url() {
    let (recipient_url, received) = mock_recipient(StatusCode::OK).await;
    let ctx = ctx(&recipient_url);
    let (_, detail_path, _) = post_form(&ctx, "/ui/bulk-import", "name=R&auth=none").await;
    post_form(
        &ctx,
        &format!("{detail_path}/manifests"),
        "manifest_url=http%3A%2F%2Fold.example%2Fm.json",
    )
    .await;
    let (_, html) = get(&ctx, &detail_path).await;
    let marker = format!("{detail_path}/manifests/");
    let mid = html
        .split(&marker)
        .nth(1)
        .and_then(|r| r.split('/').next())
        .unwrap()
        .to_string();

    post_form(
        &ctx,
        &format!("{detail_path}/manifests/{mid}/replace"),
        "manifest_url=http%3A%2F%2Fnew.example%2Fm.json",
    )
    .await;

    let bodies = received.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1);
    let params = bodies[0]["parameter"].as_array().unwrap();
    let get_p = |n: &str| {
        params
            .iter()
            .find(|p| p["name"] == n)
            .cloned()
            .unwrap_or_default()
    };
    assert_eq!(
        get_p("manifestUrl")["valueUrl"],
        "http://new.example/m.json"
    );
    assert_eq!(
        get_p("replacesManifestUrl")["valueUrl"],
        "http://old.example/m.json"
    );

    let (_, html) = get(&ctx, &detail_path).await;
    assert!(html.contains("http://new.example/m.json"));
    assert!(html.contains("Replacement accepted (200)"));
}

#[tokio::test]
async fn aborting_one_manifest_replaces_it_with_the_empty_manifest() {
    let (recipient_url, received) = mock_recipient(StatusCode::OK).await;
    let ctx = ctx(&recipient_url);
    let (_, detail_path, _) = post_form(&ctx, "/ui/bulk-import", "name=A&auth=none").await;
    post_form(
        &ctx,
        &format!("{detail_path}/manifests"),
        "manifest_url=http%3A%2F%2Fold.example%2Fm.json",
    )
    .await;
    let (_, html) = get(&ctx, &detail_path).await;
    let marker = format!("{detail_path}/manifests/");
    let mid = html
        .split(&marker)
        .nth(1)
        .and_then(|r| r.split('/').next())
        .unwrap()
        .to_string();

    post_form(&ctx, &format!("{detail_path}/manifests/{mid}/abort"), "").await;

    let bodies = received.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1);
    let params = bodies[0]["parameter"].as_array().unwrap();
    let get_p = |n: &str| {
        params
            .iter()
            .find(|p| p["name"] == n)
            .cloned()
            .unwrap_or_default()
    };
    assert!(
        get_p("manifestUrl")["valueUrl"]
            .as_str()
            .unwrap()
            .ends_with("/ui/bulk-import/empty-manifest.json")
    );
    assert_eq!(
        get_p("replacesManifestUrl")["valueUrl"],
        "http://old.example/m.json"
    );

    let (_, html) = get(&ctx, &detail_path).await;
    assert!(html.contains("Abort accepted (200)"));
}

/// #686's repro: a recipient that is a plain static file server answers 501
/// with an HTML error page. The log explains instead of pasting markup.
#[tokio::test]
async fn a_non_fhir_recipient_error_is_summarized_not_pasted() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let recipient = Router::new().fallback(|| async {
        (
            StatusCode::NOT_IMPLEMENTED,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            "<!DOCTYPE HTML><html><body><h1>Error response</h1><p>Error code: 501</p></body></html>",
        )
    });
    tokio::spawn(async move { axum::serve(listener, recipient).await.unwrap() });
    let ctx = ctx(&format!("http://{addr}"));

    let (_, detail_path, _) = post_form(&ctx, "/ui/bulk-import", "name=Static&auth=none").await;
    post_form(
        &ctx,
        &format!("{detail_path}/manifests"),
        "manifest_url=http%3A%2F%2Fone.example%2Fm.json",
    )
    .await;
    post_form(&ctx, &format!("{detail_path}/submit-all"), "").await;

    let (_, html) = get(&ctx, &detail_path).await;
    assert!(html.contains("$bulk-submit → 501"));
    assert!(
        html.contains("not a FHIR resource"),
        "explains the mismatch"
    );
    assert!(!html.contains("Error code: 501"), "markup is not pasted");
    assert!(html.contains("Failed"));
}
