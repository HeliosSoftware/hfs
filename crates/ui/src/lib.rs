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

mod i18n;

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
use helios_observability::dashboard::{DashboardSeries, DashboardSnapshot};
use i18n::{I18n, RequestLocale};
use rust_embed::RustEmbed;
use std::time::{SystemTime, UNIX_EPOCH};

/// Static UI assets (htmx, CSS) embedded into the binary at compile time.
///
/// Pinned and vendored under `assets/`; never fetched at runtime.
#[derive(Clone, RustEmbed)]
#[folder = "assets/"]
struct Assets;

/// Shared router state: values that are constant for the process lifetime.
#[derive(Clone, Copy)]
struct WebState {
    version: &'static str,
}

/// A small, self-contained system-status snapshot — the "real read path" the
/// POC renders. Kept deliberately simple so the crate stays dependency-light;
/// richer read paths (terminology lookups, resource counts) plug in the same way.
struct Status {
    version: &'static str,
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

#[derive(Template)]
#[template(path = "pages/index.html")]
struct IndexPage {
    status: Status,
    metrics: DashboardMetrics,
    chart: ChartView,
    legend: Vec<LegendEntry>,
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
pub fn mount(fhir_app: Router, hfs_version: &'static str) -> Router {
    Router::new()
        .route("/ui", get(index))
        .route("/ui/status", get(status))
        // Embedded, pinned htmx + CSS, served with br/gzip/deflate negotiation.
        .nest_service("/ui/assets", ServeEmbed::<Assets>::new())
        // Emit `Vary: HX-Request` on handlers that read the header, so caches
        // don't cross a fragment response with a full-page one.
        .layer(AutoVaryLayer)
        // One negotiated locale per request, in request extensions; every
        // handler and template reads this same value.
        .layer(middleware::from_fn(i18n::negotiate_locale))
        .with_state(WebState {
            version: hfs_version,
        })
        .fallback_service(fhir_app)
}

/// Full landing page. `?type=<ResourceType>` selects which resource type's series
/// the chart plots (defaults to the first charted type); the value is validated
/// against the snapshot, so an unknown type harmlessly falls back to the default.
async fn index(
    State(state): State<WebState>,
    locale: RequestLocale,
    RawQuery(query): RawQuery,
) -> Response {
    let selected = selected_type_from_query(query.as_deref());
    render(build_index_page(state.version, locale, selected).await)
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
        render(build_index_page(state.version, locale, None).await)
    }
}

/// Assembles the landing page from the live dashboard snapshot, or from
/// placeholder data when no provider is registered.
async fn build_index_page(
    version: &'static str,
    locale: RequestLocale,
    selected: Option<String>,
) -> IndexPage {
    let status = current_status(version);
    let i18n = I18n::new(locale);
    let snapshot = helios_observability::dashboard::snapshot()
        .await
        .unwrap_or_else(sample_snapshot);
    let (metrics, chart, legend) = build_dashboard(&snapshot, selected.as_deref());
    IndexPage {
        status,
        metrics,
        chart,
        legend,
        i18n,
    }
}

/// Projects a [`DashboardSnapshot`] into the headline metrics, chart geometry,
/// and legend/selector the template renders.
fn build_dashboard(
    snapshot: &DashboardSnapshot,
    selected: Option<&str>,
) -> (DashboardMetrics, ChartView, Vec<LegendEntry>) {
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

    let chart = build_chart(&selected_type, selected_series);

    let legend = snapshot
        .series
        .iter()
        .map(|s| LegendEntry {
            resource_type: s.resource_type.clone(),
            total: grouped(s.total),
            href: format!("/ui?type={}", s.resource_type),
            active: s.resource_type == selected_type,
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

    (metrics, chart, legend)
}

// Chart plot area within the `0 0 1060 300` viewBox: the value axis occupies the
// left gutter (x < 40), the date axis the bottom (y > 278).
const PLOT_LEFT: i64 = 40;
const PLOT_RIGHT: i64 = 1060;
const PLOT_TOP: i64 = 10;
const PLOT_BOTTOM: i64 = 278;

/// Computes the SVG geometry for one resource type's cumulative series.
fn build_chart(selected_type: &str, series: Option<&DashboardSeries>) -> ChartView {
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
                label: short_date(&point.date),
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

/// Formats an ISO `YYYY-MM-DD` date as a compact axis label like `"JUL 7"`,
/// falling back to the raw string if it is not in the expected shape.
fn short_date(iso: &str) -> String {
    let mut parts = iso.split('-');
    let (_year, month, day) = match (parts.next(), parts.next(), parts.next()) {
        (Some(y), Some(m), Some(d)) => (y, m, d),
        _ => return iso.to_string(),
    };
    let month_abbr = match month {
        "01" => "JAN",
        "02" => "FEB",
        "03" => "MAR",
        "04" => "APR",
        "05" => "MAY",
        "06" => "JUN",
        "07" => "JUL",
        "08" => "AUG",
        "09" => "SEP",
        "10" => "OCT",
        "11" => "NOV",
        "12" => "DEC",
        _ => return iso.to_string(),
    };
    let day_trimmed = day.trim_start_matches('0');
    let day_trimmed = if day_trimmed.is_empty() {
        "0"
    } else {
        day_trimmed
    };
    format!("{month_abbr} {day_trimmed}")
}

/// Extracts a `type=<value>` selection from the raw query string, if present and
/// non-empty. Resource type names are alphanumeric, so no percent-decoding is
/// needed; the value is validated against the snapshot before use.
fn selected_type_from_query(query: Option<&str>) -> Option<String> {
    let query = query?;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        if key == "type" && !value.is_empty() {
            Some(value.to_string())
        } else {
            None
        }
    })
}

/// A representative snapshot used when no dashboard provider is registered, so
/// the design renders with plausible sample data (design frame: Patient growth
/// toward ~1.2k over 30 days).
fn sample_snapshot() -> DashboardSnapshot {
    use helios_observability::dashboard::DashboardPoint;

    fn series(resource_type: &str, per_day: u64, base: u64) -> DashboardSeries {
        let mut points = Vec::with_capacity(30);
        let mut cumulative = base;
        for day in 1..=30u32 {
            let count = per_day;
            cumulative += count;
            points.push(DashboardPoint {
                date: format!("2026-05-{day:02}"),
                count,
                cumulative,
            });
        }
        DashboardSeries {
            resource_type: resource_type.to_string(),
            total: cumulative,
            points,
        }
    }

    let series = vec![
        series("Patient", 32, 240),
        series("Observation", 1180, 3400),
        series("Encounter", 260, 1500),
        series("Condition", 90, 700),
    ];
    let total_resources = series.iter().map(|s| s.total).sum();

    DashboardSnapshot {
        fhir_version: "R4".to_string(),
        total_resources,
        distinct_types: 142,
        series,
    }
}

fn current_status(version: &'static str) -> Status {
    Status {
        version,
        checked_at: unix_timestamp_seconds(),
    }
}

fn render<T: Template>(template: T) -> Response {
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
        let (metrics, chart, legend) = build_dashboard(&sample_snapshot(), None);
        IndexPage {
            status: Status {
                version,
                checked_at,
            },
            metrics,
            chart,
            legend,
            i18n,
        }
    }

    #[test]
    fn index_page_renders_version_and_local_assets() {
        let html = sample_index_page("1.2.3", 42, i18n("en"))
            .render()
            .expect("index renders");

        assert!(html.contains("Helios FHIR Server"));
        assert!(html.contains("1.2.3"));
        assert!(html.contains(r#"hx-get="/ui/status""#));
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
        assert!(html.contains("Actualizar estado"));
        assert!(html.contains("Última comprobación: 42"));
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

        assert!(html.contains("1.2.3"));
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
        assert!(Assets::get("fonts/figtree-latin.woff2").is_some());
        assert!(Assets::get("fonts/figtree-latin-ext.woff2").is_some());
        assert!(Assets::get("logo.png").is_some());
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
        let (metrics, chart, legend) = build_dashboard(&sample_snapshot(), Some("Observation"));

        // Selected type drives the chart + headline total.
        assert_eq!(chart.selected_type, "Observation");
        assert!(chart.has_data);
        assert!(!chart.polyline.is_empty());
        assert_eq!(chart.y_ticks.len(), 5);

        // Legend lists every series, with the selected one marked active.
        assert_eq!(legend.len(), 4);
        let observation = legend
            .iter()
            .find(|e| e.resource_type == "Observation")
            .expect("Observation in legend");
        assert!(observation.active);
        assert_eq!(observation.href, "/ui?type=Observation");
        assert_eq!(legend.iter().filter(|e| e.active).count(), 1);

        assert_eq!(metrics.resource_types, "142");
    }

    #[test]
    fn unknown_selected_type_falls_back_to_first_series() {
        let (_metrics, chart, _legend) = build_dashboard(&sample_snapshot(), Some("<script>"));
        assert_eq!(chart.selected_type, "Patient");
    }

    #[test]
    fn empty_snapshot_renders_axes_without_a_series() {
        let empty = DashboardSnapshot {
            fhir_version: "R4".to_string(),
            total_resources: 0,
            distinct_types: 0,
            series: Vec::new(),
        };
        let (metrics, chart, legend) = build_dashboard(&empty, None);
        assert!(!chart.has_data);
        assert!(chart.polyline.is_empty());
        assert!(legend.is_empty());
        assert_eq!(metrics.chart_total, "0");
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
        assert_eq!(short_date("2026-07-07"), "JUL 7");
        assert_eq!(short_date("bogus"), "bogus");
    }

    #[test]
    fn selected_type_parsing() {
        assert_eq!(
            selected_type_from_query(Some("type=Observation")).as_deref(),
            Some("Observation")
        );
        assert_eq!(
            selected_type_from_query(Some("lang=es&type=Encounter")).as_deref(),
            Some("Encounter")
        );
        assert_eq!(selected_type_from_query(Some("type=")), None);
        assert_eq!(selected_type_from_query(Some("lang=es")), None);
        assert_eq!(selected_type_from_query(None), None);
    }

    #[test]
    fn single_point_series_anchors_one_coordinate_at_the_axis() {
        let series = DashboardSeries {
            resource_type: "Patient".to_string(),
            total: 5,
            points: vec![helios_observability::dashboard::DashboardPoint {
                date: "2026-05-01".to_string(),
                count: 5,
                cumulative: 5,
            }],
        };
        let chart = build_chart("Patient", Some(&series));

        assert!(chart.has_data);
        // A lone point produces a single "x,y" pair pinned to the left axis.
        assert!(!chart.polyline.contains(' '));
        assert!(chart.polyline.starts_with("40,"));
        assert_eq!(chart.x_ticks.len(), 1);
    }

    #[test]
    fn short_date_rejects_out_of_range_month() {
        assert_eq!(short_date("2026-13-09"), "2026-13-09");
        assert_eq!(short_date("2026-01-09"), "JAN 9");
    }
}
