//! Upstream HTS client for the administrative UI.
//!
//! HTS-UI ships **inside** the `hts` binary, but the handlers still speak
//! HTTP over loopback rather than reaching into the terminology backend
//! directly. That keeps the UI honest — every card and cell reflects what a
//! browser would see via the FHIR REST surface — and it is what makes the
//! `HTS_UI_UPSTREAM_URL` override (design doc §7 header) meaningful: a
//! developer or CI job can point the UI at a remote HTS without recompiling.
//!
//! # Base URL resolution
//!
//! - `HTS_UI_UPSTREAM_URL` env var, when set (design doc §7 degraded state
//!   contract E3);
//! - otherwise a loopback URL derived from the `hts` binary's `HTS_SERVER_HOST`
//!   / `HTS_SERVER_PORT` (passed by the mount site in [`crate::HtsUiState`]).
//!
//! # Error contract
//!
//! Every fetch returns [`UpstreamOutcome`]: either the parsed payload or a
//! [`UpstreamError`] that carries enough context for the dashboard to render a
//! translated OperationOutcome or degraded banner. The client never panics on
//! a 5xx; it is the operator's most sensitive surface.
//!
//! # FHIR shape parsing
//!
//! Search Bundles, CodeSystem reads, and Parameters responses are walked as
//! [`serde_json::Value`] rather than through the version-gated `helios-fhir`
//! types. This is deliberate for Slice B (design doc §7.2/§7.3): the UI only
//! needs a handful of fields per surface, and typed parsing would drag the
//! whole R4/R4B/R5/R6 cfg ladder into the UI code path for no gain — the raw
//! body is also echoed verbatim into the "Raw response" workbench panel per
//! §7.3, so nothing is discarded.

use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

/// Client for reaching HTS from inside the UI process.
///
/// Cheap to clone (wraps `reqwest::Client`); intended to live on the shared
/// [`crate::HtsUiState`].
#[derive(Clone, Debug)]
pub struct UpstreamClient {
    client: reqwest::Client,
    base_url: String,
}

impl UpstreamClient {
    /// Build a client pointed at `base_url` (no trailing `/`).
    ///
    /// Timeouts are conservative for a dashboard that polls every 15 s: any
    /// request slower than 5 s degrades to the surface's error state.
    pub fn new(base_url: impl Into<String>) -> Result<Self, UpstreamError> {
        Self::new_with_timeouts(base_url, Duration::from_secs(5), Duration::from_secs(2))
    }

    /// Build a client with custom timeouts. Exposed for tests that point at
    /// a closed loopback port and want the whole matrix to finish in seconds
    /// rather than minutes; production callers should use [`Self::new`].
    pub fn new_with_timeouts(
        base_url: impl Into<String>,
        request_timeout: Duration,
        connect_timeout: Duration,
    ) -> Result<Self, UpstreamError> {
        let mut base_url = base_url.into();
        while base_url.ends_with('/') {
            base_url.pop();
        }
        // Loopback targets must never route through a system HTTP(S) proxy:
        // corporate proxies typically try to DNS-resolve the target host, and
        // `127.0.0.1` / `localhost` collapse to "no such host". The mock
        // upstream in the Rust test ring binds to `127.0.0.1:0`; production
        // sidecar deployments frequently point HTS at loopback too.
        let mut builder = reqwest::Client::builder()
            .timeout(request_timeout)
            .connect_timeout(connect_timeout)
            .user_agent(concat!("helios-hts-ui/", env!("CARGO_PKG_VERSION")));
        if is_loopback_base_url(&base_url) {
            builder = builder.no_proxy();
        }
        let client = builder.build().map_err(|source| UpstreamError::ClientBuild {
            message: source.to_string(),
        })?;
        Ok(Self { client, base_url })
    }

    /// The base URL the client is pointed at, without a trailing slash.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// `GET /health` — the liveness probe. Returns the parsed JSON body.
    ///
    /// HTS does not gate `/health` on backend readiness, so a 200 here does
    /// not mean queries will succeed. The dashboard uses this only for the
    /// status / backend / uptime card; readiness for interactive controls
    /// comes from the `/metadata` fetch below (which does touch the backend).
    pub async fn health(&self) -> Result<UpstreamHealth, UpstreamError> {
        let url = format!("{}/health", self.base_url);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest("health", &url, e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(UpstreamError::HttpStatus {
                op: "health",
                url,
                status: status.as_u16(),
            });
        }
        response.json::<UpstreamHealth>().await.map_err(|e| {
            UpstreamError::Decode {
                op: "health",
                url,
                message: e.to_string(),
            }
        })
    }

    /// `GET /metadata?mode=terminology` — the `TerminologyCapabilities`.
    ///
    /// The dashboard reads three things from this: the FHIR version chip,
    /// the count of loaded `codeSystem[]` entries (design doc §7.1 "loaded
    /// systems"), and the advertised expansion / validation parameter names.
    pub async fn terminology_capabilities(
        &self,
    ) -> Result<UpstreamTerminologyCapabilities, UpstreamError> {
        let url = format!("{}/metadata?mode=terminology", self.base_url);
        let response = self
            .client
            .get(&url)
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest("metadata", &url, e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(UpstreamError::HttpStatus {
                op: "metadata",
                url,
                status: status.as_u16(),
            });
        }
        response
            .json::<UpstreamTerminologyCapabilities>()
            .await
            .map_err(|e| UpstreamError::Decode {
                op: "metadata",
                url,
                message: e.to_string(),
            })
    }
}

/// Parsed shape of HTS's `/health` response.
///
/// Only the fields the dashboard renders are represented; unknown extras are
/// accepted silently so the UI does not brittle-crash when the server grows a
/// new probe field.
#[derive(Clone, Debug, Deserialize)]
pub struct UpstreamHealth {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub uptime_seconds: u64,
}

/// Parsed shape of the `TerminologyCapabilities` fields the dashboard reads.
///
/// A minimal projection: enough for the "loaded systems" count and the FHIR
/// version chip. Detail pages get the full resource when they need it.
#[derive(Clone, Debug, Deserialize, Default)]
pub struct UpstreamTerminologyCapabilities {
    #[serde(rename = "resourceType", default)]
    pub resource_type: String,
    #[serde(rename = "fhirVersion", default)]
    pub fhir_version: String,
    #[serde(rename = "codeSystem", default)]
    pub code_system: Vec<UpstreamCapabilitiesCodeSystem>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct UpstreamCapabilitiesCodeSystem {
    #[serde(default)]
    pub uri: String,
}

/// Fetch failures the dashboard can render as a degraded banner or an
/// OperationOutcome-shaped card, with enough context for triage but no
/// upstream implementation details.
#[derive(Clone, Debug, thiserror::Error)]
pub enum UpstreamError {
    #[error("failed to build upstream client: {message}")]
    ClientBuild { message: String },

    #[error("upstream `{op}` at {url} failed to connect: {message}")]
    Connect {
        op: &'static str,
        url: String,
        message: String,
    },

    #[error("upstream `{op}` at {url} timed out: {message}")]
    Timeout {
        op: &'static str,
        url: String,
        message: String,
    },

    #[error("upstream `{op}` at {url} returned HTTP {status}")]
    HttpStatus {
        op: &'static str,
        url: String,
        status: u16,
    },

    /// 404 from HTS — used to distinguish "unknown or soft-deleted resource"
    /// from generic 5xx so the detail page can render an explanatory
    /// OperationOutcome (design doc §7.3) instead of collapsing to a page
    /// 404. HTS returns 404 for both truly-missing and soft-deleted
    /// resources; the UI cannot tell them apart at the HTTP layer.
    #[error("upstream `{op}` at {url} returned 404 not-found")]
    NotFound { op: &'static str, url: String },

    /// HTS returned a JSON `OperationOutcome` alongside a non-success status.
    /// The parsed view feeds `partials/hts-outcome.html` (§7 error contract).
    #[error("upstream `{op}` at {url} returned OperationOutcome ({status})")]
    Outcome {
        op: &'static str,
        url: String,
        status: u16,
        outcome: OutcomeView,
    },

    #[error("upstream `{op}` at {url} returned an unrecognized body: {message}")]
    Decode {
        op: &'static str,
        url: String,
        message: String,
    },
}

impl UpstreamError {
    fn from_reqwest(op: &'static str, url: &str, source: reqwest::Error) -> Self {
        if source.is_timeout() {
            Self::Timeout {
                op,
                url: url.to_owned(),
                message: source.to_string(),
            }
        } else {
            Self::Connect {
                op,
                url: url.to_owned(),
                message: source.to_string(),
            }
        }
    }

    /// A short, translation-friendly reason key for the degraded partial.
    ///
    /// The Fluent lookup key is `hts-degraded-reason-<reason>`; unknown
    /// reasons render as `hts-degraded-reason-unknown` (see the partial).
    pub fn degraded_reason(&self) -> &'static str {
        match self {
            Self::ClientBuild { .. } => "client-build",
            Self::Connect { .. } => "upstream-down",
            Self::Timeout { .. } => "upstream-timeout",
            Self::HttpStatus { .. } | Self::Outcome { .. } => "upstream-error",
            Self::NotFound { .. } => "upstream-not-found",
            Self::Decode { .. } => "upstream-shape",
        }
    }
}

// ── Helpers required by templates ────────────────────────────────────────

impl UpstreamHealth {
    /// Human-readable uptime (`3d 4h 12m`) for the dashboard tile. Rendered
    /// as a placeable in the Fluent catalog rather than composed as prose in
    /// the template, so translations control their own formatting.
    pub fn uptime_pretty(&self) -> String {
        let mut secs = self.uptime_seconds;
        let days = secs / 86_400;
        secs %= 86_400;
        let hours = secs / 3_600;
        secs %= 3_600;
        let mins = secs / 60;
        if days > 0 {
            format!("{days}d {hours}h {mins}m")
        } else if hours > 0 {
            format!("{hours}h {mins}m")
        } else {
            format!("{mins}m")
        }
    }
}

impl UpstreamTerminologyCapabilities {
    /// Number of loaded code systems, per `TerminologyCapabilities.codeSystem[]`.
    /// This is what the dashboard shows for "Loaded systems: N" — the accurate
    /// replacement for the earlier inventory-count design that HTS could not
    /// serve without a Phase 1.5 admin route (design doc §7.1 E2).
    pub fn loaded_system_count(&self) -> usize {
        self.code_system.len()
    }
}

// ── Slice B: CodeSystem browser + detail + workbench ─────────────────────
//
// Types and methods below back the CS browser (§7.2), detail (§7.3), and the
// three embedded workbench operations ($lookup, $validate-code, $subsumes)
// per design doc §6.1 / §7.3. The raw HTS body for each operation is kept on
// the result so the workbench's "Raw response" panel can echo it verbatim
// (§7.3 wireframe).

/// A parsed FHIR `OperationOutcome` projection, feeding
/// `partials/hts-outcome.html` (Slice A shape).
#[derive(Clone, Debug, Default)]
pub struct OutcomeView {
    pub severity: String,
    pub code: String,
    pub diagnostics: String,
    pub location: Vec<String>,
    pub request_id: Option<String>,
}

impl OutcomeView {
    /// Best-effort parse of an `OperationOutcome` body. Missing / malformed
    /// bodies collapse to a synthetic outcome (`code=unknown`) rather than
    /// erroring — the caller has already decided this is an error path and
    /// needs something to render.
    pub fn from_body(body: &Value) -> Self {
        let issue = body
            .get("issue")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first());
        let severity = issue
            .and_then(|i| i.get("severity"))
            .and_then(|s| s.as_str())
            .unwrap_or("error")
            .to_owned();
        let code = issue
            .and_then(|i| i.get("code"))
            .and_then(|s| s.as_str())
            .unwrap_or("unknown")
            .to_owned();
        let diagnostics = issue
            .and_then(|i| i.get("diagnostics"))
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_owned();
        let location = issue
            .and_then(|i| i.get("location"))
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            severity,
            code,
            diagnostics,
            location,
            request_id: None,
        }
    }

    /// Synthetic outcome for pre-flight validation failures (design doc
    /// §7.3 states matrix): no HTS round-trip, just a translated code and
    /// a diagnostic string the template already localises.
    pub fn invalid_input(diagnostics: impl Into<String>) -> Self {
        Self {
            severity: "error".to_owned(),
            code: "invalid".to_owned(),
            diagnostics: diagnostics.into(),
            location: Vec::new(),
            request_id: None,
        }
    }
}

/// Filters accepted by the CS browser page and its rows fragment.
///
/// `_count` and `_offset` are parsed from strings so a malformed query
/// (`_count=abc`) collapses to the defaults instead of surfacing a 400 —
/// the browser is a discovery surface and a broken pager is worse than a
/// silently-defaulted one.
#[derive(Clone, Debug, Default)]
pub struct CsBrowserFilters {
    pub url: Option<String>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub count: u32,
    pub offset: u32,
}

impl CsBrowserFilters {
    /// Hard cap on `_count` per design doc §7.2. Above this the handler
    /// rejects the request; the form input's `max` attribute is the
    /// client-side mirror.
    pub const MAX_COUNT: u32 = 100;
    pub const DEFAULT_COUNT: u32 = 25;

    /// The effective page size, clamped to the `[1, MAX_COUNT]` range.
    /// Zero collapses to the default so an empty query renders a full page.
    pub fn effective_count(&self) -> u32 {
        if self.count == 0 {
            Self::DEFAULT_COUNT
        } else {
            self.count.clamp(1, Self::MAX_COUNT)
        }
    }

    /// Whether the requested `_count` exceeded the hard cap. The handler
    /// rejects the request when this is true (design doc §7.2 states matrix).
    pub fn count_exceeds_cap(&self) -> bool {
        self.count > Self::MAX_COUNT
    }
}

/// One row of the CS browser table (§7.2 wireframe).
///
/// Fields that HTS may omit render as an em-dash in the template — the row
/// projection intentionally stores empty strings rather than `Option` so
/// the template branch collapses to a single dash and stays legible.
#[derive(Clone, Debug, Default)]
pub struct CsBrowserRow {
    pub id: String,
    pub url: String,
    pub version: String,
    pub name: String,
    pub title: String,
    pub status: String,
}

impl CsBrowserRow {
    fn from_resource(resource: &Value) -> Self {
        let s = |k: &str| {
            resource
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned()
        };
        Self {
            id: s("id"),
            url: s("url"),
            version: s("version"),
            name: s("name"),
            title: s("title"),
            status: s("status"),
        }
    }
}

/// The browser rows partial's data. `filters` is echoed so the page can
/// preserve them in the pager and reset form state (§7.2).
#[derive(Clone, Debug)]
pub struct CsBrowserPage {
    pub rows: Vec<CsBrowserRow>,
    pub filters: CsBrowserFilters,
}

