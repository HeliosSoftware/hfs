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

mod i18n;
mod tenants;

use askama::Template;
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    middleware,
    response::{Html, IntoResponse, Response},
    routing::get,
};
use axum_embed::ServeEmbed;
use axum_htmx::{AutoVaryLayer, HxRequest};
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

/// Full landing page.
async fn index(State(state): State<WebState>, locale: RequestLocale) -> Response {
    render(IndexPage {
        status: current_status(state.version),
        metrics: DashboardMetrics::sample(),
        i18n: I18n::new(locale),
    })
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
        })
    }
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

    #[test]
    fn index_page_renders_version_and_local_assets() {
        let html = IndexPage {
            status: Status {
                version: "1.2.3",
                checked_at: 42,
            },
            metrics: DashboardMetrics::sample(),
            i18n: i18n("en"),
        }
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
        let html = IndexPage {
            status: Status {
                version: "1.2.3",
                checked_at: 42,
            },
            metrics: DashboardMetrics::sample(),
            i18n: i18n("es"),
        }
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
        let html = IndexPage {
            status: Status {
                version: "1.2.3",
                checked_at: 42,
            },
            metrics: DashboardMetrics::sample(),
            i18n: i18n("en"),
        }
        .render()
        .expect("index renders");

        assert!(html.contains(r#"data-set-theme="light""#));
        assert!(html.contains(r#"data-set-theme="dark""#));
        assert!(html.contains("<svg"));
        assert!(html.contains(r#"fill="currentColor""#));
    }
}
