//! Intermediate representation for the FHIRPath → SQL compiler.
//!
//! Two layered IRs:
//!
//! - [`SqlExpr`] is a dialect-independent value-level expression. Every FHIRPath
//!   sub-expression compiles to one of these. The [`Dialect`](super::dialect::Dialect)
//!   trait lowers an `SqlExpr` to a SQL string per backend.
//! - [`PlanNode`] is the row-source-level plan: scans, lateral unnests, filters,
//!   projections, unions, and recursive descents (`repeat:`).
//!
//! Stages 2–5 progressively populate the consumers of these types. Stage 1 just
//! defines the shapes so later work has a stable target.

#![allow(dead_code)] // Stage 1 scaffold; consumers land in stages 2–5.
#![allow(missing_docs)] // Per-field docs land alongside their consumers in stages 2–5.

use std::sync::Arc;

/// A dialect-independent value-level SQL expression.
///
/// Each variant lowers to a SQL fragment via the [`Dialect`](super::dialect::Dialect)
/// trait. Subqueries hold a [`PlanNode`] together with the scalar projection
/// extracted from each row.
#[derive(Debug, Clone)]
pub enum SqlExpr {
    /// Literal scalar.
    Lit(LitValue),

    /// Navigation through a JSON document.
    ///
    /// `root` is the alias provided by the surrounding plan node — typically
    /// `r.data` (resource scan), `fe.value` (lateral unnest), or `rec.node`
    /// (recursive CTE). `path` is the chain of steps applied to it.
    JsonPath { root: String, path: JsonPath },

    /// Bound query parameter, 1-based.
    ///
    /// Indices 1 and 2 are reserved for `tenant_id` and `resource_type`.
    /// Constants from `ViewDefinition.constant[]` and string literals lifted
    /// out of `extension(url)` etc. allocate from index 3 upward.
    Param(usize),

    /// Reference to a column projected by a CTE or subquery.
    ColRef(String),

    /// Type coercion. The dialect lowerer chooses the appropriate cast syntax.
    Cast { inner: Box<SqlExpr>, ty: SqlType },

    /// Binary operator.
    BinOp {
        op: BinOp,
        lhs: Box<SqlExpr>,
        rhs: Box<SqlExpr>,
    },

    /// Unary operator.
    UnaryOp { op: UnaryOp, inner: Box<SqlExpr> },

    /// `CASE WHEN .. THEN .. ... ELSE .. END`.
    Case {
        arms: Vec<(SqlExpr, SqlExpr)>,
        else_: Option<Box<SqlExpr>>,
    },

    /// `COALESCE(a, b, ...)`.
    Coalesce(Vec<SqlExpr>),

    /// `NULLIF(a, b)`.
    NullIf(Box<SqlExpr>, Box<SqlExpr>),

    /// Wrap a scalar as a JSON value (`to_jsonb` / `json`).
    AsJson(Box<SqlExpr>),

    /// Aggregate the rows produced by a subquery into a JSON array
    /// (`jsonb_agg` / `json_group_array`). Used for `column.collection: true`.
    JsonAgg(Box<SubQuery>),

    /// Scalar subquery — the inner plan must project exactly one value per row
    /// and return at most one row.
    Scalar(Box<SubQuery>),

    /// `EXISTS(subquery)` — collapses to a boolean.
    Exists(Box<SubQuery>),

    /// `(SELECT count(*) FROM subquery)`.
    CountSub(Box<SubQuery>),

    /// Names an inner expression for reuse (lowered as a CTE column reference
    /// when the same scalar appears in multiple projections).
    Alias { name: String, inner: Box<SqlExpr> },

    /// Extracts the id portion of a `Reference.reference` string. When
    /// `expected_type` is supplied, returns NULL unless the reference's type
    /// segment matches (e.g. `getReferenceKey(Patient)` over `Observation/123`
    /// returns NULL).
    ReferenceKey {
        reference: Box<SqlExpr>,
        expected_type: Option<String>,
    },

    /// FHIRPath `lowBoundary()` / `highBoundary()` — emits a precision-driven
    /// CASE expression over the source's text form (decimal expands by a
    /// half-step in the last digit; date/dateTime/time pad with the first or
    /// last instant of the largest unspecified unit). The expected
    /// `column.type` is supplied so the dialect can pick decimal vs.
    /// date/dateTime/time logic.
    Boundary {
        side: BoundarySide,
        kind: BoundaryKind,
        source: Box<SqlExpr>,
    },

