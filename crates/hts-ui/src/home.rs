//! Home page + refresh fragment (design doc §7.1).
//!
//! Two entry points:
//! - [`home_page`] renders the full Home shell (topbar, sidebar, cards
//!   region). It always fetches once so the initial paint is not blank,
//!   and swaps the cards region on every subsequent htmx refresh.
//! - [`home_cards_fragment`] returns just the cards partial, wired to
//!   `hx-trigger="every 15s"` from the page shell.
//!
//! The upstream fetch fans out to `/health`, `/metadata?mode=terminology`,
//! and `/metrics` in parallel and renders together so the operator sees a
//! coherent picture even when one leg fails: a red status card + a healthy
//! capabilities card is better than a blank page.
//!
//! Renamed from `dashboard.rs` on 2026-08-20 for HFS parity — HFS calls its
//! landing route "home" (`nav-home`, `active_page: "home"`, module `home`),
//! and every hook here now matches that convention.

use askama::Template;
use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use axum_htmx::HxRequest;
use serde::Deserialize;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::chart::{self, ChartView};
use crate::i18n::{I18n, RequestLocale};
use crate::metrics_parse::{self, StatusCounts};
use crate::metrics_ring::{self, MetricsSample};
use crate::upstream::{UpstreamError, UpstreamHealth, UpstreamTerminologyCapabilities};
use crate::{Chrome, HtsUiState};

// ── Chart selectors ─────────────────────────────────────────────────────────

/// The time span the request-rate chart covers, ending at "now".
///
/// Both ends of the range matter: at the Home page's 15 s poll the sample
/// ring holds at most six hours, so `H6` is the widest window that can ever
/// be fully populated, and `M15` is dense enough that a single burst is
/// still visible as a spike rather than a one-pixel blip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChartWindow {
    /// The last 15 minutes — the default, and the only window a freshly
    /// opened Home page can fill.
    #[default]
    M15,
    H1,
    H6,
}

impl ChartWindow {
    /// Parse the `?window=` query value. Anything unrecognised falls back to
    /// the default rather than erroring: a hand-edited URL should render a
    /// chart, not a 400.
    fn parse(raw: Option<&str>) -> Self {
        match raw {
            Some("1h") => Self::H1,
            Some("6h") => Self::H6,
            _ => Self::M15,
        }
    }

    /// The query-string spelling, round-tripping [`Self::parse`].
    pub fn code(self) -> &'static str {
        match self {
            Self::M15 => "15m",
            Self::H1 => "1h",
            Self::H6 => "6h",
        }
    }

    /// Fluent key for the window-picker option label.
    pub fn label_key(self) -> &'static str {
        match self {
            Self::M15 => "hts-home-chart-window-15m",
            Self::H1 => "hts-home-chart-window-1h",
            Self::H6 => "hts-home-chart-window-6h",
        }
    }

    /// Fluent key for the *sentence* form used in the chart's hint line
    /// ("Last hour"), as opposed to [`Self::label_key`]'s chip form ("1h").
    ///
    /// The hint has to name the selected window: the V3 mockup's static
    /// "Last hour" would be false in two of the three windows this picker
    /// offers, and a caption that contradicts its own chart is worse than no
    /// caption at all.
    pub fn hint_key(self) -> &'static str {
        match self {
            Self::M15 => "hts-home-chart-hint-window-15m",
            Self::H1 => "hts-home-chart-hint-window-1h",
            Self::H6 => "hts-home-chart-hint-window-6h",
        }
    }

    fn seconds(self) -> f64 {
        match self {
            Self::M15 => 900.0,
            Self::H1 => 3_600.0,
            Self::H6 => 21_600.0,
        }
    }

    /// How many x-axis labels to draw. Chosen per window so every tick lands
    /// on a whole minute or hour: 15/12/9/6/3/0 min, 60/50/…/0 min, 6/5/…/0 h.
    fn tick_count(self) -> usize {
        match self {
            Self::M15 => 6,
            Self::H1 | Self::H6 => 7,
        }
    }

    /// Localized x-axis label for a tick `secs_before` seconds in the past.
    fn axis_label(self, i18n: &I18n, secs_before: f64) -> String {
        if secs_before < 1.0 {
            return i18n.t("hts-home-chart-axis-now");
        }
        match self {
            Self::H6 => i18n.t_arg(
                "hts-home-chart-axis-hours",
                "n",
                format!("{}", (secs_before / 3_600.0).round() as i64),
            ),
            _ => i18n.t_arg(
                "hts-home-chart-axis-minutes",
                "n",
                format!("{}", (secs_before / 60.0).round() as i64),
            ),
        }
    }

    const ALL: [Self; 3] = [Self::M15, Self::H1, Self::H6];
}

