//! MongoDB aggregation-pipeline emitter for the SQL-on-FHIR compiler.
//!
//! Lowers the dialect-agnostic [`PlanNode`] IR (produced by
//! [`build_plan`](super::compile_view::build_plan)) into a MongoDB aggregation
//! pipeline (`Vec<bson::Document>`), the analogue of the SQL emitter in
//! [`emit`](super::emit). Resources are stored as native BSON sub-documents
//! under the `data` field, so a FHIR path `name.family` becomes navigation under
//! `$data`.
//!
//! Coverage is at parity with the SQL emitter for the SQL-on-FHIR v2 conformance
//! corpus: scans, `forEach`/`forEachOrNull` (with `%rowIndex`), top-level and
//! mid-path `where()` (`$filter`), `unionAll` (`$facet`), `repeat:`
//! (`$function` recursive traversal), `join()`, collection columns, the
//! boundary functions, `getReferenceKey`, and three-valued comparison.
//! A `repeat:` may sit anywhere in the stage chain (under a `forEach`, under
//! another `repeat`, or beside siblings): each one seeds its traversal from
//! its enclosing scope. `unionAll` is only lowered at the plan root; the
//! compiler distributes nested unions up to it. Constructs the compiler still
//! does not cover return [`SofError::Uncompilable`] — surfaced by the handler
//! as `422` — exactly as they do on the SQL backends.
//!
//! Numbers are stored as serde_json `arbitrary_precision` wrapper objects (see
//! [`coerce_number`]); `repeat:` uses server-side JavaScript, which must be
//! enabled on the server.
//!
//! ## Root-alias mapping
//!
//! The IR addresses values through SQL-style roots: `r.data` (the scanned
//! resource), `fe.value` / `fe2.value` (lateral-unnest iteration rows). Each
//! maps to a MongoDB field: `r.data` → `data`; `feN.value` → a synthetic
//! top-level field `__feN` that the emitter `$set`s to the unnest source and
//! then `$unwind`s; `rec_N.node` → `__rec_N`, the unwound `repeat:` node.

use mongodb::bson::{Bson, Document, doc};

use crate::core::sof_runner::SofError;

use super::ir::{
    BinOp, BoundaryKind, BoundarySide, JsonPath, LitValue, PathStep, PlanNode, RowIndexScope,
    SqlExpr, UnaryOp, nearest_left_scope,
};

/// Output of [`emit_mongo`]: a ready-to-run pipeline plus its output columns.
pub struct EmittedMongo {
    /// Aggregation stages. The leading `$match` constrains `resource_type` and
    /// `is_deleted`; the runner prepends the tenant/`since` predicates.
    pub pipeline: Vec<Document>,
    /// Output column names, in `select` order.
    pub columns: Vec<String>,
}

fn uncompilable(reason: impl Into<String>) -> SofError {
    SofError::Uncompilable {
        reason: reason.into(),
    }
}

/// Lowers a plan tree into a MongoDB aggregation pipeline.
///
/// `constants` holds the resolved `ViewDefinition.constant[]` values; the
/// emitter inlines them where the IR references `Param(n)` (n ≥ 3).
pub fn emit_mongo(plan: &PlanNode, constants: &[LitValue]) -> Result<EmittedMongo, SofError> {
    match plan {
        PlanNode::Project { .. } => {
            let (pipeline, columns) = emit_project_pipeline(plan, constants)?;
            Ok(EmittedMongo { pipeline, columns })
        }
        PlanNode::Union(branches) => emit_union_mongo(branches, constants),
        _ => Err(uncompilable(
            "Mongo emitter expects a Project or Union at the plan root",
        )),
    }
}

/// Lowers a single `Project` plan into a full pipeline (`[$match, …, $project]`)
/// plus its output column names.
fn emit_project_pipeline(
    plan: &PlanNode,
    constants: &[LitValue],
) -> Result<(Vec<Document>, Vec<String>), SofError> {
    let PlanNode::Project { parent, columns } = plan else {
        return Err(uncompilable("expected a Project plan"));
    };

    let mut pipeline = Vec::new();
    lower_source(parent, &mut pipeline, constants)?;

    // Final projection. Column names come verbatim from `column[].name`.
    let mut project = doc! { "_id": 0i32 };
    let mut names = Vec::with_capacity(columns.len());
    for column in columns {
        validate_column_name(&column.name)?;
        let expr = lower_expr(&column.expr, &Ctx::root(constants))?;
        // An absent value must still produce the key (as `null`), like the
        // SQL runners; `$project` would otherwise drop it.
        project.insert(column.name.clone(), doc! { "$ifNull": [expr, Bson::Null] });
        names.push(column.name.clone());
    }
    pipeline.push(doc! { "$project": project });

    Ok((pipeline, names))
}

/// Lowers a top-level `unionAll` (`PlanNode::Union`) into a `$facet` that runs
/// each branch sub-pipeline over the shared (tenant + resource-type filtered)
/// input, then concatenates and unwinds the branch outputs.
///
/// `$facet` is used rather than `$unionWith` because the branch sub-pipelines
/// must observe the same tenant scope, which the runner injects only on the
/// top-level pipeline's leading `$match`.
fn emit_union_mongo(
    branches: &[PlanNode],
    constants: &[LitValue],
) -> Result<EmittedMongo, SofError> {
    if branches.is_empty() {
        return Err(SofError::InvalidViewDefinition(
            "unionAll branches list is empty".to_string(),
        ));
    }

    let mut facet = Document::new();
    let mut concat: Vec<Bson> = Vec::with_capacity(branches.len());
    let mut shared_scan: Option<Document> = None;
    let mut columns: Option<Vec<String>> = None;

    for (i, branch) in branches.iter().enumerate() {
        let (mut pipe, cols) = emit_project_pipeline(branch, constants)?;
        match &columns {
            None => columns = Some(cols.clone()),
            Some(expected) if *expected != cols => {
                return Err(uncompilable(format!(
                    "unionAll branches produce different column schemas: {expected:?} vs {cols:?}"
                )));
            }
            _ => {}
        }
        // The leading `$match` (resource_type/is_deleted) is identical across
        // branches and is hoisted ahead of the `$facet`; the remaining stages
        // become the branch sub-pipeline.
        if pipe.is_empty() {
            return Err(uncompilable("unionAll branch produced an empty pipeline"));
        }
        let lead = pipe.remove(0);
        if shared_scan.is_none() {
            shared_scan = Some(lead);
        }
        let field = format!("b{i}");
        concat.push(Bson::String(format!("${field}")));
        facet.insert(field, pipe);
    }

    let mut pipeline = vec![shared_scan.expect("at least one branch")];
    pipeline.push(doc! { "$facet": facet });
    pipeline.push(doc! { "$project": { UNION_FIELD: { "$concatArrays": concat } } });
    pipeline.push(doc! { "$unwind": format!("${UNION_FIELD}") });
    pipeline.push(doc! { "$replaceRoot": { "newRoot": format!("${UNION_FIELD}") } });

    Ok(EmittedMongo {
        pipeline,
        columns: columns.expect("at least one branch"),
    })
}

/// Scratch field used to concatenate `$facet` branch outputs before unwinding.
const UNION_FIELD: &str = "__union";

