//! End-to-end tests for the Bulk Export workspace (`/ui/bulk-export`, #537).
//!
//! The workspace drives the server's own `$export` API through self-calls
//! addressed by its configured public base, so these tests mount the UI over a
//! mock FHIR export backend and serve the whole thing on a real socket: the
//! kick-off and status polls loop back into the mock.

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::{Path as AxPath, State as AxState};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::SettingsStore;

const REST_JOB_ID: &str = "11111111-1111-4111-8111-111111111111";

type SeenKickoff = (String, String, Option<String>);
type SeenPoll = (String, Option<String>);

#[derive(Debug, Clone)]
struct MockOutput {
    resource_type: String,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SeenDownload {
    path: String,
    authorization: Option<String>,
    tenant: Option<String>,
}

#[derive(Clone, Debug)]
struct CapturedRequest {
    method: Method,
    path: String,
    query: String,
    authorization: String,
    tenant: String,
    accept: String,
    content_type: String,
    body: String,
}

fn capture(
    method: Method,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: Bytes,
) -> CapturedRequest {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string()
    };
    CapturedRequest {
        method,
        path: uri.path().to_string(),
        query: uri.query().unwrap_or_default().to_string(),
        authorization: header("authorization"),
        tenant: header("x-tenant-id"),
        accept: header("accept"),
        content_type: header("content-type"),
        body: String::from_utf8_lossy(&body).to_string(),
    }
}

#[derive(Default)]
struct RequestGate {
    reached: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[derive(Clone)]
struct MockExport {
    /// (path, query, tenant header) of each kick-off received.
    kickoffs: Arc<Mutex<Vec<SeenKickoff>>>,
    poll_requests: Arc<Mutex<Vec<SeenPoll>>>,
    /// Public prefix advertised in status and output URLs. The mock is still
    /// reached through its separate loopback listener.
    advertised_base: Arc<Mutex<Option<String>>>,
    /// Polls answered so far; the first responds 202, later ones 200.
    polls: Arc<Mutex<u32>>,
    /// When set, kick-offs answer 400 with this body.
    reject: Arc<Mutex<Option<String>>>,
    cancels: Arc<Mutex<u32>>,
    delete_status: Arc<Mutex<StatusCode>>,
    delete_statuses: Arc<Mutex<VecDeque<StatusCode>>>,
    requires_access_token: Arc<Mutex<bool>>,
    outputs: Arc<Mutex<Vec<MockOutput>>>,
    downloads: Arc<Mutex<Vec<SeenDownload>>>,
    fail_output_after_chunk: Arc<Mutex<bool>>,
    status_gate: Arc<Mutex<Option<Arc<RequestGate>>>>,
    delete_gate: Arc<Mutex<Option<Arc<RequestGate>>>>,
    kickoff_gate: Arc<Mutex<Option<Arc<RequestGate>>>>,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    patients: Arc<Mutex<Vec<serde_json::Value>>>,
    search_status: Arc<Mutex<u16>>,
    patient_read_status: Arc<Mutex<u16>>,
    patient_read_body: Arc<Mutex<Option<String>>>,
}

impl Default for MockExport {
    fn default() -> Self {
        Self {
            kickoffs: Default::default(),
            poll_requests: Default::default(),
            advertised_base: Default::default(),
            polls: Default::default(),
            reject: Default::default(),
            cancels: Default::default(),
            delete_status: Arc::new(Mutex::new(StatusCode::ACCEPTED)),
            delete_statuses: Default::default(),
            requires_access_token: Arc::new(Mutex::new(true)),
            outputs: Arc::new(Mutex::new(vec![
                MockOutput {
                    resource_type: "Patient".to_string(),
                    path: format!("/export-file/{REST_JOB_ID}/Patient-0"),
                },
                MockOutput {
                    resource_type: "Observation".to_string(),
                    path: format!("/export-file/{REST_JOB_ID}/Observation-0"),
                },
            ])),
            downloads: Default::default(),
            fail_output_after_chunk: Default::default(),
            status_gate: Default::default(),
            delete_gate: Default::default(),
            kickoff_gate: Default::default(),
            requests: Default::default(),
            patients: Default::default(),
            search_status: Arc::new(Mutex::new(200)),
            patient_read_status: Arc::new(Mutex::new(200)),
            patient_read_body: Default::default(),
        }
    }
}

fn mock_fhir_app(state: MockExport) -> Router {
    async fn kickoff(
        AxState(s): AxState<MockExport>,
        method: Method,
        headers: HeaderMap,
        uri: axum::http::Uri,
        body: Bytes,
    ) -> axum::response::Response {
        s.requests
            .lock()
            .unwrap()
            .push(capture(method, uri.clone(), headers.clone(), body));
        s.kickoffs.lock().unwrap().push((
            uri.path().to_string(),
            uri.query().unwrap_or("").to_string(),
            headers
                .get("x-tenant-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
        ));
        if let Some(body) = s.reject.lock().unwrap().clone() {
            return (StatusCode::BAD_REQUEST, body).into_response();
        }
        let gate = s.kickoff_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.reached.notify_one();
            gate.release.notified().await;
        }
        let content_location = s
            .advertised_base
            .lock()
            .unwrap()
            .as_deref()
            .map(|base| format!("{base}/export-status/{REST_JOB_ID}"))
            .unwrap_or_else(|| format!("/export-status/{REST_JOB_ID}"));
        (
            StatusCode::ACCEPTED,
            [("content-location", content_location)],
            "",
        )
            .into_response()
    }
    async fn status(
        AxState(s): AxState<MockExport>,
        method: Method,
        headers: HeaderMap,
        uri: axum::http::Uri,
        body: Bytes,
    ) -> axum::response::Response {
        s.requests
            .lock()
            .unwrap()
            .push(capture(method, uri.clone(), headers.clone(), body));
        let gate = s.status_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.reached.notify_one();
            gate.release.notified().await;
        }
        s.poll_requests.lock().unwrap().push((
            uri.path().to_string(),
            headers
                .get("x-tenant-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
        ));
        let mut polls = s.polls.lock().unwrap();
        *polls += 1;
        if *polls == 1 {
            (StatusCode::ACCEPTED, [("x-progress", "18% complete")], "").into_response()
        } else {
            let advertised_base = s.advertised_base.lock().unwrap().clone();
            let host = headers["host"].to_str().unwrap();
            let output: Vec<_> = s
                .outputs
                .lock()
                .unwrap()
                .iter()
                .map(|output| {
                    serde_json::json!({
                        "type": output.resource_type,
                        "url": advertised_base
                            .as_deref()
                            .map(|base| format!("{base}{}", output.path))
                            .unwrap_or_else(|| format!("http://{host}{}", output.path)),
                    })
                })
                .collect();
            Json(serde_json::json!({
                "transactionTime": "2026-08-11T10:00:00Z",
                "request": "http://x/$export",
                "requiresAccessToken": *s.requires_access_token.lock().unwrap(),
                "output": output,
                "error": []
            }))
            .into_response()
        }
    }
    async fn cancel(
        AxState(s): AxState<MockExport>,
        method: Method,
        headers: HeaderMap,
        uri: axum::http::Uri,
        body: Bytes,
    ) -> StatusCode {
        s.requests
            .lock()
            .unwrap()
            .push(capture(method, uri, headers, body));
        let gate = s.delete_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.reached.notify_one();
            gate.release.notified().await;
        }
        *s.cancels.lock().unwrap() += 1;
        s.delete_statuses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| *s.delete_status.lock().unwrap())
    }
    async fn output(
        AxState(s): AxState<MockExport>,
        AxPath((_job, part)): AxPath<(String, String)>,
        headers: HeaderMap,
        uri: axum::http::Uri,
    ) -> axum::response::Response {
        s.downloads.lock().unwrap().push(SeenDownload {
            path: uri.path().to_string(),
            authorization: headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
            tenant: headers
                .get("x-tenant-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
        });
        let first = format!(
            "{{\"resourceType\":\"{}\",\"id\":\"1\"}}\n",
            part.split('-').next().unwrap()
        );
        if *s.fail_output_after_chunk.lock().unwrap() {
            let stream = async_stream::stream! {
                yield Ok::<Bytes, std::io::Error>(Bytes::from(first));
                yield Err(std::io::Error::other("forced body failure"));
            };
            Body::from_stream(stream).into_response()
        } else {
            let second = format!(
                "{{\"resourceType\":\"{}\",\"id\":\"2\"}}\n",
                part.split('-').next().unwrap()
            );
            let stream = async_stream::stream! {
                yield Ok::<Bytes, std::io::Error>(Bytes::from(first));
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                yield Ok::<Bytes, std::io::Error>(Bytes::from(second));
            };
            Body::from_stream(stream).into_response()
        }
    }
    async fn presigned(
        AxState(s): AxState<MockExport>,
        AxPath(part): AxPath<String>,
        headers: HeaderMap,
        uri: axum::http::Uri,
    ) -> axum::response::Response {
        output(
            AxState(s),
            AxPath((REST_JOB_ID.to_string(), part)),
            headers,
            uri,
        )
        .await
    }
    async fn tenant_output(
        AxState(s): AxState<MockExport>,
        AxPath((_tenant, job, part)): AxPath<(String, String, String)>,
        headers: HeaderMap,
        uri: axum::http::Uri,
    ) -> axum::response::Response {
        output(AxState(s), AxPath((job, part)), headers, uri).await
    }
    async fn redirect() -> axum::response::Response {
        (
            StatusCode::FOUND,
            [("location", format!("/export-file/{REST_JOB_ID}/Patient-0"))],
        )
            .into_response()
    }

    async fn patient_search(
        AxState(s): AxState<MockExport>,
        method: Method,
        uri: axum::http::Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> axum::response::Response {
        s.requests
            .lock()
            .unwrap()
            .push(capture(method, uri, headers, body));
        let status = StatusCode::from_u16(*s.search_status.lock().unwrap()).unwrap();
        if !status.is_success() {
            return status.into_response();
        }
        Json(serde_json::json!({
            "resourceType": "Bundle",
            "type": "searchset",
            "entry": s.patients.lock().unwrap().iter()
                .map(|resource| serde_json::json!({"resource": resource}))
                .collect::<Vec<_>>()
        }))
        .into_response()
    }

    async fn patient_read(
        AxState(s): AxState<MockExport>,
        AxPath(params): AxPath<HashMap<String, String>>,
        method: Method,
        uri: axum::http::Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> axum::response::Response {
        s.requests
            .lock()
            .unwrap()
            .push(capture(method, uri, headers, body));
        let status = StatusCode::from_u16(*s.patient_read_status.lock().unwrap()).unwrap();
        if let Some(body) = s.patient_read_body.lock().unwrap().clone() {
            return (status, body).into_response();
        }
        if !status.is_success() {
            return status.into_response();
        }
        let id = params.get("id").expect("route includes id");
        s.patients
            .lock()
            .unwrap()
            .iter()
            .find(|patient| patient["id"] == *id)
            .cloned()
            .map(Json)
            .map(IntoResponse::into_response)
            .unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
    }

    Router::new()
        .route("/$export", axum::routing::get(kickoff).post(kickoff))
        .route(
            "/{tenant}/$export",
            axum::routing::get(kickoff).post(kickoff),
        )
        .route(
            "/Patient/$export",
            axum::routing::get(kickoff).post(kickoff),
        )
        .route(
            "/{tenant}/Patient/$export",
            axum::routing::get(kickoff).post(kickoff),
        )
        .route("/Group/{id}/$export", axum::routing::get(kickoff))
        .route("/{tenant}/Group/{id}/$export", axum::routing::get(kickoff))
        .route("/Patient/_search", axum::routing::post(patient_search))
        .route("/Patient/{id}", axum::routing::get(patient_read))
        .route(
            "/{tenant}/Patient/_search",
            axum::routing::post(patient_search),
        )
        .route("/{tenant}/Patient/{id}", axum::routing::get(patient_read))
        .route(
            "/export-status/{id}",
            axum::routing::get(status).delete(cancel),
        )
        .route(
            "/{tenant}/export-status/{id}",
            axum::routing::get(status).delete(cancel),
        )
        .route("/export-file/{id}/{part}", axum::routing::get(output))
        .route(
            "/{tenant}/export-file/{id}/{part}",
            axum::routing::get(tenant_output),
        )
        .route("/presigned/{part}", axum::routing::get(presigned))
        .route("/redirect/{part}", axum::routing::get(redirect))
        .with_state(state)
}

async fn inject_test_principal(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if let Some(subject) = request
        .headers()
        .get("x-test-user")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
    {
        request.extensions_mut().insert(helios_auth::Principal {
            subject,
            issuer: "test".to_string(),
            tenant_id: None,
            scopes: helios_auth::scope::ScopeSet::empty(),
            jti: None,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            custom_claims: Default::default(),
        });
    }
    next.run(request).await
}

/// Serves the mounted UI (over the mock FHIR app) on a real port; returns the
/// base URL and the mock's state handles.
async fn serve_with_settings(settings_available: bool) -> (String, MockExport, Arc<SqliteBackend>) {
    let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite"));
    backend.init_schema().expect("init schema");
    let settings: Option<Arc<dyn SettingsStore>> =
        settings_available.then(|| backend.clone() as Arc<dyn SettingsStore>);

    let mock = MockExport::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let app = helios_ui::mount_with_conformance_source(
        mock_fhir_app(mock.clone()),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        helios_ui::NlSearch::default(),
        None,
        settings,
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        FhirVersion::R4,
        None,
        base.clone(),
        None,
    )
    .layer(axum::middleware::from_fn(inject_test_principal));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, mock, backend)
}

async fn serve() -> (String, MockExport, Arc<SqliteBackend>) {
    serve_with_settings(true).await
}

async fn serve_with_runtime(
    patient_name_search: helios_ui::PatientNameSearchSupport,
    tenant_path_routing: bool,
) -> (String, MockExport, Arc<SqliteBackend>) {
    let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite"));
    backend.init_schema().expect("init schema");
    let settings: Arc<dyn SettingsStore> = backend.clone();
    let mock = MockExport::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let app = helios_ui::mount_with_conformance_source_and_runtime(
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
        base.clone(),
        10 * 1024 * 1024,
        tenant_path_routing,
        None,
        base.clone(),
        patient_name_search,
    )
    .layer(axum::middleware::from_fn(inject_test_principal));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, mock, backend)
}

