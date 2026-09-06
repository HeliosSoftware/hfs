//! Server-rendered, HTMX-first web UI for the Helios FHIR Server.
//!
//! This crate is intentionally thin: handlers parse the request, gather data
//! from the rest of the workspace, and render an [`askama`] template. All markup
//! lives in `templates/`; static assets (htmx, CSS) are embedded at compile time
//! via [`rust_embed`] and served by [`axum_embed`] (with precompressed
//! negotiation), so there is no runtime CDN dependency.
//!
//! Handlers branch on the `HX-Request` header â€” read through the infallible
//! [`axum_htmx::HxRequest`] extractor â€” to return an HTML *fragment* for
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
//! only â€” this is an operator view, and per-tenant counts are never exported to
//! the public Prometheus `/metrics` endpoint.
//!
//! The chart is sampled over a [`DashboardWindow`], selected per request with
//! `?window=` (`1h`, `24h`, or the default `30d`) alongside the `?type=` series
//! selector. Both selectors are plain links, so the dashboard stays navigable
//! without JavaScript. `?all=1` is the "View all resources" toggle (#599): the
//! picker's option list widens from the tenant's stored types to every
//! resource type of the active FHIR version, offering the untouched ones at 0.

mod bulk_export;
mod bulk_import;
mod capability;
mod compartments;
mod conformance;
mod editor;
mod history;
mod i18n;
mod lookup;
mod rail_state;
mod search_params;
mod sql_export;

/// The bounded JSON-fragment engine behind the Raw CapabilityStatement fold
/// moved to `helios-ui-chrome` (#808) so HTS renders the same paginated tree
/// instead of a byte-capped `<pre>`. Re-exported under their old in-crate
/// names so every existing `capability_json::`/`json_view::` reference below
/// — `editor.rs` included — keeps compiling unchanged.
pub(crate) use helios_ui_chrome::capability_json;
pub(crate) use helios_ui_chrome::json_view;
mod sql_libraries;
mod sql_views;
mod subscriptions;
mod tenants;
mod vd_complete;

#[doc(hidden)]
pub use conformance::{
    Caller, ConformanceSource, RecordedExportCall, SqlExportParameter, SqlExportRequest,
    SqlExportStatus, SqlExportSubject, StaticConformanceSource, sql_export_parameters_body,
};

/// The locale plumbing, re-exported out of the private `i18n` module.
///
/// `mount` needs neither of these, but `tests/router_http.rs` does: it is an
/// integration test, so it lives outside the crate, and it asserts the topbar
/// account menu against `helios_ui_chrome::user_menu` rendered from the *real*
/// Fluent catalogs. Without the re-export that test would have to hard-code
/// English strings, which is precisely the drift it exists to catch (#799).
/// `RequestLocale` comes along because it is the only public way to build an
/// [`I18n`].
pub use i18n::{I18n, RequestLocale};

use askama::Template;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Query, RawQuery, State},
    http::StatusCode,
    middleware,
    response::{Html, IntoResponse, Response},
    routing::get,
};
use axum_embed::ServeEmbed;
use axum_htmx::{AutoVaryLayer, HxRequest};
use chrono::{DateTime, Datelike, Duration, Utc};
use helios_observability::dashboard::{
    DashboardPoint, DashboardSeries, DashboardSnapshot, DashboardWindow, ExportJobCounts, TypeCount,
};
use helios_persistence::core::{BulkProviderStore, ResourceStorage, SettingsStore};
use rust_embed::RustEmbed;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::{SystemTime, UNIX_EPOCH};

/// Static UI assets (htmx, CSS) embedded into the binary at compile time.
///
/// Pinned and vendored under `assets/`; never fetched at runtime.
#[derive(Clone, RustEmbed)]
#[folder = "assets/"]
struct Assets;

/// How natural-language search (#255) presents itself in the UI, mirroring the
/// server's `HFS_NL_SEARCH_*` configuration.
///
/// The three states are deliberate: `enabled: false` makes the feature vanish
/// (no page, no nav entry, no mention); enabled but unconfigured advertises
/// what it does and how to switch it on; enabled and configured is the working
/// feature. The UI never sees the API key itself â€” only whether one is set.
#[derive(Clone, Debug, Default)]
pub struct NlSearch {
    /// `HFS_NL_SEARCH_ENABLED` â€” the operator's kill switch.
    pub enabled: bool,
    /// Whether `HFS_NL_SEARCH_API_KEY` is set.
    pub configured: bool,
    /// `HFS_NL_SEARCH_MODEL` â€” shown in the setup state so an operator can see
    /// what they would be billed for.
    pub model: String,
}

/// Whether the mounted FHIR API can search Patient resources by search parameters.
///
/// Exact logical-id reads remain available in both modes. The UI keeps this
/// capability separate because standalone S3 intentionally has no search
/// index, while S3 combined with Elasticsearch does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PatientNameSearchSupport {
    #[default]
    Enabled,
    IdOnly,
}

/// Shared router state: values that are constant for the process lifetime.
#[derive(Clone)]
struct WebState {
    version: &'static str,
    /// Lazily-fetched SearchParameter snapshot per FHIR version (#238), read
    /// from the server's own `/SearchParameter` endpoint.
    sp_catalog: Arc<search_params::SpCatalog>,
    /// Natural-language search feature state (#255).
    nl: Arc<NlSearch>,
    /// Lazily-fetched CompartmentDefinitions per FHIR version (#237), read from
    /// the server's own `/CompartmentDefinition` endpoint.
    compartments: Arc<compartments::CompartmentCatalog>,
    /// The raw conformance source, for reads that are not catalog-shaped —
    /// the live CapabilityStatement fetch (`/metadata`, #653).
    conformance: Arc<dyn ConformanceSource>,
    /// Read/write path for the tenant-maintenance page. `None` when the host did
    /// not wire storage in (e.g. the UI-only unit tests), in which case the page
    /// reports the registry as unavailable rather than crashing.
    tenants: Option<Arc<dyn ResourceStorage>>,
    /// Tenant provisioning jobs started from the tenants page (#581).
    provisioning: tenants::ProvisioningRegistry,
    /// Server data directory (`HFS_DATA_DIR`), used to seed a newly-provisioned
    /// tenant's conformance resources from the tenant-maintenance page.
    data_dir: Option<PathBuf>,
    /// The server's public base URL (`HFS_BASE_URL`) — what Bulk Import
    /// submissions target as their recipient (#689). Distinct from the
    /// loopback self-call base the conformance source uses.
    public_base_url: String,
    /// Trusted loopback base used for UI calls back into this HFS process.
    /// Unlike `public_base_url`, this never carries a reverse-proxy prefix.
    self_base_url: String,
    /// Runtime capability cache. A standards-compliant 501 from Patient name
    /// search downgrades this process to exact-id lookup only.
    patient_name_search: Arc<AtomicBool>,
    /// Whether canonical tenant URLs include the selected tenant as a path.
    tenant_path_routing: bool,
    /// The server's default FHIR version, used when seeding a new tenant.
    fhir_version: helios_fhir::FhirVersion,
    /// The server's default tenant id â€” the fallback when no stored choice
    /// exists (#344).
    default_tenant: String,
    /// Terminology server base URL (`HFS_TERMINOLOGY_SERVER`), when one is
    /// configured â€” powers the editor's live `$expand` pickers (#365).
    terminology: Option<String>,
    /// Per-user settings, for the persisted FHIR-version choice (#343). `None`
    /// when the backend has no settings store; the selector then applies
    /// per-page only.
    settings: Option<Arc<dyn SettingsStore>>,
    /// Provider-side Bulk Submit submissions (#772): tenant-scoped, shared by
    /// the tenant's operators. None when the backend has no store; the Bulk
    /// Import workspace then reports itself unavailable.
    bulk_provider: Option<Arc<dyn BulkProviderStore>>,
}

/// The settings keys holding the user's FHIR-version and tenant choices, and
/// the user key the settings resolve under. The key mirrors `helios-rest`'s
/// `UserKey` post-#270 encoding â€” `u2:{issuer_len}:{issuer}:{subject}` from an
/// authenticated principal, `l2:` when auth is disabled (`/ui` also sits
/// outside the auth layer today; #320 tracks the authenticated modes). Keep in
/// step with `crates/rest/src/extractors/user.rs`.
///
/// These two go through [`SettingsStore`] **directly**, not through
/// `/_user/settings`, so they bypass the per-tenant scoping that handler applies
/// (issue #313). That is correct precisely because both are in
/// [`GLOBAL_SETTINGS_KEYS`](helios_persistence::core::GLOBAL_SETTINGS_KEYS) â€”
/// they are user-global preferences, and `tenantId` in particular has to be
/// readable *before* a tenant is known. Anything added here that is **not** in
/// that list must go through the handler instead, or it will be written outside
/// a tenant purge's reach.
const SETTINGS_VERSION_KEY: &str = "fhirVersion";
const SETTINGS_TENANT_KEY: &str = "tenantId";
const LOCAL_USER_KEY: &str = "l2:";

fn settings_user_key(principal: Option<&helios_auth::Principal>) -> String {
    match principal {
        Some(p) => format!("u2:{}:{}:{}", p.issuer().len(), p.issuer(), p.subject()),
        None => LOCAL_USER_KEY.to_string(),
    }
}

/// The FHIR version this request renders under: the user's stored choice when
/// one exists and is compiled in, the server default otherwise. Resolved once
/// per request by [`resolve_prefs`]; explicit `?version=` query parameters
/// still override it per page.
#[derive(Clone, Copy)]
pub(crate) struct RequestVersion(pub(crate) helios_fhir::FhirVersion);

impl<S> axum::extract::FromRequestParts<S> for RequestVersion
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<RequestVersion>()
            .copied()
            .unwrap_or(RequestVersion(helios_fhir::FhirVersion::default_enabled())))
    }
}

/// The tenant this request renders under (#344): the user's stored choice when
/// it is still a provisioned tenant, the server default otherwise. Carries the
/// registry display name for the selector label.
#[derive(Clone)]
pub(crate) struct RequestTenant {
    pub(crate) id: String,
    pub(crate) display: Option<String>,
    /// Whether this install has any tenant beyond the server default â€” the
    /// sidebar tenant picker only renders when it does (#544).
    pub(crate) multi: bool,
}

impl<S> axum::extract::FromRequestParts<S> for RequestTenant
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<RequestTenant>()
            .cloned()
            .unwrap_or(RequestTenant {
                id: "default".to_string(),
                display: None,
                multi: false,
            }))
    }
}

/// Middleware: stamps [`RequestVersion`] and [`RequestTenant`] from the user's
/// stored settings â€” one settings read per page load, the documented cost model
/// of that store â€” falling back to the server defaults. A stored tenant that is
/// no longer provisioned falls back too, keeping the provisioned-only model.
async fn resolve_prefs(
    State(state): State<WebState>,
    mut request: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    // Batch JSON previews need locale negotiation, but no tenant or FHIR
    // version preference. Avoid the settings and tenant-registry reads on this
    // high-frequency, stateless rendering route; the locale middleware remains
    // in the inner stack and still stamps RequestLocale before the handler.
    if request.uri().path() == "/ui/json-view/render" {
        return next.run(request).await;
    }

    let user_key = settings_user_key(request.extensions().get::<helios_auth::Principal>());
    let mut version = state.fhir_version;
    let mut tenant = RequestTenant {
        id: state.default_tenant.clone(),
        display: None,
        multi: false,
    };
    // The single settings read every rail page's state (`rail_state::RequestSettings`,
    // stamped below) is built from too — reading it again per page would break
    // that module's zero-extra-reads contract.
    let stored = match &state.settings {
        Some(store) => store.get_settings(&user_key).await.ok().flatten(),
        None => None,
    };
    if let Some(stored) = &stored {
        let document = &stored.document;
        if let Some(choice) = document
            .get(SETTINGS_VERSION_KEY)
            .and_then(|v| v.as_str())
            .and_then(search_params::version_from_str)
        {
            version = choice;
        }
        if let Some(choice) = document.get(SETTINGS_TENANT_KEY).and_then(|v| v.as_str())
            && choice != tenant.id
            && let Some(registry) = &state.tenants
            && let Ok(Some(record)) = registry.get_tenant(choice).await
        {
            tenant = RequestTenant {
                id: record.id,
                display: record.display_name,
                multi: false,
            };
        }
    }
    // The picker is pointless on a single-tenant install: show it only when
    // the effective tenant already differs from the default, or the registry
    // knows a second tenant. One indexed registry read per page load, the
    // same cost class as the settings read above.
    tenant.multi = tenant.id != state.default_tenant
        || match &state.tenants {
            Some(registry) => registry
                .list_tenants()
                .await
                .map(|records| records.iter().any(|r| r.id != state.default_tenant))
                .unwrap_or(false),
            None => false,
        };
    request.extensions_mut().insert(RequestVersion(version));
    request.extensions_mut().insert(tenant);
    request
        .extensions_mut()
        .insert(rail_state::RequestSettings {
            version: stored.as_ref().map(|s| s.version).unwrap_or(0),
            document: stored.map(|s| s.document),
            user_key,
        });
    next.run(request).await
}

/// A small, self-contained system-status snapshot â€” the "real read path" the
/// POC renders. Kept deliberately simple so the crate stays dependency-light;
/// richer read paths (terminology lookups, resource counts) plug in the same way.
pub(crate) struct Status {
    pub(crate) version: &'static str,
    checked_at: u64,
    /// The effective FHIR version for this request â€” the sidebar selector's
    /// label (#343).
    fhir_version: helios_fhir::FhirVersion,
    /// The effective tenant for this request â€” the tenant selector's label
    /// (#344).
    tenant_id: String,
    tenant_display: Option<String>,
    /// Whether the sidebar renders the tenant picker (#544).
    show_tenant_picker: bool,
    /// The safe navigation state derived from `HFS_TERMINOLOGY_SERVER` (#611).
    /// The raw value is never exposed to templates unless it is a valid HTTP(S)
    /// base URL.
    terminology: TerminologyNavigation,
}

enum TerminologyNavigation {
    Unconfigured,
    Invalid,
    Valid(String),
}

impl TerminologyNavigation {
    fn from_config(value: Option<&str>) -> Self {
        let Some(raw) = value else {
            return Self::Unconfigured;
        };

        if raw.is_empty() || raw.trim() != raw {
            return Self::Invalid;
        }

        let Ok(url) = reqwest::Url::parse(raw) else {
            return Self::Invalid;
        };
        let valid = matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none();

        if valid {
            Self::Valid(raw.to_string())
        } else {
            Self::Invalid
        }
    }
}

impl Status {
    /// Display label (`"R4"`, `"R5"`, â€¦) of the request's effective FHIR
    /// version â€” the user's stored choice when one is set, else the server
    /// default. Rendered by the sidebar selector and the dashboard's
    /// "Resource types" card (#553).
    pub(crate) fn fhir_version_label(&self) -> &'static str {
        self.fhir_version.as_str()
    }

    /// Labels of every FHIR version compiled into this build, in spec order â€”
    /// the sidebar selector's options. Each links the current page with
    /// `?version=`; pages without a version dimension ignore it.
    pub(crate) fn enabled_version_labels(&self) -> Vec<&'static str> {
        search_params::enabled_versions()
            .into_iter()
            .map(|v| v.as_str())
            .collect()
    }

    /// Whether the sidebar renders the tenant picker (#544).
    pub(crate) fn show_tenant_picker(&self) -> bool {
        self.show_tenant_picker
    }

    /// The topbar avatar menu's identity (#725). `/ui` sits outside the auth
    /// layer today (#320), so no request carries a signed-in principal — every
    /// accessor returns the signed-out shape and the menu renders its
    /// local-operator state. When the browser login flow lands, these become
    /// the seam where the IdP's profile claims (#724) surface: display name,
    /// secondary line (email or subject), initials, photo URL.
    pub(crate) fn user_display(&self) -> Option<&str> {
        None
    }

    pub(crate) fn user_secondary(&self) -> Option<&str> {
        None
    }

    pub(crate) fn user_initials(&self) -> Option<&str> {
        None
    }

    pub(crate) fn user_photo(&self) -> Option<&str> {
        None
    }

    /// Whether the menu offers Sign out — requires an interactive session,
    /// which does not exist yet (#320).
    pub(crate) fn user_can_logout(&self) -> bool {
        false
    }

    /// The topbar account menu, rendered by `helios-ui-chrome` so HFS and HTS
    /// cannot drift (#799). `i18n` is a parameter because it is a sibling field
    /// on every page struct, not part of `Status`.
    ///
    /// Takes `&I18n` rather than `I18n` even though the type is `Copy`: askama
    /// passes template fields to a method call by reference, so the generated
    /// code hands us `&self.i18n`.
    pub(crate) fn user_menu(&self, i18n: &I18n) -> Result<String, askama::Error> {
        helios_ui_chrome::user_menu(
            i18n,
            helios_ui_chrome::UserIdentity {
                display: self.user_display(),
                secondary: self.user_secondary(),
                initials: self.user_initials(),
                photo: self.user_photo(),
                can_logout: self.user_can_logout(),
                logout_href: "/ui/logout",
            },
        )
    }

    /// A browser-safe terminology destination, when the configured value is a
    /// valid absolute HTTP(S) URL (#611).
    pub(crate) fn terminology_url(&self) -> Option<&str> {
        match &self.terminology {
            TerminologyNavigation::Valid(url) => Some(url),
            TerminologyNavigation::Unconfigured | TerminologyNavigation::Invalid => None,
        }
    }

    /// Whether the environment variable exists but cannot be used as a safe
    /// browser destination (#611).
    pub(crate) fn terminology_invalid(&self) -> bool {
        matches!(self.terminology, TerminologyNavigation::Invalid)
    }

    /// The effective tenant id, for the `hfs-tenant` meta tag browser calls
    /// read (#344).
    pub(crate) fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    /// The effective tenant's display label: its registry display name, or the
    /// id when none is set.
    pub(crate) fn tenant_label(&self) -> &str {
        self.tenant_display.as_deref().unwrap_or(&self.tenant_id)
    }

    /// Up-to-two-letter avatar initials from the tenant label.
    pub(crate) fn tenant_initials(&self) -> String {
        let letters: String = self
            .tenant_label()
            .split([' ', '-', '_', '/'])
            .filter(|w| !w.is_empty())
            .take(2)
            .filter_map(|w| w.chars().next())
            .collect();
        if letters.is_empty() {
            "?".to_string()
        } else {
            letters.to_uppercase()
        }
    }
}

/// Dashboard headline metrics rendered by `pages/index.html` (design: Figma
/// "Dashboard V1.1").
///
/// `resource_types`, `stored_resources`, `export_jobs`, `import_jobs`, and
/// `chart_total` are derived from the live [`DashboardSnapshot`], scoped to
/// the request's effective tenant (#344). The card's version label is NOT
/// here: it renders from [`Status`], the same per-request source as the
/// sidebar selector, so the two can never disagree (#553). `uptime` is the
/// process uptime from `helios_observability::uptime` (#540); in a cluster it
/// describes only the node that served this request.
struct DashboardMetrics {
    resource_types: String,
    stored_resources: String,
    /// Bulk-export jobs for the tenant; `None` renders the unavailable state.
    export_jobs: Option<ExportJobCounts>,
    /// Active bulk-submit (import) jobs for the tenant; `None` renders the
    /// unavailable state.
    import_jobs: Option<u64>,
    /// Formatted process uptime; `None` renders the unavailable state (the
    /// uptime tracker was never initialized).
    uptime: Option<String>,
    chart_total: String,
}

/// Process uptime as a short human duration ("3d 4h", "5h 12m", "42m", "18s"),
/// or `None` when the tracker was never initialized and no honest figure
/// exists.
fn format_uptime(seconds: f64) -> Option<String> {
    if seconds <= 0.0 {
        return None;
    }
    let s = seconds as u64;
    let (d, h, m) = (s / 86_400, (s % 86_400) / 3_600, (s % 3_600) / 60);
    Some(if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{s}s")
    })
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

/// One plotted series of the "resources over time" chart: SVG geometry plus
/// the identity the legend and tooltip need (#555).
struct ChartSeriesView {
    resource_type: String,
    /// 1-based palette slot (`--series-N` custom properties, 6 defined).
    color: usize,
    /// `"x,y x,y â€¦"` coordinate list for the `<polyline>`.
    polyline: String,
    /// Class suffix under a legend focus (#602): `" series--focused"`,
    /// `" series--receded"`, or empty when nothing is focused.
    emphasis: &'static str,
    /// The same focus/unfocus link the legend entry carries (#602): the line
    /// itself is clickable, as a native SVG `<a>`. `None` while only one
    /// series is plotted.
    href: Option<String>,
    /// Whether this series holds the focus â€” picks the link's label.
    focused: bool,
}

/// One row of the chart's tabular alternative: a bucket label and the
/// cumulative value of every plotted series at that bucket (#555, a11y).
struct ChartTableRow {
    label: String,
    values: Vec<String>,
}

/// Server-computed SVG geometry for the "resources over time" chart.
struct ChartView {
    /// Whether any series was plotted (`false` â†’ empty state). A plotted
    /// series with a zero total â€” an "empty" type charted via #599 â€” still
    /// counts: it renders as a real flat line, not the empty state.
    has_data: bool,
    series: Vec<ChartSeriesView>,
    /// Horizontal value gridlines, top (largest) to bottom (zero).
    y_ticks: Vec<AxisTick>,
    /// X-axis date labels at evenly spaced sample points.
    x_ticks: Vec<AxisTick>,
    /// viewBox height (always [`CHART_HEIGHT`]).
    height: i64,
    /// Inert JSON the tooltip script reads (`#chart-data`): bucket labels and
    /// per-series values with their SVG coordinates, so the script does no
    /// chart math of its own.
    tip_json: String,
    /// Sampled rows (the labeled buckets) for the accessible table.
    table: Vec<ChartTableRow>,
    /// Comma-joined plotted type names, for the SVG's accessible name.
    types_label: String,
    /// Compact picker-pill label: the first type plus a `+N` overflow count.
    pick_label: String,
}

/// One entry in the chart legend: a plotted series. While more than one is
/// plotted the entry links to focusing that series â€” or, when it is already
/// the focused one, back to the unfocused view (#602). Types leave the chart
/// through the picker, never the legend.
struct LegendEntry {
    resource_type: String,
    total: String,
    color: usize,
    href: Option<String>,
    focused: bool,
}

/// One option in the chart's type picker: a link that toggles the type in or
/// out of the charted set, rendered as a checkbox row (#555).
struct PickerEntry {
    resource_type: String,
    total: String,
    href: String,
    selected: bool,
}

/// One entry in the time-window selector (`1h` / `24h` / `30d`): a link that
/// re-renders the page with the chart sampled over that window, keeping the
/// charted set.
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
    picker: Vec<PickerEntry>,
    windows: Vec<WindowEntry>,
    /// Whether "View all resources" (#599) is on: the picker offers every
    /// resource type of the active FHIR version, not just the tenant's
    /// stored ones.
    all_types: bool,
    /// Link that flips the "View all resources" toggle.
    all_types_href: String,
    /// True when no provider answered and the placeholder snapshot is shown â€”
    /// rendered with an explicit "sample data" notice, never silently (#555).
    sample_data: bool,
    i18n: I18n,
    /// Which sidebar entry carries `aria-current="page"` (see base.html).
    active_page: &'static str,
}

/// Search page (#255, Figma "Search V1.0"): natural language and the visual
/// builder as two modes over one editable FHIR query.
///
/// Rendered only when the feature is enabled. When it is enabled but no API
/// key is configured, the natural-language pane renders its setup state
/// instead of an input â€” the page still works, in visual-builder mode.
#[derive(Template)]
#[template(path = "pages/search.html")]
struct SearchPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    /// Feature state; `configured` picks between the working pane and setup.
    nl: NlSearch,
    /// How-to page for the unconfigured state (docs live in the book).
    docs_url: &'static str,
    resource_types: Vec<String>,
    /// The saved-query controls are the Saved Queries page's job, not this
    /// page's (see `partials/search-builder.html`).
    show_save: bool,
    /// The type rail (#541), server-rendered from `resource_types` and the
    /// dashboard snapshot's counts.
    rail_entries: Vec<RailEntry>,
    /// This request's resolved rail selection (`rail_state`): explicit
    /// `?type=` when given, else the stored `rails.search.last` when it still
    /// resolves, else `Patient`. Exposed as `data-selected-type` so
    /// `saved-queries.js` never falls back to a hardcoded default on initial
    /// load or `popstate` — the same contract `ResourcesPage` already
    /// carries.
    selected_type: String,
    /// The "Recently used" group's rows, server-rendered by
    /// `partials/rail_recent.html`.
    recent_entries: Vec<rail_state::ResolvedRailEntry>,
    /// `rails.<page>` key this page writes/reads (`rail_state::RailPage::key`),
    /// carried to the client so `saved-queries.js` never redeclares it.
    rail_page: &'static str,
    /// `rail_state::MAX_RECENT`, carried the same way.
    max_recent: usize,
    /// No-JS prefill for the builder's URL input (see `ResourcesPage`'s field
    /// of the same name); this page opens with no type context, so it is
    /// always `None`.
    builder_url: Option<String>,
}

/// Resources page (#282): type filter + search + edit modal, on one screen.
#[derive(Template)]
#[template(path = "pages/resources.html")]
struct ResourcesPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    nl: NlSearch,
    docs_url: &'static str,
    resource_types: Vec<String>,
    selected_type: String,
    create_label: String,
    create_disabled: bool,
    create_reason: String,
    create_resource_types: String,
    create_advertised_types: String,
    create_schema_types: String,
    create_metadata_available: bool,
    /// The search-builder partial's save controls are the Saved Queries page's
    /// job, not this one's.
    show_save: bool,
    /// The type rail (#541), server-rendered from `resource_types` and the
    /// dashboard snapshot's counts.
    rail_entries: Vec<RailEntry>,
    /// The "Recently used" group's rows, server-rendered by
    /// `partials/rail_recent.html`.
    recent_entries: Vec<rail_state::ResolvedRailEntry>,
    /// `rails.<page>` key this page writes/reads (`rail_state::RailPage::key`),
    /// carried to the client so `saved-queries.js` never redeclares it.
    rail_page: &'static str,
    /// `rail_state::MAX_RECENT`, carried the same way.
    max_recent: usize,
    /// No-JS prefill for the builder's URL input (#605): `GET /{selected_type}`,
    /// so the form already shows the query the client JS runs on load.
    builder_url: Option<String>,
}

/// Explains how to configure terminology navigation, or why the configured
/// value cannot be used (#611).
#[derive(Template)]
#[template(path = "pages/terminology.html")]
struct TerminologyPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
}

/// Saved FHIR queries page (#234). The shell is server-rendered; the list is
/// hydrated client-side from `/_user/settings` by `assets/saved-queries.js`,
/// the same per-user document (and fetch pattern) the theme toggle uses.
#[derive(Template)]
#[template(path = "pages/queries.html")]
struct QueriesPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    /// The version's resource types for the picker rail, from the spec
    /// CompartmentDefinitions already vendored for the compartment viewer.
    resource_types: Vec<String>,
    show_save: bool,
    /// The type rail (#541), server-rendered from `resource_types` and the
    /// dashboard snapshot's counts.
    rail_entries: Vec<RailEntry>,
    /// This request's resolved rail selection — see `SearchPage`'s field of
    /// the same name.
    selected_type: String,
    /// The "Recently used" group's rows, server-rendered by
    /// `partials/rail_recent.html`.
    recent_entries: Vec<rail_state::ResolvedRailEntry>,
    /// `rails.<page>` key this page writes/reads (`rail_state::RailPage::key`),
    /// carried to the client so `saved-queries.js` never redeclares it.
    rail_page: &'static str,
    /// `rail_state::MAX_RECENT`, carried the same way.
    max_recent: usize,
    /// No-JS prefill for the builder's URL input (see `ResourcesPage`'s field
    /// of the same name); this page opens with no type context, so it is
    /// always `None`.
    builder_url: Option<String>,
}

/// SearchParameter viewer (#238). Read-only against the same snapshot the
/// storage backends seed their registries from; the write half lands
/// behind #235.
#[derive(Template)]
#[template(path = "pages/search-parameters.html")]
struct SearchParametersPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    view: search_params::SpView,
}

/// Batch/Transaction workspace (#476): a static shell; batch.js does the rest.
#[derive(Template)]
#[template(path = "pages/batch.html")]
struct BatchPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
}

/// The shared highlighted JSON fragment used by Editor, Resources, and Batch.
#[derive(Template)]
#[template(path = "partials/json-view.html")]
struct JsonViewFragment {
    i18n: I18n,
    json_lines: Vec<json_view::JsonLine>,
    json_view_id: String,
    json_view_paths: bool,
}

/// Compartment viewer & route tester (#237). Read-only: the base definitions
/// are codegen'd into the binary; a tenant-scoped override layer is open
/// question 1 on the issue.
#[derive(Template)]
#[template(path = "pages/compartments.html")]
struct CompartmentsPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    view: compartments::CmpView,
}

/// The compartments page when no definitions could be fetched (#320): the
/// shell with a warning, in place of the data-bearing view.
#[derive(Template)]
#[template(path = "pages/compartments-degraded.html")]
struct CompartmentsDegradedPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
}

#[derive(Template)]
#[template(path = "partials/status.html")]
struct StatusPartial {
    status: Status,
    i18n: I18n,
}

/// One `<option>` in the search-builder's parameter datalist.
struct ParamOption {
    code: String,
    type_label: String,
    /// Comma-joined target resource types (reference params only, else empty) â€”
    /// the builder's chaining controls read these as `data-targets`.
    targets: String,
}

/// Parameter suggestions for the search builder (`/ui/queries/params`),
/// rendered from the same registry snapshot the SearchParameter viewer
/// reads. An HTML fragment the page swaps per resource type â€” hypermedia,
/// not a UI-facing JSON API.
#[derive(Template)]
#[template(path = "partials/param-options.html")]
struct ParamOptionsPartial {
    params: Vec<ParamOption>,
}

/// History & Versions screen (#236, Figma "History & Versions"): the version
/// rail and the two-layer diff. The shell is server-rendered; the version list
/// and the two compared versions are fetched by the browser from the ordinary
/// `_history` / `vread` FHIR API, then posted to [`history_diff`] to render.
#[derive(Template)]
#[template(path = "pages/history.html")]
struct HistoryPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
}

/// The rendered diff fragment, swapped in when the version selection changes.
#[derive(Template)]
#[template(path = "partials/history-diff.html")]
struct HistoryDiffFragment {
    i18n: I18n,
    diff: history::Diff,
    /// The versions being compared, for the heading (`v3 â†’ v4`).
    from_label: String,
    to_label: String,
    show_metadata: bool,
    /// A version was deleted (an R6 destructive op): render a state banner
    /// rather than a diff against a tombstone.
    deleted: bool,
    /// The two documents could not be parsed â€” the fragment says so instead of
    /// rendering an empty diff.
    parse_error: bool,
}

/// Mounts the web UI under `/ui`, falling back to the FHIR REST app for every
/// other path. The UI depends on the rest of the server, never the reverse.
///
/// The SearchParameter and CompartmentDefinition viewers read the server's own
/// FHIR API over HTTP (storage is the source of truth): `self_base_url` is the
/// loopback base URL the UI calls itself at, and `outbound_auth` supplies the
/// credentials for that self-call (a service token when auth is enabled, a
/// no-op otherwise). `data_dir` (`HFS_DATA_DIR`) and `fhir_version` are used to
/// seed a newly-provisioned tenant from the tenant-maintenance page. `tenants`
/// is the storage handle that page reads and writes; pass `None` to render the
/// UI without a live registry (the page then reports it as unavailable).
///
/// `nl` mirrors the server's natural-language search configuration. With
/// `enabled: false` the `/ui/search` route is never registered, so the page
/// 404s through to the FHIR app exactly as it did before the feature existed.
#[allow(clippy::too_many_arguments)]
pub fn mount(
    fhir_app: Router,
    hfs_version: &'static str,
    data_dir: Option<PathBuf>,
    nl: NlSearch,
    tenants: Option<Arc<dyn ResourceStorage>>,
    settings: Option<Arc<dyn SettingsStore>>,
    default_tenant: String,
    self_base_url: String,
    outbound_auth: Arc<dyn helios_auth::outbound::OutboundAuthProvider>,
    fhir_version: helios_fhir::FhirVersion,
    terminology: Option<String>,
    public_base_url: String,
    bulk_provider: Option<Arc<dyn BulkProviderStore>>,
) -> Router {
    mount_with_body_limit(
        fhir_app,
        hfs_version,
        data_dir,
        nl,
        tenants,
        settings,
        default_tenant,
        self_base_url,
        outbound_auth,
        fhir_version,
        terminology,
        public_base_url,
        10 * 1024 * 1024,
        bulk_provider,
    )
}

/// [`mount`] with an explicit request-body limit for UI rendering endpoints.
#[allow(clippy::too_many_arguments)]
pub fn mount_with_body_limit(
    fhir_app: Router,
    hfs_version: &'static str,
    data_dir: Option<PathBuf>,
    nl: NlSearch,
    tenants: Option<Arc<dyn ResourceStorage>>,
    settings: Option<Arc<dyn SettingsStore>>,
    default_tenant: String,
    self_base_url: String,
    outbound_auth: Arc<dyn helios_auth::outbound::OutboundAuthProvider>,
    fhir_version: helios_fhir::FhirVersion,
    terminology: Option<String>,
    public_base_url: String,
    max_body_size: usize,
    bulk_provider: Option<Arc<dyn BulkProviderStore>>,
) -> Router {
    mount_with_body_limit_and_tenant_routing(
        fhir_app,
        hfs_version,
        data_dir,
        nl,
        tenants,
        settings,
        default_tenant,
        self_base_url,
        outbound_auth,
        fhir_version,
        terminology,
        public_base_url,
        max_body_size,
        false,
        bulk_provider,
        PatientNameSearchSupport::Enabled,
    )
}

/// Mounts the UI with explicit tenant-path routing behavior.
#[allow(clippy::too_many_arguments)]
pub fn mount_with_body_limit_and_tenant_routing(
    fhir_app: Router,
    hfs_version: &'static str,
    data_dir: Option<PathBuf>,
    nl: NlSearch,
    tenants: Option<Arc<dyn ResourceStorage>>,
    settings: Option<Arc<dyn SettingsStore>>,
    default_tenant: String,
    self_base_url: String,
    outbound_auth: Arc<dyn helios_auth::outbound::OutboundAuthProvider>,
    fhir_version: helios_fhir::FhirVersion,
    terminology: Option<String>,
    public_base_url: String,
    max_body_size: usize,
    tenant_path_routing: bool,
    bulk_provider: Option<Arc<dyn BulkProviderStore>>,
    patient_name_search: PatientNameSearchSupport,
) -> Router {
    let source: Arc<dyn ConformanceSource> = Arc::new(conformance::HttpConformanceSource::new(
        self_base_url.clone(),
        outbound_auth,
        fhir_version,
        data_dir.clone(),
    ));
    mount_with_conformance_source_and_runtime(
        fhir_app,
        hfs_version,
        data_dir,
        nl,
        tenants,
        settings,
        default_tenant,
        source,
        fhir_version,
        terminology,
        public_base_url,
        max_body_size,
        tenant_path_routing,
        bulk_provider,
        self_base_url,
        patient_name_search,
    )
}

/// Mounts the UI with an injected [`ConformanceSource`], so tests can serve the
/// SearchParameter/CompartmentDefinition data offline instead of standing up a
/// real HTTP server. Production callers use [`mount`], which wires an HTTP
/// source pointed at the server's own loopback address.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn mount_with_conformance_source(
    fhir_app: Router,
    hfs_version: &'static str,
    data_dir: Option<PathBuf>,
    nl: NlSearch,
    tenants: Option<Arc<dyn ResourceStorage>>,
    settings: Option<Arc<dyn SettingsStore>>,
    default_tenant: String,
    source: Arc<dyn ConformanceSource>,
    fhir_version: helios_fhir::FhirVersion,
    terminology: Option<String>,
    public_base_url: String,
    bulk_provider: Option<Arc<dyn BulkProviderStore>>,
) -> Router {
    mount_with_conformance_source_and_body_limit(
        fhir_app,
        hfs_version,
        data_dir,
        nl,
        tenants,
        settings,
        default_tenant,
        source,
        fhir_version,
        terminology,
        public_base_url,
        10 * 1024 * 1024,
        bulk_provider,
    )
}

