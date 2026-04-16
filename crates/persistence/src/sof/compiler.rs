//! ViewDefinition → SQL compiler (SQLite and PostgreSQL dialects).
//!
//! Translates a raw ViewDefinition JSON object into a parameterised SQL
//! `SELECT` statement that queries the `resources` table directly.
//!
//! ## Supported patterns
//!
//! | Pattern | Description |
//! |---------|-------------|
//! | Flat columns | `select: [{column: [...]}]` — all paths are simple root-level JSON paths |
//! | Single `forEach` | One select clause has `forEach: "path"`, rest are root-level |
//! | `forEachOrNull` | Like `forEach` but uses `LEFT JOIN` so resources without the array still appear |
//!
//! ## Unsupported → `SofError::Uncompilable`
//!
//! | Pattern | Reason |
//! |---------|--------|
//! | `unionAll` | Would require UNION SQL which changes row semantics |
//! | Multiple `forEach` at same level | Cross-product semantics too complex |
//! | Nested `select` within `forEach` | Not implemented yet |
//! | `where` clause | FHIRPath filter compilation deferred to later phases |
//! | Complex column path (function calls, operators) | No FHIRPath → SQL translation |
//!
//! ## SQLite SQL shape
//!
//! ```sql
//! SELECT json_extract(r.data,'$.id') AS "id", ... FROM resources r
//! WHERE r.tenant_id=?1 AND r.resource_type=?2 AND r.is_deleted=0
//! ORDER BY r.last_updated, r.id
//! ```
//!
//! ## PostgreSQL SQL shape
//!
//! ```sql
//! SELECT r.data->>'id' AS "id", ... FROM resources r
//! WHERE r.tenant_id=$1 AND r.resource_type=$2 AND r.is_deleted=false
//! ORDER BY r.last_updated, r.id
//! ```

use serde_json::Value;

use crate::core::sof_runner::SofError;

/// SQL dialect to target during compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlDialect {
    /// SQLite: `json_extract`, `json_each`, positional `?1`/`?2` params.
    Sqlite,
    /// PostgreSQL: JSONB operators (`->>`/ `#>>`), `jsonb_array_elements`, `$1`/`$2` params.
    Postgres,
}

/// Output of a successful ViewDefinition compilation.
#[derive(Debug, Clone)]
pub struct CompiledQuery {
    /// Parameterised SQL.
    ///
    /// - SQLite: `?1 = tenant_id`, `?2 = resource_type`
    /// - PostgreSQL: `$1 = tenant_id`, `$2 = resource_type`
    pub sql: String,
    /// Column names in the order they appear in the SELECT list.
    pub columns: Vec<String>,
}

/// Compiles a raw ViewDefinition JSON value into a [`CompiledQuery`] for SQLite.
///
/// Shorthand for `compile_view_definition_dialect(view_json, SqlDialect::Sqlite)`.
pub fn compile_view_definition(view_json: &Value) -> Result<CompiledQuery, SofError> {
    compile_view_definition_dialect(view_json, SqlDialect::Sqlite)
}

/// Compiles a raw ViewDefinition JSON value into a [`CompiledQuery`] for the given dialect.
///
/// # Errors
///
/// Returns [`SofError::Uncompilable`] for any unsupported construct.
/// Returns [`SofError::InvalidViewDefinition`] if required fields are missing.
pub fn compile_view_definition_dialect(
    view_json: &Value,
    dialect: SqlDialect,
) -> Result<CompiledQuery, SofError> {
    // Extract resource type
    let resource_type = view_json
        .get("resource")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            SofError::InvalidViewDefinition("ViewDefinition.resource is required".to_string())
        })?;

    // Compile optional top-level where clauses (G7)
    let where_conditions: Vec<String> = {
        let mut conds = Vec::new();
        if let Some(wheres) = view_json.get("where").and_then(|v| v.as_array()) {
            for w in wheres {
                if let Some(path) = w.get("path").and_then(|v| v.as_str()) {
                    conds.push(compile_where_expr(path, dialect)?);
                }
            }
        }
        conds
    };

    let selects = view_json
        .get("select")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            SofError::InvalidViewDefinition(
                "ViewDefinition.select must be a non-null array".to_string(),
            )
        })?;

    if selects.is_empty() {
        return Err(SofError::InvalidViewDefinition(
            "ViewDefinition.select must have at least one clause".to_string(),
        ));
    }

    // Walk select clauses and classify
    let plan = plan_select_clauses(selects, dialect)?;

    build_sql(resource_type, &plan, &where_conditions, dialect)
}

// ============================================================================
// Planning phase
// ============================================================================

/// A single compiled column: name + SQL expression.
#[derive(Debug, Clone)]
struct CompiledColumn {
    name: String,
    /// SQL expression referencing either `r.data` (root) or `fe.value` (forEach alias).
    expr: String,
}