/// Walks the row-source chain (Scan → unnests/filters), appending pipeline
/// stages parent-first so the scan `$match` lands first.
fn lower_source(
    plan: &PlanNode,
    pipeline: &mut Vec<Document>,
    constants: &[LitValue],
) -> Result<(), SofError> {
    match plan {
        PlanNode::Scan { resource_type, .. } => {
            pipeline.push(doc! {
                "$match": { "resource_type": resource_type.clone(), "is_deleted": false }
            });
            Ok(())
        }
        PlanNode::LateralUnnest {
            parent,
            source,
            out_alias,
            left_join,
            on_filter,
            flat_index,
            ..
        } => {
            lower_source(parent, pipeline, constants)?;
            let ctx = Ctx::root(constants);
            let base = unnest_base(out_alias);
            let source_expr = lower_unnest_source(source, &ctx)?;

            // Trailing-`[N]` forEach (`forEach: "contact.telecom[0]"`) selects a
            // single element of the FLATTENED collection rather than iterating
            // it: flatten the (possibly multi-field) source path, take element
            // N, and skip the `$unwind`. `forEach` (not OrNull) drops rows where
            // the element is absent.
            if let Some(idx) = flat_index {
                let mut flattened = match source {
                    SqlExpr::JsonPath { root, path } => lower_collection_agg(root, path, &ctx)?,
                    _ => ensure_array(source_expr),
                };
                // `forEach: "x.where(crit)[N]"` / `.first()` indexes the
                // FILTERED collection: apply the criterion (rooted at
                // `<out_alias>.value`) to each element before picking one.
                if let Some(filter) = on_filter {
                    let var = scope_var(out_alias);
                    let child = ctx.child(format!("{out_alias}.value"), var.clone());
                    let cond = truthy(lower_expr(filter, &child)?);
                    flattened = doc! {
                        "$filter": { "input": flattened, "as": var, "cond": cond }
                    }
                    .into();
                }
                let elem: Bson = doc! { "$arrayElemAt": [flattened, *idx] }.into();
                pipeline.push(doc! { "$set": { &base: elem } });
                if !*left_join {
                    // Below an EMPTY `forEachOrNull` the null row survives.
                    let optional = match source {
                        SqlExpr::JsonPath { root, .. } => nearest_left_scope(parent, root),
                        _ => None,
                    };
                    match optional {
                        Some((left, _)) => push_optional_scope_guard(pipeline, &base, Some(&left)),
                        None => pipeline.push(doc! {
                            "$match": {
                                "$expr": doc! { "$ne": [doc! { "$ifNull": [format!("${base}"), Bson::Null] }, Bson::Null] }
                            }
                        }),
                    }
                }
                return Ok(());
            }

            // A multi-field source (a flattened `forEachOrNull` chain) iterates
            // the FLATTENED collection, so the null row only appears when the
            // whole collection is empty.
            let field_steps = match source {
                SqlExpr::JsonPath { path, .. } => path
                    .0
                    .iter()
                    .filter(|s| matches!(s, PathStep::Field(_)))
                    .count(),
                _ => 0,
            };
            let source_expr = match source {
                SqlExpr::JsonPath { root, path } if field_steps > 1 => {
                    lower_collection_agg(root, path, &ctx)?
                }
                _ => source_expr,
            };
            // `forEachOrNull: "x.where(crit)"` filters the array BEFORE the
            // `$unwind`, so a non-empty array whose elements all fail the
            // criterion still yields the preserved null row (the SQL runners'
            // `LEFT JOIN ... ON <crit>` behaves the same way).
            let source_expr = match on_filter {
                Some(filter) if *left_join => {
                    let var = scope_var(out_alias);
                    let child = ctx.child(format!("{out_alias}.value"), var.clone());
                    let cond = truthy(lower_expr(filter, &child)?);
                    doc! {
                        "$filter": { "input": ensure_array(source_expr), "as": var, "cond": cond }
                    }
                    .into()
                }
                _ => source_expr,
            };
            pipeline.push(doc! { "$set": { &base: source_expr } });
            // `includeArrayIndex` exposes the 0-based element position for
            // `%rowIndex` (see `lower_expr`'s `RowIndex` arm). It is `null` for a
            // `forEachOrNull` miss (empty/missing array), which `%rowIndex` maps
            // to 0. The field is dropped by the final inclusion `$project`.
            // A plain `forEach` below an EMPTY `forEachOrNull` keeps the null row.
            let optional = if *left_join {
                None
            } else {
                match source {
                    SqlExpr::JsonPath { root, .. } => {
                        nearest_left_scope(parent, root).map(|(a, _)| a)
                    }
                    _ => None,
                }
            };
            pipeline.push(doc! {
                "$unwind": {
                    "path": format!("${base}"),
                    "preserveNullAndEmptyArrays": *left_join || optional.is_some(),
                    "includeArrayIndex": unnest_index_field(out_alias),
                }
            });
            push_optional_scope_guard(pipeline, &base, optional.as_deref());
            // `forEach: "x.where(crit)"` — filter the unwound elements by the
            // criterion (rooted at `<out_alias>.value` → the `$__<out_alias>`
            // field). `forEachOrNull` was already filtered above.
            if let Some(filter) = on_filter
                && !*left_join
            {
                let pred = truthy(lower_expr(filter, &ctx)?);
                pipeline.push(doc! { "$match": { "$expr": pred } });
            }
            Ok(())
        }
        PlanNode::Filter { parent, predicate } => {
            lower_source(parent, pipeline, constants)?;
            let pred = truthy(lower_expr(predicate, &Ctx::root(constants))?);
            pipeline.push(doc! { "$match": { "$expr": pred } });
            Ok(())
        }
        PlanNode::Project { .. } => Err(uncompilable(
            "nested Project is not supported by the Mongo emitter",
        )),
        PlanNode::Union(_) => Err(uncompilable(
            "unionAll branch nested inside a forEach is not supported by the Mongo emitter",
        )),
        PlanNode::Recurse {
            parent,
            seed_root,
            step_paths,
            out_alias,
        } => {
            lower_source(parent, pipeline, constants)?;
            // The traversal starts at the enclosing scope: the resource, a
            // `forEach` element or the node of an outer `repeat`. Every scope
            // is a flat top-level field, so any chain position resolves.
            let seed = root_ref(seed_root, &Ctx::root(constants))?;
            // Field-name lists per step path (Index/OfType steps are ignored —
            // `repeat:` paths are simple field chains).
            let paths: Vec<Vec<String>> = step_paths
                .iter()
                .map(|p| {
                    p.0.iter()
                        .filter_map(|s| match s {
                            PathStep::Field(n) => Some(n.clone()),
                            _ => None,
                        })
                        .collect()
                })
                .collect();
            let base = unnest_base(out_alias);
            // `$function` walks the step paths recursively from the seed,
            // emitting every descendant node in pre-order — the same
            // traversal as the SQL recursive CTE. A missing seed (e.g. a
            // `forEachOrNull` miss) yields no nodes.
            let traversed: Bson = doc! {
                "$function": {
                    "body": REPEAT_TRAVERSE_JS,
                    "args": [seed, doc! { "$literal": bson_paths(&paths) }],
                    "lang": "js",
                }
            }
            .into();
            pipeline.push(doc! { "$set": { &base: traversed } });
            let optional = nearest_left_scope(parent, seed_root).map(|(a, _)| a);
            pipeline.push(doc! {
                "$unwind": {
                    "path": format!("${base}"),
                    "preserveNullAndEmptyArrays": optional.is_some(),
                    "includeArrayIndex": unnest_index_field(out_alias),
                }
            });
            push_optional_scope_guard(pipeline, &base, optional.as_deref());
            Ok(())
        }
    }
}

/// After an `$unwind` that preserves empties for a step below a
/// `forEachOrNull`: keep a row when the step matched OR that ancestor missed.
fn push_optional_scope_guard(pipeline: &mut Vec<Document>, base: &str, left_alias: Option<&str>) {
    let Some(left) = left_alias else { return };
    let null_of = |field: String| doc! { "$ifNull": [format!("${field}"), Bson::Null] };
    pipeline.push(doc! {
        "$match": { "$expr": { "$or": [
            doc! { "$ne": [null_of(base.to_string()), Bson::Null] },
            doc! { "$eq": [null_of(unnest_base(left)), Bson::Null] },
        ] } }
    });
}

/// Step paths as a nested BSON array for the `$function` argument.
fn bson_paths(paths: &[Vec<String>]) -> Bson {
    Bson::Array(
        paths
            .iter()
            .map(|p| Bson::Array(p.iter().map(|f| Bson::String(f.clone())).collect()))
            .collect(),
    )
}

/// Server-side JS for `repeat:` — recursive pre-order traversal of the step
/// paths from `root`, returning every descendant object node (the root itself
/// is excluded). Mirrors the reference `recursiveTraverse`.
const REPEAT_TRAVERSE_JS: &str = r#"
function(root, paths) {
  var out = [];
  function visit(node) {
    for (var i = 0; i < paths.length; i++) {
      var cur = [node];
      var fields = paths[i];
      for (var j = 0; j < fields.length; j++) {
        var next = [];
        for (var k = 0; k < cur.length; k++) {
          var c = cur[k];
          if (c && typeof c === 'object') {
            var v = c[fields[j]];
            if (Array.isArray(v)) { next = next.concat(v); }
            else if (v !== undefined && v !== null) { next.push(v); }
          }
        }
        cur = next;
      }
      for (var m = 0; m < cur.length; m++) {
        var child = cur[m];
        if (child && typeof child === 'object') { out.push(child); visit(child); }
      }
    }
  }
  if (root && typeof root === 'object') { visit(root); }
  return out;
}
"#;

