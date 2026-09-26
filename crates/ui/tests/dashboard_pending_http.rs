//! #956 / #1078: the Dashboard's degraded and qualified states, rendered over
//! HTTP.
//!
//! The bug this pins down: switching the time window during an import made the
//! page show a fabricated Patient/Observation/Encounter/Condition curve under
//! the notice "no live metrics provider is registered on this build" — while a
//! provider was registered and merely slow. The window is part of the snapshot
//! cache key, so every window switch is a cold key and takes the timeout path.
//! #1078 then asked that figures which are approximate say so, and that every
//! measured figure say when it was read.
//!
//! Registers its own [`DashboardProvider`] (process-global state), so this
//! lives in its own test binary. The provider discriminates on the window and
//! the tenant, both part of the cache key:
//!
//! - `1h` never answers (it sleeps past the end of the test) for any tenant but
//!   [`FIRST_VIEW_TENANT`], so it is permanently the cold case: the cache's own
//!   [`SnapshotState::Pending`];
//! - every other window (and `1h` under [`FIRST_VIEW_TENANT`]) answers with the
//!   tenant's [`Figures`]: [`Figures::Approximate`] under
//!   [`APPROXIMATE_TENANT`], [`Figures::Pending`] under
//!   [`FIGURES_PENDING_TENANT`] (the provider still seeding the tenant, nothing
//!   measured), [`Figures::Unsupported`] under [`COUNTS_UNSUPPORTED_TENANT`],
//!   and [`Figures::Exact`] otherwise — with active imports reported under
//!   [`IMPORT_ACTIVE_TENANT`] (and none under [`IMPORT_IDLE_TENANT`] and
//!   [`FIRST_VIEW_TENANT`]), and with distinctive figures under
//!   [`DISTINCT_FIGURES_TENANT`].
//!
//! A waiting page retries a bounded number of times. Once that budget is
//! spent, a page whose provider did answer with [`Figures::Pending`] keeps a
//! slow watch at the settled cadence, so it still picks up its figures; the
//! cache's own cold state stops. A backend that cannot count never polls. A
//! ready page always watches itself: every 5 seconds, marked moving, while its
//! figures can still move — they are approximate, or an import is running —
//! and every 10 seconds otherwise, so a tab opened on settled figures still
//! notices an import started later. Each ready render carries a state hash of
//! its figures (never of its "as of" time), so the client can tell a refresh
//! that changed something from one that did not.
//!
//! The snapshot cache is process-global too. It never answers a key from a
//! snapshot cached under another window, so the cold `1h` key stays cold
//! whatever ran first in this binary; cases with figures of their own still
//! mount the UI under a tenant of their own (the mount's default tenant is the
//! request's tenant), so their cache entries never mix.
//!
//! Every assertion reads the parsed markup ([`html::Dom`]): elements by CSS
//! selector, attributes as the browser decodes them, text as it reads.
//!
//! [`SnapshotState::Pending`]: helios_observability::dashboard::SnapshotState::Pending

use async_trait::async_trait;
use axum::{Router, body::Body, http::Request};
use chrono::{DateTime, Utc};
use helios_observability::dashboard::{
    DashboardPoint, DashboardProvider, DashboardSeries, DashboardSnapshot, DashboardWindow,
    ExportJobCounts, Figures, TypeCount, set_provider,
};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[path = "support/html.rs"]
mod html;
use html::{Dom, El};

/// A tenant whose snapshot is measured but not an exact storage match.
const APPROXIMATE_TENANT: &str = "dash-approximate";
/// A tenant whose snapshot carries figures no other case produces.
const DISTINCT_FIGURES_TENANT: &str = "dash-distinct-figures";
/// A tenant whose snapshot is exact but reports two running imports.
const IMPORT_ACTIVE_TENANT: &str = "dash-import-active";
/// A tenant whose snapshot is exact and reports no running import.
const IMPORT_IDLE_TENANT: &str = "dash-import-idle";
/// A tenant the provider is still seeding: nothing is measured yet
/// ([`Figures::Pending`]).
const FIGURES_PENDING_TENANT: &str = "dash-figures-pending";
/// A tenant on a storage backend that cannot count at all
/// ([`Figures::Unsupported`]).
const COUNTS_UNSUPPORTED_TENANT: &str = "dash-counts-unsupported";
/// A tenant answered promptly in every window, `1h` included, with every
/// headline figure known (job counts too): the first view of any window.
const FIRST_VIEW_TENANT: &str = "dash-first-view";

struct WindowScriptedProvider;

#[async_trait]
impl DashboardProvider for WindowScriptedProvider {
    async fn snapshot(
        &self,
        window: DashboardWindow,
        tenant: &str,
        _types: &[String],
        _include_empty: bool,
    ) -> DashboardSnapshot {
        if window == DashboardWindow::LastHour && tenant != FIRST_VIEW_TENANT {
            // Far past the cold-load budget and past the test's own lifetime,
            // so this key stays pending however often it is asked for — the
            // storage contention the real bug happens under, held still.
            tokio::time::sleep(std::time::Duration::from_secs(3_600)).await;
        }

        let bucket_start: DateTime<Utc> = DateTime::from_timestamp(1_752_451_200, 0).unwrap();
        let series = |resource_type: &str, total: u64| DashboardSeries {
            resource_type: resource_type.to_string(),
            total,
            points: vec![DashboardPoint {
                bucket_start,
                delta: total as i64,
                cumulative: total,
            }],
        };
        let count = |resource_type: &str, total: u64| TypeCount {
            resource_type: resource_type.to_string(),
            total,
        };

        // The provider contract: without figures, totals, `available` and
        // `series` stay empty.
        if tenant == FIGURES_PENDING_TENANT {
            return DashboardSnapshot {
                fhir_version: "R4".to_string(),
                window,
                figures: Figures::Pending,
                ..Default::default()
            };
        }
        if tenant == COUNTS_UNSUPPORTED_TENANT {
            return DashboardSnapshot {
                fhir_version: "R4".to_string(),
                window,
                figures: Figures::Unsupported,
                ..Default::default()
            };
        }

        let read_at = Utc::now();
        let figures = if tenant == APPROXIMATE_TENANT {
            Figures::Approximate {
                read_at,
                reconciled_at: read_at - chrono::Duration::minutes(1),
            }
        } else {
            Figures::Exact { read_at }
        };

        if tenant == DISTINCT_FIGURES_TENANT {
            // Figures no other case produces, so the page can only show them
            // by serving this tenant's own snapshot.
            return DashboardSnapshot {
                fhir_version: "R4".to_string(),
                total_resources: 777,
                distinct_types: 7,
                window,
                series: vec![series("Patient", 700)],
                available: vec![count("Patient", 700), count("Observation", 77)],
                figures,
                ..Default::default()
            };
        }

        DashboardSnapshot {
            fhir_version: "R4".to_string(),
            total_resources: 5,
            distinct_types: 1,
            window,
            series: vec![series("Patient", 5)],
            available: vec![count("Patient", 5)],
            export_jobs: (tenant == FIRST_VIEW_TENANT).then_some(ExportJobCounts {
                running: 0,
                queued: 0,
            }),
            reindex_active: None,
            import_jobs_active: match tenant {
                IMPORT_ACTIVE_TENANT => Some(2),
                IMPORT_IDLE_TENANT | FIRST_VIEW_TENANT => Some(0),
                _ => None,
            },
            figures,
        }
    }
}

