//! View models for the SQL Queries and SQL Views workspaces (#649).
//!
//! Both pages edit `Library` resources — a SQLQuery carries
//! `type.coding` `sql-query`, a SQLView `sql-view` (both under the SQL on
//! FHIR `LibraryTypesCodes` system, current or pre-ballot) — and run them as
//! `$sql-run` subjects. The SQL itself travels base64-encoded in a
//! `content[]` attachment; these helpers decode it for its own editor pane
//! and re-embed it on save, so nobody edits base64 by hand.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value;

const LIBRARY_TYPES_SYSTEM: &str =
    "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes";
const LEGACY_LIBRARY_TYPES_SYSTEM: &str = "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes";

/// Whether this Library's `type.coding` carries `code` under either published
/// `LibraryTypesCodes` system.
pub(crate) fn has_library_code(library: &Value, code: &str) -> bool {
    library
        .get("type")
        .and_then(|t| t.get("coding"))
        .and_then(Value::as_array)
        .is_some_and(|codings| {
            codings.iter().any(|c| {
                c.get("code").and_then(Value::as_str) == Some(code)
                    && matches!(
                        c.get("system").and_then(Value::as_str),
                        Some(LIBRARY_TYPES_SYSTEM) | Some(LEGACY_LIBRARY_TYPES_SYSTEM)
                    )
            })
        })
}

/// One rail entry: a stored Library of the page's kind.
pub(crate) struct LibSummary {
    pub id: String,
    pub name: String,
    pub status: String,
}

