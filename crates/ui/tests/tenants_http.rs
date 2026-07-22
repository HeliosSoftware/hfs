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
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        None,
        helios_ui::NlSearch::default(),
        Some(Arc::clone(store)),
        None,
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::empty()),
        FhirVersion::R4,
    )
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

    // Provisioning a tenant seeds it with conformance resources (the embedded
    // fallback SearchParameters at minimum), so a plain deregister leaves it
    // data-discovered. Purge to remove the tenant entirely.
    let res = app(&store)
        .oneshot(
            Request::delete("/ui/tenants/acme?purge=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Gone from the registry and its seeded data purged, so it disappears.
    let (_, rows) = get(&store, "/ui/tenants/rows").await;
    assert!(!rows.contains(">acme<") && !rows.contains("Acme"));
}

#[tokio::test]
async fn page_reports_registry_unavailable_without_a_store() {
    // No storage handle → the page renders the "unavailable" notice, not a crash.
    let res = helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        None,
        helios_ui::NlSearch::default(),
        None,
        None,
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::empty()),
        FhirVersion::R4,
    )
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

#[tokio::test]
async fn version_choice_persists_to_user_settings_and_redirects_back() {
    use helios_persistence::core::SettingsStore;

    let backend = Arc::new({
        let b = SqliteBackend::in_memory().expect("in-memory sqlite");
        b.init_schema().expect("init schema");
        b
    });
    let app = helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        None,
        helios_ui::NlSearch::default(),
        None,
        Some(backend.clone() as Arc<dyn SettingsStore>),
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::empty()),
        FhirVersion::R4,
    );

    // Selecting a version persists it and bounces back to the referring page.
    let res = app
        .clone()
        .oneshot(
            Request::post("/ui/version")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::REFERER, "http://localhost/ui/search-parameters")
                .body(Body::from("version=R4"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        res.headers().get(header::LOCATION).unwrap(),
        "/ui/search-parameters"
    );
    let stored = backend
        .get_settings("l2:")
        .await
        .expect("settings read")
        .expect("document stored");
    assert_eq!(stored.document["fhirVersion"], "R4");

    // A version this build does not carry is rejected, not stored.
    let res = app
        .oneshot(
            Request::post("/ui/version")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("version=R9"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn tenant_choice_persists_and_the_selector_follows_it() {
    use helios_persistence::core::SettingsStore;

    let backend = Arc::new({
        let b = SqliteBackend::in_memory().expect("in-memory sqlite");
        b.init_schema().expect("init schema");
        b
    });
    backend
        .register_tenant("acme", Some("Acme Health"))
        .await
        .expect("register tenant");
    let app = || {
        helios_ui::mount_with_conformance_source(
            Router::new(),
            "9.9.9",
            None,
            helios_ui::NlSearch::default(),
            Some(backend.clone() as Arc<dyn ResourceStorage>),
            Some(backend.clone() as Arc<dyn SettingsStore>),
            "default".to_string(),
            Arc::new(helios_ui::StaticConformanceSource::empty()),
            FhirVersion::R4,
        )
    };

    // The options fragment lists the registered tenant, with the effective
    // (default) tenant present and marked current.
    let res = app()
        .oneshot(
            Request::get("/ui/tenant/options")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let html = body_text(res).await;
    assert!(html.contains(r#"name="tenant" value="acme""#));
    assert!(html.contains("Acme Health"));
    assert!(html.contains(r#"aria-current="true""#));
    assert!(!html.contains("<html"), "fragment, not a page");

    // Choosing a provisioned tenant persists it and bounces back.
    let res = app()
        .oneshot(
            Request::post("/ui/tenant")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::REFERER, "http://localhost/ui/queries")
                .body(Body::from("tenant=acme"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(res.headers().get(header::LOCATION).unwrap(), "/ui/queries");
    let stored = backend
        .get_settings("l2:")
        .await
        .expect("settings read")
        .expect("document stored");
    assert_eq!(stored.document["tenantId"], "acme");

    // The stored choice now drives the selector label on every page.
    let res = app()
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html = body_text(res).await;
    assert!(
        html.contains("Acme Health"),
        "label follows the stored tenant"
    );
    assert!(
        html.contains(r#"content="acme""#),
        "meta tag carries the id"
    );

    // An unprovisioned tenant is rejected, not stored.
    let res = app()
        .oneshot(
            Request::post("/ui/tenant")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("tenant=made-up"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