/// [`mount_with_conformance_source`] with an explicit UI request-body limit.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn mount_with_conformance_source_and_body_limit(
    fhir_app: Router,
    hfs_version: &'static str,
    data_dir: Option<PathBuf>,
    nl: NlSearch,
    tenants: Option<Arc<dyn ResourceStorage>>,
    settings: Option<Arc<dyn SettingsStore>>,
    default_tenant: String,
    source: Arc<dyn ConformanceSource>,
    fhir_version: helios_fhir::FhirVersion,
    terminology: Option<String>,
    public_base_url: String,
    max_body_size: usize,
    bulk_provider: Option<Arc<dyn BulkProviderStore>>,
) -> Router {
    mount_with_conformance_source_and_body_limit_and_tenant_routing(
        fhir_app,
        hfs_version,
        data_dir,
        nl,
        tenants,
        settings,
        default_tenant,
        source,
        fhir_version,
        terminology,
        public_base_url,
        max_body_size,
        false,
        bulk_provider,
    )
}

/// Testable UI mount with explicit tenant-path routing behavior.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn mount_with_conformance_source_and_body_limit_and_tenant_routing(
    fhir_app: Router,
    hfs_version: &'static str,
    data_dir: Option<PathBuf>,
    nl: NlSearch,
    tenants: Option<Arc<dyn ResourceStorage>>,
    settings: Option<Arc<dyn SettingsStore>>,
    default_tenant: String,
    source: Arc<dyn ConformanceSource>,
    fhir_version: helios_fhir::FhirVersion,
    terminology: Option<String>,
    public_base_url: String,
    max_body_size: usize,
    tenant_path_routing: bool,
    bulk_provider: Option<Arc<dyn BulkProviderStore>>,
) -> Router {
    mount_with_conformance_source_and_runtime(
        fhir_app,
        hfs_version,
        data_dir,
        nl,
        tenants,
        settings,
        default_tenant,
        source,
        fhir_version,
        terminology,
        public_base_url.clone(),
        max_body_size,
        tenant_path_routing,
        bulk_provider,
        public_base_url,
        PatientNameSearchSupport::Enabled,
    )
}

/// Testable mount with explicit same-process FHIR routing and capabilities.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn mount_with_conformance_source_and_runtime(
    fhir_app: Router,
    hfs_version: &'static str,
    data_dir: Option<PathBuf>,
    nl: NlSearch,
    tenants: Option<Arc<dyn ResourceStorage>>,
    settings: Option<Arc<dyn SettingsStore>>,
    default_tenant: String,
    source: Arc<dyn ConformanceSource>,
    fhir_version: helios_fhir::FhirVersion,
    terminology: Option<String>,
    public_base_url: String,
    max_body_size: usize,
    tenant_path_routing: bool,
    bulk_provider: Option<Arc<dyn BulkProviderStore>>,
    self_base_url: String,
    patient_name_search: PatientNameSearchSupport,
) -> Router {
    let nl_enabled = nl.enabled;
    let mut parsed_self_base = reqwest::Url::parse(&self_base_url)
        .expect("UI mount requires a valid HTTP(S) self base URL");
    let trimmed_path = parsed_self_base.path().trim_end_matches('/').to_string();
    parsed_self_base.set_path(&trimmed_path);
    let self_base_url = parsed_self_base
        .to_string()
        .trim_end_matches('/')
        .to_string();
    let mut parsed_public_base = reqwest::Url::parse(&public_base_url)
        .expect("UI mount requires a valid HTTP(S) public base URL");
    let trimmed_path = parsed_public_base.path().trim_end_matches('/').to_string();
    parsed_public_base.set_path(&trimmed_path);
    let public_base_url = parsed_public_base
        .to_string()
        .trim_end_matches('/')
        .to_string();

    // Embedded, pinned htmx + CSS/JS + fonts, served with br/gzip/deflate
    // negotiation. `Cache-Control: no-cache` forces the browser to revalidate
    // against the (content-based) ETag on every load: unchanged assets come
    // back as a cheap `304`, but a rebuilt asset (e.g. app.css after a UI
    // change) is always re-fetched instead of served stale from cache.
    let assets = Router::new()
        .nest_service("/ui/assets", ServeEmbed::<Assets>::new())
        .layer(middleware::from_fn(revalidate_assets));

    let mut router = Router::new()
        .route("/ui", get(index))
        // Resources workspace (#282): the type filter + search + edit modal.
        .route("/ui/resources", get(resources))
        .route("/ui/queries", get(queries))
        .route("/ui/queries/params", get(query_params_catalog))
        .route("/ui/search-parameters", get(search_parameters))
        .route("/ui/terminology", get(terminology_page))
        .route("/ui/compartments", get(compartments_page))
        // Read-only live CapabilityStatement (#653).
        .route("/ui/capability-statement", get(capability_page))
        .route(
            "/ui/capability-statement/json-fragment",
            get(capability_json_fragment),
        )
        .route(
            "/ui/capability-statement/json-expand",
            axum::routing::post(capability_json_expand),
        )
        // Batch/Transaction workspace (#476): upload â†’ preflight â†’ response.
        .route("/ui/batch", get(batch_page))
        // SQL on FHIR workspaces (#649).
        .route(
            "/ui/sql/view-definitions",
            get(sql_view_definitions_page).post(sql_view_definitions_save),
        )
        // #753 (evaluation POC): the editor's async server lint.
        .route(
            "/ui/sql/view-definitions/lint",
            axum::routing::post(sql_view_definitions_lint),
        )
        // #821: the editor's context-completion endpoint — "where is the
        // cursor" from the browser, "what fits there" back from the server.
        .route(
            "/ui/sql/view-definitions/complete",
            axum::routing::post(vd_complete::complete),
        )
        // The playground's live preview fragment (#752, generalized to all
        // three SQL on FHIR pages in #839).
        .route(
            "/ui/sql/view-definitions/run",
            axum::routing::post(sql_view_definitions_run),
        )
        .route(
            "/ui/sql/queries",
            get(sql_queries_page).post(sql_queries_save),
        )
        .route("/ui/sql/queries/run", axum::routing::post(sql_queries_run))
        .route(
            "/ui/sql/queries/document",
            axum::routing::post(sql_queries_document),
        )
        .route("/ui/sql/views", get(sql_views_page).post(sql_views_save))
        .route("/ui/sql/views/run", axum::routing::post(sql_views_run))
        .route(
            "/ui/sql/views/document",
            axum::routing::post(sql_views_document),
        )
        // Active SQL Exports (#833): list-first, mirroring Bulk Export's
        // `/ui/bulk-export` + `/ui/bulk-export/new` shape.
        .route(
            "/ui/sql/export",
            get(sql_export::list).post(sql_export::start),
        )
        .route("/ui/sql/export/new", get(sql_export::new_page))
        // Shared Patient/Group combobox search endpoints (#836): Bulk Export's
        // own Patients field and SQL Export's "Narrow it down" Patients/Groups
        // fields all post here, distinguished by `?target=`.
        .route(
            "/ui/lookup/patient-options",
            axum::routing::post(lookup::patient_options),
        )
        .route(
            "/ui/lookup/group-options",
            axum::routing::post(lookup::group_options),
        )
        // The SQL Query/SQL View *Add table* combobox's own search endpoint
        // (#842): ViewDefinitions and `sql-view` Libraries by name.
        .route(
            "/ui/lookup/table-options",
            axum::routing::post(lookup::table_options),
        )
        // The job detail permalink (#835). `new` above is a literal segment,
        // matched ahead of the `{id}` param at the same depth regardless of
        // route registration order — axum's router always prefers a static
        // route over a dynamic one.
        .route("/ui/sql/export/{id}", get(sql_export::detail_page))
        .route(
            "/ui/sql/export/{id}/detail",
            get(sql_export::detail_fragment),
        )
        .route("/ui/sql/export/{id}/card", get(sql_export::card))
        .route(
            "/ui/sql/export/{id}/cancel",
            axum::routing::post(sql_export::cancel),
        )
        .route(
            "/ui/sql/export/{id}/retry",
            axum::routing::post(sql_export::retry),
        )
        .route(
            "/ui/sql/export/{id}/rerun",
            axum::routing::post(sql_export::rerun),
        )
        .route(
            "/ui/sql/export/{id}/remove",
            axum::routing::post(sql_export::remove),
        )
        // The job-id lookup form (#649) is retired: a job's own permalink
        // above is what it always meant to reach. `301` (rather than
        // axum's `Redirect::permanent`, which answers `308`) so a bookmark
        // or an external link keeps working exactly as before — a GET
        // stays a GET either way, and `301` is what every other permanent
        // move in this codebase (search engines, browser history) expects.
        // Deliberately drops the legacy `?job=`: the list works with
        // locally-generated ids, which that query string never carried
        // (#835).
        .route(
            "/ui/sql/files",
            get(|| async {
                (
                    StatusCode::MOVED_PERMANENTLY,
                    [(axum::http::header::LOCATION, "/ui/sql/export")],
                )
            }),
        )
        .route("/ui/subscriptions", get(subscriptions::page))
        // Schema-driven resource editor (#264). One POST endpoint applies every
        // structural mutation and re-renders: the document rides with it.
        .route("/ui/editor", get(editor::page))
        .route("/ui/editor/expand", get(editor::expand))
        .route(
            "/ui/editor/render",
            axum::routing::post(editor::render_body),
        )
        .route(
            "/ui/json-view/render",
            axum::routing::post(render_json_view).layer(DefaultBodyLimit::max(max_body_size)),
        )
        .route("/ui/status", get(status))
        .route("/ui/history", get(history_page))
        // The diff is computed server-side (the decision in
        // docs/history-diff-rendering.md); the browser posts the two versions
        // it fetched from `_history`.
        .route("/ui/history/diff", axum::routing::post(history_diff))
        .route(
            "/ui/bulk-export",
            get(bulk_export::active).post(bulk_export::start),
        )
        .route("/ui/bulk-export/new", get(bulk_export::page))
        .route("/ui/bulk-export/active", get(bulk_export::active_redirect))
        .route("/ui/bulk-export/active/{id}/card", get(bulk_export::card))
        .route(
            "/ui/bulk-export/active/{id}/cancel",
            axum::routing::post(bulk_export::cancel),
        )
        .route(
            "/ui/bulk-export/active/{id}/retry",
            axum::routing::post(bulk_export::retry),
        )
        .route(
            "/ui/bulk-export/active/{id}/delete",
            axum::routing::post(bulk_export::delete),
        )
        .route(
            "/ui/bulk-export/active/{id}/download",
            get(bulk_export::download_all),
        )
        .route(
            "/ui/bulk-import",
            get(bulk_import::page).post(bulk_import::create),
        )
        .route(
            "/ui/bulk-import/test-auth",
            axum::routing::post(bulk_import::test_auth),
        )
        .route(
            "/ui/bulk-import/empty-manifest.json",
            get(bulk_import::empty_manifest),
        )
        .route("/ui/bulk-import/keys", get(bulk_import::keys))
        .route("/ui/bulk-import/{id}", get(bulk_import::detail))
        .route(
            "/ui/bulk-import/{id}/status",
            get(bulk_import::status_fragment),
        )
        .route(
            "/ui/bulk-import/{id}/delete",
            axum::routing::post(bulk_import::delete),
        )
        .route(
            "/ui/bulk-import/{id}/edit",
            axum::routing::post(bulk_import::edit),
        )
        .route(
            "/ui/bulk-import/{id}/abort",
            axum::routing::post(bulk_import::abort),
        )
        .route(
            "/ui/bulk-import/{id}/complete",
            axum::routing::post(bulk_import::complete),
        )
        .route("/ui/tenants", get(tenants::page).post(tenants::create))
        .route("/ui/tenants/rows", get(tenants::rows))
        .route("/ui/tenants/{id}", axum::routing::delete(tenants::delete))
        // Persists the sidebar's FHIR-version choice (#343) and redirects back.
        .route("/ui/version", axum::routing::post(set_version))
        // The tenant selector (#344): lazily-loaded options and the persisted
        // choice, mirroring /ui/version.
        .route("/ui/tenant/options", get(tenant_options))
        .route("/ui/tenant", axum::routing::post(set_tenant));

    if nl_enabled {
        router = router.route("/ui/search", get(search));
    }

    let state = WebState {
        version: hfs_version,
        sp_catalog: Arc::new(search_params::SpCatalog::new(source.clone())),
        compartments: Arc::new(compartments::CompartmentCatalog::new(source.clone())),
        conformance: source,
        nl: Arc::new(nl),
        tenants,
        provisioning: Default::default(),
        settings,
        bulk_provider,
        data_dir,
        fhir_version,
        default_tenant,
        terminology,
        public_base_url,
        self_base_url,
        patient_name_search: Arc::new(AtomicBool::new(matches!(
            patient_name_search,
            PatientNameSearchSupport::Enabled
        ))),
        tenant_path_routing,
    };

    router
        .merge(assets)
        // Emit `Vary: HX-Request` on handlers that read the header, so caches
        // don't cross a fragment response with a full-page one.
        .layer(AutoVaryLayer)
        // One negotiated locale per request, in request extensions; every
        // handler and template reads this same value.
        .layer(middleware::from_fn(i18n::negotiate_locale))
        // One effective FHIR version per request (stored choice or default),
        // in request extensions next to the locale.
        .layer(middleware::from_fn_with_state(state.clone(), resolve_prefs))
        .with_state(state)
        // Registered after the UI layers so neither arm of `/` picks them up:
        // the redirect needs none, and `POST /` (FHIR batch) must reach the
        // fallback with the same middleware stack as every other FHIR route.
        .route("/", get(root_redirect).fallback_service(fhir_app.clone()))
        .fallback_service(fhir_app)
}

/// Redirects the bare root to the UI home (#896), mirroring HTS. Registered
/// on the UI router, so it only exists when the UI is mounted (a headless
/// build keeps its current behavior) and sits outside the FHIR router's auth
/// layer — an unauthenticated browser lands on `/ui` instead of a 401.
/// Temporary (307) rather than HTS's 308: `/` is also the FHIR batch
/// endpoint, and a permanent redirect gets cached hard by browsers.
async fn root_redirect() -> axum::response::Redirect {
    axum::response::Redirect::temporary("/ui")
}

/// Form body for `POST /ui/version` â€” the sidebar selector's submit.
#[derive(Deserialize)]
struct VersionForm {
    version: String,
}

/// Persists the FHIR-version choice to the user's settings document (the same
/// `/_user/settings` document the theme roams in) and bounces back to the page
/// the form was submitted from. Best-effort: with no settings store the choice
/// still applies to the redirect target via `?version=`.
async fn set_version(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    principal: Option<axum::Extension<helios_auth::Principal>>,
    axum::Form(form): axum::Form<VersionForm>,
) -> Response {
    let Some(version) = search_params::version_from_str(&form.version) else {
        return (StatusCode::BAD_REQUEST, "unknown FHIR version").into_response();
    };

    let mut persisted = false;
    if let Some(store) = &state.settings {
        let user = settings_user_key(principal.as_ref().map(|e| &e.0));
        match store
            .patch_settings(
                &user,
                serde_json::json!({ SETTINGS_VERSION_KEY: version.as_str() }),
                None,
            )
            .await
        {
            Ok(_) => persisted = true,
            Err(e) => tracing::warn!("persisting FHIR-version choice failed: {e}"),
        }
    }

    // Bounce back to the submitting page â€” same-origin `/ui` paths only.
    let back = headers
        .get(axum::http::header::REFERER)
        .and_then(|v| v.to_str().ok())
        .and_then(|r| r.parse::<axum::http::Uri>().ok())
        .map(|u| u.path().to_string())
        .filter(|p| p.starts_with("/ui"))
        .unwrap_or_else(|| "/ui".to_string());
    // Without a store the choice cannot outlive this navigation; carry it on
    // the redirect so the target page still honors it once.
    let target = if persisted {
        back
    } else {
        format!("{back}?version={}", version.as_str())
    };
    axum::response::Redirect::to(&target).into_response()
}

/// One option row of the tenant selector menu.
struct TenantOption {
    id: String,
    label: String,
    initials: String,
    current: bool,
}

/// The tenant selector's options, loaded when the menu opens (htmx) so pages
/// do not pay a registry listing per load.
#[derive(Template)]
#[template(path = "partials/tenant-options.html")]
struct TenantOptionsPartial {
    options: Vec<TenantOption>,
}

fn initials_of(label: &str) -> String {
    let letters: String = label
        .split([' ', '-', '_', '/'])
        .filter(|w| !w.is_empty())
        .take(2)
        .filter_map(|w| w.chars().next())
        .collect();
    if letters.is_empty() {
        "?".to_string()
    } else {
        letters.to_uppercase()
    }
}

/// `GET /ui/tenant/options` â€” the registry's provisioned tenants as selector
/// options, with the effective tenant marked current.
async fn tenant_options(State(state): State<WebState>, rt: RequestTenant) -> Response {
    let mut options = Vec::new();
    if let Some(registry) = &state.tenants
        && let Ok(records) = registry.list_tenants().await
    {
        for record in records {
            let label = record
                .display_name
                .clone()
                .unwrap_or_else(|| record.id.clone());
            options.push(TenantOption {
                current: record.id == rt.id,
                initials: initials_of(&label),
                id: record.id,
                label,
            });
        }
    }
    if !options.iter().any(|o| o.current) {
        let label = rt.display.clone().unwrap_or_else(|| rt.id.clone());
        options.insert(
            0,
            TenantOption {
                current: true,
                initials: initials_of(&label),
                id: rt.id.clone(),
                label,
            },
        );
    }
    render(TenantOptionsPartial { options })
}

/// Form body for `POST /ui/tenant` â€” the tenant selector's submit.
#[derive(Deserialize)]
struct TenantForm {
    tenant: String,
}

/// Persists the tenant choice to the user's settings document and bounces back
/// to the referring page. Only provisioned tenants are accepted (#252's model);
/// anything else is rejected rather than stored.
async fn set_tenant(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    principal: Option<axum::Extension<helios_auth::Principal>>,
    axum::Form(form): axum::Form<TenantForm>,
) -> Response {
    let choice = form.tenant.trim();
    let known = choice == state.default_tenant
        || match &state.tenants {
            Some(registry) => matches!(registry.get_tenant(choice).await, Ok(Some(_))),
            None => false,
        };
    if !known {
        return (StatusCode::BAD_REQUEST, "unknown tenant").into_response();
    }

    if let Some(store) = &state.settings {
        let user = settings_user_key(principal.as_ref().map(|e| &e.0));
        if let Err(e) = store
            .patch_settings(
                &user,
                serde_json::json!({ SETTINGS_TENANT_KEY: choice }),
                None,
            )
            .await
        {
            tracing::warn!("persisting tenant choice failed: {e}");
        }
    }

    let back = headers
        .get(axum::http::header::REFERER)
        .and_then(|v| v.to_str().ok())
        .and_then(|r| r.parse::<axum::http::Uri>().ok())
        .map(|u| u.path().to_string())
        .filter(|p| p.starts_with("/ui"))
        .unwrap_or_else(|| "/ui".to_string());
    axum::response::Redirect::to(&back).into_response()
}

/// The how-to page for natural-language search, linked from the setup state.
const NL_SEARCH_DOCS: &str =
    "https://heliossoftware.github.io/hfs/components/natural-language-search.html";

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

/// Full landing page. `?types=<A,B,â€¦>` selects which resource types the chart
/// plots (defaults to the tenant's largest stored types); unknown names are
/// dropped by the provider, so the set is always real. The legacy `?type=`
/// single selection still works. `?window=<1h|24h|30d>` selects the sampling
/// window, falling back to [`DashboardWindow::default`] for anything
/// unrecognised. `?all=1` is the "View all resources" toggle (#599): with it,
/// the picker offers every resource type of the active FHIR version, not just
/// the ones the tenant stores, and a type with no data can be charted as a
/// flat zero line.
async fn index(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    RawQuery(query): RawQuery,
) -> Response {
    let types: Vec<String> = query_value(query.as_deref(), "types")
        .or_else(|| query_value(query.as_deref(), "type"))
        .map(|csv| {
            csv.split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let window = query_value(query.as_deref(), "window")
        .and_then(|slug| DashboardWindow::from_slug(&slug))
        .unwrap_or_default();
    let all_types = query_value(query.as_deref(), "all").as_deref() == Some("1");
    // The full type list is only fetched when offered â€” the common,
    // flag-off case pays nothing extra for it.
    let spec_types = if all_types {
        state.compartments.resource_type_names(&rt.id, rv.0).await
    } else {
        Vec::new()
    };
    // `?focus=Type` marks one plotted series as the legend focus (#602);
    // validation against the actually-plotted set happens in build_dashboard.
    let focus = query_value(query.as_deref(), "focus")
        .filter(|f| !f.is_empty() && f.chars().all(|c| c.is_ascii_alphanumeric()));
    render(
        build_index_page(
            &state, locale, types, window, all_types, spec_types, focus, rv.0, &rt,
        )
        .await,
    )
}

/// One resource-type rail item â€” the primitive Resources, Search, and Saved
/// Queries share for their type picker (#541): a real link the server marks
/// current, with an optional instance count. `count` is `None` when no
/// dashboard provider is registered; the partial then omits the whole count
/// span rather than mixing real counts with blanks.
struct RailEntry {
    name: String,
    href: String,
    count: Option<String>,
    current: bool,
}

/// Builds the shared type-rail entries for Resources, Search, and Saved
/// Queries: one entry per resource type, linking back to `base` with
/// `?type=<name>`, marked `current` against `selected`. `available` is the
/// dashboard snapshot's per-type totals (`None` when no provider answered â€”
/// every entry then gets `count: None`, never a fabricated zero).
fn build_rail_entries(
    base: &str,
    resource_types: &[String],
    available: Option<&[TypeCount]>,
    selected: Option<&str>,
) -> Vec<RailEntry> {
    let counts: Option<std::collections::HashMap<&str, u64>> = available.map(|types| {
        types
            .iter()
            .map(|t| (t.resource_type.as_str(), t.total))
            .collect()
    });
    resource_types
        .iter()
        .map(|name| RailEntry {
            name: name.clone(),
            href: format!("{base}?type={name}"),
            count: counts
                .as_ref()
                .map(|by_name| by_name.get(name.as_str()).copied().unwrap_or(0).to_string()),
            current: selected == Some(name.as_str()),
        })
        .collect()
}

/// For a type rail (Resources, Search, Saved Queries): the stored `last`
/// when it still names one of `resource_types`, else `fallback`. Only
/// consulted when the request carried no explicit selection at all — an
/// explicit one always wins outright, valid or not (see
/// `resources_query_context`'s doc for why Resources needs its own explicit
/// resolution).
fn resolve_stored_type(last: Option<&str>, resource_types: &[String], fallback: &str) -> String {
    last.filter(|id| resource_types.iter().any(|t| t == id))
        .unwrap_or(fallback)
        .to_string()
}

/// Records an explicit, resolving type selection as this page's rail state,
/// persisting only when [`rail_state::RailState::select`] says something
/// actually changed. Returns `rail` unchanged when `explicit` is absent or
/// does not name a real type — an invalid explicit choice still renders, it
/// is simply never remembered — so a caller can always render this
/// request's "Recently used" group from the returned state, whether or not
/// a write just landed.
async fn record_type_selection(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    page: rail_state::RailPage,
    rail: rail_state::RailState,
    explicit: Option<&str>,
    resource_types: &[String],
) -> rail_state::RailState {
    let Some(id) = explicit.filter(|id| resource_types.iter().any(|t| t == id)) else {
        return rail;
    };
    match rail.select(rail_state::RailEntry::id_only(id)) {
        Some(next) => {
            rail_state::persist(&state.settings, user_key, tenant, page, &next).await;
            next
        }
        None => rail,
    }
}

/// The "Recently used" group's rows for a type rail, resolved against this
/// render's own live entries so label/count/href/current always match what
/// the scrollable list already shows. An id no longer valid for the
/// request's tenant/version (absent from `entries`) is hidden, never pruned
/// — a stale `last` already falls back to the page default in silence, and
/// a filtered render costs nothing.
fn resolve_type_recents(
    rail: &rail_state::RailState,
    entries: &[RailEntry],
    base: &str,
) -> Vec<rail_state::ResolvedRailEntry> {
    let live: std::collections::HashMap<String, rail_state::LiveRailItem> = entries
        .iter()
        .map(|e| {
            (
                e.name.clone(),
                rail_state::LiveRailItem {
                    label: e.name.clone(),
                    meta: None,
                    count: e.count.clone(),
                    href: e.href.clone(),
                    current: e.current,
                },
            )
        })
        .collect();
    let is_valid: &dyn Fn(&str) -> bool = &|id| live.contains_key(id);
    rail.resolve_recents(&live, |id| format!("{base}?type={id}"), Some(is_valid))
}

/// For a SQL rail (View Definitions, SQL Queries, SQL Views): records an
/// explicit, resolving selection as this page's rail state — `select` with
/// the resource's own `{id, name, meta}` snapshot `entry`, since the SQL
/// rails are server-paged/filtered (#741) and cannot rely on the live rail
/// always holding a recent the way a type rail does. Persists only when
/// [`rail_state::RailState::select`] says something actually changed. The
/// counterpart to [`record_type_selection`] for the three pages that carry
/// a snapshot instead of a bare id.
async fn record_snapshot_selection(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    page: rail_state::RailPage,
    rail: rail_state::RailState,
    entry: rail_state::RailEntry,
) -> rail_state::RailState {
    match rail.select(entry) {
        Some(next) => {
            rail_state::persist(&state.settings, user_key, tenant, page, &next).await;
            next
        }
        None => rail,
    }
}

/// For a SQL rail: prunes an explicit selection that did not resolve (a
/// deleted or mistyped id) from the stored registry, persisting only when
/// the id was actually recorded ([`rail_state::RailState::prune`] already
/// reports the no-op case). The request itself keeps its current "no
/// selection" render either way; this only cleans the record so a later
/// visit does not keep resolving the same id to a resource that is gone.
async fn prune_stale_selection(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    page: rail_state::RailPage,
    rail: rail_state::RailState,
    id: &str,
) -> rail_state::RailState {
    match rail.prune(id) {
        Some(next) => {
            rail_state::persist(&state.settings, user_key, tenant, page, &next).await;
            next
        }
        None => rail,
    }
}

/// Search page: natural language and the visual builder over one editable query.
async fn search(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(query): Query<SearchQuery>,
    settings: rail_state::RequestSettings,
) -> Response {
    let resource_types = state.compartments.resource_type_names(&rt.id, rv.0).await;
    let explicit_type = query.resource_type.as_deref().filter(|t| !t.is_empty());
    let rail = record_type_selection(
        &state,
        &settings.user_key,
        &rt.id,
        rail_state::RailPage::Search,
        settings.rail(rail_state::RailPage::Search, &rt.id),
        explicit_type,
        &resource_types,
    )
    .await;
    let selected_type = explicit_type
        .map(str::to_string)
        .unwrap_or_else(|| resolve_stored_type(rail.last.as_deref(), &resource_types, "Patient"));
    let live =
        helios_observability::dashboard::snapshot(DashboardWindow::default(), &rt.id, &[], false)
            .await;
    let rail_entries = build_rail_entries(
        "/ui/search",
        &resource_types,
        live.as_ref().map(|s| s.available.as_slice()),
        Some(selected_type.as_str()),
    );
    let recent_entries = resolve_type_recents(&rail, &rail_entries, "/ui/search");
    render(SearchPage {
        status: current_status(&state, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "search",
        nl: (*state.nl).clone(),
        docs_url: NL_SEARCH_DOCS,
        resource_types,
        show_save: false,
        rail_entries,
        selected_type,
        recent_entries,
        rail_page: rail_state::RailPage::Search.key(),
        max_recent: rail_state::MAX_RECENT,
        builder_url: None,
    })
}

/// Query string for the Search page: an optional pre-selected type, so the
/// rail's own links round-trip through `/ui/search?type=Observation` (#541).
#[derive(Deserialize, Default)]
struct SearchQuery {
    #[serde(rename = "type")]
    resource_type: Option<String>,
}

/// Saved FHIR queries page.
async fn queries(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(query): Query<QueriesQuery>,
    settings: rail_state::RequestSettings,
) -> Response {
    let resource_types = state.compartments.resource_type_names(&rt.id, rv.0).await;
    let explicit_type = query.resource_type.as_deref().filter(|t| !t.is_empty());
    let rail = record_type_selection(
        &state,
        &settings.user_key,
        &rt.id,
        rail_state::RailPage::Queries,
        settings.rail(rail_state::RailPage::Queries, &rt.id),
        explicit_type,
        &resource_types,
    )
    .await;
    let selected_type = explicit_type
        .map(str::to_string)
        .unwrap_or_else(|| resolve_stored_type(rail.last.as_deref(), &resource_types, "Patient"));
    let live =
        helios_observability::dashboard::snapshot(DashboardWindow::default(), &rt.id, &[], false)
            .await;
    let rail_entries = build_rail_entries(
        "/ui/queries",
        &resource_types,
        live.as_ref().map(|s| s.available.as_slice()),
        Some(selected_type.as_str()),
    );
    let recent_entries = resolve_type_recents(&rail, &rail_entries, "/ui/queries");
    render(QueriesPage {
        status: current_status(&state, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "queries",
        resource_types,
        show_save: true,
        rail_entries,
        selected_type,
        recent_entries,
        rail_page: rail_state::RailPage::Queries.key(),
        max_recent: rail_state::MAX_RECENT,
        builder_url: None,
    })
}

/// Query string for the Saved Queries page: an optional pre-selected type, so
/// the rail's own links round-trip through `/ui/queries?type=Observation` (#541).
#[derive(Deserialize, Default)]
struct QueriesQuery {
    #[serde(rename = "type")]
    resource_type: Option<String>,
}

/// Resources page (#282): the primary read/write workspace. Ties together the
/// type filter, the search (natural-language + visual builder), and â€” on a
/// result click â€” the edit modal that reuses the schema-driven editor, with
/// save / delete / version history / diff. The pieces are the same partials the
/// Search and Editor pages render; this page is the place they come together.
async fn resources(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(query): Query<ResourcesQuery>,
    settings: rail_state::RequestSettings,
) -> Response {
    let resource_types = state.compartments.resource_type_names(&rt.id, rv.0).await;
    // `url` is the builder's actual query, so it wins over the convenience
    // `type` bookmark. Parse it here too: Create must be safe before the
    // deferred client script hydrates the page. `explicit_type` is `None`
    // only when neither `url` nor `type` named anything at all — an explicit
    // selection always wins outright once present, invalid or not, so only
    // its absence falls through to the stored/fallback resolution below.
    let (explicit_type, url_from_query) = resources_query_context(&query);
    let rail = record_type_selection(
        &state,
        &settings.user_key,
        &rt.id,
        rail_state::RailPage::Resources,
        settings.rail(rail_state::RailPage::Resources, &rt.id),
        explicit_type.as_deref(),
        &resource_types,
    )
    .await;
    let selected_type = explicit_type
        .unwrap_or_else(|| resolve_stored_type(rail.last.as_deref(), &resource_types, "Patient"));
    let builder_url = url_from_query.or_else(|| Some(format!("/{selected_type}")));
    let targets = match state.conformance.metadata(rv.0, &rt.id).await {
        Ok(statement) => {
            match capability::CreateTargets::from_statement(&resource_types, &statement, rv.0) {
                Ok(targets) => Some(targets),
                Err(error) => {
                    tracing::warn!("Resources create-target metadata rejected: {error}");
                    None
                }
            }
        }
        Err(error) => {
            tracing::warn!("Resources create-target metadata fetch failed: {error}");
            None
        }
    };
    let block = targets
        .as_ref()
        .map_or(
            Err(capability::CreateTargetBlock::MetadataUnavailable),
            |targets| targets.classify(&selected_type),
        )
        .err();
    let i18n = I18n::new(locale);
    let create_reason = block
        .map(|block| create_target_reason(&i18n, block))
        .unwrap_or_default();
    let create_label = if selected_type.is_empty() {
        i18n.t("resources-create")
    } else {
        i18n.t_arg("resources-create-typed", "type", selected_type.clone())
    };
    let live =
        helios_observability::dashboard::snapshot(DashboardWindow::default(), &rt.id, &[], false)
            .await;
    let rail_entries = build_rail_entries(
        "/ui/resources",
        &resource_types,
        live.as_ref().map(|s| s.available.as_slice()),
        Some(selected_type.as_str()),
    );
    let recent_entries = resolve_type_recents(&rail, &rail_entries, "/ui/resources");
    render(ResourcesPage {
        status: current_status(&state, rv.0, &rt),
        i18n,
        active_page: "resources",
        nl: (*state.nl).clone(),
        docs_url: NL_SEARCH_DOCS,
        resource_types,
        selected_type,
        create_label,
        create_disabled: block.is_some(),
        create_reason,
        create_resource_types: targets
            .as_ref()
            .map(capability::CreateTargets::resource_types_csv)
            .unwrap_or_default(),
        create_advertised_types: targets
            .as_ref()
            .map(capability::CreateTargets::advertised_create_csv)
            .unwrap_or_default(),
        create_schema_types: targets
            .as_ref()
            .map(capability::CreateTargets::schema_resources_csv)
            .unwrap_or_default(),
        create_metadata_available: targets.is_some(),
        show_save: false,
        rail_entries,
        recent_entries,
        rail_page: rail_state::RailPage::Resources.key(),
        max_recent: rail_state::MAX_RECENT,
        builder_url,
    })
}

/// Terminology setup state (#611). The sidebar links here when the environment
/// variable is absent or cannot be used as a safe browser destination.
async fn terminology_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
) -> Response {
    render(TerminologyPage {
        status: current_status(&state, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "terminology",
    })
}

/// Query string for the Resources page: an optional pre-selected type, so the
/// nav submenu can deep-link `/ui/resources?type=Observation`.
#[derive(Deserialize, Default)]
struct ResourcesQuery {
    #[serde(rename = "type")]
    resource_type: Option<String>,
    url: Option<String>,
}

/// Resolves the Resources page's explicit selection and no-JS builder
/// prefill from the query string, ahead of the stored/fallback resolution
/// below.
///
/// `?url=` wins outright when present — the builder's actual query, so it
/// beats the convenience `?type=` bookmark (#605) — even when no type can be
/// parsed from it: an opaque or malformed URL still searches verbatim
/// (`Some(String::new())`, never re-routed through the stored/fallback path),
/// matching the page's existing "preserve invalid input" contract. Otherwise
/// `?type=` (when non-empty) is the explicit type, also used verbatim even if
/// it names no real resource type — `record_type_selection` is what decides
/// whether an explicit choice is remembered, never whether it renders.
///
/// Returns `(None, None)` only when neither is present at all, letting the
/// stored/fallback resolution below resolve the selection and synthesize
/// `/{selected_type}` as the builder prefill.
fn resources_query_context(query: &ResourcesQuery) -> (Option<String>, Option<String>) {
    if let Some(raw_url) = &query.url {
        let visible = raw_url
            .trim()
            .strip_prefix("GET ")
            .or_else(|| raw_url.trim().strip_prefix("get "))
            .unwrap_or(raw_url.trim())
            .to_string();
        return (
            Some(resource_type_from_search_url(&visible).unwrap_or_default()),
            Some(visible),
        );
    }

    let explicit = query
        .resource_type
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    (explicit, None)
}

fn resource_type_from_search_url(raw: &str) -> Option<String> {
    let mut value = raw.trim();
    if let Some(after_scheme) = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))
    {
        value = after_scheme.find('/').map(|index| &after_scheme[index..])?;
    }
    let path = value.split_once('?').map_or(value, |(path, _)| path);
    let resource_type = path.strip_prefix('/').unwrap_or(path);
    if resource_type.is_empty()
        || resource_type.contains('/')
        || !resource_type.chars().all(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    Some(resource_type.to_string())
}

fn create_target_reason(i18n: &I18n, block: capability::CreateTargetBlock) -> String {
    let key = match block {
        capability::CreateTargetBlock::InvalidType => "resources-create-invalid-type",
        capability::CreateTargetBlock::CreateNotAdvertised => "resources-create-not-advertised",
        capability::CreateTargetBlock::SchemaUnavailable => "resources-create-schema-unavailable",
        capability::CreateTargetBlock::MetadataUnavailable => {
            "resources-create-metadata-unavailable"
        }
    };
    i18n.t(key)
}

#[derive(Deserialize, Default)]
struct ParamsCatalogQuery {
    #[serde(rename = "type")]
    resource_type: Option<String>,
}

/// Parameter datalist for the search builder: the active parameters that
/// apply to the given resource type (including `Resource` /
/// `DomainResource`-level ones), from the selected version's snapshot.
async fn query_params_catalog(
    State(state): State<WebState>,
    rt: RequestTenant,
    rv: RequestVersion,
    Query(raw): Query<ParamsCatalogQuery>,
) -> Response {
    let snapshot = state.sp_catalog.snapshot(&rt.id, rv.0).await;
    let resource_type = raw.resource_type.unwrap_or_default();
    let mut params: Vec<ParamOption> = snapshot
        .params
        .iter()
        .filter(|p| p.applies_to(&resource_type))
        .map(|p| ParamOption {
            code: p.code.clone(),
            type_label: p.param_type.to_string(),
            targets: p.target.as_deref().unwrap_or_default().join(","),
        })
        .collect();
    params.sort_by(|a, b| a.code.cmp(&b.code));
    params.dedup_by(|a, b| a.code == b.code);
    render(ParamOptionsPartial { params })
}

/// Query string for the SearchParameter viewer. Every filter is a link and
/// the search box is a GET form, so the page works without JavaScript.
///
/// `base` is left as the raw, three-state `Option<String>` Serde gives it —
/// absent, present-and-empty, present-and-named all mean something different
/// (see [`resolve_sp_base`]) — rather than collapsing "absent" and "explicit
/// all types" into one `None` the way `main` used to.
#[derive(Deserialize, Default)]
struct SearchParametersQuery {
    version: Option<String>,
    base: Option<String>,
    #[serde(rename = "type")]
    ptype: Option<String>,
    source: Option<String>,
    #[serde(default)]
    q: String,
    page: Option<usize>,
    sel: Option<String>,
    /// Set by the CRUD flows after a write: drop the cached snapshot first.
    refresh: Option<String>,
}

/// Resolves Search Parameters' `base`, whose one twist is that "All types"
/// is itself an explicit, persistable state
/// ([`rail_state::RailState::select_all`], `last: ""`), not merely the
/// absence of a selection:
///
/// - `?base=` present and empty is explicit "All types".
/// - `?base=<name>` present is explicit, whether or not `<name>` is one of
///   `bases` — resolving only decides whether it is written, never whether
///   it renders (an explicit selection always wins, as `build_view` already
///   did for an unknown base before this).
/// - `?base=` absent falls back to the stored `last` when it still resolves
///   (including a stored "All types"), else the page's own fallback, "All
///   types" — neither is written, since nothing was explicitly asked.
///
/// Returns the base to render (`None` for "All types") alongside the
/// possibly-updated [`rail_state::RailState`], so this same response's
/// "Recently used" group reflects a write this request just made.
async fn resolve_sp_base(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    rail: rail_state::RailState,
    explicit_base: Option<&str>,
    bases: &std::collections::HashSet<&str>,
) -> (Option<String>, rail_state::RailState) {
    async fn persist_if_changed(
        state: &WebState,
        user_key: &str,
        tenant: &str,
        rail: rail_state::RailState,
        next: Option<rail_state::RailState>,
    ) -> rail_state::RailState {
        match next {
            Some(next) => {
                rail_state::persist(
                    &state.settings,
                    user_key,
                    tenant,
                    rail_state::RailPage::SearchParameters,
                    &next,
                )
                .await;
                next
            }
            None => rail,
        }
    }

    match explicit_base {
        Some("") => {
            let next = rail.select_all();
            let rail = persist_if_changed(state, user_key, tenant, rail, next).await;
            (None, rail)
        }
        Some(base) if bases.contains(base) => {
            let next = rail.select(rail_state::RailEntry::id_only(base));
            let rail = persist_if_changed(state, user_key, tenant, rail, next).await;
            (Some(base.to_string()), rail)
        }
        Some(base) => (Some(base.to_string()), rail),
        None => {
            let resolved = match rail.last.as_deref() {
                Some(name) if !name.is_empty() && bases.contains(name) => Some(name.to_string()),
                _ => None,
            };
            (resolved, rail)
        }
    }
}

/// SearchParameter viewer page.
async fn search_parameters(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(raw): Query<SearchParametersQuery>,
    settings: rail_state::RequestSettings,
) -> Response {
    // Explicit ?version= wins; otherwise the user's stored choice (#343).
    let version = raw.version.or_else(|| Some(rv.0.as_str().to_string()));
    let fhir_version = version
        .as_deref()
        .and_then(search_params::version_from_str)
        .unwrap_or_default();
    if raw.refresh.is_some() {
        state.sp_catalog.invalidate(&rt.id, fhir_version);
    }
    let snapshot = state.sp_catalog.snapshot(&rt.id, fhir_version).await;
    let bases: std::collections::HashSet<&str> = snapshot
        .params
        .iter()
        .flat_map(|p| p.base.iter().map(String::as_str))
        .collect();
    let (base, rail) = resolve_sp_base(
        &state,
        &settings.user_key,
        &rt.id,
        settings.rail(rail_state::RailPage::SearchParameters, &rt.id),
        raw.base.as_deref(),
        &bases,
    )
    .await;
    let query = search_params::SpQuery {
        version,
        base,
        ptype: raw.ptype.filter(|t| !t.is_empty()),
        source: raw.source.filter(|s| !s.is_empty()),
        q: raw.q,
        page: raw.page.unwrap_or(1),
        sel: raw.sel.filter(|s| !s.is_empty()),
    };
    render(SearchParametersPage {
        status: current_status(&state, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "search-parameters",
        view: search_params::build_view(&snapshot, &query, &rail),
    })
}

/// Query string for the compartment viewer & tester.
#[derive(Deserialize, Default)]
struct CompartmentsQuery {
    version: Option<String>,
    def: Option<String>,
    tab: Option<String>,
    filter: Option<String>,
    #[serde(default)]
    id: String,
    #[serde(default)]
    target: String,
    /// Set by the CRUD flows after a write: drop the cached definitions first.
    refresh: Option<String>,
}

/// Syntax-highlights arbitrary JSON without applying FHIR semantics or
/// retaining the payload. Batch sends compact JSON and receives only HTML.
async fn render_json_view(
    locale: RequestLocale,
    Json(document): Json<serde_json::Value>,
) -> Response {
    // A compact JSON array can turn a few KiB into thousands of HTML lines.
    // Keep previews below 4,000 lines and a conservative 2 MiB output estimate
    // so the configured request-body limit cannot be used for amplification.
    const MAX_LINES: usize = 4_000;
    const MAX_ESTIMATED_HTML_BYTES: usize = 2 * 1024 * 1024;

    let json_lines = match json_view::try_lines(
        &document,
        json_view::RenderOptions {
            include_paths: false,
            budget: Some(json_view::RenderBudget {
                max_lines: MAX_LINES,
                max_estimated_html_bytes: MAX_ESTIMATED_HTML_BYTES,
            }),
        },
    ) {
        Ok(lines) => lines,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                "JSON preview exceeds the rendering budget",
            )
                .into_response();
        }
    };

    render(JsonViewFragment {
        i18n: I18n::new(locale),
        json_lines,
        json_view_id: String::new(),
        json_view_paths: false,
    })
}

/// Batch/Transaction workspace page (#476). The shell is server-rendered;
/// batch.js drives upload â†’ preflight â†’ execute â†’ response entirely against
/// the ordinary FHIR root, so this crate never touches storage.
async fn batch_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
) -> Response {
    render(BatchPage {
        status: current_status(&state, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "batch",
    })
}

/// The View Definitions playground (#649, Figma `420-2`; #752): a filter
/// rail of the tenant's stored ViewDefinitions, the selected one always
/// editable as JSON, and a `$sql-run` preview that follows the editor's
/// current text — saved or not, no Run button. Save and Duplicate are plain
/// form posts (they work without JavaScript); Delete rides
/// `conformance-crud.js` like the other conformance viewers.
#[derive(Template)]
#[template(path = "pages/sql-view-definitions.html")]
struct SqlViewDefinitionsPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    rail: Vec<sql_views::VdSummary>,
    /// The rail's server-side name filter, echoed back into the search box.
    filter: String,
    /// The rail's "previous" link, `None` on the first page (#741).
    prev_href: Option<String>,
    /// The rail's "next" link, `None` when the search reported no further page.
    next_href: Option<String>,
    /// The list search failed — the page says so instead of showing an empty rail.
    degraded: Option<String>,
    /// `?vd=` (or the rail's first visible entry): id, name, and pretty JSON.
    /// `None` only when the store holds no views and none is being created.
    selected: Option<SelectedVd>,
    /// `?vd=new`: the JSON below is the starter document, not a stored view.
    is_new: bool,
    /// The guided-form card, alongside the JSON editor (#843): the same
    /// `pane=form` fragment `POST /ui/editor/render` would render for
    /// `selected`'s document, built inline instead of fetched — the page's
    /// first paint needs it in place already, not filled in after `load`,
    /// or the editor card would flash full-width before shrinking to
    /// make room for it. `None` only alongside `selected: None` (no view
    /// selected, nothing to build a form for).
    form_pane: Option<editor::EditorFormPane>,
    /// The `$sql-run` preview card and its failure notice, nested as its own
    /// template (#752) so `partials/sql_run_results.html`'s markup exists in
    /// exactly one place, shared with the `/run` fragment endpoint
    /// (`sql_view_definitions_run`) and, since #839, with the same partial
    /// on the SQL Queries and SQL Views pages. `fragment: false` here — the
    /// page's own render has nothing to swap into.
    run_results: RunResultsPartial,
    save_error: Option<String>,
    saved: bool,
    /// The "Recently used" group's own rows.
    recent_entries: Vec<rail_state::ResolvedRailEntry>,
    /// `rails.<page>` key this page writes/reads (`rail_state::RailPage::key`),
    /// carried to the template so `partials/rail_recent.html`'s clones can
    /// name it without redeclaring it.
    rail_page: &'static str,
    /// The recents cap a tampered client can never exceed, likewise from
    /// `rail_state::MAX_RECENT`, carried the same way.
    max_recent: usize,
}

struct SelectedVd {
    id: String,
    name: String,
    json: String,
}

/// The `$sql-run` preview's three renderable shapes (#752, generalized in
/// #839). A tuple enum rather than parallel `Option`s so a table can never
/// appear alongside a failure message, and the page's own "nothing has run
/// yet" state is distinct from both — the invalid combinations simply have
/// no constructor.
enum RunResultsState {
    /// A successful run — its table, plus how long the `$sql-run` call took
    /// in whole milliseconds.
    Success(sql_views::RunTable, u64),
    /// A run or parse failure, with the message rendered next to the
    /// surface's own `failed_key`, and — when the message carries
    /// sqlparser's own `Line: N` marker — the 1-based line number the
    /// editor tints (#839, [`sql_views::extract_error_line`]).
    Failure(String, Option<u32>),
    /// A SQL Query with a declared, required parameter this run has no
    /// value for (#841): `$sql-run` is never called, and the previous
    /// table (if any) is left in place — the same OOB shape as `Failure`
    /// (the stale-meta relabel, no `#run-results` swap) but rendered as a
    /// plain, non-`--warn` notice naming which `:name`(s) are missing,
    /// since this is an expected pause, not an error.
    Waiting(String),
    /// The page's own render before anything has run server-side — no
    /// `?saved=1`/`?lib=…&saved=1`, or the current selection has no preview
    /// yet. Renders the notice region's own client-driven initial-load
    /// request. Never produced by a `/run` fragment endpoint, which always
    /// ends in `Success` or `Failure`.
    Empty,
}

/// The `$sql-run` preview card and its failure notice (#752, generalized to
/// share one partial across View Definitions, SQL Queries, and SQL Views in
/// #839): `partials/sql_run_results.html`'s markup is written once and
/// rendered by two kinds of callers per page — nested as a template field
/// for the page's own initial render (`{{ run_results.render()?|safe }}`,
/// `fragment: false`), and directly as the whole response of that page's own
/// `POST …/run` fragment endpoint (`fragment: true`). `fragment` only
/// toggles the `hx-swap-oob` attributes and whether the `Empty` arm's own
/// load trigger applies; the table markup itself lives solely in the
/// template's `Success` arm.
///
/// The remaining fields are the "surface" each caller supplies (#839) so the
/// partial itself never hardcodes one page's ids or i18n keys: every field
/// below but `i18n`/`fragment`/`state` only names where this render's
/// fragment endpoint, form, and i18n keys live — never resource data.
#[derive(Template)]
#[template(path = "partials/sql_run_results.html")]
struct RunResultsPartial {
    i18n: I18n,
    fragment: bool,
    /// The surface's own `POST …/run` fragment endpoint — the `Empty` arm's
    /// `hx-post` load trigger targets it (the surface's own textarea, wired
    /// outside this partial, posts to the identical URL on every edit).
    run_href: &'static str,
    /// The id of the `<form>` the surface's editable fields live in. The
    /// `Empty` arm's shell sits outside that form, so it reaches back in via
    /// `hx-include="#{form_id}"` for its own initial-load request.
    form_id: &'static str,
    /// The results card's `<h3>` key.
    heading_key: &'static str,
    /// The failure notice's message-prefix key.
    failed_key: &'static str,
    /// `Some` renders an "Export as files" action beside the meta on a
    /// successful run; `None` renders nothing (#839). Only SQL Queries
    /// offers it, and only once the running document has a saved Library id.
    export_href: Option<String>,
    state: RunResultsState,
}

/// `partials/sql_run_results.html`'s surface descriptor for the View
/// Definitions playground — its `POST …/run` endpoint and the id of its one
/// editable form (#839). View Definitions has no Export action, so its
/// `export_href` is always `None`.
const VD_RUN_HREF: &str = "/ui/sql/view-definitions/run";
const VD_EDITOR_FORM_ID: &str = "vd-editor-form";
/// The results heading and failure-prefix keys the View Definitions surface
/// renders through the partial — unchanged text, now passed explicitly
/// instead of hardcoded inside `partials/sql_run_results.html` (#839).
const VD_RESULTS_HEADING_KEY: &str = "vd-results-heading";
const VD_RUN_FAILED_KEY: &str = "vd-run-failed";

#[derive(Deserialize, Default)]
struct SqlVdQuery {
    vd: Option<String>,
    filter: Option<String>,
    saved: Option<String>,
    /// 1-based (#741). Kept as raw text rather than `Option<usize>` so a
    /// non-numeric value fails the `parse` below instead of the whole
    /// extractor — an invalid page falls back to 1, it never 400s.
    page: Option<String>,
}

/// Shapes a stored `ViewDefinition` resource into the editor's `(id, name,
/// json)` triple. `name` falls back to `id` when the resource itself has
/// none, the same default the rail's own summaries use.
fn shape_vd(vd: &serde_json::Value) -> (String, String, String) {
    let id = vd
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let name = vd
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&id)
        .to_string();
    let json = serde_json::to_string_pretty(vd).unwrap_or_default();
    (id, name, json)
}

