//! The workbench "Raw request and response" fold (#803).
//!
//! Every operation workbench — CodeSystem `$lookup` / `$validate-code` /
//! `$subsumes`, ValueSet `$expand`, ConceptMap `$translate` — ends in the same
//! disclosure: the URL that was called, the `Parameters` resource that was
//! POSTed, and the body that came back. It exists so an operator can reproduce
//! the call by hand, which is why it lives in one struct and one partial
//! (`templates/partials/hts-raw-fold.html`) rather than three copies.
//!
//! Three things were wrong with it before #803, and this module is where two
//! of them are fixed:
//!
//! * The response was kept only on the success path, so a failed lookup —
//!   exactly when the payload is most wanted — showed the URL and nothing
//!   else. [`RawExchange`] now rides the error path, and
//!   [`RawFold::from_exchange`] reads it.
//! * The request body was never shown at all, despite the heading promising
//!   it.
//!
//! The third — the fold not looking like a control — is in the partial.

use crate::i18n::I18n;
use crate::upstream::RawExchange;
use helios_ui_chrome::json_view;

/// The render budget for one payload, matching the bound HFS puts on its
/// shared JSON preview (`crates/ui/src/lib.rs::render_json_view`).
///
/// An operation response is small, so this passes comfortably; a very large
/// `$expand` falls back to the plain `<pre>` that used to be the only
/// rendering, which is a degradation, not a failure. Keeping the budget
/// generous is deliberate: #798 is what a budget set too low to ever pass
/// looks like.
const MAX_LINES: usize = 4_000;
const MAX_ESTIMATED_HTML_BYTES: usize = 2 * 1024 * 1024;

/// The payload behind a workbench's raw fold.
#[derive(Clone, Debug, Default)]
pub struct RawFold {
    pub request_url: String,
    /// The POSTed `Parameters`, pretty-printed. Empty for a call the UI
    /// rejected before sending anything (a missing required code, say).
    pub request_body: String,
    /// The response, pretty-printed when it parsed as JSON and verbatim when
    /// it did not. Empty when there was no response to read — a connect
    /// failure or a timeout.
    pub response_body: String,
    /// The two payloads rendered into the shared highlighted JSON view, or
    /// empty when the payload exceeded the budget above and the partial should
    /// fall back to a `<pre>`. Filled by [`RawFold::highlight`] at the
    /// response funnel, because the viewer's fold buttons carry a translated
    /// label and the upstream layer has no business knowing the request
    /// locale.
    pub request_json: String,
    pub response_json: String,
    /// When set, the response body uses incremental JSON loading (#898) via
    /// this fragment endpoint URL instead of rendering inline. The URL should
    /// include all parameters needed to re-issue the operation.
    pub response_fragment_url: Option<String>,
    /// The raw URL for the no-JS fallback link (`?raw=1`). When set, the fold
    /// shows an "Open plain JSON" link that bypasses JavaScript.
    pub raw_url: Option<String>,
}

impl RawFold {
    /// The fold for a call that reached the upstream and came back.
    pub fn new(request_url: &str, request_body: &str, response_body: &str) -> Self {
        Self {
            request_url: request_url.to_owned(),
            request_body: request_body.to_owned(),
            response_body: response_body.to_owned(),
            ..Self::default()
        }
    }

    /// The fold for a call that failed, from the exchange the proxy kept.
    pub fn from_exchange(exchange: &RawExchange) -> Self {
        Self::new(
            &exchange.request_url,
            &exchange.request_body,
            &exchange.response_body,
        )
    }

    /// Whether there is anything at all to disclose. The partial renders
    /// nothing when there is not — a form the UI rejected before sending
    /// produces no exchange, and an empty fold is worse than no fold.
    pub fn is_empty(&self) -> bool {
        self.request_url.is_empty() && self.request_body.is_empty() && self.response_body.is_empty()
    }

    /// Renders both payloads into the shared JSON view.
    ///
    /// Non-JSON or over-budget text leaves the corresponding field empty; the
    /// partial then shows the raw string in a `<pre>`, which is what the fold
    /// did for every payload before #803.
    pub fn highlight(&mut self, i18n: &I18n) {
        self.request_json = highlight_one(i18n, &self.request_body);
        self.response_json = highlight_one(i18n, &self.response_body);
    }
}

/// Renders a JSON payload into the shared highlighted JSON view.
///
/// Returns the highlighted HTML, or an empty string when the payload is empty,
/// not valid JSON, or exceeds the render budget. The empty return signals that
/// the caller's template should fall back to a bare `<pre>` block.
///
/// This is the shared entry point for all raw-payload highlighting in HTS-UI:
/// the workbench fold (`RawFold::highlight`), the Concept Explorer panels, and
/// the import status page.
pub fn highlight_json(i18n: &I18n, payload: &str) -> String {
    if payload.is_empty() {
        return String::new();
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
        return String::new();
    };
    let options = json_view::RenderOptions {
        // Nothing here cross-highlights against a guided form, so the
        // per-line `data-jpath` attributes would be bytes on the wire that
        // no script reads.
        include_paths: false,
        budget: Some(json_view::RenderBudget {
            max_lines: MAX_LINES,
            max_estimated_html_bytes: MAX_ESTIMATED_HTML_BYTES,
        }),
    };
    let Ok(lines) = json_view::try_lines(&value, options) else {
        return String::new();
    };
    json_view::render(i18n, &lines, "", false).unwrap_or_else(|error| {
        tracing::error!(%error, "hts-ui raw fold json view render failed");
        String::new()
    })
}

fn highlight_one(i18n: &I18n, payload: &str) -> String {
    highlight_json(i18n, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i18n() -> I18n {
        I18n::new(crate::i18n::RequestLocale::default())
    }

    #[test]
    fn a_json_payload_is_highlighted() {
        let mut fold = RawFold::new(
            "http://hts/CodeSystem/$lookup",
            r#"{"resourceType":"Parameters"}"#,
            r#"{"resourceType":"Parameters","parameter":[{"name":"display"}]}"#,
        );
        fold.highlight(&i18n());

        assert!(fold.request_json.contains("json-view"));
        assert!(fold.response_json.contains(r#"class="jt--key""#));
    }

    #[test]
    fn a_non_json_payload_falls_back_to_the_pre_block() {
        let mut fold = RawFold::new("http://hts/CodeSystem/$lookup", "", "<html>502</html>");
        fold.highlight(&i18n());

        assert!(fold.response_json.is_empty(), "no highlighting to offer");
        assert!(
            !fold.response_body.is_empty(),
            "the payload itself must survive for the <pre> fallback",
        );
    }

    #[test]
    fn an_over_budget_payload_falls_back_to_the_pre_block() {
        let huge = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": (0..MAX_LINES + 10)
                .map(|i| serde_json::json!({ "name": format!("p{i}") }))
                .collect::<Vec<_>>(),
        });
        let mut fold = RawFold::new("http://hts/ValueSet/$expand", "", &huge.to_string());
        fold.highlight(&i18n());

        assert!(fold.response_json.is_empty());
    }

    #[test]
    fn a_form_rejected_before_sending_has_no_fold() {
        assert!(RawFold::default().is_empty());
        assert!(!RawFold::new("http://hts/CodeSystem/$lookup", "", "").is_empty());
    }
}
