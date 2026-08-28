//! SVG chart geometry for the Home request-rate card (§7.1).
//!
//! # A deliberate copy, not a shared crate
//!
//! `ChartView`, `AxisTick`, `build_chart`, [`y_axis_ticks`], [`nice_ceil`],
//! [`compact_count`] and the `PLOT_*` constants below are copied from
//! `crates/ui/src/lib.rs` (HFS's dashboard chart) so the two products render
//! visually identical cards against the *same* `.chart` / `.grid-line` /
//! `.axis-label` rules in the shared `app.css`.
//!
//! Extracting a `helios-ui-chrome` crate is deliberately deferred — see
//! `edson/docs/hts-ui-design.md` §9.0 — so the duplication is guarded by a
//! test instead: `tests/chrome_parity.rs::chart_geometry_matches_hfs` reads
//! HFS's `lib.rs` and `pages/index.html` off disk, extracts their plot
//! constants and viewBox, and asserts they still equal this module's. Drift
//! becomes a test failure rather than a silently mismatched pair of charts.
//!
//! # What differs from HFS, and why
//!
//! HFS charts dense, equal-width buckets from an in-process dashboard
//! provider, so it can map x by *sample index* and emit one `<polyline>`
//! per series. HTS has neither luxury: samples only accumulate while an
//! operator has the Home page open (see [`crate::metrics_ring`]). So:
//!
//! - x is mapped by **timestamp** against a window anchored at "now", not by
//!   index. Index mapping would stretch three samples taken a minute apart
//!   across a six-hour axis.
//! - a series is emitted as **many polylines**, one per observed run, so an
//!   unobserved stretch is a visible break rather than a straight line
//!   asserting continuity nobody measured.

/// One axis gridline or tick, in the chart's `0 0 1060 300` viewBox. `pos` is
/// the `y` coordinate for value ticks (horizontal gridlines) and the `x`
/// coordinate for time ticks; `label_y` is the text baseline (offset below a
/// value gridline; the fixed bottom row for time ticks).
#[derive(Clone, Debug)]
pub(crate) struct AxisTick {
    pub label: String,
    pub pos: i64,
    pub label_y: i64,
}

// Chart plot area within the `0 0 1060 300` viewBox: the value axis occupies
// the left gutter (x < 40), the time axis the bottom 22 units. Copied from
// `crates/ui/src/lib.rs`; `chart_geometry_matches_hfs` pins the equality.
pub(crate) const PLOT_LEFT: i64 = 40;
pub(crate) const PLOT_RIGHT: i64 = 1060;
pub(crate) const PLOT_TOP: i64 = 10;
/// HFS derives this as `CHART_HEIGHT - 22`; spelled out here as a constant so
/// the parity test can compare a literal to a literal.
pub(crate) const PLOT_BOTTOM: i64 = 278;
/// The chart's fixed viewBox height. Rendered into the `viewBox` attribute of
/// `templates/partials/hts-home-chart.html` via [`ChartView::height`], so the
/// constant is the single source of truth for both the geometry and the
/// markup.
pub(crate) const CHART_HEIGHT: i64 = 300;

/// Server-computed SVG geometry for the Home request-rate chart.
#[derive(Clone, Debug)]
pub(crate) struct ChartView {
    /// Whether any point fell inside the window. `false` renders the axis
    /// frame with no line and an explanatory `.stat__sub` — never a
    /// fabricated flat line at zero, which would claim we observed silence.
    pub has_data: bool,
    /// One `"x,y x,y …"` coordinate list per continuously observed run.
    /// Each becomes its own `<polyline class="series">`; the breaks between
    /// them are the stretches nobody was watching.
    pub polylines: Vec<String>,
    /// Horizontal value gridlines, top (largest) to bottom (zero).
    pub y_ticks: Vec<AxisTick>,
    /// Time labels along the bottom, oldest (left) to "now" (right).
    pub x_ticks: Vec<AxisTick>,
    /// viewBox height (always [`CHART_HEIGHT`]). Rendered into the `viewBox`
    /// attribute so the constant and the markup cannot drift, exactly as
    /// HFS's `pages/index.html` does.
    pub height: i64,
    /// 1-based palette slot (`--series-N`), so the line matches its legend dot.
    pub color: usize,
    /// The most recent in-window rate, compact — the card's headline number.
    /// Empty when `has_data` is false.
    pub latest: String,
}

/// Floor for the value axis when traffic is low or absent.
///
/// HFS uses `.max(1)`, which on an idle chart produces integer-divided tick
/// labels reading `1, 0, 0, 0, 0`. Four is the smallest maximum that divides
/// cleanly into the five gridlines, so an idle HTS reads `4, 3, 2, 1, 0`. It
/// scales the empty frame; it never invents a data point.
const MIN_AXIS_MAX: u64 = 4;

