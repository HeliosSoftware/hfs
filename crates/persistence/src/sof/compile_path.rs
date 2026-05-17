//! FHIRPath expression → [`SqlExpr`] compiler.
//!
//! Walks the AST produced by `helios_fhirpath::parser::parser()` and emits a
//! dialect-independent [`SqlExpr`]. The translation covers the subset needed
//! by the SoF v2 conformance corpus, expanded one feature at a time across
//! stages 2–5.
//!
//! ## Coverage by stage
//!
//! - Stage 2 (this file's current state): literals, member navigation, bracket
//!   indexing, comparison & logical operators, polarity, basic function calls
//!   (`exists`, `empty`, `count`, `first`, `last`, `iif`, `not`, `ofType`
//!   primitive), `$this`.
//! - Stage 3: focus-as-collection threading (`.where(...).select(...)` chains
//!   that nest into lateral subqueries).
//! - Stage 4: `%name` constants bound as parameters, `extension(url)`,
//!   reference keys, `ofType(complex)`, `join(sep)`.
//! - Stage 5: `lowBoundary` / `highBoundary`.

use std::collections::HashMap;

use helios_fhir::FhirVersion;
use helios_fhirpath::parse_expression;
use helios_fhirpath::parser::{Expression, Invocation, Literal, Term, TypeSpecifier};

use crate::core::sof_runner::SofError;

use super::ir::{
    BinOp, BoundaryKind, BoundarySide, JsonPath, JsonType, LitValue, PathStep, SqlExpr, SqlType,
    UnaryOp,
};

/// Compile-time environment threaded through expression lowering.
///
/// Tracks the current row-source alias (for [`SqlExpr::JsonPath`] rooting),
/// the next free parameter slot (constants and lifted string literals allocate
/// from here), and any user-supplied `ViewDefinition.constant[]` values so
/// `%name` lookups resolve to a stable parameter index.
#[derive(Debug)]
pub struct CompileEnv {
    /// SQL alias of the current focus row (typically `r`, `fe`, `it1`, …).
    pub root_alias: String,
    /// Next parameter index to allocate (1-based). Initialised to 3 (after
    /// `tenant_id` = $1 and `resource_type` = $2).
    pub next_param: usize,
    /// `ViewDefinition.constant[]` lookup. Each entry is the typed value plus
    /// the parameter slot it has been bound to (or `None` if unallocated).
    pub constants: HashMap<String, Constant>,
    /// Resolved constant values in the order they were bound — emitted by
    /// the runners as additional bound parameters after `tenant_id` /
    /// `resource_type`.
    pub param_bindings: Vec<LitValue>,
    /// Hint for the current column's declared type — used by
    /// `lowBoundary()` / `highBoundary()` to pick decimal vs.
    /// date/dateTime/time semantics. Set per column by `read_columns`.
    pub column_type_hint: Option<String>,
    /// Sequential alias counter for `<focus>.where(...)` lateral subqueries.
    /// Distinct from the plan-level `AliasSeq` so expression-internal
    /// aliases never collide with `forEach` / `repeat` aliases.
    pub next_where_alias: usize,
    /// Root FHIR resource type the ViewDefinition runs over (the
    /// `ViewDefinition.resource` value). Used to seed per-segment field-type
    /// lookups via `helios_fhir::*::get_field_type` — empty when the
    /// compiler is invoked outside the ViewDefinition entry point (e.g.
    /// expression-only unit tests).
    pub resource_type: String,
    /// FHIR version for the field-type lookup tables. Defaults to R4 when
    /// the caller doesn't supply one.
    pub fhir_version: FhirVersion,
}

/// A `ViewDefinition.constant[]` entry resolved to a typed value.
#[derive(Debug, Clone)]
pub struct Constant {
    /// Typed value parsed from the `ViewDefinition.constant[]` entry.
    pub value: LitValue,
    /// Set on first reference; subsequent `%name` references reuse the same
    /// parameter slot.
    pub bound_to: Option<usize>,
}

impl CompileEnv {
    /// Creates an env rooted at `root_alias` with no resource-type context.
    /// Use [`Self::new_for_resource`] when cardinality lookups need the
    /// ViewDefinition's resource type.
    pub fn new(root_alias: impl Into<String>) -> Self {
        Self {
            root_alias: root_alias.into(),
            next_param: 3,
            constants: HashMap::new(),
            param_bindings: Vec::new(),
            column_type_hint: None,
            next_where_alias: 0,
            resource_type: String::new(),
            fhir_version: FhirVersion::default(),
        }
    }

    /// Same as [`Self::new`] but seeds the FHIR resource type and version so
    /// downstream cardinality checks can consult the per-version
    /// `get_field_type` tables.
    pub fn new_for_resource(
        root_alias: impl Into<String>,
        resource_type: impl Into<String>,
        fhir_version: FhirVersion,
    ) -> Self {
        let mut env = Self::new(root_alias);
        env.resource_type = resource_type.into();
        env.fhir_version = fhir_version;
        env
    }
}

/// Root alias used by `compile_view` for the resource document
/// (`{ROOT_ALIAS}.data` → `"r.data"`). Polymorphic-rewrite parent walks only
/// proceed when the path's SQL root matches this alias, since paths rooted
/// at sub-scope iter aliases (`w0.value`, `fe1.value`, …) navigate off an
/// element whose FHIR type we don't track at compile time.
const RESOURCE_ROOT: &str = "r.data";

