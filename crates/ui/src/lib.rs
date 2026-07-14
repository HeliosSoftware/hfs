//! Server-rendered, HTMX-first web UI for the Helios FHIR Server.
//!
//! This crate is intentionally thin: handlers parse the request, gather data
//! from the rest of the workspace, and render an [`askama`] template. All markup
//! lives in `templates/`; static assets (htmx, CSS) are embedded at compile time
//! via [`rust_embed`] and served by [`axum_embed`] (with precompressed
//! negotiation), so there is no runtime CDN dependency.
//!
//! Handlers branch on the `HX-Request` header — read through the infallible
//! [`axum_htmx::HxRequest`] extractor — to return an HTML *fragment* for
//! htmx-driven swaps and a *full page* for hard navigations, so the UI degrades
//! to working full-page loads without JavaScript. [`axum_htmx::AutoVaryLayer`]
//! adds the matching `Vary` header so a fragment is never cached for a hard
//! navigation (or vice versa).
//!
//! The router is mounted under `/ui` by the `hfs` binary via [`mount`].
//!
//! All user-visible text is resolved from the Fluent catalogs in `locales/`
//! against the locale negotiated per request by [`i18n::negotiate_locale`]
//! (see `docs/multi-language.md`); templates hold catalog keys, not prose.
//!
//! ## Dashboard data
//!
//! The landing dashboard's "FHIR resources over time" chart and headline
//! resource counts come from the storage backend. To keep this crate free of any
//! persistence dependency, the server registers a provider with
//! [`helios_observability::dashboard`] at startup and this crate reads the latest
//! [`DashboardSnapshot`] through it. When no provider is registered (e.g. the
//! standalone example, or a build without persistence) the dashboard renders
//! placeholder figures instead. Counts reflect the server's **default tenant**
//! only — this is an operator view, and per-tenant counts are never exported to
//! the public Prometheus `/metrics` endpoint.
//!
//! The chart is sampled over a [`DashboardWindow`], selected per request with
//! `?window=` (`1h`, `24h`, or the default `30d`) alongside the `?type=` series
//! selector. Both selectors are plain links, so the dashboard stays navigable
//! without JavaScript.

mod i18n;
mod tenants;

use askama::Template;
use axum::{
    Router,
    extract::{RawQuery, State},
    http::StatusCode,
    middleware,
    response::{Html, IntoResponse, Response},
    routing::get,
};
use axum_embed::ServeEmbed;
use axum_htmx::{AutoVaryLayer, HxRequest};
use chrono::{DateTime, Datelike, Duration, Utc};
use helios_observability::dashboard::{
    DashboardPoint, DashboardSeries, DashboardSnapshot, DashboardWindow,
};
use helios_persistence::core::ResourceStorage;
use i18n::{I18n, RequestLocale};
use rust_embed::RustEmbed;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Static UI assets (htmx, CSS) embedded into the binary at compile time.
///
/// Pinned and vendored under `assets/`; never fetched at runtime.
#[derive(Clone, RustEmbed)]
#[folder = "assets/"]
struct Assets;

/// Shared router state: values that are constant for the process lifetime.
#[derive(Clone)]
struct WebState {
    version: &'static str,
    /// Read/write path for the tenant-maintenance page. `None` when the host did
    /// not wire storage in (e.g. the UI-only unit tests), in which case the page
    /// reports the registry as unavailable rather than crashing.
    tenants: Option<Arc<dyn ResourceStorage>>,
}

/// A small, self-contained system-status snapshot — the "real read path" the
/// POC renders. Kept deliberately simple so the crate stays dependency-light;
/// richer read paths (terminology lookups, resource counts) plug in the same way.
pub(crate) struct Status {
    pub(crate) version: &'static str,
    checked_at: u64,
}

/// Dashboard headline metrics rendered by `pages/index.html` (design: Figma
/// "Dashboard V1.1").
///
/// `resource_types`, `stored_resources`, `fhir_version`, and `chart_total` are
/// derived from the live [`DashboardSnapshot`] (default tenant). `export_jobs`,
/// `export_jobs_queued`, and `uptime_percent` describe other subsystems (bulk
/// export job state; availability history) that are not part of the
/// resource-count read path and remain placeholder values until those read paths
/// land.
struct DashboardMetrics {
    fhir_version: String,
    resource_types: String,
    stored_resources: String,
    export_jobs: String,
    export_jobs_queued: u32,
    uptime_percent: String,
    chart_total: String,
}

