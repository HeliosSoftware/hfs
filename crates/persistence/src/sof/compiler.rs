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
use super::emit::{ResourcePredicates, emit_plan_with_predicates};
use super::ir::{LitValue, PlanNode};

/// Appends the final output `LIMIT` that both SQL runners execute.
///
/// Every SQL shape — flat selects, expansions, unions and recursion — ends in
/// its total deterministic `ORDER BY` (the outer wrapper's for unions, the
/// final one for `repeat`), so one `LIMIT n` appended after it caps the
/// output rows to exactly the first `n` rows of the unlimited statement. It
/// is never placed per union branch, and scalar selections keep their
/// intrinsic `LIMIT 1 OFFSET N` inside the projection.
///
/// `None` appends nothing. A limit above `i64::MAX` is not representable as a
/// SQL integer and also appends nothing: the runner's client-side cap alone
/// enforces it. The validated integer is interpolated, not bound — a bound
/// PostgreSQL `Int` parameter is serialized as text.
#[cfg_attr(not(any(feature = "sqlite", feature = "postgres")), allow(dead_code))]
pub(super) fn append_output_limit(sql: &mut String, limit: Option<usize>) {
    if let Some(limit) = limit.and_then(|limit| i64::try_from(limit).ok()) {
        sql.push_str(&format!("\nLIMIT {limit}"));
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

    /// Whether a FHIRPath `where(crit)` criterion — of a `forEach` path or
    /// inside an expression — reads the ENCLOSING iteration's `%rowIndex`, as
    /// the in-process evaluator does. The SQL targets do (#1623). MongoDB
    /// keeps its existing lowering, where `%rowIndex` inside a criterion
    /// resolves from the criterion's own focus alias (a `forEach` path's
    /// criterion reads the current element's `$unwind` index), so its
    /// pipelines are unchanged.
    pub(super) fn pins_where_row_index(self) -> bool {
        match self {
            CompileTarget::Sqlite | CompileTarget::Postgres => true,
            #[cfg(feature = "mongodb")]
            CompileTarget::Mongo => false,
        }
    }

    /// Root of every resource-document navigation the plan contains: the
    /// SQL dialect's [`Dialect::resource_document`] (PostgreSQL reads a
    /// once-detoasted copy), and `r.data` — the root the MongoDB emitter
    /// maps to the stored document — for MongoDB.
    pub(super) fn resource_document(self, dialect: &dyn Dialect) -> &'static str {
        match self {
            CompileTarget::Sqlite | CompileTarget::Postgres => dialect.resource_document(),
            #[cfg(feature = "mongodb")]
            CompileTarget::Mongo => super::dialect::SCANNED_DOCUMENT,
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
    pub constants: Vec<LitValue>,
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
    SqlViewPlan::build(view_json, dialect, fhir_version)?.emit(&ResourcePredicates::none())
}

/// A ViewDefinition lowered to its SQL plan but not yet rendered.
///
/// Plan construction allocates the constant slots `$3..=$(2+constants.len())`
/// (`?N` on SQLite). Splitting it from emission lets a runner allocate its
/// runtime-filter slots once, from [`Self::first_runtime_param`], and have the
/// emitter lower the resulting [`ResourcePredicates`] into every resource scan
/// (union branches, recursive seeds, resource rejoins).
pub(super) struct SqlViewPlan {
    plan: PlanNode,
    constants: Vec<LitValue>,
    dialect: SqlDialect,
}

impl SqlViewPlan {
    /// Builds the plan for `view_json`. Compilation errors surface here,
    /// before a runner performs any I/O.
    pub(super) fn build(
        view_json: &Value,
        dialect: SqlDialect,
        fhir_version: FhirVersion,
    ) -> Result<Self, SofError> {
        let target = match dialect {
            SqlDialect::Sqlite => CompileTarget::Sqlite,
            SqlDialect::Postgres => CompileTarget::Postgres,
        };
        let dial = dialect_for(dialect);
        let (plan, constants) = build_plan(view_json, dial.as_ref(), target, fhir_version)?;
        Ok(Self {
            plan,
            constants,
            dialect,
        })
    }

    /// Resolved `ViewDefinition.constant[]` values, bound from slot 3.
    #[cfg_attr(not(any(feature = "sqlite", feature = "postgres")), allow(dead_code))]
    pub(super) fn constants(&self) -> &[LitValue] {
        &self.constants
    }

    /// First bound-parameter slot free for runtime filters: after
    /// `tenant_id`, `resource_type` and every constant.
    pub(super) fn first_runtime_param(&self) -> usize {
        3 + self.constants.len()
    }

    /// Renders the plan with `predicates` attached to every resource scan.
    ///
    /// # Errors
    ///
    /// Emitter errors, or [`SofError::Backend`] when non-empty `predicates`
    /// do not start at [`Self::first_runtime_param`] (they would alias a
    /// constant slot or leave a gap in the bound parameters).
    pub(super) fn emit(&self, predicates: &ResourcePredicates) -> Result<CompiledQuery, SofError> {
        if predicates.param_count() > 0 && predicates.first_param() != self.first_runtime_param() {
            return Err(SofError::Backend(format!(
                "runtime filter parameters start at slot {} but must start at slot {}",
                predicates.first_param(),
                self.first_runtime_param()
            )));
        }
        let dial = dialect_for(self.dialect);
        let emitted = emit_plan_with_predicates(&self.plan, dial.as_ref(), predicates)?;
        Ok(CompiledQuery {
            sql: emitted.sql,
            columns: emitted.columns,
            column_decodes: emitted.column_decodes,
            constants: self.constants.clone(),
        })
    }
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
            compile_view_definition_dialect(view_json, dialect, fhir_version).map(CompiledView::Sql)
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

    /// The `ORDER BY` that ends `sql` at the top level: the text after the
    /// last `ORDER BY` holds no `LIMIT` and closes no enclosing parenthesis
    /// (so it is not a subquery's or window's ordering).
    fn assert_ends_in_top_level_order_by(sql: &str, case: &str) {
        let at = sql
            .rfind("ORDER BY ")
            .unwrap_or_else(|| panic!("{case}: no ORDER BY\n{sql}"));
        let tail = &sql[at..];
        assert!(
            !tail.contains("LIMIT"),
            "{case}: LIMIT after ORDER BY\n{sql}"
        );
        let mut depth = 0i32;
        for byte in tail.bytes() {
            match byte {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0, "{case}: final ORDER BY is nested\n{sql}");
        }
        assert_eq!(depth, 0, "{case}: unbalanced final ORDER BY\n{sql}");
    }

    /// Every representable limit appends exactly one `LIMIT n` after the
    /// statement's final top-level `ORDER BY`; `None` and limits above
    /// `i64::MAX` leave the unlimited statement unchanged.
    fn assert_one_final_output_limit(view_plan: &SqlViewPlan, case: &str) {
        let unlimited = view_plan.emit(&ResourcePredicates::none()).unwrap().sql;
        assert_ends_in_top_level_order_by(&unlimited, case);
        let limited = |limit: Option<usize>| {
            let mut sql = unlimited.clone();
            append_output_limit(&mut sql, limit);
            sql
        };
        let mut limits = vec![0, 1, 50, 10_000];
        #[cfg(target_pointer_width = "64")]
        limits.push(i64::MAX as usize);
        for limit in limits {
            assert_eq!(
                limited(Some(limit)),
                format!("{unlimited}\nLIMIT {limit}"),
                "{case}: limit {limit}"
            );
        }
        assert_eq!(limited(None), unlimited, "{case}: unlimited");
        #[cfg(target_pointer_width = "64")]
        for oversized in [i64::MAX as usize + 1, usize::MAX] {
            assert_eq!(limited(Some(oversized)), unlimited, "{case}: oversized");
        }
    }

    #[test]
    fn test_every_row_producing_ir_takes_one_final_output_limit() {
        // Flat selects, scalar `[N]` selections, expansions, nullable
        // expansions, flat and expanded unions, and single/multi-path repeat:
        // each ends in a total top-level ORDER BY that takes the output LIMIT.
        let cases = [
            json!({"resource":"Observation","where":[{"path":"status = 'final'"}],
                "select":[{"column":[{"name":"id","path":"id"}]}]}),
            json!({"resource":"Patient","select":[{"column":[
                {"name":"family","path":"name.first().family"},
                {"name":"given","path":"name[0].given[0]"}]}]}),
            json!({"resource":"Patient","select":[{"forEach":"name.given[0]",
                "column":[{"name":"given","path":"$this"}]}]}),
            json!({"resource":"Patient","where":[{"path":"active"}],
                "select":[{"forEach":"name","column":[{"name":"family","path":"family"}]}]}),
            json!({"resource":"Patient","select":[{"forEachOrNull":"name",
                "column":[{"name":"family","path":"family"}]}]}),
            json!({"resource":"Patient","select":[{"unionAll":[
                {"column":[{"name":"id","path":"id"}]},
                {"column":[{"name":"id","path":"id"}]}]}]}),
            json!({"resource":"Patient","select":[{"unionAll":[
                {"forEach":"name","column":[{"name":"family","path":"family"}]},
                {"forEach":"name","column":[{"name":"family","path":"family"}]}]}]}),
            json!({"resource":"QuestionnaireResponse","select":[{"repeat":["item"],
                "column":[{"name":"link_id","path":"linkId"}]}]}),
            json!({"resource":"QuestionnaireResponse","select":[{"repeat":["item","answer.item"],
                "column":[{"name":"link_id","path":"linkId"}]}]}),
            json!({"resource":"QuestionnaireResponse","select":[{"unionAll":[
                {"repeat":["item"],"column":[{"name":"v","path":"linkId"}]},
                {"column":[{"name":"v","path":"id"}]}]}]}),
        ];
        for dialect in [SqlDialect::Postgres, SqlDialect::Sqlite] {
            for view in &cases {
                let case = format!("{dialect:?} {view}");
                let view_plan = SqlViewPlan::build(view, dialect, FhirVersion::default_enabled())
                    .unwrap_or_else(|error| panic!("{case}: {error}"));
                assert_one_final_output_limit(&view_plan, &case);
                // The public façade renders the same unlimited statement.
                let query = view_plan.emit(&ResourcePredicates::none()).unwrap();
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
    fn test_indexed_lateral_under_project_and_filter_takes_one_final_output_limit() {
        use super::super::ir::{Column, LitValue, SqlExpr, SqlType};
        let scan = PlanNode::Scan {
            alias: "r".into(),
            resource_type: "Patient".into(),
        };
        let plan = PlanNode::Project {
            columns: vec![Column {
                name: "v".into(),
                expr: SqlExpr::Lit(LitValue::Str("v".into())),
                collection: false,
                ty: SqlType::Text,
                decode: ColumnDecode::Text,
            }],
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
        for dialect in [SqlDialect::Postgres, SqlDialect::Sqlite] {
            let view_plan = SqlViewPlan {
                plan: plan.clone(),
                constants: Vec::new(),
                dialect,
            };
            assert_one_final_output_limit(&view_plan, &format!("{dialect:?} indexed lateral"));
        }
    }

    #[test]
    fn test_indexed_foreach_scalar_from_chain_keeps_intrinsic_and_final_limit() {
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
        let view_plan =
            SqlViewPlan::build(&view, SqlDialect::Postgres, FhirVersion::default_enabled())
                .unwrap();
        assert_one_final_output_limit(&view_plan, "indexed ScalarFromChain");
        let query = view_plan.emit(&ResourcePredicates::none()).unwrap();
        // The scalar selection keeps its intrinsic cap inside the projection;
        // the output LIMIT is only ever appended after the final ORDER BY.
        assert!(query.sql.contains("LIMIT 1 OFFSET 0"));
        let mut limited = query.sql.clone();
        append_output_limit(&mut limited, Some(50));
        assert_eq!(
            limited.matches("LIMIT 1 OFFSET 0").count(),
            query.sql.matches("LIMIT 1 OFFSET 0").count()
        );
        assert!(limited.ends_with("\nLIMIT 50"));
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

    /// The plan's iteration aliases (outermost first), projected columns
    /// and row filters, each rendered with `{:?}` so `%rowIndex` scopes read
    /// as `RowIndex(<scope>)`.
    struct PlanParts {
        aliases: Vec<String>,
        on_filters: Vec<(String, String)>,
        columns: Vec<(String, String)>,
        filters: Vec<String>,
    }

    fn plan_parts(view: &serde_json::Value) -> PlanParts {
        let (plan, _) = build_plan(
            view,
            &PgDialect,
            CompileTarget::Postgres,
            FhirVersion::default_enabled(),
        )
        .unwrap_or_else(|error| panic!("{view}: {error}"));
        let mut parts = PlanParts {
            aliases: Vec::new(),
            on_filters: Vec::new(),
            columns: Vec::new(),
            filters: Vec::new(),
        };
        let mut node = &plan;
        loop {
            node = match node {
                PlanNode::Project { parent, columns } => {
                    parts.columns = columns
                        .iter()
                        .map(|c| (c.name.clone(), format!("{:?}", c.expr)))
                        .collect();
                    parent
                }
                PlanNode::Filter { parent, predicate } => {
                    parts.filters.push(format!("{predicate:?}"));
                    parent
                }
                PlanNode::LateralUnnest {
                    parent,
                    out_alias,
                    on_filter,
                    ..
                } => {
                    parts.aliases.insert(0, out_alias.clone());
                    if let Some(filter) = on_filter {
                        parts
                            .on_filters
                            .push((out_alias.clone(), format!("{filter:?}")));
                    }
                    parent
                }
                PlanNode::Recurse {
                    parent, out_alias, ..
                } => {
                    parts.aliases.insert(0, out_alias.clone());
                    parent
                }
                _ => break,
            };
        }
        parts
    }

    fn column<'a>(parts: &'a PlanParts, name: &str) -> &'a str {
        &parts
            .columns
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("column {name}"))
            .1
    }

    #[test]
    fn test_iteration_criterion_row_index_lowers_to_the_enclosing_scope() {
        let for_each = |scope: &str| format!("RowIndex(ForEach({scope:?}))");
        let repeat = |scope: &str| format!("RowIndex(Repeat({scope:?}))");
        let top = "RowIndex(Top)";
        let nested = |kind: &str, path: &str| {
            json!({"resource":"Patient","select":[{"forEach":"name",
                "column":[{"name":"outer_i","path":"%rowIndex","type":"integer"}],
                "select":[{kind:path,"column":[{"name":"given","path":"$this"},
                    {"name":"inner_i","path":"%rowIndex","type":"integer"}]}]}]})
        };

        // Indexed iteration under `forEach: "name"`: the criterion (in the
        // selection's value, its `forEachOrNull` empty context and the
        // membership filter) reads the outer ordinal; the selection's own
        // `%rowIndex` column is the singleton's 0.
        for kind in ["forEach", "forEachOrNull"] {
            let parts = plan_parts(&nested(kind, "given[0].where(%rowIndex = 1)"));
            let [outer] = &parts.aliases[..] else {
                panic!("{kind}: one unnest expected: {:?}", parts.aliases)
            };
            assert!(column(&parts, "outer_i").contains(&for_each(outer)));
            let given = column(&parts, "given");
            assert!(given.contains("selection_filter: Some"), "{kind}: {given}");
            assert!(given.contains(&for_each(outer)), "{kind}: {given}");
            assert!(!given.contains(top), "{kind}: {given}");
            let inner_i = column(&parts, "inner_i");
            assert!(
                inner_i.contains(&format!("projection: {top}")),
                "{kind}: {inner_i}"
            );
            assert!(inner_i.contains(&for_each(outer)), "{kind}: {inner_i}");
            match kind {
                "forEach" => {
                    let [membership] = &parts.filters[..] else {
                        panic!("one membership filter: {:?}", parts.filters)
                    };
                    assert!(membership.contains(&for_each(outer)), "{membership}");
                    assert!(!membership.contains(top), "{membership}");
                }
                _ => assert!(parts.filters.is_empty(), "{:?}", parts.filters),
            }
        }

        // Ordinary iteration under `forEach: "name"`: the inner unnest's ON
        // filter reads the outer ordinal, its columns the inner one.
        for kind in ["forEach", "forEachOrNull"] {
            let parts = plan_parts(&nested(kind, "given.where(%rowIndex = 1)"));
            let [outer, inner] = &parts.aliases[..] else {
                panic!("{kind}: two unnests expected: {:?}", parts.aliases)
            };
            let [(on_alias, on_filter)] = &parts.on_filters[..] else {
                panic!("{kind}: one ON filter: {:?}", parts.on_filters)
            };
            assert_eq!(on_alias, inner);
            assert!(on_filter.contains(&for_each(outer)), "{kind}: {on_filter}");
            assert!(!on_filter.contains(&for_each(inner)), "{kind}: {on_filter}");
            assert!(column(&parts, "inner_i").contains(&for_each(inner)));
        }

        // Top level: the enclosing scope is the resource (0).
        for path in ["name[1].where(%rowIndex = 0)", "name.where(%rowIndex = 0)"] {
            let parts = plan_parts(&json!({"resource":"Patient","select":[{"forEach":path,
                "column":[{"name":"i","path":"%rowIndex","type":"integer"}]}]}));
            let criterion = parts
                .filters
                .first()
                .or(parts.on_filters.first().map(|(_, f)| f))
                .unwrap_or_else(|| panic!("{path}: no criterion"));
            assert!(criterion.contains(top), "{path}: {criterion}");
            assert!(!criterion.contains("ForEach"), "{path}: {criterion}");
        }

        // Under `repeat`: the criterion reads the node's repeat index.
        for path in [
            "answer[0].where(%rowIndex = 7)",
            "answer.where(%rowIndex = 7)",
        ] {
            let parts = plan_parts(&json!({"resource":"QuestionnaireResponse",
                "select":[{"repeat":["item"],"select":[{"forEach":path,
                    "column":[{"name":"i","path":"%rowIndex","type":"integer"}]}]}]}));
            let rec = &parts.aliases[0];
            let criterion = parts
                .filters
                .first()
                .or(parts.on_filters.first().map(|(_, f)| f))
                .unwrap_or_else(|| panic!("{path}: no criterion"));
            assert!(criterion.contains(&repeat(rec)), "{path}: {criterion}");
            assert!(!criterion.contains("ForEach"), "{path}: {criterion}");
        }

        // A `where()` inside a column expression keeps the column's scope,
        // not its criterion element's (`w<N>`).
        let parts = plan_parts(&json!({"resource":"Patient","select":[{"forEach":"name",
            "column":[{"name":"picked","path":"given.where(%rowIndex = 1).exists()",
                "type":"boolean"}]}]}));
        let [outer] = &parts.aliases[..] else {
            panic!("one unnest expected: {:?}", parts.aliases)
        };
        let picked = column(&parts, "picked");
        assert!(picked.contains(&for_each(outer)), "{picked}");
        assert!(!picked.contains("ForEach(\"w"), "{picked}");
    }

    #[test]
    fn test_sql_view_plan_allocates_runtime_slots_after_constants() {
        let view = json!({"resource":"Patient",
            "constant":[{"name":"g","valueString":"male"},{"name":"f","valueString":"x"}],
            "where":[{"path":"gender = %g"},{"path":"name.family.first() != %f"}],
            "select":[{"column":[{"name":"id","path":"id"}]}]});
        for dialect in [SqlDialect::Sqlite, SqlDialect::Postgres] {
            let plan = SqlViewPlan::build(&view, dialect, FhirVersion::default_enabled()).unwrap();
            assert_eq!(plan.constants().len(), 2);
            assert_eq!(plan.first_runtime_param(), 5);
            // Without runtime predicates the output is the public compilation.
            let public =
                compile_view_definition_dialect(&view, dialect, FhirVersion::default_enabled())
                    .unwrap();
            let none = plan.emit(&ResourcePredicates::none()).unwrap();
            assert_eq!(none.sql, public.sql);
            assert_eq!(none.columns, public.columns);
            // Runtime predicates must start right after the constants.
            for wrong in [3, 4, 6] {
                let predicates = ResourcePredicates::new(wrong, 1, vec!["1=1".into()]);
                assert!(
                    matches!(plan.emit(&predicates), Err(SofError::Backend(_))),
                    "{dialect:?} slot {wrong}"
                );
            }
            let predicates = ResourcePredicates::new(5, 1, vec!["r.id IS NOT NULL".into()]);
            let emitted = plan.emit(&predicates).unwrap();
            assert!(emitted.sql.contains("\n  AND r.id IS NOT NULL\nORDER BY"));
        }
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
        assert!(q.sql.contains("rdoc.doc->>'id' AS \"id\""), "{}", q.sql);
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
            q.sql
                .contains("coalesce(rdoc.doc#>>'{subject,0,reference}'"),
            "{}",
            q.sql
        );
        assert!(
            q.sql.contains("rdoc.doc#>>'{subject,reference}'"),
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
                .contains("JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(rdoc.doc->'name') = 'array' THEN rdoc.doc->'name' WHEN jsonb_typeof(rdoc.doc->'name') IS NOT NULL THEN jsonb_build_array(rdoc.doc->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE"),
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
                "LEFT JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(rdoc.doc->'name') = 'array' THEN rdoc.doc->'name' WHEN jsonb_typeof(rdoc.doc->'name') IS NOT NULL THEN jsonb_build_array(rdoc.doc->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE"
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
        assert!(q.sql.contains("rdoc.doc->>'id' AS \"id\""), "{}", q.sql);
        assert!(
            q.sql.contains("fe.value->>'family' AS \"family\""),
            "{}",
            q.sql
        );
        assert!(
            q.sql
                .contains("JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'name') = 'array' THEN COALESCE(rdoc.doc, r.data)->'name' WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'name') IS NOT NULL THEN jsonb_build_array(COALESCE(rdoc.doc, r.data)->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE"),
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
