//! Lowers IR ([`PlanNode`]/[`SqlExpr`]) to a concrete SQL string for a given
//! [`Dialect`].
//!
//! The emitter expects each plan tree to have a [`PlanNode::Project`] at the
//! top (directly, or under a [`PlanNode::Union`]). Beneath the project lives a
//! chain of [`PlanNode::Filter`] and [`PlanNode::LateralUnnest`] nodes, rooted
//! in a [`PlanNode::Scan`]. The emitter walks that chain to assemble FROM /
//! JOIN / WHERE / SELECT in dialect-appropriate syntax, then concatenates them.
//!
//! Stages 2–5 progressively add IR-variant coverage. Anything the emitter
//! doesn't yet understand returns [`SofError::Uncompilable`].

use crate::core::sof_runner::SofError;

use super::decode::ColumnDecode;
use super::dialect::{Dialect, pg_key_literal, pg_path_array_literal, sqlite_json_path_literal};
use super::ir::{
    BinOp, BoundaryKind, BoundarySide, Column, CompartmentFilter, JsonPath, JsonType, LitValue,
    PathStep, PlanNode, ResourceFilter, RowIndexScope, SqlExpr, SqlType, UnaryOp,
    nearest_left_scope, unnest_source_root,
};

/// Column the recursive `repeat` CTE projects to order the flattened traversal
/// in pre-order. A sortable, zero-padded, dot-joined path of array indices;
/// ranking it per resource yields the 0-based `%rowIndex`.
const REPEAT_ORD_PATH_COL: &str = "ord_path";

/// Table every compiled view scans, always under alias `r`.
const RESOURCES_TABLE: &str = "resources";

/// Collation under which resource ids are ordered and range-compared.
fn id_collation(dialect: &dyn Dialect) -> &'static str {
    if dialect.name() == "sqlite" {
        "BINARY"
    } else {
        "\"C\""
    }
}

/// Tenant, resource type, deletion and runtime predicates for one resource scan.
/// Called at structural scan sites, never located in rendered SQL text.
fn resource_scan_predicate(filter: &ResourceFilter, dialect: &dyn Dialect) -> String {
    let mut predicate = format!(
        "r.tenant_id = {}\n  AND r.resource_type = {}\n  AND r.is_deleted = {}",
        dialect.placeholder(1),
        dialect.placeholder(2),
        dialect.bool_false()
    );
    if let Some(slot) = filter.since {
        predicate.push_str(&format!(
            "\n  AND r.last_updated >= {}",
            dialect.placeholder(slot)
        ));
    }
    if let Some((after, through)) = filter.id_range {
        let collation = id_collation(dialect);
        predicate.push_str(&format!(
            "\n  AND r.id COLLATE {collation} > {}\n  AND r.id COLLATE {collation} <= {}",
            dialect.placeholder(after),
            dialect.placeholder(through)
        ));
    }
    if let Some(compartment) = &filter.compartment {
        let list_match = |expr: &str, slot| {
            let placeholder = dialect.placeholder(slot);
            if dialect.name() == "postgres" {
                format!("{expr} = ANY({placeholder}::text[])")
            } else {
                format!("{expr} IN (SELECT value FROM json_each({placeholder}))")
            }
        };
        let membership = match compartment {
            CompartmentFilter::Owner { refs } => list_match("r.id", *refs),
            CompartmentFilter::NoMatches => "1=0".to_string(),
            CompartmentFilter::SearchIndex { param_names, refs } => format!(
                "EXISTS (SELECT 1 FROM search_index si \
                 WHERE si.tenant_id = {} \
                   AND si.resource_type = {} \
                   AND si.resource_id = r.id \
                   AND si.param_name IN ({}) \
                   AND {})",
                dialect.placeholder(1),
                dialect.placeholder(2),
                param_names
                    .iter()
                    .map(|slot| dialect.placeholder(*slot))
                    .collect::<Vec<_>>()
                    .join(","),
                list_match("si.value_reference", *refs),
            ),
        };
        predicate.push_str("\n  AND ");
        predicate.push_str(&membership);
    }
    predicate
}

/// Compiled output for a single ViewDefinition.
#[derive(Debug, Clone)]
pub struct EmittedSql {
    /// Parameterised SQL — a single `SELECT` (with CTEs allowed).
    pub sql: String,
    /// Output column names in projection order. Drives `row_to_json` in the
    /// runners.
    pub columns: Vec<String>,
    /// Per-column decode mode, parallel to `columns`.
    pub column_decodes: Vec<ColumnDecode>,
    /// Expression-local parameter allocation marker (`$N` / `?N`). Runtime
    /// filters allocate from the view's consumed constant bindings instead.
    pub next_param_index: usize,
}

/// Lowers a plan tree to SQL for the given dialect.
///
/// # Errors
///
/// Returns [`SofError::InvalidViewDefinition`] for structurally invalid plans
/// and [`SofError::Uncompilable`] for IR shapes outside the implemented subset
/// at this stage.
pub fn emit_plan(plan: &PlanNode, dialect: &dyn Dialect) -> Result<EmittedSql, SofError> {
    match plan {
        PlanNode::Union(branches) => emit_union(branches, dialect),
        PlanNode::Project { parent, .. } if contains_recurse(parent) => {
            emit_recurse_select(plan, dialect)
        }
        _ => emit_select(plan, dialect),
    }
}

/// Walks a plan node downward to detect whether it's rooted in a `Recurse`
/// (possibly wrapped in `LateralUnnest` / `Filter` layers). Used by
/// [`emit_plan`] to dispatch to the recursive-CTE emitter.
fn contains_recurse(node: &PlanNode) -> bool {
    match node {
        PlanNode::Recurse { .. } => true,
        PlanNode::LateralUnnest { parent, .. } | PlanNode::Filter { parent, .. } => {
            contains_recurse(parent)
        }
        _ => false,
    }
}

// ============================================================================
// Top-level SELECT assembly
// ============================================================================

/// Emit a `SELECT … FROM … WHERE … ORDER BY` for a non-Union plan.
fn emit_select(plan: &PlanNode, dialect: &dyn Dialect) -> Result<EmittedSql, SofError> {
    // Tear the tree apart from the top down: must be Project at the root.
    let (project_cols, body) = match plan {
        PlanNode::Project { parent, columns } => (columns.as_slice(), parent.as_ref()),
        _ => {
            return Err(SofError::InvalidViewDefinition(
                "plan tree must have a Project node at the top".to_string(),
            ));
        }
    };

    // Walk down through Filter / LateralUnnest / Scan, collecting pieces.
    let mut frame = Frame::new();
    walk_body(body, dialect, &mut frame)?;

    let scan = frame
        .scan
        .as_ref()
        .ok_or_else(|| SofError::InvalidViewDefinition("plan has no Scan node".to_string()))?;

    // Build SELECT clause from the project columns.
    let mut select_parts: Vec<String> = Vec::with_capacity(project_cols.len());
    let mut columns: Vec<String> = Vec::with_capacity(project_cols.len());
    let mut column_decodes: Vec<ColumnDecode> = Vec::with_capacity(project_cols.len());
    for col in project_cols {
        let mut expr_ctx = ExprCtx::new(dialect, frame.next_param);
        let (casted, decode) = project_column(col, &mut expr_ctx)?;
        frame.next_param = expr_ctx.next_param;
        select_parts.push(format!("{casted} AS \"{}\"", sanitize_ident(&col.name)?));
        columns.push(col.name.clone());
        column_decodes.push(decode);
    }

    if select_parts.is_empty() {
        return Err(SofError::InvalidViewDefinition(
            "no output columns".to_string(),
        ));
    }
    let select_clause = select_parts.join(",\n  ");

    // Build FROM clause: `resources r` + any LATERAL joins, in order of appearance
    // from the bottom of the tree upward (Scan first, then unnests).
    let mut from_clause = format!("{} r", scan.table);
    for join in &frame.joins {
        from_clause.push('\n');
        from_clause.push_str(&join.sql);
    }

    // WHERE clause: tenant predicate first (so `$1`/`$2` line up), then filters.
    let mut where_parts = vec![resource_scan_predicate(&scan.filter, dialect)];
    for pred in &frame.predicates {
        where_parts.push(pred.clone());
    }
    let where_clause = where_parts.join("\n  AND ");

    // Keep occurrences in the evaluator's declaration/chain order. Order
    // metadata belongs to each live join: indexed SQLite lowering can hoist
    // joins into a subquery and must remove their outer alias references too.
    // A keyset page is ordered by id alone, the order its index serves.
    let id_collation = id_collation(dialect);
    let mut order_parts = vec![format!("r.id COLLATE {id_collation}")];
    if scan.filter.id_range.is_none() {
        order_parts.insert(0, "r.last_updated".to_string());
    }
    order_parts.extend(
        frame
            .joins
            .iter()
            .filter_map(|join| join.occurrence_order.clone()),
    );
    let order_clause = order_parts.join(", ");

    let sql = format!(
        "SELECT\n  {select_clause}\nFROM {from_clause}\nWHERE {where_clause}\nORDER BY {order_clause}"
    );

    Ok(EmittedSql {
        sql,
        columns,
        column_decodes,
        next_param_index: frame.next_param,
    })
}

/// Emit a `WITH RECURSIVE … SELECT … FROM <cte> [JOIN resources r ON r.id = <cte>.rid]`
/// query for a `Project` whose parent is a [`PlanNode::Recurse`]. The CTE
/// projects `(rid, node)` so sibling root columns (those whose path roots on
/// `r.data`) can resolve via a join back to `resources r`.
fn emit_recurse_select(plan: &PlanNode, dialect: &dyn Dialect) -> Result<EmittedSql, SofError> {
    let (project_cols, body) = match plan {
        PlanNode::Project { parent, columns } => (columns.as_slice(), parent.as_ref()),
        _ => unreachable!("emit_recurse_select called on non-Project plan"),
    };
    if !is_legacy_recurse_shape(body) {
        return emit_recurse_general(project_cols, body, dialect);
    }

    // Walk down past any LateralUnnest layers wrapping the Recurse — those
    // are nested-forEach unnests that get JOINed onto the recursive CTE
    // alias. Collect their sources for later join construction.
    let mut extra_unnests: Vec<&PlanNode> = Vec::new();
    let mut cur = body;
    while let PlanNode::LateralUnnest { parent, .. } = cur {
        extra_unnests.push(cur);
        cur = parent.as_ref();
    }
    let recurse_node = cur;
    let (parent_plan, step_paths, out_alias) = match recurse_node {
        PlanNode::Recurse {
            parent,
            step_paths,
            out_alias,
            ..
        } => (parent.as_ref(), step_paths.as_slice(), out_alias.as_str()),
        _ => unreachable!("emit_recurse_select called with non-Recurse parent"),
    };

    // Walk the parent plan to collect tenant predicate + any top-level
    // `where[]` filters. We expect a Scan with optional Filter chain — no
    // unnests at this level (rejected upstream).
    let mut frame = Frame::new();
    walk_body(parent_plan, dialect, &mut frame)?;
    let scan = frame
        .scan
        .as_ref()
        .ok_or_else(|| SofError::InvalidViewDefinition("plan has no Scan node".to_string()))?;

    // The same bound filter guards every seed and the optional resource rejoin.
    let tenant_pred = resource_scan_predicate(&scan.filter, dialect);
    let mut where_pred = tenant_pred.clone();
    for p in &frame.predicates {
        where_pred.push_str("\n  AND ");
        where_pred.push_str(p);
    }

    // The CTE carries a sortable pre-order `ord_path` column so `%rowIndex` can
    // rank traversal nodes. SQLite always emits it; PostgreSQL emits it only for
    // a single step path (a single recursive self-reference) — multi-path PG
    // repeats keep the original `(rid, node)` shape and don't support
    // `%rowIndex`, which no fixture exercises.
    let is_sqlite = dialect.lateral_keyword().is_empty();
    let emit_ord_path = is_sqlite || step_paths.len() == 1;

    // Build seed branches — one SELECT per step path.
    let mut seed_branches: Vec<String> = Vec::with_capacity(step_paths.len());
    for path in step_paths {
        let src = SqlExpr::JsonPath {
            root: "r.data".to_string(),
            path: path.clone(),
        };
        let branch = if is_sqlite {
            // SQLite carries a zero-padded `ord_path` of the element's `json_each`
            // array index so the outer query can rank traversal nodes in
            // pre-order for `%rowIndex` (see REPEAT_ORD_PATH_COL).
            format!(
                "SELECT r.id AS rid, je.value AS node, printf('%010d', je.key) AS {ord}\n  \
                 FROM {} r, {} je\n  WHERE {}",
                scan.table,
                emit_sqlite_unnest_source(&src)?,
                where_pred,
                ord = REPEAT_ORD_PATH_COL,
            )
        } else {
            // PostgreSQL: `WITH ORDINALITY` exposes the 1-based element position
            // (only when we emit `ord_path`).
            let unnest = dialect.unnest_array(&emit_pg_unnest_source(&src)?);
            let lateral = dialect.lateral_keyword();
            if emit_ord_path {
                format!(
                    "SELECT r.id AS rid, je.value AS node, \
                     lpad((je.ord - 1)::text, 10, '0') AS {ord}\n  \
                     FROM {} r JOIN {lateral}{unnest} WITH ORDINALITY AS je(value, ord) ON TRUE\n  \
                     WHERE {}",
                    scan.table,
                    where_pred,
                    ord = REPEAT_ORD_PATH_COL,
                )
            } else {
                format!(
                    "SELECT r.id AS rid, je.value AS node\n  \
                     FROM {} r JOIN {lateral}{unnest} AS je(value) ON TRUE\n  WHERE {}",
                    scan.table, where_pred
                )
            }
        };
        seed_branches.push(branch);
    }

    let step_branches = build_step_branches(step_paths, out_alias, dialect, emit_ord_path, false)?;

    let cte_body = assemble_cte_body(seed_branches, step_branches, dialect);

    // Determine which columns reference `r.data` (sibling root cols from
    // sibling clauses) — they need a JOIN back to `resources r`.
    let needs_resource_join = project_cols
        .iter()
        .any(|c| column_refers_to_resource(&c.expr));

    // Build SELECT clause.
    let mut select_parts: Vec<String> = Vec::with_capacity(project_cols.len());
    let mut columns: Vec<String> = Vec::with_capacity(project_cols.len());
    let mut column_decodes: Vec<ColumnDecode> = Vec::with_capacity(project_cols.len());
    for col in project_cols {
        let mut ctx = ExprCtx::new(dialect, frame.next_param);
        let (casted, decode) = project_column(col, &mut ctx)?;
        frame.next_param = ctx.next_param;
        select_parts.push(format!("{casted} AS \"{}\"", sanitize_ident(&col.name)?));
        columns.push(col.name.clone());
        column_decodes.push(decode);
    }

    let mut from_clause = if needs_resource_join {
        format!(
            "{} JOIN {} r ON r.id = {}.rid AND {}",
            out_alias, scan.table, out_alias, tenant_pred
        )
    } else {
        out_alias.to_string()
    };

    // Append any forEach unnests stacked above the recurse — emitted in
    // outer-to-inner order matching how walk_body would assemble them.
    // Iterate `extra_unnests` in reverse since we collected from outermost
    // (closest to Project) downward.
    for layer in extra_unnests.iter().rev() {
        if let PlanNode::LateralUnnest {
            source,
            out_alias: alias,
            left_join,
            on_filter,
            flat_index,
            ..
        } = layer
        {
            // Rendered through the shared unnest renderer so PostgreSQL keeps
            // `WITH ORDINALITY` (needed by `%rowIndex`) over a repeat.
            let mut scratch = Frame::new();
            scratch.next_param = frame.next_param;
            push_lateral_unnest(
                source,
                alias,
                left_join,
                on_filter,
                flat_index,
                dialect,
                &mut scratch,
            )?;
            frame.next_param = scratch.next_param;
            for join in scratch.joins {
                from_clause.push('\n');
                from_clause.push_str(&join.sql);
            }
        }
    }

    // The CTE carries the extra `ord_path` column (used by `%rowIndex` ranking)
    // whenever we emit it; otherwise it keeps the original `(rid, node)` shape.
    let cte_cols = if emit_ord_path {
        format!("rid, node, {REPEAT_ORD_PATH_COL}")
    } else {
        "rid, node".to_string()
    };
    let sql = format!(
        "WITH RECURSIVE {out_alias}({cte_cols}) AS (\n  {cte_body}\n)\nSELECT\n  {}\nFROM {from_clause}",
        select_parts.join(",\n  ")
    );

    Ok(EmittedSql {
        sql,
        columns,
        column_decodes,
        next_param_index: frame.next_param,
    })
}

