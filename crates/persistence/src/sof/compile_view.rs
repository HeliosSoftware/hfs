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

#![allow(missing_docs)] // Per-field docs land alongside their consumers in stages 4–5.

use helios_fhir::FhirVersion;
use helios_sof::ConstantValue;
use serde_json::Value;

use crate::core::sof_runner::SofError;

use super::compile_path::{CompileEnv, Constant, compile_fhirpath_expr};
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
) -> Result<PlanNode, SofError> {
    // Single-pass: collect sibling root columns + at most one `forEach` per
    // level + handle a single `unionAll` clause. Multiple unionAll clauses at
    // the same level are not exercised by the corpus.
    let mut shared_columns: Vec<Column> = Vec::new();
    let mut shared_unnests: Vec<UnnestStep> = Vec::new();
    let mut shared_recurse: Option<RecurseInfo> = None;
    let mut union_branches: Option<&Vec<Value>> = None;

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
            let parts =
                read_clause_columns_and_iter(clause, parent_focus, env, alias_seq, dialect)?;
            shared_columns.extend(parts.columns);
            shared_unnests.extend(parts.unnests);
            continue;
        }

        let parts = read_clause_columns_and_iter(clause, parent_focus, env, alias_seq, dialect)?;
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
    }

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
            };
            plan = apply_unnests(plan, &shared_unnests);
        } else {
            plan = apply_unnests(plan, &shared_unnests);
        }
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
        let parts = read_clause_columns_and_iter(branch, &branch_focus, env, alias_seq, dialect)?;
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
            }
        } else {
            // Each branch projection: parent's `where`-filtered scan + sibling
            // unnests + this branch's unnests; columns = sibling cols + branch cols.
            let mut combined_unnests = shared_unnests.clone();
            combined_unnests.extend(parts.unnests);
            apply_unnests(parent_plan.clone(), &combined_unnests)
        };
        // Apply per-branch extra filter (e.g. EXISTS-from-chain emitted by
        // trailing-`[N]` forEach lowering to drop resources whose flattened
        // chain returns no rows).
        if let Some(filter) = parts.extra_filter {
            branch_plan = PlanNode::Filter {
                parent: Box::new(branch_plan),
                predicate: filter,
            };
        }

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
    /// Step paths to walk on each iteration (`r.data` for the seed,
    /// `<alias>.node` for subsequent levels).
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
    /// Extra per-branch filter applied as `Filter(parent, predicate)`.
    /// Set by trailing-`[N]` forEach lowering to drop resources whose
    /// flattened chain returns fewer than `N+1` elements.
    extra_filter: Option<SqlExpr>,
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
        let mut columns = read_columns(clause, &focus, env)?;
        // Nested `select[]` under `repeat:` may add columns at the recursive
        // node focus AND/OR extend the row source via a forEach (e.g.
        // `repeat:[item]` with a nested `forEach: "answer"` projects answer
        // rows). Each nested forEach's unnests get hoisted onto the
        // post-recurse plan; nested repeats are rejected.
        let mut nested_unnests: Vec<UnnestStep> = Vec::new();
        if let Some(nested) = clause.get("select").and_then(|v| v.as_array()) {
            for sub in nested {
                let sub_parts = read_clause_columns_and_iter(sub, &focus, env, alias_seq, dialect)?;
                if sub_parts.recurse.is_some() {
                    return Err(SofError::Uncompilable {
                        reason: "select.repeat with nested repeat is not yet supported".to_string(),
                    });
                }
                nested_unnests.extend(sub_parts.unnests);
                columns.extend(sub_parts.columns);
            }
        }
        return Ok(ClauseParts {
            columns,
            unnests: nested_unnests,
            recurse: Some(RecurseInfo {
                step_paths,
                out_alias: alias,
            }),
            extra_filter: None,
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
        // semantics — preserving outer rows when no element matches).
        let (path_src, where_crit_src): (String, Option<String>) =
            split_trailing_where(&src).unwrap_or((src.clone(), None));

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
        if let Some(idx) = trailing_index {
            let trimmed_path = super::ir::JsonPath(path.0[..path.0.len() - 1].to_vec());
            let segments = split_path_into_segments(&trimmed_path);
            let (chain_sql, deepest_alias) =
                build_degenerate_chain_sql(&segments, parent_focus, alias_seq, dialect);
            let column_focus = format!("{deepest_alias}.value");
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
                        offset: idx,
                    },
                    collection: c.collection,
                    ty: c.ty,
                })
                .collect();
            // For `forEach` (not `forEachOrNull`), an empty chain means
            // the resource produces NO row. Surface that as a per-branch
            // EXISTS filter — wraps the branch's plan with `Filter(EXISTS
            // (SELECT 1 FROM <chain> LIMIT 1 OFFSET <idx>))`.
            let extra_filter = if is_left_join {
                None
            } else {
                Some(SqlExpr::ScalarFromChain {
                    chain_sql: chain_sql.clone(),
                    projection: Box::new(SqlExpr::Lit(LitValue::Int(1))),
                    offset: idx,
                })
            };
            return Ok(ClauseParts {
                columns,
                unnests: Vec::new(),
                recurse: None,
                extra_filter,
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
        let segments = split_path_into_segments(&path);
        let last_idx = segments.len().saturating_sub(1);
        for (i, seg_path) in segments.into_iter().enumerate() {
            let alias = alias_seq.next();
            let source = SqlExpr::JsonPath {
                root: focus.clone(),
                path: seg_path,
            };
            // Compile the trailing `where(crit)` filter against the LAST
            // unnest's iteration alias, so `name.where(use=X)` filters the
            // expanded `name` rows.
            let on_filter = if i == last_idx {
                if let Some(ref crit_src) = where_crit_src {
                    let prev_root = env.root_alias.clone();
                    env.root_alias = format!("{alias}.value");
                    let pred = compile_fhirpath_expr(crit_src, env);
                    env.root_alias = prev_root;
                    Some(pred?)
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
            focus = format!("{alias}.value");
        }
        // Apply trailing `[N]` semantics by tagging the LAST unnest with a
        // limit/offset; the emitter wraps that unnest in a `LIMIT 1 OFFSET N`
        // subquery so only the Nth element of the flattened collection is
        // iterated.
        if let Some(n) = trailing_index
            && let Some(last) = unnests.last_mut()
        {
            last.flat_index = Some(n);
        }
        (unnests, focus)
    } else {
        (Vec::new(), parent_focus.to_string())
    };

    let mut columns = read_columns(clause, &focus, env)?;

    // Nested select clauses: each contributes additional columns under the
    // current focus. If a nested clause has its own forEach we extend the
    // unnest chain; deeper unionAll inside nested select[] is not supported
    // until a real-world conformance case demands it (corpus doesn't).
    if let Some(nested) = clause.get("select").and_then(|v| v.as_array()) {
        for sub in nested {
            if sub.get("unionAll").is_some() {
                return Err(SofError::Uncompilable {
                    reason: "unionAll nested inside another select is not supported".to_string(),
                });
            }
            let sub_parts = read_clause_columns_and_iter(sub, &focus, env, alias_seq, dialect)?;
            if sub_parts.recurse.is_some() {
                return Err(SofError::Uncompilable {
                    reason: "select.repeat nested inside another select is not yet supported"
                        .to_string(),
                });
            }
            unnests.extend(sub_parts.unnests);
            columns.extend(sub_parts.columns);
        }
    }

    Ok(ClauseParts {
        columns,
        unnests,
        recurse: None,
        extra_filter: None,
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

        let ty = column_type_from_hint(column_type.as_deref());
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

/// Maps a `column.type` string (per the SoF v2 spec) onto the in-DB compiler's
/// [`SqlType`]. Unknown / absent types fall back to text — the runner's row
/// mapper auto-parses numeric-looking text as JSON numbers, which works for
/// most cases without explicit typing.
fn column_type_from_hint(hint: Option<&str>) -> SqlType {
    match hint {
        Some("boolean") => SqlType::Boolean,
        Some("integer") | Some("positiveInt") | Some("unsignedInt") => SqlType::Integer,
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
            // path. Numeric segments use `[N]`, others use `.field`.
            let mut path_str = String::from("$");
            for s in &segs {
                if s.chars().all(|c| c.is_ascii_digit()) {
                    path_str.push('[');
                    path_str.push_str(s);
                    path_str.push(']');
                } else {
                    path_str.push('.');
                    path_str.push_str(s);
                }
            }
            if prev == "r.data" && !path_str.contains('[') {
                format!("json_each({prev}, '{path_str}')")
            } else {
                let extracted = format!("json_extract({prev}, '{path_str}')");
                let type_check = format!("json_type({prev}, '{path_str}')");
                format!(
                    "json_each(CASE WHEN {type_check} = 'array' THEN {extracted} \
                     WHEN {type_check} IN ('object', 'array') THEN json_array(json({extracted})) \
                     WHEN {type_check} IS NOT NULL THEN json_array({extracted}) \
                     ELSE '[]' END)"
                )
            }
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
                format!("{prev_jsonb}->'{}'", segs[0])
            } else {
                format!("{prev_jsonb}#>'{{{}}}'", segs.join(","))
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
fn split_path_into_segments(path: &super::ir::JsonPath) -> Vec<super::ir::JsonPath> {
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