/// A single axis gridline or tick, in the chart's `0 0 1060 300` viewBox. `pos`
/// is the `y` coordinate for value ticks (horizontal gridlines) and the `x`
/// coordinate for date ticks; `label_y` is the text baseline (offset below a
/// value gridline; the fixed bottom row for date ticks).
struct AxisTick {
    label: String,
    pos: i64,
    label_y: i64,
}

/// Server-computed SVG geometry for the "resources over time" chart, for a single
/// selected resource type's cumulative series.
struct ChartView {
    /// Whether a non-empty series was plotted (`false` → axes only).
    has_data: bool,
    /// The resource type currently charted (a real, server-controlled type name).
    selected_type: String,
    /// `"x,y x,y …"` coordinate list for the `<polyline>`.
    polyline: String,
    /// Horizontal value gridlines, top (largest) to bottom (zero).
    y_ticks: Vec<AxisTick>,
    /// X-axis date labels at evenly spaced sample points.
    x_ticks: Vec<AxisTick>,
}

/// One entry in the chart legend, which doubles as the per-type series selector:
/// each is a link that re-renders the page with that type charted.
struct LegendEntry {
    resource_type: String,
    total: String,
    href: String,
    active: bool,
}

/// One entry in the time-window selector (`1h` / `24h` / `30d`): a link that
/// re-renders the page with the chart sampled over that window, keeping the
/// currently charted resource type.
struct WindowEntry {
    label: String,
    href: String,
    active: bool,
}

#[derive(Template)]
#[template(path = "pages/index.html")]
struct IndexPage {
    status: Status,
    metrics: DashboardMetrics,
    chart: ChartView,
    legend: Vec<LegendEntry>,
    windows: Vec<WindowEntry>,
    i18n: I18n,
}

#[derive(Template)]
#[template(path = "partials/status.html")]
struct StatusPartial {
    status: Status,
    i18n: I18n,
}

/// Mounts the web UI under `/ui`, falling back to the FHIR REST app for every
/// other path. The UI depends on the rest of the server, never the reverse.
///
/// `tenants` is the storage handle the tenant-maintenance page reads and writes
/// (the same backend the FHIR API uses); pass `None` to render the UI without a
/// live registry (the page then reports it as unavailable).
pub fn mount(
    fhir_app: Router,
    hfs_version: &'static str,
    tenants: Option<Arc<dyn ResourceStorage>>,
) -> Router {
    // Embedded, pinned htmx + CSS/JS + fonts, served with br/gzip/deflate
    // negotiation. `Cache-Control: no-cache` forces the browser to revalidate
    // against the (content-based) ETag on every load: unchanged assets come
    // back as a cheap `304`, but a rebuilt asset (e.g. app.css after a UI
    // change) is always re-fetched instead of served stale from cache.
    let assets = Router::new()
        .nest_service("/ui/assets", ServeEmbed::<Assets>::new())
        .layer(middleware::from_fn(revalidate_assets));

    Router::new()
        .route("/ui", get(index))
        .route("/ui/status", get(status))
        .route("/ui/tenants", get(tenants::page).post(tenants::create))
        .route("/ui/tenants/rows", get(tenants::rows))
        .route("/ui/tenants/{id}", axum::routing::delete(tenants::delete))
        .merge(assets)
        // Emit `Vary: HX-Request` on handlers that read the header, so caches
        // don't cross a fragment response with a full-page one.
        .layer(AutoVaryLayer)
        // One negotiated locale per request, in request extensions; every
        // handler and template reads this same value.
        .layer(middleware::from_fn(i18n::negotiate_locale))
        .with_state(WebState {
            version: hfs_version,
            tenants,
        })
        .fallback_service(fhir_app)
}

/// Adds `Cache-Control: no-cache` to embedded-asset responses so a rebuilt
/// asset is never served stale from the browser cache (revalidation is cheap:
/// unchanged content returns `304` via the ETag `ServeEmbed` already sets).
async fn revalidate_assets(request: axum::extract::Request, next: middleware::Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache"),
    );
    response
}

