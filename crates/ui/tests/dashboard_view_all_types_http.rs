//! "View all resources" (#599): the dashboard chart picker's `?all=1` toggle.
//!
//! Registers its own [`helios_observability::dashboard::DashboardProvider`]
//! (process-global state), so — like `dashboard_uptime_http.rs` — this lives
//! in its own test binary rather than sharing one with `router_http.rs`.
//!
//! The fake provider only ever reports `Patient` as stored; the shipped `data/`
//! CompartmentDefinition bundle (the source `resource_type_names()` reads,
//! same as the Search/Queries/Resources/Bulk Export pickers) lists the full R4
//! resource set, including plenty of types `Patient` never touches — this
//! test uses `Observation` as one of those never-stored types.

use async_trait::async_trait;
use axum::{Router, body::Body, http::Request};
use chrono::{DateTime, Utc};
use helios_observability::dashboard::{
    DashboardPoint, DashboardProvider, DashboardSeries, DashboardSnapshot, DashboardWindow,
    TypeCount, set_provider,
};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Reports exactly one stored type (`Patient`, total 5). Honors the selection
/// guard the way `helios_rest::dashboard::StorageDashboardProvider` does:
/// without `include_empty` an unstored requested type is dropped; with it,
/// it is accepted and charted as a dense, flat-zero series.
struct FakeProvider;

const STORED: &str = "Patient";

#[async_trait]
impl DashboardProvider for FakeProvider {
    async fn snapshot(
        &self,
        window: DashboardWindow,
        _tenant: &str,
        types: &[String],
        include_empty: bool,
    ) -> DashboardSnapshot {
        let available = vec![TypeCount {
            resource_type: STORED.to_string(),
            total: 5,
        }];
        // distinct_types must never move with the flag — it counts only what
        // is actually stored.
        let distinct_types = available.len();

        let selection: Vec<String> = if types.is_empty() {
            vec![STORED.to_string()]
        } else {
            types
                .iter()
                .filter(|t| include_empty || t.as_str() == STORED)
                .take(6)
                .cloned()
                .collect()
        };

        let bucket_start: DateTime<Utc> = DateTime::from_timestamp(1_752_451_200, 0).unwrap();
        let series = selection
            .into_iter()
            .map(|rt| {
                let stored = rt == STORED;
                DashboardSeries {
                    resource_type: rt,
                    total: if stored { 5 } else { 0 },
                    points: vec![DashboardPoint {
                        bucket_start,
                        delta: 0,
                        cumulative: if stored { 5 } else { 0 },
                    }],
                }
            })
            .collect();

        DashboardSnapshot {
            fhir_version: "R4".to_string(),
            total_resources: 5,
            distinct_types,
            window,
            series,
            available,
            export_jobs: None,
            import_jobs_active: None,
        }
    }
}

fn app() -> Router {
    set_provider(std::sync::Arc::new(FakeProvider));
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        helios_ui::NlSearch {
            enabled: false,
            configured: false,
            model: String::new(),
        },
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

/// Without the flag, the picker only ever offers what the tenant stores —
/// today's behavior, unchanged.
#[tokio::test]
async fn without_the_flag_the_picker_offers_only_stored_types() {
    let response = app()
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html = body_text(response).await;

    assert!(html.contains(r#"data-pick-name="Patient""#));
    assert!(!html.contains(r#"data-pick-name="Observation""#));
    // The toggle itself renders, off, and links to `?all=1`.
    assert!(html.contains("all=1"));
}

/// With `?all=1`, the picker's option list is the union of stored types and
/// every other resource type of the active FHIR version, offered at `0`.
#[tokio::test]
async fn the_flag_offers_every_type_of_the_version_with_zero_for_the_unstored_ones() {
    let response = app()
        .oneshot(Request::get("/ui?all=1").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html = body_text(response).await;

    assert!(html.contains(r#"data-pick-name="Patient""#));
    assert!(
        html.contains(r#"data-pick-name="Observation""#),
        "a never-stored type from the version's full list is offered"
    );
    // The unstored option's count renders as a real 0, not blank/omitted.
    // Scoped to the option's own `<a>...</a>` so this cannot accidentally
    // match a "0" appearing anywhere later in the page.
    let after_name = html
        .split(r#"data-pick-name="Observation""#)
        .nth(1)
        .expect("Observation option present");
    let option_html = after_name
        .split("</a>")
        .next()
        .expect("the option row closes with </a>");
    assert!(
        option_html.contains(r#"chart-pick__count">0</span>"#),
        "Observation's picker count should render 0, got: {option_html}"
    );
}

/// Selecting a never-stored type with the flag on charts it: the series is
/// present (a real `<polyline>`), not silently dropped, and the stat card
/// ("Resource types") still reflects only what is actually stored.
#[tokio::test]
async fn selecting_an_unstored_type_with_the_flag_charts_a_flat_series() {
    let response = app()
        .oneshot(
            Request::get("/ui?types=Observation&all=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;

    assert!(
        html.contains(r#"<polyline class="series"#),
        "the flat zero series is drawn, not the empty state"
    );
    assert!(html.contains("Observation"));
    // distinct_types is unaffected by the flag or the selection: still 1.
    assert!(html.contains(r#"<span class="stat__value">1</span>"#));
}

/// Without the flag, an unstored `?types=` selection is dropped — today's
/// behavior, unaffected by this feature.
#[tokio::test]
async fn selecting_an_unstored_type_without_the_flag_is_dropped() {
    let response = app()
        .oneshot(
            Request::get("/ui?types=Observation")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;

    assert!(
        !html.contains(r#"<polyline class="series"#),
        "an unstored type with no flag charts nothing"
    );
}
