//! Predicate pushdown for SQL Query dependencies (#1473).
//!
//! A SQL Query materializes each `depends-on` ViewDefinition into the embedded
//! SQLite before its own `WHERE` runs, so a selective query over a large view
//! used to trip the per-dependency row cap. This module finds the simple
//! predicates of the consuming SQL that can be applied *at the source* and
//! rewrites the ViewDefinition JSON to carry them as `where` + `constant`
//! entries, so the runner filters before any row is materialized.
//!
//! The analysis is deliberately conservative: a predicate is pushed only when
//! SQLite and FHIRPath are certain to agree on it, because a wrong pushdown
//! silently drops rows. Anything that does not match every rule is left
//! alone and behaves exactly as before. Nothing here returns an error: an
//! unparseable or unexpected statement simply yields no pushdowns.
//!
//! Rules, in short:
//! - one `SELECT` (no set operations, no top-level `WITH`); only the AND
//!   conjuncts of its top-level `WHERE` count (never `ON`, `HAVING`, `OR`,
//!   or subqueries);
//! - the dependency label appears exactly once in the whole statement, in the
//!   top-level `FROM`/`JOIN` list;
//! - `column <op> literal|:param`, where the column is a plain,
//!   uniquely-named, explicitly-typed, non-collection column outside any
//!   `forEach`/`forEachOrNull`/`repeat`/`unionAll`;
//! - string-like columns: `=` against a string; `integer`/`decimal` columns:
//!   `<`, `<=`, `>`, `>=` against a number. Never `=` on numbers (PostgreSQL
//!   compares a bound parameter as text), nor booleans and date/time types
//!   (FHIRPath partial precision differs from SQLite's text comparison).

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use rusqlite::types::Value as SqlValue;
use serde_json::{Map, Number, Value};
use sqlparser::ast::{
    BinaryOperator, Expr, ObjectName, Query, SetExpr, Statement, TableFactor, UnaryOperator,
    Value as AstValue, visit_relations,
};
use sqlparser::dialect::SQLiteDialect;
use sqlparser::parser::Parser;

use super::bind::BoundParam;
use super::engine::ColumnFhirType;

/// A ViewDefinition column a predicate may be pushed onto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushdownColumn {
    pub name: String,
    /// The column's FHIRPath `path`, evaluated against the resource.
    pub path: String,
    pub fhir_type: ColumnFhirType,
}

/// A comparison operator that can be pushed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOp {
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
}

impl PushOp {
    fn fhirpath(self) -> &'static str {
        match self {
            PushOp::Eq => "=",
            PushOp::Lt => "<",
            PushOp::Le => "<=",
            PushOp::Gt => ">",
            PushOp::Ge => ">=",
        }
    }

    /// The operator with its operands swapped (`5 < x` is `x > 5`).
    fn flipped(self) -> Self {
        match self {
            PushOp::Eq => PushOp::Eq,
            PushOp::Lt => PushOp::Gt,
            PushOp::Le => PushOp::Ge,
            PushOp::Gt => PushOp::Lt,
            PushOp::Ge => PushOp::Le,
        }
    }
}

/// One `column <op> value` predicate found in the consuming SQL.
#[derive(Debug, Clone, PartialEq)]
pub struct PushedPredicate {
    pub column: String,
    pub op: PushOp,
    /// `Text` for string columns, `Integer`/`Real` for numeric columns.
    pub value: SqlValue,
}

const STRING_CODES: &[&str] = &[
    "string",
    "code",
    "id",
    "uri",
    "url",
    "canonical",
    "oid",
    "uuid",
    "markdown",
];

/// The columns of `view` a predicate may be pushed onto, by column name.
///
/// A column qualifies when it declares a `path` and an explicit `type`, is not
/// `collection: true`, sits in a `select` with no `forEach`, `forEachOrNull`,
/// `repeat` or `unionAll` above it, and its name is declared exactly once in
/// the whole ViewDefinition.
pub fn pushable_columns(view: &Value) -> HashMap<String, PushdownColumn> {
    let mut candidates: Vec<PushdownColumn> = Vec::new();
    let mut declared: HashMap<String, usize> = HashMap::new();
    if let Some(selects) = view.get("select").and_then(|v| v.as_array()) {
        for s in selects {
            walk_select(s, true, &mut candidates, &mut declared);
        }
    }
    candidates
        .into_iter()
        .filter(|c| declared.get(&c.name) == Some(&1))
        .map(|c| (c.name.clone(), c))
        .collect()
}

