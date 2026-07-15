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

mod compartments;
mod editor;
mod i18n;
mod json_view;
mod search_params;

use askama::Template;
use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    middleware,
    response::{Html, IntoResponse, Response},
    routing::get,
};
use axum_embed::ServeEmbed;
use axum_htmx::{AutoVaryLayer, HxRequest};
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
    /// Lazily-loaded SearchParameter snapshot per FHIR version (#238).
    sp_catalog: Arc<search_params::SpCatalog>,
    /// Natural-language search feature state (#255).
    nl: Arc<NlSearch>,
}

/// A small, self-contained system-status snapshot — the "real read path" the
/// POC renders. Kept deliberately simple so the crate stays dependency-light;
/// richer read paths (terminology lookups, resource counts) plug in the same way.
struct Status {
    version: &'static str,
    checked_at: u64,
}

/// Dashboard metrics rendered by `pages/index.html` (design: Figma
/// "Dashboard V1.1"). Sample values from the design frame until the real
/// read paths into `helios-persistence` land (README: "Left for follow-up
/// work") — swapping them in is a handler-only change.
struct DashboardMetrics {
    fhir_version: &'static str,
    resource_types: &'static str,
    stored_resources: &'static str,
    export_jobs: &'static str,
    export_jobs_queued: u32,
    uptime_percent: &'static str,
    chart_total: &'static str,
}

impl DashboardMetrics {
    fn sample() -> Self {
        DashboardMetrics {
            fhir_version: "R4",
            resource_types: "142",
            stored_resources: "61.4k",
            export_jobs: "13",
            export_jobs_queued: 1,
            uptime_percent: "99.98",
            chart_total: "1,204",
        }
    }
}

#[derive(Template)]
#[template(path = "pages/index.html")]
struct IndexPage {
    status: Status,
    metrics: DashboardMetrics,
    i18n: I18n,
    /// Which sidebar entry carries `aria-current="page"` (see base.html).
    active_page: &'static str,
    /// Whether the sidebar links to the natural-language search page (#255).
    nl_enabled: bool,
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
    nl_enabled: bool,
    /// Feature state; `configured` picks between the working pane and setup.
    nl: NlSearch,
    /// How-to page for the unconfigured state (docs live in the book).
    docs_url: &'static str,
    resource_types: Vec<String>,
    /// The saved-query controls are the Saved Queries page's job, not this
    /// page's (see `partials/search-builder.html`).
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
    nl_enabled: bool,
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
    nl_enabled: bool,
    view: search_params::SpView,
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
    nl_enabled: bool,
    view: compartments::CmpView,
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

/// Mounts the web UI under `/ui`, falling back to the FHIR REST app for every
/// other path. The UI depends on the rest of the server, never the reverse.
///
/// `data_dir` is the server's data directory (`HFS_DATA_DIR`), where the
/// SearchParameter spec bundles live; `None` falls back to `./data`, matching
/// the storage backends.
///
/// `nl` mirrors the server's natural-language search configuration. With
/// `enabled: false` the `/ui/search` route is never registered, so the page
/// 404s through to the FHIR app exactly as it did before the feature existed.
pub fn mount(
    fhir_app: Router,
    hfs_version: &'static str,
    data_dir: Option<PathBuf>,
    nl: NlSearch,
) -> Router {
    let nl_enabled = nl.enabled;
    let mut router = Router::new()
        .route("/ui", get(index))
        .route("/ui/queries", get(queries))
        .route("/ui/queries/params", get(query_params_catalog))
        .route("/ui/search-parameters", get(search_parameters))
        .route("/ui/compartments", get(compartments_page))
        // Schema-driven resource editor (#264). One POST endpoint applies every
        // structural mutation and re-renders: the document rides with it.
        .route("/ui/editor", get(editor::page))
        .route(
            "/ui/editor/render",
            axum::routing::post(editor::render_body),
        )
        .route("/ui/status", get(status));

    if nl_enabled {
        router = router.route("/ui/search", get(search));
    }

    router
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
            sp_catalog: Arc::new(search_params::SpCatalog::new(data_dir)),
            nl: Arc::new(nl),
        })
        .fallback_service(fhir_app)
}

/// The how-to page for natural-language search, linked from the setup state.
const NL_SEARCH_DOCS: &str =
    "https://heliossoftware.github.io/hfs/components/natural-language-search.html";

/// Full landing page.
async fn index(State(state): State<WebState>, locale: RequestLocale) -> Response {
    render(IndexPage {
        status: current_status(state.version),
        metrics: DashboardMetrics::sample(),
        i18n: I18n::new(locale),
        active_page: "home",
        nl_enabled: state.nl.enabled,
    })
}

/// Search page: natural language and the visual builder over one editable query.
async fn search(State(state): State<WebState>, locale: RequestLocale) -> Response {
    render(SearchPage {
        status: current_status(state.version),
        i18n: I18n::new(locale),
        active_page: "search",
        nl_enabled: state.nl.enabled,
        nl: (*state.nl).clone(),
        docs_url: NL_SEARCH_DOCS,
        resource_types: compartments::resource_type_names(helios_fhir::FhirVersion::default()),
        show_save: false,
    })
}