impl CsBrowserPage {
    /// The next offset to request, or `None` when HTS returned fewer rows
    /// than the requested page size (the terminal-page heuristic — HTS's
    /// `total` is a page count, not an authoritative match count, so we
    /// can't rely on it; see hts-details.md §Search).
    pub fn next_offset(&self) -> Option<u32> {
        let requested = self.filters.effective_count();
        if (self.rows.len() as u32) >= requested {
            Some(self.filters.offset.saturating_add(self.rows.len() as u32))
        } else {
            None
        }
    }

    /// Row count for the localized "Showing N" strip (§7.2 wireframe).
    pub fn showing_count(&self) -> usize {
        self.rows.len()
    }
}

/// Metadata projection consumed by the CS detail page (§7.3 wireframe).
/// Anything the detail page renders is projected here; the raw resource
/// stays reachable via [`CodeSystemSummary::raw_body`] for the workbench's
/// "Raw response" affordance and future slices.
#[derive(Clone, Debug, Default)]
pub struct CodeSystemSummary {
    pub id: String,
    pub url: String,
    pub version: String,
    pub name: String,
    pub title: String,
    pub status: String,
    pub description: String,
    pub publisher: String,
    pub jurisdiction: Vec<String>,
    pub content: String,
    pub count: Option<u64>,
    pub supersedes: Vec<String>,
    pub superseded_by: Option<String>,
    pub raw_body: String,
}

impl CodeSystemSummary {
    fn from_resource(resource: &Value, raw_body: String) -> Self {
        let s = |k: &str| {
            resource
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned()
        };
        let count = resource
            .get("count")
            .and_then(|v| v.as_u64());
        let jurisdiction = resource
            .get("jurisdiction")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|cc| {
                        cc.get("text")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned)
                            .or_else(|| {
                                cc.get("coding")
                                    .and_then(|v| v.as_array())
                                    .and_then(|codings| codings.first())
                                    .and_then(|c| {
                                        c.get("display")
                                            .and_then(|v| v.as_str())
                                            .map(str::to_owned)
                                    })
                            })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let (supersedes, superseded_by) = extract_supersede_extensions(resource);
        Self {
            id: s("id"),
            url: s("url"),
            version: s("version"),
            name: s("name"),
            title: s("title"),
            status: s("status"),
            description: s("description"),
            publisher: s("publisher"),
            jurisdiction,
            content: s("content"),
            count,
            supersedes,
            superseded_by,
            raw_body,
        }
    }

    /// The heading used in the detail page's H1 — title falls back to name
    /// falls back to id so the operator always sees a legible label.
    pub fn heading(&self) -> &str {
        if !self.title.is_empty() {
            &self.title
        } else if !self.name.is_empty() {
            &self.name
        } else {
            &self.id
        }
    }
}

fn extract_supersede_extensions(resource: &Value) -> (Vec<String>, Option<String>) {
    let extensions = match resource.get("extension").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return (Vec::new(), None),
    };
    let mut supersedes = Vec::new();
    let mut superseded_by = None;
    for ext in extensions {
        let url = ext.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        let value = ext
            .get("valueUri")
            .or_else(|| ext.get("valueCanonical"))
            .or_else(|| ext.get("valueUrl"))
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        match (url, value) {
            (u, Some(v)) if u.ends_with("/replaces") || u.ends_with("supersedes") => {
                supersedes.push(v);
            }
            (u, Some(v)) if u.ends_with("/replacedBy") || u.ends_with("supersededBy") => {
                superseded_by = Some(v);
            }
            _ => {}
        }
    }
    (supersedes, superseded_by)
}

/// Input to `POST /CodeSystem[/{id}]/$lookup` (design doc §7.3 Lookup tab).
///
/// Field notes:
/// - `properties` is repeatable and includes the special `*` value; empty
///   collapses to "no property filter" (HTS returns all defaults).
/// - `expression` is intentionally absent — HTS returns 501 for it and the
///   design doc §7.3 forbids exposing it.
#[derive(Clone, Debug, Default)]
pub struct LookupParams {
    pub code: String,
    pub version: Option<String>,
    pub display_language: Option<String>,
    pub properties: Vec<String>,
    pub date: Option<String>,
}

/// Response projection for `$lookup` (design doc §6.3 concept renderer).
///
/// `raw_body` is the pretty-printed HTS JSON so the workbench can echo it in
/// its "Raw response" panel unchanged (design doc §7.3 wireframe).
#[derive(Clone, Debug, Default)]
pub struct LookupResult {
    pub name: String,
    pub version: String,
    pub display: String,
    pub definition: String,
    pub designations: Vec<LookupDesignation>,
    pub properties: Vec<LookupProperty>,
    pub raw_body: String,
    pub request_url: String,
}

#[derive(Clone, Debug, Default)]
pub struct LookupDesignation {
    pub language: String,
    pub use_code: String,
    pub value: String,
}

#[derive(Clone, Debug, Default)]
pub struct LookupProperty {
    pub code: String,
    pub value: String,
}

/// Input to `POST /CodeSystem/$validate-code`.
///
/// Slice B exposes the two most common modes — bare `code` and structured
/// `Coding` — matching the CS detail workbench wireframe (§7.3). The
/// `CodeableConcept` mode from ui-design-map §3 is deferred to the
/// standalone operations workbench (Slice E) where the extra chrome fits.
#[derive(Clone, Debug, Default)]
pub struct ValidateCodeParams {
    pub mode: ValidateInputMode,
    pub code: String,
    pub display: Option<String>,
    pub coding_system: String,
    pub coding_code: String,
    pub coding_display: Option<String>,
    pub display_language: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ValidateInputMode {
    #[default]
    Code,
    Coding,
}

impl ValidateInputMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Coding => "coding",
        }
    }

    pub fn from_form(s: Option<&str>) -> Self {
        match s.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("coding") => Self::Coding,
            _ => Self::Code,
        }
    }
}

/// Response projection for `$validate-code`. `result=false` is HTTP 200,
/// not an error — the badge template branches on `.result`.
#[derive(Clone, Debug, Default)]
pub struct ValidateCodeResult {
    pub result: bool,
    pub code: String,
    pub system: String,
    pub version: String,
    pub display: String,
    pub message: String,
    pub issues: Option<OutcomeView>,
    pub raw_body: String,
    pub request_url: String,
}

/// Input to `POST /CodeSystem/$subsumes` (design doc §7.3 Subsumes tab).
///
/// HTS's `$subsumes` requires both codes to share a system; the UI enforces
/// this by pinning `system` to the current CS canonical URL (embedded via
/// hidden form field) and asking only for `codeA` / `codeB`.
#[derive(Clone, Debug, Default)]
pub struct SubsumesParams {
    pub code_a: String,
    pub code_b: String,
    pub version: Option<String>,
}

/// Response projection for `$subsumes`. `outcome` is one of `equivalent`,
/// `subsumes`, `subsumed-by`, or `not-subsumed` (design doc §7.3 wireframe).
#[derive(Clone, Debug, Default)]
pub struct SubsumesResult {
    pub outcome: String,
    pub raw_body: String,
    pub request_url: String,
}

// -- HTTP method plumbing ------------------------------------------------

impl UpstreamClient {
    /// `GET /CodeSystem?...` — the browser rows fragment source.
    ///
    /// Only fields the row projection renders are extracted; the full Bundle
    /// is discarded. HTS's `total` in the Bundle is a *page* count and is
    /// deliberately not exposed to callers — the pager uses length-of-rows
    /// as the terminal-page heuristic (see [`CsBrowserPage::next_offset`]).
    pub async fn search_code_systems(
        &self,
        filters: &CsBrowserFilters,
    ) -> Result<CsBrowserPage, UpstreamError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(ref v) = filters.url {
            if !v.is_empty() {
                query.push(("url", v.clone()));
            }
        }
        if let Some(ref v) = filters.version {
            if !v.is_empty() {
                query.push(("version", v.clone()));
            }
        }
        if let Some(ref v) = filters.name {
            if !v.is_empty() {
                query.push(("name", v.clone()));
            }
        }
        if let Some(ref v) = filters.title {
            if !v.is_empty() {
                query.push(("title", v.clone()));
            }
        }
        if let Some(ref v) = filters.status {
            if !v.is_empty() {
                query.push(("status", v.clone()));
            }
        }
        query.push(("_count", filters.effective_count().to_string()));
        query.push(("_offset", filters.offset.to_string()));

        let url = format!("{}/CodeSystem", self.base_url);
        let response = self
            .client
            .get(&url)
            .query(&query)
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest("cs-search", &url, e))?;
        let status = response.status();
        let body: Value = response.json().await.map_err(|e| UpstreamError::Decode {
            op: "cs-search",
            url: url.clone(),
            message: e.to_string(),
        })?;
        if !status.is_success() {
            return Err(status_to_error("cs-search", &url, status.as_u16(), &body));
        }
        let rows: Vec<CsBrowserRow> = body
            .get("entry")
            .and_then(|v| v.as_array())
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|e| e.get("resource"))
                    .filter(|r| {
                        r.get("resourceType")
                            .and_then(|t| t.as_str())
                            .map(|t| t == "CodeSystem")
                            .unwrap_or(true)
                    })
                    .map(CsBrowserRow::from_resource)
                    .collect()
            })
            .unwrap_or_default();
        Ok(CsBrowserPage {
            rows,
            filters: filters.clone(),
        })
    }

    /// `GET /CodeSystem/{id}` — CS detail page source.
    ///
    /// A 404 surfaces as [`UpstreamError::NotFound`] so the detail handler
    /// can render an explanatory OperationOutcome partial (design doc §7.3)
    /// rather than propagating a page 404. Every other non-2xx flows
    /// through the standard `Outcome` / `HttpStatus` arm.
    pub async fn read_code_system(
        &self,
        id: &str,
    ) -> Result<CodeSystemSummary, UpstreamError> {
        let url = format!("{}/CodeSystem/{}", self.base_url, id);
        let response = self
            .client
            .get(&url)
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest("cs-read", &url, e))?;
        let status = response.status();
        let raw = response.text().await.map_err(|e| UpstreamError::Decode {
            op: "cs-read",
            url: url.clone(),
            message: e.to_string(),
        })?;
        if !status.is_success() {
            if status.as_u16() == 404 {
                return Err(UpstreamError::NotFound {
                    op: "cs-read",
                    url,
                });
            }
            let parsed: Value = serde_json::from_str(&raw).unwrap_or_default();
            return Err(status_to_error("cs-read", &url, status.as_u16(), &parsed));
        }
        let parsed: Value = serde_json::from_str(&raw).map_err(|e| UpstreamError::Decode {
            op: "cs-read",
            url: url.clone(),
            message: e.to_string(),
        })?;
        let pretty = serde_json::to_string_pretty(&parsed).unwrap_or(raw);
        Ok(CodeSystemSummary::from_resource(&parsed, pretty))
    }

    /// `POST /CodeSystem/{id}/$lookup` (design doc §7.6 proxy verb rule).
    ///
    /// The instance route derives `system` from `{id}` (hts-details.md
    /// §CodeSystem `$lookup`), so no canonical URL needs to be threaded
    /// through the UI form.
    pub async fn cs_lookup(
        &self,
        id: &str,
        params: LookupParams,
    ) -> Result<LookupResult, UpstreamError> {
        let mut parameters: Vec<Value> = Vec::new();
        parameters.push(json!({"name": "code", "valueCode": params.code}));
        if let Some(v) = trim_opt(params.version) {
            parameters.push(json!({"name": "version", "valueString": v}));
        }
        if let Some(v) = trim_opt(params.display_language) {
            parameters.push(json!({"name": "displayLanguage", "valueCode": v}));
        }
        if let Some(v) = trim_opt(params.date) {
            parameters.push(json!({"name": "date", "valueDateTime": v}));
        }
        for prop in params.properties.iter().filter(|p| !p.trim().is_empty()) {
            parameters.push(json!({"name": "property", "valueCode": prop}));
        }
        let body = json!({
            "resourceType": "Parameters",
            "parameter": parameters,
        });
        let url = format!("{}/CodeSystem/{}/$lookup", self.base_url, id);
        let (raw, parsed) = self.post_parameters("cs-lookup", &url, &body).await?;
        let mut result = LookupResult {
            raw_body: raw,
            request_url: url.clone(),
            ..LookupResult::default()
        };
        for (name, value_obj) in iter_parameters(&parsed) {
            match name {
                "name" => result.name = value_obj_str(value_obj, "valueString").to_owned(),
                "version" => result.version = value_obj_str(value_obj, "valueString").to_owned(),
                "display" => result.display = value_obj_str(value_obj, "valueString").to_owned(),
                "definition" => {
                    result.definition = value_obj_str(value_obj, "valueString").to_owned()
                }
                "designation" => {
                    if let Some(d) = parse_lookup_designation(value_obj) {
                        result.designations.push(d);
                    }
                }
                "property" => {
                    if let Some(p) = parse_lookup_property(value_obj) {
                        result.properties.push(p);
                    }
                }
                _ => {}
            }
        }
        Ok(result)
    }

    /// `POST /CodeSystem/$validate-code` (design doc §7.6 proxy verb rule).
    ///
    /// HTS has no CS instance-level `$validate-code` (hts-details.md), so
    /// the caller must pass the resolved canonical URL — the detail page
    /// gets it from the CS read that already backs the page.
    pub async fn cs_validate_code(
        &self,
        canonical_url: &str,
        params: ValidateCodeParams,
    ) -> Result<ValidateCodeResult, UpstreamError> {
        let mut parameters: Vec<Value> = Vec::new();
        parameters.push(json!({"name": "url", "valueUri": canonical_url}));
        match params.mode {
            ValidateInputMode::Code => {
                parameters.push(json!({"name": "code", "valueCode": params.code.clone()}));
                if let Some(v) = trim_opt(params.display.clone()) {
                    parameters.push(json!({"name": "display", "valueString": v}));
                }
            }
            ValidateInputMode::Coding => {
                let mut coding = serde_json::Map::new();
                coding.insert(
                    "system".to_string(),
                    Value::String(params.coding_system.clone()),
                );
                coding.insert(
                    "code".to_string(),
                    Value::String(params.coding_code.clone()),
                );
                if let Some(display) = trim_opt(params.coding_display.clone()) {
                    coding.insert("display".to_string(), Value::String(display));
                }
                parameters.push(json!({
                    "name": "coding",
                    "valueCoding": Value::Object(coding),
                }));
            }
        }
        if let Some(v) = trim_opt(params.display_language) {
            parameters.push(json!({"name": "displayLanguage", "valueCode": v}));
        }
        let body = json!({
            "resourceType": "Parameters",
            "parameter": parameters,
        });
        let url = format!("{}/CodeSystem/$validate-code", self.base_url);
        let (raw, parsed) = self.post_parameters("cs-validate", &url, &body).await?;
        let mut out = ValidateCodeResult {
            raw_body: raw,
            request_url: url.clone(),
            ..ValidateCodeResult::default()
        };
        for (name, value_obj) in iter_parameters(&parsed) {
            match name {
                "result" => {
                    out.result = value_obj
                        .get("valueBoolean")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                }
                "code" => out.code = value_obj_str(value_obj, "valueCode").to_owned(),
                "system" => out.system = value_obj_str(value_obj, "valueUri").to_owned(),
                "version" => out.version = value_obj_str(value_obj, "valueString").to_owned(),
                "display" => out.display = value_obj_str(value_obj, "valueString").to_owned(),
                "message" => out.message = value_obj_str(value_obj, "valueString").to_owned(),
                "issues" => {
                    if let Some(resource) = value_obj.get("resource") {
                        out.issues = Some(OutcomeView::from_body(resource));
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// `POST /CodeSystem/$subsumes` (design doc §7.6 proxy verb rule).
    ///
    /// Both codes are pinned to `canonical_url` server-side, matching HTS's
    /// requirement that codeA and codeB share a system (hts-details.md
    /// §`$subsumes`).
    pub async fn cs_subsumes(
        &self,
        canonical_url: &str,
        params: SubsumesParams,
    ) -> Result<SubsumesResult, UpstreamError> {
        let mut parameters: Vec<Value> = Vec::new();
        parameters.push(json!({"name": "system", "valueUri": canonical_url}));
        parameters.push(json!({"name": "codeA", "valueCode": params.code_a}));
        parameters.push(json!({"name": "codeB", "valueCode": params.code_b}));
        if let Some(v) = trim_opt(params.version) {
            parameters.push(json!({"name": "version", "valueString": v}));
        }
        let body = json!({
            "resourceType": "Parameters",
            "parameter": parameters,
        });
        let url = format!("{}/CodeSystem/$subsumes", self.base_url);
        let (raw, parsed) = self.post_parameters("cs-subsumes", &url, &body).await?;
        let mut out = SubsumesResult {
            raw_body: raw,
            request_url: url.clone(),
            ..SubsumesResult::default()
        };
        for (name, value_obj) in iter_parameters(&parsed) {
            if name == "outcome" {
                out.outcome = value_obj_str(value_obj, "valueCode").to_owned();
                break;
            }
        }
        Ok(out)
    }

    /// `POST /CodeSystem/$lookup` (type-level route, design doc §7.6). The
    /// standalone Operations workbench does not have an instance id on hand;
    /// it pins the CodeSystem via a `system=` parameter in the Parameters
    /// body. HTS's type-level `$lookup` accepts this shape per hts-details.md.
    pub async fn cs_lookup_type_level(
        &self,
        system: &str,
        params: LookupParams,
    ) -> Result<LookupResult, UpstreamError> {
        let system = system.trim();
        let mut parameters: Vec<Value> = Vec::new();
        parameters.push(json!({"name": "system", "valueUri": system}));
        parameters.push(json!({"name": "code", "valueCode": params.code}));
        if let Some(v) = trim_opt(params.version) {
            parameters.push(json!({"name": "version", "valueString": v}));
        }
        if let Some(v) = trim_opt(params.display_language) {
            parameters.push(json!({"name": "displayLanguage", "valueCode": v}));
        }
        if let Some(v) = trim_opt(params.date) {
            parameters.push(json!({"name": "date", "valueDateTime": v}));
        }
        for prop in params.properties.iter().filter(|p| !p.trim().is_empty()) {
            parameters.push(json!({"name": "property", "valueCode": prop}));
        }
        let body = json!({
            "resourceType": "Parameters",
            "parameter": parameters,
        });
        let url = format!("{}/CodeSystem/$lookup", self.base_url);
        let (raw, parsed) = self.post_parameters("cs-lookup", &url, &body).await?;
        let mut result = LookupResult {
            raw_body: raw,
            request_url: url,
            ..LookupResult::default()
        };
        for (name, value_obj) in iter_parameters(&parsed) {
            match name {
                "name" => result.name = value_obj_str(value_obj, "valueString").to_owned(),
                "version" => result.version = value_obj_str(value_obj, "valueString").to_owned(),
                "display" => result.display = value_obj_str(value_obj, "valueString").to_owned(),
                "definition" => {
                    result.definition = value_obj_str(value_obj, "valueString").to_owned()
                }
                "designation" => {
                    if let Some(d) = parse_lookup_designation(value_obj) {
                        result.designations.push(d);
                    }
                }
                "property" => {
                    if let Some(p) = parse_lookup_property(value_obj) {
                        result.properties.push(p);
                    }
                }
                _ => {}
            }
        }
        Ok(result)
    }

    /// Shared POST wrapper for the three CS operation proxies. Returns the
    /// pretty-printed raw body alongside the parsed `Value` so the workbench
    /// result partial can echo the wire response unchanged.
    async fn post_parameters(
        &self,
        op: &'static str,
        url: &str,
        body: &Value,
    ) -> Result<(String, Value), UpstreamError> {
        let response = self
            .client
            .post(url)
            .header("Accept", "application/fhir+json")
            .header("Content-Type", "application/fhir+json")
            .json(body)
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest(op, url, e))?;
        let status = response.status();
        let raw = response.text().await.map_err(|e| UpstreamError::Decode {
            op,
            url: url.to_owned(),
            message: e.to_string(),
        })?;
        let parsed: Value = serde_json::from_str(&raw).map_err(|e| UpstreamError::Decode {
            op,
            url: url.to_owned(),
            message: e.to_string(),
        })?;
        if !status.is_success() {
            return Err(status_to_error(op, url, status.as_u16(), &parsed));
        }
        let pretty = serde_json::to_string_pretty(&parsed).unwrap_or(raw);
        Ok((pretty, parsed))
    }
}

// -- Small parsing helpers (private) -------------------------------------

/// Returns true when `base_url` targets a loopback host (`127.0.0.0/8`,
/// `::1`, or literal `localhost`). Used to disable system-proxy pickup so
/// requests to a same-process mock upstream don't get DNS-resolved through
/// a corporate `HTTP(S)_PROXY`.
fn is_loopback_base_url(base_url: &str) -> bool {
    let after_scheme = base_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(base_url);
    let host = after_scheme
        .split(|c: char| c == '/' || c == '?' || c == '#')
        .next()
        .unwrap_or("");
    let host = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host)
    };
    host.eq_ignore_ascii_case("localhost")
        || host == "::1"
        || host.starts_with("127.")
}

fn trim_opt(v: Option<String>) -> Option<String> {
    v.and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn iter_parameters(body: &Value) -> impl Iterator<Item = (&str, &Value)> {
    body.get("parameter")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|p| {
            let name = p.get("name").and_then(|v| v.as_str())?;
            Some((name, p))
        })
}

fn value_obj_str<'a>(param: &'a Value, key: &str) -> &'a str {
    param.get(key).and_then(|v| v.as_str()).unwrap_or_default()
}