/// Builds View Definitions' guided-form panel (#843) against an
/// already-parsed document — the same analysis `editor::render_body`'s
/// `pane=form` branch performs over HTTP (`editor::build_form_pane`), called
/// directly instead: the page's own render (and the Save-error re-render)
/// need the panel in place on first paint, not fetched after the fact.
/// `document`'s own `resourceType` decides [`editor::EditorFormPane::legend`]
/// exactly as `render_body` would (an empty legend override derives it), and
/// falls back to `"ViewDefinition"` — every caller on this page
/// hands it a `ViewDefinition`, valid or not, except the one Save-error path
/// where the submitted document parses but carries some other type; letting
/// the schema registry decide what that renders as (or fails to) is exactly
/// what `POST /ui/editor/render` already does for an arbitrary resource.
fn render_vd_form_pane(
    i18n: I18n,
    version: helios_fhir::FhirVersion,
    document: serde_json::Value,
) -> editor::EditorFormPane {
    let registry = helios_fhir_validator::packs::core_registry(version);
    let resource_type = document
        .get("resourceType")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("ViewDefinition")
        .to_string();
    // #843: this is the View Definitions page's own inline server-side
    // render, not the shared HTTP endpoint - the card needs `needs-js` so it
    // stays hidden until `theme.js` marks `<html class="js">` and
    // `vd-editor.js` wires it up.
    // #840: nothing hidden and the legend derived, exactly as before that
    // parameter pair existed.
    editor::build_form_pane(
        i18n,
        registry,
        version,
        resource_type,
        document,
        None,
        true,
        &[],
        "",
    )
}

/// The guided-form panel for text that failed to parse as JSON (#843,
/// generalized off its original View-Definitions-only copy in #840) — the
/// Save-error path's counterpart to `editor::render_body`'s own malformed-
/// document branch, shared by View Definitions and the SQL Query/SQL View
/// Details panel: the card still appears, with the invalid-JSON notice in
/// place of rows, and the user's exact text untouched. `legend` is the
/// caller's own choice ([`editor::Legend`]) rather than derived — text that
/// never parsed carries no `resourceType` for [`editor::Legend::resolve`] to
/// read.
fn invalid_form_pane(
    i18n: I18n,
    text: String,
    parse_error: String,
    legend: editor::Legend,
) -> editor::EditorFormPane {
    editor::EditorFormPane {
        i18n,
        rows: Vec::new(),
        document: text.clone(),
        pretty: text,
        error_count: 0,
        orphan_errors: Vec::new(),
        parse_error: Some(parse_error),
        focus_path: String::new(),
        auto_open_add: false,
        // Unread while `parse_error` is `Some` (the pane's own template
        // renders the invalid-JSON notice instead of the legend), but kept
        // faithful to the caller's own host nonetheless.
        legend,
        needs_js: true, // #843: every caller's own inline render, always needs-js
    }
}

/// The View Definitions page's guided-form panel for whichever document this
/// render selected (#843): the stored view, `?vd=new`'s starter document, or
/// `None` when there is no selection at all. Shared by the page's own GET
/// handler and, indirectly through the same shape, the Save-error re-render.
fn vd_form_pane_for_selection(
    i18n: I18n,
    version: helios_fhir::FhirVersion,
    is_new: bool,
    selected_value: Option<&serde_json::Value>,
) -> Option<editor::EditorFormPane> {
    if is_new {
        Some(render_vd_form_pane(
            i18n,
            version,
            sql_views::starter_view_definition_value(),
        ))
    } else {
        selected_value.map(|vd| render_vd_form_pane(i18n, version, vd.clone()))
    }
}

/// Resolves one candidate ViewDefinition id against this render's own page
/// of the server-paged rail search (#741) or, when it is off that page (a
/// different page, or a filter that excludes it), a direct read by id — the
/// same fallback the explicit `?vd=` selection has always used, now shared
/// with a stored `last` id that is off the visible page too, and with the
/// fallback candidate the page falls back to when neither an explicit nor a
/// stored selection resolves.
async fn resolve_vd_by_id(
    state: &WebState,
    rv: helios_fhir::FhirVersion,
    tenant: &str,
    id: &str,
    page_resources: &mut Vec<serde_json::Value>,
) -> Option<serde_json::Value> {
    if let Some(i) = page_resources
        .iter()
        .position(|vd| vd.get("id").and_then(serde_json::Value::as_str) == Some(id))
    {
        return Some(page_resources.swap_remove(i));
    }
    match state
        .conformance
        .read_resource("ViewDefinition", id, rv, tenant)
        .await
    {
        Ok(vd) => Some(vd),
        Err(error) => {
            // A stale or mistyped id is ordinary navigation, not an operator
            // concern — the caller falls back to its own no-selection or
            // next-candidate handling.
            tracing::debug!("ViewDefinition read failed for vd={id}: {error}");
            None
        }
    }
}

/// For View Definitions: the "Recently used" group's rows, resolved against
/// this render's own page of the server-paged rail search so
/// label/meta/href/current match what the list already shows. "The group
/// is never itself filtered by `?filter=`" falls out for free: an id the
/// current page/filter excludes is simply absent from `summaries` (and so
/// from `live`), so [`rail_state::RailState::resolve_recents`] renders it
/// from its stored `{name, meta}` snapshot instead.
fn resolve_vd_recents(
    rail: &rail_state::RailState,
    summaries: &[sql_views::VdSummary],
    selected_id: Option<&str>,
) -> Vec<rail_state::ResolvedRailEntry> {
    let live: std::collections::HashMap<String, rail_state::LiveRailItem> = summaries
        .iter()
        .map(|item| {
            (
                item.id.clone(),
                rail_state::LiveRailItem {
                    label: item.name.clone(),
                    meta: Some(item.resource.clone()),
                    count: None,
                    href: format!("/ui/sql/view-definitions?vd={}", item.id),
                    current: selected_id == Some(item.id.as_str()),
                },
            )
        })
        .collect();
    rail.resolve_recents(
        &live,
        |id| format!("/ui/sql/view-definitions?vd={id}"),
        None,
    )
}

/// `GET /ui/sql/view-definitions`: the rail is one page of a server-side
/// `ViewDefinition` search (#741) — `name:contains`, `_sort=name`,
/// `_count`/`_offset` — never the full-collection fetch this page used
/// before #741. The old in-memory filter also matched the resource-type
/// column (`ViewDefinition.resource`); that match is dropped on purpose, not
/// carried over into `name:contains` — it was never a documented part of
/// #649 and has no standard FHIR search expression (see
/// [`sql_views::rail_search_params`] for why).
async fn sql_view_definitions_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(query): Query<SqlVdQuery>,
    settings: rail_state::RequestSettings,
) -> Response {
    let filter = query.filter.unwrap_or_default();
    // Absent or non-numeric falls back to page 1 rather than 400ing — a
    // stale or hand-edited `?page=` is a navigation mistake, not an error.
    let page = query
        .page
        .as_deref()
        .and_then(|p| p.parse::<usize>().ok())
        .filter(|&p| p >= 1)
        .unwrap_or(1);
    let offset = (page - 1) * sql_views::PAGE_SIZE;

    // The rail is one page of a server-side search (#741): filtered by
    // `name:contains`, ordered by `_sort=name`, sliced by `_count`/`_offset`.
    // It never pulls the tenant's whole ViewDefinition collection into memory
    // the way the page's other consumers of `ConformanceSource::fetch`
    // (SearchParameters, Compartments, SQL Export) still do.
    let params = sql_views::rail_search_params(&filter);
    let (mut page_resources, has_next, degraded) = match state
        .conformance
        .search_page(
            "ViewDefinition",
            &params,
            sql_views::PAGE_SIZE,
            offset,
            rv.0,
            &rt.id,
        )
        .await
    {
        Ok(page) => (page.resources, page.has_next, None),
        Err(error) => {
            tracing::warn!("ViewDefinition search failed: {error}");
            (Vec::new(), false, Some(error))
        }
    };
    let summaries = sql_views::summarize(&page_resources);
    let prev_href =
        (page > 1).then(|| sql_views::page_href(&filter, query.vd.as_deref(), page - 1));
    let next_href = has_next.then(|| sql_views::page_href(&filter, query.vd.as_deref(), page + 1));

    let rail_before = settings.rail(rail_state::RailPage::ViewDefinitions, &rt.id);
    let is_new = query.vd.as_deref() == Some("new");
    let (selected, selected_value, rail) = if is_new {
        // "Create New" is never a selection (the `?vd=new` exception):
        // nothing here reads or writes `rails.viewDefinitions`.
        (
            Some(SelectedVd {
                id: String::new(),
                name: String::new(),
                json: sql_views::starter_view_definition(),
            }),
            None,
            rail_before,
        )
    } else if let Some(explicit_id) = query.vd.clone() {
        // An explicit `?vd=` always wins, resolved or not.
        match resolve_vd_by_id(&state, rv.0, &rt.id, &explicit_id, &mut page_resources).await {
            Some(vd) => {
                let (id, name, json) = shape_vd(&vd);
                let meta = vd
                    .get("resource")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                let entry = rail_state::RailEntry::with_snapshot(id.clone(), name.clone(), meta);
                let rail = record_snapshot_selection(
                    &state,
                    &settings.user_key,
                    &rt.id,
                    rail_state::RailPage::ViewDefinitions,
                    rail_before,
                    entry,
                )
                .await;
                (Some(SelectedVd { id, name, json }), Some(vd), rail)
            }
            None => {
                // A stale or mistyped explicit id is pruned from the
                // registry; the page itself keeps its current "no selection"
                // render either way.
                let rail = prune_stale_selection(
                    &state,
                    &settings.user_key,
                    &rt.id,
                    rail_state::RailPage::ViewDefinitions,
                    rail_before,
                    &explicit_id,
                )
                .await;
                (None, None, rail)
            }
        }
    } else {
        // No explicit selection: try the stored `last`, falling back to
        // the rail's first visible entry when there is none or it no
        // longer resolves — both silently: no write either way.
        let stored_id = rail_before.last.clone().filter(|id| !id.is_empty());
        let mut resolved = match stored_id.as_deref() {
            Some(id) => resolve_vd_by_id(&state, rv.0, &rt.id, id, &mut page_resources).await,
            None => None,
        };
        if resolved.is_none() {
            let fallback_id = summaries.first().map(|e| e.id.clone());
            resolved = match fallback_id.as_deref() {
                Some(id) => resolve_vd_by_id(&state, rv.0, &rt.id, id, &mut page_resources).await,
                None => None,
            };
        }
        match resolved {
            Some(vd) => {
                let (id, name, json) = shape_vd(&vd);
                (Some(SelectedVd { id, name, json }), Some(vd), rail_before)
            }
            None => (None, None, rail_before),
        }
    };

    let recent_entries =
        resolve_vd_recents(&rail, &summaries, selected.as_ref().map(|s| s.id.as_str()));

    // `?saved=1` (Save's own redirect) runs the just-stored definition
    // through $sql-run once, server-side, so the nojs path shows results
    // without a client request. Every other render's own state is `Empty`
    // — the empty notice this produces is what carries the client-driven
    // initial-load request, covering both an ordinary `?vd=` navigation and
    // `?vd=new`'s starter document (#752, generalized to the Library-backed
    // pages in #839).
    let i18n = I18n::new(locale);
    let run_state = match (&selected_value, query.saved.as_deref() == Some("1")) {
        (Some(vd), true) => match run_sql_preview(&state, vd, &[], rv.0, &rt.id).await {
            Ok((table, _rows, ms)) => RunResultsState::Success(table, ms),
            Err(error) => {
                let line = sql_views::extract_error_line(&error);
                RunResultsState::Failure(error, line)
            }
        },
        _ => RunResultsState::Empty,
    };
    // #843: the guided-form panel next to the JSON editor, built inline from
    // whichever document `selected` already resolved above.
    let form_pane = vd_form_pane_for_selection(i18n, rv.0, is_new, selected_value.as_ref());

    render(SqlViewDefinitionsPage {
        status: current_status(&state, rv.0, &rt),
        i18n,
        active_page: "sql-view-definitions",
        rail: summaries,
        filter,
        prev_href,
        next_href,
        degraded,
        selected,
        is_new,
        form_pane,
        run_results: RunResultsPartial {
            i18n,
            fragment: false,
            run_href: VD_RUN_HREF,
            form_id: VD_EDITOR_FORM_ID,
            heading_key: VD_RESULTS_HEADING_KEY,
            failed_key: VD_RUN_FAILED_KEY,
            export_href: None,
            state: run_state,
        },
        save_error: None,
        saved: query.saved.as_deref() == Some("1"),
        recent_entries,
        rail_page: rail_state::RailPage::ViewDefinitions.key(),
        max_recent: rail_state::MAX_RECENT,
    })
}

/// Runs `$sql-run` for a preview and times the call in whole milliseconds —
/// shared by every SQL on FHIR playground's own `?…saved=1` render and its
/// `POST …/run` fragment endpoint (View Definitions since #752; SQL Queries
/// and SQL Views since #839) so the row cap and the `{ $rows } rows ·
/// { $ms } ms` meta can never drift between callers. `resource` is a
/// ViewDefinition or a Library — `$sql-run` and [`sql_views::build_table`]
/// only need it to carry a `select`, which a Library's SQL-only content does
/// not, so its table is simply every column the query's own rows returned
/// (see `build_table`'s own doc comment). NF2: never logs `resource` itself
/// — a ViewDefinition's `constant[]` or a Library's embedded SQL can carry
/// PHI.
///
/// `bindings` (#841) supplies this run's values for a SQL Query's
/// `Library.parameter` declarations; every caller in this ticket passes an
/// empty slice — a ViewDefinition or an unparameterized Library has none to
/// supply, and the Parameters card that fills this in for a parameterized
/// SQL Query lands separately.
///
/// Returns the raw JSON rows alongside the stringified [`sql_views::
/// RunTable`] (#842/04): the Columns card's own type inference
/// ([`sql_libraries::analyze_columns`]) needs a column's *values*, not
/// their rendered cell text, to tell a JSON number from a JSON string —
/// distinctions [`sql_views::build_table`]'s own `cell_text` already
/// erases. Every caller that has no use for them (View Definitions) simply
/// ignores the second element.
async fn run_sql_preview(
    state: &WebState,
    resource: &serde_json::Value,
    bindings: &[SqlExportParameter],
    version: helios_fhir::FhirVersion,
    tenant: &str,
) -> Result<(sql_views::RunTable, Vec<serde_json::Value>, u64), String> {
    let start = std::time::Instant::now();
    let rows = state
        .conformance
        .sql_run(resource, bindings, sql_views::RUN_LIMIT, version, tenant)
        .await?;
    // `Instant::elapsed` millis fits `u64` for anything short of 584 million
    // years; `unwrap_or(u64::MAX)` is just a total function, never reachable.
    let ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    let table = sql_views::build_table(resource, &rows);
    Ok((table, rows, ms))
}

#[derive(Deserialize)]
struct SqlVdSaveForm {
    /// Empty for a create; the stored id for an update.
    #[serde(default)]
    id: String,
    json: String,
    /// `save` or `duplicate` — the two submit buttons of the one form.
    #[serde(default)]
    action: String,
}

/// Saves the editor's JSON through the server's own FHIR API and bounces back
/// to the page with the stored view selected. A parse or server error
/// re-renders the page with the submitted text preserved, so nothing typed is
/// lost. Plain form semantics throughout — no JavaScript required.
async fn sql_view_definitions_save(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    axum::Form(form): axum::Form<SqlVdSaveForm>,
) -> Response {
    let error_page = |save_error: String, json: String, is_new: bool, id: String| {
        // #843: the guided-form panel keeps up with whatever the user
        // submitted — parses it exactly like `editor::render_body` would, so
        // a Save that failed only because of the resource's own contents
        // (not its JSON syntax) still shows a form built from it; text that
        // fails to parse gets the same invalid-JSON notice `render_body`'s
        // own malformed-document branch renders.
        let form_pane = match serde_json::from_str::<serde_json::Value>(&json) {
            Ok(document) => render_vd_form_pane(I18n::new(locale), rv.0, document),
            Err(parse_error) => invalid_form_pane(
                I18n::new(locale),
                json.clone(),
                parse_error.to_string(),
                editor::Legend::ViewDefinition,
            ),
        };
        SqlViewDefinitionsPage {
            status: current_status(&state, rv.0, &rt),
            i18n: I18n::new(locale),
            active_page: "sql-view-definitions",
            rail: Vec::new(),
            filter: String::new(),
            prev_href: None,
            next_href: None,
            degraded: None,
            selected: Some(SelectedVd {
                name: if is_new { String::new() } else { id.clone() },
                id,
                json,
            }),
            is_new,
            form_pane: Some(form_pane),
            // A form-validation error re-renders in place: nothing has run
            // server-side, so this render's own results are `Empty` — same
            // as any other render with no `?saved=1`. The submitted text is
            // still whatever the user typed (kept, not lost), so the
            // `Empty` arm's own load trigger runs that same text through the
            // live preview once the page opens.
            run_results: RunResultsPartial {
                i18n: I18n::new(locale),
                fragment: false,
                run_href: VD_RUN_HREF,
                form_id: VD_EDITOR_FORM_ID,
                heading_key: VD_RESULTS_HEADING_KEY,
                failed_key: VD_RUN_FAILED_KEY,
                export_href: None,
                state: RunResultsState::Empty,
            },
            save_error: Some(save_error),
            saved: false,
            // A form-validation error re-renders in place, not a navigation —
            // there is nothing new to record and no rail to repaint.
            recent_entries: Vec::new(),
            rail_page: rail_state::RailPage::ViewDefinitions.key(),
            max_recent: rail_state::MAX_RECENT,
        }
    };

    let duplicate = form.action == "duplicate";
    let mut resource: serde_json::Value = match serde_json::from_str(form.json.trim()) {
        Ok(value) => value,
        Err(e) => {
            return render(error_page(
                format!("invalid JSON: {e}"),
                form.json,
                form.id.is_empty(),
                form.id,
            ));
        }
    };
    if resource
        .get("resourceType")
        .and_then(serde_json::Value::as_str)
        != Some("ViewDefinition")
    {
        return render(error_page(
            "the document must have resourceType \"ViewDefinition\"".to_string(),
            form.json,
            form.id.is_empty(),
            form.id,
        ));
    }

    let id = if duplicate || form.id.is_empty() {
        // A create must not carry a stored id; a duplicate also gets a fresh
        // name so the two are tellable apart in the rail.
        if let Some(map) = resource.as_object_mut() {
            map.remove("id");
            if duplicate && let Some(name) = map.get("name").and_then(serde_json::Value::as_str) {
                let copy = format!("{name}_copy");
                map.insert("name".to_string(), serde_json::Value::String(copy));
            }
        }
        None
    } else {
        // The path id is authoritative for an update; the body must agree.
        if let Some(map) = resource.as_object_mut() {
            map.insert("id".to_string(), serde_json::Value::String(form.id.clone()));
        }
        Some(form.id.clone())
    };

    match state
        .conformance
        .save_resource("ViewDefinition", id.as_deref(), resource, rv.0, &rt.id)
        .await
    {
        Ok(stored) => {
            let stored_id = stored
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            axum::response::Redirect::to(&format!(
                "/ui/sql/view-definitions?vd={stored_id}&saved=1"
            ))
            .into_response()
        }
        Err(error) => render(error_page(error, form.json, id.is_none(), form.id)),
    }
}

#[derive(Deserialize)]
struct SqlVdRunForm {
    /// The editor's full text, exactly as posted — never reformatted or
    /// re-serialized before either parsing it or handing it to `$sql-run`:
    /// the editor is the source of truth.
    json: String,
}

/// `POST /ui/sql/view-definitions/run` (#752): the playground's live
/// preview fragment. Unlike the page's own `?saved=1` render, this always
/// runs the editor's *posted* text — saved or not — through the same
/// [`run_sql_preview`] helper, and renders `partials/sql_run_results.html`
/// in fragment mode instead of a full page. The editor's textarea posts to
/// it on `input changed delay:500ms`, and the results region's own empty
/// shell reposts to it on the page's `load` event — see
/// `templates/partials/sql_run_results.html`'s header comment.
///
/// Always answers `200` except for a malformed request body — a missing
/// `json` field, which `axum::Form`'s own rejection turns into a `4xx`
/// before this handler runs (`422 Unprocessable Entity` in practice for a
/// `POST` body deserialize failure, not `400`; either way a real error
/// status, not this endpoint's `2xx` fragment contract). NF3: htmx does not
/// swap `4xx`/`5xx` responses by default, so a run failure or invalid JSON
/// must not surface as an HTTP error — both render the notice-only fragment
/// instead, leaving the client's previous results table untouched.
async fn sql_view_definitions_run(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    axum::Form(form): axum::Form<SqlVdRunForm>,
) -> Response {
    let i18n = I18n::new(locale);
    let respond = |run_state: RunResultsState| {
        render(RunResultsPartial {
            i18n,
            fragment: true,
            run_href: VD_RUN_HREF,
            form_id: VD_EDITOR_FORM_ID,
            heading_key: VD_RESULTS_HEADING_KEY,
            failed_key: VD_RUN_FAILED_KEY,
            export_href: None,
            state: run_state,
        })
    };

    // A JSON parse failure never reaches $sql-run. NF2: never log
    // `form.json` itself — a ViewDefinition's `constant[]` can carry PHI.
    let view_definition: serde_json::Value = match serde_json::from_str(form.json.trim()) {
        Ok(value) => value,
        Err(error) => {
            return respond(RunResultsState::Failure(
                format!("invalid JSON: {error}"),
                None,
            ));
        }
    };

    let run_state = match run_sql_preview(&state, &view_definition, &[], rv.0, &rt.id).await {
        Ok((table, _rows, ms)) => {
            tracing::debug!(rows = table.rows.len(), ms, "ran a ViewDefinition preview");
            RunResultsState::Success(table, ms)
        }
        Err(error) => {
            let line = sql_views::extract_error_line(&error);
            RunResultsState::Failure(error, line)
        }
    };
    respond(run_state)
}

