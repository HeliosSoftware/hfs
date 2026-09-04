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

/// The starter document behind "Create New" for `code`. The SQL pane fills
/// the attachment, so the starter carries the coding, one dependency slot,
/// and an empty `application/sql` attachment to receive it.
pub(crate) fn starter_library(code: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "resourceType": "Library",
        "name": if code == "sql-view" { "new_sql_view" } else { "new_sql_query" },
        "status": STARTER_STATUS,
        "type": { "coding": [{ "system": LIBRARY_TYPES_SYSTEM, "code": code }] },
        "relatedArtifact": [
            { "type": "depends-on", "resource": "http://example.org/ViewDefinition/change-me", "label": "v" }
        ],
        "content": [ { "contentType": "application/sql" } ]
    }))
    .expect("static JSON serializes")
}

/// The starter SQL paired with [`starter_library`]'s `label`.
pub(crate) const STARTER_SQL: &str = "SELECT * FROM v";

/// One declared `Library.parameter[use=in]` entry (#837), read with the same
/// semantics `helios_sof::sqlquery::library::extract_parameters` applies:
/// only `use=in` entries carry SQL on FHIR's binding semantics, and `name`/
/// `type` are both required by the SQLQuery profile. Unlike that function —
/// which fails the whole Library on a malformed entry, appropriate for the
/// engine about to run it — this reader is forgiving: a Library the UI
/// cannot fully describe must still let every other stored subject render.
/// If the server itself rejects a malformed Library at kick-off, the job
/// fails with its own message, exactly as it does today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclaredParameter {
    /// The bare name, without its `:` SQL placeholder prefix.
    pub name: String,
    /// The FHIR type code from `Library.parameter.type` (`string`,
    /// `integer`, `date`, …).
    pub type_code: String,
    /// The declared `default[X]` value in plain-text form — `defaultString`
    /// verbatim, a number's `to_string()`, `true`/`false` for a boolean —
    /// or `None` when the entry carries no `default[X]` field, or that
    /// field's JSON shape has no plain-text representation. A parameter
    /// with a default is optional everywhere this type is consumed; one
    /// without is required.
    pub default: Option<String>,
}

/// Reads every `use=in` parameter declaration off a `sql-query` Library, in
/// document order. An entry missing `use=in`, `name`, or `type` is skipped
/// (logged at `debug`) rather than surfaced as a page-wide error — see
/// [`DeclaredParameter`]'s own docs for why.
pub(crate) fn parameters(library: &Value) -> Vec<DeclaredParameter> {
    let Some(entries) = library.get("parameter").and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|p| p.get("use").and_then(Value::as_str) == Some("in"))
        .filter_map(|p| {
            let name = p.get("name").and_then(Value::as_str);
            let type_code = p.get("type").and_then(Value::as_str);
            let (Some(name), Some(type_code)) = (name, type_code) else {
                tracing::debug!(
                    entry = ?p,
                    "skipping Library.parameter entry missing name or type"
                );
                return None;
            };
            Some(DeclaredParameter {
                name: name.to_string(),
                type_code: type_code.to_string(),
                default: default_text(p),
            })
        })
        .collect()
}

/// The plain-text form of a `default[X]` field on one `parameter` entry, per
/// [`DeclaredParameter::default`]'s own rule. Any key starting with
/// `default` and carrying at least one more character (`defaultString`,
/// `defaultValueInteger`, …) counts, forward-compatible with the same
/// tolerant match `helios_sof::sqlquery::library::read_default` applies.
fn default_text(entry: &Value) -> Option<String> {
    entry.as_object()?.iter().find_map(|(key, value)| {
        let rest = key.strip_prefix("default")?;
        if rest.is_empty() {
            return None;
        }
        match value {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    })
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
            // #839: the starter's own status never drifts from the constant
            // the title row's status chip reads on `?lib=new`.
            assert_eq!(lib["status"].as_str(), Some(STARTER_STATUS));
        }
    }

    #[test]
    fn status_is_extracted_verbatim_and_empty_when_absent() {
        let lib = library("sql-query", LIBRARY_TYPES_SYSTEM);
        assert_eq!(extract_status(&lib), "active");
        assert_eq!(extract_status(&json!({"resourceType": "Library"})), "");
    }

    // -----------------------------------------------------------------
    // parameters() (#837)
    // -----------------------------------------------------------------

    #[test]
    fn parameters_ignores_out_entries_and_entries_missing_a_type() {
        let lib = json!({
            "resourceType": "Library",
            "parameter": [
                {"name": "ward", "use": "in", "type": "string"},
                {"name": "result", "use": "out", "type": "string"},
                {"name": "untyped", "use": "in"},
            ],
        });
        let params = parameters(&lib);
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "ward");
        assert_eq!(params[0].type_code, "string");
        assert_eq!(params[0].default, None);
    }

    #[test]
    fn parameters_ignores_entries_missing_a_name() {
        let lib = json!({
            "resourceType": "Library",
            "parameter": [{"use": "in", "type": "string"}],
        });
        assert!(parameters(&lib).is_empty());
    }

    #[test]
    fn parameters_reads_a_numeric_default_as_its_text_form() {
        let lib = json!({
            "resourceType": "Library",
            "parameter": [
                {"name": "days", "use": "in", "type": "integer", "defaultInteger": 30},
            ],
        });
        let params = parameters(&lib);
        assert_eq!(params[0].default.as_deref(), Some("30"));
    }

    #[test]
    fn parameters_reads_string_and_boolean_defaults_and_falls_back_to_none_otherwise() {
        let lib = json!({
            "resourceType": "Library",
            "parameter": [
                {"name": "a", "use": "in", "type": "string", "defaultString": "west"},
                {"name": "b", "use": "in", "type": "boolean", "defaultBoolean": true},
                {"name": "c", "use": "in", "type": "string", "defaultCodeableConcept": {"text": "x"}},
            ],
        });
        let params = parameters(&lib);
        assert_eq!(params[0].default.as_deref(), Some("west"));
        assert_eq!(params[1].default.as_deref(), Some("true"));
        assert_eq!(params[2].default, None);
    }

    #[test]
    fn parameters_matches_parse_sqlquery_library_on_a_valid_library() {
        let sql = base64::engine::general_purpose::STANDARD.encode("SELECT * FROM v");
        let lib = json!({
            "resourceType": "Library",
            "type": {"coding": [{"system": LIBRARY_TYPES_SYSTEM, "code": "sql-query"}]},
            "content": [{"contentType": "application/sql", "data": sql}],
            "parameter": [
                {"name": "ward", "use": "in", "type": "string"},
                {"name": "days", "use": "in", "type": "integer", "defaultInteger": 30},
                {"name": "ignored", "use": "out", "type": "string"},
            ],
        });
        let ours: Vec<(String, String, bool)> = parameters(&lib)
            .into_iter()
            .map(|p| (p.name, p.type_code, p.default.is_some()))
            .collect();
        let theirs: Vec<(String, String, bool)> =
            helios_sof::sqlquery::parse_sqlquery_library(&lib)
                .expect("valid Library parses")
                .parameters
                .into_iter()
                .map(|p| (p.name, p.type_code, p.has_default))
                .collect();
        assert_eq!(ours, theirs);
    }
}