/// Parse `src` and compile it to [`SqlExpr`].
///
/// # Errors
///
/// Returns [`SofError::Uncompilable`] for syntax errors and FHIRPath shapes
/// not yet covered by this stage.
pub fn compile_fhirpath_expr(src: &str, env: &mut CompileEnv) -> Result<SqlExpr, SofError> {
    let trimmed = src.trim();
    if trimmed.is_empty() {
        return Err(SofError::Uncompilable {
            reason: "empty FHIRPath expression".to_string(),
        });
    }
    let parsed = parse_expression(trimmed).map_err(|e| SofError::Uncompilable { reason: e })?;
    lower_expression(&parsed, env)
}

// ============================================================================
// AST walk
// ============================================================================

fn lower_expression(expr: &Expression, env: &mut CompileEnv) -> Result<SqlExpr, SofError> {
    // Detect chains of the form `<base>.where(crit).<field>(.<field>...)?[.first()]`
    // and lift them into a scalar subquery. Done before normal lowering so we
    // can capture both the focus and the post-where projection in a single IR
    // node.
    if let Some(scalar) = try_lower_where_scalar(expr, env)? {
        return Ok(scalar);
    }
    // Detect `<base>.<field>.join(<sep>)` and lift to a string-aggregate
    // subquery before falling through to general invocation lowering.
    if let Some(agg) = try_lower_join_aggregate(expr, env)? {
        return Ok(agg);
    }
    match expr {
        Expression::Term(term) => lower_term(term, env),
        Expression::Invocation(base, inv) => lower_invocation(base, inv, env),
        Expression::Indexer(base, idx) => lower_indexer(base, idx, env),
        Expression::Polarity(sign, inner) => {
            let inner_sql = lower_expression(inner, env)?;
            Ok(match sign {
                '+' => inner_sql,
                '-' => SqlExpr::UnaryOp {
                    op: UnaryOp::Neg,
                    inner: Box::new(inner_sql),
                },
                other => {
                    return Err(SofError::Uncompilable {
                        reason: format!("unsupported polarity operator '{other}'"),
                    });
                }
            })
        }
        Expression::Equality(l, op, r) => {
            let lhs = lower_expression(l, env)?;
            let rhs = lower_expression(r, env)?;
            let bin = match op.as_str() {
                "=" => BinOp::Eq,
                "!=" => BinOp::Neq,
                "~" | "!~" => {
                    return Err(SofError::Uncompilable {
                        reason: format!(
                            "equivalence operator '{op}' is not supported by the in-DB runner"
                        ),
                    });
                }
                other => {
                    return Err(SofError::Uncompilable {
                        reason: format!("unsupported equality operator '{other}'"),
                    });
                }
            };
            Ok(SqlExpr::BinOp {
                op: bin,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            })
        }
        Expression::Inequality(l, op, r) => {
            let lhs = lower_expression(l, env)?;
            let rhs = lower_expression(r, env)?;
            let bin = match op.as_str() {
                "<" => BinOp::Lt,
                "<=" => BinOp::Lte,
                ">" => BinOp::Gt,
                ">=" => BinOp::Gte,
                other => {
                    return Err(SofError::Uncompilable {
                        reason: format!("unsupported inequality operator '{other}'"),
                    });
                }
            };
            Ok(SqlExpr::BinOp {
                op: bin,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            })
        }
        Expression::And(l, r) => {
            let lhs = lower_expression(l, env)?;
            let rhs = lower_expression(r, env)?;
            Ok(SqlExpr::BinOp {
                op: BinOp::And,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            })
        }
        Expression::Or(l, op, r) => {
            if op == "xor" {
                return Err(SofError::Uncompilable {
                    reason: "xor operator is not supported by the in-DB runner".to_string(),
                });
            }
            let lhs = lower_expression(l, env)?;
            let rhs = lower_expression(r, env)?;
            Ok(SqlExpr::BinOp {
                op: BinOp::Or,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            })
        }
        Expression::Type(expr, op, ts) => lower_type_op(expr, op, ts, env),
        Expression::Additive(l, op, r) => lower_arithmetic(l, op, r, env),
        Expression::Multiplicative(l, op, r) => lower_arithmetic(l, op, r, env),
        Expression::Union(_, _)
        | Expression::Membership(_, _, _)
        | Expression::Implies(_, _)
        | Expression::Lambda(_, _)
        | Expression::InstanceSelector(_, _) => Err(SofError::Uncompilable {
            reason: format!("FHIRPath construct {expr:?} is not yet supported by the in-DB runner"),
        }),
    }
}