async fn serve_with_separate_public_base(
    public_base: &str,
    tenant_path_routing: bool,
) -> (String, MockExport, Arc<SqliteBackend>) {
    let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite"));
    backend.init_schema().expect("init schema");
    let settings: Option<Arc<dyn SettingsStore>> = Some(backend.clone());
    let mock = MockExport::default();
    *mock.advertised_base.lock().unwrap() = Some(if tenant_path_routing {
        format!("{public_base}/default")
    } else {
        public_base.to_string()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let self_base = format!("http://{addr}");
    let app = helios_ui::mount_with_body_limit_and_tenant_routing(
        mock_fhir_app(mock.clone()),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        helios_ui::NlSearch::default(),
        None,
        settings,
        "default".to_string(),
        self_base.clone(),
        Arc::new(helios_auth::NoOpOutboundAuthProvider),
        FhirVersion::R4,
        None,
        public_base.to_string(),
        10 * 1024 * 1024,
        tenant_path_routing,
        None,
        helios_ui::PatientNameSearchSupport::Enabled,
    )
    .layer(axum::middleware::from_fn(inject_test_principal));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (self_base, mock, backend)
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

async fn post_form_body(base: &str, path: &str, form: &[(&str, &str)]) -> (u16, String) {
    let response = client()
        .post(format!("{base}{path}"))
        .form(form)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, response.text().await.unwrap())
}

async fn post_patient_query(
    base: &str,
    path: &str,
    q: &str,
    authorization: Option<&str>,
) -> (u16, HeaderMap, String) {
    let mut request = client()
        .post(format!("{base}{path}"))
        .header("HX-Request", "true")
        .form(&[("q", q)]);
    if let Some(authorization) = authorization {
        request = request.header("Authorization", authorization);
    }
    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = response.text().await.unwrap();
    (status, headers, body)
}

fn patient(id: &str, given: &str, family: &str) -> serde_json::Value {
    serde_json::json!({
        "resourceType": "Patient",
        "id": id,
        "name": [{"given": [given], "family": family}]
    })
}

fn query_values(query: &str, name: &str) -> Vec<String> {
    form_urlencoded::parse(query.as_bytes())
        .filter(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .collect()
}

async fn post_form_as(base: &str, path: &str, user: &str) -> (u16, String) {
    let res = client()
        .post(format!("{base}{path}"))
        .header("X-Test-User", user)
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

async fn start_and_complete(base: &str) -> (String, String) {
    post_form(
        base,
        "/ui/bulk-export",
        &[("name", "Complete export"), ("scope", "system")],
    )
    .await;
    let (_, html) = get_text(base, "/ui/bulk-export").await;
    let card_path = html
        .split("hx-get=\"")
        .map(|s| s.split('"').next().unwrap_or(""))
        .find(|s| s.starts_with("/ui/bulk-export/active/"))
        .expect("card poll URL")
        .to_string();
    get_text(base, &card_path).await;
    let (_, complete_html) = get_text(base, &card_path).await;
    let download_path = complete_html
        .split("href=\"")
        .map(|s| s.split('"').next().unwrap_or(""))
        .find(|s| s.ends_with("/download"))
        .expect("download URL")
        .to_string();
    (card_path, download_path)
}

async fn seed_job(backend: &SqliteBackend, tenant: &str, id: &str, job: serde_json::Value) {
    seed_job_for_user(backend, "l2:", tenant, id, job).await;
}

async fn seed_job_for_user(
    backend: &SqliteBackend,
    user_key: &str,
    tenant: &str,
    id: &str,
    job: serde_json::Value,
) {
    backend
        .patch_settings(
            user_key,
            serde_json::json!({
                "byTenant": { tenant: { "bulkExport": { "jobs": { id: job } } } }
            }),
            None,
        )
        .await
        .expect("seed export job");
}

async fn assert_no_default_user_jobs(backend: &SqliteBackend) {
    let settings = backend.get_settings("l2:").await.unwrap();
    assert!(
        settings
            .as_ref()
            .and_then(|settings| settings
                .document
                .pointer("/byTenant/default/bulkExport/jobs"))
            .is_none(),
        "invalid input must not persist any export job"
    );
}

#[tokio::test]
async fn the_root_is_the_management_page_and_new_is_the_builder() {
    let (base, mock, _) = serve().await;
    let assert_export_nav_is_current = |html: &str| {
        assert!(html.contains(r#"<a class="nav-item" href="/ui/bulk-export" aria-current="page""#));
    };

    let (status, html) = get_text(&base, "/ui/bulk-export").await;
    assert_eq!(status, 200);
    assert_export_nav_is_current(&html);
    assert!(html.contains("Exports"));
    assert!(html.contains(r#"href="/ui/bulk-export/new""#));
    assert!(!html.contains(r#"<form method="post" action="/ui/bulk-export""#));
    assert!(!html.contains(r#"class="back-link""#));

    let (status, html) = get_text(&base, "/ui/bulk-export/new").await;
    assert_eq!(status, 200);
    assert_export_nav_is_current(&html);
    assert!(html.contains("What are you exporting?"));
    assert!(html.contains("Everything"));
    assert!(html.contains("All Resources"));
    assert!(html.contains(r#"name="types" value="Patient""#));
    assert!(html.contains(r#"class="typegrid__item" data-full-name="Patient" title="Patient""#));
    assert!(html.contains(r#"<span class="typegrid__label">Patient</span>"#));
    assert!(html.contains("Narrow it down"));
    assert!(html.contains("Start Export"));
    assert!(html.contains(
        r#"<h1 class="page-head__title" data-bulk-export-name-heading>Bulk Export</h1>"#
    ));
    assert!(html.contains(r#"<script src="/ui/assets/resource-filter.js" defer></script>"#));
    assert!(html.contains(r#"<script src="/ui/assets/bulk-export.js" defer></script>"#));
    assert!(html.contains(r#"<form method="post" action="/ui/bulk-export""#));
    assert!(!html.contains("toolbar__count"));

    let (status, _) = post_form(&base, "/ui/bulk-export/new", &[("scope", "system")]).await;
    assert_eq!(status, 405);
    assert!(mock.kickoffs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_export_page_uses_form_panels_with_name_and_all_resources_up_top() {
    let (base, _, _) = serve().await;
    let (status, html) = get_text(&base, "/ui/bulk-export/new").await;
    assert_eq!(status, 200);

    // The scope choices are the designed radio-card row (#735), and nothing
    // borrows the sticky .detail sidebar layout (#608).
    assert_eq!(html.matches(r#"class="choice-card""#).count(), 3);
    assert!(!html.contains("card detail"));

    // The Name field comes before the scope radios in the first panel.
    let name_pos = html.find(r#"name="name""#).expect("name field present");
    let scope_pos = html.find(r#"name="scope""#).expect("scope radios present");
    assert!(name_pos < scope_pos, "Name should precede the scope radios");
    assert!(
        html.contains(r#"name="name" required autocomplete="off""#),
        "Name stays semantically required"
    );
    assert!(
        html.contains(r#"class="bulk-export-form" novalidate"#),
        "the app and server own validation feedback instead of the browser"
    );
    assert!(!html.contains("data-validation-started"));
    assert!(html.contains(
        r#"id="bulk-export-name-error" class="field__hint field__hint--error" role="alert" hidden>Enter a name for this export.</span>"#
    ));

    // All Resources is the explicit default above the checkbox grid. The old
    // implicit "leave everything unchecked" hint is retired.
    let all_types_pos = html
        .find(r#"<input type="checkbox" name="all_types" checked>"#)
        .expect("checked All Resources input present");
    let grid_pos = html
        .find(r#"class="typegrid""#)
        .expect("types grid present");
    assert!(
        all_types_pos < grid_pos,
        "All Resources should precede the types grid"
    );
    assert!(!html.contains("Leave everything unchecked to export every type."));

    // The server-rendered fallback leaves individual types unchecked and
    // enabled. bulk-export.js enhances this state when JavaScript is present.
    let patient_pos = html
        .find(r#"<input type="checkbox" name="types" value="Patient">"#)
        .expect("Patient type input present");
    let patient_end = patient_pos
        + html[patient_pos..]
            .find("</label>")
            .expect("Patient type label closes");
    let patient = &html[patient_pos..patient_end];
    assert!(!patient.contains("checked"));
    assert!(!patient.contains("disabled"));

    // Custom instant remains enabled in server-rendered HTML so the no-JS
    // form can still submit it; bulk-export.js disables it for other presets.
    let since_custom_pos = html
        .find(r#"name="since_custom""#)
        .expect("custom instant input present");
    let since_custom_start = html[..since_custom_pos]
        .rfind("<input")
        .expect("custom instant input starts");
    let since_custom_end = since_custom_pos
        + html[since_custom_pos..]
            .find('>')
            .expect("custom instant input ends");
    let since_custom = &html[since_custom_start..=since_custom_end];
    assert!(!since_custom.contains("disabled"));
    assert!(since_custom.contains(r#"data-pattern="([0-9]([0-9]"#));
    assert!(since_custom.contains(r#"([0-5][0-9]|60)"#));
    assert!(since_custom.contains(r#"|14:00))"#));
    assert!(
        !since_custom.contains(" pattern="),
        "the native constraint UI must not intercept app-owned validation"
    );
    assert!(html.contains(
        r#"id="bulk-export-since-custom-error" class="field__hint field__hint--error" role="alert" hidden>Enter a valid FHIR instant"#
    ));
    assert!(!html.contains("Used when Since is Custom"));
    assert!(!html.contains("bulk-export-field-since-custom-hint"));
}

#[tokio::test]
async fn the_export_builder_uses_the_localized_shared_back_link() {
    let (base, _, _) = serve().await;
    let (status, html) = get_text(&base, "/ui/bulk-export/new").await;
    assert_eq!(status, 200);

    let assert_back_link =
        |localized_html: &str, label: &str| {
            let marker = r#"<a class="back-link" href="/ui/bulk-export">"#;
            let start = localized_html.find(marker).expect("shared back link");
            let end = start
                + localized_html[start..]
                    .find("</a>")
                    .expect("back link closing tag")
                + "</a>".len();
            let back_link = &localized_html[start..end];

            assert!(back_link.contains(
                r#"<span aria-hidden="true"><svg width="5" height="8" viewBox="0 0 5 8""#
            ));
            assert!(back_link.contains(&format!("<span>{label}</span>")));
            assert_eq!(back_link.matches("<span").count(), 2);
            assert!(
                !back_link.contains('‹'),
                "spacing must come from CSS, not the former literal chevron and space"
            );
        };
    assert_back_link(&html, "Exports");
    let header_start = html
        .find(r#"<header class="page-head page-head--back-link">"#)
        .expect("shared back-link header");
    let header_end = header_start
        + html[header_start..]
            .find("</header>")
            .expect("page header closing tag");
    let header = &html[header_start..header_end];
    let back_link_position = header.find(r#"class="back-link""#).unwrap();
    let copy_position = header.find(r#"class="page-head__copy""#).unwrap();
    assert!(back_link_position < copy_position);
    assert!(!header.contains(r#"class="page-head__action""#));
    for (lang, label, name_error, since_error) in [
        (
            "es",
            "Exportaciones",
            "Ingresa un nombre para esta exportación.",
            "Ingresa un instante FHIR válido",
        ),
        (
            "de",
            "Exporte",
            "Geben Sie einen Namen für diesen Export ein.",
            "Geben Sie einen gültigen FHIR-Zeitpunkt ein",
        ),
    ] {
        let (status, localized_html) =
            get_text(&base, &format!("/ui/bulk-export/new?lang={lang}")).await;
        assert_eq!(status, 200);
        assert_back_link(&localized_html, label);
        assert!(localized_html.contains(name_error), "{localized_html}");
        assert!(localized_html.contains(since_error), "{localized_html}");
    }
}

#[tokio::test]
async fn the_legacy_active_route_permanently_redirects_to_the_fixed_root() {
    let (base, _, _) = serve().await;
    let res = client()
        .get(format!("{base}/ui/bulk-export/active?lang=es"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(res.headers()["location"], "/ui/bulk-export");
}

#[tokio::test]
async fn the_management_page_reports_unavailable_settings_without_a_new_action() {
    let (base, _, _) = serve_with_settings(false).await;
    let (status, html) = get_text(&base, "/ui/bulk-export").await;
    assert_eq!(status, 200);
    assert!(html.contains("export jobs cannot be tracked"));
    assert!(!html.contains(r#"href="/ui/bulk-export/new""#));
    assert!(!html.contains("No exports yet"));
    assert!(!html.contains("0 exports · 0 running"));
}

#[tokio::test]
async fn starting_a_system_export_kicks_off_and_tracks_the_job() {
    let (base, mock, _) = serve().await;

    let (status, location) = post_form(
        &base,
        "/ui/bulk-export",
        &[
            ("name", "  Everything <img src=x>  "),
            ("scope", "system"),
            ("types", "Patient"),
            ("types", "Observation"),
            ("elements", "id,meta"),
            ("since_preset", "week"),
        ],
    )
    .await;
    assert_eq!(status, 303);
    assert_eq!(location, "/ui/bulk-export");

    // The mock saw one kick-off with the narrowed parameters.
    let kickoffs = mock.kickoffs.lock().unwrap().clone();
    assert_eq!(kickoffs.len(), 1);
    assert_eq!(kickoffs[0].0, "/$export");
    let q = &kickoffs[0].1;
    assert_eq!(query_values(q, "_type"), vec!["Patient,Observation"], "{q}");
    assert_eq!(query_values(q, "_elements"), vec!["id,meta"], "{q}");
    assert_eq!(query_values(q, "_since").len(), 1, "{q}");

    // The Exports page shows it in progress.
    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    assert!(
        html.contains(r#"<h2 class="job-card__name">Everything &#60;img src=x&#62;</h2>"#),
        "{html}"
    );
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
    assert!(
        html.contains(r#"<h2 class="job-card__name">Everything &#60;img src=x&#62;</h2>"#),
        "{html}"
    );
    assert!(html.contains("18% complete"), "{html}");
    assert!(html.contains("every 5s"));

    // Second: the mock flips to 200 -> complete with two files, no polling.
    let (_, html) = get_text(&base, &card_path).await;
    assert!(
        html.contains(r#"<h2 class="job-card__name">Everything &#60;img src=x&#62;</h2>"#),
        "{html}"
    );
    assert!(html.contains("Complete"), "{html}");
    assert!(html.contains("Patient"));
    assert!(html.contains("Observation"));
    assert!(html.contains("Download All Resources"));
    assert!(!html.contains("every 5s"));
}

#[tokio::test]
async fn invalid_start_fields_return_one_400_without_a_job_or_kickoff() {
    let (base, mock, backend) = serve().await;
    let (status, html) = post_form_body(
        &base,
        "/ui/bulk-export",
        &[
            ("name", "   "),
            ("scope", "patient"),
            ("patient", "Patient/not/valid"),
            ("since_preset", "custom"),
            ("since_custom", "not-a-date"),
        ],
    )
    .await;

    assert_eq!(status, 400);
    assert!(html.contains("Enter a name for this export."), "{html}");
    assert!(html.contains("Enter a valid FHIR instant"), "{html}");
    assert!(html.contains("valid logical Patient IDs"), "{html}");
    assert!(html.contains(r#"id="bulk-export-name-error""#), "{html}");
    assert!(
        html.contains(r#"id="bulk-export-since-custom-error""#),
        "{html}"
    );
    assert_eq!(html.matches(r#"aria-invalid="true""#).count(), 2);
    assert!(
        html.contains(r#"novalidate data-validation-started="true""#),
        "{html}"
    );
    assert_eq!(html.matches("autofocus").count(), 1, "{html}");
    assert!(
        html.contains(r#"autofocus aria-invalid="true" aria-describedby="bulk-export-name-error""#),
        "{html}"
    );
    assert!(mock.kickoffs.lock().unwrap().is_empty());
    assert_no_default_user_jobs(&backend).await;
}

#[tokio::test]
async fn an_invalid_active_custom_instant_repopulates_the_representable_form() {
    let (base, mock, backend) = serve().await;
    let (status, html) = post_form_body(
        &base,
        "/ui/bulk-export",
        &[
            ("name", "  Preserved & named  "),
            ("scope", "patient"),
            ("group_id", "unused-group"),
            ("types", "Patient"),
            ("types", "Patient"),
            ("types", "NotAResource"),
            ("elements", "id,<meta>"),
            ("type_filter", "Patient?name=A&B"),
            ("patient", "Patient/p-104"),
            ("patient", "Patient/p-205"),
            ("since_preset", "custom"),
            ("since_custom", "2026-02-31T00:00:00Z"),
        ],
    )
    .await;

    assert_eq!(status, 400);
    assert!(html.contains("Enter a valid FHIR instant"), "{html}");
    assert!(
        html.contains(r#"novalidate data-validation-started="true""#),
        "{html}"
    );
    assert_eq!(html.matches("autofocus").count(), 1, "{html}");
    assert!(
        html.contains(
            r#"autofocus aria-invalid="true" aria-describedby="bulk-export-since-custom-error""#
        ),
        "{html}"
    );
    assert!(
        html.contains(r#"value="  Preserved &#38; named  ""#),
        "{html}"
    );
    assert!(
        html.contains(r#"name="scope" value="patient" checked"#),
        "{html}"
    );
    assert!(html.contains(r#"value="unused-group""#), "{html}");
    assert!(
        html.contains(r#"name="types" value="Patient" checked"#),
        "{html}"
    );
    assert!(!html.contains(r#"value="NotAResource""#), "{html}");
    assert!(html.contains(r#"value="id,&#60;meta&#62;""#), "{html}");
    assert!(html.contains(r#"value="Patient?name=A&#38;B""#), "{html}");
    assert!(
        html.contains(r#"<option value="custom" selected>"#),
        "{html}"
    );
    assert!(html.contains(r#"value="2026-02-31T00:00:00Z""#), "{html}");
    assert!(
        html.contains("Patient/p-104\nPatient/p-205</textarea>"),
        "{html}"
    );
    assert!(
        html.contains(r#"<a class="btn" href="/ui/bulk-export/new">Clear</a>"#),
        "{html}"
    );
    assert!(mock.kickoffs.lock().unwrap().is_empty());
    assert_no_default_user_jobs(&backend).await;
}

#[tokio::test]
async fn blank_custom_is_omitted_and_inactive_stale_custom_is_ignored() {
    let (base, mock, _) = serve().await;

    let (status, _) = post_form(
        &base,
        "/ui/bulk-export",
        &[
            ("name", "Blank custom"),
            ("scope", "system"),
            ("since_preset", "custom"),
            ("since_custom", "   "),
        ],
    )
    .await;
    assert_eq!(status, 303);

    let (status, _) = post_form(
        &base,
        "/ui/bulk-export",
        &[
            ("name", "Inactive custom"),
            ("scope", "system"),
            ("since_preset", "week"),
            ("since_custom", "not-a-date"),
        ],
    )
    .await;
    assert_eq!(status, 303);

    let kickoffs = mock.kickoffs.lock().unwrap().clone();
    assert_eq!(kickoffs.len(), 2);
    assert!(query_values(&kickoffs[0].1, "_since").is_empty());
    let inactive_since = query_values(&kickoffs[1].1, "_since");
    assert_eq!(inactive_since.len(), 1);
    assert_ne!(inactive_since[0], "not-a-date");
}

#[tokio::test]
async fn valid_fractional_and_offset_custom_instants_keep_their_wire_value() {
    let (base, mock, _) = serve().await;
    let instants = [
        "2026-08-01T00:00:00.123Z",
        "2026-08-01T03:30:00+03:30",
        "2026-12-31T23:59:60Z",
        "2026-08-01T00:00:00+14:00",
    ];

    for (index, instant) in instants.iter().enumerate() {
        let name = format!("Valid custom {index}");
        let (status, _) = post_form(
            &base,
            "/ui/bulk-export",
            &[
                ("name", &name),
                ("scope", "system"),
                ("since_preset", "custom"),
                ("since_custom", instant),
            ],
        )
        .await;
        assert_eq!(status, 303);
    }

    let kickoffs = mock.kickoffs.lock().unwrap().clone();
    assert_eq!(kickoffs.len(), instants.len());
    for (kickoff, expected) in kickoffs.iter().zip(instants) {
        assert_eq!(query_values(&kickoff.1, "_since"), vec![expected]);
    }
}

#[tokio::test]
async fn fhir_r4_lexically_invalid_instants_fail_without_a_job_or_kickoff() {
    let (base, mock, backend) = serve().await;

    for (index, instant) in [
        "0000-08-01T00:00:00Z",
        "2026-08-01T00:00:00+14:01",
        "2026-08-01T00:00:00+15:00",
    ]
    .into_iter()
    .enumerate()
    {
        let name = format!("Invalid FHIR instant {index}");
        let (status, html) = post_form_body(
            &base,
            "/ui/bulk-export",
            &[
                ("name", &name),
                ("scope", "system"),
                ("since_preset", "custom"),
                ("since_custom", instant),
            ],
        )
        .await;
        assert_eq!(status, 400, "{instant}: {html}");
        assert!(html.contains("Enter a valid FHIR instant"), "{html}");
    }

    assert!(mock.kickoffs.lock().unwrap().is_empty());
    assert_no_default_user_jobs(&backend).await;
}

#[tokio::test]
async fn all_resources_omits_type_even_when_a_hostile_form_sends_types() {
    let (base, mock, _) = serve().await;

    let (status, _) = post_form(
        &base,
        "/ui/bulk-export",
        &[
            ("name", "Hostile all resources"),
            ("scope", "system"),
            ("types", "Patient"),
            ("all_types", "not-a-boolean"),
            ("types", "Observation"),
            ("elements", "id,meta"),
        ],
    )
    .await;
    assert_eq!(status, 303);

    let kickoffs = mock.kickoffs.lock().unwrap().clone();
    assert_eq!(kickoffs.len(), 1);
    let q = &kickoffs[0].1;
    assert!(query_values(q, "_type").is_empty(), "{q}");
    assert_eq!(query_values(q, "_elements"), vec!["id,meta"], "{q}");
}

#[tokio::test]
async fn self_calls_ignore_public_host_and_prefix_but_validate_advertised_paths() {
    for (tenant_path_routing, kickoff_path, status_path, output_path) in [
        (
            false,
            "/$export",
            format!("/export-status/{REST_JOB_ID}"),
            format!("/export-file/{REST_JOB_ID}/Patient-0"),
        ),
        (
            true,
            "/default/$export",
            format!("/default/export-status/{REST_JOB_ID}"),
            format!("/default/export-file/{REST_JOB_ID}/Patient-0"),
        ),
    ] {
        let public_base = "https://public.example/fhir";
        let (base, mock, _) =
            serve_with_separate_public_base(public_base, tenant_path_routing).await;
        post_form(
            &base,
            "/ui/bulk-export",
            &[("name", "Self call"), ("scope", "system")],
        )
        .await;
        let kickoff = mock.kickoffs.lock().unwrap().clone();
        assert_eq!(kickoff.len(), 1);
        assert_eq!(kickoff[0].0, kickoff_path);
        assert_eq!(kickoff[0].2.as_deref(), Some("default"));

        let (_, html) = get_text(&base, "/ui/bulk-export").await;
        let card_path = html
            .split("hx-get=\"")
            .map(|value| value.split('"').next().unwrap_or(""))
            .find(|value| value.starts_with("/ui/bulk-export/active/"))
            .expect("polling card path");
        get_text(&base, card_path).await;
        let polls = mock.poll_requests.lock().unwrap().clone();
        assert_eq!(polls, vec![(status_path, Some("default".to_string()))]);

        let (_, complete_html) = get_text(&base, card_path).await;
        let download_path = complete_html
            .split("href=\"")
            .map(|value| value.split('"').next().unwrap_or(""))
            .find(|value| value.ends_with("/download"))
            .expect("download path");
        let response = client()
            .get(format!("{base}{download_path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response.bytes().await.unwrap();
        assert!(mock.downloads.lock().unwrap().iter().any(|download| {
            download.path == output_path && download.tenant.as_deref() == Some("default")
        }));
    }
}

#[tokio::test]
async fn legacy_urls_require_the_public_prefix_and_current_tenant() {
    let (base, mock, backend) =
        serve_with_separate_public_base("https://public.example/fhir", true).await;
    for (id, poll_url) in [
        (
            "valid",
            format!("https://old-host.invalid/fhir/default/export-status/{REST_JOB_ID}"),
        ),
        (
            "wrong-tenant",
            format!("https://old-host.invalid/fhir/other/export-status/{REST_JOB_ID}"),
        ),
        (
            "wrong-prefix",
            format!("https://old-host.invalid/default/export-status/{REST_JOB_ID}"),
        ),
    ] {
        seed_job(
            &backend,
            "default",
            id,
            serde_json::json!({
                "name": id,
                "scope": "system",
                "status": "failed",
                "pollUrl": poll_url,
                "startedAt": "2026-08-11T09:00:00Z"
            }),
        )
        .await;
    }

    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    assert_eq!(html.matches("/delete\"").count(), 1, "{html}");
    for id in ["wrong-tenant", "wrong-prefix"] {
        let (_, location) =
            post_form(&base, &format!("/ui/bulk-export/active/{id}/delete"), &[]).await;
        assert_eq!(location, "/ui/bulk-export?delete-error=remote");
    }
    assert_eq!(*mock.cancels.lock().unwrap(), 0);
}

#[tokio::test]
async fn legacy_non_202_failures_are_the_only_empty_url_jobs_deletable_locally() {
    let (base, mock, backend) = serve().await;
    for (id, error) in [
        ("rejected", "kick-off answered 400: invalid request"),
        ("transport", "error sending request for url"),
        (
            "missing-location",
            "kick-off accepted without a Content-Location",
        ),
    ] {
        seed_job(
            &backend,
            "default",
            id,
            serde_json::json!({
                "name": id,
                "scope": "system",
                "status": "failed",
                "error": error,
                "startedAt": "2026-08-11T09:00:00Z"
            }),
        )
        .await;
    }
    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    assert_eq!(html.matches("/delete\"").count(), 1, "{html}");

    post_form(&base, "/ui/bulk-export/active/rejected/delete", &[]).await;
    assert_eq!(*mock.cancels.lock().unwrap(), 0);
    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    assert!(!html.contains(">rejected<"));
    assert!(html.contains(">transport<"));
    assert!(html.contains(">missing-location<"));
}

#[tokio::test]
async fn patient_and_group_scopes_hit_their_export_paths() {
    let (base, mock, _) = serve().await;

    post_form(
        &base,
        "/ui/bulk-export",
        &[("name", "Patient export"), ("scope", "patient")],
    )
    .await;
    post_form(
        &base,
        "/ui/bulk-export",
        &[
            ("name", "Group export"),
            ("scope", "group"),
            ("group_id", "cohort-7"),
        ],
    )
    .await;

    let kickoffs = mock.kickoffs.lock().unwrap().clone();
    let paths: Vec<&str> = kickoffs.iter().map(|(p, _, _)| p.as_str()).collect();
    assert!(paths.contains(&"/Patient/$export"), "{paths:?}");
    assert!(paths.contains(&"/Group/cohort-7/$export"), "{paths:?}");
    for (_, query, _) in &kickoffs {
        assert!(
            query_values(query, "_type").is_empty(),
            "older clients that omit all_types and types still export all resources: {query}"
        );
    }
}

#[tokio::test]
async fn a_rejected_kickoff_lands_as_failed_and_retry_reruns_it() {
    let (base, mock, _) = serve().await;
    *mock.reject.lock().unwrap() =
        Some("The server ran out of time building Observation.ndjson".to_string());

    post_form(
        &base,
        "/ui/bulk-export",
        &[("name", "Diabetes registry 2024"), ("scope", "system")],
    )
    .await;

    let (_, html) = get_text(&base, "/ui/bulk-export").await;
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
    let (status, location) = post_form(&base, &retry_path, &[]).await;
    assert_eq!(status, 303);
    assert_eq!(location, "/ui/bulk-export");

    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    assert!(html.contains("In progress"), "{html}");
    let kickoffs = mock.kickoffs.lock().unwrap().clone();
    assert_eq!(kickoffs.len(), 2);
    for (_, query, _) in &kickoffs {
        assert!(
            query_values(query, "_type").is_empty(),
            "retry should preserve the empty all-resources type list: {query}"
        );
    }
}

#[tokio::test]
async fn cancelling_deletes_the_job_server_side() {
    let (base, mock, _) = serve().await;
    post_form(
        &base,
        "/ui/bulk-export",
        &[("name", "Cancelable"), ("scope", "system")],
    )
    .await;

    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    let cancel_path = html
        .split("action=\"")
        .find(|s| s.starts_with("/ui/bulk-export/active/") && s.contains("/cancel"))
        .and_then(|s| s.split('"').next())
        .expect("cancel action")
        .to_string();
    let (status, location) = post_form(&base, &cancel_path, &[]).await;
    assert_eq!(status, 303);
    assert_eq!(location, "/ui/bulk-export");

    assert_eq!(*mock.cancels.lock().unwrap(), 1, "DELETE reached the API");
    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    assert!(html.contains("Cancelled"));
}

#[tokio::test]
async fn terminal_cards_have_an_accessible_no_js_delete_disclosure() {
    let (base, _, _) = serve().await;
    let (_, download_path) = start_and_complete(&base).await;
    let card_path = download_path.trim_end_matches("/download");
    let (_, html) = get_text(&base, &format!("{card_path}/card")).await;

    assert!(html.contains(r#"<details class="job-card__delete">"#));
    assert!(html.contains("Delete Complete export"));
    assert!(html.contains(&format!(r#"action="{card_path}/delete""#)));
    assert!(html.contains(r#"href="/ui/bulk-export">Keep export"#));
    assert!(html.contains("and its output files from the server? This cannot be undone."));
}

#[tokio::test]
async fn deleting_a_terminal_remote_job_removes_only_that_settings_member() {
    let (base, mock, backend) = serve().await;
    seed_job(
        &backend,
        "default",
        "keep-me",
        serde_json::json!({
            "name": "Sibling failed export",
            "scope": "system",
            "status": "failed",
            "remoteJob": "no-remote-job",
            "startedAt": "2026-08-11T09:00:00Z"
        }),
    )
    .await;
    let (_, download_path) = start_and_complete(&base).await;
    let delete_path = format!("{}/delete", download_path.trim_end_matches("/download"));

    let (status, location) = post_form(&base, &delete_path, &[]).await;
    assert_eq!(status, 303);
    assert_eq!(location, "/ui/bulk-export");
    assert_eq!(*mock.cancels.lock().unwrap(), 1);
    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    assert!(!html.contains("Complete export"));
    assert!(html.contains("Sibling failed export"));
}

#[tokio::test]
async fn an_already_missing_remote_job_is_still_removed_locally() {
    let (base, mock, _) = serve().await;
    let (_, download_path) = start_and_complete(&base).await;
    *mock.delete_status.lock().unwrap() = StatusCode::NOT_FOUND;
    let delete_path = format!("{}/delete", download_path.trim_end_matches("/download"));

    post_form(&base, &delete_path, &[]).await;
    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    assert!(!html.contains("Complete export"));
}

#[tokio::test]
async fn a_failed_kickoff_deletes_locally_without_a_remote_call() {
    let (base, mock, _) = serve().await;
    *mock.reject.lock().unwrap() = Some("invalid export request".to_string());
    post_form(
        &base,
        "/ui/bulk-export",
        &[("name", "Rejected export"), ("scope", "system")],
    )
    .await;
    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    let delete_path = html
        .split("action=\"")
        .map(|s| s.split('"').next().unwrap_or(""))
        .find(|s| s.ends_with("/delete"))
        .expect("delete action")
        .to_string();

    post_form(&base, &delete_path, &[]).await;
    assert_eq!(*mock.cancels.lock().unwrap(), 0);
    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    assert!(!html.contains("Rejected export"));
}

#[tokio::test]
async fn remote_delete_failure_retains_the_card_and_shows_a_localized_notice() {
    let (base, mock, _) = serve().await;
    let (_, download_path) = start_and_complete(&base).await;
    *mock.delete_status.lock().unwrap() = StatusCode::INTERNAL_SERVER_ERROR;
    let delete_path = format!("{}/delete", download_path.trim_end_matches("/download"));

    let (status, location) = post_form(&base, &delete_path, &[]).await;
    assert_eq!(status, 303);
    assert_eq!(location, "/ui/bulk-export?delete-error=remote");
    let (_, html) = get_text(&base, "/ui/bulk-export?delete-error=remote&lang=es").await;
    assert!(
        html.contains("No se pudo eliminar la exportación"),
        "{html}"
    );
    assert!(html.contains("Complete export"));
}

#[tokio::test]
async fn forged_in_progress_and_untrusted_legacy_deletes_make_no_outbound_call() {
    let (base, mock, backend) = serve().await;
    seed_job(
        &backend,
        "default",
        "running",
        serde_json::json!({
            "name": "Still running",
            "scope": "system",
            "status": "in-progress",
            "remoteJob": "known",
            "remoteJobId": REST_JOB_ID,
            "startedAt": "2026-08-11T09:00:00Z"
        }),
    )
    .await;
    seed_job(
        &backend,
        "other-tenant",
        "other-tenant-job",
        serde_json::json!({
            "name": "Other tenant export",
            "scope": "system",
            "status": "complete",
            "remoteJob": "known",
            "remoteJobId": REST_JOB_ID,
            "files": [{"type": "Patient", "url": "ignored"}],
            "startedAt": "2026-08-11T09:00:00Z"
        }),
    )
    .await;
    seed_job(
        &backend,
        "default",
        "malformed",
        serde_json::json!({
            "name": "Malformed legacy export",
            "scope": "system",
            "status": "failed",
            "pollUrl": format!("http://attacker.invalid/export-status/{REST_JOB_ID}?leak=1"),
            "startedAt": "2026-08-11T09:00:00Z"
        }),
    )
    .await;

    let (_, running_location) =
        post_form(&base, "/ui/bulk-export/active/running/delete", &[]).await;
    assert_eq!(running_location, "/ui/bulk-export");
    let (_, malformed_location) =
        post_form(&base, "/ui/bulk-export/active/malformed/delete", &[]).await;
    assert_eq!(malformed_location, "/ui/bulk-export?delete-error=remote");
    let (_, other_tenant_location) =
        post_form(&base, "/ui/bulk-export/active/other-tenant-job/delete", &[]).await;
    assert_eq!(other_tenant_location, "/ui/bulk-export");
    assert_eq!(*mock.cancels.lock().unwrap(), 0);
    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    assert!(html.contains("Still running"));
    assert!(html.contains("Malformed legacy export"));
}

#[tokio::test]
async fn another_authenticated_user_cannot_delete_the_owners_job() {
    let (base, mock, backend) = serve().await;
    seed_job_for_user(
        &backend,
        "u2:4:test:owner",
        "default",
        "owners-job",
        serde_json::json!({
            "name": "Owner export",
            "scope": "system",
            "status": "complete",
            "remoteJob": "known",
            "remoteJobId": REST_JOB_ID,
            "files": [{"type": "Patient", "url": "ignored"}],
            "startedAt": "2026-08-11T09:00:00Z"
        }),
    )
    .await;

    let (_, location) = post_form_as(
        &base,
        "/ui/bulk-export/active/owners-job/delete",
        "attacker",
    )
    .await;
    assert_eq!(location, "/ui/bulk-export");
    assert_eq!(*mock.cancels.lock().unwrap(), 0);
    let owner_settings = backend
        .get_settings("u2:4:test:owner")
        .await
        .unwrap()
        .expect("owner settings remain");
    assert!(
        owner_settings.document["byTenant"]["default"]["bulkExport"]["jobs"]
            .get("owners-job")
            .is_some()
    );
}

#[tokio::test]
async fn a_stale_poll_cannot_recreate_a_concurrently_deleted_job() {
    let (base, mock, backend) = serve().await;
    post_form(
        &base,
        "/ui/bulk-export",
        &[("name", "Concurrent poll"), ("scope", "system")],
    )
    .await;
    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    let card_path = html
        .split("hx-get=\"")
        .map(|value| value.split('"').next().unwrap_or(""))
        .find(|value| value.starts_with("/ui/bulk-export/active/"))
        .unwrap()
        .to_string();
    let ui_id = card_path
        .trim_end_matches("/card")
        .rsplit('/')
        .next()
        .unwrap()
        .to_string();
    let gate = Arc::new(RequestGate::default());
    *mock.status_gate.lock().unwrap() = Some(gate.clone());
    let poll_base = base.clone();
    let poll_path = card_path.clone();
    let polling = tokio::spawn(async move { get_text(&poll_base, &poll_path).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), gate.reached.notified())
        .await
        .expect("poll reached remote status");

    let current = backend
        .get_settings("l2:")
        .await
        .unwrap()
        .expect("job settings");
    backend
        .patch_settings(
            "l2:",
            serde_json::json!({
                "byTenant": { "default": { "bulkExport": { "jobs": {
                    ui_id.clone(): { "status": "complete", "files": [{"type": "Patient", "url": "ignored"}] }
                } } } }
            }),
            Some(current.version),
        )
        .await
        .unwrap();
    post_form(
        &base,
        &format!("/ui/bulk-export/active/{ui_id}/delete"),
        &[],
    )
    .await;
    gate.release.notify_one();
    polling.await.unwrap();

    let current = backend.get_settings("l2:").await.unwrap().unwrap();
    assert!(
        current.document["byTenant"]["default"]["bulkExport"]["jobs"]
            .get(&ui_id)
            .is_none(),
        "stale poll must not recreate the removed member"
    );
}

#[tokio::test]
async fn delete_cannot_remove_a_job_that_was_concurrently_retried() {
    let (base, mock, backend) = serve().await;
    let (_, download_path) = start_and_complete(&base).await;
    let job_path = download_path.trim_end_matches("/download").to_string();
    let ui_id = job_path.rsplit('/').next().unwrap().to_string();
    let gate = Arc::new(RequestGate::default());
    *mock.delete_gate.lock().unwrap() = Some(gate.clone());
    let delete_base = base.clone();
    let delete_path = format!("{job_path}/delete");
    let deleting = tokio::spawn(async move { post_form(&delete_base, &delete_path, &[]).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), gate.reached.notified())
        .await
        .expect("delete reached remote status");

    let (_, retry_location) = post_form(&base, &format!("{job_path}/retry"), &[]).await;
    assert_eq!(retry_location, "/ui/bulk-export");
    gate.release.notify_one();
    let (_, delete_location) = deleting.await.unwrap();
    assert_eq!(delete_location, "/ui/bulk-export?delete-error=local");

    let current = backend.get_settings("l2:").await.unwrap().unwrap();
    assert_eq!(
        current.document["byTenant"]["default"]["bulkExport"]["jobs"][&ui_id]["status"],
        "in-progress"
    );
}

#[tokio::test]
async fn a_retry_that_loses_its_cas_retries_transient_cleanup_until_404() {
    let (base, mock, backend) = serve().await;
    let (_, download_path) = start_and_complete(&base).await;
    let job_path = download_path.trim_end_matches("/download").to_string();
    let ui_id = job_path.rsplit('/').next().unwrap().to_string();
    let gate = Arc::new(RequestGate::default());
    *mock.kickoff_gate.lock().unwrap() = Some(gate.clone());
    let retry_base = base.clone();
    let retry_path = format!("{job_path}/retry");
    let retrying = tokio::spawn(async move { post_form(&retry_base, &retry_path, &[]).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), gate.reached.notified())
        .await
        .expect("retry reached remote kick-off");

    let current = backend.get_settings("l2:").await.unwrap().unwrap();
    backend
        .patch_settings(
            "l2:",
            serde_json::json!({
                "byTenant": { "default": { "bulkExport": { "jobs": {
                    ui_id.clone(): { "status": "cancelled" }
                } } } }
            }),
            Some(current.version),
        )
        .await
        .unwrap();
    mock.delete_statuses
        .lock()
        .unwrap()
        .extend([StatusCode::INTERNAL_SERVER_ERROR, StatusCode::NOT_FOUND]);
    gate.release.notify_one();
    retrying.await.unwrap();

    assert_eq!(
        *mock.cancels.lock().unwrap(),
        2,
        "the newly kicked-off remote job must be deleted after stale CAS"
    );
    let current = backend.get_settings("l2:").await.unwrap().unwrap();
    assert_eq!(
        current.document["byTenant"]["default"]["bulkExport"]["jobs"][&ui_id]["status"],
        "cancelled"
    );
}

#[tokio::test]
async fn failed_remote_cleanup_creates_a_recoverable_terminal_card() {
    let (base, mock, backend) = serve().await;
    let (_, download_path) = start_and_complete(&base).await;
    let job_path = download_path.trim_end_matches("/download").to_string();
    let ui_id = job_path.rsplit('/').next().unwrap().to_string();
    let gate = Arc::new(RequestGate::default());
    *mock.kickoff_gate.lock().unwrap() = Some(gate.clone());
    let retry_base = base.clone();
    let retry_path = format!("{job_path}/retry");
    let retrying = tokio::spawn(async move { post_form(&retry_base, &retry_path, &[]).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), gate.reached.notified())
        .await
        .expect("retry reached remote kick-off");

    let current = backend.get_settings("l2:").await.unwrap().unwrap();
    backend
        .patch_settings(
            "l2:",
            serde_json::json!({
                "byTenant": { "default": { "bulkExport": { "jobs": {
                    ui_id.clone(): { "status": "cancelled" }
                } } } }
            }),
            Some(current.version),
        )
        .await
        .unwrap();
    *mock.delete_status.lock().unwrap() = StatusCode::INTERNAL_SERVER_ERROR;
    gate.release.notify_one();
    retrying.await.unwrap();
    assert_eq!(*mock.cancels.lock().unwrap(), 3, "cleanup retry bound");

    let current = backend.get_settings("l2:").await.unwrap().unwrap();
    let jobs = current.document["byTenant"]["default"]["bulkExport"]["jobs"]
        .as_object()
        .unwrap();
    let (recovery_id, recovery) = jobs
        .iter()
        .find(|(id, _)| id.as_str() != ui_id)
        .expect("recovery card");
    let recovery_id = recovery_id.clone();
    assert_eq!(recovery["status"], "failed");
    assert!(
        recovery["error"]
            .as_str()
            .unwrap()
            .contains("Delete this card to retry remote cleanup")
    );

    *mock.delete_status.lock().unwrap() = StatusCode::ACCEPTED;
    post_form(
        &base,
        &format!("/ui/bulk-export/active/{recovery_id}/delete"),
        &[],
    )
    .await;
    let current = backend.get_settings("l2:").await.unwrap().unwrap();
    let jobs = current.document["byTenant"]["default"]["bulkExport"]["jobs"]
        .as_object()
        .unwrap();
    assert!(jobs.contains_key(&ui_id));
    assert!(!jobs.contains_key(&recovery_id));
}

#[tokio::test]
async fn a_poll_version_bump_does_not_discard_a_concurrent_second_start() {
    let (base, mock, backend) = serve().await;
    post_form(
        &base,
        "/ui/bulk-export",
        &[("name", "First export"), ("scope", "system")],
    )
    .await;
    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    let first_card_path = html
        .split("hx-get=\"")
        .map(|value| value.split('"').next().unwrap_or(""))
        .find(|value| value.starts_with("/ui/bulk-export/active/"))
        .unwrap()
        .to_string();

    let gate = Arc::new(RequestGate::default());
    *mock.kickoff_gate.lock().unwrap() = Some(gate.clone());
    let second_base = base.clone();
    let second_start = tokio::spawn(async move {
        post_form(
            &second_base,
            "/ui/bulk-export",
            &[("name", "Second export"), ("scope", "system")],
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), gate.reached.notified())
        .await
        .expect("second start reached kick-off");

    get_text(&base, &first_card_path).await;
    gate.release.notify_one();
    second_start.await.unwrap();

    let current = backend.get_settings("l2:").await.unwrap().unwrap();
    let jobs = current.document["byTenant"]["default"]["bulkExport"]["jobs"]
        .as_object()
        .unwrap();
    assert_eq!(jobs.len(), 2);
    let names: Vec<_> = jobs
        .values()
        .filter_map(|job| job["name"].as_str())
        .collect();
    assert!(names.contains(&"First export"));
    assert!(names.contains(&"Second export"));
    assert_eq!(*mock.cancels.lock().unwrap(), 0);
}

#[tokio::test]
async fn a_second_start_version_bump_does_not_discard_a_concurrent_poll() {
    let (base, mock, backend) = serve().await;
    post_form(
        &base,
        "/ui/bulk-export",
        &[("name", "First export"), ("scope", "system")],
    )
    .await;
    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    let first_card_path = html
        .split("hx-get=\"")
        .map(|value| value.split('"').next().unwrap_or(""))
        .find(|value| value.starts_with("/ui/bulk-export/active/"))
        .unwrap()
        .to_string();
    let first_id = first_card_path
        .trim_end_matches("/card")
        .rsplit('/')
        .next()
        .unwrap()
        .to_string();

    let gate = Arc::new(RequestGate::default());
    *mock.status_gate.lock().unwrap() = Some(gate.clone());
    let poll_base = base.clone();
    let poll_path = first_card_path.clone();
    let polling = tokio::spawn(async move { get_text(&poll_base, &poll_path).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), gate.reached.notified())
        .await
        .expect("poll reached status endpoint");

    post_form(
        &base,
        "/ui/bulk-export",
        &[("name", "Second export"), ("scope", "system")],
    )
    .await;
    gate.release.notify_one();
    polling.await.unwrap();

    let current = backend.get_settings("l2:").await.unwrap().unwrap();
    let jobs = current.document["byTenant"]["default"]["bulkExport"]["jobs"]
        .as_object()
        .unwrap();
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[&first_id]["progress"], "18% complete");
}

#[tokio::test]
async fn retry_survives_an_unrelated_settings_version_bump() {
    let (base, mock, backend) = serve().await;
    let (_, download_path) = start_and_complete(&base).await;
    let job_path = download_path.trim_end_matches("/download").to_string();
    let ui_id = job_path.rsplit('/').next().unwrap().to_string();
    let gate = Arc::new(RequestGate::default());
    *mock.kickoff_gate.lock().unwrap() = Some(gate.clone());
    let retry_base = base.clone();
    let retry_path = format!("{job_path}/retry");
    let retrying = tokio::spawn(async move { post_form(&retry_base, &retry_path, &[]).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), gate.reached.notified())
        .await
        .expect("retry reached kick-off");

    let current = backend.get_settings("l2:").await.unwrap().unwrap();
    backend
        .patch_settings(
            "l2:",
            serde_json::json!({ "raceMarker": true }),
            Some(current.version),
        )
        .await
        .unwrap();
    gate.release.notify_one();
    retrying.await.unwrap();

    let current = backend.get_settings("l2:").await.unwrap().unwrap();
    assert_eq!(
        current.document["byTenant"]["default"]["bulkExport"]["jobs"][&ui_id]["status"],
        "in-progress"
    );
    assert_eq!(*mock.cancels.lock().unwrap(), 0);
}

#[tokio::test]
async fn delete_survives_an_unrelated_second_start_and_keeps_the_new_job() {
    let (base, mock, backend) = serve().await;
    let (_, download_path) = start_and_complete(&base).await;
    let first_path = download_path.trim_end_matches("/download").to_string();
    let first_id = first_path.rsplit('/').next().unwrap().to_string();
    let gate = Arc::new(RequestGate::default());
    *mock.delete_gate.lock().unwrap() = Some(gate.clone());
    let delete_base = base.clone();
    let delete_path = format!("{first_path}/delete");
    let deleting = tokio::spawn(async move { post_form(&delete_base, &delete_path, &[]).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), gate.reached.notified())
        .await
        .expect("delete reached status endpoint");

    post_form(
        &base,
        "/ui/bulk-export",
        &[("name", "Second export"), ("scope", "system")],
    )
    .await;
    gate.release.notify_one();
    let (_, location) = deleting.await.unwrap();
    assert_eq!(location, "/ui/bulk-export");

    let current = backend.get_settings("l2:").await.unwrap().unwrap();
    let jobs = current.document["byTenant"]["default"]["bulkExport"]["jobs"]
        .as_object()
        .unwrap();
    assert!(!jobs.contains_key(&first_id));
    assert!(jobs.values().any(|job| job["name"] == "Second export"));
}

#[tokio::test]
async fn download_all_streams_a_compatible_zip_with_safe_repeated_names() {
    let (base, mock, _) = serve().await;
    *mock.outputs.lock().unwrap() = vec![
        MockOutput {
            resource_type: "Patient/unsafe".to_string(),
            path: format!("/export-file/{REST_JOB_ID}/Patient-0"),
        },
        MockOutput {
            resource_type: "Patient/unsafe".to_string(),
            path: format!("/export-file/{REST_JOB_ID}/Patient-1"),
        },
        MockOutput {
            resource_type: "Patient-unsafe".to_string(),
            path: format!("/export-file/{REST_JOB_ID}/Patient-2"),
        },
    ];
    let (_, download_path) = start_and_complete(&base).await;
    let response = client()
        .get(format!("{base}{download_path}"))
        .header("Authorization", "Bearer secret")
        .header("X-Tenant-ID", "ignored-by-ui-resolution")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/zip");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        response.headers()["content-disposition"],
        "attachment; filename=\"bulk-export.zip\""
    );
    assert!(response.headers().get("content-length").is_none());
    let bytes = response.bytes().await.unwrap();

    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    assert_eq!(zip.len(), 3);
    for (index, expected) in [
        "Patient-unsafe-0001.ndjson",
        "Patient-unsafe-0002.ndjson",
        "Patient-unsafe-0001-02.ndjson",
    ]
    .into_iter()
    .enumerate()
    {
        let mut file = zip.by_index(index).unwrap();
        assert_eq!(file.name(), expected);
        let mut body = String::new();
        file.read_to_string(&mut body).unwrap();
        assert!(body.contains(r#""id":"1""#));
        assert!(body.contains(r#""id":"2""#));
    }
    let downloads = mock.downloads.lock().unwrap().clone();
    assert_eq!(downloads.len(), 3);
    assert!(downloads.iter().all(|seen| {
        seen.authorization.as_deref() == Some("Bearer secret")
            && seen.tenant.as_deref() == Some("default")
    }));
}

#[tokio::test]
async fn presigned_downloads_refresh_each_entry_and_receive_no_credentials() {
    let (base, mock, _) = serve().await;
    *mock.requires_access_token.lock().unwrap() = false;
    *mock.outputs.lock().unwrap() = vec![
        MockOutput {
            resource_type: "Patient".to_string(),
            path: "/presigned/Patient-0?signature=one".to_string(),
        },
        MockOutput {
            resource_type: "Observation".to_string(),
            path: "/presigned/Observation-0?signature=two".to_string(),
        },
    ];
    let (_, download_path) = start_and_complete(&base).await;
    let polls_before = *mock.polls.lock().unwrap();
    let response = client()
        .get(format!("{base}{download_path}"))
        .header("Authorization", "Bearer must-not-leak")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.bytes().await.unwrap();

    assert_eq!(*mock.polls.lock().unwrap() - polls_before, 3);
    let downloads = mock.downloads.lock().unwrap().clone();
    assert_eq!(downloads.len(), 2);
    assert!(
        downloads
            .iter()
            .all(|seen| seen.authorization.is_none() && seen.tenant.is_none())
    );
}

#[tokio::test]
async fn redirect_and_midstream_source_failures_surface_as_body_errors() {
    let (base, mock, _) = serve().await;
    *mock.requires_access_token.lock().unwrap() = false;
    *mock.outputs.lock().unwrap() = vec![MockOutput {
        resource_type: "Patient".to_string(),
        path: "/redirect/Patient-0".to_string(),
    }];
    let (_, download_path) = start_and_complete(&base).await;
    let response = client()
        .get(format!("{base}{download_path}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.bytes().await.is_err(),
        "redirect must truncate the ZIP"
    );
    assert!(mock.downloads.lock().unwrap().is_empty());

    *mock.requires_access_token.lock().unwrap() = true;
    *mock.outputs.lock().unwrap() = vec![MockOutput {
        resource_type: "Patient".to_string(),
        path: format!("/export-file/{REST_JOB_ID}/Patient-0"),
    }];
    *mock.fail_output_after_chunk.lock().unwrap() = true;
    let response = client()
        .get(format!("{base}{download_path}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.bytes().await.is_err(),
        "late source failures must be visible as a streaming body error"
    );
}

#[tokio::test]
async fn zero_output_hides_download_and_rejects_a_forged_direct_request() {
    let (base, mock, _) = serve().await;
    *mock.outputs.lock().unwrap() = Vec::new();
    post_form(
        &base,
        "/ui/bulk-export",
        &[("name", "Zero output"), ("scope", "system")],
    )
    .await;
    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    let card_path = html
        .split("hx-get=\"")
        .map(|s| s.split('"').next().unwrap_or(""))
        .find(|s| s.starts_with("/ui/bulk-export/active/"))
        .unwrap()
        .to_string();
    get_text(&base, &card_path).await;
    let (_, html) = get_text(&base, &card_path).await;
    assert!(!html.contains("Download All Resources"));

    // The complete card deliberately retained no output, so a forged direct
    // request is rejected before any manifest or output request.
    let polls_before = *mock.polls.lock().unwrap();
    let response = client()
        .get(format!("{base}{}", card_path.replace("/card", "/download")))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(*mock.polls.lock().unwrap(), polls_before);
}

#[tokio::test]
async fn a_malformed_fresh_authenticated_url_causes_no_output_fetch() {
    let (base, mock, _) = serve().await;
    let (_, download_path) = start_and_complete(&base).await;
    *mock.outputs.lock().unwrap() = vec![MockOutput {
        resource_type: "Patient".to_string(),
        path: format!("/export-file/{REST_JOB_ID}/Patient-0?forbidden=1"),
    }];
    let downloads_before = mock.downloads.lock().unwrap().len();

    let response = client()
        .get(format!("{base}{download_path}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(mock.downloads.lock().unwrap().len(), downloads_before);
}

#[tokio::test]
async fn patient_options_merge_exact_first_deduplicate_and_limit_results() {
    let (base, mock, _) = serve().await;
    *mock.patients.lock().unwrap() = (0..10)
        .map(|index| {
            patient(
                &format!("alice-{index}"),
                "Alice",
                &format!("Family {index}"),
            )
        })
        .collect();

    let (status, headers, html) =
        post_patient_query(&base, "/ui/bulk-export/patient-options", "alice-3", None).await;
    assert_eq!(status, 200);
    assert_eq!(headers["cache-control"], "private, no-store");
    assert_eq!(html.matches("data-combobox-option").count(), 8, "{html}");
    let exact = html.find("Patient/alice-3").unwrap();
    let first_other = html.find("Patient/alice-0").unwrap();
    assert!(exact < first_other, "exact id must be first: {html}");
    assert_eq!(html.matches("Patient/alice-3").count(), 3, "{html}");

    let requests = mock.requests.lock().unwrap().clone();
    let search = requests
        .iter()
        .find(|request| request.path.ends_with("/Patient/_search"))
        .expect("name search request");
    let form: HashMap<_, _> = form_urlencoded::parse(search.body.as_bytes()).collect();
    assert_eq!(
        form.get("name").map(|value| value.as_ref()),
        Some("alice-3")
    );
    assert_eq!(form.get("_count").map(|value| value.as_ref()), Some("9"));
    assert_eq!(
        form.get("_elements").map(|value| value.as_ref()),
        Some("id,name")
    );
}

#[tokio::test]
async fn patient_options_honor_query_boundaries_and_non_hx_redirect() {
    let (base, mock, _) = serve().await;
    *mock.patients.lock().unwrap() = vec![patient("a", "Ada", "Lovelace")];

    let (_, _, html) = post_patient_query(&base, "/ui/bulk-export/patient-options", "", None).await;
    assert!(html.contains("hx-swap-oob=\"innerHTML\""), "{html}");
    assert!(!html.contains("data-combobox-message-content"), "{html}");
    let (_, _, html) =
        post_patient_query(&base, "/ui/bulk-export/patient-options", "a", None).await;
    assert!(html.contains("Patient/a"));
    assert_eq!(
        mock.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.path.ends_with("/_search"))
            .count(),
        0,
        "one-character queries only attempt exact id"
    );
    let (_, _, html) = post_patient_query(
        &base,
        "/ui/bulk-export/patient-options",
        &"x".repeat(65),
        None,
    )
    .await;
    assert!(html.contains("Suggestions could not be loaded"), "{html}");
    assert!(html.contains("field__hint--error"), "{html}");

    let response = client()
        .post(format!("{base}/ui/bulk-export/patient-options"))
        .form(&[("q", "Ada")])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/ui/bulk-export/new");
}

#[tokio::test]
async fn exact_patient_read_treats_gone_as_no_match_and_rejects_invalid_success_payloads() {
    let (base, mock, _) = serve().await;
    *mock.patient_read_status.lock().unwrap() = 410;
    *mock.patients.lock().unwrap() = vec![patient("search-result", "Gone", "Replacement")];
    let (_, _, html) =
        post_patient_query(&base, "/ui/bulk-export/patient-options", "gone", None).await;
    assert!(html.contains("Patient/search-result"), "{html}");
    assert!(
        mock.requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.path.ends_with("/Patient/_search")),
        "410 exact reads should continue with name search"
    );

    for body in [
        "not json".to_string(),
        serde_json::json!({"resourceType": "Observation", "id": "broken"}).to_string(),
        serde_json::json!({"resourceType": "Patient", "id": "not/valid"}).to_string(),
        serde_json::json!({"resourceType": "Patient", "id": "different"}).to_string(),
    ] {
        let (base, mock, _) = serve().await;
        *mock.patient_read_body.lock().unwrap() = Some(body);
        let (_, _, html) =
            post_patient_query(&base, "/ui/bulk-export/patient-options", "broken", None).await;
        assert!(html.contains("Suggestions could not be loaded"), "{html}");
        assert_eq!(
            mock.requests
                .lock()
                .unwrap()
                .iter()
                .filter(|request| request.path.ends_with("/Patient/_search"))
                .count(),
            0,
            "a malformed successful exact read is a protocol error"
        );
    }
}

#[tokio::test]
async fn not_implemented_name_search_downgrades_once_but_other_failures_do_not() {
    let (base, mock, _) = serve().await;
    *mock.search_status.lock().unwrap() = 501;
    for _ in 0..2 {
        let (_, _, html) =
            post_patient_query(&base, "/ui/bulk-export/patient-options", "Nobody", None).await;
        assert!(html.contains("data-combobox-use-alternate"), "{html}");
    }
    assert_eq!(
        mock.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.path.ends_with("/_search"))
            .count(),
        1,
        "a 501 capability downgrade is cached"
    );

    for status in [403, 500] {
        let (base, mock, _) = serve().await;
        *mock.search_status.lock().unwrap() = status;
        let (_, _, html) =
            post_patient_query(&base, "/ui/bulk-export/patient-options", "Nobody", None).await;
        assert!(html.contains("Suggestions could not be loaded"), "{html}");
        assert!(!html.contains("data-combobox-use-alternate"), "{html}");
    }
}

#[tokio::test]
async fn id_only_mode_renders_its_hint_and_never_attempts_name_search() {
    let (base, mock, _) =
        serve_with_runtime(helios_ui::PatientNameSearchSupport::IdOnly, false).await;
    *mock.patients.lock().unwrap() = vec![patient("p-1", "Pat", "One")];
    let (_, page) = get_text(&base, "/ui/bulk-export/new").await;
    assert!(page.contains("Search by exact logical FHIR ID"), "{page}");
    assert!(
        page.contains("placeholder=\"Search exact FHIR ID\""),
        "{page}"
    );
    assert!(
        !page.contains("placeholder=\"Search name, surname"),
        "{page}"
    );

    let (_, _, html) =
        post_patient_query(&base, "/ui/bulk-export/patient-options", "p-1", None).await;
    assert!(html.contains("Patient/p-1"), "{html}");

    let (_, _, missing_html) =
        post_patient_query(&base, "/ui/bulk-export/patient-options", "an", None).await;
    assert!(
        missing_html.contains("No matching patients found"),
        "{missing_html}"
    );
    assert!(
        !missing_html.contains("Search by exact logical FHIR ID"),
        "the fixed ID-only hint must not be repeated as a search result: {missing_html}"
    );
    assert!(
        mock.requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| !request.path.ends_with("/_search"))
    );
}

#[tokio::test]
async fn patient_lookup_uses_tenant_path_version_and_forwarded_identity() {
    let (base, mock, _) =
        serve_with_runtime(helios_ui::PatientNameSearchSupport::Enabled, true).await;
    *mock.patients.lock().unwrap() = vec![patient("p-1", "Pat", "One")];
    let (_, _, html) = post_patient_query(
        &base,
        "/ui/bulk-export/patient-options",
        "p-1",
        Some("Bearer patient-token"),
    )
    .await;
    assert!(html.contains("Patient/p-1"), "{html}");
    let requests = mock.requests.lock().unwrap().clone();
    assert!(
        requests
            .iter()
            .all(|request| request.path.starts_with("/default/"))
    );
    assert!(
        requests
            .iter()
            .all(|request| request.authorization == "Bearer patient-token")
    );
    assert!(requests.iter().all(|request| request.tenant == "default"));
    assert!(
        requests
            .iter()
            .all(|request| request.accept.contains("fhirVersion=4.0"))
    );
}

#[tokio::test]
async fn selected_patients_use_parameters_and_retry_preserves_the_request() {
    let (base, mock, _) = serve().await;
    *mock.reject.lock().unwrap() = Some(
        serde_json::json!({
            "resourceType": "OperationOutcome",
            "issue": [{"diagnostics": "Patient/p-2 was not found"}]
        })
        .to_string(),
    );
    let (status, _) = post_form(
        &base,
        "/ui/bulk-export",
        &[
            ("name", "Selected people"),
            ("scope", "patient"),
            ("patient", " Patient/p-1, p-2\np-1 "),
            ("patient", "p-3"),
            ("types", "Observation"),
            ("elements", "id,meta"),
            ("type_filter", "Observation?status=final"),
            ("since_preset", "custom"),
            ("since_custom", "2026-08-01T00:00:00Z"),
        ],
    )
    .await;
    assert_eq!(status, 303);

    let requests = mock.requests.lock().unwrap().clone();
    let kickoff = requests.last().unwrap();
    assert_eq!(kickoff.method, Method::POST);
    assert_eq!(kickoff.path, "/Patient/$export");
    assert!(kickoff.query.is_empty());
    assert!(kickoff.content_type.contains("fhirVersion=4.0"));
    assert!(kickoff.accept.contains("fhirVersion=4.0"));
    let parameters: serde_json::Value = serde_json::from_str(&kickoff.body).unwrap();
    let entries = parameters["parameter"].as_array().unwrap();
    let patient_refs: Vec<&str> = entries
        .iter()
        .filter(|entry| entry["name"] == "patient")
        .filter_map(|entry| entry.pointer("/valueReference/reference")?.as_str())
        .collect();
    assert_eq!(patient_refs, ["Patient/p-1", "Patient/p-2", "Patient/p-3"]);
    for (name, value) in [
        ("_type", "Observation"),
        ("_elements", "id,meta"),
        ("_typeFilter", "Observation?status=final"),
    ] {
        assert!(
            entries
                .iter()
                .any(|entry| entry["name"] == name && entry["valueString"] == value)
        );
    }
    assert!(entries.iter().any(|entry| {
        entry["name"] == "_since" && entry["valueInstant"] == "2026-08-01T00:00:00Z"
    }));
    assert!(
        entries
            .iter()
            .filter(|entry| entry["name"] == "_since")
            .all(|entry| entry.get("valueString").is_none())
    );

    *mock.reject.lock().unwrap() = None;
    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    assert!(html.contains("Patient/p-2 was not found"), "{html}");
    let retry_path = html
        .split("action=\"")
        .find(|part| part.starts_with("/ui/bulk-export/active/") && part.contains("/retry"))
        .and_then(|part| part.split('"').next())
        .unwrap();
    post_form(&base, retry_path, &[]).await;
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body, requests[1].body);
    assert_eq!(requests[0].accept, requests[1].accept);
}

#[tokio::test]
async fn invalid_patient_input_fails_without_export_all_and_other_scopes_ignore_it() {
    let (base, mock, backend) = serve().await;
    let (status, html) = post_form_body(
        &base,
        "/ui/bulk-export",
        &[
            ("name", "Invalid patient"),
            ("scope", "patient"),
            ("patient", "Patient/not/valid"),
        ],
    )
    .await;
    assert_eq!(status, 400);
    assert!(html.contains("valid logical Patient IDs"), "{html}");
    assert!(
        html.contains(r#"novalidate data-validation-started="true""#),
        "{html}"
    );
    assert!(mock.kickoffs.lock().unwrap().is_empty());
    let (_, html) = get_text(&base, "/ui/bulk-export").await;
    assert!(!html.contains("valid logical Patient IDs"), "{html}");
    assert!(
        !html.contains("Retry"),
        "invalid input must not create a card"
    );
    assert_no_default_user_jobs(&backend).await;

    for scope in ["system", "group"] {
        post_form(
            &base,
            "/ui/bulk-export",
            &[
                ("name", "Ignore invalid patient"),
                ("scope", scope),
                ("group_id", "g-1"),
                ("patient", "Patient/not/valid"),
            ],
        )
        .await;
    }
    let kickoffs = mock.kickoffs.lock().unwrap();
    assert_eq!(kickoffs.len(), 2);
    assert!(kickoffs.iter().any(|(path, _, _)| path == "/$export"));
    assert!(
        kickoffs
            .iter()
            .any(|(path, _, _)| path == "/Group/g-1/$export")
    );
}