/// Full landing page. `?type=<ResourceType>` selects which resource type's series
/// the chart plots (defaults to the first charted type); the value is validated
/// against the snapshot, so an unknown type harmlessly falls back to the default.
/// `?window=<1h|24h|30d>` selects the time window the chart is sampled over,
/// falling back to [`DashboardWindow::default`] for anything unrecognised.
async fn index(
    State(state): State<WebState>,
    locale: RequestLocale,
    RawQuery(query): RawQuery,
) -> Response {
    let selected = query_value(query.as_deref(), "type");
    let window = query_value(query.as_deref(), "window")
        .and_then(|slug| DashboardWindow::from_slug(&slug))
        .unwrap_or_default();
    render(build_index_page(state.version, locale, selected, window).await)
}

/// Status read path. Returns a fragment to htmx (`HX-Request`) and a full page
/// on a hard navigation, so the same URL works with and without JavaScript.
async fn status(
    State(state): State<WebState>,
    locale: RequestLocale,
    HxRequest(is_htmx): HxRequest,
) -> Response {
    let status = current_status(state.version);
    let i18n = I18n::new(locale);
    if is_htmx {
        render(StatusPartial { status, i18n })
    } else {
        render(build_index_page(state.version, locale, None, DashboardWindow::default()).await)
    }
}

/// Assembles the landing page from the live dashboard snapshot, or from
/// placeholder data when no provider is registered.
async fn build_index_page(
    version: &'static str,
    locale: RequestLocale,
    selected: Option<String>,
    window: DashboardWindow,
) -> IndexPage {
    let status = current_status(version);
    let i18n = I18n::new(locale);
    let snapshot = helios_observability::dashboard::snapshot(window)
        .await
        .unwrap_or_else(|| sample_snapshot(window));
    let (metrics, chart, legend, windows) = build_dashboard(&snapshot, selected.as_deref());
    IndexPage {
        status,
        metrics,
        chart,
        legend,
        windows,
        i18n,
    }
}

/// Projects a [`DashboardSnapshot`] into the headline metrics, chart geometry,
/// and the two selectors (resource type, time window) the template renders.
fn build_dashboard(
    snapshot: &DashboardSnapshot,
    selected: Option<&str>,
) -> (
    DashboardMetrics,
    ChartView,
    Vec<LegendEntry>,
    Vec<WindowEntry>,
) {
    // Resolve the charted type: the requested one if it exists, else the first
    // series. `selected_type` is therefore always a real, server-controlled type
    // name (or empty when there are no series at all).
    let selected_type = selected
        .filter(|s| {
            snapshot
                .series
                .iter()
                .any(|series| series.resource_type.as_str() == *s)
        })
        .map(|s| s.to_string())
        .or_else(|| snapshot.series.first().map(|s| s.resource_type.clone()))
        .unwrap_or_default();

    let selected_series = snapshot
        .series
        .iter()
        .find(|s| s.resource_type == selected_type);

    let chart = build_chart(&selected_type, selected_series, snapshot.window);

    // Both selectors carry the other's current value, so switching type keeps the
    // window and vice versa.
    let legend = snapshot
        .series
        .iter()
        .map(|s| LegendEntry {
            resource_type: s.resource_type.clone(),
            total: grouped(s.total),
            href: format!(
                "/ui?type={}&window={}",
                s.resource_type,
                snapshot.window.as_str()
            ),
            active: s.resource_type == selected_type,
        })
        .collect();

    let windows = DashboardWindow::ALL
        .into_iter()
        .map(|w| WindowEntry {
            label: w.as_str().to_string(),
            href: format!("/ui?type={}&window={}", selected_type, w.as_str()),
            active: w == snapshot.window,
        })
        .collect();

    let metrics = DashboardMetrics {
        fhir_version: snapshot.fhir_version.clone(),
        resource_types: snapshot.distinct_types.to_string(),
        stored_resources: compact_count(snapshot.total_resources),
        // Not part of the resource-count read path — placeholder until the bulk
        // export job-state and availability read paths are wired.
        export_jobs: "13".to_string(),
        export_jobs_queued: 1,
        uptime_percent: "99.98".to_string(),
        chart_total: selected_series
            .map(|s| grouped(s.total))
            .unwrap_or_else(|| "0".to_string()),
    };

    (metrics, chart, legend, windows)
}