/// Which slice of `http_requests_total` the chart plots. Acts as the legend:
/// exactly one is active at a time, and the other three are links.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeriesFilter {
    #[default]
    All,
    S2xx,
    S4xx,
    S5xx,
}

impl SeriesFilter {
    fn parse(raw: Option<&str>) -> Self {
        match raw {
            Some("2xx") => Self::S2xx,
            Some("4xx") => Self::S4xx,
            Some("5xx") => Self::S5xx,
            _ => Self::All,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::S2xx => "2xx",
            Self::S4xx => "4xx",
            Self::S5xx => "5xx",
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            Self::All => "hts-home-chart-series-all",
            Self::S2xx => "hts-home-chart-series-2xx",
            Self::S4xx => "hts-home-chart-series-4xx",
            Self::S5xx => "hts-home-chart-series-5xx",
        }
    }

    /// Fluent key for the *sentence* form used in the chart's hint line
    /// ("all status classes" / "4xx responses only"), as opposed to
    /// [`Self::label_key`]'s chip form ("All" / "4xx"). Same reasoning as
    /// [`ChartWindow::hint_key`]: the caption must describe what is plotted.
    pub fn hint_key(self) -> &'static str {
        match self {
            Self::All => "hts-home-chart-hint-series-all",
            Self::S2xx => "hts-home-chart-hint-series-2xx",
            Self::S4xx => "hts-home-chart-hint-series-4xx",
            Self::S5xx => "hts-home-chart-hint-series-5xx",
        }
    }

    /// 1-based `--series-N` palette slot, shared by the line and the legend
    /// dot so the two always agree.
    pub fn color(self) -> usize {
        match self {
            Self::All => 1,
            Self::S2xx => 2,
            Self::S4xx => 3,
            Self::S5xx => 4,
        }
    }

    /// Pull this filter's counter out of a sample.
    fn pick(self, counts: &StatusCounts) -> u64 {
        match self {
            Self::All => counts.all,
            Self::S2xx => counts.s2xx,
            Self::S4xx => counts.s4xx,
            Self::S5xx => counts.s5xx,
        }
    }

    const ALL: [Self; 4] = [Self::All, Self::S2xx, Self::S4xx, Self::S5xx];
}

/// `?window=` / `?series=` — accepted on both the Home page and the refresh
/// fragment so a selection survives the 15 s poll (see [`refresh_href`]).
#[derive(Debug, Default, Deserialize)]
pub struct HomeQuery {
    window: Option<String>,
    series: Option<String>,
}

impl HomeQuery {
    fn selection(&self) -> (ChartWindow, SeriesFilter) {
        (
            ChartWindow::parse(self.window.as_deref()),
            SeriesFilter::parse(self.series.as_deref()),
        )
    }
}

/// Link back to the Home *page* carrying a chart selection. Window and legend
/// options are real `<a href>`s pointing here, so both work with JavaScript
/// off — a plain navigation re-renders the page with the new selection.
fn page_href(window: ChartWindow, series: SeriesFilter) -> String {
    format!("/ui/hts?window={}&series={}", window.code(), series.code())
}

/// The URL the cards region polls itself from.
///
/// This is the whole htmx contract for the chart. `#hts-home-cards` carries
/// its own `hx-get` with **no `hx-target`**, so htmx defaults the target to
/// the element itself and `hx-swap="outerHTML"` replaces it *including this
/// refreshed `hx-get`*. Because the URL carries the current selection, every
/// tick re-renders the same window and series, and a click that changes them
/// changes what the next tick asks for.
///
/// The previous shape — an outer, never-swapped `<div hx-get … hx-target>`
/// wrapper — could not do this: the wrapper survived every swap, so its
/// frozen URL would revert any selection on the next 15 s tick.
fn refresh_href(window: ChartWindow, series: SeriesFilter) -> String {
    format!(
        "/ui/hts/home/cards?window={}&series={}",
        window.code(),
        series.code()
    )
}