fn parse_lookup_designation(param: &Value) -> Option<LookupDesignation> {
    let parts = param.get("part").and_then(|v| v.as_array())?;
    let mut d = LookupDesignation::default();
    for part in parts {
        let name = match part.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => continue,
        };
        match name {
            "language" => d.language = value_obj_str(part, "valueCode").to_owned(),
            "use" => {
                d.use_code = part
                    .get("valueCoding")
                    .and_then(|c| c.get("code"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned();
            }
            "value" => d.value = value_obj_str(part, "valueString").to_owned(),
            _ => {}
        }
    }
    if d.value.is_empty() {
        None
    } else {
        Some(d)
    }
}

fn parse_lookup_property(param: &Value) -> Option<LookupProperty> {
    let parts = param.get("part").and_then(|v| v.as_array())?;
    let mut p = LookupProperty::default();
    for part in parts {
        let name = match part.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => continue,
        };
        match name {
            "code" => p.code = value_obj_str(part, "valueCode").to_owned(),
            "value" | "valueString" | "valueCode" | "valueBoolean" | "valueInteger"
            | "valueDecimal" | "valueDateTime" => {
                let candidates = [
                    "valueCode",
                    "valueString",
                    "valueBoolean",
                    "valueInteger",
                    "valueDecimal",
                    "valueDateTime",
                    "valueUri",
                ];
                for k in candidates {
                    if let Some(v) = part.get(k) {
                        p.value = match v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        break;
                    }
                }
            }
            "description" => {
                if p.value.is_empty() {
                    p.value = value_obj_str(part, "valueString").to_owned();
                }
            }
            _ => {}
        }
    }
    if p.code.is_empty() {
        None
    } else {
        Some(p)
    }
}

// ── Slice C: ValueSet browser + detail + expand ─────────────────────────
//
// Types and methods below back the VS browser (§7.4), detail (§7.4), and
// the instance-scoped `$expand` workbench per design doc §7.4. The raw HTS
// body for `$expand` is kept on the result so the workbench's "Raw response"
// panel can echo it verbatim (§7.4 wireframe).

/// Build-time ceiling for the `X-TOO-COSTLY-THRESHOLD` request header the
/// VS expand workbench attaches (design doc §7.6 threshold clause,
/// §7.4.1 F1/F4). HTS does not currently publish `HTS_MAX_EXPANSION_SIZE`
/// on `/metadata`, `/health`, or `/metrics`, so the UI mirrors the operator
/// configuration as a compile-time constant. Values above this ceiling are
/// dropped from the outgoing request and rendered as a warning (§7.4).
pub const HTS_UI_MAX_EXPANSION_SIZE_HINT: u64 = 100_000;

/// Filters accepted by the VS browser page and its rows fragment.
///
/// Same shape as [`CsBrowserFilters`] with the CS-specific decisions
/// intact (design doc §7.4 "same unified browser as §7.2"). The
/// `_count > MAX_COUNT` clamp behaves the same way: the handler renders
/// an invalid-input `OperationOutcome` above an empty table instead of a
/// hard 400.
#[derive(Clone, Debug, Default)]
pub struct VsBrowserFilters {
    pub url: Option<String>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub count: u32,
    pub offset: u32,
}

impl VsBrowserFilters {
    pub const MAX_COUNT: u32 = 100;
    pub const DEFAULT_COUNT: u32 = 25;

    pub fn effective_count(&self) -> u32 {
        if self.count == 0 {
            Self::DEFAULT_COUNT
        } else {
            self.count.clamp(1, Self::MAX_COUNT)
        }
    }

    pub fn count_exceeds_cap(&self) -> bool {
        self.count > Self::MAX_COUNT
    }
}

/// One row of the VS browser table (§7.4 mirror of §7.2 wireframe).
#[derive(Clone, Debug, Default)]
pub struct VsBrowserRow {
    pub id: String,
    pub url: String,
    pub version: String,
    pub name: String,
    pub title: String,
    pub status: String,
}

impl VsBrowserRow {
    fn from_resource(resource: &Value) -> Self {
        let s = |k: &str| {
            resource
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned()
        };
        Self {
            id: s("id"),
            url: s("url"),
            version: s("version"),
            name: s("name"),
            title: s("title"),
            status: s("status"),
        }
    }
}

/// The browser rows partial's data. `filters` is echoed so the page can
/// preserve them in the pager and reset form state (§7.4).
#[derive(Clone, Debug)]
pub struct VsBrowserPage {
    pub rows: Vec<VsBrowserRow>,
    pub filters: VsBrowserFilters,
}

impl VsBrowserPage {
    /// Terminal-page heuristic mirroring [`CsBrowserPage::next_offset`]:
    /// HTS's `Bundle.total` is a page count for the browser search (see
    /// hts-details.md §Search + §7.3.1), so the pager keys off
    /// length-of-rows and stops when HTS returned fewer than requested.
    pub fn next_offset(&self) -> Option<u32> {
        let requested = self.filters.effective_count();
        if (self.rows.len() as u32) >= requested {
            Some(self.filters.offset.saturating_add(self.rows.len() as u32))
        } else {
            None
        }
    }

    pub fn showing_count(&self) -> usize {
        self.rows.len()
    }
}

/// Metadata projection consumed by the VS detail page (§7.4 wireframe).
#[derive(Clone, Debug, Default)]
pub struct ValueSetSummary {
    pub id: String,
    pub url: String,
    pub version: String,
    pub name: String,
    pub title: String,
    pub status: String,
    pub description: String,
    pub immutable: Option<bool>,
    pub publisher: String,
    pub jurisdiction: Vec<String>,
    pub purpose: String,
    pub copyright: String,
    pub raw_body: String,
}

impl ValueSetSummary {
    fn from_resource(resource: &Value, raw_body: String) -> Self {
        let s = |k: &str| {
            resource
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned()
        };
        let jurisdiction = resource
            .get("jurisdiction")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|cc| {
                        cc.get("text")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned)
                            .or_else(|| {
                                cc.get("coding")
                                    .and_then(|v| v.as_array())
                                    .and_then(|codings| codings.first())
                                    .and_then(|c| {
                                        c.get("display")
                                            .and_then(|v| v.as_str())
                                            .map(str::to_owned)
                                    })
                            })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            id: s("id"),
            url: s("url"),
            version: s("version"),
            name: s("name"),
            title: s("title"),
            status: s("status"),
            description: s("description"),
            immutable: resource.get("immutable").and_then(|v| v.as_bool()),
            publisher: s("publisher"),
            jurisdiction,
            purpose: s("purpose"),
            copyright: s("copyright"),
            raw_body,
        }
    }

    /// Heading fallback: title → name → id, matching
    /// [`CodeSystemSummary::heading`].
    pub fn heading(&self) -> &str {
        if !self.title.is_empty() {
            &self.title
        } else if !self.name.is_empty() {
            &self.name
        } else {
            &self.id
        }
    }
}

/// Input to `POST /ValueSet[/{id}]/$expand` — the 14-of-15 inline params
/// (design doc §7.4 field-set clause, §7.4.1 F5). Missing from Slice C:
/// `designation[]` (F2 — chip multi-select ships with Slice E) and the
/// canonical/inline `valueSet` source selector (F8 — Slice C is
/// instance-only).
#[derive(Clone, Debug, Default)]
pub struct ExpandParams {
    pub filter: Option<String>,
    pub count: Option<u32>,
    pub offset: Option<u32>,
    pub display_language: Option<String>,
    pub active_only: Option<bool>,
    pub include_designations: Option<bool>,
    pub use_supplement: Vec<String>,
    pub date: Option<String>,
    pub property: Vec<String>,
    pub tx_resource: Vec<String>,
    pub system_version: Vec<String>,
    pub check_system_version: Vec<String>,
    pub force_system_version: Vec<String>,
    pub default_valueset_version: Option<String>,
    /// Tree mode: `hierarchical=true`. NEVER set together with
    /// `exclude_nested` (§7.4.1 F7).
    pub hierarchical: Option<bool>,
    /// Flat mode: `excludeNested=true`. NEVER set together with
    /// `hierarchical` (§7.4.1 F7).
    pub exclude_nested: Option<bool>,
    /// Numeric ceiling attached as `X-TOO-COSTLY-THRESHOLD` when
    /// `Some(n) && n <= HTS_UI_MAX_EXPANSION_SIZE_HINT`. Values above the
    /// build-time ceiling are surfaced as a warning by the caller and
    /// dropped before sending (§7.4 / §7.4.1 F1 / F4).
    pub threshold: Option<u64>,
}

