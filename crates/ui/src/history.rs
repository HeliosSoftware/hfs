//! History & Versions diff rendering (#236).
//!
//! The decision is recorded in `docs/history-diff-rendering.md`; this is the
//! prototype Steve asked for on PR #246. It renders the difference between two
//! versions of a resource — from `_history` — **server-side, in two layers**,
//! into an Askama template. No client-side diff library, and the essence (the
//! diff itself) is computed here, not in the browser.
//!
//! - **Semantic layer** — [`json_patch::diff`], a default-feature dependency
//!   already in the workspace, and the exact inverse of the RFC 6902 patches
//!   the FHIR `PATCH` interaction already applies. Rendered as a field-level
//!   change list (`replace /name/0/family  Smith → Smythe`).
//! - **Textual layer** — [`similar`] with word-level inline highlighting over
//!   the pretty-printed JSON of both versions. `similar` has no HTML formatter,
//!   so the changes are iterated into template rows — which is what the crate's
//!   no-embedded-markup stance wants anyway.
//!
//! `meta.versionId` / `meta.lastUpdated` change on every version, so they are
//! filtered from the semantic view by default, behind a "show metadata" toggle.

use serde_json::Value;
use similar::{ChangeTag, TextDiff};

/// One field-level change from the semantic layer.
pub struct FieldChange {
    /// `add` | `remove` | `replace` | `move` | `copy`
    pub op: &'static str,
    /// JSON Pointer to the changed element (`/name/0/family`).
    pub path: String,
    /// The value being set, pretty enough for one line. Empty for `remove`.
    pub value: String,
    /// The prior value, when the op is a `replace` (best-effort — RFC 6902
    /// `replace` does not carry the old value, so this is filled by looking it
    /// up in the previous document).
    pub was: Option<String>,
}

/// One row of the textual layer.
pub struct DiffLine {
    /// `ctx` (unchanged) | `add` | `del`
    pub tag: &'static str,
    /// The old-side line number, blank on an addition.
    pub old_ln: String,
    /// The new-side line number, blank on a deletion.
    pub new_ln: String,
    /// Word-level segments, so an intra-line edit highlights only what changed.
    pub segments: Vec<Segment>,
}

/// A run of characters within a diff line, flagged if it is the changed part.
pub struct Segment {
    pub text: String,
    /// Emphasised as the specific words that changed within the line.
    pub emphasis: bool,
}

/// The whole rendered comparison of two versions.
pub struct Diff {
    /// The field-level semantic view.
    pub changes: Vec<FieldChange>,
    /// The metadata ops that were filtered out (count, so the toggle can say
    /// how many are hidden).
    pub metadata_hidden: usize,
    /// The textual line-by-line view.
    pub lines: Vec<DiffLine>,
    /// True when the two documents are byte-identical after normalisation.
    pub identical: bool,
}

/// Renders the difference from `old` to `new`.
///
/// `show_metadata` keeps `meta.versionId` / `meta.lastUpdated` in the semantic
/// view; by default they are filtered, since they change on every version and
/// would bury the real edit.
pub fn diff(old: &Value, new: &Value, show_metadata: bool) -> Diff {
    let changes = semantic_changes(old, new, show_metadata);
    let metadata_hidden = if show_metadata {
        0
    } else {
        // How many ops we suppressed: the difference between diffing with and
        // without meta. Cheap, and it lets the toggle be honest.
        let full = json_patch::diff(old, new).0.len();
        full.saturating_sub(json_patch::diff(&strip_meta(old), &strip_meta(new)).0.len())
    };

    let old_pretty = serde_json::to_string_pretty(old).unwrap_or_default();
    let new_pretty = serde_json::to_string_pretty(new).unwrap_or_default();
    let lines = textual_lines(&old_pretty, &new_pretty);

    Diff {
        identical: changes.is_empty() && old_pretty == new_pretty,
        changes,
        metadata_hidden,
        lines,
    }
}