// Chart plot area within the `0 0 1060 300` viewBox: the value axis occupies the
// left gutter (x < 40), the date axis the bottom (y > 278).
const PLOT_LEFT: i64 = 40;
const PLOT_RIGHT: i64 = 1060;
const PLOT_TOP: i64 = 10;
const PLOT_BOTTOM: i64 = 278;

/// Computes the SVG geometry for one resource type's cumulative series. `window`
/// decides only the x-axis label format — a calendar date over daily buckets, a
/// UTC clock time over intraday ones.
fn build_chart(
    selected_type: &str,
    series: Option<&DashboardSeries>,
    window: DashboardWindow,
) -> ChartView {
    let points = match series {
        Some(s) if !s.points.is_empty() => &s.points,
        _ => {
            // No data: render an empty 0-based axis so the card still frames.
            return ChartView {
                has_data: false,
                selected_type: selected_type.to_string(),
                polyline: String::new(),
                y_ticks: y_axis_ticks(0),
                x_ticks: Vec::new(),
            };
        }
    };

    let width = PLOT_RIGHT - PLOT_LEFT;
    let height = PLOT_BOTTOM - PLOT_TOP;
    let n = points.len() as i64;

    let peak = points.iter().map(|p| p.cumulative).max().unwrap_or(0);
    let axis_max = nice_ceil(peak).max(1);

    // Map sample index -> x, cumulative value -> y (SVG y grows downward).
    let x_at = |i: i64| -> i64 {
        if n <= 1 {
            PLOT_LEFT
        } else {
            PLOT_LEFT + width * i / (n - 1)
        }
    };
    let y_at = |value: u64| -> i64 { PLOT_BOTTOM - (height * value as i64) / axis_max as i64 };

    let polyline = points
        .iter()
        .enumerate()
        .map(|(i, p)| format!("{},{}", x_at(i as i64), y_at(p.cumulative)))
        .collect::<Vec<_>>()
        .join(" ");

    // Up to six evenly spaced date labels along the window.
    let label_count = (n as usize).min(6);
    let mut x_ticks = Vec::with_capacity(label_count);
    for j in 0..label_count as i64 {
        let idx = if label_count <= 1 {
            0
        } else {
            (n - 1) * j / (label_count as i64 - 1)
        };
        if let Some(point) = points.get(idx as usize) {
            x_ticks.push(AxisTick {
                label: axis_time_label(point.bucket_start, window),
                pos: x_at(idx),
                // Date labels sit on the fixed bottom row of the viewBox.
                label_y: 298,
            });
        }
    }

    ChartView {
        has_data: true,
        selected_type: selected_type.to_string(),
        polyline,
        y_ticks: y_axis_ticks(axis_max),
        x_ticks,
    }
}

/// Five horizontal value gridlines from `axis_max` (top) down to `0` (bottom).
fn y_axis_ticks(axis_max: u64) -> Vec<AxisTick> {
    let height = PLOT_BOTTOM - PLOT_TOP;
    (0..=4i64)
        .map(|k| {
            let value = axis_max * (4 - k) as u64 / 4;
            let pos = PLOT_TOP + height * k / 4;
            AxisTick {
                label: compact_count(value),
                pos,
                // Nudge the label baseline down so it centres on the gridline.
                label_y: pos + 3,
            }
        })
        .collect()
}

/// Rounds up to one significant figure for tidy axis maxima (1204 -> 2000,
/// 38 910 -> 40 000). Returns 0 for 0.
fn nice_ceil(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut magnitude = 1u64;
    while magnitude.saturating_mul(10) <= n {
        magnitude *= 10;
    }
    n.div_ceil(magnitude) * magnitude
}

/// Compact count for axis labels and the stat card: `61 400 -> "61.4k"`,
/// `2 000 -> "2.0k"`, `1 500 000 -> "1.5M"`, small values verbatim.
fn compact_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Thousands-separated integer for prominent totals: `1204 -> "1,204"`.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// A compact x-axis label for a bucket: a UTC clock time (`"14:30"`) when the
/// window's buckets are finer than a day, otherwise a calendar date (`"JUL 7"`).
/// Both are UTC, matching the buckets themselves.
fn axis_time_label(bucket_start: DateTime<Utc>, window: DashboardWindow) -> String {
    if window.is_intraday() {
        return bucket_start.format("%H:%M").to_string();
    }
    const MONTHS: [&str; 12] = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    let month = MONTHS[(bucket_start.month0() as usize).min(11)];
    format!("{month} {}", bucket_start.day())
}