/// A designation of a concept in an expansion. Kept as a flat trio so the
/// template does not have to traverse nested Fluent values.
#[derive(Clone, Debug, Default)]
pub struct ExpansionDesignation {
    pub language: String,
    pub use_code: String,
    pub value: String,
}

/// One node in an expansion tree (or a single row in a flat expansion —
/// `children` will be empty in that case).
#[derive(Clone, Debug, Default)]
pub struct ExpansionConcept {
    pub code: String,
    pub system: String,
    pub version: String,
    pub display: String,
    pub inactive: Option<bool>,
    pub designations: Vec<ExpansionDesignation>,
    pub children: Vec<ExpansionConcept>,
}

/// Parsed shape of `ValueSet.expansion` used by the expand workbench
/// result partial (§7.4 wireframe). Any parameter set surfaces on
/// `echoed_parameters` verbatim so the operator can audit what HTS
/// actually saw.
#[derive(Clone, Debug, Default)]
pub struct ExpansionResult {
    pub total: Option<u64>,
    pub offset: Option<u64>,
    pub timestamp: String,
    pub identifier: String,
    pub contains: Vec<ExpansionConcept>,
    /// Auto-detected from the response shape: `true` when any node
    /// carries nested `contains` children (§7.4.1 F7 / F10). Tree mode
    /// hides the pager and renders `showing full tree {N}`.
    pub is_tree: bool,
    /// Every `(name, valueString)` from `expansion.parameter[]`,
    /// preserved in wire order so the raw echo panel can display them.
    pub echoed_parameters: Vec<(String, String)>,
    pub request_url: String,
    pub raw_body: String,
    /// The `filter` value the user submitted, echoed so the "no filter
    /// match" neutral-state message can reference it. Empty when no
    /// filter was applied.
    pub requested_filter: String,
    /// The `count` value the user submitted (defaulted to the browser's
    /// paging default when None) — used by the flat-mode pager to advance
    /// `offset`.
    pub requested_count: u32,
    /// The `offset` value the user submitted — used by the flat-mode
    /// pager to advance to the next window.
    pub requested_offset: u32,
    /// The `threshold` value the user submitted. Echoed back into the
    /// form input so the value survives a re-submit (per-request store
    /// per §7.4.1 F1/F4).
    pub requested_threshold: Option<u64>,
    /// Set to `Some(requested)` when the requested threshold was above
    /// [`HTS_UI_MAX_EXPANSION_SIZE_HINT`] and therefore not attached as a
    /// header on the outgoing request. The template renders a warning
    /// with the ceiling exposed.
    pub ceiling_warning: Option<u64>,
}

/// One row of a flattened expansion tree (§7.4.1 F10). `depth` counts
/// from `0` at the root, so the template can indent with a simple
/// `padding-inline-start: {depth}rem` style without recursion.
#[derive(Clone, Debug, Default)]
pub struct FlatTreeRow {
    pub depth: usize,
    pub code: String,
    pub system: String,
    pub display: String,
    pub has_children: bool,
}

impl ExpansionResult {
    /// Flatten the tree into `(depth, ExpansionConcept)` rows so the
    /// template can render tree mode with a single loop. Askama does not
    /// support recursive includes for arbitrarily-deep hierarchies —
    /// the derive macro expands templates at compile time and reaches
    /// its stack limit on self-including partials (§7.4.1 F10).
    pub fn flat_tree_rows(&self) -> Vec<FlatTreeRow> {
        let mut out = Vec::new();
        fn walk(nodes: &[ExpansionConcept], depth: usize, out: &mut Vec<FlatTreeRow>) {
            for n in nodes {
                out.push(FlatTreeRow {
                    depth,
                    code: n.code.clone(),
                    system: n.system.clone(),
                    display: n.display.clone(),
                    has_children: !n.children.is_empty(),
                });
                walk(&n.children, depth + 1, out);
            }
        }
        walk(&self.contains, 0, &mut out);
        out
    }

    /// Flat-mode pager (§7.4 / §7.4.1 F6): remaining rows after this
    /// window based on `expansion.total`. Returns `None` when
    /// `expansion.total` is missing — the caller then falls back to the
    /// terminal-page heuristic (`contains.len() < requested_count`).
    pub fn remaining(&self) -> Option<u64> {
        let total = self.total?;
        let offset = self.offset.unwrap_or(self.requested_offset as u64);
        let shown = self.contains.len() as u64;
        Some(total.saturating_sub(offset).saturating_sub(shown))
    }

    /// Flat-mode "show Load-more" decision (§7.4.1 F6). Tree mode calls
    /// [`Self::is_tree`] first and hides the pager entirely; this
    /// helper only fires when tree mode is off.
    pub fn has_more_flat(&self) -> bool {
        if self.is_tree {
            return false;
        }
        match self.remaining() {
            Some(n) => n > 0,
            None => {
                // Fallback: terminal-page heuristic.
                self.requested_count > 0
                    && (self.contains.len() as u32) >= self.requested_count
            }
        }
    }

    /// Total leaves in the expansion — used by tree mode's
    /// `showing full tree {N}` label (§7.4.1 F10).
    pub fn total_leaves(&self) -> usize {
        fn walk(nodes: &[ExpansionConcept]) -> usize {
            let mut acc = 0usize;
            for n in nodes {
                if n.children.is_empty() {
                    acc += 1;
                } else {
                    acc += walk(&n.children);
                }
            }
            acc
        }
        walk(&self.contains)
    }

    /// Next-offset for the flat-mode `Load more` link. Sums
    /// `requested_offset + contains.len()` — same pager arithmetic
    /// [`VsBrowserPage::next_offset`] uses.
    pub fn next_offset(&self) -> u32 {
        self.requested_offset
            .saturating_add(self.contains.len() as u32)
    }
}

impl UpstreamClient {
    /// `GET /ValueSet?...` — the VS browser rows fragment source.
    ///
    /// Same page-shape rules as [`UpstreamClient::search_code_systems`]:
    /// only the row projection's fields are extracted and the Bundle's
    /// `total` field is deliberately not consumed (see hts-details.md
    /// §Search).
    pub async fn search_value_sets(
        &self,
        filters: &VsBrowserFilters,
    ) -> Result<VsBrowserPage, UpstreamError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(ref v) = filters.url {
            if !v.is_empty() {
                query.push(("url", v.clone()));
            }
        }
        if let Some(ref v) = filters.version {
            if !v.is_empty() {
                query.push(("version", v.clone()));
            }
        }
        if let Some(ref v) = filters.name {
            if !v.is_empty() {
                query.push(("name", v.clone()));
            }
        }
        if let Some(ref v) = filters.title {
            if !v.is_empty() {
                query.push(("title", v.clone()));
            }
        }
        if let Some(ref v) = filters.status {
            if !v.is_empty() {
                query.push(("status", v.clone()));
            }
        }
        query.push(("_count", filters.effective_count().to_string()));
        query.push(("_offset", filters.offset.to_string()));

        let url = format!("{}/ValueSet", self.base_url);
        let response = self
            .client
            .get(&url)
            .query(&query)
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest("vs-search", &url, e))?;
        let status = response.status();
        let body: Value = response.json().await.map_err(|e| UpstreamError::Decode {
            op: "vs-search",
            url: url.clone(),
            message: e.to_string(),
        })?;
        if !status.is_success() {
            return Err(status_to_error("vs-search", &url, status.as_u16(), &body));
        }
        let rows: Vec<VsBrowserRow> = body
            .get("entry")
            .and_then(|v| v.as_array())
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|e| e.get("resource"))
                    .filter(|r| {
                        r.get("resourceType")
                            .and_then(|t| t.as_str())
                            .map(|t| t == "ValueSet")
                            .unwrap_or(true)
                    })
                    .map(VsBrowserRow::from_resource)
                    .collect()
            })
            .unwrap_or_default();
        Ok(VsBrowserPage {
            rows,
            filters: filters.clone(),
        })
    }

    /// `GET /ValueSet/{id}` — VS detail page source.
    ///
    /// 404 → [`UpstreamError::NotFound`] so the detail handler can render
    /// an explanatory OperationOutcome partial inside the page shell
    /// (design doc §7.4 states matrix, §7.4.1 invariant #5).
    pub async fn read_value_set(&self, id: &str) -> Result<ValueSetSummary, UpstreamError> {
        let url = format!("{}/ValueSet/{}", self.base_url, id);
        let response = self
            .client
            .get(&url)
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest("vs-read", &url, e))?;
        let status = response.status();
        let raw = response.text().await.map_err(|e| UpstreamError::Decode {
            op: "vs-read",
            url: url.clone(),
            message: e.to_string(),
        })?;
        if !status.is_success() {
            if status.as_u16() == 404 {
                return Err(UpstreamError::NotFound {
                    op: "vs-read",
                    url,
                });
            }
            let parsed: Value = serde_json::from_str(&raw).unwrap_or_default();
            return Err(status_to_error("vs-read", &url, status.as_u16(), &parsed));
        }
        let parsed: Value = serde_json::from_str(&raw).map_err(|e| UpstreamError::Decode {
            op: "vs-read",
            url: url.clone(),
            message: e.to_string(),
        })?;
        let pretty = serde_json::to_string_pretty(&parsed).unwrap_or(raw);
        Ok(ValueSetSummary::from_resource(&parsed, pretty))
    }

    /// `POST /ValueSet/{id}/$expand` (design doc §7.4 wireframe, §7.6
    /// proxy verb rule). Slice C is instance-only per §7.4.1 F8.
    ///
    /// Header contract: attaches `X-TOO-COSTLY-THRESHOLD: {n}` iff
    /// `params.threshold` is `Some(n)` and `n <=
    /// HTS_UI_MAX_EXPANSION_SIZE_HINT` (§7.4 threshold clause). Above
    /// the ceiling the header is dropped and `ceiling_warning` on the
    /// returned [`ExpansionResult`] carries the requested value so the
    /// template can render its warning + ceiling tooltip.
    ///
    /// Tree/flat mapping (§7.4.1 F7): emits `hierarchical=true` when
    /// `params.hierarchical == Some(true)`; emits `excludeNested=true`
    /// when `params.exclude_nested == Some(true)`. Never both — the
    /// caller enforces the invariant and the ring asserts it.
    pub async fn vs_expand_instance(
        &self,
        id: &str,
        params: &ExpandParams,
    ) -> Result<ExpansionResult, UpstreamError> {
        let mut parameters: Vec<Value> = Vec::new();
        if let Some(v) = params.filter.as_deref().and_then(non_empty_str) {
            parameters.push(json!({"name": "filter", "valueString": v}));
        }
        if let Some(c) = params.count {
            parameters.push(json!({"name": "count", "valueInteger": c}));
        }
        if let Some(o) = params.offset {
            parameters.push(json!({"name": "offset", "valueInteger": o}));
        }
        if let Some(v) = params.display_language.as_deref().and_then(non_empty_str) {
            parameters.push(json!({"name": "displayLanguage", "valueCode": v}));
        }
        if let Some(true) = params.active_only {
            parameters.push(json!({"name": "activeOnly", "valueBoolean": true}));
        }
        if let Some(true) = params.include_designations {
            parameters.push(json!({"name": "includeDesignations", "valueBoolean": true}));
        }
        for v in params.use_supplement.iter().filter_map(|s| non_empty_str(s)) {
            parameters.push(json!({"name": "useSupplement", "valueCanonical": v}));
        }
        if let Some(v) = params.date.as_deref().and_then(non_empty_str) {
            parameters.push(json!({"name": "date", "valueDateTime": v}));
        }
        for v in params.property.iter().filter_map(|s| non_empty_str(s)) {
            parameters.push(json!({"name": "property", "valueString": v}));
        }
        for v in params.tx_resource.iter().filter_map(|s| non_empty_str(s)) {
            // The valueString path is a conservative default; HTS accepts
            // a resource embed in POST but the workbench form only lets
            // the operator pin references, matching §7.4 wireframe.
            parameters.push(json!({"name": "tx-resource", "valueString": v}));
        }
        for v in params.system_version.iter().filter_map(|s| non_empty_str(s)) {
            parameters.push(json!({"name": "system-version", "valueCanonical": v}));
        }
        for v in params
            .check_system_version
            .iter()
            .filter_map(|s| non_empty_str(s))
        {
            parameters.push(json!({"name": "check-system-version", "valueCanonical": v}));
        }
        for v in params
            .force_system_version
            .iter()
            .filter_map(|s| non_empty_str(s))
        {
            parameters.push(json!({"name": "force-system-version", "valueCanonical": v}));
        }
        if let Some(v) = params
            .default_valueset_version
            .as_deref()
            .and_then(non_empty_str)
        {
            parameters.push(
                json!({"name": "default-valueset-version", "valueCanonical": v}),
            );
        }
        // §7.4.1 F7: `hierarchical` and `excludeNested` are mutually
        // exclusive — emit one, never both. If both are set upstream of
        // us (a bug in the caller) hierarchical wins arbitrarily; the
        // integration ring asserts the caller upholds the invariant.
        if let Some(true) = params.hierarchical {
            parameters.push(json!({"name": "hierarchical", "valueBoolean": true}));
        } else if let Some(true) = params.exclude_nested {
            parameters.push(json!({"name": "excludeNested", "valueBoolean": true}));
        }

        // Ceiling test for the threshold header (§7.4 / §7.4.1 F1/F4).
        let (attach_threshold, ceiling_warning) = match params.threshold {
            Some(n) if n <= HTS_UI_MAX_EXPANSION_SIZE_HINT => (Some(n), None),
            Some(n) => (None, Some(n)),
            None => (None, None),
        };

        let body = json!({
            "resourceType": "Parameters",
            "parameter": parameters,
        });
        let url = format!("{}/ValueSet/{}/$expand", self.base_url, id);
        let (raw, parsed) = self
            .post_parameters_with_headers(
                "vs-expand",
                &url,
                &body,
                attach_threshold.map(|n| ("X-TOO-COSTLY-THRESHOLD".to_owned(), n.to_string())),
            )
            .await?;

        let expansion = parsed.get("expansion").cloned().unwrap_or(Value::Null);
        let contains = parse_expansion_contains(expansion.get("contains"));
        let is_tree = contains_has_children(&contains);
        let echoed_parameters = parse_expansion_parameters(expansion.get("parameter"));
        let total = expansion
            .get("total")
            .and_then(|v| v.as_u64());
        let offset = expansion
            .get("offset")
            .and_then(|v| v.as_u64());
        let timestamp = expansion
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();
        let identifier = expansion
            .get("identifier")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();

        Ok(ExpansionResult {
            total,
            offset,
            timestamp,
            identifier,
            contains,
            is_tree,
            echoed_parameters,
            request_url: url,
            raw_body: raw,
            requested_filter: params.filter.clone().unwrap_or_default(),
            requested_count: params.count.unwrap_or(0),
            requested_offset: params.offset.unwrap_or(0),
            requested_threshold: params.threshold,
            ceiling_warning,
        })
    }

    /// Variant of [`Self::post_parameters`] that also accepts an
    /// optional extra header (used for `X-TOO-COSTLY-THRESHOLD`). Kept
    /// distinct from `post_parameters` so the CS operation methods stay
    /// header-free.
    async fn post_parameters_with_headers(
        &self,
        op: &'static str,
        url: &str,
        body: &Value,
        extra_header: Option<(String, String)>,
    ) -> Result<(String, Value), UpstreamError> {
        let mut request = self
            .client
            .post(url)
            .header("Accept", "application/fhir+json")
            .header("Content-Type", "application/fhir+json")
            .json(body);
        if let Some((name, value)) = extra_header {
            request = request.header(name, value);
        }
        let response = request
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest(op, url, e))?;
        let status = response.status();
        let raw = response.text().await.map_err(|e| UpstreamError::Decode {
            op,
            url: url.to_owned(),
            message: e.to_string(),
        })?;
        let parsed: Value = serde_json::from_str(&raw).map_err(|e| UpstreamError::Decode {
            op,
            url: url.to_owned(),
            message: e.to_string(),
        })?;
        if !status.is_success() {
            return Err(status_to_error(op, url, status.as_u16(), &parsed));
        }
        let pretty = serde_json::to_string_pretty(&parsed).unwrap_or(raw);
        Ok((pretty, parsed))
    }
}

