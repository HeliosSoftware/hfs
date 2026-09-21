//! #1245: the Import pages say when the tenant's search index is rebuilding,
//! or when its last rebuild left resources unindexed.
//!
//! A submission turns Completed when ingest finishes. With
//! `HFS_BULK_SUBMIT_DEFER_INDEXING` that is when the rebuild that makes its
//! resources searchable *starts*: on the full Synthea corpus the list and the
//! detail page read "Completed" for about 2.4 hours while searches kept
//! missing most of the import, and only Home and Resources said so (#1065).
//!
//! The status card stops polling the moment the submission is terminal, so
//! the line lives in a region that refreshes by itself — also covered here.
//!
//! Registers its own [`DashboardProvider`] (process-global state), so it lives
//! in its own test binary, and each case uses a tenant of its own so the
//! snapshot cache never mixes them.

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use helios_observability::dashboard::{
    DashboardProvider, DashboardSnapshot, DashboardWindow, Figures, ReindexActivity, set_provider,
};
use helios_persistence::{
    backends::sqlite::SqliteBackend,
    core::{BulkProviderStore, SettingsStore},
    tenant::{TenantContext, TenantId, TenantPermissions},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[path = "support/html.rs"]
mod html;
use html::Dom;

/// A tenant whose rebuild has counted its resources: 2,821,304 of 18,955,865
/// — the 14% the issue was observed at.
const REBUILDING_TENANT: &str = "import-rebuild-running";
/// A tenant whose last rebuild completed with 11,704 resources unindexed.
const FAILED_TENANT: &str = "import-rebuild-failed";
/// A tenant with no rebuild running.
const IDLE_TENANT: &str = "import-rebuild-idle";
const FAILED_JOB_ID: &str = "job-1245";

const REBUILD_REGION: &str = "#import-rebuild";
const REBUILD_LINE: &str = "[data-rebuild-notice]";

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
            figures: Figures::Exact {
                read_at: chrono::Utc::now(),
            },
            reindex_active: match tenant {
                REBUILDING_TENANT => Some(ReindexActivity::Running {
                    jobs: 1,
                    processed: 2_821_304,
                    total: 18_955_865,
                }),
                FAILED_TENANT => Some(ReindexActivity::Failed {
                    job_id: FAILED_JOB_ID.to_string(),
                    errors: 11_704,
                }),
                _ => None,
            },
            ..Default::default()
        }
    }
}

/// One tenant's Import workspace over an in-memory SQLite store.
struct World {
    tenant: &'static str,
    settings: Arc<dyn SettingsStore>,
    bulk_provider: Arc<dyn BulkProviderStore>,
}

impl World {
    fn new(tenant: &'static str) -> Self {
        let backend = Arc::new({
            let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
            backend.init_schema().expect("init schema");
            backend
        });
        Self {
            tenant,
            settings: backend.clone(),
            bulk_provider: backend,
        }
    }

    fn app(&self) -> Router {
        helios_ui::mount_with_conformance_source(
            Router::new(),
            "9.9.9",
            None,
            helios_ui::NlSearch::default(),
            None,
            Some(Arc::clone(&self.settings)),
            self.tenant.to_string(),
            Arc::new(helios_ui::StaticConformanceSource::empty()),
            helios_fhir::FhirVersion::R4,
            None,
            // Unreachable: the kick-off fails fast and is only logged.
            "http://localhost:9/".to_string(),
            Some(Arc::clone(&self.bulk_provider)),
        )
    }

