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
        None,
        "default".to_string(),
        std::sync::Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        helios_fhir::FhirVersion::R4,
        None,
    )
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn assert_recent_types_are_pinned_above_the_list(html: &str) {
    let recent = html
        .find(r#"id="type-rail-recent""#)
        .expect("Recently used group");
    let divider = html
        .find(r#"class="filter-rail__divider""#)
        .expect("recent-types divider");
    let all_types = html.find(">All Types<").expect("All Types heading");
    let list = html
        .find(r#"id="type-rail-list""#)
        .expect("scrollable type list");

    assert!(recent < divider, "Recently used must precede its divider");
    assert!(divider < all_types, "the divider must precede All Types");
    assert!(
        all_types < list,
        "Recently used and All Types must stay outside the scrollable list"
    );
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
async fn legacy_expand_query_param_is_ignored_and_harmless() {
    // `?expand=1` used to render the taller chart (#601); the affordance is
    // gone, but old bookmarks/links carrying the param must still resolve
    // cleanly, and the response must never echo it back into a live href.
    let response = app()
        .oneshot(Request::get("/ui?expand=1").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(!html.contains("expand=1"));
}

#[tokio::test]
async fn dashboard_renders_job_cards_with_unavailable_state_when_no_provider() {
    // No DashboardProvider is registered in this test router, so build_index_page
    // falls back to sample_snapshot, whose export_jobs/import_jobs_active are
    // both None — the dashboard must render the explicit "unavailable" state
    // rather than any fabricated numbers.
    let response = app()
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("Export Jobs"));
    assert!(html.contains("Import Jobs"));
    assert!(html.contains(r#"href="/ui/bulk-export""#));
    assert!(html.contains(r#"href="/ui/bulk-import""#));
    assert!(html.contains("unavailable"));
    assert!(!html.contains(">13<"));
    assert!(!html.contains("queued)"));

    // The Uptime card follows the same honesty rule (#540): this test binary
    // never calls helios_observability::uptime::init(), so the card renders
    // the unavailable state, not the old hardcoded percentage. The
    // initialized path lives in tests/dashboard_uptime_http.rs — a separate
    // binary, because the tracker is process-global.
    assert!(!html.contains("99.98"));
    assert!(!html.contains("since process start"));
}

#[tokio::test]
async fn page_wires_the_hover_rail_nav() {
    let response = app()
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html = body_text(response).await;

    // The rail expands on hover (#438): no toggle button, no state script.
    assert!(!html.contains("nav.js"));
    assert!(!html.contains("data-toggle-nav"));
    // Labels are wrapped so the resting rail can hide them (a11y-safe).
    assert!(html.contains("nav-item__label"));
    // The Batch & Data entries from the design: Import and Export.
    assert!(html.contains("Import"));
    assert!(html.contains("Export"));
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
    for asset in [
        "/ui/assets/htmx.min.js",
        "/ui/assets/app.css",
        "/ui/assets/fhir-search-value.js",
    ] {
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
        None,
        "default".to_string(),
        std::sync::Arc::new(helios_ui::StaticConformanceSource::empty()),
        helios_fhir::FhirVersion::R4,
        None,
    )
    .oneshot(Request::get("/Patient").body(Body::empty()).unwrap())
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_text(response).await, "fhir handled");
}

/// #653: the CapabilityStatement page renders the live /metadata answer —
/// summary, the batch/transaction distinction, linkified local operation
/// definitions, the filterable per-resource table, and the raw fold. Without
/// a fetchable statement it degrades to the warning, never fabricates.
#[tokio::test]
async fn capability_statement_page_renders_summary_and_degrades() {
    // The default test source seeds no metadata: the degraded warning shows.
    let response = app()
        .oneshot(
            Request::get("/ui/capability-statement")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("notice--warn"));

    // With a seeded statement the page renders the real content.
    let source = helios_ui::StaticConformanceSource::from_data_dir(std::path::Path::new(
        "../../data",
    ))
    .with_metadata(serde_json::json!({
        "resourceType": "CapabilityStatement",
        "status": "active", "kind": "instance", "date": "2026-08-24",
        "fhirVersion": "4.0.1",
        "format": ["application/fhir+json"],
        "implementation": {"description": "Helios FHIR Server", "url": "http://t/"},
        "rest": [{
            "interaction": [{"code": "batch"}, {"code": "transaction"}],
            "operation": [{"name": "export", "definition": "http://t/OperationDefinition/export"}],
            "resource": [
                {"type": "Patient", "interaction": [{"code": "read"}],
                 "searchParam": [{"name": "name"}]},
                {"type": "Observation", "interaction": [{"code": "read"}]}
            ]
        }]
    }));
    let app = helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl(true, true),
        None,
        None,
        "default".to_string(),
        std::sync::Arc::new(source),
        helios_fhir::FhirVersion::R4,
        None,
    );

    let response = app
        .clone()
        .oneshot(
            Request::get("/ui/capability-statement")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains("4.0.1"));
    assert!(html.contains(">batch<"));
    assert!(html.contains(">transaction<"));
    // The transaction-is-conditional note renders alongside the chips.
    assert!(html.contains("atomic transactions"));
    // Local operation definitions are clickable, at their local path.
    assert!(html.contains(r#"href="/OperationDefinition/export""#));
    assert!(html.contains("$export"));
    // Both resource rows, then the server-side filter narrows to one.
    assert!(html.contains(">Patient<") && html.contains(">Observation<"));
    let response = app
        .oneshot(
            Request::get("/ui/capability-statement?filter=obs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(!html.contains(">Patient<") && html.contains(">Observation<"));
    // The raw statement rides in the fold.
    assert!(html.contains("CapabilityStatement"));
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
    assert!(html.contains("<title>Search Parameters — Helios FHIR Server</title>"));
    assert!(html.contains(r#"<h1 class="page-head__title">Search Parameters</h1>"#));
    assert!(html.contains(
        r#"<table class="data-table" data-row-navigation aria-label="Search Parameters">"#
    ));
    assert!(html.contains(r#"<script src="/ui/assets/search-parameters.js" defer></script>"#));
    // The Resource Filter rail and the facet rows are server-rendered.
    assert!(html.contains(r#"id="sp-rail-list""#));
    assert!(html.contains("base=Patient"));
    // Real registry data, not placeholders: Patient supports `name`.
    assert!(html.contains("http://hl7.org/fhir/SearchParameter/Patient-name"));
    // Each result row keeps exactly one native link for keyboard and no-JS use.
    let table_body = html
        .split_once("<tbody>")
        .and_then(|(_, rest)| rest.split_once("</tbody>"))
        .map(|(body, _)| body)
        .expect("SearchParameter table body");
    let row_count = table_body.matches("<tr").count();
    assert!(row_count > 0);
    assert_eq!(table_body.matches(r#"class="row-link""#).count(), row_count);
    // This page, not Home, carries aria-current in the sidebar.
    assert!(html.contains(r#"href="/ui/search-parameters" aria-current="page""#));
    // The rail matches the flat Resources look (#603 follow-up): no bordered
    // card wrapper, and a divider + "All Types" heading separate the
    // Recently used group from the general list.
    assert!(!html.contains(r#"class="card filter-rail""#));
    assert!(html.contains(r#"class="filter-rail""#));
    assert!(html.contains(r#"class="filter-rail__divider""#));
    // "All Types" now renders twice: the new section heading, and the
    // existing "clear filter" row it sits above.
    assert_eq!(html.matches(">All Types<").count(), 2);
    let long_name = "MedicinalProductUndesirableEffect";
    assert!(html.contains(&format!(
        r#"data-type="{long_name}" data-full-name="{long_name}""#
    )));
    assert!(html.contains(&format!(r#"title="{long_name}""#)));
    assert!(html.contains(&format!(
        r#"<span class="filter-rail__label">{long_name}</span>"#
    )));
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

/// #320: when the conformance self-fetch yields nothing (an outage, or auth
/// without an outbound service token), the compartments page degrades to a
/// warning — it must not 404.
#[tokio::test]
async fn compartments_degrade_to_a_warning_when_the_fetch_is_empty() {
    let response = helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl(true, true),
        None,
        None,
        "default".to_string(),
        std::sync::Arc::new(helios_ui::StaticConformanceSource::empty()),
        helios_fhir::FhirVersion::R4,
        // No terminology server: this test is about the conformance fetch.
        None,
    )
    .oneshot(
        Request::get("/ui/compartments")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("notice--warn"));
    assert!(html.contains("<!doctype html>"));
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

    // Chaining metadata (#394): each option carries its type, and reference
    // params list their target resource types.
    assert!(html.contains(r#"data-type="date""#));
    let gp = html
        .lines()
        .find(|l| l.contains(r#"value="general-practitioner""#))
        .expect("general-practitioner option");
    assert!(gp.contains(r#"data-type="reference""#), "{gp}");
    assert!(gp.contains("Practitioner"), "targets in data-targets: {gp}");
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
    // The route renders; the search surface now lives inside Resources, so
    // there is no separate Search nav entry (#282).
}

#[tokio::test]
async fn search_and_queries_pin_recent_types_above_the_scrollable_list() {
    for path in ["/ui/search", "/ui/queries"] {
        let response = app()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let html = body_text(response).await;
        assert_recent_types_are_pinned_above_the_list(&html);
    }
}

/* The resource editor (#264). The endpoint takes the whole in-flight document
 * plus one mutation and hands back the re-rendered body — so these drive it the
 * way the browser does. */

async fn edit(form: &str) -> String {
    let response = app()
        .oneshot(
            Request::post("/ui/editor/render")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body_text(response).await
}

#[tokio::test]
async fn editor_page_renders_the_shell() {
    let response = app()
        .oneshot(
            Request::get("/ui/editor?type=Patient&id=abc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    // Brett's three panels on one screen: JSON, guided form, version rail.
    assert!(html.contains(r#"id="editor-body""#));
    assert!(html.contains(r#"id="editor-versions""#));
    // The resource is fetched by the browser from the FHIR API; the UI crate
    // never touches storage.
    assert!(html.contains(r#"data-type="Patient""#));
}

#[tokio::test]
async fn editor_offers_what_the_schema_allows_and_hides_what_is_spent() {
    let html =
        edit("doc=%7B%22resourceType%22%3A%22Patient%22%2C%22gender%22%3A%22male%22%7D&op=").await;

    // Elements the schema allows, offered by name.
    assert!(html.contains(r#"data-name="birthDate""#));
    assert!(html.contains(r#"data-name="identifier""#));
    // gender is set and does not repeat: not offered again.
    assert!(!html.contains(r#"data-name="gender""#));
    // A value[x] is a type pick, and its concrete arms are never fields.
    assert!(html.contains(r#"data-declarer="deceased""#));
    assert!(!html.contains(r#"data-name="deceasedBoolean""#));
}

/// The JSON view sits beside the guided form (Brett's layout), line-numbered
/// and foldable — a textarea cannot do either.
#[tokio::test]
async fn editor_renders_a_foldable_line_numbered_json_view() {
    let html =
        edit("doc=%7B%22resourceType%22%3A%22Patient%22%2C%22name%22%3A%5B%7B%22family%22%3A%22Duck%22%7D%5D%7D&op=")
            .await;

    // JSON and the guided form are both present — side by side, not toggled.
    assert!(html.contains("json-view"));
    assert!(html.contains("editor-tree"));
    // Line numbers in the gutter.
    assert!(html.contains("json-line__num"));
    // A fold arrow on the object and on the name array.
    assert!(html.contains("json-line--foldable"));
    assert!(html.contains("data-fold="));
    // Syntax highlighting: keys and strings are tokenised.
    assert!(html.contains("jt--key"));
    assert!(html.contains("jt--string"));
}

#[tokio::test]
async fn editor_adds_a_node_and_hands_back_the_new_document() {
    // Add a name to an empty Patient.
    let html = edit("doc=%7B%22resourceType%22%3A%22Patient%22%7D&op=add&path=&name=name").await;

    // The new node is rendered, and it can take an extension — the case that
    // matters, and the one the surveyed editors cannot do.
    assert!(html.contains(r#"data-path="name.0""#));
    assert!(
        html.contains(r#"data-path="name.0.extension""#) || html.contains(r#"data-add="name.0""#)
    );
}

/// The differentiator: an extension, by URL, at an arbitrary node, on a
/// resource that carries no profile at all.
#[tokio::test]
async fn editor_attaches_an_ad_hoc_extension_to_a_name() {
    let doc = "%7B%22resourceType%22%3A%22Patient%22%2C%22name%22%3A%5B%7B%22family%22%3A%22Duck%22%7D%5D%7D";
    let html = edit(&format!(
        "doc={doc}&op=extension&path=name.0&url=http%3A%2F%2Fexample.org%2Fmine"
    ))
    .await;

    // It landed on the name, not on the resource root.
    assert!(html.contains(r#"data-path="name.0.extension.0""#));
    assert!(html.contains("http://example.org/mine"));
    // And a fresh extension offers every value[x] arm.
    assert!(html.contains(r#"data-declarer="value""#));
}

/// The data-loss bug every editor surveyed for #264 has: an extended primitive
/// lives in two keys. Ours round-trips it, because the document is the state
/// and we never project it through a form.
#[tokio::test]
async fn editor_never_loses_a_key_it_does_not_render() {
    let doc = "%7B%22resourceType%22%3A%22Patient%22%2C%22birthDate%22%3A%221934-06-09%22%2C%22_birthDate%22%3A%7B%22extension%22%3A%5B%7B%22url%22%3A%22http%3A%2F%2Fexample.org%2Fprecision%22%2C%22valueCode%22%3A%22day%22%7D%5D%7D%7D";
    let html = edit(&format!("doc={doc}&op=add&path=&name=gender")).await;

    // The `_birthDate` sibling survived a structural mutation.
    assert!(html.contains("_birthDate"));
    assert!(html.contains("http://example.org/precision"));
}

/// Structural validation runs on every mutation and the issue lands on the row
/// that owns it — the anchoring other editors have to reconstruct by fuzzy
/// string matching, and that we get by construction.
///
/// Note what this does *not* cover: a bad *code* (`gender: "masculino"`) is a
/// terminology binding, and bindings are deferred to the async effects pass
/// (design doc §6). The live loop is the cheap structural pass; terminology is
/// checked on save.
#[tokio::test]
async fn editor_validates_on_every_mutation_and_anchors_the_issue() {
    // An element the schema does not have: caught by the structural pass.
    let doc = "%7B%22resourceType%22%3A%22Patient%22%2C%22notAnElement%22%3A%22x%22%7D";
    let html = edit(&format!("doc={doc}&op=")).await;

    assert!(
        html.contains("editor-row--error"),
        "the row knows it is wrong"
    );
    assert!(html.contains("editor-row__error"), "and says why, in place");
    // It is still rendered, and still in the document: we surface what we
    // cannot model, we do not delete it.
    assert!(html.contains(r#"data-path="notAnElement""#));
}

#[tokio::test]
async fn editor_keeps_the_users_text_when_the_json_is_broken() {
    let html = edit("doc=%7B%22resourceType%22%3A&op=").await;

    assert!(html.contains("class=\"alert\""));
    // Their text is handed straight back, not discarded.
    assert!(html.contains("resourceType"));
}

/* History & Versions (#236). The diff is computed server-side; these post two
 * versions the way the browser does after fetching them from _history. */

#[tokio::test]
async fn history_page_renders_the_shell() {
    let response = app()
        .oneshot(Request::get("/ui/history").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("<!doctype html>"));
    // Brett's tabs and the version rail.
    assert!(html.contains(r#"data-tab="instance""#));
    assert!(html.contains(r#"id="history-versions""#));
    assert!(html.contains("history.js"));
}

async fn diff(form: &str) -> String {
    let response = app()
        .oneshot(
            Request::post("/ui/history/diff")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body_text(response).await
}

#[tokio::test]
async fn diff_shows_a_rename_in_both_layers_and_hides_metadata() {
    // v3 -> v4: family Smith -> Smythe, and the meta churn that should be hidden.
    let from = "%7B%22name%22%3A%5B%7B%22family%22%3A%22Smith%22%7D%5D%2C%22meta%22%3A%7B%22versionId%22%3A%223%22%7D%7D";
    let to = "%7B%22name%22%3A%5B%7B%22family%22%3A%22Smythe%22%7D%5D%2C%22meta%22%3A%7B%22versionId%22%3A%224%22%7D%7D";
    let html = diff(&format!(
        "from={from}&to={to}&from_label=v3&to_label=v4&show_metadata=false"
    ))
    .await;

    // Semantic layer: a field-level replace on family, with the old value.
    assert!(html.contains("/name/0/family"));
    assert!(html.contains("Smith"));
    assert!(html.contains("Smythe"));
    // Metadata is filtered, and the toggle says how much.
    assert!(!html.contains("/meta/versionId"));
    assert!(html.contains("metadata"));
    // Textual layer: word-level highlight of the changed run.
    assert!(html.contains("<mark>"));
}

#[tokio::test]
async fn diff_shows_metadata_when_asked() {
    let from = "%7B%22meta%22%3A%7B%22versionId%22%3A%223%22%7D%7D";
    let to = "%7B%22meta%22%3A%7B%22versionId%22%3A%224%22%7D%7D";
    let html = diff(&format!("from={from}&to={to}&show_metadata=true")).await;
    assert!(html.contains("/meta/versionId"));
}

#[tokio::test]
async fn a_deleted_version_is_a_banner_not_a_diff() {
    let html = diff("from=%7B%7D&to=%7B%7D&to_label=v5&deleted=true").await;
    assert!(html.contains("history__banner--deleted"));
    assert!(html.contains("v5"));
    // No diff table for a tombstone.
    assert!(!html.contains("diff-table"));
}

#[tokio::test]
async fn identical_versions_say_so() {
    let doc = "%7B%22name%22%3A%5B%7B%22family%22%3A%22Smith%22%7D%5D%7D";
    let html = diff(&format!("from={doc}&to={doc}&show_metadata=true")).await;
    assert!(html.contains("history__banner--same"));
}

#[tokio::test]
async fn unparseable_versions_report_an_error_not_an_empty_diff() {
    let html = diff("from=%7Bnope&to=%7B%7D").await;
    assert!(html.contains("history__banner--error"));
}

/* Resources workspace (#282): the nav submenu + the page that ties the type
 * filter, search, and edit modal together. */

#[tokio::test]
async fn resources_page_has_the_filter_search_and_create_button() {
    let response = app()
        .oneshot(Request::get("/ui/resources").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    // The type filter rail, the search builder, and the Create button.
    assert!(html.contains(r#"id="type-rail-list""#));
    assert!(html.contains(r#"id="saved-query-form""#));
    assert!(html.contains(r#"id="resource-create""#));
    // The Create button names the selected type (#605), defaulting to
    // Patient, and the builder's URL is pre-filled so the no-JS form already
    // shows the query the client also runs on load.
    assert!(html.contains("Create new Patient"));
    assert!(html.contains(r#"value="GET /Patient""#));
    // The client-side template for the label update on rail clicks (#605):
    // the literal `{type}` placeholder, not the interpolated per-request value.
    assert!(html.contains(r#"data-msg-create="Create new {type}""#));
    // The "Recently used" group (#603) is present but hidden until
    // resource-filter.js populates it from localStorage.
    assert!(html.contains(r#"id="type-rail-recent""#));
    assert!(html.contains(r#"data-recent-key="hfs-recent-types""#));
    assert!(html.contains(r#"data-rail-list="type-rail-list""#));
    let recent_start = html.find(r#"id="type-rail-recent""#).unwrap();
    let recent_tag_end = html[recent_start..].find('>').unwrap() + recent_start;
    assert!(html[recent_start..recent_tag_end].contains("hidden"));
    assert!(html.contains(r#"src="/ui/assets/resource-filter.js" defer"#));
    // A divider and an "All Types" heading separate the Recently used group
    // from the general list (#603 follow-up), between the recent group and
    // the rail items.
    assert_recent_types_are_pinned_above_the_list(&html);
    // The edit modal shell, with its Edit / History tabs.
    assert!(html.contains(r#"id="resource-modal""#));
    assert!(html.contains(r#"data-modal-tab="history""#));
    // The rail unions the compartment enumeration with the generated Resource
    // enum (#648): HFS-served types the spec's compartments never mention —
    // ViewDefinition — are reachable, and the spec set is still there.
    assert!(html.contains(r#"data-type="ViewDefinition""#));
    assert!(html.contains(r#"data-type="EffectEvidenceSynthesis""#));
    // The nav carries a flat Resources entry, marked current on this page
    // (matching Brett's flat sidebar — the type picker is the page's own rail).
    assert!(html.contains(r#"href="/ui/resources" aria-current="page""#));
}

#[tokio::test]
async fn resources_deep_links_focus_the_selected_type() {
    let response = app()
        .oneshot(
            Request::get("/ui/resources?type=Observation")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    // The rail marks the deep-linked type, and Create targets it. The type
    // list is a flat rail in the content under the fixed page head (app-shell
    // pattern shared with Search Parameters), not a full-height menu panel.
    assert!(html.contains(r#"data-selected-type="Observation""#));
    assert!(html.contains(
        r#"data-type="Observation" data-full-name="Observation"
   href="/ui/resources?type=Observation" title="Observation" aria-current="true""#
    ));
    assert!(html.contains(r#"class="filter-rail" id="resources""#));
    // Create and the builder prefill both follow the deep-linked type.
    assert!(html.contains("Create new Observation"));
    assert!(html.contains(r#"value="GET /Observation""#));
}

#[tokio::test]
async fn version_selector_lists_the_enabled_versions() {
    let response = app()
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html = body_text(response).await;

    // A real disclosure, not the old static chip: one POST form per compiled-in
    // version, with the effective version marked current.
    assert!(html.contains(r#"action="/ui/version""#));
    assert!(html.contains(r#"name="version" value="R4""#));
    assert!(html.contains(r#"aria-current="true""#));
    // The default label is server-derived, not hardcoded markup.
    assert!(html.contains("FHIR R4"));
}

/* The live terminology picker (#365): /ui/editor/expand proxies the bound
 * value set's $expand through the server so the browser never talks to the
 * terminology server directly. */

fn app_with_terminology(terminology: Option<String>) -> Router {
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl(true, true),
        None,
        None,
        "default".to_string(),
        std::sync::Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        helios_fhir::FhirVersion::R4,
        terminology,
    )
}

#[tokio::test]
async fn terminology_navigation_reflects_the_configuration() {
    let response = app_with_terminology(None)
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(r#"href="/ui/terminology""#));

    let response = app_with_terminology(None)
        .oneshot(Request::get("/ui/terminology").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(r#"id="terminology-setup""#));
    assert!(html.contains("HFS_TERMINOLOGY_SERVER=http://localhost:8090"));
    assert!(html.contains(r#"href="/ui/terminology" aria-current="page""#));

    let valid = "https://terminology.example/fhir/";
    let response = app_with_terminology(Some(valid.to_string()))
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(&format!(r#"href="{valid}""#)));
    assert!(html.contains(r#"target="_blank""#));
    assert!(html.contains(r#"rel="noopener noreferrer""#));
    assert!(html.contains(r#"hx-boost="false""#));
    assert!(html.contains("opens in a new tab"));
    assert!(!html.contains(r#"href="/ui/terminology""#));

    let response = app_with_terminology(Some(valid.to_string()))
        .oneshot(Request::get("/ui/queries").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(&format!(r#"href="{valid}""#)));
    assert!(html.contains(r#"target="_blank""#));
    assert!(html.contains(r#"rel="noopener noreferrer""#));
    assert!(html.contains(r#"hx-boost="false""#));

    let invalid = "javascript:alert(1)";
    let response = app_with_terminology(Some(invalid.to_string()))
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(r#"href="/ui/terminology""#));
    assert!(!html.contains(invalid));

    let response = app_with_terminology(Some(invalid.to_string()))
        .oneshot(Request::get("/ui/terminology").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(r#"id="terminology-invalid" role="alert""#));
    assert!(html.contains("absolute HTTP or HTTPS URL"));
    assert!(html.contains("HFS_TERMINOLOGY_SERVER=http://localhost:8090"));
    assert!(!html.contains(invalid));

    for invalid in [
        "",
        " https://terminology.example/fhir",
        "/fhir",
        "ftp://terminology.example/fhir",
        "https://user:secret@terminology.example/fhir",
        "https://terminology.example/fhir?mode=test",
        "https://terminology.example/fhir#codes",
    ] {
        let response = app_with_terminology(Some(invalid.to_string()))
            .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{invalid}");
        let html = body_text(response).await;
        assert!(html.contains(r#"href="/ui/terminology""#), "{invalid}");
        assert!(!html.contains(&format!(r#"href="{invalid}""#)), "{invalid}");
    }
}

/// A loopback stand-in for the terminology server: one canned response for
/// `GET /ValueSet/$expand`.
async fn mock_terminology(status: StatusCode, body: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/ValueSet/$expand",
        axum::routing::get(move || async move {
            (
                status,
                [("content-type", "application/fhir+json")],
                body.to_string(),
            )
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn expand_without_a_terminology_server_returns_no_content() {
    let response = app_with_terminology(None)
        .oneshot(
            Request::get("/ui/editor/expand?url=http%3A%2F%2Floinc.org%2Fvs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn expand_proxies_the_expansion_as_a_compact_code_list() {
    let base = mock_terminology(
        StatusCode::OK,
        r#"{"resourceType":"ValueSet","expansion":{"contains":[
            {"system":"http://loinc.org","code":"8302-2","display":"Body height"},
            {"system":"http://loinc.org","code":"8867-4","display":"Heart rate"}
        ]}}"#,
    )
    .await;

    let response = app_with_terminology(Some(base))
        .oneshot(
            Request::get("/ui/editor/expand?url=http%3A%2F%2Floinc.org%2Fvs&filter=he")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    let codes = body["codes"].as_array().unwrap();
    assert_eq!(codes.len(), 2);
    assert_eq!(codes[0]["code"], "8302-2");
    assert_eq!(codes[0]["display"], "Body height");
}

#[tokio::test]
async fn expand_turns_a_terminology_failure_into_no_content() {
    let base = mock_terminology(StatusCode::INTERNAL_SERVER_ERROR, "boom").await;

    let response = app_with_terminology(Some(base))
        .oneshot(
            Request::get("/ui/editor/expand?url=http%3A%2F%2Floinc.org%2Fvs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn expand_turns_a_non_json_answer_into_no_content() {
    let base = mock_terminology(StatusCode::OK, "<html>not fhir</html>").await;

    let response = app_with_terminology(Some(base))
        .oneshot(
            Request::get("/ui/editor/expand?url=http%3A%2F%2Floinc.org%2Fvs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn expand_tolerates_an_expansion_without_contains() {
    let base = mock_terminology(StatusCode::OK, r#"{"resourceType":"ValueSet"}"#).await;

    let response = app_with_terminology(Some(base))
        .oneshot(
            Request::get("/ui/editor/expand?url=http%3A%2F%2Floinc.org%2Fvs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(body["codes"].as_array().unwrap().len(), 0);
}

/// An add that names a slice routes through `add_slice_element`. Core schemas
/// carry no slice definitions, so the seed lookup finds nothing and the item
/// is appended blank — same outcome as a plain add, via the slice branch.
#[tokio::test]
async fn editor_add_with_a_slice_name_appends_the_item() {
    let html =
        edit("doc=%7B%22resourceType%22%3A%22Patient%22%7D&op=add&path=&name=identifier&slice=mrn")
            .await;

    assert!(html.contains(r#"data-path="identifier.0""#));
}

/// #476: the Batch/Transaction workspace mounts and the nav links it.
#[tokio::test]
async fn batch_page_serves_the_workspace_shell() {
    let response = app()
        .oneshot(Request::get("/ui/batch").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("<!doctype html>"));
    // The three client-driven stages are all in the shell.
    assert!(html.contains(r#"id="batch-upload""#));
    assert!(html.contains(r#"id="batch-preflight""#));
    assert!(html.contains(r#"id="batch-response""#));
    // The nav entry is a real link now, current on this page.
    assert!(html.contains(r#"href="/ui/batch" aria-current="page""#));
    // The semantics copy rides in as data for batch.js.
    assert!(html.contains("data-msg-semantics-transaction"));
    assert!(html.contains(r#"src="/ui/assets/batch.js""#));
}

/* #546: creating a resource with required elements must not dump duplicated,
 * unanchored error lines. */

#[tokio::test]
async fn editor_orphan_issues_are_deduped_and_titled() {
    // A brand-new Claim: several required elements, none present, no rows.
    let html = edit("doc=%7B%22resourceType%22%3A%22Claim%22%7D&op=").await;

    // Styled panel with a heading, not bare lines.
    assert!(html.contains("editor__orphans-title"), "titled panel");
    // The old "priority: priority is required" duplication is gone.
    assert!(
        !html.contains("priority: priority"),
        "no duplicated element name"
    );
    assert!(html.contains("priority"), "the issue itself still shows");
}

/* #549: temporal primitives get format assistance. */

#[tokio::test]
async fn editor_temporal_primitives_carry_format_assistance() {
    // A Patient with a birthDate (type `date`) present renders its row.
    let html = edit(
        "doc=%7B%22resourceType%22%3A%22Patient%22%2C%22birthDate%22%3A%222024-01-01%22%7D&op=",
    )
    .await;
    assert!(
        html.contains(r#"placeholder="2024-05-17""#),
        "date placeholder"
    );
    assert!(
        html.contains(r#"pattern="\d{4}(-\d{2}(-\d{2})?)?""#),
        "date pattern"
    );
    // Non-temporal primitives stay bare.
    let html = edit(
        "doc=%7B%22resourceType%22%3A%22Patient%22%2C%22name%22%3A%5B%7B%22family%22%3A%22X%22%7D%5D%7D&op=",
    )
    .await;
    assert!(!html.contains(r#"placeholder="14:30:00""#));
}

/* #547: the server marks the created node so the client can focus it, and
 * an empty document opens the root add-picker by itself. */

#[tokio::test]
async fn editor_marks_the_created_node_for_focus() {
    let html =
        edit("doc=%7B%22resourceType%22%3A%22Patient%22%7D&op=add&path=&name=birthDate").await;
    assert!(
        html.contains(r#"data-focus="birthDate""#),
        "created path marked: {}",
        &html[..400]
    );
}

#[tokio::test]
async fn editor_opens_the_root_picker_on_an_empty_document() {
    let html = edit("doc=%7B%22resourceType%22%3A%22Patient%22%7D&op=").await;
    assert!(
        html.contains(r#"<details class="editor-add" open>"#),
        "root picker auto-opens"
    );

    // With content present it stays closed.
    let html = edit(
        "doc=%7B%22resourceType%22%3A%22Patient%22%2C%22birthDate%22%3A%222024-01-01%22%7D&op=",
    )
    .await;
    assert!(!html.contains(r#"<details class="editor-add" open>"#));
}