    /// FHIRPath `<focus>.where(<crit>).exists()` — lowers to an `EXISTS`
    /// subquery that iterates the focus collection (a lateral unnest of a
    /// JSON path) and tests `crit` against each element. The criterion is
    /// pre-lowered with `iter_alias.value` set as its path root.
    WhereExists {
        focus: Box<SqlExpr>,
        iter_alias: String,
        predicate: Box<SqlExpr>,
        /// Mirrors `where(crit).empty()` — negate the EXISTS.
        negate: bool,
    },

    /// FHIRPath `<focus>.where(<crit>).<navigation>` collapsed to a scalar
    /// subquery: iterate the focus collection, filter by the criterion,
    /// project the navigation off the iteration alias, return at most one
    /// row. Used when a column's path threads a `where()` call somewhere in
    /// the middle (e.g. `name.where(use='official').family`).
    WhereScalar {
        focus: Box<SqlExpr>,
        iter_alias: String,
        predicate: Box<SqlExpr>,
        projection: Box<SqlExpr>,
    },

    /// FHIRPath `<base>.<field>.join(<sep>)` — aggregates the values of
    /// `<field>` across each element of `<base>` (flattened) into a single
    /// separator-joined string. Lowers to `string_agg` (PG) /
    /// `group_concat` (SQLite) over a chained lateral unnest.
    JoinAggregate {
        outer_focus: Box<SqlExpr>,
        outer_alias: String,
        inner_field: String,
        inner_alias: String,
        separator: String,
    },

    /// `column.collection: true` projection — aggregates the flattened
    /// values of a JSON path into a JSON array. Each `Field` step in `path`
    /// becomes a lateral unnest; the final element values feed into a
    /// `json_agg` / `json_group_array`.
    CollectionAgg { root: String, path: JsonPath },

    /// Correlated scalar subquery used for `forEach: "<chain>[N]"` paths —
    /// FHIRPath indexes the FLATTENED iteration result, but SQLite forbids
    /// correlated subqueries in `FROM`. Lowering each column to a
    /// scalar-subquery in the SELECT side bypasses that limitation:
    ///
    /// `(SELECT <projection> FROM <chain_sql> LIMIT 1 OFFSET <offset>)`.
    ScalarFromChain {
        chain_sql: String,
        projection: Box<SqlExpr>,
        offset: i64,
    },
}

/// Selects between `lowBoundary()` and `highBoundary()` semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundarySide {
    Low,
    High,
}

/// Source value type for [`SqlExpr::Boundary`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryKind {
    Decimal,
    Date,
    DateTime,
    Time,
}

/// Literal scalar value embedded directly in SQL.
///
/// Strings derived from user input must be bound as parameters via
/// [`SqlExpr::Param`] — `LitValue::Str` is reserved for compile-time-constant
/// identifiers (e.g. polymorphic-type field names).
#[derive(Debug, Clone)]
pub enum LitValue {
    /// `NULL`.
    Null,
    /// Boolean — lowered to `true`/`false` (PG) or `1`/`0` (SQLite).
    Bool(bool),
    /// Integer.
    Int(i64),
    /// Decimal as a string to preserve precision.
    Decimal(String),
    /// String literal — used only for compile-time-constant idents; user input
    /// must always go through [`SqlExpr::Param`].
    Str(String),
}

/// SQL type tag used by [`SqlExpr::Cast`] and column projections.
///
/// The dialect lowerer maps each variant to its native cast syntax
/// (`::text` / `CAST(.. AS TEXT)` etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlType {
    Text,
    Integer,
    Decimal,
    Boolean,
    /// JSON value (PG: `jsonb`; SQLite: `json` returned by `json()` function).
    Json,
}

/// JSON value-type predicate, used by [`PathStep::TypeFilter`] and
/// polymorphic-field guards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonType {
    Object,
    Array,
    String,
    Number,
    Boolean,
    Null,
}

