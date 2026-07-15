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

mod history;
mod i18n;

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

/// History & Versions screen (#236, Figma "History & Versions"): the version
/// rail and the two-layer diff. The shell is server-rendered; the version list
/// and the two compared versions are fetched by the browser from the ordinary
/// `_history` / `vread` FHIR API, then posted to [`history_diff`] to render.
#[derive(Template)]
#[template(path = "pages/history.html")]
struct HistoryPage {
    status: Status,
    i18n: I18n,
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
pub fn mount(fhir_app: Router, hfs_version: &'static str) -> Router {
    Router::new()
        .route("/ui", get(index))
        .route("/ui/status", get(status))
        .route("/ui/history", get(history_page))
        // The diff is computed server-side (the decision in
        // docs/history-diff-rendering.md); the browser posts the two versions
        // it fetched from `_history`.
        .route("/ui/history/diff", axum::routing::post(history_diff))
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

/// History & Versions page shell.
async fn history_page(State(state): State<WebState>, locale: RequestLocale) -> Response {
    render(HistoryPage {
        status: current_status(state.version),
        i18n: I18n::new(locale),
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
            i18n: i18n("en"),
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
            i18n: i18n("es"),
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
