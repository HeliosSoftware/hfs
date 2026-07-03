//! Parse a SQLQuery Library (FHIR Library profile) into the parts the engine
//! needs: the SQL string, parameter declarations, and depends-on ViewDefinitions.
//!
//! See <https://build.fhir.org/ig/FHIR/sql-on-fhir-v2/StructureDefinition-SQLQuery.html>

use base64::Engine;
use serde_json::Value;

use super::SqlQueryError;

/// `Library.type.coding.system` value the SQLQuery profile fixes.
pub const LIBRARY_TYPE_SYSTEM: &str = "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes";
/// `Library.type.coding.code` value the SQLQuery profile fixes.
pub const LIBRARY_TYPE_CODE: &str = "sql-query";

/// SQL dialect the engine speaks. Used to pick the most specific
/// `application/sql;dialect=…` content attachment.
pub const ENGINE_DIALECT: &str = "sqlite";

/// One row's worth of metadata from `Library.parameter`. `use=in` only.
#[derive(Debug, Clone)]
pub struct LibraryParameter {
    pub name: String,
    /// FHIR `code` element — `string`, `integer`, `integer64`, `boolean`,
    /// `decimal`, `date`, `dateTime`, `instant`, `time`, etc. Required by the
    /// SQLQuery profile (1..1) — missing here is a malformed Library.
    pub type_code: String,
    /// Was the parameter declared with a `default[X]` value? If so we treat
    /// it as optional. The SQLQuery profile does not document defaults; this
    /// is a forward-compatible read for any `default*` field on the entry.
    pub has_default: bool,
    /// Default value as raw JSON, if any.
    pub default_value: Option<Value>,
}

/// A `depends-on` entry in the Library's `relatedArtifact`. The SQLQuery
/// profile requires `relatedArtifact.resource` to be a canonical URL — inline
/// ViewDefinition resources are **not** part of the profile and are rejected.
#[derive(Debug, Clone)]
pub struct DependsOnView {
    /// Table alias the SQL references. Constrained to `^[A-Za-z][A-Za-z0-9_]*$`
    /// by the profile (`sql-name` invariant).
    pub label: String,
    /// Canonical URL pointing to a ViewDefinition the server resolves.
    pub url: String,
}

/// A parsed SQLQuery Library.
#[derive(Debug, Clone)]
pub struct SqlQueryLibrary {
    pub sql: String,
    pub parameters: Vec<LibraryParameter>,
    pub depends_on: Vec<DependsOnView>,
}

/// Parses a Library resource JSON into a `SqlQueryLibrary`.
///
/// Enforces the SQLQuery profile constraints:
/// - `resourceType == "Library"`
/// - `Library.type.coding[*]` contains `{system: LibraryTypesCodes, code: sql-query}`
/// - At least one `content` attachment with `contentType` starting with `application/sql`
/// - All `relatedArtifact[type="depends-on"]` entries have a canonical URL `resource`
///   and a `label` matching `^[A-Za-z][A-Za-z0-9_]*$`
/// - All `Library.parameter[use="in"]` entries declare a `type`
pub fn parse_sqlquery_library(library_json: &Value) -> Result<SqlQueryLibrary, SqlQueryError> {
    if library_json.get("resourceType").and_then(|v| v.as_str()) != Some("Library") {
        return Err(SqlQueryError::MalformedLibrary(
            "resourceType must be 'Library'".to_string(),
        ));
    }

    validate_library_type(library_json)?;

    let sql = extract_sql(library_json)?;
    let parameters = extract_parameters(library_json)?;
    let depends_on = extract_depends_on(library_json)?;

    Ok(SqlQueryLibrary {
        sql,
        parameters,
        depends_on,
    })
}

/// Spec: `Library.type` SHALL carry `LibraryTypesCodes#sql-query`.
fn validate_library_type(library_json: &Value) -> Result<(), SqlQueryError> {
    let codings = library_json
        .get("type")
        .and_then(|t| t.get("coding"))
        .and_then(|c| c.as_array())
        .ok_or_else(|| {
            SqlQueryError::MalformedLibrary(
                "Library.type.coding[] is required and must include LibraryTypesCodes#sql-query"
                    .to_string(),
            )
        })?;
    let ok = codings.iter().any(|c| {
        let code = c.get("code").and_then(|v| v.as_str());
        let system = c.get("system").and_then(|v| v.as_str());
        code == Some(LIBRARY_TYPE_CODE) && (system.is_none() || system == Some(LIBRARY_TYPE_SYSTEM))
    });
    if !ok {
        return Err(SqlQueryError::MalformedLibrary(format!(
            "Library.type must include coding {{system: {LIBRARY_TYPE_SYSTEM}, code: {LIBRARY_TYPE_CODE}}}"
        )));
    }
    Ok(())
}