/// One [`helios_sof::lint::Fix`], translated (#821): the fix's own JSON
/// shape (`kind`, `pointer`, and whichever of `to`/`value` that `kind`
/// carries — [`helios_sof::lint::Fix`]'s own `#[serde(tag = "kind")]`
/// representation, flattened in here unchanged) plus a `label` rendered from
/// the matching `vd-fix-*` catalog key, ready for a button or menu item that
/// offers this fix with no further lookup on the browser's part.
#[derive(serde::Serialize)]
struct LintFixDto {
    #[serde(flatten)]
    fix: helios_sof::lint::Fix,
    label: String,
}

/// One [`helios_sof::lint::Diagnostic`], translated (#821): every field
/// except `message` passes through unchanged (`args` included — a client
/// that wants the raw value behind a translated sentence, e.g. `args.name`
/// for `undeclared-constant`, still has it). `message` here is **not**
/// [`helios_sof::lint::Diagnostic::message`] (that field is always English —
/// `$sql-run`, `sof-cli`, and `pysof` all use it verbatim); it is the
/// negotiated-locale rendering of `code` + `args` against the `vd-lint-*`
/// catalog, matching #821's split: `helios_sof` never localizes, only this
/// handler does.
#[derive(serde::Serialize)]
struct LintDiagnosticDto {
    pointer: String,
    message: String,
    severity: helios_sof::lint::Severity,
    code: helios_sof::lint::DiagnosticCode,
    span: Option<helios_sof::lint::Span>,
    args: std::collections::BTreeMap<String, String>,
    fixes: Vec<LintFixDto>,
}

/// The kebab-case wire string [`helios_sof::lint::DiagnosticCode`] already
/// serializes as (`fhirpath-syntax`, `unknown-key`, ...) — read back through
/// `serde_json` rather than hand-duplicating the mapping, so the `vd-lint-*`
/// catalog key this builds can never drift from the lint's own JSON `code`.
/// `crates/sof/src/error.rs` carries the identical trick for
/// `$sql-run`'s `422` coding, one crate over and with no code to share it
/// through.
fn diagnostic_catalog_key(code: helios_sof::lint::DiagnosticCode) -> String {
    match serde_json::to_value(code) {
        Ok(serde_json::Value::String(wire)) => format!("vd-lint-{wire}"),
        _ => unreachable!("DiagnosticCode serializes to a JSON string"),
    }
}

/// The translated `message` for one diagnostic (#821): `code` + `args`
/// against the matching `vd-lint-*` catalog key.
///
/// `vd-lint-missing-required` and `vd-lint-wrong-type` select their wording
/// on `$variant` — set only by the two diagnostics `check_constant_value`
/// reports for a constant's `value[x]` choice, never by the generic
/// `missing-required`/`wrong-type` diagnostics (a plain missing/wrong-typed
/// key), whose own `args` (per `helios_sof::lint`'s contract) has no
/// `variant` at all. `fluent-templates`' selector lookup fails the *entire*
/// message when the selector variable is completely absent from the args
/// map — unlike a present-but-unmatched value, which falls to `*[other]`
/// normally (see `t_args_selector_falls_back_only_when_the_variable_is_present_but_unmatched`
/// in `i18n.rs`) — so an empty `variant` is added here before rendering
/// those two codes specifically, for the lookup only: the diagnostic's own
/// `args` in the JSON response (`LintDiagnosticDto::args`) is untouched.
fn translate_diagnostic_message(
    i18n: I18n,
    code: helios_sof::lint::DiagnosticCode,
    args: &std::collections::BTreeMap<String, String>,
) -> String {
    use helios_sof::lint::DiagnosticCode;
    let key = diagnostic_catalog_key(code);
    if matches!(
        code,
        DiagnosticCode::MissingRequired | DiagnosticCode::WrongType
    ) && !args.contains_key("variant")
    {
        let mut with_variant = args.clone();
        with_variant.insert("variant".to_string(), String::new());
        i18n.t_args(&key, &with_variant)
    } else {
        i18n.t_args(&key, args)
    }
}

/// The last segment of an RFC 6901 pointer, unescaped (`~1` → `/` before
/// `~0` → `~`, undoing the encoding's own order) — what `vd-fix-remove-key`'s
/// `$key` names: the property being removed, not its full path.
fn pointer_last_segment(pointer: &str) -> String {
    pointer
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .replace("~1", "/")
        .replace("~0", "~")
}

/// The translated label for one fix, from its matching `vd-fix-*` catalog
/// key — see the module-level `## ViewDefinition lint messages and fixes`
/// section of `main.ftl` for the exact wording and argument per kind.
fn fix_label(i18n: I18n, fix: &helios_sof::lint::Fix) -> String {
    use helios_sof::lint::Fix;
    match fix {
        Fix::RenameKey { to, .. } => i18n.t_arg("vd-fix-rename-key", "to", to.clone()),
        Fix::RemoveKey { pointer } => {
            i18n.t_arg("vd-fix-remove-key", "key", pointer_last_segment(pointer))
        }
        Fix::SetString { value, .. } => i18n.t_arg("vd-fix-set-string", "value", value.clone()),
        // `Fix` is `#[non_exhaustive]`: a variant this crate doesn't know a
        // `vd-fix-*` key for yet must not crash the handler. Fall back to
        // the fix's own `kind` tag rather than a hand-picked English
        // sentence, so a `helios-sof` upgrade alone still renders
        // *something* until this match (and the catalog) catch up.
        _ => serde_json::to_value(fix)
            .ok()
            .and_then(|value| value.get("kind")?.as_str().map(str::to_owned))
            .unwrap_or_default(),
    }
}

/// Structural + FHIRPath-syntax lint for the ViewDefinition editor's async
/// CodeMirror 6 linter (`vd-editor.js`) (#753, #820, #821). Delegates
/// entirely to [`helios_sof::lint::lint_view_definition`] for the checks
/// themselves — this handler only decodes the request body, translates each
/// diagnostic and fix into the negotiated locale (`?lang=` / `hfs_lang`
/// cookie / `Accept-Language`, same policy as every other page — see
/// [`i18n`]), and shapes the response; it never touches storage, the
/// tenant, or the configured FHIR version, because the lint itself is
/// purely structural and version-agnostic.
///
/// Plain JSON in, JSON out — no htmx swap involved, matching the precedent
/// `/ui/editor/expand` already sets for a browser-facing JSON endpoint that
/// exists to support an editor rather than to mirror the FHIR REST surface.
/// The body is read as raw bytes (not the `Json` extractor) so a malformed
/// body reports the lint's exact `{"error": "..."}` shape instead of axum's
/// generic rejection body.
async fn sql_view_definitions_lint(locale: RequestLocale, body: Bytes) -> Response {
    let i18n = I18n::new(locale);
    let doc: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(doc) => doc,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid JSON: {error}") })),
            )
                .into_response();
        }
    };

    let diagnostics: Vec<LintDiagnosticDto> = helios_sof::lint::lint_view_definition(&doc)
        .into_iter()
        .map(|diagnostic| LintDiagnosticDto {
            message: translate_diagnostic_message(i18n, diagnostic.code, &diagnostic.args),
            fixes: diagnostic
                .fixes
                .into_iter()
                .map(|fix| LintFixDto {
                    label: fix_label(i18n, &fix),
                    fix,
                })
                .collect(),
            pointer: diagnostic.pointer,
            severity: diagnostic.severity,
            code: diagnostic.code,
            span: diagnostic.span,
            args: diagnostic.args,
        })
        .collect();
    // NF2: never log the document itself — a ViewDefinition's `constant[]`
    // can carry PHI — only how many diagnostics it produced.
    tracing::debug!(
        diagnostic_count = diagnostics.len(),
        "linted a ViewDefinition document"
    );

    Json(serde_json::json!({ "diagnostics": diagnostics })).into_response()
}

/// What tells the SQL Queries workspace apart from SQL Views: both edit and
/// run `Library` resources, differing only in the `LibraryTypesCodes` code,
/// their route, their labels, the page key each writes/reads its own
/// `rails.<page>` record under (SQL Queries and SQL Views never contaminate
/// each other's rail state), and everything else that sets the two apart in
/// the editor-first layout the shared template renders (#839) — an SVG
/// icon, a type chip, and the per-type copy for the rail's empty state, the
/// editor and results headings, the failure notice's prefix, the filter
/// placeholder, and the "no Library yet" empty state. None of it is ever
/// hardcoded in the shared handler or template — a template reads only
/// fields off this table, never an `if` comparing `code` against a string.
///
/// Every field below names a distinct per-kind i18n key or asset — never a
/// shared one a template branches on with `if code == "..."` (NF5).
struct LibraryKind {
    code: &'static str,
    base_href: &'static str,
    active_page: &'static str,
    title_key: &'static str,
    lede_key: &'static str,
    new_title_key: &'static str,
    all_heading_key: &'static str,
    page: rail_state::RailPage,
    /// The filter rail search box's placeholder.
    filter_placeholder_key: &'static str,
    /// The rail's "no Libraries of this kind" text, under the "All …" group
    /// heading.
    rail_empty_key: &'static str,
    /// The SQL editor card's `<h3>` heading.
    editor_heading_key: &'static str,
    /// The `$sql-run` preview card's `<h3>` heading, read into
    /// [`RunResultsPartial::heading_key`].
    results_heading_key: &'static str,
    /// The failure notice's message-prefix key, read into
    /// [`RunResultsPartial::failed_key`].
    failed_key: &'static str,
    /// The "no selection" empty state's card heading and lede, shown when
    /// the store holds no Library of this kind and none is being created.
    empty_title_key: &'static str,
    empty_lede_key: &'static str,
    /// The title row's `.tag--type` chip text (#839) — its own key, distinct
    /// from `title_key`: the chip names one document's type ("SQL Query"),
    /// the page head above the rail names the collection ("SQL Queries").
    chip_key: &'static str,
    /// The title row's embedded type icon (#839). `include_str!`, not
    /// `{% include %}`, because Askama cannot include a template at a path
    /// chosen at render time.
    icon_svg: &'static str,
    /// This kind's `POST …/run` fragment endpoint — the SQL textarea's own
    /// `hx-post` target and, via [`RunResultsPartial::run_href`], the
    /// results region's own initial-load repost target.
    run_href: &'static str,
    /// Whether this kind's results card offers an "Export as files" action
    /// (SQL Queries only — a SQL View has no `subject=Library/{id}` export
    /// shape of its own).
    offers_export: bool,
    /// Whether this kind's Library declares `Library.parameter[]` at all
    /// (#841): `true` for SQL Query, `false` for SQL View, whose SQLView
    /// profile fixes `Library.parameter` to `0..0`. The page renders the
    /// Parameters card (and gates Save on an empty `parameter[]`) from this
    /// field alone — never an `if` comparing `code` against a string,
    /// matching [`LibraryKind`]'s own NF5 rule.
    declares_parameters: bool,
    /// This kind's document-mutation endpoint (`POST …/document`, #841/#842):
    /// the Parameters card's *Add parameter*/*Declare* controls' own, and
    /// (#842) the Tables panel's *Add table*/*Remove* controls' own,
    /// `hx-post`/`formaction` target.
    document_href: &'static str,
    /// The Columns skeleton card's own meta line (#842) — "what the query
    /// produces" / "what the view produces". #842/03 only lays out the
    /// skeleton (heading, a fixed `0` count, this meta line, and
    /// `columns_empty_key`'s own empty-state body); a follow-up fills the
    /// card itself in with the last good run's columns.
    columns_meta_key: &'static str,
    /// The Columns skeleton's own empty-state body (#842) — "Run the query
    /// to see its columns." / "Run the view to see its columns."
    columns_empty_key: &'static str,
}

const SQL_QUERY_KIND: LibraryKind = LibraryKind {
    code: "sql-query",
    base_href: "/ui/sql/queries",
    active_page: "sql-queries",
    title_key: "sql-queries-title",
    lede_key: "sql-queries-lede",
    new_title_key: "sql-queries-new-title",
    all_heading_key: "sql-queries-rail-all-heading",
    page: rail_state::RailPage::SqlQueries,
    filter_placeholder_key: "sql-queries-filter",
    rail_empty_key: "sql-queries-rail-empty",
    editor_heading_key: "sql-queries-editor-heading",
    results_heading_key: "sql-queries-results-heading",
    failed_key: "sql-queries-run-failed",
    empty_title_key: "sql-queries-empty-title",
    empty_lede_key: "sql-queries-empty-lede",
    chip_key: "sql-queries-chip",
    icon_svg: include_str!("../templates/icons/code.svg"),
    run_href: "/ui/sql/queries/run",
    offers_export: true,
    declares_parameters: true,
    document_href: "/ui/sql/queries/document",
    columns_meta_key: "lib-columns-meta-query",
    columns_empty_key: "lib-columns-empty-query",
};

const SQL_VIEW_KIND: LibraryKind = LibraryKind {
    code: "sql-view",
    base_href: "/ui/sql/views",
    active_page: "sql-views",
    title_key: "sql-views-title",
    lede_key: "sql-views-lede",
    new_title_key: "sql-views-new-title",
    all_heading_key: "sql-views-rail-all-heading",
    page: rail_state::RailPage::SqlViews,
    filter_placeholder_key: "sql-views-filter",
    rail_empty_key: "sql-views-rail-empty",
    editor_heading_key: "sql-views-editor-heading",
    results_heading_key: "sql-views-results-heading",
    failed_key: "sql-views-run-failed",
    empty_title_key: "sql-views-empty-title",
    empty_lede_key: "sql-views-empty-lede",
    chip_key: "sql-views-chip",
    icon_svg: include_str!("../templates/icons/layers-platforms.svg"),
    run_href: "/ui/sql/views/run",
    offers_export: false,
    declares_parameters: false,
    document_href: "/ui/sql/views/document",
    columns_meta_key: "lib-columns-meta-view",
    columns_empty_key: "lib-columns-empty-view",
};

/// The Library-backed pages' one editable form's id — matches
/// `#lib-editor-form` in `pages/sql-library.html`. Shared by both kinds
/// (unlike `LibraryKind::run_href`, which differs per kind): the `/run`
/// fragment's `Empty` shell reads it via `hx-include` ([`RunResultsPartial::
/// form_id`]).
const LIB_EDITOR_FORM_ID: &str = "lib-editor-form";

/// The "Export as files" action's `href` for a Library of `kind` with the
/// given `id` (empty for an unsaved document) — `Some` only for a kind that
/// offers Export (SQL Queries) with a saved, non-empty id (#839); `None`
/// otherwise, which the results partial renders as nothing.
fn export_href(kind: &LibraryKind, id: &str) -> Option<String> {
    (kind.offers_export && !id.is_empty())
        .then(|| format!("/ui/sql/export/new?subject=Library/{id}"))
}

/// The title row's status chip class (#839): `active`/`draft`/`retired` for
/// those exact FHIR publication-status codes, `unknown` — one of the same
/// four `.tag--*` classes `app.css` already defines — for anything else,
/// including an absent status. The chip's own text is always `status`
/// verbatim (possibly empty), never replaced by this class.
fn status_tag_class(status: &str) -> &'static str {
    match status {
        "active" => "active",
        "draft" => "draft",
        "retired" => "retired",
        _ => "unknown",
    }
}

// ---------------------------------------------------------------------
// Parameters card (#841): a SQL Query's declared `Library.parameter[use=in]`
// values for the live run, the placeholders the SQL uses but does not
// declare, and the `Add parameter`/`Declare` mutations that write a new
// declaration into the Details document via `POST …/document`.
// ---------------------------------------------------------------------

/// One declared parameter's live-run value, shaped to match
/// `partials/sql_parameter_fields.html`'s own field access
/// (#837/#841) — the same field names [`crate::sql_export`]'s own
/// `ParamFieldView` exposes, duplicated here rather than shared because the
/// two live in different modules and Askama's macro call resolves fields
/// structurally, not through a trait. `error` is always `None`: unlike SQL
/// Export's own job-creation validation, a value that fails to bind its
/// declared type is left to `$sql-run`'s own error message (#841),
/// never flagged on the field itself. `required` is likewise always
/// `false` — see [`analyze_params`]'s own doc comment for why the HTML5
/// attribute must stay off even for a parameter with no default.
struct LibParamFieldView {
    name: String,
    type_code: String,
    default: Option<String>,
    value: String,
    required: bool,
    error: Option<String>,
}

/// [`analyze_params`]'s pure result: everything the Parameters card, the
/// `/run` fragment, and the `document` endpoint each need from one
/// (document, SQL, submitted values) triple, computed exactly once so none
/// of them can disagree about the signature, the bindings, or which
/// required parameters are still unfilled (the architecture note's "una
/// sola función").
struct ParamsAnalysis {
    fields: Vec<LibParamFieldView>,
    /// `:name` placeholders the SQL uses that `fields` does not declare
    /// (#841), in scanner order — see
    /// [`sql_libraries::undeclared_placeholder_names`].
    hints: Vec<String>,
    /// The card's own signature (#841) — see [`sql_libraries::params_signature`].
    signature: String,
    /// One binding per declared parameter with a non-empty submitted value
    /// (#841) — a blank value on a defaulted parameter is omitted
    /// so `$sql-run` applies its own default, never sent as an empty string.
    bindings: Vec<SqlExportParameter>,
    /// The declared, default-less parameters with no non-empty submitted
    /// value (#841) — non-empty exactly when the run this render
    /// backs must show the "waiting" notice instead of calling `$sql-run`.
    missing_required: Vec<String>,
}

/// Analyzes one `document`/`sql`/submitted-`values` triple for the
/// Parameters card (#841): reads `document`'s declared `use=in` parameters
/// ([`sql_libraries::parameters`]), scans `sql` for placeholders it does not
/// declare ([`sql_libraries::undeclared_placeholder_names`]), and resolves
/// each declared field's live-run value — a `values` entry when present
/// (even blank, echoing a deliberate clear rather than snapping back to the
/// default — the same rule [`crate::sql_export::ParamFieldView`]'s own doc
/// comment states), the declared default otherwise.
///
/// Every field's `required` attribute is deliberately left `false`
/// (`LibParamFieldView::required`) even for a parameter with no default:
/// these inputs are `form="lib-editor-form"`, the *same* form Save submits,
/// and Save must always succeed regardless of what the Parameters card
/// holds (#841 — values are session-only and never block Save). The
/// "waiting" notice this analysis's own `missing_required` drives is what
/// actually enforces the requirement, entirely server-side, never through
/// HTML5's native `required` validation.
fn analyze_params(
    document: &serde_json::Value,
    sql: &str,
    values: &std::collections::HashMap<String, String>,
) -> ParamsAnalysis {
    let declared = sql_libraries::parameters(document);
    let hints = sql_libraries::undeclared_placeholder_names(sql, &declared);
    let signature = sql_libraries::params_signature(&declared, &hints);

    let mut fields = Vec::with_capacity(declared.len());
    let mut bindings = Vec::new();
    let mut missing_required = Vec::new();
    for p in &declared {
        let submitted = values.get(&p.name).map(String::as_str);
        let value = submitted
            .unwrap_or_else(|| p.default.as_deref().unwrap_or(""))
            .to_string();
        match submitted.map(str::trim) {
            Some(trimmed) if !trimmed.is_empty() => bindings.push(SqlExportParameter {
                name: p.name.clone(),
                type_code: p.type_code.clone(),
                value: trimmed.to_string(),
            }),
            _ if p.default.is_none() => missing_required.push(p.name.clone()),
            // Empty (or never submitted) with a default: the server applies
            // it by omission, so nothing is sent and nothing is missing.
            _ => {}
        }
        fields.push(LibParamFieldView {
            name: p.name.clone(),
            type_code: p.type_code.clone(),
            default: p.default.clone(),
            value,
            required: false,
            error: None,
        });
    }
    ParamsAnalysis {
        fields,
        hints,
        signature,
        bindings,
        missing_required,
    }
}

/// The "waiting" notice's own text (#841): every missing required
/// parameter's name, `:`-prefixed and comma-joined ("`:ward`" or "`:ward,
/// :city`"), interpolated into the `lib-run-waiting` catalog message.
/// `missing_required` must be non-empty — every caller only reaches this
/// once it has confirmed that itself.
fn waiting_message(i18n: &I18n, missing_required: &[String]) -> String {
    let names = missing_required
        .iter()
        .map(|name| format!(":{name}"))
        .collect::<Vec<_>>()
        .join(", ");
    i18n.t_arg("lib-run-waiting", "names", names)
}

/// The Parameters card's own re-submitted `Add parameter` state (#841): the
/// text/select values [`sql_library_document`] echoes back into the
/// `<details>` on a rejected `add-parameter`, and the validation message
/// alongside them. `Default` is every other render's own state — closed,
/// empty, no error.
#[derive(Default)]
struct AddParamFormState {
    name: String,
    type_code: String,
    open: bool,
    error: Option<String>,
}

/// [`build_params_card`]'s presentation-only options — everything about a
/// render that [`analyze_params`] itself has no opinion on, grouped so the
/// function's own signature does not grow a parameter per caller. `Default`
/// is the page's own inline render: no OOB swap, no `data-document`, the
/// `Add parameter` panel closed and clean.
#[derive(Default)]
struct ParamsCardOptions {
    add: AddParamFormState,
    /// `true` only for the `/run` fragment's own OOB companion (#841) — the
    /// one context where this card rides alongside another element's direct
    /// target swap rather than being the response (or part of the page)
    /// itself. See `partials/sql_parameters_card.html`'s own header comment.
    oob: bool,
    /// The `document` endpoint's own `HX-Request` success response carries
    /// the updated Details document here (#841) — `host.setDoc`
    /// (`sql-library-panels.js`) applies it as one undoable transaction.
    /// `None` everywhere else, including a rejected `add-parameter`.
    data_document: Option<String>,
}

/// Builds the Parameters card (#841) from an already-computed
/// [`ParamsAnalysis`] plus `options`' presentation state — a caller that
/// also needs `analysis.bindings`/`analysis.missing_required` (the `/run`
/// fragment, the `document` endpoint) computes it once with
/// [`analyze_params`] and passes it in here; a caller that only needs the
/// card itself (the page's own render) may do the same and simply ignore
/// the rest. `kind` supplies the card's static wiring
/// (`run_href`/`document_href`); `kind.declares_parameters` is the caller's
/// own gate for whether to build this at all — this function itself always
/// builds one, trusting the caller already checked.
fn build_params_card(
    i18n: I18n,
    kind: &LibraryKind,
    analysis: ParamsAnalysis,
    options: ParamsCardOptions,
) -> LibParamsCard {
    LibParamsCard {
        i18n,
        run_href: kind.run_href,
        document_href: kind.document_href,
        fields: analysis.fields,
        hints: analysis.hints,
        type_options: helios_sof::sqlquery::BINDABLE_PARAMETER_TYPES,
        signature: analysis.signature,
        add_name: options.add.name,
        add_type: options.add.type_code,
        add_open: options.add.open,
        add_error: options.add.error,
        oob: options.oob,
        data_document: options.data_document,
    }
}

/// `partials/sql_parameters_card.html`'s render surface (#841): the live-run
/// values row (`sql_parameter_fields.html`, one `.field` per declared
/// `Library.parameter[use=in]`), the undeclared-placeholder hints, the
/// `params_sig` signature, and the `Add parameter` panel — built once by
/// [`build_params_card`] and shared by the page's own first paint, the
/// `/run` fragment's OOB companion, and the `document` endpoint's own
/// response.
#[derive(Template)]
#[template(path = "partials/sql_parameters_card.html")]
struct LibParamsCard {
    i18n: I18n,
    /// This kind's `POST …/run` fragment endpoint (#841) — the card's own
    /// `hx-post`, fired 500ms after the last keystroke in any of its value
    /// fields (they bubble `input` up to this element).
    run_href: &'static str,
    /// This kind's `POST …/document` endpoint (#841) — the `Add parameter`/
    /// `Declare` controls' own `hx-post`/`formaction`.
    document_href: &'static str,
    fields: Vec<LibParamFieldView>,
    /// Undeclared placeholder hints, in scanner order (#841).
    hints: Vec<String>,
    /// `Library.parameter[use=in]`'s own type picker options
    /// (`helios_sof::sqlquery::BINDABLE_PARAMETER_TYPES`), for the `Add
    /// parameter` `<select>`.
    type_options: &'static [&'static str],
    /// The `params_sig` hidden field's own value (#841).
    signature: String,
    add_name: String,
    add_type: String,
    add_open: bool,
    add_error: Option<String>,
    /// `true` only for the `/run` fragment's own OOB companion — see
    /// [`ParamsCardOptions::oob`].
    oob: bool,
    /// The updated Details document, present only on the `document`
    /// endpoint's own successful `HX-Request` response — see
    /// [`ParamsCardOptions::data_document`].
    data_document: Option<String>,
}

// ---------------------------------------------------------------------
// Tables panel (#842): *Reads from* (a SQL Query/SQL View's declared
// `relatedArtifact[depends-on]` table dependencies, resolved the same way
// `$sql-run`'s own graph walk resolves them) and *Used by* (which other
// Libraries and SQL Export jobs depend on this one). The pure half of this
// — reading declarations, matching a reference against an already-fetched
// candidate, and the `Add table`/`Remove` document mutations — lives in
// `sql_libraries`; everything here is the I/O half (`ConformanceSource::
// read_resource`/`search_page`/`fetch`, `sql_export::jobs_for_used_by`) plus
// the presentation shaping `sql_libraries` itself never does (this module
// never localizes, matching every other card's own split).
// ---------------------------------------------------------------------

/// Resolves one `relatedArtifact[depends-on].resource` reference against
/// storage, mirroring `crates/rest/.../graph.rs`'s own `StorageArtifactFetcher::fetch`
/// (#842's own resolution imitates, never replaces, the server's own):
/// a `Type/id` reference reads that resource
/// directly; an absolute canonical URL is searched for among stored
/// ViewDefinitions first (`search_page("ViewDefinition", url=…)`, tried
/// first since it is the more common dependency, exactly as the server's
/// own fetcher tries it first), then — only if that finds nothing — among
/// `libraries` (the rail's own already-fetched Library list, reused rather
/// than fetched again, #842/NF1). A search or read failure degrades to
/// [`sql_libraries::TableTarget::NotFound`] — the same "nothing answers to
/// this" row a genuine 404 gets, since neither the API nor this card has
/// anywhere else to explain the difference.
///
/// Also returns the resolved artifact's own raw JSON alongside its
/// [`sql_libraries::TableTarget`] classification (#842/04) — `None` only
/// for `TableTarget::NotFound`, since nothing was ever fetched. *Reads
/// from* only needs the classification; the Columns card's own origin
/// lookup ([`resolve_view_definition_dependencies`]) is what needs the
/// document itself, to read its `select[].column[]` list.
async fn resolve_table_reference(
    state: &WebState,
    version: helios_fhir::FhirVersion,
    tenant: &str,
    reference: &str,
    libraries: &[serde_json::Value],
) -> (sql_libraries::TableTarget, Option<serde_json::Value>) {
    use sql_libraries::{DependencyLookup, TableTarget};
    match sql_libraries::dependency_lookup(reference) {
        DependencyLookup::TypeId { resource_type, id } => {
            match state
                .conformance
                .read_resource(resource_type, &id, version, tenant)
                .await
            {
                Ok(artifact) => {
                    let target = sql_libraries::classify_table_artifact(resource_type, &artifact);
                    (target, Some(artifact))
                }
                Err(_) => (TableTarget::NotFound, None),
            }
        }
        DependencyLookup::Canonical { canonical, .. } => {
            let params = vec![("url".to_string(), canonical)];
            if let Ok(page) = state
                .conformance
                .search_page(
                    "ViewDefinition",
                    &params,
                    sql_views::PAGE_SIZE,
                    0,
                    version,
                    tenant,
                )
                .await
                && let Some(vd) = page
                    .resources
                    .iter()
                    .find(|vd| sql_libraries::matches_reference(reference, "ViewDefinition", vd))
            {
                let target = sql_libraries::classify_table_artifact("ViewDefinition", vd);
                return (target, Some(vd.clone()));
            }
            match libraries
                .iter()
                .find(|lib| sql_libraries::matches_reference(reference, "Library", lib))
            {
                Some(lib) => {
                    let target = sql_libraries::classify_table_artifact("Library", lib);
                    (target, Some(lib.clone()))
                }
                None => (TableTarget::NotFound, None),
            }
        }
    }
}

/// Resolves every dependency in `deps` via [`resolve_table_reference`],
/// memoizing by `resource` so a dependency repeated under two labels — or
/// two dependencies naming the same canonical URL — is only ever resolved
/// once per render (#842's own "no redundant requests"). Returns each
/// dependency's own [`sql_libraries::TableRow`] paired with the resolved
/// artifact's raw JSON (#842/04) — *Reads from*'s own render only needs the
/// former; [`resolve_view_definition_dependencies`] filters the latter down
/// to the ViewDefinitions the Columns card needs.
async fn resolve_table_rows(
    state: &WebState,
    version: helios_fhir::FhirVersion,
    tenant: &str,
    deps: &[sql_libraries::TableDependency],
    libraries: &[serde_json::Value],
) -> Vec<(sql_libraries::TableRow, Option<serde_json::Value>)> {
    let mut cache: std::collections::HashMap<
        String,
        (sql_libraries::TableTarget, Option<serde_json::Value>),
    > = std::collections::HashMap::new();
    let mut rows = Vec::with_capacity(deps.len());
    for dep in deps {
        if !cache.contains_key(&dep.resource) {
            let resolved =
                resolve_table_reference(state, version, tenant, &dep.resource, libraries).await;
            cache.insert(dep.resource.clone(), resolved);
        }
        let (target, doc) = cache
            .get(&dep.resource)
            .cloned()
            .unwrap_or((sql_libraries::TableTarget::NotFound, None));
        rows.push((
            sql_libraries::TableRow {
                label: dep.label.clone(),
                resource: dep.resource.clone(),
                target,
            },
            doc,
        ));
    }
    rows
}

/// Resolves every dependency in `deps` down to the ViewDefinitions among
/// them — the Columns card's own origin lookup (#842/04): `(label,
/// resolved document)` for each dependency [`resolve_table_rows`] resolved
/// to a `TableTarget::ViewDefinition`, in declaration order. A dependency
/// that fails to resolve, or resolves to a SQL View or anything else, is
/// simply absent — SQL Views never contribute a column origin (#842's own
/// "out of scope" rule).
///
/// Only ever called after a successful `$sql-run` (#842/04's own NF1): this
/// performs the identical fetch [`resolve_table_rows`] does for the Tables
/// panel, so it is deliberately never invoked when a run failed or never
/// ran at all — a good run's own dependency resolution "happens once per
/// dependency", not once per keystroke that leaves the run unable to
/// happen.
async fn resolve_view_definition_dependencies(
    state: &WebState,
    version: helios_fhir::FhirVersion,
    tenant: &str,
    deps: &[sql_libraries::TableDependency],
    libraries: &[serde_json::Value],
) -> Vec<(String, serde_json::Value)> {
    resolve_table_rows(state, version, tenant, deps, libraries)
        .await
        .into_iter()
        .filter_map(|(row, doc)| match (row.target, doc) {
            (sql_libraries::TableTarget::ViewDefinition { .. }, Some(doc)) => {
                Some((row.label, doc))
            }
            _ => None,
        })
        .collect()
}

/// [`analyze_tables`]'s pure result: everything the Tables panel needs from
/// one (document, already-fetched Library list, already-loaded job list)
/// triple, computed once so *Reads from*'s signature and *Used by*'s rows
/// can never disagree about what the document currently declares.
struct TablesAnalysis {
    rows: Vec<sql_libraries::TableRow>,
    /// Every table the SQL reads that no declared label names (#842/04),
    /// in first-occurrence-in-SQL order — [`sql_libraries::unknown_tables`]'s
    /// own findings, appended as their own rows after `rows`.
    unknown: Vec<String>,
    /// The card's own signature (#842, extended for #842/04) — see
    /// [`sql_libraries::tables_signature_with_unknown`].
    signature: String,
    used_by_artifacts: Vec<sql_libraries::UsedByArtifact>,
    used_by_exports: Vec<sql_libraries::UsedByExport>,
}

/// Analyzes one document/dependency-list/job-list triple for the Tables
/// panel (#842): resolves every declared dependency
/// ([`resolve_table_rows`]), and finds every *Used by* peer — other
/// Libraries depending on this one ([`sql_libraries::used_by_artifacts`])
/// and SQL Export jobs whose subjects reference it
/// ([`sql_libraries::used_by_exports`]). `deps` is the caller's own already-
/// computed [`sql_libraries::table_dependencies`] (every caller needs it a
/// moment earlier anyway, to decide whether to call this at all — see
/// `sql_library_run`'s own signature comparison, #842/NF1). `unknown_tables`
/// (#842/04) is the caller's own already-scanned
/// [`sql_libraries::unknown_tables`] result — a pure, I/O-free computation
/// every caller has to run anyway to decide whether `$sql-run` can even be
/// attempted, so it is never repeated here.
#[allow(clippy::too_many_arguments)]
async fn analyze_tables(
    state: &WebState,
    version: helios_fhir::FhirVersion,
    tenant: &str,
    document: &serde_json::Value,
    deps: &[sql_libraries::TableDependency],
    unknown_tables: &[helios_sof::sqlquery::TableRef],
    libraries: &[serde_json::Value],
    jobs: &[(String, sql_export::ExportJob)],
) -> TablesAnalysis {
    let unknown: Vec<String> = unknown_tables.iter().map(|t| t.name.clone()).collect();
    let signature = sql_libraries::tables_signature_with_unknown(deps, &unknown);
    let rows = resolve_table_rows(state, version, tenant, deps, libraries)
        .await
        .into_iter()
        .map(|(row, _doc)| row)
        .collect();
    let used_by_artifacts = sql_libraries::used_by_artifacts(libraries, document);
    let library_id = document
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let used_by_exports = sql_libraries::used_by_exports(jobs, library_id);
    TablesAnalysis {
        rows,
        unknown,
        signature,
        used_by_artifacts,
        used_by_exports,
    }
}

/// Fetches the tenant's whole Library list and job history, then runs
/// [`analyze_tables`] — the "I have nothing already fetched" path every
/// caller but the page's own render needs (`render_lib_document_page`,
/// `apply_add_table`, `apply_remove_table`'s own `HX-Request` responses):
/// unlike the page's own render, which already fetched a Library list for
/// the rail (#842/NF1 — reused there, never fetched twice), these callers
/// have no rail context of their own and fetch exactly once per response.
/// `deps`/`unknown_tables` (#842/04) are the caller's own already-computed
/// [`sql_libraries::table_dependencies`]/[`sql_libraries::unknown_tables`] —
/// every caller parses `document` and scans the editor's own current SQL
/// text a moment earlier anyway, to decide the run/notice gate this
/// analysis is no substitute for.
async fn full_tables_analysis(
    state: &WebState,
    version: helios_fhir::FhirVersion,
    tenant: &str,
    user_key: &str,
    document: &serde_json::Value,
    deps: &[sql_libraries::TableDependency],
    unknown_tables: &[helios_sof::sqlquery::TableRef],
) -> TablesAnalysis {
    let libraries = state
        .conformance
        .fetch("Library", version, tenant)
        .await
        .unwrap_or_default();
    let jobs = sql_export::jobs_for_used_by(state, user_key, tenant).await;
    analyze_tables(
        state,
        version,
        tenant,
        document,
        deps,
        unknown_tables,
        &libraries,
        &jobs,
    )
    .await
}

/// The *Add table* combobox's own search endpoint href for a Library `id`
/// (empty for `?lib=new`, per #842 — nothing to exclude when nothing
/// is open yet).
fn table_options_href(id: &str) -> String {
    if id.is_empty() {
        "/ui/lookup/table-options?target=lib-tables".to_string()
    } else {
        format!("/ui/lookup/table-options?target=lib-tables&exclude=Library/{id}")
    }
}

/// One *Reads from* row, shaped for `partials/sql_tables_card.html`
/// (#842) — [`sql_libraries::TableRow`] plus this module's own
/// localization of its target, built by [`table_row_view`].
struct TableRowView {
    label: String,
    chip: String,
    /// `"type"` for a resolved target (`.tag--type`), `"failed"` for
    /// `NotFound`/`NotATable` (`.tag--failed`) — the class suffix
    /// `sql_tables_card.html` appends to `tag--`.
    chip_class: &'static str,
    link_href: Option<String>,
    link_label: Option<String>,
    /// The failure detail line under a `NotFound`/`NotATable` row's own
    /// chip (#842); `None` for a resolved row.
    detail: Option<String>,
}

