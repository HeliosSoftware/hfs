//! ViewDefinition JSON → [`PlanNode`] compiler.
//!
//! Walks the SoF `select` tree producing a plan tree rooted in a
//! [`PlanNode::Scan`] over `resources`. Per-clause logic:
//!
//! - Plain `select.column[]` → column projections off the current focus.
//! - `forEach`/`forEachOrNull` → [`PlanNode::LateralUnnest`] over the parent.
//! - Nested `select[]` → contributes additional columns under the parent's
//!   focus (or extends the row source if it has its own `forEach`).
//! - `unionAll[]` → [`PlanNode::Union`] with sibling column[] merged into
//!   each branch.
//! - Top-level `where[].path` → [`PlanNode::Filter`] applied to the root scan.
//!
//! Stages 4–5 add chained-call collection threading, repeat:, and boundary
//! functions.

use helios_fhir::FhirVersion;
use helios_sof::ConstantValue;
use serde_json::Value;

use crate::core::sof_runner::SofError;

use super::compile_path::{
    CompileEnv, Constant, compile_fhirpath_expr, fhir_type_of_path, walk_fhir_type,
};
use super::decode::ColumnDecode;
use super::ir::{Column, LitValue, PathStep, PlanNode, RowIndexScope, SqlExpr, SqlType};

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

    // Every resource-document navigation is rooted where the target reads
    // the document (PostgreSQL: its once-detoasted copy, not `r.data`).
    let document_root = target.resource_document(dialect);
    let mut env = CompileEnv::new_for_resource(document_root, resource_type.clone(), fhir_version);
    // `where(crit)` criteria read the enclosing iteration's `%rowIndex` on
    // the SQL targets only; MongoDB keeps its pre-#1623 lowering.
    env.pin_where_row_index = target.pins_where_row_index();
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
        document_root,
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