/// Binary operator for [`SqlExpr::BinOp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
    Add,
    Sub,
    Mul,
    Div,
    /// `AND` with SQL three-valued logic.
    And,
    /// `OR` with SQL three-valued logic.
    Or,
    /// String concatenation (PG: `||`; SQLite: `||`).
    Concat,
    /// `LIKE`.
    Like,
    /// `regexp_match` / dialect-specific regex.
    RegexMatch,
}

/// Unary operator for [`SqlExpr::UnaryOp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// `NOT`.
    Not,
    /// `IS NULL`.
    IsNull,
    /// `IS NOT NULL`.
    IsNotNull,
    /// Negation (`-x`).
    Neg,
}

/// Ordered sequence of [`PathStep`]s applied to a JSON root.
#[derive(Debug, Clone, Default)]
pub struct JsonPath(pub Vec<PathStep>);

impl JsonPath {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn push(&mut self, step: PathStep) {
        self.0.push(step);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// One navigation step in a [`JsonPath`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathStep {
    /// `.field` (object key).
    Field(String),
    /// `[N]` (array index).
    Index(i64),
    /// `value.ofType(X)` resolved against FHIR's polymorphic-element JSON
    /// convention. The contained string is the FHIR type name (`Quantity`,
    /// `string`, ...). The lowerer rewrites the previous `Field` step to its
    /// `value{X}` sibling.
    OfType(String),
    /// Restricts the focus to JSON values of a given type — used by
    /// `ofType(primitive)` to make sibling polymorphic fields evaluate to NULL.
    TypeFilter(JsonType),
}

/// Row-source plan node.
///
/// Plans are trees: a [`Project`](PlanNode::Project) at the root, descending
/// through filters and lateral unnests to a [`Scan`](PlanNode::Scan) over
/// `resources`. [`Union`](PlanNode::Union) and [`Recurse`](PlanNode::Recurse)
/// wrap multiple sub-plans.
#[derive(Debug, Clone)]
pub enum PlanNode {
    /// Top-level scan over the `resources` table for a single resource type.
    /// The tenant predicate is injected by the emitter.
    Scan {
        alias: String,
        resource_type: String,
    },

    /// Lateral unnest of a JSON-array source. `out_alias` names the iteration
    /// row; `left_join` distinguishes `forEach` from `forEachOrNull`.
    /// `on_filter`, if set, is appended to the JOIN ON clause and lets a
    /// trailing `where(crit)` on the forEach path filter rows in-place
    /// (preserving LEFT JOIN semantics for `forEachOrNull`). `flat_index`,
    /// if set, restricts the unnest to the Nth element of the flattened
    /// collection (FHIRPath `name[0]` style indexing applied to the result
    /// of an array-flattening navigation).
    LateralUnnest {
        parent: Box<PlanNode>,
        source: SqlExpr,
        out_alias: String,
        left_join: bool,
        on_filter: Option<SqlExpr>,
        flat_index: Option<i64>,
    },

    /// `WHERE` filter applied to `parent`. Multiple `Filter` nodes compose
    /// AND-wise.
    Filter {
        parent: Box<PlanNode>,
        predicate: SqlExpr,
    },

    /// Output projection.
    Project {
        parent: Box<PlanNode>,
        columns: Vec<Column>,
    },

    /// `UNION ALL` of N row-compatible plans. Output schemas must align;
    /// the emitter validates this and emits a single `ORDER BY 1` outside the
    /// compound query.
    Union(Vec<PlanNode>),

    /// Recursive-CTE descent — used for SoF `repeat:` clauses.
    Recurse {
        parent: Box<PlanNode>,
        seed: SqlExpr,
        step_paths: Vec<JsonPath>,
        out_alias: String,
    },
}

/// Output column projected by a [`Project`](PlanNode::Project) node.
#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub expr: SqlExpr,
    /// When true, lower to a JSON array via [`SqlExpr::JsonAgg`] over a lateral
    /// subquery. When false, lower to a scalar (with a defensive `LIMIT 1` if
    /// the underlying expression yields a row source).
    pub collection: bool,
    pub ty: SqlType,
}

/// A subquery embedded inside a [`SqlExpr`]. Holds the inner plan together
/// with the scalar projection extracted from each row.
#[derive(Debug, Clone)]
pub struct SubQuery {
    pub plan: PlanNode,
    pub select_expr: SqlExpr,
}

/// Boxed dialect handle used by emission helpers.
pub type DialectRef = Arc<dyn super::dialect::Dialect>;
