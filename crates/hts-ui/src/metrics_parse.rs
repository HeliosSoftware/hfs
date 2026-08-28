//! Minimal Prometheus text-format parser for the Home dashboard.
//!
//! HTS already exposes `/metrics` via `helios_observability::metrics::router()`
//! and the Diagnostics page renders that body verbatim inside a `<pre>`. The
//! Home page (§7.1) needs *numeric* values out of it — total request count
//! and average request latency — without adding a heavyweight Prometheus
//! client dependency. This module walks the exposition once, groups samples
//! by metric name, and offers two aggregators tailored to those two tiles.
//!
//! Non-goals: we don't build a general-purpose Prometheus scraper. Exemplars,
//! `Info` metadata, and `EOF` markers of the OpenMetrics dialect are ignored.
//! Values that don't parse as `f64` (`NaN`, `+Inf`) drop the offending sample
//! rather than propagating an error — a single bad line must not blank the
//! entire card (fail-open per design §7 degraded contract).
//!
//! Format reference:
//!   <https://github.com/prometheus/docs/blob/main/content/docs/instrumenting/exposition_formats.md>

use std::collections::{BTreeMap, HashMap};

/// One data point emitted by `/metrics`. Labels are kept as a `BTreeMap` so
/// two samples with the same labels-in-different-order hash to the same
/// bucket for tests, and iteration order is deterministic when we surface a
/// sample in an error message.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub labels: BTreeMap<String, String>,
    pub value: f64,
}

/// Parse a full Prometheus text-format body into a `metric_name → Vec<Sample>`
/// map. Comment lines (`# HELP`, `# TYPE`, blank) are dropped; unparseable
/// lines are silently skipped so a single mis-emitted metric doesn't take
/// the whole aggregation with it.
pub fn parse(text: &str) -> HashMap<String, Vec<Sample>> {
    let mut out: HashMap<String, Vec<Sample>> = HashMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((name, sample)) = parse_line(trimmed) {
            out.entry(name).or_default().push(sample);
        }
    }
    out
}

/// Parse a single sample line. Returns `None` on any parse error rather than
/// bubbling — parsers that log-and-continue are strictly better here than
/// parsers that halt on a single garbled counter.
fn parse_line(line: &str) -> Option<(String, Sample)> {
    // Two shapes to handle:
    //   metric_name value [timestamp]
    //   metric_name{k="v",k2="v2"} value [timestamp]
    let (name_part, rest) = match line.find(|c: char| c == '{' || c.is_whitespace()) {
        Some(idx) => line.split_at(idx),
        None => return None,
    };
    if name_part.is_empty() {
        return None;
    }

    let (labels, value_str) = if let Some(rest_after_brace) = rest.strip_prefix('{') {
        let close_idx = rest_after_brace.find('}')?;
        let (raw_labels, tail) = rest_after_brace.split_at(close_idx);
        let labels = parse_labels(raw_labels);
        (labels, tail.trim_start_matches('}').trim_start())
    } else {
        (BTreeMap::new(), rest.trim_start())
    };

    // Value is the first whitespace-delimited token. Anything after (a
    // scrape timestamp) is discarded.
    let value_token = value_str.split_whitespace().next()?;
    let value: f64 = value_token.parse().ok()?;
    // Guard against NaN / infinite counters — those would poison every
    // downstream aggregation and are never emitted intentionally.
    if !value.is_finite() {
        return None;
    }

    Some((name_part.to_string(), Sample { labels, value }))
}

/// Parse the contents inside `{...}`. Values may contain escaped quotes and
/// backslashes per the Prometheus spec; we un-escape those two sequences
/// because they appear routinely in error labels. All other escapes pass
/// through as-is (good enough for the counters we consume).
fn parse_labels(inside: &str) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    let bytes = inside.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip whitespace / commas between k=v pairs.
        while i < bytes.len() && (bytes[i] == b',' || bytes[i].is_ascii_whitespace()) {
            i += 1;
        }
        // Key: up to '='.
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let key = std::str::from_utf8(&bytes[key_start..i])
            .unwrap_or("")
            .trim();
        i += 1; // skip '='
        // Value: quoted string.
        if i >= bytes.len() || bytes[i] != b'"' {
            break;
        }
        i += 1; // skip opening '"'
        let mut value = String::new();
        while i < bytes.len() && bytes[i] != b'"' {
            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                match bytes[i + 1] {
                    b'"' => value.push('"'),
                    b'\\' => value.push('\\'),
                    b'n' => value.push('\n'),
                    other => {
                        value.push('\\');
                        value.push(other as char);
                    }
                }
                i += 2;
            } else {
                value.push(bytes[i] as char);
                i += 1;
            }
        }
        i += 1; // skip closing '"'
        if !key.is_empty() {
            labels.insert(key.to_string(), value);
        }
    }
    labels
}

