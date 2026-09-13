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
//! Further tenants script the #1078 provider flags: [`TOTALS_PENDING_TENANT`]
//! (the seeding read only queued, nothing measured) and
//! [`COUNTS_UNSUPPORTED_TENANT`] (a backend that cannot count).
//!
//! A waiting page retries a bounded number of times. Once that budget is
//! spent, a page waiting on a snapshot the provider did answer (series or
//! totals pending) keeps a slow watch at the settled cadence, so it still
//! picks up its figures; the cache's own cold state stops. A backend that
//! cannot count never polls. A ready
//! page always watches itself: every 5 seconds, marked moving, while its
//! figures can still move — they are approximate, or an import is running —
//! and every 10 seconds otherwise, so a tab opened on settled figures still
//! notices an import started later. Each ready render carries a state hash of
//! its figures (never of its "as of" time), so the client can tell a refresh
//! that changed something from one that did not.
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
/// A tenant whose provider has only queued the storage read that seeds it:
/// nothing is measured yet (`totals_pending`).
const TOTALS_PENDING_TENANT: &str = "dash-totals-pending";
/// A tenant on a storage backend that cannot count at all
/// (`counts_unsupported`).
const COUNTS_UNSUPPORTED_TENANT: &str = "dash-counts-unsupported";

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

        if tenant == TOTALS_PENDING_TENANT {
            // The provider contract: totals, `available` and `series` empty.
            return DashboardSnapshot {
                fhir_version: "R4".to_string(),
                window,
                totals_pending: true,
                ..Default::default()
            };
        }
        if tenant == COUNTS_UNSUPPORTED_TENANT {
            return DashboardSnapshot {
                fhir_version: "R4".to_string(),
                window,
                counts_unsupported: true,
                ..Default::default()
            };
        }

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
            totals_pending: false,
            counts_unsupported: false,
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
    body_text(response).await
}

async fn body_text(response: axum::response::Response) -> String {
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

/// The value of attribute `name` in an opening tag, if the tag carries it.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!(r#" {name}=""#);
    let at = tag.find(&needle)? + needle.len();
    let value = &tag[at..];
    Some(&value[..value.find('"').expect("the attribute value closes")])
}

/// The live region's figure-state hash, asserted to be 16 lowercase hex chars.
fn dash_state(html: &str) -> String {
    let tag = dash_live_tag(html);
    let state = attr(tag, "data-dash-state").unwrap_or_else(|| panic!("a state hash: {tag}"));
    assert_eq!(state.len(), 16, "{tag}");
    assert!(
        state
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "lowercase hex: {tag}"
    );
    state.to_string()
}

/// Asserts what every ready page's periodic refresh shares, whatever its
/// cadence: it runs every `seconds` behind the client's guard, swaps only the
/// live region, re-requests the same view without a retry count, carries a
/// state hash of its figures, and does not also run the bounded retry.
fn assert_refreshes_every(html: &str, window: &str, seconds: u32) {
    let tag = dash_live_tag(html);
    assert_eq!(
        attr(tag, "data-dash-refresh"),
        Some(seconds.to_string().as_str()),
        "{tag}"
    );
    assert!(tag.contains(&format!("every {seconds}s")), "{tag}");
    assert!(tag.contains("hx-select=\"#dash-live\""), "{tag}");
    let href = attr(tag, "hx-get").expect("the refresh re-requests the page");
    assert!(href.starts_with("/ui?"), "{href}");
    assert!(href.contains(&format!("window={window}")), "{href}");
    assert!(!href.contains("retry="), "a refresh is not a retry: {href}");
    dash_state(html);
    assert!(
        !html.contains("hx-trigger=\"load"),
        "a ready page does not also run the bounded retry"
    );
    assert!(!html.contains("load delay"), "nor any delayed load");
}

/// Asserts the page polls itself as a ready page with moving figures does:
/// every 5 seconds, marked moving.
fn assert_polls_periodically(html: &str, window: &str) {
    assert_refreshes_every(html, window, 5);
    let tag = dash_live_tag(html);
    assert_eq!(attr(tag, "data-dash-moving"), Some("1"), "{tag}");
    assert!(!html.contains("every 10s"), "one cadence at a time");
}

/// Asserts the page watches settled figures as a ready page does: every 10
/// seconds, not marked moving, so an import started later is still noticed.
fn assert_watches_settled(html: &str, window: &str) {
    assert_refreshes_every(html, window, 10);
    let tag = dash_live_tag(html);
    assert!(!tag.contains("data-dash-moving"), "{tag}");
    assert!(!html.contains("every 5s"), "one cadence at a time");
}