/// Seconds since the UNIX epoch, as an `f64`.
///
/// Wall clock, not a monotonic instant, because samples must be comparable
/// across requests and `Instant` is not serialisable into a shared ring in a
/// meaningful way here. A clock step backwards is handled downstream: the
/// affected interval simply yields no point (see
/// [`crate::metrics_ring::rates`]).
fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Why the chart has no line to draw. Each maps to a Fluent key rendered in
/// the card's `.stat__sub` — the honest alternative to plotting a flat zero,
/// which would claim we observed silence when in fact we observed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartEmpty {
    /// `/metrics` could not be fetched at all on this tick.
    ///
    /// Outranks the other three, and is the one arm that can appear while a
    /// line is still drawn: the ring keeps whatever it observed before the
    /// upstream went away, and the card says the samples have stopped
    /// arriving rather than quietly presenting stale history as current.
    Unreachable,
    /// No sample has ever been recorded in this process.
    NoSamples,
    /// Exactly one sample: a rate needs two, so nothing is plottable yet.
    FirstInterval,
    /// Samples exist, but none fall inside the selected window — sampling
    /// only runs while somebody has this page open.
    NoneInWindow,
}

impl ChartEmpty {
    pub fn key(self) -> &'static str {
        match self {
            Self::Unreachable => "hts-home-chart-empty-unreachable",
            Self::NoSamples => "hts-home-chart-empty-none",
            Self::FirstInterval => "hts-home-chart-empty-first",
            Self::NoneInWindow => "hts-home-chart-empty-window",
        }
    }
}

/// One `.window-picker__option`.
#[derive(Clone, Debug)]
pub struct WindowOption {
    pub label_key: &'static str,
    pub href: String,
    pub active: bool,
}

/// One `.chart-legend__item`. Doubles as the series selector.
#[derive(Clone, Debug)]
pub struct LegendItem {
    pub label_key: &'static str,
    pub href: String,
    pub active: bool,
    pub color: usize,
    /// Requests of this status class observed inside the window, compact.
    pub total: String,
}

/// Everything `partials/hts-home-chart.html` renders.
#[derive(Clone, Debug)]
pub struct HomeChart {
    pub view: ChartView,
    pub windows: Vec<WindowOption>,
    pub legend: Vec<LegendItem>,
    /// `Some` when there is nothing to plot; the reason goes in `.stat__sub`.
    pub empty: Option<ChartEmpty>,
}

/// One shared card model, so the page and the refresh fragment render from
/// exactly the same shape. Never `Option<Option<_>>`: each leg either fetches
/// or degrades, and the outcome carries the reason.
#[derive(Clone, Debug)]
pub struct HomeCards {
    pub health: Result<UpstreamHealth, UpstreamError>,
    pub capabilities: Result<UpstreamTerminologyCapabilities, UpstreamError>,
    /// Advertised bundled data footprint in bytes, computed from
    /// `HTS_BOOTSTRAP_DIR` at mount time; `None` when no bootstrap directory
    /// was configured (the docker image or a bare `hts run` from a source
    /// tree). Renders as an em-dash in that arm.
    pub bundled_data_bytes: Option<u64>,
    /// Process-wide count of HTTP requests, from `http_requests_total` on
    /// `/metrics`. `None` when the fetch or parse fails — fail-open per
    /// design §7 degraded contract.
    pub requests_total: Option<u64>,
    /// Process-wide average request latency in milliseconds, from
    /// `http_request_duration_seconds` (histogram `sum / count`). `None`
    /// when the histogram hasn't recorded any samples yet or the fetch
    /// fails. Never a lie: zero counts render as em-dash, not "0 ms".
    pub avg_latency_ms: Option<f64>,
    /// The selected chart window, echoed into every self-link so the 15 s
    /// poll re-asks for the same view.
    pub window: ChartWindow,
    /// The selected status-class series, likewise.
    pub series: SeriesFilter,
    /// The request-rate chart, built from the sample ring on `HtsUiState`.
    pub chart: HomeChart,
}

