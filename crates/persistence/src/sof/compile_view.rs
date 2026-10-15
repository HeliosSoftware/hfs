//! ViewDefinition JSON → [`PlanNode`] compiler.
//!
//! Walks the SoF `select` tree producing a plan tree rooted in a
//! [`PlanNode::Scan`] over `resources`. Per-clause logic:
//!
//! - Plain `select.column[]` → column projections off the current focus.
//! - `forEach`/`forEachOrNull` → [`PlanNode::LateralUnnest`] over the parent.
//! - `repeat` → [`PlanNode::Recurse`] seeded from the focus of the enclosing
//!   clause (the resource, a `forEach` element or an outer `repeat` node).
//! - Nested `select[]` → contributes additional columns under the parent's
//!   focus (or extends the row source if it has its own `forEach` / `repeat`).
//! - `unionAll[]` → distributed to the root by [`super::normalize_union`],
//!   then one [`PlanNode::Project`] per alternative under a
//!   [`PlanNode::Union`].
//! - Top-level `where[].path` → [`PlanNode::Filter`] applied to the root scan.
//!
//! Stages 4–5 add chained-call collection threading, repeat:, and boundary
//! functions.

use helios_fhir::FhirVersion;
use helios_sof::ConstantValue;
use serde_json::Value;

use crate::core::sof_runner::SofError;

use super::compile_path::{
    CompileEnv, Constant, compile_fhirpath_ast, compile_fhirpath_expr, fhir_type_of_path,
    split_for_each_where, walk_fhir_type, walk_fhir_type_from,
};
use super::decode::ColumnDecode;
use super::dialect::{pg_key_literal, pg_path_array_literal, sqlite_json_path_literal};
use super::ir::{Column, LitValue, PathStep, PlanNode, SqlExpr, SqlType};

const ROOT_ALIAS: &str = "r";
const FOREACH_ALIAS_PREFIX: &str = "fe";

/// Build a plan tree for the given ViewDefinition JSON.
///
/// Returns the plan plus the resolved `ViewDefinition.constant[]` values in
/// the order they were bound to SQL parameter slots. The runners append them
/// after `tenant_id` / `resource_type`.
///
/// The `dialect` parameter is currently used only by the trailing-`[N]`
/// forEach lowering ([`build_degenerate_chain_sql`]) which builds a SQL
/// chain string at compile time. Other features lower through the
/// dialect-aware emit path.
pub fn build_plan(
    view_json: &Value,
    dialect: &dyn super::dialect::Dialect,
    target: super::compiler::CompileTarget,
    fhir_version: FhirVersion,
) -> Result<(PlanNode, Vec<LitValue>), SofError> {
    let resource_type = view_json
        .get("resource")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            SofError::InvalidViewDefinition("ViewDefinition.resource is required".to_string())
        })?
        .to_string();

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

    // Reject an unknown or incompatible `ansi/type` column tag up front, for
    // every compile target (the in-DB emitters render values through the
    // declared FHIR type, so the tag is validated here rather than honoured).
    helios_sof::resolve_columns(view_json).map_err(lift_sof_error)?;

    let mut env = CompileEnv::new_for_resource(
        format!("{ROOT_ALIAS}.data"),
        resource_type.clone(),
        fhir_version,
    );
    populate_constants(view_json, &mut env)?;

    // Top-level where filters apply to the resource row, before any unnest.
    let mut where_predicates: Vec<SqlExpr> = Vec::new();
    if let Some(wheres) = view_json.get("where").and_then(|v| v.as_array()) {
        for w in wheres {
            if let Some(path) = w.get("path").and_then(|v| v.as_str()) {
                // SoF v2 spec: where[].path must resolve to a boolean. A
                // plain field-navigation expression with no operators or
                // function calls is provably non-boolean — reject at
                // compile time so views like `where: [{path: "name.family"}]`
                // don't silently misbehave.
                if where_path_is_provably_non_boolean(path) {
                    return Err(SofError::InvalidViewDefinition(format!(
                        "ViewDefinition.where[].path '{path}' must resolve to a \
                         boolean (got a plain navigation expression)"
                    )));
                }
                let pred = compile_fhirpath_expr(path, &mut env)?;
                where_predicates.push(pred);
            }
        }
    }

    let scan = PlanNode::Scan {
        alias: ROOT_ALIAS.to_string(),
        resource_type: resource_type.clone(),
        filter: Default::default(),
    };
    let mut root_plan = scan;
    for pred in where_predicates {
        root_plan = PlanNode::Filter {
            parent: Box::new(root_plan),
            predicate: pred,
        };
    }

    let mut alias_seq = AliasSeq::new();
    let plan = plan_clause_list(
        selects,
        &root_plan,
        &format!("{ROOT_ALIAS}.data"),
        &mut env,
        &mut alias_seq,
        dialect,
        target,
    )
    .and_then(ensure_project)?;
    Ok((plan, env.param_bindings))
}

/// Reads `ViewDefinition.constant[]` and populates `env.constants` with typed
/// values. Each entry must have a `name` and exactly one `valueX` field per
/// the SoF v2 spec. Delegates the field walk to
/// [`helios_sof::parse_constant_from_json`] so the spec field list lives in
/// one place; here we just lift the neutral [`ConstantValue`] into the
/// compiler's [`LitValue`] (which keeps dates/times as text — FHIRPath
/// `@`/`@T` prefixing only matters for the in-process evaluator).
fn populate_constants(view_json: &Value, env: &mut CompileEnv) -> Result<(), SofError> {
    let Some(constants) = view_json.get("constant").and_then(|v| v.as_array()) else {
        return Ok(());
    };
    for c in constants {
        let (name, value) = helios_sof::parse_constant_from_json(c).map_err(lift_sof_error)?;
        env.constants.insert(
            name,
            Constant {
                value: lit_value_from_constant(value),
                bound_to: None,
            },
        );
    }
    Ok(())
}