/// Asserts the page keeps the slow watch of a waiting page whose fast retries
/// are spent: the settled cadence, marked waiting, re-requesting the same view
/// with the spent retry count kept, and no bounded retry.
fn assert_watches_while_waiting(html: &str, window: &str) {
    let tag = dash_live_tag(html);
    assert_eq!(attr(tag, "data-dash-refresh"), Some("10"), "{tag}");
    assert_eq!(attr(tag, "data-dash-waiting"), Some("1"), "{tag}");
    assert!(!tag.contains("data-dash-moving"), "{tag}");
    assert!(tag.contains("every 10s"), "{tag}");
    assert!(tag.contains("hx-select=\"#dash-live\""), "{tag}");
    let href = attr(tag, "hx-get").expect("the watch re-requests the page");
    assert!(href.starts_with("/ui?"), "{href}");
    assert!(href.contains(&format!("window={window}")), "{href}");
    assert!(
        href.contains("retry=3"),
        "the spent budget stays spent: {href}"
    );
    dash_state(html);
    assert!(!html.contains("load delay"), "no fast retry any more");
    assert!(!html.contains("every 5s"));
}

/// Asserts nothing on the page refreshes periodically, at either cadence.
fn assert_no_periodic_refresh(html: &str) {
    assert!(!html.contains("data-dash-refresh"));
    assert!(!html.contains("data-dash-moving"));
    assert!(!html.contains("every 5s"));
    assert!(!html.contains("every 10s"));
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
    // The cache's own cold state keeps no slow watch (#1078): a server too
    // busy to fill it is not polled.
    assert_no_periodic_refresh(&html);
    assert!(!html.contains("data-dash-waiting"));
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

    // The auto-refresh budget applies here too — but the page does not go
    // dead once it is spent (#1078): it watches slowly for its series.
    let spent = get_as(WARM_SIBLING_TENANT, "/ui?types=Patient&window=1h&retry=3").await;
    assert!(spent.contains(r#"data-dash-notice="series-pending""#));
    assert!(!spent.contains("hx-trigger=\"load"), "budget spent");
    assert!(spent.contains("Retry now"));
    assert_watches_while_waiting(&spent, "1h");
    assert!(
        attr(dash_live_tag(&spent), "hx-get").is_some_and(|href| href.contains("types=Patient")),
        "the watch keeps the requested selection"
    );
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
    // Spent, it watches slowly — never at the moving cadence, even though
    // the figures it borrowed are approximate.
    assert_watches_while_waiting(&spent, "1h");
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

/// An exact snapshot that reports zero running imports is settled, but the
/// page still watches it — slowly, and not marked moving — so a tab opened
/// now notices an import started later.
#[tokio::test]
async fn an_exact_snapshot_with_no_active_import_watches_slowly() {
    let html = get_as(IMPORT_IDLE_TENANT, "/ui?window=30d").await;

    assert!(html.contains("chart-data"), "the chart renders");
    assert!(html.contains(r#"data-dash-notice="live""#));
    assert!(!html.contains("hx-trigger=\"load"));
    assert_watches_settled(&html, "30d");
}

/// The state hash follows the figures, not the moment they were rendered:
/// the same view asked for twice carries the same state.
#[tokio::test]
async fn the_same_figures_rendered_twice_carry_the_same_state() {
    let uri = "/ui?types=Patient&window=30d";
    let first = get_as(IMPORT_IDLE_TENANT, uri).await;
    let second = get_as(IMPORT_IDLE_TENANT, uri).await;
    let third = get_as(IMPORT_IDLE_TENANT, uri).await;

    for html in [&first, &second, &third] {
        assert_watches_settled(html, "30d");
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
    let sibling = get_as(WARM_SIBLING_TENANT, "/ui?types=Patient&window=30d").await;

    // Both settled and ready, so only the figures differ.
    assert_watches_settled(&idle, "30d");
    assert_watches_settled(&sibling, "30d");
    assert!(sibling.contains(r#"<span class="stat__value">777</span>"#));
    assert!(!idle.contains(r#"<span class="stat__value">777</span>"#));
    assert_ne!(dash_state(&idle), dash_state(&sibling));
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
    // Nothing is waiting and nothing is approximate or importing: a ready
    // page, watched at the settled cadence.
    assert_watches_settled(&html, "24h");
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
    // Exact, complete, and no import reported: nothing moves right now, but
    // the page still watches, slowly, for figures that start moving later.
    assert_watches_settled(&html, "30d");
}

/// #1078: a provider that only queued the read seeding the tenant answers
/// with nothing measured. The page is the cold waiting page — unknown figures,
/// the waiting notice undated, the bounded retry — and never a row of zeros.
#[tokio::test]
async fn a_tenant_whose_totals_are_pending_renders_waiting_not_zeros() {
    let html = get_as(TOTALS_PENDING_TENANT, "/ui?types=Patient&window=30d").await;

    assert!(html.contains(r#"data-dash-notice="pending""#));
    assert!(html.contains("Still gathering the live figures"));
    assert!(!html.contains(r#"data-dash-notice="live""#));
    assert!(
        !html.contains("<time "),
        "nothing was read, so no \"as of\""
    );
    assert!(
        html.matches("stat__value--unavailable").count() >= 3,
        "resource types, stored resources and the chart total read as unknown"
    );
    assert!(
        !html.contains(r#"<span class="stat__value">0</span>"#),
        "no figure renders as zero"
    );
    assert!(html.contains("Waiting for the live figures"));
    assert!(!html.contains("chart-data"));
    assert!(
        !html.contains("Nothing to chart yet"),
        "nothing on the page claims the tenant is empty while it waits"
    );
    // The selectors survive the wait with the requested selection.
    assert!(html.contains(r#"href="/ui?types=Patient&window=24h""#));

    assert!(html.contains(r#"hx-trigger="load delay:1200ms""#));
    assert!(html.contains("retry=1"));
    assert!(html.contains("Retry now"));
    assert_no_periodic_refresh(&html);
}

/// #1078: with nothing requested and nothing known yet, the type picker is
/// empty — and says it is waiting, not that there is nothing to chart.
#[tokio::test]
async fn an_empty_type_picker_on_a_waiting_page_says_it_is_waiting() {
    let html = get_as(TOTALS_PENDING_TENANT, "/ui?window=30d").await;

    assert!(
        html.contains(r#"<p class="chart-pick__none">Waiting for the live figures"#),
        "the empty picker names the wait"
    );
    assert!(
        !html.contains("Nothing to chart yet"),
        "an empty picker on a waiting page does not read as an empty tenant"
    );
}

/// #1078: once the fast retries are spent, a page waiting on seeding totals
/// keeps watching slowly, keeping the spent count — while the cache's own cold
/// state (`the_automatic_retry_stops_after_its_budget`) stops.
#[tokio::test]
async fn a_spent_totals_pending_page_watches_slowly() {
    let html = get_as(
        TOTALS_PENDING_TENANT,
        "/ui?types=Patient&window=30d&retry=3",
    )
    .await;

    assert!(html.contains(r#"data-dash-notice="pending""#));
    assert!(
        !html.contains(r#"<span class="stat__value">0</span>"#),
        "still no zeros"
    );
    assert!(html.contains("Retry now"));
    assert_watches_while_waiting(&html, "30d");

    let cold = get("/ui?window=1h&all=1&retry=3").await;
    assert_no_periodic_refresh(&cold);
    assert!(!cold.contains("data-dash-waiting"));
}

/// #1078: a backend that cannot count says so, once, as a plain label: no
/// zeros, no waiting, no "as of", no retry and no polling of any kind.
#[tokio::test]
async fn a_backend_that_cannot_count_says_so_and_never_polls() {
    for uri in [
        "/ui?types=Patient&window=30d",
        "/ui?types=Patient&window=24h&retry=3",
    ] {
        let html = get_as(COUNTS_UNSUPPORTED_TENANT, uri).await;

        let tag = notice_tag(&html, "unsupported");
        assert!(
            !tag.contains("notice--warn"),
            "a label, not a warning: {tag}"
        );
        assert_eq!(html.matches("data-dash-notice=").count(), 1, "{uri}");
        assert!(html.contains("This storage backend cannot count stored resources"));
        assert!(
            html.contains(r#"<div class="chart-empty">Resource counts are not available for this storage backend.</div>"#),
            "{uri}: the chart area names the fact"
        );
        assert!(!html.contains("Waiting for the live figures"));
        assert!(!html.contains("Still gathering the live figures"));
        assert!(!html.contains("Nothing to chart yet"));
        assert!(!html.contains("chart-data"));
        assert!(!html.contains("<time "));
        assert!(!html.contains("Retry now"));
        assert!(
            html.matches("stat__value--unavailable").count() >= 3,
            "{uri}: the figures read as unavailable"
        );
        assert!(
            !html.contains(r#"<span class="stat__value">0</span>"#),
            "{uri}"
        );

        let live = dash_live_tag(&html);
        assert!(!live.contains("hx-get"), "{uri}: nothing polls: {live}");
        assert!(!live.contains("hx-trigger"), "{uri}: {live}");
        assert!(!live.contains("data-dash-waiting"), "{uri}: {live}");
        assert_no_periodic_refresh(&html);
    }
}

/// #1078: the live region names the tenant, FHIR version and locale it was
/// rendered for, so its requests can send them back.
#[tokio::test]
async fn the_live_region_names_its_context() {
    let ready = get_as(IMPORT_IDLE_TENANT, "/ui?window=30d").await;
    let ctx = attr(dash_live_tag(&ready), "data-dash-ctx").expect("a context on a ready page");
    assert_eq!(ctx, "dash-import-idle|R4|en");

    let waiting = get("/ui?window=1h&all=1").await;
    assert_eq!(
        attr(dash_live_tag(&waiting), "data-dash-ctx"),
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
    let html = body_text(matching).await;
    assert!(html.contains(r#"id="dash-live""#), "the region renders");
    assert_watches_settled(&html, "30d");

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
    assert!(body_text(without).await.contains(r#"id="dash-live""#));

    let not_htmx = send_as(
        IMPORT_IDLE_TENANT,
        "/ui?window=30d&ctx=another-tenant%7CR4%7Cen",
        false,
    )
    .await;
    assert!(not_htmx.headers().get("HX-Refresh").is_none());
    assert!(body_text(not_htmx).await.contains(r#"id="dash-live""#));
}
