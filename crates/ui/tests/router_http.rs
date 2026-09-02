//! End-to-end tests over the mounted router: the same requests a browser
//! would make, exercising [`helios_ui::mount`], the handlers, the embedded
//! asset service, the `Vary` middleware, and the FHIR fallback together.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    routing::get,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use helios_persistence::{
    StorageResult,
    core::{SettingsStore, StoredUserSettings},
};
use http_body_util::BodyExt;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

struct NoSettingsAccess;

#[async_trait::async_trait]
impl SettingsStore for NoSettingsAccess {
    async fn get_settings(&self, _user_key: &str) -> StorageResult<Option<StoredUserSettings>> {
        panic!("JSON preview must not read settings")
    }

    async fn put_settings(
        &self,
        _user_key: &str,
        _document: Value,
        _if_match_version: Option<i64>,
    ) -> StorageResult<StoredUserSettings> {
        panic!("JSON preview must not write settings")
    }

    async fn patch_settings(
        &self,
        _user_key: &str,
        _merge_patch: Value,
        _if_match_version: Option<i64>,
    ) -> StorageResult<StoredUserSettings> {
        panic!("JSON preview must not write settings")
    }

    async fn delete_settings(&self, _user_key: &str) -> StorageResult<bool> {
        panic!("JSON preview must not delete settings")
    }

    async fn purge_tenant_settings(&self, _tenant_id: &str) -> StorageResult<u64> {
        panic!("JSON preview must not purge settings")
    }
}

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
        "http://localhost:8080".to_string(),
        None,
    )
}

fn resources_app_with_metadata(resources: &[(&str, bool)]) -> Router {
    let rows: Vec<Value> = resources
        .iter()
        .map(|(resource_type, create)| {
            serde_json::json!({
                "type": resource_type,
                "interaction": if *create {
                    serde_json::json!([{"code": "read"}, {"code": "create"}])
                } else {
                    serde_json::json!([{"code": "read"}])
                }
            })
        })
        .collect();
    resources_app_with_statement(serde_json::json!({
        "resourceType": "CapabilityStatement",
        "fhirVersion": "4.0.1",
        "rest": [{"mode": "server", "resource": rows}]
    }))
}

fn resources_app_with_statement(statement: Value) -> Router {
    let source =
        helios_ui::StaticConformanceSource::from_data_dir(std::path::Path::new("../../data"))
            .with_metadata(statement);
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl(true, true),
        None,
        None,
        "default".to_string(),
        Arc::new(source),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    )
}

fn app_with_body_limit(max_body_size: usize) -> Router {
    helios_ui::mount_with_conformance_source_and_body_limit(
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
        None,
        "http://localhost:8080".to_string(),
        max_body_size,
        None,
    )
}

fn production_app() -> Router {
    helios_ui::mount(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl(true, true),
        None,
        None,
        "default".to_string(),
        "http://127.0.0.1:9".to_string(),
        Arc::new(helios_auth::NoOpOutboundAuthProvider),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    )
}

