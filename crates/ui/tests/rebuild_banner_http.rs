//! #1065: while a search-index rebuild runs for the tenant, Home and Resources
//! say so — stored resources stay readable by id, but searches can miss them
//! until it finishes — and neither page shows the line otherwise.
//!
//! #1082 stopped the rail showing placeholder zeros during a rebuild; this is
//! the other half it left: the explicit "search index rebuilding" line.
//!
//! Registers its own [`DashboardProvider`] (process-global state), so it lives
//! in its own test binary, and each case uses a tenant of its own so the
//! snapshot cache never mixes them.

use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;

use helios_observability::dashboard::{
    DashboardProvider, DashboardSnapshot, DashboardWindow, Figures, ReindexActivity, TypeCount,
    set_provider,
};

#[path = "support/html.rs"]
mod html;
use html::Dom;

/// A tenant whose rebuild has counted its resources: 8,000 of 19,000.
const REBUILDING_TENANT: &str = "rebuild-running";
/// A tenant whose rebuild is still counting what it will process.
const COUNTING_TENANT: &str = "rebuild-counting";
/// A tenant with no rebuild running.
const IDLE_TENANT: &str = "rebuild-idle";

const REBUILD_LINE: &str = "[data-rebuild-notice]";

fn app_as(tenant: &str) -> Router {
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        helios_ui::NlSearch {
            enabled: true,
            configured: true,
            model: "test-model".to_string(),
        },
        None,
        None,
        tenant.to_string(),
        Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    )
}

async fn get_as(tenant: &str, path: &str) -> Dom {
    let response = app_as(tenant)
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Dom::page(std::str::from_utf8(&bytes).unwrap())
}

/// The line's text as read, without Fluent's bidi isolation marks around
/// interpolated values.
fn rebuild_line(dom: &Dom) -> String {
    dom.one(REBUILD_LINE)
        .text()
        .replace(['\u{2068}', '\u{2069}'], "")
}

struct Rebuilding;

#[async_trait::async_trait]
impl DashboardProvider for Rebuilding {
    async fn snapshot(
        &self,
        _window: DashboardWindow,
        tenant: &str,
        _types: &[String],
        _include_empty: bool,
    ) -> DashboardSnapshot {
        DashboardSnapshot {
            available: vec![TypeCount {
                resource_type: "Patient".into(),
                total: 42,
            }],
            figures: Figures::Exact {
                read_at: chrono::Utc::now(),
            },
            reindex_active: match tenant {
                REBUILDING_TENANT => Some(ReindexActivity {
                    jobs: 1,
                    processed: 8_000,
                    total: 19_000,
                }),
                COUNTING_TENANT => Some(ReindexActivity {
                    jobs: 1,
                    processed: 0,
                    total: 0,
                }),
                _ => None,
            },
            ..Default::default()
        }
    }
}

#[tokio::test]
async fn home_and_resources_say_when_the_search_index_is_rebuilding() {
    set_provider(Arc::new(Rebuilding));

    for path in ["/ui", "/ui/resources"] {
        let dom = get_as(REBUILDING_TENANT, path).await;
        let line = rebuild_line(&dom);
        assert!(
            line.contains("Search index rebuilding — 42% (8,000 of 19,000 resources)"),
            "{path}: the line names the rebuild and its progress: {line:?}"
        );
        assert!(
            line.contains("Searches may miss stored resources until it finishes."),
            "{path}: and what it means for search: {line:?}"
        );
        assert!(
            dom.one(REBUILD_LINE)
                .attr("class")
                .is_some_and(|class| class.contains("notice--warn")),
            "{path}: it is a warning, not a label"
        );

        // Before the rebuild has counted its resources: no percentage at all,
        // never a fabricated "0%".
        let dom = get_as(COUNTING_TENANT, path).await;
        let line = rebuild_line(&dom);
        assert!(
            line.contains("Search index rebuilding.") && !line.contains('%'),
            "{path}: {line:?}"
        );

        let dom = get_as(IDLE_TENANT, path).await;
        assert_eq!(dom.count(REBUILD_LINE), 0, "{path}: no rebuild, no line");
    }

    // On Home the line sits in the live region, and a running rebuild keeps
    // that region refreshing at the fast cadence so its percentage moves.
    let running = get_as(REBUILDING_TENANT, "/ui").await;
    let live = running.one("#dash-live");
    assert_eq!(
        live.one(REBUILD_LINE).attr("data-dash-notice"),
        Some("rebuilding")
    );
    assert_eq!(live.attr("data-dash-moving"), Some("1"));
    let idle = get_as(IDLE_TENANT, "/ui").await;
    assert_eq!(idle.one("#dash-live").attr("data-dash-moving"), None);
}
