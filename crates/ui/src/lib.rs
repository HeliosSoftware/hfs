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

mod bulk_export;
mod bulk_import;
mod compartments;
mod conformance;
mod editor;
mod history;
mod i18n;
mod json_view;
mod search_params;
mod tenants;

#[doc(hidden)]
pub use conformance::{ConformanceSource, StaticConformanceSource};

use askama::Template;
use axum::{
    Router,
    extract::{Query, RawQuery, State},
    http::StatusCode,
    middleware,
    response::{Html, IntoResponse, Response},
    routing::get,
};
use axum_embed::ServeEmbed;
use axum_htmx::{AutoVaryLayer, HxRequest};
use chrono::{DateTime, Datelike, Duration, Utc};
use helios_observability::dashboard::{
    DashboardPoint, DashboardSeries, DashboardSnapshot, DashboardWindow, ExportJobCounts,
};
use helios_persistence::core::{ResourceStorage, SettingsStore};
use i18n::{I18n, RequestLocale};
use rust_embed::RustEmbed;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
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
/// feature. The UI never sees the API key itself — only whether one is set.
#[derive(Clone, Debug, Default)]
pub struct NlSearch {
    /// `HFS_NL_SEARCH_ENABLED` — the operator's kill switch.
    pub enabled: bool,
    /// Whether `HFS_NL_SEARCH_API_KEY` is set.
    pub configured: bool,
    /// `HFS_NL_SEARCH_MODEL` — shown in the setup state so an operator can see
    /// what they would be billed for.
    pub model: String,
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
    /// Read/write path for the tenant-maintenance page. `None` when the host did
    /// not wire storage in (e.g. the UI-only unit tests), in which case the page
    /// reports the registry as unavailable rather than crashing.
    tenants: Option<Arc<dyn ResourceStorage>>,
    /// Server data directory (`HFS_DATA_DIR`), used to seed a newly-provisioned
    /// tenant's conformance resources from the tenant-maintenance page.
    data_dir: Option<PathBuf>,
    /// The server's default FHIR version, used when seeding a new tenant.
    fhir_version: helios_fhir::FhirVersion,
    /// The server's default tenant id — the fallback when no stored choice
    /// exists (#344).
    default_tenant: String,
    /// Terminology server base URL (`HFS_TERMINOLOGY_SERVER`), when one is
    /// configured — powers the editor's live `$expand` pickers (#365).
    terminology: Option<String>,
    /// Per-user settings, for the persisted FHIR-version choice (#343). `None`
    /// when the backend has no settings store; the selector then applies
    /// per-page only.
    settings: Option<Arc<dyn SettingsStore>>,
}

