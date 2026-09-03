//! End-to-end tests for the Active SQL Exports workspace (`/ui/sql/export`,
//! #833): the list-first job store, the `/new` builder, the poll state
//! machine, and tenant/user isolation.
//!
//! Unlike Bulk Export, `$sql-export`'s kick-off/poll/manifest self-calls go
//! through the injectable `ConformanceSource` seam (`Caller`, #833), so these
//! tests mount the UI over a [`helios_ui::StaticConformanceSource`] instead of
//! standing up a mock FHIR server on a real socket.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::{Router, extract::Request as AxRequest, middleware::Next, response::Response};
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::SettingsStore;
use helios_ui::{SqlExportStatus, StaticConformanceSource};
use tower::ServiceExt;

fn nl() -> helios_ui::NlSearch {
    helios_ui::NlSearch {
        enabled: true,
        configured: true,
        model: "test-model".to_string(),
    }
}

fn view_definition(id: &str, name: &str) -> serde_json::Value {
    serde_json::json!({"resourceType": "ViewDefinition", "id": id, "name": name, "resource": "Patient"})
}

fn view_definition_with_status(id: &str, name: &str, status: &str) -> serde_json::Value {
    serde_json::json!({
        "resourceType": "ViewDefinition", "id": id, "name": name, "resource": "Patient",
        "status": status,
    })
}

const LIBRARY_TYPES_SYSTEM: &str =
    "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes";

fn library(id: &str, name: &str, code: &str, status: &str) -> serde_json::Value {
    serde_json::json!({
        "resourceType": "Library", "id": id, "name": name, "status": status,
        "type": {"coding": [{"system": LIBRARY_TYPES_SYSTEM, "code": code}]},
    })
}

/// Reads `X-Test-User` and injects a matching `Principal`, so a request can
/// exercise a specific authenticated settings key without standing up real
/// auth — the same pattern `bulk_export_http.rs` uses.
async fn inject_test_principal(mut request: AxRequest, next: Next) -> Response {
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

async fn backend_with_schema() -> Arc<SqliteBackend> {
    let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite"));
    backend.init_schema().expect("init schema");
    backend
}

fn mount(settings: Option<Arc<dyn SettingsStore>>, source: StaticConformanceSource) -> Router {
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl(),
        None,
        settings,
        "default".to_string(),
        Arc::new(source),
        FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    )
    .layer(axum::middleware::from_fn(inject_test_principal))
}

fn app_with_settings(
    settings: Option<Arc<SqliteBackend>>,
    source: StaticConformanceSource,
) -> Router {
    mount(
        settings.map(|backend| backend as Arc<dyn SettingsStore>),
        source,
    )
}

fn app(backend: &Arc<SqliteBackend>, source: StaticConformanceSource) -> Router {
    app_with_settings(Some(backend.clone()), source)
}

/// A [`SettingsStore`] whose reads delegate to a real backend but whose
/// `patch_settings` always fails — simulating the settings-document write
/// that `sql_export::start` performs right after a successful kick-off
/// (#833).
struct FailingPatchSettingsStore(Arc<SqliteBackend>);

#[async_trait::async_trait]
impl SettingsStore for FailingPatchSettingsStore {
    async fn get_settings(
        &self,
        user_key: &str,
    ) -> helios_persistence::StorageResult<Option<helios_persistence::core::StoredUserSettings>>
    {
        self.0.get_settings(user_key).await
    }

    async fn put_settings(
        &self,
        user_key: &str,
        document: serde_json::Value,
        if_match_version: Option<i64>,
    ) -> helios_persistence::StorageResult<helios_persistence::core::StoredUserSettings> {
        self.0
            .put_settings(user_key, document, if_match_version)
            .await
    }

    async fn patch_settings(
        &self,
        _user_key: &str,
        _merge_patch: serde_json::Value,
        _if_match_version: Option<i64>,
    ) -> helios_persistence::StorageResult<helios_persistence::core::StoredUserSettings> {
        Err(helios_persistence::error::StorageError::Backend(
            helios_persistence::error::BackendError::Internal {
                backend_name: "test".to_string(),
                message: "simulated settings write failure".to_string(),
                source: None,
            },
        ))
    }

    async fn delete_settings(&self, user_key: &str) -> helios_persistence::StorageResult<bool> {
        self.0.delete_settings(user_key).await
    }

    async fn purge_tenant_settings(
        &self,
        tenant_id: &str,
    ) -> helios_persistence::StorageResult<u64> {
        self.0.purge_tenant_settings(tenant_id).await
    }
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
            serde_json::json!({ "byTenant": { tenant: { "sqlExport": { "jobs": { id: job } } } } }),
            None,
        )
        .await
        .expect("seed sql export job");
}

fn get(path: &str) -> Request<Body> {
    Request::get(path).body(Body::empty()).unwrap()
}

fn get_as(path: &str, user: &str) -> Request<Body> {
    Request::get(path)
        .header("x-test-user", user)
        .body(Body::empty())
        .unwrap()
}

fn post_form(path: &str, body: &str) -> Request<Body> {
    Request::post(path)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn post_form_as(path: &str, body: &str, user: &str) -> Request<Body> {
    Request::post(path)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("x-test-user", user)
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn body_text(response: Response) -> String {
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// An `in-progress` job record with one ViewDefinition subject, ready to
/// seed directly (bypassing kick-off) for the poll-matrix tests below.
fn in_progress_job(server_job_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jobId": server_job_id,
        "subjects": [{"name": "patients", "reference": "ViewDefinition/vd1", "kind": "view-definition"}],
        "format": "csv",
        "status": "in-progress",
        "startedAt": "2026-01-01T09:00:00Z",
    })
}

/// A `failed` job record, ready to seed for the job-actions tests below.
fn failed_job(server_job_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jobId": server_job_id,
        "subjects": [{"name": "patients", "reference": "ViewDefinition/vd1", "kind": "view-definition"}],
        "format": "csv",
        "status": "failed",
        "startedAt": "2026-01-01T09:00:00Z",
        "finishedAt": "2026-01-01T09:05:00Z",
        "error": "the view broke",
    })
}

/// A `complete` job record with one output, ready to seed.
fn complete_job(server_job_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jobId": server_job_id,
        "subjects": [{"name": "patients", "reference": "ViewDefinition/vd1", "kind": "view-definition"}],
        "format": "csv",
        "status": "complete",
        "startedAt": "2026-01-01T09:00:00Z",
        "finishedAt": "2026-01-01T09:05:00Z",
        "outputs": [{"name": "patients", "locations": ["http://s/export/job-1/patients-0.csv"]}],
    })
}