/// Lowers an arithmetic operator (`+`, `-`, `*`, `/`) between two expressions.
/// Both operands are wrapped in a numeric cast so PG accepts arithmetic on
/// the text projections that JSON paths produce by default; SQLite is
/// dynamic-typed so the cast is a no-op there but keeps the IR uniform.
fn lower_arithmetic(
    l: &Expression,
    op: &str,
    r: &Expression,
    env: &mut CompileEnv,
) -> Result<SqlExpr, SofError> {
    let lhs = lower_expression(l, env)?;
    let rhs = lower_expression(r, env)?;
    let bin = match op {
        "+" => BinOp::Add,
        "-" => BinOp::Sub,
        "*" => BinOp::Mul,
        "/" => BinOp::Div,
        "div" | "mod" => {
            return Err(SofError::Uncompilable {
                reason: format!("integer-division operator '{op}' is not yet supported"),
            });
        }
        other => {
            return Err(SofError::Uncompilable {
                reason: format!("unsupported arithmetic operator '{other}'"),
            });
        }
    };
    Ok(SqlExpr::BinOp {
        op: bin,
        lhs: Box::new(SqlExpr::Cast {
            inner: Box::new(lhs),
            ty: SqlType::Decimal,
        }),
        rhs: Box::new(SqlExpr::Cast {
            inner: Box::new(rhs),
            ty: SqlType::Decimal,
        }),
    })
}

fn lower_term(term: &Term, env: &mut CompileEnv) -> Result<SqlExpr, SofError> {
    match term {
        Term::Literal(lit) => lower_literal(lit),
        Term::Invocation(inv) => lower_root_invocation(inv, env),
        Term::Parenthesized(inner) => lower_expression(inner, env),
        Term::ExternalConstant(name) => resolve_external_constant(name, env),
    }
}

fn lower_literal(lit: &Literal) -> Result<SqlExpr, SofError> {
    Ok(match lit {
        Literal::Null => SqlExpr::Lit(LitValue::Null),
        Literal::Boolean(b) => SqlExpr::Lit(LitValue::Bool(*b)),
        Literal::Integer(n) => SqlExpr::Lit(LitValue::Int(*n)),
        Literal::Number(d) => SqlExpr::Lit(LitValue::Decimal(d.to_string())),
        // Strings are bound as parameters in later stages once the env can
        // allocate; for now treat them as compile-time literals (acceptable
        // because they're user-controlled but already validated as FHIRPath
        // string literals by the parser).
        Literal::String(s) => SqlExpr::Lit(LitValue::Str(s.clone())),
        Literal::Date(_) | Literal::DateTime(_) | Literal::Time(_) | Literal::Quantity(_, _) => {
            return Err(SofError::Uncompilable {
                reason: format!("literal {lit:?} is not yet supported by the in-DB runner"),
            });
        }
    })
}

/// Lowers a top-level invocation that appears as the head of a path (not
/// applied to a preceding expression). Member-access from the root resolves
/// to a field navigation off `env.root_alias`.
fn lower_root_invocation(inv: &Invocation, env: &mut CompileEnv) -> Result<SqlExpr, SofError> {
    match inv {
        Invocation::Member(name) => Ok(SqlExpr::JsonPath {
            root: env.root_alias.clone(),
            path: JsonPath(vec![PathStep::Field(name.clone())]),
        }),
        Invocation::This => Ok(SqlExpr::JsonPath {
            root: env.root_alias.clone(),
            path: JsonPath::new(),
        }),
        Invocation::Function(name, args) => {
            // Zero-arg builtins that resolve against the current focus only
            // are unusual at root; defer to call site.
            lower_function_call(
                &SqlExpr::JsonPath {
                    root: env.root_alias.clone(),
                    path: JsonPath::new(),
                },
                name,
                args,
                env,
            )
        }
        Invocation::Index | Invocation::Total => Err(SofError::Uncompilable {
            reason: "$index / $total are not yet supported by the in-DB runner".to_string(),
        }),
    }
}

fn lower_invocation(
    base: &Expression,
    inv: &Invocation,
    env: &mut CompileEnv,
) -> Result<SqlExpr, SofError> {
    // Special-case the chained pattern `<focus>.where(crit).exists()` /
    // `.empty()` — lowers to an EXISTS subquery over a lateral unnest of
    // `focus`. The focus may either be a path expression (`name.where(...)`,
    // form A) or implicit (`where(...)`, form B) where the implicit focus is
    // the current FHIRPath root.
    if let Invocation::Function(term, term_args) = inv
        && term_args.is_empty()
        && (term == "exists" || term == "empty")
    {
        // Form A: `<focus>.where(crit).<term>()`.
        if let Expression::Invocation(inner_base, Invocation::Function(name, args)) = base
            && name == "where"
            && args.len() == 1
        {
            return lower_where_exists(Some(inner_base), &args[0], term == "empty", env);
        }
        // Form B: `where(crit).<term>()` — the parser wraps the leading
        // function call in `Term::Invocation`.
        if let Expression::Term(Term::Invocation(Invocation::Function(name, args))) = base
            && name == "where"
            && args.len() == 1
        {
            return lower_where_exists(None, &args[0], term == "empty", env);
        }
    }

    let base_sql = lower_expression(base, env)?;
    match inv {
        Invocation::Member(name) => extend_path(base_sql, PathStep::Field(name.clone()), env),
        Invocation::Function(name, args) => lower_function_call(&base_sql, name, args, env),
        Invocation::This => Ok(base_sql),
        Invocation::Index | Invocation::Total => Err(SofError::Uncompilable {
            reason: "$index / $total are not yet supported by the in-DB runner".to_string(),
        }),
    }
}