/// Synthetic top-level field a `feN.value` iteration row is bound to.
fn unnest_base(out_alias: &str) -> String {
    format!("__{out_alias}")
}

/// Synthetic top-level field holding a `forEach` iteration's 0-based index
/// (populated by `$unwind`'s `includeArrayIndex`). Read by `%rowIndex`.
fn unnest_index_field(out_alias: &str) -> String {
    format!("__{out_alias}__idx")
}

/// Lowering context: the inlined `ViewDefinition.constant[]` values plus a
/// stack of iteration-variable bindings.
///
/// `scope` maps an IR root alias (e.g. `w0.value`, produced by a `where()`
/// criterion or `join()`/collection sub-iteration) to a MongoDB aggregation
/// variable name, so a navigation rooted there lowers to `$$<var>` rather than
/// a top-level field.
struct Ctx<'a> {
    constants: &'a [LitValue],
    scope: Vec<(String, String)>,
}

impl<'a> Ctx<'a> {
    fn root(constants: &'a [LitValue]) -> Self {
        Self {
            constants,
            scope: Vec::new(),
        }
    }

    /// Returns a child context with `alias` bound to aggregation variable `var`.
    fn child(&self, alias: impl Into<String>, var: impl Into<String>) -> Ctx<'a> {
        let mut scope = self.scope.clone();
        scope.push((alias.into(), var.into()));
        Ctx {
            constants: self.constants,
            scope,
        }
    }

    /// Variable bound to `root`, if any (innermost binding wins).
    fn var(&self, root: &str) -> Option<&str> {
        self.scope
            .iter()
            .rev()
            .find(|(a, _)| a == root)
            .map(|(_, v)| v.as_str())
    }
}

/// `$<field>` (or `$$<var>`) reference for an IR root alias.
///
/// `r.data` → `$data`; `feN.value` → `$__feN`; a scoped alias → its `$$var`.
fn root_ref(root: &str, ctx: &Ctx) -> Result<Bson, SofError> {
    if let Some(var) = ctx.var(root) {
        return Ok(Bson::String(format!("$${var}")));
    }
    if root == "r.data" {
        return Ok(Bson::String("$data".to_string()));
    }
    if let Some(alias) = root.strip_suffix(".value")
        && alias.starts_with("fe")
    {
        return Ok(Bson::String(format!("${}", unnest_base(alias))));
    }
    // Recursive-traversal node: `<alias>.node` → the unwound `$__<alias>` field.
    if let Some(alias) = root.strip_suffix(".node") {
        return Ok(Bson::String(format!("${}", unnest_base(alias))));
    }
    Err(uncompilable(format!("unsupported value root '{root}'")))
}

/// A bare field or variable reference (`"$data"`, `"$__fe0"`, `"$$v_w0"`) or a
/// `$literal`: cheap to evaluate again, so it is never bound with `$let`.
fn is_cheap(v: &Bson) -> bool {
    match v {
        Bson::String(s) => s.starts_with('$'),
        Bson::Document(d) => d.len() == 1 && d.contains_key("$literal"),
        Bson::Null | Bson::Boolean(_) | Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_) => true,
        _ => false,
    }
}

/// Evaluates `value` once: MongoDB does not share common subexpressions, so an
/// operand used more than once by `build` is bound with `$let` to `var` and
/// `build` receives the `$$var` reference. Cheap operands pass through. `var`
/// must start with a lowercase letter and differ from the emitter's own
/// variables (`this`, `value`, `ns`, `obj`, `j`, `c`, `sf`, `v_<alias>`).
fn bind_once(value: Bson, var: &str, build: impl FnOnce(Bson) -> Bson) -> Bson {
    if is_cheap(&value) {
        return build(value);
    }
    let body = build(Bson::String(format!("$${var}")));
    doc! { "$let": { "vars": { var: value }, "in": body } }.into()
}

/// Coerces a value to an array: arrays pass through, null/missing → `[]`, and a
/// scalar → a one-element array (the FHIRPath scalar-as-singleton model).
fn ensure_array(v: Bson) -> Bson {
    bind_once(v, "cse_arr", |v| {
        doc! {
            "$cond": [
                doc! { "$isArray": v.clone() },
                v.clone(),
                doc! {
                    "$cond": [
                        doc! { "$eq": [doc! { "$ifNull": [v.clone(), Bson::Null] }, Bson::Null] },
                        Bson::Array(Vec::new()),
                        [v],
                    ]
                },
            ]
        }
        .into()
    })
}

/// Lowers a `forEach` source path to the array expression to unwind.
///
/// Unlike column navigation this does NOT first-element-flatten: `forEach`
/// iterates the whole collection. `build_plan` splits multi-segment paths into
/// one unnest per field, so each source carries a single `Field`.
fn lower_unnest_source(source: &SqlExpr, ctx: &Ctx) -> Result<Bson, SofError> {
    let SqlExpr::JsonPath { root, path } = source else {
        return Err(uncompilable("forEach source must be a JSON path"));
    };
    let mut acc = root_ref(root, ctx)?;
    for step in &path.0 {
        match step {
            PathStep::Field(name) => {
                acc = doc! { "$getField": { "field": name.clone(), "input": acc } }.into();
            }
            PathStep::Index(n) => {
                acc = doc! { "$arrayElemAt": [acc, *n] }.into();
            }
            // Polymorphic type guards are collapsed during AST lowering; treat
            // any residual as no-op navigation (matching the SQL emitter).
            PathStep::OfType(_) | PathStep::TypeFilter(_) => {}
        }
    }
    Ok(acc)
}

/// Lowers a JSON navigation used as a scalar column value, replicating the SQL
/// emitter's first-element flattening: at each `Field` step, if the current
/// focus is an array, descend into its first element before reading the field.
fn lower_json_path(root: &str, path: &JsonPath, ctx: &Ctx) -> Result<Bson, SofError> {
    let mut acc = root_ref(root, ctx)?;
    for step in &path.0 {
        match step {
            PathStep::Field(name) => {
                acc = bind_once(acc, "cse_nav", |acc| {
                    let flattened = doc! {
                        "$cond": [
                            doc! { "$isArray": acc.clone() },
                            doc! { "$first": acc.clone() },
                            acc,
                        ]
                    };
                    doc! { "$getField": { "field": name.clone(), "input": flattened } }.into()
                });
            }
            PathStep::Index(n) => {
                acc = index_step(acc, *n);
            }
            PathStep::OfType(_) | PathStep::TypeFilter(_) => {}
        }
    }
    Ok(acc)
}

/// Indexes into a value, mirroring SQLite/PG `[n]` semantics over the
/// scalar-as-singleton model: on an array, take element `n`; on a non-array
/// scalar, `[0]` is the scalar itself and `[n>0]` is null. A bare
/// `$arrayElemAt` would raise a runtime error on a non-array (e.g. the scalar
/// produced by `name.family.first()`), so the array case is guarded.
fn index_step(acc: Bson, n: i64) -> Bson {
    bind_once(acc, "cse_nav", |acc| {
        let non_array = if n == 0 { acc.clone() } else { Bson::Null };
        doc! {
            "$cond": [
                doc! { "$isArray": acc.clone() },
                doc! { "$arrayElemAt": [acc, n] },
                non_array,
            ]
        }
        .into()
    })
}

/// Lowers a `LitValue` to a BSON literal value.
fn lit_to_bson(v: &LitValue) -> Bson {
    match v {
        LitValue::Null => Bson::Null,
        LitValue::Bool(b) => Bson::Boolean(*b),
        LitValue::Int(i) => Bson::Int64(*i),
        LitValue::Decimal(s) => s
            .parse::<f64>()
            .map(Bson::Double)
            .unwrap_or_else(|_| Bson::String(s.clone())),
        LitValue::Str(s) => Bson::String(s.clone()),
    }
}

/// Wraps a value as a `$literal` so leading-`$` strings aren't read as field
/// references.
fn literal_expr(v: &LitValue) -> Bson {
    doc! { "$literal": lit_to_bson(v) }.into()
}

/// serde_json's `arbitrary_precision` representation (enabled transitively via
/// `rust_decimal`) serialises every JSON number as the object
/// `{ "$serde_json::private::Number": "<digits>" }`, so `bson::to_bson` stores
/// FHIR numbers as sub-documents rather than BSON numbers. Navigation therefore
/// yields an object that breaks comparison/arithmetic; these helpers recover the
/// numeric value (or its precise textual form) where it matters. Plain column
/// projection is unaffected — the wrapper round-trips back to a JSON number.
const NUM_KEY: &str = "$serde_json::private::Number";