/// Lowers a neutral [`ConstantValue`] into the in-DB compiler's [`LitValue`].
/// All FHIR string-shaped primitives collapse to `Str`; the date/time/instant
/// families keep their lexical form (no `@`-prefixing — SQL parameter binding
/// takes plain ISO 8601 strings).
fn lit_value_from_constant(value: ConstantValue) -> LitValue {
    match value {
        ConstantValue::String(s)
        | ConstantValue::Code(s)
        | ConstantValue::Identifier(s)
        | ConstantValue::Base64Binary(s)
        | ConstantValue::Markdown(s)
        | ConstantValue::Date(s)
        | ConstantValue::DateTime(s)
        | ConstantValue::Time(s)
        | ConstantValue::Instant(s) => LitValue::Str(s),
        ConstantValue::Boolean(b) => LitValue::Bool(b),
        ConstantValue::Integer(i)
        | ConstantValue::PositiveInt(i)
        | ConstantValue::UnsignedInt(i)
        | ConstantValue::Integer64(i) => LitValue::Int(i),
        ConstantValue::Decimal(s) => LitValue::Decimal(s),
    }
}

/// Maps `helios_sof::SofError` (raised by the shared SoF spec parser) onto
/// the persistence crate's local `SofError`. Only `InvalidViewDefinition`
/// is reachable from the parser today; other variants pass through as the
/// same flavour to keep the 422-mapping consistent.
fn lift_sof_error(e: helios_sof::SofError) -> SofError {
    match e {
        helios_sof::SofError::InvalidViewDefinition(msg) => SofError::InvalidViewDefinition(msg),
        other => SofError::InvalidViewDefinition(other.to_string()),
    }
}

/// Walks a list of select clauses sharing a parent row source and builds the
/// plan: a single `Project` when the tree has no `unionAll`, otherwise a root
/// `Union` with one `Project` per alternative.
///
/// `unionAll` may sit at any depth, but the IR only models it at the root, so
/// [`super::normalize_union::distribute_union_all`] first rewrites the tree
/// into `unionAll`-free alternatives (the union is distributed over its
/// enclosing `forEach` / `repeat`). Each alternative is then planned as a
/// plain clause list.
fn plan_clause_list(
    clauses: &[Value],
    parent_plan: &PlanNode,
    parent_focus: &str,
    env: &mut CompileEnv,
    alias_seq: &mut AliasSeq,
    dialect: &dyn super::dialect::Dialect,
    target: super::compiler::CompileTarget,
) -> Result<PlanNode, SofError> {
    let alternatives = super::normalize_union::distribute_union_all(clauses)?;
    let mut projects: Vec<PlanNode> = Vec::with_capacity(alternatives.len());
    for alternative in &alternatives {
        projects.push(plan_alternative(
            alternative,
            parent_plan,
            parent_focus,
            env,
            alias_seq,
            dialect,
            target,
        )?);
    }
    if projects.len() == 1 {
        return Ok(projects.remove(0));
    }
    Ok(PlanNode::Union(projects))
}

/// Plans one `unionAll`-free alternative: sibling clauses contribute columns
/// and row-source steps (unnests / recursive descents) in document order, so
/// siblings form a cross product and a nested clause iterates relative to the
/// focus of its enclosing clause.
fn plan_alternative(
    clauses: &[Value],
    parent_plan: &PlanNode,
    parent_focus: &str,
    env: &mut CompileEnv,
    alias_seq: &mut AliasSeq,
    dialect: &dyn super::dialect::Dialect,
    target: super::compiler::CompileTarget,
) -> Result<PlanNode, SofError> {
    let mut columns: Vec<Column> = Vec::new();
    let mut steps: Vec<IterStep> = Vec::new();
    let mut filters: Vec<SqlExpr> = Vec::new();

    for clause in clauses {
        let parts =
            read_clause_columns_and_iter(clause, parent_focus, env, alias_seq, dialect, target)?;
        columns.extend(parts.columns);
        steps.extend(parts.steps);
        filters.extend(parts.extra_filters);
    }

    if columns.is_empty() {
        return Err(SofError::InvalidViewDefinition(
            "no columns found in select clauses".to_string(),
        ));
    }
    let plan = apply_filters(apply_steps(parent_plan.clone(), steps), filters);
    Ok(PlanNode::Project {
        parent: Box::new(plan),
        columns,
    })
}

/// One LATERAL unnest step in the chain extending a parent plan.
#[derive(Debug, Clone)]
struct UnnestStep {
    source: SqlExpr,
    out_alias: String,
    left_join: bool,
    /// Optional filter applied in the JOIN ON clause — used by forEach paths
    /// that contain a `where(crit)` (e.g. `forEach: "name.where(use=X)"`).
    /// The predicate is pre-lowered against `<out_alias>.value`.
    on_filter: Option<SqlExpr>,
    /// When set, restricts the unnest to the Nth element (zero-based) of the
    /// flattened collection. Used for forEach paths ending in `[N]` —
    /// FHIRPath indexes the flattened result, not each array crossing.
    flat_index: Option<i64>,
}

/// One `repeat:` recursive descent — produces a recursive-CTE row source
/// rather than a chain of lateral unnests.
#[derive(Debug, Clone)]
struct RecurseInfo {
    /// Step paths to walk on each iteration, relative to the seed focus for
    /// the first level and to `<alias>.node` for subsequent levels.
    step_paths: Vec<super::ir::JsonPath>,
    /// Alias scope the descent is seeded from: the focus of the enclosing
    /// clause (`r.data`, `fe.value` or an outer `rec_N.node`).
    seed_root: String,
    /// Alias of the recursive CTE (also the column alias for `node`).
    out_alias: String,
}

/// One step extending a clause's row source. Steps are kept in document
/// order because each step's source refers to the focus produced by the
/// steps before it.
#[derive(Debug, Clone)]
enum IterStep {
    Unnest(UnnestStep),
    Recurse(RecurseInfo),
}

/// Output of [`read_clause_columns_and_iter`]: the columns this clause
/// contributes plus the unnests / recursive descents it adds to the row
/// source, in document order.
#[derive(Debug)]
struct ClauseParts {
    columns: Vec<Column>,
    steps: Vec<IterStep>,
    /// Extra row filters applied as `Filter(parent, predicate)` over the
    /// clause's row source. Set by trailing-`[N]` forEach lowering (in this
    /// clause or a nested one) to drop resources whose flattened chain
    /// returns fewer than `N+1` elements.
    extra_filters: Vec<SqlExpr>,
}