/// Spec dialect-selection: prefer an `application/sql;dialect=<ENGINE_DIALECT>`
/// attachment, fall back to bare `application/sql`, then any other
/// `application/sql*` variant.
fn extract_sql(library_json: &Value) -> Result<String, SqlQueryError> {
    let content = library_json
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or(SqlQueryError::MissingSql)?;

    // Bucket attachments by specificity so we can pick deterministically.
    let mut dialect_match: Option<&Value> = None;
    let mut bare: Option<&Value> = None;
    let mut other: Option<&Value> = None;

    for entry in content {
        let ct = entry
            .get("contentType")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !ct.starts_with("application/sql") {
            // Profile constraint `sql-must-be-sql-expressions` says every
            // content.contentType SHALL start with `application/sql`. We're
            // tolerant for now (skip), since the entire content array isn't
            // required to be pure SQL in practice; but we don't pick this one.
            continue;
        }
        if let Some(rest) = ct.strip_prefix("application/sql").map(str::trim_start) {
            if rest.is_empty() {
                if bare.is_none() {
                    bare = Some(entry);
                }
            } else if parses_dialect(rest, ENGINE_DIALECT) {
                dialect_match = Some(entry);
            } else if other.is_none() {
                other = Some(entry);
            }
        }
    }

    let chosen = dialect_match
        .or(bare)
        .or(other)
        .ok_or(SqlQueryError::MissingSql)?;
    read_sql_from_attachment(chosen)
}

/// Returns `true` if a contentType suffix like `;dialect=sqlite` matches
/// `dialect`. Handles whitespace and quoted values.
fn parses_dialect(suffix: &str, dialect: &str) -> bool {
    // `;dialect=sqlite`, `; dialect=SQLite`, `;dialect="sqlite"`
    let suffix = suffix.trim_start_matches(';').trim();
    for part in suffix.split(';') {
        let kv = part.trim();
        if let Some(value) = kv.strip_prefix("dialect=") {
            let v = value.trim_matches('"').trim();
            if v.eq_ignore_ascii_case(dialect) {
                return true;
            }
        }
    }
    false
}

fn read_sql_from_attachment(entry: &Value) -> Result<String, SqlQueryError> {
    // Preferred per profile: base64 `data`.
    if let Some(data_b64) = entry.get("data").and_then(|v| v.as_str()) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data_b64)
            .map_err(|e| {
                SqlQueryError::MalformedLibrary(format!(
                    "Library.content[].data is not valid base64: {e}"
                ))
            })?;
        return String::from_utf8(bytes).map_err(|e| {
            SqlQueryError::MalformedLibrary(format!("Library.content[].data is not UTF-8: {e}"))
        });
    }
    // Fallback: sql-text extension carrying plain-text SQL.
    if let Some(extensions) = entry.get("extension").and_then(|v| v.as_array()) {
        for ext in extensions {
            let url = ext.get("url").and_then(|v| v.as_str()).unwrap_or("");
            // Accept either the official URL or a relative form.
            let is_sql_text = url.ends_with("/sql-text") || url == "sql-text";
            if is_sql_text {
                if let Some(s) = ext.get("valueString").and_then(|v| v.as_str()) {
                    return Ok(s.to_string());
                }
            }
        }
    }
    Err(SqlQueryError::MissingSql)
}

fn extract_parameters(library_json: &Value) -> Result<Vec<LibraryParameter>, SqlQueryError> {
    let Some(arr) = library_json.get("parameter").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for p in arr {
        if p.get("use").and_then(|v| v.as_str()) != Some("in") {
            // SQLQuery profile only defines `use=in` parameters. `out` (and
            // any other value) has no defined semantics for $sqlquery-run,
            // so silently skip rather than reject — keeps us forward-
            // compatible with future profile additions.
            continue;
        }
        let name = p.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
            SqlQueryError::MalformedLibrary(
                "Library.parameter[*].name is required for use=in entries".to_string(),
            )
        })?;
        let type_code = p.get("type").and_then(|v| v.as_str()).ok_or_else(|| {
            SqlQueryError::MalformedLibrary(format!(
                "Library.parameter[name='{name}'].type is required (profile cardinality 1..1)"
            ))
        })?;
        let (has_default, default_value) = read_default(p);
        out.push(LibraryParameter {
            name: name.to_string(),
            type_code: type_code.to_string(),
            has_default,
            default_value,
        });
    }
    Ok(out)
}

