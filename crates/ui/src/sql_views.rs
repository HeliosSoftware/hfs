//! View models for the SQL on FHIR View Definitions workspace (#649).
//!
//! The page lists the tenant's stored `ViewDefinition`s in a filter rail,
//! shows the selected one as editable JSON, and previews its output through
//! `$sql-run`. Everything here is a pure function over resource JSON or over
//! the page's query state; the fetching, searching, and running live behind
//! [`crate::ConformanceSource`].
//!
//! The rail itself is filtered, sorted, and paginated server-side (#741) —
//! this module only shapes what the server already narrowed down to one page,
//! plus the plain-link pagination controls at its foot.

use serde_json::Value;

/// The rail's page size: how many ViewDefinitions `search_page` returns per
/// request, and the stride between `?page=` values (#741).
pub(crate) const PAGE_SIZE: usize = 50;

/// One rail entry: a stored ViewDefinition summarized for the picker.
pub(crate) struct VdSummary {
    pub id: String,
    pub name: String,
    /// The FHIR resource type the view flattens (`ViewDefinition.resource`).
    pub resource: String,
    /// `ViewDefinition.status` (`draft` | `active` | `retired` | `unknown`),
    /// empty when absent. Added for the SQL Export builder's Status column
    /// (#834) — the rail itself does not display it.
    pub status: String,
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
            let status = vd
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(VdSummary {
                id,
                name,
                resource,
                status,
            })
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    entries
}

/// The `search_page` params for the rail's current filter (#741): `_sort=name`
/// always (the order the rail has always shown), plus `name:contains=<filter>`
/// only when the search box is non-empty — an empty modifier value would
/// search for the empty string rather than mean "no filter".
///
/// Deliberately narrower than the old in-memory filter it replaces, which
/// also matched the resource type column (`ViewDefinition.resource`): that
/// match was never a documented part of #649 (it fell out of filtering in
/// memory over both fields) and has no standard FHIR search expression —
/// `resource` is a `token` param, and `:contains` is a `string` modifier the
/// server rejects on token params. Dropping it is a deliberate, spec-driven
/// narrowing (#741), not a regression.
pub(crate) fn rail_search_params(filter: &str) -> Vec<(String, String)> {
    let mut params = vec![("_sort".to_string(), "name".to_string())];
    if !filter.is_empty() {
        params.push(("name:contains".to_string(), filter.to_string()));
    }
    params
}

/// Builds an `/ui/sql/view-definitions` href for a rail pagination link,
/// preserving the rail's current `filter` and the URL's current `vd`
/// selection alongside the target `page` (#741). `page` 1 is the implicit
/// default and is omitted from the URL, matching the bare route the rail's
/// search form itself submits.
pub(crate) fn page_href(filter: &str, vd: Option<&str>, page: usize) -> String {
    let mut params: Vec<String> = Vec::new();
    if !filter.is_empty() {
        params.push(format!("filter={}", urlencode(filter)));
    }
    if let Some(id) = vd {
        params.push(format!("vd={}", urlencode(id)));
    }
    if page > 1 {
        params.push(format!("page={page}"));
    }
    if params.is_empty() {
        "/ui/sql/view-definitions".to_string()
    } else {
        format!("/ui/sql/view-definitions?{}", params.join("&"))
    }
}

/// `application/x-www-form-urlencoded` percent-encoding — the same encoding
/// a browser's own `<form method="get">` submission produces, so a
/// pagination link round-trips through [`axum::extract::Query`] identically
/// to a search-box submit.
fn urlencode(value: &str) -> String {
    form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// The `$sql-run` preview's row cap, shared by the server-side `?saved=1`
/// render and all three `/run` fragment endpoints (#752, #839): a
/// navigation or keystroke preview, never an export.
pub(crate) const RUN_LIMIT: usize = 50;

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

/// The 1-based line number out of a `$sql-run` failure message, when the
/// server's message carries sqlparser's own `Line: N` marker (as in
/// `… at Line: 2, Column: 8`, the shape a SQL *parse* failure takes — a
/// SQLite *execution* error carries no such marker). `None` for a message
/// with no `Line: ` marker at all, or where the text right after it is not a
/// plain, in-range non-negative integer (#839).
///
/// Shared by the View Definitions, SQL Queries, and SQL Views playgrounds so
/// the editor's own line-tinting (#839) can locate a parse error the same
/// way regardless of which page ran it.
pub(crate) fn extract_error_line(message: &str) -> Option<u32> {
    let after_marker = message.split_once("Line: ")?.1;
    let digits: String = after_marker
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
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

    #[test]
    fn rail_search_params_always_sorts_and_only_filters_when_non_empty() {
        assert_eq!(
            rail_search_params(""),
            vec![("_sort".to_string(), "name".to_string())],
        );
        assert_eq!(
            rail_search_params("Blood"),
            vec![
                ("_sort".to_string(), "name".to_string()),
                ("name:contains".to_string(), "Blood".to_string()),
            ],
        );
    }

    #[test]
    fn error_line_extraction_reads_the_sqlparser_marker() {
        assert_eq!(
            extract_error_line("sql parser error: … at Line: 2, Column: 8"),
            Some(2)
        );
        // No marker at all — a SQLite execution error, not a parse error.
        assert_eq!(extract_error_line("no such table: v"), None);
        // A marker whose number is out of `u32` range is still "not a
        // plain number" as far as the tinted line goes.
        assert_eq!(
            extract_error_line("… at Line: 99999999999999999999, Column: 1"),
            None
        );
        // A marker with nothing numeric right after it.
        assert_eq!(extract_error_line("… at Line: , Column: 1"), None);
    }

    #[test]
    fn page_href_omits_empty_parts_and_page_one() {
        assert_eq!(page_href("", None, 1), "/ui/sql/view-definitions");
        assert_eq!(page_href("", None, 2), "/ui/sql/view-definitions?page=2");
        assert_eq!(
            page_href("blood pressure", Some("vd-1"), 3),
            "/ui/sql/view-definitions?filter=blood+pressure&vd=vd-1&page=3"
        );
    }
}
