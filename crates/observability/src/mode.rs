//! Observability arm selection (`HELIOS_OBS_MODE`).
//!
//! Per-request instrumentation is not free, and on a terminology server
//! answering tens of thousands of requests per second it is measurable against
//! the handler work itself. This module exists so that cost can be *attributed*
//! rather than argued about: run the same commit twice on the same host with
//! only `HELIOS_OBS_MODE` varying, and the throughput delta is the price of the
//! component that changed.
//!
//! It is a diagnostic knob, not a tuning knob. [`ObsMode::Default`] is what
//! production ships and what the benchmark measures unless told otherwise; the
//! other arms deliberately misconfigure the server to isolate a cost, and
//! [`ObsMode::Full`] exists to reproduce a historical (pre-fix) profile for
//! comparison. Reaching for a non-default arm to make a number look better
//! would be measuring a server nobody runs.

use std::sync::OnceLock;

/// Which per-request instrumentation [`crate::middleware::track`] performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObsMode {
    /// Production behaviour: Prometheus metrics always; the request span only
    /// when a tracing layer actually exports it; the reqlog ring buffer only
    /// when a server has registered a consumer for it.
    Default,
    /// Force the request span *and* reqlog recording on, regardless of whether
    /// anything consumes them. Reproduces the instrumentation profile that
    /// shipped before issue #227 was fixed, for A/B attribution.
    Full,
    /// Everything [`ObsMode::Default`] does, minus the request span.
    NoSpan,
    /// The middleware runs but performs no instrumentation. Establishes the
    /// floor: the difference between this and [`ObsMode::Default`] is the total
    /// price of observability on the request path.
    Off,
}

impl ObsMode {
    /// Whether `track` should open (and enter) a per-request tracing span.
    ///
    /// `traces_live` is the caller's answer to "is a layer actually exporting
    /// spans?" — see [`crate::telemetry::traces_live`]. Under
    /// [`ObsMode::Default`] an unexported span is pure cost, so it is skipped.
    pub fn span_enabled(self, traces_live: bool) -> bool {
        match self {
            ObsMode::Full => true,
            ObsMode::Off | ObsMode::NoSpan => false,
            ObsMode::Default => traces_live,
        }
    }

    /// Whether `track` should record Prometheus metrics. `/metrics` is a public
    /// product surface, so this is on everywhere except the [`ObsMode::Off`]
    /// floor arm.
    pub fn metrics_enabled(self) -> bool {
        self != ObsMode::Off
    }

    /// Whether `track` should push to the reqlog ring buffer.
    ///
    /// `consumer_registered` is the caller's answer to "has a server registered
    /// a reader for this buffer?" — see [`crate::reqlog::enabled`]. Under
    /// [`ObsMode::Default`] / [`ObsMode::NoSpan`] the buffer is filled only when
    /// something reads it; [`ObsMode::Full`] forces it on for A/B attribution,
    /// and [`ObsMode::Off`] forces it off.
    pub fn reqlog_enabled(self, consumer_registered: bool) -> bool {
        match self {
            ObsMode::Full => true,
            ObsMode::Off => false,
            ObsMode::Default | ObsMode::NoSpan => consumer_registered,
        }
    }
}

fn parse(raw: &str) -> Option<ObsMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "default" => Some(ObsMode::Default),
        "full" => Some(ObsMode::Full),
        "no-span" | "no_span" => Some(ObsMode::NoSpan),
        "off" => Some(ObsMode::Off),
        _ => None,
    }
}

/// The process-wide observability arm, read once from `HELIOS_OBS_MODE`.
///
/// An unrecognised value warns and falls back to [`ObsMode::Default`] rather
/// than failing startup: a typo in a diagnostic env var should not take a
/// server down, and the warning is enough to catch a mistyped benchmark arm.
pub fn mode() -> ObsMode {
    static MODE: OnceLock<ObsMode> = OnceLock::new();
    *MODE.get_or_init(|| {
        let raw = std::env::var("HELIOS_OBS_MODE").unwrap_or_default();
        parse(&raw).unwrap_or_else(|| {
            tracing::warn!(
                value = %raw,
                "unrecognised HELIOS_OBS_MODE; expected one of default|full|no-span|off — using default"
            );
            ObsMode::Default
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_arms() {
        assert_eq!(parse("default"), Some(ObsMode::Default));
        assert_eq!(parse(""), Some(ObsMode::Default));
        assert_eq!(parse("full"), Some(ObsMode::Full));
        assert_eq!(parse("no-span"), Some(ObsMode::NoSpan));
        assert_eq!(parse("no_span"), Some(ObsMode::NoSpan));
        assert_eq!(parse("off"), Some(ObsMode::Off));
    }

    #[test]
    fn parsing_is_case_and_whitespace_insensitive() {
        assert_eq!(parse("  FULL "), Some(ObsMode::Full));
        assert_eq!(parse("Off"), Some(ObsMode::Off));
    }

    #[test]
    fn rejects_unknown_arm() {
        assert_eq!(parse("verbose"), None);
    }

    #[test]
    fn default_arm_ties_span_to_a_live_exporter() {
        // The whole point of the fix: no exporter, no span.
        assert!(!ObsMode::Default.span_enabled(false));
        assert!(ObsMode::Default.span_enabled(true));
    }

    #[test]
    fn full_arm_forces_span_even_with_no_exporter() {
        assert!(ObsMode::Full.span_enabled(false));
    }

    #[test]
    fn off_arm_never_spans_even_with_a_live_exporter() {
        assert!(!ObsMode::Off.span_enabled(true));
        assert!(!ObsMode::NoSpan.span_enabled(true));
    }

    #[test]
    fn metrics_are_on_everywhere_except_the_floor_arm() {
        assert!(ObsMode::Default.metrics_enabled());
        assert!(ObsMode::Full.metrics_enabled());
        assert!(ObsMode::NoSpan.metrics_enabled());
        assert!(!ObsMode::Off.metrics_enabled());
    }

    #[test]
    fn default_arm_ties_reqlog_to_a_registered_consumer() {
        assert!(!ObsMode::Default.reqlog_enabled(false));
        assert!(ObsMode::Default.reqlog_enabled(true));
        assert!(!ObsMode::NoSpan.reqlog_enabled(false));
        assert!(ObsMode::NoSpan.reqlog_enabled(true));
    }

    #[test]
    fn full_forces_reqlog_and_off_suppresses_it() {
        assert!(ObsMode::Full.reqlog_enabled(false));
        assert!(!ObsMode::Off.reqlog_enabled(true));
    }

    #[test]
    fn mode_reads_once_and_is_stable() {
        // Exercises the process-global read path. The value depends on the
        // ambient `HELIOS_OBS_MODE` (unset in the unit-test process → Default),
        // so assert on the invariant that matters rather than a fixed arm: the
        // OnceLock hands back the same arm on every call.
        let first = mode();
        assert_eq!(first, mode());
    }
}
