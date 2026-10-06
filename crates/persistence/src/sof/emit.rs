//! Lowers IR ([`PlanNode`]/[`SqlExpr`]) to a concrete SQL string for a given
//! [`Dialect`].
//!
//! The emitter expects each plan tree to have a [`PlanNode::Project`] at the
//! top (directly, or under a [`PlanNode::Union`]). Beneath the project lives a
//! chain of [`PlanNode::Filter`] and [`PlanNode::LateralUnnest`] nodes, rooted
//! in a [`PlanNode::Scan`]. The emitter walks that chain to assemble FROM /
//! JOIN / WHERE / SELECT in dialect-appropriate syntax, then concatenates them.
//!
//! Every statement ends in a total `ORDER BY` (see [the ordering
//! contract](#structured-composition-and-the-ordering-contract) below), and
//! [`ResourcePredicates`] are placed structurally on every resource scan.
//! Anything the emitter doesn't yet understand returns
//! [`SofError::Uncompilable`].
//!
//! # Structured composition and the ordering contract
//!
//! Every statement gets a total, documented order (all keys ascending):
//!
//! | shape | ORDER BY |
//! |---|---|
//! | ordinary / expanded select | `r.last_updated`, `r.id`, then every expansion occurrence ordinal in IR chain order |
//! | `unionAll` | first visible column (the historical `ORDER BY 1` value, explicit NULL placement), resource timestamp, resource id, common enclosing occurrence ordinals, flattened leaf-branch number, branch-local occurrence identity |
//! | `repeat:` (standalone) | first visible column (the historical `ORDER BY 1` value, explicit NULL placement), resource timestamp, resource id, complete traversal identity, post-repeat expansion occurrence ordinals |
//!
//! NULL placement of the first visible column is PostgreSQL `NULLS LAST`,
//! SQLite `NULLS FIRST`. Occurrence ordinals are zero-based element positions
//! (PostgreSQL `WITH ORDINALITY - 1`, SQLite `json_each.rowid`); a synthetic
//! `forEachOrNull` miss row orders as `-1`, before any real occurrence. An
//! indexed `forEach: "<chain>[N]"` contributes no ordinal: it selects at most
//! one occurrence per enclosing row.
//!
//! A recursive body's traversal identity is one edge per navigation from the
//! resource — repeat-path declaration index, every navigation occurrence
//! ordinal, terminator — so it is injective per resource and sorts in
//! pre-order (a node before its subtree, its subtree before its next
//! sibling). Post-repeat ordinals follow one more terminator, which keeps
//! "identity, then ordinals" a single comparable key (and so the same key in
//! a union's tail).
//!
//! A compound SELECT can only order by its output columns, so each union
//! branch projects an equal-width, equally-typed hidden tail after its
//! visible columns; an outer SELECT restores the visible names and applies
//! the ORDER BY over the tail. Every inner field uses a positional alias
//! (`c<n>` visible, `k<n>` hidden), so no user column name can collide with
//! them and none is reserved. Only visible columns reach
//! [`EmittedSql::columns`].
//!
//! # Reading the resource document once (PostgreSQL)
//!
//! A FHIR resource's `data` can be large enough to be TOASTed (compressed,
//! possibly stored out of line), and PostgreSQL detoasts a jsonb column
//! again for *every* operator that reads it. Resource-level projections are
//! evaluated once per output row, so a view projecting `k` paths over `n`
//! expansion rows of one resource paid for `k × n` detoasts of the same
//! document. Every PostgreSQL scan of `resources r` therefore binds the
//! document through [`Dialect::resource_document_lateral`] and every read
//! of it — projections, view `where` filters and `where()` criteria,
//! `forEach`/`forEachOrNull` sources and `ON` filters, indexed-`forEach`
//! chains, `%rowIndex`, every `unionAll` branch, `repeat:` seeds and the
//! recursive resource rejoin — is rooted at `rdoc.doc`
//! ([`Dialect::resource_document`], chosen when the plan is built), never at
//! `r.data`. The lateral's form depends on the scan's [`ScanFanOut`]:
//!
//! ```sql
//! -- Expanded: forEach / forEachOrNull / repeat rows multiply the resource row
//! FROM resources r
//! CROSS JOIN LATERAL jsonb_extract_path(r.data, VARIADIC '{}'::text[]) AS rdoc(doc)
//! -- Single: one output row per resource (flat selects, indexed picks)
//! FROM resources r
//! CROSS JOIN LATERAL (SELECT r.data AS doc) AS rdoc
//! ```
//!
//! - **Expanded.** The empty path returns the whole document, detoasted once
//!   per resource (a non-set-returning function scan yields exactly one
//!   row). On a 500k-Observation subset (PostgreSQL 16) a
//!   `forEach: component` view fell from 3.89M to 134k shared-buffer hits
//!   (≈18 per output row before); on a synthetic 1.5M-Observation table
//!   (12% with toasted components) it ran in 5.3 s instead of 30 s. A
//!   function scan stays parallel-safe: every benchmark view kept its
//!   previous plan (`Gather`, index and `Memoize` nodes alike) plus the
//!   one-row function scan. An `OFFSET 0` subquery
//!   (`(SELECT r.data #> '{}' AS doc OFFSET 0)`) was 12% faster on that
//!   Observation view but, referring to `r`, is parallel-restricted: the
//!   benchmark Patient `unionAll` and cartesian views planned serially and
//!   ran up to 2× (export) and 10× (preview) slower on untoasted documents.
//! - **Single.** A plain alias: the planner pulls it up and plans exactly
//!   the `r.data` statement. Detoasting costs more than it saves here —
//!   each document is read once per resource anyway: on 1.3M synthetic
//!   untoasted ~1 KB Observations the function scan made a flat statement
//!   17% slower (the subquery: serial and 2.2× slower).
//!
//! A one-row lateral has no statistics, so the planner takes `rdoc.doc` for
//! a single distinct value and caches each expansion behind a `Memoize`
//! keyed on whole documents, which never hits (2.8× slower on the subset
//! above). Expansion sources rooted at the document therefore read it as
//! [`Dialect::expansion_document`] — `COALESCE(rdoc.doc, r.data)`: the same
//! value, since `data` is `NOT NULL` the fallback is never evaluated, but
//! naming `r.data` (one distinct value per row in its statistics) puts it in
//! the cache key and the planner drops the `Memoize`. Sources that are not
//! plain paths off the document (`where(…)` chains lowered to scalars) keep
//! `rdoc.doc` and may still be memoized.
//!
//! Everything that does not read the document is unchanged: the
//! tenant/type/liveness conjuncts, the [`ResourcePredicates`] (they read
//! `r.last_updated`, `r.id` and `search_index`), the ORDER BY keys and the
//! hidden union keys, so a preview's final `LIMIT` still walks `resources`
//! in index order. SQLite (and MongoDB) read `r.data` directly; their output
//! is byte-identical.

use std::borrow::Cow;

use crate::core::sof_runner::SofError;

use super::decode::ColumnDecode;
use super::dialect::{Dialect, SCANNED_DOCUMENT, ScanFanOut};
use super::ir::{
    BinOp, BoundaryKind, BoundarySide, JsonPath, JsonType, LitValue, PathStep, PlanNode,
    RowIndexScope, SqlExpr, SqlType, UnaryOp,
};

/// Column of the recursive `repeat` CTE holding each node's complete
/// traversal identity (see [`KeyEncoding::recursion_edge`]): PostgreSQL
/// `bigint[]`, SQLite BINARY-sortable fixed-width TEXT. It sorts in
/// pre-order and is injective per resource.
const REPEAT_IDENTITY_COL: &str = "ident";

/// Column the `%rowIndex` ranking pass adds over the recursive CTE rows: the
/// node's zero-based pre-order position per resource.
const REPEAT_ROW_INDEX_COL: &str = "row_index";

/// CTE column carrying the seed resource's `last_updated` through every
/// recursive step, so a recursive body orders (and fills a union's hidden
/// tail) without rejoining `resources`.
const RECURSE_LAST_UPDATED_COL: &str = "last_updated";

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
}

/// Resource-level runtime predicates (`_since`, Patient/Group compartment)
/// that the emitter attaches to **every** scan of `resources r`: each plain
/// select, each `unionAll` branch, each recursive (`repeat:`) seed, and the
/// resource rejoin of a recursive select. They are resource-row predicates,
/// so they never move into iteration scopes (`forEach` ON filters,
/// `where()` membership).
///
/// Bound-parameter layout shared by the SQL runners:
///
/// | slots | bound value |
/// |---|---|
/// | `1`, `2` | `tenant_id`, `resource_type` |
/// | `3 ..= 2 + constants.len()` | `ViewDefinition.constant[]`, allocated by `build_plan` |
/// | `first_param .. first_param + param_count` | runtime filter values |
///
/// The runtime slots are allocated once, before emission, starting at
/// `3 + constants.len()`; every fragment reuses the same placeholders in
/// every scan, so each value is bound exactly once. The emitter itself never
/// allocates a slot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourcePredicates {
    first_param: usize,
    param_count: usize,
    fragments: Vec<String>,
}

impl ResourcePredicates {
    /// No runtime predicates — the emitted SQL is the plain compiled view.
    pub fn none() -> Self {
        Self::default()
    }

    /// Predicates whose `fragments` reference the scanned resource row as
    /// `r` and bind only `$1`/`$2` plus the slots
    /// `first_param .. first_param + param_count`.
    pub fn new(first_param: usize, param_count: usize, fragments: Vec<String>) -> Self {
        Self {
            first_param,
            param_count,
            fragments,
        }
    }

    /// First bound-parameter slot the fragments use.
    pub fn first_param(&self) -> usize {
        self.first_param
    }

    /// Number of bound-parameter slots the fragments use.
    pub fn param_count(&self) -> usize {
        self.param_count
    }

    /// AND-composed predicate fragments, in binding order.
    pub fn fragments(&self) -> &[String] {
        &self.fragments
    }
}

/// Lowers a plan tree to SQL for the given dialect, without runtime filters.
///
/// # Errors
///
/// Returns [`SofError::InvalidViewDefinition`] for structurally invalid plans
/// and [`SofError::Uncompilable`] for IR shapes outside the implemented subset
/// at this stage.
pub fn emit_plan(plan: &PlanNode, dialect: &dyn Dialect) -> Result<EmittedSql, SofError> {
    emit_plan_with_predicates(plan, dialect, &ResourcePredicates::none())
}

/// Lowers a plan tree to SQL, attaching `predicates` to every resource scan
/// (see [`ResourcePredicates`]).
///
/// # Errors
///
/// As [`emit_plan`].
pub fn emit_plan_with_predicates(
    plan: &PlanNode,
    dialect: &dyn Dialect,
    predicates: &ResourcePredicates,
) -> Result<EmittedSql, SofError> {
    let composed = compose_plan(plan, dialect, predicates)?;
    Ok(EmittedSql {
        sql: composed.render(),
        columns: composed.columns,
        column_decodes: composed.column_decodes,
    })
}

/// Composes a plan into structured SELECT bodies, before final rendering.
fn compose_plan(
    plan: &PlanNode,
    dialect: &dyn Dialect,
    predicates: &ResourcePredicates,
) -> Result<ComposedQuery, SofError> {
    match plan {
        PlanNode::Union(branches) => compose_union(branches, dialect, predicates),
        _ => compose_leaf(plan, dialect, predicates).map(ComposedQuery::select),
    }
}

/// Composes one non-union `Project` plan: a recursive (`repeat:`) body when
/// it is rooted in a `Recurse`, otherwise an ordinary/expanded select.
fn compose_leaf(
    plan: &PlanNode,
    dialect: &dyn Dialect,
    predicates: &ResourcePredicates,
) -> Result<(SelectBody, Vec<String>, Vec<ColumnDecode>), SofError> {
    match plan {
        PlanNode::Project { parent, .. } if contains_recurse(parent) => {
            compose_recurse_select(plan, dialect, predicates)
        }
        _ => compose_select(plan, dialect, predicates),
    }
}

// ============================================================================
// Structured composition and the ordering contract — documented in the
// module docs above (the canonical per-shape ORDER BY table).
// ============================================================================

/// Ordering a standalone composed statement receives when rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalOrder {
    /// `ORDER BY r.last_updated, r.id, <occurrence ordinals…>` — ordinary
    /// and expanded selects.
    ResourceKey,
    /// `ORDER BY 1 <explicit NULLS>, <last_updated>, <id>, <traversal
    /// identity>` — standalone recursive (`repeat:`) selects.
    Traversal(KeyEncoding),
}

/// One projected output column of a select body.
#[derive(Debug, Clone)]
struct Projection {
    /// Lowered, cast SQL value expression (no alias).
    expr: String,
    /// Visible column name (already checked by [`sanitize_ident`]).
    name: String,
}

/// One row-producing expansion (`forEach` / `forEachOrNull` lateral unnest)
/// of a select body, in IR chain order.
#[derive(Debug, Clone)]
struct Occurrence {
    /// Iteration alias (`fe`, `fe2`, …). Unions are built from shared unnests
    /// cloned into every branch plus fresh branch-local aliases, so the
    /// common alias prefix of the branches is their enclosing iteration.
    alias: String,
    /// Zero-based occurrence ordinal: PostgreSQL `bigint`, SQLite INTEGER.
    /// `-1` marks a synthetic `forEachOrNull` miss row.
    ordinal: String,
}

/// Resource ordering key a body can project: timestamp and id.
#[derive(Debug, Clone)]
struct ResourceKeySql {
    last_updated: String,
    id: String,
}

impl ResourceKeySql {
    fn scanned() -> Self {
        Self {
            last_updated: "r.last_updated".to_string(),
            id: "r.id".to_string(),
        }
    }
}

/// One `SELECT` operand, kept in pieces until final rendering. The union
/// composer combines bodies without any textual surgery on rendered SQL.
#[derive(Debug, Clone)]
struct SelectBody {
    /// `<name>(<cols>) AS (…)` definition of the `WITH RECURSIVE` CTE a
    /// `repeat:` body reads from. Its presence marks the body recursive.
    recursive_cte: Option<String>,
    /// Projected visible columns, in output-column order.
    projections: Vec<Projection>,
    /// `FROM` clause contents (resource scan or CTE, plus joins).
    from: String,
    /// AND-composed `WHERE` conjuncts; empty renders no `WHERE`.
    conjuncts: Vec<String>,
    /// Ordering the body receives when rendered as a whole statement.
    order: FinalOrder,
    /// Resource key in scope for this body's rows (a recursive body reads
    /// the key its CTE carries).
    resource_key: Option<ResourceKeySql>,
    /// Expansion occurrences of a non-recursive body, in IR chain order. A
    /// recursive body folds its post-repeat ordinals into
    /// `traversal_identity` and leaves this empty.
    occurrences: Vec<Occurrence>,
    /// Identity of a recursive body's rows, in the union identity type
    /// (PostgreSQL `bigint[]`, SQLite BINARY-sortable TEXT): the complete
    /// traversal identity (path index, every navigation occurrence, edge
    /// terminators), then — after one more terminator — any post-repeat
    /// expansion ordinals.
    traversal_identity: Option<String>,
}

impl SelectBody {
    fn is_recursive(&self) -> bool {
        self.recursive_cte.is_some()
    }

    /// Visible projection items: `<expr> AS "<name>"`.
    fn named_items(&self) -> Vec<String> {
        self.projections
            .iter()
            .map(|p| format!("{} AS \"{}\"", p.expr, p.name))
            .collect()
    }

    /// Renders the body with the given SELECT-list items and no ordering.
    fn render_with(&self, items: &[String]) -> String {
        let mut sql = String::new();
        if let Some(cte) = &self.recursive_cte {
            sql.push_str("WITH RECURSIVE ");
            sql.push_str(cte);
            sql.push('\n');
        }
        sql.push_str("SELECT\n  ");
        sql.push_str(&items.join(",\n  "));
        sql.push_str("\nFROM ");
        sql.push_str(&self.from);
        if !self.conjuncts.is_empty() {
            sql.push_str("\nWHERE ");
            sql.push_str(&self.conjuncts.join("\n  AND "));
        }
        sql
    }

    /// `ORDER BY` list for the body rendered as a whole statement.
    fn order_by(&self) -> String {
        match self.order {
            FinalOrder::ResourceKey => {
                let key = self
                    .resource_key
                    .clone()
                    .unwrap_or_else(ResourceKeySql::scanned);
                let mut keys = vec![key.last_updated, key.id];
                keys.extend(self.occurrences.iter().map(|o| o.ordinal.clone()));
                keys.join(", ")
            }
            FinalOrder::Traversal(encoding) => {
                let key = self
                    .resource_key
                    .clone()
                    .unwrap_or_else(ResourceKeySql::scanned);
                let mut keys = vec![encoding.primary_key("1"), key.last_updated, key.id];
                if let Some(identity) = &self.traversal_identity {
                    keys.push(encoding.identity_key(identity));
                }
                keys.join(", ")
            }
        }
    }
}

/// Dialect-specific encodings of the union's hidden ordering keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyEncoding {
    /// `bigint` ordinals/branch number, `bigint[]` identity, NULLS LAST.
    Postgres,
    /// INTEGER ordinals/branch number, fixed-width TEXT identity compared
    /// with `COLLATE BINARY`, NULLS FIRST.
    Sqlite,
}

impl KeyEncoding {
    fn of(dialect: &dyn Dialect) -> Self {
        if dialect.lateral_keyword().is_empty() {
            Self::Sqlite
        } else {
            Self::Postgres
        }
    }

    /// Flattened leaf-branch number.
    fn branch_number(self, index: usize) -> String {
        match self {
            Self::Postgres => format!("CAST({index} AS bigint)"),
            Self::Sqlite => index.to_string(),
        }
    }