/// Builds the recursive (step) branches of a `repeat` CTE: one `SELECT` per
/// step path walking off `<out_alias>.node`. `with_pkey` carries the scope-key
/// column of a keyed CTE (see [`emit_recurse_general`]) through the recursion.
fn build_step_branches(
    step_paths: &[JsonPath],
    out_alias: &str,
    dialect: &dyn Dialect,
    emit_ord_path: bool,
    with_pkey: bool,
) -> Result<Vec<String>, SofError> {
    let is_sqlite = dialect.lateral_keyword().is_empty();
    let pk = if with_pkey {
        format!(", {out_alias}.pkey")
    } else {
        String::new()
    };
    let mut step_branches: Vec<String> = Vec::with_capacity(step_paths.len());
    // Step branches — walk each path off `<alias>.node`. Multi-segment
    // step paths (`answer.item`) chain a lateral unnest per Field so
    // path-through-array flattening matches FHIRPath semantics.
    //
    // PG additionally requires that a recursive CTE reference its own name
    // at most once. When `step_paths` has more than one entry, fold all
    // step navigations into a single `SELECT … FROM rec_0, LATERAL (path₁
    // UNION ALL path₂ UNION ALL …)` so `rec_0` is referenced exactly once.
    let is_pg_dialect = !dialect.lateral_keyword().is_empty();
    let mut pg_lateral_branches: Vec<String> = Vec::new();
    for path in step_paths {
        let segs: Vec<&str> = path
            .0
            .iter()
            .filter_map(|s| match s {
                PathStep::Field(n) => Some(n.as_str()),
                _ => None,
            })
            .collect();
        if segs.is_empty() {
            continue;
        }
        let mut prev_root = format!("{out_alias}.node");
        let mut from_parts: Vec<String> = Vec::new();
        for (i, field) in segs.iter().enumerate() {
            let alias = format!("rs{i}");
            let src = SqlExpr::JsonPath {
                root: prev_root.clone(),
                path: super::ir::JsonPath(vec![PathStep::Field((*field).to_string())]),
            };
            if is_sqlite {
                from_parts.push(format!("{} {alias}", emit_sqlite_unnest_source(&src)?));
            } else {
                // The leaf unnest gets `WITH ORDINALITY` so the step branch can
                // extend `ord_path` with this child's position (single-path PG
                // only — see `emit_ord_path`).
                let is_leaf = i == segs.len() - 1;
                if emit_ord_path && is_leaf && !(is_pg_dialect && step_paths.len() > 1) {
                    from_parts.push(format!(
                        "{}{} WITH ORDINALITY AS {alias}(value, ord)",
                        dialect.lateral_keyword(),
                        dialect.unnest_array(&emit_pg_unnest_source(&src)?)
                    ));
                } else {
                    from_parts.push(format!(
                        "{}{} AS {alias}(value)",
                        dialect.lateral_keyword(),
                        dialect.unnest_array(&emit_pg_unnest_source(&src)?)
                    ));
                }
            }
            prev_root = format!("{alias}.value");
        }
        let leaf_alias = format!("rs{}", segs.len() - 1);
        if is_pg_dialect && step_paths.len() > 1 {
            // Build a sub-SELECT that returns just the leaf value; we'll
            // wrap them all in one LATERAL below.
            let mut from_clause = String::new();
            for (i, fp) in from_parts.iter().enumerate() {
                if i == 0 {
                    from_clause.push_str(fp);
                } else {
                    from_clause.push_str(" JOIN ");
                    from_clause.push_str(fp);
                    from_clause.push_str(" ON TRUE");
                }
            }
            pg_lateral_branches.push(format!("SELECT {leaf_alias}.value FROM {from_clause}"));
        } else if dialect.lateral_keyword().is_empty() {
            // SQLite: extend the parent's `ord_path` with this child's
            // array index so descendants sort immediately after their
            // parent (pre-order) and before the parent's later siblings.
            let from_clause = format!("{out_alias}, {}", from_parts.join(", "));
            step_branches.push(format!(
                "SELECT {out_alias}.rid, {leaf_alias}.value AS node, \
                 {out_alias}.{ord} || '.' || printf('%010d', {leaf_alias}.key) AS {ord}{pk}\n  \
                 FROM {from_clause}",
                ord = REPEAT_ORD_PATH_COL,
            ));
        } else {
            let mut s = out_alias.to_string();
            for fp in &from_parts {
                s.push_str(" JOIN ");
                s.push_str(fp);
                s.push_str(" ON TRUE");
            }
            if emit_ord_path {
                // Extend the parent's `ord_path` with this child's position.
                step_branches.push(format!(
                    "SELECT {out_alias}.rid, {leaf_alias}.value AS node, \
                     {out_alias}.{ord} || '.' || lpad(({leaf_alias}.ord - 1)::text, 10, '0') AS {ord}{pk}\n  \
                     FROM {s}",
                    ord = REPEAT_ORD_PATH_COL,
                ));
            } else {
                step_branches.push(format!(
                    "SELECT {out_alias}.rid, {leaf_alias}.value AS node{pk}\n  FROM {s}"
                ));
            }
        }
    }
    if is_pg_dialect && !pg_lateral_branches.is_empty() {
        // Single recursive SELECT that references `rec_0` once and
        // explores all step paths via a lateral UNION ALL of leaf values.
        let unioned = pg_lateral_branches.join("\n    UNION ALL\n    ");
        step_branches.push(format!(
            "SELECT {out_alias}.rid, _step.value AS node{pk}\n  \
             FROM {out_alias}, LATERAL ({unioned}) AS _step(value)"
        ));
    }
    Ok(step_branches)
}

/// Joins seed and step branches into one CTE body. PG requires exactly one
/// `UNION ALL` between the non-recursive and recursive terms, so multi-branch
/// sides are parenthesised there; SQLite is fine with the flat form.
fn assemble_cte_body(
    mut seed_branches: Vec<String>,
    mut step_branches: Vec<String>,
    dialect: &dyn Dialect,
) -> String {
    let is_pg = !dialect.lateral_keyword().is_empty();
    if is_pg && (seed_branches.len() > 1 || step_branches.len() > 1) {
        let seeds = if seed_branches.len() == 1 {
            seed_branches.remove(0)
        } else {
            format!("({})", seed_branches.join("\n  UNION ALL\n  "))
        };
        let steps = if step_branches.is_empty() {
            String::new()
        } else if step_branches.len() == 1 {
            step_branches.remove(0)
        } else {
            format!("({})", step_branches.join("\n  UNION ALL\n  "))
        };
        if steps.is_empty() {
            seeds
        } else {
            format!("{seeds}\n  UNION ALL\n  {steps}")
        }
    } else {
        let mut all = seed_branches;
        all.extend(step_branches);
        all.join("\n  UNION ALL\n  ")
    }
}

/// True for the original single-descent shape: exactly one `Recurse` seeded
/// from the resource, sitting directly on `Scan`/`Filter` nodes, with only
/// unnests above it. Those plans keep the unkeyed `(rid, node[, ord_path])` CTE.
fn is_legacy_recurse_shape(body: &PlanNode) -> bool {
    fn scan_filter_chain(node: &PlanNode) -> bool {
        match node {
            PlanNode::Scan { .. } => true,
            PlanNode::Filter { parent, .. } => scan_filter_chain(parent),
            _ => false,
        }
    }
    let mut cur = body;
    while let PlanNode::LateralUnnest { parent, .. } = cur {
        cur = parent.as_ref();
    }
    match cur {
        PlanNode::Recurse {
            parent, seed_root, ..
        } => seed_root == "r.data" && scan_filter_chain(parent),
        _ => false,
    }
}

/// One iteration step (`LateralUnnest` or `Recurse`) of a general repeat plan,
/// with the step that produces its focus (`None` = the resource document).
struct ScopeStep<'a> {
    node: &'a PlanNode,
    alias: &'a str,
    parent: Option<usize>,
}

/// Alias behind a focus root (`fe.value` → `fe`, `rec_0.node` → `rec_0`);
/// `None` for the resource document.
fn scope_root_alias(root: &str) -> Option<&str> {
    if root.starts_with("r.data") {
        None
    } else {
        Some(root.rsplit_once('.').map_or(root, |(alias, _)| alias))
    }
}