/// Reads a single clause (already free of `unionAll`, see
/// [`super::normalize_union`]): its `forEach[OrNull]` / `repeat`, `column[]`,
/// and any nested `select[]` clauses. Nested clauses contribute columns at
/// the same focus (or extend the row source if they themselves iterate).
fn read_clause_columns_and_iter(
    clause: &Value,
    parent_focus: &str,
    env: &mut CompileEnv,
    alias_seq: &mut AliasSeq,
    dialect: &dyn super::dialect::Dialect,
    target: super::compiler::CompileTarget,
) -> Result<ClauseParts, SofError> {
    // The IR models `unionAll` only at the plan root; the normalization pass
    // removes every other one. Reaching one here would silently drop its
    // branches.
    if clause.get("unionAll").is_some() {
        return Err(SofError::Uncompilable {
            reason: "unionAll must be normalized before clause planning".to_string(),
        });
    }
    // `repeat:` is mutually exclusive with `forEach`/`forEachOrNull`.
    if let Some(repeat) = clause.get("repeat").and_then(|v| v.as_array()) {
        if repeat.is_empty() {
            return Err(SofError::InvalidViewDefinition(
                "ViewDefinition select.repeat must contain at least one path".to_string(),
            ));
        }
        if clause.get("forEach").is_some() || clause.get("forEachOrNull").is_some() {
            return Err(SofError::Uncompilable {
                reason: "select.repeat combined with forEach is not yet supported".to_string(),
            });
        }
        let mut step_paths: Vec<super::ir::JsonPath> = Vec::with_capacity(repeat.len());
        for p in repeat {
            let s = p.as_str().ok_or_else(|| {
                SofError::InvalidViewDefinition("select.repeat entries must be strings".to_string())
            })?;
            let prev_root = env.root_alias.clone();
            env.root_alias = parent_focus.to_string();
            let expr = compile_fhirpath_expr(s, env)?;
            env.root_alias = prev_root;
            match expr {
                SqlExpr::JsonPath { path, .. } => step_paths.push(path),
                _ => {
                    return Err(SofError::Uncompilable {
                        reason: format!("repeat path '{s}' must be a simple JSON path"),
                    });
                }
            }
        }
        let alias = alias_seq.next_recurse();
        let focus = format!("{alias}.node");
        if let Some(node_type) = repeat_node_type(parent_focus, &step_paths, env) {
            env.focus_types.insert(focus.clone(), node_type);
        }
        let mut columns = read_columns(clause, &focus, env)?;
        // Nested `select[]` under `repeat:` may add columns at the recursive
        // node focus AND/OR extend the row source via a forEach or another
        // repeat (e.g. `repeat:[item]` with a nested `forEach: "answer"`
        // projects answer rows). Each nested step is hoisted onto the plan
        // after this descent, in document order, and resolves against the
        // node focus.
        let mut extra_filters: Vec<SqlExpr> = Vec::new();
        let mut steps: Vec<IterStep> = vec![IterStep::Recurse(RecurseInfo {
            step_paths,
            seed_root: parent_focus.to_string(),
            out_alias: alias,
        })];
        if let Some(nested) = clause.get("select").and_then(|v| v.as_array()) {
            for sub in nested {
                let sub_parts =
                    read_clause_columns_and_iter(sub, &focus, env, alias_seq, dialect, target)?;
                extra_filters.extend(sub_parts.extra_filters);
                steps.extend(sub_parts.steps);
                columns.extend(sub_parts.columns);
            }
        }
        return Ok(ClauseParts {
            columns,
            steps,
            extra_filters,
        });
    }

    let for_each_expr = clause
        .get("forEach")
        .and_then(|v| v.as_str())
        .map(String::from);
    let for_each_or_null_expr = clause
        .get("forEachOrNull")
        .and_then(|v| v.as_str())
        .map(String::from);

    let iter_path_src = for_each_expr.or(for_each_or_null_expr.clone());
    let is_left_join = for_each_or_null_expr.is_some();

    let (unnests, focus): (Vec<UnnestStep>, String) = if let Some(src) = iter_path_src {
        // Detect a trailing `where(crit)` on the forEach path
        // (`forEach: "name.where(use = X)"`), optionally followed by
        // `.first()` / `[N]`. Without an index the criterion is lifted into
        // the JOIN ON clause of the last lateral unnest so the iteration
        // skips non-matching elements (and `forEachOrNull` keeps left-join
        // semantics — preserving outer rows when no element matches). With
        // an index it filters the flattened chain before indexing.
        let for_each_where = split_for_each_where(&src)?;
        let prev_root = env.root_alias.clone();
        env.root_alias = parent_focus.to_string();
        let path_expr = match &for_each_where {
            Some(fw) => compile_fhirpath_ast(&fw.base, env),
            None => compile_fhirpath_expr(&src, env),
        };
        env.root_alias = prev_root;
        let path = match path_expr? {
            SqlExpr::JsonPath { path, .. } => path,
            _ => {
                return Err(SofError::Uncompilable {
                    reason: format!("forEach path '{src}' must be a simple JSON path"),
                });
            }
        };
        let (where_crit, where_index) = match for_each_where {
            Some(fw) => (Some(fw.criterion), fw.index),
            None => (None, None),
        };
        // FHIRPath `[N]` indexes the flattened collection result, not each
        // individual array crossing. SQLite forbids correlated subqueries in
        // FROM, so trailing-Index forEach paths short-circuit into a
        // *degenerate* iteration: no unnest in the FROM, each column wrapped
        // in a correlated `ScalarFromChain` subquery in the SELECT. A
        // `where(crit).first()` / `where(crit)[N]` path indexes the filtered
        // chain the same way.
        let (chain_path, trailing_index) = match (where_index, path.0.last()) {
            (Some(n), _) if !path.is_empty() => (path.clone(), Some(n)),
            (None, Some(super::ir::PathStep::Index(n))) if path.0.len() > 1 => (
                super::ir::JsonPath(path.0[..path.0.len() - 1].to_vec()),
                Some(*n),
            ),
            _ => (path.clone(), None),
        };
        // SQL targets lower trailing-`[N]` forEach into a correlated
        // `ScalarFromChain` subquery (SQLite forbids correlated FROM
        // subqueries). Targets without that constraint — MongoDB — instead
        // fall through to the normal unnest path below, which tags the last
        // unnest with `flat_index` for the emitter to lower to `$arrayElemAt`.
        let has_nested_select = clause
            .get("select")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());
        if let Some(idx) = trailing_index
            && target.supports_correlated_from_subqueries()
            && !has_nested_select
        {
            let segments = split_path_into_segments(&chain_path);
            let (chain_sql, deepest_alias) =
                build_degenerate_chain_sql(&segments, parent_focus, alias_seq, dialect);
            let column_focus = format!("{deepest_alias}.value");
            if let Some((ty, _)) = walk_fhir_type(parent_focus, &chain_path, env) {
                env.focus_types.insert(column_focus.clone(), ty);
            }
            let filter = match &where_crit {
                Some(crit) => {
                    let prev_root = env.root_alias.clone();
                    env.root_alias = column_focus.clone();
                    let pred = compile_fhirpath_ast(crit, env);
                    env.root_alias = prev_root;
                    Some(Box::new(pred?))
                }
                None => None,
            };
            let raw_columns = read_columns(clause, &column_focus, env)?;
            // Wrap every column in a correlated scalar subquery. The
            // outer SELECT sees one row per resource; the column projects
            // the [N]-th element of the flattened chain (or NULL).
            let columns: Vec<Column> = raw_columns
                .into_iter()
                .map(|c| Column {
                    name: c.name,
                    expr: SqlExpr::ScalarFromChain {
                        chain_sql: chain_sql.clone(),
                        projection: Box::new(c.expr),
                        filter: filter.clone(),
                        offset: idx,
                    },
                    collection: c.collection,
                    ty: c.ty,
                    decode: c.decode,
                })
                .collect();
            // For `forEach` (not `forEachOrNull`), an empty chain means
            // the resource produces NO row. Surface that as a per-branch
            // EXISTS filter — wraps the branch's plan with `Filter(EXISTS
            // (SELECT 1 FROM <chain> LIMIT 1 OFFSET <idx>))`.
            let extra_filters = if is_left_join {
                Vec::new()
            } else {
                let has_element = SqlExpr::UnaryOp {
                    op: super::ir::UnaryOp::IsNotNull,
                    inner: Box::new(SqlExpr::ScalarFromChain {
                        chain_sql: chain_sql.clone(),
                        projection: Box::new(SqlExpr::Lit(LitValue::Int(1))),
                        filter,
                        offset: idx,
                    }),
                };
                if parent_focus.starts_with("r.data") {
                    vec![has_element]
                } else {
                    // Below an iteration scope: a NULL scope means an enclosing
                    // `forEachOrNull` missed, whose null row survives whatever
                    // is nested in it (a present scope without the element
                    // still drops the row).
                    vec![SqlExpr::BinOp {
                        op: super::ir::BinOp::Or,
                        lhs: Box::new(has_element),
                        rhs: Box::new(SqlExpr::UnaryOp {
                            op: super::ir::UnaryOp::IsNull,
                            inner: Box::new(SqlExpr::JsonPath {
                                root: parent_focus.to_string(),
                                path: super::ir::JsonPath(Vec::new()),
                            }),
                        }),
                    }]
                }
            };
            return Ok(ClauseParts {
                columns,
                steps: Vec::new(),
                extra_filters,
            });
        }
        // FHIRPath flattens through array boundaries automatically — emit
        // one lateral unnest per `Field` step so `forEach: "contact.telecom"`
        // produces one row per inner element. `Index` steps stay attached to
        // the prior segment as plain navigation.
        let mut unnests: Vec<UnnestStep> = Vec::new();
        let mut focus = parent_focus.to_string();
        // Trailing-`[N]` (Mongo fall-through): `forEach: "contact.telecom[0]"`
        // selects element N of the FLATTENED `contact.telecom` collection, not
        // per-contact. Keep the full multi-field path in a single unnest so the
        // emitter flattens before indexing (splitting into per-field unnests
        // would apply `[N]` per parent element instead). The trailing `[N]` is
        // dropped from the unnest path (`chain_path`) and applied as
        // `flat_index` — otherwise it would be indexed twice. A `where(crit)`
        // before the index filters the flattened collection first (the
        // criterion rides on `on_filter`, rooted at the unnest's alias).
        if let Some(n) = trailing_index
            && target.supports_correlated_from_subqueries()
        {
            // SQL targets with a nested `select`: the degenerate scalar-subquery
            // lowering cannot host nested iteration, so unnest a one-element
            // array holding the N-th flattened element instead; nested clauses
            // then resolve against it like any `forEach` element.
            let segments = split_path_into_segments(&chain_path);
            let (chain_sql, deepest_alias) =
                build_degenerate_chain_sql(&segments, parent_focus, alias_seq, dialect);
            let leaf_focus = format!("{deepest_alias}.value");
            if let Some((ty, _)) = walk_fhir_type(parent_focus, &chain_path, env) {
                env.focus_types.insert(leaf_focus.clone(), ty);
            }
            let filter = match &where_crit {
                Some(crit) => {
                    let prev_root = env.root_alias.clone();
                    env.root_alias = leaf_focus.clone();
                    let pred = compile_fhirpath_ast(crit, env);
                    env.root_alias = prev_root;
                    Some(Box::new(pred?))
                }
                None => None,
            };
            let alias = alias_seq.next();
            let focus = format!("{alias}.value");
            if let Some((ty, _)) = walk_fhir_type(parent_focus, &chain_path, env) {
                env.focus_types.insert(focus.clone(), ty);
            }
            (
                vec![UnnestStep {
                    source: SqlExpr::ChainArray {
                        chain_sql,
                        leaf_alias: deepest_alias,
                        filter,
                        offset: Some(n),
                        scope_root: parent_focus.to_string(),
                    },
                    out_alias: alias,
                    left_join: is_left_join,
                    on_filter: None,
                    flat_index: None,
                }],
                focus,
            )
        } else if let Some(n) = trailing_index {
            let alias = alias_seq.next();
            let focus = format!("{alias}.value");
            if let Some((ty, _)) = walk_fhir_type(parent_focus, &chain_path, env) {
                env.focus_types.insert(focus.clone(), ty);
            }
            let on_filter = match &where_crit {
                Some(crit) => {
                    let prev_root = env.root_alias.clone();
                    env.root_alias = focus.clone();
                    let pred = compile_fhirpath_ast(crit, env);
                    env.root_alias = prev_root;
                    Some(pred?)
                }
                None => None,
            };
            (
                vec![UnnestStep {
                    source: SqlExpr::JsonPath {
                        root: parent_focus.to_string(),
                        path: chain_path,
                    },
                    out_alias: alias,
                    left_join: is_left_join,
                    on_filter,
                    flat_index: Some(n),
                }],
                focus,
            )
        } else {
            let segments = split_path_into_segments(&chain_path);
            // `forEachOrNull` over a multi-step path yields its null row only
            // when the whole FLATTENED collection is empty. A LEFT JOIN per
            // step would add a null row for every intermediate element that
            // has no children (and an INNER JOIN on the intermediates drops
            // the resource), so the chain collapses to a single unnest over
            // the flattened collection: SQL targets aggregate it into one
            // JSON array (`ChainArray`); MongoDB flattens a multi-field unnest
            // path itself.
            let flatten = is_left_join && segments.len() > 1;
            // Each step: its path off the running focus, or the prebuilt
            // flattened source when the chain collapses to one step.
            let steps: Vec<(Option<SqlExpr>, Vec<PathStep>)> = if flatten {
                let source = if target.supports_correlated_from_subqueries() {
                    let (chain_sql, leaf_alias) =
                        build_degenerate_chain_sql(&segments, parent_focus, alias_seq, dialect);
                    SqlExpr::ChainArray {
                        chain_sql,
                        leaf_alias,
                        filter: None,
                        offset: None,
                        scope_root: parent_focus.to_string(),
                    }
                } else {
                    SqlExpr::JsonPath {
                        root: parent_focus.to_string(),
                        path: chain_path.clone(),
                    }
                };
                vec![(Some(source), chain_path.0.clone())]
            } else {
                segments.into_iter().map(|seg| (None, seg.0)).collect()
            };
            let last_idx = steps.len().saturating_sub(1);
            for (i, (prebuilt, seg_steps)) in steps.into_iter().enumerate() {
                let alias = alias_seq.next();
                let source = prebuilt.unwrap_or_else(|| SqlExpr::JsonPath {
                    root: focus.clone(),
                    path: super::ir::JsonPath(seg_steps.clone()),
                });
                // Compile the trailing `where(crit)` filter against the LAST
                // unnest's iteration alias, so `name.where(use=X)` filters the
                // expanded `name` rows.
                let on_filter = match (&where_crit, i == last_idx) {
                    (Some(crit), true) => {
                        let prev_root = env.root_alias.clone();
                        env.root_alias = format!("{alias}.value");
                        let pred = compile_fhirpath_ast(crit, env);
                        env.root_alias = prev_root;
                        Some(pred?)
                    }
                    _ => None,
                };
                unnests.push(UnnestStep {
                    source,
                    out_alias: alias.clone(),
                    // A forEachOrNull chain is a single (possibly flattened)
                    // step here, so only that last step is a LEFT JOIN.
                    left_join: is_left_join && i == last_idx,
                    on_filter,
                    flat_index: None,
                });
                if let Some((ty, _)) = walk_fhir_type(&focus, &super::ir::JsonPath(seg_steps), env)
                {
                    env.focus_types.insert(format!("{alias}.value"), ty);
                }
                focus = format!("{alias}.value");
            }
            (unnests, focus)
        }
    } else {
        (Vec::new(), parent_focus.to_string())
    };

    let mut steps: Vec<IterStep> = unnests.into_iter().map(IterStep::Unnest).collect();
    let mut columns = read_columns(clause, &focus, env)?;
    let mut extra_filters: Vec<SqlExpr> = Vec::new();

    // Nested select clauses: each contributes additional columns under the
    // current focus. If a nested clause iterates (forEach or repeat) we
    // extend the row-source chain, relative to this clause's focus.
    if let Some(nested) = clause.get("select").and_then(|v| v.as_array()) {
        for sub in nested {
            let sub_parts =
                read_clause_columns_and_iter(sub, &focus, env, alias_seq, dialect, target)?;
            steps.extend(sub_parts.steps);
            columns.extend(sub_parts.columns);
            extra_filters.extend(sub_parts.extra_filters);
        }
    }

    Ok(ClauseParts {
        columns,
        steps,
        extra_filters,
    })
}