/// Lowers `<base>.where(<crit>).exists()` (and the negated `.empty()` form).
///
/// The base path navigates to a JSON value (typically an array). The criterion
/// is compiled with the iteration alias's `value` field as its FHIRPath root
/// so `name.where(use = 'official')` lowers as
/// `EXISTS(SELECT 1 FROM <unnest of name> w WHERE w.value->>'use' = 'official')`
/// in the dialect's preferred form.
/// Recognises chains of the form
/// `<base>.where(<crit>).<navigation>` (or `extension(url)` as sugar for the
/// same shape — `extension.where(url = <url>)`) and lifts the chain into a
/// [`SqlExpr::WhereScalar`] subquery. Returns `Ok(None)` when the expression
/// doesn't fit the shape so the caller falls back to normal lowering.
///
/// Navigation steps after the lifted where include `.field`, `[N]`, indexed
/// access, `.first()`, and `.ofType(T)` (which lowers via the polymorphic
/// path rewrite the rest of the compiler already handles).
fn try_lower_where_scalar(
    expr: &Expression,
    env: &mut CompileEnv,
) -> Result<Option<SqlExpr>, SofError> {
    let mut steps: Vec<PostStep> = Vec::new();
    let mut cur = expr;
    loop {
        match cur {
            Expression::Invocation(base, Invocation::Member(name)) => {
                steps.push(PostStep::Field(name.clone()));
                cur = base;
            }
            Expression::Invocation(base, Invocation::Function(name, args))
                if name == "first" && args.is_empty() =>
            {
                cur = base;
            }
            Expression::Invocation(base, Invocation::Function(name, args))
                if name == "ofType" && args.len() == 1 =>
            {
                let ty = type_name_from_arg(&args[0])?;
                steps.push(PostStep::OfType(ty));
                cur = base;
            }
            Expression::Indexer(base, idx) => {
                if let Expression::Term(Term::Literal(Literal::Integer(n))) = idx.as_ref() {
                    steps.push(PostStep::Index(*n));
                    cur = base;
                } else {
                    return Ok(None);
                }
            }
            // Found a where call: lift the rest of the chain into a scalar
            // projection over a filtered lateral subquery.
            Expression::Invocation(inner_base, Invocation::Function(name, args))
                if name == "where" && args.len() == 1 =>
            {
                if steps.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(build_where_scalar(
                    inner_base, None, &args[0], steps, env,
                )?));
            }
            // `extension(url)` — sugar for `extension.where(url = url)`.
            // Lifts into a scalar subquery the same way as `where(...)`.
            Expression::Invocation(inner_base, Invocation::Function(name, args))
                if name == "extension" && args.len() == 1 =>
            {
                if steps.is_empty() {
                    return Ok(None);
                }
                let url_arg = args[0].clone();
                return Ok(Some(build_where_scalar(
                    inner_base,
                    Some("extension".to_string()),
                    &url_arg,
                    steps,
                    env,
                )?));
            }
            // `Term::Invocation(Function("extension", [url]))` — same as
            // above but with an implicit base (the FHIRPath root).
            Expression::Term(Term::Invocation(Invocation::Function(name, args)))
                if name == "extension" && args.len() == 1 =>
            {
                if steps.is_empty() {
                    return Ok(None);
                }
                let url_arg = args[0].clone();
                return Ok(Some(build_where_scalar_at_root(
                    Some("extension".to_string()),
                    &url_arg,
                    steps,
                    env,
                )?));
            }
            _ => return Ok(None),
        }
    }
}

/// Builds a WhereScalar IR node from an extracted base path, optional
/// extension-style sugar, criterion, and post-projection steps.
fn build_where_scalar(
    base: &Expression,
    sugar_field: Option<String>,
    crit_or_url: &Expression,
    steps: Vec<PostStep>,
    env: &mut CompileEnv,
) -> Result<SqlExpr, SofError> {
    let is_ext = sugar_field.is_some();
    let mut focus = lower_expression(base, env)?;
    if let Some(field) = sugar_field {
        focus = extend_path(focus, PathStep::Field(field), env)?;
    }
    finish_where_scalar(focus, crit_or_url, steps, is_ext, env)
}

fn build_where_scalar_at_root(
    sugar_field: Option<String>,
    crit_or_url: &Expression,
    steps: Vec<PostStep>,
    env: &mut CompileEnv,
) -> Result<SqlExpr, SofError> {
    let is_ext = sugar_field.is_some();
    let mut focus = SqlExpr::JsonPath {
        root: env.root_alias.clone(),
        path: super::ir::JsonPath::new(),
    };
    if let Some(field) = sugar_field {
        focus = extend_path(focus, PathStep::Field(field), env)?;
    }
    finish_where_scalar(focus, crit_or_url, steps, is_ext, env)
}

