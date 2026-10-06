//! ViewDefinition compiler (SQLite/PostgreSQL SQL and MongoDB pipelines).
//!
//! Thin façade over the IR-based pipeline:
//!
//! 1. [`build_plan`] walks the ViewDefinition JSON and produces a
//!    [`PlanNode`](super::ir::PlanNode) tree plus the resolved
//!    `ViewDefinition.constant[]` values. The [`CompileTarget`] tunes
//!    target-specific lowering (e.g. trailing-`[N]` forEach).
//! 2. The emitter lowers the plan to the target form: [`emit_plan`] for SQL via
//!    the [`Dialect`] trait, or [`emit_mongo`](super::emit_mongo::emit_mongo)
//!    for a MongoDB aggregation pipeline.
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
#[cfg(any(feature = "sqlite", feature = "postgres", test))]
use super::emit::{RESOURCES_TABLE, tenant_predicate};
use super::ir::PlanNode;

/// Where a runtime cap can be applied without changing the existing sort's
/// treatment of ties between rows produced by one resource. PostgreSQL
/// retains its client-side cap for row-producing expansions, unions and recursion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OutputLimitStrategy {
    Direct,
    RuntimeOnly,
}

impl OutputLimitStrategy {
    fn for_plan(plan: &PlanNode) -> Self {
        match plan {
            PlanNode::Scan { .. } => Self::Direct,
            PlanNode::Project { parent, .. } | PlanNode::Filter { parent, .. } => {
                Self::for_plan(parent)
            }
            PlanNode::LateralUnnest { .. } | PlanNode::Union(_) | PlanNode::Recurse { .. } => {
                Self::RuntimeOnly
            }
        }
    }
}

/// SQL dialect to target during compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlDialect {
    /// SQLite: `json_extract`, `json_each`, positional `?1`/`?2` params.
    Sqlite,
    /// PostgreSQL: JSONB operators (`->>`/ `#>>`), `jsonb_array_elements`, `$1`/`$2` params.
    Postgres,
}

/// Backend a ViewDefinition is being compiled for. Drives target-specific
/// lowering decisions in [`build_plan`] (e.g. whether trailing-`[N]` forEach
/// paths may use a correlated subquery) and selects the emitter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileTarget {
    /// SQLite SQL emitter.
    Sqlite,
    /// PostgreSQL SQL emitter.
    Postgres,
    /// MongoDB aggregation-pipeline emitter.
    #[cfg(feature = "mongodb")]
    Mongo,
}

impl CompileTarget {
    /// Whether the target can index a flattened collection via a correlated
    /// subquery in `FROM`. SQL backends can (`ScalarFromChain`); the MongoDB
    /// emitter instead carries `flat_index` on the unnest and lowers it to
    /// `$arrayElemAt`, so `build_plan` must NOT produce `ScalarFromChain` nodes
    /// for it.
    pub(super) fn supports_correlated_from_subqueries(self) -> bool {
        match self {
            CompileTarget::Sqlite | CompileTarget::Postgres => true,
            #[cfg(feature = "mongodb")]
            CompileTarget::Mongo => false,
        }
    }
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
    /// How each column's text value is turned into JSON by the runners,
    /// parallel to `columns`.
    pub column_decodes: Vec<super::decode::ColumnDecode>,
    /// Resolved `ViewDefinition.constant[]` values, in allocation order.
    /// Bound by the runners as `$3..` / `?3..` after `tenant_id` and
    /// `resource_type`.
    pub constants: Vec<super::ir::LitValue>,
}

/// Compiled SQL-on-FHIR view, in the form the target backend executes:
/// parameterised SQL, or a MongoDB aggregation pipeline.
#[derive(Debug, Clone)]
pub enum CompiledView {
    /// SQL text + bind constants for the SQLite / PostgreSQL runners.
    Sql(CompiledQuery),
    /// Aggregation pipeline for the MongoDB runner.
    #[cfg(feature = "mongodb")]
    Mongo(CompiledPipeline),
}