fn resolve_scope(root: &str, steps: &[ScopeStep<'_>]) -> Result<Option<usize>, SofError> {
    match scope_root_alias(root) {
        None => Ok(None),
        Some(alias) => steps
            .iter()
            .position(|s| s.alias == alias)
            .map(Some)
            .ok_or_else(|| SofError::Uncompilable {
                reason: format!("repeat/forEach focus '{root}' has no enclosing iteration"),
            }),
    }
}

/// A `repeat` node is identified by `(pkey, ord_path)` only when `ord_path`
/// is unique per node: one path of one segment (the leaf index alone is not
/// unique for `a.b` paths, and multi-path ids collide across paths).
fn repeat_node_is_identifiable(step_paths: &[JsonPath]) -> bool {
    step_paths.len() == 1
        && step_paths[0]
            .0
            .iter()
            .filter(|s| matches!(s, PathStep::Field(_)))
            .count()
            == 1
}

/// SQL expression identifying the iteration row that produced `scope`, built
/// from the aliases visible in a FROM clause that contains `scope`'s chain.
/// Equal keys mean "same enclosing element", which is what a keyed CTE joins on.
fn scope_key(
    scope: Option<usize>,
    steps: &[ScopeStep<'_>],
    dialect: &dyn Dialect,
) -> Result<String, SofError> {
    let is_sqlite = dialect.lateral_keyword().is_empty();
    let Some(i) = scope else {
        return Ok(if is_sqlite { "''" } else { "''::text" }.to_string());
    };
    let step = &steps[i];
    let alias = step.alias;
    match step.node {
        PlanNode::LateralUnnest { flat_index, .. } => {
            let parent = scope_key(step.parent, steps, dialect)?;
            let idx = if flat_index.is_some() {
                "'0'".to_string()
            } else if is_sqlite {
                format!("printf('%010d', COALESCE({alias}.rowid, -1))")
            } else {
                format!("lpad(COALESCE({alias}.ordinality, -1)::text, 10, '0')")
            };
            Ok(format!("{parent} || '/' || {idx}"))
        }
        PlanNode::Recurse { step_paths, .. } => {
            if !repeat_node_is_identifiable(step_paths) {
                return Err(SofError::Uncompilable {
                    reason: "a repeat with several or multi-segment paths cannot enclose another \
                             repeat or forEach yet"
                        .to_string(),
                });
            }
            Ok(format!(
                "{alias}.pkey || '/' || {alias}.{REPEAT_ORD_PATH_COL}"
            ))
        }
        _ => Err(SofError::InvalidViewDefinition(
            "scope step must be an unnest or a recurse".to_string(),
        )),
    }
}

/// For a step that is not itself a `forEachOrNull` but sits below one: the SQL
/// predicates "this step matched" and "its nearest `forEachOrNull` ancestor
/// missed". Indexed (`[N]`) steps are not covered.
fn optional_scope_guard(
    i: usize,
    steps: &[ScopeStep<'_>],
    is_sqlite: bool,
) -> Option<(String, String)> {
    let matched = match steps[i].node {
        PlanNode::Recurse { .. } => format!("{}.rid IS NOT NULL", steps[i].alias),
        PlanNode::LateralUnnest {
            left_join: false,
            flat_index: None,
            ..
        } => format!("{} IS NOT NULL", miss_signal(steps[i].alias, is_sqlite)),
        _ => return None,
    };
    let mut p = steps[i].parent;
    while let Some(j) = p {
        if let PlanNode::LateralUnnest {
            left_join: true,
            flat_index,
            ..
        } = steps[j].node
        {
            let missed = if flat_index.is_some() {
                format!("{}.value IS NULL", steps[j].alias)
            } else {
                format!("{} IS NULL", miss_signal(steps[j].alias, is_sqlite))
            };
            return Some((matched, missed));
        }
        p = steps[j].parent;
    }
    None
}

/// Column that is NULL exactly when a LEFT-joined unnest found no element.
fn miss_signal(alias: &str, is_sqlite: bool) -> String {
    if is_sqlite {
        format!("{alias}.rowid")
    } else {
        format!("{alias}.ordinality")
    }
}

/// FROM-clause join for one step of a general repeat plan: an unnest renders
/// like [`walk_body`] does; a recursive CTE joins on `rid` plus its scope key.
fn render_step_join(
    i: usize,
    steps: &[ScopeStep<'_>],
    dialect: &dyn Dialect,
    next_param: &mut usize,
) -> Result<String, SofError> {
    let step = &steps[i];
    match step.node {
        PlanNode::LateralUnnest {
            source,
            out_alias,
            left_join,
            on_filter,
            flat_index,
            ..
        } => {
            let mut scratch = Frame::new();
            scratch.next_param = *next_param;
            push_lateral_unnest(
                source,
                out_alias,
                left_join,
                on_filter,
                flat_index,
                dialect,
                &mut scratch,
            )?;
            *next_param = scratch.next_param;
            scratch
                .joins
                .into_iter()
                .next()
                .map(|j| j.sql)
                .ok_or_else(|| {
                    SofError::InvalidViewDefinition("unnest rendered no join".to_string())
                })
        }
        _ => {
            let alias = step.alias;
            let key = scope_key(step.parent, steps, dialect)?;
            Ok(format!(
                "JOIN {alias} ON {alias}.rid = r.id AND {alias}.pkey = {key}"
            ))
        }
    }
}

/// Emits a plan with `Recurse` nodes anywhere in the iteration chain: nested
/// in a `forEach`, in another `repeat`, or as siblings.
///
/// Every recursive CTE is keyed: besides `(rid, node[, ord_path])` it carries
/// `pkey`, the scope key of the row it was seeded from (see [`scope_key`]). Its
/// seed replays only the ancestors of its seed alias, and the main query joins
/// it back on `rid` and the same key, so each enclosing element sees exactly
/// its own descendants. Siblings share a key and cross-multiply, as `select`
/// siblings do.
fn emit_recurse_general(
    project_cols: &[Column],
    body: &PlanNode,
    dialect: &dyn Dialect,
) -> Result<EmittedSql, SofError> {
    let is_sqlite = dialect.lateral_keyword().is_empty();

    // Flatten the chain bottom-up: Scan, leading filters, then iteration steps.
    let mut chain: Vec<&PlanNode> = Vec::new();
    let mut cur = body;
    loop {
        chain.push(cur);
        match cur {
            PlanNode::Scan { .. } => break,
            PlanNode::Filter { parent, .. }
            | PlanNode::LateralUnnest { parent, .. }
            | PlanNode::Recurse { parent, .. } => cur = parent.as_ref(),
            _ => {
                return Err(SofError::InvalidViewDefinition(
                    "unexpected node in a repeat plan chain".to_string(),
                ));
            }
        }
    }
    chain.reverse();
    let first_step = chain
        .iter()
        .position(|n| matches!(n, PlanNode::LateralUnnest { .. } | PlanNode::Recurse { .. }))
        .unwrap_or(chain.len());

    // Scan plus the top-level `where[]` filters: they guard every seed.
    let mut frame = Frame::new();
    walk_body(chain[first_step - 1], dialect, &mut frame)?;
    let scan = frame
        .scan
        .as_ref()
        .ok_or_else(|| SofError::InvalidViewDefinition("plan has no Scan node".to_string()))?;
    let table = scan.table;
    let tenant_pred = resource_scan_predicate(&scan.filter, dialect);
    let mut seed_where = tenant_pred.clone();
    for p in &frame.predicates {
        seed_where.push_str("\n  AND ");
        seed_where.push_str(p);
    }

    let mut steps: Vec<ScopeStep<'_>> = Vec::new();
    let mut mid_filters: Vec<&SqlExpr> = Vec::new();
    for node in &chain[first_step..] {
        match node {
            PlanNode::Filter { predicate, .. } => mid_filters.push(predicate),
            PlanNode::LateralUnnest {
                source,
                out_alias,
                flat_index,
                ..
            } => {
                if is_sqlite && flat_index.is_some() {
                    return Err(SofError::Uncompilable {
                        reason: "an indexed forEach cannot be combined with repeat on SQLite"
                            .to_string(),
                    });
                }
                let parent = match unnest_source_root(source) {
                    Some(root) => resolve_scope(root, &steps)?,
                    None => steps.len().checked_sub(1),
                };
                steps.push(ScopeStep {
                    node,
                    alias: out_alias,
                    parent,
                });
            }
            PlanNode::Recurse {
                seed_root,
                out_alias,
                ..
            } => {
                let parent = resolve_scope(seed_root, &steps)?;
                steps.push(ScopeStep {
                    node,
                    alias: out_alias,
                    parent,
                });
            }
            _ => {}
        }
    }

    // One keyed CTE per Recurse, in chain order.
    let mut ctes: Vec<String> = Vec::new();
    let mut keyed: Vec<String> = Vec::new();
    for step in &steps {
        let PlanNode::Recurse {
            step_paths,
            out_alias,
            seed_root,
            ..
        } = step.node
        else {
            continue;
        };
        let emit_ord_path = is_sqlite || step_paths.len() == 1;

        // Seed prefix: only the ancestors the seed alias depends on.
        let mut ancestors: Vec<usize> = Vec::new();
        let mut p = step.parent;
        while let Some(i) = p {
            ancestors.push(i);
            p = steps[i].parent;
        }
        ancestors.sort_unstable();
        let mut prefix = String::new();
        for i in ancestors {
            prefix.push('\n');
            prefix.push_str(&render_step_join(
                i,
                &steps,
                dialect,
                &mut frame.next_param,
            )?);
        }
        let key = scope_key(step.parent, &steps, dialect)?;

        let mut seeds: Vec<String> = Vec::with_capacity(step_paths.len());
        for path in step_paths {
            let src = SqlExpr::JsonPath {
                root: seed_root.clone(),
                path: path.clone(),
            };
            let branch = if is_sqlite {
                format!(
                    "SELECT r.id AS rid, je.value AS node, printf('%010d', je.key) AS {ord}, \
                     {key} AS pkey\n  FROM {table} r{prefix}\n  JOIN {} je ON 1=1\n  WHERE {seed_where}",
                    emit_sqlite_unnest_source(&src)?,
                    ord = REPEAT_ORD_PATH_COL,
                )
            } else {
                let unnest = dialect.unnest_array(&emit_pg_unnest_source(&src)?);
                let lateral = dialect.lateral_keyword();
                if emit_ord_path {
                    format!(
                        "SELECT r.id AS rid, je.value AS node, \
                         lpad((je.ord - 1)::text, 10, '0') AS {ord}, {key} AS pkey\n  \
                         FROM {table} r{prefix}\n  \
                         JOIN {lateral}{unnest} WITH ORDINALITY AS je(value, ord) ON TRUE\n  \
                         WHERE {seed_where}",
                        ord = REPEAT_ORD_PATH_COL,
                    )
                } else {
                    format!(
                        "SELECT r.id AS rid, je.value AS node, {key} AS pkey\n  \
                         FROM {table} r{prefix}\n  \
                         JOIN {lateral}{unnest} AS je(value) ON TRUE\n  WHERE {seed_where}"
                    )
                }
            };
            seeds.push(branch);
        }
        let step_branches =
            build_step_branches(step_paths, out_alias, dialect, emit_ord_path, true)?;
        let cte_body = assemble_cte_body(seeds, step_branches, dialect);
        let cols = if emit_ord_path {
            format!("rid, node, {REPEAT_ORD_PATH_COL}, pkey")
        } else {
            "rid, node, pkey".to_string()
        };
        ctes.push(format!("{out_alias}({cols}) AS (\n  {cte_body}\n)"));
        keyed.push(out_alias.clone());
    }

    // Main query: the resource, then every step in chain order. A step below
    // a `forEachOrNull` is LEFT-joined and kept when that ancestor missed
    // (an empty `forEachOrNull` yields one null row whatever is nested in
    // it); a present ancestor with no match still drops the row.
    let mut from_clause = format!("{table} r");
    let mut where_parts = vec![seed_where];
    for i in 0..steps.len() {
        from_clause.push('\n');
        let mut join = render_step_join(i, &steps, dialect, &mut frame.next_param)?;
        if let Some((matched, missed)) = optional_scope_guard(i, &steps, is_sqlite)
            && let Some(rest) = join.strip_prefix("JOIN ")
        {
            join = format!("LEFT JOIN {rest}");
            where_parts.push(format!("({matched} OR {missed})"));
        }
        from_clause.push_str(&join);
    }

    for pred in &mid_filters {
        let mut ctx = ExprCtx::new(dialect, frame.next_param);
        ctx.keyed_repeats = keyed.clone();
        let sql = lower_expr(pred, &mut ctx)?;
        frame.next_param = ctx.next_param;
        where_parts.push(dialect.truthy_predicate(&sql));
    }

    let mut select_parts: Vec<String> = Vec::with_capacity(project_cols.len());
    let mut columns: Vec<String> = Vec::with_capacity(project_cols.len());
    let mut column_decodes: Vec<ColumnDecode> = Vec::with_capacity(project_cols.len());
    for col in project_cols {
        let mut ctx = ExprCtx::new(dialect, frame.next_param);
        ctx.keyed_repeats = keyed.clone();
        let (casted, decode) = project_column(col, &mut ctx)?;
        frame.next_param = ctx.next_param;
        select_parts.push(format!("{casted} AS \"{}\"", sanitize_ident(&col.name)?));
        columns.push(col.name.clone());
        column_decodes.push(decode);
    }

    let sql = format!(
        "WITH RECURSIVE {}\nSELECT\n  {}\nFROM {from_clause}\nWHERE {}",
        ctes.join(",\n"),
        select_parts.join(",\n  "),
        where_parts.join("\n  AND "),
    );
    Ok(EmittedSql {
        sql,
        columns,
        column_decodes,
        next_param_index: frame.next_param,
    })
}

/// Returns true when `expr` (or any sub-expression) navigates off the
/// `r.data` document — used by `emit_recurse_select` to decide whether to
/// JOIN the recursive CTE back to `resources`.
fn column_refers_to_resource(expr: &SqlExpr) -> bool {
    match expr {
        SqlExpr::JsonPath { root, .. } => root == "r.data" || root.starts_with("r.data"),
        SqlExpr::Cast { inner, .. }
        | SqlExpr::UnaryOp { inner, .. }
        | SqlExpr::AsJson(inner)
        | SqlExpr::Alias { inner, .. } => column_refers_to_resource(inner),
        SqlExpr::BinOp { lhs, rhs, .. } => {
            column_refers_to_resource(lhs) || column_refers_to_resource(rhs)
        }
        SqlExpr::Case { arms, else_ } => {
            arms.iter()
                .any(|(c, v)| column_refers_to_resource(c) || column_refers_to_resource(v))
                || else_.as_deref().is_some_and(column_refers_to_resource)
        }
        SqlExpr::Coalesce(parts) => parts.iter().any(column_refers_to_resource),
        SqlExpr::NullIf(a, b) => column_refers_to_resource(a) || column_refers_to_resource(b),
        SqlExpr::ReferenceKey { reference, .. } => column_refers_to_resource(reference),
        SqlExpr::Boundary { source, .. } => column_refers_to_resource(source),
        SqlExpr::CollectionAgg { root, .. } => root.starts_with("r.data"),
        SqlExpr::JoinAggregate { outer_focus, .. } => column_refers_to_resource(outer_focus),
        SqlExpr::WhereScalar {
            focus,
            predicate,
            projection,
            ..
        } => {
            column_refers_to_resource(focus)
                || column_refers_to_resource(predicate)
                || column_refers_to_resource(projection)
        }
        _ => false,
    }
}

/// Emit a `UNION ALL` query — each branch is emitted as a standalone SELECT,
/// and trailing `ORDER BY` is stripped from each. The compound query is left
/// unordered: row order is not part of the SoF contract, and a sort would hold
/// back every row until the whole result is built.
fn emit_union(branches: &[PlanNode], dialect: &dyn Dialect) -> Result<EmittedSql, SofError> {
    if branches.is_empty() {
        return Err(SofError::InvalidViewDefinition(
            "unionAll branches list is empty".to_string(),
        ));
    }

    let mut branch_sqls: Vec<String> = Vec::with_capacity(branches.len());
    let mut columns: Option<Vec<String>> = None;
    let mut column_decodes: Vec<ColumnDecode> = Vec::new();
    let mut next_param = 3usize;

    for branch in branches {
        let emitted = emit_plan(branch, dialect)?;

        match &columns {
            None => {
                columns = Some(emitted.columns.clone());
                column_decodes = emitted.column_decodes.clone();
            }
            Some(expected) if *expected != emitted.columns => {
                return Err(SofError::Uncompilable {
                    reason: format!(
                        "unionAll branches produce different column schemas: {:?} vs {:?}",
                        expected, emitted.columns
                    ),
                });
            }
            _ => {
                // Same column names: reconcile decode modes by position.
                for (acc, d) in column_decodes.iter_mut().zip(&emitted.column_decodes) {
                    *acc = acc.merge(*d);
                }
            }
        }

        next_param = next_param.max(emitted.next_param_index);
        // Branches containing a `WITH RECURSIVE` (emitted for `repeat:`)
        // can't appear bare in a compound SELECT — neither dialect allows
        // `WITH ... UNION ALL WITH ...`. Wrap as `SELECT * FROM (WITH ...
        // SELECT ...)` so each branch is a plain SELECT operand.
        let body = strip_trailing_order_by(&emitted.sql).to_string();
        let needs_wrap = body.trim_start().starts_with("WITH");
        if needs_wrap {
            // PG requires every parenthesised subquery in `FROM` to have an
            // alias; SQLite allows it bare. Tag with a unique alias so PG
            // is happy without affecting SQLite (which ignores it).
            let alias = format!("_recurse_{}", branch_sqls.len());
            branch_sqls.push(format!("SELECT * FROM ({body}) AS {alias}"));
        } else {
            branch_sqls.push(body);
        }
    }

    let sql = branch_sqls.join("\nUNION ALL\n");
    Ok(EmittedSql {
        sql,
        columns: columns.unwrap_or_default(),
        column_decodes,
        next_param_index: next_param,
    })
}

// ============================================================================
// Frame: accumulates pieces of a single SELECT during the bottom-up walk
// ============================================================================

#[derive(Debug)]
struct Frame {
    scan: Option<ScanInfo>,
    /// Lateral joins, in the order they appear in the FROM clause.
    joins: Vec<JoinClause>,
    /// AND-composed predicates (excluding the tenant predicate).
    predicates: Vec<String>,
    /// Next free bound-parameter index — threaded through expression lowering
    /// so that predicates and column expressions allocate non-overlapping
    /// `$N` / `?N` slots.
    next_param: usize,
}

#[derive(Debug)]
struct ScanInfo {
    table: &'static str,
    filter: ResourceFilter,
}

#[derive(Debug)]
struct JoinClause {
    sql: String,
    /// Only aliases still visible in this SELECT may contribute an order key.
    occurrence_order: Option<String>,
}

impl Frame {
    fn new() -> Self {
        Self {
            scan: None,
            joins: Vec::new(),
            predicates: Vec::new(),
            // $1 = tenant_id, $2 = resource_type — both reserved by emit_select.
            next_param: 3,
        }
    }
}

/// Walks `body` (the sub-tree below the top `Project`), pushing pieces into
/// `frame` as it goes.
fn walk_body(node: &PlanNode, dialect: &dyn Dialect, frame: &mut Frame) -> Result<(), SofError> {
    match node {
        PlanNode::Scan { alias, filter, .. } => {
            if alias != "r" {
                return Err(SofError::Uncompilable {
                    reason: format!("Scan alias must be 'r' in current emitter (got '{alias}')"),
                });
            }
            frame.scan = Some(ScanInfo {
                table: RESOURCES_TABLE,
                filter: filter.clone(),
            });
            Ok(())
        }
        PlanNode::Filter { parent, predicate } => {
            walk_body(parent, dialect, frame)?;
            let mut ctx = ExprCtx::new(dialect, frame.next_param);
            let pred_sql = lower_expr(predicate, &mut ctx)?;
            frame.next_param = ctx.next_param;
            // FHIRPath three-valued boundary — empty / NULL filters the row
            // out. Dialect-specific because PG is strict-typed (text from
            // `->>` must be cast to boolean) while SQLite is permissive.
            frame.predicates.push(dialect.truthy_predicate(&pred_sql));
            Ok(())
        }
        PlanNode::LateralUnnest {
            parent,
            source,
            out_alias,
            left_join,
            on_filter,
            flat_index,
        } => {
            walk_body(parent, dialect, frame)?;
            let joins_before = frame.joins.len();
            push_lateral_unnest(
                source, out_alias, left_join, on_filter, flat_index, dialect, frame,
            )?;
            // A plain `forEach` below an EMPTY `forEachOrNull` keeps the null
            // row: LEFT-join it and accept "no match" only when the ancestor
            // missed (a present ancestor with no match still drops the row).
            if !*left_join
                && flat_index.is_none()
                && let Some(root) = unnest_source_root(source)
                && let Some((left, left_flat)) = nearest_left_scope(parent, root)
                && let Some(join) = frame.joins.get_mut(joins_before)
                && let Some(rest) = join.sql.strip_prefix("JOIN ")
            {
                let is_sqlite = dialect.lateral_keyword().is_empty();
                join.sql = format!("LEFT JOIN {rest}");
                let missed = if left_flat {
                    format!("{left}.value IS NULL")
                } else {
                    format!("{} IS NULL", miss_signal(&left, is_sqlite))
                };
                frame.predicates.push(format!(
                    "({} IS NOT NULL OR {missed})",
                    miss_signal(out_alias, is_sqlite)
                ));
            }
            Ok(())
        }
        PlanNode::Project { .. } => Err(SofError::InvalidViewDefinition(
            "nested Project nodes are not supported by the current emitter".to_string(),
        )),
        PlanNode::Union(_) => Err(SofError::InvalidViewDefinition(
            "Union node may only appear at the top of a plan".to_string(),
        )),
        PlanNode::Recurse { .. } => Err(SofError::Uncompilable {
            reason: "Recurse (repeat:) is not yet implemented in the emitter".to_string(),
        }),
    }
}

/// Renders one `LateralUnnest` as a FROM-clause join and pushes it onto
/// `frame.joins`. Shared by [`walk_body`] and the recursive-CTE emitter.
fn push_lateral_unnest(
    source: &SqlExpr,
    out_alias: &String,
    left_join: &bool,
    on_filter: &Option<SqlExpr>,
    flat_index: &Option<i64>,
    dialect: &dyn Dialect,
    frame: &mut Frame,
) -> Result<(), SofError> {
    let join_kw = if *left_join { "LEFT JOIN" } else { "JOIN" };
    let lateral = dialect.lateral_keyword();
    // Lower the optional ON-clause filter (used by `forEach` paths
    // that contain a trailing `where(crit)`).
    let extra_on = if let Some(filter) = on_filter {
        let mut ctx = ExprCtx::new(dialect, frame.next_param);
        let sql = lower_expr(filter, &mut ctx)?;
        frame.next_param = ctx.next_param;
        Some(sql)
    } else {
        None
    };
    let join_sql = if lateral.is_empty() {
        // SQLite — `json_each(<root>, '$.path')` two-arg form when the
        // source is a simple JSON path off the resource document;
        // falls back to `json_each(<sql_expr>)` for anything richer.
        let source_sql = emit_sqlite_unnest_source(source)?;
        let on = match &extra_on {
            Some(f) => format!("1=1 AND {f}"),
            None => "1=1".to_string(),
        };
        if let Some(idx) = flat_index {
            // `forEach: "<chain>[N]"` — FHIRPath indexes the
            // FLATTENED collection, not each per-step iteration.
            // Hoist any prior joins (collected for this select) into
            // the LIMITed subquery so the outer SELECT sees at most
            // one row per resource.
            let inner = format!("{out_alias}_src");
            let prior = std::mem::take(&mut frame.joins);
            let prior_sources: Vec<String> = prior
                .iter()
                .map(|j| {
                    j.sql
                        .strip_prefix("JOIN ")
                        .and_then(|s| s.find(" ON ").map(|i| s[..i].to_string()))
                        .unwrap_or_else(|| j.sql.clone())
                })
                .collect();
            let from_chain = if prior_sources.is_empty() {
                format!("{source_sql} {inner}")
            } else {
                format!("{}, {source_sql} {inner}", prior_sources.join(", "))
            };
            format!(
                "{join_kw} (SELECT {inner}.value AS value FROM {from_chain} \
                     WHERE {on} LIMIT 1 OFFSET {idx}) {out_alias} ON 1=1"
            )
        } else {
            format!("{join_kw} {source_sql} {out_alias} ON {on}")
        }
    } else {
        // PostgreSQL — `jsonb_array_elements(<json_value>)` over the
        // JSON-valued navigation (note: must use `->`, not `->>`).
        let source_sql = emit_pg_unnest_source(source)?;
        let unnest = dialect.unnest_array(&source_sql);
        let on = match &extra_on {
            Some(f) => format!("TRUE AND {f}"),
            None => "TRUE".to_string(),
        };
        if let Some(idx) = flat_index {
            format!(
                "{join_kw} LATERAL (SELECT value FROM {unnest} AS sub(value) \
                     WHERE {on} LIMIT 1 OFFSET {idx}) AS {out_alias}(value) ON TRUE"
            )
        } else {
            // `WITH ORDINALITY` exposes a 1-based `ordinality` column so
            // `%rowIndex` (see `lower_row_index`) can read the element's
            // position. Harmless for forEach queries that don't use it.
            format!(
                "{join_kw} {lateral}{unnest} WITH ORDINALITY AS {out_alias}(value, ordinality) ON {on}"
            )
        }
    };
    frame.joins.push(JoinClause {
        sql: join_sql,
        // Indexed sources emit at most one row and don't expose an
        // ordinal. Preserve their existing pick/index semantics.
        occurrence_order: flat_index
            .is_none()
            .then(|| occurrence_order(out_alias, dialect)),
    });
    Ok(())
}

/// SQL occurrence position, separate from the public `%rowIndex` value.
/// PostgreSQL ordinality remains bigint; SQLite's per-call json_each rowid
/// also covers scalar/object sources whose key is null or a member name.
fn occurrence_order(alias: &str, dialect: &dyn Dialect) -> String {
    let ordinal = if dialect.name() == "sqlite" {
        "rowid"
    } else {
        "ordinality"
    };
    format!("COALESCE({alias}.{ordinal}, -1)")
}

// ============================================================================
// Expression lowering
// ============================================================================

/// Mutable context threaded through [`lower_expr`] — tracks the next free
/// parameter slot so nested expressions don't reuse indices.
struct ExprCtx<'a> {
    dialect: &'a dyn Dialect,
    next_param: usize,
    /// Aliases of keyed `repeat` CTEs (those carrying a `pkey` scope column);
    /// their `%rowIndex` ranks restart per seed scope.
    keyed_repeats: Vec<String>,
}

impl<'a> ExprCtx<'a> {
    fn new(dialect: &'a dyn Dialect, next_param: usize) -> Self {
        Self {
            dialect,
            next_param,
            keyed_repeats: Vec::new(),
        }
    }
}

/// Lowers a `%rowIndex` reference to dialect-specific SQL (SQLite and PostgreSQL).
///
/// `COALESCE(.., 0)` on the `forEach` case covers the `forEachOrNull` empty
/// case: a LEFT JOIN miss yields a NULL key/ordinality, which the spec maps to a
/// null row with `%rowIndex` = 0.
fn lower_row_index(
    scope: &RowIndexScope,
    dialect: &dyn Dialect,
    keyed_repeats: &[String],
) -> Result<String, SofError> {
    let is_sqlite = dialect.lateral_keyword().is_empty();
    Ok(match scope {
        // Top level and non-iterating scopes are always 0.
        RowIndexScope::Top => "0".to_string(),
        RowIndexScope::ForEach(alias) if is_sqlite => {
            // SQLite `json_each` exposes a 0-based integer `key` for array
            // elements — exactly the iteration index.
            format!("COALESCE(CAST({alias}.key AS INTEGER), 0)")
        }
        RowIndexScope::ForEach(alias) => {
            // PostgreSQL `WITH ORDINALITY` exposes a 1-based ordinality.
            format!("COALESCE(CAST({alias}.ordinality AS INTEGER) - 1, 0)")
        }
        // Both dialects: the recursive CTE carries a sortable pre-order
        // `ord_path`; ranking it per resource yields the 0-based pre-order
        // position.
        RowIndexScope::Repeat(alias) => {
            // A keyed CTE holds one traversal per seed scope (`pkey`), so the
            // rank restarts for each enclosing element, not just per resource.
            let partition = if keyed_repeats.contains(alias) {
                format!("{alias}.rid, {alias}.pkey")
            } else {
                format!("{alias}.rid")
            };
            format!(
                "(DENSE_RANK() OVER (PARTITION BY {partition} ORDER BY {alias}.{REPEAT_ORD_PATH_COL}) - 1)"
            )
        }
    })
}

fn lower_expr(expr: &SqlExpr, ctx: &mut ExprCtx<'_>) -> Result<String, SofError> {
    match expr {
        SqlExpr::Lit(v) => lower_lit(v, ctx.dialect),
        SqlExpr::JsonPath { root, path } => Ok(lower_json_path(root, path, ctx.dialect)),
        SqlExpr::Param(n) => Ok(ctx.dialect.placeholder(*n)),
        SqlExpr::ColRef(name) => Ok(name.clone()),
        SqlExpr::RowIndex(scope) => lower_row_index(scope, ctx.dialect, &ctx.keyed_repeats),
        SqlExpr::Cast { inner, ty } => {
            let inner = lower_expr(inner, ctx)?;
            Ok(ctx.dialect.cast(&inner, *ty))
        }
        SqlExpr::BinOp { op, lhs, rhs } => lower_binop_dialect(*op, lhs, rhs, ctx),
        SqlExpr::UnaryOp { op, inner } => {
            let inner = lower_expr(inner, ctx)?;
            Ok(match op {
                UnaryOp::Not => format!("NOT ({inner})"),
                UnaryOp::IsNull => format!("({inner}) IS NULL"),
                UnaryOp::IsNotNull => format!("({inner}) IS NOT NULL"),
                UnaryOp::Neg => format!("-({inner})"),
            })
        }
        SqlExpr::Case { arms, else_ } => {
            let mut s = String::from("CASE");
            for (cond, val) in arms {
                let c = lower_expr(cond, ctx)?;
                let v = lower_expr(val, ctx)?;
                s.push_str(&format!(" WHEN {c} THEN {v}"));
            }
            if let Some(e) = else_ {
                let v = lower_expr(e, ctx)?;
                s.push_str(&format!(" ELSE {v}"));
            }
            s.push_str(" END");
            Ok(s)
        }
        SqlExpr::Coalesce(parts) => {
            let parts: Result<Vec<String>, _> = parts.iter().map(|p| lower_expr(p, ctx)).collect();
            Ok(format!("coalesce({})", parts?.join(", ")))
        }
        SqlExpr::NullIf(a, b) => {
            let a = lower_expr(a, ctx)?;
            let b = lower_expr(b, ctx)?;
            Ok(format!("nullif({a}, {b})"))
        }
        SqlExpr::AsJson(inner) => {
            let inner = lower_expr(inner, ctx)?;
            Ok(ctx.dialect.cast(&inner, SqlType::Json))
        }
        SqlExpr::JsonAgg(_) | SqlExpr::Scalar(_) | SqlExpr::Exists(_) | SqlExpr::CountSub(_) => {
            Err(SofError::Uncompilable {
                reason: "subquery-valued expressions are not yet supported by the in-DB runner"
                    .to_string(),
            })
        }
        SqlExpr::Alias { inner, .. } => lower_expr(inner, ctx),
        SqlExpr::Boundary { side, kind, source } => {
            let src = lower_expr(source, ctx)?;
            Ok(lower_boundary(*side, *kind, &src, ctx.dialect))
        }
        SqlExpr::ScalarFromChain {
            chain_sql,
            projection,
            filter,
            offset,
        } => {
            let proj_sql = lower_expr(projection, ctx)?;
            let where_sql = match filter {
                Some(f) => format!(
                    " WHERE {}",
                    ctx.dialect.truthy_predicate(&lower_expr(f, ctx)?)
                ),
                None => String::new(),
            };
            Ok(format!(
                "(SELECT {proj_sql} FROM {chain_sql}{where_sql} LIMIT 1 OFFSET {offset})"
            ))
        }
        SqlExpr::ChainArray {
            chain_sql,
            leaf_alias,
            filter,
            offset: Some(offset),
            ..
        } => {
            // One-element array: the element at `offset` of the (filtered)
            // flattened chain, or NULL (read as `[]` by the unnest guard)
            // when absent.
            let where_sql = match filter {
                Some(f) => format!(
                    " WHERE {}",
                    ctx.dialect.truthy_predicate(&lower_expr(f, ctx)?)
                ),
                None => String::new(),
            };
            let element = if ctx.dialect.lateral_keyword().is_empty() {
                format!(
                    "json_array(CASE {leaf_alias}.type \
                     WHEN 'object' THEN json({leaf_alias}.value) \
                     WHEN 'array' THEN json({leaf_alias}.value) \
                     WHEN 'true' THEN json('true') \
                     WHEN 'false' THEN json('false') \
                     ELSE {leaf_alias}.value END)"
                )
            } else {
                format!("jsonb_build_array({leaf_alias}.value)")
            };
            Ok(format!(
                "(SELECT {element} FROM {chain_sql}{where_sql} LIMIT 1 OFFSET {offset})"
            ))
        }
        SqlExpr::ChainArray {
            chain_sql,
            leaf_alias,
            ..
        } => {
            if ctx.dialect.lateral_keyword().is_empty() {
                // SQLite's `json_each.value` is plain text for containers and
                // 1/0 for booleans; re-tag each element so the array keeps its
                // JSON shape.
                let element = format!(
                    "CASE {leaf_alias}.type \
                     WHEN 'object' THEN json({leaf_alias}.value) \
                     WHEN 'array' THEN json({leaf_alias}.value) \
                     WHEN 'true' THEN json('true') \
                     WHEN 'false' THEN json('false') \
                     ELSE {leaf_alias}.value END"
                );
                Ok(format!(
                    "(SELECT {} FROM {chain_sql})",
                    ctx.dialect.json_agg(&element, &[])
                ))
            } else {
                Ok(format!(
                    "(SELECT {} FROM {chain_sql})",
                    ctx.dialect.json_agg(&format!("{leaf_alias}.value"), &[])
                ))
            }
        }
        SqlExpr::CollectionAgg { root, path } => {
            let mut field_steps: Vec<&str> = Vec::new();
            for step in &path.0 {
                if let PathStep::Field(name) = step {
                    field_steps.push(name.as_str());
                }
            }
            if field_steps.is_empty() {
                return Ok(format!(
                    "(SELECT {} FROM (SELECT {root} AS v) WHERE v IS NOT NULL)",
                    ctx.dialect.json_agg("v", &[])
                ));
            }
            // For 1-segment paths (e.g. `name`), unnest once and aggregate.
            // For 2-segment (e.g. `name.family`), unnest the outer; project
            // the inner field — handles the common scalar-leaf case.
            // For deeper paths or array-of-array shapes (`name.given`), we
            // need a guarded second unnest that handles both array and
            // scalar leaves.
            let lateral = ctx.dialect.lateral_keyword();
            if field_steps.len() == 1 {
                let src = SqlExpr::JsonPath {
                    root: root.clone(),
                    path: super::ir::JsonPath(vec![PathStep::Field(field_steps[0].to_string())]),
                };
                let from = if lateral.is_empty() {
                    format!("{} ca0", emit_sqlite_unnest_source(&src)?)
                } else {
                    format!(
                        "{}{} WITH ORDINALITY AS ca0(value, ordinality)",
                        lateral,
                        ctx.dialect.unnest_array(&emit_pg_unnest_source(&src)?)
                    )
                };
                let agg = ctx
                    .dialect
                    .json_agg("ca0.value", &[occurrence_order("ca0", ctx.dialect)]);
                return Ok(format!("(SELECT {agg} FROM {from})"));
            }
            // Multi-segment: unnest outer, then guard-unnest the leaf so
            // both array leaves (flattened) and scalar leaves (single-element)
            // contribute their values to the aggregate.
            let outer_src = SqlExpr::JsonPath {
                root: root.clone(),
                path: super::ir::JsonPath(vec![PathStep::Field(field_steps[0].to_string())]),
            };
            let leaf_field = field_steps[field_steps.len() - 1];
            let middle_fields = &field_steps[1..field_steps.len() - 1];
            // Compose the path through the middle and to the leaf field
            // so we can read its value off the outer iteration alias.
            let mut leaf_path_segs: Vec<&str> = Vec::new();
            for m in middle_fields {
                leaf_path_segs.push(m);
            }
            leaf_path_segs.push(leaf_field);
            let leaf_value_sql = if lateral.is_empty() {
                format!(
                    "json_extract(ca0.value, {})",
                    sqlite_json_path_literal(&leaf_path_segs)
                )
            } else {
                let segs = leaf_path_segs.to_vec();
                ctx.dialect.json_path("ca0.value", &segs)
            };
            let outer_from = if lateral.is_empty() {
                format!("{} ca0", emit_sqlite_unnest_source(&outer_src)?)
            } else {
                format!(
                    "{}{} WITH ORDINALITY AS ca0(value, ordinality)",
                    lateral,
                    ctx.dialect
                        .unnest_array(&emit_pg_unnest_source(&outer_src)?)
                )
            };
            // Guard-unnest: if the leaf value is an array, iterate; otherwise
            // wrap in a single-element array so json_each / unnest emits one
            // row with the scalar value.
            if lateral.is_empty() {
                // SQLite — `json_each` over a CASE that wraps non-array
                // values in a single-element array. We check array-ness via
                // `json_type(parent, '$.path')` (the path-bearing form),
                // which works on raw values; the bare-value `json_type(x)`
                // form errors on already-extracted scalars.
                let type_check = format!(
                    "json_type(ca0.value, {})",
                    sqlite_json_path_literal(&leaf_path_segs)
                );
                let guarded = format!(
                    "json_each(CASE WHEN {type_check} = 'array' \
                     THEN {leaf_value_sql} \
                     ELSE json_array({leaf_value_sql}) END)"
                );
                let agg = ctx.dialect.json_agg(
                    "ca1.value",
                    &[
                        occurrence_order("ca0", ctx.dialect),
                        occurrence_order("ca1", ctx.dialect),
                    ],
                );
                Ok(format!(
                    "(SELECT {agg} FROM {outer_from}, {guarded} ca1 \
                     WHERE {type_check} IS NOT NULL)"
                ))
            } else {
                // PG — `jsonb_array_elements` requires an array. Wrap with
                // `case when jsonb_typeof = 'array' then ... else jsonb_build_array(...)`.
                let guarded = format!(
                    "jsonb_array_elements(\
                     CASE WHEN jsonb_typeof({leaf_value_sql}) = 'array' \
                     THEN {leaf_value_sql} \
                     ELSE jsonb_build_array({leaf_value_sql}) END)"
                );
                let agg = ctx.dialect.json_agg(
                    "ca1.value",
                    &[
                        occurrence_order("ca0", ctx.dialect),
                        occurrence_order("ca1", ctx.dialect),
                    ],
                );
                Ok(format!(
                    "(SELECT {agg} FROM {outer_from} \
                     JOIN LATERAL {guarded} WITH ORDINALITY AS ca1(value, ordinality) ON TRUE \
                     WHERE {leaf_value_sql} IS NOT NULL)"
                ))
            }
        }
        SqlExpr::JoinAggregate {
            outer_focus,
            outer_alias,
            inner_field,
            inner_alias,
            separator,
        } => {
            // Two nested lateral unnests, then string-aggregate the inner
            // values. The separator is caller-supplied text inlined into the
            // SQL, so it is rendered through the dialect's string literal.
            let sep_lit = lower_string_literal(separator, ctx.dialect)?;
            let unnest_outer = if ctx.dialect.lateral_keyword().is_empty() {
                let src = emit_sqlite_unnest_source(outer_focus)?;
                format!("FROM {src} {outer_alias}")
            } else {
                let src = emit_pg_unnest_source(outer_focus)?;
                format!(
                    "FROM {}{} WITH ORDINALITY AS {outer_alias}(value, ordinality)",
                    ctx.dialect.lateral_keyword(),
                    ctx.dialect.unnest_array(&src)
                )
            };
            // Without an inner field the outer elements are joined directly.
            let (unnest_inner, value_alias, order) = match inner_field {
                Some(field) => {
                    let inner_src = SqlExpr::JsonPath {
                        root: format!("{outer_alias}.value"),
                        path: super::ir::JsonPath(vec![PathStep::Field(field.clone())]),
                    };
                    let unnest_inner = if ctx.dialect.lateral_keyword().is_empty() {
                        let src = emit_sqlite_unnest_source(&inner_src)?;
                        format!(", {src} {inner_alias}")
                    } else {
                        let src = emit_pg_unnest_source(&inner_src)?;
                        format!(
                            " JOIN {}{} WITH ORDINALITY AS {inner_alias}(value, ordinality) ON TRUE",
                            ctx.dialect.lateral_keyword(),
                            ctx.dialect.unnest_array(&src)
                        )
                    };
                    let order = vec![
                        occurrence_order(outer_alias, ctx.dialect),
                        occurrence_order(inner_alias, ctx.dialect),
                    ];
                    (unnest_inner, inner_alias.as_str(), order)
                }
                None => (
                    String::new(),
                    outer_alias.as_str(),
                    vec![occurrence_order(outer_alias, ctx.dialect)],
                ),
            };
            let value_text = if ctx.dialect.lateral_keyword().is_empty() {
                format!("{value_alias}.value")
            } else {
                format!("({value_alias}.value #>> '{{}}')")
            };
            let agg = ctx.dialect.string_agg(&value_text, &sep_lit, &order);
            // Empty input collections yield NULL (empty output), not an empty
            // string, per the FHIRPath spec (SoF v2 PR #349). `string_agg` /
            // `group_concat` over zero rows already returns NULL.
            Ok(format!("(SELECT {agg} {unnest_outer}{unnest_inner})"))
        }
        SqlExpr::WhereScalar {
            focus,
            iter_alias,
            predicate,
            projection,
        } => {
            let unnest = if ctx.dialect.lateral_keyword().is_empty() {
                let src = emit_sqlite_unnest_source(focus)?;
                format!("FROM {src} {iter_alias}")
            } else {
                let src = emit_pg_unnest_source(focus)?;
                format!(
                    "FROM {}{} WITH ORDINALITY AS {iter_alias}(value, ordinality)",
                    ctx.dialect.lateral_keyword(),
                    ctx.dialect.unnest_array(&src)
                )
            };
            let pred_sql = lower_expr(predicate, ctx)?;
            let proj_sql = lower_expr(projection, ctx)?;
            let order = occurrence_order(iter_alias, ctx.dialect);
            Ok(format!(
                "(SELECT {proj_sql} {unnest} WHERE {pred_sql} ORDER BY {order} LIMIT 1)"
            ))
        }
        SqlExpr::WhereExists {
            focus,
            iter_alias,
            predicate,
            negate,
        } => {
            let unnest = emit_where_exists_unnest(focus, iter_alias, ctx)?;
            let pred_sql = lower_expr(predicate, ctx)?;
            let kw = if *negate { "NOT EXISTS" } else { "EXISTS" };
            Ok(format!("{kw} (SELECT 1 {unnest} WHERE {pred_sql})"))
        }
        SqlExpr::ReferenceKey {
            reference,
            expected_type,
        } => {
            let ref_sql = lower_expr(reference, ctx)?;
            let last = ctx.dialect.last_path_segment(&ref_sql);
            match expected_type {
                None => Ok(last),
                Some(ty) => {
                    // `getReferenceKey(Type)` returns NULL when the
                    // reference's type segment doesn't match. The simplest
                    // portable check is two LIKE patterns, covering the
                    // relative form `Type/id` and the absolute form
                    // `http://.../Type/id`.
                    let p1 = lower_string_literal(&format!("{ty}/%"), ctx.dialect)?;
                    let p2 = lower_string_literal(&format!("%/{ty}/%"), ctx.dialect)?;
                    Ok(format!(
                        "CASE WHEN {ref_sql} LIKE {p1} OR {ref_sql} LIKE {p2} \
                         THEN {last} ELSE NULL END"
                    ))
                }
            }
        }
    }
}

/// Lowers one output column to its SELECT-list SQL, returning the decode mode
/// the runners must apply to it.
///
/// An untyped column whose path's FHIR type could not be resolved
/// ([`ColumnDecode::Auto`]) would otherwise reach the runner as bare text,
/// where a string such as `"2.1"` or `"2101"` is indistinguishable from a
/// number. A plain path is therefore projected as the JSON representation of
/// its value instead (`->` / `::text` of the `jsonb`), which keeps strings
/// quoted, and decoded as [`ColumnDecode::Json`].
fn project_column(
    col: &super::ir::Column,
    ctx: &mut ExprCtx<'_>,
) -> Result<(String, ColumnDecode), SofError> {
    if col.collection {
        return Err(SofError::Uncompilable {
            reason: "column.collection=true is not yet supported by the in-DB runner".to_string(),
        });
    }
    if col.decode == ColumnDecode::Auto
        && col.ty == SqlType::Text
        && let SqlExpr::JsonPath { root, path } = &col.expr
        && !path.is_empty()
    {
        let repr = lower_json_path_with(root, path, ctx.dialect, |d, base, segs| {
            d.json_path_repr(base, segs)
        });
        return Ok((repr, ColumnDecode::Json));
    }
    let expr_sql = lower_expr(&col.expr, ctx)?;
    let casted = match col.ty {
        // Default text projection. Path-rooted expressions already produce
        // text via `->>` (PG) / `json_extract` (SQLite); compound
        // expressions (boolean predicates, arithmetic, ...) need an
        // explicit text cast so the runners' `Option<String>` row reader
        // can deserialize them.
        SqlType::Text => project_text(&col.expr, &expr_sql, ctx.dialect),
        other => ctx.dialect.cast(&expr_sql, other),
    };
    Ok((casted, col.decode))
}

/// Wraps a column projection's lowered SQL so the row mapper reads it as
/// text.
///
/// - `JsonPath { path: empty }` references a row-source alias directly. In
///   PG that alias is `jsonb` (`fe.value` etc.); `#>>'{}'` extracts it as
///   text and unwraps scalar JSON strings (`'"foo"'::jsonb #>> '{}'` →
///   `foo`, not `"foo"`). SQLite's loose typing returns the raw value.
/// - `JsonPath` with non-empty path is already text via `->>`/`#>>` (PG)
///   or `json_extract` (SQLite); pass through verbatim.
/// - `Lit` is always text (or NULL); pass through.
/// - Compound expressions go through the dialect's generic text cast.
fn project_text(expr: &SqlExpr, lowered: &str, dialect: &dyn Dialect) -> String {
    match expr {
        SqlExpr::JsonPath { path, .. } if path.is_empty() => {
            if dialect.name() == "postgres" {
                format!("({lowered})#>>'{{}}'")
            } else {
                lowered.to_string()
            }
        }
        SqlExpr::JsonPath { .. } | SqlExpr::Lit(_) => lowered.to_string(),
        _ => dialect.cast(lowered, SqlType::Text),
    }
}

/// Renders caller-supplied text as a SQL string literal for `dialect`.
///
/// This is the single way text is inlined into SQL here: the dialect decides
/// the quoting (see [`Dialect::string_literal`]). NUL cannot be represented in
/// SQL text on either backend, and silently dropping it would change the value
/// being compared, so it is refused instead.
fn lower_string_literal(s: &str, dialect: &dyn Dialect) -> Result<String, SofError> {
    if s.contains('\0') {
        return Err(SofError::Uncompilable {
            reason: "string literals containing a NUL character are not supported by the \
                     in-DB runner"
                .to_string(),
        });
    }
    Ok(dialect.string_literal(s))
}

/// True when `s` is a plain decimal numeral: optional sign, digits with an
/// optional fraction, optional exponent. Decimal literals are inlined verbatim,
/// so this is what keeps arbitrary text out of the SQL through them.
fn is_decimal_numeral(s: &str) -> bool {
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let s = s.strip_prefix(['+', '-']).unwrap_or(s);
    let (mantissa, exponent) = match s.split_once(['e', 'E']) {
        Some((m, e)) => (m, Some(e.strip_prefix(['+', '-']).unwrap_or(e))),
        None => (s, None),
    };
    let (int, frac) = match mantissa.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (mantissa, None),
    };
    digits(int) && frac.is_none_or(digits) && exponent.is_none_or(digits)
}

fn lower_lit(v: &LitValue, dialect: &dyn Dialect) -> Result<String, SofError> {
    Ok(match v {
        LitValue::Null => "NULL".to_string(),
        LitValue::Bool(true) => dialect.bool_true().to_string(),
        LitValue::Bool(false) => dialect.bool_false().to_string(),
        LitValue::Int(n) => n.to_string(),
        LitValue::Decimal(s) if is_decimal_numeral(s) => s.clone(),
        LitValue::Decimal(s) => {
            return Err(SofError::Uncompilable {
                reason: format!("decimal literal {s:?} is not a plain number"),
            });
        }
        // FHIRPath string literals (`where`, `extension(url)`, `iif`, ...) are
        // inlined, not bound, so they go through the dialect's string literal.
        // ViewDefinition constants never reach this arm: they are bound as
        // `SqlExpr::Param`.
        LitValue::Str(s) => lower_string_literal(s, dialect)?,
    })
}

fn lower_json_path(root: &str, path: &JsonPath, dialect: &dyn Dialect) -> String {
    lower_json_path_with(root, path, dialect, |d, base, segs| {
        d.json_path_text(base, segs)
    })
}

/// [`lower_json_path`] with the per-path accessor supplied by the caller —
/// the scalar text form (`->>` / `json_extract`) or the JSON representation
/// form ([`Dialect::json_path_repr`]).
fn lower_json_path_with(
    root: &str,
    path: &JsonPath,
    dialect: &dyn Dialect,
    access: impl Fn(&dyn Dialect, &str, &[&str]) -> String,
) -> String {
    if path.is_empty() {
        return root.to_string();
    }
    // Resolve the path to plain field/index segments (OfType / TypeFilter
    // were already collapsed during AST lowering).
    let raw_segments: Vec<String> = path
        .0
        .iter()
        .filter_map(|step| match step {
            PathStep::Field(name) => Some(name.clone()),
            PathStep::Index(n) => Some(n.to_string()),
            PathStep::OfType(_) | PathStep::TypeFilter(_) => None,
        })
        .collect();
    if raw_segments.is_empty() {
        return root.to_string();
    }

    // FHIRPath flattens collections automatically — `name.family` over a
    // resource where `name` is an array yields a collection of family
    // strings. Column extractions (which want a single value when
    // `collection: false`) need to pick the first element. Without FHIR
    // schema, the simplest portable approach is `coalesce(<array-first>,
    // <plain>)`: the array-first form returns the value when the
    // intermediate is an array; plain handles scalar intermediates.
    //
    // Capped at two-segment paths — deeper paths skip the fallback rather
    // than emit 2^N combinations. Index segments preserve their literal
    // position.
    let field_count = path
        .0
        .iter()
        .filter(|s| matches!(s, PathStep::Field(_)))
        .count();
    let trailing_zero_from_first =
        matches!(path.0.last(), Some(PathStep::Index(0))) && field_count >= 2;
    let other_indices = path
        .0
        .iter()
        .enumerate()
        .any(|(i, s)| matches!(s, PathStep::Index(_)) && i + 1 != path.0.len());

    let segs: Vec<&str> = raw_segments.iter().map(String::as_str).collect();

    // Path with a trailing `Index(0)` from `.first()` on a multi-Field path:
    // lift the index to the array boundary so `name.family.first()` becomes
    // `name[0].family` (the family of the first name) rather than the
    // (invalid) first character of a string.
    if trailing_zero_from_first && !other_indices {
        let mut interleaved: Vec<String> = Vec::new();
        let mut first_field_seen = false;
        for step in &path.0[..path.0.len() - 1] {
            match step {
                PathStep::Field(n) => {
                    interleaved.push(n.clone());
                    if !first_field_seen {
                        interleaved.push("0".to_string());
                        first_field_seen = true;
                    }
                }
                PathStep::Index(n) => interleaved.push(n.to_string()),
                _ => {}
            }
        }
        let lifted: Vec<&str> = interleaved.iter().map(String::as_str).collect();
        return access(dialect, root, &lifted);
    }

    let already_indexed =
        other_indices || matches!(path.0.last(), Some(PathStep::Index(_))) && field_count < 2;

    // Multi-Field paths (no explicit Index) get an `array-first → plain`
    // coalesce — `[0]` is inserted after the first Field so that paths like
    // `name.family` or `link.other.reference` traverse arrays of objects.
    if field_count >= 2 && !already_indexed {
        let array_segs: Vec<String> = path
            .0
            .iter()
            .enumerate()
            .flat_map(|(i, step)| match step {
                PathStep::Field(name) if i == 0 => vec![name.clone(), "0".to_string()],
                PathStep::Field(name) => vec![name.clone()],
                PathStep::Index(n) => vec![n.to_string()],
                _ => Vec::new(),
            })
            .collect();
        let array_refs: Vec<&str> = array_segs.iter().map(String::as_str).collect();
        return format!(
            "coalesce({}, {})",
            access(dialect, root, &array_refs),
            access(dialect, root, &segs)
        );
    }

    access(dialect, root, &segs)
}

fn lower_binop(op: BinOp) -> &'static str {
    match op {
        BinOp::Eq => "=",
        BinOp::Neq => "!=",
        BinOp::Lt => "<",
        BinOp::Lte => "<=",
        BinOp::Gt => ">",
        BinOp::Gte => ">=",
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::And => "AND",
        BinOp::Or => "OR",
        BinOp::Concat => "||",
        BinOp::Like => "LIKE",
        BinOp::RegexMatch => "~",
    }
}

/// Dialect-aware binary-operator lowering.
///
/// SQLite is loose-typed and accepts `text op number` natively, so we just
/// emit the operands verbatim. PostgreSQL is strict-typed and `->>`/`#>>`
/// projections return `text`; comparing or arithmetic-combining text with
/// numeric or boolean literals raises `operator does not exist` at runtime,
/// so we cast based on the literal side's type:
///
/// - `Eq`/`Neq` against `Bool(b)` → emit `'true'`/`'false'` text literal so
///   the JSON-text projection compares string-to-string.
/// - `Eq`/`Neq` against `Int`/`Decimal` → cast the non-literal side to
///   `numeric`.
/// - Numeric ops (`Lt`/`Lte`/`Gt`/`Gte`/`Add`/`Sub`/`Mul`/`Div`) → cast
///   non-literal sides to `numeric`. Numeric literals stay bare.
/// - `And`/`Or` → cast each side to `boolean` so JSON-text-projected boolean
///   paths participate in three-valued logic.
fn lower_binop_dialect(
    op: BinOp,
    lhs: &SqlExpr,
    rhs: &SqlExpr,
    ctx: &mut ExprCtx<'_>,
) -> Result<String, SofError> {
    if ctx.dialect.name() != "postgres" {
        let l = lower_expr(lhs, ctx)?;
        let r = lower_expr(rhs, ctx)?;
        return Ok(format!("({l} {} {r})", lower_binop(op)));
    }

    let op_sql = lower_binop(op);

    match op {
        BinOp::Eq | BinOp::Neq => {
            // Boolean literal on either side → compare as text against
            // `'true'`/`'false'` so the JSON `->>` projection lines up.
            if let Some(b) = bool_literal(rhs) {
                let l = lower_expr(lhs, ctx)?;
                let lit = if b { "'true'" } else { "'false'" };
                return Ok(format!("({l} {op_sql} {lit})"));
            }
            if let Some(b) = bool_literal(lhs) {
                let r = lower_expr(rhs, ctx)?;
                let lit = if b { "'true'" } else { "'false'" };
                return Ok(format!("({lit} {op_sql} {r})"));
            }

            // Numeric literal on either side → cast the other side to numeric.
            if is_numeric_literal(rhs) {
                let l = lower_expr(lhs, ctx)?;
                let r = lower_expr(rhs, ctx)?;
                return Ok(format!("({} {op_sql} {r})", cast_pg_numeric(lhs, &l)));
            }
            if is_numeric_literal(lhs) {
                let l = lower_expr(lhs, ctx)?;
                let r = lower_expr(rhs, ctx)?;
                return Ok(format!("({l} {op_sql} {})", cast_pg_numeric(rhs, &r)));
            }

            // Default: text-vs-text comparison (covers JsonPath = JsonPath
            // and JsonPath = string literal/param).
            let l = lower_expr(lhs, ctx)?;
            let r = lower_expr(rhs, ctx)?;
            Ok(format!("({l} {op_sql} {r})"))
        }
        BinOp::Lt | BinOp::Lte | BinOp::Gt | BinOp::Gte => {
            let l = lower_expr(lhs, ctx)?;
            let r = lower_expr(rhs, ctx)?;
            Ok(format!(
                "({} {op_sql} {})",
                cast_pg_numeric(lhs, &l),
                cast_pg_numeric(rhs, &r)
            ))
        }
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
            let l = lower_expr(lhs, ctx)?;
            let r = lower_expr(rhs, ctx)?;
            Ok(format!(
                "({} {op_sql} {})",
                cast_pg_numeric(lhs, &l),
                cast_pg_numeric(rhs, &r)
            ))
        }
        BinOp::And | BinOp::Or => {
            // FHIRPath text-projected booleans need an explicit `::boolean`
            // cast for `AND`/`OR` to type-check in PG. Bare boolean
            // sub-expressions (`x IS NOT NULL`, comparisons) cast cheaply.
            let l = lower_expr(lhs, ctx)?;
            let r = lower_expr(rhs, ctx)?;
            Ok(format!("(({l})::boolean {op_sql} ({r})::boolean)"))
        }
        BinOp::Concat | BinOp::Like | BinOp::RegexMatch => {
            let l = lower_expr(lhs, ctx)?;
            let r = lower_expr(rhs, ctx)?;
            Ok(format!("({l} {op_sql} {r})"))
        }
    }
}