/// A copy of the resource without `meta`, so the noise floor
/// (`versionId`/`lastUpdated`, which move every version) does not dominate.
fn strip_meta(value: &Value) -> Value {
    let mut clone = value.clone();
    if let Some(object) = clone.as_object_mut() {
        object.remove("meta");
    }
    clone
}

/// The semantic layer: `json_patch::diff`, rendered field by field.
fn semantic_changes(old: &Value, new: &Value, show_metadata: bool) -> Vec<FieldChange> {
    let (old, new) = if show_metadata {
        (old.clone(), new.clone())
    } else {
        (strip_meta(old), strip_meta(new))
    };

    json_patch::diff(&old, &new)
        .0
        .into_iter()
        .map(|operation| render_operation(operation, &old))
        .collect()
}

/// Turns one RFC 6902 operation into a display row. For a `replace`, the old
/// value is recovered by resolving the pointer against the previous document —
/// the patch format itself does not carry it.
fn render_operation(operation: json_patch::PatchOperation, previous: &Value) -> FieldChange {
    use json_patch::PatchOperation::*;

    let one_line = |value: &Value| -> String {
        let text = serde_json::to_string(value).unwrap_or_default();
        // Keep the change list to one line per op; the textual view is where a
        // whole added object is read in full.
        if text.chars().count() > 80 {
            let mut truncated: String = text.chars().take(77).collect();
            truncated.push('…');
            truncated
        } else {
            text
        }
    };
    let plain = |value: &Value| -> String {
        match value {
            Value::String(text) => text.clone(),
            other => one_line(other),
        }
    };

    match operation {
        Add(op) => FieldChange {
            op: "add",
            path: op.path.to_string(),
            value: plain(&op.value),
            was: None,
        },
        Remove(op) => FieldChange {
            op: "remove",
            path: op.path.to_string(),
            value: String::new(),
            was: previous.pointer(op.path.as_str()).map(plain),
        },
        Replace(op) => FieldChange {
            op: "replace",
            path: op.path.to_string(),
            value: plain(&op.value),
            was: previous.pointer(op.path.as_str()).map(plain),
        },
        Move(op) => FieldChange {
            op: "move",
            path: op.path.to_string(),
            value: format!("from {}", op.from),
            was: None,
        },
        Copy(op) => FieldChange {
            op: "copy",
            path: op.path.to_string(),
            value: format!("from {}", op.from),
            was: None,
        },
        Test(op) => FieldChange {
            op: "test",
            path: op.path.to_string(),
            value: plain(&op.value),
            was: None,
        },
    }
}