fn walk_select(
    select: &Value,
    pushable: bool,
    out: &mut Vec<PushdownColumn>,
    declared: &mut HashMap<String, usize>,
) {
    let iterates = ["forEach", "forEachOrNull", "repeat"]
        .iter()
        .any(|k| select.get(*k).is_some());
    let here = pushable && !iterates;
    if let Some(cols) = select.get("column").and_then(|v| v.as_array()) {
        for col in cols {
            let Some(name) = col.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            *declared.entry(name.to_string()).or_default() += 1;
            let path = col.get("path").and_then(|v| v.as_str());
            let ty = col.get("type").and_then(|v| v.as_str());
            let collection = col
                .get("collection")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if let (true, false, Some(path), Some(ty)) = (here, collection, path, ty)
                && !path.trim().is_empty()
            {
                out.push(PushdownColumn {
                    name: name.to_string(),
                    path: path.to_string(),
                    fhir_type: ColumnFhirType::from_code(ty),
                });
            }
        }
    }
    if let Some(nested) = select.get("select").and_then(|v| v.as_array()) {
        for s in nested {
            walk_select(s, here, out, declared);
        }
    }
    if let Some(union) = select.get("unionAll").and_then(|v| v.as_array()) {
        for s in union {
            walk_select(s, false, out, declared);
        }
    }
}

/// Finds the pushable predicates of `sql` per dependency label.
///
/// `labels` are the names this statement selects its dependencies by, and
/// `columns` maps a label to the [`pushable_columns`] of its ViewDefinition
/// (labels absent from the map are ignored). `bindings` resolve `:name`
/// placeholders. Returns an empty map for anything unexpected.
pub fn extract_pushdowns(
    sql: &str,
    labels: &[String],
    columns: &HashMap<String, HashMap<String, PushdownColumn>>,
    bindings: &[BoundParam],
) -> HashMap<String, Vec<PushedPredicate>> {
    let mut result = HashMap::new();
    let Ok(stmts) = Parser::parse_sql(&SQLiteDialect {}, sql) else {
        return result;
    };
    let [Statement::Query(query)] = stmts.as_slice() else {
        return result;
    };
    let Some((relations, selection)) = simple_select(query) else {
        return result;
    };
    let Some(selection) = selection else {
        return result;
    };
    let mut conjuncts = Vec::new();
    flatten_and(selection, &mut conjuncts);

    for label in labels {
        let Some(cols) = columns.get(label) else {
            continue;
        };
        if cols.is_empty() || count_relations(&stmts[0], label) != 1 {
            continue;
        }
        // The label's relation in the top-level FROM, and whether it is the
        // only one (the condition for resolving unqualified columns).
        let Some(rel) = relations
            .iter()
            .find(|r| r.name.as_deref() == Some(label.as_str()))
        else {
            continue;
        };
        let only_relation = relations.len() == 1;
        let qualifier = rel.alias.as_deref().unwrap_or(label.as_str());

        let preds: Vec<PushedPredicate> = conjuncts
            .iter()
            .filter_map(|c| conjunct_predicate(c, qualifier, only_relation, cols, bindings))
            .collect();
        if !preds.is_empty() {
            result.insert(label.clone(), preds);
        }
    }
    result
}