fn bool_literal(e: &SqlExpr) -> Option<bool> {
    match e {
        SqlExpr::Lit(LitValue::Bool(b)) => Some(*b),
        _ => None,
    }
}

fn is_numeric_literal(e: &SqlExpr) -> bool {
    matches!(
        e,
        SqlExpr::Lit(LitValue::Int(_)) | SqlExpr::Lit(LitValue::Decimal(_))
    )
}

/// Wraps a PG sub-expression with a `::numeric` cast.
///
/// `SqlExpr::Param(_)` and `SqlExpr::Lit(Str)` get a redundant `::text` cast
/// first. Reason: PG resolves `$N::numeric` eagerly when planning the
/// statement and pins the parameter type to `numeric`. `tokio_postgres` then
/// reports `numeric` to the binder, which fails because constants are bound
/// as text strings (see `PgParam::Bool`/`Int`/`Decimal`). The intermediate
/// `::text` keeps the parameter inferred as text; the runtime `text →
/// numeric` cast still works for any numeric-string input.
///
/// Numeric literals (`Int`/`Decimal`) skip the cast — they're already typed.
fn cast_pg_numeric(expr: &SqlExpr, lowered: &str) -> String {
    if is_numeric_literal(expr) {
        return lowered.to_string();
    }
    if matches!(expr, SqlExpr::Param(_) | SqlExpr::Lit(LitValue::Str(_))) {
        return format!("({lowered}::text)::numeric");
    }
    format!("({lowered})::numeric")
}