/// Reads the `column[]` array for a clause, lowering each path under `focus`.
fn read_columns(
    clause: &Value,
    focus: &str,
    env: &mut CompileEnv,
) -> Result<Vec<Column>, SofError> {
    let columns = match clause.get("column").and_then(|v| v.as_array()) {
        Some(cols) if !cols.is_empty() => cols,
        _ => return Ok(Vec::new()),
    };

    let prev_root = env.root_alias.clone();
    env.root_alias = focus.to_string();

    let mut out = Vec::with_capacity(columns.len());
    for col in columns {
        let path = col.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            SofError::InvalidViewDefinition("column.path is required".to_string())
        })?;
        let name = col.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
            SofError::InvalidViewDefinition("column.name is required".to_string())
        })?;
        let collection_opt = col.get("collection").and_then(|v| v.as_bool());
        let collection = collection_opt.unwrap_or(false);
        // SoF v2 spec: when `collection: false` is EXPLICITLY declared, the
        // path MUST yield at most one value. Without FHIR schema we can't
        // verify cardinality precisely, but a multi-Field path through
        // commonly-multi-valued FHIR root fields is a strong signal — reject
        // those at compile time so the validator/conformance test passes.
        if collection_opt == Some(false)
            && path_likely_multi_valued(path, &env.resource_type, env.fhir_version)
        {
            return Err(SofError::InvalidViewDefinition(format!(
                "column '{}' declares `collection: false` but path '{}' may yield \
                 multiple values; declare `collection: true` or pick a single element",
                col.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                path
            )));
        }

        // Make the column's declared type visible to function-call lowering
        // (currently used by `lowBoundary()` / `highBoundary()` to pick
        // decimal vs. date/dateTime/time semantics).
        let column_type = col.get("type").and_then(|v| v.as_str()).map(String::from);
        let prev_type_hint = env.column_type_hint.take();
        env.column_type_hint = column_type.clone();
        let expr_result = compile_fhirpath_expr(path, env);
        env.column_type_hint = prev_type_hint;
        let expr = expr_result?;

        let mut ty = column_type_from_hint(column_type.as_deref());
        let decode = match ColumnDecode::from_declared(column_type.as_deref(), collection) {
            ColumnDecode::Auto => {
                let inferred = infer_decode(&expr, env);
                // An untyped boolean-valued expression (`true`, `a = b`,
                // `x.exists()`) is projected like a `type: boolean` column, so
                // both dialects emit `'true'`/`'false'` rather than SQLite's
                // 1/0 or PostgreSQL's native boolean.
                if is_boolean_valued(&expr) {
                    ty = SqlType::Boolean;
                }
                inferred
            }
            declared => declared,
        };
        // For `collection: true` columns, swap the scalar projection for a
        // [`SqlExpr::CollectionAgg`] over the same path. Only paths that
        // lower to a plain `JsonPath` qualify — anything more complex
        // (where(), join(), etc.) keeps its scalar form.
        let final_expr = if collection {
            match expr {
                SqlExpr::JsonPath { root, path } => SqlExpr::CollectionAgg { root, path },
                other => other,
            }
        } else {
            expr
        };
        out.push(Column {
            name: name.to_string(),
            expr: final_expr,
            collection: false, // emit-time array projection is in the SqlExpr
            ty,
            decode,
        });
    }
    env.root_alias = prev_root;
    Ok(out)
}