/// Adds the predicates to a copy of `view` as `where` entries that reference
/// freshly named `constant`s. Returns `None` when there is nothing to add.
pub fn apply_pushdowns(
    view: &Value,
    cols: &HashMap<String, PushdownColumn>,
    preds: &[PushedPredicate],
) -> Option<Value> {
    let mut object = view.as_object()?.clone();
    let mut constants = take_array(&mut object, "constant");
    let mut wheres = take_array(&mut object, "where");
    let mut used: HashSet<String> = constants
        .iter()
        .filter_map(|c| c.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .collect();

    let mut added = 0usize;
    for pred in preds {
        let Some(col) = cols.get(&pred.column) else {
            continue;
        };
        let (key, json) = match &pred.value {
            SqlValue::Text(s) => ("valueString", Value::String(s.clone())),
            SqlValue::Integer(i) => ("valueInteger", Value::from(*i)),
            SqlValue::Real(f) => match Number::from_f64(*f) {
                Some(n) => ("valueDecimal", Value::Number(n)),
                None => continue,
            },
            _ => continue,
        };
        let mut n = added;
        let name = loop {
            let candidate = format!("__sof_pd_{n}");
            if used.insert(candidate.clone()) {
                break candidate;
            }
            n += 1;
        };
        let mut constant = Map::new();
        constant.insert("name".into(), Value::String(name.clone()));
        constant.insert(key.into(), json);
        constants.push(Value::Object(constant));
        let mut clause = Map::new();
        clause.insert(
            "path".into(),
            Value::String(format!("({}) {} %{name}", col.path, pred.op.fhirpath())),
        );
        wheres.push(Value::Object(clause));
        added += 1;
    }
    if added == 0 {
        return None;
    }
    object.insert("constant".into(), Value::Array(constants));
    object.insert("where".into(), Value::Array(wheres));
    Some(Value::Object(object))
}

fn take_array(object: &mut Map<String, Value>, key: &str) -> Vec<Value> {
    match object.remove(key) {
        Some(Value::Array(a)) => a,
        _ => Vec::new(),
    }
}

/// One table in the top-level `FROM` list (joins included).
struct FromRelation {
    /// The bare table name; `None` for anything that is not a plain,
    /// unqualified table reference.
    name: Option<String>,
    alias: Option<String>,
}

/// The top-level relations and `WHERE` of a plain `SELECT`; `None` for set
/// operations, parenthesised bodies and statements with a top-level `WITH`.
fn simple_select(query: &Query) -> Option<(Vec<FromRelation>, Option<&Expr>)> {
    if query.with.is_some() {
        return None;
    }
    let SetExpr::Select(select) = query.body.as_ref() else {
        return None;
    };
    let mut relations = Vec::new();
    for twj in &select.from {
        relations.push(from_relation(&twj.relation));
        for join in &twj.joins {
            relations.push(from_relation(&join.relation));
        }
    }
    Some((relations, select.selection.as_ref()))
}

fn from_relation(factor: &TableFactor) -> FromRelation {
    match factor {
        TableFactor::Table {
            name, alias, args, ..
        } if args.is_none() => FromRelation {
            name: bare_name(name),
            alias: alias.as_ref().map(|a| a.name.value.clone()),
        },
        _ => FromRelation {
            name: None,
            alias: None,
        },
    }
}

fn bare_name(name: &ObjectName) -> Option<String> {
    match name.0.as_slice() {
        [ident] => Some(ident.value.clone()),
        _ => None,
    }
}

/// How many times `label` is used as a relation anywhere in `stmt`
/// (case-insensitively, since SQLite identifiers are, and by last name part).
fn count_relations(stmt: &Statement, label: &str) -> usize {
    let mut n = 0usize;
    let _ = visit_relations(stmt, |name| {
        if name
            .0
            .last()
            .is_some_and(|i| i.value.eq_ignore_ascii_case(label))
        {
            n += 1;
        }
        ControlFlow::<()>::Continue(())
    });
    n
}

fn flatten_and<'a>(expr: &'a Expr, out: &mut Vec<&'a Expr>) {
    match expr {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            flatten_and(left, out);
            flatten_and(right, out);
        }
        Expr::Nested(inner) => flatten_and(inner, out),
        other => out.push(other),
    }
}

fn conjunct_predicate(
    expr: &Expr,
    qualifier: &str,
    only_relation: bool,
    cols: &HashMap<String, PushdownColumn>,
    bindings: &[BoundParam],
) -> Option<PushedPredicate> {
    let Expr::BinaryOp { left, op, right } = expr else {
        return None;
    };
    let op = match op {
        BinaryOperator::Eq => PushOp::Eq,
        BinaryOperator::Lt => PushOp::Lt,
        BinaryOperator::LtEq => PushOp::Le,
        BinaryOperator::Gt => PushOp::Gt,
        BinaryOperator::GtEq => PushOp::Ge,
        _ => return None,
    };
    let (column, value, op) = match (
        column_name(left, qualifier, only_relation),
        column_name(right, qualifier, only_relation),
    ) {
        (Some(c), None) => (c, literal(right, bindings)?, op),
        (None, Some(c)) => (c, literal(left, bindings)?, op.flipped()),
        _ => return None,
    };
    let col = cols.get(&column)?;
    let allowed = match (&col.fhir_type, &value) {
        (ColumnFhirType::String(code), SqlValue::Text(_)) => {
            STRING_CODES.contains(&code.as_str()) && op == PushOp::Eq
        }
        (
            ColumnFhirType::Integer | ColumnFhirType::Integer64 | ColumnFhirType::Decimal,
            SqlValue::Integer(_) | SqlValue::Real(_),
        ) => op != PushOp::Eq,
        _ => false,
    };
    allowed.then_some(PushedPredicate { column, op, value })
}