/// Sum every sample of a counter across all label combinations. Returns
/// `None` when the metric isn't present at all (as opposed to being zero).
pub fn sum_counter(map: &HashMap<String, Vec<Sample>>, name: &str) -> Option<f64> {
    let samples = map.get(name)?;
    if samples.is_empty() {
        return None;
    }
    Some(samples.iter().map(|s| s.value).sum())
}

/// Read a single-sample gauge (`uptime_seconds`). Returns `None` when the
/// metric is absent or carries no sample. When a gauge somehow reports more
/// than one series we take the first — `uptime_seconds` is unlabelled, so
/// that arm is defensive rather than expected.
pub fn gauge(map: &HashMap<String, Vec<Sample>>, name: &str) -> Option<f64> {
    map.get(name)?.first().map(|s| s.value)
}

/// Routes the Home request-rate chart must not count.
///
/// The Home page polls `/ui/hts/home/cards` every 15 s, and that poll is
/// itself an HTTP request the upstream counts. Left in the series, an idle
/// server would plot a permanent flat line of ~4 req/min — the chart
/// measuring its own refresh. `/metrics` is excluded for the same reason:
/// each poll scrapes it once. Both are matched against the `route` label,
/// which `helios_observability` fills from axum's templated `MatchedPath`,
/// so these are exact route templates, not raw URIs.
pub const SELF_ROUTES: [&str; 2] = ["/ui/hts/home/cards", "/metrics"];

/// Cumulative `http_requests_total` split by HTTP status class.
///
/// Each field is a *counter* value (monotonically increasing for the life of
/// the upstream process), never a rate. Turning these into a rate is
/// [`crate::metrics_ring`]'s job.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatusCounts {
    /// Every counted request, whatever its status class — including 1xx/3xx,
    /// which get no series of their own.
    pub all: u64,
    pub s2xx: u64,
    pub s4xx: u64,
    pub s5xx: u64,
}

/// Bucket `http_requests_total` by status class, dropping any sample whose
/// `route` label appears in `excluded_routes`.
///
/// Samples with an unparseable or missing `status` label still count toward
/// `all` — dropping them would understate total traffic, and `all` is the
/// default series. Fractional counter values (never emitted in practice) are
/// truncated toward zero rather than rejected.
pub fn request_counts_by_status_class(
    map: &HashMap<String, Vec<Sample>>,
    excluded_routes: &[&str],
) -> StatusCounts {
    let mut out = StatusCounts::default();
    let Some(samples) = map.get("http_requests_total") else {
        return out;
    };
    for sample in samples {
        if let Some(route) = sample.labels.get("route")
            && excluded_routes.contains(&route.as_str())
        {
            continue;
        }
        if sample.value < 0.0 {
            continue;
        }
        let value = sample.value as u64;
        out.all = out.all.saturating_add(value);
        match sample
            .labels
            .get("status")
            .and_then(|s| s.as_bytes().first())
        {
            Some(b'2') => out.s2xx = out.s2xx.saturating_add(value),
            Some(b'4') => out.s4xx = out.s4xx.saturating_add(value),
            Some(b'5') => out.s5xx = out.s5xx.saturating_add(value),
            _ => {}
        }
    }
    out
}