/// Heuristic: returns true when the FHIRPath source `path` is plain field
/// navigation with no operators, function calls, or boolean-yielding
/// constructs — therefore guaranteed not to resolve to a boolean. Used by
/// the top-level `where[]` validator to reject views whose where expressions
/// can't possibly yield true/false.
fn where_path_is_provably_non_boolean(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return false;
    }
    // A bare boolean field (`active`, `deceased`) is fine — we coerce at
    // the WHERE boundary. Reject only multi-segment paths with no operators
    // / function calls / boolean keywords.
    let has_operator = trimmed.contains('=')
        || trimmed.contains('!')
        || trimmed.contains('<')
        || trimmed.contains('>');
    let has_call = trimmed.contains('(');
    let has_bool_kw = [" and ", " or ", " not ", " in ", " contains "]
        .iter()
        .any(|k| trimmed.contains(k));
    !has_operator && !has_call && !has_bool_kw && trimmed.contains('.')
}

/// Returns true when the FHIRPath source `path` navigates *through* a
/// collection-cardinality FHIR element. Used by the strict `collection: false`
/// check to reject views the runtime would mishandle.
///
/// Uses the per-version `get_field_type` lookup tables generated from FHIR
/// StructureDefinitions (see `helios_fhir::{r4,r4b,r5,r6}::FIELD_TYPES`). The
/// walk only handles plain dot navigation — any segment containing `(`, `[`,
/// or whitespace is treated as opaque and stops the walk (returning the
/// accumulated result so far). This stays conservative: function calls like
/// `.first()` or `.where(...)` may change cardinality in ways the lookup
/// can't model, so we don't speculate past them.
fn path_likely_multi_valued(path: &str, resource_type: &str, fhir_version: FhirVersion) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() || resource_type.is_empty() {
        return false;
    }
    let mut parent = resource_type.to_string();
    let mut segments = trimmed.split('.').peekable();
    while let Some(seg) = segments.next() {
        // Opaque segment (function call, indexer, anything non-trivial) —
        // bail rather than guess.
        if seg.is_empty() || seg.chars().any(|c| !c.is_ascii_alphanumeric()) {
            return false;
        }
        let Some((field_type, is_collection)) =
            super::lookup_field_type(fhir_version, &parent, seg)
        else {
            return false;
        };
        // We only fail the column when the collection appears *before* the
        // final segment — `path = "name"` (which yields the full list) is
        // accepted because the column projection wraps it in a JSON array.
        if is_collection && segments.peek().is_some() {
            return true;
        }
        parent = field_type.to_string();
    }
    false
}