impl HomeCards {
    /// Fetch every card leg and fold this scrape into the chart's sample ring.
    ///
    /// The `/metrics` body is fetched exactly **once** and used three ways —
    /// the requests tile, the latency tile, and the chart sample — so the
    /// chart costs no additional upstream traffic.
    async fn fetch(
        state: &HtsUiState,
        window: ChartWindow,
        series: SeriesFilter,
        i18n: &I18n,
    ) -> Self {
        let (health, capabilities, metrics_result) = tokio::join!(
            state.upstream.health(),
            state.upstream.terminology_capabilities(),
            state.upstream.metrics_text(),
        );

        // Fail-open: any failure in the metrics leg leaves both tiles as
        // em-dash without disturbing the other cards. The dashboard is
        // still useful with health + capabilities alone.
        let (requests_total, avg_latency_ms, metrics_ok) = match &metrics_result {
            Ok(text) => {
                let map = metrics_parse::parse(text);
                let requests = metrics_parse::sum_counter(&map, "http_requests_total")
                    .and_then(|v| if v.is_finite() { Some(v as u64) } else { None });
                let latency = metrics_parse::histogram_avg(&map, "http_request_duration_seconds")
                    .map(|seconds| seconds * 1000.0);

                // Fold this scrape into the ring. Self-traffic is excluded
                // *here*, before it is ever recorded, so a stale sample can
                // never reintroduce it. `uptime_seconds` rides along so the
                // ring can detect an upstream restart; a missing gauge is
                // treated as 0, which never looks like a regression against
                // a real uptime and so never spuriously clears the ring.
                let counts = metrics_parse::request_counts_by_status_class(
                    &map,
                    &metrics_parse::SELF_ROUTES,
                );
                let uptime_secs = metrics_parse::gauge(&map, "uptime_seconds").unwrap_or(0.0);
                // Synchronous, allocation-free critical section — no `.await`
                // is reachable from here while the ring's guard is held.
                state.metrics_ring.push(MetricsSample {
                    at_secs: now_secs(),
                    uptime_secs,
                    counts,
                });

                (requests, latency, true)
            }
            Err(_) => (None, None, false),
        };

        let chart = build_home_chart(&state.metrics_ring, window, series, metrics_ok, i18n);

        Self {
            health,
            capabilities,
            bundled_data_bytes: state.bundled_data_bytes,
            requests_total,
            avg_latency_ms,
            window,
            series,
            chart,
        }
    }

    /// The URL `#hts-home-cards` polls itself from — see [`refresh_href`].
    pub fn refresh_href(&self) -> String {
        refresh_href(self.window, self.series)
    }

    /// Any-leg failure surfaces as `Some(reason)` for the degraded banner.
    /// The Home page renders the banner **and** the successful cards below
    /// it — a partial degrade is still informative. The metrics leg is
    /// intentionally excluded from this check: `/metrics` unavailable
    /// hides two tiles behind em-dash but doesn't warrant the red banner.
    pub fn degraded_reason(&self) -> Option<&'static str> {
        match (&self.health, &self.capabilities) {
            (Err(e), _) | (_, Err(e)) => Some(e.degraded_reason()),
            _ => None,
        }
    }

    /// Loaded systems count, per `TerminologyCapabilities.codeSystem[]`.
    /// Returns `None` when the capabilities fetch failed — the tile then
    /// renders an em-dash instead of a zero, which would be a lie.
    pub fn loaded_system_count(&self) -> Option<usize> {
        self.capabilities
            .as_ref()
            .ok()
            .map(|c| c.loaded_system_count())
    }

    /// Bundled data footprint in mebibytes, rounded down. Rendered as prose
    /// so the Fluent placeable can localise the unit.
    pub fn bundled_data_mib(&self) -> Option<u64> {
        self.bundled_data_bytes.map(|b| b / (1024 * 1024))
    }

    /// FHIR version for the Status tile's sub-line.
    ///
    /// Prefers what the upstream actually advertises in
    /// `TerminologyCapabilities.fhirVersion`; falls back to the version this
    /// binary was compiled for when the fetch failed or the field is blank.
    /// The fallback is never wrong for a loopback deployment, and for a
    /// remote `HTS_UI_UPSTREAM_URL` the advertised value is the honest one —
    /// hence the preference order.
    ///
    /// Takes the fallback as an argument because it lives on `Chrome`, which
    /// the template owns; threading it through keeps `HomeCards` free of
    /// chrome state.
    pub fn fhir_version_label(&self, fallback: &str) -> String {
        match self.capabilities.as_ref() {
            Ok(c) if !c.fhir_version.is_empty() => c.fhir_version.clone(),
            _ => fallback.to_owned(),
        }
    }
}