/// Walks a list of select clauses sharing a parent row source. Builds either
/// a single `Project` (one row per parent row) or a `Union` of Projects (when
/// any clause is `unionAll`).
fn plan_clause_list(
    clauses: &[Value],
    parent_plan: &PlanNode,
    parent_focus: &str,
    env: &mut CompileEnv,
    alias_seq: &mut AliasSeq,
    dialect: &dyn super::dialect::Dialect,
    target: super::compiler::CompileTarget,
) -> Result<PlanNode, SofError> {
    // Single-pass: collect sibling root columns + at most one `forEach` per
    // level + handle a single `unionAll` clause. Multiple unionAll clauses at
    // the same level are not exercised by the corpus.
    let mut shared_columns: Vec<Column> = Vec::new();
    let mut shared_unnests: Vec<UnnestStep> = Vec::new();
    let mut shared_recurse: Option<RecurseInfo> = None;
    // Membership filters of indexed `forEach` clauses at this level (or
    // nested in them); every row of this level must satisfy them.
    let mut shared_filters: Vec<SqlExpr> = Vec::new();
    let mut union_branches: Option<&Vec<Value>> = None;
    // Resource-dependency sidecar: every resource-document read recorded
    // while compiling this level's clauses (a `repeat:` clause excludes its
    // own seed paths, which its recursive seed scans directly).
    let reads_at_start = env.resource_reads;

    for clause in clauses {
        if let Some(branches) = clause.get("unionAll").and_then(|v| v.as_array()) {
            if union_branches.is_some() {
                return Err(SofError::Uncompilable {
                    reason: "multiple unionAll clauses at the same level are not supported"
                        .to_string(),
                });
            }
            if branches.is_empty() {
                return Err(SofError::InvalidViewDefinition(
                    "unionAll branches list is empty".to_string(),
                ));
            }
            union_branches = Some(branches);
            // Sibling columns/forEach in this same clause are merged into
            // every branch (handled below).
            let parts = read_clause_columns_and_iter(
                clause,
                parent_focus,
                env,
                alias_seq,
                dialect,
                target,
            )?;
            shared_columns.extend(parts.columns);
            shared_unnests.extend(parts.unnests);
            shared_filters.extend(parts.extra_filters);
            continue;
        }

        let parts =
            read_clause_columns_and_iter(clause, parent_focus, env, alias_seq, dialect, target)?;
        if let Some(rec) = parts.recurse {
            if shared_recurse.is_some() {
                return Err(SofError::Uncompilable {
                    reason: "multiple repeat clauses at the same level are not supported"
                        .to_string(),
                });
            }
            shared_recurse = Some(rec);
        }
        shared_columns.extend(parts.columns);
        shared_unnests.extend(parts.unnests);
        shared_filters.extend(parts.extra_filters);
    }
    // Shared columns and unnests of this level read the resource row, so a
    // recursive select carrying them must rejoin `resources`.
    let shared_reads_resource = env.resource_reads != reads_at_start;

    // No unionAll → single Project, possibly under a chain of LATERAL unnests
    // or wrapping a recursive descent.
    let Some(branches) = union_branches else {
        if shared_columns.is_empty() {
            return Err(SofError::InvalidViewDefinition(
                "no columns found in select clauses".to_string(),
            ));
        }
        let mut plan = parent_plan.clone();
        if let Some(rec) = shared_recurse {
            // Recurse first, then apply any nested forEach unnests on top so
            // `repeat:[item]` with a nested `forEach: "answer"` joins each
            // visited node against its answer array.
            plan = PlanNode::Recurse {
                parent: Box::new(plan),
                seed: SqlExpr::Lit(LitValue::Null), // unused; emitter walks parent
                step_paths: rec.step_paths,
                out_alias: rec.out_alias,
                needs_resource_row: shared_reads_resource,
            };
            plan = apply_unnests(plan, &shared_unnests);
        } else {
            plan = apply_unnests(plan, &shared_unnests);
        }
        // Above the row source: a filter may read any unnest (an indexed
        // clause nested under an ordinary `forEach`) or the recursive node.
        plan = apply_filters(plan, shared_filters);
        return Ok(PlanNode::Project {
            parent: Box::new(plan),
            columns: shared_columns,
        });
    };

    if shared_recurse.is_some() {
        return Err(SofError::Uncompilable {
            reason: "select.repeat combined with sibling unionAll is not yet supported".to_string(),
        });
    }

    // Flatten nested `unionAll` clauses one level deep — a branch whose only
    // content is another `unionAll` array expands to its inner branches.
    let flat_branches = flatten_union_branches(branches);

    // The branches must read against the focus produced by the shared
    // unnests (e.g. when the unionAll lives inside a `forEach: "contact"`
    // clause, each branch's paths resolve relative to the contact iteration
    // alias, not the resource document).
    let branch_focus = shared_unnests
        .last()
        .map(|u| format!("{}.value", u.out_alias))
        .unwrap_or_else(|| parent_focus.to_string());

    // unionAll → one Project per branch, sibling cols/unnests merged in,
    // wrapped in a Union.
    let mut branch_plans: Vec<PlanNode> = Vec::with_capacity(flat_branches.len());
    for branch in &flat_branches {
        let reads_before_branch = env.resource_reads;
        let parts =
            read_clause_columns_and_iter(branch, &branch_focus, env, alias_seq, dialect, target)?;
        let branch_reads_resource = env.resource_reads != reads_before_branch;
        // A unionAll branch may itself carry a `repeat:` clause — wrap that
        // branch's plan in a Recurse and let the per-branch Project read off
        // the recursive CTE alias.
        let mut branch_plan = if let Some(rec) = parts.recurse {
            if !shared_unnests.is_empty() || !parts.unnests.is_empty() {
                return Err(SofError::Uncompilable {
                    reason: "select.repeat inside a unionAll branch combined with forEach is \
                             not yet supported"
                        .to_string(),
                });
            }
            PlanNode::Recurse {
                parent: Box::new(parent_plan.clone()),
                seed: SqlExpr::Lit(LitValue::Null),
                step_paths: rec.step_paths,
                out_alias: rec.out_alias,
                // The branch carries the shared columns merged below.
                needs_resource_row: shared_reads_resource || branch_reads_resource,
            }
        } else {
            // Each branch projection: parent's `where`-filtered scan + sibling
            // unnests + this branch's unnests; columns = sibling cols + branch cols.
            let mut combined_unnests = shared_unnests.clone();
            combined_unnests.extend(parts.unnests);
            apply_unnests(parent_plan.clone(), &combined_unnests)
        };
        // Membership filters of indexed `forEach` clauses: the shared ones
        // (their columns are merged into every branch) and this branch's.
        let mut branch_filters = shared_filters.clone();
        branch_filters.extend(parts.extra_filters);
        branch_plan = apply_filters(branch_plan, branch_filters);

        let mut combined_cols = shared_columns.clone();
        combined_cols.extend(parts.columns);
        if combined_cols.is_empty() {
            return Err(SofError::InvalidViewDefinition(
                "unionAll branch produced no output columns".to_string(),
            ));
        }
        branch_plans.push(PlanNode::Project {
            parent: Box::new(branch_plan),
            columns: combined_cols,
        });
    }

    Ok(PlanNode::Union(branch_plans))
}