/// Localizes one resolved [`sql_libraries::TableRow`] into a
/// [`TableRowView`] (#842) — the chip text/class, the resolved
/// target's own link, and, for an unresolved row, the failure detail
/// message. `sql_libraries` itself never localizes (matching every other
/// pure-model/view split in this module), so this is the one place a
/// [`sql_libraries::TableTarget`] variant becomes catalog text.
fn table_row_view(i18n: &I18n, row: sql_libraries::TableRow) -> TableRowView {
    match row.target {
        sql_libraries::TableTarget::ViewDefinition { id, name } => TableRowView {
            label: row.label,
            chip: i18n.t("lib-tables-kind-view-definition"),
            chip_class: "type",
            link_href: Some(format!("/ui/sql/view-definitions?vd={id}")),
            link_label: Some(name),
            detail: None,
        },
        sql_libraries::TableTarget::SqlView { id, name } => TableRowView {
            label: row.label,
            chip: i18n.t("sql-views-chip"),
            chip_class: "type",
            link_href: Some(format!("/ui/sql/views?lib={id}")),
            link_label: Some(name),
            detail: None,
        },
        sql_libraries::TableTarget::NotFound => TableRowView {
            chip: i18n.t("lib-tables-target-not-found"),
            chip_class: "failed",
            link_href: None,
            link_label: None,
            detail: Some(i18n.t_arg(
                "lib-tables-target-not-found-detail",
                "resource",
                row.resource,
            )),
            label: row.label,
        },
        sql_libraries::TableTarget::NotATable => TableRowView {
            chip: i18n.t("lib-tables-target-not-a-table"),
            chip_class: "failed",
            link_href: None,
            link_label: None,
            detail: Some(i18n.t("lib-tables-target-not-a-table-detail")),
            label: row.label,
        },
    }
}

/// One *Reads from* row for a table the SQL reads but no dependency
/// declares (#842/04) — rendered after every resolved [`TableRowView`],
/// with its own chip and *Declare* action rather than reusing
/// [`sql_libraries::TableTarget::NotFound`]'s shape: unlike a declared
/// dependency whose target fails to resolve, this table names no
/// `relatedArtifact.resource` to resolve, or fail to resolve, at all.
struct UnknownTableRowView {
    /// The table name exactly as the SQL spells it — both the row's own
    /// `<code>` text and the *Declare* button's own prefill value.
    name: String,
}

/// One *Used by* row, shaped for `partials/sql_tables_card.html`
/// (#842) — an artifact or an export, already indistinguishable by the
/// time the card renders them (both are just a chip, a link and a label).
struct UsedByRowView {
    chip: String,
    href: String,
    label: String,
}

/// Localizes [`TablesAnalysis::used_by_artifacts`]/`used_by_exports` into
/// the card's own row order (#842: artifacts first, then exports —
/// each group already sorted by its own producer, name for artifacts
/// ([`sql_libraries::used_by_artifacts`]), most-recently-started for
/// exports ([`sql_export::jobs_for_used_by`])).
fn used_by_view(i18n: &I18n, analysis: &TablesAnalysis) -> Vec<UsedByRowView> {
    let mut rows: Vec<UsedByRowView> = analysis
        .used_by_artifacts
        .iter()
        .map(|artifact| {
            let (chip_key, base_href) = match artifact.kind {
                sql_libraries::LibraryArtifactKind::SqlQuery => {
                    ("sql-queries-chip", "/ui/sql/queries")
                }
                sql_libraries::LibraryArtifactKind::SqlView => ("sql-views-chip", "/ui/sql/views"),
            };
            UsedByRowView {
                chip: i18n.t(chip_key),
                href: format!("{base_href}?lib={}", artifact.id),
                label: artifact.name.clone(),
            }
        })
        .collect();
    rows.extend(analysis.used_by_exports.iter().map(|export| UsedByRowView {
        chip: i18n.t("lib-used-by-export-kind"),
        href: format!("/ui/sql/export/{}", export.job_id),
        label: export.label.clone(),
    }));
    rows
}

/// The *Add table* panel's own re-submitted state (#842): the `table`/
/// `alias` text [`sql_library_document`] echoes back into the `<details>`
/// on a rejected `add-table`, and the validation message alongside them.
/// `Default` is every other render's own state — closed, empty, no error.
#[derive(Default)]
struct AddTableFormState {
    table: String,
    alias: String,
    open: bool,
    error: Option<String>,
}

/// [`build_tables_card`]'s presentation-only options — mirrors
/// [`ParamsCardOptions`] for the Tables panel: `Default` is the page's own
/// inline render (no OOB swap, no `data_document`, the *Add table* panel
/// closed and clean).
#[derive(Default)]
struct TablesCardOptions {
    add: AddTableFormState,
    /// `true` only for the `/run` fragment's own OOB companion (#842) — see
    /// [`ParamsCardOptions::oob`]'s identical role for the Parameters card.
    oob: bool,
    /// The `document` endpoint's own `HX-Request` success response carries
    /// the updated Details document here (#842) — `host.setDoc`
    /// (`sql-library-panels.js`) applies it exactly as it already does for
    /// `#lib-params`.
    data_document: Option<String>,
}

/// Builds the Tables panel's left-hand card (#842) from an already-computed
/// [`TablesAnalysis`] plus `options`' presentation state — mirrors
/// [`build_params_card`]'s own role for the Parameters card.
///
/// #842/04: on a page-level render (`!options.oob` — the page's own first
/// paint, the `document` endpoint's own no-JS echo, or a validation-error
/// re-render; never the `/run` fragment's own OOB companion) whose SQL
/// reads at least one unknown table, the panel opens itself with the
/// *first* one's own name already in the alias field whenever
/// `options.add` is still its own untouched default (closed, empty, no
/// error — never the case after a rejected *Add table* submission, which
/// always sets at least one of those) — the no-JS half of *Declare*'s own
/// contract: a no-JS visitor has no other way to reach the panel at
/// all. With JavaScript, the live `/run` fragment never auto-opens it —
/// only clicking a specific row's own *Declare {name}* button
/// (`sql-library-panels.js`) does, so introducing a typo while typing
/// never yanks focus into a panel the visitor did not ask for.
fn build_tables_card(
    i18n: I18n,
    kind: &LibraryKind,
    analysis: TablesAnalysis,
    table_options_href: String,
    options: TablesCardOptions,
) -> LibTablesCard {
    let used_by = used_by_view(&i18n, &analysis);
    let rows = analysis
        .rows
        .into_iter()
        .map(|row| table_row_view(&i18n, row))
        .collect();
    let unknown_rows: Vec<UnknownTableRowView> = analysis
        .unknown
        .into_iter()
        .map(|name| UnknownTableRowView { name })
        .collect();
    let add_is_default = !options.oob
        && options.add.table.is_empty()
        && options.add.alias.is_empty()
        && !options.add.open
        && options.add.error.is_none();
    let (add_alias, add_open) = if add_is_default && let Some(first) = unknown_rows.first() {
        (first.name.clone(), true)
    } else {
        (options.add.alias, options.add.open)
    };
    LibTablesCard {
        i18n,
        document_href: kind.document_href,
        table_options_href,
        rows,
        unknown_rows,
        used_by,
        signature: analysis.signature,
        add_table: options.add.table,
        add_alias,
        add_open,
        add_error: options.add.error,
        oob: options.oob,
        data_document: options.data_document,
    }
}

/// `partials/sql_tables_card.html`'s render surface (#842): the resolved
/// *Reads from* rows, the *Used by* rows, the `tables_sig` signature, and
/// the *Add table* panel — built once by [`build_tables_card`] and shared
/// by the page's own first paint, the `/run` fragment's OOB companion, and
/// the `document` endpoint's own response, exactly like [`LibParamsCard`].
#[derive(Template)]
#[template(path = "partials/sql_tables_card.html")]
struct LibTablesCard {
    i18n: I18n,
    /// This kind's `POST …/document` endpoint (#842) — the *Add table*/
    /// *Remove* controls' own `hx-post`/`formaction`.
    document_href: &'static str,
    /// The *Add table* combobox's own search endpoint (#842).
    table_options_href: String,
    rows: Vec<TableRowView>,
    /// Tables the SQL reads that no dependency declares (#842/04),
    /// rendered as their own rows after `rows` — never counted in the
    /// card head's own `.toolbar__count`, which still names only the
    /// *declared* dependencies.
    unknown_rows: Vec<UnknownTableRowView>,
    used_by: Vec<UsedByRowView>,
    /// The `tables_sig` hidden field's own value (#842).
    signature: String,
    add_table: String,
    add_alias: String,
    add_open: bool,
    add_error: Option<String>,
    /// `true` only for the `/run` fragment's own OOB companion — see
    /// [`TablesCardOptions::oob`].
    oob: bool,
    /// The updated Details document, present only on the `document`
    /// endpoint's own successful `HX-Request` response — see
    /// [`TablesCardOptions::data_document`].
    data_document: Option<String>,
}

/// One *Columns* row, shaped for `partials/sql_columns_card.html`
/// (#842/04) — a [`sql_libraries::ColumnInfo`] with its type/origin already
/// rendered to display text, `"—"` standing in for either `None`
/// (`crate::column_rows`, the one place a [`sql_libraries::ColumnInfo`]
/// becomes catalog-free display text — there is no i18n key for it, this
/// placeholder is the same in every locale).
struct ColumnRowView {
    name: String,
    type_text: String,
    /// `"{label}.{column}"` when [`sql_libraries::ColumnInfo::origin`] is
    /// `Some`, `"—"` otherwise.
    origin_text: String,
}

/// The Tables panel's right-hand card (#842, filled in for #842/04):
/// *Columns* — the last good run's own column list, or, with `rows` empty,
/// the same skeleton #842/03 rendered (a fixed `0` count and
/// `empty_key`'s own "run it to see its columns" body). One struct/template
/// covers both: the skeleton is simply this card with nothing to show yet,
/// never a separate render path.
///
/// Rendered two ways:
///   - The page's own inline render (`oob: false`) — skeleton on every
///     render but a successful `?…&saved=1` (`crate::sql_library_page`),
///     which fills `rows` in instead.
///   - The `/run` fragment's own OOB companion (`oob: true`) — only on a
///     successful run; a failure of any kind relabels `#lib-columns-meta`
///     on its own instead of sending this card at all (#842/04's own
///     `ColumnsFragment::Stale`), so `rows` is never empty when this
///     variant travels.
#[derive(Template)]
#[template(path = "partials/sql_columns_card.html")]
struct LibColumnsCard {
    i18n: I18n,
    /// "what the query produces" / "what the view produces"
    /// (`kind.columns_meta_key`) — never the "last successful run" stale
    /// text, which only ever travels as `ColumnsFragment::Stale`'s own
    /// meta-only OOB update, never through this card.
    meta: String,
    rows: Vec<ColumnRowView>,
    /// The skeleton's own empty-state body (`kind.columns_empty_key`) —
    /// read only when `rows` is empty.
    empty_key: &'static str,
    /// `true` only for the `/run` fragment's own OOB companion — see this
    /// struct's own doc comment.
    oob: bool,
}

/// Builds the Columns card (#842/04) — the skeleton (`rows: Vec::new()`,
/// every render but a good run) or the last good run's own column list,
/// localizing each [`sql_libraries::ColumnInfo`] [`sql_libraries::
/// analyze_columns`] returns into a [`ColumnRowView`].
fn build_columns_card(
    i18n: I18n,
    kind: &LibraryKind,
    rows: Vec<sql_libraries::ColumnInfo>,
    oob: bool,
) -> LibColumnsCard {
    let rows = rows
        .into_iter()
        .map(|c| ColumnRowView {
            name: c.name,
            type_text: c.type_code.unwrap_or_else(|| "—".to_string()),
            origin_text: c
                .origin
                .map(|(label, column)| format!("{label}.{column}"))
                .unwrap_or_else(|| "—".to_string()),
        })
        .collect();
    LibColumnsCard {
        i18n,
        meta: i18n.t(kind.columns_meta_key),
        rows,
        empty_key: kind.columns_empty_key,
        oob,
    }
}

/// The unknown-table lint's own `#run-notice` (#842/04) — rendered in
/// place of [`RunResultsPartial`]'s own `Failure` arm because this notice
/// needs a `data-diagnostics` attribute `partials/sql_run_results.html` has
/// no reason to carry for every page that shares it (View Definitions has
/// no SQL to scan tables out of at all, and that partial's own doc comment
/// asks that it stay exactly as it is). Otherwise the identical OOB shape
/// as that partial's own `Failure` arm: the previous `#run-results` table
/// (if any) is left untouched, and only its meta is relabelled "last
/// successful run" — see [`unknown_tables_notice`] for how its own fields
/// are built.
#[derive(Template)]
#[template(path = "partials/lib_run_unknown_tables.html")]
struct UnknownTablesNotice {
    i18n: I18n,
    fragment: bool,
    /// The *first* unknown table's own 1-based line — `data-error-line`,
    /// the same attribute [`sql_views::extract_error_line`]'s own findings
    /// already tint via `sql-editor.js`.
    first_line: usize,
    /// The whole notice's own text — the first table's own long sentence,
    /// then one short sentence per additional table.
    message: String,
    /// `{ "from", "to", "message", "table" }` objects, one per unknown
    /// table, JSON-encoded (`data-diagnostics`) — `sql-editor.js`'s own
    /// `setDiagnostics` input.
    diagnostics_json: String,
}

/// Builds [`UnknownTablesNotice`]'s own text and diagnostics from the
/// scanner's own findings (#842/04) — shared by every surface that gates
/// `$sql-run` on it the same way: the `/run` fragment, `?…&saved=1`'s own
/// server-side run, and the `document` endpoint's own no-JS echo. `tables` must be
/// non-empty; every caller only reaches here once
/// [`sql_libraries::unknown_tables`] returned something.
///
/// Every diagnostic's own `"message"` is the *short* per-table sentence
/// (`lib-run-unknown-table-more`), even the first one — the long "declare
/// it under Reads from" sentence only ever appears once, in the notice's
/// own banner text, never repeated in each position's own hover tooltip.
fn unknown_tables_notice(
    i18n: I18n,
    fragment: bool,
    tables: &[helios_sof::sqlquery::TableRef],
) -> UnknownTablesNotice {
    let mut message = String::new();
    let mut diagnostics = Vec::with_capacity(tables.len());
    for (index, table) in tables.iter().enumerate() {
        let line = table.position.line.to_string();
        let short = i18n.t_arg2(
            "lib-run-unknown-table-more",
            "name",
            table.name.clone(),
            "line",
            line.clone(),
        );
        if index == 0 {
            message.push_str(&i18n.t_arg2(
                "lib-run-unknown-table",
                "name",
                table.name.clone(),
                "line",
                line,
            ));
        } else {
            message.push(' ');
            message.push_str(&short);
        }
        diagnostics.push(serde_json::json!({
            "from": table.position.offset,
            "to": table.position.offset + table.position.length,
            "message": short,
            "table": table.name,
        }));
    }
    UnknownTablesNotice {
        i18n,
        fragment,
        first_line: tables[0].position.line,
        message,
        diagnostics_json: serde_json::to_string(&diagnostics).unwrap_or_default(),
    }
}

/// The Library-backed pages' own `#run-notice` region (#842/04) — either
/// the shared [`RunResultsPartial`] every SQL on FHIR playground uses, or
/// the unknown-table lint's own [`UnknownTablesNotice`] taking its place: a
/// SQL View with declared parameters, a required parameter
/// with no value, and an actual `$sql-run` failure all render as
/// [`Self::Standard`]; only "the SQL reads a table no dependency declares"
/// renders as [`Self::UnknownTables`], since only that case needs
/// `data-diagnostics`. Used both by the `/run` fragment
/// ([`LibRunFragment::run_results`]) and by every full-page render
/// ([`SqlLibraryPage::run_results`]) that must show the identical notice
/// in place of results (`?…&saved=1`, the `document` endpoint's own no-JS
/// echo).
enum LibRunNotice {
    Standard(RunResultsPartial),
    UnknownTables(UnknownTablesNotice),
}

/// The SQL Queries / SQL Views workspace (#649): the same shape as View
/// Definitions — rail, editor, `$sql-run` preview — over `Library` resources
/// of the page's kind, with the SQL decoded out of its base64 attachment into
/// an editor pane of its own and re-embedded on save.
#[derive(Template)]
#[template(path = "pages/sql-library.html")]
struct SqlLibraryPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    base_href: &'static str,
    /// This kind's `POST …/run` fragment endpoint (`kind.run_href`), read by
    /// the SQL textarea's own `hx-post` (#839).
    run_href: &'static str,
    title_key: &'static str,
    lede_key: &'static str,
    new_title_key: &'static str,
    all_heading_key: &'static str,
    /// The filter rail search box's placeholder key (`kind.filter_placeholder_key`).
    filter_placeholder_key: &'static str,
    /// The rail's own "no Libraries of this kind" key (`kind.rail_empty_key`).
    rail_empty_key: &'static str,
    /// The SQL editor card's heading key (`kind.editor_heading_key`).
    editor_heading_key: &'static str,
    /// The "no selection" empty state's heading and lede keys
    /// (`kind.empty_title_key`/`kind.empty_lede_key`).
    empty_title_key: &'static str,
    empty_lede_key: &'static str,
    /// The title row's `.tag--type` chip text key (`kind.chip_key`) and
    /// embedded type icon (`kind.icon_svg`), both from #839.
    chip_key: &'static str,
    icon_svg: &'static str,
    rail: Vec<sql_libraries::LibSummary>,
    filter: String,
    degraded: Option<String>,
    selected: Option<SelectedLib>,
    is_new: bool,
    /// The Details card's guided-form panel (#840), alongside its own JSON
    /// editor — the same shape View Definitions' `form_pane` is, built
    /// inline from the document `selected.json` already shows so the page's
    /// first paint never flashes full-width before shrinking to make room
    /// for it. `None` only alongside `selected: None`.
    details: Option<editor::EditorFormPane>,
    /// Whether this kind declares `Library.parameter[]` at all
    /// (`kind.declares_parameters`, #841) — the template's own gate for
    /// rendering `params_card`, never an `if` on `code`.
    declares_parameters: bool,
    /// The Parameters card (#841), `Some` only alongside
    /// `declares_parameters` and a selection — `None` for SQL Views and for
    /// the "no Library yet" empty state.
    params_card: Option<LibParamsCard>,
    /// The Tables panel's left-hand card (#842, both kinds): *Reads from*
    /// and *Used by*. `Some` whenever a Library is selected — including
    /// `?lib=new` — `None` only for the "no Library yet" empty state.
    tables_card: Option<LibTablesCard>,
    /// The Tables panel's right-hand card (#842, filled in for #842/04):
    /// *Columns* — the skeleton on every render but a successful
    /// `?…&saved=1`, gated the same way as `tables_card` (present only
    /// alongside a selection, so this needs no further gate of its own in
    /// the template).
    columns_card: Option<LibColumnsCard>,
    /// The `$sql-run` preview card and its failure notice — either the
    /// shared `partials/sql_run_results.html` markup or the unknown-table
    /// lint's own notice (#842/04, [`LibRunNotice`]). `fragment: false` on
    /// the `Standard` arm here — the page's own render has nothing to
    /// swap into.
    run_results: LibRunNotice,
    save_error: Option<String>,
    saved: bool,
    /// The "Recently used" group's own rows.
    recent_entries: Vec<rail_state::ResolvedRailEntry>,
    /// `rails.<page>` key this page writes/reads (`kind.page.key()`), carried
    /// to the template so `partials/rail_recent.html`'s clones can name it
    /// without redeclaring it.
    rail_page: &'static str,
    /// The recents cap a tampered client can never exceed, likewise from
    /// `rail_state::MAX_RECENT`, carried the same way.
    max_recent: usize,
}

struct SelectedLib {
    id: String,
    name: String,
    json: String,
    sql: String,
    /// The resource's own `status`, verbatim — the title row's status chip
    /// text (#839), possibly empty when the resource carries none.
    status: String,
    /// The `.tag--{status_class}` chip class [`status_tag_class`] derives
    /// from `status` — `unknown` for anything but `active`/`draft`/`retired`.
    status_class: &'static str,
}

#[derive(Deserialize, Default)]
struct SqlLibQuery {
    lib: Option<String>,
    filter: Option<String>,
    /// `?saved=1` (Save's own redirect): renders the just-saved Library's
    /// `$sql-run` results server-side. There is no `?run=1` — the live
    /// preview is progressive enhancement over `POST …/run` (#839).
    saved: Option<String>,
}

/// Shapes a stored `Library` resource into the editor's `(id, name, json,
/// sql)` quadruple: `sql` decoded out of the base64 `application/sql`
/// attachment for the SQL card, `json` the Details card's own document —
/// `lib` with that same attachment stripped back out (#840,
/// [`sql_libraries::strip_sql_attachment`]) — so the two cards never show
/// the SQL text twice.
fn shape_lib(lib: &serde_json::Value) -> (String, String, String, String) {
    let id = lib
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let name = lib
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&id)
        .to_string();
    let sql = sql_libraries::extract_sql(lib);
    let json =
        serde_json::to_string_pretty(&sql_libraries::strip_sql_attachment(lib)).unwrap_or_default();
    (id, name, json, sql)
}

/// Builds SQL Query/SQL View's Details guided-form panel (#840) against an
/// already-parsed document — the same analysis `editor::render_body`'s
/// `pane=form` branch performs over HTTP (`editor::build_form_pane`) with
/// `hidden=["content"]` and `legend=sql-library`, called directly instead:
/// the page's own render (and the Save-error re-render) need the panel in
/// place on first paint, not fetched after the fact. Mirrors
/// [`render_vd_form_pane`]; `document`'s own `resourceType` decides the
/// fallback resource type when absent, falling back to `"Library"` — every
/// caller on this page hands it a `Library` (its SQL attachment already
/// stripped by the caller), except the one Save-error path where the
/// submitted document parses but carries some other type.
fn render_lib_details_pane(
    i18n: I18n,
    version: helios_fhir::FhirVersion,
    document: serde_json::Value,
) -> editor::EditorFormPane {
    let registry = helios_fhir_validator::packs::core_registry(version);
    let resource_type = document
        .get("resourceType")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Library")
        .to_string();
    // #840: the SQL attachment lives in its own card below, never in the
    // Details form — hidden from both its rows and its own "+ Add" list —
    // and the two-line legend names what Save actually gates on this page
    // (the Library type coding and the SQL attachment), not the generic
    // constraints/terminology promise `Legend::Resource` makes.
    editor::build_form_pane(
        i18n,
        registry,
        version,
        resource_type,
        document,
        None,
        true,
        &[String::from("content")],
        "sql-library",
    )
}

/// The Details panel for whichever document this render selected (#840):
/// the stored library — its SQL attachment stripped — or `?lib=new`'s
/// starter document (which carries none to begin with), mirroring
/// [`vd_form_pane_for_selection`]. `None` only alongside `selected: None`.
fn lib_details_pane_for_selection(
    i18n: I18n,
    version: helios_fhir::FhirVersion,
    kind: &LibraryKind,
    is_new: bool,
    selected_value: Option<&serde_json::Value>,
) -> Option<editor::EditorFormPane> {
    if is_new {
        Some(render_lib_details_pane(
            i18n,
            version,
            sql_libraries::starter_library_value(kind.code),
        ))
    } else {
        selected_value.map(|lib| {
            render_lib_details_pane(i18n, version, sql_libraries::strip_sql_attachment(lib))
        })
    }
}

/// Resolves one candidate Library id, requiring it to carry `code`: a stored
/// `last` or the rail's own fallback candidate must belong to this page's
/// kind, never resolve to a Library of the other one. Deliberately stricter
/// than the explicit `?lib=` lookup in [`sql_library_page`] below, which —
/// as #649 always has — trusts whatever id the URL names outright;
/// narrowing that one too is a pre-existing behavior left alone here.
fn resolve_lib_of_kind(
    id: &str,
    code: &str,
    libraries: &mut Vec<serde_json::Value>,
) -> Option<serde_json::Value> {
    let i = libraries.iter().position(|l| {
        l.get("id").and_then(serde_json::Value::as_str) == Some(id)
            && sql_libraries::has_library_code(l, code)
    })?;
    Some(libraries.swap_remove(i))
}

/// For the Library-backed rails (SQL Queries, SQL Views): the "Recently
/// used" group's rows, resolved against this render's own (kind- and
/// search-box-filtered) rail list so label/meta/href/current match what the
/// list already shows. "The group is never itself filtered by `?filter=`"
/// falls out for free: an id the current filter excludes is simply absent
/// from `summaries` (and so from `live`), so
/// [`rail_state::RailState::resolve_recents`] renders it from its stored
/// `{name, meta}` snapshot instead.
fn resolve_lib_recents(
    rail: &rail_state::RailState,
    summaries: &[sql_libraries::LibSummary],
    base_href: &str,
    selected_id: Option<&str>,
) -> Vec<rail_state::ResolvedRailEntry> {
    let live: std::collections::HashMap<String, rail_state::LiveRailItem> = summaries
        .iter()
        .map(|item| {
            (
                item.id.clone(),
                rail_state::LiveRailItem {
                    label: item.name.clone(),
                    meta: Some(item.status.clone()),
                    count: None,
                    href: format!("{base_href}?lib={}", item.id),
                    current: selected_id == Some(item.id.as_str()),
                },
            )
        })
        .collect();
    rail.resolve_recents(&live, |id| format!("{base_href}?lib={id}"), None)
}

#[allow(clippy::too_many_arguments)]
async fn sql_library_page(
    state: WebState,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    query: SqlLibQuery,
    kind: &LibraryKind,
    settings: rail_state::RequestSettings,
    user_key: String,
) -> Response {
    let filter = query.filter.unwrap_or_default();
    let (mut libraries, degraded) = match state.conformance.fetch("Library", rv.0, &rt.id).await {
        Ok(resources) => (resources, None),
        Err(error) => {
            tracing::warn!("Library self-fetch failed: {error}");
            (Vec::new(), Some(error))
        }
    };
    let summaries = {
        let mut s = sql_libraries::summarize(&libraries, kind.code);
        if !filter.is_empty() {
            let needle = filter.to_lowercase();
            s.retain(|e| e.name.to_lowercase().contains(&needle));
        }
        s
    };

    let rail_before = settings.rail(kind.page, &rt.id);
    let is_new = query.lib.as_deref() == Some("new");
    let (selected, selected_value, rail) = if is_new {
        // "Create New" is never a selection (the `?lib=new` exception):
        // nothing here reads or writes `rails.<page>`.
        (
            Some(SelectedLib {
                id: String::new(),
                name: String::new(),
                json: sql_libraries::starter_library(kind.code),
                sql: sql_libraries::STARTER_SQL.to_string(),
                status: sql_libraries::STARTER_STATUS.to_string(),
                status_class: status_tag_class(sql_libraries::STARTER_STATUS),
            }),
            None,
            rail_before,
        )
    } else if let Some(explicit_id) = query.lib.clone() {
        // An explicit `?lib=` always wins, resolved or not.
        let found = libraries
            .iter()
            .position(|l| l.get("id").and_then(serde_json::Value::as_str) == Some(&explicit_id));
        match found.map(|i| libraries.swap_remove(i)) {
            Some(lib) => {
                let (id, name, json, sql) = shape_lib(&lib);
                let status = sql_libraries::extract_status(&lib);
                let meta = (!status.is_empty()).then(|| status.clone());
                let entry = rail_state::RailEntry::with_snapshot(id.clone(), name.clone(), meta);
                let rail = record_snapshot_selection(
                    &state,
                    &settings.user_key,
                    &rt.id,
                    kind.page,
                    rail_before,
                    entry,
                )
                .await;
                (
                    Some(SelectedLib {
                        id,
                        name,
                        json,
                        sql,
                        status_class: status_tag_class(&status),
                        status,
                    }),
                    Some(lib),
                    rail,
                )
            }
            None => {
                // A stale or mistyped explicit id is pruned from the
                // registry; the page itself keeps its current "no selection"
                // render either way.
                let rail = prune_stale_selection(
                    &state,
                    &settings.user_key,
                    &rt.id,
                    kind.page,
                    rail_before,
                    &explicit_id,
                )
                .await;
                (None, None, rail)
            }
        }
    } else {
        // No explicit selection: try the stored `last`, falling back to
        // the rail's first visible entry when there is none or it no
        // longer resolves — both silently: no write either way.
        let stored_id = rail_before.last.clone().filter(|id| !id.is_empty());
        let mut resolved =
            stored_id.and_then(|id| resolve_lib_of_kind(&id, kind.code, &mut libraries));
        if resolved.is_none() {
            let fallback_id = summaries.first().map(|e| e.id.clone());
            resolved =
                fallback_id.and_then(|id| resolve_lib_of_kind(&id, kind.code, &mut libraries));
        }
        match resolved {
            Some(lib) => {
                let (id, name, json, sql) = shape_lib(&lib);
                let status = sql_libraries::extract_status(&lib);
                (
                    Some(SelectedLib {
                        id,
                        name,
                        json,
                        sql,
                        status_class: status_tag_class(&status),
                        status,
                    }),
                    Some(lib),
                    rail_before,
                )
            }
            None => (None, None, rail_before),
        }
    };

    let recent_entries = resolve_lib_recents(
        &rail,
        &summaries,
        kind.base_href,
        selected.as_ref().map(|s| s.id.as_str()),
    );

    // `?saved=1` (Save's own redirect) runs the just-stored Library through
    // $sql-run once, server-side, so the nojs path shows results without a
    // client request. Every other render's own state is `Empty` — the empty
    // notice this produces is what carries the client-driven initial-load
    // request, covering both an ordinary `?lib=` navigation and `?lib=new`'s
    // starter document (#839, mirroring View Definitions' own `?vd=…&saved=
    // 1` handling in `sql_view_definitions_page`).
    let i18n = I18n::new(locale);
    // Owned, not borrowed: `selected` itself moves into the response below,
    // in the same expression that still needs this id for `export_href`.
    let selected_id = selected.as_ref().map(|s| s.id.clone()).unwrap_or_default();

    // #842: the Tables panel, for whichever document this render selected —
    // the stored library, or `?lib=new`'s starter document (which still
    // declares one dependency, `change-me`, unresolved). Reuses the
    // very `libraries` fetch the rail above already made (NF1): by this
    // point the selected library's own entry has already been removed from
    // it (`swap_remove`, above), so it never lists itself as a dependency
    // target nor as a *Used by* peer.
    let tables_document: Option<serde_json::Value> = if is_new {
        Some(sql_libraries::starter_library_value(kind.code))
    } else {
        selected_value.clone()
    };
    // #842/04: the unknown-table lint applies to `?…&saved=1`'s own
    // server-side run exactly as it does to the `/run` fragment — computed
    // once here (pure, no I/O) and reused both to gate the run below and to
    // feed *Reads from*'s own unknown rows. `selected.sql` (never
    // `tables_document`'s own, possibly stale, embedded attachment) is the
    // editor's current text — for `?lib=new` that is the starter SQL, which
    // its own declared `v` dependency already covers.
    let deps = tables_document
        .as_ref()
        .map(sql_libraries::table_dependencies)
        .unwrap_or_default();
    let unknown_tables = selected
        .as_ref()
        .map(|s| sql_libraries::unknown_tables(&s.sql, &deps))
        .unwrap_or_default();
    let tables_card = match &tables_document {
        Some(document) => {
            let jobs = sql_export::jobs_for_used_by(&state, &user_key, &rt.id).await;
            let analysis = analyze_tables(
                &state,
                rv.0,
                &rt.id,
                document,
                &deps,
                &unknown_tables,
                &libraries,
                &jobs,
            )
            .await;
            Some(build_tables_card(
                i18n,
                kind,
                analysis,
                table_options_href(&selected_id),
                TablesCardOptions::default(),
            ))
        }
        None => None,
    };

    // #841: this render never has any submitted parameter values — a fresh
    // navigation has nothing to echo, and `?…&saved=1`'s own POST body is
    // lost to Save's redirect — so the analysis and
    // the card it feeds are always built off an empty map. A SQL Query with
    // a declared, required parameter therefore always needs a value typed
    // in before `?…&saved=1` can show a table; until then this render's own
    // `run_results` below shows the same "waiting" notice the `/run`
    // fragment would.
    let no_values = std::collections::HashMap::new();
    let analysis = kind
        .declares_parameters
        .then_some(selected_value.as_ref())
        .flatten()
        .map(|lib| {
            let sql = selected
                .as_ref()
                .map(|s| s.sql.as_str())
                .unwrap_or_default();
            analyze_params(lib, sql, &no_values)
        });
    // #842/04: a SQL View's own non-empty `parameter[]` — the same gate
    // `sql_library_run` checks first, against the *submitted* document
    // rather than an `analyze_params`-style analysis, since it never even
    // reaches SQL Views. `?…&saved=1` only ever shows a *stored* Library,
    // which Save's own gate (#841/#842) already keeps from carrying one —
    // this exists only for a document a raw API write put here instead.
    let view_has_parameters = !kind.declares_parameters
        && selected_value
            .as_ref()
            .and_then(|lib| lib.get("parameter"))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|params| !params.is_empty());
    let export_href_val = export_href(kind, &selected_id);
    let standard_notice = |state: RunResultsState| {
        LibRunNotice::Standard(RunResultsPartial {
            i18n,
            fragment: false,
            run_href: kind.run_href,
            form_id: LIB_EDITOR_FORM_ID,
            heading_key: kind.results_heading_key,
            failed_key: kind.failed_key,
            export_href: export_href_val.clone(),
            state,
        })
    };
    let (run_results, columns_rows) = match (&selected_value, query.saved.as_deref() == Some("1")) {
        (Some(_), true) if view_has_parameters => (
            standard_notice(RunResultsState::Failure(
                i18n.t("lib-save-view-parameters"),
                None,
            )),
            Vec::new(),
        ),
        (Some(_), true) if !unknown_tables.is_empty() => (
            LibRunNotice::UnknownTables(unknown_tables_notice(i18n, false, &unknown_tables)),
            Vec::new(),
        ),
        (Some(_), true)
            if analysis
                .as_ref()
                .is_some_and(|a| !a.missing_required.is_empty()) =>
        {
            (
                standard_notice(RunResultsState::Waiting(waiting_message(
                    &i18n,
                    &analysis.as_ref().expect("checked above").missing_required,
                ))),
                Vec::new(),
            )
        }
        (Some(lib), true) => {
            let bindings = analysis
                .as_ref()
                .map(|a| a.bindings.as_slice())
                .unwrap_or(&[]);
            match run_sql_preview(&state, lib, bindings, rv.0, &rt.id).await {
                Ok((table, raw_rows, ms)) => {
                    // #842/04's own NF1: the Columns card's origin lookup
                    // resolves dependencies to ViewDefinitions only after a
                    // run has actually succeeded — and only the ones the
                    // SQL actually reads (`dependencies_used_by_sql`), so a
                    // stale, no-longer-queried label never manufactures a
                    // false "ambiguous origin".
                    let run_sql = selected
                        .as_ref()
                        .map(|s| s.sql.as_str())
                        .unwrap_or_default();
                    let used_deps = sql_libraries::dependencies_used_by_sql(run_sql, &deps);
                    let view_definitions = resolve_view_definition_dependencies(
                        &state, rv.0, &rt.id, &used_deps, &libraries,
                    )
                    .await;
                    let columns = sql_libraries::analyze_columns(
                        &table.columns,
                        &raw_rows,
                        &view_definitions,
                    );
                    (
                        standard_notice(RunResultsState::Success(table, ms)),
                        columns,
                    )
                }
                Err(error) => {
                    let line = sql_views::extract_error_line(&error);
                    (
                        standard_notice(RunResultsState::Failure(error, line)),
                        Vec::new(),
                    )
                }
            }
        }
        _ => (standard_notice(RunResultsState::Empty), Vec::new()),
    };
    let params_card = analysis
        .map(|analysis| build_params_card(i18n, kind, analysis, ParamsCardOptions::default()));
    // #842/04: the skeleton on every render but a good `?…&saved=1` run,
    // gated identically to `tables_card` — `Some` alongside any selection,
    // `None` only for the "no Library yet" empty state.
    let columns_card = tables_card
        .is_some()
        .then(|| build_columns_card(i18n, kind, columns_rows, false));
    // #840: the Details card's guided-form panel, built inline from the same
    // document `selected.json` already shows — `selected_value` is still
    // borrowed here, ahead of `selected` itself moving into the response.
    let details = lib_details_pane_for_selection(i18n, rv.0, kind, is_new, selected_value.as_ref());

    render(SqlLibraryPage {
        status: current_status(&state, rv.0, &rt),
        i18n,
        active_page: kind.active_page,
        base_href: kind.base_href,
        run_href: kind.run_href,
        title_key: kind.title_key,
        lede_key: kind.lede_key,
        new_title_key: kind.new_title_key,
        all_heading_key: kind.all_heading_key,
        filter_placeholder_key: kind.filter_placeholder_key,
        rail_empty_key: kind.rail_empty_key,
        editor_heading_key: kind.editor_heading_key,
        empty_title_key: kind.empty_title_key,
        empty_lede_key: kind.empty_lede_key,
        chip_key: kind.chip_key,
        icon_svg: kind.icon_svg,
        rail: summaries,
        filter,
        degraded,
        selected,
        is_new,
        details,
        declares_parameters: kind.declares_parameters,
        params_card,
        tables_card,
        columns_card,
        run_results,
        save_error: None,
        saved: query.saved.as_deref() == Some("1"),
        recent_entries,
        rail_page: kind.page.key(),
        max_recent: rail_state::MAX_RECENT,
    })
}