fn non_empty_str(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_owned())
    }
}

fn parse_expansion_contains(node: Option<&Value>) -> Vec<ExpansionConcept> {
    let arr = match node.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter().map(parse_expansion_concept).collect()
}

fn parse_expansion_concept(item: &Value) -> ExpansionConcept {
    let s = |k: &str| {
        item.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    let designations = item
        .get("designation")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|d| ExpansionDesignation {
                    language: d
                        .get("language")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned(),
                    use_code: d
                        .get("use")
                        .and_then(|u| u.get("code"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned(),
                    value: d
                        .get("value")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();
    let children = parse_expansion_contains(item.get("contains"));
    ExpansionConcept {
        code: s("code"),
        system: s("system"),
        version: s("version"),
        display: s("display"),
        inactive: item.get("inactive").and_then(|v| v.as_bool()),
        designations,
        children,
    }
}

fn contains_has_children(nodes: &[ExpansionConcept]) -> bool {
    nodes.iter().any(|n| !n.children.is_empty())
}

fn parse_expansion_parameters(node: Option<&Value>) -> Vec<(String, String)> {
    let arr = match node.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let name = match entry.get("name").and_then(|v| v.as_str()) {
            Some(n) => n.to_owned(),
            None => continue,
        };
        let value = entry
            .as_object()
            .and_then(|o| {
                o.iter().find_map(|(k, v)| {
                    if !k.starts_with("value") {
                        return None;
                    }
                    Some(match v {
                        Value::String(s) => s.clone(),
                        Value::Bool(b) => b.to_string(),
                        Value::Number(n) => n.to_string(),
                        other => other.to_string(),
                    })
                })
            })
            .unwrap_or_default();
        out.push((name, value));
    }
    out
}

/// Map a non-success HTTP status to a typed [`UpstreamError`] variant,
/// preferring the OperationOutcome arm when the body parses as one so the
/// UI can render its structured error partial.
fn status_to_error(op: &'static str, url: &str, status: u16, body: &Value) -> UpstreamError {
    let is_outcome = body
        .get("resourceType")
        .and_then(|v| v.as_str())
        .map(|t| t == "OperationOutcome")
        .unwrap_or(false);
    if is_outcome {
        UpstreamError::Outcome {
            op,
            url: url.to_owned(),
            status,
            outcome: OutcomeView::from_body(body),
        }
    } else {
        UpstreamError::HttpStatus {
            op,
            url: url.to_owned(),
            status,
        }
    }
}

// ── Slice D: ConceptMap browser + detail + translate ────────────────────
//
// Types and methods below back the CM browser (§7.5), detail (§7.5), and
// the instance-scoped `$translate` workbench per design doc §7.5. The raw
// HTS body for `$translate` is kept on the result so the workbench's "Raw
// response" panel can echo it verbatim (§7.5 wireframe).
//
// Unsupported inputs per §7.5 and hts-details.md `$translate`:
//   - `version` (of the ConceptMap resource) — HTS ignores it
//   - `dependency` — HTS does not implement it
//   - `targetsystem` (lowercase alias) — HTS does not accept it; only the
//     camelCase `targetSystem` reaches the wire.
// The form templates omit these fields entirely and the tests grep the
// rendered HTML to confirm they never leak.

/// Filters accepted by the CM browser page and its rows fragment.
///
/// CM search accepts `source-uri` / `target-uri` per FHIR spec plus the
/// per-resource facets shared with CS / VS. The filter strip exposes the
/// URL-scoped side (`source` / `target`), skipping `source-code` /
/// `target-code` which are per-mapping searches not per-CM facets.
/// `version` of the ConceptMap resource is deliberately absent — HTS
/// ignores it in `$translate` (hts-details.md §`$translate`) and the
/// browser does not filter by it either, keeping the filter strip aligned
/// with the operator-usable knobs.
#[derive(Clone, Debug, Default)]
pub struct CmBrowserFilters {
    pub url: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub source: Option<String>,
    pub target: Option<String>,
    pub status: Option<String>,
    pub count: u32,
    pub offset: u32,
}

impl CmBrowserFilters {
    pub const MAX_COUNT: u32 = 100;
    pub const DEFAULT_COUNT: u32 = 25;

    pub fn effective_count(&self) -> u32 {
        if self.count == 0 {
            Self::DEFAULT_COUNT
        } else {
            self.count.clamp(1, Self::MAX_COUNT)
        }
    }

    pub fn count_exceeds_cap(&self) -> bool {
        self.count > Self::MAX_COUNT
    }
}

/// One row of the CM browser table (§7.5 wireframe row shape).
///
/// Slice D renders url + version + title + status, matching the shared
/// row shape used by CS / VS. Source / target URIs are stored so the
/// template can surface them in a compact "maps X → Y" caption without
/// clicking through to detail.
#[derive(Clone, Debug, Default)]
pub struct CmBrowserRow {
    pub id: String,
    pub url: String,
    pub version: String,
    pub name: String,
    pub title: String,
    pub status: String,
    pub source_uri: String,
    pub target_uri: String,
}

impl CmBrowserRow {
    fn from_resource(resource: &Value) -> Self {
        let s = |k: &str| {
            resource
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned()
        };
        // R4/R4B use `sourceUri` / `sourceCanonical` / `sourceReference`.
        // R5/R6 use `sourceScopeUri` / `sourceScopeCanonical`. The row
        // renders whichever the server chose to emit; empty falls through
        // to an em-dash in the template.
        let source_uri = pick_first_str(
            resource,
            &["sourceUri", "sourceCanonical", "sourceScopeUri", "sourceScopeCanonical"],
        );
        let target_uri = pick_first_str(
            resource,
            &["targetUri", "targetCanonical", "targetScopeUri", "targetScopeCanonical"],
        );
        Self {
            id: s("id"),
            url: s("url"),
            version: s("version"),
            name: s("name"),
            title: s("title"),
            status: s("status"),
            source_uri,
            target_uri,
        }
    }
}

/// The browser rows partial's data (§7.5). Same terminal-page heuristic
/// as CS / VS since HTS's `Bundle.total` for search is a page count, not
/// an authoritative match count.
#[derive(Clone, Debug)]
pub struct CmBrowserPage {
    pub rows: Vec<CmBrowserRow>,
    pub filters: CmBrowserFilters,
}

impl CmBrowserPage {
    pub fn next_offset(&self) -> Option<u32> {
        let requested = self.filters.effective_count();
        if (self.rows.len() as u32) >= requested {
            Some(self.filters.offset.saturating_add(self.rows.len() as u32))
        } else {
            None
        }
    }

    pub fn showing_count(&self) -> usize {
        self.rows.len()
    }
}

/// Metadata projection consumed by the CM detail page (§7.5 wireframe).
///
/// The source / target sides carry both `*Uri` and `*Canonical` slots so
/// R4/R4B and R5/R6 responses render legibly without cfg branching. HTS
/// does not currently emit `group[].source`/`target` per-group counts on
/// `_summary=true` reads, so `group_count` may be `None` even when the
/// resource has groups; the template falls back to an em-dash in that
/// case.
#[derive(Clone, Debug, Default)]
pub struct ConceptMapSummary {
    pub id: String,
    pub url: String,
    pub version: String,
    pub name: String,
    pub title: String,
    pub status: String,
    pub description: String,
    pub publisher: String,
    pub jurisdiction: Vec<String>,
    pub purpose: String,
    pub source_uri: String,
    pub source_canonical: String,
    pub target_uri: String,
    pub target_canonical: String,
    pub group_count: Option<usize>,
    pub raw_body: String,
}

impl ConceptMapSummary {
    fn from_resource(resource: &Value, raw_body: String) -> Self {
        let s = |k: &str| {
            resource
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned()
        };
        let jurisdiction = resource
            .get("jurisdiction")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|cc| {
                        cc.get("text")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned)
                            .or_else(|| {
                                cc.get("coding")
                                    .and_then(|v| v.as_array())
                                    .and_then(|codings| codings.first())
                                    .and_then(|c| {
                                        c.get("display")
                                            .and_then(|v| v.as_str())
                                            .map(str::to_owned)
                                    })
                            })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let group_count = resource
            .get("group")
            .and_then(|v| v.as_array())
            .map(|a| a.len());
        // Prefer R4/R4B naming (sourceUri / sourceCanonical); fall back to
        // R5/R6 (`sourceScopeUri` / `sourceScopeCanonical`). The template
        // renders whichever slot is populated.
        let source_uri = pick_first_str(resource, &["sourceUri", "sourceScopeUri"]);
        let source_canonical =
            pick_first_str(resource, &["sourceCanonical", "sourceScopeCanonical"]);
        let target_uri = pick_first_str(resource, &["targetUri", "targetScopeUri"]);
        let target_canonical =
            pick_first_str(resource, &["targetCanonical", "targetScopeCanonical"]);
        Self {
            id: s("id"),
            url: s("url"),
            version: s("version"),
            name: s("name"),
            title: s("title"),
            status: s("status"),
            description: s("description"),
            publisher: s("publisher"),
            jurisdiction,
            purpose: s("purpose"),
            source_uri,
            source_canonical,
            target_uri,
            target_canonical,
            group_count,
            raw_body,
        }
    }

    /// Heading fallback: title → name → id (mirrors CS / VS heading).
    pub fn heading(&self) -> &str {
        if !self.title.is_empty() {
            &self.title
        } else if !self.name.is_empty() {
            &self.name
        } else {
            &self.id
        }
    }
}

/// Direction the operator submitted on the Translate tab. Governs which
/// FHIR parameters the wire body carries and whether `reverse=true` is
/// attached (hts-details.md §`$translate`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranslateDirection {
    Forward,
    Reverse,
}

impl Default for TranslateDirection {
    fn default() -> Self {
        Self::Forward
    }
}

impl TranslateDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Reverse => "reverse",
        }
    }

    /// Parse a form-submitted string; unknown values default to `Forward`
    /// so a stale bookmark still lands the operator in a usable form.
    pub fn from_form(s: Option<&str>) -> Self {
        match s.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("reverse") => Self::Reverse,
            _ => Self::Forward,
        }
    }
}

/// Input to `POST /ConceptMap[/{id}]/$translate` (design doc §7.5 wireframe,
/// §7.6 proxy verb rule).
///
/// The struct carries both forward and reverse fields so the caller can
/// hand the whole form parse in without pre-branching — the emitter picks
/// only the direction-appropriate slots. Version-of-CM, `dependency`, and
/// lowercase `targetsystem` are deliberately absent per §7.5.
#[derive(Clone, Debug, Default)]
pub struct TranslateParams {
    pub direction: TranslateDirection,
    /// Forward-mode: source `code` (valueCode).
    pub code: Option<String>,
    /// Forward-mode: source `system` (valueUri).
    pub system: Option<String>,
    /// Forward-mode: optional `display` (valueString).
    pub display: Option<String>,
    /// Reverse-mode: `targetCode` (valueCode).
    pub target_code: Option<String>,
    /// Both directions: opposite-side system filter (valueUri).
    pub target_system: Option<String>,
    /// Both directions: canonical URL of the source ValueSet (valueUri).
    pub source_url: Option<String>,
    /// Both directions: canonical URL of the target ValueSet (valueUri).
    pub target_url: Option<String>,
    /// Both directions: optional ISO date/time filter (valueDateTime).
    pub date: Option<String>,
}

/// Which mapping-kind field the HTS response carried. R4/R4B emit
/// `equivalence`; R5/R6 emit `relationship`. The Slice D UI reads whichever
/// showed up rather than picking off a compile-time cfg (§7.5) — a Rust
/// build compiled for R5 can still be pointed at an R4 HTS via
/// `HTS_UI_UPSTREAM_URL`, so the field name is not knowable at compile
/// time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MappingKind {
    Equivalence,
    Relationship,
    Unknown,
}

impl Default for MappingKind {
    fn default() -> Self {
        Self::Unknown
    }
}

impl MappingKind {
    /// Lowercase key rendered into `hts-cm-translate-column-mapping = { $kind }`.
    /// The Fluent catalog selects a translated label off of this value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Equivalence => "equivalence",
            Self::Relationship => "relationship",
            Self::Unknown => "unknown",
        }
    }
}

/// One row of the Translate match grid (§7.5 wireframe columns).
///
/// The `mapping_value` string is the concrete `equivalence` / `relationship`
/// value HTS returned (e.g. `equivalent`, `wider`, `related-to`, ...) —
/// the template renders it as a compact code chip. `origin` is the
/// `originMap` URI in forward mode and the `source` URI in reverse mode.
#[derive(Clone, Debug, Default)]
pub struct TranslateMatch {
    pub code: String,
    pub system: String,
    pub display: Option<String>,
    pub mapping_value: Option<String>,
    pub origin: Option<String>,
}

/// Parsed shape of the `Parameters` response (§7.5). `raw_body` is the
/// pretty-printed HTS JSON so the workbench can echo it in its "Raw
/// response" panel.
#[derive(Clone, Debug, Default)]
pub struct TranslateResult {
    pub result: bool,
    pub message: String,
    pub matches: Vec<TranslateMatch>,
    pub mapping_kind: MappingKind,
    pub raw_body: String,
    pub request_url: String,
}

