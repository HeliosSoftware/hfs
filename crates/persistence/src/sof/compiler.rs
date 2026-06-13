//! ViewDefinition → SQL compiler (SQLite and PostgreSQL dialects).
//!
//! Thin façade over the IR-based pipeline:
//!
//! 1. [`build_plan`] walks the ViewDefinition JSON and produces a
//!    [`PlanNode`](super::ir::PlanNode) tree plus the resolved
//!    `ViewDefinition.constant[]` values.
//! 2. [`emit_plan`] lowers the plan to dialect-appropriate SQL via the
//!    [`Dialect`] trait.
//!
//! Returns [`SofError::Uncompilable`] for FHIRPath constructs the in-DB
//! pipeline doesn't yet handle (e.g. `where(crit)` chains, the boundary
//! functions without a column type hint, deeper unionAll/repeat nesting).
//! There is no in-process fallback — the REST handler maps these errors
//! to `422 Unprocessable Entity`.

use helios_fhir::FhirVersion;
use serde_json::Value;

use crate::core::sof_runner::SofError;

use super::compile_view::build_plan;
use super::dialect::{Dialect, PgDialect, SqliteDialect};
use super::emit::emit_plan;

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
    /// - SQLite: `?1 = tenant_id`, `?2 = resource_type`, `?3..N = constants`
    /// - PostgreSQL: `$1 = tenant_id`, `$2 = resource_type`, `$3..N = constants`
    pub sql: String,
    /// Column names in the order they appear in the SELECT list.
    pub columns: Vec<String>,
    /// Resolved `ViewDefinition.constant[]` values, in allocation order.
    /// Bound by the runners as `$3..` / `?3..` after `tenant_id` and
    /// `resource_type`.
    pub constants: Vec<super::ir::LitValue>,
}

/// Picks the dialect implementation for a given [`SqlDialect`].
fn dialect_for(d: SqlDialect) -> Box<dyn Dialect> {
    match d {
        SqlDialect::Sqlite => Box::new(SqliteDialect),
        SqlDialect::Postgres => Box::new(PgDialect),
    }
}

/// Compiles a raw ViewDefinition JSON value into a [`CompiledQuery`] for SQLite.
///
/// Shorthand for `compile_view_definition_dialect(view_json, SqlDialect::Sqlite,
/// FhirVersion::default())`.
pub fn compile_view_definition(view_json: &Value) -> Result<CompiledQuery, SofError> {
    compile_view_definition_dialect(view_json, SqlDialect::Sqlite, FhirVersion::default())
}