    /// Branch-local occurrence identity from zero-based ordinals (`-1` for a
    /// synthetic miss). PostgreSQL compares `bigint[]` element-wise; SQLite
    /// concatenates fixed-width tokens `ordinal + 1` (so `-1` → all zeros)
    /// that compare correctly as BINARY text. Either way a prefix sorts
    /// before its extensions.
    fn identity(self, ordinals: &[&str]) -> String {
        match self {
            Self::Postgres => format!("ARRAY[{}]::bigint[]", ordinals.join(", ")),
            Self::Sqlite if ordinals.is_empty() => "''".to_string(),
            Self::Sqlite => ordinals
                .iter()
                .map(|ordinal| format!("printf('%020d', {ordinal} + 1)"))
                .collect::<Vec<_>>()
                .join(" || "),
        }
    }

    /// The first visible column with today's default NULL placement made
    /// explicit (PostgreSQL ASC defaults to NULLS LAST, SQLite to FIRST).
    fn primary_key(self, column: &str) -> String {
        match self {
            Self::Postgres => format!("{column} ASC NULLS LAST"),
            Self::Sqlite => format!("{column} ASC NULLS FIRST"),
        }
    }

    fn identity_key(self, column: &str) -> String {
        match self {
            Self::Postgres => column.to_string(),
            // `COLLATE` binds tighter than `||`: parenthesize compound
            // identities so the collation covers the whole key.
            Self::Sqlite if column.contains(' ') => format!("({column}) COLLATE BINARY"),
            Self::Sqlite => format!("{column} COLLATE BINARY"),
        }
    }

    /// One recursive edge of a traversal identity: the repeat-path
    /// declaration index, the zero-based occurrence ordinal of every
    /// navigation along the path (a seed's single full-path unnest, or one
    /// per `Field` segment of a step), and a terminator.
    ///
    /// PostgreSQL: a `bigint[]` of non-negative path/occurrence tokens closed
    /// by `-1`. SQLite: concatenated 20-digit decimal tokens (wide enough for
    /// any signed 64-bit value), each non-negative value `+ 1`, terminator
    /// `0`, compared as BINARY text. Either way an identity is a prefix of —
    /// and sorts before — every descendant's; distinct paths, repeated paths
    /// and different seeds produce distinct identities.
    fn recursion_edge(self, path_index: usize, ordinals: &[String]) -> String {
        match self {
            Self::Postgres => {
                let mut tokens = vec![path_index.to_string()];
                tokens.extend(ordinals.iter().cloned());
                tokens.push("-1".to_string());
                format!("ARRAY[{}]::bigint[]", tokens.join(", "))
            }
            Self::Sqlite => {
                let mut tokens = vec![format!("'{:020}'", path_index + 1)];
                tokens.extend(
                    ordinals
                        .iter()
                        .map(|ordinal| format!("printf('%020d', {ordinal} + 1)")),
                );
                tokens.push(Self::SQLITE_TERMINATOR.to_string());
                tokens.join(" || ")
            }
        }
    }

    /// SQLite edge terminator token (`0`, 20 digits wide).
    const SQLITE_TERMINATOR: &'static str = "'00000000000000000000'";

    /// A recursive body's complete row identity: the traversal identity,
    /// then — after one more terminator — the post-repeat expansion
    /// ordinals (`-1` for a miss). The terminator sorts below any path
    /// index, so this single key orders exactly like the tuple
    /// `(traversal identity, ordinals…)`.
    fn close_traversal(self, identity: &str, ordinals: &[&str]) -> String {
        if ordinals.is_empty() {
            return identity.to_string();
        }
        match self {
            Self::Postgres => format!("{identity} || ARRAY[-1, {}]::bigint[]", ordinals.join(", ")),
            Self::Sqlite => format!(
                "{identity} || {} || {}",
                Self::SQLITE_TERMINATOR,
                self.identity(ordinals)
            ),
        }
    }
}

/// Derived-table alias of the wrapped `UNION ALL`.
const UNION_ALIAS: &str = "u";

/// A composed `unionAll`: flattened leaf bodies, each with its hidden tail.
#[derive(Debug, Clone)]
struct UnionQuery {
    branches: Vec<UnionBranch>,
    /// ORDER BY items over the wrapped union's internal aliases.
    order: Vec<String>,
}

#[derive(Debug, Clone)]
struct UnionBranch {
    body: SelectBody,
    /// Hidden key expressions projected as `k1..km`; equal width and types
    /// in every branch of one union.
    hidden: Vec<String>,
}

impl UnionBranch {
    /// Renders the operand with positional aliases: visible `c<n>`, hidden
    /// `k<n>`.
    fn render(&self, index: usize) -> String {
        let mut items: Vec<String> = self
            .body
            .projections
            .iter()
            .enumerate()
            .map(|(i, p)| format!("{} AS c{}", p.expr, i + 1))
            .collect();
        items.extend(
            self.hidden
                .iter()
                .enumerate()
                .map(|(i, key)| format!("{key} AS k{}", i + 1)),
        );
        let body = self.body.render_with(&items);
        // A recursive body can't appear bare in a compound SELECT — neither
        // dialect allows `WITH ... UNION ALL WITH ...` — so it becomes
        // `SELECT * FROM (WITH ... SELECT ...) AS _recurse_<i>`. PG requires
        // the derived-table alias; SQLite ignores it.
        if self.body.is_recursive() {
            format!("SELECT * FROM ({body}) AS _recurse_{index}")
        } else {
            body
        }
    }
}

/// Statement shape: a single body or a wrapped `UNION ALL` of bodies.
#[derive(Debug, Clone)]
enum QueryShape {
    Select(SelectBody),
    UnionAll(UnionQuery),
}

/// A composed statement plus its visible output columns.
#[derive(Debug, Clone)]
struct ComposedQuery {
    shape: QueryShape,
    columns: Vec<String>,
    column_decodes: Vec<ColumnDecode>,
}

impl ComposedQuery {
    fn select(
        (body, columns, column_decodes): (SelectBody, Vec<String>, Vec<ColumnDecode>),
    ) -> Self {
        Self {
            shape: QueryShape::Select(body),
            columns,
            column_decodes,
        }
    }

    /// Renders the full statement with its final ordering.
    fn render(&self) -> String {
        match &self.shape {
            QueryShape::Select(body) => format!(
                "{}\nORDER BY {}",
                body.render_with(&body.named_items()),
                body.order_by()
            ),
            QueryShape::UnionAll(union) => {
                let visible: Vec<String> = union
                    .branches
                    .first()
                    .map(|branch| {
                        branch
                            .body
                            .projections
                            .iter()
                            .enumerate()
                            .map(|(i, p)| format!("{UNION_ALIAS}.c{} AS \"{}\"", i + 1, p.name))
                            .collect()
                    })
                    .unwrap_or_default();
                let operands: Vec<String> = union
                    .branches
                    .iter()
                    .enumerate()
                    .map(|(index, branch)| branch.render(index))
                    .collect();
                format!(
                    "SELECT\n  {}\nFROM (\n{}\n) AS {UNION_ALIAS}\nORDER BY {}",
                    visible.join(",\n  "),
                    operands.join("\nUNION ALL\n"),
                    union.order.join(", ")
                )
            }
        }
    }
}