/// FHIR type of the nodes a `repeat:` descent visits, when it is unambiguous.
///
/// Each step path is applied first to the parent focus and then, recursively,
/// to every node it reaches, so a step may only resolve at one of those
/// levels (`answer.item` on a `QuestionnaireResponse` resolves only from an
/// item). The node type is the one shared by every step that resolves from
/// the parent focus, provided every step that resolves from that type lands
/// back on it.
fn repeat_node_type(
    parent_focus: &str,
    step_paths: &[super::ir::JsonPath],
    env: &CompileEnv,
) -> Option<String> {
    let mut seeds = step_paths
        .iter()
        .filter_map(|p| walk_fhir_type(parent_focus, p, env).map(|(t, _)| t));
    let node_type = seeds.next()?;
    if seeds.any(|t| t != node_type) {
        return None;
    }
    step_paths
        .iter()
        .filter_map(|p| walk_fhir_type_from(&node_type, p, env).map(|(t, _)| t))
        .all(|t| t == node_type)
        .then_some(node_type)
}

/// Infers how an untyped column's text must be decoded from the shape of its
/// expression. Anything that can't be resolved stays [`ColumnDecode::Auto`].
///
/// Paths rooted at a `forEach` / `repeat` focus resolve through the focus
/// types the compiler registers in [`CompileEnv::focus_types`]; a path ending
/// on a repeating field stays [`ColumnDecode::Auto`].
fn infer_decode(expr: &SqlExpr, env: &CompileEnv) -> ColumnDecode {
    if is_boolean_valued(expr) {
        return ColumnDecode::Boolean;
    }
    match expr {
        SqlExpr::Lit(LitValue::Str(_))
        | SqlExpr::ReferenceKey { .. }
        | SqlExpr::JoinAggregate { .. } => ColumnDecode::Text,
        SqlExpr::Lit(LitValue::Int(_)) | SqlExpr::RowIndex(_) => ColumnDecode::Integer,
        SqlExpr::Lit(LitValue::Decimal(_)) => ColumnDecode::Decimal,
        // Indexed forEach columns wrap the per-element projection.
        SqlExpr::ScalarFromChain { projection, .. } => infer_decode(projection, env),
        SqlExpr::JsonPath { root, path } => fhir_type_of_path(root, path, env)
            .map(|t| ColumnDecode::from_fhir_type(&t))
            .unwrap_or(ColumnDecode::Auto),
        SqlExpr::Alias { inner, .. } => infer_decode(inner, env),
        _ => ColumnDecode::Auto,
    }
}