/// Classification of the select plan after walking all select clauses.
#[derive(Debug)]
struct SelectPlan {
    /// Root-level columns (from selects without forEach).
    root_columns: Vec<CompiledColumn>,
    /// Optional forEach join: (json_path, is_left_join, columns_from_iteration_scope)
    for_each: Option<ForEachPlan>,
    /// unionAll branches — when present, each branch compiles to a sub-SELECT
    /// and the whole query is assembled as `UNION ALL`.
    union_branches: Vec<SelectPlan>,
}

#[derive(Debug)]
struct ForEachPlan {
    json_path: String,
    is_left_join: bool,
    columns: Vec<CompiledColumn>,
}

fn plan_select_clauses(selects: &[Value], dialect: SqlDialect) -> Result<SelectPlan, SofError> {
    let mut root_columns: Vec<CompiledColumn> = Vec::new();
    let mut for_each_plan: Option<ForEachPlan> = None;
    let mut union_branches: Vec<SelectPlan> = Vec::new();

    for clause in selects {
        // Handle unionAll (G8)
        if let Some(branches) = clause
            .get("unionAll")
            .and_then(|v| v.as_array())
            .filter(|a| !a.is_empty())
        {
            for branch in branches {
                // Each branch is treated as a single-clause select
                let branch_plan = plan_single_clause(branch, dialect)?;
                union_branches.push(branch_plan);
            }
            continue;
        }

        // Regular clause (column + optional forEach)
        let branch = plan_single_clause(clause, dialect)?;
        root_columns.extend(branch.root_columns);
        if let Some(fe) = branch.for_each {
            if for_each_plan.is_some() {
                return Err(SofError::Uncompilable {
                    reason: "multiple forEach clauses at the same level are not supported by \
                             the in-DB runner (would produce a cross join)"
                        .to_string(),
                });
            }
            for_each_plan = Some(fe);
        }
    }

    if root_columns.is_empty() && for_each_plan.is_none() && union_branches.is_empty() {
        return Err(SofError::InvalidViewDefinition(
            "no columns found in select clauses".to_string(),
        ));
    }

    Ok(SelectPlan {
        root_columns,
        for_each: for_each_plan,
        union_branches,
    })
}

/// Plans a single select clause (no unionAll handling at this level).
fn plan_single_clause(clause: &Value, dialect: SqlDialect) -> Result<SelectPlan, SofError> {
    // Reject nested selects inside forEach for now
    let has_nested_select = clause
        .get("select")
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);

    let for_each_expr = clause
        .get("forEach")
        .and_then(|v| v.as_str())
        .map(String::from);
    let for_each_or_null_expr = clause
        .get("forEachOrNull")
        .and_then(|v| v.as_str())
        .map(String::from);
    let iteration_expr = for_each_expr.or(for_each_or_null_expr.clone());
    let is_left_join = for_each_or_null_expr.is_some();

    let (root_columns, for_each) = if let Some(iter_path) = &iteration_expr {
        if has_nested_select {
            return Err(SofError::Uncompilable {
                reason: "nested select within forEach is not yet supported by the \
                         in-DB runner"
                    .to_string(),
            });
        }
        // Validate path
        simple_path_to_json(iter_path)?;
        let cols = extract_columns(clause, ColumnContext::ForEach, dialect)?;
        (
            vec![],
            Some(ForEachPlan {
                json_path: iter_path.clone(),
                is_left_join,
                columns: cols,
            }),
        )
    } else {
        if has_nested_select {
            return Err(SofError::Uncompilable {
                reason: "nested select at root level is not yet supported by the in-DB runner"
                    .to_string(),
            });
        }
        (extract_columns(clause, ColumnContext::Root, dialect)?, None)
    };

    Ok(SelectPlan {
        root_columns,
        for_each,
        union_branches: vec![],
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ColumnContext {
    Root,
    ForEach,
}

fn extract_columns(
    clause: &Value,
    ctx: ColumnContext,
    dialect: SqlDialect,
) -> Result<Vec<CompiledColumn>, SofError> {
    let columns = match clause.get("column").and_then(|v| v.as_array()) {
        Some(cols) if !cols.is_empty() => cols,
        _ => return Ok(vec![]),
    };

    let mut result = Vec::new();
    for col in columns {
        let path = col.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            SofError::InvalidViewDefinition("column.path is required".to_string())
        })?;

        let name = col.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
            SofError::InvalidViewDefinition("column.name is required".to_string())
        })?;

        let expr = match dialect {
            SqlDialect::Sqlite => {
                let json_path = simple_path_to_json(path)?;
                let src = match ctx {
                    ColumnContext::Root => "r.data",
                    ColumnContext::ForEach => "fe.value",
                };
                format!("json_extract({src}, '{json_path}')")
            }
            SqlDialect::Postgres => {
                let src = match ctx {
                    ColumnContext::Root => "r.data",
                    ColumnContext::ForEach => "fe.value",
                };
                path_to_pg_expr(path, src)?
            }
        };

        result.push(CompiledColumn {
            name: name.to_string(),
            expr,
        });
    }
    Ok(result)
}