// ============================================================================
// Helpers
// ============================================================================

/// Emits the `FROM ...` clause of a `WhereExists` subquery.
///
/// A `JsonPath` focus with more than one `Field` step is unnested once per
/// step (each step rooted on the previous alias's `.value`), so an array
/// anywhere along the path (`component.code.coding`) is flattened. The last
/// step keeps `iter_alias`, which the pre-lowered criterion is rooted on; the
/// intermediate aliases are derived from it (`w0_0`, `w0_1`, ...) so nested
/// `exists` subqueries cannot collide. Single-step and computed foci keep the
/// one-unnest form.
fn emit_where_exists_unnest(
    focus: &SqlExpr,
    iter_alias: &str,
    ctx: &ExprCtx,
) -> Result<String, SofError> {
    let sqlite = ctx.dialect.lateral_keyword().is_empty();
    let one = |source: &SqlExpr, alias: &str| -> Result<String, SofError> {
        if sqlite {
            Ok(format!("{} {alias}", emit_sqlite_unnest_source(source)?))
        } else {
            let src = emit_pg_unnest_source(source)?;
            Ok(format!(
                "{}{} AS {alias}(value)",
                ctx.dialect.lateral_keyword(),
                ctx.dialect.unnest_array(&src)
            ))
        }
    };
    if let SqlExpr::JsonPath { root, path } = focus {
        let segments = super::compile_view::split_path_into_segments(path);
        let fields = path
            .0
            .iter()
            .filter(|s| matches!(s, PathStep::Field(_)))
            .count();
        if fields > 1 && segments.len() == fields {
            let last = segments.len() - 1;
            let mut parts = Vec::with_capacity(segments.len());
            let mut prev_root = root.clone();
            for (i, seg) in segments.into_iter().enumerate() {
                let alias = if i == last {
                    iter_alias.to_string()
                } else {
                    format!("{iter_alias}_{i}")
                };
                let source = SqlExpr::JsonPath {
                    root: prev_root.clone(),
                    path: seg,
                };
                parts.push(one(&source, &alias)?);
                prev_root = format!("{alias}.value");
            }
            return Ok(format!("FROM {}", parts.join(", ")));
        }
    }
    Ok(format!("FROM {}", one(focus, iter_alias)?))
}

