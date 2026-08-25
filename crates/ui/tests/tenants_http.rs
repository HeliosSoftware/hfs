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

use std::sync::Arc;
use std::time::Duration;

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
/// marker is the real in-flight row class (`class="provisioning"`), not an
/// invented one.
async fn wait_settled(router: &Router) -> String {
    for _ in 0..200 {
        let (_, html) = get(router, "/ui/tenants/rows").await;
        if !html.contains(r#"class="provisioning""#) {
            return html;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("provisioning did not settle in time");
}

#[tokio::test]
async fn page_renders_and_lists_registered_and_discovered_tenants() {
    let store = store();
    let router = app(&store);

    // A registered tenant (via the page's own form) ...
    let (status, _) = post_form(&router, "id=acme-health&display_name=Acme+Health").await;
    assert_eq!(status, StatusCode::OK);
    wait_settled(&router).await;

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

    let (status, html) = get(&router, "/ui/tenants").await;
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
        )
    };

    // Default version (R4): the queries page's type rail carries the R4 set.
    let res = app()
        .oneshot(Request::get("/ui/queries").body(Body::empty()).unwrap())
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
        .oneshot(Request::get("/ui/queries").body(Body::empty()).unwrap())
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
        ok_body.contains(r#"class="provisioning""#),
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
    assert!(!html.contains(r#"class="provisioning""#));
}
