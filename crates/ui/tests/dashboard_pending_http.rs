//! #956: the Dashboard's three degraded states, rendered over HTTP.
//!
//! The bug this pins down: switching the time window during an import made the
//! page show a fabricated Patient/Observation/Encounter/Condition curve under
//! the notice "no live metrics provider is registered on this build" — while a
//! provider was registered and merely slow. The window is part of the snapshot
//! cache key, so every window switch is a cold key and takes the timeout path.
//!
//! Registers its own [`DashboardProvider`] (process-global state), so this
//! lives in its own test binary. The provider discriminates on the window,
//! which is also the cache key, so the three cases below never interfere:
//!
//! - `1h` never answers (it sleeps past the end of the test), so it is
//!   permanently the cold-timeout case;
//! - `24h` answers with `partial` set, the half-failed case;
//! - `30d` answers completely.

use async_trait::async_trait;
use axum::{Router, body::Body, http::Request};
use chrono::{DateTime, Utc};
use helios_observability::dashboard::{
    DashboardPoint, DashboardProvider, DashboardSeries, DashboardSnapshot, DashboardWindow,
    TypeCount, set_provider,
};
use http_body_util::BodyExt;
use tower::ServiceExt;

struct WindowScriptedProvider;

#[async_trait]
impl DashboardProvider for WindowScriptedProvider {
    async fn snapshot(
        &self,
        window: DashboardWindow,
        _tenant: &str,
        _types: &[String],
        _include_empty: bool,
    ) -> DashboardSnapshot {
        if window == DashboardWindow::LastHour {
            // Far past the 800ms cold-load budget and past the test's own
            // lifetime, so this key stays pending however often it is asked
            // for — the storage contention the real bug happens under, held
            // still.
            tokio::time::sleep(std::time::Duration::from_secs(3_600)).await;
        }

        let bucket_start: DateTime<Utc> = DateTime::from_timestamp(1_752_451_200, 0).unwrap();
        DashboardSnapshot {
            fhir_version: "R4".to_string(),
            total_resources: 5,
            distinct_types: 1,
            window,
            series: vec![DashboardSeries {
                resource_type: "Patient".to_string(),
                total: 5,
                points: vec![DashboardPoint {
                    bucket_start,
                    delta: 5,
                    cumulative: 5,
                }],
            }],
            available: vec![TypeCount {
                resource_type: "Patient".to_string(),
                total: 5,
            }],
            export_jobs: None,
            import_jobs_active: None,
            partial: window == DashboardWindow::LastDay,
        }
    }
}

fn app() -> Router {
    set_provider(std::sync::Arc::new(WindowScriptedProvider));
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
        "http://localhost:8080".to_string(),
        None,
    )
}

async fn get(uri: &str) -> String {
    let response = app()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// The regression itself: a window whose snapshot has not landed says it is
/// waiting — it does not claim the build has no metrics, and it invents
/// nothing.
#[tokio::test]
async fn a_slow_window_renders_waiting_not_sample_data() {
    let html = get("/ui?window=1h").await;

    assert!(
        html.contains("Still gathering the live figures"),
        "the waiting notice should render"
    );
    assert!(
        !html.contains("no live metrics provider"),
        "a registered-but-slow provider must not be reported as absent"
    );
    // The sample curve's headline figures (sample_snapshot: 142 distinct
    // types, ~1.2k Patients) must be nowhere on the page.
    assert!(
        !html.contains(r#"class="stat__value">142<"#),
        "the invented distinct-type count must not appear"
    );
    assert!(
        !html.contains("chart-data"),
        "nothing is charted while waiting, so no chart data carrier is emitted"
    );
    // The three snapshot-derived figures render as unknown, not as zero.
    assert!(
        html.matches("stat__value--unavailable").count() >= 3,
        "resource types, stored resources and the chart total all read as \
         unknown while waiting, got: {}",
        html.matches("stat__value--unavailable").count()
    );
}

/// Waiting is recoverable both ways: htmx re-requests the live region on its
/// own, and the notice always carries a plain link for a browser without it.
#[tokio::test]
async fn the_waiting_page_offers_an_automatic_and_a_manual_retry() {
    let html = get("/ui?window=1h").await;

    assert!(html.contains(r#"id="dash-live""#));
    assert!(
        html.contains("hx-select=\"#dash-live\""),
        "the auto-refresh swaps just the live region"
    );
    assert!(
        html.contains("retry=1"),
        "the first attempt asks for the second"
    );
    assert!(html.contains("Retry now"), "the no-JS way out is present");
}

/// The auto-refresh is budgeted: the page stops re-requesting itself after a
/// bounded number of attempts, leaving only the manual link. The wait is
/// caused by load, so the recovery must not add to it indefinitely.
#[tokio::test]
async fn the_automatic_retry_stops_after_its_budget() {
    let html = get("/ui?window=1h&retry=3").await;

    assert!(
        html.contains("Still gathering the live figures"),
        "still waiting"
    );
    assert!(
        !html.contains("hx-trigger=\"load"),
        "the budget is spent, so nothing re-requests itself"
    );
    assert!(
        html.contains("Retry now"),
        "the manual retry outlives the budget"
    );
}

/// A snapshot the provider had to fill in is labelled, so its zeros are never
/// read as measurements — and it is not confused with either of the other two
/// states.
#[tokio::test]
async fn a_partial_snapshot_says_so() {
    let html = get("/ui?window=24h").await;

    assert!(html.contains("Some figures could not be read from storage"));
    assert!(!html.contains("no live metrics provider"));
    assert!(!html.contains("Still gathering the live figures"));
    // Unlike the waiting page, the figures it does have are shown.
    assert!(html.contains("chart-data"));
}

/// A complete snapshot carries no notice at all — the states above must not
/// leak into the ordinary page.
#[tokio::test]
async fn a_complete_snapshot_carries_no_notice() {
    let html = get("/ui?window=30d").await;

    assert!(!html.contains("no live metrics provider"));
    assert!(!html.contains("Still gathering the live figures"));
    assert!(!html.contains("Some figures could not be read from storage"));
    assert!(!html.contains("notice notice--warn"));
    assert!(html.contains("chart-data"), "the chart renders");
}
