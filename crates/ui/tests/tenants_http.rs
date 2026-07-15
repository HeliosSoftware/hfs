//! End-to-end tests for the tenant-maintenance page (`/ui/tenants`), driving the
//! mounted router against a real in-memory SQLite-backed store so the handlers,
//! the registry read/write path, and result shaping are all exercised.

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// An in-memory SQLite store with the schema initialised, as an
/// `Arc<dyn ResourceStorage>` ready to hand to `mount`.
fn store() -> Arc<dyn ResourceStorage> {
    let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
    backend.init_schema().expect("init schema");
    Arc::new(backend)
}

/// Builds the UI router over the given store. A fresh router per request is
/// fine — they all share the same backend `Arc`, so state persists.
fn app(store: &Arc<dyn ResourceStorage>) -> Router {
    helios_ui::mount(Router::new(), "9.9.9", Some(Arc::clone(store)))
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn get(store: &Arc<dyn ResourceStorage>, uri: &str) -> (StatusCode, String) {
    let res = app(store)
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    (status, body_text(res).await)
}

/// POSTs an `application/x-www-form-urlencoded` body to `/ui/tenants`.
async fn post_form(store: &Arc<dyn ResourceStorage>, form: &str) -> (StatusCode, String) {
    let res = app(store)
        .oneshot(
            Request::post("/ui/tenants")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(form.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    (status, body_text(res).await)
}

#[tokio::test]
async fn page_renders_and_lists_registered_and_discovered_tenants() {
    let store = store();

    // A registered tenant (via the page's own form) ...
    let (status, _) = post_form(&store, "id=acme-health&display_name=Acme+Health").await;
    assert_eq!(status, StatusCode::OK);

    // ... and a data-only tenant, seeded straight through the backend.
    let northwind =
        TenantContext::new(TenantId::new("northwind"), TenantPermissions::full_access());
    store
        .create(
            &northwind,
            "Patient",
            serde_json::json!({}),
            FhirVersion::R4,
        )
        .await
        .unwrap();

    let (status, html) = get(&store, "/ui/tenants").await;
    assert_eq!(status, StatusCode::OK);
    // Registered tenant: display name + id slug + a creation date.
    assert!(html.contains("Acme Health"));
    assert!(html.contains("acme-health"));
    // Discovered tenant: shown, flagged unregistered.
    assert!(html.contains("northwind"));
    assert!(html.contains("unregistered"));
    // Stat cards.
    assert!(html.contains("Tenant Maintenance"));
}

#[tokio::test]
async fn rows_fragment_filters_by_search_term() {
    let store = store();
    post_form(&store, "id=acme-health&display_name=Acme+Health").await;
    post_form(
        &store,
        "id=riverside-labs&display_name=Riverside+Diagnostics",
    )
    .await;

    // Unfiltered: both present, and it's a fragment (no full document).
    let (_, all) = get(&store, "/ui/tenants/rows").await;
    assert!(all.contains("Acme Health"));
    assert!(all.contains("Riverside Diagnostics"));
    assert!(!all.contains("<html"));

    // Filtered by name.
    let (_, filtered) = get(&store, "/ui/tenants/rows?q=river").await;
    assert!(filtered.contains("Riverside Diagnostics"));
    assert!(!filtered.contains("Acme Health"));
}

#[tokio::test]
async fn create_rejects_invalid_id_and_conflicts() {
    let store = store();

    // Invalid id → the fragment carries an error banner, not a new row.
    let (_, bad) = post_form(&store, "id=has%20space").await;
    assert!(bad.contains("form-error"));

    // Valid create, then a duplicate → conflict surfaced as a banner.
    post_form(&store, "id=acme").await;
    let (_, dup) = post_form(&store, "id=acme").await;
    assert!(dup.contains("form-error"));
    assert!(dup.contains("already exists"));
}

#[tokio::test]
async fn delete_deregisters_a_tenant() {
    let store = store();
    post_form(&store, "id=acme&display_name=Acme").await;

    let res = app(&store)
        .oneshot(
            Request::delete("/ui/tenants/acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Gone from the registry (no data seeded, so it disappears entirely).
    let (_, rows) = get(&store, "/ui/tenants/rows").await;
    assert!(!rows.contains(">acme<") && !rows.contains("Acme"));
}

#[tokio::test]
async fn page_reports_registry_unavailable_without_a_store() {
    // No storage handle → the page renders the "unavailable" notice, not a crash.
    let res = helios_ui::mount(Router::new(), "9.9.9", None)
        .oneshot(Request::get("/ui/tenants").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let html = body_text(res).await;
    assert!(html.contains("not available"));
}

#[tokio::test]
async fn embedded_assets_carry_no_cache_so_rebuilt_css_is_never_stale() {
    let store = store();
    let res = app(&store)
        .oneshot(
            Request::get("/ui/assets/app.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cc = res
        .headers()
        .get(header::CACHE_CONTROL)
        .expect("assets carry Cache-Control")
        .to_str()
        .unwrap();
    assert!(cc.contains("no-cache"), "got {cc}");
}
