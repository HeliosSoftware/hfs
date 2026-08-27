//! Read model for the CapabilityStatement page (`/ui/capability-statement`,
//! #653).
//!
//! `/metadata` composes the statement fresh on every request from live server
//! state — registered search parameters, backend capabilities, enabled
//! features — so there is nothing stored to edit and the page is genuinely
//! read-only. The fetch rides the same loopback self-call as the other
//! conformance viewers ([`crate::conformance`]); a failed fetch degrades to a
//! warning, never to fabricated capabilities.

use serde_json::Value;

/// The server-summary block: identity fields lifted off the statement root.
pub(crate) struct CapabilitySummary {
    pub description: String,
    pub implementation_url: String,
    pub fhir_version: String,
    pub status: String,
    pub kind: String,
    pub date: String,
    pub formats: Vec<String>,
}

/// One system-level interaction (`rest[0].interaction`). `transaction` is
/// advertised only when the backend supports atomicity while `batch` is
/// unconditional — the flag lets the template say so instead of rendering an
/// undifferentiated list.
pub(crate) struct SystemInteraction {
    pub code: String,
    pub conditional: bool,
}

/// One server operation (`rest[0].operation`), with its OperationDefinition
/// link when the definition points at this server.
pub(crate) struct OperationRow {
    pub name: String,
    /// A same-server `/OperationDefinition/{id}` path, when the canonical
    /// resolves locally; external canonicals render as plain text.
    pub definition_path: String,
    pub definition: String,
}

/// One per-resource row (`rest[0].resource[]`).
pub(crate) struct ResourceRow {
    pub resource_type: String,
    pub interactions: Vec<String>,
    pub search_param_count: usize,
    pub include_count: usize,
    pub revinclude_count: usize,
}

pub(crate) struct CapabilityView {
    pub summary: CapabilitySummary,
    pub interactions: Vec<SystemInteraction>,
    pub operations: Vec<OperationRow>,
    pub resources: Vec<ResourceRow>,
}

fn str_at<'a>(value: &'a Value, path: &[&str]) -> &'a str {
    let mut cur = value;
    for p in path {
        match cur.get(p) {
            Some(v) => cur = v,
            None => return "",
        }
    }
    cur.as_str().unwrap_or("")
}

fn arr<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// Projects the raw CapabilityStatement into the page's view. Defensive over
/// the JSON: absent fields render empty, never panic — the statement shape
/// varies with enabled features and FHIR version.
pub(crate) fn build_view(statement: &Value) -> CapabilityView {
    let rest = arr(statement, "rest")
        .first()
        .cloned()
        .unwrap_or(Value::Null);

    let summary = CapabilitySummary {
        description: str_at(statement, &["implementation", "description"]).to_string(),
        implementation_url: str_at(statement, &["implementation", "url"]).to_string(),
        fhir_version: str_at(statement, &["fhirVersion"]).to_string(),
        status: str_at(statement, &["status"]).to_string(),
        kind: str_at(statement, &["kind"]).to_string(),
        date: str_at(statement, &["date"]).to_string(),
        formats: arr(statement, "format")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
    };

    let interactions = arr(&rest, "interaction")
        .iter()
        .map(|i| {
            let code = str_at(i, &["code"]).to_string();
            SystemInteraction {
                // `transaction` only appears when the backend is atomic;
                // `batch` always does. Mark the conditional one.
                conditional: code == "transaction",
                code,
            }
        })
        .collect();

    let operations = arr(&rest, "operation")
        .iter()
        .map(|o| {
            let definition = str_at(o, &["definition"]).to_string();
            // A canonical of this server's own OperationDefinition gets a
            // relative, clickable path; foreign canonicals stay text.
            let definition_path = definition
                .find("/OperationDefinition/")
                .map(|i| definition[i..].to_string())
                .unwrap_or_default();
            OperationRow {
                name: str_at(o, &["name"]).to_string(),
                definition_path,
                definition,
            }
        })
        .collect();

    let resources = arr(&rest, "resource")
        .iter()
        .map(|r| ResourceRow {
            resource_type: str_at(r, &["type"]).to_string(),
            interactions: arr(r, "interaction")
                .iter()
                .map(|i| str_at(i, &["code"]).to_string())
                .collect(),
            search_param_count: arr(r, "searchParam").len(),
            include_count: arr(r, "searchInclude").len(),
            revinclude_count: arr(r, "searchRevInclude").len(),
        })
        .collect();

    CapabilityView {
        summary,
        interactions,
        operations,
        resources,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn projects_the_statement_defensively() {
        let statement = json!({
            "resourceType": "CapabilityStatement",
            "status": "active", "kind": "instance", "date": "2026-08-24",
            "fhirVersion": "4.0.1",
            "format": ["application/fhir+json"],
            "implementation": {"description": "Helios FHIR Server", "url": "http://x/"},
            "rest": [{
                "interaction": [{"code": "batch"}, {"code": "transaction"}],
                "operation": [
                    {"name": "export", "definition": "http://x/OperationDefinition/export"},
                    {"name": "viewdef", "definition": "http://sql-on-fhir.org/OperationDefinition/run"}
                ],
                "resource": [{
                    "type": "Patient",
                    "interaction": [{"code": "read"}, {"code": "search-type"}],
                    "searchParam": [{"name": "name"}, {"name": "birthdate"}],
                    "searchInclude": ["Patient:organization"]
                }]
            }]
        });
        let view = build_view(&statement);
        assert_eq!(view.summary.fhir_version, "4.0.1");
        assert_eq!(view.summary.formats.len(), 1);
        assert!(
            view.interactions
                .iter()
                .any(|i| i.code == "transaction" && i.conditional)
        );
        assert!(
            view.interactions
                .iter()
                .any(|i| i.code == "batch" && !i.conditional)
        );
        // Any /OperationDefinition/{id} canonical links relatively: the
        // statement this server emits advertises its own definitions, and
        // GET /OperationDefinition/{id} is routed.
        assert_eq!(
            view.operations[0].definition_path,
            "/OperationDefinition/export"
        );
        assert_eq!(
            view.operations[1].definition_path,
            "/OperationDefinition/run"
        );
        assert_eq!(view.resources[0].search_param_count, 2);
        assert_eq!(view.resources[0].include_count, 1);
        assert_eq!(view.resources[0].revinclude_count, 0);

        // An empty statement renders empty, never panics.
        let empty = build_view(&json!({}));
        assert!(empty.resources.is_empty());
        assert!(empty.summary.fhir_version.is_empty());
    }
}