/// Compiles a raw ViewDefinition JSON value into a [`CompiledQuery`] for the given dialect.
///
/// `fhir_version` controls which generated `get_field_type` lookup table the
/// compile-time cardinality validator consults. Pass the configured server
/// default when calling from a runner.
///
/// # Errors
///
/// Returns [`SofError::Uncompilable`] for any unsupported construct.
/// Returns [`SofError::InvalidViewDefinition`] if required fields are missing.
pub fn compile_view_definition_dialect(
    view_json: &Value,
    dialect: SqlDialect,
    fhir_version: FhirVersion,
) -> Result<CompiledQuery, SofError> {
    let dial = dialect_for(dialect);
    let (plan, constants) = build_plan(view_json, dial.as_ref(), fhir_version)?;
    let emitted = emit_plan(&plan, dial.as_ref())?;
    Ok(CompiledQuery {
        sql: emitted.sql,
        columns: emitted.columns,
        constants,
    })
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
    fn test_accepts_literal_string_path() {
        // A column whose path is a bare string literal compiles to a constant
        // projection — `'hello'` is a valid FHIRPath expression even if
        // unusual as a column.path.
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "'hello'", "name": "x"}]}]
        });
        let q = compile(view).unwrap();
        assert!(q.sql.contains("'hello' AS \"x\""), "{}", q.sql);
    }

    #[test]
    fn test_accepts_exists_function_call_path() {
        // `name.exists()` in a column path lowers to an existence predicate.
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "name.exists()", "name": "has_name"}]}]
        });
        let q = compile(view).unwrap();
        assert!(q.sql.contains("IS NOT NULL"), "{}", q.sql);
        assert!(q.sql.contains("AS \"has_name\""), "{}", q.sql);
    }

    #[test]
    fn test_sibling_foreach_emits_cross_join() {
        // Sibling forEach clauses produce a cartesian product via two
        // sequential lateral unnests off `r.data` — one per clause.
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [
                {"forEach": "name", "column": [{"path": "family", "name": "family"}]},
                {"forEach": "address", "column": [{"path": "city", "name": "city"}]}
            ]
        });
        let q = compile(view).unwrap();
        assert_eq!(q.columns, vec!["family", "city"]);
        // First unnest keeps the `fe` alias (legacy), second uses `fe2`.
        assert!(
            q.sql.contains("JOIN json_each(r.data, '$.name') fe ON"),
            "{}",
            q.sql
        );
        assert!(
            q.sql.contains("JOIN json_each(r.data, '$.address') fe2 ON"),
            "{}",
            q.sql
        );
    }

    #[test]
    fn test_accepts_bare_boolean_where() {
        // Top-level `where: [{path: "active"}]` lowers to a boolean coercion
        // around the bare field — FHIRPath's three-valued logic boundary is
        // applied as `IS TRUE` so empty/NULL filter the row out.
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "where": [{"path": "active"}],
            "select": [{"column": [{"path": "id", "name": "id"}]}]
        });
        let q = compile(view).unwrap();
        // SQLite truthy boundary doesn't use `IS TRUE` (which is strict-typed
        // in some dialects) — it checks IS NOT NULL + non-zero / not 'false'.
        assert!(q.sql.contains("IS NOT NULL"), "{}", q.sql);
        assert!(
            q.sql.contains("json_extract(r.data, '$.active')"),
            "{}",
            q.sql
        );
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

    // -----------------------------------------------------------------------
    // PostgreSQL dialect golden tests
    // -----------------------------------------------------------------------

    fn compile_pg(view: serde_json::Value) -> Result<CompiledQuery, SofError> {
        compile_view_definition_dialect(&view, SqlDialect::Postgres, FhirVersion::default())
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
        // The compiler emits `coalesce(<array-first>, <plain>)` for two-Field
        // paths so navigation through arrays (e.g. `name.family`) auto-picks
        // the first element when the intermediate is array-shaped.
        assert!(
            q.sql.contains("coalesce(r.data#>>'{subject,0,reference}'"),
            "{}",
            q.sql
        );
        assert!(
            q.sql.contains("r.data#>>'{subject,reference}'"),
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
                .contains("JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) AS fe(value) ON TRUE"),
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
                "LEFT JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) AS fe(value) ON TRUE"
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
                .contains("JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) AS fe(value) ON TRUE"),
            "{}",
            q.sql
        );
    }

    #[test]
    fn test_repeat_unionall_sql() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "QuestionnaireResponse",
            "select": [
                {"column": [{"name": "id", "path": "id"}]},
                {"unionAll": [
                    {"repeat": ["item"], "column": [
                        {"name": "type", "path": "'item'"},
                        {"name": "linkId", "path": "linkId"}
                    ]},
                    {"repeat": ["item", "answer.item"], "column": [
                        {"name": "type", "path": "'answer-item'"},
                        {"name": "linkId", "path": "linkId"}
                    ]}
                ]}
            ]
        });
        let q = compile(view).unwrap();
        eprintln!("REPEAT-UNION SQL:\n{}", q.sql);
    }

    #[test]
    fn test_union_nested_sql() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [{
                "column": [{"name": "id", "path": "id"}],
                "unionAll": [
                    {"forEach": "telecom[0]", "column": [{"name": "tel", "path": "value"}]},
                    {"unionAll": [
                        {"forEach": "telecom[0]", "column": [{"name": "tel", "path": "value"}]},
                        {"forEach": "contact.telecom[0]", "column": [{"name": "tel", "path": "value"}]}
                    ]}
                ]
            }]
        });
        let q = compile(view).unwrap();
        eprintln!("UNION NESTED SQL:\n{}", q.sql);
    }

    #[test]
    fn test_foreach_with_union_all_sql() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [
                {"column": [{"path": "id", "name": "id"}]},
                {"forEach": "contact", "unionAll": [
                    {"column": [{"path": "name.family", "name": "name", "type": "string"}]},
                    {"forEach": "name.given", "column": [{"path": "$this", "name": "name", "type": "string"}]}
                ]}
            ]
        });
        let q = compile(view).unwrap();
        eprintln!("SQL:\n{}", q.sql);
    }

    #[test]
    fn test_collection_emits_full_query() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [{"column": [
                {"path": "id", "name": "id"},
                {"path": "name.family", "name": "lf", "type": "string", "collection": true}
            ]}]
        });
        let q = compile(view).unwrap();
        eprintln!("FULL SQL:\n{}", q.sql);
    }

    #[test]
    fn test_collection_true_emits_json_agg() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [{"column": [
                {"path": "id", "name": "id"},
                {"path": "name.family", "name": "lf", "type": "string", "collection": true}
            ]}]
        });
        let q = compile(view).unwrap();
        eprintln!("SQL:\n{}", q.sql);
        assert!(q.sql.contains("json_group_array"), "{}", q.sql);
    }

    #[test]
    fn test_two_segment_path_emits_coalesce() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [
                {"path": "id", "name": "id"},
                {"path": "name.family", "name": "family"}
            ]}]
        });
        let q = compile(view).unwrap();
        eprintln!("SQL:\n{}", q.sql);
        assert!(q.sql.contains("coalesce("), "{}", q.sql);
    }

    #[test]
    fn test_repeat_emits_recursive_cte() {
        // SoF `repeat:` directive lowers to a `WITH RECURSIVE … SELECT`
        // shape; the CTE projects (rid, node) and the outer SELECT joins
        // back to `resources r` to resolve sibling root columns.
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "QuestionnaireResponse",
            "select": [
                {"column": [{"path": "id", "name": "id"}]},
                {"repeat": ["item"], "column": [
                    {"path": "linkId", "name": "linkId"},
                    {"path": "text", "name": "text"}
                ]}
            ]
        });
        let q = compile(view).unwrap();
        assert_eq!(q.columns, vec!["id", "linkId", "text"]);
        assert!(q.sql.contains("WITH RECURSIVE"), "{}", q.sql);
        assert!(q.sql.contains("UNION ALL"), "{}", q.sql);
    }

    #[test]
    fn test_pg_accepts_exists_function_call() {
        // PG version of test_accepts_exists_function_call_path — confirms
        // `.exists()` lowers to an `IS NOT NULL` predicate.
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "name.exists()", "name": "has_name"}]}]
        });
        let q = compile_pg(view).unwrap();
        assert!(q.sql.contains("IS NOT NULL"), "{}", q.sql);
        assert!(q.sql.contains("AS \"has_name\""), "{}", q.sql);
    }
}