/// A `cancelled` job record, ready to seed.
fn cancelled_job(server_job_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jobId": server_job_id,
        "subjects": [{"name": "patients", "reference": "ViewDefinition/vd1", "kind": "view-definition"}],
        "format": "csv",
        "status": "cancelled",
        "startedAt": "2026-01-01T09:00:00Z",
        "finishedAt": "2026-01-01T09:05:00Z",
    })
}

// ---------------------------------------------------------------------------
// Empty list, the builder, and validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn empty_list_shows_a_notice_and_new_offers_subjects_with_their_kind() {
    let backend = backend_with_schema().await;
    let source = StaticConformanceSource::empty().with(
        "ViewDefinition",
        FhirVersion::R4,
        vec![view_definition("vd1", "patients")],
    );
    let app = app(&backend, source);

    let response = app.clone().oneshot(get("/ui/sql/export")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("No SQL exports yet"));
    assert!(html.contains(r#"href="/ui/sql/export/new""#));
    assert!(html.contains("0 exports"));

    let response = app
        .clone()
        .oneshot(get("/ui/sql/export/new"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(r#"value="ViewDefinition/vd1""#));
    assert!(html.contains(">ViewDefinition<"));
}

/// The full builder structure (#834): the back link to the list, the
/// optional name field, one table row per subject carrying its kind tag,
/// `data-kind`, and status, the four format radios with NDJSON checked by
/// default, a single submit control, the table's filter/switch/select-all
/// tools rendered `hidden`, and the "n of m selected" hint reflecting that
/// nothing is checked yet.
#[tokio::test]
async fn new_page_renders_the_builder_structure() {
    let backend = backend_with_schema().await;
    let source = StaticConformanceSource::empty()
        .with(
            "ViewDefinition",
            FhirVersion::R4,
            vec![
                view_definition_with_status("vd1", "patients_flat", "active"),
                view_definition_with_status("vd2", "encounters_flat", "draft"),
            ],
        )
        .with(
            "Library",
            FhirVersion::R4,
            vec![library("q1", "patient_counts", "sql-query", "active")],
        );
    let app = app(&backend, source);

    let response = app
        .clone()
        .oneshot(get("/ui/sql/export/new"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // Back link to the list, sharing the list page's own title.
    assert!(html.contains(r#"<a class="back-link" href="/ui/sql/export">"#));
    assert!(html.contains(">SQL Exports<"));

    // The name field: present, optional (no `required`).
    assert!(html.contains(r#"<input class="field__input" type="text" name="name""#));
    assert!(!html.contains(r#"name="name" required"#));

    // One row per subject — ViewDefinitions then SQL Queries, each group
    // name-sorted — each carrying its kind tag, `data-kind`, and status.
    assert!(html.contains(r#"data-kind="view-definition" data-name="encounters_flat""#));
    assert!(html.contains(r#"data-kind="view-definition" data-name="patients_flat""#));
    assert!(html.contains(r#"data-kind="sql-query" data-name="patient_counts""#));
    assert!(html.contains(">ViewDefinition<"));
    assert!(html.contains(">SQL Query<"));
    assert!(html.contains(">active<"));
    assert!(html.contains(">draft<"));

    // Format radios: four, NDJSON checked, nothing else.
    assert_eq!(html.matches(r#"name="format""#).count(), 4);
    assert!(html.contains(r#"value="ndjson" checked"#));
    assert!(!html.contains(r#"value="csv" checked"#));
    assert!(!html.contains(r#"value="json" checked"#));
    assert!(!html.contains(r#"value="parquet" checked"#));

    // Exactly one Start Export control, wired to the builder's own form
    // (the shared shell's sidebar renders its own unrelated submit buttons,
    // one per enabled FHIR version, so this checks the specific control
    // rather than every `type="submit"` on the page).
    assert!(
        html.contains(r#"<form method="post" action="/ui/sql/export" class="bulk-export-form">"#)
    );
    assert_eq!(html.matches("Start Export").count(), 1);

    // The table's tools and the header select-all render, but stay hidden
    // and inert until a later enhancement reveals them.
    assert!(html.contains(r#"class="card-head__tools card-head__tools--subjects" hidden"#));
    assert!(html.contains(r#"aria-label="Select all" hidden"#));
    assert_eq!(html.matches(r#"class="seg__btn""#).count(), 4);
    assert!(html.contains(r#"type="search""#));
    assert!(html.contains(r#"aria-label="Filter subjects""#));

    // Nothing marked yet: "0 of 3 selected".
    assert!(html.contains("0 of 3 selected"));
}

#[tokio::test]
async fn prefill_checks_matching_subjects_and_silently_ignores_unknown_ones() {
    let backend = backend_with_schema().await;
    let source = StaticConformanceSource::empty()
        .with(
            "ViewDefinition",
            FhirVersion::R4,
            vec![
                view_definition("vd1", "patients"),
                view_definition("vd2", "encounters"),
            ],
        )
        .with(
            "Library",
            FhirVersion::R4,
            vec![library("q1", "counts", "sql-query", "active")],
        );
    let app = app(&backend, source);

    let response = app
        .clone()
        .oneshot(get(
            "/ui/sql/export/new?subject=ViewDefinition%2Fvd1&subject=Library%2Fq1&subject=Library%2Fnope",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(r#"value="ViewDefinition/vd1" aria-label="patients" checked"#));
    assert!(html.contains(r#"value="Library/q1" aria-label="counts" checked"#));
    assert!(html.contains(r#"value="ViewDefinition/vd2" aria-label="encounters">"#));
    assert!(!html.contains("Library/nope"));
    assert!(html.contains("2 of 3 selected"));

    // No `?subject=` at all renders exactly like a bare `GET /new`.
    let response = app.oneshot(get("/ui/sql/export/new")).await.unwrap();
    let html = body_text(response).await;
    assert!(html.contains("0 of 3 selected"));
}

#[tokio::test]
async fn an_empty_store_offers_the_two_creation_links_but_a_degraded_fetch_does_not() {
    let backend = backend_with_schema().await;
    let app = app(&backend, StaticConformanceSource::empty());

    let response = app
        .clone()
        .oneshot(get("/ui/sql/export/new"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("Nothing to export yet"));
    assert!(html.contains(r#"href="/ui/sql/view-definitions?vd=new""#));
    assert!(html.contains(r#"href="/ui/sql/queries?lib=new""#));
    // No builder form at all — not the name field, not a format, not Start
    // Export. (The page's shared shell renders its own, unrelated `<form>`
    // for the sidebar's FHIR-version switcher, so this checks the specific
    // markup the builder itself would own rather than a blanket "<form".)
    assert!(!html.contains(r#"action="/ui/sql/export""#));
    assert!(!html.contains(r#"name="format""#));
    assert!(!html.contains("Start Export"));

    // A fetch failure with nothing loaded is not the same as an empty store:
    // the "Nothing to export yet" card would be a false claim, so only the
    // degraded notice renders.
    let degraded = StaticConformanceSource::empty().with_fetch_error(
        "ViewDefinition",
        FhirVersion::R4,
        "boom",
    );
    let response = app_with_settings(None, degraded)
        .oneshot(get("/ui/sql/export/new"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("could not be loaded"));
    assert!(!html.contains("Nothing to export yet"));
    assert!(!html.contains(r#"action="/ui/sql/export""#));
}

#[tokio::test]
async fn starting_without_a_subject_rerenders_the_builder_with_an_error() {
    let backend = backend_with_schema().await;
    let source = StaticConformanceSource::empty().with(
        "ViewDefinition",
        FhirVersion::R4,
        vec![view_definition("vd1", "patients")],
    );
    let app = app(&backend, source.clone());

    let response = app
        .clone()
        .oneshot(post_form("/ui/sql/export", "name=My+export&format=csv"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("at least one subject"));
    // The name and format the submission carried are conserved.
    assert!(html.contains(r#"value="My export""#));
    assert!(html.contains(r#"value="csv" checked"#));
    assert!(
        source.export_calls().is_empty(),
        "no kick-off without a subject"
    );
}

#[tokio::test]
async fn an_unknown_subject_reference_rerenders_the_builder_without_a_kickoff() {
    let backend = backend_with_schema().await;
    let source = StaticConformanceSource::empty().with(
        "ViewDefinition",
        FhirVersion::R4,
        vec![view_definition("vd1", "patients")],
    );
    let app = app(&backend, source.clone());

    let response = app
        .clone()
        .oneshot(post_form(
            "/ui/sql/export",
            "name=x&subject=ViewDefinition%2Fvd1&subject=ViewDefinition%2Fgone&format=json",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("no longer available"));
    // The name, format, and every still-valid reference are conserved.
    assert!(html.contains(r#"value="x""#));
    assert!(html.contains(r#"value="json" checked"#));
    assert!(html.contains(r#"value="ViewDefinition/vd1" aria-label="patients" checked"#));
    assert!(
        source.export_calls().is_empty(),
        "an unresolved subject must not reach $sql-export"
    );
    assert!(backend.get_settings("l2:").await.unwrap().is_none());
}

// ---------------------------------------------------------------------------
// Kick-off: the stored record and the list's card
// ---------------------------------------------------------------------------

#[tokio::test]
async fn starting_an_export_stores_an_in_progress_record_and_the_list_polls_its_card() {
    let backend = backend_with_schema().await;
    let source = StaticConformanceSource::empty()
        .with(
            "ViewDefinition",
            FhirVersion::R4,
            vec![view_definition("vd1", "patients")],
        )
        // Keeps the list-page poll from moving the job past in-progress, so
        // this test can focus on the kick-off record and the card shape.
        .with_export_status(SqlExportStatus::Running(None));
    let app = app(&backend, source);

    let response = app
        .clone()
        .oneshot(post_form(
            "/ui/sql/export",
            "subject=ViewDefinition%2Fvd1&format=csv",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/ui/sql/export");

    let stored = backend
        .get_settings("l2:")
        .await
        .unwrap()
        .expect("settings written");
    let jobs = stored.document["byTenant"]["default"]["sqlExport"]["jobs"]
        .as_object()
        .cloned()
        .expect("jobs object");
    assert_eq!(jobs.len(), 1);
    let (id, job) = jobs.iter().next().unwrap();
    assert_eq!(job["jobId"], "static-job");
    assert_eq!(job["status"], "in-progress");
    assert_eq!(job["format"], "csv");
    assert!(!job["startedAt"].as_str().unwrap_or_default().is_empty());
    assert_eq!(
        job["subjects"],
        serde_json::json!([
            {"name": "patients", "reference": "ViewDefinition/vd1", "kind": "view-definition"}
        ])
    );

    let response = app.clone().oneshot(get("/ui/sql/export")).await.unwrap();
    let html = body_text(response).await;
    assert!(html.contains(&format!(r#"id="job-{id}""#)));
    assert!(html.contains(&format!(r#"hx-get="/ui/sql/export/{id}/card""#)));
    assert!(html.contains("every 5s"));
    assert!(html.contains("1 exports"));
    assert!(html.contains("1 running"));
}

/// #833: a successful kick-off whose settings-store write then fails must
/// not vanish silently — the redirect carries the server's job id, and the
/// list shows a visible notice with a way to still reach Files, instead of
/// just a `tracing::error!` no one sees.
#[tokio::test]
async fn a_kickoff_that_cannot_be_stored_still_surfaces_a_visible_notice() {
    let backend = backend_with_schema().await;
    let source = StaticConformanceSource::empty().with(
        "ViewDefinition",
        FhirVersion::R4,
        vec![view_definition("vd1", "patients")],
    );
    let app = mount(
        Some(Arc::new(FailingPatchSettingsStore(backend.clone())) as Arc<dyn SettingsStore>),
        source,
    );

    let response = app
        .clone()
        .oneshot(post_form(
            "/ui/sql/export",
            "subject=ViewDefinition%2Fvd1&format=csv",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers()["location"].to_str().unwrap().to_string();
    assert_eq!(location, "/ui/sql/export?store-error=static-job");

    // Nothing was actually persisted (every patch fails on this double).
    assert!(backend.get_settings("l2:").await.unwrap().is_none());

    let response = app
        .clone()
        .oneshot(get("/ui/sql/export?store-error=static-job"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("could not be added to this list"));
    assert!(html.contains(">static-job<"));
    // #835: no notebook entry means no detail page to link to (the record
    // never got written) — Copy job id, server-rendered hidden like every
    // other one, is all the notice offers.
    assert!(html.contains(r#"data-copy-job-id="static-job""#));
    assert!(!html.contains("/ui/sql/files"));
}

// ---------------------------------------------------------------------------
// The poll state machine, exercised on directly-seeded jobs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_running_job_shows_progress_and_keeps_polling() {
    let backend = backend_with_schema().await;
    seed_job(&backend, "default", "job-a", in_progress_job("job-1")).await;
    let source = StaticConformanceSource::empty()
        .with_export_status(SqlExportStatus::Running(Some("35%".to_string())));
    let app = app(&backend, source);

    let response = app
        .clone()
        .oneshot(get("/ui/sql/export/job-a/card"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("35% · 1 subject (1 ViewDefinition) · CSV · started 2026-01-01 09:00 UTC")
    );
    assert!(html.contains("every 5s"));

    let stored = backend.get_settings("l2:").await.unwrap().unwrap();
    let job = &stored.document["byTenant"]["default"]["sqlExport"]["jobs"]["job-a"];
    assert_eq!(job["status"], "in-progress");
    assert_eq!(job["progress"], "35%");
}

#[tokio::test]
async fn a_done_job_with_a_successful_manifest_completes_and_persists_its_outputs() {
    let backend = backend_with_schema().await;
    seed_job(&backend, "default", "job-a", in_progress_job("job-1")).await;
    let manifest = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "output", "part": [
            {"name": "name", "valueString": "patients"},
            {"name": "location", "valueUri": "http://s/export/job-1/patients-0.csv"},
        ]}]
    });
    let source = StaticConformanceSource::empty()
        .with_export_status(SqlExportStatus::Done)
        .with_export_manifest(Ok(manifest));
    let app = app(&backend, source);

    let response = app
        .clone()
        .oneshot(get("/ui/sql/export/job-a/card"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    // #835: View files now links to the job's own detail page (its local
    // id), not the retired Files lookup (the server's job id).
    assert!(html.contains(r#"href="/ui/sql/export/job-a""#));
    assert!(!html.contains("hx-get"), "a terminal card must not poll");
    assert!(html.contains("1 file"));

    let stored = backend.get_settings("l2:").await.unwrap().unwrap();
    let job = &stored.document["byTenant"]["default"]["sqlExport"]["jobs"]["job-a"];
    assert_eq!(job["status"], "complete");
    assert_eq!(
        job["outputs"],
        // The manifest's absolute location is persisted as a same-origin
        // path (#833).
        serde_json::json!([{"name": "patients", "locations": ["/export/job-1/patients-0.csv"]}])
    );
    assert!(!job["finishedAt"].as_str().unwrap_or_default().is_empty());
}

#[tokio::test]
async fn a_done_job_with_a_failing_manifest_fails_with_the_message() {
    let backend = backend_with_schema().await;
    seed_job(&backend, "default", "job-a", in_progress_job("job-1")).await;
    let source = StaticConformanceSource::empty()
        .with_export_status(SqlExportStatus::Done)
        .with_export_manifest(Err("the result endpoint returned 500".to_string()));
    let app = app(&backend, source);

    let response = app
        .clone()
        .oneshot(get("/ui/sql/export/job-a/card"))
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains("the result endpoint returned 500"));
    assert!(html.contains(">Failed<"));

    let stored = backend.get_settings("l2:").await.unwrap().unwrap();
    let job = &stored.document["byTenant"]["default"]["sqlExport"]["jobs"]["job-a"];
    assert_eq!(job["status"], "failed");
    assert_eq!(job["error"], "the result endpoint returned 500");
}

#[tokio::test]
async fn an_unknown_job_is_marked_cancelled_with_a_translated_reason() {
    let backend = backend_with_schema().await;
    seed_job(&backend, "default", "job-a", in_progress_job("job-1")).await;
    let source = StaticConformanceSource::empty().with_export_status(SqlExportStatus::Unknown);
    let app = app(&backend, source);

    let response = app
        .clone()
        .oneshot(get("/ui/sql/export/job-a/card"))
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains(">Cancelled<"));
    assert!(html.contains("the server no longer knows this job"));

    let stored = backend.get_settings("l2:").await.unwrap().unwrap();
    let job = &stored.document["byTenant"]["default"]["sqlExport"]["jobs"]["job-a"];
    assert_eq!(job["status"], "cancelled");
    assert_eq!(job["error"], "the server no longer knows this job");
}

#[tokio::test]
async fn an_unavailable_poll_keeps_the_job_in_progress_and_still_polls() {
    let backend = backend_with_schema().await;
    seed_job(&backend, "default", "job-a", in_progress_job("job-1")).await;
    let source = StaticConformanceSource::empty().with_export_status(SqlExportStatus::Unavailable(
        "status poll answered 401".to_string(),
    ));
    let app = app(&backend, source);

    let response = app
        .clone()
        .oneshot(get("/ui/sql/export/job-a/card"))
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains(">In progress<"));
    assert!(html.contains("status unavailable: status poll answered 401"));
    assert!(html.contains("every 5s"), "still polling while unavailable");

    let stored = backend.get_settings("l2:").await.unwrap().unwrap();
    let job = &stored.document["byTenant"]["default"]["sqlExport"]["jobs"]["job-a"];
    assert_eq!(job["status"], "in-progress");
    assert_eq!(job["pollError"], "status poll answered 401");
}

/// The list page itself polls an in-progress job before rendering, not just
/// the card's own htmx fragment — a plain reload without JavaScript must
/// see a server-side completion.
#[tokio::test]
async fn the_list_page_transitions_a_job_without_htmx() {
    let backend = backend_with_schema().await;
    seed_job(&backend, "default", "job-a", in_progress_job("job-1")).await;
    let manifest = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "output", "part": [
            {"name": "name", "valueString": "patients"},
            {"name": "location", "valueUri": "http://s/export/job-1/patients-0.csv"},
        ]}]
    });
    let source = StaticConformanceSource::empty()
        .with_export_status(SqlExportStatus::Done)
        .with_export_manifest(Ok(manifest));
    let app = app(&backend, source);

    let response = app.clone().oneshot(get("/ui/sql/export")).await.unwrap();
    let html = body_text(response).await;
    assert!(html.contains(">Complete<"));
    assert!(html.contains(r#"href="/ui/sql/export/job-a""#));

    let stored = backend.get_settings("l2:").await.unwrap().unwrap();
    assert_eq!(
        stored.document["byTenant"]["default"]["sqlExport"]["jobs"]["job-a"]["status"],
        "complete"
    );
}

// ---------------------------------------------------------------------------
// Isolation and availability
// ---------------------------------------------------------------------------

#[tokio::test]
async fn card_fragment_of_an_unknown_id_is_not_found() {
    let backend = backend_with_schema().await;
    let app = app(&backend, StaticConformanceSource::empty());
    let response = app
        .oneshot(get("/ui/sql/export/does-not-exist/card"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn jobs_are_isolated_by_tenant_and_by_user() {
    let backend = backend_with_schema().await;
    seed_job(
        &backend,
        "other-tenant",
        "other-tenant-job",
        in_progress_job("job-x"),
    )
    .await;
    seed_job_for_user(
        &backend,
        "u2:4:test:owner",
        "default",
        "owners-job",
        in_progress_job("job-y"),
    )
    .await;
    let app = app(&backend, StaticConformanceSource::empty());

    // The default tenant/user's list sees neither job.
    let response = app.clone().oneshot(get("/ui/sql/export")).await.unwrap();
    let html = body_text(response).await;
    assert!(html.contains("No SQL exports yet"));

    // Neither job's fragment is reachable from the wrong scope.
    let response = app
        .clone()
        .oneshot(get("/ui/sql/export/other-tenant-job/card"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = app
        .clone()
        .oneshot(get("/ui/sql/export/owners-job/card"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // A different authenticated user cannot reach the owner's job either.
    let response = app
        .clone()
        .oneshot(get_as("/ui/sql/export/owners-job/card", "attacker"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // The owner's own request does see it.
    let response = app
        .clone()
        .oneshot(get_as("/ui/sql/export", "owner"))
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains(r#"id="job-owners-job""#));
}

#[tokio::test]
async fn no_settings_store_reports_unavailable_without_a_new_button() {
    let app = app_with_settings(None, StaticConformanceSource::empty());

    let response = app.clone().oneshot(get("/ui/sql/export")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("cannot be tracked"));
    assert!(!html.contains(r#"href="/ui/sql/export/new""#));
    assert!(!html.contains("No SQL exports yet"));

    // The builder still works without a settings store — only the list's
    // tracking is degraded.
    let response = app
        .clone()
        .oneshot(get("/ui/sql/export/new"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// $sql-export receives the subject's display name, not the id segment of
// its reference (#833)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn starting_an_export_submits_display_names_disambiguating_duplicates() {
    let backend = backend_with_schema().await;
    let uuid_id = "3f9c9a2e-8f2a-4e7a-9d38-2a7a6b6a2b11";
    let source = StaticConformanceSource::empty()
        .with(
            "ViewDefinition",
            FhirVersion::R4,
            vec![
                view_definition(uuid_id, "patients_flat"),
                view_definition("vd2", "patients_flat"),
            ],
        )
        .with_export_status(SqlExportStatus::Running(None));
    let app = app(&backend, source.clone());

    let response = app
        .clone()
        .oneshot(post_form(
            "/ui/sql/export",
            &format!("subject=ViewDefinition%2F{uuid_id}&subject=ViewDefinition%2Fvd2&format=csv"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let calls = source.export_calls();
    let start = calls
        .iter()
        .find(|c| c.operation == "start")
        .expect("a start call was recorded");
    assert_eq!(
        start.subjects,
        vec![
            (
                "patients_flat".to_string(),
                format!("ViewDefinition/{uuid_id}")
            ),
            (
                "patients_flat-2".to_string(),
                "ViewDefinition/vd2".to_string()
            ),
        ],
        "the output name is the ViewDefinition's display name, not its id, and a shared name is disambiguated"
    );
}

// ---------------------------------------------------------------------------
// Job actions: Cancel, Retry, Run again, Remove from list (#833)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancelling_an_in_progress_job_marks_it_cancelled_with_a_clean_slate() {
    let backend = backend_with_schema().await;
    seed_job(&backend, "default", "job-a", in_progress_job("job-1")).await;
    let source = StaticConformanceSource::empty();
    let app = app(&backend, source.clone());

    let response = app
        .clone()
        .oneshot(post_form("/ui/sql/export/job-a/cancel", ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/ui/sql/export");

    let stored = backend.get_settings("l2:").await.unwrap().unwrap();
    let job = &stored.document["byTenant"]["default"]["sqlExport"]["jobs"]["job-a"];
    assert_eq!(job["status"], "cancelled");
    assert!(job["error"].is_null(), "a user cancel carries no error");
    assert!(job["progress"].is_null());
    assert!(!job["finishedAt"].as_str().unwrap_or_default().is_empty());

    let calls = source.export_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].operation, "cancel");
}

#[tokio::test]
async fn cancelling_a_terminal_job_is_a_silent_no_op() {
    let backend = backend_with_schema().await;
    seed_job(&backend, "default", "job-a", failed_job("job-1")).await;
    let source = StaticConformanceSource::empty();
    let app = app(&backend, source.clone());

    let response = app
        .clone()
        .oneshot(post_form("/ui/sql/export/job-a/cancel", ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/ui/sql/export");

    let stored = backend.get_settings("l2:").await.unwrap().unwrap();
    let job = &stored.document["byTenant"]["default"]["sqlExport"]["jobs"]["job-a"];
    assert_eq!(job["status"], "failed");
    assert!(
        source.export_calls().is_empty(),
        "no cancel call for a job that is not in-progress"
    );
}

#[tokio::test]
async fn retrying_a_failed_job_creates_a_new_record_and_leaves_the_original_untouched() {
    let backend = backend_with_schema().await;
    seed_job(&backend, "default", "job-a", failed_job("job-1")).await;
    let source =
        StaticConformanceSource::empty().with_export_status(SqlExportStatus::Running(None));
    let app = app(&backend, source.clone());

    let response = app
        .clone()
        .oneshot(post_form("/ui/sql/export/job-a/retry", ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/ui/sql/export");

    let stored = backend.get_settings("l2:").await.unwrap().unwrap();
    let jobs = stored.document["byTenant"]["default"]["sqlExport"]["jobs"]
        .as_object()
        .unwrap()
        .clone();
    assert_eq!(jobs.len(), 2, "the original and a new record both exist");

    let original = &jobs["job-a"];
    assert_eq!(original["status"], "failed");
    assert_eq!(original["jobId"], "job-1");

    let (new_id, new_job) = jobs.iter().find(|(id, _)| id.as_str() != "job-a").unwrap();
    assert_ne!(new_id, "job-a");
    assert_eq!(new_job["status"], "in-progress");
    assert_eq!(new_job["jobId"], "static-job");
    assert_eq!(new_job["subjects"], original["subjects"]);
    assert_eq!(new_job["format"], original["format"]);
    assert!(new_job["finishedAt"].is_null());

    let calls = source.export_calls();
    assert_eq!(calls.iter().filter(|c| c.operation == "start").count(), 1);
}

#[tokio::test]
async fn retry_is_a_silent_no_op_outside_failed() {
    for job in [
        in_progress_job("job-1"),
        complete_job("job-1"),
        cancelled_job("job-1"),
    ] {
        let backend = backend_with_schema().await;
        seed_job(&backend, "default", "job-a", job).await;
        let source = StaticConformanceSource::empty();
        let app = app(&backend, source.clone());

        let response = app
            .clone()
            .oneshot(post_form("/ui/sql/export/job-a/retry", ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        let stored = backend.get_settings("l2:").await.unwrap().unwrap();
        let jobs = stored.document["byTenant"]["default"]["sqlExport"]["jobs"]
            .as_object()
            .unwrap();
        assert_eq!(jobs.len(), 1, "retry must only ever act on a failed job");
        assert!(source.export_calls().is_empty());
    }
}

#[tokio::test]
async fn rerunning_a_complete_or_cancelled_job_creates_a_new_record() {
    for seed in [complete_job("job-1"), cancelled_job("job-1")] {
        let original_status = seed["status"].as_str().unwrap().to_string();
        let backend = backend_with_schema().await;
        seed_job(&backend, "default", "job-a", seed).await;
        let source = StaticConformanceSource::empty();
        let app = app(&backend, source.clone());

        let response = app
            .clone()
            .oneshot(post_form("/ui/sql/export/job-a/rerun", ""))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::SEE_OTHER,
            "{original_status}"
        );

        let stored = backend.get_settings("l2:").await.unwrap().unwrap();
        let jobs = stored.document["byTenant"]["default"]["sqlExport"]["jobs"]
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(jobs.len(), 2, "{original_status}");
        assert_eq!(jobs["job-a"]["status"], original_status);
        let (_, new_job) = jobs.iter().find(|(id, _)| id.as_str() != "job-a").unwrap();
        assert_eq!(new_job["status"], "in-progress", "{original_status}");
    }
}

#[tokio::test]
async fn rerunning_an_in_progress_job_is_a_silent_no_op() {
    let backend = backend_with_schema().await;
    seed_job(&backend, "default", "job-a", in_progress_job("job-1")).await;
    let source = StaticConformanceSource::empty();
    let app = app(&backend, source.clone());

    let response = app
        .clone()
        .oneshot(post_form("/ui/sql/export/job-a/rerun", ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let stored = backend.get_settings("l2:").await.unwrap().unwrap();
    let jobs = stored.document["byTenant"]["default"]["sqlExport"]["jobs"]
        .as_object()
        .unwrap();
    assert_eq!(jobs.len(), 1);
    assert!(source.export_calls().is_empty());
}

#[tokio::test]
async fn removing_a_terminal_job_deletes_the_record() {
    let backend = backend_with_schema().await;
    seed_job(&backend, "default", "job-a", failed_job("job-1")).await;
    let app = app(&backend, StaticConformanceSource::empty());

    let response = app
        .clone()
        .oneshot(post_form("/ui/sql/export/job-a/remove", ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/ui/sql/export");

    let stored = backend.get_settings("l2:").await.unwrap().unwrap();
    assert!(
        stored.document["byTenant"]["default"]["sqlExport"]["jobs"]
            .get("job-a")
            .is_none()
    );
}

#[tokio::test]
async fn removing_an_in_progress_job_is_a_silent_no_op() {
    let backend = backend_with_schema().await;
    seed_job(&backend, "default", "job-a", in_progress_job("job-1")).await;
    let app = app(&backend, StaticConformanceSource::empty());

    let response = app
        .clone()
        .oneshot(post_form("/ui/sql/export/job-a/remove", ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let stored = backend.get_settings("l2:").await.unwrap().unwrap();
    assert!(
        stored.document["byTenant"]["default"]["sqlExport"]["jobs"]
            .get("job-a")
            .is_some(),
        "an in-progress job can only be removed after Cancel"
    );
}

#[tokio::test]
async fn removing_another_users_job_is_a_silent_no_op() {
    let backend = backend_with_schema().await;
    seed_job_for_user(
        &backend,
        "u2:4:test:owner",
        "default",
        "owners-job",
        failed_job("job-1"),
    )
    .await;
    let app = app(&backend, StaticConformanceSource::empty());

    let response = app
        .clone()
        .oneshot(post_form_as(
            "/ui/sql/export/owners-job/remove",
            "",
            "attacker",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let stored = backend
        .get_settings("u2:4:test:owner")
        .await
        .unwrap()
        .unwrap();
    assert!(
        stored.document["byTenant"]["default"]["sqlExport"]["jobs"]
            .get("owners-job")
            .is_some(),
        "the owner's record must survive another user's remove attempt"
    );
}

#[tokio::test]
async fn every_action_on_an_unknown_id_is_a_silent_no_op() {
    let backend = backend_with_schema().await;
    let source = StaticConformanceSource::empty();
    let app = app(&backend, source.clone());

    for action in ["cancel", "retry", "rerun", "remove"] {
        let response = app
            .clone()
            .oneshot(post_form(
                &format!("/ui/sql/export/does-not-exist/{action}"),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "{action}");
        assert_eq!(response.headers()["location"], "/ui/sql/export", "{action}");
    }
    assert!(backend.get_settings("l2:").await.unwrap().is_none());
    assert!(source.export_calls().is_empty());
}

// ---------------------------------------------------------------------------
// Job detail (#835): GET /ui/sql/export/{id} and its htmx fragment
// ---------------------------------------------------------------------------

/// A `complete` job with two outputs — one of them two shards — and subjects
/// of two kinds, ready to seed for the detail page tests below. The
/// `outputs` order deliberately does not match the `subjects` order, so
/// resolving each row back to its subject by name (not position) actually
/// gets exercised.
fn complete_job_with_two_outputs(server_job_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jobId": server_job_id,
        "name": "Monthly flat files",
        "subjects": [
            {"name": "patients", "reference": "ViewDefinition/vd1", "kind": "view-definition"},
            {"name": "encounter_counts", "reference": "Library/lib1", "kind": "sql-query"},
        ],
        "format": "parquet",
        "status": "complete",
        "startedAt": "2026-01-01T09:00:00Z",
        "finishedAt": "2026-01-01T09:05:08Z",
        "outputs": [
            {"name": "encounter_counts", "locations": ["/export/job-1/encounter_counts-0.parquet"]},
            {"name": "patients", "locations": [
                "/export/job-1/patients-0.parquet",
                "/export/job-1/patients-1.parquet"
            ]},
        ],
    })
}

/// A `failed` job whose error names one of its own subjects via the
/// server's `query '<name>': …` pattern.
fn failed_job_naming_a_subject(server_job_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jobId": server_job_id,
        "subjects": [{"name": "v03_counts", "reference": "Library/lib1", "kind": "sql-query"}],
        "format": "csv",
        "status": "failed",
        "startedAt": "2026-01-01T09:00:00Z",
        "finishedAt": "2026-01-01T09:02:00Z",
        "error": "Export job 'x' failed: query 'v03_counts': column \"ward\" does not exist",
    })
}

/// A `failed` job whose error matches no known pattern (a kick-off failure,
/// or a message this UI does not recognize) — the generic notice fallback.
fn failed_job_with_unmatched_error(server_job_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jobId": server_job_id,
        "subjects": [{"name": "patients", "reference": "ViewDefinition/vd1", "kind": "view-definition"}],
        "format": "csv",
        "status": "failed",
        "startedAt": "2026-01-01T09:00:00Z",
        "finishedAt": "2026-01-01T09:02:00Z",
        "error": "connection refused",
    })
}

/// A `cancelled` job carrying the reaper's own reason (#833's
/// `sql-export-cancelled-reason`).
fn cancelled_job_with_reason(server_job_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jobId": server_job_id,
        "subjects": [{"name": "patients", "reference": "ViewDefinition/vd1", "kind": "view-definition"}],
        "format": "csv",
        "status": "cancelled",
        "startedAt": "2026-01-01T09:00:00Z",
        "finishedAt": "2026-01-01T09:05:00Z",
        "error": "the server no longer knows this job",
    })
}

/// A `failed` job whose kick-off never got a server job id at all: the Job
/// id field falls back to an em dash, and Copy job id never renders.
fn kickoff_failed_job() -> serde_json::Value {
    serde_json::json!({
        "subjects": [{"name": "patients", "reference": "ViewDefinition/vd1", "kind": "view-definition"}],
        "format": "csv",
        "status": "failed",
        "startedAt": "2026-01-01T09:00:00Z",
        "finishedAt": "2026-01-01T09:00:01Z",
        "error": "the export could not be started",
    })
}

#[tokio::test]
async fn detail_page_renders_a_complete_job_with_resolved_outputs() {
    let backend = backend_with_schema().await;
    seed_job(
        &backend,
        "default",
        "job-a",
        complete_job_with_two_outputs("job-1"),
    )
    .await;
    let app = app(&backend, StaticConformanceSource::empty());

    let response = app.oneshot(get("/ui/sql/export/job-a")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // Header: name, lede, chip, Run again (not Retry).
    assert!(html.contains(">Monthly flat files<"));
    assert!(html.contains("Finished 2026-01-01 09:05 UTC · 5m 08s · Parquet"));
    assert!(html.contains(">Complete<"));
    assert!(html.contains(r#"action="/ui/sql/export/job-a/rerun""#));
    assert!(!html.contains(r#"action="/ui/sql/export/job-a/retry""#));
    assert!(!html.contains(r#"action="/ui/sql/export/job-a/cancel""#));

    // No progress bar in a terminal state, and no more polling.
    assert!(!html.contains("progress--in-progress"));
    assert!(!html.contains("hx-get"));

    // The Job card: id in <code>, format, started (with seconds), duration,
    // and both subjects as pills.
    assert!(html.contains("<code>job-1</code>"));
    assert!(html.contains(">2026-01-01 09:00:00 UTC<"));
    assert!(html.contains(">5m 08s<"));
    assert!(html.contains(r#"<span class="tag tag--type">ViewDefinition</span> patients"#));
    assert!(html.contains(r#"<span class="tag tag--type">SQL Query</span> encounter_counts"#));

    // The Output files table: 3 downloads total, in the record's own
    // `outputs` order — encounter_counts first, even though it is the
    // *second* subject submitted — each row resolved to its subject by
    // output name rather than by manifest position (build_output_rows_*
    // unit tests exercise that resolution directly).
    assert!(html.contains(">3<"));
    assert!(html.contains("patients-0.parquet"));
    assert!(html.contains("patients-1.parquet"));
    assert!(html.contains("encounter_counts-0.parquet"));
    let encounter_output_at = html.find("<td>encounter_counts</td>").expect("output row");
    let patients_output_at = html.find("<td>patients</td>").expect("output row");
    assert!(
        encounter_output_at < patients_output_at,
        "outputs render in the record's own persisted order, not submission order"
    );
    let encounter_subject_at = html[encounter_output_at..]
        .find(r#"<span class="tag tag--type">SQL Query</span> encounter_counts"#)
        .expect("the encounter_counts row resolves to its own sql-query subject");
    assert!(
        encounter_output_at + encounter_subject_at < patients_output_at,
        "the resolved subject pill belongs to the encounter_counts row, not a later one"
    );
}

#[tokio::test]
async fn detail_page_names_the_failed_subject_when_the_error_matches_a_known_pattern() {
    let backend = backend_with_schema().await;
    seed_job(
        &backend,
        "default",
        "job-a",
        failed_job_naming_a_subject("job-1"),
    )
    .await;
    let app = app(&backend, StaticConformanceSource::empty());

    let response = app.oneshot(get("/ui/sql/export/job-a")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(html.contains("The export stopped on subject"));
    assert!(html.contains("<strong>v03_counts</strong>"));
    assert!(html.contains("does not exist"));
    assert!(html.contains(r#"action="/ui/sql/export/job-a/retry""#));
    assert!(html.contains(r#"<button type="submit" class="btn btn--primary">"#));
    // A failed job never has files: the empty-state row, and a 0 count.
    assert!(html.contains(">0<"));
    assert!(html.contains(r#"<tr class="data-table__empty">"#));
}

#[tokio::test]
async fn detail_page_falls_back_to_the_generic_notice_when_the_error_matches_no_pattern() {
    let backend = backend_with_schema().await;
    seed_job(
        &backend,
        "default",
        "job-a",
        failed_job_with_unmatched_error("job-1"),
    )
    .await;
    let app = app(&backend, StaticConformanceSource::empty());

    let response = app.oneshot(get("/ui/sql/export/job-a")).await.unwrap();
    let html = body_text(response).await;

    assert!(html.contains("The export failed:"));
    assert!(html.contains("connection refused"));
    assert!(!html.contains("stopped on subject"));
}

#[tokio::test]
async fn detail_page_shows_the_reaper_reason_on_a_cancelled_job() {
    let backend = backend_with_schema().await;
    seed_job(
        &backend,
        "default",
        "job-a",
        cancelled_job_with_reason("job-1"),
    )
    .await;
    let app = app(&backend, StaticConformanceSource::empty());

    let response = app.oneshot(get("/ui/sql/export/job-a")).await.unwrap();
    let html = body_text(response).await;

    assert!(
        html.contains("Cancelled 2026-01-01 09:05 UTC · CSV · the server no longer knows this job")
    );
    assert!(html.contains(">Cancelled<"));
    assert!(html.contains(r#"action="/ui/sql/export/job-a/rerun""#));
    assert!(html.contains(r#"action="/ui/sql/export/job-a/remove""#));
}

/// The page's own render polls an in-progress job exactly once and persists
/// it, same as the list; the `/detail` fragment is the endpoint htmx's own
/// 5s refresh calls back into, and its poll is what eventually
/// carries the job to a terminal state and stops the polling.
#[tokio::test]
async fn detail_page_shows_progress_in_progress_and_the_fragment_completes_it() {
    let backend = backend_with_schema().await;
    seed_job(&backend, "default", "job-a", in_progress_job("job-1")).await;

    let running_app = app(
        &backend,
        StaticConformanceSource::empty()
            .with_export_status(SqlExportStatus::Running(Some("40%".to_string()))),
    );
    let response = running_app
        .oneshot(get("/ui/sql/export/job-a"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(">In progress<"));
    assert!(html.contains(r#"aria-valuenow="40""#));
    assert!(html.contains(r#"hx-get="/ui/sql/export/job-a/detail""#));
    assert!(html.contains("every 5s"));
    assert!(html.contains(r#"action="/ui/sql/export/job-a/cancel""#));
    assert!(html.contains(r#"<details class="menu" hidden>"#));

    let manifest = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "output", "part": [
            {"name": "name", "valueString": "patients"},
            {"name": "location", "valueUri": "http://s/export/job-1/patients-0.csv"},
        ]}]
    });
    let done_app = app(
        &backend,
        StaticConformanceSource::empty()
            .with_export_status(SqlExportStatus::Done)
            .with_export_manifest(Ok(manifest)),
    );
    let response = done_app
        .oneshot(get("/ui/sql/export/job-a/detail"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(">Complete<"));
    assert!(
        !html.contains("hx-get"),
        "a terminal fragment must not poll"
    );

    let stored = backend.get_settings("l2:").await.unwrap().unwrap();
    assert_eq!(
        stored.document["byTenant"]["default"]["sqlExport"]["jobs"]["job-a"]["status"],
        "complete"
    );
}

#[tokio::test]
async fn detail_page_shows_an_em_dash_and_no_copy_button_without_a_kickoff_job_id() {
    let backend = backend_with_schema().await;
    seed_job(&backend, "default", "job-a", kickoff_failed_job()).await;
    let app = app(&backend, StaticConformanceSource::empty());

    let response = app.oneshot(get("/ui/sql/export/job-a")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(">Failed<"));
    assert!(!html.contains("data-copy-job-id"));
    assert!(html.contains("<div>—</div>"));
}

/// An unknown id, another user's, and another tenant's are all a `404`
/// inside the full shell — never distinguished from each other, and never
/// leaking the id or the reason.
#[tokio::test]
async fn sql_export_detail_page_isolates_jobs_by_tenant_and_user_with_a_404_inside_the_shell() {
    let backend = backend_with_schema().await;
    seed_job(
        &backend,
        "other-tenant",
        "other-tenant-job",
        complete_job("job-x"),
    )
    .await;
    seed_job_for_user(
        &backend,
        "u2:4:test:owner",
        "default",
        "owners-job",
        complete_job("job-y"),
    )
    .await;
    let app = app(&backend, StaticConformanceSource::empty());

    for path in [
        "/ui/sql/export/does-not-exist",
        "/ui/sql/export/other-tenant-job",
        "/ui/sql/export/owners-job",
    ] {
        let response = app.clone().oneshot(get(path)).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        let html = body_text(response).await;
        assert!(html.contains(">Not found<"), "{path}");
        // The full shell renders — the sidebar nav is present.
        assert!(html.contains(r#"class="nav">"#), "{path}");
        assert!(
            html.contains(r#"<a class="btn" href="/ui/sql/export">SQL Exports</a>"#),
            "{path}"
        );
    }

    // The fragment endpoint answers a bare 404, no body, matching `/card`.
    let response = app
        .clone()
        .oneshot(get("/ui/sql/export/owners-job/detail"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// `/ui/sql/export/new` is a literal segment: it must keep serving the
/// builder rather than being captured by the `{id}` detail route.
#[tokio::test]
async fn new_route_still_serves_the_builder_not_the_detail_route() {
    let backend = backend_with_schema().await;
    let app = app(&backend, StaticConformanceSource::empty());

    let response = app.oneshot(get("/ui/sql/export/new")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(!html.contains("Not found"));
}

/// The contextual action and the overflow's items follow the status
/// exactly, and Copy job id is always server-rendered `hidden` — an
/// in-progress card's overflow, which would otherwise hold nothing but
/// that hidden button, starts hidden itself.
#[tokio::test]
async fn card_shows_the_right_contextual_action_and_overflow_items_per_status() {
    async fn card_html(seed: serde_json::Value) -> String {
        let backend = backend_with_schema().await;
        seed_job(&backend, "default", "job-a", seed).await;
        let source = StaticConformanceSource::empty()
            .with_export_status(SqlExportStatus::Running(Some("10%".to_string())));
        let app = app(&backend, source);
        let response = app.oneshot(get("/ui/sql/export/job-a/card")).await.unwrap();
        body_text(response).await
    }

    // in-progress: Cancel contextual, overflow is Copy job id only, and the
    // whole overflow starts hidden (no other item would work without JS).
    let html = card_html(in_progress_job("job-1")).await;
    assert!(html.contains(r#"action="/ui/sql/export/job-a/cancel""#));
    assert!(!html.contains("/retry\""));
    assert!(!html.contains("/rerun\""));
    assert!(!html.contains("/remove\""));
    assert!(html.contains(r#"<details class="menu" hidden>"#));
    assert!(html.contains(r#"data-copy-job-id="job-1""#));

    // complete: View files contextual, overflow = Run again, Copy job id,
    // Remove from list, and the overflow itself is not hidden.
    let html = card_html(complete_job("job-1")).await;
    // #835: the card's own title also links to the detail page.
    assert!(html.contains(r#"<h2 class="job-card__name"><a href="/ui/sql/export/job-a">"#));
    assert!(html.contains(r#"href="/ui/sql/export/job-a""#));
    assert!(html.contains(r#"action="/ui/sql/export/job-a/rerun""#));
    assert!(html.contains(r#"action="/ui/sql/export/job-a/remove""#));
    assert!(html.contains(r#"data-copy-job-id="job-1""#));
    assert!(!html.contains(r#"<details class="menu" hidden>"#));

    // failed: Retry contextual, overflow = Copy job id, Remove from list —
    // no Run again.
    let html = card_html(failed_job("job-1")).await;
    assert!(html.contains(r#"action="/ui/sql/export/job-a/retry""#));
    assert!(html.contains(r#"action="/ui/sql/export/job-a/remove""#));
    assert!(html.contains(r#"data-copy-job-id="job-1""#));
    assert!(!html.contains(r#"action="/ui/sql/export/job-a/rerun""#));
    assert!(!html.contains(r#"action="/ui/sql/export/job-a/cancel""#));

    // cancelled: no contextual action, overflow = Run again, Copy job id,
    // Remove from list.
    let html = card_html(cancelled_job("job-1")).await;
    assert!(!html.contains(r#"action="/ui/sql/export/job-a/cancel""#));
    assert!(!html.contains(r#"action="/ui/sql/export/job-a/retry""#));
    assert!(!html.contains("href=\"/ui/sql/files"));
    assert!(html.contains(r#"action="/ui/sql/export/job-a/rerun""#));
    assert!(html.contains(r#"action="/ui/sql/export/job-a/remove""#));
    assert!(html.contains(r#"data-copy-job-id="job-1""#));
}
