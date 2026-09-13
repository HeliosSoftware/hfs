//! #956 / #1078: the Dashboard's degraded and qualified states, rendered over
//! HTTP.
//!
//! The bug this pins down: switching the time window during an import made the
//! page show a fabricated Patient/Observation/Encounter/Condition curve under
//! the notice "no live metrics provider is registered on this build" — while a
//! provider was registered and merely slow. The window is part of the snapshot
//! cache key, so every window switch is a cold key and takes the timeout path.
//! #1078 then asked that a slow window not blank the figures the page already
//! knows, and that figures which are stale or approximate say so.
//!
//! Registers its own [`DashboardProvider`] (process-global state), so this
//! lives in its own test binary. The provider discriminates on the window and
//! the tenant, both part of the cache key:
//!
//! - `1h` never answers (it sleeps past the end of the test), so it is
//!   permanently the cold-timeout case;
//! - `24h` answers with `partial` set, the half-failed case;
//! - `30d` answers completely — with `approximate` set under
//!   [`APPROXIMATE_TENANT`] and [`APPROXIMATE_WAITING_TENANT`], with active
//!   imports reported under [`IMPORT_ACTIVE_TENANT`] (and none under
//!   [`IMPORT_IDLE_TENANT`]), and with its own distinctive figures under
//!   [`WARM_SIBLING_TENANT`].
//!
//! A waiting page retries a bounded number of times; a ready page polls
//! itself every 10 seconds only while its figures can still move — they are
//! approximate, or an import is running — and never otherwise.
//!
//! The snapshot cache is process-global too, so the cases are kept apart:
//!
//! - The truly cold case is always requested with `all=1` under the default
//!   tenant. Since #1078 the cache answers a cold window from a snapshot the
//!   same tenant has cached under another window with the same "View all
//!   resources" setting (totals kept, series marked pending), so a `1h`
//!   request without the flag would borrow the `24h`/`30d` figures whenever
//!   those tests ran first in this binary. No other default-tenant case sets
//!   `all=1`, so that `1h` key has no sibling and stays a truly cold tenant.
//! - The warm-sibling, approximate and import cases mount the UI under a
//!   tenant of their own (the mount's default tenant is the request's tenant), so their
//!   cache entries are never anyone else's sibling.

