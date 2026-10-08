//! End-to-end tests for the tenant-maintenance page (`/ui/tenants`), driving the
//! mounted router against a real in-memory SQLite-backed store so the handlers,
//! the registry read/write path, and result shaping are all exercised.
//!
//! Provisioning runs in the background (#581), so a single router (and the
//! `WebState::provisioning` registry it owns) must be built once per test and
//! reused across every request in that test — a fresh router per request
//! would carry a fresh, empty provisioning registry each time, breaking the
//! continuity the tests below rely on (an in-flight claim made by one POST
//! would be invisible to the next). `Router` is cheap to `clone()` (it shares
//! its state behind an `Arc`), so each request clones the one router built at
//! the top of the test.
//!
//! Resource counts and data-only tenants arrive in the background (#1851):
//! no response awaits them. Tests that need them wait for the count status
//! to settle ([`wait_counts_settled`]); tests about that deferral hold the
//! cross-tenant scan behind [`GatedStorage`]'s gate and prove each response
//! returns while it is held, without measuring time.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::{ErasedScope, ResourceStorage, WriteEvent, WriteObserver};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[path = "support/gated_storage.rs"]
mod gated_storage;
#[path = "support/html.rs"]
mod html;

use gated_storage::GatedStorage;
use html::Dom;

/// Bounds every wait on a response, so a request that blocks on the held
/// count fails the test instead of hanging it. Never a latency threshold.
const HANG_GUARD: Duration = Duration::from_secs(10);

/// An in-memory SQLite store with the schema initialised, as an
/// `Arc<dyn ResourceStorage>` ready to hand to `mount`.
fn store() -> Arc<dyn ResourceStorage> {
    let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
    backend.init_schema().expect("init schema");
    Arc::new(backend)
}

/// Builds the UI router over the given store. Build this once per test and
/// reuse (clone) it for every request — see the module doc.
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
        None,
        "http://localhost:8080".to_string(),
        None,
    )
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn get(router: &Router, uri: &str) -> (StatusCode, String) {
    let res = router
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    (status, body_text(res).await)
}

