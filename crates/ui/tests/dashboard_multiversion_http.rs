//! T3 (#600): the dashboard's "View all resources" picker (#599) offers the
//! *requested* FHIR version's resource-type set, not whichever version the
//! server booted with — end to end through the same `POST /ui/version` the
//! sidebar submits.
//!
//! This is the cheap Rust ring only. [`helios_ui::StaticConformanceSource`]
//! resolves a version's compartment bundle directly from the shipped spec
//! files (`crates/ui/src/conformance.rs`), so it *always* honors whichever
//! version it is asked for — this ring proves the plumbing (the sidebar
//! selector enumerates every compiled version; `index()` threads the
//! resolved request version, not the server default, into
//! `resource_type_names()` when building the "View all resources" union) but
//! it cannot reproduce the "always the seed" regression a real multi-version
//! binary can exhibit against its own self-fetched conformance endpoint —
//! `HttpConformanceSource` fetching over HTTP is exactly the path this double
//! bypasses. That gap is what the e2e ring covers instead, against a real
//! `--features R4,R4B,R5,R6` build:
//! `crates/ui/e2e/tests/dashboard-multiversion.spec.ts`.

use std::sync::Arc;

#[cfg(feature = "R4B")]
use axum::http::{StatusCode, header};
use axum::{Router, body::Body, http::Request};
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::SettingsStore;
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Builds the UI router with a real (in-memory) settings store, so a
/// `POST /ui/version` persists and later requests observe it — exactly the
/// sidebar's own round trip. Build once per test and `.clone()` the `Router`
/// per request (it is `Arc`-backed state).
fn app() -> Router {
    let backend = Arc::new({
        let b = SqliteBackend::in_memory().expect("in-memory sqlite");
        b.init_schema().expect("init schema");
        b
    });
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        helios_ui::NlSearch::default(),
        None,
        Some(backend as Arc<dyn SettingsStore>),
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        FhirVersion::R4,
        None,
    )
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// The sidebar's FHIR-version disclosure lists every version this binary was
/// compiled with (#343 plumbing) — the picker's coverage floor. On the
/// default (`R4`-only) build this is a single entry; the multi-version legs
/// (`--features R4,R4B,R5,R6`) grow it to four without any change here.
#[tokio::test]
async fn sidebar_lists_every_compiled_fhir_version() {
    let response = app()
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html = body_text(response).await;

    for version in FhirVersion::enabled_versions() {
        let label = version.as_str();
        assert!(
            html.contains(&format!(r#"value="{label}">"#)),
            "sidebar version selector is missing the compiled-in {label} option"
        );
    }
}

/// #600: the dashboard's "View all resources" union (#599) follows the
/// *request's* resolved FHIR version, not the server's default, through the
/// same `POST /ui/version` the sidebar submits. `Ingredient` joined the spec
/// in R4B; `EffectEvidenceSynthesis` left after R4 — the same pair
/// `tenants_http.rs::version_choice_changes_the_resource_type_lists` pins for
/// the Queries rail, reused here so a spec refresh that retires the pair
/// fails loudly in one place, not two silently-drifting ones.
#[cfg(feature = "R4B")]
#[tokio::test]
async fn all_types_picker_follows_the_requested_version_not_the_seed() {
    let app = app();

    // Default version (R4): the picker's full-set union carries R4's types.
    let response = app
        .clone()
        .oneshot(Request::get("/ui?all=1").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains(r#"data-pick-name="EffectEvidenceSynthesis""#));
    assert!(!html.contains(r#"data-pick-name="Ingredient""#));

    // Select R4B through the sidebar's own endpoint.
    let response = app
        .clone()
        .oneshot(
            Request::post("/ui/version")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("version=R4B"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    // Same route, now under R4B: the union follows the request, not the seed.
    let response = app
        .oneshot(Request::get("/ui?all=1").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains(r#"data-pick-name="Ingredient""#));
    assert!(!html.contains(r#"data-pick-name="EffectEvidenceSynthesis""#));
}
