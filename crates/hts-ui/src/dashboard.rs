//! Dashboard page + refresh fragment (design doc §7.1).
//!
//! Two entry points:
//! - [`dashboard_page`] renders the full dashboard shell (topbar, sidebar,
//!   cards region). It always fetches once so the initial paint is not blank,
//!   and swaps the cards region on every subsequent htmx refresh.
//! - [`dashboard_cards_fragment`] returns just the cards partial, wired to
//!   `hx-trigger="every 15s"` from the page shell.
//!
//! The upstream fetch is a two-call fan-out (`/health` and
//! `/metadata?mode=terminology`) rendered together so the operator sees a
//! coherent picture even when one leg fails: a red status card + a healthy
//! capabilities card is better than a blank page.

use askama::Template;
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use axum_htmx::HxRequest;
use std::sync::Arc;

use crate::i18n::{I18n, RequestLocale};
use crate::upstream::{
    UpstreamError, UpstreamHealth, UpstreamTerminologyCapabilities,
};
use crate::{Chrome, HtsUiState};

/// One shared card model, so the page and the refresh fragment render from
/// exactly the same shape. Never `Option<Option<_>>`: each leg either fetches
/// or degrades, and the outcome carries the reason.
#[derive(Clone, Debug)]
pub struct DashboardCards {
    pub health: Result<UpstreamHealth, UpstreamError>,
    pub capabilities: Result<UpstreamTerminologyCapabilities, UpstreamError>,
    /// Advertised bundled data footprint in bytes, computed from
    /// `HTS_BOOTSTRAP_DIR` at mount time; `None` when no bootstrap directory
    /// was configured (the docker image or a bare `hts run` from a source
    /// tree). Renders as an em-dash in that arm.
    pub bundled_data_bytes: Option<u64>,
}

impl DashboardCards {
    async fn fetch(state: &HtsUiState) -> Self {
        let (health, capabilities) = tokio::join!(
            state.upstream.health(),
            state.upstream.terminology_capabilities(),
        );
        Self {
            health,
            capabilities,
            bundled_data_bytes: state.bundled_data_bytes,
        }
    }

    /// Any-leg failure surfaces as `Some(reason)` for the degraded banner.
    /// The dashboard renders the banner **and** the successful cards below
    /// it — a partial degrade is still informative.
    pub fn degraded_reason(&self) -> Option<&'static str> {
        match (&self.health, &self.capabilities) {
            (Err(e), _) | (_, Err(e)) => Some(e.degraded_reason()),
            _ => None,
        }
    }

    /// Loaded systems count, per `TerminologyCapabilities.codeSystem[]`.
    /// Returns `None` when the capabilities fetch failed — the tile then
    /// renders an em-dash instead of a zero, which would be a lie.
    pub fn loaded_system_count(&self) -> Option<usize> {
        self.capabilities
            .as_ref()
            .ok()
            .map(|c| c.loaded_system_count())
    }

    /// Bundled data footprint in mebibytes, rounded down. Rendered as prose
    /// so the Fluent placeable can localise the unit.
    pub fn bundled_data_mib(&self) -> Option<u64> {
        self.bundled_data_bytes.map(|b| b / (1024 * 1024))
    }
}

#[derive(Template)]
#[template(path = "pages/dashboard.html")]
struct DashboardPage<'a> {
    chrome: Chrome<'a>,
    cards: DashboardCards,
}

#[derive(Template)]
#[template(path = "partials/hts-dashboard-cards.html")]
struct DashboardCardsPartial<'a> {
    chrome: Chrome<'a>,
    cards: DashboardCards,
}

pub(crate) fn routes() -> Router<Arc<HtsUiState>> {
    Router::new()
        .route("/hts", get(dashboard_page))
        // Trailing-slash canonicalization: axum matches paths exactly, so
        // `/ui/hts/` would 404 without this. Redirect to the canonical form
        // used by every internal link in the UI. `Redirect::permanent` emits
        // 308 (preserves method + body); safe here since GET is the only
        // dashboard verb.
        .route("/hts/", get(|| async { Redirect::permanent("/ui/hts") }))
        .route("/hts/dashboard/cards", get(dashboard_cards_fragment))
}

async fn dashboard_page(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    // On a hard navigation we always fetch once so first paint is
    // meaningful. On an htmx request that targeted `/hts` (unusual — the
    // refresh fragment endpoint below is the normal path) we still return
    // the full page: htmx will swap the requested element out of it.
    let cards = DashboardCards::fetch(&state).await;
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "dashboard",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    let page = DashboardPage { chrome, cards };
    let _ = is_htmx; // reserved: fragment-only paths handled by the endpoint below.
    crate::render_page(page.render()).into_response()
}

async fn dashboard_cards_fragment(
    State(state): State<Arc<HtsUiState>>,
    // Extracted only so `AutoVaryLayer` (axum-htmx) sees the handler
    // participate in htmx negotiation and appends `Vary: HX-Request`. The
    // fragment body is identical in both htmx and hard-navigation arms.
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
) -> Response {
    let cards = DashboardCards::fetch(&state).await;
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "dashboard",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    let partial = DashboardCardsPartial { chrome, cards };
    match partial.render() {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!(?err, "hts-ui dashboard cards fragment render failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "hts-ui: fragment render error",
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upstream::UpstreamClient;

    fn state_with(upstream: UpstreamClient, bundled: Option<u64>) -> Arc<HtsUiState> {
        Arc::new(HtsUiState {
            fhir_version: "R4",
            version: "0.0.0-test",
            upstream,
            bundled_data_bytes: bundled,
        })
    }

    /// The test client points at a URL that will refuse to connect (port 1
    /// on loopback is closed by convention). Both legs must fail and the
    /// dashboard must still render.
    fn state_with_unreachable_upstream() -> Arc<HtsUiState> {
        let upstream = UpstreamClient::new("http://127.0.0.1:1").expect("client");
        state_with(upstream, None)
    }

    #[tokio::test]
    async fn cards_render_the_degraded_banner_when_upstream_is_unreachable() {
        let state = state_with_unreachable_upstream();
        let cards = DashboardCards::fetch(&state).await;
        assert!(cards.health.is_err());
        assert!(cards.capabilities.is_err());
        assert!(cards.degraded_reason().is_some());
    }
}