/// Flattens nested `unionAll` clauses one level deep — a branch whose only
/// content is another `unionAll` array expands to its inner branches. The
/// SoF v2 spec treats nested unionAll as semantically equivalent to a single
/// flat list, so the compiler can simplify before plan assembly.
fn flatten_union_branches(branches: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for b in branches {
        if let Some(inner) = b.get("unionAll").and_then(|v| v.as_array())
            && b.as_object().map(|o| o.len() == 1).unwrap_or(false)
        {
            out.extend(flatten_union_branches(inner));
        } else {
            out.push(b.clone());
        }
    }
    out
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
    /// Step paths to walk on each iteration (the resource document for the
    /// seed, `<alias>.node` for subsequent levels).
    step_paths: Vec<super::ir::JsonPath>,
    /// Alias of the recursive CTE (also the column alias for `node`).
    out_alias: String,
}

/// Output of [`read_clause_columns_and_iter`]: the columns this clause
/// contributes plus any unnests / recurse it adds to the row source.
#[derive(Debug)]
struct ClauseParts {
    columns: Vec<Column>,
    unnests: Vec<UnnestStep>,
    recurse: Option<RecurseInfo>,
    /// Membership filters applied as `Filter(parent, predicate)` above the
    /// enclosing scope's row source. Set by trailing-`[N]` `forEach`
    /// lowering (here or in any nested clause) to drop enclosing occurrences
    /// whose flattened chain has no `N`-th element.
    extra_filters: Vec<SqlExpr>,
}