/// Tenant/type/liveness predicate on the scanned resource row `r`.
fn tenant_predicate(dialect: &dyn Dialect) -> String {
    format!(
        "r.tenant_id = {}\n  AND r.resource_type = {}\n  AND r.is_deleted = {}",
        dialect.placeholder(1),
        dialect.placeholder(2),
        dialect.bool_false()
    )
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

/// Compose a `SELECT … FROM … WHERE …` body for a non-Union, non-recursive
/// plan, ordered by resource key.
fn compose_select(
    plan: &PlanNode,
    dialect: &dyn Dialect,
    predicates: &ResourcePredicates,
) -> Result<(SelectBody, Vec<String>, Vec<ColumnDecode>), SofError> {
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

    let (projections, columns, column_decodes) = project_columns(project_cols, dialect)?;
    if projections.is_empty() {
        return Err(SofError::InvalidViewDefinition(
            "no output columns".to_string(),
        ));
    }

    // Build FROM clause: `resources r` (plus the dialect's document lateral)
    // + any LATERAL joins, in order of appearance from the bottom of the tree
    // upward (Scan first, then unnests). Every row-multiplying unnest left an
    // occurrence; indexed picks yield at most one row and leave none.
    let fan_out = if frame.occurrences.is_empty() {
        ScanFanOut::Single
    } else {
        ScanFanOut::Expanded
    };
    let mut from = format!("{} r", scan.table);
    if let Some(document) = dialect.resource_document_lateral(fan_out) {
        from.push('\n');
        from.push_str(document);
    }
    for join in &frame.joins {
        from.push('\n');
        from.push_str(&join.sql);
    }

    // WHERE clause: tenant predicate first (so `$1`/`$2` line up), then the
    // view's `where` filters, then the runtime resource predicates.
    let mut conjuncts: Vec<String> = vec![tenant_predicate(dialect)];
    conjuncts.extend(frame.predicates);
    conjuncts.extend(predicates.fragments().iter().cloned());

    Ok((
        SelectBody {
            recursive_cte: None,
            projections,
            from,
            conjuncts,
            order: FinalOrder::ResourceKey,
            resource_key: Some(ResourceKeySql::scanned()),
            occurrences: frame.occurrences,
            traversal_identity: None,
        },
        columns,
        column_decodes,
    ))
}

/// Lowers `Project` columns to cast value expressions plus their names.
fn project_columns(
    project_cols: &[super::ir::Column],
    dialect: &dyn Dialect,
) -> Result<(Vec<Projection>, Vec<String>, Vec<ColumnDecode>), SofError> {
    let mut projections: Vec<Projection> = Vec::with_capacity(project_cols.len());
    let mut columns: Vec<String> = Vec::with_capacity(project_cols.len());
    let mut column_decodes: Vec<ColumnDecode> = Vec::with_capacity(project_cols.len());
    for col in project_cols {
        if col.collection {
            return Err(SofError::Uncompilable {
                reason: "column.collection=true is not yet supported by the in-DB runner"
                    .to_string(),
            });
        }
        let mut expr_ctx = ExprCtx::new(dialect);
        let expr_sql = lower_expr(&col.expr, &mut expr_ctx)?;
        let casted = match col.ty {
            // Default text projection. Path-rooted expressions already produce
            // text via `->>` (PG) / `json_extract` (SQLite); compound
            // expressions (boolean predicates, arithmetic, ...) need an
            // explicit text cast so the runners' `Option<String>` row reader
            // can deserialize them.
            SqlType::Text => project_text(&col.expr, &expr_sql, dialect),
            other => dialect.cast(&expr_sql, other),
        };
        projections.push(Projection {
            expr: casted,
            name: sanitize_ident(&col.name)?.to_string(),
        });
        columns.push(col.name.clone());
        column_decodes.push(col.decode);
    }
    Ok((projections, columns, column_decodes))
}

/// Compose a `WITH RECURSIVE … SELECT … FROM <cte> [JOIN resources r ON r.id = <cte>.rid]`
/// body for a `Project` whose parent is a [`PlanNode::Recurse`].
///
/// The CTE projects `(rid, node, ident, last_updated)`:
///
/// - `rid` / `last_updated` — the seed resource's key, carried through every
///   step, so the body orders (and fills a union's hidden tail) without a
///   resource rejoin.
/// - `ident` — the node's complete traversal identity (see
///   [`KeyEncoding::recursion_edge`]): one edge per navigation from the
///   resource, each the repeat-path declaration index, every navigation
///   occurrence ordinal and a terminator. It is collision-free across paths,
///   repeated paths and seeds, and a parent's identity is a prefix of — so
///   sorts before — its descendants' (pre-order, the evaluator's traversal
///   order).
///
/// Row production is unchanged from the original lowering: each seed is one
/// unnest of the full path off the resource document, each step one unnest
/// per `Field` segment off the parent node; the ordinals only observe those
/// unnests. PostgreSQL multi-path recursion keeps a single recursive
/// self-reference (a lateral `UNION ALL` of the step paths, each returning
/// its edge).
///
/// `resources r` is rejoined only when the compile-time sidecar
/// ([`PlanNode::Recurse::needs_resource_row`]) — or, defensively, a
/// projection or post-repeat expansion source — reads the resource row. The
/// runtime resource predicates constrain every seed and that rejoin, and on
/// PostgreSQL both bind the detoasted document lateral (see the module
/// docs).
///
/// Repeat-scope `%rowIndex` is ranked over `ident` per resource in a pass
/// over the CTE rows *before* post-repeat expansion joins can multiply or
/// drop nodes.
fn compose_recurse_select(
    plan: &PlanNode,
    dialect: &dyn Dialect,
    predicates: &ResourcePredicates,
) -> Result<(SelectBody, Vec<String>, Vec<ColumnDecode>), SofError> {
    let PlanNode::Project {
        parent: body,
        columns: project_cols,
    } = plan
    else {
        return Err(SofError::InvalidViewDefinition(
            "plan tree must have a Project node at the top".to_string(),
        ));
    };

    // Walk down past any LateralUnnest layers wrapping the Recurse — those
    // are post-repeat (nested or sibling) forEach unnests that get JOINed
    // onto the recursive CTE alias — and the membership filters of indexed
    // `forEach` clauses beside or under the repeat, which constrain the
    // joined rows.
    let mut extra_unnests: Vec<&PlanNode> = Vec::new();
    let mut membership_filters: Vec<&SqlExpr> = Vec::new();
    let mut cur = body.as_ref();
    loop {
        match cur {
            PlanNode::LateralUnnest { parent, .. } => {
                extra_unnests.push(cur);
                cur = parent.as_ref();
            }
            PlanNode::Filter { parent, predicate } => {
                membership_filters.push(predicate);
                cur = parent.as_ref();
            }
            _ => break,
        }
    }
    let PlanNode::Recurse {
        parent: parent_plan,
        step_paths,
        out_alias,
        needs_resource_row,
        ..
    } = cur
    else {
        // E.g. a row filter between the projection and the recursion.
        return Err(SofError::Uncompilable {
            reason: "select.repeat under a row filter or other non-forEach row source is not \
                     yet supported by the in-DB runner"
                .to_string(),
        });
    };
    let out_alias = out_alias.as_str();

    // Walk the parent plan to collect the scan plus any top-level `where[]`
    // filters. We expect a Scan with optional Filter chain — no unnests at
    // this level (rejected upstream).
    let mut frame = Frame::new();
    walk_body(parent_plan, dialect, &mut frame)?;
    let scan = frame
        .scan
        .as_ref()
        .ok_or_else(|| SofError::InvalidViewDefinition("plan has no Scan node".to_string()))?;

    // Seed predicate: tenant, the view's `where` filters, then the runtime
    // resource predicates — every seed scans `resources r`.
    let tenant_pred = tenant_predicate(dialect);
    let mut where_pred = tenant_pred.clone();
    for p in frame.predicates.iter().chain(predicates.fragments()) {
        where_pred.push_str("\n  AND ");
        where_pred.push_str(p);
    }

    let encoding = KeyEncoding::of(dialect);
    let lateral = dialect.lateral_keyword();
    let ident = REPEAT_IDENTITY_COL;
    let lu = RECURSE_LAST_UPDATED_COL;

    // Seeds — one SELECT per repeat path, its edge opening the identity.
    let mut seed_branches: Vec<String> = Vec::with_capacity(step_paths.len());
    for (path_index, path) in step_paths.iter().enumerate() {
        let src = SqlExpr::JsonPath {
            root: dialect.expansion_document().to_string(),
            path: path.clone(),
        };
        let branch = match encoding {
            KeyEncoding::Sqlite => {
                let edge = encoding.recursion_edge(path_index, &["je.rowid".to_string()]);
                format!(
                    "SELECT r.id AS rid, je.value AS node, {edge} AS {ident}, r.{lu}\n  \
                     FROM {} r, {} je\n  WHERE {where_pred}",
                    scan.table,
                    emit_sqlite_unnest_source(&src),
                )
            }
            KeyEncoding::Postgres => {
                let edge = encoding.recursion_edge(path_index, &["(je.ord - 1)".to_string()]);
                let unnest = dialect.unnest_array(&emit_pg_unnest_source(&src));
                let document = dialect
                    .resource_document_lateral(ScanFanOut::Expanded)
                    .map(|lateral| format!(" {lateral}"))
                    .unwrap_or_default();
                format!(
                    "SELECT r.id AS rid, je.value AS node, {edge} AS {ident}, r.{lu}\n  \
                     FROM {} r{document} JOIN {lateral}{unnest} WITH ORDINALITY AS je(value, ord) \
                     ON TRUE\n  \
                     WHERE {where_pred}",
                    scan.table,
                )
            }
        };
        seed_branches.push(branch);
    }

    // Steps — walk each path off `<alias>.node`. Multi-segment step paths
    // (`answer.item`) chain a lateral unnest per Field so path-through-array
    // flattening matches FHIRPath semantics; every one of those unnests
    // contributes its occurrence ordinal to the edge.
    //
    // PG additionally requires that a recursive CTE reference its own name
    // at most once. When `step_paths` has more than one entry, fold all
    // step navigations into a single `SELECT … FROM rec_0, LATERAL (path₁
    // UNION ALL path₂ UNION ALL …) AS _step(value, edge)`.
    let pg_single_reference = encoding == KeyEncoding::Postgres && step_paths.len() > 1;
    let mut step_branches: Vec<String> = Vec::with_capacity(step_paths.len());
    let mut pg_lateral_branches: Vec<String> = Vec::new();
    for (path_index, path) in step_paths.iter().enumerate() {
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
        let mut from_parts: Vec<String> = Vec::with_capacity(segs.len());
        let mut ordinals: Vec<String> = Vec::with_capacity(segs.len());
        for (i, field) in segs.iter().enumerate() {
            let alias = format!("rs{i}");
            let src = SqlExpr::JsonPath {
                root: prev_root.clone(),
                path: super::ir::JsonPath(vec![PathStep::Field((*field).to_string())]),
            };
            match encoding {
                KeyEncoding::Sqlite => {
                    from_parts.push(format!("{} {alias}", emit_sqlite_unnest_source(&src)));
                    ordinals.push(format!("{alias}.rowid"));
                }
                KeyEncoding::Postgres => {
                    from_parts.push(format!(
                        "{lateral}{} WITH ORDINALITY AS {alias}(value, ord)",
                        dialect.unnest_array(&emit_pg_unnest_source(&src))
                    ));
                    ordinals.push(format!("({alias}.ord - 1)"));
                }
            }
            prev_root = format!("{alias}.value");
        }
        let leaf_alias = format!("rs{}", segs.len() - 1);
        let edge = encoding.recursion_edge(path_index, &ordinals);
        match encoding {
            KeyEncoding::Sqlite => step_branches.push(format!(
                "SELECT {out_alias}.rid, {leaf_alias}.value AS node, \
                 {out_alias}.{ident} || {edge} AS {ident}, {out_alias}.{lu}\n  \
                 FROM {out_alias}, {}",
                from_parts.join(", ")
            )),
            KeyEncoding::Postgres if pg_single_reference => {
                // A sub-SELECT returning the leaf value and its edge; all of
                // them are wrapped in one LATERAL below.
                let chain = from_parts
                    .iter()
                    .enumerate()
                    .map(|(i, part)| {
                        if i == 0 {
                            part.clone()
                        } else {
                            format!("JOIN {part} ON TRUE")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                pg_lateral_branches.push(format!("SELECT {leaf_alias}.value, {edge} FROM {chain}"));
            }
            KeyEncoding::Postgres => {
                let joins: String = from_parts
                    .iter()
                    .map(|part| format!(" JOIN {part} ON TRUE"))
                    .collect();
                step_branches.push(format!(
                    "SELECT {out_alias}.rid, {leaf_alias}.value AS node, \
                     {out_alias}.{ident} || {edge} AS {ident}, {out_alias}.{lu}\n  \
                     FROM {out_alias}{joins}"
                ));
            }
        }
    }
    if !pg_lateral_branches.is_empty() {
        let unioned = pg_lateral_branches.join("\n    UNION ALL\n    ");
        step_branches.push(format!(
            "SELECT {out_alias}.rid, _step.value AS node, \
             {out_alias}.{ident} || _step.edge AS {ident}, {out_alias}.{lu}\n  \
             FROM {out_alias}, LATERAL ({unioned}) AS _step(value, edge)"
        ));
    }

    // PG's `WITH RECURSIVE` requires exactly one `UNION ALL` separating
    // the non-recursive term from the recursive term. Wrap each side in
    // parens so multiple seed/step paths stay on the correct side of the
    // split. SQLite is permissive, so the flat `UNION ALL` form is fine.
    let cte_body = if encoding == KeyEncoding::Postgres
        && (seed_branches.len() > 1 || step_branches.len() > 1)
    {
        let parenthesize = |mut branches: Vec<String>| match branches.len() {
            0 => String::new(),
            1 => branches.remove(0),
            _ => format!("({})", branches.join("\n  UNION ALL\n  ")),
        };
        let seeds = parenthesize(seed_branches);
        let steps = parenthesize(step_branches);
        if steps.is_empty() {
            seeds
        } else {
            format!("{seeds}\n  UNION ALL\n  {steps}")
        }
    } else {
        let mut all = seed_branches;
        all.extend(step_branches);
        all.join("\n  UNION ALL\n  ")
    };

    let (projections, columns, column_decodes) = project_columns(project_cols, dialect)?;

    // Resource rejoin: the compile-time sidecar decides; the expression
    // walk is only a defensive backstop.
    let reads_resource = |expr: &SqlExpr| column_refers_to_resource(expr, dialect);
    let needs_resource_join = *needs_resource_row
        || project_cols.iter().any(|c| reads_resource(&c.expr))
        || extra_unnests.iter().any(|layer| {
            matches!(layer, PlanNode::LateralUnnest { source, on_filter, .. }
                if reads_resource(source) || on_filter.as_ref().is_some_and(reads_resource))
        });

    // Repeat-scope `%rowIndex` reads a rank computed over the CTE rows, before
    // the joins below — also from a post-repeat iteration's criterion (an
    // unnest's ON filter or an indexed selection's membership filter).
    let needs_rank = project_cols
        .iter()
        .any(|c| uses_repeat_row_index(&c.expr, out_alias))
        || extra_unnests.iter().any(|layer| {
            matches!(layer, PlanNode::LateralUnnest { on_filter: Some(filter), .. }
                if uses_repeat_row_index(filter, out_alias))
        })
        || membership_filters
            .iter()
            .any(|filter| uses_repeat_row_index(filter, out_alias));
    let mut from_clause = if needs_rank {
        format!(
            "(SELECT {out_alias}.*, ROW_NUMBER() OVER (PARTITION BY {out_alias}.rid \
             ORDER BY {}) - 1 AS {REPEAT_ROW_INDEX_COL} FROM {out_alias}) AS {out_alias}",
            encoding.identity_key(&format!("{out_alias}.{ident}"))
        )
    } else {
        out_alias.to_string()
    };
    if needs_resource_join {
        // The rejoin is a resource scan too: it carries the runtime
        // predicates alongside the tenant predicate.
        from_clause.push_str(&format!(
            " JOIN {} r ON r.id = {out_alias}.rid AND {tenant_pred}",
            scan.table
        ));
        for p in predicates.fragments() {
            from_clause.push_str("\n  AND ");
            from_clause.push_str(p);
        }
        // One rejoined row per traversal node: an expanded scan.
        if let Some(document) = dialect.resource_document_lateral(ScanFanOut::Expanded) {
            from_clause.push('\n');
            from_clause.push_str(document);
        }
    }

    // Append the forEach unnests stacked above the recurse, outer-to-inner
    // (collected from the projection downward, so iterate in reverse). Each
    // exposes its zero-based occurrence ordinal for the ordering contract and
    // its own forEach-scope `%rowIndex`.
    let mut post_repeat_ordinals: Vec<String> = Vec::new();
    for layer in extra_unnests.iter().rev() {
        let PlanNode::LateralUnnest {
            source,
            out_alias: alias,
            left_join,
            on_filter,
            ..
        } = layer
        else {
            continue;
        };
        let join_kw = if *left_join { "LEFT JOIN" } else { "JOIN" };
        let extra_on = if let Some(filter) = on_filter {
            let mut ctx = ExprCtx::new(dialect);
            Some(lower_expr(filter, &mut ctx)?)
        } else {
            None
        };
        match encoding {
            KeyEncoding::Sqlite => {
                let source_sql = emit_sqlite_unnest_source(source);
                let on = match &extra_on {
                    Some(f) => format!("1=1 AND {f}"),
                    None => "1=1".to_string(),
                };
                from_clause.push_str(&format!("\n{join_kw} {source_sql} {alias} ON {on}"));
                post_repeat_ordinals
                    .push(occurrence_ordinal(&format!("{alias}.rowid"), *left_join));
            }
            KeyEncoding::Postgres => {
                let source = expansion_source(source, dialect);
                let unnest = dialect.unnest_array(&emit_pg_unnest_source(&source));
                let on = match &extra_on {
                    Some(f) => format!("TRUE AND {f}"),
                    None => "TRUE".to_string(),
                };
                from_clause.push_str(&format!(
                    "\n{join_kw} {lateral}{unnest} WITH ORDINALITY AS {alias}(value, ordinality) \
                     ON {on}"
                ));
                post_repeat_ordinals.push(occurrence_ordinal(
                    &format!("{alias}.ordinality - 1"),
                    *left_join,
                ));
            }
        }
    }

    // Membership filters (FHIRPath three-valued boundary, as `walk_body`'s
    // `Filter`) over the joined rows.
    let mut conjuncts: Vec<String> = Vec::with_capacity(membership_filters.len());
    for predicate in membership_filters.iter().rev() {
        let mut ctx = ExprCtx::new(dialect);
        conjuncts.push(dialect.truthy_predicate(&lower_expr(predicate, &mut ctx)?));
    }

    let ordinal_refs: Vec<&str> = post_repeat_ordinals.iter().map(String::as_str).collect();
    let traversal_identity =
        encoding.close_traversal(&format!("{out_alias}.{ident}"), &ordinal_refs);
    Ok((
        SelectBody {
            recursive_cte: Some(format!(
                "{out_alias}(rid, node, {ident}, {lu}) AS (\n  {cte_body}\n)"
            )),
            projections,
            from: from_clause,
            conjuncts,
            order: FinalOrder::Traversal(encoding),
            resource_key: Some(ResourceKeySql {
                last_updated: format!("{out_alias}.{lu}"),
                id: format!("{out_alias}.rid"),
            }),
            occurrences: Vec::new(),
            traversal_identity: Some(traversal_identity),
        },
        columns,
        column_decodes,
    ))
}

/// True when `expr` or any sub-expression satisfies `pred`. Subquery-valued
/// variants the SQL emitter cannot lower (`JsonAgg`, `Scalar`, …) are leaves.
fn expr_any(expr: &SqlExpr, pred: &dyn Fn(&SqlExpr) -> bool) -> bool {
    if pred(expr) {
        return true;
    }
    match expr {
        SqlExpr::Cast { inner, .. }
        | SqlExpr::UnaryOp { inner, .. }
        | SqlExpr::AsJson(inner)
        | SqlExpr::Alias { inner, .. } => expr_any(inner, pred),
        SqlExpr::BinOp { lhs, rhs, .. } => expr_any(lhs, pred) || expr_any(rhs, pred),
        SqlExpr::Case { arms, else_ } => {
            arms.iter()
                .any(|(c, v)| expr_any(c, pred) || expr_any(v, pred))
                || else_.as_deref().is_some_and(|e| expr_any(e, pred))
        }
        SqlExpr::Coalesce(parts) => parts.iter().any(|p| expr_any(p, pred)),
        SqlExpr::NullIf(a, b) => expr_any(a, pred) || expr_any(b, pred),
        SqlExpr::ReferenceKey { reference, .. } => expr_any(reference, pred),
        SqlExpr::Boundary { source, .. } => expr_any(source, pred),
        SqlExpr::WhereExists {
            focus, predicate, ..
        } => expr_any(focus, pred) || expr_any(predicate, pred),
        SqlExpr::WhereScalar {
            focus,
            predicate,
            projection,
            ..
        } => expr_any(focus, pred) || expr_any(predicate, pred) || expr_any(projection, pred),
        SqlExpr::JoinAggregate { outer_focus, .. } => expr_any(outer_focus, pred),
        SqlExpr::ScalarFromChain {
            projection,
            selection_filter,
            ..
        } => {
            expr_any(projection, pred)
                || selection_filter
                    .as_deref()
                    .is_some_and(|crit| expr_any(crit, pred))
        }
        _ => false,
    }
}

/// `source` re-rooted at [`Dialect::expansion_document`] when it navigates
/// straight off [`Dialect::resource_document`]; any other source unchanged.
/// Only the root's form changes, not its value (see the module docs on
/// `Memoize`). Non-path sources (`where(…)` chains lowered to scalars) keep
/// their roots.
fn expansion_source<'a>(source: &'a SqlExpr, dialect: &dyn Dialect) -> Cow<'a, SqlExpr> {
    match source {
        SqlExpr::JsonPath { root, path } if root == dialect.resource_document() => {
            Cow::Owned(SqlExpr::JsonPath {
                root: dialect.expansion_document().to_string(),
                path: path.clone(),
            })
        }
        _ => Cow::Borrowed(source),
    }
}

/// Defensive backstop for [`compose_recurse_select`]'s resource rejoin: true
/// when `expr` (or any sub-expression) navigates off the resource document —
/// the dialect's [`Dialect::resource_document`] or the scanned `r.data`
/// column itself. The rejoin decision itself comes from the compiler's
/// resource-dependency sidecar ([`PlanNode::Recurse::needs_resource_row`]);
/// pre-rendered `ScalarFromChain` chains are opaque here.
fn column_refers_to_resource(expr: &SqlExpr, dialect: &dyn Dialect) -> bool {
    expr_any(expr, &|e| match e {
        SqlExpr::JsonPath { root, .. } | SqlExpr::CollectionAgg { root, .. } => {
            root.starts_with(dialect.resource_document()) || root.starts_with(SCANNED_DOCUMENT)
        }
        _ => false,
    })
}

/// True when `expr` reads the repeat-scope `%rowIndex` of recursion `alias`.
fn uses_repeat_row_index(expr: &SqlExpr, alias: &str) -> bool {
    expr_any(
        expr,
        &|e| matches!(e, SqlExpr::RowIndex(RowIndexScope::Repeat(a)) if a == alias),
    )
}

/// Compose a `UNION ALL` query from structured branch bodies. Every branch
/// (including a recursive one) scans resources with the same runtime
/// predicates and placeholders and projects the same hidden ordering tail:
///
/// `k1` resource timestamp, `k2` resource id, `k3..` one ordinal per common
/// enclosing occurrence, then the flattened leaf-branch number and the
/// branch-local occurrence identity. The wrapped union is ordered by its first
/// visible column (explicit NULL placement), then the tail.
fn compose_union(
    branches: &[PlanNode],
    dialect: &dyn Dialect,
    predicates: &ResourcePredicates,
) -> Result<ComposedQuery, SofError> {
    if branches.is_empty() {
        return Err(SofError::InvalidViewDefinition(
            "unionAll branches list is empty".to_string(),
        ));
    }

    // A nested union contributes its leaves directly: branch numbers count
    // flattened leaves.
    let mut leaves: Vec<&PlanNode> = Vec::new();
    collect_union_leaves(branches, &mut leaves);

    let mut bodies: Vec<SelectBody> = Vec::with_capacity(leaves.len());
    let mut columns: Option<Vec<String>> = None;
    let mut column_decodes: Vec<ColumnDecode> = Vec::new();

    for leaf in leaves {
        let (body, leaf_columns, leaf_decodes) = compose_leaf(leaf, dialect, predicates)?;
        match &columns {
            None => {
                columns = Some(leaf_columns);
                column_decodes = leaf_decodes.clone();
            }
            Some(expected) if *expected != leaf_columns => {
                return Err(SofError::Uncompilable {
                    reason: format!(
                        "unionAll branches produce different column schemas: {:?} vs {:?}",
                        expected, leaf_columns
                    ),
                });
            }
            _ => {
                // Same column names: reconcile decode modes by position.
                for (acc, d) in column_decodes.iter_mut().zip(&leaf_decodes) {
                    *acc = acc.merge(*d);
                }
            }
        }
        bodies.push(body);
    }

    let encoding = KeyEncoding::of(dialect);
    let shared = common_occurrence_prefix(&bodies);
    let mut union_branches: Vec<UnionBranch> = Vec::with_capacity(bodies.len());
    for (index, body) in bodies.into_iter().enumerate() {
        let key = body.resource_key.clone().ok_or_else(|| {
            SofError::Backend("unionAll branch has no resource ordering key".to_string())
        })?;
        let mut hidden = vec![key.last_updated, key.id];
        hidden.extend(body.occurrences[..shared].iter().map(|o| o.ordinal.clone()));
        hidden.push(encoding.branch_number(index));
        let identity = match &body.traversal_identity {
            Some(identity) => identity.clone(),
            None => {
                let local: Vec<&str> = body.occurrences[shared..]
                    .iter()
                    .map(|o| o.ordinal.as_str())
                    .collect();
                encoding.identity(&local)
            }
        };
        hidden.push(identity);
        union_branches.push(UnionBranch { body, hidden });
    }

    let width = union_branches.first().map_or(0, |b| b.hidden.len());
    let mut order = vec![encoding.primary_key(&format!("{UNION_ALIAS}.c1"))];
    for k in 1..width {
        order.push(format!("{UNION_ALIAS}.k{k}"));
    }
    order.push(encoding.identity_key(&format!("{UNION_ALIAS}.k{width}")));

    Ok(ComposedQuery {
        shape: QueryShape::UnionAll(UnionQuery {
            branches: union_branches,
            order,
        }),
        columns: columns.unwrap_or_default(),
        column_decodes,
    })
}

/// Flattens nested `Union` nodes into their leaf plans, in branch order.
fn collect_union_leaves<'a>(branches: &'a [PlanNode], leaves: &mut Vec<&'a PlanNode>) {
    for branch in branches {
        match branch {
            PlanNode::Union(inner) => collect_union_leaves(inner, leaves),
            leaf => leaves.push(leaf),
        }
    }
}

/// Number of leading occurrences every branch shares (same iteration alias):
/// the union's common enclosing iterations. A recursive branch lists none
/// (its post-repeat ordinals live in its traversal identity), so it leaves no
/// shared prefix (the compiler rejects `repeat:` under a shared `forEach`).
fn common_occurrence_prefix(bodies: &[SelectBody]) -> usize {
    let Some((first, rest)) = bodies.split_first() else {
        return 0;
    };
    let mut shared = first.occurrences.len();
    for body in rest {
        shared = shared.min(
            first
                .occurrences
                .iter()
                .zip(&body.occurrences)
                .take_while(|(a, b)| a.alias == b.alias)
                .count(),
        );
    }
    shared
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
    /// Row-producing expansion occurrences in scope, in IR chain order.
    occurrences: Vec<Occurrence>,
}

#[derive(Debug)]
struct ScanInfo {
    table: &'static str,
}

#[derive(Debug)]
struct JoinClause {
    sql: String,
}

impl Frame {
    fn new() -> Self {
        Self {
            scan: None,
            joins: Vec::new(),
            predicates: Vec::new(),
            occurrences: Vec::new(),
        }
    }
}

/// Zero-based occurrence ordinal for an iteration alias. A `forEachOrNull`
/// (LEFT JOIN) miss row has no element, so its ordinal is `-1`, distinct from
/// a real first occurrence `0`.
fn occurrence_ordinal(position: &str, left_join: bool) -> String {
    if left_join {
        format!("COALESCE({position}, -1)")
    } else if position.contains(' ') {
        format!("({position})")
    } else {
        position.to_string()
    }
}

/// Walks `body` (the sub-tree below the top `Project`), pushing pieces into
/// `frame` as it goes.
fn walk_body(node: &PlanNode, dialect: &dyn Dialect, frame: &mut Frame) -> Result<(), SofError> {
    match node {
        PlanNode::Scan { alias, .. } => {
            if alias != "r" {
                return Err(SofError::Uncompilable {
                    reason: format!("Scan alias must be 'r' in current emitter (got '{alias}')"),
                });
            }
            frame.scan = Some(ScanInfo { table: "resources" });
            Ok(())
        }
        PlanNode::Filter { parent, predicate } => {
            walk_body(parent, dialect, frame)?;
            let mut ctx = ExprCtx::new(dialect);
            let pred_sql = lower_expr(predicate, &mut ctx)?;
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
            let join_kw = if *left_join { "LEFT JOIN" } else { "JOIN" };
            let lateral = dialect.lateral_keyword();
            // Lower the optional ON-clause filter (used by `forEach` paths
            // that contain a trailing `where(crit)`).
            let extra_on = if let Some(filter) = on_filter {
                let mut ctx = ExprCtx::new(dialect);
                Some(lower_expr(filter, &mut ctx)?)
            } else {
                None
            };
            let join_sql = if lateral.is_empty() {
                // SQLite — `json_each(<root>, '$.path')` two-arg form when the
                // source is a simple JSON path off the resource document;
                // falls back to `json_each(<sql_expr>)` for anything richer.
                let source_sql = emit_sqlite_unnest_source(source);
                let on = match &extra_on {
                    Some(f) => format!("1=1 AND {f}"),
                    None => "1=1".to_string(),
                };
                if let Some(idx) = flat_index {
                    // `forEach: "<chain>[N]"` — FHIRPath indexes the
                    // FLATTENED collection. SQLite has no correlated `FROM`
                    // subqueries, so the element is picked in a scalar
                    // subquery feeding a one-element `json_each` (key 0, the
                    // singleton `%rowIndex`); an absent pick iterates
                    // nothing. Prior joins (sibling/outer iterations) stay
                    // in the outer FROM with their occurrences.
                    let pick = flat_index_pick(source, out_alias, &on, *idx, dialect);
                    let t = out_alias;
                    format!(
                        "{join_kw} json_each((SELECT json_array(CASE {t}.type \
                         WHEN 'object' THEN json({t}.value) WHEN 'array' THEN json({t}.value) \
                         WHEN 'true' THEN json('true') WHEN 'false' THEN json('false') \
                         ELSE {t}.value END) FROM {pick})) {out_alias} ON 1=1"
                    )
                } else {
                    // `json_each.rowid` is the zero-based element position
                    // for array, object and primitive sources alike, reset
                    // per invocation (the bundled SQLite's `jsonEachFilter`
                    // resets the counter; `key` is NULL for primitives).
                    frame.occurrences.push(Occurrence {
                        alias: out_alias.clone(),
                        ordinal: occurrence_ordinal(&format!("{out_alias}.rowid"), *left_join),
                    });
                    format!("{join_kw} {source_sql} {out_alias} ON {on}")
                }
            } else {
                // PostgreSQL — `jsonb_array_elements(<json_value>)` over the
                // JSON-valued navigation (note: must use `->`, not `->>`).
                let source_sql = emit_pg_unnest_source(&expansion_source(source, dialect));
                let unnest = dialect.unnest_array(&source_sql);
                let on = match &extra_on {
                    Some(f) => format!("TRUE AND {f}"),
                    None => "TRUE".to_string(),
                };
                if let Some(idx) = flat_index {
                    // `forEach: "<chain>[N]"` over the flattened chain, picked
                    // in element order; at most one row, whose constant
                    // ordinality makes its `%rowIndex` 0.
                    let pick = flat_index_pick(source, out_alias, &on, *idx, dialect);
                    format!(
                        "{join_kw} LATERAL (SELECT {out_alias}.value, 1::bigint FROM {pick}) \
                         AS {out_alias}(value, ordinality) ON TRUE"
                    )
                } else {
                    // `WITH ORDINALITY` exposes a 1-based `ordinality` column so
                    // `%rowIndex` (see `lower_row_index`) can read the element's
                    // position, and the ordering contract its zero-based
                    // occurrence ordinal.
                    frame.occurrences.push(Occurrence {
                        alias: out_alias.clone(),
                        ordinal: occurrence_ordinal(
                            &format!("{out_alias}.ordinality - 1"),
                            *left_join,
                        ),
                    });
                    format!(
                        "{join_kw} {lateral}{unnest} WITH ORDINALITY AS {out_alias}(value, ordinality) ON {on}"
                    )
                }
            };
            frame.joins.push(JoinClause { sql: join_sql });
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

/// `<chain> WHERE <on> ORDER BY <element order> LIMIT 1 OFFSET <idx>` — the
/// direct-IR `flat_index` pick: the unnest source flattened through every
/// `Field` step (as the MongoDB lowering flattens it), its innermost row
/// bound to `out_alias` so the pre-lowered `on` filter (`<out_alias>.value`)
/// applies to the candidates before indexing. A non-path source is one
/// unnest of the computed value.
fn flat_index_pick(
    source: &SqlExpr,
    out_alias: &str,
    on: &str,
    idx: i64,
    dialect: &dyn Dialect,
) -> String {
    let chain = match source {
        SqlExpr::JsonPath { root, path } if !path.is_empty() => {
            let segments = path.field_segments();
            let last = segments.len() - 1;
            let aliased: Vec<(String, JsonPath)> = segments
                .into_iter()
                .enumerate()
                .map(|(i, seg)| {
                    let alias = if i == last {
                        out_alias.to_string()
                    } else {
                        format!("{out_alias}_f{i}")
                    };
                    (alias, seg)
                })
                .collect();
            flattened_chain_sql(root, &aliased, dialect)
        }
        _ if dialect.lateral_keyword().is_empty() => FlatChain {
            from_sql: format!("{} {out_alias}", emit_sqlite_unnest_source(source)),
            order_sql: format!("{out_alias}.rowid"),
        },
        _ => FlatChain {
            from_sql: format!(
                "{} WITH ORDINALITY AS {out_alias}(value, ordinality)",
                dialect.unnest_array(&emit_pg_unnest_source(source))
            ),
            order_sql: format!("{out_alias}.ordinality"),
        },
    };
    format!(
        "{} WHERE {on} ORDER BY {} LIMIT 1 OFFSET {idx}",
        chain.from_sql, chain.order_sql
    )
}

// ============================================================================
// Expression lowering
// ============================================================================

/// Context threaded through [`lower_expr`]. Expression lowering allocates no
/// bound parameters: constants arrive as pre-allocated [`SqlExpr::Param`]
/// slots and runtime filters as [`ResourcePredicates`].
struct ExprCtx<'a> {
    dialect: &'a dyn Dialect,
}

impl<'a> ExprCtx<'a> {
    fn new(dialect: &'a dyn Dialect) -> Self {
        Self { dialect }
    }
}

/// Lowers a `%rowIndex` reference to dialect-specific SQL (SQLite and PostgreSQL).
///
/// `COALESCE(.., 0)` on the `forEach` case covers the `forEachOrNull` empty
/// case: a LEFT JOIN miss yields a NULL key/ordinality, which the spec maps to a
/// null row with `%rowIndex` = 0. A direct-IR `flat_index` unnest exposes key
/// 0 (SQLite) / ordinality 1 (PostgreSQL) for its single row. An indexed
/// `forEach` lowered to [`SqlExpr::ScalarFromChain`] compiles its `%rowIndex`
/// as [`RowIndexScope::Top`] instead (see `CompileEnv::indexed_focus`).
fn lower_row_index(scope: &RowIndexScope, dialect: &dyn Dialect) -> Result<String, SofError> {
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
        // Both dialects: the recursive select ranks its CTE rows by their
        // traversal identity per resource (pre-order, before any post-repeat
        // expansion) — see `compose_recurse_select`.
        RowIndexScope::Repeat(alias) => format!("{alias}.{REPEAT_ROW_INDEX_COL}"),
    })
}

fn lower_expr(expr: &SqlExpr, ctx: &mut ExprCtx<'_>) -> Result<String, SofError> {
    match expr {
        SqlExpr::Lit(v) => Ok(lower_lit(v, ctx.dialect)),
        SqlExpr::JsonPath { root, path } => Ok(lower_json_path(root, path, ctx.dialect)),
        SqlExpr::Param(n) => Ok(ctx.dialect.placeholder(*n)),
        SqlExpr::ColRef(name) => Ok(name.clone()),
        SqlExpr::RowIndex(scope) => lower_row_index(scope, ctx.dialect),
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
            order_sql,
            value_alias,
            projection,
            offset,
            empty_context,
            selection_filter,
        } => {
            let proj_sql = lower_expr(projection, ctx)?;
            let pick = format!("{chain_sql} ORDER BY {order_sql} LIMIT 1 OFFSET {offset}");
            // The trailing `where(crit)` tests the selected occurrence AFTER
            // indexing, so it reads the picked row (re-exposed under
            // `<value_alias>`), never filters the chain before `OFFSET`. It
            // is lowered like the ordinary `forEach` path's `ON` filter.
            let crit_sql = selection_filter
                .as_deref()
                .map(|crit| lower_expr(crit, ctx))
                .transpose()?;
            let selected =
                format!("(SELECT {value_alias}.value AS value FROM {pick}) AS {value_alias}");
            if *empty_context {
                // One context row, LEFT JOINed to the selected occurrence: an
                // absent (or rejected) selection evaluates the projection
                // over a NULL `<value_alias>.value` — the empty iteration
                // context, as a `forEachOrNull` LEFT JOIN miss does — instead
                // of the empty scalar subquery's NULL.
                let always = if ctx.dialect.lateral_keyword().is_empty() {
                    "1=1"
                } else {
                    "TRUE"
                };
                let on = match &crit_sql {
                    Some(crit) => format!("{always} AND {crit}"),
                    None => always.to_string(),
                };
                Ok(format!(
                    "(SELECT {proj_sql} FROM (SELECT 1 AS one) AS {value_alias}_ctx \
                     LEFT JOIN {selected} ON {on})"
                ))
            } else if let Some(crit) = crit_sql {
                // A rejected selection yields no row: NULL value, and no
                // membership for `forEach`.
                Ok(format!("(SELECT {proj_sql} FROM {selected} WHERE {crit})"))
            } else {
                Ok(format!("(SELECT {proj_sql} FROM {pick})"))
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
                    ctx.dialect.json_agg("v")
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
                    format!("{} ca0", emit_sqlite_unnest_source(&src))
                } else {
                    format!(
                        "{}{} AS ca0(value)",
                        lateral,
                        ctx.dialect.unnest_array(&emit_pg_unnest_source(&src))
                    )
                };
                let agg = ctx.dialect.json_agg("ca0.value");
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
                let mut path = String::from("$");
                for s in &leaf_path_segs {
                    path.push('.');
                    path.push_str(s);
                }
                format!("json_extract(ca0.value, '{path}')")
            } else {
                let segs = leaf_path_segs.to_vec();
                ctx.dialect.json_path("ca0.value", &segs)
            };
            let outer_from = if lateral.is_empty() {
                format!("{} ca0", emit_sqlite_unnest_source(&outer_src))
            } else {
                format!(
                    "{}{} AS ca0(value)",
                    lateral,
                    ctx.dialect.unnest_array(&emit_pg_unnest_source(&outer_src))
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
                let mut leaf_path_str = String::from("$");
                for s in &leaf_path_segs {
                    leaf_path_str.push('.');
                    leaf_path_str.push_str(s);
                }
                let type_check = format!("json_type(ca0.value, '{leaf_path_str}')");
                let guarded = format!(
                    "json_each(CASE WHEN {type_check} = 'array' \
                     THEN {leaf_value_sql} \
                     ELSE json_array({leaf_value_sql}) END)"
                );
                let agg = ctx.dialect.json_agg("ca1.value");
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
                let agg = ctx.dialect.json_agg("ca1.value");
                Ok(format!(
                    "(SELECT {agg} FROM {outer_from} \
                     JOIN LATERAL {guarded} AS ca1(value) ON TRUE \
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
            // values. The separator is inlined as a SQL string literal —
            // the FHIRPath parser has already validated it as a string
            // literal so escaping is a simple `''`-doubling.
            let sep_lit = format!("'{}'", separator.replace('\'', "''"));
            let unnest_outer = if ctx.dialect.lateral_keyword().is_empty() {
                let src = emit_sqlite_unnest_source(outer_focus);
                format!("FROM {src} {outer_alias}")
            } else {
                let src = emit_pg_unnest_source(outer_focus);
                format!(
                    "FROM {}{} AS {outer_alias}(value)",
                    ctx.dialect.lateral_keyword(),
                    ctx.dialect.unnest_array(&src)
                )
            };
            let inner_src = SqlExpr::JsonPath {
                root: format!("{outer_alias}.value"),
                path: super::ir::JsonPath(vec![PathStep::Field(inner_field.clone())]),
            };
            let unnest_inner = if ctx.dialect.lateral_keyword().is_empty() {
                let src = emit_sqlite_unnest_source(&inner_src);
                format!(", {src} {inner_alias}")
            } else {
                let src = emit_pg_unnest_source(&inner_src);
                format!(
                    " JOIN {}{} AS {inner_alias}(value) ON TRUE",
                    ctx.dialect.lateral_keyword(),
                    ctx.dialect.unnest_array(&src)
                )
            };
            let value_text = if ctx.dialect.lateral_keyword().is_empty() {
                format!("{inner_alias}.value")
            } else {
                format!("({inner_alias}.value #>> '{{}}')")
            };
            let agg = ctx.dialect.string_agg(&value_text, &sep_lit);
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
                let src = emit_sqlite_unnest_source(focus);
                format!("FROM {src} {iter_alias}")
            } else {
                let src = emit_pg_unnest_source(focus);
                format!(
                    "FROM {}{} AS {iter_alias}(value)",
                    ctx.dialect.lateral_keyword(),
                    ctx.dialect.unnest_array(&src)
                )
            };
            let pred_sql = lower_expr(predicate, ctx)?;
            let proj_sql = lower_expr(projection, ctx)?;
            Ok(format!(
                "(SELECT {proj_sql} {unnest} WHERE {pred_sql} LIMIT 1)"
            ))
        }
        SqlExpr::WhereExists {
            focus,
            iter_alias,
            predicate,
            negate,
        } => {
            let unnest = if ctx.dialect.lateral_keyword().is_empty() {
                let src = emit_sqlite_unnest_source(focus);
                format!("FROM {src} {iter_alias}")
            } else {
                let src = emit_pg_unnest_source(focus);
                format!(
                    "FROM {}{} AS {iter_alias}(value)",
                    ctx.dialect.lateral_keyword(),
                    ctx.dialect.unnest_array(&src)
                )
            };
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
                    let p1 = format!("{ty}/%").replace('\'', "''");
                    let p2 = format!("%/{ty}/%").replace('\'', "''");
                    Ok(format!(
                        "CASE WHEN {ref_sql} LIKE '{p1}' OR {ref_sql} LIKE '{p2}' \
                         THEN {last} ELSE NULL END"
                    ))
                }
            }
        }
    }
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

fn lower_lit(v: &LitValue, dialect: &dyn Dialect) -> String {
    match v {
        LitValue::Null => "NULL".to_string(),
        LitValue::Bool(true) => dialect.bool_true().to_string(),
        LitValue::Bool(false) => dialect.bool_false().to_string(),
        LitValue::Int(n) => n.to_string(),
        LitValue::Decimal(s) => s.clone(),
        // Compile-time-constant idents only (e.g. a polymorphic-field key).
        // User strings must always go through `SqlExpr::Param`.
        LitValue::Str(s) => format!("'{}'", s.replace('\'', "''")),
    }
}

fn lower_json_path(root: &str, path: &JsonPath, dialect: &dyn Dialect) -> String {
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
        return dialect.json_path_text(root, &lifted);
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
            dialect.json_path_text(root, &array_refs),
            dialect.json_path_text(root, &segs)
        );
    }

    dialect.json_path_text(root, &segs)
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

