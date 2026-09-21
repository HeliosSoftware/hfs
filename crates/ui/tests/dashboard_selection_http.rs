//! Integration coverage for the Home chart selection persistence (#1358):
//! an explicit selection is written to the settings document's `dashboard`
//! key (tenant-scoped), a bare `/ui` visit restores it, and neither the
//! `dash-live` polling refresh nor a bare visit writes anything. Uses the
//! shared [`support::InMemorySettingsStore`] double.
//!
//! The pure `DashboardSelection` sanitization and `persist_dashboard`/
//! `RequestSettings::dashboard` logic are `pub(crate)` and unit-tested inside
//! `helios_ui::rail_state`; this crate can only reach the mounted router.

mod support;

use axum::{
    Router,
    body::Body,
    http::{Request, header::HeaderName},
    response::Response,
};
use helios_persistence::core::SettingsStore;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

use support::InMemorySettingsStore;

fn nl() -> helios_ui::NlSearch {
    helios_ui::NlSearch {
        enabled: false,
        configured: false,
        model: String::new(),
    }
}

fn app_with(settings: Arc<InMemorySettingsStore>) -> Router {
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl(),
        None,
        Some(settings),
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    )
}

async fn body_text(response: Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .replace("\r\n", "\n")
}

/// A plain full-page GET.
async fn get(app: Router, path: &str) -> Response {
    app.oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// A GET carrying the htmx headers for the named region target.
async fn get_htmx(app: Router, path: &str, target: &str) -> Response {
    app.oneshot(
        Request::builder()
            .uri(path)
            .header(HeaderName::from_static("hx-request"), "true")
            .header(HeaderName::from_static("hx-target"), target)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
}

/// The stored `dashboard` value for the default tenant, or `None` when the
/// route wrote nothing under that key.
fn stored_dashboard(store: &InMemorySettingsStore) -> Option<Value> {
    let doc = store.peek("l2:")?;
    doc.get("byTenant")?
        .get("default")?
        .get("dashboard")
        .cloned()
}

/// An explicit full-page selection is persisted under the tenant-scoped
/// `dashboard` key — the curated types and window slug. The transient "view
/// all" flag is deliberately not persisted, even when the request carries it.
#[tokio::test]
async fn an_explicit_full_page_selection_is_persisted() {
    let store = Arc::new(InMemorySettingsStore::new());
    let resp = get(
        app_with(store.clone()),
        "/ui?types=Patient,Observation&window=24h&all=1",
    )
    .await;
    assert!(resp.status().is_success());

    assert_eq!(
        stored_dashboard(&store),
        Some(json!({"types": ["Patient", "Observation"], "window": "24h"})),
    );
}

/// The `dash-live` polling refresh must not write settings, even though its
/// poll URL carries the current selection — otherwise every refresh tick
/// would hit the store.
#[tokio::test]
async fn the_live_refresh_does_not_persist() {
    let store = Arc::new(InMemorySettingsStore::new());
    let resp = get_htmx(
        app_with(store.clone()),
        "/ui?types=Patient&window=24h",
        "dash-live",
    )
    .await;
    assert!(resp.status().is_success() || resp.status() == axum::http::StatusCode::NO_CONTENT);

    assert_eq!(
        stored_dashboard(&store),
        None,
        "a dash-live refresh must not write the dashboard selection"
    );
}

/// A bare `/ui` visit carries no selection, so it must not overwrite a stored
/// one — it is the restore path, not a selection.
#[tokio::test]
async fn a_bare_visit_does_not_persist() {
    let store = Arc::new(InMemorySettingsStore::new());
    let resp = get(app_with(store.clone()), "/ui").await;
    assert!(resp.status().is_success());

    assert_eq!(
        stored_dashboard(&store),
        None,
        "a bare /ui visit must not write the dashboard selection"
    );
    // The restore reads the same settings document `resolve_prefs` already
    // fetched — exactly one read, no extra store access for the fallback.
    assert_eq!(store.get_settings_calls(), 1);
}

/// A bare `/ui` visit restores the stored selection: the window slug the user
/// last chose (`1h`) drives the render, not the `30d` default.
#[tokio::test]
async fn a_bare_visit_restores_the_stored_window() {
    let store = Arc::new(InMemorySettingsStore::new());
    store
        .patch_settings(
            "l2:",
            json!({"byTenant": {"default": {"dashboard": {"types": ["Patient"], "window": "1h"}}}}),
            None,
        )
        .await
        .unwrap();

    let body = body_text(get(app_with(store.clone()), "/ui").await).await;
    // The window picker marks exactly one option active. Isolate that option's
    // markup (from the active class to its closing tag) and confirm it is the
    // 1h one, so this asserts the *restored* window, not merely that 1h appears
    // as one of the always-present selector links.
    let active = body
        .split("window-picker__option--active")
        .nth(1)
        .and_then(|rest| rest.split("</a>").next())
        .expect("an active window option should be rendered");
    assert!(
        active.contains("window=1h"),
        "the active window should be the restored 1h, got: {active}"
    );
}