fn finish_where_scalar(
    focus: SqlExpr,
    crit_or_url: &Expression,
    steps: Vec<PostStep>,
    is_extension_sugar: bool,
    env: &mut CompileEnv,
) -> Result<SqlExpr, SofError> {
    let alias = format!("w{}", env.next_where_alias);
    env.next_where_alias += 1;
    let prev_root = env.root_alias.clone();
    env.root_alias = format!("{alias}.value");
    let predicate = if is_extension_sugar {
        // Build `url = <arg>` as a SqlExpr: lhs is `<alias>.value->>'url'`,
        // rhs is the lowered argument.
        let url_path = SqlExpr::JsonPath {
            root: env.root_alias.clone(),
            path: super::ir::JsonPath(vec![PathStep::Field("url".to_string())]),
        };
        let rhs = lower_expression(crit_or_url, env);
        let rhs_expr = match rhs {
            Ok(e) => e,
            Err(e) => {
                env.root_alias = prev_root;
                return Err(e);
            }
        };
        SqlExpr::BinOp {
            op: BinOp::Eq,
            lhs: Box::new(url_path),
            rhs: Box::new(rhs_expr),
        }
    } else {
        match lower_expression(crit_or_url, env) {
            Ok(e) => e,
            Err(e) => {
                env.root_alias = prev_root;
                return Err(e);
            }
        }
    };
    // Build the projection by replaying collected steps in reverse.
    let mut projection = SqlExpr::JsonPath {
        root: env.root_alias.clone(),
        path: super::ir::JsonPath::new(),
    };
    for step in steps.into_iter().rev() {
        projection = match step {
            PostStep::Field(name) => extend_path(projection, PathStep::Field(name), env)?,
            PostStep::Index(n) => extend_path(projection, PathStep::Index(n), env)?,
            PostStep::OfType(t) => extend_path(projection, PathStep::OfType(t), env)?,
        };
    }
    env.root_alias = prev_root;
    Ok(SqlExpr::WhereScalar {
        focus: Box::new(focus),
        iter_alias: alias,
        predicate: Box::new(predicate),
        projection: Box::new(projection),
    })
}

#[derive(Debug)]
enum PostStep {
    Field(String),
    Index(i64),
    OfType(String),
}

/// Recognises `<base>.<field>.join(<sep>)` and lifts it to a
/// [`SqlExpr::JoinAggregate`] (string aggregate over a flattened collection).
/// `<base>` is unnested as the outer scope; `<field>` is unnested per element
/// as the inner scope; the inner values are aggregated with the supplied
/// separator (defaults to `''` when no argument is given).
fn try_lower_join_aggregate(
    expr: &Expression,
    env: &mut CompileEnv,
) -> Result<Option<SqlExpr>, SofError> {
    // Outer call must be `Function("join", [sep?])` on `<base>.<field>`.
    let (call_base, sep_arg_opt) = match expr {
        Expression::Invocation(b, Invocation::Function(name, args))
            if name == "join" && args.len() <= 1 =>
        {
            (b.as_ref(), args.first())
        }
        _ => return Ok(None),
    };
    // `<base>.<field>` shape.
    let (outer_base_expr, inner_field) = match call_base {
        Expression::Invocation(b, Invocation::Member(field)) => (b.as_ref(), field.clone()),
        _ => return Ok(None),
    };
    // Separator must be a string literal (or absent → empty).
    let sep = match sep_arg_opt {
        None => String::new(),
        Some(Expression::Term(Term::Literal(Literal::String(s)))) => s.clone(),
        Some(_) => return Ok(None),
    };
    let outer_focus = lower_expression(outer_base_expr, env)?;
    let outer_alias = format!("ja{}", env.next_where_alias);
    env.next_where_alias += 1;
    let inner_alias = format!("ja{}", env.next_where_alias);
    env.next_where_alias += 1;
    Ok(Some(SqlExpr::JoinAggregate {
        outer_focus: Box::new(outer_focus),
        outer_alias,
        inner_field,
        inner_alias,
        separator: sep,
    }))
}

fn lower_where_exists(
    base: Option<&Expression>,
    crit: &Expression,
    negate: bool,
    env: &mut CompileEnv,
) -> Result<SqlExpr, SofError> {
    // Form B (implicit focus, `where(crit).exists()` at top level): the focus
    // is the current single resource, so `where(crit)` returns either
    // `[resource]` or `[]` and `.exists()` collapses to evaluating `crit`
    // against the resource directly. No lateral subquery needed.
    if base.is_none() {
        let predicate = lower_expression(crit, env)?;
        return Ok(if negate {
            SqlExpr::UnaryOp {
                op: UnaryOp::Not,
                inner: Box::new(predicate),
            }
        } else {
            predicate
        });
    }

    // Form A (`<focus>.where(crit).exists()`): build an EXISTS subquery that
    // unnests the focus collection and tests `crit` against each element.
    let focus = lower_expression(base.unwrap(), env)?;
    let alias = format!("w{}", env.next_where_alias);
    env.next_where_alias += 1;
    let prev_root = env.root_alias.clone();
    env.root_alias = format!("{alias}.value");
    let predicate = lower_expression(crit, env);
    env.root_alias = prev_root;
    let predicate = predicate?;
    Ok(SqlExpr::WhereExists {
        focus: Box::new(focus),
        iter_alias: alias,
        predicate: Box::new(predicate),
        negate,
    })
}