/// Extracts `key=<value>` from the raw query string, if present and non-empty.
/// Both values we read this way (a FHIR resource type, a window slug) are
/// alphanumeric, so no percent-decoding is needed; each is validated — against
/// the snapshot's series, or `DashboardWindow::from_slug` — before use.
fn query_value(query: Option<&str>, key: &str) -> Option<String> {
    let query = query?;
    query.split('&').find_map(|pair| {
        let (k, value) = pair.split_once('=')?;
        if k == key && !value.is_empty() {
            Some(value.to_string())
        } else {
            None
        }
    })
}

/// A representative snapshot used when no dashboard provider is registered, so
/// the design renders with plausible sample data (design frame: Patient growth
/// toward ~1.2k over 30 days).
fn sample_snapshot(window: DashboardWindow) -> DashboardSnapshot {
    // Dense buckets for the requested window, ending now — the same shape a real
    // provider returns, so the placeholder exercises the same rendering path. The
    // per-bucket growth is scaled down for the shorter windows so the sample curve
    // stays plausible at every zoom.
    let bucket = Duration::seconds(window.bucket_seconds());
    let scale = window.span_seconds() as f64 / DashboardWindow::LastMonth.span_seconds() as f64;
    let last_bucket = bucket_floor_utc(Utc::now(), window.bucket_seconds());
    let first_bucket = last_bucket - bucket * (window.points().saturating_sub(1) as i32);

    let series = |resource_type: &str, per_day: u64, base: u64| -> DashboardSeries {
        let per_bucket = ((per_day as f64 * scale) / window.points() as f64).round() as i64;
        let mut points = Vec::with_capacity(window.points());
        let mut cumulative = base;
        for i in 0..window.points() {
            cumulative += per_bucket.max(0) as u64;
            points.push(DashboardPoint {
                bucket_start: first_bucket + bucket * (i as i32),
                delta: per_bucket,
                cumulative,
            });
        }
        DashboardSeries {
            resource_type: resource_type.to_string(),
            total: cumulative,
            points,
        }
    };

    let series = vec![
        series("Patient", 960, 240),
        series("Observation", 35_400, 3_400),
        series("Encounter", 7_800, 1_500),
        series("Condition", 2_700, 700),
    ];
    let total_resources = series.iter().map(|s| s.total).sum();

    DashboardSnapshot {
        fhir_version: "R4".to_string(),
        total_resources,
        distinct_types: 142,
        window,
        series,
    }
}

/// Floors `ts` to the start of the epoch-aligned bucket containing it. Mirrors
/// `helios_persistence::core::bucket_floor`, which this crate cannot call — it
/// deliberately does not depend on persistence — and is used only to shape the
/// placeholder series.
fn bucket_floor_utc(ts: DateTime<Utc>, bucket_seconds: i64) -> DateTime<Utc> {
    if bucket_seconds <= 0 {
        return ts;
    }
    let floored = ts.timestamp().div_euclid(bucket_seconds) * bucket_seconds;
    DateTime::from_timestamp(floored, 0).unwrap_or(ts)
}

pub(crate) fn current_status(version: &'static str) -> Status {
    Status {
        version,
        checked_at: unix_timestamp_seconds(),
    }
}

pub(crate) fn render<T: Template>(template: T) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("template render error: {error}"),
        )
            .into_response(),
    }
}