/// The settings keys holding the user's FHIR-version and tenant choices, and
/// the user key the settings resolve under. The key mirrors `helios-rest`'s
/// `UserKey` post-#270 encoding — `u2:{issuer_len}:{issuer}:{subject}` from an
/// authenticated principal, `l2:` when auth is disabled (`/ui` also sits
/// outside the auth layer today; #320 tracks the authenticated modes). Keep in
/// step with `crates/rest/src/extractors/user.rs`.
///
/// These two go through [`SettingsStore`] **directly**, not through
/// `/_user/settings`, so they bypass the per-tenant scoping that handler applies
/// (issue #313). That is correct precisely because both are in
/// [`GLOBAL_SETTINGS_KEYS`](helios_persistence::core::GLOBAL_SETTINGS_KEYS) —
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
    /// Whether this install has any tenant beyond the server default — the
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
/// stored settings — one settings read per page load, the documented cost model
/// of that store — falling back to the server defaults. A stored tenant that is
/// no longer provisioned falls back too, keeping the provisioned-only model.
async fn resolve_prefs(
    State(state): State<WebState>,
    mut request: axum::extract::Request,
    next: middleware::Next,
) -> Response {
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

/// A small, self-contained system-status snapshot — the "real read path" the
/// POC renders. Kept deliberately simple so the crate stays dependency-light;
/// richer read paths (terminology lookups, resource counts) plug in the same way.
pub(crate) struct Status {
    pub(crate) version: &'static str,
    checked_at: u64,
    /// The effective FHIR version for this request — the sidebar selector's
    /// label (#343).
    fhir_version: helios_fhir::FhirVersion,
    /// The effective tenant for this request — the tenant selector's label
    /// (#344).
    tenant_id: String,
    tenant_display: Option<String>,
    /// Whether the sidebar renders the tenant picker (#544).
    show_tenant_picker: bool,
}

impl Status {
    /// The default FHIR version's display label (`"R4"`, `"R5"`, …).
    pub(crate) fn fhir_version_label(&self) -> &'static str {
        self.fhir_version.as_str()
    }

    /// Labels of every FHIR version compiled into this build, in spec order —
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
/// `resource_types`, `stored_resources`, `fhir_version`, `export_jobs`,
/// `import_jobs`, and `chart_total` are derived from the live
/// [`DashboardSnapshot`] (default tenant). `uptime_percent` describes
/// availability history, which is not part of that read path and remains a
/// placeholder value until #540 wires it to a real source.
struct DashboardMetrics {
    fhir_version: String,
    resource_types: String,
    stored_resources: String,
    /// Bulk-export jobs for the tenant; `None` renders the unavailable state.
    export_jobs: Option<ExportJobCounts>,
    /// Active bulk-submit (import) jobs for the tenant; `None` renders the
    /// unavailable state.
    import_jobs: Option<u64>,
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

/// One plotted series of the "resources over time" chart: SVG geometry plus
/// the identity the legend and tooltip need (#555).
struct ChartSeriesView {
    resource_type: String,
    /// 1-based palette slot (`--series-N` custom properties, 6 defined).
    color: usize,
    /// `"x,y x,y …"` coordinate list for the `<polyline>`.
    polyline: String,
    total: String,
}

/// One row of the chart's tabular alternative: a bucket label and the
/// cumulative value of every plotted series at that bucket (#555, a11y).
struct ChartTableRow {
    label: String,
    values: Vec<String>,
}

/// Server-computed SVG geometry for the "resources over time" chart.
struct ChartView {
    /// Whether any non-empty series was plotted (`false` → empty state).
    has_data: bool,
    series: Vec<ChartSeriesView>,
    /// Horizontal value gridlines, top (largest) to bottom (zero).
    y_ticks: Vec<AxisTick>,
    /// X-axis date labels at evenly spaced sample points.
    x_ticks: Vec<AxisTick>,
    /// viewBox height — 300, or 520 when the card is expanded.
    height: i64,
    expanded: bool,
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

/// One entry in the chart legend: a plotted series, with a link that removes
/// it from the charted set (present only while more than one is plotted).
struct LegendEntry {
    resource_type: String,
    total: String,
    color: usize,
    href: Option<String>,
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
/// charted set and expansion.
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
    /// The expand/collapse link (the inverse of the current state).
    expand_href: String,
    /// True when no provider answered and the placeholder snapshot is shown —
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
/// instead of an input — the page still works, in visual-builder mode.
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
    /// The search-builder partial's save controls are the Saved Queries page's
    /// job, not this one's.
    show_save: bool,
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
    /// Counts hydrate client-side via `_summary=count`.
    resource_types: Vec<String>,
    show_save: bool,
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
    /// Comma-joined target resource types (reference params only, else empty) —
    /// the builder's chaining controls read these as `data-targets`.
    targets: String,
}

/// Parameter suggestions for the search builder (`/ui/queries/params`),
/// rendered from the same registry snapshot the SearchParameter viewer
/// reads. An HTML fragment the page swaps per resource type — hypermedia,
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
    /// The versions being compared, for the heading (`v3 → v4`).
    from_label: String,
    to_label: String,
    show_metadata: bool,
    /// A version was deleted (an R6 destructive op): render a state banner
    /// rather than a diff against a tombstone.
    deleted: bool,
    /// The two documents could not be parsed — the fragment says so instead of
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
) -> Router {
    let source: Arc<dyn ConformanceSource> = Arc::new(conformance::HttpConformanceSource::new(
        self_base_url,
        outbound_auth,
    ));
    mount_with_conformance_source(
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
) -> Router {
    let nl_enabled = nl.enabled;

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
        .route("/ui/compartments", get(compartments_page))
        // Batch/Transaction workspace (#476): upload → preflight → response.
        .route("/ui/batch", get(batch_page))
        // Schema-driven resource editor (#264). One POST endpoint applies every
        // structural mutation and re-renders: the document rides with it.
        .route("/ui/editor", get(editor::page))
        .route("/ui/editor/expand", get(editor::expand))
        .route(
            "/ui/editor/render",
            axum::routing::post(editor::render_body),
        )
        .route("/ui/status", get(status))
        .route("/ui/history", get(history_page))
        // The diff is computed server-side (the decision in
        // docs/history-diff-rendering.md); the browser posts the two versions
        // it fetched from `_history`.
        .route("/ui/history/diff", axum::routing::post(history_diff))
        .route(
            "/ui/bulk-export",
            get(bulk_export::page).post(bulk_export::start),
        )
        .route("/ui/bulk-export/active", get(bulk_export::active))
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
        .route(
            "/ui/bulk-import/{id}/manifests/{mid}/replace",
            axum::routing::post(bulk_import::replace_manifest),
        )
        .route(
            "/ui/bulk-import/{id}/manifests/{mid}/abort",
            axum::routing::post(bulk_import::abort_manifest),
        )
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
            "/ui/bulk-import/{id}/abort",
            axum::routing::post(bulk_import::abort),
        )
        .route(
            "/ui/bulk-import/{id}/complete",
            axum::routing::post(bulk_import::complete),
        )
        .route(
            "/ui/bulk-import/{id}/submit-all",
            axum::routing::post(bulk_import::submit_all),
        )
        .route(
            "/ui/bulk-import/{id}/manifests",
            axum::routing::post(bulk_import::add_manifest),
        )
        .route(
            "/ui/bulk-import/{id}/manifests/{mid}/delete",
            axum::routing::post(bulk_import::delete_manifest),
        )
        .route(
            "/ui/bulk-import/{id}/manifests/{mid}/submit",
            axum::routing::post(bulk_import::submit_manifest),
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
        compartments: Arc::new(compartments::CompartmentCatalog::new(source)),
        nl: Arc::new(nl),
        tenants,
        settings,
        data_dir,
        fhir_version,
        default_tenant,
        terminology,
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

/// Form body for `POST /ui/version` — the sidebar selector's submit.
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

    // Bounce back to the submitting page — same-origin `/ui` paths only.
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

/// `GET /ui/tenant/options` — the registry's provisioned tenants as selector
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

/// Form body for `POST /ui/tenant` — the tenant selector's submit.
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

/// Full landing page. `?types=<A,B,…>` selects which resource types the chart
/// plots (defaults to the tenant's largest stored types); unknown names are
/// dropped by the provider, so the set is always real. The legacy `?type=`
/// single selection still works. `?window=<1h|24h|30d>` selects the sampling
/// window, falling back to [`DashboardWindow::default`] for anything
/// unrecognised, and `?expand=1` renders the taller chart.
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
    let expand = query_value(query.as_deref(), "expand").as_deref() == Some("1");
    render(build_index_page(state.version, locale, types, window, expand, rv.0, &rt).await)
}

/// Search page: natural language and the visual builder over one editable query.
async fn search(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
) -> Response {
    let resource_types = state
        .compartments
        .resource_type_names(&rt.id, helios_fhir::FhirVersion::default())
        .await;
    render(SearchPage {
        status: current_status(state.version, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "search",
        nl: (*state.nl).clone(),
        docs_url: NL_SEARCH_DOCS,
        resource_types,
        show_save: false,
    })
}

/// Saved FHIR queries page.
async fn queries(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
) -> Response {
    let resource_types = state
        .compartments
        .resource_type_names(&rt.id, helios_fhir::FhirVersion::default())
        .await;
    render(QueriesPage {
        status: current_status(state.version, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "queries",
        resource_types,
        show_save: true,
    })
}

/// Resources page (#282): the primary read/write workspace. Ties together the
/// type filter, the search (natural-language + visual builder), and — on a
/// result click — the edit modal that reuses the schema-driven editor, with
/// save / delete / version history / diff. The pieces are the same partials the
/// Search and Editor pages render; this page is the place they come together.
async fn resources(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(query): Query<ResourcesQuery>,
) -> Response {
    let resource_types = state
        .compartments
        .resource_type_names(&rt.id, helios_fhir::FhirVersion::default())
        .await;
    render(ResourcesPage {
        status: current_status(state.version, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "resources",
        nl: (*state.nl).clone(),
        docs_url: NL_SEARCH_DOCS,
        resource_types,
        // The type the rail opens focused on (from the nav submenu deep link).
        selected_type: query.resource_type.unwrap_or_else(|| "Patient".to_string()),
        show_save: false,
    })
}

/// Query string for the Resources page: an optional pre-selected type, so the
/// nav submenu can deep-link `/ui/resources?type=Observation`.
#[derive(Deserialize, Default)]
struct ResourcesQuery {
    #[serde(rename = "type")]
    resource_type: Option<String>,
}

#[derive(Deserialize, Default)]
struct ParamsCatalogQuery {
    #[serde(rename = "type")]
    resource_type: Option<String>,
}

/// Parameter datalist for the search builder: the active parameters that
/// apply to the given resource type (including `Resource` /
/// `DomainResource`-level ones), from the default-version snapshot.
async fn query_params_catalog(
    State(state): State<WebState>,
    rt: RequestTenant,
    Query(raw): Query<ParamsCatalogQuery>,
) -> Response {
    let snapshot = state
        .sp_catalog
        .snapshot(&rt.id, helios_fhir::FhirVersion::default())
        .await;
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
        status: current_status(state.version, rv.0, &rt),
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

/// Batch/Transaction workspace page (#476). The shell is server-rendered;
/// batch.js drives upload → preflight → execute → response entirely against
/// the ordinary FHIR root, so this crate never touches storage.
async fn batch_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
) -> Response {
    render(BatchPage {
        status: current_status(state.version, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "batch",
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
            status: current_status(state.version, rv.0, &rt),
            i18n: I18n::new(locale),
            active_page: "compartments",
            view,
        }),
        // No definitions means the self-fetch degraded (an outage, or auth
        // without an outbound token, #320) — a warning, not a 404. The failed
        // fetch is not cached, so the next request re-attempts it.
        None => render(CompartmentsDegradedPage {
            status: current_status(state.version, rv.0, &rt),
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
    let status = current_status(state.version, rv.0, &rt);
    let i18n = I18n::new(locale);
    if is_htmx {
        render(StatusPartial { status, i18n })
    } else {
        render(
            build_index_page(
                state.version,
                locale,
                Vec::new(),
                DashboardWindow::default(),
                false,
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
        status: current_status(state.version, rv.0, &rt),
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
/// placeholder data when no provider is registered — in which case the page
/// says so explicitly rather than presenting invented numbers as real (#555).
async fn build_index_page(
    version: &'static str,
    locale: RequestLocale,
    types: Vec<String>,
    window: DashboardWindow,
    expand: bool,
    fhir_version: helios_fhir::FhirVersion,
    tenant: &RequestTenant,
) -> IndexPage {
    let status = current_status(version, fhir_version, tenant);
    let i18n = I18n::new(locale);
    let live = helios_observability::dashboard::snapshot(window, &tenant.id, &types).await;
    let sample_data = live.is_none();
    let snapshot = live.unwrap_or_else(|| sample_snapshot(window));
    let dash = build_dashboard(&snapshot, expand);
    IndexPage {
        status,
        metrics: dash.metrics,
        chart: dash.chart,
        legend: dash.legend,
        picker: dash.picker,
        windows: dash.windows,
        expand_href: dash.expand_href,
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
    expand_href: String,
}

/// A `/ui` link carrying the whole chart state: charted set, window, and
/// expansion. Every selector emits these so changing one control keeps the
/// other two.
fn dash_href(types: &[String], window: DashboardWindow, expand: bool) -> String {
    let mut href = format!("/ui?types={}&window={}", types.join(","), window.as_str());
    if expand {
        href.push_str("&expand=1");
    }
    href
}

/// Projects a [`DashboardSnapshot`] into the headline metrics, chart geometry,
/// and the three selectors (type picker, legend, time window) the template
/// renders. The charted set is `snapshot.series` itself — the provider already
/// resolved the request to real stored types.
fn build_dashboard(snapshot: &DashboardSnapshot, expand: bool) -> DashboardView {
    let charted: Vec<String> = snapshot
        .series
        .iter()
        .map(|s| s.resource_type.clone())
        .collect();

    let chart = build_chart(&snapshot.series, snapshot.window, expand);

    // The picker offers every stored type; each option toggles membership.
    let picker = snapshot
        .available
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
                href: dash_href(&toggled, snapshot.window, expand),
                selected,
            }
        })
        .collect();

    // The legend names each plotted series; while more than one is plotted,
    // an entry doubles as its own remove link.
    let legend = snapshot
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let href = (snapshot.series.len() > 1).then(|| {
                let rest: Vec<String> = charted
                    .iter()
                    .filter(|c| **c != s.resource_type)
                    .cloned()
                    .collect();
                dash_href(&rest, snapshot.window, expand)
            });
            LegendEntry {
                resource_type: s.resource_type.clone(),
                total: grouped(s.total),
                color: i % SERIES_COLORS + 1,
                href,
            }
        })
        .collect();

    let windows = DashboardWindow::ALL
        .into_iter()
        .map(|w| WindowEntry {
            label: w.as_str().to_string(),
            href: dash_href(&charted, w, expand),
            active: w == snapshot.window,
        })
        .collect();

    let metrics = DashboardMetrics {
        fhir_version: snapshot.fhir_version.clone(),
        resource_types: snapshot.distinct_types.to_string(),
        stored_resources: compact_count(snapshot.total_resources),
        export_jobs: snapshot.export_jobs,
        import_jobs: snapshot.import_jobs_active,
        // Uptime is still a placeholder until #540 wires it to a real source.
        uptime_percent: "99.98".to_string(),
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
        expand_href: dash_href(&charted, snapshot.window, !expand),
    }
}

// Chart plot area within the `0 0 1060 H` viewBox: the value axis occupies the
// left gutter (x < 40), the date axis the bottom 22 units.
const PLOT_LEFT: i64 = 40;
const PLOT_RIGHT: i64 = 1060;
const PLOT_TOP: i64 = 10;
/// Default and expanded viewBox heights (#555).
const CHART_HEIGHT: i64 = 300;
const CHART_HEIGHT_EXPANDED: i64 = 520;
/// Palette slots defined as `--series-N` custom properties in app.css.
const SERIES_COLORS: usize = 6;
/// Most series plotted at once — mirrors the provider's `MAX_CHARTED_TYPES`
/// (and the palette), so a picker link never asks for more than the server
/// will chart. The default selection is smaller (the provider's three).
const CHART_MAX_SERIES: usize = 6;

/// Computes the SVG geometry for one resource type's cumulative series. `window`
/// decides only the x-axis label format — a calendar date over daily buckets, a
/// UTC clock time over intraday ones.
fn build_chart(all: &[DashboardSeries], window: DashboardWindow, expand: bool) -> ChartView {
    let height = if expand {
        CHART_HEIGHT_EXPANDED
    } else {
        CHART_HEIGHT
    };
    let plot_bottom = height - 22;
    let plotted: Vec<&DashboardSeries> = all.iter().filter(|s| !s.points.is_empty()).collect();
    let has_data = plotted.iter().any(|s| s.total > 0) && !plotted.is_empty();
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
            expanded: expand,
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

    // One y scale across every plotted series, so the curves are comparable.
    let peak = plotted
        .iter()
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
            total: grouped(s.total),
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
        expanded: expand,
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

pub(crate) fn current_status(
    version: &'static str,
    fhir_version: helios_fhir::FhirVersion,
    tenant: &RequestTenant,
) -> Status {
    Status {
        version,
        checked_at: unix_timestamp_seconds(),
        fhir_version,
        tenant_id: tenant.id.clone(),
        tenant_display: tenant.display.clone(),
        show_tenant_picker: tenant.multi,
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
        let dash = build_dashboard(&sample_snapshot(DashboardWindow::default()), false);
        IndexPage {
            status: Status {
                version,
                checked_at,
                fhir_version: helios_fhir::FhirVersion::R4,
                tenant_id: "default".to_string(),
                tenant_display: None,
                show_tenant_picker: true,
            },
            metrics: dash.metrics,
            chart: dash.chart,
            legend: dash.legend,
            picker: dash.picker,
            windows: dash.windows,
            expand_href: dash.expand_href,
            sample_data: true,
            i18n,
            active_page: "home",
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
                fhir_version: helios_fhir::FhirVersion::R4,
                tenant_id: "default".to_string(),
                tenant_display: None,
                show_tenant_picker: true,
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
        let html = QueriesPage {
            status: Status {
                version: "1.2.3",
                checked_at: 42,
                fhir_version: helios_fhir::FhirVersion::R4,
                tenant_id: "default".to_string(),
                tenant_display: None,
                show_tenant_picker: true,
            },
            i18n: i18n("en"),
            active_page: "queries",
            show_save: true,
            resource_types: vec!["Patient".to_string(), "Observation".to_string()],
        }
        .render()
        .expect("queries page renders");

        assert!(html.contains(r#"id="saved-query-form""#));
        assert!(html.contains(r#"id="saved-queries""#));
        assert!(html.contains("/ui/assets/saved-queries.js"));
        // Search Builder: the featured GET URL input, both submit intents,
        // and the Recent dropdown shell the script hydrates.
        assert!(html.contains(r#"name="url""#));
        assert!(html.contains(r#"data-intent="run""#));
        assert!(html.contains(r#"data-intent="save""#));
        assert!(html.contains(r#"id="recent-searches""#));
        // The Recent panel closes from an explicit X as well as outside
        // click / Esc (addbox.js covers details.menu too).
        assert!(html.contains("data-addbox-close"));
        // Resource picker rail: the version's full type list, with count
        // slots the script hydrates via _summary=count.
        assert!(html.contains(r#"data-rail-type="Patient""#));
        assert!(html.contains(r#"data-rail-type="Observation""#));
        assert!(html.contains(r#"data-count-for="Patient""#));
        // Saved Queries has no nav entry any more (#282 folded search / editor
        // / history / saved-queries into Resources); the route still renders.
        assert!(!html.contains(r#"href="/ui/queries" aria-current="page""#));
        // The delete-confirm string reaches the script with its {name} slot.
        assert!(html.contains("{name}"));
    }

    #[test]
    fn queries_page_renders_in_the_negotiated_locale() {
        let html = QueriesPage {
            status: Status {
                version: "1.2.3",
                checked_at: 42,
                fhir_version: helios_fhir::FhirVersion::R4,
                tenant_id: "default".to_string(),
                tenant_display: None,
                show_tenant_picker: true,
            },
            i18n: i18n("es"),
            active_page: "queries",
            show_save: true,
            resource_types: vec!["Patient".to_string()],
        }
        .render()
        .expect("queries page renders");

        assert!(html.contains("Consultas guardadas"));
    }

    /// The saved-queries script owns a structural read-modify-write against
    /// the shared settings document, so — unlike theme.js — it must use the
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
        let dash = build_dashboard(&sample_snapshot(DashboardWindow::default()), false);

        // Every series the snapshot carries is plotted, on one shared y scale.
        assert!(dash.chart.has_data);
        assert_eq!(dash.chart.series.len(), 4);
        assert!(dash.chart.series.iter().all(|s| !s.polyline.is_empty()));
        assert_eq!(dash.chart.y_ticks.len(), 5);
        assert!(dash.chart.types_label.contains("Observation"));

        // The legend names each plotted series; while several are plotted each
        // entry is a remove link that drops exactly itself and keeps the window.
        assert_eq!(dash.legend.len(), 4);
        let observation = dash
            .legend
            .iter()
            .find(|e| e.resource_type == "Observation")
            .expect("Observation in legend");
        let href = observation
            .href
            .as_deref()
            .expect("removable while several are plotted");
        assert!(!href.contains("Observation"));
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
        let windows = build_dashboard(&sample_snapshot(DashboardWindow::LastHour), false).windows;

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
        let hour_chart = build_dashboard(&sample_snapshot(DashboardWindow::LastHour), false).chart;
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

        let month_chart =
            build_dashboard(&sample_snapshot(DashboardWindow::LastMonth), false).chart;
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
            let chart = build_dashboard(&snapshot, false).chart;
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

    /// The expanded chart grows the viewBox and moves the date row with it;
    /// the tooltip carrier and the tabular alternative describe the same
    /// buckets the axis labels sample.
    #[test]
    fn expand_grows_the_plot_and_the_carriers_stay_consistent() {
        let dash = build_dashboard(&sample_snapshot(DashboardWindow::default()), true);
        assert!(dash.chart.expanded);
        assert_eq!(dash.chart.height, 520);
        assert!(dash.chart.x_ticks.iter().all(|t| t.label_y == 518));
        // The expand link flips back to the compact view.
        assert!(!dash.expand_href.contains("expand=1"));

        let tip: serde_json::Value =
            serde_json::from_str(&dash.chart.tip_json).expect("tip carrier is valid JSON");
        assert_eq!(tip["series"].as_array().map(Vec::len), Some(4));
        assert_eq!(
            tip["labels"].as_array().map(Vec::len),
            tip["xs"].as_array().map(Vec::len)
        );
        assert_eq!(dash.chart.table.len(), dash.chart.x_ticks.len());
        assert!(dash.chart.table.iter().all(|r| r.values.len() == 4));
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
        let dash = build_dashboard(&empty, false);
        assert!(!dash.chart.has_data);
        assert!(dash.chart.series.is_empty());
        assert!(dash.legend.is_empty());
        assert!(dash.picker.is_empty());
        assert_eq!(dash.metrics.chart_total, "0");
        // The window selector still renders, so an empty server is not a dead end.
        assert_eq!(dash.windows.len(), DashboardWindow::ALL.len());
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
            false,
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
            false,
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