fn lower_indexer(
    base: &Expression,
    idx: &Expression,
    env: &mut CompileEnv,
) -> Result<SqlExpr, SofError> {
    let base_sql = lower_expression(base, env)?;
    let idx_n = match idx {
        Expression::Term(Term::Literal(Literal::Integer(n))) => *n,
        // `%name_index` — must resolve to an integer-typed constant. The
        // index is inlined at compile time (SQL JSON path syntax doesn't
        // bind parameters inside the path braces).
        Expression::Term(Term::ExternalConstant(name)) => {
            match env.constants.get(name).map(|c| c.value.clone()) {
                Some(LitValue::Int(n)) => n,
                Some(other) => {
                    return Err(SofError::Uncompilable {
                        reason: format!(
                            "constant '%{name}' used as array index must be an integer (got {other:?})"
                        ),
                    });
                }
                None => {
                    return Err(SofError::InvalidViewDefinition(format!(
                        "FHIRPath references undefined constant '%{name}' \
                         in array index position"
                    )));
                }
            }
        }
        _ => {
            return Err(SofError::Uncompilable {
                reason: "only integer-literal or %integer-constant index expressions are \
                         supported by the in-DB runner"
                    .to_string(),
            });
        }
    };
    extend_path(base_sql, PathStep::Index(idx_n), env)
}

/// Extends an existing path-valued expression with another step. Returns
/// `Uncompilable` if the base is not a path (e.g., a function-call result).
///
/// Supports two extra shapes used by chained-call lifts:
/// - `WhereScalar { projection, .. }` — appends the step to the inner
///   projection so `extension(url).value.ofType(Coding).code` keeps lifting
///   into the same scalar subquery.
///
/// When `step` is an [`PathStep::OfType`] following a [`PathStep::Field`],
/// the pair is collapsed to a single polymorphic-field step (e.g.
/// `value.ofType(Quantity)` → `valueQuantity`) iff the FHIR
/// `StructureDefinition`-derived [`super::lookup_field_type`] table
/// confirms the typed-variant field exists. Parent-context resolution walks
/// the existing path from `env.resource_type`; sub-scope paths (rooted at a
/// `where`/`forEach` iter alias) fall back to a parent-free scan via
/// [`super::field_exists_anywhere`].
fn extend_path(base: SqlExpr, step: PathStep, env: &CompileEnv) -> Result<SqlExpr, SofError> {
    match base {
        SqlExpr::JsonPath { root, mut path } => {
            if let PathStep::OfType(type_name) = &step
                && let Some(PathStep::Field(prev)) = path.0.last()
            {
                let variant = format!("{prev}{}", uppercase_first(type_name));
                if polymorphic_variant_exists(&root, &path, &variant, env) {
                    let last = path.0.len() - 1;
                    path.0[last] = PathStep::Field(variant);
                    return Ok(SqlExpr::JsonPath { root, path });
                }
            }
            path.push(step);
            Ok(SqlExpr::JsonPath { root, path })
        }
        SqlExpr::WhereScalar {
            focus,
            iter_alias,
            predicate,
            projection,
        } => {
            let new_projection = extend_path(*projection, step, env)?;
            Ok(SqlExpr::WhereScalar {
                focus,
                iter_alias,
                predicate,
                projection: Box::new(new_projection),
            })
        }
        other => Err(SofError::Uncompilable {
            reason: format!("cannot extend non-path expression {other:?} with a path step"),
        }),
    }
}

/// Returns true when the FHIR field `variant` (e.g. `valueQuantity`) exists
/// as a typed-variant of the polymorphic field that's the last segment in
/// `path`. Resource-rooted paths consult the FIELD_TYPES table at the exact
/// `(parent, variant)` pair; sub-scope paths scan for the variant name
/// anywhere in the table.
fn polymorphic_variant_exists(
    root: &str,
    path: &JsonPath,
    variant: &str,
    env: &CompileEnv,
) -> bool {
    match parent_type_of_last_field(root, path, env) {
        Some(parent) => super::lookup_field_type(env.fhir_version, &parent, variant).is_some(),
        None => super::field_exists_anywhere(env.fhir_version, variant),
    }
}

/// Walks `path` from `env.resource_type` through the FIELD_TYPES table to
/// determine the FHIR parent type of the last [`PathStep::Field`]. Returns
/// `None` when the path's root isn't the resource document, the resource
/// type is unset, or any intermediate segment can't be resolved (unknown
/// field, type-filter step, etc.).
fn parent_type_of_last_field(root: &str, path: &JsonPath, env: &CompileEnv) -> Option<String> {
    if root != RESOURCE_ROOT || env.resource_type.is_empty() {
        return None;
    }
    let last_field_pos = path
        .0
        .iter()
        .rposition(|s| matches!(s, PathStep::Field(_)))?;
    let mut parent = env.resource_type.clone();
    for step in &path.0[..last_field_pos] {
        match step {
            PathStep::Field(name) => {
                let (ty, _) = super::lookup_field_type(env.fhir_version, &parent, name)?;
                parent = ty.to_string();
            }
            // Indexing into a collection returns an element of the same type.
            PathStep::Index(_) => {}
            // A surviving `OfType` step means the previous polymorphic-rewrite
            // attempt didn't fire — treat the step as a type cast and adopt
            // the casted type as the new parent.
            PathStep::OfType(t) => parent = t.clone(),
            PathStep::TypeFilter(_) => return None,
        }
    }
    Some(parent)
}

fn uppercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// ============================================================================
// Function calls
// ============================================================================