fn app_with_unavailable_settings() -> Router {
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl(true, true),
        None,
        Some(Arc::new(NoSettingsAccess)),
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

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    // Normalized to LF: what line endings the response carries depends on how
    // the build checkout materialized the templates (#671), which is exactly
    // what these assertions must not depend on.
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .replace("\r\n", "\n")
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

/// #753: the vendored CodeMirror 6 + lezer-fhirpath bundle is
/// served like every other embedded asset — same route shape, same
/// JavaScript content type — with no change to how assets are declared or
/// served (rust-embed already walks subfolders; `assets/fonts/` is the
/// existing precedent for `assets/vendor/`).
#[tokio::test]
async fn codemirror_vendor_bundle_is_served() {
    let response = app()
        .oneshot(
            Request::get("/ui/assets/vendor/codemirror.bundle.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("content-type header present")
        .to_str()
        .unwrap();
    assert!(
        content_type.contains("javascript"),
        "expected a JavaScript content-type, got {content_type}"
    );
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
        "http://localhost:8080".to_string(),
        None,
    )
    .oneshot(Request::get("/Patient").body(Body::empty()).unwrap())
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_text(response).await, "fhir handled");
}

/// #653: the CapabilityStatement page renders the live /metadata answer —
/// summary, safe external documentation links, semantic interaction tags, the
/// progressively enhanced resource filter, and bounded raw JSON shell. Without
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
        "rawOnlyMarker": "RAW_ONLY_CAPABILITY_MARKER",
        "format": ["application/fhir+json"],
        "implementation": {"description": "Helios FHIR Server", "url": "http://t/"},
        "rest": [{
            "mode": "server",
            "interaction": [
                {"code": "batch"}, {"code": "transaction"}, {"code": "transaction"},
                {"code": "search-system"}, {"code": "history-system"},
                {"code": "future-code"}
            ],
            "operation": [
                {"name": "export", "definition": "https://example.org/OperationDefinition/export|1.2.3"},
                {"name": "unsafe", "definition": "javascript:alert(1)"}
            ],
            "resource": [
                {"type": "Patient", "profile": "http://hl7.org/fhir/StructureDefinition/Patient",
                 "interaction": [{"code": "read"}, {"code": "delete"}],
                 "searchParam": [{"name": "name"}]},
                {"type": "Observation", "profile": "https://example.org/StructureDefinition/Observation|2.0",
                 "interaction": [{"code": "create"}, {"code": "future-code"}]},
                {"type": "Encounter", "profile": "urn:oid:1.2.3",
                 "interaction": [{"code": "read"}]},
                {"type": "NotARealResource", "profile": "https://example.org/custom|1.0",
                 "interaction": [{"code": "read"}]},
                {"type": "UnknownUnsafe", "profile": "javascript:alert(2)",
                 "interaction": [{"code": "read"}]}
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
        "http://localhost:8080".to_string(),
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
    assert!(html.contains(r#"<div class="kv-grid kv-grid--flush">"#));
    assert!(
        html.contains(r#"<div class="detail__field detail__field--wide"><span>Description</span>"#)
    );
    assert!(
        html.contains(r#"<div class="detail__field detail__field--wide"><span>Base URL</span>"#)
    );
    assert!(html.contains(
        r#"href="https://hl7.org/fhir/R4/http.html#batch" target="_blank" rel="noopener">batch</a>"#
    ));
    assert!(html.contains(
        r#"href="https://hl7.org/fhir/R4/http.html#transaction" target="_blank" rel="noopener">transaction</a>"#
    ));
    assert!(html.contains(r#"href="https://hl7.org/fhir/R4/http.html#search""#));
    assert!(html.contains(r#"href="https://hl7.org/fhir/R4/http.html#history""#));
    assert!(html.contains(r#"class="tag tag--muted">future-code</span>"#));
    // Duplicate transaction declarations still produce one subordinate note.
    assert_eq!(html.matches("atomic transactions").count(), 1);
    assert!(html.contains(r#"<p class="cap-transaction-note">"#));
    assert!(!html.contains(r#"<p class="page-head__lede">atomic transactions"#));
    assert!(html.contains("#primarysecondary-role-matrix"));
    // Absolute HTTP(S) canonicals are clickable and their FHIR `|version`
    // qualifier is omitted from href while remaining visible as link text.
    assert!(html.contains(
        r#"href="https://example.org/OperationDefinition/export" target="_blank" rel="noopener">https://example.org/OperationDefinition/export|1.2.3</a>"#
    ));
    assert!(html.contains("javascript:alert(1)"));
    assert!(!html.contains(r#"href="javascript:"#));
    assert!(html.contains("$export"));
    // The real HFS core profile becomes a versioned documentation link. Safe
    // custom profiles stay intact, unsafe known profiles use the core page,
    // and unknown unsafe types remain plain text.
    assert!(html.contains(
        r#"href="https://hl7.org/fhir/R4/patient.html" target="_blank" rel="noopener">Patient</a>"#
    ));
    assert!(html.contains(
        r#"href="https://example.org/StructureDefinition/Observation" target="_blank" rel="noopener">Observation</a>"#
    ));
    assert!(html.contains(
        r#"href="https://hl7.org/fhir/R4/encounter.html" target="_blank" rel="noopener">Encounter</a>"#
    ));
    assert!(html.contains(
        r#"href="https://example.org/custom" target="_blank" rel="noopener">NotARealResource</a>"#
    ));
    assert!(html.contains("<span>UnknownUnsafe</span>"));
    assert!(!html.contains(r#"href="javascript:alert(2)""#));
    assert!(html.contains(r#"class="tag tag--member">read</span>"#));
    assert!(html.contains(r#"class="tag tag--config">create</span>"#));
    assert!(html.contains(r#"class="tag tag--excluded">delete</span>"#));
    // The ordinary page carries only a lazy shell. It neither renders nor
    // pretty-prints the large CapabilityStatement before the fold is opened.
    assert!(html.contains(r#"id="capability-json-fold""#));
    assert!(html.contains(r#"id="capability-json-body""#));
    assert!(html.contains(r#"data-fragment-url="/ui/capability-statement/json-fragment?"#));
    assert!(html.contains("/ui/capability-statement/json-fragment?"));
    assert!(html.contains("raw=1"));
    assert!(html.contains("version=R4"));
    assert!(html.contains("Open plain JSON"));
    assert!(html.contains(r#"role="status""#));
    assert!(!html.contains(r#"id="capability-json""#));
    assert!(!html.contains(r#"class="json-line"#));
    assert!(!html.contains(r#"<pre class="detail__code">"#));
    assert!(!html.contains("RAW_ONLY_CAPABILITY_MARKER"));

    // An explicit raw request is the no-JavaScript fallback: plain JSON in an
    // open disclosure, never the expensive highlighted DOM.
    let response = app
        .clone()
        .oneshot(
            Request::get("/ui/capability-statement?raw=1&version=R4&filter=Patient")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let raw_html = body_text(response).await;
    assert!(raw_html.contains(r#"id="capability-json-fold" open"#));
    assert!(raw_html.contains(r#"<pre class="detail__code">"#));
    assert!(raw_html.contains("RAW_ONLY_CAPABILITY_MARKER"));
    assert!(raw_html.contains("Plain JSON fallback"));
    assert!(!raw_html.contains(r#"class="json-view""#));
    assert!(!raw_html.contains(r#"class="json-line"#));
    assert!(!raw_html.contains(r#"data-capability-json-body""#));

    // This small statement still uses the unchanged shared renderer when the
    // bounded fragment endpoint is requested.
    let response = app
        .clone()
        .oneshot(
            Request::get(
                "/ui/capability-statement/json-fragment?version=R4&path=&offset=0&limit=100",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let fragment = body_text(response).await;
    assert!(fragment.contains(r#"class="json-view" id="capability-json""#));
    assert!(fragment.contains(r#"class="jt--key""#));
    assert!(fragment.contains("data-fold="));
    // The real GET form remains the fallback while htmx enhances live input.
    assert!(html.contains(r#"method="get" action="/ui/capability-statement""#));
    assert!(html.contains(r#"<input type="hidden" name="version" value="R4">"#));
    assert!(!html.contains(r#"name="raw""#));
    assert!(html.contains(r#"hx-get="/ui/capability-statement" hx-include="closest form""#));
    assert!(html.contains(r#"hx-trigger="input changed delay:300ms, search""#));
    assert!(html.contains(r##"hx-target="#cap-resource-table" hx-select="#cap-resource-table""##));
    assert!(html.contains(r#"class="card table-card cap-resource-card""#));
    assert!(html.contains(r#"class="filter-rail__search cap-resource-filter""#));
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
}

#[cfg(feature = "R5")]
#[tokio::test]
async fn capability_statement_filter_preserves_explicit_non_default_version() {
    let source =
        helios_ui::StaticConformanceSource::from_data_dir(std::path::Path::new("../../data"))
            .with_metadata(serde_json::json!({
                "resourceType": "CapabilityStatement",
                "status": "active",
                "kind": "instance",
                "fhirVersion": "5.0.0",
                "rest": [{
                    "mode": "server",
                    "resource": [
                        {
                            "type": "Patient",
                            "profile": "http://hl7.org/fhir/StructureDefinition/Patient",
                            "interaction": [{"code": "read"}]
                        },
                        {
                            "type": "Observation",
                            "profile": "http://hl7.org/fhir/StructureDefinition/Observation",
                            "interaction": [{"code": "read"}]
                        }
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
        Arc::new(source),
        // R4 is deliberately the server default: the query must override it.
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    );

    let response = app
        .oneshot(
            Request::get("/ui/capability-statement?version=R5&filter=obs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(html.contains(r#"<input type="hidden" name="version" value="R5">"#));
    assert!(!html.contains(r#"name="raw""#));
    assert!(!html.contains(">Patient<") && html.contains(">Observation<"));
    assert!(html.contains(
        r#"href="https://hl7.org/fhir/R5/observation.html" target="_blank" rel="noopener">Observation</a>"#
    ));
    assert!(
        html.contains(r#"data-fragment-url="/ui/capability-statement/json-fragment?version=R5"#)
    );
    assert!(html.contains("FHIR R5"));
}

#[tokio::test]
async fn capability_statement_large_json_is_plain_without_js_and_paged_with_htmx() {
    let oversized = Value::Array(
        (0..100_001)
            .map(|index| Value::from(index as u64))
            .collect(),
    );
    let source =
        helios_ui::StaticConformanceSource::from_data_dir(std::path::Path::new("../../data"))
            .with_metadata(serde_json::json!({
                "resourceType": "CapabilityStatement",
                "fhirVersion": "4.0.1",
                "extension": oversized,
                "rest": [{"mode": "server"}]
            }));
    let app = helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl(true, true),
        None,
        None,
        "default".to_string(),
        Arc::new(source),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    );

    let response = app
        .clone()
        .oneshot(
            Request::get("/ui/capability-statement?raw=1&version=R4")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(r#"<pre class="detail__code">"#));
    assert!(html.contains("CapabilityStatement"));
    assert!(html.contains("100000"));
    assert!(!html.contains(r#"class="json-view""#));
    assert!(!html.contains(r#"class="json-line"#));

    let response = app
        .clone()
        .oneshot(
            Request::get(
                "/ui/capability-statement/json-fragment?version=R4&path=&offset=0&limit=100",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let root = body_text(response).await;
    assert!(root.len() <= 1024 * 1024);
    assert!(root.contains(r#"data-capability-json-page"#));
    assert!(root.contains(r#"data-item-count="4""#));
    assert!(root.contains("extension"), "{root}");
    assert!(root.contains("[ 100001 ]"));
    assert!(root.contains("path=%2Fextension"));
    assert!(!root.contains("100000"));

    let response = app
        .clone()
        .oneshot(
            Request::get(
                "/ui/capability-statement/json-fragment?version=R4&path=%2Fextension&offset=0&limit=100",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let first_page = body_text(response).await;
    assert!(first_page.len() <= 1024 * 1024);
    assert!(first_page.contains(r#"data-item-count="100""#));
    assert!(first_page.contains("1–100 / 100001"));
    assert!(first_page.contains("offset=100"));
    assert!(!first_page.contains(">100<"));

    let response = app
        .clone()
        .oneshot(
            Request::get(
                "/ui/capability-statement/json-fragment?version=R4&path=%2Fextension&offset=100000&limit=100",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let last_page = body_text(response).await;
    assert!(last_page.contains(r#"data-item-count="1""#));
    assert!(last_page.contains("100001–100001 / 100001"));

    for uri in [
        "/ui/capability-statement/json-fragment?version=R4&path=not-a-pointer",
        "/ui/capability-statement/json-fragment?version=R4&path=%2Fextension&limit=101",
        "/ui/capability-statement/json-fragment?version=R4&path=%2Fextension&offset=100002",
        "/ui/capability-statement/json-fragment?version=R7&path=",
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
    }
    let response = app
        .clone()
        .oneshot(
            Request::get(
                "/ui/capability-statement/json-fragment?path=%2Frest%2F0%2Fmode&offset=0&limit=100",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let subtree = body_text(response).await;
    assert!(subtree.contains(r#"class="json-view""#));
    assert!(!subtree.contains(r#"id="capability-json""#));

    let response = app
        .oneshot(
            Request::get("/ui/capability-statement/json-fragment?version=R4&path=%2Fmissing")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn capability_statement_json_fragment_degrades_when_metadata_is_unavailable() {
    struct FailingMetadataSource;

    #[async_trait::async_trait]
    impl helios_ui::ConformanceSource for FailingMetadataSource {
        async fn fetch(
            &self,
            _resource_type: &str,
            _version: helios_fhir::FhirVersion,
            _tenant: &str,
        ) -> Result<Vec<Value>, String> {
            Ok(Vec::new())
        }
    }

    let app = helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl(true, true),
        None,
        None,
        "default".to_string(),
        Arc::new(FailingMetadataSource),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    );

    let response = app
        .oneshot(
            Request::get("/ui/capability-statement/json-fragment")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body_text(response).await,
        "CapabilityStatement is unavailable"
    );
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
        "http://localhost:8080".to_string(),
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
    assert_eq!(html.matches(r#"id="json-view""#).count(), 1);
    assert!(html.contains(r#"data-jpath="name.0.family""#));
    assert!(html.contains(r#"class="json-line__num" aria-hidden="true""#));
    assert!(html.contains(r#"aria-expanded="true""#));
}

#[tokio::test]
async fn json_view_endpoint_renders_a_normal_bundle_without_editor_contracts() {
    let response = app()
        .oneshot(
            Request::post("/ui/json-view/render")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"resourceType":"Bundle","type":"batch","entry":[{"resource":{"resourceType":"Patient","a\"\\\n\t\u0001":"<script>alert(1)</script>"},"request":{"method":"POST","url":"Patient"}}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/html; charset=utf-8"
    );
    let html = body_text(response).await;
    assert!(html.contains(r#"class="json-view""#));
    assert!(!html.contains(r#"id="json-view""#));
    assert!(!html.contains("data-jpath"));
    assert!(html.contains("&#60;script&#62;alert(1)&#60;/script&#62;"));
    assert!(!html.contains("<script>alert(1)</script>"));
    assert!(html.contains(r#"a\&#34;\\\n\t\u0001"#));
}

#[tokio::test]
async fn json_view_endpoint_rejects_compact_structural_amplification() {
    // Roughly 20 KiB on the wire used to expand to about 3.5 MiB of HTML.
    // It is comfortably below the default body limit but above the rendering
    // budget, so rejection happens before a large line Vec/template String.
    let document = format!(
        "[{}]",
        std::iter::repeat_n("0", 10_000)
            .collect::<Vec<_>>()
            .join(",")
    );
    assert!(document.len() < 25_000);

    let response = app()
        .oneshot(
            Request::post("/ui/json-view/render")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(document))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn json_view_endpoint_skips_preferences_but_keeps_locale_negotiation() {
    let response = app_with_unavailable_settings()
        .oneshot(
            Request::post("/ui/json-view/render")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT_LANGUAGE, "es")
                .body(Body::from(r#"{"nested":{"n":1}}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(r#"aria-label="Alternar sección JSON""#));
}

#[tokio::test]
async fn json_view_endpoint_rejects_invalid_or_oversized_json() {
    let invalid = app()
        .oneshot(
            Request::post("/ui/json-view/render")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let oversized = app_with_body_limit(16)
        .oneshot(
            Request::post("/ui/json-view/render")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"long":"01234567890123456789"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn production_mount_applies_the_default_json_view_body_limit() {
    let app = production_app();

    let normal = app
        .clone()
        .oneshot(
            Request::post("/ui/json-view/render")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"resourceType":"Patient"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(normal.status(), StatusCode::OK);

    let over_default_limit = format!(r#"{{"value":"{}"}}"#, "x".repeat(10 * 1024 * 1024));
    let oversized = app
        .oneshot(
            Request::post("/ui/json-view/render")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(over_default_limit))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
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
    let response = resources_app_with_metadata(&[("Patient", true), ("Observation", true)])
        .oneshot(Request::get("/ui/resources").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    // The type filter rail, the search builder, and the Create button.
    assert!(html.contains(r#"id="type-rail-list""#));
    assert!(html.contains(r#"id="saved-query-form""#));
    assert!(html.contains(r#"id="resource-create""#));
    assert!(html.contains(r#"data-create-eligible="true""#));
    // The Create button names the selected type (#605), defaulting to
    // Patient, and the builder's URL is pre-filled so the no-JS form already
    // shows the query the client also runs on load.
    assert!(html.contains("Create new Patient"));
    assert!(html.contains(r#"value="GET /Patient""#));
    // The client-side template for the label update on rail clicks (#605):
    // the literal `{type}` placeholder, not the interpolated per-request value.
    assert!(html.contains(r#"data-msg-create="Create new {type}""#));
    // The "Recently used" group (#603, server-rendered since #754/#755) is
    // present but hidden: no settings store is wired for this test's app, so
    // there is nothing stored to show (RF9).
    assert!(html.contains(r#"id="type-rail-recent""#));
    assert!(html.contains(r#"data-rail-page="resources""#));
    assert!(html.contains(r#"data-max-recent="5""#));
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
    let response = resources_app_with_metadata(&[("Patient", true), ("Observation", true)])
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
    // Debug-printed on failure so a byte-level mismatch (a stray CR, an
    // attribute drift) is visible in CI output instead of a blind false.
    let anchor = r#"data-type="Observation" data-full-name="Observation""#;
    // Explicit \n, not a raw literal spanning source lines: the literal must
    // not inherit whatever endings this file was checked out with.
    let expected = "data-type=\"Observation\" data-full-name=\"Observation\"\n   href=\"/ui/resources?type=Observation\" title=\"Observation\" aria-current=\"true\"";
    assert!(
        html.contains(expected),
        "rail entry mismatch; rendered around the anchor: {:?}",
        html.find(anchor).map(|i| &html[i..html.len().min(i + 220)]),
    );
    assert!(html.contains(r#"class="filter-rail" id="resources""#));
    // Create and the builder prefill both follow the deep-linked type.
    assert!(html.contains("Create new Observation"));
    assert!(html.contains(r#"value="GET /Observation""#));
}

#[tokio::test]
async fn resources_fail_closed_for_metadata_and_non_create_capabilities() {
    let response = app()
        .oneshot(Request::get("/ui/resources").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains(r#"id="resource-create""#));
    assert!(html.contains("Server capabilities are unavailable"));
    assert!(html.contains(r#"data-create-metadata="unavailable""#));
    assert!(html.contains(r#"data-create-eligible="false""#));

    let response = resources_app_with_metadata(&[("Patient", false)])
        .oneshot(Request::get("/ui/resources").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains("does not allow creating this resource type"));
    assert!(html.contains(r#"data-create-eligible="false""#));

    for statement in [
        serde_json::json!({"resourceType": "CapabilityStatement"}),
        serde_json::json!({
            "resourceType": "CapabilityStatement",
            "fhirVersion": "5.0.0",
            "rest": [{"mode": "server", "resource": [{
                "type": "Patient", "interaction": [{"code": "create"}]
            }]}]
        }),
        serde_json::json!({
            "resourceType": "CapabilityStatement",
            "fhirVersion": "4.0.1",
            "rest": [{"mode": "client", "resource": [{
                "type": "Patient", "interaction": [{"code": "create"}]
            }]}]
        }),
    ] {
        let response = resources_app_with_statement(statement)
            .oneshot(Request::get("/ui/resources").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let html = body_text(response).await;
        assert!(html.contains("Server capabilities are unavailable"));
        assert!(html.contains(r#"data-create-metadata="unavailable""#));
        assert!(html.contains(r#"data-create-eligible="false""#));
    }
}

#[tokio::test]
async fn resources_preserve_invalid_inputs_and_url_wins_over_type() {
    let app = resources_app_with_metadata(&[("Patient", true), ("Observation", true)]);

    for path in [
        "/ui/resources?type=patient",
        "/ui/resources?type=NoLongerValid",
        "/ui/resources?type=",
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let html = body_text(response).await;
        if path.ends_with("type=") {
            assert!(html.contains(r#"data-selected-type="Patient""#));
            assert!(html.contains(r#"data-create-eligible="true""#));
        } else {
            assert!(html.contains("not available in the selected FHIR version"));
            assert!(html.contains(r#"data-create-eligible="false""#));
        }
    }

    let response = app
        .clone()
        .oneshot(
            Request::get("/ui/resources?type=Observation&url=%2FPatient%3Fname%3DSmith")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains(r#"data-selected-type="Patient""#));
    assert!(html.contains(r#"value="GET /Patient?name=Smith""#));
    assert!(html.contains(r#"data-create-target="Patient""#));

    let response = app
        .oneshot(
            Request::get("/ui/resources?url=%2F%3Cimg%20src%3Dx%20onerror%3Dalert%281%29%3E")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(
        html.contains("onerror"),
        "the editable value must remain visible"
    );
    assert!(!html.contains("<img src=x"), "the value must stay escaped");
    assert!(html.contains(r#"data-selected-type="""#));
    assert!(html.contains(r#"data-create-eligible="false""#));
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
        "http://localhost:8080".to_string(),
        None,
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
    assert!(html.contains(r#"src="/ui/assets/json-view.js""#));
}

/// #679: the shared busy convention. The helper is a global asset loaded from
/// the layout before any page script, and the batch page pre-renders the
/// status region — a live region injected at busy time is not reliably
/// announced — with its labels riding on `data-msg-*` like the rest of the
/// page copy.
#[tokio::test]
async fn batch_page_carries_the_shared_busy_affordances() {
    let response = app()
        .oneshot(
            Request::get("/ui/assets/busy.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let js = body_text(response).await;
    assert!(js.contains("hfsBusy"));

    let response = app()
        .oneshot(Request::get("/ui/batch").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html = body_text(response).await;
    // The status region ships in the shell with its full shape pinned:
    // hidden, empty, role="status", the shared region class, and the spinner
    // + label pair the helper drives.
    assert!(
        html.contains(r#"<p class="busy-status batch-busy" id="batch-busy" role="status" hidden>"#)
    );
    assert!(html.contains(
        r#"<span class="spinner" aria-hidden="true"></span><span data-busy-label></span>"#
    ));
    // The rendered copy, not just the attribute name: a missing Fluent key
    // falls back to the key itself and would still carry the attribute.
    assert!(html.contains(r#"data-msg-reading="Reading bundle…""#));
    assert!(html.contains(r#"data-msg-executing="Executing…""#));
    assert!(html.contains(r#"data-msg-read-failed="The file could not be read.""#));
    // The stage that receives focus when the preflight appears (#679).
    assert!(html.contains(r#"<section id="batch-preflight" tabindex="-1" hidden>"#));
    // The helper loads from the shared layout, before the page script (both
    // defer, so document order is execution order).
    let busy = html
        .find(r#"src="/ui/assets/busy.js""#)
        .expect("busy.js in the layout");
    let batch = html
        .find(r#"src="/ui/assets/batch.js""#)
        .expect("batch.js in the page");
    assert!(busy < batch);
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

/// #649: SQL on FHIR is a top-level nav section whose five children are real
/// routes — the dead `nav-item--soon` placeholder is gone — and each page
/// answers 200 and marks its own nav entry current.
#[tokio::test]
async fn sql_on_fhir_section_navigates_to_real_pages() {
    let response = app()
        .oneshot(Request::get("/ui/batch").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(">SQL on FHIR</div>"));
    for href in [
        "/ui/sql/view-definitions",
        "/ui/sql/queries",
        "/ui/sql/views",
        "/ui/sql/export",
        "/ui/sql/files",
    ] {
        assert!(
            html.contains(&format!(r#"href="{href}""#)),
            "{href} missing from the nav"
        );
    }
    // No entry in the menu is a dead placeholder any more.
    assert!(!html.contains("nav-item--soon"));

    for href in [
        "/ui/sql/view-definitions",
        "/ui/sql/queries",
        "/ui/sql/views",
        "/ui/sql/export",
        "/ui/sql/files",
    ] {
        let response = app()
            .oneshot(Request::get(href).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{href}");
        let html = body_text(response).await;
        assert!(
            html.contains(&format!(r#"href="{href}" aria-current="page""#)),
            "{href} does not mark its nav entry current"
        );
    }
}

/// #649: SQL Export offers the stored subjects, follows a job by ?job= —
/// running with a cancel form, finished with a link to Files — and Files
/// tables a finished job's manifest as download links.
#[tokio::test]
async fn sql_export_and_files_follow_a_job_through_the_manifest() {
    let system = "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes";
    let manifest = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "exportId", "valueString": "job-9"},
            {"name": "_format", "valueCode": "csv"},
            {"name": "output", "part": [
                {"name": "name", "valueString": "patients"},
                {"name": "location", "valueUri": "http://s/export/job-9/patients-0.csv"},
            ]},
        ]
    });
    let source = helios_ui::StaticConformanceSource::empty()
        .with(
            "ViewDefinition",
            helios_fhir::FhirVersion::R4,
            vec![
                serde_json::json!({"resourceType": "ViewDefinition", "id": "vd1",
                "name": "patients", "resource": "Patient"}),
            ],
        )
        .with(
            "Library",
            helios_fhir::FhirVersion::R4,
            vec![
                serde_json::json!({"resourceType": "Library", "id": "q1", "name": "counts",
                "status": "active",
                "type": {"coding": [{"system": system, "code": "sql-query"}]}}),
            ],
        )
        .with_export_status(helios_ui::SqlExportStatus::Running(Some("2/3".to_string())))
        .with_export_manifest(Ok(manifest));
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
        "http://localhost:8080".to_string(),
        None,
    );

    // The form offers both stored subjects.
    let response = app
        .clone()
        .oneshot(Request::get("/ui/sql/export").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(r#"value="ViewDefinition/vd1""#));
    assert!(html.contains(r#"value="Library/q1""#));
    assert!(html.contains(
        r#"<form method="post" action="/ui/sql/export" class="card__body detail-stack">"#
    ));

    // Starting redirects to the job the gateway handed back.
    let response = app
        .clone()
        .oneshot(
            Request::post("/ui/sql/export")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("subject=ViewDefinition%2Fvd1&format=csv"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()["location"],
        "/ui/sql/export?job=static-job&started=1"
    );
    // No subject selected: the page explains instead of submitting.
    let response = app
        .clone()
        .oneshot(
            Request::post("/ui/sql/export")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("format=csv"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("at least one subject"));

    // A running job shows its progress and the cancel form.
    let response = app
        .clone()
        .oneshot(
            Request::get("/ui/sql/export?job=job-9")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains("2/3"));
    assert!(html.contains(r#"<div class="card__body">"#));
    assert!(html.contains(r#"<div class="kv-grid kv-grid--flush">"#));
    assert_eq!(
        html.matches(r#"class="detail__field detail__field--wide""#)
            .count(),
        2
    );
    assert!(html.contains("/ui/sql/export/cancel"));

    // Files tables the manifest with its download links.
    let response = app
        .clone()
        .oneshot(
            Request::get("/ui/sql/files?job=job-9")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(
        html.contains(
            r#"<form method="get" action="/ui/sql/files" class="card__body detail-stack">"#
        )
    );
    assert!(html.contains("patients"));
    assert!(html.contains(r#"href="http://s/export/job-9/patients-0.csv""#));
    assert!(html.contains(">csv<"));
}

/// #649: the SQL Queries and SQL Views workspaces list Libraries of their own
/// kind only, decode the SQL attachment into its editor pane, and save via a
/// plain form that re-embeds the SQL and redirects to the stored library.
#[tokio::test]
async fn sql_library_workspaces_split_kinds_and_roundtrip_sql() {
    let system = "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes";
    let libs = vec![
        serde_json::json!({"resourceType": "Library", "id": "q1", "name": "patient_counts",
            "status": "active",
            "type": {"coding": [{"system": system, "code": "sql-query"}]},
            "content": [{"contentType": "application/sql", "data": "U0VMRUNUIDE="}]}),
        serde_json::json!({"resourceType": "Library", "id": "v1", "name": "flat_patients",
            "status": "draft",
            "type": {"coding": [{"system": system, "code": "sql-view"}]}}),
    ];
    let source = helios_ui::StaticConformanceSource::empty().with(
        "Library",
        helios_fhir::FhirVersion::R4,
        libs,
    );
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
        "http://localhost:8080".to_string(),
        None,
    );

    // The Queries page lists only the sql-query Library and decodes its SQL.
    let response = app
        .clone()
        .oneshot(Request::get("/ui/sql/queries").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("patient_counts"));
    assert!(!html.contains("flat_patients"));
    assert!(html.contains("SELECT 1"));
    assert!(html.contains(r#"name="sql""#));
    assert!(html.contains(r#"data-type="Library""#));

    // The Views page holds the other kind.
    let response = app
        .clone()
        .oneshot(Request::get("/ui/sql/views").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains("flat_patients"));
    assert!(!html.contains("patient_counts"));

    // Save re-embeds the SQL pane and redirects to the stored library.
    let body = "id=&action=save&sql=SELECT%202&json=%7B%22resourceType%22%3A%22Library%22%2C%22name%22%3A%22x%22%7D";
    let response = app
        .clone()
        .oneshot(
            Request::post("/ui/sql/queries")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()["location"],
        "/ui/sql/queries?lib=static-created&saved=1"
    );

    // Bad JSON re-renders with both panes preserved.
    let response = app
        .clone()
        .oneshot(
            Request::post("/ui/sql/views")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("id=&action=save&sql=SELECT%203&json=%7Bnope"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("invalid JSON"));
    assert!(html.contains("{nope"));
    assert!(html.contains("SELECT 3"));
}

/// `application/x-www-form-urlencoded`-encodes `s`, byte by byte, so tests
/// can post arbitrary text (newlines, tabs, quotes, non-ASCII) without
/// pulling in a form-encoding crate just for this.
fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The inverse of Askama's default HTML auto-escaping, so a value read back
/// out of a rendered `<textarea>` can be compared against the exact text
/// that was posted.
fn html_unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The exact text between the SQL pane's `<textarea name="sql" ...>` open
/// tag and its `</textarea>` close tag, HTML-unescaped.
fn sql_textarea_value(html: &str) -> String {
    let open_tag_end = html
        .find(r#"name="sql""#)
        .and_then(|from| html[from..].find('>').map(|to| from + to + 1))
        .expect("a <textarea name=\"sql\"> open tag");
    let close = html[open_tag_end..]
        .find("</textarea>")
        .expect("a matching </textarea>");
    html_unescape(&html[open_tag_end..open_tag_end + close])
}

/// #838: arbitrary SQL text — newlines, a tab, single
/// quotes, a SQLite `:ward`-style bind parameter, a non-ASCII character, and
/// a `--` line comment — must round-trip byte for byte through both halves
/// of the SQL pane's plumbing: decoding the stored base64 attachment back
/// into the textarea (`extract_sql`, exercised via a GET), and posting the
/// textarea's exact text back through the form (`SqlLibSaveForm`, exercised
/// via a POST). `StaticConformanceSource::save_resource` is a stub that
/// echoes an id but does not persist (see `sql_library_workspaces_split_
/// kinds_and_roundtrip_sql` above, which stops at the redirect for the same
/// reason), so the POST half is proven the same way that test's own "bad
/// JSON" case already does: force the error-page branch, which re-renders
/// the exact `sql` field it was posted, in the same response, with no
/// storage round trip involved.
#[tokio::test]
async fn sql_editor_save_roundtrips_special_characters_byte_for_byte() {
    let sql = "SELECT *\nFROM patients\t-- niño's row\nWHERE ward = :ward\n";
    let system = "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes";
    let libs = vec![
        serde_json::json!({"resourceType": "Library", "id": "q1", "name": "special_chars",
        "status": "active",
        "type": {"coding": [{"system": system, "code": "sql-query"}]},
        "content": [{"contentType": "application/sql", "data": BASE64.encode(sql)}]}),
    ];
    let source = helios_ui::StaticConformanceSource::empty().with(
        "Library",
        helios_fhir::FhirVersion::R4,
        libs,
    );
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
        "http://localhost:8080".to_string(),
        None,
    );

    // Decode half: the stored attachment comes back exactly as-is.
    let response = app
        .clone()
        .oneshot(
            Request::get("/ui/sql/queries?lib=q1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert_eq!(sql_textarea_value(&html), sql);

    // Encode/parse half: an invalid-JSON save re-renders the same response
    // (no redirect, no storage) with the posted `sql` field preserved as-is.
    let body = format!("id=&action=save&sql={}&json=%7Bnope", form_encode(sql));
    let response = app
        .clone()
        .oneshot(
            Request::post("/ui/sql/queries")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("invalid JSON"));
    assert_eq!(sql_textarea_value(&html), sql);
}

/// #649: the View Definitions workspace lists stored views in the rail
/// (name-sorted, first selected), edits the selection as JSON, and offers
/// the starter document under Create New. #752 ticket 02, RF1: there is no
/// `?run=1` — it is no longer read by the handler and has no effect.
#[tokio::test]
async fn view_definitions_workspace_lists_edits_and_previews() {
    let vds = vec![
        serde_json::json!({"resourceType": "ViewDefinition", "id": "vd2", "name": "blood_pressure",
            "resource": "Observation",
            "select": [{"column": [{"name": "id", "path": "getResourceKey()"}]}]}),
        serde_json::json!({"resourceType": "ViewDefinition", "id": "vd1", "name": "active_patients",
            "resource": "Patient",
            "select": [{"column": [{"name": "id", "path": "getResourceKey()"},
                                    {"name": "family", "path": "name.family.first()"}]}]}),
    ];
    let source = helios_ui::StaticConformanceSource::empty()
        .with("ViewDefinition", helios_fhir::FhirVersion::R4, vds)
        .with_sql_run(Ok(vec![serde_json::json!({"family": "Doe", "id": "p1"})]));
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
        "http://localhost:8080".to_string(),
        None,
    );

    let response = app
        .clone()
        .oneshot(
            Request::get("/ui/sql/view-definitions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    // Both rail entries, and the name-sorted first one selected by default.
    assert!(html.contains(r#"href="/ui/sql/view-definitions?vd=vd1""#));
    assert!(html.contains(r#"href="/ui/sql/view-definitions?vd=vd2""#));
    assert!(html.contains("active_patients"));
    assert!(html.contains(r#"name="json""#));
    // Delete goes through the shared conformance CRUD script.
    assert!(html.contains(r#"data-crud-delete"#));
    assert!(html.contains("/ui/assets/conformance-crud.js"));
    // #752 ticket 02, RF1/RF2: no Run link, no `<details>` fold — the
    // editor card is always open with the "Runs as you type" legend, and
    // the results region's own empty notice is always present.
    assert!(!html.contains("run=1"));
    assert!(!html.contains("json-fold"));
    assert!(html.contains("editor-legend__live"));
    assert!(html.contains(r#"id="vd-run-notice""#));
    assert!(html.contains(r#"hx-post="/ui/sql/view-definitions/run""#));
    // RF4: no server-side results yet, so the notice's own empty shell
    // carries the initial-load trigger.
    assert!(html.contains(r#"hx-trigger="load""#));
    // RF3: no results card until something has actually run — only the
    // empty placeholder the first live fragment's OOB swap anchors onto.
    assert!(!html.contains("table-card"));
    assert!(html.contains(r#"<div id="vd-results"></div>"#));

    // RF1: `?run=1` is no longer read by the handler — no results card.
    let response = app
        .clone()
        .oneshot(
            Request::get("/ui/sql/view-definitions?vd=vd1&run=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(!html.contains("table-card"));

    // Create New offers the starter document in the editor.
    let response = app
        .clone()
        .oneshot(
            Request::get("/ui/sql/view-definitions?vd=new")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains("new_view"));
    assert!(html.contains("getResourceKey()"));
    // RF4/NF5: `?vd=new` selects the starter document, so the results
    // region's own load trigger fires for it exactly like a stored view.
    assert!(html.contains(r#"hx-trigger="load""#));
}

/// #752 ticket 02, RF6: `?vd=<id>&saved=1` (Save's own redirect) renders the
/// just-stored definition's `$sql-run` results server-side — the nojs path
/// to the playground's live preview. The success case's results card comes
/// with a working meta and no load trigger left behind (RF4: results
/// already present, so the client must not ask again); the failure case
/// shows `vd-run-failed` instead, with no results card.
#[tokio::test]
async fn view_definitions_saved_redirect_renders_results_server_side() {
    let vd = serde_json::json!({"resourceType": "ViewDefinition", "id": "vd1", "name": "active_patients",
        "resource": "Patient",
        "select": [{"column": [{"name": "id", "path": "getResourceKey()"}]}]});

    let source = helios_ui::StaticConformanceSource::empty()
        .with(
            "ViewDefinition",
            helios_fhir::FhirVersion::R4,
            vec![vd.clone()],
        )
        .with_sql_run(Ok(vec![serde_json::json!({"id": "p1"})]));
    let app = view_definitions_app(source);
    let response = app
        .oneshot(
            Request::get("/ui/sql/view-definitions?vd=vd1&saved=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(r#"id="vd-results""#));
    assert!(html.contains("<th>id</th>"));
    assert!(html.contains("<td>p1</td>"));
    let meta = text_between(
        &html,
        r#"id="vd-results-meta" class="card-head__meta">"#,
        "</span>",
    );
    assert!(meta.starts_with("1 rows"), "{meta}");
    // RF4: results already arrived server-side, so the notice must not also
    // carry the client-driven initial-load trigger.
    assert!(!html.contains(r#"hx-trigger="load""#));

    let source = helios_ui::StaticConformanceSource::empty()
        .with("ViewDefinition", helios_fhir::FhirVersion::R4, vec![vd])
        .with_sql_run(Err("boom".into()));
    let app = view_definitions_app(source);
    let response = app
        .oneshot(
            Request::get("/ui/sql/view-definitions?vd=vd1&saved=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("notice--warn"));
    assert!(html.contains("boom"));
    // RF7: no real results card on a failed run — just the anchor placeholder
    // a later successful edit's OOB swap needs (see the partial's own
    // header comment for why).
    assert!(!html.contains("table-card"));
    assert!(html.contains(r#"<div id="vd-results"></div>"#));
    assert!(!html.contains(r#"hx-trigger="load""#));
}

/// The text between two markers in `html`, panicking (with the marker named)
/// if either is missing — used by the `/run` fragment tests to read the
/// meta span's own text (`{ $rows } rows · { $ms } ms`) without depending on
/// the exact millisecond count a test run measures.
fn text_between<'a>(html: &'a str, start_marker: &str, end_marker: &str) -> &'a str {
    let start = html
        .find(start_marker)
        .unwrap_or_else(|| panic!("{start_marker} present in {html}"))
        + start_marker.len();
    let end = html[start..]
        .find(end_marker)
        .unwrap_or_else(|| panic!("{end_marker} present after {start_marker}"))
        + start;
    &html[start..end]
}

fn urlencoded_json_body(document: &serde_json::Value) -> String {
    form_urlencoded::Serializer::new(String::new())
        .append_pair("json", &document.to_string())
        .finish()
}

/// #752 ticket 01, RF4: the fragment endpoint runs the *posted* text, not a
/// stored resource — the playground's whole point. The success fragment
/// opens with an empty `#vd-run-notice`, then `#vd-results` carries its own
/// `hx-swap-oob`, with the view's declared column order, the canned row,
/// and a `{ $rows } rows · { $ms } ms` meta.
#[tokio::test]
async fn view_definitions_run_previews_the_posted_document_via_an_oob_fragment() {
    let source = helios_ui::StaticConformanceSource::empty()
        .with("ViewDefinition", helios_fhir::FhirVersion::R4, Vec::new())
        .with_sql_run(Ok(vec![serde_json::json!({"id": "p1", "family": "Doe"})]));
    let app = view_definitions_app(source);

    // Deliberately never stored (RF2/RF3: "contenido no guardado") — proves
    // the handler ran the request body, not a resource fetched by id.
    let vd = serde_json::json!({
        "resourceType": "ViewDefinition",
        "name": "unsaved_view",
        "status": "draft",
        "resource": "Patient",
        "select": [{"column": [
            {"name": "id", "path": "getResourceKey()"},
            {"name": "family", "path": "name.family.first()"}
        ]}]
    });

    let response = app
        .oneshot(
            Request::post("/ui/sql/view-definitions/run")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(urlencoded_json_body(&vd)))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    let html = body_text(response).await;
    assert!(html.contains(r#"<div id="vd-run-notice"></div>"#));
    assert!(html.contains(r#"id="vd-results""#));
    assert!(html.contains(r#"hx-swap-oob="outerHTML""#));
    assert!(html.contains("<th>id</th><th>family</th>"));
    assert!(html.contains("<td>p1</td><td>Doe</td>"));
    let meta = text_between(
        &html,
        r#"id="vd-results-meta" class="card-head__meta">"#,
        "</span>",
    );
    assert!(meta.starts_with("1 rows"), "{meta}");
    assert!(meta.ends_with(" ms"), "{meta}");
}

/// #752 ticket 01, RF5: a failed run answers `200` (NF3 — htmx never swaps
/// an error status) with the notice carrying the server's message, the meta
/// relabelled to "last successful run" via its own OOB swap, and no
/// `#vd-results` at all — the client's previous table is left alone.
#[tokio::test]
async fn view_definitions_run_reports_a_failed_run_without_a_results_card() {
    let source = helios_ui::StaticConformanceSource::empty().with_sql_run(Err("boom".into()));
    let app = view_definitions_app(source);
    let vd = serde_json::json!({
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "select": [{"column": [{"name": "id", "path": "getResourceKey()"}]}]
    });

    let response = app
        .oneshot(
            Request::post("/ui/sql/view-definitions/run")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(urlencoded_json_body(&vd)))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("boom"));
    assert!(html.contains(r#"<div id="vd-run-notice">"#));
    assert!(html.contains(
        r#"id="vd-results-meta" class="card-head__meta" hx-swap-oob="outerHTML">last successful run"#
    ));
    assert!(!html.contains(r#"id="vd-results""#));
}

/// #752 ticket 01, RF6: invalid JSON never reaches `$sql-run` — the seeded
/// rows would show up in the response if it had — and reports the parse
/// error in the same notice-only shape as a failed run, still `200`.
#[tokio::test]
async fn view_definitions_run_reports_invalid_json_without_calling_sql_run() {
    let source = helios_ui::StaticConformanceSource::empty()
        .with_sql_run(Ok(vec![serde_json::json!({"id": "p1"})]));
    let app = view_definitions_app(source);

    let response = app
        .oneshot(
            Request::post("/ui/sql/view-definitions/run")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("json=%7B"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("invalid JSON"));
    assert!(!html.contains(r#"id="vd-results""#));
    assert!(!html.contains("<table"));
    assert!(!html.contains("<td>p1</td>"));
}

/// #752 ticket 01, RF2: a body with no `json` field is the one case this
/// endpoint answers with a genuine error status — axum's own `Form`
/// rejection, not a hand-rolled one. `422 Unprocessable Entity`, not `400`:
/// axum's `Form` extractor reports a `POST` body it cannot deserialize (a
/// missing field included) as `422`, reserving `400` for a query-string
/// (`GET`) rejection or a request with the wrong content type — see
/// `axum::form::tests::deserialize_error_status_codes` (axum 0.8.4). Either
/// way it is a real 4xx, not the 2xx fragment contract RF4–RF6 describe.
#[tokio::test]
async fn view_definitions_run_without_a_json_field_is_unprocessable() {
    let app = view_definitions_app(helios_ui::StaticConformanceSource::empty());

    let response = app
        .oneshot(
            Request::post("/ui/sql/view-definitions/run")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("other=1"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// #752 ticket 01, RF4: a run with no output rows still renders the results
/// card — `data-table__empty` plus a `0 rows` meta — not the failure notice.
#[tokio::test]
async fn view_definitions_run_with_no_rows_renders_the_empty_state() {
    let source = helios_ui::StaticConformanceSource::empty().with_sql_run(Ok(Vec::new()));
    let app = view_definitions_app(source);
    let vd = serde_json::json!({
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "select": [{"column": [{"name": "id", "path": "getResourceKey()"}]}]
    });

    let response = app
        .oneshot(
            Request::post("/ui/sql/view-definitions/run")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(urlencoded_json_body(&vd)))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains(r#"class="data-table__empty""#));
    let meta = text_between(
        &html,
        r#"id="vd-results-meta" class="card-head__meta">"#,
        "</span>",
    );
    assert!(meta.starts_with("0 rows"), "{meta}");
}

/// #838: the vendored CodeMirror 6 bundle and the
/// shared `code-editor.js` mount helper load, in that order, on every page
/// that mounts a CodeMirror editor — `code-editor.js` reads
/// `window.HfsCodeMirror` at the top of its IIFE, so the bundle must load
/// first. Each page's own editor script (`vd-editor.js` on the
/// ViewDefinition page, `sql-editor.js` on the two SQL Library pages) reads
/// both `window.HfsCodeMirror` and `window.HfsCodeEditor`, so it must load
/// after the helper. The ViewDefinition page never loads `sql-editor.js`
/// and the SQL Library pages never load `vd-editor.js` — each page mounts
/// exactly one editor script. No other page (checked here: the dashboard
/// and the Resource Editor) mentions any of the three scripts.
#[tokio::test]
async fn sql_editor_and_vd_editor_scripts_load_only_on_their_own_pages() {
    // (route, the page's own editor script, the other page's editor script
    // that must NOT appear here).
    let editor_pages = [
        ("/ui/sql/view-definitions", "vd-editor.js", "sql-editor.js"),
        ("/ui/sql/queries", "sql-editor.js", "vd-editor.js"),
        ("/ui/sql/views", "sql-editor.js", "vd-editor.js"),
    ];
    for (route, own_editor, other_editor) in editor_pages {
        let response = app()
            .oneshot(Request::get(route).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{route}");
        let html = body_text(response).await;
        let own_script = format!(r#"<script src="/ui/assets/{own_editor}" defer></script>"#);
        assert!(
            html.contains(
                r#"<script src="/ui/assets/vendor/codemirror.bundle.js" defer></script>"#
            )
        );
        assert!(html.contains(r#"<script src="/ui/assets/code-editor.js" defer></script>"#));
        assert!(html.contains(&own_script), "{route} must load {own_editor}");
        assert!(
            !html.contains(other_editor),
            "{route} must not load {other_editor}"
        );

        // Positions are searched by full asset path, not bare filename: an
        // explanatory HTML comment earlier in the same template mentions
        // each script's bare name in prose (e.g. "must load before
        // sql-editor.js"), which would otherwise be found before the real
        // `<script src>` tag it is describing.
        let bundle_pos = html.find("/ui/assets/vendor/codemirror.bundle.js");
        let helper_pos = html.find("/ui/assets/code-editor.js");
        let own_editor_pos = html.find(&format!("/ui/assets/{own_editor}"));
        assert!(
            bundle_pos < helper_pos,
            "{route}: the CodeMirror bundle must load before code-editor.js"
        );
        assert!(
            helper_pos < own_editor_pos,
            "{route}: code-editor.js must load before {own_editor}"
        );
    }

    for other in ["/ui", "/ui/editor?type=Patient&id=abc"] {
        let response = app()
            .oneshot(Request::get(other).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{other}");
        let html = body_text(response).await;
        assert!(
            !html.contains("/ui/assets/vendor/codemirror.bundle.js"),
            "{other} must not load the CodeMirror bundle"
        );
        assert!(
            !html.contains("/ui/assets/code-editor.js"),
            "{other} must not load code-editor.js"
        );
        assert!(
            !html.contains("vd-editor.js"),
            "{other} must not load vd-editor.js"
        );
        assert!(
            !html.contains("sql-editor.js"),
            "{other} must not load sql-editor.js"
        );
    }
}

/// #753: `POST /ui/sql/view-definitions/lint` is the CodeMirror
/// linter's server call — plain JSON in, `{"diagnostics": [...]}` out, no
/// htmx swap (the precedent is `/ui/editor/expand`). The rule logic itself
/// belongs to `helios_sof::lint`; this only checks the handler's own
/// contract: status codes, the JSON envelope, and the kebab-case/`span`
/// serialization shape the browser depends on.
#[tokio::test]
async fn view_definitions_lint_returns_diagnostics_for_an_invalid_document() {
    let doc = serde_json::json!({
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "select": [{
            "column": [{ "name": "id", "path": "getResourceKey(" }]
        }],
        "notAField": "oops"
    });
    let response = app()
        .oneshot(
            Request::post("/ui/sql/view-definitions/lint")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(doc.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    let diagnostics = body["diagnostics"].as_array().expect("diagnostics array");
    assert!(diagnostics.len() >= 2, "{diagnostics:?}");

    let unknown_key = diagnostics
        .iter()
        .find(|d| d["code"] == "unknown-key")
        .expect("an unknown-key diagnostic for notAField");
    assert_eq!(unknown_key["pointer"], "/notAField");
    assert_eq!(unknown_key["severity"], "error");
    assert!(unknown_key["span"].is_null());

    let syntax = diagnostics
        .iter()
        .find(|d| d["code"] == "fhirpath-syntax")
        .expect("a fhirpath-syntax diagnostic for the unclosed call");
    assert_eq!(syntax["pointer"], "/select/0/column/0/path");
    assert_eq!(syntax["severity"], "error");
    assert!(syntax["span"].is_object());
    assert!(syntax["span"]["start"].is_u64());
    assert!(syntax["span"]["end"].is_u64());
}

#[tokio::test]
async fn view_definitions_lint_returns_no_diagnostics_for_a_valid_document() {
    let doc = serde_json::json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [{ "name": "id", "path": "getResourceKey()" }]
        }]
    });
    let response = app()
        .oneshot(
            Request::post("/ui/sql/view-definitions/lint")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(doc.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(body["diagnostics"], serde_json::json!([]));
}

#[tokio::test]
async fn view_definitions_lint_rejects_a_non_json_body() {
    let response = app()
        .oneshot(
            Request::post("/ui/sql/view-definitions/lint")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("not json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    let error = body["error"].as_str().expect("error message");
    assert!(error.starts_with("invalid JSON: "), "{error}");
}

/// #649: Save is a plain form post — a valid document redirects to the stored
/// view, a broken one re-renders with the submitted text preserved so nothing
/// typed is lost.
#[tokio::test]
async fn view_definitions_save_roundtrips_and_rejects_bad_json() {
    let source = helios_ui::StaticConformanceSource::empty().with(
        "ViewDefinition",
        helios_fhir::FhirVersion::R4,
        Vec::new(),
    );
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
        "http://localhost:8080".to_string(),
        None,
    );

    let body = "id=&action=save&json=%7B%22resourceType%22%3A%22ViewDefinition%22%2C%22name%22%3A%22x%22%7D";
    let response = app
        .clone()
        .oneshot(
            Request::post("/ui/sql/view-definitions")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()["location"],
        "/ui/sql/view-definitions?vd=static-created&saved=1"
    );

    let response = app
        .clone()
        .oneshot(
            Request::post("/ui/sql/view-definitions")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("id=&action=save&json=%7Bnope"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("invalid JSON"));
    assert!(html.contains("{nope"));
}

fn view_definitions_app(source: helios_ui::StaticConformanceSource) -> Router {
    helios_ui::mount_with_conformance_source(
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
        "http://localhost:8080".to_string(),
        None,
    )
}

/// The `id="vd-rail-list"` (or `id="lib-rail-list"`) scrollable list's own
/// HTML — its opening tag up to the closing `</div>` immediately after it,
/// with no nested `<div>` in between (the list holds only `<a>` items and,
/// when empty, a `<p>`) — so a test can assert what the paginated/filtered
/// list shows without also matching the "Recently used" group above it,
/// which (ticket 03) can legitimately render an id the list itself excludes
/// (RF4: the group is never filtered).
fn rail_list_html<'a>(html: &'a str, list_id: &str) -> &'a str {
    let start = html
        .find(&format!(r#"id="{list_id}""#))
        .unwrap_or_else(|| panic!("{list_id} present"));
    let end = html[start..]
        .find("</div>")
        .map(|i| i + start)
        .unwrap_or_else(|| panic!("{list_id} closing tag present"));
    &html[start..end]
}

/// #741: the rail is one page of a server-side search, not the whole
/// tenant collection — with more than one page of stored views, page 1 holds
/// the first 50 (name-sorted) with a "next" link, and page 2 holds the rest
/// with a "previous" link and no "next". Both links preserve the (empty)
/// filter.
#[tokio::test]
async fn view_definitions_rail_paginates_across_pages_of_fifty() {
    let vds: Vec<Value> = (1..=52)
        .map(|n| {
            serde_json::json!({"resourceType": "ViewDefinition", "id": format!("vd{n:03}"),
                "name": format!("vd_{n:03}"), "resource": "Patient"})
        })
        .collect();
    let source = helios_ui::StaticConformanceSource::empty().with(
        "ViewDefinition",
        helios_fhir::FhirVersion::R4,
        vds,
    );
    let app = view_definitions_app(source);

    let response = app
        .clone()
        .oneshot(
            Request::get("/ui/sql/view-definitions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains(r#"data-type="vd001""#),
        "first page holds vd001"
    );
    assert!(
        html.contains(r#"data-type="vd050""#),
        "first page holds vd050"
    );
    assert!(
        !html.contains(r#"data-type="vd051""#),
        "first page stops at 50"
    );
    assert!(
        html.contains(r#"href="/ui/sql/view-definitions?page=2""#),
        "a next link to page 2"
    );

    let response = app
        .oneshot(
            Request::get("/ui/sql/view-definitions?page=2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        !html.contains(r#"data-type="vd050""#),
        "the second page does not repeat the first"
    );
    assert!(
        html.contains(r#"data-type="vd051""#),
        "second page holds vd051"
    );
    assert!(
        html.contains(r#"data-type="vd052""#),
        "second page holds vd052"
    );
    // Page 1 is the implicit default, so the "previous" link omits `?page=`
    // entirely rather than spelling out `page=1`.
    assert!(
        html.contains(r#"class="pagination""#)
            && html.contains(r#"href="/ui/sql/view-definitions""#),
        "a previous link back to the bare route (page 1)"
    );
    assert!(!html.contains("page=3"), "no next link past the last page");
}

/// #741: the search box filters server-side by name only — a substring that
/// matches none of the stored names (even one that matches a resource type
/// column, the old in-memory filter's now-removed extra match) narrows the
/// rail to nothing, case-insensitively.
#[tokio::test]
async fn view_definitions_rail_filters_by_name_case_insensitively() {
    let vds = vec![
        serde_json::json!({"resourceType": "ViewDefinition", "id": "vd1",
            "name": "Active_Patients", "resource": "Patient"}),
        serde_json::json!({"resourceType": "ViewDefinition", "id": "vd2",
            "name": "blood_pressure", "resource": "Observation"}),
    ];
    let source = helios_ui::StaticConformanceSource::empty().with(
        "ViewDefinition",
        helios_fhir::FhirVersion::R4,
        vds,
    );
    let app = view_definitions_app(source);

    // A mixed-case substring of the name matches, case-insensitively.
    let response = app
        .clone()
        .oneshot(
            Request::get("/ui/sql/view-definitions?filter=PATIENT")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains(r#"data-type="vd1""#));
    assert!(!html.contains(r#"data-type="vd2""#));

    // "Observation" matches vd2's resource type but neither stored name —
    // the retired in-memory filter used to match this, `name:contains` does
    // not.
    let response = app
        .oneshot(
            Request::get("/ui/sql/view-definitions?filter=Observation")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains(r#"class="filter-rail__heading filter-rail__heading--group""#));
    assert!(!html.contains(r#"data-type="vd1""#));
    assert!(!html.contains(r#"data-type="vd2""#));
}

/// #741: a `?vd=` the current filter excludes from the rail still loads
/// through the direct-by-id read (ticket 01) — selection is independent of
/// what the rail happens to show.
#[tokio::test]
async fn view_definitions_selection_survives_a_filter_that_excludes_it() {
    let vds = vec![
        serde_json::json!({"resourceType": "ViewDefinition", "id": "keep",
            "name": "keep_me", "resource": "Patient"}),
        serde_json::json!({"resourceType": "ViewDefinition", "id": "other",
            "name": "exclude_me", "resource": "Patient"}),
    ];
    let source = helios_ui::StaticConformanceSource::empty().with(
        "ViewDefinition",
        helios_fhir::FhirVersion::R4,
        vds,
    );
    let app = view_definitions_app(source);

    let response = app
        .oneshot(
            Request::get("/ui/sql/view-definitions?vd=keep&filter=exclude")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    // The rail's scrollable list shows only what the filter matches...
    let list = rail_list_html(&html, "vd-rail-list");
    assert!(list.contains(r#"data-type="other""#));
    assert!(!list.contains(r#"data-type="keep""#));
    // ...but the just-selected "keep" still surfaces through the "Recently
    // used" group above it (ticket 03, RF4: the group is never itself
    // filtered) — from its snapshot, since the filter takes it off the list
    // this render shows.
    assert!(html.contains(r#"id="vd-rail-recent""#));
    // ...and the editor still holds the selected view the filter excluded.
    assert!(html.contains(r#"name="json""#));
    assert!(html.contains("keep_me"));
}

/// #741: a failed rail search shows the same degradation notice the page has
/// always shown for a failed fetch — never a 500, and never a stale
/// full-collection fallback.
#[tokio::test]
async fn view_definitions_rail_degrades_when_search_fails() {
    struct FailingSearchSource;

    #[async_trait::async_trait]
    impl helios_ui::ConformanceSource for FailingSearchSource {
        async fn fetch(
            &self,
            _resource_type: &str,
            _version: helios_fhir::FhirVersion,
            _tenant: &str,
        ) -> Result<Vec<Value>, String> {
            Ok(Vec::new())
        }
        // `search_page` falls back to the trait's default `Err`.
    }

    let app = helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl(true, true),
        None,
        None,
        "default".to_string(),
        std::sync::Arc::new(FailingSearchSource),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    );

    let response = app
        .oneshot(
            Request::get("/ui/sql/view-definitions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("The view definition list could not be loaded."));
    assert!(html.contains("search is not available from this source"));
}

#[tokio::test]
async fn user_menu_carries_language_and_the_signed_out_state() {
    // #725: the avatar is a <details> menu holding the language selector and
    // the identity block; the inline lang-switcher nav is gone. /ui has no
    // signed-in principal (#320), so the local-operator state renders and no
    // Sign out row exists.
    let response = app()
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("menu--user"));
    assert!(!html.contains("lang-switcher"));
    for lang in ["en", "es", "de"] {
        assert!(
            html.contains(&format!("?lang={lang}")),
            "missing ?lang={lang}"
        );
    }
    // English is the negotiated default: its option is current, once.
    assert!(html.contains(r#"href="?lang=en" aria-current="true""#));
    assert!(!html.contains(r#"href="?lang=es" aria-current="true""#));
    assert!(html.contains("Anonymous user"));
    assert!(html.contains("Authentication is disabled"));
    assert!(!html.contains("/ui/logout"));
}

/// The rendered bytes of the account menu, pinned.
///
/// `tests/golden/user-menu-en.html` was captured from the **pristine tree**, in
/// `42974c22a`, before #799 lifted the block out of
/// `crates/ui/templates/layouts/base.html` into `crates/ui-chrome`. So a green
/// here is the proof that the extraction changed nothing: what `/ui` serves
/// today is byte-identical to what it served when the markup was still inline.
///
/// Checked in with `text eol=lf` (see `.gitattributes`), and `body_text`
/// normalizes the response the same way, so this holds on a Windows checkout
/// too (#671).
#[tokio::test]
async fn user_menu_fragment_is_stable() {
    const GOLDEN: &str = include_str!("golden/user-menu-en.html");

    let response = app()
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains(GOLDEN),
        "the account menu's rendered bytes moved.\n\n\
         This fragment is no longer HFS's alone: since #799 it is produced by \
         `helios-ui-chrome` and spliced into *both* products' topbars, so \
         whatever drifted here has already shipped to HTS as well. Do not \
         re-record the golden to make this green — first decide whether the \
         change was intended for both UIs. If it was, update \
         `crates/ui-chrome`, re-capture `tests/golden/user-menu-en.html` from \
         the rendered page, and say so in the commit.\n\n\
         Expected to find:\n{GOLDEN}",
    );
}

/// The page's account menu *is* the shared component's output — not a
/// look-alike.
///
/// Rendering `helios_ui_chrome::user_menu` here and demanding the page contain
/// it verbatim is stricter than the golden: the golden would still pass if a
/// future edit re-inlined equivalent markup into the layout and left the shared
/// crate unused.
///
/// Its twin lives in `crates/hts-ui/tests/chrome_parity.rs` (Track G) and
/// asserts the same function's output against the HTS page. Neither test knows
/// about the other crate, yet together they are a transitive byte-identity
/// proof — HFS == `user_menu(..)` == HTS — with no cross-crate dev-dependency
/// and no second golden to keep in sync.
#[tokio::test]
async fn the_account_menu_is_the_shared_component_verbatim() {
    // `RequestLocale::default()` is `en`, which is also what `/ui` negotiates
    // for a request carrying no `?lang=`, cookie, or `Accept-Language`.
    let i18n = helios_ui::I18n::new(helios_ui::RequestLocale::default());
    // The signed-out shape (#320): `can_logout` defaults to false, so the
    // Sign out row does not render and `logout_href` is inert — it is spelled
    // out because it is what `Status::user_menu` passes in production.
    let expected = helios_ui_chrome::user_menu(
        &i18n,
        helios_ui_chrome::UserIdentity {
            logout_href: "/ui/logout",
            ..Default::default()
        },
    )
    .expect("the shared user-menu template has no fallible construct")
    .replace("\r\n", "\n");

    let response = app()
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains(&expected),
        "the /ui topbar does not contain `helios_ui_chrome::user_menu(..)` \
         verbatim — the account menu has been re-inlined into \
         `crates/ui/templates/layouts/base.html`, or the layout is passing a \
         different `UserIdentity` than the signed-out one.\n\n\
         Expected to find:\n{expected}",
    );
}
