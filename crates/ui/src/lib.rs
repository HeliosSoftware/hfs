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
mod capability_json;
mod compartments;
mod conformance;
mod editor;
mod history;
mod i18n;
mod json_view;
mod search_params;
mod sql_export;
mod sql_libraries;
mod sql_views;
mod subscriptions;
mod tenants;

#[doc(hidden)]
pub use conformance::{
    Caller, ConformanceSource, RecordedExportCall, SqlExportStatus, StaticConformanceSource,
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
    http::{HeaderMap, StatusCode},
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

/// Whether the mounted FHIR API can search Patient resources by name.
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

    let mut version = state.fhir_version;
    let mut tenant = RequestTenant {
        id: state.default_tenant.clone(),
        display: None,
        multi: false,
    };
    let document = match &state.settings {
        Some(store) => {
            let user = settings_user_key(request.extensions().get::<helios_auth::Principal>());
            match store.get_settings(&user).await {
                Ok(stored) => stored.map(|s| s.document),
                Err(_) => None,
            }
        }
        None => None,
    };
    if let Some(document) = &document {
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

/// A small CapabilityStatement (or subtree) rendered by the unchanged shared
/// JSON highlighter.
#[derive(Template)]
#[template(path = "partials/capability-json-full.html")]
struct CapabilityJsonFullFragment {
    i18n: I18n,
    json_lines: Vec<json_view::JsonLine>,
    json_view_id: String,
    json_view_paths: bool,
}

/// One bounded level of a large CapabilityStatement.
#[derive(Template)]
#[template(path = "partials/capability-json-outline.html")]
struct CapabilityJsonOutlineFragment {
    i18n: I18n,
    outline: capability_json::Outline,
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
        // Batch/Transaction workspace (#476): upload â†’ preflight â†’ response.
        .route("/ui/batch", get(batch_page))
        // SQL on FHIR workspaces (#649).
        .route(
            "/ui/sql/view-definitions",
            get(sql_view_definitions_page).post(sql_view_definitions_save),
        )
        // #753 ticket 03 (evaluation POC): the editor's async server lint.
        .route(
            "/ui/sql/view-definitions/lint",
            axum::routing::post(sql_view_definitions_lint),
        )
        .route(
            "/ui/sql/queries",
            get(sql_queries_page).post(sql_queries_save),
        )
        .route("/ui/sql/views", get(sql_views_page).post(sql_views_save))
        // Active SQL Exports (#833): list-first, mirroring Bulk Export's
        // `/ui/bulk-export` + `/ui/bulk-export/new` shape.
        .route(
            "/ui/sql/export",
            get(sql_export::list).post(sql_export::start),
        )
        .route("/ui/sql/export/new", get(sql_export::new_page))
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
        .route("/ui/sql/files", get(sql_files_page))
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
        .route(
            "/ui/bulk-export/patient-options",
            axum::routing::post(bulk_export::patient_options),
        )
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
        .fallback_service(fhir_app)
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

/// Search page: natural language and the visual builder over one editable query.
async fn search(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(query): Query<SearchQuery>,
) -> Response {
    let resource_types = state.compartments.resource_type_names(&rt.id, rv.0).await;
    let live =
        helios_observability::dashboard::snapshot(DashboardWindow::default(), &rt.id, &[], false)
            .await;
    let rail_entries = build_rail_entries(
        "/ui/search",
        &resource_types,
        live.as_ref().map(|s| s.available.as_slice()),
        query.resource_type.as_deref(),
    );
    render(SearchPage {
        status: current_status(&state, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "search",
        nl: (*state.nl).clone(),
        docs_url: NL_SEARCH_DOCS,
        resource_types,
        show_save: false,
        rail_entries,
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
) -> Response {
    let resource_types = state.compartments.resource_type_names(&rt.id, rv.0).await;
    let live =
        helios_observability::dashboard::snapshot(DashboardWindow::default(), &rt.id, &[], false)
            .await;
    let rail_entries = build_rail_entries(
        "/ui/queries",
        &resource_types,
        live.as_ref().map(|s| s.available.as_slice()),
        query.resource_type.as_deref(),
    );
    render(QueriesPage {
        status: current_status(&state, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "queries",
        resource_types,
        show_save: true,
        rail_entries,
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
) -> Response {
    let resource_types = state.compartments.resource_type_names(&rt.id, rv.0).await;
    // `url` is the builder's actual query, so it wins over the convenience
    // `type` bookmark. Parse it here too: Create must be safe before the
    // deferred client script hydrates the page.
    let (selected_type, builder_url) = resources_query_context(&query);
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

fn resources_query_context(query: &ResourcesQuery) -> (String, Option<String>) {
    if let Some(raw_url) = &query.url {
        let visible = raw_url
            .trim()
            .strip_prefix("GET ")
            .or_else(|| raw_url.trim().strip_prefix("get "))
            .unwrap_or(raw_url.trim())
            .to_string();
        return (
            resource_type_from_search_url(&visible).unwrap_or_default(),
            Some(visible),
        );
    }

    let selected = query
        .resource_type
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or("Patient")
        .to_string();
    (selected.clone(), Some(format!("/{selected}")))
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

/// SearchParameter viewer page.
async fn search_parameters(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(raw): Query<SearchParametersQuery>,
) -> Response {
    let query = search_params::SpQuery {
        // Explicit ?version= wins; otherwise the user's stored choice (#343).
        version: raw.version.or_else(|| Some(rv.0.as_str().to_string())),
        base: raw.base.filter(|b| !b.is_empty()),
        ptype: raw.ptype.filter(|t| !t.is_empty()),
        source: raw.source.filter(|s| !s.is_empty()),
        q: raw.q,
        page: raw.page.unwrap_or(1),
        sel: raw.sel.filter(|s| !s.is_empty()),
    };
    if raw.refresh.is_some() {
        state.sp_catalog.invalidate(&rt.id, query.fhir_version());
    }
    let snapshot = state
        .sp_catalog
        .snapshot(&rt.id, query.fhir_version())
        .await;
    render(SearchParametersPage {
        status: current_status(&state, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "search-parameters",
        view: search_params::build_view(&snapshot, &query),
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

/// The View Definitions workspace (#649, Figma `420-2`): a filter rail of the
/// tenant's stored ViewDefinitions, the selected one as editable JSON, and a
/// `$sql-run` preview of its output. Save and Duplicate are plain form posts
/// (they work without JavaScript); Delete rides `conformance-crud.js` like
/// the other conformance viewers.
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
    results: Option<sql_views::RunTable>,
    run_error: Option<String>,
    save_error: Option<String>,
    saved: bool,
}

struct SelectedVd {
    id: String,
    name: String,
    json: String,
}

#[derive(Deserialize, Default)]
struct SqlVdQuery {
    vd: Option<String>,
    filter: Option<String>,
    run: Option<String>,
    saved: Option<String>,
    /// 1-based (#741). Kept as raw text rather than `Option<usize>` so a
    /// non-numeric value fails the `parse` below instead of the whole
    /// extractor — an invalid page falls back to 1, it never 400s.
    page: Option<String>,
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

    let is_new = query.vd.as_deref() == Some("new");
    let (selected, selected_value) = if is_new {
        (
            Some(SelectedVd {
                id: String::new(),
                name: String::new(),
                json: sql_views::starter_view_definition(),
            }),
            None,
        )
    } else {
        // ?vd=<id>, defaulting to the rail's first visible entry. When the id
        // is on the visible page the resource is taken from the search
        // above — no second round trip; otherwise (a different page, or a
        // filter that no longer matches it) it's read directly by id, so
        // selection stays independent of what the rail currently shows
        // (#741).
        let wanted = query
            .vd
            .clone()
            .or_else(|| summaries.first().map(|e| e.id.clone()));
        let on_page = wanted.as_deref().and_then(|id| {
            page_resources
                .iter()
                .position(|vd| vd.get("id").and_then(serde_json::Value::as_str) == Some(id))
        });
        let resource = match on_page {
            Some(i) => Some(page_resources.swap_remove(i)),
            None => match &wanted {
                Some(id) => {
                    match state
                        .conformance
                        .read_resource("ViewDefinition", id, rv.0, &rt.id)
                        .await
                    {
                        Ok(vd) => Some(vd),
                        Err(error) => {
                            // A stale or mistyped id is ordinary navigation,
                            // not an operator concern — the page just falls
                            // back to its no-selection state below.
                            tracing::debug!("ViewDefinition read failed for vd={id}: {error}");
                            None
                        }
                    }
                }
                None => None,
            },
        };
        match resource {
            Some(vd) => {
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
                let json = serde_json::to_string_pretty(&vd).unwrap_or_default();
                (Some(SelectedVd { id, name, json }), Some(vd))
            }
            None => (None, None),
        }
    };

    // `?run=1` previews the selected view through $sql-run — a plain link, so
    // it works without JavaScript. Capped: this is a preview, not an export.
    let (results, run_error) = match (&selected_value, query.run.as_deref() == Some("1")) {
        (Some(vd), true) => match state.conformance.sql_run(vd, 50, rv.0, &rt.id).await {
            Ok(rows) => (Some(sql_views::build_table(vd, &rows)), None),
            Err(error) => (None, Some(error)),
        },
        _ => (None, None),
    };

    render(SqlViewDefinitionsPage {
        status: current_status(&state, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "sql-view-definitions",
        rail: summaries,
        filter,
        prev_href,
        next_href,
        degraded,
        selected,
        is_new,
        results,
        run_error,
        save_error: None,
        saved: query.saved.as_deref() == Some("1"),
    })
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
    let error_page =
        |save_error: String, json: String, is_new: bool, id: String| SqlViewDefinitionsPage {
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
            results: None,
            run_error: None,
            save_error: Some(save_error),
            saved: false,
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

/// #753 ticket 03 (evaluation POC, not merged upstream): structural +
/// FHIRPath-syntax lint for the ViewDefinition editor's async CodeMirror 6
/// linter (ticket 02's `vd-editor.js`, RF7). Delegates entirely to
/// [`helios_sof::lint::lint_view_definition`] — this handler only decodes
/// the request body and shapes the response; it never touches storage, the
/// tenant, or the configured FHIR version, because the lint itself is
/// purely structural (RF5) and version-agnostic.
///
/// Plain JSON in, JSON out — no htmx swap involved, matching the precedent
/// `/ui/editor/expand` already sets for a browser-facing JSON endpoint that
/// exists to support an editor rather than to mirror the FHIR REST surface.
/// The body is read as raw bytes (not the `Json` extractor) so a malformed
/// body reports RF5's exact `{"error": "..."}` shape instead of axum's
/// generic rejection body.
async fn sql_view_definitions_lint(body: Bytes) -> Response {
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

    let diagnostics = helios_sof::lint::lint_view_definition(&doc);
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
/// their route, and their labels.
struct LibraryKind {
    code: &'static str,
    base_href: &'static str,
    active_page: &'static str,
    title_key: &'static str,
    lede_key: &'static str,
    new_title_key: &'static str,
}

const SQL_QUERY_KIND: LibraryKind = LibraryKind {
    code: "sql-query",
    base_href: "/ui/sql/queries",
    active_page: "sql-queries",
    title_key: "sql-queries-title",
    lede_key: "sql-queries-lede",
    new_title_key: "sql-queries-new-title",
};

const SQL_VIEW_KIND: LibraryKind = LibraryKind {
    code: "sql-view",
    base_href: "/ui/sql/views",
    active_page: "sql-views",
    title_key: "sql-views-title",
    lede_key: "sql-views-lede",
    new_title_key: "sql-views-new-title",
};

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
    title_key: &'static str,
    lede_key: &'static str,
    new_title_key: &'static str,
    rail: Vec<sql_libraries::LibSummary>,
    filter: String,
    degraded: Option<String>,
    selected: Option<SelectedLib>,
    is_new: bool,
    results: Option<sql_views::RunTable>,
    run_error: Option<String>,
    save_error: Option<String>,
    saved: bool,
}

struct SelectedLib {
    id: String,
    name: String,
    json: String,
    sql: String,
}

#[derive(Deserialize, Default)]
struct SqlLibQuery {
    lib: Option<String>,
    filter: Option<String>,
    run: Option<String>,
    saved: Option<String>,
}

async fn sql_library_page(
    state: WebState,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    query: SqlLibQuery,
    kind: &LibraryKind,
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

    let is_new = query.lib.as_deref() == Some("new");
    let (selected, selected_value) = if is_new {
        (
            Some(SelectedLib {
                id: String::new(),
                name: String::new(),
                json: sql_libraries::starter_library(kind.code),
                sql: sql_libraries::STARTER_SQL.to_string(),
            }),
            None,
        )
    } else {
        let wanted = query
            .lib
            .clone()
            .or_else(|| summaries.first().map(|e| e.id.clone()));
        let found = wanted.and_then(|id| {
            libraries
                .iter()
                .position(|l| l.get("id").and_then(serde_json::Value::as_str) == Some(&id))
        });
        match found.map(|i| libraries.swap_remove(i)) {
            Some(lib) => {
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
                let sql = sql_libraries::extract_sql(&lib);
                let json = serde_json::to_string_pretty(&lib).unwrap_or_default();
                (
                    Some(SelectedLib {
                        id,
                        name,
                        json,
                        sql,
                    }),
                    Some(lib),
                )
            }
            None => (None, None),
        }
    };

    let (results, run_error) = match (&selected_value, query.run.as_deref() == Some("1")) {
        (Some(lib), true) => match state.conformance.sql_run(lib, 50, rv.0, &rt.id).await {
            Ok(rows) => (Some(sql_views::build_table(lib, &rows)), None),
            Err(error) => (None, Some(error)),
        },
        _ => (None, None),
    };

    render(SqlLibraryPage {
        status: current_status(&state, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: kind.active_page,
        base_href: kind.base_href,
        title_key: kind.title_key,
        lede_key: kind.lede_key,
        new_title_key: kind.new_title_key,
        rail: summaries,
        filter,
        degraded,
        selected,
        is_new,
        results,
        run_error,
        save_error: None,
        saved: query.saved.as_deref() == Some("1"),
    })
}

#[derive(Deserialize)]
struct SqlLibSaveForm {
    #[serde(default)]
    id: String,
    json: String,
    /// The decoded SQL pane; re-embedded as the base64 attachment on save.
    #[serde(default)]
    sql: String,
    #[serde(default)]
    action: String,
}

async fn sql_library_save(
    state: WebState,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    form: SqlLibSaveForm,
    kind: &LibraryKind,
) -> Response {
    let error_page =
        |save_error: String, json: String, sql: String, is_new: bool, id: String| SqlLibraryPage {
            status: current_status(&state, rv.0, &rt),
            i18n: I18n::new(locale),
            active_page: kind.active_page,
            base_href: kind.base_href,
            title_key: kind.title_key,
            lede_key: kind.lede_key,
            new_title_key: kind.new_title_key,
            rail: Vec::new(),
            filter: String::new(),
            degraded: None,
            selected: Some(SelectedLib {
                name: if is_new { String::new() } else { id.clone() },
                id,
                json,
                sql,
            }),
            is_new,
            results: None,
            run_error: None,
            save_error: Some(save_error),
            saved: false,
        };

    let duplicate = form.action == "duplicate";
    let mut resource: serde_json::Value = match serde_json::from_str(form.json.trim()) {
        Ok(value) => value,
        Err(e) => {
            return render(error_page(
                format!("invalid JSON: {e}"),
                form.json,
                form.sql,
                form.id.is_empty(),
                form.id,
            ));
        }
    };
    if resource
        .get("resourceType")
        .and_then(serde_json::Value::as_str)
        != Some("Library")
    {
        return render(error_page(
            "the document must have resourceType \"Library\"".to_string(),
            form.json,
            form.sql,
            form.id.is_empty(),
            form.id,
        ));
    }
    sql_libraries::embed_sql(&mut resource, &form.sql);

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
        Err(error) => render(error_page(
            error,
            form.json,
            form.sql,
            id.is_none(),
            form.id,
        )),
    }
}

async fn sql_queries_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(query): Query<SqlLibQuery>,
) -> Response {
    sql_library_page(state, locale, rv, rt, query, &SQL_QUERY_KIND).await
}

async fn sql_queries_save(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    axum::Form(form): axum::Form<SqlLibSaveForm>,
) -> Response {
    sql_library_save(state, locale, rv, rt, form, &SQL_QUERY_KIND).await
}

async fn sql_views_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(query): Query<SqlLibQuery>,
) -> Response {
    sql_library_page(state, locale, rv, rt, query, &SQL_VIEW_KIND).await
}

async fn sql_views_save(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    axum::Form(form): axum::Form<SqlLibSaveForm>,
) -> Response {
    sql_library_save(state, locale, rv, rt, form, &SQL_VIEW_KIND).await
}

/// The Files page (#649): a finished job's completion manifest — one row per
/// output with its shard download links, served straight off the FHIR API's
/// result endpoint.
#[derive(Template)]
#[template(path = "pages/sql-files.html")]
struct SqlFilesPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    job: String,
    outputs: Option<Vec<sql_export::ManifestOutput>>,
    format: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize, Default)]
struct SqlFilesQuery {
    job: Option<String>,
}

async fn sql_files_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    headers: HeaderMap,
    Query(query): Query<SqlFilesQuery>,
) -> Response {
    let job = query.job.unwrap_or_default();
    let caller = Caller::from_request(&headers, &rt.id);
    let (outputs, format, error) = if job.is_empty() {
        (None, None, None)
    } else {
        match state.conformance.sql_export_manifest(&job, &caller).await {
            Ok(manifest) => (
                Some(sql_export::manifest_outputs(&manifest)),
                sql_export::manifest_value(&manifest, "_format"),
                None,
            ),
            Err(error) => (None, None, Some(error)),
        }
    };
    render(SqlFilesPage {
        status: current_status(&state, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "sql-files",
        job,
        outputs,
        format,
        error,
    })
}

/// The read-only CapabilityStatement page (#653): the live `/metadata`
/// answer for the sidebar's tenant and FHIR version, summarized and filterable,
/// with the raw statement one fold away.
#[derive(Template)]
#[template(path = "pages/capability-statement.html")]
struct CapabilityPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    /// `None` when the self-fetch failed — the page degrades to a warning.
    view: Option<capability::CapabilityView>,
    /// Pretty-printed JSON for the explicit no-JavaScript fallback. This never
    /// passes through the highlighted renderer.
    raw: String,
    /// Whether this request explicitly asked for the plain JSON fallback.
    raw_requested: bool,
    /// Version- and filter-preserving URL used by the ordinary no-JS link.
    raw_url: String,
    /// Bounded, server-driven JSON root loaded by htmx.
    fragment_url: String,
    /// Effective version carried by the resource filter's ordinary GET form.
    effective_version: &'static str,
    /// The server-side resource-type filter, echoed back into the form.
    filter: String,
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

fn bounded_capability_fragment<T: Template>(template: T) -> Response {
    match template.render() {
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
    match capability_json::plan(&statement, &query.path, query.offset, limit, version) {
        Ok(capability_json::View::Full(json_lines)) => {
            bounded_capability_fragment(CapabilityJsonFullFragment {
                i18n: I18n::new(locale),
                json_lines,
                json_view_id: if query.path.is_empty() {
                    "capability-json".to_string()
                } else {
                    String::new()
                },
                json_view_paths: false,
            })
        }
        Ok(capability_json::View::Outline(outline)) => {
            bounded_capability_fragment(CapabilityJsonOutlineFragment {
                i18n: I18n::new(locale),
                outline,
            })
        }
        Err(capability_json::Error::NotFound) => {
            (StatusCode::NOT_FOUND, "JSON path not found").into_response()
        }
        Err(capability_json::Error::InvalidPointer | capability_json::Error::InvalidPage) => {
            (StatusCode::BAD_REQUEST, "Invalid JSON fragment request").into_response()
        }
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
    let fetched = state.conformance.metadata(version, &rt.id).await;
    let (view, raw) = match fetched {
        Ok(statement) => {
            let mut view = capability::build_view(&statement, version);
            if !filter.is_empty() {
                let needle = filter.to_lowercase();
                view.resources
                    .retain(|r| r.resource_type.to_lowercase().contains(&needle));
            }
            let raw = if raw_requested {
                serde_json::to_string_pretty(&statement).unwrap_or_default()
            } else {
                String::new()
            };
            (Some(view), raw)
        }
        Err(error) => {
            tracing::warn!("CapabilityStatement self-fetch failed: {error}");
            (None, String::new())
        }
    };
    render(CapabilityPage {
        status: current_status(&state, version, &rt),
        i18n: I18n::new(locale),
        active_page: "capability-statement",
        view,
        raw,
        raw_requested,
        raw_url,
        fragment_url: capability_json::root_fragment_url(version),
        effective_version: version.as_str(),
        filter,
    })
}

/// Compartment viewer & tester page.
async fn compartments_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(raw): Query<CompartmentsQuery>,
) -> Response {
    let query = compartments::CmpQuery {
        // Explicit ?version= wins; otherwise the user's stored choice (#343).
        version: raw.version.or_else(|| Some(rv.0.as_str().to_string())),
        def: raw.def,
        tab: raw.tab,
        filter: raw.filter,
        id: raw.id,
        target: raw.target,
    };
    if raw.refresh.is_some() {
        state.compartments.invalidate(&rt.id, query.fhir_version());
    }
    let defs = state
        .compartments
        .definitions(&rt.id, query.fhir_version())
        .await;
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
        let response =
            bounded_capability_fragment(BoundedFragmentTestTemplate { value: &oversized });
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn capability_fragments_report_template_errors() {
        let response = bounded_capability_fragment(FailingFragmentTestTemplate {
            value: FailingDisplay,
        });
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

    /// #753 ticket 01: the CodeMirror 6 + lezer-fhirpath vendoring ritual's
    /// one committed output (`crates/ui/vendor/codemirror/README.md`) is
    /// embedded exactly like any other subfolder asset — `assets/fonts/` is
    /// the existing precedent for rust-embed walking into `assets/vendor/` —
    /// and opens with the license banner rollup.config.js generates, wrapping
    /// the `window.HfsCodeMirror` global tickets 02 and 03 build against.
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
    }

    /// #753 ticket 02: vd-editor.js — the hand-written mount script that
    /// progressively enhances the ViewDefinition textarea with the ticket 01
    /// bundle — is embedded like every other page script.
    #[test]
    fn vd_editor_script_is_embedded() {
        assert!(Assets::get("vd-editor.js").is_some());
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
            rail_entries,
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
            rail_entries,
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