impl UpstreamClient {
    /// `GET /ConceptMap?...` — the CM browser rows fragment source.
    ///
    /// Emits `source-uri` / `target-uri` search params for the URL-scoped
    /// filter chips. Per §7.5 the browser does not expose `source-code` /
    /// `target-code` (those are per-mapping searches and would leak the
    /// browser's shape). `Bundle.total` is a page count for CM search
    /// (hts-details.md §Search) so the pager keys off `rows.len()` here
    /// the same way CS / VS do.
    pub async fn search_concept_maps(
        &self,
        filters: &CmBrowserFilters,
    ) -> Result<CmBrowserPage, UpstreamError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(ref v) = filters.url {
            if !v.is_empty() {
                query.push(("url", v.clone()));
            }
        }
        if let Some(ref v) = filters.name {
            if !v.is_empty() {
                query.push(("name", v.clone()));
            }
        }
        if let Some(ref v) = filters.title {
            if !v.is_empty() {
                query.push(("title", v.clone()));
            }
        }
        if let Some(ref v) = filters.source {
            if !v.is_empty() {
                query.push(("source-uri", v.clone()));
            }
        }
        if let Some(ref v) = filters.target {
            if !v.is_empty() {
                query.push(("target-uri", v.clone()));
            }
        }
        if let Some(ref v) = filters.status {
            if !v.is_empty() {
                query.push(("status", v.clone()));
            }
        }
        query.push(("_count", filters.effective_count().to_string()));
        query.push(("_offset", filters.offset.to_string()));

        let url = format!("{}/ConceptMap", self.base_url);
        let response = self
            .client
            .get(&url)
            .query(&query)
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest("cm-search", &url, e))?;
        let status = response.status();
        let body: Value = response.json().await.map_err(|e| UpstreamError::Decode {
            op: "cm-search",
            url: url.clone(),
            message: e.to_string(),
        })?;
        if !status.is_success() {
            return Err(status_to_error("cm-search", &url, status.as_u16(), &body));
        }
        let rows: Vec<CmBrowserRow> = body
            .get("entry")
            .and_then(|v| v.as_array())
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|e| e.get("resource"))
                    .filter(|r| {
                        r.get("resourceType")
                            .and_then(|t| t.as_str())
                            .map(|t| t == "ConceptMap")
                            .unwrap_or(true)
                    })
                    .map(CmBrowserRow::from_resource)
                    .collect()
            })
            .unwrap_or_default();
        Ok(CmBrowserPage {
            rows,
            filters: filters.clone(),
        })
    }

    /// `GET /ConceptMap/{id}` — CM detail page source.
    ///
    /// 404 → [`UpstreamError::NotFound`] so the detail handler renders an
    /// explanatory OperationOutcome partial inside the page shell (§7.5
    /// states matrix, mirroring §7.3.1 / §7.4.1 invariant #5).
    pub async fn read_concept_map(&self, id: &str) -> Result<ConceptMapSummary, UpstreamError> {
        let url = format!("{}/ConceptMap/{}", self.base_url, id);
        let response = self
            .client
            .get(&url)
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest("cm-read", &url, e))?;
        let status = response.status();
        let raw = response.text().await.map_err(|e| UpstreamError::Decode {
            op: "cm-read",
            url: url.clone(),
            message: e.to_string(),
        })?;
        if !status.is_success() {
            if status.as_u16() == 404 {
                return Err(UpstreamError::NotFound {
                    op: "cm-read",
                    url,
                });
            }
            let parsed: Value = serde_json::from_str(&raw).unwrap_or_default();
            return Err(status_to_error("cm-read", &url, status.as_u16(), &parsed));
        }
        let parsed: Value = serde_json::from_str(&raw).map_err(|e| UpstreamError::Decode {
            op: "cm-read",
            url: url.clone(),
            message: e.to_string(),
        })?;
        let pretty = serde_json::to_string_pretty(&parsed).unwrap_or(raw);
        Ok(ConceptMapSummary::from_resource(&parsed, pretty))
    }

    /// `POST /ConceptMap/{id}/$translate` (design doc §7.5, §7.6 proxy
    /// verb rule).
    ///
    /// Parameter emission:
    /// - Forward: `code` (valueCode), `system` (valueUri), optional
    ///   `display` (valueString).
    /// - Reverse: `targetCode` (valueCode) plus `reverse=true` (valueBoolean)
    ///   per hts-details.md §`$translate` "Explicit reverse mode".
    /// - Both: optional `targetSystem` (valueUri), `source` (valueUri —
    ///   source ValueSet), `target` (valueUri — target ValueSet), `date`
    ///   (valueDateTime).
    ///
    /// Never emits `version` (of the ConceptMap), `dependency`, or
    /// lowercase `targetsystem`. §7.5 explicitly lists these as UI-hidden.
    pub async fn cm_translate_instance(
        &self,
        id: &str,
        params: &TranslateParams,
    ) -> Result<TranslateResult, UpstreamError> {
        let mut parameters: Vec<Value> = Vec::new();
        match params.direction {
            TranslateDirection::Forward => {
                if let Some(v) = params.code.as_deref().and_then(non_empty_str) {
                    parameters.push(json!({"name": "code", "valueCode": v}));
                }
                if let Some(v) = params.system.as_deref().and_then(non_empty_str) {
                    parameters.push(json!({"name": "system", "valueUri": v}));
                }
                if let Some(v) = params.display.as_deref().and_then(non_empty_str) {
                    parameters.push(json!({"name": "display", "valueString": v}));
                }
            }
            TranslateDirection::Reverse => {
                parameters.push(json!({"name": "reverse", "valueBoolean": true}));
                if let Some(v) = params.target_code.as_deref().and_then(non_empty_str) {
                    parameters.push(json!({"name": "targetCode", "valueCode": v}));
                }
            }
        }
        if let Some(v) = params.target_system.as_deref().and_then(non_empty_str) {
            parameters.push(json!({"name": "targetSystem", "valueUri": v}));
        }
        if let Some(v) = params.source_url.as_deref().and_then(non_empty_str) {
            parameters.push(json!({"name": "source", "valueUri": v}));
        }
        if let Some(v) = params.target_url.as_deref().and_then(non_empty_str) {
            parameters.push(json!({"name": "target", "valueUri": v}));
        }
        if let Some(v) = params.date.as_deref().and_then(non_empty_str) {
            parameters.push(json!({"name": "date", "valueDateTime": v}));
        }
        let body = json!({
            "resourceType": "Parameters",
            "parameter": parameters,
        });
        let url = format!("{}/ConceptMap/{}/$translate", self.base_url, id);
        let (raw, parsed) = self.post_parameters("cm-translate", &url, &body).await?;

        let mut out = TranslateResult {
            raw_body: raw,
            request_url: url.clone(),
            ..TranslateResult::default()
        };
        for (name, value_obj) in iter_parameters(&parsed) {
            match name {
                "result" => {
                    out.result = value_obj
                        .get("valueBoolean")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                }
                "message" => {
                    out.message = value_obj_str(value_obj, "valueString").to_owned();
                }
                "match" => {
                    if let Some((mat, kind)) = parse_translate_match(value_obj) {
                        // First match wins for the whole-result mapping
                        // kind. HTS emits either `equivalence` (R4/R4B) or
                        // `relationship` (R5/R6) uniformly across all
                        // matches in one response, so first-wins is safe.
                        if out.mapping_kind == MappingKind::Unknown {
                            out.mapping_kind = kind;
                        }
                        out.matches.push(mat);
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }
}

/// Parse one `parameter[name=match]` group. Returns the parsed match plus
/// the [`MappingKind`] detected from the `part[]` (which of `equivalence`
/// / `relationship` was populated). Returns `None` when the `part` array
/// is absent — the workbench template would have nothing legible to
/// render.
fn parse_translate_match(param: &Value) -> Option<(TranslateMatch, MappingKind)> {
    let parts = param.get("part").and_then(|v| v.as_array())?;
    let mut mat = TranslateMatch::default();
    let mut kind = MappingKind::Unknown;
    for part in parts {
        let name = match part.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => continue,
        };
        match name {
            "equivalence" => {
                let v = value_obj_str(part, "valueCode").to_owned();
                if !v.is_empty() {
                    mat.mapping_value = Some(v);
                }
                kind = MappingKind::Equivalence;
            }
            "relationship" => {
                let v = value_obj_str(part, "valueCode").to_owned();
                if !v.is_empty() {
                    mat.mapping_value = Some(v);
                }
                kind = MappingKind::Relationship;
            }
            "concept" => {
                if let Some(coding) = part.get("valueCoding") {
                    mat.code = coding
                        .get("code")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned();
                    mat.system = coding
                        .get("system")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned();
                    let display = coding
                        .get("display")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                        .filter(|s| !s.is_empty());
                    mat.display = display;
                }
            }
            // Forward mode: HTS attaches an `originMap` URI part.
            "originMap" => {
                let v = value_obj_str(part, "valueUri").to_owned();
                if !v.is_empty() {
                    mat.origin = Some(v);
                }
            }
            // Reverse mode: HTS attaches a `source` URI part.
            "source" => {
                let v = value_obj_str(part, "valueUri").to_owned();
                if !v.is_empty() && mat.origin.is_none() {
                    mat.origin = Some(v);
                }
            }
            _ => {}
        }
    }
    Some((mat, kind))
}

/// Pick the first non-empty string value at any of the given keys — used
/// to collapse the R4/R4B `sourceUri` and R5/R6 `sourceScopeUri` slots
/// into one projection field.
fn pick_first_str(resource: &Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(v) = resource.get(*k).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                return v.to_owned();
            }
        }
    }
    String::new()
}

// ─── Slice E — ConceptMap `$closure` ────────────────────────────────────────

/// Build-time ceiling for the number of concurrent per-row `$validate-code`
/// upstream calls the batch-validate workbench will fan out at any given
/// time (design doc §7.6 batch fan-out F1=D). Enforced via a
/// `tokio::sync::Semaphore` on the per-row handler so the UI cannot
/// stampede HTS on a large row list.
pub const HTS_UI_BATCH_FANOUT_CONCURRENCY: usize = 8;

/// Input to `POST /ConceptMap/$closure` (design doc §7.6 Closure op).
///
/// HTS's `$closure` accepts a `name` (required) plus repeatable `concept`
/// Coding entries. The empty-concept case (just a `name`) is a legal
/// initial-state seed — HTS returns an empty ConceptMap that the UI
/// renders as a neutral state, not an error (§7.6.1 F6/F7 decision).
#[derive(Clone, Debug, Default)]
pub struct ClosureParams {
    pub name: String,
    pub concepts: Vec<ClosureConcept>,
}

/// One `concept` coding row in a `$closure` request.
#[derive(Clone, Debug, Default)]
pub struct ClosureConcept {
    pub system: String,
    pub code: String,
}

/// A single edge in the edge-list projection of a `$closure` response
/// (source → equivalence → target). Kept flat so the template does not
/// have to walk a nested ConceptMap group/element/target tree.
#[derive(Clone, Debug, Default)]
pub struct ClosureEdge {
    pub source_system: String,
    pub source_code: String,
    pub target_system: String,
    pub target_code: String,
    /// The `equivalence` (R4/R4B) or `relationship` (R5/R6) value HTS
    /// returned — rendered as a small badge next to the arrow.
    pub relation: String,
}

/// Response projection for `$closure` (design doc §7.6 F6/F7). The
/// `is_empty_graph` flag is set when the returned ConceptMap has no
/// groups or no elements — the neutral empty-state banner branches on
/// this rather than on the raw JSON.
#[derive(Clone, Debug, Default)]
pub struct ClosureResult {
    pub name: String,
    pub map_url: String,
    pub map_version: String,
    pub edges: Vec<ClosureEdge>,
    pub is_empty_graph: bool,
    pub request_url: String,
    pub raw_body: String,
}

// ─── Slice E — ValueSet `$validate-code` ────────────────────────────────────

/// Which ValueSet-source shape a `$validate-code` call is scoped to
/// (design doc §7.6 VS Validate). Determines the HTS route + which
/// parameter carries the source: `Instance(id)` → instance URL,
/// `Canonical(url)` → `url=`, `Inline(json)` → `valueSet=` (embedded
/// resource).
#[derive(Clone, Debug)]
pub enum VsValidateSource {
    Instance(String),
    Canonical(String),
    Inline(String),
}

impl Default for VsValidateSource {
    fn default() -> Self {
        VsValidateSource::Canonical(String::new())
    }
}

/// Input-mode selector for VS `$validate-code` (design doc §7.6 F4
/// widened surface). `Code` submits a bare `code`+optional `system`;
/// `Coding` submits a structured Coding; `CodeableConcept` submits a
/// nested Coding array with optional text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VsValidateMode {
    #[default]
    Code,
    Coding,
    CodeableConcept,
}

impl VsValidateMode {
    /// Parse the mode radio value into an enum, defaulting to `Code`
    /// when the form omits `mode` (nojs first submit).
    pub fn from_form_value(s: &str) -> Self {
        match s {
            "coding" => Self::Coding,
            "codeable-concept" | "codeableConcept" => Self::CodeableConcept,
            _ => Self::Code,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Coding => "coding",
            Self::CodeableConcept => "codeable-concept",
        }
    }
}

/// Input to VS `$validate-code` (design doc §7.6 field-set). Mirrors
/// the CS variant but widens to the full HTS parameter matrix
/// (skill §6). Repeatable fields ship as `Vec<String>` and are
/// trimmed of empties at emit time.
#[derive(Clone, Debug, Default)]
pub struct VsValidateParams {
    pub mode: VsValidateMode,
    /// Bare-code mode payload.
    pub code: String,
    pub system: String,
    pub system_version: Option<String>,
    /// Coding-mode payload.
    pub coding_system: String,
    pub coding_code: String,
    pub coding_display: String,
    /// CodeableConcept-mode payload — one Coding row per index; text
    /// is optional.
    pub coding_rows: Vec<ClosureConcept>,
    pub codeable_concept_text: Option<String>,
    /// Always-optional fields shared across the three input modes.
    pub display: Option<String>,
    pub display_language: Option<String>,
    pub valueset_version: Option<String>,
    pub date: Option<String>,
    pub active_only: Option<bool>,
    pub abstract_ok: Option<bool>,
    pub lenient_display_validation: Option<bool>,
    pub use_supplement: Vec<String>,
    pub tx_resource: Vec<String>,
    pub system_version_pins: Vec<String>,
    pub check_system_version: Vec<String>,
    pub force_system_version: Vec<String>,
    pub default_valueset_version: Option<String>,
}

/// Response projection for VS `$validate-code`. Same shape as the CS
/// variant — `result=false` on HTTP 200 is the neutral no-membership
/// state, NOT an error (§7.6.1 F11 companion to §7.5 F11).
#[derive(Clone, Debug, Default)]
pub struct VsValidateResult {
    pub result: bool,
    pub code: String,
    pub system: String,
    pub version: String,
    pub display: String,
    pub message: String,
    pub issues: Option<OutcomeView>,
    pub request_url: String,
    pub raw_body: String,
}