/// Emits the SQLite `json_each(...)` source clause for a lateral unnest.
///
/// Paths wrap with a type guard: arrays iterate; non-array singletons (FHIR
/// singleton elements like `Condition.code` or `contact.name`) wrap in a
/// single-element array; missing intermediates produce zero rows.
/// Computed sources propagate lowering errors instead of becoming empty.
fn emit_sqlite_unnest_source(source: &SqlExpr) -> Result<String, SofError> {
    if let SqlExpr::JsonPath { root, path } = source {
        let segments_owned: Vec<String> = path
            .0
            .iter()
            .filter_map(|s| match s {
                PathStep::Field(n) => Some(n.clone()),
                PathStep::Index(n) => Some(n.to_string()),
                _ => None,
            })
            .collect();
        let segments: Vec<&str> = segments_owned.iter().map(String::as_str).collect();
        let path_step_count = path
            .0
            .iter()
            .filter(|s| matches!(s, PathStep::Field(_) | PathStep::Index(_)))
            .count();
        if segments.len() == path_step_count && !segments.is_empty() {
            // Build SQLite JSON path syntax — numeric segments are array
            // indices `[N]`, others are dotted fields. The result is a
            // complete, escaped SQL string literal (quotes included).
            let path_lit = sqlite_json_path_literal(&segments);
            // Always type-guarded, also off `r.data`: the two-arg
            // `json_each(r.data, '$.code')` iterates the MEMBERS of a
            // singleton object (`Condition.code`), not the object itself.
            let extracted = format!("json_extract({root}, {path_lit})");
            let type_check = format!("json_type({root}, {path_lit})");
            // For non-array values, wrap with `json_array(json(<extract>))`
            // — `json(...)` re-parses the extracted text so the wrapped
            // value preserves its JSON shape (otherwise SQLite's `json_array`
            // sees a TEXT argument and JSON-quotes it as a string, which
            // would iterate as one stringified row rather than the original
            // object).
            return Ok(format!(
                "json_each(CASE WHEN {type_check} = 'array' THEN {extracted} \
                 WHEN {type_check} IN ('object', 'array') THEN json_array(json({extracted})) \
                 WHEN {type_check} IS NOT NULL THEN json_array({extracted}) \
                 ELSE '[]' END)"
            ));
        }
    }
    let mut ctx = ExprCtx::new(&super::dialect::SqliteDialect, 3);
    let computed = lower_expr(source, &mut ctx)?;
    Ok(format!("json_each(coalesce({computed}, '[]'))"))
}

/// Emits the PostgreSQL JSON-valued navigation expression that becomes the
/// argument of `jsonb_array_elements(...)`. Forces the `->` (JSON) operator
/// rather than the `->>` (text) operator that scalar-projection paths use.
///
/// Wraps the result in a `jsonb_typeof`-based guard that mirrors the SQLite
/// branch in `emit_sqlite_unnest_source`: arrays pass through; non-array
/// non-null values get wrapped in a single-element array (handles FHIR
/// singleton elements like `Patient.contact.name` that are object-shaped);
/// null / missing intermediates produce zero rows instead of raising at
/// runtime.
/// Computed sources propagate lowering errors instead of becoming empty.
fn emit_pg_unnest_source(source: &SqlExpr) -> Result<String, SofError> {
    let raw = if let SqlExpr::JsonPath { root, path } = source {
        let segments: Vec<String> = path
            .0
            .iter()
            .filter_map(|s| match s {
                PathStep::Field(n) => Some(n.clone()),
                PathStep::Index(n) => Some(n.to_string()),
                _ => None,
            })
            .collect();
        if segments.is_empty() {
            root.clone()
        } else if segments.len() == 1 {
            format!("{root}->{}", pg_key_literal(&segments[0]))
        } else {
            let segs: Vec<&str> = segments.iter().map(String::as_str).collect();
            format!("{root}#>{}", pg_path_array_literal(&segs))
        }
    } else {
        // Non-`JsonPath` sources include nested `WhereScalar`/`ScalarFromChain`
        // results (e.g. `extension(url1).extension(url2)`). Their projections
        // are lowered as text via `->>`/`#>>`; an explicit `::jsonb` cast
        // re-parses the JSON text so the surrounding `jsonb_typeof` /
        // `jsonb_array_elements` operators type-check.
        let mut ctx = ExprCtx::new(&super::dialect::PgDialect, 3);
        let inner = lower_expr(source, &mut ctx)?;
        format!("({inner})::jsonb")
    };
    Ok(format!(
        "(CASE WHEN jsonb_typeof({raw}) = 'array' THEN {raw} \
         WHEN jsonb_typeof({raw}) IS NOT NULL THEN jsonb_build_array({raw}) \
         ELSE '[]'::jsonb END)"
    ))
}

/// Lowers a [`SqlExpr::Boundary`] to a CASE expression. Decimal expands the
/// last digit by ±0.5; date/dateTime/time pad with the first/last instant of
/// the largest unspecified unit. The expressions match the SoF v2
/// `fn_boundary` conformance fixture's expected outputs and return NULL for
/// any input the function isn't defined for (e.g. `lowBoundary()` on a
/// dateTime column when the source is actually a Quantity).
///
/// String-form-driven so it works on both dialects, with `instr` /
/// `GLOB`-style operations switched per dialect (PG has neither builtin).
fn lower_boundary(
    side: BoundarySide,
    kind: BoundaryKind,
    src: &str,
    dialect: &dyn Dialect,
) -> String {
    let is_sqlite = dialect.lateral_keyword().is_empty();
    // SQLite uses `instr(haystack, needle)` (1-based, 0 when not found);
    // PG uses `position(needle in haystack)` with the same 1-based / 0
    // semantics. Both return integer; the surrounding CASE handles 0.
    let dot_pos = if is_sqlite {
        format!("instr({src}, '.')")
    } else {
        format!("position('.' in {src})")
    };
    // Detect "non-numeric" input. SQLite has `GLOB '*[A-Za-z]*'`; PG uses
    // POSIX regex `~`. Both return boolean.
    let alpha_check = if is_sqlite {
        format!("({src}) || '' GLOB '*[A-Za-z]*'")
    } else {
        format!("({src})::text ~ '[A-Za-z]'")
    };
    match kind {
        BoundaryKind::Decimal => {
            // The text projection is JSON: numbers like `1.0` or `1`.
            // Treat NULL/non-numeric input as NULL.
            //
            //   precision = digits after `.` in the source string
            //   delta     = 0.5 * 10^-precision
            //   low / high = value ∓ delta
            let len_after_dot = format!(
                "(length({src}) - CASE WHEN {dot_pos} = 0 \
                                       THEN length({src}) \
                                       ELSE {dot_pos} END)"
            );
            // delta = 0.5 / 10^precision = 5 * 10^(-precision-1)
            // Compute as `0.5 / power10(precision)` with `power(10, n)` (PG)
            // or `1.0 * exp(...)` (SQLite has no `power` by default — use
            // `(1.0 * substr('1.0', ...))` trick? Cleaner: emit a CASE on
            // the small set of precisions actually exercised. The corpus
            // uses precision 1 only, so dispatch on `len_after_dot`).
            let half_step = format!(
                "CASE {len_after_dot} \
                   WHEN 0 THEN 0.5 \
                   WHEN 1 THEN 0.05 \
                   WHEN 2 THEN 0.005 \
                   WHEN 3 THEN 0.0005 \
                   WHEN 4 THEN 0.00005 \
                   WHEN 5 THEN 0.000005 \
                   WHEN 6 THEN 0.0000005 \
                   ELSE 0.00000005 END"
            );
            let op = match side {
                BoundarySide::Low => "-",
                BoundarySide::High => "+",
            };
            // PG strict-typed: text projection must be cast to numeric for
            // arithmetic. SQLite happily coerces.
            let numeric_src = if is_sqlite {
                format!("({src})")
            } else {
                format!("({src})::numeric")
            };
            // Wrap in CASE so non-numeric inputs (e.g. a date string) yield
            // NULL rather than an error.
            format!(
                "CASE WHEN {src} IS NULL THEN NULL \
                 WHEN {alpha_check} THEN NULL \
                 ELSE {numeric_src} {op} {half_step} END"
            )
        }
        BoundaryKind::Date => {
            // Pad year/month-only dates to first/last day of that period.
            let pad_month_only = match side {
                BoundarySide::Low => "'-01-01'",
                BoundarySide::High => "'-12-31'",
            };
            let day_pad = match side {
                BoundarySide::Low => "'-01'".to_string(),
                BoundarySide::High => format!(
                    "'-' || CASE substr({src}, 6, 2) \
                       WHEN '02' THEN '28' \
                       WHEN '04' THEN '30' \
                       WHEN '06' THEN '30' \
                       WHEN '09' THEN '30' \
                       WHEN '11' THEN '30' \
                       ELSE '31' END"
                ),
            };
            format!(
                "CASE \
                   WHEN {src} IS NULL THEN NULL \
                   WHEN length({src}) = 10 THEN {src} \
                   WHEN length({src}) = 7 THEN {src} || {day_pad} \
                   WHEN length({src}) = 4 THEN {src} || {pad_month_only} \
                   ELSE NULL END"
            )
        }
        BoundaryKind::DateTime => {
            // SoF v2 PR FHIR/sql-on-fhir-v2#357: a column whose type is
            // `dateTime` may carry results from either a `date` or a
            // `dateTime` source. FHIRPath `lowBoundary()`/`highBoundary()`
            // preserves the source's precision, so the SQL emit dispatches
            // on input length:
            //
            //   length 4  ("YYYY")        → date semantics: pad to "YYYY-01-01"
            //                                                 or "YYYY-12-31"
            //   length 7  ("YYYY-MM")     → date semantics: pad to month start
            //                                                 or last day of month
            //   length 10 ("YYYY-MM-DD")  → datetime semantics: append
            //                                "T00:00:00.000+14:00" (low)
            //                                or "T23:59:59.999-12:00" (high)
            //
            // Anything else (full datetime already present, malformed) returns
            // NULL — matches the BoundaryKind::Date emit's behavior for
            // off-spec inputs.
            let pad_full_day = match side {
                BoundarySide::Low => "'T00:00:00.000+14:00'",
                BoundarySide::High => "'T23:59:59.999-12:00'",
            };
            let pad_month_only = match side {
                BoundarySide::Low => "'-01-01'",
                BoundarySide::High => "'-12-31'",
            };
            let day_pad = match side {
                BoundarySide::Low => "'-01'".to_string(),
                BoundarySide::High => format!(
                    "'-' || CASE substr({src}, 6, 2) \
                       WHEN '02' THEN '28' \
                       WHEN '04' THEN '30' \
                       WHEN '06' THEN '30' \
                       WHEN '09' THEN '30' \
                       WHEN '11' THEN '30' \
                       ELSE '31' END"
                ),
            };
            format!(
                "CASE \
                   WHEN {src} IS NULL THEN NULL \
                   WHEN length({src}) = 10 THEN {src} || {pad_full_day} \
                   WHEN length({src}) = 7 THEN {src} || {day_pad} \
                   WHEN length({src}) = 4 THEN {src} || {pad_month_only} \
                   ELSE NULL END"
            )
        }
        BoundaryKind::Time => {
            // FHIRPath `lowBoundary()`/`highBoundary()` preserves the source's
            // precision, so the emit dispatches on input length:
            //
            //   length 5  ("HH:MM")        → fill seconds + millis
            //   length 8  ("HH:MM:SS")     → fill millis only (seconds are
            //                                 already specified, so high does
            //                                 NOT roll them to :59)
            //   length 12 ("HH:MM:SS.fff") → already full precision, unchanged
            //
            // Anything else (malformed) returns NULL, matching the Date/DateTime
            // emit's behavior for off-spec inputs.
            let minute_pad = match side {
                BoundarySide::Low => "':00.000'",
                BoundarySide::High => "':59.999'",
            };
            let second_pad = match side {
                BoundarySide::Low => "'.000'",
                BoundarySide::High => "'.999'",
            };
            format!(
                "CASE \
                   WHEN {src} IS NULL THEN NULL \
                   WHEN length({src}) = 5 THEN {src} || {minute_pad} \
                   WHEN length({src}) = 8 THEN {src} || {second_pad} \
                   WHEN length({src}) = 12 THEN {src} \
                   ELSE NULL END"
            )
        }
    }
}

/// Strips a trailing `ORDER BY …` line (case-insensitive). Used when
/// assembling UNION ALL branches — `ORDER BY` inside an individual compound
/// SELECT term is not portable. Only a final line counts: recursive branches
/// end without one and carry `ORDER BY` inside window functions and
/// subqueries, which must stay.
fn strip_trailing_order_by(sql: &str) -> &str {
    let upper = sql.to_ascii_uppercase();
    match upper.rfind("\nORDER BY") {
        Some(pos) if !sql[pos + 1..].contains('\n') => &sql[..pos],
        _ => sql,
    }
}

/// Rejects identifiers that would break SQL `AS "…"` quoting. Per the SoF v2
/// spec column names are restricted to identifier characters, so this is a
/// safety net for malformed input rather than a deliberate escape.
fn sanitize_ident(name: &str) -> Result<&str, SofError> {
    if name.contains('"') || name.contains('\0') {
        return Err(SofError::InvalidViewDefinition(format!(
            "column name '{name}' contains an unsupported character"
        )));
    }
    Ok(name)
}

// Unused JsonType import-warning silencer: variants are referenced inside
// PathStep::TypeFilter pattern matches that get exercised in later stages.
const _: Option<JsonType> = None;

#[cfg(test)]
mod tests {
    //! Backstop tests: the compiler rejects non-plain member names, but the
    //! emitter must also keep any name or string value inside the literal it
    //! is embedded in, because IR can be built by anything (`PathStep::Field`,
    //! `LitValue::Str`, ...). String literals are checked under both settings
    //! of PostgreSQL's `standard_conforming_strings`.

    use super::super::dialect::test_support::{STRING_LITERAL_CASES, scan_literals};
    use super::super::dialect::{PgDialect, SqliteDialect};
    use super::*;

    /// Every dialect the emitter targets, as `(label, dialect, backslash
    /// escapes in ordinary literals)`. The last flag models PostgreSQL with
    /// `standard_conforming_strings = off`; SQLite and PostgreSQL's default
    /// read a backslash in an ordinary `'...'` literal as itself.
    fn targets() -> [(&'static str, &'static dyn Dialect, bool); 3] {
        [
            ("sqlite", &SqliteDialect, false),
            ("postgres", &PgDialect, false),
            (
                "postgres, standard_conforming_strings = off",
                &PgDialect,
                true,
            ),
        ]
    }

    /// Names that would break out of, or corrupt, a naively spliced literal:
    /// quotes, backslashes, JSON-path and array-literal metacharacters, and
    /// an injection payload.
    const HOSTILE: &[&str] = &[
        "a'b",
        "'",
        "x') OR 1=1 --",
        "x\\') OR 1=1 --",
        "a\"b",
        "a\\b",
        "a.b",
        "a b",
        "a,b",
        "a}b",
        "{a}",
        "a[0]",
        "",
        "null",
    ];

    /// String values that need care inside a SQL literal, kept ordinary on
    /// purpose: a quote, a backslash, both, and runs of each.
    const STRING_VALUES: &[&str] = &[
        "a'b",
        "a\\b",
        "it's",
        "a\\'b",
        "it's a\\b",
        "'",
        "\\",
        "\\\\",
        "''",
        "\\''",
        "",
    ];

    /// Stands in for a plain string value; every literal that carries it is
    /// compared against the same shape built with a hostile value.
    const PLACEHOLDER: &str = "xyzzy";

    fn field(name: &str) -> PathStep {
        PathStep::Field(name.to_string())
    }

    fn json_path(root: &str, steps: Vec<PathStep>) -> SqlExpr {
        SqlExpr::JsonPath {
            root: root.to_string(),
            path: JsonPath(steps),
        }
    }

    fn scan() -> PlanNode {
        PlanNode::Scan {
            alias: "r".to_string(),
            resource_type: "Patient".to_string(),
            filter: Default::default(),
        }
    }

    fn project(parent: PlanNode, expr: SqlExpr) -> PlanNode {
        PlanNode::Project {
            parent: Box::new(parent),
            columns: vec![super::super::ir::Column {
                name: "c".to_string(),
                expr,
                collection: false,
                ty: SqlType::Text,
                decode: super::super::decode::ColumnDecode::Text,
            }],
        }
    }

    fn unnest(source: SqlExpr) -> PlanNode {
        PlanNode::LateralUnnest {
            parent: Box::new(scan()),
            source,
            out_alias: "fe".to_string(),
            left_join: false,
            on_filter: None,
            flat_index: None,
        }
    }

    fn filtered(predicate: SqlExpr) -> PlanNode {
        PlanNode::Filter {
            parent: Box::new(scan()),
            predicate,
        }
    }

    #[test]
    fn repeat_cells_rejoin_only_when_their_focus_needs_the_resource() {
        for root in ["r.data", "rec_0.node"] {
            let focus = json_path(root, vec![field("item")]);
            let cells = [
                SqlExpr::CollectionAgg {
                    root: root.into(),
                    path: JsonPath(vec![field("item"), field("linkId")]),
                },
                SqlExpr::JoinAggregate {
                    outer_focus: Box::new(focus.clone()),
                    outer_alias: "ja0".into(),
                    inner_field: Some("linkId".into()),
                    inner_alias: "ja1".into(),
                    separator: ",".into(),
                },
                SqlExpr::WhereScalar {
                    focus: Box::new(focus),
                    iter_alias: "w0".into(),
                    predicate: Box::new(SqlExpr::Lit(LitValue::Bool(true))),
                    projection: Box::new(json_path("w0.value", vec![field("linkId")])),
                },
            ];
            for cell in cells {
                let plan = project(
                    PlanNode::Recurse {
                        parent: Box::new(scan()),
                        seed_root: "r.data".to_string(),
                        step_paths: vec![JsonPath(vec![field("item")])],
                        out_alias: "rec_0".into(),
                    },
                    cell,
                );
                for (label, dialect, _) in targets() {
                    let sql = emit_plan(&plan, dialect).unwrap().sql;
                    assert_eq!(
                        sql.contains("JOIN resources r ON"),
                        root == "r.data",
                        "{label}, focus {root}: {sql}"
                    );
                }
            }
        }
    }