#[derive(Default)]
struct SqlLibSaveForm {
    id: String,
    json: String,
    /// The decoded SQL pane; re-embedded as the base64 attachment on save.
    sql: String,
    action: String,
    /// Every submitted `param:{name}` value (#841) — Save reads these only
    /// to echo the Parameters card back exactly as the user had it on a
    /// rejected submission; [`sql_libraries::add_parameter`] is the only
    /// writer of `Library.parameter[]`, so nothing here ever reaches a
    /// saved resource.
    values: std::collections::HashMap<String, String>,
}

/// Parses `SqlLibSaveForm`'s fields out of a raw urlencoded body (#841): the
/// dynamic `param:{name}` values a `#[derive(Deserialize)]` struct cannot
/// express, alongside the fixed `id`/`json`/`sql`/`action` fields — the same
/// by-hand parse `sql_export::start` uses for its own `param:{reference}:
/// {name}` fields.
fn parse_lib_save_form(body: &[u8]) -> SqlLibSaveForm {
    let mut form = SqlLibSaveForm::default();
    for (key, value) in form_urlencoded::parse(body) {
        if let Some(name) = key.strip_prefix("param:") {
            form.values.insert(name.to_string(), value.into_owned());
            continue;
        }
        match key.as_ref() {
            "id" => form.id = value.into_owned(),
            "json" => form.json = value.into_owned(),
            "sql" => form.sql = value.into_owned(),
            "action" => form.action = value.into_owned(),
            _ => {}
        }
    }
    form
}

/// Builds the Details guided-form panel and the Parameters card together
/// from one submitted JSON string (#840/#841): the panel from the parsed
/// document, with the same document feeding [`analyze_params`] for the
/// card when `kind.declares_parameters`; the invalid-JSON notice in the
/// panel's place, and no card at all, when `json` does not parse — there is
/// no document left to read declarations from either. Shared by Save's own
/// validation-error re-render and the `document` endpoint's own no-JS
/// response ([`render_lib_document_page`]), the two places that reconstruct
/// this pairing from raw submitted text rather than an already-resolved
/// document.
fn lib_details_and_params(
    i18n: I18n,
    version: helios_fhir::FhirVersion,
    kind: &LibraryKind,
    json: &str,
    sql: &str,
    values: &std::collections::HashMap<String, String>,
    add: AddParamFormState,
) -> (editor::EditorFormPane, Option<LibParamsCard>) {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(document) => {
            let details = render_lib_details_pane(i18n, version, document.clone());
            let params_card = kind.declares_parameters.then(|| {
                let analysis = analyze_params(&document, sql, values);
                build_params_card(
                    i18n,
                    kind,
                    analysis,
                    ParamsCardOptions {
                        add,
                        ..Default::default()
                    },
                )
            });
            (details, params_card)
        }
        Err(parse_error) => {
            let details = invalid_form_pane(
                i18n,
                json.to_string(),
                parse_error.to_string(),
                editor::Legend::SqlLibrary,
            );
            (details, None)
        }
    }
}

/// Re-renders the whole SQL Query/SQL View workspace around a submitted —
/// not necessarily saved — document (#840, extended for #841): Save's own
/// validation-error path and the `document` endpoint's own no-JS response
/// both land here. Neither writes anything to storage; both show exactly
/// the `json`/`sql`/parameter `values` the request carried (for the
/// `document` endpoint's own success case, `json` is the document
/// [`sql_libraries::add_parameter`] just produced, pretty-printed — not
/// what was submitted); neither ever calls `$sql-run` — the submitted text
/// is what the live-preview wiring runs once the page opens, once
/// JavaScript re-fires it — so the results region is always either empty
/// (`RunResultsState::Empty`, the same "nothing has run server-side yet"
/// shell every render with no `?…&saved=1` uses) or, when the SQL reads an
/// undeclared table, the unknown-table lint's own notice in its place
/// (#842/04 — free to compute, no I/O involved).
///
/// `save_error` is the page-level `.notice--warn` banner: `Some` for Save's
/// own rejections and the `document` endpoint's whole-document failures (an
/// invalid JSON body, or the wrong `resourceType` — #841/#842), `None`
/// otherwise, including the `document` endpoint's own success and its
/// own `add-parameter`/`add-table`-specific validation failures, which
/// surface through the Parameters/Tables cards' own `add` panels instead
/// ([`AddParamFormState`]/[`AddTableFormState`]).
///
/// #842: the Tables panel renders here too — built from `json` the same way
/// [`lib_details_and_params`] builds `params_card`, `None` alongside it
/// when `json` fails to parse (there is no document to resolve
/// dependencies from either), `Some` otherwise via [`full_tables_analysis`]
/// (this function has no rail-fetched Library list of its own to reuse,
/// unlike [`sql_library_page`]'s own render).
#[allow(clippy::too_many_arguments)]
async fn render_lib_document_page(
    state: &WebState,
    locale: RequestLocale,
    version: helios_fhir::FhirVersion,
    rt: &RequestTenant,
    kind: &LibraryKind,
    user_key: &str,
    json: String,
    sql: String,
    is_new: bool,
    id: String,
    status: String,
    // Owned, not borrowed: every caller but `sql_library_document`'s own
    // `apply_*` handlers reaches this from a plain (non-`async`) closure
    // whose own by-value parameter would otherwise not outlive the `Future`
    // this `async fn` returns (see `sql_library_save`'s own `error_page`).
    values: std::collections::HashMap<String, String>,
    save_error: Option<String>,
    add: AddParamFormState,
    add_table: AddTableFormState,
) -> SqlLibraryPage {
    // Computed before `id` moves into `SelectedLib` below.
    let export_href = export_href(kind, &id);
    let status_class = status_tag_class(&status);
    let i18n = I18n::new(locale);
    let (details, params_card) =
        lib_details_and_params(i18n, version, kind, &json, &sql, &values, add);
    // #842/04: the unknown-table lint applies to this endpoint's own
    // no-JS re-render too — a document mutation never itself calls
    // `$sql-run` (the submitted text is what the live-preview wiring runs
    // once the page opens, per this function's own doc comment above), but
    // detecting an unknown table is pure and free, so there is no reason
    // not to show the same notice a `/run` fragment would have.
    let (tables_card, run_results) = match serde_json::from_str::<serde_json::Value>(json.trim()) {
        Ok(document) => {
            let deps = sql_libraries::table_dependencies(&document);
            let unknown_tables = sql_libraries::unknown_tables(&sql, &deps);
            let analysis = full_tables_analysis(
                state,
                version,
                &rt.id,
                user_key,
                &document,
                &deps,
                &unknown_tables,
            )
            .await;
            let tables_card = Some(build_tables_card(
                i18n,
                kind,
                analysis,
                table_options_href(&id),
                TablesCardOptions {
                    add: add_table,
                    ..Default::default()
                },
            ));
            let run_results = if unknown_tables.is_empty() {
                LibRunNotice::Standard(RunResultsPartial {
                    i18n,
                    fragment: false,
                    run_href: kind.run_href,
                    form_id: LIB_EDITOR_FORM_ID,
                    heading_key: kind.results_heading_key,
                    failed_key: kind.failed_key,
                    export_href: export_href.clone(),
                    state: RunResultsState::Empty,
                })
            } else {
                LibRunNotice::UnknownTables(unknown_tables_notice(i18n, false, &unknown_tables))
            };
            (tables_card, run_results)
        }
        Err(_) => (
            None,
            LibRunNotice::Standard(RunResultsPartial {
                i18n,
                fragment: false,
                run_href: kind.run_href,
                form_id: LIB_EDITOR_FORM_ID,
                heading_key: kind.results_heading_key,
                failed_key: kind.failed_key,
                export_href: export_href.clone(),
                state: RunResultsState::Empty,
            }),
        ),
    };
    // #842/04: this endpoint never runs `$sql-run`, so Columns is always
    // its own empty skeleton — gated identically to `tables_card`.
    let columns_card = tables_card
        .is_some()
        .then(|| build_columns_card(i18n, kind, Vec::new(), false));
    SqlLibraryPage {
        status: current_status(state, version, rt),
        i18n,
        active_page: kind.active_page,
        base_href: kind.base_href,
        run_href: kind.run_href,
        title_key: kind.title_key,
        lede_key: kind.lede_key,
        new_title_key: kind.new_title_key,
        all_heading_key: kind.all_heading_key,
        filter_placeholder_key: kind.filter_placeholder_key,
        rail_empty_key: kind.rail_empty_key,
        editor_heading_key: kind.editor_heading_key,
        empty_title_key: kind.empty_title_key,
        empty_lede_key: kind.empty_lede_key,
        chip_key: kind.chip_key,
        icon_svg: kind.icon_svg,
        rail: Vec::new(),
        filter: String::new(),
        degraded: None,
        selected: Some(SelectedLib {
            name: if is_new { String::new() } else { id.clone() },
            id,
            json,
            sql,
            status,
            status_class,
        }),
        is_new,
        details: Some(details),
        declares_parameters: kind.declares_parameters,
        params_card,
        tables_card,
        columns_card,
        run_results,
        save_error,
        saved: false,
        // Neither a save nor a navigation — there is nothing new to record
        // and no rail to repaint.
        recent_entries: Vec::new(),
        rail_page: kind.page.key(),
        max_recent: rail_state::MAX_RECENT,
    }
}

async fn sql_library_save(
    state: WebState,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    user_key: String,
    form: SqlLibSaveForm,
    kind: &LibraryKind,
) -> Response {
    // A form-validation error re-renders in place: nothing has run
    // server-side, no rail to repaint, and the submitted text is kept
    // rather than lost — [`render_lib_document_page`]'s own shape, shared
    // with the `document` endpoint's no-JS response (#841/#842). A plain
    // (not `async`) closure: calling `render_lib_document_page` here only
    // builds its `Future`, so every call site below still has to `.await`
    // it — the closure itself stays a bare `Fn`, callable more than once,
    // which an `async` closure capturing `&state`/`&rt` by move could not
    // be.
    let error_page = |save_error: String,
                      json: String,
                      sql: String,
                      is_new: bool,
                      id: String,
                      status: String,
                      values: std::collections::HashMap<String, String>| {
        render_lib_document_page(
            &state,
            locale,
            rv.0,
            &rt,
            kind,
            &user_key,
            json,
            sql,
            is_new,
            id,
            status,
            values,
            Some(save_error),
            AddParamFormState::default(),
            AddTableFormState::default(),
        )
    };

    let duplicate = form.action == "duplicate";
    let mut resource: serde_json::Value = match serde_json::from_str(form.json.trim()) {
        Ok(value) => value,
        Err(e) => {
            // No parsed resource to read a status off of — the JSON itself
            // never parsed, so the re-rendered chip is empty (`unknown`).
            return render(
                error_page(
                    format!("invalid JSON: {e}"),
                    form.json,
                    form.sql,
                    form.id.is_empty(),
                    form.id,
                    String::new(),
                    form.values,
                )
                .await,
            );
        }
    };
    if resource
        .get("resourceType")
        .and_then(serde_json::Value::as_str)
        != Some("Library")
    {
        let status = sql_libraries::extract_status(&resource);
        return render(
            error_page(
                "the document must have resourceType \"Library\"".to_string(),
                form.json,
                form.sql,
                form.id.is_empty(),
                form.id,
                status,
                form.values,
            )
            .await,
        );
    }
    // #840: this page only ever shows and saves Libraries of its own kind —
    // saving a `sql-view` from SQL Queries (or the reverse) would silently
    // vanish it from the rail it was just edited on. Checked ahead of
    // `embed_sql` below, against the resource exactly as submitted, so a
    // rejected Save changes nothing about what the user typed.
    if !sql_libraries::has_library_code(&resource, kind.code) {
        let status = sql_libraries::extract_status(&resource);
        return render(
            error_page(
                I18n::new(locale).t_arg("lib-save-wrong-kind", "code", kind.code.to_string()),
                form.json,
                form.sql,
                form.id.is_empty(),
                form.id,
                status,
                form.values,
            )
            .await,
        );
    }
    // #841: a SQL View's own profile fixes `Library.parameter` to
    // `0..0` — reject a save (or Duplicate) that would persist a non-empty
    // one rather than silently keeping declarations the page never lets the
    // user act on. Checked after #840's own type gate above, against the
    // resource exactly as submitted (before `embed_sql`), same as it is.
    if !kind.declares_parameters
        && resource
            .get("parameter")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|params| !params.is_empty())
    {
        let status = sql_libraries::extract_status(&resource);
        return render(
            error_page(
                I18n::new(locale).t("lib-save-view-parameters"),
                form.json,
                form.sql,
                form.id.is_empty(),
                form.id,
                status,
                form.values,
            )
            .await,
        );
    }
    sql_libraries::embed_sql(&mut resource, &form.sql);
    // Read before `resource` moves into `save_resource` below — only the
    // save-failure branch needs it, but the value must be captured here.
    let status = sql_libraries::extract_status(&resource);

    let id = if duplicate || form.id.is_empty() {
        if let Some(map) = resource.as_object_mut() {
            map.remove("id");
            if duplicate && let Some(name) = map.get("name").and_then(serde_json::Value::as_str) {
                let copy = format!("{name}_copy");
                map.insert("name".to_string(), serde_json::Value::String(copy));
            }
        }
        None
    } else {
        if let Some(map) = resource.as_object_mut() {
            map.insert("id".to_string(), serde_json::Value::String(form.id.clone()));
        }
        Some(form.id.clone())
    };

    match state
        .conformance
        .save_resource("Library", id.as_deref(), resource, rv.0, &rt.id)
        .await
    {
        Ok(stored) => {
            let stored_id = stored
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            axum::response::Redirect::to(&format!("{}?lib={stored_id}&saved=1", kind.base_href))
                .into_response()
        }
        Err(error) => render(
            error_page(
                error,
                form.json,
                form.sql,
                id.is_none(),
                form.id,
                status,
                form.values,
            )
            .await,
        ),
    }
}

#[derive(Default)]
struct SqlLibRunForm {
    /// The Library id the posted document was opened from, empty for an
    /// unsaved one — only ever used to gate the Export action's `href`
    /// (#839); a stored Library is never read back through it, so a
    /// mismatched or nonexistent id changes nothing about what runs.
    id: String,
    /// The editor's full text, exactly as posted — never reformatted or
    /// re-serialized before either parsing it or embedding `sql` into it.
    json: String,
    /// The SQL pane's exact posted text, embedded into `json`'s
    /// `application/sql` attachment the same way Save does
    /// ([`sql_libraries::embed_sql`]).
    sql: String,
    /// Every submitted `param:{name}` value (#841), keyed by name.
    values: std::collections::HashMap<String, String>,
    /// The `params_sig` hidden field's own value (#841) — the browser's own
    /// record of what the Parameters card last declared.
    params_sig: String,
    /// The `tables_sig` hidden field's own value (#842) — the browser's own
    /// record of what the Tables panel's *Reads from* card last declared.
    tables_sig: String,
}

/// Parses `SqlLibRunForm`'s fields out of a raw urlencoded body (#841): the
/// dynamic `param:{name}` values a `#[derive(Deserialize)]` struct cannot
/// express, alongside the fixed `id`/`json`/`sql`/`params_sig`/`tables_sig`
/// fields — the same by-hand parse [`parse_lib_save_form`] uses for its own
/// `values`.
fn parse_lib_run_form(body: &[u8]) -> SqlLibRunForm {
    let mut form = SqlLibRunForm::default();
    for (key, value) in form_urlencoded::parse(body) {
        if let Some(name) = key.strip_prefix("param:") {
            form.values.insert(name.to_string(), value.into_owned());
            continue;
        }
        match key.as_ref() {
            "id" => form.id = value.into_owned(),
            "json" => form.json = value.into_owned(),
            "sql" => form.sql = value.into_owned(),
            "params_sig" => form.params_sig = value.into_owned(),
            "tables_sig" => form.tables_sig = value.into_owned(),
            _ => {}
        }
    }
    form
}

/// The `/run` fragment's own Columns-card companion (#842/04) — mirrors
/// `params_card`/`tables_card`'s own `Option` shape, but *always* carries
/// something: unlike those two cards (silent when their own signature
/// hasn't changed), every fragment response says *something* new about
/// Columns — the full card, replaced wholesale, on a good run,
/// or just its own meta relabelled "last successful run" on any failure
/// (a JSON parse failure, the wrong `resourceType`, a SQL View's own
/// `parameter[]`, an unknown table, a required parameter with no value, or
/// an actual `$sql-run` failure — every arm but a genuine success).
enum ColumnsFragment {
    Filled(LibColumnsCard),
    Stale(String),
}

/// The `/run` fragment's whole response (#841, extended for #842/#842-04):
/// the results notice (targeted directly by the request that triggered
/// it) plus, only when the freshly computed signature differs from what
/// the browser posted, the Parameters and/or Tables cards riding along as
/// their own `hx-swap-oob` companions, and — always — the Columns card's
/// own update — one Askama template nesting all of them, the same way
/// every other multi-part response in this crate nests one `Template`'s
/// render inside another's rather than concatenating strings by hand.
/// `params_card` is always `None` for SQL Views (the card never shows
/// there) and for a SQL Query render whose signature matched (the card
/// never travels just because a value changed); `tables_card` is `None`
/// whenever the (unknown-table-extended) signature matched, for both
/// kinds.
#[derive(Template)]
#[template(path = "partials/lib_run_fragment.html")]
struct LibRunFragment {
    run_results: LibRunNotice,
    params_card: Option<LibParamsCard>,
    tables_card: Option<LibTablesCard>,
    columns: ColumnsFragment,
}

/// `POST /ui/sql/queries/run` and `POST /ui/sql/views/run` (#839, extended
/// for #841): the Library-backed playgrounds' live preview fragment, the
/// same shape [`sql_view_definitions_run`] gives View Definitions. Always
/// runs the *posted* `json`/`sql` — saved or not, and never a lookup of `id`
/// against storage — through `$sql-run`, embedding `sql` into `json` first
/// exactly as Save does. Renders [`LibRunFragment`] (`partials/sql_run_
/// results.html` plus, when it changed, the Parameters card's own OOB
/// companion).
///
/// Unlike [`sql_view_definitions_run`], this always answers `200`: `form`
/// comes from [`parse_lib_run_form`]'s own by-hand parse (`param:{name}`
/// needs one, #841), which — like [`sql_export::start`]'s identical
/// `RawForm` parse — never fails to extract, so a missing `json`/`sql`
/// simply parses (or fails to parse, `RunResultsState::Failure`) as the
/// empty string, itself already a normal fragment response; there is no
/// "malformed body" case left for this endpoint to 4xx on.
///
/// Builds the `/run` fragment's own Columns update for a *successful* run
/// (#842/04, NF1): fetches the tenant's Library list, resolves whichever of
/// `deps` the SQL actually reads down to their ViewDefinitions
/// (`sql_libraries::dependencies_used_by_sql`,
/// [`resolve_view_definition_dependencies`]), and analyzes `table`/
/// `raw_rows` against them ([`sql_libraries::analyze_columns`]). A SQL
/// Query and a SQL View share this identical "good run" shape, differing
/// only in whether declared parameters were involved getting here — called
/// from both of [`sql_library_run`]'s own branches, never from a failure
/// path (see [`ColumnsFragment::Stale`] for those).
#[allow(clippy::too_many_arguments)]
async fn columns_fragment_for_success(
    state: &WebState,
    version: helios_fhir::FhirVersion,
    tenant: &str,
    i18n: I18n,
    kind: &LibraryKind,
    table: &sql_views::RunTable,
    raw_rows: &[serde_json::Value],
    sql: &str,
    deps: &[sql_libraries::TableDependency],
) -> ColumnsFragment {
    let used_deps = sql_libraries::dependencies_used_by_sql(sql, deps);
    let libraries = state
        .conformance
        .fetch("Library", version, tenant)
        .await
        .unwrap_or_default();
    let view_definitions =
        resolve_view_definition_dependencies(state, version, tenant, &used_deps, &libraries).await;
    let columns_rows = sql_libraries::analyze_columns(&table.columns, raw_rows, &view_definitions);
    ColumnsFragment::Filled(build_columns_card(i18n, kind, columns_rows, true))
}

/// #841's own decision order ahead of `$sql-run`, extended by #842/04's own
/// unknown-table lint — one clock, one source of truth, rather than a
/// separate check running on its own schedule:
///
/// 1. A SQL View with a non-empty `parameter[]` never runs (its own
///    profile forbids the declaration at all).
/// 2. The SQL reading a table no declared label names never runs either —
///    the unknown-table lint's own notice, [`LibRunNotice::UnknownTables`].
/// 3. A SQL Query with an unfilled required parameter never runs either
///    (the "waiting" notice, [`RunResultsState::Waiting`]).
/// 4. Only once every check above passes does this call `$sql-run`, with
///    that render's own bindings.
///
/// The Columns card ([`ColumnsFragment`]) rides every response: `Filled`
/// only for step 4's own success, `Stale` for every other outcome
/// (#842/04's own NF1 — its dependency resolution never runs on a request
/// that stops at step 1, 2, or 3, or that reaches step 4 and fails).
async fn sql_library_run(
    state: WebState,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    user_key: String,
    form: SqlLibRunForm,
    kind: &LibraryKind,
) -> Response {
    let i18n = I18n::new(locale);
    let export_href = export_href(kind, &form.id);
    let standard = |state: RunResultsState| {
        LibRunNotice::Standard(RunResultsPartial {
            i18n,
            fragment: true,
            run_href: kind.run_href,
            form_id: LIB_EDITOR_FORM_ID,
            heading_key: kind.results_heading_key,
            failed_key: kind.failed_key,
            export_href: export_href.clone(),
            state,
        })
    };
    let stale_columns = || ColumnsFragment::Stale(i18n.t("vd-results-stale"));
    let respond = |run_results: LibRunNotice,
                   params_card: Option<LibParamsCard>,
                   tables_card: Option<LibTablesCard>,
                   columns: ColumnsFragment| {
        render(LibRunFragment {
            run_results,
            params_card,
            tables_card,
            columns,
        })
    };

    // A JSON parse failure never reaches $sql-run. NF1: never log
    // `form.json`/`form.sql`/`form.values` themselves — a Library's
    // embedded SQL, JSON body, or parameter values can carry PHI.
    let mut resource: serde_json::Value = match serde_json::from_str(form.json.trim()) {
        Ok(value) => value,
        Err(error) => {
            return respond(
                standard(RunResultsState::Failure(
                    format!("invalid JSON: {error}"),
                    None,
                )),
                None,
                None,
                stale_columns(),
            );
        }
    };
    if resource
        .get("resourceType")
        .and_then(serde_json::Value::as_str)
        != Some("Library")
    {
        return respond(
            standard(RunResultsState::Failure(
                "the document must have resourceType \"Library\"".to_string(),
                None,
            )),
            None,
            None,
            stale_columns(),
        );
    }

    // #841: a SQL View's own profile fixes `Library.parameter` to
    // `0..0` — this kind never declares parameters, so it never builds the
    // Parameters card (`kind.declares_parameters` gates that below) and
    // never calls `$sql-run` for a document that carries a declaration
    // anyway (however it got there — Details, most likely).
    if !kind.declares_parameters
        && resource
            .get("parameter")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|params| !params.is_empty())
    {
        return respond(
            standard(RunResultsState::Failure(
                i18n.t("lib-save-view-parameters"),
                None,
            )),
            None,
            None,
            stale_columns(),
        );
    }

    // #842: the Tables panel resolves identically for both kinds, ahead of
    // the SQL Query/SQL View branches below — but only when the freshly
    // computed (unknown-table-extended, #842/04) signature differs from
    // what the browser posted (NF1): a keystroke that changes only a
    // value, or only the SQL text in a way that doesn't add or remove an
    // unknown table, never touches this signature, so most `/run` requests
    // skip this fetch entirely. The unknown-table scan itself is pure and
    // always runs — the run/notice gate below needs its result regardless
    // of whether the card travels.
    let deps = sql_libraries::table_dependencies(&resource);
    let unknown_tables = sql_libraries::unknown_tables(&form.sql, &deps);
    let unknown_names: Vec<String> = unknown_tables.iter().map(|t| t.name.clone()).collect();
    let tables_signature = sql_libraries::tables_signature_with_unknown(&deps, &unknown_names);
    let tables_card = if tables_signature != form.tables_sig {
        let libraries = state
            .conformance
            .fetch("Library", rv.0, &rt.id)
            .await
            .unwrap_or_default();
        let jobs = sql_export::jobs_for_used_by(&state, &user_key, &rt.id).await;
        let analysis = analyze_tables(
            &state,
            rv.0,
            &rt.id,
            &resource,
            &deps,
            &unknown_tables,
            &libraries,
            &jobs,
        )
        .await;
        Some(build_tables_card(
            i18n,
            kind,
            analysis,
            table_options_href(&form.id),
            TablesCardOptions {
                oob: true,
                ..Default::default()
            },
        ))
    } else {
        None
    };

    // #842/04: the SQL reads a table no declared label names —
    // never call `$sql-run` at all, so a stray table name never
    // materializes an in-memory database for nothing. The previous good
    // table (if any) is left untouched (`RunResultsPartial`'s own OOB
    // shape for a failure, mirrored here) and Columns' own meta relabels
    // to "last successful run".
    if !unknown_tables.is_empty() {
        return respond(
            LibRunNotice::UnknownTables(unknown_tables_notice(i18n, true, &unknown_tables)),
            None,
            tables_card,
            stale_columns(),
        );
    }

    if !kind.declares_parameters {
        sql_libraries::embed_sql(&mut resource, &form.sql);
        let (run_results, columns) =
            match run_sql_preview(&state, &resource, &[], rv.0, &rt.id).await {
                Ok((table, raw_rows, ms)) => {
                    tracing::debug!(
                        rows = table.rows.len(),
                        ms,
                        kind = kind.code,
                        "ran a Library preview"
                    );
                    let columns = columns_fragment_for_success(
                        &state, rv.0, &rt.id, i18n, kind, &table, &raw_rows, &form.sql, &deps,
                    )
                    .await;
                    (standard(RunResultsState::Success(table, ms)), columns)
                }
                Err(error) => {
                    let line = sql_views::extract_error_line(&error);
                    (
                        standard(RunResultsState::Failure(error, line)),
                        stale_columns(),
                    )
                }
            };
        return respond(run_results, None, tables_card, columns);
    }

    // #841: SQL Query — declared parameters, undeclared-placeholder hints,
    // and the values/bindings this run supplies, all from one analysis so
    // the signature comparison below and the run itself never disagree.
    let analysis = analyze_params(&resource, &form.sql, &form.values);
    let oob = analysis.signature != form.params_sig;

    let (run_results, columns) = if !analysis.missing_required.is_empty() {
        (
            standard(RunResultsState::Waiting(waiting_message(
                &i18n,
                &analysis.missing_required,
            ))),
            stale_columns(),
        )
    } else {
        sql_libraries::embed_sql(&mut resource, &form.sql);
        // Borrowed across the `await`, not cloned: `analysis` itself is
        // untouched until after this call returns, so its own `bindings`
        // stay valid for the whole request.
        match run_sql_preview(&state, &resource, &analysis.bindings, rv.0, &rt.id).await {
            Ok((table, raw_rows, ms)) => {
                tracing::debug!(
                    rows = table.rows.len(),
                    ms,
                    kind = kind.code,
                    "ran a Library preview"
                );
                let columns = columns_fragment_for_success(
                    &state, rv.0, &rt.id, i18n, kind, &table, &raw_rows, &form.sql, &deps,
                )
                .await;
                (standard(RunResultsState::Success(table, ms)), columns)
            }
            Err(error) => {
                let line = sql_views::extract_error_line(&error);
                (
                    standard(RunResultsState::Failure(error, line)),
                    stale_columns(),
                )
            }
        }
    };
    let params_card = oob.then(|| {
        build_params_card(
            i18n,
            kind,
            analysis,
            ParamsCardOptions {
                oob: true,
                ..Default::default()
            },
        )
    });
    respond(run_results, params_card, tables_card, columns)
}

#[derive(Default)]
struct SqlLibDocumentForm {
    id: String,
    json: String,
    sql: String,
    /// Every submitted `param:{name}` value (#841) — echoed back into the
    /// Parameters card's own fields on every response this endpoint
    /// produces, exactly as `/run` does.
    values: std::collections::HashMap<String, String>,
    params_sig: String,
    /// `add-parameter`/`add-table` (#841/#842) — *Remove* has no `op` value
    /// of its own; see [`Self::table_label`]. Ignored when
    /// [`Self::declare_param`]/[`Self::table_label`] is `Some`.
    op: String,
    param_name: String,
    param_type: String,
    /// The `declare_param` field a *Declare :name* button submits as its
    /// own name/value pair (#841) — present only when that button,
    /// not the `Add parameter` panel's own submit, triggered this request;
    /// its value is the placeholder name to declare, type `string`.
    declare_param: Option<String>,
    /// `op=add-table`'s own `table` field (#842) — the reference the *Add
    /// table* combobox (or its no-JS fallback textarea) submits.
    table: String,
    /// `op=add-table`'s own `table_alias` field (#842).
    table_alias: String,
    /// One row's own *Remove* button's own name/value pair (#842) — the
    /// exact `relatedArtifact.label` of the dependency it removes, present
    /// only when that specific button (not the *Add table* panel's own
    /// submit) triggered this request. Mirrors [`Self::declare_param`]'s own
    /// "the field's presence is the operation" shape: with several *Remove*
    /// buttons sharing one `<form>`, only the one actually clicked
    /// contributes its name/value pair at all — there is no single shared
    /// `op=remove-table` value every row's button could carry alongside its
    /// own label without a second name/value pair, which a plain HTML
    /// `<button>` cannot have.
    table_label: Option<String>,
}

/// Parses `SqlLibDocumentForm`'s fields out of a raw urlencoded body
/// (#841/#842) — the same by-hand parse [`parse_lib_run_form`]/
/// [`parse_lib_save_form`] use for their own `values`.
fn parse_lib_document_form(body: &[u8]) -> SqlLibDocumentForm {
    let mut form = SqlLibDocumentForm::default();
    for (key, value) in form_urlencoded::parse(body) {
        if let Some(name) = key.strip_prefix("param:") {
            form.values.insert(name.to_string(), value.into_owned());
            continue;
        }
        match key.as_ref() {
            "id" => form.id = value.into_owned(),
            "json" => form.json = value.into_owned(),
            "sql" => form.sql = value.into_owned(),
            "params_sig" => form.params_sig = value.into_owned(),
            "op" => form.op = value.into_owned(),
            "param_name" => form.param_name = value.into_owned(),
            "param_type" => form.param_type = value.into_owned(),
            "declare_param" => form.declare_param = Some(value.into_owned()),
            "table" => form.table = value.into_owned(),
            "table_alias" => form.table_alias = value.into_owned(),
            "table_label" => form.table_label = Some(value.into_owned()),
            _ => {}
        }
    }
    form
}

/// Translates one [`sql_libraries::AddParameterError`] into its
/// `lib-params-add-*` catalog message (#841) — this module's own split
/// from `sql_libraries`, which never localizes (matching
/// [`sql_libraries::parameters`]'s own convention).
fn add_parameter_error_message(i18n: &I18n, error: sql_libraries::AddParameterError) -> String {
    match error {
        sql_libraries::AddParameterError::InvalidName => i18n.t("lib-params-add-invalid-name"),
        sql_libraries::AddParameterError::DuplicateName(name) => {
            i18n.t_arg("lib-params-add-duplicate", "name", name)
        }
        sql_libraries::AddParameterError::UnknownType => i18n.t("lib-params-add-unknown-type"),
    }
}

/// One mutation `POST …/document` can apply (#841/#842) — [`SqlLibDocumentForm`]'s
/// own submission, parsed into the shape each `apply_*` function below
/// actually needs. [`parse_doc_mutation`] is the only place that reads
/// `form.op`/`form.declare_param` directly; everything downstream matches on
/// this enum instead.
enum DocMutation {
    AddParameter { name: String, type_code: String },
    AddTable { table: String, alias: String },
    RemoveTable { label: String },
}

/// Reads `form`'s own `op` (or `declare_param`/`table_label`, a single
/// button's own shorthand for a mutation `op` cannot itself carry a second
/// field for, #841/#842) into a [`DocMutation`] — `None` for anything else,
/// which [`sql_library_document`] answers with a bare `400` rather than
/// silently doing nothing.
fn parse_doc_mutation(form: &SqlLibDocumentForm) -> Option<DocMutation> {
    if let Some(name) = &form.declare_param {
        return Some(DocMutation::AddParameter {
            name: name.trim().to_string(),
            type_code: "string".to_string(),
        });
    }
    if let Some(label) = &form.table_label {
        return Some(DocMutation::RemoveTable {
            label: label.clone(),
        });
    }
    match form.op.as_str() {
        "add-parameter" => Some(DocMutation::AddParameter {
            name: form.param_name.trim().to_string(),
            type_code: form.param_type.clone(),
        }),
        "add-table" => Some(DocMutation::AddTable {
            table: form.table.trim().to_string(),
            alias: form.table_alias.trim().to_string(),
        }),
        _ => None,
    }
}

/// A whole-document failure (invalid JSON, or the wrong `resourceType`) is
/// treated the same as `/run`'s own check (#841/#842) — identical message
/// text, no mutation attempted — surfaced through whichever card's own
/// *Add* error slot `mutation` belongs to on the `HX-Request` path (there is
/// no other notice a card-only response can show it in) and the page-level
/// `.notice--warn` banner on the no-JS path (matching Save's own shape for
/// the same failure — both cards' own `add` state resets to
/// [`AddParamFormState::default`]/[`AddTableFormState::default`] there,
/// since the whole document failed to parse and there is nothing of either
/// panel's own submission left worth re-showing next to the banner).
#[allow(clippy::too_many_arguments)]
async fn document_whole_error_response(
    state: &WebState,
    locale: RequestLocale,
    version: helios_fhir::FhirVersion,
    rt: &RequestTenant,
    kind: &LibraryKind,
    user_key: &str,
    is_htmx: bool,
    form: &SqlLibDocumentForm,
    mutation: &DocMutation,
    message: String,
) -> Response {
    let i18n = I18n::new(locale);
    if is_htmx {
        match mutation {
            DocMutation::AddParameter { name, type_code } => render(LibParamsCard {
                i18n,
                run_href: kind.run_href,
                document_href: kind.document_href,
                fields: Vec::new(),
                hints: Vec::new(),
                type_options: helios_sof::sqlquery::BINDABLE_PARAMETER_TYPES,
                signature: String::new(),
                add_name: name.clone(),
                add_type: type_code.clone(),
                add_open: true,
                add_error: Some(message),
                oob: false,
                data_document: None,
            }),
            DocMutation::AddTable { .. } | DocMutation::RemoveTable { .. } => {
                render(LibTablesCard {
                    i18n,
                    document_href: kind.document_href,
                    table_options_href: table_options_href(&form.id),
                    rows: Vec::new(),
                    unknown_rows: Vec::new(),
                    used_by: Vec::new(),
                    signature: String::new(),
                    add_table: form.table.clone(),
                    add_alias: form.table_alias.clone(),
                    add_open: true,
                    add_error: Some(message),
                    oob: false,
                    data_document: None,
                })
            }
        }
    } else {
        render(
            render_lib_document_page(
                state,
                locale,
                version,
                rt,
                kind,
                user_key,
                form.json.clone(),
                form.sql.clone(),
                form.id.is_empty(),
                form.id.clone(),
                String::new(),
                form.values.clone(),
                Some(message),
                AddParamFormState::default(),
                AddTableFormState::default(),
            )
            .await,
        )
    }
}