/// Like `post_form` but hands back the raw response (headers included).
async fn post_form_raw(router: &Router, form: &str) -> axum::response::Response {
    router
        .clone()
        .oneshot(
            Request::post("/ui/tenants")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(form.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

/// POSTs an `application/x-www-form-urlencoded` body to `/ui/tenants`.
async fn post_form(router: &Router, form: &str) -> (StatusCode, String) {
    let res = post_form_raw(router, form).await;
    let status = res.status();
    (status, body_text(res).await)
}

/// Polls the rows fragment until no provisioning row remains (every
/// background job settled, one way or another), or panics after ~10s. The
/// marker is the real in-flight row class (`class="busy-status"`, the
/// shared busy region since #679), not an invented one. Counts still on
/// their way do not count: they have their own marker (#1851), see
/// [`wait_counts_settled`].
async fn wait_settled(router: &Router) -> String {
    for _ in 0..200 {
        let (_, html) = get(router, "/ui/tenants/rows").await;
        if !html.contains(r#"class="busy-status""#) {
            return html;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("provisioning did not settle in time");
}

/// The count status of a page or fragment (`data-counts-state`, #1851).
fn counts_state(html: &str) -> String {
    let dom = Dom::page(html);
    dom.one("#tenant-counts-status [data-counts-state]")
        .attr("data-counts-state")
        .expect("the status carries its state")
        .to_string()
}

/// Polls `uri` until the counts are no longer on their way (any state but
/// `pending` and `refreshing`), the way the page's own poller would, and
/// returns that response; panics after ~10s.
async fn wait_counts_settled(router: &Router, uri: &str) -> String {
    for _ in 0..200 {
        let (_, html) = get(router, uri).await;
        if !matches!(counts_state(&html).as_str(), "pending" | "refreshing") {
            return html;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the counts did not settle in time");
}

/// Awaits a response that must not wait for the held count.
async fn within<T>(what: &str, response: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(HANG_GUARD, response)
        .await
        .unwrap_or_else(|_| panic!("{what} waited for the held count"))
}

/// The `data-count-state` of `id`'s resources cell, if `id` has a row.
fn cell_state(html: &str, id: &str) -> Option<String> {
    let dom = Dom::page(html);
    dom.all("tbody tr")
        .into_iter()
        .find(|row| row.all(".tenant-id__slug").iter().any(|s| s.text() == id))
        .map(|row| {
            row.one("td.col-num")
                .attr("data-count-state")
                .unwrap_or("none")
                .to_string()
        })
}

/// The row ids a page or fragment lists.
fn row_ids(html: &str) -> Vec<String> {
    Dom::page(html)
        .all(".tenant-id__slug")
        .iter()
        .map(|slug| slug.text())
        .collect()
}

/// A tenant context with full access, for seeding data.
fn ctx(tenant: &str) -> TenantContext {
    TenantContext::new(TenantId::new(tenant), TenantPermissions::full_access())
}

/// Stores one Patient for `tenant`, straight through the backend.
async fn seed_patient(store: &Arc<dyn ResourceStorage>, tenant: &str) {
    store
        .create(
            &ctx(tenant),
            "Patient",
            serde_json::json!({"resourceType": "Patient"}),
            FhirVersion::R4,
        )
        .await
        .expect("seed a patient");
}

/// An in-memory SQLite store wrapped in [`GatedStorage`], and the UI router
/// over the wrapper. Seed through `inner` or the wrapper alike.
fn gated() -> (Arc<GatedStorage>, Arc<dyn ResourceStorage>, Router) {
    let inner = store();
    let gated = GatedStorage::new(Arc::clone(&inner));
    let router = app(&(gated.clone() as Arc<dyn ResourceStorage>));
    (gated, inner, router)
}

async fn delete_uri(router: &Router, uri: &str) -> (StatusCode, String) {
    let res = router
        .clone()
        .oneshot(Request::delete(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    (status, body_text(res).await)
}

#[tokio::test]
async fn page_renders_and_lists_registered_and_discovered_tenants() {
    let store = store();
    let router = app(&store);

    // A data-only tenant, seeded straight through the backend ...
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

    // ... and a registered tenant (via the page's own form).
    let (status, _) = post_form(&router, "id=acme-health&display_name=Acme+Health").await;
    assert_eq!(status, StatusCode::OK);
    wait_settled(&router).await;

    // Data-only tenants come from the background inventory (#1851): wait
    // for the counts the way the page's poller does, then reload.
    wait_counts_settled(&router, "/ui/tenants/rows").await;
    let (status, html) = get(&router, "/ui/tenants").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(counts_state(&html), "ready");
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
    let router = app(&store);
    post_form(&router, "id=acme-health&display_name=Acme+Health").await;
    post_form(
        &router,
        "id=riverside-labs&display_name=Riverside+Diagnostics",
    )
    .await;
    wait_settled(&router).await;

    // Unfiltered: both present, and it's a fragment (no full document).
    let (_, all) = get(&router, "/ui/tenants/rows").await;
    assert!(all.contains("Acme Health"));
    assert!(all.contains("Riverside Diagnostics"));
    assert!(!all.contains("<html"));

    // Filtered by name.
    let (_, filtered) = get(&router, "/ui/tenants/rows?q=river").await;
    assert!(filtered.contains("Riverside Diagnostics"));
    assert!(!filtered.contains("Acme Health"));
}

#[tokio::test]
async fn create_rejects_invalid_id_and_conflicts() {
    let store = store();
    let router = app(&store);

    // Invalid id → the fragment carries an error banner, not a new row.
    let (_, bad) = post_form(&router, "id=has%20space").await;
    assert!(bad.contains("alert"));

    // Valid create, settle, then a duplicate → conflict surfaced as a banner.
    post_form(&router, "id=acme").await;
    wait_settled(&router).await;
    let (_, dup) = post_form(&router, "id=acme").await;
    assert!(dup.contains("alert"));
    assert!(dup.contains("already exists"));
}

#[tokio::test]
async fn delete_deregisters_a_tenant() {
    let store = store();
    let router = app(&store);
    post_form(&router, "id=acme&display_name=Acme").await;
    wait_settled(&router).await;

    // Provisioning a tenant seeds it with conformance resources (the embedded
    // fallback SearchParameters at minimum), so a plain deregister leaves it
    // data-discovered. Purge to remove the tenant entirely.
    let res = router
        .clone()
        .oneshot(
            Request::delete("/ui/tenants/acme?purge=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Gone from the registry and its seeded data purged, so it disappears.
    let (_, rows) = get(&router, "/ui/tenants/rows").await;
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
        None,
        "http://localhost:8080".to_string(),
        None,
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

/// A storage failure must surface as the error banner, never as an empty
/// 'no tenants' table that hides every row and purge affordance. A store
/// whose schema was never initialised makes every query fail, standing in
/// for a transient error (e.g. a pool wait under concurrent seeding).
#[tokio::test]
async fn storage_failure_renders_the_error_banner_not_a_silent_empty_table() {
    let broken: Arc<dyn ResourceStorage> =
        Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite"));
    let router = app(&broken);

    let (status, body) = get(&router, "/ui/tenants/rows?q=").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("alert"), "rows fragment: {body}");

    let (status, body) = post_form(&router, "id=acme&display_name=").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("alert"), "create fragment: {body}");

    let res = router
        .clone()
        .oneshot(
            Request::delete("/ui/tenants/acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_text(res).await;
    assert!(body.contains("alert"), "delete fragment: {body}");
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
        None,
        "http://localhost:8080".to_string(),
        None,
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

/// #562: the resource-type lists follow the sidebar's version choice
/// end-to-end — selecting a version through `/ui/version` changes which
/// compartment bundle the pickers enumerate. Ingredient exists only in R4B;
/// EffectEvidenceSynthesis only in R4. Gated on the R4B feature (CI runs
/// `--all-features`); a single-version build has nothing to flip.
#[cfg(feature = "R4B")]
#[tokio::test]
async fn version_choice_changes_the_resource_type_lists() {
    use helios_persistence::core::SettingsStore;

    let backend = Arc::new({
        let b = SqliteBackend::in_memory().expect("in-memory sqlite");
        b.init_schema().expect("init schema");
        b
    });
    let app = || {
        helios_ui::mount_with_conformance_source(
            Router::new(),
            "9.9.9",
            Some(std::path::PathBuf::from("../../data")),
            helios_ui::NlSearch::default(),
            None,
            Some(backend.clone() as Arc<dyn SettingsStore>),
            "default".to_string(),
            Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
                std::path::Path::new("../../data"),
            )),
            FhirVersion::R4,
            None,
            "http://localhost:8080".to_string(),
            None,
        )
    };

    // Default version (R4): the Resources page's type rail carries the R4 set.
    let res = app()
        .oneshot(Request::get("/ui/resources").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = body_text(res).await;
    // Anchored to the rail markup: bare `Ingredient` would also match R4's
    // MedicinalProductIngredient.
    assert!(body.contains(r#"data-type="EffectEvidenceSynthesis""#));
    assert!(!body.contains(r#"data-type="Ingredient""#));

    // Select R4B through the same endpoint the sidebar posts to.
    let res = app()
        .oneshot(
            Request::post("/ui/version")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("version=R4B"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // The same page now enumerates the R4B set.
    let res = app()
        .oneshot(Request::get("/ui/resources").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = body_text(res).await;
    assert!(body.contains(r#"data-type="Ingredient""#));
    assert!(!body.contains(r#"data-type="EffectEvidenceSynthesis""#));
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
            None,
            "http://localhost:8080".to_string(),
            None,
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
                .header(header::REFERER, "http://localhost/ui/resources")
                .body(Body::from("tenant=acme"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        res.headers().get(header::LOCATION).unwrap(),
        "/ui/resources"
    );
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

/// This route is destructive and — unlike `/admin/tenants` — is mounted outside
/// the auth layer, so before #317 an unauthenticated
/// `DELETE /ui/tenants/__system__?purge=true` wiped the AuditEvent trail and the
/// shared terminology. It now refuses reserved ids.
///
/// This closes the reserved-id hole only; whether `/ui/*` should be
/// authenticated at all is a separate, deliberately untouched question.
#[tokio::test]
async fn delete_refuses_the_reserved_system_tenant() {
    let store = store();

    // Seed the shared tenant the way the database audit sink does.
    let system = TenantContext::system();
    let created = store
        .create(
            &system,
            "AuditEvent",
            serde_json::json!({ "resourceType": "AuditEvent" }),
            FhirVersion::R4,
        )
        .await
        .expect("seed system-tenant AuditEvent");

    let res = app(&store)
        .oneshot(
            Request::delete("/ui/tenants/__system__?purge=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // The page reports the refusal in its error banner rather than 500-ing.
    let body = body_text(res).await;
    assert!(
        body.contains("alert"),
        "the refusal must surface as an error banner, got: {body}"
    );

    // The data consequence: nothing was purged.
    let survivor = store
        .read(&system, "AuditEvent", created.id())
        .await
        .expect("read must not error");
    assert!(
        survivor.is_some(),
        "a refused purge must leave the audit trail intact"
    );
}

/// The reservation is exact, so an ordinary `__`-prefixed tenant is still fully
/// manageable from this page.
#[tokio::test]
async fn delete_still_accepts_underscore_prefixed_tenants() {
    let store = store();
    let router = app(&store);
    post_form(&router, "id=__legacy").await;
    wait_settled(&router).await;

    let res = router
        .clone()
        .oneshot(
            Request::delete("/ui/tenants/__legacy?purge=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

/// #544: the sidebar tenant picker only renders once a tenant beyond the
/// server default exists.
#[tokio::test]
async fn the_tenant_picker_hides_until_a_second_tenant_exists() {
    let store = store();
    let router = app(&store);

    // Fresh install: default tenant only — no picker in the chrome.
    let (_, html) = get(&router, "/ui/tenants").await;
    assert!(
        !html.contains(r#"class="selector""#),
        "picker hidden on a single-tenant install"
    );

    // Provision a second tenant through the page's own form, and let the
    // background registration finish — the picker reads the real registry
    // (`list_tenants`), not the in-flight job notice.
    let res = router
        .clone()
        .oneshot(
            Request::post("/ui/tenants")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("id=acme&display_name=Acme"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(res.status().is_success());
    wait_settled(&router).await;

    let (_, html) = get(&router, "/ui/tenants").await;
    assert!(
        html.contains(r#"class="selector""#),
        "picker appears once a second tenant exists"
    );
}

/// #545: the Add-tenant panel carries explicit ways out.
#[tokio::test]
async fn the_add_tenant_panel_offers_cancel_and_close() {
    let store = store();
    let router = app(&store);
    let (_, html) = get(&router, "/ui/tenants").await;
    assert!(html.contains("data-addbox-close"), "close controls present");
    assert!(html.contains("Cancel"));
    assert!(html.contains("/ui/assets/tenants.js"));
}

/// #581: the create request answers as soon as it is accepted, so the form
/// only needs to guard against a double submit — there is no long-running
/// pending state on the form itself any more (the table's spinner row covers
/// that, see `create_signals_success_with_hx_trigger_and_failures_do_not`).
#[tokio::test]
async fn the_add_tenant_form_disables_submit_while_posting() {
    let store = store();
    let router = app(&store);
    let (_, html) = get(&router, "/ui/tenants").await;
    assert!(html.contains(r#"hx-disabled-elt="find button[type=submit]""#));
    assert!(!html.contains("hx-indicator"));
    assert!(!html.contains(r#"id="tenant-add-pending""#));
}

/// #582: the display name is mirrored into the tenant id client-side, so the
/// form renders the display name field first and tags both inputs with
/// `data-*` attributes the script can find without depending on `name`.
#[tokio::test]
async fn the_add_tenant_form_puts_the_display_name_before_the_id_and_tags_both_for_the_slug_mirror()
{
    let store = store();
    let router = app(&store);
    let (_, html) = get(&router, "/ui/tenants").await;
    let name = html
        .find("data-tenant-name")
        .expect("display name input tagged");
    let id = html.find("data-tenant-id").expect("id input tagged");
    assert!(name < id, "display name renders above the tenant id");
    assert!(
        html.contains(r#"name="id" required"#),
        "id stays required without JS"
    );
}

/// #583/#581: a successful create answers immediately with `HX-Trigger:
/// tenant-created` and a rows fragment that already shows the tenant as an
/// in-flight (spinner) row; failures (duplicate id, invalid id, a duplicate
/// claim) never carry the header. Once the background job settles, the
/// duplicate check reports "already exists" instead of "already being
/// provisioned".
#[tokio::test]
async fn create_signals_success_with_hx_trigger_and_failures_do_not() {
    let store = store();
    let router = app(&store);

    let ok = post_form_raw(&router, "id=acme&display_name=Acme").await;
    assert_eq!(ok.status(), StatusCode::OK);
    assert_eq!(
        ok.headers().get("HX-Trigger").and_then(|v| v.to_str().ok()),
        Some("tenant-created")
    );
    let ok_body = body_text(ok).await;
    assert!(
        ok_body.contains(r#"class="busy-status""#),
        "the accepted response already shows the in-flight row: {ok_body}"
    );

    // An immediate second POST for the same id races the background job, but
    // never signals success either way.
    let dup = post_form_raw(&router, "id=acme").await;
    assert_eq!(dup.status(), StatusCode::OK);
    assert!(dup.headers().get("HX-Trigger").is_none());
    let dup_body = body_text(dup).await;
    assert!(
        dup_body.contains("already being provisioned") || dup_body.contains("already exists"),
        "got: {dup_body}"
    );

    // Once the job has settled, the id is definitely taken.
    wait_settled(&router).await;
    let settled_dup = post_form_raw(&router, "id=acme").await;
    assert!(settled_dup.headers().get("HX-Trigger").is_none());
    assert!(body_text(settled_dup).await.contains("already exists"));

    let bad = post_form_raw(&router, "id=has%20space").await;
    assert!(bad.headers().get("HX-Trigger").is_none());
}

/// #581: once the background job finishes, the spinner row is replaced by a
/// normal, deletable row — the provisioning marker is gone.
#[tokio::test]
async fn a_provisioned_tenant_settles_into_a_normal_row() {
    let store = store();
    let router = app(&store);

    post_form(&router, "id=acme&display_name=Acme").await;
    let html = wait_settled(&router).await;

    assert!(html.contains(r#"hx-delete="/ui/tenants/acme""#));
    assert!(!html.contains(r#"class="busy-status""#));
}

/// Records every write event the UI reports to its observer.
#[derive(Default)]
struct RecordingObserver(std::sync::Mutex<Vec<WriteEvent>>);

impl WriteObserver for RecordingObserver {
    fn on_write(&self, event: &WriteEvent) {
        self.0.lock().unwrap().push(event.clone());
    }
}

impl RecordingObserver {
    /// The tenant-level events reported since the first `from` events, as
    /// `(what, tenant)`: purges of a whole tenant and deregistrations.
    fn tenant_events(&self, from: usize) -> Vec<(&'static str, TenantId)> {
        self.0.lock().unwrap()[from..]
            .iter()
            .filter_map(|event| match event {
                WriteEvent::Erased { tenant, scope } => {
                    assert_eq!(
                        scope,
                        &ErasedScope::Tenant,
                        "a tenant purge erases the tenant"
                    );
                    Some(("erased", tenant.clone()))
                }
                WriteEvent::TenantRemoved { tenant } => Some(("removed", tenant.clone())),
                _ => None,
            })
            .collect()
    }

    fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

/// #1078: deleting a tenant tells the write observer the tenant is gone, so
/// the dashboard can drop what it holds for it — right after the purge's own
/// `Erased` event when the data was purged, and on its own when it was not.
#[tokio::test]
async fn delete_reports_the_removed_tenant_to_the_write_observer() {
    let store = store();
    let observer = Arc::new(RecordingObserver::default());
    let router = helios_ui::mount_with_conformance_source_and_runtime(
        Router::new(),
        "9.9.9",
        None,
        helios_ui::NlSearch::default(),
        Some(Arc::clone(&store)),
        None,
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::empty()),
        FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        10 * 1024 * 1024,
        false,
        None,
        "http://localhost:8080".to_string(),
        Arc::new(helios_auth::outbound::NoOpOutboundAuthProvider),
        helios_ui::PatientNameSearchSupport::Enabled,
        Some(observer.clone() as Arc<dyn WriteObserver>),
    );
    for form in ["id=acme&display_name=Acme", "id=beta&display_name=Beta"] {
        let (status, _) = post_form(&router, form).await;
        assert_eq!(status, StatusCode::OK);
        wait_settled(&router).await;
    }

    let delete = |uri: &'static str| {
        let router = router.clone();
        async move {
            router
                .oneshot(Request::delete(uri).body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status()
        }
    };

    let before = observer.len();
    assert_eq!(delete("/ui/tenants/acme?purge=true").await, StatusCode::OK);
    assert_eq!(
        observer.tenant_events(before),
        [
            ("erased", TenantId::new("acme")),
            ("removed", TenantId::new("acme")),
        ]
    );

    let before = observer.len();
    assert_eq!(delete("/ui/tenants/beta").await, StatusCode::OK);
    assert_eq!(
        observer.tenant_events(before),
        [("removed", TenantId::new("beta"))]
    );
}

/// #1850: a mounted UI subscribes its tenant inventory to the server's
/// write-observer fan-out, so purges made outside the UI (the
/// `/admin/tenants` API) invalidate it too. A plain observer that is not a
/// fan-out leaves nothing to subscribe to, and a UI without storage has no
/// inventory to subscribe.
#[test]
fn mount_subscribes_the_tenant_inventory_to_the_server_fan_out() {
    let mount = |tenants: Option<Arc<dyn ResourceStorage>>, observer: Arc<dyn WriteObserver>| {
        helios_ui::mount_with_conformance_source_and_runtime(
            Router::new(),
            "9.9.9",
            None,
            helios_ui::NlSearch::default(),
            tenants,
            None,
            "default".to_string(),
            Arc::new(helios_ui::StaticConformanceSource::empty()),
            FhirVersion::R4,
            None,
            "http://localhost:8080".to_string(),
            10 * 1024 * 1024,
            false,
            None,
            "http://localhost:8080".to_string(),
            Arc::new(helios_auth::outbound::NoOpOutboundAuthProvider),
            helios_ui::PatientNameSearchSupport::Enabled,
            Some(observer),
        )
    };
    let observers = Arc::new(helios_persistence::core::WriteObservers::new());
    let app = mount(Some(store()), observers.clone());
    assert_eq!(observers.len(), 1, "the inventory subscribed");
    let _second = mount(Some(store()), observers.clone());
    assert_eq!(observers.len(), 2, "one inventory per mounted app");
    drop(app);
    let _third = mount(Some(store()), observers.clone());
    assert_eq!(
        observers.len(),
        2,
        "a torn-down app's subscription is dropped on the next mount"
    );

    let headless = Arc::new(helios_persistence::core::WriteObservers::new());
    let _no_storage = mount(None, headless.clone());
    assert!(headless.is_empty(), "no storage, no inventory");
}

// ---- Registry first, counts deferred (#1851) ------------------------------

/// Every Tenants response renders the registry without waiting for the
/// cross-tenant count: the page, the search, the provisioning poll, a create
/// and a delete all answer while the count is held, with the counts pending
/// and the poller armed. All of them share the one held count.
#[tokio::test]
async fn registry_responses_return_while_the_count_is_held() {
    let (gated, inner, router) = gated();
    inner.register_tenant("acme", Some("Acme")).await.unwrap();
    inner.register_tenant("beta", None).await.unwrap();
    seed_patient(&inner, "acme").await;
    gated.hold();

    let (status, page) = within("the page", get(&router, "/ui/tenants")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(row_ids(&page).contains(&"acme".to_string()), "{page}");
    assert_eq!(counts_state(&page), "pending");
    assert_eq!(cell_state(&page, "acme").as_deref(), Some("pending"));
    let dom = Dom::page(&page);
    // Count loading is its own status, never the provisioning spinner.
    assert_eq!(dom.count(".busy-status"), 0);
    assert_eq!(
        dom.one("#tenant-counts-status").attr("role"),
        Some("status")
    );
    // The no-JS way forward while counts are on their way (#1848 D18).
    assert_eq!(
        dom.one(".counts-status__refresh").attr("href"),
        Some("/ui/tenants")
    );
    let poller = dom.one("[data-counts-poll]");
    assert_eq!(poller.attr("hx-trigger"), Some("every 2s"));
    assert_eq!(poller.attr("hx-include"), Some("[name='q']"));
    // No figure is claimed for the resources card.
    assert_eq!(dom.count("#tenant-stats .stat__value--unavailable"), 1);
    gated.wait_until_waiting(1).await;

    let (_, search) = within("the search", get(&router, "/ui/tenants/rows?q=ac")).await;
    assert_eq!(row_ids(&search), ["acme"]);
    assert_eq!(counts_state(&search), "pending");

    // Create: accepted, the in-flight row shows, the poll answers too.
    let created = within(
        "the create",
        post_form(&router, "id=newco&display_name=NewCo"),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK);
    assert!(
        created.1.contains(r#"class="busy-status""#),
        "{}",
        created.1
    );
    let (_, poll) = within("the provisioning poll", get(&router, "/ui/tenants/rows")).await;
    assert!(row_ids(&poll).contains(&"newco".to_string()));
    within("provisioning", wait_settled(&router)).await;

    // Delete (deregistration and purge alike).
    let (status, deleted) = within("the delete", delete_uri(&router, "/ui/tenants/beta")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!row_ids(&deleted).contains(&"beta".to_string()));
    let (status, _) = within(
        "the purge",
        delete_uri(&router, "/ui/tenants/acme?purge=true"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Still the first count, still held: nothing started a second one.
    assert_eq!(gated.discover_calls(), 1);
    assert_eq!(
        gated.count_by_tenant_calls(),
        0,
        "the UI never counts itself"
    );
    gated.release();
    let settled = wait_counts_settled(&router, "/ui/tenants/rows").await;
    assert_eq!(counts_state(&settled), "ready");
    assert_eq!(cell_state(&settled, "newco").as_deref(), Some("number"));
    assert!(!row_ids(&settled).contains(&"acme".to_string()), "purged");
}

/// Concurrent visits, searches and polls share one count, and a result that
/// lands after every request has gone is kept for the next one.
#[tokio::test]
async fn concurrent_visits_share_one_count_and_keep_its_result() {
    let (gated, inner, router) = gated();
    inner.register_tenant("acme", None).await.unwrap();
    seed_patient(&inner, "acme").await;
    gated.hold();

    let uris = [
        "/ui/tenants",
        "/ui/tenants/rows",
        "/ui/tenants/rows?q=a",
        "/ui/tenants/rows?q=ac&poll=1",
        "/ui/tenants/rows?poll=3",
        "/ui/tenants?q=acme",
    ];
    let requests = uris.iter().cycle().take(18).map(|uri| {
        let (router, uri) = (router.clone(), *uri);
        async move { get(&router, uri).await }
    });
    let responses = within("the burst", spawn_all(requests)).await;
    assert!(
        responses
            .iter()
            .all(|(status, _)| *status == StatusCode::OK)
    );
    gated.wait_until_waiting(1).await;
    assert_eq!(gated.discover_calls(), 1, "one count for the whole burst");

    // Every requester is gone; the count still completes and is kept.
    drop(responses);
    gated.release();
    let settled = wait_counts_settled(&router, "/ui/tenants/rows").await;
    assert_eq!(cell_state(&settled, "acme").as_deref(), Some("number"));
    let (_, page) = get(&router, "/ui/tenants").await;
    assert_eq!(counts_state(&page), "ready");
    assert_eq!(gated.discover_calls(), 1, "a fresh result starts nothing");
    // A settled page arms no poller.
    assert_eq!(Dom::page(&page).count("[data-counts-poll]"), 0);
}

/// Runs every request concurrently (each on its own task) and collects the
/// responses in order.
async fn spawn_all<F>(futures: impl Iterator<Item = F>) -> Vec<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let handles: Vec<_> = futures.map(tokio::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for handle in handles {
        out.push(handle.await.expect("request task"));
    }
    out
}

/// A count-only failure with a healthy registry keeps the roster on show,
/// with the counts marked unavailable and no error banner; nothing polls.
#[tokio::test]
async fn a_count_only_failure_keeps_the_roster() {
    let (gated, inner, router) = gated();
    inner.register_tenant("acme", Some("Acme")).await.unwrap();
    seed_patient(&inner, "acme").await;
    gated.fail_counts(true);

    let settled = wait_counts_settled(&router, "/ui/tenants/rows").await;
    assert_eq!(counts_state(&settled), "unavailable");
    assert_eq!(row_ids(&settled), ["acme"], "the roster stays");
    assert_eq!(cell_state(&settled, "acme").as_deref(), Some("unavailable"));
    let dom = Dom::page(&settled);
    assert_eq!(dom.count(".alert"), 0, "no registry banner: {settled}");
    assert_eq!(
        dom.count("[data-counts-poll]"),
        0,
        "a failure does not poll"
    );
    assert_eq!(dom.count(".counts-status__refresh"), 0);
    // The cards say what is known: one registered tenant, no total.
    assert_eq!(dom.all("#tenant-stats .stat__value")[0].text(), "1");
    assert_eq!(dom.count("#tenant-stats .stat__value--unavailable"), 1);
    assert!(
        dom.one("#tenant-stats")
            .text()
            .contains("Not available right now")
    );
}

/// A failed recount keeps the last counts on show, marked stale.
#[tokio::test]
async fn a_failed_recount_keeps_the_last_counts_as_stale() {
    let (gated, inner, router) = gated();
    inner.register_tenant("acme", None).await.unwrap();
    inner.register_tenant("beta", None).await.unwrap();
    seed_patient(&inner, "acme").await;
    let ready = wait_counts_settled(&router, "/ui/tenants/rows").await;
    assert_eq!(cell_state(&ready, "acme").as_deref(), Some("number"));

    gated.fail_counts(true);
    // A deregistration marks the inventory stale; the next view recounts.
    delete_uri(&router, "/ui/tenants/beta").await;
    let stale = wait_counts_settled(&router, "/ui/tenants/rows").await;
    assert_eq!(counts_state(&stale), "stale");
    let dom = Dom::page(&stale);
    let cell = dom
        .all("td.col-num")
        .into_iter()
        .find(|td| td.attr("data-count-state") == Some("number"))
        .expect("the last count stays on show");
    assert_eq!(cell.attr("data-count-stale"), Some("true"));
    assert!(cell.text().contains('1'));
    assert!(
        dom.one("#tenant-stats")
            .text()
            .contains("may be out of date")
    );
}

/// Deferred discovery keeps registered-empty tenants (a measured zero) and
/// data-only tenants, never shows the system tenant, follows the search, and
/// updates the global cards out of band with the rows.
#[tokio::test]
async fn discovered_tenants_follow_the_search_and_the_cards_stay_global() {
    let store = store();
    store.register_tenant("acme", None).await.unwrap();
    seed_patient(&store, "northwind").await;
    seed_patient(&store, "northwind").await;
    store
        .create(
            &TenantContext::system(),
            "AuditEvent",
            serde_json::json!({"resourceType": "AuditEvent"}),
            FhirVersion::R4,
        )
        .await
        .unwrap();
    let router = app(&store);

    let all = wait_counts_settled(&router, "/ui/tenants/rows").await;
    assert_eq!(row_ids(&all), ["acme", "northwind"], "no system tenant");
    assert_eq!(cell_state(&all, "acme").as_deref(), Some("zero"));
    assert_eq!(cell_state(&all, "northwind").as_deref(), Some("number"));
    assert!(!all.contains("__system__"));

    let (_, filtered) = get(&router, "/ui/tenants/rows?q=north").await;
    assert_eq!(row_ids(&filtered), ["northwind"]);
    let dom = Dom::page(&filtered);
    let stats = dom.one("#tenant-stats");
    assert_eq!(stats.attr("hx-swap-oob"), Some("outerHTML"));
    let values: Vec<String> = stats.all(".stat__value").iter().map(|v| v.text()).collect();
    assert_eq!(values, ["2", "2"], "global figures, not the filtered ones");
    assert!(stats.text().contains("1 registered"));
    assert_eq!(
        dom.one("#tenant-counts-status").attr("hx-swap-oob"),
        Some("innerHTML")
    );
}

/// Deregistering keeps a tenant with leftover data as an unregistered row,
/// in the delete's own response.
#[tokio::test]
async fn deregistration_keeps_leftover_data_as_an_unregistered_row() {
    let store = store();
    store.register_tenant("acme", Some("Acme")).await.unwrap();
    seed_patient(&store, "acme").await;
    let router = app(&store);
    wait_counts_settled(&router, "/ui/tenants/rows").await;

    let (_, html) = delete_uri(&router, "/ui/tenants/acme").await;
    let dom = Dom::page(&html);
    let row = dom
        .all("tbody tr")
        .into_iter()
        .find(|row| row.text().contains("acme"))
        .expect("the data keeps the row");
    assert_eq!(row.one(".tag--muted").text(), "unregistered");
    assert_eq!(
        row.one("td.col-num").attr("data-count-state"),
        Some("number")
    );
}

/// A purge that lands while a recount that began before it is held: the
/// held, pre-purge answer never brings the tenant back, and the follow-up
/// recount confirms it gone.
#[tokio::test]
async fn a_purge_during_a_held_recount_is_not_undone_by_it() {
    let (gated, inner, router) = gated();
    inner.register_tenant("acme", None).await.unwrap();
    inner.register_tenant("beta", None).await.unwrap();
    seed_patient(&inner, "acme").await;
    let ready = wait_counts_settled(&router, "/ui/tenants/rows").await;
    assert_eq!(cell_state(&ready, "acme").as_deref(), Some("number"));

    // A recount begins (a deregistration marked the inventory stale) and
    // reads acme's data before it is held.
    gated.hold();
    delete_uri(&router, "/ui/tenants/beta").await;
    get(&router, "/ui/tenants/rows").await;
    gated.wait_until_waiting(1).await;

    let (_, purged) = delete_uri(&router, "/ui/tenants/acme?purge=true").await;
    assert!(!row_ids(&purged).contains(&"acme".to_string()), "{purged}");

    gated.release();
    for _ in 0..200 {
        let (_, html) = get(&router, "/ui/tenants/rows").await;
        assert!(
            !row_ids(&html).contains(&"acme".to_string()),
            "a pre-purge snapshot resurrected the tenant: {html}"
        );
        if counts_state(&html) == "ready" {
            // The held recount, then one follow-up after the purge.
            assert_eq!(gated.discover_calls(), 3);
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the counts did not settle in time");
}

/// The create and delete reloads keep the active search term (#1851).
#[tokio::test]
async fn mutation_responses_keep_the_search_term() {
    let store = store();
    store
        .register_tenant("acme-health", Some("Acme Health"))
        .await
        .unwrap();
    store
        .register_tenant("riverside-labs", Some("Riverside Diagnostics"))
        .await
        .unwrap();
    let router = app(&store);

    // A rejected create (invalid id) and an accepted one alike.
    let (_, bad) = post_form(&router, "id=has%20space&q=river").await;
    assert_eq!(row_ids(&bad), ["riverside-labs"]);
    let (_, created) = post_form(&router, "id=newco&q=river").await;
    assert_eq!(
        row_ids(&created),
        ["riverside-labs"],
        "newco does not match"
    );
    wait_settled(&router).await;

    let (_, deleted) = delete_uri(&router, "/ui/tenants/newco?q=river").await;
    assert_eq!(row_ids(&deleted), ["riverside-labs"]);

    // The page's controls send it: the form and the delete buttons include
    // the search box, and every request queues on the table card.
    let (_, page) = get(&router, "/ui/tenants?q=acme").await;
    let dom = Dom::page(&page);
    let form = dom.one("form[hx-post='/ui/tenants']");
    assert_eq!(form.attr("hx-include"), Some("[name='q']"));
    assert_eq!(form.attr("hx-sync"), Some("closest .table-card:queue all"));
    let delete = dom.one("[hx-delete='/ui/tenants/acme-health']");
    assert_eq!(delete.attr("hx-include"), Some("[name='q']"));
    assert_eq!(
        delete.attr("hx-sync"),
        Some("closest .table-card:queue all")
    );
    assert_eq!(
        dom.one("input[name=q]").attr("hx-sync"),
        Some("closest .table-card:queue all")
    );
}