/// Output of compiling a ViewDefinition to a MongoDB aggregation pipeline.
#[cfg(feature = "mongodb")]
#[derive(Debug, Clone)]
pub struct CompiledPipeline {
    /// Aggregation stages, ready to pass to `Collection::aggregate`. The leading
    /// `$match` already constrains `tenant_id`/`resource_type`/`is_deleted`.
    pub pipeline: Vec<mongodb::bson::Document>,
    /// Column names in `select` order (the keys of the final `$project`).
    pub columns: Vec<String>,
    /// Resolved `ViewDefinition.constant[]` values, in allocation order.
    ///
    /// MongoDB has no out-of-band bind parameters, so the emitter inlines these
    /// as BSON literals; they are surfaced here for parity/diagnostics only.
    pub constants: Vec<super::ir::LitValue>,
}

/// Picks the dialect implementation for a given [`SqlDialect`].
fn dialect_for(d: SqlDialect) -> Box<dyn Dialect> {
    match d {
        SqlDialect::Sqlite => Box::new(SqliteDialect),
        SqlDialect::Postgres => Box::new(PgDialect),
    }
}

/// Attaches runtime filter `conditions` (already AND-joined and parameterised)
/// to every scan of `resources r` in compiled `sql`: each `unionAll` branch,
/// each `repeat` seed and the `repeat` join-back, not just the last `WHERE`
/// (#1701). Every scan carries the emitter's tenant predicate, so the
/// conditions go right after each occurrence of it.
///
/// Returns [`SofError::Uncompilable`] when the number of tenant predicates
/// differs from the number of `resources r` scans: a scan the conditions
/// cannot be attached to must not run unfiltered.
#[cfg(any(feature = "sqlite", feature = "postgres", test))]
pub(super) fn attach_runtime_conditions(
    sql: &str,
    dialect: SqlDialect,
    conditions: &str,
) -> Result<String, SofError> {
    let anchor = tenant_predicate(dialect_for(dialect).as_ref());
    let anchors = sql.matches(anchor.as_str()).count();
    let scans = count_resource_scans(sql);
    if anchors == 0 || anchors != scans {
        return Err(SofError::Uncompilable {
            reason: format!(
                "the patient, group and _since filters cannot be applied to every part of \
                 this view ({anchors} tenant predicates for {scans} scans of {RESOURCES_TABLE})"
            ),
        });
    }
    Ok(sql.replace(anchor.as_str(), &format!("{anchor} AND {conditions}")))
}

/// Counts scans of `resources r`, ignoring matches inside a longer identifier.
#[cfg(any(feature = "sqlite", feature = "postgres", test))]
fn count_resource_scans(sql: &str) -> usize {
    let needle = format!("{RESOURCES_TABLE} r");
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    sql.match_indices(needle.as_str())
        .filter(|(i, m)| {
            !sql[..*i].chars().next_back().is_some_and(is_ident)
                && !sql[i + m.len()..].chars().next().is_some_and(is_ident)
        })
        .count()
}

/// Compiles a raw ViewDefinition JSON value into a [`CompiledQuery`] for SQLite.
///
/// Shorthand for `compile_view_definition_dialect(view_json, SqlDialect::Sqlite,
/// FhirVersion::default_enabled())`.
pub fn compile_view_definition(view_json: &Value) -> Result<CompiledQuery, SofError> {
    compile_view_definition_dialect(
        view_json,
        SqlDialect::Sqlite,
        FhirVersion::default_enabled(),
    )
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
    compile_view_definition_with_limit_strategy(view_json, dialect, fhir_version)
        .map(|(query, _)| query)
}

/// Compile once and retain the IR's row shape for PostgreSQL's runtime cap.
/// Scalar expressions and their intrinsic LIMIT 1 remain inside projections;
/// row-producing plan nodes retain the existing runtime-only cap.
pub(super) fn compile_view_definition_with_limit_strategy(
    view_json: &Value,
    dialect: SqlDialect,
    fhir_version: FhirVersion,
) -> Result<(CompiledQuery, OutputLimitStrategy), SofError> {
    let target = match dialect {
        SqlDialect::Sqlite => CompileTarget::Sqlite,
        SqlDialect::Postgres => CompileTarget::Postgres,
    };
    let dial = dialect_for(dialect);
    let (plan, constants) = build_plan(view_json, dial.as_ref(), target, fhir_version)?;
    let strategy = OutputLimitStrategy::for_plan(&plan);
    let emitted = emit_plan(&plan, dial.as_ref())?;
    Ok((
        CompiledQuery {
            sql: emitted.sql,
            columns: emitted.columns,
            column_decodes: emitted.column_decodes,
            constants,
        },
        strategy,
    ))
}