fn read_default(entry: &Value) -> (bool, Option<Value>) {
    if let Some(obj) = entry.as_object() {
        for (k, v) in obj {
            if let Some(rest) = k.strip_prefix("default") {
                if !rest.is_empty() {
                    return (true, Some(v.clone()));
                }
            }
        }
    }
    (false, None)
}

fn extract_depends_on(library_json: &Value) -> Result<Vec<DependsOnView>, SqlQueryError> {
    let Some(rels) = library_json
        .get("relatedArtifact")
        .and_then(|v| v.as_array())
    else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    let mut seen_labels = std::collections::HashSet::new();
    for entry in rels {
        if entry.get("type").and_then(|v| v.as_str()) != Some("depends-on") {
            continue;
        }
        let label = entry
            .get("label")
            .and_then(|v| v.as_str())
            .ok_or(SqlQueryError::MissingDependsOnLabel)?;
        if !is_valid_sql_label(label) {
            return Err(SqlQueryError::MalformedLibrary(format!(
                "relatedArtifact.label '{label}' violates the sql-name constraint \
                 (^[A-Za-z][A-Za-z0-9_]*$)"
            )));
        }
        if !seen_labels.insert(label.to_string()) {
            return Err(SqlQueryError::MalformedLibrary(format!(
                "duplicate depends-on label '{label}'"
            )));
        }
        // Profile pins `relatedArtifact.resource` to canonical([Resource]).
        let url = entry
            .get("resource")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SqlQueryError::MalformedLibrary(format!(
                    "relatedArtifact label='{label}' must carry a canonical URL in 'resource'; \
                     inline ViewDefinition resources are not part of the SQLQuery profile"
                ))
            })?;
        if url.is_empty() {
            return Err(SqlQueryError::MalformedLibrary(format!(
                "relatedArtifact label='{label}' has an empty 'resource' canonical URL"
            )));
        }
        out.push(DependsOnView {
            label: label.to_string(),
            url: url.to_string(),
        });
    }
    Ok(out)
}