/// Turn the sample ring into everything the chart card renders.
///
/// Ordering of the empty arms matters: they answer four *different*
/// questions, and collapsing them would leave an operator staring at a blank
/// chart with no idea whether the server is unreachable, freshly started, or
/// simply unwatched for the last hour. `Unreachable` is checked first because
/// "I cannot reach the server right now" outranks anything the retained
/// history could say — including when that history still draws a line.
fn build_home_chart(
    ring: &metrics_ring::MetricsRing,
    window: ChartWindow,
    series: SeriesFilter,
    metrics_ok: bool,
    i18n: &I18n,
) -> HomeChart {
    let samples = ring.snapshot();
    let now = now_secs();
    let window_secs = window.seconds();
    let start = now - window_secs;

    let points = metrics_ring::rates(&samples, |c| series.pick(c));
    let view = chart::build_chart(
        &points,
        now,
        window_secs,
        window.tick_count(),
        series.color(),
        |secs_before| window.axis_label(i18n, secs_before),
    );

    let empty = if !metrics_ok {
        Some(ChartEmpty::Unreachable)
    } else if samples.is_empty() {
        Some(ChartEmpty::NoSamples)
    } else if points.is_empty() {
        // One sample (or only unusable intervals): a rate needs two
        // comparable scrapes.
        Some(ChartEmpty::FirstInterval)
    } else if !view.has_data {
        Some(ChartEmpty::NoneInWindow)
    } else {
        None
    };

    let windows = ChartWindow::ALL
        .iter()
        .map(|w| WindowOption {
            label_key: w.label_key(),
            href: page_href(*w, series),
            active: *w == window,
        })
        .collect();

    let legend = SeriesFilter::ALL
        .iter()
        .map(|s| LegendItem {
            label_key: s.label_key(),
            href: page_href(window, *s),
            active: *s == series,
            color: s.color(),
            // Observed requests of this class inside the window — summed
            // from the per-interval deltas, so an unobserved stretch adds
            // nothing rather than dumping its whole backlog on the window.
            total: chart::compact_count(metrics_ring::window_total(&samples, start, now, |c| {
                s.pick(c)
            })),
        })
        .collect();

    HomeChart {
        view,
        windows,
        legend,
        empty,
    }
}

#[derive(Template)]
#[template(path = "pages/home.html")]
struct HomePage<'a> {
    chrome: Chrome<'a>,
    cards: HomeCards,
}

#[derive(Template)]
#[template(path = "partials/hts-home-cards.html")]
struct HomeCardsPartial<'a> {
    chrome: Chrome<'a>,
    cards: HomeCards,
}

pub(crate) fn routes() -> Router<Arc<HtsUiState>> {
    Router::new()
        .route("/hts", get(home_page))
        // Trailing-slash canonicalization: axum matches paths exactly, so
        // `/ui/hts/` would 404 without this. Redirect to the canonical form
        // used by every internal link in the UI. `Redirect::permanent` emits
        // 308 (preserves method + body); safe here since GET is the only
        // Home verb.
        .route("/hts/", get(|| async { Redirect::permanent("/ui/hts") }))
        .route("/hts/home/cards", get(home_cards_fragment))
}