/// Reads a single (non-unionAll) clause: its `forEach[OrNull]`, `column[]`,
/// and any nested `select[]` clauses. Nested clauses contribute columns at
/// the same focus (or extend the row source if they themselves have a
/// forEach).
fn read_clause_columns_and_iter(
    clause: &Value,
    parent_focus: &str,
    env: &mut CompileEnv,
    alias_seq: &mut AliasSeq,
    dialect: &dyn super::dialect::Dialect,
    target: super::compiler::CompileTarget,
) -> Result<ClauseParts, SofError> {
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
        // The seed paths are navigated by the recursive seed's own resource
        // scan; they are not a resource dependency of the outer select.
        let reads_before_seeds = env.resource_reads;
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
        env.resource_reads = reads_before_seeds;
        let alias = alias_seq.next_recurse();
        let focus = format!("{alias}.node");
        // The recursive node has the type every step lands on, when they agree.
        let mut step_types = step_paths
            .iter()
            .map(|p| walk_fhir_type(parent_focus, p, env).map(|(t, _)| t));
        if let Some(Some(first)) = step_types.next()
            && step_types.all(|t| t.as_deref() == Some(first.as_str()))
        {
            env.focus_types.insert(focus.clone(), first);
        }
        let mut columns = read_columns(clause, &focus, env)?;
        // Nested `select[]` under `repeat:` may add columns at the recursive
        // node focus AND/OR extend the row source via a forEach (e.g.
        // `repeat:[item]` with a nested `forEach: "answer"` projects answer
        // rows). Each nested forEach's unnests get hoisted onto the
        // post-recurse plan; nested repeats are rejected.
        let mut nested_unnests: Vec<UnnestStep> = Vec::new();
        let mut extra_filters: Vec<SqlExpr> = Vec::new();
        if let Some(nested) = clause.get("select").and_then(|v| v.as_array()) {
            for sub in nested {
                let sub_parts =
                    read_clause_columns_and_iter(sub, &focus, env, alias_seq, dialect, target)?;
                if sub_parts.recurse.is_some() {
                    return Err(SofError::Uncompilable {
                        reason: "select.repeat with nested repeat is not yet supported".to_string(),
                    });
                }
                nested_unnests.extend(sub_parts.unnests);
                columns.extend(sub_parts.columns);
                // An indexed `forEach` nested under the repeat filters the
                // visited nodes (applied above the post-repeat row source).
                extra_filters.extend(sub_parts.extra_filters);
            }
        }
        return Ok(ClauseParts {
            columns,
            unnests: nested_unnests,
            recurse: Some(RecurseInfo {
                step_paths,
                out_alias: alias,
            }),
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

    let (mut unnests, focus): (Vec<UnnestStep>, String) = if let Some(src) = iter_path_src {
        // Detect a trailing `where(crit)` on the forEach path
        // (`forEach: "name.where(use = X)"`). The criterion is lifted into
        // the JOIN ON clause of the last lateral unnest so the iteration
        // skips non-matching elements (and `forEachOrNull` keeps left-join
        // semantics — preserving outer rows when no element matches). On an
        // indexed path (`name[N].where(...)`) it instead filters the selected
        // occurrence (see the `ScalarFromChain` lowering below). The
        // filter-first form `name.where(...)[N]` is not a simple JSON path
        // and is rejected as uncompilable.
        let (path_src, where_crit_src): (String, Option<String>) =
            split_trailing_where(&src).unwrap_or((src.clone(), None));
        // The evaluator evaluates the whole `forEach` expression — its
        // criterion included — with the ENCLOSING iteration's variables and
        // binds the new `%rowIndex` only for the produced columns: the
        // criterion navigates the iterated element but reads the enclosing
        // scope's `%rowIndex`.
        let enclosing_row_index = enclosing_row_index_scope(env, parent_focus);

        let prev_root = env.root_alias.clone();
        env.root_alias = parent_focus.to_string();
        let path_expr = compile_fhirpath_expr(&path_src, env)?;
        env.root_alias = prev_root;
        let path = match path_expr {
            SqlExpr::JsonPath { path, .. } => path,
            _ => {
                return Err(SofError::Uncompilable {
                    reason: format!("forEach path '{src}' must be a simple JSON path"),
                });
            }
        };
        // FHIRPath `[N]` indexes the flattened collection result, not each
        // individual array crossing. SQLite forbids correlated subqueries in
        // FROM, so trailing-Index forEach paths short-circuit into a
        // *degenerate* iteration: no unnest in the FROM, each column wrapped
        // in a correlated `ScalarFromChain` subquery in the SELECT.
        let trailing_index = match path.0.last() {
            Some(super::ir::PathStep::Index(n)) if path.0.len() > 1 => Some(*n),
            _ => None,
        };
        // SQL targets lower trailing-`[N]` forEach into a correlated
        // `ScalarFromChain` subquery (SQLite forbids correlated FROM
        // subqueries). Targets without that constraint — MongoDB — instead
        // fall through to the normal unnest path below, which tags the last
        // unnest with `flat_index` for the emitter to lower to `$arrayElemAt`.
        if let Some(idx) = trailing_index
            && target.supports_correlated_from_subqueries()
        {
            let trimmed_path = super::ir::JsonPath(path.0[..path.0.len() - 1].to_vec());
            let segments = split_path_into_segments(&trimmed_path);
            let (chain, deepest_alias) =
                build_degenerate_chain_sql(&segments, parent_focus, alias_seq, dialect);
            let column_focus = format!("{deepest_alias}.value");
            // `%rowIndex` in the indexed scope is the singleton iteration's
            // index (0), also for an absent `forEachOrNull` selection — not
            // the chain alias's element position.
            let prev_indexed_focus = env.indexed_focus.replace(column_focus.clone());
            let raw_columns = read_columns(clause, &column_focus, env);
            // A trailing `where(crit)` (`name[N].where(use = X)`) filters the
            // SELECTED occurrence: FHIRPath indexes first, then filters the
            // one-element result. Compiled over the selection's value, it
            // gates the value, the membership filter and the
            // `forEachOrNull` empty context alike. Its `%rowIndex` is the
            // enclosing scope's, not the selection's 0.
            let selection_filter = where_crit_src.as_deref().map(|crit_src| {
                compile_iteration_criterion(crit_src, &column_focus, &enclosing_row_index, env)
            });
            env.indexed_focus = prev_indexed_focus;
            let raw_columns = raw_columns?;
            let selection_filter = selection_filter.transpose()?.map(Box::new);
            // Wrap every column in a correlated scalar subquery. The outer
            // SELECT sees one row per enclosing occurrence; the column
            // projects the [N]-th element of the flattened chain, in element
            // order. `forEachOrNull` evaluates it against the empty
            // iteration context when nothing is selected.
            let columns: Vec<Column> = raw_columns
                .into_iter()
                .map(|c| Column {
                    name: c.name,
                    expr: SqlExpr::ScalarFromChain {
                        chain_sql: chain.from_sql.clone(),
                        order_sql: chain.order_sql.clone(),
                        value_alias: deepest_alias.clone(),
                        projection: Box::new(c.expr),
                        offset: idx,
                        empty_context: is_left_join,
                        selection_filter: selection_filter.clone(),
                    },
                    collection: c.collection,
                    ty: c.ty,
                    decode: c.decode,
                })
                .collect();
            // For `forEach` (not `forEachOrNull`), an absent or rejected
            // selection means the enclosing occurrence produces NO row.
            // Surface that as a membership filter on the selection's presence
            // — `(SELECT 1 FROM <chain> … LIMIT 1 OFFSET <idx>)`, plus the
            // trailing `where(crit)` — independent of the selected value, so
            // a selected JSON `null` still counts as present. Every
            // enclosing scope (clause list, nested select, union branch,
            // repeat) applies it above its row source.
            let extra_filters = if is_left_join {
                Vec::new()
            } else {
                vec![SqlExpr::ScalarFromChain {
                    chain_sql: chain.from_sql,
                    order_sql: chain.order_sql,
                    value_alias: deepest_alias,
                    projection: Box::new(SqlExpr::Lit(LitValue::Int(1))),
                    offset: idx,
                    empty_context: false,
                    selection_filter,
                }]
            };
            // Known gap, outside #1623: a nested `select` under an indexed
            // iteration is not lowered here (its columns are dropped).
            return Ok(ClauseParts {
                columns,
                unnests: Vec::new(),
                recurse: None,
                extra_filters,
            });
        }
        // FHIRPath flattens through array boundaries automatically — emit
        // one lateral unnest per `Field` step so `forEach: "contact.telecom"`
        // produces one row per inner element. `Index` steps stay attached to
        // the prior segment as plain navigation. Only the LAST `forEach`
        // step uses LEFT JOIN for `forEachOrNull` so missing intermediate
        // levels still drop the row (matching the FHIRPath empty-collection
        // semantics).
        let mut unnests: Vec<UnnestStep> = Vec::new();
        let mut focus = parent_focus.to_string();
        // When a trailing `[N]` is present (the non-SQL fall-through, e.g.
        // MongoDB), drop it from the unnest segments and apply it as
        // `flat_index` below — otherwise the segment navigation would index
        // `[N]` and `flat_index` would index it a second time.
        let unnest_path = if trailing_index.is_some() {
            super::ir::JsonPath(path.0[..path.0.len() - 1].to_vec())
        } else {
            path.clone()
        };
        // Trailing-`[N]` (Mongo fall-through): `forEach: "contact.telecom[0]"`
        // selects element N of the FLATTENED `contact.telecom` collection, not
        // per-contact. Keep the full multi-field path in a single unnest so the
        // emitter flattens before indexing (splitting into per-field unnests
        // would apply `[N]` per parent element instead).
        if let Some(n) = trailing_index {
            let alias = alias_seq.next();
            let focus = format!("{alias}.value");
            (
                vec![UnnestStep {
                    source: SqlExpr::JsonPath {
                        root: parent_focus.to_string(),
                        path: unnest_path,
                    },
                    out_alias: alias,
                    left_join: is_left_join,
                    on_filter: None,
                    flat_index: Some(n),
                }],
                focus,
            )
        } else {
            let segments = split_path_into_segments(&unnest_path);
            let last_idx = segments.len().saturating_sub(1);
            for (i, seg_path) in segments.into_iter().enumerate() {
                let alias = alias_seq.next();
                let seg_path_steps = seg_path.0.clone();
                let source = SqlExpr::JsonPath {
                    root: focus.clone(),
                    path: seg_path,
                };
                // Compile the trailing `where(crit)` filter against the LAST
                // unnest's iteration alias, so `name.where(use=X)` filters the
                // expanded `name` rows; its `%rowIndex` is the enclosing
                // scope's.
                let on_filter = if i == last_idx {
                    if let Some(ref crit_src) = where_crit_src {
                        Some(compile_iteration_criterion(
                            crit_src,
                            &format!("{alias}.value"),
                            &enclosing_row_index,
                            env,
                        )?)
                    } else {
                        None
                    }
                } else {
                    None
                };
                unnests.push(UnnestStep {
                    source,
                    out_alias: alias.clone(),
                    left_join: is_left_join && i == last_idx,
                    on_filter,
                    flat_index: None,
                });
                if let Some((ty, _)) =
                    walk_fhir_type(&focus, &super::ir::JsonPath(seg_path_steps), env)
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

    let mut columns = read_columns(clause, &focus, env)?;
    let mut extra_filters: Vec<SqlExpr> = Vec::new();

    // Nested select clauses: each contributes additional columns under the
    // current focus. If a nested clause has its own forEach we extend the
    // unnest chain; deeper unionAll inside nested select[] is not supported
    // until a real-world conformance case demands it (corpus doesn't). A
    // nested indexed `forEach` contributes its membership filter, applied by
    // the enclosing scope above this clause's unnests.
    if let Some(nested) = clause.get("select").and_then(|v| v.as_array()) {
        for sub in nested {
            if sub.get("unionAll").is_some() {
                return Err(SofError::Uncompilable {
                    reason: "unionAll nested inside another select is not supported".to_string(),
                });
            }
            let sub_parts =
                read_clause_columns_and_iter(sub, &focus, env, alias_seq, dialect, target)?;
            if sub_parts.recurse.is_some() {
                return Err(SofError::Uncompilable {
                    reason: "select.repeat nested inside another select is not yet supported"
                        .to_string(),
                });
            }
            unnests.extend(sub_parts.unnests);
            columns.extend(sub_parts.columns);
            extra_filters.extend(sub_parts.extra_filters);
        }
    }

    Ok(ClauseParts {
        columns,
        unnests,
        recurse: None,
        extra_filters,
    })
}

/// The `%rowIndex` scope at `parent_focus`: the enclosing iteration a
/// `forEach` path's `where(crit)` criterion reads.
fn enclosing_row_index_scope(env: &mut CompileEnv, parent_focus: &str) -> RowIndexScope {
    let prev_root = std::mem::replace(&mut env.root_alias, parent_focus.to_string());
    let scope = env.row_index_scope();
    env.root_alias = prev_root;
    scope
}

/// Compiles a `forEach` path's trailing `where(crit)` with the iterated
/// element (`element_focus`) as navigation focus and `%rowIndex` pinned to
/// the enclosing scope (SQL targets; see [`CompileEnv::pin_where_row_index`]).
fn compile_iteration_criterion(
    crit_src: &str,
    element_focus: &str,
    enclosing_row_index: &RowIndexScope,
    env: &mut CompileEnv,
) -> Result<SqlExpr, SofError> {
    let prev_root = std::mem::replace(&mut env.root_alias, element_focus.to_string());
    let prev_pin = env.pin_row_index_scope_to(enclosing_row_index.clone());
    let pred = compile_fhirpath_expr(crit_src, env);
    env.root_alias = prev_root;
    env.pinned_row_index_scope = prev_pin;
    pred
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

        let ty = column_type_from_hint(column_type.as_deref());
        let decode = match ColumnDecode::from_declared(column_type.as_deref(), collection) {
            ColumnDecode::Auto => infer_decode(&expr, env),
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

/// Splits a forEach path source like `"name.where(use = X)"` into the base
/// path (`"name"`) and the criterion source (`"use = X"`). Returns `None`
/// when the source doesn't end in a `where(...)` call so callers fall back
/// to plain path lowering. Detection is purely textual to avoid round-trips
/// through the FHIRPath AST in the common case.
fn split_trailing_where(src: &str) -> Option<(String, Option<String>)> {
    let trimmed = src.trim();
    let suffix = ".where(";
    let pos = trimmed.rfind(suffix)?;
    if !trimmed.ends_with(')') {
        return None;
    }
    let base = trimmed[..pos].trim().to_string();
    let crit = trimmed[pos + suffix.len()..trimmed.len() - 1]
        .trim()
        .to_string();
    Some((base, Some(crit)))
}

/// Infers how an untyped column's text must be decoded from the shape of its
/// expression. Anything that can't be resolved stays [`ColumnDecode::Auto`].
///
/// Paths rooted at a `forEach` / `repeat` focus resolve through the focus
/// types the compiler registers in [`CompileEnv::focus_types`]; a path ending
/// on a repeating field stays [`ColumnDecode::Auto`].
fn infer_decode(expr: &SqlExpr, env: &CompileEnv) -> ColumnDecode {
    match expr {
        SqlExpr::Lit(LitValue::Str(_))
        | SqlExpr::ReferenceKey { .. }
        | SqlExpr::JoinAggregate { .. } => ColumnDecode::Text,
        SqlExpr::RowIndex(_) => ColumnDecode::Integer,
        SqlExpr::JsonPath { root, path } => fhir_type_of_path(root, path, env)
            .map(|t| ColumnDecode::from_fhir_type(&t))
            .unwrap_or(ColumnDecode::Auto),
        SqlExpr::Alias { inner, .. } => infer_decode(inner, env),
        _ => ColumnDecode::Auto,
    }
}

/// Maps a `column.type` string (per the SoF v2 spec) onto the in-DB compiler's
/// [`SqlType`]. Unknown / absent types fall back to text; how that text is
/// turned into JSON is decided separately by [`ColumnDecode`].
fn column_type_from_hint(hint: Option<&str>) -> SqlType {
    match hint {
        Some("boolean") => SqlType::Boolean,
        Some("integer") | Some("positiveInt") | Some("unsignedInt") => SqlType::Integer,
        Some("decimal") => SqlType::Decimal,
        _ => SqlType::Text,
    }
}

/// Builds the inline `FROM` items for a flattened forEach chain — one unnest
/// per Field segment, comma-joined — used by the trailing-`[N]` indexed
/// forEach lowering, which can't put correlated subqueries in the FROM on
/// SQLite (PG supports it via LATERAL, but both use the same SELECT-side
/// scalar-subquery shape for uniformity).
///
/// Returns the chain, its element-order `ORDER BY` list (so `LIMIT 1 OFFSET
/// N` picks the evaluator's `[N]`) and the alias of the innermost iteration
/// row, so callers can root column projections on `<deepest>.value`. See
/// [`super::emit::flattened_chain_sql`] for the per-segment type guards.
fn build_degenerate_chain_sql(
    segments: &[super::ir::JsonPath],
    parent_focus: &str,
    alias_seq: &mut AliasSeq,
    dialect: &dyn super::dialect::Dialect,
) -> (super::emit::FlatChain, String) {
    let aliased: Vec<(String, super::ir::JsonPath)> = segments
        .iter()
        .map(|seg| (alias_seq.next(), seg.clone()))
        .collect();
    let deepest = aliased
        .last()
        .map(|(alias, _)| alias.clone())
        .unwrap_or_default();
    (
        super::emit::flattened_chain_sql(parent_focus, &aliased, dialect),
        deepest,
    )
}

/// Splits a FHIRPath JSON path into one [`JsonPath`] per `Field` step (see
/// [`super::ir::JsonPath::field_segments`]).
fn split_path_into_segments(path: &super::ir::JsonPath) -> Vec<super::ir::JsonPath> {
    path.field_segments()
}

/// Wraps `parent` in a chain of LateralUnnest nodes — outer-most last so the
/// emitter walks from Scan upward and orders the JOINs correctly.
fn apply_unnests(parent: PlanNode, unnests: &[UnnestStep]) -> PlanNode {
    let mut p = parent;
    for u in unnests {
        p = PlanNode::LateralUnnest {
            parent: Box::new(p),
            source: u.source.clone(),
            out_alias: u.out_alias.clone(),
            left_join: u.left_join,
            on_filter: u.on_filter.clone(),
            flat_index: u.flat_index,
        };
    }
    p
}

/// Wraps `parent` in one `Filter` per membership predicate.
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
            let (chain, _alias) =
                build_degenerate_chain_sql(&segments, "r.data", &mut AliasSeq::new(), dialect);
            let sql = chain.from_sql;
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
        let sqlite = sqlite.from_sql;
        assert!(
            sqlite.starts_with("json_each(r.data, '$.name') "),
            "{sqlite}"
        );
        let (pg, _) =
            build_degenerate_chain_sql(&segments, "r.data", &mut AliasSeq::new(), &PgDialect);
        let pg = pg.from_sql;
        assert!(pg.contains("(r.data)::jsonb->'name'"), "{pg}");
    }
}