fn unix_timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i18n(tag: &str) -> I18n {
        I18n::from_tag(tag).expect("supported locale")
    }

    /// Builds an `IndexPage` from the sample snapshot for template-rendering tests.
    fn sample_index_page(version: &'static str, checked_at: u64, i18n: I18n) -> IndexPage {
        let (metrics, chart, legend, windows) =
            build_dashboard(&sample_snapshot(DashboardWindow::default()), None);
        IndexPage {
            status: Status {
                version,
                checked_at,
            },
            metrics,
            chart,
            legend,
            windows,
            i18n,
        }
    }

    /// A point at a fixed instant, for geometry tests that don't care when.
    fn point_at(epoch_secs: i64, delta: i64, cumulative: u64) -> DashboardPoint {
        DashboardPoint {
            bucket_start: DateTime::from_timestamp(epoch_secs, 0).expect("valid instant"),
            delta,
            cumulative,
        }
    }

    #[test]
    fn index_page_renders_version_and_local_assets() {
        let html = sample_index_page("1.2.3", 42, i18n("en"))
            .render()
            .expect("index renders");

        assert!(html.contains("Helios FHIR Server"));
        assert!(html.contains("1.2.3"));
        assert!(html.contains("/ui/assets/htmx.min.js"));
        // No runtime CDN dependency.
        assert!(!html.contains("unpkg.com"));
    }

    #[test]
    fn index_page_renders_in_the_negotiated_locale() {
        let html = sample_index_page("1.2.3", 42, i18n("es"))
            .render()
            .expect("index renders");

        assert!(html.contains(r#"<html lang="es">"#));
        assert!(html.contains("Inicio"));
        // The language switcher marks the active locale.
        assert!(html.contains(r#"href="?lang=es" aria-current="true""#));
    }

    #[test]
    fn status_partial_is_fragment_not_full_page() {
        let html = StatusPartial {
            status: Status {
                version: "1.2.3",
                checked_at: 42,
            },
            i18n: i18n("en"),
        }
        .render()
        .expect("status renders");

        assert!(html.contains("Last checked: 42"));
        assert!(!html.contains("<html"));
        assert!(!html.contains("<!doctype"));
    }

    #[test]
    fn htmx_asset_is_embedded() {
        let file = Assets::get("htmx.min.js").expect("htmx asset embedded");
        assert!(!file.data.is_empty());
    }

    #[test]
    fn css_asset_is_embedded() {
        assert!(Assets::get("app.css").is_some());
    }

    /// The dashboard shell's own assets: theme switcher, vendored Figtree,
    /// and the brand logo exported from the design file.
    #[test]
    fn design_assets_are_embedded() {
        assert!(Assets::get("theme.js").is_some());
        assert!(Assets::get("nav.js").is_some());
        assert!(Assets::get("fonts/figtree-latin.woff2").is_some());
        assert!(Assets::get("fonts/figtree-latin-ext.woff2").is_some());
        assert!(Assets::get("logo.png").is_some());
    }

    /// The collapsible-nav script persists the state to the per-user settings
    /// document (like the theme, #197): read on load, merge-patch `nav` on
    /// toggle, with a localStorage first-paint cache. Guards the wiring.
    #[test]
    fn nav_script_is_wired_to_user_settings() {
        let file = Assets::get("nav.js").expect("nav.js embedded");
        let source = std::str::from_utf8(&file.data).expect("nav.js is UTF-8");
        assert!(source.contains("/_user/settings"));
        assert!(source.contains("PATCH"));
        assert!(source.contains("hfs-nav"), "localStorage cache stays");
        assert!(source.contains("data-nav"), "sets the collapse attribute");
    }

    /// The theme script persists the choice to the per-user settings document
    /// (#197): it must read the document on load and merge-patch `theme` on
    /// toggle, with localStorage kept as the first-paint cache. Guards the
    /// wiring; the endpoint round-trip itself is covered in helios-rest's
    /// `user_settings` tests.
    #[test]
    fn theme_script_is_wired_to_user_settings() {
        let file = Assets::get("theme.js").expect("theme.js embedded");
        let source = std::str::from_utf8(&file.data).expect("theme.js is UTF-8");
        assert!(source.contains("/_user/settings"));
        assert!(source.contains("PATCH"));
        assert!(source.contains("hfs-theme"), "localStorage cache stays");
    }

    /// Both theme buttons render, and icons are inlined (so `currentColor`
    /// theming applies) rather than referenced as external images.
    #[test]
    fn index_page_renders_theme_toggle_and_inline_icons() {
        let html = sample_index_page("1.2.3", 42, i18n("en"))
            .render()
            .expect("index renders");

        assert!(html.contains(r#"data-set-theme="light""#));
        assert!(html.contains(r#"data-set-theme="dark""#));
        assert!(html.contains("<svg"));
        assert!(html.contains(r#"fill="currentColor""#));
    }

    #[test]
    fn dashboard_projects_snapshot_counts_and_chart() {
        let (metrics, chart, legend, _windows) = build_dashboard(
            &sample_snapshot(DashboardWindow::default()),
            Some("Observation"),
        );

        // Selected type drives the chart + headline total.
        assert_eq!(chart.selected_type, "Observation");
        assert!(chart.has_data);
        assert!(!chart.polyline.is_empty());
        assert_eq!(chart.y_ticks.len(), 5);

        // Legend lists every series, with the selected one marked active. Each
        // href carries the current window, so switching type does not reset it.
        assert_eq!(legend.len(), 4);
        let observation = legend
            .iter()
            .find(|e| e.resource_type == "Observation")
            .expect("Observation in legend");
        assert!(observation.active);
        assert_eq!(observation.href, "/ui?type=Observation&window=30d");
        assert_eq!(legend.iter().filter(|e| e.active).count(), 1);

        assert_eq!(metrics.resource_types, "142");
    }

    /// The window selector offers every window, marks the snapshot's own as
    /// active, and carries the charted type across a window switch.
    #[test]
    fn window_selector_marks_the_active_window_and_keeps_the_charted_type() {
        let (_metrics, _chart, _legend, windows) = build_dashboard(
            &sample_snapshot(DashboardWindow::LastHour),
            Some("Encounter"),
        );

        assert_eq!(windows.len(), DashboardWindow::ALL.len());
        assert_eq!(windows.iter().filter(|w| w.active).count(), 1);
        let active = windows.iter().find(|w| w.active).expect("an active window");
        assert_eq!(active.label, "1h");
        assert!(
            windows
                .iter()
                .all(|w| w.href.starts_with("/ui?type=Encounter&window=")),
            "every window link keeps the charted type"
        );
        assert_eq!(active.href, "/ui?type=Encounter&window=1h");
    }

    /// Intraday windows label the x-axis with clock times; the 30-day window
    /// keeps calendar dates. Same series, different axis vocabulary.
    #[test]
    fn axis_labels_follow_the_window_resolution() {
        let (_m, hour_chart, _l, _w) =
            build_dashboard(&sample_snapshot(DashboardWindow::LastHour), None);
        assert!(
            hour_chart
                .x_ticks
                .iter()
                .all(|t| t.label.contains(':') && t.label.len() == 5),
            "1h axis should read as HH:MM, got {:?}",
            hour_chart
                .x_ticks
                .iter()
                .map(|t| &t.label)
                .collect::<Vec<_>>()
        );

        let (_m, month_chart, _l, _w) =
            build_dashboard(&sample_snapshot(DashboardWindow::LastMonth), None);
        assert!(
            month_chart.x_ticks.iter().all(|t| !t.label.contains(':')),
            "30d axis should read as a calendar date, got {:?}",
            month_chart
                .x_ticks
                .iter()
                .map(|t| &t.label)
                .collect::<Vec<_>>()
        );
    }

    /// Every window produces a chart the SVG can actually draw: a dense series of
    /// the window's own length, and at most six x-axis labels however many
    /// buckets it holds.
    #[test]
    fn every_window_renders_a_bounded_chart() {
        for window in DashboardWindow::ALL {
            let snapshot = sample_snapshot(window);
            let (_m, chart, _l, _w) = build_dashboard(&snapshot, None);
            assert!(chart.has_data, "{}", window.as_str());
            assert_eq!(snapshot.series[0].points.len(), window.points());
            assert!(
                chart.x_ticks.len() <= 6,
                "{} produced {} x labels",
                window.as_str(),
                chart.x_ticks.len()
            );
        }
    }

    #[test]
    fn unknown_selected_type_falls_back_to_first_series() {
        let (_metrics, chart, _legend, _windows) = build_dashboard(
            &sample_snapshot(DashboardWindow::default()),
            Some("<script>"),
        );
        assert_eq!(chart.selected_type, "Patient");
    }

    #[test]
    fn empty_snapshot_renders_axes_without_a_series() {
        let empty = DashboardSnapshot {
            fhir_version: "R4".to_string(),
            total_resources: 0,
            distinct_types: 0,
            window: DashboardWindow::default(),
            series: Vec::new(),
        };
        let (metrics, chart, legend, windows) = build_dashboard(&empty, None);
        assert!(!chart.has_data);
        assert!(chart.polyline.is_empty());
        assert!(legend.is_empty());
        assert_eq!(metrics.chart_total, "0");
        // The window selector still renders, so an empty server is not a dead end.
        assert_eq!(windows.len(), DashboardWindow::ALL.len());
    }

    #[test]
    fn formatting_helpers() {
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(2_000), "2.0k");
        assert_eq!(compact_count(61_400), "61.4k");
        assert_eq!(compact_count(1_500_000), "1.5M");
        assert_eq!(grouped(1_204), "1,204");
        assert_eq!(grouped(38_910), "38,910");
        assert_eq!(grouped(7), "7");
        assert_eq!(nice_ceil(1_204), 2_000);
        assert_eq!(nice_ceil(38_910), 40_000);
        assert_eq!(nice_ceil(0), 0);
    }

    /// `2026-07-07T14:30:00Z`, formatted for each window's axis.
    #[test]
    fn axis_time_label_formats_per_window() {
        let at = DateTime::from_timestamp(1_752_503_400, 0).expect("valid instant");
        assert_eq!(axis_time_label(at, DashboardWindow::LastHour), "14:30");
        assert_eq!(axis_time_label(at, DashboardWindow::LastDay), "14:30");
        assert_eq!(axis_time_label(at, DashboardWindow::LastMonth), "JUL 14");
    }

    #[test]
    fn query_value_parsing() {
        assert_eq!(
            query_value(Some("type=Observation"), "type").as_deref(),
            Some("Observation")
        );
        assert_eq!(
            query_value(Some("lang=es&type=Encounter"), "type").as_deref(),
            Some("Encounter")
        );
        assert_eq!(query_value(Some("type="), "type"), None);
        assert_eq!(query_value(Some("lang=es"), "type"), None);
        assert_eq!(query_value(None, "type"), None);

        // The window slug is read the same way, and validated by `from_slug`.
        assert_eq!(
            query_value(Some("type=Patient&window=24h"), "window").as_deref(),
            Some("24h")
        );
        assert_eq!(
            query_value(Some("window=24h"), "window")
                .as_deref()
                .and_then(DashboardWindow::from_slug),
            Some(DashboardWindow::LastDay)
        );
        // An unrecognised window is dropped, and the caller falls back to default.
        assert_eq!(
            query_value(Some("window=7d"), "window")
                .as_deref()
                .and_then(DashboardWindow::from_slug),
            None
        );
    }

    #[test]
    fn single_point_series_anchors_one_coordinate_at_the_axis() {
        let series = DashboardSeries {
            resource_type: "Patient".to_string(),
            total: 5,
            points: vec![point_at(1_752_503_400, 5, 5)],
        };
        let chart = build_chart("Patient", Some(&series), DashboardWindow::default());

        assert!(chart.has_data);
        // A lone point produces a single "x,y" pair pinned to the left axis.
        assert!(!chart.polyline.contains(' '));
        assert!(chart.polyline.starts_with("40,"));
        assert_eq!(chart.x_ticks.len(), 1);
    }

    /// A bucket where deletions outweigh creations dips the curve. The y-axis is
    /// scaled from the cumulative peak, so a dip must not push a point off-canvas.
    #[test]
    fn a_net_negative_bucket_dips_the_curve_without_escaping_the_plot() {
        let series = DashboardSeries {
            resource_type: "Patient".to_string(),
            total: 4,
            points: vec![
                point_at(1_752_503_400, 10, 10),
                point_at(1_752_503_460, -6, 4),
                point_at(1_752_503_520, 0, 4),
            ],
        };
        let chart = build_chart("Patient", Some(&series), DashboardWindow::LastHour);

        assert!(chart.has_data);
        // Every plotted y sits inside the plot area (10..=278 in the viewBox).
        for pair in chart.polyline.split(' ') {
            let y: i64 = pair
                .split_once(',')
                .expect("x,y pair")
                .1
                .parse()
                .expect("integer y");
            assert!(
                (PLOT_TOP..=PLOT_BOTTOM).contains(&y),
                "y {y} escaped the plot"
            );
        }
    }
}