async fn home_page(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(is_htmx): HxRequest,
    locale: RequestLocale,
    // The chart's window/legend links are plain `<a href>`s back to this
    // page, so the selection arrives here on a full navigation and works
    // with JavaScript disabled.
    Query(query): Query<HomeQuery>,
) -> Response {
    let (window, series) = query.selection();
    let i18n = I18n::new(locale);
    // On a hard navigation we always fetch once so first paint is
    // meaningful. On an htmx request that targeted `/hts` (unusual — the
    // refresh fragment endpoint below is the normal path) we still return
    // the full page: htmx will swap the requested element out of it.
    let cards = HomeCards::fetch(&state, window, series, &i18n).await;
    let chrome = Chrome {
        i18n,
        active_page: "home",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    let page = HomePage { chrome, cards };
    let _ = is_htmx; // reserved: fragment-only paths handled by the endpoint below.
    crate::render_page(page.render()).into_response()
}

async fn home_cards_fragment(
    State(state): State<Arc<HtsUiState>>,
    // Extracted only so `AutoVaryLayer` (axum-htmx) sees the handler
    // participate in htmx negotiation and appends `Vary: HX-Request`. The
    // fragment body is identical in both htmx and hard-navigation arms.
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
    // Carried by the region's own `hx-get`, which the swap replaces along
    // with the rest of the element — that is what makes a chart selection
    // survive the 15 s poll instead of being reverted by it.
    Query(query): Query<HomeQuery>,
) -> Response {
    let (window, series) = query.selection();
    let i18n = I18n::new(locale);
    let cards = HomeCards::fetch(&state, window, series, &i18n).await;
    let chrome = Chrome {
        i18n,
        active_page: "home",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    let partial = HomeCardsPartial { chrome, cards };
    match partial.render() {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!(?err, "hts-ui home cards fragment render failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "hts-ui: fragment render error",
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upstream::UpstreamClient;

    fn state_with(upstream: UpstreamClient, bundled: Option<u64>) -> Arc<HtsUiState> {
        Arc::new(HtsUiState {
            fhir_version: "R4",
            version: "0.0.0-test",
            upstream,
            bundled_data_bytes: bundled,
            // Per-state, so two `#[tokio::test]`s in this file never see
            // each other's samples.
            metrics_ring: Default::default(),
        })
    }

    /// Default selection, English catalog — what every test below wants
    /// unless it says otherwise.
    async fn default_cards(state: &Arc<HtsUiState>) -> HomeCards {
        let i18n = I18n::new(RequestLocale::default());
        HomeCards::fetch(
            state,
            ChartWindow::default(),
            SeriesFilter::default(),
            &i18n,
        )
        .await
    }

    /// The test client points at a URL that will refuse to connect (port 1
    /// on loopback is closed by convention). Every leg must fail and the
    /// Home page must still render.
    fn state_with_unreachable_upstream() -> Arc<HtsUiState> {
        let upstream = UpstreamClient::new("http://127.0.0.1:1").expect("client");
        state_with(upstream, None)
    }

    #[tokio::test]
    async fn cards_render_the_degraded_banner_when_upstream_is_unreachable() {
        let state = state_with_unreachable_upstream();
        let cards = default_cards(&state).await;
        assert!(cards.health.is_err());
        assert!(cards.capabilities.is_err());
        assert!(cards.degraded_reason().is_some());
    }

    #[tokio::test]
    async fn metrics_tiles_fall_back_to_none_when_metrics_fetch_fails() {
        // Fail-open contract: an unreachable `/metrics` must not raise a
        // banner or panic — it just leaves the two tiles empty so the
        // rest of the Home renders normally.
        let state = state_with_unreachable_upstream();
        let cards = default_cards(&state).await;
        assert!(cards.requests_total.is_none());
        assert!(cards.avg_latency_ms.is_none());
    }

    // ── Chart selection plumbing ──────────────────────────────────────────

    #[test]
    fn unknown_query_values_fall_back_to_the_defaults() {
        // A hand-edited or stale URL must render a chart, not a 400.
        assert_eq!(ChartWindow::parse(Some("nonsense")), ChartWindow::M15);
        assert_eq!(ChartWindow::parse(None), ChartWindow::M15);
        assert_eq!(SeriesFilter::parse(Some("3xx")), SeriesFilter::All);
        assert_eq!(SeriesFilter::parse(None), SeriesFilter::All);
    }

    #[test]
    fn window_and_series_codes_round_trip_through_parse() {
        for w in ChartWindow::ALL {
            assert_eq!(ChartWindow::parse(Some(w.code())), w);
        }
        for s in SeriesFilter::ALL {
            assert_eq!(SeriesFilter::parse(Some(s.code())), s);
        }
    }

    #[test]
    fn the_refresh_url_carries_the_current_selection() {
        // This is what makes a selection self-perpetuating across the 15 s
        // poll: the swapped element's own `hx-get` re-asks for the same view.
        assert_eq!(
            refresh_href(ChartWindow::H1, SeriesFilter::S4xx),
            "/ui/hts/home/cards?window=1h&series=4xx",
        );
        assert_eq!(
            page_href(ChartWindow::H6, SeriesFilter::S5xx),
            "/ui/hts?window=6h&series=5xx",
        );
    }

    #[test]
    fn every_legend_and_window_key_resolves_in_the_catalog() {
        // A missing Fluent key renders as the key itself; catch that here
        // rather than in a screenshot.
        let i18n = I18n::new(RequestLocale::default());
        let mut keys: Vec<&str> = vec![
            "hts-home-chart-title",
            "hts-home-chart-window",
            "hts-home-chart-series",
            "hts-home-chart-axis-now",
        ];
        keys.extend(ChartWindow::ALL.iter().map(|w| w.label_key()));
        keys.extend(SeriesFilter::ALL.iter().map(|s| s.label_key()));
        // The hint's two halves: the sentence forms naming the selected
        // window and status class. Distinct from `label_key()`, which is the
        // chip form ("1h" / "All").
        keys.extend(ChartWindow::ALL.iter().map(|w| w.hint_key()));
        keys.extend(SeriesFilter::ALL.iter().map(|s| s.hint_key()));
        keys.extend(
            [
                ChartEmpty::Unreachable,
                ChartEmpty::NoSamples,
                ChartEmpty::FirstInterval,
                ChartEmpty::NoneInWindow,
            ]
            .iter()
            .map(|e| e.key()),
        );
        for key in keys {
            assert_ne!(i18n.t(key), key, "Fluent key `{key}` is missing from en");
        }
        // The two placeable formats must substitute, not echo the key.
        assert_eq!(i18n.t_arg("hts-home-chart-axis-minutes", "n", "9"), "-9m");
        assert_eq!(i18n.t_arg("hts-home-chart-axis-hours", "n", "3"), "-3h");

        // The hint is composed, not fixed: it names whichever window and
        // status class are selected. A static caption would be wrong in two
        // of the three windows the picker offers, so assert the composition
        // actually substitutes rather than echoing keys.
        let hint = i18n.t_arg2_msg(
            "hts-home-chart-hint",
            "window",
            ChartWindow::M15.hint_key(),
            "classes",
            SeriesFilter::S4xx.hint_key(),
        );
        assert!(
            hint.contains("15 minutes") && hint.contains("4xx"),
            "the hint must name the selected window and class; got {hint:?}",
        );
        assert!(
            !hint.contains("hts-home-chart-hint"),
            "no key should leak into the rendered hint; got {hint:?}",
        );
        // The sampling caveat the V3 mockup drops: it is the only thing that
        // explains an otherwise inexplicably sparse chart.
        assert!(
            hint.contains("while this page is open"),
            "the sampling caveat must survive; got {hint:?}",
        );
    }

    // ── Empty-state arms (§7.1: never fabricate a line) ────────────────────

    fn ring_chart(ring: &metrics_ring::MetricsRing, metrics_ok: bool) -> HomeChart {
        let i18n = I18n::new(RequestLocale::default());
        build_home_chart(ring, ChartWindow::M15, SeriesFilter::All, metrics_ok, &i18n)
    }

    fn sample(at: f64, all: u64) -> MetricsSample {
        MetricsSample {
            at_secs: at,
            uptime_secs: at,
            counts: StatusCounts {
                all,
                s2xx: all,
                s4xx: 0,
                s5xx: 0,
            },
        }
    }

    #[test]
    fn zero_samples_says_so_instead_of_plotting_a_flat_zero() {
        let ring = metrics_ring::MetricsRing::new();
        let chart = ring_chart(&ring, true);
        assert_eq!(chart.empty, Some(ChartEmpty::NoSamples));
        assert!(!chart.view.has_data);
        assert!(chart.view.polylines.is_empty());
    }

    #[test]
    fn an_unreachable_metrics_endpoint_is_reported_as_such() {
        // Distinct from "no samples yet": the operator needs to know the
        // difference between a quiet server and an unreachable one.
        let ring = metrics_ring::MetricsRing::new();
        ring.push(sample(now_secs() - 30.0, 10));
        ring.push(sample(now_secs() - 15.0, 20));
        let chart = ring_chart(&ring, false);
        assert_eq!(chart.empty, Some(ChartEmpty::Unreachable));
    }

    #[test]
    fn one_sample_reports_the_first_interval_still_collecting() {
        let ring = metrics_ring::MetricsRing::new();
        ring.push(sample(now_secs() - 5.0, 10));
        let chart = ring_chart(&ring, true);
        assert_eq!(chart.empty, Some(ChartEmpty::FirstInterval));
        assert!(chart.view.polylines.is_empty(), "a rate needs two samples");
    }

    #[test]
    fn samples_outside_the_window_report_the_window_not_an_absence_of_data() {
        // The poll only ticks while the page is open, so "I have samples,
        // just none from the last 15 minutes" is a routine state and must
        // read differently from "I have nothing".
        let ring = metrics_ring::MetricsRing::new();
        let long_ago = now_secs() - 20_000.0;
        ring.push(sample(long_ago, 10));
        ring.push(sample(long_ago + 15.0, 25));
        let chart = ring_chart(&ring, true);
        assert_eq!(chart.empty, Some(ChartEmpty::NoneInWindow));
        assert!(!chart.view.has_data);
    }

    #[test]
    fn two_recent_samples_plot_a_line_and_clear_the_empty_state() {
        let ring = metrics_ring::MetricsRing::new();
        let now = now_secs();
        ring.push(sample(now - 30.0, 100));
        ring.push(sample(now - 15.0, 115));
        let chart = ring_chart(&ring, true);
        assert_eq!(chart.empty, None);
        assert!(chart.view.has_data);
        assert_eq!(chart.view.polylines.len(), 1);
        assert_eq!(chart.view.latest, "60", "15 requests over 15 s is 60/min");
    }

    #[test]
    fn the_legend_totals_are_per_status_class_and_window_scoped() {
        let ring = metrics_ring::MetricsRing::new();
        let now = now_secs();
        let mk = |at: f64, s2xx: u64, s4xx: u64| MetricsSample {
            at_secs: at,
            uptime_secs: at,
            counts: StatusCounts {
                all: s2xx + s4xx,
                s2xx,
                s4xx,
                s5xx: 0,
            },
        };
        ring.push(mk(now - 30.0, 0, 0));
        ring.push(mk(now - 15.0, 12, 3));
        let chart = ring_chart(&ring, true);
        let totals: Vec<(&str, &str)> = chart
            .legend
            .iter()
            .map(|l| (l.label_key, l.total.as_str()))
            .collect();
        assert_eq!(
            totals,
            vec![
                ("hts-home-chart-series-all", "15"),
                ("hts-home-chart-series-2xx", "12"),
                ("hts-home-chart-series-4xx", "3"),
                ("hts-home-chart-series-5xx", "0"),
            ],
        );
        assert!(
            chart.legend.iter().filter(|l| l.active).count() == 1,
            "exactly one legend entry is the active selection",
        );
    }

    #[tokio::test]
    async fn each_state_owns_its_own_sample_ring() {
        // The reason the ring is a state field and not a `static`: two
        // concurrent tests must not pollute each other's samples.
        let a = state_with_unreachable_upstream();
        let b = state_with_unreachable_upstream();
        a.metrics_ring.push(sample(now_secs(), 1));
        assert_eq!(a.metrics_ring.len(), 1);
        assert_eq!(b.metrics_ring.len(), 0);
        // And the unreachable-upstream fetch records nothing at all.
        let _ = default_cards(&b).await;
        assert_eq!(
            b.metrics_ring.len(),
            0,
            "a failed /metrics leg must not record a bogus sample",
        );
    }
}