fn app(tenant: &str) -> Router {
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
        tenant.to_string(),
        std::sync::Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    )
}

/// The whole page `uri` renders under `tenant`, parsed.
async fn get_as(tenant: &str, uri: &str) -> Dom {
    let response = app(tenant)
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    Dom::page(&body_text(response).await)
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn get(uri: &str) -> Dom {
    get_as("default", uri).await
}

/// The swappable live region, where its refresh lives.
fn dash_live(dom: &Dom) -> El<'_> {
    dom.one("div#dash-live")
}

/// The notice line with this slug.
fn notice<'a>(dom: &'a Dom, slug: &str) -> El<'a> {
    dom.one(&format!(r#"p[data-dash-notice="{slug}"]"#))
}

/// Every notice line on the page.
fn notices(dom: &Dom) -> Vec<El<'_>> {
    dom.all("[data-dash-notice]")
}

/// Whether a headline figure reads exactly `value` (a known figure, not the
/// unavailable mark).
fn shows_figure(dom: &Dom, value: &str) -> bool {
    dom.all(".stat__value:not(.stat__value--unavailable)")
        .iter()
        .any(|figure| figure.text() == value)
}

/// Whether a notice line offers the plain "Retry now" link.
fn has_retry_link(dom: &Dom) -> bool {
    dom.all("p[data-dash-notice] a[href]")
        .iter()
        .any(|link| link.text() == "Retry now")
}

/// The live region's figure-state hash, asserted to be 16 lowercase hex chars.
fn dash_state(dom: &Dom) -> String {
    let live = dash_live(dom);
    let state = live
        .attr("data-dash-state")
        .unwrap_or_else(|| panic!("a state hash: {live:?}"));
    assert_eq!(state.len(), 16, "{live:?}");
    assert!(
        state
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "lowercase hex: {live:?}"
    );
    state.to_string()
}