/// The digit-string inside a number wrapper, or null if `expr` isn't one.
/// The `$getField` is guarded so a non-object input doesn't raise.
fn number_string(expr: Bson) -> Bson {
    doc! {
        "$let": {
            "vars": {
                "obj": doc! {
                    "$cond": [
                        doc! { "$eq": [doc! { "$type": expr.clone() }, "object"] },
                        expr,
                        Bson::Document(Document::new()),
                    ]
                }
            },
            // `field` must be wrapped in `$literal` because the key begins with `$`.
            "in": doc! { "$getField": { "field": doc! { "$literal": NUM_KEY }, "input": "$$obj" } },
        }
    }
    .into()
}

/// Coerces a possibly-wrapped number to a real double for use as a comparison or
/// arithmetic operand. Non-number values pass through unchanged.
fn coerce_number(expr: Bson) -> Bson {
    bind_once(expr, "cse_num", |expr| {
        doc! {
            "$let": {
                "vars": { "ns": number_string(expr.clone()) },
                "in": doc! {
                    "$cond": [
                        doc! { "$ne": [doc! { "$ifNull": ["$$ns", Bson::Null] }, Bson::Null] },
                        doc! { "$toDouble": "$$ns" },
                        expr,
                    ]
                }
            }
        }
        .into()
    })
}

/// Lowers an IR value expression to a MongoDB aggregation expression.
fn lower_expr(expr: &SqlExpr, ctx: &Ctx) -> Result<Bson, SofError> {
    match expr {
        SqlExpr::Lit(v) => Ok(literal_expr(v)),
        SqlExpr::JsonPath { root, path } => lower_json_path(root, path, ctx),
        // `getResourceKey()` on the resource: the SQL `id` column is the
        // document's `id`.
        SqlExpr::ColRef(name) if name == super::compile_path::RESOURCE_ID_COLUMN => {
            lower_json_path(
                "r.data",
                &JsonPath(vec![PathStep::Field("id".to_string())]),
                ctx,
            )
        }
        SqlExpr::RowIndex(scope) => match scope {
            // Top level / non-iterating scopes are always 0. Must be wrapped in
            // `$literal` — a bare `0` in `$project` means "exclude this field".
            RowIndexScope::Top => Ok(literal_expr(&LitValue::Int(0))),
            // `includeArrayIndex` populates `__feN__idx`; a `forEachOrNull` miss
            // leaves it null, which maps to 0.
            RowIndexScope::ForEach(alias) => Ok(Bson::Document(doc! {
                "$ifNull": [format!("${}", unnest_index_field(alias)), Bson::Int32(0)]
            })),
            // The recursive `$unwind` captures the pre-order position into
            // `__<alias>__idx` (the `$function` returns nodes pre-order).
            RowIndexScope::Repeat(alias) => Ok(Bson::Document(doc! {
                "$ifNull": [format!("${}", unnest_index_field(alias)), Bson::Int32(0)]
            })),
        },
        SqlExpr::Param(n) => {
            // 1 = tenant_id, 2 = resource_type are scan predicates and never
            // appear in value position. 3.. index the inlined constants.
            if *n >= 3 {
                ctx.constants
                    .get(*n - 3)
                    .map(literal_expr)
                    .ok_or_else(|| uncompilable(format!("constant param ${n} out of range")))
            } else {
                Err(uncompilable(format!(
                    "parameter ${n} is not valid in value position"
                )))
            }
        }
        SqlExpr::Alias { inner, .. } => lower_expr(inner, ctx),
        SqlExpr::AsJson(inner) => lower_expr(inner, ctx),
        SqlExpr::Cast { inner, .. } => {
            // MongoDB is dynamically typed; stage 1 passes the value through.
            lower_expr(inner, ctx)
        }
        SqlExpr::BinOp { op, lhs, rhs } => lower_binop(*op, lhs, rhs, ctx),
        SqlExpr::UnaryOp { op, inner } => lower_unaryop(*op, inner, ctx),
        SqlExpr::Case { arms, else_ } => {
            let mut branches = Vec::with_capacity(arms.len());
            for (cond, val) in arms {
                let case = truthy(lower_expr(cond, ctx)?);
                let then = lower_expr(val, ctx)?;
                branches.push(Bson::from(doc! { "case": case, "then": then }));
            }
            let default = match else_ {
                Some(e) => lower_expr(e, ctx)?,
                None => Bson::Null,
            };
            Ok(doc! { "$switch": { "branches": branches, "default": default } }.into())
        }
        SqlExpr::Coalesce(parts) => {
            let mut args = Vec::with_capacity(parts.len() + 1);
            for p in parts {
                args.push(lower_expr(p, ctx)?);
            }
            args.push(Bson::Null);
            Ok(doc! { "$ifNull": args }.into())
        }
        SqlExpr::NullIf(a, b) => {
            let a = lower_expr(a, ctx)?;
            let b = lower_expr(b, ctx)?;
            Ok(bind_once(a, "cse_nif", |a| {
                doc! {
                    "$cond": [ doc! { "$eq": [a.clone(), b] }, Bson::Null, a ]
                }
                .into()
            }))
        }
        SqlExpr::ReferenceKey {
            reference,
            expected_type,
        } => lower_reference_key(reference, expected_type.as_deref(), ctx),
        SqlExpr::WhereExists {
            focus,
            iter_alias,
            predicate,
            negate,
        } => lower_where_exists(focus, iter_alias, predicate, *negate, ctx),
        SqlExpr::WhereScalar {
            focus,
            iter_alias,
            predicate,
            projection,
        } => lower_where_scalar(focus, iter_alias, predicate, projection, ctx),
        SqlExpr::JoinAggregate {
            outer_focus,
            outer_alias,
            inner_field,
            inner_alias,
            separator,
        } => lower_join_aggregate(
            outer_focus,
            outer_alias,
            inner_field.as_deref(),
            inner_alias,
            separator,
            ctx,
        ),
        SqlExpr::CollectionAgg { root, path } => lower_collection_agg(root, path, ctx),
        SqlExpr::Boundary { side, kind, source } => lower_boundary(*side, *kind, source, ctx),
        // Constructs reserved for later stages.
        SqlExpr::ColRef(_)
        | SqlExpr::JsonAgg(_)
        | SqlExpr::Scalar(_)
        | SqlExpr::Exists(_)
        | SqlExpr::CountSub(_)
        | SqlExpr::ScalarFromChain { .. }
        | SqlExpr::ChainArray { .. } => Err(uncompilable(
            "expression construct is not supported by the Mongo emitter (stage 1)",
        )),
    }
}

/// A fresh aggregation-variable name for an IR iteration alias (Mongo variable
/// names must start with a lowercase letter; the `v_` prefix guarantees that).
fn scope_var(alias: &str) -> String {
    format!("v_{}", alias.replace(|c: char| !c.is_alphanumeric(), "_"))
}

/// `<focus>.where(<crit>).exists()` → test that the filtered collection is
/// non-empty (negated for `.empty()`).
fn lower_where_exists(
    focus: &SqlExpr,
    iter_alias: &str,
    predicate: &SqlExpr,
    negate: bool,
    ctx: &Ctx,
) -> Result<Bson, SofError> {
    let filtered = lower_where_filter(focus, iter_alias, predicate, true, ctx)?;
    let exists: Bson = doc! { "$gt": [doc! { "$size": filtered }, 0i64] }.into();
    if negate {
        Ok(doc! { "$not": [exists] }.into())
    } else {
        Ok(exists)
    }
}

/// `<focus>.where(<crit>).<navigation>` → project the navigation off the first
/// surviving element.
fn lower_where_scalar(
    focus: &SqlExpr,
    iter_alias: &str,
    predicate: &SqlExpr,
    projection: &SqlExpr,
    ctx: &Ctx,
) -> Result<Bson, SofError> {
    let filtered = lower_where_filter(focus, iter_alias, predicate, false, ctx)?;
    let var = scope_var(iter_alias);
    let child = ctx.child(format!("{iter_alias}.value"), var.clone());
    let projection = lower_expr(projection, &child)?;
    Ok(doc! {
        "$let": {
            "vars": { var: doc! { "$first": filtered } },
            "in": projection,
        }
    }
    .into())
}