/// The column name when `expr` is a column of the dependency (qualified by its
/// alias or label, or bare when it is the only relation).
fn column_name(expr: &Expr, qualifier: &str, only_relation: bool) -> Option<String> {
    match expr {
        Expr::Identifier(id) if only_relation => Some(id.value.clone()),
        Expr::CompoundIdentifier(parts) => match parts.as_slice() {
            [q, c] if q.value == qualifier => Some(c.value.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// A literal, or a `:name` placeholder resolved through `bindings`.
fn literal(expr: &Expr, bindings: &[BoundParam]) -> Option<SqlValue> {
    match expr {
        Expr::Nested(inner) => literal(inner, bindings),
        Expr::Value(AstValue::SingleQuotedString(s)) => Some(SqlValue::Text(s.clone())),
        Expr::Value(AstValue::Number(n, _)) => number(n, false),
        Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr,
        } => match expr.as_ref() {
            Expr::Value(AstValue::Number(n, _)) => number(n, true),
            _ => None,
        },
        Expr::UnaryOp {
            op: UnaryOperator::Plus,
            expr,
        } => match expr.as_ref() {
            Expr::Value(AstValue::Number(n, _)) => number(n, false),
            _ => None,
        },
        Expr::Value(AstValue::Placeholder(p)) => {
            let name = p.strip_prefix(':')?;
            let bound = bindings.iter().find(|b| b.name == name)?;
            match &bound.value {
                v @ (SqlValue::Text(_) | SqlValue::Integer(_)) => Some(v.clone()),
                SqlValue::Real(f) if f.is_finite() => Some(SqlValue::Real(*f)),
                _ => None,
            }
        }
        _ => None,
    }
}

fn number(text: &str, negative: bool) -> Option<SqlValue> {
    let signed = if negative {
        format!("-{text}")
    } else {
        text.to_string()
    };
    if let Ok(i) = signed.parse::<i64>() {
        return Some(SqlValue::Integer(i));
    }
    signed
        .parse::<f64>()
        .ok()
        .filter(|f| f.is_finite())
        .map(SqlValue::Real)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obs_view() -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Observation",
            "constant": [{"name": "__sof_pd_0", "valueString": "taken"}],
            "select": [{
                "column": [
                    {"name": "id", "path": "id", "type": "id"},
                    {"name": "code", "path": "code.coding.first().code", "type": "code"},
                    {"name": "value", "path": "value.ofType(Quantity).value", "type": "decimal"},
                    {"name": "n", "path": "component.count()", "type": "integer"},
                    {"name": "flag", "path": "active", "type": "boolean"},
                    {"name": "effective", "path": "effective.ofType(dateTime)", "type": "dateTime"},
                    {"name": "untyped", "path": "status"},
                    {"name": "multi", "path": "category.coding.code", "type": "code", "collection": true},
                    {"name": "dup", "path": "status", "type": "code"}
                ]
            }, {
                "forEach": "component",
                "column": [{"name": "inside", "path": "code.text", "type": "string"}]
            }, {
                "column": [{"name": "dup", "path": "intent", "type": "code"}]
            }, {
                "unionAll": [{"column": [{"name": "u", "path": "id", "type": "string"}]}]
            }, {
                "select": [{"column": [{"name": "nested", "path": "subject.reference", "type": "string"}]}]
            }]
        })
    }

    fn cols() -> HashMap<String, PushdownColumn> {
        pushable_columns(&obs_view())
    }

    fn run(sql: &str, bindings: &[BoundParam]) -> Vec<PushedPredicate> {
        let mut columns = HashMap::new();
        columns.insert("obs".to_string(), cols());
        extract_pushdowns(sql, &["obs".to_string()], &columns, bindings)
            .remove("obs")
            .unwrap_or_default()
    }

    fn pred(column: &str, op: PushOp, value: SqlValue) -> PushedPredicate {
        PushedPredicate {
            column: column.into(),
            op,
            value,
        }
    }

    fn param(name: &str, value: SqlValue) -> BoundParam {
        BoundParam {
            name: name.into(),
            value,
        }
    }

    #[test]
    fn pushable_columns_applies_the_column_rules() {
        let c = cols();
        for ok in ["id", "code", "value", "n", "flag", "effective", "nested"] {
            assert!(c.contains_key(ok), "{ok} should be pushable");
        }
        for bad in ["untyped", "multi", "dup", "inside", "u"] {
            assert!(!c.contains_key(bad), "{bad} must not be pushable");
        }
        assert_eq!(c["code"].path, "code.coding.first().code");
    }

    #[test]
    fn accepted_shapes() {
        let cases: Vec<(&str, Vec<BoundParam>, Vec<PushedPredicate>)> = vec![
            (
                "SELECT * FROM obs WHERE code = '8302-2'",
                vec![],
                vec![pred("code", PushOp::Eq, SqlValue::Text("8302-2".into()))],
            ),
            (
                "SELECT * FROM obs WHERE obs.code = '8302-2' AND obs.value > 150",
                vec![],
                vec![
                    pred("code", PushOp::Eq, SqlValue::Text("8302-2".into())),
                    pred("value", PushOp::Gt, SqlValue::Integer(150)),
                ],
            ),
            (
                "SELECT * FROM obs o WHERE o.value >= :min AND (o.n < 3)",
                vec![param("min", SqlValue::Real(1.5))],
                vec![
                    pred("value", PushOp::Ge, SqlValue::Real(1.5)),
                    pred("n", PushOp::Lt, SqlValue::Integer(3)),
                ],
            ),
            (
                // Literal on the left flips the operator.
                "SELECT * FROM obs WHERE 150 < value AND -2 <= n",
                vec![],
                vec![
                    pred("value", PushOp::Gt, SqlValue::Integer(150)),
                    pred("n", PushOp::Ge, SqlValue::Integer(-2)),
                ],
            ),
            (
                "SELECT * FROM obs WHERE code = :c",
                vec![param("c", SqlValue::Text("x".into()))],
                vec![pred("code", PushOp::Eq, SqlValue::Text("x".into()))],
            ),
            (
                // A decimal column against an integer literal stays a range.
                "SELECT * FROM obs WHERE value < 10 GROUP BY id",
                vec![],
                vec![pred("value", PushOp::Lt, SqlValue::Integer(10))],
            ),
            (
                // Other (unpushable) conjuncts do not block the pushable ones.
                "SELECT * FROM obs WHERE code = 'a' AND (id = 'x' OR id = 'y') AND untyped = 'q'",
                vec![],
                vec![pred("code", PushOp::Eq, SqlValue::Text("a".into()))],
            ),
        ];
        for (sql, bindings, expected) in cases {
            assert_eq!(run(sql, &bindings), expected, "{sql}");
        }
    }

    #[test]
    fn rejected_shapes() {
        let cases = [
            "SELECT * FROM obs LIMIT 5",
            "SELECT * FROM obs WHERE code = 'a' OR code = 'b'",
            "SELECT * FROM obs WHERE code != 'a'",
            "SELECT * FROM obs WHERE code > 'a'",
            "SELECT * FROM obs WHERE value = 150",
            "SELECT * FROM obs WHERE n = 3",
            "SELECT * FROM obs WHERE value > '150'",
            "SELECT * FROM obs WHERE code = 5",
            "SELECT * FROM obs WHERE flag = 1",
            "SELECT * FROM obs WHERE effective > '2020-01-01'",
            "SELECT * FROM obs WHERE untyped = 'a'",
            "SELECT * FROM obs WHERE multi = 'a'",
            "SELECT * FROM obs WHERE dup = 'a'",
            "SELECT * FROM obs WHERE inside = 'a'",
            "SELECT * FROM obs WHERE nosuch = 'a'",
            "SELECT * FROM obs WHERE Code = 'a'",
            "SELECT * FROM obs WHERE value > :missing",
            "SELECT * FROM obs WHERE code = ?",
            // Self-join: the label is used twice.
            "SELECT * FROM obs a JOIN obs b ON a.id = b.id WHERE a.code = 'x'",
            // Used inside a subquery or CTE as well.
            "SELECT * FROM obs WHERE code = 'x' AND id IN (SELECT id FROM obs)",
            "WITH c AS (SELECT * FROM obs) SELECT * FROM c WHERE code = 'x'",
            "SELECT * FROM (SELECT * FROM obs) WHERE code = 'x'",
            // Set operations.
            "SELECT id FROM obs WHERE code = 'x' UNION SELECT id FROM obs",
            // Unqualified column with a second relation in FROM.
            "SELECT * FROM obs JOIN other ON other.id = obs.id WHERE code = 'x'",
            // A predicate only in ON or HAVING is never taken.
            "SELECT * FROM other JOIN obs ON obs.code = 'x' AND obs.id = other.id",
            "SELECT id FROM obs GROUP BY id HAVING code = 'x'",
            // Wrong qualifier / alias hides the label.
            "SELECT * FROM obs o WHERE obs.code = 'x'",
            "SELECT * FROM obs WHERE z.code = 'x'",
            // Not a SELECT, or not parseable.
            "DELETE FROM obs WHERE code = 'x'",
            "SELEKT nonsense",
        ];
        for sql in cases {
            assert!(
                run(sql, &[]).is_empty(),
                "{sql} must not be pushed: {:?}",
                run(sql, &[])
            );
        }
    }

    #[test]
    fn qualified_join_predicates_on_the_label_are_pushed() {
        let got = run(
            "SELECT * FROM other f JOIN obs o ON o.id = f.id WHERE o.code = 'x' AND f.name = 'y'",
            &[],
        );
        assert_eq!(
            got,
            vec![pred("code", PushOp::Eq, SqlValue::Text("x".into()))]
        );
        let got = run(
            "SELECT * FROM other f LEFT JOIN obs o ON o.id = f.id WHERE o.value > 1",
            &[],
        );
        assert_eq!(got, vec![pred("value", PushOp::Gt, SqlValue::Integer(1))]);
    }

    #[test]
    fn apply_pushdowns_adds_constants_and_wheres_with_unique_names() {
        let view = obs_view();
        let preds = vec![
            pred("code", PushOp::Eq, SqlValue::Text("8302-2".into())),
            pred("value", PushOp::Gt, SqlValue::Real(150.5)),
            pred("n", PushOp::Le, SqlValue::Integer(4)),
        ];
        let out = apply_pushdowns(&view, &cols(), &preds).expect("rewritten");
        // `__sof_pd_0` already exists in the view, so the first free name is used.
        let constants = out["constant"].as_array().unwrap();
        let names: Vec<&str> = constants
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec!["__sof_pd_0", "__sof_pd_1", "__sof_pd_2", "__sof_pd_3"]
        );
        assert_eq!(constants[0]["valueString"], "taken");
        assert_eq!(constants[1]["valueString"], "8302-2");
        assert_eq!(constants[2]["valueDecimal"], 150.5);
        assert_eq!(constants[3]["valueInteger"], 4);
        let wheres: Vec<&str> = out["where"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["path"].as_str().unwrap())
            .collect();
        assert_eq!(
            wheres,
            vec![
                "(code.coding.first().code) = %__sof_pd_1",
                "(value.ofType(Quantity).value) > %__sof_pd_2",
                "(component.count()) <= %__sof_pd_3",
            ]
        );
        // The select (and so the table schema) is untouched; the input is not mutated.
        assert_eq!(out["select"], view["select"]);
        assert!(view.get("where").is_none());
    }

    #[test]
    fn apply_pushdowns_keeps_existing_where_and_returns_none_when_empty() {
        let mut view = obs_view();
        view["where"] = json!([{"path": "status = 'final'"}]);
        let preds = vec![pred("code", PushOp::Eq, SqlValue::Text("x".into()))];
        let out = apply_pushdowns(&view, &cols(), &preds).unwrap();
        let wheres = out["where"].as_array().unwrap();
        assert_eq!(wheres.len(), 2);
        assert_eq!(wheres[0]["path"], "status = 'final'");
        assert!(apply_pushdowns(&view, &cols(), &[]).is_none());
        let unknown = vec![pred("nosuch", PushOp::Eq, SqlValue::Text("x".into()))];
        assert!(apply_pushdowns(&view, &cols(), &unknown).is_none());
    }
}