/// Compiles a ViewDefinition for an arbitrary [`CompileTarget`], returning the
/// target-appropriate [`CompiledView`]. Single funnel through [`build_plan`]
/// so every target shares the JSON→IR lowering.
#[cfg(feature = "mongodb")]
fn compile_view_target(
    view_json: &Value,
    target: CompileTarget,
    fhir_version: FhirVersion,
) -> Result<CompiledView, SofError> {
    match target {
        CompileTarget::Sqlite | CompileTarget::Postgres => {
            let dialect = if target == CompileTarget::Postgres {
                SqlDialect::Postgres
            } else {
                SqlDialect::Sqlite
            };
            compile_view_definition_with_limit_strategy(view_json, dialect, fhir_version)
                .map(|(query, _)| CompiledView::Sql(query))
        }
        #[cfg(feature = "mongodb")]
        CompileTarget::Mongo => {
            // The dialect is unused on the Mongo path (build_plan only consults
            // it inside the correlated-subquery lowering, which Mongo skips), so
            // a SQLite dialect serves purely as a never-called placeholder.
            let dial = dialect_for(SqlDialect::Sqlite);
            let (plan, constants) = build_plan(view_json, dial.as_ref(), target, fhir_version)?;
            let emitted = super::emit_mongo::emit_mongo(&plan, &constants)?;
            Ok(CompiledView::Mongo(CompiledPipeline {
                pipeline: emitted.pipeline,
                columns: emitted.columns,
                constants,
            }))
        }
    }
}