/// A flattened navigation chain rendered as comma-joined `FROM` items — one
/// unnest per `Field` segment, each over the previous segment's row — plus
/// the `ORDER BY` list enumerating its rows in element order (every
/// segment's zero-based element ordinal, outer to inner: SQLite
/// `json_each.rowid`, PostgreSQL `WITH ORDINALITY`).
pub(super) struct FlatChain {
    /// Comma-joined `FROM` items; the last one binds the innermost alias.
    pub(super) from_sql: String,
    /// `ORDER BY` list selecting occurrences in element order.
    pub(super) order_sql: String,
}

/// Renders the flattened chain over `segments` (each `(alias, path)`, the
/// first navigated off `root`). Each segment's unnest source is wrapped in a
/// dialect-appropriate type guard so non-array intermediates (FHIR
/// singletons like `Patient.contact.name`) produce one row instead of
/// erroring, and missing ones produce none. SQLite has no correlated
/// subqueries in `FROM`, so callers place the chain inside a scalar subquery
/// (or a table-valued function argument); PostgreSQL uses the same shape.
pub(super) fn flattened_chain_sql(
    root: &str,
    segments: &[(String, JsonPath)],
    dialect: &dyn Dialect,
) -> FlatChain {
    let mut from_parts: Vec<String> = Vec::with_capacity(segments.len());
    let mut ordinals: Vec<String> = Vec::with_capacity(segments.len());
    let mut prev = root.to_string();
    let is_sqlite = dialect.lateral_keyword().is_empty();
    for (alias, seg) in segments {
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
        if is_sqlite {
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
            let unnest_sql = if prev == SCANNED_DOCUMENT && !path_str.contains('[') {
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
            };
            from_parts.push(format!("{unnest_sql} {alias}"));
            // `json_each.rowid` is the zero-based element position, reset per
            // invocation (see the ordering contract).
            ordinals.push(format!("{alias}.rowid"));
        } else {
            // PostgreSQL — `jsonb_array_elements` over a `jsonb_typeof`
            // type-guard so object intermediates (FHIR singletons) get
            // wrapped in a single-element array. Numeric segments are
            // path-array integers; field segments are path-array strings.
            //
            // `prev` may be either a jsonb expression (e.g. `rdoc.doc` or
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
            from_parts.push(format!(
                "jsonb_array_elements(CASE WHEN jsonb_typeof({nav}) = 'array' THEN {nav} \
                 WHEN jsonb_typeof({nav}) IS NOT NULL THEN jsonb_build_array({nav}) \
                 ELSE '[]'::jsonb END) WITH ORDINALITY AS {alias}(value, ordinality)"
            ));
            ordinals.push(format!("{alias}.ordinality"));
        }
        prev = format!("{alias}.value");
    }
    FlatChain {
        from_sql: from_parts.join(", "),
        order_sql: ordinals.join(", "),
    }
}