/// True for expressions whose value is a FHIRPath Boolean by construction:
/// boolean literals, comparisons, logical operators and existence tests.
/// Plain paths are typed through the FHIR element tables instead.
fn is_boolean_valued(expr: &SqlExpr) -> bool {
    use super::ir::{BinOp, UnaryOp};
    match expr {
        SqlExpr::Lit(LitValue::Bool(_)) | SqlExpr::WhereExists { .. } => true,
        SqlExpr::BinOp { op, .. } => matches!(
            op,
            BinOp::Eq
                | BinOp::Neq
                | BinOp::Lt
                | BinOp::Lte
                | BinOp::Gt
                | BinOp::Gte
                | BinOp::And
                | BinOp::Or
                | BinOp::Like
                | BinOp::RegexMatch
        ),
        SqlExpr::UnaryOp { op, .. } => {
            matches!(op, UnaryOp::Not | UnaryOp::IsNull | UnaryOp::IsNotNull)
        }
        SqlExpr::Alias { inner, .. } => is_boolean_valued(inner),
        _ => false,
    }
}

/// Maps a `column.type` string (per the SoF v2 spec) onto the in-DB compiler's
/// [`SqlType`]. Unknown / absent types fall back to text; how that text is
/// turned into JSON is decided separately by [`ColumnDecode`].
fn column_type_from_hint(hint: Option<&str>) -> SqlType {
    match hint {
        Some("boolean") => SqlType::Boolean,
        // `integer64` fits `SqlType::Integer`: PostgreSQL casts to `bigint`
        // and SQLite's INTEGER is 64-bit.
        Some("integer") | Some("positiveInt") | Some("unsignedInt") | Some("integer64") => {
            SqlType::Integer
        }
        Some("decimal") => SqlType::Decimal,
        _ => SqlType::Text,
    }
}

/// Builds an inline FROM-clause string for a flattened forEach chain — one
/// unnest per Field segment, comma-joined. Used by the trailing-`[N]`
/// degenerate-forEach lowering, which can't put correlated subqueries in
/// the FROM on SQLite (SQLite restriction; PG supports it via LATERAL,
/// but we use the same SELECT-side scalar-subquery shape on both for
/// uniformity).
///
/// Returns the chain SQL plus the alias of the innermost iteration row so
/// callers can root column projections on `<deepest>.value`. Each segment's
/// unnest source is wrapped in a dialect-appropriate type guard so
/// non-array intermediates (FHIR singletons like `Patient.contact.name`)
/// produce one row instead of erroring.
fn build_degenerate_chain_sql(
    segments: &[super::ir::JsonPath],
    parent_focus: &str,
    alias_seq: &mut AliasSeq,
    dialect: &dyn super::dialect::Dialect,
) -> (String, String) {
    use super::ir::PathStep;
    let mut from_parts: Vec<String> = Vec::new();
    let mut prev = parent_focus.to_string();
    let mut last_alias = String::new();
    let is_sqlite = dialect.lateral_keyword().is_empty();
    for seg in segments {
        let alias = alias_seq.next();
        let segs_owned: Vec<String> = seg
            .0
            .iter()
            .filter_map(|s| match s {
                PathStep::Field(n) => Some(n.clone()),
                PathStep::Index(n) => Some(n.to_string()),
                _ => None,
            })
            .collect();
        let segs: Vec<&str> = segs_owned.iter().map(String::as_str).collect();
        let unnest_sql = if is_sqlite {
            // SQLite — single-arg `json_each` with a JSON-text source +
            // path. Numeric segments use `[N]`, others use `.field`. The
            // path is a complete, escaped SQL string literal (quotes included).
            // Type-guarded even off `r.data`: the two-arg form would iterate
            // the members of a singleton object instead of the object.
            let path_lit = sqlite_json_path_literal(&segs);
            let extracted = format!("json_extract({prev}, {path_lit})");
            let type_check = format!("json_type({prev}, {path_lit})");
            format!(
                "json_each(CASE WHEN {type_check} = 'array' THEN {extracted} \
                 WHEN {type_check} IN ('object', 'array') THEN json_array(json({extracted})) \
                 WHEN {type_check} IS NOT NULL THEN json_array({extracted}) \
                 ELSE '[]' END)"
            )
        } else {
            // PostgreSQL — `jsonb_array_elements` over a `jsonb_typeof`
            // type-guard so object intermediates (FHIR singletons) get
            // wrapped in a single-element array. Numeric segments are
            // path-array integers; field segments are path-array strings.
            //
            // `prev` may be either a jsonb expression (e.g. `r.data` or
            // `<alias>.value` from jsonb_array_elements) or a text-typed
            // correlated SELECT (when feeding from a prior ScalarFromChain
            // whose projection used the `->>` text operator). Cast to
            // jsonb so navigation works in both cases — `(jsonb)::jsonb`
            // is a no-op, `(text)::jsonb` parses the JSON text.
            let prev_jsonb = format!("({prev})::jsonb");
            let nav = if segs.len() == 1 {
                format!("{prev_jsonb}->{}", pg_key_literal(segs[0]))
            } else {
                format!("{prev_jsonb}#>{}", pg_path_array_literal(&segs))
            };
            format!(
                "jsonb_array_elements(CASE WHEN jsonb_typeof({nav}) = 'array' THEN {nav} \
                 WHEN jsonb_typeof({nav}) IS NOT NULL THEN jsonb_build_array({nav}) \
                 ELSE '[]'::jsonb END)"
            )
        };
        let from_part = if is_sqlite {
            format!("{unnest_sql} {alias}")
        } else {
            // PG — give the table-function alias `<alias>(value)` so callers
            // can reference `<alias>.value` uniformly.
            format!("{unnest_sql} AS {alias}(value)")
        };
        from_parts.push(from_part);
        last_alias = alias.clone();
        prev = format!("{alias}.value");
    }
    (from_parts.join(", "), last_alias)
}