/// Compute the process-wide average of a histogram, expressed in the
/// histogram's native unit. Uses `sum(name_sum) / sum(name_count)` — the
/// canonical Prometheus recipe. Returns `None` when either series is
/// absent or the count is zero (division by zero is a lie, not a value).
pub fn histogram_avg(map: &HashMap<String, Vec<Sample>>, name: &str) -> Option<f64> {
    let sum_name = format!("{name}_sum");
    let count_name = format!("{name}_count");
    let sum: f64 = map.get(&sum_name)?.iter().map(|s| s.value).sum();
    let count: f64 = map.get(&count_name)?.iter().map(|s| s.value).sum();
    if count <= 0.0 {
        return None;
    }
    Some(sum / count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Realistic sample matching what `helios_observability::metrics` emits
    /// for the two series the Home page consumes. Trimmed to keep the test
    /// readable; a full body has hundreds of buckets.
    const SAMPLE: &str = "\
# HELP http_requests_total Total HTTP requests.
# TYPE http_requests_total counter
http_requests_total{method=\"GET\",route=\"/ui/hts\",status=\"200\",service=\"hts\"} 42
http_requests_total{method=\"GET\",route=\"/metrics\",status=\"200\",service=\"hts\"} 8
http_requests_total{method=\"POST\",route=\"/import\",status=\"200\",service=\"hts\"} 3
# HELP http_request_duration_seconds Request latency histogram.
# TYPE http_request_duration_seconds histogram
http_request_duration_seconds_bucket{route=\"/ui/hts\",le=\"0.1\"} 40
http_request_duration_seconds_bucket{route=\"/ui/hts\",le=\"+Inf\"} 42
http_request_duration_seconds_sum{route=\"/ui/hts\"} 1.2
http_request_duration_seconds_count{route=\"/ui/hts\"} 42
http_request_duration_seconds_sum{route=\"/metrics\"} 0.05
http_request_duration_seconds_count{route=\"/metrics\"} 8
";

    #[test]
    fn parse_extracts_counter_series() {
        let map = parse(SAMPLE);
        let counters = map.get("http_requests_total").expect("counter present");
        assert_eq!(counters.len(), 3);
        assert!(
            counters
                .iter()
                .any(|s| s.labels.get("route") == Some(&"/ui/hts".to_string()))
        );
    }

    #[test]
    fn sum_counter_sums_every_label_combination() {
        let map = parse(SAMPLE);
        let total = sum_counter(&map, "http_requests_total").expect("counter present");
        assert_eq!(total, 53.0);
    }

    #[test]
    fn sum_counter_returns_none_when_metric_missing() {
        let map = parse(SAMPLE);
        assert!(sum_counter(&map, "no_such_counter").is_none());
    }

    #[test]
    fn histogram_avg_computes_sum_over_count() {
        let map = parse(SAMPLE);
        let avg = histogram_avg(&map, "http_request_duration_seconds").expect("hist present");
        // Combined sum = 1.25 s over combined count = 50 → 0.025 s (25 ms).
        assert!((avg - 0.025).abs() < 1e-9, "expected ~0.025, got {avg}");
    }

    #[test]
    fn histogram_avg_returns_none_when_count_is_zero() {
        let text = "\
foo_seconds_sum 0
foo_seconds_count 0
";
        let map = parse(text);
        assert!(histogram_avg(&map, "foo_seconds").is_none());
    }

    #[test]
    fn nan_and_infinite_values_are_dropped() {
        // A `NaN` counter must not poison the aggregation.
        let text = "\
broken_counter NaN
broken_counter{route=\"/x\"} 5
broken_counter{route=\"/y\"} +Inf
";
        let map = parse(text);
        let samples = map
            .get("broken_counter")
            .expect("only finite samples survive");
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].value, 5.0);
    }

    #[test]
    fn labels_with_escaped_quotes_and_backslashes_parse() {
        let text = "custom_metric{msg=\"a \\\"quoted\\\" val\",path=\"C:\\\\tmp\"} 1\n";
        let map = parse(text);
        let s = &map.get("custom_metric").unwrap()[0];
        assert_eq!(s.labels.get("msg"), Some(&"a \"quoted\" val".to_string()));
        assert_eq!(s.labels.get("path"), Some(&"C:\\tmp".to_string()));
    }

    /// Self-traffic exclusion (§7.1 chart): the Home poll route and
    /// `/metrics` must not appear in the charted counts, or an idle server
    /// plots a flat line generated entirely by the chart's own refresh.
    #[test]
    fn request_counts_exclude_self_traffic_routes() {
        let text = "\
http_requests_total{method=\"GET\",route=\"/ui/hts\",status=\"200\"} 10
http_requests_total{method=\"GET\",route=\"/ui/hts/home/cards\",status=\"200\"} 400
http_requests_total{method=\"GET\",route=\"/metrics\",status=\"200\"} 400
http_requests_total{method=\"GET\",route=\"/CodeSystem/$lookup\",status=\"404\"} 3
http_requests_total{method=\"POST\",route=\"/import\",status=\"500\"} 1
";
        let map = parse(text);
        let counts = request_counts_by_status_class(&map, &SELF_ROUTES);
        assert_eq!(
            counts.all, 14,
            "self-traffic (800 requests) must be excluded from the charted total",
        );
        assert_eq!(counts.s2xx, 10);
        assert_eq!(counts.s4xx, 3);
        assert_eq!(counts.s5xx, 1);

        // Without the exclusion list every self-request is counted — proving
        // the filter, not the absence of the data.
        let unfiltered = request_counts_by_status_class(&map, &[]);
        assert_eq!(unfiltered.all, 814);
    }

    #[test]
    fn request_counts_keep_unlabelled_status_in_the_all_series() {
        // `all` is the default series; a sample we cannot classify must still
        // be counted there rather than silently understating traffic.
        let map = parse("http_requests_total{route=\"/x\"} 7\n");
        let counts = request_counts_by_status_class(&map, &SELF_ROUTES);
        assert_eq!(counts.all, 7);
        assert_eq!(counts.s2xx, 0);
    }

    #[test]
    fn request_counts_are_zero_when_the_counter_is_absent() {
        let counts = request_counts_by_status_class(&parse("other_metric 1\n"), &SELF_ROUTES);
        assert_eq!(counts, StatusCounts::default());
    }

    #[test]
    fn gauge_reads_uptime_seconds() {
        let map = parse("uptime_seconds 123.5\n");
        assert_eq!(gauge(&map, "uptime_seconds"), Some(123.5));
        assert!(gauge(&map, "missing_gauge").is_none());
    }

    #[test]
    fn comment_and_blank_lines_are_ignored() {
        let text = "\
# HELP something ignored
# TYPE something counter

garbled line without value
valid_counter 7
";
        let map = parse(text);
        assert_eq!(sum_counter(&map, "valid_counter"), Some(7.0));
        assert!(map.get("garbled").is_none());
    }
}