/// Translates a simple FHIRPath to a PostgreSQL JSONB expression.
///
/// - `id` → `r.data->>'id'`
/// - `name.family` → `r.data#>>'{name,family}'`
/// - `name[0].family` → `r.data#>>'{name,0,family}'`
fn path_to_pg_expr(path: &str, src: &str) -> Result<String, SofError> {
    // Validate the path using the same rules as the SQLite path validator
    let trimmed = path.trim();

    if trimmed.is_empty() {
        return Err(SofError::Uncompilable {
            reason: "empty column path".to_string(),
        });
    }
    if trimmed.starts_with('\'') || trimmed.starts_with('"') {
        return Err(SofError::Uncompilable {
            reason: format!("literal string '{trimmed}' cannot be compiled to SQL"),
        });
    }
    if trimmed.starts_with('%') {
        return Err(SofError::Uncompilable {
            reason: format!("FHIRPath variable '{trimmed}' cannot be compiled to SQL"),
        });
    }
    if trimmed.contains('(') || trimmed.contains(')') {
        return Err(SofError::Uncompilable {
            reason: format!("FHIRPath function call '{trimmed}' cannot be compiled to SQL"),
        });
    }
    if trimmed.contains('=')
        || trimmed.contains('!')
        || trimmed.contains('>')
        || trimmed.contains('<')
        || trimmed.contains(' ')
    {
        return Err(SofError::Uncompilable {
            reason: format!(
                "FHIRPath expression '{trimmed}' contains operators; cannot compile to SQL"
            ),
        });
    }

    // Build PostgreSQL path segments from `a.b[0].c` → `{a,b,0,c}`
    // First expand `x[n]` → `x,n`
    let expanded = expand_bracket_indices(trimmed)?;
    let segments: Vec<&str> = expanded.split('.').collect();

    if segments.len() == 1 {
        // Simple single-key: `src->>'key'`
        Ok(format!("{src}->>'{}'", segments[0]))
    } else {
        // Multi-key path: `src#>>'{a,b,c}'`
        let pg_path = segments.join(",");
        Ok(format!("{src}#>>'{{{pg_path}}}'"))
    }
}

/// Expands `name[0]` segments to `name.0` for later splitting on `.`.
fn expand_bracket_indices(path: &str) -> Result<String, SofError> {
    let mut result = String::new();
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' {
            // Collect digits until `]`
            let mut idx = String::new();
            for ic in chars.by_ref() {
                if ic == ']' {
                    break;
                }
                if ic.is_ascii_digit() {
                    idx.push(ic);
                } else {
                    return Err(SofError::Uncompilable {
                        reason: format!("non-integer bracket index in path '{path}'"),
                    });
                }
            }
            result.push('.');
            result.push_str(&idx);
        } else {
            result.push(c);
        }
    }
    Ok(result)
}

// ============================================================================
// Path translation: FHIRPath subset → SQLite JSON path
// ============================================================================

/// Translates a simple FHIRPath identifier/dot-path into a SQLite JSON path.
///
/// Supported input:
/// - Single identifier: `id` → `$.id`
/// - Dot-navigation: `name.family` → `$.name.family`
/// - Indexed access: `name[0].family` → `$.name[0].family`
///
/// Rejected (→ `Uncompilable`):
/// - Function calls (`exists()`, `count()`, etc.)
/// - Operators (`=`, `>`, `!`, etc.)
/// - Variables (`%varname`)
/// - Parentheses
fn simple_path_to_json(path: &str) -> Result<String, SofError> {
    // Quick sanity check: reject obvious non-path expressions
    let trimmed = path.trim();

    // Reject empty path
    if trimmed.is_empty() {
        return Err(SofError::Uncompilable {
            reason: "empty column path".to_string(),
        });
    }

    // Reject literal strings
    if trimmed.starts_with('\'') || trimmed.starts_with('"') {
        return Err(SofError::Uncompilable {
            reason: format!(
                "literal string '{trimmed}' cannot be compiled to SQL; \
                 use the in-process runner"
            ),
        });
    }

    // Reject variables
    if trimmed.starts_with('%') {
        return Err(SofError::Uncompilable {
            reason: format!(
                "FHIRPath variable '{trimmed}' cannot be compiled to SQL; \
                 use the in-process runner"
            ),
        });
    }

    // Reject anything with parens (function calls)
    if trimmed.contains('(') || trimmed.contains(')') {
        return Err(SofError::Uncompilable {
            reason: format!(
                "FHIRPath function call in path '{trimmed}' cannot be compiled to SQL; \
                 use the in-process runner"
            ),
        });
    }

    // Reject operators
    if trimmed.contains('=')
        || trimmed.contains('!')
        || trimmed.contains('>')
        || trimmed.contains('<')
        || trimmed.contains(' ')
    {
        return Err(SofError::Uncompilable {
            reason: format!(
                "FHIRPath expression '{trimmed}' contains operators and cannot be compiled \
                 to SQL; use the in-process runner"
            ),
        });
    }

    // Parse the path: identifiers, dots, and bracket indices
    // Valid chars: [a-zA-Z0-9_], '.', '[', ']', digits inside brackets
    if !trimmed
        .chars()
        .all(|c| c.is_alphanumeric() || c == '.' || c == '_' || c == '[' || c == ']' || c == '-')
    {
        return Err(SofError::Uncompilable {
            reason: format!(
                "path '{trimmed}' contains unsupported characters; use the in-process runner"
            ),
        });
    }

    // Convert to JSONPath: prefix with '$.' and keep the rest as-is
    // SQLite JSONPath uses the same syntax for dots and bracket indices as the input.
    Ok(format!("$.{trimmed}"))
}