/// Saved FHIR queries page.
async fn queries(State(state): State<WebState>, locale: RequestLocale) -> Response {
    render(QueriesPage {
        status: current_status(state.version),
        i18n: I18n::new(locale),
        active_page: "queries",
        nl_enabled: state.nl.enabled,
        resource_types: compartments::resource_type_names(helios_fhir::FhirVersion::default()),
        show_save: true,
    })
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
    Query(raw): Query<ParamsCatalogQuery>,
) -> Response {
    let snapshot = state
        .sp_catalog
        .snapshot(helios_fhir::FhirVersion::default());
    let resource_type = raw.resource_type.unwrap_or_default();
    let mut params: Vec<ParamOption> = snapshot
        .params
        .iter()
        .filter(|p| p.applies_to(&resource_type))
        .map(|p| ParamOption {
            code: p.code.clone(),
            type_label: p.param_type.to_string(),
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
}

/// SearchParameter viewer page.
async fn search_parameters(
    State(state): State<WebState>,
    locale: RequestLocale,
    Query(raw): Query<SearchParametersQuery>,
) -> Response {
    let query = search_params::SpQuery {
        version: raw.version,
        base: raw.base.filter(|b| !b.is_empty()),
        ptype: raw.ptype.filter(|t| !t.is_empty()),
        source: raw.source.filter(|s| !s.is_empty()),
        q: raw.q,
        page: raw.page.unwrap_or(1),
        sel: raw.sel.filter(|s| !s.is_empty()),
    };
    let snapshot = state.sp_catalog.snapshot(query.fhir_version());
    render(SearchParametersPage {
        status: current_status(state.version),
        i18n: I18n::new(locale),
        active_page: "search-parameters",
        nl_enabled: state.nl.enabled,
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
}

/// Compartment viewer & tester page.
async fn compartments_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    Query(raw): Query<CompartmentsQuery>,
) -> Response {
    let query = compartments::CmpQuery {
        version: raw.version,
        def: raw.def,
        tab: raw.tab,
        filter: raw.filter,
        id: raw.id,
        target: raw.target,
    };
    match compartments::build_view(&query) {
        Some(view) => render(CompartmentsPage {
            status: current_status(state.version),
            i18n: I18n::new(locale),
            active_page: "compartments",
            nl_enabled: state.nl.enabled,
            view,
        }),
        None => StatusCode::NOT_FOUND.into_response(),
    }
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
        render(IndexPage {
            status,
            metrics: DashboardMetrics::sample(),
            i18n,
            active_page: "home",
            nl_enabled: state.nl.enabled,
        })
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

    #[test]
    fn index_page_renders_version_and_local_assets() {
        let html = IndexPage {
            status: Status {
                version: "1.2.3",
                checked_at: 42,
            },
            metrics: DashboardMetrics::sample(),
            nl_enabled: true,
            i18n: i18n("en"),
            active_page: "home",
        }
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
        let html = IndexPage {
            status: Status {
                version: "1.2.3",
                checked_at: 42,
            },
            metrics: DashboardMetrics::sample(),
            nl_enabled: true,
            i18n: i18n("es"),
            active_page: "home",
        }
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
            },
            i18n: i18n("en"),
            active_page: "queries",
            nl_enabled: true,
            show_save: true,
            resource_types: compartments::resource_type_names(helios_fhir::FhirVersion::default()),
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
        // Resource picker rail: the version's full type list, with count
        // slots the script hydrates via _summary=count.
        assert!(html.contains(r#"data-rail-type="Patient""#));
        assert!(html.contains(r#"data-rail-type="Observation""#));
        assert!(html.contains(r#"data-count-for="Patient""#));
        // This page, not Home, carries aria-current in the sidebar.
        assert!(html.contains(r#"href="/ui/queries" aria-current="page""#));
        assert!(!html.contains(r#"href="/ui" aria-current="page""#));
        // The delete-confirm string reaches the script with its {name} slot.
        assert!(html.contains("{name}"));
    }

    #[test]
    fn queries_page_renders_in_the_negotiated_locale() {
        let html = QueriesPage {
            status: Status {
                version: "1.2.3",
                checked_at: 42,
            },
            i18n: i18n("es"),
            active_page: "queries",
            nl_enabled: true,
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
                },
                ParamOption {
                    code: "name".into(),
                    type_label: "string".into(),
                },
            ],
        }
        .render()
        .expect("partial renders");

        assert!(html.contains(r#"<datalist id="param-options">"#));
        assert!(html.contains(r#"<option value="birthdate" label="date">"#));
        assert!(!html.contains("<html"), "fragment, not a page");
    }

    /// Both theme buttons render, and icons are inlined (so `currentColor`
    /// theming applies) rather than referenced as external images.
    #[test]
    fn index_page_renders_theme_toggle_and_inline_icons() {
        let html = IndexPage {
            status: Status {
                version: "1.2.3",
                checked_at: 42,
            },
            metrics: DashboardMetrics::sample(),
            nl_enabled: true,
            i18n: i18n("en"),
            active_page: "home",
        }
        .render()
        .expect("index renders");

        assert!(html.contains(r#"data-set-theme="light""#));
        assert!(html.contains(r#"data-set-theme="dark""#));
        assert!(html.contains("<svg"));
        assert!(html.contains(r#"fill="currentColor""#));
    }
}
