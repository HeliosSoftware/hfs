//! View models for the SQL on FHIR View Definitions workspace (#649).
//!
//! The page lists the tenant's stored `ViewDefinition`s in a filter rail,
//! shows the selected one as editable JSON, and previews its output through
//! `$sql-run`. Everything here is a pure function over resource JSON; the
//! fetching and running live behind [`crate::ConformanceSource`].

use serde_json::Value;

/// One rail entry: a stored ViewDefinition summarized for the picker.
pub(crate) struct VdSummary {
    pub id: String,
    pub name: String,
    /// The FHIR resource type the view flattens (`ViewDefinition.resource`).
    pub resource: String,
}

/// Summarizes fetched ViewDefinitions into rail entries, sorted by name.
/// Resources without an `id` are unreachable by the page's `?vd=` selection
/// and are dropped rather than rendered as dead rows.
pub(crate) fn summarize(resources: &[Value]) -> Vec<VdSummary> {
    let mut entries: Vec<VdSummary> = resources
        .iter()
        .filter_map(|vd| {
            let id = vd.get("id")?.as_str()?.to_string();
            let name = vd
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(&id)
                .to_string();
            let resource = vd
                .get("resource")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(VdSummary { id, name, resource })
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    entries
}

/// The view's output column names, in declaration order: a depth-first walk of
/// `select[].column[].name` through nested `select` and `unionAll` branches.
/// This is the authoritative column order — `_format=json` rows are objects,
/// whose key order is not something to depend on.
pub(crate) fn column_names(view_definition: &Value) -> Vec<String> {
    let mut names = Vec::new();
    collect_columns(view_definition.get("select"), &mut names);
    names
}

fn collect_columns(select: Option<&Value>, names: &mut Vec<String>) {
    let Some(selects) = select.and_then(Value::as_array) else {
        return;
    };
    for s in selects {
        if let Some(columns) = s.get("column").and_then(Value::as_array) {
            for c in columns {
                if let Some(name) = c.get("name").and_then(Value::as_str) {
                    if !names.iter().any(|n| n == name) {
                        names.push(name.to_string());
                    }
                }
            }
        }
        collect_columns(s.get("select"), names);
        collect_columns(s.get("unionAll"), names);
    }
}

/// The `$sql-run` preview, shaped for a `data-table`.
pub(crate) struct RunTable {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Tables the JSON rows under the view's declared columns. A column missing
/// from a row renders empty; a row key the view does not declare (or, with no
/// declared columns at all, every key of the first row) extends the header, so
/// nothing the server returned is silently dropped.
pub(crate) fn build_table(view_definition: &Value, rows: &[Value]) -> RunTable {
    let mut columns = column_names(view_definition);
    for row in rows {
        if let Some(map) = row.as_object() {
            for key in map.keys() {
                if !columns.iter().any(|c| c == key) {
                    columns.push(key.clone());
                }
            }
        }
    }
    let rows = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|c| cell_text(row.get(c)))
                .collect::<Vec<_>>()
        })
        .collect();
    RunTable { columns, rows }
}

/// A cell's display text: strings verbatim, `null`/absent empty, anything
/// else (numbers, booleans, arrays from collection columns) as compact JSON.
fn cell_text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// The starter document behind "Create New" — the smallest runnable view.
pub(crate) fn starter_view_definition() -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "resourceType": "ViewDefinition",
        "name": "new_view",
        "status": "draft",
        "resource": "Patient",
        "select": [
            { "column": [ { "name": "id", "path": "getResourceKey()" } ] }
        ]
    }))
    .expect("static JSON serializes")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn summaries_sort_by_name_and_skip_idless_resources() {
        let vds = vec![
            json!({"id": "b", "name": "blood_pressure", "resource": "Observation"}),
            json!({"name": "unsaved_no_id", "resource": "Patient"}),
            json!({"id": "a", "name": "active_patients", "resource": "Patient"}),
            json!({"id": "x", "resource": "Patient"}),
        ];
        let rail = summarize(&vds);
        let names: Vec<&str> = rail.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["active_patients", "blood_pressure", "x"]);
        assert_eq!(rail[0].resource, "Patient");
    }

    #[test]
    fn column_order_follows_the_select_walk_not_the_row_objects() {
        let vd = json!({
            "resource": "Patient",
            "select": [
                { "column": [ {"name": "id", "path": "id"}, {"name": "name", "path": "name.family"} ] },
                { "unionAll": [ { "column": [ {"name": "zip", "path": "address.postalCode"} ] } ] }
            ]
        });
        assert_eq!(column_names(&vd), ["id", "name", "zip"]);

        let rows = vec![json!({"zip": "10001", "name": "Doe", "id": "p1", "extra": 7})];
        let table = build_table(&vd, &rows);
        // Declared order first, then whatever the server added.
        assert_eq!(table.columns, ["id", "name", "zip", "extra"]);
        assert_eq!(table.rows[0], ["p1", "Doe", "10001", "7"]);
    }

    #[test]
    fn cells_render_strings_verbatim_and_null_empty() {
        let vd = json!({"select": [{ "column": [{"name": "a", "path": "x"}, {"name": "b", "path": "y"}] }]});
        let rows = vec![json!({"a": null, "b": [1, 2]})];
        let table = build_table(&vd, &rows);
        assert_eq!(table.rows[0], ["", "[1,2]"]);
    }

    #[test]
    fn the_starter_document_parses_and_declares_a_column() {
        let vd: Value = serde_json::from_str(&starter_view_definition()).unwrap();
        assert_eq!(vd["resourceType"], "ViewDefinition");
        assert_eq!(column_names(&vd), ["id"]);
    }
}
