//! MongoDB aggregation-pipeline emitter for the SQL-on-FHIR compiler.
//!
//! Lowers the dialect-agnostic [`PlanNode`] IR (produced by
//! [`build_plan`](super::compile_view::build_plan)) into a MongoDB aggregation
//! pipeline (`Vec<bson::Document>`), the analogue of the SQL emitter in
//! [`emit`](super::emit). Resources are stored as native BSON sub-documents
//! under the `data` field, so a FHIR path `name.family` becomes navigation under
//! `$data`.
//!
//! This is the **Stage 1** emitter: it covers the common path (scans, `forEach`
//! unnests, projections, top-level `where`, and scalar expressions) and returns
//! [`SofError::Uncompilable`] — surfaced by the handler as `422` — for
//! constructs that need later stages (collections, `where()` chains, unions,
//! `repeat:`, and the boundary functions).
//!
//! ## Root-alias mapping
//!
//! The IR addresses values through SQL-style roots: `r.data` (the scanned
//! resource), `fe.value` / `fe2.value` (lateral-unnest iteration rows). Each
//! maps to a MongoDB field: `r.data` → `data`; `feN.value` → a synthetic
//! top-level field `__feN` that the emitter `$set`s to the unnest source and
//! then `$unwind`s.

use mongodb::bson::{Bson, Document, doc};

use crate::core::sof_runner::SofError;

use super::ir::{BinOp, JsonPath, LitValue, PathStep, PlanNode, SqlExpr, UnaryOp};

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
    let PlanNode::Project { parent, columns } = plan else {
        return Err(uncompilable(
            "Mongo emitter expects a Project at the plan root (unions/recursion unsupported)",
        ));
    };

    let mut pipeline = Vec::new();
    lower_source(parent, &mut pipeline, constants)?;

    // Final projection. Column names come verbatim from `column[].name`.
    let mut project = doc! { "_id": 0i32 };
    let mut names = Vec::with_capacity(columns.len());
    for column in columns {
        validate_column_name(&column.name)?;
        if column.collection {
            return Err(uncompilable(
                "collection columns are not supported by the Mongo emitter (stage 1)",
            ));
        }
        let expr = lower_expr(&column.expr, constants)?;
        project.insert(column.name.clone(), expr);
        names.push(column.name.clone());
    }
    pipeline.push(doc! { "$project": project });

    Ok(EmittedMongo {
        pipeline,
        columns: names,
    })
}

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
        } => {
            lower_source(parent, pipeline, constants)?;
            if flat_index.is_some() {
                return Err(uncompilable(
                    "trailing-[N] forEach indexing is not supported by the Mongo emitter (stage 1)",
                ));
            }
            if on_filter.is_some() {
                return Err(uncompilable(
                    "forEach with a trailing where() filter is not supported by the Mongo emitter (stage 1)",
                ));
            }
            let base = unnest_base(out_alias);
            let source_expr = lower_unnest_source(source)?;
            pipeline.push(doc! { "$set": { &base: source_expr } });
            pipeline.push(doc! {
                "$unwind": { "path": format!("${base}"), "preserveNullAndEmptyArrays": *left_join }
            });
            Ok(())
        }
        PlanNode::Filter { parent, predicate } => {
            lower_source(parent, pipeline, constants)?;
            let pred = truthy(lower_expr(predicate, constants)?);
            pipeline.push(doc! { "$match": { "$expr": pred } });
            Ok(())
        }
        PlanNode::Project { .. } => Err(uncompilable(
            "nested Project is not supported by the Mongo emitter",
        )),
        PlanNode::Union(_) => Err(uncompilable(
            "unionAll is not supported by the Mongo emitter (stage 1)",
        )),
        PlanNode::Recurse { .. } => Err(uncompilable(
            "repeat: is not supported by the Mongo emitter",
        )),
    }
}

/// Synthetic top-level field a `feN.value` iteration row is bound to.
fn unnest_base(out_alias: &str) -> String {
    format!("__{out_alias}")
}

/// Resolves an IR root alias to the MongoDB field it lives under.
///
/// `r.data` → `data`; `feN.value` → `__feN`. Anything else (e.g. a recursion
/// node) is unsupported.
fn root_base(root: &str) -> Result<String, SofError> {
    if root == "r.data" {
        return Ok("data".to_string());
    }
    if let Some(alias) = root.strip_suffix(".value")
        && alias.starts_with("fe")
    {
        return Ok(unnest_base(alias));
    }
    Err(uncompilable(format!("unsupported value root '{root}'")))
}