/// Shared `$filter` for the `where()` constructs: iterate `focus` (coerced to an
/// array), binding each element to the alias variable, keeping the truthy ones.
///
/// With `flatten`, a `JsonPath` focus crossing more than one field step is
/// flattened across every repeating step (`$reduce`/`$concatArrays`) instead of
/// following only the first element of each intermediate array.
fn lower_where_filter(
    focus: &SqlExpr,
    iter_alias: &str,
    predicate: &SqlExpr,
    flatten: bool,
    ctx: &Ctx,
) -> Result<Bson, SofError> {
    let arr = match focus {
        SqlExpr::JsonPath { root, path }
            if flatten
                && path
                    .0
                    .iter()
                    .filter(|s| matches!(s, PathStep::Field(_)))
                    .count()
                    > 1 =>
        {
            lower_collection_agg(root, path, ctx)?
        }
        _ => ensure_array(lower_expr(focus, ctx)?),
    };
    let var = scope_var(iter_alias);
    let child = ctx.child(format!("{iter_alias}.value"), var.clone());
    let cond = truthy(lower_expr(predicate, &child)?);
    Ok(doc! { "$filter": { "input": arr, "as": var, "cond": cond } }.into())
}

/// `<base>.<field>.join(<sep>)` — flatten `<field>` across `<base>` and join the
/// non-null values with `separator`; without a `<field>` the elements of
/// `<base>` are joined directly. Empty input yields null (matching the SQL
/// `group_concat`/`string_agg` aggregate over no rows).
fn lower_join_aggregate(
    outer_focus: &SqlExpr,
    _outer_alias: &str,
    inner_field: Option<&str>,
    _inner_alias: &str,
    separator: &str,
    ctx: &Ctx,
) -> Result<Bson, SofError> {
    let outer = ensure_array(lower_expr(outer_focus, ctx)?);
    // Flatten `inner_field` across each outer element.
    let flat: Bson = match inner_field {
        Some(inner_field) => doc! {
            "$reduce": {
                "input": outer,
                "initialValue": Bson::Array(Vec::new()),
                "in": {
                    "$concatArrays": [
                        "$$value",
                        ensure_array(doc! { "$getField": { "field": inner_field, "input": "$$this" } }.into()),
                    ]
                }
            }
        }
        .into(),
        None => outer,
    };
    let non_null: Bson = doc! {
        "$filter": {
            "input": flat,
            "as": "j",
            "cond": doc! { "$ne": [doc! { "$ifNull": ["$$j", Bson::Null] }, Bson::Null] },
        }
    }
    .into();
    // Join with `null` as the "first element" sentinel so genuine empty-string
    // elements are preserved; an all-empty input reduces to null.
    let joined: Bson = doc! {
        "$reduce": {
            "input": non_null,
            "initialValue": Bson::Null,
            "in": {
                "$cond": [
                    doc! { "$eq": [doc! { "$ifNull": ["$$value", Bson::Null] }, Bson::Null] },
                    doc! { "$toString": "$$this" },
                    doc! { "$concat": ["$$value", separator, doc! { "$toString": "$$this" }] },
                ]
            }
        }
    }
    .into();
    Ok(joined)
}

/// `column.collection: true` — flatten the path navigation into an array of all
/// matched (non-null) values.
fn lower_collection_agg(root: &str, path: &JsonPath, ctx: &Ctx) -> Result<Bson, SofError> {
    let mut acc: Bson = Bson::Array(vec![root_ref(root, ctx)?]);
    for step in &path.0 {
        match step {
            PathStep::Field(name) => {
                acc = doc! {
                    "$reduce": {
                        "input": acc,
                        "initialValue": Bson::Array(Vec::new()),
                        "in": {
                            "$concatArrays": [
                                "$$value",
                                ensure_array(doc! { "$getField": { "field": name.clone(), "input": "$$this" } }.into()),
                            ]
                        }
                    }
                }
                .into();
            }
            PathStep::Index(n) => {
                acc = ensure_array(index_step(acc, *n));
            }
            PathStep::OfType(_) | PathStep::TypeFilter(_) => {}
        }
    }
    // Drop nulls so an absent leaf contributes nothing to the collection.
    Ok(doc! {
        "$filter": {
            "input": acc,
            "as": "c",
            "cond": doc! { "$ne": [doc! { "$ifNull": ["$$c", Bson::Null] }, Bson::Null] },
        }
    }
    .into())
}

/// Lowers `lowBoundary()`/`highBoundary()`. Date/dateTime/time pad the string
/// form with the first/last instant of the largest unspecified unit; decimal
/// expands the last digit by ±half-a-ulp. Mirrors the SQL `lower_boundary`.
///
/// NOTE: decimal precision depends on the value's textual form. MongoDB stores
/// FHIR decimals as BSON doubles, so `$toString` cannot recover trailing zeros
/// (e.g. `1.0` → `"1"`); the decimal arm is therefore best-effort and may
/// differ from the SQL backends for values with significant trailing zeros.
fn lower_boundary(
    side: BoundarySide,
    kind: BoundaryKind,
    source: &SqlExpr,
    ctx: &Ctx,
) -> Result<Bson, SofError> {
    let src = lower_expr(source, ctx)?;
    let is_null_or_nonstring: Bson =
        doc! { "$ne": [doc! { "$type": src.clone() }, "string"] }.into();
    let strlen: Bson = doc! { "$strLenCP": src.clone() }.into();
    let len_eq = |n: i64| -> Bson { doc! { "$eq": [strlen.clone(), n] }.into() };

    // Last day of the month given the "YYYY-MM…" prefix in `src` (chars 5..7).
    let month_chars: Bson = doc! { "$substrCP": [src.clone(), 5i64, 2i64] }.into();
    let high_day_pad: Bson = doc! {
        "$concat": ["-", doc! { "$switch": {
            "branches": [
                doc! { "case": doc! { "$eq": [month_chars.clone(), "02"] }, "then": "28" },
                doc! { "case": doc! { "$eq": [month_chars.clone(), "04"] }, "then": "30" },
                doc! { "case": doc! { "$eq": [month_chars.clone(), "06"] }, "then": "30" },
                doc! { "case": doc! { "$eq": [month_chars.clone(), "09"] }, "then": "30" },
                doc! { "case": doc! { "$eq": [month_chars, "11"] }, "then": "30" },
            ],
            "default": "31",
        } }]
    }
    .into();

    match kind {
        BoundaryKind::Time => {
            let (minute_pad, second_pad) = match side {
                BoundarySide::Low => (":00.000", ".000"),
                BoundarySide::High => (":59.999", ".999"),
            };
            Ok(doc! { "$switch": {
                "branches": [
                    doc! { "case": is_null_or_nonstring, "then": Bson::Null },
                    doc! { "case": len_eq(5), "then": doc! { "$concat": [src.clone(), minute_pad] } },
                    doc! { "case": len_eq(8), "then": doc! { "$concat": [src.clone(), second_pad] } },
                    doc! { "case": len_eq(12), "then": src },
                ],
                "default": Bson::Null,
            } }
            .into())
        }
        BoundaryKind::Date => {
            let month_only = match side {
                BoundarySide::Low => "-01-01",
                BoundarySide::High => "-12-31",
            };
            let day_pad: Bson = match side {
                BoundarySide::Low => "-01".into(),
                BoundarySide::High => high_day_pad,
            };
            Ok(doc! { "$switch": {
                "branches": [
                    doc! { "case": is_null_or_nonstring, "then": Bson::Null },
                    doc! { "case": len_eq(10), "then": src.clone() },
                    doc! { "case": len_eq(7), "then": doc! { "$concat": [src.clone(), day_pad] } },
                    doc! { "case": len_eq(4), "then": doc! { "$concat": [src, month_only] } },
                ],
                "default": Bson::Null,
            } }
            .into())
        }
        BoundaryKind::DateTime => {
            let full = match side {
                BoundarySide::Low => "T00:00:00.000+14:00",
                BoundarySide::High => "T23:59:59.999-12:00",
            };
            let month_only = match side {
                BoundarySide::Low => "-01-01",
                BoundarySide::High => "-12-31",
            };
            let day_pad: Bson = match side {
                BoundarySide::Low => "-01".into(),
                BoundarySide::High => high_day_pad,
            };
            Ok(doc! { "$switch": {
                "branches": [
                    doc! { "case": is_null_or_nonstring, "then": Bson::Null },
                    doc! { "case": len_eq(10), "then": doc! { "$concat": [src.clone(), full] } },
                    doc! { "case": len_eq(7), "then": doc! { "$concat": [src.clone(), day_pad] } },
                    doc! { "case": len_eq(4), "then": doc! { "$concat": [src, month_only] } },
                ],
                "default": Bson::Null,
            } }
            .into())
        }
        BoundaryKind::Decimal => {
            // The textual form drives precision. Prefer the arbitrary-precision
            // wrapper's exact digit-string (preserves trailing zeros, e.g.
            // "1.0"); fall back to `$toString` for a real BSON number.
            let is_realnum: Bson = doc! {
                "$in": [doc! { "$type": src.clone() }, ["int", "long", "double", "decimal"]]
            }
            .into();
            let str_form: Bson = doc! {
                "$ifNull": [
                    number_string(src.clone()),
                    doc! { "$cond": [is_realnum, doc! { "$toString": src }, Bson::Null] },
                ]
            }
            .into();
            let dot: Bson = doc! { "$indexOfCP": ["$$sf", "."] }.into();
            let after: Bson = doc! {
                "$cond": [
                    doc! { "$eq": [dot.clone(), -1i64] },
                    0i64,
                    doc! { "$subtract": [doc! { "$strLenCP": "$$sf" }, doc! { "$add": [dot, 1i64] }] },
                ]
            }
            .into();
            let half_step: Bson = doc! { "$switch": {
                "branches": [
                    doc! { "case": doc! { "$eq": [after.clone(), 0i64] }, "then": 0.5f64 },
                    doc! { "case": doc! { "$eq": [after.clone(), 1i64] }, "then": 0.05f64 },
                    doc! { "case": doc! { "$eq": [after.clone(), 2i64] }, "then": 0.005f64 },
                    doc! { "case": doc! { "$eq": [after.clone(), 3i64] }, "then": 0.0005f64 },
                    doc! { "case": doc! { "$eq": [after, 4i64] }, "then": 0.00005f64 },
                ],
                "default": 0.000005f64,
            } }
            .into();
            let op = match side {
                BoundarySide::Low => "$subtract",
                BoundarySide::High => "$add",
            };
            Ok(doc! {
                "$let": {
                    "vars": { "sf": str_form },
                    "in": doc! {
                        "$cond": [
                            doc! { "$ne": [doc! { "$ifNull": ["$$sf", Bson::Null] }, Bson::Null] },
                            doc! { op: [doc! { "$toDouble": "$$sf" }, half_step] },
                            Bson::Null,
                        ]
                    }
                }
            }
            .into())
        }
    }
}

