//! End-to-end tests over the mounted router: the same requests a browser
//! would make, exercising [`helios_ui::mount`], the handlers, the embedded
//! asset service, the `Vary` middleware, and the FHIR fallback together.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    routing::get,
};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn app() -> Router {
    app_with(nl(true, true))
}

/// The natural-language search feature state (#255) the router is mounted with.
fn nl(enabled: bool, configured: bool) -> helios_ui::NlSearch {
    helios_ui::NlSearch {
        enabled,
        configured,
        model: "test-model".to_string(),
    }
}

fn app_with(nl: helios_ui::NlSearch) -> Router {
    // Inject an offline conformance source seeded from the shipped `data/`
    // bundles, so the SearchParameter/CompartmentDefinition viewers render real
    // data without a running server (production fetches these over HTTP).
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl,
        None,
        std::sync::Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        helios_fhir::FhirVersion::R4,
    )
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn index_serves_the_full_landing_page() {
    let response = app()
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("<!doctype html>"));
    assert!(html.contains("9.9.9"));
}

#[tokio::test]
async fn page_wires_the_collapsible_nav() {
    let response = app()
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html = body_text(response).await;

    // The first-paint script is loaded (non-deferred, in <head>).
    assert!(html.contains(r#"src="/ui/assets/nav.js""#));
    // The toggle is a real, accessible button controlling the sidebar.
    assert!(html.contains("data-toggle-nav"));
    assert!(html.contains(r#"aria-controls="sidebar""#));
    assert!(html.contains("aria-expanded"));
    // Labels are wrapped so the collapsed rail can hide them (a11y-safe).
    assert!(html.contains("nav-item__label"));
    // The sidebar is addressable by the toggle's aria-controls.
    assert!(html.contains(r#"id="sidebar""#));
}

#[tokio::test]
async fn status_is_a_full_page_on_hard_navigation() {
    let response = app()
        .oneshot(Request::get("/ui/status").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("<!doctype html>"),
        "without HX-Request the same URL must render the whole document"
    );
    assert!(html.contains("9.9.9"));
}

#[tokio::test]
async fn status_is_a_fragment_for_htmx_and_varies_on_the_header() {
    let response = app()
        .oneshot(
            Request::get("/ui/status")
                .header("HX-Request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let vary: Vec<_> = response
        .headers()
        .get_all(header::VARY)
        .iter()
        .map(|v| v.to_str().unwrap().to_ascii_lowercase())
        .collect();
    assert!(
        vary.iter().any(|v| v.contains("hx-request")),
        "AutoVaryLayer must emit Vary: HX-Request so caches never cross a \
         fragment with a full page, got {vary:?}"
    );

    let html = body_text(response).await;
    assert!(html.contains("Last checked:"));
    assert!(!html.contains("<html"), "fragment, not a full page");
}

#[tokio::test]
async fn embedded_assets_are_served() {
    for asset in ["/ui/assets/htmx.min.js", "/ui/assets/app.css"] {
        let response = app()
            .oneshot(Request::get(asset).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{asset}");
    }
}

#[tokio::test]
async fn non_ui_paths_fall_through_to_the_fhir_app() {
    // Stand-in for the FHIR REST router: proves /ui never shadows it.
    let fhir_app = Router::new().route("/Patient", get(|| async { "fhir handled" }));
    let response = helios_ui::mount_with_conformance_source(
        fhir_app,
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl(true, true),
        None,
        std::sync::Arc::new(helios_ui::StaticConformanceSource::empty()),
        helios_fhir::FhirVersion::R4,
    )
    .oneshot(Request::get("/Patient").body(Body::empty()).unwrap())
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_text(response).await, "fhir handled");
}

#[tokio::test]
async fn search_parameters_page_serves_the_registry_view() {
    let response = app()
        .oneshot(
            Request::get("/ui/search-parameters?base=Patient")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("<!doctype html>"));
    // The Resource Filter rail and the facet rows are server-rendered.
    assert!(html.contains(r#"id="sp-rail-list""#));
    assert!(html.contains("base=Patient"));
    // Real registry data, not placeholders: Patient supports `name`.
    assert!(html.contains("http://hl7.org/fhir/SearchParameter/Patient-name"));
    // This page, not Home, carries aria-current in the sidebar.
    assert!(html.contains(r#"href="/ui/search-parameters" aria-current="page""#));
}

#[tokio::test]
async fn search_parameters_selection_renders_the_detail_panel() {
    let response = app()
        .oneshot(
            Request::get(
                "/ui/search-parameters?base=Patient&sel=http%3A%2F%2Fhl7.org%2Ffhir%2FSearchParameter%2FPatient-name",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(r#"aria-selected="true""#));
    // The detail panel shows the FHIRPath expression of the spec parameter.
    assert!(html.contains("Patient.name"));
}

#[tokio::test]
async fn compartments_page_defaults_to_patient() {
    let response = app()
        .oneshot(
            Request::get("/ui/compartments")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("http://hl7.org/fhir/CompartmentDefinition/patient"));
    assert!(html.contains(r#"href="/ui/compartments" aria-current="page""#));
}

#[tokio::test]
async fn compartment_tester_resolves_membership_via_get() {
    let response = app()
        .oneshot(
            Request::get("/ui/compartments?def=Patient&tab=tester&id=example&target=Observation")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    // The equivalent flat search the server runs, straight from the
    // codegen'd table the REST handler consults.
    assert!(html.contains("subject=Patient/example"));
    assert!(html.contains("performer=Patient/example"));
}

#[tokio::test]
async fn compartment_tester_reports_non_members_as_404() {
    let response = app()
        .oneshot(
            Request::get("/ui/compartments?def=Patient&tab=tester&id=example&target=Medication")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("404 Not Found"));
    assert!(html.contains("OperationOutcome"));
}

#[tokio::test]
async fn queries_param_catalog_is_a_registry_fed_fragment() {
    let response = app()
        .oneshot(
            Request::get("/ui/queries/params?type=Patient")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(r#"<datalist id="param-options">"#));
    // Real registry data: Patient's own params plus Resource-level ones.
    assert!(html.contains(r#"value="birthdate""#));
    assert!(html.contains(r#"value="_id""#));
    // Not applicable to Patient.
    assert!(!html.contains(r#"value="clinical-status""#));
    assert!(!html.contains("<html"), "fragment, not a page");
}

/* Natural-language search (#255) has three states, and the difference between
 * them is the whole point of the feature's configuration: off means gone. */

#[tokio::test]
async fn nl_search_disabled_removes_the_page_and_every_mention_of_it() {
    let app = app_with(nl(false, false));

    let response = app
        .clone()
        .oneshot(Request::get("/ui/search").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "the route is never mounted"
    );

    // And nothing advertises it: the sidebar entry stays the coming-soon
    // placeholder it was before the feature existed.
    let html = body_text(
        app.oneshot(Request::get("/ui").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    assert!(!html.contains(r#"href="/ui/search""#));
    assert!(!html.to_lowercase().contains("natural language"));
}

#[tokio::test]
async fn nl_search_unconfigured_advertises_the_setup_without_an_input() {
    let response = app_with(nl(true, false))
        .oneshot(Request::get("/ui/search").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // The setup state names the env vars and links the how-to...
    assert!(html.contains("HFS_NL_SEARCH_API_KEY"));
    assert!(html.contains("HFS_NL_SEARCH_ENABLED=false"));
    assert!(html.contains("test-model"), "the model it would bill for");
    assert!(html.contains("components/natural-language-search.html"));
    // ...but there is nothing to type into, and the translator script that
    // would call the endpoint is not even loaded.
    assert!(!html.contains(r#"id="nl-text""#));
    assert!(!html.contains("nl-search.js"));
    // The visual builder still works — that is the fallback the setup names.
    assert!(html.contains(r#"id="saved-query-form""#));
}

#[tokio::test]
async fn nl_search_configured_renders_the_translator_over_an_editable_query() {
    let response = app_with(nl(true, true))
        .oneshot(Request::get("/ui/search").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(html.contains(r#"id="nl-text""#));
    assert!(html.contains("nl-search.js"));
    assert!(html.contains(r#"data-mode-btn="builder""#), "both modes");
    // The generated query lands in a plain editable input, not a read-only
    // display — reviewing and correcting it before running is the contract.
    assert!(html.contains(r#"class="query-builder__url""#));
    assert!(!html.contains("readonly"));
    // The key itself never reaches the page.
    assert!(!html.contains("HFS_NL_SEARCH_API_KEY"));
    // Enabled → the sidebar links the page.
    assert!(html.contains(r#"href="/ui/search""#));
}