/// `$<field>` reference for an IR root alias.
fn root_ref(root: &str) -> Result<Bson, SofError> {
    Ok(Bson::String(format!("${}", root_base(root)?)))
}

/// Lowers a `forEach` source path to the array expression to unwind.
///
/// Unlike column navigation this does NOT first-element-flatten: `forEach`
/// iterates the whole collection. `build_plan` splits multi-segment paths into
/// one unnest per field, so each source carries a single `Field`.
fn lower_unnest_source(source: &SqlExpr) -> Result<Bson, SofError> {
    let SqlExpr::JsonPath { root, path } = source else {
        return Err(uncompilable("forEach source must be a JSON path"));
    };
    let mut acc = root_ref(root)?;
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
fn lower_json_path(root: &str, path: &JsonPath, _constants: &[LitValue]) -> Result<Bson, SofError> {
    let mut acc = root_ref(root)?;
    for step in &path.0 {
        match step {
            PathStep::Field(name) => {
                let flattened = doc! {
                    "$cond": [
                        doc! { "$isArray": acc.clone() },
                        doc! { "$first": acc.clone() },
                        acc,
                    ]
                };
                acc = doc! { "$getField": { "field": name.clone(), "input": flattened } }.into();
            }
            PathStep::Index(n) => {
                acc = doc! { "$arrayElemAt": [acc, *n] }.into();
            }
            PathStep::OfType(_) | PathStep::TypeFilter(_) => {}
        }
    }
    Ok(acc)
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

/// Lowers an IR value expression to a MongoDB aggregation expression.
fn lower_expr(expr: &SqlExpr, constants: &[LitValue]) -> Result<Bson, SofError> {
    match expr {
        SqlExpr::Lit(v) => Ok(literal_expr(v)),
        SqlExpr::JsonPath { root, path } => lower_json_path(root, path, constants),
        SqlExpr::Param(n) => {
            // 1 = tenant_id, 2 = resource_type are scan predicates and never
            // appear in value position. 3.. index the inlined constants.
            if *n >= 3 {
                constants
                    .get(*n - 3)
                    .map(literal_expr)
                    .ok_or_else(|| uncompilable(format!("constant param ${n} out of range")))
            } else {
                Err(uncompilable(format!(
                    "parameter ${n} is not valid in value position"
                )))
            }
        }
        SqlExpr::Alias { inner, .. } => lower_expr(inner, constants),
        SqlExpr::AsJson(inner) => lower_expr(inner, constants),
        SqlExpr::Cast { inner, .. } => {
            // MongoDB is dynamically typed; stage 1 passes the value through.
            lower_expr(inner, constants)
        }
        SqlExpr::BinOp { op, lhs, rhs } => lower_binop(*op, lhs, rhs, constants),
        SqlExpr::UnaryOp { op, inner } => lower_unaryop(*op, inner, constants),
        SqlExpr::Case { arms, else_ } => {
            let mut branches = Vec::with_capacity(arms.len());
            for (cond, val) in arms {
                let case = truthy(lower_expr(cond, constants)?);
                let then = lower_expr(val, constants)?;
                branches.push(Bson::from(doc! { "case": case, "then": then }));
            }
            let default = match else_ {
                Some(e) => lower_expr(e, constants)?,
                None => Bson::Null,
            };
            Ok(doc! { "$switch": { "branches": branches, "default": default } }.into())
        }
        SqlExpr::Coalesce(parts) => {
            let mut args = Vec::with_capacity(parts.len() + 1);
            for p in parts {
                args.push(lower_expr(p, constants)?);
            }
            args.push(Bson::Null);
            Ok(doc! { "$ifNull": args }.into())
        }
        SqlExpr::NullIf(a, b) => {
            let a = lower_expr(a, constants)?;
            let b = lower_expr(b, constants)?;
            Ok(doc! {
                "$cond": [ doc! { "$eq": [a.clone(), b] }, Bson::Null, a ]
            }
            .into())
        }
        SqlExpr::ReferenceKey {
            reference,
            expected_type,
        } => lower_reference_key(reference, expected_type.as_deref(), constants),
        // Constructs reserved for later stages.
        SqlExpr::ColRef(_)
        | SqlExpr::JsonAgg(_)
        | SqlExpr::Scalar(_)
        | SqlExpr::Exists(_)
        | SqlExpr::CountSub(_)
        | SqlExpr::Boundary { .. }
        | SqlExpr::WhereExists { .. }
        | SqlExpr::WhereScalar { .. }
        | SqlExpr::JoinAggregate { .. }
        | SqlExpr::CollectionAgg { .. }
        | SqlExpr::ScalarFromChain { .. } => Err(uncompilable(
            "expression construct is not supported by the Mongo emitter (stage 1)",
        )),
    }
}

fn lower_binop(
    op: BinOp,
    lhs: &SqlExpr,
    rhs: &SqlExpr,
    constants: &[LitValue],
) -> Result<Bson, SofError> {
    // AND/OR coerce their operands to booleans; the rest operate on values.
    if matches!(op, BinOp::And | BinOp::Or) {
        let l = truthy(lower_expr(lhs, constants)?);
        let r = truthy(lower_expr(rhs, constants)?);
        let key = if op == BinOp::And { "$and" } else { "$or" };
        return Ok(doc! { key: [l, r] }.into());
    }

    let l = lower_expr(lhs, constants)?;
    let r = lower_expr(rhs, constants)?;
    let key = match op {
        BinOp::Eq => "$eq",
        BinOp::Neq => "$ne",
        BinOp::Lt => "$lt",
        BinOp::Lte => "$lte",
        BinOp::Gt => "$gt",
        BinOp::Gte => "$gte",
        BinOp::Add => "$add",
        BinOp::Sub => "$subtract",
        BinOp::Mul => "$multiply",
        BinOp::Div => "$divide",
        BinOp::Concat => "$concat",
        BinOp::Like | BinOp::RegexMatch => {
            return Err(uncompilable(
                "LIKE/regex matching is not supported by the Mongo emitter (stage 1)",
            ));
        }
        BinOp::And | BinOp::Or => unreachable!("handled above"),
    };
    Ok(doc! { key: [l, r] }.into())
}

fn lower_unaryop(op: UnaryOp, inner: &SqlExpr, constants: &[LitValue]) -> Result<Bson, SofError> {
    let v = lower_expr(inner, constants)?;
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
    constants: &[LitValue],
) -> Result<Bson, SofError> {
    let reference = lower_expr(reference, constants)?;
    // Guard against null references (`$split` of null errors).
    let last_segment = doc! {
        "$cond": [
            doc! { "$eq": [ doc! { "$ifNull": [reference.clone(), Bson::Null] }, Bson::Null ] },
            Bson::Null,
            doc! { "$arrayElemAt": [ doc! { "$split": [reference.clone(), "/"] }, -1i64 ] },
        ]
    };

    match expected_type {
        None => Ok(last_segment.into()),
        Some(ty) => {
            // Keep the key only when the reference points at `<ty>/...`.
            let regex = format!("(^|/){ty}/");
            Ok(doc! {
                "$cond": [
                    doc! { "$regexMatch": { "input": reference, "regex": regex } },
                    last_segment,
                    Bson::Null,
                ]
            }
            .into())
        }
    }
}

/// FHIRPath/SoF truthiness for `$match`/`$expr` and boolean operands: a value
/// is truthy iff it is non-null, non-`false`, non-zero, and non-empty-string.
/// Mirrors the SQL `truthy_predicate` dialect helper.
fn truthy(expr: Bson) -> Bson {
    doc! {
        "$and": [
            doc! { "$ne": [ doc! { "$ifNull": [expr.clone(), Bson::Null] }, Bson::Null ] },
            doc! { "$ne": [expr.clone(), false] },
            doc! { "$ne": [expr.clone(), 0i64] },
            doc! { "$ne": [expr, ""] },
        ]
    }
    .into()
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

    use crate::core::sof_runner::SofError;
    use crate::sof::compiler::compile_view_definition_mongo;

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
    fn low_boundary_is_uncompilable() {
        let err = compile_view_definition_mongo(
            &json!({
                "resourceType": "ViewDefinition",
                "resource": "Observation",
                "select": [{ "column": [{
                    "name": "low", "path": "value.ofType(Quantity).value.lowBoundary()"
                }] }],
            }),
            FhirVersion::R4,
        );
        assert!(matches!(err, Err(SofError::Uncompilable { .. })));
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
    fn collection_column_is_uncompilable() {
        let err = compile_view_definition_mongo(
            &json!({
                "resourceType": "ViewDefinition",
                "resource": "Patient",
                "select": [{ "column": [{
                    "name": "given", "path": "name.given", "collection": true
                }] }],
            }),
            FhirVersion::R4,
        );
        assert!(matches!(err, Err(SofError::Uncompilable { .. })));
    }
}