use async_trait::async_trait;
use axum::{Router, body::Body, http::Request};
use chrono::{DateTime, Utc};
use helios_observability::dashboard::{
    DashboardPoint, DashboardProvider, DashboardSeries, DashboardSnapshot, DashboardWindow,
    TypeCount, set_provider,
};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// A tenant whose `30d` snapshot is measured but not reconciled with storage.
const APPROXIMATE_TENANT: &str = "dash-approximate";
/// A tenant whose `30d` snapshot is warm when its `1h` window is asked for.
const WARM_SIBLING_TENANT: &str = "dash-warm-sibling";
/// A tenant whose approximate `30d` snapshot is warm when its `1h` window is
/// asked for: a page that is both waiting and approximate.
const APPROXIMATE_WAITING_TENANT: &str = "dash-approximate-waiting";
/// A tenant whose `30d` snapshot is exact but reports two running imports.
const IMPORT_ACTIVE_TENANT: &str = "dash-import-active";
/// A tenant whose `30d` snapshot is exact and reports no running import.
const IMPORT_IDLE_TENANT: &str = "dash-import-idle";

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
        if window == DashboardWindow::LastHour {
            // Far past the 800ms cold-load budget and past the test's own
            // lifetime, so this key stays pending however often it is asked
            // for — the storage contention the real bug happens under, held
            // still.
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

        if tenant == WARM_SIBLING_TENANT {
            // Figures no other case produces, so the page can only show them
            // by serving this snapshot's totals.
            return DashboardSnapshot {
                fhir_version: "R4".to_string(),
                total_resources: 777,
                distinct_types: 7,
                window,
                series: vec![series("Patient", 700)],
                available: vec![count("Patient", 700), count("Observation", 77)],
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
            export_jobs: None,
            import_jobs_active: match tenant {
                IMPORT_ACTIVE_TENANT => Some(2),
                IMPORT_IDLE_TENANT => Some(0),
                _ => None,
            },
            partial: window == DashboardWindow::LastDay,
            // Left unset on purpose: the cache stamps it, and the page must
            // still say when the figures were read.
            generated_at: None,
            approximate: tenant == APPROXIMATE_TENANT || tenant == APPROXIMATE_WAITING_TENANT,
            series_pending: false,
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

async fn get_as(tenant: &str, uri: &str) -> String {
    let response = app(tenant)
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    // Links are asserted as written; the escaping of `&` in attributes is
    // the template engine's business, not the page's.
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .replace("&amp;", "&")
        .replace("&#38;", "&")
}

async fn get(uri: &str) -> String {
    get_as("default", uri).await
}

/// The opening tag of the swappable live region, where its refresh lives.
fn dash_live_tag(html: &str) -> &str {
    let start = html
        .find(r#"<div id="dash-live""#)
        .expect("the live region");
    let end = start + html[start..].find('>').expect("its opening tag closes");
    &html[start..=end]
}

/// The opening `<p>` tag of the notice line with this slug.
fn notice_tag<'a>(html: &'a str, slug: &str) -> &'a str {
    let at = html
        .find(&format!(r#"data-dash-notice="{slug}">"#))
        .unwrap_or_else(|| panic!("the {slug} notice line"));
    let start = html[..at].rfind("<p ").expect("the line's <p>");
    let end = at + html[at..].find('>').unwrap();
    &html[start..=end]
}

/// Asserts the page polls itself as a ready page does: every 10 seconds,
/// swapping only the live region, from the same view without a retry count.
fn assert_polls_periodically(html: &str, window: &str) {
    let tag = dash_live_tag(html);
    assert!(tag.contains(r#"data-dash-refresh="5""#), "{tag}");
    assert!(tag.contains("every 5s"), "{tag}");
    assert!(tag.contains("hx-select=\"#dash-live\""), "{tag}");
    let href_at = tag
        .find(r#"hx-get=""#)
        .expect("the refresh re-requests the page");
    let href = &tag[href_at + r#"hx-get=""#.len()..];
    let href = &href[..href.find('"').unwrap()];
    assert!(href.starts_with("/ui?"), "{href}");
    assert!(href.contains(&format!("window={window}")), "{href}");
    assert!(!href.contains("retry="), "a refresh is not a retry: {href}");
    assert!(
        !html.contains("hx-trigger=\"load"),
        "a ready page does not also run the bounded retry"
    );
}

/// Asserts nothing on the page polls periodically.
fn assert_no_periodic_refresh(html: &str) {
    assert!(!html.contains("data-dash-refresh"));
    assert!(!html.contains("every 5s"));
}

/// The regression itself: a window whose snapshot has not landed says it is
/// waiting — it does not claim the build has no metrics, and it invents
/// nothing.
#[tokio::test]
async fn a_slow_window_renders_waiting_not_sample_data() {
    let html = get("/ui?window=1h&all=1").await;

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
    // A truly cold tenant has no reading to date and no figures to qualify.
    assert!(
        !html.contains("<time "),
        "nothing was read, so no \"as of\""
    );
    assert!(!html.contains(r#"data-dash-notice="series-pending""#));
}

/// Waiting is recoverable both ways: htmx re-requests the live region on its
/// own, and the notice always carries a plain link for a browser without it.
#[tokio::test]
async fn the_waiting_page_offers_an_automatic_and_a_manual_retry() {
    let html = get("/ui?window=1h&all=1").await;

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
    assert!(
        html.contains(r#"hx-trigger="load delay:1200ms""#),
        "waiting keeps its bounded retry"
    );
    assert_no_periodic_refresh(&html);
}

/// The auto-refresh is budgeted: the page stops re-requesting itself after a
/// bounded number of attempts, leaving only the manual link. The wait is
/// caused by load, so the recovery must not add to it indefinitely.
#[tokio::test]
async fn the_automatic_retry_stops_after_its_budget() {
    let html = get("/ui?window=1h&all=1&retry=3").await;

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
    assert_no_periodic_refresh(&html);
}

/// #1078: a tenant whose figures are already cached under another window
/// keeps them on screen when it switches to a window that is slow to compute.
/// The cards and the type picker render from the borrowed snapshot, the chart
/// area alone waits (never an empty chart), the notice says when the figures
/// were read, and the page keeps retrying until the window's own series land.
#[tokio::test]
async fn a_slow_window_with_a_warm_sibling_keeps_the_figures_and_waits_for_the_chart() {
    // Warm the tenant's 30d key with the same selection.
    let warm = get_as(WARM_SIBLING_TENANT, "/ui?types=Patient&window=30d").await;
    assert!(warm.contains("chart-data"), "the 30d chart renders");

    let html = get_as(WARM_SIBLING_TENANT, "/ui?types=Patient&window=1h").await;

    // The headline figures are the sibling's real ones, not unknown.
    assert!(
        html.contains(r#"<span class="stat__value">7</span>"#),
        "resource types card: {html}"
    );
    assert!(
        html.contains(r#"<span class="stat__value">777</span>"#),
        "stored resources card"
    );
    // The picker offers the tenant's types with their counts, checked from
    // the requested selection, and a toggle keeps that selection.
    assert!(html.contains(r#"data-pick-name="Patient""#));
    assert!(html.contains(r#"data-pick-name="Observation""#));
    let option = |name: &str| {
        let at = html
            .find(&format!(r#"data-pick-name="{name}""#))
            .expect("picker option");
        let start = html[..at].rfind("<a ").expect("option anchor");
        html[start..at].to_string()
    };
    assert!(
        option("Patient").contains("chart-pick__option--on"),
        "the requested type is checked"
    );
    assert!(
        option("Patient").contains(r#"href="/ui?types=&window=1h""#),
        "unchecking it empties the requested selection"
    );
    assert!(!option("Observation").contains("chart-pick__option--on"));
    assert!(
        html.contains(r#"href="/ui?types=Patient,Observation&window=1h""#),
        "picking another type adds to the requested selection"
    );
    assert!(
        html.contains(r#"href="/ui?types=Patient&window=30d""#),
        "the window selector keeps the requested selection"
    );

    // The chart area alone waits: no chart, no chart data, no charted total.
    assert!(html.contains("Waiting for the live figures"));
    assert!(!html.contains(r#"<svg class="chart""#));
    assert!(!html.contains("chart-data"));
    assert!(
        !html.contains("Nothing to chart yet"),
        "a series still loading is not an empty chart"
    );
    assert!(
        !html.contains("Still gathering the live figures"),
        "not the cold-tenant notice"
    );
    assert!(!html.contains("no live metrics provider"));

    // The notice names the state, dates the figures, and offers both retries.
    assert!(html.contains(r#"data-dash-notice="series-pending""#));
    assert!(html.contains("The chart for this window is still loading"));
    assert!(html.contains(r#"<time datetime=""#), "figures are dated");
    assert!(html.contains("As of "));
    assert!(html.contains("Retry now"));
    assert!(
        html.contains("hx-trigger=\"load"),
        "the page re-requests itself while the series load"
    );
    assert!(html.contains("retry=1"));
    assert_no_periodic_refresh(&html);

    // The auto-refresh budget applies here too.
    let spent = get_as(WARM_SIBLING_TENANT, "/ui?types=Patient&window=1h&retry=3").await;
    assert!(spent.contains(r#"data-dash-notice="series-pending""#));
    assert!(!spent.contains("hx-trigger=\"load"), "budget spent");
    assert!(spent.contains("Retry now"));
    assert_no_periodic_refresh(&spent);
}

/// A page still waiting for its chart keeps the bounded retry even when the
/// figures it borrowed are approximate: the periodic refresh is for ready
/// pages only, never stacked on a retry nor outliving its budget.
#[tokio::test]
async fn a_waiting_page_with_approximate_figures_retries_and_does_not_poll() {
    let warm = get_as(APPROXIMATE_WAITING_TENANT, "/ui?types=Patient&window=30d").await;
    assert_polls_periodically(&warm, "30d");

    let html = get_as(APPROXIMATE_WAITING_TENANT, "/ui?types=Patient&window=1h").await;
    assert!(html.contains(r#"data-dash-notice="series-pending""#));
    assert!(html.contains(r#"hx-trigger="load delay:1200ms""#));
    assert!(html.contains("retry=1"));
    assert_no_periodic_refresh(&html);

    let spent = get_as(
        APPROXIMATE_WAITING_TENANT,
        "/ui?types=Patient&window=1h&retry=3",
    )
    .await;
    assert!(spent.contains(r#"data-dash-notice="series-pending""#));
    assert!(!spent.contains("hx-trigger=\"load"), "budget spent");
    assert_no_periodic_refresh(&spent);
}

/// #1078: figures counted from recent writes rather than read exactly from
/// storage chart normally, but are labelled approximate and dated. Nothing is
/// missing, so the page neither warns nor retries — but the figures keep
/// moving, so it refreshes itself periodically instead of waiting for a
/// reload.
#[tokio::test]
async fn an_approximate_snapshot_charts_with_a_dated_label_and_refreshes_periodically() {
    let html = get_as(APPROXIMATE_TENANT, "/ui?window=30d").await;

    assert!(html.contains(r#"<svg class="chart""#), "the chart renders");
    assert!(html.contains("chart-data"));
    assert!(html.contains(r#"<span class="stat__value">5</span>"#));
    assert!(html.contains(r#"data-dash-notice="approximate""#));
    assert!(html.contains("Approximate: counted from recent writes"));
    assert!(html.contains(r#"<time datetime=""#), "figures are dated");
    assert!(
        !html.contains("notice notice--warn"),
        "approximate figures are labelled, not flagged"
    );
    assert!(!html.contains("hx-trigger=\"load"), "nothing to wait for");
    assert!(!html.contains("Retry now"));
    assert!(!html.contains("Waiting for the live figures"));
    assert_polls_periodically(&html, "30d");
}

/// An exact snapshot still moves while an import runs, so the page polls.
#[tokio::test]
async fn an_exact_snapshot_with_an_active_import_refreshes_periodically() {
    let html = get_as(IMPORT_ACTIVE_TENANT, "/ui?window=30d").await;

    assert!(html.contains("chart-data"), "the chart renders");
    assert!(!html.contains(r#"data-dash-notice="approximate""#));
    assert!(html.contains(r#"data-dash-notice="live""#));
    assert!(!html.contains("Retry now"));
    assert_polls_periodically(&html, "30d");
}

/// An exact snapshot that reports zero running imports is settled: nothing
/// polls.
#[tokio::test]
async fn an_exact_snapshot_with_no_active_import_does_not_poll() {
    let html = get_as(IMPORT_IDLE_TENANT, "/ui?window=30d").await;

    assert!(html.contains("chart-data"), "the chart renders");
    assert!(html.contains(r#"data-dash-notice="live""#));
    assert!(!html.contains("hx-trigger=\"load"));
    assert_no_periodic_refresh(&html);
}

/// A periodic swap must not re-announce a notice the user already heard:
/// `notices=` names the lines the previous render showed, and those render
/// `aria-live="off"`. A line whose kind is not in the list is new, and is
/// announced.
#[tokio::test]
async fn notices_already_shown_are_not_announced_again() {
    let fresh = get_as(APPROXIMATE_TENANT, "/ui?window=30d").await;
    let tag = notice_tag(&fresh, "approximate");
    assert!(tag.contains(r#"aria-live="polite""#), "{tag}");

    let unchanged = get_as(APPROXIMATE_TENANT, "/ui?window=30d&notices=approximate").await;
    let tag = notice_tag(&unchanged, "approximate");
    assert!(tag.contains(r#"aria-live="off""#), "{tag}");
    assert!(!tag.contains(r#"aria-live="polite""#), "{tag}");

    let changed = get_as(APPROXIMATE_TENANT, "/ui?window=30d&notices=live").await;
    let tag = notice_tag(&changed, "approximate");
    assert!(tag.contains(r#"aria-live="polite""#), "{tag}");
    assert!(!tag.contains(r#"aria-live="off""#), "{tag}");
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
    assert!(html.contains(r#"<time datetime=""#), "and dated");
}

/// A complete snapshot carries no warning at all — the states above must not
/// leak into the ordinary page. It still says when its figures were read
/// (#1078): a snapshot can be served stale while a refresh runs.
#[tokio::test]
async fn a_complete_snapshot_carries_only_its_as_of_time() {
    let html = get("/ui?window=30d").await;

    assert!(!html.contains("no live metrics provider"));
    assert!(!html.contains("Still gathering the live figures"));
    assert!(!html.contains("Some figures could not be read from storage"));
    assert!(!html.contains("Approximate:"));
    assert!(!html.contains("notice notice--warn"));
    assert!(html.contains(r#"data-dash-notice="live""#));
    assert!(html.contains(r#"<time datetime=""#));
    assert!(!html.contains("hx-trigger=\"load"));
    assert!(html.contains("chart-data"), "the chart renders");
    // Exact, complete, and no import reported: nothing moves, nothing polls.
    assert_no_periodic_refresh(&html);
}