/// Asserts nothing on the page runs the bounded retry, or any delayed load.
fn assert_no_auto_retry(dom: &Dom) {
    assert_eq!(
        dom.count(r#"[hx-trigger^="load"]"#),
        0,
        "a ready page does not also run the bounded retry"
    );
    assert_eq!(
        dom.count(r#"[hx-trigger*="load delay"]"#),
        0,
        "nor any delayed load"
    );
}

/// Asserts what every ready page's periodic refresh shares, whatever its
/// cadence: it runs every `seconds` behind the client's guard, swaps only the
/// live region, re-requests the same view without a retry count, carries a
/// state hash of its figures, and does not also run the bounded retry.
fn assert_refreshes_every(dom: &Dom, window: &str, seconds: u32) {
    let live = dash_live(dom);
    assert_eq!(
        live.attr("data-dash-refresh"),
        Some(seconds.to_string().as_str()),
        "{live:?}"
    );
    assert_eq!(
        live.attr("hx-trigger"),
        Some(format!("every {seconds}s [hfsDashCanRefresh()]").as_str()),
        "{live:?}"
    );
    assert_eq!(live.attr("hx-select"), Some("#dash-live"), "{live:?}");
    let href = live
        .attr("hx-get")
        .expect("the refresh re-requests the page");
    assert!(href.starts_with("/ui?"), "{href}");
    assert!(href.contains(&format!("window={window}")), "{href}");
    assert!(!href.contains("retry="), "a refresh is not a retry: {href}");
    dash_state(dom);
    assert_no_auto_retry(dom);
}

/// Asserts the page polls itself as a ready page with moving figures does:
/// every 5 seconds, marked moving.
fn assert_polls_periodically(dom: &Dom, window: &str) {
    assert_refreshes_every(dom, window, 5);
    let live = dash_live(dom);
    assert_eq!(live.attr("data-dash-moving"), Some("1"), "{live:?}");
    assert_eq!(
        dom.count(r#"[hx-trigger*="every 10s"]"#),
        0,
        "one cadence at a time"
    );
}

/// Asserts the page watches settled figures as a ready page does: every 10
/// seconds, not marked moving, so an import started later is still noticed.
fn assert_watches_settled(dom: &Dom, window: &str) {
    assert_refreshes_every(dom, window, 10);
    let live = dash_live(dom);
    assert!(!live.has_attr("data-dash-moving"), "{live:?}");
    assert_eq!(
        dom.count(r#"[hx-trigger*="every 5s"]"#),
        0,
        "one cadence at a time"
    );
}

/// Asserts the page keeps the slow watch of a waiting page whose fast retries
/// are spent: the settled cadence, marked waiting, re-requesting the same view
/// with the spent retry count kept, and no bounded retry.
fn assert_watches_while_waiting(dom: &Dom, window: &str) {
    let live = dash_live(dom);
    assert_eq!(live.attr("data-dash-refresh"), Some("10"), "{live:?}");
    assert_eq!(live.attr("data-dash-waiting"), Some("1"), "{live:?}");
    assert!(!live.has_attr("data-dash-moving"), "{live:?}");
    assert_eq!(
        live.attr("hx-trigger"),
        Some("every 10s [hfsDashCanRefresh()]"),
        "{live:?}"
    );
    assert_eq!(live.attr("hx-select"), Some("#dash-live"), "{live:?}");
    let href = live.attr("hx-get").expect("the watch re-requests the page");
    assert!(href.starts_with("/ui?"), "{href}");
    assert!(href.contains(&format!("window={window}")), "{href}");
    assert!(
        href.contains("retry=3"),
        "the spent budget stays spent: {href}"
    );
    dash_state(dom);
    assert_eq!(
        dom.count(r#"[hx-trigger*="load delay"]"#),
        0,
        "no fast retry any more"
    );
    assert_eq!(dom.count(r#"[hx-trigger*="every 5s"]"#), 0);
}

/// Asserts nothing on the page refreshes periodically, at either cadence.
fn assert_no_periodic_refresh(dom: &Dom) {
    assert_eq!(dom.count("[data-dash-refresh]"), 0);
    assert_eq!(dom.count("[data-dash-moving]"), 0);
    assert_eq!(dom.count(r#"[hx-trigger*="every "]"#), 0);
}

/// Asserts the page is the cold waiting page's figures: unknown, not zero,
/// nothing charted and nothing dated.
fn assert_figures_unknown(dom: &Dom) {
    assert!(
        dom.count(".stat__value--unavailable") >= 3,
        "resource types, stored resources and the chart total all read as \
         unknown while waiting, got: {}",
        dom.count(".stat__value--unavailable")
    );
    assert!(!shows_figure(dom, "0"), "no figure renders as zero");
    assert_eq!(
        dom.count("#chart-data"),
        0,
        "nothing is charted while waiting, so no chart data carrier is emitted"
    );
    assert_eq!(dom.count("time"), 0, "nothing was read, so no \"as of\"");
}

/// The regression itself: a window whose snapshot has not landed says it is
/// waiting — it does not claim the build has no metrics, and it invents
/// nothing.
#[tokio::test]
async fn a_slow_window_renders_waiting_not_sample_data() {
    let dom = get("/ui?window=1h").await;

    let line = notice(&dom, "pending");
    assert!(
        line.text().contains("Still gathering the live figures"),
        "the waiting notice should render: {line:?}"
    );
    assert!(line.has_class("notice--warn"), "{line:?}");
    assert_eq!(notices(&dom).len(), 1, "one line");
    assert!(
        !dom.text().contains("no live metrics provider"),
        "a registered-but-slow provider must not be reported as absent"
    );
    // The sample curve's headline figures (sample_snapshot: 142 distinct
    // types, ~1.2k Patients) must be nowhere on the page.
    assert!(
        !shows_figure(&dom, "142"),
        "the invented distinct-type count must not appear"
    );
    // The three snapshot-derived figures render as unknown, not as zero, and
    // a truly cold tenant has no reading to date and no figures to qualify.
    assert_figures_unknown(&dom);
}

/// Waiting is recoverable both ways: htmx re-requests the live region on its
/// own, and the notice always carries a plain link for a browser without it.
#[tokio::test]
async fn the_waiting_page_offers_an_automatic_and_a_manual_retry() {
    let dom = get("/ui?window=1h").await;

    let live = dash_live(&dom);
    assert_eq!(
        live.attr("hx-select"),
        Some("#dash-live"),
        "the auto-refresh swaps just the live region"
    );
    assert!(
        live.attr("hx-get")
            .is_some_and(|href| href.contains("retry=1")),
        "the first attempt asks for the second: {live:?}"
    );
    assert!(has_retry_link(&dom), "the no-JS way out is present");
    assert_eq!(
        live.attr("hx-trigger"),
        Some("load delay:1200ms"),
        "waiting keeps its bounded retry"
    );
    assert_no_periodic_refresh(&dom);
}

/// The auto-refresh is budgeted: the page stops re-requesting itself after a
/// bounded number of attempts, leaving only the manual link. The wait is
/// caused by load, so the recovery must not add to it indefinitely.
#[tokio::test]
async fn the_automatic_retry_stops_after_its_budget() {
    let dom = get("/ui?window=1h&retry=3").await;

    assert!(
        notice(&dom, "pending")
            .text()
            .contains("Still gathering the live figures"),
        "still waiting"
    );
    assert_eq!(
        dom.count(r#"[hx-trigger^="load"]"#),
        0,
        "the budget is spent, so nothing re-requests itself"
    );
    assert!(has_retry_link(&dom), "the manual retry outlives the budget");
    // The cache's own cold state keeps no slow watch (#1078): a server too
    // busy to fill it is not polled.
    assert_no_periodic_refresh(&dom);
    assert_eq!(dom.count("[data-dash-waiting]"), 0);
}

/// #1078: a window that is slow to compute never borrows the figures the same
/// tenant has cached under another window — those would be another window's
/// reading. It is the cold waiting page: unknown figures, the undated waiting
/// line, the bounded retry and, once that is spent, no slow watch.
#[tokio::test]
async fn a_slow_window_never_borrows_another_windows_figures() {
    // Warm the tenant's 30d key with the same selection.
    let warm = get_as(DISTINCT_FIGURES_TENANT, "/ui?types=Patient&window=30d").await;
    assert_eq!(warm.count("#chart-data"), 1, "the 30d chart renders");
    assert!(shows_figure(&warm, "777"));

    let dom = get_as(DISTINCT_FIGURES_TENANT, "/ui?types=Patient&window=1h").await;

    let line = notice(&dom, "pending");
    assert_eq!(notices(&dom).len(), 1, "one line");
    assert!(line.text().contains("Still gathering the live figures"));
    assert!(
        !shows_figure(&dom, "777"),
        "no figure from the 30d snapshot"
    );
    assert!(!shows_figure(&dom, "7"));
    assert_figures_unknown(&dom);
    // The selectors survive the wait with the requested selection.
    assert_eq!(
        dom.count(r#".window-picker a[href="/ui?types=Patient&window=30d"]"#),
        1
    );
    let live = dash_live(&dom);
    assert_eq!(live.attr("hx-trigger"), Some("load delay:1200ms"));
    assert!(
        live.attr("hx-get")
            .is_some_and(|href| href.contains("retry=1")),
        "{live:?}"
    );
    assert_no_periodic_refresh(&dom);

    let spent = get_as(
        DISTINCT_FIGURES_TENANT,
        "/ui?types=Patient&window=1h&retry=3",
    )
    .await;
    notice(&spent, "pending");
    assert_eq!(spent.count(r#"[hx-trigger^="load"]"#), 0, "budget spent");
    assert!(has_retry_link(&spent));
    assert_no_periodic_refresh(&spent);
    assert_eq!(spent.count("[data-dash-waiting]"), 0);
}

/// #1078: figures counted from recent writes rather than read exactly from
/// storage chart normally, but are labelled approximate and dated. Nothing is
/// missing, so the page neither warns nor retries — but the figures keep
/// moving, so it refreshes itself periodically instead of waiting for a
/// reload.
#[tokio::test]
async fn an_approximate_snapshot_charts_with_a_dated_label_and_refreshes_periodically() {
    let dom = get_as(APPROXIMATE_TENANT, "/ui?window=30d").await;

    assert_eq!(dom.count("svg.chart"), 1, "the chart renders");
    assert_eq!(dom.count("#chart-data"), 1);
    assert!(shows_figure(&dom, "5"));
    let line = notice(&dom, "approximate");
    assert!(
        line.text()
            .contains("Approximate: counted from recent writes"),
        "{line:?}"
    );
    assert!(
        line.one("time[datetime]")
            .attr("datetime")
            .is_some_and(|at| !at.is_empty()),
        "figures are dated: {line:?}"
    );
    assert_eq!(
        dom.count(".notice--warn"),
        0,
        "approximate figures are labelled, not flagged"
    );
    assert!(!has_retry_link(&dom));
    assert!(!dom.text().contains("Waiting for the live figures"));
    // Nothing to wait for; the figures move, so the page polls.
    assert_polls_periodically(&dom, "30d");
}

/// An exact snapshot still moves while an import runs, so the page polls.
#[tokio::test]
async fn an_exact_snapshot_with_an_active_import_refreshes_periodically() {
    let dom = get_as(IMPORT_ACTIVE_TENANT, "/ui?window=30d").await;

    assert_eq!(dom.count("#chart-data"), 1, "the chart renders");
    assert_eq!(dom.count(r#"[data-dash-notice="approximate"]"#), 0);
    notice(&dom, "live");
    assert!(!has_retry_link(&dom));
    assert_polls_periodically(&dom, "30d");
}

/// An exact snapshot that reports zero running imports is settled, but the
/// page still watches it — slowly, and not marked moving — so a tab opened
/// now notices an import started later.
#[tokio::test]
async fn an_exact_snapshot_with_no_active_import_watches_slowly() {
    let dom = get_as(IMPORT_IDLE_TENANT, "/ui?window=30d").await;

    assert_eq!(dom.count("#chart-data"), 1, "the chart renders");
    notice(&dom, "live");
    assert_watches_settled(&dom, "30d");
}

/// #1078 (moved from the browser suite): a ready page — whatever the window,
/// `1h` included — has its figures on its very first render. It schedules no
/// auto-retry, polls itself with a state hash, charts at least one series, and
/// no headline card reads as unavailable.
#[tokio::test]
async fn a_ready_first_render_shows_its_figures_in_every_window() {
    // Every headline card can be known only once the uptime tracker runs.
    helios_observability::uptime::init();

    for window in ["1h", "24h", "30d"] {
        let dom = get_as(
            FIRST_VIEW_TENANT,
            &format!("/ui?types=Patient&window={window}"),
        )
        .await;

        for slug in ["pending", "sample"] {
            assert_eq!(
                dom.count(&format!(r#"[data-dash-notice="{slug}"]"#)),
                0,
                "{window}: never the {slug} page"
            );
        }
        notice(&dom, "live");
        let live = dash_live(&dom);
        assert!(
            !live
                .attr("hx-trigger")
                .is_some_and(|trigger| trigger.starts_with("load delay:")),
            "{window}: a ready page schedules no auto-retry: {live:?}"
        );
        assert!(
            live.has_attr("data-dash-refresh"),
            "{window}: every ready page polls itself: {live:?}"
        );
        assert_watches_settled(&dom, window);
        assert_eq!(dom.count(".chart-empty"), 0, "{window}: not waiting");
        assert!(
            !dom.all("svg.chart polyline.series").is_empty(),
            "{window}: a chart"
        );
        assert!(dom.count(".stat-grid .stat__value") >= 5, "{window}");
        assert_eq!(
            dom.count(".stat__value--unavailable"),
            0,
            "{window}: headline cards show figures"
        );
    }
}

/// #1078 (moved from the browser suite): the chart card has no expand/collapse
/// toggle; the chart always renders at its one fixed size.
#[tokio::test]
async fn the_chart_has_no_expand_toggle() {
    let dom = get_as(IMPORT_IDLE_TENANT, "/ui?types=Patient&window=30d").await;

    assert!(!dom.all("svg.chart polyline.series").is_empty(), "charted");
    assert_eq!(dom.count(r#"[href*="expand=1"]"#), 0);
    assert_eq!(dom.count(".chart-card__tools a.pill--square"), 0);
    assert_eq!(dom.one("svg.chart").attr("viewBox"), Some("0 0 1060 300"));
}

/// The state hash follows the figures, not the moment they were rendered:
/// the same view asked for twice carries the same state.
#[tokio::test]
async fn the_same_figures_rendered_twice_carry_the_same_state() {
    let uri = "/ui?types=Patient&window=30d";
    let first = get_as(IMPORT_IDLE_TENANT, uri).await;
    let second = get_as(IMPORT_IDLE_TENANT, uri).await;
    let third = get_as(IMPORT_IDLE_TENANT, uri).await;

    for dom in [&first, &second, &third] {
        assert_watches_settled(dom, "30d");
    }
    // The server folds the current minute into the digest so a settled page
    // still re-renders about once a minute; three back-to-back renders cross
    // at most one minute boundary, so at least one consecutive pair agrees.
    let (a, b, c) = (dash_state(&first), dash_state(&second), dash_state(&third));
    assert!(a == b || b == c, "{a} {b} {c}");
}

/// Different figures carry a different state: two tenants whose providers
/// answer the same view with different totals never share a hash.
#[tokio::test]
async fn different_figures_carry_a_different_state() {
    let idle = get_as(IMPORT_IDLE_TENANT, "/ui?types=Patient&window=30d").await;
    let distinct = get_as(DISTINCT_FIGURES_TENANT, "/ui?types=Patient&window=30d").await;

    // Both settled and ready, so only the figures differ.
    assert_watches_settled(&idle, "30d");
    assert_watches_settled(&distinct, "30d");
    assert!(shows_figure(&distinct, "777"));
    assert!(!shows_figure(&idle, "777"));
    assert_ne!(dash_state(&idle), dash_state(&distinct));
}

/// A periodic swap must not re-announce a notice the user already heard:
/// `notices=` names the lines the previous render showed, and those render
/// `aria-live="off"`. A line whose kind is not in the list is new, and is
/// announced. Checked for every kind of line this provider produces.
#[tokio::test]
async fn notices_already_shown_are_not_announced_again() {
    let fresh = get_as(APPROXIMATE_TENANT, "/ui?window=30d").await;
    assert_eq!(
        notice(&fresh, "approximate").attr("aria-live"),
        Some("polite")
    );

    let unchanged = get_as(APPROXIMATE_TENANT, "/ui?window=30d&notices=approximate").await;
    assert_eq!(
        notice(&unchanged, "approximate").attr("aria-live"),
        Some("off")
    );

    let changed = get_as(APPROXIMATE_TENANT, "/ui?window=30d&notices=live").await;
    assert_eq!(
        notice(&changed, "approximate").attr("aria-live"),
        Some("polite")
    );

    // Moved from the browser suite: whichever kinds a render carries, a first
    // render announces every line, and a refresh naming `approximate,live`
    // silences exactly the lines of those kinds.
    let quiet = ["approximate", "live"];
    let mut outcomes = Vec::new();
    for (tenant, uri) in [
        (APPROXIMATE_TENANT, "/ui?window=30d"),
        (IMPORT_IDLE_TENANT, "/ui?window=30d"),
        ("default", "/ui?window=1h"),
        (COUNTS_UNSUPPORTED_TENANT, "/ui?window=30d"),
    ] {
        let first = get_as(tenant, uri).await;
        let lines = notices(&first);
        assert!(
            !lines.is_empty(),
            "{tenant} {uri}: a page names its figures"
        );
        for line in lines {
            assert_eq!(
                line.attr("aria-live"),
                Some("polite"),
                "{tenant} {uri}: first render: {line:?}"
            );
        }

        let again = get_as(tenant, &format!("{uri}&notices={}", quiet.join(","))).await;
        let lines = notices(&again);
        assert!(!lines.is_empty(), "{tenant} {uri}");
        for line in lines {
            let kind = line.attr("data-dash-notice").unwrap();
            let expected = if quiet.contains(&kind) {
                "off"
            } else {
                "polite"
            };
            assert_eq!(
                line.attr("aria-live"),
                Some(expected),
                "{tenant} {uri}: {kind} after notices={quiet:?}: {line:?}"
            );
            outcomes.push(expected);
        }
    }
    assert!(outcomes.contains(&"off") && outcomes.contains(&"polite"));
}

/// Exact figures carry no warning at all — the states above must not leak into
/// the ordinary page. They still say when they were read (#1078): a snapshot
/// can be served stale while a refresh runs.
#[tokio::test]
async fn a_complete_snapshot_carries_only_its_as_of_time() {
    let dom = get("/ui?window=30d").await;

    let text = dom.text();
    assert!(!text.contains("no live metrics provider"));
    assert!(!text.contains("Still gathering the live figures"));
    assert!(!text.contains("Approximate:"));
    assert_eq!(dom.count(".notice--warn"), 0);
    assert_eq!(notices(&dom).len(), 1, "one line");
    notice(&dom, "live").one("time[datetime]");
    assert_eq!(dom.count("#chart-data"), 1, "the chart renders");
    // Exact, complete, and no import reported: nothing moves right now, but
    // the page still watches, slowly, for figures that start moving later.
    assert_watches_settled(&dom, "30d");
}

/// #1078: a provider still seeding the tenant answers with nothing measured
/// ([`Figures::Pending`]). The page is the cold waiting page — unknown figures,
/// the waiting notice undated, the bounded retry — and never a row of zeros.
#[tokio::test]
async fn a_tenant_whose_figures_are_pending_renders_waiting_not_zeros() {
    let dom = get_as(FIGURES_PENDING_TENANT, "/ui?types=Patient&window=30d").await;

    assert!(
        notice(&dom, "pending")
            .text()
            .contains("Still gathering the live figures")
    );
    assert_eq!(dom.count(r#"[data-dash-notice="live"]"#), 0);
    assert_figures_unknown(&dom);
    assert!(
        dom.one(".chart-empty")
            .text()
            .starts_with("Waiting for the live figures")
    );
    assert!(
        !dom.text().contains("Nothing to chart yet"),
        "nothing on the page claims the tenant is empty while it waits"
    );
    // The selectors survive the wait with the requested selection.
    assert_eq!(
        dom.count(r#".window-picker a[href="/ui?types=Patient&window=24h"]"#),
        1
    );

    let live = dash_live(&dom);
    assert_eq!(live.attr("hx-trigger"), Some("load delay:1200ms"));
    assert!(
        live.attr("hx-get")
            .is_some_and(|href| href.contains("retry=1")),
        "{live:?}"
    );
    assert!(has_retry_link(&dom));
    assert_no_periodic_refresh(&dom);
}

/// #1078: with nothing requested and nothing known yet, the type picker is
/// empty — and says it is waiting, not that there is nothing to chart.
#[tokio::test]
async fn an_empty_type_picker_on_a_waiting_page_says_it_is_waiting() {
    let dom = get_as(FIGURES_PENDING_TENANT, "/ui?window=30d").await;

    assert!(
        dom.one("#chart-pick p.chart-pick__none")
            .text()
            .starts_with("Waiting for the live figures"),
        "the empty picker names the wait"
    );
    assert!(
        !dom.text().contains("Nothing to chart yet"),
        "an empty picker on a waiting page does not read as an empty tenant"
    );
}

/// #1078: once the fast retries are spent, a page waiting on a tenant the
/// provider is still seeding keeps watching slowly, keeping the spent count —
/// while the cache's own cold state (`the_automatic_retry_stops_after_its_budget`)
/// stops.
#[tokio::test]
async fn a_spent_figures_pending_page_watches_slowly() {
    let dom = get_as(
        FIGURES_PENDING_TENANT,
        "/ui?types=Patient&window=30d&retry=3",
    )
    .await;

    notice(&dom, "pending");
    assert!(!shows_figure(&dom, "0"), "still no zeros");
    assert!(has_retry_link(&dom));
    assert_watches_while_waiting(&dom, "30d");

    let cold = get("/ui?window=1h&retry=3").await;
    assert_no_periodic_refresh(&cold);
    assert_eq!(cold.count("[data-dash-waiting]"), 0);
}

/// #1078: a backend that cannot count says so, once, as a plain label: no
/// zeros, no waiting, no "as of", no retry and no polling of any kind.
#[tokio::test]
async fn a_backend_that_cannot_count_says_so_and_never_polls() {
    for uri in [
        "/ui?types=Patient&window=30d",
        "/ui?types=Patient&window=24h&retry=3",
    ] {
        let dom = get_as(COUNTS_UNSUPPORTED_TENANT, uri).await;

        let line = notice(&dom, "unsupported");
        assert!(
            !line.has_class("notice--warn"),
            "a label, not a warning: {line:?}"
        );
        assert_eq!(notices(&dom).len(), 1, "{uri}");
        assert!(
            line.text()
                .contains("This storage backend cannot count stored resources"),
            "{uri}: {line:?}"
        );
        assert_eq!(
            dom.one(".chart-empty").text(),
            "Resource counts are not available for this storage backend.",
            "{uri}: the chart area names the fact"
        );
        let text = dom.text();
        assert!(!text.contains("Waiting for the live figures"), "{uri}");
        assert!(!text.contains("Still gathering the live figures"), "{uri}");
        assert!(!text.contains("Nothing to chart yet"), "{uri}");
        assert!(!has_retry_link(&dom), "{uri}");
        // The figures read as unavailable, never zero, nothing charted or dated.
        assert_figures_unknown(&dom);

        let live = dash_live(&dom);
        assert!(!live.has_attr("hx-get"), "{uri}: nothing polls: {live:?}");
        assert!(!live.has_attr("hx-trigger"), "{uri}: {live:?}");
        assert!(!live.has_attr("data-dash-waiting"), "{uri}: {live:?}");
        assert_no_periodic_refresh(&dom);
    }
}

/// #1078: the live region names the tenant, FHIR version and locale it was
/// rendered for, so its requests can send them back.
#[tokio::test]
async fn the_live_region_names_its_context() {
    let ready = get_as(IMPORT_IDLE_TENANT, "/ui?window=30d").await;
    assert_eq!(
        dash_live(&ready).attr("data-dash-ctx"),
        Some("dash-import-idle|R4|en"),
        "a context on a ready page"
    );

    let waiting = get("/ui?window=1h").await;
    assert_eq!(
        dash_live(&waiting).attr("data-dash-ctx"),
        Some("default|R4|en"),
        "the bounded retry sends it too"
    );
}

async fn send_as(tenant: &str, uri: &str, htmx: bool) -> axum::response::Response {
    let mut request = Request::get(uri);
    if htmx {
        request = request.header("HX-Request", "true");
    }
    app(tenant)
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// #1078: a live-region request made for another tenant, FHIR version or
/// locale than the one the server resolves now gets an empty `HX-Refresh`
/// answer, so htmx reloads the page instead of mixing contexts. A matching
/// context, a request without one, or a non-htmx request renders as usual.
#[tokio::test]
async fn a_live_region_request_from_another_context_reloads_the_page() {
    for stale in [
        "another-tenant%7CR4%7Cen",
        "dash-import-idle%7CR5%7Cen",
        "dash-import-idle%7CR4%7Ces",
    ] {
        let response = send_as(
            IMPORT_IDLE_TENANT,
            &format!("/ui?window=30d&ctx={stale}"),
            true,
        )
        .await;
        assert_eq!(response.status(), 200, "{stale}");
        assert_eq!(
            response
                .headers()
                .get("HX-Refresh")
                .and_then(|v| v.to_str().ok()),
            Some("true"),
            "{stale}"
        );
        assert!(body_text(response).await.is_empty(), "{stale}: empty body");
    }

    let matching = send_as(
        IMPORT_IDLE_TENANT,
        "/ui?window=30d&ctx=dash-import-idle%7CR4%7Cen",
        true,
    )
    .await;
    assert!(matching.headers().get("HX-Refresh").is_none());
    let dom = Dom::page(&body_text(matching).await);
    // The region renders (dash_live panics otherwise).
    assert_watches_settled(&dom, "30d");

    // An unencoded separator is the same context.
    let raw = send_as(
        IMPORT_IDLE_TENANT,
        "/ui?window=30d&ctx=dash-import-idle|R4|en",
        true,
    )
    .await;
    assert!(raw.headers().get("HX-Refresh").is_none());

    let without = send_as(IMPORT_IDLE_TENANT, "/ui?window=30d", true).await;
    assert!(without.headers().get("HX-Refresh").is_none());
    dash_live(&Dom::page(&body_text(without).await));

    let not_htmx = send_as(
        IMPORT_IDLE_TENANT,
        "/ui?window=30d&ctx=another-tenant%7CR4%7Cen",
        false,
    )
    .await;
    assert!(not_htmx.headers().get("HX-Refresh").is_none());
    dash_live(&Dom::page(&body_text(not_htmx).await));
}

/// Sends `uri` under `tenant` with the given request headers.
async fn send_with(tenant: &str, uri: &str, headers: &[(&str, &str)]) -> axum::response::Response {
    let mut request = Request::get(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    app(tenant)
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// The headers of a live-region (`#dash-live`) htmx request.
const LIVE: &[(&str, &str)] = &[("HX-Request", "true"), ("HX-Target", "dash-live")];
/// The headers of a type-picker (`#dash-chart`) htmx request.
const CHART: &[(&str, &str)] = &[("HX-Request", "true"), ("HX-Target", "dash-chart")];

/// Parses a `#dash-live` fragment response, asserting the region is its one
/// and only top-level element.
async fn live_fragment(response: axum::response::Response) -> Dom {
    let dom = Dom::fragment(&body_text(response).await);
    let root = dom.root();
    assert!(root.is("div#dash-live"), "{root:?}");
    dom
}

/// Asserts the body is the whole page: layout, sidebar and the live region.
fn assert_full_page(body: &str, context: &str) {
    let dom = Dom::page(body);
    dom.one("head title");
    assert_eq!(dom.count("#sidebar"), 1, "{context}: the layout's sidebar");
    dash_live(&dom);
}

/// #1078 phase 5: the dashboard answers by `HX-Target`. A plain load and a
/// history restore get the whole page; the live region's request gets just
/// `#dash-live`; the picker's request gets just the chart card and pushes the
/// selection's own link, free of every request-only parameter.
#[tokio::test]
async fn the_dashboard_answers_each_region_with_its_own_block() {
    let uri = "/ui?types=Patient&window=30d";

    let plain = body_text(send_with(IMPORT_IDLE_TENANT, uri, &[]).await).await;
    assert_full_page(&plain, "a plain load is the whole page");

    let restore = send_with(
        IMPORT_IDLE_TENANT,
        uri,
        &[
            ("HX-Request", "true"),
            ("HX-Target", "dash-live"),
            ("HX-History-Restore-Request", "true"),
        ],
    )
    .await;
    assert!(restore.headers().get("HX-Push-Url").is_none());
    assert_full_page(
        &body_text(restore).await,
        "a history restore rebuilds the whole page",
    );

    let other = send_with(
        IMPORT_IDLE_TENANT,
        uri,
        &[("HX-Request", "true"), ("HX-Target", "main")],
    )
    .await;
    assert_full_page(&body_text(other).await, "another target");

    let live = send_with(IMPORT_IDLE_TENANT, uri, LIVE).await;
    assert_eq!(live.status(), 200);
    assert!(live.headers().get("HX-Push-Url").is_none());
    // The region alone: its one root, no layout around it.
    let live = live_fragment(live).await;
    assert_eq!(live.count("#sidebar"), 0, "no layout");
    assert_eq!(live.count("title"), 0, "no layout");
    live.root().one("section#dash-chart");
    assert_watches_settled(&live, "30d");

    let chart = send_with(
        IMPORT_IDLE_TENANT,
        "/ui?types=Patient&window=30d&ctx=dash-import-idle%7CR4%7Cen&open=pick,table\
         &state=0123456789abcdef&notices=live&retry=2",
        CHART,
    )
    .await;
    assert_eq!(chart.status(), 200);
    let pushed = chart
        .headers()
        .get("HX-Push-Url")
        .and_then(|v| v.to_str().ok())
        .expect("the picker pushes its selection")
        .to_string();
    assert_eq!(pushed, "/ui?types=Patient&window=30d");
    let chart = Dom::fragment(&body_text(chart).await);
    let root = chart.root();
    assert!(root.is("section#dash-chart.card.chart-card"), "{root:?}");
    assert_eq!(chart.count("#dash-live"), 0, "only the card");
    assert_eq!(chart.count("#sidebar"), 0);
    assert_eq!(chart.count("title"), 0);
}

/// The link a picker request pushes keeps the whole chart state — the "View
/// all resources" toggle and a still-charted focus included.
#[tokio::test]
async fn the_pushed_url_is_the_selections_own_link() {
    let response = send_with(
        IMPORT_IDLE_TENANT,
        "/ui?types=Patient&window=24h&all=1&focus=Patient&ctx=dash-import-idle%7CR4%7Cen",
        CHART,
    )
    .await;
    let pushed = response
        .headers()
        .get("HX-Push-Url")
        .and_then(|v| v.to_str().ok())
        .unwrap();
    assert_eq!(pushed, "/ui?types=Patient&window=24h&all=1&focus=Patient");
    assert!(!pushed.contains("ctx"));
}

/// Fetches the live region with `state` set to what a fresh render shows,
/// retrying once so a digest crossing a minute boundary cannot flake.
async fn live_with_matching_state(tenant: &str, uri: &str) -> axum::response::Response {
    let mut last = None;
    for _ in 0..2 {
        let state = dash_state(&live_fragment(send_with(tenant, uri, LIVE).await).await);
        let response = send_with(tenant, &format!("{uri}&state={state}"), LIVE).await;
        if response.status() == 204 {
            return response;
        }
        last = Some(response);
    }
    last.unwrap()
}

/// #1078 phase 5: a settled tick whose figures are unchanged is answered
/// `204` with no body, so htmx swaps nothing; a different digest gets the
/// region.
#[tokio::test]
async fn an_unchanged_settled_tick_is_answered_no_content() {
    let uri = "/ui?types=Patient&window=30d";
    let same = live_with_matching_state(IMPORT_IDLE_TENANT, uri).await;
    assert_eq!(same.status(), 204);
    assert!(body_text(same).await.is_empty());

    let differs = send_with(
        IMPORT_IDLE_TENANT,
        &format!("{uri}&state=0000000000000000"),
        LIVE,
    )
    .await;
    assert_eq!(differs.status(), 200);
    live_fragment(differs).await;

    // Not a digest: ignored, so never a match.
    let junk = send_with(IMPORT_IDLE_TENANT, &format!("{uri}&state=not-hex"), LIVE).await;
    assert_eq!(junk.status(), 200);

    // A plain load with a matching digest is still the page.
    let state = dash_state(&get_as(IMPORT_IDLE_TENANT, uri).await);
    let plain = send_with(IMPORT_IDLE_TENANT, &format!("{uri}&state={state}"), &[]).await;
    assert_eq!(plain.status(), 200);
}

/// Figures that can still move, and a waiting page's slow watch, always get
/// the region back, even when the digest matches.
#[tokio::test]
async fn moving_or_waiting_figures_are_never_answered_no_content() {
    for (tenant, uri) in [
        (APPROXIMATE_TENANT, "/ui?window=30d"),
        (IMPORT_ACTIVE_TENANT, "/ui?window=30d"),
        (FIGURES_PENDING_TENANT, "/ui?window=30d&retry=3"),
    ] {
        let response = live_with_matching_state(tenant, uri).await;
        assert_eq!(response.status(), 200, "{tenant}");
        live_fragment(response).await;
    }
}

/// #1078 phase 5: what the page has open comes back open from the server —
/// the picker preserved as-is on a refresh, rendered open (and replaced) on
/// its own request — and unknown tokens are ignored.
#[tokio::test]
async fn open_controls_are_rendered_open() {
    let uri = "/ui?types=Patient&window=30d";

    let dom =
        live_fragment(send_with(IMPORT_IDLE_TENANT, &format!("{uri}&open=pick,table"), LIVE).await)
            .await;
    let pick = dom.one("details#chart-pick");
    assert!(
        pick.has_attr("open") && pick.has_attr("hx-preserve"),
        "{pick:?}"
    );
    let table = dom.one("details#chart-table");
    assert!(table.has_attr("open"), "{table:?}");
    // Never a preserved element inside a preserved one: the kept picker
    // carries its filter along.
    let filter = dom.one("#chart-pick-filter");
    assert!(!filter.has_attr("hx-preserve"), "{filter:?}");

    let dom = live_fragment(
        send_with(IMPORT_IDLE_TENANT, &format!("{uri}&open=table,bogus"), LIVE).await,
    )
    .await;
    let pick = dom.one("details#chart-pick");
    assert!(!pick.has_attr("open"), "{pick:?}");
    assert!(!pick.has_attr("hx-preserve"), "{pick:?}");
    assert!(dom.one("#chart-pick-filter").has_attr("hx-preserve"));
    assert!(dom.one("details#chart-table").has_attr("open"));

    let dom = live_fragment(
        send_with(
            IMPORT_IDLE_TENANT,
            &format!("{uri}&open=picker,tables"),
            LIVE,
        )
        .await,
    )
    .await;
    assert!(!dom.one("details#chart-pick").has_attr("open"));
    assert!(!dom.one("details#chart-table").has_attr("open"));

    let plain = get_as(IMPORT_IDLE_TENANT, uri).await;
    assert!(!plain.one("details#chart-pick").has_attr("open"));
    assert!(!plain.one("details#chart-table").has_attr("open"));

    let chart = Dom::fragment(&body_text(send_with(IMPORT_IDLE_TENANT, uri, CHART).await).await);
    assert!(chart.root().is("section#dash-chart"));
    let pick = chart.one("details#chart-pick");
    assert!(pick.has_attr("open"), "{pick:?}");
    assert!(
        !pick.has_attr("hx-preserve"),
        "its checkboxes must update: {pick:?}"
    );
}

/// A picker request from another context reloads the page like a refresh.
#[tokio::test]
async fn a_picker_request_from_another_context_reloads_the_page() {
    let response = send_with(
        IMPORT_IDLE_TENANT,
        "/ui?types=Patient&window=30d&ctx=another-tenant%7CR4%7Cen",
        CHART,
    )
    .await;
    assert_eq!(
        response
            .headers()
            .get("HX-Refresh")
            .and_then(|v| v.to_str().ok()),
        Some("true")
    );
    assert!(response.headers().get("HX-Push-Url").is_none());
    assert!(body_text(response).await.is_empty());
}

/// The htmx wiring the client relies on: each picker option (and "View all
/// resources") re-requests its own link into `#dash-chart`, the live region's
/// polls drop while a picker request is in flight, and the filter survives
/// both swaps.
#[tokio::test]
async fn the_picker_and_region_carry_their_htmx_wiring() {
    for (tenant, uri) in [
        (IMPORT_IDLE_TENANT, "/ui?types=Patient&window=30d"),
        ("default", "/ui?window=1h"),
    ] {
        let dom = get_as(tenant, uri).await;
        let live = dash_live(&dom);
        assert_eq!(live.attr("hx-sync"), Some("this:drop"), "{uri}: {live:?}");
        assert_eq!(
            live.attr("hx-select"),
            Some("#dash-live"),
            "{uri}: {live:?}"
        );

        let options = dom.all("#chart-pick a.chart-pick__option");
        for option in &options {
            let href = option.attr("href").expect("a no-JS link");
            assert_eq!(option.attr("hx-get"), Some(href), "{option:?}");
            assert_eq!(option.attr("hx-target"), Some("#dash-chart"), "{option:?}");
            assert_eq!(option.attr("hx-select"), Some("#dash-chart"), "{option:?}");
            assert_eq!(option.attr("hx-swap"), Some("outerHTML"), "{option:?}");
            assert_eq!(
                option.attr("hx-sync"),
                Some("#dash-live:replace"),
                "{option:?}"
            );
        }
        assert!(!options.is_empty(), "{uri}: at least the view-all link");
        dom.one("#chart-pick a.chart-pick__option--all");

        let filter = dom.one("#chart-pick-filter");
        assert!(filter.has_attr("hx-preserve"), "{filter:?}");
        assert!(filter.has_attr("data-pick-filter"), "{filter:?}");

        // Window and legend links stay plain navigations.
        let navigations = dom.all(".window-picker a, .chart-legend a");
        assert!(!navigations.is_empty(), "{uri}: the window links");
        for link in navigations {
            assert!(
                link.attr_names()
                    .iter()
                    .all(|name| !name.starts_with("hx-")),
                "{uri}: {link:?}"
            );
        }
    }
}