// ============================================================================
// SQL generation
// ============================================================================

fn build_sql(
    resource_type: &str,
    plan: &SelectPlan,
    where_conditions: &[String],
    dialect: SqlDialect,
) -> Result<CompiledQuery, SofError> {
    // Handle unionAll (G8): build one SELECT per branch and UNION ALL them
    if !plan.union_branches.is_empty() {
        return build_union_all_sql(resource_type, plan, where_conditions, dialect);
    }

    let mut select_parts: Vec<String> = Vec::new();
    let mut columns: Vec<String> = Vec::new();

    // Root-level columns come first
    for col in &plan.root_columns {
        select_parts.push(format!("{} AS \"{}\"", col.expr, col.name));
        columns.push(col.name.clone());
    }

    // forEach columns come after
    if let Some(fe) = &plan.for_each {
        for col in &fe.columns {
            select_parts.push(format!("{} AS \"{}\"", col.expr, col.name));
            columns.push(col.name.clone());
        }
    }

    if columns.is_empty() {
        return Err(SofError::InvalidViewDefinition(
            "no output columns compiled".to_string(),
        ));
    }

    let select_clause = select_parts.join(",\n  ");

    // Build FROM / JOIN (dialect-specific)
    let from_clause = match dialect {
        SqlDialect::Sqlite => {
            if let Some(fe) = &plan.for_each {
                let join_type = if fe.is_left_join { "LEFT JOIN" } else { "JOIN" };
                // SQLite: json_each(r.data, '$.path') uses the $.path form
                let sqlite_path = format!("$.{}", fe.json_path);
                format!("resources r\n{join_type} json_each(r.data, '{sqlite_path}') fe ON 1=1")
            } else {
                "resources r".to_string()
            }
        }
        SqlDialect::Postgres => {
            if let Some(fe) = &plan.for_each {
                let join_type = if fe.is_left_join { "LEFT JOIN" } else { "JOIN" };
                // Postgres: jsonb_array_elements(r.data->'path') or #>'{a,b}'
                let pg_array_src = raw_path_to_pg_jsonb_expr(&fe.json_path, "r.data");
                format!(
                    "resources r\n{join_type} LATERAL jsonb_array_elements({pg_array_src}) AS fe(value) ON TRUE"
                )
            } else {
                "resources r".to_string()
            }
        }
    };

    // WHERE clause + params (dialect-specific)
    let base_where = match dialect {
        SqlDialect::Sqlite => {
            "r.tenant_id = ?1\n  AND r.resource_type = ?2\n  AND r.is_deleted = 0"
        }
        SqlDialect::Postgres => {
            "r.tenant_id = $1\n  AND r.resource_type = $2\n  AND r.is_deleted = false"
        }
    };

    let where_clause = if where_conditions.is_empty() {
        base_where.to_string()
    } else {
        format!("{base_where}\n  AND {}", where_conditions.join("\n  AND "))
    };

    let sql = format!(
        "SELECT\n  {select}\nFROM {from}\nWHERE {where}\nORDER BY r.last_updated, r.id",
        select = select_clause,
        from = from_clause,
        where = where_clause,
    );

    Ok(CompiledQuery { sql, columns })
}