/// Emits the SQLite `json_each(...)` source clause for a lateral unnest.
///
/// `r.data`-rooted simple paths use the two-arg `json_each(r.data, '$.path')`
/// shortcut. Intermediate paths (off `<alias>.value`) wrap with a type guard:
/// arrays iterate; non-array singletons (FHIR singleton elements like
/// `contact.name`) wrap in a single-element array; missing intermediates
/// produce zero rows.
fn emit_sqlite_unnest_source(source: &SqlExpr) -> String {
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
            // indices `[N]`, others are dotted fields.
            let mut path_str = String::from("$");
            for s in &segments {
                if s.chars().all(|c| c.is_ascii_digit()) {
                    path_str.push('[');
                    path_str.push_str(s);
                    path_str.push(']');
                } else {
                    path_str.push('.');
                    path_str.push_str(s);
                }
            }
            // Has the path crossed an explicit index? Indexed paths
            // (`telecom[0]`) always select a single element (an object) and
            // need the type-guard so json_each wraps the singleton in an
            // array rather than iterating its keys. Non-indexed `r.data`
            // paths are typically arrays — keep the cheaper two-arg form
            // for back-compat with existing test assertions.
            let has_index = path.0.iter().any(|s| matches!(s, PathStep::Index(_)));
            if root == SCANNED_DOCUMENT && !has_index {
                return format!("json_each({root}, '{path_str}')");
            }
            let extracted = format!("json_extract({root}, '{path_str}')");
            let type_check = format!("json_type({root}, '{path_str}')");
            // For non-array values, wrap with `json_array(json(<extract>))`
            // — `json(...)` re-parses the extracted text so the wrapped
            // value preserves its JSON shape (otherwise SQLite's `json_array`
            // sees a TEXT argument and JSON-quotes it as a string, which
            // would iterate as one stringified row rather than the original
            // object).
            return format!(
                "json_each(CASE WHEN {type_check} = 'array' THEN {extracted} \
                 WHEN {type_check} IN ('object', 'array') THEN json_array(json({extracted})) \
                 WHEN {type_check} IS NOT NULL THEN json_array({extracted}) \
                 ELSE '[]' END)"
            );
        }
    }
    let mut ctx = ExprCtx::new(&super::dialect::SqliteDialect);
    let computed = lower_expr(source, &mut ctx).unwrap_or_else(|_| "NULL".to_string());
    format!("json_each(coalesce({computed}, '[]'))")
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
fn emit_pg_unnest_source(source: &SqlExpr) -> String {
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
            format!("{root}->'{}'", segments[0])
        } else {
            format!("{root}#>'{{{}}}'", segments.join(","))
        }
    } else {
        // Non-`JsonPath` sources include nested `WhereScalar`/`ScalarFromChain`
        // results (e.g. `extension(url1).extension(url2)`). Their projections
        // are lowered as text via `->>`/`#>>`; an explicit `::jsonb` cast
        // re-parses the JSON text so the surrounding `jsonb_typeof` /
        // `jsonb_array_elements` operators type-check.
        let mut ctx = ExprCtx::new(&super::dialect::PgDialect);
        let inner = lower_expr(source, &mut ctx).unwrap_or_else(|_| "NULL".to_string());
        format!("({inner})::jsonb")
    };
    format!(
        "(CASE WHEN jsonb_typeof({raw}) = 'array' THEN {raw} \
         WHEN jsonb_typeof({raw}) IS NOT NULL THEN jsonb_build_array({raw}) \
         ELSE '[]'::jsonb END)"
    )
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
    use helios_fhir::FhirVersion;
    use serde_json::{Value, json};

    use super::super::compile_view::build_plan;
    use super::super::compiler::CompileTarget;
    use super::super::dialect::{PgDialect, SqliteDialect};
    use super::*;

    fn plan_for(view: &Value, dialect: &dyn Dialect) -> PlanNode {
        let target = if dialect.lateral_keyword().is_empty() {
            CompileTarget::Sqlite
        } else {
            CompileTarget::Postgres
        };
        build_plan(view, dialect, target, FhirVersion::default_enabled())
            .expect("build test plan")
            .0
    }

    fn emit_with(view: &Value, dialect: &dyn Dialect, predicates: &ResourcePredicates) -> String {
        emit_plan_with_predicates(&plan_for(view, dialect), dialect, predicates)
            .expect("emit test plan")
            .sql
    }

    /// `_since` and a Patient filter in the slots following one constant.
    fn pg_predicates() -> ResourcePredicates {
        ResourcePredicates::new(
            4,
            2,
            vec!["r.last_updated >= $4".into(), "(r.id = $5)".into()],
        )
    }

    /// Pieces of a rendered `unionAll` statement: the outer visible
    /// projection items, the `UNION ALL` operands, and the final ORDER BY.
    struct UnionParts<'a> {
        outer: Vec<&'a str>,
        operands: Vec<&'a str>,
        order_by: &'a str,
    }

    fn union_parts(sql: &str) -> UnionParts<'_> {
        let rest = sql
            .strip_prefix("SELECT\n  ")
            .expect("union is wrapped in an outer SELECT");
        let (outer, rest) = rest.split_once("\nFROM (\n").expect("wrapped union");
        let (inner, order_by) = rest
            .split_once("\n) AS u\nORDER BY ")
            .expect("outer ORDER BY over the wrapped union");
        UnionParts {
            outer: outer.split(",\n  ").collect(),
            operands: inner.split("\nUNION ALL\n").collect(),
            order_by,
        }
    }

    fn union_operands(sql: &str) -> Vec<&str> {
        union_parts(sql).operands
    }

    /// The projection items of one rendered (non-recursive) union operand.
    fn operand_items(operand: &str) -> Vec<&str> {
        operand
            .strip_prefix("SELECT\n  ")
            .and_then(|rest| rest.split_once("\nFROM ").map(|(items, _)| items))
            .expect("operand SELECT list")
            .split(",\n  ")
            .collect()
    }

    fn order_by(sql: &str) -> &str {
        sql.rsplit_once("\nORDER BY ")
            .map(|(_, order)| order)
            .expect("final ORDER BY")
    }

    fn view(resource: &str, select: Value) -> Value {
        json!({"resource": resource, "select": select})
    }

    #[test]
    fn test_expanded_selects_order_by_every_occurrence_ordinal_in_chain_order() {
        let cases = [
            (
                "nullable",
                json!([{"forEachOrNull":"name","column":[{"path":"family","name":"family"}]}]),
                "r.last_updated, r.id, COALESCE(fe.rowid, -1)",
                "r.last_updated, r.id, COALESCE(fe.ordinality - 1, -1)",
            ),
            (
                "nullable-where-on",
                json!([{"forEachOrNull":"name.where(use = 'official')",
                    "column":[{"path":"family","name":"family"}]}]),
                "r.last_updated, r.id, COALESCE(fe.rowid, -1)",
                "r.last_updated, r.id, COALESCE(fe.ordinality - 1, -1)",
            ),
            (
                "cartesian",
                json!([
                    {"forEach":"name","column":[{"path":"family","name":"family"}]},
                    {"forEach":"address","column":[{"path":"city","name":"city"}]}
                ]),
                "r.last_updated, r.id, fe.rowid, fe2.rowid",
                "r.last_updated, r.id, (fe.ordinality - 1), (fe2.ordinality - 1)",
            ),
            (
                "nested",
                json!([{"forEach":"name","select":[
                    {"column":[{"path":"family","name":"family"}]},
                    {"forEachOrNull":"given","column":[{"path":"$this","name":"given"}]}
                ]}]),
                "r.last_updated, r.id, fe.rowid, COALESCE(fe2.rowid, -1)",
                "r.last_updated, r.id, (fe.ordinality - 1), COALESCE(fe2.ordinality - 1, -1)",
            ),
            (
                "chained",
                json!([{"forEach":"name.given","column":[{"path":"$this","name":"given"}]}]),
                "r.last_updated, r.id, fe.rowid, fe2.rowid",
                "r.last_updated, r.id, (fe.ordinality - 1), (fe2.ordinality - 1)",
            ),
            (
                "flat",
                json!([{"column":[{"path":"id","name":"id"}]}]),
                "r.last_updated, r.id",
                "r.last_updated, r.id",
            ),
            (
                // Indexed iteration is one scalar row per resource (N4 owns
                // its index semantics); it adds no occurrence key.
                "indexed",
                json!([{"forEach":"name[1]","column":[{"path":"family","name":"family"}]}]),
                "r.last_updated, r.id",
                "r.last_updated, r.id",
            ),
        ];
        for (case, select, sqlite_order, pg_order) in cases {
            let v = view("Patient", select);
            for (dialect, expected) in [
                (&SqliteDialect as &dyn Dialect, sqlite_order),
                (&PgDialect, pg_order),
            ] {
                let sql = emit_with(&v, dialect, &ResourcePredicates::none());
                assert_eq!(
                    order_by(&sql),
                    expected,
                    "{case} ({}): {sql}",
                    dialect.name()
                );
            }
        }
    }

    #[test]
    fn test_union_projects_typed_hidden_tail_under_visible_outer_projection() {
        let v = view(
            "Patient",
            json!([{"unionAll":[
                {"column":[{"path":"'tie'","name":"tie"},{"path":"id","name":"value"}]},
                {"forEachOrNull":"name","column":[{"path":"'tie'","name":"tie"},
                    {"path":"family","name":"value"}]}
            ]}]),
        );
        let tenant = "WHERE r.tenant_id = ?1\n  AND r.resource_type = ?2\n  AND r.is_deleted = 0";
        let emitted = emit_plan(&plan_for(&v, &SqliteDialect), &SqliteDialect).unwrap();
        assert_eq!(emitted.columns, ["tie", "value"]);
        assert_eq!(
            emitted.sql,
            format!(
                "SELECT\n  u.c1 AS \"tie\",\n  u.c2 AS \"value\"\nFROM (\n\
                 SELECT\n  'tie' AS c1,\n  json_extract(r.data, '$.id') AS c2,\n  \
                 r.last_updated AS k1,\n  r.id AS k2,\n  0 AS k3,\n  '' AS k4\n\
                 FROM resources r\n{tenant}\n\
                 UNION ALL\n\
                 SELECT\n  'tie' AS c1,\n  json_extract(fe.value, '$.family') AS c2,\n  \
                 r.last_updated AS k1,\n  r.id AS k2,\n  1 AS k3,\n  \
                 printf('%020d', COALESCE(fe.rowid, -1) + 1) AS k4\n\
                 FROM resources r\nLEFT JOIN json_each(r.data, '$.name') fe ON 1=1\n{tenant}\n\
                 ) AS u\n\
                 ORDER BY u.c1 ASC NULLS FIRST, u.k1, u.k2, u.k3, u.k4 COLLATE BINARY"
            )
        );

        let emitted = emit_plan(&plan_for(&v, &PgDialect), &PgDialect).unwrap();
        assert_eq!(emitted.columns, ["tie", "value"]);
        let parts = union_parts(&emitted.sql);
        assert_eq!(parts.outer, ["u.c1 AS \"tie\"", "u.c2 AS \"value\""]);
        assert_eq!(
            parts.order_by,
            "u.c1 ASC NULLS LAST, u.k1, u.k2, u.k3, u.k4"
        );
        assert_eq!(
            operand_items(parts.operands[0])[2..],
            [
                "r.last_updated AS k1",
                "r.id AS k2",
                "CAST(0 AS bigint) AS k3",
                "ARRAY[]::bigint[] AS k4"
            ]
        );
        assert_eq!(
            operand_items(parts.operands[1])[2..],
            [
                "r.last_updated AS k1",
                "r.id AS k2",
                "CAST(1 AS bigint) AS k3",
                "ARRAY[COALESCE(fe.ordinality - 1, -1)]::bigint[] AS k4"
            ]
        );
    }

    #[test]
    fn test_union_hidden_tails_have_equal_width_across_branch_depths() {
        // A flat branch, a one-level and a two-level expansion: every operand
        // projects the same k1..k4 tail; depth lives inside the identity.
        let v = view(
            "Patient",
            json!([{"unionAll":[
                {"column":[{"path":"id","name":"v"}]},
                {"forEach":"address","column":[{"path":"city","name":"v"}]},
                {"forEach":"name","select":[{"forEach":"given","column":[{"path":"$this","name":"v"}]}]}
            ]}]),
        );
        let expected_identity = [
            ("'' AS k4", "ARRAY[]::bigint[] AS k4"),
            (
                "printf('%020d', fe.rowid + 1) AS k4",
                "ARRAY[(fe.ordinality - 1)]::bigint[] AS k4",
            ),
            (
                "printf('%020d', fe2.rowid + 1) || printf('%020d', fe3.rowid + 1) AS k4",
                "ARRAY[(fe2.ordinality - 1), (fe3.ordinality - 1)]::bigint[] AS k4",
            ),
        ];
        for (dialect, is_sqlite) in [(&SqliteDialect as &dyn Dialect, true), (&PgDialect, false)] {
            let sql = emit_with(&v, dialect, &ResourcePredicates::none());
            let parts = union_parts(&sql);
            assert_eq!(parts.outer, ["u.c1 AS \"v\""], "{sql}");
            assert_eq!(parts.operands.len(), 3, "{sql}");
            for (index, (operand, (sqlite_id, pg_id))) in
                parts.operands.iter().zip(expected_identity).enumerate()
            {
                let items = operand_items(operand);
                assert_eq!(items.len(), 5, "equal-width tail: {operand}");
                assert_eq!(items[0].rsplit_once(" AS ").unwrap().1, "c1");
                let number = if is_sqlite {
                    format!("{index} AS k3")
                } else {
                    format!("CAST({index} AS bigint) AS k3")
                };
                assert_eq!(items[3], number, "{operand}");
                assert_eq!(items[4], if is_sqlite { sqlite_id } else { pg_id });
            }
        }
    }

    #[test]
    fn test_outer_foreach_union_orders_shared_iteration_before_branch_number() {
        let v = view(
            "Patient",
            json!([{"forEach":"name","unionAll":[
                {"column":[{"path":"family","name":"v"}]},
                {"forEach":"given","column":[{"path":"$this","name":"v"}]}
            ]}]),
        );
        let sqlite = emit_with(&v, &SqliteDialect, &ResourcePredicates::none());
        let parts = union_parts(&sqlite);
        assert_eq!(
            parts.order_by,
            "u.c1 ASC NULLS FIRST, u.k1, u.k2, u.k3, u.k4, u.k5 COLLATE BINARY"
        );
        assert_eq!(
            operand_items(parts.operands[0])[1..],
            [
                "r.last_updated AS k1",
                "r.id AS k2",
                "fe.rowid AS k3",
                "0 AS k4",
                "'' AS k5"
            ]
        );
        assert_eq!(
            operand_items(parts.operands[1])[1..],
            [
                "r.last_updated AS k1",
                "r.id AS k2",
                "fe.rowid AS k3",
                "1 AS k4",
                "printf('%020d', fe2.rowid + 1) AS k5"
            ]
        );
        let pg = emit_with(&v, &PgDialect, &ResourcePredicates::none());
        let parts = union_parts(&pg);
        assert_eq!(
            parts.order_by,
            "u.c1 ASC NULLS LAST, u.k1, u.k2, u.k3, u.k4, u.k5"
        );
        assert_eq!(
            operand_items(parts.operands[1])[1..],
            [
                "r.last_updated AS k1",
                "r.id AS k2",
                "(fe.ordinality - 1) AS k3",
                "CAST(1 AS bigint) AS k4",
                "ARRAY[(fe2.ordinality - 1)]::bigint[] AS k5"
            ]
        );
    }

    #[test]
    fn test_union_internal_aliases_cannot_collide_with_user_column_names() {
        // User columns spelled like the internal aliases (and the derived
        // table alias) keep their visible names and positions.
        let v = view(
            "Patient",
            json!([{"unionAll":[
                {"column":[{"path":"id","name":"k1"},{"path":"gender","name":"c1"},
                    {"path":"'x'","name":"u"},{"path":"'y'","name":"_ord"}]},
                {"forEach":"name","column":[{"path":"family","name":"k1"},
                    {"path":"'g'","name":"c1"},{"path":"'x'","name":"u"},
                    {"path":"'y'","name":"_ord"}]}
            ]}]),
        );
        for dialect in [&SqliteDialect as &dyn Dialect, &PgDialect] {
            let emitted = emit_plan(&plan_for(&v, dialect), dialect).unwrap();
            assert_eq!(emitted.columns, ["k1", "c1", "u", "_ord"]);
            let parts = union_parts(&emitted.sql);
            assert_eq!(
                parts.outer,
                [
                    "u.c1 AS \"k1\"",
                    "u.c2 AS \"c1\"",
                    "u.c3 AS \"u\"",
                    "u.c4 AS \"_ord\""
                ]
            );
            for operand in &parts.operands {
                let items = operand_items(operand);
                let aliases: Vec<&str> = items
                    .iter()
                    .map(|item| item.rsplit_once(" AS ").unwrap().1)
                    .collect();
                assert_eq!(aliases, ["c1", "c2", "c3", "c4", "k1", "k2", "k3", "k4"]);
            }
            assert!(
                parts.order_by.starts_with("u.c1 ASC NULLS"),
                "{}",
                emitted.sql
            );
        }
    }

    #[test]
    fn test_union_repeat_branch_carries_resource_key_and_traversal_identity() {
        // #1623 2C replaced the N2 placeholder (`ord_path`, and an empty
        // identity for PostgreSQL multi-path repeats) with the traversal
        // identity every recursive branch now carries.
        let v = view(
            "QuestionnaireResponse",
            json!([{"unionAll":[
                {"repeat":["item"],"column":[{"path":"linkId","name":"v"}]},
                {"repeat":["item","answer.item"],"column":[{"path":"linkId","name":"v"}]},
                {"column":[{"path":"id","name":"v"}]}
            ]}]),
        );
        let sqlite = emit_with(&v, &SqliteDialect, &ResourcePredicates::none());
        let parts = union_parts(&sqlite);
        assert_eq!(parts.operands.len(), 3, "{sqlite}");
        assert!(parts.operands[0].starts_with(
            "SELECT * FROM (WITH RECURSIVE rec_0(rid, node, ident, last_updated) AS ("
        ));
        assert!(parts.operands[0].contains("rec_0.last_updated AS k1,\n  rec_0.rid AS k2,\n  0 AS k3,\n  rec_0.ident AS k4\nFROM rec_0) AS _recurse_0"), "{sqlite}");
        assert!(parts.operands[1].contains("rec_1.last_updated AS k1,\n  rec_1.rid AS k2,\n  1 AS k3,\n  rec_1.ident AS k4\nFROM rec_1) AS _recurse_1"), "{sqlite}");
        assert_eq!(
            operand_items(parts.operands[2])[1..],
            ["r.last_updated AS k1", "r.id AS k2", "2 AS k3", "'' AS k4"]
        );
        assert_eq!(
            parts.order_by,
            "u.c1 ASC NULLS FIRST, u.k1, u.k2, u.k3, u.k4 COLLATE BINARY"
        );

        let pg = emit_with(&v, &PgDialect, &ResourcePredicates::none());
        let parts = union_parts(&pg);
        for (index, operand) in parts.operands[..2].iter().enumerate() {
            assert!(
                operand.starts_with(&format!(
                    "SELECT * FROM (WITH RECURSIVE rec_{index}(rid, node, ident, last_updated) AS ("
                )),
                "{pg}"
            );
            assert!(
                operand.contains(&format!(
                    "rec_{index}.last_updated AS k1,\n  rec_{index}.rid AS k2,\n  \
                     CAST({index} AS bigint) AS k3,\n  rec_{index}.ident AS k4\n\
                     FROM rec_{index}) AS _recurse_{index}"
                )),
                "{pg}"
            );
        }
        assert_eq!(
            operand_items(parts.operands[2])[1..],
            [
                "r.last_updated AS k1",
                "r.id AS k2",
                "CAST(2 AS bigint) AS k3",
                "ARRAY[]::bigint[] AS k4"
            ]
        );
        assert_eq!(
            parts.order_by,
            "u.c1 ASC NULLS LAST, u.k1, u.k2, u.k3, u.k4"
        );
    }

    #[test]
    fn test_emit_without_predicates_keeps_flat_and_expansion_sql() {
        let flat = json!({"resource":"Patient","constant":[{"name":"g","valueString":"male"}],
            "where":[{"path":"gender = %g"}],"select":[{"column":[{"path":"id","name":"id"}]}]});
        let expansion = json!({"resource":"Patient","select":[
            {"column":[{"path":"id","name":"id"}]},
            {"forEachOrNull":"name.where(use = 'official')","column":[
                {"path":"family","name":"family"},
                {"path":"%rowIndex","name":"i","type":"integer"}]}]});
        let cases: [(&Value, &dyn Dialect, &str); 4] = [
            (
                &flat,
                &SqliteDialect,
                "SELECT\n  json_extract(r.data, '$.id') AS \"id\"\nFROM resources r\n\
                 WHERE r.tenant_id = ?1\n  AND r.resource_type = ?2\n  AND r.is_deleted = 0\n  \
                 AND ((json_extract(r.data, '$.gender') = ?3)) IS NOT NULL \
                 AND ((json_extract(r.data, '$.gender') = ?3)) != 0 \
                 AND ((json_extract(r.data, '$.gender') = ?3)) != 'false'\n\
                 ORDER BY r.last_updated, r.id",
            ),
            (
                &flat,
                &PgDialect,
                "SELECT\n  rdoc.doc->>'id' AS \"id\"\nFROM resources r\n\
                 CROSS JOIN LATERAL (SELECT r.data AS doc) AS rdoc\n\
                 WHERE r.tenant_id = $1\n  AND r.resource_type = $2\n  AND r.is_deleted = false\n  \
                 AND ((rdoc.doc->>'gender' = $3))::boolean IS TRUE\n\
                 ORDER BY r.last_updated, r.id",
            ),
            (
                &expansion,
                &SqliteDialect,
                "SELECT\n  json_extract(r.data, '$.id') AS \"id\",\n  \
                 json_extract(fe.value, '$.family') AS \"family\",\n  \
                 CAST(COALESCE(CAST(fe.key AS INTEGER), 0) AS INTEGER) AS \"i\"\n\
                 FROM resources r\n\
                 LEFT JOIN json_each(r.data, '$.name') fe ON 1=1 \
                 AND (json_extract(fe.value, '$.use') = 'official')\n\
                 WHERE r.tenant_id = ?1\n  AND r.resource_type = ?2\n  AND r.is_deleted = 0\n\
                 ORDER BY r.last_updated, r.id, COALESCE(fe.rowid, -1)",
            ),
            (
                &expansion,
                &PgDialect,
                "SELECT\n  rdoc.doc->>'id' AS \"id\",\n  fe.value->>'family' AS \"family\",\n  \
                 ((COALESCE(CAST(fe.ordinality AS INTEGER) - 1, 0))::bigint)::text AS \"i\"\n\
                 FROM resources r\n\
                 CROSS JOIN LATERAL jsonb_extract_path(r.data, VARIADIC '{}'::text[]) AS rdoc(doc)\n\
                 LEFT JOIN LATERAL jsonb_array_elements((CASE \
                 WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'name') = 'array' \
                 THEN COALESCE(rdoc.doc, r.data)->'name' \
                 WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'name') IS NOT NULL \
                 THEN jsonb_build_array(COALESCE(rdoc.doc, r.data)->'name') ELSE '[]'::jsonb END)) \
                 WITH ORDINALITY AS fe(value, ordinality) ON TRUE AND (fe.value->>'use' = 'official')\n\
                 WHERE r.tenant_id = $1\n  AND r.resource_type = $2\n  AND r.is_deleted = false\n\
                 ORDER BY r.last_updated, r.id, COALESCE(fe.ordinality - 1, -1)",
            ),
        ];
        for (view, dialect, expected) in cases {
            let plan = plan_for(view, dialect);
            assert_eq!(emit_plan(&plan, dialect).unwrap().sql, expected);
            assert_eq!(
                emit_plan_with_predicates(&plan, dialect, &ResourcePredicates::none())
                    .unwrap()
                    .sql,
                expected
            );
        }
    }

    #[test]
    fn test_predicates_reuse_slots_in_every_union_branch_and_recursive_seed() {
        let view = json!({"resource":"QuestionnaireResponse",
        "constant":[{"name":"s","valueString":"completed"}],
        "where":[{"path":"status = %s"}],
        "select":[{"unionAll":[
            {"repeat":["item","answer.item"],"column":[{"path":"linkId","name":"v"}]},
            {"forEach":"item","column":[{"path":"linkId","name":"v"}]},
            {"column":[{"path":"id","name":"v"}]}
        ]}]});
        let sql = emit_with(&view, &PgDialect, &pg_predicates());
        let operands = union_operands(&sql);
        assert_eq!(operands.len(), 3, "{sql}");
        // The recursive operand is identified structurally and wrapped.
        assert!(operands[0].starts_with("SELECT * FROM (WITH RECURSIVE rec_0"));
        assert!(operands[0].ends_with(") AS _recurse_0"), "{sql}");
        // Two seeds plus one scan per flat/expanded branch; every scan carries
        // the constant filter and both runtime predicates, in the same slots.
        for (operand, scans) in operands.iter().zip([2, 1, 1]) {
            assert_eq!(operand.matches("FROM resources r").count(), scans);
            assert_eq!(operand.matches("$3").count(), scans, "{operand}");
            assert_eq!(operand.matches("r.last_updated >= $4").count(), scans);
            assert_eq!(operand.matches("(r.id = $5)").count(), scans);
        }
        assert!(!sql.contains("$6"), "{sql}");
        // Each scan's conjuncts run tenant → view where → runtime predicates.
        assert!(operands[2].ends_with(
            "AND ((rdoc.doc->>'status' = $3))::boolean IS TRUE\n  \
             AND r.last_updated >= $4\n  AND (r.id = $5)"
        ));
    }

    #[test]
    fn test_predicates_reach_seed_and_rejoin_but_not_iteration_scopes() {
        let view = json!({"resource":"QuestionnaireResponse","select":[
            {"column":[{"path":"id","name":"id"}]},
            {"repeat":["item"],"select":[
                {"column":[{"path":"linkId","name":"link_id"}]},
                {"forEachOrNull":"answer.where(value.exists())",
                    "column":[{"path":"valueString","name":"answer"}]}]}]});
        let predicates = ResourcePredicates::new(3, 1, vec!["r.last_updated >= ?3".into()]);
        let sql = emit_with(&view, &SqliteDialect, &predicates);
        let (cte, outer) = sql.split_once("\n)\nSELECT").expect("recursive statement");
        // Seed (one repeat path) and the resource rejoin are resource scans.
        assert_eq!(cte.matches("r.last_updated >= ?3").count(), 1, "{sql}");
        let rejoin = outer
            .lines()
            .position(|line| line.contains("JOIN resources r ON r.id = rec_0.rid"))
            .expect("resource rejoin");
        let join_lines: Vec<&str> = outer.lines().collect();
        // The rejoin's ON clause spans the tenant predicate and the runtime
        // predicate; the forEach join that follows keeps only its own filter.
        let foreach = join_lines
            .iter()
            .position(|line| line.starts_with("LEFT JOIN json_each"))
            .expect("forEach join");
        assert!(rejoin < foreach, "{sql}");
        assert!(
            join_lines[rejoin..foreach]
                .iter()
                .any(|line| line.contains("r.last_updated >= ?3")),
            "{sql}"
        );
        assert!(!join_lines[foreach].contains("r.last_updated"), "{sql}");
        assert_eq!(sql.matches("r.last_updated >= ?3").count(), 2, "{sql}");
        // #1623 2C: `ORDER BY 1` keeps its value and NULL placement, then the
        // resource key, traversal identity and the post-repeat ordinal.
        assert!(
            sql.ends_with(
                "\nORDER BY 1 ASC NULLS FIRST, rec_0.last_updated, rec_0.rid, \
                 (rec_0.ident || '00000000000000000000' || printf('%020d', COALESCE(fe2.rowid, -1) + 1)) \
                 COLLATE BINARY"
            ),
            "{sql}"
        );

        // A plain expansion: the `where()` membership stays in the ON clause;
        // the runtime predicate lands only in the resource WHERE.
        let expansion = json!({"resource":"Patient","select":[
            {"forEach":"name.where(use = 'official')","column":[{"path":"family","name":"f"}]}]});
        let sql = emit_with(&expansion, &SqliteDialect, &predicates);
        let (from, conjuncts) = sql.split_once("\nWHERE ").expect("WHERE clause");
        assert!(from.contains("ON 1=1 AND (json_extract(fe.value, '$.use') = 'official')"));
        assert!(!from.contains("r.last_updated"), "{sql}");
        assert_eq!(
            conjuncts,
            "r.tenant_id = ?1\n  AND r.resource_type = ?2\n  AND r.is_deleted = 0\n  \
             AND r.last_updated >= ?3\nORDER BY r.last_updated, r.id, fe.rowid"
        );
    }

    // ------------------------------------------------------------------
    // #1623 2C: recursive traversal identity, `%rowIndex`, resource access
    // ------------------------------------------------------------------

    /// `(cte, outer)` of a standalone recursive statement.
    fn recursive_split(sql: &str) -> (&str, &str) {
        sql.split_once("\n)\nSELECT")
            .expect("standalone WITH RECURSIVE statement")
    }

    const SQLITE_TERMINATOR: &str = "'00000000000000000000'";

    #[test]
    fn test_repeat_single_path_identity_rank_and_order() {
        let v = view(
            "QuestionnaireResponse",
            json!([{"repeat":["item"],"column":[
                {"path":"linkId","name":"link"},
                {"path":"%rowIndex","name":"i","type":"integer"}]}]),
        );

        let sqlite = emit_with(&v, &SqliteDialect, &ResourcePredicates::none());
        let (cte, outer) = recursive_split(&sqlite);
        assert!(
            cte.starts_with("WITH RECURSIVE rec_0(rid, node, ident, last_updated) AS ("),
            "{sqlite}"
        );
        // Seed edge: path #0 (token 1), the occurrence, a terminator.
        assert!(
            cte.contains(&format!(
                "SELECT r.id AS rid, je.value AS node, '00000000000000000001' || \
                 printf('%020d', je.rowid + 1) || {SQLITE_TERMINATOR} AS ident, \
                 r.last_updated\n  FROM resources r, json_each(r.data, '$.item') je"
            )),
            "{sqlite}"
        );
        // Step edge appended to the parent's identity.
        assert!(
            cte.contains(&format!(
                "SELECT rec_0.rid, rs0.value AS node, rec_0.ident || '00000000000000000001' || \
                 printf('%020d', rs0.rowid + 1) || {SQLITE_TERMINATOR} AS ident, \
                 rec_0.last_updated\n  FROM rec_0, json_each("
            )),
            "{sqlite}"
        );
        assert!(
            outer.contains("CAST(rec_0.row_index AS INTEGER) AS \"i\""),
            "{sqlite}"
        );
        assert!(
            outer.ends_with(
                "\nFROM (SELECT rec_0.*, ROW_NUMBER() OVER (PARTITION BY rec_0.rid \
                 ORDER BY rec_0.ident COLLATE BINARY) - 1 AS row_index FROM rec_0) AS rec_0\n\
                 ORDER BY 1 ASC NULLS FIRST, rec_0.last_updated, rec_0.rid, \
                 rec_0.ident COLLATE BINARY"
            ),
            "{sqlite}"
        );

        let pg = emit_with(&v, &PgDialect, &ResourcePredicates::none());
        let (cte, outer) = recursive_split(&pg);
        assert!(
            cte.starts_with("WITH RECURSIVE rec_0(rid, node, ident, last_updated) AS ("),
            "{pg}"
        );
        assert!(
            cte.contains(
                "SELECT r.id AS rid, je.value AS node, ARRAY[0, (je.ord - 1), -1]::bigint[] \
                 AS ident, r.last_updated\n  FROM resources r \
                 CROSS JOIN LATERAL jsonb_extract_path(r.data, VARIADIC '{}'::text[]) AS rdoc(doc) \
                 JOIN LATERAL jsonb_array_elements((CASE WHEN \
                 jsonb_typeof(COALESCE(rdoc.doc, r.data)->'item')"
            ),
            "{pg}"
        );
        assert!(
            cte.contains(") WITH ORDINALITY AS je(value, ord) ON TRUE"),
            "{pg}"
        );
        assert!(
            cte.contains(
                "SELECT rec_0.rid, rs0.value AS node, \
                 rec_0.ident || ARRAY[0, (rs0.ord - 1), -1]::bigint[] AS ident, \
                 rec_0.last_updated\n  FROM rec_0 JOIN LATERAL jsonb_array_elements("
            ),
            "{pg}"
        );
        assert!(
            cte.contains(") WITH ORDINALITY AS rs0(value, ord) ON TRUE"),
            "{pg}"
        );
        assert!(
            outer.ends_with(
                "\nFROM (SELECT rec_0.*, ROW_NUMBER() OVER (PARTITION BY rec_0.rid \
                 ORDER BY rec_0.ident) - 1 AS row_index FROM rec_0) AS rec_0\n\
                 ORDER BY 1 ASC NULLS LAST, rec_0.last_updated, rec_0.rid, rec_0.ident"
            ),
            "{pg}"
        );

        // Without `%rowIndex` no ranking pass is added; the order is the same.
        let plain = view(
            "QuestionnaireResponse",
            json!([{"repeat":["item"],"column":[{"path":"linkId","name":"link"}]}]),
        );
        let pg = emit_with(&plain, &PgDialect, &ResourcePredicates::none());
        assert!(
            pg.ends_with(
                "\nFROM rec_0\nORDER BY 1 ASC NULLS LAST, rec_0.last_updated, rec_0.rid, \
                 rec_0.ident"
            ),
            "{pg}"
        );
        assert!(!pg.contains("ROW_NUMBER"), "{pg}");
    }

    #[test]
    fn test_repeat_multi_path_identity_has_path_index_every_ordinal_and_one_self_reference() {
        let v = view(
            "QuestionnaireResponse",
            json!([{"repeat":["item","answer.item"],"column":[
                {"path":"linkId","name":"link"},
                {"path":"%rowIndex","name":"i","type":"integer"}]}]),
        );

        let pg = emit_with(&v, &PgDialect, &ResourcePredicates::none());
        let (cte, outer) = recursive_split(&pg);
        // Distinct seed path indices.
        assert!(cte.contains("ARRAY[0, (je.ord - 1), -1]::bigint[] AS ident"));
        assert!(cte.contains("ARRAY[1, (je.ord - 1), -1]::bigint[] AS ident"));
        // Exactly one recursive self-reference: a lateral UNION ALL of the
        // step paths, each carrying its path index and every navigation
        // occurrence (the intermediate `answer` too).
        assert_eq!(cte.matches("FROM rec_0").count(), 1, "{pg}");
        assert!(
            cte.contains(
                "SELECT rec_0.rid, _step.value AS node, rec_0.ident || _step.edge AS ident, \
                 rec_0.last_updated\n  FROM rec_0, LATERAL (SELECT rs0.value, \
                 ARRAY[0, (rs0.ord - 1), -1]::bigint[] FROM "
            ),
            "{pg}"
        );
        assert!(
            cte.contains(
                "SELECT rs1.value, ARRAY[1, (rs0.ord - 1), (rs1.ord - 1), -1]::bigint[] FROM "
            ),
            "{pg}"
        );
        assert!(cte.contains(") AS _step(value, edge)"), "{pg}");
        assert!(!pg.contains("ord_path"), "{pg}");
        assert!(
            outer.contains("ORDER BY rec_0.ident) - 1 AS row_index"),
            "{pg}"
        );

        let sqlite = emit_with(&v, &SqliteDialect, &ResourcePredicates::none());
        let (cte, _) = recursive_split(&sqlite);
        assert!(cte.contains(&format!(
            "'00000000000000000001' || printf('%020d', je.rowid + 1) || {SQLITE_TERMINATOR} AS ident"
        )));
        assert!(cte.contains(&format!(
            "'00000000000000000002' || printf('%020d', je.rowid + 1) || {SQLITE_TERMINATOR} AS ident"
        )));
        assert!(
            cte.contains(&format!(
                "rec_0.ident || '00000000000000000002' || printf('%020d', rs0.rowid + 1) || \
                 printf('%020d', rs1.rowid + 1) || {SQLITE_TERMINATOR} AS ident"
            )),
            "{sqlite}"
        );
    }

    #[test]
    fn test_repeat_with_nested_foreach_ranks_before_expansion_and_orders_post_repeat_ordinals() {
        let v = view(
            "QuestionnaireResponse",
            json!([{"repeat":["item"],"select":[
                {"column":[{"path":"linkId","name":"link"},
                    {"path":"%rowIndex","name":"node_i","type":"integer"}]},
                {"forEach":"answer","column":[{"path":"valueString","name":"ans"},
                    {"path":"%rowIndex","name":"ans_i","type":"integer"}]}]}]),
        );

        let pg = emit_with(&v, &PgDialect, &ResourcePredicates::none());
        let (_, outer) = recursive_split(&pg);
        let ranked = outer
            .find("ROW_NUMBER() OVER (PARTITION BY rec_0.rid ORDER BY rec_0.ident) - 1 AS row_index FROM rec_0) AS rec_0")
            .expect("rank pass over the CTE");
        let expansion = outer
            .find(") WITH ORDINALITY AS fe2(value, ordinality) ON TRUE")
            .expect("post-repeat expansion carries its ordinality");
        assert!(ranked < expansion, "{pg}");
        assert!(
            outer.contains("((rec_0.row_index)::bigint)::text AS \"node_i\""),
            "{pg}"
        );
        assert!(
            outer.contains(
                "((COALESCE(CAST(fe2.ordinality AS INTEGER) - 1, 0))::bigint)::text AS \"ans_i\""
            ),
            "{pg}"
        );
        assert!(
            pg.ends_with(
                "ORDER BY 1 ASC NULLS LAST, rec_0.last_updated, rec_0.rid, \
                 rec_0.ident || ARRAY[-1, (fe2.ordinality - 1)]::bigint[]"
            ),
            "{pg}"
        );

        let sqlite = emit_with(&v, &SqliteDialect, &ResourcePredicates::none());
        let (_, outer) = recursive_split(&sqlite);
        let ranked = outer
            .find("AS row_index FROM rec_0) AS rec_0")
            .expect("rank pass over the CTE");
        let expansion = outer
            .find("\nJOIN json_each(")
            .expect("post-repeat expansion");
        assert!(ranked < expansion, "{sqlite}");
        assert!(
            sqlite.ends_with(&format!(
                "ORDER BY 1 ASC NULLS FIRST, rec_0.last_updated, rec_0.rid, \
                 (rec_0.ident || {SQLITE_TERMINATOR} || printf('%020d', fe2.rowid + 1)) COLLATE BINARY"
            )),
            "{sqlite}"
        );
    }

    #[test]
    fn test_union_repeat_branch_tail_carries_the_traversal_identity() {
        let v = view(
            "QuestionnaireResponse",
            json!([{"unionAll":[
                {"repeat":["item","answer.item"],"column":[{"path":"linkId","name":"v"}]},
                {"column":[{"path":"id","name":"v"}]}
            ]}]),
        );
        for (dialect, number) in [
            (&SqliteDialect as &dyn Dialect, "0 AS k3"),
            (&PgDialect, "CAST(0 AS bigint) AS k3"),
        ] {
            let sql = emit_with(&v, dialect, &ResourcePredicates::none());
            let parts = union_parts(&sql);
            assert!(
                parts.operands[0].starts_with(
                    "SELECT * FROM (WITH RECURSIVE rec_0(rid, node, ident, last_updated) AS ("
                ),
                "{sql}"
            );
            assert!(
                parts.operands[0].contains(&format!(
                    "rec_0.last_updated AS k1,\n  rec_0.rid AS k2,\n  {number},\n  \
                     rec_0.ident AS k4\nFROM rec_0) AS _recurse_0"
                )),
                "{sql}"
            );
        }
    }

    #[test]
    fn test_recursive_resource_rejoin_follows_compile_time_dependencies() {
        let repeat = json!({"repeat":["item"],"column":[{"path":"linkId","name":"link"}]});
        let cases = [
            ("repeat-alone", json!([repeat.clone()]), false),
            (
                "literal-sibling",
                json!([{"column":[{"path":"'x'","name":"x"}]}, repeat.clone()]),
                false,
            ),
            (
                "resource-column-sibling",
                json!([{"column":[{"path":"id","name":"id"}]}, repeat.clone()]),
                true,
            ),
            (
                "resource-foreach-sibling",
                json!([repeat.clone(),
                    {"forEach":"item","column":[{"path":"linkId","name":"top"}]}]),
                true,
            ),
            (
                "where-projection-sibling",
                json!([repeat.clone(),
                    {"column":[{"path":"item.where(linkId = 'a').linkId","name":"w"}]}]),
                true,
            ),
            (
                "indexed-sibling",
                json!([repeat.clone(),
                    {"forEach":"item[0]","column":[{"path":"linkId","name":"first"}]}]),
                true,
            ),
            (
                "nested-foreach-under-repeat",
                json!([{"repeat":["item"],"select":[
                    {"column":[{"path":"linkId","name":"link"}]},
                    {"forEach":"answer","column":[{"path":"valueString","name":"a"}]}]}]),
                false,
            ),
        ];
        for (case, select, rejoin) in cases {
            let v = view("QuestionnaireResponse", select);
            for dialect in [&SqliteDialect as &dyn Dialect, &PgDialect] {
                let sql = emit_with(&v, dialect, &ResourcePredicates::none());
                let (_, outer) = recursive_split(&sql);
                assert_eq!(
                    outer.contains("JOIN resources r ON r.id = rec_0.rid"),
                    rejoin,
                    "{case} ({}): {sql}",
                    dialect.name()
                );
                // Without the rejoin nothing outside the CTE may read `r`
                // (or bind its document); with it, PostgreSQL binds the
                // detoasted document after the rejoin.
                if !rejoin {
                    assert!(!outer.contains("r.data"), "{case}: {sql}");
                    assert!(!outer.contains("rdoc"), "{case}: {sql}");
                } else if dialect
                    .resource_document_lateral(ScanFanOut::Expanded)
                    .is_some()
                {
                    assert_eq!(
                        outer.matches(PG_EXPANDED_LATERAL).count(),
                        1,
                        "{case}: {sql}"
                    );
                }
            }
        }

        // A shared resource column merged into a recursive union branch.
        let v = view(
            "QuestionnaireResponse",
            json!([{"column":[{"path":"id","name":"id"}]},
                {"unionAll":[{"repeat":["item"],"column":[{"path":"linkId","name":"v"}]},
                    {"column":[{"path":"'flat'","name":"v"}]}]}]),
        );
        for dialect in [&SqliteDialect as &dyn Dialect, &PgDialect] {
            let sql = emit_with(&v, dialect, &ResourcePredicates::none());
            assert!(
                union_operands(&sql)[0].contains("JOIN resources r ON r.id = rec_0.rid"),
                "{sql}"
            );
        }
    }

    #[test]
    fn test_indexed_foreach_picks_in_element_order_with_zero_row_index() {
        let indexed = view(
            "Patient",
            json!([{"forEach":"name.given[1]","column":[
                {"path":"$this","name":"g"},
                {"path":"%rowIndex","name":"i","type":"integer"}]}]),
        );
        // SQLite's truthy predicate repeats its operand three times.
        for (dialect, order, membership_copies) in [
            (&SqliteDialect as &dyn Dialect, "fe.rowid, fe2.rowid", 3),
            (&PgDialect, "fe.ordinality, fe2.ordinality", 1),
        ] {
            let sql = emit_with(&indexed, dialect, &ResourcePredicates::none());
            // Value, %rowIndex and membership picks, each in element order
            // before `LIMIT 1 OFFSET 1`.
            let pick = format!(" ORDER BY {order} LIMIT 1 OFFSET 1)");
            assert_eq!(sql.matches(&pick).count(), 2 + membership_copies, "{sql}");
            // %rowIndex is the singleton index, not an element position.
            assert!(sql.contains("(SELECT 0 FROM "), "{sql}");
            assert!(
                !sql.contains("fe2.key") && !sql.contains("CAST(fe2.ordinality"),
                "{sql}"
            );
            // The membership filter (presence, not the value) is a WHERE
            // conjunct; the outer order is the resource key alone.
            assert!(sql.contains("(SELECT 1 FROM "), "{sql}");
            assert!(order_by(&sql) == "r.last_updated, r.id", "{sql}");
        }
    }

    #[test]
    fn test_indexed_foreach_or_null_evaluates_the_empty_context_without_a_filter() {
        let indexed = view(
            "Patient",
            json!([{"forEachOrNull":"name[1]","column":[
                {"path":"%rowIndex","name":"i","type":"integer"}]}]),
        );
        for (dialect, on) in [
            (&SqliteDialect as &dyn Dialect, "ON 1=1)"),
            (&PgDialect, "ON TRUE)"),
        ] {
            let sql = emit_with(&indexed, dialect, &ResourcePredicates::none());
            assert!(
                sql.contains("(SELECT 0 FROM (SELECT 1 AS one) AS fe_ctx LEFT JOIN (SELECT fe.value AS value FROM "),
                "{sql}"
            );
            assert!(sql.contains(&format!(") AS fe {on}")), "{sql}");
            assert!(!sql.contains("(SELECT 1 FROM "), "{sql}");
        }
    }

    #[test]
    fn test_indexed_membership_filters_reach_nested_union_and_repeat_scopes() {
        let membership = "(SELECT 1 FROM ";
        let nested = view(
            "Patient",
            json!([{"forEach":"contact","select":[
                {"forEach":"telecom[1]","column":[{"path":"value","name":"v"}]}]}]),
        );
        let union = view(
            "Patient",
            json!([{"forEach":"telecom[0]","column":[{"path":"value","name":"t"}]},
                {"unionAll":[{"column":[{"path":"id","name":"v"}]},
                    {"forEach":"name[1]","column":[{"path":"family","name":"v"}]}]}]),
        );
        let repeat = view(
            "QuestionnaireResponse",
            json!([{"repeat":["item"],"column":[{"path":"linkId","name":"l"}],
                "select":[{"forEach":"answer[1]","column":[{"path":"valueString","name":"a"}]}]}]),
        );
        // SQLite's truthy predicate repeats its operand three times.
        for (dialect, copies) in [(&SqliteDialect as &dyn Dialect, 3), (&PgDialect, 1)] {
            let sql = emit_with(&nested, dialect, &ResourcePredicates::none());
            let (_, conjuncts) = sql.split_once("\nWHERE ").expect("WHERE");
            assert!(conjuncts.contains(membership), "nested: {sql}");
            assert!(
                conjuncts.contains("fe.value"),
                "nested chain reads the outer row: {sql}"
            );

            // The shared indexed sibling filters both branches; the second
            // branch adds its own.
            let sql = emit_with(&union, dialect, &ResourcePredicates::none());
            let operands = union_operands(&sql);
            assert_eq!(operands[0].matches(membership).count(), copies, "{sql}");
            assert_eq!(operands[1].matches(membership).count(), 2 * copies, "{sql}");

            // Under the repeat: a conjunct over the joined CTE rows.
            let sql = emit_with(&repeat, dialect, &ResourcePredicates::none());
            let body = sql.rsplit_once("\nORDER BY ").unwrap().0;
            let (_, conjuncts) = body.rsplit_once("\nWHERE ").expect("repeat WHERE");
            assert!(conjuncts.contains(membership), "repeat: {sql}");
            assert!(conjuncts.contains("rec_0.node"), "repeat: {sql}");
        }
    }

    #[test]
    fn test_direct_ir_flat_index_keeps_prior_joins_and_picks_the_flattened_element() {
        let scan = PlanNode::Scan {
            alias: "r".into(),
            resource_type: "Patient".into(),
        };
        let path = |root: &str, fields: &[&str]| SqlExpr::JsonPath {
            root: root.to_string(),
            path: JsonPath(
                fields
                    .iter()
                    .map(|f| PathStep::Field(f.to_string()))
                    .collect(),
            ),
        };
        let sibling = PlanNode::LateralUnnest {
            parent: Box::new(scan),
            source: path("r.data", &["telecom"]),
            out_alias: "ft".into(),
            left_join: false,
            on_filter: None,
            flat_index: None,
        };
        let plan = PlanNode::Project {
            parent: Box::new(PlanNode::LateralUnnest {
                parent: Box::new(sibling),
                source: path("r.data", &["contact", "telecom"]),
                out_alias: "fe".into(),
                left_join: true,
                on_filter: None,
                flat_index: Some(2),
            }),
            columns: vec![super::super::ir::Column {
                name: "i".into(),
                expr: SqlExpr::RowIndex(RowIndexScope::ForEach("fe".into())),
                collection: false,
                ty: SqlType::Integer,
                decode: ColumnDecode::Auto,
            }],
        };
        for (dialect, sibling_join, pick, order) in [
            (
                &SqliteDialect as &dyn Dialect,
                "JOIN json_each(r.data, '$.telecom') ft ON 1=1",
                "LEFT JOIN json_each((SELECT json_array(",
                "ORDER BY fe_f0.rowid, fe.rowid LIMIT 1 OFFSET 2)) fe ON 1=1",
            ),
            (
                &PgDialect,
                "WITH ORDINALITY AS ft(value, ordinality) ON TRUE",
                "LEFT JOIN LATERAL (SELECT fe.value, 1::bigint FROM ",
                "ORDER BY fe_f0.ordinality, fe.ordinality LIMIT 1 OFFSET 2) \
                 AS fe(value, ordinality) ON TRUE",
            ),
        ] {
            let sql = emit_plan(&plan, dialect).unwrap().sql;
            assert!(sql.contains(sibling_join), "{sql}");
            assert!(sql.contains(pick) && sql.contains(order), "{sql}");
            // The prior sibling keeps its occurrence; the pick adds none.
            let sibling_ordinal = if dialect.lateral_keyword().is_empty() {
                "ft.rowid"
            } else {
                "(ft.ordinality - 1)"
            };
            assert_eq!(
                order_by(&sql),
                format!("r.last_updated, r.id, {sibling_ordinal}"),
                "{sql}"
            );
        }
    }

    #[test]
    fn test_indexed_trailing_where_filters_the_selection_after_indexing() {
        let filtered = |kind: &str| {
            view(
                "Patient",
                json!([{kind:"name[100].where(use = 'official')","column":[
                    {"path":"family","name":"family"},
                    {"path":"%rowIndex","name":"i","type":"integer"}]}]),
            )
        };
        for (dialect, pick, always, crit) in [
            (
                &SqliteDialect as &dyn Dialect,
                "(SELECT fe.value AS value FROM json_each(r.data, '$.name') fe \
                 ORDER BY fe.rowid LIMIT 1 OFFSET 100) AS fe",
                "1=1",
                "(json_extract(fe.value, '$.use') = 'official')",
            ),
            (
                &PgDialect,
                "(SELECT fe.value AS value FROM jsonb_array_elements(",
                "TRUE",
                "(fe.value->>'use' = 'official')",
            ),
        ] {
            let sql = emit_with(&filtered("forEach"), dialect, &ResourcePredicates::none());
            // The criterion reads the picked row after `LIMIT 1 OFFSET 100`;
            // it never filters the chain before the index.
            let after_pick = format!(") AS fe WHERE {crit})");
            assert!(sql.contains(pick), "{sql}");
            assert!(!sql.contains(&format!("WHERE {crit} ORDER BY")), "{sql}");
            // Value, %rowIndex and the membership filter are all gated.
            assert!(
                sql.contains("(SELECT fe.value->>'family' FROM (SELECT fe.value AS value FROM ")
                    || sql.contains(
                        "(SELECT json_extract(fe.value, '$.family') FROM (SELECT fe.value AS value FROM "
                    ),
                "{sql}"
            );
            assert!(
                sql.contains("(SELECT 0 FROM (SELECT fe.value AS value FROM "),
                "{sql}"
            );
            assert!(
                sql.contains("(SELECT 1 FROM (SELECT fe.value AS value FROM "),
                "{sql}"
            );
            let (projection, conjuncts) = sql.split_once("\nWHERE ").expect("membership WHERE");
            assert_eq!(projection.matches(&after_pick).count(), 2, "{sql}");
            assert!(conjuncts.contains(&after_pick), "{sql}");

            // `forEachOrNull`: the criterion is the LEFT JOIN condition, so a
            // rejected selection evaluates the empty context; no membership.
            let sql = emit_with(
                &filtered("forEachOrNull"),
                dialect,
                &ResourcePredicates::none(),
            );
            assert_eq!(
                sql.matches(&format!(") AS fe ON {always} AND {crit})"))
                    .count(),
                2,
                "{sql}"
            );
            assert!(
                sql.contains("(SELECT 0 FROM (SELECT 1 AS one) AS fe_ctx LEFT JOIN "),
                "{sql}"
            );
            assert!(!sql.contains("(SELECT 1 FROM "), "{sql}");
            assert!(!sql.contains("\n  AND ((SELECT"), "{sql}");
        }
    }

    #[test]
    fn test_indexed_trailing_where_reaches_the_union_and_nested_membership() {
        let union = view(
            "Patient",
            json!([{"unionAll":[{"column":[{"path":"id","name":"v"}]},
                {"forEach":"name[1].where(use = 'official')","column":[{"path":"family","name":"v"}]}]}]),
        );
        let nested = view(
            "Patient",
            json!([{"forEach":"contact","select":[
                {"forEach":"telecom[1].where(system = 'phone')","column":[{"path":"value","name":"v"}]}]}]),
        );
        for dialect in [&SqliteDialect as &dyn Dialect, &PgDialect] {
            let sql = emit_with(&union, dialect, &ResourcePredicates::none());
            let operands = union_operands(&sql);
            assert!(!operands[0].contains("(SELECT 1 FROM "), "{sql}");
            assert!(
                operands[1].contains("(SELECT 1 FROM (SELECT fe.value AS value FROM "),
                "{sql}"
            );
            assert!(operands[1].contains(") AS fe WHERE ("), "{sql}");
            assert!(operands[1].contains(" = 'official')))"), "{sql}");

            let sql = emit_with(&nested, dialect, &ResourcePredicates::none());
            let (_, conjuncts) = sql.split_once("\nWHERE ").expect("WHERE");
            assert!(
                conjuncts.contains("(SELECT 1 FROM (SELECT fe2.value AS value FROM "),
                "{sql}"
            );
            assert!(conjuncts.contains(") AS fe2 WHERE ("), "{sql}");
            assert!(conjuncts.contains(" = 'phone')))"), "{sql}");
        }
    }

    #[test]
    fn test_filter_then_index_foreach_is_rejected_not_miscompiled() {
        // `name.where(crit)[N]` filters first, then indexes; it is not a
        // simple JSON path, so the SQL runners refuse it rather than
        // silently evaluating `name[N].where(crit)` (or dropping a part).
        for kind in ["forEach", "forEachOrNull"] {
            let v = view(
                "Patient",
                json!([{kind:"name.where(use = 'official')[0]","column":[
                    {"path":"family","name":"family"}]}]),
            );
            for (dialect, target) in [
                (&SqliteDialect as &dyn Dialect, CompileTarget::Sqlite),
                (&PgDialect, CompileTarget::Postgres),
            ] {
                let err = build_plan(&v, dialect, target, FhirVersion::default_enabled())
                    .expect_err("filter-then-index forEach must not compile");
                assert!(
                    matches!(err, SofError::Uncompilable { ref reason } if reason.contains("simple JSON path")),
                    "{kind}: {err:?}"
                );
            }
        }
    }

    // ------------------------------------------------------------------
    // PostgreSQL reads the resource document once per resource scan
    // ------------------------------------------------------------------

    /// Detoasting lateral of a scan whose row expansions multiply.
    const PG_EXPANDED_LATERAL: &str =
        "CROSS JOIN LATERAL jsonb_extract_path(r.data, VARIADIC '{}'::text[]) AS rdoc(doc)";
    /// Inlined alias lateral of a scan with one output row per resource.
    const PG_SINGLE_LATERAL: &str = "CROSS JOIN LATERAL (SELECT r.data AS doc) AS rdoc";
    /// Root of expansion sources off the resource document.
    const PG_EXPANSION_DOCUMENT: &str = "COALESCE(rdoc.doc, r.data)";

    /// [`PgDialect`] reading `r.data` directly, without the document
    /// lateral: the PostgreSQL emission the lateral replaced.
    struct PgScannedDocument;

    impl Dialect for PgScannedDocument {
        fn name(&self) -> &'static str {
            PgDialect.name()
        }
        fn placeholder(&self, idx: usize) -> String {
            PgDialect.placeholder(idx)
        }
        fn json_field(&self, base: &str, key: &str) -> String {
            PgDialect.json_field(base, key)
        }
        fn json_field_text(&self, base: &str, key: &str) -> String {
            PgDialect.json_field_text(base, key)
        }
        fn json_path(&self, base: &str, segments: &[&str]) -> String {
            PgDialect.json_path(base, segments)
        }
        fn json_path_text(&self, base: &str, segments: &[&str]) -> String {
            PgDialect.json_path_text(base, segments)
        }
        fn unnest_array(&self, expr: &str) -> String {
            PgDialect.unnest_array(expr)
        }
        fn coalesce_array(&self, expr: &str) -> String {
            PgDialect.coalesce_array(expr)
        }
        fn json_type(&self, expr: &str) -> String {
            PgDialect.json_type(expr)
        }
        fn json_agg(&self, expr: &str) -> String {
            PgDialect.json_agg(expr)
        }
        fn string_agg(&self, expr: &str, sep_param: &str) -> String {
            PgDialect.string_agg(expr, sep_param)
        }
        fn bool_true(&self) -> &'static str {
            PgDialect.bool_true()
        }
        fn bool_false(&self) -> &'static str {
            PgDialect.bool_false()
        }
        fn lateral_keyword(&self) -> &'static str {
            PgDialect.lateral_keyword()
        }
        fn cast(&self, inner: &str, ty: SqlType) -> String {
            PgDialect.cast(inner, ty)
        }
        fn has_json_type(&self, expr: &str, ty: JsonType) -> String {
            PgDialect.has_json_type(expr, ty)
        }
        fn truthy_predicate(&self, expr: &str) -> String {
            PgDialect.truthy_predicate(expr)
        }
        fn last_path_segment(&self, s: &str) -> String {
            PgDialect.last_path_segment(s)
        }
        fn resource_document(&self) -> &'static str {
            SCANNED_DOCUMENT
        }
        fn resource_document_lateral(&self, _fan_out: ScanFanOut) -> Option<&'static str> {
            None
        }
        fn expansion_document(&self) -> &'static str {
            SCANNED_DOCUMENT
        }
    }

    /// Emits `view` for `target` with `_since` and Patient predicates.
    fn emit_document_case(view: &Value, dialect: &dyn Dialect, target: CompileTarget) -> String {
        let (plan, _) = build_plan(view, dialect, target, FhirVersion::default_enabled())
            .expect("build test plan");
        let predicates = ResourcePredicates::new(
            3,
            2,
            vec!["r.last_updated >= $3".into(), "(r.id = $4)".into()],
        );
        emit_plan_with_predicates(&plan, dialect, &predicates)
            .expect("emit test plan")
            .sql
    }

    /// Every SQL shape that reads the resource document: projections, view
    /// `where`, `forEach`/`forEachOrNull` sources and `where()` criteria,
    /// indexed chains, `%rowIndex`, unions, recursive seeds and the rejoin.
    fn document_cases() -> Vec<(&'static str, Value)> {
        vec![
            (
                "flat-where",
                json!({"resource":"Patient","where":[{"path":"active = true"}],
                    "select":[{"column":[
                        {"path":"getResourceKey()","name":"id"},
                        {"path":"name.family","name":"family"},
                        {"path":"name.given.join(',')","name":"given"},
                        {"path":"managingOrganization.getReferenceKey(Organization)","name":"org"},
                        {"path":"telecom.where(system = 'phone').value","name":"phone"},
                        {"path":"birthDate.exists()","name":"has_dob","type":"boolean"}]}]}),
            ),
            (
                "expanded",
                view(
                    "Patient",
                    json!([{"column":[{"path":"id","name":"id"}]},
                        {"forEach":"name.where(use = 'official')","column":[
                            {"path":"family","name":"family"},
                            {"path":"%rowIndex","name":"i","type":"integer"}],
                         "select":[{"forEachOrNull":"given","column":[
                            {"path":"$this","name":"given"}]}]}]),
                ),
            ),
            (
                "indexed",
                view(
                    "Patient",
                    json!([{"column":[{"path":"id","name":"id"}]},
                        {"forEach":"telecom[0]","column":[{"path":"value","name":"t"}]},
                        {"forEachOrNull":"contact.telecom[1]","column":[
                            {"path":"value","name":"c"},
                            {"path":"%rowIndex","name":"ci","type":"integer"}]}]),
                ),
            ),
            (
                "union",
                view(
                    "Patient",
                    json!([{"column":[{"path":"id","name":"id"}],"unionAll":[
                        {"forEach":"telecom","column":[{"path":"value","name":"v"}]},
                        {"unionAll":[
                            {"forEach":"address","column":[{"path":"city","name":"v"}]},
                            {"forEach":"contact.telecom[0]","column":[{"path":"value","name":"v"}]}]},
                        {"column":[{"path":"gender","name":"v"}]}]}]),
                ),
            ),
            (
                "repeat-rejoin",
                json!({"resource":"QuestionnaireResponse","where":[{"path":"status = 'completed'"}],
                    "select":[{"column":[{"path":"id","name":"id"}]},
                        {"repeat":["item","answer.item"],"select":[
                            {"column":[{"path":"linkId","name":"link"},
                                {"path":"%rowIndex","name":"ri","type":"integer"}]},
                            {"forEachOrNull":"answer","column":[
                                {"path":"valueString","name":"a"}]}]}]}),
            ),
            (
                "union-repeat",
                view(
                    "QuestionnaireResponse",
                    json!([{"column":[{"path":"id","name":"id"}]},
                        {"unionAll":[{"repeat":["item"],"column":[{"path":"linkId","name":"v"}]},
                            {"forEach":"item","column":[{"path":"linkId","name":"v"}]}]}]),
                ),
            ),
        ]
    }

    /// `sql` without its document laterals and with every document read
    /// back on `r.data`.
    fn without_document_laterals(sql: &str) -> String {
        let mut restored = sql.to_string();
        for lateral in [PG_EXPANDED_LATERAL, PG_SINGLE_LATERAL] {
            restored = restored
                .replace(&format!("\n{lateral}"), "")
                .replace(&format!(" {lateral}"), "");
        }
        restored
            .replace(PG_EXPANSION_DOCUMENT, SCANNED_DOCUMENT)
            .replace("rdoc.doc", SCANNED_DOCUMENT)
    }

    #[test]
    fn test_pg_binds_the_document_once_per_resource_scan_and_never_reads_r_data() {
        for (case, view) in document_cases() {
            let sql = emit_document_case(&view, &PgDialect, CompileTarget::Postgres);
            let scans = sql.matches("resources r").count();
            assert!(scans > 0, "{case}: {sql}");
            assert_eq!(
                sql.matches(PG_EXPANDED_LATERAL).count() + sql.matches(PG_SINGLE_LATERAL).count(),
                scans,
                "{case}: one document lateral per resource scan: {sql}"
            );
            // `r.data` is read only by the laterals; the expansion root
            // names it as a never-evaluated fallback.
            let reads = sql
                .replace(PG_EXPANDED_LATERAL, "")
                .replace(PG_SINGLE_LATERAL, "")
                .replace(PG_EXPANSION_DOCUMENT, "");
            assert!(
                !reads.contains("r.data"),
                "{case}: every document read goes through rdoc.doc: {sql}"
            );
            assert!(sql.contains("rdoc.doc"), "{case}: {sql}");
        }
    }

    #[test]
    fn test_pg_lateral_form_follows_the_scan_fan_out() {
        // (case, expanded scans, single scans); a recursive statement scans
        // once per seed path plus its rejoin.
        let expected = [
            ("flat-where", 0, 1),
            ("expanded", 1, 0),
            ("indexed", 0, 1),
            ("union", 2, 2),
            ("repeat-rejoin", 3, 0),
            ("union-repeat", 3, 0),
        ];
        let cases = document_cases();
        assert_eq!(cases.len(), expected.len());
        for ((case, view), (name, expanded, single)) in cases.iter().zip(expected) {
            assert_eq!(*case, name);
            let sql = emit_document_case(view, &PgDialect, CompileTarget::Postgres);
            assert_eq!(
                sql.matches(PG_EXPANDED_LATERAL).count(),
                expanded,
                "{case}: {sql}"
            );
            assert_eq!(
                sql.matches(PG_SINGLE_LATERAL).count(),
                single,
                "{case}: {sql}"
            );
        }
    }

    #[test]
    fn test_pg_expansion_sources_off_the_document_name_r_data_for_the_planner() {
        let expanded = &document_cases()[1].1;
        let sql = emit_document_case(expanded, &PgDialect, CompileTarget::Postgres);
        assert!(
            sql.contains(
                "JOIN LATERAL jsonb_array_elements((CASE WHEN \
                 jsonb_typeof(COALESCE(rdoc.doc, r.data)->'name') = 'array' \
                 THEN COALESCE(rdoc.doc, r.data)->'name' "
            ),
            "{sql}"
        );
        // Nested expansions read their parent element, not the document.
        assert!(sql.contains("jsonb_typeof(fe.value->'given')"), "{sql}");
        // Recursive seeds unnest through the expansion root too.
        let repeat = &document_cases()[4].1;
        let sql = emit_document_case(repeat, &PgDialect, CompileTarget::Postgres);
        assert!(
            sql.contains("jsonb_typeof(COALESCE(rdoc.doc, r.data)->'item') = 'array'"),
            "{sql}"
        );
    }

    #[test]
    fn test_pg_union_binds_each_branch_lateral_by_its_own_fan_out() {
        let v = view(
            "Patient",
            json!([{"column":[{"path":"id","name":"id"}],"unionAll":[
                {"forEach":"telecom","column":[{"path":"value","name":"v"}]},
                {"column":[{"path":"gender","name":"v"}]}]}]),
        );
        // The `forEach` branch detoasts once and unnests through the
        // expansion root; the flat branch keeps the inlined alias.
        let expected = "SELECT\n  u.c1 AS \"id\",\n  u.c2 AS \"v\"\nFROM (\nSELECT\n  \
             rdoc.doc->>'id' AS c1,\n  fe.value->>'value' AS c2,\n  r.last_updated AS k1,\n  \
             r.id AS k2,\n  CAST(0 AS bigint) AS k3,\n  ARRAY[(fe.ordinality - 1)]::bigint[] AS k4\n\
             FROM resources r\n\
             CROSS JOIN LATERAL jsonb_extract_path(r.data, VARIADIC '{}'::text[]) AS rdoc(doc)\n\
             JOIN LATERAL jsonb_array_elements((CASE \
             WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'telecom') = 'array' \
             THEN COALESCE(rdoc.doc, r.data)->'telecom' \
             WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'telecom') IS NOT NULL \
             THEN jsonb_build_array(COALESCE(rdoc.doc, r.data)->'telecom') ELSE '[]'::jsonb END)) \
             WITH ORDINALITY AS fe(value, ordinality) ON TRUE\n\
             WHERE r.tenant_id = $1\n  AND r.resource_type = $2\n  AND r.is_deleted = false\n\
             UNION ALL\nSELECT\n  rdoc.doc->>'id' AS c1,\n  rdoc.doc->>'gender' AS c2,\n  \
             r.last_updated AS k1,\n  r.id AS k2,\n  CAST(1 AS bigint) AS k3,\n  \
             ARRAY[]::bigint[] AS k4\n\
             FROM resources r\n\
             CROSS JOIN LATERAL (SELECT r.data AS doc) AS rdoc\n\
             WHERE r.tenant_id = $1\n  AND r.resource_type = $2\n  AND r.is_deleted = false\n\
             ) AS u\nORDER BY u.c1 ASC NULLS LAST, u.k1, u.k2, u.k3, u.k4";
        assert_eq!(
            emit_with(&v, &PgDialect, &ResourcePredicates::none()),
            expected
        );
    }

    #[test]
    fn test_pg_document_lateral_changes_nothing_but_document_reads() {
        for (case, view) in document_cases() {
            let sql = emit_document_case(&view, &PgDialect, CompileTarget::Postgres);
            let direct = emit_document_case(&view, &PgScannedDocument, CompileTarget::Postgres);
            assert!(!direct.contains("rdoc"), "{case}: {direct}");
            // Dropping the laterals and reading `r.data` again restores the
            // previous statement byte for byte: same joins, conjuncts,
            // runtime predicates, hidden keys and ORDER BY.
            assert_eq!(without_document_laterals(&sql), direct, "{case}");
        }
    }

    #[test]
    fn test_sqlite_reads_r_data_directly() {
        assert_eq!(SqliteDialect.resource_document(), SCANNED_DOCUMENT);
        assert_eq!(SqliteDialect.expansion_document(), SCANNED_DOCUMENT);
        for fan_out in [ScanFanOut::Single, ScanFanOut::Expanded] {
            assert_eq!(SqliteDialect.resource_document_lateral(fan_out), None);
        }
        assert_eq!(
            PgDialect.resource_document_lateral(ScanFanOut::Single),
            Some(PG_SINGLE_LATERAL)
        );
        assert_eq!(
            PgDialect.resource_document_lateral(ScanFanOut::Expanded),
            Some(PG_EXPANDED_LATERAL)
        );
        assert_eq!(PgDialect.expansion_document(), PG_EXPANSION_DOCUMENT);
        for (case, view) in document_cases() {
            let sql = emit_document_case(&view, &SqliteDialect, CompileTarget::Sqlite);
            assert!(!sql.contains("rdoc"), "{case}: {sql}");
            assert!(sql.contains("r.data"), "{case}: {sql}");
        }
    }
}