/// [`DocMutation::AddParameter`]'s own handler (#841) — unchanged behavior
/// from before #842's own `op` dispatch grew a `match`, just moved into its
/// own function alongside [`apply_add_table`]/[`apply_remove_table`].
#[allow(clippy::too_many_arguments)]
async fn apply_add_parameter(
    state: &WebState,
    locale: RequestLocale,
    version: helios_fhir::FhirVersion,
    rt: &RequestTenant,
    kind: &LibraryKind,
    user_key: &str,
    is_htmx: bool,
    form: SqlLibDocumentForm,
    document: serde_json::Value,
    name: String,
    type_code: String,
) -> Response {
    let i18n = I18n::new(locale);
    let mut updated = document.clone();
    match sql_libraries::add_parameter(&mut updated, &name, &type_code) {
        Ok(()) => {
            // The updated document, pretty-printed exactly as Details
            // itself always shows it — never the SQL-embedded form
            // `/run`/Save build; this endpoint never touches `content`.
            let updated_pretty = serde_json::to_string_pretty(&updated).unwrap_or_default();
            if is_htmx {
                let analysis = analyze_params(&updated, &form.sql, &form.values);
                render(build_params_card(
                    i18n,
                    kind,
                    analysis,
                    ParamsCardOptions {
                        // Closed again after a successful add (#841) — with
                        // JS this also happens implicitly, since
                        // `host.setDoc` swaps in this same server-rendered
                        // (closed) card once it strips `data-document` off.
                        add: AddParamFormState::default(),
                        oob: false,
                        data_document: Some(updated_pretty),
                    },
                ))
            } else {
                let status = sql_libraries::extract_status(&updated);
                render(
                    render_lib_document_page(
                        state,
                        locale,
                        version,
                        rt,
                        kind,
                        user_key,
                        updated_pretty,
                        form.sql,
                        form.id.is_empty(),
                        form.id,
                        status,
                        form.values.clone(),
                        None,
                        AddParamFormState::default(),
                        AddTableFormState::default(),
                    )
                    .await,
                )
            }
        }
        Err(error) => {
            let message = add_parameter_error_message(&i18n, error);
            let add = AddParamFormState {
                name,
                type_code,
                open: true,
                error: Some(message),
            };
            if is_htmx {
                let analysis = analyze_params(&document, &form.sql, &form.values);
                render(build_params_card(
                    i18n,
                    kind,
                    analysis,
                    ParamsCardOptions {
                        add,
                        oob: false,
                        data_document: None,
                    },
                ))
            } else {
                let status = sql_libraries::extract_status(&document);
                render(
                    render_lib_document_page(
                        state,
                        locale,
                        version,
                        rt,
                        kind,
                        user_key,
                        form.json,
                        form.sql,
                        form.id.is_empty(),
                        form.id,
                        status,
                        form.values.clone(),
                        None,
                        add,
                        AddTableFormState::default(),
                    )
                    .await,
                )
            }
        }
    }
}

/// Resolves `table_ref` — an *Add table* submission, always the relative
/// `ViewDefinition/{id}`/`Library/{id}` shape the combobox's own `value` (or
/// the fallback textarea's documented hint) uses, never a canonical URL —
/// into the artifact it names, once [`sql_libraries::AddTableError`]'s own
/// gate (#842) is satisfied: it must actually resolve, resolve to a
/// ViewDefinition or a `sql-view` Library, and — for a Library — not be
/// `own_id` (`Type/id` alone cannot rule out a self-canonical reference,
/// which is why this check lives here rather than in
/// [`sql_libraries::dependency_lookup`]). `None` for anything else,
/// including a bare canonical URL — #842's own *Add table* never offers
/// one, and accepting it here would let a hand-edited fallback submission
/// bypass the "distinct from the artifact itself" rule for a Library
/// matched by `url` rather than `id`.
async fn resolve_table_target(
    state: &WebState,
    version: helios_fhir::FhirVersion,
    tenant: &str,
    table_ref: &str,
    own_id: &str,
) -> Option<serde_json::Value> {
    let sql_libraries::DependencyLookup::TypeId { resource_type, id } =
        sql_libraries::dependency_lookup(table_ref)
    else {
        return None;
    };
    if resource_type != "ViewDefinition" && resource_type != "Library" {
        return None;
    }
    let artifact = state
        .conformance
        .read_resource(resource_type, &id, version, tenant)
        .await
        .ok()?;
    let valid = match resource_type {
        "ViewDefinition" => true,
        "Library" => {
            sql_libraries::has_library_code(&artifact, "sql-view")
                && !(!own_id.is_empty() && id == own_id)
        }
        _ => false,
    };
    valid.then_some(artifact)
}

/// [`DocMutation::AddTable`]'s own handler (#842): resolves `table`,
/// then validates the alias in the order the spec documents — target
/// present and resolvable, alias present (falling back to the target's own
/// `name`), alias shape, alias uniqueness (case-insensitive) — appending
/// `{"type": "depends-on", "label": alias, "resource": R}` on success, `R`
/// the target's own `url` when it has one, else its `Type/id`
/// (#842's own "no version pin" rule).
#[allow(clippy::too_many_arguments)]
async fn apply_add_table(
    state: &WebState,
    locale: RequestLocale,
    version: helios_fhir::FhirVersion,
    rt: &RequestTenant,
    kind: &LibraryKind,
    user_key: &str,
    is_htmx: bool,
    form: SqlLibDocumentForm,
    document: serde_json::Value,
    table: String,
    alias_input: String,
) -> Response {
    let i18n = I18n::new(locale);
    let own_id = document
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();

    let target = resolve_table_target(state, version, &rt.id, &table, &own_id).await;
    let outcome = match &target {
        None => Err(i18n.t("lib-tables-add-error-required")),
        Some(artifact) => {
            let default_alias = artifact
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let alias = if alias_input.is_empty() {
                default_alias
            } else {
                Some(alias_input.clone())
            };
            match alias {
                None => Err(i18n.t("lib-tables-add-error-alias-required")),
                Some(alias) if !sql_libraries::is_valid_parameter_name(&alias) => {
                    Err(i18n.t("lib-tables-add-error-alias-invalid"))
                }
                Some(alias) if sql_libraries::label_declared(&document, &alias) => {
                    Err(i18n.t_arg("lib-tables-add-error-alias-duplicate", "alias", alias))
                }
                Some(alias) => {
                    let resource_type = artifact
                        .get("resourceType")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    let id = artifact
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    let resource_ref = artifact
                        .get("url")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("{resource_type}/{id}"));
                    Ok((alias, resource_ref))
                }
            }
        }
    };

    match outcome {
        Ok((alias, resource_ref)) => {
            let mut updated = document.clone();
            sql_libraries::add_table(&mut updated, &alias, &resource_ref);
            let updated_pretty = serde_json::to_string_pretty(&updated).unwrap_or_default();
            if is_htmx {
                let deps = sql_libraries::table_dependencies(&updated);
                let unknown_tables = sql_libraries::unknown_tables(&form.sql, &deps);
                let analysis = full_tables_analysis(
                    state,
                    version,
                    &rt.id,
                    user_key,
                    &updated,
                    &deps,
                    &unknown_tables,
                )
                .await;
                render(build_tables_card(
                    i18n,
                    kind,
                    analysis,
                    table_options_href(&form.id),
                    TablesCardOptions {
                        // Closed again after a successful add (#842) —
                        // mirrors `apply_add_parameter`'s own rule.
                        add: AddTableFormState::default(),
                        oob: false,
                        data_document: Some(updated_pretty),
                    },
                ))
            } else {
                let status = sql_libraries::extract_status(&updated);
                render(
                    render_lib_document_page(
                        state,
                        locale,
                        version,
                        rt,
                        kind,
                        user_key,
                        updated_pretty,
                        form.sql,
                        form.id.is_empty(),
                        form.id,
                        status,
                        form.values.clone(),
                        None,
                        AddParamFormState::default(),
                        AddTableFormState::default(),
                    )
                    .await,
                )
            }
        }
        Err(message) => {
            let add = AddTableFormState {
                table,
                alias: alias_input,
                open: true,
                error: Some(message),
            };
            if is_htmx {
                let deps = sql_libraries::table_dependencies(&document);
                let unknown_tables = sql_libraries::unknown_tables(&form.sql, &deps);
                let analysis = full_tables_analysis(
                    state,
                    version,
                    &rt.id,
                    user_key,
                    &document,
                    &deps,
                    &unknown_tables,
                )
                .await;
                render(build_tables_card(
                    i18n,
                    kind,
                    analysis,
                    table_options_href(&form.id),
                    TablesCardOptions {
                        add,
                        oob: false,
                        data_document: None,
                    },
                ))
            } else {
                let status = sql_libraries::extract_status(&document);
                render(
                    render_lib_document_page(
                        state,
                        locale,
                        version,
                        rt,
                        kind,
                        user_key,
                        form.json,
                        form.sql,
                        form.id.is_empty(),
                        form.id,
                        status,
                        form.values.clone(),
                        None,
                        AddParamFormState::default(),
                        add,
                    )
                    .await,
                )
            }
        }
    }
}

/// [`DocMutation::RemoveTable`]'s own handler (#842) — always
/// "succeeds": a `label` matching no `depends-on` entry leaves the document
/// untouched (`sql_libraries::remove_table`'s own contract) rather than
/// erroring, so this never has a failure branch to render.
#[allow(clippy::too_many_arguments)]
async fn apply_remove_table(
    state: &WebState,
    locale: RequestLocale,
    version: helios_fhir::FhirVersion,
    rt: &RequestTenant,
    kind: &LibraryKind,
    user_key: &str,
    is_htmx: bool,
    form: SqlLibDocumentForm,
    document: serde_json::Value,
    label: String,
) -> Response {
    let i18n = I18n::new(locale);
    let mut updated = document.clone();
    sql_libraries::remove_table(&mut updated, &label);
    let updated_pretty = serde_json::to_string_pretty(&updated).unwrap_or_default();
    if is_htmx {
        let deps = sql_libraries::table_dependencies(&updated);
        let unknown_tables = sql_libraries::unknown_tables(&form.sql, &deps);
        let analysis = full_tables_analysis(
            state,
            version,
            &rt.id,
            user_key,
            &updated,
            &deps,
            &unknown_tables,
        )
        .await;
        render(build_tables_card(
            i18n,
            kind,
            analysis,
            table_options_href(&form.id),
            TablesCardOptions {
                data_document: Some(updated_pretty),
                ..Default::default()
            },
        ))
    } else {
        let status = sql_libraries::extract_status(&updated);
        render(
            render_lib_document_page(
                state,
                locale,
                version,
                rt,
                kind,
                user_key,
                updated_pretty,
                form.sql,
                form.id.is_empty(),
                form.id,
                status,
                form.values.clone(),
                None,
                AddParamFormState::default(),
                AddTableFormState::default(),
            )
            .await,
        )
    }
}

/// `POST /ui/sql/queries/document` and `POST /ui/sql/views/document`
/// (#841, extended for #842): the Parameters card's *Add parameter*/
/// *Declare* mutations and the Tables panel's *Add table*/*Remove*
/// mutations, all applied to the *submitted* Details document — never a
/// stored resource — and handed back unsaved, the same "never touches
/// storage" contract Save's own validation-error path already keeps
/// ([`render_lib_document_page`]). [`parse_doc_mutation`] decides which of
/// [`apply_add_parameter`]/[`apply_add_table`]/[`apply_remove_table`]
/// handles the request; an unrecognized submission answers a bare `400`
/// rather than silently doing nothing.
#[allow(clippy::too_many_arguments)]
async fn sql_library_document(
    state: WebState,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    user_key: String,
    is_htmx: bool,
    form: SqlLibDocumentForm,
    kind: &LibraryKind,
) -> Response {
    let Some(mutation) = parse_doc_mutation(&form) else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    // NF1: never log `form.json`/`form.sql`/`form.values` — a Library's
    // embedded SQL, JSON body, or parameter values can carry PHI.
    let document: serde_json::Value = match serde_json::from_str(form.json.trim()) {
        Ok(value) => value,
        Err(error) => {
            return document_whole_error_response(
                &state,
                locale,
                rv.0,
                &rt,
                kind,
                &user_key,
                is_htmx,
                &form,
                &mutation,
                format!("invalid JSON: {error}"),
            )
            .await;
        }
    };
    if document
        .get("resourceType")
        .and_then(serde_json::Value::as_str)
        != Some("Library")
    {
        return document_whole_error_response(
            &state,
            locale,
            rv.0,
            &rt,
            kind,
            &user_key,
            is_htmx,
            &form,
            &mutation,
            "the document must have resourceType \"Library\"".to_string(),
        )
        .await;
    }

    match mutation {
        DocMutation::AddParameter { name, type_code } => {
            apply_add_parameter(
                &state, locale, rv.0, &rt, kind, &user_key, is_htmx, form, document, name,
                type_code,
            )
            .await
        }
        DocMutation::AddTable { table, alias } => {
            apply_add_table(
                &state, locale, rv.0, &rt, kind, &user_key, is_htmx, form, document, table, alias,
            )
            .await
        }
        DocMutation::RemoveTable { label } => {
            apply_remove_table(
                &state, locale, rv.0, &rt, kind, &user_key, is_htmx, form, document, label,
            )
            .await
        }
    }
}

async fn sql_queries_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<axum::Extension<helios_auth::Principal>>,
    Query(query): Query<SqlLibQuery>,
    settings: rail_state::RequestSettings,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    sql_library_page(
        state,
        locale,
        rv,
        rt,
        query,
        &SQL_QUERY_KIND,
        settings,
        user_key,
    )
    .await
}

async fn sql_queries_save(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<axum::Extension<helios_auth::Principal>>,
    axum::extract::RawForm(body): axum::extract::RawForm,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    sql_library_save(
        state,
        locale,
        rv,
        rt,
        user_key,
        parse_lib_save_form(&body),
        &SQL_QUERY_KIND,
    )
    .await
}

async fn sql_queries_run(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<axum::Extension<helios_auth::Principal>>,
    axum::extract::RawForm(body): axum::extract::RawForm,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    sql_library_run(
        state,
        locale,
        rv,
        rt,
        user_key,
        parse_lib_run_form(&body),
        &SQL_QUERY_KIND,
    )
    .await
}

async fn sql_queries_document(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<axum::Extension<helios_auth::Principal>>,
    HxRequest(is_htmx): HxRequest,
    axum::extract::RawForm(body): axum::extract::RawForm,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    sql_library_document(
        state,
        locale,
        rv,
        rt,
        user_key,
        is_htmx,
        parse_lib_document_form(&body),
        &SQL_QUERY_KIND,
    )
    .await
}

async fn sql_views_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<axum::Extension<helios_auth::Principal>>,
    Query(query): Query<SqlLibQuery>,
    settings: rail_state::RequestSettings,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    sql_library_page(
        state,
        locale,
        rv,
        rt,
        query,
        &SQL_VIEW_KIND,
        settings,
        user_key,
    )
    .await
}

async fn sql_views_save(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<axum::Extension<helios_auth::Principal>>,
    axum::extract::RawForm(body): axum::extract::RawForm,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    sql_library_save(
        state,
        locale,
        rv,
        rt,
        user_key,
        parse_lib_save_form(&body),
        &SQL_VIEW_KIND,
    )
    .await
}

async fn sql_views_run(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<axum::Extension<helios_auth::Principal>>,
    axum::extract::RawForm(body): axum::extract::RawForm,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    sql_library_run(
        state,
        locale,
        rv,
        rt,
        user_key,
        parse_lib_run_form(&body),
        &SQL_VIEW_KIND,
    )
    .await
}

async fn sql_views_document(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<axum::Extension<helios_auth::Principal>>,
    HxRequest(is_htmx): HxRequest,
    axum::extract::RawForm(body): axum::extract::RawForm,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    sql_library_document(
        state,
        locale,
        rv,
        rt,
        user_key,
        is_htmx,
        parse_lib_document_form(&body),
        &SQL_VIEW_KIND,
    )
    .await
}

/// The shared cards, already HTML.
///
/// Rendered in the handler rather than from the template so the page keeps
/// one error path: `CapabilityCards` returns `Result`, and a template that
/// called it would have to decide what a half-rendered page looks like.
struct RenderedCapabilityCards {
    summary: String,
    interactions: String,
    operations: String,
    resources: String,
    /// The Raw CapabilityStatement fold (#808 follow-up to #798) — the same
    /// htmx-lazy, paginated JSON tree HTS now renders too, in place of a
    /// second bespoke raw-payload mechanism.
    raw: String,
}

impl RenderedCapabilityCards {
    /// HFS's dialect of the shared cards: the backend-role footnote, the
    /// include/revinclude columns its search layer actually populates, and
    /// the progressively enhanced type filter its ~150-row table needs.
    #[allow(clippy::too_many_arguments)]
    fn render(
        i18n: &I18n,
        view: &helios_ui_chrome::capability::CapabilityView,
        version: helios_fhir::FhirVersion,
        filter: &str,
        raw_requested: bool,
        raw_text: &str,
        raw_url: &str,
        expand_url: &str,
        initial_outline: Option<&capability_json::Outline>,
    ) -> Result<Self, askama::Error> {
        let cards = helios_ui_chrome::capability::CapabilityCards::new(i18n, view)
            .transaction_note_href(Some(ROLE_MATRIX_URL))
            .show_include_columns(true)
            .filter(Some(helios_ui_chrome::capability::ResourceFilter {
                action: "/ui/capability-statement",
                version: version.as_str(),
                value: filter,
            }));
        Ok(Self {
            summary: cards.summary()?,
            interactions: cards.interactions()?,
            operations: cards.operations()?,
            resources: cards.resources()?,
            raw: cards.raw(
                raw_requested,
                raw_text,
                raw_url,
                expand_url,
                initial_outline,
            )?,
        })
    }
}

/// Where the `transaction` footnote sends an operator asking why the verb is
/// advertised here and not on another deployment.
const ROLE_MATRIX_URL: &str = "https://github.com/HeliosSoftware/hfs/blob/main/crates/persistence/README.md#primarysecondary-role-matrix";

/// The read-only CapabilityStatement page (#653): the live `/metadata`
/// answer for the sidebar's tenant and FHIR version, summarized and filterable,
/// with the raw statement one fold away.
#[derive(Template)]
#[template(path = "pages/capability-statement.html")]
struct CapabilityPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    /// The five cards shared with HTS, pre-rendered (#808) — the four summary
    /// cards plus the Raw CapabilityStatement fold. `None` when the self-fetch
    /// failed — HFS degrades the whole page to one warning, where HTS
    /// degrades card by card, because HFS has a single source to fail.
    cards: Option<RenderedCapabilityCards>,
}

#[derive(Deserialize, Default)]
struct CapabilityQuery {
    filter: Option<String>,
    version: Option<String>,
    /// A string flag because the public query spelling is `raw=1`, not a Serde
    /// boolean literal such as `raw=true`.
    raw: Option<String>,
}

fn capability_raw_url(filter: &str, version: helios_fhir::FhirVersion) -> String {
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("raw", "1");
    query.append_pair("version", version.as_str());
    query.append_pair("filter", filter);
    format!("/ui/capability-statement?{}", query.finish())
}

#[derive(Deserialize, Default)]
struct CapabilityJsonQuery {
    version: Option<String>,
    #[serde(default)]
    path: String,
    #[serde(default)]
    offset: usize,
    limit: Option<usize>,
}

fn bounded_capability_fragment(rendered: Result<String, askama::Error>) -> Response {
    match rendered {
        Ok(html) if html.len() <= capability_json::MAX_FRAGMENT_HTML_BYTES => {
            Html(html).into_response()
        }
        Ok(_) => (
            StatusCode::PAYLOAD_TOO_LARGE,
            "CapabilityStatement fragment exceeds the rendering budget",
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("template render error: {error}"),
        )
            .into_response(),
    }
}

fn capability_json_fragment_endpoint(
    version: helios_fhir::FhirVersion,
) -> capability_json::FragmentEndpoint<'static> {
    capability_json::FragmentEndpoint {
        base_path: "/ui/capability-statement/json-fragment",
        version: version.as_str(),
        extra_query: "",
    }
}

fn capability_json_expand_url(version: helios_fhir::FhirVersion) -> String {
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("version", version.as_str());
    format!("/ui/capability-statement/json-expand?{}", query.finish())
}

async fn capability_json_fragment(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(query): Query<CapabilityJsonQuery>,
) -> Response {
    let version = match query.version.as_deref() {
        Some(value) => match search_params::version_from_str(value) {
            Some(version) => version,
            None => return (StatusCode::BAD_REQUEST, "Unsupported FHIR version").into_response(),
        },
        None => rv.0,
    };
    let statement = match state.conformance.metadata(version, &rt.id).await {
        Ok(statement) => statement,
        Err(error) => {
            tracing::warn!("CapabilityStatement fragment fetch failed: {error}");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "CapabilityStatement is unavailable",
            )
                .into_response();
        }
    };
    let limit = query.limit.unwrap_or(capability_json::DEFAULT_PAGE_SIZE);
    let i18n = I18n::new(locale);
    let endpoint = capability_json_fragment_endpoint(version);
    match capability_json::plan(&statement, &query.path, query.offset, limit, endpoint) {
        Ok(capability_json::View::Full(json_lines)) => bounded_capability_fragment(
            capability_json::render_full(&i18n, json_lines, query.path.is_empty()),
        ),
        Ok(capability_json::View::Outline(outline)) => {
            bounded_capability_fragment(capability_json::render_outline(&i18n, &outline))
        }
        Err(capability_json::Error::NotFound) => {
            (StatusCode::NOT_FOUND, "JSON path not found").into_response()
        }
        Err(capability_json::Error::InvalidPointer | capability_json::Error::InvalidPage) => {
            (StatusCode::BAD_REQUEST, "Invalid JSON fragment request").into_response()
        }
    }
}

async fn capability_json_expand(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(query): Query<CapabilityJsonQuery>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    let version = match query.version.as_deref() {
        Some(value) => match search_params::version_from_str(value) {
            Some(version) => version,
            None => return (StatusCode::BAD_REQUEST, "Unsupported FHIR version").into_response(),
        },
        None => rv.0,
    };
    if !headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| {
            value
                .trim()
                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
        })
    {
        return (StatusCode::BAD_REQUEST, "Invalid JSON page state").into_response();
    }
    let pages = match capability_json::parse_page_descriptors(&body) {
        Ok(pages) => pages,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid JSON page state").into_response(),
    };
    let statement = match state.conformance.metadata(version, &rt.id).await {
        Ok(statement) => statement,
        Err(error) => {
            tracing::warn!("CapabilityStatement aggregate fetch failed: {error}");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "CapabilityStatement is unavailable",
            )
                .into_response();
        }
    };
    let i18n = I18n::new(locale);
    let expanded = match capability_json::plan_expanded(
        &statement,
        &pages,
        capability_json_fragment_endpoint(version),
    ) {
        Ok(expanded) => expanded,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid JSON page state").into_response(),
    };
    match capability_json::render_expanded(&i18n, &expanded) {
        Ok(html) => Html(html).into_response(),
        Err(capability_json::ExpandedRenderError::TooLarge) => (
            StatusCode::PAYLOAD_TOO_LARGE,
            "CapabilityStatement expansion exceeds the rendering budget",
        )
            .into_response(),
        Err(capability_json::ExpandedRenderError::Template(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("template render error: {error}"),
        )
            .into_response(),
    }
}

async fn capability_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(query): Query<CapabilityQuery>,
) -> Response {
    let filter = query.filter.unwrap_or_default();
    let version = query
        .version
        .as_deref()
        .and_then(search_params::version_from_str)
        .unwrap_or(rv.0);
    let raw_requested = query.raw.as_deref() == Some("1");
    let raw_url = capability_raw_url(&filter, version);
    let expand_url = capability_json_expand_url(version);
    let i18n = I18n::new(locale);
    let fetched = state.conformance.metadata(version, &rt.id).await;
    let cards = match fetched {
        Ok(statement) => {
            let mut view = capability::build_view(&statement, version);
            if !filter.is_empty() {
                let needle = filter.to_lowercase();
                view.resources
                    .retain(|r| r.resource_type.to_lowercase().contains(&needle));
            }
            let raw_text = if raw_requested {
                serde_json::to_string_pretty(&statement).unwrap_or_default()
            } else {
                String::new()
            };
            let initial_outline = if raw_requested {
                None
            } else {
                match capability_json::plan(
                    &statement,
                    "",
                    0,
                    capability_json::DEFAULT_PAGE_SIZE,
                    capability_json_fragment_endpoint(version),
                ) {
                    Ok(capability_json::View::Outline(outline)) => Some(outline),
                    Ok(capability_json::View::Full(_)) | Err(_) => None,
                }
            };
            match RenderedCapabilityCards::render(
                &i18n,
                &view,
                version,
                &filter,
                raw_requested,
                &raw_text,
                &raw_url,
                &expand_url,
                initial_outline.as_ref(),
            ) {
                Ok(cards) => Some(cards),
                Err(error) => {
                    // The shared partials have no fallible construct, so this
                    // is unreachable in practice — but a blank page is not an
                    // acceptable way to find that out.
                    tracing::error!("CapabilityStatement card render failed: {error}");
                    None
                }
            }
        }
        Err(error) => {
            tracing::warn!("CapabilityStatement self-fetch failed: {error}");
            None
        }
    };
    render(CapabilityPage {
        status: current_status(&state, version, &rt),
        i18n,
        active_page: "capability-statement",
        cards,
    })
}

/// Compartment viewer & tester page.
///
/// `def` is resolved *before* `CmpQuery` is built, reusing the same
/// [`record_type_selection`] the type rails already write through — its
/// "explicit wins outright, invalid or not; otherwise the stored `last`
/// only if it still names a real choice" contract is exactly what
/// Compartments needs, with `defs`' codes standing in for `resource_types`.
/// That keeps [`compartments::build_view`] pure and untouched: it still
/// owns the `Patient` → first fallback chain, so an unresolvable explicit
/// code or a stale `last` both fall through to it in silence, with nothing
/// written. Compartments has no "Recently used" group (only 4-5
/// definitions, which would make one noise), so `rail` is otherwise only
/// read to resolve `def` — never rendered.
async fn compartments_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(raw): Query<CompartmentsQuery>,
    settings: rail_state::RequestSettings,
) -> Response {
    // Explicit ?version= wins; otherwise the user's stored choice (#343).
    let version = raw.version.or_else(|| Some(rv.0.as_str().to_string()));
    let fhir_version = version
        .as_deref()
        .and_then(search_params::version_from_str)
        .unwrap_or_default();
    if raw.refresh.is_some() {
        state.compartments.invalidate(&rt.id, fhir_version);
    }
    let defs = state.compartments.definitions(&rt.id, fhir_version).await;
    let codes: Vec<String> = defs.iter().map(|d| d.code.clone()).collect();

    let explicit_def = raw.def.as_deref().filter(|d| !d.is_empty());
    let rail = record_type_selection(
        &state,
        &settings.user_key,
        &rt.id,
        rail_state::RailPage::Compartments,
        settings.rail(rail_state::RailPage::Compartments, &rt.id),
        explicit_def,
        &codes,
    )
    .await;
    // Explicit always wins verbatim (build_view falls back on its own for an
    // unknown code); otherwise the stored `last` is used only when it still
    // names one of this version's definitions — a stale `last` is left
    // absent so build_view's own fallback applies, silently.
    let def = raw.def.or_else(|| {
        rail.last
            .as_deref()
            .filter(|id| !id.is_empty() && codes.iter().any(|code| code == id))
            .map(str::to_string)
    });

    let query = compartments::CmpQuery {
        version,
        def,
        tab: raw.tab,
        filter: raw.filter,
        id: raw.id,
        target: raw.target,
    };
    match compartments::build_view(&query, &defs) {
        Some(view) => render(CompartmentsPage {
            status: current_status(&state, rv.0, &rt),
            i18n: I18n::new(locale),
            active_page: "compartments",
            view,
        }),
        // No definitions means the self-fetch degraded (an outage, or auth
        // without an outbound token, #320) â€” a warning, not a 404. The failed
        // fetch is not cached, so the next request re-attempts it.
        None => render(CompartmentsDegradedPage {
            status: current_status(&state, rv.0, &rt),
            i18n: I18n::new(locale),
            active_page: "compartments",
        }),
    }
}

/// Status read path. Returns a fragment to htmx (`HX-Request`) and a full page
/// on a hard navigation, so the same URL works with and without JavaScript.
async fn status(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    HxRequest(is_htmx): HxRequest,
) -> Response {
    let status = current_status(&state, rv.0, &rt);
    let i18n = I18n::new(locale);
    if is_htmx {
        render(StatusPartial { status, i18n })
    } else {
        render(
            build_index_page(
                &state,
                locale,
                Vec::new(),
                DashboardWindow::default(),
                false,
                Vec::new(),
                None,
                rv.0,
                &rt,
            )
            .await,
        )
    }
}

/// History & Versions page shell.
async fn history_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
) -> Response {
    render(HistoryPage {
        status: current_status(&state, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "history",
    })
}

/// A diff request: the two versions to compare, and the metadata toggle.
#[derive(serde::Deserialize)]
struct DiffForm {
    /// The older version's JSON.
    from: String,
    /// The newer version's JSON.
    to: String,
    #[serde(default)]
    from_label: String,
    #[serde(default)]
    to_label: String,
    #[serde(default)]
    show_metadata: String,
    /// Set when the newer side is a deleted (tombstone) version.
    #[serde(default)]
    deleted: String,
}

/// Renders the diff between two posted versions. The versions themselves are
/// fetched by the browser from the FHIR `_history` API; computing the diff here
/// keeps it off the client (no diff library shipped) and on the one code path
/// the decision doc settled on.
async fn history_diff(locale: RequestLocale, axum::Form(form): axum::Form<DiffForm>) -> Response {
    let i18n = I18n::new(locale);
    let show_metadata = form.show_metadata == "true";
    let deleted = form.deleted == "true";

    let (from, to) = (
        serde_json::from_str::<serde_json::Value>(&form.from),
        serde_json::from_str::<serde_json::Value>(&form.to),
    );

    let (Ok(from), Ok(to)) = (from, to) else {
        return render(HistoryDiffFragment {
            i18n,
            diff: history::diff(&serde_json::Value::Null, &serde_json::Value::Null, true),
            from_label: form.from_label,
            to_label: form.to_label,
            show_metadata,
            deleted,
            parse_error: true,
        });
    };

    render(HistoryDiffFragment {
        i18n,
        diff: history::diff(&from, &to, show_metadata),
        from_label: form.from_label,
        to_label: form.to_label,
        show_metadata,
        deleted,
        parse_error: false,
    })
}

/// Assembles the landing page from the live dashboard snapshot, or from
/// placeholder data when no provider is registered â€” in which case the page
/// says so explicitly rather than presenting invented numbers as real (#555).
#[allow(clippy::too_many_arguments)]
async fn build_index_page(
    state: &WebState,
    locale: RequestLocale,
    types: Vec<String>,
    window: DashboardWindow,
    all_types: bool,
    spec_types: Vec<String>,
    focus: Option<String>,
    fhir_version: helios_fhir::FhirVersion,
    tenant: &RequestTenant,
) -> IndexPage {
    let status = current_status(state, fhir_version, tenant);
    let i18n = I18n::new(locale);
    let live =
        helios_observability::dashboard::snapshot(window, &tenant.id, &types, all_types).await;
    let sample_data = live.is_none();
    let snapshot = live.unwrap_or_else(|| sample_snapshot(window));
    let dash = build_dashboard(&snapshot, all_types, &spec_types, focus.as_deref());
    IndexPage {
        status,
        metrics: dash.metrics,
        chart: dash.chart,
        legend: dash.legend,
        picker: dash.picker,
        windows: dash.windows,
        all_types: dash.all_types,
        all_types_href: dash.all_types_href,
        sample_data,
        i18n,
        active_page: "home",
    }
}

/// Everything `build_dashboard` hands the landing page.
struct DashboardView {
    metrics: DashboardMetrics,
    chart: ChartView,
    legend: Vec<LegendEntry>,
    picker: Vec<PickerEntry>,
    windows: Vec<WindowEntry>,
    /// Whether "View all resources" is on (#599).
    all_types: bool,
    /// Link that flips the "View all resources" toggle, keeping the charted
    /// set and window.
    all_types_href: String,
}

/// A `/ui` link carrying the whole chart state: charted set, window, the
/// "View all resources" toggle (#599), and the focused series (#602). Every
/// selector emits these so changing one control keeps the others.
fn dash_href(
    types: &[String],
    window: DashboardWindow,
    all_types: bool,
    focus: Option<&str>,
) -> String {
    let mut href = format!("/ui?types={}&window={}", types.join(","), window.as_str());
    if all_types {
        href.push_str("&all=1");
    }
    // Focus survives a link only while the focused type is still charted.
    if let Some(f) = focus
        && types.iter().any(|t| t == f)
    {
        href.push_str(&format!("&focus={f}"));
    }
    href
}

/// Projects a [`DashboardSnapshot`] into the headline metrics, chart geometry,
/// and the selectors (type picker, legend, time window, "View all resources")
/// the template renders. The charted set is `snapshot.series` itself â€” the
/// provider already resolved the request to real (or, with `all_types`,
/// explicitly requested) types.
///
/// `all_types` is the "View all resources" toggle (#599). When set,
/// `spec_types` â€” every resource type of the active FHIR version, from
/// [`compartments::CompartmentCatalog::resource_type_names`] â€” is unioned into
/// the picker's option list alongside the tenant's stored types, so a type
/// with no data can still be picked (and, once picked, charts as a flat zero
/// line via the provider's relaxed selection guard). `spec_types` is ignored
/// when `all_types` is `false`. `focus` names the series a legend click has
/// asked to focus (#602); it is validated against the plotted set below.
fn build_dashboard(
    snapshot: &DashboardSnapshot,
    all_types: bool,
    spec_types: &[String],
    focus: Option<&str>,
) -> DashboardView {
    let charted: Vec<String> = snapshot
        .series
        .iter()
        .map(|s| s.resource_type.clone())
        .collect();

    // Focus only means something for a plotted series (#602); anything else
    // in the query renders the ordinary unfocused view.
    let focus = focus.filter(|f| charted.iter().any(|c| c == f));

    let mut chart = build_chart(&snapshot.series, snapshot.window, focus);

    // The plotted lines carry the same focus links as their legend entries
    // (#602): clicking a line focuses its series, clicking the focused one
    // links back. Native SVG anchors, so the no-JS contract holds.
    if chart.series.len() > 1 {
        for s in &mut chart.series {
            let target = if s.focused {
                None
            } else {
                Some(s.resource_type.as_str())
            };
            s.href = Some(dash_href(&charted, snapshot.window, all_types, target));
        }
    }

    // The picker's option list: the tenant's stored types (largest first,
    // from the provider), plus â€” with `all_types` â€” every other type of the
    // active FHIR version, at 0, alphabetically after (never duplicating a
    // type the provider already listed).
    let mut options: Vec<TypeCount> = snapshot.available.clone();
    if all_types {
        let stored: std::collections::HashSet<&str> =
            options.iter().map(|t| t.resource_type.as_str()).collect();
        let mut empties: Vec<TypeCount> = spec_types
            .iter()
            .filter(|name| !stored.contains(name.as_str()))
            .map(|name| TypeCount {
                resource_type: name.clone(),
                total: 0,
            })
            .collect();
        empties.sort_by(|a, b| a.resource_type.cmp(&b.resource_type));
        options.extend(empties);
    }

    // Each option toggles membership.
    let picker = options
        .iter()
        .map(|t| {
            let selected = charted.contains(&t.resource_type);
            let toggled: Vec<String> = if selected {
                charted
                    .iter()
                    .filter(|c| **c != t.resource_type)
                    .cloned()
                    .collect()
            } else {
                // Selecting past the cap swaps the oldest series out
                // (mirrors the provider's MAX_CHARTED_TYPES).
                let mut set: Vec<String> = charted
                    .iter()
                    .skip(charted.len().saturating_sub(CHART_MAX_SERIES - 1))
                    .cloned()
                    .collect();
                set.push(t.resource_type.clone());
                set
            };
            PickerEntry {
                resource_type: t.resource_type.clone(),
                total: grouped(t.total),
                // dash_href drops the focus itself if this toggle removes
                // the focused type.
                href: dash_href(&toggled, snapshot.window, all_types, focus),
                selected,
            }
        })
        .collect();

    // The legend names each plotted series; while more than one is plotted,
    // an entry links to focusing that series â€” or back out of the focus when
    // it already holds it (#602). Removal lives in the picker.
    let legend = snapshot
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let focused = focus == Some(s.resource_type.as_str());
            let href = (snapshot.series.len() > 1).then(|| {
                let target = if focused {
                    None
                } else {
                    Some(s.resource_type.as_str())
                };
                dash_href(&charted, snapshot.window, all_types, target)
            });
            LegendEntry {
                resource_type: s.resource_type.clone(),
                total: grouped(s.total),
                color: i % SERIES_COLORS + 1,
                href,
                focused,
            }
        })
        .collect();

    let windows = DashboardWindow::ALL
        .into_iter()
        .map(|w| WindowEntry {
            label: w.as_str().to_string(),
            href: dash_href(&charted, w, all_types, focus),
            active: w == snapshot.window,
        })
        .collect();

    let metrics = DashboardMetrics {
        resource_types: snapshot.distinct_types.to_string(),
        stored_resources: compact_count(snapshot.total_resources),
        export_jobs: snapshot.export_jobs,
        import_jobs: snapshot.import_jobs_active,
        uptime: format_uptime(helios_observability::uptime::uptime_seconds()),
        chart_total: {
            let sum: u64 = snapshot.series.iter().map(|s| s.total).sum();
            grouped(sum)
        },
    };

    DashboardView {
        metrics,
        chart,
        legend,
        picker,
        windows,
        all_types,
        all_types_href: dash_href(&charted, snapshot.window, !all_types, focus),
    }
}