/// Translates a raw FHIRPath (stored without `$.` prefix) to a PostgreSQL JSONB
/// navigation expression that returns a **JSONB** value (not text).
///
/// Used for `jsonb_array_elements(…)` argument in forEach joins:
/// - `name`       → `r.data->'name'`
/// - `name.given` → `r.data#>'{name,given}'`
fn raw_path_to_pg_jsonb_expr(raw_path: &str, src: &str) -> String {
    let expanded = expand_bracket_indices(raw_path).unwrap_or_else(|_| raw_path.to_string());
    let segments: Vec<&str> = expanded.split('.').collect();
    if segments.len() == 1 {
        format!("{src}->'{}'", segments[0])
    } else {
        let pg_path = segments.join(",");
        format!("{src}#>'{{{}}}' ", pg_path)
    }
}

/// Strips a trailing `ORDER BY …` clause from a SQL string (case-insensitive).
///
/// Used when assembling UNION ALL branches: `ORDER BY` inside individual
/// compound-SELECT terms is not portable across SQLite versions. Instead a
/// single `ORDER BY` is placed at the end of the whole compound query.
fn strip_order_by(sql: &str) -> &str {
    // rfind is correct here — if there's a nested subquery with its own ORDER BY
    // we want the outermost (last) occurrence.
    if let Some(pos) = find_outer_order_by(sql) {
        sql[..pos].trim_end()
    } else {
        sql
    }
}