/// Compute the SVG geometry for one request-rate series.
///
/// `now_secs` anchors the right edge of the plot and `window_secs` its width,
/// so the axis always means "the last N minutes ending now" whether or not
/// anybody was sampling for all of it. Points outside that span are dropped
/// rather than squeezed in.
///
/// `x_label` receives *seconds before now* for each tick and returns the
/// localized label, keeping this module free of i18n.
pub(crate) fn build_chart(
    points: &[crate::metrics_ring::RatePoint],
    now_secs: f64,
    window_secs: f64,
    tick_count: usize,
    color: usize,
    x_label: impl Fn(f64) -> String,
) -> ChartView {
    let height = CHART_HEIGHT;
    let plot_bottom = PLOT_BOTTOM;
    let width = PLOT_RIGHT - PLOT_LEFT;
    let plot_height = plot_bottom - PLOT_TOP;
    let start = now_secs - window_secs;

    let in_window: Vec<&crate::metrics_ring::RatePoint> = points
        .iter()
        .filter(|p| p.at_secs >= start && p.at_secs <= now_secs)
        .collect();

    let peak = in_window.iter().map(|p| p.per_min).max().unwrap_or(0);
    let axis_max = nice_ceil(peak).max(MIN_AXIS_MAX);

    // Map timestamp -> x and rate -> y (SVG y grows downward). The clamp
    // guards float drift at the window edges only; genuinely out-of-window
    // points were already filtered out above.
    let x_at = |t: f64| -> i64 {
        let frac = if window_secs > 0.0 {
            ((t - start) / window_secs).clamp(0.0, 1.0)
        } else {
            1.0
        };
        PLOT_LEFT + (width as f64 * frac).round() as i64
    };
    let y_at = |value: u64| -> i64 { plot_bottom - (plot_height * value as i64) / axis_max as i64 };

    // Split into continuously observed runs. The first in-window point always
    // opens a segment even when its `break_before` is false — its predecessor
    // fell outside the window, so there is nothing to connect it to.
    let mut segments: Vec<Vec<String>> = Vec::new();
    for point in &in_window {
        if segments.is_empty() || point.break_before {
            segments.push(Vec::new());
        }
        if let Some(current) = segments.last_mut() {
            current.push(format!("{},{}", x_at(point.at_secs), y_at(point.per_min)));
        }
    }
    let polylines: Vec<String> = segments.iter().map(|seg| seg.join(" ")).collect();

    // Evenly spaced time labels across the window, oldest first. Tick count
    // is per-window so every label lands on a whole minute or hour.
    let ticks = tick_count.max(2);
    let x_ticks = (0..ticks)
        .map(|j| {
            let frac = j as f64 / (ticks - 1) as f64;
            let at = start + window_secs * frac;
            AxisTick {
                label: x_label(now_secs - at),
                pos: x_at(at),
                // Time labels sit on the fixed bottom row of the viewBox.
                label_y: height - 2,
            }
        })
        .collect();

    ChartView {
        has_data: !in_window.is_empty(),
        polylines,
        y_ticks: y_axis_ticks(axis_max, height, plot_bottom),
        x_ticks,
        height,
        color,
        latest: in_window
            .last()
            .map(|p| compact_count(p.per_min))
            .unwrap_or_default(),
    }
}

