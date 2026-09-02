//! Markup tests for the shared JSON view.
//!
//! The tokenizer's own tests live beside it in `src/json_view.rs` and assert
//! the line model. These assert the two properties the *hosts* depend on and
//! neither host can check for itself: that the fragment is safe to splice with
//! `|safe`, and that its bytes still match what `crates/ui` emitted when the
//! partial was included from its own template root (#803).

use helios_ui_chrome::{ChromeLabels, json_view};
use serde_json::json;

/// Locale-neutral labels: every key renders as `[key]`, so assertions read as
/// structure rather than as English.
struct Stub;

impl ChromeLabels for Stub {
    fn lang(&self) -> String {
        "en".to_string()
    }

    fn t(&self, key: &str) -> String {
        format!("[{key}]")
    }
}

fn render(value: serde_json::Value, id: &str, paths: bool) -> String {
    json_view::render(&Stub, &json_view::lines(&value), id, paths)
        .expect("the shared partial must render")
}

/// `crates/ui` splices this fragment into pages with `|safe`, which is only
/// sound because askama escapes every value on the way in. A key or string
/// carrying markup must not reach the page as markup.
#[test]
fn values_are_html_escaped_before_the_safe_filter() {
    let out = render(json!({ "<script>": "</span><img onerror=x>" }), "", false);

    assert!(
        !out.contains("<script>") && !out.contains("<img onerror"),
        "raw markup from the document reached the fragment: {out}",
    );
    // askama's HTML escaper emits numeric entities, so the key survives as
    // `"<script>"` spelled `&#34;&#60;script&#62;&#34;`.
    assert!(
        out.contains("&#60;script&#62;"),
        "the key must still render: {out}",
    );
}

/// The fragment replaced an `{% include %}` in `editor-body.html` and
/// `capability-json-full.html`, both of which supply their own newline after
/// it. Askama drops a template file's final newline whether it is rendered
/// directly or included, so this render must end at `</div>` — a trailing
/// newline here would grow a blank line on every editor and
/// CapabilityStatement page against the pre-extraction output.
#[test]
fn renders_without_a_trailing_newline() {
    let out = render(json!({ "a": 1 }), "", false);

    assert!(
        out.ends_with("</div>"),
        "render must end at </div>, exactly as the include did, got: {:?}",
        &out[out.len().saturating_sub(24)..],
    );
}

/// The id is what `capability-json.js` and `editor-sync.js` address the view
/// by; a host with nothing to find must not emit an empty `id=""` on the
/// container. (Per-line `data-fold-id` is unrelated and always present.)
#[test]
fn the_id_attribute_is_emitted_only_when_asked_for() {
    assert!(
        render(json!({ "a": 1 }), "json-view", false)
            .starts_with(r#"<div class="json-view" id="json-view">"#)
    );
    assert!(render(json!({ "a": 1 }), "", false).starts_with(r#"<div class="json-view">"#));
}

/// `data-jpath` is the Resource Editor's cross-highlight hook. Every other
/// host passes `paths: false` and should not pay for the attributes.
#[test]
fn path_attributes_are_opt_in() {
    let with = render(json!({ "name": [{ "family": "Duck" }] }), "", true);
    let without = render(json!({ "name": [{ "family": "Duck" }] }), "", false);

    assert!(with.contains(r#"data-jpath="name.0.family""#));
    assert!(!without.contains("data-jpath"));
}

/// Every foldable container gets the button json-view.js binds to, carrying
/// the host's translated label.
#[test]
fn foldable_lines_carry_a_labelled_toggle() {
    let out = render(json!({ "name": [1, 2, 3] }), "", false);

    assert!(out.contains(r#"class="json-line json-line--foldable""#));
    assert!(out.contains(r#"aria-label="[json-view-toggle-fold]""#));
    assert!(
        out.contains(r#"<span class="json-line__summary">[ 3 ]</span>"#),
        "the collapsed-state summary must render for the array",
    );
}