// Chart plot area within the `0 0 1060 H` viewBox: the value axis occupies the
// left gutter (x < 40), the date axis the bottom 22 units.
const PLOT_LEFT: i64 = 40;
const PLOT_RIGHT: i64 = 1060;
const PLOT_TOP: i64 = 10;
/// The chart's fixed viewBox height (#555, #601).
const CHART_HEIGHT: i64 = 300;
/// Palette slots defined as `--series-N` custom properties in app.css.
const SERIES_COLORS: usize = 6;
/// Most series plotted at once â€” mirrors the provider's `MAX_CHARTED_TYPES`
/// (and the palette), so a picker link never asks for more than the server
/// will chart. The default selection is smaller (the provider's three).
const CHART_MAX_SERIES: usize = 6;

/// Computes the SVG geometry for one resource type's cumulative series. `window`
/// decides only the x-axis label format â€” a calendar date over daily buckets, a
/// UTC clock time over intraday ones. Under a legend `focus` (#602) the y axis
/// re-fits the focused series so a small type is legible next to a large one;
/// the other series stay plotted at their true values, receded, and the plot
/// clip-path cuts them where they exceed the focused scale.
fn build_chart(all: &[DashboardSeries], window: DashboardWindow, focus: Option<&str>) -> ChartView {
    let height = CHART_HEIGHT;
    let plot_bottom = height - 22;
    let plotted: Vec<&DashboardSeries> = all.iter().filter(|s| !s.points.is_empty()).collect();
    // Something is charted as soon as a series is plotted, even an all-zero
    // one (#599, "View all resources"): that renders as a real flat line at
    // 0, not the "nothing to chart" empty state.
    let has_data = !plotted.is_empty();
    let types_label = plotted
        .iter()
        .map(|s| s.resource_type.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    // The pill stays one name wide however many series are plotted.
    let pick_label = match plotted.len() {
        0 => String::new(),
        1 => plotted[0].resource_type.clone(),
        n => format!("{} +{}", plotted[0].resource_type, n - 1),
    };

    if plotted.is_empty() {
        return ChartView {
            has_data: false,
            series: Vec::new(),
            y_ticks: y_axis_ticks(0, height, plot_bottom),
            x_ticks: Vec::new(),
            height,
            tip_json: "{}".to_string(),
            table: Vec::new(),
            types_label,
            pick_label: String::new(),
        };
    }

    let width = PLOT_RIGHT - PLOT_LEFT;
    let plot_height = plot_bottom - PLOT_TOP;
    // Every series shares the window, so bucket geometry comes from the first.
    let points = &plotted[0].points;
    let n = points.len() as i64;

    // One y scale across every plotted series, so the curves are comparable â€”
    // unless a series is focused, in which case the axis fits that series.
    let peak = plotted
        .iter()
        .filter(|s| focus.is_none() || focus == Some(s.resource_type.as_str()))
        .flat_map(|s| s.points.iter().map(|p| p.cumulative))
        .max()
        .unwrap_or(0);
    let axis_max = nice_ceil(peak).max(1);

    // Map sample index -> x, cumulative value -> y (SVG y grows downward).
    let x_at = |i: i64| -> i64 {
        if n <= 1 {
            PLOT_LEFT
        } else {
            PLOT_LEFT + width * i / (n - 1)
        }
    };
    let y_at = |value: u64| -> i64 { plot_bottom - (plot_height * value as i64) / axis_max as i64 };

    let series: Vec<ChartSeriesView> = plotted
        .iter()
        .enumerate()
        .map(|(si, s)| ChartSeriesView {
            resource_type: s.resource_type.clone(),
            color: si % SERIES_COLORS + 1,
            polyline: s
                .points
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{},{}", x_at(i as i64), y_at(p.cumulative)))
                .collect::<Vec<_>>()
                .join(" "),
            emphasis: match focus {
                None => "",
                Some(f) if f == s.resource_type => " series--focused",
                Some(_) => " series--receded",
            },
            // build_dashboard fills the focus link in; geometry stays the
            // only concern here.
            href: None,
            focused: focus == Some(s.resource_type.as_str()),
        })
        .collect();

    // Up to six evenly spaced date labels along the window; the same sampled
    // buckets become the rows of the accessible table.
    let label_count = (n as usize).min(6);
    let mut x_ticks = Vec::with_capacity(label_count);
    let mut table = Vec::with_capacity(label_count);
    for j in 0..label_count as i64 {
        let idx = if label_count <= 1 {
            0
        } else {
            (n - 1) * j / (label_count as i64 - 1)
        };
        if let Some(point) = points.get(idx as usize) {
            let label = axis_time_label(point.bucket_start, window);
            x_ticks.push(AxisTick {
                label: label.clone(),
                pos: x_at(idx),
                // Date labels sit on the fixed bottom row of the viewBox.
                label_y: height - 2,
            });
            table.push(ChartTableRow {
                label,
                values: plotted
                    .iter()
                    .map(|s| {
                        s.points
                            .get(idx as usize)
                            .map(|p| grouped(p.cumulative))
                            .unwrap_or_default()
                    })
                    .collect(),
            });
        }
    }

    // Everything the tooltip script needs, precomputed: it does no scaling of
    // its own beyond mapping the pointer to the nearest bucket x.
    let tip_json = serde_json::json!({
        "labels": points
            .iter()
            .map(|p| axis_time_label(p.bucket_start, window))
            .collect::<Vec<_>>(),
        "xs": (0..n).map(&x_at).collect::<Vec<_>>(),
        "series": plotted
            .iter()
            .enumerate()
            .map(|(si, s)| serde_json::json!({
                "type": s.resource_type,
                "color": si % SERIES_COLORS + 1,
                "values": s.points.iter().map(|p| p.cumulative).collect::<Vec<_>>(),
                "ys": s.points.iter().map(|p| y_at(p.cumulative)).collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
    })
    .to_string();

    ChartView {
        has_data,
        series,
        y_ticks: y_axis_ticks(axis_max, height, plot_bottom),
        x_ticks,
        height,
        tip_json,
        table,
        types_label,
        pick_label,
    }
}

/// Five horizontal value gridlines from `axis_max` (top) down to `0` (bottom).
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
pub(crate) fn grouped(n: u64) -> String {
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
/// alphanumeric, so no percent-decoding is needed; each is validated â€” against
/// the snapshot's series, or `DashboardWindow::from_slug` â€” before use.
pub(crate) fn query_value(query: Option<&str>, key: &str) -> Option<String> {
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
    // Dense buckets for the requested window, ending now â€” the same shape a real
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
    let available = series
        .iter()
        .map(|s| helios_observability::dashboard::TypeCount {
            resource_type: s.resource_type.clone(),
            total: s.total,
        })
        .collect();

    DashboardSnapshot {
        fhir_version: "R4".to_string(),
        total_resources,
        distinct_types: 142,
        window,
        series,
        available,
        export_jobs: None,
        import_jobs_active: None,
    }
}

/// Floors `ts` to the start of the epoch-aligned bucket containing it. Mirrors
/// `helios_persistence::core::bucket_floor`, which this crate cannot call â€” it
/// deliberately does not depend on persistence â€” and is used only to shape the
/// placeholder series.
fn bucket_floor_utc(ts: DateTime<Utc>, bucket_seconds: i64) -> DateTime<Utc> {
    if bucket_seconds <= 0 {
        return ts;
    }
    let floored = ts.timestamp().div_euclid(bucket_seconds) * bucket_seconds;
    DateTime::from_timestamp(floored, 0).unwrap_or(ts)
}

pub(crate) fn current_status(
    state: &WebState,
    fhir_version: helios_fhir::FhirVersion,
    tenant: &RequestTenant,
) -> Status {
    Status {
        version: state.version,
        checked_at: unix_timestamp_seconds(),
        fhir_version,
        tenant_id: tenant.id.clone(),
        tenant_display: tenant.display.clone(),
        show_tenant_picker: tenant.multi,
        terminology: TerminologyNavigation::from_config(state.terminology.as_deref()),
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

/// The generic "not found" page (#835): a `404` inside the full shell for a
/// route naming something this request cannot see — an unknown id, or one
/// belonging to another user/tenant, deliberately indistinguishable from
/// each other. Reusable by any future route needing the same shape: nothing
/// here is specific to the one caller that exists today
/// (`sql_export::detail_page`) beyond its own `back_href`/`back_label`.
/// Never reveals which of "unknown" or "not yours" applies, nor any other
/// internal detail.
#[derive(Template)]
#[template(path = "pages/not-found.html")]
struct NotFoundPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    back_href: String,
    back_label: String,
}

/// Renders [`NotFoundPage`] with a `404` status — the shared tail for any
/// route that must answer "not found", full shell included.
pub(crate) fn render_not_found(
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    back_href: impl Into<String>,
    back_label: impl Into<String>,
) -> Response {
    let page = NotFoundPage {
        status,
        i18n,
        active_page,
        back_href: back_href.into(),
        back_label: back_label.into(),
    };
    (StatusCode::NOT_FOUND, render(page)).into_response()
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

    #[derive(Template)]
    #[template(source = "{{ value }}", ext = "html")]
    struct BoundedFragmentTestTemplate<'a> {
        value: &'a str,
    }

    struct FailingDisplay;

    impl std::fmt::Display for FailingDisplay {
        fn fmt(&self, _formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            Err(std::fmt::Error)
        }
    }

    #[derive(Template)]
    #[template(source = "{{ value }}", ext = "html")]
    struct FailingFragmentTestTemplate {
        value: FailingDisplay,
    }

    fn i18n(tag: &str) -> I18n {
        I18n::from_tag(tag).expect("supported locale")
    }

    #[test]
    fn capability_fragments_enforce_the_rendering_budget() {
        let oversized = "x".repeat(capability_json::MAX_FRAGMENT_HTML_BYTES + 1);
        let rendered = BoundedFragmentTestTemplate { value: &oversized }.render();
        let response = bounded_capability_fragment(rendered);
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn capability_fragments_report_template_errors() {
        let rendered = FailingFragmentTestTemplate {
            value: FailingDisplay,
        }
        .render();
        let response = bounded_capability_fragment(rendered);
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Builds an `IndexPage` from the sample snapshot for template-rendering tests.
    fn sample_index_page(version: &'static str, checked_at: u64, i18n: I18n) -> IndexPage {
        let dash = build_dashboard(
            &sample_snapshot(DashboardWindow::default()),
            false,
            &[],
            None,
        );
        IndexPage {
            status: Status {
                version,
                checked_at,
                fhir_version: helios_fhir::FhirVersion::R4,
                tenant_id: "default".to_string(),
                tenant_display: None,
                show_tenant_picker: true,
                terminology: TerminologyNavigation::Unconfigured,
            },
            metrics: dash.metrics,
            chart: dash.chart,
            legend: dash.legend,
            picker: dash.picker,
            windows: dash.windows,
            all_types: dash.all_types,
            all_types_href: dash.all_types_href,
            sample_data: true,
            i18n,
            active_page: "home",
        }
    }

    /// #602: a legend focus keeps every series plotted, re-fits the y axis to
    /// the focused one, and flips the focused entry's link into the way back.
    #[test]
    fn legend_focus_rescales_recedes_and_links_back() {
        let snapshot = sample_snapshot(DashboardWindow::default());
        let unfocused = build_dashboard(&snapshot, false, &[], None);
        // Patient is the smallest sample series, so focusing it must lower
        // the axis ceiling relative to the shared (Observation-driven) scale.
        let dash = build_dashboard(&snapshot, false, &[], Some("Patient"));

        assert_eq!(dash.chart.series.len(), unfocused.chart.series.len());
        for s in &dash.chart.series {
            if s.resource_type == "Patient" {
                assert_eq!(s.emphasis, " series--focused");
            } else {
                assert_eq!(s.emphasis, " series--receded");
            }
        }
        let top = |d: &DashboardView| d.chart.y_ticks.first().map(|t| t.label.clone()).unwrap();
        assert_ne!(top(&dash), top(&unfocused), "axis re-fits the focus");

        let entry = |d: &DashboardView, t: &str| {
            d.legend
                .iter()
                .find(|l| l.resource_type == t)
                .expect("legend entry")
                .href
                .clone()
                .expect("legend link")
        };
        // The focused entry links back out; the others move the focus.
        assert!(!entry(&dash, "Patient").contains("focus="));
        assert!(entry(&dash, "Observation").contains("focus=Observation"));
        // Nothing was removed: every legend link keeps the full charted set.
        assert!(entry(&dash, "Observation").contains("Patient"));

        // The plotted lines carry the same links as their legend entries:
        // the focused one links back out, the others move the focus.
        let line = |d: &DashboardView, t: &str| {
            d.chart
                .series
                .iter()
                .find(|s| s.resource_type == t)
                .expect("series")
                .href
                .clone()
                .expect("line link")
        };
        assert!(!line(&dash, "Patient").contains("focus="));
        assert!(line(&dash, "Observation").contains("focus=Observation"));

        // A focus that names an uncharted type renders the ordinary view.
        let bogus = build_dashboard(&snapshot, false, &[], Some("Nope"));
        assert!(bogus.chart.series.iter().all(|s| s.emphasis.is_empty()));

        // The focus link keeps "View all resources" on too (#599 + #602
        // interaction): a focus click while `?all=1` is active must not drop
        // the flag.
        let with_all_types = build_dashboard(&snapshot, true, &[], Some("Patient"));
        let all_types_entry = with_all_types
            .legend
            .iter()
            .find(|l| l.resource_type == "Observation")
            .expect("legend entry")
            .href
            .clone()
            .expect("legend link");
        assert!(all_types_entry.contains("focus=Observation"));
        assert!(all_types_entry.contains("all=1"));
    }

    #[test]
    fn format_uptime_picks_the_two_leading_units() {
        assert_eq!(format_uptime(0.0), None);
        assert_eq!(format_uptime(-5.0), None);
        assert_eq!(format_uptime(18.4), Some("18s".to_string()));
        assert_eq!(format_uptime(42.0 * 60.0 + 30.0), Some("42m".to_string()));
        assert_eq!(
            format_uptime(5.0 * 3_600.0 + 12.0 * 60.0),
            Some("5h 12m".to_string())
        );
        assert_eq!(
            format_uptime(3.0 * 86_400.0 + 4.0 * 3_600.0 + 59.0 * 60.0),
            Some("3d 4h".to_string())
        );
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
        assert!(html.contains(r#"<link rel="icon" type="image/png" href="/ui/assets/logo.png">"#));
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
    fn sidebar_groups_compartments_under_server_and_labels_tools() {
        for (locale, tools_label) in [("en", "Tools"), ("es", "Herramientas"), ("de", "Werkzeuge")]
        {
            let html = sample_index_page("1.2.3", 42, i18n(locale))
                .render()
                .expect("index renders");

            assert!(html.contains(tools_label));
        }

        let html = sample_index_page("1.2.3", 42, i18n("en"))
            .render()
            .expect("index renders");
        let server = html.find(">Server</div>").expect("Server section");
        let compartments = html
            .find(r#"href="/ui/compartments"#)
            .expect("Compartments link");
        let tools = html.find(">Tools</div>").expect("Tools section");

        assert!(server < compartments && compartments < tools);
        assert!(!html.contains("Admin / Ops"));
    }

    #[test]
    fn status_partial_is_fragment_not_full_page() {
        let html = StatusPartial {
            status: Status {
                version: "1.2.3",
                checked_at: 42,
                fhir_version: helios_fhir::FhirVersion::R4,
                tenant_id: "default".to_string(),
                tenant_display: None,
                show_tenant_picker: true,
                terminology: TerminologyNavigation::Unconfigured,
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
        assert!(Assets::get("fonts/figtree-latin.woff2").is_some());
        assert!(Assets::get("fonts/figtree-latin-ext.woff2").is_some());
        assert!(Assets::get("logo.png").is_some());
    }

    /// #753: the CodeMirror 6 + lezer-fhirpath vendoring ritual's
    /// one committed output (`crates/ui/vendor/codemirror/README.md`) is
    /// embedded exactly like any other subfolder asset — `assets/fonts/` is
    /// the existing precedent for rust-embed walking into `assets/vendor/` —
    /// and opens with the license banner rollup.config.js generates, wrapping
    /// the `window.HfsCodeMirror` global `vd-editor.js` and `sql-editor.js`
    /// build against.
    ///
    /// #838 adds `@codemirror/lang-sql` to the same ritual: this asserts
    /// the banner lists it, so a bundle regenerated without SQL support
    /// does not pass silently.
    #[test]
    fn codemirror_vendor_bundle_is_embedded() {
        let file = Assets::get("vendor/codemirror.bundle.js").expect("CodeMirror bundle embedded");
        let source = std::str::from_utf8(&file.data).expect("bundle is UTF-8");
        assert!(
            source.starts_with("/*!"),
            "bundle must open with the license banner"
        );
        assert!(
            source.contains("HfsCodeMirror"),
            "bundle must define the window.HfsCodeMirror global"
        );
        assert!(
            source.contains("@codemirror/lang-sql"),
            "bundle banner must list @codemirror/lang-sql (#838)"
        );
    }

    /// #821: the vendoring ritual's raw-size budget (`crates/ui/vendor/codemirror/README.md`
    /// § "Measured sizes") — a regeneration that pulls in an unexpectedly heavy
    /// package should fail a test, not just a number nobody re-checks in the README.
    #[test]
    fn codemirror_vendor_bundle_stays_within_its_raw_size_budget() {
        let file = Assets::get("vendor/codemirror.bundle.js").expect("CodeMirror bundle embedded");
        const RAW_SIZE_BUDGET_BYTES: usize = 500_000;
        assert!(
            file.data.len() <= RAW_SIZE_BUDGET_BYTES,
            "codemirror.bundle.js is {} bytes, over the {}-byte raw budget",
            file.data.len(),
            RAW_SIZE_BUDGET_BYTES
        );
    }

    /// #821: `lezer-fhirpath`'s license is MIT, declared only in its published
    /// README (no `license` field, no `LICENSE` file — see the vendor README's
    /// citation) — the banner records that via `rollup.config.js`'s license
    /// override map and must no longer show the old "not declared" placeholder.
    #[test]
    fn codemirror_vendor_bundle_banner_documents_lezer_fhirpath_license() {
        let file = Assets::get("vendor/codemirror.bundle.js").expect("CodeMirror bundle embedded");
        let source = std::str::from_utf8(&file.data).expect("bundle is UTF-8");
        assert!(
            source.contains("lezer-fhirpath") && source.contains("MIT"),
            "banner must document lezer-fhirpath's MIT license"
        );
        assert!(
            !source.contains("not declared in package metadata"),
            "banner must no longer show the unresolved-license placeholder"
        );
    }

    /// #821: the vendoring ritual's README documents `eval`/`new Function`/
    /// `document.write` as a hard constraint on this bundle, checked by hand
    /// on every regeneration — this makes that check automatic.
    #[test]
    fn codemirror_vendor_bundle_has_no_eval_or_dynamic_code() {
        let file = Assets::get("vendor/codemirror.bundle.js").expect("CodeMirror bundle embedded");
        let source = std::str::from_utf8(&file.data).expect("bundle is UTF-8");
        assert!(!source.contains("eval("), "bundle must not call eval(...)");
        assert!(
            !source.contains("new Function("),
            "bundle must not construct dynamic functions"
        );
        assert!(
            !source.contains("document.write("),
            "bundle must not call document.write(...)"
        );
    }

    /// #753: vd-editor.js — the hand-written mount script that
    /// progressively enhances the ViewDefinition textarea with the vendored
    /// bundle — is embedded like every other page script.
    #[test]
    fn vd_editor_script_is_embedded() {
        assert!(Assets::get("vd-editor.js").is_some());
    }

    /// #838: `code-editor.js`, the mount helper `vd-editor.js` was
    /// generalized out of (and every future CodeMirror editor in this crate
    /// builds on), is embedded like every other page script.
    #[test]
    fn code_editor_helper_script_is_embedded() {
        assert!(Assets::get("code-editor.js").is_some());
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

    #[test]
    fn queries_page_renders_shell_and_marks_nav_current() {
        let resource_types = vec!["Patient".to_string(), "Observation".to_string()];
        let rail_entries = build_rail_entries("/ui/queries", &resource_types, None, None);
        let html = QueriesPage {
            status: Status {
                version: "1.2.3",
                checked_at: 42,
                fhir_version: helios_fhir::FhirVersion::R4,
                tenant_id: "default".to_string(),
                tenant_display: None,
                show_tenant_picker: true,
                terminology: TerminologyNavigation::Unconfigured,
            },
            i18n: i18n("en"),
            active_page: "queries",
            show_save: true,
            resource_types,
            selected_type: String::new(),
            rail_entries,
            recent_entries: Vec::new(),
            rail_page: rail_state::RailPage::Queries.key(),
            max_recent: rail_state::MAX_RECENT,
            builder_url: None,
        }
        .render()
        .expect("queries page renders");

        assert!(html.contains(r#"id="saved-query-form""#));
        assert!(html.contains(r#"id="saved-queries""#));
        assert!(html.contains("/ui/assets/fhir-search-value.js"));
        assert!(html.contains("/ui/assets/saved-queries.js"));
        assert!(
            html.find("/ui/assets/fhir-search-value.js") < html.find("/ui/assets/saved-queries.js"),
            "the FHIR search-value codec must load before its consumer"
        );
        // Search Builder: the featured GET URL input, both submit intents,
        // and the Recent dropdown shell the script hydrates.
        assert!(html.contains(r#"name="url""#));
        assert!(html.contains(r#"data-intent="run""#));
        assert!(html.contains(r#"data-intent="save""#));
        assert!(html.contains(r#"id="recent-searches""#));
        // The Recent panel closes from an explicit X as well as outside
        // click / Esc (addbox.js covers details.menu too).
        assert!(html.contains("data-addbox-close"));
        // Resource picker rail (#541): a real link per type, server-marked
        // current; no count without a dashboard provider.
        for resource_type in ["Patient", "Observation"] {
            let attributes =
                format!(r#"data-type="{resource_type}" data-full-name="{resource_type}""#);
            let href = format!(r#"href="/ui/queries?type={resource_type}""#);
            assert!(
                html.contains(&attributes),
                "{resource_type} rail attributes"
            );
            assert!(html.contains(&href), "{resource_type} rail link");
        }
        assert!(!html.contains(r#"class="count""#));
        // Saved Queries has no nav entry any more (#282 folded search / editor
        // / history / saved-queries into Resources); the route still renders.
        assert!(!html.contains(r#"href="/ui/queries" aria-current="page""#));
        // The delete-confirm string reaches the script with its {name} slot.
        assert!(html.contains("{name}"));
    }

    #[test]
    fn queries_page_renders_in_the_negotiated_locale() {
        let resource_types = vec!["Patient".to_string()];
        let rail_entries = build_rail_entries("/ui/queries", &resource_types, None, None);
        let html = QueriesPage {
            status: Status {
                version: "1.2.3",
                checked_at: 42,
                fhir_version: helios_fhir::FhirVersion::R4,
                tenant_id: "default".to_string(),
                tenant_display: None,
                show_tenant_picker: true,
                terminology: TerminologyNavigation::Unconfigured,
            },
            i18n: i18n("es"),
            active_page: "queries",
            show_save: true,
            resource_types,
            selected_type: String::new(),
            rail_entries,
            recent_entries: Vec::new(),
            rail_page: rail_state::RailPage::Queries.key(),
            max_recent: rail_state::MAX_RECENT,
            builder_url: None,
        }
        .render()
        .expect("queries page renders");

        assert!(html.contains("Consultas guardadas"));
    }

    /// The saved-queries script owns a structural read-modify-write against
    /// the shared settings document, so â€” unlike theme.js â€” it must use the
    /// conditional-request cycle: capture the ETag, send If-Match, and absorb
    /// a 412 by re-reading. Guards the wiring; the endpoint semantics are
    /// covered in helios-rest's `user_settings` tests.
    #[test]
    fn saved_queries_script_is_wired_to_user_settings() {
        let file = Assets::get("saved-queries.js").expect("saved-queries.js embedded");
        let source = std::str::from_utf8(&file.data).expect("saved-queries.js is UTF-8");
        assert!(source.contains("/_user/settings"));
        assert!(source.contains("savedQueries"));
        assert!(source.contains("If-Match"));
        assert!(
            source.contains("412"),
            "recovers from optimistic-lock races"
        );
        assert!(source.contains("lastAccessedAt"));
        // Every run is recorded to the roaming recent-searches list.
        assert!(source.contains("recentSearches"));
        // Results render in-page from the FHIR API itself, and the builder's
        // parameter suggestions come from the server-rendered datalist.
        assert!(source.contains("application/fhir+json"));
        assert!(source.contains("/ui/queries/params"));
    }

    #[test]
    fn fhir_search_value_codec_is_embedded() {
        let file = Assets::get("fhir-search-value.js").expect("FHIR search-value codec embedded");
        let source = std::str::from_utf8(&file.data).expect("FHIR search-value codec is UTF-8");
        assert!(source.contains("parseAlternatives"));
        assert!(source.contains("serializeAlternative"));
    }

    /// The builder's datalist fragment is fed by the SearchParameter
    /// registry and scoped to the requested resource type.
    #[test]
    fn param_options_partial_renders_datalist() {
        let html = ParamOptionsPartial {
            params: vec![
                ParamOption {
                    code: "birthdate".into(),
                    type_label: "date".into(),
                    targets: String::new(),
                },
                ParamOption {
                    code: "general-practitioner".into(),
                    type_label: "reference".into(),
                    targets: "Organization,Practitioner".into(),
                },
            ],
        }
        .render()
        .expect("partial renders");

        assert!(html.contains(r#"<datalist id="param-options">"#));
        assert!(html.contains(r#"value="birthdate""#));
        assert!(html.contains(r#"data-type="date""#));
        assert!(html.contains(r#"data-targets="Organization,Practitioner""#));
        assert!(!html.contains("<html"), "fragment, not a page");
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
        let dash = build_dashboard(
            &sample_snapshot(DashboardWindow::default()),
            false,
            &[],
            None,
        );

        // Every series the snapshot carries is plotted, on one shared y scale.
        assert!(dash.chart.has_data);
        assert_eq!(dash.chart.series.len(), 4);
        assert!(dash.chart.series.iter().all(|s| !s.polyline.is_empty()));
        assert_eq!(dash.chart.y_ticks.len(), 5);
        assert!(dash.chart.types_label.contains("Observation"));

        // The legend names each plotted series; while several are plotted each
        // entry is a focus link that keeps the whole charted set and the
        // window (#602) â€” removal belongs to the picker.
        assert_eq!(dash.legend.len(), 4);
        let observation = dash
            .legend
            .iter()
            .find(|e| e.resource_type == "Observation")
            .expect("Observation in legend");
        let href = observation
            .href
            .as_deref()
            .expect("focusable while several are plotted");
        assert!(href.contains("focus=Observation"));
        assert!(href.contains("Patient"));
        assert!(href.contains("window=30d"));

        // The picker offers every stored type, ticking the plotted ones; a
        // selected option's link drops exactly itself.
        assert_eq!(dash.picker.len(), 4);
        assert!(dash.picker.iter().all(|p| p.selected));
        let patient = dash
            .picker
            .iter()
            .find(|p| p.resource_type == "Patient")
            .expect("Patient offered");
        assert!(!patient.href.contains("types=Patient"));
        assert!(patient.href.contains("Observation"));

        assert_eq!(dash.metrics.resource_types, "142");
    }

    /// The window selector offers every window, marks the snapshot's own as
    /// active, and carries the charted type across a window switch.
    #[test]
    fn window_selector_marks_the_active_window_and_keeps_the_charted_type() {
        let windows = build_dashboard(
            &sample_snapshot(DashboardWindow::LastHour),
            false,
            &[],
            None,
        )
        .windows;

        assert_eq!(windows.len(), DashboardWindow::ALL.len());
        assert_eq!(windows.iter().filter(|w| w.active).count(), 1);
        let active = windows.iter().find(|w| w.active).expect("an active window");
        assert_eq!(active.label, "1h");
        assert!(
            windows
                .iter()
                .all(|w| w.href.starts_with("/ui?types=") && w.href.contains("Patient")),
            "every window link keeps the charted set"
        );
        assert!(active.href.contains("Patient"));
        assert!(active.href.ends_with("window=1h"));
    }

    /// Intraday windows label the x-axis with clock times; the 30-day window
    /// keeps calendar dates. Same series, different axis vocabulary.
    #[test]
    fn axis_labels_follow_the_window_resolution() {
        let hour_chart = build_dashboard(
            &sample_snapshot(DashboardWindow::LastHour),
            false,
            &[],
            None,
        )
        .chart;
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

        let month_chart = build_dashboard(
            &sample_snapshot(DashboardWindow::LastMonth),
            false,
            &[],
            None,
        )
        .chart;
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
            let chart = build_dashboard(&snapshot, false, &[], None).chart;
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
    fn empty_snapshot_renders_the_empty_state() {
        let empty = DashboardSnapshot {
            fhir_version: "R4".to_string(),
            total_resources: 0,
            distinct_types: 0,
            window: DashboardWindow::default(),
            series: Vec::new(),
            available: Vec::new(),
            export_jobs: None,
            import_jobs_active: None,
        };
        let dash = build_dashboard(&empty, false, &[], None);
        assert!(!dash.chart.has_data);
        assert!(dash.chart.series.is_empty());
        assert!(dash.legend.is_empty());
        assert!(dash.picker.is_empty());
        assert_eq!(dash.metrics.chart_total, "0");
        // The window selector still renders, so an empty server is not a dead end.
        assert_eq!(dash.windows.len(), DashboardWindow::ALL.len());
    }

    /// `dash_href` only appends `&all=1` when the toggle is on â€” the common,
    /// flag-off links must stay exactly as they were before #599.
    #[test]
    fn dash_href_carries_the_all_types_flag_only_when_on() {
        let types = vec!["Patient".to_string()];
        let off = dash_href(&types, DashboardWindow::LastDay, false, None);
        let on = dash_href(&types, DashboardWindow::LastDay, true, None);
        assert_eq!(off, "/ui?types=Patient&window=24h");
        assert_eq!(on, "/ui?types=Patient&window=24h&all=1");
    }

    /// With "View all resources" (#599), the picker offers the tenant's stored
    /// types (as today, largest first) plus every other type of the version at
    /// 0, alphabetically after â€” never duplicating a type already stored, and
    /// every option link carries `all=1` so the toggle survives a click.
    #[test]
    fn view_all_unions_spec_types_after_stored_ones_with_data_first() {
        let snapshot = DashboardSnapshot {
            fhir_version: "R4".to_string(),
            total_resources: 5,
            distinct_types: 1,
            window: DashboardWindow::default(),
            series: vec![DashboardSeries {
                resource_type: "Patient".to_string(),
                total: 5,
                points: vec![point_at(1_752_451_200, 5, 5)],
            }],
            available: vec![helios_observability::dashboard::TypeCount {
                resource_type: "Patient".to_string(),
                total: 5,
            }],
            export_jobs: None,
            import_jobs_active: None,
        };
        let spec_types = vec![
            "Observation".to_string(),
            "Aardvark".to_string(),
            "Patient".to_string(), // already stored â€” must not be duplicated
        ];

        let dash = build_dashboard(&snapshot, true, &spec_types, None);

        let names: Vec<&str> = dash
            .picker
            .iter()
            .map(|p| p.resource_type.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["Patient", "Aardvark", "Observation"],
            "stored types first, then the rest of the version alphabetically"
        );
        let observation = dash
            .picker
            .iter()
            .find(|p| p.resource_type == "Observation")
            .expect("Observation offered even though unstored");
        assert_eq!(observation.total, "0");
        assert!(!observation.selected);
        assert!(
            observation.href.contains("all=1"),
            "toggling an option keeps the flag on: {}",
            observation.href
        );

        // Without the flag, the union never happens â€” today's behavior.
        let off = build_dashboard(&snapshot, false, &spec_types, None);
        assert_eq!(off.picker.len(), 1, "only the stored type is offered");
        assert!(!off.picker[0].href.contains("all=1"));
    }

    /// The toggle link itself flips `all_types` while keeping the charted set
    /// and window untouched.
    #[test]
    fn all_types_toggle_link_flips_the_flag_and_keeps_the_rest_of_the_state() {
        let snapshot = DashboardSnapshot {
            window: DashboardWindow::LastDay,
            series: vec![DashboardSeries {
                resource_type: "Patient".to_string(),
                total: 5,
                points: vec![point_at(1_752_451_200, 5, 5)],
            }],
            ..DashboardSnapshot::default()
        };

        let off = build_dashboard(&snapshot, false, &[], None);
        assert!(!off.all_types);
        assert!(off.all_types_href.contains("Patient"));
        assert!(off.all_types_href.contains("window=24h"));
        assert!(off.all_types_href.ends_with("all=1"));

        let on = build_dashboard(&snapshot, true, &[], None);
        assert!(on.all_types);
        assert!(!on.all_types_href.contains("all=1"), "toggles back off");
    }

    /// A charted type with no data (#599: a "View all resources" selection) is
    /// a real flat line at 0, not the chart's empty state.
    #[test]
    fn charting_a_zero_total_series_renders_a_flat_line_not_the_empty_state() {
        let empty_type = DashboardSeries {
            resource_type: "Observation".to_string(),
            total: 0,
            points: vec![
                point_at(1_752_451_200, 0, 0),
                point_at(1_752_454_800, 0, 0),
                point_at(1_752_458_400, 0, 0),
            ],
        };
        let chart = build_chart(
            std::slice::from_ref(&empty_type),
            DashboardWindow::LastHour,
            None,
        );

        assert!(chart.has_data, "a plotted series, even all-zero, has data");
        assert_eq!(chart.series.len(), 1);
        assert!(
            !chart.series[0].polyline.is_empty(),
            "the flat line is still drawn, not omitted"
        );
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
        let chart = build_chart(
            std::slice::from_ref(&series),
            DashboardWindow::default(),
            None,
        );

        assert!(chart.has_data);
        // A lone point produces a single "x,y" pair pinned to the left axis.
        let polyline = &chart.series[0].polyline;
        assert!(!polyline.contains(' '));
        assert!(polyline.starts_with("40,"));
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
        let chart = build_chart(
            std::slice::from_ref(&series),
            DashboardWindow::LastHour,
            None,
        );

        assert!(chart.has_data);
        // Every plotted y sits inside the plot area (10..=278 in the viewBox).
        for pair in chart.series[0].polyline.split(' ') {
            let y: i64 = pair
                .split_once(',')
                .expect("x,y pair")
                .1
                .parse()
                .expect("integer y");
            assert!(
                (PLOT_TOP..=CHART_HEIGHT - 22).contains(&y),
                "y {y} escaped the plot"
            );
        }
    }
}