fn lower_function_call(
    focus: &SqlExpr,
    name: &str,
    args: &[Expression],
    env: &mut CompileEnv,
) -> Result<SqlExpr, SofError> {
    match name {
        "exists" if args.is_empty() => Ok(SqlExpr::UnaryOp {
            op: UnaryOp::IsNotNull,
            inner: Box::new(focus.clone()),
        }),
        "empty" if args.is_empty() => Ok(SqlExpr::UnaryOp {
            op: UnaryOp::IsNull,
            inner: Box::new(focus.clone()),
        }),
        "not" if args.is_empty() => Ok(SqlExpr::UnaryOp {
            op: UnaryOp::Not,
            inner: Box::new(focus.clone()),
        }),
        "first" if args.is_empty() => {
            // `path.first()` — append `[0]` to the focus's JsonPath so
            // subsequent navigation reads the first element. For scalar
            // (non-array) values, `[0]` returns NULL in SQLite/PG; the
            // emitter wraps multi-Field column paths with a `coalesce`
            // fallback to plain navigation, so common cases still work.
            match focus {
                SqlExpr::JsonPath { root, path } => {
                    let mut new_path = path.clone();
                    new_path.push(PathStep::Index(0));
                    Ok(SqlExpr::JsonPath {
                        root: root.clone(),
                        path: new_path,
                    })
                }
                _ => Ok(focus.clone()),
            }
        }
        "last" if args.is_empty() => {
            // Without knowing the array length at compile time, last()
            // requires a runtime aggregate. Defer to a future stage; for now
            // treat as identity so scalar/singleton-array cases work.
            Ok(focus.clone())
        }
        "iif" if (args.len() == 2 || args.len() == 3) => {
            let cond = lower_expression(&args[0], env)?;
            let then_expr = lower_expression(&args[1], env)?;
            let else_expr = if args.len() == 3 {
                Some(Box::new(lower_expression(&args[2], env)?))
            } else {
                None
            };
            Ok(SqlExpr::Case {
                arms: vec![(cond, then_expr)],
                else_: else_expr,
            })
        }
        "ofType" if args.len() == 1 => {
            let ty = type_name_from_arg(&args[0])?;
            extend_path(focus.clone(), PathStep::OfType(ty), env)
        }
        "getResourceKey" if args.is_empty() => {
            // Per SoF v2: returns the resource's id. The focus is the
            // resource document; navigate `id` off it.
            extend_path(focus.clone(), PathStep::Field("id".to_string()), env)
        }
        "getReferenceKey" if args.is_empty() => {
            // `Reference.reference` looks like `Type/id`; extract the id —
            // the substring after the LAST `/`. The simplest portable form
            // applies to both PG `regexp_replace` (POSIX) and SQLite's
            // built-in `instr`/`substr` shimmed via a registered UDF in
            // stage 6; for now both dialects use a SQL-only expression.
            let reference =
                extend_path(focus.clone(), PathStep::Field("reference".to_string()), env)?;
            Ok(SqlExpr::ReferenceKey {
                reference: Box::new(reference),
                expected_type: None,
            })
        }
        "getReferenceKey" if args.len() == 1 => {
            let expected = type_name_from_arg(&args[0])?;
            let reference =
                extend_path(focus.clone(), PathStep::Field("reference".to_string()), env)?;
            Ok(SqlExpr::ReferenceKey {
                reference: Box::new(reference),
                expected_type: Some(expected),
            })
        }
        "count" if args.is_empty() => {
            // For a scalar focus, count is `1` when present and `0` when
            // empty. Multi-element collection focuses (e.g. inside chained
            // `where()` lateral subqueries) lower to a count subquery in
            // stage 5; the conformance corpus uses `.count()` mostly on
            // scalar paths.
            Ok(SqlExpr::Case {
                arms: vec![(
                    SqlExpr::UnaryOp {
                        op: UnaryOp::IsNotNull,
                        inner: Box::new(focus.clone()),
                    },
                    SqlExpr::Lit(LitValue::Int(1)),
                )],
                else_: Some(Box::new(SqlExpr::Lit(LitValue::Int(0)))),
            })
        }
        "join" if args.len() <= 1 => {
            // Scalar focus: `join(sep)` is identity (the lone value is its
            // own join result). For arrays, lowers to `string_agg` /
            // `group_concat` over a lateral subquery — added when
            // collection-flow infrastructure lands.
            Ok(focus.clone())
        }
        "extension" if args.len() == 1 => {
            // `<focus>.extension(<url>)` — sugar for filtered-extension
            // navigation. Lifts to a WhereScalar over the focus's
            // `.extension` array, projecting the matched element. Used
            // when followed by further navigation (`.value.ofType(...)`)
            // or chained as `extension(...).extension(...)`.
            let alias = format!("w{}", env.next_where_alias);
            env.next_where_alias += 1;
            let ext_focus =
                extend_path(focus.clone(), PathStep::Field("extension".to_string()), env)?;
            let prev_root = env.root_alias.clone();
            env.root_alias = format!("{alias}.value");
            let url_path = SqlExpr::JsonPath {
                root: env.root_alias.clone(),
                path: super::ir::JsonPath(vec![PathStep::Field("url".to_string())]),
            };
            let url_arg = lower_expression(&args[0], env);
            let projection = SqlExpr::JsonPath {
                root: env.root_alias.clone(),
                path: super::ir::JsonPath::new(),
            };
            env.root_alias = prev_root;
            let url_arg = url_arg?;
            Ok(SqlExpr::WhereScalar {
                focus: Box::new(ext_focus),
                iter_alias: alias,
                predicate: Box::new(SqlExpr::BinOp {
                    op: BinOp::Eq,
                    lhs: Box::new(url_path),
                    rhs: Box::new(url_arg),
                }),
                projection: Box::new(projection),
            })
        }
        "lowBoundary" if args.is_empty() => Ok(SqlExpr::Boundary {
            side: BoundarySide::Low,
            kind: boundary_kind_from_hint(env)?,
            source: Box::new(focus.clone()),
        }),
        "highBoundary" if args.is_empty() => Ok(SqlExpr::Boundary {
            side: BoundarySide::High,
            kind: boundary_kind_from_hint(env)?,
            source: Box::new(focus.clone()),
        }),
        // Stage 5+ adds the rest (where(crit), select(expr), exists(crit),
        // extension(), boundary fns).
        other => Err(SofError::Uncompilable {
            reason: format!(
                "FHIRPath function {other}({}) is not yet supported by the in-DB runner",
                args.len()
            ),
        }),
    }
}