/// `key(l, r)` that is null when either operand is null. The right operand is
/// only bound (and so only evaluated) once the left is known to be non-null:
/// `$let` evaluates its vars eagerly, while the old `$or` short-circuited, and
/// `r` may throw (`$divide` by zero, `$concat` of a non-string). Bound variables
/// are safe from capture because every operand is a closed expression.
fn null_guarded_comparison(key: &str, l: Bson, r: Bson) -> Bson {
    let is_null =
        |v: &Bson| doc! { "$eq": [doc! { "$ifNull": [v.clone(), Bson::Null] }, Bson::Null] };
    bind_once(l, "cse_lhs", |l| {
        let on_non_null_left = bind_once(r, "cse_rhs", |r| {
            doc! { "$cond": [is_null(&r), Bson::Null, doc! { key: [l.clone(), r] }] }.into()
        });
        doc! { "$cond": [is_null(&l), Bson::Null, on_non_null_left] }.into()
    })
}

fn lower_binop(op: BinOp, lhs: &SqlExpr, rhs: &SqlExpr, ctx: &Ctx) -> Result<Bson, SofError> {
    // AND/OR coerce their operands to booleans; the rest operate on values.
    if matches!(op, BinOp::And | BinOp::Or) {
        let l = truthy(lower_expr(lhs, ctx)?);
        let r = truthy(lower_expr(rhs, ctx)?);
        let key = if op == BinOp::And { "$and" } else { "$or" };
        return Ok(doc! { key: [l, r] }.into());
    }

    // Numeric operands may be stored as arbitrary-precision wrapper objects;
    // coerce them to real numbers so comparison/arithmetic work.
    let l = coerce_number(lower_expr(lhs, ctx)?);
    let r = coerce_number(lower_expr(rhs, ctx)?);
    let (key, comparison) = match op {
        BinOp::Eq => ("$eq", true),
        BinOp::Neq => ("$ne", true),
        BinOp::Lt => ("$lt", true),
        BinOp::Lte => ("$lte", true),
        BinOp::Gt => ("$gt", true),
        BinOp::Gte => ("$gte", true),
        BinOp::Add => ("$add", false),
        BinOp::Sub => ("$subtract", false),
        BinOp::Mul => ("$multiply", false),
        BinOp::Div => ("$divide", false),
        BinOp::Concat => ("$concat", false),
        BinOp::Like | BinOp::RegexMatch => {
            return Err(uncompilable(
                "LIKE/regex matching is not supported by the Mongo emitter (stage 1)",
            ));
        }
        BinOp::And | BinOp::Or => unreachable!("handled above"),
    };
    if comparison {
        // FHIRPath/SQL three-valued comparison: if either operand is empty
        // (null/missing) the result is null, not a boolean. SQL gets this from
        // `NULL = x → NULL`; Mongo's `$eq`/`$lt`/… return a boolean, so guard
        // explicitly. (Arithmetic/`$concat` already propagate null in Mongo.)
        Ok(null_guarded_comparison(key, l, r))
    } else {
        Ok(doc! { key: [l, r] }.into())
    }
}

fn lower_unaryop(op: UnaryOp, inner: &SqlExpr, ctx: &Ctx) -> Result<Bson, SofError> {
    let v = lower_expr(inner, ctx)?;
    let out = match op {
        UnaryOp::Not => doc! { "$not": [ truthy(v) ] },
        UnaryOp::IsNull => doc! { "$eq": [ doc! { "$ifNull": [v, Bson::Null] }, Bson::Null ] },
        UnaryOp::IsNotNull => doc! { "$ne": [ doc! { "$ifNull": [v, Bson::Null] }, Bson::Null ] },
        UnaryOp::Neg => doc! { "$multiply": [v, -1i64] },
    };
    Ok(out.into())
}

/// `getReferenceKey()` — the trailing id segment of `Reference.reference`, with
/// an optional resource-type guard.
fn lower_reference_key(
    reference: &SqlExpr,
    expected_type: Option<&str>,
    ctx: &Ctx,
) -> Result<Bson, SofError> {
    let reference = lower_expr(reference, ctx)?;
    Ok(bind_once(reference, "cse_rk", |reference| {
        reference_key_body(reference, expected_type)
    }))
}

fn reference_key_body(reference: Bson, expected_type: Option<&str>) -> Bson {
    // Guard against null references (`$split` of null errors).
    let last_segment = doc! {
        "$cond": [
            doc! { "$eq": [ doc! { "$ifNull": [reference.clone(), Bson::Null] }, Bson::Null ] },
            Bson::Null,
            doc! { "$arrayElemAt": [ doc! { "$split": [reference.clone(), "/"] }, -1i64 ] },
        ]
    };

    match expected_type {
        None => last_segment.into(),
        Some(ty) => {
            // Keep the key only when the reference points at `<ty>/...`.
            let regex = format!("(^|/){ty}/");
            doc! {
                "$cond": [
                    doc! { "$regexMatch": { "input": reference, "regex": regex } },
                    last_segment,
                    Bson::Null,
                ]
            }
            .into()
        }
    }
}

/// FHIRPath/SoF truthiness for `$match`/`$expr` and boolean operands: a value
/// is truthy iff it is non-null, non-`false`, non-zero, and non-empty-string.
/// Mirrors the SQL `truthy_predicate` dialect helper.
fn truthy(expr: Bson) -> Bson {
    bind_once(expr, "cse_tv", |expr| {
        doc! {
            "$and": [
                doc! { "$ne": [ doc! { "$ifNull": [expr.clone(), Bson::Null] }, Bson::Null ] },
                doc! { "$ne": [expr.clone(), false] },
                doc! { "$ne": [expr.clone(), 0i64] },
                doc! { "$ne": [expr, ""] },
            ]
        }
        .into()
    })
}