/// Compiles a raw ViewDefinition JSON value into a MongoDB aggregation pipeline.
///
/// # Errors
///
/// Returns [`SofError::Uncompilable`] for constructs the Mongo emitter does not
/// yet support (e.g. `lowBoundary`/`highBoundary`, `repeat:`, collections).
#[cfg(feature = "mongodb")]
pub fn compile_view_definition_mongo(
    view_json: &Value,
    fhir_version: FhirVersion,
) -> Result<CompiledPipeline, SofError> {
    match compile_view_target(view_json, CompileTarget::Mongo, fhir_version)? {
        CompiledView::Mongo(p) => Ok(p),
        CompiledView::Sql(_) => unreachable!("Mongo target never compiles to SQL"),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sof::decode::ColumnDecode;
    use serde_json::json;

    fn compile(view: serde_json::Value) -> Result<CompiledQuery, SofError> {
        compile_view_definition(&view)
    }

    #[test]
    fn test_output_limit_strategy_follows_row_producing_ir() {
        let cases = [
            (
                json!({"resource":"Observation","where":[{"path":"status = 'final'"}],
                "select":[{"column":[{"name":"id","path":"id"}]}]}),
                OutputLimitStrategy::Direct,
            ),
            (
                json!({"resource":"Patient","select":[{"column":[
                {"name":"family","path":"name.first().family"},
                {"name":"given","path":"name[0].given[0]"}]}]}),
                OutputLimitStrategy::Direct,
            ),
            (
                json!({"resource":"Patient","select":[{"forEach":"name.given[0]",
                "column":[{"name":"given","path":"$this"}]}]}),
                OutputLimitStrategy::Direct,
            ),
            (
                json!({"resource":"Patient","where":[{"path":"active"}],
                "select":[{"forEach":"name","column":[{"name":"family","path":"family"}]}]}),
                OutputLimitStrategy::RuntimeOnly,
            ),
            (
                json!({"resource":"Patient","select":[{"forEachOrNull":"name",
                "column":[{"name":"family","path":"family"}]}]}),
                OutputLimitStrategy::RuntimeOnly,
            ),
            (
                json!({"resource":"Patient","select":[{"unionAll":[
                {"column":[{"name":"id","path":"id"}]},
                {"column":[{"name":"id","path":"id"}]}]}]}),
                OutputLimitStrategy::RuntimeOnly,
            ),
            (
                json!({"resource":"Patient","select":[{"unionAll":[
                {"forEach":"name","column":[{"name":"family","path":"family"}]},
                {"forEach":"name","column":[{"name":"family","path":"family"}]}]}]}),
                OutputLimitStrategy::RuntimeOnly,
            ),
            (
                json!({"resource":"QuestionnaireResponse","select":[{"repeat":["item"],
                "column":[{"name":"link_id","path":"linkId"}]}]}),
                OutputLimitStrategy::RuntimeOnly,
            ),
            (
                json!({"resource":"QuestionnaireResponse","select":[{"repeat":["item","answer.item"],
                "column":[{"name":"link_id","path":"linkId"}]}]}),
                OutputLimitStrategy::RuntimeOnly,
            ),
        ];
        for dialect in [SqlDialect::Postgres, SqlDialect::Sqlite] {
            for (view, expected) in &cases {
                let (query, strategy) = compile_view_definition_with_limit_strategy(
                    view,
                    dialect,
                    FhirVersion::default_enabled(),
                )
                .unwrap_or_else(|error| panic!("{dialect:?} {view}: {error}"));
                assert_eq!(strategy, *expected, "{dialect:?} {view}");
                let public =
                    compile_view_definition_dialect(view, dialect, FhirVersion::default_enabled())
                        .unwrap();
                assert_eq!(public.sql, query.sql);
                assert_eq!(public.columns, query.columns);
                assert_eq!(public.constants.len(), query.constants.len());
            }
        }
    }

    #[test]
    fn test_indexed_lateral_under_project_and_filter_keeps_runtime_only_limit() {
        use super::super::ir::{LitValue, SqlExpr};
        let scan = PlanNode::Scan {
            alias: "r".into(),
            resource_type: "Patient".into(),
        };
        let plan = PlanNode::Project {
            columns: Vec::new(),
            parent: Box::new(PlanNode::Filter {
                predicate: SqlExpr::Lit(LitValue::Bool(true)),
                parent: Box::new(PlanNode::LateralUnnest {
                    parent: Box::new(scan),
                    source: SqlExpr::Lit(LitValue::Null),
                    out_alias: "fe".into(),
                    left_join: false,
                    on_filter: None,
                    flat_index: Some(0),
                }),
            }),
        };
        assert_eq!(
            OutputLimitStrategy::for_plan(&plan),
            OutputLimitStrategy::RuntimeOnly
        );
    }

    #[test]
    fn test_indexed_foreach_scalar_from_chain_keeps_direct_limit() {
        let view = json!({"resource":"Patient", "constant":[{"name":"g","valueString":"male"}],
            "where":[{"path":"gender = %g"}],
            "select":[{"forEach":"name.given[0]", "column":[{"name":"given","path":"$this"}]}]});
        let dialect = PgDialect;
        let (plan, _) = build_plan(
            &view,
            &dialect,
            CompileTarget::Postgres,
            FhirVersion::default_enabled(),
        )
        .unwrap();
        let PlanNode::Project { columns, .. } = &plan else {
            panic!("expected projection")
        };
        assert!(columns.iter().any(|column| matches!(
            column.expr,
            super::super::ir::SqlExpr::ScalarFromChain { .. }
        )));
        assert_eq!(
            OutputLimitStrategy::for_plan(&plan),
            OutputLimitStrategy::Direct
        );
        let (query, strategy) = compile_view_definition_with_limit_strategy(
            &view,
            SqlDialect::Postgres,
            FhirVersion::default_enabled(),
        )
        .unwrap();
        assert_eq!(strategy, OutputLimitStrategy::Direct);
        assert!(query.sql.contains("LIMIT 1 OFFSET 0"));
        assert!(
            matches!(&query.constants[..], [super::super::ir::LitValue::Str(value)] if value == "male")
        );
        let public = compile_view_definition_dialect(
            &view,
            SqlDialect::Postgres,
            FhirVersion::default_enabled(),
        )
        .unwrap();
        assert_eq!(public.sql, query.sql);
        assert_eq!(public.columns, query.columns);
        assert!(
            matches!(&public.constants[..], [super::super::ir::LitValue::Str(value)] if value == "male")
        );
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
    fn test_attach_runtime_conditions_refuses_a_scan_without_the_tenant_predicate() {
        for dialect in [SqlDialect::Sqlite, SqlDialect::Postgres] {
            let bare = "SELECT r.id FROM resources r WHERE r.id = 'x'";
            assert!(matches!(
                attach_runtime_conditions(bare, dialect, "1=0"),
                Err(SofError::Uncompilable { .. })
            ));

            let view = json!({
                "resourceType": "ViewDefinition",
                "resource": "Patient",
                "status": "active",
                "select": [{"column": [{"path": "id", "name": "id"}]}]
            });
            let flat =
                compile_view_definition_dialect(&view, dialect, FhirVersion::default_enabled())
                    .unwrap();
            let extra_scan = format!("{} UNION ALL SELECT r.id FROM resources r", flat.sql);
            assert!(matches!(
                attach_runtime_conditions(&extra_scan, dialect, "1=0"),
                Err(SofError::Uncompilable { .. })
            ));

            let union_view = json!({
                "resourceType": "ViewDefinition",
                "resource": "Patient",
                "status": "active",
                "select": [{"unionAll": [
                    {"column": [{"path": "id", "name": "id"}]},
                    {"column": [{"path": "id", "name": "id"}]}
                ]}]
            });
            let union = compile_view_definition_dialect(
                &union_view,
                dialect,
                FhirVersion::default_enabled(),
            )
            .unwrap();
            let attached = attach_runtime_conditions(&union.sql, dialect, "1=0").unwrap();
            assert_eq!(attached.matches(" AND 1=0").count(), 2, "{attached}");
        }
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
                .contains("JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE"),
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
                "LEFT JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE"
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
                .contains("JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE"),
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

    // -----------------------------------------------------------------------
    // Member-name validation: backtick-delimited identifiers may contain any
    // character, but only plain identifiers may reach the SQL text.
    // -----------------------------------------------------------------------

    /// A ViewDefinition whose single `select` clause is `clause` (a JSON
    /// object) over `Patient`, with top-level `where` predicates `wheres`.
    fn view_with(clause: serde_json::Value, wheres: &[&str]) -> serde_json::Value {
        let wheres: Vec<_> = wheres.iter().map(|p| json!({"path": p})).collect();
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "where": wheres,
            "select": [clause]
        })
    }

    fn column_view(path: &str) -> serde_json::Value {
        view_with(json!({"column": [{"path": path, "name": "c"}]}), &[])
    }

    /// Asserts the view is refused by both dialects with `Uncompilable` and a
    /// message that names the offending construct.
    fn assert_rejected(view: serde_json::Value, what: &str) {
        let results = [
            ("sqlite", compile(view.clone())),
            ("postgres", compile_pg(view)),
        ];
        for (dialect, result) in results {
            match result {
                Err(SofError::Uncompilable { reason }) => assert!(
                    reason.contains("not supported by the in-DB runner")
                        && reason.contains("plain identifiers"),
                    "{what} ({dialect}): unexpected reason: {reason}"
                ),
                Err(other) => panic!("{what} ({dialect}): wrong error: {other:?}"),
                Ok(q) => panic!("{what} ({dialect}): compiled to SQL: {}", q.sql),
            }
        }
    }

    /// Member names that are not plain identifiers; each is a valid
    /// backtick-delimited identifier for the FHIRPath parser.
    const HOSTILE_MEMBERS: &[&str] = &[
        "a'b",
        "x') OR 1=1 --",
        "a\"b",
        "a.b",
        "a b",
        "a,b",
        "a}b",
        "a[0]",
        "a\\\\b",
        "1a",
    ];

    #[test]
    fn test_non_plain_member_names_are_rejected_in_column_paths() {
        for m in HOSTILE_MEMBERS {
            assert_rejected(column_view(&format!("`{m}`")), &format!("root `{m}`"));
            assert_rejected(column_view(&format!("name.`{m}`")), &format!("name.`{m}`"));
            assert_rejected(
                column_view(&format!("name.family.`{m}`")),
                &format!("name.family.`{m}`"),
            );
            assert_rejected(
                column_view(&format!("name[0].`{m}`")),
                &format!("name[0].`{m}`"),
            );
            assert_rejected(
                column_view(&format!("name.first().`{m}`")),
                &format!("name.first().`{m}`"),
            );
        }
    }

    #[test]
    fn test_non_plain_member_names_are_rejected_in_where() {
        for m in HOSTILE_MEMBERS {
            assert_rejected(
                view_with(
                    json!({"column": [{"path": "id", "name": "c"}]}),
                    &[&format!("`{m}`.exists()")],
                ),
                &format!("where `{m}`.exists()"),
            );
            assert_rejected(
                view_with(
                    json!({"column": [{"path": "id", "name": "c"}]}),
                    &[&format!("name.where(`{m}` = 'x').exists()")],
                ),
                &format!("where name.where(`{m}` = 'x').exists()"),
            );
        }
    }

    #[test]
    fn test_non_plain_member_names_are_rejected_in_iteration_paths() {
        for m in HOSTILE_MEMBERS {
            for key in ["forEach", "forEachOrNull"] {
                for src in [format!("`{m}`"), format!("name.`{m}`")] {
                    assert_rejected(
                        view_with(
                            json!({key: src, "column": [{"path": "$this", "name": "c"}]}),
                            &[],
                        ),
                        &format!("{key} {src}"),
                    );
                }
            }
            assert_rejected(
                view_with(
                    json!({"repeat": [format!("`{m}`")], "column": [{"path": "id", "name": "c"}]}),
                    &[],
                ),
                &format!("repeat `{m}`"),
            );
        }
    }

    #[test]
    fn test_non_plain_member_names_are_rejected_in_chained_navigation() {
        for m in HOSTILE_MEMBERS {
            // `<base>.<field>.join()` lowers through its own path.
            assert_rejected(
                column_view(&format!("name.`{m}`.join(',')")),
                &format!("join over `{m}`"),
            );
            // `where(...)` / `extension(url)` followed by navigation.
            assert_rejected(
                column_view(&format!("name.where(use = 'official').`{m}`")),
                &format!("where().`{m}`"),
            );
            assert_rejected(
                column_view(&format!("extension('http://x').`{m}`")),
                &format!("extension().`{m}`"),
            );
        }
    }

    #[test]
    fn test_non_plain_type_names_are_rejected() {
        for m in HOSTILE_MEMBERS {
            assert_rejected(
                column_view(&format!("subject.getReferenceKey(`{m}`)")),
                &format!("getReferenceKey(`{m}`)"),
            );
            assert_rejected(
                column_view(&format!("value.ofType(`{m}`)")),
                &format!("ofType(`{m}`)"),
            );
        }
    }

    #[test]
    fn test_plain_backtick_identifiers_still_compile() {
        // Backticks are legitimate for plain names (e.g. keywords), and must
        // keep producing the same SQL as the bare identifier.
        let bare = compile(column_view("name.family")).unwrap();
        let ticked = compile(column_view("`name`.`family`")).unwrap();
        assert_eq!(bare.sql, ticked.sql);
        let bare = compile_pg(column_view("name.family")).unwrap();
        let ticked = compile_pg(column_view("`name`.`family`")).unwrap();
        assert_eq!(bare.sql, ticked.sql);
        // `_birthDate` (a primitive-extension sibling) is a plain identifier.
        let q = compile(column_view("_birthDate")).unwrap();
        assert!(q.sql.contains("'$._birthDate'"), "{}", q.sql);
    }

    // -----------------------------------------------------------------------
    // String literals are inlined through the dialect's string literal;
    // ViewDefinition constants stay bound parameters.
    // -----------------------------------------------------------------------

    use super::super::dialect::test_support::STRING_LITERAL_CASES;

    /// FHIRPath source for the string `value`: `\` and `'` are escaped with a
    /// backslash.
    fn fhirpath_string(value: &str) -> String {
        format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
    }

    fn where_view(path: &str) -> serde_json::Value {
        view_with(json!({"column": [{"path": "id", "name": "id"}]}), &[path])
    }

    #[test]
    fn test_where_string_literals_use_the_dialects_literal() {
        for (value, sqlite, pg) in STRING_LITERAL_CASES {
            let view = where_view(&format!("gender = {}", fhirpath_string(value)));
            let q = compile(view.clone()).unwrap();
            assert!(
                q.sql.contains(&format!("= {sqlite})")),
                "sqlite {value:?}: {}",
                q.sql
            );
            let q = compile_pg(view).unwrap();
            assert!(
                q.sql.contains(&format!("= {pg})")),
                "postgres {value:?}: {}",
                q.sql
            );
        }
    }

    #[test]
    fn test_plain_where_string_literal_sql_is_unchanged() {
        let view = where_view("gender = 'male'");
        let q = compile(view.clone()).unwrap();
        assert!(
            q.sql
                .contains("(json_extract(r.data, '$.gender') = 'male')"),
            "{}",
            q.sql
        );
        let q = compile_pg(view).unwrap();
        assert!(q.sql.contains("(r.data->>'gender' = 'male')"), "{}", q.sql);
    }

    #[test]
    fn test_other_string_literal_sites_use_the_dialects_literal() {
        for (value, sqlite, pg) in STRING_LITERAL_CASES {
            let lit = fhirpath_string(value);
            // `join(sep)` separator.
            let join = column_view(&format!("name.given.join({lit})"));
            // `extension(url)` predicate.
            let extension = column_view(&format!("extension({lit}).value.ofType(string)"));
            // `iif` branches.
            let iif = column_view(&format!("iif(active, {lit}, {lit})"));
            // A string on the left of the comparison, inside a `where(...)`.
            let nested = where_view(&format!("name.where(use = {lit}).exists()"));
            for (label, view) in [
                ("join", join),
                ("extension", extension),
                ("iif", iif),
                ("where()", nested),
            ] {
                let q = compile(view.clone()).unwrap();
                assert!(
                    q.sql.contains(sqlite),
                    "sqlite {label} {value:?}: {}",
                    q.sql
                );
                let q = compile_pg(view).unwrap();
                assert!(q.sql.contains(pg), "postgres {label} {value:?}: {}", q.sql);
            }
        }
    }

    #[test]
    fn test_string_constants_are_bound_not_inlined() {
        for (value, _, _) in STRING_LITERAL_CASES {
            let view = json!({
                "resourceType": "ViewDefinition",
                "resource": "Patient",
                "status": "active",
                "constant": [{"name": "g", "valueString": value}],
                "where": [{"path": "gender = %g"}],
                "select": [{"column": [{"path": "id", "name": "id"}]}]
            });
            for (label, q, placeholder) in [
                ("sqlite", compile(view.clone()).unwrap(), "?3"),
                ("postgres", compile_pg(view.clone()).unwrap(), "$3"),
            ] {
                assert!(
                    q.sql.contains(&format!("= {placeholder})")),
                    "{label} {value:?}: {}",
                    q.sql
                );
                // The value travels as a bound parameter, untouched...
                assert!(
                    matches!(&q.constants[..], [super::super::ir::LitValue::Str(v)] if v == value),
                    "{label} {value:?}: {:?}",
                    q.constants
                );
                // ...and never appears in the SQL text, quoted or not.
                if value.chars().any(char::is_alphabetic) {
                    for form in [value.to_string(), value.replace('\'', "''")] {
                        assert!(!q.sql.contains(&form), "{label} {value:?}: {}", q.sql);
                    }
                }
            }
        }
    }

    #[test]
    fn test_nul_in_a_string_literal_is_rejected() {
        // `\u0000` is a valid FHIRPath string escape; SQL text cannot carry it,
        // and dropping it would change the comparison.
        let view = where_view("gender = 'a\\u0000b'");
        for (label, result) in [
            ("sqlite", compile(view.clone())),
            ("postgres", compile_pg(view)),
        ] {
            match result {
                Err(SofError::Uncompilable { reason }) => {
                    assert!(reason.contains("NUL"), "{label}: {reason}")
                }
                other => panic!("{label}: expected Uncompilable, got {other:?}"),
            }
        }
    }

    // --- Per-column decode modes (#1769) ---

    fn decodes(view: Value) -> Vec<ColumnDecode> {
        compile(view).unwrap().column_decodes
    }

    fn condition_view(columns: Value) -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Condition",
            "status": "active",
            "select": [{"column": columns}]
        })
    }

    #[test]
    fn test_declared_types_set_decode() {
        let d = decodes(condition_view(json!([
            {"name": "a", "path": "id", "type": "string"},
            {"name": "b", "path": "code.coding.first().code", "type": "code"},
            {"name": "c", "path": "active", "type": "boolean"},
            {"name": "d", "path": "id", "type": "integer"},
            {"name": "e", "path": "id", "type": "decimal"},
            {"name": "f", "path": "code", "type": "CodeableConcept"}
        ])));
        assert_eq!(
            d,
            vec![
                ColumnDecode::Text,
                ColumnDecode::Text,
                ColumnDecode::Boolean,
                ColumnDecode::Integer,
                ColumnDecode::Decimal,
                ColumnDecode::Json
            ]
        );
    }

    #[test]
    fn test_collection_column_is_json() {
        let d = decodes(condition_view(json!([
            {"name": "codes", "path": "code.coding.code", "type": "code", "collection": true}
        ])));
        assert_eq!(d, vec![ColumnDecode::Json]);
    }

    #[test]
    fn test_untyped_root_path_is_inferred_from_fhir_schema() {
        let d = decodes(condition_view(json!([
            {"name": "id", "path": "id"},
            {"name": "code", "path": "code.coding.first().code"},
            {"name": "system", "path": "code.coding.first().system"},
            {"name": "cc", "path": "code"}
        ])));
        assert_eq!(
            d,
            vec![
                ColumnDecode::Text,
                ColumnDecode::Text,
                ColumnDecode::Text,
                ColumnDecode::Json
            ]
        );
    }

    #[test]
    fn test_untyped_unresolved_stays_auto() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{
                "forEach": "name",
                "column": [{"name": "family", "path": "family"}]
            }, {
                "column": [
                    {"name": "has_name", "path": "name.exists()"},
                    {"name": "nope", "path": "notAField"}
                ]
            }]
        });
        // `family` resolves through the forEach focus type; the rest can't.
        let d = decodes(view);
        assert_eq!(
            d,
            vec![ColumnDecode::Text, ColumnDecode::Auto, ColumnDecode::Auto]
        );
    }

    #[test]
    fn test_union_merges_decodes_by_position() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"unionAll": [
                {"column": [
                    {"name": "a", "path": "id", "type": "string"},
                    {"name": "b", "path": "id", "type": "string"}
                ]},
                {"column": [
                    {"name": "a", "path": "id", "type": "string"},
                    {"name": "b", "path": "id", "type": "integer"}
                ]}
            ]}]
        });
        assert_eq!(decodes(view), vec![ColumnDecode::Text, ColumnDecode::Auto]);
    }

    #[test]
    fn test_decode_parallels_columns_and_survives_trailing_index() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{
                "forEach": "name[0]",
                "column": [{"name": "family", "path": "family", "type": "string"}]
            }]
        });
        let q = compile(view).unwrap();
        assert_eq!(q.columns.len(), q.column_decodes.len());
        assert_eq!(q.column_decodes, vec![ColumnDecode::Text]);
    }

    #[test]
    fn test_repeat_columns_keep_declared_decode() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "QuestionnaireResponse",
            "status": "active",
            "select": [{"repeat": ["item"], "column": [
                {"name": "linkId", "path": "linkId", "type": "string"},
                {"name": "other", "path": "linkId"}
            ]}]
        });
        // The repeat focus type is resolved, so the untyped column is inferred too.
        assert_eq!(decodes(view), vec![ColumnDecode::Text, ColumnDecode::Text]);
    }

    #[test]
    fn test_untyped_repeating_last_field_stays_auto() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [
                {"name": "given", "path": "name.given"},
                {"name": "first_given", "path": "name.given.first()"},
                {"name": "id", "path": "id"}
            ]}]
        });
        assert_eq!(
            decodes(view),
            vec![ColumnDecode::Auto, ColumnDecode::Auto, ColumnDecode::Text]
        );
    }
}