    fn rec(parent: PlanNode, seed_root: &str, paths: &[&[&str]], alias: &str) -> PlanNode {
        PlanNode::Recurse {
            parent: Box::new(parent),
            seed_root: seed_root.to_string(),
            step_paths: paths
                .iter()
                .map(|p| JsonPath(p.iter().map(|n| field(n)).collect()))
                .collect(),
            out_alias: alias.to_string(),
        }
    }

    fn link_of(root: &str) -> SqlExpr {
        json_path(root, vec![field("linkId")])
    }

    #[test]
    fn single_repeat_on_the_resource_keeps_the_unkeyed_cte() {
        let plan = project(
            rec(scan(), "r.data", &[&["item"]], "rec_0"),
            SqlExpr::RowIndex(RowIndexScope::Repeat("rec_0".into())),
        );
        for (label, dialect, _) in targets() {
            let sql = emit_plan(&plan, dialect).unwrap().sql;
            assert!(!sql.contains("pkey"), "{label}: {sql}");
            assert!(
                sql.contains("PARTITION BY rec_0.rid ORDER BY"),
                "{label}: {sql}"
            );
        }
    }

    #[test]
    fn repeat_seeded_from_a_foreach_is_keyed_by_the_iteration_row() {
        let each = unnest(json_path("r.data", vec![field("item")]));
        let plan = project(
            rec(each, "fe.value", &[&["item"]], "rec_1"),
            link_of("rec_1.node"),
        );
        for (label, dialect, _) in targets() {
            let sql = emit_plan(&plan, dialect).unwrap().sql;
            assert_eq!(sql.matches("WITH RECURSIVE").count(), 1, "{label}: {sql}");
            assert!(
                sql.contains("rec_1(rid, node, ord_path, pkey)"),
                "{label}: {sql}"
            );
            // The seed replays the enclosing forEach before descending.
            let seed = sql.split("UNION ALL").next().unwrap();
            assert!(
                seed.contains("fe.ordinality") || seed.contains("fe.rowid"),
                "{label}: {sql}"
            );
            // The main query joins the CTE back on resource and scope key.
            assert!(
                sql.contains("JOIN rec_1 ON rec_1.rid = r.id AND rec_1.pkey = "),
                "{label}: {sql}"
            );
            assert!(sql.contains("rec_1.node"), "{label}: {sql}");
        }
    }

    #[test]
    fn repeat_nested_in_a_repeat_reads_the_parent_cte() {
        let plan = project(
            rec(
                rec(scan(), "r.data", &[&["item"]], "rec_0"),
                "rec_0.node",
                &[&["item"]],
                "rec_1",
            ),
            link_of("rec_1.node"),
        );
        for (label, dialect, _) in targets() {
            let sql = emit_plan(&plan, dialect).unwrap().sql;
            assert_eq!(sql.matches("WITH RECURSIVE").count(), 1, "{label}: {sql}");
            let rec_1 = sql.split("rec_1(rid").nth(1).unwrap();
            assert!(
                rec_1.contains("JOIN rec_0 ON rec_0.rid = r.id AND rec_0.pkey = "),
                "{label}: {sql}"
            );
            assert!(
                sql.contains("rec_0.pkey || '/' || rec_0.ord_path"),
                "{label}: {sql}"
            );
            // The parent CTE keeps the resource-level (empty) scope key.
            assert!(
                sql.contains("rec_0(rid, node, ord_path, pkey)"),
                "{label}: {sql}"
            );
        }
    }

    #[test]
    fn sibling_repeats_share_a_scope_and_both_join_the_main_query() {
        let plan = project(
            rec(
                rec(scan(), "r.data", &[&["item"]], "rec_0"),
                "r.data",
                &[&["item"]],
                "rec_1",
            ),
            link_of("rec_1.node"),
        );
        for (label, dialect, _) in targets() {
            let sql = emit_plan(&plan, dialect).unwrap().sql;
            assert!(
                sql.contains("JOIN rec_0 ON rec_0.rid = r.id AND rec_0.pkey = "),
                "{label}: {sql}"
            );
            assert!(
                sql.contains("JOIN rec_1 ON rec_1.rid = r.id AND rec_1.pkey = "),
                "{label}: {sql}"
            );
            // A sibling seed never replays the other repeat.
            let rec_1 = sql.split("rec_1(rid").nth(1).unwrap();
            let seed = rec_1.split("UNION ALL").next().unwrap();
            assert!(!seed.contains("JOIN rec_0"), "{label}: {sql}");
        }
    }

    #[test]
    fn keyed_repeat_row_index_restarts_per_seed_scope() {
        let each = unnest(json_path("r.data", vec![field("item")]));
        let plan = project(
            rec(each, "fe.value", &[&["item"]], "rec_1"),
            SqlExpr::RowIndex(RowIndexScope::Repeat("rec_1".into())),
        );
        for (label, dialect, _) in targets() {
            let sql = emit_plan(&plan, dialect).unwrap().sql;
            assert!(
                sql.contains("PARTITION BY rec_1.rid, rec_1.pkey ORDER BY rec_1.ord_path"),
                "{label}: {sql}"
            );
        }
    }

    #[test]
    fn repeat_that_cannot_be_identified_refuses_to_enclose_another() {
        for paths in [
            &[&["item"][..], &["answer", "item"][..]][..],
            &[&["answer", "item"][..]][..],
        ] {
            let plan = project(
                rec(
                    rec(scan(), "r.data", paths, "rec_0"),
                    "rec_0.node",
                    &[&["item"]],
                    "rec_1",
                ),
                link_of("rec_1.node"),
            );
            for (label, dialect, _) in targets() {
                let err = emit_plan(&plan, dialect).unwrap_err();
                assert!(
                    matches!(err, SofError::Uncompilable { .. }),
                    "{label}: {err:?}"
                );
            }
        }
    }

    #[test]
    fn filter_above_a_repeat_is_applied_not_a_panic() {
        let plan = project(
            PlanNode::Filter {
                parent: Box::new(rec(scan(), "r.data", &[&["item"]], "rec_0")),
                predicate: SqlExpr::Lit(LitValue::Bool(true)),
            },
            link_of("rec_0.node"),
        );
        for (label, dialect, _) in targets() {
            let sql = emit_plan(&plan, dialect).unwrap().sql;
            assert!(sql.contains("JOIN rec_0 ON"), "{label}: {sql}");
        }
    }

    #[test]
    fn unnest_over_a_repeat_keeps_postgres_ordinality() {
        let plan = project(
            PlanNode::LateralUnnest {
                parent: Box::new(rec(scan(), "r.data", &[&["item"]], "rec_0")),
                source: json_path("rec_0.node", vec![field("answer")]),
                out_alias: "fe".to_string(),
                left_join: false,
                on_filter: None,
                flat_index: None,
            },
            SqlExpr::RowIndex(RowIndexScope::ForEach("fe".into())),
        );
        let sql = emit_plan(&plan, &super::super::dialect::PgDialect)
            .unwrap()
            .sql;
        assert!(
            sql.contains("WITH ORDINALITY AS fe(value, ordinality)"),
            "{sql}"
        );
    }

    #[test]
    fn chained_and_cartesian_joins_keep_declared_occurrence_order() {
        let mut parent = scan();
        for (root, member, alias, left_join) in [
            ("r.data", "name", "z_outer", false),
            ("z_outer.value", "given", "a_inner", true),
            ("r.data", "telecom", "b_cartesian", false),
        ] {
            parent = PlanNode::LateralUnnest {
                parent: Box::new(parent),
                source: json_path(root, vec![field(member)]),
                out_alias: alias.into(),
                left_join,
                on_filter: None,
                flat_index: None,
            };
        }
        let plan = project(parent, json_path("a_inner.value", vec![]));
        for (label, dialect, _) in targets() {
            let emitted = emit_plan(&plan, dialect).unwrap();
            let collation = if dialect.name() == "sqlite" {
                "BINARY"
            } else {
                "\"C\""
            };
            let ordinal = if dialect.name() == "sqlite" {
                "rowid"
            } else {
                "ordinality"
            };
            assert_eq!(
                emitted.sql.lines().last().unwrap(),
                format!(
                    "ORDER BY r.last_updated, r.id COLLATE {collation}, COALESCE(z_outer.{ordinal}, -1), COALESCE(a_inner.{ordinal}, -1), COALESCE(b_cartesian.{ordinal}, -1)"
                ),
                "{label}: {}",
                emitted.sql
            );
            assert_eq!(emitted.columns, ["c"]);
        }
    }

    #[test]
    fn indexed_sqlite_hoist_removes_hidden_aliases_from_outer_order() {
        let prior = unnest(json_path("r.data", vec![field("name")]));
        let indexed = PlanNode::LateralUnnest {
            parent: Box::new(prior),
            source: json_path("fe.value", vec![field("given")]),
            out_alias: "picked".into(),
            left_join: false,
            on_filter: None,
            flat_index: Some(0),
        };
        let plan = project(
            PlanNode::LateralUnnest {
                parent: Box::new(indexed),
                source: json_path("r.data", vec![field("telecom")]),
                out_alias: "live".into(),
                left_join: false,
                on_filter: None,
                flat_index: None,
            },
            json_path("picked.value", vec![]),
        );
        let sqlite = emit_plan(&plan, &SqliteDialect).unwrap().sql;
        assert_eq!(
            sqlite.lines().last().unwrap(),
            "ORDER BY r.last_updated, r.id COLLATE BINARY, COALESCE(live.rowid, -1)"
        );
        let pg = emit_plan(&plan, &PgDialect).unwrap().sql;
        assert_eq!(
            pg.lines().last().unwrap(),
            "ORDER BY r.last_updated, r.id COLLATE \"C\", COALESCE(fe.ordinality, -1), COALESCE(live.ordinality, -1)"
        );
    }