impl UpstreamClient {
    /// `POST /ConceptMap/$closure` (design doc §7.6, ui-design-map §9).
    ///
    /// The empty-`concept` case is a legal seed — HTS returns an empty
    /// ConceptMap with no groups, which the workbench renders neutrally
    /// (`is_empty_graph = true`) rather than as an error.
    pub async fn cm_closure(
        &self,
        params: &ClosureParams,
    ) -> Result<ClosureResult, UpstreamError> {
        let mut parameters: Vec<Value> = Vec::new();
        parameters.push(json!({"name": "name", "valueString": params.name}));
        for concept in &params.concepts {
            let system = concept.system.trim();
            let code = concept.code.trim();
            if system.is_empty() && code.is_empty() {
                continue;
            }
            let mut coding = serde_json::Map::new();
            if !system.is_empty() {
                coding.insert("system".to_string(), Value::String(system.to_owned()));
            }
            if !code.is_empty() {
                coding.insert("code".to_string(), Value::String(code.to_owned()));
            }
            parameters.push(json!({
                "name": "concept",
                "valueCoding": Value::Object(coding),
            }));
        }
        let body = json!({
            "resourceType": "Parameters",
            "parameter": parameters,
        });
        let url = format!("{}/ConceptMap/$closure", self.base_url);
        let (raw, parsed) = self.post_parameters("cm-closure", &url, &body).await?;

        // HTS wraps the ConceptMap in the classical `return.resource`
        // slot; some builds return the map at the top level, so we
        // accept both shapes.
        let resource = iter_parameters(&parsed)
            .find(|(name, _)| *name == "return")
            .and_then(|(_, value_obj)| value_obj.get("resource").cloned())
            .or_else(|| {
                if parsed.get("resourceType").and_then(|v| v.as_str()) == Some("ConceptMap") {
                    Some(parsed.clone())
                } else {
                    None
                }
            })
            .unwrap_or(Value::Null);

        let map_url = resource
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();
        let map_version = resource
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();

        let edges = parse_closure_edges(&resource);
        let is_empty_graph = edges.is_empty();

        Ok(ClosureResult {
            name: params.name.clone(),
            map_url,
            map_version,
            edges,
            is_empty_graph,
            request_url: url,
            raw_body: raw,
        })
    }

    /// `POST /ValueSet[/{id}]/$validate-code` (design doc §7.6, ui-design-map §6).
    ///
    /// Dispatches on `source` — instance route when `Instance(id)`,
    /// type-level with `url=` when `Canonical`, type-level with
    /// `valueSet=` (an inline ValueSet resource) when `Inline`. HTS
    /// tolerates `result=false` on 200 as the neutral no-membership
    /// state; the caller must NOT surface it as an error partial.
    pub async fn vs_validate_code(
        &self,
        source: &VsValidateSource,
        params: &VsValidateParams,
    ) -> Result<VsValidateResult, UpstreamError> {
        let mut parameters: Vec<Value> = Vec::new();

        // Source. Instance takes the id on the URL path; canonical and
        // inline shapes go on the type-level route with a parameter.
        let (url, source_id) = match source {
            VsValidateSource::Instance(id) => (
                format!("{}/ValueSet/{}/$validate-code", self.base_url, id),
                None,
            ),
            VsValidateSource::Canonical(canonical) => {
                let canonical = canonical.trim();
                if !canonical.is_empty() {
                    parameters.push(json!({"name": "url", "valueUri": canonical}));
                }
                (
                    format!("{}/ValueSet/$validate-code", self.base_url),
                    Some("canonical"),
                )
            }
            VsValidateSource::Inline(vs_json) => {
                // Accepts either a JSON object (`{...}`) or the string
                // as-is; parse-failure surfaces as an invalid-input
                // OperationOutcome from HTS rather than a UI crash.
                let inline: Value =
                    serde_json::from_str(vs_json.trim()).unwrap_or(Value::Null);
                if !inline.is_null() {
                    parameters.push(json!({"name": "valueSet", "resource": inline}));
                }
                (
                    format!("{}/ValueSet/$validate-code", self.base_url),
                    Some("inline"),
                )
            }
        };
        let _ = source_id;

        // Mode-specific payload.
        match params.mode {
            VsValidateMode::Code => {
                if !params.code.is_empty() {
                    parameters
                        .push(json!({"name": "code", "valueCode": params.code.clone()}));
                }
                if !params.system.is_empty() {
                    parameters.push(
                        json!({"name": "system", "valueUri": params.system.clone()}),
                    );
                }
                if let Some(v) = trim_opt(params.system_version.clone()) {
                    parameters.push(json!({"name": "systemVersion", "valueString": v}));
                }
            }
            VsValidateMode::Coding => {
                let mut coding = serde_json::Map::new();
                coding.insert(
                    "system".to_string(),
                    Value::String(params.coding_system.clone()),
                );
                coding.insert(
                    "code".to_string(),
                    Value::String(params.coding_code.clone()),
                );
                if !params.coding_display.is_empty() {
                    coding.insert(
                        "display".to_string(),
                        Value::String(params.coding_display.clone()),
                    );
                }
                parameters.push(json!({
                    "name": "coding",
                    "valueCoding": Value::Object(coding),
                }));
            }
            VsValidateMode::CodeableConcept => {
                let mut coding_array: Vec<Value> = Vec::new();
                for row in &params.coding_rows {
                    let system = row.system.trim();
                    let code = row.code.trim();
                    if system.is_empty() && code.is_empty() {
                        continue;
                    }
                    let mut coding = serde_json::Map::new();
                    if !system.is_empty() {
                        coding.insert(
                            "system".to_string(),
                            Value::String(system.to_owned()),
                        );
                    }
                    if !code.is_empty() {
                        coding.insert(
                            "code".to_string(),
                            Value::String(code.to_owned()),
                        );
                    }
                    coding_array.push(Value::Object(coding));
                }
                let mut cc = serde_json::Map::new();
                cc.insert("coding".to_string(), Value::Array(coding_array));
                if let Some(t) = trim_opt(params.codeable_concept_text.clone()) {
                    cc.insert("text".to_string(), Value::String(t));
                }
                parameters.push(json!({
                    "name": "codeableConcept",
                    "valueCodeableConcept": Value::Object(cc),
                }));
            }
        }

        // Optional common fields.
        if let Some(v) = trim_opt(params.display.clone()) {
            parameters.push(json!({"name": "display", "valueString": v}));
        }
        if let Some(v) = trim_opt(params.display_language.clone()) {
            parameters.push(json!({"name": "displayLanguage", "valueCode": v}));
        }
        if let Some(v) = trim_opt(params.valueset_version.clone()) {
            parameters.push(json!({"name": "valueSetVersion", "valueString": v}));
        }
        if let Some(v) = trim_opt(params.date.clone()) {
            parameters.push(json!({"name": "date", "valueDateTime": v}));
        }
        if let Some(true) = params.active_only {
            parameters.push(json!({"name": "activeOnly", "valueBoolean": true}));
        }
        if let Some(true) = params.abstract_ok {
            parameters.push(json!({"name": "abstract", "valueBoolean": true}));
        }
        if let Some(true) = params.lenient_display_validation {
            parameters.push(
                json!({"name": "lenient-display-validation", "valueBoolean": true}),
            );
        }
        for v in params.use_supplement.iter().filter_map(|s| non_empty_str(s)) {
            parameters.push(json!({"name": "useSupplement", "valueCanonical": v}));
        }
        for v in params.tx_resource.iter().filter_map(|s| non_empty_str(s)) {
            parameters.push(json!({"name": "tx-resource", "valueString": v}));
        }
        for v in params
            .system_version_pins
            .iter()
            .filter_map(|s| non_empty_str(s))
        {
            parameters.push(json!({"name": "system-version", "valueCanonical": v}));
        }
        for v in params
            .check_system_version
            .iter()
            .filter_map(|s| non_empty_str(s))
        {
            parameters
                .push(json!({"name": "check-system-version", "valueCanonical": v}));
        }
        for v in params
            .force_system_version
            .iter()
            .filter_map(|s| non_empty_str(s))
        {
            parameters
                .push(json!({"name": "force-system-version", "valueCanonical": v}));
        }
        if let Some(v) = trim_opt(params.default_valueset_version.clone()) {
            parameters
                .push(json!({"name": "default-valueset-version", "valueCanonical": v}));
        }

        let body = json!({
            "resourceType": "Parameters",
            "parameter": parameters,
        });
        let (raw, parsed) = self.post_parameters("vs-validate", &url, &body).await?;

        let mut out = VsValidateResult {
            raw_body: raw,
            request_url: url,
            ..VsValidateResult::default()
        };
        for (name, value_obj) in iter_parameters(&parsed) {
            match name {
                "result" => {
                    out.result = value_obj
                        .get("valueBoolean")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                }
                "code" => out.code = value_obj_str(value_obj, "valueCode").to_owned(),
                "system" => out.system = value_obj_str(value_obj, "valueUri").to_owned(),
                "version" => {
                    out.version = value_obj_str(value_obj, "valueString").to_owned()
                }
                "display" => {
                    out.display = value_obj_str(value_obj, "valueString").to_owned()
                }
                "message" => {
                    out.message = value_obj_str(value_obj, "valueString").to_owned()
                }
                "issues" => {
                    if let Some(resource) = value_obj.get("resource") {
                        out.issues = Some(OutcomeView::from_body(resource));
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }
}

// ── Slice F: Import Bundle ─────────────────────────────────────────────
//
// Types + method backing the standalone Import page (§7.7). HTS's
// `POST /import` returns:
//   • 200 OK — clean import; body is a JSON object with per-resource
//     counts (`code_systems`, `value_sets`, `concept_maps`, `concepts`)
//     and no `errors[]`.
//   • 207 Multi-Status — partial success; same shape as 200 plus a
//     non-empty `errors[]` array of freeform diagnostic strings (one
//     per non-fatal issue).
//   • 400 Bad Request — malformed Bundle / non-JSON body; HTS returns
//     an `OperationOutcome` via its shared error mapping.
//   • 413 Payload Too Large — transport-level; no body guarantee.
//   • 5xx — storage failure; also an OperationOutcome per HTS.
//
// `ImportCounts` is optional because a 413 (or a decode failure on the
// success body) carries none. The status handler renders "—" when the
// counts are absent rather than fabricating zeros.

/// Status variant returned by [`UpstreamClient::import_bundle`], driving
/// the four visual states in `partials/hts-import-status.html`
/// (design doc §7.7 states matrix).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportStatus {
    /// HTTP 200 — clean import, no non-fatal errors.
    Success,
    /// HTTP 207 — import ran but produced non-fatal errors.
    PartialSuccess,
    /// HTTP 400 — malformed Bundle, HTS refused it before touching storage.
    Rejected,
    /// HTTP 413 — request body exceeded a transport limit. Surface a
    /// split-the-Bundle hint via `hts-import-too-large-hint`.
    TooLarge,
}

impl ImportStatus {
    /// Fluent key suffix rendered by the status partial:
    /// `hts-import-status-{success | partial | rejected | too-large}`.
    pub fn key_suffix(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::PartialSuccess => "partial",
            Self::Rejected => "rejected",
            Self::TooLarge => "too-large",
        }
    }
}

/// Per-resource-type counts returned by HTS on 200 / 207. Absent on
/// 400 / 413 where HTS has nothing to report. Kept as `u32` because
/// the HTS `ImportResponse` struct uses `u32` for each field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportCounts {
    pub code_systems: u32,
    pub value_sets: u32,
    pub concept_maps: u32,
    /// Total number of concept rows inserted across the CS resources
    /// in the Bundle. Rendered as a compact stat below the counts
    /// table in the status partial.
    pub concepts: u32,
}

/// Parsed result of a `POST /import` round-trip. The UI Import handler
/// discriminates on `status` to pick the right variant of the status
/// partial (§7.7 states matrix).
#[derive(Clone, Debug)]
pub struct ImportResult {
    pub status: ImportStatus,
    /// Populated for 200 / 207 when HTS returned its `ImportResponse`
    /// JSON. `None` for 400 / 413 (no counts) or when the body could
    /// not be decoded (still surfaced as `Rejected` rather than a
    /// panic).
    pub counts: Option<ImportCounts>,
    /// Non-fatal issues reported by HTS. On 207 these are the
    /// `errors[]` strings from the success body; on 400 / 5xx they
    /// come from the OperationOutcome (severity/code discarded — each
    /// entry is a single line of diagnostic prose the UI can render
    /// inside `<details><summary>`).
    pub issues: Vec<String>,
    /// Structured OperationOutcome, when the response body was one.
    /// Only populated for `Rejected` (400) responses so the shared
    /// `hts-outcome.html` partial can render the first issue with
    /// full severity / code / location context. `None` on 200 / 207
    /// (where the body is HTS's custom shape) and on 413 (no body).
    pub outcome: Option<OutcomeView>,
    /// URL the request was sent to, for the "Request URL" line in
    /// the status partial (matches the workbench pattern from §7.3).
    pub request_url: String,
    /// Pretty-printed raw response body. Empty on 413.
    pub raw_body: String,
}

impl UpstreamClient {
    /// `POST /import` (design doc §7.7). Body is the raw JSON Bundle
    /// text with `Content-Type: application/fhir+json`. Returns a
    /// structured [`ImportResult`] regardless of upstream status —
    /// only transport-level failures collapse into
    /// [`UpstreamError::Connect`] / [`UpstreamError::Timeout`].
    ///
    /// Rationale for absorbing 4xx into `Ok(ImportResult)`: the Import
    /// page's status region has bespoke rendering per status (200 /
    /// 207 / 400 / 413) that would be lossy to squeeze through the
    /// generic `UpstreamError::Outcome` / `HttpStatus` arms. 5xx
    /// still flows through `UpstreamError` so the shared degraded
    /// banner picks it up.
    pub async fn import_bundle(
        &self,
        bundle_json: &str,
    ) -> Result<ImportResult, UpstreamError> {
        let url = format!("{}/import", self.base_url);
        let response = self
            .client
            .post(&url)
            .header("Accept", "application/fhir+json")
            .header("Content-Type", "application/fhir+json")
            .body(bundle_json.to_owned())
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest("import", &url, e))?;
        let status = response.status();
        let raw = response
            .text()
            .await
            .map_err(|e| UpstreamError::Decode {
                op: "import",
                url: url.clone(),
                message: e.to_string(),
            })?;
        let status_u16 = status.as_u16();

        // 413 typically has no body — synthesize the result without
        // touching serde_json.
        if status_u16 == 413 {
            return Ok(ImportResult {
                status: ImportStatus::TooLarge,
                counts: None,
                issues: Vec::new(),
                outcome: None,
                request_url: url,
                raw_body: raw,
            });
        }

        // 5xx (and anything else outside the handled 200/207/400/413
        // matrix) surfaces through the standard error path so the
        // degraded banner catches it.
        if status_u16 >= 500 {
            let parsed: Value = serde_json::from_str(&raw).unwrap_or_default();
            return Err(status_to_error("import", &url, status_u16, &parsed));
        }