/// Finds the byte offset of the outermost `ORDER BY` clause (i.e. not inside
/// nested parentheses).  Returns `None` if no such clause exists.
fn find_outer_order_by(sql: &str) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
            }
            b'\'' => {
                // Skip string literal
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\'' {
                        i += 1;
                        if i < bytes.len() && bytes[i] == b'\'' {
                            i += 1; // escaped quote
                        } else {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            _ if depth == 0 => {
                // Case-insensitive match for ORDER (followed by whitespace+BY)
                let remaining = &sql[i..];
                if remaining.len() >= 8
                    && remaining[..5].eq_ignore_ascii_case("ORDER")
                    && remaining.as_bytes()[5].is_ascii_whitespace()
                {
                    // Verify " BY" follows
                    let after = remaining[5..].trim_start();
                    if after.len() >= 2 && after[..2].eq_ignore_ascii_case("BY") {
                        return Some(i);
                    }
                }
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    None
}

// ============================================================================
// G8: unionAll SQL generation
// ============================================================================

/// Generates a UNION ALL query from a plan that contains `union_branches`.
///
/// Root columns from the parent plan are prepended to every branch so the
/// output schema is consistent across all branches.
fn build_union_all_sql(
    resource_type: &str,
    plan: &SelectPlan,
    where_conditions: &[String],
    dialect: SqlDialect,
) -> Result<CompiledQuery, SofError> {
    let mut branch_sqls: Vec<String> = Vec::new();
    let mut columns: Option<Vec<String>> = None;

    for branch in &plan.union_branches {
        // Merge root columns from parent plan into each branch
        let merged = SelectPlan {
            root_columns: plan
                .root_columns
                .iter()
                .cloned()
                .chain(branch.root_columns.iter().cloned())
                .collect(),
            for_each: branch.for_each.as_ref().map(|fe| ForEachPlan {
                json_path: fe.json_path.clone(),
                is_left_join: fe.is_left_join,
                columns: fe.columns.clone(),
            }),
            union_branches: vec![],
        };

        let compiled = build_sql(resource_type, &merged, where_conditions, dialect)?;

        // All branches must share the same column schema
        match &columns {
            None => columns = Some(compiled.columns.clone()),
            Some(expected) if *expected != compiled.columns => {
                return Err(SofError::Uncompilable {
                    reason: format!(
                        "unionAll branches produce different column schemas: \
                         {:?} vs {:?}",
                        expected, compiled.columns
                    ),
                });
            }
            _ => {}
        }

        // Strip the trailing ORDER BY from each branch — ORDER BY inside
        // individual compound-SELECT terms is not portable. A single ORDER BY
        // is added at the end of the full UNION ALL instead. Also do not wrap
        // in parentheses: older SQLite versions reject `(SELECT ...) UNION ALL`.
        let branch_sql = strip_order_by(&compiled.sql);
        branch_sqls.push(branch_sql.to_string());
    }

    if branch_sqls.is_empty() {
        return Err(SofError::InvalidViewDefinition(
            "unionAll produced no branches".to_string(),
        ));
    }

    // ORDER BY on the outer compound query (column position to avoid alias issues)
    let order_by = match dialect {
        SqlDialect::Sqlite => "\nORDER BY 1",
        SqlDialect::Postgres => "\nORDER BY 1",
    };

    let sql = format!("{}{order_by}", branch_sqls.join("\nUNION ALL\n"));

    Ok(CompiledQuery {
        sql,
        columns: columns.unwrap_or_default(),
    })
}

// ============================================================================
// G7: where-clause expression compiler
// ============================================================================

/// Compiles a single FHIRPath expression from `ViewDefinition.where[].path`
/// into a SQL boolean condition.
///
/// ## Supported patterns
///
/// | FHIRPath | SQL (SQLite) |
/// |----------|-------------|
/// | `field.exists()` | `json_extract(r.data,'$.field') IS NOT NULL` |
/// | `field.exists().not()` | `json_extract(r.data,'$.field') IS NULL` |
/// | `field.empty()` | `json_extract(r.data,'$.field') IS NULL` |
/// | `field = 'value'` | `json_extract(r.data,'$.field') = 'value'` |
/// | `field != 'value'` | `json_extract(r.data,'$.field') != 'value'` |
/// | `field = 42` | `json_extract(r.data,'$.field') = 42` |
/// | `field = true` | `json_extract(r.data,'$.field') = 1` (SQLite) |
pub(crate) fn compile_where_expr(path: &str, dialect: SqlDialect) -> Result<String, SofError> {
    let expr = path.trim();

    // --- exists() ---
    if let Some(field) = expr.strip_suffix(".exists()") {
        return compile_exists(field.trim(), false, dialect);
    }

    // --- exists().not() ---
    if let Some(field) = expr.strip_suffix(".exists().not()") {
        return compile_exists(field.trim(), true, dialect);
    }

    // --- empty() ---
    if let Some(field) = expr.strip_suffix(".empty()") {
        return compile_exists(field.trim(), true, dialect);
    }

    // --- binary operator: field OP value ---
    if let Some((field, op, value)) = parse_binary_expr(expr) {
        return compile_binary(field, op, value, dialect);
    }

    Err(SofError::Uncompilable {
        reason: format!(
            "where expression '{expr}' is not in the supported subset \
             (field.exists(), field = 'value', field != 'value'); \
             use the in-process runner"
        ),
    })
}

fn compile_exists(field: &str, negate: bool, dialect: SqlDialect) -> Result<String, SofError> {
    let null_check = if negate { "IS NULL" } else { "IS NOT NULL" };
    Ok(match dialect {
        SqlDialect::Sqlite => {
            let json_path = simple_path_to_json(field)?;
            format!("json_extract(r.data, '{json_path}') {null_check}")
        }
        SqlDialect::Postgres => {
            let pg_expr = path_to_pg_expr(field, "r.data")?;
            format!("{pg_expr} {null_check}")
        }
    })
}

fn compile_binary(
    field: &str,
    op: &str,
    value: &str,
    dialect: SqlDialect,
) -> Result<String, SofError> {
    let sql_op = match op {
        "=" => "=",
        "!=" => "!=",
        _ => {
            return Err(SofError::Uncompilable {
                reason: format!("operator '{op}' is not supported in where expressions"),
            });
        }
    };

    let sql_value = compile_literal(value, dialect)?;

    Ok(match dialect {
        SqlDialect::Sqlite => {
            let json_path = simple_path_to_json(field)?;
            format!("json_extract(r.data, '{json_path}') {sql_op} {sql_value}")
        }
        SqlDialect::Postgres => {
            let pg_expr = path_to_pg_expr(field, "r.data")?;
            format!("{pg_expr} {sql_op} {sql_value}")
        }
    })
}

/// Converts a FHIRPath literal (string, number, boolean) to a SQL literal.
fn compile_literal(value: &str, dialect: SqlDialect) -> Result<String, SofError> {
    // Single-quoted string: 'hello'
    if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
        let inner = &value[1..value.len() - 1];
        // Escape embedded single quotes for SQL
        return Ok(format!("'{}'", inner.replace('\'', "''")));
    }

    // Numeric literal
    if value.parse::<f64>().is_ok() {
        return Ok(value.to_string());
    }

    // Boolean literals
    match value {
        "true" => {
            return Ok(match dialect {
                SqlDialect::Sqlite => "1".to_string(),
                SqlDialect::Postgres => "true".to_string(),
            });
        }
        "false" => {
            return Ok(match dialect {
                SqlDialect::Sqlite => "0".to_string(),
                SqlDialect::Postgres => "false".to_string(),
            });
        }
        _ => {}
    }

    Err(SofError::Uncompilable {
        reason: format!(
            "literal value '{value}' is not supported in where expressions; \
             use a single-quoted string, a number, or true/false"
        ),
    })
}

/// Parses `field OP value` from a FHIRPath expression.
///
/// Returns `Some((field, op, value))` or `None` if no binary operator is found.
fn parse_binary_expr(expr: &str) -> Option<(&str, &str, &str)> {
    // Try `!=` first (longer operator)
    for op in &["!=", "="] {
        if let Some(pos) = expr.find(op) {
            // Make sure it's not inside a quoted string
            let before = &expr[..pos];
            let after = &expr[pos + op.len()..];
            if !before.contains('\'') {
                return Some((before.trim(), op, after.trim()));
            }
        }
    }
    None
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn compile(view: serde_json::Value) -> Result<CompiledQuery, SofError> {
        compile_view_definition(&view)
    }

    // --- Happy path ---

    #[test]
    fn test_flat_single_column() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "id", "name": "id", "type": "string"}]}]
        });
        let q = compile(view).unwrap();
        assert_eq!(q.columns, vec!["id"]);
        assert!(
            q.sql.contains("json_extract(r.data, '$.id') AS \"id\""),
            "{}",
            q.sql
        );
        assert!(q.sql.contains("r.tenant_id = ?1"), "{}", q.sql);
        assert!(q.sql.contains("r.resource_type = ?2"), "{}", q.sql);
        assert!(q.sql.contains("r.is_deleted = 0"), "{}", q.sql);
    }

    #[test]
    fn test_flat_multiple_columns() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{
                "column": [
                    {"path": "id", "name": "id"},
                    {"path": "gender", "name": "gender"},
                    {"path": "birthDate", "name": "dob"}
                ]
            }]
        });
        let q = compile(view).unwrap();
        assert_eq!(q.columns, vec!["id", "gender", "dob"]);
        assert!(
            q.sql.contains("json_extract(r.data, '$.id') AS \"id\""),
            "{}",
            q.sql
        );
        assert!(
            q.sql
                .contains("json_extract(r.data, '$.gender') AS \"gender\""),
            "{}",
            q.sql
        );
        assert!(
            q.sql
                .contains("json_extract(r.data, '$.birthDate') AS \"dob\""),
            "{}",
            q.sql
        );
    }

    #[test]
    fn test_multiple_flat_select_clauses() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [
                {"column": [{"path": "id", "name": "id"}]},
                {"column": [{"path": "gender", "name": "gender"}]}
            ]
        });
        let q = compile(view).unwrap();
        assert_eq!(q.columns, vec!["id", "gender"]);
    }

    #[test]
    fn test_for_each_produces_join() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{
                "forEach": "name",
                "column": [
                    {"path": "family", "name": "family"},
                    {"path": "use", "name": "use"}
                ]
            }]
        });
        let q = compile(view).unwrap();
        assert_eq!(q.columns, vec!["family", "use"]);
        assert!(
            q.sql.contains("JOIN json_each(r.data, '$.name') fe ON 1=1"),
            "{}",
            q.sql
        );
        assert!(
            q.sql
                .contains("json_extract(fe.value, '$.family') AS \"family\""),
            "{}",
            q.sql
        );
    }

    #[test]
    fn test_for_each_or_null_produces_left_join() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{
                "forEachOrNull": "name",
                "column": [{"path": "family", "name": "family"}]
            }]
        });
        let q = compile(view).unwrap();
        assert!(
            q.sql
                .contains("LEFT JOIN json_each(r.data, '$.name') fe ON 1=1"),
            "{}",
            q.sql
        );
    }

    #[test]
    fn test_mixed_root_and_foreach() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [
                {"column": [{"path": "id", "name": "id"}]},
                {"forEach": "name", "column": [{"path": "family", "name": "family"}]}
            ]
        });
        let q = compile(view).unwrap();
        assert_eq!(q.columns, vec!["id", "family"]);
        assert!(
            q.sql.contains("json_extract(r.data, '$.id') AS \"id\""),
            "{}",
            q.sql
        );
        assert!(
            q.sql
                .contains("json_extract(fe.value, '$.family') AS \"family\""),
            "{}",
            q.sql
        );
        assert!(
            q.sql.contains("JOIN json_each(r.data, '$.name') fe ON 1=1"),
            "{}",
            q.sql
        );
    }

    // --- unionAll (G8: now compiles to SQL UNION ALL) ---

    #[test]
    fn test_union_all_compiles_to_sql_union_all() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"unionAll": [
                {"column": [{"path": "id", "name": "id"}]},
                {"column": [{"path": "id", "name": "id"}]}
            ]}]
        });
        let q = compile(view).unwrap();
        assert!(
            q.sql.contains("UNION ALL"),
            "expected UNION ALL in compiled SQL: {}",
            q.sql
        );
    }

    #[test]
    fn test_rejects_literal_string_path() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "'hello'", "name": "x"}]}]
        });
        let err = compile(view).unwrap_err();
        assert!(matches!(err, SofError::Uncompilable { .. }), "{err:?}");
    }

    #[test]
    fn test_rejects_function_call_path() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "name.exists()", "name": "x"}]}]
        });
        let err = compile(view).unwrap_err();
        assert!(matches!(err, SofError::Uncompilable { .. }), "{err:?}");
    }

    #[test]
    fn test_rejects_multiple_foreach() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [
                {"forEach": "name", "column": [{"path": "family", "name": "family"}]},
                {"forEach": "address", "column": [{"path": "city", "name": "city"}]}
            ]
        });
        let err = compile(view).unwrap_err();
        assert!(matches!(err, SofError::Uncompilable { .. }), "{err:?}");
    }

    #[test]
    fn test_rejects_where_clause() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "where": [{"path": "active"}],
            "select": [{"column": [{"path": "id", "name": "id"}]}]
        });
        let err = compile(view).unwrap_err();
        assert!(matches!(err, SofError::Uncompilable { .. }), "{err:?}");
    }

    #[test]
    fn test_rejects_missing_resource() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "select": [{"column": [{"path": "id", "name": "id"}]}]
        });
        let err = compile(view).unwrap_err();
        assert!(matches!(err, SofError::InvalidViewDefinition(_)), "{err:?}");
    }

    #[test]
    fn test_path_translation_dotted() {
        let result = simple_path_to_json("subject.reference").unwrap();
        assert_eq!(result, "$.subject.reference");
    }

    #[test]
    fn test_path_translation_indexed() {
        let result = simple_path_to_json("name[0].family").unwrap();
        assert_eq!(result, "$.name[0].family");
    }

    // -----------------------------------------------------------------------
    // PostgreSQL dialect golden tests
    // -----------------------------------------------------------------------

    fn compile_pg(view: serde_json::Value) -> Result<CompiledQuery, SofError> {
        compile_view_definition_dialect(&view, SqlDialect::Postgres)
    }

    #[test]
    fn test_pg_flat_single_column() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "id", "name": "id", "type": "string"}]}]
        });
        let q = compile_pg(view).unwrap();
        assert_eq!(q.columns, vec!["id"]);
        assert!(q.sql.contains("r.data->>'id' AS \"id\""), "{}", q.sql);
        assert!(q.sql.contains("r.tenant_id = $1"), "{}", q.sql);
        assert!(q.sql.contains("r.resource_type = $2"), "{}", q.sql);
        assert!(q.sql.contains("r.is_deleted = false"), "{}", q.sql);
    }

    #[test]
    fn test_pg_flat_dotted_path() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Observation",
            "status": "active",
            "select": [{"column": [{"path": "subject.reference", "name": "subject_ref"}]}]
        });
        let q = compile_pg(view).unwrap();
        assert!(
            q.sql
                .contains("r.data#>>'{subject,reference}' AS \"subject_ref\""),
            "{}",
            q.sql
        );
    }

    #[test]
    fn test_pg_foreach_produces_lateral_join() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{
                "forEach": "name",
                "column": [
                    {"path": "family", "name": "family"},
                    {"path": "use", "name": "use_code"}
                ]
            }]
        });
        let q = compile_pg(view).unwrap();
        assert_eq!(q.columns, vec!["family", "use_code"]);
        assert!(
            q.sql
                .contains("JOIN LATERAL jsonb_array_elements(r.data->'name') AS fe(value) ON TRUE"),
            "{}",
            q.sql
        );
        assert!(
            q.sql.contains("fe.value->>'family' AS \"family\""),
            "{}",
            q.sql
        );
        assert!(
            q.sql.contains("fe.value->>'use' AS \"use_code\""),
            "{}",
            q.sql
        );
    }

    #[test]
    fn test_pg_foreach_or_null_produces_left_lateral_join() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{
                "forEachOrNull": "name",
                "column": [{"path": "family", "name": "family"}]
            }]
        });
        let q = compile_pg(view).unwrap();
        assert!(
            q.sql.contains(
                "LEFT JOIN LATERAL jsonb_array_elements(r.data->'name') AS fe(value) ON TRUE"
            ),
            "{}",
            q.sql
        );
    }

    #[test]
    fn test_pg_mixed_root_and_foreach() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [
                {"column": [{"path": "id", "name": "id"}]},
                {"forEach": "name", "column": [{"path": "family", "name": "family"}]}
            ]
        });
        let q = compile_pg(view).unwrap();
        assert_eq!(q.columns, vec!["id", "family"]);
        assert!(q.sql.contains("r.data->>'id' AS \"id\""), "{}", q.sql);
        assert!(
            q.sql.contains("fe.value->>'family' AS \"family\""),
            "{}",
            q.sql
        );
        assert!(
            q.sql
                .contains("JOIN LATERAL jsonb_array_elements(r.data->'name') AS fe(value) ON TRUE"),
            "{}",
            q.sql
        );
    }

    #[test]
    fn test_pg_rejects_function_call() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "name.exists()", "name": "x"}]}]
        });
        let err = compile_pg(view).unwrap_err();
        assert!(matches!(err, SofError::Uncompilable { .. }), "{err:?}");
    }
}