/// Five horizontal value gridlines from `axis_max` (top) down to `0` (bottom).
/// Copied verbatim from `crates/ui/src/lib.rs`.
fn y_axis_ticks(axis_max: u64, _height: i64, plot_bottom: i64) -> Vec<AxisTick> {
    let plot_height = plot_bottom - PLOT_TOP;
    (0..=4i64)
        .map(|k| {
            let value = axis_max * (4 - k) as u64 / 4;
            let pos = PLOT_TOP + plot_height * k / 4;
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
/// 38 910 -> 40 000). Returns 0 for 0. Copied verbatim from
/// `crates/ui/src/lib.rs`.
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
/// `2 000 -> "2.0k"`, `1 500 000 -> "1.5M"`, small values verbatim. Copied
/// verbatim from `crates/ui/src/lib.rs`.
pub(crate) fn compact_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics_ring::RatePoint;

    fn point(at: f64, per_min: u64, break_before: bool) -> RatePoint {
        RatePoint {
            at_secs: at,
            per_min,
            break_before,
        }
    }

    fn label(secs: f64) -> String {
        format!("-{}s", secs.round() as i64)
    }

    #[test]
    fn nice_ceil_rounds_to_one_significant_figure() {
        assert_eq!(nice_ceil(0), 0);
        assert_eq!(nice_ceil(7), 7);
        assert_eq!(nice_ceil(1204), 2000);
        assert_eq!(nice_ceil(38_910), 40_000);
    }

    #[test]
    fn compact_count_matches_hfs_formatting() {
        assert_eq!(compact_count(0), "0");
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(2_000), "2.0k");
        assert_eq!(compact_count(61_400), "61.4k");
        assert_eq!(compact_count(1_500_000), "1.5M");
    }

    #[test]
    fn y_ticks_run_from_axis_max_down_to_zero() {
        let ticks = y_axis_ticks(4, CHART_HEIGHT, PLOT_BOTTOM);
        assert_eq!(ticks.len(), 5);
        assert_eq!(
            ticks.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(),
            vec!["4", "3", "2", "1", "0"],
            "an idle chart must read 4..0, not HFS's 1,0,0,0,0",
        );
        assert_eq!(ticks[0].pos, PLOT_TOP);
        assert_eq!(ticks[4].pos, PLOT_BOTTOM);
    }

    #[test]
    fn empty_input_still_renders_an_axis_frame_with_no_line() {
        let view = build_chart(&[], 1_000.0, 900.0, 6, 1, label);
        assert!(!view.has_data);
        assert!(view.polylines.is_empty(), "no data must plot no polyline");
        assert_eq!(view.y_ticks.len(), 5, "the axis frame still renders");
        assert_eq!(view.x_ticks.len(), 6);
        assert!(view.latest.is_empty());
    }

    #[test]
    fn x_is_mapped_by_timestamp_not_by_sample_index() {
        // Three points bunched in the last minute of a 15-minute window must
        // land on the right-hand edge, not spread across the whole axis the
        // way index mapping would put them.
        let now = 10_000.0;
        let points = [
            point(now - 45.0, 10, true),
            point(now - 30.0, 10, false),
            point(now - 15.0, 10, false),
        ];
        let view = build_chart(&points, now, 900.0, 6, 1, label);
        let xs: Vec<i64> = view.polylines[0]
            .split(' ')
            .map(|pair| pair.split(',').next().unwrap().parse().unwrap())
            .collect();
        assert!(
            xs.iter().all(|x| *x > 1_000),
            "recent samples must cluster at the right edge, got {xs:?}",
        );
        assert!(
            xs[0] < xs[2],
            "x must still increase with time, got {xs:?}",
        );
    }

    #[test]
    fn breaks_split_the_series_into_separate_polylines() {
        let now = 10_000.0;
        let points = [
            point(now - 800.0, 10, true),
            point(now - 785.0, 12, false),
            point(now - 60.0, 90, true), // resumed after a break
            point(now - 45.0, 95, false),
        ];
        let view = build_chart(&points, now, 900.0, 6, 1, label);
        assert_eq!(
            view.polylines.len(),
            2,
            "an unobserved stretch must break the line into two polylines",
        );
        assert_eq!(view.polylines[0].split(' ').count(), 2);
        assert_eq!(view.polylines[1].split(' ').count(), 2);
    }

    #[test]
    fn points_outside_the_window_are_dropped_not_squeezed_in() {
        let now = 10_000.0;
        let points = [
            point(now - 5_000.0, 500, true), // older than the 15 m window
            point(now - 100.0, 10, false),
        ];
        let view = build_chart(&points, now, 900.0, 6, 1, label);
        assert_eq!(view.polylines.len(), 1);
        assert_eq!(
            view.polylines[0].split(' ').count(),
            1,
            "only the in-window point survives",
        );
        assert_eq!(
            view.y_ticks[0].label, "10",
            "the out-of-window peak of 500 must not set the axis",
        );
    }

    #[test]
    fn the_first_in_window_point_opens_a_segment_even_without_a_break_flag() {
        // Its predecessor scrolled out of the window, so there is nothing to
        // connect it back to; it must not be dropped for lack of a segment.
        let now = 10_000.0;
        let points = [
            point(now - 2_000.0, 5, true),
            point(now - 100.0, 8, false),
        ];
        let view = build_chart(&points, now, 900.0, 6, 1, label);
        assert_eq!(view.polylines.len(), 1);
        assert_eq!(view.latest, "8");
    }

    #[test]
    fn the_axis_fits_the_in_window_peak() {
        let now = 10_000.0;
        let points = [point(now - 60.0, 130, true), point(now - 45.0, 40, false)];
        let view = build_chart(&points, now, 900.0, 6, 1, label);
        // nice_ceil(130) == 200, so the top gridline reads 200.
        assert_eq!(view.y_ticks[0].label, "200");
        assert_eq!(view.y_ticks[4].label, "0");
    }

    #[test]
    fn geometry_stays_inside_the_plot_box() {
        let now = 10_000.0;
        let points: Vec<RatePoint> = (0..40)
            .map(|i| point(now - 900.0 + i as f64 * 22.0, i * 7, i == 0))
            .collect();
        let view = build_chart(&points, now, 900.0, 6, 1, label);
        for pair in view.polylines.join(" ").split(' ') {
            let (x, y) = pair.split_once(',').expect("coordinate pair");
            let (x, y): (i64, i64) = (x.parse().unwrap(), y.parse().unwrap());
            assert!((PLOT_LEFT..=PLOT_RIGHT).contains(&x), "x out of plot box: {x}");
            assert!((PLOT_TOP..=PLOT_BOTTOM).contains(&y), "y out of plot box: {y}");
        }
    }
}