/// Splits a FHIRPath JSON path into one [`JsonPath`] per `Field` step.
///
/// `Index` steps stay grouped with the immediately-preceding `Field` so that
/// `name[0].use` still drives a single navigation step into the first name
/// before unnesting `use`. `OfType` / `TypeFilter` follow the same grouping.
pub(super) fn split_path_into_segments(path: &super::ir::JsonPath) -> Vec<super::ir::JsonPath> {
    let mut segments: Vec<super::ir::JsonPath> = Vec::new();
    let mut current: Vec<PathStep> = Vec::new();
    for step in &path.0 {
        match step {
            PathStep::Field(_) => {
                if !current.is_empty() {
                    segments.push(super::ir::JsonPath(std::mem::take(&mut current)));
                }
                current.push(step.clone());
            }
            _ => current.push(step.clone()),
        }
    }
    if !current.is_empty() {
        segments.push(super::ir::JsonPath(current));
    }
    segments
}

/// Wraps `parent` in a chain of LateralUnnest / Recurse nodes, one per step
/// in document order — outer-most last so the emitter walks from Scan upward
/// and orders the JOINs correctly.
fn apply_steps(parent: PlanNode, steps: Vec<IterStep>) -> PlanNode {
    steps.into_iter().fold(parent, |p, step| match step {
        IterStep::Unnest(u) => PlanNode::LateralUnnest {
            parent: Box::new(p),
            source: u.source,
            out_alias: u.out_alias,
            left_join: u.left_join,
            on_filter: u.on_filter,
            flat_index: u.flat_index,
        },
        IterStep::Recurse(r) => PlanNode::Recurse {
            parent: Box::new(p),
            seed_root: r.seed_root,
            step_paths: r.step_paths,
            out_alias: r.out_alias,
        },
    })
}

/// Wraps `parent` in one [`PlanNode::Filter`] per predicate.
fn apply_filters(parent: PlanNode, filters: Vec<SqlExpr>) -> PlanNode {
    filters
        .into_iter()
        .fold(parent, |plan, predicate| PlanNode::Filter {
            parent: Box::new(plan),
            predicate,
        })
}

/// Final-step sanity check — `plan_clause_list` always returns either a
/// `Project` or a `Union` of `Project`s; nothing else should reach the
/// emitter at the top level.
fn ensure_project(plan: PlanNode) -> Result<PlanNode, SofError> {
    match &plan {
        PlanNode::Project { .. } | PlanNode::Union(_) => Ok(plan),
        other => Err(SofError::InvalidViewDefinition(format!(
            "plan_clause_list returned an unexpected top node: {other:?}"
        ))),
    }
}

/// Sequentially-numbered alias generator for lateral unnests (`fe1`, `fe2`, …).
/// Keeps generated SQL deterministic and avoids alias collisions when sibling
/// or nested clauses each introduce their own forEach.
#[derive(Debug, Default)]
struct AliasSeq {
    next: usize,
}

impl AliasSeq {
    fn new() -> Self {
        Self { next: 0 }
    }
    fn next(&mut self) -> String {
        self.next += 1;
        // The first unnest gets the legacy `fe` alias so existing test
        // assertions (which look for `fe.value`/`AS fe(value)`) keep matching.
        if self.next == 1 {
            FOREACH_ALIAS_PREFIX.to_string()
        } else {
            format!("{FOREACH_ALIAS_PREFIX}{}", self.next)
        }
    }
    fn next_recurse(&mut self) -> String {
        self.next += 1;
        format!("rec_{}", self.next - 1)
    }
}

// PathStep is consumed when read_clause receives a JsonPath from
// compile_fhirpath_expr — keep the import referenced for clarity.
const _: Option<PathStep> = None;

#[cfg(test)]
mod tests {
    use super::super::dialect::{Dialect, PgDialect, SqliteDialect};
    use super::super::ir::JsonPath;
    use super::*;

    /// `build_degenerate_chain_sql` splices member names into path literals
    /// itself; a name that is not a plain identifier must stay inside them.
    #[test]
    fn degenerate_chain_escapes_member_names() {
        let hostile = "x') OR 1=1 --";
        let segments = vec![
            JsonPath(vec![
                PathStep::Field(hostile.to_string()),
                PathStep::Index(0),
            ]),
            JsonPath(vec![PathStep::Field(hostile.to_string())]),
        ];
        let dialects: [&dyn Dialect; 2] = [&SqliteDialect, &PgDialect];
        for dialect in dialects {
            let (sql, _alias) =
                build_degenerate_chain_sql(&segments, "r.data", &mut AliasSeq::new(), dialect);
            // The payload's quote must be doubled, never left to close the
            // literal.
            assert!(
                !sql.contains("x') OR"),
                "{}: payload escaped its literal:\n{sql}",
                dialect.name()
            );
            assert!(
                sql.contains("x'') OR 1=1 --"),
                "{}: payload not escaped:\n{sql}",
                dialect.name()
            );
        }
    }

    #[test]
    fn degenerate_chain_plain_names_unchanged() {
        let segments = vec![JsonPath(vec![PathStep::Field("name".to_string())])];
        let (sqlite, _) =
            build_degenerate_chain_sql(&segments, "r.data", &mut AliasSeq::new(), &SqliteDialect);
        assert!(
            sqlite.starts_with(
                "json_each(CASE WHEN json_type(r.data, '$.name') = 'array' \
                 THEN json_extract(r.data, '$.name')"
            ),
            "{sqlite}"
        );
        let (pg, _) =
            build_degenerate_chain_sql(&segments, "r.data", &mut AliasSeq::new(), &PgDialect);
        assert!(pg.contains("(r.data)::jsonb->'name'"), "{pg}");
    }
}