        // 400 — body is an OperationOutcome per HTS's error mapping.
        // Parse loosely so a body that isn't quite an OO still surfaces
        // a rejection state (never a 5xx-shaped panic).
        if status_u16 == 400 {
            let parsed: Value = serde_json::from_str(&raw).unwrap_or_default();
            let outcome = if parsed
                .get("resourceType")
                .and_then(|v| v.as_str())
                .map(|t| t == "OperationOutcome")
                .unwrap_or(false)
            {
                Some(OutcomeView::from_body(&parsed))
            } else {
                None
            };
            let issues = collect_outcome_diagnostics(&parsed);
            let pretty = serde_json::to_string_pretty(&parsed).unwrap_or(raw);
            return Ok(ImportResult {
                status: ImportStatus::Rejected,
                counts: None,
                issues,
                outcome,
                request_url: url,
                raw_body: pretty,
            });
        }

        // 200 / 207 — HTS's custom ImportResponse shape.
        let parsed: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(err) => {
                // Body was not JSON. Surface as `Rejected` with a
                // synthetic outcome so the operator sees something
                // more useful than a crash.
                return Ok(ImportResult {
                    status: ImportStatus::Rejected,
                    counts: None,
                    issues: vec![err.to_string()],
                    outcome: Some(OutcomeView::invalid_input(err.to_string())),
                    request_url: url,
                    raw_body: raw,
                });
            }
        };
        let counts = parse_import_counts(&parsed);
        let issues = parsed
            .get("errors")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let import_status = if status_u16 == 207 || !issues.is_empty() {
            ImportStatus::PartialSuccess
        } else {
            ImportStatus::Success
        };
        let pretty = serde_json::to_string_pretty(&parsed).unwrap_or(raw);
        Ok(ImportResult {
            status: import_status,
            counts,
            issues,
            outcome: None,
            request_url: url,
            raw_body: pretty,
        })
    }
}

/// Parse an HTS `ImportResponse` body into counts. Returns `None` when
/// none of the count fields are present (a body that has an `errors`
/// array but no counts is still legal — the caller records the issues
/// and renders "—" for the columns).
fn parse_import_counts(body: &Value) -> Option<ImportCounts> {
    let obj = body.as_object()?;
    let has_any = ["code_systems", "value_sets", "concept_maps", "concepts"]
        .iter()
        .any(|k| obj.contains_key(*k));
    if !has_any {
        return None;
    }
    let read = |k: &str| -> u32 {
        obj.get(k)
            .and_then(|v| v.as_u64())
            .map(|n| n.min(u32::MAX as u64) as u32)
            .unwrap_or(0)
    };
    Some(ImportCounts {
        code_systems: read("code_systems"),
        value_sets: read("value_sets"),
        concept_maps: read("concept_maps"),
        concepts: read("concepts"),
    })
}

/// Best-effort projection of `OperationOutcome.issue[].diagnostics`
/// into a flat list of strings. Used for the 400-rejection issue list
/// under the shared outcome banner. Missing / empty diagnostics fall
/// back to the issue code so the operator sees something.
fn collect_outcome_diagnostics(body: &Value) -> Vec<String> {
    body.get("issue")
        .and_then(|v| v.as_array())
        .map(|issues| {
            issues
                .iter()
                .map(|i| {
                    let d = i.get("diagnostics").and_then(|v| v.as_str()).unwrap_or("");
                    if !d.is_empty() {
                        d.to_owned()
                    } else {
                        i.get("code")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_owned()
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Walk a ConceptMap resource's `group[].element[].target[]` matrix
/// into a flat edge list. Accepts both R4/R4B (`equivalence`) and
/// R5/R6 (`relationship`) flavors — whichever field HTS returned is
/// preserved as-is in `ClosureEdge::relation`.
fn parse_closure_edges(resource: &Value) -> Vec<ClosureEdge> {
    let mut out: Vec<ClosureEdge> = Vec::new();
    let groups = match resource.get("group").and_then(|v| v.as_array()) {
        Some(g) => g,
        None => return out,
    };
    for group in groups {
        let source_system = group
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();
        let target_system = group
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();
        let elements = match group.get("element").and_then(|v| v.as_array()) {
            Some(e) => e,
            None => continue,
        };
        for element in elements {
            let source_code = element
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            let targets = match element.get("target").and_then(|v| v.as_array()) {
                Some(t) => t,
                None => continue,
            };
            for target in targets {
                let target_code = target
                    .get("code")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned();
                let relation = target
                    .get("equivalence")
                    .and_then(|v| v.as_str())
                    .or_else(|| target.get("relationship").and_then(|v| v.as_str()))
                    .unwrap_or_default()
                    .to_owned();
                out.push(ClosureEdge {
                    source_system: source_system.clone(),
                    source_code: source_code.clone(),
                    target_system: target_system.clone(),
                    target_code,
                    relation,
                });
            }
        }
    }
    out
}

// ── Slice G: Diagnostics fetches ────────────────────────────────────────
//
// Three read-only surfaces the Diagnostics page (design doc §7.9) sits on:
//
//   • `GET /metadata`                     — FHIR CapabilityStatement.
//   • `GET /metadata?mode=terminology`    — FHIR TerminologyCapabilities.
//   • `GET /metrics`                      — Prometheus text-format body.
//
// The existing [`UpstreamClient::terminology_capabilities`] method (used
// by the dashboard) parses only the fields Slice A's cards render
// (`codeSystem[].uri` + `fhirVersion`); Slice G's tabs need the richer
// identity block (`url`, `version`, `name`, `title`, `status`, `date`)
// plus per-`codeSystem[].version[]` details. Rather than mutate the
// dashboard's projection, Slice G ships a parallel
// [`terminology_capabilities_view`] method that returns
// [`TerminologyCapabilitiesView`] — a strict superset of the fields the
// TerminologyCap tab shows.
//
// `metrics_text` returns the raw body verbatim: the metrics tab wraps it
// in `<pre>` inside a `<figure>` and does no numeric parsing.

/// Projection of a FHIR `CapabilityStatement` for the Diagnostics
/// **Capability** tab (design doc §7.9). Only the identity block plus a
/// REST-resource summary is parsed — the tab is a documentation surface,
/// not a machine consumer, so unknown fields are silently dropped.
#[derive(Clone, Debug, Default)]
pub struct CapabilityView {
    pub url: String,
    pub version: String,
    pub name: String,
    pub title: String,
    pub status: String,
    pub date: String,
    /// Flattened `rest[].resource[]` summary — resource type + the list of
    /// advertised interaction verbs (`read`, `search-type`, ...). Empty
    /// when the upstream response does not carry a `rest[]` section.
    pub resources: Vec<CapabilityRestResource>,
}

/// One row in [`CapabilityView::resources`].
#[derive(Clone, Debug, Default)]
pub struct CapabilityRestResource {
    pub resource_type: String,
    pub interactions: Vec<String>,
}

/// Projection of the `TerminologyCapabilities` fields the Diagnostics
/// **TerminologyCap** tab renders (design doc §7.9). Complementary to
/// [`UpstreamTerminologyCapabilities`] — that one exposes the fields
/// Slice A's dashboard cards read (loaded-system count + FHIR version);
/// this one exposes the identity block and the per-system version list
/// the operator surface needs.
#[derive(Clone, Debug, Default)]
pub struct TerminologyCapabilitiesView {
    pub url: String,
    pub version: String,
    pub name: String,
    pub title: String,
    pub status: String,
    pub code_systems: Vec<TerminologyCodeSystemEntry>,
}

/// One row in [`TerminologyCapabilitiesView::code_systems`]. HTS today
/// emits `codeSystem[].uri` but no `codeSystem[].version[]`; the parser
/// accepts either the FHIR-spec array shape (`version[].code`) or a
/// convenience flat string, so a richer server does not break the tab.
#[derive(Clone, Debug, Default)]
pub struct TerminologyCodeSystemEntry {
    pub uri: String,
    pub version: String,
}

impl UpstreamClient {
    /// `GET /metadata` — the FHIR `CapabilityStatement`. Feeds the
    /// Diagnostics **Capability** tab.
    pub async fn capability_statement(&self) -> Result<CapabilityView, UpstreamError> {
        let url = format!("{}/metadata", self.base_url);
        let response = self
            .client
            .get(&url)
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest("metadata", &url, e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(UpstreamError::HttpStatus {
                op: "metadata",
                url,
                status: status.as_u16(),
            });
        }
        let body: Value = response.json().await.map_err(|e| UpstreamError::Decode {
            op: "metadata",
            url: url.clone(),
            message: e.to_string(),
        })?;
        Ok(parse_capability_statement(&body))
    }

    /// `GET /metadata?mode=terminology` — the FHIR
    /// `TerminologyCapabilities`, projected into the richer
    /// [`TerminologyCapabilitiesView`]. Feeds the Diagnostics
    /// **TerminologyCap** tab; the dashboard keeps using
    /// [`Self::terminology_capabilities`] for the loaded-systems count.
    pub async fn terminology_capabilities_view(
        &self,
    ) -> Result<TerminologyCapabilitiesView, UpstreamError> {
        let url = format!("{}/metadata?mode=terminology", self.base_url);
        let response = self
            .client
            .get(&url)
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest("metadata", &url, e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(UpstreamError::HttpStatus {
                op: "metadata",
                url,
                status: status.as_u16(),
            });
        }
        let body: Value = response.json().await.map_err(|e| UpstreamError::Decode {
            op: "metadata",
            url: url.clone(),
            message: e.to_string(),
        })?;
        Ok(parse_terminology_capabilities_view(&body))
    }

    /// `GET /metrics` — the raw Prometheus text-format body. Returned
    /// verbatim so the Diagnostics **/metrics** tab can render it inside
    /// a `<pre>` without a numeric-parser dependency.
    pub async fn metrics_text(&self) -> Result<String, UpstreamError> {
        let url = format!("{}/metrics", self.base_url);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest("metrics", &url, e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(UpstreamError::HttpStatus {
                op: "metrics",
                url,
                status: status.as_u16(),
            });
        }
        response.text().await.map_err(|e| UpstreamError::Decode {
            op: "metrics",
            url,
            message: e.to_string(),
        })
    }
}

fn parse_capability_statement(body: &Value) -> CapabilityView {
    let get_str = |key: &str| -> String {
        body.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    let resources = body
        .get("rest")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .flat_map(|rest| {
                    rest.get("resource")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default()
                })
                .map(|resource| CapabilityRestResource {
                    resource_type: resource
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned(),
                    interactions: resource
                        .get("interaction")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|i| {
                                    i.get("code")
                                        .and_then(|c| c.as_str())
                                        .map(|s| s.to_owned())
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    CapabilityView {
        url: get_str("url"),
        version: get_str("version"),
        name: get_str("name"),
        title: get_str("title"),
        status: get_str("status"),
        date: get_str("date"),
        resources,
    }
}

fn parse_terminology_capabilities_view(body: &Value) -> TerminologyCapabilitiesView {
    let get_str = |key: &str| -> String {
        body.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    let code_systems = body
        .get("codeSystem")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|entry| {
                    let uri = entry
                        .get("uri")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned();
                    // FHIR spec shape: `version` is an array of
                    // BackboneElement with a `code` scalar. Fall back to
                    // a flat string if a server flattens it.
                    let version = entry
                        .get("version")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|first| first.get("code"))
                        .and_then(|v| v.as_str())
                        .or_else(|| entry.get("version").and_then(|v| v.as_str()))
                        .unwrap_or_default()
                        .to_owned();
                    TerminologyCodeSystemEntry { uri, version }
                })
                .collect()
        })
        .unwrap_or_default();
    TerminologyCapabilitiesView {
        url: get_str("url"),
        version: get_str("version"),
        name: get_str("name"),
        title: get_str("title"),
        status: get_str("status"),
        code_systems,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_strips_trailing_slashes() {
        let c = UpstreamClient::new("http://127.0.0.1:8090///").expect("builder");
        assert_eq!(c.base_url(), "http://127.0.0.1:8090");
    }

    #[test]
    fn uptime_pretty_shapes_units_from_seconds() {
        let cases = [
            (0, "0m"),
            (59, "0m"),
            (60, "1m"),
            (3_599, "59m"),
            (3_660, "1h 1m"),
            (86_400, "1d 0h 0m"),
            (90_061, "1d 1h 1m"),
        ];
        for (secs, expected) in cases {
            let h = UpstreamHealth {
                uptime_seconds: secs,
                ..UpstreamHealth::default()
            };
            assert_eq!(h.uptime_pretty(), expected, "for {secs}s");
        }
    }

    #[test]
    fn degraded_reason_is_stable() {
        let e = UpstreamError::HttpStatus {
            op: "health",
            url: "http://example.invalid/health".into(),
            status: 503,
        };
        assert_eq!(e.degraded_reason(), "upstream-error");
    }

    #[test]
    fn browser_filters_clamp_count_to_the_hard_cap() {
        let f = CsBrowserFilters {
            count: 500,
            ..CsBrowserFilters::default()
        };
        assert!(f.count_exceeds_cap());
        assert_eq!(f.effective_count(), CsBrowserFilters::MAX_COUNT);

        let f = CsBrowserFilters {
            count: 0,
            ..CsBrowserFilters::default()
        };
        assert!(!f.count_exceeds_cap());
        assert_eq!(f.effective_count(), CsBrowserFilters::DEFAULT_COUNT);
    }

    #[test]
    fn browser_page_next_offset_stops_when_rows_are_short() {
        let page = CsBrowserPage {
            rows: vec![CsBrowserRow::default(); 5],
            filters: CsBrowserFilters {
                count: 25,
                offset: 0,
                ..CsBrowserFilters::default()
            },
        };
        assert_eq!(page.next_offset(), None);

        let page = CsBrowserPage {
            rows: vec![CsBrowserRow::default(); 25],
            filters: CsBrowserFilters {
                count: 25,
                offset: 50,
                ..CsBrowserFilters::default()
            },
        };
        assert_eq!(page.next_offset(), Some(75));
    }

    #[test]
    fn outcome_view_parses_hts_operation_outcome_shape() {
        let body = serde_json::json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": "not-found",
                "diagnostics": "unknown concept",
                "location": ["Parameters.parameter[0]"]
            }]
        });
        let v = OutcomeView::from_body(&body);
        assert_eq!(v.severity, "error");
        assert_eq!(v.code, "not-found");
        assert_eq!(v.diagnostics, "unknown concept");
        assert_eq!(v.location, vec!["Parameters.parameter[0]".to_string()]);
    }

    #[test]
    fn code_system_summary_heading_falls_back_through_title_name_id() {
        let empty = CodeSystemSummary {
            id: "abc".into(),
            ..CodeSystemSummary::default()
        };
        assert_eq!(empty.heading(), "abc");

        let named = CodeSystemSummary {
            id: "abc".into(),
            name: "MySystem".into(),
            ..CodeSystemSummary::default()
        };
        assert_eq!(named.heading(), "MySystem");

        let titled = CodeSystemSummary {
            id: "abc".into(),
            name: "MySystem".into(),
            title: "My Titled System".into(),
            ..CodeSystemSummary::default()
        };
        assert_eq!(titled.heading(), "My Titled System");
    }
}

impl Default for UpstreamHealth {
    fn default() -> Self {
        Self {
            status: String::new(),
            service: String::new(),
            version: String::new(),
            backend: String::new(),
            uptime_seconds: 0,
        }
    }
}