/// MongoDB forbids field names that are empty, start with `$`, or contain `.`.
fn validate_column_name(name: &str) -> Result<(), SofError> {
    if name.is_empty() || name.starts_with('$') || name.contains('.') {
        return Err(uncompilable(format!(
            "column name '{name}' is not a valid MongoDB projection key"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use helios_fhir::FhirVersion;
    use serde_json::json;

    use super::null_guarded_comparison;
    use crate::sof::compiler::compile_view_definition_mongo;
    use mongodb::bson::{Bson, Document, doc};

    fn compile(view: serde_json::Value) -> Vec<mongodb::bson::Document> {
        compile_view_definition_mongo(&view, FhirVersion::R4)
            .expect("view should compile")
            .pipeline
    }

    fn stage_op(stage: &mongodb::bson::Document) -> &str {
        stage.keys().next().map(String::as_str).unwrap_or("")
    }

    #[test]
    fn flat_column_scans_and_projects() {
        let pipeline = compile(json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [{ "column": [{ "name": "id", "path": "id" }] }],
        }));

        assert_eq!(pipeline.len(), 2, "expected [$match, $project]");

        let match_doc = pipeline[0].get_document("$match").unwrap();
        assert_eq!(match_doc.get_str("resource_type").unwrap(), "Patient");
        assert!(!match_doc.get_bool("is_deleted").unwrap());

        let project = pipeline[1].get_document("$project").unwrap();
        assert_eq!(project.get_i32("_id").unwrap(), 0);
        assert!(project.contains_key("id"), "id column must be projected");
    }

    #[test]
    fn dotted_path_uses_first_element_flattening() {
        let pipeline = compile(json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [{ "column": [{ "name": "family", "path": "name.family" }] }],
        }));
        // The projected expression must navigate `family` off the FIRST element
        // of the `name` array — i.e. contain a `$first`/`$isArray` guard.
        let rendered = format!("{:?}", pipeline.last().unwrap());
        assert!(
            rendered.contains("$getField"),
            "expected $getField navigation"
        );
        assert!(
            rendered.contains("$first"),
            "expected first-element flattening"
        );
        assert!(rendered.contains("$isArray"), "expected array guard");
    }

    fn exists_where_pipeline(path: &str) -> String {
        let pipeline = compile(json!({
            "resourceType": "ViewDefinition",
            "resource": "Observation",
            "select": [{ "column": [{ "name": "id", "path": "id" }] }],
            "where": [{ "path": path }],
        }));
        format!("{pipeline:?}")
    }

    #[test]
    fn exists_over_multi_step_focus_flattens_every_repeating_step() {
        let rendered = exists_where_pipeline(
            "component.code.coding.exists(system = 'http://loinc.org' and code = '8480-6')",
        );
        assert!(rendered.contains("$filter"), "expected a $filter");
        assert!(
            rendered.contains("$reduce") && rendered.contains("$concatArrays"),
            "expected $reduce/$concatArrays flattening: {rendered}"
        );

        let negated = exists_where_pipeline("component.code.coding.where(code = '8480-6').empty()");
        assert!(negated.contains("$reduce"), "empty() must flatten too");
    }

    /// MongoDB does no common-subexpression elimination, so a path step that
    /// repeated its operand grew the pipeline as 3^depth (91 KB for this filter).
    /// Operands are bound once with `$let`: the size stays linear in the depth.
    #[test]
    fn multi_step_path_filter_stays_small() {
        let pipeline = compile(json!({
            "resourceType": "ViewDefinition",
            "resource": "Observation",
            "select": [{ "column": [{ "name": "id", "path": "id" }] }],
            "where": [{ "path": "code.coding.first().code = '8302-2'" }],
        }));
        let bytes: usize = pipeline
            .iter()
            .map(|stage| mongodb::bson::to_vec(stage).unwrap().len())
            .sum();
        assert!(
            bytes < 8 * 1024,
            "where filter compiled to {bytes} bytes; operands are being copied"
        );
        let rendered = format!("{pipeline:?}");
        assert!(rendered.contains("$let"), "expected $let bindings");
        assert!(rendered.contains("$$cse_nav"), "expected bound navigation");
    }

    #[test]
    fn comparison_right_operand_is_not_evaluated_for_null_left() {
        // A throwing right operand ($divide by zero) must stay inside the
        // non-null branch, not in the outer $let vars.
        let r = Bson::Document(doc! { "$divide": ["$a", 0] });
        let out = null_guarded_comparison("$eq", Bson::Document(doc! { "$first": "$x" }), r);
        let outer = out.as_document().unwrap().get_document("$let").unwrap();
        let vars = outer.get_document("vars").unwrap();
        assert!(vars.contains_key("cse_lhs") && !vars.contains_key("cse_rhs"));
        let cond = outer
            .get_document("in")
            .unwrap()
            .get_array("$cond")
            .unwrap();
        assert!(
            !format!("{:?}", cond[0]).contains("$divide"),
            "guard must not touch r"
        );
        assert!(
            format!("{:?}", cond[2]).contains("$divide"),
            "r lives in the else branch"
        );
    }

    #[test]
    fn nested_comparisons_bind_only_the_left_outside() {
        let inner = |k| {
            null_guarded_comparison(
                k,
                doc! { "$first": "$a" }.into(),
                doc! { "$first": "$b" }.into(),
            )
        };
        let out = null_guarded_comparison("$eq", inner("$eq"), inner("$lt"));
        let vars = out
            .as_document()
            .unwrap()
            .get_document("$let")
            .unwrap()
            .get_document("vars")
            .unwrap();
        assert_eq!(vars.keys().collect::<Vec<_>>(), ["cse_lhs"]);
    }

    #[test]
    fn exists_over_single_repeating_step_keeps_plain_lowering() {
        let rendered = exists_where_pipeline("category.exists(text = 'vital-signs')");
        assert!(rendered.contains("$filter"), "expected a $filter");
        assert!(!rendered.contains("$reduce"), "single-step focus unchanged");
    }

    #[test]
    fn foreach_emits_set_and_unwind() {
        let pipeline = compile(json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [{ "forEach": "name", "column": [{ "name": "family", "path": "family" }] }],
        }));
        // [$match, $set(__fe), $unwind(__fe), $project]
        assert_eq!(pipeline.len(), 4);
        assert_eq!(stage_op(&pipeline[1]), "$set");
        assert!(
            pipeline[1]
                .get_document("$set")
                .unwrap()
                .contains_key("__fe")
        );

        let unwind = pipeline[2].get_document("$unwind").unwrap();
        assert_eq!(unwind.get_str("path").unwrap(), "$__fe");
        assert!(!unwind.get_bool("preserveNullAndEmptyArrays").unwrap());
    }

    /// A `$project` column's expression, inside the `$ifNull: [expr, null]`
    /// wrapper that keeps absent values as explicit nulls.
    fn column_expr<'a>(project: &'a Document, name: &str) -> &'a Bson {
        let args = project
            .get_document(name)
            .unwrap()
            .get_array("$ifNull")
            .unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(args[1], Bson::Null);
        &args[0]
    }

    #[test]
    fn foreach_row_index_uses_unwind_array_index() {
        let pipeline = compile(json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [{
                "forEach": "name",
                "column": [
                    { "name": "idx", "path": "%rowIndex", "type": "integer" },
                    { "name": "family", "path": "family" },
                ],
            }],
        }));
        // The $unwind captures the element index into `__fe__idx`.
        let unwind = pipeline[2].get_document("$unwind").unwrap();
        assert_eq!(unwind.get_str("includeArrayIndex").unwrap(), "__fe__idx");

        // The `%rowIndex` column reads it back, defaulting a null (forEachOrNull
        // miss) to 0.
        let project = pipeline[3].get_document("$project").unwrap();
        let idx_expr = column_expr(project, "idx").as_document().unwrap();
        let args = idx_expr.get_array("$ifNull").unwrap();
        assert_eq!(args[0].as_str().unwrap(), "$__fe__idx");
        assert_eq!(args[1].as_i32().unwrap(), 0);
    }

    #[test]
    fn top_level_row_index_is_zero() {
        let pipeline = compile(json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [{ "column": [{ "name": "idx", "path": "%rowIndex", "type": "integer" }] }],
        }));
        let project = pipeline.last().unwrap().get_document("$project").unwrap();
        // Wrapped in `$literal` so `$project` treats it as the value 0, not a
        // field-exclusion flag.
        let idx = column_expr(project, "idx").as_document().unwrap();
        assert_eq!(idx.get_i64("$literal").unwrap(), 0);
    }

    #[test]
    fn foreach_or_null_preserves_empties() {
        let pipeline = compile(json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [{ "forEachOrNull": "name", "column": [{ "name": "family", "path": "family" }] }],
        }));
        let unwind = pipeline[2].get_document("$unwind").unwrap();
        assert!(unwind.get_bool("preserveNullAndEmptyArrays").unwrap());
    }

    #[test]
    fn low_boundary_compiles() {
        // Boundary functions are supported by the Mongo emitter.
        let out = compile_view_definition_mongo(
            &json!({
                "resourceType": "ViewDefinition",
                "resource": "Observation",
                "select": [{ "column": [{
                    "name": "low", "path": "value.ofType(Quantity).value.lowBoundary()", "type": "decimal"
                }] }],
            }),
            FhirVersion::R4,
        );
        assert!(out.is_ok(), "boundary should compile: {out:?}");
    }

    #[test]
    fn columns_match_sqlite_compilation() {
        use crate::sof::compiler::{SqlDialect, compile_view_definition_dialect};

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [{
                "column": [
                    { "name": "id", "path": "id" },
                    { "name": "family", "path": "name.family" },
                ]
            }],
        });

        let mongo = compile_view_definition_mongo(&view, FhirVersion::R4).unwrap();
        let sql =
            compile_view_definition_dialect(&view, SqlDialect::Sqlite, FhirVersion::R4).unwrap();
        // The output schema must not depend on the target backend.
        assert_eq!(mongo.columns, sql.columns);
        assert_eq!(mongo.columns, vec!["id".to_string(), "family".to_string()]);
    }

    #[test]
    fn collection_column_compiles() {
        // Collection columns are supported by the Mongo emitter via $reduce.
        let out = compile_view_definition_mongo(
            &json!({
                "resourceType": "ViewDefinition",
                "resource": "Patient",
                "select": [{ "column": [{
                    "name": "given", "path": "name.given", "collection": true
                }] }],
            }),
            FhirVersion::R4,
        );
        assert!(out.is_ok(), "collection column should compile: {out:?}");
    }

    // ---- Hand-built plans: repeat anywhere in the chain, root-level union ----

    use crate::sof::decode::ColumnDecode;
    use crate::sof::emit_mongo::emit_mongo;
    use crate::sof::ir::{
        Column, JsonPath, LitValue, PathStep, PlanNode, ResourceFilter, SqlExpr, SqlType,
    };

    fn path(root: &str, fields: &[&str]) -> SqlExpr {
        SqlExpr::JsonPath {
            root: root.to_string(),
            path: JsonPath(
                fields
                    .iter()
                    .map(|f| PathStep::Field(f.to_string()))
                    .collect(),
            ),
        }
    }

    fn col(name: &str, expr: SqlExpr) -> Column {
        Column {
            name: name.to_string(),
            expr,
            collection: false,
            ty: SqlType::Text,
            decode: ColumnDecode::default(),
        }
    }

    fn scan() -> PlanNode {
        PlanNode::Scan {
            alias: "r".to_string(),
            resource_type: "Questionnaire".to_string(),
            filter: ResourceFilter::default(),
        }
    }

    fn unnest(parent: PlanNode, root: &str, field: &str, alias: &str) -> PlanNode {
        PlanNode::LateralUnnest {
            parent: Box::new(parent),
            source: path(root, &[field]),
            out_alias: alias.to_string(),
            left_join: false,
            on_filter: None,
            flat_index: None,
            array_source: false,
        }
    }

    fn recurse(parent: PlanNode, seed_root: &str, alias: &str) -> PlanNode {
        PlanNode::Recurse {
            parent: Box::new(parent),
            seed_root: seed_root.to_string(),
            step_paths: vec![JsonPath(vec![PathStep::Field("item".to_string())])],
            out_alias: alias.to_string(),
        }
    }

    fn project(parent: PlanNode, columns: Vec<Column>) -> PlanNode {
        PlanNode::Project {
            parent: Box::new(parent),
            columns,
        }
    }

    /// The `$function` seed argument of the `$set` stage that defines `field`.
    fn seed_of(pipeline: &[Document], field: &str) -> String {
        let stage = pipeline
            .iter()
            .find_map(|s| {
                s.get_document("$set")
                    .ok()
                    .filter(|d| d.contains_key(field))
            })
            .unwrap_or_else(|| panic!("no $set for {field}"));
        let func = stage
            .get_document(field)
            .unwrap()
            .get_document("$function")
            .unwrap();
        func.get_array("args").unwrap()[0]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn unwind_paths(pipeline: &[Document]) -> Vec<String> {
        pipeline
            .iter()
            .filter_map(|s| s.get_document("$unwind").ok())
            .map(|u| u.get_str("path").unwrap().to_string())
            .collect()
    }

    #[test]
    fn top_level_repeat_still_seeds_from_the_resource() {
        let plan = project(
            recurse(scan(), "r.data", "rec_0"),
            vec![col("link", path("rec_0.node", &["linkId"]))],
        );
        let out = emit_mongo(&plan, &[]).unwrap();
        assert_eq!(seed_of(&out.pipeline, "__rec_0"), "$data");
    }

    #[test]
    fn repeat_under_foreach_seeds_from_the_element() {
        let plan = project(
            recurse(unnest(scan(), "r.data", "item", "fe"), "fe.value", "rec_1"),
            vec![
                col("outer", path("fe.value", &["linkId"])),
                col("inner", path("rec_1.node", &["linkId"])),
            ],
        );
        let out = emit_mongo(&plan, &[]).unwrap();
        assert_eq!(seed_of(&out.pipeline, "__rec_1"), "$__fe");
        assert_eq!(unwind_paths(&out.pipeline), ["$__fe", "$__rec_1"]);
    }

    #[test]
    fn repeat_inside_repeat_seeds_from_the_outer_node() {
        let plan = project(
            recurse(recurse(scan(), "r.data", "rec_0"), "rec_0.node", "rec_1"),
            vec![
                col("outer", path("rec_0.node", &["linkId"])),
                col("inner", path("rec_1.node", &["linkId"])),
            ],
        );
        let out = emit_mongo(&plan, &[]).unwrap();
        assert_eq!(seed_of(&out.pipeline, "__rec_0"), "$data");
        assert_eq!(seed_of(&out.pipeline, "__rec_1"), "$__rec_0");
        assert_eq!(unwind_paths(&out.pipeline), ["$__rec_0", "$__rec_1"]);
    }

    #[test]
    fn sibling_repeats_both_seed_from_the_same_scope() {
        let plan = project(
            recurse(recurse(scan(), "r.data", "rec_0"), "r.data", "rec_1"),
            vec![
                col("a", path("rec_0.node", &["linkId"])),
                col("b", path("rec_1.node", &["linkId"])),
            ],
        );
        let out = emit_mongo(&plan, &[]).unwrap();
        assert_eq!(seed_of(&out.pipeline, "__rec_0"), "$data");
        assert_eq!(seed_of(&out.pipeline, "__rec_1"), "$data");
    }

    #[test]
    fn repeat_row_index_reads_its_own_unwind_index() {
        let plan = project(
            recurse(unnest(scan(), "r.data", "item", "fe"), "fe.value", "rec_1"),
            vec![col(
                "idx",
                SqlExpr::RowIndex(crate::sof::ir::RowIndexScope::Repeat("rec_1".to_string())),
            )],
        );
        let out = emit_mongo(&plan, &[]).unwrap();
        let project = out
            .pipeline
            .last()
            .unwrap()
            .get_document("$project")
            .unwrap();
        let expr = column_expr(project, "idx").as_document().unwrap();
        assert_eq!(
            expr.get_array("$ifNull").unwrap()[0].as_str().unwrap(),
            "$__rec_1__idx"
        );
    }

    #[test]
    fn union_of_repeat_branches_lowers_to_one_facet() {
        let branch = |alias: &str| {
            project(
                recurse(scan(), "r.data", alias),
                vec![col("v", path(&format!("{alias}.node"), &["linkId"]))],
            )
        };
        let plan = PlanNode::Union(vec![branch("rec_0"), branch("rec_1")]);
        let out = emit_mongo(&plan, &[]).unwrap();
        let facet = out
            .pipeline
            .iter()
            .find_map(|s| s.get_document("$facet").ok())
            .expect("union lowers to $facet");
        assert_eq!(facet.len(), 2);
        assert_eq!(out.columns, vec!["v".to_string()]);
    }

    #[test]
    fn nested_union_is_still_refused() {
        let plan = project(
            PlanNode::Union(vec![project(
                scan(),
                vec![col("v", SqlExpr::Lit(LitValue::Null))],
            )]),
            vec![col("v", SqlExpr::Lit(LitValue::Null))],
        );
        assert!(emit_mongo(&plan, &[]).is_err());
    }
}
