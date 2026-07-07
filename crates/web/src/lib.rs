//! `helios-web` — a server-rendered, **HTMX-first** web UI for the Helios FHIR
//! Server (HFS).
//!
//! This crate is intentionally thin. Handlers do three things and no more:
//!
//! 1. parse the request,
//! 2. call into the existing HFS crates for data (`helios-rest`,
//!    `helios-persistence`, `helios-hts`, …), and
//! 3. render an HTML template.
//!
//! All markup lives in `templates/`; all browser assets live in `assets/`.
//! There is deliberately **no** HTML in Rust source and **no** browser-facing
//! JSON API — HTMX consumes HTML fragments, not JSON. See `README.md` for the
//! full rationale and the "rules of the road".
//!
//! # Proof of concept
//!
//! [`router`] returns an [`axum::Router`] exposing:
//!
//! - `GET /`            — a full HTML page (the app shell + an active-search box)
//! - `GET /search?q=`   — an **HTML fragment** of matching results, swapped into
//!   the page by HTMX. On a hard navigation (no `HX-Request` header) it renders
//!   the full page instead, so the UI degrades gracefully without JavaScript.
//! - `GET /assets/*`    — vendored, version-pinned static files (HTMX, CSS).
//!
//! The search is backed by a static, in-memory dataset (FHIR resource-type
//! names) purely to keep the skeleton self-contained. Replacing it with a real
//! read path (e.g. a CodeSystem/ValueSet lookup) is the first follow-up task.
//!
//! Run it standalone:
//!
//! ```bash
//! cargo run -p helios-web --example serve
//! # then open http://127.0.0.1:8088/
//! ```

use std::path::PathBuf;

use askama::Template;
use axum::{
    Router,
    extract::Query,
    http::HeaderMap,
    response::{Html, IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use tower_http::services::ServeDir;

/// `?q=…` search parameter.
#[derive(Deserialize)]
struct SearchParams {
    q: Option<String>,
}

/// A single result row rendered by the search partial.
pub struct Hit {
    pub name: String,
}

// ---- Templates -------------------------------------------------------------
//
// Template files live under `crates/web/templates/`. Askama checks them at
// compile time, so a typo in a field name or a missing template is a build
// error, not a runtime 500.

#[derive(Template)]
#[template(path = "pages/index.html")]
struct IndexPage {
    query: String,
    hits: Vec<Hit>,
}

#[derive(Template)]
#[template(path = "partials/search_results.html")]
struct SearchResults {
    query: String,
    hits: Vec<Hit>,
}

// ---- Handlers --------------------------------------------------------------

/// `GET /` — the application shell plus an empty search box.
async fn index() -> Response {
    render(IndexPage {
        query: String::new(),
        hits: Vec::new(),
    })
}

/// `GET /search?q=…`
///
/// Demonstrates the canonical HTMX pattern: when the request carries the
/// `HX-Request` header we return **only the results fragment** (for HTMX to
/// swap into `#results`); otherwise — e.g. a shared/bookmarked URL or a
/// no-JavaScript client — we return the **full page** with results embedded.
async fn search(headers: HeaderMap, Query(params): Query<SearchParams>) -> Response {
    let query = params.q.unwrap_or_default();
    let hits = search_dataset(&query);

    if is_htmx_request(&headers) {
        render(SearchResults { query, hits })
    } else {
        render(IndexPage { query, hits })
    }
}

// ---- Router ----------------------------------------------------------------

/// Build the web-UI router.
///
/// Mount it under a path prefix from the host application, e.g. in the `hfs`
/// binary:
///
/// ```ignore
/// let app = existing_router.nest("/ui", helios_web::router());
/// ```
///
/// `assets_dir` is the directory containing the vendored static files. Callers
/// that embed assets in the binary (e.g. via `rust-embed`) can serve them
/// however they like and pass their own asset route instead.
pub fn router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/search", get(search))
        .nest_service("/assets", ServeDir::new(assets_dir()))
}

/// Absolute path to this crate's vendored `assets/` directory.
///
/// Resolved from `CARGO_MANIFEST_DIR` so it works regardless of the process's
/// current working directory. A production deployment that embeds assets in the
/// binary would not use this.
pub fn assets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
}

// ---- Helpers ---------------------------------------------------------------

/// Is this an HTMX-issued request? HTMX sets `HX-Request: true` on every
/// request it makes (see <https://htmx.org/reference/#request_headers>).
fn is_htmx_request(headers: &HeaderMap) -> bool {
    headers
        .get("HX-Request")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Render an Askama template into an HTML response, turning a render failure
/// into a 500 rather than panicking.
fn render<T: Template>(tpl: T) -> Response {
    match tpl.render() {
        Ok(body) => Html(body).into_response(),
        Err(err) => {
            tracing::error!(error = %err, "template render failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "template render error",
            )
                .into_response()
        }
    }
}

/// Case-insensitive substring match over a static dataset.
///
/// Placeholder for a real read path (persistence / terminology lookup). An
/// empty query returns nothing so the initial page load is quiet.
fn search_dataset(query: &str) -> Vec<Hit> {
    const DATA: &[&str] = &[
        "Patient",
        "Observation",
        "Condition",
        "Encounter",
        "Procedure",
        "MedicationRequest",
        "DiagnosticReport",
        "CodeSystem",
        "ValueSet",
        "ConceptMap",
    ];
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    DATA.iter()
        .filter(|name| name.to_ascii_lowercase().contains(&q))
        .map(|name| Hit {
            name: (*name).to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_no_hits() {
        assert!(search_dataset("").is_empty());
        assert!(search_dataset("   ").is_empty());
    }

    #[test]
    fn search_is_case_insensitive_substring() {
        let hits = search_dataset("val");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "ValueSet");
    }

    #[test]
    fn index_page_renders() {
        let page = IndexPage {
            query: String::new(),
            hits: Vec::new(),
        };
        let html = page.render().expect("renders");
        assert!(html.contains("<html"));
        assert!(html.contains("/assets/htmx.min.js"));
    }

    #[test]
    fn results_fragment_has_no_html_document_wrapper() {
        // A fragment must be swappable into an existing DOM — it must NOT be a
        // full document.
        let frag = SearchResults {
            query: "pat".into(),
            hits: search_dataset("pat"),
        }
        .render()
        .expect("renders");
        assert!(!frag.contains("<html"));
        assert!(frag.contains("Patient"));
    }
}