    async fn get(&self, path: &str) -> (StatusCode, String) {
        let response = self
            .app()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    async fn page(&self, path: &str) -> Dom {
        let (status, body) = self.get(path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        Dom::page(&body)
    }

    /// Creates a submission and stores it as Completed — what the recipient's
    /// clean `200` leaves behind once ingest finishes. Returns its detail path.
    async fn completed_submission(&self) -> String {
        let response = self
            .app()
            .oneshot(
                Request::post("/ui/bulk-import")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "name=Synthea&manifest_url=http%3A%2F%2Fone.example%2Fm.json&auth=none",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let detail_path = response.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string();
        let id = detail_path.rsplit('/').next().expect("submission id");

        let tenant =
            TenantContext::new(TenantId::new(self.tenant), TenantPermissions::full_access());
        let stored = self
            .bulk_provider
            .get_provider_submission(&tenant, id)
            .await
            .expect("read submission")
            .expect("submission exists");
        let mut document = stored.document;
        document["status"] = serde_json::json!("completed");
        self.bulk_provider
            .put_provider_submission(&tenant, id, document, Some(stored.version))
            .await
            .expect("store the completed submission");
        detail_path
    }
}

/// The line's text as read, without Fluent's bidi isolation marks around
/// interpolated values.
fn rebuild_line(dom: &Dom) -> String {
    dom.one(REBUILD_LINE)
        .text()
        .replace(['\u{2068}', '\u{2069}'], "")
}

/// The `hx-trigger` cadence of the region, in seconds.
fn cadence(dom: &Dom) -> String {
    dom.one(REBUILD_REGION)
        .attr("hx-trigger")
        .expect("the region refreshes itself")
        .to_string()
}

#[tokio::test]
async fn a_completed_submission_does_not_read_done_while_its_index_is_rebuilding() {
    set_provider(Arc::new(Rebuilding));
    let world = World::new(REBUILDING_TENANT);
    let detail_path = world.completed_submission().await;

    for path in ["/ui/bulk-import", detail_path.as_str()] {
        let dom = world.page(path).await;
        assert!(
            dom.text().contains("Completed"),
            "{path}: the fixture is the issue's — a submission shown Completed"
        );
        let line = rebuild_line(&dom);
        assert!(
            line.contains("Search index rebuilding — 14% (2,821,304 of 18,955,865 resources)"),
            "{path}: the line names the rebuild and its progress: {line:?}"
        );
        assert!(
            line.contains("Searches may miss stored resources until it finishes."),
            "{path}: and what it means for search: {line:?}"
        );
        let notice = dom.one(REBUILD_LINE);
        assert!(notice.has_class("notice--warn"), "{path}: it is a warning");
        assert_eq!(notice.attr("data-rebuild-state"), Some("running"), "{path}");

        // The status card stops polling once the submission is terminal, so
        // the region follows the rebuild itself, at the moving cadence.
        assert_eq!(cadence(&dom), "every 5s", "{path}");
    }
}

#[tokio::test]
async fn the_import_pages_keep_saying_when_the_last_rebuild_left_resources_unindexed() {
    set_provider(Arc::new(Rebuilding));
    let world = World::new(FAILED_TENANT);
    let detail_path = world.completed_submission().await;

    for path in ["/ui/bulk-import", detail_path.as_str()] {
        let dom = world.page(path).await;
        let line = rebuild_line(&dom);
        assert!(
            line.contains("The last search index rebuild left 11,704 resources unindexed.")
                && line.contains("$reindex-status/job-1245"),
            "{path}: how many, and where to find which: {line:?}"
        );
        assert_eq!(
            dom.one(REBUILD_LINE).attr("data-rebuild-state"),
            Some("failed"),
            "{path}"
        );
        // A failed rebuild is a settled fact: watched, not followed.
        assert_eq!(cadence(&dom), "every 10s", "{path}");
    }
}

#[tokio::test]
async fn no_rebuild_no_line_but_the_pages_still_watch_for_one() {
    set_provider(Arc::new(Rebuilding));
    let world = World::new(IDLE_TENANT);
    let detail_path = world.completed_submission().await;

    for path in ["/ui/bulk-import", detail_path.as_str()] {
        let dom = world.page(path).await;
        assert_eq!(dom.count(REBUILD_LINE), 0, "{path}: no rebuild, no line");
        // A page opened while the manifest still ingests must learn that the
        // rebuild began: nothing else on it polls once the status is terminal.
        assert_eq!(cadence(&dom), "every 10s", "{path}");
    }
}

#[tokio::test]
async fn the_region_refresh_swaps_only_a_changed_line_and_announces_only_a_new_state() {
    set_provider(Arc::new(Rebuilding));
    let world = World::new(REBUILDING_TENANT);

    let page = world.page("/ui/bulk-import").await;
    let refresh = page
        .one(REBUILD_REGION)
        .attr("hx-get")
        .expect("the region names its refresh")
        .to_string();
    assert!(
        refresh.starts_with("/ui/bulk-import/rebuild?digest=")
            && refresh.ends_with("&seen=running"),
        "{refresh}"
    );

    // The same line again: nothing to swap, so nothing re-announced.
    let (status, body) = world.get(&refresh).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // The percentage moved (a stale digest), the state did not: swapped in,
    // quietly.
    let (status, body) = world
        .get("/ui/bulk-import/rebuild?digest=stale&seen=running")
        .await;
    assert_eq!(status, StatusCode::OK);
    let fragment = Dom::fragment(&body);
    assert_eq!(fragment.one(REBUILD_LINE).attr("aria-live"), Some("off"));
    assert_eq!(
        fragment.one(REBUILD_REGION).attr("hx-get"),
        Some(refresh.as_str()),
        "the swapped region carries the refresh on"
    );

    // The rebuild began under a page that showed none: announced.
    let (_, body) = world
        .get("/ui/bulk-import/rebuild?digest=stale&seen=idle")
        .await;
    assert_eq!(
        Dom::fragment(&body).one(REBUILD_LINE).attr("aria-live"),
        Some("polite")
    );

    // And once it ends cleanly the line goes, with the watch kept.
    let idle = World::new(IDLE_TENANT);
    let (status, body) = idle
        .get("/ui/bulk-import/rebuild?digest=stale&seen=running")
        .await;
    assert_eq!(status, StatusCode::OK);
    let fragment = Dom::fragment(&body);
    assert_eq!(fragment.count(REBUILD_LINE), 0);
    assert_eq!(cadence(&fragment), "every 10s");
}
