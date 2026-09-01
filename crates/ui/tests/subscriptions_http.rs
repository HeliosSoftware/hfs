//! The Subscriptions operator page (#580), over the mounted router.
//!
//! One test, three phases in order — the read path is a process-global
//! provider, so the unavailable state must be asserted before any provider is
//! registered, and the phases cannot run as separate (parallel) tests.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;

use helios_observability::subscriptions::{
    SubscriptionRow, SubscriptionsProvider, SubscriptionsSnapshot, set_provider,
};

fn app() -> Router {
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
        Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    )
}

async fn get(path: &str) -> (StatusCode, String) {
    let response = app()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

struct Fixed;

impl SubscriptionsProvider for Fixed {
    fn snapshot(&self, tenant: &str) -> SubscriptionsSnapshot {
        if tenant != "default" {
            return SubscriptionsSnapshot::default();
        }
        SubscriptionsSnapshot {
            rows: vec![
                SubscriptionRow {
                    id: "encounter-start".into(),
                    topic_url: "http://example.org/topics/encounter-start".into(),
                    channel_type: "rest-hook".into(),
                    endpoint: Some("https://ehr-bridge.example.org/hook".into()),
                    status: "active".into(),
                    events_since_start: 4182,
                    consecutive_failures: 0,
                    ws_clients: None,
                    delivered_24h: 4182,
                    first_try_24h: 4031,
                    failed_24h: 0,
                    delivered_series: vec![80, 95, 90, 110, 120, 100, 115],
                },
                SubscriptionRow {
                    id: "obs-critical".into(),
                    topic_url: "http://example.org/topics/obs-critical".into(),
                    channel_type: "rest-hook".into(),
                    endpoint: Some("https://pager-svc.example.org/hook".into()),
                    status: "error".into(),
                    events_since_start: 318,
                    consecutive_failures: 7,
                    ws_clients: None,
                    delivered_24h: 311,
                    first_try_24h: 290,
                    failed_24h: 7,
                    delivered_series: vec![40, 35, 20, 8, 0, 0, 0],
                },
                SubscriptionRow {
                    id: "admit-feed".into(),
                    topic_url: "http://example.org/topics/admit-feed".into(),
                    channel_type: "websocket".into(),
                    endpoint: None,
                    status: "active".into(),
                    events_since_start: 0,
                    consecutive_failures: 0,
                    ws_clients: Some(0),
                    delivered_24h: 0,
                    first_try_24h: 0,
                    failed_24h: 0,
                    delivered_series: Vec::new(),
                },
            ],
        }
    }
}

#[tokio::test]
async fn the_page_goes_from_unavailable_to_the_live_dashboard() {
    // Phase 1 — no provider registered: the nav entry is present anyway
    // (#767) and the page explains what to do, naming the switch and the
    // build feature rather than a bare "unavailable".
    let (status, html) = get("/ui/subscriptions").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains("not enabled on this server"),
        "unavailable state renders"
    );
    assert!(
        html.contains("HFS_SUBSCRIPTIONS_ENABLED"),
        "the off state names the switch"
    );
    assert!(
        html.contains("--features subscriptions"),
        "the off state names the build feature"
    );
    assert!(
        html.contains(r#"href="/ui/subscriptions""#),
        "nav entry present with the engine off"
    );

    // Phase 2 — provider registered: cards, rows, dots, and the nav entry.
    set_provider(Arc::new(Fixed));
    let (status, html) = get("/ui/subscriptions").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains(r#"href="/ui/subscriptions""#),
        "nav entry live"
    );

    // Cards: 1 failing, 1 idle, 1 active, 4,500 notifications.
    assert!(
        html.contains("4,493"),
        "delivered card sums the 24h counters"
    );
    assert!(html.contains("96.2"), "first-try rate renders");

    // Rows: the failing subscription leads (worst first) and carries the red
    // edge, statuses render as dots with plain labels, the idle websocket
    // shows the 0-clients state with no streak, and the streak prints.
    let error_at = html.find("obs-critical").expect("error row");
    let active_at = html.find("encounter-start").expect("active row");
    assert!(error_at < active_at, "errors sort first");
    assert!(html.contains(r#"class="row--alert""#), "failing row edge");
    assert!(html.contains(r#"status-dot--danger"#));
    assert!(html.contains(r#"status-dot--ok"#));
    assert!(html.contains(r#"status-dot--warn"#));
    assert!(!html.contains(r#"tag--error"#), "pills gave way to dots");
    assert!(html.contains("0 clients"));
    assert!(html.contains(">7<"), "fail streak renders");
    assert!(html.contains("4,182"));

    // The Last 24 hrs sparkline (#782): rows with a series draw a state-tinted
    // polyline (with the count as its accessible name); the idle websocket has
    // no series and keeps the plain count.
    assert!(html.contains(r#"class="spark spark--active""#), "{html}");
    assert!(html.contains(r#"class="spark spark--error""#));
    assert!(
        !html.contains(r#"class="spark spark--idle""#),
        "no series, no line"
    );
    assert!(html.contains(r#"<polyline class="spark__line""#));
    assert!(html.contains(r#"aria-label="4,182""#));

    // Phase 3 — the sort control re-orders: most sent leads with the biggest
    // counter.
    let (_, html) = get("/ui/subscriptions?sort=sent").await;
    let first = html.find("encounter-start").expect("most sent row");
    let second = html.find("obs-critical").expect("second row");
    assert!(first < second, "sort=sent puts the biggest counter first");
}