/// The textual layer: a line diff with word-level intra-line highlighting.
///
/// `iter_inline_changes` gives, per changed line, the runs that actually
/// differ — so a one-character rename highlights `Smith`→`Smythe`, not the
/// whole line. `grouped_ops(3)` keeps three lines of context around each
/// change, the way a unified diff reads.
fn textual_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let text_diff = TextDiff::from_lines(old, new);
    let mut lines = Vec::new();

    for group in text_diff.grouped_ops(3) {
        for op in group {
            for change in text_diff.iter_inline_changes(&op) {
                let tag = match change.tag() {
                    ChangeTag::Equal => "ctx",
                    ChangeTag::Delete => "del",
                    ChangeTag::Insert => "add",
                };
                let segments = change
                    .iter_strings_lossy()
                    .map(|(emphasis, text)| Segment {
                        text: text.trim_end_matches('\n').to_string(),
                        emphasis: emphasis && tag != "ctx",
                    })
                    .collect();

                lines.push(DiffLine {
                    tag,
                    old_ln: change
                        .old_index()
                        .map(|i| (i + 1).to_string())
                        .unwrap_or_default(),
                    new_ln: change
                        .new_index()
                        .map(|i| (i + 1).to_string())
                        .unwrap_or_default(),
                    segments,
                });
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The exact scenario the decision doc characterizes: v4 renames a family,
    /// front-inserts a new name, and appends a telecom. The point is not to be
    /// pretty about array churn (accepted for v1) but to confirm the layers
    /// produce what the doc says they do.
    fn v3() -> Value {
        json!({
            "resourceType": "Patient", "id": "a12",
            "meta": { "versionId": "3", "lastUpdated": "2026-07-09T10:32:00Z" },
            "name": [
                { "family": "Smith", "given": ["Johanna"], "use": "maiden" },
                { "family": "Jones", "given": ["Jon"], "use": "official" }
            ],
            "telecom": [{ "system": "phone", "value": "555" }]
        })
    }
    fn v4() -> Value {
        json!({
            "resourceType": "Patient", "id": "a12",
            "meta": { "versionId": "4", "lastUpdated": "2026-07-09T12:01:00Z" },
            "name": [
                { "family": "Smythe", "given": ["Johanna"], "use": "official" },
                { "family": "Jones", "given": ["Jon"] },
                { "family": "Smith", "given": ["J"], "use": "nickname" }
            ],
            "telecom": [
                { "system": "phone", "value": "555" },
                { "system": "email", "value": "j@x.org" }
            ]
        })
    }

    #[test]
    fn metadata_is_filtered_by_default_and_counted() {
        let result = diff(&v3(), &v4(), false);
        // No versionId / lastUpdated in the field list.
        assert!(
            !result
                .changes
                .iter()
                .any(|change| change.path.contains("meta"))
        );
        // Exactly the two meta ops were hidden.
        assert_eq!(result.metadata_hidden, 2);
    }

    #[test]
    fn metadata_appears_when_asked_for() {
        let result = diff(&v3(), &v4(), true);
        assert!(
            result
                .changes
                .iter()
                .any(|change| change.path == "/meta/versionId")
        );
        assert_eq!(result.metadata_hidden, 0);
    }

    #[test]
    fn a_rename_is_a_replace_that_carries_the_old_value() {
        let result = diff(&v3(), &v4(), false);
        let family = result
            .changes
            .iter()
            .find(|change| change.path == "/name/0/family")
            .expect("the family rename is in the semantic view");
        assert_eq!(family.op, "replace");
        assert_eq!(family.value, "Smythe");
        assert_eq!(family.was.as_deref(), Some("Smith"));
    }

    #[test]
    fn a_clean_append_is_a_single_op() {
        let result = diff(&v3(), &v4(), false);
        // The appended telecom is one add, as the doc's spike showed.
        assert!(
            result
                .changes
                .iter()
                .any(|change| change.op == "add" && change.path == "/telecom/1")
        );
    }

    #[test]
    fn the_textual_layer_shows_the_rename_as_a_delete_and_an_insert() {
        let result = diff(&v3(), &v4(), false);
        assert!(result.lines.iter().any(|line| line.tag == "del"));
        assert!(result.lines.iter().any(|line| line.tag == "add"));
        // Line numbers are tracked per side.
        assert!(result.lines.iter().any(|line| !line.old_ln.is_empty()));
        assert!(result.lines.iter().any(|line| !line.new_ln.is_empty()));
    }

    #[test]
    fn word_level_highlighting_isolates_the_changed_run() {
        // A minimal one-field rename, so the highlighted run is unambiguous.
        let old = json!({ "family": "Smith" });
        let new = json!({ "family": "Smythe" });
        let result = diff(&old, &new, false);

        // Some changed line emphasises a proper substring, not the whole line.
        let emphasised: Vec<&str> = result
            .lines
            .iter()
            .filter(|line| line.tag != "ctx")
            .flat_map(|line| &line.segments)
            .filter(|segment| segment.emphasis)
            .map(|segment| segment.text.as_str())
            .collect();
        assert!(
            emphasised
                .iter()
                .any(|text| text.contains("Smy") || text.contains("th")),
            "the changed characters are highlighted, got {emphasised:?}"
        );
    }

    #[test]
    fn identical_versions_report_no_change() {
        let result = diff(&v4(), &v4(), true);
        assert!(result.identical);
        assert!(result.changes.is_empty());
    }
}