    #[test]
    #[cfg(feature = "sqlite")]
    fn ordered_sqlite_cells_preserve_values_and_first_matching_occurrence() {
        use serde_json::json;
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        let resource = json!({
            "cells":[{"nested":["z",null]}, "null", "true", "0123", null],
            "name":[
                {"use":"old", "family":"Skipped", "given":["z","null","true","0123",null]},
                {"use":"official", "family":"Zulu", "given":["a"]},
                {"use":"official", "family":"Alpha", "given":[]}
            ]
        });
        let input = resource.to_string();
        for (path, expected) in [
            (vec![field("cells")], resource["cells"].clone()),
            (
                vec![field("name"), field("given")],
                json!(["z", "null", "true", "0123", null, "a"]),
            ),
        ] {
            let mut ctx = ExprCtx::new(&SqliteDialect, 3);
            let expr = SqlExpr::CollectionAgg {
                root: "?1".into(),
                path: JsonPath(path),
            };
            let sql = format!("SELECT {}", lower_expr(&expr, &mut ctx).unwrap());
            let result: String = connection
                .query_row(&sql, [&input], |row| row.get(0))
                .unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&result).unwrap(),
                expected
            );
        }
        let focus = json_path("?1", vec![field("name")]);
        let joined = SqlExpr::JoinAggregate {
            outer_focus: Box::new(focus.clone()),
            outer_alias: "ja0".into(),
            inner_field: Some("given".into()),
            inner_alias: "ja1".into(),
            separator: "|".into(),
        };
        let picked = SqlExpr::WhereScalar {
            focus: Box::new(focus),
            iter_alias: "w0".into(),
            predicate: Box::new(binop(
                BinOp::Eq,
                json_path("w0.value", vec![field("use")]),
                lit("official"),
            )),
            projection: Box::new(json_path("w0.value", vec![field("family")])),
        };
        for (expr, expected) in [(joined, "z|null|true|0123|a"), (picked, "Zulu")] {
            let mut ctx = ExprCtx::new(&SqliteDialect, 3);
            let sql = format!("SELECT {}", lower_expr(&expr, &mut ctx).unwrap());
            let result: String = connection
                .query_row(&sql, [&input], |row| row.get(0))
                .unwrap();
            assert_eq!(result, expected);
        }
    }

    fn binop(op: BinOp, lhs: SqlExpr, rhs: SqlExpr) -> SqlExpr {
        SqlExpr::BinOp {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        }
    }

    fn lit(value: &str) -> SqlExpr {
        SqlExpr::Lit(LitValue::Str(value.to_string()))
    }

    /// One plan per site that embeds a member name in SQL.
    fn plans_for(name: &str) -> Vec<(&'static str, PlanNode)> {
        let n = || field(name);
        vec![
            ("column", project(scan(), json_path("r.data", vec![n()]))),
            (
                "column, two segments",
                project(scan(), json_path("r.data", vec![n(), n()])),
            ),
            (
                "column, indexed",
                project(
                    scan(),
                    json_path("r.data", vec![n(), PathStep::Index(0), n()]),
                ),
            ),
            (
                "forEach source",
                project(
                    unnest(json_path("r.data", vec![n()])),
                    json_path("fe.value", vec![n()]),
                ),
            ),
            (
                "forEach source, guarded",
                project(
                    unnest(json_path("x.value", vec![n(), n()])),
                    json_path("fe.value", vec![]),
                ),
            ),
            (
                "forEach source, indexed",
                project(
                    unnest(json_path("r.data", vec![n(), PathStep::Index(0), n()])),
                    json_path("fe.value", vec![]),
                ),
            ),
            (
                "collection, one segment",
                project(
                    scan(),
                    SqlExpr::CollectionAgg {
                        root: "r.data".to_string(),
                        path: JsonPath(vec![n()]),
                    },
                ),
            ),
            (
                "collection, three segments",
                project(
                    scan(),
                    SqlExpr::CollectionAgg {
                        root: "r.data".to_string(),
                        path: JsonPath(vec![n(), n(), n()]),
                    },
                ),
            ),
            (
                "join aggregate",
                project(
                    scan(),
                    SqlExpr::JoinAggregate {
                        outer_focus: Box::new(json_path("r.data", vec![n()])),
                        outer_alias: "ja0".to_string(),
                        inner_field: Some(name.to_string()),
                        inner_alias: "ja1".to_string(),
                        separator: ",".to_string(),
                    },
                ),
            ),
            (
                "where exists",
                project(
                    filtered(SqlExpr::WhereExists {
                        focus: Box::new(json_path("r.data", vec![n()])),
                        iter_alias: "w0".to_string(),
                        predicate: Box::new(json_path("w0.value", vec![n()])),
                        negate: false,
                    }),
                    json_path("r.data", vec![]),
                ),
            ),
            (
                "repeat",
                project(
                    PlanNode::Recurse {
                        parent: Box::new(scan()),
                        seed_root: "r.data".to_string(),
                        step_paths: vec![JsonPath(vec![n()]), JsonPath(vec![n(), n()])],
                        out_alias: "rec_0".to_string(),
                    },
                    json_path("rec_0.node", vec![n()]),
                ),
            ),
        ]
    }

    /// One plan per site that inlines a string value into SQL: string
    /// literals in `where` (every operator shape the PostgreSQL emitter
    /// special-cases), `extension(url)` predicates, `iif`, `coalesce`,
    /// `forEach ... where`, a `join()` separator and a `getReferenceKey(T)`
    /// pattern.
    fn string_plans_for(value: &str) -> Vec<(&'static str, PlanNode)> {
        let gender = || json_path("r.data", vec![field("gender")]);
        let id = || json_path("r.data", vec![field("id")]);
        let url = || json_path("w0.value", vec![field("url")]);
        let extensions = || json_path("r.data", vec![field("extension")]);
        vec![
            (
                "where: path = literal",
                project(filtered(binop(BinOp::Eq, gender(), lit(value))), id()),
            ),
            (
                "where: literal = path",
                project(filtered(binop(BinOp::Eq, lit(value), gender())), id()),
            ),
            (
                "where: path != literal",
                project(filtered(binop(BinOp::Neq, gender(), lit(value))), id()),
            ),
            (
                "where: ordering comparison",
                project(filtered(binop(BinOp::Lt, gender(), lit(value))), id()),
            ),
            (
                "where: arithmetic",
                project(
                    filtered(binop(
                        BinOp::Gt,
                        binop(BinOp::Add, gender(), lit(value)),
                        SqlExpr::Lit(LitValue::Int(1)),
                    )),
                    id(),
                ),
            ),
            (
                "where: and/or",
                project(
                    filtered(binop(
                        BinOp::Or,
                        binop(BinOp::Eq, gender(), lit(value)),
                        binop(BinOp::Neq, gender(), lit(value)),
                    )),
                    id(),
                ),
            ),
            (
                "where: extension url exists",
                project(
                    filtered(SqlExpr::WhereExists {
                        focus: Box::new(extensions()),
                        iter_alias: "w0".to_string(),
                        predicate: Box::new(binop(BinOp::Eq, url(), lit(value))),
                        negate: false,
                    }),
                    id(),
                ),
            ),
            (
                "column: extension url",
                project(
                    scan(),
                    SqlExpr::WhereScalar {
                        focus: Box::new(extensions()),
                        iter_alias: "w0".to_string(),
                        predicate: Box::new(binop(BinOp::Eq, url(), lit(value))),
                        projection: Box::new(json_path("w0.value", vec![field("valueString")])),
                    },
                ),
            ),
            (
                "column: iif",
                project(
                    scan(),
                    SqlExpr::Case {
                        arms: vec![(
                            SqlExpr::UnaryOp {
                                op: UnaryOp::IsNotNull,
                                inner: Box::new(gender()),
                            },
                            lit(value),
                        )],
                        else_: Some(Box::new(lit(value))),
                    },
                ),
            ),
            (
                "column: coalesce",
                project(scan(), SqlExpr::Coalesce(vec![gender(), lit(value)])),
            ),
            (
                "column: nullif",
                project(
                    scan(),
                    SqlExpr::NullIf(Box::new(gender()), Box::new(lit(value))),
                ),
            ),
            ("column: bare literal", project(scan(), lit(value))),
            (
                "forEach: on-filter",
                project(
                    PlanNode::LateralUnnest {
                        parent: Box::new(scan()),
                        source: json_path("r.data", vec![field("name")]),
                        out_alias: "fe".to_string(),
                        left_join: true,
                        on_filter: Some(binop(
                            BinOp::Eq,
                            json_path("fe.value", vec![field("use")]),
                            lit(value),
                        )),
                        flat_index: None,
                    },
                    json_path("fe.value", vec![field("family")]),
                ),
            ),
            (
                "column: join separator",
                project(
                    scan(),
                    SqlExpr::JoinAggregate {
                        outer_focus: Box::new(json_path("r.data", vec![field("name")])),
                        outer_alias: "ja0".to_string(),
                        inner_field: Some("given".to_string()),
                        inner_alias: "ja1".to_string(),
                        separator: value.to_string(),
                    },
                ),
            ),
            (
                "column: getReferenceKey(Type)",
                project(
                    scan(),
                    SqlExpr::ReferenceKey {
                        reference: Box::new(json_path(
                            "r.data",
                            vec![field("subject"), field("reference")],
                        )),
                        expected_type: Some(value.to_string()),
                    },
                ),
            ),
        ]
    }

    #[test]
    fn hostile_member_names_stay_inside_their_literals() {
        for name in HOSTILE {
            for (label, plan) in plans_for(name) {
                for (target, dialect, backslash_escapes) in targets() {
                    let emitted = emit_plan(&plan, dialect).unwrap_or_else(|e| {
                        panic!("{target} {label} {name:?}: emit failed: {e:?}")
                    });
                    // Panics on an unterminated literal.
                    let outside = scan_literals(&emitted.sql, backslash_escapes).skeleton;
                    for marker in ["OR 1=1", "--", "DROP"] {
                        assert!(
                            !outside.contains(marker),
                            "{target} {label} {name:?}: {marker:?} escaped its literal:\n{}",
                            emitted.sql
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn string_values_stay_inside_their_literals() {
        let reference = string_plans_for(PLACEHOLDER);
        for value in STRING_VALUES {
            let hostile = string_plans_for(value);
            assert_eq!(reference.len(), hostile.len());
            for ((label, ref_plan), (_, plan)) in reference.iter().zip(&hostile) {
                for (target, dialect, backslash_escapes) in targets() {
                    let emit = |plan: &PlanNode| {
                        emit_plan(plan, dialect)
                            .unwrap_or_else(|e| {
                                panic!("{target} {label} {value:?}: emit failed: {e:?}")
                            })
                            .sql
                    };
                    let (expected_sql, sql) = (emit(ref_plan), emit(plan));
                    // Panics on an unterminated literal.
                    let expected = scan_literals(&expected_sql, backslash_escapes);
                    let got = scan_literals(&sql, backslash_escapes);
                    assert!(
                        expected.literals.iter().any(|l| l.contains(PLACEHOLDER)),
                        "{target} {label}: shape inlines no string:\n{expected_sql}"
                    );
                    // Nothing outside the literals moved...
                    assert_eq!(
                        got.skeleton, expected.skeleton,
                        "{target} {label} {value:?}: the value changed the SQL around it:\n{sql}"
                    );
                    // ...and every literal decodes to what it should be.
                    let want: Vec<String> = expected
                        .literals
                        .iter()
                        .map(|l| l.replace(PLACEHOLDER, value))
                        .collect();
                    assert_eq!(
                        got.literals, want,
                        "{target} {label} {value:?}: a literal means something else:\n{sql}"
                    );
                }
            }
        }
    }

    #[test]
    fn constants_are_bound_never_inlined() {
        // A `ViewDefinition.constant[]` lowers to `SqlExpr::Param`: the
        // emitter writes a placeholder and no literal, so the value (whatever
        // it contains) never appears in the SQL text.
        let plan_with = |rhs: SqlExpr| {
            project(
                filtered(binop(
                    BinOp::Eq,
                    json_path("r.data", vec![field("gender")]),
                    rhs,
                )),
                json_path("r.data", vec![field("id")]),
            )
        };
        for (target, dialect, backslash_escapes) in targets() {
            let bound = emit_plan(&plan_with(SqlExpr::Param(3)), dialect)
                .unwrap()
                .sql;
            let inlined = emit_plan(&plan_with(lit("v")), dialect).unwrap().sql;
            assert!(
                bound.contains(&format!("= {})", dialect.placeholder(3))),
                "{target}: {bound}"
            );
            // The same query with the value inlined has the literal `'v'`
            // (where the WHERE emitter repeats the predicate, once per
            // occurrence); the bound one has none, and nothing else differs.
            let bound_literals = scan_literals(&bound, backslash_escapes).literals;
            let inlined_literals = scan_literals(&inlined, backslash_escapes).literals;
            assert!(inlined_literals.iter().any(|l| l == "v"), "{target}");
            assert!(
                !bound_literals.iter().any(|l| l == "v"),
                "{target}: {bound}"
            );
            let without_v: Vec<String> =
                inlined_literals.into_iter().filter(|l| l != "v").collect();
            assert_eq!(bound_literals, without_v, "{target}: {bound}");
        }
    }

    #[test]
    fn lower_lit_string_golden() {
        for (value, sqlite, pg) in STRING_LITERAL_CASES {
            let lit = LitValue::Str((*value).to_string());
            assert_eq!(lower_lit(&lit, &SqliteDialect).unwrap(), *sqlite);
            assert_eq!(lower_lit(&lit, &PgDialect).unwrap(), *pg);
        }
    }

    #[test]
    fn plain_string_literals_emit_unchanged_sql() {
        // A `where` over a plain string is byte-for-byte what the hand-built
        // `'...'` produced before the dialect rendered it.
        let plan = project(
            filtered(binop(
                BinOp::Eq,
                json_path("r.data", vec![field("gender")]),
                lit("male"),
            )),
            json_path("r.data", vec![field("id")]),
        );
        let sqlite = emit_plan(&plan, &SqliteDialect).unwrap().sql;
        assert!(
            sqlite.contains("(json_extract(r.data, '$.gender') = 'male')"),
            "{sqlite}"
        );
        let pg = emit_plan(&plan, &PgDialect).unwrap().sql;
        assert!(pg.contains("(r.data->>'gender' = 'male')"), "{pg}");

        // A quote is doubled in both; a backslash changes only PostgreSQL.
        let with = |value: &str| {
            let plan = project(
                filtered(binop(
                    BinOp::Eq,
                    json_path("r.data", vec![field("gender")]),
                    lit(value),
                )),
                json_path("r.data", vec![field("id")]),
            );
            (
                emit_plan(&plan, &SqliteDialect).unwrap().sql,
                emit_plan(&plan, &PgDialect).unwrap().sql,
            )
        };
        let (sqlite, pg) = with("it's");
        assert!(sqlite.contains("= 'it''s')"), "{sqlite}");
        assert!(pg.contains("= 'it''s')"), "{pg}");
        let (sqlite, pg) = with("a\\b");
        assert!(sqlite.contains("= 'a\\b')"), "{sqlite}");
        assert!(pg.contains("= E'a\\\\b')"), "{pg}");
        let (sqlite, pg) = with("a\\'b");
        assert!(sqlite.contains("= 'a\\''b')"), "{sqlite}");
        assert!(pg.contains("= E'a\\\\''b')"), "{pg}");
    }

    #[test]
    fn string_values_with_nul_are_refused_not_altered() {
        for (label, plan) in string_plans_for("a\0b") {
            for (target, dialect, _) in targets() {
                match emit_plan(&plan, dialect) {
                    Err(SofError::Uncompilable { reason }) => {
                        assert!(reason.contains("NUL"), "{target} {label}: {reason}")
                    }
                    other => panic!("{target} {label}: expected Uncompilable, got {other:?}"),
                }
            }
        }
    }

    #[test]
    fn unlowerable_unnest_sources_are_refused_not_emptied() {
        // Both join kinds and the indexed-pick branch must preserve the
        // lowering error instead of compiling an empty iteration source.
        for bad in [
            LitValue::Str("a\0b".to_string()),
            LitValue::Decimal("not-a-number".to_string()),
        ] {
            for left_join in [false, true] {
                for flat_index in [None, Some(0)] {
                    let mut parent = unnest(SqlExpr::Lit(bad.clone()));
                    let PlanNode::LateralUnnest {
                        left_join: left,
                        flat_index: index,
                        ..
                    } = &mut parent
                    else {
                        unreachable!()
                    };
                    *left = left_join;
                    *index = flat_index;
                    let plan = project(parent, json_path("fe.value", vec![]));
                    for (target, dialect, _) in targets() {
                        let Err(SofError::Uncompilable { reason }) = emit_plan(&plan, dialect)
                        else {
                            panic!(
                                "{target} {bad:?} left={left_join} index={flat_index:?}: source must be refused"
                            )
                        };
                        let expected = match &bad {
                            LitValue::Str(_) => "NUL",
                            _ => "not a plain number",
                        };
                        assert!(reason.contains(expected), "{target}: {reason}");
                    }
                }
            }
        }
    }

    #[test]
    fn nested_computed_unnest_sources_preserve_the_lowering_error() {
        // Chained extension()/where() expressions create a computed focus
        // beneath another unnest, even though forEach itself admits paths only.
        let invalid_focus = SqlExpr::WhereScalar {
            focus: Box::new(json_path("r.data", vec![field("extension")])),
            iter_alias: "w0".to_string(),
            predicate: Box::new(binop(
                BinOp::Eq,
                json_path("w0.value", vec![field("url")]),
                lit("a\0b"),
            )),
            projection: Box::new(json_path("w0.value", vec![])),
        };
        let scalar = SqlExpr::WhereScalar {
            focus: Box::new(invalid_focus.clone()),
            iter_alias: "w1".to_string(),
            predicate: Box::new(SqlExpr::Lit(LitValue::Bool(true))),
            projection: Box::new(json_path("w1.value", vec![])),
        };
        let exists = SqlExpr::WhereExists {
            focus: Box::new(invalid_focus.clone()),
            iter_alias: "w1".to_string(),
            predicate: Box::new(SqlExpr::Lit(LitValue::Bool(true))),
            negate: false,
        };
        let join = SqlExpr::JoinAggregate {
            outer_focus: Box::new(invalid_focus.clone()),
            outer_alias: "ja0".to_string(),
            inner_field: Some("valueString".to_string()),
            inner_alias: "ja1".to_string(),
            separator: ",".to_string(),
        };
        let mut recursive_unnest = unnest(invalid_focus.clone());
        let PlanNode::LateralUnnest { parent, .. } = &mut recursive_unnest else {
            unreachable!()
        };
        **parent = PlanNode::Recurse {
            parent: Box::new(scan()),
            seed_root: "r.data".to_string(),
            step_paths: vec![JsonPath(vec![field("extension")])],
            out_alias: "rec_0".to_string(),
        };
        for plan in [
            project(unnest(invalid_focus), lit("value")),
            project(recursive_unnest, lit("value")),
            project(scan(), scalar),
            project(filtered(exists), json_path("r.data", vec![field("id")])),
            project(scan(), join),
        ] {
            for (target, dialect, _) in targets() {
                let Err(SofError::Uncompilable { reason }) = emit_plan(&plan, dialect) else {
                    panic!("{target}: nested computed source must be refused")
                };
                assert!(reason.contains("NUL"), "{target}: {reason}");
            }
        }
    }

    #[test]
    fn lowerable_computed_unnest_sources_keep_their_sql() {
        // Actual null is still a valid empty source. Only an error changes
        // behavior; the existing SQL guards retain their exact shape.
        let source = SqlExpr::Lit(LitValue::Null);
        assert_eq!(
            emit_sqlite_unnest_source(&source).unwrap(),
            "json_each(coalesce(NULL, '[]'))"
        );
        assert_eq!(
            emit_pg_unnest_source(&source).unwrap(),
            "(CASE WHEN jsonb_typeof((NULL)::jsonb) = 'array' THEN (NULL)::jsonb \
             WHEN jsonb_typeof((NULL)::jsonb) IS NOT NULL THEN jsonb_build_array((NULL)::jsonb) \
             ELSE '[]'::jsonb END)"
        );
    }

    #[test]
    fn decimal_literals_are_inlined_only_when_numeric() {
        for ok in ["0", "1.5", "-2", "+3.25", "10.000", "1e5", "1E-7", "2.5e+3"] {
            for dialect in [&SqliteDialect as &dyn Dialect, &PgDialect] {
                assert_eq!(
                    lower_lit(&LitValue::Decimal(ok.to_string()), dialect).unwrap(),
                    ok
                );
            }
        }
        for bad in [
            "",
            ".",
            "1.",
            ".5",
            "e5",
            "1e",
            "1e+",
            "--1",
            "1 OR 1",
            "1) OR (1",
            "1;",
            "'1'",
            "NaN",
            "inf",
            "0x10",
            "1_000",
            "\u{FF11}\u{FF12}",
            "1\0",
        ] {
            for dialect in [&SqliteDialect as &dyn Dialect, &PgDialect] {
                assert!(
                    matches!(
                        lower_lit(&LitValue::Decimal(bad.to_string()), dialect),
                        Err(SofError::Uncompilable { .. })
                    ),
                    "{bad:?} must be refused"
                );
            }
        }
    }

    #[test]
    fn plain_member_names_emit_unchanged_sql() {
        // The escaping must be invisible for real FHIR names: same text as
        // before the hardening.
        let plan = project(
            unnest(json_path("r.data", vec![field("name")])),
            json_path("fe.value", vec![field("family")]),
        );
        let sqlite = emit_plan(&plan, &SqliteDialect).unwrap().sql;
        assert!(
            sqlite.contains(
                "JOIN json_each(CASE WHEN json_type(r.data, '$.name') = 'array' \
                 THEN json_extract(r.data, '$.name')"
            ),
            "{sqlite}"
        );
        assert!(
            sqlite.contains("json_extract(fe.value, '$.family') AS \"c\""),
            "{sqlite}"
        );
        let pg = emit_plan(&plan, &PgDialect).unwrap().sql;
        assert!(pg.contains("r.data->'name'"), "{pg}");
        assert!(pg.contains("fe.value->>'family' AS \"c\""), "{pg}");
    }

    fn where_exists(focus_fields: &[&str]) -> PlanNode {
        project(
            filtered(SqlExpr::WhereExists {
                focus: Box::new(json_path(
                    "r.data",
                    focus_fields.iter().map(|f| field(f)).collect(),
                )),
                iter_alias: "w0".to_string(),
                predicate: Box::new(binop(
                    BinOp::Eq,
                    json_path("w0.value", vec![field("code")]),
                    lit("8480-6"),
                )),
                negate: false,
            }),
            json_path("r.data", vec![field("id")]),
        )
    }

    #[test]
    fn where_exists_unnests_every_repeating_step_of_the_focus() {
        let plan = where_exists(&["component", "code", "coding"]);

        let sqlite = emit_plan(&plan, &SqliteDialect).unwrap().sql;
        assert!(
            sqlite.contains("json_type(r.data, '$.component') = 'array'"),
            "{sqlite}"
        );
        assert!(
            sqlite.contains("json_type(w0_0.value, '$.code') = 'array'"),
            "{sqlite}"
        );
        assert!(
            sqlite.contains("json_type(w0_1.value, '$.coding') = 'array'"),
            "{sqlite}"
        );
        assert!(sqlite.contains(" w0_0, json_each("), "{sqlite}");
        assert!(sqlite.contains(" w0_1, json_each("), "{sqlite}");
        assert!(sqlite.contains(" w0 WHERE "), "{sqlite}");
        assert!(!sqlite.contains("'$.component.code"), "{sqlite}");

        let pg = emit_plan(&plan, &PgDialect).unwrap().sql;
        assert!(pg.contains("r.data->'component'"), "{pg}");
        assert!(pg.contains("w0_0.value->'code'"), "{pg}");
        assert!(pg.contains("w0_1.value->'coding'"), "{pg}");
        assert!(pg.contains("AS w0_0(value), "), "{pg}");
        assert!(pg.contains("AS w0_1(value), "), "{pg}");
        assert!(pg.contains("AS w0(value) WHERE "), "{pg}");
        assert!(!pg.contains("#>"), "{pg}");
    }

    #[test]
    fn where_exists_with_a_single_step_focus_is_unchanged() {
        let plan = where_exists(&["coding"]);
        let sqlite = emit_plan(&plan, &SqliteDialect).unwrap().sql;
        assert!(
            sqlite.contains(
                "EXISTS (SELECT 1 FROM json_each(CASE WHEN json_type(r.data, '$.coding') = 'array'"
            ),
            "{sqlite}"
        );
        assert!(!sqlite.contains("w0_0"), "{sqlite}");
        let pg = emit_plan(&plan, &PgDialect).unwrap().sql;
        assert!(pg.contains("AS w0(value) WHERE "), "{pg}");
        assert!(!pg.contains("w0_0"), "{pg}");
    }
}