/// Spec invariant `sql-name`: `^[A-Za-z][A-Za-z0-9_]*$`.
pub fn is_valid_sql_label(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use serde_json::json;

    fn library_skeleton(sql: &str) -> Value {
        let data = STANDARD.encode(sql.as_bytes());
        json!({
            "resourceType": "Library",
            "type": {"coding": [{"system": LIBRARY_TYPE_SYSTEM, "code": LIBRARY_TYPE_CODE}]},
            "content": [{ "contentType": "application/sql", "data": data }]
        })
    }

    #[test]
    fn parses_minimal_library() {
        let lib = library_skeleton("SELECT 1");
        let parsed = parse_sqlquery_library(&lib).unwrap();
        assert_eq!(parsed.sql, "SELECT 1");
        assert!(parsed.parameters.is_empty());
        assert!(parsed.depends_on.is_empty());
    }

    #[test]
    fn parses_sql_text_extension() {
        let mut lib = library_skeleton("ignored");
        lib["content"] = json!([{
            "contentType": "application/sql",
            "extension": [{
                "url": "https://sql-on-fhir.org/ig/StructureDefinition/sql-text",
                "valueString": "SELECT 2"
            }]
        }]);
        let parsed = parse_sqlquery_library(&lib).unwrap();
        assert_eq!(parsed.sql, "SELECT 2");
    }

    #[test]
    fn picks_engine_dialect_over_default() {
        let lib_sqlite = STANDARD.encode("SELECT sqlite_version()");
        let lib_default = STANDARD.encode("SELECT 'default'");
        let lib_pg = STANDARD.encode("SELECT pg_version()");
        let mut lib = library_skeleton("placeholder");
        lib["content"] = json!([
            { "contentType": "application/sql;dialect=postgresql", "data": lib_pg },
            { "contentType": "application/sql", "data": lib_default },
            { "contentType": "application/sql;dialect=sqlite", "data": lib_sqlite },
        ]);
        let parsed = parse_sqlquery_library(&lib).unwrap();
        assert_eq!(parsed.sql, "SELECT sqlite_version()");
    }

    #[test]
    fn falls_back_to_bare_when_no_dialect_match() {
        let lib_default = STANDARD.encode("SELECT 'default'");
        let lib_pg = STANDARD.encode("SELECT pg_version()");
        let mut lib = library_skeleton("placeholder");
        lib["content"] = json!([
            { "contentType": "application/sql;dialect=postgresql", "data": lib_pg },
            { "contentType": "application/sql", "data": lib_default },
        ]);
        let parsed = parse_sqlquery_library(&lib).unwrap();
        assert_eq!(parsed.sql, "SELECT 'default'");
    }

    #[test]
    fn rejects_non_library() {
        let err = parse_sqlquery_library(&json!({"resourceType": "Bundle"})).unwrap_err();
        assert!(matches!(err, SqlQueryError::MalformedLibrary(_)));
    }

    #[test]
    fn rejects_library_without_sql_query_type() {
        let mut lib = library_skeleton("SELECT 1");
        lib["type"] = json!({"coding": [{"code": "logic-library"}]});
        let err = parse_sqlquery_library(&lib).unwrap_err();
        assert!(matches!(err, SqlQueryError::MalformedLibrary(_)));
    }

    #[test]
    fn rejects_library_without_type() {
        let mut lib = library_skeleton("SELECT 1");
        lib.as_object_mut().unwrap().remove("type");
        let err = parse_sqlquery_library(&lib).unwrap_err();
        assert!(matches!(err, SqlQueryError::MalformedLibrary(_)));
    }

    #[test]
    fn rejects_no_sql() {
        let mut lib = library_skeleton("ignored");
        lib.as_object_mut().unwrap().remove("content");
        let err = parse_sqlquery_library(&lib).unwrap_err();
        assert!(matches!(err, SqlQueryError::MissingSql));
    }

    #[test]
    fn parses_parameters_and_depends_on() {
        let mut lib = library_skeleton("SELECT * FROM t");
        lib["parameter"] = json!([
            {"name": "p1", "use": "in", "type": "integer"},
            {"name": "p2", "use": "out", "type": "string"} // skipped silently
        ]);
        lib["relatedArtifact"] = json!([
            {"type": "depends-on", "label": "t", "resource": "http://example.org/VD"},
            {"type": "documentation", "label": "ignored"}
        ]);
        let parsed = parse_sqlquery_library(&lib).unwrap();
        assert_eq!(parsed.parameters.len(), 1);
        assert_eq!(parsed.parameters[0].name, "p1");
        assert_eq!(parsed.parameters[0].type_code, "integer");
        assert_eq!(parsed.depends_on.len(), 1);
        assert_eq!(parsed.depends_on[0].label, "t");
        assert_eq!(parsed.depends_on[0].url, "http://example.org/VD");
    }

    #[test]
    fn rejects_parameter_without_type() {
        let mut lib = library_skeleton("SELECT 1");
        lib["parameter"] = json!([{"name": "p1", "use": "in"}]);
        let err = parse_sqlquery_library(&lib).unwrap_err();
        assert!(matches!(err, SqlQueryError::MalformedLibrary(_)));
    }

    #[test]
    fn rejects_depends_on_without_label() {
        let mut lib = library_skeleton("SELECT 1");
        lib["relatedArtifact"] = json!([
            {"type": "depends-on", "resource": "http://example.org/VD"}
        ]);
        let err = parse_sqlquery_library(&lib).unwrap_err();
        assert!(matches!(err, SqlQueryError::MissingDependsOnLabel));
    }

    #[test]
    fn rejects_label_violating_sql_name_invariant() {
        let mut lib = library_skeleton("SELECT 1");
        lib["relatedArtifact"] = json!([
            {"type": "depends-on", "label": "1bad", "resource": "http://example.org/VD"}
        ]);
        let err = parse_sqlquery_library(&lib).unwrap_err();
        assert!(matches!(err, SqlQueryError::MalformedLibrary(_)));
    }

    #[test]
    fn rejects_duplicate_label() {
        let mut lib = library_skeleton("SELECT 1");
        lib["relatedArtifact"] = json!([
            {"type": "depends-on", "label": "t", "resource": "http://example.org/A"},
            {"type": "depends-on", "label": "t", "resource": "http://example.org/B"}
        ]);
        let err = parse_sqlquery_library(&lib).unwrap_err();
        assert!(matches!(err, SqlQueryError::MalformedLibrary(_)));
    }

    #[test]
    fn rejects_inline_view_definition() {
        let mut lib = library_skeleton("SELECT 1");
        lib["relatedArtifact"] = json!([
            {"type": "depends-on", "label": "t", "resource": {"resourceType": "ViewDefinition"}}
        ]);
        let err = parse_sqlquery_library(&lib).unwrap_err();
        assert!(matches!(err, SqlQueryError::MalformedLibrary(_)));
    }

    #[test]
    fn label_invariant_helper() {
        assert!(is_valid_sql_label("abc"));
        assert!(is_valid_sql_label("A1_b"));
        assert!(!is_valid_sql_label(""));
        assert!(!is_valid_sql_label("1abc"));
        assert!(!is_valid_sql_label("_abc"));
        assert!(!is_valid_sql_label("a-b"));
        assert!(!is_valid_sql_label("a b"));
        assert!(!is_valid_sql_label("a\"b"));
    }
}