/// Summarizes fetched Libraries carrying `code` into rail entries, name-sorted.
pub(crate) fn summarize(resources: &[Value], code: &str) -> Vec<LibSummary> {
    let mut entries: Vec<LibSummary> = resources
        .iter()
        .filter(|l| has_library_code(l, code))
        .filter_map(|l| {
            let id = l.get("id")?.as_str()?.to_string();
            let name = l
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(&id)
                .to_string();
            let status = extract_status(l);
            Some(LibSummary { id, name, status })
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    entries
}

/// The resource's own `status`, verbatim, empty when absent — the same
/// extraction each rail entry above already applies, and the source of the
/// editor-first title row's own status chip (#839), which shows this text
/// as-is even when it names no FHIR publication-status code Helios
/// recognizes.
pub(crate) fn extract_status(library: &Value) -> String {
    library
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The decoded SQL of the first `application/sql` content attachment, empty
/// when there is none (or its data is not valid base64 UTF-8).
pub(crate) fn extract_sql(library: &Value) -> String {
    library
        .get("content")
        .and_then(Value::as_array)
        .and_then(|atts| {
            atts.iter().find(|a| {
                a.get("contentType")
                    .and_then(Value::as_str)
                    .is_some_and(|ct| ct.starts_with("application/sql"))
            })
        })
        .and_then(|a| a.get("data").and_then(Value::as_str))
        .and_then(|data| BASE64.decode(data).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
}

/// Embeds `sql` as the base64 `data` of the Library's first `application/sql`
/// attachment, appending one when none exists. Other attachments are left
/// alone.
pub(crate) fn embed_sql(library: &mut Value, sql: &str) {
    let encoded = Value::String(BASE64.encode(sql));
    let Some(map) = library.as_object_mut() else {
        return;
    };
    let content = map
        .entry("content")
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(atts) = content.as_array_mut() else {
        return;
    };
    let existing = atts.iter_mut().find(|a| {
        a.get("contentType")
            .and_then(Value::as_str)
            .is_some_and(|ct| ct.starts_with("application/sql"))
    });
    match existing {
        Some(att) => {
            if let Some(att) = att.as_object_mut() {
                att.insert("data".to_string(), encoded);
            }
        }
        None => atts.push(serde_json::json!({
            "contentType": "application/sql",
            "data": encoded,
        })),
    }
}

/// The starter document's fixed `status` — its own constant so
/// [`starter_library`]'s literal and the "Create New" title row's status
/// chip (#839, [`crate::status_tag_class`]) can never drift apart.
pub(crate) const STARTER_STATUS: &str = "draft";

/// The starter document behind "Create New" for `code`, as a parsed value —
/// the Details panel's own first paint (`crate::render_lib_details_pane`)
/// builds directly off this rather than re-parsing [`starter_library`]'s
/// string. Carries no `content` (#840): the SQL pane owns the
/// `application/sql` attachment — [`STARTER_SQL`] is its own starter text —
/// and [`embed_sql`] adds the attachment on save/run, exactly as it would
/// for any other document that has none yet.
pub(crate) fn starter_library_value(code: &str) -> Value {
    serde_json::json!({
        "resourceType": "Library",
        "name": if code == "sql-view" { "new_sql_view" } else { "new_sql_query" },
        "status": STARTER_STATUS,
        "type": { "coding": [{ "system": LIBRARY_TYPES_SYSTEM, "code": code }] },
        "relatedArtifact": [
            { "type": "depends-on", "resource": "http://example.org/ViewDefinition/change-me", "label": "v" }
        ]
    })
}

/// [`starter_library_value`], pretty-printed — the SQL pane's own starter
/// text (`STARTER_SQL`'s counterpart in the JSON pane) before the Details
/// panel existed to build the guided form from the parsed value directly.
pub(crate) fn starter_library(code: &str) -> String {
    serde_json::to_string_pretty(&starter_library_value(code)).expect("static JSON serializes")
}

/// The starter SQL paired with [`starter_library`]'s `label`.
pub(crate) const STARTER_SQL: &str = "SELECT * FROM v";

/// Returns a copy of `library` without any `content[]` attachment whose
/// `contentType` starts with `application/sql` (#840) — the document
/// Details edits, since the SQL attachment lives in its own card. `content`
/// is dropped entirely when stripping it empties the array; a `library`
/// whose `content` is missing or not an array comes back unchanged. Other
/// attachments (CQL, plain text, …) keep their order and content.
///
/// Paired with [`extract_sql`]/[`embed_sql`] at save/run time: for a Library
/// with a single `application/sql` attachment,
/// `embed_sql(strip_sql_attachment(lib), extract_sql(lib))` reconstructs
/// `lib` (see the invariant test below) — stripping and re-embedding is a
/// round trip except that a re-embedded attachment always lands last, which
/// only matters when other attachments preceded it.
///
/// The Details panel's own document, both on the page's first paint
/// (`crate::shape_lib`, `crate::render_lib_details_pane`) and in the
/// `POST /ui/sql/queries`/`/ui/sql/views` Save error re-render.
pub(crate) fn strip_sql_attachment(library: &Value) -> Value {
    let mut out = library.clone();
    let Some(map) = out.as_object_mut() else {
        return out;
    };
    let Some(atts) = map.get("content").and_then(Value::as_array) else {
        return out;
    };
    let kept: Vec<Value> = atts
        .iter()
        .filter(|attachment| {
            !attachment
                .get("contentType")
                .and_then(Value::as_str)
                .is_some_and(|ct| ct.starts_with("application/sql"))
        })
        .cloned()
        .collect();
    if kept.is_empty() {
        map.remove("content");
    } else {
        map.insert("content".to_string(), Value::Array(kept));
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn library(code: &str, system: &str) -> Value {
        json!({
            "resourceType": "Library", "id": "l1", "name": "q",
            "status": "active",
            "type": {"coding": [{"system": system, "code": code}]},
        })
    }

    #[test]
    fn kind_matching_accepts_both_published_systems_and_nothing_else() {
        assert!(has_library_code(
            &library("sql-query", LIBRARY_TYPES_SYSTEM),
            "sql-query"
        ));
        assert!(has_library_code(
            &library("sql-query", LEGACY_LIBRARY_TYPES_SYSTEM),
            "sql-query"
        ));
        assert!(!has_library_code(
            &library("sql-query", "http://elsewhere"),
            "sql-query"
        ));
        assert!(!has_library_code(
            &library("sql-view", LIBRARY_TYPES_SYSTEM),
            "sql-query"
        ));
    }

    #[test]
    fn summaries_keep_only_the_requested_kind() {
        let libs = vec![
            library("sql-query", LIBRARY_TYPES_SYSTEM),
            library("sql-view", LIBRARY_TYPES_SYSTEM),
        ];
        assert_eq!(summarize(&libs, "sql-query").len(), 1);
        assert_eq!(summarize(&libs, "sql-view").len(), 1);
    }

    #[test]
    fn sql_roundtrips_through_the_attachment() {
        let mut lib = library("sql-query", LIBRARY_TYPES_SYSTEM);
        assert_eq!(extract_sql(&lib), "");
        embed_sql(&mut lib, "SELECT 1");
        assert_eq!(extract_sql(&lib), "SELECT 1");
        // Re-embedding replaces, not appends.
        embed_sql(&mut lib, "SELECT 2");
        assert_eq!(extract_sql(&lib), "SELECT 2");
        assert_eq!(lib["content"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn starters_parse_and_carry_their_coding() {
        for code in ["sql-query", "sql-view"] {
            let lib: Value = serde_json::from_str(&starter_library(code)).unwrap();
            assert!(has_library_code(&lib, code));
            assert_eq!(extract_sql(&lib), "");
            // #840: no `content` at all — the SQL card owns that branch and
            // embeds it on save/run, so the starter carries nothing for
            // Details (#840's guided form) to show or hide in the meantime.
            assert!(lib.get("content").is_none());
            // #839: the starter's own status never drifts from the constant
            // the title row's status chip reads on `?lib=new`.
            assert_eq!(lib["status"].as_str(), Some(STARTER_STATUS));
        }
    }

    #[test]
    fn strip_sql_attachment_drops_the_key_when_sql_was_the_only_attachment() {
        let mut lib = library("sql-query", LIBRARY_TYPES_SYSTEM);
        embed_sql(&mut lib, "SELECT 1");
        let stripped = strip_sql_attachment(&lib);
        assert!(stripped.get("content").is_none());
        // Nothing else in the document moved.
        assert_eq!(stripped["name"], lib["name"]);
    }

    #[test]
    fn strip_sql_attachment_keeps_other_attachments_in_order() {
        let mut lib = library("sql-query", LIBRARY_TYPES_SYSTEM);
        lib["content"] = json!([
            { "contentType": "text/cql", "data": "cql-data" },
            { "contentType": "application/sql", "data": "sql-data" },
            { "contentType": "text/plain", "data": "plain-data" },
        ]);
        let stripped = strip_sql_attachment(&lib);
        let content = stripped["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["contentType"], "text/cql");
        assert_eq!(content[1]["contentType"], "text/plain");
    }

    #[test]
    fn strip_sql_attachment_passes_through_a_missing_or_non_array_content() {
        let no_content = library("sql-query", LIBRARY_TYPES_SYSTEM);
        assert_eq!(strip_sql_attachment(&no_content), no_content);

        let mut non_array_content = library("sql-query", LIBRARY_TYPES_SYSTEM);
        non_array_content["content"] = json!("not-an-array");
        assert_eq!(strip_sql_attachment(&non_array_content), non_array_content);
    }

    /// #840's own round-trip invariant: for a Library with a single SQL
    /// attachment, stripping it out and re-embedding the SQL it carried
    /// reconstructs the original document — the attachment only moves when
    /// other attachments already surrounded it (untested here, since there
    /// are none), never when it was alone.
    #[test]
    fn strip_then_embed_reconstructs_a_library_with_only_a_sql_attachment() {
        let mut lib = library("sql-query", LIBRARY_TYPES_SYSTEM);
        embed_sql(&mut lib, "SELECT 1 FROM t");

        let mut reconstructed = strip_sql_attachment(&lib);
        embed_sql(&mut reconstructed, &extract_sql(&lib));

        assert_eq!(reconstructed, lib);
    }

    #[test]
    fn status_is_extracted_verbatim_and_empty_when_absent() {
        let lib = library("sql-query", LIBRARY_TYPES_SYSTEM);
        assert_eq!(extract_status(&lib), "active");
        assert_eq!(extract_status(&json!({"resourceType": "Library"})), "");
    }
}