/// Extracts the type name from a single-argument `ofType(T)` call. The parser
/// allows `T` to be parsed as a member-access term (the simplest shape) or a
/// type literal.
fn type_name_from_arg(arg: &Expression) -> Result<String, SofError> {
    match arg {
        Expression::Term(Term::Invocation(Invocation::Member(name))) => Ok(name.clone()),
        _ => Err(SofError::Uncompilable {
            reason: format!("ofType() argument must be a bare type identifier (got {arg:?})"),
        }),
    }
}

/// Picks the [`BoundaryKind`] for the current column based on its declared
/// `column.type`. Required because the FHIRPath compiler can't reliably
/// infer the source's value type after the polymorphic-field rewrite has
/// collapsed `value.ofType(X)` into `valueX`.
fn boundary_kind_from_hint(env: &CompileEnv) -> Result<BoundaryKind, SofError> {
    match env.column_type_hint.as_deref() {
        Some("decimal") | Some("integer") | Some("positiveInt") | Some("unsignedInt") => {
            Ok(BoundaryKind::Decimal)
        }
        Some("date") => Ok(BoundaryKind::Date),
        Some("dateTime") | Some("instant") => Ok(BoundaryKind::DateTime),
        Some("time") => Ok(BoundaryKind::Time),
        Some(other) => Err(SofError::Uncompilable {
            reason: format!(
                "lowBoundary()/highBoundary() requires a column.type of decimal/date/dateTime/time \
                 to disambiguate the source value type (got '{other}')"
            ),
        }),
        None => Err(SofError::Uncompilable {
            reason: "lowBoundary()/highBoundary() requires the enclosing column to declare a \
                     `type` so the compiler can pick decimal vs. date/dateTime/time semantics"
                .to_string(),
        }),
    }
}

/// Resolves a `%name` external-constant reference to a typed
/// [`SqlExpr::Param`].
///
/// Each constant is bound to a single SQL parameter slot on first reference
/// and reused on subsequent references in the same compilation. The runner
/// receives the resolved values via [`CompileEnv::param_bindings`].
fn resolve_external_constant(name: &str, env: &mut CompileEnv) -> Result<SqlExpr, SofError> {
    let constant = env.constants.get(name).cloned().ok_or_else(|| {
        SofError::InvalidViewDefinition(format!(
            "FHIRPath references undefined constant '%{name}' (not declared in ViewDefinition.constant[])"
        ))
    })?;
    if let Some(idx) = constant.bound_to {
        return Ok(SqlExpr::Param(idx));
    }
    let idx = env.next_param;
    env.next_param += 1;
    env.param_bindings.push(constant.value.clone());
    if let Some(slot) = env.constants.get_mut(name) {
        slot.bound_to = Some(idx);
    }
    Ok(SqlExpr::Param(idx))
}

fn lower_type_op(
    expr: &Expression,
    op: &str,
    ts: &TypeSpecifier,
    env: &mut CompileEnv,
) -> Result<SqlExpr, SofError> {
    let TypeSpecifier::QualifiedIdentifier(a, b) = ts;
    let type_name = match b {
        Some(t) => t.clone(),
        None => a.clone(),
    };
    let base = lower_expression(expr, env)?;
    match op {
        "is" => {
            // For the conformance subset, `x is T` reduces to "x has the
            // appropriate JSON type" — full implementation lands in stage 4.
            let _ = base;
            let _ = type_name;
            Err(SofError::Uncompilable {
                reason: "'is' operator is not yet implemented in the in-DB runner".to_string(),
            })
        }
        "as" => extend_path(base, PathStep::OfType(type_name), env),
        other => Err(SofError::Uncompilable {
            reason: format!("unsupported type operator '{other}'"),
        }),
    }
}

// JsonType is consumed by later stages (TypeFilter / has_json_type lowering).
const _: Option<JsonType> = None;
// SqlType is consumed by Cast lowering in column type projection.
const _: Option<SqlType> = None;
